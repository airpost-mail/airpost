// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `SyncWorker`: one IMAP connection and one store, and the folder-scan steps both mail
//! threads share (range diff, CONDSTORE/QRESYNC changes, VANISHED, body fetches, the body
//! cache). The background pass is in [`crate::background`], the IDLE cycle in
//! [`crate::foreground`].
//!
//! Derived from Mailspring-Sync `MailSync/SyncWorker.*`, Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ms_core::constants::{
    ls, BODY_SYNC_BATCH, BODY_SYNC_MAX_AGE, MAX_FULL_HEADERS_REQUEST_SIZE,
    MODSEQ_TRUNCATION_THRESHOLD, MODSEQ_TRUNCATION_UID_COUNT, NO_BODY_CACHE_ROLES,
    STREAM_DELAY_WORKER_MS, UID_LIST_CHUNK,
};
use ms_core::{logmsg, ErrorCode, Result, UidSet};
use ms_imap::{FetchKind, FolderStatus, ImapAuth, ImapConfig, ImapSession, Session};
use ms_net::HttpClient;
use ms_store::{Folder, Message, Model, Query, Store};
use rusqlite::params;
use serde_json::Value;
use tracing::{error, info, warn};

use crate::mail_processor::{attributes_of, id_for_message, MailProcessor};
use crate::WorkerContext;

/// A mailcore `Range`: `location` through `location + length` inclusive, `length`
/// `u64::MAX` for `location:*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Range {
    pub location: u32,
    pub length: u64,
}

impl Range {
    pub fn new(location: u32, length: u64) -> Self {
        Range { location, length }
    }

    pub fn open(location: u32) -> Self {
        Range {
            location,
            length: u64::MAX,
        }
    }

    fn is_open(&self) -> bool {
        self.length == u64::MAX
    }

    fn end(&self) -> Option<u32> {
        if self.is_open() {
            return None;
        }
        Some((self.location as u64 + self.length).min(u32::MAX as u64) as u32)
    }

    pub fn uid_set(&self) -> UidSet {
        UidSet::range(self.location, self.end())
    }

    /// `1 - 120`, or `1 - *` for an open range (the location + length would wrap).
    fn describe(&self) -> String {
        match self.is_open() {
            true => format!("{} - *", self.location),
            false => format!("{} - {}", self.location, self.location as u64 + self.length),
        }
    }
}

/// The result of syncing a UID range (`UIDRangeSyncResult`). `truncated` is set when the
/// range held more messages needing full headers than one request takes; `synced_min_uid`
/// is then the lowest UID ingested, and everything below it still needs fetching. Callers
/// must not record the range as synced past it, or those messages are lost until
/// UIDVALIDITY changes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UidRangeSyncResult {
    pub truncated: bool,
    pub synced_min_uid: u32,
    /// Messages in the range that needed full headers, before truncation.
    pub needed: usize,
}

/// `localStatus[key]` as a number, if it is one.
pub(crate) fn ls_i64(status: &Value, key: &str) -> Option<i64> {
    let v = status.get(key)?;
    v.as_i64()
        .or_else(|| v.as_u64().map(|u| u as i64))
        .or_else(|| v.as_f64().map(|f| f as i64))
}

pub(crate) fn ls_u32(status: &Value, key: &str) -> u32 {
    ls_i64(status, key).unwrap_or(0).clamp(0, u32::MAX as i64) as u32
}

pub(crate) fn ls_u64(status: &Value, key: &str) -> u64 {
    let Some(v) = status.get(key) else { return 0 };
    v.as_u64()
        .or_else(|| v.as_f64().map(|f| f as u64))
        .unwrap_or(0)
}

pub(crate) fn set_ls(folder: &mut Folder, key: &str, value: impl Into<Value>) {
    folder
        .local_status_mut()
        .insert(key.to_owned(), value.into());
}

