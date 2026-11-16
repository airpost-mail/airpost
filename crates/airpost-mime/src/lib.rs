// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! MIME: parse a fetched message into what the store keeps, and build outgoing mail.
//!
//! Derived from Mailspring-Sync (`MailProcessor.cpp` body handling, `Models/File.cpp`,
//! `TaskProcessor.cpp` send and RSVP paths), Copyright © 2017 Foundry 376, GPL-3.0, and
//! mailcore2 (`MCMessageParser`, `MCHTMLRenderer`, `MCAttachment`, `MCAbstractPart`,
//! `MCMessageBuilder`, `MCString::flattenHTML`, `MCData` charset detection), Copyright ©
//! 2001-2013 MailCore team, BSD-3-Clause, and the libetpan MIME code under it, Copyright ©
//! 2001-2005 DINH Viet Hoa, BSD-3-Clause. See NOTICE.md ("mailcore2", "libEtPan!").
//!
//! - [`parse_message`] turns `BODY.PEEK[]` bytes into a [`ParsedMessage`].
//! - [`mime::parse_tree`] exposes the part tree with mailcore's partID/uniqueID numbering.
//! - [`build`] writes an [`OutgoingMessage`] for SendDraft and EventRSVP.

pub mod builder;
mod charset;
mod files;
mod flatten;
mod header;
pub mod mime;
mod parse;
mod render;
mod text;
mod transfer;

pub use builder::{
    build, build_filtering_bcc, OutgoingAttachment, OutgoingBody, OutgoingMessage, USER_AGENT,
};
pub use files::{
    counts_as_attachment, default_attachment_name, mime_type_for_filename, safe_filename,
    INLINE_IMAGE_COUNT_THRESHOLD,
};
pub use flatten::flatten_html_with;
pub use header::decode_header_value;
pub use parse::{
    flatten_html, parse_message, search_text, snippet, ParsedMessage, ParsedPart,
    SEARCH_TEXT_LENGTH, SNIPPET_LENGTH,
};
pub use text::{strip_whitespace, truncate_utf16};

/// Decodes text bytes in a declared charset, recovering from mislabelled charsets
/// (`Data::stringWithDetectedCharset`).
pub fn decode_text(data: &[u8], charset: Option<&str>, is_html: bool) -> String {
    charset::decode_text(data, charset, is_html)
}
