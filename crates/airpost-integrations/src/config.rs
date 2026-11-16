// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Provider settings, as the Integrations tab collects them. Deserialized from the
//! request's camelCase JSON; `Debug` redacts every key and username, and the userinfo of
//! any URL.

use std::fmt;

use serde::Deserialize;

use crate::error::{IntegrationError, Result};
use crate::secret::{filled, Secret};

pub const DEEPL_FREE_URL: &str = "https://api-free.deepl.com";
pub const DEEPL_PRO_URL: &str = "https://api.deepl.com";
pub const LANGUAGETOOL_URL: &str = "https://api.languagetool.org";
pub const LANGUAGETOOL_PREMIUM_URL: &str = "https://api.languagetoolplus.com";
pub const PEOPLE_DATA_LABS_URL: &str = "https://api.peopledatalabs.com";

/// A LibreTranslate server: libretranslate.com (key required) or self-hosted (key only if
/// the server was started with `--api-keys`).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibreTranslateConfig {
    pub url: String,
    #[serde(default)]
    pub api_key: Option<Secret>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepLConfig {
    pub api_key: Secret,
    /// Overrides the endpoint picked from the key (tests, proxies).
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageToolConfig {
    /// [`LANGUAGETOOL_URL`] when absent, null or blank.
    #[serde(default)]
    pub url: Option<String>,
    /// Premium account email; set together with `api_key` or not at all.
    #[serde(default)]
    pub username: Option<Secret>,
    #[serde(default)]
    pub api_key: Option<Secret>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleDataLabsConfig {
    pub api_key: Secret,
    /// Overrides [`PEOPLE_DATA_LABS_URL`] (tests).
    #[serde(default)]
    pub url: Option<String>,
}

/// The translation provider the user picked. JSON: `{"provider":"deepl","apiKey":"…"}`.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "provider")]
pub enum TranslationProvider {
    #[serde(rename = "libretranslate")]
    LibreTranslate(LibreTranslateConfig),
    #[serde(rename = "deepl")]
    DeepL(DeepLConfig),
}

/// Any provider, for [`crate::Integrations::test`].
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "provider")]
pub enum ProviderConfig {
    #[serde(rename = "libretranslate")]
    LibreTranslate(LibreTranslateConfig),
    #[serde(rename = "deepl")]
    DeepL(DeepLConfig),
    #[serde(rename = "languagetool")]
    LanguageTool(LanguageToolConfig),
    #[serde(rename = "peopledatalabs")]
    PeopleDataLabs(PeopleDataLabsConfig),
}

impl From<TranslationProvider> for ProviderConfig {
    fn from(p: TranslationProvider) -> Self {
        match p {
            TranslationProvider::LibreTranslate(c) => ProviderConfig::LibreTranslate(c),
            TranslationProvider::DeepL(c) => ProviderConfig::DeepL(c),
        }
    }
}

impl LibreTranslateConfig {
    pub fn base_url(&self) -> Result<String> {
        base_url(&self.url, "the LibreTranslate URL")
    }

    pub(crate) fn key(&self) -> Option<&str> {
        filled(&self.api_key)
    }
}

impl DeepLConfig {
    /// DeepL API Free keys end in `:fx` and only work on [`DEEPL_FREE_URL`]; every other
    /// key is a Pro key for [`DEEPL_PRO_URL`] (developers.deepl.com, "Authentication").
    pub fn base_url(&self) -> Result<String> {
        if let Some(url) = override_url(&self.url) {
            return base_url(url, "the DeepL URL");
        }
        let key = self.key()?;
        Ok(if key.ends_with(":fx") {
            DEEPL_FREE_URL
        } else {
            DEEPL_PRO_URL
        }
        .into())
    }

    pub(crate) fn key(&self) -> Result<&str> {
        required(&self.api_key, "the DeepL API key")
    }
}

impl LanguageToolConfig {
    /// Premium accounts are served by [`LANGUAGETOOL_PREMIUM_URL`] only, so credentials with
    /// the default free URL go there. A self-hosted URL is used as given. A trailing `/v2`
    /// is dropped, since users paste the documented endpoint.
    pub fn base_url(&self) -> Result<String> {
        let url = base_url(
            override_url(&self.url).unwrap_or(LANGUAGETOOL_URL),
            "the LanguageTool URL",
        )?;
        let url = url.strip_suffix("/v2").unwrap_or(&url).to_owned();
        if self.credentials()?.is_some() && url == LANGUAGETOOL_URL {
            return Ok(LANGUAGETOOL_PREMIUM_URL.into());
        }
        Ok(url)
    }

