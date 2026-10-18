// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The search grammar (docs/desktop-client-plan.md 3.2, T6): the Gmail-style query text the
//! desktop's search bar takes, parsed and compiled to a `WHERE` clause on `Thread` over the
//! `ThreadSearch` columns. `query.search {grammar: true}` and `search {grammar: true}` use it.
//!
//! Derived from Mailspring `app/src/services/search/search-query-parser.ts`,
//! `search-query-ast.ts` and `search-query-backend-local.ts`, Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! ```text
//! query      := and_query
//! and_query  := or_query [[AND] and_query | NOT and_query]
//! or_query   := simple [OR or_query]
//! simple     := "(" query ")" | field ":" TEXT | is:(read|unread|starred|unstarred)
//!             | has:attachment[s] | TEXT
//! field      := from | to | subject | in | before | after | since
//! TEXT       := "quoted string" | a run of characters other than whitespace, ( ) and :
//! ```
//!
//! Keywords and `AND`/`OR`/`NOT` match in any case. `x NOT y` is `x AND NOT y`, and NOT
//! takes everything after it up to a `)` or `OR`, as on the desktop. Where the desktop's
//! parser differs, on purpose:
//! - A keyword is a field only when a `:` follows it: `trip to paris` is three words (the
//!   desktop throws and falls back to a plain search).
//! - A quoted token is never a keyword or a parenthesis: `"or"` and `"("` are text.
//! - A word glued to a `:` (`http://x.test`, `re:hello`, `subject:re:hello`) stays one word;
//!   the desktop splits it into words and a `:` that matches nothing.
//! - A term with no letters or digits (`-`, a stray `:`, `""`) is an error: the index has
//!   nothing to match it with, and it would empty the search.
//! - `is:` and `has:` with another value, a missing value, a stray `)`, empty parentheses
//!   and a trailing `AND`/`OR`/`NOT` are errors (the desktop throws on some, crashes on
//!   others and silently matches nothing on the rest).
//! - At most [`MAX_TOKENS`] tokens, and the part answered by the index nests at most
//!   [`MAX_MATCH_DEPTH`] deep (runs of AND or OR are flat), for FTS5's parser stack.
//! - Dates are resolved by [`crate::search_dates`] against the request's zone and clock;
//!   `after:` includes its day (`>=` the day's start), `before:` excludes it.

use rusqlite::types::Value as SqlValue;
use serde::Serialize;

use crate::search_dates::{self, DateContext};

/// A parsed query, in the desktop's `QueryExpression` shapes. The JSON form is the one
/// `apps/desktop/app/spec/search-query-capture-spec.ts` writes for the desktop's AST.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Expr {
    And {
        e1: Box<Expr>,
        e2: Box<Expr>,
    },
    Or {
        e1: Box<Expr>,
        e2: Box<Expr>,
    },
    /// `e1` and not `e2`.
    Not {
        e1: Box<Expr>,
        e2: Box<Expr>,
    },
    From {
        text: String,
    },
    To {
        text: String,
    },
    Subject {
        text: String,
    },
    In {
        text: String,
    },
    Generic {
        text: String,
    },
    Unread {
        status: bool,
    },
    Starred {
        status: bool,
    },
    HasAttachment,
    Date {
        text: String,
        direction: DateDirection,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DateDirection {
    Before,
    After,
}

/// Why a query can't be searched. The message is meant for the person who typed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrammarError(pub String);

impl std::fmt::Display for GrammarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GrammarError {}

fn err<T>(msg: impl Into<String>) -> Result<T, GrammarError> {
    Err(GrammarError(msg.into()))
}

// -- Tokens ---------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    s: String,
    quoted: bool,
}

impl Token {
    /// An unquoted token equal to `word` in any case.
    fn is(&self, word: &str) -> bool {
        !self.quoted && self.s.eq_ignore_ascii_case(word)
    }
}

/// The desktop's whitespace: space, tab and newline.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n')
}

fn is_special(c: char) -> bool {
    matches!(c, '(' | ')' | ':')
}

