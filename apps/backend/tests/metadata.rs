// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! B1-M gate (docs/backend-plan.md 6.1, 6.2): push, backfill, head, pages, the stream and
//! claims against a server on an ephemeral port. Stream tests run on paused tokio time, and
//! read the stream the way `MetadataWorker.cpp` does ([`CppDevice`]).

mod common;

use std::collections::HashMap;
use std::time::Duration;

use airpost_backend::config::Signup;
use airpost_backend::repo::{MetadataWrite, Repo, User};
use airpost_backend::wire::{self, ObjectType};
use common::{basic, TestServer};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Method, StatusCode};
use serde_json::{json, Value};
use tokio::time::Instant;

const AID: &str = "0a1b2c3d";

struct Fixture {
    s: TestServer,
    user: User,
    auth: String,
}

async fn fixture() -> Fixture {
    let s = TestServer::start(Signup::Closed).await;
    let user = s.add_user("ada@example.com", "correct horse");
    let auth = basic(&s.add_token(&user));
    Fixture { s, user, auth }
}

fn write(object_id: &str, version: Option<u64>, value: Value) -> MetadataWrite {
    MetadataWrite {
        object_type: ObjectType::Thread,
        object_id: object_id.into(),
        plugin_id: "thread-snooze".into(),
        version,
        value,
        header_message_id: None,
    }
}

type Key = (String, String, String);

fn key(object_type: &str, object_id: &str, plugin_id: &str) -> Key {
    (object_type.into(), object_id.into(), plugin_id.into())
}

/// What `MetadataWorker.cpp` keeps for one account: the persisted cursor and, per model, the
/// metadata `upsertMetadata` accepted.
#[derive(Default)]
struct CppDevice {
    aid: String,
    cursor: String,
    models: HashMap<Key, (u32, Value)>,
    /// Records seen, applied or not, in arrival order, with the cursor that came with them.
    received: Vec<(Key, u32, String)>,
    events: Vec<String>,
    buffer: Vec<u8>,
}

impl CppDevice {
    fn new(aid: &str) -> CppDevice {
        CppDevice {
            aid: aid.into(),
            ..Default::default()
        }
    }

    /// `MetadataFromJSON` (`get<string>`, `get<uint32_t>`) and `upsertMetadata`, which keeps
    /// only greater versions. Returns the key and version.
    fn apply(&mut self, m: &Value) -> (Key, u32) {
        let s = |k: &str| {
            m[k].as_str()
                .unwrap_or_else(|| panic!("{k} must be a string: {m}"))
                .to_owned()
        };
        let k = (s("object_type"), s("object_id"), s("plugin_id"));
        assert!(
            ["message", "thread", "contact"].contains(&k.0.as_str()),
            "C++ aborts on class {}",
            k.0
        );
        assert_eq!(s("aid"), self.aid);
        let v = m["v"].as_u64().expect("v must be an unsigned integer");
        let v = u32::try_from(v).expect("v must fit uint32_t");
        if self.models.get(&k).is_none_or(|(have, _)| v > *have) {
            self.models.insert(k.clone(), (v, m["value"].clone()));
        }
        (k, v)
    }

