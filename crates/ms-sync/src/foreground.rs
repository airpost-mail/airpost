// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `foreground` thread: IDLE on the primary folder, body fetches, task remote phases
//! (`runForegroundSyncWorker`, `SyncWorker::idleCycleIteration`).
//!
//! Derived from Mailspring-Sync `MailSync/SyncWorker.cpp` and the worker loop in
//! `MailSync/main.cpp`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::sync::{Condvar, Mutex};

use ms_core::constants::{ls, WORKER_SLEEP};
use ms_core::{logmsg, Error, ErrorCode, Result, SyncError};
use ms_imap::{IdleInterrupt, ImapSession};
use ms_store::{Folder, Message, Model, Query, Store, Task};
use tracing::{error, info, warn};

use crate::worker::{ls_u32, set_ls, Range, SyncWorker};
use crate::WorkerContext;

/// What other threads can ask of the foreground worker: end its IDLE now, and fetch some
/// bodies next (`idleInterrupt`, `idleQueueBodiesToSync`).
#[derive(Default)]
pub struct ForegroundControl {
    interrupt: IdleInterrupt,
    bodies: Mutex<Vec<String>>,
    /// Wakes a worker on a server without IDLE, which waits here instead (`idleCv`).
    signalled: Mutex<bool>,
    cv: Condvar,
}

impl ForegroundControl {
    pub fn new() -> Self {
        ForegroundControl::default()
    }

    pub fn interrupt(&self) {
        self.interrupt.interrupt();
        *self.signalled.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
    }

    /// The handle the worker passes to `ImapSession::idle`.
    pub fn idle_interrupt(&self) -> &IdleInterrupt {
        &self.interrupt
    }

    /// Blocks until [`ForegroundControl::interrupt`] is called, or returns at once if it
    /// was called since the last wait. Leaves the IDLE interrupt pending, so the next
    /// iteration still sees it.
    pub fn wait_for_interrupt(&self) {
        let mut signalled = self.signalled.lock().unwrap_or_else(|e| e.into_inner());
        while !*signalled {
            signalled = self.cv.wait(signalled).unwrap_or_else(|e| e.into_inner());
        }
        *signalled = false;
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

    /// The most recently queued body request (`idleFetchBodyIDs.back()`).
    fn pop_queued_body(&self) -> Option<String> {
        self.bodies.lock().unwrap_or_else(|e| e.into_inner()).pop()
    }
}

/// The thread body (`runForegroundSyncWorker`). Returns when the account stops.
pub fn run_foreground(ctx: WorkerContext) {
    let mut fg = match Foreground::new(ctx.clone()) {
        Ok(f) => f,
        Err(e) => return ctx.fail_with(&e),
    };
    while !ctx.is_stopping() {
        let result = fg.w.configure().and_then(|()| fg.idle_cycle_iteration());
        match result {
            // Note: an iteration whose IDLE died on a dropped connection proves nothing about
            // the network, so it leaves a connection error the background worker just raised
            // in place (the C++ cleared it here, hiding the error from the client when every
            // socket went stale at once). The next clean iteration or background pass clears it.
            Ok(()) if fg.idle_exited_with_error => {}
            Ok(()) => ctx.deltas.end_connection_error(ctx.account.id()),
            Err(e) => {
                if !crate::report_worker_error(&ctx, &e) {
                    return;
                }
                ctx.sleep_until_wake_or(WORKER_SLEEP);
            }
        }
    }
}

struct Foreground {
    w: SyncWorker,
    /// The last IDLE ended in an error: its connection is gone with the VANISHED it had,
    /// so the inbox is re-selected before the CHANGEDSINCE fetch (`idleExitedWithError`).
    idle_exited_with_error: bool,
}

impl Foreground {
    fn new(ctx: WorkerContext) -> Result<Self> {
        Ok(Foreground {
            w: SyncWorker::new(ctx)?,
            idle_exited_with_error: false,
        })
    }

    /// `idleShouldReloop`: an interrupt since the last check ends this iteration early.
    fn should_reloop(&self) -> bool {
        self.w.ctx.foreground.idle_interrupt().take()
    }

    fn find_by_role(&self, role: &str) -> Result<Option<Folder>> {
        self.w.store.find::<Folder>(
            &Query::new()
                .equal("accountId", self.w.account_id())
                .equal("role", role),
        )
    }

