// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `--mode serve`: every account in one process (docs/engine-protocol-plan.md section 4).
//!
//! The engine starts with no account, binds `--socket` (default
//! `$CONFIG_DIR_PATH/run/engine.sock`) and keeps `run/engine.json` listing the accounts it
//! holds. The parent's pipes are the control channel: stdin takes JSON-RPC requests, the
//! socket's envelope, for `account.add`, `account.remove`, `account.test`, `account.reset`,
//! `engine.resetDatabase`, `engine.setLogLevel` and `engine.shutdown` plus every socket
//! method; stdout carries their responses and the accounts' secrets deltas (`delta`
//! notifications) and nothing else. Credentials therefore never touch the socket, which
//! refuses those methods. stdin is read only once the socket is bound, so the parent's first
//! response marks the engine ready. The slow parent methods (test, reset, resetDatabase,
//! setLogLevel) run on their own threads and answer when done, so responses can come out of
//! order. Closed stdin still exits 141 after 30 s.
//!
//! Each account runs in its own [`AccountRuntime`] with its own `DeltaStream` into the
//! shared hub, `main` thread and log file. A failure on one of its threads stops that
//! account alone and sends `account.state` `stopped` with the error once its threads have
//! exited; the engine never restarts it, the client may add it again. Engine-wide threads
//! (stdin, socket, signals) still abort the process on a panic.

use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use ms_core::constants::{
    thread, ACCOUNT_RESET_STRAGGLER_WAIT_MS, ACCOUNT_START_WAIT_MS, ACCOUNT_STOP_TIMEOUT_MS,
    DATABASE_FILE, ENGINE_LOG_FILE, EXIT_FAILURE, EXIT_OK, EXIT_SOCKET_IN_USE,
};
use ms_core::Account;
use ms_ipc::{
    account_state, code, method, AccountAddParams, AccountAddResult, AccountIdParams,
    AccountRemoveResult, AccountResetParams, AccountStateParams, AccountStatus, AccountTestParams,
    ErrorObject, Request as RpcRequest, ResetDatabaseParams, ResetDatabaseResult, Response,
    SetLogLevelParams, SetLogLevelResult, MAX_LINE_BYTES,
};
use ms_store::{DeltaSink, DeltaStream};
use ms_sync::{AccountFailure, AccountRuntime, WorkerContext};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::cli::Options;
use crate::dispatch::{self, Command, Reply, Request};
use crate::output::{one_shot_error, write_stdout};
use crate::socket::{self, Accounts, Hub, Server, Session};
use crate::workers::{self, Lane};

type Runtime = AccountRuntime<Command>;

pub fn run(opts: &Options) -> i32 {
    let socket_path = opts
        .socket
        .clone()
        .unwrap_or_else(|| default_socket(&opts.config_dir));
    if let Err(e) = ms_core::log::init_file(&opts.config_dir.join(ENGINE_LOG_FILE)) {
        one_shot_error(&format!("Setup Failed: {e}"));
        return EXIT_FAILURE;
    }
    crate::modes::install_panic_hook(true);
    info!("------------- Starting the engine (serve) ---------------");

    let fail = |message: String| {
        tracing::error!("-X {message}");
        one_shot_error(&message);
        EXIT_FAILURE
    };
    let listener = match crate::platform::uds::bind(&socket_path) {
        Ok(l) => l,
        Err(e) if e.in_use => {
            fail(e.message);
            return EXIT_SOCKET_IN_USE;
        }
        Err(e) => return fail(e.message),
    };
    crate::platform::remove_on_exit(socket_path.clone());
    match socket::write_engine_discovery(&opts.config_dir, &socket_path, &[]) {
        Ok(path) => crate::platform::remove_on_exit(path),
        Err(e) => return fail(format!("Could not write the discovery file: {e}")),
    }
    if let Err(e) = crate::platform::remove_on_termination() {
        warn!("-X Could not watch for SIGTERM; the socket files may stay behind: {e}");
    }

    let hub = Hub::new(socket::queue_limit());
    let database = socket::DatabaseGeneration::default();
    let settings = Arc::new(crate::settings::Settings::open(&opts.config_dir));
    let engine = Arc::new_cyclic(|me| Engine {
        config_dir: opts.config_dir.clone(),
        socket: socket_path.clone(),
        verbose_until: AtomicI64::new(if opts.verbose { i64::MAX } else { 0 }),
        hub: hub.clone(),
        registry: Mutex::new(Registry::default()),
        changed: Condvar::new(),
        resetting: Mutex::new(HashSet::new()),
        resetting_database: AtomicBool::new(false),
        database: database.clone(),
        settings: settings.clone(),
        me: me.clone(),
    });
    let server = Arc::new(Server {
        hub,
        accounts: engine.clone(),
        started: Instant::now(),
        config_dir: opts.config_dir.clone(),
        database,
        settings: Some(settings),
    });
    socket::serve(listener, server.clone());
    info!(
        "{}",
        ms_core::logmsg::socket_listening(&socket_path.to_string_lossy())
    );

    let orphan = opts.orphan;
    crate::modes::spawn(thread::STDIN, move || {
        parent_loop(engine, Session::new(server), orphan)
    });
    // The stdin thread, `engine.shutdown` or a signal ends the process.
    loop {
        std::thread::park();
    }
}

