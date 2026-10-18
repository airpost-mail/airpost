// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A lenient IMAP response parser.
//!
//! Plays the part of libetpan's `mailimap_parser.c` (Copyright © 2001-2005 DINH Viet Hoa,
//! BSD-3-Clause; see NOTICE.md, "libEtPan!"), with a different design: a response is first
//! read into a generic token tree, and each response kind is then interpreted from the
//! tree, so a field without the expected shape costs that field, not the command. The spike
//! (docs/spikes/imap.md) found real servers sending unescaped quotes in display names and
//! `(NIL)` address lists, which a strict grammar rejects outright.

use ms_core::UidSet;

/// One token of a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    Nil,
    /// An atom, number or flag. Bracketed sections stay part of the atom, so
    /// `BODY[HEADER.FIELDS (References)]<0>` is one token.
    Atom(Vec<u8>),
    /// A quoted string or a literal.
    Str(Vec<u8>),
    List(Vec<Token>),
}

impl Token {
    /// Atom or string bytes; `None` for NIL and lists.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Token::Atom(b) | Token::Str(b) => Some(b),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<String> {
        self.bytes()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    pub fn number(&self) -> Option<u64> {
        let b = self.bytes()?;
        if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(b).ok()?.parse().ok()
    }

    pub fn list(&self) -> Option<&[Token]> {
        match self {
            Token::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_atom(&self, name: &str) -> bool {
        matches!(self, Token::Atom(a) if a.eq_ignore_ascii_case(name.as_bytes()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Ok,
    No,
    Bad,
    Bye,
    PreAuth,
}

impl Status {
    fn from_word(word: &str) -> Option<Status> {
        Some(match word.to_ascii_uppercase().as_str() {
            "OK" => Status::Ok,
            "NO" => Status::No,
            "BAD" => Status::Bad,
            "BYE" => Status::Bye,
            "PREAUTH" => Status::PreAuth,
            _ => return None,
        })
    }
}

/// A response code, `[NAME args]`. `name` is uppercased; `args` is the raw rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Code {
    pub name: String,
    pub args: String,
}

impl Code {
    pub fn arg_words(&self) -> impl Iterator<Item = &str> {
        self.args.split_whitespace()
    }

    pub fn number(&self) -> Option<u64> {
        self.arg_words().next()?.parse().ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusText {
    pub status: Status,
    pub code: Option<Code>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Untagged {
    /// `* OK/NO/BAD/BYE/PREAUTH [code] text`
    State(StatusText),
    Capability(Vec<String>),
    Enabled(Vec<String>),
    Exists(u32),
    Recent(u32),
    Expunge(u32),
    /// `* n FETCH (name value …)`; names as sent.
    Fetch {
        seq: u32,
        items: Vec<(String, Token)>,
    },
    /// LIST, LSUB or XLIST.
    List {
        flags: Vec<String>,
        delimiter: Option<char>,
        name: Vec<u8>,
    },
    /// `* STATUS mailbox (NAME n …)`; names uppercased.
    MailboxStatus {
        mailbox: Vec<u8>,
        items: Vec<(String, u64)>,
    },
    /// The personal namespaces of a NAMESPACE response.
    Namespace {
        personal: Vec<(Vec<u8>, Option<char>)>,
    },
    Vanished {
        earlier: bool,
        uids: UidSet,
    },
    Id(Vec<(String, String)>),
    /// Anything else (FLAGS, SEARCH, LSUB extensions, …), by its uppercased keyword.
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Response {
    Continue(String),
    Tagged { tag: String, st: StatusText },
    Untagged(Untagged),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParseError(pub String);

/// Parses one complete response as [`crate::transport`] frames it: literals inline, the
/// final CRLF included or not.
pub(crate) fn parse_response(buf: &[u8]) -> Result<Response, ParseError> {
    let mut t = Tokenizer::new(buf);
    let first = t.word();
    match first.as_slice() {
        b"+" => Ok(Response::Continue(t.rest_text())),
        b"*" => parse_untagged(&mut t).map(Response::Untagged),
        b"" => Err(ParseError("empty response".into())),
        tag => {
            let tag = String::from_utf8_lossy(tag).into_owned();
            let word = String::from_utf8_lossy(&t.word()).into_owned();
            let status = Status::from_word(&word)
                .filter(|s| matches!(s, Status::Ok | Status::No | Status::Bad))
                .ok_or_else(|| ParseError(format!("tagged response without status: {word:?}")))?;
            Ok(Response::Tagged {
                tag,
                st: t.status_text(status),
            })
        }
    }
}

fn parse_untagged(t: &mut Tokenizer<'_>) -> Result<Untagged, ParseError> {
    let word = t.word();
    let word_s = String::from_utf8_lossy(&word).to_ascii_uppercase();
    if !word.is_empty() && word.iter().all(u8::is_ascii_digit) {
        let n: u32 = word_s
            .parse()
            .map_err(|_| ParseError(format!("number out of range: {word_s}")))?;
        let kind = String::from_utf8_lossy(&t.word()).to_ascii_uppercase();
        return Ok(match kind.as_str() {
            "EXISTS" => Untagged::Exists(n),
            "RECENT" => Untagged::Recent(n),
            "EXPUNGE" => Untagged::Expunge(n),
            "FETCH" => {
                let items = match t.next_token() {
                    Some(Token::List(items)) => fetch_items(items),
                    _ => return Err(ParseError("FETCH without attribute list".into())),
                };
                Untagged::Fetch { seq: n, items }
            }
            _ => Untagged::Other(kind),
        });
    }
    if let Some(status) = Status::from_word(&word_s) {
        return Ok(Untagged::State(t.status_text(status)));
    }
    Ok(match word_s.as_str() {
        "CAPABILITY" => Untagged::Capability(t.rest_words()),
        "ENABLED" => Untagged::Enabled(t.rest_words()),
        "LIST" | "LSUB" | "XLIST" => parse_list(t)?,
        "STATUS" => parse_status(t)?,
        "NAMESPACE" => parse_namespace(t),
        "VANISHED" => parse_vanished(t)?,
        "ID" => parse_id(t),
        _ => Untagged::Other(word_s),
    })
}

fn fetch_items(tokens: Vec<Token>) -> Vec<(String, Token)> {
    let mut items = Vec::with_capacity(tokens.len() / 2);
    let mut it = tokens.into_iter();
    while let Some(name) = it.next() {
        let Some(name) = name.text() else {
            // Not an attribute name: the pairing is lost, so stop rather than misread.
            break;
        };
        let Some(value) = it.next() else { break };
        items.push((name, value));
    }
    items
}

fn delimiter_of(tok: Option<&Token>) -> Option<char> {
    let b = tok?.bytes()?;
    std::str::from_utf8(b).ok()?.chars().next()
}

fn parse_list(t: &mut Tokenizer<'_>) -> Result<Untagged, ParseError> {
    let flags = match t.next_token() {
        Some(Token::List(items)) => items.iter().filter_map(Token::text).collect(),
        Some(Token::Nil) => vec![],
        _ => return Err(ParseError("LIST without attributes".into())),
    };
    let delimiter = delimiter_of(t.next_token().as_ref());
    let name = match t.next_token() {
        Some(tok) => tok.bytes().map(<[u8]>::to_vec),
        None => None,
    }
    .ok_or_else(|| ParseError("LIST without mailbox name".into()))?;
    Ok(Untagged::List {
        flags,
        delimiter,
        name,
    })
}

fn parse_status(t: &mut Tokenizer<'_>) -> Result<Untagged, ParseError> {
    let mailbox = t
        .next_token()
        .and_then(|tok| tok.bytes().map(<[u8]>::to_vec))
        .ok_or_else(|| ParseError("STATUS without mailbox".into()))?;
    let mut items = vec![];
    if let Some(Token::List(list)) = t.next_token() {
        let mut it = list.iter();
        while let (Some(name), Some(value)) = (it.next(), it.next()) {
            if let (Some(name), Some(value)) = (name.text(), value.number()) {
                items.push((name.to_ascii_uppercase(), value));
            }
        }
    }
    Ok(Untagged::MailboxStatus { mailbox, items })
}

fn parse_namespace(t: &mut Tokenizer<'_>) -> Untagged {
    let mut personal = vec![];
    if let Some(Token::List(entries)) = t.next_token() {
        for entry in entries {
            let Some(parts) = entry.list() else { continue };
            let Some(prefix) = parts.first().and_then(Token::bytes) else {
                continue;
            };
            personal.push((prefix.to_vec(), delimiter_of(parts.get(1))));
        }
    }
    Untagged::Namespace { personal }
}

fn parse_vanished(t: &mut Tokenizer<'_>) -> Result<Untagged, ParseError> {
    let mut tok = t.next_token();
    let mut earlier = false;
    if let Some(Token::List(items)) = &tok {
        earlier = items.iter().any(|i| i.is_atom("EARLIER"));
        tok = t.next_token();
    }
    let set = tok
        .as_ref()
        .and_then(Token::text)
        .and_then(|s| UidSet::parse(&s))
        .ok_or_else(|| ParseError("VANISHED without a UID set".into()))?;
    Ok(Untagged::Vanished { earlier, uids: set })
}

fn parse_id(t: &mut Tokenizer<'_>) -> Untagged {
    let mut pairs = vec![];
    if let Some(Token::List(items)) = t.next_token() {
        let mut it = items.iter();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            if let Some(k) = k.text() {
                pairs.push((k, v.text().unwrap_or_default()));
            }
        }
    }
    Untagged::Id(pairs)
}

pub(crate) struct Tokenizer<'a> {
    buf: &'a [u8],
    pos: usize,
}

fn is_line_end(b: u8) -> bool {
    b == b'\r' || b == b'\n'
}

impl<'a> Tokenizer<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Tokenizer { buf, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    fn at_end(&self) -> bool {
        self.peek().is_none_or(is_line_end)
    }

    /// The bytes up to the next space or line end.
    fn word(&mut self) -> Vec<u8> {
        self.skip_spaces();
        let start = self.pos;
        while self.peek().is_some_and(|b| b != b' ' && !is_line_end(b)) {
            self.pos += 1;
        }
        self.buf[start..self.pos].to_vec()
    }

    fn rest_text(&mut self) -> String {
        self.skip_spaces();
        let start = self.pos;
        while self.peek().is_some_and(|b| !is_line_end(b)) {
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.buf[start..self.pos]).into_owned()
    }

    fn rest_words(&mut self) -> Vec<String> {
        self.rest_text()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    /// `[code] text` after a status word.
    fn status_text(&mut self, status: Status) -> StatusText {
        self.skip_spaces();
        let mut code = None;
        if self.peek() == Some(b'[') {
            let start = self.pos + 1;
            let mut end = start;
            while self
                .buf
                .get(end)
                .is_some_and(|b| *b != b']' && !is_line_end(*b))
            {
                end += 1;
            }
            let inner = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
            let (name, args) = match inner.split_once(' ') {
                Some((n, a)) => (n, a.trim()),
                None => (inner.as_str(), ""),
            };
            code = Some(Code {
                name: name.to_ascii_uppercase(),
                args: args.to_owned(),
            });
            self.pos = if self.buf.get(end) == Some(&b']') {
                end + 1
            } else {
                end
            };
        }
        StatusText {
            status,
            code,
            text: self.rest_text(),
        }
    }

    pub fn next_token(&mut self) -> Option<Token> {
        loop {
            self.skip_spaces();
            if self.at_end() {
                return None;
            }
            match self.peek()? {
                // A stray close paren outside any list: skip it.
                b')' => self.pos += 1,
                b'(' => return Some(self.list()),
                b'"' => return Some(Token::Str(self.quoted())),
                b'{' => {
                    if let Some(lit) = self.literal() {
                        return Some(Token::Str(lit));
                    }
                    return Some(self.atom());
                }
                b'~' if self.buf.get(self.pos + 1) == Some(&b'{') => {
                    self.pos += 1;
                    if let Some(lit) = self.literal() {
                        return Some(Token::Str(lit));
                    }
                    return Some(self.atom());
                }
                _ => return Some(self.atom()),
            }
        }
    }

    /// A list; a list still open at the end of the response is closed there.
    fn list(&mut self) -> Token {
        self.pos += 1;
        let mut items = vec![];
        loop {
            self.skip_spaces();
            if self.at_end() {
                break;
            }
            if self.peek() == Some(b')') {
                self.pos += 1;
                break;
            }
            match self.next_token() {
                Some(tok) => items.push(tok),
                None => break,
            }
        }
        Token::List(items)
    }

    fn atom(&mut self) -> Token {
        let start = self.pos;
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'(' | b')' | b'\r' | b'\n' => break,
                b'[' => {
                    // A section: everything up to the matching bracket belongs to the atom.
                    let mut depth = 0usize;
                    while let Some(c) = self.peek() {
                        if is_line_end(c) {
                            break;
                        }
                        self.pos += 1;
                        if c == b'[' {
                            depth += 1;
                        } else if c == b']' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                }
                _ => self.pos += 1,
            }
        }
        let bytes = &self.buf[start..self.pos];
        if bytes.eq_ignore_ascii_case(b"NIL") {
            Token::Nil
        } else {
            Token::Atom(bytes.to_vec())
        }
    }

    /// A quoted string. An unescaped quote ends it only where a token could end: before a
    /// space, `)` or the line end, and, before a space, only if what follows does not look
    /// like the rest of the same string (a run that contains a quote but does not start
    /// with one). So `"Jo "Q" Doe" NIL` reads as `Jo "Q" Doe`.
    fn quoted(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut out = vec![];
        while let Some(b) = self.peek() {
            match b {
                b'\\' if self.pos + 1 < self.buf.len() && !is_line_end(self.buf[self.pos + 1]) => {
                    out.push(self.buf[self.pos + 1]);
                    self.pos += 2;
                }
                b'\r' | b'\n' => break,
                b'"' => {
                    self.pos += 1;
                    if self.quote_closes(self.pos) {
                        break;
                    }
                    out.push(b'"');
                }
                _ => {
                    out.push(b);
                    self.pos += 1;
                }
            }
        }
        out
    }

    fn quote_closes(&self, after: usize) -> bool {
        match self.buf.get(after) {
            None => true,
            Some(b')' | b']' | b'\r' | b'\n') => true,
            Some(b' ') => {
                let mut j = after;
                while self.buf.get(j) == Some(&b' ') {
                    j += 1;
                }
                if self.buf.get(j) == Some(&b'"') {
                    return true;
                }
                let mut k = j;
                while let Some(&c) = self.buf.get(k) {
                    match c {
                        b' ' | b'(' | b')' | b'\r' | b'\n' => return true,
                        b'"' => return false,
                        _ => k += 1,
                    }
                }
                true
            }
            Some(_) => false,
        }
    }

    /// `{n}` CRLF and n bytes (`{n+}` too). `None`, with nothing consumed, if the braces do
    /// not hold a literal marker.
    fn literal(&mut self) -> Option<Vec<u8>> {
        let start = self.pos;
        let mut p = start + 1;
        let digits_start = p;
        while self.buf.get(p).is_some_and(u8::is_ascii_digit) {
            p += 1;
        }
        if p == digits_start {
            return None;
        }
        let n: usize = std::str::from_utf8(&self.buf[digits_start..p])
            .ok()?
            .parse()
            .ok()?;
        if self.buf.get(p) == Some(&b'+') {
            p += 1;
        }
        if self.buf.get(p) != Some(&b'}') {
            return None;
        }
        p += 1;
        if self.buf.get(p) == Some(&b'\r') {
            p += 1;
        }
        if self.buf.get(p) != Some(&b'\n') {
            return None;
        }
        p += 1;
        // The length is the server's: a huge one must not overflow.
        let end = p.saturating_add(n).min(self.buf.len());
        self.pos = end;
        Some(self.buf[p..end].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn untagged(raw: &str) -> Untagged {
        match parse_response(raw.as_bytes()).unwrap() {
            Response::Untagged(u) => u,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_literal_length_near_usize_max_does_not_panic() {
        let raw = format!("* 1 FETCH (UID 4 BODY[] {{{}}}\r\nabc)\r\n", usize::MAX);
        let _ = parse_response(raw.as_bytes());
        let raw = format!("* 1 FETCH (UID 4 BODY[] {{{}}}\r\nabc)\r\n", usize::MAX - 2);
        let _ = parse_response(raw.as_bytes());
    }

    fn tokens(raw: &str) -> Vec<Token> {
        let mut t = Tokenizer::new(raw.as_bytes());
        std::iter::from_fn(|| t.next_token()).collect()
    }

    fn s(v: &str) -> Token {
        Token::Str(v.as_bytes().to_vec())
    }

    fn a(v: &str) -> Token {
        Token::Atom(v.as_bytes().to_vec())
    }

    #[test]
    fn tokens_basic() {
        assert_eq!(
            tokens("(UID 1 FLAGS (\\Seen $Junk) X \"a\\\"b\" NIL)"),
            vec![Token::List(vec![
                a("UID"),
                a("1"),
                a("FLAGS"),
                Token::List(vec![a("\\Seen"), a("$Junk")]),
                a("X"),
                s("a\"b"),
                Token::Nil,
            ])]
        );
        assert_eq!(
            tokens("BODY[HEADER.FIELDS (References)]<0> {5}\r\nab\r\nc rest"),
            vec![
                a("BODY[HEADER.FIELDS (References)]<0>"),
                s("ab\r\nc"),
                a("rest")
            ]
        );
        assert_eq!(
            tokens("(a (b"),
            vec![Token::List(vec![a("a"), Token::List(vec![a("b")])])]
        );
        assert_eq!(tokens("{3+}\r\nxyz"), vec![s("xyz")]);
        assert_eq!(tokens("\"\" \"x\""), vec![s(""), s("x")]);
    }

    #[test]
    fn unescaped_quotes() {
        assert_eq!(
            tokens("(\"Jo \"Q\" Doe\" NIL \"j\" \"d.test\")"),
            vec![Token::List(vec![
                s("Jo \"Q\" Doe"),
                Token::Nil,
                s("j"),
                s("d.test")
            ])]
        );
        assert_eq!(tokens("\"say \"hi\"\")"), vec![s("say \"hi\"")]);
    }

    #[test]
    fn eight_bit() {
        assert_eq!(tokens("\"caf\u{e9}\""), vec![s("caf\u{e9}")]);
    }

    #[test]
    fn tagged() {
        match parse_response(b"12 OK [MESSAGELIMIT 50 11] FETCH completed\r\n").unwrap() {
            Response::Tagged { tag, st } => {
                assert_eq!(tag, "12");
                assert_eq!(st.status, Status::Ok);
                let code = st.code.unwrap();
                assert_eq!(code.name, "MESSAGELIMIT");
                assert_eq!(code.args, "50 11");
                assert_eq!(st.text, "FETCH completed");
            }
            other => panic!("{other:?}"),
        }
        match parse_response(b"C3 NO Mailbox doesn't exist\r\n").unwrap() {
            Response::Tagged { tag, st } => {
                assert_eq!(tag, "C3");
                assert_eq!(st.status, Status::No);
                assert!(st.code.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(parse_response(b"5 WHAT\r\n").is_err());
        assert!(matches!(
            parse_response(b"+ idling\r\n").unwrap(),
            Response::Continue(t) if t == "idling"
        ));
        assert!(matches!(
            parse_response(b"+\r\n").unwrap(),
            Response::Continue(_)
        ));
    }

    #[test]
    fn untagged_kinds() {
        assert_eq!(untagged("* 12 EXISTS\r\n"), Untagged::Exists(12));
        assert_eq!(untagged("* 3 EXPUNGE\r\n"), Untagged::Expunge(3));
        assert_eq!(
            untagged("* CAPABILITY IMAP4rev1 IDLE\r\n"),
            Untagged::Capability(vec!["IMAP4rev1".into(), "IDLE".into()])
        );
        assert_eq!(
            untagged("* OK [HIGHESTMODSEQ 7] x\r\n"),
            Untagged::State(StatusText {
                status: Status::Ok,
                code: Some(Code {
                    name: "HIGHESTMODSEQ".into(),
                    args: "7".into()
                }),
                text: "x".into()
            })
        );
        assert_eq!(
            untagged("* OK [NOMODSEQ] Sorry\r\n"),
            Untagged::State(StatusText {
                status: Status::Ok,
                code: Some(Code {
                    name: "NOMODSEQ".into(),
                    args: "".into()
                }),
                text: "Sorry".into()
            })
        );
        assert_eq!(
            untagged("* XLIST (\\HasNoChildren \\Inbox) \"/\" \"Inbox\"\r\n"),
            Untagged::List {
                flags: vec!["\\HasNoChildren".into(), "\\Inbox".into()],
                delimiter: Some('/'),
                name: b"Inbox".to_vec()
            }
        );
        assert_eq!(
            untagged("* LIST (\\Noselect) NIL \"\"\r\n"),
            Untagged::List {
                flags: vec!["\\Noselect".into()],
                delimiter: None,
                name: vec![]
            }
        );
        assert_eq!(
            untagged("* LIST () \".\" {5}\r\nIN\"BX\r\n"),
            Untagged::List {
                flags: vec![],
                delimiter: Some('.'),
                name: b"IN\"BX".to_vec()
            }
        );
        assert_eq!(
            untagged("* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n"),
            Untagged::Namespace {
                personal: vec![(vec![], Some('/'))]
            }
        );
        assert_eq!(
            untagged("* NAMESPACE ((\"INBOX.\" \".\")) NIL ((\"#shared.\" \".\"))\r\n"),
            Untagged::Namespace {
                personal: vec![(b"INBOX.".to_vec(), Some('.'))]
            }
        );
        assert_eq!(
            untagged(
                "* STATUS \"Sent Items\" (MESSAGES 3 UIDNEXT 9 HIGHESTMODSEQ 12345678901)\r\n"
            ),
            Untagged::MailboxStatus {
                mailbox: b"Sent Items".to_vec(),
                items: vec![
                    ("MESSAGES".into(), 3),
                    ("UIDNEXT".into(), 9),
                    ("HIGHESTMODSEQ".into(), 12345678901)
                ]
            }
        );
        assert_eq!(
            untagged("* VANISHED (EARLIER) 4:5,9\r\n"),
            Untagged::Vanished {
                earlier: true,
                uids: UidSet::parse("4:5,9").unwrap()
            }
        );
        assert_eq!(
            untagged("* VANISHED 7\r\n"),
            Untagged::Vanished {
                earlier: false,
                uids: UidSet::single(7)
            }
        );
        assert_eq!(
            untagged("* ID (\"name\" \"Coremail\" \"vendor\" NIL)\r\n"),
            Untagged::Id(vec![
                ("name".into(), "Coremail".into()),
                ("vendor".into(), "".into())
            ])
        );
        assert_eq!(
            untagged("* SEARCH 1 2\r\n"),
            Untagged::Other("SEARCH".into())
        );
    }

    #[test]
    fn fetch() {
        let u = untagged(
            "* 1 FETCH (UID 4 MODSEQ (12) BODY[HEADER.FIELDS (REFERENCES)] {13}\r\nReferences:\r\n FLAGS ())\r\n",
        );
        match u {
            Untagged::Fetch { seq, items } => {
                assert_eq!(seq, 1);
                assert_eq!(items[0], ("UID".into(), a("4")));
                assert_eq!(items[1], ("MODSEQ".into(), Token::List(vec![a("12")])));
                assert_eq!(
                    items[2],
                    (
                        "BODY[HEADER.FIELDS (REFERENCES)]".into(),
                        s("References:\r\n")
                    )
                );
                assert_eq!(items[3], ("FLAGS".into(), Token::List(vec![])));
            }
            other => panic!("{other:?}"),
        }
        assert!(parse_response(b"* 1 FETCH UID\r\n").is_err());
    }
}
