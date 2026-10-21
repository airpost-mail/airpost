// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Log lines that something outside the engine matches on.
//!
//! The harness (`apps/engine/test`) waits for and counts these with regexes, and the client
//! reads a few. Each function names the scenario or harness code that depends on it; change
//! the wording only together with that consumer. `tests/logmsg.rs` checks every function
//! against the regexes copied from those files.
//!
//! Most of the wording is the C++ engine's (Mailspring-Sync `SyncWorker.cpp`,
//! `MailProcessor.cpp`, `TaskProcessor.cpp`, `main.cpp`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! Formatting follows the C++ engine: `{}` of a bool prints `true` / `false`, paths are
//! printed as stored (modified UTF-7), and sub-steps are prefixed `"- "`, `"-- "` or `"-X "`
//! (repo CLAUDE.md "Logging").

// -- Harness core (test/harness/engine.py) ------------------------------------------------

/// Quiescence: the last `background` line must be exactly this and at least 2 s old.
pub fn sync_loop_complete() -> &'static str {
    "Sync loop complete."
}

/// `sync_pass` waits for `^Marking all folders as \`busy\`` after `wake-workers`.
pub fn marking_all_folders_busy() -> &'static str {
    "Marking all folders as `busy`"
}

/// Quiescence without `--verbose`: the last foreground engine line starts with
/// `Idling on folder`.
pub fn idling_on_folder(path: &str) -> String {
    format!("Idling on folder {path}")
}

/// Quiescence on servers without IDLE (NetEase): the last foreground line starts with
/// `Connection does not support idling`.
pub fn idle_not_supported() -> &'static str {
    "Connection does not support idling. Locking until more to do..."
}

/// A worker hit a retryable error and sleeps (connection-dropped-during-command,
/// list-refused-transiently wait for `^--sleeping` / `--sleeping`).
pub fn sleeping() -> &'static str {
    "--sleeping"
}

/// `<direction> <line>` for protocol traffic: `IMAP_LINE` in engine.py splits on it, the
/// foreground idle check matches `sent` lines against `^\d+ IDLE$`, and scenarios match
/// `recv \* VANISHED` (qresync-bulk-expunge-during-idle, qresync-wide-vanished-range,
/// orphan-sweep-with-unreadable-folder) and `sent \d+ APPEND` (gmail-send-and-labels).
pub fn protocol_line(direction: &str, line: &str) -> String {
    format!("{direction} {line}")
}

// -- Background sync (SyncWorker.cpp) ----------------------------------------------------

/// folder-replaced-empty-with-shared-uidvalidity: `Receipts and Invoices share a UIDVALIDITY
/// other folders have too`.
pub fn empty_folder_uidvalidity_shared(old_path: &str, new_path: &str) -> String {
    format!(
        "- {old_path} and {new_path} share a UIDVALIDITY other folders have too, and {old_path} holds no messages to compare."
    )
}

/// uidvalidity-change-large-mailbox: `syncFolderUIDRange for INBOX, UIDs: 1 - \*, Heavy: false`.
/// `uids` is `"<lo> - <hi>"` or `"<lo> - *"` for an open-ended range.
pub fn sync_folder_uid_range(path: &str, uids: &str, heavy: bool) -> String {
    format!("syncFolderUIDRange for {path}, UIDs: {uids}, Heavy: {heavy}")
}

/// heavy-fetch-truncation-backlog, uidvalidity-change-large-mailbox:
/// `Fetching full headers for 1024 \(of \d+ needed\)`.
pub fn fetching_full_headers(count: usize, needed: usize) -> String {
    format!("- Fetching full headers for {count} (of {needed} needed)")
}

/// messagelimit-partial-fetch: `Archive: server applied MESSAGELIMIT, only UIDs \d+ and up
/// were scanned`.
pub fn message_limit_applied(path: &str, min_uid: u32) -> String {
    format!("- {path}: server applied MESSAGELIMIT, only UIDs {min_uid} and up were scanned")
}

/// folder-renamed-on-server: `Projects renamed to Clients/Projects \(same UIDVALIDITY\)`;
/// folder-renamed-in-tern: `\(recorded rename\)`.
pub fn folder_kept_across_rename(old_path: &str, new_path: &str, how: &str) -> String {
    format!("- {old_path} renamed to {new_path} ({how}), keeping its id")
}

/// folder-replaced-with-same-uidvalidity: `Receipts and Invoices share a UIDVALIDITY but not
/// their messages`.
pub fn folders_share_uidvalidity_only(old_path: &str, new_path: &str) -> String {
    format!("- {old_path} and {new_path} share a UIDVALIDITY but not their messages.")
}

/// uidvalidity-change-large-mailbox: `UIDInvalidity! Resetting placement UIDs in INBOX`.
pub fn uidvalidity_reset(path: &str) -> String {
    format!("UIDInvalidity! Resetting placement UIDs in {path}, rebuilding index. This may take a moment...")
}

