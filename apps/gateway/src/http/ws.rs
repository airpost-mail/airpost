// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `/gateway/v1/ws`: one engine socket connection per WebSocket (docs/web-gateway-plan.md
//! 2.4, 2.6). Every upgrade is accepted and a refused one is closed with a code, because a
//! browser only sees 1006 for an upgrade answered with an HTTP error.
//!
//! The browser's first frame must be `hello`, which the gateway forwards as a relayed
//! `hello` for the device. After that each browser frame is parsed, checked against ms-ipc's
//! `policy` and re-encoded before it is written to the engine: the engine frames by newline,
//! and valid JSON may hold raw newlines between tokens, so the browser's bytes are never
//! forwarded as they are. Engine lines go to the browser unchanged and in order, with no
//! notification filter of the gateway's own: what the engine sends one connection only
//! (`ai.delta`, `ai.triage.progress`) reaches this WebSocket alone, and the frame limit
//! counts browser frames only, so a burst of them can't trip it. The engine connection
//! ends with the relay, which is how the engine cancels what a closed tab started
//! (docs/ai-plan.md A7). Frames for the browser wait in a small queue bounded by bytes; when
//! it is full the gateway stops reading the engine connection, so the engine's own
//! slow-subscriber drop applies and the gateway buffers no more than the queue and the
//! WebSocket's write buffer.
//!
//! Heartbeat (plan 2.4): a ping every `interval`, and the WebSocket closes with 1001, which
//! drops the engine connection, when nothing has come from the browser for `interval +
//! timeout`. A ping can't overtake the bytes already ahead of it, so a slow tab working
//! through a burst must not look dead: the browser's frames are read while a write to it
//! waits, a waiting write that moves on counts as hearing from the browser (its socket only
//! turns writable when the peer acknowledges bytes), and during a burst a ping also goes
//! out every 64 KiB, so that the browser meets one often while it reads through what the
//! socket buffers took without making the gateway wait.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{ready, Context, Poll};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::{HOST, ORIGIN};
use axum::http::HeaderMap;
use axum::response::Response;
use futures::stream::{SplitSink, SplitStream};
use futures::task::{self, ArcWake, AtomicWaker};
use futures::{SinkExt, StreamExt};
use governor::{Quota, RateLimiter};
use ms_ipc::{code, feature, method, policy, ErrorObject, Request, Response as RpcResponse};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::time::{Instant, MissedTickBehavior};

use super::{AppState, SocketSlot};
use crate::config::Heartbeat;
use crate::devices::Device;
use crate::engine;

/// Close codes (plan 2.6).
pub mod close {
    pub const NORMAL: u16 = 1000;
    pub const GOING_AWAY: u16 = 1001;
    pub const FELL_BEHIND: u16 = 4008;
    pub const PROTOCOL: u16 = 4400;
    pub const UNPAIRED: u16 = 4401;
    pub const FORBIDDEN: u16 = 4403;
    pub const LIMITED: u16 = 4429;
    pub const ENGINE_UNAVAILABLE: u16 = 4502;
}

/// Largest frame a browser may send.
pub const MAX_FRAME_BYTES: usize = 8 << 20;
/// Frames a second per connection, and the burst above it (plan 2.3).
const FRAMES_PER_SECOND: u32 = 50;
const FRAME_BURST: u32 = 200;
/// How long a new connection may take to send `hello`.
const HELLO_WAIT: Duration = Duration::from_secs(10);
/// Bytes queued for the browser past which the gateway stops reading the engine; at twice
/// this it stops reading the browser too, whose refused requests are answered here.
const OUT_QUEUE_BYTES: usize = 256 << 10;
/// How long the close frame may take; the browser may be gone.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

