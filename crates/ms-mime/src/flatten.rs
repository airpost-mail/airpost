// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! HTML to plain text for snippets, search text and the text alternative of outgoing mail.
//!
//! A port of mailcore2's `String::flattenHTMLAndShowBlockquoteAndLink` (`MCString.cpp`,
//! Copyright © 2001-2013 MailCore team, BSD-3-Clause; see NOTICE.md, "mailcore2"), which
//! ran libxml2's SAX HTML parser over a tidied copy of the document. Here html5ever's
//! tokenizer produces the events, and a small open-element stack stands in for libxml2's
//! implied and auto-closed elements (`<html>` around everything, `<p>` closed by the next
//! block, end tags that would close a higher-priority element ignored), so the SAX
//! callbacks below see the nesting they expect. The callbacks themselves follow mailcore
//! line for line. Output drifts from the C++ engine where tidy would have repaired the
//! markup differently (plan section 2).

use std::cell::RefCell;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

use crate::text::{is_whitespace, strip_whitespace};

/// Flattens HTML to text.
/// - `show_blockquote`: when false, `<blockquote type="cite">` and its content are dropped;
///   other blockquotes are always shown, prefixed with `> ` per level.
/// - `show_link`: append ` (href)` after a link's text unless the text already ends with
///   it.
pub fn flatten_html_with(html: &str, show_blockquote: bool, show_link: bool) -> String {
    let sink = Sink {
        state: RefCell::new(State::new(show_blockquote, show_link)),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from_slice(html));
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    let mut state = tokenizer.sink.state.into_inner();
    state.flush_text();
    state.close_all();
    clean_terminal_space(&mut state.result);
    state.result
}

/// The snippet and search-text flattening: `flattenHTML()` then `stripWhitespace()`
/// (`MailProcessor.cpp:408`). `flattenHTML()` is `flattenHTMLAndShowBlockquote(true)`, so
/// cited blockquotes stay in, `> `-prefixed. Plan section 6 says they are dropped; the C++
/// keeps them and this follows the C++.
pub fn flatten_for_snippet(html: &str) -> String {
    strip_whitespace(&flatten_html_with(html, true, true))
}

/// The deepest open-element stack the flattener keeps; see `State::start_tag`.
const MAX_DEPTH: usize = 512;

struct Sink {
    state: RefCell<State>,
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut st = self.state.borrow_mut();
        match token {
            Token::CharacterTokens(t) => st.pending.push_str(&t),
            Token::TagToken(tag) => {
                st.flush_text();
                let name = tag.name.to_string();
                match tag.kind {
                    TagKind::StartTag => {
                        let attrs: Vec<(String, String)> = tag
                            .attrs
                            .iter()
                            .map(|a| (a.name.local.to_string(), a.value.to_string()))
                            .collect();
                        let opened = st.start_tag(&name, &attrs, tag.self_closing);
                        if opened {
                            return match name.as_str() {
                                "script" => TokenSinkResult::RawData(RawKind::ScriptData),
                                "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                                    TokenSinkResult::RawData(RawKind::Rawtext)
                                }
                                "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
                                "plaintext" => TokenSinkResult::Plaintext,
                                _ => TokenSinkResult::Continue,
                            };
                        }
                    }
                    TagKind::EndTag => st.end_tag(&name),
                }
            }
            Token::CommentToken(_) | Token::DoctypeToken(_) => st.flush_text(),
            Token::NullCharacterToken | Token::ParseError(_) => {}
            Token::EOFToken => st.flush_text(),
        }
        TokenSinkResult::Continue
    }
}

const VOID: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "command", "embed", "frame", "hr", "img",
    "input", "isindex", "keygen", "link", "meta", "param", "source", "track", "wbr",
];

/// mailcore's `blockElements()`.
const BLOCK: &[&str] = &[
    "address", "div", "p", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "ul", "ol", "li", "dl", "dt",
    "dd", "form", "col", "colgroup", "th", "tbody", "thead", "tfoot", "table", "tr", "td",
];

const HEAD_CONTENT: &[&str] = &[
    "title", "meta", "link", "style", "script", "base", "noscript", "object", "basefont",
];

