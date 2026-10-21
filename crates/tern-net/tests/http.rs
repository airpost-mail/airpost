// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

mod common;

use std::time::Duration;

use common::{closed_port_url, MockServer, Reply};
use serde_json::json;
use tern_net::HttpClient;

fn sync_err(e: tern_core::Error) -> tern_core::SyncError {
    e.as_sync().cloned().expect("a SyncError")
}

#[test]
fn json_request_shape_and_body() {
    let server = MockServer::start(|req| match req.path() {
        "/ok" => Reply::json(200, json!({"hello": "world"})),
        "/html" => Reply::text(200, "<html>portal</html>"),
        _ => Reply::json(201, json!({"created": true})),
    });
    let http = HttpClient::new().unwrap();

    let v = http
        .json("GET", &format!("{}/ok", server.url()), Some("tok"), None)
        .unwrap();
    assert_eq!(v, json!({"hello": "world"}));

    let v = http
        .json(
            "POST",
            &format!("{}/create", server.url()),
            None,
            Some(&json!({"a": 1})),
        )
        .unwrap();
    assert_eq!(v, json!({"created": true}));

    let v = http
        .json("GET", &format!("{}/html", server.url()), None, None)
        .unwrap();
    assert_eq!(v, json!({"text": "<html>portal</html>"}));

    let reqs = server.requests();
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].header("authorization"), Some("Bearer tok"));
    assert_eq!(reqs[0].header("accept"), Some("application/json"));
    assert_eq!(reqs[0].header("content-type"), None);
    assert_eq!(reqs[1].method, "POST");
    assert_eq!(reqs[1].header("authorization"), None);
    assert_eq!(reqs[1].header("content-type"), Some("application/json"));
    assert_eq!(reqs[1].json(), json!({"a": 1}));
}

#[test]
fn http_status_errors() {
    let server = MockServer::start(|req| match req.path() {
        "/401" => Reply::json(401, json!({"error": "unauthorized"})),
        "/403" => Reply::json(403, json!({})),
        "/grant" => Reply::json(400, json!({"error": "invalid_grant"})),
        "/redirect" => Reply::redirect(302, "/elsewhere"),
        _ => Reply::text(500, "boom"),
    });
    let http = HttpClient::new().unwrap();
    let url = |p: &str| format!("{}{p}", server.url());

    let e = sync_err(http.json("GET", &url("/500"), None, None).unwrap_err());
    assert_eq!(e.key, "Invalid Response Code: 500");
    assert_eq!(e.debuginfo, format!("{} RETURNED boom", url("/500")));
    assert!(e.retryable && !e.offline);

    for p in ["/401", "/403", "/grant"] {
        let e = sync_err(http.json("GET", &url(p), None, None).unwrap_err());
        assert!(!e.retryable && !e.offline, "{p}");
    }

    // Redirects are not followed, so the 302 itself is the answer.
    let e = sync_err(http.json("GET", &url("/redirect"), None, None).unwrap_err());
    assert_eq!(e.key, "Invalid Response Code: 302");
    assert_eq!(server.count(), 5);
}

#[test]
fn transport_errors_are_offline() {
    let http = HttpClient::new().unwrap();

    let closed = closed_port_url();
    let e = sync_err(http.json("GET", &closed, None, None).unwrap_err());
    assert_eq!(e.key, "Could not connect to server");
    assert!(e.retryable && e.offline);
    assert_eq!(e.debuginfo, closed);

    let e = sync_err(
        http.json("GET", "http://nonexistent-host.invalid/x", None, None)
            .unwrap_err(),
    );
    assert_eq!(e.key, "Could not resolve hostname");
    assert!(e.retryable && e.offline);
    assert_eq!(e.debuginfo, "http://nonexistent-host.invalid/x");
}

#[test]
fn timeouts_are_offline() {
    let server = MockServer::start(|_| Reply::json(200, json!({})).delayed(Duration::from_secs(3)));
    let http =
        HttpClient::with_timeouts(Duration::from_secs(5), Duration::from_millis(300)).unwrap();
    let e = sync_err(http.json("GET", &server.url(), None, None).unwrap_err());
    assert_eq!(e.key, "Timeout was reached");
    assert!(e.retryable && e.offline);
}

#[test]
fn tls_failure_is_offline() {
    let server = MockServer::start(|_| Reply::json(200, json!({})));
    let http = HttpClient::new().unwrap();
    let url = server.url().replace("http://", "https://");
    let e = sync_err(http.json("GET", &url, None, None).unwrap_err());
    assert_eq!(e.key, "SSL connect error");
    assert!(e.retryable && e.offline);
}

#[test]
fn bad_urls_are_fatal() {
    let http = HttpClient::new().unwrap();
    let e = sync_err(http.json("GET", "not a url", None, None).unwrap_err());
    assert_eq!(e.key, "URL using bad/illegal format or missing URL");
    assert!(!e.retryable && !e.offline);

    let e = sync_err(
        http.json("GET", "ftp://example.test/", None, None)
            .unwrap_err(),
    );
    assert_eq!(e.key, "Unsupported protocol");
    assert!(!e.retryable);
}

#[test]
fn post_form_encodes() {
    let server = MockServer::start(|_| Reply::json(200, json!({"ok": 1})));
    let http = HttpClient::new().unwrap();
    http.post_form(&server.url(), &[("a", "b c"), ("d", "e&f")])
        .unwrap();
    let r = &server.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(r.form_value("a").as_deref(), Some("b c"));
    assert_eq!(r.form_value("d").as_deref(), Some("e&f"));
}

#[test]
fn expected_redirect() {
    let server = MockServer::start(|req| match req.path() {
        "/.well-known/carddav" => Reply::redirect(301, "/dav/"),
        "/.well-known/caldav" => Reply::redirect(308, "https://cal.example.test/root/"),
        "/temporary" => Reply::redirect(307, "/t"),
        "/see-other" => Reply::redirect(303, "/s"),
        _ => Reply::json(200, json!({})),
    });
    let http = HttpClient::new().unwrap();
    let url = |p: &str| format!("{}{p}", server.url());

    assert_eq!(
        http.expected_redirect(&url("/.well-known/carddav")),
        url("/dav/")
    );
    assert_eq!(
        http.expected_redirect(&url("/.well-known/caldav")),
        "https://cal.example.test/root/"
    );
    assert_eq!(http.expected_redirect(&url("/temporary")), url("/t"));
    assert_eq!(http.expected_redirect(&url("/see-other")), url("/s"));
    assert_eq!(http.expected_redirect(&url("/plain")), "");
    assert_eq!(http.expected_redirect(&closed_port_url()), "");
}

#[test]
fn probe_reports_any_status() {
    let server = MockServer::start(|_| Reply::text(404, "nope"));
    let http = HttpClient::new().unwrap();
    assert_eq!(http.probe(&server.url()), Ok(404));
    let err = http.probe(&closed_port_url()).unwrap_err();
    assert!(err.starts_with("Could not connect to server"), "{err}");
}
