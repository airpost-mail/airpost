// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{closed_port_url, MockServer, Reply};
use serde_json::{json, Value};
use tern_core::{Account, Clock, ManualClock};
use tern_net::oauth::MICROSOFT_MAIL_SCOPES;
use tern_net::{Endpoints, HttpClient, TokenManager};

fn account(provider: &str, client_id: &str) -> Account {
    Account::from_json(json!({
        "id": "acct-1", "provider": provider, "emailAddress": "me@example.test",
        "settings": {
            "imap_username": "me@example.test", "refresh_token": "r1",
            "refresh_client_id": client_id,
        }
    }))
    .unwrap()
}

struct Fixture {
    server: MockServer,
    clock: Arc<ManualClock>,
    rotated: Arc<Mutex<Vec<Account>>>,
    tokens: TokenManager,
    http: HttpClient,
}

/// A token endpoint answering with `responses` in order (the last one repeats).
fn fixture(responses: Vec<Reply>) -> Fixture {
    let queue = Mutex::new(responses);
    let server = MockServer::start(move |_| {
        let mut q = queue.lock().unwrap();
        if q.len() > 1 {
            q.remove(0)
        } else {
            q[0].clone()
        }
    });
    let clock = Arc::new(ManualClock::new(1_000_000));
    let rotated = Arc::new(Mutex::new(Vec::new()));
    let sink = rotated.clone();
    let tokens = TokenManager::new(Some(Box::new(move |a: &Account| {
        sink.lock().unwrap().push(a.clone())
    })))
    .with_clock(clock.clone())
    .with_endpoints(Endpoints::all_at(&server.url()))
    .with_gmail_client(None);
    Fixture {
        server,
        clock,
        rotated,
        tokens,
        http: HttpClient::new().unwrap(),
    }
}

fn token(access: &str, expires_in: Value) -> Reply {
    Reply::json(
        200,
        json!({"access_token": access, "expires_in": expires_in, "token_type": "Bearer"}),
    )
}

fn sync_err(e: tern_core::Error) -> tern_core::SyncError {
    e.as_sync().cloned().expect("a SyncError")
}

#[test]
fn google_refresh_and_cache() {
    let f = fixture(vec![token("a1", json!(3600))]);
    let acct = account("gmail", "cid");

    let t = f.tokens.token_for(&f.http, &acct).unwrap();
    assert_eq!(t.access_token, "a1");
    assert_eq!(t.username, "me@example.test");
    assert_eq!(t.expires_at, 1_000_000 + 3600);
    assert_eq!(f.tokens.token_for(&f.http, &acct).unwrap(), t);
    assert_eq!(f.server.count(), 1);

    let r = &f.server.requests()[0];
    assert_eq!((r.method.as_str(), r.path()), ("POST", "/google/token"));
    assert_eq!(r.header("accept"), Some("application/json"));
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(r.form_value("grant_type").as_deref(), Some("refresh_token"));
    assert_eq!(r.form_value("client_id").as_deref(), Some("cid"));
    assert_eq!(r.form_value("refresh_token").as_deref(), Some("r1"));
    assert_eq!(r.form_value("scope"), None);
    assert_eq!(r.form_value("client_secret"), None);
    assert!(f.rotated.lock().unwrap().is_empty());
}

#[test]
fn cache_margin_is_sixty_seconds() {
    let f = fixture(vec![token("a1", json!(100)), token("a2", json!(100))]);
    let acct = account("gmail", "cid");

    assert_eq!(
        f.tokens.token_for(&f.http, &acct).unwrap().access_token,
        "a1"
    );
    // expires at +100: still served while more than 60 s remain.
    f.clock.advance(39);
    assert_eq!(
        f.tokens.token_for(&f.http, &acct).unwrap().access_token,
        "a1"
    );
    assert_eq!(f.server.count(), 1);
    f.clock.advance(1);
    assert_eq!(
        f.tokens.token_for(&f.http, &acct).unwrap().access_token,
        "a2"
    );
    assert_eq!(f.server.count(), 2);
}

#[test]
fn expires_in_string_number_or_missing() {
    let f = fixture(vec![
        token("a1", json!("120")),
        Reply::json(200, json!({"access_token": "a2"})),
        token("a3", json!("not-a-number")),
        token("a4", json!(1800.5)),
    ]);
    let acct = account("gmail", "cid");
    let expect = |access: &str, lifetime: i64| {
        let t = f.tokens.token_for(&f.http, &acct).unwrap();
        assert_eq!(t.access_token, access);
        assert_eq!(t.expires_at, f.clock.now() + lifetime, "{access}");
        // Jump past the cached token so the next call refreshes.
        f.clock.advance(lifetime);
    };
    expect("a1", 120);
    expect("a2", 3600);
    expect("a3", 3600);
    expect("a4", 1800);
    assert_eq!(f.server.count(), 4);
}