/// Elements a start tag implicitly closes, and the elements that stop the search
/// (after libxml2's `htmlStartClose` table, reduced to the cases that change the text).
fn closes(start: &str) -> &'static [(&'static str, &'static [&'static str])] {
    const P_SCOPE: &[&str] = &[
        "table", "td", "th", "li", "dd", "dt", "button", "body", "html",
    ];
    const LI_SCOPE: &[&str] = &["ul", "ol", "menu", "dir", "table", "body", "html"];
    const DL_SCOPE: &[&str] = &["dl", "table", "body", "html"];
    const TABLE_SCOPE: &[&str] = &["table", "html"];
    const SELECT_SCOPE: &[&str] = &["select", "datalist", "html"];
    match start {
        "p" | "div" | "ul" | "ol" | "dl" | "table" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        | "pre" | "blockquote" | "form" | "address" | "hr" | "center" | "fieldset" | "section"
        | "article" | "header" | "footer" | "nav" | "aside" | "figure" | "menu" | "dir"
        | "listing" | "xmp" | "main" | "details" | "summary" => &[("p", P_SCOPE)],
        "li" => &[("p", P_SCOPE), ("li", LI_SCOPE)],
        "dt" | "dd" => &[("p", P_SCOPE), ("dt", DL_SCOPE), ("dd", DL_SCOPE)],
        "tr" => &[
            ("td", TABLE_SCOPE),
            ("th", TABLE_SCOPE),
            ("tr", TABLE_SCOPE),
        ],
        "td" | "th" => &[("td", TABLE_SCOPE), ("th", TABLE_SCOPE)],
        "thead" | "tbody" | "tfoot" => &[
            ("td", TABLE_SCOPE),
            ("th", TABLE_SCOPE),
            ("tr", TABLE_SCOPE),
            ("thead", TABLE_SCOPE),
            ("tbody", TABLE_SCOPE),
            ("tfoot", TABLE_SCOPE),
        ],
        "option" | "optgroup" => &[("option", SELECT_SCOPE)],
        _ => &[],
    }
}

/// libxml2's `htmlGetEndPriority`: an end tag never closes an open element of higher
/// priority.
fn end_priority(name: &str) -> u32 {
    match name {
        "div" => 150,
        "td" | "th" => 160,
        "tr" => 170,
        "thead" | "tbody" | "tfoot" => 180,
        "table" => 190,
        "head" | "body" => 200,
        "html" => 220,
        _ => 100,
    }
}

/// mailcore's `struct parserState`, plus the open-element stack.
struct State {
    result: String,
    pending: String,
    stack: Vec<String>,
    body_seen: bool,
    level: i32,
    enabled: bool,
    disabled_level: i32,
    quote_level: i32,
    has_text: bool,
    has_quote: bool,
    has_return_to_line: bool,
    show_blockquote: bool,
    show_link: bool,
    last_char_is_whitespace: bool,
    link_stack: Vec<(String, usize)>,
    paragraph_spacing_stack: Vec<bool>,
}

impl State {
    fn new(show_blockquote: bool, show_link: bool) -> Self {
        State {
            result: String::new(),
            pending: String::new(),
            stack: Vec::new(),
            body_seen: false,
            level: 0,
            enabled: true,
            disabled_level: 0,
            quote_level: 0,
            has_text: false,
            has_quote: false,
            has_return_to_line: false,
            show_blockquote,
            show_link,
            last_char_is_whitespace: true,
            link_stack: Vec::new(),
            paragraph_spacing_stack: Vec::new(),
        }
    }

    // -- the open-element stack ---------------------------------------------------------

    fn ensure_html(&mut self) {
        if self.stack.is_empty() {
            self.open("html", &[]);
        }
    }

    fn open(&mut self, name: &str, attrs: &[(String, String)]) {
        self.element_started(name, attrs);
        self.stack.push(name.to_owned());
    }

    fn pop(&mut self) {
        if let Some(name) = self.stack.pop() {
            self.element_ended(&name);
        }
    }