/// orphan-sweep-with-unreadable-folder: `unable to get folder status for Shared`.
pub fn folder_status_failed(path: &str, error: &str) -> String {
    format!("SyncNow: unable to get folder status for {path} ({error}), skipping...")
}

/// orphan-sweep-with-unreadable-folder (counted once):
/// `Orphans older than 8s are swept without waiting for Shared`.
pub fn orphans_swept_without_waiting(
    max_wait: i64,
    path: &str,
    hours_since_covered: f64,
) -> String {
    format!(
        "Orphans older than {max_wait}s are swept without waiting for {path}, not fully scanned for {hours_since_covered:.1}h."
    )
}

/// Same as [`orphans_swept_without_waiting`], for a folder never fully scanned since launch.
/// orphan-sweep-waits-for-initial-walk asserts `swept without waiting for Big` is absent.
pub fn orphans_swept_without_waiting_since_launch(max_wait: i64, path: &str) -> String {
    format!(
        "Orphans older than {max_wait}s are swept without waiting for {path}, not fully scanned since launch."
    )
}

/// proton-all-mail-duplicates: `skipped duplicate`.
pub fn skipped_duplicate_all_folder(path: &str) -> String {
    format!("SyncNow: skipped duplicate \\All folder {path}")
}

/// modseq-truncation: `request limited to \d+:\d+, remaining changes will be detected via
/// deep scan` (also waited on as `request limited to`). `top_uid` is `u64::MAX` when STATUS
/// had no UIDNEXT, as the C++ printed it.
pub fn condstore_request_limited(bottom_uid: u32, top_uid: u64) -> String {
    format!(
        "syncFolderChangesViaCondstore - request limited to {bottom_uid}:{top_uid}, remaining changes will be detected via deep scan"
    )
}

// -- Mail processing (MailProcessor.cpp) ---------------------------------------------------

/// messagelimit-partial-fetch: `Orphan sweep limited to messages orphaned`.
pub fn orphan_sweep_limited(seconds_before_pass: i64) -> String {
    format!(
        "Orphan sweep limited to messages orphaned more than {seconds_before_pass}s before this pass: a folder was skipped, still in initial sync, or had a fetch truncated."
    )
}

/// orphan-sweep-waits-for-initial-walk: `Orphan sweep skipped`.
pub fn orphan_sweep_skipped() -> &'static str {
    "Orphan sweep skipped: a folder still in initial sync has not been fully scanned since launch."
}

/// mid-pass-foreground-tombstone asserts `has a copy in INBOX \(UID 20\)` is absent.
pub fn message_has_copy_in(message_id: &str, path: &str, uid: u32) -> String {
    format!("- Message {message_id} has a copy in {path} (UID {uid})")
}

/// rules-ready-gmail waits for `XGMLabels`. Arguments are the JSON-dumped label arrays.
pub fn xgm_labels_changed(before: &str, after: &str) -> String {
    format!("-- XGMLabels ({before} to {after})")
}

/// move-with-permuted-copyuid asserts `lost a placement to another message` is absent.
pub fn lost_placement(message_id: &str) -> String {
    format!("- Message {message_id} lost a placement to another message at the same UID")
}

// -- Foreground (SyncWorker.cpp) -----------------------------------------------------------

/// qresync-vanished-during-body-fetch (`Fetching body for message`) and rules-ready-incoming
/// (`Fetching body for message ID`) wait for this.
pub fn fetching_body(message_id: &str) -> String {
    format!("Fetching body for message ID {message_id}")
}

// -- Tasks (TaskProcessor.cpp) ---------------------------------------------------------------

/// move-rejected-by-server, star-rejected-on-condstore: `-X Changing copies in INBOX failed`.
pub fn changing_copies_failed(path: &str) -> String {
    format!("-X Changing copies in {path} failed; releasing the task's messages")
}

/// move-without-move-keeps-other-deleted, move-without-move-connection-drop assert
/// `Expunging all` is absent.
pub fn expunging_all_deleted(path: &str) -> String {
    format!("-- Expunging all \\Deleted messages in {path}")
}

/// trash-two-placements-without-uidplus asserts `Could not find new UID` is absent.
pub fn could_not_find_new_uid(message_id: &str, dest_path: &str) -> String {
    format!("-- Could not find new UID for message {message_id} moved to {dest_path}")
}

/// trash-two-placements-without-uidplus asserts `COPYUID` is absent.
pub fn copyuid_mispaired(dest_path: &str, unpaired: usize, total: usize) -> String {
    format!("-- COPYUID into {dest_path} paired {unpaired} of {total} copies with another message's UID")
}

