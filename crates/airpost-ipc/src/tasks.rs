// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Typed views of the task JSON `task.queue` takes and `tasks.list`, `tasks.get` and `Task`
//! deltas return, one struct per `__cls` the engine runs. Like the model views
//! ([`crate::models`]) they describe the JSON without owning it: `extra` keeps keys a struct
//! doesn't name, and the engine still takes the task as a `Value`.
//!
//! `crates/tern-client/tests/task_views.rs` checks that the task builders the CLI and MCP
//! use produce JSON that fits these structs' schema.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::MessageJson;
use crate::models::{ContactGroupJson, ContactJson, EventJson, Extra, FolderJson, LabelJson};

/// A task's `status`: `local` until its local phase ran, `remote` until the server took
/// it, then `complete` or `cancelled`.
pub mod task_status {
    named_consts! { &str;
        LOCAL = "local";
        REMOTE = "remote";
        COMPLETE = "complete";
        CANCELLED = "cancelled";
    }
}

macro_rules! task_json {
    ($(#[doc = $doc:literal])* $name:ident, $cls:literal { $($fields:tt)* }) => {
        $(#[doc = $doc])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase")]
        pub struct $name {
            pub id: String,
            pub aid: String,
            /// Set by the engine when it stores the task; clients leave it out.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub v: Option<i64>,
            #[serde(rename = "__cls")]
            #[schemars(extend("const" = $cls))]
            pub cls: String,
            /// One of [`task_status`].
            #[schemars(extend("enum" = ["local", "remote", "complete", "cancelled"]))]
            pub status: String,
            /// Who queued it, for the log (`tern-cli`, `mail-rules`, ...).
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub source: Option<String>,
            /// The `SyncError` JSON of a failed task.
            #[serde(default)]
            #[schemars(with = "Option<crate::methods::SyncErrorJson>")]
            pub error: Option<Value>,
            /// `task.cancel` asked for it.
            #[serde(rename = "should_cancel", default, skip_serializing_if = "Option::is_none")]
            pub should_cancel: Option<bool>,
            $($fields)*
            #[serde(flatten)]
            #[schemars(skip)]
            pub extra: Extra,
        }
    };
}

/// The fields of the four mail-change tasks.
macro_rules! change_mail_task {
    ($(#[doc = $doc:literal])* $name:ident, $cls:literal { $($fields:tt)* }) => {
        task_json!(
            $(#[doc = $doc])*
            $name, $cls {
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub task_description: Option<String>,
                pub thread_ids: Vec<String>,
                pub message_ids: Vec<String>,
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub can_be_undone: Option<bool>,
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub is_undo: Option<bool>,
                /// Set when its remote phase failed for good and the engine put back the
                /// local change the server didn't take.
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub reverted: Option<bool>,
                $($fields)*
            }
        );
    };
}

change_mail_task!(ChangeUnreadTaskJson, "ChangeUnreadTask" {
    pub unread: bool,
    /// Set by the local phase: each message it changed, with its value before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_flags: Option<BTreeMap<String, bool>>,
});

change_mail_task!(ChangeStarredTaskJson, "ChangeStarredTask" {
    pub starred: bool,
    /// Set by the local phase: each message it changed, with its value before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_flags: Option<BTreeMap<String, bool>>,
});

change_mail_task!(ChangeFolderTaskJson, "ChangeFolderTask" {
    pub folder: FolderJson,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_folder_ids: Option<Vec<String>>,
    /// Set by the local phase: where each message was, for an undo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_placements: Option<Value>,
    /// An undo: the placements to put back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_placements: Option<Value>,
});

change_mail_task!(ChangeLabelsTaskJson, "ChangeLabelsTask" {
    pub labels_to_add: Vec<LabelJson>,
    pub labels_to_remove: Vec<LabelJson>,
    /// Set by the local phase: the X-GM-LABELS each message it changed gained and lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_labels: Option<BTreeMap<String, LabelChange>>,
});

/// One message's entry in a ChangeLabelsTask's `undoLabels`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LabelChange {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
}

task_json!(DestroyDraftTaskJson, "DestroyDraftTask" {
    pub message_ids: Vec<String>,
});

task_json!(SyncbackDraftTaskJson, "SyncbackDraftTask" {
    pub header_message_id: String,
    pub draft: MessageJson,
});

task_json!(SendDraftTaskJson, "SendDraftTask" {
    pub header_message_id: String,
    pub draft: MessageJson,
    /// Recipient email -> the body sent to them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_recipient_bodies: Option<BTreeMap<String, String>>,
    /// No notification when it's sent (a send-later or a mail-rule forward).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent: Option<bool>,
    /// When the send started waiting for the network (unix seconds).
    #[serde(rename = "_offlineSince", default, skip_serializing_if = "Option::is_none")]
    pub offline_since: Option<i64>,
});

task_json!(SyncbackMetadataTaskJson, "SyncbackMetadataTask" {
    pub plugin_id: String,
    pub model_id: String,
    /// `Thread` or `Message`.
    pub model_class_name: String,
    /// `null` for a model that isn't a Message.
    #[serde(default)]
    pub model_header_message_id: Option<String>,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_value: Option<Value>,
});

task_json!(ExpungeAllInFolderTaskJson, "ExpungeAllInFolderTask" {
    pub folder: FolderJson,
});

task_json!(SyncbackCategoryTaskJson, "SyncbackCategoryTask" {
    pub path: String,
    /// A rename: the folder or label's current path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_path: Option<String>,
    /// Set by the remote phase: the category created or renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Value>,
});

task_json!(DestroyCategoryTaskJson, "DestroyCategoryTask" {
    pub path: String,
});

task_json!(ChangeRoleMappingTaskJson, "ChangeRoleMappingTask" {
    pub path: String,
    #[schemars(with = "crate::models::CategoryRole")]
    pub role: String,
});

task_json!(GetMessageRFC2822TaskJson, "GetMessageRFC2822Task" {
    pub message_id: String,
    /// Local clients only. Without it (file-id mode) the engine saves the File `fileId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filepath: Option<String>,
    /// Set by the local phase in file-id mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
});

task_json!(GetManyRFC2822TaskJson, "GetManyRFC2822Task" {
    pub folder_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    /// `eml` or `mbox`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mbox_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
});

task_json!(EventRSVPTaskJson, "EventRSVPTask" {
    pub ics: String,
    #[serde(rename = "icsRSVPStatus")]
    pub ics_rsvp_status: String,
    pub to: String,
    pub subject: String,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
});

task_json!(SyncbackContactTaskJson, "SyncbackContactTask" {
    pub contact: ContactJson,
});

task_json!(DestroyContactTaskJson, "DestroyContactTask" {
    pub contacts: Vec<ContactJson>,
});

task_json!(SyncbackContactGroupTaskJson, "SyncbackContactGroupTask" {
    pub group: ContactGroupJson,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_name: Option<String>,
});

task_json!(DestroyContactGroupTaskJson, "DestroyContactGroupTask" {
    pub group: ContactGroupJson,
});

task_json!(ChangeContactGroupMembershipTaskJson, "ChangeContactGroupMembershipTask" {
    pub group: ContactGroupJson,
    pub contacts: Vec<ContactJson>,
    /// `add` or `remove`.
    pub direction: String,
});

task_json!(SyncbackEventTaskJson, "SyncbackEventTask" {
    pub event: EventJson,
    pub calendar_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsvp: Option<Value>,
    /// An undo or redo, re-sent over the copy stored now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
});

task_json!(DestroyEventTaskJson, "DestroyEventTask" {
    pub events: Vec<EventJson>,
});

/// Any task the engine runs, by `__cls`. Schema only: in Rust a task deserialises as the
/// struct its `__cls` names.
#[derive(JsonSchema)]
#[serde(untagged)]
// Schema only, never built, so the variants' sizes don't matter.
#[allow(dead_code, clippy::large_enum_variant)]
pub enum TaskJson {
    ChangeUnreadTask(ChangeUnreadTaskJson),
    ChangeStarredTask(ChangeStarredTaskJson),
    ChangeFolderTask(ChangeFolderTaskJson),
    ChangeLabelsTask(ChangeLabelsTaskJson),
    DestroyDraftTask(DestroyDraftTaskJson),
    SyncbackDraftTask(SyncbackDraftTaskJson),
    SendDraftTask(SendDraftTaskJson),
    SyncbackMetadataTask(SyncbackMetadataTaskJson),
    ExpungeAllInFolderTask(ExpungeAllInFolderTaskJson),
    SyncbackCategoryTask(SyncbackCategoryTaskJson),
    DestroyCategoryTask(DestroyCategoryTaskJson),
    ChangeRoleMappingTask(ChangeRoleMappingTaskJson),
    GetMessageRFC2822Task(GetMessageRFC2822TaskJson),
    GetManyRFC2822Task(GetManyRFC2822TaskJson),
    EventRSVPTask(EventRSVPTaskJson),
    SyncbackContactTask(SyncbackContactTaskJson),
    DestroyContactTask(DestroyContactTaskJson),
    SyncbackContactGroupTask(SyncbackContactGroupTaskJson),
    DestroyContactGroupTask(DestroyContactGroupTaskJson),
    ChangeContactGroupMembershipTask(ChangeContactGroupMembershipTaskJson),
    SyncbackEventTask(SyncbackEventTaskJson),
    DestroyEventTask(DestroyEventTaskJson),
}

/// Every task class above, by `__cls`.
pub const TASK_CLASSES: &[&str] = &[
    "ChangeUnreadTask",
    "ChangeStarredTask",
    "ChangeFolderTask",
    "ChangeLabelsTask",
    "DestroyDraftTask",
    "SyncbackDraftTask",
    "SendDraftTask",
    "SyncbackMetadataTask",
    "ExpungeAllInFolderTask",
    "SyncbackCategoryTask",
    "DestroyCategoryTask",
    "ChangeRoleMappingTask",
    "GetMessageRFC2822Task",
    "GetManyRFC2822Task",
    "EventRSVPTask",
    "SyncbackContactTask",
    "DestroyContactTask",
    "SyncbackContactGroupTask",
    "DestroyContactGroupTask",
    "ChangeContactGroupMembershipTask",
    "SyncbackEventTask",
    "DestroyEventTask",
];
