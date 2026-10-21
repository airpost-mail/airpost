// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Store helpers for server-synced drafts (`docs/drafts-sync-design.md`): moving a draft to
//! the id its Message-ID gives it.

use rusqlite::params;
use serde_json::{json, Value};
use tern_core::Result;

use crate::{Message, Model, Query, Store};

impl Store {
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
