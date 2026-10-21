// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Account setup on `serve`'s socket: discovery
//! over [`tern_providers::discovery`], the OAuth sign-in whose PKCE verifier, `state` and client
//! secret never leave the engine, Google grant revocation, and the account JSON a new account
//! gets (its id, trimmed hosts, numeric ports).
//!
//! Ports the desktop's `onboarding-helpers.ts` (`buildGmailAccountFromAuthResponse`,
//! `buildMicrosoftAccountFromAuthResponse`, `finalizeAndValidateAccount`) and
//! `oauth-revocation.ts`, from Mailspring, Copyright © 2017 Foundry 376, GPL-3.0. See
//! NOTICE.md.
//!
//! The client receives the browser's redirect (the desktop's main process, or the CLI, on
//! 127.0.0.1) and passes its `code` and `state` on; the engine checks the state, exchanges the
//! code and fetches the profile. `MAILSYNC_GOOGLE_USERINFO_URL`,
//! `MAILSYNC_MICROSOFT_GRAPH_ME_URL` and `MAILSYNC_GOOGLE_REVOKE_URL` point those calls at a
//! test server, as `MAILSYNC_GOOGLE_TOKEN_URL` already does for the token endpoint.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Map, Value};
use tern_ipc::{
    code, AccountDiscoverParams, AccountDiscoverResult, ErrorObject, OAuthBeginParams,
    OAuthBeginResult, OAuthCompleteParams,
};
use tern_net::{Endpoints, HttpClient};
use tern_providers::discovery::{self, Lookups};
use tern_providers::OAuthProvider;
use tracing::{info, warn};

use crate::dispatch::Reply;

pub const ENV_GOOGLE_USERINFO_URL: &str = "MAILSYNC_GOOGLE_USERINFO_URL";
pub const ENV_MICROSOFT_GRAPH_ME_URL: &str = "MAILSYNC_MICROSOFT_GRAPH_ME_URL";
pub const ENV_GOOGLE_REVOKE_URL: &str = "MAILSYNC_GOOGLE_REVOKE_URL";
/// Debug builds: discovery returns these settings (a JSON object) without any lookup, so
/// tests never reach a real provider.
const ENV_TEST_DISCOVERY: &str = "TERN_TEST_DISCOVERY";

/// How long a sign-in may take between `begin` and `complete`.
const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
/// Sign-ins kept at once; the oldest goes first.
const MAX_SESSIONS: usize = 16;
const AUTOCONFIG_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const AUTOCONFIG_TIMEOUT: Duration = Duration::from_secs(10);
const OAUTH_TIMEOUT: Duration = Duration::from_secs(30);
const REVOKE_TIMEOUT: Duration = Duration::from_secs(5);

fn invalid(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, message)
}

fn env_url(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

// -- Account JSON ------------------------------------------------------------------------------

/// Checks an address enough to build URLs and DNS names from its domain.
fn check_email(email: &str) -> Result<(), ErrorObject> {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return Err(invalid("invalid params: email must be an address"));
    };
    // A name, not an IP literal: discovery fetches https://<domain>/.
    let tld_ok = domain
        .rsplit('.')
        .next()
        .is_some_and(|t| t.bytes().any(|b| !b.is_ascii_digit()));
    let domain_ok = !domain.is_empty()
        && domain.len() <= 253
        && domain.contains('.')
        && tld_ok
        && !domain.starts_with('.')
        && !domain.contains("..")
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b >= 0x80);
    if local.is_empty() || !domain_ok || email.chars().any(char::is_whitespace) {
        return Err(invalid(format!(
            "invalid params: {email:?} is not an email address"
        )));
    }
    Ok(())
}

/// What `finalizeAndValidateAccount` did before testing: hosts trimmed, numeric ports, a
/// label that is an address replaced by the account's own, and an id when it has none
/// ([`tern_core::ids::account_id`]).
pub fn prepare_new_account(json: &mut Value) {
    let email = json
        .get("emailAddress")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if let Some(settings) = json.get_mut("settings").and_then(Value::as_object_mut) {
        for key in ["imap_host", "smtp_host"] {
            if let Some(Value::String(h)) = settings.get_mut(key) {
                *h = h.trim().to_owned();
            }
        }
        for key in ["imap_port", "smtp_port"] {
            let port = settings
                .get(key)
                .and_then(Value::as_str)
                .and_then(|p| p.trim().parse::<u16>().ok());
            if let Some(p) = port {
                settings.insert(key.to_owned(), json!(p));
            }
        }
    }
    let Some(fields) = json.as_object_mut() else {
        return;
    };
    if fields
        .get("label")
        .and_then(Value::as_str)
        .is_none_or(|l| l.contains('@'))
    {
        fields.insert("label".into(), json!(email));
    }
    let has_id = fields
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    if !has_id {
        let settings = fields
            .get("settings")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        fields.insert(
            "id".into(),
            json!(tern_core::ids::account_id(&email, &settings)),
        );
    }
}

