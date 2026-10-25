// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine's local socket protocol, v1.
//!
//! JSON-RPC 2.0 over newline-delimited JSON on a Unix domain socket. Clients send requests
//! with an `id` and get one response each; the engine's deltas arrive as `delta`
//! notifications once the client has called `subscribe`. `hello` must be the first request.
//!
//! Depends on serde, serde_json and schemars only, so the CLI and third-party tools can use
//! it without pulling in the engine. The engine serves the protocol; this crate also has a
//! blocking [`Client`], used by the CLI and the engine's tests.
//!
//! The types describe the protocol for clients too: [`schema`] turns them into one JSON
//! Schema document, from which `packages/client` (`@tern/client`) generates the TypeScript
//! types the desktop and the web use. [`models`] and [`tasks`] describe the model and task
//! JSON the engine stores and sends; [`query_attrs`] is what `query` can match on.

/// Constants with their doc comments: `pub const NAME: T = value;` for each, and `NAMED`,
/// every `(name, value, doc)` in order, for the schema ([`schema`]).
macro_rules! named_consts {
    ($ty:ty; $($(#[doc = $doc:literal])* $name:ident = $value:expr;)*) => {
        $($(#[doc = $doc])* pub const $name: $ty = $value;)*
        /// Every constant in this module: its name, value and doc comment.
        pub const NAMED: &[(&str, $ty, &str)] = &[$((stringify!($name), $value, concat!($($doc, "\n",)*)),)*];
    };
}

mod accounts;
mod ai;
mod client;
mod drafts;
mod envelope;
mod files;
mod frame;
mod integrations;
mod mail;
mod methods;
pub mod models;
mod pim;
pub mod policy;
pub mod query_attrs;
mod read;
mod rules;
pub mod schema;
mod settings;
pub mod tasks;
mod templates;

pub use accounts::*;
pub use ai::*;
pub use client::{Client, ClientError, Notifications};
pub use drafts::*;
pub use envelope::{
    delta_notification_line, parse_message, parse_request, ErrorObject, Id, Message, Notification,
    Request, Response,
};
pub use files::*;
pub use frame::{encode, read_frame, write_frame, FrameError};
pub use integrations::*;
pub use mail::*;
pub use methods::*;
pub use models::*;
pub use pim::*;
pub use read::*;
pub use rules::*;
pub use settings::*;
pub use tasks::*;
pub use templates::*;

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
    named_consts! { &str;
        HELLO = "hello";
        SUBSCRIBE = "subscribe";
        TASK_QUEUE = "task.queue";
        TASK_CANCEL = "task.cancel";
        SYNC_WAKE = "sync.wake";
        BODIES_FETCH = "bodies.fetch";
        CALENDAR_SYNC = "calendar.sync";
        ENGINE_STATUS = "engine.status";
        FOLDERS_LIST = "folders.list";
        LABELS_LIST = "labels.list";
        THREADS_LIST = "threads.list";
        THREADS_GET = "threads.get";
        MESSAGES_GET = "messages.get";
        SEARCH = "search";
        CONTACTS_SEARCH = "contacts.search";
        TASKS_GET = "tasks.get";
        /// Every `ThreadCounts` row of the accounts ([`feature::COUNTS`](crate::feature::COUNTS)).
        COUNTS_GET = "counts.get";
        /// Stored tasks by account, status, class or draft ([`feature::TASKS_LIST`](crate::feature::TASKS_LIST)).
        TASKS_LIST = "tasks.list";
        /// Messages or threads carrying a plugin's metadata
        /// ([`feature::METADATA_LIST`](crate::feature::METADATA_LIST)).
        METADATA_LIST = "metadata.list";
        /// A ModelQuery over the stored models ([`feature::QUERY`](crate::feature::QUERY)).
        QUERY = "query";
        /// A File by id, downloaded first when missing ([`feature::FILES`](crate::feature::FILES)),
        /// params [`FilesGetParams`](crate::FilesGetParams). Local clients only; answered out of
        /// order while it waits for the download.
        FILES_GET = "files.get";
        /// A chunk of a downloaded File, params [`FilesReadParams`](crate::FilesReadParams).
        FILES_READ = "files.read";
        /// Stores bytes as a new File for a draft, params [`FilesPutParams`](crate::FilesPutParams).
        FILES_PUT = "files.put";
        /// Moves threads to Snoozed with the metadata that brings them back
        /// ([`feature::SNOOZE`](crate::feature::SNOOZE)), params
        /// [`ThreadsSnoozeParams`](crate::ThreadsSnoozeParams).
        THREADS_SNOOZE = "threads.snooze";
        /// Ends a snooze now, params [`ThreadsUnsnoozeParams`](crate::ThreadsUnsnoozeParams).
        THREADS_UNSNOOZE = "threads.unsnooze";
        /// Unsubscribes from the list a message came from
        /// ([`feature::UNSUBSCRIBE`](crate::feature::UNSUBSCRIBE)), params
        /// [`MessagesUnsubscribeParams`](crate::MessagesUnsubscribeParams). Socket only; answered
        /// out of order while a one-click POST runs.
        MESSAGES_UNSUBSCRIBE = "messages.unsubscribe";
        /// Builds a draft as the desktop does and saves it
        /// ([`feature::DRAFT_PIPELINE`](crate::feature::DRAFT_PIPELINE)), params
        /// [`DraftCreateParams`](crate::DraftCreateParams); answers with its Message JSON.
        DRAFT_CREATE = "draft.create";
        /// Sends a draft now or after an undo window, params
        /// [`DraftSendParams`](crate::DraftSendParams).
        DRAFT_SEND = "draft.send";
        /// Cancels a send waiting out its undo window, params
        /// [`DraftCancelSendParams`](crate::DraftCancelSendParams).
        DRAFT_CANCEL_SEND = "draft.cancelSend";
        /// The checks before a send, params [`DraftValidateParams`](crate::DraftValidateParams).
        DRAFT_VALIDATE = "draft.validate";
        /// An edit to a stored draft ([`feature::DRAFT_EDITING`](crate::feature::DRAFT_EDITING)),
        /// params [`DraftUpdateParams`](crate::DraftUpdateParams); answers with its Message JSON.
        DRAFT_UPDATE = "draft.update";
        /// Deletes a draft, params [`DraftDeleteParams`](crate::DraftDeleteParams).
        DRAFT_DELETE = "draft.delete";
        /// The templates ([`feature::TEMPLATES`](crate::feature::TEMPLATES)), result
        /// [`TemplatesListResult`](crate::TemplatesListResult).
        TEMPLATES_LIST = "templates.list";
        /// One template with its body, params [`TemplateNameParams`](crate::TemplateNameParams),
        /// result [`TemplateResult`](crate::TemplateResult).
        TEMPLATES_GET = "templates.get";
        /// Saves a template, params [`TemplateSaveParams`](crate::TemplateSaveParams), result
        /// [`TemplateResult`](crate::TemplateResult). Local clients only.
        TEMPLATES_SAVE = "templates.save";
        /// Deletes a template's file, params [`TemplateNameParams`](crate::TemplateNameParams).
        /// Local clients only.
        TEMPLATES_DELETE = "templates.delete";
        /// Occurrences of stored events or of an ICS
        /// ([`feature::CALENDAR_EDITING`](crate::feature::CALENDAR_EDITING)), params
        /// [`EventsExpandParams`](crate::EventsExpandParams).
        EVENTS_EXPAND = "events.expand";
        /// A new event, params [`EventsCreateParams`](crate::EventsCreateParams).
        EVENTS_CREATE = "events.create";
        /// An edit to an event or some of its occurrences, params
        /// [`EventsUpdateParams`](crate::EventsUpdateParams).
        EVENTS_UPDATE = "events.update";
        /// An answer to an invitation or a counter-proposal, params
        /// [`EventsRsvpParams`](crate::EventsRsvpParams).
        EVENTS_RSVP = "events.rsvp";
        /// A contact's details ([`feature::CONTACT_EDITING`](crate::feature::CONTACT_EDITING)),
        /// params [`ContactsGetParams`](crate::ContactsGetParams).
        CONTACTS_GET = "contacts.get";
        /// A new contact, params [`ContactsCreateParams`](crate::ContactsCreateParams).
        CONTACTS_CREATE = "contacts.create";
        /// An edit to a contact, params [`ContactsUpdateParams`](crate::ContactsUpdateParams).
        CONTACTS_UPDATE = "contacts.update";
        /// Contacts from a .vcf file, params [`ContactsImportParams`](crate::ContactsImportParams).
        CONTACTS_IMPORT = "contacts.import";
        /// Contacts as a .vcf file, params [`ContactsExportParams`](crate::ContactsExportParams).
        CONTACTS_EXPORT = "contacts.export";
        /// Every account the engine holds, with its state.
        ACCOUNT_LIST = "account.list";
        /// `serve` mode: register an account, store its secrets and start syncing it
        /// ([`AccountAddParams`](crate::AccountAddParams)). On the parent's stdin, and on the
        /// socket of an engine with [`feature::ACCOUNT_SETUP`](crate::feature::ACCOUNT_SETUP)
        /// (local-only class, [`policy`](crate::policy)).
        ACCOUNT_ADD = "account.add";
        /// `serve` mode: stop an account, unregister it and delete its secrets, with `revoke`
        /// also its OAuth grant and with `deleteData` its rows
        /// ([`AccountRemoveParams`](crate::AccountRemoveParams)). Without `deleteData` the rows
        /// stay. Where `account.add` is taken.
        ACCOUNT_REMOVE = "account.remove";
        /// IMAP and SMTP settings for an address ([`AccountDiscoverParams`](crate::AccountDiscoverParams)).
        /// Open to relayed connections; answered out of order (MX and autoconfig lookups).
        ACCOUNT_DISCOVER = "account.discover";
        /// Starts an OAuth sign-in: the engine keeps the PKCE verifier, `state` and client
        /// secret ([`OAuthBeginParams`](crate::OAuthBeginParams)).
        ACCOUNT_OAUTH_BEGIN = "account.oauth.begin";
        /// Ends it with the redirect's `code` and `state`: exchange, profile, discovery, test,
        /// then the account is stored and started ([`OAuthCompleteParams`](crate::OAuthCompleteParams)).
        /// Answered out of order.
        ACCOUNT_OAUTH_COMPLETE = "account.oauth.complete";
        /// New credentials for a registered account ([`AccountSetSecretsParams`](crate::AccountSetSecretsParams)).
        ACCOUNT_SET_SECRETS = "account.setSecrets";
        /// Changes an account's JSON ([`AccountUpdateParams`](crate::AccountUpdateParams)).
        /// Relayed devices without the admin scope may change only
        /// [`ACCOUNT_DISPLAY_FIELDS`](crate::ACCOUNT_DISPLAY_FIELDS).
        ACCOUNT_UPDATE = "account.update";
        /// The order of `account.list` ([`AccountReorderParams`](crate::AccountReorderParams)).
        ACCOUNT_REORDER = "account.reorder";
        /// Stores a secret that isn't an account's ([`SecretsSetParams`](crate::SecretsSetParams)).
        /// There is no `secrets.get`.
        SECRETS_SET = "secrets.set";
        SECRETS_DELETE = "secrets.delete";
        /// Which of the named secrets are stored ([`SecretsHasParams`](crate::SecretsHasParams)).
        SECRETS_HAS = "secrets.has";
        /// The Tern server the accounts sync metadata with ([`ServerSetParams`](crate::ServerSetParams)).
        SERVER_SET = "server.set";
        /// Forgets it and its token; accounts sync metadata locally.
        SERVER_CLEAR = "server.clear";
        /// `serve` mode, the parent's stdin only (the socket too on a `--detached` engine): stop
        /// every account, remove the socket files and exit 0.
        ENGINE_SHUTDOWN = "engine.shutdown";
        /// `serve` mode, the parent's stdin only: register accounts and store secrets a client
        /// held until now, params
        /// [`CredentialsImportParams`](crate::CredentialsImportParams).
        CREDENTIALS_IMPORT = "credentials.import";
        /// Notification: an account's registered JSON changed, `params` =
        /// [`AccountChangedParams`](crate::AccountChangedParams). Filtered by account, never held
        /// for a later subscriber: a client calls `account.list` after `subscribe`.
        ACCOUNT_CHANGED = "account.changed";
        /// `serve` mode: check an account's IMAP and SMTP settings, as `--mode test` does
        /// ([`AccountTestParams`](crate::AccountTestParams)). The result is that mode's JSON
        /// object, with any rotated refresh token in its `account` on stdin only. Where
        /// `account.add` is taken; answered out of order.
        ACCOUNT_TEST = "account.test";
        /// `serve` mode: stop the account if the engine holds it, delete its rows from the
        /// database (`--mode reset`) and start it again from the registry to resync (a client
        /// that adds it again gets ACCOUNT_EXISTS). Where `account.add` is taken; answered out of
        /// order.
        ACCOUNT_RESET = "account.reset";
        /// `serve` mode, the parent's stdin only: stop and forget every account, delete the
        /// database and create an empty one. The client adds the accounts again.
        ENGINE_RESET_DATABASE = "engine.resetDatabase";
        /// `serve` mode, the parent's stdin only: verbose protocol logging until a time, for
        /// every account; running accounts restart to pick it up.
        ENGINE_SET_LOG_LEVEL = "engine.setLogLevel";
        /// The end of an account's log, or of the engine's own log.
        ENGINE_LOGS_TAIL = "engine.logs.tail";
        /// The stored engine-owned settings ([`feature::SETTINGS`](crate::feature::SETTINGS)),
        /// params [`SettingsGetParams`](crate::SettingsGetParams), result
        /// [`SettingsValues`](crate::SettingsValues).
        SETTINGS_GET = "settings.get";
        /// Stores settings, params [`SettingsValues`](crate::SettingsValues), result `{}`.
        SETTINGS_SET = "settings.set";
        /// `serve` mode, the parent's stdin only, debug builds only: stop one account as a
        /// fatal error would (`test-crash` on stdin in `sync` mode).
        TEST_CRASH = "test.crash";
        /// As [`TEST_CRASH`], with a panic on the account's `main` thread.
        TEST_PANIC = "test.panic";
        /// Notification: an account changed state, `params` =
        /// [`AccountStateParams`](crate::AccountStateParams).
        ACCOUNT_STATE = "account.state";
        /// Notification: one flushed delta, `params` = [`DeltaParams`](crate::DeltaParams).
        DELTA = "delta";
        /// Notification: the engine is about to close the connection, `params` =
        /// [`DisconnectParams`](crate::DisconnectParams).
        DISCONNECT = "disconnect";
        /// Notification: new mail, `params` = [`MailNewParams`](crate::MailNewParams). Filtered by
        /// account only, never held for a later subscriber ([`feature::MAIL_NEW`](crate::feature::MAIL_NEW)).
        MAIL_NEW = "mail.new";
        /// Notification: settings another connection (or the parent) changed, `params` =
        /// [`SettingsValues`](crate::SettingsValues) with the new values, null for a cleared key.
        /// Sent to every subscribed connection whatever its filter, never held for a later
        /// subscriber: a client calls `settings.get` after `subscribe`.
        SETTINGS_CHANGED = "settings.changed";
        /// The `integrations.*` methods ([`feature::INTEGRATIONS`](crate::feature::INTEGRATIONS)).
        /// Each carries the
        /// provider's `config`, key included, until the engine owns the credentials. Local
        /// clients only, answered out of order on the engine's integrations workers (T21); a
        /// provider failure is [`code::INTEGRATION_FAILED`](crate::code::INTEGRATION_FAILED).
        INTEGRATIONS_TRANSLATE = "integrations.translate";
        INTEGRATIONS_DETECT = "integrations.detect";
        INTEGRATIONS_LANGUAGES = "integrations.languages";
        INTEGRATIONS_GRAMMAR = "integrations.grammar";
        INTEGRATIONS_ENRICH = "integrations.enrich";
        INTEGRATIONS_TEST = "integrations.test";
        /// Notification: the engine's `capabilities` changed ([`feature::CAPABILITIES`](crate::feature::CAPABILITIES)),
        /// `params` = the new capabilities object, whole. Sent to every subscribed connection
        /// whatever its filter, never held for a later subscriber: `hello` and `engine.status`
        /// carry the current value.
        CAPABILITIES_CHANGED = "capabilities.changed";
        /// The `ai.*` methods ([`feature::AI`](crate::feature::AI)), params and results
        /// in [`crate::AiSettings`]. Until the engine keeps the AI
        /// settings and keys, local clients send the provider `config`, key included, and the
        /// `settings` with each call; a relayed connection may send neither
        /// ([`policy`](crate::policy)). Answered out of order on the engine's AI workers (T21);
        /// a failed model call is [`code::AI_FAILED`](crate::code::AI_FAILED).
        AI_MODELS = "ai.models";
        AI_TEST = "ai.test";
        AI_SUMMARIZE = "ai.summarize";
        AI_SUGGEST_REPLIES = "ai.suggestReplies";
        AI_DRAFT_REPLY = "ai.draftReply";
        /// Answered at once, in order: stops the calling connection's stream `streamId`.
        AI_CANCEL = "ai.cancel";
        /// Token counts per day, provider and feature since the engine started.
        AI_USAGE = "ai.usage";
        /// Inbox triage; not served yet.
        AI_TRIAGE_RUN = "ai.triage.run";
        AI_TRIAGE_LIST = "ai.triage.list";
        /// Notification: the next piece of a streamed answer, `params` =
        /// [`AiDeltaParams`](crate::AiDeltaParams). Sent only to the connection that started the
        /// stream, in order, without `subscribe`; the call's response comes after the last one
        /// and carries the whole text.
        AI_DELTA = "ai.delta";
        /// Notification: triage progress for the connection that called `ai.triage.run`.
        AI_TRIAGE_PROGRESS = "ai.triage.progress";
        /// Mail rules ([`feature::RULES`](crate::feature::RULES)): params and results in
        /// [`crate::RulesReprocessParams`] and [`crate::RulesCancelParams`]. The rules themselves
        /// are the `mailRules` setting.
        RULES_REPROCESS = "rules.reprocess";
        RULES_CANCEL = "rules.cancel";
        /// Notification: a `rules.reprocess` job's progress, to the connection that started it
        /// only, without `subscribe`; never held.
        RULES_PROGRESS = "rules.progress";
        /// Notification: the engine disabled a failing rule. Sent to every subscribed connection
        /// whatever its filter, never held: the rule's `disabled` and `disabledReason` are saved in
        /// `mailRules`, and `settings.changed` carries them as well.
        RULES_DISABLED = "rules.disabled";
    }
}

/// The names in `hello.features` and `engine.status.features`.
/// Each one is additive: a client checks for it before it
/// relies on the method, field or notification it names.
pub mod feature {
    named_consts! { &str;
        /// `counts.get`, and `ThreadCounts` deltas whenever a category's counts change.
        COUNTS = "counts";
        /// `tasks.list`.
        TASKS_LIST = "tasks-list";
        /// `metadata.list`.
        METADATA_LIST = "metadata-list";
        /// `query`. Its `search.grammar: true` comes with [`SEARCH_GRAMMAR`].
        QUERY = "query";
        /// The search grammar (`from:`, `in:`, `is:`, `has:attachment`, `before:`/`after:`,
        /// `AND`/`OR`/`NOT`, parentheses) in `query.search {grammar: true, tz, now}` and in
        /// `search {grammar: true, tz, now}`, and `search {accounts}`.
        SEARCH_GRAMMAR = "search-grammar";
        /// `hidden` on the Message JSON the read methods return (`Message.isHidden()`).
        MESSAGE_HIDDEN = "message-hidden";
        /// `mail.new` notifications for subscribed connections.
        MAIL_NEW = "mail-new";
        /// `hello.relayed`: the engine applies [`policy`](crate::policy) to relayed
        /// connections. `tern-gateway` refuses to relay to an engine without it, which would
        /// ignore the field.
        RELAYED = "relayed";
        /// `settings.get`, `settings.set` and `settings.changed`.
        SETTINGS = "settings";
        /// `files.get`, `files.read`, `files.put`, and GetMessageRFC2822Task's file-id mode (no
        /// `filepath`: the export becomes the File `fileId`, set by the local phase).
        FILES = "files";
        /// The engine acts on due `send-later`, `thread-snooze` and `send-reminders` metadata
        /// itself; only `send-later` rows marked
        /// `clientFinalize`, and other plugins' rows, still come as `metadata-expiration` deltas
        /// for a client to act on.
        EXPIRATION_EXECUTOR = "expiration-executor";
        /// `threads.snooze` and `threads.unsnooze`.
        SNOOZE = "snooze";
        /// `messages.unsubscribe`.
        UNSUBSCRIBE = "unsubscribe";
        /// `serve`: `draft.create`, `draft.send` (with `bodies` kept with the pending send),
        /// `draft.cancelSend` and `draft.validate`, and the `composing.quoteTexts` setting.
        DRAFT_PIPELINE = "draft-pipeline";
        /// `serve`: `draft.update`, `draft.delete`, and `attach` on `draft.create`.
        DRAFT_EDITING = "draft-editing";
        /// `serve`: `templates.list|get|save|delete`, `template` on `draft.create`, and the
        /// `template-variable` warning of `draft.validate`.
        TEMPLATES = "templates";
        /// `warnings` ([`MessageWarning`](crate::MessageWarning)) on the Message JSON of
        /// `messages.get` and `threads.get`, left out when there are none.
        MESSAGE_WARNINGS = "message-warnings";
        /// `draft.send` and the expiration executor send drafts with `open-tracking` or
        /// `link-tracking` on tracked, in the Tern URL format, when the connected server offers
        /// tracking (`capabilities.server.readReceipts` / `linkTracking`); the executor claims due
        /// rows on the server first (docs/backend-plan.md 3.6, 3.8). Clients don't transform
        /// tracked drafts themselves.
        TRACKING = "tracking";
        /// The `integrations.*` methods.
        INTEGRATIONS = "integrations";
        /// `capabilities` in `hello` and `engine.status`, and `capabilities.changed`
        /// ([`Capabilities`](crate::Capabilities)).
        CAPABILITIES = "capabilities";
        /// The `ai.*` methods but triage, `ai.delta`, and `capabilities.ai`.
        AI = "ai";
        /// The engine owns the accounts: the registry
        /// (`accounts.json`) and secret store behind `account.add` and `account.remove` (which
        /// unregisters and deletes the secrets), accounts started at launch, rotated tokens
        /// stored instead of sent, the restart policy (`retryAt`, `authFailed`),
        /// `credentials.import`, `account.changed`, and `credentials`, `build` and `detached` in
        /// `hello`.
        ACCOUNTS = "accounts";
        /// Triage: the per-account triage of new inbox mail,
        /// `ai.triage.run`, `ai.triage.list`, `ai.triage.progress`, `capabilities.ai.triage`, and
        /// the `core.ai.*` settings keys.
        AI_TRIAGE = "ai-triage";
        /// AI providers' keys in the secret store: stored providers read
        /// `integration:ai-<id>`, the AI calls take `provider` without a key, and a stored
        /// provider's key goes when `settings.set` removes the provider or points it at another
        /// kind or server.
        AI_KEYS = "ai-keys";
        /// Account setup on the socket: `account.discover`,
        /// `account.oauth.begin|complete`, `account.add` and `account.test` with `secrets`,
        /// `account.setSecrets`, `account.update`, `account.reorder`, `account.remove {revoke,
        /// deleteData}`, `account.reset`, `secrets.*` and `server.*`; ids derived by the engine;
        /// `integrations.*` read a missing key from the secret `integration:<provider>`.
        ACCOUNT_SETUP = "account-setup";
        /// `serve` applies the `mailRules` setting to arriving mail itself (T12), with
        /// `rules.reprocess`, `rules.cancel`, `rules.progress`, `rules.disabled` and the
        /// `mailRulesAutoSince` setting. Clients that see it stop evaluating rules.
        RULES = "rules";
        /// `events.expand`, `events.create`, `events.update` and `events.rsvp`, and
        /// SyncbackEventTask's `restore` (an undo or redo re-sent over the copy stored now).
        CALENDAR_EDITING = "calendar-editing";
        /// `contacts.get`, `contacts.create`, `contacts.update`, `contacts.import` and
        /// `contacts.export`.
        CONTACT_EDITING = "contact-editing";
    }
}

/// Error codes. The standard JSON-RPC codes, then the engine's from `-32000`
/// down.
pub mod code {
    named_consts! { i64;
        /// The line is not JSON. The response's `id` is null.
        PARSE_ERROR = -32700;
        /// JSON, but not a request object.
        INVALID_REQUEST = -32600;
        METHOD_NOT_FOUND = -32601;
        INVALID_PARAMS = -32602;
        INTERNAL_ERROR = -32603;
        /// `hello` named a protocol version the engine does not speak; `data.supported` lists
        /// the ones it does.
        UNSUPPORTED_PROTOCOL = -32000;
        /// The `accountId` is not one the engine holds.
        UNKNOWN_ACCOUNT = -32001;
        /// The task's local phase failed; `data` is the `SyncError` JSON.
        TASK_REJECTED = -32002;
        /// A request other than `hello` came before a successful `hello`.
        HELLO_REQUIRED = -32003;
        /// A read named a folder, label, thread or message the account does not have.
        NOT_FOUND = -32004;
        /// The account is known but not running (it stopped on an error, or is being
        /// removed); `data` is `{state, error}`.
        ACCOUNT_STOPPED = -32005;
        /// `account.add` named an account that is already starting or running.
        ACCOUNT_EXISTS = -32006;
        /// The method is only accepted on the parent's stdin (`credentials.import`,
        /// `engine.shutdown`, ...), or only on the socket (`subscribe`).
        WRONG_CHANNEL = -32007;
        /// A relayed connection asked for a method or task its device's scopes don't allow
        /// ([`policy`](crate::policy)).
        FORBIDDEN_RELAYED = -32008;
        /// The engine already has as many socket connections as it accepts.
        TOO_MANY_CONNECTIONS = -32009;
        /// "Integration failed": a provider call failed; `data` is the serialized
        /// `IntegrationError` `{key, message, retryable, status?, retryAfter?}`.
        INTEGRATION_FAILED = -32010;
        /// "AI request failed": a model call failed or the account or feature is off; `data`
        /// is the serialized `AiError` `{key, message, retryable, status?, retryAfter?}`.
        AI_FAILED = -32011;
        /// The engine's workers for this kind of call are all busy and their queue is full; the
        /// client may try again later.
        ENGINE_BUSY = -32012;
        /// "Account test failed": `account.add {test}`, `account.setSecrets {test}` or
        /// `account.oauth.complete` found the settings or credentials don't work; `data` is the
        /// `account.test` result (`error`, `error_service`, `log`, ...).
        ACCOUNT_TEST_FAILED = -32013;
        /// "OAuth sign-in failed": the code exchange or the profile request failed, or the
        /// account has no mailbox; `data` is `{key, status?, providerError?}`.
        OAUTH_FAILED = -32014;
    }
}

/// The `state` of an account in `hello`, `engine.status`, `account.list` and `account.state`.
pub mod account_state {
    named_consts! { &str;
        /// Added; its `main` thread has not opened the database yet.
        STARTING = "starting";
        RUNNING = "running";
        /// Stopped by a failure (`error` says which). The engine does not restart it; the
        /// client may `account.add` it again.
        STOPPED = "stopped";
        /// `account.remove` finished; only sent as an `account.state` notification.
        REMOVED = "removed";
    }
}
