// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! SendDraftTask, and the removal of the server's own Sent copies.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp` (`performRemoteSendDraft`,
//! `_removeSentCopies`), Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! A send runs once: `_performRemoteRan` is saved before anything else, and every retry is
//! a new task the user starts. After SMTP accepts the message the draft's server copies are
//! deleted, the provider's Sent copy is looked for (or ours appended), and that copy is
//! ingested with the local MIME as its body so the client sees the sent message at once.

use std::path::Path;
use std::time::Duration;

use ms_core::constants::{
    ls, ENGINE_SEND_OFFLINE_RETRY_WINDOW, FILES_DIR, SENT_COPY_CLEANUP_WINDOW,
};
use ms_core::{logmsg, Address, Error, MessageFlags, Result, SyncError, UidSet};
use ms_imap::{FetchKind, ImapSession, StoreOp};
use ms_mime::{OutgoingAttachment, OutgoingBody, OutgoingMessage};
use ms_store::model::json_str;
use ms_store::{File, Folder, Label, Message, Model, Query, Task, Thread};
use ms_sync::mail_processor::MailProcessor;
use ms_sync::mailbox_model;
use serde_json::{json, Value};
use tracing::{error, info};

use crate::imap_ops::{remove_message_copies_resilient, remove_messages_resilient};
use crate::json::{at, elements, first_element, get_bool, get_str, has};
use crate::messages::{is_gmail, xgm_key_for_label};
use crate::TaskProcessor;

/// Waits between looks at Sent for the copy the provider files itself. Exchange Online
/// files it 0.5-3.5 s after the SMTP 250, sometimes 6-8 s, and was seen to take ~3 min (76
/// sends, 2026-09-27); Gmail, Yahoo and Zoho file one too, iCloud and Fastmail by default
/// do not. A Sent folder where a copy has turned up before gets the longer series, so
/// providers that never file one don't slow every send.
const SENT_POLL_DELAYS: [u64; 4] = [0, 1, 1, 2];
const SENT_POLL_DELAYS_SERVER_SAVES: [u64; 7] = [0, 1, 1, 2, 2, 3, 3];

/// Saved on the task once SMTP has accepted the message. A relaunch that finds the task
/// still `remote` with this set knows the message went out, so the draft can go; without it
/// the engine may have died before or during SMTP and the draft is kept for the user.
const SMTP_DONE_KEY: &str = "_smtpDone";

/// `SendDraftTask` key: a send the engine queued itself, which waits out an unreachable
/// server instead of failing (`ms_sync::expiration_executor::RETRY_WHEN_ONLINE_KEY`).
const RETRY_WHEN_ONLINE_KEY: &str = ms_sync::expiration_executor::RETRY_WHEN_ONLINE_KEY;

/// When a deferred send first failed to reach its server.
const OFFLINE_SINCE_KEY: &str = "_offlineSince";

/// Metadata whose pluginId starts with this is attached to the thread, prefix stripped.
const THREAD_METADATA_PREFIX: &str = "thread:";

/// `MailUtils::addressFromContactJSON`.
pub(crate) fn address_from_contact_json(j: &Value) -> Result<Address> {
    let email = get_str(at(j, "email"))?;
    Ok(Address::new(at(j, "name").as_str(), &email))
}

fn addresses(list: &Value) -> Result<Vec<Address>> {
    elements(list)
        .into_iter()
        .map(address_from_contact_json)
        .collect()
}

fn body(plaintext: bool, text: String) -> OutgoingBody {
    if plaintext {
        OutgoingBody::Text(text)
    } else {
        OutgoingBody::Html(text)
    }
}

/// Finds UIDs of the message in `path`; a failed look counts as none found, as mailcore's
/// `findUIDsOfRecentHeaderMessageID` reported nothing on error.
fn find_recent(session: &mut dyn ImapSession, path: &str, header_message_id: &str) -> Vec<u32> {
    match session.find_uids_of_recent_header_message_id(path, header_message_id) {
        Ok(mut uids) => {
            uids.sort_unstable();
            uids.dedup();
            uids
        }
        Err(e) => {
            error!(
                "-X IMAP Error: {}. This may result in duplicate messages in the Sent folder.",
                e.code.name()
            );
            Vec::new()
        }
    }
}

