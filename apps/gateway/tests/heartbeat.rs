// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The WebSocket heartbeat against a fake engine (docs/web-gateway-plan.md 2.4, 2.6): a
//! browser that stops answering pings is closed with 1001 within `interval + timeout` and its
//! engine connection closes with it (which cancels its AI streams, docs/ai-plan.md A7); one
//! that answers, or keeps sending, stays open; and a slow reader working through a burst of
//! `ai.delta` isn't taken for dead while its ping waits behind the backlog. When the engine
//! drops a slow reader, what was queued for it still arrives, then a close with 4008, even if
//! the tab sends requests while the queue drains.

#![cfg(unix)]

mod common;

use std::time::Duration;

use common::{delta_text, Event, Setup, WAIT};
use futures::{SinkExt, StreamExt};
use ms_ipc::policy;
use serde_json::{json, Value};
use tern_gateway::config::Heartbeat;
use tern_gateway::http::close;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

/// Short, but not so short that a machine busy with other builds stalls a test past it.
const BEAT: Heartbeat = Heartbeat {
    interval: Duration::from_millis(300),
    timeout: Duration::from_millis(700),
};
const SILENCE: Duration = Duration::from_millis(1000);
/// Scheduling slack on a loaded machine.
const SLACK: Duration = Duration::from_millis(1500);

fn closed(n: usize) -> impl Fn(&Event) -> bool {
    move |e| matches!(e, Event::Closed(m) if *m == n)
}

/// A WebSocket client written out by hand, because a library client answers every ping it
/// reads: this one answers none, and writes only when told to.
struct RawWs {
    r: BufReader<OwnedReadHalf>,
    w: OwnedWriteHalf,
}

const OP_TEXT: u8 = 1;
const OP_CLOSE: u8 = 8;
const OP_PING: u8 = 9;

