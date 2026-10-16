// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Reading the stored model JSON the engine returns (`Thread`, `Message`, `Folder`,
//! `Label`, `File`), and the tool output shapes of the desktop plugin's
//! `mcp-serializers.ts`. Callers check the policy before serializing; every message in the
//! output carries `draft` so a model can tell an unsent draft from received mail.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

pub fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn bool_of(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        // Thread `unread` / `starred` are counts of messages.
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        _ => false,
    }
}

pub fn i64_of(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|n| n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

pub fn array_of<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// An epoch-seconds timestamp as JSON.stringify writes a JS Date.
pub fn iso_date(secs: i64) -> Value {
    match DateTime::<Utc>::from_timestamp(secs, 0) {
        Some(d) if secs > 0 => Value::from(d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        _ => Value::Null,
    }
}

/// Ids of a thread's folders and labels (the desktop's `Thread.categories`).
pub fn thread_category_ids(thread: &Value) -> impl Iterator<Item = &str> {
    array_of(thread, "folders")
        .iter()
        .chain(array_of(thread, "labels"))
        .map(|c| str_of(c, "id"))
}

/// Ids of the folders that hold a copy of the message (`Message.folderIds()`: the keys of
/// its `folders` map).
pub fn message_folder_ids(message: &Value) -> impl Iterator<Item = &str> {
    message
        .get("folders")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|m| m.keys().map(String::as_str))
}

pub fn is_draft(message: &Value) -> bool {
    bool_of(message, "draft")
}

/// `Message.isHidden()`: a deletion placeholder, or a reminder the user sent themselves
/// (`… via Tern`, the send-reminders plugin's sender name; `… via Mailspring` before the rename).
/// An engine with the `message-hidden` feature works it out and sends it as `hidden`.
pub fn is_hidden(message: &Value) -> bool {
    if let Some(h) = message.get("hidden").and_then(Value::as_bool) {
        return h;
    }
    if str_of(message, "id").starts_with("deleted-") {
        return true;
    }
    let (to, from) = (array_of(message, "to"), array_of(message, "from"));
    to.len() == 1 && from.len() == 1 && str_of(&to[0], "email") == str_of(&from[0], "email") && {
        let name = str_of(&from[0], "name");
        name.ends_with("via Tern") || name.ends_with("via Mailspring")
    }
}

/// Decodes IMAP modified UTF-7 (RFC 3501 5.1.3). Malformed input comes back as is.
pub fn decode_imap_utf7(s: &str) -> String {
    fn sextet(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b',' => 63,
            _ => return None,
        } as u32)
    }
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'&' {
                i += 1;
            }
            out.push_str(&s[start..i]);
            continue;
        }
        let Some(end) = bytes[i + 1..]
            .iter()
            .position(|&b| b == b'-')
            .map(|p| i + 1 + p)
        else {
            return s.to_owned();
        };
        if end == i + 1 {
            out.push('&');
        } else {
            let (mut bits, mut nbits, mut units) = (0u32, 0u32, Vec::new());
            for &c in &bytes[i + 1..end] {
                let Some(v) = sextet(c) else {
                    return s.to_owned();
                };
                bits = (bits << 6) | v;
                nbits += 6;
                if nbits >= 16 {
                    nbits -= 16;
                    units.push(((bits >> nbits) & 0xFFFF) as u16);
                }
            }
            out.push_str(&String::from_utf16_lossy(&units));
        }
        i = end + 1;
    }
    out
}

/// `Category.displayName`: the decoded path without the INBOX / [Gmail] / [Mailspring] prefix.
pub fn category_display_name(category: &Value) -> String {
    let decoded = decode_imap_utf7(str_of(category, "path"));
    for prefix in ["INBOX", "[Gmail]", "[Mailspring]"] {
        if decoded.starts_with(prefix) && decoded.chars().count() > prefix.len() + 1 {
            if let Some(rest) = decoded.get(prefix.len() + 1..) {
                return rest.to_owned();
            }
        }
    }
    if decoded.starts_with("Mailspring/") || decoded.starts_with("Mailspring.") {
        return decoded[11..].to_owned();
    }
    if decoded == "INBOX" {
        return "Inbox".into();
    }
    decoded
}

