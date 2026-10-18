// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A fake engine on a Unix socket and a gateway in front of it, for the relay tests that
//! need an engine whose timing they control. The fake answers the connection that asked and
//! nothing else, as the engine does.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tern_gateway::config::{Config, ConfigInput, Heartbeat};
use tern_gateway::http::{now_ms, AppState, Server};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

pub const WAIT: Duration = Duration::from_secs(20);
#[derive(Debug)]
pub enum Event {
    /// A connection's `hello` and the `client` the gateway sent.
    Hello(usize, String),
    Request(usize, String, Value),
    Closed(usize),
}

pub fn line(v: Value) -> Vec<u8> {
    let mut b = serde_json::to_vec(&v).unwrap();
    b.push(b'\n');
    b
}

pub fn reply(id: &Value, result: Value) -> Vec<u8> {
    line(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

pub fn notify(method: &str, params: Value) -> Vec<u8> {
    line(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
}

/// The `i`th delta's text: its number, then `pad` bytes of filler.
pub fn delta_text(i: usize, pad: usize) -> String {
    format!("{i} {}", "x".repeat(pad))
}

pub fn delta(stream: &str, i: usize, pad: usize) -> Vec<u8> {
    notify(
        "ai.delta",
        json!({ "streamId": stream, "text": delta_text(i, pad) }),
    )
}

pub async fn fake_engine(listener: UnixListener, events: mpsc::UnboundedSender<Event>) {
    let mut n = 0;
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(connection(n, stream, events.clone()));
        n += 1;
    }
}

/// One engine connection. `ai.summarize {streamId, count, pad}` sends `count` deltas in one
/// write and then the reply (with the full text only when `pad` is 0); with `hold: true` it
/// sends a delta every 10 ms until the connection ends; with `drop: true` it sends the deltas,
/// then `disconnect` in place of the reply, and closes the connection, as the engine does with
/// a subscriber that fell behind. `ai.triage.run {count}` replies with a job id, then sends
/// `count` progress notifications.
async fn connection(n: usize, stream: UnixStream, events: mpsc::UnboundedSender<Event>) {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut held: Option<(String, usize)> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    let mut dropping = false;
    loop {
        let out = tokio::select! {
            l = lines.next_line() => {
                let Ok(Some(l)) = l else { break };
                let req: Value = serde_json::from_str(&l).unwrap();
                let (id, p) = (req["id"].clone(), req["params"].clone());
                let method = req["method"].as_str().unwrap_or_default().to_owned();
                let stream = p["streamId"].as_str().unwrap_or_default().to_owned();
                let count = p["count"].as_u64().unwrap_or(0) as usize;
                let pad = p["pad"].as_u64().unwrap_or(0) as usize;
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
                    "ai.summarize" if p["drop"] == true => {
                        dropping = true;
                        let mut out: Vec<u8> =
                            (0..count).flat_map(|i| delta(&stream, i, pad)).collect();
                        out.extend(notify("disconnect", json!({ "reason": "fell behind" })));
                        out
                    }
                    "ai.summarize" => {
                        // Off the runtime: a big burst takes a while to build in a debug build.
                        tokio::task::spawn_blocking(move || {
                            let mut out: Vec<u8> =
                                (0..count).flat_map(|i| delta(&stream, i, pad)).collect();
                            // A padded burst's full text could pass a client's frame limit.
                            let text: String = if pad == 0 {
                                (0..count).map(|i| delta_text(i, pad)).collect()
                            } else {
                                String::new()
                            };
                            let res = json!({ "text": text, "model": "fake", "cached": false });
                            out.extend(reply(&id, res));
                            out
                        })
                        .await
                        .unwrap()
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
                delta(stream, *i - 1, 0)
            }
        };
        if w.write_all(&out).await.is_err() || dropping {
            break;
        }
    }
    let _ = events.send(Event::Closed(n));
}

pub struct Setup {
    pub addr: SocketAddr,
    pub state: AppState,
    pub events: mpsc::UnboundedReceiver<Event>,
    pub log: Vec<Event>,
    _dir: tempfile::TempDir,
}

impl Setup {
    pub async fn start() -> Setup {
        Setup::start_with(Heartbeat::default()).await
    }

    pub async fn start_with(heartbeat: Heartbeat) -> Setup {
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
        let mut config = Config::new(ConfigInput {
            bind: addr,
            public_url: None,
            insecure_public_url: false,
            allow_origins: &[],
            trusted_proxies: &[],
            web_root: None,
            config_dir: dir.path().to_owned(),
        })
        .unwrap();
        config.heartbeat = heartbeat;
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

    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Pairs a device and returns its `Cookie` header.
    pub async fn pair(&self, admin: bool) -> String {
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
    pub async fn ws(&mut self, cookie: &str, tag: &str) -> (Ws, usize) {
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
        let n = self.engine_connection(tag).await;
        (ws, n)
    }

    /// The fake's index for the engine connection of the WebSocket that said `hello` as
    /// `tern-web/<tag>`.
    pub async fn engine_connection(&mut self, tag: &str) -> usize {
        let suffix = format!("(tern-web/{tag})");
        let found = self
            .wait_for(
                |e| matches!(e, Event::Hello(_, c) if c.ends_with(&suffix)),
                WAIT,
            )
            .await;
        assert!(found, "no hello from {tag}");
        self.log
            .iter()
            .find_map(|e| match e {
                Event::Hello(n, c) if c.ends_with(&suffix) => Some(*n),
                _ => None,
            })
            .unwrap()
    }

    /// Whether an event matching `pred` arrives (or arrived) within `wait`.
    pub async fn wait_for(&mut self, mut pred: impl FnMut(&Event) -> bool, wait: Duration) -> bool {
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

    pub fn drain(&mut self) {
        while let Ok(e) = self.events.try_recv() {
            self.log.push(e);
        }
    }
}

pub struct Ws(pub tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>);

impl Ws {
    pub async fn send(&mut self, id: i64, method: &str, params: Value) {
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.0
            .send(Message::Text(req.to_string().into()))
            .await
            .unwrap();
    }

    /// The next message. Panics on a close or a timeout.
    pub async fn next(&mut self) -> Value {
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
    pub async fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(id, method, params).await;
        let v = self.next().await;
        assert_eq!(v["id"], id, "expected the response to {method}, got {v}");
        v
    }

    /// The `ai.delta` texts before the response to `id`, each checked to be for `stream`.
    pub async fn stream(&mut self, id: i64, stream: &str) -> (Vec<String>, Value) {
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