    /// `onDeltaData`: split on `\n`, skip lines of 0 or 1 bytes (heartbeats), parse the rest.
    fn on_delta_data(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            if pos > 1 {
                let delta: Value =
                    serde_json::from_slice(&self.buffer[..pos]).unwrap_or_else(|e| {
                        panic!("invalid JSON line ({e}): {:?}", &self.buffer[..pos])
                    });
                self.on_delta(&delta);
            }
            self.buffer.drain(..=pos);
        }
    }

    /// `onDelta`.
    fn on_delta(&mut self, delta: &Value) {
        match delta["object"].as_str().expect("object") {
            "metadata" => {
                let (k, v) = self.apply(&delta["attributes"]);
                // `setDeltaCursor(delta["cursor"])` throws on anything but a string.
                let cursor = delta["cursor"]
                    .as_str()
                    .expect("cursor must be a JSON string");
                self.cursor = cursor.to_owned();
                self.received.push((k, v, cursor.to_owned()));
            }
            "backend-event" => self
                .events
                .push(delta["event"].as_str().unwrap().to_owned()),
            other => panic!("unexpected delta type {other}"),
        }
    }

    fn get(&self, object_type: &str, object_id: &str, plugin_id: &str) -> Option<&(u32, Value)> {
        self.models.get(&key(object_type, object_id, plugin_id))
    }

    /// `fetchDeltaCursor` then `fetchMetadata` pages, as `run()` does without a cursor. C++
    /// sends the body `null` with `Content-Type: application/json` on these GETs.
    async fn initial_sync(&mut self, s: &TestServer, auth: &str) {
        let h = [
            ("authorization", auth),
            ("accept", "application/json"),
            ("content-type", "application/json"),
        ];
        let r = s
            .request(
                Method::GET,
                &format!("/deltas/{}/head", self.aid),
                &h,
                "null",
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        self.cursor = r.json()["cursor"]
            .as_str()
            .expect("head cursor is a string")
            .to_owned();
        for page in 0.. {
            let path = format!("/metadata/{}?limit=500&offset={}", self.aid, 500 * page);
            let r = s.request(Method::GET, &path, &h, "null").await;
            assert_eq!(r.status, StatusCode::OK, "{}", r.body);
            let rows = r.json();
            let rows = rows.as_array().expect("a page is an array");
            for m in rows {
                self.apply(m);
            }
            if rows.len() != 500 {
                break;
            }
        }
    }

    /// C++'s push: the local upsert raises the version, then `{objectType, version, value}`.
    async fn push(
        &mut self,
        s: &TestServer,
        auth: &str,
        object_type: &str,
        object_id: &str,
        plugin_id: &str,
        value: Value,
    ) -> Value {
        let k = key(object_type, object_id, plugin_id);
        let v = self.models.get(&k).map_or(1, |(v, _)| v + 1);
        self.models.insert(k, (v, value.clone()));
        let r = s
            .post_json(
                &format!("/metadata/{}/{object_id}/{plugin_id}", self.aid),
                auth,
                &json!({"objectType": object_type, "version": v, "value": value}),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        r.json()
    }
}

/// An open `/deltas/<aid>/streaming` response.
struct Stream {
    body: Incoming,
    bytes: usize,
    last_frame: Instant,
    max_gap: Duration,
}

/// The longest any stream test reads before it gives up (virtual time).
const READ_GUARD: Duration = Duration::from_secs(4000);

impl Stream {
    /// Opens the stream over an in-memory connection (see `TestServer::open_in_memory`).
    async fn open(s: &TestServer, auth: &str, aid: &str, cursor: &str) -> Stream {
        Self::from_response(Self::request(s, auth, aid, cursor).await)
    }

    /// Opens the stream over TCP, through `axum::serve`.
    async fn open_tcp(s: &TestServer, auth: &str, aid: &str, cursor: &str) -> Stream {
        let res = s
            .open(
                Method::GET,
                &Self::path(aid, cursor),
                &Self::headers(auth),
                "",
            )
            .await;
        Self::from_response(res)
    }

    fn from_response(res: hyper::Response<Incoming>) -> Stream {
        assert_eq!(res.status(), StatusCode::OK);
        let h = res.headers();
        assert_eq!(h["content-type"], "application/x-ndjson");
        assert_eq!(h["x-accel-buffering"], "no");
        assert_eq!(h["transfer-encoding"], "chunked");
        Stream {
            body: res.into_body(),
            bytes: 0,
            last_frame: Instant::now(),
            max_gap: Duration::ZERO,
        }
    }

    /// The C++ request line: `p`, `ih` (curl-escaped IMAP host) and `cursor`.
    fn path(aid: &str, cursor: &str) -> String {
        format!("/deltas/{aid}/streaming?p=darwin&ih=imap.example.com&cursor={cursor}")
    }

    fn headers(auth: &str) -> [(&str, &str); 2] {
        [("authorization", auth), ("accept", "application/json")]
    }

    async fn request(
        s: &TestServer,
        auth: &str,
        aid: &str,
        cursor: &str,
    ) -> hyper::Response<Incoming> {
        s.open_in_memory(
            Method::GET,
            &Self::path(aid, cursor),
            &Self::headers(auth),
            "",
        )
        .await
    }

    /// One data frame into `dev`; `false` when the server ended the stream cleanly.
    async fn step(&mut self, dev: &mut CppDevice) -> bool {
        match self.body.frame().await {
            None => false,
            Some(Err(e)) => panic!("stream error: {e}"),
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    let now = Instant::now();
                    self.max_gap = self.max_gap.max(now - self.last_frame);
                    self.last_frame = now;
                    self.bytes += data.len();
                    dev.on_delta_data(&data);
                }
                true
            }
        }
    }

    async fn read_until(&mut self, dev: &mut CppDevice, done: impl Fn(&CppDevice) -> bool) {
        tokio::time::timeout(READ_GUARD, async {
            while !done(dev) {
                assert!(self.step(dev).await, "stream ended early");
            }
        })
        .await
        .expect("condition not reached");
    }

    async fn read_for(&mut self, dev: &mut CppDevice, d: Duration) {
        let _ = tokio::time::timeout(d, async {
            while self.step(dev).await {}
            panic!("stream ended early");
        })
        .await;
    }

    async fn read_to_end(&mut self, dev: &mut CppDevice) {
        tokio::time::timeout(READ_GUARD, async { while self.step(dev).await {} })
            .await
            .expect("stream did not end");
    }
}

#[tokio::test]
async fn push_applies_the_version_rule() {
    let f = fixture().await;
    let push = |path: &'static str, body: Value| {
        let (s, auth) = (&f.s, f.auth.as_str());
        async move { s.post_json(path, auth, &body).await }
    };
    let path = "/metadata/0a1b2c3d/t:abc/thread-snooze";
    let cases = [
        (json!(null), 1),
        (json!(1), 2),
        (json!(10), 10),
        (json!(0), 11),
        (json!(5), 12),
        (json!(1u64 << 40), wire::V_MAX),
    ];
    for (version, want) in cases {
        let r = push(
            path,
            json!({"objectType": "thread", "version": version, "value": {"n": 1}}),
        )
        .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        assert_eq!(r.json(), json!({ "v": want }), "version {version}");
    }

    // Without Content-Type and without `version`, as long as the body is JSON.
    let r =
        f.s.request(
            Method::POST,
            "/metadata/0a1b2c3d/m1/open-tracking",
            &[("authorization", f.auth.as_str())],
            r#"{"objectType":"message","value":{}}"#,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.json(), json!({"v": 1}));

    let rows = f.s.repo.metadata_page(&f.user.id, AID, 500, 0).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].object_id, "t:abc");
    assert_eq!(rows[0].v, wire::V_MAX);
    assert_eq!(rows[1].object_type, ObjectType::Message);
    f.s.stop().await;
}

