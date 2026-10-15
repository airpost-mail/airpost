//! The `background` thread: folder listing, scan passes and the orphan sweep, then it
//! starts `foreground` (`runBackgroundSyncWorker`, `SyncWorker::syncNow`,
//! `SyncWorker::syncFoldersAndLabels`).
//!
//! Derived from Mailspring-Sync `MailSync/SyncWorker.cpp` and the worker loop in
//! `MailSync/main.cpp`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Duration;

use ms_core::constants::{
    ls, orphan_sweep_max_wait, thread, CACHE_CLEANUP_INTERVAL, CONDSTORE_GAP_SCAN_INTERVAL,
    DEEP_SCAN_INTERVAL, FIRST_SYNC_CHUNK_SIZE, FOLDER_SYNC_ROLE_ORDER, MAILSPRING_FOLDER_PREFIX_V2,
    MAX_FULL_HEADERS_REQUEST_SIZE, MAX_SINGLE_RANGE_SWEEP_SIZE, NEW_MAIL_BODY_FETCH_LIMIT,
    SHALLOW_SCAN_DEPTH, SHALLOW_SCAN_INTERVAL, WORKER_SLEEP,
};
use ms_core::wake::sleep_worker_until_wake_or;
use ms_core::{logmsg, Error, ErrorCode, FolderFlags, Result, SyncError};
use ms_imap::{FolderStatus, ImapSession, ListedFolder, Namespace};
use ms_providers::roles::{role_for_folder, role_for_folder_flags, role_for_folder_path, ROLES};
use ms_store::{Folder, Label, Message, Model, Query};
use serde_json::Value;
use tracing::{error, info, warn};

use crate::foreground::run_foreground;
use crate::mail_processor::UnplacedMessages;
use crate::mailbox_model;
use crate::worker::{ls_i64, ls_u32, set_ls, Range, SyncWorker};
use crate::WorkerContext;

/// The thread body: waits out the account's start delay, then lists folders, starts the
/// foreground thread and runs passes until there is nothing left to do, sleeping between
/// rounds until the interval passes or `wake-workers` arrives.
pub fn run_background(ctx: WorkerContext) {
    // Staggers many accounts' first database opens.
    sleep_worker_until_wake_or(ctx.account.start_delay());
    let mut bg = match Background::new(ctx.clone()) {
        Ok(b) => b,
        Err(e) => crate::abort_worker(&e),
    };
    let mut started = false;
    loop {
        match bg.run_once(&mut started) {
            Ok(()) => {}
            Err(e) => crate::report_worker_error(&ctx, &e),
        }
        sleep_worker_until_wake_or(WORKER_SLEEP);
    }
}

/// A Folder or a Label in the listing (`syncFoldersAndLabels` keeps both as `Folder *`).
enum Category {
    Folder(Folder),
    Label(Label),
}

impl Category {
    fn path(&self) -> &str {
        match self {
            Category::Folder(f) => f.path(),
            Category::Label(l) => l.path(),
        }
    }

    fn role(&self) -> &str {
        match self {
            Category::Folder(f) => f.role(),
            Category::Label(l) => l.role(),
        }
    }

    fn set_role(&mut self, role: &str) {
        match self {
            Category::Folder(f) => f.set_role(role),
            Category::Label(l) => l.set_role(role),
        }
    }

    fn version(&self) -> i64 {
        match self {
            Category::Folder(f) => f.version(),
            Category::Label(l) => l.version(),
        }
    }

    fn table(&self) -> &'static str {
        match self {
            Category::Folder(_) => Folder::TABLE,
            Category::Label(_) => Label::TABLE,
        }
    }
}

struct Background {
    w: SyncWorker,
    iterations_since_launch: u64,
    /// Per folder, the start of the last pass whose scan covered its whole range, which
    /// bounds how old an orphan must be before the sweep removes it. Not persisted: after
    /// a relaunch a folder counts as never covered, which only makes the sweep wait longer.
    folder_covered_at: HashMap<String, i64>,
    folders_past_orphan_wait: HashSet<String>,
    foreground_started: bool,
}

/// What one pass knows about the orphan sweep bound (`syncNow`'s `recordCoverage`).
struct SweepBound {
    pass_started_at: i64,
    wait_limit: i64,
    max_wait: i64,
    sweep_before: i64,
}

impl Background {
    fn new(ctx: WorkerContext) -> Result<Self> {
        Ok(Background {
            w: SyncWorker::new(ctx)?,
            iterations_since_launch: 0,
            folder_covered_at: HashMap::new(),
            folders_past_orphan_wait: HashSet::new(),
            foreground_started: false,
        })
    }

    fn ctx(&self) -> &WorkerContext {
        &self.w.ctx
    }

