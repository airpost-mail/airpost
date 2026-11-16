// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Store helpers for server-synced drafts (`docs/drafts-sync-design.md`): a draft's push
//! bookkeeping, and moving a draft to the id its Message-ID gives it.

use airpost_core::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Folder, Message, Model, Query, Store, Thread};

/// The Message JSON key holding a draft's [`DraftSync`].
pub const DRAFT_SYNC_KEY: &str = "_dsync";

/// The server copy a draft was last pushed as or adopted from: `f` the folder id the copy
/// is recorded in (All Mail on Gmail), `u` its UID, `d` its Date header (epoch seconds),
/// which tells that copy found again at another UID (a UIDVALIDITY reset) from another
/// device's revision.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftBase {
    pub f: String,
    pub u: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<i64>,
}

/// A draft's push state, kept in its Message JSON under `_dsync` by the engine alone
/// (the copy the client echoes back is ignored). Each save bumps `rev`; a push of revision
/// `rev` sets `pushedRev` to it, so `rev > pushedRev` means the server copy is behind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftSync {
    #[serde(default)]
    pub rev: u64,
    #[serde(default, rename = "pushedRev")]
    pub pushed_rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<DraftBase>,
    /// When the client last saved the draft (epoch seconds): pushes wait for a quiet spell.
    #[serde(default, rename = "savedAt")]
    pub saved_at: i64,
    /// Consecutive failed pushes, and when the next may run.
    #[serde(default)]
    pub fails: u32,
    #[serde(default, rename = "nextAt")]
    pub next_at: i64,
    /// The revision that was too large for the server (APPENDLIMIT): it stays local until
    /// the next save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<u64>,
}

impl DraftSync {
    /// The server copy is behind the local draft.
    pub fn is_dirty(&self) -> bool {
        self.rev > self.pushed_rev
    }
}

