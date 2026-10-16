// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `--socket`: the engine protocol on a Unix domain socket (docs/engine-protocol-plan.md
//! sections 1 and 2). Unix-only, like the transport; its OS calls are in
//! [`platform::uds`](crate::platform::uds).
//!
//! One acceptor thread, and per connection a reader thread and a writer thread draining a
//! bounded queue (1.6). The read methods are answered on the reader thread
//! ([`crate::read_api`]); requests other than those, `hello` and `subscribe` become
//! [`Command`]s for the main thread. Each connection's requests are answered in order. The [`Hub`] is the `DeltaStream`'s sink: it
//! fans each flushed line out to every subscriber whose filter matches, without blocking;
//! a subscriber whose queue passes the bound is sent `disconnect` and closed (1.4). The
//! account secrets delta never reaches the socket: it stays on the parent's stdout, where
//! the secrets came in (section 2).

use std::collections::{HashSet, VecDeque};
use std::io::{BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ms_core::constants::{thread, SOCKET_BACKLOG_BYTES, SOCKET_QUEUE_BYTES};
use ms_core::Account;
use ms_ipc::{
    code, method, AccountInfo, AccountStatus, BodiesFetchParams, CalendarSyncParams,
    DisconnectParams, EngineStatusResult, ErrorObject, FrameError, HelloParams, HelloResult,
    Notification, Request as RpcRequest, Response, SubscribeParams, SyncWakeParams,
    TaskCancelParams, TaskQueueParams, MAX_LINE_BYTES, PROTOCOL_VERSION,
};
use ms_store::{DeltaSink, DeltaStream, SECRETS_UPDATED};
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

/// Writes `$CONFIG_DIR_PATH/run/<accountId>.json` (1.5) and returns its path.
pub fn write_discovery(
    config_dir: &Path,
    account_id: &str,
    socket: &Path,
) -> std::io::Result<PathBuf> {
    let dir = config_dir.join("run");
    platform::uds::create_private_dir(&dir)?;
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
    let path = dir.join(format!("{name}.json"));
    let socket = std::path::absolute(socket).unwrap_or_else(|_| socket.to_owned());
    let body = json!({
        "pid": std::process::id(),
        "socket": socket.to_string_lossy(),
        "protocol": PROTOCOL_VERSION,
        "accountId": account_id,
    });
    platform::uds::write_private_file(&path, body.to_string().as_bytes())?;
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
    /// False when `accounts` names none of this engine's accounts.
    accounts: bool,
    classes: Option<HashSet<String>>,
}

impl Subscriber {
    fn wants(&self, model_class: &str) -> bool {
        self.accounts
            && self
                .classes
                .as_ref()
                .is_none_or(|c| c.contains(model_class))
    }
}

#[derive(Default)]
struct Backlog {
    lines: Vec<(String, Arc<[u8]>)>,
    bytes: usize,
}

struct HubState {
    subscribers: Vec<Subscriber>,
    /// Deltas flushed before the first unfiltered `subscribe`, for it. `None` from then on,
    /// or once it passed [`SOCKET_BACKLOG_BYTES`].
    backlog: Option<Backlog>,
}

/// The socket `DeltaSink`: one account's deltas, as `delta` notifications, to every
/// matching subscriber.
pub struct Hub {
    account_id: String,
    limit: usize,
    state: Mutex<HubState>,
}

struct HubSink {
    hub: Arc<Hub>,
    parent: Box<dyn DeltaSink>,
}

impl DeltaSink for HubSink {
    fn write_item(&mut self, model_class: &str, line: &str) {
        if model_class == SECRETS_UPDATED {
            self.parent.write_item(model_class, line);
        } else {
            self.hub.publish(model_class, line);
        }
    }
}

impl Hub {
    pub fn new(account_id: &str, limit: usize) -> Arc<Self> {
        Arc::new(Hub {
            account_id: account_id.to_owned(),
            limit,
            state: Mutex::new(HubState {
                subscribers: Vec::new(),
                backlog: Some(Backlog::default()),
            }),
        })
    }

    /// The stream's sink: [`SECRETS_UPDATED`] goes to `parent` (stdout), the rest to the
    /// subscribers.
    pub fn sink(self: &Arc<Self>, parent: Box<dyn DeltaSink>) -> Box<dyn DeltaSink> {
        Box::new(HubSink {
            hub: self.clone(),
            parent,
        })
    }

    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn publish(&self, model_class: &str, line: &str) {
        let mut guard = self.lock();
        let st = &mut *guard;
        if st.subscribers.is_empty() && st.backlog.is_none() {
            return;
        }
        let note = framed(ms_ipc::delta_notification_line(&self.account_id, line).into_bytes());
        if let Some(b) = st.backlog.as_mut() {
            if b.bytes + note.len() > SOCKET_BACKLOG_BYTES.min(self.limit) {
                warn!("-X Dropping the deltas held for the first subscriber: over the bound");
                st.backlog = None;
            } else {
                b.bytes += note.len();
                b.lines.push((model_class.to_owned(), note.clone()));
            }
        }
        st.subscribers.retain(|s| {
            if !s.wants(model_class) {
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
    /// first subscriber that wants everything then gets the backlog; a filtered one (a CLI
    /// watching one class) gets live deltas only and leaves the backlog for the desktop.
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
        let takes_backlog = sub.accounts && sub.classes.is_none();
        if let Some(backlog) = st.backlog.take_if(|_| takes_backlog) {
            for (_, line) in backlog.lines {
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

// -- Server ---------------------------------------------------------------------------------

/// What every connection shares.
pub struct Server {
    pub hub: Arc<Hub>,
    pub account: Arc<Account>,
    pub deltas: Arc<DeltaStream>,
    pub started: Instant,
    pub commands: Sender<Command>,
    /// Where the read methods open their own read-only database connection.
    pub config_dir: PathBuf,
}

/// Starts the acceptor thread on a bound listener.
pub fn serve(listener: UnixListener, server: Server) {
    let server = Arc::new(server);
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
            Ok(true) => start_connection(next_id, stream, server.clone()),
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

fn start_connection(id: u64, stream: UnixStream, server: Arc<Server>) {
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
    let reads = ReadApi::new(server.config_dir.clone(), server.account.id().to_owned());
    let conn = Connection {
        id,
        server,
        outbox: outbox.clone(),
        hello: false,
        reads,
    };
    let reader = std::thread::Builder::new()
        .name(thread::SOCKET_READER.into())
        .spawn(move || conn.read_loop(stream));
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

struct Connection {
    id: u64,
    server: Arc<Server>,
    outbox: Arc<Outbox>,
    hello: bool,
    reads: ReadApi,
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
        self.server.hub.unsubscribe(self.id);
        self.outbox.close();
    }

    /// Queues a response. False once the connection is closed or dropped.
    fn respond(&self, response: Response) -> bool {
        match self.outbox.push(ms_ipc::encode(&response).into()) {
            Ok(()) => true,
            Err(PushError::Closed) => false,
            Err(PushError::Full) => {
                drop_slow(self.id, &self.outbox);
                self.server.hub.unsubscribe(self.id);
                false
            }
        }
    }

    /// The response to `req`, or `None` when there is none to send (a client notification,
    /// which runs but is not answered, or `subscribe`, which queues its own).
    fn handle(&mut self, req: RpcRequest) -> Option<Response> {
        let result = match req.method.as_str() {
            method::HELLO => self.hello(&req),
            _ if !self.hello => Err(ErrorObject::new(
                code::HELLO_REQUIRED,
                "hello must be the first request",
            )),
            method::SUBSCRIBE => match self.subscribe(&req) {
                Ok(()) => return None,
                Err(e) => Err(e),
            },
            method::ENGINE_STATUS => Ok(self.status()),
            name if is_read_method(name) => self.reads.call(&req),
            method::TASK_QUEUE => req.params_as::<TaskQueueParams>().and_then(|p| {
                self.check_account(&p.account_id)?;
                if !p.task.is_object() {
                    return Err(ErrorObject::new(
                        code::INVALID_PARAMS,
                        "the task must be an object",
                    ));
                }
                self.dispatch(Request::QueueTask(p.task))
            }),
            method::TASK_CANCEL => req.params_as::<TaskCancelParams>().and_then(|p| {
                self.check_account(&p.account_id)?;
                self.dispatch(Request::CancelTask(p.task_id))
            }),
            method::SYNC_WAKE => req.params_as::<SyncWakeParams>().and_then(|p| {
                if let Some(a) = &p.account_id {
                    self.check_account(a)?;
                }
                self.dispatch(Request::WakeWorkers)
            }),
            method::BODIES_FETCH => req.params_as::<BodiesFetchParams>().and_then(|p| {
                self.check_account(&p.account_id)?;
                self.dispatch(Request::NeedBodies(p.ids))
            }),
            method::CALENDAR_SYNC => req.params_as::<CalendarSyncParams>().and_then(|p| {
                self.check_account(&p.account_id)?;
                self.dispatch(Request::SyncCalendar)
            }),
            other => Err(ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("unknown method {other}"),
            )),
        };
        let id = req.id?;
        Some(match result {
            Ok(v) => Response::ok(Some(id), v),
            Err(e) => Response::err(Some(id), e),
        })
    }

    fn hello(&mut self, req: &RpcRequest) -> Reply {
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
        self.hello = true;
        info!("Socket connection {}: hello from {:?}", self.id, p.client);
        let account = &self.server.account;
        to_value(serde_json::to_value(HelloResult {
            protocol: PROTOCOL_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
            accounts: vec![AccountInfo {
                account_id: account.id().to_owned(),
                email: account.email_address().to_owned(),
                state: "running".to_owned(),
            }],
        }))
    }

    fn subscribe(&self, req: &RpcRequest) -> Result<(), ErrorObject> {
        let p = req.params_as::<SubscribeParams>()?;
        let accounts = match &p.accounts {
            None => true,
            Some(list) => {
                for a in list {
                    self.check_account(a)?;
                }
                !list.is_empty()
            }
        };
        let response = req
            .id
            .clone()
            .map(|id| ms_ipc::encode(&Response::ok(Some(id), json!({}))).into());
        self.server.hub.subscribe(
            Subscriber {
                conn: self.id,
                outbox: self.outbox.clone(),
                accounts,
                classes: p.classes.map(|c| c.into_iter().collect()),
            },
            response,
        );
        Ok(())
    }

    fn status(&self) -> Value {
        let account = &self.server.account;
        to_value(serde_json::to_value(EngineStatusResult {
            protocol: PROTOCOL_VERSION,
            engine: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
            uptime_secs: self.server.started.elapsed().as_secs(),
            accounts: vec![AccountStatus {
                account_id: account.id().to_owned(),
                email: account.email_address().to_owned(),
                state: "running".to_owned(),
                connection_error: self.server.deltas.connection_error(),
            }],
        }))
        .unwrap_or(Value::Null)
    }

    fn check_account(&self, account_id: &str) -> Result<(), ErrorObject> {
        if account_id == self.server.account.id() {
            return Ok(());
        }
        Err(ErrorObject::new(
            code::UNKNOWN_ACCOUNT,
            format!("unknown account {account_id}"),
        ))
    }

    /// Hands `request` to the main thread and waits for its result.
    fn dispatch(&self, request: Request) -> Reply {
        let internal = || ErrorObject::new(code::INTERNAL_ERROR, "the engine is shutting down");
        let (tx, rx) = mpsc::channel();
        self.server
            .commands
            .send(Command {
                request,
                reply: Some(tx),
            })
            .map_err(|_| internal())?;
        rx.recv().unwrap_or_else(|_| Err(internal()))
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
        let outbox = Arc::new(Outbox::new(1 << 20));
        let sub = Subscriber {
            conn,
            outbox: outbox.clone(),
            accounts: true,
            classes: classes.map(|c| c.iter().map(|s| (*s).to_owned()).collect()),
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
        let hub = Hub::new("a1", 1 << 20);
        let parent = Captured::default();
        let mut sink = hub.sink(Box::new(parent.clone()));
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
        let hub = Hub::new("a1", 1 << 20);
        let mut sink = hub.sink(Box::new(Captured::default()));
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
        let hub = Hub::new("a1", 1 << 20);
        let mut sink = hub.sink(Box::new(Captured::default()));
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
}
