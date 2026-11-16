// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! ENVELOPE and fetched header sections to [`MessageHeader`].
//!
//! Derived from mailcore2's `MessageHeader::importIMAPEnvelope`, `importIMAPReferences`,
//! `importIMFFields` (`MCMessageHeader.cpp`) and `Address::addressWithIMAPAddress`
//! (`MCAddress.cpp`), Copyright © 2001-2013 MailCore team, BSD-3-Clause, with
//! Mailspring-Sync's changes, Copyright © 2017 Foundry 376, GPL-3.0, and libetpan's
//! `mailimf_msg_id_parse` (`mailimf.c`), Copyright © 2001-2005 DINH Viet Hoa, BSD-3-Clause.
//! See NOTICE.md ("mailcore2", "libEtPan!").
//!
//! The header-section parser here is deliberately small: it handles the fields the
//! FullHeaders fallback asks for, not MIME (tern-mime owns that, and tern-imap cannot depend on
//! it).

use tern_core::{date, rfc2047, Address, MessageHeader};

use crate::parser::Token;

/// `importIMAPEnvelope`. The fields are date, subject, from, sender, reply-to, to, cc, bcc,
/// in-reply-to and message-id. An envelope with more than ten items is taken to have a
/// subject the tokenizer split (an unescaped quote), and the extra items are joined back
/// into it; a short one is padded with NIL.
pub(crate) fn header_from_envelope(items: &[Token]) -> MessageHeader {
    let mut h = MessageHeader::default();
    let nil = Token::Nil;
    let (subject, rest): (Option<Vec<u8>>, &[Token]) = if items.len() > 10 {
        let middle = &items[1..items.len() - 8];
        (Some(join_tokens(middle)), &items[items.len() - 8..])
    } else {
        (
            items.get(1).and_then(Token::bytes).map(<[u8]>::to_vec),
            items.get(2..).unwrap_or(&[]),
        )
    };
    let field = |i: usize| rest.get(i).unwrap_or(&nil);

    if let Some(raw) = items.first().and_then(Token::text) {
        h.date = date::parse_rfc5322(&raw).or_else(|| date::parse_internaldate(&raw));
        h.date_raw = Some(raw);
    }
    if let Some(raw) = subject {
        h.subject = Some(rfc2047::decode(&raw));
        h.subject_raw = Some(raw);
    }
    h.from = address_list(field(0)).into_iter().take(1).collect();
    h.sender = address_list(field(1)).into_iter().take(1).collect();
    h.reply_to = address_list(field(2));
    h.to = address_list(field(3));
    h.cc = address_list(field(4));
    h.bcc = address_list(field(5));
    if let Some(raw) = field(6).bytes() {
        h.in_reply_to = msg_id_list(raw);
    }
    if let Some(raw) = field(7).bytes() {
        h.message_id = msg_id(raw).map(|(id, _)| id);
    }
    h
}

fn join_tokens(tokens: &[Token]) -> Vec<u8> {
    let mut out = vec![];
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        if let Some(b) = t.bytes() {
            out.extend_from_slice(b);
        }
    }
    out
}

/// An ENVELOPE address list. Items that are not lists (`(NIL)`) are skipped; an address with
/// more than four parts is taken to have a split display name, which is rejoined.
fn address_list(tok: &Token) -> Vec<Address> {
    let Some(items) = tok.list() else {
        return vec![];
    };
    items
        .iter()
        .filter_map(Token::list)
        .map(|parts| {
            let nil = Token::Nil;
            let (name, mailbox, host) = if parts.len() > 4 {
                let n = parts.len();
                (
                    Some(join_tokens(&parts[..n - 3])),
                    &parts[n - 2],
                    &parts[n - 1],
                )
            } else {
                (
                    parts.first().and_then(Token::bytes).map(<[u8]>::to_vec),
                    parts.get(2).unwrap_or(&nil),
                    parts.get(3).unwrap_or(&nil),
                )
            };
            imap_address(name.as_deref(), mailbox.bytes(), host.bytes())
        })
        .collect()
}

