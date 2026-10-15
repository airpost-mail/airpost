// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! `GET /deltas/<aid>/streaming` (docs/backend-plan.md 3.5): NDJSON over a chunked HTTP/1.1
//! response, fed by one task per stream.
//!
//! The task subscribes to the account's watch before the replay query, so a write committed
//! during the replay wakes it and is re-read from the last cursor sent. It sends a heartbeat
//! right away (so the headers go out) and every 5 s, checks the token on each heartbeat, and
//! ends cleanly after an hour, on revocation, on account removal, on server shutdown, and when
//! a newer stream of the same token replaces it (see `hub.rs`).

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use http_body::Frame;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, MissedTickBehavior};

use super::api::valid_aid;
use super::hub::{Limits, Subscription};
use super::{ApiError, AppState, AuthUser};
use crate::repo::{Authenticated, User};
use crate::wire::{
    BackendEvent, StreamLine, HEARTBEAT_BYTES, HEARTBEAT_INTERVAL_SECS, MAX_ACCOUNTS_PER_USER,
    MAX_STREAMS_PER_ACCOUNT, PAGE_SIZE, STREAM_MAX_SECS,
};

const HEARTBEAT: [u8; HEARTBEAT_BYTES] = [b'\n'; HEARTBEAT_BYTES];
/// Chunks queued between the task and hyper.
const CHANNEL_DEPTH: usize = 8;
/// A send that waits this long ends the stream. It only fires once the channel, hyper's buffer
/// and the socket buffers are all full, which at heartbeat rates takes far longer than this;
/// dead peers are caught by the same token reconnecting (`hub.rs`) and by the socket options
/// in `mod.rs`.
const SEND_TIMEOUT: Duration = Duration::from_secs(60);
/// One stream per account per device, and as many devices per account as the account limit
/// allows, so arbitrary `aid`s cannot open unbounded streams.
const STREAM_LIMITS: Limits = Limits {
    per_account: MAX_STREAMS_PER_ACCOUNT,
    per_user: MAX_STREAMS_PER_ACCOUNT * MAX_ACCOUNTS_PER_USER,
    per_token: MAX_ACCOUNTS_PER_USER,
};
/// Replay lines are grouped into chunks of about this size.
const CHUNK_BYTES: usize = 64 * 1024;

/// Parses the `cursor` query parameter: absent, empty or `0` replay everything.
pub fn parse_cursor(c: Option<&str>) -> Option<i64> {
    match c {
        None | Some("") => Some(0),
        Some(s) if s.bytes().all(|b| b.is_ascii_digit()) => s.parse().ok(),
        Some(_) => None,
    }
}

pub async fn stream(
    State(state): State<AppState>,
    AuthUser(auth): AuthUser,
    Path(aid): Path<String>,
    // `p` and `ih` are accepted and dropped (3.5).
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    if !valid_aid(&aid) {
        return Err(ApiError::BadRequest("invalid account id".into()));
    }
    let mut cursor = parse_cursor(q.get("cursor").map(String::as_str))
        .ok_or_else(|| ApiError::BadRequest("invalid cursor".into()))?;
    // A cursor above the counter was issued before a restore rolled seq back, so its
    // position means nothing here: replay everything instead of skipping new writes.
    if cursor > 0 && cursor > state.blocking(|r| r.current_seq()).await? {
        cursor = 0;
    }
    let sub = state
        .hub()
        .subscribe(&auth.user.id, &aid, auth.token_id, STREAM_LIMITS)
        .ok_or(ApiError::TooManyRequests)?;
    let (tx, rx) = mpsc::channel(CHANNEL_DEPTH);
    tokio::spawn(run(state, auth, aid, cursor, sub, tx));
    let mut res = Response::new(Body::new(ChannelBody(rx)));
    *res.status_mut() = StatusCode::OK;
    let h = res.headers_mut();
    h.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

/// The response body: whatever the task sends. It ends when the task drops its sender.
struct ChannelBody(mpsc::Receiver<Bytes>);

impl http_body::Body for ChannelBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.get_mut()
            .0
            .poll_recv(cx)
            .map(|b| b.map(|b| Ok(Frame::data(b))))
    }
}

struct Out(mpsc::Sender<Bytes>);

impl Out {
    /// False when the client is gone or stopped reading.
    async fn send(&self, b: Bytes) -> bool {
        matches!(
            tokio::time::timeout(SEND_TIMEOUT, self.0.send(b)).await,
            Ok(Ok(()))
        )
    }
}

