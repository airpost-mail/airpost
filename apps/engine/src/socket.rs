// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine protocol on a Unix domain socket (2 and 4),
//! for `sync --socket` and `serve`. Unix-only, like the transport; its OS calls are
//! in [`platform::uds`](crate::platform::uds).
//!
//! One acceptor thread, and per connection a reader thread and a writer thread draining a
//! bounded queue (1.6). The read methods are answered out of order on the connection's
//! read pool ([`crate::read_pool`]); account-scoped requests become [`Command`]s for that
//! account's `main` thread, found through [`Accounts`]. A connection's other requests are
//! answered in order. The [`Hub`] is every account's `DeltaStream` sink: it fans each flushed line out
//! to every subscriber whose filter matches, without blocking; a subscriber whose queue
//! passes the bound is sent `disconnect` and closed (1.4). The account secrets delta never
//! reaches the socket: it stays on the parent's stdout, where the secrets came in
//! (section 2), and so do `account.add` and `account.remove`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::ops::ControlFlow;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tern_core::constants::{
    thread, DELTA_RING_BYTES, SOCKET_BACKLOG_BYTES, SOCKET_MAX_CONNECTIONS, SOCKET_QUEUE_BYTES,
};
use tern_core::Account;
use tern_ipc::{
    account_state, code, feature, method, policy, task_status, AccountChangedParams, AccountInfo,
    AccountListResult, AccountStateParams, AccountStatus, AiDeltaParams, AiTriageProgressParams,
    BodiesFetchParams, CalendarSyncParams, Capabilities, CredentialsStatus, DeltaCursor,
    DisconnectParams, EngineStatusResult, ErrorObject, FrameError, HelloParams, HelloResult,
    LogsTailParams, LogsTailResult, MailNewParams, Notification, RelayDevice,
    Request as RpcRequest, Response, RulesCancelParams, RulesDisabledParams, RulesProgressParams,
    RulesReprocessParams, SettingsGetParams, SettingsValues, SubscribeParams, SubscribeResult,
    SyncWakeParams, TaskCancelParams, TaskDoneParams, TaskQueueParams, ThreadsSnoozeParams,
    ThreadsUnsnoozeParams, LOGS_TAIL_DEFAULT_BYTES, LOGS_TAIL_MAX_BYTES, MAIL_RULES_KEY,
    MAX_LINE_BYTES, PROTOCOL_VERSION,
};
use tern_store::{DeltaItem, DeltaSink, DeltaStream, MAIL_NEW, SECRETS_UPDATED};
use tracing::{info, warn};

use crate::ai_api::{self, is_ai_method, StoredAi, Streams};
use crate::dispatch::{Command, Deferred, Reply, Request};
use crate::files::is_files_method;
use crate::integrations_api::{self, is_capability_setting, is_integrations_method};
use crate::platform;
use crate::read_api::{is_read_method, ReadApi};
use crate::settings::{Settings, SettingsGuard};
use crate::workers::{self, Lane};

/// How long a blocked write waits before the writer checks whether its connection is
/// being dropped.
const WRITE_POLL: Duration = Duration::from_millis(250);
/// A dropped connection's writer keeps trying to deliver the `disconnect` notification
/// this long, then closes the socket regardless.
const DROP_GRACE: Duration = Duration::from_secs(2);

/// The per-connection queue bound. Debug builds read `TERN_ENGINE_SOCKET_QUEUE_BYTES` so the
/// tests can reach it quickly.
pub fn queue_limit() -> usize {
    if cfg!(debug_assertions) {
        if let Some(n) = std::env::var("TERN_ENGINE_SOCKET_QUEUE_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            return n;
        }
    }
    SOCKET_QUEUE_BYTES
}

/// Writes `$CONFIG_DIR_PATH/run/<accountId>.json` (1.5) for `sync --socket` and returns
/// its path.
pub fn write_discovery(
    config_dir: &Path,
    account_id: &str,
    socket: &Path,
) -> std::io::Result<PathBuf> {
    let name: String = account_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    write_discovery_file(config_dir, &name, socket, |body| {
        body.insert("accountId".into(), json!(account_id));
    })
}

/// Writes `$CONFIG_DIR_PATH/run/engine.json` for `serve`, listing `accounts`, and returns
/// its path. Rewritten whenever an account is added or removed.
pub fn write_engine_discovery(
    config_dir: &Path,
    socket: &Path,
    accounts: &[String],
    detached: bool,
) -> std::io::Result<PathBuf> {
    write_discovery_file(config_dir, ENGINE_DISCOVERY_NAME, socket, |body| {
        body.insert("accounts".into(), json!(accounts));
        if detached {
            body.insert("detached".into(), json!(true));
        }
    })
}

/// `sha256:<hex>` of the running executable, so a client can tell whether an engine it
/// finds running is the build it bundles. Hashed once; `None` when the file can't be read.
pub fn build_id() -> Option<String> {
    static BUILD: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BUILD
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let mut file = std::fs::File::open(exe).ok()?;
            let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
            let mut buf = vec![0u8; 1 << 16];
            loop {
                let n = file.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                ctx.update(&buf[..n]);
            }
            let hex: String = ctx
                .finish()
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            Some(format!("sha256:{hex}"))
        })
        .clone()
}

/// `run/<name>.json` of a `serve` engine.
pub const ENGINE_DISCOVERY_NAME: &str = "engine";