// -- Registry -------------------------------------------------------------------------------

struct Slot {
    account: Arc<Account>,
    /// The identity line `account.add` passed, for restarting the account.
    identity: Value,
    /// Which `account.add` started the current runtime, so a late report from an earlier
    /// one is ignored.
    generation: u64,
    state: &'static str,
    error: Option<Value>,
    deltas: Arc<DeltaStream>,
    runtime: Option<Arc<Runtime>>,
    /// The `accountStop` thread is joining the account's threads after a failure.
    stopping: bool,
    /// `account.remove` or `engine.shutdown` is stopping it.
    removing: bool,
}

#[derive(Default)]
struct Registry {
    /// In the order they were added.
    slots: Vec<Slot>,
    next_generation: u64,
}

impl Registry {
    fn get(&self, id: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.account.id() == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.account.id() == id)
    }

    fn current(&mut self, id: &str, generation: u64) -> Option<&mut Slot> {
        self.get_mut(id).filter(|s| s.generation == generation)
    }

    fn ids(&self) -> Vec<String> {
        self.slots
            .iter()
            .map(|s| s.account.id().to_owned())
            .collect()
    }
}

pub struct Engine {
    config_dir: PathBuf,
    socket: PathBuf,
    /// Unix milliseconds: accounts started before then log protocol traffic
    /// (`engine.setLogLevel`; `--verbose` sets it to forever).
    verbose_until: AtomicI64,
    hub: Arc<Hub>,
    registry: Mutex<Registry>,
    /// Notified on every state change, for `account.add` and `account.remove` waiting on
    /// one.
    changed: Condvar,
    /// Accounts an `account.reset` is deleting; `account.add` refuses them until it is done.
    resetting: Mutex<HashSet<String>>,
    /// `engine.resetDatabase` is running; `account.add` refuses every account.
    resetting_database: AtomicBool,
    /// Bumped when the database file is replaced, so socket sessions reopen it.
    database: socket::DatabaseGeneration,
    /// The settings store, which the accounts' expiration executors read.
    settings: Arc<crate::settings::Settings>,
    /// The engine itself, for the callbacks an account's threads make.
    me: Weak<Engine>,
}

impl Accounts for Engine {
    fn statuses(&self) -> Vec<AccountStatus> {
        self.lock()
            .slots
            .iter()
            .map(|s| AccountStatus {
                account_id: s.account.id().to_owned(),
                email: s.account.email_address().to_owned(),
                state: s.state.to_owned(),
                connection_error: s.deltas.connection_error(),
                error: s.error.clone(),
                account: Some(ms_ipc::policy::public_account_json(s.account.as_json())),
            })
            .collect()
    }

    fn check(&self, account_id: &str) -> Result<(), ErrorObject> {
        match self.lock().get(account_id) {
            Some(_) => Ok(()),
            None => Err(socket::unknown_account(account_id)),
        }
    }

