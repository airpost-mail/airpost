// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `--mode serve`: every account in one process.
//!
//! The engine starts with no account, binds `--socket` (default
//! `$CONFIG_DIR_PATH/run/engine.sock`) and keeps `run/engine.json` listing the accounts it
//! holds. The parent's pipes are the control channel: stdin takes JSON-RPC requests, the
//! socket's envelope, for `engine.resetDatabase`, `engine.setLogLevel`, `engine.shutdown`
//! and `credentials.import` plus every socket method; stdout carries their responses and the
//! accounts' secrets deltas (`delta` notifications) and nothing else. stdin is read only once
//! the socket is bound, so the parent's first response marks the engine ready. The slow
//! methods run on worker lanes and answer when done, so responses can come out of order.
//! Closed stdin still exits 141 after 30 s.
//!
//! The account methods (`account.add`, `.test`, `.oauth.*`, `.setSecrets`, `.update`, `.remove`,
//! ..., `secrets.*`, `server.*`) are taken on stdin and on the socket alike: secrets go client to
//! engine and are never read back (T13). A relayed connection makes them only with the admin scope
//! (tern-ipc's `policy`).
//!
//! `--detached` has no parent channel: local, non-relayed socket connections also take the
//! engine methods but `credentials.import`, the slow ones on the same worker lanes, and
//! nothing secret goes to stdout, which is then a terminal or a service log.
//!
//! Each account runs in its own [`AccountRuntime`] with its own `DeltaStream` into the
//! shared hub, `main` thread and log file. A failure on one of its threads stops that
//! account alone and sends `account.state` `stopped` with the error once its threads have
//! exited; the engine never restarts it, the client may add it again. Engine-wide threads
//! (stdin, socket, signals) still abort the process on a panic.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tern_core::constants::{
    thread, ACCOUNT_RESET_STRAGGLER_WAIT_MS, ACCOUNT_START_WAIT_MS, ACCOUNT_STOP_TIMEOUT_MS,
    DATABASE_FILE, ENGINE_LOG_FILE, EXIT_FAILURE, EXIT_OK, EXIT_SOCKET_IN_USE,
};
use tern_core::Account;
use tern_ipc::{
    account_state, code, method, AccountAddParams, AccountAddResult, AccountChangedParams,
    AccountDiscoverParams, AccountIdParams, AccountRemoveParams, AccountRemoveResult,
    AccountReorderParams, AccountResetParams, AccountSetSecretsParams, AccountStateParams,
    AccountStatus, AccountTestParams, AccountUpdateParams, AccountUpdateResult,
    CredentialsImportParams, CredentialsStatus, ErrorObject, OAuthBeginParams, OAuthCompleteParams,
    OAuthCompleteResult, Request as RpcRequest, ResetDatabaseParams, ResetDatabaseResult, Response,
    SecretsDeleteParams, SecretsHasParams, SecretsSetParams, ServerSetParams, SetLogLevelParams,
    SetLogLevelResult, ACCOUNT_DISPLAY_FIELDS, MAX_LINE_BYTES,
};
use tern_net::TernServer;
use tern_store::{DeltaSink, DeltaStream};
use tern_sync::{AccountFailure, AccountRuntime, WorkerContext};
use tracing::{info, warn};

use crate::accounts_api;
use crate::cli::Options;
use crate::dispatch::{self, Command, Reply, Request};
use crate::output::{one_shot_error, write_stdout};
use crate::registry::{self, Registry as Registered};
use crate::restart::{self, Decision, History};
use crate::socket::{self, Accounts, Hub, Server, Session, SocketJob, SocketParentCall};
use crate::workers;

type Runtime = AccountRuntime<Command>;

