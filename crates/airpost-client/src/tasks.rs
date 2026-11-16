// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Tasks: the JSON the desktop queues for drafts, metadata and exports (`Task.toJSON()` of
//! the classes in `apps/desktop/src/data/tasks`), with attribute keys as their `jsonKey`
//! (`aid`, `v`), undefined attributes left out, `status: "local"` as the bridge sets it
//! before queueing, and `__cls`; and the check a client waiting on a task makes. A task is
//! done when a `Task` delta shows it `complete` or `cancelled`, as the desktop's `TaskQueue`
//! decides; its `error` says whether it failed. `apps/mcp/tests/fixtures/*.json` hold the
//! expected shapes. Mail and folder changes aren't built here: they are the engine's
//! intents (`threads.*`, `categories.*`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use airpost_ipc::DeltaParams;
use serde_json::{json, Map, Value};

/// The `__cls` of a send, for `tasks.list`'s `class`.
pub const SEND_DRAFT_TASK: &str = "SendDraftTask";

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn bool_of(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

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

pub fn syncback_draft(draft: Value) -> Value {
    let account = str_of(&draft, "aid").to_owned();
    let hmid = str_of(&draft, "hMsgId").to_owned();
    task(
        "SyncbackDraftTask",
        &account,
        json!({"headerMessageId": hmid, "draft": draft}),
    )
}

/// `SendDraftTask.forSending(draft, {silent: true})` with no composer extensions: the body
/// goes as it is (plaintext drafts lose their zero-width spaces).
pub fn send_draft(mut draft: Value) -> Value {
    if bool_of(&draft, "plaintext") {
        if let Some(Value::String(b)) = draft.get_mut("body") {
            *b = b.replace('\u{200B}', "");
        }
    }
    let account = str_of(&draft, "aid").to_owned();
    let hmid = str_of(&draft, "hMsgId").to_owned();
    task(
        SEND_DRAFT_TASK,
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
    let hmid = str_of(message, "hMsgId");
    let mut fields = json!({
        "pluginId": plugin_id,
        "modelId": str_of(message, "id"),
        "modelClassName": "message",
        "modelHeaderMessageId": if hmid.is_empty() { Value::Null } else { Value::from(hmid) },
        "value": value,
    });
    if let Some(u) = undo_value {
        fields["undoValue"] = u;
    }
    task("SyncbackMetadataTask", str_of(message, "aid"), fields)
}

/// `SyncbackMetadataTask.forSaving` on a Thread: `value` replaces the plugin's value, and a
/// value without `expiration` ends any expiration it had.
pub fn syncback_thread_metadata(
    account_id: &str,
    thread_id: &str,
    plugin_id: &str,
    value: Value,
) -> Value {
    task(
        "SyncbackMetadataTask",
        account_id,
        json!({
            "pluginId": plugin_id,
            "modelId": thread_id,
            "modelClassName": "thread",
            "modelHeaderMessageId": null,
            "value": value,
        }),
    )
}

/// `GetMessageRFC2822Task`: the engine fetches the message's raw bytes from the server and
/// writes them to `filepath` (absolute), dated like the message.
pub fn get_message_rfc2822(account_id: &str, message_id: &str, filepath: &str) -> Value {
    task(
        "GetMessageRFC2822Task",
        account_id,
        json!({"messageId": message_id, "filepath": filepath}),
    )
}

/// `GetManyRFC2822Task` as the desktop's .eml folder export queues it: one
/// `{index:05} - {subject} - {date}.eml` per message of the folder in `outputDir`, in UID
/// order. Without `format: "mbox"`, so the desktop's mbox runner leaves it alone.
pub fn get_many_rfc2822(account_id: &str, folder: &Value, output_dir: &str) -> Value {
    task(
        "GetManyRFC2822Task",
        account_id,
        json!({
            "folderId": str_of(folder, "id"),
            "folderPath": str_of(folder, "path"),
            "outputDir": output_dir,
        }),
    )
}

/// A task id no other client picks: `prefix`, the pid, the time in nanoseconds and a
/// counter (the clock may not tick between two calls).
pub fn new_task_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{nanos:x}-{seq}", std::process::id())
}

/// Fills in what the desktop's `Task` model always sends: `id` (from [`new_task_id`] with
/// `id_prefix`), `aid` and `status`. Keys already set are kept.
pub fn complete_task(mut task: Value, account_id: &str, id_prefix: &str) -> Value {
    if let Some(obj) = task.as_object_mut() {
        obj.entry("id")
            .or_insert_with(|| json!(new_task_id(id_prefix)));
        obj.entry("aid").or_insert_with(|| json!(account_id));
        obj.entry("status").or_insert_with(|| json!("local"));
    }
    task
}

/// The class of the deltas a task wait watches.
pub const TASK_CLASS: &str = "Task";

/// Whether the stored task has finished, one way or the other.
pub fn is_final(task: &Value) -> bool {
    matches!(task["status"].as_str(), Some("complete" | "cancelled"))
}

/// Whether the task is still queued or running (`local` or `remote`).
pub fn is_pending(task: &Value) -> bool {
    matches!(task["status"].as_str(), Some("local" | "remote"))
}

/// Task `id` in a `Task` delta once it is final, or removed.
pub fn finished(d: &DeltaParams, id: &str) -> Option<Value> {
    if d.model_class != TASK_CLASS {
        return None;
    }
    d.model_jsons
        .iter()
        .find(|t| t["id"].as_str() == Some(id))
        .filter(|t| d.kind == "unpersist" || is_final(t))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(kind: &str, class: &str, models: Vec<Value>) -> DeltaParams {
        DeltaParams {
            account_id: "a".into(),
            seq: None,
            model_class: class.into(),
            kind: kind.into(),
            model_jsons: models,
        }
    }

    #[test]
    fn completes_the_task_json() {
        let t = complete_task(
            json!({"__cls": "ChangeStarredTask", "starred": true}),
            "acc",
            "airpost-cli",
        );
        assert!(t["id"].as_str().unwrap().starts_with("airpost-cli-"));
        assert_eq!(
            (t["aid"].as_str(), t["status"].as_str()),
            (Some("acc"), Some("local"))
        );
        let t = complete_task(
            json!({"__cls": "X", "id": "mine", "aid": "other"}),
            "acc",
            "airpost-cli",
        );
        assert_eq!(
            (t["id"].as_str(), t["aid"].as_str()),
            (Some("mine"), Some("other"))
        );
        assert_ne!(new_task_id("p"), new_task_id("p"));
    }

    #[test]
    fn finished_only_for_the_task_in_a_final_state() {
        let local = json!({"id": "t1", "status": "remote"});
        let done = json!({"id": "t1", "status": "complete", "error": null});
        assert!(is_pending(&local) && !is_final(&local));
        assert!(finished(&delta("persist", "Task", vec![local.clone()]), "t1").is_none());
        assert!(finished(&delta("persist", "Thread", vec![done.clone()]), "t1").is_none());
        assert!(finished(&delta("persist", "Task", vec![done.clone()]), "t2").is_none());
        assert_eq!(
            finished(
                &delta("persist", "Task", vec![json!({"id": "x"}), done.clone()]),
                "t1"
            ),
            Some(done)
        );
        assert!(finished(&delta("unpersist", "Task", vec![json!({"id": "t1"})]), "t1").is_some());
    }

    #[test]
    fn temp_ids_look_like_the_desktops() {
        let id = temp_id();
        assert!(
            id.starts_with("local-") && id.len() == "local-xxxxxxxx-xxxx".len(),
            "{id}"
        );
    }
}
