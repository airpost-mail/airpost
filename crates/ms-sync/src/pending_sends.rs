// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The bodies a client's send hooks made (`draft.send`'s `bodies`, docs/desktop-client-plan.md
//! 3.3), kept until the undo window ends and the executor sends the draft.
//!
//! They live in a file per draft, `<config>/pending-sends/<accountId>/<draftId>.json`, not in
//! the send-later metadata: metadata syncs to the user's other devices and the metadata
//! backend, and these bodies (per recipient, possibly with tracking links) are for this one
//! send only. The file names the expiration it was written for, so a send rescheduled or
//! cancelled through the metadata alone (an older client, the CLI) doesn't pick up stale
//! bodies; such files go at the executor's next start. "Send now", which moves the undo window
//! earlier and keeps `isUndoSend`, keeps them ([`PendingBodies::belong_to`]). A draft sent from another device
//! goes out with its stored body.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const PENDING_SENDS_DIR: &str = "pending-sends";

/// What goes out instead of the stored body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingBodies {
    /// The send-later expiration (Unix seconds) these bodies belong to.
    pub expiration: i64,
    /// The body sent and saved to Sent.
    pub self_body: Option<String>,
    /// Recipient address to body.
    pub per_recipient: BTreeMap<String, String>,
}

impl PendingBodies {
    fn to_json(&self) -> Value {
        json!({
            "expiration": self.expiration,
            "self": self.self_body,
            "perRecipient": self.per_recipient,
        })
    }

