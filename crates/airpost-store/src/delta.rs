// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The stdout delta stream. Derived from Mailspring-Sync `MailSync/DeltaStream.*`,
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! One instance per process, shared by every thread's [`Store`](crate::Store). Items are
//! buffered by model class and coalesced: a new item merges into the last buffered item of
//! its class when the type matches, and JSONs with the same id merge their keys over the
//! earlier entry. A `DeltaStreamFlush` thread writes the buffer, classes in alphabetical
//! order, when the earliest requested deadline passes; an emit asking for an earlier
//! deadline than the one scheduled flushes at once. Nothing is flushed on exit.
//!
//! Flushed lines go to a [`DeltaSink`]: stdout for the stdio transport, the socket hub
//! with `--socket`.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use airpost_core::constants::{thread, DELTA_TYPE_PERSIST};
use airpost_core::Account;
use serde_json::{json, Value};

use crate::Model;

/// One stdout line: `{"modelClass":…,"modelJSONs":[…],"type":"persist"|"unpersist"}`.
#[derive(Clone, Debug)]
pub struct DeltaItem {
    /// `persist`, `unpersist` or `metadata-expiration` (`airpost_core::constants::DELTA_TYPE_*`).
    pub kind: String,
    pub model_class: String,
    model_jsons: Vec<Value>,
    /// id → index in `model_jsons`.
    id_indexes: HashMap<String, usize>,
}

impl PartialEq for DeltaItem {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.model_class == other.model_class
            && self.model_jsons == other.model_jsons
    }
}

impl DeltaItem {
    /// An item of `model_class`; JSONs repeating an id merge as in [`DeltaItem::concatenate`].
    pub fn new(kind: &str, model_class: &str, model_jsons: Vec<Value>) -> Self {
        let mut item = DeltaItem {
            kind: kind.to_owned(),
            model_class: model_class.to_owned(),
            model_jsons: Vec::new(),
            id_indexes: HashMap::new(),
        };
        for j in model_jsons {
            item.upsert_model_json(j);
        }
        item
    }

    /// One model's `delta_json` (`DeltaStreamItem(type, model)`).
    pub fn for_model<M: Model>(kind: &str, model: &M) -> Self {
        DeltaItem::new(kind, M::TABLE, vec![model.delta_json()])
    }

    /// `DeltaStreamItem(type, models)`: the class of the first model, or `"none"` with an
    /// empty list when there are none.
    pub fn for_models<M: Model>(kind: &str, models: &[M]) -> Self {
        if models.is_empty() {
            return DeltaItem::new(kind, "none", Vec::new());
        }
        DeltaItem::new(
            kind,
            M::TABLE,
            models.iter().map(Model::delta_json).collect(),
        )
    }

    pub fn model_jsons(&self) -> &[Value] {
        &self.model_jsons
    }

    /// Merges `other` into this item when the class and type match. Returns false (and
    /// changes nothing) when they cannot be merged.
    pub fn concatenate(&mut self, other: &DeltaItem) -> bool {
        if other.kind != self.kind || other.model_class != self.model_class {
            return false;
        }
        for j in &other.model_jsons {
            self.upsert_model_json(j.clone());
        }
        true
    }

    /// Two back-to-back saves of one object leave one entry. Keys of the later JSON replace
    /// the earlier one's, and keys only the earlier one had survive: `body` and
    /// `rulesReady` are sent conditionally and must not be lost to a later save.
    fn upsert_model_json(&mut self, item: Value) {
        let Some(id) = item.get("id").and_then(Value::as_str).map(str::to_owned) else {
            self.model_jsons.push(item);
            return;
        };
        match self.id_indexes.get(&id) {
            Some(&idx) => match (&mut self.model_jsons[idx], item) {
                (Value::Object(existing), Value::Object(incoming)) => {
                    for (k, v) in incoming {
                        existing.insert(k, v);
                    }
                }
                (slot, incoming) => *slot = incoming,
            },
            None => {
                self.id_indexes.insert(id, self.model_jsons.len());
                self.model_jsons.push(item);
            }
        }
    }

    /// The line written to stdout, keys sorted at every level, no spaces, without the
    /// trailing newline. Sorting does not rely on serde_json's map type, which a
    /// `preserve_order` feature anywhere in the build would change.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        write_sorted(
            &json!({
                "type": self.kind,
                "modelJSONs": self.model_jsons,
                "modelClass": self.model_class,
            }),
            &mut out,
        );
        out
    }
}

