// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The user's accounts as the tools see them, and `Message.toJSON()` for a stored draft sent
//! back in a `SyncbackDraftTask` or `SendDraftTask`, the way the desktop's draft session
//! saves it. New and reply drafts are built by `crate::compose`.

use serde_json::{json, Map, Value};
use tern_compose::contacts::{contact_from_string, emails_equivalent};

use crate::config::AccountMeta;
use crate::model::{array_of, bool_of, str_of};

/// An account's identity for composing: what `config.json` says about it, or just the
/// address the engine reports when the desktop hasn't written one.
#[derive(Clone, Debug, Default)]
pub struct Sender {
    pub account_id: String,
    pub meta: AccountMeta,
}

/// A recipient as the tools take them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    pub name: String,
    pub email: String,
}

pub fn contact_json(name: &str, email: &str) -> Value {
    json!({"name": name, "email": email, "__cls": "Contact"})
}

/// The addresses that count as the user (`Contact.isMe`): every account and alias.
#[derive(Clone, Debug, Default)]
pub struct Me {
    emails: Vec<String>,
}

impl Me {
    pub fn new(senders: &[Sender]) -> Me {
        let mut emails = Vec::new();
        for s in senders {
            emails.push(s.meta.email.clone());
            emails.extend(s.meta.aliases.iter().map(|a| contact_from_string(a).email));
        }
        emails.retain(|e| !e.is_empty());
        Me { emails }
    }

    pub fn is_me(&self, email: &str) -> bool {
        self.emails.iter().any(|m| emails_equivalent(m, email))
    }
}

/// `Message.toJSON()` of a stored draft: its model attributes only (the engine fills in
/// its own bookkeeping from the stored copy), plus `body` when known, `file_ids` and
/// `object`. A missing `body` leaves the stored one in place.
pub fn desktop_message_json(stored: &Value, body: Option<&str>) -> Value {
    const KEYS: &[&str] = &[
        "id",
        "aid",
        "v",
        "metadata",
        "to",
        "cc",
        "bcc",
        "from",
        "replyTo",
        "date",
        "files",
        "unread",
        "events",
        "starred",
        "snippet",
        "threadId",
        "hMsgId",
        "subject",
        "draft",
        "pristine",
        "plaintext",
        "rthMsgId",
        "fwdMsgId",
        "folders",
        "hListUnsub",
        "hListUnsubPost",
    ];
    let mut out = Map::new();
    for k in KEYS {
        if let Some(v) = stored.get(*k).filter(|v| !v.is_null()) {
            out.insert((*k).to_owned(), v.clone());
        }
    }
    for k in ["to", "cc", "bcc", "from", "replyTo", "files", "events"] {
        out.entry(k).or_insert_with(|| Value::Array(Vec::new()));
    }
    out.entry("subject").or_insert_with(|| Value::from(""));
    // The engine requires it in a send; the desktop's Message model defaults it to false.
    out.entry("plaintext").or_insert(Value::Bool(false));
    if let Some(b) = body {
        out.insert("body".into(), Value::from(b));
    }
    let file_ids: Vec<Value> = array_of(stored, "files")
        .iter()
        .map(|f| Value::from(str_of(f, "id")))
        .collect();
    out.insert("file_ids".into(), Value::Array(file_ids));
    if bool_of(stored, "draft") {
        out.insert("object".into(), Value::from("draft"));
    }
    out.insert("__cls".into(), Value::from("Message"));
    Value::Object(out)
}

pub fn recipients_json(list: &[Recipient]) -> Value {
    Value::Array(
        list.iter()
            .map(|c| contact_json(&c.name, &c.email))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn me_and_stored_draft_json() {
        let me = Me::new(&[Sender {
            account_id: "a1".into(),
            meta: AccountMeta {
                id: "a1".into(),
                name: "Me".into(),
                email: "me@x.test".into(),
                aliases: vec!["Work Me <work@x.test>".into()],
                ..Default::default()
            },
        }]);
        assert!(me.is_me("Me+tag@x.test") && me.is_me("work@x.test") && !me.is_me("bob@y.test"));

        let stored = json!({"id": "d1", "aid": "a1", "hMsgId": "H@x.test", "threadId": "t1",
            "rthMsgId": "<orig@y.test>", "draft": true, "_sa": 5, "_suc": 0, "gMsgId": "",
            "files": [{"id": "f1", "filename": "a.pdf"}], "folders": {"fd": 4}});
        let j = desktop_message_json(&stored, Some("<p>new</p>"));
        assert_eq!(j["body"], "<p>new</p>");
        assert_eq!(j["file_ids"], json!(["f1"]));
        assert!(j.get("_sa").is_none() && j.get("gMsgId").is_none());
        assert_eq!(j["to"], json!([]));
        assert_eq!(j["plaintext"], false);
        assert_eq!(j["__cls"], "Message");
        assert!(desktop_message_json(&stored, None).get("body").is_none());
    }
}
