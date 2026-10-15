//! Message placements: one `MessageFolder` row per copy of a message on the server, and the
//! store helpers that are the only writers of those rows. Derived from Mailspring-Sync
//! `MailSync/Placement.*` and the placement section of `MailSync/MailStore.*`, Copyright ©
//! 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! The rules are in "Message Identity and Placements" in legacy/mailsync-cpp/CLAUDE.md. In
//! short: the table is canonical; every helper that takes a [`Message`] marks it, and the
//! caller saves it, which rebuilds its `folders` snapshot and derived flags from the rows
//! ([`Store::refresh_message_from_placements`]). Bulk helpers touch rows only and return the
//! ids of the messages they affected so the caller can load, refresh and save those.
//! `MessageOrphan` is exact once the transaction commits: a message is listed iff it has no
//! `MessageFolder` row.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use ms_core::flags::placement_flag_bits;
use ms_core::{Result, UidSet};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde_json::{json, Map, Value};

use crate::{Folder, Message, Model, Store};

/// A `MessageFolder` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub rowid: i64,
    pub account_id: String,
    pub message_id: String,
    pub folder_id: String,
    /// 0 = not on the server (a local draft, or after a UIDVALIDITY reset).
    pub remote_uid: u32,
    pub unread: bool,
    pub starred: bool,
    pub draft: bool,
    /// X-GM-LABELS of this copy (Gmail only).
    pub labels: Vec<String>,
    /// Set while an optimistic move is in flight.
    pub pending_folder_id: Option<String>,
}

impl Placement {
    /// The bits stored per folder in the Message JSON's `folders` map.
    pub fn flag_bits(&self) -> i64 {
        placement_flag_bits(self.unread, self.starred, self.draft)
    }

    /// The folder the client sees this copy in: the move target while a move is pending.
    pub fn reported_folder_id(&self) -> &str {
        self.pending_folder_id.as_deref().unwrap_or(&self.folder_id)
    }

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Placement> {
        let labels: Option<String> = row.get("remoteXGMLabels")?;
        Ok(Placement {
            rowid: row.get("rowid")?,
            account_id: row.get("accountId")?,
            message_id: row.get("messageId")?,
            folder_id: row.get("folderId")?,
            remote_uid: row.get::<_, i64>("remoteUID")? as u32,
            unread: row.get::<_, i64>("unread")? != 0,
            starred: row.get::<_, i64>("starred")? != 0,
            draft: row.get::<_, i64>("draft")? != 0,
            labels: parse_labels(labels.as_deref().unwrap_or("")),
            pending_folder_id: row
                .get::<_, Option<String>>("pendingFolderId")?
                .filter(|p| !p.is_empty()),
        })
    }
}

/// `remoteXGMLabels` text as a list. Never fails: a cached statement left mid-step by an
/// error would pin the WAL read snapshot.
fn parse_labels(text: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(a)) => a
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// The attributes of one server copy as fetched (`MessageAttributes`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageAttributes {
    pub uid: u32,
    pub unread: bool,
    pub starred: bool,
    pub draft: bool,
    /// Sorted X-GM-LABELS, without `\Trash` and `\Spam` (Gmail exposes those as folders).
    pub labels: Vec<String>,
    /// The row's message, set only by [`Store::fetch_messages_attributes_in_range`];
    /// attributes read from the server carry no id.
    pub message_id: String,
}

impl MessageAttributes {
    /// `MessageAttributesForMessage`: from the FLAGS and X-GM-LABELS of a fetch. A copy is a
    /// draft when flagged `\Draft` or labelled `\Draft`, unless it is in Trash or Spam.
    pub fn from_flags(
        uid: u32,
        flags: ms_core::MessageFlags,
        gmail_labels: Option<&[String]>,
    ) -> Self {
        use ms_core::MessageFlags;
        let mut labels = Vec::new();
        let (mut draft_label, mut trash_or_spam) = (false, false);
        for l in gmail_labels.unwrap_or(&[]) {
            if l == "\\Trash" || l == "\\Spam" {
                trash_or_spam = true;
                continue;
            }
            if l == "\\Draft" {
                draft_label = true;
            }
            labels.push(l.clone());
        }
        labels.sort();
        MessageAttributes {
            uid,
            unread: !flags.contains(MessageFlags::SEEN),
            starred: flags.contains(MessageFlags::FLAGGED),
            draft: (flags.contains(MessageFlags::DRAFT) || draft_label) && !trash_or_spam,
            labels,
            message_id: String::new(),
        }
    }