/// send-draft asserts `Sleeping 3` is absent: a server that never files a Sent copy keeps
/// the short poll series.
pub fn sent_poll_sleeping(found: usize, expected: usize, delay: u64) -> String {
    format!("-- Found {found} of {expected} copies. Sleeping {delay} to wait for sent folder to settle...")
}

/// gmail-send-and-labels: `Found a message added to the sent folder by the SMTP gateway`.
pub fn found_sent_copy_from_gateway(uid: u32, copies: usize) -> String {
    format!("-- Found a message added to the sent folder by the SMTP gateway (UID {uid}, {copies} copies)")
}

/// sent-copy-saved-late counts `Placing a new message` once.
pub fn placing_new_sent_message() -> &'static str {
    "-- Placing a new message with `self` body in the sent folder."
}

/// gmail-send-and-labels: `Found the sent message in \[Gmail\]/All Mail`.
pub fn found_sent_message_in(path: &str, uid: u32) -> String {
    format!("-- Found the sent message in {path} (UID {uid})")
}

/// rules-ready-inbox-copy-first waits for `Syncing sent message`.
pub fn syncing_sent_message(path: &str, uid: u32) -> String {
    format!("-- Syncing sent message ({path} UID {uid}) to the local mail store")
}

/// sent-copy-cleanup-store-refused (2x), sent-copy-saved-late (1x):
/// `Removing 1 late server copies`.
pub fn removing_late_sent_copies(
    count: usize,
    message_id: &str,
    path: &str,
    keep_uid: u32,
) -> String {
    format!("- Removing {count} late server copies of sent message {message_id} from {path}, keeping UID {keep_uid}")
}

/// sent-copy-send-interrupted: `Removing the draft of a send interrupted after SMTP`.
pub fn removing_draft_of_interrupted_send(header_message_id: &str) -> String {
    format!("- Removing the draft of a send interrupted after SMTP: {header_message_id}")
}

/// expiration-send-offline: `Send deferred until the account is online`.
pub fn send_deferred_until_online(header_message_id: &str, error: &str) -> String {
    format!("-- Send deferred until the account is online: {header_message_id} ({error})")
}

// -- Process (main.cpp) ----------------------------------------------------------------------

/// The first line of every sync process's log.
pub fn starting_sync(email: &str) -> String {
    format!("------------- Starting Sync ({email}) ---------------")
}

/// Which metadata backend the sync process uses (docs/backend-plan.md 4.1). Never the token.
/// backend-metadata-push: `^Metadata backend: tern-server 127\.0\.0\.1`;
/// backend-identity-invalid: absent `^Metadata backend: tern-server`.
pub fn metadata_backend_tern(host: &str) -> String {
    format!("Metadata backend: tern-server {host}")
}

/// The local-only backend, and why (`no server`, or the refused field of a bad server object).
/// backend-identity-invalid: `^Metadata backend: local only \(invalid url\)`.
pub fn metadata_backend_local(reason: &str) -> String {
    format!("Metadata backend: local only ({reason})")
}

/// `--socket`: the socket is bound and accepting. apps/engine/tests/socket.rs waits for it.
pub fn socket_listening(path: &str) -> String {
    format!("Listening on socket {path}")
}

/// `--socket`: a subscriber fell behind past the queue bound and is disconnected.
/// apps/engine/tests/socket.rs `slow_subscriber_is_dropped` waits for
/// `^-X Dropping socket connection \d+: `.
pub fn socket_connection_dropped(conn: u64, reason: &str) -> String {
    format!("-X Dropping socket connection {conn}: {reason}")
}

/// Logged for `wake-workers`.
pub fn waking_all_workers() -> &'static str {
    "Waking all workers..."
}

/// `main.cpp:253`. The scenarios dav-initial-sync and google-people-initial-sync wait for
/// `Calendar sync pass 1, with contacts`.
pub fn calendar_sync_pass(pass: u64, with_contacts: bool) -> String {
    format!(
        "Calendar sync pass {pass}{}",
        if with_contacts { ", with contacts" } else { "" }
    )
}

/// The end of a calContacts pass whose workers all finished; every store write of the pass
/// is committed (its deltas may still be up to 500 ms behind). dav-create-update-delete
/// waits for `^Calendar sync pass 1 complete\.$` (anchored). Not logged by the C++ engine.
pub fn calendar_sync_pass_complete(pass: u64) -> String {
    format!("Calendar sync pass {pass} complete.")
}

/// The end of a calContacts pass that failed, after its error line and before any
/// suspension. google-people-sync-token-expired waits for `^-X Calendar sync pass 1 failed\.$`
/// (anchored): the expired sync token is cleared by then. Not logged by the C++ engine.
pub fn calendar_sync_pass_failed(pass: u64) -> String {
    format!("-X Calendar sync pass {pass} failed.")
}

