// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `Thread`. Derived from Mailspring-Sync `MailSync/Models/Thread.*` and
//! `MailUtils::labelForXGMLabelName`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! A thread's `folders` and `labels` arrays hold the category JSON plus two counters per
//! entry: `_refs` (messages of the thread in it) and `_u` (of those, unread). They are kept
//! by diffing each message's snapshot before and after a save
//! ([`Thread::apply_message_attribute_changes`]) rather than recounted, and `after_save`
//! mirrors them into `ThreadCategory` and `ThreadCounts`. `apps/engine/test/harness/
//! invariants.py` (and `ms_store::invariants`) recount every layer to check this.

use std::collections::{BTreeMap, HashSet};

use ms_core::flags::PLACEMENT_FLAG_UNREAD;
use ms_core::Result;
use rusqlite::Statement;
use serde_json::{json, Value};

use crate::model::{
    base_json, bind_base, bind_named, json_i64, json_str, metadata_after_remove,
    metadata_after_save, metadata_before_save, metadata_value, metadata_versions, object_or_err,
    MetadataVersions,
};
use crate::{Label, Message, MessageSnapshot, Model, Store};

/// The send-reminders plugin's id (`plugins/send-reminders`).
pub const SEND_REMINDERS_PLUGIN: &str = "send-reminders";

/// `fmt` of a thread with no dated message yet (`INT_MAX`).
const FIRST_MESSAGE_TIMESTAMP_UNSET: i64 = i32::MAX as i64;

#[derive(Clone, Debug)]
pub struct Thread {
    pub data: Value,
    initial_metadata: MetadataVersions,
    initial_lmst: i64,
    initial_lmrt: i64,
    initial_category_ids: BTreeMap<String, bool>,
    initial_search_categories: String,
}

impl PartialEq for Thread {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Thread {
    /// `Thread(msgId, accountId, subject, gThreadId)`: id `t:<first message id>`, no
    /// messages counted yet.
    pub fn create(
        first_message_id: &str,
        account_id: &str,
        subject: &str,
        gmail_thread_id: u64,
    ) -> Self {
        let mut data = base_json(&ms_core::ids::thread_id(first_message_id), account_id, 0);
        data["subject"] = json!(subject);
        data["lmt"] = json!(0);
        data["fmt"] = json!(FIRST_MESSAGE_TIMESTAMP_UNSET);
        data["lmst"] = json!(0);
        data["lmrt"] = json!(0);
        data["gThrId"] = json!(if gmail_thread_id != 0 {
            gmail_thread_id.to_string()
        } else {
            String::new()
        });
        data["unread"] = json!(0);
        data["starred"] = json!(0);
        data["inAllMail"] = json!(false);
        data["attachmentCount"] = json!(0);
        data["searchRowId"] = json!(0);
        data["folders"] = json!([]);
        data["labels"] = json!([]);
        data["participants"] = json!([]);
        Thread::new(data)
    }

    pub fn new(data: Value) -> Self {
        let mut t = Thread {
            initial_metadata: metadata_versions(&data),
            data,
            initial_lmst: 0,
            initial_lmrt: 0,
            initial_category_ids: BTreeMap::new(),
            initial_search_categories: String::new(),
        };
        t.capture_initial_state();
        t
    }

    fn capture_initial_state(&mut self) {
        self.initial_lmst = self.last_message_sent_timestamp();
        self.initial_lmrt = self.last_message_received_timestamp();
        self.initial_category_ids = self.category_ids();
        self.initial_search_categories = self.categories_search_string();
    }

