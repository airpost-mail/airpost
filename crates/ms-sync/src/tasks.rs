//! The task runner seam. `TaskProcessor` lives in ms-tasks, which depends on this crate,
//! so workers reach it through these traits and the bin passes ms-tasks' factory in.

use ms_core::Result;
use ms_imap::ImapSession;
use ms_store::{Folder, Store, Task};

use crate::WorkerContext;

/// The parts of `TaskProcessor` the sync workers call.
pub trait RemoteTaskRunner {
    /// Prunes finished tasks beyond the last 100 (`cleanupOldTasksAtRuntime`).
    fn cleanup_old_tasks_at_runtime(&mut self) -> Result<()>;
    /// Runs a task's remote phase and saves its final status (`performRemote`).
    fn perform_remote(&mut self, task: &mut Task, session: &mut dyn ImapSession) -> Result<()>;
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
}
