// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Client tasks (`TaskProcessor`).
//!
//! Derived from Mailspring-Sync (`MailSync/TaskProcessor.*`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! A task goes `local` → `remote` → `complete` / `cancelled`. The local phase runs on the
//! main thread when the client queues it; the remote phase runs on the foreground worker,
//! which reaches this crate through [`tern_sync::TaskRunnerFactory`]. No automatic retries:
//! a failure ends as `complete` with `error` set.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tern_core::constants::{
    ls, SENT_COPY_CLEANUP_WINDOW, TASK_NEXT_ATTEMPT_KEY as NEXT_ATTEMPT_KEY,
};
use tern_core::{logmsg, Account, Error, Result, SyncError, UidSet};
use tern_imap::ImapSession;
use tern_net::{LocalOnly, MetadataBackend, MetadataPush, TokenManager};
use tern_store::model::{json_i64, json_str};
use tern_store::{Folder, Message, Model, Query, Store, Task};
use tern_sync::mail_processor::{MailProcessor, UnplacedMessages};
use tern_sync::{RemoteTaskRunner, TaskRunnerFactory, WorkerContext};
use tracing::{error, info};

mod contacts;
mod draft_sync;
mod drafts;
mod events;
mod export;
mod folders;
mod imap_ops;
mod json;
pub mod mail_actions;
pub mod mail_rules;
mod mailer;
mod messages;
mod rsvp;
mod send;

pub use export::sanitize_eml_filename;
pub use mailer::{smtp_mailer, thread_sleeper, Mailer, MailerFactory, Sleeper};

use json::{at, get_str};

/// `cleanupOldTasksAtRuntime` keeps this many complete / cancelled tasks...
const KEPT_FINISHED_TASKS: i64 = 100;
/// ...and runs only once more than this many beyond them have accumulated.
const FINISHED_TASKS_SLOP: i64 = 10;

pub use contacts::CONTACT_TASKS;
pub use events::EVENT_TASKS;

fn unsure_how_to_process(class: &str) -> String {
    format!("Unsure of how to process this task type {class}")
}

pub struct TaskProcessor<'a> {
    pub account: Arc<Account>,
    pub store: &'a Store,
    metadata: Arc<dyn MetadataBackend>,
    /// OAuth access tokens for the People API and CalDAV/CardDAV (contact and event tasks).
    tokens: Arc<TokenManager>,
    /// `--verbose`: DAV writes log the vCard or ICS they send.
    verbose: bool,
    /// `$CONFIG_DIR_PATH`: attachments are read from and written to `files/` under it.
    config_dir: Option<PathBuf>,
    mailer: MailerFactory,
    sleep: Sleeper,
    /// Replaces the DAV transport and host lookup of the contact and event tasks (tests).
    dav_connector: Option<DavConnector>,
}

/// Builds the DAV client and host resolver a contact or event task's DavWorker uses.
pub type DavConnector =
    Arc<dyn Fn() -> (tern_dav::DavClient, Box<dyn tern_dav::HostResolver>) + Send + Sync>;

impl<'a> TaskProcessor<'a> {
    /// A processor with the local-only metadata backend, SMTP with the account's password
    /// or a token refreshed for this processor, and no config dir.
    pub fn new(account: Arc<Account>, store: &'a Store) -> Self {
        let tokens = Arc::new(TokenManager::new(None));
        TaskProcessor {
            account,
            store,
            metadata: Arc::new(LocalOnly),
            tokens: tokens.clone(),
            verbose: false,
            config_dir: None,
            mailer: smtp_mailer(tokens, false),
            sleep: thread_sleeper(),
            dav_connector: None,
        }
    }