/// `nextToken`: a quoted string, one of `( ) :`, or a run of anything else up to
/// whitespace or one of those three. `None` at the end of the text.
fn next_token(text: &str) -> Result<(Option<Token>, &str), GrammarError> {
    let text = text.trim_start_matches(is_space);
    let Some(first) = text.chars().next() else {
        return Ok((None, text));
    };
    if first == '"' {
        let body = &text[1..];
        return match body.find('"') {
            Some(end) => Ok((
                Some(Token {
                    s: body[..end].to_owned(),
                    quoted: true,
                }),
                &body[end + 1..],
            )),
            None => err("a quoted phrase is missing its closing \""),
        };
    }
    let end = if is_special(first) {
        first.len_utf8()
    } else {
        text.find(|c| is_space(c) || is_special(c))
            .unwrap_or(text.len())
    };
    Ok((
        Some(Token {
            s: text[..end].to_owned(),
            quoted: false,
        }),
        &text[end..],
    ))
}

fn peek_is(text: &str, word: &str) -> Result<bool, GrammarError> {
    Ok(next_token(text)?.0.is_some_and(|t| t.is(word)))
}

/// `consumeExpectedToken`.
fn expect<'a>(text: &'a str, word: &str) -> Result<&'a str, GrammarError> {
    match next_token(text)? {
        (Some(t), rest) if t.is(word) => Ok(rest),
        _ => err(format!("expected '{word}'")),
    }
}

// -- Parser ---------------------------------------------------------------------------------

type Parsed<'a> = Result<(Option<Expr>, &'a str), GrammarError>;

/// A field's value: one word (with any `:` glued to it, as for a plain word) or quoted
/// phrase.
fn field_value<'a>(text: &'a str, field: &str) -> Result<(String, &'a str), GrammarError> {
    match next_token(text)? {
        (Some(t), rest) if t.quoted => Ok((t.s, rest)),
        (Some(t), rest) if !is_special(t.s.chars().next().unwrap_or(' ')) => {
            Ok(glued_word(t.s, rest))
        }
        _ => err(format!(
            "{field}: needs a value, a word or a \"quoted phrase\""
        )),
    }
}

/// `text`, if the index has something to match it with. The tokenizer keeps only letters
/// and digits, and a phrase without any matches nothing, which would empty the whole
/// search without a word.
fn searchable(text: String) -> Result<String, GrammarError> {
    if text.chars().any(char::is_alphanumeric) {
        Ok(text)
    } else if text.trim().is_empty() {
        err("an empty \"\" phrase has nothing to search for")
    } else {
        err(format!("{text:?} has no letters or digits to search for"))
    }
}

/// A plain word, with any `:` and word glued to its end (no whitespace between).
fn glued_word(word: String, mut rest: &str) -> (String, &str) {
    let mut text = word;
    while let Some(after_colon) = rest.strip_prefix(':') {
        text.push(':');
        rest = after_colon;
        let end = rest
            .find(|c| is_space(c) || is_special(c) || c == '"')
            .unwrap_or(rest.len());
        text.push_str(&rest[..end]);
        rest = &rest[end..];
    }
    (text, rest)
}