type Close = (u16, &'static str);
type Tx = SplitSink<WebSocket, Message>;
type Rx = SplitStream<WebSocket>;

fn admit(state: &AppState, headers: &HeaderMap) -> Result<(Device, SocketSlot), Close> {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    if !header(HOST).is_some_and(|h| state.config().host_allowed(h)) {
        return Err((close::FORBIDDEN, "host not allowed"));
    }
    if !header(ORIGIN).is_some_and(|o| state.config().origin_allowed(o)) {
        return Err((close::FORBIDDEN, "origin not allowed"));
    }
    let (device, _) = state
        .authenticate(headers)
        .ok_or((close::UNPAIRED, "unpaired"))?;
    let slot = state
        .take_socket(&device.id)
        .ok_or((close::LIMITED, "too many connections"))?;
    Ok((device, slot))
}

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let admitted = admit(&state, &headers);
    upgrade
        .max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            let (mut tx, mut rx) = socket.split();
            let (code, reason) = match admitted {
                Err(refusal) => {
                    tracing::info!(code = refusal.0, "websocket refused");
                    refusal
                }
                Ok((device, _slot)) => {
                    let out = relay(&state, &device, &mut tx, &mut rx).await;
                    tracing::info!(device = %device.id, code = out.0, "websocket closed");
                    out
                }
            };
            let frame = CloseFrame {
                code,
                reason: reason.into(),
            };
            let _ = tokio::time::timeout(CLOSE_WAIT, tx.send(Message::Close(Some(frame)))).await;
        })
}

async fn send_rpc(tx: &mut Tx, res: &RpcResponse) -> Result<(), Close> {
    tx.send(rpc_message(res))
        .await
        .map_err(|_| (close::NORMAL, ""))
}

fn rpc_message(res: &RpcResponse) -> Message {
    let line = ms_ipc::encode(res);
    Message::Text(
        String::from_utf8_lossy(trim_newline(&line))
            .into_owned()
            .into(),
    )
}

fn trim_newline(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}

/// Relays until either side ends; returns how to close the WebSocket.
async fn relay(state: &AppState, device: &Device, tx: &mut Tx, rx: &mut Rx) -> Close {
    match relay_inner(state, device, tx, rx).await {
        Ok(never) => match never {},
        Err(close) => close,
    }
}

enum Never {}

