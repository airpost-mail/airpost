// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The `summary` a Thread row carries with `include: ["summary"]`: what the thread list shows
//! of a thread's messages, read from the Message table for the rows of the same read, so a
//! list needs no message read per thread.
//!
//! Ports what the desktop's thread list derives from `thread.__messages`
//! (`features/thread-list/data-source.ts`, `participants.tsx`, `icon.tsx`, `columns.tsx`):
//! hidden messages ([`crate::models::is_hidden_json`]) are left out first, as the list's join
//! leaves them out.
//!
//! - `messageCount`: the visible messages, drafts included.
//! - `senders`: `getTokensFromMessages`: each non-draft message's `from` contacts in date
//!   order (its `to` when every visible message is from the user), a contact repeating the
//!   one before it (same name, equivalent address) folded into it with its unread flag
//!   or'ed, and `me` on the user's addresses (any account's address or alias).
//! - `hasDraft`, and `sending` when a draft has a pending `SendDraftTask`.
//! - `hasAttachment`: a visible message has a file that isn't an inline image of at most
//!   12 KB (`Utils.showIconForAttachments`).
//! - `snippet`: the last visible message's snippet that isn't empty.
//! - `lastReply`: `replied` or `forwarded` when the thread has more than one non-draft
//!   message and the last one is from the user (`icon.tsx`). `isForwarded` looks at the first
//!   250 characters of the body; the summary looks at the snippet, which is that text.

use std::collections::{HashMap, HashSet};

use airpost_core::Result;
use rusqlite::params_from_iter;
use rusqlite::types::Value as SqlValue;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::read::ReadStore;

/// The user's addresses, compared as [`equivalent_form`]s.
#[derive(Clone, Debug, Default)]
pub struct OwnAddresses(HashSet<String>);

impl OwnAddresses {
    pub fn new<'a>(addresses: impl IntoIterator<Item = &'a str>) -> Self {
        OwnAddresses(
            addresses
                .into_iter()
                .filter(|a| !a.trim().is_empty())
                .map(equivalent_form)
                .collect(),
        )
    }

    pub fn contains(&self, email: &str) -> bool {
        !self.0.is_empty() && self.0.contains(&equivalent_form(email))
    }
}

/// `Utils.toEquivalentEmailForm`: lowercased, without a `+tag` in the local part.
pub fn equivalent_form(email: &str) -> String {
    let Some(at) = email.rfind('@') else {
        return email.trim().to_lowercase();
    };
    let (local, domain) = (&email[..at], &email[at + 1..]);
    let user = local.split('+').next().unwrap_or("");
    if user.is_empty() || domain.is_empty() {
        return email.trim().to_lowercase();
    }
    format!("{user}@{domain}").trim().to_lowercase()
}

/// `Utils.emailIsEquivalent`.
fn email_equivalent(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    a == b || equivalent_form(&a) == equivalent_form(&b)
}

#[derive(Debug, Default, Deserialize)]
struct Person {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AttachedFile {
    #[serde(default, rename = "contentId")]
    content_id: Option<String>,
    #[serde(default)]
    size: Option<f64>,
}

/// The keys of a Message's JSON the summary reads; serde skips the rest.
#[derive(Debug, Default, Deserialize)]
struct SlimMessage {
    #[serde(default)]
    id: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    unread: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default, rename = "hMsgId")]
    header_message_id: String,
    #[serde(default)]
    from: Vec<Person>,
    #[serde(default)]
    to: Vec<Person>,
    #[serde(default)]
    files: Vec<AttachedFile>,
}

impl SlimMessage {
    /// [`crate::models::is_hidden_json`] on the keys it reads.
    fn is_hidden(&self) -> bool {
        if airpost_core::ids::is_deletion_placeholder_id(&self.id) {
            return true;
        }
        let ([from], [to]) = (self.from.as_slice(), self.to.as_slice()) else {
            return false;
        };
        let name = from.name.as_deref().unwrap_or("");
        from.email.is_some()
            && from.email == to.email
            && (name.ends_with("via Airpost") || name.ends_with("via Mailspring"))
    }

    fn is_from_me(&self, own: &OwnAddresses) -> bool {
        self.from
            .first()
            .and_then(|p| p.email.as_deref())
            .is_some_and(|e| own.contains(e))
    }

    /// `Message.isForwarded()`, with the snippet standing in for the body.
    fn is_forwarded(&self) -> bool {
        let subject = self.subject.to_lowercase();
        if subject.starts_with("fwd:") {
            return true;
        }
        if subject.starts_with("re:") {
            return false;
        }
        let head: String = self
            .snippet
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(250)
            .collect::<String>()
            .to_lowercase();
        head.contains("forwarded") || head.contains("fwd")
    }

