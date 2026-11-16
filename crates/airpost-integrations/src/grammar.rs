// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Grammar checks on LanguageTool (`POST /v2/check`, `GET /v2/languages`;
//! languagetool.org/http-api). The free public server allows 20 requests and 75 KB per
//! minute per IP and 20 KB per request (dev.languagetool.org/public-http-api); premium
//! accounts authenticate with `username` and `apiKey` form fields.
//!
//! Matches are mapped to the desktop's `GrammarError`
//! (`composer-grammar-check/lib/grammar-check-service.ts`).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::config::LanguageToolConfig;
use crate::error::Result;
use crate::http::{self, is_success, parse, status_error, Http};
use crate::translate::host_of;
use crate::{Language, TestReport};

const LANGUAGETOOL: &str = "languagetool";

/// `language` absent or `"auto"` means detect; `preferredVariants` (`en-GB`, `de-AT`) only
/// applies then. Misspellings are dropped unless `includeMisspellings`, because the
/// composer's spellchecker already marks them.
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrammarRequest {
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub disabled_rules: Vec<String>,
    #[serde(default)]
    pub preferred_variants: Vec<String>,
    #[serde(default)]
    pub include_misspellings: bool,
}

impl fmt::Debug for GrammarRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrammarRequest")
            .field("text_len", &self.text.len())
            .field("language", &self.language)
            .field("disabled_rules", &self.disabled_rules)
            .field("preferred_variants", &self.preferred_variants)
            .field("include_misspellings", &self.include_misspellings)
            .finish()
    }
}

/// The desktop's `GrammarError`, field for field.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrammarError {
    pub offset: usize,
    pub length: usize,
    pub message: String,
    pub short_message: String,
    pub replacements: Vec<String>,
    pub rule_id: String,
    pub rule_description: String,
    pub category: String,
    pub category_name: String,
    pub issue_type: String,
    pub sentence: String,
    pub context_text: String,
    pub context_offset: usize,
    pub context_length: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrammarCheck {
    pub matches: Vec<GrammarError>,
    /// The language checked against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_language: Option<Language>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckResponse {
    #[serde(default)]
    language: Option<ResponseLanguage>,
    matches: Vec<Match>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseLanguage {
    name: String,
    code: String,
    #[serde(default)]
    detected_language: Option<NameCode>,
}

#[derive(Deserialize)]
struct NameCode {
    name: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Match {
    message: String,
    #[serde(default)]
    short_message: String,
    offset: usize,
    length: usize,
    #[serde(default)]
    replacements: Vec<Replacement>,
    #[serde(default)]
    context: Context,
    #[serde(default)]
    sentence: String,
    rule: Rule,
}

#[derive(Deserialize)]
struct Replacement {
    value: String,
}

#[derive(Default, Deserialize)]
struct Context {
    text: String,
    offset: usize,
    length: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rule {
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    issue_type: String,
    category: Category,
}

#[derive(Deserialize)]
struct Category {
    id: String,
    #[serde(default)]
    name: String,
}

impl From<Match> for GrammarError {
    fn from(m: Match) -> Self {
        GrammarError {
            offset: m.offset,
            length: m.length,
            message: m.message,
            short_message: m.short_message,
            replacements: m.replacements.into_iter().map(|r| r.value).collect(),
            rule_id: m.rule.id,
            rule_description: m.rule.description,
            category: m.rule.category.id,
            category_name: m.rule.category.name,
            issue_type: m.rule.issue_type,
            sentence: m.sentence,
            context_text: m.context.text,
            context_offset: m.context.offset,
            context_length: m.context.length,
        }
    }
}

fn language(name: String, code: String) -> Language {
    Language {
        code,
        name,
        targets: Vec::new(),
    }
}

pub(crate) fn check(
    http: &Http,
    config: &LanguageToolConfig,
    req: &GrammarRequest,
) -> Result<GrammarCheck> {
    let base = config.base_url()?;
    if req.text.trim().is_empty() {
        return Ok(GrammarCheck {
            matches: Vec::new(),
            language: None,
            detected_language: None,
        });
    }
    let lang = req
        .language
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .unwrap_or("auto");
    let disabled = req.disabled_rules.join(",");
    let variants = req.preferred_variants.join(",");
    let mut form: Vec<(&str, &str)> = vec![("text", &req.text), ("language", lang)];
    if !disabled.is_empty() {
        form.push(("disabledRules", &disabled));
    }
    if lang == "auto" && !variants.is_empty() {
        form.push(("preferredVariants", &variants));
    }
    if let Some((username, key)) = config.credentials()? {
        form.push(("username", username));
        form.push(("apiKey", key));
    }

    let url = http::url(&base, "/v2/check", &[])?;
    let reply = http.send(
        LANGUAGETOOL,
        &url,
        http.client().post(url.clone()).form(&form),
    )?;
    if !is_success(&reply) {
        return Err(status_error(&reply));
    }
    let r: CheckResponse = parse(&reply)?;
    let (language, detected_language) = match r.language {
        Some(l) => (
            Some(language(l.name, l.code)),
            l.detected_language.map(|d| language(d.name, d.code)),
        ),
        None => (None, None),
    };
    Ok(GrammarCheck {
        matches: r
            .matches
            .into_iter()
            .filter(|m| req.include_misspellings || m.rule.issue_type != "misspelling")
            .map(GrammarError::from)
            .collect(),
        language,
        detected_language,
    })
}

/// Codes are LanguageTool's `longCode` (`en-US`), which `/v2/check` takes as `language`.
pub(crate) fn languages(http: &Http, config: &LanguageToolConfig) -> Result<Vec<Language>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        name: String,
        long_code: String,
    }
    let base = config.base_url()?;
    let url = http::url(&base, "/v2/languages", &[])?;
    let reply = http.send(LANGUAGETOOL, &url, http.client().get(url.clone()))?;
    if !is_success(&reply) {
        return Err(status_error(&reply));
    }
    let entries: Vec<Entry> = parse(&reply)?;
    Ok(entries
        .into_iter()
        .map(|e| language(e.name, e.long_code))
        .collect())
}

pub(crate) fn test(http: &Http, config: &LanguageToolConfig) -> Result<TestReport> {
    let req = GrammarRequest {
        text: "Hello world.".into(),
        language: Some("en-US".into()),
        ..GrammarRequest::default()
    };
    check(http, config, &req)?;
    Ok(TestReport {
        host: host_of(&config.base_url()?),
        usage: None,
    })
}