pub fn run(opts: &Options) -> i32 {
    let socket_path = opts
        .socket
        .clone()
        .unwrap_or_else(|| default_socket(&opts.config_dir));
    if let Err(e) = crate::platform::uds::restrict_dir(&opts.config_dir) {
        one_shot_error(&format!("Setup Failed: {e}"));
        return EXIT_FAILURE;
    }
    if let Err(e) = tern_core::log::init_file(&opts.config_dir.join(ENGINE_LOG_FILE)) {
        one_shot_error(&format!("Setup Failed: {e}"));
        return EXIT_FAILURE;
    }
    crate::modes::install_panic_hook(true);
    info!("------------- Starting the engine (serve) ---------------");
    // Hashed while the database migrates, for `hello.build`.
    crate::modes::spawn(thread::BUILD_ID, || {
        socket::build_id();
    });

    let fail = |message: String| {
        tracing::error!("-X {message}");
        one_shot_error(&message);
        EXIT_FAILURE
    };
    // Bound before migrating, so a second engine on this config directory stops here and
    // only one migrates; nothing is accepted until the database is current.
    let listener = match crate::platform::uds::bind(&socket_path) {
        Ok(l) => l,
        Err(e) if e.in_use => {
            fail(e.message);
            return EXIT_SOCKET_IN_USE;
        }
        Err(e) => return fail(e.message),
    };
    crate::platform::remove_on_exit(socket_path.clone());
    let mut progress = Vec::new();
    let migrated =
        tern_store::migrate::migrate(&opts.config_dir, &tern_core::SystemClock, &mut progress);
    let progress = String::from_utf8_lossy(&progress);
    if !progress.trim().is_empty() {
        info!(
            "Migrate: {}",
            progress.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
    if let Err(e) = migrated {
        return fail(format!("migrate failed: {e}"));
    }
    match socket::write_engine_discovery(&opts.config_dir, &socket_path, &[], opts.detached) {
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
        detached: opts.detached,
        verbose_until: AtomicI64::new(if opts.verbose { i64::MAX } else { 0 }),
        hub: hub.clone(),
        registry: Mutex::new(Registry::default()),
        registered: Registered::open(&opts.config_dir),
        identity: Mutex::new(Value::Null),
        unstarted: Mutex::new(HashMap::new()),
        restarts: Mutex::new(HashMap::new()),
        restart_delay: restart::base_delay(),
        changed: Condvar::new(),
        resetting: Mutex::new(HashSet::new()),
        resetting_database: AtomicBool::new(false),
        database: database.clone(),
        settings: settings.clone(),
        oauth: accounts_api::OAuthSessions::default(),
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
    crate::ai_api::init_usage(&opts.config_dir);
    let for_keys = Arc::downgrade(&engine);
    crate::ai_api::on_secret(Box::new(move |name| {
        // An unreadable store holds no key here; `hello.credentials` reports why.
        for_keys.upgrade()?.registered.secret(name).ok().flatten()
    }));
    let for_forget = Arc::downgrade(&engine);
    crate::ai_api::on_forget_secret(Box::new(move |name| match for_forget.upgrade() {
        Some(engine) => engine.registered.forget_secret(name),
        None => Err("the engine is shutting down".to_owned()),
    }));
    let for_capabilities = Arc::downgrade(&server);
    crate::ai_triage::on_capabilities_changed(Box::new(move || {
        if let Some(server) = for_capabilities.upgrade() {
            socket::publish_capabilities(&server);
        }
    }));
    let for_rules = Arc::downgrade(&server);
    crate::rules::on_disable(Box::new(move |rule_id, error| match for_rules.upgrade() {
        Some(server) => socket::disable_rule(&server, rule_id, error),
        None => false,
    }));
    socket::serve(listener, server.clone());
    info!(
        "{}",
        tern_core::logmsg::socket_listening(&socket_path.to_string_lossy())
    );

    let starting = engine.clone();
    crate::modes::spawn(thread::ACCOUNT_RESTART, move || starting.start_registered());

    if opts.detached {
        info!("Detached: no parent channel; engine.shutdown or a signal stops the engine");
    } else {
        let orphan = opts.orphan;
        crate::modes::spawn(thread::STDIN, move || {
            parent_loop(engine, Session::new(server), orphan)
        });
    }
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
    /// The account's AI triage queue, closed when it stops.
    triage: Arc<crate::ai_triage::Feed>,
    /// The account's mail rules queue, closed when it stops.
    rules: Arc<crate::rules::Feed>,
    runtime: Option<Arc<Runtime>>,
    /// The `accountStop` thread is joining the account's threads after a failure.
    stopping: bool,
    /// `account.remove` or `engine.shutdown` is stopping it.
    removing: bool,
    /// Stopped: when the restart policy starts it again (Unix ms), or `None` when it won't.
    retry_at: Option<i64>,
    /// Stopped because the server refused its credentials; only `account.add` restarts it.
    auth_failed: bool,
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
    /// `--detached`: no parent channel, so the socket takes the parent methods.
    detached: bool,
    /// Unix milliseconds: accounts started before then log protocol traffic
    /// (`engine.setLogLevel`; `--verbose` sets it to forever).
    verbose_until: AtomicI64,
    hub: Arc<Hub>,
    registry: Mutex<Registry>,
    /// `accounts.json` and the secret store.
    registered: Registered,
    /// The identity line of the latest `account.add`, for the accounts the engine starts
    /// itself.
    identity: Mutex<Value>,
    /// Registered accounts that could not be started, with the error `account.list` shows.
    unstarted: Mutex<HashMap<String, Value>>,
    /// Each account's recent stops, for the restart policy; `account.add` clears them.
    restarts: Mutex<HashMap<String, History>>,
    restart_delay: Duration,
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
    /// OAuth sign-ins between `account.oauth.begin` and `complete`.
    oauth: accounts_api::OAuthSessions,
    /// The engine itself, for the callbacks an account's threads make.
    me: Weak<Engine>,
}

impl Accounts for Engine {
    /// The running and stopped accounts, then the registered ones not started, each with
    /// its non-secret JSON.
    fn statuses(&self) -> Vec<AccountStatus> {
        let mut out: Vec<AccountStatus> = self
            .lock()
            .slots
            .iter()
            .map(|s| AccountStatus {
                account_id: s.account.id().to_owned(),
                email: s.account.email_address().to_owned(),
                state: s.state.to_owned(),
                connection_error: s.deltas.connection_error(),
                error: s.error.clone(),
                account: Some(registry::strip_secrets(s.account.as_json())),
                retry_at: s.retry_at,
                auth_failed: s.auth_failed,
            })
            .collect();
        let unstarted = self.unstarted_lock().clone();
        for account in self.registered.accounts() {
            let id = registry::account_id(&account)
                .unwrap_or_default()
                .to_owned();
            if out.iter().any(|s| s.account_id == id) {
                continue;
            }
            out.push(AccountStatus {
                email: account
                    .get("emailAddress")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                state: account_state::STOPPED.to_owned(),
                error: unstarted.get(&id).cloned(),
                account_id: id,
                account: Some(account),
                ..Default::default()
            });
        }
        // In the registry's order (`account.reorder`); unregistered ones last.
        let order = self.registered.ids();
        out.sort_by_key(|s| {
            order
                .iter()
                .position(|id| *id == s.account_id)
                .unwrap_or(usize::MAX)
        });
        out
    }

    fn credentials(&self) -> Option<CredentialsStatus> {
        Some(self.registered.status())
    }

    fn detached(&self) -> bool {
        self.detached
    }

    fn socket_parent_call(&self, req: &RpcRequest) -> Option<SocketParentCall> {
        if !self.detached {
            return None;
        }
        if is_slow_parent_method(&req.method) {
            let Some(engine) = self.me.upgrade() else {
                return Some(SocketParentCall::Now(Err(ErrorObject::new(
                    code::INTERNAL_ERROR,
                    "the engine is stopping",
                ))));
            };
            return Some(match slow_parent_job(engine, req) {
                Ok(job) => SocketParentCall::Later(job),
                Err(e) => SocketParentCall::Now(Err(e)),
            });
        }
        Some(SocketParentCall::Now(match req.method.as_str() {
            method::CREDENTIALS_IMPORT => Err(ErrorObject::new(
                code::WRONG_CHANNEL,
                "credentials.import is only accepted on the engine's stdin",
            )),
            method::ENGINE_SHUTDOWN => {
                let me = self.weak();
                crate::modes::spawn(thread::ENGINE_MAINTENANCE, move || {
                    if let Some(engine) = me.upgrade() {
                        engine.shutdown();
                    }
                    // Lets the connection's writer send the answer.
                    std::thread::sleep(Duration::from_millis(SHUTDOWN_FLUSH_MS));
                    crate::platform::exit_now(EXIT_OK);
                });
                Ok(json!({}))
            }
            _ => self.parent_call(req),
        }))
    }

    fn check(&self, account_id: &str) -> Result<(), ErrorObject> {
        if self.lock().get(account_id).is_some() || self.registered.contains(account_id) {
            return Ok(());
        }
        Err(socket::unknown_account(account_id))
    }

    fn commands(&self, account_id: &str) -> Result<Sender<Command>, ErrorObject> {
        let reg = self.lock();
        let Some(slot) = reg.get(account_id) else {
            if self.registered.contains(account_id) {
                return Err(ErrorObject::new(
                    code::ACCOUNT_STOPPED,
                    format!("account {account_id} is not running"),
                )
                .with_data(json!({ "state": account_state::STOPPED })));
            }
            return Err(socket::unknown_account(account_id));
        };
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

    fn secret(&self, name: &str) -> Option<String> {
        self.registered
            .secret(name)
            .map_err(|e| warn!("-X Could not read secret {name}: {e}"))
            .ok()
            .flatten()
    }

    fn account_call(&self, req: &RpcRequest) -> Option<SocketParentCall> {
        if !socket::is_account_method(&req.method) {
            return None;
        }
        Some(match self.me.upgrade() {
            Some(engine) => account_call(engine, req),
            None => SocketParentCall::Now(Err(ErrorObject::new(
                code::INTERNAL_ERROR,
                "the engine is stopping",
            ))),
        })
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
        let previous = tern_core::log::thread_account();
        tern_core::log::set_thread_account(Some(id.into()));
        AccountScope(previous)
    }
}

impl Drop for AccountScope {
    fn drop(&mut self) {
        tern_core::log::set_thread_account(self.0.take());
    }
}

/// An account's secrets delta, as a `delta` notification on the parent's stdout. A detached
/// engine has no parent: its stdout is a terminal or a service log, so the delta is dropped.
struct ParentSink {
    account_id: String,
    detached: bool,
}

impl DeltaSink for ParentSink {
    fn write_item(&mut self, _model_class: &str, line: &str) {
        if self.detached {
            warn!(
                "-X Account {}: dropped a secrets delta, a detached engine has no parent",
                self.account_id
            );
            return;
        }
        let note = tern_ipc::delta_notification_line(&self.account_id, None, line);
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
        self.notify(AccountStateParams {
            account_id: id.to_owned(),
            state: state.to_owned(),
            error,
            ..Default::default()
        });
    }

    fn notify(&self, params: AccountStateParams) {
        self.hub.account_state(&params);
        self.changed.notify_all();
    }

    fn write_discovery(&self, reg: &Registry) {
        let written = socket::write_engine_discovery(
            &self.config_dir,
            &self.socket,
            &reg.ids(),
            self.detached,
        );
        if let Err(e) = written {
            warn!("-X Could not update the discovery file: {e}");
        }
    }

    fn unstarted_lock(&self) -> MutexGuard<'_, HashMap<String, Value>> {
        self.unstarted.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn restarts_lock(&self) -> MutexGuard<'_, HashMap<String, History>> {
        self.restarts.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn identity_lock(&self) -> MutexGuard<'_, Value> {
        self.identity.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `account.changed` for a registered account whose JSON changed.
    fn announce(&self, account: &Value) {
        if let Some(id) = registry::account_id(account) {
            self.hub.account_changed(&AccountChangedParams {
                account_id: id.to_owned(),
                account: account.clone(),
            });
        }
    }

    /// Starts every registered account, at engine start.
    fn start_registered(&self) {
        let accounts = self.registered.accounts();
        if accounts.is_empty() {
            return;
        }
        info!("Starting {} registered accounts", accounts.len());
        let identity = self.default_identity();
        for account in accounts {
            let id = registry::account_id(&account)
                .unwrap_or_default()
                .to_owned();
            if self.lock().get(&id).is_some() {
                continue;
            }
            let full = self
                .registered
                .with_secrets(account)
                .map_err(|e| format!("the account's secrets can't be read: {e}"))
                .and_then(|json| parse_account(json).map_err(|e| e.message));
            let outcome = full.and_then(|a| {
                self.start(a, identity.clone())
                    .map(|_| ())
                    .map_err(|e| e.message)
            });
            if let Err(message) = outcome {
                warn!("-X Could not start registered account {id}: {message}");
                let error = json!({
                    "what": tern_core::error::SYNC_ERROR_WHAT,
                    "key": "ErrorAccountNotStarted",
                    "debuginfo": message,
                    "retryable": false,
                    "offline": false,
                });
                if self.lock().get(&id).is_none() {
                    self.unstarted_lock().insert(id.clone(), error.clone());
                    self.notify_state(&id, account_state::STOPPED, Some(error));
                }
            }
        }
    }

    /// `credentials.import` (stdin only): registers the accounts and stores their secrets
    /// without starting them; the client adds them next.
    fn import(&self, req: &RpcRequest) -> Reply {
        let p: CredentialsImportParams = req.params_as()?;
        let ai_keys = p.secrets.keys().any(|k| k.starts_with("integration-ai-"));
        let result = self.registered.import(p);
        if ai_keys {
            crate::ai_triage::capabilities_changed();
        }
        for id in &result.imported {
            if let Some(account) = self.registered.get(id) {
                self.announce(&account);
            }
        }
        serde_json::to_value(result)
            .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
    }

    /// `account.add`: starts the account and answers once its `main` thread is up (or the
    /// account failed to start, or [`ACCOUNT_START_WAIT_MS`] passed).
    ///
    /// The account is registered: the secrets it carries are stored, the ones it lacks come
    /// from the store, and the engine starts it itself from then on. Stored secrets are used
    /// only with the servers and usernames they were stored for: an account added with other
    /// ones must carry its secrets, which then replace the stored set. `stored` is false when
    /// the store refused, and the account then runs only until the engine stops. A running
    /// account added with another identity, other settings or other secrets restarts with
    /// them; added as it runs, it answers ACCOUNT_EXISTS.
    ///
    /// `secrets` go into the account's settings first. An account without an id gets the
    /// desktop's ([`accounts_api::prepare_new_account`]). With `test` it is tested first and
    /// a failure answers [`code::ACCOUNT_TEST_FAILED`] without storing anything. A null
    /// identity means the latest one, or else the Tern server of `server.set`.
    fn add(&self, req: &RpcRequest) -> Reply {
        let p: AccountAddParams = req.params_as()?;
        let mut json = p.account;
        if let Some(secrets) = &p.secrets {
            if !json.get("settings").is_some_and(Value::is_object) {
                return Err(invalid("Invalid Account JSON: settings must be an object"));
            }
            secrets.apply_to(&mut json);
        }
        self.add_account(json, p.identity, p.test, false)
    }

    /// `account.add`'s work. `renew` restarts a running account even when nothing changed
    /// (a reconnect that got the same credentials back).
    fn add_account(&self, mut json: Value, identity: Value, test: bool, renew: bool) -> Reply {
        if registry::account_id(&json).is_none() {
            accounts_api::prepare_new_account(&mut json);
        }
        let sent_secrets = registry::has_secrets(&json);
        match self.registered.with_secrets_for_same_servers(json.clone()) {
            Ok((_, true)) if !sent_secrets => {
                return Err(invalid(
                    "new server settings need new credentials: the account is registered \
                     with other servers or usernames",
                ))
            }
            Ok((full, _)) => json = full,
            Err(e) => warn!("-X Could not read the stored secrets: {e}"),
        }
        let mut account = parse_account(json)?;
        let id = account.id().to_owned();
        if self.resetting_database.load(Ordering::SeqCst) || self.resetting_lock().contains(&id) {
            return Err(ErrorObject::new(
                code::ACCOUNT_EXISTS,
                format!("account {id} is being reset; add it again later"),
            ));
        }
        if test {
            let result = self.test_account(&account)?;
            if !accounts_api::test_passed(&result) {
                return Err(accounts_api::test_failed(result));
            }
            // The test may have rotated the refresh token.
            if let Some(tested) = result
                .get("account")
                .filter(|a| a.is_object())
                .and_then(|a| Account::from_json(a.clone()).ok())
            {
                account = tested;
            }
        }
        let stored = match self.registered.register(account.as_json()) {
            Ok((public, changed)) => {
                if changed {
                    self.announce(&public);
                }
                true
            }
            Err(e) => {
                warn!("-X Account {id} is not registered: {e}");
                false
            }
        };
        self.restarts_lock().remove(&id);
        self.unstarted_lock().remove(&id);
        let identity = if identity.is_null() {
            self.default_identity()
        } else {
            *self.identity_lock() = identity.clone();
            identity
        };
        let changed = self.lock().get(&id).and_then(|s| {
            if s.state == account_state::STOPPED || s.removing {
                None
            } else if !identity.is_null() && s.identity != identity {
                Some("a new identity")
            } else if registry::stored_form(s.account.as_json())
                != registry::stored_form(account.as_json())
            {
                Some("new settings")
            } else if sent_secrets
                && registry::secrets_of(s.account.as_json())
                    != registry::secrets_of(account.as_json())
            {
                Some("new credentials")
            } else if renew {
                Some("renewed credentials")
            } else {
                None
            }
        });
        if let Some(what) = changed {
            info!("Restarting account {id} with {what}");
            self.remove_account(&id, false)?;
        }
        let mut result = self.start(account, identity)?;
        result["stored"] = json!(stored);
        Ok(result)
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

        if let Err(e) = tern_core::log::open_account_log(&self.config_dir, &id) {
            warn!("-X Could not open the log of account {id}; its lines go to engine.log: {e}");
        }
        let scope = AccountScope::enter(&id);
        info!(
            "{}",
            tern_core::logmsg::starting_sync(account.email_address())
        );
        let (ctx, deltas, triage, rules) = self.context(account.clone(), parsed, generation);
        let slot = Slot {
            account,
            identity,
            generation,
            state: account_state::STARTING,
            error: None,
            deltas,
            triage: triage.clone(),
            rules: rules.clone(),
            runtime: None,
            stopping: false,
            removing: false,
            retry_at: None,
            auth_failed: false,
        };
        match reg.get_mut(&id) {
            Some(old) => *old = slot,
            None => reg.slots.push(slot),
        }
        self.notify_state(&id, account_state::STARTING, None);
        self.write_discovery(&reg);

        let weak = self.weak();
        let started_id = id.clone();
        let dispatcher: tern_sync::runtime::Dispatcher<Command> = Box::new(move |ctx, rx, _| {
            dispatch::run(ctx, rx, move || {
                if let Some(engine) = weak.upgrade() {
                    engine.started(&started_id, generation);
                }
            })
        });
        // The registry stays locked until the runtime is stored, so a thread failing at once
        // finds it to stop.
        let runtime = Arc::new(AccountRuntime::start(ctx, Some(dispatcher)));
        crate::ai_triage::start(runtime.context(), triage);
        crate::rules::start(runtime.context(), rules);
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
            stored: false,
        })
        .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
    }

    /// The account's context: its own delta stream into the hub (secrets to stdout) through
    /// its mail rules and AI triage feeds, token manager, metadata backend and failure
    /// handler.
    fn context(
        &self,
        account: Arc<Account>,
        identity: crate::modes::IdentityLine,
        generation: u64,
    ) -> (
        WorkerContext,
        Arc<DeltaStream>,
        Arc<crate::ai_triage::Feed>,
        Arc<crate::rules::Feed>,
    ) {
        let id = account.id().to_owned();
        let triage = crate::ai_triage::Feed::register(&id);
        let rules = crate::rules::Feed::register(&id);
        let deltas = DeltaStream::with_sink(rules.tap(triage.tap(self.hub.sink(
            &id,
            Box::new(ParentSink {
                account_id: id.clone(),
                detached: self.detached,
            }),
        ))));
        let metadata =
            crate::modes::metadata_backend(&self.config_dir, &account, identity, &deltas);
        let for_tokens = deltas.clone();
        let weak = self.weak();
        let detached = self.detached;
        // A registered account's rotated token goes to the store; any other account's to
        // the parent, as before the engine kept secrets. A detached engine has no parent, so
        // the token stays in the token manager, which the account's restarts use.
        let tokens = tern_net::TokenManager::new(Some(Box::new(move |a: &Account| {
            let stored = weak.upgrade().map(|engine| {
                engine
                    .registered
                    .store_refresh_token(a.id(), a.refresh_token())
            });
            match stored {
                Some(Ok(true)) => {
                    info!("Stored the rotated refresh token of {}", a.id());
                    return;
                }
                Some(Err(e)) => warn!(
                    "-X Could not store the rotated refresh token of {}: {e}",
                    a.id()
                ),
                _ => {}
            }
            if detached {
                warn!(
                    "-X The rotated refresh token of {} is kept until the engine stops",
                    a.id()
                );
            } else {
                for_tokens.send_updated_secrets(a);
            }
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
            Arc::new(tern_tasks::Tasks),
            Arc::new(on_failure),
        );
        ctx.verbose = now_ms() < self.verbose_until.load(Ordering::SeqCst);
        ctx.settings = Some(self.settings.clone());
        (ctx, deltas, triage, rules)
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
        slot.triage.close();
        slot.rules.close();
        drop(reg);
        self.changed.notify_all();

        let weak = self.weak();
        let stop_id = id.to_owned();
        let spawned = tern_core::log::spawn_account_thread(
            Some(id.into()),
            thread::ACCOUNT_STOP,
            move || {
                if let Some(rt) = &runtime {
                    rt.stop(Duration::from_millis(ACCOUNT_STOP_TIMEOUT_MS));
                }
                if let Some(engine) = weak.upgrade() {
                    engine.stopped(&stop_id, generation);
                }
            },
        );
        if let Err(e) = spawned {
            tern_core::critical!("Could not start the {} thread: {e}", thread::ACCOUNT_STOP);
            tern_core::log::flush();
            std::process::abort();
        }
    }

    /// The `accountStop` thread is done: the account's threads exited or were left after
    /// the timeout. The restart policy ([`restart`]) decides whether it starts again.
    fn stopped(&self, id: &str, generation: u64) {
        let mut reg = self.lock();
        let Some(slot) = reg.current(id, generation) else {
            return;
        };
        slot.stopping = false;
        // The restart uses the secrets the account last had, a rotated token included.
        if let Some(rt) = slot.runtime.take() {
            let current = rt
                .context()
                .tokens
                .account_with_current_secrets(&slot.account);
            slot.account = Arc::new(current);
        }
        // A thread left running past the timeout must not publish for the account any more.
        slot.deltas.close();
        let error = slot.error.clone();
        if slot.removing {
            self.changed.notify_all();
            drop(reg);
            tern_core::log::flush();
            return;
        }
        let decision = self
            .restarts_lock()
            .entry(id.to_owned())
            .or_default()
            .record(error.as_ref(), Instant::now(), self.restart_delay);
        let delay = match decision {
            Decision::AuthFailed => {
                slot.auth_failed = true;
                info!("Account {id} stopped: the server refused its credentials");
                None
            }
            Decision::GiveUp => {
                info!(
                    "Account {id} stopped {} times in {} minutes; not restarting it",
                    restart::CRASH_LIMIT,
                    restart::CRASH_WINDOW.as_secs() / 60
                );
                None
            }
            Decision::RetryIn(delay) => {
                info!(
                    "Account {id} stopped; restarting it in {}s",
                    delay.as_secs_f32()
                );
                Some(delay)
            }
        };
        slot.retry_at = delay.map(|d| now_ms() + i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        self.notify(AccountStateParams {
            account_id: id.to_owned(),
            state: account_state::STOPPED.to_owned(),
            error,
            retry_at: slot.retry_at,
            auth_failed: slot.auth_failed,
        });
        drop(reg);
        tern_core::log::flush();
        if let Some(delay) = delay {
            let weak = self.weak();
            let id = id.to_owned();
            crate::modes::spawn(thread::ACCOUNT_RESTART, move || {
                std::thread::sleep(delay);
                if let Some(engine) = weak.upgrade() {
                    engine.restart(&id, generation);
                }
            });
        }
    }

    /// The restart policy's retry: starts the account again unless something else did (or
    /// removed it) meanwhile.
    fn restart(&self, id: &str, generation: u64) {
        let reg = self.lock();
        let Some(slot) = reg.get(id).filter(|s| {
            s.generation == generation
                && s.state == account_state::STOPPED
                && !s.removing
                && !s.stopping
                && s.retry_at.is_some()
        }) else {
            return;
        };
        let (account, identity) = ((*slot.account).clone(), slot.identity.clone());
        drop(reg);
        let _scope = AccountScope::enter(id);
        info!("Restarting account {id}");
        if let Err(e) = self.start(account, identity) {
            warn!("-X Could not restart account {id}: {}", e.message);
        }
    }

    /// `account.remove`: stops the account, forgets it and sends `removed`. Its database rows
    /// stay; `--mode reset` deletes them. The result lists the account's threads still
    /// running after the stop timeout (`stillRunning`), which may still write to the
    /// database: a client about to reset the account should wait and retry.
    ///
    /// A registered account is unregistered and its secrets deleted, whether it ran or not.
    ///
    /// `revoke` then drops a Google account's OAuth grant ([`accounts_api::revoke`]) and
    /// `deleteData` its rows, as `account.reset` without metadata does; both run on a worker
    /// lane.
    fn remove(&self, req: &RpcRequest) -> Reply {
        let p: AccountRemoveParams = req.params_as()?;
        let id = p.account_id;
        let running = self.lock().get(&id).is_some();
        if !running && !self.registered.contains(&id) {
            return Err(socket::unknown_account(&id));
        }
        let grant = p.revoke.then(|| self.grant_of(&id)).flatten();
        self.restarts_lock().remove(&id);
        self.unstarted_lock().remove(&id);
        let still_running = if running {
            self.remove_account(&id, true)?
        } else {
            self.forget_registered(&id)
                .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e))?;
            self.notify_state(&id, account_state::REMOVED, None);
            Vec::new()
        };
        if let Some((provider, token)) = grant {
            accounts_api::revoke(&provider, &token);
        }
        if p.delete_data {
            if !still_running.is_empty() {
                std::thread::sleep(Duration::from_millis(ACCOUNT_RESET_STRAGGLER_WAIT_MS));
            }
            self.reset_account(&id, false)?;
        }
        removed_result(still_running)
    }

    /// The provider and refresh token of an account, its rotated token included.
    fn grant_of(&self, id: &str) -> Option<(String, String)> {
        let running = self.lock().get(id).map(|s| match &s.runtime {
            Some(rt) => rt
                .context()
                .tokens
                .account_with_current_secrets(&s.account)
                .to_json(),
            None => s.account.as_json().clone(),
        });
        let json = running.or_else(|| {
            self.registered
                .get(id)
                .and_then(|a| self.registered.with_secrets(a).ok())
        })?;
        let token = json
            .pointer("/settings/refresh_token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())?;
        let provider = json.get("provider").and_then(Value::as_str)?;
        Some((provider.to_owned(), token.to_owned()))
    }

    fn forget_registered(&self, id: &str) -> Result<(), String> {
        if self.registered.forget(id)? {
            info!("Account {id} unregistered; its secrets are deleted");
        }
        Ok(())
    }

    /// Stops the account and drops its slot, and with `forget` unregisters it; returns its
    /// threads still running. `account.reset` and `engine.resetDatabase` keep it registered:
    /// the reset starts it again, the database reset leaves it until the client adds it.
    fn remove_account(&self, id: &str, forget: bool) -> Result<Vec<String>, ErrorObject> {
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
            Some(s) => {
                s.triage.close();
                s.rules.close();
                (s.runtime.take(), s.deltas.clone())
            }
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

        if forget {
            if let Err(e) = self.forget_registered(&id) {
                warn!("-X Account {id} stopped but could not be unregistered: {e}");
            }
        }
        let mut reg = self.lock();
        reg.slots.retain(|s| s.account.id() != id);
        self.write_discovery(&reg);
        self.notify_state(&id, account_state::REMOVED, None);
        drop(reg);
        tern_core::log::close_account_log(&id);
        Ok(still_running)
    }

    /// `--mode test`'s checks and result, for an account the engine need not hold. The
    /// result's `account` carries the secrets (a rotated token): `account.test` strips them
    /// ([`tested_publicly`]). Runs on a worker lane.
    fn test_account(&self, account: &Account) -> Reply {
        info!("Testing account {}", account.id());
        // Debug builds: the test fails as a bug in it would (serve tests).
        if cfg!(debug_assertions)
            && account.settings().get(TEST_PANIC_SETTING) == Some(&json!(true))
        {
            panic!("{TEST_PANIC_SETTING} in account.test");
        }
        Ok(crate::modes::test_auth::result(account))
    }

    /// `account.reset`: stops the account if the engine holds it, deletes its rows, keeping
    /// its plugin metadata for the resync when `keep_metadata`, and starts it again from the
    /// registry with the identity it had, so a socket client's reset leaves it syncing. An
    /// account the engine didn't hold (or `account.remove`'s, already unregistered) stays
    /// stopped. A client that adds it again gets ACCOUNT_EXISTS. Runs on its own thread.
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
        let outcome: Result<Option<Value>, ErrorObject> = (|| {
            let held = self.lock().get(id).map(|s| s.identity.clone());
            if held.is_some() {
                let still_running = self.remove_account(id, false)?;
                if !still_running.is_empty() {
                    info!(
                        "Waiting for {} of account {id} before resetting it",
                        still_running.join(", ")
                    );
                    std::thread::sleep(Duration::from_millis(ACCOUNT_RESET_STRAGGLER_WAIT_MS));
                }
            }
            let sink = tern_store::DeltaStream::new(Box::new(std::io::sink()));
            tern_store::Store::open(&self.config_dir, sink)
                .and_then(|store| store.reset_for_account(id, keep_metadata))
                .map_err(|e| {
                    ErrorObject::new(code::INTERNAL_ERROR, format!("reset failed: {e}"))
                })?;
            crate::ai_api::clear_cache();
            if !keep_metadata {
                // Their send-later metadata went with the drafts.
                tern_sync::pending_sends::PendingSends::new(&self.config_dir, id).remove_all();
            }
            info!("Account {id} reset");
            Ok(held)
        })();
        self.resetting_lock().remove(id);
        if let Ok(Some(identity)) = &outcome {
            self.restart_reset(id, identity.clone());
        }
        outcome.map(|_| json!({}))
    }

    /// Starts a reset account again from the registry. A failure leaves it stopped, as
    /// before the restart was the engine's.
    fn restart_reset(&self, id: &str, identity: Value) {
        let Some(public) = self.registered.get(id) else {
            return;
        };
        let started = self
            .registered
            .with_secrets(public)
            .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e))
            .and_then(parse_account)
            .and_then(|account| self.start(account, identity));
        if let Err(e) = started {
            warn!(
                "-X Account {id} was reset but could not be started again: {}",
                e.message
            );
        }
    }

    /// `engine.resetDatabase`: stops and forgets every account, deletes the database and
    /// creates an empty one, carrying the plugin metadata over by Message-ID
    /// ([`tern_store::metadata_stash`]) when `keep_metadata`. Socket sessions reopen it on their
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
                stragglers |= !self.remove_account(id, false)?.is_empty();
            }
            if stragglers {
                std::thread::sleep(Duration::from_millis(ACCOUNT_RESET_STRAGGLER_WAIT_MS));
            }
            // A database that can't be read (often why it is being rebuilt) loses its
            // metadata; the rebuild goes ahead.
            let kept = if keep_metadata {
                tern_store::metadata_stash::collect_database(&self.config_dir).unwrap_or_else(|e| {
                    warn!("-X Could not keep the plugin metadata of the old database: {e}");
                    Vec::new()
                })
            } else {
                tern_sync::pending_sends::PendingSends::remove_every_account(&self.config_dir);
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
            tern_store::migrate::migrate(
                &self.config_dir,
                &tern_core::SystemClock,
                &mut std::io::sink(),
            )
            .map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, format!("migrate failed: {e}")))?;
            if let Err(e) = tern_store::metadata_stash::save_database(&self.config_dir, &kept) {
                warn!("-X Could not keep the plugin metadata of the old database: {e}");
            }
            crate::ai_api::clear_cache();
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
        let restarted = self.restart_running(None)?;
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

    /// Restarts the running accounts with their current secrets, and with `identity` in
    /// place of their own when given; returns their ids.
    fn restart_running(&self, identity: Option<Value>) -> Result<Vec<String>, ErrorObject> {
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
                (
                    account,
                    identity.clone().unwrap_or_else(|| s.identity.clone()),
                )
            })
            .collect();
        let mut restarted = Vec::new();
        for (account, identity) in running {
            let id = account.id().to_owned();
            self.remove_account(&id, false)?;
            self.start(account, identity)?;
            restarted.push(id);
        }
        Ok(restarted)
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

