// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `warnings` on the messages the read API returns (`feature::MESSAGE_WARNINGS`): the
//! desktop's `phishing-detection` plugin (`lib/main.tsx`), which warns when the first
//! Reply-To address is in another registered domain than the first From address.
//!
//! The plugin finds registered domains with the `tld` package's 2011 copy of the Public
//! Suffix List, which reads a domain under a suffix it doesn't know (`.app`, `.dev`, `.test`)
//! as none, so two such domains always match; the `psl` crate's current list reads them as
//! their last label, as the list's default rule does.

use serde_json::Value;
use tern_ipc::MessageWarning;

pub const REPLY_TO_MISMATCH: &str = "reply-to-mismatch";

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

/// The warnings for a Message's JSON, none for most.
pub fn message_warnings(message: &Value) -> Vec<MessageWarning> {
    let from = first_email(message, "from");
    let reply_to = first_email(message, "replyTo");
    if from.is_empty() || reply_to.is_empty() {
        return Vec::new();
    }
    if registered_domain(&from) == registered_domain(&reply_to) {
        return Vec::new();
    }
    vec![MessageWarning {
        code: REPLY_TO_MISMATCH.to_owned(),
        message: format!(
            "This message looks suspicious! It originates from {from} but replies will go to {reply_to}."
        ),
    }]
}

/// The JSON with `warnings` set when there are any.
pub fn with_warnings(mut message: Value) -> Value {
    let warnings = message_warnings(&message);
    if let (false, Some(obj)) = (warnings.is_empty(), message.as_object_mut()) {
        obj.insert(
            "warnings".into(),
            serde_json::to_value(warnings).unwrap_or(Value::Null),
        );
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn warned(from: &str, reply_to: &[&str]) -> Vec<String> {
        let reply_to: Vec<Value> = reply_to.iter().map(|e| json!({"email": e})).collect();
        message_warnings(&json!({"from": [{"email": from}], "replyTo": reply_to}))
            .into_iter()
            .map(|w| w.message)
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
    fn sets_warnings_only_when_there_are_some() {
        let m = with_warnings(json!({"id": "m", "from": [{"email": "a@example.com"}]}));
        assert!(m.get("warnings").is_none());
        let m = with_warnings(json!({"id": "m", "from": [{"email": "a@example.com"}],
                                     "replyTo": [{"email": "b@other.org"}]}));
        assert_eq!(m["warnings"][0]["code"], REPLY_TO_MISMATCH);
    }
}
