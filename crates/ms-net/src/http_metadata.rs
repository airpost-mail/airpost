// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! [`HttpMetadataBackend`]: metadata sync with a Tern server (docs/backend-plan.md 3.5, 4.1).
//! The transport follows `MailSync/MetadataWorker.cpp` and `performRemoteSyncbackMetadata`
//! (Copyright © 2017 Foundry 376, GPL-3.0); the outbox, backfill, cursor binding, thread
//! anchor and 401 dormancy are Tern additions.
//!
//! - Pushes go to `POST /metadata/<aid>/<objectId>/<pluginId>`. A push the server cannot take
//!   now (connect error, timeout, 429, 5xx, 401/403) is queued in the [`Outbox`] and reported
//!   as a success, so `SyncbackMetadataTask` behaves as before. A push that does reach the
//!   server drops what is queued for its key, since the server keeps the last value written.
//! - Before each stream connect the `metadata` thread asks for the binding
//!   (`GET /tern/v1/me`), then drains the outbox, then pulls.
//! - Pulls stream `GET /deltas/<aid>/streaming` from the cursor, or from `0` on a first pull,
//!   instead of head + pages, which avoids losing pages when the head is saved first.
//! - A 401 or 403 on any call emits `ProcessIdentityRefreshNeeded` once and makes the backend
//!   dormant for the life of the process: no more requests, pushes go to the outbox, and
//!   `pull` fails retryably so the thread backs off.
//! - The last `/tern/v1/me` answer is kept: it carries `trackingNS`, `publicURL` and the
//!   server's features, which the tracking transforms read ([`MetadataBackend::tracking`]).
//! - Expiration claims (3.8) are `POST /tern/v1/claim/<aid>/<objectId>/<pluginId>/<expiration>`:
//!   200 wins, 409 loses, anything else lets the device act.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ms_core::constants::{thread, ACCOUNT_STOP_POLL_MS};
use ms_core::{Error, Metadata, Result, SyncError};
use ms_wire::{
    BackendEvent, BackfillRecord, BackfillResponse, Me, MetadataRecord, ObjectType, StreamLine,
    TernServer, BACKFILL_MAX_RECORDS, LOW_SPEED_WINDOW_SECS,
};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Url;
use serde_json::Value;
use tracing::{error, info, warn};

use crate::http::{check_status, transport_error, HttpClient, CONNECT_TIMEOUT};
use crate::metadata::{
    is_local_only_plugin, Claim, MetadataBackend, MetadataDelta, MetadataPush, TrackingTarget,
};
use crate::outbox::Outbox;

/// What the client is told when the backend has gone dormant after a 401/403.
const DORMANT_KEY: &str = "The Tern server rejected this device's token";

/// How long a `/tern/v1/me` answer serves the tracking transforms before it is asked again.
/// The metadata thread asks before each stream connect, and the server ends streams after an
/// hour, so it is normally fresher than this.
const ME_MAX_AGE: Duration = Duration::from_secs(2 * 3600);

/// The bound on the requests a send or an expiration waits for (`/tern/v1/me` for tracking, a
/// claim): long enough for a slow server, short enough not to hold up a send for long.
const QUICK_TIMEOUT: Duration = Duration::from_secs(15);

/// Told of each `/tern/v1/me` answer, and of `None` once the token is refused.
pub type MeListener = Box<dyn Fn(Option<&Me>) + Send + Sync>;

pub struct HttpMetadataBackend {
    server: TernServer,
    account_id: String,
    authorization: String,
    push_client: HttpClient,
    /// HTTP/1.1 only (the workspace reqwest negotiates h2, which proxies buffer), and a 30 s
    /// timeout that blocking reqwest applies to the headers and to each `read` separately:
    /// curl's low-speed abort in C++.
    stream_client: Client,
    outbox: Outbox,
    /// Held across each push or drained entry and its outbox update. The server keeps the
    /// last value written, so a queued value must never reach it after a newer direct push.
    send_lock: Mutex<()>,
    dormant: AtomicBool,
    binding_mismatch_reported: AtomicBool,
    on_identity_refresh: Box<dyn Fn() + Send + Sync>,
    /// The last `/tern/v1/me` answer and when it came.
    me: Mutex<Option<(Me, Instant)>>,
    on_me: Option<MeListener>,
}

