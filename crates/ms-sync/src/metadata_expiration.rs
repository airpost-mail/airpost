//! The `metadataExpiration` thread: emits `metadata-expiration` deltas when snoozes,
//! send-later and reminders come due (`MetadataExpirationWorker`).

use std::time::Duration;

use ms_core::wake::expiration_signal;

use crate::WorkerContext;

/// The thread body. P0: waits on the account's expiration signal and does nothing.
pub fn run_metadata_expiration(ctx: WorkerContext) {
    let signal = expiration_signal(ctx.account.id());
    let clock = ctx.clock.clone();
    loop {
        signal.wait(&|| clock.now(), Duration::from_secs(3600));
    }
}
