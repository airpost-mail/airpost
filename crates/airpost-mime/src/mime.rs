// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The MIME part tree of a message, shaped and numbered the way mailcore builds it.
//!
//! Derived from mailcore2 `Attachment::attachmentsWithMIME` / `attachmentWithSingleMIME`
//! (`MCAttachment.cpp`), `MessageParser::setBytes` / `setupPartID` (`MCMessageParser.cpp`)
//! and `AbstractPart::applyUniquePartID` (`MCAbstractPart.cpp:311-337`), and from
//! libetpan's `mailmime_parse_with_default` (Copyright © 2001-2013 MailCore team and ©
//! 2001-2005 DINH Viet Hoa, BSD-3-Clause; see NOTICE.md, "mailcore2" and "libEtPan!"):
//! - `multipart/*` without a boundary is a single part;
//! - `message/rfc822` under base64 or quoted-printable is a single part (an attachment);
//! - children of `multipart/digest` default to `message/rfc822`;
//! - a multipart whose first delimiter is missing has no children.

use airpost_core::MessageHeader;

use crate::header::{self, MimeFields};
use crate::transfer::TransferEncoding;

/// Nesting beyond this depth is kept as an opaque single part.
const MAX_DEPTH: usize = 64;

/// mailcore's `PartType`. Every multipart subtype other than related, alternative and
/// signed is mixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Single,
    Message,
    Mixed,
    Related,
    Alternative,
    Signed,
}

/// One node of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub kind: PartKind,
    /// `type/subtype`, lowercased. Set on single parts only, as in mailcore.
    pub mime_type: Option<String>,
    /// IMAP-style part number (`MessageParser::setupPartID`).
    pub part_id: String,
    /// Breadth-first number of single parts (`applyUniquePartID`); `None` elsewhere.
    pub unique_id: Option<String>,
    /// Content-Disposition `filename`, else Content-Type `name`, decoded.
    pub filename: Option<String>,
    pub charset: Option<String>,
    /// Content-ID without angle brackets.
    pub content_id: Option<String>,
    pub content_description: Option<String>,
    pub content_location: Option<String>,
    /// `Content-Disposition: inline`.
    pub inline: bool,
    /// `Content-Disposition: attachment`.
    pub attachment: bool,
    /// The transfer-decoded body of a single part.
    pub data: Vec<u8>,
    /// Children of a multipart; the one main part of a message part.
    pub parts: Vec<Part>,
    /// The embedded message's header, for message parts.
    pub header: Option<Box<MessageHeader>>,
}

impl Part {
    fn new(kind: PartKind) -> Self {
        Part {
            kind,
            mime_type: None,
            part_id: String::new(),
            unique_id: None,
            filename: None,
            charset: None,
            content_id: None,
            content_description: None,
            content_location: None,
            inline: false,
            attachment: false,
            data: Vec::new(),
            parts: Vec::new(),
            header: None,
        }
    }

    pub fn is_multipart(&self) -> bool {
        matches!(
            self.kind,
            PartKind::Mixed | PartKind::Related | PartKind::Alternative | PartKind::Signed
        )
    }