impl TaskProcessor<'_> {
    fn role_folder(&self, role: &str) -> Result<Option<Folder>> {
        self.store.find::<Folder>(
            &Query::new()
                .equal("accountId", self.account.id())
                .equal("role", role),
        )
    }

    /// The `sent` folder, or on Gmail the `sent` label.
    fn sent_folder_or_label(&self) -> Result<Folder> {
        if let Some(f) = self.role_folder("sent")? {
            return Ok(f);
        }
        let label = self.store.find::<Label>(
            &Query::new()
                .equal("accountId", self.account.id())
                .equal("role", "sent"),
        )?;
        label.map(Folder::from).ok_or_else(|| {
            SyncError::fatal(
                "no-sent-folder",
                "Tern doesn't know which folder to use for sent mail. Visit Preferences > Folders to assign a sent folder.",
            )
            .into()
        })
    }

    fn outgoing_attachments(&self, draft: &Message) -> Result<Vec<OutgoingAttachment>> {
        let root = self
            .config_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."))
            .join(FILES_DIR);
        let mut out = Vec::new();
        for file_json in elements(draft.files()) {
            let file = File {
                data: file_json.clone(),
            };
            // mailcore attached an empty part for a file it could not read; sending
            // without the attachment the user chose is worse than not sending (plan
            // section 6, "Send").
            let unreadable = |detail: String| {
                Error::from(SyncError::fatal(
                    "send-failed",
                    format!(
                        "Could not read the attachment {}: {detail}",
                        file.filename()
                    ),
                ))
            };
            let path = file
                .find_in(&root)
                .ok_or_else(|| unreadable("not found in the files directory".into()))?;
            let data = std::fs::read(&path).map_err(|e| unreadable(e.to_string()))?;
            out.push(OutgoingAttachment {
                filename: file.safe_filename(),
                mime_type: String::new(),
                content_id: file.content_id().as_str().map(str::to_owned),
                data,
            });
        }
        Ok(out)
    }

    /// `performRemoteSendDraft`.
    pub(crate) fn perform_remote_send_draft(
        &self,
        task: &mut Task,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        if has(&task.data, "_performRemoteRan") {
            if has(&task.data, SMTP_DONE_KEY) {
                // The engine was stopped (the client quit) between SMTP and removing the
                // draft, usually while it waited for the Sent copy.
                self.finish_interrupted_send(task, session)?;
            }
            return Ok(());
        }
        task.data["_performRemoteRan"] = Value::Bool(true);
        self.store.save(task)?;

        let per_recipient = at(&task.data, "perRecipientBodies").clone();
        let draft_json = &mut task.data["draft"];
        let draft_body = get_str(at(draft_json, "body"))?;
        let plaintext = get_bool(at(draft_json, "plaintext"))?;
        let multisend = per_recipient.as_object();

        let existing = if has(draft_json, "id") {
            let id = get_str(at(draft_json, "id"))?;
            self.store.find::<Message>(&Query::new().equal("id", &id))?
        } else {
            None
        };
        let now = self.store.now();
        let draft = self.inflate_client_draft_json(draft_json, existing.as_ref(), now)?;

        info!("- Sending draft {}", draft.header_message_id());
        let sent = self.sent_folder_or_label()?;
        info!("-- Identified `sent` folder: {}", sent.path());

        let self_body =
            match multisend {
                Some(bodies) => match bodies.get("self") {
                    Some(b) => get_str(b)?,
                    None => return Err(SyncError::fatal(
                        "no-self-body",
                        "If `perRecipientBodies` is populated, you must provide a `self` entry.",
                    )
                    .into()),
                },
                None => draft_body,
            };

        let non_empty = |s: &str| Some(s.to_owned()).filter(|s| !s.is_empty());
        let from = address_from_contact_json(first_element(draft.from())?)?;
        let mut message = OutgoingMessage {
            from: vec![from.clone()],
            to: addresses(draft.to())?,
            cc: addresses(draft.cc())?,
            bcc: addresses(draft.bcc())?,
            reply_to: addresses(draft.reply_to())?,
            subject: draft.subject().to_owned(),
            message_id: draft.header_message_id().to_owned(),
            date: now,
            in_reply_to: non_empty(draft.reply_to_header_message_id()),
            forwarded_message_id: non_empty(draft.forwarded_header_message_id()),
            body: body(plaintext, self_body),
            importance: draft
                .data
                .get("hImportance")
                .and_then(Value::as_str)
                .map(str::to_owned),
            attachments: self.outgoing_attachments(&draft)?,
            ..Default::default()
        };

        // The copy for Sent: the `self` body, Bcc kept.
        let sent_raw = ms_mime::build(&message)?;

        self.deliver(&mut message, multisend, plaintext, &from.email, &sent_raw)?;
        task.data[SMTP_DONE_KEY] = Value::Bool(true);
        self.store.save(task)?;

        // Delete the draft from the server first, so the user sees it has been sent and the
        // next scan of Drafts does not bring it back.
        remove_message_copies_resilient(session, self.store, self.account.id(), &draft)?;

        let mut sent_uid = 0u32;
        {
            let server_saves = sent.local_status().get(ls::SERVER_SAVES_SENT).is_some();
            let delays: &[u64] = if server_saves {
                &SENT_POLL_DELAYS_SERVER_SAVES
            } else {
                &SENT_POLL_DELAYS
            };
            // A multisend is one SMTP submission per recipient; the server files one copy each.
            let expected = multisend.map_or(1, |b| b.len().saturating_sub(1));
            let mut uids = Vec::new();
            for &delay in delays {
                if delay > 0 {
                    info!(
                        "{}",
                        logmsg::sent_poll_sleeping(uids.len(), expected, delay)
                    );
                    (self.sleep)(Duration::from_secs(delay));
                }
                uids = find_recent(session, sent.path(), draft.header_message_id());
                if uids.len() >= expected {
                    break;
                }
            }

            if !uids.is_empty() && !server_saves {
                let mut sent = sent.clone();
                let initial = sent.local_status().clone();
                sent.local_status_mut()
                    .insert(ls::SERVER_SAVES_SENT.into(), Value::Bool(true));
                self.store.save_folder_status(&mut sent, &initial)?;
            }

            if multisend.is_some() && !uids.is_empty() {
                // They carry the per-recipient bodies (open and link tracking); ours, with
                // the `self` body, is appended below.
                info!(
                    "-- Deleting {} messages added to {} by the SMTP gateway.",
                    uids.len(),
                    sent.path()
                );
                self.remove_sent_copies(session, &sent, &uids)?;

                // Gmail files the messages moved out of Sent -> Trash and expunged in All
                // Mail; deleting them again there removes them.
                if let Some(all) = self.role_folder("all")? {
                    let all_uids = find_recent(session, all.path(), draft.header_message_id());
                    if !all_uids.is_empty() {
                        info!(
                            "-- Deleting {} messages just moved to {} by the SMTP gateway.",
                            all_uids.len(),
                            all.path()
                        );
                        remove_messages_resilient(
                            session,
                            self.store,
                            self.account.id(),
                            all.path(),
                            &UidSet::from_uids(all_uids.iter().copied()),
                        )?;
                    }
                }
            } else if multisend.is_none() && !uids.is_empty() {
                sent_uid = uids[0];
                info!(
                    "{}",
                    logmsg::found_sent_copy_from_gateway(sent_uid, uids.len())
                );
            } else {
                info!("-- No messages matching the message-id were found in the Sent folder.");
            }
        }

        let mut appended_to_sent = false;
        if sent_uid == 0 {
            info!("{}", logmsg::placing_new_sent_message());
            match session.append(sent.path(), &sent_raw, MessageFlags::SEEN) {
                Err(e) => error!(
                    "-X IMAP Error: {}. Could not place a message into the Sent folder. This means no metadata will be attached!",
                    e.code.name()
                ),
                Ok(uid) => {
                    // Without APPENDUID (no UIDPLUS) the copy is looked up by Message-ID.
                    // The poll above found none, so a single match is ours; with more the
                    // server's own copy landed meanwhile and ours can't be told apart.
                    sent_uid = uid.unwrap_or_else(|| {
                        match find_recent(session, sent.path(), draft.header_message_id())[..] {
                            [only] => only,
                            _ => 0,
                        }
                    });
                    appended_to_sent = true;
                }
            }
            if is_gmail(session) && sent_uid != 0 {
                self.copy_thread_labels_to_sent(session, &draft, &sent, sent_uid)?;
            }
        }

        if sent_uid == 0 {
            // Nothing to ingest; sending succeeded, so the draft goes.
            return self.remove_sent_draft(&draft);
        }

        // Gmail's Sent is a label and the copy's real home is All Mail, the one folder the
        // sync worker SELECTs. The placement is recorded there when Gmail already shows the
        // message in All Mail; if it lags, the copy is filed under the Sent label for now and
        // the next All Mail scan replaces that transient placement (Gmail keeps one
        // placement per message).
        let gmail = is_gmail(session);
        let mut placement_folder = sent.clone();
        let mut placement_uid = sent_uid;
        if let Some(home) = self.sent_placement_home(session)? {
            if home.id() != sent.id() {
                let all_uids = find_recent(session, home.path(), draft.header_message_id());
                if all_uids.len() == 1 {
                    placement_uid = all_uids[0];
                    info!(
                        "{}",
                        logmsg::found_sent_message_in(home.path(), placement_uid)
                    );
                    placement_folder = home;
                } else {
                    info!(
                        "-- Sent message not yet visible in {} ({} matches), recording it under {} until the next scan",
                        home.path(),
                        all_uids.len(),
                        sent.path()
                    );
                }
            }
        }

        info!(
            "{}",
            logmsg::syncing_sent_message(placement_folder.path(), placement_uid)
        );
        // Courier (and maybe others) show a message created in the selected folder only
        // after a new SELECT.
        if session.profile().reselect_after_append {
            let _ = session.select(placement_folder.path());
        }
        let sync_data_timestamp = self.store.now();
        let fetched = session.fetch_by_uid(
            placement_folder.path(),
            FetchKind::Heavy,
            &UidSet::single(placement_uid),
        );

        // As close as possible to the insert, so the client does not flicker, and before
        // the error checks: sending succeeded, so the draft always goes.
        self.remove_sent_draft(&draft)?;

        let fetched = match fetched {
            Ok(f) => f,
            Err(e) => {
                error!(
                    "-X Error: {} occurred syncing the sent message to the local mail store. Metadata will not be attached.",
                    e.code.name()
                );
                return Ok(());
            }
        };
        let Some(remote) = fetched.messages.last() else {
            error!("-X Error: No messages were returned. Metadata will not be attached!");
            return Ok(());
        };

        let parsed = ms_mime::parse_message(&sent_raw)?;
        let mut processor = MailProcessor::new(&self.account, self.store);
        processor.is_gmail = gmail;
        if let Some(dir) = &self.config_dir {
            processor = processor.with_config_dir(dir);
        }
        let mut local = processor.insert_fallback_to_update_message(
            remote,
            &placement_folder,
            sync_data_timestamp,
        )?;
        processor.retrieved_message_body(&mut local, &parsed)?;

        // A copy the server files after we gave up waiting for it is removed by a later scan
        // of Sent (removeLateSentCopies), which keeps the one we appended: `_asc`.
        if appended_to_sent && placement_folder.id() == sent.id() {
            self.store.transaction("performRemoteSendDraft", |s| {
                if let Some(mut fresh) = s.find::<Message>(&Query::new().equal("id", local.id()))? {
                    fresh.data["_asc"] = json!({
                        "f": sent.id(),
                        "u": placement_uid,
                        "x": s.now() + SENT_COPY_CLEANUP_WINDOW,
                    });
                    s.save(&mut fresh)?;
                    s.unsafe_erase_transaction_deltas();
                }
                Ok(())
            })?;
        }

        info!(
            "-- Synced sent message ({} UID {} = Local ID {})",
            placement_folder.path(),
            placement_uid,
            local.id()
        );

        self.queue_draft_metadata(&draft, &local)
    }

    /// SMTP: one submission of the `self` body, or with `perRecipientBodies` one per
    /// recipient, each with RCPT set to that address and the full headers. A multisend that
    /// fails part way stops there and lists who already got the message.
    fn deliver(
        &self,
        message: &mut OutgoingMessage,
        multisend: Option<&serde_json::Map<String, Value>>,
        plaintext: bool,
        from_email: &str,
        sent_raw: &[u8],
    ) -> Result<()> {
        let mut mailer = (self.mailer)(&self.account)?;
        let mut succeeded = String::new();
        let result = match multisend {
            Some(bodies) => {
                info!("-- Sending customized message bodies to each recipient:");
                let mut result = Ok(());
                for (recipient, text) in bodies {
                    if recipient == "self" {
                        continue;
                    }
                    info!("--- Sending to {recipient}");
                    message.body = body(plaintext, get_str(text)?);
                    let raw = ms_mime::build(message)?;
                    if let Err(e) =
                        mailer.send_to(from_email, std::slice::from_ref(recipient), &raw)
                    {
                        result = Err(e);
                        break;
                    }
                    succeeded.push_str("\n - ");
                    succeeded.push_str(recipient);
                }
                result
            }
            None => {
                info!("-- Sending a single message body to all recipients:");
                mailer.send(sent_raw)
            }
        };

        let Err(e) = result else {
            return Ok(());
        };
        info!(
            "-X An SMTP error occurred: {} SMTP code: {} ({})",
            e.code.name(),
            mailer.last_response_code(),
            e.detail
        );
        let err = if succeeded.is_empty() {
            // Retryable (an unreachable server) only matters to a send that may wait for
            // the account to come back online ([`Self::defer_send_until_online`]).
            let code = SyncError::from_code(e.code, "");
            SyncError::new("send-failed", e.code.name(), code.retryable, code.offline)
        } else {
            SyncError::fatal(
                "send-partially-failed",
                format!("{}:::{succeeded}", e.code.name()),
            )
        };
        Err(err.into())
    }

    /// Where the sent message is placed when it is found there: All Mail on Gmail, Sent
    /// otherwise. `None` when the account has no such folder.
    fn sent_placement_home(&self, session: &dyn ImapSession) -> Result<Option<Folder>> {
        let model = mailbox_model::for_profile(session.profile().mailbox_model, self.account.id());
        match model.sent_placement_folder(self.store) {
            Ok(f) => Ok(Some(f)),
            Err(Error::Sync(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// On Gmail, the labels of the thread being replied to go on the new Sent copy, or the
    /// thread would move only to Sent.
    fn copy_thread_labels_to_sent(
        &self,
        session: &mut dyn ImapSession,
        draft: &Message,
        sent: &Folder,
        sent_uid: u32,
    ) -> Result<()> {
        if draft.thread_id().is_empty() {
            return Ok(());
        }
        let Some(thread) = self
            .store
            .find::<Thread>(&Query::new().equal("id", draft.thread_id()))?
        else {
            return Ok(());
        };
        let mut xgm = Vec::new();
        for l in elements(thread.labels()) {
            let role = get_str(at(l, "role"))?;
            if matches!(role.as_str(), "inbox" | "sent" | "drafts") {
                continue;
            }
            let key = xgm_key_for_label(l)?;
            info!("-- Will add label to new message: {key}");
            xgm.push(key);
        }
        if xgm.is_empty() {
            return Ok(());
        }
        if let Err(e) =
            session.store_labels(sent.path(), &UidSet::single(sent_uid), StoreOp::Add, &xgm)
        {
            error!(
                "-X IMAP Error: {}. Could not add labels to new message in sent folder. This means the thread may disappear from the inbox.",
                e.code.name()
            );
        }
        Ok(())
    }

    /// The tail of a send whose engine was stopped after SMTP accepted it: the draft's
    /// server copies and its row go. The Sent copy is not placed again (a copy the
    /// provider filed is synced as usual).
    fn finish_interrupted_send(&self, task: &Task, session: &mut dyn ImapSession) -> Result<()> {
        let draft_json = at(&task.data, "draft");
        let Some(id) = draft_json.get("id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(draft) = self.store.find::<Message>(&Query::new().equal("id", id))? else {
            return Ok(());
        };
        info!(
            "{}",
            logmsg::removing_draft_of_interrupted_send(draft.header_message_id())
        );
        remove_message_copies_resilient(session, self.store, self.account.id(), &draft)?;
        self.remove_sent_draft(&draft)
    }

    /// Removes the draft that was sent, as stored (its snapshot is what the thread counts
    /// reflect), or as the task described it when it was never saved.
    fn remove_sent_draft(&self, draft: &Message) -> Result<()> {
        let stored = self
            .store
            .find::<Message>(&Query::new().equal("id", draft.id()))?;
        let mut target = stored.unwrap_or_else(|| draft.clone());
        self.store.remove(&mut target)
    }

    /// A send the engine started itself (`retryWhenOnline`, docs/tech-debt.md TD33) that
    /// failed on a retryable error before SMTP accepted it goes back to `remote`, to run
    /// again once the account is online, for up to a day after its first such failure.
    /// Returns whether it did; the task is left for the caller to save.
    pub(crate) fn defer_send_until_online(&self, task: &mut Task, e: &Error) -> bool {
        let Error::Sync(s) = e else {
            return false;
        };
        let retry = task
            .data
            .get(RETRY_WHEN_ONLINE_KEY)
            .and_then(Value::as_bool)
            == Some(true);
        if !retry || !s.retryable || has(&task.data, SMTP_DONE_KEY) {
            return false;
        }
        let now = self.store.now();
        let since = match task.data.get(OFFLINE_SINCE_KEY).and_then(Value::as_i64) {
            Some(since) => since,
            None => now,
        };
        if now - since >= ENGINE_SEND_OFFLINE_RETRY_WINDOW {
            return false;
        }
        info!(
            "{}",
            logmsg::send_deferred_until_online(json_str(&task.data, "headerMessageId"), &s.key)
        );
        task.data[OFFLINE_SINCE_KEY] = json!(since);
        if let Some(obj) = task.data.as_object_mut() {
            obj.remove("_performRemoteRan");
        }
        task.set_status("remote");
        true
    }

    /// `draft.metadata` becomes SyncbackMetadataTasks on the sent message, or on its thread
    /// for a pluginId with the `thread:` prefix, which lets plugins declare thread-level
    /// intent when the draft is written. They are saved as `remote` and run in this same
    /// foreground iteration.
    fn queue_draft_metadata(&self, draft: &Message, local: &Message) -> Result<()> {
        for m in elements(at(&draft.data, "metadata")) {
            let plugin_id = get_str(at(m, "pluginId"))?;
            let value = at(m, "value").clone();
            let data = match plugin_id.strip_prefix(THREAD_METADATA_PREFIX) {
                Some(actual) => {
                    info!(
                        "-- Queueing task to attach {actual} draft metadata to thread {} (thread: prefix).",
                        local.thread_id()
                    );
                    json!({
                        "modelId": local.thread_id(),
                        "modelClassName": "thread",
                        "pluginId": actual,
                        "value": value,
                    })
                }
                None => {
                    info!("-- Queueing task to attach {plugin_id} draft metadata to new message.");
                    json!({
                        "modelId": local.id(),
                        "modelClassName": "message",
                        "modelHeaderMessageId": local.header_message_id(),
                        "pluginId": plugin_id,
                        "value": value,
                    })
                }
            };
            let mut task = Task::create("SyncbackMetadataTask", self.account.id(), &data);
            self.perform_local(&mut task)?;
        }
        Ok(())
    }

    /// Deletes copies from Sent on the server and drops their placements, so they leave the
    /// client now rather than on the folder's next deep scan. A copy the server kept keeps
    /// its placement: on a CONDSTORE server nothing would report it again until the 24 h
    /// gap scan (`sent-copy-cleanup-store-refused`).
    pub(crate) fn remove_sent_copies(
        &self,
        session: &mut dyn ImapSession,
        sent: &Folder,
        uids: &[u32],
    ) -> Result<()> {
        let set = UidSet::from_uids(uids.iter().copied());
        if remove_messages_resilient(session, self.store, self.account.id(), sent.path(), &set)? {
            MailProcessor::new(&self.account, self.store).delete_vanished_placements(sent, uids)?;
        }
        Ok(())
    }
}
