//! The `background` thread: folder listing, scan passes and the orphan sweep, then it
//! starts `foreground` (`runBackgroundSyncWorker`, `SyncWorker::syncNow`).

use ms_core::constants::{thread, WORKER_SLEEP};
use ms_core::wake::sleep_worker_until_wake_or;

use crate::foreground::run_foreground;
use crate::WorkerContext;

/// The thread body. P0: waits out the account's start delay, starts the foreground
/// thread as the real worker does after its first pass, then idles until woken.
pub fn run_background(ctx: WorkerContext) {
    sleep_worker_until_wake_or(ctx.account.start_delay());
    tracing::info!("Background worker is not implemented in this build; idling.");

    let fg_ctx = ctx.clone();
    if let Err(e) = std::thread::Builder::new()
        .name(thread::FOREGROUND.into())
        .spawn(move || run_foreground(fg_ctx))
    {
        ms_core::critical!("Could not start the foreground thread: {e}");
        std::process::abort();
    }

    loop {
        sleep_worker_until_wake_or(WORKER_SLEEP);
    }
}