fn parse_simple(text: &str) -> Parsed<'_> {
    let (tok, after) = next_token(text)?;
    let Some(tok) = tok else {
        return Ok((None, after));
    };
    if tok.is("(") {
        let (exp, after_exp) = parse_and(after)?;
        let Some(exp) = exp else {
            return err("empty parentheses");
        };
        let rest =
            expect(after_exp, ")").map_err(|_| GrammarError("a '(' is never closed".into()))?;
        return Ok((Some(exp), rest));
    }
    if tok.is(")") {
        return err("a ')' has no '(' before it");
    }
    let field = [
        "from", "to", "subject", "in", "is", "has", "before", "after", "since",
    ]
    .into_iter()
    .find(|f| tok.is(f));
    if let Some(field) = field.filter(|_| peek_is(after, ":").unwrap_or(false)) {
        let after_colon = expect(after, ":")?;
        if field == "is" || field == "has" {
            let (value, rest) = field_value(after_colon, field)?;
            let exp = match (field, value.to_ascii_uppercase().as_str()) {
                ("is", "READ") => Expr::Unread { status: false },
                ("is", "UNREAD") => Expr::Unread { status: true },
                ("is", "STARRED") => Expr::Starred { status: true },
                ("is", "UNSTARRED") => Expr::Starred { status: false },
                ("has", "ATTACHMENT" | "ATTACHMENTS") => Expr::HasAttachment,
                ("is", _) => {
                    return err(format!(
                        "is:{value} isn't a search filter: use is:unread, is:read, is:starred \
                         or is:unstarred"
                    ))
                }
                _ => {
                    return err(format!(
                        "has:{value} isn't a search filter: use has:attachment"
                    ))
                }
            };
            return Ok((Some(exp), rest));
        }
        let (text, rest) = field_value(after_colon, field)?;
        let text = match field {
            "before" | "after" | "since" => text,
            _ => searchable(text)?,
        };
        let exp = match field {
            "from" => Expr::From { text },
            "to" => Expr::To { text },
            "subject" => Expr::Subject { text },
            "in" => Expr::In { text },
            "before" => Expr::Date {
                text,
                direction: DateDirection::Before,
            },
            _ => Expr::Date {
                text,
                direction: DateDirection::After,
            },
        };
        return Ok((Some(exp), rest));
    }
    if tok.quoted {
        let text = searchable(tok.s)?;
        return Ok((Some(Expr::Generic { text }), after));
    }
    let (text, rest) = glued_word(tok.s, after);
    let text = searchable(text)?;
    Ok((Some(Expr::Generic { text }), rest))
}

fn parse_or(text: &str) -> Parsed<'_> {
    let (lhs, after_lhs) = parse_simple(text)?;
    let Some(lhs) = lhs else {
        return Ok((None, after_lhs));
    };
    let (tok, after_or) = next_token(after_lhs)?;
    if !tok.is_some_and(|t| t.is("OR")) {
        return Ok((Some(lhs), after_lhs));
    }
    let (rhs, after_rhs) = parse_or(after_or)?;
    let Some(rhs) = rhs else {
        return err("OR needs a search term after it");
    };
    Ok((
        Some(Expr::Or {
            e1: Box::new(lhs),
            e2: Box::new(rhs),
        }),
        after_rhs,
    ))
}

fn parse_and(text: &str) -> Parsed<'_> {
    let (lhs, after_lhs) = parse_or(text)?;
    let Some(lhs) = lhs else {
        return Ok((None, after_lhs));
    };
    let (tok, after_tok) = next_token(after_lhs)?;
    let Some(tok) = tok else {
        return Ok((Some(lhs), after_lhs));
    };
    // Within a group, terms without an operator between them are ANDed; only a `)` or an
    // OR ends the AND.
    if tok.is("OR") || tok.is(")") {
        return Ok((Some(lhs), after_lhs));
    }
    let (mut tok, mut rhs_start) = (tok, after_lhs);
    if tok.is("AND") {
        rhs_start = after_tok;
        tok = match next_token(after_tok)? {
            (Some(t), _) => t,
            (None, _) => return err("AND needs a search term after it"),
        };
    }
    if tok.is("NOT") {
        let after_not = expect(rhs_start, "NOT")?;
        let (rhs, after_rhs) = parse_and(after_not)?;
        let Some(rhs) = rhs else {
            return err("NOT needs a search term after it");
        };
        return Ok((
            Some(Expr::Not {
                e1: Box::new(lhs),
                e2: Box::new(rhs),
            }),
            after_rhs,
        ));
    }
    let (rhs, after_rhs) = parse_and(rhs_start)?;
    let Some(rhs) = rhs else {
        return err("AND needs a search term after it");
    };
    Ok((
        Some(Expr::And {
            e1: Box::new(lhs),
            e2: Box::new(rhs),
        }),
        after_rhs,
    ))
}