/// `Address::addressWithIMAPAddress`. A missing host is a group marker: the mailbox part
/// (the group name, or nothing at the end of a group) becomes the address.
fn imap_address(name: Option<&[u8]>, mailbox: Option<&[u8]>, host: Option<&[u8]>) -> Address {
    let lossy = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let email = match (mailbox, host) {
        (m, None) => rfc2047::decode(m.unwrap_or_default()),
        (None, Some(h)) => format!("@{}", lossy(h)),
        (Some(m), Some(h)) => format!("{}@{}", lossy(m), lossy(h)),
    };
    Address {
        name: name.map(rfc2047::decode),
        email,
    }
}

/// `mailimf_msg_id_parse`: an id in angle brackets (a doubled bracket is tolerated), or
/// without them up to the line end. Spaces and tabs inside are dropped. Returns the id and
/// the number of bytes consumed.
pub(crate) fn msg_id(raw: &[u8]) -> Option<(String, usize)> {
    let mut i = skip_cfws(raw, 0);
    let bracketed = raw.get(i) == Some(&b'<');
    if bracketed {
        i += 1;
        if raw.get(i) == Some(&b'<') {
            i += 1;
        }
    }
    let start = i;
    while raw
        .get(i)
        .is_some_and(|b| !matches!(b, b'>' | b'\r' | b'\n'))
    {
        i += 1;
    }
    if i == start {
        return None;
    }
    let id: Vec<u8> = raw[start..i]
        .iter()
        .copied()
        .filter(|b| *b != b' ' && *b != b'\t')
        .collect();
    if bracketed {
        if raw.get(i) != Some(&b'>') {
            return None;
        }
        i += 1;
        if raw.get(i) == Some(&b'>') {
            i += 1;
        }
    }
    Some((String::from_utf8_lossy(&id).into_owned(), i))
}

/// `mailimf_msg_id_list_parse` with `mailimf_unstrict_msg_id_parse`: ids separated by
/// whitespace, commas or stray words (`In-Reply-To: <a@b> (Joe's message)`).
pub(crate) fn msg_id_list(raw: &[u8]) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0;
    while i < raw.len() {
        i = skip_cfws(raw, i);
        match raw[i..].iter().position(|b| *b == b'<') {
            Some(off) => {
                let at = i + off;
                match msg_id(&raw[at..]) {
                    Some((id, used)) => {
                        out.push(id);
                        i = at + used;
                    }
                    None => break,
                }
            }
            None => {
                // No more bracketed ids. A bare id is accepted only as the whole value, as
                // libetpan's unstrict parser does for mbox-style mail.
                let rest = trim(&raw[i..]);
                let bare = !rest.is_empty()
                    && rest.contains(&b'@')
                    && !rest
                        .iter()
                        .any(|b| b.is_ascii_whitespace() || b"(),;".contains(b));
                if out.is_empty() && bare {
                    if let Some((id, _)) = msg_id(rest) {
                        out.push(id);
                    }
                }
                break;
            }
        }
    }
    out
}

/// Skips whitespace and (nested) comments.
fn skip_cfws(raw: &[u8], mut i: usize) -> usize {
    loop {
        while raw.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if raw.get(i) != Some(&b'(') {
            return i;
        }
        let mut depth = 0usize;
        while let Some(&b) = raw.get(i) {
            i += 1;
            match b {
                b'\\' => i += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Header fields of a fetched header section, unfolded, in order.
pub(crate) fn header_fields(block: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut fields: Vec<(String, Vec<u8>)> = vec![];
    for line in block.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            // End of the header block.
            if !fields.is_empty() {
                break;
            }
            continue;
        }
        if line[0] == b' ' || line[0] == b'\t' {
            if let Some((_, value)) = fields.last_mut() {
                value.push(b' ');
                value.extend_from_slice(trim(line));
            }
            continue;
        }
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            continue;
        };
        let name = trim(&line[..colon]);
        if name.is_empty() || name.iter().any(|b| *b <= b' ' || *b >= 127) {
            continue;
        }
        fields.push((
            String::from_utf8_lossy(name).into_owned(),
            trim(&line[colon + 1..]).to_vec(),
        ));
    }
    fields
}

fn trim(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |e| e + 1);
    &s[start..end]
}