async fn relay_inner(
    state: &AppState,
    device: &Device,
    tx: &mut Tx,
    rx: &mut Rx,
) -> Result<Never, Close> {
    let socket = engine::find(&state.config().config_dir)
        .ok_or((close::ENGINE_UNAVAILABLE, engine::NOT_RUNNING))?;
    let stream = UnixStream::connect(&socket)
        .await
        .map_err(|_| (close::ENGINE_UNAVAILABLE, engine::NOT_RUNNING))?;
    let (r, mut w) = stream.into_split();
    let mut engine_lines = BufReader::new(r);
    hello(device, tx, rx, &mut engine_lines, &mut w).await?;

    let frames = RateLimiter::direct(
        Quota::per_second(NonZeroU32::new(FRAMES_PER_SECOND).expect("non-zero"))
            .allow_burst(NonZeroU32::new(FRAME_BURST).expect("non-zero")),
    );
    let Heartbeat { interval, timeout } = state.config().heartbeat;
    let silence = interval + timeout;
    let mut pings = tokio::time::interval_at(Instant::now() + interval, interval);
    pings.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut heard = Instant::now();
    let dead = tokio::time::sleep_until(heard + silence);
    tokio::pin!(dead);

    let mut generation = state.generation();
    let mut shutdown = state.shutdown();
    let mut out = Outbox::default();
    let mut line = Vec::new();
    let mut fell_behind = false;
    // Set when the engine connection ended; the queue drains to the browser first.
    let mut engine_ended: Option<Close> = None;
    loop {
        if let Some(close) = engine_ended {
            if out.idle() {
                return Err(close);
            }
        }
        tokio::select! {
            sent = std::future::poll_fn(|cx| out.poll_send(tx, cx)), if !out.idle() => {
                match sent {
                    Ok(true) => heard = Instant::now(),
                    Ok(false) => {}
                    Err(_) => return Err((close::NORMAL, "")),
                }
            }
            read = engine_lines.read_until(b'\n', &mut line),
                if engine_ended.is_none() && out.bytes < OUT_QUEUE_BYTES =>
            {
                match read {
                    Ok(0) | Err(_) => {
                        engine_ended = Some(if fell_behind {
                            (close::FELL_BEHIND, "the engine dropped this connection")
                        } else {
                            (close::ENGINE_UNAVAILABLE, "the engine connection ended")
                        });
                        continue;
                    }
                    Ok(_) => {}
                }
                if line.len() > ms_ipc::MAX_LINE_BYTES {
                    return Err((close::ENGINE_UNAVAILABLE, "the engine sent an oversized line"));
                }
                let text = trim_newline(&line);
                fell_behind |= is_disconnect(text);
                out.push(Message::Text(String::from_utf8_lossy(text).into_owned().into()));
                line.clear();
            }
            msg = rx.next(), if out.bytes < 2 * OUT_QUEUE_BYTES => {
                if matches!(msg, Some(Ok(_))) {
                    heard = Instant::now();
                }
                let text = match msg {
                    None | Some(Ok(Message::Close(_))) => return Err((close::NORMAL, "")),
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Binary(_))) => {
                        return Err((close::PROTOCOL, "binary frames are not accepted"));
                    }
                    Some(Err(_)) => return Err((close::PROTOCOL, "invalid or oversized frame")),
                };
                if frames.check().is_err() {
                    return Err((close::LIMITED, "too many frames"));
                }
                if let Some(line) = checked(text.as_str(), &device.scopes, &mut out)? {
                    w.write_all(&line)
                        .await
                        .map_err(|_| (close::ENGINE_UNAVAILABLE, "the engine connection ended"))?;
                }
            }
            _ = pings.tick() => out.ping(),
            () = &mut dead => {
                if heard.elapsed() >= silence {
                    tracing::info!(device = %device.id, "the browser stopped answering pings");
                    return Err((close::GOING_AWAY, "the browser stopped answering pings"));
                }
                dead.as_mut().reset(heard + silence);
            }
            _ = generation.changed() => {
                if !state.device_live(&device.id) {
                    return Err((close::UNPAIRED, "unpaired"));
                }
            }
            _ = shutdown.changed() => {
                return Err((close::GOING_AWAY, "the gateway is shutting down"));
            }
        }
    }
}

/// Frames waiting for the browser, oldest first. A ping goes to the front, so that it is
/// the next frame written.
#[derive(Default)]
struct Outbox {
    queue: VecDeque<Message>,
    bytes: usize,
    ping_queued: bool,
    /// Bytes handed to the sink since the last ping.
    since_ping: usize,
    /// Frames were handed to the sink since its last flush.
    unflushed: bool,
    /// The sink returned Pending on the last poll.
    waiting: bool,
    wake: Arc<SinkWake>,
}

/// Set when the sink wakes the relay. tokio-tungstenite wakes its sink waker only on socket
/// readiness, which a gone browser never gives: the socket turns writable when the peer
/// acknowledges bytes, or readable when it sends some.
#[derive(Default)]
struct SinkWake {
    woken: AtomicBool,
    relay: AtomicWaker,
}

impl ArcWake for SinkWake {
    fn wake_by_ref(this: &Arc<Self>) {
        this.woken.store(true, Ordering::Release);
        this.relay.wake();
    }
}

/// A ping goes out after this many bytes as well as every interval: a ping can't overtake
/// the bytes ahead of it, and when kernel buffers take a burst without making the gateway
/// wait, the pongs to these show the browser working through it.
const PING_EVERY_BYTES: usize = 64 << 10;

fn frame_len(m: &Message) -> usize {
    match m {
        Message::Text(t) => t.len(),
        Message::Binary(b) | Message::Ping(b) | Message::Pong(b) => b.len(),
        Message::Close(_) => 0,
    }
}

impl Outbox {
    fn push(&mut self, m: Message) {
        self.bytes += frame_len(&m);
        self.queue.push_back(m);
    }

    fn ping(&mut self) {
        if !self.ping_queued {
            self.ping_queued = true;
            self.queue.push_front(Message::Ping(Bytes::new()));
        }
    }