    fn pop_through(&mut self, idx: usize) {
        while self.stack.len() > idx {
            self.pop();
        }
    }

    /// Returns whether an element was opened (and so whether raw-text tokenizing applies).
    fn start_tag(&mut self, name: &str, attrs: &[(String, String)], self_closing: bool) -> bool {
        match name {
            "html" => {
                if self.stack.is_empty() {
                    self.open("html", attrs);
                }
                return false;
            }
            "head" => {
                self.ensure_html();
                if self.body_seen || self.stack.iter().any(|e| e == "head") {
                    return false;
                }
                self.open("head", attrs);
                return false;
            }
            "body" => {
                self.ensure_html();
                self.close_head();
                if self.body_seen {
                    return false;
                }
                self.body_seen = true;
                self.open("body", attrs);
                return false;
            }
            _ => {}
        }
        self.ensure_html();
        if !HEAD_CONTENT.contains(&name) {
            self.close_head();
            self.body_seen = true;
        }
        for (target, scope) in closes(name) {
            if let Some(idx) = self.find_open(target, scope) {
                self.pop_through(idx);
            }
        }
        let void = VOID.contains(&name) || self_closing;
        if !void && self.stack.len() >= MAX_DEPTH {
            // Each closing blockquote re-emits every open level's "> ", so the output grows
            // with the square of the nesting; a 1 MB run of <blockquote> would ask for
            // gigabytes. Deeper elements are ignored, and an end tag of an ignored element
            // may close an open one of the same name early. Mail never nests this deep.
            return false;
        }
        self.open(name, attrs);
        if void {
            self.pop();
            return false;
        }
        true
    }

    fn close_head(&mut self) {
        if let Some(idx) = self.stack.iter().rposition(|e| e == "head") {
            self.pop_through(idx);
        }
    }

    fn find_open(&self, target: &str, scope: &[&str]) -> Option<usize> {
        for (idx, e) in self.stack.iter().enumerate().rev() {
            if e == target {
                return Some(idx);
            }
            if scope.contains(&e.as_str()) {
                return None;
            }
        }
        None
    }

    fn end_tag(&mut self, name: &str) {
        if matches!(name, "html" | "body") {
            return;
        }
        let Some(idx) = self.stack.iter().rposition(|e| e == name) else {
            return;
        };
        let priority = end_priority(name);
        if self.stack[idx + 1..]
            .iter()
            .any(|open| end_priority(open) > priority)
        {
            return;
        }
        self.pop_through(idx);
    }

    fn close_all(&mut self) {
        self.pop_through(0);
    }