/// `importIMAPReferences`: the References of a `HEADER.FIELDS (References)` section.
pub(crate) fn references_from_section(block: &[u8]) -> Vec<String> {
    header_fields(block)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("References"))
        .map(|(_, v)| msg_id_list(&v))
        .unwrap_or_default()
}

/// `importHeadersData` / `importIMFFields`, for the fields the FullHeaders fallback fetches.
pub(crate) fn header_from_section(block: &[u8]) -> MessageHeader {
    let mut h = MessageHeader::default();
    let mut seen: Vec<String> = vec![];
    for (name, value) in header_fields(block) {
        let key = name.to_ascii_lowercase();
        // mailimf_single_fields keeps the first occurrence of each field.
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        match key.as_str() {
            "date" => {
                let raw = String::from_utf8_lossy(&value).into_owned();
                h.date = date::parse_rfc5322(&raw);
                h.date_raw = Some(raw);
            }
            "subject" => {
                h.subject = Some(rfc2047::decode(&value));
                h.subject_raw = Some(value);
            }
            "from" => h.from = parse_addresses(&value).into_iter().take(1).collect(),
            "sender" => h.sender = parse_addresses(&value).into_iter().take(1).collect(),
            "reply-to" => h.reply_to = parse_addresses(&value),
            "to" => h.to = parse_addresses(&value),
            "cc" => h.cc = parse_addresses(&value),
            "bcc" => h.bcc = parse_addresses(&value),
            "message-id" => h.message_id = msg_id(&value).map(|(id, _)| id),
            "references" => h.references = msg_id_list(&value),
            "in-reply-to" => h.in_reply_to = msg_id_list(&value),
            _ => h.extra.push((name, rfc2047::decode(&value))),
        }
    }
    h
}

/// An RFC 5322 address list, groups flattened to their members (libetpan's
/// `lep_address_list_from_lep_addr`).
pub(crate) fn parse_addresses(value: &[u8]) -> Vec<Address> {
    let mut out = vec![];
    for item in split_top_level(value, b",:;") {
        let (piece, sep) = item;
        if sep == Some(b':') {
            // A group's display name: not an address.
            continue;
        }
        if let Some(a) = parse_mailbox(piece) {
            out.push(a);
        }
    }
    out
}

