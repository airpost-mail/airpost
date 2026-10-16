// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Translation: LibreTranslate (`/translate`, `/detect`, `/languages`; docs.libretranslate.com)
//! and DeepL (`/v2/translate`, `/v2/languages`, `/v2/usage`; developers.deepl.com).
//!
//! The desktop's translation plugin sends one HTML string of `<b>…</b>` text blocks
//! (`translation/lib/service.ts`) and reads the blocks back from the result, so HTML is
//! translated as markup on both providers.

use std::fmt;

use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cache::KeyBuilder;
use crate::config::{DeepLConfig, LibreTranslateConfig, TranslationProvider};
use crate::error::{IntegrationError, Result};
use crate::http::{self, is_success, parse, provider_message, status_error, Http, Reply};
use crate::{Integrations, Language, TestReport, Usage};

const LIBRETRANSLATE: &str = "libretranslate";
const DEEPL: &str = "deepl";

/// DeepL reports this `character_limit` for an account without one.
const DEEPL_NO_LIMIT: u64 = 1_000_000_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextFormat {
    #[default]
    Text,
    Html,
}

impl TextFormat {
    fn as_str(self) -> &'static str {
        match self {
            TextFormat::Text => "text",
            TextFormat::Html => "html",
        }
    }
}

/// `source` absent, empty or `"auto"` means detect. Codes are the desktop's (ISO 639-1,
/// lowercase: `en`, `pt`, `zh`, `no`); see [`to_provider_code`].
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateRequest {
    pub text: String,
    #[serde(default)]
    pub source: Option<String>,
    pub target: String,
    #[serde(default)]
    pub format: TextFormat,
}

impl fmt::Debug for TranslateRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TranslateRequest")
            .field("text_len", &self.text.len())
            .field("source", &self.source)
            .field("target", &self.target)
            .field("format", &self.format)
            .finish()
    }
}

/// LibreTranslate's response shape, which the desktop's plugin was written against.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Translation {
    pub translated_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_language: Option<DetectedLanguage>,
}

/// `confidence` is 0-100 (LibreTranslate); DeepL reports none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedLanguage {
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

struct Normalised<'a> {
    text: &'a str,
    source: Option<&'a str>,
    target: &'a str,
    format: TextFormat,
}

impl<'a> Normalised<'a> {
    fn new(req: &'a TranslateRequest) -> Result<Self> {
        let target = req.target.trim();
        if target.is_empty() {
            return Err(IntegrationError::bad_request("no target language"));
        }
        let source = req
            .source
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"));
        Ok(Normalised {
            text: &req.text,
            source,
            target,
            format: req.format,
        })
    }
}

pub(crate) fn translate(
    ix: &Integrations,
    provider: &TranslationProvider,
    req: &TranslateRequest,
) -> Result<Translation> {
    let n = Normalised::new(req)?;
    let (name, base, key) = match provider {
        TranslationProvider::LibreTranslate(c) => (LIBRETRANSLATE, c.base_url()?, c.key()),
        TranslationProvider::DeepL(c) => (DEEPL, c.base_url()?, Some(c.key()?)),
    };
    if n.text.trim().is_empty() {
        return Ok(Translation {
            translated_text: n.text.to_owned(),
            detected_language: None,
        });
    }
    let cache_key = KeyBuilder::new("translate")
        .part(name)
        .part(&base)
        .part(key.unwrap_or(""))
        .part(&n.source.unwrap_or("auto").to_ascii_lowercase())
        .part(&n.target.to_ascii_lowercase())
        .part(n.format.as_str())
        .part(n.text)
        .finish();
    if let Some(hit) = ix.translations.get(&cache_key) {
        return Ok(hit);
    }
    let translation = match provider {
        TranslationProvider::LibreTranslate(c) => libre_translate(&ix.http, c, &base, &n)?,
        TranslationProvider::DeepL(c) => deepl_translate(&ix.http, c, &base, &n)?,
    };
    ix.translations
        .insert(cache_key, translation.clone(), ix.options.translation_ttl);
    Ok(translation)
}