/// A message with neither a Date header nor INTERNALDATE gets a location-based id.
pub fn message_without_date(folder_path: &str, uid: u32) -> String {
    format!("- Message {folder_path}:{uid} has no date; its id is based on its location")
}

// -- AI triage (apps/engine/src/ai_triage.rs) -------------------------------------------

/// A thread's triage result was stored. test_ai.py `ai_triage_new_mail` waits for
/// `^- AI triage stored for thread \S+$`. Never the category or the reason.
pub fn ai_triage_stored(thread_id: &str) -> String {
    format!("- AI triage stored for thread {thread_id}")
}

/// The provider answered `rate-limited` or `quota-exceeded`: the account's triage queue
/// waits. test_ai.py `ai_triage_rate_limit_pause` waits for
/// `^-X AI triage paused for \d+ s: rate-limited$`.
pub fn ai_triage_paused(secs: u64, key: &str) -> String {
    format!("-X AI triage paused for {secs} s: {key}")
}

/// The pause is over and the queue runs again. test_ai.py `ai_triage_rate_limit_pause`
/// waits for `^AI triage resumed$`.
pub fn ai_triage_resumed() -> &'static str {
    "AI triage resumed"
}

// -- Server-synced drafts (docs/drafts-sync-design.md) ------------------------------------

/// A draft row moved to the id its Message-ID gives it. draft-remote-edit-adopt and
/// import-legacy-drafts-pushed match `Draft \S+ is now \S+`.
pub fn draft_rekeyed(old_id: &str, new_id: &str) -> String {
    format!("- Draft {old_id} is now {new_id}")
}

/// A draft revision was APPENDed and its UID recorded. The draft-push-* scenarios count
/// `Saved draft \S+ to .+ UID \d+`.
pub fn draft_pushed(header_message_id: &str, path: &str, uid: u32) -> String {
    format!("- Saved draft {header_message_id} to {path} UID {uid}")
}

/// A push failed and backs off. The draft-push-* scenarios assert `Could not save draft`
/// absent.
pub fn draft_push_failed(header_message_id: &str, error: &str, retry_in: i64) -> String {
    format!(
        "-X Could not save draft {header_message_id} to the server ({error}); trying again in {retry_in}s"
    )
}

/// A revision that can't be built as a message (no From, an attachment that can't be
/// read) stays local until the next save. The draft-push-* scenarios assert `Could not save
/// draft` absent.
pub fn draft_push_held(header_message_id: &str, reason: &str) -> String {
    format!(
        "-X Could not save draft {header_message_id} to the server ({reason}); it stays local until it is saved again"
    )
}

/// A revision over the server's APPENDLIMIT is kept local until the next save.
/// draft-attachments waits for `\S+ is \d+ bytes, over the server's APPENDLIMIT`.
pub fn draft_over_append_limit(header_message_id: &str, size: usize, limit: u64) -> String {
    format!(
        "-X Draft {header_message_id} is {size} bytes, over the server's APPENDLIMIT of {limit}; it stays local"
    )
}

/// Another device saved a revision while this one had unsaved edits: ours wins and theirs
/// goes to Trash. draft-conflict-local-wins waits for `draft-conflict: \S+`.
pub fn draft_conflict(header_message_id: &str, copies: usize) -> String {
    format!(
        "-X draft-conflict: {header_message_id} changed here and elsewhere; moving {copies} other copies to Trash"
    )
}

/// A newer revision from another device or webmail replaced the local one.
/// draft-remote-edit-adopt waits for `Draft \S+ changed on the server`.
pub fn draft_adopted(header_message_id: &str, path: &str, uid: u32) -> String {
    format!(
        "- Draft {header_message_id} changed on the server ({path} UID {uid}); taking that revision"
    )
}

/// A server copy of a draft that was sent or deleted here is deleted instead of synced.
/// draft-send-before-scan and draft-destroy-after-push match `of sent or deleted draft`.
pub fn deleting_tombstoned_draft_copy(header_message_id: &str, path: &str, uid: u32) -> String {
    format!("- Deleting {path} UID {uid}, a copy of sent or deleted draft {header_message_id}")
}

/// Save drafts to server is on for the first time: the local drafts are queued to push.
/// import-legacy-drafts-pushed waits for `Saving \d+ local drafts to the server`.
pub fn draft_sync_enabled(drafts: usize) -> String {
    format!("Saving {drafts} local drafts to the server.")
}

// Patterns the scenarios assert are absent and that no engine line produces on purpose:
// `UNIQUE constraint` and `no such (table|column)` (SQLite error text that would reach the
// log through a failure), `Unsafe Login` (Coremail's refusal), `count is not falling` (no
// longer logged), `\[critical\]` (the level field of a critical line). The scenario
// migration-from-pre-placements-db waits for `Unlinking messages`, but only from the old
// C++ binary it starts on.
