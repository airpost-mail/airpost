// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! OAuth access tokens for Google and Microsoft accounts. Derived from
//! `MailSync/XOAuth2TokenManager.cpp` and `MakeOAuthRefreshRequest` in
//! `MailSync/NetworkRequestUtils.cpp` (Copyright © 2017 Foundry 376, GPL-3.0).
//!
//! Tokens are cached per account id and refreshed once fewer than
//! [`OAUTH_EXPIRY_MARGIN`] seconds remain. `expires_in` may be a number, a string or missing
//! (RFC 6749 §5.1 makes it optional). When the provider rotates the refresh token,
//! `on_rotate` is called with the updated account so the bin can emit
//! `ProcessAccountSecretsUpdated`; the rotated token is kept here too, because the workers'
//! `Account` is shared and immutable, and later refreshes in this process must use it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use ms_core::constants::{OAUTH_ASSUMED_EXPIRES_IN, OAUTH_EXPIRY_MARGIN};
use ms_core::{Account, Clock, Result, SyncError};
use ms_providers::OAuthProvider;
use serde_json::Value;
use tracing::info;

use crate::{Endpoints, HttpClient};

/// `XOAuth2Parts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessToken {
    pub username: String,
    pub access_token: String,
    /// Epoch seconds.
    pub expires_at: i64,
}

pub type RotateCallback = Box<dyn Fn(&Account) + Send + Sync>;

/// The env var holding the OAuth client id that `GMAIL_CLIENT_SECRET` belongs to.
pub const ENV_GMAIL_CLIENT_ID: &str = "GMAIL_CLIENT_ID";
pub const ENV_GMAIL_CLIENT_SECRET: &str = "GMAIL_CLIENT_SECRET";

/// Microsoft issues one access token per resource, so the refresh asks for the Outlook
/// IMAP and SMTP scopes explicitly; Graph (contacts, calendars) would need its own token.
pub const MICROSOFT_MAIL_SCOPES: &str =
    "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send";

const ERROR_KEY: &str = "invalid-xoauth2-resp";
const SENSITIVE_KEYS: [&str; 3] = ["access_token", "refresh_token", "id_token"];
const LOG_BODY_LIMIT: usize = 512;

/// One account's cached token and the refresh token it rotated to, if it did.
#[derive(Default)]
struct AccountTokens {
    cached: Option<AccessToken>,
    rotated_refresh_token: Option<String>,
}

/// Shared by every thread of the process (`SharedXOAuth2TokenManager`). Each account has its
/// own lock, so one account's refresh never waits for another's.
pub struct TokenManager {
    accounts: Mutex<HashMap<String, Arc<Mutex<AccountTokens>>>>,
    on_rotate: Option<RotateCallback>,
    clock: Arc<dyn Clock>,
    endpoints: Endpoints,
    gmail_client: Option<(String, String)>,
}

