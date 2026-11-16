// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A fetched message (`BODY.PEEK[]`) to what `MailProcessor::retrievedMessageBody` stores:
//! the body, the plaintext flag, the snippet and search text, the files and the extra
//! headers.
//!
//! Derived from Mailspring-Sync `MailProcessor.cpp:334-503` and `Models/File.cpp:23-66`
//! (Copyright © 2017 Foundry 376, GPL-3.0).

use tern_core::{MessageHeader, Result};
use tracing::info;

use crate::files::default_attachment_name;
use crate::flatten::flatten_for_snippet;
use crate::mime::{parse_tree, Part};
use crate::render::render;
use crate::text::truncate_utf16;

/// Snippet length in UTF-16 code units (`MailProcessor.cpp:504`).
pub const SNIPPET_LENGTH: usize = 400;

/// Search text appended per message, in UTF-16 code units (`appendToThreadSearchContent`).
pub const SEARCH_TEXT_LENGTH: usize = 5000;

/// A part that becomes a `File`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedPart {
    /// mailcore's partID: `1` for a single top-level part, `idx+1` joined with `.` inside
    /// multiparts, `message/rfc822` adding a `.1` before a single main part.
    pub part_id: String,
    /// Breadth-first number of single parts from the root, `0, 1, …`.
    pub unique_id: String,
    /// The part's name, or the default name for its type (`File.cpp:40-65`).
    pub filename: String,
    /// `type/subtype`, lowercased.
    pub mime_type: String,
    /// Set when the part is inline and has a Content-ID, or when the body references
    /// `cid:<filename>` and the part has no Content-ID of its own.
    pub content_id: Option<String>,
    /// `Content-Disposition: inline`, or a Content-ID the body references as `cid:`.
    pub inline: bool,
    pub data: Vec<u8>,
}

impl ParsedPart {
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Whether the file counts toward the thread's `attachmentCount`.
    pub fn counts_as_attachment(&self) -> bool {
        crate::files::counts_as_attachment(self.content_id.as_deref(), self.data.len())
    }
}

/// Everything `MailProcessor` takes from a message body.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedMessage {
    pub header: MessageHeader,
    /// The rendered body: HTML, or the plain text when [`ParsedMessage::plaintext`].
    pub body: String,
    /// No HTML part was rendered; the body is stored with `plaintext=true`.
    pub plaintext: bool,
    /// The text the snippet and search index are cut from: the plain body as it is, or the
    /// HTML flattened with whitespace collapsed.
    pub text: String,
    /// The files, duplicates by partID removed, in the C++ order: attachments met while
    /// rendering, then the inline parts of `multipart/related`.
    pub attachments: Vec<ParsedPart>,
    /// `hListUnsub`.
    pub list_unsubscribe: Option<String>,
    /// `hListUnsubPost`.
    pub list_unsubscribe_post: Option<String>,
    /// `hImportance`: `high`, `low` or `normal`, from Importance, else X-Priority, else
    /// X-MSMail-Priority.
    pub importance: Option<String>,
}

impl ParsedMessage {
    /// The first 400 UTF-16 units of [`ParsedMessage::text`].
    pub fn snippet(&self) -> String {
        snippet(&self.text)
    }

    /// The first 5000 UTF-16 units of [`ParsedMessage::text`].
    pub fn search_text(&self) -> String {
        search_text(&self.text)
    }
}

/// Parses and renders a full RFC 5322 message.
pub fn parse_message(raw: &[u8]) -> Result<ParsedMessage> {
    let tree = parse_tree(raw);
    let rendering = render(&tree.root);

    let text = if rendering.plaintext {
        rendering.body.clone()
    } else {
        flatten_for_snippet(&rendering.body)
    };

    let mut files: Vec<ParsedPart> = Vec::new();
    for part in rendering.attachments.iter().chain(rendering.related.iter()) {
        let file = file_for_part(part, &rendering.body);
        if files.iter().any(|f| f.part_id == file.part_id) {
            info!(
                "Attachment is duplicate: partId {} filename {}",
                file.part_id, file.filename
            );
            continue;
        }
        files.push(file);
    }

    let header = tree.header;
    let list_unsubscribe = header.extra_value("List-Unsubscribe").map(str::to_owned);
    let list_unsubscribe_post = header
        .extra_value("List-Unsubscribe-Post")
        .map(str::to_owned);
    let importance = importance(&header);

    Ok(ParsedMessage {
        header,
        body: rendering.body,
        plaintext: rendering.plaintext,
        text,
        attachments: files,
        list_unsubscribe,
        list_unsubscribe_post,
        importance,
    })
}

