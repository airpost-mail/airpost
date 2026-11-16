// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Undo and redo built from the data a task's local phase wrote, and the revert of a mail
//! task whose remote phase failed for good.
//!
//! The inverses port the desktop's `createUndoTask` / `createUndoTasks`
//! (`apps/desktop/src/data/tasks/*`) and the redo copies its `createIdenticalTask`, with one
//! difference for the flag and label tasks: their local phase records each message it changed
//! (`undoFlags`, `undoLabels`), so the undo puts back exactly those messages' values rather
//! than flipping every message of the threads.

use std::collections::{BTreeMap, BTreeSet};

use airpost_core::constants::TASK_NEXT_ATTEMPT_KEY;
use airpost_core::Result;
use airpost_store::model::json_str;
use airpost_store::{Message, Model, Store, Task};
use serde_json::{json, Map, Value};
use tracing::info;

use crate::messages::{set_sorted_labels, xgm_key_for_label, UNDO_FLAGS, UNDO_LABELS};
use crate::{TaskProcessor, REMOTE_ATTEMPTS_KEY};

/// The mail tasks whose failed remote phase puts back what the server didn't take.
pub const REVERTED_CLASSES: [&str; 4] = [
    "ChangeUnreadTask",
    "ChangeStarredTask",
    "ChangeFolderTask",
    "ChangeLabelsTask",
];

/// Task data: set when a failed remote phase put the local change back.
pub const REVERTED_KEY: &str = "reverted";

/// The classes [`undo_tasks`] can reverse.
pub fn undoable_class(class: &str) -> bool {
    matches!(
        class,
        "ChangeUnreadTask"
            | "ChangeStarredTask"
            | "ChangeFolderTask"
            | "ChangeLabelsTask"
            | "SyncbackMetadataTask"
            | "SyncbackEventTask"
            | "ChangeContactGroupMembershipTask"
            | "SyncbackContactGroupTask"
            | "DestroyContactGroupTask"
    )
}

fn non_empty_object(v: &Value) -> bool {
    v.as_object().is_some_and(|o| !o.is_empty())
}

/// Whether the task, after its local phase, changed something its undo would put back. A
/// flag, label or move task that found every message already as asked has nothing to undo,
/// and neither has a task of a class [`undo_tasks`] doesn't know.
pub fn has_undo(task: &Value) -> bool {
    match json_str(task, "__cls") {
        "ChangeUnreadTask" | "ChangeStarredTask" => non_empty_object(&task[UNDO_FLAGS]),
        "ChangeLabelsTask" => non_empty_object(&task[UNDO_LABELS]),
        "ChangeFolderTask" => non_empty_object(&task["undoPlacements"]),
        "SyncbackMetadataTask" => task.get("undoValue").is_some_and(|v| !v.is_null()),
        "SyncbackEventTask" => task["undoData"].is_object(),
        "SyncbackContactGroupTask" => task["previousName"].is_string(),
        "ChangeContactGroupMembershipTask" | "DestroyContactGroupTask" => true,
        _ => false,
    }
}

/// The keys a task's runs write, which a copy queued again must not carry.
const RUN_KEYS: [&str; 13] = [
    "id",
    "status",
    "v",
    "error",
    "should_cancel",
    "createdAt",
    "undoPlacements",
    "restorePlacements",
    UNDO_FLAGS,
    UNDO_LABELS,
    REVERTED_KEY,
    "modelMetadataNewVersion",
    REMOTE_ATTEMPTS_KEY,
];

/// The task's own fields, without those its runs wrote.
fn fields(task: &Value) -> Map<String, Value> {
    let mut out = task.as_object().cloned().unwrap_or_default();
    for k in RUN_KEYS {
        out.remove(k);
    }
    out.remove(TASK_NEXT_ATTEMPT_KEY);
    out.remove("__cls");
    out.remove("aid");
    out
}

fn new_task(class: &str, account_id: &str, data: Map<String, Value>) -> Task {
    Task::create(class, account_id, &Value::Object(data))
}

/// The ids a mail task addresses, under the key it used.
fn target_of(task: &Value) -> (&'static str, Value) {
    if task.get("threadIds").is_some_and(non_empty_array) {
        ("threadIds", task["threadIds"].clone())
    } else {
        ("messageIds", task["messageIds"].clone())
    }
}

