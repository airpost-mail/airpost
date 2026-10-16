// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The task JSON the desktop queues for the same actions (`Task.toJSON()` of the classes
//! in `apps/desktop/app/src/flux/tasks`): attribute keys as their `jsonKey` (`aid`, `v`),
//! undefined attributes left out, `status: "local"` as the bridge sets it before queueing,
//! and `__cls`. `tests/fixtures/*.json` hold the expected shapes.

use serde_json::{json, Map, Value};

/// `source` on every task tern-mcp queues, as the plugin's.
pub const SOURCE: &str = "MCP";

/// `generateTempId()`: `local-xxxxxxxx-xxxx`.
pub fn temp_id() -> String {
    let hex = uuid::Uuid::new_v4().simple().to_string();
    format!("local-{}-{}", &hex[..8], &hex[8..12])
}

fn task(class: &str, account_id: &str, fields: Value) -> Value {
    let mut t = Map::new();
    t.insert("id".into(), Value::from(temp_id()));
    t.insert("aid".into(), Value::from(account_id));
    t.insert("status".into(), Value::from("local"));
    if let Value::Object(f) = fields {
        t.extend(f);
    }
    t.insert("__cls".into(), Value::from(class));
    Value::Object(t)
}

/// The `ChangeMailTask` attributes every thread task carries.
fn change_mail(class: &str, account_id: &str, thread_ids: &[String], extra: Value) -> Value {
    let mut fields = json!({
        "source": SOURCE,
        "threadIds": thread_ids,
        "messageIds": [],
        "canBeUndone": true,
    });
    if let (Value::Object(f), Value::Object(e)) = (&mut fields, extra) {
        f.extend(e);
    }
    task(class, account_id, fields)
}

pub fn change_starred(account_id: &str, thread_ids: &[String], starred: bool) -> Value {
    change_mail(
        "ChangeStarredTask",
        account_id,
        thread_ids,
        json!({"starred": starred}),
    )
}

/// `TaskFactory.taskForSettingUnread`.
pub fn change_unread(account_id: &str, thread_ids: &[String], unread: bool) -> Value {
    change_mail(
        "ChangeUnreadTask",
        account_id,
        thread_ids,
        json!({"unread": unread}),
    )
}

/// Labels are the stored Label JSON from `labels.list`.
pub fn change_labels(
    account_id: &str,
    thread_ids: &[String],
    add: Vec<Value>,
    remove: Vec<Value>,
) -> Value {
    change_mail(
        "ChangeLabelsTask",
        account_id,
        thread_ids,
        json!({"labelsToAdd": add, "labelsToRemove": remove}),
    )
}

/// A move with no perspective, as MCP and mail rules queue it: `sourceFolderIds` empty, so
/// the engine moves every copy outside Sent and Drafts. `folder` is the stored Folder JSON.
pub fn change_folder(account_id: &str, thread_ids: &[String], folder: Value) -> Value {
    change_mail(
        "ChangeFolderTask",
        account_id,
        thread_ids,
        json!({"folder": folder, "sourceFolderIds": []}),
    )
}

pub fn syncback_draft(draft: Value) -> Value {
    let account = crate::model::str_of(&draft, "aid").to_owned();
    let hmid = crate::model::str_of(&draft, "hMsgId").to_owned();
    task(
        "SyncbackDraftTask",
        &account,
        json!({"headerMessageId": hmid, "draft": draft}),
    )
}

/// `SendDraftTask.forSending(draft, {silent: true})` with no composer extensions: the body
/// goes as it is (plaintext drafts lose their zero-width spaces).
pub fn send_draft(mut draft: Value) -> Value {
    if crate::model::bool_of(&draft, "plaintext") {
        if let Some(Value::String(b)) = draft.get_mut("body") {
            *b = b.replace('\u{200B}', "");
        }
    }
    let account = crate::model::str_of(&draft, "aid").to_owned();
    let hmid = crate::model::str_of(&draft, "hMsgId").to_owned();
    task(
        "SendDraftTask",
        &account,
        json!({"draft": draft, "headerMessageId": hmid, "silent": true}),
    )
}

pub fn destroy_draft(account_id: &str, message_ids: &[String]) -> Value {
    task(
        "DestroyDraftTask",
        account_id,
        json!({"messageIds": message_ids}),
    )
}

/// `SyncbackMetadataTask.forSaving` on a Message. `expiration` values are epoch seconds
/// already, as `forSaving` rounds them to.
pub fn syncback_message_metadata(
    message: &Value,
    plugin_id: &str,
    value: Value,
    undo_value: Option<Value>,
) -> Value {
    let hmid = crate::model::str_of(message, "hMsgId");
    let mut fields = json!({
        "pluginId": plugin_id,
        "modelId": crate::model::str_of(message, "id"),
        "modelClassName": "message",
        "modelHeaderMessageId": if hmid.is_empty() { Value::Null } else { Value::from(hmid) },
        "value": value,
    });
    if let Some(u) = undo_value {
        fields["undoValue"] = u;
    }
    task(
        "SyncbackMetadataTask",
        crate::model::str_of(message, "aid"),
        fields,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn ids() -> Vec<String> {
        vec!["t1".into(), "t2".into()]
    }

    #[test]
    fn thread_tasks_match_the_desktop_shapes() {
        assert_matches_fixture(
            change_starred("a1", &ids(), true),
            include_str!("../tests/fixtures/change-starred-task.json"),
        );
        assert_matches_fixture(
            change_unread("a1", &ids(), false),
            include_str!("../tests/fixtures/change-unread-task.json"),
        );
        let inbox = json!({"id": "l-inbox", "aid": "a1", "v": 1, "path": "\\Inbox", "role": "inbox", "__cls": "Label"});
        assert_matches_fixture(
            change_labels("a1", &ids(), vec![], vec![inbox]),
            include_str!("../tests/fixtures/change-labels-task.json"),
        );
        let archive = json!({"id": "f-archive", "aid": "a1", "v": 1, "path": "Archive", "role": "archive", "__cls": "Folder"});
        assert_matches_fixture(
            change_folder("a1", &ids(), archive),
            include_str!("../tests/fixtures/change-folder-task.json"),
        );
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