    fn from_json(v: &Value) -> Option<PendingBodies> {
        Some(PendingBodies {
            expiration: v.get("expiration")?.as_i64()?,
            self_body: v.get("self").and_then(Value::as_str).map(str::to_owned),
            per_recipient: v
                .get("perRecipient")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Whether these bodies go out with the draft's send-later `value`: the window they were
    /// kept for, or the same undo window moved earlier ("Send now" keeps `isUndoSend`). A
    /// cleared, later or plain send-later schedule (rescheduled through the metadata alone)
    /// doesn't take them.
    pub fn belong_to(&self, value: &Value) -> bool {
        let Some(due) = value
            .get("expiration")
            .and_then(Value::as_f64)
            .map(|e| e as i64)
        else {
            return false;
        };
        due == self.expiration
            || (due < self.expiration
                && value.get("isUndoSend").and_then(Value::as_bool) == Some(true))
    }

    /// `SendDraftTask.perRecipientBodies` for a send with `body` as the sent copy's.
    pub fn per_recipient_json(&self, body: &str) -> Value {
        if self.per_recipient.is_empty() {
            return Value::Null;
        }
        let mut m = serde_json::Map::new();
        m.insert("self".into(), json!(body));
        for (k, v) in &self.per_recipient {
            m.insert(k.clone(), json!(v));
        }
        Value::Object(m)
    }
}

/// Why `per_recipient` can't go out with `draft` (its JSON), `None` when it can. A multisend
/// goes to the map's keys only, so they must be the draft's To, Cc and Bcc addresses, each
/// once (compared case-insensitively; a `self` key is the sent copy's and is ignored): a
/// missing one would never get the mail and an extra one would get it unseen.
pub fn recipient_mismatch(
    draft: &Value,
    per_recipient: &BTreeMap<String, String>,
) -> Option<String> {
    if per_recipient.is_empty() {
        return None;
    }
    let norm = |s: &str| s.trim().to_ascii_lowercase();
    let want: BTreeSet<String> = ["to", "cc", "bcc"]
        .iter()
        .filter_map(|k| draft.get(*k).and_then(Value::as_array))
        .flatten()
        .filter_map(|c| c.get("email").and_then(Value::as_str))
        .map(norm)
        .filter(|e| !e.is_empty())
        .collect();
    let mut got = BTreeSet::new();
    let mut repeated = Vec::new();
    for key in per_recipient.keys().filter(|k| k.as_str() != "self") {
        if !got.insert(norm(key)) {
            repeated.push(key.clone());
        }
    }
    let missing: Vec<&str> = want.difference(&got).map(String::as_str).collect();
    let extra: Vec<&str> = got.difference(&want).map(String::as_str).collect();
    let mut why = Vec::new();
    if !missing.is_empty() {
        why.push(format!("no body for {}", missing.join(", ")));
    }
    if !extra.is_empty() {
        why.push(format!("{} not a recipient", extra.join(", ")));
    }
    if !repeated.is_empty() {
        why.push(format!("{} given twice", repeated.join(", ")));
    }
    (!why.is_empty()).then(|| {
        format!(
            "perRecipient needs one body for each To, Cc and Bcc address: {}",
            why.join("; ")
        )
    })
}

/// File-name-safe: the ids are hex or base64url already; anything else becomes `_`.
fn safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// One account's pending bodies.
#[derive(Clone, Debug)]
pub struct PendingSends {
    dir: PathBuf,
}

impl PendingSends {
    pub fn new(config_dir: &Path, account_id: &str) -> PendingSends {
        PendingSends {
            dir: config_dir.join(PENDING_SENDS_DIR).join(safe(account_id)),
        }
    }

    fn path(&self, draft_id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", safe(draft_id)))
    }

    /// Written to a temporary file and renamed, readable by the user only.
    pub fn put(&self, draft_id: &str, bodies: &PendingBodies) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.path(draft_id);
        let tmp = path.with_extension("json.tmp");
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            io::Write::write_all(&mut f, bodies.to_json().to_string().as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)
    }

    pub fn get(&self, draft_id: &str) -> Option<PendingBodies> {
        let text = fs::read_to_string(self.path(draft_id)).ok()?;
        PendingBodies::from_json(&serde_json::from_str(&text).ok()?)
    }

    pub fn remove(&self, draft_id: &str) {
        let _ = fs::remove_file(self.path(draft_id));
    }

    /// The draft ids with a file.
    pub fn ids(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".json").map(str::to_owned)
            })
            .collect()
    }

    /// Every file of the account (its data deleted or reset).
    pub fn remove_all(&self) {
        let _ = fs::remove_dir_all(&self.dir);
    }

    /// Every account's (the database rebuilt without its metadata).
    pub fn remove_every_account(config_dir: &Path) {
        let _ = fs::remove_dir_all(config_dir.join(PENDING_SENDS_DIR));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let p = PendingSends::new(dir.path(), "acc/1");
        let bodies = PendingBodies {
            expiration: 100,
            self_body: Some("<p>x</p>".into()),
            per_recipient: [("a@b.example".to_owned(), "<p>a</p>".to_owned())].into(),
        };
        p.put("d1", &bodies).unwrap();
        assert_eq!(p.get("d1"), Some(bodies.clone()));
        assert_eq!(p.ids(), ["d1"]);
        assert_eq!(
            bodies.per_recipient_json("<p>s</p>"),
            json!({"self": "<p>s</p>", "a@b.example": "<p>a</p>"})
        );
        p.remove("d1");
        assert_eq!(p.get("d1"), None);
        assert!(bodies.belong_to(&json!({"expiration": 100})));
        assert!(bodies.belong_to(&json!({"expiration": 90, "isUndoSend": true})));
        assert!(!bodies.belong_to(&json!({"expiration": 90})));
        assert!(!bodies.belong_to(&json!({"expiration": 110, "isUndoSend": true})));
        assert!(!bodies.belong_to(&json!({"expiration": null, "isUndoSend": true})));
        p.put("d2", &bodies).unwrap();
        PendingSends::remove_every_account(dir.path());
        assert!(p.ids().is_empty());
    }

    #[test]
    fn per_recipient_bodies_must_match_the_recipients() {
        let draft = json!({
            "to": [{"email": "Ann@x.example"}],
            "cc": [{"email": "bob@x.example"}],
            "bcc": [{"email": "cat@x.example"}, {"email": ""}],
        });
        let map = |keys: &[&str]| -> BTreeMap<String, String> {
            keys.iter()
                .map(|k| (k.to_string(), "<p>b</p>".to_owned()))
                .collect()
        };
        assert_eq!(recipient_mismatch(&draft, &BTreeMap::new()), None);
        assert_eq!(
            recipient_mismatch(
                &draft,
                &map(&["ann@x.example", "BOB@x.example", "cat@x.example", "self"])
            ),
            None
        );
        let why = recipient_mismatch(&draft, &map(&["ann@x.example", "eve@x.example"])).unwrap();
        assert!(
            why.contains("no body for bob@x.example, cat@x.example"),
            "{why}"
        );
        assert!(why.contains("eve@x.example not a recipient"), "{why}");
        let why = recipient_mismatch(
            &draft,
            &map(&[
                "ann@x.example",
                "Ann@x.example",
                "bob@x.example",
                "cat@x.example",
            ]),
        )
        .unwrap();
        assert!(why.contains("given twice"), "{why}");
    }
}