/// How a request ended.
enum Outcome {
    Ok(String),
    /// 401 or 403.
    Unauthorized,
    /// Try again later: no answer, 408, 429, 5xx.
    Retry(Error),
    /// The server refused this request for good (400, 404, 413, …).
    Rejected(Error),
}

impl HttpMetadataBackend {
    /// `on_identity_refresh` sends `ProcessIdentityRefreshNeeded` to the client.
    pub fn new(
        server: TernServer,
        account_id: &str,
        config_dir: &Path,
        on_identity_refresh: Box<dyn Fn() + Send + Sync>,
    ) -> Result<Self> {
        let push_client = HttpClient::new()?;
        let stream_client = Client::builder()
            .http1_only()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(Duration::from_secs(LOW_SPEED_WINDOW_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_rustls()
            .build()
            .map_err(|e| Error::other(format!("Could not create the HTTP client: {e}")))?;
        let authorization = format!("Basic {}", STANDARD.encode(format!("{}:", server.token)));
        Ok(HttpMetadataBackend {
            server,
            account_id: account_id.to_owned(),
            authorization,
            push_client,
            stream_client,
            outbox: Outbox::open(config_dir)?,
            send_lock: Mutex::new(()),
            dormant: AtomicBool::new(false),
            binding_mismatch_reported: AtomicBool::new(false),
            on_identity_refresh,
            me: Mutex::new(None),
            on_me: None,
        })
    }

    /// Calls `listener` with each `/tern/v1/me` answer, and with `None` when the token is
    /// refused; the engine follows the server's features with it.
    pub fn with_me_listener(mut self, listener: MeListener) -> Self {
        self.on_me = Some(listener);
        self
    }

    /// The last `/tern/v1/me` answer, if any.
    pub fn me(&self) -> Option<Me> {
        self.me_lock().as_ref().map(|(me, _)| me.clone())
    }

    fn me_lock(&self) -> MutexGuard<'_, Option<(Me, Instant)>> {
        self.me.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `GET /tern/v1/me`, kept and passed to the listener. `timeout` bounds the whole request
    /// (the push client's own bound is 300 s).
    fn fetch_me(&self, timeout: Option<Duration>) -> Result<Me> {
        if self.is_dormant() {
            return Err(self.dormant_error());
        }
        let url = self.url(&["tern", "v1", "me"])?;
        let mut req = self
            .push_client
            .inner()
            .get(url.clone())
            .header(ACCEPT, "application/json");
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let body = match self.send(req, &url) {
            Outcome::Ok(b) => b,
            Outcome::Unauthorized => {
                self.go_dormant("/tern/v1/me");
                return Err(self.dormant_error());
            }
            Outcome::Retry(e) => return Err(e),
            Outcome::Rejected(e) => return Err(as_retryable(e)),
        };
        let me: Me = serde_json::from_str(&body).map_err(|e| {
            Error::from(SyncError::new(
                "Invalid /tern/v1/me response",
                e.to_string(),
                true,
                false,
            ))
        })?;
        *self.me_lock() = Some((me.clone(), Instant::now()));
        if let Some(listener) = &self.on_me {
            listener(Some(&me));
        }
        Ok(me)
    }

    pub fn server(&self) -> &TernServer {
        &self.server
    }

    pub fn outbox(&self) -> &Outbox {
        &self.outbox
    }

    pub fn is_dormant(&self) -> bool {
        self.dormant.load(Ordering::SeqCst)
    }

    fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = Url::parse(&self.server.url)
            .map_err(|e| Error::other(format!("Invalid Tern server URL: {e}")))?;
        url.path_segments_mut()
            .map_err(|_| Error::other("Invalid Tern server URL"))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    fn send_lock(&self) -> MutexGuard<'_, ()> {
        self.send_lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn dormant_error(&self) -> Error {
        SyncError::new(DORMANT_KEY, self.server.host(), true, false).into()
    }

    fn go_dormant(&self, during: &str) {
        if !self.dormant.swap(true, Ordering::SeqCst) {
            error!(
                "{} ({during}); no more requests to {} until the engine restarts.",
                DORMANT_KEY,
                self.server.host()
            );
            (self.on_identity_refresh)();
            *self.me_lock() = None;
            if let Some(listener) = &self.on_me {
                listener(None);
            }
        }
    }

    fn send(&self, req: RequestBuilder, url: &Url) -> Outcome {
        let url = url.as_str();
        let resp = match req.header(AUTHORIZATION, &self.authorization).send() {
            Ok(r) => r,
            Err(e) => return Outcome::Retry(transport_error(&e, url).into()),
        };
        let status = resp.status().as_u16();
        let body = match resp.text() {
            Ok(b) => b,
            Err(e) => return Outcome::Retry(transport_error(&e, url).into()),
        };
        classify(status, url, body)
    }

    fn post_json(&self, url: &Url, body: &Value) -> Outcome {
        let req = self
            .push_client
            .inner()
            .post(url.clone())
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string());
        self.send(req, url)
    }

    fn post_push(&self, push: &MetadataPush) -> Result<Outcome> {
        let Some(object_type) = ObjectType::parse(&push.model_class.to_ascii_lowercase()) else {
            return Ok(Outcome::Rejected(Error::other(format!(
                "Metadata for unsupported model class {}",
                push.model_class
            ))));
        };
        let url = self.url(&[
            "metadata",
            &push.account_id,
            &push.model_id,
            &push.plugin_id,
        ])?;
        let body = ms_wire::MetadataPush {
            object_type,
            version: Some(push.version.max(0) as u64),
            value: push.value.clone(),
            header_message_id: push.header_message_id.clone(),
        };
        let body = serde_json::to_value(&body).map_err(|e| Error::other(e.to_string()))?;
        Ok(self.post_json(&url, &body))
    }

    /// One batch of backfill entries. Records the server lists as rejected are dropped too:
    /// they are invalid and would never be taken.
    fn post_backfill(&self, entries: &[crate::outbox::OutboxEntry]) -> Result<Outcome> {
        let records: Vec<BackfillRecord> = entries
            .iter()
            .filter_map(|e| {
                Some(BackfillRecord {
                    object_type: ObjectType::parse(&e.push.model_class.to_ascii_lowercase())?,
                    object_id: e.push.model_id.clone(),
                    plugin_id: e.push.plugin_id.clone(),
                    version: Some(e.push.version.max(0) as u64),
                    value: e.push.value.clone(),
                    header_message_id: e.push.header_message_id.clone(),
                })
            })
            .collect();
        if records.is_empty() {
            return Ok(Outcome::Ok(String::new()));
        }
        let url = self.url(&["tern", "v1", "backfill", &self.account_id])?;
        let body = serde_json::to_value(&records).map_err(|e| Error::other(e.to_string()))?;
        let outcome = self.post_json(&url, &body);
        if let Outcome::Ok(text) = &outcome {
            match serde_json::from_str::<BackfillResponse>(text) {
                Ok(r) => info!(
                    "Backfilled {} metadata values ({} changed on the server, {} rejected).",
                    records.len(),
                    r.written,
                    r.rejected.len()
                ),
                Err(_) => info!("Backfilled {} metadata values.", records.len()),
            }
        }
        Ok(outcome)
    }

    fn queue(&self, push: &MetadataPush, why: &str) -> Result<()> {
        info!(
            "Queued metadata {}:{} for the Tern server ({why}).",
            push.model_id, push.plugin_id
        );
        self.outbox.put(push)
    }

    fn record_from_stream(&self, attrs: MetadataRecord) -> MetadataDelta {
        let metadata = Metadata {
            object_type: attrs.object_type.as_str().to_owned(),
            object_id: attrs.object_id,
            account_id: attrs.aid,
            plugin_id: attrs.plugin_id,
            version: i64::from(attrs.v),
            value: attrs.value,
        };
        match attrs.header_message_id {
            Some(h) if attrs.object_type == ObjectType::Thread && !h.is_empty() => {
                MetadataDelta::AnchoredThreadMetadata(metadata, h)
            }
            _ => MetadataDelta::Metadata(metadata),
        }
    }
}

/// Where `me`'s tracking URLs point, `None` when the server offers no tracking or gave
/// something no recipient could use. A missing `publicURL` falls back to the server URL
/// (O8).
pub fn tracking_target(me: &Me, server_url: &str) -> Option<TrackingTarget> {
    if !me.features.tracking {
        return None;
    }
    let ns_ok = !me.tracking_ns.is_empty()
        && me
            .tracking_ns
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let public = me.public_url.trim().trim_end_matches('/');
    let public_url = if public.is_empty() {
        server_url
    } else {
        public
    };
    // The URL ends up in HTML and in links; allow only RFC 3986 unreserved, sub-delims, ':',
    // '/' and '%', which rules out quotes and angle brackets.
    let url_chars_ok = public_url
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:/%".contains(&b));
    if !ns_ok || !url_chars_ok || ms_wire::check_url(public_url).is_err() {
        warn!("-X The Tern server offers tracking but its trackingNS or publicURL is unusable; sending untracked");
        return None;
    }
    Some(TrackingTarget {
        public_url: public_url.to_owned(),
        ns: me.tracking_ns.clone(),
    })
}

fn classify(status: u16, url: &str, body: String) -> Outcome {
    match status {
        200..=209 => Outcome::Ok(body),
        401 | 403 => Outcome::Unauthorized,
        _ => {
            let err = check_status(status, url, &body).expect_err("not a success status");
            if status == 408 || status == 429 || status >= 500 {
                let Error::Sync(mut s) = err else {
                    return Outcome::Retry(err);
                };
                s.retryable = true;
                Outcome::Retry(s.into())
            } else {
                Outcome::Rejected(err)
            }
        }
    }
}

/// `p` in the stream query, as the C++ engine sent it.
fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

impl MetadataBackend for HttpMetadataBackend {
    fn push(&self, push: &MetadataPush) -> Result<()> {
        if is_local_only_plugin(&push.plugin_id) {
            return Ok(());
        }
        if self.is_dormant() {
            return self.queue(push, "token rejected");
        }
        let _guard = self.send_lock();
        match self.post_push(push)? {
            // Anything still queued for this key is older and must not be sent after it.
            Outcome::Ok(_) => self.outbox.remove_key(push),
            Outcome::Unauthorized => {
                self.go_dormant("push");
                self.queue(push, "token rejected")
            }
            Outcome::Retry(e) => self.queue(push, &e.to_string()),
            Outcome::Rejected(e) => Err(e),
        }
    }