    fn run_once(&mut self, started: &mut bool) -> Result<()> {
        self.w.configure()?;
        // Existing folders show as syncing until the first pass gets through them.
        let mark_all = self.ctx().background_mark_all.swap(false, Ordering::SeqCst);
        if !*started || mark_all {
            self.mark_all_folders_busy()?;
        }
        if !*started {
            self.sync_folders_and_labels()?;
            self.ctx()
                .deltas
                .end_connection_error(self.ctx().account.id());
            // The folder list exists now, so the foreground worker has an inbox to idle on.
            self.start_foreground();
            *started = true;
        }
        // A hard loop until a pass reports nothing more to do; the interval between rounds
        // can be long because the foreground thread is idling.
        while self.sync_now()? {}
        self.ctx()
            .deltas
            .end_connection_error(self.ctx().account.id());
        Ok(())
    }

    fn start_foreground(&mut self) {
        if self.foreground_started {
            return;
        }
        let ctx = self.ctx().clone();
        if let Err(e) = std::thread::Builder::new()
            .name(thread::FOREGROUND.into())
            .spawn(move || run_foreground(ctx))
        {
            ms_core::critical!("Could not start the foreground thread: {e}");
            ms_core::log::flush();
            std::process::abort();
        }
        self.foreground_started = true;
    }

    fn mark_all_folders_busy(&mut self) -> Result<()> {
        info!("{}", logmsg::marking_all_folders_busy());
        let account_id = self.w.account_id().to_owned();
        self.w.store.transaction("markAllFoldersBusy", |s| {
            for mut f in s.find_all::<Folder>(&Query::new().equal("accountId", &account_id))? {
                f.local_status_mut()
                    .insert(ls::BUSY.into(), Value::Bool(true));
                s.save(&mut f)?;
            }
            Ok(())
        })
    }

    /// Every `localStatus` key "fully synced and idle" implies, for a folder the worker
    /// deliberately never scans (`markFolderStatusSynced`). Not for the end of a real pass,
    /// where `busy` reflects work left behind.
    fn mark_folder_status_synced(&self, folder: &mut Folder, remote: &FolderStatus) {
        let now = self.w.now();
        set_ls(folder, ls::LAST_SHALLOW, now);
        set_ls(folder, ls::LAST_DEEP, now);
        set_ls(folder, ls::BODIES_WANTED, 0);
        set_ls(folder, ls::SYNCED_MIN_UID, 1);
        set_ls(folder, ls::UIDNEXT, remote.uid_next);
        set_ls(folder, ls::MESSAGE_COUNT, remote.message_count);
        set_ls(folder, ls::UNSEEN_COUNT, remote.unseen_count);
        set_ls(folder, ls::RECENT_COUNT, remote.recent_count);
        // Busy was set for every folder at launch and on wake-workers; a folder that never
        // reaches the end-of-pass bookkeeping has to clear it here.
        set_ls(folder, ls::BUSY, false);
    }

    /// A folder the pass did not cover in full may hold a copy the sweep would otherwise
    /// treat as gone, so the sweep takes only orphans older than that folder's last full
    /// coverage, but never waits more than `ORPHAN_SWEEP_MAX_WAIT` for it unless its
    /// initial walk moved down this pass: a walk through a very large folder can take
    /// days, and it will reach every copy.
    fn record_coverage(
        &mut self,
        bound: &mut SweepBound,
        folder: &Folder,
        covered: bool,
        walk_progressed: bool,
    ) {
        if covered {
            self.folder_covered_at
                .insert(folder.id().to_owned(), bound.pass_started_at);
            self.folders_past_orphan_wait.remove(folder.id());
            return;
        }
        let covered_at = self
            .folder_covered_at
            .get(folder.id())
            .copied()
            .unwrap_or(0);
        if walk_progressed {
            bound.sweep_before = bound.sweep_before.min(covered_at);
            return;
        }
        if covered_at < bound.wait_limit
            && self.folders_past_orphan_wait.insert(folder.id().to_owned())
        {
            if covered_at != 0 {
                warn!(
                    "{}",
                    logmsg::orphans_swept_without_waiting(
                        bound.max_wait,
                        folder.path(),
                        (bound.pass_started_at - covered_at) as f64 / 3600.0
                    )
                );
            } else {
                warn!(
                    "{}",
                    logmsg::orphans_swept_without_waiting_since_launch(
                        bound.max_wait,
                        folder.path()
                    )
                );
            }
        }
        bound.sweep_before = bound.sweep_before.min(covered_at.max(bound.wait_limit));
    }