    /// The single parts below this one, depth first.
    pub fn leaves(&self) -> Vec<&Part> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves<'a>(&'a self, out: &mut Vec<&'a Part>) {
        if self.kind == PartKind::Single {
            out.push(self);
        }
        for p in &self.parts {
            p.collect_leaves(out);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefaultType {
    TextPlain,
    Message,
}

/// A parsed message: its header and its main part (mailcore's `mainPart`).
#[derive(Clone, Debug)]
pub struct MimeTree {
    pub header: MessageHeader,
    pub root: Part,
}

/// Parses a full message. An mbox `From ` line in front of the headers is skipped
/// (`MessageParser::setBytes`).
pub fn parse_tree(raw: &[u8]) -> MimeTree {
    let raw = skip_mbox_from_line(raw);
    let (header, fields, body) = read_entity_header(raw, true);
    let mut root = entity(&fields, body, DefaultType::TextPlain, 0);
    assign_part_ids(&mut root, "");
    assign_unique_ids(&mut root);
    MimeTree {
        header: header.unwrap_or_default(),
        root,
    }
}

fn skip_mbox_from_line(raw: &[u8]) -> &[u8] {
    if raw.len() > 5 && raw.starts_with(b"From ") {
        if let Some(nl) = raw.iter().position(|b| *b == b'\n') {
            return &raw[nl + 1..];
        }
    }
    raw
}

fn read_entity_header(
    data: &[u8],
    with_message_header: bool,
) -> (Option<MessageHeader>, MimeFields, &[u8]) {
    let (fields_end, body_start) = header::split_block(data);
    let block = &data[..fields_end];
    let raw = header::fields(block);
    let mime = header::mime_fields(block, &raw);
    let msg_header = with_message_header.then(|| header::message_header(block, &raw));
    (msg_header, mime, &data[body_start..])
}

fn entity(fields: &MimeFields, body: &[u8], default: DefaultType, depth: usize) -> Part {
    let (ctype, subtype) = match (&fields.content_type, default) {
        (Some((t, s)), _) => (t.as_str(), s.as_str()),
        (None, DefaultType::TextPlain) => ("text", "plain"),
        (None, DefaultType::Message) => ("message", "rfc822"),
    };
    let encoding = TransferEncoding::from_header(fields.transfer_encoding.as_deref());

    if depth < MAX_DEPTH && ctype == "multipart" {
        if let Some(boundary) = fields.boundary.as_deref() {
            let kind = match subtype {
                "alternative" => PartKind::Alternative,
                "related" => PartKind::Related,
                "signed" => PartKind::Signed,
                _ => PartKind::Mixed,
            };
            let child_default = if subtype == "digest" {
                DefaultType::Message
            } else {
                DefaultType::TextPlain
            };
            let mut part = Part::new(kind);
            for child in split_multipart(body, boundary.as_bytes()) {
                let (_, child_fields, child_body) = read_entity_header(child, false);
                part.parts
                    .push(entity(&child_fields, child_body, child_default, depth + 1));
            }
            return part;
        }
    }

    let is_message = ctype == "message" && subtype == "rfc822";
    if depth < MAX_DEPTH
        && is_message
        && !matches!(
            encoding,
            TransferEncoding::Base64 | TransferEncoding::QuotedPrintable
        )
    {
        let (inner_header, inner_fields, inner_body) = read_entity_header(body, true);
        let mut part = Part::new(PartKind::Message);
        part.header = inner_header.map(Box::new);
        part.parts.push(entity(
            &inner_fields,
            inner_body,
            DefaultType::TextPlain,
            depth + 1,
        ));
        return part;
    }

    single(fields, ctype, subtype, encoding.decode(body))
}

fn single(fields: &MimeFields, ctype: &str, subtype: &str, data: Vec<u8>) -> Part {
    let mut part = Part::new(PartKind::Single);
    part.mime_type = Some(format!("{ctype}/{subtype}"));
    part.data = data;
    part.filename = fields
        .disposition_param("filename")
        .or_else(|| fields.param("name"))
        .map(str::to_owned)
        .filter(|f| !f.is_empty());
    part.charset = fields
        .param("charset")
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty());
    part.content_id = fields.content_id.clone();
    part.content_description = fields.content_description.clone();
    part.content_location = fields.content_location.clone();
    match fields.disposition.as_deref() {
        Some("inline") => part.inline = true,
        Some("attachment") => part.attachment = true,
        _ => {}
    }
    part
}

/// The bodies between the delimiter lines of a multipart. A delimiter is `--boundary` at
/// the start of a line followed by blanks and a line break, or `--` for the close
/// delimiter; each body excludes the line break before the next delimiter. Without a
/// close delimiter the last body runs to the end.
fn split_multipart<'a>(body: &'a [u8], boundary: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let Some(mut delim) = find_delimiter(body, 0, boundary) else {
        return parts;
    };
    // libetpan skips every blank line after the first delimiter, and one after each later
    // delimiter (`mailmime_multipart_next_parse`), so a part may start with its headers
    // after a stray blank line.
    delim.next = skip_blank_lines(body, delim.next, usize::MAX);
    loop {
        if delim.is_close {
            break;
        }
        let start = delim.next;
        match find_delimiter(body, start, boundary) {
            Some(mut next) => {
                parts.push(&body[start..line_break_start(body, start, next.start)]);
                next.next = skip_blank_lines(body, next.next, 1);
                delim = next;
            }
            None => {
                parts.push(&body[start.min(body.len())..]);
                break;
            }
        }
    }
    parts
}

fn skip_blank_lines(body: &[u8], mut at: usize, max: usize) -> usize {
    for _ in 0..max {
        let mut j = at;
        while j < body.len() && (body[j] == b' ' || body[j] == b'\t') {
            j += 1;
        }
        if body[j..].starts_with(b"\r\n") {
            at = j + 2;
        } else if body[j..].starts_with(b"\n") || body[j..].starts_with(b"\r") {
            at = j + 1;
        } else {
            break;
        }
    }
    at
}

