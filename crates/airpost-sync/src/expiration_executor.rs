// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The expiration executor. In `serve` the
//! `metadataExpiration` thread acts on due metadata of the built-in plugins itself, so a
//! scheduled send, a snooze or a reminder no longer needs a desktop window, and several
//! clients can't fire one twice. `hello.features` announces it as `expiration-executor`;
//! clients that see it stop acting on those rows.
//!
//! - `send-later` on a draft: the metadata is cleared, the current draft is finalised
//!   ([`finalise_draft`]) and a `SendDraftTask` queued, then the after-send action. Undo send
//!   writes the same metadata (`isUndoSend`), so it is covered too, and `draft.send` with a
//!   delay; the bodies its client's send hooks made go out instead of the stored one
//!   ([`crate::pending_sends`]). A draft that lacks its
//!   body, its plaintext flag or an attachment (synced from another device, or not yet
//!   downloaded) gets a body fetch and another look after 1, 2, 4 and 8 minutes, then the
//!   send fails with `error` on the metadata. A row marked `clientFinalize` (third-party send
//!   transforms or an action outside [`SEND_ACTIONS`], T9) is left to the client, which gets
//!   it as a `metadata-expiration` delta as before. On a message that is no longer a draft the
//!   row is cleared.
//! - `thread-snooze`: the expiration is cleared and, while the thread is still in Snoozed, it
//!   moves back to the inbox and resurfaces (`snooze-store.ts`, `snooze-utils.ts`).
//! - `send-reminders`: the baseline and the clear on reply happen on every thread save
//!   ([`Thread::reminder_update`]); when due, `shouldNotify` is set, the expiration cleared and
//!   the thread resurfaced (`send-reminders-store.ts`).
//!
//! Metadata changes go through `SyncbackMetadataTask`'s local phase, as a client's would, so
//! they reach the metadata backend. Every other plugin id is emitted as before.
//!
//! With a Airpost server connected (T22, docs/backend-plan.md 3.6 and 3.8):
//! - Before acting on a due row the executor claims it ([`MetadataBackend::claim`]), keyed on
//!   the Message-ID of the draft, or of the thread's oldest message, which every device of the
//!   user's has in common; a claim another device won leaves the row to that device. A server
//!   that can't answer lets this device act (D2), accepting a possible duplicate.
//! - A scheduled draft with `open-tracking` or `link-tracking` on is sent tracked when the
//!   server offers tracking ([`apply_tracking`]).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use airpost_core::constants::{EXECUTOR_INCOMPLETE_DRAFT_WAITS, FILES_DIR};
use airpost_core::{Account, Error, Result};
use airpost_net::{Claim, LocalOnly, MetadataBackend, TrackingTarget};
use airpost_store::model::{json_str, metadata_value};
use airpost_store::{File, Folder, Label, Message, Model, Query, Store, Task, Thread};

use crate::pending_sends::{recipient_mismatch, PendingSends};
use serde_json::{json, Value};
use tracing::{error, info, warn};

pub const SEND_LATER: &str = "send-later";
pub const THREAD_SNOOZE: &str = "thread-snooze";
pub const SEND_REMINDERS: &str = airpost_store::SEND_REMINDERS_PLUGIN;

/// The after-send actions the engine performs (T9); a scheduled draft with another
/// `actionKey` is left to the client.
pub const SEND_ACTIONS: [&str; 2] = ["send", "send-and-archive"];

/// `config-schema.ts`: resurface an unsnoozed thread with a message (default) rather than
/// marking it unread.
pub const UNSNOOZE_TO_TOP_KEY: &str = "core.notifications.unsnoozeToTop";
/// The resurfacing message bodies, written localized by the desktop.
pub const SNOOZE_BODY_KEY: &str = "resurfacing.snoozeBody";
pub const REMINDER_BODY_KEY: &str = "resurfacing.reminderBody";
pub const SNOOZE_BODY_EN: &str = "<p><strong>Airpost Reminder:</strong> This thread has been moved to the top of your inbox by Airpost.</p><p>--The Airpost Team</p>";
pub const REMINDER_BODY_EN: &str = "<p><strong>Airpost Reminder:</strong> This thread has been moved to the top of your inbox by Airpost because no one has replied to your message.</p><p>--The Airpost Team</p>";

