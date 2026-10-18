// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! [`ProviderConfig`]: one configured model provider, as Preferences > AI collects it.
//! Deserialized from the request's camelCase JSON; `Debug` redacts the key and the userinfo
//! of the URL.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ms_integrations::{RedactedUrl, Secret};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::error::{AiError, Result};

pub const OPENAI_URL: &str = "https://api.openai.com/v1";
pub const ANTHROPIC_URL: &str = "https://api.anthropic.com";
pub const OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Ollama's `num_ctx` when `contextTokens` is unset. Ollama's own default window is
/// smaller and it truncates silently past it (docs/ai-plan.md A2).
pub const OLLAMA_CONTEXT_TOKENS: u32 = 8192;
/// The input budget for cloud models when `contextTokens` is unset. Their windows are far
/// larger; this bounds what one request costs.
pub const CLOUD_CONTEXT_TOKENS: u32 = 16384;
/// The default output limits, lowered to a quarter of `contextTokens` when that is small.
pub const OLLAMA_MAX_OUTPUT_TOKENS: u32 = 1024;
/// Higher than the answers need: reasoning models spend output tokens thinking, and only
/// generated tokens are billed.
pub const CLOUD_MAX_OUTPUT_TOKENS: u32 = 4096;

/// The wire format a provider speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// `/chat/completions` under a versioned base URL: OpenAI, OpenRouter, Groq, Mistral,
    /// Gemini's compatible endpoint, LM Studio, llama.cpp, vLLM.
    #[serde(rename = "openai")]
    OpenAi,
    /// `/v1/messages`.
    Anthropic,
    /// Ollama's native `/api/chat`, which honours `num_ctx`.
    Ollama,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Ollama => "ollama",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => OPENAI_URL,
            ProviderKind::Anthropic => ANTHROPIC_URL,
            ProviderKind::Ollama => OLLAMA_URL,
        }
    }
}

/// JSON: `{"id":"work","kind":"anthropic","apiKey":"…","model":"claude-haiku-5-5"}`.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// The user's name for this provider entry; part of cache keys and usage records.
    pub id: String,
    pub kind: ProviderKind,
    /// [`ProviderKind::default_url`] when absent or blank.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub api_key: Option<Secret>,
    /// Required for every call but [`crate::Ai::models`].
    #[serde(default)]
    pub model: String,
    /// The model's context window as the budget sees it; `num_ctx` on Ollama.
    #[serde(default)]
    pub context_tokens: Option<u32>,
    /// The output limit of every request; defaults per kind.
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

/// A validated config, ready for a request.
#[derive(Clone)]
pub(crate) struct Target {
    pub kind: ProviderKind,
    pub base: String,
    pub host: String,
    key: Option<String>,
    pub model: String,
    pub context_tokens: u32,
    pub max_output_tokens: u32,
}

impl Target {
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }
}

impl ProviderConfig {
    /// The base URL requests are built on: trimmed, without trailing slashes and without
    /// a path suffix users paste from provider docs (`/chat/completions`, `/v1`, `/api`).
    pub fn base_url(&self) -> Result<String> {
        let raw = self
            .url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or(self.kind.default_url());
        let mut url = raw.trim_end_matches('/');
        let suffixes: &[&str] = match self.kind {
            ProviderKind::OpenAi => &["/chat/completions"],
            ProviderKind::Anthropic => &["/v1/messages", "/v1"],
            ProviderKind::Ollama => &["/api/chat", "/api", "/v1"],
        };
        for suffix in suffixes {
            if let Some(stripped) = url.strip_suffix(suffix) {
                url = stripped.trim_end_matches('/');
                break;
            }
        }
        match Url::parse(url) {
            Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => {
                Ok(url.to_owned())
            }
            _ => Err(AiError::bad_request(
                "the provider URL is not a valid http(s) URL",
            )),
        }
    }