/// Tokens a query may have.
pub const MAX_TOKENS: usize = 200;

/// Parses query text (`SearchQueryParser.parse`).
pub fn parse(text: &str) -> Result<Expr, GrammarError> {
    // The parser and the compiler recurse once per token, on a socket reader's stack.
    let mut rest = text;
    for _ in 0..MAX_TOKENS {
        match next_token(rest)? {
            (Some(_), r) => rest = r,
            (None, _) => break,
        }
    }
    if next_token(rest)?.0.is_some() {
        return err(format!(
            "the search is too long: at most {MAX_TOKENS} words, operators and parentheses"
        ));
    }
    let (exp, rest) = parse_and(text)?;
    let Some(exp) = exp else {
        return err("the search has nothing to look for");
    };
    if next_token(rest)?.0.is_some() {
        // parse_and stops early only at a `)` it didn't open.
        return err("a ')' has no '(' before it");
    }
    Ok(exp)
}

// -- Compiler -------------------------------------------------------------------------------

impl Expr {
    /// `isMatchCompatible`: answered by the FTS index alone.
    fn is_match_compatible(&self) -> bool {
        match self {
            Expr::And { e1, e2 } | Expr::Or { e1, e2 } | Expr::Not { e1, e2 } => {
                e1.is_match_compatible() && e2.is_match_compatible()
            }
            Expr::From { .. }
            | Expr::To { .. }
            | Expr::Subject { .. }
            | Expr::In { .. }
            | Expr::Generic { .. } => true,
            Expr::Unread { .. }
            | Expr::Starred { .. }
            | Expr::HasAttachment
            | Expr::Date { .. } => false,
        }
    }

    /// `MatchQueryExpressionVisitor`: the FTS5 expression of a match-compatible subtree.
    /// Every value is one quoted phrase (inner quotes doubled) matched as a prefix, so no
    /// character in it is read as FTS syntax. A run of ANDs or ORs is written flat, and so is
    /// a left run of NOTs (FTS5's NOT is left-associative), because FTS5's parser has a
    /// fixed stack that a right-nested chain of a few dozen terms overflows.
    fn match_expression(&self) -> String {
        let mut out = String::new();
        self.write_match(&mut out);
        out
    }

    fn write_match(&self, out: &mut String) {
        let phrase = |out: &mut String, column: &str, t: &str| {
            let t = t.replace('"', "\"\"");
            if column.is_empty() {
                out.push_str(&format!("(\"{t}\"*)"));
            } else {
                out.push_str(&format!("({column} : \"{t}\"*)"));
            }
        };
        match self {
            Expr::From { text } => phrase(out, "from_", text),
            Expr::To { text } => phrase(out, "to_", text),
            Expr::Subject { text } => phrase(out, "subject", text),
            Expr::In { text } => phrase(out, "categories", text),
            Expr::Generic { text } => phrase(out, "", text),
            Expr::And { .. } | Expr::Or { .. } | Expr::Not { .. } => {
                let (sep, operands) = self.operands();
                out.push('(');
                for (i, e) in operands.into_iter().enumerate() {
                    if i > 0 {
                        out.push_str(sep);
                    }
                    e.write_match(out);
                }
                out.push(')');
            }
            other => unreachable!("not match compatible: {other:?}"),
        }
    }