/// Zero-width spaces mark soft line breaks in the composer's plaintext editor and are never
/// sent (`send-draft-task.ts`, `ZERO_WIDTH_SPACE`).
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// The settings the executor reads: `serve`'s settings store.
pub trait EngineSettings: Send + Sync {
    /// The stored value of `key`, `None` when unset.
    fn get(&self, key: &str) -> Option<Value>;
}

/// What became of one due row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Acted on; the row is cleared or moved on.
    Done,
    /// Left to the client: emitted as a `metadata-expiration` delta.
    Declined,
    /// Not yet: look again at this epoch second.
    Waiting(i64),
}

/// Saves a task and runs its local phase on the scanning thread's store; an error is a
/// database error. It may run inside the executor's transaction, so it doesn't wake anyone.
pub type QueueTask<'a> = Box<dyn Fn(&mut Task) -> Result<()> + 'a>;
/// Asks the foreground worker for these messages' bodies (and so their attachments).
pub type FetchBodies<'a> = Box<dyn Fn(Vec<String>) + 'a>;
/// Wakes the foreground worker to run the queued tasks' remote phases, once the transaction
/// that queued them has committed.
pub type Wake<'a> = Box<dyn Fn() + 'a>;

pub struct Executor<'a> {
    account: Arc<Account>,
    store: &'a Store,
    files_root: PathBuf,
    settings: Arc<dyn EngineSettings>,
    queue: QueueTask<'a>,
    fetch_bodies: FetchBodies<'a>,
    wake: Wake<'a>,
    /// A task was queued since the last wake.
    queued: Cell<bool>,
    /// `draft.send`'s bodies, kept for the undo window.
    pending: PendingSends,
    /// Scheduled drafts waiting to be complete: looks taken so far, next look.
    waiting: RefCell<HashMap<String, (usize, i64)>>,
    /// The account's metadata backend: claims and the tracking target.
    metadata: Arc<dyn MetadataBackend>,
    /// `(model id, plugin id, expiration)` claims another device won.
    lost: RefCell<HashSet<(String, String, i64)>>,
}

impl<'a> Executor<'a> {
    pub fn new(
        account: Arc<Account>,
        store: &'a Store,
        config_dir: &std::path::Path,
        settings: Arc<dyn EngineSettings>,
        queue: QueueTask<'a>,
        fetch_bodies: FetchBodies<'a>,
        wake: Wake<'a>,
    ) -> Self {
        let pending = PendingSends::new(config_dir, account.id());
        let executor = Executor {
            account,
            store,
            files_root: config_dir.join(FILES_DIR),
            settings,
            queue,
            fetch_bodies,
            wake,
            queued: Cell::new(false),
            pending,
            waiting: RefCell::new(HashMap::new()),
            metadata: Arc::new(LocalOnly),
            lost: RefCell::new(HashSet::new()),
        };
        if let Err(e) = executor.sweep_pending() {
            warn!("-X Could not look over the pending sends: {e}");
        }
        executor
    }

