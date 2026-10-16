// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine's local socket protocol, v1 (docs/engine-protocol-plan.md section 1).
//!
//! JSON-RPC 2.0 over newline-delimited JSON on a Unix domain socket. Clients send requests
//! with an `id` and get one response each; the engine's deltas arrive as `delta`
//! notifications once the client has called `subscribe`. `hello` must be the first request.
//!
//! Depends on serde and serde_json only, so the CLI and third-party tools can use it
//! without pulling in the engine. The engine serves the protocol; this crate also has a
//! blocking [`Client`], used by the CLI and the engine's tests.

mod client;
mod envelope;
mod frame;
mod methods;
mod read;

pub use client::{Client, ClientError, Notifications};
pub use envelope::{
    delta_notification_line, parse_message, parse_request, ErrorObject, Id, Message, Notification,
    Request, Response,
};
pub use frame::{encode, read_frame, write_frame, FrameError};
pub use methods::*;
pub use read::*;

/// The protocol version a `hello` names. Additive changes (a new method, a new optional
/// field) keep it; a breaking change bumps it.
pub const PROTOCOL_VERSION: u32 = 1;

/// The `jsonrpc` member of every message.
pub const JSONRPC_VERSION: &str = "2.0";

/// The longest line either side accepts, without its newline. A longer line is a framing
/// error and ends the connection.
pub const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// The rows a read method returns when the request names no `limit`.
pub const READ_DEFAULT_LIMIT: usize = 50;

/// The most rows a read method returns; a larger `limit` is clamped to it.
pub const READ_MAX_LIMIT: usize = 500;

/// Method and notification names.
pub mod method {
    pub const HELLO: &str = "hello";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const TASK_QUEUE: &str = "task.queue";
    pub const TASK_CANCEL: &str = "task.cancel";
    pub const SYNC_WAKE: &str = "sync.wake";
    pub const BODIES_FETCH: &str = "bodies.fetch";
    pub const CALENDAR_SYNC: &str = "calendar.sync";
    pub const ENGINE_STATUS: &str = "engine.status";
    pub const FOLDERS_LIST: &str = "folders.list";
    pub const LABELS_LIST: &str = "labels.list";
    pub const THREADS_LIST: &str = "threads.list";
    pub const THREADS_GET: &str = "threads.get";
    pub const MESSAGES_GET: &str = "messages.get";
    pub const SEARCH: &str = "search";
    pub const CONTACTS_SEARCH: &str = "contacts.search";
    pub const TASKS_GET: &str = "tasks.get";
    /// Every `ThreadCounts` row of the accounts ([`feature::COUNTS`](crate::feature::COUNTS)).
    pub const COUNTS_GET: &str = "counts.get";
    /// Stored tasks by account, status, class or draft ([`feature::TASKS_LIST`](crate::feature::TASKS_LIST)).
    pub const TASKS_LIST: &str = "tasks.list";
    /// Messages or threads carrying a plugin's metadata
    /// ([`feature::METADATA_LIST`](crate::feature::METADATA_LIST)).
    pub const METADATA_LIST: &str = "metadata.list";
    /// Every account the engine holds, with its state.
    pub const ACCOUNT_LIST: &str = "account.list";
    /// `serve` mode, the parent's stdin only: start syncing an account.
    pub const ACCOUNT_ADD: &str = "account.add";
    /// `serve` mode, the parent's stdin only: stop an account and forget it. Its database
    /// rows stay.
    pub const ACCOUNT_REMOVE: &str = "account.remove";
    /// `serve` mode, the parent's stdin only: stop every account, remove the socket files
    /// and exit 0.
    pub const ENGINE_SHUTDOWN: &str = "engine.shutdown";
    /// `serve` mode, the parent's stdin only, debug builds only: stop one account as a
    /// fatal error would (`test-crash` on stdin in `sync` mode).
    pub const TEST_CRASH: &str = "test.crash";
    /// As [`TEST_CRASH`], with a panic on the account's `main` thread.
    pub const TEST_PANIC: &str = "test.panic";
    /// Notification: an account changed state, `params` =
    /// [`AccountStateParams`](crate::AccountStateParams).
    pub const ACCOUNT_STATE: &str = "account.state";
    /// Notification: one flushed delta, `params` = [`DeltaParams`](crate::DeltaParams).
    pub const DELTA: &str = "delta";
    /// Notification: the engine is about to close the connection, `params` =
    /// [`DisconnectParams`](crate::DisconnectParams).
    pub const DISCONNECT: &str = "disconnect";
    /// Notification: new mail, `params` = [`MailNewParams`](crate::MailNewParams). Filtered by
    /// account only, never held for a later subscriber ([`feature::MAIL_NEW`](crate::feature::MAIL_NEW)).
    pub const MAIL_NEW: &str = "mail.new";
}

/// The names in `hello.features` and `engine.status.features`
/// (docs/desktop-client-plan.md 3.1). Each one is additive: a client checks for it before it
/// relies on the method, field or notification it names.
pub mod feature {
    /// `counts.get`, and `ThreadCounts` deltas whenever a category's counts change.
    pub const COUNTS: &str = "counts";
    /// `tasks.list`.
    pub const TASKS_LIST: &str = "tasks-list";
    /// `metadata.list`.
    pub const METADATA_LIST: &str = "metadata-list";
    /// `hidden` on the Message JSON the read methods return (`Message.isHidden()`).
    pub const MESSAGE_HIDDEN: &str = "message-hidden";
    /// `mail.new` notifications for subscribed connections.
    pub const MAIL_NEW: &str = "mail-new";
}

/// Error codes (plan 1.3). The standard JSON-RPC codes, then the engine's from `-32000`
/// down.
pub mod code {
    /// The line is not JSON. The response's `id` is null.
    pub const PARSE_ERROR: i64 = -32700;
    /// JSON, but not a request object.
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// `hello` named a protocol version the engine does not speak; `data.supported` lists
    /// the ones it does.
    pub const UNSUPPORTED_PROTOCOL: i64 = -32000;
    /// The `accountId` is not one the engine holds.
    pub const UNKNOWN_ACCOUNT: i64 = -32001;
    /// The task's local phase failed; `data` is the `SyncError` JSON.
    pub const TASK_REJECTED: i64 = -32002;
    /// A request other than `hello` came before a successful `hello`.
    pub const HELLO_REQUIRED: i64 = -32003;
    /// A read named a folder, label, thread or message the account does not have.
    pub const NOT_FOUND: i64 = -32004;
    /// The account is known but not running (it stopped on an error, or is being
    /// removed); `data` is `{state, error}`.
    pub const ACCOUNT_STOPPED: i64 = -32005;
    /// `account.add` named an account that is already starting or running.
    pub const ACCOUNT_EXISTS: i64 = -32006;
    /// The method is only accepted on the parent's stdin (`account.add`, `account.remove`,
    /// `engine.shutdown`), or only on the socket (`subscribe`).
    pub const WRONG_CHANNEL: i64 = -32007;
}

/// The `state` of an account in `hello`, `engine.status`, `account.list` and `account.state`.
pub mod account_state {
    /// Added; its `main` thread has not opened the database yet.
    pub const STARTING: &str = "starting";
    pub const RUNNING: &str = "running";
    /// Stopped by a failure (`error` says which). The engine does not restart it; the
    /// client may `account.add` it again.
    pub const STOPPED: &str = "stopped";
    /// `account.remove` finished; only sent as an `account.state` notification.
    pub const REMOVED: &str = "removed";
}