    fn flush_text(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending);
        if self.stack.is_empty() && text.chars().all(is_whitespace) {
            return;
        }
        self.ensure_html();
        if !self.stack.iter().any(|e| e == "head") {
            self.body_seen = true;
        }
        self.characters_parsed(&text);
    }

    // -- mailcore's SAX callbacks -------------------------------------------------------

    fn characters_parsed(&mut self, ch: &str) {
        if !self.enabled {
            return;
        }
        let has_initial_ws_raw = ch.chars().next().is_some_and(is_whitespace);
        let has_terminal_ws = ch.chars().last().is_some_and(is_whitespace);
        let mut has_initial_ws = has_initial_ws_raw;
        let mut modified = strip_whitespace(ch);
        if has_terminal_ws {
            if modified.is_empty() {
                has_initial_ws = false;
            }
            modified.push(' ');
        }
        if modified.is_empty() {
            return;
        }
        let last_is_ws = modified.ends_with(' ');
        let is_ws = last_is_ws && modified.len() == 1;
        if is_ws {
            if !self.last_char_is_whitespace && self.has_text {
                self.result.push(' ');
                self.last_char_is_whitespace = true;
                self.has_text = true;
            }
        } else {
            if !self.has_quote {
                self.append_quote();
                self.has_quote = true;
            }
            if has_initial_ws && !self.last_char_is_whitespace {
                self.result.push(' ');
            }
            self.result.push_str(&modified);
            self.last_char_is_whitespace = last_is_ws;
            self.has_text = true;
        }
    }

    fn append_quote(&mut self) {
        for _ in 0..self.quote_level.max(0) {
            self.result.push_str("> ");
        }
        self.last_char_is_whitespace = true;
    }

    fn return_to_line(&mut self) {
        if !self.has_quote {
            self.append_quote();
            self.has_quote = true;
        }
        clean_terminal_space(&mut self.result);
        if !self.result.ends_with("\n\n") {
            self.result.push('\n');
        }
        self.has_text = false;
        self.last_char_is_whitespace = true;
        self.has_quote = false;
        self.has_return_to_line = false;
    }

    fn return_to_line_at_beginning_of_block(&mut self) {
        if self.has_text {
            self.return_to_line();
        }
        self.has_quote = false;
    }

    fn element_started(&mut self, name: &str, attrs: &[(String, String)]) {
        let attr = |key: &str| {
            attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str())
        };
        if name == "blockquote" {
            self.quote_level += 1;
        } else if name == "a" {
            let link = attr("href").unwrap_or("").to_owned();
            self.link_stack.push((link, self.result.len()));
        } else if name == "p" {
            let has_spacing = !attr("style").is_some_and(|style| {
                [
                    "margin: 0.0px 0.0px 0.0px 0.0px;",
                    "margin: 0px 0px 0px 0px;",
                    "margin: 0.0px;",
                    "margin: 0px;",
                ]
                .iter()
                .any(|m| style.contains(m))
            });
            self.paragraph_spacing_stack.push(has_spacing);
        }

        if self.enabled {
            if self.level == 1 && name == "head" {
                self.enabled = false;
                self.disabled_level = self.level;
            }
            if matches!(name, "style" | "script" | "title") {
                self.enabled = false;
                self.disabled_level = self.level;
            } else if name == "p" {
                self.return_to_line_at_beginning_of_block();
                if self.paragraph_spacing_stack.last().copied().unwrap_or(true) {
                    self.return_to_line();
                }
            } else if BLOCK.contains(&name) {
                self.return_to_line_at_beginning_of_block();
            } else if name == "blockquote" {
                let cite = attr("type").is_some_and(|t| t.eq_ignore_ascii_case("cite"));
                if !self.show_blockquote && cite {
                    self.enabled = false;
                    self.disabled_level = self.level;
                } else {
                    self.return_to_line_at_beginning_of_block();
                }
            } else if name == "br" {
                self.return_to_line();
                self.has_return_to_line = true;
            }
        }
        self.level += 1;
    }

    fn element_ended(&mut self, name: &str) {
        if name == "blockquote" {
            self.quote_level -= 1;
        }
        self.level -= 1;
        if !self.enabled && self.level == self.disabled_level {
            self.enabled = true;
        }

        let mut has_return_to_line = false;
        if name == "a" {
            if self.enabled {
                if let Some((link, offset)) = self.link_stack.last() {
                    if self.show_link
                        && *offset != self.result.len()
                        && !link.is_empty()
                        && !self.result.ends_with(link.as_str())
                    {
                        let link = link.clone();
                        if !self.last_char_is_whitespace {
                            self.result.push(' ');
                        }
                        self.result.push('(');
                        self.result.push_str(&link);
                        self.result.push(')');
                        self.has_text = true;
                        self.last_char_is_whitespace = false;
                    }
                }
            }
            self.link_stack.pop();
        } else if name == "p" {
            if self.enabled && self.paragraph_spacing_stack.last().copied().unwrap_or(true) {
                self.return_to_line();
            }
            self.paragraph_spacing_stack.pop();
            has_return_to_line = true;
        } else if BLOCK.contains(&name) || name == "blockquote" {
            has_return_to_line = true;
        }

        if has_return_to_line && self.enabled && !self.has_return_to_line {
            self.return_to_line();
        }
    }
}