impl Engine {
    /// A quick parent method on a detached engine's socket, answered on the connection's
    /// thread; the slow ones go to [`slow_parent_job`]'s lanes, as on stdin.
    fn parent_call(&self, req: &RpcRequest) -> Reply {
        match req.method.as_str() {
            method::TEST_CRASH | method::TEST_PANIC if cfg!(debug_assertions) => {
                self.test_failure(req)
            }
            other => Err(ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("unknown method {other}"),
            )),
        }
    }
}

// -- Account setup --------------------------------------------------------------------------

/// Where an account method runs: answered now, on the caller's thread, or on a worker lane
/// when it reaches a server or waits on the store (tests, sign-ins, revocation, deletions,
/// restarts of every account).
fn account_call(engine: Arc<Engine>, req: &RpcRequest) -> SocketParentCall {
    match account_job(engine, req) {
        Ok(call) => call,
        Err(e) => SocketParentCall::Now(Err(e)),
    }
}

fn account_job(engine: Arc<Engine>, req: &RpcRequest) -> Result<SocketParentCall, ErrorObject> {
    use SocketParentCall::{Later, Now};
    let flag = |name: &str| req.params.get(name) == Some(&json!(true));
    let named = |req: &RpcRequest| -> Option<Arc<str>> {
        req.params
            .get("accountId")
            .and_then(Value::as_str)
            .map(Into::into)
    };
    let later =
        |lane: &'static workers::Lane,
         account: Option<Arc<str>>,
         work: Box<dyn FnOnce() -> Reply + Send>| { Ok(Later((lane, account, work))) };
    let req = req.clone();
    match req.method.as_str() {
        method::ACCOUNT_DISCOVER => {
            let p: AccountDiscoverParams = req.params_as()?;
            later(
                &workers::ACCOUNT_SETUP,
                None,
                Box::new(move || accounts_api::discover(&p)),
            )
        }
        method::ACCOUNT_OAUTH_BEGIN => Ok(Now(engine.oauth_begin(&req))),
        method::ACCOUNT_OAUTH_COMPLETE => {
            let p: OAuthCompleteParams = req.params_as()?;
            later(
                &workers::ACCOUNT_SETUP,
                None,
                Box::new(move || engine.oauth_complete(&p)),
            )
        }
        method::ACCOUNT_ADD if flag("test") => {
            let account = req
                .params
                .pointer("/account/id")
                .and_then(Value::as_str)
                .map(Into::into);
            later(
                &workers::ACCOUNT_SETUP,
                account,
                Box::new(move || engine.add(&req)),
            )
        }
        method::ACCOUNT_ADD => Ok(Now(engine.add(&req))),
        method::ACCOUNT_SET_SECRETS if flag("test") => {
            let account = named(&req);
            later(
                &workers::ACCOUNT_SETUP,
                account,
                Box::new(move || engine.set_secrets(&req)),
            )
        }
        method::ACCOUNT_SET_SECRETS => Ok(Now(engine.set_secrets(&req))),
        method::ACCOUNT_UPDATE => Ok(Now(engine.update(&req))),
        method::ACCOUNT_REORDER => Ok(Now(engine.reorder(&req))),
        method::ACCOUNT_REMOVE if flag("revoke") || flag("deleteData") => {
            let account = named(&req);
            later(
                &workers::ACCOUNT_RESET,
                account,
                Box::new(move || engine.remove(&req)),
            )
        }
        method::ACCOUNT_REMOVE => Ok(Now(engine.remove(&req))),
        method::ACCOUNT_TEST => {
            let p: AccountTestParams = req.params_as()?;
            let mut json = p.account;
            if let Some(secrets) = &p.secrets {
                secrets.apply_to(&mut json);
            }
            if registry::account_id(&json).is_none() {
                accounts_api::prepare_new_account(&mut json);
            }
            let account = parse_account(json)?;
            let id = account.id().into();
            later(
                &workers::ACCOUNT_TEST,
                Some(id),
                Box::new(move || engine.test_account(&account).map(tested_publicly)),
            )
        }
        method::ACCOUNT_RESET => {
            let p = req.params_as::<AccountResetParams>()?;
            let id = p.account_id.as_str().into();
            later(
                &workers::ACCOUNT_RESET,
                Some(id),
                Box::new(move || engine.reset_account(&p.account_id, p.keep_metadata)),
            )
        }
        method::SECRETS_SET => Ok(Now(engine.secrets_set(&req))),
        method::SECRETS_DELETE => Ok(Now(engine.secrets_delete(&req))),
        method::SECRETS_HAS => Ok(Now(engine.secrets_has(&req))),
        method::SERVER_SET | method::SERVER_CLEAR => later(
            &workers::ENGINE_MAINTENANCE,
            None,
            Box::new(move || engine.server_call(&req)),
        ),
        other => Err(ErrorObject::new(
            code::METHOD_NOT_FOUND,
            format!("unknown method {other}"),
        )),
    }
}

