// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! File naming: default names for unnamed parts, the on-disk name, MIME types from
//! extensions, and the attachment-count rule.
//!
//! Derived from Mailspring-Sync `Models/File.cpp:40-65, 92-95` (Copyright © 2017 Foundry
//! 376, GPL-3.0) and mailcore2 `Attachment::mimeTypeForFilename` (Copyright © 2001-2013
//! MailCore team, BSD-3-Clause). See NOTICE.md ("mailcore2").

/// Files at or below this size with a Content-ID are treated as inline images and not
/// counted in a thread's `attachmentCount` (`Message::fileCountForThreadList`,
/// `Message.cpp:331-338`).
pub const INLINE_IMAGE_COUNT_THRESHOLD: usize = 12 * 1024;

/// The name `File` gives a part without one (`File.cpp:40-65`).
pub fn default_attachment_name(mime_type: &str) -> String {
    match mime_type {
        "text/calendar" => "Event.ics",
        "image/png" | "image/x-png" => "Unnamed Image.png",
        "image/jpg" | "image/jpeg" => "Unnamed Image.jpg",
        "image/gif" => "Unnamed Image.gif",
        "message/delivery-status" => "Delivery Status.txt",
        "message/feedback-report" => "Feedback Report.txt",
        _ => "Unnamed Attachment",
    }
    .to_owned()
}

/// `File::safeFilename`: each of `\ / : | ? * > < " #` becomes `-`.
pub fn safe_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '|' | '?' | '*' | '>' | '<' | '"' | '#' => '-',
            c => c,
        })
        .collect()
}

/// Whether a file counts toward `attachmentCount`: it has no Content-ID, or it is larger
/// than [`INLINE_IMAGE_COUNT_THRESHOLD`].
pub fn counts_as_attachment(content_id: Option<&str>, size: usize) -> bool {
    content_id.is_none() || size > INLINE_IMAGE_COUNT_THRESHOLD
}

/// The MIME type for a file name's extension, `application/octet-stream` when unknown.
/// mailcore read `/etc/apache2/mime.types`, which only macOS ships; this is the common
/// subset of that table.
pub fn mime_type_for_filename(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "heic" => "image/heic",
        "html" | "htm" => "text/html",
        "txt" | "text" | "log" => "text/plain",
        "csv" => "text/csv",
        "ics" | "ifb" => "text/calendar",
        "vcf" => "text/vcard",
        "css" => "text/css",
        "xml" => "application/xml",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "rtf" => "application/rtf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "tar" => "application/x-tar",
        "doc" | "dot" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" | "pps" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "key" => "application/vnd.apple.keynote",
        "pages" => "application/vnd.apple.pages",
        "numbers" => "application/vnd.apple.numbers",
        "eml" => "message/rfc822",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/x-wav",
        "ogg" => "audio/ogg",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "webm" => "video/webm",
        "js" => "application/javascript",
        "p7s" => "application/pkcs7-signature",
        "asc" | "sig" => "application/pgp-signature",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names() {
        assert_eq!(default_attachment_name("text/calendar"), "Event.ics");
        assert_eq!(default_attachment_name("image/x-png"), "Unnamed Image.png");
        assert_eq!(default_attachment_name("image/jpg"), "Unnamed Image.jpg");
        assert_eq!(default_attachment_name("image/gif"), "Unnamed Image.gif");
        assert_eq!(
            default_attachment_name("message/delivery-status"),
            "Delivery Status.txt"
        );
        assert_eq!(
            default_attachment_name("message/feedback-report"),
            "Feedback Report.txt"
        );
        assert_eq!(
            default_attachment_name("application/pdf"),
            "Unnamed Attachment"
        );
    }

    #[test]
    fn safe_names() {
        assert_eq!(
            safe_filename(r#"a\b/c:d|e?f*g>h<i"j#k.txt"#),
            "a-b-c-d-e-f-g-h-i-j-k.txt"
        );
        assert_eq!(safe_filename("résumé (1).pdf"), "résumé (1).pdf");
    }

    #[test]
    fn attachment_count_rule() {
        assert!(counts_as_attachment(None, 10));
        assert!(!counts_as_attachment(Some("cid"), 12 * 1024));
        assert!(counts_as_attachment(Some("cid"), 12 * 1024 + 1));
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_type_for_filename("Photo.JPG"), "image/jpeg");
        assert_eq!(mime_type_for_filename("report.pdf"), "application/pdf");
        assert_eq!(mime_type_for_filename("noext"), "application/octet-stream");
    }
}
