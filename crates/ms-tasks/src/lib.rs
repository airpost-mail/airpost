//! Client tasks (`TaskProcessor`).
//!
//! Derived from Mailspring-Sync (`MailSync/TaskProcessor.*`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! A task goes `local` → `remote` → `complete` / `cancelled`. The local phase runs on the
//! main thread when the client queues it; the remote phase runs on the foreground worker,
//! which reaches this crate through [`ms_sync::TaskRunnerFactory`]. No automatic retries:
//! a failure ends as `complete` with `error` set.

use std::sync::Arc;

use ms_core::{Account, Result};
use ms_imap::ImapSession;
use ms_store::{Folder, Store, Task};
use ms_sync::{RemoteTaskRunner, TaskRunnerFactory, WorkerContext};

pub struct TaskProcessor<'a> {
    pub account: Arc<Account>,
    pub store: &'a Store,
}

impl<'a> TaskProcessor<'a> {
    pub fn new(account: Arc<Account>, store: &'a Store) -> Self {
        TaskProcessor { account, store }
    }

    /// On launch: deletes tasks still in `local` (`cleanupTasksAfterLaunch`).
    pub fn cleanup_tasks_after_launch(&self) -> Result<()> {
        todo!("P3-T: TaskProcessor::cleanupTasksAfterLaunch")
    }

    /// `queue-task`: saves the task and applies its local change (`performLocal`).
    pub fn perform_local(&self, task: &mut Task) -> Result<()> {
        let _ = task;
        todo!("P3-T: TaskProcessor::performLocal")
    }

    /// `cancel-task`: cancels the task if its remote phase has not started.
    pub fn cancel(&self, task_id: &str) -> Result<()> {
        let _ = task_id;
        todo!("P3-T: TaskProcessor::cancel")
    }
}

impl RemoteTaskRunner for TaskProcessor<'_> {
    fn cleanup_old_tasks_at_runtime(&mut self) -> Result<()> {
        todo!("P3-T: TaskProcessor::cleanupOldTasksAtRuntime")
    }

    fn perform_remote(&mut self, task: &mut Task, session: &mut dyn ImapSession) -> Result<()> {
        let _ = (task, session);
        todo!("P3-T: TaskProcessor::performRemote")
    }

    fn remove_late_sent_copies(
        &mut self,
        sent: &Folder,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let _ = (sent, session);
        todo!("P3-S: TaskProcessor::removeLateSentCopies")
    }
}

/// The factory the bin hands to the workers.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tasks;

impl TaskRunnerFactory for Tasks {
    fn remote_runner<'a>(
        &self,
        store: &'a Store,
        ctx: &WorkerContext,
    ) -> Box<dyn RemoteTaskRunner + 'a> {
        Box::new(TaskProcessor::new(ctx.account.clone(), store))
    }
}