    /// The processor a thread of the sync process uses, on that thread's store.
    pub fn for_context(ctx: &WorkerContext, store: &'a Store) -> Self {
        TaskProcessor {
            account: ctx.account.clone(),
            store,
            metadata: ctx.metadata.clone(),
            tokens: ctx.tokens.clone(),
            verbose: ctx.verbose,
            config_dir: Some(ctx.config_dir.clone()),
            mailer: smtp_mailer(ctx.tokens.clone(), ctx.verbose),
            sleep: thread_sleeper(),
            dav_connector: None,
        }
    }

    pub fn with_config_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config_dir = Some(dir.into());
        self
    }

    /// Replaces the OAuth token source of the People API calls (tests).
    pub fn with_tokens(mut self, tokens: Arc<TokenManager>) -> Self {
        self.tokens = tokens;
        self
    }

    /// Replaces the DAV transport and host lookup (tests).
    pub fn with_dav(mut self, connector: DavConnector) -> Self {
        self.dav_connector = Some(connector);
        self
    }

    /// Replaces SMTP (tests).
    pub fn with_mailer(mut self, mailer: MailerFactory) -> Self {
        self.mailer = mailer;
        self
    }

    /// Replaces the waits between polls of the Sent folder (tests).
    pub fn with_sleeper(mut self, sleep: Sleeper) -> Self {
        self.sleep = sleep;
        self
    }

    /// On launch: deletes tasks still in `local`. They are saved right before their local
    /// phase runs, so the app most likely crashed running them; deleting them keeps it
    /// from crashing again (`cleanupTasksAfterLaunch`).
    pub fn cleanup_tasks_after_launch(&self) -> Result<()> {
        let stuck = self.store.find_all::<Task>(
            &Query::new()
                .equal("accountId", self.account.id())
                .equal("status", "local"),
        )?;
        for mut t in stuck {
            self.store.remove(&mut t)?;
        }
        self.cleanup_old_tasks()
    }

    /// Keeps the last 100 finished tasks, once more than 10 beyond that have piled up. They
    /// are removed one by one rather than with a DELETE, because the client observes the
    /// Task table and releases only rows it gets an `unpersist` for.
    fn cleanup_old_tasks(&self) -> Result<()> {
        let finished = || {
            Query::new()
                .equal("accountId", self.account.id())
                .equal_any("status", ["complete", "cancelled"])
        };
        let count_to_remove = self.store.count::<Task>(&finished())? - KEPT_FINISHED_TASKS;
        if count_to_remove <= FINISHED_TASKS_SLOP {
            return Ok(());
        }
        self.store.transaction("cleanupOldTasksAtRuntime", |s| {
            let unneeded = finished()
                .order_by("rowid", true)
                .limit(count_to_remove as usize);
            for mut task in s.find_all::<Task>(&unneeded)? {
                s.remove(&mut task)?;
            }
            Ok(())
        })
    }

    /// `queue-task` on the main thread: saves the task, applies its local change and moves
    /// it to `remote`. A failure ends the task as `complete` with `error`; there are no
    /// retries. A database error leaves the task as it was and is returned (the caller
    /// aborts, as the C++ main loop did): it says nothing about whether the task can run.
    pub fn perform_local(&self, task: &mut Task) -> Result<()> {
        let class = task.class().to_owned();
        let id = task.id().to_owned();
        info!("[{id}] Running {class} performLocal:");

        if let Err(e) = self.store.save(task) {
            error!("[{id}] -- Exception: Task could not be saved to the database. {e}");
            return Ok(());
        }

        match self.perform_local_by_class(&class, task) {
            Ok(()) => {
                info!("[{id}] -- Succeeded. Changing status to `remote`");
                task.set_status("remote");
            }
            Err(e) => self.fail(task, e)?,
        }
        self.store.save(task)
    }

    fn perform_local_by_class(&self, class: &str, task: &mut Task) -> Result<()> {
        let account_id = task.account_id().to_owned();
        if account_id != self.account.id() {
            return Err(SyncError::fatal("generic", "You must provide an account id.").into());
        }
        let data = &mut task.data;
        match class {
            "ChangeUnreadTask" => {
                self.perform_local_change_on_messages(data, messages::apply_unread)
            }
            "ChangeStarredTask" => {
                self.perform_local_change_on_messages(data, messages::apply_starred)
            }
            "ChangeFolderTask" => {
                data["undoPlacements"] = Value::Object(Default::default());
                self.perform_local_change_on_messages(data, messages::apply_folder)
            }
            "ChangeLabelsTask" => {
                self.perform_local_change_on_messages(data, messages::apply_labels)
            }
            c if CONTACT_TASKS.contains(&c) => self.perform_local_contact_task(c, data),
            c if EVENT_TASKS.contains(&c) => self.perform_local_event_task(c, data),
            "SyncbackDraftTask" => self.perform_local_save_draft(data),
            "DestroyDraftTask" => self.perform_local_destroy_draft(data),
            "SyncbackMetadataTask" => self.perform_local_syncback_metadata(&account_id, data),
            "ChangeRoleMappingTask" => self.perform_local_change_role_mapping(&account_id, data),
            "SyncbackCategoryTask"
            | "DestroyCategoryTask"
            | "SendDraftTask"
            | "ExpungeAllInFolderTask"
            | "GetManyRFC2822Task"
            | "EventRSVPTask" => Ok(()),
            "GetMessageRFC2822Task" => {
                export::assign_export_file_id(data);
                Ok(())
            }
            other => {
                error!("{}", unsure_how_to_process(other));
                Ok(())
            }
        }
    }

    /// `cancel-task`: flags the task; its remote phase ends it as `cancelled` if it has not
    /// started, and a long export stops at its next chunk.
    pub fn cancel(&self, task_id: &str) -> Result<()> {
        self.store.transaction("cancel", |s| {
            let task = s.find::<Task>(
                &Query::new()
                    .equal("id", task_id)
                    .equal("accountId", self.account.id()),
            )?;
            if let Some(mut task) = task {
                task.set_should_cancel();
                s.save(&mut task)?;
            }
            Ok(())
        })
    }

    /// The remote phase. Saves the task's final status itself; only a database error is
    /// returned, leaving the task in `remote` for the next pass or launch.
    fn perform_remote_task(&self, task: &mut Task, session: &mut dyn ImapSession) -> Result<()> {
        let class = task.class().to_owned();
        let id = task.id().to_owned();
        info!("[{id}] Running {class} performRemote:");
        let may_retry = may_retry(&class, &task.data);

        let result = (|| -> Result<()> {
            if task.account_id() != self.account.id() {
                return Err(SyncError::fatal("generic", "You must provide an account id.").into());
            }
            if task.should_cancel() {
                task.set_status("cancelled");
                return Ok(());
            }
            self.perform_remote_by_class(&class, task, session, may_retry)?;
            // A long-running task (GetManyRFC2822Task) can see a cancel request while it
            // runs and stop early; the check above only covers cancels before it starts.
            if task.should_cancel() {
                info!("[{id}] -- Cancelled. Changing status to `cancelled`");
                task.set_status("cancelled");
            } else {
                info!("[{id}] -- Succeeded. Changing status to `complete`");
                task.set_status("complete");
            }
            Ok(())
        })();
        if let Err(e) = result {
            if class == "SendDraftTask" && self.defer_send_until_online(task, &e) {
                self.store.save(task)?;
                return Err(e);
            }
            if may_retry && e.is_retryable() {
                self.retry_later(task, &e);
                self.store.save(task)?;
                return Err(e);
            }
            self.fail(task, e)?;
        }
        self.store.save(task)
    }

    fn perform_remote_by_class(
        &self,
        class: &str,
        task: &mut Task,
        session: &mut dyn ImapSession,
        may_retry: bool,
    ) -> Result<()> {
        let account_id = task.account_id().to_owned();
        let on_messages: Option<(bool, messages::RemoteChange)> = match class {
            "ChangeUnreadTask" => Some((false, messages::apply_unread_in_imap_folder)),
            "ChangeStarredTask" => Some((false, messages::apply_starred_in_imap_folder)),
            "ChangeFolderTask" => Some((true, messages::apply_folder_move_in_imap_folder)),
            "ChangeLabelsTask" => Some((false, messages::apply_label_change_in_imap_folder)),
            _ => None,
        };
        if let Some((is_move, apply)) = on_messages {
            return self
                .perform_remote_change_on_messages(&task.data, session, is_move, apply, may_retry);
        }
        match class {
            // Drafts are not synced back to the server.
            "SyncbackDraftTask" | "ChangeRoleMappingTask" => Ok(()),
            "DestroyDraftTask" => self.perform_remote_destroy_draft(&task.data, session),
            "SyncbackCategoryTask" => {
                self.perform_remote_syncback_category(&account_id, &mut task.data, session)
            }
            "DestroyCategoryTask" => self.perform_remote_destroy_category(&task.data, session),
            "SyncbackMetadataTask" => self.perform_remote_syncback_metadata(&task.data),
            "ExpungeAllInFolderTask" => {
                self.perform_remote_expunge_all_in_folder(&task.data, session)
            }
            "GetMessageRFC2822Task" => self.perform_remote_get_message_rfc2822(&task.data, session),
            "GetManyRFC2822Task" => self.perform_remote_get_many_rfc2822(task, session),
            "SendDraftTask" => self.perform_remote_send_draft(task, session),
            "EventRSVPTask" => self.perform_remote_send_rsvp(&task.data),
            c if CONTACT_TASKS.contains(&c) => self.perform_remote_contact_task(c, &task.data),
            c if EVENT_TASKS.contains(&c) => self.perform_remote_event_task(c, &task.data),
            other => {
                error!("{}", unsure_how_to_process(other));
                Ok(())
            }
        }
    }

    /// Leaves a task whose remote phase hit a retryable error in `remote` for another
    /// attempt after a backoff (TD43): the user's change stays applied locally meanwhile.
    fn retry_later(&self, task: &mut Task, e: &Error) {
        let attempts = remote_attempts(&task.data) + 1;
        let delay = retry_delay(attempts);
        info!(
            "[{}] -- Failed with a retryable error ({e}); attempt {attempts} of {MAX_REMOTE_ATTEMPTS}, next in {delay}s",
            task.id()
        );
        task.data[REMOTE_ATTEMPTS_KEY] = json!(attempts);
        task.data[NEXT_ATTEMPT_KEY] = json!(self.store.now() + delay);
        task.set_status("remote");
    }

    /// Ends a task that failed as `complete` with its error JSON. A database error is not
    /// the task's fault: it is logged and returned, and the task left as it was.
    fn fail(&self, task: &mut Task, e: Error) -> Result<()> {
        let id = task.id().to_owned();
        match e {
            Error::Database(_) => {
                error!("[{id}] -- Database error, not marking the task complete: {e}");
                tern_core::log::flush();
                return Err(e);
            }
            Error::Sync(ref s) => {
                error!(
                    "[{id}] -- Failed ({}). Changing status to `complete`",
                    s.to_json()
                );
            }
            ref other => {
                error!(
                    "[{id}] -- Failed with an unexpected exception ({other}). Changing status to `complete`"
                );
            }
        }
        tern_core::log::flush();
        task.set_error(e.to_json());
        task.set_status("complete");
        Ok(())
    }

    /// `upsertMetadata` on the model the task names, any class `findGeneric` knows, and
    /// the new version recorded as `modelMetadataNewVersion` for the remote phase.
    fn perform_local_syncback_metadata(&self, account_id: &str, data: &mut Value) -> Result<()> {
        let id = get_str(at(data, "modelId"))?;
        let class = get_str(at(data, "modelClassName"))?;
        let plugin_id = get_str(at(data, "pluginId"))?;
        let value = at(data, "value").clone();
        let version = self.store.transaction("performLocalSyncbackMetadata", |s| {
            let query = Query::new().equal("id", &id).equal("accountId", account_id);
            let Some(mut model) = s.find_generic(&class, &query)? else {
                info!(
                    "cannot apply metadata locally because no model matching type:{class} id:{id}, aid:{account_id} could be found."
                );
                return Ok(None);
            };
            let version = model.upsert_metadata(&plugin_id, &value, None);
            model.save(s)?;
            Ok(version)
        })?;
        if let Some(v) = version {
            data["modelMetadataNewVersion"] = Value::from(v);
        }
        Ok(())
    }

    /// Hands the change to the metadata backend. With the local-only backend there is
    /// nothing remote, as when the C++ ran without an identity, and neither is
    /// there for a local-only plugin (`tern_net::LOCAL_ONLY_PLUGINS`), whose value is not logged.
    fn perform_remote_syncback_metadata(&self, data: &Value) -> Result<()> {
        if self.metadata.is_local_only()
            || tern_net::is_local_only_plugin(json_str(data, "pluginId"))
        {
            return Ok(());
        }
        let model_id = get_str(at(data, "modelId"))?;
        let model_class = json_str(data, "modelClassName").to_owned();
        let mut header_message_id = at(data, "modelHeaderMessageId")
            .as_str()
            .filter(|h| !h.is_empty())
            .map(str::to_owned);
        // Threads have no Message-ID of their own; the anchor lets other devices find theirs.
        if header_message_id.is_none() && model_class.eq_ignore_ascii_case("thread") {
            header_message_id = tern_sync::metadata_worker::thread_anchor(self.store, &model_id)?;
        }
        let push = MetadataPush {
            account_id: self.account.id().to_owned(),
            model_id,
            model_class,
            plugin_id: get_str(at(data, "pluginId"))?,
            value: at(data, "value").clone(),
            version: json_i64(at(data, "modelMetadataNewVersion")),
            header_message_id,
        };
        self.metadata.push(&push)?;
        info!(
            "Syncback of metadata {}:{} = {} succeeded.",
            push.model_id, push.plugin_id, push.value
        );
        Ok(())
    }
}

