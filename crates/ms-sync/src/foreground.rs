//! The `foreground` thread: IDLE on the primary folder, body fetches, task remote phases
//! (`runForegroundSyncWorker`, `SyncWorker::idleCycleIteration`).

use std::sync::Mutex;

use ms_imap::IdleInterrupt;

use crate::WorkerContext;

/// What other threads can ask of the foreground worker: end its IDLE now, and fetch some
/// bodies next (`idleInterrupt`, `idleQueueBodiesToSync`).
#[derive(Default)]
pub struct ForegroundControl {
    interrupt: IdleInterrupt,
    bodies: Mutex<Vec<String>>,
}

impl ForegroundControl {
    pub fn new() -> Self {
        ForegroundControl::default()
    }

    pub fn interrupt(&self) {
        self.interrupt.interrupt();
    }

    /// The handle the worker passes to `ImapSession::idle`.
    pub fn idle_interrupt(&self) -> &IdleInterrupt {
        &self.interrupt
    }

    /// `need-bodies`: queued in order, duplicates dropped.
    pub fn queue_bodies(&self, ids: impl IntoIterator<Item = String>) {
        let mut queued = self.bodies.lock().unwrap_or_else(|e| e.into_inner());
        for id in ids {
            if !queued.contains(&id) {
                queued.push(id);
            }
        }
    }

    pub fn take_queued_bodies(&self) -> Vec<String> {
        std::mem::take(&mut *self.bodies.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// The thread body. P0: no IMAP yet, so it waits for wake-ups and interrupts and does
/// nothing with them.
pub fn run_foreground(ctx: WorkerContext) {
    tracing::info!("Foreground worker is not implemented in this build; idling.");
    loop {
        ms_core::wake::sleep_worker_until_wake_or(ms_core::constants::WORKER_SLEEP);
        ctx.foreground.idle_interrupt().take();
        ctx.foreground.take_queued_bodies();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_queue_dedupes_and_drains() {
        let c = ForegroundControl::new();
        c.queue_bodies(["a".to_owned(), "b".to_owned()]);
        c.queue_bodies(["a".to_owned(), "c".to_owned()]);
        assert_eq!(c.take_queued_bodies(), ["a", "b", "c"]);
        assert!(c.take_queued_bodies().is_empty());
        c.interrupt();
        assert!(c.idle_interrupt().take());
        assert!(!c.idle_interrupt().take());
    }
}