    /// Claims due rows and tracks sends through `metadata`, the account's backend. Without
    /// it (or with [`LocalOnly`]) nothing is claimed and nothing tracked.
    pub fn with_metadata(mut self, metadata: Arc<dyn MetadataBackend>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Removes the pending bodies of drafts that are gone or no longer wait to be sent in the
    /// undo window the bodies were kept for ([`crate::pending_sends::PendingBodies::belong_to`]).
    fn sweep_pending(&self) -> Result<()> {
        for id in self.pending.ids() {
            let draft = self
                .store
                .find::<Message>(&Query::new().equal("id", id.as_str()))?;
            let value = draft
                .as_ref()
                .and_then(|d| metadata_value(&d.data, SEND_LATER))
                .cloned()
                .unwrap_or(Value::Null);
            let current = self.pending.get(&id).is_some_and(|p| p.belong_to(&value));
            if !current {
                self.pending.remove(&id);
            }
        }
        Ok(())
    }

    /// Acts on `plugin_id`'s due row on model `id` of `class`. A failure other than a
    /// database error is logged and the row left to the client.
    pub fn handle(&self, class: &str, id: &str, plugin_id: &str, now: i64) -> Result<Outcome> {
        let known = matches!(
            (class, plugin_id),
            ("Message", SEND_LATER) | ("Thread", THREAD_SNOOZE) | ("Thread", SEND_REMINDERS)
        );
        if !known {
            return Ok(Outcome::Declined);
        }
        // Requests to the server happen here, outside the transaction below.
        if !self.claimed(class, id, plugin_id, now)? {
            return Ok(Outcome::Done);
        }
        let result = match (class, plugin_id) {
            // One write transaction from reading the draft to queuing its send, so a
            // `draft.cancelSend` or `draft.send` (each one transaction too) on another
            // connection comes wholly before or after it.
            ("Message", SEND_LATER) => {
                let target = self.tracking_target(id)?;
                self.store.transaction("expirationSendLater", |_| {
                    self.send_later(id, now, target.as_ref())
                })
            }
            ("Thread", THREAD_SNOOZE) => self.unsnooze(id),
            _ => self.remind(id, now),
        };
        if self.queued.replace(false) {
            (self.wake)();
        }
        match result {
            Err(e) if !matches!(e, Error::Database(_)) => {
                error!("-X Could not act on {plugin_id} of {class} {id}: {e}");
                Ok(Outcome::Declined)
            }
            other => other,
        }
    }

    // -- claims and tracking -------------------------------------------------------------

    /// Whether this device acts on the row: true unless another device holds the claim on
    /// it. Rows the handler leaves to the client or finds not due aren't claimed.
    fn claimed(&self, class: &str, id: &str, plugin_id: &str, now: i64) -> Result<bool> {
        let (data, object) = if class == "Message" {
            let Some(m) = self.store.find::<Message>(&Query::new().equal("id", id))? else {
                return Ok(true);
            };
            let hmid = m.header_message_id().to_owned();
            (m.data, hmid)
        } else {
            let Some(t) = self.store.find::<Thread>(&Query::new().equal("id", id))? else {
                return Ok(true);
            };
            (t.data, self.oldest_header_message_id(id)?)
        };
        let Some(value) = metadata_value(&data, plugin_id) else {
            return Ok(true);
        };
        let Some(expiration) = value
            .get("expiration")
            .and_then(Value::as_f64)
            .map(|e| e as i64)
        else {
            return Ok(true);
        };
        let declined = plugin_id == SEND_LATER
            && (value.get("clientFinalize").and_then(Value::as_bool) == Some(true)
                || !SEND_ACTIONS.contains(
                    &value
                        .get("actionKey")
                        .and_then(Value::as_str)
                        .unwrap_or(SEND_ACTIONS[0]),
                ));
        if declined || expiration > now || object.is_empty() {
            return Ok(true);
        }
        let key = (id.to_owned(), plugin_id.to_owned(), expiration);
        if self.lost.borrow().contains(&key) {
            return Ok(false);
        }
        match self.metadata.claim(&object, plugin_id, expiration) {
            Claim::Lost => {
                info!(
                    "-- Another device claimed {plugin_id} of {class} {id}; leaving it to that one"
                );
                self.lost.borrow_mut().insert(key);
                Ok(false)
            }
            Claim::Won => {
                info!("-- Claimed {plugin_id} of {class} {id}");
                Ok(true)
            }
            Claim::NoServer | Claim::Unreachable => Ok(true),
        }
    }

    /// The Message-ID of the thread's oldest message: the same on every device that synced
    /// the thread, where thread ids and anchors may differ.
    fn oldest_header_message_id(&self, thread_id: &str) -> Result<String> {
        Ok(self
            .store
            .find::<Message>(
                &Query::new()
                    .equal("threadId", thread_id)
                    .equal("draft", false)
                    .order_by("date", true)
                    .limit(1),
            )?
            .map(|m| m.header_message_id().to_owned())
            .unwrap_or_default())
    }

    /// The tracking target when draft `id` tracks anything.
    fn tracking_target(&self, id: &str) -> Result<Option<TrackingTarget>> {
        let Some(draft) = self.store.find::<Message>(&Query::new().equal("id", id))? else {
            return Ok(None);
        };
        if airpost_compose::tracking::kinds_of(&draft.data).is_none() {
            return Ok(None);
        }
        Ok(self.metadata.tracking())
    }

    // -- send-later ---------------------------------------------------------------------

    fn send_later(&self, id: &str, now: i64, tracking: Option<&TrackingTarget>) -> Result<Outcome> {
        let Some(draft) = self.store.find::<Message>(&Query::new().equal("id", id))? else {
            return Ok(Outcome::Done);
        };
        let value = metadata_value(&draft.data, SEND_LATER)
            .cloned()
            .unwrap_or(Value::Null);
        if value.get("clientFinalize").and_then(Value::as_bool) == Some(true) {
            return Ok(Outcome::Declined);
        }
        // The row was due when the scan read it; a cancel or another send may have come since.
        match send_later_expiration(&draft) {
            None => return Ok(Outcome::Done),
            Some(at) if at > now => return Ok(Outcome::Waiting(at)),
            Some(_) => {}
        }
        if !draft.is_draft() {
            info!("-- Clearing send-later on message {id}, which is no longer a draft");
            self.set_metadata(&draft, SEND_LATER, json!({ "expiration": null }))?;
            return Ok(Outcome::Done);
        }
        let action = value
            .get("actionKey")
            .and_then(Value::as_str)
            .unwrap_or(SEND_ACTIONS[0]);
        if !SEND_ACTIONS.contains(&action) {
            info!("-- Leaving draft {id} to the client: its send action is {action}");
            return Ok(Outcome::Declined);
        }

        if let Some(missing) = self.missing_part(&draft)? {
            return self.wait_for(&draft, &value, missing, now);
        }
        self.waiting.borrow_mut().remove(id);

        let pending = self.pending.get(id).filter(|p| p.belong_to(&value));
        self.pending.remove(id);
        if let Some(why) = pending
            .as_ref()
            .and_then(|p| recipient_mismatch(&draft.data, &p.per_recipient))
        {
            // The recipients changed since draft.send checked them.
            self.fail_send(&draft, &value, &why)?;
            return Ok(Outcome::Done);
        }
        let body = match pending.as_ref().and_then(|p| p.self_body.clone()) {
            Some(b) => b,
            None => self.body(id)?.unwrap_or_default(),
        };
        let finalised = match finalise_draft(&self.account, &draft, &body) {
            Ok(f) => f,
            Err(reason) => {
                self.fail_send(&draft, &value, &reason)?;
                return Ok(Outcome::Done);
            }
        };
        info!(
            "-- Sending scheduled draft {} ({action})",
            draft.header_message_id()
        );
        self.set_metadata(&draft, SEND_LATER, json!({ "expiration": null }))?;
        // The draft as stored after the clear, so the task carries the current metadata.
        let draft = self
            .store
            .find::<Message>(&Query::new().equal("id", id))?
            .unwrap_or(draft);
        let mut data = send_task_json(&draft, &finalised, false);
        if let Some(p) = &pending {
            data["perRecipientBodies"] = p.per_recipient_json(&finalised.body);
        }
        if let Some(target) = tracking {
            apply_tracking(&mut data, target);
        }
        self.queue_task("SendDraftTask", &data)?;
        if action == "send-and-archive" {
            if let Some((class, data)) =
                archive_task(self.store, self.account.id(), draft.thread_id())?
            {
                self.queue_task(class, &data)?;
            }
        }
        Ok(Outcome::Done)
    }

    /// What a scheduled draft still lacks to be sent as the user wrote it: its body, its
    /// plaintext flag (both come with the body fetch), or a file to attach.
    fn missing_part(&self, draft: &Message) -> Result<Option<&'static str>> {
        if self.body(draft.id())?.is_none() {
            return Ok(Some("body"));
        }
        if draft.data.get("plaintext").is_none_or(|p| !p.is_boolean()) {
            return Ok(Some("body"));
        }
        let body = self.body(draft.id())?.unwrap_or_default();
        let missing_file = kept_files(draft, &body)
            .into_iter()
            .any(|f| File { data: f }.find_in(&self.files_root).is_none());
        Ok(missing_file.then_some("attachments"))
    }