    /// Runs body requests from the client. The inbox is the folder this worker keeps
    /// selected, so a copy there is fetched without a SELECT.
    fn fetch_requested_bodies(&mut self) -> Result<()> {
        let mut preferred: Option<Option<Folder>> = None;
        while let Some(id) = self.w.ctx.foreground.pop_queued_body() {
            if preferred.is_none() {
                preferred = Some(self.find_by_role("inbox")?);
            }
            let Some(mut msg) = self
                .w
                .store
                .find::<Message>(&Query::new().equal("id", &id))?
            else {
                continue;
            };
            info!("{}", logmsg::fetching_body(msg.id()));
            if self.w.session.is_disconnected() {
                warn!("IMAP session not connected, connecting before body fetch");
                if let Err(e) = self.w.session.connect_if_needed() {
                    error!("Failed to connect for body fetch: {}", e.code.name());
                    continue;
                }
                if let Err(e) = self.w.session.login_if_needed() {
                    error!("Failed to login for body fetch: {}", e.code.name());
                    continue;
                }
            }
            let inbox = preferred.as_ref().and_then(Option::as_ref);
            self.w.sync_message_body(&mut msg, inbox)?;
        }
        Ok(())
    }

    /// Runs every task waiting for its remote phase, oldest first. Tasks queued while
    /// these run (a SendDraftTask queueing a SyncbackMetadataTask) are picked up by the next
    /// round instead of waiting for the next iteration.
    fn run_remote_tasks(&mut self) -> Result<()> {
        let w = &mut self.w;
        let mut runner = w.ctx.tasks.remote_runner(&w.store, &w.ctx);
        runner.cleanup_old_tasks_at_runtime()?;
        let mut after_rowid: i64 = -1;
        // A task put back for when the account is online again (a deferred send): the
        // others still run, then the worker sleeps on its error and tries again.
        let mut deferred: Option<Error> = None;
        loop {
            let tasks = remote_tasks_after(&w.store, w.ctx.account.id(), &mut after_rowid)?;
            if tasks.is_empty() {
                return deferred.map_or(Ok(()), Err);
            }
            for mut task in tasks {
                match runner.perform_remote(&mut task, &mut w.session) {
                    Err(e @ Error::Sync(_)) => {
                        deferred.get_or_insert(e);
                    }
                    other => other?,
                }
            }
        }
    }