#[tokio::test]
async fn push_validation() {
    let f = fixture().await;
    let path = "/metadata/0a1b2c3d/t:abc/thread-snooze";
    let r =
        f.s.request(
            Method::POST,
            path,
            &[],
            r#"{"objectType":"thread","value":{}}"#,
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.header("www-authenticate").is_none());

    for body in [
        json!({"objectType": "Thread", "version": 1, "value": {}}),
        json!({"objectType": "event", "version": 1, "value": {}}),
        json!({"objectType": "thread", "version": 1}),
        json!({"objectType": "thread", "version": -1, "value": {}}),
        json!({"objectType": "thread", "version": 1, "value": {}, "headerMessageId": ""}),
        json!([]),
    ] {
        let r = f.s.post_json(path, &f.auth, &body).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{body}");
    }
    let r =
        f.s.request(
            Method::POST,
            path,
            &[("authorization", f.auth.as_str())],
            "not json",
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r =
        f.s.post_json(
            "/metadata/a%20b/t:abc/thread-snooze",
            &f.auth,
            &json!({"objectType": "thread", "value": {}}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // The value limit is on the serialized value; a body past the route limit is cut off.
    let big = "x".repeat(wire::MAX_VALUE_BYTES);
    let r =
        f.s.post_json(
            path,
            &f.auth,
            &json!({"objectType": "thread", "value": big}),
        )
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    let fits = "x".repeat(wire::MAX_VALUE_BYTES - 2);
    let r =
        f.s.post_json(
            path,
            &f.auth,
            &json!({"objectType": "thread", "value": fits}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let huge = "x".repeat(wire::MAX_VALUE_BYTES * 2);
    let r =
        f.s.post_json(
            path,
            &f.auth,
            &json!({"objectType": "thread", "value": huge}),
        )
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);

    // Tombstones are values like any other.
    for tomb in [json!({}), json!([])] {
        let r =
            f.s.post_json(
                path,
                &f.auth,
                &json!({"objectType": "thread", "value": tomb}),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK);
    }
    f.s.stop().await;
}

#[tokio::test]
async fn head_and_pages_like_cpp() {
    let f = fixture().await;
    let mut dev = CppDevice::new(AID);
    dev.initial_sync(&f.s, &f.auth).await;
    assert_eq!(dev.cursor, "0");
    assert!(dev.models.is_empty());

    for i in 0..1100 {
        let value = if i % 7 == 0 {
            json!({})
        } else {
            json!({"i": i})
        };
        f.s.repo
            .push_metadata(
                &f.user.id,
                AID,
                &write(&format!("t:{i}"), Some(i % 5), value),
                1,
            )
            .unwrap();
    }
    // Another account and another user, both invisible here.
    f.s.repo
        .push_metadata(
            &f.user.id,
            "ffffffff",
            &write("t:other", None, json!({})),
            1,
        )
        .unwrap();
    let bob = f.s.add_user("bob@example.com", "correct horse");
    f.s.repo
        .push_metadata(&bob.id, AID, &write("t:bob", None, json!({})), 1)
        .unwrap();
    let head = f.s.repo.metadata_head(&f.user.id, AID).unwrap();

    let mut dev = CppDevice::new(AID);
    dev.initial_sync(&f.s, &f.auth).await;
    assert_eq!(dev.cursor, head.to_string());
    assert_eq!(dev.models.len(), 1100, "tombstones are included");
    assert_eq!(
        dev.get("thread", "t:7", "thread-snooze").unwrap().1,
        json!({})
    );
    assert_eq!(
        dev.get("thread", "t:8", "thread-snooze").unwrap(),
        &(3, json!({"i": 8}))
    );
    assert!(dev.get("thread", "t:bob", "thread-snooze").is_none());

    // Offsets stay put while rows are updated, including into tombstones.
    let h = [("authorization", f.auth.as_str())];
    let ids = |r: common::Reply| -> Vec<String> {
        r.json()
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["object_id"].as_str().unwrap().to_owned())
            .collect()
    };
    let before = ids(f.s.get("/metadata/0a1b2c3d?limit=500&offset=500", &h).await);
    for i in [3, 499, 500, 777] {
        f.s.repo
            .push_metadata(
                &f.user.id,
                AID,
                &write(&format!("t:{i}"), None, json!([])),
                2,
            )
            .unwrap();
    }
    let after = ids(f.s.get("/metadata/0a1b2c3d?limit=500&offset=500", &h).await);
    assert_eq!(before, after);
    assert_eq!(after[0], "t:500");

    // limit is capped at 500, defaults to it, and must be a number.
    assert_eq!(
        ids(f.s.get("/metadata/0a1b2c3d?limit=5000", &h).await).len(),
        500
    );
    assert_eq!(ids(f.s.get("/metadata/0a1b2c3d", &h).await).len(), 500);
    assert_eq!(
        ids(f.s.get("/metadata/0a1b2c3d?limit=10&offset=1095", &h).await).len(),
        5
    );
    assert!(ids(f.s.get("/metadata/0a1b2c3d?offset=5000", &h).await).is_empty());
    for q in ["limit=x", "offset=-1"] {
        let r = f.s.get(&format!("/metadata/0a1b2c3d?{q}"), &h).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
    }
    assert_eq!(
        f.s.get("/metadata/0a1b2c3d", &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.s.get("/deltas/0a1b2c3d/head", &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let head = f.s.repo.metadata_head(&f.user.id, AID).unwrap();
    let r = f.s.get("/deltas/0a1b2c3d/head", &h).await;
    assert_eq!(r.body, format!(r#"{{"cursor":"{head}"}}"#));
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn cpp_flow_head_pages_then_stream() {
    let f = fixture().await;
    let other_device = basic(&f.s.add_token(&f.user));
    let mut a = CppDevice::new(AID);
    a.push(
        &f.s,
        &other_device,
        "thread",
        "t:1",
        "thread-snooze",
        json!({"expiration": 1}),
    )
    .await;

    let mut b = CppDevice::new(AID);
    b.initial_sync(&f.s, &f.auth).await;
    assert_eq!(b.get("thread", "t:1", "thread-snooze").unwrap().0, 1);
    let mut stream = Stream::open_tcp(&f.s, &f.auth, AID, &b.cursor.clone()).await;

    // Another device writes; B gets it with a string cursor equal to the new head.
    let res = a
        .push(
            &f.s,
            &other_device,
            "thread",
            "t:1",
            "thread-snooze",
            json!({"expiration": 2}),
        )
        .await;
    assert_eq!(res, json!({"v": 2}));
    stream
        .read_until(&mut b, |d| {
            d.get("thread", "t:1", "thread-snooze")
                .is_some_and(|m| m.0 == 2)
        })
        .await;
    let head = f.s.repo.metadata_head(&f.user.id, AID).unwrap();
    assert_eq!(b.cursor, head.to_string());
    assert_eq!(
        b.get("thread", "t:1", "thread-snooze").unwrap().1,
        json!({"expiration": 2})
    );

    // B's own push comes back with the version it already holds and is ignored.
    b.push(
        &f.s,
        &f.auth,
        "message",
        "m:9",
        "open-tracking",
        json!({"open_count": 0}),
    )
    .await;
    stream.read_until(&mut b, |d| d.received.len() == 2).await;
    assert_eq!(b.received[1].1, 1);
    assert_eq!(b.get("message", "m:9", "open-tracking").unwrap().0, 1);

    // A deletes it; B converges on the tombstone.
    a.push(
        &f.s,
        &other_device,
        "thread",
        "t:1",
        "thread-snooze",
        json!({}),
    )
    .await;
    stream
        .read_until(&mut b, |d| {
            d.get("thread", "t:1", "thread-snooze").unwrap().0 == 3
        })
        .await;
    assert_eq!(
        b.get("thread", "t:1", "thread-snooze").unwrap().1,
        json!({})
    );
    drop(stream);
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stream_replays_from_cursor() {
    let f = fixture().await;
    let mut seqs = Vec::new();
    for i in 0..1200u64 {
        let w =
            f.s.repo
                .push_metadata(
                    &f.user.id,
                    AID,
                    &write(&format!("t:{i}"), Some(i), json!({"i": i})),
                    1,
                )
                .unwrap();
        seqs.push(w.seq);
    }
    // An update moves the row to the end of the replay.
    let moved =
        f.s.repo
            .push_metadata(&f.user.id, AID, &write("t:0", None, json!([])), 1)
            .unwrap();

    for cursor in ["", "0"] {
        let mut dev = CppDevice::new(AID);
        let mut stream = Stream::open(&f.s, &f.auth, AID, cursor).await;
        stream
            .read_until(&mut dev, |d| d.received.len() == 1200)
            .await;
        let cursors: Vec<i64> = dev.received.iter().map(|r| r.2.parse().unwrap()).collect();
        assert!(cursors.windows(2).all(|w| w[0] < w[1]), "seq order");
        assert_eq!(dev.models.len(), 1200, "each row once");
        assert_eq!(dev.received[1199].0 .1, "t:0");
        assert_eq!(dev.cursor, moved.seq.to_string());
        assert_eq!(
            dev.get("thread", "t:0", "thread-snooze").unwrap(),
            &(2, json!([]))
        );
        assert_eq!(dev.get("thread", "t:5", "thread-snooze").unwrap().0, 5);
        // Nothing else follows but heartbeats.
        stream.read_for(&mut dev, Duration::from_secs(12)).await;
        assert_eq!(dev.received.len(), 1200);
    }

    let mut dev = CppDevice::new(AID);
    let mut stream = Stream::open(&f.s, &f.auth, AID, &seqs[999].to_string()).await;
    stream
        .read_until(&mut dev, |d| d.received.len() == 201)
        .await;
    assert_eq!(dev.received[0].0 .1, "t:1000");
    stream.read_for(&mut dev, Duration::from_secs(6)).await;
    assert_eq!(dev.received.len(), 201);

    // A cursor past the account's head but not the global counter (other accounts wrote
    // since) replays nothing.
    f.s.repo
        .push_metadata(&f.user.id, "ffffffff", &write("t:x", None, json!({})), 1)
        .unwrap();
    let mut dev = CppDevice::new(AID);
    let past_head = (moved.seq + 1).to_string();
    let mut stream = Stream::open(&f.s, &f.auth, AID, &past_head).await;
    stream.read_for(&mut dev, Duration::from_secs(6)).await;
    assert!(dev.received.is_empty());

    // A cursor past the global counter predates a restore: everything is replayed.
    let mut dev = CppDevice::new(AID);
    let mut stream = Stream::open(&f.s, &f.auth, AID, "999999").await;
    stream
        .read_until(&mut dev, |d| d.received.len() == 1200)
        .await;
    assert_eq!(dev.cursor, moved.seq.to_string());
    drop(stream);
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stream_heartbeats() {
    let f = fixture().await;
    let mut dev = CppDevice::new(AID);
    let start = Instant::now();
    let mut stream = Stream::open(&f.s, &f.auth, AID, "0").await;
    stream.step(&mut dev).await;
    assert!(
        stream.last_frame - start < Duration::from_millis(100),
        "first heartbeat is immediate"
    );
    assert_eq!(stream.bytes, wire::HEARTBEAT_BYTES);

    stream.read_for(&mut dev, Duration::from_secs(61)).await;
    // 1 immediate + 12 periodic heartbeats; no gap exceeds the interval.
    assert_eq!(stream.bytes, 13 * wire::HEARTBEAT_BYTES);
    assert!(
        stream.max_gap
            <= Duration::from_secs(wire::HEARTBEAT_INTERVAL_SECS) + Duration::from_millis(100),
        "{:?}",
        stream.max_gap
    );
    assert!(dev.received.is_empty() && dev.events.is_empty());
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stream_closes_after_an_hour() {
    let f = fixture().await;
    let mut dev = CppDevice::new(AID);
    let start = Instant::now();
    let mut stream = Stream::open(&f.s, &f.auth, AID, "0").await;
    stream.read_to_end(&mut dev).await;
    let lived = Instant::now() - start;
    let max = Duration::from_secs(wire::STREAM_MAX_SECS);
    assert!(
        lived >= max && lived < max + Duration::from_secs(1),
        "{lived:?}"
    );
    assert!(stream.max_gap <= Duration::from_secs(5) + Duration::from_millis(100));
    // The slot is free again; the client reconnects with its cursor.
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(f.s.state.open_streams(&f.user.id, AID), 0);
    let res = Stream::request(&f.s, &f.auth, AID, &dev.cursor).await;
    assert_eq!(res.status(), StatusCode::OK);
    drop(res);
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stream_closes_on_revocation() {
    let f = fixture().await;
    let mut dev = CppDevice::new(AID);
    let mut stream = Stream::open(&f.s, &f.auth, AID, "0").await;
    stream.read_for(&mut dev, Duration::from_secs(7)).await;

    // As `airpost-backend token revoke` from another process would.
    let id = f.s.repo.list_tokens(Some(&f.user.id)).unwrap()[0].id;
    let revoked = Instant::now();
    assert!(f.s.repo.revoke_token(id, 1).unwrap());
    stream.read_to_end(&mut dev).await;
    let took = Instant::now() - revoked;
    assert!(
        took <= Duration::from_secs(wire::HEARTBEAT_INTERVAL_SECS),
        "{took:?}"
    );

    let res = Stream::request(&f.s, &f.auth, AID, "0").await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(res.headers().get("www-authenticate").is_none());
    drop(res);

    // Disabling the user closes its streams too.
    let token = f.s.add_token(&f.user);
    let auth = basic(&token);
    let mut stream = Stream::open(&f.s, &auth, AID, "0").await;
    f.s.repo.set_user_disabled(&f.user.id, true, 2).unwrap();
    stream.read_to_end(&mut dev).await;
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stream_limit_is_429() {
    let f = fixture().await;
    // One device per stream: a second stream of the same token replaces the first.
    let mut devices = vec![f.auth.clone()];
    while devices.len() < wire::MAX_STREAMS_PER_ACCOUNT + 1 {
        devices.push(basic(&f.s.add_token(&f.user)));
    }
    let mut streams = Vec::new();
    for auth in &devices[..wire::MAX_STREAMS_PER_ACCOUNT] {
        streams.push(Stream::open(&f.s, auth, AID, "0").await);
    }
    let fifth = &devices[wire::MAX_STREAMS_PER_ACCOUNT];
    let res = Stream::request(&f.s, fifth, AID, "0").await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty(), "429 has an empty body");

    // Other accounts and other users have their own budget.
    let other_aid = Stream::open(&f.s, fifth, "ffffffff", "0").await;
    let bob = f.s.add_user("bob@example.com", "correct horse");
    let bob_auth = basic(&f.s.add_token(&bob));
    let bobs = Stream::open(&f.s, &bob_auth, AID, "0").await;

    // A client that goes away frees its slot.
    drop(streams.pop());
    for _ in 0..100 {
        if f.s.state.open_streams(&f.user.id, AID) < wire::MAX_STREAMS_PER_ACCOUNT {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let mut again = Stream::open(&f.s, fifth, AID, "0").await;
    let mut dev = CppDevice::new(AID);
    again.step(&mut dev).await;
    drop((streams, other_aid, bobs, again));
    f.s.stop().await;
}

/// A device whose connection died without the server noticing (sleep, network switch) holds
/// its slot; when it reconnects with the same token, the new stream replaces the old one, so
/// a full account never answers 429 to a returning device.
#[tokio::test(start_paused = true)]
async fn a_reconnecting_device_replaces_its_stale_stream() {
    let f = fixture().await;
    let mut others = Vec::new();
    for _ in 1..wire::MAX_STREAMS_PER_ACCOUNT {
        let auth = basic(&f.s.add_token(&f.user));
        others.push(Stream::open(&f.s, &auth, AID, "0").await);
    }
    let mut stale = Stream::open(&f.s, &f.auth, AID, "0").await;
    let mut stale_dev = CppDevice::new(AID);
    stale.step(&mut stale_dev).await;
    // Wake up a few times without the old connections being torn down.
    let mut kept = Vec::new();
    for _ in 0..3 {
        let s = Stream::open(&f.s, &f.auth, AID, "0").await;
        assert_eq!(
            f.s.state.open_streams(&f.user.id, AID),
            wire::MAX_STREAMS_PER_ACCOUNT
        );
        kept.push(s);
    }
    // The replaced stream ends cleanly; the newest one is live and gets writes.
    stale.read_to_end(&mut stale_dev).await;
    let mut live = kept.pop().unwrap();
    for mut s in kept {
        s.read_to_end(&mut stale_dev).await;
    }
    let mut dev = CppDevice::new(AID);
    let laptop = basic(&f.s.add_token(&f.user));
    let mut writer = CppDevice::new(AID);
    writer
        .push(
            &f.s,
            &laptop,
            "thread",
            "t:1",
            "thread-snooze",
            json!({"x": 1}),
        )
        .await;
    live.read_until(&mut dev, |d| d.received.len() == 1).await;
    drop((others, live));
    f.s.stop().await;
}

/// Streams of made-up account ids are capped per token (one per account up to the account
/// limit), so one token cannot open unbounded streams.
#[tokio::test(start_paused = true)]
async fn streams_per_token_are_capped() {
    let f = fixture().await;
    let mut streams = Vec::new();
    for i in 0..wire::MAX_ACCOUNTS_PER_USER {
        streams.push(Stream::open(&f.s, &f.auth, &format!("{i:08x}"), "0").await);
    }
    let res = Stream::request(&f.s, &f.auth, "ffffffff", "0").await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    // Reconnecting to an account it already streams is still a replacement.
    let res = Stream::request(&f.s, &f.auth, "00000000", "0").await;
    assert_eq!(res.status(), StatusCode::OK);
    drop((streams, res));
    f.s.stop().await;
}

#[tokio::test]
async fn stream_rejects_bad_requests() {
    let f = fixture().await;
    for (path, auth, want) in [
        (
            "/deltas/0a1b2c3d/streaming?cursor=0",
            "",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/deltas/0a1b2c3d/streaming?cursor=abc",
            f.auth.as_str(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/deltas/0a1b2c3d/streaming?cursor=-5",
            f.auth.as_str(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/deltas/a%20b/streaming?cursor=0",
            f.auth.as_str(),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let h: Vec<(&str, &str)> = if auth.is_empty() {
            vec![]
        } else {
            vec![("authorization", auth)]
        };
        let r = f.s.get(path, &h).await;
        assert_eq!(r.status, want, "{path}");
    }
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn writes_fan_out_to_every_stream_of_the_account() {
    let f = fixture().await;
    let laptop = basic(&f.s.add_token(&f.user));
    let mut d1 = CppDevice::new(AID);
    let mut d2 = CppDevice::new(AID);
    let mut d3 = CppDevice::new("ffffffff");
    let mut s1 = Stream::open(&f.s, &f.auth, AID, "0").await;
    let mut s2 = Stream::open(&f.s, &laptop, AID, "0").await;
    let mut s3 = Stream::open(&f.s, &f.auth, "ffffffff", "0").await;

    let mut writer = CppDevice::new(AID);
    for i in 0..3 {
        writer
            .push(
                &f.s,
                &laptop,
                "thread",
                &format!("t:{i}"),
                "send-reminders",
                json!({"i": i}),
            )
            .await;
    }
    s1.read_until(&mut d1, |d| d.received.len() == 3).await;
    s2.read_until(&mut d2, |d| d.received.len() == 3).await;
    assert_eq!(d1.models, d2.models);
    assert_eq!(d1.cursor, d2.cursor);
    s3.read_for(&mut d3, Duration::from_secs(6)).await;
    assert!(d3.received.is_empty());
    drop((s1, s2, s3));
    f.s.stop().await;
}

#[tokio::test]
async fn writes_during_replay_are_not_lost() {
    let f = fixture().await;
    for i in 0..1500 {
        f.s.repo
            .push_metadata(
                &f.user.id,
                AID,
                &write(&format!("t:{i}"), None, json!({"i": i})),
                1,
            )
            .unwrap();
    }
    let writer = {
        let auth = f.auth.clone();
        let addr = f.s.url("");
        tokio::spawn(async move {
            let client =
                hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                    .build_http::<http_body_util::Full<hyper::body::Bytes>>();
            for i in 0..300 {
                let req = hyper::Request::post(format!(
                    "{addr}/metadata/{AID}/t:{}/thread-snooze",
                    i * 5
                ))
                .header("authorization", &auth)
                .body(http_body_util::Full::new(hyper::body::Bytes::from(
                    json!({"objectType": "thread", "version": null, "value": {"w": i}}).to_string(),
                )))
                .unwrap();
                let res = client.request(req).await.unwrap();
                assert_eq!(res.status(), StatusCode::OK);
            }
        })
    };
    let mut dev = CppDevice::new(AID);
    let mut stream = Stream::open(&f.s, &f.auth, AID, "0").await;
    writer.await.unwrap();
    let head = f.s.repo.metadata_head(&f.user.id, AID).unwrap().to_string();
    tokio::time::timeout(
        Duration::from_secs(60),
        stream.read_until(&mut dev, |d| d.cursor == head),
    )
    .await
    .expect("caught up");
    let server = f.s.repo.metadata_since(&f.user.id, AID, 0, 5000).unwrap();
    assert_eq!(server.len(), 1500);
    for (_, r) in server {
        let got = dev.get("thread", &r.object_id, "thread-snooze").unwrap();
        assert_eq!(got, &(r.v, r.value), "{}", r.object_id);
    }
    drop(stream);
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn account_removal_closes_its_streams() {
    let f = fixture().await;
    let mut dev = CppDevice::new(AID);
    let mut stream = Stream::open(&f.s, &f.auth, AID, "0").await;
    let mut other_dev = CppDevice::new("ffffffff");
    let mut other = Stream::open(&f.s, &f.auth, "ffffffff", "0").await;
    let r =
        f.s.request(
            Method::DELETE,
            "/airpost/v1/accounts/0a1b2c3d",
            &[("authorization", f.auth.as_str())],
            "",
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let start = Instant::now();
    stream.read_to_end(&mut dev).await;
    assert!(Instant::now() - start < Duration::from_secs(1));
    other.read_for(&mut other_dev, Duration::from_secs(6)).await;
    drop(other);
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn identity_change_sends_a_backend_event() {
    let f = fixture().await;
    let mut dev = CppDevice::new(AID);
    let mut stream = Stream::open(&f.s, &f.auth, AID, "0").await;
    stream.read_for(&mut dev, Duration::from_secs(6)).await;
    assert!(dev.events.is_empty());

    // Another process edits the user.
    let c = rusqlite::Connection::open(f.s.data_dir.join("airpost.db")).unwrap();
    c.busy_timeout(Duration::from_secs(5)).unwrap();
    c.execute(
        "UPDATE users SET first_name = 'Augusta' WHERE id = ?1",
        [&f.user.id],
    )
    .unwrap();
    drop(c);
    stream.read_until(&mut dev, |d| !d.events.is_empty()).await;
    assert_eq!(dev.events, ["identity-refresh-needed"]);
    stream.read_for(&mut dev, Duration::from_secs(11)).await;
    assert_eq!(dev.events.len(), 1, "sent once per change");
    drop(stream);
    f.s.stop().await;
}

#[tokio::test(start_paused = true)]
async fn backfill_keeps_server_values() {
    let f = fixture().await;
    let laptop = basic(&f.s.add_token(&f.user));
    let mut a = CppDevice::new(AID);
    a.push(
        &f.s,
        &laptop,
        "thread",
        "t:a",
        "thread-snooze",
        json!({"server": 1}),
    )
    .await;
    a.push(
        &f.s,
        &laptop,
        "thread",
        "t:a",
        "thread-snooze",
        json!({"server": 2}),
    )
    .await;
    a.push(
        &f.s,
        &laptop,
        "thread",
        "t:a",
        "thread-snooze",
        json!({"server": 3}),
    )
    .await;
    let head = f.s.repo.metadata_head(&f.user.id, AID).unwrap();

    let mut b = CppDevice::new(AID);
    let mut stream = Stream::open(&f.s, &f.auth, AID, &head.to_string()).await;
    let r =
        f.s.post_json(
            "/airpost/v1/backfill/0a1b2c3d",
            &f.auth,
            &json!([
                {"objectType": "thread", "objectId": "t:a", "pluginId": "thread-snooze",
                 "version": 2, "value": {"local": 1}},
                {"objectType": "message", "objectId": "m:1", "pluginId": "open-tracking",
                 "version": 4, "value": {"local": 2}, "headerMessageId": "<m1@x>"},
            ]),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.json(), json!({"written": 1, "rejected": []}));
    stream.read_until(&mut b, |d| d.received.len() == 1).await;
    assert_eq!(
        b.get("message", "m:1", "open-tracking").unwrap(),
        &(4, json!({"local": 2}))
    );

    // Same version as the server's row: the server's value comes back one version up.
    b.models.insert(
        key("thread", "t:a", "thread-snooze"),
        (3, json!({"local": 1})),
    );
    let r =
        f.s.post_json(
            "/airpost/v1/backfill/0a1b2c3d",
            &f.auth,
            &json!([{"objectType": "thread", "objectId": "t:a", "pluginId": "thread-snooze",
                     "version": 3, "value": {"local": 1}}]),
        )
        .await;
    assert_eq!(r.json(), json!({"written": 1, "rejected": []}));
    stream.read_until(&mut b, |d| d.received.len() == 2).await;
    assert_eq!(
        b.get("thread", "t:a", "thread-snooze").unwrap(),
        &(4, json!({"server": 3}))
    );

    let r =
        f.s.post_json("/airpost/v1/backfill/0a1b2c3d", &f.auth, &json!([]))
            .await;
    assert_eq!(r.json(), json!({"written": 0, "rejected": []}));
    let too_many: Vec<Value> = (0..=wire::BACKFILL_MAX_RECORDS)
        .map(|i| json!({"objectType": "thread", "objectId": format!("t:{i}"), "pluginId": "p", "value": {}}))
        .collect();
    let r =
        f.s.post_json("/airpost/v1/backfill/0a1b2c3d", &f.auth, &json!(too_many))
            .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // Invalid records are skipped and reported; the rest of the batch is stored.
    let r =
        f.s.post_json(
            "/airpost/v1/backfill/0a1b2c3d",
            &f.auth,
            &json!([
                {"objectType": "Thread", "objectId": "t:x", "pluginId": "p", "value": {}},
                {"objectType": "thread", "objectId": "t:big", "pluginId": "p",
                 "value": "x".repeat(wire::MAX_VALUE_BYTES)},
                {"objectType": "thread", "objectId": "t:ok", "pluginId": "p", "version": 1,
                 "value": {"ok": true}},
                {"objectType": "thread", "objectId": "", "pluginId": "p", "value": {}},
                "not a record",
            ]),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.json(), json!({"written": 1, "rejected": [0, 1, 3, 4]}));
    stream.read_until(&mut b, |d| d.received.len() == 3).await;
    assert!(b.get("thread", "t:ok", "p").is_some());
    let r =
        f.s.request(Method::POST, "/airpost/v1/backfill/0a1b2c3d", &[], "[]")
            .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // Nothing from the rejected batch or records was stored.
    assert_eq!(
        f.s.repo
            .metadata_page(&f.user.id, AID, 500, 0)
            .unwrap()
            .len(),
        3
    );
    drop(stream);
    f.s.stop().await;
}

#[tokio::test]
async fn expiration_claims() {
    let f = fixture().await;
    let laptop = basic(&f.s.add_token(&f.user));
    let claim = |auth: &str, path: &str| {
        let (s, auth, path) = (&f.s, auth.to_owned(), path.to_owned());
        async move {
            s.request(Method::POST, &path, &[("authorization", auth.as_str())], "")
                .await
        }
    };
    let path = "/airpost/v1/claim/0a1b2c3d/t:abc/send-reminders/1760000000";
    let r = claim(&f.auth, path).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(claim(&laptop, path).await.status, StatusCode::CONFLICT);
    assert_eq!(
        claim(&f.auth, path).await.status,
        StatusCode::OK,
        "holder may retry"
    );
    assert_eq!(
        claim(
            &laptop,
            "/airpost/v1/claim/0a1b2c3d/t:abc/send-reminders/1760000600"
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        claim(
            &laptop,
            "/airpost/v1/claim/0a1b2c3d/t:abc/thread-snooze/1760000000"
        )
        .await
        .status,
        StatusCode::OK
    );
    for bad in [
        "/airpost/v1/claim/0a1b2c3d/t:abc/send-reminders/soon",
        "/airpost/v1/claim/0a1b2c3d/t:abc/send-reminders/-1",
        "/airpost/v1/claim/a%20b/t:abc/send-reminders/1",
    ] {
        assert_eq!(
            claim(&f.auth, bad).await.status,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    assert_eq!(claim("", path).await.status, StatusCode::UNAUTHORIZED);

    let bob = f.s.add_user("bob@example.com", "correct horse");
    let bob_auth = basic(&f.s.add_token(&bob));
    assert_eq!(
        claim(&bob_auth, path).await.status,
        StatusCode::OK,
        "per user"
    );
    f.s.stop().await;
}

#[tokio::test]
async fn shutdown_ends_open_streams() {
    let f = fixture().await;
    let mut stream = Stream::open_tcp(&f.s, &f.auth, AID, "0").await;
    let mut dev = CppDevice::new(AID);
    stream.step(&mut dev).await;
    let reader = tokio::spawn(async move {
        stream.read_to_end(&mut dev).await;
    });
    tokio::time::timeout(Duration::from_secs(10), f.s.stop())
        .await
        .expect("graceful shutdown with an open stream");
    reader.await.unwrap();
}