    /// `MessageAttributesMatch`: everything but `message_id`.
    pub fn matches(&self, other: &MessageAttributes) -> bool {
        self.unread == other.unread
            && self.starred == other.starred
            && self.draft == other.draft
            && self.uid == other.uid
            && self.labels == other.labels
    }
}

/// A copy of a message the server can be asked to FETCH: a placement with a UID in a
/// folder that still exists.
#[derive(Clone, Debug)]
pub struct FetchableCopy {
    pub folder: Rc<Folder>,
    pub uid: u32,
}

/// Ids or UIDs bound per bulk statement, well under SQLite's parameter limit.
const BULK_CHUNK: usize = 500;

const PLACEMENT_COLUMNS: &str =
    "rowid, accountId, messageId, folderId, remoteUID, unread, starred, draft, remoteXGMLabels, pendingFolderId";

fn qmarks(n: usize) -> String {
    vec!["?"; n].join(",")
}

fn labels_text(labels: &[String]) -> String {
    serde_json::to_string(labels).unwrap_or_else(|_| "[]".into())
}

/// Distinct ids in first-seen order, from a statement whose first column is a message id
/// (`_collectMessageIds`).
fn collect_ids(mut rows: rusqlite::Rows<'_>) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    Ok(out)
}

impl Store {
    pub fn placements_for_message(&self, message_id: &str) -> Result<Vec<Placement>> {
        let mut stmt = self.conn().prepare_cached(&format!(
            "SELECT {PLACEMENT_COLUMNS} FROM MessageFolder WHERE messageId = ? ORDER BY rowid"
        ))?;
        let rows = stmt
            .query_map([message_id], Placement::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The copies of `msg` that can be fetched, cheapest first: the folder the caller has
    /// selected, then ordinary folders, then Spam and Trash, whose copies the server may
    /// purge at any time. Callers try the next copy when a FETCH fails, since a copy can be
    /// expunged after the last scan.
    pub fn fetchable_copies_of_message(
        &self,
        msg: &Message,
        preferred_folder: Option<&Folder>,
    ) -> Result<Vec<FetchableCopy>> {
        let mut ranked = Vec::new();
        for p in self.placements_for_message(msg.id())? {
            if p.remote_uid == 0 {
                continue;
            }
            let Some(folder) = self.folder_by_id(msg.account_id(), &p.folder_id)? else {
                continue;
            };
            let rank = if preferred_folder.is_some_and(|pf| pf.id() == folder.id()) {
                0
            } else if matches!(folder.role(), "spam" | "trash") {
                2
            } else {
                1
            };
            ranked.push((
                rank,
                FetchableCopy {
                    folder,
                    uid: p.remote_uid,
                },
            ));
        }
        ranked.sort_by_key(|(rank, _)| *rank);
        Ok(ranked.into_iter().map(|(_, c)| c).collect())
    }

    /// Rebuilds the message's `folders` snapshot and derived flags from its rows and clears
    /// [`Message::placements_changed`]. `Message::before_save` calls it when a helper marked
    /// the message; call it directly only to inspect the result before deciding to save.
    ///
    /// - `folders` lists copies under the folder the client should see them in (the pending
    ///   destination during an optimistic move), OR-ing bits when one folder holds several.
    /// - unread / starred / draft are OR'd over the rows; draft is also true when a copy is
    ///   in the Drafts folder. `labels` is the sorted union of the rows' X-GM-LABELS.
    /// - A message with no rows keeps its flags and labels and is recorded in
    ///   `MessageOrphan`: it is in transit to a folder not scanned yet, or about to be swept.
    ///   Any row clears the record, which is what keeps a moved message's id.
    pub fn refresh_message_from_placements(&self, msg: &mut Message) -> Result<()> {
        msg.set_placements_changed(false);
        let rows = self.placements_for_message(msg.id())?;

        let mut folders = Map::new();
        let mut labels = BTreeSet::new();
        let (mut unread, mut starred, mut draft) = (false, false, false);
        for p in &rows {
            unread |= p.unread;
            starred |= p.starred;
            draft |= p.draft;
            let key = p.reported_folder_id().to_owned();
            let existing = folders.get(&key).and_then(Value::as_i64).unwrap_or(0);
            folders.insert(key, json!(existing | p.flag_bits()));
            labels.extend(p.labels.iter().cloned());
            if self
                .folder_by_id(msg.account_id(), &p.folder_id)?
                .is_some_and(|f| f.role() == "drafts")
            {
                draft = true;
            }
        }
        msg.set_folders(folders);

        if rows.is_empty() {
            // Note: OR IGNORE keeps the earliest `since`. OR REPLACE would restart the grace
            // period on every rebuild, so an orphan refreshed every pass would never expire.
            self.conn()
                .prepare_cached("INSERT OR IGNORE INTO MessageOrphan (messageId, accountId, since) VALUES (?, ?, ?)")?
                .execute(params![msg.id(), msg.account_id(), self.now()])?;
            return Ok(());
        }
        self.conn()
            .prepare_cached("DELETE FROM MessageOrphan WHERE messageId = ?")?
            .execute([msg.id()])?;
        msg.set_unread(unread);
        msg.set_starred(starred);
        msg.set_draft(draft);
        msg.data["labels"] = json!(labels.into_iter().collect::<Vec<_>>());
        Ok(())
    }

    /// Records one physical copy at (folder, uid) with the flags the server reported, and
    /// returns the id of a different message that held (folder, uid) and lost it.
    ///
    /// - A row already at (folder, uid) is refreshed in place. If it belonged to another
    ///   message (a UID the server reused without a UIDVALIDITY change), that message loses
    ///   the copy and is recorded in `MessageOrphan` if it has no other.
    /// - A row for this message in this folder at UID 0 (a local draft, or a copy waiting
    ///   for a UIDVALIDITY rebuild to relink it) is replaced by the real row, so the
    ///   rebuild converges instead of leaving both.
    ///
    /// An existing row's pending move marker is kept: a scan can run between a task's MOVE
    /// and its commit, and only the commit or an abandoned task settles it. A row taken over
    /// from another message loses it, since that move was the other message's.
    pub fn upsert_placement(
        &self,
        msg: &mut Message,
        folder: &Folder,
        uid: u32,
        attrs: &MessageAttributes,
    ) -> Result<Option<String>> {
        let conn = self.conn();
        let mut displaced = None;
        if uid > 0 {
            let holder: Option<String> = conn
                .prepare_cached(
                    "SELECT messageId FROM MessageFolder WHERE accountId = ? AND folderId = ? AND remoteUID = ? AND remoteUID > 0",
                )?
                .query_row(params![msg.account_id(), folder.id(), uid], |r| r.get(0))
                .optional()?;
            displaced = holder.filter(|h| h != msg.id());
        } else {
            // UID 0 rows are outside the unique index, so refresh one by hand rather than
            // accumulating a row per save of a local draft.
            let updated = conn
                .prepare_cached(
                    "UPDATE MessageFolder SET unread = ?, starred = ?, draft = ?, remoteXGMLabels = ? WHERE messageId = ? AND folderId = ? AND remoteUID = 0",
                )?
                .execute(params![
                    attrs.unread,
                    attrs.starred,
                    attrs.draft,
                    labels_text(&attrs.labels),
                    msg.id(),
                    folder.id()
                ])?;
            if updated > 0 {
                msg.set_placements_changed(true);
                return Ok(None);
            }
        }

        conn.prepare_cached(
            "INSERT INTO MessageFolder (accountId, messageId, folderId, remoteUID, unread, starred, draft, remoteXGMLabels, pendingFolderId) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL) \
             ON CONFLICT (accountId, folderId, remoteUID) WHERE remoteUID > 0 DO UPDATE SET \
             messageId = excluded.messageId, unread = excluded.unread, starred = excluded.starred, draft = excluded.draft, \
             remoteXGMLabels = excluded.remoteXGMLabels, \
             pendingFolderId = CASE WHEN messageId = excluded.messageId THEN pendingFolderId END",
        )?
        .execute(params![
            msg.account_id(),
            msg.id(),
            folder.id(),
            uid,
            attrs.unread,
            attrs.starred,
            attrs.draft,
            labels_text(&attrs.labels)
        ])?;

        if let Some(d) = &displaced {
            self.record_orphans_among(std::slice::from_ref(d))?;
        }
        if uid > 0 {
            conn.prepare_cached(
                "DELETE FROM MessageFolder WHERE messageId = ? AND folderId = ? AND remoteUID = 0",
            )?
            .execute(params![msg.id(), folder.id()])?;
        }
        msg.set_placements_changed(true);
        Ok(displaced)
    }

    /// Gmail keeps one placement per message: All Mail, Spam and Trash are mutually
    /// exclusive on the server, and a message the send path filed under a Label is only
    /// there until All Mail reports it. UID 0 rows are local drafts and stay.
    pub fn remove_placements_outside_folder(
        &self,
        msg: &mut Message,
        folder_id: &str,
    ) -> Result<()> {
        self.conn()
            .prepare_cached(
                "DELETE FROM MessageFolder WHERE messageId = ? AND folderId != ? AND remoteUID > 0",
            )?
            .execute(params![msg.id(), folder_id])?;
        msg.set_placements_changed(true);
        Ok(())
    }

    /// A client flag change fans out to every copy; otherwise the next scan of an untouched
    /// copy re-derives the old value. Each flag is written on its own so copies that
    /// disagree on the other flag keep their own value.
    pub fn set_placement_unread(&self, msg: &mut Message, unread: bool) -> Result<()> {
        self.conn()
            .prepare_cached("UPDATE MessageFolder SET unread = ? WHERE messageId = ?")?
            .execute(params![unread, msg.id()])?;
        msg.set_placements_changed(true);
        Ok(())
    }

    pub fn set_placement_starred(&self, msg: &mut Message, starred: bool) -> Result<()> {
        self.conn()
            .prepare_cached("UPDATE MessageFolder SET starred = ? WHERE messageId = ?")?
            .execute(params![starred, msg.id()])?;
        msg.set_placements_changed(true);
        Ok(())
    }

    pub fn set_placement_labels(&self, msg: &mut Message, labels: &[String]) -> Result<()> {
        self.conn()
            .prepare_cached("UPDATE MessageFolder SET remoteXGMLabels = ? WHERE messageId = ?")?
            .execute(params![labels_text(labels), msg.id()])?;
        msg.set_placements_changed(true);
        Ok(())
    }

    /// Optimistic move of one copy: the row keeps its server folder and UID so the remote
    /// phase can address it, and the snapshot reports it under the destination at once.
    /// Keyed by (folder, UID) because two copies in one folder can be bound for different
    /// folders when an undo spreads them back over their sources.
    pub fn begin_placement_move(
        &self,
        msg: &mut Message,
        from_folder_id: &str,
        uid: u32,
        to_folder_id: &str,
    ) -> Result<()> {
        self.conn()
            .prepare_cached(
                "UPDATE MessageFolder SET pendingFolderId = ? WHERE messageId = ? AND folderId = ? AND remoteUID = ?",
            )?
            .execute(params![to_folder_id, msg.id(), from_folder_id, uid])?;
        msg.set_placements_changed(true);
        Ok(())
    }

    /// Records that the copy at (from, old_uid) now sits at (to, new_uid), and returns the
    /// id of a different message displaced at the destination. The destination row may
    /// already exist, because the destination's scan can run between the MOVE and this
    /// commit:
    ///
    /// - held by this message: the scan recorded the moved copy, and the source row is
    ///   deleted since both describe one server copy;
    /// - held by another message: a stale row for a UID the server has reassigned. It is
    ///   deleted so the unique (folder, UID) index admits ours, and that message is
    ///   recorded as after [`Store::upsert_placement`].
    ///
    /// A move whose source and destination are the same copy only clears the pending
    /// marker. It is handled before the holder lookup, which would otherwise find this very
    /// row, take it for the destination the scan recorded, and delete it as the source.
    pub fn commit_placement_move(
        &self,
        msg: &mut Message,
        from_folder_id: &str,
        old_uid: u32,
        to_folder_id: &str,
        new_uid: u32,
    ) -> Result<Option<String>> {
        let conn = self.conn();
        if from_folder_id == to_folder_id && old_uid == new_uid {
            conn.prepare_cached(
                "UPDATE MessageFolder SET pendingFolderId = NULL WHERE messageId = ? AND folderId = ? AND remoteUID = ?",
            )?
            .execute(params![msg.id(), from_folder_id, old_uid])?;
            msg.set_placements_changed(true);
            return Ok(None);
        }

        let mut holder = None;
        if new_uid > 0 {
            holder = conn
                .prepare_cached(
                    "SELECT messageId FROM MessageFolder WHERE accountId = ? AND folderId = ? AND remoteUID = ? AND remoteUID > 0",
                )?
                .query_row(params![msg.account_id(), to_folder_id, new_uid], |r| r.get::<_, String>(0))
                .optional()?;
        }

        if holder.as_deref() == Some(msg.id()) {
            self.remove_placement(msg, from_folder_id, old_uid)?;
            return Ok(None);
        }
        if let Some(h) = &holder {
            conn.prepare_cached(
                "DELETE FROM MessageFolder WHERE accountId = ? AND folderId = ? AND remoteUID = ?",
            )?
            .execute(params![msg.account_id(), to_folder_id, new_uid])?;
            self.record_orphans_among(std::slice::from_ref(h))?;
        }
        conn.prepare_cached(
            "UPDATE MessageFolder SET folderId = ?, remoteUID = ?, pendingFolderId = NULL WHERE messageId = ? AND folderId = ? AND remoteUID = ?",
        )?
        .execute(params![to_folder_id, new_uid, msg.id(), from_folder_id, old_uid])?;
        msg.set_placements_changed(true);
        Ok(holder)
    }

    /// A move whose server operation failed: the copy stays where the server has it. A
    /// marker towards another folder belongs to a later task and is left for that task.
    pub fn abandon_placement_move(
        &self,
        msg: &mut Message,
        folder_id: &str,
        uid: u32,
        to_folder_id: &str,
    ) -> Result<()> {
        let changed = self
            .conn()
            .prepare_cached(
                "UPDATE MessageFolder SET pendingFolderId = NULL WHERE messageId = ? AND folderId = ? AND remoteUID = ? AND pendingFolderId = ?",
            )?
            .execute(params![msg.id(), folder_id, uid, to_folder_id])?;
        if changed > 0 {
            msg.set_placements_changed(true);
        }
        Ok(())
    }

    pub fn remove_placement(&self, msg: &mut Message, folder_id: &str, uid: u32) -> Result<()> {
        self.conn()
            .prepare_cached(
                "DELETE FROM MessageFolder WHERE messageId = ? AND folderId = ? AND remoteUID = ?",
            )?
            .execute(params![msg.id(), folder_id, uid])?;
        msg.set_placements_changed(true);
        Ok(())
    }

    // -- Bulk helpers: rows only, no Message loaded. Every one that deletes rows records
    // the messages it left with none in MessageOrphan in the same transaction, because the
    // caller catches the snapshots up in later transactions and nothing but MessageOrphan
    // leads back to a message that has no row. UID 0 rows (local drafts, UIDVALIDITY
    // resets) never match a range because the server never reported them.

    fn record_orphans_among(&self, message_ids: &[String]) -> Result<()> {
        let now = self.now();
        for chunk in message_ids.chunks(BULK_CHUNK) {
            let sql = format!(
                "INSERT OR IGNORE INTO MessageOrphan (messageId, accountId, since) \
                 SELECT id, accountId, ? FROM Message WHERE id IN ({}) \
                 AND NOT EXISTS (SELECT 1 FROM MessageFolder WHERE MessageFolder.messageId = Message.id)",
                qmarks(chunk.len())
            );
            let mut binds: Vec<SqlValue> = vec![SqlValue::Integer(now)];
            binds.extend(chunk.iter().map(|id| SqlValue::Text(id.clone())));
            self.conn()
                .prepare(&sql)?
                .execute(params_from_iter(binds))?;
        }
        Ok(())
    }

    /// Deletes the placements at `uids` in `folder`; returns the affected message ids.
    pub fn delete_vanished_placements(&self, folder: &Folder, uids: &[u32]) -> Result<Vec<String>> {
        let mut affected: Vec<String> = Vec::new();
        for chunk in uids.chunks(BULK_CHUNK) {
            let sql = format!(
                "DELETE FROM MessageFolder WHERE accountId = ? AND folderId = ? AND remoteUID > 0 \
                 AND remoteUID IN ({}) RETURNING messageId",
                qmarks(chunk.len())
            );
            let mut binds: Vec<SqlValue> = vec![
                SqlValue::Text(folder.account_id().to_owned()),
                SqlValue::Text(folder.id().to_owned()),
            ];
            binds.extend(chunk.iter().map(|u| SqlValue::Integer(*u as i64)));
            let mut stmt = self.conn().prepare(&sql)?;
            for id in collect_ids(stmt.query(params_from_iter(binds))?)? {
                if !affected.contains(&id) {
                    affected.push(id);
                }
            }
        }
        self.record_orphans_among(&affected)?;
        Ok(affected)
    }

    /// [`Store::delete_vanished_placements`] for a UID set that may be open-ended (`12:*`,
    /// as VANISHED reports), so it cannot always be expanded to a list.
    pub fn delete_vanished_placements_in(
        &self,
        folder: &Folder,
        uids: &UidSet,
    ) -> Result<Vec<String>> {
        let mut affected: Vec<String> = Vec::new();
        let mut stmt = self.conn().prepare_cached(
            "DELETE FROM MessageFolder WHERE accountId = ? AND folderId = ? AND remoteUID > 0 \
             AND remoteUID >= ? AND remoteUID <= ? RETURNING messageId",
        )?;
        for r in uids.ranges() {
            let end = r.end.map_or(i64::MAX, i64::from);
            let rows = stmt.query(params![folder.account_id(), folder.id(), r.start, end])?;
            for id in collect_ids(rows)? {
                if !affected.contains(&id) {
                    affected.push(id);
                }
            }
        }
        drop(stmt);
        self.record_orphans_among(&affected)?;
        Ok(affected)
    }

    /// UIDVALIDITY changed: every UID in the folder is meaningless. Rows stay at UID 0
    /// (still visible, still counted) until the rebuild assigns new UIDs. Nothing to emit.
    /// The `remoteUID > 0` term lets the partial `MessageFolderUIDIndex` serve this.
    pub fn reset_placement_uids(&self, folder: &Folder) -> Result<()> {
        self.conn()
            .prepare_cached("UPDATE MessageFolder SET remoteUID = 0 WHERE accountId = ? AND folderId = ? AND remoteUID > 0")?
            .execute(params![folder.account_id(), folder.id()])?;
        Ok(())
    }

    /// After a UIDVALIDITY rebuild has visited every UID in the folder, a row still at UID 0
    /// is a copy the server no longer has. Draft rows are exempt: a local draft sits at
    /// UID 0 by design until it is sent.
    pub fn delete_unassigned_placements(&self, folder: &Folder) -> Result<Vec<String>> {
        let affected = {
            let mut stmt = self.conn().prepare_cached(
                "DELETE FROM MessageFolder WHERE accountId = ? AND folderId = ? AND remoteUID = 0 AND draft = 0 RETURNING messageId",
            )?;
            let ids = collect_ids(stmt.query(params![folder.account_id(), folder.id()])?)?;
            ids
        };
        self.record_orphans_among(&affected)?;
        Ok(affected)
    }

    /// Messages orphaned (no placement) since before `before` (epoch seconds).
    pub fn orphan_message_ids_before(&self, account_id: &str, before: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT messageId FROM MessageOrphan WHERE accountId = ? AND since < ?",
        )?;
        let ids = collect_ids(stmt.query(params![account_id, before])?)?;
        Ok(ids)
    }

    /// The subset of `among` (at most a few hundred ids) still orphaned since before `before`.
    pub fn orphan_message_ids_before_among(
        &self,
        account_id: &str,
        before: i64,
        among: &[String],
    ) -> Result<Vec<String>> {
        if among.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT messageId FROM MessageOrphan WHERE accountId = ? AND since < ? AND messageId IN ({})",
            qmarks(among.len())
        );
        let mut binds = vec![
            SqlValue::Text(account_id.to_owned()),
            SqlValue::Integer(before),
        ];
        binds.extend(among.iter().map(|id| SqlValue::Text(id.clone())));
        let mut stmt = self.conn().prepare(&sql)?;
        let ids = collect_ids(stmt.query(params_from_iter(binds))?)?;
        Ok(ids)
    }