    fn wait_for(&self, draft: &Message, value: &Value, missing: &str, now: i64) -> Result<Outcome> {
        let mut waiting = self.waiting.borrow_mut();
        let entry = waiting.entry(draft.id().to_owned()).or_insert((0, 0));
        if now < entry.1 {
            return Ok(Outcome::Waiting(entry.1));
        }
        let Some(wait) = EXECUTOR_INCOMPLETE_DRAFT_WAITS.get(entry.0) else {
            waiting.remove(draft.id());
            drop(waiting);
            let reason = format!(
                "This draft could not be sent at its scheduled time: its {missing} didn't arrive from the server."
            );
            self.fail_send(draft, value, &reason)?;
            return Ok(Outcome::Done);
        };
        info!(
            "-- Scheduled draft {} lacks its {missing}; fetching it and looking again in {wait}s",
            draft.header_message_id()
        );
        entry.0 += 1;
        entry.1 = now + wait;
        let next = entry.1;
        drop(waiting);
        (self.fetch_bodies)(vec![draft.id().to_owned()]);
        Ok(Outcome::Waiting(next))
    }

    /// Ends a scheduled send without sending: the expiration goes and `error` says why, so
    /// every client can show it.
    fn fail_send(&self, draft: &Message, value: &Value, reason: &str) -> Result<()> {
        warn!(
            "-X Not sending scheduled draft {}: {reason}",
            draft.header_message_id()
        );
        let mut next = match value {
            Value::Object(_) => value.clone(),
            _ => json!({}),
        };
        next["expiration"] = Value::Null;
        next["error"] = json!(reason);
        self.set_metadata(draft, SEND_LATER, next)
    }