    /// One pass over every folder (`syncNow`). Returns true when there is more to do
    /// straight away.
    fn sync_now(&mut self) -> Result<bool> {
        let mut sync_again_immediately = false;

        // A message orphaned before this instant has had every folder this pass covers in
        // full scanned since. Taken before any scan so the foreground worker's orphans get
        // the same grace, and one clock serves every folder.
        let pass_started_at = self.w.now();
        let max_wait = orphan_sweep_max_wait();
        let mut bound = SweepBound {
            pass_started_at,
            wait_limit: pass_started_at - max_wait,
            max_wait,
            sweep_before: pass_started_at,
        };

        let mut folders = self.sync_folders_and_labels()?;
        let has_condstore = self.w.has_capability("CONDSTORE");
        let has_qresync = self.w.has_capability("QRESYNC");
        let is_gmail = self.w.is_gmail();
        let profile = self.w.session.profile().clone();

        // STATUS of the folder this connection left selected on the last pass would report
        // that pass's counts, and nothing in a quiet pass refreshes the view, so new mail
        // there would go unnoticed. QRESYNC needs this too: VANISHED reports expunges, not
        // appends.
        if self.w.session.current_folder().is_some() && profile.noop_before_selected_status {
            self.w.noop_selected_folder()?;
        }

        // On Gmail only All Mail, Spam and Trash are folders here; labels come from
        // X-GM-LABELS.
        let rank = |f: &Folder| {
            FOLDER_SYNC_ROLE_ORDER
                .iter()
                .position(|r| *r == f.role())
                .unwrap_or(FOLDER_SYNC_ROLE_ORDER.len())
        };
        folders.sort_by_key(|f| rank(f));

        let container = self.ctx().account.container_folder().to_owned();
        for folder in folders.iter_mut() {
            let path = folder.path().to_owned();
            let remote = match self.w.session.status(&path) {
                Ok(s) => s,
                Err(e) => {
                    warn!("{}", logmsg::folder_status_failed(&path, e.code.name()));
                    // Busy was set by markAllFoldersBusy and nothing below will clear it.
                    let initial = folder.local_status().clone();
                    set_ls(folder, ls::BUSY, false);
                    self.w.store.save_folder_status(folder, &initial)?;
                    self.record_coverage(&mut bound, folder, false, false);
                    continue;
                }
            };
            let initial = folder.local_status().clone();
            let now = self.w.now();
            let mut first_chunk = false;
            let mut deep_scan_incomplete = false;

            // Coremail omits UIDNEXT from STATUS, and hMailServer and home.pl report 0
            // (Geary imap-status-data.vala), so new mail cannot be detected from it. The
            // counts are the only signal, and a change in any of them deep-scans the folder.
            let mut counts_changed = false;
            if remote.uid_next == 0 {
                let changed = |key: &str, value: u32| {
                    ls_i64(folder.local_status(), key) != Some(value as i64)
                };
                counts_changed = changed(ls::MESSAGE_COUNT, remote.message_count)
                    || changed(ls::UNSEEN_COUNT, remote.unseen_count)
                    || changed(ls::RECENT_COUNT, remote.recent_count);
            }

            // Step 1: a folder seen for the first time. Its HIGHESTMODSEQ is taken now and
            // the walk starts from UIDNEXT, so once the walk reaches UID 1 CONDSTORE picks up
            // from the oldest block's modseq and no change is missed.
            let status_empty = folder
                .local_status()
                .as_object()
                .is_none_or(|o| o.is_empty());
            if status_empty
                || folder
                    .local_status()
                    .get(ls::UIDVALIDITY)
                    .is_none_or(Value::is_null)
            {
                set_ls(folder, ls::HIGHESTMODSEQ, remote.highest_modseq);
                set_ls(folder, ls::UIDVALIDITY, remote.uid_validity);
                set_ls(folder, ls::UIDVALIDITY_RESET_COUNT, 0);
                set_ls(folder, ls::UIDNEXT, remote.uid_next);
                set_ls(folder, ls::SYNCED_MIN_UID, remote.uid_next);
                set_ls(folder, ls::LAST_SHALLOW, 0);
                set_ls(folder, ls::LAST_DEEP, 0);
                first_chunk = true;
            }

            // Step 1.5: an \All mailbox is a duplicate view of mail that also lives in Inbox,
            // Sent and Archive (ProtonMail Bridge). Syncing it would double header traffic,
            // file every thread under Archive, and a delete from All Mail on Bridge is a
            // delete everywhere, so it is skipped but kept for the client to archive into
            // (Mailspring-Sync PR #137). Gmail is the exception: All Mail is its primary
            // store. The container-folder clause keeps a Bridge without SPECIAL-USE, whose
            // "All Mail" has no role, behaving as before.
            let duplicate_all_mail =
                (profile.skip_all_mail_folder && !is_gmail && folder.role() == "all")
                    || (!container.is_empty() && path == "All Mail");
            if duplicate_all_mail {
                info!("{}", logmsg::skipped_duplicate_all_folder(&path));
                self.mark_folder_status_synced(folder, &remote);
                self.w.store.save_folder_status(folder, &initial)?;
                self.record_coverage(&mut bound, folder, true, false);
                continue;
            }

            if ls_u32(folder.local_status(), ls::UIDVALIDITY) != remote.uid_validity {
                // The folder's UIDs mean nothing now. Every placement goes to UID 0 (still
                // visible, no deltas) and a full scan re-fetches the headers, hashes them to
                // the same ids and assigns the new UIDs through the ordinary upsert. Only a
                // scan of the whole folder can call a row still at UID 0 gone; a truncated
                // rebuild leaves its tail for the initial-sync walk, which deletes the
                // leftovers when it reaches UID 1.
                warn!("{}", logmsg::uidvalidity_reset(&path));
                self.w.store.reset_placement_uids(folder)?;
                let rebuilt = self
                    .w
                    .sync_folder_uid_range(folder, Range::open(1), false, None)?;
                if !rebuilt.truncated {
                    self.w.processor().delete_unassigned_placements(folder)?;
                }
                // A folder without the key counts as reset once already, as in the C++.
                let resets =
                    ls_i64(folder.local_status(), ls::UIDVALIDITY_RESET_COUNT).unwrap_or(1);
                set_ls(folder, ls::UIDVALIDITY_RESET_COUNT, resets + 1);
                set_ls(folder, ls::HIGHESTMODSEQ, remote.highest_modseq);
                set_ls(folder, ls::UIDVALIDITY, remote.uid_validity);
                set_ls(folder, ls::UIDNEXT, remote.uid_next);
                // A rebuild that could not re-map everything leaves syncedMinUID above the
                // remainder so the initial-sync walk picks it up.
                set_ls(
                    folder,
                    ls::SYNCED_MIN_UID,
                    if rebuilt.truncated {
                        rebuilt.synced_min_uid
                    } else {
                        1
                    },
                );
                let now = self.w.now();
                set_ls(folder, ls::LAST_SHALLOW, now);
                set_ls(folder, ls::LAST_DEEP, now);
                set_ls(folder, ls::MESSAGE_COUNT, remote.message_count);
                set_ls(folder, ls::UNSEEN_COUNT, remote.unseen_count);
                set_ls(folder, ls::RECENT_COUNT, remote.recent_count);
                // This branch skips the end-of-loop bookkeeping that clears `busy`. A rebuild
                // that left a remainder asks for another pass at once, or the rest would
                // drain one chunk per sleep interval with the folder half unlinked.
                set_ls(folder, ls::BUSY, rebuilt.truncated);
                if rebuilt.truncated {
                    sync_again_immediately = true;
                }
                self.w.store.save_folder_status(folder, &initial)?;
                self.record_coverage(&mut bound, folder, !rebuilt.truncated, false);
                continue;
            }

            // Step 2: initial sync. Until UID 1 is reached, take chunks from the top down.
            // The chunk is one full-headers request; a larger one would be safe, since only
            // what was ingested is recorded, but would cost an extra attributes-only fetch.
            let mut synced_min_uid = ls_u32(folder.local_status(), ls::SYNCED_MIN_UID);
            let chunk_size = if first_chunk {
                FIRST_SYNC_CHUNK_SIZE
            } else {
                MAX_FULL_HEADERS_REQUEST_SIZE
            };
            let walk_start_uid = synced_min_uid;
            if synced_min_uid > 1 {
                // The UID space is sparse: a folder with UIDNEXT 100000 and 100 messages is
                // fetched in one range, not walked through mostly empty chunks.
                let mut chunk_min_uid = synced_min_uid.saturating_sub(chunk_size).max(1);
                if remote.message_count < MAX_SINGLE_RANGE_SWEEP_SIZE {
                    chunk_min_uid = 1;
                }
                let chunk = self.w.sync_folder_uid_range(
                    folder,
                    Range::new(chunk_min_uid, (synced_min_uid - chunk_min_uid) as u64),
                    true,
                    None,
                )?;
                // Only what was ingested is recorded: advancing past the remainder would drop
                // it for good, because CONDSTORE reports only changes since our modseq.
                if chunk.truncated
                    && chunk.synced_min_uid > chunk_min_uid
                    && chunk.synced_min_uid < synced_min_uid
                {
                    chunk_min_uid = chunk.synced_min_uid;
                }
                set_ls(folder, ls::SYNCED_MIN_UID, chunk_min_uid);
                synced_min_uid = chunk_min_uid;
                if chunk_min_uid <= 1 {
                    // The walk covered the whole folder: a completed deep pass, or a new
                    // account follows initial sync with a redundant full scan.
                    set_ls(folder, ls::LAST_DEEP, self.w.now());
                    // Every UID has been fetched once, so a placement still at UID 0 (the
                    // tail of a truncated UIDVALIDITY rebuild) is a copy the server lost.
                    self.w.processor().delete_unassigned_placements(folder)?;
                }
            }

            // A folder still walking down has copies this pass has not recorded, so the
            // sweep cannot tell one that moved into it from one the server dropped.
            let mut covered = synced_min_uid <= 1;

            // Step 3: new messages, changed messages, deleted messages. CONDSTORE does the
            // first two, QRESYNC the third.
            if has_condstore && has_qresync {
                self.w
                    .sync_folder_changes_via_condstore(folder, &remote, true)?;

                // CONDSTORE reports only messages whose modseq changed, so one never
                // ingested during initial sync stays missing. Once the walk has reached
                // UID 1, an occasional attributes-only pass over the folder repairs gaps.
                let last_deep = ls_i64(folder.local_status(), ls::LAST_DEEP).unwrap_or(0);
                if synced_min_uid <= 1
                    && self.iterations_since_launch > 0
                    && self.w.now() - last_deep > CONDSTORE_GAP_SCAN_INTERVAL
                {
                    // Note: a failure here must not abort the folder loop; this is a check
                    // on top of an otherwise complete sync.
                    match self
                        .w
                        .sync_folder_uid_range(folder, Range::open(1), false, None)
                    {
                        Ok(deep) => {
                            covered = covered && !deep.truncated;
                            // A backlog larger than one pass leaves lastDeep alone so the next
                            // pass comes straight back, while the backlog is shrinking.
                            if self.w.should_retry_truncated_scan(folder, &deep) {
                                deep_scan_incomplete = true;
                            } else {
                                set_ls(folder, ls::LAST_DEEP, self.w.now());
                            }
                        }
                        Err(Error::Sync(e)) => {
                            warn!(
                                "- {}: gap scan failed ({}), will retry after the normal interval.",
                                path,
                                e.to_json()
                            );
                            set_ls(folder, ls::LAST_DEEP, self.w.now());
                            covered = false;
                        }
                        Err(e) => return Err(e),
                    }
                }
            } else {
                let remote_uidnext = remote.uid_next;
                let local_uidnext = ls_u32(folder.local_status(), ls::UIDNEXT);
                let new_messages = remote_uidnext > local_uidnext;
                let last_deep = ls_i64(folder.local_status(), ls::LAST_DEEP).unwrap_or(0);
                let last_shallow = ls_i64(folder.local_status(), ls::LAST_SHALLOW).unwrap_or(0);
                let time_for_deep_scan = counts_changed
                    || (self.iterations_since_launch > 0 && now - last_deep > DEEP_SCAN_INTERVAL);
                let time_for_shallow_scan =
                    !time_for_deep_scan && now - last_shallow > SHALLOW_SCAN_INTERVAL;

                // New mail gets a heavy fetch and its bodies at once, so it shows quickly. A
                // huge range is truncated and the deep scan picks up the rest.
                if new_messages {
                    let mut synced: Vec<(Message, u32)> = Vec::new();
                    let fetched = self.w.sync_folder_uid_range(
                        folder,
                        Range::new(local_uidnext, (remote_uidnext - local_uidnext) as u64),
                        true,
                        Some(&mut synced),
                    )?;
                    // A truncated fetch leaves uidnext so the next pass comes back for the
                    // rest; otherwise the range would be fetched again every pass, and on a
                    // server without QRESYNC that repeat FETCH resurrects a copy the
                    // foreground connection has just moved out.
                    if !fetched.truncated {
                        set_ls(folder, ls::UIDNEXT, remote_uidnext);
                    } else {
                        covered = false;
                    }
                    if matches!(folder.role(), "inbox" | "all") {
                        synced.sort_by(|a, b| b.1.cmp(&a.1));
                        let mut fetched_bodies = 0;
                        for (mut m, _) in synced {
                            // All Mail that is not in the inbox waits for the body pass.
                            if !m.is_in_inbox(&self.w.store)? {
                                continue;
                            }
                            self.w.sync_message_body(&mut m, Some(folder))?;
                            fetched_bodies += 1;
                            if fetched_bodies >= NEW_MAIL_BODY_FETCH_LIMIT {
                                break;
                            }
                        }
                    }
                }

                if time_for_shallow_scan {
                    // From the local uidnext: everything above it was just fetched.
                    let mut bottom_uid = self.w.store.fetch_message_uid_at_depth(
                        folder,
                        SHALLOW_SCAN_DEPTH,
                        local_uidnext,
                    )?;
                    if bottom_uid < synced_min_uid {
                        bottom_uid = synced_min_uid;
                    }
                    if remote_uidnext > bottom_uid {
                        let shallow = self.w.sync_folder_uid_range(
                            folder,
                            Range::new(bottom_uid, (remote_uidnext - bottom_uid) as u64),
                            false,
                            None,
                        )?;
                        covered = covered && !shallow.truncated;
                    }
                    set_ls(folder, ls::LAST_SHALLOW, self.w.now());
                    set_ls(folder, ls::UIDNEXT, remote_uidnext);
                }

                if time_for_deep_scan {
                    let deep = self.w.sync_folder_uid_range(
                        folder,
                        Range::open(synced_min_uid),
                        false,
                        None,
                    )?;
                    covered = covered && !deep.truncated;
                    if synced_min_uid == 0 {
                        synced_min_uid = 1;
                        set_ls(folder, ls::SYNCED_MIN_UID, 1);
                    }
                    set_ls(folder, ls::LAST_SHALLOW, self.w.now());
                    set_ls(folder, ls::UIDNEXT, remote_uidnext);
                    // Only a deep scan that got through the backlog is done; otherwise come
                    // back next pass instead of waiting DEEP_SCAN_INTERVAL per batch.
                    if self.w.should_retry_truncated_scan(folder, &deep) {
                        deep_scan_incomplete = true;
                    } else {
                        set_ls(folder, ls::LAST_DEEP, self.w.now());
                    }
                }
            }

            set_ls(folder, ls::MESSAGE_COUNT, remote.message_count);
            set_ls(folder, ls::UNSEEN_COUNT, remote.unseen_count);
            set_ls(folder, ls::RECENT_COUNT, remote.recent_count);

            // Some bodies alongside the header walk, so snippets show early.
            let mut more_to_do = self.w.sync_message_bodies(folder)?;
            if synced_min_uid > 1 || deep_scan_incomplete {
                more_to_do = true;
            }

            // The cache queries are expensive, so the stats are recomputed hourly and
            // incremented blindly in between. Note: <= 1, since syncedMinUID is seeded from
            // UIDNEXT, which some servers report as 0, and on a CONDSTORE folder only the
            // plain deep scan normalises it.
            let last_cleanup = ls_i64(folder.local_status(), ls::LAST_CLEANUP).unwrap_or(0);
            if synced_min_uid <= 1 && self.w.now() - last_cleanup > CACHE_CLEANUP_INTERVAL {
                self.w.clean_message_cache(folder)?;
                set_ls(folder, ls::LAST_CLEANUP, self.w.now());
            }

            set_ls(folder, ls::BUSY, more_to_do);
            sync_again_immediately = sync_again_immediately || more_to_do;

            // Only the keys this pass changed are written, and nothing when nothing changed:
            // every Folder save is a delta the client re-renders.
            self.w.store.save_folder_status(folder, &initial)?;
            self.record_coverage(&mut bound, folder, covered, synced_min_uid < walk_start_uid);
        }

        if profile.late_sent_copies {
            for folder in folders.iter().filter(|f| f.role() == "sent") {
                let SyncWorker {
                    ctx,
                    store,
                    session,
                    ..
                } = &mut self.w;
                let mut runner = ctx.tasks.remote_runner(store, ctx);
                runner.remove_late_sent_copies(folder, session)?;
            }
        }

        // An orphan whose copy reappeared in a scanned folder had its record cleared by the
        // upsert; the rest that are old enough go. Without the per-folder bound, a bulk move
        // larger than one fetch would have its remainder removed here and re-created,
        // without metadata, once the destination caught up.
        self.w
            .processor()
            .sweep_expired_orphans(bound.sweep_before, pass_started_at)?;

        info!("{}", logmsg::sync_loop_complete());
        self.iterations_since_launch += 1;
        Ok(sync_again_immediately)
    }