    /// Called from `Message::after_remove`, so the orphan record goes with the message.
    pub fn delete_placements_for_message(&self, message_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.prepare_cached("DELETE FROM MessageFolder WHERE messageId = ?")?
            .execute([message_id])?;
        conn.prepare_cached("DELETE FROM MessageOrphan WHERE messageId = ?")?
            .execute([message_id])?;
        Ok(())
    }

    /// The ids a folder's deletion affects, including messages pointed at it only by a move
    /// still in flight, so the caller can detach them in bounded transactions.
    pub fn message_ids_with_placements_in_folder(&self, folder_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT DISTINCT messageId FROM MessageFolder WHERE folderId = ? OR pendingFolderId = ?",
        )?;
        let ids = collect_ids(stmt.query(params![folder_id, folder_id])?)?;
        Ok(ids)
    }

    /// Detaches `message_ids` from a folder being deleted: moves towards it are abandoned
    /// and its rows removed.
    pub fn delete_placements_for_folder_among(
        &self,
        folder_id: &str,
        message_ids: &[String],
    ) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let in_list = format!(" AND messageId IN ({})", qmarks(message_ids.len()));
        let mut binds = vec![SqlValue::Text(folder_id.to_owned())];
        binds.extend(message_ids.iter().map(|id| SqlValue::Text(id.clone())));
        for sql in [
            format!("UPDATE MessageFolder SET pendingFolderId = NULL WHERE pendingFolderId = ?{in_list}"),
            format!("DELETE FROM MessageFolder WHERE folderId = ?{in_list}"),
        ] {
            self.conn().prepare(&sql)?.execute(params_from_iter(binds.iter()))?;
        }
        self.record_orphans_among(message_ids)
    }

    /// A move still in flight towards the deleted folder is abandoned: the copy stays where
    /// the server has it. Called in the transaction that removes the folder; the caller
    /// refreshes the returned messages once it commits.
    pub fn delete_placements_for_folder(&self, folder_id: &str) -> Result<Vec<String>> {
        let mut affected = {
            let mut stmt = self.conn().prepare_cached(
                "UPDATE MessageFolder SET pendingFolderId = NULL WHERE pendingFolderId = ? RETURNING messageId",
            )?;
            let ids = collect_ids(stmt.query([folder_id])?)?;
            ids
        };
        let removed = {
            let mut stmt = self.conn().prepare_cached(
                "DELETE FROM MessageFolder WHERE folderId = ? RETURNING messageId",
            )?;
            let ids = collect_ids(stmt.query([folder_id])?)?;
            ids
        };
        for id in removed {
            if !affected.contains(&id) {
                affected.push(id);
            }
        }
        self.record_orphans_among(&affected)?;
        Ok(affected)
    }

    /// UID → attributes of the placements in `folder` within `uids` (open-ended allowed).
    /// Served from `MessageFolder` so the fat Message rows are never read; the explicit
    /// `remoteUID > 0` term lets the planner use the partial `MessageFolderUIDIndex`.
    pub fn fetch_messages_attributes_in_range(
        &self,
        uids: &UidSet,
        folder: &Folder,
    ) -> Result<HashMap<u32, MessageAttributes>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT remoteUID, messageId, unread, starred, draft, remoteXGMLabels FROM MessageFolder \
             WHERE accountId = ? AND folderId = ? AND remoteUID >= ? AND remoteUID <= ? AND remoteUID > 0",
        )?;
        let mut out = HashMap::new();
        for r in uids.ranges() {
            let end = r.end.map_or(i64::MAX, i64::from);
            let mut rows = stmt.query(params![folder.account_id(), folder.id(), r.start, end])?;
            while let Some(row) = rows.next()? {
                let uid = row.get::<_, i64>(0)? as u32;
                let labels: Option<String> = row.get(5)?;
                out.insert(
                    uid,
                    MessageAttributes {
                        uid,
                        message_id: row.get(1)?,
                        unread: row.get::<_, i64>(2)? != 0,
                        starred: row.get::<_, i64>(3)? != 0,
                        draft: row.get::<_, i64>(4)? != 0,
                        labels: parse_labels(labels.as_deref().unwrap_or("")),
                    },
                );
            }
        }
        Ok(out)
    }

    /// The UID `depth` placements below `before` in `folder`, or 1 when the folder holds
    /// fewer (`fetchMessageUIDAtDepth`).
    pub fn fetch_message_uid_at_depth(
        &self,
        folder: &Folder,
        depth: u32,
        before: u32,
    ) -> Result<u32> {
        let uid: Option<i64> = self
            .conn()
            .prepare_cached(
                "SELECT remoteUID FROM MessageFolder WHERE accountId = ? AND folderId = ? AND remoteUID < ? AND remoteUID > 0 \
                 ORDER BY remoteUID DESC LIMIT 1 OFFSET ?",
            )?
            .query_row(params![folder.account_id(), folder.id(), before, depth], |r| r.get(0))
            .optional()?;
        Ok(uid.map_or(1, |u| u as u32))
    }
}