    fn binding(&self) -> Result<Option<String>> {
        let me = self.fetch_me(None)?;
        let binding = me.binding();
        if binding != self.server.binding()
            && !self.binding_mismatch_reported.swap(true, Ordering::SeqCst)
        {
            warn!(
                "The Tern server reports server {} and user {}, not the {} / {} the client gave; using the server's.",
                me.server_id, me.user_id, self.server.server_id, self.server.user_id
            );
            (self.on_identity_refresh)();
        }
        Ok(Some(binding))
    }

    fn tracking(&self) -> Option<TrackingTarget> {
        if self.is_dormant() {
            return None;
        }
        let cached = self.me_lock().clone();
        let me = match cached {
            Some((me, at)) if at.elapsed() < ME_MAX_AGE => me,
            stale => match self.fetch_me(Some(QUICK_TIMEOUT)) {
                Ok(me) => me,
                Err(e) => {
                    warn!("-X Could not ask the Tern server about tracking: {e}");
                    stale.map(|(me, _)| me)?
                }
            },
        };
        tracking_target(&me, &self.server.url)
    }

    fn claim(&self, object_id: &str, plugin_id: &str, expiration: i64) -> Claim {
        if self.is_dormant() {
            return Claim::Unreachable;
        }
        let url = match self.url(&[
            "tern",
            "v1",
            "claim",
            &self.account_id,
            object_id,
            plugin_id,
            &expiration.to_string(),
        ]) {
            Ok(u) => u,
            Err(e) => {
                warn!("-X Could not claim {plugin_id} of {object_id}: {e}");
                return Claim::Unreachable;
            }
        };
        let resp = self
            .push_client
            .inner()
            .post(url.clone())
            .timeout(QUICK_TIMEOUT)
            .header(AUTHORIZATION, &self.authorization)
            .header(ACCEPT, "application/json")
            .send();
        let status = match resp {
            Ok(r) => r.status().as_u16(),
            Err(e) => {
                warn!(
                    "-X Could not claim {plugin_id} of {object_id}; acting anyway: {}",
                    transport_error(&e, url.as_str())
                );
                return Claim::Unreachable;
            }
        };
        match status {
            200..=209 => Claim::Won,
            409 => Claim::Lost,
            401 | 403 => {
                self.go_dormant("claim");
                Claim::Unreachable
            }
            other => {
                warn!("-X The Tern server answered {other} to a claim of {plugin_id} of {object_id}; acting anyway");
                Claim::Unreachable
            }
        }
    }

