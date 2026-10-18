// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Params and results of the phase A methods (plan 1.2) and the notifications. Keys are
//! camelCase, as the desktop's stdin commands and delta objects already are.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    pub protocol: u32,
    /// Free-form, e.g. `tern-cli/0.1.0`; logged by the engine.
    #[serde(default)]
    pub client: String,
    /// Set by `tern-gateway` for a connection it relays for a remote device. The engine
    /// then applies [`policy`](crate::policy) to it. The first `hello` fixes this, `scopes`
    /// and `device` for the connection.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relayed: bool,
    /// The relayed device's scopes, e.g. [`policy::SCOPE_ADMIN`](crate::policy::SCOPE_ADMIN).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// The relayed device, for the engine's log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<RelayDevice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol: u32,
    /// The engine's version.
    pub engine: String,
    pub pid: u32,
    pub accounts: Vec<AccountInfo>,
    /// What this engine offers beyond protocol v1's base methods, from
    /// [`feature`](crate::feature). Clients gate behaviour on these, never on `engine`. Empty
    /// from an engine that predates them.
    #[serde(default)]
    pub features: Vec<String>,
    /// What the engine can do for the client now ([`feature::CAPABILITIES`](crate::feature::CAPABILITIES));
    /// `capabilities.changed` carries later values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<crate::Capabilities>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub account_id: String,
    pub email: String,
    /// One of [`account_state`](crate::account_state).
    pub state: String,
    /// The `SyncError` JSON that stopped the account, when `state` is `stopped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// `null` (or absent) means every account, or every model class.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeParams {
    #[serde(default)]
    pub accounts: Option<Vec<String>>,
    #[serde(default)]
    pub classes: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskQueueParams {
    pub account_id: String,
    /// The task JSON the desktop sends in `queue-task`.
    pub task: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCancelParams {
    pub account_id: String,
    pub task_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncWakeParams {
    /// `None` wakes every account.
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BodiesFetchParams {
    pub account_id: String,
    /// Message ids.
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSyncParams {
    pub account_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSyncResult {
    /// False when a pass started by an earlier call is still running.
    pub started: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatusResult {
    pub protocol: u32,
    pub engine: String,
    pub pid: u32,
    /// Seconds since the engine started.
    pub uptime_secs: u64,
    pub accounts: Vec<AccountStatus>,
    /// As in [`HelloResult::features`].
    #[serde(default)]
    pub features: Vec<String>,
    /// As in [`HelloResult::capabilities`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<crate::Capabilities>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub account_id: String,
    pub email: String,
    pub state: String,
    /// The last `ProcessState` the account sent: true while it cannot reach its server.
    pub connection_error: bool,
    /// The `SyncError` JSON that stopped the account, when `state` is `stopped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    /// The account's settings without credentials, from
    /// [`public_account_json`](crate::policy::public_account_json). `account.list` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Value>,
}

/// `account.list` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountListResult {
    pub accounts: Vec<AccountStatus>,
}

/// `account.add` params. The account JSON carries credentials, which is why the method is
/// only accepted on the parent's stdin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountAddParams {
    /// The account JSON `sync` mode reads from `--account` or stdin.
    pub account: Value,
    /// The identity line `sync` mode reads: a Tern server object, a Mailspring identity
    /// (ignored) or null.
    #[serde(default)]
    pub identity: Value,
}

/// `account.add` result: the account's state when the call returns, `running` once its
/// `main` thread is up, `stopped` (with `error`) when it failed to start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountAddResult {
    pub account_id: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// `account.remove` result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRemoveResult {
    /// The names of the account's threads still running after the stop timeout, left to
    /// finish a blocking call (e.g. `background`). Empty when every thread exited.
    #[serde(default)]
    pub still_running: Vec<String>,
}

/// `account.remove`, `test.crash` and `test.panic` params.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountIdParams {
    pub account_id: String,
}

/// `account.reset` params. `keepMetadata` (default true) keeps the account's plugin metadata
/// for the resync; a client removing the account passes false so it goes with the rest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountResetParams {
    pub account_id: String,
    #[serde(default = "keep_metadata_default")]
    pub keep_metadata: bool,
}

/// `engine.resetDatabase` params, all optional. `keepMetadata` (default true) carries the
/// plugin metadata into the new database; false when the last account was removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetDatabaseParams {
    #[serde(default = "keep_metadata_default")]
    pub keep_metadata: bool,
}

impl Default for ResetDatabaseParams {
    fn default() -> Self {
        ResetDatabaseParams {
            keep_metadata: true,
        }
    }
}

fn keep_metadata_default() -> bool {
    true
}

/// `account.test` params. The account JSON carries credentials, which is why the method is
/// only accepted on the parent's stdin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountTestParams {
    pub account: Value,
}

/// `engine.resetDatabase` result: the accounts it stopped and forgot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetDatabaseResult {
    pub removed: Vec<String>,
}

/// `engine.setLogLevel` params.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLogLevelParams {
    /// Unix milliseconds; verbose logging is on for accounts started before then. 0 or a
    /// past time turns it off.
    pub verbose_until: i64,
}

