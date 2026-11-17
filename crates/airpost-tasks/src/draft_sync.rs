// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Server-synced drafts, the foreground worker's part: with
//! "Save drafts to server" on, a saved draft is APPENDed to the Drafts folder once the user
//! has stopped editing it, its new copy recorded at once, and the copies it replaces
//! deleted. A save never waits for the server: SyncbackDraftTask only bumps `_dsync.rev`,
//! and the foreground pushes after its tasks, between IDLEs.
//!
//! Before pushing, the foreground reconciles each draft with the copies a scan found on the
//! server (3.3): a copy at a higher UID than the one the draft was pushed as is another
//! device's revision. It is taken when the draft has no unpushed edits here, and otherwise
//! moved to Trash (not expunged) so the local revision wins at the next push.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use airpost_core::constants::{
    draft_push_backoff, draft_push_quiet, DRAFT_PUSHES_PER_PASS, DRAFT_PUSH_LARGE_BYTES,
    DRAFT_TOMBSTONE_LIFETIME, FILES_DIR,
};
use airpost_core::{logmsg, MessageFlags, Result, UidSet};
use airpost_imap::{FetchKind, ImapSession, StoreOp};
use airpost_store::drafts::{DraftBase, DraftSync};
use airpost_store::{File, Folder, Label, Message, MessageAttributes, Model, Placement, Query};
use airpost_sync::drafts::TombstonedCopy;
use airpost_sync::mail_processor::MailProcessor;
use serde_json::Value;
use tracing::info;

use crate::imap_ops::remove_messages_resilient;
use crate::json::elements;
use crate::messages::is_gmail;
use crate::send::{body, find_recent};
use crate::TaskProcessor;

/// Tasks that end a draft: it is not pushed while one is queued.
const DRAFT_ENDING_TASKS: [&str; 2] = ["SendDraftTask", "DestroyDraftTask"];

/// How often a draft from before drafts synced looks again whether Sent has been walked.
const SENT_WALK_POLL: i64 = 30;

/// Where pushed drafts go: the folder APPENDed to, and the folder the copy is recorded in
/// (on Gmail the drafts label and All Mail, elsewhere both the Drafts folder).
struct PushTarget {
    append_path: String,
    home: Folder,
    gmail: bool,
}

/// What became of one push.
enum Pushed {
    Done,
    /// The server could not be written to; the rest wait for the next iteration.
    Failed {
        retry_at: i64,
    },
    /// Not pushed, and not to be tried again until the next save.
    Skipped,
}