fn write_sorted(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_sorted(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, e) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_sorted(e, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&serde_json::to_string(scalar).unwrap_or_default()),
    }
}

/// The model class of the delta carrying the account JSON, passwords and tokens included.
/// It only goes to the parent process.
pub const SECRETS_UPDATED: &str = "ProcessAccountSecretsUpdated";

/// The model class of the items behind the socket's `mail.new` notifications, one JSON
/// `{id, threadId}` per new message (the id is the message's, so a repeat within one flush
/// merges). Buffered with the deltas so it goes out only once the message is committed, but
/// never written as a delta: the socket hub turns it into notifications and every other sink
/// drops it.
pub const MAIL_NEW: &str = "MailNew";

/// Classes whose items are not deltas and never reach stdout.
pub fn is_socket_only(model_class: &str) -> bool {
    model_class == MAIL_NEW
}

/// Where flushed items go. Called on the flush thread with the buffer locked, once per item
/// in flush order, so an implementation must not block: a slow consumer is the sink's
/// problem (the socket hub queues per connection and drops subscribers that fall behind).
pub trait DeltaSink: Send {
    /// One item: its model class and its line ([`DeltaItem::dump`]), without a newline.
    fn write_item(&mut self, model_class: &str, line: &str);

    /// [`DeltaSink::write_item`] with the item itself, for a sink that reads its models.
    fn write_delta(&mut self, item: &DeltaItem, line: &str) {
        self.write_item(&item.model_class, line);
    }

    /// Called after the items of one flush.
    fn flushed(&mut self) {}
}

/// A sink writing each line and a newline to `W`, flushing after each, ignoring errors: a
/// closed stdout must not take the engine down; the client is gone.
pub struct WriteSink<W>(pub W);

impl<W: Write + Send> DeltaSink for WriteSink<W> {
    fn write_item(&mut self, model_class: &str, line: &str) {
        if is_socket_only(model_class) {
            return;
        }
        let _ = self.0.write_all(format!("{line}\n").as_bytes());
        let _ = self.0.flush();
    }
}

#[derive(Default)]
struct Buffer {
    /// Keyed by model class; flushed in key order.
    items: BTreeMap<String, Vec<DeltaItem>>,
    /// When the scheduled flush is due, if one is scheduled.
    deadline: Option<Instant>,
    connection_error: bool,
    /// [`DeltaStream::close`] was called: later items are dropped.
    closed: bool,
}

pub struct DeltaStream {
    out: Mutex<Box<dyn DeltaSink>>,
    buffer: Mutex<Buffer>,
    cv: Condvar,
}

impl DeltaStream {
    /// A stream writing lines to `out` ([`WriteSink`]), with its flush thread.
    pub fn new(out: Box<dyn Write + Send>) -> Arc<Self> {
        DeltaStream::with_sink(Box::new(WriteSink(out)))
    }

    /// A stream flushing into `sink`, with its flush thread. The thread ends when the last
    /// reference to the stream is dropped.
    pub fn with_sink(out: Box<dyn DeltaSink>) -> Arc<Self> {
        let stream = Arc::new(DeltaStream {
            out: Mutex::new(out),
            buffer: Mutex::new(Buffer::default()),
            cv: Condvar::new(),
        });
        let weak = Arc::downgrade(&stream);
        // Note: without the thread, deltas would wait for an explicit flush; spawning only
        // fails when the process is out of threads, which nothing else survives either.
        let _ = std::thread::Builder::new()
            .name(thread::DELTA_STREAM_FLUSH.to_owned())
            .spawn(move || flush_loop(weak));
        stream
    }

    /// The process stream on stdout.
    pub fn stdout() -> Arc<Self> {
        DeltaStream::new(Box::new(std::io::stdout()))
    }