fn write_discovery_file(
    config_dir: &Path,
    name: &str,
    socket: &Path,
    extra: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> std::io::Result<PathBuf> {
    let dir = config_dir.join("run");
    platform::uds::create_private_dir(&dir)?;
    let path = dir.join(format!("{name}.json"));
    let socket = std::path::absolute(socket).unwrap_or_else(|_| socket.to_owned());
    let mut body = serde_json::Map::new();
    body.insert("pid".into(), json!(std::process::id()));
    body.insert("socket".into(), json!(socket.to_string_lossy()));
    body.insert("protocol".into(), json!(PROTOCOL_VERSION));
    extra(&mut body);
    platform::uds::write_private_file(&path, Value::Object(body).to_string().as_bytes())?;
    Ok(path)
}

fn framed(mut line: Vec<u8>) -> Arc<[u8]> {
    line.push(b'\n');
    line.into()
}

fn disconnect_line(reason: &str) -> Arc<[u8]> {
    let n = Notification::new(
        method::DISCONNECT,
        serde_json::to_value(DisconnectParams {
            reason: reason.to_owned(),
        })
        .unwrap_or(Value::Null),
    );
    tern_ipc::encode(&n).into()
}

// -- Per-connection queue -------------------------------------------------------------------

pub(crate) enum PushError {
    Full,
    Closed,
}

#[derive(Default)]
struct OutState {
    lines: VecDeque<Arc<[u8]>>,
    bytes: usize,
    /// No more pushes; the writer drains what is queued, then closes the socket.
    closed: bool,
    /// When the connection was dropped for falling behind.
    dropped_at: Option<Instant>,
}

/// A connection's unsent lines, bounded by bytes. Responses and notifications share it, so
/// the client sees them in the order they were queued.
pub(crate) struct Outbox {
    state: Mutex<OutState>,
    cv: Condvar,
    limit: usize,
}

impl Outbox {
    fn new(limit: usize) -> Self {
        Outbox {
            state: Mutex::new(OutState::default()),
            cv: Condvar::new(),
            limit,
        }
    }

    fn lock(&self) -> MutexGuard<'_, OutState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queues `line`. A line longer than the bound still goes through on an empty queue,
    /// or it could never be sent.
    pub(crate) fn push(&self, line: Arc<[u8]>) -> Result<(), PushError> {
        let mut st = self.lock();
        if st.closed {
            return Err(PushError::Closed);
        }
        if !st.lines.is_empty() && st.bytes + line.len() > self.limit {
            return Err(PushError::Full);
        }
        st.bytes += line.len();
        st.lines.push_back(line);
        drop(st);
        self.cv.notify_all();
        Ok(())
    }

    /// Drops what is queued and leaves only `last` to send.
    fn drop_with(&self, last: Arc<[u8]>) {
        let mut st = self.lock();
        st.bytes = last.len();
        st.lines.clear();
        st.lines.push_back(last);
        st.closed = true;
        st.dropped_at = Some(Instant::now());
        drop(st);
        self.cv.notify_all();
    }

    fn close(&self) {
        self.lock().closed = true;
        self.cv.notify_all();
    }

    /// The writer is gone: nothing more will be sent.
    fn kill(&self) {
        let mut st = self.lock();
        st.closed = true;
        st.lines.clear();
        st.bytes = 0;
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// The next line, waiting for one; `None` once closed and drained.
    fn pop(&self) -> Option<Arc<[u8]>> {
        let mut st = self.lock();
        loop {
            if let Some(line) = st.lines.pop_front() {
                st.bytes -= line.len();
                return Some(line);
            }
            if st.closed {
                return None;
            }
            st = self.cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn past_drop_grace(&self) -> bool {
        self.lock()
            .dropped_at
            .is_some_and(|t| t.elapsed() > DROP_GRACE)
    }
}

/// Queues `line` for connection `conn`, which is dropped when its queue is full. False once
/// the connection is closed or dropped.
pub(crate) fn deliver(hub: &Hub, conn: u64, outbox: &Outbox, line: Arc<[u8]>) -> bool {
    match outbox.push(line) {
        Ok(()) => true,
        Err(PushError::Closed) => false,
        Err(PushError::Full) => {
            drop_slow(conn, outbox);
            hub.unsubscribe(conn);
            false
        }
    }
}

/// Sends `disconnect` and closes the connection: its queue passed the bound.
fn drop_slow(conn: u64, outbox: &Outbox) {
    let reason = format!("its queue passed {} bytes", outbox.limit);
    warn!(
        "{}",
        tern_core::logmsg::socket_connection_dropped(conn, &reason)
    );
    outbox.drop_with(disconnect_line(&format!(
        "the connection fell behind: {reason}"
    )));
}

// -- Delta hub ------------------------------------------------------------------------------

struct Subscriber {
    conn: u64,
    outbox: Arc<Outbox>,
    /// `None` for every account.
    accounts: Option<HashSet<String>>,
    classes: Option<HashSet<String>>,
    /// A gateway connection for a remote device.
    relayed: bool,
}

impl Subscriber {
    fn wants_account(&self, account_id: &str) -> bool {
        self.accounts
            .as_ref()
            .is_none_or(|a| a.contains(account_id))
    }

    /// `model_class` is `None` for an `account.state` notification, which no class filter
    /// holds back.
    fn wants(&self, account_id: &str, model_class: Option<&str>) -> bool {
        self.wants_account(account_id)
            && match (model_class, &self.classes) {
                (Some(class), Some(classes)) => classes.contains(class),
                _ => true,
            }
    }

    /// Only a local subscriber that wants every account and every class takes the backlog:
    /// a CLI watching one account or one class leaves it for the desktop, and taking it
    /// would throw away the other accounts' lines. A browser tab reconnecting through the
    /// gateway after an engine restart must not take the desktop's start-up deltas either.
    fn takes_backlog(&self) -> bool {
        self.classes.is_none() && self.accounts.is_none() && !self.relayed
    }
}

#[derive(Default)]
struct Backlog {
    lines: Vec<Arc<[u8]>>,
    bytes: usize,
}

struct HubState {
    subscribers: Vec<Subscriber>,
    /// Notifications sent before the first unfiltered `subscribe`, for it. `None` from then
    /// on, or once it passed [`SOCKET_BACKLOG_BYTES`].
    backlog: Option<Backlog>,
    /// The last `delta.seq` given out.
    seq: u64,
    /// The latest deltas, for `subscribe {since}`.
    ring: Ring,
    /// Tasks queued with `task.queue {notify}`, by account and task id, with the connection
    /// to send `task.done` to.
    waiting: HashMap<(String, String), Waiter>,
}

struct Waiter {
    conn: u64,
    outbox: Arc<Outbox>,
}

struct RingEntry {
    seq: u64,
    account_id: String,
    model_class: String,
    line: Arc<[u8]>,
}

/// The newest `delta` lines, bounded by [`DELTA_RING_BYTES`]; the newest one always stays.
#[derive(Default)]
struct Ring {
    entries: VecDeque<RingEntry>,
    bytes: usize,
}

impl Ring {
    fn push(&mut self, entry: RingEntry) {
        self.bytes += entry.line.len();
        self.entries.push_back(entry);
        while self.bytes > DELTA_RING_BYTES && self.entries.len() > 1 {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.line.len();
            }
        }
    }

    /// Whether every delta after `since` is still held, `latest` being the last one given
    /// out.
    fn holds_after(&self, since: u64, latest: u64) -> bool {
        since <= latest
            && match self.entries.front() {
                Some(first) => since + 1 >= first.seq,
                None => since == latest,
            }
    }
}

/// The socket `DeltaSink` of every account, as `delta` notifications, plus the
/// `account.state` notifications, to every matching subscriber.
pub struct Hub {
    limit: usize,
    /// Names this process's delta sequence (`subscribe`'s `feed`).
    feed: String,
    state: Mutex<HubState>,
    /// The live queries' thread, once the socket serves ([`crate::live`]).
    live: OnceLock<crate::live::Handle>,
}

struct HubSink {
    hub: Arc<Hub>,
    account_id: String,
    parent: Box<dyn DeltaSink>,
    /// The ids of each class this flush persisted or removed, for the live queries.
    touched: crate::live::Touched,
}

impl HubSink {
    fn write(&mut self, model_class: &str, item: Option<&DeltaItem>, line: &str) {
        if model_class == SECRETS_UPDATED {
            self.parent.write_item(model_class, line);
        } else if model_class == MAIL_NEW {
            self.hub.mail_new(&self.account_id, line);
        } else {
            self.hub
                .publish_delta(&self.account_id, model_class, item, line);
        }
    }
}

impl DeltaSink for HubSink {
    fn write_item(&mut self, model_class: &str, line: &str) {
        self.write(model_class, None, line);
    }

    fn write_delta(&mut self, item: &DeltaItem, line: &str) {
        crate::live::note_touched(&mut self.touched, item);
        match with_derived(item) {
            Some(derived) => self.write(&item.model_class, Some(item), &derived),
            None => self.write(&item.model_class, Some(item), line),
        }
    }

    fn flushed(&mut self) {
        if !self.touched.is_empty() {
            let touched = std::mem::take(&mut self.touched);
            if let Some(live) = self.hub.live.get() {
                live.changed(&self.account_id, touched);
            }
        }
    }
}

/// The line of a persist delta with the fields [`tern_store::derived`] adds, for the
/// classes that have them ([`feature::DERIVED_FIELDS`]); stdout's deltas stay as stored.
fn with_derived(item: &DeltaItem) -> Option<String> {
    if item.kind != tern_core::constants::DELTA_TYPE_PERSIST
        || !tern_store::derived::has_derived(&item.model_class)
    {
        return None;
    }
    let class = item.model_class.as_str();
    let models = item
        .model_jsons()
        .iter()
        .map(|m| tern_store::derived::annotated(class, m.clone()))
        .collect();
    Some(DeltaItem::new(&item.kind, class, models).dump())
}

impl Hub {
    pub fn new(limit: usize) -> Arc<Self> {
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Arc::new(Hub {
            limit,
            feed: format!("{}-{started:x}", std::process::id()),
            state: Mutex::new(HubState {
                subscribers: Vec::new(),
                backlog: Some(Backlog::default()),
                seq: 0,
                ring: Ring::default(),
                waiting: HashMap::new(),
            }),
            live: OnceLock::new(),
        })
    }

    /// The live queries' thread, for the socket's connections.
    pub(crate) fn live(&self) -> Option<&crate::live::Handle> {
        self.live.get()
    }

    pub(crate) fn set_live(&self, live: crate::live::Handle) {
        let _ = self.live.set(live);
    }

    /// The sink of `account_id`'s `DeltaStream`: [`SECRETS_UPDATED`] goes to `parent` (the
    /// parent's stdout), the rest to the subscribers.
    pub fn sink(
        self: &Arc<Self>,
        account_id: &str,
        parent: Box<dyn DeltaSink>,
    ) -> Box<dyn DeltaSink> {
        Box::new(HubSink {
            hub: self.clone(),
            account_id: account_id.to_owned(),
            parent,
            touched: Default::default(),
        })
    }

    /// Sends `account.state` to the subscribers of `params.account_id`. An account that
    /// starts or is removed has its rows reset or deleted without deltas, so the live queries
    /// over it read again.
    pub fn account_state(&self, params: &AccountStateParams) {
        let n = Notification::new(
            method::ACCOUNT_STATE,
            serde_json::to_value(params).unwrap_or(Value::Null),
        );
        self.publish(&params.account_id, None, tern_ipc::encode(&n).into());
        if matches!(
            params.state.as_str(),
            account_state::RUNNING | account_state::REMOVED
        ) {
            if let Some(live) = self.live.get() {
                live.rerun(&params.account_id);
            }
        }
    }

    /// Sends one flushed delta with the next `seq` to the matching subscribers, keeps it in
    /// the ring, and sends `task.done` for the waited-on tasks a `Task` delta finishes.
    fn publish_delta(
        &self,
        account_id: &str,
        model_class: &str,
        item: Option<&DeltaItem>,
        line: &str,
    ) {
        let mut guard = self.lock();
        let st = &mut *guard;
        st.seq += 1;
        let seq = st.seq;
        let note =
            framed(tern_ipc::delta_notification_line(account_id, Some(seq), line).into_bytes());
        st.ring.push(RingEntry {
            seq,
            account_id: account_id.to_owned(),
            model_class: model_class.to_owned(),
            line: note.clone(),
        });
        self.fan_out(st, account_id, Some(model_class), note, true);
        if model_class == TASK_CLASS && !st.waiting.is_empty() {
            for task in item.map(DeltaItem::model_jsons).unwrap_or_default() {
                finish_waiter(st, account_id, task);
            }
        }
    }

    /// Queues `response` (the `task.queue` answer, none for a notification) for `conn`, then
    /// waits for the task to finish, so `task.done` can't come first. False when the connection is gone.
    pub(crate) fn wait_for_task(
        &self,
        conn: u64,
        outbox: &Arc<Outbox>,
        account_id: &str,
        task_id: &str,
        response: Option<Arc<[u8]>>,
    ) -> bool {
        let mut st = self.lock();
        match response.map_or(Ok(()), |r| outbox.push(r)) {
            Ok(()) => {}
            Err(PushError::Closed) => return false,
            Err(PushError::Full) => {
                drop_slow(conn, outbox);
                st.subscribers.retain(|s| s.conn != conn);
                return false;
            }
        }
        st.waiting.insert(
            (account_id.to_owned(), task_id.to_owned()),
            Waiter {
                conn,
                outbox: outbox.clone(),
            },
        );
        true
    }

    /// `task.done` for `task` (its stored JSON) if it is finished and a connection waits
    /// for it.
    pub(crate) fn task_finished(&self, account_id: &str, task: &Value) {
        finish_waiter(&mut self.lock(), account_id, task);
    }

    /// Sends `mail.new` for each message in a [`MAIL_NEW`] item to the subscribers of
    /// `account_id`: no class filter holds it back, and
    /// nothing is kept for a later subscriber, since mail that arrives with no client
    /// connected is not announced.
    fn mail_new(&self, account_id: &str, line: &str) {
        let Ok(item) = serde_json::from_str::<Value>(line) else {
            return;
        };
        for m in item["modelJSONs"].as_array().into_iter().flatten() {
            let params = MailNewParams {
                account_id: account_id.to_owned(),
                thread_id: m["threadId"].as_str().unwrap_or("").to_owned(),
                message_id: m["id"].as_str().unwrap_or("").to_owned(),
            };
            let n = Notification::new(
                method::MAIL_NEW,
                serde_json::to_value(params).unwrap_or(Value::Null),
            );
            self.publish_to(account_id, None, tern_ipc::encode(&n).into(), false);
        }
    }

    /// Sends `rules.disabled` to every subscriber whatever its filter; nothing is kept for a
    /// later subscriber, which finds the rule disabled in `mailRules`.
    fn rules_disabled(&self, params: &RulesDisabledParams) {
        let n = Notification::new(
            method::RULES_DISABLED,
            serde_json::to_value(params).unwrap_or(Value::Null),
        );
        let note: Arc<[u8]> = tern_ipc::encode(&n).into();
        self.lock()
            .subscribers
            .retain(|s| match s.outbox.push(note.clone()) {
                Ok(()) => true,
                Err(PushError::Closed) => false,
                Err(PushError::Full) => {
                    drop_slow(s.conn, &s.outbox);
                    false
                }
            });
    }

    /// Sends `settings.changed` to every subscriber but `origin`, the connection that made
    /// the change: no filter holds it back, and nothing is
    /// kept for a later subscriber, which calls `settings.get` instead.
    fn settings_changed(&self, values: &serde_json::Map<String, Value>, origin: Option<u64>) {
        if values
            .keys()
            .any(|k| k.starts_with(crate::live::TRIAGE_SETTINGS_PREFIX))
        {
            if let Some(live) = self.live.get() {
                live.triage_settings_changed();
            }
        }
        let n = Notification::new(
            method::SETTINGS_CHANGED,
            serde_json::to_value(SettingsValues {
                values: values.clone(),
                store_id: None,
            })
            .unwrap_or(Value::Null),
        );
        let note: Arc<[u8]> = tern_ipc::encode(&n).into();
        self.lock().subscribers.retain(|s| {
            if Some(s.conn) == origin {
                return !s.outbox.is_closed();
            }
            match s.outbox.push(note.clone()) {
                Ok(()) => true,
                Err(PushError::Closed) => false,
                Err(PushError::Full) => {
                    drop_slow(s.conn, &s.outbox);
                    false
                }
            }
        });
    }

    /// Sends `account.changed` to the subscribers of the account,
    /// with the fields a remote device may see on a relayed one. Nothing is kept for a
    /// later subscriber, which calls `account.list`.
    pub fn account_changed(&self, params: &AccountChangedParams) {
        let line = |account: Value| -> Arc<[u8]> {
            let n = Notification::new(
                method::ACCOUNT_CHANGED,
                serde_json::to_value(AccountChangedParams {
                    account_id: params.account_id.clone(),
                    account,
                })
                .unwrap_or(Value::Null),
            );
            tern_ipc::encode(&n).into()
        };
        let local = line(params.account.clone());
        let public = line(policy::public_account_json(&params.account));
        self.lock().subscribers.retain(|s| {
            if !s.wants_account(&params.account_id) {
                return !s.outbox.is_closed();
            }
            let note = if s.relayed { &public } else { &local };
            match s.outbox.push(note.clone()) {
                Ok(()) => true,
                Err(PushError::Closed) => false,
                Err(PushError::Full) => {
                    drop_slow(s.conn, &s.outbox);
                    false
                }
            }
        });
    }

    /// Sends `capabilities.changed` to every subscriber, the connection whose change caused
    /// it included; nothing is kept for a later one, which reads `hello`.
    fn capabilities_changed(&self, capabilities: &Capabilities) {
        let n = Notification::new(
            method::CAPABILITIES_CHANGED,
            serde_json::to_value(capabilities).unwrap_or(Value::Null),
        );
        let note: Arc<[u8]> = tern_ipc::encode(&n).into();
        self.lock()
            .subscribers
            .retain(|s| match s.outbox.push(note.clone()) {
                Ok(()) => true,
                Err(PushError::Closed) => false,
                Err(PushError::Full) => {
                    drop_slow(s.conn, &s.outbox);
                    false
                }
            });
    }

    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn publish(&self, account_id: &str, model_class: Option<&str>, note: Arc<[u8]>) {
        self.publish_to(account_id, model_class, note, true);
    }

    /// `held` keeps the line for the first unfiltered subscriber while there is none.
    fn publish_to(&self, account_id: &str, model_class: Option<&str>, note: Arc<[u8]>, held: bool) {
        let mut guard = self.lock();
        self.fan_out(&mut guard, account_id, model_class, note, held);
    }

    fn fan_out(
        &self,
        st: &mut HubState,
        account_id: &str,
        model_class: Option<&str>,
        note: Arc<[u8]>,
        held: bool,
    ) {
        if st.subscribers.is_empty() && (st.backlog.is_none() || !held) {
            return;
        }
        if let Some(b) = st.backlog.as_mut().filter(|_| held) {
            if b.bytes + note.len() > SOCKET_BACKLOG_BYTES.min(self.limit) {
                warn!("-X Dropping the deltas held for the first subscriber: over the bound");
                st.backlog = None;
            } else {
                b.bytes += note.len();
                b.lines.push(note.clone());
            }
        }
        st.subscribers.retain(|s| {
            if !s.wants(account_id, model_class) {
                return !s.outbox.is_closed();
            }
            match s.outbox.push(note.clone()) {
                Ok(()) => true,
                Err(PushError::Closed) => false,
                Err(PushError::Full) => {
                    drop_slow(s.conn, &s.outbox);
                    false
                }
            }
        });
    }

    /// Adds or replaces `conn`'s subscription. The response to `id` (none for a
    /// notification) is queued under the hub lock, so every delta the connection gets comes
    /// after it. With `since`, the held deltas after it that the filter takes follow, when
    /// the ring still has all of them; without, the first subscriber that wants every class
    /// gets the backlog of its accounts.
    fn subscribe(&self, sub: Subscriber, id: Option<tern_ipc::Id>, since: Option<DeltaCursor>) {
        let mut guard = self.lock();
        let st = &mut *guard;
        st.subscribers.retain(|s| s.conn != sub.conn);
        let resumed = since
            .as_ref()
            .map(|c| c.feed == self.feed && st.ring.holds_after(c.seq, st.seq));
        if let Some(id) = id {
            let result = SubscribeResult {
                feed: self.feed.clone(),
                seq: st.seq,
                resumed,
            };
            let response = Response::ok(
                Some(id),
                serde_json::to_value(result).unwrap_or(Value::Null),
            );
            if let Err(e) = sub.outbox.push(tern_ipc::encode(&response).into()) {
                if matches!(e, PushError::Full) {
                    drop_slow(sub.conn, &sub.outbox);
                }
                return;
            }
        }
        if let Some(since) = &since {
            if resumed == Some(true) {
                let replay = st.ring.entries.iter().filter(|e| {
                    e.seq > since.seq && sub.wants(&e.account_id, Some(&e.model_class))
                });
                for e in replay {
                    if let Err(e) = sub.outbox.push(e.line.clone()) {
                        if matches!(e, PushError::Full) {
                            drop_slow(sub.conn, &sub.outbox);
                        }
                        return;
                    }
                }
            }
            st.subscribers.push(sub);
            return;
        }
        if let Some(backlog) = st.backlog.take_if(|_| sub.takes_backlog()) {
            for line in backlog.lines {
                if let Err(e) = sub.outbox.push(line) {
                    if matches!(e, PushError::Full) {
                        drop_slow(sub.conn, &sub.outbox);
                    }
                    return;
                }
            }
        }
        st.subscribers.push(sub);
    }

    fn unsubscribe(&self, conn: u64) {
        let mut st = self.lock();
        st.subscribers.retain(|s| s.conn != conn);
        st.waiting.retain(|_, w| w.conn != conn);
    }
}

/// The class of the deltas that finish a waited-on task.
const TASK_CLASS: &str = "Task";

/// Sends `task.done` for `task` and stops waiting for it, when it is `complete` or
/// `cancelled` and a connection waits.
fn finish_waiter(st: &mut HubState, account_id: &str, task: &Value) {
    let status = task["status"].as_str().unwrap_or("");
    if !matches!(status, task_status::COMPLETE | task_status::CANCELLED) {
        return;
    }
    let Some(id) = task["id"].as_str() else {
        return;
    };
    let Some(w) = st.waiting.remove(&(account_id.to_owned(), id.to_owned())) else {
        return;
    };
    let params = TaskDoneParams {
        account_id: account_id.to_owned(),
        task_id: id.to_owned(),
        status: status.to_owned(),
        error: task.get("error").filter(|e| !e.is_null()).cloned(),
        task: task.clone(),
    };
    let n = Notification::new(
        method::TASK_DONE,
        serde_json::to_value(params).unwrap_or(Value::Null),
    );
    if let Err(PushError::Full) = w.outbox.push(tern_ipc::encode(&n).into()) {
        drop_slow(w.conn, &w.outbox);
        st.subscribers.retain(|s| s.conn != w.conn);
        st.waiting.retain(|_, x| x.conn != w.conn);
    }
}

// -- Accounts -------------------------------------------------------------------------------

/// The accounts a socket serves and the way to each one's `main` thread. `sync` mode holds
/// one ([`SingleAccount`]); `serve` mode's registry is [`crate::serve`]'s.
pub trait Accounts: Send + Sync {
    /// Every account with its state, for `hello`, `engine.status` and `account.list`.
    fn statuses(&self) -> Vec<AccountStatus>;
    /// Ok when the engine holds `account_id`, in any state; [`code::UNKNOWN_ACCOUNT`]
    /// otherwise.
    fn check(&self, account_id: &str) -> Result<(), ErrorObject>;
    /// The command channel of a running account; [`code::ACCOUNT_STOPPED`] for one that
    /// isn't.
    fn commands(&self, account_id: &str) -> Result<Sender<Command>, ErrorObject>;
    /// The channels of every running account.
    fn all_commands(&self) -> Vec<Sender<Command>>;
    /// Whether the parent-only methods (`account.add`, ...) exist, so the socket refuses
    /// them rather than not knowing them.
    fn has_parent_channel(&self) -> bool;
    /// The registry and secret store (`serve`), for `hello.credentials`.
    fn credentials(&self) -> Option<CredentialsStatus> {
        None
    }
    /// `serve --detached`, which has no parent channel.
    fn detached(&self) -> bool {
        false
    }
    /// A parent method made on the socket of a local connection: `Some` when this engine
    /// takes it there (a detached `serve`).
    fn socket_parent_call(&self, _req: &RpcRequest) -> Option<SocketParentCall> {
        None
    }
    /// An account method ([`is_account_method`]): `Some` when this engine takes them
    /// (`serve`, on any connection the relay policy lets through).
    fn account_call(&self, _req: &RpcRequest) -> Option<SocketParentCall> {
        None
    }
    /// A stored secret that isn't an account's (`integration:<provider>`), for the engine's
    /// own calls; never sent to a client.
    fn secret(&self, _name: &str) -> Option<String> {
        None
    }
}

/// A job for a worker lane: the lane, the account it logs for, and the work.
pub type SocketJob = (
    &'static Lane,
    Option<Arc<str>>,
    Box<dyn FnOnce() -> Reply + Send>,
);

/// How the engine answers a parent or account method on its socket.
pub enum SocketParentCall {
    Now(Reply),
    /// Run on the lane's workers and answered when done, out of order with the
    /// connection's later requests.
    Later(SocketJob),
}

pub fn unknown_account(account_id: &str) -> ErrorObject {
    ErrorObject::new(
        code::UNKNOWN_ACCOUNT,
        format!("unknown account {account_id}"),
    )
}

/// `sync` mode's one account, running for as long as the process.
pub struct SingleAccount {
    pub account: Arc<Account>,
    pub deltas: Arc<DeltaStream>,
    pub commands: Sender<Command>,
}

impl Accounts for SingleAccount {
    fn statuses(&self) -> Vec<AccountStatus> {
        vec![AccountStatus {
            account_id: self.account.id().to_owned(),
            email: self.account.email_address().to_owned(),
            state: account_state::RUNNING.to_owned(),
            connection_error: self.deltas.connection_error(),
            error: None,
            account: Some(crate::registry::strip_secrets(self.account.as_json())),
            ..Default::default()
        }]
    }

    fn check(&self, account_id: &str) -> Result<(), ErrorObject> {
        if account_id == self.account.id() {
            return Ok(());
        }
        Err(unknown_account(account_id))
    }

    fn commands(&self, account_id: &str) -> Result<Sender<Command>, ErrorObject> {
        self.check(account_id)?;
        Ok(self.commands.clone())
    }

    fn all_commands(&self) -> Vec<Sender<Command>> {
        vec![self.commands.clone()]
    }

    fn has_parent_channel(&self) -> bool {
        false
    }
}

// -- Server ---------------------------------------------------------------------------------

/// What every connection shares.
pub struct Server {
    pub hub: Arc<Hub>,
    pub accounts: Arc<dyn Accounts>,
    pub started: Instant,
    /// Where the read methods open their own read-only database connection.
    pub config_dir: PathBuf,
    pub database: DatabaseGeneration,
    /// `serve` mode's settings store; `sync` mode has none, since several per-account
    /// processes would each hold their own copy of the file.
    pub settings: Option<Arc<Settings>>,
}

/// Bumped when `serve` replaces the database file (`engine.resetDatabase`), so each
/// session's read connection, still on the deleted file, is reopened.
#[derive(Clone, Debug, Default)]
pub struct DatabaseGeneration(Arc<AtomicU64>);

impl DatabaseGeneration {
    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Starts the live queries' thread and the acceptor thread on a bound listener.
pub fn serve(listener: UnixListener, server: Arc<Server>) {
    crate::live::start(&server);
    crate::modes::spawn(thread::SOCKET, move || accept_loop(&listener, &server));
}

fn accept_loop(listener: &UnixListener, server: &Arc<Server>) {
    let mut next_id: u64 = 0;
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                warn!("-X Could not accept a socket connection: {e}");
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        next_id += 1;
        match platform::uds::peer_is_same_user(&stream) {
            Ok(true) => match OpenConnection::take() {
                Some(slot) => start_connection(next_id, stream, server.clone(), slot),
                None => refuse_busy(next_id, stream),
            },
            Ok(false) => {
                warn!("-X Refusing socket connection {next_id}: the peer is another user");
                let _ = stream.shutdown(Shutdown::Both);
            }
            Err(e) => {
                warn!("-X Refusing socket connection {next_id}: no peer credentials ({e})");
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

static OPEN_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// One of the [`SOCKET_MAX_CONNECTIONS`] connection slots, given back when dropped.
struct OpenConnection;

impl OpenConnection {
    fn take() -> Option<OpenConnection> {
        OPEN_CONNECTIONS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < SOCKET_MAX_CONNECTIONS).then_some(n + 1)
            })
            .ok()
            .map(|_| OpenConnection)
    }
}

impl Drop for OpenConnection {
    fn drop(&mut self) {
        OPEN_CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn refuse_busy(id: u64, mut stream: UnixStream) {
    warn!("-X Refusing socket connection {id}: {SOCKET_MAX_CONNECTIONS} are open");
    let e = ErrorObject::new(
        code::TOO_MANY_CONNECTIONS,
        format!("the engine already has {SOCKET_MAX_CONNECTIONS} connections open"),
    );
    let _ = stream.set_write_timeout(Some(WRITE_POLL));
    let _ = stream.write_all(&tern_ipc::encode(&Response::err(None, e)));
    let _ = stream.shutdown(Shutdown::Both);
}

fn start_connection(id: u64, stream: UnixStream, server: Arc<Server>, slot: OpenConnection) {
    let write_half = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            warn!("-X Dropping socket connection {id}: {e}");
            return;
        }
    };
    let outbox = Arc::new(Outbox::new(server.hub.limit));
    let o = outbox.clone();
    let writer = std::thread::Builder::new()
        .name(thread::SOCKET_WRITER.into())
        .spawn(move || write_loop(&write_half, &o));
    if let Err(e) = writer {
        warn!("-X Dropping socket connection {id}: {e}");
        return;
    }
    let conn = Connection {
        id,
        reads: crate::read_pool::ReadPool::new(id, server.clone(), outbox.clone()),
        session: Session::new(server),
        outbox: outbox.clone(),
        hello: false,
        relay: None,
        ai_streams: Arc::default(),
        rules_jobs: Default::default(),
    };
    let reader = std::thread::Builder::new()
        .name(thread::SOCKET_READER.into())
        .spawn(move || {
            let _slot = slot;
            conn.read_loop(stream)
        });
    if let Err(e) = reader {
        warn!("-X Dropping socket connection {id}: {e}");
        outbox.close();
    }
}

fn write_loop(stream: &UnixStream, outbox: &Outbox) {
    let _ = stream.set_write_timeout(Some(WRITE_POLL));
    while let Some(line) = outbox.pop() {
        if !write_line(stream, &line, outbox) {
            break;
        }
    }
    outbox.kill();
    let _ = stream.shutdown(Shutdown::Both);
}

/// Writes all of `line`, giving up once the connection has been dropped for longer than
/// [`DROP_GRACE`] or the peer is gone.
fn write_line(mut stream: &UnixStream, mut line: &[u8], outbox: &Outbox) -> bool {
    while !line.is_empty() {
        match stream.write(line) {
            Ok(0) => return false,
            Ok(n) => line = &line[n..],
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if outbox.past_drop_grace() {
                    return false;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

// -- Methods --------------------------------------------------------------------------------

/// The methods the socket and `serve` mode's parent channel both answer, with the channel's
/// read-only database connection.
pub struct Session {
    pub server: Arc<Server>,
    pub(crate) reads: ReadApi,
}

impl Session {
    pub fn new(server: Arc<Server>) -> Self {
        let reads = ReadApi::new(
            server.config_dir.clone(),
            server.accounts.clone(),
            server.database.clone(),
        );
        Session { server, reads }
    }

    /// The reply to `req` when it is one of the shared methods, `None` otherwise.
    pub fn call(&mut self, req: &RpcRequest) -> Option<Reply> {
        let accounts = &self.server.accounts;
        Some(match req.method.as_str() {
            method::HELLO => self.hello(req),
            method::ENGINE_STATUS => Ok(self.status()),
            method::ACCOUNT_LIST => to_value(serde_json::to_value(AccountListResult {
                accounts: accounts.statuses(),
            })),
            name if is_read_method(name) => self.reads.call(req),
            method::TASK_QUEUE => req.params_as::<TaskQueueParams>().and_then(|p| {
                accounts.check(&p.account_id)?;
                if !p.task.is_object() {
                    return Err(ErrorObject::new(
                        code::INVALID_PARAMS,
                        "the task must be an object",
                    ));
                }
                self.dispatch(&p.account_id, Request::QueueTask(p.task))
            }),
            method::TASK_CANCEL => req
                .params_as::<TaskCancelParams>()
                .and_then(|p| self.dispatch(&p.account_id, Request::CancelTask(p.task_id))),
            method::SYNC_WAKE => {
                req.params_as::<SyncWakeParams>()
                    .and_then(|p| match p.account_id {
                        Some(a) => self.dispatch(&a, Request::WakeWorkers),
                        None => self.wake_all(),
                    })
            }
            method::BODIES_FETCH => req
                .params_as::<BodiesFetchParams>()
                .and_then(|p| self.dispatch(&p.account_id, Request::NeedBodies(p.ids))),
            method::THREADS_SNOOZE => req.params_as::<ThreadsSnoozeParams>().and_then(|p| {
                accounts.check(&p.account_id)?;
                if p.until <= tern_core::Clock::now(&tern_core::SystemClock) {
                    return Err(ErrorObject::new(
                        code::INVALID_PARAMS,
                        "invalid params: until must be in the future",
                    ));
                }
                self.dispatch(
                    &p.account_id,
                    Request::Snooze {
                        thread_ids: p.thread_ids,
                        until: p.until,
                    },
                )
            }),
            method::THREADS_UNSNOOZE => req.params_as::<ThreadsUnsnoozeParams>().and_then(|p| {
                accounts.check(&p.account_id)?;
                self.dispatch(
                    &p.account_id,
                    Request::Unsnooze {
                        thread_ids: p.thread_ids,
                    },
                )
            }),
            method::CALENDAR_SYNC => req
                .params_as::<CalendarSyncParams>()
                .and_then(|p| self.dispatch(&p.account_id, Request::SyncCalendar)),
            method::ENGINE_LOGS_TAIL => req
                .params_as::<LogsTailParams>()
                .and_then(|p| logs_tail(&self.server.config_dir, &p)),
            method::SETTINGS_GET | method::SETTINGS_SET if self.server.settings.is_some() => {
                settings_call(&self.server, req, None, |_| {})
            }
            name if crate::templates_api::is_template_method(name)
                && self.server.settings.is_some() =>
            {
                crate::templates_api::call(self, req)
            }
            name if crate::drafts_api::is_draft_method(name) && self.server.settings.is_some() => {
                crate::drafts_api::call(self, req)
            }
            name if crate::pim_api::is_pim_method(name) => crate::pim_api::call(self, req),
            _ => return None,
        })
    }

    fn hello(&self, req: &RpcRequest) -> Reply {
        let p = req.params_as::<HelloParams>()?;
        if p.protocol != PROTOCOL_VERSION {
            return Err(ErrorObject::new(
                code::UNSUPPORTED_PROTOCOL,
                format!(
                    "protocol {} is not supported; this engine speaks {PROTOCOL_VERSION}",
                    p.protocol
                ),
            )
            .with_data(json!({ "supported": [PROTOCOL_VERSION] })));
        }
        let accounts = self
            .server
            .accounts
            .statuses()
            .into_iter()
            .map(|a| AccountInfo {
                account_id: a.account_id,
                email: a.email,
                state: a.state,
                error: a.error,
                retry_at: a.retry_at,
                auth_failed: a.auth_failed,
            })
            .collect();
        let engine = &self.server.accounts;
        to_value(serde_json::to_value(HelloResult {
            protocol: PROTOCOL_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
            accounts,
            features: features(&self.server),
            capabilities: capabilities(&self.server),
            build: build_id(),
            detached: engine.detached(),
            credentials: engine.credentials(),
        }))
    }

    fn status(&self) -> Value {
        to_value(serde_json::to_value(EngineStatusResult {
            protocol: PROTOCOL_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
            uptime_secs: self.server.started.elapsed().as_secs(),
            accounts: self.server.accounts.statuses(),
            features: features(&self.server),
            capabilities: capabilities(&self.server),
            build: build_id(),
            detached: self.server.accounts.detached(),
            credentials: self.server.accounts.credentials(),
        }))
        .unwrap_or(Value::Null)
    }

    /// Hands `request` to the account's `main` thread and waits for its result.
    pub(crate) fn dispatch(&self, account_id: &str, request: Request) -> Reply {
        dispatch_on(&self.server, account_id, request)
    }

    /// `sync.wake` without an account: every running account, in parallel.
    fn wake_all(&self) -> Reply {
        let pending: Vec<_> = self
            .server
            .accounts
            .all_commands()
            .iter()
            .filter_map(|c| send(c, Request::WakeWorkers).ok())
            .collect();
        for rx in pending {
            let _ = rx.recv();
        }
        Ok(json!({}))
    }
}

/// [`Session::dispatch`] from any thread.
pub(crate) fn dispatch_on(server: &Server, account_id: &str, request: Request) -> Reply {
    let commands = server.accounts.commands(account_id)?;
    let rx = send(&commands, request)?;
    rx.recv()
        .unwrap_or_else(|_| Err(stopped_before_answering()))
}

/// `hello.features` and `engine.status.features`.
fn features(server: &Server) -> Vec<String> {
    let mut out: Vec<String> = [
        feature::COUNTS,
        feature::TASKS_LIST,
        feature::METADATA_LIST,
        feature::MESSAGE_HIDDEN,
        feature::MESSAGE_WARNINGS,
        feature::MAIL_NEW,
        feature::RELAYED,
        feature::QUERY,
        feature::SEARCH_GRAMMAR,
        feature::FILES,
        feature::SNOOZE,
        feature::UNSUBSCRIBE,
        feature::CALENDAR_EDITING,
        feature::CONTACT_EDITING,
        feature::LIVE_QUERIES,
        feature::EVENTS_SUBSCRIBE,
        feature::DELTA_SEQ,
        feature::TASK_DONE,
        feature::READ_LIMITS,
        feature::THREAD_SUMMARY,
        feature::DERIVED_FIELDS,
        feature::READ_POOL,
    ]
    .map(str::to_owned)
    .to_vec();
    if server.accounts.credentials().is_some() {
        out.push(feature::ACCOUNTS.to_owned());
        out.push(feature::ACCOUNT_SETUP.to_owned());
    }
    if server.settings.is_some() {
        out.push(feature::SETTINGS.to_owned());
        // The settings store is serve's, and so is the executor that reads it, the
        // integrations, the AI methods and the capabilities that follow the settings.
        out.push(feature::EXPIRATION_EXECUTOR.to_owned());
        out.push(feature::INTEGRATIONS.to_owned());
        out.push(feature::CAPABILITIES.to_owned());
        out.push(feature::AI.to_owned());
        out.push(feature::AI_TRIAGE.to_owned());
        out.push(feature::AI_TRIAGE_SUBSCRIBE.to_owned());
        out.push(feature::DRAFT_PIPELINE.to_owned());
        out.push(feature::DRAFT_EDITING.to_owned());
        out.push(feature::TEMPLATES.to_owned());
        out.push(feature::TRACKING.to_owned());
        out.push(feature::RULES.to_owned());
        if server.accounts.credentials().is_some() {
            out.push(feature::AI_KEYS.to_owned());
        }
    }
    out
}

/// `serve`'s `core.ai.*` settings; none without a store, or when it can't be read.
fn stored_ai(server: &Server) -> StoredAi {
    stored_ai_in(server.settings.as_ref())
}

/// [`stored_ai`] from the settings store itself.
pub(crate) fn stored_ai_in(settings: Option<&Arc<Settings>>) -> StoredAi {
    settings
        .and_then(|s| s.lock().get(None).ok())
        .map(|values| StoredAi::from_values(&values))
        .unwrap_or_default()
}

/// Sends `capabilities.changed` with the current capabilities: an AI triage pause began or
/// ended.
/// Disables a failing mail rule in `mailRules` and sends `settings.changed` (to every
/// subscriber: the engine made the change) and `rules.disabled`. False when the rule is gone,
/// disabled already, or the store can't be written.
pub fn disable_rule(server: &Server, rule_id: &str, error: &str) -> bool {
    let Some(store) = &server.settings else {
        return false;
    };
    let key = [MAIL_RULES_KEY.to_owned()];
    let mut guard = store.lock();
    let Ok(mut current) = guard.get(Some(&key)) else {
        return false;
    };
    let Some(values) =
        crate::rules::disabled_values(current.remove(&key[0]).as_ref(), rule_id, error)
    else {
        return false;
    };
    match guard.set(values) {
        Ok(changed) if !changed.is_empty() => {
            server.hub.settings_changed(&changed, None);
            server.hub.rules_disabled(&RulesDisabledParams {
                rule_id: rule_id.to_owned(),
                error: error.to_owned(),
            });
            true
        }
        Ok(_) => false,
        Err(e) => {
            warn!("-X Could not disable mail rule {rule_id}: {}", e.message);
            false
        }
    }
}

pub fn publish_capabilities(server: &Server) {
    if let Some(caps) = capabilities(server) {
        server.hub.capabilities_changed(&caps);
    }
}

/// `serve`'s capabilities; `sync` mode has none.
fn capabilities(server: &Server) -> Option<Capabilities> {
    server
        .settings
        .as_ref()
        .map(|store| capabilities_of(&store.lock()))
}

/// An unreadable store reads as empty: every integration not set up.
fn capabilities_of(guard: &SettingsGuard<'_>) -> Capabilities {
    let values = guard.get(None).unwrap_or_default();
    let mut caps = integrations_api::capabilities(&values);
    integrations_api::apply_server(&mut caps, crate::server_features::tracking());
    caps.ai = ai_api::capabilities(&StoredAi::from_values(&values));
    caps
}

/// `settings.get` or `settings.set` on `server`'s store, which must exist. `deliver` runs
/// with the store still locked, and so does the `settings.changed` a change sends to every
/// subscriber but `origin`: a connection that queues its answer in `deliver` gets it in
/// order with the notifications of other connections' changes.
fn settings_call(
    server: &Server,
    req: &RpcRequest,
    origin: Option<u64>,
    deliver: impl FnOnce(&Reply),
) -> Reply {
    let Some(store) = &server.settings else {
        return Err(ErrorObject::new(
            code::METHOD_NOT_FOUND,
            format!("unknown method {}", req.method),
        ));
    };
    let mut guard = store.lock();
    let result = if req.method == method::SETTINGS_GET {
        req.params_as::<SettingsGetParams>().and_then(|p| {
            to_value(serde_json::to_value(SettingsValues {
                values: guard.get(p.keys.as_deref())?,
                store_id: guard.store_id(),
            }))
        })
    } else {
        req.params_as::<SettingsValues>().and_then(|p| {
            let before = p
                .values
                .keys()
                .any(|k| is_capability_setting(k) || k.starts_with("core.ai."))
                .then(|| capabilities_of(&guard));
            // A provider's key goes before the write that removes or re-points the
            // provider; a write that would be refused deletes nothing.
            if let Some(after) = p.values.get(ai_api::PROVIDERS_KEY) {
                crate::settings::check(&p.values)?;
                let key = [ai_api::PROVIDERS_KEY.to_owned()];
                let before = guard.get(Some(&key))?;
                let after = Some(after).filter(|v| !v.is_null());
                ai_api::forget_unbound_keys(before.get(ai_api::PROVIDERS_KEY), after)
                    .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e))?;
            }
            let mut changed = guard.set(p.values)?;
            let now_ms = tern_core::Clock::now(&tern_core::SystemClock).saturating_mul(1000);
            let since = crate::rules::set_auto_since(&mut guard, &changed, now_ms);
            changed.extend(since);
            if !changed.is_empty() {
                server.hub.settings_changed(&changed, origin);
            }
            if let Some(before) = before {
                let after = capabilities_of(&guard);
                if after != before {
                    server.hub.capabilities_changed(&after);
                }
            }
            Ok(json!({}))
        })
    };
    deliver(&result);
    result
}

fn stopped_before_answering() -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, "the engine is shutting down")
}

fn send(
    commands: &Sender<Command>,
    request: Request,
) -> Result<mpsc::Receiver<Reply>, ErrorObject> {
    let (tx, rx) = mpsc::channel();
    commands
        .send(Command {
            request,
            reply: Some(tx),
        })
        .map_err(|_| stopped_before_answering())?;
    Ok(rx)
}

/// `engine.logs.tail`: the end of `engine-<accountId>.log` or `engine.log`. The account
/// need not be held, so a removed account's log can still be read.
fn logs_tail(config_dir: &Path, p: &LogsTailParams) -> Reply {
    use std::io::{Seek, SeekFrom};
    let path = match &p.account_id {
        Some(id) => {
            // The id becomes part of a file name.
            let valid = !id.is_empty()
                && id.len() <= 64
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid {
                return Err(ErrorObject::new(
                    code::INVALID_PARAMS,
                    format!("invalid accountId {id:?}"),
                ));
            }
            tern_core::log::log_path(config_dir, id)
        }
        None => config_dir.join(tern_core::constants::ENGINE_LOG_FILE),
    };
    let want = p
        .bytes
        .unwrap_or(LOGS_TAIL_DEFAULT_BYTES)
        .min(LOGS_TAIL_MAX_BYTES);
    let internal = |e: std::io::Error| {
        ErrorObject::new(
            code::INTERNAL_ERROR,
            format!("reading {}: {e}", path.display()),
        )
    };
    let mut file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return to_value(serde_json::to_value(LogsTailResult::default()))
        }
        Err(e) => return Err(internal(e)),
    };
    let len = file.metadata().map_err(internal)?.len();
    let start = len.saturating_sub(want);
    file.seek(SeekFrom::Start(start)).map_err(internal)?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.take(want).read_to_end(&mut buf).map_err(internal)?;
    to_value(serde_json::to_value(LogsTailResult {
        text: String::from_utf8_lossy(&buf).into_owned(),
        truncated: start > 0,
    }))
}

/// `account.list` and `engine.status` for a relayed connection: each account's JSON cut to
/// the fields a remote device may see.
fn public_accounts(method_name: &str, mut v: Value) -> Value {
    if matches!(method_name, method::ACCOUNT_LIST | method::ENGINE_STATUS) {
        for a in v["accounts"].as_array_mut().into_iter().flatten() {
            if let Some(account) = a.get_mut("account") {
                *account = policy::public_account_json(account);
            }
        }
    }
    v
}

/// The reads whose answers [`REPLY_MAX_BYTES`](tern_ipc::REPLY_MAX_BYTES) bounds.
fn is_budgeted(name: &str) -> bool {
    is_read_method(name) || name == method::EVENTS_EXPAND
}

/// `serve`'s account methods, on stdin and the socket.
pub fn is_account_method(name: &str) -> bool {
    matches!(
        name,
        method::ACCOUNT_DISCOVER
            | method::ACCOUNT_OAUTH_BEGIN
            | method::ACCOUNT_OAUTH_COMPLETE
            | method::ACCOUNT_ADD
            | method::ACCOUNT_TEST
            | method::ACCOUNT_SET_SECRETS
            | method::ACCOUNT_UPDATE
            | method::ACCOUNT_REORDER
            | method::ACCOUNT_REMOVE
            | method::ACCOUNT_RESET
            | method::SECRETS_SET
            | method::SECRETS_DELETE
            | method::SECRETS_HAS
            | method::SERVER_SET
            | method::SERVER_CLEAR
    )
}

/// The methods only `serve` mode's parent channel takes (and a detached engine's socket,
/// `credentials.import` aside).
pub fn is_parent_method(name: &str) -> bool {
    matches!(
        name,
        method::CREDENTIALS_IMPORT
            | method::ENGINE_RESET_DATABASE
            | method::ENGINE_SET_LOG_LEVEL
            | method::ENGINE_SHUTDOWN
            | method::TEST_CRASH
            | method::TEST_PANIC
    )
}

// -- Connection -----------------------------------------------------------------------------

struct Connection {
    id: u64,
    session: Session,
    outbox: Arc<Outbox>,
    hello: bool,
    /// Set by the first successful `hello` that says `relayed`; a later `hello` can't
    /// change it.
    relay: Option<Relay>,
    /// The connection's `ai.*` calls in flight, cancelled when it closes.
    ai_streams: Arc<Streams>,
    /// The connection's `rules.reprocess` jobs, cancelled when it closes.
    rules_jobs: crate::rules::ConnectionJobs,
    /// The read methods, answered out of order.
    reads: Arc<crate::read_pool::ReadPool>,
}

struct Relay {
    scopes: Vec<String>,
}

fn forbidden_relayed(what: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(
        code::FORBIDDEN_RELAYED,
        format!("{what} is not allowed on a relayed connection without the admin scope"),
    )
}

impl Connection {
    fn read_loop(mut self, stream: UnixStream) {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        loop {
            match tern_ipc::read_frame(&mut reader, &mut line, MAX_LINE_BYTES) {
                Ok(true) => {}
                Ok(false) | Err(FrameError::Io(_)) => break,
                Err(FrameError::TooLong) => {
                    let e = ErrorObject::new(
                        code::PARSE_ERROR,
                        format!("parse error: the line exceeds {MAX_LINE_BYTES} bytes"),
                    );
                    self.respond(Response::err(None, e));
                    break;
                }
            }
            let text = std::mem::take(&mut line);
            if text.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let response = match tern_ipc::parse_request(&text) {
                Ok(req) => self.handle(req),
                Err((id, e)) => Some(Response::err(id, e)),
            };
            if let Some(r) = response {
                if !self.respond(r) {
                    break;
                }
            }
        }
        self.session.server.hub.unsubscribe(self.id);
        if let Some(live) = self.session.server.hub.live() {
            live.closed(self.id);
        }
        self.ai_streams.close();
        self.rules_jobs.close();
        self.reads.close();
        self.outbox.close();
    }

    /// Queues a response. False once the connection is closed or dropped.
    fn respond(&self, response: Response) -> bool {
        self.respond_line(tern_ipc::encode(&response).into())
    }

    fn respond_line(&self, line: Arc<[u8]>) -> bool {
        match self.outbox.push(line) {
            Ok(()) => true,
            Err(PushError::Closed) => false,
            Err(PushError::Full) => {
                drop_slow(self.id, &self.outbox);
                self.session.server.hub.unsubscribe(self.id);
                false
            }
        }
    }

    /// The response to `req`, or `None` when there is none to send (a client notification,
    /// which runs but is not answered, or `subscribe`, which queues its own).
    fn handle(&mut self, mut req: RpcRequest) -> Option<Response> {
        if self.relay.is_some() && req.method == method::QUERY {
            policy::cap_relayed_query(&mut req.params);
        }
        let result = match req.method.as_str() {
            method::HELLO => {
                let r = self.session.call(&req).unwrap_or(Ok(Value::Null));
                if r.is_ok() && !self.hello {
                    self.hello = true;
                    self.on_first_hello(&req);
                }
                r
            }
            _ if !self.hello => Err(ErrorObject::new(
                code::HELLO_REQUIRED,
                "hello must be the first request",
            )),
            _ if self.relay_refuses(&req) => Err(forbidden_relayed(&req.method)),
            method::SUBSCRIBE => match self.subscribe(&req) {
                Ok(()) => return None,
                Err(e) => Err(e),
            },
            method::TASK_QUEUE
                if req.params.get("notify").and_then(Value::as_bool) == Some(true) =>
            {
                match self.task_queue_notify(&req) {
                    Ok(()) => return None,
                    Err(e) => Err(e),
                }
            }
            name if crate::live::is_live_method(name) => match self.session.server.hub.live() {
                Some(live) => {
                    live.call(self.id, self.outbox.clone(), self.relay.is_some(), req);
                    return None;
                }
                None => Err(ErrorObject::new(
                    code::METHOD_NOT_FOUND,
                    format!("unknown method {name}"),
                )),
            },
            name if is_read_method(name) => {
                self.reads.submit(req);
                return None;
            }
            method::SETTINGS_GET | method::SETTINGS_SET
                if self.session.server.settings.is_some() =>
            {
                let id = req.id.clone();
                // Answered from `deliver`, under the store's lock.
                let _ = settings_call(&self.session.server, &req, Some(self.id), |result| {
                    if let Some(id) = id {
                        self.respond(match result {
                            Ok(v) => Response::ok(Some(id), v.clone()),
                            Err(e) => Response::err(Some(id), e.clone()),
                        });
                    }
                });
                return None;
            }
            name if is_files_method(name) => match self.session.files_call(&req) {
                Deferred::Now(r) => r,
                Deferred::Later(work) => {
                    self.answer_later(&workers::FILES, req.id, work);
                    return None;
                }
            },
            method::MESSAGES_UNSUBSCRIBE => match self.session.unsubscribe_call(&req) {
                Deferred::Now(r) => r,
                Deferred::Later(work) => {
                    self.answer_later(&workers::UNSUBSCRIBE, req.id, work);
                    return None;
                }
            },
            // serve's, beside the settings that decide their capabilities.
            name if is_integrations_method(name) && self.session.server.settings.is_some() => {
                let id = req.id.clone();
                let accounts = self.session.server.accounts.clone();
                let store = self.session.server.settings.clone();
                self.answer_later(
                    &workers::INTEGRATIONS,
                    id,
                    Box::new(move || {
                        // Unreadable settings name no url, so the key goes to defaults only.
                        let settings = store
                            .as_ref()
                            .map(|s| s.lock().get(None).unwrap_or_default())
                            .unwrap_or_default();
                        let req = integrations_api::with_stored_key(req, &settings, |n| {
                            accounts.secret(n)
                        });
                        integrations_api::call(&req)
                    }),
                );
                return None;
            }
            method::AI_CANCEL if self.session.server.settings.is_some() => {
                self.ai_streams.cancel(&req)
            }
            method::AI_USAGE if self.session.server.settings.is_some() => ai_api::usage(&req),
            method::AI_TRIAGE_RUN if self.session.server.settings.is_some() => {
                match self.triage_run(&req) {
                    Ok(()) => return None,
                    Err(e) => Err(e),
                }
            }
            method::RULES_REPROCESS if self.session.server.settings.is_some() => {
                match self.rules_reprocess(&req) {
                    Ok(()) => return None,
                    Err(e) => Err(e),
                }
            }
            method::RULES_CANCEL if self.session.server.settings.is_some() => req
                .params_as::<RulesCancelParams>()
                .and_then(crate::rules::cancel),
            method::AI_TRIAGE_LIST if self.session.server.settings.is_some() => {
                let stored = stored_ai(&self.session.server);
                crate::ai_triage::list(
                    &req,
                    &mut self.session.reads,
                    &*self.session.server.accounts,
                    &stored,
                )
            }
            name if is_ai_method(name) && self.session.server.settings.is_some() => {
                match self.ai_call(&req) {
                    Ok(()) => return None,
                    Err(e) => Err(e),
                }
            }
            name if is_account_method(name) => {
                match self.session.server.accounts.account_call(&req) {
                    Some(SocketParentCall::Now(r)) => r,
                    Some(SocketParentCall::Later((lane, account, work))) => {
                        self.answer_later_for(lane, account, req.id, work);
                        return None;
                    }
                    None => Err(ErrorObject::new(
                        code::METHOD_NOT_FOUND,
                        format!("{name} is served by serve mode's engine only"),
                    )),
                }
            }
            name if is_parent_method(name) && self.session.server.accounts.has_parent_channel() => {
                if self.relay.is_none() {
                    match self.session.server.accounts.socket_parent_call(&req) {
                        Some(SocketParentCall::Now(r)) => {
                            return req.id.map(|id| match r {
                                Ok(v) => Response::ok(Some(id), v),
                                Err(e) => Response::err(Some(id), e),
                            });
                        }
                        Some(SocketParentCall::Later((lane, account, work))) => {
                            self.answer_later_for(lane, account, req.id, work);
                            return None;
                        }
                        None => {}
                    }
                }
                Err(ErrorObject::new(
                    code::WRONG_CHANNEL,
                    format!(
                        "{name} is only accepted on the engine's stdin, from the process that \
                         started it"
                    ),
                ))
            }
            other => self.session.call(&req).unwrap_or_else(|| {
                Err(ErrorObject::new(
                    code::METHOD_NOT_FOUND,
                    format!("unknown method {other}"),
                ))
            }),
        };
        let id = req.id?;
        let result = match result {
            Ok(v) if self.relay.is_some() => Ok(public_accounts(&req.method, v)),
            other => other,
        };
        match result {
            Ok(v) if is_budgeted(&req.method) => {
                let line: Arc<[u8]> = tern_ipc::encode(&Response::ok(Some(id.clone()), v)).into();
                match crate::read_api::over_budget(line.len()) {
                    Some(e) => Some(Response::err(Some(id), e)),
                    None => {
                        self.respond_line(line);
                        None
                    }
                }
            }
            Ok(v) => Some(Response::ok(Some(id), v)),
            Err(e) => Some(Response::err(Some(id), e)),
        }
    }

    /// Runs `work` on one of `lane`'s workers and queues its answer then, out of order with
    /// this connection's later requests (T21). A full lane answers `ENGINE_BUSY` now.
    fn answer_later(
        &self,
        lane: &'static Lane,
        id: Option<tern_ipc::Id>,
        work: Box<dyn FnOnce() -> Reply + Send>,
    ) {
        self.answer_later_for(lane, None, id, work);
    }

    /// [`Connection::answer_later`] for a job that logs to `account`'s log.
    fn answer_later_for(
        &self,
        lane: &'static Lane,
        account: Option<Arc<str>>,
        id: Option<tern_ipc::Id>,
        work: Box<dyn FnOnce() -> Reply + Send>,
    ) {
        let conn = self.id;
        let outbox = self.outbox.clone();
        let hub = self.session.server.hub.clone();
        lane.submit(account, work, move |result| {
            let Some(id) = id else {
                return;
            };
            let response = match result {
                Ok(v) => Response::ok(Some(id), v),
                Err(e) => Response::err(Some(id), e),
            };
            if let Err(PushError::Full) = outbox.push(tern_ipc::encode(&response).into()) {
                drop_slow(conn, &outbox);
                hub.unsubscribe(conn);
            }
        });
    }

    /// Checks an `ai.*` model call and reads its mail here, then runs it on the AI workers
    /// with a sink that sends its `ai.delta` notifications to this connection only, ahead
    /// of the response.
    fn ai_call(&mut self, req: &RpcRequest) -> Result<(), ErrorObject> {
        let stored = stored_ai(&self.session.server);
        let call = ai_api::prepare(
            req,
            &mut self.session.reads,
            &*self.session.server.accounts,
            &stored,
        )?;
        let registration = self.ai_streams.register(call.stream_id())?;
        let mut sink = AiSink {
            conn: self.id,
            outbox: self.outbox.clone(),
            hub: self.session.server.hub.clone(),
            stream_id: call.stream_id().map(str::to_owned),
            token: registration.token.clone(),
        };
        self.answer_later(
            &workers::AI,
            req.id.clone(),
            Box::new(move || {
                let reply = call.run(&mut sink);
                drop(registration);
                reply
            }),
        );
        Ok(())
    }

    /// `ai.triage.run`: queues the job on the account's triage thread and answers; its
    /// `ai.triage.progress` notifications come after the answer, to this connection only.
    fn triage_run(&mut self, req: &RpcRequest) -> Result<(), ErrorObject> {
        let stored = stored_ai(&self.session.server);
        let (conn, outbox, hub) = (
            self.id,
            self.outbox.clone(),
            self.session.server.hub.clone(),
        );
        let progress: crate::ai_triage::Progress = Box::new(move |p: &AiTriageProgressParams| {
            let n = Notification::new(
                method::AI_TRIAGE_PROGRESS,
                serde_json::to_value(p).unwrap_or(Value::Null),
            );
            if let Err(PushError::Full) = outbox.push(tern_ipc::encode(&n).into()) {
                drop_slow(conn, &outbox);
                hub.unsubscribe(conn);
            }
        });
        let id = req.id.clone();
        let (answer_outbox, answer_hub) = (self.outbox.clone(), self.session.server.hub.clone());
        let respond = move |result: Value| {
            let Some(id) = id else {
                return;
            };
            let line = tern_ipc::encode(&Response::ok(Some(id), result));
            if let Err(PushError::Full) = answer_outbox.push(line.into()) {
                drop_slow(conn, &answer_outbox);
                answer_hub.unsubscribe(conn);
            }
        };
        let job = crate::ai_triage::run(
            req,
            &mut self.session.reads,
            &*self.session.server.accounts,
            &stored,
            progress,
            respond,
        )?;
        self.ai_streams.add_job(&job);
        Ok(())
    }

    /// `rules.reprocess`: queues the job on the account's rules thread and answers; its
    /// `rules.progress` notifications come after the answer, to this connection only.
    fn rules_reprocess(&mut self, req: &RpcRequest) -> Result<(), ErrorObject> {
        let p = req.params_as::<RulesReprocessParams>()?;
        let (conn, outbox, hub) = (
            self.id,
            self.outbox.clone(),
            self.session.server.hub.clone(),
        );
        let progress: crate::rules::Progress = Box::new(move |p: &RulesProgressParams| {
            let n = Notification::new(
                method::RULES_PROGRESS,
                serde_json::to_value(p).unwrap_or(Value::Null),
            );
            if let Err(PushError::Full) = outbox.push(tern_ipc::encode(&n).into()) {
                drop_slow(conn, &outbox);
                hub.unsubscribe(conn);
            }
        });
        let id = req.id.clone();
        let (answer_outbox, answer_hub) = (self.outbox.clone(), self.session.server.hub.clone());
        let respond = move |result: Value| {
            let Some(id) = id else {
                return;
            };
            let line = tern_ipc::encode(&Response::ok(Some(id), result));
            if let Err(PushError::Full) = answer_outbox.push(line.into()) {
                drop_slow(conn, &answer_outbox);
                answer_hub.unsubscribe(conn);
            }
        };
        let accounts = self.session.server.accounts.clone();
        let reads = self.session.reads.store()?;
        crate::rules::reprocess(p, reads, &*accounts, &self.rules_jobs, progress, respond)
    }

    fn on_first_hello(&mut self, req: &RpcRequest) {
        let Ok(p) = req.params_as::<HelloParams>() else {
            return;
        };
        if !p.relayed {
            info!("Socket connection {}: hello from {:?}", self.id, p.client);
            return;
        }
        let RelayDevice { id, name } = p.device.unwrap_or(RelayDevice {
            id: String::new(),
            name: String::new(),
        });
        info!(
            "Socket connection {}: hello from {:?}, relayed for device {id} {name:?} with \
             scopes {:?}",
            self.id, p.client, p.scopes
        );
        self.relay = Some(Relay { scopes: p.scopes });
    }

    /// Whether this is a relayed connection that may not make `req` (tern-ipc's `policy`).
    fn relay_refuses(&self, req: &RpcRequest) -> bool {
        let Some(relay) = &self.relay else {
            return false;
        };
        !policy::relayed_request_allowed(&req.method, &req.params, &relay.scopes)
    }

    fn subscribe(&self, req: &RpcRequest) -> Result<(), ErrorObject> {
        let p = req.params_as::<SubscribeParams>()?;
        if let Some(list) = &p.accounts {
            for a in list {
                self.session.server.accounts.check(a)?;
            }
        }
        self.session.server.hub.subscribe(
            Subscriber {
                conn: self.id,
                outbox: self.outbox.clone(),
                accounts: p.accounts.map(|a| a.into_iter().collect()),
                classes: p.classes.map(|c| c.into_iter().collect()),
                relayed: self.relay.is_some(),
            },
            req.id.clone(),
            p.since,
        );
        Ok(())
    }

    /// `task.queue {notify}`: answers as `task.queue` does, then sends `task.done` when the
    /// task finishes, from its `Task` delta or, when it finished before the wait began, from
    /// the stored row.
    fn task_queue_notify(&mut self, req: &RpcRequest) -> Result<(), ErrorObject> {
        let p = req.params_as::<TaskQueueParams>()?;
        let task = self.session.call(req).unwrap_or_else(|| {
            Err(ErrorObject::new(
                code::INTERNAL_ERROR,
                "task.queue is not served",
            ))
        })?;
        let response: Option<Arc<[u8]>> = req
            .id
            .clone()
            .map(|id| tern_ipc::encode(&Response::ok(Some(id), task.clone())).into());
        let Some(task_id) = task["id"].as_str() else {
            if let Some(line) = response {
                self.respond_line(line);
            }
            return Ok(());
        };
        let hub = self.session.server.hub.clone();
        if !hub.wait_for_task(self.id, &self.outbox, &p.account_id, task_id, response) {
            return Ok(());
        }
        let stored = match task["status"].as_str() {
            Some(task_status::COMPLETE | task_status::CANCELLED) => Some(task.clone()),
            _ => self
                .session
                .reads
                .store()
                .ok()
                .and_then(|s| s.task(&p.account_id, task_id).ok().flatten()),
        };
        if let Some(t) = stored {
            hub.task_finished(&p.account_id, &t);
        }
        Ok(())
    }
}

/// Where an `ai.*` call's text goes as it streams: `ai.delta` on the calling connection,
/// when the call named a `streamId`. A closed or dropped connection stops the call.
struct AiSink {
    conn: u64,
    outbox: Arc<Outbox>,
    hub: Arc<Hub>,
    stream_id: Option<String>,
    token: tern_ai::CancelToken,
}

impl tern_ai::Sink for AiSink {
    fn delta(&mut self, text: &str) -> ControlFlow<()> {
        let Some(stream_id) = &self.stream_id else {
            return ControlFlow::Continue(());
        };
        let n = Notification::new(
            method::AI_DELTA,
            serde_json::to_value(AiDeltaParams {
                stream_id: stream_id.clone(),
                text: text.to_owned(),
            })
            .unwrap_or(Value::Null),
        );
        match self.outbox.push(tern_ipc::encode(&n).into()) {
            Ok(()) => ControlFlow::Continue(()),
            Err(PushError::Closed) => ControlFlow::Break(()),
            Err(PushError::Full) => {
                drop_slow(self.conn, &self.outbox);
                self.hub.unsubscribe(self.conn);
                ControlFlow::Break(())
            }
        }
    }

    fn cancel_token(&self) -> Option<&tern_ai::CancelToken> {
        Some(&self.token)
    }
}

fn to_value(v: serde_json::Result<Value>) -> Reply {
    v.map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<String>>>);

    impl DeltaSink for Captured {
        fn write_item(&mut self, _model_class: &str, line: &str) {
            self.0.lock().unwrap().push(line.to_owned());
        }
    }

    fn subscriber(conn: u64, classes: Option<&[&str]>) -> (Subscriber, Arc<Outbox>) {
        filtered_subscriber(conn, None, classes)
    }

    fn filtered_subscriber(
        conn: u64,
        accounts: Option<&[&str]>,
        classes: Option<&[&str]>,
    ) -> (Subscriber, Arc<Outbox>) {
        let set = |l: &[&str]| l.iter().map(|s| (*s).to_owned()).collect();
        let outbox = Arc::new(Outbox::new(1 << 20));
        let sub = Subscriber {
            conn,
            outbox: outbox.clone(),
            accounts: accounts.map(set),
            classes: classes.map(set),
            relayed: false,
        };
        (sub, outbox)
    }

    fn queued(outbox: &Outbox) -> Vec<String> {
        outbox
            .lock()
            .lines
            .iter()
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect()
    }

    #[test]
    fn secrets_go_to_the_parent_only() {
        let hub = Hub::new(1 << 20);
        let parent = Captured::default();
        let mut sink = hub.sink("a1", Box::new(parent.clone()));
        let (sub, outbox) = subscriber(1, None);
        hub.subscribe(sub, None, None);

        let secret = r#"{"modelClass":"ProcessAccountSecretsUpdated","modelJSONs":[{"refresh":"s3cret"}],"type":"persist"}"#;
        sink.write_item(SECRETS_UPDATED, secret);
        sink.write_item(
            "Task",
            r#"{"modelClass":"Task","modelJSONs":[],"type":"persist"}"#,
        );

        assert_eq!(*parent.0.lock().unwrap(), vec![secret.to_owned()]);
        let lines = queued(&outbox);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"Task\""));
        assert!(!lines.concat().contains("s3cret"));
    }

    #[test]
    fn secrets_stay_out_of_the_backlog() {
        let hub = Hub::new(1 << 20);
        let mut sink = hub.sink("a1", Box::new(Captured::default()));
        sink.write_item(
            SECRETS_UPDATED,
            r#"{"modelClass":"ProcessAccountSecretsUpdated","modelJSONs":[{"refresh":"s3cret"}]}"#,
        );
        let (sub, outbox) = subscriber(1, None);
        hub.subscribe(sub, None, None);
        assert!(queued(&outbox).is_empty());
    }

    #[test]
    fn a_filtered_subscriber_leaves_the_backlog() {
        let hub = Hub::new(1 << 20);
        let mut sink = hub.sink("a1", Box::new(Captured::default()));
        sink.write_item("ProcessState", r#"{"modelClass":"ProcessState"}"#);
        sink.write_item("Task", r#"{"modelClass":"Task"}"#);

        let (filtered, f_out) = subscriber(1, Some(&["Task"]));
        hub.subscribe(filtered, Some(tern_ipc::Id::Num(1)), None);
        let lines = queued(&f_out);
        assert_eq!(lines.len(), 1);
        let answer: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(answer["result"]["seq"], 2);

        let (all, a_out) = subscriber(2, None);
        hub.subscribe(all, None, None);
        let lines = queued(&a_out);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("ProcessState"));

        // Taken: a later subscriber gets live deltas only.
        let (late, l_out) = subscriber(3, None);
        hub.subscribe(late, None, None);
        assert!(queued(&l_out).is_empty());
    }

    /// A subscriber filtered to one account leaves the backlog, every account's lines in it,
    /// for the first unfiltered one.
    #[test]
    fn an_account_filtered_subscriber_leaves_the_backlog() {
        let hub = Hub::new(1 << 20);
        let mut a = hub.sink("a1", Box::new(Captured::default()));
        let mut b = hub.sink("b1", Box::new(Captured::default()));
        a.write_item("Task", r#"{"modelClass":"Task","from":"a1"}"#);
        b.write_item("Task", r#"{"modelClass":"Task","from":"b1"}"#);

        let (watch, w_out) = filtered_subscriber(1, Some(&["a1"]), None);
        hub.subscribe(watch, None, None);
        assert!(queued(&w_out).is_empty());

        let (all, a_out) = subscriber(2, None);
        hub.subscribe(all, None, None);
        let lines = queued(&a_out).concat();
        assert!(lines.contains("\"accountId\":\"a1\""), "{lines}");
        assert!(lines.contains("\"accountId\":\"b1\""), "{lines}");
    }

    /// `mail.new` routing: by account only, one per
    /// message, and never held for a later subscriber.
    #[test]
    fn mail_new_goes_live_to_the_account_subscribers() {
        let hub = Hub::new(1 << 20);
        let mut a = hub.sink("a1", Box::new(Captured::default()));
        let item = r#"{"modelClass":"MailNew","modelJSONs":[{"id":"m1","threadId":"t1"},{"id":"m2","threadId":"t1"}],"type":"persist"}"#;
        a.write_item(MAIL_NEW, item);
        let (early, e_out) = subscriber(1, None);
        hub.subscribe(early, None, None);
        assert!(queued(&e_out).is_empty(), "no backlog for mail.new");

        let (tasks_only, t_out) = subscriber(2, Some(&["Task"]));
        hub.subscribe(tasks_only, None, None);
        let (other, o_out) = filtered_subscriber(3, Some(&["b1"]), None);
        hub.subscribe(other, None, None);
        a.write_item(MAIL_NEW, item);
        for out in [&e_out, &t_out] {
            let lines = queued(out);
            assert_eq!(lines.len(), 2, "{lines:?}");
            let n: Value = serde_json::from_str(&lines[0]).unwrap();
            assert_eq!(n["method"], method::MAIL_NEW);
            assert_eq!(
                n["params"],
                json!({"accountId": "a1", "threadId": "t1", "messageId": "m1"})
            );
        }
        assert!(queued(&o_out).is_empty());
    }

    #[test]
    fn mail_new_never_reaches_stdout() {
        let mut sink = tern_store::WriteSink(Vec::new());
        sink.write_item(MAIL_NEW, r#"{"modelClass":"MailNew"}"#);
        sink.write_item("Task", r#"{"modelClass":"Task"}"#);
        assert_eq!(
            String::from_utf8(sink.0).unwrap(),
            "{\"modelClass\":\"Task\"}\n"
        );
    }

    fn delta(class: &str, models: Value) -> DeltaItem {
        DeltaItem::new("persist", class, serde_json::from_value(models).unwrap())
    }

    fn params(line: &str) -> Value {
        serde_json::from_str::<Value>(line).unwrap()["params"].clone()
    }

    #[test]
    fn deltas_carry_a_sequence_and_resume_after_it() {
        let hub = Hub::new(1 << 20);
        let mut a = hub.sink("a1", Box::new(Captured::default()));
        let (first, f_out) = subscriber(1, None);
        hub.subscribe(first, Some(tern_ipc::Id::Num(1)), None);
        for (i, class) in ["Thread", "Task", "Thread"].iter().enumerate() {
            let item = delta(class, json!([{"id": format!("x{i}")}]));
            a.write_delta(&item, &item.dump());
        }
        let lines = queued(&f_out);
        let start: Value = serde_json::from_str(&lines[0]).unwrap();
        let feed = start["result"]["feed"].as_str().unwrap().to_owned();
        assert_eq!(start["result"]["seq"], 0);
        let seqs: Vec<Value> = lines[1..]
            .iter()
            .map(|l| params(l)["seq"].clone())
            .collect();
        assert_eq!(seqs, [json!(1), json!(2), json!(3)]);

        // A Thread watcher that saw seq 1 gets 3 only; one from another engine gets nothing.
        let (back, b_out) = subscriber(2, Some(&["Thread"]));
        let since = DeltaCursor {
            feed: feed.clone(),
            seq: 1,
        };
        hub.subscribe(back, Some(tern_ipc::Id::Num(2)), Some(since));
        let lines = queued(&b_out);
        let answer: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(
            answer["result"],
            json!({"feed": feed, "seq": 3, "resumed": true})
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(params(&lines[1])["seq"], 3);

        let (stranger, s_out) = subscriber(3, None);
        let since = DeltaCursor {
            feed: "other".into(),
            seq: 1,
        };
        hub.subscribe(stranger, Some(tern_ipc::Id::Num(3)), Some(since));
        let lines = queued(&s_out);
        assert_eq!(lines.len(), 1);
        let answer: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(answer["result"]["resumed"], false);
    }

    #[test]
    fn the_ring_forgets_the_oldest_deltas() {
        let mut ring = Ring::default();
        let line: Arc<[u8]> = vec![b'x'; DELTA_RING_BYTES / 2 + 1].into();
        for seq in 1..=3 {
            ring.push(RingEntry {
                seq,
                account_id: "a".into(),
                model_class: "Thread".into(),
                line: line.clone(),
            });
        }
        assert_eq!(ring.entries.len(), 1);
        assert!(ring.holds_after(2, 3) && ring.holds_after(3, 3));
        assert!(!ring.holds_after(1, 3) && !ring.holds_after(4, 3));
        assert!(Ring::default().holds_after(0, 0));
    }

    #[test]
    fn task_done_goes_to_the_waiting_connection_once() {
        let hub = Hub::new(1 << 20);
        let mut a = hub.sink("a1", Box::new(Captured::default()));
        let outbox = Arc::new(Outbox::new(1 << 20));
        let answer: Arc<[u8]> = b"answer\n".to_vec().into();
        assert!(hub.wait_for_task(7, &outbox, "a1", "t1", Some(answer)));
        let remote = delta("Task", json!([{"id": "t1", "status": "remote"}]));
        a.write_delta(&remote, &remote.dump());
        assert_eq!(queued(&outbox), ["answer\n"]);

        let done = delta(
            "Task",
            json!([{"id": "t1", "status": "complete", "error": {"key": "ErrorX"}}]),
        );
        a.write_delta(&done, &done.dump());
        hub.task_finished("a1", &json!({"id": "t1", "status": "complete"}));
        a.write_delta(&done, &done.dump());
        let lines = queued(&outbox);
        assert_eq!(lines.len(), 2, "{lines:?}");
        let n: Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(n["method"], method::TASK_DONE);
        assert_eq!(n["params"]["taskId"], "t1");
        assert_eq!(n["params"]["status"], "complete");
        assert_eq!(n["params"]["error"]["key"], "ErrorX");

        // A closed connection stops waiting.
        assert!(hub.wait_for_task(8, &outbox, "a1", "t2", None));
        hub.unsubscribe(8);
        hub.task_finished("a1", &json!({"id": "t2", "status": "cancelled"}));
        assert_eq!(queued(&outbox).len(), 2);
    }
}