/// `serializeCategory`: `{id, name}`. Gmail's system labels have no path; their role
/// stands in, as the desktop's `Category.name` does.
pub fn serialize_category(c: &Value) -> Value {
    let mut name = category_display_name(c);
    if name.is_empty() {
        name = str_of(c, "role").to_owned();
    }
    json!({"id": str_of(c, "id"), "name": name})
}

fn serialize_contact(c: &Value) -> Value {
    json!({"name": c.get("name").cloned().unwrap_or(Value::Null),
           "email": c.get("email").cloned().unwrap_or(Value::Null)})
}

pub fn contacts(v: &Value, key: &str) -> Value {
    Value::Array(array_of(v, key).iter().map(serialize_contact).collect())
}

pub fn file_display_name(f: &Value) -> String {
    let name = str_of(f, "filename");
    if !name.is_empty() {
        return name.to_owned();
    }
    match str_of(f, "contentType") {
        "text/calendar" => "Event.ics".into(),
        "image/png" => "Unnamed Image.png".into(),
        "image/jpg" | "image/jpeg" => "Unnamed Image.jpg".into(),
        _ => "Unnamed Attachment".into(),
    }
}

fn serialize_file(f: &Value) -> Value {
    json!({
        "id": str_of(f, "id"),
        "filename": file_display_name(f),
        "contentType": f.get("contentType").filter(|v| !v.is_null() && *v != "").cloned().unwrap_or(Value::Null),
        "size": f.get("size").cloned().unwrap_or(Value::Null),
        "isInline": !str_of(f, "contentId").is_empty(),
    })
}

/// `serializeThreadSummary`.
pub fn thread_summary(thread: &Value, include_message_count: bool) -> Value {
    let mut v = json!({
        "id": str_of(thread, "id"),
        "subject": str_of(thread, "subject"),
        "snippet": str_of(thread, "snippet"),
        "unread": bool_of(thread, "unread"),
        "starred": bool_of(thread, "starred"),
        "lastMessageReceivedTimestamp": iso_date(i64_of(thread, "lmrt")),
        "participantCount": array_of(thread, "participants").len(),
        "attachmentCount": i64_of(thread, "attachmentCount"),
        "accountId": str_of(thread, "aid"),
        "categories": thread_categories(thread),
    });
    if include_message_count {
        // The stored Thread has no message count; the plugin's field was always null too.
        v["messageCount"] = Value::Null;
    }
    v
}

fn thread_categories(thread: &Value) -> Value {
    Value::Array(
        array_of(thread, "folders")
            .iter()
            .chain(array_of(thread, "labels"))
            .map(serialize_category)
            .collect(),
    )
}

/// `serializeMessageDetail`. `folders_by_id` resolves the message's folder ids to names;
/// unknown ids are left out, as `Message.categories()` does.
pub fn message_detail(
    message: &Value,
    body: Option<&str>,
    folders_by_id: &HashMap<String, Value>,
) -> Value {
    let folders: Vec<Value> = message_folder_ids(message)
        .filter_map(|id| folders_by_id.get(id))
        .map(serialize_category)
        .collect();
    let opt_str = |k: &str| {
        message
            .get(k)
            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            .cloned()
            .unwrap_or(Value::Null)
    };
    json!({
        "id": str_of(message, "id"),
        "headerMessageId": str_of(message, "hMsgId"),
        "threadId": str_of(message, "threadId"),
        "from": contacts(message, "from"),
        "to": contacts(message, "to"),
        "cc": contacts(message, "cc"),
        "bcc": contacts(message, "bcc"),
        "subject": str_of(message, "subject"),
        "date": iso_date(i64_of(message, "date")),
        "body": body.map(Value::from).unwrap_or(Value::Null),
        "snippet": str_of(message, "snippet"),
        "files": array_of(message, "files").iter().map(serialize_file).collect::<Vec<_>>(),
        "unread": bool_of(message, "unread"),
        "starred": bool_of(message, "starred"),
        "draft": is_draft(message),
        "replyToHeaderMessageId": opt_str("rthMsgId"),
        "accountId": str_of(message, "aid"),
        "folders": folders,
    })
}

/// `serializeThreadDetail`: messages oldest first; the caller drops blocked ones.
pub fn thread_detail(thread: &Value, messages: Vec<Value>) -> Value {
    json!({
        "id": str_of(thread, "id"),
        "subject": str_of(thread, "subject"),
        "unread": bool_of(thread, "unread"),
        "starred": bool_of(thread, "starred"),
        "accountId": str_of(thread, "aid"),
        "lastMessageReceivedTimestamp": iso_date(i64_of(thread, "lmrt")),
        "categories": thread_categories(thread),
        "messages": messages,
    })
}

