// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `calContacts` thread: CalDAV, CardDAV and Google People on a fixed cadence
//! (`runCalContactsSyncWorker`), and the one-off `calendar` thread for `sync-calendar`.
//!
//! Derived from Mailspring-Sync `MailSync/main.cpp` (`runCalContactsSyncWorker`, the
//! `calContacts` thread start and the `sync-calendar` command) and `DAVWorker::run`,
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! The cadence is fixed and ignores `wake-workers`: Google's Calendar and People API
//! quotas ran out within hours when every manual sync also synced contacts (main.cpp).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ms_core::constants::{
    CALENDAR_SYNC_INTERVAL, CAL_CONTACTS_INITIAL_DELAY, CONTACT_SYNC_INTERVAL, DAV_FATAL_SUSPEND,
    DAV_QUOTA_SUSPEND,
};
use ms_core::{logmsg, Error, Result};
use ms_store::Store;
use tracing::{error, info};

use crate::dav::DavWorker;
use crate::google_contacts::GoogleContactsWorker;
use crate::WorkerContext;

/// The thread body. Returns when the account stops or can no longer authenticate.
pub fn run_cal_contacts(ctx: WorkerContext) {
    // Gives the mail workers a head start, so they don't all open the database at once.
    if ctx.pause(Duration::from_secs(
        CAL_CONTACTS_INITIAL_DELAY + ctx.account.start_delay(),
    )) {
        return;
    }
    let store = match open_store(&ctx) {
        Ok(s) => s,
        Err(e) => return stop_unexpected(&e),
    };
    let mut workers = match CalContactsWorkers::new(&ctx, &store) {
        Ok(w) => w,
        Err(e) => return stop_unexpected(&e),
    };

    let mut schedule = Schedule::default();
    for pass in 1u64.. {
        let with_contacts = schedule.take_contacts_due(Instant::now());
        info!("{}", logmsg::calendar_sync_pass(pass, with_contacts));
        let result = if with_contacts {
            workers.run()
        } else {
            workers.run_calendars()
        };
        match result {
            Ok(()) => info!("{}", logmsg::calendar_sync_pass_complete(pass)),
            Err(e) => {
                let failure = PassFailure::of(&e);
                info!("{}", logmsg::calendar_sync_pass_failed(pass));
                match failure {
                    PassFailure::Stop => return,
                    PassFailure::Suspend(d) => {
                        if ctx.pause(d) {
                            return;
                        }
                    }
                    PassFailure::Retry => {}
                }
            }
        }
        if ctx.pause(Duration::from_secs(CALENDAR_SYNC_INTERVAL)) {
            return;
        }
    }
}

/// `sync-calendar`: one calendars-only pass on its own thread and store. Contacts keep
/// their own cadence: they spend a scarcer quota and have no ctag early-out. A failure is
/// logged; the periodic pass retries on its schedule.
pub fn run_calendar_pass(ctx: WorkerContext) {
    let result =
        open_store(&ctx).and_then(|store| CalContactsWorkers::new(&ctx, &store)?.run_calendars());
    if let Err(e) = result {
        error!("{}", crate::worker_error_text(&e));
    }
}

fn open_store(ctx: &WorkerContext) -> Result<Store> {
    let store = Store::open(&ctx.config_dir, ctx.deltas.clone())?;
    store.set_clock(ctx.clock.clone());
    Ok(store)
}

fn stop_unexpected(e: &Error) {
    error!("{}", crate::worker_error_text(e));
    info!("Stopping cal/contacts sync. In the future this will kill the mailsync process.");
}

/// The account's calendar and contact workers. Gmail syncs contacts through the People
/// API; every other account through CardDAV, and every account's calendars through
/// CalDAV (the Gmail CalDAV home is fixed). The DAV worker lives as long as the thread,
/// so its discovery results do too.
struct CalContactsWorkers<'a> {
    google: Option<GoogleContactsWorker<'a>>,
    dav: DavWorker<'a>,
}

impl<'a> CalContactsWorkers<'a> {
    fn new(ctx: &WorkerContext, store: &'a Store) -> Result<Self> {
        let google = if ctx.account.provider() == "gmail" {
            Some(GoogleContactsWorker::new(
                ctx.account.clone(),
                store,
                Arc::clone(&ctx.tokens),
            )?)
        } else {
            None
        };
        let dav = DavWorker::new(
            ctx.account.clone(),
            store,
            Arc::clone(&ctx.tokens),
            ctx.verbose,
        )?;
        Ok(CalContactsWorkers { google, dav })
    }

