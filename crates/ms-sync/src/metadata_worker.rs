//! The `metadata` thread: pulls plugin metadata from the backend (`MetadataWorker`). Not
//! started for [`ms_net::LocalOnly`].

use crate::WorkerContext;

/// The thread body. P0: returns at once for a local-only backend (the bin does not start
/// the thread then); otherwise idles.
pub fn run_metadata_worker(ctx: WorkerContext) {
    if ctx.metadata.is_local_only() {
        return;
    }
    loop {
        std::thread::park();
    }
}
