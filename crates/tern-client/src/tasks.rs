// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Tasks: the JSON the desktop queues for the same actions (`Task.toJSON()` of the classes
//! in `apps/desktop/src/data/tasks`), with attribute keys as their `jsonKey` (`aid`, `v`),
//! undefined attributes left out, `status: "local"` as the bridge sets it before queueing,
//! and `__cls`; and the check a client waiting on a task makes. A task is done when a `Task`
//! delta shows it `complete` or `cancelled`, as the desktop's `TaskQueue` decides; its
//! `error` says whether it failed. `apps/mcp/tests/fixtures/*.json` hold the expected shapes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use tern_ipc::DeltaParams;

/// `source` on every task tern-mcp queues, as the desktop MCP plugin's.
pub const SOURCE: &str = "MCP";
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

/// `SyncbackCategoryTask.forCreating`: a folder, or a label on Gmail (the engine decides by
/// the account). `name` has `/` between levels; the engine writes the server's delimiter
/// and namespace prefix. The finished task's `created` is the new Folder or Label.
pub fn create_category(account_id: &str, name: &str) -> Value {
    task(
        "SyncbackCategoryTask",
        account_id,
        json!({"path": crate::utf7::encode(name)}),
    )
}

/// `SyncbackCategoryTask.forRenaming`: `existing_path` is the stored `path`, `new_name` the
/// whole new name, as for [`create_category`].
pub fn rename_category(account_id: &str, existing_path: &str, new_name: &str) -> Value {
    task(
        "SyncbackCategoryTask",
        account_id,
        json!({"existingPath": existing_path, "path": crate::utf7::encode(new_name)}),
    )
}

/// `DestroyCategoryTask` on the stored `path`. On a folder account the server deletes the
/// mail in it too.
pub fn destroy_category(account_id: &str, path: &str) -> Value {
    task("DestroyCategoryTask", account_id, json!({"path": path}))
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
            "tern-cli",
        );
        assert!(t["id"].as_str().unwrap().starts_with("tern-cli-"));
        assert_eq!(
            (t["aid"].as_str(), t["status"].as_str()),
            (Some("acc"), Some("local"))
        );
        let t = complete_task(
            json!({"__cls": "X", "id": "mine", "aid": "other"}),
            "acc",
            "tern-cli",
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
