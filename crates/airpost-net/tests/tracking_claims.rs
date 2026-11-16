// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The HTTP backend's tracking target and expiration claims (docs/backend-plan.md 3.6, 3.8)
//! against a mock Airpost server: `/airpost/v1/me` read once and kept, tracking only with
//! `features.tracking`, and claims that win on 200, lose on 409 and act on anything else.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use airpost_net::{AirpostServer, Claim, HttpMetadataBackend, Me, MetadataBackend, TrackingTarget};
use common::{closed_port_url, MockServer, Reply, Request};
use serde_json::{json, Value};

const AID: &str = "acc1";
const NS: &str = "nsnsnsnsnsnsnsnsnsnsns";

fn me(tracking: bool, public_url: &str) -> Value {
    json!({
        "userId": "u1", "email": "me@example.test", "trackingNS": NS, "serverId": "s1",
        "publicURL": public_url, "features": {"tracking": tracking, "sharing": false},
    })
}

fn backend(url: &str, dir: &tempfile::TempDir) -> HttpMetadataBackend {
    let server = AirpostServer {
        url: url.to_owned(),
        token: "airpost_secret".into(),
        user_id: "u1".into(),
        server_id: "s1".into(),
    };
    HttpMetadataBackend::new(server, AID, dir.path(), Box::new(|| {})).unwrap()
}

fn me_requests(server: &MockServer) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.path() == "/airpost/v1/me")
        .count()
}

#[test]
fn tracking_comes_from_me_and_is_asked_once() {
    let server = MockServer::start(|r: &Request| match r.path() {
        "/airpost/v1/me" => Reply::json(200, me(true, "https://t.example/")),
        _ => Reply::json(404, json!({})),
    });
    let dir = tempfile::tempdir().unwrap();
    let seen: Arc<Mutex<Vec<Option<Me>>>> = Arc::default();
    let into = seen.clone();
    let b = backend(&server.url(), &dir)
        .with_me_listener(Box::new(move |m| into.lock().unwrap().push(m.cloned())));
    let want = Some(TrackingTarget {
        public_url: "https://t.example".into(),
        ns: NS.into(),
    });
    assert_eq!(b.tracking(), want);
    assert_eq!(b.tracking(), want);
    assert_eq!(me_requests(&server), 1);
    let auth = server.requests()[0]
        .header("authorization")
        .unwrap()
        .to_owned();
    assert!(auth.starts_with("Basic "));
    assert!(seen.lock().unwrap()[0]
        .as_ref()
        .is_some_and(|m| m.features.tracking));

    // binding() refreshes the same answer.
    assert_eq!(b.binding().unwrap().as_deref(), Some("s1:u1"));
    assert_eq!(me_requests(&server), 2);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn no_tracking_without_the_feature_or_a_usable_url() {
    let dir = tempfile::tempdir().unwrap();
    let off = MockServer::start(|_: &Request| Reply::json(200, me(false, "https://t.example")));
    assert_eq!(backend(&off.url(), &dir).tracking(), None);

    let plain_http =
        MockServer::start(|_: &Request| Reply::json(200, me(true, "http://t.example")));
    assert_eq!(backend(&plain_http.url(), &dir).tracking(), None);

    // No publicURL: the server URL serves (here a loopback http one).
    let fallback = MockServer::start(|_: &Request| Reply::json(200, me(true, "")));
    let url = fallback.url();
    assert_eq!(
        backend(&url, &dir).tracking().map(|t| t.public_url),
        Some(url)
    );

    assert_eq!(backend(&closed_port_url(), &dir).tracking(), None);
}

#[test]
fn a_refused_token_turns_tracking_off_and_tells_the_listener() {
    let server = MockServer::start(|_: &Request| Reply::json(401, json!({})));
    let dir = tempfile::tempdir().unwrap();
    let seen: Arc<Mutex<Vec<Option<Me>>>> = Arc::default();
    let into = seen.clone();
    let b = backend(&server.url(), &dir)
        .with_me_listener(Box::new(move |m| into.lock().unwrap().push(m.cloned())));
    assert_eq!(b.tracking(), None);
    assert!(b.is_dormant());
    assert_eq!(*seen.lock().unwrap(), vec![None]);
    // Dormant: no more requests, and claims let the device act.
    assert_eq!(b.claim("m@x", "thread-snooze", 1), Claim::Unreachable);
    assert_eq!(server.count(), 1);
}

#[test]
fn claims_win_once_and_lose_after() {
    let held: Arc<Mutex<Vec<String>>> = Arc::default();
    let first = held.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let server = MockServer::start(move |r: &Request| {
        counted.fetch_add(1, Ordering::SeqCst);
        if r.method != "POST" || !r.path().starts_with("/airpost/v1/claim/") {
            return Reply::json(404, json!({}));
        }
        let mut held = first.lock().unwrap();
        if held.iter().any(|p| p == r.path()) {
            Reply::json(409, json!({}))
        } else {
            held.push(r.path().to_owned());
            Reply::json(200, json!({}))
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let b = backend(&server.url(), &dir);
    assert_eq!(
        b.claim("<a/b@example.test>", "thread-snooze", 1_800_000_000),
        Claim::Won
    );
    assert_eq!(
        held.lock().unwrap()[0],
        "/airpost/v1/claim/acc1/%3Ca%2Fb@example.test%3E/thread-snooze/1800000000"
    );
    assert_eq!(
        b.claim("<a/b@example.test>", "thread-snooze", 1_800_000_000),
        Claim::Lost
    );
    assert!(!Claim::Lost.acts());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_claim_the_server_cant_answer_lets_the_device_act() {
    let dir = tempfile::tempdir().unwrap();
    let broken = MockServer::start(|_: &Request| Reply::json(500, json!({})));
    let claim = backend(&broken.url(), &dir).claim("m@x", "send-later", 5);
    assert_eq!(claim, Claim::Unreachable);
    assert!(claim.acts());
    assert_eq!(
        backend(&closed_port_url(), &dir).claim("m@x", "send-later", 5),
        Claim::Unreachable
    );
    // A server without the claims endpoint answers 404: act.
    let old = MockServer::start(|_: &Request| Reply::json(404, json!({})));
    assert_eq!(
        backend(&old.url(), &dir).claim("m@x", "send-later", 5),
        Claim::Unreachable
    );
}