/// Splits on any of `seps` outside quotes, comments and angle brackets, returning each
/// piece with the separator that ended it.
fn split_top_level<'a>(value: &'a [u8], seps: &[u8]) -> Vec<(&'a [u8], Option<u8>)> {
    let mut out = vec![];
    let (mut in_quote, mut comment, mut angle) = (false, 0usize, false);
    let mut start = 0;
    let mut i = 0;
    while i < value.len() {
        let b = value[i];
        match b {
            b'\\' if in_quote || comment > 0 => i += 1,
            b'"' if comment == 0 => in_quote = !in_quote,
            b'(' if !in_quote => comment += 1,
            b')' if !in_quote && comment > 0 => comment -= 1,
            b'<' if !in_quote && comment == 0 => angle = true,
            b'>' if !in_quote && comment == 0 => angle = false,
            _ if !in_quote && comment == 0 && !angle && seps.contains(&b) => {
                out.push((&value[start..i], Some(b)));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push((&value[start.min(value.len())..], None));
    out
}

/// One mailbox: `name <addr>`, `addr (comment)` or a bare `addr`.
fn parse_mailbox(piece: &[u8]) -> Option<Address> {
    let piece = trim(piece);
    if piece.is_empty() {
        return None;
    }
    if let Some(lt) = find_unquoted(piece, b'<') {
        let gt = piece[lt..]
            .iter()
            .position(|b| *b == b'>')
            .map_or(piece.len(), |p| lt + p);
        let addr = String::from_utf8_lossy(trim(&piece[lt + 1..gt]));
        // A source route (`<@a,@b:x@y>`) is dropped.
        let addr = addr.rsplit_once(':').map_or(&*addr, |(_, a)| a).to_owned();
        let display = unquote(trim(&piece[..lt]));
        let name = (!display.is_empty()).then(|| rfc2047::decode(&display));
        return Some(Address { name, email: addr });
    }
    let without_comments = strip_comments(piece);
    let email = String::from_utf8_lossy(trim(&without_comments))
        .split_whitespace()
        .collect::<String>();
    if email.is_empty() {
        return None;
    }
    Some(Address { name: None, email })
}

fn find_unquoted(s: &[u8], needle: u8) -> Option<usize> {
    let mut in_quote = false;
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'\\' if in_quote => i += 1,
            b'"' => in_quote = !in_quote,
            b if b == needle && !in_quote => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn unquote(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'"' => {}
            b'\\' if i + 1 < s.len() => {
                i += 1;
                out.push(s[i]);
            }
            b => out.push(b),
        }
        i += 1;
    }
    out
}

fn strip_comments(s: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut depth = 0usize;
    let mut in_quote = false;
    for &b in s {
        match b {
            b'"' if depth == 0 => {
                in_quote = !in_quote;
                out.push(b);
            }
            b'(' if !in_quote => depth += 1,
            b')' if !in_quote && depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(b),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Tokenizer;

    fn env(raw: &str) -> MessageHeader {
        let mut t = Tokenizer::new(raw.as_bytes());
        let tok = t.next_token().unwrap();
        header_from_envelope(tok.list().unwrap())
    }

    #[test]
    fn envelope() {
        let h = env("(\"Mon, 5 Jan 2026 09:00:00 +0000\" \"=?utf-8?q?caf=C3=A9?=\" ((\"Ann\" NIL \"ann\" \"a.test\")) ((\"Ann\" NIL \"ann\" \"a.test\")) NIL ((NIL NIL \"bob\" \"b.test\") (\"C\" NIL \"c\" \"c.test\")) NIL NIL \"<p@a.test>\" \"<m@a.test>\")");
        assert_eq!(h.date, Some(1767603600));
        assert_eq!(
            h.date_raw.as_deref(),
            Some("Mon, 5 Jan 2026 09:00:00 +0000")
        );
        assert_eq!(h.subject.as_deref(), Some("café"));
        assert_eq!(
            h.subject_raw.as_deref(),
            Some(&b"=?utf-8?q?caf=C3=A9?="[..])
        );
        assert_eq!(h.from, vec![Address::new(Some("Ann"), "ann@a.test")]);
        assert_eq!(h.sender.len(), 1);
        assert!(h.reply_to.is_empty());
        assert_eq!(
            h.to,
            vec![
                Address::new(None, "bob@b.test"),
                Address::new(Some("C"), "c@c.test")
            ]
        );
        assert_eq!(h.in_reply_to, vec!["p@a.test"]);
        assert_eq!(h.message_id.as_deref(), Some("m@a.test"));
    }

    #[test]
    fn lenient_envelopes() {
        // Unescaped quotes in a display name and in the subject.
        let h = env("(NIL \"a \"b\" c\" ((\"Jo \"Q\" Doe\" NIL \"j\" \"d.test\")) NIL NIL NIL NIL NIL NIL NIL)");
        assert_eq!(h.subject.as_deref(), Some("a \"b\" c"));
        assert_eq!(h.from[0].name.as_deref(), Some("Jo \"Q\" Doe"));
        assert_eq!(h.from[0].email, "j@d.test");
        // An address that still splits: the name is rejoined from the extra parts.
        let h = env(
            "(NIL \"x\" ((\"A \"B C\" D E\" NIL \"e\" \"f.test\")) NIL NIL NIL NIL NIL NIL NIL)",
        );
        assert_eq!(h.from[0].email, "e@f.test");
        assert!(h.from[0].name.as_deref().unwrap().starts_with("A \"B C"));
        // (NIL) as an address list, a short envelope, a group.
        let h = env("(NIL \"x\" (NIL) NIL NIL ((NIL NIL \"team\" NIL) (NIL NIL \"a\" \"b.test\") (NIL NIL NIL NIL)))");
        assert!(h.from.is_empty());
        assert_eq!(
            h.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(),
            vec!["team", "a@b.test", ""]
        );
        assert_eq!(h.message_id, None);
        // A quote inside the subject, and a subject split into extra envelope items.
        let h = env("(NIL \"one\" two\" NIL NIL NIL NIL NIL NIL NIL NIL)");
        assert_eq!(h.subject.as_deref(), Some("one\" two"));
        let h = env("(NIL \"a\" \"b\" c NIL NIL NIL NIL NIL NIL NIL \"<m@x>\")");
        assert_eq!(h.subject.as_deref(), Some("a b c"));
        assert_eq!(h.message_id.as_deref(), Some("m@x"));
        // Mailbox without host.
        let h = env("(NIL NIL ((NIL NIL NIL \"h.test\")) NIL NIL NIL NIL NIL NIL NIL)");
        assert_eq!(h.from[0].email, "@h.test");
    }

    #[test]
    fn message_ids() {
        assert_eq!(msg_id(b" <a@b> ").unwrap().0, "a@b");
        assert_eq!(msg_id(b"<<a@b>>").unwrap().0, "a@b");
        assert_eq!(msg_id(b"a@b").unwrap().0, "a@b");
        assert_eq!(msg_id(b"<a @b>").unwrap().0, "a@b");
        assert_eq!(msg_id(b"<a@b"), None);
        assert_eq!(msg_id(b""), None);
        assert_eq!(
            msg_id_list(b"<a@b> <c@d>\r\n\t<e@f>"),
            vec!["a@b", "c@d", "e@f"]
        );
        assert_eq!(msg_id_list(b"<a@b> (Joe's message of Monday)"), vec!["a@b"]);
        assert_eq!(msg_id_list(b"Message from Joe <a@b>"), vec!["a@b"]);
        assert_eq!(msg_id_list(b"bare@id"), vec!["bare@id"]);
        assert!(msg_id_list(b"").is_empty());
        assert!(msg_id_list(b"Joe's message").is_empty());
    }

    #[test]
    fn references_section() {
        let block = b"References: <a@b>\r\n <c@d>\r\n\r\n";
        assert_eq!(references_from_section(block), vec!["a@b", "c@d"]);
        assert!(references_from_section(b"\r\n").is_empty());
    }

    #[test]
    fn full_headers() {
        let block = b"Date: Mon, 05 Jan 2026 09:00:00 +0000\r\nSubject: =?utf-8?b?w6k=?=\r\nFrom: \"Doe, Jo\" <jo@d.test>\r\nTo: a@b.test (A), Team: x@y.test, \"Z\" <z@y.test>;, <c@d.test>\r\nMessage-ID: <m@d.test>\r\nIn-Reply-To: <p@d.test>\r\nReferences: <o@d.test> <p@d.test>\r\nX-Other: v\r\n\r\n";
        let h = header_from_section(block);
        assert_eq!(h.date, Some(1767603600));
        assert_eq!(h.subject.as_deref(), Some("é"));
        assert_eq!(h.from, vec![Address::new(Some("Doe, Jo"), "jo@d.test")]);
        assert_eq!(
            h.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(),
            vec!["a@b.test", "x@y.test", "z@y.test", "c@d.test"]
        );
        assert_eq!(h.message_id.as_deref(), Some("m@d.test"));
        assert_eq!(h.in_reply_to, vec!["p@d.test"]);
        assert_eq!(h.references, vec!["o@d.test", "p@d.test"]);
        assert_eq!(h.extra_value("x-other"), Some("v"));
    }
}