    /// A binary node's operator and the operands it is written flat with: the leaves of a
    /// run of the same AND or OR, or the left run of a NOT (`(a NOT b) NOT c` is
    /// `a NOT b NOT c`; `a NOT (b NOT c)` keeps its parentheses).
    fn operands(&self) -> (&'static str, Vec<&Expr>) {
        fn run<'a>(
            e: &'a Expr,
            same: fn(&Expr) -> Option<(&Expr, &Expr)>,
            out: &mut Vec<&'a Expr>,
        ) {
            // Iterative on the right, where the parser nests.
            let mut e = e;
            while let Some((e1, e2)) = same(e) {
                run(e1, same, out);
                e = e2;
            }
            out.push(e);
        }
        let mut out = Vec::new();
        match self {
            Expr::And { .. } => {
                run(
                    self,
                    |e| match e {
                        Expr::And { e1, e2 } => Some((&**e1, &**e2)),
                        _ => None,
                    },
                    &mut out,
                );
                (" AND ", out)
            }
            Expr::Or { .. } => {
                run(
                    self,
                    |e| match e {
                        Expr::Or { e1, e2 } => Some((&**e1, &**e2)),
                        _ => None,
                    },
                    &mut out,
                );
                (" OR ", out)
            }
            Expr::Not { .. } => {
                let mut e = self;
                let mut rights = Vec::new();
                while let Expr::Not { e1, e2 } = e {
                    rights.push(e2.as_ref());
                    e = e1;
                }
                out.push(e);
                out.extend(rights.into_iter().rev());
                (" NOT ", out)
            }
            _ => (" ", vec![self]),
        }
    }

    /// How deep [`Expr::match_expression`] nests its parentheses.
    fn match_depth(&self) -> usize {
        match self {
            Expr::And { .. } | Expr::Or { .. } | Expr::Not { .. } => {
                1 + self
                    .operands()
                    .1
                    .into_iter()
                    .map(Expr::match_depth)
                    .max()
                    .unwrap_or(0)
            }
            _ => 1,
        }
    }

    /// The date terms in the query, in order.
    pub fn dates(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_dates(&mut out);
        out
    }

    fn collect_dates<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Expr::And { e1, e2 } | Expr::Or { e1, e2 } | Expr::Not { e1, e2 } => {
                e1.collect_dates(out);
                e2.collect_dates(out);
            }
            Expr::Date { text, .. } => out.push(text),
            _ => {}
        }
    }
}

/// How deep a `ThreadSearch MATCH` expression may nest. The bundled SQLite's FTS5 parser
/// overflows its stack at 34 levels.
pub const MAX_MATCH_DEPTH: usize = 30;

/// A `WHERE` clause and its parameters, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Where {
    pub sql: String,
    pub params: Vec<SqlValue>,
}

/// Compiles `expr` to a condition on the `Thread` table (`LocalSearchQueryBackend`):
/// match-compatible subtrees become one `ThreadSearch MATCH` each, the rest columns of
/// `Thread`. Date terms are resolved with `dates`.
pub fn compile_thread(expr: &Expr, dates: &DateContext) -> Result<Where, GrammarError> {
    let mut w = Where::default();
    thread_where(expr, dates, &mut w)?;
    Ok(w)
}

fn thread_where(expr: &Expr, dates: &DateContext, w: &mut Where) -> Result<(), GrammarError> {
    if expr.is_match_compatible() {
        if expr.match_depth() > MAX_MATCH_DEPTH {
            return err(format!(
                "the search nests too deeply: at most {MAX_MATCH_DEPTH} levels of NOT and \
                 parentheses"
            ));
        }
        w.sql.push_str(
            "(`Thread`.`id` IN (SELECT `content_id` FROM `ThreadSearch` WHERE `ThreadSearch` MATCH ?))",
        );
        w.params.push(SqlValue::Text(expr.match_expression()));
        return Ok(());
    }
    let pair = |e1: &Expr, sep: &str, e2: &Expr, w: &mut Where| {
        w.sql.push('(');
        thread_where(e1, dates, w)?;
        w.sql.push_str(sep);
        thread_where(e2, dates, w)?;
        w.sql.push(')');
        Ok::<_, GrammarError>(())
    };
    match expr {
        Expr::And { e1, e2 } => pair(e1, " AND ", e2, w)?,
        Expr::Or { e1, e2 } => pair(e1, " OR ", e2, w)?,
        Expr::Not { e1, e2 } => pair(e1, " AND NOT ", e2, w)?,
        Expr::Unread { status } => w.sql.push_str(if *status {
            "(`Thread`.`unread` > 0)"
        } else {
            "(`Thread`.`unread` = 0)"
        }),
        Expr::Starred { status } => w.sql.push_str(if *status {
            "(`Thread`.`starred` > 0)"
        } else {
            "(`Thread`.`starred` = 0)"
        }),
        // Note: `attachmentCount` is only in the JSON, as on the desktop.
        Expr::HasAttachment => w
            .sql
            .push_str("(`Thread`.`data` NOT LIKE '%\"attachmentCount\":0%')"),
        Expr::Date { text, direction } => {
            let start = search_dates::resolve(text, dates)?;
            w.sql.push_str(match direction {
                DateDirection::Before => "(`Thread`.`lastMessageReceivedTimestamp` < ?)",
                DateDirection::After => "(`Thread`.`lastMessageReceivedTimestamp` >= ?)",
            });
            w.params.push(SqlValue::Integer(start));
        }
        Expr::From { .. }
        | Expr::To { .. }
        | Expr::Subject { .. }
        | Expr::In { .. }
        | Expr::Generic { .. } => unreachable!("match compatible"),
    }
    Ok(())
}