    /// What a save does to a pending reminder, from `send-reminders-store.ts`
    /// (`_onDatabaseChanged`), so every client sees it without the desktop open: a reminder
    /// that carries an `expiration` but no `lastReplyTimestamp` (one the engine promoted from
    /// a draft, or one another client wrote) gets the thread's `lastMessageReceivedTimestamp`
    /// as its baseline; once that timestamp moves away from the baseline a reply arrived and
    /// the reminder is cleared (`{}`). `None` when nothing changes.
    pub fn reminder_update(&self) -> Option<Value> {
        let value = metadata_value(&self.data, SEND_REMINDERS_PLUGIN)?;
        if !value.get("expiration").is_some_and(Value::is_number) {
            return None;
        }
        let lmrt = self.last_message_received_timestamp();
        match value.get("lastReplyTimestamp").filter(|v| v.is_number()) {
            None => {
                let mut next = value.clone();
                next["lastReplyTimestamp"] = json!(lmrt);
                next["shouldNotify"] = json!(false);
                Some(next)
            }
            Some(baseline) if json_i64(baseline) != lmrt => Some(json!({})),
            Some(_) => None,
        }
    }

    pub fn subject(&self) -> &str {
        json_str(&self.data, "subject")
    }

    pub fn set_subject(&mut self, s: &str) {
        self.data["subject"] = json!(s);
    }

    pub fn unread(&self) -> i64 {
        json_i64(&self.data["unread"])
    }

    pub fn set_unread(&mut self, u: i64) {
        self.data["unread"] = json!(u);
    }

    pub fn starred(&self) -> i64 {
        json_i64(&self.data["starred"])
    }

    pub fn set_starred(&mut self, s: i64) {
        self.data["starred"] = json!(s);
    }

    pub fn attachment_count(&self) -> i64 {
        json_i64(&self.data["attachmentCount"])
    }

    pub fn set_attachment_count(&mut self, n: i64) {
        self.data["attachmentCount"] = json!(n);
    }

    /// The `ThreadSearch` rowid, 0 when not indexed.
    pub fn search_row_id(&self) -> i64 {
        json_i64(&self.data["searchRowId"])
    }

    pub fn set_search_row_id(&mut self, id: i64) {
        self.data["searchRowId"] = json!(id);
    }

    pub fn participants(&self) -> &Value {
        &self.data["participants"]
    }

    pub fn gmail_thread_id(&self) -> &str {
        json_str(&self.data, "gThrId")
    }

    pub fn in_all_mail(&self) -> bool {
        self.data["inAllMail"].as_bool().unwrap_or(false)
    }

    pub fn last_message_timestamp(&self) -> i64 {
        json_i64(&self.data["lmt"])
    }

    pub fn first_message_timestamp(&self) -> i64 {
        json_i64(&self.data["fmt"])
    }

    pub fn last_message_received_timestamp(&self) -> i64 {
        json_i64(&self.data["lmrt"])
    }

    pub fn last_message_sent_timestamp(&self) -> i64 {
        json_i64(&self.data["lmst"])
    }

    pub fn folders(&self) -> &Value {
        &self.data["folders"]
    }

    pub fn labels(&self) -> &Value {
        &self.data["labels"]
    }

    fn entries<'a>(&'a self, key: &str) -> impl Iterator<Item = &'a Value> + 'a {
        self.data
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }

    /// The roles (or paths, for folders without one) of every category, space-separated,
    /// for `ThreadSearch.categories`.
    pub fn categories_search_string(&self) -> String {
        let mut out = String::new();
        for f in self.entries("folders").chain(self.entries("labels")) {
            let role = json_str(f, "role");
            out.push_str(if role.is_empty() {
                json_str(f, "path")
            } else {
                role
            });
            out.push(' ');
        }
        out
    }

    /// Zeroes the counters and empties the category arrays, before re-applying every
    /// message against the empty snapshot.
    pub fn reset_counted_attributes(&mut self) {
        self.set_unread(0);
        self.set_starred(0);
        self.set_attachment_count(0);
        self.data["folders"] = json!([]);
        self.data["labels"] = json!([]);
    }