pub(crate) fn languages(http: &Http, provider: &TranslationProvider) -> Result<Vec<Language>> {
    match provider {
        TranslationProvider::LibreTranslate(c) => libre_languages(http, c),
        TranslationProvider::DeepL(c) => deepl_languages(http, c),
    }
}

pub(crate) fn detect(
    http: &Http,
    provider: &TranslationProvider,
    text: &str,
) -> Result<Vec<DetectedLanguage>> {
    match provider {
        TranslationProvider::LibreTranslate(c) => libre_detect(http, c, text),
        TranslationProvider::DeepL(_) => Err(IntegrationError::Unsupported {
            provider: "DeepL",
            operation: "language detection",
        }),
    }
}

// LibreTranslate

fn libre_body(config: &LibreTranslateConfig, mut body: Value) -> Value {
    if let Some(key) = config.key() {
        body["api_key"] = Value::from(key);
    }
    body
}

/// LibreTranslate answers a key-requiring server's keyless request with 400 "Visit … to get
/// an API key" and bans flooding IPs with 403 "Too many request limits violations"
/// (`libretranslate/app.py`, `access_check`).
fn libre_error(reply: &Reply) -> IntegrationError {
    let message = provider_message(&reply.body);
    match reply.status {
        400 if message.contains("API key") => IntegrationError::Auth { status: 400 },
        403 if message.contains("Too many request limits") => IntegrationError::RateLimited {
            retry_after: reply.retry_after,
        },
        _ => status_error(reply),
    }
}

fn libre_post<T: serde::de::DeserializeOwned>(
    http: &Http,
    config: &LibreTranslateConfig,
    base: &str,
    path: &str,
    body: Value,
) -> Result<T> {
    let url = http::url(base, path, &[])?;
    let req = http
        .client()
        .post(url.clone())
        .json(&libre_body(config, body));
    let reply = http.send(LIBRETRANSLATE, &url, req)?;
    if !is_success(&reply) {
        return Err(libre_error(&reply));
    }
    parse(&reply)
}

fn libre_translate(
    http: &Http,
    config: &LibreTranslateConfig,
    base: &str,
    n: &Normalised,
) -> Result<Translation> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        translated_text: String,
        #[serde(default)]
        detected_language: Option<DetectedLanguage>,
    }
    let body = json!({
        "q": n.text,
        "source": n.source.map_or("auto", to_provider_code),
        "target": to_provider_code(n.target),
        "format": n.format.as_str(),
    });
    let r: Response = libre_post(http, config, base, "/translate", body)?;
    Ok(Translation {
        translated_text: r.translated_text,
        detected_language: r.detected_language.map(desktop_detected),
    })
}

fn libre_detect(
    http: &Http,
    config: &LibreTranslateConfig,
    text: &str,
) -> Result<Vec<DetectedLanguage>> {
    let base = config.base_url()?;
    let found: Vec<DetectedLanguage> =
        libre_post(http, config, &base, "/detect", json!({ "q": text }))?;
    Ok(found.into_iter().map(desktop_detected).collect())
}

fn libre_languages(http: &Http, config: &LibreTranslateConfig) -> Result<Vec<Language>> {
    #[derive(Deserialize)]
    struct Entry {
        code: String,
        name: String,
        #[serde(default)]
        targets: Vec<String>,
    }
    let base = config.base_url()?;
    let url = http::url(&base, "/languages", &[])?;
    let reply = http.send(LIBRETRANSLATE, &url, http.client().get(url.clone()))?;
    if !is_success(&reply) {
        return Err(libre_error(&reply));
    }
    let entries: Vec<Entry> = parse(&reply)?;
    Ok(desktop_languages(entries.into_iter().map(|e| Language {
        code: e.code,
        name: e.name,
        targets: e.targets,
    })))
}

