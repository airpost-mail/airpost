//! `File`: an attachment or inline part of a message. Derived from Mailspring-Sync
//! `MailSync/Models/File.*`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

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