    /// Replaces this thread's `folders` or `labels` entry for `category` (a Folder's or
    /// Label's JSON) with that JSON, without `localStatus` and keeping the entry's `_refs`
    /// and `_u`, as the entry is first built. A renamed folder keeps its id, and its threads
    /// would otherwise show the old path. Returns whether an entry changed.
    pub fn refresh_category_entry(&mut self, category: &Value) -> bool {
        let id = json_str(category, "id");
        let mut changed = false;
        for key in ["folders", "labels"] {
            let Some(list) = self.data.get_mut(key).and_then(Value::as_array_mut) else {
                continue;
            };
            for entry in list.iter_mut().filter(|e| json_str(e, "id") == id) {
                let mut next = category.clone();
                if let Some(obj) = next.as_object_mut() {
                    obj.remove("localStatus");
                    for counter in ["_refs", "_u"] {
                        if let Some(v) = entry.get(counter) {
                            obj.insert(counter.to_owned(), v.clone());
                        }
                    }
                }
                if *entry != next {
                    *entry = next;
                    changed = true;
                }
            }
        }
        changed
    }

    /// The threads `ThreadCategory` lists under `category_id`.
    pub fn ids_in_category(store: &Store, category_id: &str) -> Result<Vec<String>> {
        let ids = store
            .conn()
            .prepare_cached("SELECT id FROM ThreadCategory WHERE value = ?")?
            .query_map([category_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// The account's threads holding a copy whose stored X-GM-LABELS include a name that
    /// resolves differently under `after` than under `before`: to another label, to none, or
    /// to a label whose path or role changed. Their label refs were counted against
    /// `before` and the label JSON they embed may be stale.
    pub fn ids_affected_by_label_change(
        store: &Store,
        account_id: &str,
        before: &[Label],
        after: &[Label],
    ) -> Result<Vec<String>> {
        let names: Vec<String> = store
            .conn()
            .prepare(
                "SELECT DISTINCT j.value FROM MessageFolder mf, json_each(mf.remoteXGMLabels) j \
                 WHERE mf.accountId = ? AND j.type = 'text'",
            )?
            .query_map([account_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let key = |l: Option<&Label>| {
            l.map(|l| (l.id().to_owned(), l.path().to_owned(), l.role().to_owned()))
        };
        let changed: Vec<&String> = names
            .iter()
            .filter(|n| {
                key(label_for_xgm_label_name_quiet(n, before))
                    != key(label_for_xgm_label_name_quiet(n, after))
            })
            .collect();
        if changed.is_empty() {
            return Ok(Vec::new());
        }
        let ids = store
            .conn()
            .prepare(
                "SELECT DISTINCT m.threadId FROM Message m \
                 JOIN MessageFolder mf ON mf.messageId = m.id, json_each(mf.remoteXGMLabels) j \
                 WHERE mf.accountId = ?1 AND j.value IN (SELECT value FROM json_each(?2)) \
                 AND m.threadId IS NOT NULL AND m.threadId != ''",
            )?
            .query_map(
                rusqlite::params![account_id, json!(changed).to_string()],
                |r| r.get(0),
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Recounts the threads from all of their messages, against the current labels, and
    /// saves them. The caller owns the transaction.
    pub fn recount_from_messages(store: &Store, thread_ids: &[String]) -> Result<()> {
        if thread_ids.is_empty() {
            return Ok(());
        }
        let mut threads: BTreeMap<String, Thread> = store
            .find_all::<Thread>(&crate::Query::new().equal_any("id", thread_ids.iter()))?
            .into_iter()
            .map(|t| (t.id().to_owned(), t))
            .collect();
        for t in threads.values_mut() {
            t.reset_counted_attributes();
        }
        let messages = store
            .find_all::<Message>(&crate::Query::new().equal_any("threadId", thread_ids.iter()))?;
        for msg in &messages {
            if let Some(t) = threads.get_mut(msg.thread_id()) {
                t.apply_message_attribute_changes(&MessageSnapshot::empty(), Some(msg), store)?;
            }
        }
        for t in threads.values_mut() {
            store.save(t)?;
        }
        Ok(())
    }

    /// Updates the counters and category sets from a before and after view of one message:
    /// `old` is the message as it was last counted (the empty snapshot for a new message),
    /// `next` its current state, or `None` when it is being removed.
    ///
    /// Folder refcounts are per distinct folder: two copies of a message in one folder are
    /// one reference. A folder's `_u` uses that copy's own unread bit, so an unread Inbox
    /// copy counts for Inbox while the read Sent copy of the same message does not. Labels
    /// live inside All Mail, so a message adds to a label's `_u` only when it is unread and
    /// in All Mail.
    pub fn apply_message_attribute_changes(
        &mut self,
        old: &MessageSnapshot,
        next: Option<&Message>,
        store: &Store,
    ) -> Result<()> {
        let all_labels = store.all_labels_cache(self.account_id())?;

        self.set_unread(self.unread() - old.unread as i64);
        self.set_starred(self.starred() - old.starred as i64);
        self.set_attachment_count(self.attachment_count() - old.file_count);

        self.adjust_folder_refs(store, &old.folders, -1)?;
        let old_in_all_mail = Message::is_in_all_mail(store, self.account_id(), &old.folders)?;
        self.adjust_label_refs(&all_labels, &old.labels, -1, old.unread && old_in_all_mail);

        if let Some(next) = next {
            self.set_unread(self.unread() + next.is_unread() as i64);
            self.set_starred(self.starred() + next.is_starred() as i64);
            self.set_attachment_count(self.attachment_count() + next.file_count_for_thread_list());

            if !next.is_draft() && !next.is_deletion_placeholder() {
                let date = next.date();
                if date > self.last_message_timestamp() {
                    self.data["lmt"] = json!(date);
                }
                if date < self.first_message_timestamp() {
                    self.data["fmt"] = json!(date);
                }
                let sent_by_user = next.is_sent_by_user(store)?;
                if sent_by_user
                    && !next.is_hidden_reminder()
                    && date > self.last_message_sent_timestamp()
                {
                    self.data["lmst"] = json!(date);
                }
                // Mail you send yourself moves lmrt too, so "not sent by me" is not enough.
                if next.is_in_inbox(store)? || !sent_by_user {
                    let fallback = self.data.get("lmrt_is_fallback").is_some();
                    if fallback || date > self.last_message_received_timestamp() {
                        if let Some(obj) = self.data.as_object_mut() {
                            obj.remove("lmrt_is_fallback");
                        }
                        self.data["lmrt"] = json!(date);
                    }
                } else if self.last_message_received_timestamp() == 0 {
                    // A zero lmrt sorts the thread to the bottom of label views and shows
                    // "1969" in search; use this date until a received message turns up
                    // (sync walks from new to old, so one may come later).
                    self.data["lmrt_is_fallback"] = json!(true);
                    self.data["lmrt"] = json!(date);
                }
            }

            self.adjust_folder_refs(store, next.folders(), 1)?;
            let unread_in_all_mail = next.is_unread() && next.in_all_mail(store)?;
            self.adjust_label_refs(&all_labels, next.labels(), 1, unread_in_all_mail);

            let mut emails: HashSet<String> = self
                .entries("participants")
                .filter_map(|p| p.get("email").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
            for list in [next.to(), next.cc(), next.from()] {
                self.add_missing_participants(&mut emails, list);
            }
        }

        // In All Mail unless every folder of the thread is Spam or Trash.
        let folders: Vec<&Value> = self.entries("folders").collect();
        let spam_or_trash = folders
            .iter()
            .filter(|f| matches!(json_str(f, "role"), "spam" | "trash"))
            .count();
        let in_all_mail = folders.len() > spam_or_trash;
        self.data["inAllMail"] = json!(in_all_mail);
        Ok(())
    }

    /// Applies `delta` to the refcount of every folder in a `{ folderId: bits }` map, adding
    /// the folder's JSON on first reference (without `localStatus`) and dropping the entry
    /// at zero. `_u` moves by the copy's own unread bit.
    fn adjust_folder_refs(&mut self, store: &Store, folder_bits: &Value, delta: i64) -> Result<()> {
        let Some(bits_map) = folder_bits.as_object() else {
            return Ok(());
        };
        for (folder_id, bits) in bits_map {
            let unread_bit = (json_i64(bits) & PLACEMENT_FLAG_UNREAD != 0) as i64;
            let mut found = false;
            let mut next_folders = Vec::new();
            for mut f in self.entries("folders").cloned().collect::<Vec<_>>() {
                if json_str(&f, "id") != folder_id {
                    next_folders.push(f);
                    continue;
                }
                found = true;
                let r = json_i64(&f["_refs"]) + delta;
                if r > 0 {
                    f["_refs"] = json!(r);
                    f["_u"] = json!(json_i64(&f["_u"]) + delta * unread_bit);
                    next_folders.push(f);
                }
            }
            if !found && delta > 0 {
                let mut f = match store.folder_by_id(self.account_id(), folder_id)? {
                    Some(folder) => {
                        let mut j = folder.to_json();
                        if let Some(obj) = j.as_object_mut() {
                            obj.remove("localStatus");
                        }
                        j
                    }
                    // The folder row is gone but a placement still names it; keep the shape
                    // categories_search_string and the inAllMail check expect.
                    None => json!({
                        "id": folder_id,
                        "aid": self.account_id(),
                        "path": "",
                        "role": "",
                        "__cls": "Folder",
                    }),
                };
                f["_refs"] = json!(1);
                f["_u"] = json!(unread_bit);
                next_folders.push(f);
            }
            self.data["folders"] = Value::Array(next_folders);
        }
        Ok(())
    }

    fn adjust_label_refs(
        &mut self,
        all_labels: &[Label],
        label_names: &Value,
        delta: i64,
        unread_in_all_mail: bool,
    ) {
        let unread_bit = unread_in_all_mail as i64;
        // Note: an account without Label rows has nothing to count; its copies' label
        // keywords wait for the keyword label store.
        let Some(names) = label_names.as_array().filter(|_| !all_labels.is_empty()) else {
            return;
        };
        for name in names.iter().filter_map(Value::as_str) {
            let Some(label) = label_for_xgm_label_name(name, all_labels) else {
                continue;
            };
            let mut found = false;
            let mut next_labels = Vec::new();
            for mut l in self.entries("labels").cloned().collect::<Vec<_>>() {
                if json_str(&l, "id") != label.id() {
                    next_labels.push(l);
                    continue;
                }
                found = true;
                let r = json_i64(&l["_refs"]) + delta;
                if r > 0 {
                    l["_refs"] = json!(r);
                    l["_u"] = json!(json_i64(&l["_u"]) + delta * unread_bit);
                    next_labels.push(l);
                }
            }
            if !found && delta > 0 {
                let mut l = label.to_json();
                // Labels keep a UIDVALIDITY there for rename pairing; threads don't need it.
                if let Some(obj) = l.as_object_mut() {
                    obj.remove("localStatus");
                }
                l["_refs"] = json!(1);
                l["_u"] = json!(unread_bit);
                next_labels.push(l);
            }
            self.data["labels"] = Value::Array(next_labels);
        }
    }

    fn add_missing_participants(&mut self, existing: &mut HashSet<String>, incoming: &Value) {
        let Some(list) = incoming.as_array() else {
            return;
        };
        for contact in list {
            let Some(email) = contact.get("email").and_then(Value::as_str) else {
                continue;
            };
            if existing.insert(email.to_owned()) {
                if !self.data["participants"].is_array() {
                    self.data["participants"] = json!([]);
                }
                self.data["participants"]
                    .as_array_mut()
                    .expect("array")
                    .push(contact.clone());
            }
        }
    }

    /// Category id → "has an unread message here" (`_u > 0`).
    fn category_ids(&self) -> BTreeMap<String, bool> {
        self.entries("folders")
            .chain(self.entries("labels"))
            .map(|e| (json_str(e, "id").to_owned(), json_i64(&e["_u"]) > 0))
            .collect()
    }

    /// Mirrors the category arrays into `ThreadCategory` and the per-category counters into
    /// `ThreadCounts`, by diff against the state the thread was loaded (or last saved) with.
    fn sync_categories(&mut self, store: &Store) -> Result<()> {
        let conn = store.conn();
        let in_all_mail = self.in_all_mail();
        let lmrt = self.last_message_received_timestamp();
        let lmst = self.last_message_sent_timestamp();
        let category_ids = self.category_ids();

        if self.initial_category_ids != category_ids
            || self.initial_lmrt != lmrt
            || self.initial_lmst != lmst
        {
            conn.prepare_cached("DELETE FROM ThreadCategory WHERE id = ?")?
                .execute([self.id()])?;
            let mut insert = conn.prepare_cached(
                "INSERT INTO ThreadCategory (id, value, inAllMail, unread, lastMessageReceivedTimestamp, lastMessageSentTimestamp) VALUES (?,?,?,?,?,?)",
            )?;
            for (category, unread) in &category_ids {
                insert.execute(rusqlite::params![
                    self.id(),
                    category,
                    in_all_mail,
                    unread,
                    lmrt,
                    lmst
                ])?;
            }
        }

        if self.initial_category_ids != category_ids {
            // [unread, total] per category.
            let mut diffs: BTreeMap<&str, [i64; 2]> = BTreeMap::new();
            for (category, unread) in &self.initial_category_ids {
                diffs.insert(category, [-(*unread as i64), -1]);
            }
            for (category, unread) in &category_ids {
                let d = diffs.entry(category).or_insert([0, 0]);
                if d[1] == -1 {
                    *d = [d[0] + *unread as i64, 0];
                } else {
                    *d = [*unread as i64, 1];
                }
            }
            let mut change = conn.prepare_cached(
                "UPDATE ThreadCounts SET unread = unread + ?, total = total + ? WHERE categoryId = ?",
            )?;
            let mut changed = Vec::new();
            for (category, [unread, total]) in &diffs {
                if *unread == 0 && *total == 0 {
                    continue;
                }
                if change.execute(rusqlite::params![unread, total, category])? > 0 {
                    changed.push(*category);
                }
            }
            crate::counts::emit_rows(store, changed)?;
        }

        // A renamed folder or label keeps its id, so its path (or role) can change under
        // unchanged ids; `in:` search reads this column.
        let search_categories = self.categories_search_string();
        if (self.initial_category_ids != category_ids
            || self.initial_search_categories != search_categories)
            && self.search_row_id() != 0
        {
            conn.prepare_cached("UPDATE ThreadSearch SET categories = ? WHERE rowid = ?")?
                .execute(rusqlite::params![search_categories, self.search_row_id()])?;
        }
        // Note: the C++ kept the load-time state, so a second save of the same object
        // re-applied the ThreadCounts diff. Saving twice is legitimate; count it once.
        self.capture_initial_state();
        Ok(())
    }
}

impl Model for Thread {
    model_boilerplate!(
        "Thread",
        [
            "id",
            "data",
            "accountId",
            "version",
            "gThrId",
            "unread",
            "starred",
            "inAllMail",
            "subject",
            "lastMessageTimestamp",
            "lastMessageReceivedTimestamp",
            "lastMessageSentTimestamp",
            "firstMessageTimestamp",
            "hasAttachments"
        ]
    );

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "Thread")?;
        Ok(Thread::new(data))
    }

    fn supports_metadata() -> bool {
        true
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_base(self, stmt)?;
        bind_named(stmt, ":unread", self.unread())?;
        bind_named(stmt, ":starred", self.starred())?;
        bind_named(stmt, ":subject", self.subject())?;
        bind_named(stmt, ":inAllMail", self.in_all_mail())?;
        bind_named(stmt, ":gThrId", self.gmail_thread_id())?;
        bind_named(stmt, ":lastMessageTimestamp", self.last_message_timestamp())?;
        bind_named(
            stmt,
            ":lastMessageSentTimestamp",
            self.last_message_sent_timestamp(),
        )?;
        bind_named(
            stmt,
            ":lastMessageReceivedTimestamp",
            self.last_message_received_timestamp(),
        )?;
        bind_named(
            stmt,
            ":firstMessageTimestamp",
            self.first_message_timestamp(),
        )?;
        bind_named(stmt, ":hasAttachments", self.attachment_count())?;
        Ok(())
    }

    fn before_save(&mut self, store: &Store) -> Result<()> {
        metadata_before_save(self, store)?;
        if let Some(value) = self.reminder_update() {
            self.upsert_metadata(SEND_REMINDERS_PLUGIN, &value, None);
        }
        Ok(())
    }

    fn after_save(&mut self, store: &Store) -> Result<()> {
        metadata_after_save(self, &self.initial_metadata, store)?;
        self.initial_metadata = metadata_versions(&self.data);
        self.sync_categories(store)
    }

    /// Every message has already left the thread, so syncing the categories removes the
    /// thread's `ThreadCategory` rows and its `ThreadCounts` contributions.
    fn after_remove(&mut self, store: &Store) -> Result<()> {
        metadata_after_remove(self, store)?;
        self.sync_categories(store)?;
        if self.search_row_id() != 0 {
            store
                .conn()
                .prepare_cached("DELETE FROM ThreadSearch WHERE rowid = ?")?
                .execute([self.search_row_id()])?;
        }
        Ok(())
    }
}

/// The label an X-GM-LABELS name refers to (`MailUtils::labelForXGMLabelName`). A user
/// label name is matched by exact path. A system name like `\Important` is matched by role
/// (also in its plural, `\Draft` → `drafts`), then by a `[Gmail]/` or `[Google Mail]/`
/// path naming it, case-insensitively.
///
/// Note: C++ tried every label's exact path first and then matched `\Name` against paths
/// without `[Gmail]/` and roles in one pass, first hit wins. A user label named
/// "Important" then took `\Important`, and an Important label listed without the
/// `\Important` flag became a user chip (docs/fix-backlog.md F2).
pub fn label_for_xgm_label_name<'a>(name: &str, all_labels: &'a [Label]) -> Option<&'a Label> {
    let found = label_for_xgm_label_name_quiet(name, all_labels);
    if found.is_none() {
        tracing::warn!("Label not found: {name}");
    }
    found
}

fn label_for_xgm_label_name_quiet<'a>(name: &str, all_labels: &'a [Label]) -> Option<&'a Label> {
    match name.strip_prefix('\\') {
        Some(system) => system_label(system, all_labels),
        None => all_labels.iter().find(|l| l.path() == name),
    }
}

fn system_label<'a>(name: &str, all_labels: &'a [Label]) -> Option<&'a Label> {
    let needle = name.to_ascii_lowercase();
    let plural = format!("{needle}s");
    let is_system_name = |s: &str| s == needle || s == plural;
    if let Some(l) = all_labels.iter().find(|l| is_system_name(l.role())) {
        return Some(l);
    }
    all_labels.iter().find(|l| {
        let path = l.path().to_ascii_lowercase();
        ["[gmail]/", "[google mail]/"]
            .iter()
            .any(|p| path.strip_prefix(p).is_some_and(is_system_name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(lmrt: i64, value: Value) -> Thread {
        Thread::new(json!({
            "id": "t1", "aid": "a1", "v": 1, "lmrt": lmrt,
            "metadata": [{"pluginId": SEND_REMINDERS_PLUGIN, "v": 1, "value": value}],
        }))
    }

    #[test]
    fn reminders_get_a_baseline_and_clear_on_reply() {
        let fresh = thread(
            100,
            json!({"expiration": 500, "sentHeaderMessageId": "s@x"}),
        );
        assert_eq!(
            fresh.reminder_update(),
            Some(json!({
                "expiration": 500, "sentHeaderMessageId": "s@x",
                "lastReplyTimestamp": 100, "shouldNotify": false,
            }))
        );
        let waiting = thread(100, json!({"expiration": 500, "lastReplyTimestamp": 100}));
        assert_eq!(waiting.reminder_update(), None);
        let replied = thread(200, json!({"expiration": 500, "lastReplyTimestamp": 100}));
        assert_eq!(replied.reminder_update(), Some(json!({})));
        let fired = thread(200, json!({"expiration": null, "lastReplyTimestamp": 100}));
        assert_eq!(fired.reminder_update(), None);
    }
}