    fn idle_cycle_iteration(&mut self) -> Result<()> {
        self.fetch_requested_bodies()?;
        if self.should_reloop() {
            return Ok(());
        }

        if let Err(e) = self.w.session.connect_if_needed() {
            let advice = ms_core::tls_failure_advice(e.code, self.w.session.last_tls_error());
            return Err(match advice {
                Some(advice) => {
                    error!("{advice}");
                    e.into_sync(&format!("connectIfNeeded: {advice}"))
                }
                None => e.into_sync("connectIfNeeded"),
            }
            .into());
        }
        if self.should_reloop() {
            return Ok(());
        }
        self.w
            .session
            .login_if_needed()
            .map_err(|e| e.into_sync("loginIfNeeded"))?;
        if self.should_reloop() {
            return Ok(());
        }

        self.run_remote_tasks()?;

        if self.should_reloop() {
            return Ok(());
        }

        // The preferred idle folder: the inbox, or All Mail.
        let inbox = match self.find_by_role("inbox")? {
            Some(f) => f,
            None => self.find_by_role("all")?.ok_or_else(|| {
                SyncError::fatal("no-inbox", "There is no inbox or all folder to IDLE on.")
            })?,
        };
        let mut inbox = inbox;
        let initial = inbox.local_status().clone();
        if self.should_reloop() {
            return Ok(());
        }

        let path = inbox.path().to_owned();
        let started_syncing = inbox
            .local_status()
            .get(ls::SYNCED_MIN_UID)
            .is_some_and(serde_json::Value::is_number);
        if started_syncing {
            if self.idle_exited_with_error {
                self.idle_exited_with_error = false;
                // The dead connection's VANISHED are lost and the reconnect above leaves
                // nothing selected; re-select so the CHANGEDSINCE fetch re-reports them as
                // VANISHED (EARLIER).
                self.w
                    .session
                    .select_if_needed(&path)
                    .map_err(|e| e.into_sync("selectIfNeeded after IDLE exited with an error"))?;
            }

            // Expunges the server reported on this connection, during the IDLE just exited
            // but also alongside the body fetches above. It reports them once, so they are
            // lost unless applied before the FETCH below.
            let vanished = self.w.session.take_vanished(&path);
            self.w
                .delete_vanished_uids(&inbox, &vanished, "this connection")?;

            let remote = self
                .w
                .session
                .status(&path)
                .map_err(|e| e.into_sync("folderStatus"))?;

            // With CONDSTORE but not QRESYNC, an expunge shows only at the next shallow scan.
            if self.w.has_capability("CONDSTORE") {
                self.w
                    .sync_folder_changes_via_condstore(&mut inbox, &remote, false)?;
            } else {
                let uidnext = remote.uid_next;
                let synced_min_uid = ls_u32(inbox.local_status(), ls::SYNCED_MIN_UID);
                let bottom_uid = self
                    .w
                    .store
                    .fetch_message_uid_at_depth(&inbox, 100, uidnext)?
                    .max(synced_min_uid);
                let mut truncated = false;
                if uidnext > bottom_uid {
                    truncated = self
                        .w
                        .sync_folder_uid_range(
                            &inbox,
                            Range::new(bottom_uid, (uidnext - bottom_uid) as u64),
                            false,
                            None,
                        )?
                        .truncated;
                }
                set_ls(&mut inbox, ls::LAST_SHALLOW, self.w.now());
                // The background pass finds new mail by uidnext; advancing it past a
                // truncated fetch would hide the remainder until the next deep scan.
                if !truncated {
                    set_ls(&mut inbox, ls::UIDNEXT, uidnext);
                }
            }

            self.w.sync_message_bodies(&mut inbox)?;
            self.w.store.save_folder_status(&mut inbox, &initial)?;
        }

        if self.should_reloop() {
            return Ok(());
        }
        if self.w.session.supports_idle() {
            info!("{}", logmsg::idling_on_folder(&path));
            let interrupt = self.w.ctx.foreground.idle_interrupt().clone();
            let result = self.w.session.idle(&path, &interrupt);
            let code = match &result {
                Ok(()) => ErrorCode::None,
                Err(e) => e.code,
            };
            info!("Idle exited with code {}", code.as_i32());
            // Not an error: Yandex and others drop IDLE connections abruptly. A reconnect
            // that fails on the next iteration is what goes down the retry path.
            self.idle_exited_with_error = result.is_err();
        } else {
            info!("{}", logmsg::idle_not_supported());
            self.w.ctx.foreground.wait_for_interrupt();
        }
        Ok(())
    }
}

/// Tasks in `remote` with a rowid above `after_rowid`, which is advanced past them.
fn remote_tasks_after(store: &Store, account_id: &str, after_rowid: &mut i64) -> Result<Vec<Task>> {
    let mut stmt = store.conn().prepare_cached(
        "SELECT rowid, data FROM Task WHERE accountId = ? AND status = 'remote' AND rowid > ?",
    )?;
    let rows = stmt.query_map(rusqlite::params![account_id, *after_rowid], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut tasks = Vec::new();
    for row in rows {
        let (rowid, data) = row?;
        *after_rowid = (*after_rowid).max(rowid);
        tasks.push(Task::from_json(serde_json::from_str(&data)?)?);
    }
    Ok(tasks)
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
        c.queue_bodies(["a".to_owned(), "b".to_owned()]);
        assert_eq!(c.pop_queued_body().as_deref(), Some("b"));
        c.interrupt();
        assert!(c.idle_interrupt().take());
        assert!(!c.idle_interrupt().take());
    }

    #[test]
    fn remote_tasks_are_read_in_rowid_rounds() {
        let dir = tempfile::tempdir().unwrap();
        let clock = ms_core::ManualClock::new(1_700_000_000);
        ms_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let deltas = ms_store::DeltaStream::new(Box::new(std::io::sink()));
        let store = Store::open(dir.path(), deltas).unwrap();
        let save = |account: &str, status: &str| {
            let mut t = Task::create("ChangeUnreadTask", account, &serde_json::json!({}));
            t.set_status(status);
            store.save(&mut t).unwrap();
            t.id().to_owned()
        };
        let first = save("acc1", "remote");
        save("acc1", "complete");
        save("acc2", "remote");
        let second = save("acc1", "remote");

        let mut after = -1;
        let ids = |tasks: Vec<Task>| tasks.iter().map(|t| t.id().to_owned()).collect::<Vec<_>>();
        assert_eq!(
            ids(remote_tasks_after(&store, "acc1", &mut after).unwrap()),
            [first, second]
        );
        assert!(remote_tasks_after(&store, "acc1", &mut after)
            .unwrap()
            .is_empty());
        // A task queued by one that ran is picked up by the next round.
        let queued = save("acc1", "remote");
        assert_eq!(
            ids(remote_tasks_after(&store, "acc1", &mut after).unwrap()),
            [queued]
        );
    }

    #[test]
    fn wait_for_interrupt_is_sticky() {
        let c = std::sync::Arc::new(ForegroundControl::new());
        c.interrupt();
        c.wait_for_interrupt();
        let c2 = c.clone();
        let t = std::thread::spawn(move || c2.wait_for_interrupt());
        std::thread::sleep(std::time::Duration::from_millis(20));
        c.interrupt();
        t.join().unwrap();
        // The IDLE flag stays for the worker's next check.
        assert!(c.idle_interrupt().take());
    }
}
