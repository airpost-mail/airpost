// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Params and results of the phase B read methods (plan 1.2 and 3). Models are the stored
//! JSON the desktop parses (`Thread`, `Message`, `Folder`, `Label`, `Contact`), passed
//! through as [`Value`]s. A `limit` defaults to [`READ_DEFAULT_LIMIT`](crate::READ_DEFAULT_LIMIT)
//! and is clamped to `1..=`[`READ_MAX_LIMIT`](crate::READ_MAX_LIMIT).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `folders.list` and `labels.list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoriesListParams {
    pub account_id: String,
}

/// A category's `ThreadCounts` row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCounts {
    pub unread: i64,
    pub total: i64,
}

/// `folders.list`: the account's folders by path. `counts` is keyed by folder id and
/// leaves out folders without a `ThreadCounts` row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FoldersListResult {
    pub folders: Vec<Value>,
    pub counts: BTreeMap<String, CategoryCounts>,
}

/// `labels.list`, shaped like [`FoldersListResult`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabelsListResult {
    pub labels: Vec<Value>,
    pub counts: BTreeMap<String, CategoryCounts>,
}

/// `threads.list`. At most one of `folderId`, `labelId` and `role` (a folder's role, or
/// else a label's; not empty); none lists every thread of the account. A category other
/// than spam or trash lists only threads in all mail.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsListParams {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// The `next` of the previous page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

/// Newest first by `(lastMessageTimestamp, id)`. `next` is an opaque cursor for `before`,
/// null on the last page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadsListResult {
    pub threads: Vec<Value>,
    pub next: Option<String>,
}

/// `threads.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsGetParams {
    pub account_id: String,
    pub id: String,
}

/// The thread and its messages, oldest first, without bodies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadsGetResult {
    pub thread: Value,
    pub messages: Vec<Value>,
}

/// `messages.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesGetParams {
    pub account_id: String,
    pub id: String,
    /// Include the body from `MessageBody`.
    #[serde(default)]
    pub body: bool,
}

/// `body` is null unless asked for and fetched (`bodies.fetch` fetches it). `files` is the
/// message's `File` JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessagesGetResult {
    pub message: Value,
    pub body: Option<String>,
    pub files: Vec<Value>,
}

/// `search` (over `ThreadSearch`) and `contacts.search` (over `ContactSearch`). The query
/// is plain text: each whitespace-separated term must match, and nothing in it is read as
/// FTS syntax. Contact terms match word prefixes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    pub account_id: String,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// `search`: matching threads, newest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub threads: Vec<Value>,
}

/// `contacts.search`: matching contacts that aren't hidden, most referenced first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContactsSearchResult {
    pub contacts: Vec<Value>,
}

/// `tasks.get`: a task of the account by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TasksGetParams {
    pub account_id: String,
    pub id: String,
}

/// The stored `Task` JSON. Finished tasks are kept until the newest hundred of the account
/// push them out (`cleanupOldTasksAtRuntime`), so a missing task may have finished long ago.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TasksGetResult {
    pub task: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_params_keys() {
        let p: ThreadsListParams =
            serde_json::from_value(json!({"accountId": "a", "labelId": "l", "limit": 5})).unwrap();
        assert_eq!(p.label_id.as_deref(), Some("l"));
        assert_eq!((p.folder_id, p.limit, p.before), (None, Some(5), None));
        let p: MessagesGetParams =
            serde_json::from_value(json!({"accountId": "a", "id": "m"})).unwrap();
        assert!(!p.body);
        let r = ThreadsListResult {
            threads: vec![],
            next: None,
        };
        assert_eq!(
            serde_json::to_value(r).unwrap(),
            json!({"threads": [], "next": null})
        );
        let r = MessagesGetResult {
            message: json!({}),
            body: None,
            files: vec![],
        };
        assert_eq!(serde_json::to_value(r).unwrap()["body"], Value::Null);
    }
}