/// "Has a server copy in this folder", as a correlated subquery on Message. The body
/// queries are driven from the Message date indexes rather than from the folder's
/// placements: on a 180k-message folder the placement walk costs 0.8-1.3 s, the
/// date-bounded walk over the account's recent messages 30-70 ms.
const HAS_SERVER_COPY_IN_FOLDER: &str = " AND EXISTS (SELECT 1 FROM MessageFolder WHERE MessageFolder.messageId = Message.id AND MessageFolder.folderId = ? AND MessageFolder.remoteUID > 0)";

/// Messages wanted in the body cache are the recent ones plus drafts of any age, always
/// as two statements: OR'd in one WHERE the planner drops the date bound and walks the
/// whole account, and given a choice it walks every older message for the drafts too, so
/// the draft statement names its index.
const RECENT_BODY_CANDIDATES: &str =
    "FROM Message WHERE Message.accountId = ? AND Message.date > ?";
const OLDER_DRAFT_BODY_CANDIDATES: &str = "FROM Message INDEXED BY MessageListDraftIndex WHERE Message.accountId = ? AND Message.draft = 1 AND Message.date <= ?";

pub(crate) struct SyncWorker {
    pub ctx: WorkerContext,
    pub store: Store,
    pub session: Session,
    http: Option<HttpClient>,
    /// Per folder, how many messages the last truncated full-folder scan still needed: a
    /// draining backlog (the count falls each pass) versus one that never drains.
    last_truncated_scan_needed: HashMap<String, usize>,
}

impl SyncWorker {
    pub fn new(ctx: WorkerContext) -> Result<Self> {
        let store = Store::open(&ctx.config_dir, ctx.deltas.clone())?;
        store.set_clock(ctx.clock.clone());
        store.set_stream_delay(STREAM_DELAY_WORKER_MS);
        let auth = ImapAuth::Password(ctx.account.imap_password().to_owned());
        let mut config = ImapConfig::for_account(&ctx.account, auth);
        if ctx.verbose {
            config.logger = Some(std::sync::Arc::new(ms_core::protolog::TracingLogger));
        }
        Ok(SyncWorker {
            ctx,
            store,
            session: Session::new(config),
            http: None,
            last_truncated_scan_needed: HashMap::new(),
        })
    }

    pub fn now(&self) -> i64 {
        self.ctx.clock.now()
    }

    pub fn account_id(&self) -> &str {
        self.ctx.account.id()
    }

    /// `configure` / `MailUtils::configureSessionForAccount`: an OAuth account gets a
    /// current access token before every iteration. The refresh is blocking HTTP, so it
    /// runs here, outside the session's runtime.
    pub fn configure(&mut self) -> Result<()> {
        if self.ctx.account.refresh_token().is_empty() {
            return Ok(());
        }
        if self.http.is_none() {
            self.http = Some(HttpClient::new()?);
        }
        let http = self.http.as_ref().expect("set above");
        let token = self.ctx.tokens.token_for(http, &self.ctx.account)?;
        self.session.set_auth(ImapAuth::XOAuth2(token.access_token));
        Ok(())
    }

    pub fn is_gmail(&self) -> bool {
        self.session.profile().mailbox_model == ms_providers::MailboxModelKind::GmailLabels
    }