pub(crate) fn test_libretranslate(
    http: &Http,
    config: &LibreTranslateConfig,
) -> Result<TestReport> {
    let base = config.base_url()?;
    libre_detect(http, config, "Hello")?;
    Ok(TestReport {
        host: host_of(&base),
        usage: None,
    })
}

// DeepL

/// DeepL deprecated the bare `EN` and `PT` targets in favour of a variant.
fn deepl_target(code: &str) -> String {
    let code = to_provider_code(code).to_ascii_uppercase();
    match code.as_str() {
        "EN" => "EN-US".into(),
        "PT" => "PT-BR".into(),
        _ => code,
    }
}

/// Source languages have no variants (`EN`, not `EN-GB`).
fn deepl_source(code: &str) -> String {
    let code = to_provider_code(code).to_ascii_uppercase();
    code.split('-').next().unwrap_or(&code).to_owned()
}

fn deepl_error(reply: &Reply) -> IntegrationError {
    match reply.status {
        // 456: the character quota is used up (developers.deepl.com, "Error handling").
        456 => IntegrationError::QuotaExceeded { status: 456 },
        _ => status_error(reply),
    }
}

fn deepl_auth(config: &DeepLConfig) -> Result<String> {
    Ok(format!("DeepL-Auth-Key {}", config.key()?))
}

fn deepl_get<T: serde::de::DeserializeOwned>(
    http: &Http,
    config: &DeepLConfig,
    path: &str,
    query: &[(&str, &str)],
) -> Result<T> {
    let base = config.base_url()?;
    let url = http::url(&base, path, query)?;
    let req = http
        .client()
        .get(url.clone())
        .header(AUTHORIZATION, deepl_auth(config)?);
    let reply = http.send(DEEPL, &url, req)?;
    if !is_success(&reply) {
        return Err(deepl_error(&reply));
    }
    parse(&reply)
}

fn deepl_translate(
    http: &Http,
    config: &DeepLConfig,
    base: &str,
    n: &Normalised,
) -> Result<Translation> {
    #[derive(Deserialize)]
    struct Response {
        translations: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        text: String,
        #[serde(default)]
        detected_source_language: Option<String>,
    }
    let mut body = json!({
        "text": [n.text],
        "target_lang": deepl_target(n.target),
    });
    if let Some(source) = n.source {
        body["source_lang"] = Value::from(deepl_source(source));
    }
    if n.format == TextFormat::Html {
        body["tag_handling"] = Value::from("html");
    }
    let url = http::url(base, "/v2/translate", &[])?;
    let req = http
        .client()
        .post(url.clone())
        .header(AUTHORIZATION, deepl_auth(config)?)
        .json(&body);
    let reply = http.send(DEEPL, &url, req)?;
    if !is_success(&reply) {
        return Err(deepl_error(&reply));
    }
    let r: Response = parse(&reply)?;
    let item = r
        .translations
        .into_iter()
        .next()
        .ok_or_else(|| IntegrationError::unreadable(reply.status))?;
    Ok(Translation {
        translated_text: item.text,
        detected_language: item.detected_source_language.map(|l| DetectedLanguage {
            language: to_desktop_code(&l),
            confidence: None,
        }),
    })
}

/// `GET /v2/languages?type=target`. DeepL marks it deprecated in favour of
/// `/v3/languages` with no sunset date; v2 is what Free and Pro both serve today.
fn deepl_languages(http: &Http, config: &DeepLConfig) -> Result<Vec<Language>> {
    #[derive(Deserialize)]
    struct Entry {
        language: String,
        name: String,
    }
    let entries: Vec<Entry> = deepl_get(http, config, "/v2/languages", &[("type", "target")])?;
    Ok(desktop_languages(entries.into_iter().map(|e| Language {
        code: e.language,
        name: e.name,
        targets: Vec::new(),
    })))
}

