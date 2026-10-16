//! Log lines that something outside the engine matches on.
//!
//! The harness (`apps/engine/test`) waits for and counts these with regexes, and the client
//! reads a few. Each function names the scenario or harness code that depends on it; change
//! the wording only together with that consumer. `tests/logmsg.rs` checks every function
//! against the regexes copied from those files.
//!
//! Formatting follows the C++ engine: `{}` of a bool prints `true` / `false`, paths are
//! printed as stored (modified UTF-7), and sub-steps are prefixed `"- "`, `"-- "` or `"-X "`
//! (repo CLAUDE.md "Logging").

// -- Harness core (test/harness/mailsync.py) ------------------------------------------------

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

/// `<direction> <line>` for protocol traffic: `IMAP_LINE` in mailsync.py splits on it, the
/// foreground idle check matches `sent` lines against `^\d+ IDLE$`, and scenarios match
/// `recv \* VANISHED` (qresync-bulk-expunge-during-idle, qresync-wide-vanished-range,
/// orphan-sweep-with-unreadable-folder) and `sent \d+ APPEND` (gmail-send-and-labels).
pub fn protocol_line(direction: &str, line: &str) -> String {
    format!("{direction} {line}")
}

// -- Background sync (SyncWorker.cpp) ----------------------------------------------------

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

// -- Process (main.cpp) ----------------------------------------------------------------------

/// The first line of every sync process's log.
pub fn starting_sync(email: &str) -> String {
    format!("------------- Starting Sync ({email}) ---------------")
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

/// A message with neither a Date header nor INTERNALDATE gets a location-based id
/// (docs/engine-port-plan.md section 5).
pub fn message_without_date(folder_path: &str, uid: u32) -> String {
    format!("- Message {folder_path}:{uid} has no date; its id is based on its location")
}

// Patterns the scenarios assert are absent and that no engine line produces on purpose:
// `UNIQUE constraint` and `no such (table|column)` (SQLite error text that would reach the
// log through a failure), `Unsafe Login` (Coremail's refusal), `count is not falling` (no
// longer logged), `\[critical\]` (the level field of a critical line). The scenario
// migration-from-pre-placements-db waits for `Unlinking messages`, but only from the old
// C++ binary it starts on.
