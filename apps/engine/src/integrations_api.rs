// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The `integrations.*` methods on `serve`'s socket,
//! over [`airpost_integrations`], and the engine's `capabilities`.
//! Each call carries its provider `config`; a key it lacks comes from the secret
//! store (`integration:<provider>`, [`with_stored_key`]). Nothing here keeps or logs it. The
//! socket runs the calls on the integrations workers ([`crate::workers::INTEGRATIONS`]), since
//! one can take up to 30 s.

use std::sync::{Arc, Mutex};

use airpost_integrations::{
    GrammarRequest, IntegrationError, Integrations, ProviderConfig, TranslateRequest,
    TranslationProvider,
};
use airpost_ipc::{
    capability_reason, code, integration_kind, method, server_feature, Capabilities, Capability,
    ErrorObject, Request as RpcRequest, CREDENTIALS_PER_REQUEST,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::dispatch::Reply;

pub fn is_integrations_method(name: &str) -> bool {
    matches!(
        name,
        method::INTEGRATIONS_TRANSLATE
            | method::INTEGRATIONS_DETECT
            | method::INTEGRATIONS_LANGUAGES
            | method::INTEGRATIONS_GRAMMAR
            | method::INTEGRATIONS_ENRICH
            | method::INTEGRATIONS_TEST
    )
}

/// The params: the provider `config` and the request's own fields beside it.
#[derive(Deserialize)]
struct WithConfig<C, R> {
    config: C,
    #[serde(flatten)]
    request: R,
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

#[derive(Deserialize)]
struct Email {
    email: String,
}

#[derive(Deserialize)]
struct Nothing {}

fn params<C: DeserializeOwned, R: DeserializeOwned>(
    req: &RpcRequest,
) -> Result<(C, R), ErrorObject> {
    req.params_as::<WithConfig<C, R>>()
        .map(|p| (p.config, p.request))
}

/// `-32010` with the serialized error as `data`. Its text never holds a key, a username,
/// the request text or an address.
fn failed(e: IntegrationError) -> ErrorObject {
    let data = serde_json::to_value(&e).unwrap_or(Value::Null);
    ErrorObject::new(code::INTEGRATION_FAILED, format!("Integration failed: {e}")).with_data(data)
}

fn answer<T: serde::Serialize>(r: airpost_integrations::Result<T>) -> Reply {
    let value = r.map_err(failed)?;
    serde_json::to_value(value).map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
}

/// The process's [`Integrations`], made on the first call: its caches are shared by every
/// connection.
fn integrations() -> Result<Arc<Integrations>, ErrorObject> {
    static SHARED: Mutex<Option<Arc<Integrations>>> = Mutex::new(None);
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ix) = &*shared {
        return Ok(ix.clone());
    }
    let ix = Arc::new(Integrations::new().map_err(failed)?);
    *shared = Some(ix.clone());
    Ok(ix)
}

/// `req` with a `config` that names a provider but no `apiKey` given the key stored under
/// `integration:<provider>` (`secrets.set`), when there is one. LanguageTool's key goes
/// with a username, so it is filled in only beside one. The key goes only to the provider's
/// default endpoint (no `url`) or to the `url` the stored settings name for that provider
/// (`core.integrations.<kind>`): a caller that names another host must bring its own key,
/// or `integrations.*` would read out a write-only secret.
pub fn with_stored_key(
    mut req: RpcRequest,
    settings: &Map<String, Value>,
    secret: impl Fn(&str) -> Option<String>,
) -> RpcRequest {
    let Some(config) = req.params.get_mut("config").and_then(Value::as_object_mut) else {
        return req;
    };
    let blank = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
    };
    let Some(provider) = config.get("provider").and_then(Value::as_str) else {
        return req;
    };
    if !blank(config.get("apiKey")) || (provider == "languagetool" && blank(config.get("username")))
    {
        return req;
    }
    if !blank(config.get("url")) && !is_stored_url(settings, provider, config.get("url")) {
        return req;
    }
    if let Some(key) = secret(&airpost_secrets::names::integration(provider)) {
        config.insert("apiKey".into(), Value::String(key));
    }
    req
}

