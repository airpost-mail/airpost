// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Engine-wide constants and tunables. Derived from Mailspring-Sync `MailSync/constants.h`,
//! the `#define`s at the top of `MailSync/SyncWorker.cpp`, and values scattered through
//! `main.cpp`, `MailStore.cpp`, `TaskProcessor.cpp`, `DAVWorker.cpp` and
//! `XOAuth2TokenManager.cpp`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//! Durations are in seconds unless the name says otherwise.

/// Folder `localStatus` keys. Starred ones drive the client's sync progress display.
pub mod ls {
    /// *
    pub const BUSY: &str = "busy";
    /// *
    pub const UIDNEXT: &str = "uidnext";
    /// *
    pub const SYNCED_MIN_UID: &str = "syncedMinUID";
    /// *
    pub const BODIES_PRESENT: &str = "bodiesPresent";
    /// *
    pub const BODIES_WANTED: &str = "bodiesWanted";
    pub const LAST_CLEANUP: &str = "lastCleanup";
    /// Deep and shallow scans run only on servers without CONDSTORE + QRESYNC.
    pub const LAST_SHALLOW: &str = "lastShallow";
    pub const LAST_DEEP: &str = "lastDeep";
    pub const HIGHESTMODSEQ: &str = "highestmodseq";
    pub const UIDVALIDITY: &str = "uidvalidity";
    pub const UIDVALIDITY_RESET_COUNT: &str = "uidvalidityResetCount";
    pub const MESSAGE_COUNT: &str = "messageCount";
    pub const UNSEEN_COUNT: &str = "unseenCount";
    pub const RECENT_COUNT: &str = "recentCount";
    /// Sent only: the SMTP server has filed a copy of a send here before.
    pub const SERVER_SAVES_SENT: &str = "serverSavesSent";
    /// The server takes new keywords in this folder (`\*` in PERMANENTFLAGS or FLAGS at the last
    /// SELECT).
    pub const KEYWORDS: &str = "keywords";
}

/// The container folder for helper folders such as Snoozed when the account names none.
pub const CONTAINER_FOLDER_DEFAULT: &str = "Tern";
/// Container folders Mailspring created on the server. Their helper folders are still
/// recognised, so an account that has one keeps using it.
pub const MAILSPRING_FOLDER_PREFIX_V1: &str = "[Mailspring]";
pub const MAILSPRING_FOLDER_PREFIX_V2: &str = "Mailspring";

/// Folder roles in the order role detection assigns them (`MailUtils::roles`).
pub const ROLES: [&str; 10] = [
    "all",
    "sent",
    "drafts",
    "spam",
    "important",
    "starred",
    "archive",
    "inbox",
    "trash",
    "snoozed",
];

/// The order the background pass visits folders in (`SyncWorker::syncNow`); others follow.
pub const FOLDER_SYNC_ROLE_ORDER: [&str; 7] =
    ["inbox", "sent", "drafts", "all", "archive", "trash", "spam"];

// -- Background and foreground sync (SyncWorker.cpp) --------------------------------------

pub const CACHE_CLEANUP_INTERVAL: i64 = 60 * 60;
pub const SHALLOW_SCAN_INTERVAL: i64 = 60 * 2;
pub const DEEP_SCAN_INTERVAL: i64 = 60 * 10;
/// How often a CONDSTORE + QRESYNC folder gets a full attributes-only scan. Those servers
/// report every change, so this only repairs messages never ingested in the first place.
pub const CONDSTORE_GAP_SCAN_INTERVAL: i64 = 60 * 60 * 24;
/// The longest the orphan sweep waits for a folder not fully scanned since a message was
/// orphaned; see the repo CLAUDE.md "Message Identity and Placements".
pub const ORPHAN_SWEEP_MAX_WAIT: i64 = 60 * 60 * 24;
/// Overrides [`ORPHAN_SWEEP_MAX_WAIT`] (seconds) so the harness can reach it.
pub const ORPHAN_SWEEP_MAX_WAIT_ENV: &str = "ORPHAN_SWEEP_MAX_WAIT";

