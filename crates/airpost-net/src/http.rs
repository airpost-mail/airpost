// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! JSON-over-HTTP requests with the engine's error mapping. Derived from
//! `MailSync/NetworkRequestUtils.cpp` (Copyright © 2017 Foundry 376, GPL-3.0).
//!
//! - A transport failure becomes a [`SyncError`] keyed by the libcurl message for the
//!   equivalent `CURLcode`, with the URL as debuginfo. Resolve, connect, timeout, TLS and
//!   transfer failures are retryable and offline (`SyncException(CURLcode, di)`).
//! - A status outside 200-209 becomes `Invalid Response Code: <status>` with
//!   `"<url> RETURNED <body>"` as debuginfo. It is retryable unless the status is 401 or 403
//!   or the body contains `invalid_grant` (`ValidateRequestResp`).
//! - A body that is not JSON is returned as `{"text": <body>}` (`PerformJSONRequest`).
//!
//! Redirects are never followed, as with curl's defaults; [`HttpClient::expected_redirect`]
//! reads the target of one instead.

use std::time::Duration;

use airpost_core::{Error, Result, SyncError};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, LOCATION};
use reqwest::{Method, StatusCode, Url};
use serde_json::{json, Value};

/// `CURLOPT_CONNECTTIMEOUT` for API requests.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// `CURLOPT_CONNECTTIMEOUT` for `.well-known` redirect probes.
pub const REDIRECT_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// curl had no overall limit. This one only stops a stalled server from holding a worker
/// thread forever; it is far above any People API or token request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

// libcurl's `curl_easy_strerror` texts (curl 8), which the C++ engine used as error keys.
const CURL_UNSUPPORTED_PROTOCOL: &str = "Unsupported protocol";
const CURL_URL_MALFORMAT: &str = "URL using bad/illegal format or missing URL";
const CURL_COULDNT_RESOLVE_HOST: &str = "Could not resolve hostname";
const CURL_COULDNT_CONNECT: &str = "Could not connect to server";
const CURL_OPERATION_TIMEDOUT: &str = "Timeout was reached";
const CURL_SSL_CONNECT_ERROR: &str = "SSL connect error";
const CURL_PEER_FAILED_VERIFICATION: &str = "SSL peer certificate or SSH remote key was not OK";
const CURL_TOO_MANY_REDIRECTS: &str = "Number of redirects hit maximum amount";
const CURL_GOT_NOTHING: &str = "Server returned nothing (no headers, no data)";
const CURL_RECV_ERROR: &str = "Failure when receiving data from the peer";
const CURL_SEND_ERROR: &str = "Failed sending data to the peer";

/// A blocking HTTP client. Never call it from inside a tokio runtime (reqwest's blocking
/// client runs its own).
#[derive(Clone, Debug)]
pub struct HttpClient {
    client: Client,
}

impl HttpClient {
    pub fn new() -> Result<Self> {
        HttpClient::with_timeouts(CONNECT_TIMEOUT, REQUEST_TIMEOUT)
    }

    pub fn with_timeouts(connect: Duration, total: Duration) -> Result<Self> {
        ensure_crypto_provider();
        let client = Client::builder()
            .connect_timeout(connect)
            .timeout(total)
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_rustls()
            .build()
            .map_err(|e| Error::other(format!("Could not create the HTTP client: {e}")))?;
        Ok(HttpClient { client })
    }

    /// For protocol code (DAV) that needs requests this module doesn't shape. Pair it with
    /// [`transport_error`] and [`check_status`] to keep the error mapping.
    pub fn inner(&self) -> &Client {
        &self.client
    }

    /// A JSON request authorised with a bearer token (`CreateJSONRequest` +
    /// `PerformJSONRequest`). A status outside 200-209 is an error.
    pub fn json(
        &self,
        method: &str,
        url: &str,
        bearer: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value> {
        let authorization = bearer.map(|t| format!("Bearer {t}"));
        self.json_with_authorization(method, url, authorization.as_deref(), body)
    }

    /// As [`HttpClient::json`], with the whole `Authorization` header value (`Basic …`,
    /// `Bearer …`).
    pub fn json_with_authorization(
        &self,
        method: &str,
        url: &str,
        authorization: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|_| Error::other(format!("Invalid HTTP method: {method}")))?;
        let mut req = self
            .client
            .request(method, url)
            .header(ACCEPT, "application/json");
        if let Some(a) = authorization.filter(|a| !a.is_empty()) {
            req = req.header(AUTHORIZATION, a);
        }
        if let Some(body) = body {
            req = req
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        perform_json(req, url)
    }

    /// A form-encoded POST returning JSON, as the OAuth token endpoints expect.
    pub fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<Value> {
        let req = self
            .client
            .post(url)
            .header(ACCEPT, "application/json")
            .form(form);
        perform_json(req, url)
    }

    /// `PerformExpectedRedirect`: GETs `url` and returns the absolute target of a 301, 302,
    /// 303, 307 or 308, or `""` for any other answer or failure. RFC 6764 §5 requires
    /// clients to follow redirects on `.well-known`; 308 postdates the RFC.
    pub fn expected_redirect(&self, url: &str) -> String {
        let Ok(resp) = self.client.get(url).timeout(REDIRECT_PROBE_TIMEOUT).send() else {
            return String::new();
        };
        let is_redirect = matches!(resp.status().as_u16(), 301 | 302 | 303 | 307 | 308);
        if !is_redirect {
            return String::new();
        }
        resp.headers()
            .get(LOCATION)
            .and_then(|l| l.to_str().ok())
            .and_then(|l| resp.url().join(l).ok())
            .map(String::from)
            .unwrap_or_default()
    }

    /// The `install-check` HTTP probe: any HTTP answer means TLS and the network work, so
    /// this returns its status; a transport failure returns the reqwest error.
    pub fn probe(&self, url: &str) -> std::result::Result<u16, reqwest::Error> {
        self.client
            .get(url)
            .header(ACCEPT, "application/json")
            .send()
            .map(|resp| resp.status().as_u16())
    }
}

/// reqwest panics when building a rustls client with no process-wide provider. The bin
/// installs ring at startup; this covers tests and any caller that runs before it.
pub(crate) fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

fn perform_json(req: RequestBuilder, url: &str) -> Result<Value> {
    let resp = req.send().map_err(|e| transport_error(&e, url))?;
    let (effective_url, status, body) = read(resp, url)?;
    check_status(status.as_u16(), &effective_url, &body)?;
    Ok(parse_json_body(&body))
}

fn read(resp: Response, url: &str) -> Result<(String, StatusCode, String)> {
    let effective_url = resp.url().to_string();
    let status = resp.status();
    let body = resp.text().map_err(|e| transport_error(&e, url))?;
    Ok((effective_url, status, body))
}

/// `ValidateRequestResp` for a response that arrived.
pub fn check_status(status: u16, url: &str, body: &str) -> Result<()> {
    if (200..=209).contains(&status) {
        return Ok(());
    }
    let retryable = status != 401 && status != 403 && !body.contains("invalid_grant");
    Err(SyncError::new(
        format!("Invalid Response Code: {status}"),
        format!("{url} RETURNED {body}"),
        retryable,
        false,
    )
    .into())
}

/// `PerformJSONRequest`'s body handling: JSON if it parses, otherwise `{"text": body}`.
pub fn parse_json_body(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| json!({ "text": body }))
}

