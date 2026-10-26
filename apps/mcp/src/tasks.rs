// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The task JSON the desktop queues for the same actions, from `tern-client`, checked here
//! against `tests/fixtures/*.json`.

pub use tern_client::tasks::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// Compares with a fixture after blanking the fields that differ per call.
    fn assert_matches_fixture(mut actual: Value, fixture: &str) {
        let expected: Value = serde_json::from_str(fixture).unwrap();
        let id = actual["id"].as_str().unwrap().to_owned();
        assert!(
            id.starts_with("local-") && id.len() == "local-xxxxxxxx-xxxx".len(),
            "{id}"
        );
        actual["id"] = Value::from("<temp-id>");
        assert_eq!(actual, expected);
    }

    #[test]
    fn draft_tasks_match_the_desktop_shapes() {
        let draft = json!({"id": "d1", "aid": "a1", "hMsgId": "H@x.test", "body": "<p>x\u{200B}</p>",
                           "plaintext": false, "draft": true, "__cls": "Message"});
        assert_matches_fixture(
            syncback_draft(draft.clone()),
            include_str!("../tests/fixtures/syncback-draft-task.json"),
        );
        assert_matches_fixture(
            send_draft(draft.clone()),
            include_str!("../tests/fixtures/send-draft-task.json"),
        );
        let mut plain = draft.clone();
        plain["plaintext"] = Value::Bool(true);
        assert_eq!(send_draft(plain)["draft"]["body"], "<p>x</p>");
        assert_matches_fixture(
            destroy_draft("a1", &["d1".into()]),
            include_str!("../tests/fixtures/destroy-draft-task.json"),
        );
        assert_matches_fixture(
            syncback_message_metadata(
                &draft,
                "send-later",
                json!({"expiration": 1_900_000_000, "actionKey": "send"}),
                Some(json!({"expiration": null})),
            ),
            include_str!("../tests/fixtures/syncback-metadata-task.json"),
        );
        let cancel =
            syncback_message_metadata(&draft, "send-later", json!({"expiration": null}), None);
        assert!(cancel.get("undoValue").is_none());
    }
}