    fn lock(&self) -> MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Buffers `items` for delivery within `max_delay_ms` (0 flushes as soon as the flush
    /// thread runs). A later emit with an earlier deadline than the scheduled one flushes
    /// immediately.
    pub fn emit(&self, items: Vec<DeltaItem>, max_delay_ms: u64) {
        let mut buf = self.lock();
        queue_locked(&mut buf, items, max_delay_ms);
        drop(buf);
        self.cv.notify_all();
    }

    /// Writes everything buffered now (`flushBuffer`).
    pub fn flush(&self) {
        let mut buf = self.lock();
        self.write_buffer(&mut buf);
    }

    /// Flushes and drops everything emitted from now on. `serve` closes an account's stream
    /// once its threads stopped, so a thread left running from that run cannot publish under
    /// the account's id after it was stopped, removed or added again.
    pub fn close(&self) {
        let mut buf = self.lock();
        self.write_buffer(&mut buf);
        buf.closed = true;
    }

    /// Writes while holding the buffer lock, as the C++ did, so lines from two flushes
    /// never interleave and a later emit cannot overtake an earlier one.
    fn write_buffer(&self, buf: &mut Buffer) {
        let items = std::mem::take(&mut buf.items);
        buf.deadline = None;
        if items.is_empty() {
            return;
        }
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        for list in items.values() {
            for item in list {
                out.write_delta(item, &item.dump());
            }
        }
        out.flushed();
    }

    /// Whether the last `ProcessState` sent said the account cannot reach its server.
    pub fn connection_error(&self) -> bool {
        self.lock().connection_error
    }

    /// [`SECRETS_UPDATED`] after an OAuth refresh token rotated.
    pub fn send_updated_secrets(&self, account: &Account) {
        let mut j = account.to_json();
        if let Some(obj) = j.as_object_mut() {
            obj.entry("__cls").or_insert_with(|| json!("Account"));
        }
        self.emit(
            vec![DeltaItem::new(DELTA_TYPE_PERSIST, SECRETS_UPDATED, vec![j])],
            0,
        );
    }

    /// `ProcessIdentityRefreshNeeded`.
    pub fn send_identity_refresh_needed(&self) {
        self.emit(
            vec![DeltaItem::new(
                DELTA_TYPE_PERSIST,
                "ProcessIdentityRefreshNeeded",
                Vec::new(),
            )],
            0,
        );
    }

    /// `ProcessState {accountId, id, connectionError: true}`, sent on every call.
    pub fn begin_connection_error(&self, account_id: &str) {
        let mut buf = self.lock();
        buf.connection_error = true;
        queue_locked(&mut buf, vec![process_state(account_id, true)], 0);
        drop(buf);
        self.cv.notify_all();
    }

    /// `ProcessState {…, connectionError: false}`, sent only after a begin.
    pub fn end_connection_error(&self, account_id: &str) {
        let mut buf = self.lock();
        if std::mem::replace(&mut buf.connection_error, false) {
            queue_locked(&mut buf, vec![process_state(account_id, false)], 0);
        }
        drop(buf);
        self.cv.notify_all();
    }
}

/// The flag and its delta change under one lock: with the C++'s separate steps, a begin and
/// an end on two workers could interleave so the queued state was `true` while the flag
/// was already `false`, and no later end ever cleared it.
fn process_state(account_id: &str, connection_error: bool) -> DeltaItem {
    let state =
        json!({ "accountId": account_id, "id": account_id, "connectionError": connection_error });
    DeltaItem::new(DELTA_TYPE_PERSIST, "ProcessState", vec![state])
}

fn queue_locked(buf: &mut Buffer, items: Vec<DeltaItem>, max_delay_ms: u64) {
    if buf.closed {
        return;
    }
    let desired = Instant::now() + Duration::from_millis(max_delay_ms);
    for item in items {
        let list = buf.items.entry(item.model_class.clone()).or_default();
        let merged = list.last_mut().is_some_and(|last| last.concatenate(&item));
        if !merged {
            list.push(item);
        }
    }
    match buf.deadline {
        None => buf.deadline = Some(desired),
        Some(scheduled) if desired < scheduled => buf.deadline = Some(Instant::now()),
        Some(_) => {}
    }
}

/// Waits for a scheduled deadline, re-reading it on every wake because an emit can pull it
/// in, and flushes when it passes. Exits once the stream is dropped.
fn flush_loop(stream: std::sync::Weak<DeltaStream>) {
    const IDLE_CHECK: Duration = Duration::from_secs(1);
    loop {
        let Some(s) = stream.upgrade() else {
            return;
        };
        let mut buf = s.lock();
        let now = Instant::now();
        match buf.deadline {
            Some(deadline) if deadline <= now => s.write_buffer(&mut buf),
            Some(deadline) => {
                let _ = s.cv.wait_timeout(buf, deadline - now);
            }
            None => {
                let _ = s.cv.wait_timeout(buf, IDLE_CHECK);
            }
        }
    }
}