    fn rebind(&self, local: &[MetadataPush]) -> Result<()> {
        let local: Vec<MetadataPush> = local
            .iter()
            .filter(|p| !is_local_only_plugin(&p.plugin_id))
            .cloned()
            .collect();
        info!(
            "Metadata binding changed: replaying from 0 and backfilling {} local values.",
            local.len()
        );
        self.outbox.rebind(&self.account_id, &local)
    }

    fn drain_outbox(&self) -> Result<()> {
        loop {
            if self.is_dormant() {
                return Err(self.dormant_error());
            }
            let batch = self
                .outbox
                .pending(&self.account_id, BACKFILL_MAX_RECORDS)?;
            if batch.is_empty() {
                return Ok(());
            }
            let (backfill, pushes): (Vec<_>, Vec<_>) = batch.into_iter().partition(|e| e.backfill);
            let guard = self.send_lock();
            // A direct push since `pending` may have replaced or removed some of these.
            let backfill = self.outbox.still_queued(backfill)?;
            if !backfill.is_empty() {
                match self.post_backfill(&backfill)? {
                    Outcome::Ok(_) => {}
                    Outcome::Unauthorized => {
                        self.go_dormant("backfill");
                        return Err(self.dormant_error());
                    }
                    Outcome::Retry(e) => return Err(e),
                    Outcome::Rejected(e) => {
                        warn!("Dropping a backfill batch the server refused: {e}")
                    }
                }
                let ids: Vec<i64> = backfill.iter().map(|e| e.id).collect();
                self.outbox.remove(&ids)?;
            }
            drop(guard);
            for entry in pushes {
                let _guard = self.send_lock();
                if !self.outbox.is_queued(entry.id)? {
                    continue;
                }
                match self.post_push(&entry.push)? {
                    Outcome::Ok(_) => {}
                    Outcome::Unauthorized => {
                        self.go_dormant("push");
                        return Err(self.dormant_error());
                    }
                    Outcome::Retry(e) => return Err(e),
                    Outcome::Rejected(e) => warn!(
                        "Dropping queued metadata {}:{} the server refused: {e}",
                        entry.push.model_id, entry.push.plugin_id
                    ),
                }
                self.outbox.remove(&[entry.id])?;
            }
        }
    }

