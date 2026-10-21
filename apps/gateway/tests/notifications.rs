// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Notifications the engine sends one connection only, through the relay against a fake
//! engine: `ai.delta` and
//! `ai.triage.progress` reach only the WebSocket whose engine connection asked for them,
//! every one and in order under a burst far past the browser frame limit, and closing a
//! WebSocket closes its engine connection, which is how the engine cancels the provider
//! request.

#![cfg(unix)]

mod common;

use common::{Event, Setup, WAIT};
use ms_ipc::policy;
use serde_json::json;

/// Far past the gateway's limit on browser frames (50 a second, burst 200).
const BURST: usize = 5000;

/// Until AI2 opens the `ai.*` methods to relayed devices they are
/// local-only, so only an admin device's request reaches the engine.
fn ai_needs_admin() -> bool {
    !policy::relayed_method_allowed("ai.summarize", &[])
}

#[tokio::test(flavor = "multi_thread")]
async fn ai_notifications_reach_only_the_websocket_that_asked() {
    let mut s = Setup::start().await;
    let device_a = s.pair(ai_needs_admin()).await;
    let device_b = s.pair(ai_needs_admin()).await;
    let (mut a1, _) = s.ws(&device_a, "a1").await;
    let (mut a2, _) = s.ws(&device_a, "a2").await;
    let (mut b, _) = s.ws(&device_b, "b").await;

    // Two tabs of one device stream at once.
    for (ws, stream) in [(&mut a1, "s-a1"), (&mut a2, "s-a2")] {
        let params = json!({ "accountId": "acc", "threadId": "t1", "streamId": stream,
                             "count": BURST });
        ws.send(2, "ai.summarize", params).await;
    }
    let want: Vec<String> = (0..BURST).map(|i| format!("{i} ")).collect();
    for (ws, stream) in [(&mut a1, "s-a1"), (&mut a2, "s-a2")] {
        let (texts, res) = ws.stream(2, stream).await;
        assert_eq!(texts, want, "{stream}: deltas lost or out of order");
        assert_eq!(res["result"]["text"], want.concat());
    }

    let run = a1
        .call(
            3,
            "ai.triage.run",
            json!({ "accountId": "acc", "sinceDays": 7, "count": 300 }),
        )
        .await;
    let job = run["result"]["jobId"].clone();
    for done in 1..=300 {
        let v = a1.next().await;
        assert_eq!(v["method"], "ai.triage.progress", "{v}");
        assert_eq!(
            v["params"],
            json!({ "jobId": job, "done": done, "total": 300 })
        );
    }

    // The other tab and the other device got none of it, and every socket is still open.
    for ws in [&mut a1, &mut a2, &mut b] {
        assert_eq!(
            ws.call(9, "engine.status", json!({})).await["result"],
            json!({})
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_websocket_closes_its_engine_connection() {
    let mut s = Setup::start().await;
    let device = s.pair(ai_needs_admin()).await;
    let (mut a, na) = s.ws(&device, "a").await;
    let (mut b, nb) = s.ws(&device, "b").await;
    let (mut c, nc) = s.ws(&device, "c").await;
    for (ws, stream) in [(&mut a, "s-a"), (&mut b, "s-b"), (&mut c, "s-c")] {
        let params = json!({ "accountId": "acc", "threadId": "t1", "streamId": stream,
                             "hold": true });
        ws.send(2, "ai.summarize", params).await;
        for _ in 0..3 {
            assert_eq!(ws.next().await["params"]["streamId"], stream);
        }
    }

    // A close frame mid-stream.
    a.0.close(None).await.unwrap();
    let closed = |n: usize| move |e: &Event| matches!(e, Event::Closed(m) if *m == n);
    assert!(
        s.wait_for(closed(na), WAIT).await,
        "a's engine connection stayed open"
    );

    // A connection dropped without one.
    drop(c);
    assert!(
        s.wait_for(closed(nc), WAIT).await,
        "c's engine connection stayed open"
    );

    // b's stream goes on.
    for _ in 0..3 {
        assert_eq!(b.next().await["params"]["streamId"], "s-b");
    }
    s.drain();
    assert!(
        !s.log.iter().any(closed(nb)),
        "b's engine connection closed"
    );
}

/// Every `ai.*` method. Each can carry the provider config, so the relayed `config` refusal has to
/// cover all of them.
const AI_METHODS: &[&str] = &[
    "ai.models",
    "ai.test",
    "ai.summarize",
    "ai.suggestReplies",
    "ai.draftReply",
    "ai.triage.run",
    "ai.triage.list",
    "ai.cancel",
    "ai.usage",
];

/// The gateway applies ms-ipc's relay policy to `ai.*` requests, params included, so the
/// `config` refusal AI2 adds there holds on both sides (G6) for every `ai.*` method. Until
/// then `ai.*` are local-only.
#[tokio::test(flavor = "multi_thread")]
async fn relayed_ai_config_follows_the_shared_policy() {
    let mut s = Setup::start().await;
    let config = json!({ "config": { "id": "p", "kind": "openai",
                                     "url": "http://192.168.1.10:8080/v1", "model": "m" } });
    let provider = json!({ "provider": "p" });
    for admin in [false, true] {
        let scopes: Vec<String> = if admin { vec!["admin".into()] } else { vec![] };
        for &method in AI_METHODS {
            if policy::method_class(method) == policy::MethodClass::Open {
                assert!(
                    !policy::relayed_request_allowed(method, &config, &scopes),
                    "a relayed {method} request may never carry config"
                );
            }
        }
        let cookie = s.pair(admin).await;
        let tag = format!("admin-{admin}");
        let (mut ws, n) = s.ws(&cookie, &tag).await;
        let mut id = 2;
        for &method in AI_METHODS {
            for params in [&config, &provider] {
                let allowed = policy::relayed_request_allowed(method, params, &scopes);
                let res = ws.call(id, method, params.clone()).await;
                id += 1;
                if allowed {
                    assert!(res.get("result").is_some(), "{method}: {res}");
                } else {
                    assert_eq!(res["error"]["code"], -32008, "{method}: {res}");
                }
                // The fake logs a request before it answers.
                s.drain();
                let reached = s.log.iter().any(|e| {
                    matches!(e, Event::Request(m, name, p)
                        if *m == n && name == method && p == params)
                });
                assert_eq!(reached, allowed, "admin {admin}: {method} {params}");
            }
        }
    }
}
