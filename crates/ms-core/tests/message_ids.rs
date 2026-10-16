// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Golden message ids. `message_ids.json` was produced by an independent Python
//! implementation (hashlib, `email.header`, a hand-written base58); see the plan, section 5.
//! Changing an expected id means every existing message gets a new id on resync. The cases
//! with a Bcc pin that Bcc is not a participant (docs/fix-backlog.md F1).

use base64::Engine;
use ms_core::ids::{message_id, message_id_source, MessageIdInput};
use ms_core::{Address, MessageHeader};
use serde_json::Value;

#[test]
fn golden_message_ids() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("message_ids.json")).expect("valid golden file");
    assert!(cases.len() >= 20);
    for c in &cases {
        let name = c["name"].as_str().unwrap();
        let subject: Option<Vec<u8>> = if let Some(b) = c.get("subject_base64") {
            Some(
                base64::engine::general_purpose::STANDARD
                    .decode(b.as_str().unwrap())
                    .unwrap(),
            )
        } else {
            c.get("subject")
                .map(|s| s.as_str().unwrap().as_bytes().to_vec())
        };
        // Through `MessageHeader::recipient_emails`, which picks the participants: the
        // cases with a Bcc pin that it is left out.
        let addresses = |key: &str| -> Vec<Address> {
            c.get(key)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .map(|v| Address::new(None, v.as_str().unwrap()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let header = MessageHeader {
            to: addresses("to"),
            cc: addresses("cc"),
            bcc: addresses("bcc"),
            ..Default::default()
        };
        let recipients = header.recipient_emails();
        let input = MessageIdInput {
            account_id: c["account_id"].as_str().unwrap(),
            date: c.get("date").and_then(Value::as_str),
            internal_date: c.get("internal_date").and_then(Value::as_i64),
            subject: subject.as_deref(),
            recipients: &recipients,
            message_id: c.get("message_id").and_then(Value::as_str),
            folder_path: c["folder_path"].as_str().unwrap(),
            uid: c["uid"].as_u64().unwrap() as u32,
        };
        assert_eq!(
            message_id_source(&input),
            c["source"].as_str().unwrap(),
            "source of {name}"
        );
        assert_eq!(
            message_id(&input),
            c["id"].as_str().unwrap(),
            "id of {name}"
        );
    }
}