pub(crate) fn test_deepl(http: &Http, config: &DeepLConfig) -> Result<TestReport> {
    #[derive(Deserialize)]
    struct Response {
        character_count: u64,
        character_limit: Option<u64>,
    }
    let r: Response = deepl_get(http, config, "/v2/usage", &[])?;
    Ok(TestReport {
        host: host_of(&config.base_url()?),
        usage: Some(Usage {
            used: r.character_count,
            limit: r.character_limit.filter(|l| *l < DEEPL_NO_LIMIT),
        }),
    })
}

// Language codes

/// A desktop code as both providers spell it. The desktop's Norwegian is `no`; both
/// providers only know Bokmål, `nb`. Other codes pass through: LibreTranslate maps `zh` and
/// `pt` to its models itself (`libretranslate/language.py`, `iso2model`).
fn to_provider_code(code: &str) -> &str {
    if code.eq_ignore_ascii_case("no") {
        "nb"
    } else {
        code
    }
}

/// A provider's code as the desktop's `AllLanguages` keys it: lowercase, `nb` → `no`, and
/// `zh-hans` (LibreTranslate's `model2iso` for `zh`, DeepL's Simplified Chinese target) →
/// `zh`. Region variants with no desktop key (`pt-br`, `zh-hant`, `en-gb`) stay as they are.
fn to_desktop_code(code: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    match code.as_str() {
        "nb" => "no".into(),
        "zh-hans" => "zh".into(),
        _ => code,
    }
}

fn desktop_detected(d: DetectedLanguage) -> DetectedLanguage {
    DetectedLanguage {
        language: to_desktop_code(&d.language),
        ..d
    }
}

/// Codes mapped with [`to_desktop_code`]; when two map to one (DeepL lists `ZH` and
/// `ZH-HANS`) the first is kept.
fn desktop_languages(langs: impl Iterator<Item = Language>) -> Vec<Language> {
    let mut seen = std::collections::HashSet::new();
    langs
        .filter_map(|l| {
            let code = to_desktop_code(&l.code);
            if !seen.insert(code.clone()) {
                return None;
            }
            let mut targets: Vec<String> = Vec::with_capacity(l.targets.len());
            for t in l.targets.iter().map(|t| to_desktop_code(t)) {
                if !targets.contains(&t) {
                    targets.push(t);
                }
            }
            Some(Language {
                code,
                name: l.name,
                targets,
            })
        })
        .collect()
}

pub(crate) fn host_of(base: &str) -> String {
    reqwest::Url::parse(base)
        .ok()
        .and_then(|u| {
            let host = u.host_str()?.to_owned();
            Some(match u.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepl_codes() {
        assert_eq!(deepl_target("en"), "EN-US");
        assert_eq!(deepl_target("pt"), "PT-BR");
        assert_eq!(deepl_target("en-gb"), "EN-GB");
        assert_eq!(deepl_target("de"), "DE");
        assert_eq!(deepl_source("en-gb"), "EN");
        assert_eq!(deepl_source("fr"), "FR");
        assert_eq!(deepl_target("no"), "NB");
        assert_eq!(deepl_source("no"), "NB");
    }

    #[test]
    fn desktop_codes() {
        assert_eq!(to_provider_code("no"), "nb");
        assert_eq!(to_provider_code("zh"), "zh");
        assert_eq!(to_desktop_code("NB"), "no");
        assert_eq!(to_desktop_code("zh-Hans"), "zh");
        assert_eq!(to_desktop_code("ZH-HANT"), "zh-hant");
        assert_eq!(to_desktop_code("pt-BR"), "pt-br");
        let lang = |code: &str, targets: &[&str]| Language {
            code: code.into(),
            name: code.into(),
            targets: targets.iter().map(|t| t.to_string()).collect(),
        };
        let langs = desktop_languages(
            [
                lang("en", &["nb", "zh", "zh-Hans", "pt-BR"]),
                lang("ZH", &[]),
                lang("ZH-HANS", &[]),
            ]
            .into_iter(),
        );
        assert_eq!(langs.len(), 2);
        assert_eq!(langs[0].targets, ["no", "zh", "pt-br"]);
        assert_eq!(langs[1].code, "zh");
    }
}
