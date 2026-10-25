// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Params and results of the phase B read methods. Models are the stored
//! JSON the desktop parses (`Thread`, `Message`, `Folder`, `Label`, `Contact`), passed
//! through as [`Value`]s. A `limit` defaults to [`READ_DEFAULT_LIMIT`](crate::READ_DEFAULT_LIMIT)
//! and is clamped to `1..=`[`READ_MAX_LIMIT`](crate::READ_MAX_LIMIT).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `folders.list` and `labels.list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoriesListParams {
    pub account_id: String,
}

/// A category's `ThreadCounts` row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CategoryCounts {
    pub unread: i64,
    pub total: i64,
}

/// `folders.list`: the account's folders by path. `counts` is keyed by folder id and
/// leaves out folders without a `ThreadCounts` row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FoldersListResult {
    #[schemars(with = "Vec<crate::models::FolderJson>")]
    pub folders: Vec<Value>,
    pub counts: BTreeMap<String, CategoryCounts>,
}

/// `labels.list`, shaped like [`FoldersListResult`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LabelsListResult {
    #[schemars(with = "Vec<crate::models::LabelJson>")]
    pub labels: Vec<Value>,
    pub counts: BTreeMap<String, CategoryCounts>,
}

/// `threads.list`. At most one of `folderId`, `labelId` and `role` (a folder's role, or
/// else a label's; not empty); none lists every thread of the account. A category other
/// than spam or trash lists only threads in all mail.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThreadsListResult {
    #[schemars(with = "Vec<crate::models::ThreadJson>")]
    pub threads: Vec<Value>,
    pub next: Option<String>,
}

/// `threads.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsGetParams {
    pub account_id: String,
    pub id: String,
}

/// The thread and its messages, oldest first, without bodies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThreadsGetResult {
    #[schemars(with = "crate::models::ThreadJson")]
    pub thread: Value,
    #[schemars(with = "Vec<crate::models::MessageJson>")]
    pub messages: Vec<Value>,
}

/// `messages.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessagesGetResult {
    #[schemars(with = "crate::models::MessageJson")]
    pub message: Value,
    pub body: Option<String>,
    #[schemars(with = "Vec<crate::models::FileJson>")]
    pub files: Vec<Value>,
}

/// `search` (over `ThreadSearch`). Without `grammar`, `query` is plain text: each
/// whitespace-separated term must match, and nothing in it is read as FTS syntax. With
/// `grammar: true` (feature `search-grammar`) it is the search grammar, its date terms read
/// in `tz` (an IANA name, UTC when absent) and counted back from `now` (unix seconds; needed
/// only by relative dates); text that doesn't parse, or a date outside the documented
/// subset, is `INVALID_PARAMS`. The threads are `accountId`'s, or `accounts`' (feature
/// `search-grammar`), or, with neither, every account's the engine holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts: Option<Vec<String>>,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub grammar: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<i64>,
}

/// `contacts.search` (over `ContactSearch`): plain text, each term matching a word prefix
/// of the contact's email or name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContactsSearchParams {
    pub account_id: String,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// `search`: matching threads, newest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SearchResult {
    #[schemars(with = "Vec<crate::models::ThreadJson>")]
    pub threads: Vec<Value>,
}

/// `contacts.search`: matching contacts that aren't hidden, most referenced first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContactsSearchResult {
    #[schemars(with = "Vec<crate::models::ContactJson>")]
    pub contacts: Vec<Value>,
}

/// `tasks.get`: a task of the account by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasksGetParams {
    pub account_id: String,
    pub id: String,
}

/// The stored `Task` JSON. Finished tasks are kept until the newest hundred of the account
/// push them out (`cleanupOldTasksAtRuntime`), so a missing task may have finished long ago.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TasksGetResult {
    #[schemars(with = "crate::tasks::TaskJson")]
    pub task: Value,
}

/// `counts.get`. `accounts` null (or absent) means every account the engine holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CountsGetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts: Option<Vec<String>>,
}

/// Every `ThreadCounts` row of the accounts' folders and labels, keyed by category id. The
/// same rows arrive as `ThreadCounts` deltas (`{id, categoryId, unread, total}`, `persist`
/// when a count changes or a category is added, `unpersist` when it is removed), so a client
/// reads this once per connection and applies the deltas after it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CountsGetResult {
    pub counts: BTreeMap<String, CategoryCounts>,
}

/// `tasks.list`: the stored tasks, the pending ones (`local`, `remote`) first, oldest queued
/// first, then the finished ones, newest first. Every filter is optional: no
/// `accountId` lists every account the engine holds, no `status` every status (`local`,
/// `remote`, `complete`, `cancelled`). `headerMessageId` keeps the tasks naming that draft,
/// in their `headerMessageId` or their `draft`'s.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasksListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<String>>,
    /// The task's `__cls`, e.g. `SendDraftTask`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TasksListResult {
    #[schemars(with = "Vec<crate::tasks::TaskJson>")]
    pub tasks: Vec<Value>,
}

/// `metadata.list`: the models carrying `pluginId`'s metadata (a non-empty value), newest
/// first by `date` (Message) or `lastMessageTimestamp` (Thread), then id. `objectType` is
/// `Message` (the default) or `Thread`; `draft` only applies to messages. `expiresBefore`
/// keeps rows whose expiration is set and earlier; `hasExpiration` rows with (true) or
/// without (false) one. `after` and `before` bound the model's date, exclusive, in epoch
/// seconds. `offset` skips rows, for paging past [`READ_MAX_LIMIT`](crate::READ_MAX_LIMIT).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetadataListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts: Option<Vec<String>>,
    pub plugin_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_before: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_expiration: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetadataListResult {
    #[schemars(with = "Vec<crate::models::MetadataModelJson>")]
    pub models: Vec<Value>,
}