impl RawWs {
    /// Connects and sends `hello` as `tern-web/<tag>`; returns the client and the fake's
    /// index for its engine connection.
    async fn open(s: &mut Setup, cookie: &str, tag: &str) -> (RawWs, usize) {
        let stream = TcpStream::connect(s.addr).await.unwrap();
        let (r, mut w) = stream.into_split();
        let req = format!(
            "GET /gateway/v1/ws HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nCookie: {cookie}\r\n\
             Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
            s.addr,
            s.origin()
        );
        w.write_all(req.as_bytes()).await.unwrap();
        let mut r = BufReader::new(r);
        let mut status = String::new();
        r.read_line(&mut status).await.unwrap();
        assert!(status.starts_with("HTTP/1.1 101"), "{status}");
        loop {
            let mut l = String::new();
            r.read_line(&mut l).await.unwrap();
            if l == "\r\n" {
                break;
            }
        }
        let mut ws = RawWs { r, w };
        let hello = json!({ "jsonrpc": "2.0", "id": 1, "method": "hello",
                            "params": { "protocol": 1, "client": format!("tern-web/{tag}") } });
        ws.send(&hello).await;
        let (op, body) = ws.frame().await;
        assert_eq!(op, OP_TEXT);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("result").is_some(), "{v}");
        let n = s.engine_connection(tag).await;
        (ws, n)
    }

    /// One masked text frame.
    async fn send(&mut self, v: &Value) {
        let payload = v.to_string().into_bytes();
        let mut frame = vec![0x80 | OP_TEXT];
        match payload.len() {
            n if n < 126 => frame.push(0x80 | n as u8),
            n => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(n as u16).to_be_bytes());
            }
        }
        let mask = [0x12, 0x34, 0x56, 0x78];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.w.write_all(&frame).await.unwrap();
    }

    /// The next frame's opcode and payload (the gateway doesn't fragment).
    async fn frame(&mut self) -> (u8, Vec<u8>) {
        let read = async {
            let mut h = [0u8; 2];
            self.r.read_exact(&mut h).await.unwrap();
            let len = match h[1] & 0x7f {
                126 => u64::from(self.r.read_u16().await.unwrap()),
                127 => self.r.read_u64().await.unwrap(),
                n => u64::from(n),
            };
            let mut body = vec![0u8; len as usize];
            self.r.read_exact(&mut body).await.unwrap();
            (h[0] & 0x0f, body)
        };
        tokio::time::timeout(WAIT, read).await.expect("a frame")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_that_never_answers_is_closed_with_its_engine_connection() {
    let mut s = Setup::start_with(BEAT).await;
    let cookie = s
        .pair(!policy::relayed_method_allowed("ai.summarize", &[]))
        .await;
    let (mut ws, n) = RawWs::open(&mut s, &cookie, "silent").await;
    let opened = Instant::now();
    // A stream it started goes on until the gateway gives up on it.
    let params = json!({ "accountId": "acc", "threadId": "t1", "streamId": "s", "hold": true });
    ws.send(&json!({ "jsonrpc": "2.0", "id": 2, "method": "ai.summarize", "params": params }))
        .await;

    assert!(
        s.wait_for(closed(n), SILENCE + SLACK).await,
        "the engine connection stayed open"
    );
    let took = opened.elapsed();
    assert!(
        took >= SILENCE - Duration::from_millis(50),
        "closed after {took:?}"
    );

    // What the gateway sent before giving up: the stream, pings, then a close with 1001.
    let mut pings = 0;
    loop {
        match ws.frame().await {
            (OP_TEXT, _) => {}
            (OP_PING, _) => pings += 1,
            (OP_CLOSE, body) => {
                assert_eq!(u16::from_be_bytes([body[0], body[1]]), close::GOING_AWAY);
                break;
            }
            (op, _) => panic!("unexpected opcode {op}"),
        }
    }
    assert!(pings >= 1, "no ping before the close");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_that_answers_or_keeps_sending_stays_open() {
    let mut s = Setup::start_with(BEAT).await;
    let cookie = s.pair(false).await;
    let (mut ponging, np) = s.ws(&cookie, "ponging").await;
    let (mut sending, ns) = RawWs::open(&mut s, &cookie, "sending").await;

    // For three silences: one client reads (and so answers each ping), the other never
    // reads but sends a request every 100 ms.
    let until = Instant::now() + SILENCE * 3;
    let reader = async {
        let mut pings = 0;
        while let Ok(m) = tokio::time::timeout_at(until, ponging.0.next()).await {
            match m {
                Some(Ok(Message::Ping(_))) => pings += 1,
                other => panic!("expected pings only, got {other:?}"),
            }
        }
        pings
    };
    let writer = async {
        let mut id = 10;
        while Instant::now() < until {
            sending
                .send(&json!({ "jsonrpc": "2.0", "id": id, "method": "engine.status" }))
                .await;
            id += 1;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        id
    };
    let (pings, last_id) = tokio::join!(reader, writer);
    assert!(pings >= 5, "{pings} pings in three silences");

    // Both are still open, and so are their engine connections.
    assert_eq!(
        ponging.call(2, "engine.status", json!({})).await["result"],
        json!({})
    );
    let mut answered = 10;
    let mut pings = 0;
    while answered < last_id {
        match sending.frame().await {
            (OP_TEXT, body) => {
                let v: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(v["id"], answered, "{v}");
                answered += 1;
            }
            (OP_PING, _) => pings += 1,
            (op, _) => panic!("unexpected opcode {op}"),
        }
    }
    assert!(pings >= 5, "{pings} pings to the sending client");
    s.drain();
    assert!(
        !s.log.iter().any(|e| closed(np)(e) || closed(ns)(e)),
        "{:?}",
        s.log
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_reader_under_a_burst_is_not_taken_for_dead() {
    let mut s = Setup::start_with(BEAT).await;
    let cookie = s
        .pair(!policy::relayed_method_allowed("ai.summarize", &[]))
        .await;
    let (mut ws, n) = s.ws(&cookie, "slow").await;
    const COUNT: usize = 5000;
    const PAD: usize = 1000;
    let params = json!({ "accountId": "acc", "threadId": "t1", "streamId": "s",
                         "count": COUNT, "pad": PAD });
    ws.send(2, "ai.summarize", params).await;

    // About 1 MB a second, so the burst takes several silences to read, and the backlog
    // that socket buffers take ahead of a ping takes longer than a silence.
    let start = Instant::now();
    let mut i = 0;
    let mut pings = 0;
    loop {
        let m = tokio::time::timeout(WAIT, ws.0.next())
            .await
            .expect("a message");
        let t = match m {
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Ping(_))) => {
                pings += 1;
                continue;
            }
            other => panic!(
                "closed after {i} deltas and {:?}: {other:?}",
                start.elapsed()
            ),
        };
        let v: Value = serde_json::from_str(&t).unwrap();
        if v["id"] == 2 {
            break;
        }
        assert_eq!(v["params"]["text"], delta_text(i, PAD));
        i += 1;
        if i % 100 == 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    assert_eq!(i, COUNT);
    let took = start.elapsed();
    assert!(took > SILENCE * 2, "the burst took only {took:?}");
    assert!(pings >= 1, "no ping during the burst");
    assert_eq!(
        ws.call(3, "engine.status", json!({})).await["result"],
        json!({})
    );
    s.drain();
    assert!(!s.log.iter().any(closed(n)), "the engine connection closed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_slow_reader_gets_its_backlog_then_4008() {
    let mut s = Setup::start().await;
    let cookie = s
        .pair(!policy::relayed_method_allowed("ai.summarize", &[]))
        .await;
    let (mut ws, n) = s.ws(&cookie, "dropped").await;
    const COUNT: usize = 5000;
    const PAD: usize = 1000;
    let params = json!({ "accountId": "acc", "threadId": "t1", "streamId": "s",
                         "count": COUNT, "pad": PAD, "drop": true });
    ws.send(2, "ai.summarize", params).await;

    // Read slowly, and send a request every 100 deltas: some go out after the engine closed
    // its end, while the gateway still drains the backlog.
    let mut i = 0;
    let mut id = 10;
    let mut sent_after_close = 0;
    let mut disconnect = false;
    let code = loop {
        let m = tokio::time::timeout(WAIT, ws.0.next())
            .await
            .expect("a message");
        let t = match m {
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Close(Some(f)))) => break u16::from(f.code),
            other => panic!("after {i} deltas: {other:?}"),
        };
        let v: Value = serde_json::from_str(&t).unwrap();
        assert!(!disconnect, "a frame after disconnect: {v}");
        if v["method"] == "disconnect" {
            disconnect = true;
            continue;
        }
        assert_eq!(v["method"], "ai.delta", "{v}");
        assert_eq!(v["params"]["text"], delta_text(i, PAD));
        i += 1;
        if i % 100 == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            s.drain();
            if s.log.iter().any(closed(n)) {
                sent_after_close += 1;
            }
            // A relay that gave up early has closed the socket: the close code says so.
            let req = json!({ "jsonrpc": "2.0", "id": id, "method": "engine.status" });
            let _ = ws.0.send(Message::Text(req.to_string().into())).await;
            id += 1;
        }
    };
    assert_eq!(i, COUNT);
    assert!(disconnect, "no disconnect before the close");
    assert!(
        sent_after_close >= 1,
        "no request went out after the engine closed"
    );
    assert_eq!(code, close::FELL_BEHIND);
}
