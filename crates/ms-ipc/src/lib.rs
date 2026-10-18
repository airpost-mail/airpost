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

mod ai;
mod client;
mod envelope;
mod files;
mod frame;
mod integrations;
mod mail;
mod methods;
pub mod policy;
mod read;
mod settings;

pub use ai::*;
pub use client::{Client, ClientError, Notifications};
pub use envelope::{
    delta_notification_line, parse_message, parse_request, ErrorObject, Id, Message, Notification,
    Request, Response,
};
pub use files::*;
pub use frame::{encode, read_frame, write_frame, FrameError};
pub use integrations::*;
pub use mail::*;
pub use methods::*;
pub use read::*;
pub use settings::*;

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
    /// A ModelQuery over the stored models ([`feature::QUERY`](crate::feature::QUERY),
    /// desktop-client-plan 3.2).
    pub const QUERY: &str = "query";
    /// A File by id, downloaded first when missing ([`feature::FILES`](crate::feature::FILES)),
    /// params [`FilesGetParams`](crate::FilesGetParams). Local clients only; answered out of
    /// order while it waits for the download.
    pub const FILES_GET: &str = "files.get";
    /// A chunk of a downloaded File, params [`FilesReadParams`](crate::FilesReadParams).
    pub const FILES_READ: &str = "files.read";
    /// Stores bytes as a new File for a draft, params [`FilesPutParams`](crate::FilesPutParams).
    pub const FILES_PUT: &str = "files.put";
    /// Moves threads to Snoozed with the metadata that brings them back
    /// ([`feature::SNOOZE`](crate::feature::SNOOZE)), params
    /// [`ThreadsSnoozeParams`](crate::ThreadsSnoozeParams).
    pub const THREADS_SNOOZE: &str = "threads.snooze";
    /// Ends a snooze now, params [`ThreadsUnsnoozeParams`](crate::ThreadsUnsnoozeParams).
    pub const THREADS_UNSNOOZE: &str = "threads.unsnooze";
    /// Unsubscribes from the list a message came from
    /// ([`feature::UNSUBSCRIBE`](crate::feature::UNSUBSCRIBE)), params
    /// [`MessagesUnsubscribeParams`](crate::MessagesUnsubscribeParams). Socket only; answered
    /// out of order while a one-click POST runs.
    pub const MESSAGES_UNSUBSCRIBE: &str = "messages.unsubscribe";
    /// Every account the engine holds, with its state.
    pub const ACCOUNT_LIST: &str = "account.list";
    /// `serve` mode, the parent's stdin only (the socket too on a `--detached` engine):
    /// register an account, store its secrets and start syncing it.
    pub const ACCOUNT_ADD: &str = "account.add";
    /// `serve` mode, the parent's stdin only (the socket too on a `--detached` engine): stop
    /// an account; with `forget`, also drop it from the registry and delete its secrets.
    /// Its database rows stay.
    pub const ACCOUNT_REMOVE: &str = "account.remove";
    /// `serve` mode, the parent's stdin only (the socket too on a `--detached` engine): stop
    /// every account, remove the socket files and exit 0.
    pub const ENGINE_SHUTDOWN: &str = "engine.shutdown";
    /// `serve` mode, the parent's stdin only: register accounts and store secrets a client
    /// held until now (docs/desktop-client-plan.md section 5), params
    /// [`CredentialsImportParams`](crate::CredentialsImportParams).
    pub const CREDENTIALS_IMPORT: &str = "credentials.import";
    /// Notification: an account's registered JSON changed, `params` =
    /// [`AccountChangedParams`](crate::AccountChangedParams). Filtered by account, never held
    /// for a later subscriber: a client calls `account.list` after `subscribe`.
    pub const ACCOUNT_CHANGED: &str = "account.changed";
    /// `serve` mode, the parent's stdin only: check an account's IMAP and SMTP settings,
    /// as `--mode test` does. The result is that mode's JSON object, with any rotated
    /// refresh token in its `account`.
    pub const ACCOUNT_TEST: &str = "account.test";
    /// `serve` mode, the parent's stdin only: stop and forget the account if the engine
    /// holds it, then delete its rows from the database (`--mode reset`). The client adds
    /// it again to resync.
    pub const ACCOUNT_RESET: &str = "account.reset";
    /// `serve` mode, the parent's stdin only: stop and forget every account, delete the
    /// database and create an empty one. The client adds the accounts again.
    pub const ENGINE_RESET_DATABASE: &str = "engine.resetDatabase";
    /// `serve` mode, the parent's stdin only: verbose protocol logging until a time, for
    /// every account; running accounts restart to pick it up.
    pub const ENGINE_SET_LOG_LEVEL: &str = "engine.setLogLevel";
    /// The end of an account's log, or of the engine's own log.
    pub const ENGINE_LOGS_TAIL: &str = "engine.logs.tail";
    /// The stored engine-owned settings ([`feature::SETTINGS`](crate::feature::SETTINGS)),
    /// params [`SettingsGetParams`](crate::SettingsGetParams), result
    /// [`SettingsValues`](crate::SettingsValues).
    pub const SETTINGS_GET: &str = "settings.get";
    /// Stores settings, params [`SettingsValues`](crate::SettingsValues), result `{}`.
    pub const SETTINGS_SET: &str = "settings.set";
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
    /// Notification: settings another connection (or the parent) changed, `params` =
    /// [`SettingsValues`](crate::SettingsValues) with the new values, null for a cleared key.
    /// Sent to every subscribed connection whatever its filter, never held for a later
    /// subscriber: a client calls `settings.get` after `subscribe`.
    pub const SETTINGS_CHANGED: &str = "settings.changed";
    /// The `integrations.*` methods ([`feature::INTEGRATIONS`](crate::feature::INTEGRATIONS)),
    /// params and results in docs/integrations-plan.md section 4. Each carries the
    /// provider's `config`, key included, until the engine owns the credentials. Local
    /// clients only, answered out of order on the engine's integrations workers (T21); a
    /// provider failure is [`code::INTEGRATION_FAILED`](crate::code::INTEGRATION_FAILED).
    pub const INTEGRATIONS_TRANSLATE: &str = "integrations.translate";
    pub const INTEGRATIONS_DETECT: &str = "integrations.detect";
    pub const INTEGRATIONS_LANGUAGES: &str = "integrations.languages";
    pub const INTEGRATIONS_GRAMMAR: &str = "integrations.grammar";
    pub const INTEGRATIONS_ENRICH: &str = "integrations.enrich";
    pub const INTEGRATIONS_TEST: &str = "integrations.test";
    /// Notification: the engine's `capabilities` changed ([`feature::CAPABILITIES`](crate::feature::CAPABILITIES)),
    /// `params` = the new capabilities object, whole. Sent to every subscribed connection
    /// whatever its filter, never held for a later subscriber: `hello` and `engine.status`
    /// carry the current value.
    pub const CAPABILITIES_CHANGED: &str = "capabilities.changed";
    /// The `ai.*` methods ([`feature::AI`](crate::feature::AI)), params and results in
    /// docs/ai-plan.md section 3 and [`crate::AiSettings`]. Until the engine keeps the AI
    /// settings and keys, local clients send the provider `config`, key included, and the
    /// `settings` with each call; a relayed connection may send neither
    /// ([`policy`](crate::policy)). Answered out of order on the engine's AI workers (T21);
    /// a failed model call is [`code::AI_FAILED`](crate::code::AI_FAILED).
    pub const AI_MODELS: &str = "ai.models";
    pub const AI_TEST: &str = "ai.test";
    pub const AI_SUMMARIZE: &str = "ai.summarize";
    pub const AI_SUGGEST_REPLIES: &str = "ai.suggestReplies";
    pub const AI_DRAFT_REPLY: &str = "ai.draftReply";
    /// Answered at once, in order: stops the calling connection's stream `streamId`.
    pub const AI_CANCEL: &str = "ai.cancel";
    /// Token counts per day, provider and feature since the engine started.
    pub const AI_USAGE: &str = "ai.usage";
    /// Inbox triage (docs/ai-plan.md section 4); not served yet.
    pub const AI_TRIAGE_RUN: &str = "ai.triage.run";
    pub const AI_TRIAGE_LIST: &str = "ai.triage.list";
    /// Notification: the next piece of a streamed answer, `params` =
    /// [`AiDeltaParams`](crate::AiDeltaParams). Sent only to the connection that started the
    /// stream, in order, without `subscribe`; the call's response comes after the last one
    /// and carries the whole text.
    pub const AI_DELTA: &str = "ai.delta";
    /// Notification: triage progress for the connection that called `ai.triage.run`.
    pub const AI_TRIAGE_PROGRESS: &str = "ai.triage.progress";
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
    /// `query`. Its `search.grammar: true` comes with [`SEARCH_GRAMMAR`].
    pub const QUERY: &str = "query";
    /// The search grammar (`from:`, `in:`, `is:`, `has:attachment`, `before:`/`after:`,
    /// `AND`/`OR`/`NOT`, parentheses) in `query.search {grammar: true, tz, now}` and in
    /// `search {grammar: true, tz, now}`, and `search {accounts}`.
    pub const SEARCH_GRAMMAR: &str = "search-grammar";
    /// `hidden` on the Message JSON the read methods return (`Message.isHidden()`).
    pub const MESSAGE_HIDDEN: &str = "message-hidden";
    /// `mail.new` notifications for subscribed connections.
    pub const MAIL_NEW: &str = "mail-new";
    /// `hello.relayed`: the engine applies [`policy`](crate::policy) to relayed
    /// connections. `tern-gateway` refuses to relay to an engine without it, which would
    /// ignore the field.
    pub const RELAYED: &str = "relayed";
    /// `settings.get`, `settings.set` and `settings.changed`.
    pub const SETTINGS: &str = "settings";
    /// `files.get`, `files.read`, `files.put`, and GetMessageRFC2822Task's file-id mode (no
    /// `filepath`: the export becomes the File `fileId`, set by the local phase).
    pub const FILES: &str = "files";
    /// The engine acts on due `send-later`, `thread-snooze` and `send-reminders` metadata
    /// itself (docs/desktop-client-plan.md 3.3); only `send-later` rows marked
    /// `clientFinalize`, and other plugins' rows, still come as `metadata-expiration` deltas
    /// for a client to act on.
    pub const EXPIRATION_EXECUTOR: &str = "expiration-executor";
    /// `threads.snooze` and `threads.unsnooze`.
    pub const SNOOZE: &str = "snooze";
    /// `messages.unsubscribe`.
    pub const UNSUBSCRIBE: &str = "unsubscribe";
    /// The `integrations.*` methods (docs/integrations-plan.md section 4).
    pub const INTEGRATIONS: &str = "integrations";
    /// `capabilities` in `hello` and `engine.status`, and `capabilities.changed`
    /// ([`Capabilities`](crate::Capabilities)).
    pub const CAPABILITIES: &str = "capabilities";
    /// The `ai.*` methods but triage, `ai.delta`, and `capabilities.ai`.
    pub const AI: &str = "ai";
    /// The engine owns the accounts (docs/desktop-client-plan.md 3.6): the registry
    /// (`accounts.json`) and secret store behind `account.add` and `account.remove
    /// {forget}`, accounts started at launch, rotated tokens stored instead of sent,
    /// the restart policy (`retryAt`, `authFailed`), `credentials.import`,
    /// `account.changed`, and `credentials`, `build` and `detached` in `hello`.
    pub const ACCOUNTS: &str = "accounts";
    /// Triage (docs/ai-plan.md section 4): the per-account triage of new inbox mail,
    /// `ai.triage.run`, `ai.triage.list`, `ai.triage.progress`, `capabilities.ai.triage`, and
    /// the `core.ai.*` settings keys.
    pub const AI_TRIAGE: &str = "ai-triage";
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
    /// A relayed connection asked for a method or task its device's scopes don't allow
    /// ([`policy`](crate::policy)).
    pub const FORBIDDEN_RELAYED: i64 = -32008;
    /// The engine already has as many socket connections as it accepts.
    pub const TOO_MANY_CONNECTIONS: i64 = -32009;
    /// "Integration failed": a provider call failed; `data` is the serialized
    /// `IntegrationError` `{key, message, retryable, status?, retryAfter?}`.
    pub const INTEGRATION_FAILED: i64 = -32010;
    /// "AI request failed": a model call failed or the account or feature is off; `data`
    /// is the serialized `AiError` `{key, message, retryable, status?, retryAfter?}`.
    pub const AI_FAILED: i64 = -32011;
    /// The engine's workers for this kind of call are all busy and their queue is full; the
    /// client may try again later.
    pub const ENGINE_BUSY: i64 = -32012;
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