fn line(l: &StreamLine, buf: &mut Vec<u8>) {
    serde_json::to_writer(&mut *buf, l).expect("stream lines serialize");
    buf.push(b'\n');
}

async fn until_true(mut rx: watch::Receiver<bool>) {
    let _ = rx.wait_for(|v| *v).await;
}

/// Sends every row of the account with seq above `last`, advancing `last`. False when the
/// stream should end.
async fn replay(state: &AppState, user_id: &str, aid: &str, last: &mut i64, out: &Out) -> bool {
    loop {
        let (u, a, after) = (user_id.to_owned(), aid.to_owned(), *last);
        let rows = match state
            .blocking(move |r| r.metadata_since(&u, &a, after, PAGE_SIZE))
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(error = ?e, "stream replay failed");
                return false;
            }
        };
        let n = rows.len();
        let mut buf = Vec::new();
        for (seq, attributes) in rows {
            line(
                &StreamLine::Metadata {
                    attributes,
                    cursor: seq.to_string(),
                },
                &mut buf,
            );
            *last = seq;
            if buf.len() >= CHUNK_BYTES && !out.send(std::mem::take(&mut buf).into()).await {
                return false;
            }
        }
        if !buf.is_empty() && !out.send(buf.into()).await {
            return false;
        }
        if n < PAGE_SIZE {
            return true;
        }
    }
}

fn identity_changed(a: &User, b: &User) -> bool {
    a.email != b.email || a.first_name != b.first_name || a.last_name != b.last_name
}

async fn run(
    state: AppState,
    auth: Authenticated,
    aid: String,
    cursor: i64,
    mut sub: Subscription,
    tx: mpsc::Sender<Bytes>,
) {
    let deadline = tokio::time::sleep(Duration::from_secs(STREAM_MAX_SECS));
    tokio::pin!(deadline);
    let shutdown = until_true(state.shutdown_rx());
    tokio::pin!(shutdown);
    let out = Out(tx);
    let token_id = auth.token_id;
    let mut user = auth.user;
    let mut last = cursor;

    if !out.send(Bytes::from_static(&HEARTBEAT)).await
        || !replay(&state, &user.id, &aid, &mut last, &out).await
    {
        return;
    }
    let period = Duration::from_secs(HEARTBEAT_INTERVAL_SECS);
    let mut heartbeat = tokio::time::interval_at(Instant::now() + period, period);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let reason = loop {
        tokio::select! {
            _ = &mut deadline => break "max duration",
            _ = &mut shutdown => break "server shutdown",
            _ = out.0.closed() => break "client gone",
            _ = &mut sub.evicted => break "replaced by a newer stream of the token",
            changed = sub.rx.changed() => {
                if changed.is_err() {
                    break "hub gone";
                }
                let signal = *sub.rx.borrow_and_update();
                if signal.closes != sub.closes {
                    break "account removed";
                }
                if signal.seq > last && !replay(&state, &user.id, &aid, &mut last, &out).await {
                    break "send failed";
                }
            }
            _ = heartbeat.tick() => {
                if !out.send(Bytes::from_static(&HEARTBEAT)).await {
                    break "send failed";
                }
                match state.blocking(move |r| r.live_token_user(token_id)).await {
                    Ok(None) => break "token revoked",
                    Ok(Some(now)) => {
                        if identity_changed(&user, &now) {
                            let mut buf = Vec::new();
                            line(
                                &StreamLine::BackendEvent { event: BackendEvent::IdentityRefreshNeeded },
                                &mut buf,
                            );
                            if !out.send(buf.into()).await {
                                break "send failed";
                            }
                        }
                        user = now;
                    }
                    Err(e) => tracing::warn!(error = ?e, "stream token check failed"),
                }
            }
        }
    };
    tracing::debug!(reason, "metadata stream closed");
}

#[cfg(test)]
mod tests {
    use super::parse_cursor;

    #[test]
    fn cursors() {
        assert_eq!(parse_cursor(None), Some(0));
        assert_eq!(parse_cursor(Some("")), Some(0));
        assert_eq!(parse_cursor(Some("0")), Some(0));
        assert_eq!(parse_cursor(Some("42")), Some(42));
        assert_eq!(parse_cursor(Some("-1")), None);
        assert_eq!(parse_cursor(Some("+1")), None);
        assert_eq!(parse_cursor(Some("1.5")), None);
        assert_eq!(parse_cursor(Some("abc")), None);
        assert_eq!(parse_cursor(Some("99999999999999999999")), None);
    }
}