    /// Inserts or updates the message a fetch returned. A panic while processing it (a
    /// parser bug on hostile input) quarantines its UID in this folder (TD45): it is skipped
    /// from then on instead of stopping the account at every pass. `None` for a skipped UID.
    pub fn ingest(
        &self,
        remote_msg: &ms_imap::ImapMessage,
        folder: &Folder,
        sync_data_timestamp: i64,
    ) -> Result<Option<Message>> {
        let uid = remote_msg.uid;
        let key = quarantine_key(folder);
        let quarantined = self.store.get_key_value(&key)?.unwrap_or_default();
        if parse_uids(&quarantined).contains(&uid) {
            return Ok(None);
        }
        if self.store.in_transaction() {
            return self
                .processor()
                .insert_fallback_to_update_message(remote_msg, folder, sync_data_timestamp)
                .map(Some);
        }
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.processor().insert_fallback_to_update_message(
                remote_msg,
                folder,
                sync_data_timestamp,
            )
        }));
        match outcome {
            Ok(r) => r.map(Some),
            Err(_) => {
                self.store.abandon_after_panic();
                error!(
                    "- {} UID {uid}: processing the message panicked; it is skipped from now on",
                    folder.path()
                );
                let mut uids = parse_uids(&quarantined);
                uids.push(uid);
                if uids.len() > MAX_QUARANTINED_UIDS {
                    uids.drain(..uids.len() - MAX_QUARANTINED_UIDS);
                }
                self.store
                    .save_key_value(&key, &serde_json::to_string(&uids).unwrap_or_default())?;
                Ok(None)
            }
        }
    }

    pub fn processor(&self) -> MailProcessor<'_> {
        let mut p = MailProcessor::new(&self.ctx.account, &self.store)
            .with_config_dir(&self.ctx.config_dir);
        p.is_gmail = self.is_gmail();
        p
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.session.capabilities().has(cap)
    }

    /// Dovecot answers STATUS and FETCH for the mailbox a connection has selected from that
    /// connection's own view of it; only NOOP, IDLE or another SELECT delivers the EXISTS /
    /// EXPUNGE / VANISHED that bring it up to date, and FETCH data even precedes the
    /// EXPUNGE lines in the same response (RFC 3501 6.1.2; test/README.md "Things learned
    /// from Dovecot", conformance `probe_stale_fetch`).
    pub fn noop_selected_folder(&mut self) -> Result<()> {
        self.session.noop().map_err(|e| e.into_sync("noop"))?;
        Ok(())
    }

    /// True if a full-folder scan left work behind and the backlog is still shrinking, so
    /// the caller should come straight back (`shouldRetryTruncatedScan`). Some messages can
    /// never be ingested while a user's task on them is in flight, so they are reported as
    /// needing headers on every pass; without the progress check that is an unthrottled
    /// loop that also starves other folders. Must be called for every full-folder scan.
    pub fn should_retry_truncated_scan(
        &mut self,
        folder: &Folder,
        scan: &UidRangeSyncResult,
    ) -> bool {
        if !scan.truncated {
            self.last_truncated_scan_needed.remove(folder.id());
            return false;
        }
        let draining = self
            .last_truncated_scan_needed
            .get(folder.id())
            .is_none_or(|prev| scan.needed < *prev);
        // Note: kept on the back-off path too; it is what bounds the retry loop.
        self.last_truncated_scan_needed
            .insert(folder.id().to_owned(), scan.needed);
        if !draining {
            warn!(
                "- {}: {} messages still need full headers after a full pass and the count is not falling. Backing off instead of rescanning immediately.",
                folder.path(),
                scan.needed
            );
        }
        draining
    }

    /// Diffs one UID range of a folder against the store (`syncFolderUIDRange`): new and
    /// changed copies are ingested, copies the server no longer reports are deleted.
    pub fn sync_folder_uid_range(
        &mut self,
        folder: &Folder,
        mut range: Range,
        mut heavy_initial_request: bool,
        mut synced: Option<&mut Vec<(Message, u32)>>,
    ) -> Result<UidRangeSyncResult> {
        let remote_path = folder.path().to_owned();
        // "0" is not a valid start and makes the server return only the last item.
        if range.location == 0 {
            range.location = 1;
        }
        // An attributes-only sync of a range too large to ask full headers for in one go.
        if range.length > MAX_FULL_HEADERS_REQUEST_SIZE as u64 {
            heavy_initial_request = false;
        }
        let mut result = UidRangeSyncResult {
            truncated: false,
            synced_min_uid: range.location,
            needed: 0,
        };
        info!(
            "{}",
            logmsg::sync_folder_uid_range(&remote_path, &range.describe(), heavy_initial_request)
        );

        // A FETCH of the folder this connection already has selected is answered from its
        // stale view, so a copy the foreground connection just moved out would be re-added.
        // With QRESYNC the trailing VANISHED corrects that; without it a NOOP must come first.
        if !self.session.is_qresync_enabled()
            && self.session.profile().noop_before_selected_status
            && self
                .session
                .current_folder()
                .is_some_and(|c| c.eq_ignore_ascii_case(&remote_path))
        {
            self.noop_selected_folder()?;
        }

        let set = range.uid_set();
        // Local attributes first: the remote fetch may take a long time, and deletions must
        // be computed old <> old, not new <> old, or messages ingested meanwhile by the
        // other worker would look deleted. Re-downloading is better.
        let mut local = self
            .store
            .fetch_messages_attributes_in_range(&set, folder)?;

        let mut sync_data_timestamp = self.now();
        let kind = if heavy_initial_request {
            FetchKind::Heavy
        } else {
            FetchKind::Light
        };
        let remote = self
            .session
            .fetch_by_uid(&remote_path, kind, &set)
            .map_err(|e| e.into_sync("syncFolderUIDRange - fetchMessagesByUID"))?;

        // RFC 9738 3: a server advertising MESSAGELIMIT (Yahoo) may answer a FETCH over more
        // messages for only the highest-UID ones, flagged on the tagged OK. Copies below the
        // lowest UID returned were not looked at, so they are not gone.
        let mut scanned_min_uid = 0;
        if remote.hit_message_limit && !remote.messages.is_empty() {
            scanned_min_uid = remote.messages.iter().map(|m| m.uid).min().unwrap_or(0);
            result.truncated = true;
            result.synced_min_uid = result.synced_min_uid.max(scanned_min_uid);
            warn!(
                "{}",
                logmsg::message_limit_applied(&remote_path, scanned_min_uid)
            );
        }

        info!(
            "- {}: remote={}, local={}, folderId={}",
            remote_path,
            remote.messages.len(),
            local.len(),
            folder.id()
        );

        let mut last_sleep = Instant::now();
        let mut heavy_needed_uids: Vec<u32> = Vec::new();
        for remote_msg in remote.messages.iter().rev() {
            // Never insert for more than ~250 ms without a pause, so the other thread waiting
            // for the database is not starved.
            if last_sleep.elapsed() > Duration::from_millis(500) {
                std::thread::sleep(Duration::from_millis(50));
                last_sleep = Instant::now();
            }
            let uid = remote_msg.uid;
            let existing = local.remove(&uid);
            let mut same = existing
                .as_ref()
                .is_some_and(|l| l.matches(&attributes_of(remote_msg)));

            // With headers in hand, a row that names another message is wrong however well
            // its flags match (a mis-paired COPYUID). Ingesting the UID gives it to the
            // message it holds; the ingest of the displaced message's own UID relinks it.
            if same && heavy_initial_request {
                let remote_id = id_for_message(folder.account_id(), &remote_path, remote_msg);
                let recorded = existing
                    .as_ref()
                    .map(|l| l.message_id.as_str())
                    .unwrap_or("");
                if remote_id != recorded {
                    warn!(
                        "- {} UID {} holds message {}, recorded as {}",
                        remote_path, uid, remote_id, recorded
                    );
                    same = false;
                }
            }

            if !same {
                if heavy_initial_request {
                    let m = self.ingest(remote_msg, folder, sync_data_timestamp)?;
                    if let (Some(s), Some(m)) = (synced.as_deref_mut(), m) {
                        s.push((m, uid));
                    }
                } else {
                    // Collected and chosen from below: truncating here would depend on the
                    // order the server returned messages in, which need not be by UID.
                    heavy_needed_uids.push(uid);
                }
            }
        }

        if !heavy_initial_request && !heavy_needed_uids.is_empty() {
            // Could be enormous (a mass import, years offline, an earlier sync bug), so only
            // the newest batch is fetched and the range reported synced down to it.
            let ideal = heavy_needed_uids.len();
            result.needed = ideal;
            heavy_needed_uids.sort_unstable();
            // Note: a request over MESSAGELIMIT would be answered only in part.
            let mut request_size = MAX_FULL_HEADERS_REQUEST_SIZE as usize;
            if let Some(limit) = self.session.message_limit().filter(|l| *l > 0) {
                request_size = request_size.min(limit as usize);
            }
            if heavy_needed_uids.len() > request_size {
                // The highest (newest) UIDs, so the remainder is one contiguous block at
                // the bottom that `synced_min_uid` can describe.
                heavy_needed_uids.drain(..heavy_needed_uids.len() - request_size);
                result.truncated = true;
                result.synced_min_uid = heavy_needed_uids[0];
            }
            info!(
                "{}",
                logmsg::fetching_full_headers(heavy_needed_uids.len(), ideal)
            );
            sync_data_timestamp = self.now();
            let heavy = self
                .session
                .fetch_by_uid(
                    &remote_path,
                    FetchKind::Heavy,
                    &UidSet::from_uids(heavy_needed_uids.iter().copied()),
                )
                .map_err(|e| e.into_sync("syncFolderUIDRange - fetchMessagesByUID (heavy)"))?;
            for remote_msg in heavy.messages.iter().rev() {
                let m = self.ingest(remote_msg, folder, sync_data_timestamp)?;
                if let (Some(s), Some(m)) = (synced.as_deref_mut(), m) {
                    s.push((m, remote_msg.uid));
                }
            }
        }

        // The UIDs left are copies we had in the range which the server no longer reports.
        // The end-of-pass sweep removes messages left with no copy that do not turn up in
        // another folder first.
        if !local.is_empty() {
            let mut deleted: Vec<u32> = local
                .keys()
                .copied()
                .filter(|u| *u >= scanned_min_uid)
                .collect();
            deleted.sort_unstable();
            for chunk in deleted.chunks(UID_LIST_CHUNK) {
                self.processor().delete_vanished_placements(folder, chunk)?;
            }
        }
        Ok(result)
    }

    /// CONDSTORE / QRESYNC changes since the stored HIGHESTMODSEQ
    /// (`syncFolderChangesViaCondstore`). `must_sync_all` is the background pass, which
    /// never truncates the request.
    pub fn sync_folder_changes_via_condstore(
        &mut self,
        folder: &mut Folder,
        remote_status: &FolderStatus,
        must_sync_all: bool,
    ) -> Result<()> {
        let uidnext = ls_u32(folder.local_status(), ls::UIDNEXT);
        let modseq = ls_u64(folder.local_status(), ls::HIGHESTMODSEQ);
        let remote_modseq = remote_status.highest_modseq;
        let remote_uidnext = remote_status.uid_next;
        let sync_data_timestamp = self.now();
        info!(
            "syncFolderChangesViaCondstore - {}: modseq {} to {}, uidnext {} to {}",
            folder.path(),
            modseq,
            remote_modseq,
            uidnext,
            remote_uidnext
        );
        let path = folder.path().to_owned();

        // Before the early return below, and before HIGHESTMODSEQ moves.
        let vanished = self.session.take_vanished(&path);
        self.delete_vanished_uids(folder, &vanished, "this connection")?;

        if modseq == remote_modseq && uidnext == remote_uidnext {
            return Ok(());
        }
        // A folder that never held a message has nothing to report, and `1:0` would go on
        // the wire as `1:*`.
        if remote_uidnext == 1 {
            set_ls(folder, ls::UIDNEXT, remote_uidnext);
            set_ls(folder, ls::HIGHESTMODSEQ, remote_modseq);
            return Ok(());
        }

        // The set ends at UIDNEXT-1, not `*`: RFC 7162 3.2.6 limits VANISHED to UIDs in the
        // set and `*` is the highest UID still in the mailbox, so Cyrus never reports an
        // expunge above it (cyrus-imapd #6071). UIDNEXT-1 is the bound RFC 7162 3.2.5.1
        // uses. A STATUS without UIDNEXT (NetEase) reports 0 and keeps `*`.
        let top_uid: u64 =
            if remote_uidnext > 1 && self.session.profile().vanished_upper_bound_uidnext {
                remote_uidnext as u64 - 1
            } else {
                u64::MAX
            };

        // A very large modseq gap makes a request that can take long enough to block the
        // foreground worker's mailbox actions. Ask for the newest UIDs only; the deep scan
        // recovers the rest.
        let mut bottom_uid: u32 = 1;
        let mut limited = false;
        if !must_sync_all && remote_modseq.saturating_sub(modseq) > MODSEQ_TRUNCATION_THRESHOLD {
            bottom_uid = if remote_uidnext > MODSEQ_TRUNCATION_UID_COUNT {
                remote_uidnext - MODSEQ_TRUNCATION_UID_COUNT
            } else {
                1
            };
            limited = true;
            warn!("{}", logmsg::condstore_request_limited(bottom_uid, top_uid));
        }
        let uids = match top_uid {
            u64::MAX => UidSet::range(bottom_uid, None),
            top => UidSet::range(bottom_uid, Some(top as u32)),
        };

        let result = self
            .session
            .sync_by_uid(&path, FetchKind::Heavy, &uids, modseq)
            .map_err(|e| e.into_sync("syncFolderChangesViaCondstore - syncMessagesByUID"))?;
        info!(
            "syncFolderChangesViaCondstore - Changes since HMODSEQ {}: {} changed, {} vanished",
            modseq,
            result.modified_or_added.len(),
            result.vanished.len().unwrap_or(u64::MAX)
        );
        for m in &result.modified_or_added {
            self.ingest(m, folder, sync_data_timestamp)?;
        }
        self.delete_vanished_uids(folder, &result.vanished, "FETCH CHANGEDSINCE")?;
        // An unrelated expunge can arrive untagged while this FETCH is in flight.
        let vanished = self.session.take_vanished(&path);
        self.delete_vanished_uids(folder, &vanished, "this connection")?;

        set_ls(folder, ls::UIDNEXT, remote_uidnext);
        // A limited request has not applied the changes below its bottom UID. Recording the
        // server's modseq would make the background pass see nothing left to fetch.
        if !limited {
            set_ls(folder, ls::HIGHESTMODSEQ, remote_modseq);
        }
        Ok(())
    }

    pub fn delete_vanished_uids(
        &mut self,
        folder: &Folder,
        vanished: &UidSet,
        source: &str,
    ) -> Result<()> {
        if vanished.is_empty() {
            return Ok(());
        }
        info!(
            "Deleting {} VANISHED UID range(s) in {} reported by {}",
            vanished.ranges().len(),
            folder.path(),
            source
        );
        self.processor()
            .delete_vanished_placements_in(folder, vanished)
    }

    // -- Body cache ------------------------------------------------------------------------

    fn max_age_for_body_sync(&self) -> i64 {
        BODY_SYNC_MAX_AGE
    }

    pub fn should_cache_bodies_in_folder(folder: &Folder) -> bool {
        !NO_BODY_CACHE_ROLES.contains(&folder.role())
    }

    pub fn clean_message_cache(&mut self, folder: &mut Folder) -> Result<()> {
        info!("Cleaning local cache and updating stats");
        // Only bodies fetched more than 14 days ago, so one being read is not deleted.
        let purged = self.store.conn().execute(
            "DELETE FROM MessageBody WHERE MessageBody.fetchedAt < datetime('now', '-14 days') AND MessageBody.id IN (\
             SELECT Message.id FROM MessageFolder INNER JOIN Message ON Message.id = MessageFolder.messageId \
             WHERE MessageFolder.accountId = ? AND MessageFolder.folderId = ? AND MessageFolder.remoteUID > 0 \
             AND Message.draft = 0 AND Message.date < ?)",
            params![
                folder.account_id(),
                folder.id(),
                self.now() - self.max_age_for_body_sync()
            ],
        )?;
        info!("-- {} message bodies deleted from local cache.", purged);
        let present = self.count_bodies_downloaded(folder)?;
        let wanted = self.count_bodies_needed(folder)?;
        set_ls(folder, ls::BODIES_PRESENT, present);
        set_ls(folder, ls::BODIES_WANTED, wanted);
        Ok(())
    }

    fn count_bodies_downloaded(&self, folder: &Folder) -> Result<i64> {
        Ok(self.store.conn().query_row(
            "SELECT COUNT(DISTINCT MessageFolder.messageId) FROM MessageFolder INNER JOIN MessageBody ON MessageBody.id = MessageFolder.messageId \
             WHERE MessageFolder.accountId = ? AND MessageFolder.folderId = ? AND MessageFolder.remoteUID > 0 \
             AND MessageBody.value IS NOT NULL",
            params![folder.account_id(), folder.id()],
            |r| r.get(0),
        )?)
    }

    fn count_bodies_needed(&self, folder: &Folder) -> Result<i64> {
        if !Self::should_cache_bodies_in_folder(folder) {
            return Ok(0);
        }
        let cutoff = self.now() - self.max_age_for_body_sync();
        let mut total = 0i64;
        for candidates in [RECENT_BODY_CANDIDATES, OLDER_DRAFT_BODY_CANDIDATES] {
            let sql = format!("SELECT COUNT(*) {candidates}{HAS_SERVER_COPY_IN_FOLDER}");
            let n: i64 = self.store.conn().query_row(
                &sql,
                params![folder.account_id(), cutoff, folder.id()],
                |r| r.get(0),
            )?;
            total += n;
        }
        Ok(total)
    }

    /// Fetches the newest missing bodies in the folder, at most 30. Returns whether it did
    /// any work (`syncMessageBodies`).
    pub fn sync_message_bodies(&mut self, folder: &mut Folder) -> Result<bool> {
        if !Self::should_cache_bodies_in_folder(folder) {
            return Ok(false);
        }
        let limit = BODY_SYNC_BATCH;
        let cutoff = self.now() - self.max_age_for_body_sync();
        let tail = format!(
            "{HAS_SERVER_COPY_IN_FOLDER} AND NOT EXISTS (SELECT 1 FROM MessageBody WHERE MessageBody.id = Message.id) ORDER BY Message.date DESC LIMIT ?"
        );
        let mut ids: Vec<String> = Vec::new();
        for candidates in [RECENT_BODY_CANDIDATES, OLDER_DRAFT_BODY_CANDIDATES] {
            if ids.len() >= limit {
                break;
            }
            let sql = format!("SELECT Message.id {candidates}{tail}");
            let mut stmt = self.store.conn().prepare_cached(&sql)?;
            let rows = stmt.query_map(
                params![
                    folder.account_id(),
                    cutoff,
                    folder.id(),
                    (limit - ids.len()) as i64
                ],
                |r| r.get::<_, String>(0),
            )?;
            for id in rows {
                ids.push(id?);
            }
        }
        if ids.is_empty() {
            return Ok(false);
        }

        let results = self.store.transaction("syncMessageBodies", |s| {
            // Re-checked inside a transaction: an empty body reserves each message for this
            // worker so the other one does not fetch it at the same time, and a persistent
            // fetch failure is tried only once instead of keeping the account syncing.
            let sql = format!(
                "SELECT Message.id FROM Message LEFT JOIN MessageBody ON MessageBody.id = Message.id WHERE Message.id IN ({}) AND MessageBody.id IS NULL",
                vec!["?"; ids.len()].join(",")
            );
            let still: Vec<String> = s
                .conn()
                .prepare(&sql)?
                .query_map(rusqlite::params_from_iter(ids.iter()), |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            if still.len() < ids.len() {
                info!(
                    "Body for {} messages already being fetched.",
                    ids.len() - still.len()
                );
            }
            if still.is_empty() {
                return Ok(Vec::new());
            }
            let messages = s.find_all::<Message>(&Query::new().equal_any("id", still.iter()))?;
            let mut placeholder = s
                .conn()
                .prepare_cached("INSERT OR IGNORE INTO MessageBody (id, value) VALUES (?, NULL)")?;
            for m in &messages {
                placeholder.execute([m.id()])?;
            }
            Ok(messages)
        })?;

        if ls_i64(folder.local_status(), ls::BODIES_PRESENT).is_none() {
            set_ls(folder, ls::BODIES_PRESENT, 0);
        }
        let did_work = !results.is_empty();
        for mut m in results {
            // Recomputed by COUNT(*) during cleanup, so a lost increment is fine.
            let present = ls_i64(folder.local_status(), ls::BODIES_PRESENT).unwrap_or(0);
            set_ls(folder, ls::BODIES_PRESENT, present + 1);
            self.sync_message_body(&mut m, Some(folder))?;
        }
        Ok(did_work)
    }

    /// Fetches the body from the first copy the server returns, in the order
    /// `fetchable_copies_of_message` ranks them (`syncMessageBody`). A copy the server
    /// refuses to FETCH (typically expunged since the last scan) gives way to the next;
    /// any other error goes to the caller.
    pub fn sync_message_body(
        &mut self,
        message: &mut Message,
        preferred: Option<&Folder>,
    ) -> Result<()> {
        let candidates = self.store.fetchable_copies_of_message(message, preferred)?;
        if candidates.is_empty() {
            info!(
                "No copy of message \"{}\" ({}) is on the server to fetch a body from.",
                message.subject(),
                message.id()
            );
            return Ok(());
        }
        for candidate in candidates {
            let path = candidate.folder.path().to_owned();
            let data = match self.session.fetch_message_bytes(&path, candidate.uid) {
                Ok(d) => d,
                Err(e) => {
                    error!(
                        "Unable to fetch body for message \"{}\" ({} UID {}). Error {}",
                        message.subject(),
                        path,
                        candidate.uid,
                        e.code.name()
                    );
                    // The copy may already be gone: the scan may not have caught up yet, and
                    // drafts in particular come and go.
                    if e.code == ErrorCode::Fetch {
                        continue;
                    }
                    return Err(e.into_sync("syncMessageBody - fetchMessageByUID").into());
                }
            };
            let Some(data) = data else {
                error!(
                    "fetchMessageByUID returned null data for message \"{}\" ({} UID {})",
                    message.subject(),
                    path,
                    candidate.uid
                );
                continue;
            };
            let parsed = match ms_mime::parse_message(&data) {
                Ok(p) => p,
                Err(e) => {
                    error!(
                        "Unable to parse the body of message \"{}\" ({} UID {}): {e}",
                        message.subject(),
                        path,
                        candidate.uid
                    );
                    return Ok(());
                }
            };
            return self.processor().retrieved_message_body(message, &parsed);
        }
        Ok(())
    }
}

