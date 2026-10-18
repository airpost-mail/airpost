// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Notifications the engine sends one connection only, through the relay against a fake
//! engine (docs/ai-plan.md A7 and section 3, docs/web-gateway-plan.md G2): `ai.delta` and
//! `ai.triage.progress` reach only the WebSocket whose engine connection asked for them,
//! every one and in order under a burst far past the browser frame limit, and closing a
//! WebSocket closes its engine connection, which is how the engine cancels the provider
//! request. The fake answers the connection that asked and nothing else, as the engine does.

#![cfg(unix)]

use std::net::SocketAddr;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use ms_ipc::policy;
use serde_json::{json, Value};
use tern_gateway::config::{Config, ConfigInput};
use tern_gateway::http::{now_ms, AppState, Server};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(20);
/// Far past the gateway's limit on browser frames (50 a second, burst 200).
const BURST: usize = 5000;

#[derive(Debug)]
enum Event {
    /// A connection's `hello` and the `client` the gateway sent.
    Hello(usize, String),
    Request(usize, String, Value),
    Closed(usize),
}

fn line(v: Value) -> Vec<u8> {
    let mut b = serde_json::to_vec(&v).unwrap();
    b.push(b'\n');
    b
}

fn reply(id: &Value, result: Value) -> Vec<u8> {
    line(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn notify(method: &str, params: Value) -> Vec<u8> {
    line(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
}

fn delta(stream: &str, i: usize) -> Vec<u8> {
    notify(
        "ai.delta",
        json!({ "streamId": stream, "text": format!("{i} ") }),
    )
}

async fn fake_engine(listener: UnixListener, events: mpsc::UnboundedSender<Event>) {
    let mut n = 0;
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(connection(n, stream, events.clone()));
        n += 1;
    }
}

/// One engine connection. `ai.summarize {streamId, count}` sends `count` deltas in one write
/// and then the reply; with `hold: true` it sends a delta every 10 ms until the connection
/// ends. `ai.triage.run {count}` replies with a job id, then sends `count` progress
/// notifications.
async fn connection(n: usize, stream: UnixStream, events: mpsc::UnboundedSender<Event>) {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut held: Option<(String, usize)> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        let out = tokio::select! {
            l = lines.next_line() => {
                let Ok(Some(l)) = l else { break };
                let req: Value = serde_json::from_str(&l).unwrap();
                let (id, p) = (req["id"].clone(), req["params"].clone());
                let method = req["method"].as_str().unwrap_or_default().to_owned();
                let stream = p["streamId"].as_str().unwrap_or_default().to_owned();
                let count = p["count"].as_u64().unwrap_or(0) as usize;
                let out = match method.as_str() {
                    "hello" => {
                        let client = p["client"].as_str().unwrap_or_default().to_owned();
                        let _ = events.send(Event::Hello(n, client));
                        reply(&id, json!({ "protocol": 1, "engine": "fake",
                                           "pid": std::process::id(), "accounts": [],
                                           "features": ["relayed"] }))
                    }
                    "ai.summarize" if p["hold"] == true => {
                        held = Some((stream, 0));
                        Vec::new()
                    }
                    "ai.summarize" => {
                        let mut out: Vec<u8> = (0..count).flat_map(|i| delta(&stream, i)).collect();
                        let text: String = (0..count).map(|i| format!("{i} ")).collect();
                        out.extend(reply(&id, json!({ "text": text, "model": "fake", "cached": false })));
                        out
                    }
                    "ai.triage.run" => {
                        let job = format!("job-{n}");
                        let mut out = reply(&id, json!({ "jobId": job, "queued": count }));
                        for done in 1..=count {
                            out.extend(notify("ai.triage.progress",
                                              json!({ "jobId": job, "done": done, "total": count })));
                        }
                        out
                    }
                    _ => reply(&id, json!({})),
                };
                let _ = events.send(Event::Request(n, method, p));
                out
            }
            _ = tick.tick(), if held.is_some() => {
                let (stream, i) = held.as_mut().unwrap();
                *i += 1;
                delta(stream, *i - 1)
            }
        };
        if w.write_all(&out).await.is_err() {
            break;
        }
    }
    let _ = events.send(Event::Closed(n));
}

struct Setup {
    addr: SocketAddr,
    state: AppState,
    events: mpsc::UnboundedReceiver<Event>,
    log: Vec<Event>,
    _dir: tempfile::TempDir,
}

impl Setup {
    async fn start() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        for d in ["s", "run"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let socket = dir.path().join("s").join("e.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (tx, events) = mpsc::unbounded_channel();
        tokio::spawn(fake_engine(listener, tx));
        let entry = json!({ "pid": std::process::id(), "socket": socket, "protocol": 1 });
        std::fs::write(
            dir.path().join("run").join("engine.json"),
            entry.to_string(),
        )
        .unwrap();

        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let config = Config::new(ConfigInput {
            bind: addr,
            public_url: None,
            insecure_public_url: false,
            allow_origins: &[],
            trusted_proxies: &[],
            web_root: None,
            config_dir: dir.path().to_owned(),
        })
        .unwrap();
        let state = AppState::new(config).unwrap();
        tokio::spawn(Server::with_listener(tcp, state.clone()).run(std::future::pending()));
        Setup {
            addr,
            state,
            events,
            log: Vec::new(),
            _dir: dir,
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Pairs a device and returns its `Cookie` header.
    async fn pair(&self, admin: bool) -> String {
        let code = self
            .state
            .devices()
            .mint_code(None, admin, 600_000, now_ms())
            .unwrap();
        self.state.reload().unwrap();
        let body = format!(r#"{{"code":"{code}"}}"#);
        let mut s = TcpStream::connect(self.addr).await.unwrap();
        let req = format!(
            "POST /gateway/v1/pair HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nConnection: close\r\n\
             Content-Length: {}\r\n\r\n{body}",
            self.addr,
            self.origin(),
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut res = String::new();
        s.read_to_string(&mut res).await.unwrap();
        let set = res
            .lines()
            .find_map(|l| l.strip_prefix("set-cookie: "))
            .unwrap_or_else(|| panic!("{res}"));
        set.split(';').next().unwrap().to_owned()
    }

    /// A WebSocket past `hello` as `tern-web/<tag>`, and the fake's index for its engine
    /// connection.
    async fn ws(&mut self, cookie: &str, tag: &str) -> (Ws, usize) {
        let mut req = format!("ws://{}/gateway/v1/ws", self.addr)
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("Origin", self.origin().parse().unwrap());
        req.headers_mut().insert("Cookie", cookie.parse().unwrap());
        let mut ws = Ws(tokio_tungstenite::connect_async(req).await.unwrap().0);
        let hello = ws
            .call(
                1,
                "hello",
                json!({ "protocol": 1, "client": format!("tern-web/{tag}") }),
            )
            .await;
        assert!(hello.get("result").is_some(), "{hello}");
        let suffix = format!("(tern-web/{tag})");
        let found = self
            .wait_for(
                |e| matches!(e, Event::Hello(_, c) if c.ends_with(&suffix)),
                WAIT,
            )
            .await;
        assert!(found, "no hello from {tag}");
        let n = self
            .log
            .iter()
            .find_map(|e| match e {
                Event::Hello(n, c) if c.ends_with(&suffix) => Some(*n),
                _ => None,
            })
            .unwrap();
        (ws, n)
    }

    /// Whether an event matching `pred` arrives (or arrived) within `wait`.
    async fn wait_for(&mut self, mut pred: impl FnMut(&Event) -> bool, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        loop {
            if self.log.iter().any(&mut pred) {
                return true;
            }
            match tokio::time::timeout_at(deadline, self.events.recv()).await {
                Ok(Some(e)) => self.log.push(e),
                _ => return false,
            }
        }
    }

    fn drain(&mut self) {
        while let Ok(e) = self.events.try_recv() {
            self.log.push(e);
        }
    }
}

struct Ws(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>);

impl Ws {
    async fn send(&mut self, id: i64, method: &str, params: Value) {
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.0
            .send(Message::Text(req.to_string().into()))
            .await
            .unwrap();
    }

    /// The next message. Panics on a close or a timeout.
    async fn next(&mut self) -> Value {
        loop {
            match tokio::time::timeout(WAIT, self.0.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(&t).unwrap(),
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
                other => panic!("expected a message, got {other:?}"),
            }
        }
    }

    /// Sends a request and returns its response, which must be the next message: nothing
    /// meant for another connection arrives in between.
    async fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(id, method, params).await;
        let v = self.next().await;
        assert_eq!(v["id"], id, "expected the response to {method}, got {v}");
        v
    }

    /// The `ai.delta` texts before the response to `id`, each checked to be for `stream`.
    async fn stream(&mut self, id: i64, stream: &str) -> (Vec<String>, Value) {
        let mut texts = Vec::new();
        loop {
            let v = self.next().await;
            if v["id"] == id {
                return (texts, v);
            }
            assert_eq!(v["method"], "ai.delta", "{v}");
            assert_eq!(v["params"]["streamId"], stream, "{v}");
            texts.push(v["params"]["text"].as_str().unwrap().to_owned());
        }
    }
}

/// Until AI2 opens the `ai.*` methods to relayed devices (docs/ai-plan.md section 3) they are
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

/// Every `ai.*` method of docs/ai-plan.md section 3. Until stages 3 and 11 each can carry the
/// provider config, so the relayed `config` refusal has to cover all of them.
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