    fn commands(&self, account_id: &str) -> Result<Sender<Command>, ErrorObject> {
        let reg = self.lock();
        let slot = reg
            .get(account_id)
            .ok_or_else(|| socket::unknown_account(account_id))?;
        let commands = (!slot.removing)
            .then(|| slot.runtime.as_ref().and_then(|r| r.commands()))
            .flatten();
        commands.ok_or_else(|| {
            ErrorObject::new(
                code::ACCOUNT_STOPPED,
                format!("account {account_id} is not running"),
            )
            .with_data(json!({ "state": slot.state, "error": slot.error }))
        })
    }

    fn all_commands(&self) -> Vec<Sender<Command>> {
        self.lock()
            .slots
            .iter()
            .filter(|s| !s.removing)
            .filter_map(|s| s.runtime.as_ref().and_then(|r| r.commands()))
            .collect()
    }

    fn has_parent_channel(&self) -> bool {
        true
    }
}

fn invalid(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, message)
}

/// Marks the calling thread as working for an account while it lives, so the lines logged
/// while adding it go to its log.
struct AccountScope(Option<Arc<str>>);

impl AccountScope {
    fn enter(id: &str) -> Self {
        let previous = ms_core::log::thread_account();
        ms_core::log::set_thread_account(Some(id.into()));
        AccountScope(previous)
    }
}

impl Drop for AccountScope {
    fn drop(&mut self) {
        ms_core::log::set_thread_account(self.0.take());
    }
}

/// An account's secrets delta, as a `delta` notification on the parent's stdout.
struct ParentSink {
    account_id: String,
}

impl DeltaSink for ParentSink {
    fn write_item(&mut self, _model_class: &str, line: &str) {
        let note = ms_ipc::delta_notification_line(&self.account_id, line);
        write_stdout(&format!("{note}\n"));
    }
}

impl Engine {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn weak(&self) -> Weak<Engine> {
        self.me.clone()
    }