/// Whether `url` is the one the stored settings give `provider`.
fn is_stored_url(settings: &Map<String, Value>, provider: &str, url: Option<&Value>) -> bool {
    let norm = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(|u| u.trim().trim_end_matches('/').to_owned())
            .filter(|u| !u.is_empty())
    };
    let Some(url) = norm(url) else {
        return false;
    };
    let Some((kind, _)) = KINDS.iter().find(|(_, ps)| ps.contains(&provider)) else {
        return false;
    };
    let Some(stored) = settings
        .get(&format!("core.integrations.{kind}"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    stored.get("provider").and_then(Value::as_str) == Some(provider)
        && norm(stored.get("url")).as_deref() == Some(url.as_str())
}

/// Answers one `integrations.*` request. Blocking: run it on a worker.
pub fn call(req: &RpcRequest) -> Reply {
    // Params first, so bad JSON is invalid params even when the HTTP client can't start.
    match req.method.as_str() {
        method::INTEGRATIONS_TRANSLATE => {
            let (provider, request) = params::<TranslationProvider, TranslateRequest>(req)?;
            answer(integrations()?.translate(&provider, &request))
        }
        method::INTEGRATIONS_DETECT => {
            let (provider, Text { text }) = params::<TranslationProvider, Text>(req)?;
            answer(integrations()?.detect_language(&provider, &text))
        }
        method::INTEGRATIONS_LANGUAGES => {
            let (config, Nothing {}) = params::<ProviderConfig, Nothing>(req)?;
            let ix = integrations()?;
            match config {
                ProviderConfig::LibreTranslate(c) => {
                    answer(ix.translation_languages(&TranslationProvider::LibreTranslate(c)))
                }
                ProviderConfig::DeepL(c) => {
                    answer(ix.translation_languages(&TranslationProvider::DeepL(c)))
                }
                ProviderConfig::LanguageTool(c) => answer(ix.grammar_languages(&c)),
                ProviderConfig::PeopleDataLabs(_) => Err(ErrorObject::new(
                    code::INVALID_PARAMS,
                    "invalid params: integrations.languages takes a translation provider or \
                     LanguageTool",
                )),
            }
        }
        method::INTEGRATIONS_GRAMMAR => {
            let (config, request) = params::<_, GrammarRequest>(req)?;
            answer(integrations()?.check_grammar(&config, &request))
        }
        method::INTEGRATIONS_ENRICH => {
            let (config, Email { email }) = params::<_, Email>(req)?;
            answer(integrations()?.enrich_contact(&config, &email))
        }
        method::INTEGRATIONS_TEST => {
            let (config, Nothing {}) = params::<ProviderConfig, Nothing>(req)?;
            answer(integrations()?.test(&config))
        }
        other => Err(ErrorObject::new(
            code::METHOD_NOT_FOUND,
            format!("unknown method {other}"),
        )),
    }
}

/// The providers of each kind, the only one first where the kind has one: the desktop
/// picks it without asking (`readSettings` in src/engine/client/integrations.ts).
const KINDS: &[(&str, &[&str])] = &[
    (integration_kind::TRANSLATION, &["libretranslate", "deepl"]),
    (integration_kind::GRAMMAR, &["languagetool"]),
    (integration_kind::ENRICHMENT, &["peopledatalabs"]),
];

const SERVER_FEATURES: &[&str] = &[
    server_feature::READ_RECEIPTS,
    server_feature::LINK_TRACKING,
    server_feature::THREAD_SHARING,
    server_feature::ACTIVITY_SHARING,
    server_feature::METADATA_SYNC,
];

/// The settings keys the capabilities follow.
pub fn is_capability_setting(key: &str) -> bool {
    key.starts_with("core.integrations.")
}

/// The engine's `capabilities`, from the stored settings (`core.integrations.<kind>`,
/// `{provider, enabled, url?, username?}`). The keys are the client's, so an integration is
/// available once its provider is on and has what the engine can check (the LibreTranslate
/// URL); the client still checks for its key.
pub fn capabilities(settings: &Map<String, Value>) -> Capabilities {
    let mut caps = Capabilities::default();
    for (kind, providers) in KINDS {
        let stored = settings
            .get(&format!("core.integrations.{kind}"))
            .and_then(Value::as_object);
        let field = |name: &str| stored.and_then(|s| s.get(name));
        let provider = field("provider")
            .and_then(Value::as_str)
            .filter(|p| providers.contains(p))
            .or(if providers.len() == 1 {
                Some(providers[0])
            } else {
                None
            });
        let enabled = field("enabled").and_then(Value::as_bool) == Some(true);
        let has_url = field("url")
            .and_then(Value::as_str)
            .is_some_and(|u| !u.trim().is_empty());
        let reason = match provider {
            None => Some(capability_reason::NOT_SET_UP),
            Some(_) if !enabled => Some(capability_reason::NOT_SET_UP),
            Some("libretranslate") if !has_url => Some(capability_reason::MISSING_URL),
            Some(_) => None,
        };
        caps.integrations.insert(
            (*kind).to_owned(),
            Capability {
                available: reason.is_none(),
                provider: provider.map(str::to_owned),
                credentials: reason.is_none().then(|| CREDENTIALS_PER_REQUEST.to_owned()),
                reason: reason.map(str::to_owned),
                ..Capability::default()
            },
        );
    }
    for feature in SERVER_FEATURES {
        caps.server.insert(
            (*feature).to_owned(),
            Capability {
                available: false,
                reason: Some(capability_reason::NEEDS_AIRPOST_SERVER.to_owned()),
                ..Capability::default()
            },
        );
    }
    caps
}

/// The server features the engine serves once a server is connected: open and link tracking
/// when `tracking` (the server's `features.tracking`, `None` without a server) is on, done
/// by the engine's tracking transforms (T22). The others stay as [`capabilities`] left them.
pub fn apply_server(caps: &mut Capabilities, tracking: Option<bool>) {
    for feature in [server_feature::READ_RECEIPTS, server_feature::LINK_TRACKING] {
        let reason = match tracking {
            None => Some(capability_reason::NEEDS_AIRPOST_SERVER),
            Some(false) => Some(capability_reason::NOT_ON_SERVER),
            Some(true) => None,
        };
        caps.server.insert(
            feature.to_owned(),
            Capability {
                available: reason.is_none(),
                reason: reason.map(str::to_owned),
                ..Capability::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn caps(settings: Value) -> Value {
        serde_json::to_value(capabilities(settings.as_object().unwrap())).unwrap()
    }

    #[test]
    fn tracking_follows_the_connected_server() {
        let with = |tracking| {
            let mut c = capabilities(&Map::new());
            apply_server(&mut c, tracking);
            serde_json::to_value(c).unwrap()["server"].clone()
        };
        let none = with(None);
        assert_eq!(none["readReceipts"]["reason"], "needs-airpost-server");
        assert_eq!(none["linkTracking"]["available"], false);
        let off = with(Some(false));
        assert_eq!(off["linkTracking"]["reason"], "not-on-server");
        let on = with(Some(true));
        assert_eq!(on["readReceipts"], json!({"available": true}));
        assert_eq!(on["linkTracking"], json!({"available": true}));
        assert_eq!(on["threadSharing"]["reason"], "needs-airpost-server");
    }

    #[test]
    fn stored_keys_fill_missing_ones() {
        let req = |config: Value| {
            RpcRequest::new(1, method::INTEGRATIONS_TEST, json!({ "config": config }))
        };
        let none = Map::new();
        let store = |n: &str| (n == "integration:deepl").then(|| "stored".to_owned());
        let filled =
            |config: Value| with_stored_key(req(config), &none, store).params["config"].clone();
        assert_eq!(filled(json!({"provider": "deepl"}))["apiKey"], "stored");
        assert_eq!(
            filled(json!({"provider": "deepl", "apiKey": " "}))["apiKey"],
            "stored"
        );
        assert_eq!(
            filled(json!({"provider": "deepl", "apiKey": "mine"}))["apiKey"],
            "mine"
        );
        assert_eq!(
            filled(json!({"provider": "libretranslate"})).get("apiKey"),
            None
        );
        let lt = |n: &str| (n == "integration:languagetool").then(|| "k".to_owned());
        let no_user = with_stored_key(req(json!({"provider": "languagetool"})), &none, lt);
        assert_eq!(no_user.params["config"].get("apiKey"), None);
        let user = with_stored_key(
            req(json!({"provider": "languagetool", "username": "u"})),
            &none,
            lt,
        );
        assert_eq!(user.params["config"]["apiKey"], "k");
    }

    #[test]
    fn stored_keys_go_only_to_the_stored_url() {
        let req = |config: Value| {
            RpcRequest::new(1, method::INTEGRATIONS_TEST, json!({ "config": config }))
        };
        let store = |_: &str| Some("stored".to_owned());
        let settings = json!({
            "core.integrations.translation": {"provider": "libretranslate", "enabled": true, "url": "https://lt.example/"},
            "core.integrations.grammar": {"provider": "languagetool", "enabled": true, "username": "u"},
        });
        let settings = settings.as_object().unwrap();
        let key = |config: Value| {
            with_stored_key(req(config), settings, store).params["config"]
                .get("apiKey")
                .cloned()
        };
        // A foreign host gets no key, whatever the provider.
        let evil = "http://evil.example";
        assert_eq!(key(json!({"provider": "deepl", "url": evil})), None);
        assert_eq!(
            key(json!({"provider": "libretranslate", "url": evil})),
            None
        );
        assert_eq!(
            key(json!({"provider": "languagetool", "username": "u", "url": evil})),
            None
        );
        assert_eq!(
            key(json!({"provider": "peopledatalabs", "url": evil})),
            None
        );
        // Nor does the stored url under another provider.
        assert_eq!(
            key(json!({"provider": "deepl", "url": "https://lt.example"})),
            None
        );
        // The stored url, trimmed, and the default endpoint do.
        assert_eq!(
            key(json!({"provider": "libretranslate", "url": " https://lt.example "})),
            Some(json!("stored"))
        );
        assert_eq!(
            key(json!({"provider": "deepl", "url": ""})),
            Some(json!("stored"))
        );
        assert_eq!(
            key(json!({"provider": "peopledatalabs"})),
            Some(json!("stored"))
        );
    }

    #[test]
    fn integrations_follow_the_settings() {
        let none = caps(json!({}));
        assert_eq!(
            none["integrations"]["translation"],
            json!({"available": false, "reason": "not-set-up"})
        );
        assert_eq!(
            none["integrations"]["grammar"],
            json!({"available": false, "provider": "languagetool", "reason": "not-set-up"})
        );
        assert_eq!(
            none["server"]["linkTracking"],
            json!({"available": false, "reason": "needs-airpost-server"})
        );

        let set = caps(json!({
            "core.integrations.translation": {"provider": "libretranslate", "enabled": true, "url": " "},
            "core.integrations.grammar": {"provider": "languagetool", "enabled": true},
            "core.integrations.enrichment": {"provider": "nope", "enabled": true},
        }));
        assert_eq!(
            set["integrations"]["translation"],
            json!({"available": false, "provider": "libretranslate", "reason": "missing-url"})
        );
        assert_eq!(
            set["integrations"]["grammar"],
            json!({"available": true, "provider": "languagetool", "credentials": "request"})
        );
        // An unknown provider falls back to the kind's only one.
        assert_eq!(set["integrations"]["enrichment"]["available"], json!(true));

        let deepl = caps(json!({
            "core.integrations.translation": {"provider": "deepl", "enabled": true},
        }));
        assert_eq!(
            deepl["integrations"]["translation"]["available"],
            json!(true)
        );
        let off = caps(json!({
            "core.integrations.translation": {"provider": "deepl", "enabled": false},
        }));
        assert_eq!(
            off["integrations"]["translation"]["reason"],
            json!("not-set-up")
        );
    }

    #[test]
    fn bad_params_are_invalid_params() {
        let r = |m: &str, p: Value| call(&RpcRequest::new(1, m, p)).unwrap_err().code;
        assert_eq!(
            r(
                "integrations.translate",
                json!({"text": "x", "target": "es"})
            ),
            code::INVALID_PARAMS
        );
        assert_eq!(
            r(
                "integrations.translate",
                json!({"config": {"provider": "languagetool"}, "text": "x", "target": "es"})
            ),
            code::INVALID_PARAMS
        );
        assert_eq!(
            r(
                "integrations.languages",
                json!({"config": {"provider": "peopledatalabs", "apiKey": "k"}})
            ),
            code::INVALID_PARAMS
        );
        assert_eq!(
            r(
                "integrations.enrich",
                json!({"config": {"provider": "peopledatalabs", "apiKey": "k"}})
            ),
            code::INVALID_PARAMS
        );
    }

    #[test]
    fn a_missing_setting_is_integration_failed() {
        let e = call(&RpcRequest::new(
            1,
            "integrations.translate",
            json!({"config": {"provider": "libretranslate", "url": ""}, "text": "x", "target": "es"}),
        ))
        .unwrap_err();
        assert_eq!(e.code, code::INTEGRATION_FAILED);
        assert_eq!(e.data.unwrap()["key"], json!("not-configured"));
    }
}