/// `engine.setLogLevel` result: the accounts restarted to apply it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLogLevelResult {
    pub restarted: Vec<String>,
}

/// The most `engine.logs.tail` returns, and its default.
pub const LOGS_TAIL_MAX_BYTES: u64 = 1024 * 1024;
pub const LOGS_TAIL_DEFAULT_BYTES: u64 = 64 * 1024;

/// `engine.logs.tail` params. Without `accountId`, the engine's own log (`engine.log`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsTailParams {
    #[serde(default)]
    pub account_id: Option<String>,
    /// At most [`LOGS_TAIL_MAX_BYTES`]; [`LOGS_TAIL_DEFAULT_BYTES`] when absent.
    #[serde(default)]
    pub bytes: Option<u64>,
}

/// `engine.logs.tail` result. `text` is the last `bytes` of the current log file, decoded
/// lossily; empty when the file does not exist.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsTailResult {
    pub text: String,
    /// The file is longer than `text`.
    pub truncated: bool,
}

/// `account.state` notification params.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStateParams {
    pub account_id: String,
    /// One of [`account_state`](crate::account_state).
    pub state: String,
    /// The `SyncError` JSON, with `stopped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// The result of methods that return nothing: `{}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Empty {}

/// `delta` params: today's stdout delta object plus the account it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaParams {
    pub account_id: String,
    pub model_class: String,
    /// `persist`, `unpersist` or `metadata-expiration`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "modelJSONs")]
    pub model_jsons: Vec<Value>,
}

/// `disconnect` params.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectParams {
    pub reason: String,
}

/// `mail.new` params: a message that arrived unread in the inbox after the folder's first
/// headers sync. Sent once per message to the connections subscribed to its account, never
/// held for a later subscriber; a client that sees a `messageId` twice drops the repeat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailNewParams {
    pub account_id: String,
    pub thread_id: String,
    pub message_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn features_default_to_none() {
        let h: HelloResult = serde_json::from_value(
            json!({"protocol": 1, "engine": "0.1.0", "pid": 1, "accounts": []}),
        )
        .unwrap();
        assert!(h.features.is_empty());
        let n = MailNewParams {
            account_id: "a".into(),
            thread_id: "t".into(),
            message_id: "m".into(),
        };
        assert_eq!(
            serde_json::to_value(&n).unwrap(),
            json!({"accountId": "a", "threadId": "t", "messageId": "m"})
        );
    }

    #[test]
    fn resets_keep_metadata_unless_asked_not_to() {
        let p: AccountResetParams = serde_json::from_value(json!({"accountId": "a"})).unwrap();
        assert!(p.keep_metadata);
        let p: AccountResetParams =
            serde_json::from_value(json!({"accountId": "a", "keepMetadata": false})).unwrap();
        assert!(!p.keep_metadata);
        let p: ResetDatabaseParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(p, ResetDatabaseParams::default());
        assert!(p.keep_metadata);
        let p: ResetDatabaseParams =
            serde_json::from_value(json!({"keepMetadata": false})).unwrap();
        assert!(!p.keep_metadata);
    }

    #[test]
    fn camel_case_keys() {
        let p: TaskQueueParams =
            serde_json::from_value(json!({"accountId": "a", "task": {"id": "t"}})).unwrap();
        assert_eq!(p.account_id, "a");
        let p: TaskCancelParams =
            serde_json::from_value(json!({"accountId": "a", "taskId": "t"})).unwrap();
        assert_eq!(p.task_id, "t");
        let p: SyncWakeParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(p.account_id, None);
        let p: SubscribeParams =
            serde_json::from_value(json!({"accounts": null, "classes": ["Thread"]})).unwrap();
        assert_eq!(p.classes, Some(vec!["Thread".to_owned()]));
        assert_eq!(serde_json::to_value(Empty {}).unwrap(), json!({}));

        let s = EngineStatusResult {
            protocol: 1,
            engine: "0.1.0".into(),
            pid: 1,
            uptime_secs: 3,
            features: vec![crate::feature::COUNTS.into()],
            capabilities: None,
            accounts: vec![AccountStatus {
                account_id: "a".into(),
                email: "e".into(),
                state: "running".into(),
                connection_error: false,
                error: None,
                account: None,
            }],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["uptimeSecs"], 3);
        assert_eq!(v["accounts"][0]["connectionError"], false);
        assert!(v["accounts"][0].get("error").is_none());
        assert_eq!(v["features"], json!(["counts"]));
        assert_eq!(serde_json::from_value::<EngineStatusResult>(v).unwrap(), s);

        let p: AccountAddParams = serde_json::from_value(json!({"account": {"id": "a"}})).unwrap();
        assert_eq!(p.identity, Value::Null);
        let n = AccountStateParams {
            account_id: "a".into(),
            state: crate::account_state::STOPPED.into(),
            error: Some(json!({"key": "x"})),
        };
        assert_eq!(
            serde_json::to_value(&n).unwrap(),
            json!({"accountId": "a", "state": "stopped", "error": {"key": "x"}})
        );
    }
}
