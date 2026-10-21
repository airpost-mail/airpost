// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine-owned settings: `settings.get`,
//! `settings.set` and the `settings.changed` notification. Keys are the desktop's config key
//! paths, kept flat (`core.sending.undoSend`, not nested objects). A key with no stored value
//! is absent from every result; each client applies its own default.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The JSON type a key's value must have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Bool,
    /// A JSON number without a fraction, at least 0.
    Count,
    String,
    Object,
    Array,
}

impl SettingKind {
    pub fn accepts(self, v: &Value) -> bool {
        match self {
            SettingKind::Bool => v.is_boolean(),
            SettingKind::Count => v.as_u64().is_some(),
            SettingKind::String => v.is_string(),
            SettingKind::Object => v.is_object(),
            SettingKind::Array => v.is_array(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SettingKind::Bool => "a boolean",
            SettingKind::Count => "a whole number of at least 0",
            SettingKind::String => "a string",
            SettingKind::Object => "an object",
            SettingKind::Array => "an array",
        }
    }
}

/// Every key the store accepts, with its kind.
pub const SETTINGS_KEYS: &[(&str, SettingKind)] = &[
    // Milliseconds; 0 turns undo send off.
    ("core.sending.undoSend", SettingKind::Count),
    // An account id, or `selected-mailbox`.
    ("core.sending.defaultAccountIdForSend", SettingKind::String),
    ("core.notifications.unsnoozeToTop", SettingKind::Bool),
    ("core.composing.html", SettingKind::Bool),
    // Signature id -> `{id, title, body, data}`.
    ("signatures", SettingKind::Object),
    // Account email -> signature id (or null).
    ("defaultSignatures", SettingKind::Object),
    ("templates", SettingKind::Array),
    ("mailRules", SettingKind::Array),
    // Milliseconds since the epoch.
    ("mailRulesAutoSince", SettingKind::Count),
    ("resurfacing.snoozeBody", SettingKind::String),
    ("resurfacing.reminderBody", SettingKind::String),
    // The reply attribution, forward header and date pattern in the user's language and hour
    // cycle, for the drafts the engine builds (`ms_compose::settings::QuoteTexts`).
    ("composing.quoteTexts", SettingKind::Object),
    // `{provider, enabled, url?, username?}`, never a key.
    // They decide `capabilities.integrations`.
    ("core.integrations.translation", SettingKind::Object),
    ("core.integrations.grammar", SettingKind::Object),
    ("core.integrations.enrichment", SettingKind::Object),
    // The AI settings, never a key: the providers
    // (`[{id, kind, name?, url?, model, contextTokens?, maxOutputTokens?}]`), the features
    // (`{summary|reply|triage: {enabled, provider?}}`), the account ids opted in, the limits
    // (`{triagePerHour?, dailyTokens?: {<providerId>: n}}`) and "Include email addresses".
    // A relayed connection may read them but never write them (`policy`).
    ("core.ai.providers", SettingKind::Array),
    ("core.ai.features", SettingKind::Object),
    ("core.ai.accounts", SettingKind::Array),
    ("core.ai.limits", SettingKind::Object),
    ("core.ai.includeAddresses", SettingKind::Bool),
];

pub fn setting_kind(key: &str) -> Option<SettingKind> {
    SETTINGS_KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, kind)| *kind)
}

/// Why `key` can't take `value` (null clears any key), or `None` when it can.
pub fn setting_error(key: &str, value: &Value) -> Option<String> {
    match setting_kind(key) {
        None => Some(format!("{key:?} is not a setting the engine keeps")),
        Some(_) if value.is_null() => None,
        Some(_) if key == AI_PROVIDERS && holds_key(value) => Some(format!(
            "{key} takes no apiKey: the engine keeps provider keys in its secret store"
        )),
        Some(kind) if kind.accepts(value) => None,
        Some(kind) => Some(format!("{key} must be {}", kind.name())),
    }
}

const AI_PROVIDERS: &str = "core.ai.providers";

/// A provider entry with an `apiKey`, which the settings file must never hold.
fn holds_key(providers: &Value) -> bool {
    providers
        .as_array()
        .into_iter()
        .flatten()
        .any(|p| p.get("apiKey").is_some())
}

/// `settings.get` params. Without `keys`, every stored value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsGetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
}

/// `settings.get` result, `settings.set` params and `settings.changed` params. In
/// `settings.set`, null clears a key; in `settings.changed`, null means it was cleared.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsValues {
    pub values: Map<String, Value>,
    /// `settings.get` only: the store's id, new whenever the engine starts a store from
    /// nothing (no file, or a corrupt one moved aside). A client that copied its values into
    /// the store copies them again when the id changes, rather than taking the missing keys
    /// as cleared.
    #[serde(rename = "storeId", default, skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kinds_are_checked() {
        assert_eq!(setting_error("core.sending.undoSend", &json!(5000)), None);
        assert_eq!(setting_error("core.sending.undoSend", &Value::Null), None);
        assert!(setting_error("core.sending.undoSend", &json!(-1)).is_some());
        assert!(setting_error("core.sending.undoSend", &json!(1.5)).is_some());
        assert!(setting_error("core.sending.undoSend", &json!("5000")).is_some());
        assert_eq!(setting_error("signatures", &json!({"s1": {}})), None);
        assert!(setting_error("signatures", &json!([])).is_some());
        let unknown = setting_error("core.themes", &json!([])).unwrap();
        assert!(unknown.contains("not a setting"), "{unknown}");
    }

    #[test]
    fn ai_providers_take_no_key() {
        let keyless = json!([{"id": "p", "kind": "ollama", "model": "m"}]);
        assert_eq!(setting_error("core.ai.providers", &keyless), None);
        let keyed = json!([{"id": "p", "kind": "openai", "model": "m", "apiKey": "sk-x"}]);
        let e = setting_error("core.ai.providers", &keyed).unwrap();
        assert!(e.contains("apiKey") && !e.contains("sk-x"), "{e}");
        let empty_key = json!([{"id": "p", "kind": "openai", "apiKey": null}]);
        assert!(setting_error("core.ai.providers", &empty_key).is_some());
    }

    #[test]
    fn shapes() {
        let p: SettingsGetParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(p.keys, None);
        let v: SettingsValues =
            serde_json::from_value(json!({"values": {"core.composing.html": false}})).unwrap();
        assert_eq!(v.values["core.composing.html"], false);
        assert_eq!(v.store_id, None);
        assert_eq!(
            serde_json::to_value(&v).unwrap(),
            json!({"values": {"core.composing.html": false}})
        );
        let v = SettingsValues {
            store_id: Some("s".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&v).unwrap(),
            json!({"values": {}, "storeId": "s"})
        );
    }
}
