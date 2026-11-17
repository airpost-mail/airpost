// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Login, auth, server info, `/airpost/v1/me`, sign-out, unserved identity-server paths, account
//! removal and CORS against a server on an ephemeral port.

mod common;

use airpost_backend::config::Signup;
use airpost_backend::repo::Repo;
use common::{basic, login_result, TestServer, PUBLIC_URL};
use hyper::{Method, StatusCode};

const UA: &str = "Mozilla/5.0 Airpost/1.17.0 Electron/37";

#[tokio::test]
async fn ping() {
    let s = TestServer::start(Signup::Closed).await;
    let r = s.get("/ping", &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json(), serde_json::json!({}));
    s.stop().await;
}

#[tokio::test]
async fn login_returns_token_in_the_page_body() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("Ada@Example.com", "correct horse");

    let page = s
        .get(
            "/airpost/v1/login?device=Ada%27s%20laptop",
            &[("user-agent", UA)],
        )
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.header("cache-control"), Some("no-store"));
    assert_eq!(page.header("referrer-policy"), Some("no-referrer"));
    let csp = page.header("content-security-policy").unwrap();
    assert!(csp.starts_with("default-src 'none'; style-src 'unsafe-inline'; form-action 'self'"));
    assert!(page
        .header("content-type")
        .unwrap()
        .starts_with("text/html"));
    assert!(page.body.contains(r#"action="login""#));
    assert!(page.body.contains(r#"name="device" value="Ada's laptop""#));
    assert!(!page.body.contains("<script"));
    assert!(!page.body.contains("Create account"), "sign-up is closed");
    assert!(login_result(&page.body).is_none());

    let res = s
        .post_form(
            "/airpost/v1/login",
            &[("user-agent", UA)],
            &[
                ("action", "login"),
                ("device", "Ada's laptop"),
                ("email", "ada@example.com"),
                ("password", "correct horse"),
            ],
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control"), Some("no-store"));
    assert_eq!(res.header("referrer-policy"), Some("no-referrer"));
    assert!(res.header("location").is_none());
    assert!(!res.body.contains("<script"));
    let result = login_result(&res.body).expect("#airpost-login-result");
    let token = result["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("airpost_"), "{token}");
    assert_eq!(
        result,
        serde_json::json!({
            "type": "airpost-login", "version": 1, "token": token, "userId": user.id,
            "serverId": s.repo.server_id().unwrap(), "email": "ada@example.com",
        })
    );
    // The token appears once, inside the hidden element, and nowhere in plain text.
    assert!(!res.body.contains(&token));

    // The token works, and it is named after the device.
    let me = s
        .get("/airpost/v1/me", &[("authorization", &basic(&token))])
        .await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(
        me.json(),
        serde_json::json!({
            "userId": user.id, "email": "ada@example.com", "firstName": "Ada",
            "lastName": "Lovelace", "trackingNS": user.tracking_ns,
            "serverId": s.repo.server_id().unwrap(), "publicURL": PUBLIC_URL,
            "features": {"tracking": false, "sharing": false},
        })
    );
    let tokens = s.repo.list_tokens(Some(&user.id)).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].name, "Ada's laptop");

    // A second login is a second device; without `device` the User-Agent names it, and
    // without either a placeholder does. Long device names are cut to 64 chars.
    let long = "d".repeat(100);
    for (device, ua, expected) in [
        ("", UA, UA.to_owned()),
        ("", "", "unknown device".to_owned()),
        (long.as_str(), "", "d".repeat(64)),
    ] {
        let headers: Vec<(&str, &str)> = if ua.is_empty() {
            vec![]
        } else {
            vec![("user-agent", ua)]
        };
        let again = s
            .post_form(
                "/airpost/v1/login",
                &headers,
                &[
                    ("email", "ada@example.com"),
                    ("password", "correct horse"),
                    ("device", device),
                ],
            )
            .await;
        let second = login_result(&again.body).unwrap();
        assert_ne!(second["token"], token.as_str());
        assert_eq!(
            s.repo
                .list_tokens(Some(&user.id))
                .unwrap()
                .last()
                .unwrap()
                .name,
            expected
        );
    }
    s.stop().await;
}

#[tokio::test]
async fn login_rejects_bad_credentials() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    for (email, password) in [
        ("ada@example.com", "wrong horse"),
        ("nobody@example.com", "correct horse"),
        ("", ""),
    ] {
        let r = s
            .post_form(
                "/airpost/v1/login",
                &[],
                &[("email", email), ("password", password)],
            )
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{email}");
        assert_eq!(r.header("cache-control"), Some("no-store"));
        assert!(login_result(&r.body).is_none());
        assert!(r.body.contains("Incorrect email address or password."));
    }
    s.repo.set_user_disabled(&user.id, true, 1).unwrap();
    let r = s
        .post_form(
            "/airpost/v1/login",
            &[],
            &[("email", "ada@example.com"), ("password", "correct horse")],
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(s.repo.list_tokens(None).unwrap().is_empty());
    s.stop().await;
}

#[tokio::test]
async fn login_escapes_input() {
    let s = TestServer::start(Signup::Closed).await;
    let r = s
        .post_form(
            "/airpost/v1/login",
            &[],
            &[("email", "\"><script>alert(1)</script>"), ("password", "x")],
        )
        .await;
    assert!(!r.body.contains("<script>"));
    assert!(r.body.contains("&quot;&gt;&lt;script&gt;"));
    s.stop().await;
}

#[tokio::test]
async fn login_rate_limits_logins() {
    let s = TestServer::start(Signup::Closed).await;
    s.add_user("ada@example.com", "correct horse");
    for _ in 0..5 {
        let r = s
            .post_form(
                "/airpost/v1/login",
                &[],
                &[("email", "ada@example.com"), ("password", "nope")],
            )
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    // Even the right password is refused once the budget is spent.
    let r = s
        .post_form(
            "/airpost/v1/login",
            &[],
            &[("email", "ada@example.com"), ("password", "correct horse")],
        )
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(login_result(&r.body).is_none());
    s.stop().await;
}

/// Behind a trusted proxy (the test peer is loopback), each client has its own budget, keyed by
/// the rightmost `X-Forwarded-For` hop that is not a trusted proxy.
#[tokio::test]
async fn login_rate_limits_by_forwarded_client() {
    let s = TestServer::start(Signup::Closed).await;
    let login = |xff: &'static str, email: &'static str| {
        let s = &s;
        async move {
            s.post_form(
                "/airpost/v1/login",
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
async fn login_rejects_oversized_email_before_rate_limiting() {
    let s = TestServer::start(Signup::Closed).await;
    let long = format!("{}@example.com", "a".repeat(300));
    for _ in 0..6 {
        let r = s
            .post_form(
                "/airpost/v1/login",
                &[],
                &[("email", long.as_str()), ("password", "nope")],
            )
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
    }
    // No budget was spent.
    let r = s
        .post_form(
            "/airpost/v1/login",
            &[],
            &[("email", "ada@example.com"), ("password", "nope")],
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    s.stop().await;
}

#[tokio::test]
async fn server_info() {
    let s = TestServer::start(Signup::Closed).await;
    let r = s.get("/airpost/v1/server", &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.json(),
        serde_json::json!({
            "serverId": s.repo.server_id().unwrap(), "version": env!("CARGO_PKG_VERSION"),
            "publicURL": PUBLIC_URL, "signup": "closed",
            "features": {"tracking": false, "sharing": false}, "apiVersion": 1,
        })
    );
    s.stop().await;
    let s = TestServer::start(Signup::Open).await;
    assert_eq!(
        s.get("/airpost/v1/server", &[]).await.json()["signup"],
        "open"
    );
    s.stop().await;
}

#[tokio::test]
async fn signup_closed_by_default() {
    let s = TestServer::start(Signup::Closed).await;
    let r = s
        .post_form(
            "/airpost/v1/login",
            &[],
            &[
                ("action", "signup"),
                ("email", "new@example.com"),
                ("password", "password1"),
            ],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(login_result(&r.body).is_none());
    assert!(s.repo.list_users().unwrap().is_empty());
    s.stop().await;
}

#[tokio::test]
async fn signup_open() {
    let s = TestServer::start(Signup::Open).await;
    let page = s.get("/airpost/v1/login", &[]).await;
    assert!(page.body.contains("Create account"));

    let signup = |email: &'static str, password: &'static str| {
        let s = &s;
        async move {
            s.post_form(
                "/airpost/v1/login",
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
    let id = login_result(&r.body).unwrap();
    assert_eq!(id["email"], "zoe@example.com");
    let token = id["token"].as_str().unwrap();
    let me = s
        .get("/airpost/v1/me", &[("authorization", &basic(token))])
        .await;
    assert_eq!(me.json()["userId"], id["userId"]);
    assert_eq!(me.json()["firstName"], "Zoë");
    assert_eq!(me.json()["lastName"], "Ñúñez");

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

    let r = s.get("/airpost/v1/me", &[]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(
        r.header("www-authenticate").is_none(),
        "would make Electron prompt"
    );

    for bad in [
        basic("airpost_unknown"),
        "Basic !!!".to_owned(),
        format!("Token {token}"),
        basic(""),
    ] {
        let r = s.get("/airpost/v1/me", &[("authorization", &bad)]).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{bad}");
    }

    let r = s
        .get(
            "/airpost/v1/me",
            &[("authorization", &format!("Bearer {token}"))],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["userId"], user.id.as_str());
    // Nothing implemented yet, so nothing advertised.
    assert_eq!(
        r.json()["features"],
        serde_json::json!({"tracking": false, "sharing": false})
    );

    // last_used_at is recorded (in the background).
    for _ in 0..1000 {
        if s.repo.list_tokens(None).unwrap()[0].last_used_at.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(s.repo.list_tokens(None).unwrap()[0].last_used_at.is_some());

    s.repo.set_user_disabled(&user.id, true, 1).unwrap();
    let r = s
        .get("/airpost/v1/me", &[("authorization", &basic(&token))])
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    s.repo.set_user_disabled(&user.id, false, 1).unwrap();
    assert_eq!(
        s.get("/airpost/v1/me", &[("authorization", &basic(&token))])
            .await
            .status,
        StatusCode::OK
    );

    let id = s.repo.list_tokens(None).unwrap()[0].id;
    s.repo.revoke_token(id, 2).unwrap();
    let r = s
        .get("/airpost/v1/me", &[("authorization", &basic(&token))])
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    s.stop().await;
}

#[tokio::test]
async fn sign_out_revokes_only_the_calling_token() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    let laptop = basic(&s.add_token(&user));
    let phone = basic(&s.add_token(&user));
    let r = s
        .request(Method::DELETE, "/airpost/v1/tokens/current", &[], "")
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = s
        .request(
            Method::DELETE,
            "/airpost/v1/tokens/current",
            &[("authorization", &laptop)],
            "",
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let me = |auth: String| {
        let s = &s;
        async move {
            s.get("/airpost/v1/me", &[("authorization", &auth)])
                .await
                .status
        }
    };
    assert_eq!(me(laptop.clone()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me(phone).await, StatusCode::OK);
    let r = s
        .request(
            Method::DELETE,
            "/airpost/v1/tokens/current",
            &[("authorization", &laptop)],
            "",
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    s.stop().await;
}

#[tokio::test]
async fn identity_server_paths_are_not_served() {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    let auth = basic(&s.add_token(&user));
    let h = [
        ("authorization", auth.as_str()),
        ("content-type", "application/json"),
    ];
    for path in [
        "/onboarding",
        "/onboarding/skip",
        "/api/me",
        "/api/feature_usage_event",
        "/api/resolve-dav-hosts",
        "/api/login-link",
        "/api/newsletter/subscribe",
        "/api/translate",
        "/api/grammar/check",
        "/api/info-for-email-v2/a@b.c",
        "/api/save-public-asset",
        "/api/share-static-page",
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
        .request(Method::DELETE, "/airpost/v1/accounts/0a1b2c3d", &[], "")
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    let h = [("authorization", auth.as_str())];
    let r = s
        .request(Method::DELETE, "/airpost/v1/accounts/0a1b2c3d", &h, "")
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
        .request(Method::DELETE, "/airpost/v1/accounts/0a1b2c3d", &h, "")
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = s
        .request(Method::DELETE, "/airpost/v1/accounts/a%20b", &h, "")
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
                "/airpost/v1/me",
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
    let r = s.get("/airpost/v1/me", &[("origin", "null")]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.header("access-control-allow-origin"), Some("null"));

    for origin in ["https://evil.example", "http://localhost:3000"] {
        let r = s
            .request(
                Method::OPTIONS,
                "/airpost/v1/me",
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
    let big = "x".repeat(airpost_backend::wire::MAX_VALUE_BYTES + 1);
    let r = s
        .request(
            Method::POST,
            "/airpost/v1/login",
            &[("content-type", "application/x-www-form-urlencoded")],
            format!("email=a&password={big}"),
        )
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    s.stop().await;
}