/// `cleanTerminalSpace`: drops one trailing space.
fn clean_terminal_space(s: &mut String) {
    if s.ends_with(' ') {
        s.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(html: &str) -> String {
        flatten_for_snippet(html)
    }

    #[test]
    fn links_get_their_url() {
        assert_eq!(
            snip(r#"<p>See <a href="https://x.test/a">the docs</a> now</p>"#),
            "See the docs (https://x.test/a) now"
        );
        // the text already ends with the URL
        assert_eq!(
            snip(r#"<a href="https://x.test">https://x.test</a>"#),
            "https://x.test"
        );
        // an empty link adds nothing
        assert_eq!(snip(r#"a<a href="https://x.test"></a>b"#), "ab");
        assert_eq!(snip(r#"<a name="top">anchor</a>"#), "anchor");
    }

    #[test]
    fn cited_blockquotes_are_kept() {
        assert_eq!(
            snip(
                r#"<div>Thanks!</div><blockquote type="cite">On Monday Bob wrote:<br>old text</blockquote><div>-- me</div>"#
            ),
            "Thanks! > On Monday Bob wrote: > old text -- me"
        );
        assert_eq!(
            flatten_html_with(
                r#"<div>Thanks!</div><blockquote type="cite">old text</blockquote><div>-- me</div>"#,
                false,
                true
            ),
            "Thanks!\n\n-- me\n"
        );
        assert_eq!(
            snip(r#"<div>Thanks!</div><blockquote>shown</blockquote>"#),
            "Thanks! > shown"
        );
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let n = 20_000;
        let html = format!("{}x{}", "<blockquote>".repeat(n), "</blockquote>".repeat(n));
        let out = flatten_html_with(&html, true, true);
        assert!(
            out.starts_with(&"> ".repeat(MAX_DEPTH - 1)),
            "{}",
            &out[..80]
        );
        assert!(out.len() < MAX_DEPTH * MAX_DEPTH * 2, "{}", out.len());
    }

    #[test]
    fn head_style_script_hidden() {
        assert_eq!(
            snip("<html><head><title>T</title><style>p{color:red}</style></head><body><script>var x = '<p>';</script><p>Hello</p></body></html>"),
            "Hello"
        );
        // head never closed
        assert_eq!(snip("<html><head><meta charset=utf-8><body>Hi"), "Hi");
        // a style block in the body
        assert_eq!(snip("<p>a</p><style>.x{}</style><p>b</p>"), "a b");
    }

    #[test]
    fn whitespace_and_entities() {
        assert_eq!(snip("  Hello&nbsp;&amp;\n\n  world  "), "Hello & world");
        assert_eq!(snip("<b>bold</b><i>italic</i>"), "bolditalic");
        assert_eq!(snip("one<br>two<br/>three"), "one two three");
        assert_eq!(snip("<table><tr><td>a</td><td>b</td></tr></table>"), "a b");
    }

    #[test]
    fn structured_text_for_the_text_alternative() {
        // Note: libxml2's SAX callbacks end a paragraph twice: once for its spacing and
        // once as a block.
        assert_eq!(
            flatten_html_with("<p>Hi Bob</p>", true, true),
            "\nHi Bob\n\n"
        );
        assert_eq!(
            flatten_html_with("<div>one</div><div>two</div>", true, true),
            "one\ntwo\n"
        );
        assert_eq!(
            flatten_html_with(
                "Reply<blockquote type=\"cite\">quoted<br>more</blockquote>",
                true,
                true
            ),
            "Reply\n> quoted\n> more"
        );
        assert_eq!(
            flatten_html_with("<ul><li>a<li>b</ul>", true, true),
            "a\nb\n\n"
        );
        assert_eq!(
            flatten_html_with(
                "<p style=\"margin: 0px;\">tight</p><p style=\"margin: 0px;\">lines</p>",
                true,
                true
            ),
            "tight\nlines\n"
        );
    }

    #[test]
    fn unbalanced_markup() {
        assert_eq!(snip("<div><span>a</div>b"), "a b");
        // </span> does not close the div it sits outside of
        assert_eq!(snip("<span><div>a</span>b</div>c"), "ab c");
        assert_eq!(snip("<p>one<p>two"), "one two");
        assert_eq!(snip("text with < and > signs"), "text with < and > signs");
        assert_eq!(snip(""), "");
    }
}