    /// The account's container components: `Mailspring`, or the custom container folder
    /// split on `/`.
    fn container_components(&self) -> Vec<String> {
        let container = self.ctx().account.container_folder();
        if container.is_empty() || container == MAILSPRING_FOLDER_PREFIX_V2 {
            info!("Syncing folder list...");
            vec![MAILSPRING_FOLDER_PREFIX_V2.to_owned()]
        } else {
            info!("Syncing folder list on custom container folder {container} ...");
            container.split('/').map(str::to_owned).collect()
        }
    }

    fn namespace(&self) -> Namespace {
        self.w
            .session
            .default_namespace()
            .cloned()
            .unwrap_or_default()
    }

    /// Creates the container folder when it is not listed (`ensureRootMailspringFolder`).
    fn ensure_root_folder(&mut self, components: &[String], remote: &[ListedFolder]) {
        let parts: Vec<&str> = components.iter().map(String::as_str).collect();
        let desired = self.namespace().path_for_components(&parts);
        if remote.iter().any(|r| r.path == desired) {
            return;
        }
        match self.w.session.create_folder(&desired) {
            Ok(()) => info!("Created container folder: {desired}."),
            Err(e) => error!(
                "Could not create container folder: {desired}. {}",
                e.code.name()
            ),
        }
    }