    /// Waits on [`Engine::changed`] until `done` or `timeout`; returns the guard and
    /// whether `done` held.
    fn wait_until<'a>(
        &self,
        mut reg: MutexGuard<'a, Registry>,
        timeout: Duration,
        done: impl Fn(&Registry) -> bool,
    ) -> (MutexGuard<'a, Registry>, bool) {
        let deadline = Instant::now() + timeout;
        loop {
            if done(&reg) {
                return (reg, true);
            }
            let now = Instant::now();
            if now >= deadline {
                return (reg, false);
            }
            reg = self
                .changed
                .wait_timeout(reg, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn notify_state(&self, id: &str, state: &str, error: Option<Value>) {
        self.hub.account_state(&AccountStateParams {
            account_id: id.to_owned(),
            state: state.to_owned(),
            error,
        });
        self.changed.notify_all();
    }

    fn write_discovery(&self, reg: &Registry) {
        if let Err(e) = socket::write_engine_discovery(&self.config_dir, &self.socket, &reg.ids()) {
            warn!("-X Could not update the discovery file: {e}");
        }
    }

    /// `account.add`: starts the account and answers once its `main` thread is up (or the
    /// account failed to start, or [`ACCOUNT_START_WAIT_MS`] passed).
    fn add(&self, req: &RpcRequest) -> Reply {
        let p: AccountAddParams = req.params_as()?;
        let account = parse_account(p.account)?;
        if self.resetting_database.load(Ordering::SeqCst)
            || self.resetting_lock().contains(account.id())
        {
            return Err(ErrorObject::new(
                code::ACCOUNT_EXISTS,
                format!(
                    "account {} is being reset; add it again later",
                    account.id()
                ),
            ));
        }
        self.start(account, p.identity)
    }

    fn resetting_lock(&self) -> MutexGuard<'_, HashSet<String>> {
        self.resetting.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts `account` and answers once its `main` thread is up (or it failed to start, or
    /// [`ACCOUNT_START_WAIT_MS`] passed).
    fn start(&self, account: Account, identity: Value) -> Reply {
        let identity_text = match &identity {
            Value::Null => "null".to_owned(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        // The error's `log` may echo the line, so only the message is returned.
        let parsed =
            crate::modes::parse_identity(&identity_text).map_err(|e| invalid(e.message))?;
        let account = Arc::new(account);
        let id = account.id().to_owned();

        let reg = self.lock();
        // A failed account whose threads are still being joined is restarted after them.
        let stop_wait = Duration::from_millis(ACCOUNT_STOP_TIMEOUT_MS + 2_000);
        let (mut reg, settled) = self.wait_until(reg, stop_wait, |r| {
            r.get(&id).is_none_or(|s| !s.stopping && !s.removing)
        });
        if !settled {
            return Err(ErrorObject::new(
                code::ACCOUNT_EXISTS,
                format!("account {id} is still stopping; add it again later"),
            ));
        }
        if let Some(slot) = reg.get(&id) {
            if slot.state != account_state::STOPPED {
                return Err(ErrorObject::new(
                    code::ACCOUNT_EXISTS,
                    format!("account {id} is already {}", slot.state),
                ));
            }
        }
        let generation = reg.next_generation;
        reg.next_generation += 1;

        if let Err(e) = ms_core::log::open_account_log(&self.config_dir, &id) {
            warn!("-X Could not open the log of account {id}; its lines go to engine.log: {e}");
        }
        let scope = AccountScope::enter(&id);
        info!(
            "{}",
            ms_core::logmsg::starting_sync(account.email_address())
        );
        let (ctx, deltas) = self.context(account.clone(), parsed, generation);
        let slot = Slot {
            account,
            identity,
            generation,
            state: account_state::STARTING,
            error: None,
            deltas,
            runtime: None,
            stopping: false,
            removing: false,
        };
        match reg.get_mut(&id) {
            Some(old) => *old = slot,
            None => reg.slots.push(slot),
        }
        self.notify_state(&id, account_state::STARTING, None);
        self.write_discovery(&reg);

        let weak = self.weak();
        let started_id = id.clone();
        let dispatcher: ms_sync::runtime::Dispatcher<Command> = Box::new(move |ctx, rx, _| {
            dispatch::run(ctx, rx, move || {
                if let Some(engine) = weak.upgrade() {
                    engine.started(&started_id, generation);
                }
            })
        });
        // The registry stays locked until the runtime is stored, so a thread failing at once
        // finds it to stop.
        let runtime = Arc::new(AccountRuntime::start(ctx, Some(dispatcher)));
        if let Some(slot) = reg.current(&id, generation) {
            slot.runtime = Some(runtime);
        }
        drop(scope);

        let wait = Duration::from_millis(ACCOUNT_START_WAIT_MS);
        let (reg, _) = self.wait_until(reg, wait, |r| {
            r.get(&id)
                .is_none_or(|s| s.generation != generation || s.state != account_state::STARTING)
        });
        let (state, error) = match reg.get(&id).filter(|s| s.generation == generation) {
            Some(s) => (s.state, s.error.clone()),
            None => (account_state::REMOVED, None),
        };
        serde_json::to_value(AccountAddResult {
            account_id: id,
            state: state.to_owned(),
            error,
        })
        .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
    }

    /// The account's context: its own delta stream into the hub (secrets to stdout), token
    /// manager, metadata backend and failure handler.
    fn context(
        &self,
        account: Arc<Account>,
        identity: crate::modes::IdentityLine,
        generation: u64,
    ) -> (WorkerContext, Arc<DeltaStream>) {
        let id = account.id().to_owned();
        let deltas = DeltaStream::with_sink(self.hub.sink(
            &id,
            Box::new(ParentSink {
                account_id: id.clone(),
            }),
        ));
        let metadata =
            crate::modes::metadata_backend(&self.config_dir, &account, identity, &deltas);
        let for_tokens = deltas.clone();
        let tokens = ms_net::TokenManager::new(Some(Box::new(move |a: &Account| {
            for_tokens.send_updated_secrets(a)
        })));
        let weak = self.weak();
        let on_failure = move |failure: &AccountFailure| {
            // Logged at error, not critical: stderr is the process's crash channel.
            tracing::error!("{}", failure.message);
            if let Some(engine) = weak.upgrade() {
                engine.failed(&id, generation, failure);
            }
        };
        let mut ctx = WorkerContext::new(
            account,
            self.config_dir.clone(),
            deltas.clone(),
            Arc::new(tokens),
            metadata,
            Arc::new(ms_tasks::Tasks),
            Arc::new(on_failure),
        );
        ctx.verbose = now_ms() < self.verbose_until.load(Ordering::SeqCst);
        ctx.settings = Some(self.settings.clone());
        (ctx, deltas)
    }

    /// The account's `main` thread opened the database and is taking commands.
    fn started(&self, id: &str, generation: u64) {
        let mut reg = self.lock();
        let Some(slot) = reg.current(id, generation) else {
            return;
        };
        if slot.state == account_state::STARTING && !slot.removing {
            slot.state = account_state::RUNNING;
            self.notify_state(id, account_state::RUNNING, None);
        }
    }

    /// The failure handler, on the failing thread: marks the account stopped and leaves the
    /// join and the notification to an `accountStop` thread, since the failing thread is one
    /// of those to join.
    fn failed(&self, id: &str, generation: u64, failure: &AccountFailure) {
        let mut reg = self.lock();
        let Some(slot) = reg.current(id, generation) else {
            return;
        };
        if slot.removing || slot.stopping {
            return;
        }
        slot.state = account_state::STOPPED;
        slot.error = Some(failure.error.clone());
        slot.stopping = true;
        let runtime = slot.runtime.clone();
        drop(reg);
        self.changed.notify_all();

        let weak = self.weak();
        let stop_id = id.to_owned();
        let spawned =
            ms_core::log::spawn_account_thread(Some(id.into()), thread::ACCOUNT_STOP, move || {
                if let Some(rt) = &runtime {
                    rt.stop(Duration::from_millis(ACCOUNT_STOP_TIMEOUT_MS));
                }
                if let Some(engine) = weak.upgrade() {
                    engine.stopped(&stop_id, generation);
                }
            });
        if let Err(e) = spawned {
            ms_core::critical!("Could not start the {} thread: {e}", thread::ACCOUNT_STOP);
            ms_core::log::flush();
            std::process::abort();
        }
    }

    /// The `accountStop` thread is done: the account's threads exited or were left after
    /// the timeout.
    fn stopped(&self, id: &str, generation: u64) {
        let mut reg = self.lock();
        let Some(slot) = reg.current(id, generation) else {
            return;
        };
        slot.stopping = false;
        slot.runtime = None;
        // A thread left running past the timeout must not publish for the account any more.
        slot.deltas.close();
        let error = slot.error.clone();
        let removing = slot.removing;
        if !removing {
            info!("Account {id} stopped");
            self.notify_state(id, account_state::STOPPED, error);
        } else {
            self.changed.notify_all();
        }
        drop(reg);
        ms_core::log::flush();
    }

    /// `account.remove`: stops the account, forgets it and sends `removed`. Its database rows
    /// stay; `--mode reset` deletes them. The result lists the account's threads still
    /// running after the stop timeout (`stillRunning`), which may still write to the
    /// database: a client about to reset the account should wait and retry.
    fn remove(&self, req: &RpcRequest) -> Reply {
        let p: AccountIdParams = req.params_as()?;
        removed_result(self.remove_account(&p.account_id)?)
    }

    /// Stops the account and forgets it; returns its threads still running.
    fn remove_account(&self, id: &str) -> Result<Vec<String>, ErrorObject> {
        let id = id.to_owned();
        let mut reg = self.lock();
        let slot = reg
            .get_mut(&id)
            .ok_or_else(|| socket::unknown_account(&id))?;
        if slot.removing {
            return Err(ErrorObject::new(
                code::ACCOUNT_STOPPED,
                format!("account {id} is already being removed"),
            ));
        }
        slot.removing = true;
        let stop_wait = Duration::from_millis(ACCOUNT_STOP_TIMEOUT_MS + 2_000);
        let (mut reg, _) =
            self.wait_until(reg, stop_wait, |r| r.get(&id).is_none_or(|s| !s.stopping));
        let (runtime, deltas) = match reg.get_mut(&id) {
            Some(s) => (s.runtime.take(), s.deltas.clone()),
            None => return Ok(Vec::new()),
        };
        drop(reg);

        let mut still_running = Vec::new();
        if let Some(rt) = runtime {
            let _scope = AccountScope::enter(&id);
            still_running = rt.stop(Duration::from_millis(ACCOUNT_STOP_TIMEOUT_MS));
            info!("Account {id} removed");
        }
        deltas.close();

        let mut reg = self.lock();
        reg.slots.retain(|s| s.account.id() != id);
        self.write_discovery(&reg);
        self.notify_state(&id, account_state::REMOVED, None);
        drop(reg);
        ms_core::log::close_account_log(&id);
        Ok(still_running)
    }

    /// `account.test`: `--mode test`'s checks and result, for an account the engine need not
    /// hold. Runs on its own thread.
    fn test_account(&self, account: &Account) -> Reply {
        info!("Testing account {}", account.id());
        Ok(crate::modes::test_auth::result(account))
    }

    /// `account.reset`: forgets the account if the engine holds it, then deletes its rows,
    /// keeping its plugin metadata for the resync when `keep_metadata`. The client adds it
    /// again (or, when removing it, doesn't). Runs on its own thread.
    fn reset_account(&self, id: &str, keep_metadata: bool) -> Reply {
        {
            let mut resetting = self.resetting_lock();
            if !resetting.insert(id.to_owned()) {
                return Err(ErrorObject::new(
                    code::ACCOUNT_EXISTS,
                    format!("account {id} is already being reset"),
                ));
            }
        }
        let outcome = (|| {
            if self.lock().get(id).is_some() {
                let still_running = self.remove_account(id)?;
                if !still_running.is_empty() {
                    info!(
                        "Waiting for {} of account {id} before resetting it",
                        still_running.join(", ")
                    );
                    std::thread::sleep(Duration::from_millis(ACCOUNT_RESET_STRAGGLER_WAIT_MS));
                }
            }
            let sink = ms_store::DeltaStream::new(Box::new(std::io::sink()));
            ms_store::Store::open(&self.config_dir, sink)
                .and_then(|store| store.reset_for_account(id, keep_metadata))
                .map_err(|e| {
                    ErrorObject::new(code::INTERNAL_ERROR, format!("reset failed: {e}"))
                })?;
            info!("Account {id} reset");
            Ok(json!({}))
        })();
        self.resetting_lock().remove(id);
        outcome
    }

    /// `engine.resetDatabase`: stops and forgets every account, deletes the database and
    /// creates an empty one, carrying the plugin metadata over by Message-ID
    /// ([`ms_store::metadata_stash`]) when `keep_metadata`. Socket sessions reopen it on their
    /// next read. Runs on its own thread.
    fn reset_database(&self, keep_metadata: bool) -> Reply {
        if self.resetting_database.swap(true, Ordering::SeqCst) {
            return Err(ErrorObject::new(
                code::INTERNAL_ERROR,
                "the database is already being reset",
            ));
        }
        let outcome = (|| {
            let ids = self.lock().ids();
            let mut stragglers = false;
            for id in &ids {
                stragglers |= !self.remove_account(id)?.is_empty();
            }
            if stragglers {
                std::thread::sleep(Duration::from_millis(ACCOUNT_RESET_STRAGGLER_WAIT_MS));
            }
            // A database that can't be read (often why it is being rebuilt) loses its
            // metadata; the rebuild goes ahead.
            let kept = if keep_metadata {
                ms_store::metadata_stash::collect_database(&self.config_dir).unwrap_or_else(|e| {
                    warn!("-X Could not keep the plugin metadata of the old database: {e}");
                    Vec::new()
                })
            } else {
                Vec::new()
            };
            for suffix in ["", "-wal", "-shm"] {
                let path = self.config_dir.join(format!("{DATABASE_FILE}{suffix}"));
                match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(ErrorObject::new(
                            code::INTERNAL_ERROR,
                            format!("could not delete {}: {e}", path.display()),
                        ));
                    }
                    _ => {}
                }
            }
            self.database.bump();
            ms_store::migrate::migrate(
                &self.config_dir,
                &ms_core::SystemClock,
                &mut std::io::sink(),
            )
            .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, format!("migrate failed: {e}")))?;
            if let Err(e) = ms_store::metadata_stash::save_database(&self.config_dir, &kept) {
                warn!("-X Could not keep the plugin metadata of the old database: {e}");
            }
            info!(
                "Database reset; {} accounts removed, {} plugin metadata values kept",
                ids.len(),
                kept.len()
            );
            serde_json::to_value(ResetDatabaseResult { removed: ids })
                .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
        })();
        self.resetting_database.store(false, Ordering::SeqCst);
        outcome
    }

    /// `engine.setLogLevel`: sets the verbose deadline and restarts the running accounts,
    /// which read it when they start, with their current secrets. Runs on its own thread.
    fn set_log_level(&self, req: &RpcRequest) -> Reply {
        let p: SetLogLevelParams = req.params_as()?;
        self.verbose_until.store(p.verbose_until, Ordering::SeqCst);
        let running: Vec<(Account, Value)> = self
            .lock()
            .slots
            .iter()
            .filter(|s| !s.removing && !s.stopping && s.state != account_state::STOPPED)
            .map(|s| {
                let account = match &s.runtime {
                    Some(rt) => rt.context().tokens.account_with_current_secrets(&s.account),
                    None => (*s.account).clone(),
                };
                (account, s.identity.clone())
            })
            .collect();
        let mut restarted = Vec::new();
        for (account, identity) in running {
            let id = account.id().to_owned();
            self.remove_account(&id)?;
            self.start(account, identity)?;
            restarted.push(id);
        }
        info!(
            "Verbose logging {}; {} accounts restarted",
            if p.verbose_until > now_ms() {
                "on"
            } else {
                "off"
            },
            restarted.len()
        );
        serde_json::to_value(SetLogLevelResult { restarted })
            .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
    }

    /// `engine.shutdown`: stops every account and flushes what they buffered. The caller
    /// answers and exits.
    fn shutdown(&self) {
        let mut reg = self.lock();
        let mut stopping = Vec::new();
        for slot in &mut reg.slots {
            slot.removing = true;
            stopping.push((slot.runtime.take(), slot.deltas.clone()));
        }
        drop(reg);
        // Every account is asked first, so the waits overlap.
        for (rt, _) in &stopping {
            if let Some(rt) = rt {
                rt.context().request_stop();
            }
        }
        for (rt, deltas) in &stopping {
            if let Some(rt) = rt {
                rt.stop(Duration::from_millis(ACCOUNT_STOP_TIMEOUT_MS));
            }
            deltas.close();
        }
        info!("Engine shutdown: {} accounts stopped", stopping.len());
    }

    /// `test.crash` / `test.panic` (debug builds): the account fails as it would on a fatal
    /// error or a bug.
    fn test_failure(&self, req: &RpcRequest) -> Reply {
        let p: AccountIdParams = req.params_as()?;
        let commands = self.commands(&p.account_id)?;
        if req.method == method::TEST_PANIC {
            // The `main` thread unwinds without answering.
            let _ = commands.send(Command {
                request: Request::TestPanic,
                reply: None,
            });
            return Ok(json!({}));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = commands.send(Command {
            request: Request::TestCrash,
            reply: Some(tx),
        });
        rx.recv().unwrap_or_else(|_| Ok(json!({})))
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn parse_account(json: Value) -> Result<Account, ErrorObject> {
    let account =
        Account::from_json(json).map_err(|e| invalid(format!("Invalid Account JSON: {e}")))?;
    account.validate().map_err(invalid)?;
    Ok(account)
}

fn removed_result(still_running: Vec<String>) -> Reply {
    serde_json::to_value(AccountRemoveResult { still_running })
        .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
}

// -- Parent channel -------------------------------------------------------------------------

/// Reads the parent's requests from stdin and answers each on stdout, in order. The process
/// ends from here: `engine.shutdown` exits 0, closed stdin 141 after the grace period.
fn parent_loop(engine: Arc<Engine>, mut session: Session, orphan: bool) {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match ms_ipc::read_frame(&mut reader, &mut line, MAX_LINE_BYTES) {
            Ok(true) => {}
            Ok(false) | Err(ms_ipc::FrameError::Io(_)) => {
                if orphan {
                    return;
                }
                crate::stdin_loop::wait_for_parent_then_exit();
            }
            Err(ms_ipc::FrameError::TooLong) => {
                let e = ErrorObject::new(
                    code::PARSE_ERROR,
                    format!("parse error: the line exceeds {MAX_LINE_BYTES} bytes"),
                );
                respond(&Response::err(None, e));
                // The rest of the line is still to come; skip to its end.
                let _ = reader.skip_until(b'\n');
                continue;
            }
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let req = match ms_ipc::parse_request(&line) {
            Ok(req) => req,
            Err((id, e)) => {
                respond(&Response::err(id, e));
                continue;
            }
        };
        if req.method == method::ENGINE_SHUTDOWN {
            engine.shutdown();
            if let Some(id) = req.id {
                respond(&Response::ok(Some(id), json!({})));
            }
            crate::platform::exit_now(EXIT_OK);
        }
        if is_slow_parent_method(&req.method) {
            spawn_parent_call(engine.clone(), req);
            continue;
        }
        let result = handle_parent(&engine, &mut session, &req);
        if let Some(id) = req.id {
            respond(&match result {
                Ok(v) => Response::ok(Some(id), v),
                Err(e) => Response::err(Some(id), e),
            });
        }
    }
}

/// A slow parent method's work, run on its own thread.
type ParentWork = Box<dyn FnOnce(&Engine) -> Reply + Send>;

fn is_slow_parent_method(name: &str) -> bool {
    matches!(
        name,
        method::ACCOUNT_TEST
            | method::ACCOUNT_RESET
            | method::ENGINE_RESET_DATABASE
            | method::ENGINE_SET_LOG_LEVEL
    )
}

/// Runs a slow parent method on its lane's workers ([`crate::workers`]), which answer when
/// done. The account methods' jobs carry the account's id, so they log to its log, and their
/// lanes are contained: a panic is answered as an error instead of ending the process. The
/// engine-wide ones run one at a time and abort on a panic, as the stdin thread would.
fn spawn_parent_call(engine: Arc<Engine>, req: RpcRequest) {
    let request_id = req.id.clone();
    let answer = move |result: Reply| {
        if let Some(id) = request_id {
            respond(&match result {
                Ok(v) => Response::ok(Some(id), v),
                Err(e) => Response::err(Some(id), e),
            });
        }
    };
    let (lane, account_id, work): (&'static Lane, Option<String>, ParentWork) =
        match req.method.as_str() {
            method::ACCOUNT_TEST => {
                let account = match req
                    .params_as::<AccountTestParams>()
                    .and_then(|p| parse_account(p.account))
                {
                    Ok(a) => a,
                    Err(e) => return answer(Err(e)),
                };
                let id = account.id().to_owned();
                (
                    &workers::ACCOUNT_TEST,
                    Some(id),
                    Box::new(move |e| e.test_account(&account)),
                )
            }
            method::ACCOUNT_RESET => {
                let p = match req.params_as::<AccountResetParams>() {
                    Ok(p) => p,
                    Err(e) => return answer(Err(e)),
                };
                let id = p.account_id.clone();
                (
                    &workers::ACCOUNT_RESET,
                    Some(id),
                    Box::new(move |e| e.reset_account(&p.account_id, p.keep_metadata)),
                )
            }
            method::ENGINE_RESET_DATABASE => {
                let p = match req.params_as::<ResetDatabaseParams>() {
                    Ok(p) => p,
                    Err(e) => return answer(Err(e)),
                };
                (
                    &workers::ENGINE_MAINTENANCE,
                    None,
                    Box::new(move |e| e.reset_database(p.keep_metadata)),
                )
            }
            _ => {
                let params = req.clone();
                (
                    &workers::ENGINE_MAINTENANCE,
                    None,
                    Box::new(move |e| e.set_log_level(&params)),
                )
            }
        };
    lane.submit(account_id.map(Into::into), move || work(&engine), answer);
}

fn respond(response: &Response) {
    write_stdout(&String::from_utf8_lossy(&ms_ipc::encode(response)));
}

fn handle_parent(engine: &Engine, session: &mut Session, req: &RpcRequest) -> Reply {
    match req.method.as_str() {
        method::ACCOUNT_ADD => engine.add(req),
        method::ACCOUNT_REMOVE => engine.remove(req),
        method::TEST_CRASH | method::TEST_PANIC if cfg!(debug_assertions) => {
            engine.test_failure(req)
        }
        method::SUBSCRIBE => Err(ErrorObject::new(
            code::WRONG_CHANNEL,
            "subscribe on the socket: stdout carries only responses and account secrets",
        )),
        other => session.call(req).unwrap_or_else(|| {
            Err(ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("unknown method {other}"),
            ))
        }),
    }
}

/// The socket path `serve` uses without `--socket` (plan 1.5).
pub fn default_socket(config_dir: &Path) -> PathBuf {
    config_dir.join("run").join("engine.sock")
}
