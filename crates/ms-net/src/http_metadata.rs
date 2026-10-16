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

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
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
use crate::metadata::{MetadataBackend, MetadataDelta, MetadataPush};
use crate::outbox::Outbox;

/// What the client is told when the backend has gone dormant after a 401/403.
const DORMANT_KEY: &str = "The Tern server rejected this device's token";

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
        })
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
        if self.is_dormant() {
            return Err(self.dormant_error());
        }
        let url = self.url(&["tern", "v1", "me"])?;
        let req = self
            .push_client
            .inner()
            .get(url.clone())
            .header(ACCEPT, "application/json");
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

    fn rebind(&self, local: &[MetadataPush]) -> Result<()> {
        info!(
            "Metadata binding changed: replaying from 0 and backfilling {} local values.",
            local.len()
        );
        self.outbox.rebind(&self.account_id, local)
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

    fn pull(&self, cursor: Option<&str>, sink: &mut dyn FnMut(MetadataDelta)) -> Result<String> {
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
        let mut reader = BufReader::new(resp);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    return Err(SyncError::offline(
                        "Metadata stream interrupted",
                        format!("{url}: {e}"),
                    )
                    .into())
                }
            }
            let text = line.trim_ascii();
            if text.len() <= 1 {
                continue;
            }
            match serde_json::from_slice::<StreamLine>(text) {
                Ok(StreamLine::Metadata { attributes, cursor }) => {
                    sink(self.record_from_stream(attributes));
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

/// The metadata thread aborts the process on a non-retryable error; nothing the server
/// answers should do that.
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
