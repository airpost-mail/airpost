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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol: u32,
    /// The engine's version.
    pub engine: String,
    pub pid: u32,
    pub accounts: Vec<AccountInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub account_id: String,
    pub email: String,
    /// `running`; phase C adds `stopped`.
    pub state: String,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub account_id: String,
    pub email: String,
    pub state: String,
    /// The last `ProcessState` the account sent: true while it cannot reach its server.
    pub connection_error: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            accounts: vec![AccountStatus {
                account_id: "a".into(),
                email: "e".into(),
                state: "running".into(),
                connection_error: false,
            }],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["uptimeSecs"], 3);
        assert_eq!(v["accounts"][0]["connectionError"], false);
        assert_eq!(serde_json::from_value::<EngineStatusResult>(v).unwrap(), s);
    }
}