    fn body(&self, id: &str) -> Result<Option<String>> {
        stored_body(self.store, id)
    }

    // -- thread-snooze ------------------------------------------------------------------

    fn unsnooze(&self, id: &str) -> Result<Outcome> {
        let Some(thread) = self.store.find::<Thread>(&Query::new().equal("id", id))? else {
            return Ok(Outcome::Done);
        };
        self.set_metadata(&thread, THREAD_SNOOZE, json!({ "expiration": null }))?;
        if !has_role(thread.folders(), "snoozed") && !has_role(thread.labels(), "snoozed") {
            info!("-- Thread {id} left Snoozed already; clearing its snooze");
            return Ok(Outcome::Done);
        }
        info!("-- Unsnoozing thread {id}");
        self.move_out_of_snoozed(&thread)?;
        let to_top = self
            .settings
            .get(UNSNOOZE_TO_TOP_KEY)
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if to_top {
            self.resurface(&thread, "", SNOOZE_BODY_KEY, SNOOZE_BODY_EN)?;
        } else if thread.unread() == 0 {
            self.queue_task(
                "ChangeUnreadTask",
                &json!({
                    "threadIds": [id], "unread": true, "source": "Unsnoozed message",
                    "canBeUndone": false,
                }),
            )?;
        }
        Ok(Outcome::Done)
    }

    /// `snooze-utils.ts` `moveThreads` with `snooze: false`: the Snoozed label swapped for the
    /// inbox's on Gmail; elsewhere only what is in Snoozed moves back to the inbox.
    fn move_out_of_snoozed(&self, thread: &Thread) -> Result<()> {
        let common = json!({
            "threadIds": [thread.id()], "source": "Snooze Move",
            "taskDescription": "Unsnoozed", "canBeUndone": false,
        });
        let mut data = common.clone();
        if let Some(snoozed) = self.label("snoozed")? {
            let add: Vec<Value> = self.label("inbox")?.iter().map(Model::to_json).collect();
            data["labelsToAdd"] = json!(add);
            data["labelsToRemove"] = json!([snoozed.to_json()]);
            return self.queue_task("ChangeLabelsTask", &data);
        }
        let (Some(snoozed), Some(inbox)) = (self.folder("snoozed")?, self.folder("inbox")?) else {
            warn!(
                "-X Thread {} is in Snoozed but the account has no Snoozed or Inbox folder",
                thread.id()
            );
            return Ok(());
        };
        data["folder"] = inbox.to_json();
        data["sourceFolderIds"] = json!([snoozed.id()]);
        self.queue_task("ChangeFolderTask", &data)
    }

    // -- send-reminders -----------------------------------------------------------------

    fn remind(&self, id: &str, now: i64) -> Result<Outcome> {
        let Some(thread) = self.store.find::<Thread>(&Query::new().equal("id", id))? else {
            return Ok(Outcome::Done);
        };
        let Some(mut value) = metadata_value(&thread.data, SEND_REMINDERS).cloned() else {
            return Ok(Outcome::Done);
        };
        if let Some(update) = thread.reminder_update() {
            if update.as_object().is_some_and(|o| o.is_empty()) {
                info!("-- A reply arrived on thread {id}; clearing its reminder");
                self.set_metadata(&thread, SEND_REMINDERS, update)?;
                return Ok(Outcome::Done);
            }
            value = update;
        }
        let due = value
            .get("expiration")
            .and_then(Value::as_f64)
            .is_some_and(|e| e as i64 <= now);
        if !due {
            return Ok(Outcome::Done);
        }
        info!("-- Reminding about thread {id}: no reply arrived");
        let sent = json_str(&value, "sentHeaderMessageId").to_owned();
        value["expiration"] = Value::Null;
        value["shouldNotify"] = json!(true);
        self.set_metadata(&thread, SEND_REMINDERS, value)?;
        self.resurface(&thread, &sent, REMINDER_BODY_KEY, REMINDER_BODY_EN)?;
        Ok(Outcome::Done)
    }

