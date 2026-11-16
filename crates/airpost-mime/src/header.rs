// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Header blocks: splitting, decoding (RFC 2047 and raw 8-bit text) and the
//! [`MessageHeader`] of a full message.
//!
//! Derived from mailcore2 `MessageHeader::importIMFFields` and
//! `String::stringByDecodingMIMEHeaderValue` (Copyright © 2001-2013 MailCore team,
//! BSD-3-Clause) and libetpan's `mailimf_fields_parse` (Copyright © 2001-2005 DINH Viet Hoa,
//! BSD-3-Clause). See NOTICE.md ("mailcore2", "libEtPan!").
//!
//! Structured fields (addresses, message ids, MIME parameters with RFC 2231 continuations)
//! are parsed by `mail-parser`; this module feeds it a UTF-8 copy of the block so raw 8-bit
//! names come out right, and does the unstructured decoding itself.

use std::borrow::Cow;

use mail_parser::{Addr, HeaderName, HeaderValue, MessageParser, MimeHeaders};
use tern_core::{Address, MessageHeader};

use crate::charset;

/// One header field as it appears in the block: the name as sent and the raw value after
/// the colon, folding included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawField {
    pub name: String,
    pub value: Vec<u8>,
}

/// Where a header block ends: `(fields_end, body_start)`. Fields run while lines look like
/// `name:` or continue a field with leading blanks; a blank line or any other line ends
/// them (libetpan `mailimf_envelope_and_optional_fields_parse` + `mailimf_crlf_parse`).
pub(crate) fn split_block(data: &[u8]) -> (usize, usize) {
    let mut pos = 0;
    while pos < data.len() {
        let line_end = data[pos..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(data.len(), |p| pos + p + 1);
        let line = &data[pos..line_end];
        if line == b"\r\n" || line == b"\n" {
            return (pos, line_end);
        }
        let continuation = pos > 0 && (line[0] == b' ' || line[0] == b'\t');
        if !continuation && !is_field_line(line) {
            return (pos, pos);
        }
        pos = line_end;
    }
    (pos, pos)
}

fn is_field_line(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|b| *b == b':') else {
        return false;
    };
    let name = trim_end_blanks(&line[..colon]);
    !name.is_empty() && name.iter().all(|b| (33..=126).contains(b) && *b != b':')
}

fn trim_end_blanks(s: &[u8]) -> &[u8] {
    let mut end = s.len();
    while end > 0 && (s[end - 1] == b' ' || s[end - 1] == b'\t') {
        end -= 1;
    }
    &s[..end]
}

/// The fields of a header block, in order.
pub(crate) fn fields(block: &[u8]) -> Vec<RawField> {
    let mut out: Vec<RawField> = Vec::new();
    let mut pos = 0;
    while pos < block.len() {
        let line_end = block[pos..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(block.len(), |p| pos + p + 1);
        let line = &block[pos..line_end];
        if (line[0] == b' ' || line[0] == b'\t') && !out.is_empty() {
            if let Some(last) = out.last_mut() {
                last.value.extend_from_slice(line);
            }
        } else if let Some(colon) = line.iter().position(|b| *b == b':') {
            let name = String::from_utf8_lossy(trim_end_blanks(&line[..colon])).into_owned();
            out.push(RawField {
                name,
                value: line[colon + 1..].to_vec(),
            });
        }
        pos = line_end;
    }
    out
}

/// Removes line breaks, keeping the blank that starts each continuation line, and trims.
pub(crate) fn unfold(value: &[u8]) -> Vec<u8> {
    let unfolded: Vec<u8> = value
        .iter()
        .copied()
        .filter(|b| *b != b'\r' && *b != b'\n')
        .collect();
    trim_blanks(&unfolded).to_vec()
}

fn trim_blanks(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |p| p + 1);
    &s[start..end]
}

