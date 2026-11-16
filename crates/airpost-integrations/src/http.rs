// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The blocking HTTP client and the error mapping every provider shares.
//!
//! Built as `airpost_net::HttpClient` is (reqwest blocking, rustls on ring, no redirects) but
//! without its `SyncError` mapping and engine dependencies. Each request logs one line with
//! the provider, host, status and duration; never the path or query, which can carry an
//! email address.

use std::time::{Duration, Instant};

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::RETRY_AFTER;
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{info, warn};

use crate::error::{IntegrationError, Result};

/// Longest provider message carried in an error.
const MAX_MESSAGE_CHARS: usize = 200;

pub(crate) struct Http {
    client: Client,
}

/// A response that arrived, with any status.
pub(crate) struct Reply {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: String,
}

impl Http {
    pub fn new(connect: Duration, total: Duration) -> Result<Self> {
        ensure_crypto_provider();
        let client = Client::builder()
            .connect_timeout(connect)
            .timeout(total)
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_rustls()
            .user_agent(concat!("Airpost/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| IntegrationError::Offline {
                reason: "HTTP client unavailable",
            })?;
        Ok(Http { client })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Sends `req` (built for `url`) and reads the whole body.
    pub fn send(&self, provider: &'static str, url: &Url, req: RequestBuilder) -> Result<Reply> {
        let host = url.host_str().unwrap_or("");
        let started = Instant::now();
        let result = req
            .send()
            .and_then(|resp| {
                let status = resp.status().as_u16();
                let retry_after = resp
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_retry_after);
                resp.text().map(|body| Reply {
                    status,
                    retry_after,
                    body,
                })
            })
            .map_err(|e| transport_error(&e));
        let ms = started.elapsed().as_millis();
        match &result {
            Ok(reply) => info!(
                "- integrations: {provider} {host} HTTP {} in {ms} ms",
                reply.status
            ),
            Err(e) => warn!("-X integrations: {provider} {host} failed after {ms} ms: {e}"),
        }
        result
    }
}

/// `base` + `path` + `query`, percent-encoded.
pub(crate) fn url(base: &str, path: &str, query: &[(&str, &str)]) -> Result<Url> {
    let mut url = Url::parse(&format!("{base}{path}"))
        .map_err(|_| IntegrationError::bad_request("the provider URL is not valid"))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url)
}

/// reqwest panics when building a rustls client with no process-wide provider. The engine
/// installs ring at startup; this covers tests and callers that run before it.
pub fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// Retry-After in delta-seconds. The HTTP-date form is not used by these providers.
fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// The failure class only: reqwest's own message includes the URL.
pub fn transport_error(err: &reqwest::Error) -> IntegrationError {
    if err.is_builder() {
        return IntegrationError::bad_request("the provider URL is not valid");
    }
    let reason = if err.is_timeout() {
        "timed out"
    } else if chain_has_tls_error(err) {
        "TLS handshake failed"
    } else if err.is_connect() {
        if chain_mentions(err, "dns error") {
            "host not found"
        } else {
            "connection refused"
        }
    } else {
        "connection lost"
    };
    IntegrationError::Offline { reason }
}

fn sources(err: &reqwest::Error) -> impl Iterator<Item = &(dyn std::error::Error + 'static)> {
    std::iter::successors(std::error::Error::source(err), |e| e.source())
}

fn chain_mentions(err: &reqwest::Error, needle: &str) -> bool {
    sources(err).any(|e| e.to_string().contains(needle))
}

/// hyper-rustls wraps the rustls error in nested `io::Error`s whose `source` skips the
/// wrapped error, so look inside each layer.
fn chain_has_tls_error(err: &reqwest::Error) -> bool {
    fn is_tls(e: &(dyn std::error::Error + 'static)) -> bool {
        if e.downcast_ref::<rustls::Error>().is_some() {
            return true;
        }
        match e
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
        {
            Some(inner) => is_tls(inner),
            None => false,
        }
    }
    sources(err).any(is_tls)
}

/// The success body as `T`; a 2xx with another shape is a provider error.
pub(crate) fn parse<T: DeserializeOwned>(reply: &Reply) -> Result<T> {
    serde_json::from_str(&reply.body).map_err(|_| IntegrationError::unreadable(reply.status))
}

pub(crate) fn is_success(reply: &Reply) -> bool {
    (200..300).contains(&reply.status)
}

/// The mapping for statuses a provider gives no special meaning.
pub(crate) fn status_error(reply: &Reply) -> IntegrationError {
    let status = reply.status;
    match status {
        401 | 403 => IntegrationError::Auth { status },
        402 => IntegrationError::QuotaExceeded { status },
        429 => IntegrationError::RateLimited {
            retry_after: reply.retry_after,
        },
        300..=399 => IntegrationError::BadRequest {
            status: Some(status),
            message: "the provider redirected the request; check the URL".into(),
        },
        400..=499 => IntegrationError::BadRequest {
            status: Some(status),
            message: provider_message(&reply.body),
        },
        _ => IntegrationError::Provider {
            status,
            message: provider_message(&reply.body),
        },
    }
}

/// The provider's own error text: `error`, `message` or `error.message` of a JSON body,
/// else the first line of a text one; truncated.
pub fn provider_message(body: &str) -> String {
    let from_json = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        [&v["error"], &v["message"], &v["error"]["message"]]
            .into_iter()
            .find_map(|m| m.as_str().map(str::to_owned))
    });
    let text = from_json.unwrap_or_else(|| body.lines().next().unwrap_or("").trim().to_owned());
    if text.is_empty() {
        return "no details".into();
    }
    truncate(&text, MAX_MESSAGE_CHARS)
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            retry_after: None,
            body: body.into(),
        }
    }

    #[test]
    fn messages() {
        assert_eq!(
            provider_message(r#"{"error":"Invalid API key"}"#),
            "Invalid API key"
        );
        assert_eq!(
            provider_message(r#"{"message":"Bad target"}"#),
            "Bad target"
        );
        assert_eq!(
            provider_message(r#"{"error":{"type":"x","message":"nope"}}"#),
            "nope"
        );
        assert_eq!(
            provider_message("Error: too long\nstack"),
            "Error: too long"
        );
        assert_eq!(provider_message(""), "no details");
        let long = "é".repeat(500);
        assert_eq!(
            provider_message(&long).chars().count(),
            MAX_MESSAGE_CHARS + 1
        );
    }

    #[test]
    fn default_status_mapping() {
        assert_eq!(status_error(&reply(401, "")).key(), "auth");
        assert_eq!(status_error(&reply(403, "")).key(), "auth");
        assert_eq!(status_error(&reply(402, "")).key(), "quota-exceeded");
        assert_eq!(status_error(&reply(429, "")).key(), "rate-limited");
        assert_eq!(status_error(&reply(302, "")).key(), "bad-request");
        assert_eq!(status_error(&reply(413, "")).key(), "bad-request");
        let e = status_error(&reply(503, "down"));
        assert_eq!(e.key(), "provider-error");
        assert!(e.is_retryable());
    }

    #[test]
    fn retry_after() {
        assert_eq!(parse_retry_after(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    }
}