/// `SyncException(CURLcode, url)` for a request that got no usable response.
pub fn transport_error(err: &reqwest::Error, url: &str) -> SyncError {
    let key = transport_error_key(err);
    let offline = key != CURL_URL_MALFORMAT && key != CURL_UNSUPPORTED_PROTOCOL;
    SyncError::new(key, url, offline, offline)
}

fn transport_error_key(err: &reqwest::Error) -> &'static str {
    if err.is_builder() {
        return match err.url().map(Url::scheme) {
            Some(s) if s != "http" && s != "https" => CURL_UNSUPPORTED_PROTOCOL,
            _ => CURL_URL_MALFORMAT,
        };
    }
    if err.is_timeout() {
        return CURL_OPERATION_TIMEDOUT;
    }
    if err.is_redirect() {
        return CURL_TOO_MANY_REDIRECTS;
    }
    if let Some(tls) = find_source::<rustls::Error>(err) {
        return match tls {
            rustls::Error::InvalidCertificate(_) => CURL_PEER_FAILED_VERIFICATION,
            _ => CURL_SSL_CONNECT_ERROR,
        };
    }
    if err.is_connect() {
        return if chain_mentions(err, "dns error") {
            CURL_COULDNT_RESOLVE_HOST
        } else {
            CURL_COULDNT_CONNECT
        };
    }
    if err.is_body() || err.is_decode() {
        return CURL_RECV_ERROR;
    }
    if chain_mentions(err, "connection closed before message completed")
        || chain_mentions(err, "IncompleteMessage")
    {
        return CURL_GOT_NOTHING;
    }
    if err.is_request() {
        return CURL_SEND_ERROR;
    }
    CURL_RECV_ERROR
}

fn sources(err: &reqwest::Error) -> impl Iterator<Item = &(dyn std::error::Error + 'static)> {
    std::iter::successors(std::error::Error::source(err), |e| e.source())
}

fn find_source<T: std::error::Error + 'static>(err: &reqwest::Error) -> Option<&T> {
    sources(err).find_map(unwrap_io::<T>)
}

/// hyper-rustls wraps the rustls error in nested `io::Error`s, and `io::Error::source`
/// skips the wrapped error itself, so look inside each layer.
fn unwrap_io<'a, T: std::error::Error + 'static>(
    e: &'a (dyn std::error::Error + 'static),
) -> Option<&'a T> {
    if let Some(t) = e.downcast_ref::<T>() {
        return Some(t);
    }
    let inner = e.downcast_ref::<std::io::Error>()?.get_ref()?;
    unwrap_io::<T>(inner)
}

fn chain_mentions(err: &reqwest::Error, needle: &str) -> bool {
    sources(err).any(|e| e.to_string().contains(needle) || format!("{e:?}").contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_policy() {
        assert!(check_status(200, "u", "").is_ok());
        assert!(check_status(209, "u", "").is_ok());

        let e = check_status(500, "https://x/y", "oops").unwrap_err();
        let s = e.as_sync().unwrap();
        assert_eq!(s.key, "Invalid Response Code: 500");
        assert_eq!(s.debuginfo, "https://x/y RETURNED oops");
        assert!(s.retryable && !s.offline);

        for status in [401, 403] {
            assert!(!check_status(status, "u", "").unwrap_err().is_retryable());
        }
        let grant = check_status(400, "u", r#"{"error":"invalid_grant"}"#).unwrap_err();
        assert!(!grant.is_retryable());
        assert!(check_status(400, "u", r#"{"error":"invalid_request"}"#)
            .unwrap_err()
            .is_retryable());
        assert!(check_status(302, "u", "").is_err());
    }

    #[test]
    fn body_parsing() {
        assert_eq!(parse_json_body(r#"{"a":1}"#), json!({"a": 1}));
        assert_eq!(parse_json_body("<html>"), json!({"text": "<html>"}));
        assert_eq!(parse_json_body(""), json!({"text": ""}));
    }
}