/// `stringByDecodingMIMEHeaderValue`: unfolds, decodes RFC 2047 encoded words (adjacent
/// words are joined before charset decoding and the blanks between them dropped, RFC 2047
/// section 6.2) and turns raw 8-bit text into UTF-8, guessing its charset when it is not
/// valid UTF-8.
pub fn decode_header_value(raw: &[u8]) -> String {
    let value = unfold(raw);
    let segments = split_encoded_words(&value);

    let plain: Vec<u8> = segments
        .iter()
        .filter_map(|s| match s {
            Segment::Text(t) => Some(*t),
            Segment::Word { .. } => None,
        })
        .flatten()
        .copied()
        .collect();
    let plain_encoding = if std::str::from_utf8(&plain).is_ok() {
        None
    } else {
        Some(charset::detect(&plain))
    };
    let decode_plain = |t: &[u8]| -> String {
        match (std::str::from_utf8(t), plain_encoding) {
            (Ok(s), _) => s.to_owned(),
            (Err(_), Some(enc)) => enc.decode_without_bom_handling(t).0.into_owned(),
            (Err(_), None) => charset::latin1(t),
        }
    };

    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < segments.len() {
        match &segments[i] {
            Segment::Text(t) => {
                let between_words = i > 0
                    && matches!(segments[i - 1], Segment::Word { .. })
                    && matches!(segments.get(i + 1), Some(Segment::Word { .. }))
                    && t.iter().all(|b| *b == b' ' || *b == b'\t');
                if !between_words {
                    out.push_str(&decode_plain(t));
                }
                i += 1;
            }
            Segment::Word { charset: cs, .. } => {
                let mut joined = Vec::new();
                let mut j = i;
                while j < segments.len() {
                    match &segments[j] {
                        Segment::Word { charset: c, bytes } if c.eq_ignore_ascii_case(cs) => {
                            joined.extend_from_slice(bytes);
                            j += 1;
                        }
                        Segment::Text(t)
                            if t.iter().all(|b| *b == b' ' || *b == b'\t')
                                && matches!(segments.get(j + 1), Some(Segment::Word { charset: c, .. }) if c.eq_ignore_ascii_case(cs)) =>
                        {
                            j += 1;
                        }
                        _ => break,
                    }
                }
                let label = cs.split('*').next().unwrap_or(cs);
                out.push_str(&charset::decode_text(&joined, Some(label), false));
                i = j;
            }
        }
    }
    out
}

enum Segment<'a> {
    Text(&'a [u8]),
    Word { charset: String, bytes: Vec<u8> },
}

fn split_encoded_words(input: &[u8]) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while i + 1 < input.len() {
        if input[i] == b'=' && input[i + 1] == b'?' {
            if let Some((end, charset, bytes)) = parse_word(input, i) {
                if text_start < i {
                    segments.push(Segment::Text(&input[text_start..i]));
                }
                segments.push(Segment::Word { charset, bytes });
                i = end;
                text_start = end;
                continue;
            }
        }
        i += 1;
    }
    if text_start < input.len() {
        segments.push(Segment::Text(&input[text_start..]));
    }
    segments
}

/// `=?charset?B|Q?text?=` at `start`: the end offset, the charset and the decoded bytes.
fn parse_word(input: &[u8], start: usize) -> Option<(usize, String, Vec<u8>)> {
    let rest = &input[start + 2..];
    let q1 = rest.iter().position(|b| *b == b'?')?;
    let charset = &rest[..q1];
    if charset.is_empty() || charset.iter().any(|b| b.is_ascii_whitespace()) {
        return None;
    }
    let after = &rest[q1 + 1..];
    if after.len() < 2 || after[1] != b'?' {
        return None;
    }
    let kind = after[0].to_ascii_uppercase();
    if kind != b'B' && kind != b'Q' {
        return None;
    }
    let text_and_rest = &after[2..];
    let end_rel = text_and_rest.windows(2).position(|w| w == b"?=")?;
    let text = &text_and_rest[..end_rel];
    let bytes = if kind == b'B' {
        crate::transfer::decode_base64(text)
    } else {
        let spaced: Vec<u8> = text
            .iter()
            .map(|b| if *b == b'_' { b' ' } else { *b })
            .collect();
        crate::transfer::decode_quoted_printable(&spaced)
    };
    let end = start + 2 + q1 + 1 + 2 + end_rel + 2;
    Some((end, String::from_utf8_lossy(charset).into_owned(), bytes))
}