    /// Some servers, IMAP gateways especially (DavMail, Proton Bridge, Zoho), list a
    /// mailbox twice: a repeated LIST line, or "Inbox" next to "INBOX". Both hash to one
    /// folder id, and the second insert used to abort on every launch (Mailspring-Sync PR
    /// #139). Keeps the first copy with the flags of all of them, so role detection does
    /// not depend on the order the server used (`removeDuplicateFolders`).
    fn remove_duplicate_folders(&self, remote: &mut Vec<ListedFolder>) {
        let account_id = self.w.account_id().to_owned();
        let mut first_by_id: HashMap<String, usize> = HashMap::new();
        let mut kept: Vec<ListedFolder> = Vec::with_capacity(remote.len());
        for r in remote.drain(..) {
            let id = ms_core::ids::folder_id(&account_id, &r.path);
            match first_by_id.get(&id) {
                Some(&i) => {
                    warn!(
                        "-X the server listed {} more than once - ignoring the duplicate.",
                        r.path
                    );
                    kept[i].flags |= r.flags;
                }
                None => {
                    first_by_id.insert(id, kept.len());
                    kept.push(r);
                }
            }
        }
        *remote = kept;
    }

    /// Lists the server's folders, creates the helper folders, assigns roles and brings
    /// the local Folder and Label rows in line (`syncFoldersAndLabels`). Returns the
    /// folders to scan.
    fn sync_folders_and_labels(&mut self) -> Result<Vec<Folder>> {
        let account = self.ctx().account.clone();
        let account_id = account.id().to_owned();
        let container = account.container_folder().to_owned();
        let components = self.container_components();

        let mut remote = match self.w.session.list_folders() {
            Ok(r) => r,
            // A NO to LIST names no folder. O365 answers "NO Server Unavailable. 15" then BYE
            // during transient outages (observed 2026-09-24).
            Err(e)
                if e.code == ErrorCode::NonExistantFolder
                    && self.w.session.profile().list_no_is_connection_error =>
            {
                return Err(SyncError::from_code(
                    ErrorCode::Connection,
                    "syncFoldersAndLabels - fetchAllFolders refused",
                )
                .into());
            }
            Err(e) => return Err(e.into_sync("syncFoldersAndLabels - fetchAllFolders").into()),
        };
        let namespace = self.namespace();
        let main_prefix = namespace.prefix.clone();

        // Accounts with createHelperFolders false (O365 shared mailboxes, where any folder
        // we create is visible to every member) go without; snooze is unavailable there.
        let mut ensured_root = false;
        if account.create_helper_folders() {
            for helper in ["Snoozed"] {
                let role = helper.to_lowercase();
                let exists = remote.iter().any(|r| {
                    role_for_folder(&container, &main_prefix, &r.path, r.delimiter, r.flags)
                        == Some(role.as_str())
                });
                if exists {
                    continue;
                }
                if !ensured_root {
                    self.ensure_root_folder(&components, &remote);
                    ensured_root = true;
                }
                let mut parts: Vec<&str> = components.iter().map(String::as_str).collect();
                parts.push(helper);
                let desired = namespace.path_for_components(&parts);
                if let Err(e) = self.w.session.create_folder(&desired) {
                    error!(
                        "Could not create required folder: {desired}. {}",
                        e.code.name()
                    );
                    continue;
                }
                info!("Created required folder: {desired}.");
                remote.push(ListedFolder {
                    path: desired,
                    delimiter: namespace.delimiter,
                    flags: FolderFlags::empty(),
                });
            }
        }

        remote.retain(|r| !r.flags.contains(FolderFlags::NO_SELECT));
        if self.w.session.profile().merge_duplicate_list_entries {
            self.remove_duplicate_folders(&mut remote);
        }

        // Read local state and compute the diff outside the transaction, to keep the write
        // lock on the shared database short.
        let q = Query::new().equal("accountId", &account_id);
        let mut unused_folders = self.w.store.find_all_map::<Folder>(&q, "id")?;
        let mut unused_labels = self.w.store.find_all_map::<Label>(&q, "id")?;
        let model = mailbox_model::for_profile(self.w.session.profile().mailbox_model, &account_id);
        let (_, labels) = model.classify_folders(&remote);
        let label_ids: HashSet<String> = labels.iter().map(|l| l.id().to_owned()).collect();

        let mut categories: Vec<Category> = Vec::with_capacity(remote.len());
        let mut created: Vec<usize> = Vec::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        for r in &remote {
            let id = ms_core::ids::folder_id(&account_id, &r.path);
            let category = if label_ids.contains(&id) {
                match unused_labels.remove(&id) {
                    Some(l) => Category::Label(l),
                    None => {
                        let mut l = Label::new(&id, &account_id, 0);
                        l.set_path(&r.path);
                        created.push(categories.len());
                        Category::Label(l)
                    }
                }
            } else {
                match unused_folders.remove(&id) {
                    Some(f) => Category::Folder(f),
                    None => {
                        let mut f = Folder::new(&id, &account_id, 0);
                        f.set_path(&r.path);
                        created.push(categories.len());
                        Category::Folder(f)
                    }
                }
            };
            by_id.insert(id, categories.len());
            categories.push(category);
        }

        // Roles: for each role in order, a folder already holding it wins, then a folder
        // whose flags say it, then one whose name does.
        let mut role_updates: Vec<(String, &'static str)> = Vec::new();
        for role in ROLES {
            if categories.iter().any(|c| c.role() == role) {
                continue;
            }
            let by_flags = |r: &ListedFolder| role_for_folder_flags(r.flags) == Some(role);
            let by_path = |r: &ListedFolder| {
                role_for_folder_path(&container, &main_prefix, &r.path, r.delimiter) == Some(role)
            };
            for matcher in [&by_flags as &dyn Fn(&ListedFolder) -> bool, &by_path] {
                let mut found = false;
                for r in remote.iter().filter(|r| matcher(r)) {
                    let id = ms_core::ids::folder_id(&account_id, &r.path);
                    let Some(&i) = by_id.get(&id) else {
                        warn!("-X found folder for role, couldn't find local object for {role}");
                        continue;
                    };
                    categories[i].set_role(role);
                    // A new row is saved with its role; an existing one is reloaded fresh
                    // inside the transaction.
                    if categories[i].version() > 0 {
                        role_updates.push((id, role));
                    }
                    found = true;
                    break;
                }
                if found {
                    break;
                }
            }
        }

        // Detach the copies in folders the server no longer has before their rows go, in
        // bounded transactions, so a quit before the removal leaves every message's
        // snapshot matching its rows.
        let processor = self.w.processor();
        for id in unused_folders.keys().chain(unused_labels.keys()) {
            processor.detach_messages_from_folder(id, Duration::ZERO)?;
        }
        drop(processor);

        let placed_since_detach = self.w.store.transaction("syncFoldersAndLabels", |s| {
            for &i in &created {
                // A folder that already exists must never take the process down; the row is
                // there either way and the next pass picks it up.
                let saved = match &mut categories[i] {
                    Category::Folder(f) => s.save(f),
                    Category::Label(l) => s.save(l),
                };
                if let Err(e) = saved {
                    if !e.is_constraint_violation() {
                        return Err(e);
                    }
                    let c = &categories[i];
                    error!(
                        "-X could not create {} {}: {e}. Continuing.",
                        c.table(),
                        c.path()
                    );
                }
            }
            for (id, role) in &role_updates {
                let fresh_q = Query::new().equal("id", id);
                if label_ids.contains(id) {
                    if let Some(mut l) = s.find::<Label>(&fresh_q)? {
                        l.set_role(role);
                        s.save(&mut l)?;
                    }
                } else if let Some(mut f) = s.find::<Folder>(&fresh_q)? {
                    f.set_role(role);
                    s.save(&mut f)?;
                }
            }
            // A copy the foreground worker recorded in a removed folder after the detach is
            // deleted with the folder, and its message caught up once this commits.
            let mut placed = Vec::new();
            for f in unused_folders.values_mut() {
                s.remove(f)?;
                placed.extend(s.delete_placements_for_folder(f.id())?);
            }
            for l in unused_labels.values_mut() {
                s.remove(l)?;
                placed.extend(s.delete_placements_for_folder(l.id())?);
            }
            Ok(placed)
        })?;
        self.w.processor().refresh_messages(
            &placed_since_detach,
            UnplacedMessages::Remove,
            "syncFoldersAndLabels",
        )?;

        Ok(categories
            .into_iter()
            .filter_map(|c| match c {
                Category::Folder(f) => Some(f),
                Category::Label(_) => None,
            })
            .collect())
    }
}