    fn idle(&self) -> bool {
        self.queue.is_empty() && !self.unflushed
    }

    /// Writes and flushes what is queued. Ready when it is all flushed, or when the sink
    /// went on after making the relay wait: `Ok(true)` then says the browser is taking
    /// bytes, though its pong may still be behind them.
    fn poll_send(&mut self, tx: &mut Tx, cx: &mut Context<'_>) -> Poll<Result<bool, axum::Error>> {
        self.wake.relay.register(cx.waker());
        let progressed = self.wake.woken.swap(false, Ordering::AcqRel) && self.waiting;
        let wake = self.wake.clone();
        let waker = task::waker_ref(&wake);
        let mut sink_cx = Context::from_waker(&waker);
        match self.drive(tx, &mut sink_cx) {
            Poll::Pending => {
                self.waiting = true;
                if progressed {
                    Poll::Ready(Ok(true))
                } else {
                    Poll::Pending
                }
            }
            Poll::Ready(Ok(())) => Poll::Ready(Ok(progressed || std::mem::take(&mut self.waiting))),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }

    fn drive(&mut self, tx: &mut Tx, cx: &mut Context<'_>) -> Poll<Result<(), axum::Error>> {
        while !self.queue.is_empty() {
            ready!(tx.poll_ready_unpin(cx))?;
            let m = self.queue.pop_front().expect("not empty");
            let len = frame_len(&m);
            self.bytes -= len;
            if matches!(m, Message::Ping(_)) {
                self.ping_queued = false;
                self.since_ping = 0;
            } else {
                self.since_ping += len;
            }
            tx.start_send_unpin(m)?;
            self.unflushed = true;
            if self.since_ping >= PING_EVERY_BYTES {
                self.ping();
            }
        }
        if self.unflushed {
            ready!(tx.poll_flush_unpin(cx))?;
            self.unflushed = false;
        }
        Poll::Ready(Ok(()))
    }
}

/// The engine's `disconnect` notification: it dropped this connection for falling behind.
fn is_disconnect(line: &[u8]) -> bool {
    line.len() < 4096
        && line.windows(12).any(|w| w == b"\"disconnect\"")
        && serde_json::from_slice::<Value>(line)
            .is_ok_and(|v| v["method"] == method::DISCONNECT && v.get("id").is_none())
}

/// Waits for the browser's `hello`, sends the relayed one, and forwards the engine's answer
/// with the gateway's version and the device's scopes added.
async fn hello(
    device: &Device,
    tx: &mut Tx,
    rx: &mut Rx,
    engine_lines: &mut BufReader<OwnedReadHalf>,
    w: &mut OwnedWriteHalf,
) -> Result<(), Close> {
    let first = tokio::time::timeout(HELLO_WAIT, rx.next())
        .await
        .map_err(|_| (close::PROTOCOL, "hello must come first"))?;
    let Some(Ok(Message::Text(text))) = first else {
        return Err((close::PROTOCOL, "hello must come first"));
    };
    let req = ms_ipc::parse_request(text.as_bytes())
        .ok()
        .filter(|r| r.method == method::HELLO && r.id.is_some())
        .ok_or((close::PROTOCOL, "hello must come first"))?;
    let id = req.id.clone().expect("checked");
    let relayed = Request::new(id, method::HELLO, relayed_hello_params(req.params, device));
    w.write_all(&ms_ipc::encode(&relayed))
        .await
        .map_err(|_| (close::ENGINE_UNAVAILABLE, engine::NOT_RUNNING))?;

    let mut line = Vec::new();
    let read = tokio::time::timeout(HELLO_WAIT, engine_lines.read_until(b'\n', &mut line)).await;
    let unavailable = (close::ENGINE_UNAVAILABLE, engine::NOT_RUNNING);
    if !matches!(read, Ok(Ok(n)) if n > 0) {
        return Err(unavailable);
    }
    let mut res: RpcResponse = serde_json::from_slice(&line).map_err(|_| unavailable)?;
    let Some(result) = res.result.as_mut() else {
        // The engine refused it (a protocol version it doesn't speak): the browser sees why.
        send_rpc(tx, &res).await?;
        return Err((close::ENGINE_UNAVAILABLE, "the engine refused hello"));
    };
    let relays = result["features"]
        .as_array()
        .is_some_and(|f| f.iter().any(|x| x == feature::RELAYED));
    if !relays {
        tracing::warn!("the engine doesn't support relayed connections; update it");
        return Err((close::ENGINE_UNAVAILABLE, engine::TOO_OLD));
    }
    result["gateway"] = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "scopes": device.scopes,
    });
    send_rpc(tx, &res).await
}

