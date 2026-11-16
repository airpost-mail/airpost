// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The undo log (`undo.*`, plan section 4.3): one entry per undoable intent, snooze, send with
//! an undo window or recorded group of tasks, in memory, bounded by
//! [`UNDO_LOG_MAX_ENTRIES`] and [`UNDO_LOG_MAX_SECS`]. An entry keeps the tasks in effect as
//! their local phases left them, which is what the undo is built from
//! ([`airpost_tasks::undo`]); the socket's connections queue the undo and redo tasks.
//!
//! Every method that changes the log returns the entries whose state changed, for
//! `undo.changed`. The log never calls into the [`crate::socket::Hub`], which calls it with
//! its own lock held.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use airpost_core::Clock;
use airpost_ipc::{
    code, undo_state, ErrorObject, UndoEntry, UNDO_LOG_MAX_ENTRIES, UNDO_LOG_MAX_SECS,
};
use airpost_store::model::json_str;
use serde_json::Value;

/// What an entry undoes.
#[derive(Clone, Debug)]
pub enum Kind {
    /// Tasks by account, as their local phases left them. `undo` holds the undo's tasks
    /// while the entry is undone.
    Tasks {
        tasks: Vec<(String, Value)>,
        undo: Vec<(String, Value)>,
    },
    /// A send waiting out its undo window until `until`; its undo is `draft.cancelSend`.
    Send {
        account_id: String,
        header_message_id: String,
        until: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Done,
    /// Undone at this point of the log's sequence, which orders redo.
    Undone(u64),
    /// An undo or redo is being queued; it goes back to `was` if that fails.
    Busy {
        was: Was,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Was {
    Done,
    Undone(u64),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub token: String,
    pub client: String,
    pub action: String,
    pub label: String,
    pub count: usize,
    pub at: i64,
    pub kind: Kind,
    state: State,
}

/// A new entry's description.
pub struct NewEntry {
    pub client: String,
    pub action: &'static str,
    pub label: String,
    pub count: usize,
    pub kind: Kind,
}

/// The entry to undo or redo, taken out of reach of another `undo.apply` until
/// [`UndoLog::finish`] or [`UndoLog::abort`].
pub struct Taken {
    pub token: String,
    pub kind: Kind,
}

#[derive(Default)]
struct Log {
    entries: VecDeque<Entry>,
    seq: u64,
}

pub struct UndoLog {
    log: Mutex<Log>,
    clock: Arc<dyn Clock>,
}

impl Default for UndoLog {
    fn default() -> Self {
        UndoLog::new(Arc::new(airpost_core::SystemClock))
    }
}

fn nothing(what: &str) -> ErrorObject {
    ErrorObject::new(code::NOT_FOUND, format!("nothing to {what}"))
}

impl Entry {
    fn account_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |a: &str| {
            if !out.iter().any(|x| x == a) {
                out.push(a.to_owned());
            }
        };
        match &self.kind {
            Kind::Tasks { tasks, undo } => {
                for (a, _) in tasks.iter().chain(undo) {
                    add(a);
                }
            }
            Kind::Send { account_id, .. } => add(account_id),
        }
        out
    }

    /// The tasks in effect: the change's, or the undo's while undone.
    fn live_tasks(&self) -> &[(String, Value)] {
        match (&self.kind, self.state) {
            (Kind::Tasks { undo, .. }, State::Undone(_)) => undo,
            (Kind::Tasks { tasks, .. }, _) => tasks,
            (Kind::Send { .. }, _) => &[],
        }
    }

    fn expired(&self, now: i64) -> bool {
        match &self.kind {
            Kind::Send { until, .. } => now >= *until,
            Kind::Tasks { .. } => now >= self.at + UNDO_LOG_MAX_SECS,
        }
    }

    fn json(&self, now: i64, gone: Option<&str>) -> UndoEntry {
        let (state, can_undo, can_redo) = match (gone, self.state) {
            (Some(_), _) => (undo_state::GONE, false, false),
            (None, State::Done) => (undo_state::DONE, !self.expired(now), false),
            (None, State::Undone(_)) => (
                undo_state::UNDONE,
                false,
                matches!(self.kind, Kind::Tasks { .. }),
            ),
            (None, State::Busy { was: Was::Done }) => (undo_state::DONE, false, false),
            (None, State::Busy { .. }) => (undo_state::UNDONE, false, false),
        };
        UndoEntry {
            token: self.token.clone(),
            client: self.client.clone(),
            action: self.action.clone(),
            label: self.label.clone(),
            count: self.count,
            at: self.at,
            account_ids: self.account_ids(),
            task_ids: self
                .live_tasks()
                .iter()
                .map(|(_, t)| json_str(t, "id").to_owned())
                .collect(),
            state: state.to_owned(),
            can_undo,
            can_redo,
            reason: gone.map(str::to_owned),
        }
    }
}

impl UndoLog {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        UndoLog {
            log: Mutex::new(Log::default()),
            clock,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Log> {
        self.log.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> i64 {
        self.clock.now()
    }

    /// Drops the entries past their time, as gone.
    fn prune(log: &mut Log, now: i64, changed: &mut Vec<UndoEntry>) {
        log.entries.retain(|e| {
            if matches!(e.state, State::Busy { .. }) || !e.expired(now) {
                return true;
            }
            let reason = match e.kind {
                Kind::Send { .. } => "sent",
                Kind::Tasks { .. } => "expired",
            };
            changed.push(e.json(now, Some(reason)));
            false
        });
    }

    /// Adds an entry. The client's undone entries leave the log: a new change ends what it
    /// could redo.
    pub fn record(&self, new: NewEntry) -> (String, Vec<UndoEntry>) {
        let now = self.now();
        let mut changed = Vec::new();
        let mut log = self.lock();
        Self::prune(&mut log, now, &mut changed);
        log.entries.retain(|e| {
            if e.client == new.client && matches!(e.state, State::Undone(_)) {
                changed.push(e.json(now, Some("undone")));
                return false;
            }
            true
        });
        log.seq += 1;
        let token = format!("u{}-{}", log.seq, airpost_core::ids::random_id());
        let entry = Entry {
            token: token.clone(),
            client: new.client,
            action: new.action.to_owned(),
            label: new.label,
            count: new.count,
            at: now,
            kind: new.kind,
            state: State::Done,
        };
        changed.push(entry.json(now, None));
        log.entries.push_back(entry);
        while log.entries.len() > UNDO_LOG_MAX_ENTRIES {
            if let Some(old) = log.entries.pop_front() {
                changed.push(old.json(now, Some("replaced")));
            }
        }
        (token, changed)
    }

    /// The entry `undo.apply` (`redo` false) or `undo.redo` acts on: `token`'s, else the
    /// newest the client (any client with `any`) can undo, or the one it undid last.
    pub fn take(
        &self,
        client: &str,
        token: Option<&str>,
        any: bool,
        redo: bool,
    ) -> Result<(Taken, Vec<UndoEntry>), ErrorObject> {
        let now = self.now();
        let mut changed = Vec::new();
        let mut log = self.lock();
        Self::prune(&mut log, now, &mut changed);
        let what = if redo { "redo" } else { "undo" };
        let eligible = |e: &Entry| match (redo, e.state) {
            (false, State::Done) => !e.expired(now),
            (true, State::Undone(_)) => matches!(e.kind, Kind::Tasks { .. }),
            _ => false,
        };
        let index = match token {
            Some(t) => {
                let i = log
                    .entries
                    .iter()
                    .position(|e| e.token == t)
                    .ok_or_else(|| nothing(what))?;
                if !eligible(&log.entries[i]) {
                    return Err(ErrorObject::new(
                        code::INVALID_PARAMS,
                        format!("entry {t} can't be {what}ne now"),
                    ));
                }
                i
            }
            None => {
                let mine = |e: &Entry| (any || e.client == client) && eligible(e);
                if redo {
                    log.entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| mine(e))
                        .max_by_key(|(_, e)| match e.state {
                            State::Undone(seq) => seq,
                            _ => 0,
                        })
                        .map(|(i, _)| i)
                } else {
                    log.entries.iter().rposition(mine)
                }
                .ok_or_else(|| nothing(what))?
            }
        };
        let entry = &mut log.entries[index];
        let was = match entry.state {
            State::Undone(seq) => Was::Undone(seq),
            _ => Was::Done,
        };
        entry.state = State::Busy { was };
        Ok((
            Taken {
                token: entry.token.clone(),
                kind: entry.kind.clone(),
            },
            changed,
        ))
    }

    /// The undo or redo of a taken entry is queued: `tasks` are its tasks as their local
    /// phases left them. An undone send, or an undo that had nothing to put back, leaves
    /// the log.
    pub fn finish(&self, token: &str, tasks: Vec<(String, Value)>) -> Vec<UndoEntry> {
        let now = self.now();
        let mut log = self.lock();
        log.seq += 1;
        let seq = log.seq;
        let Some(i) = log.entries.iter().position(|e| e.token == token) else {
            return Vec::new();
        };
        let entry = &mut log.entries[i];
        let State::Busy { was } = entry.state else {
            return Vec::new();
        };
        let gone = match (&mut entry.kind, was) {
            (Kind::Send { .. }, _) => true,
            (Kind::Tasks { undo, .. }, Was::Done) => {
                let empty = tasks.is_empty();
                *undo = tasks;
                entry.state = State::Undone(seq);
                empty
            }
            (Kind::Tasks { tasks: done, undo }, Was::Undone(_)) => {
                *done = tasks;
                undo.clear();
                entry.state = State::Done;
                false
            }
        };
        if gone {
            let entry = log.entries.remove(i).expect("the entry is there");
            return vec![entry.json(now, Some("undone"))];
        }
        vec![log.entries[i].json(now, None)]
    }

    /// The undo or redo of a taken entry failed: it is as it was.
    pub fn abort(&self, token: &str) {
        let mut log = self.lock();
        if let Some(e) = log.entries.iter_mut().find(|e| e.token == token) {
            if let State::Busy { was } = e.state {
                e.state = match was {
                    Was::Done => State::Done,
                    Was::Undone(seq) => State::Undone(seq),
                };
            }
        }
    }

    /// The entry as it is now, for an `undo.apply` answer.
    pub fn entry(&self, token: &str) -> Option<UndoEntry> {
        let now = self.now();
        self.lock()
            .entries
            .iter()
            .find(|e| e.token == token)
            .map(|e| e.json(now, None))
    }

    /// A task of an entry finished with an error: the change it stands for is not (or not
    /// all) in effect, so the entry leaves the log.
    pub fn task_failed(&self, account_id: &str, task_id: &str) -> Vec<UndoEntry> {
        let now = self.now();
        let mut changed = Vec::new();
        self.lock().entries.retain(|e| {
            let hit = e
                .live_tasks()
                .iter()
                .any(|(a, t)| a == account_id && json_str(t, "id") == task_id);
            if hit {
                changed.push(e.json(now, Some("failed")));
            }
            !hit
        });
        changed
    }

    /// The entries newest first, `client`'s only with `mine`.
    pub fn list(&self, client: &str, mine: bool, limit: Option<usize>) -> Vec<UndoEntry> {
        let now = self.now();
        let mut changed = Vec::new();
        let mut log = self.lock();
        Self::prune(&mut log, now, &mut changed);
        log.entries
            .iter()
            .rev()
            .filter(|e| !mine || e.client == client)
            .take(limit.unwrap_or(usize::MAX))
            .map(|e| e.json(now, None))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airpost_core::ManualClock;
    use serde_json::json;

    fn log() -> (UndoLog, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_000));
        (UndoLog::new(clock.clone()), clock)
    }

    fn tasks(client: &str, id: &str) -> NewEntry {
        NewEntry {
            client: client.into(),
            action: "archive",
            label: format!("Archived {id}"),
            count: 1,
            kind: Kind::Tasks {
                tasks: vec![("a".into(), json!({"id": id, "__cls": "ChangeFolderTask"}))],
                undo: Vec::new(),
            },
        }
    }

    fn undo_task(id: &str) -> Vec<(String, Value)> {
        vec![("a".into(), json!({"id": id}))]
    }

    #[test]
    fn undo_takes_the_callers_own_latest_entry() {
        let (log, _) = log();
        let (desktop, _) = log.record(tasks("airpost-desktop", "t1"));
        let (cli, _) = log.record(tasks("airpost-cli", "t2"));

        let (taken, _) = log.take("airpost-desktop", None, false, false).unwrap();
        assert_eq!(taken.token, desktop);
        // Taken: a second undo can't have it, and the CLI's own is the CLI's.
        assert!(log.take("airpost-desktop", None, false, false).is_err());
        let changed = log.finish(&desktop, undo_task("u1"));
        assert_eq!(changed[0].state, "undone");
        assert!(changed[0].can_redo);
        assert_eq!(changed[0].task_ids, ["u1"]);

        let (taken, _) = log.take("airpost-mcp", None, true, false).unwrap();
        assert_eq!(taken.token, cli);
        log.abort(&cli);
        assert!(log.entry(&cli).unwrap().can_undo);

        let entries = log.list("airpost-desktop", false, None);
        assert_eq!(
            entries.iter().map(|e| e.token.as_str()).collect::<Vec<_>>(),
            [cli.as_str(), desktop.as_str()]
        );
        assert_eq!(log.list("airpost-desktop", true, None).len(), 1);
    }

    #[test]
    fn redo_takes_the_last_undone_and_a_new_change_ends_redo() {
        let (log, _) = log();
        let (one, _) = log.record(tasks("c", "t1"));
        let (two, _) = log.record(tasks("c", "t2"));
        for t in [&two, &one] {
            log.take("c", Some(t), false, false).unwrap();
            log.finish(t, undo_task("u"));
        }
        let (taken, _) = log.take("c", None, false, true).unwrap();
        assert_eq!(taken.token, one);
        let changed = log.finish(&one, undo_task("r1"));
        assert_eq!(
            (changed[0].state.as_str(), changed[0].can_undo),
            ("done", true)
        );
        assert_eq!(changed[0].task_ids, ["r1"]);

        let (_, changed) = log.record(tasks("c", "t3"));
        assert!(changed
            .iter()
            .any(|e| e.token == two && e.state == "gone" && e.reason.as_deref() == Some("undone")));
        assert!(log.take("c", None, false, true).is_err());
    }

    #[test]
    fn entries_go_on_failure_expiry_and_the_bound() {
        let (log, clock) = log();
        let (failed, _) = log.record(tasks("c", "t1"));
        let changed = log.task_failed("a", "t1");
        assert_eq!(changed[0].token, failed);
        assert_eq!(changed[0].reason.as_deref(), Some("failed"));
        assert!(log.entry(&failed).is_none());

        let (send, _) = log.record(NewEntry {
            client: "c".into(),
            action: "send",
            label: "Sending".into(),
            count: 1,
            kind: Kind::Send {
                account_id: "a".into(),
                header_message_id: "h".into(),
                until: 1_010,
            },
        });
        assert!(log.entry(&send).unwrap().can_undo);
        clock.set(1_010);
        assert!(log.take("c", Some(&send), false, false).is_err());
        assert!(log.list("c", false, None).is_empty());

        for i in 0..UNDO_LOG_MAX_ENTRIES + 1 {
            log.record(tasks(&format!("c{i}"), &format!("t{i}")));
        }
        assert_eq!(log.list("x", false, None).len(), UNDO_LOG_MAX_ENTRIES);
        clock.set(1_010 + UNDO_LOG_MAX_SECS);
        assert!(log.list("x", false, None).is_empty());
    }

    #[test]
    fn an_undo_with_nothing_to_put_back_leaves_the_log() {
        let (log, _) = log();
        let (t, _) = log.record(tasks("c", "t1"));
        log.take("c", None, false, false).unwrap();
        let changed = log.finish(&t, Vec::new());
        assert_eq!(changed[0].state, "gone");
        assert!(log.entry(&t).is_none());
    }
}
