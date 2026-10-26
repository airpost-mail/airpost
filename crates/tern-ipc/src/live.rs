// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Live reads and the event plumbing around them: `query.subscribe` and its
//! `query.changed` feed, `events.subscribe` and `events.changed`, `task.done`, and the
//! sequence numbers on `delta` that let a raw-feed client resume (`subscribe {since}`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which rows of a live query the client holds: `limit` rows from `offset` in the query's
/// order. No `limit` holds every row from `offset`; a relayed connection's window stops at
/// [`READ_MAX_LIMIT`](crate::READ_MAX_LIMIT) rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueryWindow {
    #[serde(default)]
    pub offset: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// `query.subscribe`. `query` is a `query` request without `limit`, `offset`, `cursor`,
/// `idsOnly` or `count`: the window decides which rows come back. `window` absent holds
/// every row. `key` names the subscription on this connection: a later `query.subscribe`
/// with the same key ends the earlier one, as `query.unsubscribe` would.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QuerySubscribeParams {
    pub query: crate::QueryParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<QueryWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// `query.subscribe`'s result: the window's rows, how many rows the whole query has, and
/// the version the next `query.changed` builds on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuerySubscribeResult {
    pub sub_id: String,
    #[schemars(with = "Vec<crate::models::AnyModelJson>")]
    pub rows: Vec<Value>,
    pub total: i64,
    pub version: u64,
}

/// `query.setWindow`: moves the subscription's window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuerySetWindowParams {
    pub sub_id: String,
    pub window: QueryWindow,
}

/// `query.setWindow`'s result: the new window's rows, replacing the ones the client holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QueryWindowResult {
    #[schemars(with = "Vec<crate::models::AnyModelJson>")]
    pub rows: Vec<Value>,
    pub total: i64,
    pub version: u64,
}

/// `query.unsubscribe` and `events.unsubscribe`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubIdParams {
    pub sub_id: String,
}

/// One step of a `query.changed`, applied in order to the rows the client holds: `remove`
/// drops the row with `id`; `insert` puts `row` at `index`; `move` takes the row with `id`
/// out and puts it back at `index`; `update` replaces the row with `row`'s id in place. An
/// index counts the rows as they are after the steps before it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum LiveOp {
    Remove {
        id: String,
    },
    Insert {
        index: usize,
        #[schemars(with = "crate::models::AnyModelJson")]
        row: Value,
    },
    Move {
        id: String,
        index: usize,
    },
    Update {
        #[schemars(with = "crate::models::AnyModelJson")]
        row: Value,
    },
}

/// `query.changed`: what changed in a subscription's window, at most once per 50 ms. Either
/// `ops`, or `reset` with the whole window when the steps would outnumber its rows.
/// `version` is one more than the last one the client saw for the subscription.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryChangedParams {
    pub sub_id: String,
    pub version: u64,
    pub total: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops: Option<Vec<LiveOp>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Vec<crate::models::AnyModelJson>>")]
    pub reset: Option<Vec<Value>>,
}

/// `events.subscribe`: the occurrences of the stored events in `[start, end)`, as
/// `events.expand` gives them, kept current. `key` as in `query.subscribe`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventsSubscribeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_ids: Option<Vec<String>>,
    pub start: i64,
    pub end: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// `events.subscribe`'s result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventsSubscribeResult {
    pub sub_id: String,
    pub occurrences: Vec<Value>,
    /// As in [`EventsExpandResult::truncated`](crate::EventsExpandResult::truncated).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// `events.changed`: the subscription's occurrences, whole, after an event or calendar of
/// its accounts changed them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventsChangedParams {
    pub sub_id: String,
    pub occurrences: Vec<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// `task.done`: a task queued with `task.queue {notify: true}` on this connection finished.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskDoneParams {
    pub account_id: String,
    pub task_id: String,
    /// `complete` or `cancelled`.
    #[schemars(extend("enum" = ["complete", "cancelled"]))]
    pub status: String,
    /// The `SyncError` JSON of a failed task; null when it succeeded.
    #[schemars(with = "Option<crate::methods::SyncErrorJson>", extend("x-null" = true))]
    pub error: Option<Value>,
    /// The stored task as it finished.
    #[schemars(with = "crate::tasks::TaskJson")]
    pub task: Value,
}

/// Where a raw-feed client stopped: the `feed` of the engine it was connected to and the
/// last `delta.seq` it applied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeltaCursor {
    pub feed: String,
    pub seq: u64,
}

/// `subscribe`'s result. `feed` names this engine process's delta sequence; `seq` is the
/// last sequence number it gave out before the call. With `since`, `resumed` says whether
/// every delta after it that the filter takes follows the response (seq at most `seq`,
/// then live ones); false means the engine no longer holds them, or `since` is another
/// engine's, and the client reads again what it shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResult {
    pub feed: String,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ops_are_tagged() {
        let ops = vec![
            LiveOp::Remove { id: "a".into() },
            LiveOp::Insert {
                index: 0,
                row: json!({"id": "b"}),
            },
            LiveOp::Move {
                id: "c".into(),
                index: 2,
            },
            LiveOp::Update {
                row: json!({"id": "c"}),
            },
        ];
        assert_eq!(
            serde_json::to_value(&ops).unwrap(),
            json!([{"op": "remove", "id": "a"}, {"op": "insert", "index": 0, "row": {"id": "b"}},
                   {"op": "move", "id": "c", "index": 2}, {"op": "update", "row": {"id": "c"}}])
        );
        let p: QuerySubscribeParams = serde_json::from_value(
            json!({"query": {"class": "Thread"}, "window": {"offset": 0, "limit": 50}}),
        )
        .unwrap();
        assert_eq!(p.window.unwrap().limit, Some(50));
        let done = TaskDoneParams {
            account_id: "a".into(),
            task_id: "t".into(),
            status: "complete".into(),
            error: None,
            task: json!({}),
        };
        assert_eq!(serde_json::to_value(&done).unwrap()["error"], Value::Null);
    }
}
