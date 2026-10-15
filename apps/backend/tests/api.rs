// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! B0 gate (docs/backend-plan.md 6.2): onboarding, auth, `/api/me`, stubs, account removal and
//! CORS against a server on an ephemeral port.

mod common;

use common::{basic, identity_result, TestServer, PUBLIC_URL};
use hyper::{Method, StatusCode};
use tern_backend::config::Signup;
use tern_backend::repo::Repo;

const UA: &str = "Mozilla/5.0 Mailspring/1.17.0 Electron/37";

#[tokio::test]
async fn ping() {
    let s = TestServer::start(Signup::Closed).await;
    let r = s.get("/ping", &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json(), serde_json::json!({}));
    s.stop().await;
}

#[tokio::test]
async fn onboarding_login_returns_identity() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("Ada@Example.com", "correct horse");

    let page = s
        .get(
            "/onboarding?version=1.17.0&skipSupported=true",
            &[("user-agent", UA)],
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.header("cache-control"), Some("no-store"));
    assert!(page
        .header("content-type")
        .unwrap()
        .starts_with("text/html"));
    assert!(page.body.contains(r#"action="onboarding""#));
    assert!(page.body.contains(r#"href="onboarding/skip""#));
    assert!(!page.body.contains("Create account"), "sign-up is closed");
    assert!(identity_result(&page.body).is_none());

    let res = s
        .post_form(
            "/onboarding",
            &[("user-agent", UA)],
            &[
                ("action", "login"),
                ("skip_supported", "true"),
                ("email", "ada@example.com"),
                ("password", "correct horse"),
            ],
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control"), Some("no-store"));
    let id = identity_result(&res.body).expect("#identity-result");

    assert_eq!(id["id"], user.id.as_str());
    assert_eq!(id["emailAddress"], "ada@example.com");
    assert_eq!(id["firstName"], "Ada");
    assert_eq!(id["lastName"], "Lovelace");
    let token = id["token"].as_str().unwrap();
    assert!(token.starts_with("tern_"), "{token}");
    let created_at = id["createdAt"].as_str().unwrap();
    let parsed = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%dT%H:%M:%S.000Z")
        .unwrap_or_else(|e| panic!("{created_at}: {e}"));
    let cutoff = chrono::NaiveDate::from_ymd_opt(2018, 2, 15)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    assert!(parsed > cutoff, "{created_at}");
    assert_eq!(id["stripePlan"], "Tern");
    assert_eq!(id["stripePlanEffective"], "Tern");
    assert_eq!(id["featureUsage"], serde_json::json!({}));
    assert_eq!(id["tern"]["serverId"], s.repo.server_id().unwrap().as_str());
    assert_eq!(id["tern"]["publicURL"], PUBLIC_URL);
    assert_eq!(id["tern"]["trackingNS"], user.tracking_ns.as_str());

    // The token works, and it is named after the device.
    let me = s.get("/api/me", &[("authorization", &basic(token))]).await;
    assert_eq!(me.status, StatusCode::OK);
    let me = me.json();
    assert!(me.get("token").is_none());
    let mut expected = id.clone();
    expected.as_object_mut().unwrap().remove("token");
    assert_eq!(me, expected);
    let tokens = s.repo.list_tokens(Some(&user.id)).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].name, UA);

    // A second login is a second device.
    let again = s
        .post_form(
            "/onboarding",
            &[],
            &[("email", "ada@example.com"), ("password", "correct horse")],
        )
        .await;
    let second = identity_result(&again.body).unwrap();
    assert_ne!(second["token"], id["token"]);
    assert_eq!(
        s.repo.list_tokens(Some(&user.id)).unwrap()[1].name,
        "unknown device"
    );
    s.stop().await;
}

#[tokio::test]
async fn onboarding_rejects_bad_credentials() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    for (email, password) in [
        ("ada@example.com", "wrong horse"),
        ("nobody@example.com", "correct horse"),
        ("", ""),
    ] {
        let r = s
            .post_form(
                "/onboarding",
                &[],
                &[("email", email), ("password", password)],
            )
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{email}");
        assert_eq!(r.header("cache-control"), Some("no-store"));
        assert!(identity_result(&r.body).is_none());
        assert!(r.body.contains("Incorrect email address or password."));
    }
    s.repo.set_user_disabled(&user.id, true, 1).unwrap();
    let r = s
        .post_form(
            "/onboarding",
            &[],
            &[("email", "ada@example.com"), ("password", "correct horse")],
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(s.repo.list_tokens(None).unwrap().is_empty());
    s.stop().await;
}

#[tokio::test]
async fn onboarding_escapes_input() {
    let s = TestServer::start(Signup::Closed).await;
    let r = s
        .post_form(
            "/onboarding",
            &[],
            &[("email", "\"><script>alert(1)</script>"), ("password", "x")],
        )
        .await;
    assert!(!r.body.contains("<script>"));
    assert!(r.body.contains("&quot;&gt;&lt;script&gt;"));
    s.stop().await;
}

#[tokio::test]
async fn onboarding_rate_limits_logins() {
    let s = TestServer::start(Signup::Closed).await;
    s.add_user("ada@example.com", "correct horse");
    for _ in 0..5 {
        let r = s
            .post_form(
                "/onboarding",
                &[],
                &[("email", "ada@example.com"), ("password", "nope")],
            )
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    // Even the right password is refused once the budget is spent.
    let r = s
        .post_form(
            "/onboarding",
            &[],
            &[("email", "ada@example.com"), ("password", "correct horse")],
        )
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(identity_result(&r.body).is_none());
    s.stop().await;
}

/// Behind a trusted proxy (the test peer is loopback), each client has its own budget, keyed by
/// the rightmost `X-Forwarded-For` hop that is not a trusted proxy.
#[tokio::test]
async fn onboarding_rate_limits_by_forwarded_client() {
    let s = TestServer::start(Signup::Closed).await;
    let login = |xff: &'static str, email: &'static str| {
        let s = &s;
        async move {
            s.post_form(
                "/onboarding",
                &[("x-forwarded-for", xff)],
                &[("email", email), ("password", "nope")],
            )
            .await
            .status
        }
    };
    for i in 0..5 {
        let email = ["a@x.test", "b@x.test", "c@x.test", "d@x.test", "e@x.test"][i];
        assert_eq!(login("203.0.113.9", email).await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        login("203.0.113.9", "f@x.test").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // A client cannot escape by prepending its own entry, nor by passing through a trusted hop.
    assert_eq!(
        login("198.51.100.1, 203.0.113.9", "f@x.test").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        login("203.0.113.9, 127.0.0.1", "f@x.test").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another client behind the same proxy is unaffected, and so is one in another IPv6 /64.
    assert_eq!(
        login("198.51.100.1", "f@x.test").await,
        StatusCode::UNAUTHORIZED
    );
    for i in 0..5 {
        let email = ["g@x.test", "h@x.test", "i@x.test", "j@x.test", "k@x.test"][i];
        assert_eq!(
            login("2001:db8:1:1::1", email).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        login("2001:db8:1:1:ffff::2", "l@x.test").await,
        StatusCode::TOO_MANY_REQUESTS,
        "same /64"
    );
    assert_eq!(
        login("2001:db8:1:2::1", "l@x.test").await,
        StatusCode::UNAUTHORIZED
    );
    s.stop().await;
}

#[tokio::test]
async fn onboarding_rejects_oversized_email_before_rate_limiting() {
    let s = TestServer::start(Signup::Closed).await;
    let long = format!("{}@example.com", "a".repeat(300));
    for _ in 0..6 {
        let r = s
            .post_form(
                "/onboarding",
                &[],
                &[("email", long.as_str()), ("password", "nope")],
            )
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
    }
    // No budget was spent.
    let r = s
        .post_form(
            "/onboarding",
            &[],
            &[("email", "ada@example.com"), ("password", "nope")],
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    s.stop().await;
}

#[tokio::test]
async fn onboarding_skip() {
    let s = TestServer::start(Signup::Closed).await;
    let page = s.get("/onboarding", &[]).await;
    assert!(
        !page.body.contains("onboarding/skip"),
        "no skip link without skipSupported"
    );
    let r = s.get("/onboarding/skip", &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_eq!(
        identity_result(&r.body).unwrap(),
        serde_json::json!({"skipped": true})
    );
    s.stop().await;
}

#[tokio::test]
async fn signup_closed_by_default() {
    let s = TestServer::start(Signup::Closed).await;
    let r = s
        .post_form(
            "/onboarding",
            &[],
            &[
                ("action", "signup"),
                ("email", "new@example.com"),
                ("password", "password1"),
            ],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(identity_result(&r.body).is_none());
    assert!(s.repo.list_users().unwrap().is_empty());
    s.stop().await;
}

#[tokio::test]
async fn signup_open() {
    let s = TestServer::start(Signup::Open).await;
    let page = s.get("/onboarding?skipSupported=true", &[]).await;
    assert!(page.body.contains("Create account"));

    let signup = |email: &'static str, password: &'static str| {
        let s = &s;
        async move {
            s.post_form(
                "/onboarding",
                &[],
                &[
                    ("action", "signup"),
                    ("first_name", "Zoë"),
                    ("last_name", "Ñúñez"),
                    ("email", email),
                    ("password", password),
                ],
            )
            .await
        }
    };
    let r = signup("zoe@example.com", "short").await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body.contains("at least 8 characters"));

    let r = signup("zoe@example.com", "long enough").await;
    assert_eq!(r.status, StatusCode::OK);
    // Non-ASCII names survive the atob round trip as \u escapes.
    let id = identity_result(&r.body).unwrap();
    assert_eq!(id["firstName"], "Zoë");
    assert_eq!(id["lastName"], "Ñúñez");
    assert_eq!(id["emailAddress"], "zoe@example.com");
    let token = id["token"].as_str().unwrap();
    let me = s.get("/api/me", &[("authorization", &basic(token))]).await;
    assert_eq!(me.json()["id"], id["id"]);

    let r = signup("ZOE@example.com", "long enough").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(s.repo.list_users().unwrap().len(), 1);
    s.stop().await;
}

#[tokio::test]
async fn api_auth() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    let token = s.add_token(&user);

    let r = s.get("/api/me", &[]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(
        r.header("www-authenticate").is_none(),
        "would make Electron prompt"
    );

    for bad in [
        basic("tern_unknown"),
        "Basic !!!".to_owned(),
        format!("Token {token}"),
        basic(""),
    ] {
        let r = s.get("/api/me", &[("authorization", &bad)]).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{bad}");
    }

    let r = s
        .get("/api/me", &[("authorization", &format!("Bearer {token}"))])
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["id"], user.id.as_str());
    // Mailspring's "Basic" plan would turn Pro features off in an unmodified client.
    assert_eq!(r.json()["stripePlanEffective"], "Tern");
    // Nothing implemented yet, so nothing advertised.
    assert_eq!(
        r.json()["tern"]["features"],
        serde_json::json!({"tracking": false, "sharing": false})
    );

    // last_used_at is recorded (in the background).
    for _ in 0..50 {
        if s.repo.list_tokens(None).unwrap()[0].last_used_at.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(s.repo.list_tokens(None).unwrap()[0].last_used_at.is_some());

    s.repo.set_user_disabled(&user.id, true, 1).unwrap();
    let r = s.get("/api/me", &[("authorization", &basic(&token))]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    s.repo.set_user_disabled(&user.id, false, 1).unwrap();
    assert_eq!(
        s.get("/api/me", &[("authorization", &basic(&token))])
            .await
            .status,
        StatusCode::OK
    );

    let id = s.repo.list_tokens(None).unwrap()[0].id;
    s.repo.revoke_token(id, 2).unwrap();
    let r = s.get("/api/me", &[("authorization", &basic(&token))]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    s.stop().await;
}

#[tokio::test]
async fn stubs() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    let auth = basic(&s.add_token(&user));
    let h = [
        ("authorization", auth.as_str()),
        ("content-type", "application/json"),
    ];

    let r = s
        .request(
            Method::POST,
            "/api/feature_usage_event",
            &h,
            r#"{"feature":"snooze"}"#,
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = s
        .request(
            Method::POST,
            "/api/feature_usage_event",
            &[],
            r#"{"feature":"snooze"}"#,
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    for path in [
        "/api/resolve-dav-hosts",
        "/api/login-link",
        "/api/newsletter/subscribe",
        "/api/translate",
        "/api/grammar/check",
        "/api/info-for-email-v2/a@b.c",
        "/api/save-public-asset",
        "/nope",
    ] {
        for method in [Method::GET, Method::POST] {
            let r = s.request(method.clone(), path, &h, "{}").await;
            assert_eq!(r.status, StatusCode::NOT_FOUND, "{method} {path}");
        }
    }
    s.stop().await;
}

#[tokio::test]
async fn delete_account() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    let other = s.add_user("bob@example.com", "correct horse");
    let auth = basic(&s.add_token(&user));
    s.repo.ensure_account(&user.id, "0a1b2c3d", 1).unwrap();
    s.repo.ensure_account(&other.id, "0a1b2c3d", 1).unwrap();

    let r = s
        .request(Method::DELETE, "/tern/v1/accounts/0a1b2c3d", &[], "")
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    let h = [("authorization", auth.as_str())];
    let r = s
        .request(Method::DELETE, "/tern/v1/accounts/0a1b2c3d", &h, "")
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(
        !s.repo.delete_account(&user.id, "0a1b2c3d").unwrap(),
        "already gone"
    );
    assert!(
        s.repo.delete_account(&other.id, "0a1b2c3d").unwrap(),
        "other user's account kept"
    );

    // Idempotent.
    let r = s
        .request(Method::DELETE, "/tern/v1/accounts/0a1b2c3d", &h, "")
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = s
        .request(Method::DELETE, "/tern/v1/accounts/a%20b", &h, "")
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    s.stop().await;
}

#[tokio::test]
async fn cors() {
    let s = TestServer::start(Signup::Closed).await;
    for origin in ["file://", "null"] {
        let r = s
            .request(
                Method::OPTIONS,
                "/api/me",
                &[
                    ("origin", origin),
                    ("access-control-request-method", "GET"),
                    (
                        "access-control-request-headers",
                        "authorization,content-type",
                    ),
                ],
                "",
            )
            .await;
        assert!(r.status.is_success(), "{origin}: {}", r.status);
        assert_eq!(r.header("access-control-allow-origin"), Some(origin));
        assert_eq!(r.header("access-control-allow-credentials"), Some("true"));
        let allowed = r
            .header("access-control-allow-headers")
            .unwrap()
            .to_ascii_lowercase();
        assert!(allowed.contains("authorization") && allowed.contains("content-type"));
        let methods = r.header("access-control-allow-methods").unwrap();
        assert!(methods.contains("GET") && methods.contains("POST") && methods.contains("DELETE"));
    }

    // Errors carry the headers too, or the renderer cannot read the 401.
    let r = s.get("/api/me", &[("origin", "null")]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.header("access-control-allow-origin"), Some("null"));

    for origin in ["https://evil.example", "http://localhost:3000"] {
        let r = s
            .request(
                Method::OPTIONS,
                "/api/me",
                &[("origin", origin), ("access-control-request-method", "GET")],
                "",
            )
            .await;
        assert!(
            r.header("access-control-allow-origin").is_none(),
            "{origin}"
        );
        let r = s.get("/ping", &[("origin", origin)]).await;
        assert!(
            r.header("access-control-allow-origin").is_none(),
            "{origin}"
        );
    }
    s.stop().await;
}

#[tokio::test]
async fn body_limit() {
    let s = TestServer::start(Signup::Closed).await;
    let big = "x".repeat(tern_backend::wire::MAX_VALUE_BYTES + 1);
    let r = s
        .request(
            Method::POST,
            "/onboarding",
            &[("content-type", "application/x-www-form-urlencoded")],
            format!("email=a&password={big}"),
        )
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    s.stop().await;
}