/// [`ORPHAN_SWEEP_MAX_WAIT`], or the positive value of `$ORPHAN_SWEEP_MAX_WAIT`.
pub fn orphan_sweep_max_wait() -> i64 {
    std::env::var(ORPHAN_SWEEP_MAX_WAIT_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(ORPHAN_SWEEP_MAX_WAIT)
}

/// Server-synced drafts (docs/drafts-sync-design.md 3.2): a saved draft is pushed once it
/// has been left alone this long (seconds), so a burst of saves while the user types
/// uploads once...
pub const DRAFT_PUSH_QUIET: i64 = 20;
/// ...or this long when its attachments add up to more than [`DRAFT_PUSH_LARGE_BYTES`].
pub const DRAFT_PUSH_QUIET_LARGE: i64 = 120;
pub const DRAFT_PUSH_LARGE_BYTES: u64 = 1 << 20;
/// Overrides both quiet windows (seconds, 0 allowed) so the harness can reach them.
pub const DRAFT_PUSH_QUIET_ENV: &str = "TERN_DRAFT_PUSH_QUIET";
/// Most drafts pushed per foreground iteration; the rest wait for the next one.
pub const DRAFT_PUSHES_PER_PASS: usize = 5;

/// [`DRAFT_PUSH_QUIET`] or [`DRAFT_PUSH_QUIET_LARGE`], or the value of
/// `$TERN_DRAFT_PUSH_QUIET`.
pub fn draft_push_quiet(large: bool) -> i64 {
    std::env::var(DRAFT_PUSH_QUIET_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v >= 0)
        .unwrap_or(if large {
            DRAFT_PUSH_QUIET_LARGE
        } else {
            DRAFT_PUSH_QUIET
        })
}

/// The wait after `fails` failed pushes of a draft: 30 s, doubling, at most an hour.
pub fn draft_push_backoff(fails: u32) -> i64 {
    (30i64 << fails.saturating_sub(1).min(7)).min(60 * 60)
}

/// Most full headers requested at once; larger ranges are scanned attributes-only first.
pub const MAX_FULL_HEADERS_REQUEST_SIZE: u32 = 1024;
/// Chunk size of the first initial-sync request of a folder.
pub const FIRST_SYNC_CHUNK_SIZE: u32 = 750;
/// A folder holding fewer messages than this is swept as one UID range during initial
/// sync, however sparse its UID space.
pub const MAX_SINGLE_RANGE_SWEEP_SIZE: u32 = 5000;
/// A CONDSTORE change request whose modseq gap exceeds this asks only for the last
/// [`MODSEQ_TRUNCATION_UID_COUNT`] UIDs; the deep scan recovers the rest.
pub const MODSEQ_TRUNCATION_THRESHOLD: u64 = 4000;
pub const MODSEQ_TRUNCATION_UID_COUNT: u32 = 12000;
/// The shallow scan covers the newest messages down to this depth (`fetchMessageUIDAtDepth`).
pub const SHALLOW_SCAN_DEPTH: u32 = 399;
/// The foreground's plain-IMAP inbox check depth.
pub const FOREGROUND_SCAN_DEPTH: u32 = 100;
/// After new mail arrives in Inbox / All, bodies of up to this many are fetched at once
/// (the C++ loop's `if (count++ > 30) break;` stops after 32).
pub const NEW_MAIL_BODY_FETCH_LIMIT: usize = 32;

/// Bodies are cached for messages newer than this (three months) and for drafts.
pub const BODY_SYNC_MAX_AGE: i64 = 24 * 60 * 60 * 30 * 3;
/// Bodies fetched per folder per pass.
pub const BODY_SYNC_BATCH: usize = 30;
/// Cached bodies outside the window are purged once fetched this long ago.
pub const BODY_PURGE_AFTER_DAYS: i64 = 14;
/// Roles whose bodies are never cached.
pub const NO_BODY_CACHE_ROLES: [&str; 2] = ["spam", "trash"];

/// IDLE is restarted after this long (RFC 3501 asks clients to re-IDLE within 29 minutes).
pub const IDLE_TIMEOUT: i64 = 28 * 60;
/// A rename `SyncbackCategoryTask` recorded stays usable this long (seconds) for pairing a
/// folder the listing no longer shows with the path it was renamed to. Not in C++.
pub const FOLDER_RENAME_RECORD_TTL: i64 = 60 * 60;
/// Folder sync asks STATUS for at most this many newly listed folders per pass to pair them
/// with vanished ones by UIDVALIDITY; above it they are created afresh. Not in C++.
pub const RENAME_PAIRING_STATUS_LIMIT: usize = 50;
/// Gmail labels whose UIDVALIDITY is recorded per pass (one STATUS each, once per label),
/// so a label renamed on the server can be paired with its new name. Not in C++.
pub const LABEL_UIDVALIDITY_LOOKUPS_PER_PASS: usize = 25;
/// Workers sleep this long between passes and after a retryable error, unless woken.
pub const WORKER_SLEEP: u64 = 120;
/// `queue-task` interrupts the foreground IDLE after this debounce, so back-to-back tasks
/// run in one remote pass.
pub const QUEUE_TASK_IDLE_DEBOUNCE_MS: u64 = 300;

/// A Sent copy appearing within this window after a send is the server's late copy of it.
/// Days rather than the C++ 30 minutes: a provider that files its copy late can take longer
/// than that, and the check is cheap.
pub const SENT_COPY_CLEANUP_WINDOW: i64 = 7 * 24 * 60 * 60;
/// Delays between looks for the provider's Sent copy after a send, and the longer series
/// used once the Sent folder has `serverSavesSent`.
pub const SENT_POLL_DELAYS: [u64; 4] = [0, 1, 1, 2];
pub const SENT_POLL_DELAYS_SERVER_SAVES: [u64; 7] = [0, 1, 1, 2, 2, 3, 3];
/// `findUIDsOfRecentHeaderMessageID` looks at this many of the newest messages.
pub const RECENT_HEADER_SEARCH_DEPTH: u32 = 16;

// -- Store (MailStore.cpp, constants.h) -----------------------------------------------------

/// The C++ engine's last schema version (`CURRENT_VERSION`), which `V12_SCHEMA` creates.
/// Tern's own migrations start above it.
pub const BASE_SCHEMA_VERSION: i32 = 12;
/// The schema version this build migrates to: the last entry of `tern_store::schema::MIGRATIONS`.
pub const SCHEMA_VERSION: i32 = 13;
/// `_State` key marking a database this engine created.
pub const STATE_ENGINE_KEY: &str = "engine";
pub const STATE_ENGINE_RUST: &str = "rust";
pub const VACUUM_TIME_KEY: &str = "VACUUM_TIME";
/// Task data: when (Unix seconds) a remote phase put back after a retryable error runs
/// again; the foreground worker skips the task before then (TD43).
pub const TASK_NEXT_ATTEMPT_KEY: &str = "_nextAttemptAt";
/// The engine's database in the config dir. The C++ engine and Mailspring use `edgehill.db`.
pub const DATABASE_FILE: &str = "tern.db";
/// Where `migrate` moves a database the C++ engine wrote, for `import-legacy`.
pub const LEGACY_DATABASE_FILE: &str = "tern.legacy.db";
pub const FILES_DIR: &str = "files";
pub const SQLITE_BUSY_TIMEOUT_MS: u64 = 60 * 1000;

/// Chunk sizes (repo CLAUDE.md "Database access").
pub const FIND_LARGE_SET_CHUNK: usize = 900;
pub const THREAD_REFRESH_CHUNK: usize = 500;
pub const UID_LIST_CHUNK: usize = 200;
pub const REMOVE_CHUNK: usize = 100;

/// Delta stream delivery delays in milliseconds.
pub const STREAM_DELAY_WORKER_MS: u64 = 500;
pub const STREAM_DELAY_MAIN_MS: u64 = 5;
pub const DELTA_TYPE_PERSIST: &str = "persist";
pub const DELTA_TYPE_UNPERSIST: &str = "unpersist";
pub const DELTA_TYPE_METADATA_EXPIRATION: &str = "metadata-expiration";

/// `MetadataExpirationWorker::run`, which names none of these: seconds before the first
/// scan (plugins load after the main window), the wake with nothing scheduled, the
/// shortest sleep after a scan that emitted (the client needs time to clear what it was
/// sent) and after one that did not (the undo-send window), models per lookup, and the
/// delta delivery delay in milliseconds.
pub const METADATA_EXPIRATION_START_DELAY: u64 = 15;
pub const METADATA_EXPIRATION_IDLE: i64 = 2 * 60 * 60;
pub const METADATA_EXPIRATION_MIN_DELAY_AFTER_EMIT: i64 = 15;
pub const METADATA_EXPIRATION_MIN_DELAY_IDLE: i64 = 5;
pub const METADATA_EXPIRATION_CHUNK: usize = 100;
pub const METADATA_EXPIRATION_EMIT_DELAY_MS: u64 = 500;

/// The expiration executor's waits for a scheduled draft that lacks its body, its plaintext
/// flag or an attachment: seconds until each next look,
/// with a body fetch queued before each; after the last one the send fails.
pub const EXECUTOR_INCOMPLETE_DRAFT_WAITS: [i64; 4] = [60, 120, 240, 480];

/// How long a send the engine started itself stays queued while its SMTP server can't be
/// reached, in seconds (docs/tech-debt.md TD33); after that it fails as any send does.
pub const ENGINE_SEND_OFFLINE_RETRY_WINDOW: i64 = 24 * 60 * 60;

/// Finished tasks kept, and how far past it pruning waits (`cleanupOldTasksAtRuntime`).
pub const TASK_KEEP_FINISHED: usize = 100;
pub const TASK_PRUNE_SLOP: usize = 10;

/// `_sa` (syncedAt) hold set by a task's local phase.
pub const SYNCED_AT_HOLD: i64 = 24 * 60 * 60;

/// Message `rr` (RulesReadyState).
pub const RULES_READY_AWAITING_BODY: i64 = 0;
pub const RULES_READY_HAS_BODY: i64 = 1;
pub const RULES_READY_EMITTED: i64 = 2;

pub const CONTACT_SOURCE_MAIL: &str = "mail";

/// The plugin id of AI triage results on threads. Local to each
/// device: the metadata backends never send it to a Tern server.
pub const AI_TRIAGE_PLUGIN: &str = "tern-ai-triage";
pub const CONTACT_MAX_REFS: i64 = 100_000;
/// A message to more than this many addresses creates no contacts.
pub const CONTACT_MAX_RECIPIENTS: usize = 25;

/// Files at or below this size with a contentId do not count towards `attachmentCount`.
pub const INLINE_ATTACHMENT_COUNT_THRESHOLD: u64 = 12 * 1024;
/// Snippet and search text lengths, in UTF-16 code units.
pub const SNIPPET_LENGTH: usize = 400;
pub const SEARCH_TEXT_LENGTH: usize = 5000;

// -- Calendar and contacts (main.cpp, DAVWorker.cpp) ----------------------------------------

/// The first calContacts pass waits this long plus the account's start delay.
pub const CAL_CONTACTS_INITIAL_DELAY: u64 = 15;
pub const CALENDAR_SYNC_INTERVAL: u64 = 15 * 60;
pub const CONTACT_SYNC_INTERVAL: u64 = 75 * 60;
pub const CALENDAR_SYNC_PAST_MONTHS: u32 = 12;
pub const CALENDAR_SYNC_FUTURE_MONTHS: u32 = 18;
pub const DAV_MAX_BACKOFF_MS: u64 = 60_000;
pub const DAV_MIN_BACKOFF_MS: u64 = 100;
/// After Google's daily API quota is hit.
pub const DAV_QUOTA_SUSPEND: u64 = 4 * 60 * 60;
/// After a non-retryable calendar/contact error.
pub const DAV_FATAL_SUSPEND: u64 = 90 * 60;
pub const GOOGLE_SYNC_SOURCE: &str = "gpeople";
pub const CARDDAV_SYNC_SOURCE: &str = "carddav";

// -- OAuth (XOAuth2TokenManager.cpp) ---------------------------------------------------------

/// Cached access tokens are refreshed this long before they expire.
pub const OAUTH_EXPIRY_MARGIN: i64 = 60;
/// Assumed lifetime when the token response has no usable `expires_in`.
pub const OAUTH_ASSUMED_EXPIRES_IN: i64 = 3600;

// -- Process (main.cpp) ----------------------------------------------------------------------

/// The process exits with [`EXIT_STDIN_CLOSED`] once stdin has been closed this long.
pub const STDIN_ORPHAN_TIMEOUT: u64 = 30;
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_PATH_CHECK: i32 = 2;
/// `serve` found another engine listening on its socket, typically the previous one still
/// shutting down. The client retries it rather than counting a crash.
pub const EXIT_SOCKET_IN_USE: i32 = 3;
pub const EXIT_STDIN_CLOSED: i32 = 141;
/// How long stopping an account waits for its threads to exit before leaving the rest to
/// finish on their own (a blocking network call ends only at its own timeout).
pub const ACCOUNT_STOP_TIMEOUT_MS: u64 = 5_000;
/// How often an account's `main` thread checks for a stop between commands.
pub const ACCOUNT_STOP_POLL_MS: u64 = 200;
/// `account.reset` of an account the engine still held: how long to let the threads its stop
/// left running finish before deleting the account's rows. They cannot be joined.
pub const ACCOUNT_RESET_STRAGGLER_WAIT_MS: u64 = 10_000;
/// `serve`: how long `account.add` waits for the account's `main` thread to open the
/// database before answering with `starting`.
pub const ACCOUNT_START_WAIT_MS: u64 = 10_000;
/// `serve`: `$CONFIG_DIR_PATH/<this>` takes the lines of threads that work for no account.
pub const ENGINE_LOG_FILE: &str = "engine.log";
/// `--socket`: a connection whose unsent responses and deltas pass this many bytes is sent
/// a `disconnect` notification and closed.
pub const SOCKET_QUEUE_BYTES: usize = 64 * 1024 * 1024;
/// `--socket`: deltas flushed before the first `subscribe` are kept for it up to this many
/// bytes, so the client that started the engine sees what stdout would have shown it.
pub const SOCKET_BACKLOG_BYTES: usize = 16 * 1024 * 1024;
/// `--socket`: connections open at once; each holds two threads. `tern-gateway` keeps at
/// most 64 of them for its browsers, which leaves room for
/// the desktop's windows and the CLI.
pub const SOCKET_MAX_CONNECTIONS: usize = 128;
/// Log rotation (`rotating_file_sink_mt(path, 1048576 * 5, 3)`).
pub const LOG_MAX_BYTES: u64 = 1_048_576 * 5;
pub const LOG_MAX_FILES: usize = 3;
/// Log lines are flushed this long after the last activity.
pub const LOG_FLUSH_DEBOUNCE_MS: u64 = 1000;

/// Thread names, which appear in every log line and which the harness filters on.
pub mod thread {
    pub const MAIN: &str = "main";
    pub const BACKGROUND: &str = "background";
    pub const FOREGROUND: &str = "foreground";
    pub const CAL_CONTACTS: &str = "calContacts";
    pub const METADATA: &str = "metadata";
    pub const METADATA_EXPIRATION: &str = "metadataExpiration";
    /// Reads the metadata stream's lines for the `metadata` thread, which can then stop
    /// while a read is blocked. Logs nothing.
    pub const METADATA_STREAM: &str = "metadataStream";
    /// The one-off thread a `sync-calendar` command starts.
    pub const CALENDAR: &str = "calendar";
    /// The short-lived thread that interrupts IDLE after `queue-task`'s debounce.
    pub const QUEUE_TASK_DEBOUNCE: &str = "queueTaskDebounce";
    pub const DELTA_STREAM_FLUSH: &str = "DeltaStreamFlush";
    /// Reads stdin commands for the main thread and watches for the parent going away.
    pub const STDIN: &str = "stdin";
    /// `--socket`: accepts connections.
    pub const SOCKET: &str = "socket";
    /// `--socket`: one per connection, reading its requests.
    pub const SOCKET_READER: &str = "socketReader";
    /// `--socket`: one per connection, writing its queue.
    pub const SOCKET_WRITER: &str = "socketWriter";
    /// `--socket`: removes the socket files on SIGTERM / SIGINT.
    pub const SIGNALS: &str = "signals";
    /// `--socket`: the workers answering `files.get` once its download has written the file.
    pub const FILES: &str = "files";
    /// `--socket`: the workers running `messages.unsubscribe` calls that wait for a body or
    /// make a one-click POST.
    pub const UNSUBSCRIBE: &str = "unsubscribe";
    /// `serve`: the workers running `integrations.*` calls.
    pub const INTEGRATIONS: &str = "integrations";
    /// `serve`: the workers running `ai.*` model calls.
    pub const AI: &str = "ai";
    /// `serve`: one per account, triaging its new inbox mail and `ai.triage.run` jobs.
    pub const AI_TRIAGE: &str = "aiTriage";
    /// `serve`: one per account, applying the mail rules to its arriving mail and running
    /// its `rules.reprocess` jobs.
    pub const MAIL_RULES: &str = "mailRules";
    /// `serve`: waits for local midnight while a provider is over its daily token cap, to
    /// send `capabilities.changed` when the cap resets.
    pub const AI_CAP_RESET: &str = "aiCapReset";
    /// `serve`: the workers running `account.discover`, `account.oauth.complete` and
    /// `account.add` / `account.setSecrets` with a test.
    pub const ACCOUNT_SETUP: &str = "accountSetup";
    /// `serve`: joins a failed account's threads and reports it stopped.
    pub const ACCOUNT_STOP: &str = "accountStop";
    /// `serve`: starts the registered accounts at engine start, and restarts a stopped one
    /// after its backoff.
    pub const ACCOUNT_RESTART: &str = "accountRestart";
    /// `serve`: hashes the executable for `hello.build`.
    pub const BUILD_ID: &str = "buildId";
    /// `serve`: the workers running `account.test` for the parent.
    pub const ACCOUNT_TEST: &str = "accountTest";
    /// `serve`: the workers running `account.reset` for the parent.
    pub const ACCOUNT_RESET: &str = "accountReset";
    /// `serve`: the worker running `engine.resetDatabase` and `engine.setLogLevel` for the
    /// parent, one at a time.
    pub const ENGINE_MAINTENANCE: &str = "engineMaintenance";
}