impl TaskProcessor<'_> {
    /// The other messages of this account with `msg`'s Message-ID.
    fn messages_sharing_header_message_id(&self, msg: &Message) -> Result<Vec<String>> {
        if msg.header_message_id().is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .store
            .conn()
            .prepare_cached(
                "SELECT id FROM Message WHERE headerMessageId = ? AND accountId = ? AND id != ?",
            )?
            .query_map(
                rusqlite::params![msg.header_message_id(), self.account.id(), msg.id()],
                |r| r.get(0),
            )?
            .collect::<rusqlite::Result<_>>()?)
    }
}

impl RemoteTaskRunner for TaskProcessor<'_> {
    fn cleanup_old_tasks_at_runtime(&mut self) -> Result<()> {
        self.cleanup_old_tasks()
    }

    fn perform_remote(&mut self, task: &mut Task, session: &mut dyn ImapSession) -> Result<()> {
        self.perform_remote_task(task, session)
    }

    fn sync_drafts(
        &mut self,
        session: &mut dyn ImapSession,
        tombstoned: &[tern_sync::drafts::TombstonedCopy],
    ) -> Result<Option<std::time::Duration>> {
        self.remove_sent_drafts(session)?;
        self.delete_tombstoned_copies(session, tombstoned)?;
        self.reconcile_draft_copies(session)?;
        let next_push = self.push_dirty_drafts(session)?;
        // After the pushes, so a draft just pushed to Gmail takes its copy's thread.
        self.thread_unthreaded_drafts(session)?;
        Ok(next_push)
    }

    /// A sent message appended to Sent because the server's own copy had not appeared yet
    /// may get that copy minutes later (Exchange Online, ~3 min, 2026-09-27), and each
    /// recipient of a multisend adds one. Within `SENT_COPY_CLEANUP_WINDOW` of the send,
    /// while the appended copy (`_asc`) is still there, every other copy in Sent of that
    /// message, or of a message with its Message-ID, is removed from the server and the
    /// store (`removeLateSentCopies`).
    fn remove_late_sent_copies(
        &mut self,
        sent: &Folder,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let now = self.store.now();
        // Runs every pass: start from the messages dated within the window (their Date is
        // the send time) so the account's date index keeps this off the rest of the mail.
        let holders: Vec<String> = self
            .store
            .conn()
            .prepare_cached(
                "SELECT DISTINCT m.id FROM Message m JOIN MessageFolder mf ON mf.messageId = m.id \
                 WHERE m.accountId = ? AND m.date >= ? AND mf.folderId = ? AND mf.remoteUID > 0 \
                 AND json_extract(m.data, '$._asc.f') = ?",
            )?
            .query_map(
                rusqlite::params![
                    self.account.id(),
                    now - SENT_COPY_CLEANUP_WINDOW,
                    sent.id(),
                    sent.id()
                ],
                |r| r.get(0),
            )?
            .collect::<rusqlite::Result<_>>()?;

        for message_id in holders {
            let Some(msg) = self
                .store
                .find::<Message>(&Query::new().equal("id", &message_id))?
            else {
                continue;
            };
            let Some(asc) = msg.json().get("_asc") else {
                continue;
            };
            if json_str(asc, "f") != sent.id() || json_i64(&asc["x"]) < now {
                continue;
            }
            let keep_uid = json_i64(&asc["u"]) as u32;
            let kept_present = self
                .store
                .placements_for_message(&message_id)?
                .iter()
                .any(|p| p.folder_id == sent.id() && p.remote_uid == keep_uid);
            if !kept_present {
                continue;
            }

            // A late copy usually hashes to the same id. One that doesn't (a provider that
            // rewrote a header the id covers) still has the Message-ID.
            let mut late = UidSet::new();
            for p in self.store.placements_for_message(&message_id)? {
                if p.folder_id == sent.id() && p.remote_uid != 0 && p.remote_uid != keep_uid {
                    late.insert(p.remote_uid);
                }
            }
            // Only the rows that hold one of the copies being removed: a draft or orphan
            // with this Message-ID and no Sent copy is not ours to delete below.
            let mut others = Vec::new();
            for id in self.messages_sharing_header_message_id(&msg)? {
                let mut holds_copy = false;
                for p in self.store.placements_for_message(&id)? {
                    if p.folder_id == sent.id() && p.remote_uid != 0 {
                        late.insert(p.remote_uid);
                        holds_copy = true;
                    }
                }
                if holds_copy {
                    others.push(id);
                }
            }
            if late.is_empty() {
                continue;
            }
            if sent.local_status().get(ls::SERVER_SAVES_SENT).is_none() {
                let mut sent = sent.clone();
                let initial = sent.local_status().clone();
                sent.local_status_mut()
                    .insert(ls::SERVER_SAVES_SENT.into(), Value::Bool(true));
                self.store.save_folder_status(&mut sent, &initial)?;
            }
            info!(
                "{}",
                logmsg::removing_late_sent_copies(
                    late.iter().count(),
                    &message_id,
                    sent.path(),
                    keep_uid
                )
            );
            let late: Vec<u32> = late.iter().collect();
            self.remove_sent_copies(session, sent, &late)?;
            // Those rows lost a copy we deleted ourselves, so it has not moved anywhere: one
            // left with no copy goes now instead of waiting for the orphan sweep.
            MailProcessor::new(&self.account, self.store).refresh_messages(
                &others,
                UnplacedMessages::Remove,
                "removeLateSentCopies",
            )?;
        }
        Ok(())
    }
}

