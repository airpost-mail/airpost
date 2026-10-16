// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Several accounts' runtimes in one process, against an IMAP port nothing listens on:
//! each account logs to its own file, wakes and stops alone, and a failure on one of its
//! threads stops only it. One test, because the log subscriber is process-wide.

use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ms_core::{Account, ManualClock, SyncError};
use ms_imap::ImapSession;
use ms_net::{LocalOnly, MetadataBackend, MetadataDelta, MetadataPush, TokenManager};
use ms_store::{DeltaStream, Folder, Store, Task};
use ms_sync::runtime::{next_command, Dispatcher};
use ms_sync::{AccountFailure, AccountRuntime, RemoteTaskRunner, TaskRunnerFactory, WorkerContext};
use serde_json::json;

const LOG_TARGET: &str = "ms_sync::runtime_test";
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

enum Command {
    Panic,
    Fail,
}

struct NoTasks;

impl RemoteTaskRunner for NoTasks {
    fn cleanup_old_tasks_at_runtime(&mut self) -> ms_core::Result<()> {
        Ok(())
    }
    fn perform_remote(&mut self, _: &mut Task, _: &mut dyn ImapSession) -> ms_core::Result<()> {
        Ok(())
    }
    fn remove_late_sent_copies(
        &mut self,
        _: &Folder,
        _: &mut dyn ImapSession,
    ) -> ms_core::Result<()> {
        Ok(())
    }
}

impl TaskRunnerFactory for NoTasks {
    fn remote_runner<'a>(&self, _: &'a Store, _: &WorkerContext) -> Box<dyn RemoteTaskRunner + 'a> {
        Box::new(NoTasks)
    }
}

/// A port that refuses connections, so every IMAP connect fails at once and retryably.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn account(id: &str, port: u16) -> Arc<Account> {
    // Ids starting with "t" have no start delay.
    assert!(id.starts_with('t'));
    Arc::new(
        Account::from_json(json!({
            "id": id, "provider": "imap", "emailAddress": format!("{id}@example.test"),
            "settings": {
                "imap_host": "127.0.0.1", "imap_port": port, "imap_security": "none",
                "imap_username": id, "imap_password": "pw",
                "smtp_host": "127.0.0.1", "smtp_port": port, "smtp_password": "pw",
            },
        }))
        .unwrap(),
    )
}

struct Started {
    runtime: AccountRuntime<Command>,
    failures: mpsc::Receiver<AccountFailure>,
}

/// A Tern server's stream that stays open: `pull` returns only once the account stops.
struct EndlessStream;

impl MetadataBackend for EndlessStream {
    fn push(&self, _: &MetadataPush) -> ms_core::Result<()> {
        Ok(())
    }

