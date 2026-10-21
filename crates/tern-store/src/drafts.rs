// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Store helpers for server-synced drafts (`docs/drafts-sync-design.md`): a draft's push
//! bookkeeping, and moving a draft to the id its Message-ID gives it.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tern_core::Result;

use crate::{Folder, Message, Model, Query, Store};

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
            .filter(|id| !tern_core::ids::is_deletion_placeholder_id(id))
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
            .filter(|id| !tern_core::ids::is_deletion_placeholder_id(id))
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
