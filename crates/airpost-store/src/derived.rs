// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Fields derived from a model's JSON that the engine adds to the copies clients read (the
//! rows of `query`, live queries and the read methods, and the socket's deltas), so no client
//! re-derives them. Never stored.
//!
//! - Message: `hidden` (`Message.isHidden()`, [`is_hidden_json`]) and `warnings`, left out
//!   when there are none ([`message_warnings`]).
//! - Folder and Label: `displayName` (`Category.displayName`) and `sortOrder`
//!   ([`category_sort_order`]).
//!
//! `warnings` ports the desktop's `phishing-detection` plugin (`lib/main.tsx`), which warns
//! when the first Reply-To address is in another registered domain than the first From
//! address. The plugin finds registered domains with the `tld` package's 2011 copy of the
//! Public Suffix List, which reads a domain under a suffix it doesn't know (`.app`, `.dev`,
//! `.test`) as none, so two such domains always match; the `psl` crate's current list reads
//! them as their last label, as the list's default rule does.

use serde_json::{json, Value};

use crate::models::is_hidden_json;

pub const REPLY_TO_MISMATCH: &str = "reply-to-mismatch";

/// The desktop's `Category.StandardRoles`, in the sidebar's order.
pub const STANDARD_ROLES: &[&str] = &[
    "inbox",
    "important",
    "snoozed",
    "sent",
    "drafts",
    "all",
    "spam",
    "archive",
    "trash",
];

/// Whether [`annotate`] adds anything to models of `class`.
pub fn has_derived(class: &str) -> bool {
    matches!(class, "Message" | "Folder" | "Label")
}

/// Adds the derived fields of `class` to `model` (see the module doc).
pub fn annotate(class: &str, model: &mut Value) {
    match class {
        "Message" => {
            let hidden = is_hidden_json(model);
            let warnings = message_warnings(model);
            if let Some(obj) = model.as_object_mut() {
                obj.insert("hidden".into(), Value::Bool(hidden));
                if warnings.is_empty() {
                    obj.remove("warnings");
                } else {
                    obj.insert("warnings".into(), Value::Array(warnings));
                }
            }
        }
        "Folder" | "Label" => {
            let path = model.get("path").and_then(Value::as_str).unwrap_or("");
            let role = model.get("role").and_then(Value::as_str).unwrap_or("");
            let (name, order) = (category_display_name(path), category_sort_order(role));
            if let Some(obj) = model.as_object_mut() {
                obj.insert("displayName".into(), Value::String(name));
                obj.insert("sortOrder".into(), json!(order));
            }
        }
        _ => {}
    }
}

/// [`annotate`] on an owned model.
pub fn annotated(class: &str, mut model: Value) -> Value {
    annotate(class, &mut model);
    model
}