    /// A pass with contacts: Google People first, then `DAVWorker::run` (CardDAV contacts,
    /// which Gmail skips, then calendars).
    fn run(&mut self) -> Result<()> {
        if let Some(google) = &self.google {
            google.run()?;
        }
        self.dav.run()
    }

    fn run_calendars(&mut self) -> Result<()> {
        self.dav.run_calendars()
    }
}

/// When contacts are due. By elapsed time rather than pass count, since a pass can end in
/// a long suspension; a failed contacts pass counts as run, so a failing server is retried
/// on the contact cadence.
#[derive(Debug, Default)]
struct Schedule {
    last_contact_sync: Option<Instant>,
}

impl Schedule {
    fn take_contacts_due(&mut self, now: Instant) -> bool {
        let due = self.last_contact_sync.is_none_or(|last| {
            now.duration_since(last) >= Duration::from_secs(CONTACT_SYNC_INTERVAL)
        });
        if due {
            self.last_contact_sync = Some(now);
        }
        due
    }
}

/// What the loop does after a failed pass (the `catch` arms of `runCalContactsSyncWorker`).
/// Calendar and contact failures never end the process or put the account in an error
/// state; they back off and retry, except when the account can no longer authenticate.
#[derive(Debug, PartialEq, Eq)]
enum PassFailure {
    /// Sleep this long, then the normal interval.
    Suspend(Duration),
    /// Wait the normal interval.
    Retry,
    /// End the thread.
    Stop,
}

impl PassFailure {
    fn of(e: &Error) -> PassFailure {
        error!("{}", crate::worker_error_text(e));
        let Some(s) = e.as_sync() else {
            info!("Stopping cal/contacts sync. In the future this will kill the mailsync process.");
            return PassFailure::Stop;
        };
        if s.debuginfo.contains("dailyLimitExceeded") {
            info!("Suspending sync for 4 hours - Google app API request quota hit.");
            PassFailure::Suspend(Duration::from_secs(DAV_QUOTA_SUSPEND))
        } else if s.key.contains("Code: 403")
            || s.key.contains("Code: 401")
            || s.debuginfo.contains("invalid_grant")
        {
            info!("Stopping sync - unable to authenticate.");
            PassFailure::Stop
        } else if !s.is_retryable() {
            error!("Suspending sync for 90min - unlikely a retry would resolve this error.");
            PassFailure::Suspend(Duration::from_secs(DAV_FATAL_SUSPEND))
        } else {
            PassFailure::Retry
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ms_core::SyncError;

    #[test]
    fn contacts_run_first_then_every_75_minutes() {
        let start = Instant::now();
        let mut s = Schedule::default();
        assert!(s.take_contacts_due(start));
        let minutes = |m: u64| start + Duration::from_secs(m * 60);
        assert!(!s.take_contacts_due(minutes(15)));
        assert!(!s.take_contacts_due(minutes(74)));
        assert!(s.take_contacts_due(minutes(75)));
        assert!(!s.take_contacts_due(minutes(90)));
        assert!(s.take_contacts_due(minutes(150)));
    }

    #[test]
    fn failures_back_off_as_main_cpp() {
        let http = |status: u16, body: &str| -> Error {
            SyncError::new(
                format!("Invalid Response Code: {status}"),
                format!("https://people.googleapis.com/v1/x RETURNED {body}"),
                status != 401 && status != 403,
                false,
            )
            .into()
        };
        assert_eq!(
            PassFailure::of(&http(403, r#"{"error":{"reason":"dailyLimitExceeded"}}"#)),
            PassFailure::Suspend(Duration::from_secs(4 * 3600))
        );
        assert_eq!(PassFailure::of(&http(401, "{}")), PassFailure::Stop);
        assert_eq!(PassFailure::of(&http(403, "{}")), PassFailure::Stop);
        let refused: Error = SyncError::fatal("invalid-xoauth2-resp", "invalid_grant").into();
        assert_eq!(PassFailure::of(&refused), PassFailure::Stop);
        let fatal: Error = SyncError::fatal("x", "y").into();
        assert_eq!(
            PassFailure::of(&fatal),
            PassFailure::Suspend(Duration::from_secs(90 * 60))
        );
        assert_eq!(
            PassFailure::of(&http(400, "Sync token is expired.")),
            PassFailure::Retry
        );
        assert_eq!(
            PassFailure::of(&Error::other("type error")),
            PassFailure::Stop
        );
    }
}
