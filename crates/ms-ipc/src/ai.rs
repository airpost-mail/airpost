// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Params and results of the `ai.*` methods (docs/ai-plan.md section 3). The provider
//! config stays a JSON value here; the engine reads it as `ms_ai::ProviderConfig`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The features in [`Capabilities::ai`](crate::Capabilities::ai) and in
/// [`AiSettings::features`]. `reply` covers suggestions and drafts.
pub mod ai_feature {
    pub const SUMMARY: &str = "summary";
    pub const REPLY: &str = "reply";
    pub const TRIAGE: &str = "triage";
}

/// Which provider a call uses: `config`, the whole provider config with its key (local
/// clients, until the engine holds the keys), or `provider`, the id of one the engine keeps.
/// Its `Debug` hides `config`, which holds the key.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiProviderParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl std::fmt::Debug for AiProviderParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiProviderParams")
            .field("config", &self.config.as_ref().map(|_| "<redacted>"))
            .field("provider", &self.provider)
            .finish()
    }
}

/// The AI switches a local client sends with each call until the engine keeps them
/// (docs/ai-plan.md A4): the accounts opted in, the features turned on, and whether the
/// model may see addresses. The engine checks them before it reads any mail.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    /// Account ids.
    #[serde(default)]
    pub accounts: Vec<String>,
    /// By [`ai_feature`]: `true`, or an object whose `enabled` is `true`.
    #[serde(default)]
    pub features: Map<String, Value>,
    #[serde(default)]
    pub include_addresses: bool,
}

impl AiSettings {
    pub fn account_enabled(&self, account_id: &str) -> bool {
        self.accounts.iter().any(|a| a == account_id)
    }

    pub fn feature_enabled(&self, feature: &str) -> bool {
        match self.features.get(feature) {
            Some(Value::Bool(on)) => *on,
            Some(Value::Object(o)) => o.get("enabled") == Some(&Value::Bool(true)),
            _ => false,
        }
    }
}

/// `ai.models` and `ai.test`.
pub type AiModelsParams = AiProviderParams;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSummarizeParams {
    pub account_id: String,
    pub thread_id: String,
    /// Chosen by the client; the text streams as `ai.delta` with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    /// Ask again even when a summary is cached.
    #[serde(default)]
    pub refresh: bool,
    #[serde(flatten)]
    pub provider: AiProviderParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<AiSettings>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSuggestRepliesParams {
    pub account_id: String,
    pub thread_id: String,
    /// 3 when absent, at most 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// Chosen by the client so `ai.cancel` can stop the call. Suggestions don't stream: no
    /// `ai.delta` carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(flatten)]
    pub provider: AiProviderParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<AiSettings>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDraftReplyParams {
    pub account_id: String,
    pub thread_id: String,
    /// What the user typed ("say I'll call Tuesday").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    /// The text of the suggestion the user picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(flatten)]
    pub provider: AiProviderParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<AiSettings>,
}

impl std::fmt::Debug for AiDraftReplyParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiDraftReplyParams")
            .field("account_id", &self.account_id)
            .field("thread_id", &self.thread_id)
            .field("stream_id", &self.stream_id)
            .finish_non_exhaustive()
    }
}

/// `ai.cancel`: one of the two.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCancelParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDeltaParams {
    pub stream_id: String,
    pub text: String,
}

impl std::fmt::Debug for AiDeltaParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiDeltaParams")
            .field("stream_id", &self.stream_id)
            .finish_non_exhaustive()
    }
}

/// `ai.triage.run`: the threads to triage, by id or as the inbox threads of the last
/// `sinceDays` days; at most [`AI_TRIAGE_RUN_MAX`] either way. The provider is the one the
/// engine keeps for triage unless the call names one (`provider`) or, from a local client,
/// carries one with its key (`config`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTriageRunParams {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_days: Option<u32>,
    #[serde(flatten)]
    pub provider: AiProviderParams,
}

/// The most threads one `ai.triage.run` queues.
pub const AI_TRIAGE_RUN_MAX: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTriageRunResult {
    /// Named by `ai.triage.progress` and `ai.cancel {jobId}`.
    pub job_id: String,
    /// How many threads the job holds.
    pub queued: usize,
}

