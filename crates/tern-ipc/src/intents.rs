// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Mail actions as verbs ([`feature::INTENTS`](crate::feature::INTENTS)): `threads.*` and
//! `categories.*`. The engine resolves what each one means on each account (Gmail labels or
//! folders, the archive and all-mail fallbacks, the copies a move takes from the list the
//! user is looking at), queues the tasks and answers after their local phases with the rows
//! as they now are and, for the undoable ones, an undo token ([`crate::UndoEntry`]).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The list the user acted from: the categories it shows (of any accounts), and whether it
/// is the starred or unread view. A move takes only the copies in the perspective's folder
/// of each account; `threads.removeFromView` decides from it what "remove" means.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Perspective {
    /// Folder or label ids. Empty for a view without categories (search, starred).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub category_ids: Vec<String>,
    /// The starred view.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub starred: bool,
    /// The unread view of `categoryIds`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unread: bool,
}

/// What every `threads.*` intent acts on: threads, or messages, of any accounts the engine
/// holds. Exactly one of the two lists is non-empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailTargets {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thread_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_ids: Vec<String>,
}

/// `threads.archive`, `threads.trash`, `threads.spam`, `threads.notSpam`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsActionParams {
    #[serde(flatten)]
    pub targets: MailTargets,
    /// The list it was done from; without it a move takes every copy outside Sent and Drafts
    /// (and Trash and Spam take every copy whatever it says).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<Perspective>,
    /// Send `task.done` on this connection for each task it queues
    /// ([`feature::TASK_DONE`](crate::feature::TASK_DONE)).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// `threads.move`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsMoveParams {
    #[serde(flatten)]
    pub targets: MailTargets,
    /// On each account: a folder or label id, else a role (`inbox`, `archive`, ...), else a
    /// path or display name, without regard to case. A folder takes the copies the
    /// perspective shows (or every copy outside Sent and Drafts); a Gmail label replaces the
    /// threads' labels. An account where it names nothing fails the call before anything is
    /// queued.
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<Perspective>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// `threads.label`: Gmail labels added and removed, each named as `threads.move`'s `to`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsLabelParams {
    #[serde(flatten)]
    pub targets: MailTargets,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// `threads.markRead`: read, or with `unread` unread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsMarkReadParams {
    #[serde(flatten)]
    pub targets: MailTargets,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unread: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// `threads.star`: starred, or with `starred: false` unstarred.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsStarParams {
    #[serde(flatten)]
    pub targets: MailTargets,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub starred: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// `threads.removeFromView`: what Delete, Backspace or a swipe does in the list the user is
/// looking at (`MailboxPerspective.tasksForRemovingItems`). From Archive or All Mail: to the
/// trash. From Trash or Spam: nothing. From another folder: to the archive (the trash with
/// `preferTrash` or without an archive), only the copies in that folder. From a Gmail label:
/// the label and the inbox label removed, or to the trash. The unread view also marks them
/// read; the starred view unstars them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsRemoveFromViewParams {
    #[serde(flatten)]
    pub targets: MailTargets,
    pub perspective: Perspective,
    /// The client's "Delete rather than archive" preference.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prefer_trash: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// What a `threads.*` intent, `categories.*` or an `undo.apply`/`undo.redo` did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntentResult {
    /// The tasks queued, in order; their local phases have run.
    pub task_ids: Vec<String>,
    /// The token `undo.apply` takes; null when there is nothing to undo.
    #[serde(default)]
    #[schemars(extend("x-null" = true))]
    pub undo: Option<String>,
    /// The Thread rows the call changed, as they are after the local phases (and for
    /// `messageIds`, the Message rows first), so a client over the gateway can show them
    /// before its live queries catch up.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<crate::models::MetadataModelJson>")]
    pub models: Vec<Value>,
    /// The thread or message ids asked for that the call didn't change (already read,
    /// already in the archive, ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchanged: Vec<String>,
}

/// `categories.markRead`: every unread thread of the folder or label, read (in all mail,
/// unless it is Trash or Spam).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryIdParams {
    pub category_id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// `categories.create`: a folder, or a label on Gmail. `name` has `/` between levels; the
/// engine writes the server's delimiter and namespace. The finished task's `created` is the
/// new Folder or Label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoriesCreateParams {
    pub account_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}

/// `categories.rename`: the whole new name, as for `categories.create`. Not for a folder or
/// label with a role.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoriesRenameParams {
    pub category_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
}
