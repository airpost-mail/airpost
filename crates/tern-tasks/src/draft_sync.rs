// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Server-synced drafts, the foreground worker's part (docs/drafts-sync-design.md 3.2): with
//! "Save drafts to server" on, a saved draft is APPENDed to the Drafts folder once the user
//! has stopped editing it, its new copy recorded at once, and the copies it replaces
//! deleted. A save never waits for the server: SyncbackDraftTask only bumps `_dsync.rev`,
//! and the foreground pushes after its tasks, between IDLEs.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use tern_core::constants::{
    draft_push_backoff, draft_push_quiet, DRAFT_PUSHES_PER_PASS, DRAFT_PUSH_LARGE_BYTES, FILES_DIR,
};
use tern_core::{logmsg, MessageFlags, Result, UidSet};
use tern_imap::ImapSession;
use tern_store::drafts::{DraftBase, DraftSync};
use tern_store::{File, Folder, Label, Message, MessageAttributes, Model, Placement, Query};
use tracing::info;

use crate::imap_ops::remove_messages_resilient;
use crate::json::elements;
use crate::messages::is_gmail;
use crate::send::{body, find_recent};
use crate::TaskProcessor;

/// Tasks that end a draft: it is not pushed while one is queued.
const DRAFT_ENDING_TASKS: [&str; 2] = ["SendDraftTask", "DestroyDraftTask"];

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

    /// Pushes the drafts that are due and returns how long until the next one is.
    pub(crate) fn push_dirty_drafts(
        &self,
        session: &mut dyn ImapSession,
    ) -> Result<Option<Duration>> {
        if !self.account.save_drafts_to_server() {
            return Ok(None);
        }
        let ids = self.store.dirty_draft_ids(self.account.id())?;
        if ids.is_empty() {
            return Ok(None);
        }
        let busy = self.drafts_with_pending_tasks()?;
        let now = self.store.now();
        let mut next: Option<i64> = None;
        let mut pushed = 0;
        let mut target = None;
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

    /// The ids and Message-IDs of drafts a queued send or destroy names.
    fn drafts_with_pending_tasks(&self) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        for task in self
            .store
            .pending_task_data(self.account.id(), &DRAFT_ENDING_TASKS)?
        {
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
        Ok(out)
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
        let draft_id = tern_core::ids::draft_id(self.account.id(), &hmid);
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
            .push(("X-Tern-Draft".to_owned(), "1".to_owned()));
        tern_mime::build(&message)
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
            let parsed = tern_mime::parse_message(&bytes)?;
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

fn attachment_bytes(draft: &Message) -> u64 {
    elements(draft.files())
        .into_iter()
        .filter_map(|f| f.get("size").and_then(Value::as_u64))
        .sum()
}