    fn pull(
        &self,
        cursor: Option<&str>,
        stop: &dyn Fn() -> bool,
        _: &mut dyn FnMut(MetadataDelta),
    ) -> ms_core::Result<String> {
        while !stop() {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(cursor.unwrap_or("0").to_owned())
    }
}

fn start(dir: &Path, id: &str, port: u16) -> Started {
    start_with(dir, id, port, Arc::new(LocalOnly))
}

fn start_with(dir: &Path, id: &str, port: u16, metadata: Arc<dyn MetadataBackend>) -> Started {
    ms_core::log::open_account_log(dir, id).unwrap();
    let (tx, failures) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let ctx = WorkerContext::new(
        account(id, port),
        dir.to_owned(),
        DeltaStream::new(Box::new(std::io::sink())),
        Arc::new(TokenManager::new(None)),
        metadata,
        Arc::new(NoTasks),
        Arc::new(move |f: &AccountFailure| {
            tracing::error!(target: LOG_TARGET, "{}", f.message);
            let _ = tx.lock().unwrap().send(f.clone());
        }),
    );
    let dispatcher: Dispatcher<Command> = Box::new(|ctx, rx, _| {
        tracing::info!(target: LOG_TARGET, "dispatcher for {}", ctx.account.id());
        while let Some(command) = next_command(&ctx, &rx) {
            match command {
                Command::Panic => panic!("boom"),
                Command::Fail => ctx.fail_with(&SyncError::fatal("test", "forced").into()),
            }
        }
    });
    Started {
        runtime: AccountRuntime::start(ctx, Some(dispatcher)),
        failures,
    }
}

fn log_of(dir: &Path, name: &str) -> String {
    ms_core::log::flush();
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn account_log(dir: &Path, id: &str) -> String {
    log_of(dir, &format!("mailsync-{id}.log"))
}

fn sleeps_logged(dir: &Path, id: &str) -> usize {
    account_log(dir, id)
        .lines()
        .filter(|l| l.contains("[background] [info] --sleeping"))
        .count()
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn has_thread(runtime: &AccountRuntime<Command>, name: &str) -> bool {
    runtime.running_threads().iter().any(|t| t == name)
}

#[test]
fn accounts_in_one_process_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    ms_store::migrate::migrate(dir, &ManualClock::new(1_000_000), &mut Vec::new()).unwrap();
    ms_core::log::init_file(&dir.join("engine.log")).unwrap();
    let port = closed_port();

    let one = start(dir, "t-one", port);
    let two = start(dir, "t-two", port);
    for id in ["t-one", "t-two"] {
        wait_for(&format!("{id}'s first failed pass"), || {
            sleeps_logged(dir, id) >= 1
        });
    }

    // Each account's lines are in its own file, under the thread that logged them.
    tracing::info!(target: LOG_TARGET, "from a thread without an account");
    let (log_one, log_two) = (account_log(dir, "t-one"), account_log(dir, "t-two"));
    assert!(
        log_one.contains("[main] [info] dispatcher for t-one"),
        "{log_one}"
    );
    assert!(!log_one.contains("dispatcher for t-two"), "{log_one}");
    assert!(
        log_two.contains("[main] [info] dispatcher for t-two"),
        "{log_two}"
    );
    assert!(log_two.contains("[background] [error] "), "{log_two}");
    let engine = log_of(dir, "engine.log");
    assert!(
        engine.contains("from a thread without an account"),
        "{engine}"
    );
    assert!(!engine.contains("dispatcher for"), "{engine}");
    assert!(!engine.contains("[background]"), "{engine}");

    // wake-workers for one account ends only its background sleep.
    let sleeps_two = sleeps_logged(dir, "t-two");
    one.runtime.context().wake_workers();
    wait_for("t-one's woken pass", || sleeps_logged(dir, "t-one") >= 2);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sleeps_logged(dir, "t-two"), sleeps_two);

    // A panic on an account's thread stops that account only.
    let panicking = start(dir, "t-panic", port);
    let sender = panicking.runtime.commands().unwrap();
    sender.send(Command::Panic).unwrap();
    let failure = panicking
        .failures
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(failure.thread, "main");
    assert_eq!(failure.message, "Unhandled panic on thread main: boom");
    assert_eq!(failure.error["key"], "unhandled-exception");
    // The handler runs before the stop is requested, on the failing thread.
    wait_for("t-panic's stop", || {
        panicking.runtime.context().is_stopping()
    });
    assert!(panicking.runtime.commands().is_none());
    assert_eq!(panicking.runtime.stop(STOP_TIMEOUT), Vec::<String>::new());
    assert!(panicking.runtime.running_threads().is_empty());
    let log = account_log(dir, "t-panic");
    assert!(
        log.contains("[main] [error] Unhandled panic on thread main: boom"),
        "{log}"
    );

    // So does a fatal error, reported once with its SyncError.
    let failing = start(dir, "t-fail", port);
    failing
        .runtime
        .commands()
        .unwrap()
        .send(Command::Fail)
        .unwrap();
    let failure = failing
        .failures
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(failure.error["key"], "test");
    assert_eq!(failure.error["retryable"], false);
    assert_eq!(failing.runtime.stop(STOP_TIMEOUT), Vec::<String>::new());
    assert!(failing.failures.try_recv().is_err());

    for runtime in [&one.runtime, &two.runtime] {
        for name in ["main", "background", "calContacts", "metadataExpiration"] {
            assert!(has_thread(runtime, name), "{name} stopped");
        }
    }

    // A cooperative stop ends sleeping workers well before their timeouts, and leaves the
    // other account running.
    let started = Instant::now();
    assert_eq!(one.runtime.stop(STOP_TIMEOUT), Vec::<String>::new());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(one.runtime.running_threads().is_empty());
    assert!(one.failures.try_recv().is_err());
    for name in ["main", "background", "calContacts", "metadataExpiration"] {
        assert!(has_thread(&two.runtime, name), "{name} stopped");
    }
    let sleeps_two = sleeps_logged(dir, "t-two");
    two.runtime.context().wake_workers();
    wait_for("t-two's woken pass", || {
        sleeps_logged(dir, "t-two") > sleeps_two
    });

    // A late stop from an earlier run of an account (a thread left past the timeout that
    // then fails) leaves a later run's expiration signal open.
    let again = start(dir, "t-one", port);
    one.runtime.context().request_stop();
    let signal = again.runtime.context().expiration_signal().clone();
    assert!(!Arc::ptr_eq(
        &signal,
        one.runtime.context().expiration_signal()
    ));
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    };
    assert!(
        !signal.wait_until(now() + 1, &now),
        "closed by the earlier run"
    );
    assert_eq!(again.runtime.stop(STOP_TIMEOUT), Vec::<String>::new());

    // An open metadata stream ends with the stop rather than holding it up.
    let streaming = start_with(dir, "t-stream", port, Arc::new(EndlessStream));
    wait_for("t-stream's metadata thread", || {
        has_thread(&streaming.runtime, "metadata")
    });
    let started = Instant::now();
    assert_eq!(streaming.runtime.stop(STOP_TIMEOUT), Vec::<String>::new());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );

    assert_eq!(two.runtime.stop(STOP_TIMEOUT), Vec::<String>::new());
    for id in ["t-one", "t-two", "t-panic", "t-fail", "t-stream"] {
        ms_core::log::close_account_log(id);
    }
}
