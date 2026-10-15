//! The `calContacts` thread: CalDAV, CardDAV and Google People on a fixed cadence
//! (`runCalContactsSyncWorker`), and the one-off `calendar` thread for `sync-calendar`.

use std::time::Duration;

use ms_core::constants::CAL_CONTACTS_INITIAL_DELAY;

use crate::WorkerContext;

/// The thread body. P0: waits out the initial delay and idles.
pub fn run_cal_contacts(ctx: WorkerContext) {
    std::thread::sleep(Duration::from_secs(
        CAL_CONTACTS_INITIAL_DELAY + ctx.account.start_delay(),
    ));
    loop {
        std::thread::park();
    }
}

/// `sync-calendar`: one calendars-only pass. P0: nothing to do.
pub fn run_calendar_pass(ctx: WorkerContext) {
    let _ = ctx;
}
