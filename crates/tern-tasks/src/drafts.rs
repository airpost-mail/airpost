// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! SyncbackDraftTask (local only) and DestroyDraftTask.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp` (`inflateClientDraftJSON`,
//! `draftsFolder`, `performLocalSaveDraft`, `ensureDraftPlacement`,
//! `performLocalDestroyDraft`, `performRemoteDestroyDraft`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.

use serde_json::{json, Value};
use tern_core::{Error, Result, SyncError};
use tern_imap::ImapSession;
use tern_store::model::json_str;
use tern_store::{Folder, Message, MessageAttributes, Model, Query, Store};
use tracing::info;

use crate::imap_ops::remove_message_copies_resilient;
use crate::json::{at, get_str, has, str_vec};
use crate::TaskProcessor;

impl TaskProcessor<'_> {
    /// Builds the engine's Message for draft JSON the client sent. The client serializes
    /// only the fields it knows about, so the local copy of the draft (when there is one)
    /// supplies the rest: engine bookkeeping, metadata and the `folders` snapshot. A
    /// `folders` echoed by the client may be stale and is discarded; a brand-new draft gets
    /// its Drafts placement from `ensure_draft_placement`.
    pub(crate) fn inflate_client_draft_json(
        &self,
        draft_json: &mut Value,
        existing: Option<&Message>,
        now: i64,
    ) -> Result<Message> {
        let Some(obj) = draft_json.as_object_mut() else {
            return Err(Error::other(format!(
                "[json.exception.type_error.307] cannot use erase() with {}",
                if draft_json.is_null() {
                    "null"
                } else {
                    "a non-object"
                }
            )));
        };
        obj.remove("folders");

        let base = match existing {
            Some(e) => e.data.clone(),
            None => {
                let id = tern_core::ids::draft_id(
                    &get_str(obj.get("aid").unwrap_or(&Value::Null))?,
                    &get_str(obj.get("hMsgId").unwrap_or(&Value::Null))?,
                );
                json!({
                    "draft": true,
                    "unread": false,
                    "starred": false,
                    "folders": {},
                    "date": now,
                    "_sa": 0,
                    "_suc": 0,
                    "labels": [],
                    "id": id,
                    "threadId": "",
                    "gMsgId": "",
                    "files": [],
                    "from": [],
                    "to": [],
                    "cc": [],
                    "bcc": [],
                    "replyTo": [],
                })
            }
        };
        // Keys the client sent win; the base fills in the rest.
        if let Value::Object(base) = base {
            for (k, v) in base {
                obj.entry(k).or_insert(v);
            }
        }
        // A synced draft the engine put in a thread (`thread_unthreaded_drafts`) stays there
        // when a composer that opened it unthreaded saves it again.
        if let Some(e) = existing.filter(|e| {
            self.account.save_drafts_to_server()
                && !e.thread_id().is_empty()
                && obj
                    .get("threadId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .is_empty()
        }) {
            obj.insert("threadId".into(), json!(e.thread_id()));
        }
        obj.insert("date".into(), json!(now));

        let msg = Message::new(draft_json.clone());
        if msg.account_id() != self.account.id() {
            return Err(SyncError::fatal(
                "bad-accountid",
                "The draft in this task has the wrong account ID.",
            )
            .into());
        }
        Ok(msg)
    }

    /// The stored draft the client's draft JSON names: by `id`, else, when the account
    /// syncs drafts, the one draft row with its `hMsgId`. A synced draft can be re-keyed while
    /// a composer holds it (tern_store `rekey_message`), and the client's next save or send
    /// still names the old id; the JSON is pointed at the row it means.
    pub(crate) fn find_client_draft(
        &self,
        store: &Store,
        draft_json: &mut Value,
    ) -> Result<Option<Message>> {
        if has(draft_json, "id") {
            let id = get_str(at(draft_json, "id"))?;
            if let Some(m) = store.find::<Message>(&Query::new().equal("id", &id))? {
                return Ok(Some(m));
            }
            if !self.account.save_drafts_to_server() {
                return Ok(None);
            }
            let hmid = json_str(draft_json, "hMsgId").to_owned();
            if hmid.is_empty() {
                return Ok(None);
            }
            let ids = store.draft_ids_with_header_message_id(self.account.id(), &hmid, &id)?;
            let [found] = &ids[..] else {
                return Ok(None);
            };
            let found = store.find::<Message>(&Query::new().equal("id", found))?;
            if let Some(m) = &found {
                draft_json["id"] = json!(m.id());
            }
            return Ok(found);
        }
        // Without an id the draft is new to the client, but a synced draft of that
        // Message-ID may be here already, under the id a new one would get.
        let hmid = json_str(draft_json, "hMsgId");
        if !self.account.save_drafts_to_server() || hmid.is_empty() {
            return Ok(None);
        }
        let id = tern_core::ids::draft_id(self.account.id(), hmid);
        store.find::<Message>(&Query::new().equal("id", &id))
    }

    fn drafts_folder(&self, store: &Store) -> Result<Folder> {
        let by_role = |role: &str| {
            store.find::<Folder>(
                &Query::new()
                    .equal("accountId", self.account.id())
                    .equal("role", role),
            )
        };
        if let Some(f) = by_role("drafts")? {
            return Ok(f);
        }
        if let Some(f) = by_role("all")? {
            return Ok(f);
        }
        Err(SyncError::fatal(
            "no-drafts-folder",
            "Tern can't find your Drafts folder. To create and send mail, visit Preferences > Folders and choose a Drafts folder.",
        )
        .into())
    }

    /// A draft with no copy anywhere would be swept as an orphan at the end of the next
    /// sync pass. A brand-new draft, or one whose server copy vanished while the user kept
    /// editing it, is given a Drafts-folder placement at UID 0 (not on the server).
    fn ensure_draft_placement(&self, store: &Store, draft: &mut Message) -> Result<()> {
        if !store.placements_for_message(draft.id())?.is_empty() {
            return Ok(());
        }
        let folder = self.drafts_folder(store)?;
        let attrs = MessageAttributes {
            uid: 0,
            unread: draft.is_unread(),
            starred: draft.is_starred(),
            draft: true,
            labels: Vec::new(),
            message_id: String::new(),
        };
        store.upsert_placement(draft, &folder, 0, &attrs)?;
        Ok(())
    }

    pub(crate) fn perform_local_save_draft(&self, data: &mut Value) -> Result<()> {
        let draft_json = &mut data["draft"];
        self.store.transaction("performLocalSaveDraft", |s| {
            // An existing draft is updated in place so its save hooks see the change.
            let existing = self.find_client_draft(s, draft_json)?;
            let mut draft =
                self.inflate_client_draft_json(draft_json, existing.as_ref(), s.now())?;
            draft.set_draft_sync(self.next_draft_sync(existing.as_ref(), s.now()).as_ref());
            let draft_id = draft.id().to_owned();

            match existing {
                Some(mut existing) => {
                    // The client's data may carry an older version, and the version decides
                    // between INSERT and UPDATE, so the stored one is bumped.
                    let existing_version = existing.version();
                    existing.data = draft.data;
                    existing.data["v"] = json!(existing_version + 1);
                    self.ensure_draft_placement(s, &mut existing)?;
                    s.save(&mut existing)?;
                }
                None => {
                    let mut draft = draft;
                    self.ensure_draft_placement(s, &mut draft)?;
                    s.save(&mut draft)?;
                }
            }

            if self.account.save_drafts_to_server() {
                s.clear_draft_tombstone(self.account.id(), json_str(draft_json, "hMsgId"))?;
            }

            if has(draft_json, "body") {
                let body = get_str(at(draft_json, "body"))?;
                s.conn()
                    .prepare_cached("REPLACE INTO MessageBody (id, value) VALUES (?, ?)")?
                    .execute(rusqlite::params![draft_id, body])?;
            }
            Ok(())
        })
    }

    /// Destroys drafts locally right away and leaves an invisible placeholder in their
    /// place until the deletion reaches the server. The placeholder takes over each draft's
    /// server placement, so the Drafts scan keeps recognising the UID instead of
    /// re-inserting the draft, and reports it under Trash so the thread leaves the Drafts
    /// view. The draft's id is freed immediately because the user can switch a draft
    /// between accounts and re-create one with the same accountId + headerMessageId.
    pub(crate) fn perform_local_destroy_draft(&self, data: &mut Value) -> Result<()> {
        let message_ids = str_vec(at(data, "messageIds"))?;
        info!("-- Hiding / detatching drafts while they're deleted...");

        let account_id = self.account.id();
        let trash = self.store.find::<Folder>(
            &Query::new()
                .equal("accountId", account_id)
                .equal("role", "trash"),
        )?;

        let stub_ids = self.store.transaction("performLocalDestroyDraft", |s| {
            let mut stub_ids = Vec::new();
            for mut draft in s.find_large_set::<Message>("id", &message_ids)? {
                let placements = s.placements_for_message(draft.id())?;
                s.remove(&mut draft)?;
                self.tombstone_draft_in(s, draft.header_message_id())?;

                let mut stub = Message::deletion_placeholder_for(&draft, s.now());
                for p in &placements {
                    if p.remote_uid == 0 {
                        continue;
                    }
                    let Some(folder) = s.folder_by_id(account_id, &p.folder_id)? else {
                        continue;
                    };
                    let attrs = MessageAttributes {
                        uid: p.remote_uid,
                        unread: p.unread,
                        starred: p.starred,
                        draft: p.draft,
                        labels: p.labels.clone(),
                        message_id: String::new(),
                    };
                    s.upsert_placement(&mut stub, &folder, p.remote_uid, &attrs)?;
                    if let Some(trash) = &trash {
                        s.begin_placement_move(&mut stub, &p.folder_id, p.remote_uid, trash.id())?;
                    }
                }
                // Rebuilt before the flags below so the save keeps them, and unconditionally
                // so a stub for a draft that was never on the server is recorded as an orphan.
                s.refresh_message_from_placements(&mut stub)?;
                // The placement keeps the draft's flags so the Drafts scan sees no change;
                // the placeholder itself must not appear in the draft list.
                stub.set_draft(false);
                stub.set_unread(false);
                stub.set_starred(false);
                s.save(&mut stub)?;
                stub_ids.push(Value::from(stub.id()));

                info!("-- Replacing local ID {} with {}", draft.id(), stub.id());
            }
            Ok(stub_ids)
        })?;
        data["stubIds"] = Value::Array(stub_ids);
        Ok(())
    }

    /// With "Save drafts to server" on, keeps the server copies of a draft sent or deleted
    /// here from syncing back (docs/drafts-sync-design.md 3.3).
    pub(crate) fn tombstone_draft(&self, header_message_id: &str) -> Result<()> {
        self.tombstone_draft_in(self.store, header_message_id)
    }

    pub(crate) fn tombstone_draft_in(&self, store: &Store, header_message_id: &str) -> Result<()> {
        if !self.account.save_drafts_to_server() || header_message_id.is_empty() {
            return Ok(());
        }
        store.add_draft_tombstone(self.account.id(), header_message_id)
    }

    /// Placeholders with no placement (the draft was never on the server) are removed here
    /// too: the orphan sweep only runs at the end of a sync pass and a stub carries a
    /// `_sa` hold, so nothing else would clean them up promptly.
    pub(crate) fn perform_remote_destroy_draft(
        &self,
        data: &Value,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let stub_ids = str_vec(at(data, "stubIds"))?;
        for mut stub in self.store.find_large_set::<Message>("id", &stub_ids)? {
            info!("-- Deleting remote draft {}", stub.id());
            remove_message_copies_resilient(session, self.store, self.account.id(), &stub)?;
            // Sync would remove it eventually, but its body would be fetched first.
            self.store.remove(&mut stub)?;
        }
        Ok(())
    }
}