#[test]
fn rotated_refresh_token_is_reported_and_reused() {
    let f = fixture(vec![
        Reply::json(
            200,
            json!({"access_token": "a1", "expires_in": 100, "refresh_token": "r2"}),
        ),
        Reply::json(
            200,
            json!({"access_token": "a2", "expires_in": 100, "refresh_token": "r2"}),
        ),
    ]);
    let acct = account("office365", "ms-client");

    f.tokens.token_for(&f.http, &acct).unwrap();
    {
        let rotated = f.rotated.lock().unwrap();
        assert_eq!(rotated.len(), 1);
        assert_eq!(rotated[0].refresh_token(), "r2");
        assert_eq!(rotated[0].id(), "acct-1");
        assert_eq!(rotated[0].imap_username(), "me@example.test");
    }
    assert_eq!(
        f.tokens.account_with_current_secrets(&acct).refresh_token(),
        "r2"
    );

    f.clock.advance(100);
    f.tokens.token_for(&f.http, &acct).unwrap();
    let reqs = f.server.requests();
    assert_eq!(reqs[1].form_value("refresh_token").as_deref(), Some("r2"));
    // The same token coming back again is not a rotation.
    assert_eq!(f.rotated.lock().unwrap().len(), 1);
}

#[test]
fn microsoft_asks_for_mail_scopes() {
    for provider in ["office365", "outlook"] {
        let f = fixture(vec![token("a1", json!(3600))]);
        f.tokens
            .token_for(&f.http, &account(provider, "ms-client"))
            .unwrap();
        let r = &f.server.requests()[0];
        assert_eq!(r.path(), "/microsoft/token");
        assert_eq!(
            r.form_value("scope").as_deref(),
            Some(MICROSOFT_MAIL_SCOPES)
        );
        assert_eq!(r.form_value("client_secret"), None);
    }
}

#[test]
fn gmail_client_secret_only_for_its_client_id() {
    let mut f = fixture(vec![token("a1", json!(3600))]);
    f.tokens = f
        .tokens
        .with_gmail_client(Some(("cid".into(), "s3cret".into())));

    f.tokens
        .token_for(&f.http, &account("gmail", "cid"))
        .unwrap();
    let other = Account::from_json(json!({
        "id": "acct-2", "provider": "gmail",
        "settings": {"refresh_token": "r", "refresh_client_id": "other"}
    }))
    .unwrap();
    f.tokens.token_for(&f.http, &other).unwrap();

    let reqs = f.server.requests();
    assert_eq!(
        reqs[0].form_value("client_secret").as_deref(),
        Some("s3cret")
    );
    assert_eq!(reqs[1].form_value("client_secret"), None);
}

/// An account bound to an older client still sends that client's secret once a new one is
/// configured.
#[test]
fn gmail_refreshes_send_the_secret_of_the_accounts_client() {
    let mut f = fixture(vec![token("a1", json!(3600)), token("a2", json!(3600))]);
    f.tokens = f.tokens.with_gmail_clients(vec![
        ("new".into(), "new-s3cret".into()),
        ("cid".into(), "old-s3cret".into()),
    ]);
    f.tokens
        .token_for(&f.http, &account("gmail", "cid"))
        .unwrap();
    let newer = Account::from_json(json!({
        "id": "acct-2", "provider": "gmail",
        "settings": {"refresh_token": "r", "refresh_client_id": "new"}
    }))
    .unwrap();
    f.tokens.token_for(&f.http, &newer).unwrap();
    let reqs = f.server.requests();
    assert_eq!(
        reqs[0].form_value("client_secret").as_deref(),
        Some("old-s3cret")
    );
    assert_eq!(
        reqs[1].form_value("client_secret").as_deref(),
        Some("new-s3cret")
    );
}

#[test]
fn provider_refusals_are_not_retryable() {
    // invalid_grant: the refresh token was revoked.
    let f = fixture(vec![Reply::json(
        400,
        json!({"error": "invalid_grant", "error_description": "Token has been revoked."}),
    )]);
    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("gmail", "cid"))
            .unwrap_err(),
    );
    assert_eq!(e.key, "Invalid Response Code: 400");
    assert!(e.debuginfo.contains("/google/token RETURNED "));
    assert!(e.debuginfo.contains("invalid_grant"));
    assert!(!e.retryable && !e.offline);

    // A 2xx carrying an OAuth error object.
    let f = fixture(vec![Reply::json(
        200,
        json!({"error": "unauthorized_client"}),
    )]);
    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("gmail", "cid"))
            .unwrap_err(),
    );
    assert_eq!(e.key, "invalid-xoauth2-resp");
    assert!(e
        .debuginfo
        .starts_with("The token endpoint did not return an access token. Response: "));
    assert!(!e.retryable && !e.offline);

    // 401 from the endpoint.
    let f = fixture(vec![Reply::json(401, json!({"error": "invalid_client"}))]);
    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("gmail", "cid"))
            .unwrap_err(),
    );
    assert!(!e.retryable);
}

