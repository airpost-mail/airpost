// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The task runner seam. `TaskProcessor` lives in airpost-tasks, which depends on this crate,
//! so workers reach it through these traits and the bin passes airpost-tasks' factory in.

use airpost_core::Result;
use airpost_imap::ImapSession;
use airpost_store::{Folder, Store, Task};

use crate::WorkerContext;

/// The parts of `TaskProcessor` the sync workers call.
pub trait RemoteTaskRunner {
    /// Prunes finished tasks beyond the last 100 (`cleanupOldTasksAtRuntime`).
    fn cleanup_old_tasks_at_runtime(&mut self) -> Result<()>;
    /// Runs a task's remote phase and saves its final status (`performRemote`). A database
    /// error is returned with the task left as it was. A `SyncError` means the task was put
    /// back in `remote` to run again once the account is back online (a send the engine
    /// started); the caller goes on with the other tasks and then
    /// sleeps on that error like any retryable one.
    fn perform_remote(&mut self, task: &mut Task, session: &mut dyn ImapSession) -> Result<()>;
    /// Server-synced drafts, after the queued tasks have run:
    /// deletes the `tombstoned` copies scans skipped, takes or overrules other devices'
    /// revisions, and pushes the saved drafts that are due. Returns how long until the next
    /// push is due, for the foreground to wake then.
    fn sync_drafts(
        &mut self,
        _session: &mut dyn ImapSession,
        _tombstoned: &[crate::drafts::TombstonedCopy],
    ) -> Result<Option<std::time::Duration>> {
        Ok(None)
    }
    /// Background pass hook for Sent-role folders (`removeLateSentCopies`).
    fn remove_late_sent_copies(
        &mut self,
        sent: &Folder,
        session: &mut dyn ImapSession,
    ) -> Result<()>;
}

pub trait TaskRunnerFactory: Send + Sync {
    /// A runner bound to the calling thread's store.
    fn remote_runner<'a>(
        &self,
        store: &'a Store,
        ctx: &WorkerContext,
    ) -> Box<dyn RemoteTaskRunner + 'a>;

    /// Saves a task the engine queues itself and runs its local phase on the calling
    /// thread's store, as `task.queue` does on the account's `main` thread (`performLocal`).
    /// The caller wakes the foreground worker for the remote phase.
    fn perform_local(&self, _store: &Store, _ctx: &WorkerContext, _task: &mut Task) -> Result<()> {
        Err(airpost_core::Error::other(
            "this engine runs no local task phases",
        ))
    }
}