    // -- resurfacing --------------------------------------------------------------------

    /// `DraftFactory.createDraftForResurfacing` sent as a silent `SendDraftTask`: a message
    /// from the account to itself, named "<account name> via Airpost" so `Message.isHidden`
    /// hides it, answering `in_reply_to` or else the thread's newest message, which brings
    /// the thread to the top of the inbox in every mail client.
    fn resurface(
        &self,
        thread: &Thread,
        in_reply_to: &str,
        body_key: &str,
        fallback: &str,
    ) -> Result<()> {
        let body = self
            .settings
            .get(body_key)
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| fallback.to_owned());
        let reply_to = if in_reply_to.is_empty() {
            self.store
                .find::<Message>(
                    &Query::new()
                        .equal("threadId", thread.id())
                        .equal("draft", false)
                        .order_by("date", false)
                        .limit(1),
                )?
                .map(|m| m.header_message_id().to_owned())
                .unwrap_or_default()
        } else {
            in_reply_to.to_owned()
        };
        let identity = airpost_compose::Identity::from_account_json(self.account.as_json());
        let draft = airpost_compose::draft::resurface_draft(
            &identity,
            thread.id(),
            thread.subject(),
            &reply_to,
            &body,
            self.store.now(),
        );
        let header_message_id = json_str(&draft, "hMsgId").to_owned();
        self.queue_task(
            "SendDraftTask",
            &json!({
                "draft": draft,
                "headerMessageId": header_message_id,
                "perRecipientBodies": null,
                "silent": true,
                RETRY_WHEN_ONLINE_KEY: true,
            }),
        )
    }

    // -- helpers ------------------------------------------------------------------------

    fn label(&self, role: &str) -> Result<Option<Label>> {
        self.store.find::<Label>(
            &Query::new()
                .equal("accountId", self.account.id())
                .equal("role", role),
        )
    }

    fn folder(&self, role: &str) -> Result<Option<Folder>> {
        self.store.find::<Folder>(
            &Query::new()
                .equal("accountId", self.account.id())
                .equal("role", role),
        )
    }

    fn queue_task(&self, class: &str, data: &Value) -> Result<()> {
        let mut task = Task::create(class, self.account.id(), data);
        self.queued.set(true);
        (self.queue)(&mut task)
    }

    /// `SyncbackMetadataTask.forSaving`, as the plugins write their metadata.
    fn set_metadata<M: Model>(&self, model: &M, plugin_id: &str, value: Value) -> Result<()> {
        let mut data = json!({
            "modelId": model.id(),
            "modelClassName": M::TABLE.to_ascii_lowercase(),
            "pluginId": plugin_id,
            "value": value,
        });
        let header_message_id = json_str(model.json(), "hMsgId");
        if !header_message_id.is_empty() {
            data["modelHeaderMessageId"] = json!(header_message_id);
        }
        self.queue_task("SyncbackMetadataTask", &data)
    }
}

/// `SendDraftTask` key: on a retryable or offline error before SMTP accepted the message,
/// the task goes back to `remote` and runs again once the account is online
/// (docs/tech-debt.md TD33). Set on the sends the engine starts itself.
pub const RETRY_WHEN_ONLINE_KEY: &str = "retryWhenOnline";

/// A draft ready to send: its body and the files that go with it.
#[derive(Clone, Debug, PartialEq)]
pub struct FinalDraft {
    pub body: String,
    pub files: Vec<Value>,
}