    /// The host requests go to, for "Sent to api.anthropic.com". Empty when the URL is
    /// invalid.
    pub fn host(&self) -> String {
        self.base_url()
            .ok()
            .and_then(|u| Url::parse(&u).ok())
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// Whether requests stay on this computer or the local network: a loopback or private
    /// address, or a `localhost` name. Drives the privacy note.
    pub fn is_local(&self) -> bool {
        self.base_url()
            .ok()
            .and_then(|u| Url::parse(&u).ok())
            .is_some_and(|u| is_local_url(&u))
    }

    pub fn context_tokens(&self) -> u32 {
        self.context_tokens
            .filter(|&n| n > 0)
            .unwrap_or(match self.kind {
                ProviderKind::Ollama => OLLAMA_CONTEXT_TOKENS,
                _ => CLOUD_CONTEXT_TOKENS,
            })
    }

    /// `maxOutputTokens`, else the kind's default capped at a quarter of the window, so a
    /// small local window keeps room for the thread. Each request lowers it further when
    /// the prompt needs the room.
    pub fn max_output_tokens(&self) -> u32 {
        self.max_output_tokens
            .filter(|&n| n > 0)
            .unwrap_or_else(|| {
                let default = match self.kind {
                    ProviderKind::Ollama => OLLAMA_MAX_OUTPUT_TOKENS,
                    _ => CLOUD_MAX_OUTPUT_TOKENS,
                };
                default.min(self.context_tokens() / 4)
            })
    }

    /// Checks the settings a call needs. Anthropic always needs a key, OpenAI's own API
    /// does (a compatible server may not), Ollama doesn't. Plain http is only for a server
    /// on this computer or network: mail and keys go nowhere else in clear text.
    pub(crate) fn target(&self, need_model: bool) -> Result<Target> {
        let base = self.base_url()?;
        let url =
            Url::parse(&base).map_err(|_| AiError::bad_request("the provider URL is not valid"))?;
        if url.scheme() == "http" && !is_local_url(&url) && !is_private_name(&url) {
            return Err(AiError::bad_request(
                "use https for a provider that isn't on this computer or network",
            ));
        }
        let key = self
            .api_key
            .as_ref()
            .filter(|k| !k.is_blank())
            .map(|k| k.expose().trim().to_owned());
        let key_required = match self.kind {
            ProviderKind::Anthropic => true,
            ProviderKind::OpenAi => base == OPENAI_URL,
            ProviderKind::Ollama => false,
        };
        if key_required && key.is_none() {
            return Err(AiError::NotConfigured {
                what: "the API key",
            });
        }
        let model = self.model.trim().to_owned();
        if need_model && model.is_empty() {
            return Err(AiError::NotConfigured { what: "the model" });
        }
        Ok(Target {
            kind: self.kind,
            host: self.host(),
            base,
            key,
            model,
            context_tokens: self.context_tokens(),
            max_output_tokens: self.max_output_tokens(),
        })
    }
}

fn is_local_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_local_ip(ip);
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost")
}

