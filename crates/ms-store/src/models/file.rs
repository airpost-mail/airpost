// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `File`: an attachment or inline part of a message. Derived from Mailspring-Sync
//! `MailSync/Models/File.*`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::path::{Path, PathBuf};

use ms_core::Result;
use rusqlite::Statement;
use serde_json::{json, Value};

use crate::model::{base_json, bind_base, bind_named, json_i64, json_str, object_or_err};
use crate::{Message, Model};

/// One attachment or inline part as the MIME parser describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilePart<'a> {
    pub part_id: &'a str,
    /// Set only for inline parts.
    pub content_id: Option<&'a str>,
    pub content_type: Option<&'a str>,
    /// "" for an unnamed part, which gets [`File::default_filename`].
    pub filename: &'a str,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub data: Value,
}

impl File {
    /// `File(Message *, Attachment *)`: `id` is `ms_core::ids::file_id` of the part.
    pub fn create(id: &str, message: &Message, part: &FilePart<'_>) -> Self {
        let mut data = base_json(id, message.account_id(), 0);
        data["messageId"] = json!(message.id());
        data["partId"] = json!(part.part_id);
        // Always present: MailProcessor.cpp reads `contentId()` through a non-const
        // `_data["contentId"]`, which writes null for a part without one.
        data["contentId"] = part.content_id.map_or(Value::Null, |cid| json!(cid));
        if let Some(ct) = part.content_type {
            data["contentType"] = json!(ct);
        }
        let name = if part.filename.is_empty() {
            File::default_filename(part.content_type.unwrap_or(""))
        } else {
            part.filename
        };
        data["filename"] = json!(name);
        data["size"] = json!(part.size);
        File { data }
    }

    /// The name of an unnamed part, by MIME type (`File.cpp:40-65`).
    pub fn default_filename(content_type: &str) -> &'static str {
        match content_type {
            "text/calendar" => "Event.ics",
            "image/png" | "image/x-png" => "Unnamed Image.png",
            "image/jpg" | "image/jpeg" => "Unnamed Image.jpg",
            "image/gif" => "Unnamed Image.gif",
            "message/delivery-status" => "Delivery Status.txt",
            "message/feedback-report" => "Feedback Report.txt",
            _ => "Unnamed Attachment",
        }
    }

    pub fn filename(&self) -> &str {
        json_str(&self.data, "filename")
    }

    /// The on-disk name: each of `\ / : | ? * > < " #` becomes `-`.
    pub fn safe_filename(&self) -> String {
        self.filename()
            .chars()
            .map(|c| if "\\/:|?*><\"#".contains(c) { '-' } else { c })
            .collect()
    }

    /// The client's `File.displayName()`: the filename, or a default by MIME type. The
    /// client localises "Unnamed Attachment"; the engine names every part it creates, so
    /// only rows written by older clients reach that fallback.
    pub fn display_name(&self) -> &str {
        if !self.filename().is_empty() {
            return self.filename();
        }
        match self.content_type() {
            "text/calendar" => "Event.ics",
            "image/png" => "Unnamed Image.png",
            "image/jpg" | "image/jpeg" => "Unnamed Image.jpg",
            _ => "Unnamed Attachment",
        }
    }

    /// The client's `File.safeDisplayName()`, the name the client gives the file on disk
    /// (`attachment-store.ts` `pathForFile`): C0/C1 controls and bidi controls removed,
    /// the [`safe_filename`](Self::safe_filename) characters replaced, trailing dots and
    /// spaces trimmed, reserved Windows names prefixed with `_`, and `-` for an empty name.
    pub fn safe_display_name(&self) -> String {
        let mut name: String = self
            .display_name()
            .chars()
            .filter(|&c| {
                !matches!(c, '\u{00}'..='\u{1F}' | '\u{7F}'..='\u{9F}')
                    && !matches!(
                        c,
                        '\u{061C}'
                            | '\u{200E}'
                            | '\u{200F}'
                            | '\u{202A}'..='\u{202E}'
                            | '\u{2066}'..='\u{2069}'
                    )
            })
            .map(|c| if "\\/:|?*><\"#".contains(c) { '-' } else { c })
            .collect();
        name.truncate(name.trim_end_matches(['.', ' ']).len());
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem[3..].chars().count() == 1
            && stem[3..]
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³')));
        if reserved {
            name.insert(0, '_');
        }
        if name.is_empty() {
            name.push('-');
        }
        name
    }

    /// `files/<id[0:2]>/<id[2:4]>/<id>` under `files_root`, id lowercased. File ids come
    /// from clients too (a draft's `files`), so anything but ASCII letters, digits, `-` and
    /// `_` becomes `_`: an id like `/Users/me/.ssh` or `../..` would otherwise leave
    /// `files_root` (`Path::join` with an absolute segment replaces the root).
    pub fn dir_in(&self, files_root: &Path) -> PathBuf {
        let id: String = self
            .id()
            .chars()
            .map(|c| match c {
                'a'..='z' | '0'..='9' | '-' | '_' => c,
                'A'..='Z' => c.to_ascii_lowercase(),
                _ => '_',
            })
            .collect();
        files_root
            .join(id.get(0..2).unwrap_or(&id))
            .join(id.get(2..4).unwrap_or(""))
            .join(&id)
    }

    /// Where the client reads and writes this file (`AttachmentStore.pathForFile`).
    pub fn client_path_in(&self, files_root: &Path) -> PathBuf {
        self.dir_in(files_root).join(self.safe_display_name())
    }

    pub fn part_id(&self) -> &str {
        json_str(&self.data, "partId")
    }

    pub fn message_id(&self) -> &str {
        json_str(&self.data, "messageId")
    }

    /// `contentId`, null for attachments.
    pub fn content_id(&self) -> &Value {
        &self.data["contentId"]
    }

    pub fn set_content_id(&mut self, cid: &str) {
        self.data["contentId"] = json!(cid);
    }

    pub fn content_type(&self) -> &str {
        json_str(&self.data, "contentType")
    }

    pub fn size(&self) -> i64 {
        json_i64(&self.data["size"])
    }
}

impl Model for File {
    model_boilerplate!("File", ["id", "data", "accountId", "version", "filename"]);

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "File")?;
        Ok(File { data })
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_base(self, stmt)?;
        bind_named(stmt, ":filename", self.filename())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str) -> File {
        File {
            data: json!({ "id": id }),
        }
    }

    #[test]
    fn dir_in_stays_under_the_root() {
        let root = Path::new("/files");
        assert_eq!(
            file("AbCdef12").dir_in(root),
            Path::new("/files/ab/cd/abcdef12")
        );
        for id in ["/Users/me/.ssh", "../../etc", "a/../../b", "..", "/", "é/x"] {
            let dir = file(id).dir_in(root);
            assert!(dir.starts_with(root), "{id} → {}", dir.display());
            assert!(
                dir.components().all(|c| matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )),
                "{id} → {}",
                dir.display()
            );
        }
    }
}
