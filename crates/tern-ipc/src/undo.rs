// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine's undo log ([`feature::UNDO`](crate::feature::UNDO)). Each undoable intent,
//! snooze, send with an undo window, or recorded task is one entry, kept in memory (at most
//! [`UNDO_LOG_MAX_ENTRIES`], for [`UNDO_LOG_MAX_SECS`]). An entry belongs to the client that
//! made the change: the `hello.client` name up to its `/` (`tern-desktop`, `tern-cli`), with
//! the device id for a relayed connection. Without a token `undo.apply` and `undo.redo` act on
//! the caller's own latest entry, so Cmd-Z in the app doesn't undo what an agent did.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most entries the log keeps; the oldest goes first.
pub const UNDO_LOG_MAX_ENTRIES: usize = 100;

/// How long an entry stays undoable, in seconds.
pub const UNDO_LOG_MAX_SECS: i64 = 24 * 60 * 60;

/// An entry's `state`.
pub mod undo_state {
    named_consts! { &str;
        /// Its change is in effect: `undo.apply` takes it.
        DONE = "done";
        /// Undone: `undo.redo` takes it, until its client makes another change.
        UNDONE = "undone";
        /// Out of the log: undone for good, sent, expired, or its change failed on the server
        /// and the engine put back what the server didn't take (`reason`). Only
        /// `undo.changed` carries it.
        GONE = "gone";
    }
}

/// What an entry's change was, for the client's own wording of `label`.
pub mod undo_action {
    named_consts! { &str;
        ARCHIVE = "archive";
        TRASH = "trash";
        SPAM = "spam";
        NOT_SPAM = "notSpam";
        MOVE = "move";
        LABEL = "label";
        MARK_READ = "markRead";
        MARK_UNREAD = "markUnread";
        STAR = "star";
        UNSTAR = "unstar";
        REMOVE_FROM_VIEW = "removeFromView";
        /// `categories.markRead`.
        CATEGORY_MARK_READ = "categoryMarkRead";
        SNOOZE = "snooze";
        /// A send waiting out its undo window: `undo.apply` is `draft.cancelSend`.
        SEND = "send";
        /// Tasks a client queued itself and handed to `undo.record`.
        TASKS = "tasks";
    }
}

/// One entry of the log.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoEntry {
    pub token: String,
    /// Whose change it is (see the module doc).
    pub client: String,
    /// One of [`undo_action`].
    pub action: String,
    /// What the change was, in English (`Archived 3 threads`); clients that localize word it
    /// from `action` and `count`.
    pub label: String,
    /// Threads, messages or categories it acted on.
    pub count: usize,
    /// Unix seconds, when it was made.
    pub at: i64,
    pub account_ids: Vec<String>,
    /// The tasks now in effect: the change's, or after an undo the undo's.
    pub task_ids: Vec<String>,
    /// One of [`undo_state`].
    #[schemars(extend("enum" = ["done", "undone", "gone"]))]
    pub state: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub can_undo: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub can_redo: bool,
    /// Why it is gone: `undone`, `sent`, `expired`, `replaced` (out of the log's bound),
    /// `failed` (its change failed on the server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `undo.apply` and `undo.redo`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoApplyParams {
    /// The entry; without it, the caller's latest entry that can be undone (or redone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Without a token: the latest entry of any client (`tern undo --any`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub any: bool,
    /// The IANA zone an event restore reads floating times in (the client's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// Send `task.done` for each task it queues.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// What `undo.apply` and `undo.redo` did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoApplyResult {
    /// The entry as it is now.
    pub entry: UndoEntry,
    /// The tasks queued; their local phases have run.
    pub task_ids: Vec<String>,
    /// The Thread rows they changed, as after the local phases (as an intent's `models`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<crate::models::MetadataModelJson>")]
    pub models: Vec<serde_json::Value>,
    /// The undo of a send: whether it was still waiting (false: it had already gone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<bool>,
}

/// `undo.list`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoListParams {
    /// At most this many, newest first; all of them without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Only the caller's own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mine: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoListResult {
    /// Newest first.
    pub entries: Vec<UndoEntry>,
}

/// One task of `undo.record`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskRef {
    pub account_id: String,
    pub task_id: String,
}

/// `undo.record`: tasks the caller queued with `task.queue`, as one entry. Each must have
/// run its local phase without error and be of a class the engine can undo
/// (ChangeUnread/Starred/Folder/Labels, SyncbackMetadata with `undoValue`, SyncbackEvent with
/// `undoData`, the contact group tasks).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoRecordParams {
    pub tasks: Vec<TaskRef>,
    /// The entry's `label`; the tasks' `taskDescription` without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoRecordResult {
    /// Null when the tasks changed nothing there is to undo.
    #[serde(default)]
    #[schemars(extend("x-null" = true))]
    pub token: Option<String>,
}

/// `undo.changed`: an entry was added, undone, redone or left the log. To every subscribed
/// connection that takes one of its accounts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoChangedParams {
    pub entry: UndoEntry,
}
