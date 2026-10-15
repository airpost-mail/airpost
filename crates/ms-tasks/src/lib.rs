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

use ms_core::constants::{ls, SENT_COPY_CLEANUP_WINDOW};
use ms_core::{logmsg, Account, Result, UidSet};
use ms_imap::ImapSession;
use ms_store::model::{json_i64, json_str};
use ms_store::{Folder, Message, Model, Query, Store, Task};
use ms_sync::{RemoteTaskRunner, TaskRunnerFactory, WorkerContext};
use serde_json::Value;
use tracing::info;

/// `cleanupOldTasksAtRuntime` keeps this many complete / cancelled tasks...
const KEPT_FINISHED_TASKS: i64 = 100;
/// ...and runs only once more than this many beyond them have accumulated.
const FINISHED_TASKS_SLOP: i64 = 10;

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
    /// Keeps the last 100 finished tasks, once more than 10 beyond that have piled up. They
    /// are removed one by one rather than with a DELETE, because the client observes the
    /// Task table and releases only rows it gets an `unpersist` for.
    fn cleanup_old_tasks_at_runtime(&mut self) -> Result<()> {
        let finished = || {
            Query::new()
                .equal("accountId", self.account.id())
                .equal_any("status", ["complete", "cancelled"])
        };
        let count_to_remove = self.store.count::<Task>(&finished())? - KEPT_FINISHED_TASKS;
        if count_to_remove <= FINISHED_TASKS_SLOP {
            return Ok(());
        }
        self.store.transaction("cleanupOldTasksAtRuntime", |s| {
            let unneeded = finished()
                .order_by("rowid", true)
                .limit(count_to_remove as usize);
            for mut task in s.find_all::<Task>(&unneeded)? {
                s.remove(&mut task)?;
            }
            Ok(())
        })
    }

    fn perform_remote(&mut self, task: &mut Task, session: &mut dyn ImapSession) -> Result<()> {
        let _ = (task, session);
        todo!("P3-T: TaskProcessor::performRemote")
    }

    /// A sent message appended to Sent because the server's own copy had not appeared yet
    /// may get that copy minutes later (Exchange Online, ~3 min, 2026-09-27), and each
    /// recipient of a multisend adds one. Within `SENT_COPY_CLEANUP_WINDOW` of the send,
    /// while the appended copy is still there, every other copy in Sent is removed from
    /// the server and the store (`removeLateSentCopies`).
    fn remove_late_sent_copies(
        &mut self,
        sent: &Folder,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let now = self.store.now();
        // Runs every pass: start from the messages dated within the window (their Date is
        // the send time) so the account's date index keeps this off the rest of Sent.
        let message_ids: Vec<String> = self
            .store
            .conn()
            .prepare_cached(
                "SELECT mf.messageId FROM Message m JOIN MessageFolder mf ON mf.messageId = m.id \
                 WHERE m.accountId = ? AND m.date >= ? AND mf.folderId = ? AND mf.remoteUID > 0 \
                 GROUP BY mf.messageId HAVING COUNT(*) > 1",
            )?
            .query_map(
                rusqlite::params![self.account.id(), now - SENT_COPY_CLEANUP_WINDOW, sent.id()],
                |r| r.get(0),
            )?
            .collect::<rusqlite::Result<_>>()?;

        for message_id in message_ids {
            let Some(msg) = self
                .store
                .find::<Message>(&Query::new().equal("id", &message_id))?
            else {
                continue;
            };
            let Some(asc) = msg.json().get("_asc") else {
                continue;
            };
            if json_str(asc, "f") != sent.id() || json_i64(&asc["x"]) < now {
                continue;
            }
            let keep_uid = json_i64(&asc["u"]) as u32;
            let mut late = UidSet::new();
            let mut kept_present = false;
            for p in self.store.placements_for_message(&message_id)? {
                if p.folder_id != sent.id() || p.remote_uid == 0 {
                    continue;
                }
                if p.remote_uid == keep_uid {
                    kept_present = true;
                } else {
                    late.insert(p.remote_uid);
                }
            }
            if !kept_present || late.is_empty() {
                continue;
            }
            if sent.local_status().get(ls::SERVER_SAVES_SENT).is_none() {
                let mut sent = sent.clone();
                let initial = sent.local_status().clone();
                sent.local_status_mut()
                    .insert(ls::SERVER_SAVES_SENT.into(), Value::Bool(true));
                self.store.save_folder_status(&mut sent, &initial)?;
            }
            info!(
                "{}",
                logmsg::removing_late_sent_copies(
                    late.iter().count(),
                    &message_id,
                    sent.path(),
                    keep_uid
                )
            );
            let _ = &session;
            // Only a message the send path stamped with `_asc` gets here.
            todo!("P3-S: _removeSentCopies / _removeMessagesResilient")
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use ms_store::DeltaStream;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let clock = ms_core::ManualClock::new(1_700_000_000);
        ms_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
        (dir, store)
    }

    fn save_tasks(store: &Store, account_id: &str, status: &str, n: usize) -> Vec<String> {
        (0..n)
            .map(|_| {
                let mut t = Task::create("ChangeStarredTask", account_id, &json!({}));
                t.set_status(status);
                store.save(&mut t).unwrap();
                t.id().to_owned()
            })
            .collect()
    }

    fn ids(store: &Store, account_id: &str) -> Vec<String> {
        let q = Query::new()
            .equal("accountId", account_id)
            .order_by("rowid", true);
        store
            .find_all::<Task>(&q)
            .unwrap()
            .iter()
            .map(|t| t.id().to_owned())
            .collect()
    }

    #[test]
    fn cleanup_keeps_the_newest_hundred_finished_tasks() {
        let (_dir, store) = store();
        let account = Arc::new(Account::from_json(json!({"id": "acc1"})).unwrap());
        let mut oldest = save_tasks(&store, "acc1", "complete", 8);
        oldest.extend(save_tasks(&store, "acc1", "cancelled", 8));
        let remote = save_tasks(&store, "acc1", "remote", 3);
        let newer = save_tasks(&store, "acc1", "complete", 100);
        let other_account = save_tasks(&store, "acc2", "complete", 120);

        let mut p = TaskProcessor::new(account, &store);
        p.cleanup_old_tasks_at_runtime().unwrap();

        let mut expected = remote.clone();
        expected.extend(newer);
        assert_eq!(ids(&store, "acc1"), expected);
        assert_eq!(ids(&store, "acc2"), other_account);
    }

    #[test]
    fn cleanup_waits_for_more_than_ten_extra() {
        let (_dir, store) = store();
        let account = Arc::new(Account::from_json(json!({"id": "acc1"})).unwrap());
        let all = save_tasks(&store, "acc1", "complete", 110);
        TaskProcessor::new(account, &store)
            .cleanup_old_tasks_at_runtime()
            .unwrap();
        assert_eq!(ids(&store, "acc1"), all);
    }
}