/// Send finalisation, shared by the executor and `draft.send`: the From address
/// must be the account's or one of its aliases (the composer moves a draft to the account of
/// its From, `ensureCorrectAccount`; a draft whose From never went through the composer and
/// names no account fails here rather than being moved), it must have a recipient, inline
/// files the body no longer
/// references are pruned, and zero-width spaces are stripped from plaintext
/// (`draft-store.ts` `_onSendDraft`, `send-draft-task.ts`). `Err` says why it can't be sent.
pub fn finalise_draft(
    account: &Account,
    draft: &Message,
    body: &str,
) -> std::result::Result<FinalDraft, String> {
    let from = draft
        .from()
        .as_array()
        .and_then(|f| f.first())
        .map(|c| json_str(c, "email").trim().to_owned())
        .unwrap_or_default();
    if from.is_empty() {
        return Err("This draft has no From address.".into());
    }
    if !account
        .own_addresses()
        .any(|a| a.eq_ignore_ascii_case(&from))
    {
        return Err(format!(
            "The From address {from} doesn't belong to the account {}. Open the draft and choose a From address of that account.",
            account.email_address()
        ));
    }
    // Checked here rather than left to SMTP, which would refuse it only once the undo window
    // was over and the user had moved on.
    let has_recipient = [draft.to(), draft.cc(), draft.bcc()].iter().any(|list| {
        list.as_array()
            .is_some_and(|l| l.iter().any(|c| !json_str(c, "email").trim().is_empty()))
    });
    if !has_recipient {
        return Err(
            "This draft has no recipients. Open the draft and add one before sending.".into(),
        );
    }
    let body = if draft.plaintext() {
        body.replace(ZERO_WIDTH_SPACE, "")
    } else {
        body.to_owned()
    };
    let files = kept_files(draft, &body);
    Ok(FinalDraft { body, files })
}

/// The `SendDraftTask` fields for `draft` as finalised.
pub fn send_task_json(draft: &Message, finalised: &FinalDraft, silent: bool) -> Value {
    let mut draft_json = draft.data.clone();
    draft_json["body"] = json!(finalised.body);
    draft_json["files"] = json!(finalised.files);
    json!({
        "draft": draft_json,
        "headerMessageId": draft.header_message_id(),
        "perRecipientBodies": null,
        "silent": silent,
        RETRY_WHEN_ONLINE_KEY: true,
    })
}

