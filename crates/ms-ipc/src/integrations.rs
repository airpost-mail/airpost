// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `capabilities` in `hello` and `engine.status`, and the `capabilities.changed`
//! notification (docs/desktop-client-plan.md 3.1): what this engine can do for its clients
//! now, so that every client shows the same availability.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The kinds in [`Capabilities::integrations`], as in the `core.integrations.<kind>`
/// settings.
pub mod integration_kind {
    pub const TRANSLATION: &str = "translation";
    pub const GRAMMAR: &str = "grammar";
    pub const ENRICHMENT: &str = "enrichment";
}

/// The features in [`Capabilities::server`].
pub mod server_feature {
    pub const READ_RECEIPTS: &str = "readReceipts";
    pub const LINK_TRACKING: &str = "linkTracking";
    pub const THREAD_SHARING: &str = "threadSharing";
    pub const ACTIVITY_SHARING: &str = "activitySharing";
    pub const METADATA_SYNC: &str = "metadataSync";
}

/// [`Capability::reason`] values. A client maps them to its own words and shows unknown ones
/// as a generic "unavailable".
pub mod capability_reason {
    /// No provider is chosen, or it is turned off.
    pub const NOT_SET_UP: &str = "not-set-up";
    /// The provider needs a URL (LibreTranslate) and has none.
    pub const MISSING_URL: &str = "missing-url";
    /// Only a Tern server offers it, and none is connected.
    pub const NEEDS_TERN_SERVER: &str = "needs-tern-server";
    /// A Tern server is connected but doesn't offer it (its `features` leave it off).
    pub const NOT_ON_SERVER: &str = "not-on-server";
    /// AI: the provider needs an API key and the secret store holds none under
    /// `integration:ai-<id>` (Ollama needs none).
    pub const NEEDS_ENGINE_KEY: &str = "needs-engine-key";
    /// AI triage: the provider is rate limiting; triage waits and resumes by itself.
    pub const RATE_LIMITED: &str = "rate-limited";
    /// AI triage: the provider's quota or the daily token cap is used up.
    pub const QUOTA_EXCEEDED: &str = "quota-exceeded";
}

/// [`Capability::credentials`]: the client sends the provider's key with each request (in
/// `config`), so the engine can't tell whether one is saved; the client checks. Engines
/// before desktop-client-plan stage 11 also set it on [`Capabilities::ai`] entries, meaning
/// the client sends the AI switches and applies them itself.
pub const CREDENTIALS_PER_REQUEST: &str = "request";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// By [`integration_kind`].
    #[serde(default)]
    pub integrations: BTreeMap<String, Capability>,
    /// By [`server_feature`].
    #[serde(default)]
    pub server: BTreeMap<String, Capability>,
    /// By [`ai_feature`](crate::ai_feature) (docs/ai-plan.md section 3): the AI features
    /// this engine serves. A feature it doesn't serve yet is absent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai: BTreeMap<String, Capability>,
    /// Groups a newer engine adds, kept as they came.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