/// `File(message, attachment)` plus the two Content-ID repairs of `retrievedMessageBody`.
fn file_for_part(part: &Part, body: &str) -> ParsedPart {
    let mut inline = part.inline;
    // A Content-ID without an inline disposition: when the body references it, the MIME
    // lacked the disposition and the client should still render it inline.
    if let Some(cid) = &part.content_id {
        if !inline && body.contains(&format!("cid:{cid}")) {
            inline = true;
        }
    }
    let mime_type = part
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let filename = part
        .filename
        .clone()
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| default_attachment_name(&mime_type));
    let mut content_id = if inline {
        part.content_id.clone()
    } else {
        None
    };
    // Bodies sometimes reference `cid:filename.png@garbage` while the part has no
    // Content-ID; the client only matches Content-IDs, so the filename stands in.
    if content_id.is_none() && body.contains(&format!("cid:{filename}")) {
        content_id = Some(filename.clone());
    }
    ParsedPart {
        part_id: part.part_id.clone(),
        unique_id: part.unique_id.clone().unwrap_or_default(),
        filename,
        mime_type,
        content_id,
        inline,
        data: part.data.clone(),
    }
}

/// Lowercased and trimmed of ASCII whitespace (`lowerTrimmed`).
fn lower_trimmed(raw: &str) -> String {
    raw.trim_matches(|c: char| c.is_ascii_whitespace())
        .to_ascii_lowercase()
}

/// `classifyImportanceText`: Importance / X-MSMail-Priority values, by first token so
/// `abnormal` is not `normal`; the MAPI `above normal` / `below normal` pass through some
/// gateways.
fn classify_importance_text(raw: &str) -> Option<&'static str> {
    let v = lower_trimmed(raw);
    if v.is_empty() {
        return None;
    }
    if v.starts_with("above normal") {
        return Some("high");
    }
    if v.starts_with("below normal") {
        return Some("low");
    }
    let token = v.split([' ', '\t', '(', ',', ';']).next().unwrap_or("");
    match token {
        "high" => Some("high"),
        "low" => Some("low"),
        "normal" => Some("normal"),
        _ => None,
    }
}

/// `classifyXPriority`: RFC 2076's 1-5 scale by its first digit.
fn classify_x_priority(raw: &str) -> Option<&'static str> {
    match lower_trimmed(raw).chars().next()? {
        '1' | '2' => Some("high"),
        '3' => Some("normal"),
        '4' | '5' => Some("low"),
        _ => None,
    }
}

/// Importance wins outright when present, even with an unrecognised value, so a
/// lower-precedence header cannot contradict it.
fn importance(header: &MessageHeader) -> Option<String> {
    if let Some(h) = header.extra_value("Importance") {
        return classify_importance_text(h).map(str::to_owned);
    }
    header
        .extra_value("X-Priority")
        .and_then(classify_x_priority)
        .or_else(|| {
            header
                .extra_value("X-MSMail-Priority")
                .and_then(classify_importance_text)
        })
        .map(str::to_owned)
}

/// MailUtils' `flattenHTML` as the snippet and search text use it: links get ` (url)`,
/// `<blockquote type=cite>` is dropped, whitespace is collapsed.
pub fn flatten_html(html: &str) -> String {
    flatten_for_snippet(html)
}

/// The first 400 UTF-16 units of the text.
pub fn snippet(text: &str) -> String {
    truncate_utf16(text, SNIPPET_LENGTH).to_owned()
}

/// The first 5000 UTF-16 units of the text, for ThreadSearch.
pub fn search_text(text: &str) -> String {
    truncate_utf16(text, SEARCH_TEXT_LENGTH).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importance_precedence() {
        let h = |pairs: &[(&str, &str)]| MessageHeader {
            extra: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            importance(&h(&[("Importance", "High")])).as_deref(),
            Some("high")
        );
        assert_eq!(
            importance(&h(&[("Importance", "abnormal"), ("X-Priority", "1")])),
            None
        );
        assert_eq!(
            importance(&h(&[("X-Priority", "5 (Lowest)")])).as_deref(),
            Some("low")
        );
        assert_eq!(
            importance(&h(&[
                ("X-Priority", "urgent"),
                ("X-MSMail-Priority", "High")
            ]))
            .as_deref(),
            Some("high")
        );
        assert_eq!(
            importance(&h(&[("Importance", " Above Normal ")])).as_deref(),
            Some("high")
        );
        assert_eq!(
            importance(&h(&[("X-Priority", "3")])).as_deref(),
            Some("normal")
        );
        assert_eq!(importance(&h(&[("X-Priority", "7")])), None);
        assert_eq!(
            importance(&h(&[("Importance", "low; reason")])).as_deref(),
            Some("low")
        );
        assert_eq!(importance(&h(&[])), None);
    }

    #[test]
    fn snippet_lengths() {
        let long = "x".repeat(6000);
        assert_eq!(snippet(&long).len(), 400);
        assert_eq!(search_text(&long).len(), 5000);
    }
}