impl Message {
    pub fn draft_sync(&self) -> Option<DraftSync> {
        self.data
            .get(DRAFT_SYNC_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn set_draft_sync(&mut self, sync: Option<&DraftSync>) {
        let Some(obj) = self.data.as_object_mut() else {
            return;
        };
        match sync.and_then(|s| serde_json::to_value(s).ok()) {
            Some(v) => {
                obj.insert(DRAFT_SYNC_KEY.into(), v);
            }
            None => {
                obj.remove(DRAFT_SYNC_KEY);
            }
        }
    }
}

impl Store {
    /// The account's drafts whose server copy is behind (`_dsync.rev > _dsync.pushedRev`),
    /// least recently saved first.
    pub fn dirty_draft_ids(&self, account_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT id FROM Message WHERE accountId = ? AND draft = 1 \
             AND json_extract(data, '$._dsync.rev') > IFNULL(json_extract(data, '$._dsync.pushedRev'), 0) \
             ORDER BY json_extract(data, '$._dsync.savedAt')",
        )?;
        let ids = stmt
            .query_map([account_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    /// The account's drafts that are in no thread.
    pub fn unthreaded_draft_ids(&self, account_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT id FROM Message WHERE accountId = ? AND draft = 1 AND IFNULL(threadId, '') = ''",
        )?;
        let ids = stmt
            .query_map([account_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids
            .into_iter()
            .filter(|id| !airpost_core::ids::is_deletion_placeholder_id(id))
            .collect())
    }

    /// Puts draft `draft_id`, which is in no thread, into one, the way the client saves a
    /// reply into the thread it answers: the account's thread of Gmail thread
    /// `gmail_thread_id` (0: none known), else the thread of the message the draft replies
    /// to or forwards (an unthreaded draft it answers is threaded first), else a thread that
    /// references either Message-ID. With none and `create`, it gets a thread of its own,
    /// which records both Message-IDs so the message it answers and its own sent copy join
    /// it when they sync. Returns the thread's id, `None` when the draft is gone, already in
    /// a thread, or left as it was. Call it inside a transaction.
    pub fn thread_unthreaded_draft(
        &self,
        account_id: &str,
        draft_id: &str,
        gmail_thread_id: u64,
        create: bool,
    ) -> Result<Option<String>> {
        self.thread_draft_at_depth(account_id, draft_id, gmail_thread_id, create, 0)
    }

    fn thread_draft_at_depth(
        &self,
        account_id: &str,
        draft_id: &str,
        gmail_thread_id: u64,
        create: bool,
        depth: usize,
    ) -> Result<Option<String>> {
        // Drafts answering drafts are rare and short; the bound only stops a cycle.
        const MAX_DRAFT_CHAIN: usize = 4;
        let Some(mut draft) = self.find::<Message>(&Query::new().equal("id", draft_id))? else {
            return Ok(None);
        };
        if !draft.is_draft() || !draft.thread_id().is_empty() {
            return Ok(None);
        }
        let answers: Vec<String> = [
            draft.reply_to_header_message_id(),
            draft.forwarded_header_message_id(),
        ]
        .into_iter()
        .filter(|h| !h.is_empty())
        .map(str::to_owned)
        .collect();

        let mut thread = None;
        if gmail_thread_id != 0 {
            thread = self.find::<Thread>(
                &Query::new()
                    .equal("accountId", account_id)
                    .equal("gThrId", gmail_thread_id.to_string()),
            )?;
        }
        if thread.is_none() && depth < MAX_DRAFT_CHAIN {
            thread = self.thread_of_answered(account_id, draft_id, &answers, create, depth)?;
        }
        if thread.is_none() {
            thread =
                self.find_thread_by_references(account_id, draft.header_message_id(), &answers)?;
        }
        let thread_id = match thread {
            Some(t) => t.id().to_owned(),
            None if create => {
                let own = airpost_core::ids::thread_id(draft_id);
                if self
                    .find::<Thread>(&Query::new().equal("id", &own))?
                    .is_none()
                {
                    let mut t =
                        Thread::create(draft_id, account_id, draft.subject(), gmail_thread_id);
                    self.save(&mut t)?;
                    self.upsert_thread_references(
                        &own,
                        account_id,
                        draft.header_message_id(),
                        &answers,
                    )?;
                }
                own
            }
            None => return Ok(None),
        };
        draft.set_thread_id(&thread_id);
        // Nothing of it is counted in that thread yet.
        draft.reset_snapshot();
        self.save(&mut draft)?;
        Ok(Some(thread_id))
    }

    /// The thread of a message with one of the Message-IDs `answers`, threading an
    /// unthreaded draft among them first.
    fn thread_of_answered(
        &self,
        account_id: &str,
        draft_id: &str,
        answers: &[String],
        create: bool,
        depth: usize,
    ) -> Result<Option<Thread>> {
        for hmid in answers {
            let answered = self.find_all::<Message>(
                &Query::new()
                    .equal("accountId", account_id)
                    .equal("headerMessageId", hmid.as_str()),
            )?;
            for m in answered.iter().filter(|m| m.id() != draft_id) {
                let thread_id = if m.thread_id().is_empty() && m.is_draft() {
                    self.thread_draft_at_depth(account_id, m.id(), 0, create, depth + 1)?
                        .unwrap_or_default()
                } else {
                    m.thread_id().to_owned()
                };
                if thread_id.is_empty() {
                    continue;
                }
                if let Some(t) = self.find::<Thread>(&Query::new().equal("id", &thread_id))? {
                    return Ok(Some(t));
                }
            }
        }
        Ok(None)
    }

    /// Whether the account has a draft waiting for its first push that was never saved
    /// since it synced drafts (`_dsync.savedAt` 0: from before, or imported).
    pub fn has_dirty_drafts_never_saved(&self, account_id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .prepare_cached(
                "SELECT 1 FROM Message WHERE accountId = ? AND draft = 1 \
                 AND json_extract(data, '$._dsync.savedAt') = 0 \
                 AND json_extract(data, '$._dsync.rev') > IFNULL(json_extract(data, '$._dsync.pushedRev'), 0)",
            )?
            .exists([account_id])?)
    }

    /// Keeps the server copies of draft `header_message_id`, sent or deleted here, from
    /// syncing back.
    pub fn add_draft_tombstone(&self, account_id: &str, header_message_id: &str) -> Result<()> {
        self.conn()
            .prepare_cached(
                "INSERT OR REPLACE INTO DraftTombstone (accountId, headerMessageId, since) VALUES (?, ?, ?)",
            )?
            .execute(params![account_id, header_message_id, self.now()])?;
        Ok(())
    }

    /// The draft is being written again (a send undone, a draft re-created).
    pub fn clear_draft_tombstone(&self, account_id: &str, header_message_id: &str) -> Result<()> {
        self.conn()
            .prepare_cached(
                "DELETE FROM DraftTombstone WHERE accountId = ? AND headerMessageId = ?",
            )?
            .execute(params![account_id, header_message_id])?;
        Ok(())
    }

    pub fn is_draft_tombstoned(&self, account_id: &str, header_message_id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .prepare_cached(
                "SELECT 1 FROM DraftTombstone WHERE accountId = ? AND headerMessageId = ?",
            )?
            .exists(params![account_id, header_message_id])?)
    }

    pub fn expire_draft_tombstones(&self, account_id: &str, before: i64) -> Result<()> {
        self.conn()
            .prepare_cached("DELETE FROM DraftTombstone WHERE accountId = ? AND since < ?")?
            .execute(params![account_id, before])?;
        Ok(())
    }

    /// The folder synced drafts live in: the Drafts folder, or All Mail where Drafts is a
    /// label (Gmail).
    pub fn draft_home_folder(&self, account_id: &str) -> Result<Option<Folder>> {
        for role in ["drafts", "all"] {
            let f = self.find::<Folder>(
                &Query::new()
                    .equal("accountId", account_id)
                    .equal("role", role),
            )?;
            if f.is_some() {
                return Ok(f);
            }
        }
        Ok(None)
    }

    /// The account's drafts with a copy in `home_folder_id` that their `_dsync.base` does not
    /// account for: a copy at a higher UID than the base (another device's revision), or any
    /// copy of a draft with no base yet.
    pub fn drafts_with_unreconciled_copies(
        &self,
        account_id: &str,
        home_folder_id: &str,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT DISTINCT m.id FROM Message m JOIN MessageFolder mf ON mf.messageId = m.id \
             WHERE m.accountId = ? AND m.draft = 1 AND mf.folderId = ? AND mf.remoteUID > 0 \
             AND (json_extract(m.data, '$._dsync.base') IS NULL \
                  OR json_extract(m.data, '$._dsync.base.f') != mf.folderId \
                  OR mf.remoteUID > json_extract(m.data, '$._dsync.base.u'))",
        )?;
        let ids = stmt
            .query_map(params![account_id, home_folder_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    /// The account's drafts that exist only here (no server copy) and have no `_dsync`:
    /// saved before "Save drafts to server" was on, or imported from the C++ engine.
    pub fn local_drafts_without_sync_state(&self, account_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT id FROM Message WHERE accountId = ? AND draft = 1 \
             AND json_extract(data, '$._dsync') IS NULL \
             AND NOT EXISTS (SELECT 1 FROM MessageFolder mf WHERE mf.messageId = Message.id AND mf.remoteUID > 0)",
        )?;
        let ids = stmt
            .query_map([account_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids
            .into_iter()
            .filter(|id| !airpost_core::ids::is_deletion_placeholder_id(id))
            .collect())
    }

    /// The data of the account's tasks of these classes still to run (`local` or `remote`).
    pub fn pending_task_data(&self, account_id: &str, classes: &[&str]) -> Result<Vec<Value>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT data FROM Task WHERE accountId = ? AND status IN ('local', 'remote')",
        )?;
        let rows = stmt
            .query_map([account_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows
            .iter()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .filter(|t| {
                t.get("__cls")
                    .and_then(Value::as_str)
                    .is_some_and(|c| classes.contains(&c))
            })
            .collect())
    }

    /// The data of the account's tasks of these classes that ended with an error.
    pub fn failed_task_data(&self, account_id: &str, classes: &[&str]) -> Result<Vec<Value>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT data FROM Task WHERE accountId = ? AND status = 'complete' \
             AND json_type(data, '$.error') IS NOT NULL AND json_type(data, '$.error') != 'null'",
        )?;
        let rows = stmt
            .query_map([account_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows
            .iter()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .filter(|t| {
                t.get("__cls")
                    .and_then(Value::as_str)
                    .is_some_and(|c| classes.contains(&c))
            })
            .collect())
    }

    /// (draft id, message id) for each of the account's drafts whose Message-ID a message
    /// that is not a draft also has.
    pub fn drafts_sharing_header_message_id(
        &self,
        account_id: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT d.id, m.id FROM Message d JOIN Message m ON m.headerMessageId = d.headerMessageId \
             WHERE d.accountId = ? AND d.draft = 1 AND d.headerMessageId != '' \
             AND m.accountId = d.accountId AND m.draft = 0 AND m.id != d.id",
        )?;
        let pairs = stmt
            .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<(String, String)>>>()?;
        Ok(pairs)
    }

    /// The account's draft rows with Message-ID `header_message_id` other than `except`,
    /// deletion placeholders aside (they carry a `deleted-…` Message-ID of their own).
    pub fn draft_ids_with_header_message_id(
        &self,
        account_id: &str,
        header_message_id: &str,
        except: &str,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT id FROM Message WHERE accountId = ? AND headerMessageId = ? AND draft = 1 AND id != ?",
        )?;
        let ids = stmt
            .query_map(params![account_id, header_message_id, except], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids
            .into_iter()
            .filter(|id| !airpost_core::ids::is_deletion_placeholder_id(id))
            .collect())
    }