/// `query`: the JSON form of the desktop's `ModelQuery`
/// (`Query.toJSON()`). `where` is a matcher (`{attr, op, value}`, `{and|or|not: [..]}`) or a
/// list of them, all of which must match. `accounts: None` reads every account. An empty
/// `order` is the class's natural order. `cursor` pages by keyset: `""` for the first page,
/// then the last page's `nextCursor`. No limit is applied when `limit` is absent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryParams {
    #[schemars(with = "QueryClassName")]
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts: Option<Vec<String>>,
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<QueryWhere>")]
    pub where_: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<QuerySearch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<QueryOrder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub distinct: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ids_only: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub count: bool,
}

/// `query.search`. `grammar: false` is the plain FTS prefix match of `.search(text)`;
/// `grammar: true` (feature `search-grammar`, Thread only) is the search grammar, with date
/// terms read in `tz` (IANA, UTC when absent) and counted back from `now` (unix seconds).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuerySearch {
    pub text: String,
    #[serde(default)]
    pub grammar: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueryOrder {
    pub attr: String,
    /// `"asc"` or `"desc"`.
    #[schemars(extend("enum" = ["asc", "desc"]))]
    pub dir: String,
}

/// The classes `query` reads ([`crate::query_attrs::QUERY_CLASSES`]). Schema only.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub enum QueryClassName {
    Thread,
    Message,
    Folder,
    Label,
    Contact,
    ContactGroup,
    ContactBook,
    Event,
    Calendar,
    Task,
    File,
}

/// `query.where`: one matcher, or a list that must all match. Schema only.
#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum QueryWhere {
    One(MatcherJson),
    All(Vec<MatcherJson>),
}

/// A matcher as `Matcher.toJSON()` writes it: `attr` is the desktop's model key
/// (`lastMessageReceivedTimestamp`), not the JSON key. Schema only.
#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum MatcherJson {
    Attr {
        attr: String,
        op: QueryOp,
        value: Value,
    },
    And {
        and: Vec<MatcherJson>,
    },
    Or {
        or: Vec<MatcherJson>,
    },
    Not {
        not: Vec<MatcherJson>,
    },
}

/// The operators of a matcher (desktop `src/data/attributes/matcher.ts`). Schema only.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub enum QueryOp {
    #[serde(rename = "=")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "in")]
    In,
    #[serde(rename = "not in")]
    NotIn,
    #[serde(rename = "contains")]
    Contains,
    #[serde(rename = "containsAny")]
    ContainsAny,
    #[serde(rename = "startsWith")]
    StartsWith,
    #[serde(rename = "like")]
    Like,
}

/// `query`'s result: `rows` always (empty for `idsOnly` and `count`), `ids` for `idsOnly`,
/// `count` for `count`, and `nextCursor` for a cursor query (`null` after the last page).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    /// The class's model JSON (with `include`'s joined data).
    #[schemars(with = "Vec<crate::models::AnyModelJson>")]
    pub rows: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    /// `null` after the last page.
    #[schemars(with = "Option<String>", extend("x-null" = true))]
    pub next_cursor: Option<Option<String>>,
}

/// `Some(None)` as `null`, `None` as absent.
mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Option<String>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Option<String>>, D::Error> {
        Option::<String>::deserialize(d).map(Some)
    }
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

    #[test]
    fn stage_one_params_keys() {
        let p: TasksListParams = serde_json::from_value(
            json!({"accountId": "a", "status": ["local"], "class": "SendDraftTask",
                   "headerMessageId": "h@x"}),
        )
        .unwrap();
        assert_eq!(p.class.as_deref(), Some("SendDraftTask"));
        assert_eq!(p.header_message_id.as_deref(), Some("h@x"));
        let p: MetadataListParams = serde_json::from_value(
            json!({"pluginId": "send-later", "objectType": "Message", "hasExpiration": true,
                   "expiresBefore": 5}),
        )
        .unwrap();
        assert_eq!(
            (p.has_expiration, p.expires_before, p.accounts),
            (Some(true), Some(5), None)
        );
        assert!(serde_json::from_value::<MetadataListParams>(json!({})).is_err());
        let p: CountsGetParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(p.accounts, None);
    }

    #[test]
    fn query_shape() {
        let p: QueryParams = serde_json::from_value(json!({
            "class": "Thread", "accounts": null,
            "where": {"and": [{"attr": "inAllMail", "op": "=", "value": true}]},
            "search": {"text": "ann", "grammar": false},
            "order": [{"attr": "lastMessageReceivedTimestamp", "dir": "desc"}],
            "limit": 100, "offset": null, "cursor": "", "distinct": true, "include": [],
            "idsOnly": false, "count": false,
        }))
        .unwrap();
        assert_eq!(
            (p.accounts.as_ref(), p.limit, p.distinct),
            (None, Some(100), true)
        );
        assert_eq!(p.cursor.as_deref(), Some(""));
        let back = serde_json::to_value(&p).unwrap();
        assert_eq!(back["order"][0]["dir"], "desc");
        assert!(back.get("idsOnly").is_none() && back.get("accounts").is_none());

        let r = |v: Value| serde_json::from_value::<QueryResult>(v).unwrap();
        assert_eq!(r(json!({"rows": []})).next_cursor, None);
        assert_eq!(
            r(json!({"rows": [], "nextCursor": null})).next_cursor,
            Some(None)
        );
        let end = QueryResult {
            next_cursor: Some(None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(end).unwrap(),
            json!({"rows": [], "nextCursor": null})
        );
        let ids = QueryResult {
            ids: Some(vec!["t1".into()]),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(ids).unwrap(),
            json!({"rows": [], "ids": ["t1"]})
        );
    }
}
