// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Third-party providers the desktop's local features call: translation (LibreTranslate,
//! DeepL), grammar checks (LanguageTool) and contact and company enrichment (People Data
//! Labs).
//!
//! Transport-free: the engine exposes [`Integrations`] as the `integrations.*` socket
//! methods. Every call carries its provider config (URL, key) because the desktop owns the
//! credentials until the engine does; nothing here persists them.
//!
//! Everything is blocking (reqwest's blocking client runs its own runtime), so call it from
//! an engine thread and never from inside a tokio runtime.
//!
//! Privacy: logs carry the provider, host, status and timing only. Keys, usernames, message
//! text and email addresses are never logged, never part of a cache key in clear, and
//! redacted from `Debug` output.

mod cache;
mod config;
mod enrich;
mod error;
mod grammar;
mod http;
mod secret;
mod translate;

use std::time::Duration;

pub use config::{
    DeepLConfig, LanguageToolConfig, LibreTranslateConfig, PeopleDataLabsConfig, ProviderConfig,
    RedactedUrl, TranslationProvider, DEEPL_FREE_URL, DEEPL_PRO_URL, LANGUAGETOOL_PREMIUM_URL,
    LANGUAGETOOL_URL, PEOPLE_DATA_LABS_URL,
};
pub use enrich::{
    Company, CompanyCategory, CompanyMetrics, ContactProfile, Employment, Handle, Person,
};
pub use error::{IntegrationError, Result};
pub use grammar::{GrammarCheck, GrammarError, GrammarRequest};
pub use secret::Secret;
pub use translate::{DetectedLanguage, TextFormat, TranslateRequest, Translation};

/// Shared with `airpost-ai`, which calls model providers on the same HTTP stack and caches the
/// same way.
pub use cache::{CacheKey, KeyBuilder, TtlCache};
pub use http::{ensure_crypto_provider, provider_message, transport_error};

use http::Http;
use serde::Serialize;

/// Timeouts and cache bounds. [`Options::default`] is what the engine uses.
#[derive(Clone, Debug)]
pub struct Options {
    pub connect_timeout: Duration,
    /// Whole-request limit. Translating a long HTML body on a small self-hosted
    /// LibreTranslate takes several seconds.
    pub request_timeout: Duration,
    pub translation_ttl: Duration,
    pub max_cached_translations: usize,
    /// For a person or company that was found.
    pub enrichment_ttl: Duration,
    /// For a lookup that found nothing. Shorter, because provider data fills in over time.
    pub negative_enrichment_ttl: Duration,
    /// Per kind (people, companies).
    pub max_cached_profiles: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            translation_ttl: Duration::from_secs(24 * 3600),
            max_cached_translations: 1000,
            enrichment_ttl: Duration::from_secs(7 * 24 * 3600),
            negative_enrichment_ttl: Duration::from_secs(24 * 3600),
            max_cached_profiles: 500,
        }
    }
}

/// A language a provider supports. `targets` lists what it translates into (LibreTranslate
/// only; empty elsewhere).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Language {
    pub code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
}

/// What [`Integrations::test`] learned besides "the URL and key work".
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    /// The host that answered, for the Integrations tab ("Connected to api-free.deepl.com").
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// Characters used in the billing period (DeepL).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// The provider clients and their caches. One per engine process; `Send + Sync`, so
/// connection threads share it.
pub struct Integrations {
    http: Http,
    options: Options,
    translations: TtlCache<Translation>,
    people: TtlCache<Option<enrich::PersonMatch>>,
    companies: TtlCache<Option<Company>>,
}

impl std::fmt::Debug for Integrations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Integrations")
            .field("options", &self.options)
            .field("cached_translations", &self.translations.len())
            .field("cached_people", &self.people.len())
            .field("cached_companies", &self.companies.len())
            .finish()
    }
}

impl Integrations {
    pub fn new() -> Result<Self> {
        Integrations::with_options(Options::default())
    }

    pub fn with_options(options: Options) -> Result<Self> {
        Ok(Integrations {
            http: Http::new(options.connect_timeout, options.request_timeout)?,
            translations: TtlCache::new(options.max_cached_translations),
            people: TtlCache::new(options.max_cached_profiles),
            companies: TtlCache::new(options.max_cached_profiles),
            options,
        })
    }

    /// Drops every cached translation and profile.
    pub fn clear_cache(&self) {
        self.translations.clear();
        self.people.clear();
        self.companies.clear();
    }

    /// Translates `request.text`, from the cache when the same provider, key and request
    /// were translated within [`Options::translation_ttl`].
    pub fn translate(
        &self,
        provider: &TranslationProvider,
        request: &TranslateRequest,
    ) -> Result<Translation> {
        translate::translate(self, provider, request)
    }

    /// Languages the provider translates between.
    pub fn translation_languages(&self, provider: &TranslationProvider) -> Result<Vec<Language>> {
        translate::languages(&self.http, provider)
    }

    /// The likely languages of `text`, best first. LibreTranslate only: DeepL detects only
    /// as part of a translation, which it bills.
    pub fn detect_language(
        &self,
        provider: &TranslationProvider,
        text: &str,
    ) -> Result<Vec<DetectedLanguage>> {
        translate::detect(&self.http, provider, text)
    }

    /// Grammar and style problems in `request.text`. Not cached: a draft changes between
    /// checks.
    pub fn check_grammar(
        &self,
        config: &LanguageToolConfig,
        request: &GrammarRequest,
    ) -> Result<GrammarCheck> {
        grammar::check(&self.http, config, request)
    }

    pub fn grammar_languages(&self, config: &LanguageToolConfig) -> Result<Vec<Language>> {
        grammar::languages(&self.http, config)
    }

    /// The person with this email address, or `None` when the provider has no match.
    pub fn enrich_person(
        &self,
        config: &PeopleDataLabsConfig,
        email: &str,
    ) -> Result<Option<Person>> {
        Ok(enrich::person(self, config, email)?.map(|m| m.person))
    }

    /// The company at this domain (or URL), or `None` when the provider has no match.
    pub fn enrich_company(
        &self,
        config: &PeopleDataLabsConfig,
        domain: &str,
    ) -> Result<Option<Company>> {
        enrich::company(self, config, domain)
    }

    /// The sidebar's profile for `email`: the person, and their company from the person's
    /// employer website or, failing that, the address's domain unless it is a webmail one.
    pub fn enrich_contact(
        &self,
        config: &PeopleDataLabsConfig,
        email: &str,
    ) -> Result<ContactProfile> {
        enrich::contact(self, config, email)
    }

    /// The cheapest call that proves the URL and credentials work:
    ///
    /// - LibreTranslate: `POST /detect` of a five-letter word. `/languages` needs no key, so
    ///   it would not prove one.
    /// - DeepL: `GET /v2/usage`, which bills nothing and reports the period's usage.
    /// - LanguageTool: `POST /v2/check` of one short sentence (`/v2/languages` ignores the
    ///   premium credentials).
    /// - People Data Labs: `GET /v5/autocomplete`, free for any active key; enrichment
    ///   calls bill per match. Anything but a 200 fails.
    pub fn test(&self, config: &ProviderConfig) -> Result<TestReport> {
        match config {
            ProviderConfig::LibreTranslate(c) => translate::test_libretranslate(&self.http, c),
            ProviderConfig::DeepL(c) => translate::test_deepl(&self.http, c),
            ProviderConfig::LanguageTool(c) => grammar::test(&self.http, c),
            ProviderConfig::PeopleDataLabs(c) => enrich::test(&self.http, c),
        }
    }
}