/// A name that public DNS never resolves: one label (`gpu`), or under `.local` (mDNS),
/// `.lan`, `.home.arpa` or `.internal`. Such a host is on the user's own network, so plain
/// http is allowed; [`ProviderConfig::is_local`] still doesn't claim it.
fn is_private_name(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.starts_with('[') || host.parse::<IpAddr>().is_ok() {
        return false;
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    !name.contains('.')
        || [".local", ".lan", ".home.arpa", ".internal"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_v4(v4);
            }
            let first = v6.segments()[0];
            v6 == Ipv6Addr::LOCALHOST
                // fc00::/7 unique local, fe80::/10 link-local.
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("url", &self.url.as_deref().map(RedactedUrl))
            .field("api_key", &self.api_key)
            .field("model", &self.model)
            .field("context_tokens", &self.context_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(kind: ProviderKind, url: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            id: "p".into(),
            kind,
            url: url.map(str::to_owned),
            api_key: None,
            model: "m".into(),
            context_tokens: None,
            max_output_tokens: None,
        }
    }

    #[test]
    fn base_urls() {
        let base = |kind, url| config(kind, url).base_url().unwrap();
        assert_eq!(base(ProviderKind::OpenAi, None), OPENAI_URL);
        assert_eq!(base(ProviderKind::OpenAi, Some(" ")), OPENAI_URL);
        assert_eq!(
            base(
                ProviderKind::OpenAi,
                Some("https://openrouter.ai/api/v1/chat/completions/")
            ),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            base(
                ProviderKind::Anthropic,
                Some("https://api.anthropic.com/v1/")
            ),
            ANTHROPIC_URL
        );
        assert_eq!(
            base(ProviderKind::Ollama, Some("http://gpu.lan:11434/api")),
            "http://gpu.lan:11434"
        );
        assert_eq!(
            config(ProviderKind::OpenAi, Some("api.openai.com"))
                .base_url()
                .unwrap_err()
                .key(),
            "bad-request"
        );
    }

    #[test]
    fn local_hosts() {
        let local = |url: &str| config(ProviderKind::OpenAi, Some(url)).is_local();
        assert!(config(ProviderKind::Ollama, None).is_local());
        assert!(local("http://localhost:1234/v1"));
        assert!(local("http://10.0.0.7:8000/v1"));
        assert!(local("http://192.168.1.20/v1"));
        assert!(local("http://172.20.0.1/v1"));
        assert!(local("http://[::1]:8080/v1"));
        assert!(local("http://[fd12::1]/v1"));
        assert!(!local("https://api.openai.com/v1"));
        assert!(!local("http://172.32.0.1/v1"));
        assert!(!local("http://8.8.8.8/v1"));
        assert!(!local("http://ollama.lan/v1"));
    }

    #[test]
    fn required_settings() {
        let err = |c: ProviderConfig| c.target(true).err().map(|e| e.to_string());
        assert_eq!(
            err(config(ProviderKind::Anthropic, None)).as_deref(),
            Some("the API key is not set")
        );
        assert_eq!(
            err(config(ProviderKind::OpenAi, None)).as_deref(),
            Some("the API key is not set")
        );
        assert_eq!(
            err(config(
                ProviderKind::OpenAi,
                Some("http://localhost:1234/v1")
            )),
            None
        );
        let mut no_model = config(ProviderKind::Ollama, None);
        no_model.model = " ".into();
        assert_eq!(
            err(no_model.clone()).as_deref(),
            Some("the model is not set")
        );
        assert!(no_model.target(false).is_ok());
    }

    #[test]
    fn plain_http_only_nearby() {
        let check = |url: &str| {
            let mut c = config(ProviderKind::OpenAi, Some(url));
            c.api_key = Some("sk-secret".into());
            c.target(true).err().map(|e| e.key())
        };
        for remote in [
            "http://8.8.8.8/v1",
            "http://api.anthropic.com",
            "http://llm.example.org/v1",
            "http://[2001:4860::8888]/v1",
        ] {
            assert_eq!(check(remote), Some("bad-request"), "{remote}");
        }
        for nearby in [
            "https://llm.example.org/v1",
            "http://127.0.0.1:1234/v1",
            "http://localhost:8000/v1",
            "http://192.168.1.20/v1",
            "http://[fd12::1]/v1",
            "http://gpu:8000/v1",
            "http://gpu.local:8000/v1",
            "http://ollama.lan/v1",
            "http://box.home.arpa/v1",
        ] {
            assert_eq!(check(nearby), None, "{nearby}");
        }
        let e = config(ProviderKind::Anthropic, Some("http://api.anthropic.com"))
            .target(false)
            .err()
            .unwrap();
        assert!(e.to_string().contains("use https"), "{e}");
    }

    #[test]
    fn defaults_per_kind() {
        let ollama = config(ProviderKind::Ollama, None);
        assert_eq!(ollama.context_tokens(), OLLAMA_CONTEXT_TOKENS);
        assert_eq!(ollama.max_output_tokens(), OLLAMA_MAX_OUTPUT_TOKENS);
        let mut cloud = config(ProviderKind::Anthropic, None);
        assert_eq!(cloud.context_tokens(), CLOUD_CONTEXT_TOKENS);
        cloud.context_tokens = Some(0);
        assert_eq!(cloud.context_tokens(), CLOUD_CONTEXT_TOKENS);
        cloud.context_tokens = Some(200_000);
        assert_eq!(cloud.context_tokens(), 200_000);
        assert_eq!(cloud.max_output_tokens(), CLOUD_MAX_OUTPUT_TOKENS);
        // A small window keeps three quarters for the thread.
        cloud.context_tokens = Some(4096);
        assert_eq!(cloud.max_output_tokens(), 1024);
        cloud.max_output_tokens = Some(3000);
        assert_eq!(cloud.max_output_tokens(), 3000);
    }

    #[test]
    fn deserializes_and_redacts() {
        let c: ProviderConfig = serde_json::from_value(serde_json::json!({
            "id": "home",
            "kind": "openai",
            "url": "https://alice:hunter2@llm.example.org/v1",
            "apiKey": "sk-secret",
            "model": "gpt-x",
            "contextTokens": 32000,
        }))
        .unwrap();
        assert_eq!(c.kind, ProviderKind::OpenAi);
        assert_eq!(c.context_tokens(), 32000);
        let debug = format!("{c:?}");
        for secret in ["sk-secret", "alice", "hunter2"] {
            assert!(!debug.contains(secret), "{debug}");
        }
    }
}