impl TokenManager {
    /// Uses the system clock, [`Endpoints::from_env`] and the `GMAIL_CLIENT_ID` /
    /// `GMAIL_CLIENT_SECRET` pair from the environment, else the Google client the engine
    /// signs in with ([`OAuthProvider::client_from_env`]).
    pub fn new(on_rotate: Option<RotateCallback>) -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let gmail_client = env(ENV_GMAIL_CLIENT_ID)
            .zip(env(ENV_GMAIL_CLIENT_SECRET))
            .or_else(|| {
                let client = OAuthProvider::Google.client_from_env();
                client.secret.map(|secret| (client.id, secret))
            });
        TokenManager {
            accounts: Mutex::new(HashMap::new()),
            on_rotate,
            clock: ms_core::clock::system(),
            endpoints: Endpoints::from_env(),
            gmail_client,
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    /// The client secret sent with refreshes for the Google client id it belongs to.
    pub fn with_gmail_client(mut self, client: Option<(String, String)>) -> Self {
        self.gmail_client = client;
        self
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    /// `account` with any refresh token rotated during this process applied, for output that
    /// echoes the account back to the client (`test` mode).
    pub fn account_with_current_secrets(&self, account: &Account) -> Account {
        let mut updated = account.clone();
        let entry = self.entry(account.id());
        if let Some(t) = &lock(&entry).rotated_refresh_token {
            updated.set_refresh_token(t);
        }
        updated
    }

    /// Drops what is cached for the account: it was removed from the process.
    pub fn forget(&self, account_id: &str) {
        self.accounts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(account_id);
    }

    /// A valid access token for `account`, refreshing it if needed (`partsForAccount`).
    ///
    /// The account's lock is held across the refresh, so concurrent callers for the same
    /// account wait for one request instead of each making their own.
    pub fn token_for(&self, http: &HttpClient, account: &Account) -> Result<AccessToken> {
        let entry = self.entry(account.id());
        let mut state = lock(&entry);

        if let Some(cached) = &state.cached {
            // Margin: the token has to stay valid while the caller uses it.
            if cached.expires_at > self.clock.now() + OAUTH_EXPIRY_MARGIN {
                return Ok(cached.clone());
            }
        }

        let client_id = account.refresh_client_id();
        if client_id.is_empty() {
            return Err(SyncError::fatal(
                ERROR_KEY,
                "XOAuth2 token expired and the account has no refresh_client_id to renew it.",
            )
            .into());
        }

        let refresh_token = state
            .rotated_refresh_token
            .clone()
            .unwrap_or_else(|| account.refresh_token().to_owned());

        info!(
            "Fetching XOAuth2 access token ({}) for {}",
            account.provider(),
            account.id()
        );
        let resp = self.request_refresh(http, account.provider(), client_id, &refresh_token)?;

        let access_token = access_token_of(&resp);
        if access_token.is_empty() {
            // A 2xx is not necessarily a token response: a captive portal or intercepting
            // proxy answers 200 with its own page. An OAuth error object is the provider
            // refusing, which a retry cannot fix; anything else is transient.
            let provider_refused = resp.get("error").is_some();
            return Err(SyncError::new(
                ERROR_KEY,
                summarize_for_log(&resp),
                !provider_refused,
                !provider_refused,
            )
            .into());
        }

        if let Some(updated) = resp.get("refresh_token").and_then(Value::as_str) {
            if updated != refresh_token {
                info!(
                    "Saving updated XOAuth2 refresh token ({}) for {}",
                    account.provider(),
                    account.id()
                );
                state.rotated_refresh_token = Some(updated.to_owned());
                let mut rotated = account.clone();
                rotated.set_refresh_token(updated);
                if let Some(cb) = &self.on_rotate {
                    cb(&rotated);
                }
            } else {
                info!("XOAuth2 refresh token was returned but matches existing one.");
            }
        }

        let token = AccessToken {
            username: account.imap_username().to_owned(),
            access_token,
            expires_at: self.clock.now() + expires_in_of(&resp),
        };
        state.cached = Some(token.clone());
        Ok(token)
    }

    /// `MakeOAuthRefreshRequest`.
    fn request_refresh(
        &self,
        http: &HttpClient,
        provider: &str,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<Value> {
        let Some(oauth) = OAuthProvider::from_account_provider(provider) else {
            return Err(SyncError::fatal(
                "URL using bad/illegal format or missing URL",
                format!("No OAuth token endpoint for provider '{provider}'"),
            )
            .into());
        };

        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
        ];
        if oauth == OAuthProvider::Microsoft {
            form.push(("scope", MICROSOFT_MAIL_SCOPES));
        }
        // Google desktop clients must send the client secret; it is an extension of the
        // client id rather than a secret (RFC 8252 §8.5). Only send it for the client id it
        // was issued with.
        if oauth == OAuthProvider::Google {
            if let Some((id, secret)) = &self.gmail_client {
                if id == client_id {
                    form.push(("client_secret", secret));
                }
            }
        }
        http.post_form(self.endpoints.token_url(oauth), &form)
    }

    fn entry(&self, account_id: &str) -> Arc<Mutex<AccountTokens>> {
        self.accounts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(account_id.to_owned())
            .or_default()
            .clone()
    }
}

/// Each field is replaced whole, so a panic while the lock is held (which stops only that
/// account) leaves nothing half-written for a poisoned lock to expose.
fn lock(entry: &Mutex<AccountTokens>) -> MutexGuard<'_, AccountTokens> {
    entry.lock().unwrap_or_else(|p| p.into_inner())
}

/// `expiresInOf`: a number, or a string that starts with one (as `stoi` reads it), else
/// [`OAUTH_ASSUMED_EXPIRES_IN`].
pub fn expires_in_of(resp: &Value) -> i64 {
    match resp.get("expires_in") {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(OAUTH_ASSUMED_EXPIRES_IN),
        Some(Value::String(s)) => stoi(s).unwrap_or(OAUTH_ASSUMED_EXPIRES_IN),
        _ => OAUTH_ASSUMED_EXPIRES_IN,
    }
}

/// `std::stoi`: leading whitespace, an optional sign, then digits up to the first
/// non-digit; `None` when there are no digits or the value overflows an `int`.
fn stoi(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let (negative, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    let n: i64 = digits[..end].parse().ok()?;
    let n = if negative { -n } else { n };
    i32::try_from(n).ok().map(i64::from)
}

fn access_token_of(resp: &Value) -> String {
    resp.get("access_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// `summarizeForLog`: this reaches the log and, in `test` mode, the client. The body may
/// be a captive portal's whole page, so it is cut short and token fields are masked.
pub fn summarize_for_log(resp: &Value) -> String {
    let mut redacted = resp.clone();
    if let Some(obj) = redacted.as_object_mut() {
        for key in SENSITIVE_KEYS {
            if let Some(v) = obj.get_mut(key) {
                *v = Value::String("*********".into());
            }
        }
    }
    let mut text = redacted.to_string();
    if text.len() > LOG_BODY_LIMIT {
        let mut cut = LOG_BODY_LIMIT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("... (truncated)");
    }
    format!("The token endpoint did not return an access token. Response: {text}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expires_in_variants() {
        assert_eq!(expires_in_of(&json!({"expires_in": 1800})), 1800);
        assert_eq!(expires_in_of(&json!({"expires_in": 1799.9})), 1799);
        assert_eq!(expires_in_of(&json!({"expires_in": "1200"})), 1200);
        assert_eq!(expires_in_of(&json!({"expires_in": " 900s"})), 900);
        assert_eq!(expires_in_of(&json!({"expires_in": "soon"})), 3600);
        assert_eq!(expires_in_of(&json!({"expires_in": ""})), 3600);
        assert_eq!(expires_in_of(&json!({"expires_in": "99999999999"})), 3600);
        assert_eq!(expires_in_of(&json!({"expires_in": null})), 3600);
        assert_eq!(expires_in_of(&json!({})), 3600);
        assert_eq!(expires_in_of(&json!({"text": "<html>"})), 3600);
    }

    #[test]
    fn summary_masks_and_truncates() {
        let s = summarize_for_log(&json!({"access_token": "", "refresh_token": "r", "x": 1}));
        assert!(s.contains(r#""refresh_token":"*********""#));
        assert!(!s.contains(r#""r""#));

        let long = summarize_for_log(&json!({"text": "é".repeat(600)}));
        assert!(long.ends_with("... (truncated)"));
        assert!(long.len() < 600);
    }
}