    fn shows_attachment_icon(&self) -> bool {
        self.files.iter().any(|f| {
            f.content_id.as_deref().is_none_or(str::is_empty)
                || f.size.unwrap_or(0.0) > 12.0 * 1024.0
        })
    }
}

struct Token {
    name: Option<String>,
    email: String,
    unread: bool,
    me: bool,
}

/// `getTokensFromMessages` over the visible messages.
fn senders(visible: &[&SlimMessage], own: &OwnAddresses) -> Vec<Token> {
    let use_to = visible.iter().all(|m| m.is_from_me(own));
    let mut tokens: Vec<Token> = Vec::new();
    for m in visible.iter().filter(|m| !m.draft) {
        let contacts = if use_to { &m.to } else { &m.from };
        for c in contacts {
            let email = c.email.clone().unwrap_or_default();
            if let Some(last) = tokens.last_mut() {
                if email_equivalent(&last.email, &email) && last.name == c.name {
                    last.unread |= m.unread;
                    continue;
                }
            }
            tokens.push(Token {
                me: own.contains(&email),
                name: c.name.clone(),
                email,
                unread: m.unread,
            });
        }
    }
    tokens
}

fn summarize(messages: &[SlimMessage], own: &OwnAddresses, pending: &HashSet<String>) -> Value {
    let visible: Vec<&SlimMessage> = messages.iter().filter(|m| !m.is_hidden()).collect();
    let sent: Vec<&&SlimMessage> = visible
        .iter()
        .filter(|m| !m.id.is_empty() && !m.draft)
        .collect();
    let last_reply = match sent.last() {
        Some(last) if sent.len() > 1 && last.is_from_me(own) => Some(if last.is_forwarded() {
            "forwarded"
        } else {
            "replied"
        }),
        _ => None,
    };
    let snippet = visible
        .iter()
        .rev()
        .find_map(|m| m.snippet.as_deref().filter(|s| !s.is_empty()));
    let senders: Vec<Value> = senders(&visible, own)
        .into_iter()
        .map(|t| {
            let mut v = json!({"email": t.email, "unread": t.unread, "me": t.me});
            if let Some(name) = t.name {
                v["name"] = json!(name);
            }
            v
        })
        .collect();
    let mut summary = json!({
        "messageCount": visible.len(),
        "senders": senders,
        "hasDraft": visible.iter().any(|m| m.draft),
        "sending": visible
            .iter()
            .any(|m| m.draft && pending.contains(&m.header_message_id)),
        "hasAttachment": visible.iter().any(|m| m.shows_attachment_icon()),
        "snippet": snippet,
    });
    if let Some(r) = last_reply {
        summary["lastReply"] = json!(r);
    }
    summary
}

/// The most thread ids one statement binds.
const CHUNK: usize = 500;

impl ReadStore {
    /// The `summary` of each thread in `thread_ids` that has messages, by thread id.
    pub fn thread_summaries(
        &self,
        thread_ids: &[&str],
        own: &OwnAddresses,
    ) -> Result<HashMap<String, Value>> {
        let mut by_thread: HashMap<String, Vec<SlimMessage>> = HashMap::new();
        let mut has_drafts = false;
        for chunk in thread_ids.chunks(CHUNK) {
            let sql = format!(
                "SELECT threadId, data FROM Message WHERE threadId IN ({}) \
                 ORDER BY threadId, date ASC",
                vec!["?"; chunk.len()].join(",")
            );
            let mut stmt = self.conn().prepare(&sql)?;
            let mut rows = stmt.query(params_from_iter(
                chunk.iter().map(|id| SqlValue::Text((*id).to_owned())),
            ))?;
            while let Some(r) = rows.next()? {
                let thread: String = r.get(0)?;
                let m: SlimMessage = match r.get_ref(1)? {
                    rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
                        serde_json::from_slice(b).unwrap_or_default()
                    }
                    _ => SlimMessage::default(),
                };
                has_drafts |= m.draft;
                by_thread.entry(thread).or_default().push(m);
            }
        }
        let pending = if has_drafts {
            self.pending_send_header_ids()?
        } else {
            HashSet::new()
        };
        Ok(by_thread
            .into_iter()
            .map(|(id, messages)| {
                let s = summarize(&messages, own, &pending);
                (id, s)
            })
            .collect())
    }

    /// The `headerMessageId`s of the drafts a pending `SendDraftTask` is sending.
    fn pending_send_header_ids(&self) -> Result<HashSet<String>> {
        let mut stmt = self
            .conn()
            .prepare_cached("SELECT data FROM Task WHERE status IN ('local', 'remote')")?;
        let mut rows = stmt.query([])?;
        let mut out = HashSet::new();
        while let Some(r) = rows.next()? {
            let task: Value = match r.get_ref(0)? {
                rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
                    serde_json::from_slice(b).unwrap_or(Value::Null)
                }
                _ => Value::Null,
            };
            if task["__cls"] != "SendDraftTask" {
                continue;
            }
            for h in [&task["headerMessageId"], &task["draft"]["hMsgId"]] {
                if let Some(h) = h.as_str().filter(|h| !h.is_empty()) {
                    out.insert(h.to_owned());
                }
            }
        }
        Ok(out)
    }
}