    /// Moves message `old_id` to `new_id` with its copies, body, plugin metadata and files,
    /// and returns it as saved under the new id. The client sees the old row unpersisted and
    /// the new one persisted; the composer, which keys drafts by Message-ID, keeps editing it.
    /// `None` when there is no `old_id` or a row already holds `new_id`. Call it inside a
    /// transaction.
    pub fn rekey_message(&self, old_id: &str, new_id: &str) -> Result<Option<Message>> {
        let Some(mut old) = self.find::<Message>(&Query::new().equal("id", old_id))? else {
            return Ok(None);
        };
        if self
            .find::<Message>(&Query::new().equal("id", new_id))?
            .is_some()
        {
            return Ok(None);
        }
        let conn = self.conn();
        conn.prepare_cached("UPDATE MessageFolder SET messageId = ? WHERE messageId = ?")?
            .execute(params![new_id, old_id])?;
        conn.prepare_cached("DELETE FROM MessageOrphan WHERE messageId = ?")?
            .execute([old_id])?;
        conn.prepare_cached("UPDATE MessageBody SET id = ? WHERE id = ?")?
            .execute(params![new_id, old_id])?;
        conn.prepare_cached("UPDATE ModelPluginMetadata SET id = ? WHERE id = ?")?
            .execute(params![new_id, old_id])?;
        conn.prepare_cached(
            "UPDATE File SET data = json_set(data, '$.messageId', ?) WHERE accountId = ? AND json_extract(data, '$.messageId') = ?",
        )?
        .execute(params![new_id, old.account_id(), old_id])?;

        let mut data = old.data.clone();
        data["id"] = json!(new_id);
        // Version 0: counted into the thread as a new message, while removing the old row
        // takes its own contribution out, so the thread's counters stay even.
        data["v"] = json!(0);
        if let Some(files) = data.get_mut("files").and_then(Value::as_array_mut) {
            for f in files {
                if f.get("messageId").and_then(Value::as_str) == Some(old_id) {
                    f["messageId"] = json!(new_id);
                }
            }
        }
        let mut new = Message::new(data);
        new.set_placements_changed(true);
        self.save(&mut new)?;
        self.remove(&mut old)?;
        Ok(Some(new))
    }
}