/// The factory the bin hands to the workers.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tasks;

/// The classes whose remote phase can run again after a partial failure without doing
/// anything twice: flags, labels and moves are set to a target state, and category and
/// expunge tasks name what they act on (TD43). `SendDraftTask` has its own deferral.
const RETRIED_CLASSES: [&str; 7] = [
    "ChangeUnreadTask",
    "ChangeStarredTask",
    "ChangeFolderTask",
    "ChangeLabelsTask",
    "SyncbackCategoryTask",
    "DestroyCategoryTask",
    "ExpungeAllInFolderTask",
];
/// Attempts before a retried task fails for good.
pub const MAX_REMOTE_ATTEMPTS: u64 = 8;
/// Task data: remote attempts that failed with a retryable error so far.
pub const REMOTE_ATTEMPTS_KEY: &str = "_remoteAttempts";

fn remote_attempts(data: &Value) -> u64 {
    data.get(REMOTE_ATTEMPTS_KEY)
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// Whether a retryable failure of this attempt leaves the task for another.
fn may_retry(class: &str, data: &Value) -> bool {
    RETRIED_CLASSES.contains(&class) && remote_attempts(data) + 1 < MAX_REMOTE_ATTEMPTS
}

/// Seconds before attempt `attempts + 1`: 5 s doubling to 5 minutes.
fn retry_delay(attempts: u64) -> i64 {
    let doublings = u32::try_from(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
    5i64.saturating_mul(2i64.saturating_pow(doublings)).min(300)
}

impl TaskRunnerFactory for Tasks {
    fn remote_runner<'a>(
        &self,
        store: &'a Store,
        ctx: &WorkerContext,
    ) -> Box<dyn RemoteTaskRunner + 'a> {
        Box::new(TaskProcessor::for_context(ctx, store))
    }

    fn perform_local(&self, store: &Store, ctx: &WorkerContext, task: &mut Task) -> Result<()> {
        TaskProcessor::for_context(ctx, store).perform_local(task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tern_store::DeltaStream;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let clock = tern_core::ManualClock::new(1_700_000_000);
        tern_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
        (dir, store)
    }

    fn save_tasks(store: &Store, account_id: &str, status: &str, n: usize) -> Vec<String> {
        (0..n)
            .map(|_| {
                let mut t = Task::create("ChangeStarredTask", account_id, &json!({}));
                t.set_status(status);
                store.save(&mut t).unwrap();
                t.id().to_owned()
            })
            .collect()
    }

    fn ids(store: &Store, account_id: &str) -> Vec<String> {
        let q = Query::new()
            .equal("accountId", account_id)
            .order_by("rowid", true);
        store
            .find_all::<Task>(&q)
            .unwrap()
            .iter()
            .map(|t| t.id().to_owned())
            .collect()
    }

    #[test]
    fn cleanup_keeps_the_newest_hundred_finished_tasks() {
        let (_dir, store) = store();
        let account = Arc::new(Account::from_json(json!({"id": "acc1"})).unwrap());
        let mut oldest = save_tasks(&store, "acc1", "complete", 8);
        oldest.extend(save_tasks(&store, "acc1", "cancelled", 8));
        let remote = save_tasks(&store, "acc1", "remote", 3);
        let newer = save_tasks(&store, "acc1", "complete", 100);
        let other_account = save_tasks(&store, "acc2", "complete", 120);

        let mut p = TaskProcessor::new(account, &store);
        p.cleanup_old_tasks_at_runtime().unwrap();

        let mut expected = remote.clone();
        expected.extend(newer);
        assert_eq!(ids(&store, "acc1"), expected);
        assert_eq!(ids(&store, "acc2"), other_account);
    }

    #[test]
    fn cleanup_waits_for_more_than_ten_extra() {
        let (_dir, store) = store();
        let account = Arc::new(Account::from_json(json!({"id": "acc1"})).unwrap());
        let all = save_tasks(&store, "acc1", "complete", 110);
        TaskProcessor::new(account, &store)
            .cleanup_old_tasks_at_runtime()
            .unwrap();
        assert_eq!(ids(&store, "acc1"), all);
    }
}
