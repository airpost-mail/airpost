// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine protocol on a Unix domain socket (docs/engine-protocol-plan.md sections 1, 2
//! and 4), for `sync --socket` and `serve`. Unix-only, like the transport; its OS calls are
//! in [`platform::uds`](crate::platform::uds).
//!
//! One acceptor thread, and per connection a reader thread and a writer thread draining a
//! bounded queue (1.6). The read methods are answered on the reader thread
//! ([`crate::read_api`]); account-scoped requests become [`Command`]s for that account's
//! `main` thread, found through [`Accounts`]. Each connection's requests are answered in
//! order. The [`Hub`] is every account's `DeltaStream` sink: it fans each flushed line out
//! to every subscriber whose filter matches, without blocking; a subscriber whose queue
//! passes the bound is sent `disconnect` and closed (1.4). The account secrets delta never
//! reaches the socket: it stays on the parent's stdout, where the secrets came in
//! (section 2), and so do `account.add` and `account.remove`.

use std::collections::{HashSet, VecDeque};
use std::io::{BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ms_core::constants::{
    thread, SOCKET_BACKLOG_BYTES, SOCKET_MAX_CONNECTIONS, SOCKET_QUEUE_BYTES,
};
use ms_core::Account;
use ms_ipc::{
    account_state, code, feature, method, policy, AccountInfo, AccountListResult,
    AccountStateParams, AccountStatus, BodiesFetchParams, CalendarSyncParams, DisconnectParams,
    EngineStatusResult, ErrorObject, FrameError, HelloParams, HelloResult, LogsTailParams,
    LogsTailResult, MailNewParams, Notification, RelayDevice, Request as RpcRequest, Response,
    SubscribeParams, SyncWakeParams, TaskCancelParams, TaskQueueParams, LOGS_TAIL_DEFAULT_BYTES,
    LOGS_TAIL_MAX_BYTES, MAX_LINE_BYTES, PROTOCOL_VERSION,
};
use ms_store::{DeltaSink, DeltaStream, MAIL_NEW, SECRETS_UPDATED};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::dispatch::{Command, Reply, Request};
use crate::platform;
use crate::read_api::{is_read_method, ReadApi};

/// How long a blocked write waits before the writer checks whether its connection is
/// being dropped.
const WRITE_POLL: Duration = Duration::from_millis(250);
/// A dropped connection's writer keeps trying to deliver the `disconnect` notification
/// this long, then closes the socket regardless.
const DROP_GRACE: Duration = Duration::from_secs(2);

/// The per-connection queue bound. Debug builds read `MAILSYNC_SOCKET_QUEUE_BYTES` so the
/// tests can reach it quickly.
pub fn queue_limit() -> usize {
    if cfg!(debug_assertions) {
        if let Some(n) = std::env::var("MAILSYNC_SOCKET_QUEUE_BYTES")
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
) -> std::io::Result<PathBuf> {
    write_discovery_file(config_dir, ENGINE_DISCOVERY_NAME, socket, |body| {
        body.insert("accounts".into(), json!(accounts));
    })
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
    ms_ipc::encode(&n).into()
}

// -- Per-connection queue -------------------------------------------------------------------

enum PushError {
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
struct Outbox {
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
    fn push(&self, line: Arc<[u8]>) -> Result<(), PushError> {
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

/// Sends `disconnect` and closes the connection: its queue passed the bound.
fn drop_slow(conn: u64, outbox: &Outbox) {
    let reason = format!("its queue passed {} bytes", outbox.limit);
    warn!(
        "{}",
        ms_core::logmsg::socket_connection_dropped(conn, &reason)
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
}

/// The socket `DeltaSink` of every account, as `delta` notifications, plus the
/// `account.state` notifications, to every matching subscriber.
pub struct Hub {
    limit: usize,
    state: Mutex<HubState>,
}

struct HubSink {
    hub: Arc<Hub>,
    account_id: String,
    parent: Box<dyn DeltaSink>,
}

impl DeltaSink for HubSink {
    fn write_item(&mut self, model_class: &str, line: &str) {
        if model_class == SECRETS_UPDATED {
            self.parent.write_item(model_class, line);
        } else if model_class == MAIL_NEW {
            self.hub.mail_new(&self.account_id, line);
        } else {
            let note = framed(ms_ipc::delta_notification_line(&self.account_id, line).into_bytes());
            self.hub.publish(&self.account_id, Some(model_class), note);
        }
    }
}

impl Hub {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Hub {
            limit,
            state: Mutex::new(HubState {
                subscribers: Vec::new(),
                backlog: Some(Backlog::default()),
            }),
        })
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
        })
    }

    /// Sends `account.state` to the subscribers of `params.account_id`.
    pub fn account_state(&self, params: &AccountStateParams) {
        let n = Notification::new(
            method::ACCOUNT_STATE,
            serde_json::to_value(params).unwrap_or(Value::Null),
        );
        self.publish(&params.account_id, None, ms_ipc::encode(&n).into());
    }

    /// Sends `mail.new` for each message in a [`MAIL_NEW`] item to the subscribers of
    /// `account_id` (docs/desktop-client-plan.md 3.1): no class filter holds it back, and
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
            self.publish_to(account_id, None, ms_ipc::encode(&n).into(), false);
        }
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
        let st = &mut *guard;
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

    /// Adds or replaces `conn`'s subscription. The response (none for a notification) is
    /// queued under the hub lock, so every delta the connection gets comes after it. The
    /// first subscriber that wants every class then gets the backlog of its accounts.
    fn subscribe(&self, sub: Subscriber, response: Option<Arc<[u8]>>) {
        let mut guard = self.lock();
        let st = &mut *guard;
        st.subscribers.retain(|s| s.conn != sub.conn);
        if let Some(response) = response {
            if let Err(e) = sub.outbox.push(response) {
                if matches!(e, PushError::Full) {
                    drop_slow(sub.conn, &sub.outbox);
                }
                return;
            }
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
        self.lock().subscribers.retain(|s| s.conn != conn);
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
            account: Some(policy::public_account_json(self.account.as_json())),
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

/// Starts the acceptor thread on a bound listener.
pub fn serve(listener: UnixListener, server: Arc<Server>) {
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
    let _ = stream.write_all(&ms_ipc::encode(&Response::err(None, e)));
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
        session: Session::new(server),
        outbox: outbox.clone(),
        hello: false,
        relay: None,
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
    reads: ReadApi,
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
            method::CALENDAR_SYNC => req
                .params_as::<CalendarSyncParams>()
                .and_then(|p| self.dispatch(&p.account_id, Request::SyncCalendar)),
            method::ENGINE_LOGS_TAIL => req
                .params_as::<LogsTailParams>()
                .and_then(|p| logs_tail(&self.server.config_dir, &p)),
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
            })
            .collect();
        to_value(serde_json::to_value(HelloResult {
            protocol: PROTOCOL_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
            accounts,
            features: features(),
        }))
    }

    fn status(&self) -> Value {
        to_value(serde_json::to_value(EngineStatusResult {
            protocol: PROTOCOL_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
            uptime_secs: self.server.started.elapsed().as_secs(),
            accounts: self.server.accounts.statuses(),
            features: features(),
        }))
        .unwrap_or(Value::Null)
    }

    /// Hands `request` to the account's `main` thread and waits for its result.
    fn dispatch(&self, account_id: &str, request: Request) -> Reply {
        let commands = self.server.accounts.commands(account_id)?;
        let rx = send(&commands, request)?;
        rx.recv()
            .unwrap_or_else(|_| Err(stopped_before_answering()))
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

/// `hello.features` and `engine.status.features`.
fn features() -> Vec<String> {
    [
        feature::COUNTS,
        feature::TASKS_LIST,
        feature::METADATA_LIST,
        feature::MESSAGE_HIDDEN,
        feature::MAIL_NEW,
        feature::RELAYED,
    ]
    .map(str::to_owned)
    .to_vec()
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

/// `engine.logs.tail`: the end of `mailsync-<accountId>.log` or `engine.log`. The account
/// need not be held, so a removed account's log can still be read.
fn logs_tail(config_dir: &Path, p: &LogsTailParams) -> Reply {
    use std::io::{Read, Seek, SeekFrom};
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
            ms_core::log::log_path(config_dir, id)
        }
        None => config_dir.join(ms_core::constants::ENGINE_LOG_FILE),
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

/// The methods only `serve` mode's parent channel takes.
pub fn is_parent_method(name: &str) -> bool {
    matches!(
        name,
        method::ACCOUNT_ADD
            | method::ACCOUNT_REMOVE
            | method::ACCOUNT_TEST
            | method::ACCOUNT_RESET
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
            match ms_ipc::read_frame(&mut reader, &mut line, MAX_LINE_BYTES) {
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
            let response = match ms_ipc::parse_request(&text) {
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
        self.outbox.close();
    }

    /// Queues a response. False once the connection is closed or dropped.
    fn respond(&self, response: Response) -> bool {
        match self.outbox.push(ms_ipc::encode(&response).into()) {
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
    fn handle(&mut self, req: RpcRequest) -> Option<Response> {
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
            name if is_parent_method(name) && self.session.server.accounts.has_parent_channel() => {
                Err(ErrorObject::new(
                    code::WRONG_CHANNEL,
                    format!(
                        "{name} is only accepted on the engine's stdin, from the process that \
                         started it: account credentials never go over the socket"
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
        Some(match result {
            Ok(v) => Response::ok(Some(id), v),
            Err(e) => Response::err(Some(id), e),
        })
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

    /// Whether this is a relayed connection that may not make `req` (ms-ipc's `policy`).
    fn relay_refuses(&self, req: &RpcRequest) -> bool {
        let Some(relay) = &self.relay else {
            return false;
        };
        if !policy::relayed_method_allowed(&req.method, &relay.scopes) {
            return true;
        }
        req.method == method::TASK_QUEUE
            && req
                .params_as::<TaskQueueParams>()
                .is_ok_and(|p| !policy::relayed_task_allowed(&p.task, &relay.scopes))
    }

    fn subscribe(&self, req: &RpcRequest) -> Result<(), ErrorObject> {
        let p = req.params_as::<SubscribeParams>()?;
        if let Some(list) = &p.accounts {
            for a in list {
                self.session.server.accounts.check(a)?;
            }
        }
        let response = req
            .id
            .clone()
            .map(|id| ms_ipc::encode(&Response::ok(Some(id), json!({}))).into());
        self.session.server.hub.subscribe(
            Subscriber {
                conn: self.id,
                outbox: self.outbox.clone(),
                accounts: p.accounts.map(|a| a.into_iter().collect()),
                classes: p.classes.map(|c| c.into_iter().collect()),
                relayed: self.relay.is_some(),
            },
            response,
        );
        Ok(())
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
        hub.subscribe(sub, None);

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
        hub.subscribe(sub, None);
        assert!(queued(&outbox).is_empty());
    }

    #[test]
    fn a_filtered_subscriber_leaves_the_backlog() {
        let hub = Hub::new(1 << 20);
        let mut sink = hub.sink("a1", Box::new(Captured::default()));
        sink.write_item("ProcessState", r#"{"modelClass":"ProcessState"}"#);
        sink.write_item("Task", r#"{"modelClass":"Task"}"#);

        let (filtered, f_out) = subscriber(1, Some(&["Task"]));
        hub.subscribe(filtered, Some(framed(b"{}".to_vec())));
        assert_eq!(queued(&f_out), vec!["{}\n".to_owned()]);

        let (all, a_out) = subscriber(2, None);
        hub.subscribe(all, None);
        let lines = queued(&a_out);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("ProcessState"));

        // Taken: a later subscriber gets live deltas only.
        let (late, l_out) = subscriber(3, None);
        hub.subscribe(late, None);
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
        hub.subscribe(watch, None);
        assert!(queued(&w_out).is_empty());

        let (all, a_out) = subscriber(2, None);
        hub.subscribe(all, None);
        let lines = queued(&a_out).concat();
        assert!(lines.contains("\"accountId\":\"a1\""), "{lines}");
        assert!(lines.contains("\"accountId\":\"b1\""), "{lines}");
    }

    /// `mail.new` routing (docs/desktop-client-plan.md 3.1): by account only, one per
    /// message, and never held for a later subscriber.
    #[test]
    fn mail_new_goes_live_to_the_account_subscribers() {
        let hub = Hub::new(1 << 20);
        let mut a = hub.sink("a1", Box::new(Captured::default()));
        let item = r#"{"modelClass":"MailNew","modelJSONs":[{"id":"m1","threadId":"t1"},{"id":"m2","threadId":"t1"}],"type":"persist"}"#;
        a.write_item(MAIL_NEW, item);
        let (early, e_out) = subscriber(1, None);
        hub.subscribe(early, None);
        assert!(queued(&e_out).is_empty(), "no backlog for mail.new");

        let (tasks_only, t_out) = subscriber(2, Some(&["Task"]));
        hub.subscribe(tasks_only, None);
        let (other, o_out) = filtered_subscriber(3, Some(&["b1"]), None);
        hub.subscribe(other, None);
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
        let mut sink = ms_store::WriteSink(Vec::new());
        sink.write_item(MAIL_NEW, r#"{"modelClass":"MailNew"}"#);
        sink.write_item("Task", r#"{"modelClass":"Task"}"#);
        assert_eq!(
            String::from_utf8(sink.0).unwrap(),
            "{\"modelClass\":\"Task\"}\n"
        );
    }
}