#[test]
fn interception_and_server_errors_are_retryable() {
    // A captive portal answering 200 with HTML.
    let f = fixture(vec![Reply::text(200, "<html>Sign in to Wi-Fi</html>")]);
    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("gmail", "cid"))
            .unwrap_err(),
    );
    assert_eq!(e.key, "invalid-xoauth2-resp");
    assert!(e.debuginfo.contains("Sign in to Wi-Fi"));
    assert!(e.retryable && e.offline);

    // A token response whose access token is empty has its secrets masked in the log.
    let f = fixture(vec![Reply::json(
        200,
        json!({"access_token": "", "refresh_token": "leak", "id_token": "leak"}),
    )]);
    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("gmail", "cid"))
            .unwrap_err(),
    );
    assert!(!e.debuginfo.contains("leak"));
    assert!(e.retryable && e.offline);

    let f = fixture(vec![Reply::text(503, "unavailable")]);
    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("gmail", "cid"))
            .unwrap_err(),
    );
    assert_eq!(e.key, "Invalid Response Code: 503");
    assert!(e.retryable && !e.offline);

    // Nothing cached after failures: the next call asks again.
    assert_eq!(f.server.count(), 1);
    let _ = f.tokens.token_for(&f.http, &account("gmail", "cid"));
    assert_eq!(f.server.count(), 2);
}

#[test]
fn unreachable_endpoint_is_offline() {
    let tokens = TokenManager::new(None)
        .with_endpoints(Endpoints::all_at(&closed_port_url()))
        .with_gmail_client(None);
    let http = HttpClient::new().unwrap();
    let e = sync_err(
        tokens
            .token_for(&http, &account("gmail", "cid"))
            .unwrap_err(),
    );
    assert_eq!(e.key, "Could not connect to server");
    assert!(e.retryable && e.offline);
}

#[test]
fn accounts_that_cannot_refresh() {
    let f = fixture(vec![token("a1", json!(3600))]);

    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("gmail", ""))
            .unwrap_err(),
    );
    assert_eq!(e.key, "invalid-xoauth2-resp");
    assert!(!e.retryable && !e.offline);

    let e = sync_err(
        f.tokens
            .token_for(&f.http, &account("imap", "cid"))
            .unwrap_err(),
    );
    assert_eq!(e.key, "URL using bad/illegal format or missing URL");
    assert!(!e.retryable);

    assert_eq!(f.server.count(), 0);
}

#[test]
fn cache_is_per_account() {
    let f = fixture(vec![token("a1", json!(3600)), token("b1", json!(3600))]);
    let a = account("gmail", "cid");
    let b = Account::from_json(json!({
        "id": "acct-2", "provider": "gmail",
        "settings": {"imap_username": "b", "refresh_token": "rb", "refresh_client_id": "cid"}
    }))
    .unwrap();
    assert_eq!(f.tokens.token_for(&f.http, &a).unwrap().access_token, "a1");
    assert_eq!(f.tokens.token_for(&f.http, &b).unwrap().access_token, "b1");
    assert_eq!(f.tokens.token_for(&f.http, &a).unwrap().access_token, "a1");
    assert_eq!(f.server.count(), 2);
}

#[test]
fn concurrent_callers_share_one_refresh() {
    let f = fixture(vec![
        token("a1", json!(3600)).delayed(Duration::from_millis(300))
    ]);
    let f = Arc::new(f);
    let acct = Arc::new(account("gmail", "cid"));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let f = f.clone();
            let acct = acct.clone();
            std::thread::spawn(move || f.tokens.token_for(&f.http, &acct).unwrap())
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap().access_token, "a1");
    }
    assert_eq!(f.server.count(), 1);
}

#[test]
fn one_accounts_slow_refresh_does_not_hold_up_another() {
    let server = MockServer::start(|r| {
        if r.form_value("refresh_token").as_deref() == Some("slow") {
            token("slow-token", json!(3600)).delayed(Duration::from_secs(2))
        } else {
            token("fast-token", json!(3600))
        }
    });
    let tokens = Arc::new(
        TokenManager::new(None)
            .with_clock(Arc::new(ManualClock::new(1_000_000)))
            .with_endpoints(Endpoints::all_at(&server.url()))
            .with_gmail_client(None),
    );
    let with_refresh_token = |id: &str, refresh: &str| {
        Account::from_json(json!({
            "id": id, "provider": "gmail", "emailAddress": "me@example.test",
            "settings": {
                "imap_username": "me@example.test", "refresh_token": refresh,
                "refresh_client_id": "cid",
            }
        }))
        .unwrap()
    };
    let slow = with_refresh_token("slow-acct", "slow");
    let fast = with_refresh_token("fast-acct", "fast");

    let t = tokens.clone();
    let slow_refresh =
        std::thread::spawn(move || t.token_for(&HttpClient::new().unwrap(), &slow).unwrap());
    while server.count() == 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = std::time::Instant::now();
    let got = tokens
        .token_for(&HttpClient::new().unwrap(), &fast)
        .unwrap();
    assert_eq!(got.access_token, "fast-token");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "waited {:?} for the other account's refresh",
        started.elapsed()
    );
    assert_eq!(slow_refresh.join().unwrap().access_token, "slow-token");
}