fn non_empty_array(v: &Value) -> bool {
    v.as_array().is_some_and(|a| !a.is_empty())
}

/// The fields every undo of a mail task carries.
fn undo_of_mail(task: &Value) -> Map<String, Value> {
    let mut m = Map::new();
    for key in ["source", "taskDescription"] {
        if let Some(v) = task.get(key).filter(|v| !v.is_null()) {
            m.insert(key.into(), v.clone());
        }
    }
    m.insert("isUndo".into(), json!(true));
    m.insert("canBeUndone".into(), json!(true));
    m
}

/// The tasks that reverse `task` (its JSON after the local phase), in the order to queue
/// them; empty when there is nothing to put back. A move goes back from its destination to
/// where each copy was shown; the flags and labels of exactly the messages it changed go
/// back to their values before.
pub fn undo_tasks(store: &Store, task: &Value, tz: Option<&str>) -> Result<Vec<Task>> {
    if !has_undo(task) {
        return Ok(Vec::new());
    }
    let class = json_str(task, "__cls");
    let aid = json_str(task, "aid");
    Ok(match class {
        "ChangeUnreadTask" | "ChangeStarredTask" => {
            let key = if class == "ChangeUnreadTask" {
                "unread"
            } else {
                "starred"
            };
            let mut by_value: BTreeMap<bool, Vec<String>> = BTreeMap::new();
            for (id, before) in task[UNDO_FLAGS].as_object().into_iter().flatten() {
                if let Some(b) = before.as_bool() {
                    by_value.entry(b).or_default().push(id.clone());
                }
            }
            by_value
                .into_iter()
                .map(|(value, ids)| {
                    let mut m = undo_of_mail(task);
                    m.insert("messageIds".into(), json!(ids));
                    m.insert(key.into(), json!(value));
                    new_task(class, aid, m)
                })
                .collect()
        }
        "ChangeLabelsTask" => {
            let mut label_json: BTreeMap<String, Value> = BTreeMap::new();
            for key in ["labelsToAdd", "labelsToRemove"] {
                for l in task[key].as_array().into_iter().flatten() {
                    if let Ok(k) = xgm_key_for_label(l) {
                        label_json.insert(k, l.clone());
                    }
                }
            }
            let mut groups: BTreeMap<(Vec<String>, Vec<String>), Vec<String>> = BTreeMap::new();
            for (id, change) in task[UNDO_LABELS].as_object().into_iter().flatten() {
                let keys = |k: &str| -> Vec<String> {
                    let mut v: Vec<String> = change[k]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect();
                    v.sort();
                    v
                };
                groups
                    .entry((keys("added"), keys("removed")))
                    .or_default()
                    .push(id.clone());
            }
            let as_json = |keys: &[String]| -> Vec<Value> {
                keys.iter()
                    .filter_map(|k| label_json.get(k).cloned())
                    .collect()
            };
            groups
                .into_iter()
                .map(|((added, removed), ids)| {
                    let mut m = undo_of_mail(task);
                    m.insert("messageIds".into(), json!(ids));
                    m.insert("labelsToAdd".into(), json!(as_json(&removed)));
                    m.insert("labelsToRemove".into(), json!(as_json(&added)));
                    new_task(class, aid, m)
                })
                .collect()
        }
        "ChangeFolderTask" => {
            // `folder` only describes the undo: each copy goes to its recorded folder. None
            // of them still existing leaves nothing to send back.
            let placements = &task["undoPlacements"];
            let mut first = None;
            'find: for entries in placements.as_object().into_iter().flatten().map(|(_, e)| e) {
                for e in entries.as_array().into_iter().flatten() {
                    if let Some(f) = store.folder_by_id(aid, json_str(e, "folderId"))? {
                        first = Some(f.to_json());
                        break 'find;
                    }
                }
            }
            let Some(folder) = first else {
                return Ok(Vec::new());
            };
            let mut m = undo_of_mail(task);
            let (key, ids) = target_of(task);
            m.insert(key.into(), ids);
            m.insert("folder".into(), folder);
            m.insert(
                "sourceFolderIds".into(),
                json!([json_str(&task["folder"], "id")]),
            );
            m.insert("restorePlacements".into(), placements.clone());
            vec![new_task(class, aid, m)]
        }
        "SyncbackMetadataTask" => {
            let mut m = fields(task);
            m.insert("value".into(), task["undoValue"].clone());
            m.insert("undoValue".into(), task["value"].clone());
            vec![new_task(class, aid, m)]
        }
        "SyncbackEventTask" => {
            let undo = &task["undoData"];
            let mut event = task["event"].clone();
            for key in ["ics", "recurrenceStart", "recurrenceEnd"] {
                event[key] = undo[key].clone();
            }
            let description = task["taskDescription"]
                .as_str()
                .filter(|d| !d.is_empty())
                .unwrap_or("event change");
            let mut m = Map::new();
            m.insert("event".into(), event);
            m.insert("calendarId".into(), task["calendarId"].clone());
            m.insert("undoData".into(), task["newData"].clone());
            m.insert("newData".into(), undo.clone());
            m.insert(
                "taskDescription".into(),
                json!(format!("Undo {description}")),
            );
            m.insert("restore".into(), json!(true));
            if let Some(tz) = tz.or_else(|| task["tz"].as_str()) {
                m.insert("tz".into(), json!(tz));
            }
            vec![new_task(class, aid, m)]
        }
        "ChangeContactGroupMembershipTask" => {
            let mut m = fields(task);
            let back = if json_str(task, "direction") == "remove" {
                "add"
            } else {
                "remove"
            };
            m.insert("direction".into(), json!(back));
            vec![new_task(class, aid, m)]
        }
        "SyncbackContactGroupTask" => {
            let mut group = task["group"].clone();
            let name = group["name"].clone();
            group["name"] = task["previousName"].clone();
            let mut m = Map::new();
            m.insert("group".into(), group);
            m.insert("previousName".into(), name);
            vec![new_task(class, aid, m)]
        }
        "DestroyContactGroupTask" => {
            let group = json!({
                "__cls": "ContactGroup",
                "aid": aid,
                "name": task["group"]["name"],
            });
            let mut m = Map::new();
            m.insert("group".into(), group);
            vec![new_task("SyncbackContactGroupTask", aid, m)]
        }
        _ => Vec::new(),
    })
}