/// The browser's `hello` params with the relay fields the gateway alone may set.
fn relayed_hello_params(params: Value, device: &Device) -> Value {
    let mut p = match params {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let client = p.get("client").and_then(Value::as_str).unwrap_or("");
    let client = format!(
        "tern-gateway/{} ({})",
        env!("CARGO_PKG_VERSION"),
        client.chars().take(64).collect::<String>()
    );
    p.insert("client".into(), json!(client));
    p.insert("relayed".into(), json!(true));
    p.insert("scopes".into(), json!(device.scopes));
    p.insert(
        "device".into(),
        json!({ "id": device.id, "name": device.name }),
    );
    Value::Object(p)
}

/// The engine line for a browser frame, or None when the gateway answered or dropped it
/// itself.
fn checked(text: &str, scopes: &[String], out: &mut Outbox) -> Result<Option<Vec<u8>>, Close> {
    let req = match ms_ipc::parse_request(text.as_bytes()) {
        Ok(r) => r,
        Err((id, e)) => {
            out.push(rpc_message(&RpcResponse::err(id, e)));
            return Ok(None);
        }
    };
    if req.method == method::HELLO {
        return Err((close::PROTOCOL, "hello was already sent"));
    }
    if !policy::relayed_request_allowed(&req.method, &req.params, scopes) {
        tracing::info!(method = %log_name(&req.method), "refused a relayed request");
        if req.id.is_some() {
            let e = ErrorObject::new(
                code::FORBIDDEN_RELAYED,
                format!(
                    "{} is not allowed on a relayed connection without the admin scope",
                    log_name(&req.method)
                ),
            );
            out.push(rpc_message(&RpcResponse::err(req.id, e)));
        }
        return Ok(None);
    }
    Ok(Some(ms_ipc::encode(&req)))
}

/// Method names come from the browser: log the known ones only.
fn log_name(name: &str) -> String {
    match policy::method_class(name) {
        policy::MethodClass::LocalOnly => format!("unknown({})", name.len()),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_params_are_the_gateways() {
        let device: Device = serde_json::from_value(json!({
            "id": "d1", "name": "Laptop", "scopes": [], "tokenHash": "x",
            "pairedAt": 0, "lastUsedAt": 0,
        }))
        .unwrap();
        let p = relayed_hello_params(
            json!({ "protocol": 1, "client": "tern-web/0.1.0", "relayed": false,
                    "scopes": ["admin"], "device": {"id": "evil", "name": "x"} }),
            &device,
        );
        assert_eq!(p["relayed"], true);
        assert_eq!(p["scopes"], json!([]));
        assert_eq!(p["device"]["id"], "d1");
        assert_eq!(p["protocol"], 1);
        assert!(p["client"].as_str().unwrap().ends_with("(tern-web/0.1.0)"));
    }

    #[test]
    fn disconnect_lines() {
        assert!(is_disconnect(
            br#"{"jsonrpc":"2.0","method":"disconnect","params":{"reason":"x"}}"#
        ));
        assert!(!is_disconnect(
            br#"{"jsonrpc":"2.0","id":1,"result":{"note":"disconnect"}}"#
        ));
    }

    #[test]
    fn log_names() {
        assert_eq!(log_name("threads.list"), "threads.list");
        assert_eq!(log_name("evil\nline"), "unknown(9)");
    }
}