/// A draft in a thread, as `reply_to_thread` lists them.
pub fn draft_summary(message: &Value) -> Value {
    json!({
        "id": str_of(message, "id"),
        "headerMessageId": str_of(message, "hMsgId"),
        "subject": str_of(message, "subject"),
        "date": iso_date(i64_of(message, "date")),
        "draft": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf7_and_display_names() {
        assert_eq!(decode_imap_utf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_imap_utf7("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_imap_utf7("A&-B"), "A&B");
        assert_eq!(decode_imap_utf7("bad&"), "bad&");
        let name = |p: &str| category_display_name(&json!({"path": p}));
        assert_eq!(name("INBOX"), "Inbox");
        assert_eq!(name("INBOX.Receipts"), "Receipts");
        assert_eq!(name("[Gmail]/Sent Mail"), "Sent Mail");
        assert_eq!(name("Mailspring/Snoozed"), "Snoozed");
        assert_eq!(name("Archive"), "Archive");
        assert_eq!(
            serialize_category(&json!({"id": "l1", "path": "", "role": "important"})),
            json!({"id": "l1", "name": "important"})
        );
    }

    #[test]
    fn thread_and_message_shapes() {
        let thread = json!({"id": "t1", "aid": "a", "subject": "S", "snippet": "sn", "unread": 2,
            "starred": 0, "lmrt": 1_700_000_000, "participants": [{}, {}], "attachmentCount": 1,
            "folders": [{"id": "f1", "path": "INBOX"}], "labels": [{"id": "l1", "path": "Work"}]});
        let s = thread_summary(&thread, true);
        assert_eq!(s["unread"], true);
        assert_eq!(s["starred"], false);
        assert_eq!(
            s["lastMessageReceivedTimestamp"],
            "2023-11-14T22:13:20.000Z"
        );
        assert_eq!(
            s["categories"],
            json!([{"id": "f1", "name": "Inbox"}, {"id": "l1", "name": "Work"}])
        );
        assert_eq!(s["messageCount"], Value::Null);

        let msg = json!({"id": "m1", "aid": "a", "hMsgId": "<x@y>", "threadId": "t1", "draft": true,
            "from": [{"name": "A", "email": "a@x.test", "id": "c1"}], "date": 1_700_000_000,
            "folders": {"f1": 4, "gone": 0}, "rthMsgId": "",
            "files": [{"id": "fl", "filename": "", "contentType": "image/png", "size": 3, "contentId": "cid1"}]});
        let folders = HashMap::from([("f1".to_owned(), json!({"id": "f1", "path": "Drafts"}))]);
        let d = message_detail(&msg, Some("<p>b</p>"), &folders);
        assert_eq!(d["draft"], true);
        assert_eq!(d["from"], json!([{"name": "A", "email": "a@x.test"}]));
        assert_eq!(d["folders"], json!([{"id": "f1", "name": "Drafts"}]));
        assert_eq!(d["replyToHeaderMessageId"], Value::Null);
        assert_eq!(
            d["files"],
            json!([{"id": "fl", "filename": "Unnamed Image.png", "contentType": "image/png",
                    "size": 3, "isInline": true}])
        );
        assert_eq!(d["body"], "<p>b</p>");
    }

    #[test]
    fn hidden_messages() {
        assert!(is_hidden(&json!({"id": "deleted-1"})));
        assert!(is_hidden(&json!({"id": "m",
            "to": [{"email": "me@x.test"}], "from": [{"email": "me@x.test", "name": "Me via Mailspring"}]})));
        assert!(is_hidden(&json!({"id": "m",
            "to": [{"email": "me@x.test"}], "from": [{"email": "me@x.test", "name": "Me via Tern"}]})));
        assert!(!is_hidden(
            &json!({"id": "m", "to": [{"email": "me@x.test"}], "from": [{"email": "me@x.test"}]})
        ));
        // The engine's answer wins.
        assert!(is_hidden(&json!({"id": "m", "hidden": true})));
        assert!(!is_hidden(&json!({"id": "deleted-1", "hidden": false})));
    }
}