/// The MIME fields of one part, as `Attachment::attachmentWithSingleMIME` reads them.
#[derive(Clone, Debug, Default)]
pub(crate) struct MimeFields {
    /// `type/subtype`, lowercased. `None` when there is no parseable Content-Type.
    pub content_type: Option<(String, String)>,
    /// Content-Type parameters, names lowercased, values decoded.
    pub params: Vec<(String, String)>,
    pub disposition: Option<String>,
    pub disposition_params: Vec<(String, String)>,
    pub transfer_encoding: Option<String>,
    pub content_id: Option<String>,
    pub content_description: Option<String>,
    pub content_location: Option<String>,
    /// The Content-Type `boundary`, read the way libetpan reads it: present even when
    /// empty, and allowing folding around the `=`.
    pub boundary: Option<String>,
}

impl MimeFields {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn disposition_param(&self, name: &str) -> Option<&str> {
        self.disposition_params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// The block in UTF-8, so `mail-parser` decodes raw 8-bit names correctly, followed by
/// the blank line it needs to see the end of the headers.
fn utf8_block(block: &[u8]) -> Vec<u8> {
    let mut out = match std::str::from_utf8(block) {
        Ok(_) => block.to_vec(),
        Err(_) => charset::detect(block)
            .decode_without_bom_handling(block)
            .0
            .into_owned()
            .into_bytes(),
    };
    if !out.ends_with(b"\n") && !out.is_empty() {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out
}

pub(crate) fn mime_fields(block: &[u8], raw: &[RawField]) -> MimeFields {
    let mut out = MimeFields::default();
    if !raw
        .iter()
        .any(|f| f.name.to_ascii_lowercase().starts_with("content-"))
    {
        return out;
    }
    let utf8 = utf8_block(block);
    let Some(parsed) = MessageParser::default().parse_headers(&utf8) else {
        return out;
    };
    let part = parsed.root_part();
    if let Some(ct) = part.content_type() {
        let ctype = ct.ctype().trim().to_ascii_lowercase();
        if !ctype.is_empty() {
            let sub = ct
                .subtype()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".to_owned());
            out.content_type = Some((ctype, sub));
        }
        out.params = attributes(ct.attributes());
    }
    if let Some(cd) = part.content_disposition() {
        out.disposition = Some(cd.ctype().trim().to_ascii_lowercase());
        out.disposition_params = attributes(cd.attributes());
    }
    out.boundary = field(raw, "content-type")
        .and_then(|v| boundary_param(&unfold(&v)))
        .or_else(|| {
            out.params
                .iter()
                .find(|(k, _)| k == "boundary")
                .map(|(_, v)| v.clone())
        });
    out.transfer_encoding = raw_value(raw, "content-transfer-encoding");
    out.content_id = part
        .content_id()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| raw_value(raw, "content-id").map(|v| strip_brackets(&v)));
    out.content_description = field(raw, "content-description").map(|v| decode_header_value(&v));
    out.content_location = raw_value(raw, "content-location");
    out
}

/// The `boundary` parameter of a raw Content-Type value. Whitespace may surround the `=`
/// (`boundary=\r\n "..."` is common), the value may be empty, and empty parameters
/// (`multipart/mixed;; boundary=...`) are skipped where libetpan would stop parsing.
fn boundary_param(value: &[u8]) -> Option<String> {
    let mut i = value.iter().position(|b| *b == b';')?;
    while i < value.len() {
        // At a ';'.
        i += 1;
        while i < value.len() && matches!(value[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        let name_start = i;
        while i < value.len() && !matches!(value[i], b'=' | b';' | b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        let name = &value[name_start..i];
        while i < value.len() && matches!(value[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if value.get(i) != Some(&b'=') {
            i = next_semicolon(value, i);
            continue;
        }
        i += 1;
        while i < value.len() && matches!(value[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        let mut v = Vec::new();
        if value.get(i) == Some(&b'"') {
            i += 1;
            while i < value.len() && value[i] != b'"' {
                if value[i] == b'\\' && i + 1 < value.len() {
                    i += 1;
                }
                v.push(value[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < value.len() && !matches!(value[i], b';' | b' ' | b'\t' | b'\r' | b'\n') {
                v.push(value[i]);
                i += 1;
            }
        }
        if name.eq_ignore_ascii_case(b"boundary") {
            return Some(String::from_utf8_lossy(&v).into_owned());
        }
        i = next_semicolon(value, i);
    }
    None
}

fn next_semicolon(value: &[u8], from: usize) -> usize {
    let mut i = from;
    let mut quoted = false;
    while i < value.len() {
        match value[i] {
            b'"' => quoted = !quoted,
            b'\\' if quoted => i += 1,
            b';' if !quoted => return i,
            _ => {}
        }
        i += 1;
    }
    value.len()
}

fn attributes(attrs: Option<&[mail_parser::Attribute<'_>]>) -> Vec<(String, String)> {
    attrs
        .unwrap_or_default()
        .iter()
        .map(|a| {
            let v: &str = a.value.as_ref();
            let v = if v.contains("=?") {
                decode_header_value(v.as_bytes())
            } else {
                v.to_owned()
            };
            (a.name.to_ascii_lowercase(), v)
        })
        .collect()
}

fn field(raw: &[RawField], name: &str) -> Option<Vec<u8>> {
    raw.iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .map(|f| f.value.clone())
}

fn raw_value(raw: &[RawField], name: &str) -> Option<String> {
    field(raw, name)
        .map(|v| String::from_utf8_lossy(&unfold(&v)).into_owned())
        .filter(|v| !v.is_empty())
}

fn strip_brackets(v: &str) -> String {
    tern_core::ids::strip_angle_brackets(v).to_owned()
}

/// The RFC 5322 fields libetpan parses itself; every other field is an "optional field"
/// and lands in mailcore's `extraHeaders` (MIME fields included).
const STANDARD_FIELDS: &[&str] = &[
    "return-path",
    "received",
    "date",
    "from",
    "sender",
    "reply-to",
    "to",
    "cc",
    "bcc",
    "message-id",
    "in-reply-to",
    "references",
    "subject",
    "comments",
    "keywords",
    "resent-date",
    "resent-from",
    "resent-sender",
    "resent-to",
    "resent-cc",
    "resent-bcc",
    "resent-message-id",
];

/// The [`MessageHeader`] of a message's top-level block.
pub(crate) fn message_header(block: &[u8], raw: &[RawField]) -> MessageHeader {
    let mut header = MessageHeader::default();
    let utf8 = utf8_block(block);
    let parsed = MessageParser::default().parse_headers(&utf8);

    if let Some(date) = field(raw, "date") {
        let date = String::from_utf8_lossy(&unfold(&date)).into_owned();
        header.date = tern_core::date::parse_rfc5322(&date);
        header.date_raw = Some(date);
    }
    if let Some(subject) = field(raw, "subject") {
        let trimmed = trim_blanks(&subject).to_vec();
        header.subject = Some(decode_header_value(&trimmed));
        header.subject_raw = Some(trimmed);
    }
    header.message_id = field(raw, "message-id")
        .map(|v| String::from_utf8_lossy(&unfold(&v)).into_owned())
        .map(|v| first_msg_id(&v))
        .filter(|v| !v.is_empty());

    if let Some(p) = &parsed {
        header.in_reply_to = id_list(p.in_reply_to());
        header.references = id_list(p.references());
        header.from = addresses(p.header(HeaderName::From));
        header.sender = addresses(p.header(HeaderName::Sender));
        header.reply_to = addresses(p.header(HeaderName::ReplyTo));
        header.to = addresses(p.header(HeaderName::To));
        header.cc = addresses(p.header(HeaderName::Cc));
        header.bcc = addresses(p.header(HeaderName::Bcc));
    }

    for f in raw {
        let lower = f.name.to_ascii_lowercase();
        if STANDARD_FIELDS.contains(&lower.as_str()) {
            continue;
        }
        header
            .extra
            .push((f.name.clone(), decode_header_value(&f.value)));
    }
    header
}

/// The first `<id>` of a Message-ID value, or the whole trimmed value when it has none.
fn first_msg_id(v: &str) -> String {
    let v = v.trim();
    if let (Some(open), Some(close)) = (v.find('<'), v.find('>')) {
        if open < close {
            return v[open + 1..close].trim().to_owned();
        }
    }
    tern_core::ids::strip_angle_brackets(v).to_owned()
}

fn id_list(value: &HeaderValue<'_>) -> Vec<String> {
    match value {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(|s| s.to_string()).collect(),
        _ => Vec::new(),
    }
}

fn addresses(value: Option<&HeaderValue<'_>>) -> Vec<Address> {
    let Some(HeaderValue::Address(addr)) = value else {
        return Vec::new();
    };
    addr.iter().filter_map(address).collect()
}

fn address(a: &Addr<'_>) -> Option<Address> {
    let email = a
        .address
        .as_ref()
        .map(|e| e.trim().to_owned())
        .unwrap_or_default();
    let name = a
        .name
        .as_ref()
        .map(|n: &Cow<'_, str>| n.trim().to_owned())
        .filter(|n| !n.is_empty() && *n != email);
    if email.is_empty() && name.is_none() {
        return None;
    }
    Some(Address { name, email })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_blocks() {
        assert_eq!(split_block(b"A: 1\r\nB: 2\r\n\r\nbody"), (12, 14));
        assert_eq!(split_block(b"A: 1\nB: 2\n continued\n\nbody"), (21, 22));
        assert_eq!(split_block(b"\r\nbody"), (0, 2));
        assert_eq!(split_block(b"not a header line\r\nmore"), (0, 0));
        assert_eq!(split_block(b"A: 1\r\nbody without blank\r\n"), (6, 6));
        assert_eq!(split_block(b"A: 1"), (4, 4));
    }

    #[test]
    fn reads_fields() {
        let f = fields(b"Subject: hello\r\n  world\r\nX-Thing : v\r\n");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].name, "Subject");
        assert_eq!(unfold(&f[0].value), b"hello  world");
        assert_eq!(f[1].name, "X-Thing");
    }

    #[test]
    fn decodes_values() {
        let d = |s: &[u8]| decode_header_value(s);
        assert_eq!(d(b" =?UTF-8?B?Q2Fmw6k=?= "), "Café");
        assert_eq!(d(b"=?iso-8859-1?q?caf=E9_cr=E8me?="), "café crème");
        assert_eq!(d(b"=?UTF-8?Q?=C3?= =?UTF-8?Q?=A9?="), "é");
        assert_eq!(d(b"=?utf-8?q?a?= - =?utf-8?q?b?="), "a - b");
        assert_eq!(
            d(b"Re: =?utf-8?q?Gr=C3=BC=C3=9Fe?=\r\n aus Berlin"),
            "Re: Grüße aus Berlin"
        );
        // raw 8-bit text in a legacy charset
        assert_eq!(d(b"\xcf\xf0\xe8\xe2\xe5\xf2 \xec\xe8\xf0"), "Привет мир");
        // raw UTF-8
        assert_eq!(d("Grüße".as_bytes()), "Grüße");
        // an encoded word mislabelled as latin-1 that is really UTF-8
        assert_eq!(d(b"=?iso-8859-1?B?Q2Fmw6k=?="), "Café");
        // GB2312 words
        assert_eq!(d(b"=?gb2312?B?xOO6ww==?="), "你好");
        // unknown charset: the bytes are used as UTF-8
        assert_eq!(d(b"=?x-bogus?Q?ok?="), "ok");
    }

    #[test]
    fn message_header_fields() {
        let block = b"From: =?utf-8?q?J=C3=B6rg?= <jorg@example.com>\r\nTo: a@x.test, \"B, Jr\" <b@x.test>\r\nCc: team: c@x.test, d@x.test;\r\nSubject: =?utf-8?B?SGVsbG8=?=\r\nDate: Mon, 7 Feb 1994 21:52:25 -0800\r\nMessage-ID: <abc@host>\r\nReferences: <r1@h> <r2@h>\r\nIn-Reply-To: <r2@h>\r\nList-Unsubscribe: <mailto:u@x>,\r\n <https://x/u>\r\nX-Priority: 1\r\n";
        let raw = fields(block);
        let h = message_header(block, &raw);
        assert_eq!(h.from, vec![Address::new(Some("Jörg"), "jorg@example.com")]);
        assert_eq!(
            h.to,
            vec![
                Address::new(None, "a@x.test"),
                Address::new(Some("B, Jr"), "b@x.test")
            ]
        );
        assert_eq!(h.cc.len(), 2);
        assert_eq!(h.subject.as_deref(), Some("Hello"));
        assert_eq!(h.subject_raw.as_deref(), Some(&b"=?utf-8?B?SGVsbG8=?="[..]));
        assert_eq!(h.date, Some(760_686_745));
        assert_eq!(h.message_id.as_deref(), Some("abc@host"));
        assert_eq!(h.references, vec!["r1@h", "r2@h"]);
        assert_eq!(h.in_reply_to, vec!["r2@h"]);
        assert_eq!(
            h.extra_value("list-unsubscribe"),
            Some("<mailto:u@x>, <https://x/u>")
        );
        assert_eq!(h.extra_value("X-Priority"), Some("1"));
    }

    #[test]
    fn raw_8bit_display_names() {
        let block = b"From: \xc4\xe3\xba\xc3 <n@x.test>\r\nSubject: \xc4\xe3\xba\xc3\r\n";
        let raw = fields(block);
        let h = message_header(block, &raw);
        assert_eq!(h.from[0].email, "n@x.test");
        assert!(h.from[0]
            .name
            .as_deref()
            .is_some_and(|n| !n.contains('\u{fffd}')));
        assert!(h
            .subject
            .as_deref()
            .is_some_and(|s| !s.contains('\u{fffd}')));
    }

    #[test]
    fn mime_fields_parse() {
        let block = b"Content-Type: Text/HTML; charset=\"ISO-8859-1\"; Name=\"a b.html\"\r\nContent-Disposition: attachment;\r\n filename*=UTF-8''r%C3%A9sum%C3%A9.pdf\r\nContent-ID: <img1@x>\r\nContent-Transfer-Encoding: Base64\r\n";
        let raw = fields(block);
        let m = mime_fields(block, &raw);
        assert_eq!(m.content_type, Some(("text".into(), "html".into())));
        assert_eq!(m.param("charset"), Some("ISO-8859-1"));
        assert_eq!(m.param("name"), Some("a b.html"));
        assert_eq!(m.disposition.as_deref(), Some("attachment"));
        assert_eq!(m.disposition_param("filename"), Some("résumé.pdf"));
        assert_eq!(m.content_id.as_deref(), Some("img1@x"));
        assert_eq!(m.transfer_encoding.as_deref(), Some("Base64"));
    }

    #[test]
    fn mime_fields_encoded_word_filename() {
        let block = b"Content-Type: application/pdf; name=\"=?UTF-8?B?w6l0w6kucGRm?=\"\r\n";
        let raw = fields(block);
        let m = mime_fields(block, &raw);
        assert_eq!(m.param("name"), Some("été.pdf"));
    }
}