/// `ai.triage.progress`, to the connection that called `ai.triage.run`: `done` of `total`
/// threads handled, triaged or skipped. The last one has `done == total`, or `stopped`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTriageProgressParams {
    pub job_id: String,
    pub done: usize,
    pub total: usize,
    /// The job ended after `done` threads: it was cancelled, the account stopped, triage
    /// was turned off for it, or the provider it named went away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
}

/// `ai.triage.list`: the opted-in accounts' inbox threads with a triage result, newest
/// first. `category` is one of the triage categories; `needs-reply` also takes threads whose
/// result says `needsReply` under another category.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTriageListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// At most [`READ_MAX_LIMIT`](crate::READ_MAX_LIMIT).
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTriageListResult {
    pub thread_ids: Vec<String>,
    /// Every match, not just this page.
    pub total: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiUsageParams {
    /// 30 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
}

/// One row of `ai.usage`'s result, an array newest day first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageRow {
    /// `YYYY-MM-DD` in the engine's local time.
    pub day: String,
    /// The provider config's id.
    pub provider: String,
    /// From [`ai_feature`], or `test`.
    pub feature: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub requests: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_switches() {
        let s: AiSettings = serde_json::from_value(json!({
            "accounts": ["a1"],
            "features": {"summary": true, "reply": {"enabled": true, "provider": "p"},
                         "triage": {"enabled": false}},
        }))
        .unwrap();
        assert!(s.account_enabled("a1") && !s.account_enabled("a2"));
        assert!(s.feature_enabled("summary") && s.feature_enabled("reply"));
        assert!(!s.feature_enabled("triage") && !s.feature_enabled("other"));
        assert!(!s.include_addresses);
    }

    #[test]
    fn params_take_config_or_provider_beside_their_own() {
        let p: AiSummarizeParams = serde_json::from_value(json!({
            "accountId": "a1", "threadId": "t1", "streamId": "s1",
            "config": {"id": "p", "kind": "ollama"},
        }))
        .unwrap();
        assert_eq!(p.stream_id.as_deref(), Some("s1"));
        assert_eq!(p.provider.config.unwrap()["kind"], "ollama");
        assert!(p.settings.is_none() && !p.refresh);
        let p: AiSuggestRepliesParams =
            serde_json::from_value(json!({"accountId": "a1", "threadId": "t1", "provider": "p"}))
                .unwrap();
        assert_eq!(p.provider.provider.as_deref(), Some("p"));
        assert!(serde_json::from_value::<AiSummarizeParams>(json!({"accountId": "a1"})).is_err());
    }

    #[test]
    fn triage_shapes() {
        let p: AiTriageRunParams =
            serde_json::from_value(json!({"accountId": "a1", "sinceDays": 7})).unwrap();
        assert_eq!((p.since_days, p.thread_ids), (Some(7), None));
        assert!(p.provider.config.is_none());
        let r = AiTriageRunResult {
            job_id: "j".into(),
            queued: 3,
        };
        assert_eq!(
            serde_json::to_value(r).unwrap(),
            json!({"jobId": "j", "queued": 3})
        );
        let l: AiTriageListParams =
            serde_json::from_value(json!({"category": "needs-reply", "limit": 50})).unwrap();
        assert_eq!((l.limit, l.offset, l.account_ids), (50, None, None));
        assert_eq!(
            serde_json::to_value(AiTriageListResult::default()).unwrap(),
            json!({"threadIds": [], "total": 0})
        );
    }

    #[test]
    fn debug_hides_the_provider_config() {
        let config = json!({"id": "p", "kind": "openai", "apiKey": "sk-secret",
                            "url": "https://user:pw@example.com/v1"});
        let summarize: AiSummarizeParams = serde_json::from_value(json!({
            "accountId": "a1", "threadId": "t1", "config": config,
        }))
        .unwrap();
        let suggest: AiSuggestRepliesParams = serde_json::from_value(json!({
            "accountId": "a1", "threadId": "t1", "config": config,
        }))
        .unwrap();
        let d = format!("{summarize:?} {suggest:?} {:?}", summarize.provider);
        assert!(!d.contains("sk-secret") && !d.contains("pw@"), "{d}");
        assert!(d.contains("<redacted>"), "{d}");
    }
}
