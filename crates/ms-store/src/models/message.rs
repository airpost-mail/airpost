// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `Message`. Derived from Mailspring-Sync `MailSync/Models/Message.*`, Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! A Message is the logical message; its copies on the server are `MessageFolder` rows
//! (see "Message Identity and Placements" in legacy/mailsync-cpp/CLAUDE.md). The JSON's
//! `folders` map is a snapshot of those rows, rebuilt on save by
//! [`Store::refresh_message_from_placements`] whenever a placement helper marked the
//! message. Besides its JSON a Message carries in-memory state for one save cycle: the
//! snapshot its thread was last counted from, the body to put in the next delta, and the
//! rules-ready flag.

use ms_core::constants::{
    INLINE_ATTACHMENT_COUNT_THRESHOLD, RULES_READY_AWAITING_BODY, RULES_READY_EMITTED,
    RULES_READY_HAS_BODY,
};
use ms_core::flags::placement_flag_bits;
use ms_core::{MessageHeader, Result};
use rusqlite::{OptionalExtension, Statement};
use serde_json::{json, Map, Value};

use crate::model::{
    base_json, bind_base, bind_named, json_bool, json_i64, json_str, metadata_after_remove,
    metadata_after_save, metadata_before_save, metadata_versions, object_or_err, MetadataVersions,
};
use crate::placement::MessageAttributes;
use crate::{File, Folder, Model, Query, Store, Thread};

/// The state of a message that contributes to its thread's counters, captured when the
/// message is loaded and compared against the message after a save so the thread can be
/// updated by diff. `unread`/`starred` are the message-level flags behind the thread's
/// counters and per-label `_u`; they survive an orphan's empty `folders`, whose bits drive
/// only the per-folder `_u`.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageSnapshot {
    pub unread: bool,
    pub starred: bool,
    pub file_count: i64,
    /// X-GM-LABELS names.
    pub labels: Value,
    /// `{ folderId: flagBits }`.
    pub folders: Value,
}

impl MessageSnapshot {
    /// `MessageEmptySnapshot`: what a message not yet counted in its thread contributes.
    pub fn empty() -> Self {
        MessageSnapshot {
            unread: false,
            starred: false,
            file_count: 0,
            labels: Value::Array(Vec::new()),
            folders: Value::Object(Map::new()),
        }
    }
}

/// What the IMAP header fetch knows about a new message, for [`Message::from_incoming`].
#[derive(Clone, Debug)]
pub struct IncomingMessage<'a> {
    /// The deterministic id (`ms_core::ids::message_id`).
    pub id: &'a str,
    pub header: &'a MessageHeader,
    /// INTERNALDATE, used when the header has no date.
    pub received_date: i64,
    /// The Message-ID, or the generated `uuid@hostname` stand-in when the message has none.
    pub header_message_id: &'a str,
    /// X-GM-MSGID, 0 off Gmail.
    pub gmail_message_id: u64,
    pub attrs: &'a MessageAttributes,
    pub sync_data_timestamp: i64,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub data: Value,
    initial_metadata: MetadataVersions,
    last_snapshot: MessageSnapshot,
    body_for_dispatch: String,
    body_fetched: bool,
    rules_ready_for_dispatch: bool,
    placements_changed: bool,
    /// Set by callers that already applied this message to its thread before saving
    /// (`_skipThreadUpdatesAfterSave`).
    pub skip_thread_updates_after_save: bool,
}

impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Message {
    /// A message from client or stored JSON (`Message(json)`). `folders` and `labels` are
    /// filled in when missing: client-authored drafts carry neither, the engine assigns
    /// placements. A message at version 0 has not been counted in a thread yet.
    pub fn new(mut data: Value) -> Self {
        if let Some(obj) = data.as_object_mut() {
            if !obj.get("folders").is_some_and(Value::is_object) {
                obj.insert("folders".into(), Value::Object(Map::new()));
            }
            if !obj.get("labels").is_some_and(Value::is_array) {
                obj.insert("labels".into(), Value::Array(Vec::new()));
            }
        }
        let mut msg = Message {
            initial_metadata: metadata_versions(&data),
            data,
            last_snapshot: MessageSnapshot::empty(),
            body_for_dispatch: String::new(),
            body_fetched: false,
            rules_ready_for_dispatch: false,
            placements_changed: false,
            skip_thread_updates_after_save: false,
        };
        if msg.version() != 0 {
            msg.last_snapshot = msg.snapshot();
        }
        msg
    }

    /// `Message(IMAPMessage *, Folder &, syncDataTimestamp)`: a newly fetched message with
    /// one copy in `folder`. It has no placement row yet; the caller upserts it.
    pub fn from_incoming(inc: &IncomingMessage<'_>, folder: &Folder) -> Self {
        let h = inc.header;
        let mut data = base_json(inc.id, folder.account_id(), 0);
        data["_sa"] = json!(inc.sync_data_timestamp);
        data["_suc"] = json!(0);
        // Only ingestion sets it, so client-authored drafts never become rules-ready.
        data["rr"] = json!(RULES_READY_AWAITING_BODY);
        data["files"] = json!([]);
        data["date"] = json!(h.date.unwrap_or(inc.received_date));
        data["hMsgId"] = json!(inc.header_message_id);
        data["subject"] = json!(h.subject.as_deref().unwrap_or("No Subject"));
        data["gMsgId"] = json!(inc.gmail_message_id.to_string());
        data["rthMsgId"] = match h.in_reply_to.last() {
            Some(irt) => json!(irt),
            None => Value::Null,
        };
        let draft = inc.attrs.draft || folder.role() == "drafts";
        data["unread"] = json!(inc.attrs.unread);
        data["starred"] = json!(inc.attrs.starred);
        data["labels"] = json!(inc.attrs.labels);
        data["draft"] = json!(draft);
        let mut folders = Map::new();
        folders.insert(
            folder.id().to_owned(),
            json!(placement_flag_bits(
                inc.attrs.unread,
                inc.attrs.starred,
                draft
            )),
        );
        data["folders"] = Value::Object(folders);
        let mut extra = Map::new();
        for (k, v) in &h.extra {
            extra.insert(k.clone(), json!(v));
        }
        data["extraHeaders"] = Value::Object(extra);
        let contacts = |list: &[ms_core::Address]| {
            Value::Array(list.iter().map(|a| a.to_contact_json()).collect())
        };
        data["from"] = contacts(&h.from[..h.from.len().min(1)]);
        data["to"] = contacts(&h.to);
        data["cc"] = contacts(&h.cc);
        data["bcc"] = contacts(&h.bcc);
        data["replyTo"] = contacts(&h.reply_to);
        Message::new(data)
    }

    /// `messageWithDeletionPlaceholderFor`: an invisible stand-in that takes over a deleted
    /// draft's (Drafts, UID) placement until the deletion reaches the server, so a Drafts
    /// scan does not put the draft back. It has a fresh id and headerMessageId so the
    /// draft's can be reused at once; the caller moves the placements over. `v = 0` so the
    /// stand-in is both added to and removed from the thread, keeping the counters even.
    pub fn deletion_placeholder_for(draft: &Message, now: i64) -> Message {
        let mut stub = draft.to_json();
        let id = ms_core::ids::deletion_placeholder_id();
        stub["hMsgId"] = json!(format!("deleted-{id}"));
        stub["id"] = json!(id);
        stub["subject"] = json!("Deleting...");
        stub["folders"] = json!({});
        stub["labels"] = json!([]);
        if let Some(obj) = stub.as_object_mut() {
            obj.remove("rr");
        }
        stub["v"] = json!(0);
        let mut msg = Message::new(stub);
        msg.set_draft(false);
        msg.set_unread(false);
        msg.set_starred(false);
        msg.set_sync_unsaved_changes(1);
        msg.set_synced_at(now + 60 * 60);
        msg
    }

    pub fn snapshot(&self) -> MessageSnapshot {
        MessageSnapshot {
            unread: self.is_unread(),
            starred: self.is_starred(),
            file_count: self.file_count_for_thread_list(),
            labels: self.labels().clone(),
            folders: self.folders().clone(),
        }
    }

    /// Records the current state as what the thread has counted (`captureSnapshot`).
    pub fn capture_snapshot(&mut self) {
        self.last_snapshot = self.snapshot();
    }

    /// Treats the message as not yet counted in its thread, for a message moving into a
    /// thread it was never part of.
    pub(crate) fn reset_snapshot(&mut self) {
        self.last_snapshot = MessageSnapshot::empty();
    }

    pub fn last_snapshot(&self) -> &MessageSnapshot {
        &self.last_snapshot
    }

    pub fn is_deletion_placeholder(&self) -> bool {
        ms_core::ids::is_deletion_placeholder_id(self.id())
    }

    /// "Ben via Mailspring" reminders and unsnoozes, which do not move the thread's
    /// last-sent timestamp.
    pub fn is_hidden_reminder(&self) -> bool {
        let from = self.from();
        let Some(arr) = from.as_array() else {
            return false;
        };
        if arr.len() != 1 {
            return false;
        }
        let Some(name) = arr[0].get("name").and_then(Value::as_str) else {
            return false;
        };
        name.len() >= 15 && name.as_bytes().ends_with(b"via Mailspring")
    }

    /// `{ "<folderId>": bits }`: live copies only, keyed by the folder the client sees each
    /// copy in. Read-only: written by [`Store::refresh_message_from_placements`].
    pub fn folders(&self) -> &Value {
        static EMPTY: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        match self.data.get("folders") {
            Some(v) if v.is_object() => v,
            _ => EMPTY.get_or_init(|| Value::Object(Map::new())),
        }
    }

    pub(crate) fn set_folders(&mut self, folders: Map<String, Value>) {
        self.data["folders"] = Value::Object(folders);
    }

    pub fn folder_ids(&self) -> Vec<String> {
        self.folders()
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn folder_role(&self, store: &Store, folder_id: &str) -> Result<String> {
        Ok(store
            .folder_by_id(self.account_id(), folder_id)?
            .map(|f| f.role().to_owned())
            .unwrap_or_default())
    }

    /// A message is "in all mail" when at least one copy is somewhere other than spam or
    /// trash; one with no copies (in transit between folders) is not (`isInAllMail`).
    pub fn is_in_all_mail(store: &Store, account_id: &str, folder_bits: &Value) -> Result<bool> {
        if let Some(obj) = folder_bits.as_object() {
            for id in obj.keys() {
                let role = store
                    .folder_by_id(account_id, id)?
                    .map(|f| f.role().to_owned())
                    .unwrap_or_default();
                if role != "spam" && role != "trash" {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn in_all_mail(&self, store: &Store) -> Result<bool> {
        Message::is_in_all_mail(store, self.account_id(), self.folders())
    }

    pub fn is_unread(&self) -> bool {
        json_bool(&self.data, "unread")
    }

    /// The message-level value only. Per-copy bits live on the placements and reach
    /// `folders` through the placement helpers.
    pub fn set_unread(&mut self, u: bool) {
        self.data["unread"] = json!(u);
    }

    pub fn is_starred(&self) -> bool {
        json_bool(&self.data, "starred")
    }

    pub fn set_starred(&mut self, s: bool) {
        self.data["starred"] = json!(s);
    }

    pub fn is_draft(&self) -> bool {
        json_bool(&self.data, "draft")
    }

    pub fn set_draft(&mut self, d: bool) {
        self.data["draft"] = json!(d);
    }

    /// X-GM-LABELS of the live copies (Gmail has one); the client reads it as `labels`.
    pub fn labels(&self) -> &Value {
        static EMPTY: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        match self.data.get("labels") {
            Some(v) if v.is_array() => v,
            _ => EMPTY.get_or_init(|| Value::Array(Vec::new())),
        }
    }

    fn label_names(&self) -> impl Iterator<Item = &str> {
        self.labels()
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }

    pub fn thread_id(&self) -> &str {
        json_str(&self.data, "threadId")
    }

    pub fn set_thread_id(&mut self, id: &str) {
        self.data["threadId"] = json!(id);
    }

    pub fn snippet(&self) -> &str {
        json_str(&self.data, "snippet")
    }

    pub fn set_snippet(&mut self, s: &str) {
        self.data["snippet"] = json!(s);
    }

    pub fn plaintext(&self) -> bool {
        json_bool(&self.data, "plaintext")
    }

    pub fn set_plaintext(&mut self, p: bool) {
        self.data["plaintext"] = json!(p);
    }

    /// `rthMsgId`, "" when null or absent.
    pub fn reply_to_header_message_id(&self) -> &str {
        json_str(&self.data, "rthMsgId")
    }

    pub fn set_reply_to_header_message_id(&mut self, s: &str) {
        self.data["rthMsgId"] = json!(s);
    }

    pub fn forwarded_header_message_id(&self) -> &str {
        json_str(&self.data, "fwdMsgId")
    }

    pub fn set_forwarded_header_message_id(&mut self, s: &str) {
        self.data["fwdMsgId"] = json!(s);
    }

    pub fn files(&self) -> &Value {
        &self.data["files"]
    }

    pub fn set_files(&mut self, files: &[File]) {
        self.data["files"] = Value::Array(files.iter().map(Model::to_json).collect());
    }

    /// The files the client shows the attachment icon for: no contentId, or larger than
    /// 12 KB (`fileCountForThreadList`).
    pub fn file_count_for_thread_list(&self) -> i64 {
        let Some(files) = self.data.get("files").and_then(Value::as_array) else {
            return 0;
        };
        files
            .iter()
            .filter(|f| {
                f.get("contentId").is_none_or(Value::is_null)
                    || json_i64(&f["size"]) > INLINE_ATTACHMENT_COUNT_THRESHOLD as i64
            })
            .count() as i64
    }

    /// `_sa`: while in the future, scans ignore server data for copies already recorded.
    pub fn synced_at(&self) -> i64 {
        json_i64(&self.data["_sa"])
    }

    pub fn set_synced_at(&mut self, t: i64) {
        self.data["_sa"] = json!(t);
    }

    /// `_suc`: the number of tasks holding the `_sa` guard.
    pub fn sync_unsaved_changes(&self) -> i64 {
        json_i64(&self.data["_suc"])
    }

    pub fn set_sync_unsaved_changes(&mut self, n: i64) {
        self.data["_suc"] = json!(n);
    }

    /// The body to send in the next delta, just fetched from the server.
    pub fn set_body_for_dispatch(&mut self, body: &str) {
        self.body_for_dispatch = body.to_owned();
        self.body_fetched = true;
    }

    /// `markBodyStored`: the body is in `MessageBody`, so the message can become
    /// rules-ready. Bodies purged from the cache later still count.
    pub fn mark_body_stored(&mut self) {
        if self.data.get("rr").map(json_i64) == Some(RULES_READY_AWAITING_BODY) {
            self.data["rr"] = json!(RULES_READY_HAS_BODY);
        }
    }

    /// Whether some copy makes this mail the user received rather than sent, which is what
    /// mail rules apply to (`hasIncomingCopy`). Sent and Drafts copies are the user's own;
    /// Spam and Trash bodies are fetched only when opened, and rules should not depend on
    /// that. On Gmail the one All Mail copy counts once it is filed under `\Inbox`, or when
    /// it carries neither `\Sent` nor `\Draft`.
    pub fn has_incoming_copy(&self, store: &Store) -> Result<bool> {
        for folder_id in self.folder_ids() {
            let role = self.folder_role(store, &folder_id)?;
            if matches!(role.as_str(), "sent" | "drafts" | "spam" | "trash") {
                continue;
            }
            if role != "all" {
                return Ok(true);
            }
            let (mut in_inbox, mut outgoing) = (false, false);
            for name in self.label_names() {
                in_inbox |= name == "\\Inbox";
                outgoing |= name == "\\Sent" || name == "\\Draft";
            }
            if in_inbox || !outgoing {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn is_sent_by_user(&self, store: &Store) -> Result<bool> {
        self.is_in(store, "sent")
    }

    pub fn is_in_inbox(&self, store: &Store) -> Result<bool> {
        self.is_in(store, "inbox")
    }

    /// True when a copy is in a folder with the role, or (Gmail) in All Mail carrying a
    /// label whose name contains it, case-insensitively (`_isIn`).
    fn is_in(&self, store: &Store, role_also_label_name: &str) -> Result<bool> {
        let needle = role_also_label_name.to_ascii_uppercase();
        for folder_id in self.folder_ids() {
            let role = self.folder_role(store, &folder_id)?;
            if role == role_also_label_name {
                return Ok(true);
            }
            if role != "all" {
                continue;
            }
            if self
                .label_names()
                .any(|l| l.to_ascii_uppercase().contains(&needle))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn to(&self) -> &Value {
        &self.data["to"]
    }

    pub fn cc(&self) -> &Value {
        &self.data["cc"]
    }

    pub fn bcc(&self) -> &Value {
        &self.data["bcc"]
    }

    pub fn from(&self) -> &Value {
        &self.data["from"]
    }

    pub fn reply_to(&self) -> &Value {
        &self.data["replyTo"]
    }

    pub fn date(&self) -> i64 {
        json_i64(&self.data["date"])
    }

    pub fn subject(&self) -> &str {
        json_str(&self.data, "subject")
    }

    pub fn gmail_message_id(&self) -> &str {
        json_str(&self.data, "gMsgId")
    }

    pub fn header_message_id(&self) -> &str {
        json_str(&self.data, "hMsgId")
    }

    /// Set by the placement helpers; the next save rebuilds the snapshot from the rows.
    pub fn placements_changed(&self) -> bool {
        self.placements_changed
    }

    pub fn set_placements_changed(&mut self, changed: bool) {
        self.placements_changed = changed;
    }

    /// Mail rules run once per incoming message, on the delta that carries `rulesReady`:
    /// the first save with a stored body and an incoming copy, whichever arrived second
    /// (`_updateRulesReady`). The state lives in `rr` and needs no query, because every save
    /// of a message reloads it inside the saving transaction. The delta always carries the
    /// body, for "Body contains" rules; when the copy arrived second it is read back here.
    fn update_rules_ready(&mut self, store: &Store) -> Result<()> {
        self.rules_ready_for_dispatch = false;
        if self.data.get("rr").map(json_i64) != Some(RULES_READY_HAS_BODY) {
            return Ok(());
        }
        if !self.has_incoming_copy(store)? {
            return Ok(());
        }
        self.data["rr"] = json!(RULES_READY_EMITTED);
        self.rules_ready_for_dispatch = true;
        if self.body_for_dispatch.is_empty() {
            let body: Option<String> = store
                .conn()
                .prepare_cached("SELECT value FROM MessageBody WHERE id = ? AND value IS NOT NULL")?
                .query_row([self.id()], |r| r.get(0))
                .optional()?;
            if let Some(b) = body {
                self.body_for_dispatch = b;
            }
        }
        Ok(())
    }

    fn update_thread(&mut self, store: &Store, removed: bool) -> Result<()> {
        if self.thread_id().is_empty() {
            return Ok(());
        }
        let Some(mut thread) = store.find::<Thread>(&Query::new().equal("id", self.thread_id()))?
        else {
            return Ok(());
        };
        let old = self.last_snapshot.clone();
        if removed {
            thread.apply_message_attribute_changes(&old, None, store)?;
            if thread.folders().as_array().is_none_or(Vec::is_empty) {
                store.remove(&mut thread)?;
            } else {
                store.save(&mut thread)?;
            }
        } else {
            thread.apply_message_attribute_changes(&old, Some(self), store)?;
            store.save(&mut thread)?;
            self.last_snapshot = self.snapshot();
        }
        Ok(())
    }
}

impl Model for Message {
    model_boilerplate!(
        "Message",
        [
            "id",
            "data",
            "accountId",
            "version",
            "headerMessageId",
            "subject",
            "gMsgId",
            "date",
            "draft",
            "unread",
            "starred",
            "threadId"
        ]
    );

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "Message")?;
        Ok(Message::new(data))
    }

    fn supports_metadata() -> bool {
        true
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_base(self, stmt)?;
        bind_named(stmt, ":date", self.date())?;
        bind_named(stmt, ":unread", self.is_unread())?;
        bind_named(stmt, ":starred", self.is_starred())?;
        bind_named(stmt, ":draft", self.is_draft())?;
        bind_named(stmt, ":headerMessageId", self.header_message_id())?;
        bind_named(stmt, ":subject", self.subject())?;
        bind_named(stmt, ":threadId", self.thread_id())?;
        bind_named(stmt, ":gMsgId", self.gmail_message_id())?;
        Ok(())
    }

    /// Runs before `bind` serialises the JSON and before `after_save` diffs the thread
    /// against the last snapshot, so both see the rebuilt `folders`.
    fn before_save(&mut self, store: &Store) -> Result<()> {
        metadata_before_save(self, store)?;
        crate::legacy_import::attach_pending_message_metadata(self, store)?;
        if self.placements_changed {
            store.refresh_message_from_placements(self)?;
        }
        self.update_rules_ready(store)
    }

    fn after_save(&mut self, store: &Store) -> Result<()> {
        metadata_after_save(self, &self.initial_metadata, store)?;
        self.initial_metadata = metadata_versions(&self.data);
        if !self.skip_thread_updates_after_save {
            self.update_thread(store, false)?;
        }
        crate::legacy_import::attach_pending_thread_metadata(self, store)
    }

    fn after_remove(&mut self, store: &Store) -> Result<()> {
        metadata_after_remove(self, store)?;
        store.delete_placements_for_message(self.id())?;
        self.update_thread(store, true)?;
        store
            .conn()
            .prepare_cached("DELETE FROM MessageBody WHERE id = ?")?
            .execute([self.id()])?;
        Ok(())
    }

    /// Adds `body` (and the deprecated `fullSyncComplete` when it was just fetched rather
    /// than read back), `rulesReady`, and `headersSyncComplete` on the first save. Deltas
    /// are merged in the buffer before they are sent, so the client may only see version 2
    /// when the body syncs quickly; these keys survive that merge (`Message.cpp:605-625`).
    fn delta_json(&self) -> Value {
        let mut j = self.to_json();
        if !self.body_for_dispatch.is_empty() {
            j["body"] = json!(self.body_for_dispatch);
            if self.body_fetched {
                j["fullSyncComplete"] = json!(true);
            }
        }
        if self.rules_ready_for_dispatch {
            j["rulesReady"] = json!(true);
        }
        if self.version() == 1 {
            j["headersSyncComplete"] = json!(true);
        }
        j
    }
}