struct Delimiter {
    /// Offset of the `--`.
    start: usize,
    /// Offset just after the delimiter line.
    next: usize,
    is_close: bool,
}

fn find_delimiter(body: &[u8], from: usize, boundary: &[u8]) -> Option<Delimiter> {
    let mut i = from;
    while i + 2 + boundary.len() <= body.len() {
        let at_line_start = i == 0 || body[i - 1] == b'\n';
        if at_line_start && body[i..].starts_with(b"--") && body[i + 2..].starts_with(boundary) {
            let mut j = i + 2 + boundary.len();
            let is_close = body[j..].starts_with(b"--");
            if is_close {
                j += 2;
            }
            while j < body.len() && (body[j] == b' ' || body[j] == b'\t') {
                j += 1;
            }
            let at_eol = j == body.len() || body[j] == b'\r' || body[j] == b'\n';
            if is_close || at_eol {
                let next = body[j..]
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(body.len(), |p| j + p + 1);
                return Some(Delimiter {
                    start: i,
                    next,
                    is_close,
                });
            }
        }
        i = match body[i..].iter().position(|b| *b == b'\n') {
            Some(p) => i + p + 1,
            None => break,
        };
    }
    None
}

/// Where the line break before the delimiter at `delim` begins, not before `start`.
fn line_break_start(body: &[u8], start: usize, delim: usize) -> usize {
    let mut end = delim;
    if end > start && body[end - 1] == b'\n' {
        end -= 1;
        if end > start && body[end - 1] == b'\r' {
            end -= 1;
        }
    }
    end.max(start)
}

/// `MessageParser::recursiveSetupPartIDWithPart`.
fn assign_part_ids(part: &mut Part, prefix: &str) {
    match part.kind {
        PartKind::Single => {
            part.part_id = if prefix.is_empty() {
                "1".into()
            } else {
                prefix.into()
            };
        }
        PartKind::Message => {
            let own = if prefix.is_empty() {
                "1".to_owned()
            } else {
                prefix.to_owned()
            };
            part.part_id = own.clone();
            if let Some(main) = part.parts.first_mut() {
                let child_prefix = match main.kind {
                    PartKind::Single | PartKind::Message => format!("{own}.1"),
                    _ => own,
                };
                assign_part_ids(main, &child_prefix);
            }
        }
        _ => {
            part.part_id = prefix.to_owned();
            for (idx, child) in part.parts.iter_mut().enumerate() {
                let id = if prefix.is_empty() {
                    (idx + 1).to_string()
                } else {
                    format!("{prefix}.{}", idx + 1)
                };
                assign_part_ids(child, &id);
            }
        }
    }
}

