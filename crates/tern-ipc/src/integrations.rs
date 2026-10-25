// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `capabilities` in `hello` and `engine.status`, and the `capabilities.changed`
//! notification: what this engine can do for its clients
//! now, so that every client shows the same availability.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The kinds in [`Capabilities::integrations`], as in the `core.integrations.<kind>`
/// settings.
pub mod integration_kind {
    named_consts! { &str;
        TRANSLATION = "translation";
        GRAMMAR = "grammar";
        ENRICHMENT = "enrichment";
    }
}

/// The features in [`Capabilities::server`].
pub mod server_feature {
    named_consts! { &str;
        READ_RECEIPTS = "readReceipts";
        LINK_TRACKING = "linkTracking";
        THREAD_SHARING = "threadSharing";
        ACTIVITY_SHARING = "activitySharing";
        METADATA_SYNC = "metadataSync";
    }
}

/// [`Capability::reason`] values. A client maps them to its own words and shows unknown ones
/// as a generic "unavailable".
pub mod capability_reason {
    named_consts! { &str;
        /// No provider is chosen, or it is turned off.
        NOT_SET_UP = "not-set-up";
        /// The provider needs a URL (LibreTranslate) and has none.
        MISSING_URL = "missing-url";
        /// Only a Tern server offers it, and none is connected.
        NEEDS_TERN_SERVER = "needs-tern-server";
        /// A Tern server is connected but doesn't offer it (its `features` leave it off).
        NOT_ON_SERVER = "not-on-server";
        /// AI: the provider needs an API key and the secret store holds none under
        /// `integration:ai-<id>` (Ollama needs none).
        NEEDS_ENGINE_KEY = "needs-engine-key";
        /// AI triage: the provider is rate limiting; triage waits and resumes by itself.
        RATE_LIMITED = "rate-limited";
        /// AI triage: the provider's quota or the daily token cap is used up.
        QUOTA_EXCEEDED = "quota-exceeded";
    }
}

/// [`Capability::credentials`]: the client sends the provider's key with each request (in
/// `config`), so the engine can't tell whether one is saved; the client checks. Older engines
/// also set it on [`Capabilities::ai`] entries, meaning
/// the client sends the AI switches and applies them itself.
pub const CREDENTIALS_PER_REQUEST: &str = "request";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    /// By [`integration_kind`].
    #[serde(default)]
    pub integrations: BTreeMap<String, Capability>,
    /// By [`server_feature`].
    #[serde(default)]
    pub server: BTreeMap<String, Capability>,
    /// By [`ai_feature`](crate::ai_feature): the AI features
    /// this engine serves. A feature it doesn't serve yet is absent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai: BTreeMap<String, Capability>,
    /// Groups a newer engine adds, kept as they came.
    #[serde(flatten)]
    #[schemars(skip)]
    pub other: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Capability {
    pub available: bool,
    /// The provider the user chose, e.g. `deepl`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// [`CREDENTIALS_PER_REQUEST`], or absent when the engine needs none from the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
    /// Whether the provider runs on this computer or network (AI entries), which the
    /// privacy note names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<bool>,
    /// Why it is unavailable, from [`capability_reason`]; absent when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// An integration provider with its settings, key included (tern-integrations
/// `ProviderConfig`), as `config` in the `integrations.*` params.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationConfig {
    /// `libretranslate`, `deepl`, `languagetool` or `peopledatalabs`.
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

impl std::fmt::Debug for IntegrationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntegrationConfig")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

/// The `integrations.*` params: the provider's `config` and the call's own fields (`text`,
/// `target`, `email`, ...). Without `config` the engine uses its settings
/// ([`feature::ACCOUNT_SETUP`](crate::feature::ACCOUNT_SETUP)).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<IntegrationConfig>,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

/// `integrations.test`'s answer (tern-integrations `TestReport`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationTestReport {
    /// The host that answered.
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<IntegrationUsage>,
}

/// Characters used in the billing period (DeepL).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationUsage {
    pub used: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_and_keeps_unknown_groups() {
        let v = json!({
            "integrations": {
                "translation": {"available": true, "provider": "deepl", "credentials": "request"},
                "grammar": {"available": false, "reason": "not-set-up"},
            },
            "server": {"linkTracking": {"available": false, "reason": "needs-tern-server"}},
            "ai": {"summary": {"available": true, "credentials": "request"}},
            "later": {"x": {"available": false}},
        });
        let c: Capabilities = serde_json::from_value(v.clone()).unwrap();
        assert!(c.integrations["translation"].available);
        assert!(c.ai["summary"].available);
        assert_eq!(c.other["later"], json!({"x": {"available": false}}));
        assert_eq!(serde_json::to_value(&c).unwrap(), v);
    }
}