// -- Discovery ---------------------------------------------------------------------------------

/// DNS and HTTPS for [`discovery::discover`].
struct Network {
    http: Option<HttpClient>,
}

impl Lookups for Network {
    fn mx(&mut self, domain: &str) -> Vec<String> {
        tern_dav::discovery::DnsResolver.mx(domain)
    }

    fn fetch(&mut self, url: &str) -> Option<String> {
        if self.http.is_none() {
            self.http = HttpClient::with_timeouts(AUTOCONFIG_CONNECT_TIMEOUT, AUTOCONFIG_TIMEOUT)
                .map_err(|e| warn!("-X Could not create the HTTP client: {e}"))
                .ok();
        }
        let resp = self.http.as_ref()?.inner().get(url).send().ok()?;
        // A redirect to plain http could hand out an attacker's servers.
        if !resp.status().is_success() || resp.url().scheme() != "https" {
            return None;
        }
        let mut body = Vec::new();
        resp.take(discovery::AUTOCONFIG_MAX_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .ok()?;
        String::from_utf8(body).ok()
    }
}

/// The settings for a new account, with where they came from.
pub fn discover_settings(email: &str, provider: &str) -> discovery::Discovered {
    if cfg!(debug_assertions) {
        if let Some(settings) = std::env::var(ENV_TEST_DISCOVERY)
            .ok()
            .and_then(|t| serde_json::from_str::<Map<String, Value>>(&t).ok())
        {
            return discovery::Discovered {
                source: discovery::Source::Fallback,
                template: Some("test".into()),
                settings,
            };
        }
    }
    discovery::discover(email, provider, &mut Network { http: None })
}

/// `account.discover`.
pub fn discover(p: &AccountDiscoverParams) -> Reply {
    let email = p.email.trim();
    check_email(email)?;
    let provider = p.provider.as_deref().unwrap_or("imap");
    let found = discover_settings(email, provider);
    info!(
        "Discovered settings for a {provider} account at {}: {}",
        discovery::email_domain(email),
        found.source.as_str()
    );
    serde_json::to_value(AccountDiscoverResult {
        account: json!({
            "emailAddress": email,
            "provider": provider,
            "settings": found.settings,
        }),
        source: found.source.as_str().to_owned(),
        template: found.template,
    })
    .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
}

// -- OAuth -------------------------------------------------------------------------------------

/// One sign-in between `begin` and `complete`.
pub struct OAuthSession {
    /// `gmail`, `office365` or `outlook`.
    pub provider: String,
    pub oauth: OAuthProvider,
    verifier: String,
    state: String,
    pub redirect_uri: String,
    pub shared_mailbox: Option<String>,
    /// Reconnect: the registered account the sign-in must be for.
    pub account_id: Option<String>,
    pub container_folder: Option<String>,
    created: Instant,
}

/// The sign-ins in progress. Each is used once.
#[derive(Default)]
pub struct OAuthSessions(Mutex<HashMap<String, OAuthSession>>);

fn random_token() -> Result<String, ErrorObject> {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| ErrorObject::new(code::INTERNAL_ERROR, "no random source"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// RFC 7636 §4.2 `S256`.
fn code_challenge(verifier: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref())
}

fn same_bytes(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

impl OAuthSessions {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, OAuthSession>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `account.oauth.begin`.
    pub fn begin(&self, p: &OAuthBeginParams) -> Result<OAuthBeginResult, ErrorObject> {
        let oauth = OAuthProvider::from_account_provider(&p.provider).ok_or_else(|| {
            invalid(format!(
                "invalid params: {:?} doesn't sign in with OAuth (gmail, office365 or outlook)",
                p.provider
            ))
        })?;
        if let Some(shared) = &p.shared_mailbox {
            if p.provider != "office365" {
                return Err(invalid(
                    "invalid params: sharedMailbox is for office365 accounts",
                ));
            }
            check_email(shared)?;
        }
        let port = p
            .redirect_port
            .unwrap_or(tern_providers::oauth::DEFAULT_REDIRECT_PORT);
        if port == 0 {
            return Err(invalid("invalid params: redirectPort must not be 0"));
        }
        let redirect_uri = oauth.redirect_uri(port);
        let (verifier, state, session_id) = (random_token()?, random_token()?, random_token()?);
        let client = oauth.client_from_env();
        let auth_url = oauth.authorization_request(
            oauth.authorize_url(),
            &client.id,
            &redirect_uri,
            &code_challenge(&verifier),
            &state,
        );
        let mut sessions = self.lock();
        sessions.retain(|_, s| s.created.elapsed() < SESSION_TTL);
        while sessions.len() >= MAX_SESSIONS {
            let oldest = sessions
                .iter()
                .min_by_key(|(_, s)| s.created)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => sessions.remove(&k),
                None => break,
            };
        }
        sessions.insert(
            session_id.clone(),
            OAuthSession {
                provider: p.provider.clone(),
                oauth,
                verifier,
                state: state.clone(),
                redirect_uri: redirect_uri.clone(),
                shared_mailbox: p.shared_mailbox.clone(),
                account_id: p.account_id.clone(),
                container_folder: p.container_folder.clone(),
                created: Instant::now(),
            },
        );
        info!("OAuth sign-in started for a {} account", p.provider);
        Ok(OAuthBeginResult {
            session_id,
            auth_url,
            redirect_uri,
            state,
        })
    }

    /// The session `complete` names, once its `state` matches; it is used up then. A
    /// mismatched state leaves it for the redirect that carries the right one.
    pub fn take(&self, p: &OAuthCompleteParams) -> Result<OAuthSession, ErrorObject> {
        let mut sessions = self.lock();
        let expired = sessions
            .get(&p.session_id)
            .is_some_and(|s| s.created.elapsed() >= SESSION_TTL);
        if expired {
            sessions.remove(&p.session_id);
        }
        let Some(session) = sessions.get(&p.session_id) else {
            return Err(invalid(
                "invalid params: no such sign-in, or it expired; start again",
            ));
        };
        if !same_bytes(&session.state, &p.state) {
            return Err(invalid(
                "invalid params: the sign-in response doesn't match this sign-in (state)",
            ));
        }
        Ok(sessions.remove(&p.session_id).expect("checked above"))
    }
}

fn oauth_failed(key: &str, message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(code::OAUTH_FAILED, message).with_data(json!({ "key": key }))
}

/// An HTTP failure of the sign-in as [`code::OAUTH_FAILED`]. The provider's error body is
/// kept (it names the problem, never a token: the response carries none on failure).
fn http_failed(key: &str, what: &str, e: tern_core::Error) -> ErrorObject {
    let (message, data) = match e.as_sync() {
        Some(se) => (
            format!("{what} failed: {}: {}", se.key, se.debuginfo),
            json!({ "key": key, "offline": se.is_offline() }),
        ),
        None => (format!("{what} failed: {e}"), json!({ "key": key })),
    };
    ErrorObject::new(code::OAUTH_FAILED, message).with_data(data)
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// The `email`, `preferred_username` or `unique_name` claim of an id token, when it looks
/// like an address. The token isn't verified: it came straight from the token endpoint
/// over TLS, and only names the mailbox the user just signed in to.
fn id_token_email(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let candidate = ["email", "preferred_username", "unique_name"]
        .iter()
        .find_map(|k| text(&claims, k))?;
    let looks_like_address = candidate
        .split_once('@')
        .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.'))
        && !candidate.contains(char::is_whitespace);
    looks_like_address.then(|| candidate.to_owned())
}

/// Exchanges the code and reads the profile: the new account's JSON, refresh token
/// included, without servers yet.
pub fn sign_in(session: &OAuthSession, code: &str) -> Result<Value, ErrorObject> {
    let http = HttpClient::with_timeouts(OAUTH_TIMEOUT, OAUTH_TIMEOUT)
        .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))?;
    let oauth = session.oauth;
    let client = oauth.client_from_env();
    let token_url = Endpoints::from_env().token_url(oauth).to_owned();
    let scope = oauth.exchange_scope();
    let mut form: Vec<(&str, &str)> = vec![
        ("code", code),
        ("client_id", &client.id),
        ("code_verifier", &session.verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", &session.redirect_uri),
    ];
    if let Some(secret) = &client.secret {
        form.push(("client_secret", secret));
    }
    if let Some(scope) = &scope {
        form.push(("scope", scope));
    }
    let tokens = http
        .post_form(&token_url, &form)
        .map_err(|e| http_failed("exchange", "The OAuth code exchange", e))?;
    let (Some(access_token), Some(refresh_token)) = (
        text(&tokens, "access_token"),
        text(&tokens, "refresh_token"),
    ) else {
        return Err(oauth_failed(
            "exchange",
            "The OAuth code exchange returned no access or refresh token",
        ));
    };

    let (name, mut email) = match oauth {
        OAuthProvider::Google => {
            let url = env_url(
                ENV_GOOGLE_USERINFO_URL,
                tern_providers::oauth::GOOGLE_USERINFO_URL,
            );
            let me = http
                .json("GET", &url, Some(access_token), None)
                .map_err(|e| http_failed("profile", "The Google profile request", e))?;
            (
                text(&me, "name").unwrap_or_default().to_owned(),
                text(&me, "email").map(str::to_owned),
            )
        }
        OAuthProvider::Microsoft => {
            let url = env_url(
                ENV_MICROSOFT_GRAPH_ME_URL,
                tern_providers::oauth::MICROSOFT_GRAPH_ME_URL,
            );
            let me = http
                .json("GET", &url, Some(access_token), None)
                .map_err(|e| http_failed("profile", "The Microsoft profile request", e))?;
            // Graph can answer 200 with an error body.
            if let Some(err) = me.get("error") {
                return Err(oauth_failed(
                    "profile",
                    format!(
                        "The Microsoft profile request failed: {}: {}",
                        text(err, "code").unwrap_or("error"),
                        text(err, "message").unwrap_or_default()
                    ),
                ));
            }
            // Personal accounts and accounts without an Exchange Online licence may have
            // neither; the id token's claims name the mailbox then.
            let email = text(&me, "mail")
                .or_else(|| text(&me, "userPrincipalName"))
                .map(str::to_owned)
                .or_else(|| text(&tokens, "id_token").and_then(id_token_email));
            (
                text(&me, "displayName").unwrap_or_default().to_owned(),
                email,
            )
        }
    };
    let Some(signed_in) = email.take() else {
        return Err(oauth_failed(
            "no-mailbox",
            "There is no email mailbox associated with this account.",
        ));
    };

    let mut settings = json!({
        "refresh_client_id": client.id,
        "refresh_token": refresh_token,
    });
    let mut address = signed_in.clone();
    // A shared mailbox is opened with the signed-in user's grant: IMAP takes the shared
    // address as the XOAUTH2 identity, SMTP AUTH only the user, who sends as the shared
    // address. `smtp_verification: login` keeps the test from sending mail the mailbox's
    // members would see; no helper folders in a mailbox others share.
    if let Some(shared) = &session.shared_mailbox {
        settings["smtp_username"] = json!(signed_in);
        settings["smtp_verification"] = json!("login");
        settings["create_helper_folders"] = json!(false);
        address = shared.clone();
    }
    Ok(json!({
        "name": name,
        "emailAddress": address,
        "provider": session.provider,
        "settings": settings,
    }))
}

/// Fills in the servers of a signed-in account: its own settings win over discovery's
/// (`Object.assign(defaults, settings)`), then [`prepare_new_account`].
pub fn with_discovered_servers(mut account: Value, container_folder: Option<&str>) -> Value {
    let email = text(&account, "emailAddress")
        .unwrap_or_default()
        .to_owned();
    let provider = text(&account, "provider").unwrap_or("imap").to_owned();
    let found = discover_settings(&email, &provider);
    let mut settings = found.settings;
    if let Some(own) = account.get("settings").and_then(Value::as_object) {
        for (k, v) in own {
            settings.insert(k.clone(), v.clone());
        }
    }
    let folder_unset = settings
        .get("container_folder")
        .is_none_or(|v| v.as_str() == Some(""));
    if let (true, Some(folder)) = (folder_unset, container_folder) {
        settings.insert("container_folder".into(), json!(folder));
    }
    account["settings"] = Value::Object(settings);
    prepare_new_account(&mut account);
    account
}

/// Asks Google to drop the grant behind a refresh token (`revokeOAuthGrant`). Revocation is
/// grant-wide: every install signed in to the account with this client loses it, which a
/// removal accepts. Failures (offline, already revoked) are logged and ignored.
pub fn revoke(provider: &str, refresh_token: &str) {
    if provider != "gmail" || refresh_token.is_empty() {
        return;
    }
    let url = env_url(
        ENV_GOOGLE_REVOKE_URL,
        tern_providers::oauth::GOOGLE_REVOKE_URL,
    );
    let outcome = HttpClient::with_timeouts(REVOKE_TIMEOUT, REVOKE_TIMEOUT)
        .and_then(|http| http.post_form(&url, &[("token", refresh_token)]));
    match outcome {
        Ok(_) => info!("Revoked the account's Google grant"),
        Err(e) => warn!("-X Could not revoke the account's Google grant: {e}"),
    }
}

/// `account.test`'s failure as [`code::ACCOUNT_TEST_FAILED`].
pub fn test_failed(result: Value) -> ErrorObject {
    let what = result
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("the test failed")
        .to_owned();
    let service = result
        .get("error_service")
        .and_then(Value::as_str)
        .unwrap_or("imap")
        .to_owned();
    ErrorObject::new(
        code::ACCOUNT_TEST_FAILED,
        format!("Account test failed ({service}): {what}"),
    )
    .with_data(result)
}

/// Whether an `account.test` result passed.
pub fn test_passed(result: &Value) -> bool {
    result.get("error").is_none_or(Value::is_null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_accounts_get_the_desktop_id() {
        let mut a = json!({
            "emailAddress": "ben@gmail.com", "provider": "gmail",
            "settings": {
                "imap_username": "ben@gmail.com", "imap_host": " imap.gmail.com ",
                "smtp_username": "ben@gmail.com", "smtp_host": "smtp.gmail.com",
                "imap_port": "993",
            },
        });
        prepare_new_account(&mut a);
        assert_eq!(a["id"], "6c210fa1");
        assert_eq!(a["settings"]["imap_host"], "imap.gmail.com");
        assert_eq!(a["settings"]["imap_port"], 993);
        assert_eq!(a["label"], "ben@gmail.com");
        let mut kept =
            json!({"id": "mine", "emailAddress": "a@b.test", "label": "Work", "settings": {}});
        prepare_new_account(&mut kept);
        assert_eq!(
            (kept["id"].as_str(), kept["label"].as_str()),
            (Some("mine"), Some("Work"))
        );
    }

    #[test]
    fn sessions_check_state_and_expire_once_used() {
        let sessions = OAuthSessions::default();
        let begun = sessions
            .begin(&OAuthBeginParams {
                provider: "gmail".into(),
                shared_mailbox: None,
                redirect_port: Some(4000),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(begun.redirect_uri, "http://127.0.0.1:4000");
        assert!(begun.auth_url.contains(&format!("state={}", begun.state)));
        assert!(begun.auth_url.contains("code_challenge_method=S256"));
        let complete = |state: &str| OAuthCompleteParams {
            session_id: begun.session_id.clone(),
            code: "c".into(),
            state: state.into(),
            test: None,
        };
        assert_eq!(
            sessions.take(&complete("wrong")).err().unwrap().code,
            code::INVALID_PARAMS
        );
        let s = sessions.take(&complete(&begun.state)).unwrap();
        assert_eq!(code_challenge(&s.verifier).len(), 43);
        assert!(sessions.take(&complete(&begun.state)).is_err(), "used once");
        assert!(sessions
            .begin(&OAuthBeginParams {
                provider: "imap".into(),
                shared_mailbox: None,
                ..Default::default()
            },)
            .is_err());
        assert!(sessions
            .begin(&OAuthBeginParams {
                provider: "gmail".into(),
                shared_mailbox: Some("s@x.test".into()),
                ..Default::default()
            },)
            .is_err());
    }

    #[test]
    fn id_token_claims() {
        let b64 = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
        let token = |claims: Value| format!("h.{}.s", b64(claims));
        assert_eq!(
            id_token_email(&token(json!({"preferred_username": "me@live.com"}))).as_deref(),
            Some("me@live.com")
        );
        assert_eq!(id_token_email(&token(json!({"email": "8f0e-guid"}))), None);
        assert_eq!(id_token_email("garbage"), None);
    }

    #[test]
    fn addresses() {
        assert!(check_email("a@b.test").is_ok());
        for bad in [
            "nobody",
            "a@",
            "@b.test",
            "a@b",
            "a b@c.test",
            "a@b/c.test",
            "a@b.test/x",
            "a@10.0.0.1",
            "a@.b.test",
        ] {
            assert!(check_email(bad).is_err(), "{bad}");
        }
    }
}