impl TaskProcessor<'_> {
    /// The `_dsync` a draft being saved gets: the stored draft's, never the copy the client
    /// echoes, with `rev` bumped when the account syncs drafts.
    pub(crate) fn next_draft_sync(
        &self,
        existing: Option<&Message>,
        now: i64,
    ) -> Option<DraftSync> {
        let prior = existing.and_then(Message::draft_sync);
        if !self.account.save_drafts_to_server() {
            return prior;
        }
        let mut sync = prior.unwrap_or_default();
        sync.rev += 1;
        sync.saved_at = now;
        sync.fails = 0;
        sync.next_at = 0;
        Some(sync)
    }

    fn files_root(&self) -> PathBuf {
        self.config_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(FILES_DIR)
    }

    /// Deletes server copies of drafts sent or deleted here, which scans skipped, and lets
    /// tombstones older than [`DRAFT_TOMBSTONE_LIFETIME`] go.
    pub(crate) fn delete_tombstoned_copies(
        &self,
        session: &mut dyn ImapSession,
        copies: &[TombstonedCopy],
    ) -> Result<()> {
        self.store.expire_draft_tombstones(
            self.account.id(),
            self.store.now() - DRAFT_TOMBSTONE_LIFETIME,
        )?;
        let mut by_folder: BTreeMap<&str, Vec<&TombstonedCopy>> = BTreeMap::new();
        for c in copies {
            by_folder.entry(&c.folder_id).or_default().push(c);
        }
        for (folder_id, copies) in by_folder {
            let Some(folder) = self.store.folder_by_id(self.account.id(), folder_id)? else {
                continue;
            };
            for c in &copies {
                info!(
                    "{}",
                    logmsg::deleting_tombstoned_draft_copy(
                        &c.header_message_id,
                        folder.path(),
                        c.uid
                    )
                );
            }
            let set = UidSet::from_uids(copies.iter().map(|c| c.uid));
            remove_messages_resilient(session, self.store, self.account.id(), folder.path(), &set)?;
        }
        Ok(())
    }

    /// Takes or overrules the server revisions of drafts other devices saved.
    pub(crate) fn reconcile_draft_copies(&self, session: &mut dyn ImapSession) -> Result<()> {
        if !self.account.save_drafts_to_server() {
            return Ok(());
        }
        let Some(home) = self.store.draft_home_folder(self.account.id())? else {
            return Ok(());
        };
        let ids = self
            .store
            .drafts_with_unreconciled_copies(self.account.id(), home.id())?;
        if ids.is_empty() {
            return Ok(());
        }
        let busy = self.drafts_with_pending_tasks()?;
        for id in ids {
            let Some(draft) = self.store.find::<Message>(&Query::new().equal("id", &id))? else {
                continue;
            };
            if busy.contains(draft.id()) || busy.contains(draft.header_message_id()) {
                continue;
            }
            self.reconcile_draft(session, &home, draft)?;
        }
        Ok(())
    }

    fn reconcile_draft(
        &self,
        session: &mut dyn ImapSession,
        home: &Folder,
        draft: Message,
    ) -> Result<()> {
        let sync = draft.draft_sync().unwrap_or_default();
        let copies: Vec<u32> = self
            .store
            .placements_for_message(draft.id())?
            .iter()
            .filter(|p| p.folder_id == home.id() && p.remote_uid > 0)
            .map(|p| p.remote_uid)
            .collect();
        let base = sync.base.clone().filter(|b| b.f == home.id());
        let newer: Vec<u32> = copies
            .iter()
            .copied()
            .filter(|u| base.as_ref().is_none_or(|b| *u > b.u))
            .collect();
        let Some(&newest) = newer.iter().max() else {
            return Ok(());
        };
        let Some(base) = base else {
            // A server draft seen for the first time, or a copy pushed without learning its
            // UID: that copy is what the draft is.
            return self.set_draft_base(draft.id(), home, newest, None);
        };
        let Ok(Some(bytes)) = session.fetch_message_bytes(home.path(), newest) else {
            return Ok(());
        };
        let parsed = airpost_mime::parse_message(&bytes)?;
        if base.d.is_some() && parsed.header.date == base.d {
            // The copy pushed from here, found again at another UID.
            return self.set_draft_base(draft.id(), home, newest, base.d);
        }
        let hmid = draft.header_message_id().to_owned();
        if sync.is_dirty() {
            info!("{}", logmsg::draft_conflict(&hmid, newer.len()));
            return self.move_foreign_draft_copies_to_trash(session, home, draft.id(), &newer);
        }

        info!("{}", logmsg::draft_adopted(&hmid, home.path(), newest));
        let h = &parsed.header;
        let adopted = self.store.transaction("adoptDraft", |s| {
            let Some(mut fresh) = s.find::<Message>(&Query::new().equal("id", draft.id()))? else {
                return Ok(None);
            };
            let contacts = |list: &[airpost_core::Address]| {
                Value::Array(list.iter().map(|a| a.to_contact_json()).collect())
            };
            fresh.data["subject"] = serde_json::json!(h.subject.as_deref().unwrap_or(""));
            if let Some(date) = h.date {
                fresh.data["date"] = serde_json::json!(date);
            }
            fresh.data["from"] = contacts(&h.from[..h.from.len().min(1)]);
            fresh.data["to"] = contacts(&h.to);
            fresh.data["cc"] = contacts(&h.cc);
            fresh.data["bcc"] = contacts(&h.bcc);
            fresh.data["replyTo"] = contacts(&h.reply_to);
            if let Some(irt) = h.in_reply_to.last() {
                fresh.data["rthMsgId"] = serde_json::json!(irt);
            }
            let mut ds = fresh.draft_sync().unwrap_or_default();
            ds.rev = ds.rev.max(ds.pushed_rev);
            ds.pushed_rev = ds.rev;
            ds.base = Some(DraftBase {
                f: home.id().to_owned(),
                u: newest,
                d: h.date,
            });
            fresh.set_draft_sync(Some(&ds));
            s.save(&mut fresh)?;
            Ok(Some(fresh))
        })?;
        if let Some(mut adopted) = adopted {
            let mut processor = MailProcessor::new(&self.account, self.store);
            if let Some(dir) = &self.config_dir {
                processor = processor.with_config_dir(dir);
            }
            processor.retrieved_message_body(&mut adopted, &parsed)?;
        }
        Ok(())
    }

    fn set_draft_base(&self, id: &str, home: &Folder, uid: u32, date: Option<i64>) -> Result<()> {
        self.store.transaction("adoptDraft", |s| {
            if let Some(mut fresh) = s.find::<Message>(&Query::new().equal("id", id))? {
                let mut ds = fresh.draft_sync().unwrap_or_default();
                ds.base = Some(DraftBase {
                    f: home.id().to_owned(),
                    u: uid,
                    d: date,
                });
                fresh.set_draft_sync(Some(&ds));
                s.save(&mut fresh)?;
                s.unsafe_erase_transaction_deltas();
            }
            Ok(())
        })
    }

    /// The other device's revisions go to Trash without an expunge, so they can still be
    /// recovered there; the draft lets go of those copies.
    fn move_foreign_draft_copies_to_trash(
        &self,
        session: &mut dyn ImapSession,
        home: &Folder,
        id: &str,
        uids: &[u32],
    ) -> Result<()> {
        let Some(trash) = self.role_folder("trash")? else {
            return Ok(());
        };
        let set = UidSet::from_uids(uids.iter().copied());
        let moved = if session.capabilities().has("MOVE") {
            session.move_uids(home.path(), &set, trash.path()).is_ok()
        } else {
            session.copy_uids(home.path(), &set, trash.path()).is_ok()
                && session
                    .store_flags(home.path(), &set, StoreOp::Add, MessageFlags::DELETED)
                    .is_ok()
                && session.expunge_uids(home.path(), &set).is_ok()
        };
        if !moved {
            return Ok(());
        }
        self.store.transaction("adoptDraft", |s| {
            let Some(mut fresh) = s.find::<Message>(&Query::new().equal("id", id))? else {
                return Ok(());
            };
            for uid in uids {
                s.remove_placement(&mut fresh, home.id(), *uid)?;
            }
            s.save(&mut fresh)
        })
    }

    /// Puts drafts that are in no thread into one ([`airpost_store::Store::thread_unthreaded_draft`]):
    /// the thread list shows a draft (its pencil, the thread's Drafts category) only through
    /// the thread it is in. Drafts get here from Import from Mailspring (a reply whose
    /// original has not synced, every new message), from webmail before its copy synced, or
    /// as new messages written here. On Gmail a draft with a server copy takes the thread of
    /// that copy's X-GM-THRID, which is how Gmail threaded it; one with no copy yet waits
    /// for its push unless the message it answers is here.
    pub(crate) fn thread_unthreaded_drafts(&self, session: &mut dyn ImapSession) -> Result<()> {
        if !self.account.save_drafts_to_server() {
            return Ok(());
        }
        let ids = self.store.unthreaded_draft_ids(self.account.id())?;
        if ids.is_empty() {
            return Ok(());
        }
        let busy = self.drafts_with_pending_tasks()?;
        let gmail = is_gmail(session);
        let mut copy_uid: HashMap<String, u32> = HashMap::new();
        let mut gmail_thread_of: HashMap<u32, u64> = HashMap::new();
        if gmail {
            if let Some(all) = self.role_folder("all")? {
                for id in &ids {
                    let uid = self
                        .store
                        .placements_for_message(id)?
                        .iter()
                        .filter(|p| p.folder_id == all.id() && p.remote_uid > 0)
                        .map(|p| p.remote_uid)
                        .max();
                    if let Some(uid) = uid {
                        copy_uid.insert(id.clone(), uid);
                    }
                }
                if !copy_uid.is_empty() {
                    let uids = UidSet::from_uids(copy_uid.values().copied());
                    match session.fetch_by_uid(all.path(), FetchKind::Heavy, &uids) {
                        Ok(fetched) => {
                            for m in fetched.messages {
                                gmail_thread_of.insert(m.uid, m.gmail_thread_id.unwrap_or(0));
                            }
                        }
                        Err(e) => {
                            info!(
                                "-X Could not read the Gmail threads of drafts: {}",
                                e.code.name()
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }
        for id in ids {
            if busy.contains(&id) {
                continue;
            }
            let uid = copy_uid.get(&id).copied();
            let gmail_thread_id = uid
                .and_then(|u| gmail_thread_of.get(&u))
                .copied()
                .unwrap_or(0);
            let create = !gmail || uid.is_some();
            let threaded = self.store.transaction("threadDraft", |s| {
                s.thread_unthreaded_draft(self.account.id(), &id, gmail_thread_id, create)
            })?;
            if let Some(thread_id) = threaded {
                info!("- Draft {id} is now in thread {thread_id}");
            }
        }
        Ok(())
    }

    /// Pushes the drafts that are due and returns how long until the next one is.
    pub(crate) fn push_dirty_drafts(
        &self,
        session: &mut dyn ImapSession,
    ) -> Result<Option<Duration>> {
        if !self.account.save_drafts_to_server() {
            return Ok(None);
        }
        self.queue_existing_local_drafts()?;
        let ids = self.store.dirty_draft_ids(self.account.id())?;
        if ids.is_empty() {
            return Ok(None);
        }
        let busy = self.drafts_with_pending_tasks()?;
        let now = self.store.now();
        let mut next: Option<i64> = None;
        let mut pushed = 0;
        let mut target = None;
        let sent_walk_done = airpost_sync::drafts::sent_walk_done(self.store, self.account.id())?;
        for id in ids {
            let Some(draft) = self.store.find::<Message>(&Query::new().equal("id", &id))? else {
                continue;
            };
            let Some(sync) = draft.draft_sync() else {
                continue;
            };
            if busy.contains(draft.id())
                || busy.contains(draft.header_message_id())
                || draft.data.get("pristine").and_then(Value::as_bool) == Some(true)
                || sync.held == Some(sync.rev)
            {
                continue;
            }
            // A draft from before drafts synced may be the leftover of a send whose Sent copy
            // the first walk of Sent has yet to bring in ([`Self::remove_sent_drafts`]).
            if sync.saved_at == 0 && !sent_walk_done {
                let retry = now + SENT_WALK_POLL;
                next = Some(next.map_or(retry, |n| n.min(retry)));
                continue;
            }
            let large = attachment_bytes(&draft) > DRAFT_PUSH_LARGE_BYTES;
            let due = (sync.saved_at + draft_push_quiet(large)).max(sync.next_at);
            if due > now || pushed == DRAFT_PUSHES_PER_PASS {
                next = Some(next.map_or(due, |n| n.min(due)));
                continue;
            }
            if target.is_none() {
                target = self.draft_push_target(session)?;
            }
            let Some(target) = &target else {
                info!("-X No Drafts folder to save drafts to.");
                return Ok(None);
            };
            pushed += 1;
            if let Pushed::Failed { retry_at } = self.push_draft(session, target, draft, sync)? {
                next = Some(next.map_or(retry_at, |n| n.min(retry_at)));
                break;
            }
        }
        // A second at least: a draft due now waits for the next iteration, not a busy loop.
        Ok(next.map(|at| Duration::from_secs((at - now).max(1) as u64)))
    }

    /// Drafts saved before the account synced drafts, or imported from the C++ engine, are
    /// pushed like a fresh save once, a few per iteration:
    /// those with a body or recipients, not pristine. The others are marked seen and wait
    /// for their next save.
    fn queue_existing_local_drafts(&self) -> Result<()> {
        let ids = self
            .store
            .local_drafts_without_sync_state(self.account.id())?;
        if ids.is_empty() {
            return Ok(());
        }
        let queued = self.store.transaction("queueLocalDrafts", |s| {
            let mut queued = 0;
            for id in &ids {
                let Some(mut draft) = s.find::<Message>(&Query::new().equal("id", id))? else {
                    continue;
                };
                let body: String = s
                    .conn()
                    .prepare_cached("SELECT value FROM MessageBody WHERE id = ?")?
                    .query_row([id], |r| r.get(0))
                    .unwrap_or_default();
                let has_recipients = [draft.to(), draft.cc(), draft.bcc()]
                    .iter()
                    .any(|l| !elements(l).is_empty());
                let pristine = draft.data.get("pristine").and_then(Value::as_bool) == Some(true);
                let push = !pristine && (!body.trim().is_empty() || has_recipients);
                let sync = DraftSync {
                    rev: u64::from(push),
                    ..Default::default()
                };
                queued += usize::from(push);
                draft.set_draft_sync(Some(&sync));
                s.save(&mut draft)?;
            }
            s.unsafe_erase_transaction_deltas();
            Ok(queued)
        })?;
        if queued > 0 {
            info!("{}", logmsg::draft_sync_enabled(queued));
        }
        Ok(())
    }

    /// Drafts whose message went out already: a copy with their Message-ID that is not a
    /// draft is in Sent (on Gmail, labelled Sent). They are left by a send stopped before
    /// its draft went, by this engine before the relaunch
    /// cleaned up or by the C++ engine and imported with Import from Mailspring. Pushed, one
    /// would show as a draft inside the sent thread, so instead the row goes, its Message-ID
    /// is tombstoned, and the draft copies it was pushed as are deleted; the Sent copy is
    /// another row and is never touched. A draft a queued send or destroy names is left until
    /// that task ends, and one a failed send names is left for the user (a multisend that
    /// failed part way has Sent copies of the recipients it reached).
    pub(crate) fn remove_sent_drafts(&self, session: &mut dyn ImapSession) -> Result<()> {
        if !self.account.save_drafts_to_server() {
            return Ok(());
        }
        let pairs = self
            .store
            .drafts_sharing_header_message_id(self.account.id())?;
        if pairs.is_empty() {
            return Ok(());
        }
        let mut sent = Vec::new();
        for (draft_id, message_id) in pairs {
            if sent.contains(&draft_id) {
                continue;
            }
            let Some(m) = self
                .store
                .find::<Message>(&Query::new().equal("id", &message_id))?
            else {
                continue;
            };
            if !m.is_deletion_placeholder() && m.is_sent_by_user(self.store)? {
                sent.push(draft_id);
            }
        }
        if sent.is_empty() {
            return Ok(());
        }
        let failed = drafts_named_by(
            self.store
                .failed_task_data(self.account.id(), &["SendDraftTask"])?,
        );
        for id in sent {
            let removed = self.store.transaction("removeSentDraft", |s| {
                let Some(mut draft) = s.find::<Message>(&Query::new().equal("id", &id))? else {
                    return Ok(None);
                };
                let hmid = draft.header_message_id().to_owned();
                // Read inside the write transaction, so a send queued meanwhile is seen.
                let busy = self.drafts_with_pending_tasks()?;
                if [&id, &hmid]
                    .iter()
                    .any(|k| busy.contains(*k) || failed.contains(*k))
                {
                    return Ok(None);
                }
                let mut copies: BTreeMap<String, Vec<u32>> = BTreeMap::new();
                for p in s.placements_for_message(&id)? {
                    if p.remote_uid == 0 {
                        continue;
                    }
                    let Some(folder) = s.folder_by_id(self.account.id(), &p.folder_id)? else {
                        continue;
                    };
                    if p.draft || folder.role() == "drafts" {
                        copies
                            .entry(folder.path().to_owned())
                            .or_default()
                            .push(p.remote_uid);
                    }
                }
                self.tombstone_draft_in(s, &hmid)?;
                s.remove(&mut draft)?;
                Ok(Some((hmid, copies)))
            })?;
            let Some((hmid, copies)) = removed else {
                continue;
            };
            info!(
                "{}",
                logmsg::removing_sent_draft(&hmid, copies.values().map(Vec::len).sum())
            );
            // A copy the server keeps is a tombstoned draft copy now, which the scan that
            // next fetches it queues for deletion again.
            for (path, uids) in copies {
                remove_messages_resilient(
                    session,
                    self.store,
                    self.account.id(),
                    &path,
                    &UidSet::from_uids(uids),
                )?;
            }
        }
        Ok(())
    }

    /// The ids and Message-IDs of drafts a queued send or destroy names.
    fn drafts_with_pending_tasks(&self) -> Result<HashSet<String>> {
        Ok(drafts_named_by(self.store.pending_task_data(
            self.account.id(),
            &DRAFT_ENDING_TASKS,
        )?))
    }

    fn draft_push_target(&self, session: &dyn ImapSession) -> Result<Option<PushTarget>> {
        if is_gmail(session) {
            let label = self.store.find::<Label>(
                &Query::new()
                    .equal("accountId", self.account.id())
                    .equal("role", "drafts"),
            )?;
            let all = self.role_folder("all")?;
            return Ok(label.zip(all).map(|(label, home)| PushTarget {
                append_path: Folder::from(label).path().to_owned(),
                home,
                gmail: true,
            }));
        }
        Ok(self.role_folder("drafts")?.map(|home| PushTarget {
            append_path: home.path().to_owned(),
            home,
            gmail: false,
        }))
    }

    fn push_draft(
        &self,
        session: &mut dyn ImapSession,
        target: &PushTarget,
        mut draft: Message,
        sync: DraftSync,
    ) -> Result<Pushed> {
        let hmid = draft.header_message_id().to_owned();
        if hmid.is_empty() {
            return Ok(Pushed::Skipped);
        }
        // The server copy will be keyed by its Message-ID; the row moves there first.
        let draft_id = airpost_core::ids::draft_id(self.account.id(), &hmid);
        if draft.id() != draft_id {
            let old_id = draft.id().to_owned();
            match self
                .store
                .transaction("pushDraft", |s| s.rekey_message(&old_id, &draft_id))?
            {
                Some(rekeyed) => {
                    info!("{}", logmsg::draft_rekeyed(&old_id, &draft_id));
                    draft = rekeyed;
                }
                None => {
                    info!("-X Draft {old_id} can't take the id {draft_id}, which is in use.");
                    self.hold_draft(draft.id(), sync.rev)?;
                    return Ok(Pushed::Skipped);
                }
            }
        }

        let raw = match self.draft_mime(session, &draft) {
            Ok(raw) => raw,
            Err(e) => {
                info!("{}", logmsg::draft_push_held(&hmid, &e.to_string()));
                self.hold_draft(draft.id(), sync.rev)?;
                return Ok(Pushed::Skipped);
            }
        };
        if let Some(limit) = session
            .capabilities()
            .value_of("APPENDLIMIT")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|l| *l > 0)
        {
            if raw.len() as u64 > limit {
                info!(
                    "{}",
                    logmsg::draft_over_append_limit(&hmid, raw.len(), limit)
                );
                self.hold_draft(draft.id(), sync.rev)?;
                return Ok(Pushed::Skipped);
            }
        }

        let appended = match session.append(
            &target.append_path,
            &raw,
            MessageFlags::DRAFT | MessageFlags::SEEN,
        ) {
            Ok(uid) => uid,
            Err(e) => {
                let fails = sync.fails + 1;
                let wait = draft_push_backoff(fails);
                info!("{}", logmsg::draft_push_failed(&hmid, e.code.name(), wait));
                self.store.transaction("pushDraft", |s| {
                    if let Some(mut fresh) =
                        s.find::<Message>(&Query::new().equal("id", draft.id()))?
                    {
                        let mut ds = fresh.draft_sync().unwrap_or_default();
                        ds.fails = fails;
                        ds.next_at = s.now() + wait;
                        fresh.set_draft_sync(Some(&ds));
                        s.save(&mut fresh)?;
                        s.unsafe_erase_transaction_deltas();
                    }
                    Ok(())
                })?;
                return Ok(Pushed::Failed {
                    retry_at: self.store.now() + wait,
                });
            }
        };

        let known: HashSet<u32> = self
            .store
            .placements_for_message(draft.id())?
            .iter()
            .filter(|p| p.folder_id == target.home.id())
            .map(|p| p.remote_uid)
            .collect();
        let uid = match appended.filter(|u| *u > 0 && !target.gmail) {
            Some(uid) => uid,
            // Gmail's APPENDUID is the label view's; the copy is recorded where the sync
            // worker sees it, in All Mail. Without UIDPLUS the copy is the one new UID with
            // the Message-ID (the search re-SELECTs, which Courier needs to show it).
            None => find_recent(session, target.home.path(), &hmid)
                .into_iter()
                .filter(|u| !known.contains(u))
                .max()
                .unwrap_or(0),
        };

        let gmail_labels = if target.gmail {
            vec!["\\Draft".to_owned()]
        } else {
            Vec::new()
        };
        let stale = self.store.transaction("pushDraft", |s| {
            let Some(mut fresh) = s.find::<Message>(&Query::new().equal("id", draft.id()))? else {
                return Ok(None);
            };
            if uid > 0 {
                let attrs = MessageAttributes {
                    uid,
                    unread: false,
                    starred: fresh.is_starred(),
                    draft: true,
                    labels: gmail_labels.clone(),
                    message_id: hmid.clone(),
                };
                s.upsert_placement(&mut fresh, &target.home, uid, &attrs)?;
            }
            let mut stale = Vec::new();
            for p in s.placements_for_message(fresh.id())? {
                if p.remote_uid == 0 || (p.folder_id == target.home.id() && p.remote_uid == uid) {
                    continue;
                }
                let is_drafts = s
                    .folder_by_id(self.account.id(), &p.folder_id)?
                    .is_some_and(|f| f.role() == "drafts");
                if p.draft || is_drafts {
                    stale.push(p);
                }
            }
            let mut ds = fresh.draft_sync().unwrap_or_default();
            ds.pushed_rev = ds.pushed_rev.max(sync.rev);
            ds.base = (uid > 0).then(|| DraftBase {
                f: target.home.id().to_owned(),
                u: uid,
                d: Some(draft.date()),
            });
            ds.fails = 0;
            ds.next_at = 0;
            ds.held = None;
            fresh.set_draft_sync(Some(&ds));
            s.save(&mut fresh)?;
            Ok(Some(stale))
        })?;
        let Some(stale) = stale else {
            // Sent or destroyed while it uploaded: the new copy goes too.
            if uid > 0 {
                remove_messages_resilient(
                    session,
                    self.store,
                    self.account.id(),
                    target.home.path(),
                    &UidSet::single(uid),
                )?;
            }
            return Ok(Pushed::Done);
        };
        info!("{}", logmsg::draft_pushed(&hmid, target.home.path(), uid));
        self.delete_stale_draft_copies(session, draft.id(), &stale)?;
        Ok(Pushed::Done)
    }

    /// Deletes copies a push replaced and drops their placements. The new copy is recorded
    /// first, so the draft never has none; a crash in between leaves two, which collapse
    /// onto the row and go at the next push.
    fn delete_stale_draft_copies(
        &self,
        session: &mut dyn ImapSession,
        draft_id: &str,
        stale: &[Placement],
    ) -> Result<()> {
        let mut by_folder: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for p in stale {
            by_folder
                .entry(p.folder_id.clone())
                .or_default()
                .push(p.remote_uid);
        }
        for (folder_id, uids) in by_folder {
            let Some(folder) = self.store.folder_by_id(self.account.id(), &folder_id)? else {
                continue;
            };
            let set = UidSet::from_uids(uids.iter().copied());
            info!(
                "-- Deleting {} older copies of draft {draft_id} from {}",
                uids.len(),
                folder.path()
            );
            if !remove_messages_resilient(
                session,
                self.store,
                self.account.id(),
                folder.path(),
                &set,
            )? {
                continue;
            }
            self.store.transaction("pushDraft", |s| {
                let Some(mut fresh) = s.find::<Message>(&Query::new().equal("id", draft_id))?
                else {
                    return Ok(());
                };
                for uid in &uids {
                    s.remove_placement(&mut fresh, &folder_id, *uid)?;
                }
                s.save(&mut fresh)
            })?;
        }
        Ok(())
    }

    /// Keeps revision `rev` of a draft off the server until the next save.
    fn hold_draft(&self, id: &str, rev: u64) -> Result<()> {
        self.store.transaction("pushDraft", |s| {
            if let Some(mut fresh) = s.find::<Message>(&Query::new().equal("id", id))? {
                let mut ds = fresh.draft_sync().unwrap_or_default();
                ds.held = Some(rev);
                fresh.set_draft_sync(Some(&ds));
                s.save(&mut fresh)?;
                s.unsafe_erase_transaction_deltas();
            }
            Ok(())
        })
    }

    /// The draft as MIME, as it would be sent (Bcc kept), dated when it was last saved.
    fn draft_mime(&self, session: &mut dyn ImapSession, draft: &Message) -> Result<Vec<u8>> {
        self.fetch_missing_draft_files(session, draft)?;
        let text: Option<String> = self
            .store
            .conn()
            .prepare_cached("SELECT value FROM MessageBody WHERE id = ?")?
            .query_row([draft.id()], |r| r.get(0))
            .ok();
        let plaintext = draft.data.get("plaintext").and_then(Value::as_bool) == Some(true);
        let mut message = self.outgoing_message(
            draft,
            body(plaintext, text.unwrap_or_default()),
            draft.date(),
        )?;
        message
            .extra_headers
            .push(("X-Airpost-Draft".to_owned(), "1".to_owned()));
        airpost_mime::build(&message)
    }

    /// A draft that came from the server (another device, webmail) may name files whose
    /// data was never downloaded: they are taken from its server copy, matched by
    /// Content-ID, else by filename.
    fn fetch_missing_draft_files(
        &self,
        session: &mut dyn ImapSession,
        draft: &Message,
    ) -> Result<()> {
        let root = self.files_root();
        let missing: Vec<File> = elements(draft.files())
            .into_iter()
            .map(|f| File { data: f.clone() })
            .filter(|f| f.find_in(&root).is_none())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        for copy in self.store.fetchable_copies_of_message(draft, None)? {
            let Ok(Some(bytes)) = session.fetch_message_bytes(copy.folder.path(), copy.uid) else {
                continue;
            };
            let parsed = airpost_mime::parse_message(&bytes)?;
            for file in &missing {
                let cid = file.content_id().as_str().filter(|c| !c.is_empty());
                let part = parsed
                    .attachments
                    .iter()
                    .find(|p| cid.is_some() && p.content_id.as_deref() == cid)
                    .or_else(|| {
                        parsed
                            .attachments
                            .iter()
                            .find(|p| p.filename == file.filename())
                    });
                if let Some(part) = part {
                    if let Err(e) = file.write_in(&root, &part.data) {
                        info!("-X Could not save the data of {}: {e}", file.filename());
                    }
                }
            }
            break;
        }
        Ok(())
    }
}

/// The draft ids and Message-IDs these SendDraft / DestroyDraft tasks name.
fn drafts_named_by(tasks: Vec<Value>) -> HashSet<String> {
    let mut out = HashSet::new();
    for task in tasks {
        for key in ["headerMessageId", "messageIds", "draft"] {
            match task.get(key) {
                Some(Value::String(s)) => {
                    out.insert(s.clone());
                }
                Some(Value::Array(ids)) => {
                    out.extend(ids.iter().filter_map(Value::as_str).map(str::to_owned));
                }
                Some(d @ Value::Object(_)) => {
                    for k in ["id", "hMsgId"] {
                        if let Some(s) = d.get(k).and_then(Value::as_str) {
                            out.insert(s.to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn attachment_bytes(draft: &Message) -> u64 {
    elements(draft.files())
        .into_iter()
        .filter_map(|f| f.get("size").and_then(Value::as_u64))
        .sum()
}