    fn pull(
        &self,
        cursor: Option<&str>,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(MetadataDelta),
    ) -> Result<String> {
        if self.is_dormant() {
            return Err(self.dormant_error());
        }
        let from = cursor.filter(|c| !c.is_empty()).unwrap_or("0");
        let mut url = self.url(&["deltas", &self.account_id, "streaming"])?;
        url.query_pairs_mut()
            .append_pair("p", platform())
            .append_pair("cursor", from);
        let resp = self
            .stream_client
            .get(url.clone())
            .header(AUTHORIZATION, &self.authorization)
            .send()
            .map_err(|e| as_retryable(transport_error(&e, url.as_str()).into()))?;
        let status = resp.status().as_u16();
        if !(200..=209).contains(&status) {
            let body = resp.text().unwrap_or_default();
            return Err(match classify(status, url.as_str(), body) {
                Outcome::Unauthorized => {
                    self.go_dormant("stream");
                    self.dormant_error()
                }
                Outcome::Retry(e) | Outcome::Rejected(e) => as_retryable(e),
                Outcome::Ok(_) => unreachable!("checked above"),
            });
        }
        info!("Metadata stream open from cursor {from}.");
        let mut last = from.to_owned();
        let lines = read_lines_on_a_thread(resp)?;
        loop {
            if stop() {
                info!("Metadata stream closed at cursor {last}: the account is stopping.");
                return Ok(last);
            }
            let line = match lines.recv_timeout(Duration::from_millis(ACCOUNT_STOP_POLL_MS)) {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => {
                    return Err(SyncError::offline(
                        "Metadata stream interrupted",
                        format!("{url}: {e}"),
                    )
                    .into())
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let text = line.trim_ascii();
            if text.len() <= 1 {
                continue;
            }
            match serde_json::from_slice::<StreamLine>(text) {
                Ok(StreamLine::Metadata { attributes, cursor }) => {
                    // Never pushed from a device; a server that holds one got it elsewhere.
                    if !is_local_only_plugin(&attributes.plugin_id) {
                        sink(self.record_from_stream(attributes));
                    }
                    sink(MetadataDelta::Cursor(cursor.clone()));
                    last = cursor;
                }
                Ok(StreamLine::BackendEvent {
                    event: BackendEvent::IdentityRefreshNeeded,
                }) => sink(MetadataDelta::IdentityRefreshNeeded),
                Err(e) => warn!("Skipping a metadata stream line that is not valid: {e}"),
            }
        }
        info!("Metadata stream closed at cursor {last}.");
        Ok(last)
    }
}

/// Reads the stream's lines, heartbeats included, on a `metadataStream` thread, so the pull
/// can return when the account stops while a read is blocked: the server only closes a
/// stream after an hour. The thread ends at the stream's end, on a read error, or at the
/// first line after the pull returned (a heartbeat comes every few seconds), closing the
/// connection.
fn read_lines_on_a_thread(
    resp: reqwest::blocking::Response,
) -> Result<Receiver<std::io::Result<Vec<u8>>>> {
    // Bounded so a fast stream cannot run ahead of the database.
    let (tx, rx) = mpsc::sync_channel(STREAM_LINES_AHEAD);
    ms_core::log::spawn_account_thread(
        ms_core::log::thread_account(),
        thread::METADATA_STREAM,
        move || {
            let mut reader = BufReader::new(resp);
            loop {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => return,
                    Ok(_) => {
                        if tx.send(Ok(line)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                }
            }
        },
    )
    .map_err(|e| {
        SyncError::new(
            "Could not start the metadata stream reader",
            e.to_string(),
            true,
            false,
        )
    })?;
    Ok(rx)
}

/// How many stream lines the reader thread may read ahead of the pull.
const STREAM_LINES_AHEAD: usize = 256;

/// A non-retryable error stops the account (aborts the process in `sync` mode); nothing the
/// server answers should do that.
fn as_retryable(e: Error) -> Error {
    match e {
        Error::Sync(mut s) => {
            s.retryable = true;
            s.into()
        }
        other => {
            SyncError::new("Tern server request failed", other.to_string(), true, false).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(public_url: &str) -> Me {
        Me {
            user_id: "u".into(),
            email: "a@example.com".into(),
            first_name: String::new(),
            last_name: String::new(),
            tracking_ns: "ns_1-A".into(),
            server_id: "s".into(),
            public_url: public_url.into(),
            features: ms_wire::Features {
                tracking: true,
                sharing: false,
            },
        }
    }

    #[test]
    fn tracking_target_checks_the_public_url() {
        let t = tracking_target(&me("https://t.example/base/"), "https://srv.example").unwrap();
        assert_eq!(t.public_url, "https://t.example/base");
        assert_eq!(t.ns, "ns_1-A");
        let t = tracking_target(&me(""), "https://srv.example").unwrap();
        assert_eq!(t.public_url, "https://srv.example");
        for bad in [
            r#"https://t.example/a"><a/href="https://evil">Pay/now</a><img/src="x"#,
            "https://t.example/a<b",
            "https://t.example/a>b",
            "https://t.example/a\"b",
            "https://t.example/a`b",
            "https://t.example/a\\b",
            "https://t.example/a{b}",
            "https://t.example/a b",
            "https://t.example/a?b",
        ] {
            assert!(
                tracking_target(&me(bad), "https://srv.example").is_none(),
                "{bad}"
            );
        }
        let mut off = me("https://t.example");
        off.features.tracking = false;
        assert!(tracking_target(&off, "https://srv.example").is_none());
    }

    #[test]
    fn statuses() {
        assert!(matches!(classify(200, "u", String::new()), Outcome::Ok(_)));
        assert!(matches!(
            classify(401, "u", String::new()),
            Outcome::Unauthorized
        ));
        assert!(matches!(
            classify(403, "u", String::new()),
            Outcome::Unauthorized
        ));
        for s in [408, 429, 500, 502, 503] {
            match classify(s, "u", String::new()) {
                Outcome::Retry(e) => assert!(e.is_retryable(), "{s}"),
                _ => panic!("{s} not retryable"),
            }
        }
        for s in [400, 404, 413] {
            assert!(
                matches!(classify(s, "u", String::new()), Outcome::Rejected(_)),
                "{s}"
            );
        }
    }
}