/// UIDs quarantined per folder, the newest kept.
const MAX_QUARANTINED_UIDS: usize = 1000;

/// The `_State` key listing a folder's quarantined UIDs (JSON array). Keyed by UIDVALIDITY
/// too, so a reset UID space starts with none.
fn quarantine_key(folder: &Folder) -> String {
    let validity = ls_u32(folder.local_status(), ls::UIDVALIDITY);
    format!("quarantine-{}-{validity}", folder.id())
}

fn parse_uids(text: &str) -> Vec<u32> {
    serde_json::from_str(text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_is_inclusive_and_open_ended() {
        let r = Range::new(5, 10);
        assert_eq!(r.describe(), "5 - 15");
        assert_eq!(r.uid_set(), UidSet::range(5, Some(15)));
        let open = Range::open(1);
        assert_eq!(open.describe(), "1 - *");
        assert_eq!(open.uid_set(), UidSet::range(1, None));
    }

    #[test]
    fn local_status_helpers() {
        let mut f = Folder::new("f", "a", 0);
        set_ls(&mut f, ls::UIDNEXT, 42u32);
        assert_eq!(ls_u32(f.local_status(), ls::UIDNEXT), 42);
        assert_eq!(ls_i64(f.local_status(), ls::LAST_DEEP), None);
        assert_eq!(ls_u32(f.local_status(), ls::LAST_DEEP), 0);
    }
}