/// `account.test`'s result without the secrets of its `account`.
fn tested_publicly(mut result: Value) -> Value {
    if let Some(account) = result.get_mut("account").filter(|a| a.is_object()) {
        *account = registry::strip_secrets(account);
    }
    result
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

/// Not started until the client sends its credentials again.
fn needs_credentials_error(message: &str) -> Value {
    json!({
        "what": tern_core::error::SYNC_ERROR_WHAT,
        "key": "ErrorAccountNotStarted",
        "debuginfo": message,
        "retryable": false,
        "offline": false,
    })
}

/// The most names `secrets.has` takes.
const MAX_SECRET_NAMES: usize = 256;

/// An AI provider's key decides `capabilities.ai`.
fn secret_changed(name: &str) {
    if name.starts_with(tern_secrets::names::AI_PROVIDER_PREFIX) {
        crate::ai_triage::capabilities_changed();
    }
}

impl Engine {
    /// The identity for an account added without one: the latest `account.add` gave, or
    /// else the Tern server of `server.set` with its token, or none.
    fn default_identity(&self) -> Value {
        let explicit = self.identity_lock().clone();
        if !explicit.is_null() {
            return explicit;
        }
        self.server_identity()
    }

    fn server_identity(&self) -> Value {
        let Some(server) = self.registered.server() else {
            return Value::Null;
        };
        let token = match self
            .registered
            .secret(tern_secrets::names::TERN_SERVER_TOKEN)
        {
            Ok(Some(t)) => t,
            Ok(None) => {
                warn!(
                    "-X A Tern server is set but its token isn't stored; syncing metadata locally"
                );
                return Value::Null;
            }
            Err(e) => {
                warn!("-X The Tern server's token can't be read: {e}");
                return Value::Null;
            }
        };
        let field = |k: &str| {
            server
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        TernServer {
            url: field("url"),
            token,
            user_id: field("userId"),
            server_id: field("serverId"),
        }
        .to_json()
    }

    /// `account.oauth.begin`. A reconnect names a registered account of the same provider.
    fn oauth_begin(&self, req: &RpcRequest) -> Reply {
        let p: OAuthBeginParams = req.params_as()?;
        if let Some(id) = &p.account_id {
            let registered = self
                .registered
                .get(id)
                .ok_or_else(|| socket::unknown_account(id))?;
            if registered.get("provider").and_then(Value::as_str) != Some(p.provider.as_str()) {
                return Err(invalid(format!(
                    "invalid params: account {id} is not a {} account",
                    p.provider
                )));
            }
        }
        serde_json::to_value(self.oauth.begin(&p)?).map_err(internal)
    }

    /// `account.oauth.complete`: exchanges the code, reads the profile, fills in the servers
    /// and adds the account as `account.add {test}` would. A reconnect keeps the account's
    /// id and settings and replaces only its grant.
    fn oauth_complete(&self, p: &OAuthCompleteParams) -> Reply {
        let session = self.oauth.take(p)?;
        let signed_in = accounts_api::sign_in(&session, &p.code)?;
        let address = |v: &Value| {
            v.get("emailAddress")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
        };
        let json = match &session.account_id {
            Some(id) => {
                let mut registered = self
                    .registered
                    .get(id)
                    .ok_or_else(|| socket::unknown_account(id))?;
                if address(&registered) != address(&signed_in) {
                    return Err(ErrorObject::new(
                        code::OAUTH_FAILED,
                        format!(
                            "You signed in as {}, not as the account's {}",
                            address(&signed_in).unwrap_or_default(),
                            address(&registered).unwrap_or_default()
                        ),
                    )
                    .with_data(json!({ "key": "wrong-account" })));
                }
                if let Some(settings) = registered
                    .get_mut("settings")
                    .and_then(Value::as_object_mut)
                {
                    for k in ["refresh_client_id", "refresh_token"] {
                        if let Some(v) = signed_in.pointer(&format!("/settings/{k}")) {
                            settings.insert(k.to_owned(), v.clone());
                        }
                    }
                }
                registered
            }
            None => accounts_api::with_discovered_servers(
                signed_in,
                session.container_folder.as_deref(),
            ),
        };
        let fallback = registry::stored_form(&json);
        let added: AccountAddResult = serde_json::from_value(self.add_account(
            json,
            Value::Null,
            p.test.unwrap_or(true),
            session.account_id.is_some(),
        )?)
        .map_err(internal)?;
        let account = self.registered.get(&added.account_id).unwrap_or(fallback);
        serde_json::to_value(OAuthCompleteResult { account, added }).map_err(internal)
    }

    /// `account.setSecrets`: stores new credentials (the others stay) and restarts the
    /// account with them, testing them first with `test`.
    fn set_secrets(&self, req: &RpcRequest) -> Reply {
        let p: AccountSetSecretsParams = req.params_as()?;
        if p.secrets.entries().is_empty() {
            return Err(invalid("invalid params: secrets names none"));
        }
        let mut json = self
            .registered
            .get(&p.account_id)
            .ok_or_else(|| socket::unknown_account(&p.account_id))?;
        p.secrets.apply_to(&mut json);
        let identity = self
            .lock()
            .get(&p.account_id)
            .map(|s| s.identity.clone())
            .unwrap_or(Value::Null);
        self.add_account(json, identity, p.test, true)
    }

    /// `account.update`: patches the registered account. A change of where it connects
    /// deletes its secrets and stops it until `account.setSecrets`; another change but how
    /// it is shown restarts it.
    fn update(&self, req: &RpcRequest) -> Reply {
        let p: AccountUpdateParams = req.params_as()?;
        let id = p.account_id;
        let before = self
            .registered
            .get(&id)
            .ok_or_else(|| socket::unknown_account(&id))?;
        let after = registry::patched(&before, &p.patch)
            .map(|a| registry::stored_form(&a))
            .map_err(|e| invalid(format!("invalid params: {e}")))?;
        // Checked as an account whatever secrets it holds: they aren't in `after`.
        let mut probe = after.clone();
        if let Some(settings) = probe.get_mut("settings").and_then(Value::as_object_mut) {
            for k in ["imap_password", "smtp_password", "refresh_token"] {
                settings.entry(k).or_insert_with(|| json!("-"));
            }
        }
        parse_account(probe)?;
        let result = |secrets_cleared, restarted| {
            serde_json::to_value(AccountUpdateResult {
                account: after.clone(),
                secrets_cleared,
                restarted,
            })
            .map_err(internal)
        };
        if after == before {
            return result(false, false);
        }
        let cleared = self.registered.update(&id, &after).map_err(internal)?;
        self.announce(&after);
        let shown_only = {
            let hide = |v: &Value| {
                let mut v = v.clone();
                if let Some(fields) = v.as_object_mut() {
                    for k in ACCOUNT_DISPLAY_FIELDS {
                        fields.remove(*k);
                    }
                }
                v
            };
            hide(&before) == hide(&after)
        };
        let slot = self.lock().get(&id).map(|s| s.identity.clone());
        if cleared {
            info!("Account {id} moved to other servers; its secrets are deleted");
            if slot.is_some() {
                self.remove_account(&id, false)?;
            }
            let error = needs_credentials_error(
                "the account's servers changed: send its credentials again (account.setSecrets)",
            );
            self.unstarted_lock().insert(id.clone(), error.clone());
            self.notify_state(&id, account_state::STOPPED, Some(error));
            return result(true, false);
        }
        let Some(identity) = slot else {
            return result(false, false);
        };
        if shown_only {
            let mut reg = self.lock();
            if let Some(s) = reg.get_mut(&id) {
                let patched = registry::patched(s.account.as_json(), &p.patch)
                    .ok()
                    .and_then(|j| Account::from_json(j).ok());
                if let Some(a) = patched {
                    s.account = Arc::new(a);
                }
            }
            return result(false, false);
        }
        info!("Restarting account {id} with new settings");
        let full = self
            .registered
            .with_secrets(after.clone())
            .map_err(internal)?;
        let account = parse_account(full)?;
        self.restarts_lock().remove(&id);
        self.remove_account(&id, false)?;
        self.start(account, identity)?;
        result(false, true)
    }

    /// `account.reorder`.
    fn reorder(&self, req: &RpcRequest) -> Reply {
        let p: AccountReorderParams = req.params_as()?;
        self.registered.reorder(&p.ids).map_err(internal)?;
        Ok(json!({}))
    }

    /// `secrets.set`: a secret that isn't an account's (`integration:<provider>`,
    /// `tern-server-token`, ...). Accounts started after a `tern-server-token` change use it;
    /// `server.set` restarts the running ones.
    fn secrets_set(&self, req: &RpcRequest) -> Reply {
        let p: SecretsSetParams = req.params_as()?;
        if !registry::writable_secret_name(&p.name) {
            return Err(invalid(format!(
                "invalid params: {:?} is not a secret name secrets.set writes",
                p.name
            )));
        }
        if p.value.is_empty() || p.value.len() > registry::MAX_SECRET_BYTES {
            return Err(invalid(format!(
                "invalid params: the value must hold 1 to {} bytes",
                registry::MAX_SECRET_BYTES
            )));
        }
        self.registered
            .set_secret(&p.name, &p.value)
            .map_err(internal)?;
        info!("Secret {} stored", p.name);
        secret_changed(&p.name);
        Ok(json!({}))
    }

    fn secrets_delete(&self, req: &RpcRequest) -> Reply {
        let p: SecretsDeleteParams = req.params_as()?;
        if !registry::writable_secret_name(&p.name) {
            return Err(invalid(format!(
                "invalid params: {:?} is not a secret name secrets.delete deletes",
                p.name
            )));
        }
        self.registered.forget_secret(&p.name).map_err(internal)?;
        info!("Secret {} deleted", p.name);
        secret_changed(&p.name);
        Ok(json!({}))
    }

    /// `secrets.has`: whether each name holds a value; any name, an account's included.
    fn secrets_has(&self, req: &RpcRequest) -> Reply {
        let p: SecretsHasParams = req.params_as()?;
        if p.names.len() > MAX_SECRET_NAMES || p.names.iter().any(|n| n.len() > 256) {
            return Err(invalid(format!(
                "invalid params: at most {MAX_SECRET_NAMES} names of up to 256 bytes"
            )));
        }
        let found = self.registered.has_secrets(&p.names).map_err(internal)?;
        serde_json::to_value(found).map_err(internal)
    }

    /// `server.set` / `server.clear`: the Tern server the accounts sync metadata with. The
    /// running accounts restart with it (or with none).
    fn server_call(&self, req: &RpcRequest) -> Reply {
        if req.method == method::SERVER_SET {
            let p: ServerSetParams = req.params_as()?;
            let probe = TernServer {
                url: p.url.clone(),
                token: "token".into(),
                user_id: p.user_id.clone(),
                server_id: p.server_id.clone(),
            };
            TernServer::from_json(&probe.to_json())
                .map_err(|e| invalid(format!("invalid params: {}: {}", e.field, e.reason)))?;
            let next = json!({"url": p.url, "userId": p.user_id, "serverId": p.server_id});
            // The token was given for the stored server: another one doesn't get it, as an
            // account's secrets go when its host changes (section 5). The client sends it
            // again with `secrets.set`. A token stored before any server (credentials.import)
            // is kept.
            let moved = self.registered.server().is_some_and(|prev| prev != next);
            self.registered.set_server(&next).map_err(internal)?;
            if moved {
                self.registered
                    .delete_secret(tern_secrets::names::TERN_SERVER_TOKEN)
                    .map_err(internal)?;
            }
            info!("Tern server set to {}", probe.host());
        } else {
            self.registered.clear_server().map_err(internal)?;
            info!("Tern server cleared; metadata syncs locally");
        }
        // The restarted accounts' backends report the new server's features.
        crate::server_features::reported(None);
        // The server replaces the identity clients passed to account.add.
        *self.identity_lock() = Value::Null;
        let restarted = self.restart_running(Some(self.server_identity()))?;
        Ok(json!({ "restarted": restarted }))
    }
}

/// An account setting that makes `account.test` panic in debug builds.
const TEST_PANIC_SETTING: &str = "tern_test_panic";

/// How long a detached engine waits after answering `engine.shutdown` before it exits.
const SHUTDOWN_FLUSH_MS: u64 = 300;

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
        match tern_ipc::read_frame(&mut reader, &mut line, MAX_LINE_BYTES) {
            Ok(true) => {}
            Ok(false) | Err(tern_ipc::FrameError::Io(_)) => {
                if orphan {
                    return;
                }
                crate::stdin_loop::wait_for_parent_then_exit();
            }
            Err(tern_ipc::FrameError::TooLong) => {
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
        let req = match tern_ipc::parse_request(&line) {
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
        if socket::is_account_method(&req.method) {
            let request_id = req.id.clone();
            let answer = move |result: Reply| {
                if let Some(id) = request_id {
                    respond(&match result {
                        Ok(v) => Response::ok(Some(id), v),
                        Err(e) => Response::err(Some(id), e),
                    });
                }
            };
            match account_call(engine.clone(), &req) {
                SocketParentCall::Now(r) => answer(r),
                SocketParentCall::Later((lane, account, work)) => {
                    lane.submit(account, work, answer)
                }
            }
            continue;
        }
        if is_slow_parent_method(&req.method) {
            let request_id = req.id.clone();
            let answer = move |result: Reply| {
                if let Some(id) = request_id {
                    respond(&match result {
                        Ok(v) => Response::ok(Some(id), v),
                        Err(e) => Response::err(Some(id), e),
                    });
                }
            };
            match slow_parent_job(engine.clone(), &req) {
                Ok((lane, account, work)) => lane.submit(account, work, answer),
                Err(e) => answer(Err(e)),
            }
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

fn is_slow_parent_method(name: &str) -> bool {
    matches!(
        name,
        method::ENGINE_RESET_DATABASE | method::ENGINE_SET_LOG_LEVEL
    )
}

/// A slow engine method's job for its lane's workers ([`crate::workers`]), which answer
/// when done, on stdin or on a detached engine's socket. They run one at a time and abort on
/// a panic, as the stdin thread would.
fn slow_parent_job(engine: Arc<Engine>, req: &RpcRequest) -> Result<SocketJob, ErrorObject> {
    Ok(match req.method.as_str() {
        method::ENGINE_RESET_DATABASE => {
            let p = req.params_as::<ResetDatabaseParams>()?;
            (
                &workers::ENGINE_MAINTENANCE,
                None,
                Box::new(move || engine.reset_database(p.keep_metadata)),
            )
        }
        _ => {
            let params = req.clone();
            (
                &workers::ENGINE_MAINTENANCE,
                None,
                Box::new(move || engine.set_log_level(&params)),
            )
        }
    })
}

fn respond(response: &Response) {
    write_stdout(&String::from_utf8_lossy(&tern_ipc::encode(response)));
}

fn handle_parent(engine: &Engine, session: &mut Session, req: &RpcRequest) -> Reply {
    match req.method.as_str() {
        method::CREDENTIALS_IMPORT => engine.import(req),
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

/// The socket path `serve` uses without `--socket`.
pub fn default_socket(config_dir: &Path) -> PathBuf {
    config_dir.join("run").join("engine.sock")
}