/// `AbstractPart::applyUniquePartID`: single parts numbered breadth-first from 0.
fn assign_unique_ids(root: &mut Part) {
    let mut queue: std::collections::VecDeque<&mut Part> = std::collections::VecDeque::new();
    queue.push_back(root);
    let mut next = 0u32;
    while let Some(part) = queue.pop_front() {
        if part.kind == PartKind::Single {
            part.unique_id = Some(next.to_string());
            next += 1;
        } else {
            for child in part.parts.iter_mut() {
                queue.push_back(child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(p: &Part) -> String {
        match p.kind {
            PartKind::Single => format!(
                "{}#{}@{}",
                p.mime_type.as_deref().unwrap_or("?"),
                p.unique_id.as_deref().unwrap_or("?"),
                p.part_id
            ),
            _ => {
                let inner: Vec<String> = p.parts.iter().map(shape).collect();
                format!("{:?}@{}[{}]", p.kind, p.part_id, inner.join(","))
            }
        }
    }

    #[test]
    fn single_part() {
        let t = parse_tree(b"Subject: x\r\n\r\nhello\r\n");
        assert_eq!(shape(&t.root), "text/plain#0@1");
        assert_eq!(t.root.data, b"hello\r\n");
    }

    #[test]
    fn rfc3501_part_numbering() {
        // RFC 3501 section 6.4.5's example structure.
        let raw = b"Content-Type: multipart/mixed; boundary=x\r\n\r\n\
--x\r\nContent-Type: text/plain\r\n\r\npart 1\r\n\
--x\r\nContent-Type: application/octet-stream\r\n\r\npart 2\r\n\
--x\r\nContent-Type: message/rfc822\r\n\r\nSubject: inner\r\nContent-Type: multipart/mixed; boundary=y\r\n\r\n\
--y\r\nContent-Type: text/plain\r\n\r\npart 3.1\r\n\
--y\r\nContent-Type: application/octet-stream\r\n\r\npart 3.2\r\n\
--y--\r\n\
--x\r\nContent-Type: image/gif\r\n\r\npart 4.1\r\n\
--x--\r\n";
        let t = parse_tree(raw);
        assert_eq!(
            shape(&t.root),
            "Mixed@[text/plain#0@1,application/octet-stream#1@2,Message@3[Mixed@3[text/plain#3@3.1,application/octet-stream#4@3.2]],image/gif#2@4]"
        );
        let p31 = &t.root.parts[2].parts[0].parts[0];
        assert_eq!(p31.data, b"part 3.1");
        assert_eq!(
            t.root.parts[2].header.as_ref().unwrap().subject.as_deref(),
            Some("inner")
        );
    }

    #[test]
    fn message_with_single_body_adds_dot_one() {
        let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nintro\r\n--b\r\nContent-Type: message/rfc822\r\n\r\nSubject: fwd\r\n\r\ninner text\r\n--b--\r\n";
        let t = parse_tree(raw);
        assert_eq!(
            shape(&t.root),
            "Mixed@[text/plain#0@1,Message@2[text/plain#1@2.1]]"
        );
    }

    #[test]
    fn libetpan_shape_rules() {
        // no boundary: a single part
        let t = parse_tree(b"Content-Type: multipart/mixed\r\n\r\nbody");
        assert_eq!(shape(&t.root), "multipart/mixed#0@1");
        // base64 message/rfc822 stays an attachment
        let t = parse_tree(b"Content-Type: message/rfc822\r\nContent-Transfer-Encoding: base64\r\n\r\nU3ViamVjdDogeA0KDQpoaQ==\r\n");
        assert_eq!(shape(&t.root), "message/rfc822#0@1");
        assert_eq!(t.root.data, b"Subject: x\r\n\r\nhi");
        // digest children default to message/rfc822
        let t = parse_tree(b"Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n\r\nSubject: one\r\n\r\nbody one\r\n--d--\r\n");
        assert_eq!(shape(&t.root), "Mixed@[Message@1[text/plain#0@1.1]]");
        // missing first delimiter: no children
        let t =
            parse_tree(b"Content-Type: multipart/mixed; boundary=zz\r\n\r\nno delimiters here\r\n");
        assert_eq!(shape(&t.root), "Mixed@[]");
        // top-level message/rfc822
        let t = parse_tree(b"Content-Type: message/rfc822\r\n\r\nSubject: in\r\n\r\nx");
        assert_eq!(shape(&t.root), "Message@1[text/plain#0@1.1]");
    }

    #[test]
    fn delimiters() {
        let raw = b"Content-Type: multipart/alternative; boundary=\"=_a\"\r\n\r\npreamble\r\n--=_a \r\n\r\none\r\n--=_ab\r\nstill one\r\n--=_a\r\n\r\ntwo\r\n--=_a--  \r\nepilogue\r\n";
        let t = parse_tree(raw);
        assert_eq!(t.root.parts.len(), 2);
        assert_eq!(t.root.parts[0].data, b"one\r\n--=_ab\r\nstill one");
        assert_eq!(t.root.parts[1].data, b"two");
        // no close delimiter, LF line endings
        let t = parse_tree(b"Content-Type: multipart/mixed; boundary=q\n\n--q\n\nlast part\n");
        assert_eq!(t.root.parts.len(), 1);
        assert_eq!(t.root.parts[0].data, b"last part\n");
    }

    #[test]
    fn unique_ids_are_breadth_first() {
        let raw = b"Content-Type: multipart/mixed; boundary=o\r\n\r\n--o\r\nContent-Type: multipart/alternative; boundary=i\r\n\r\n--i\r\n\r\nplain\r\n--i\r\nContent-Type: text/html\r\n\r\n<b>x</b>\r\n--i--\r\n--o\r\nContent-Type: image/png\r\n\r\npng\r\n--o--\r\n";
        let t = parse_tree(raw);
        assert_eq!(
            shape(&t.root),
            "Mixed@[Alternative@1[text/plain#1@1.1,text/html#2@1.2],image/png#0@2]"
        );
    }

    #[test]
    fn mbox_from_line() {
        let t = parse_tree(b"From someone Mon Jan  1 00:00:00 2001\nSubject: s\n\nbody\n");
        assert_eq!(t.header.subject.as_deref(), Some("s"));
    }
}