/// The tasks that make `task`'s change again after an undo: a copy without what its run
/// wrote, and for an event the state it wrote, re-sent as an undo is (`restore`).
pub fn redo_tasks(task: &Value, tz: Option<&str>) -> Vec<Task> {
    let class = json_str(task, "__cls");
    let aid = json_str(task, "aid");
    let mut m = fields(task);
    if class == "SyncbackEventTask" && task["newData"].is_object() {
        let new = &task["newData"];
        let mut event = task["event"].clone();
        for key in ["ics", "recurrenceStart", "recurrenceEnd"] {
            event[key] = new[key].clone();
        }
        m.insert("event".into(), event);
        m.insert("restore".into(), json!(true));
        if let Some(tz) = tz.or_else(|| task["tz"].as_str()) {
            m.insert("tz".into(), json!(tz));
        }
    }
    vec![new_task(class, aid, m)]
}

impl TaskProcessor<'_> {
    /// After a mail task's remote phase failed for good: the flags and labels of the
    /// messages none of whose copies the server changed go back to what they were before the
    /// local phase (moves the server didn't make are already back, see
    /// `perform_remote_change_on_messages`). A message another task still holds is left to
    /// that task, and one the server changed in part to the folders' next scan. Marks the
    /// task `reverted`.
    pub(crate) fn revert_local_change(
        &self,
        task: &mut Task,
        reached: &BTreeSet<String>,
    ) -> Result<()> {
        let class = task.class().to_owned();
        let data = &task.data;
        let pending = |key: &str| -> Vec<(String, Value)> {
            data[key]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(id, _)| !reached.contains(*id))
                .map(|(id, v)| (id.clone(), v.clone()))
                .collect()
        };
        let changes = match class.as_str() {
            "ChangeUnreadTask" | "ChangeStarredTask" => pending(UNDO_FLAGS),
            "ChangeLabelsTask" => pending(UNDO_LABELS),
            _ => Vec::new(),
        };
        let ids: Vec<String> = changes.iter().map(|(id, _)| id.clone()).collect();
        let reverted = self.store.transaction("revertLocalChange", |s| {
            let mut n = 0;
            let by_id: BTreeMap<String, Value> = changes.iter().cloned().collect();
            for mut msg in s.find_large_set::<Message>("id", &ids)? {
                if msg.sync_unsaved_changes() > 0 {
                    continue;
                }
                let Some(change) = by_id.get(msg.id()) else {
                    continue;
                };
                match class.as_str() {
                    "ChangeUnreadTask" => {
                        s.set_placement_unread(&mut msg, change.as_bool().unwrap_or(false))?
                    }
                    "ChangeStarredTask" => {
                        s.set_placement_starred(&mut msg, change.as_bool().unwrap_or(false))?
                    }
                    _ => {
                        let keys = |k: &str| -> Vec<String> {
                            change[k]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|s| s.as_str().map(str::to_owned))
                                .collect()
                        };
                        let (added, removed) = (keys("added"), keys("removed"));
                        let mut labels: Vec<String> = msg
                            .labels()
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|l| l.as_str().map(str::to_owned))
                            .filter(|l| !added.contains(l))
                            .collect();
                        for l in removed {
                            if !labels.contains(&l) {
                                labels.push(l);
                            }
                        }
                        set_sorted_labels(s, &mut msg, labels)?;
                    }
                }
                s.save(&mut msg)?;
                n += 1;
            }
            Ok(n)
        })?;
        info!(
            "[{}] -- Put back the local change the server did not take, on {reverted} of its \
             messages",
            task.id()
        );
        task.data[REVERTED_KEY] = json!(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(task: &Task, key: &str) -> Vec<String> {
        task.data[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let clock = airpost_core::ManualClock::new(1_700_000_000);
        airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let store = Store::open(
            dir.path(),
            airpost_store::DeltaStream::new(Box::new(std::io::sink())),
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn flags_go_back_per_message() {
        let (_d, store) = store();
        let task = json!({"__cls": "ChangeUnreadTask", "aid": "a", "id": "t", "status": "remote",
                          "threadIds": ["th"], "unread": false, "source": "x",
                          "undoFlags": {"m1": true, "m2": true}});
        let undo = undo_tasks(&store, &task, None).unwrap();
        assert_eq!(undo.len(), 1);
        assert_eq!(undo[0].class(), "ChangeUnreadTask");
        assert_eq!(ids(&undo[0], "messageIds"), ["m1", "m2"]);
        assert_eq!(undo[0].data["unread"], json!(true));
        assert_eq!(undo[0].data["isUndo"], json!(true));
        assert!(undo[0].data.get("threadIds").is_none());

        let nothing = json!({"__cls": "ChangeStarredTask", "aid": "a", "starred": true,
                             "threadIds": ["th"], "undoFlags": {}});
        assert!(!has_undo(&nothing));
        assert!(undo_tasks(&store, &nothing, None).unwrap().is_empty());
    }

    #[test]
    fn labels_go_back_by_what_each_message_gained_and_lost() {
        let (_d, store) = store();
        let inbox = json!({"id": "li", "role": "inbox", "path": "INBOX"});
        let work = json!({"id": "lw", "role": "", "path": "Work"});
        let task = json!({"__cls": "ChangeLabelsTask", "aid": "a", "threadIds": ["th"],
        "labelsToAdd": [work], "labelsToRemove": [inbox],
        "undoLabels": {
            "m1": {"added": ["Work"], "removed": ["\\Inbox"]},
            "m2": {"added": ["Work"]},
        }});
        let undo = undo_tasks(&store, &task, None).unwrap();
        assert_eq!(undo.len(), 2);
        let both = undo
            .iter()
            .find(|t| ids(t, "messageIds") == ["m1"])
            .unwrap();
        assert_eq!(both.data["labelsToAdd"], json!([inbox]));
        assert_eq!(both.data["labelsToRemove"], json!([work]));
        let only_added = undo
            .iter()
            .find(|t| ids(t, "messageIds") == ["m2"])
            .unwrap();
        assert_eq!(only_added.data["labelsToAdd"], json!([]));
        assert_eq!(only_added.data["labelsToRemove"], json!([work]));
    }

    #[test]
    fn metadata_swaps_its_values_and_events_restore() {
        let (_d, store) = store();
        let meta = json!({"__cls": "SyncbackMetadataTask", "aid": "a", "id": "t", "v": 3,
                          "status": "complete", "modelId": "th", "modelClassName": "thread",
                          "pluginId": "thread-snooze", "value": {"expiration": 5},
                          "undoValue": {"expiration": null}, "modelMetadataNewVersion": 2});
        let undo = undo_tasks(&store, &meta, None).unwrap();
        assert_eq!(undo[0].data["value"], json!({"expiration": null}));
        assert_eq!(undo[0].data["undoValue"], json!({"expiration": 5}));
        assert!(undo[0].data.get("modelMetadataNewVersion").is_none());
        assert_ne!(undo[0].data["id"], json!("t"));
        assert_eq!(undo[0].data["status"], json!("local"));

        let no_undo = json!({"__cls": "SyncbackMetadataTask", "aid": "a", "value": {}});
        assert!(!has_undo(&no_undo));

        let event = json!({"__cls": "SyncbackEventTask", "aid": "a", "calendarId": "c",
                           "event": {"id": "e", "ics": "NEW", "recurrenceStart": 2, "recurrenceEnd": 3},
                           "undoData": {"ics": "OLD", "recurrenceStart": 0, "recurrenceEnd": 1},
                           "newData": {"ics": "NEW", "recurrenceStart": 2, "recurrenceEnd": 3},
                           "taskDescription": "Move event"});
        let undo = undo_tasks(&store, &event, Some("Europe/Paris")).unwrap();
        let u = &undo[0].data;
        assert_eq!(u["event"]["ics"], json!("OLD"));
        assert_eq!(u["undoData"]["ics"], json!("NEW"));
        assert_eq!(u["newData"]["ics"], json!("OLD"));
        assert_eq!(
            (u["restore"].clone(), u["tz"].clone()),
            (json!(true), json!("Europe/Paris"))
        );
        assert_eq!(u["taskDescription"], json!("Undo Move event"));
        let redo = redo_tasks(&event, None);
        assert_eq!(redo[0].data["event"]["ics"], json!("NEW"));
        assert_eq!(redo[0].data["restore"], json!(true));
    }

    #[test]
    fn contact_groups_reverse() {
        let (_d, store) = store();
        let membership = json!({"__cls": "ChangeContactGroupMembershipTask", "aid": "a",
                                "direction": "add", "contacts": [{"id": "c"}], "group": {"id": "g"}});
        assert_eq!(
            undo_tasks(&store, &membership, None).unwrap()[0].data["direction"],
            json!("remove")
        );
        let destroy = json!({"__cls": "DestroyContactGroupTask", "aid": "a",
                             "group": {"id": "g", "name": "Friends"}});
        let undo = undo_tasks(&store, &destroy, None).unwrap();
        assert_eq!(undo[0].class(), "SyncbackContactGroupTask");
        assert_eq!(undo[0].data["group"]["name"], json!("Friends"));
        assert!(undo[0].data["group"].get("id").is_none());
        let rename = json!({"__cls": "SyncbackContactGroupTask", "aid": "a",
                            "group": {"id": "g", "name": "New"}, "previousName": "Old"});
        let undo = undo_tasks(&store, &rename, None).unwrap();
        assert_eq!(undo[0].data["group"]["name"], json!("Old"));
        assert_eq!(undo[0].data["previousName"], json!("New"));
        let create =
            json!({"__cls": "SyncbackContactGroupTask", "aid": "a", "group": {"name": "X"}});
        assert!(!has_undo(&create));
    }

    #[test]
    fn a_redo_copy_drops_what_the_run_wrote() {
        let task = json!({"__cls": "ChangeFolderTask", "aid": "a", "id": "t", "v": 2,
                          "status": "complete", "threadIds": ["th"], "folder": {"id": "f"},
                          "undoPlacements": {"m": []}, "_remoteAttempts": 2,
                          "_nextAttemptAt": 5, "error": null});
        let redo = redo_tasks(&task, None);
        let d = &redo[0].data;
        assert_eq!(redo[0].class(), "ChangeFolderTask");
        assert_eq!(d["threadIds"], json!(["th"]));
        for gone in [
            "undoPlacements",
            "_remoteAttempts",
            "_nextAttemptAt",
            "error",
        ] {
            assert!(d.get(gone).is_none(), "{gone}");
        }
        assert_ne!(d["id"], json!("t"));
    }
}