fn first_email(message: &Value, key: &str) -> String {
    message
        .get(key)
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|c| c.get("email"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()
}

/// `tld.registered(email.split('@')[1] || '')`.
fn registered_domain(email: &str) -> Option<&str> {
    let domain = email.split('@').nth(1).unwrap_or("");
    psl::domain_str(domain)
}

/// A Message's warnings as `{code, message}` (`tern_ipc::MessageWarning`), none for most.
pub fn message_warnings(message: &Value) -> Vec<Value> {
    // Most messages have no Reply-To: check it first, so they cost one lookup.
    let reply_to = first_email(message, "replyTo");
    if reply_to.is_empty() {
        return Vec::new();
    }
    let from = first_email(message, "from");
    if from.is_empty() {
        return Vec::new();
    }
    if registered_domain(&from) == registered_domain(&reply_to) {
        return Vec::new();
    }
    vec![json!({
        "code": REPLY_TO_MISMATCH,
        "message": format!(
            "This message looks suspicious! It originates from {from} but replies will go to {reply_to}."
        ),
    })]
}

/// `Category.displayName`: the decoded path without the `INBOX`, `[Gmail]` or `[Mailspring]`
/// parent or the Tern / Mailspring container folder, `Inbox` for `INBOX`. Role names are the
/// client's to localize.
pub fn category_display_name(path: &str) -> String {
    let decoded = tern_core::utf7::decode_or_raw(path);
    for prefix in ["INBOX", "[Gmail]", "[Mailspring]"] {
        if let Some(rest) = decoded.strip_prefix(prefix) {
            // The delimiter, then at least one character.
            let mut chars = rest.chars();
            if chars.next().is_some() && !chars.as_str().is_empty() {
                return chars.as_str().to_owned();
            }
        }
    }
    for prefix in ["Tern/", "Tern.", "Mailspring/", "Mailspring."] {
        if let Some(rest) = decoded.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    if decoded == "INBOX" {
        return "Inbox".into();
    }
    decoded
}

/// Where a category sorts in the sidebar: its role's index in [`STANDARD_ROLES`], or the
/// length of that list for every other category, which clients order by `displayName`.
pub fn category_sort_order(role: &str) -> usize {
    STANDARD_ROLES
        .iter()
        .position(|r| *r == role)
        .unwrap_or(STANDARD_ROLES.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warned(from: &str, reply_to: &[&str]) -> Vec<String> {
        let reply_to: Vec<Value> = reply_to.iter().map(|e| json!({"email": e})).collect();
        message_warnings(&json!({"from": [{"email": from}], "replyTo": reply_to}))
            .into_iter()
            .map(|w| w["message"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn warns_when_replies_go_to_another_registered_domain() {
        assert_eq!(
            warned("billing@paypal.com", &["Support@PaypaI-help.net"]),
            ["This message looks suspicious! It originates from billing@paypal.com but replies will go to support@paypai-help.net."]
        );
        // The same registered domain, below its public suffix.
        assert!(warned("news@mail.example.co.uk", &["help@example.co.uk"]).is_empty());
        assert!(warned("a@example.com", &["B@EXAMPLE.COM"]).is_empty());
        // Only the first Reply-To counts.
        assert!(warned("a@example.com", &["b@example.com", "c@other.org"]).is_empty());
        assert_eq!(
            warned("a@x.example.com", &["b@other.org", "c@example.com"]).len(),
            1
        );
        // Sites under a private suffix are separate domains.
        assert_eq!(
            warned("a@alice.github.io", &["b@mallory.github.io"]).len(),
            1
        );
        assert_eq!(warned("a@x.test", &["b@y.test"]).len(), 1);
    }

    #[test]
    fn no_warning_without_both_addresses() {
        assert!(warned("a@example.com", &[]).is_empty());
        assert!(warned("", &["b@other.org"]).is_empty());
        assert!(message_warnings(&json!({"replyTo": [{"email": "b@other.org"}]})).is_empty());
        assert!(message_warnings(
            &json!({"from": [{"name": "A"}], "replyTo": [{"email": "b@o.org"}]})
        )
        .is_empty());
        // Neither has a registered domain: as the plugin's null === null.
        assert!(warned("root@localhost", &["admin@com"]).is_empty());
    }

    #[test]
    fn messages_get_hidden_and_warnings_only_when_there_are_some() {
        let m = annotated(
            "Message",
            json!({"id": "m", "from": [{"email": "a@example.com"}]}),
        );
        assert_eq!(m["hidden"], false);
        assert!(m.get("warnings").is_none());
        let m = annotated(
            "Message",
            json!({"id": "m", "from": [{"email": "a@example.com"}],
                   "replyTo": [{"email": "b@other.org"}]}),
        );
        assert_eq!(m["warnings"][0]["code"], REPLY_TO_MISMATCH);
        assert_eq!(
            annotated("Message", json!({"id": "deleted-1"}))["hidden"],
            true
        );
        assert_eq!(annotated("Thread", json!({"id": "t"})), json!({"id": "t"}));
    }

    #[test]
    fn category_names_as_the_desktop_shows_them() {
        let cases = [
            ("INBOX", "Inbox"),
            ("INBOX/Receipts", "Receipts"),
            ("INBOX.Receipts", "Receipts"),
            ("INBOX/", "INBOX/"),
            ("[Gmail]/All Mail", "All Mail"),
            ("[Mailspring]/Snoozed", "Snoozed"),
            ("Tern/Snoozed", "Snoozed"),
            ("Mailspring.Reminders", "Reminders"),
            ("Projects/&AMk-t&AOk-", "Projects/Été"),
            ("Work", "Work"),
        ];
        for (path, name) in cases {
            assert_eq!(category_display_name(path), name, "{path}");
        }
        let f = annotated(
            "Folder",
            json!({"id": "f", "path": "[Gmail]/Sent Mail", "role": "sent"}),
        );
        assert_eq!(
            (f["displayName"].as_str(), f["sortOrder"].as_u64()),
            (Some("Sent Mail"), Some(3))
        );
        let l = annotated("Label", json!({"id": "l", "path": "Work", "role": ""}));
        assert_eq!(l["sortOrder"], STANDARD_ROLES.len());
    }
}