/// [`parse`] then [`compile_thread`].
pub fn thread_condition(text: &str, dates: &DateContext) -> Result<Where, GrammarError> {
    compile_thread(&parse(text)?, dates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generic(t: &str) -> Expr {
        Expr::Generic { text: t.into() }
    }
    fn and(a: Expr, b: Expr) -> Expr {
        Expr::And {
            e1: Box::new(a),
            e2: Box::new(b),
        }
    }
    fn or(a: Expr, b: Expr) -> Expr {
        Expr::Or {
            e1: Box::new(a),
            e2: Box::new(b),
        }
    }
    fn not(a: Expr, b: Expr) -> Expr {
        Expr::Not {
            e1: Box::new(a),
            e2: Box::new(b),
        }
    }
    fn to(t: &str) -> Expr {
        Expr::To { text: t.into() }
    }
    fn from(t: &str) -> Expr {
        Expr::From { text: t.into() }
    }
    fn subject(t: &str) -> Expr {
        Expr::Subject { text: t.into() }
    }
    fn in_(t: &str) -> Expr {
        Expr::In { text: t.into() }
    }
    fn p(s: &str) -> Expr {
        parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    // The cases of the desktop's `search-query-parser-spec.ts`, in order.
    #[test]
    fn desktop_spec_simple_queries() {
        assert_eq!(p("blah"), generic("blah"));
        assert_eq!(p("\"foo bar\""), generic("foo bar"));
        assert_eq!(p("to:blah"), to("blah"));
        assert_eq!(p("from:blah"), from("blah"));
        assert_eq!(p("subject:blah"), subject("blah"));
        assert_eq!(p("to:mhahnenb@gmail.com"), to("mhahnenb@gmail.com"));
        assert_eq!(p("to:\"mhahnenb@gmail.com\""), to("mhahnenb@gmail.com"));
        assert_eq!(
            p("to:\"Mark mhahnenb@gmail.com\""),
            to("Mark mhahnenb@gmail.com")
        );
        assert_eq!(p("is:unread"), Expr::Unread { status: true });
        assert_eq!(p("is:read"), Expr::Unread { status: false });
        assert_eq!(p("is:starred"), Expr::Starred { status: true });
        assert_eq!(p("is:unstarred"), Expr::Starred { status: false });
        assert_eq!(p("in:foo"), in_("foo"));
        assert_eq!(p("has:attachment"), Expr::HasAttachment);
    }

    #[test]
    fn desktop_spec_reserved_words_as_text() {
        assert_eq!(p("to:blah"), to("blah"));
        assert_eq!(p("to:to"), to("to"));
        assert_eq!(p("to:subject"), to("subject"));
        assert_eq!(p("to:from"), to("from"));
        assert_eq!(p("to:unread"), to("unread"));
        assert_eq!(p("to:starred"), to("starred"));
    }

    #[test]
    fn desktop_spec_compound_queries() {
        let (foo, bar, baz) = (generic("foo"), generic("bar"), generic("baz"));
        assert_eq!(p("foo bar"), and(foo.clone(), bar.clone()));
        assert_eq!(p("foo AND bar"), and(foo.clone(), bar.clone()));
        assert_eq!(p("foo OR bar"), or(foo.clone(), bar.clone()));
        assert_eq!(p("to:foo OR bar"), or(to("foo"), bar.clone()));
        assert_eq!(p("foo OR to:bar"), or(foo.clone(), to("bar")));
        assert_eq!(
            p("foo bar baz"),
            and(foo.clone(), and(bar.clone(), baz.clone()))
        );
        assert_eq!(
            p("foo AND bar AND baz"),
            and(foo.clone(), and(bar.clone(), baz.clone()))
        );
        assert_eq!(
            p("foo OR bar AND baz"),
            and(or(foo.clone(), bar.clone()), baz.clone())
        );
        assert_eq!(
            p("foo OR bar OR baz"),
            or(foo.clone(), or(bar.clone(), baz.clone()))
        );
        assert_eq!(
            p("foo is:unread"),
            and(foo.clone(), Expr::Unread { status: true })
        );
        assert_eq!(p("is:unread foo"), and(Expr::Unread { status: true }, foo));
    }

    #[test]
    fn not_parentheses_and_case() {
        assert_eq!(p("foo NOT bar"), not(generic("foo"), generic("bar")));
        assert_eq!(p("foo AND NOT bar"), not(generic("foo"), generic("bar")));
        assert_eq!(p("foo or bar"), or(generic("foo"), generic("bar")));
        assert_eq!(p("FROM:ann"), from("ann"));
        assert_eq!(p("from : ann"), from("ann"));
        assert_eq!(p("Has:Attachments"), Expr::HasAttachment);
        // The desktop's precedence: NOT takes the rest of the group.
        assert_eq!(
            p("is:unread NOT hello AND in:inbox"),
            not(
                Expr::Unread { status: true },
                and(generic("hello"), in_("inbox"))
            )
        );
        assert_eq!(
            p("subject:\"q3 plan\" NOT (in:trash OR in:spam)"),
            not(subject("q3 plan"), or(in_("trash"), in_("spam")))
        );
        assert_eq!(
            p("(a OR b) c"),
            and(or(generic("a"), generic("b")), generic("c"))
        );
        assert_eq!(
            p("since:2024-01-02 before:yesterday"),
            and(
                Expr::Date {
                    text: "2024-01-02".into(),
                    direction: DateDirection::After
                },
                Expr::Date {
                    text: "yesterday".into(),
                    direction: DateDirection::Before
                }
            )
        );
    }

    #[test]
    fn deviations_from_the_desktop() {
        assert_eq!(
            p("trip to paris"),
            and(generic("trip"), and(generic("to"), generic("paris")))
        );
        assert_eq!(p("\"or\""), generic("or"));
        assert_eq!(
            p("a \"OR\" b"),
            and(generic("a"), and(generic("OR"), generic("b")))
        );
        assert_eq!(p("\"(a\""), generic("(a"));
        assert_eq!(p("http://x.test/a"), generic("http://x.test/a"));
        assert_eq!(p("re:hello"), generic("re:hello"));
        assert_eq!(p("subject:re:hello"), subject("re:hello"));
        assert_eq!(
            p("to:mailto:x@y.test b"),
            and(to("mailto:x@y.test"), generic("b"))
        );
        assert_eq!(
            p("this is it"),
            and(generic("this"), and(generic("is"), generic("it")))
        );
    }

    #[test]
    fn errors() {
        for q in [
            "",
            "   ",
            "\"unterminated",
            "from:",
            "from: ",
            "to:(bob)",
            "is:weird",
            "is:",
            "has:pdf",
            "foo)",
            ")",
            "(foo",
            "()",
            "foo OR",
            "foo AND",
            "foo NOT",
            "(foo OR) bar",
            "a - b",
            "a : b",
            "\"\"",
            "a \" \"",
            "\"--\" OR a",
            "from:\"\"",
            "subject:-",
            "\"(\"",
        ] {
            assert!(parse(q).is_err(), "{q:?} parsed as {:?}", parse(q));
        }
        let long = vec!["a"; MAX_TOKENS].join(" ");
        assert!(parse(&long).is_ok());
        assert!(parse(&format!("{long} b"))
            .unwrap_err()
            .0
            .contains("too long"));
        assert!(parse(&"(".repeat(100_000))
            .unwrap_err()
            .0
            .contains("too long"));
        // The deepest tree that parses still compiles on a socket reader's stack (the
        // default 2 MiB), in a debug build.
        let deep = format!("{}a{}", "(".repeat(99), ")".repeat(99));
        let compiled = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || thread_condition(&deep, &DateContext::utc(None)).map(|w| w.params.len()))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(compiled, Ok(1));
        let ors = vec!["a"; MAX_TOKENS / 2].join(" OR ");
        assert!(parse(&ors).is_ok());
    }

    #[test]
    fn match_expressions() {
        assert_eq!(
            p("from:ann \"q 3\" OR in:sent").match_expression(),
            r#"((from_ : "ann"*) AND (("q 3"*) OR (categories : "sent"*)))"#
        );
        assert_eq!(
            p("subject:a NOT to:b").match_expression(),
            r#"((subject : "a"*) NOT (to_ : "b"*))"#
        );
        assert_eq!(p("say\"hi").match_expression(), r#"("say""hi"*)"#);
        // Runs of one operator are flat; other nesting keeps its parentheses.
        assert_eq!(
            p("a b (c d)").match_expression(),
            r#"(("a"*) AND ("b"*) AND ("c"*) AND ("d"*))"#
        );
        assert_eq!(
            p("a OR (b OR c)").match_expression(),
            r#"(("a"*) OR ("b"*) OR ("c"*))"#
        );
        assert_eq!(
            p("(a b) OR c").match_expression(),
            r#"((("a"*) AND ("b"*)) OR ("c"*))"#
        );
        assert_eq!(
            p("(a NOT b) NOT c").match_expression(),
            r#"(("a"*) NOT ("b"*) NOT ("c"*))"#
        );
        assert_eq!(
            p("a NOT b NOT c").match_expression(),
            r#"(("a"*) NOT (("b"*) NOT ("c"*)))"#
        );
        assert_eq!(
            p("(a b) NOT c").match_expression(),
            r#"((("a"*) AND ("b"*)) NOT ("c"*))"#
        );
        assert_eq!(p(&vec!["a"; MAX_TOKENS].join(" ")).match_depth(), 2);
    }

    #[test]
    fn compiles_mixed_trees() {
        let ctx = DateContext::utc(None);
        let w = thread_condition("foo is:unread", &ctx).unwrap();
        assert_eq!(
            w.sql,
            "((`Thread`.`id` IN (SELECT `content_id` FROM `ThreadSearch` WHERE `ThreadSearch` MATCH ?)) \
             AND (`Thread`.`unread` > 0))"
        );
        assert_eq!(w.params, [SqlValue::Text("(\"foo\"*)".into())]);
        let w = thread_condition("a b NOT is:starred OR has:attachment", &ctx).unwrap();
        assert!(w.sql.contains("AND NOT"), "{}", w.sql);
        assert_eq!(
            w.params,
            [
                SqlValue::Text("(\"a\"*)".into()),
                SqlValue::Text("(\"b\"*)".into())
            ]
        );
        let w = thread_condition("(a b) is:unread", &ctx).unwrap();
        assert_eq!(w.params, [SqlValue::Text("((\"a\"*) AND (\"b\"*))".into())]);
        let w = thread_condition("before:2024-01-02", &ctx).unwrap();
        assert_eq!(w.sql, "(`Thread`.`lastMessageReceivedTimestamp` < ?)");
        assert_eq!(w.params, [SqlValue::Integer(1_704_153_600)]);
        assert!(thread_condition("after:someday", &ctx).is_err());
    }
}