    /// Username and API key, or `None` for the free tier. Only one of the two is an error:
    /// sending neither would quietly check on the free tier instead.
    pub(crate) fn credentials(&self) -> Result<Option<(&str, &str)>> {
        match (filled(&self.username), filled(&self.api_key)) {
            (Some(user), Some(key)) => Ok(Some((user, key))),
            (None, None) => Ok(None),
            (Some(_), None) => Err(IntegrationError::NotConfigured {
                what: "the LanguageTool API key",
            }),
            (None, Some(_)) => Err(IntegrationError::NotConfigured {
                what: "the LanguageTool username",
            }),
        }
    }
}

impl PeopleDataLabsConfig {
    pub fn base_url(&self) -> Result<String> {
        base_url(
            override_url(&self.url).unwrap_or(PEOPLE_DATA_LABS_URL),
            "the People Data Labs URL",
        )
    }

    pub(crate) fn key(&self) -> Result<&str> {
        required(&self.api_key, "the People Data Labs API key")
    }
}

/// An optional URL setting; blank counts as absent.
fn override_url(url: &Option<String>) -> Option<&str> {
    url.as_deref().filter(|u| !u.trim().is_empty())
}

fn required<'a>(secret: &'a Secret, what: &'static str) -> Result<&'a str> {
    if secret.is_blank() {
        return Err(IntegrationError::NotConfigured { what });
    }
    Ok(secret.expose().trim())
}

/// `url` trimmed, without trailing slashes, and with an http or https scheme.
fn base_url(url: &str, what: &'static str) -> Result<String> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err(IntegrationError::NotConfigured { what });
    }
    match reqwest::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => {
            Ok(url.to_owned())
        }
        _ => Err(IntegrationError::bad_request(format!(
            "{what} is not a valid http(s) URL"
        ))),
    }
}

/// A URL setting for `Debug`: userinfo (a proxy's basic auth) becomes `***`, and anything
/// but an http(s) URL is not shown, since it may be credentials pasted without a scheme.
pub struct RedactedUrl<'a>(pub &'a str);

impl fmt::Debug for RedactedUrl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `alice:hunter2@host` parses with scheme `alice`, so only http(s) URLs are shown.
        match reqwest::Url::parse(self.0.trim()) {
            Ok(mut u) if matches!(u.scheme(), "http" | "https") => {
                if !u.username().is_empty() || u.password().is_some() {
                    let _ = u.set_password(None);
                    let _ = u.set_username("***");
                }
                write!(f, "{:?}", u.as_str())
            }
            _ if self.0.trim().is_empty() => f.write_str("\"\""),
            _ => f.write_str("<unparsed URL>"),
        }
    }
}

fn redacted(url: &Option<String>) -> Option<RedactedUrl<'_>> {
    url.as_deref().map(RedactedUrl)
}

impl fmt::Debug for LibreTranslateConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibreTranslateConfig")
            .field("url", &RedactedUrl(&self.url))
            .field("api_key", &self.api_key)
            .finish()
    }
}

impl fmt::Debug for DeepLConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeepLConfig")
            .field("api_key", &self.api_key)
            .field("url", &redacted(&self.url))
            .finish()
    }
}

impl fmt::Debug for LanguageToolConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LanguageToolConfig")
            .field("url", &redacted(&self.url))
            .field("username", &self.username)
            .field("api_key", &self.api_key)
            .finish()
    }
}

impl fmt::Debug for PeopleDataLabsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeopleDataLabsConfig")
            .field("api_key", &self.api_key)
            .field("url", &redacted(&self.url))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_urls() {
        let show = |u: &str| format!("{:?}", RedactedUrl(u));
        assert_eq!(
            show("https://alice:hunter2@lt.example.org/x"),
            "\"https://***@lt.example.org/x\""
        );
        assert_eq!(
            show("https://lt.example.org"),
            "\"https://lt.example.org/\""
        );
        assert_eq!(show("alice:hunter2@lt.example.org"), "<unparsed URL>");
        assert_eq!(show(" "), "\"\"");
    }
}