/// Tracks a `SendDraftTask` (its JSON, as [`send_task_json`] builds it) whose draft has
/// `open-tracking` or `link-tracking` on: every recipient gets a body with their own pixel and
/// links, the sent copy stays untracked, and the draft's tracking values gain `trackingId`,
/// `recipients` and `links`, which the send moves to the sent message and pushes to the
/// server (docs/backend-plan.md 3.6). The bodies already in `perRecipientBodies` (a client's
/// send hooks) are tracked in place of the draft's. A body that can't be rewritten goes out
/// untracked. Returns whether it tracked.
pub fn apply_tracking(data: &mut Value, target: &TrackingTarget) -> bool {
    let draft = &data["draft"];
    let given = data["perRecipientBodies"].as_object();
    let self_body = given
        .and_then(|b| b.get("self"))
        .or_else(|| draft.get("body"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let per_recipient: BTreeMap<String, String> = given
        .map(|b| {
            b.iter()
                .filter(|(k, _)| k.as_str() != "self")
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let target = airpost_compose::tracking::Target {
        public_url: target.public_url.clone(),
        ns: target.ns.clone(),
    };
    let hmid = json_str(draft, "hMsgId").to_owned();
    match airpost_compose::tracking::track_send(draft, &self_body, &per_recipient, &target) {
        None => false,
        Some(Err(e)) => {
            warn!("-X Could not add tracking to draft {hmid}; sending it untracked: {e}");
            false
        }
        Some(Ok(sent)) => {
            info!(
                "-- Tracking draft {hmid} for {} recipients",
                sent.bodies.len() - 1
            );
            data["perRecipientBodies"] = Value::Object(sent.bodies);
            for (plugin_id, value) in sent.values {
                airpost_compose::tracking::set_plugin_value(&mut data["draft"], plugin_id, value);
            }
            true
        }
    }
}

/// The stored body of message `id`.
pub fn stored_body(store: &Store, id: &str) -> Result<Option<String>> {
    let mut stmt = store
        .conn()
        .prepare_cached("SELECT value FROM MessageBody WHERE id = ?")?;
    let mut rows = stmt.query([id])?;
    Ok(match rows.next()? {
        Some(row) => match row.get_ref(0)? {
            rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
                Some(String::from_utf8_lossy(b).into_owned())
            }
            _ => None,
        },
        None => None,
    })
}

/// The send-later expiration of a draft (Unix seconds), when one is set.
pub fn send_later_expiration(draft: &Message) -> Option<i64> {
    metadata_value(&draft.data, SEND_LATER)?
        .get("expiration")?
        .as_f64()
        .map(|e| e as i64)
}

/// `send-and-archive-extension.ts` with `TaskFactory.tasksForArchiving`: on Gmail the inbox
/// label goes, elsewhere the thread moves to Archive (or All Mail). The task's class and
/// data, `None` when there is nowhere to archive to.
pub fn archive_task(
    store: &Store,
    account_id: &str,
    thread_id: &str,
) -> Result<Option<(&'static str, Value)>> {
    if thread_id.is_empty() {
        return Ok(None);
    }
    let by_role = |table_label: bool, role: &str| -> Result<Option<Value>> {
        let q = Query::new()
            .equal("accountId", account_id)
            .equal("role", role);
        Ok(if table_label {
            store.find::<Label>(&q)?.map(|l| l.to_json())
        } else {
            store.find::<Folder>(&q)?.map(|f| f.to_json())
        })
    };
    let source = "Send and Archive";
    if let Some(inbox) = by_role(true, "inbox")? {
        return Ok(Some((
            "ChangeLabelsTask",
            json!({
                "threadIds": [thread_id], "labelsToAdd": [], "labelsToRemove": [inbox],
                "source": source,
            }),
        )));
    }
    let archive = match by_role(false, "archive")? {
        Some(f) => Some(f),
        None => by_role(false, "all")?,
    };
    Ok(archive.map(|folder| {
        (
            "ChangeFolderTask",
            json!({
                "threadIds": [thread_id], "folder": folder, "sourceFolderIds": [],
                "source": source,
            }),
        )
    }))
}

/// The draft's files without inline ones its body no longer references.
fn kept_files(draft: &Message, body: &str) -> Vec<Value> {
    draft
        .files()
        .as_array()
        .map(|files| {
            files
                .iter()
                .filter(|f| match f.get("contentId").and_then(Value::as_str) {
                    Some(cid) if !cid.is_empty() => body.contains(&format!("cid:{cid}")),
                    _ => true,
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn has_role(categories: &Value, role: &str) -> bool {
    categories
        .as_array()
        .is_some_and(|list| list.iter().any(|c| json_str(c, "role") == role))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account::from_json(json!({
            "id": "a1", "emailAddress": "me@example.com", "name": "Me",
            "aliases": ["Work Me <work@example.com>", "other@example.com"],
        }))
        .unwrap()
    }

    fn draft(from: &str, plaintext: bool, files: Value) -> Message {
        Message::new(json!({
            "id": "d1", "aid": "a1", "v": 1, "hMsgId": "d1@example.com", "draft": true,
            "from": [{"email": from}], "to": [{"email": "you@example.org"}],
            "plaintext": plaintext, "files": files,
        }))
    }

    #[test]
    fn finalise_needs_a_recipient() {
        let a = account();
        let mut d = draft("me@example.com", false, json!([]));
        for empty in [json!([]), json!([{"email": " "}]), Value::Null] {
            d.data["to"] = empty;
            let err = finalise_draft(&a, &d, "x").unwrap_err();
            assert!(err.contains("no recipients"), "{err}");
        }
        d.data["bcc"] = json!([{"email": "hidden@example.org"}]);
        assert!(finalise_draft(&a, &d, "x").is_ok());
    }

    #[test]
    fn finalise_checks_the_from_address() {
        let a = account();
        for ok in [
            "me@example.com",
            "ME@example.com",
            "work@example.com",
            "other@example.com",
        ] {
            assert!(
                finalise_draft(&a, &draft(ok, false, json!([])), "x").is_ok(),
                "{ok}"
            );
        }
        let err =
            finalise_draft(&a, &draft("stranger@example.org", false, json!([])), "x").unwrap_err();
        assert!(err.contains("stranger@example.org"), "{err}");
        assert!(finalise_draft(&a, &draft("", false, json!([])), "x").is_err());
    }

    #[test]
    fn finalise_prunes_unreferenced_inline_files_and_strips_zero_width_spaces() {
        let files = json!([
            {"id": "f1", "contentId": "used"},
            {"id": "f2", "contentId": "gone"},
            {"id": "f3", "contentId": null},
        ]);
        let f = finalise_draft(
            &account(),
            &draft("me@example.com", false, files.clone()),
            "<img src=\"cid:used\">",
        )
        .unwrap();
        let ids: Vec<&str> = f.files.iter().map(|f| f["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["f1", "f3"]);
        let f = finalise_draft(
            &account(),
            &draft("me@example.com", true, json!([])),
            "a\u{200B}b",
        )
        .unwrap();
        assert_eq!(f.body, "ab");
    }
}