/// An empty summary, for a thread without messages.
pub fn empty_summary() -> Value {
    summarize(&[], &OwnAddresses::default(), &HashSet::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::is_hidden_json;

    fn msg(v: Value) -> SlimMessage {
        serde_json::from_value(v).unwrap()
    }

    fn own() -> OwnAddresses {
        OwnAddresses::new(["me@example.com", "alias@example.org"])
    }

    #[test]
    fn equivalent_addresses() {
        assert_eq!(equivalent_form("Me+News@Example.com"), "me@example.com");
        assert!(own().contains("ME+x@example.com"));
        assert!(!own().contains("you@example.com"));
        assert!(!OwnAddresses::default().contains(""));
    }

    #[test]
    fn senders_fold_repeats_and_skip_drafts_and_hidden() {
        let messages = [
            msg(
                json!({"id": "1", "from": [{"name": "Ann", "email": "ann@x.com"}], "unread": false}),
            ),
            msg(
                json!({"id": "2", "from": [{"name": "Ann", "email": "ANN@x.com"}], "unread": true}),
            ),
            msg(
                json!({"id": "3", "from": [{"name": "Me", "email": "me@example.com"}],
                       "subject": "Re: hi", "snippet": "thanks"}),
            ),
            msg(
                json!({"id": "4", "from": [{"email": "me@example.com"}], "draft": true,
                       "hMsgId": "h4", "snippet": "draft text"}),
            ),
            msg(json!({"id": "deleted-5", "from": [{"email": "zed@x.com"}]})),
        ];
        let s = summarize(&messages, &own(), &HashSet::from(["h4".to_owned()]));
        assert_eq!(s["messageCount"], 4);
        assert_eq!(
            s["senders"],
            json!([{"name": "Ann", "email": "ann@x.com", "unread": true, "me": false},
                   {"name": "Me", "email": "me@example.com", "unread": false, "me": true}])
        );
        assert_eq!(
            (s["hasDraft"].as_bool(), s["sending"].as_bool()),
            (Some(true), Some(true))
        );
        assert_eq!(s["snippet"], "draft text");
        assert_eq!(s["lastReply"], "replied");
        assert_eq!(s["hasAttachment"], false);
    }

    #[test]
    fn a_thread_only_from_me_names_its_recipients() {
        let messages = [
            msg(json!({"id": "1", "from": [{"email": "me@example.com"}],
                       "to": [{"name": "Bob", "email": "bob@x.com"}],
                       "subject": "Fwd: plans",
                       "files": [{"contentId": "c1", "size": 100}]})),
            msg(json!({"id": "2", "from": [{"email": "alias@example.org"}],
                       "to": [{"name": "Bob", "email": "bob@x.com"}], "subject": "plans",
                       "snippet": "---------- Forwarded message ---------",
                       "files": [{"contentId": "c2", "size": 20000}]})),
        ];
        let s = summarize(&messages, &own(), &HashSet::new());
        assert_eq!(
            s["senders"],
            json!([{"name": "Bob", "email": "bob@x.com", "unread": false, "me": false}])
        );
        assert_eq!(s["lastReply"], "forwarded");
        assert_eq!(s["hasAttachment"], true);
        assert_eq!(s["sending"], false);
    }

    #[test]
    fn hidden_as_the_store_decides() {
        let reminder = json!({"id": "r", "from": [{"name": "Me via Airpost", "email": "me@x.com"}],
                              "to": [{"email": "me@x.com"}]});
        assert!(is_hidden_json(&reminder));
        assert!(msg(reminder).is_hidden());
        let plain = json!({"id": "p", "from": [{"name": "Me", "email": "me@x.com"}],
                           "to": [{"email": "me@x.com"}]});
        assert!(!msg(plain).is_hidden());
        let s = empty_summary();
        assert_eq!(
            (s["messageCount"].as_u64(), s["snippet"].is_null()),
            (Some(0), true)
        );
        assert!(s.get("lastReply").is_none());
    }
}
