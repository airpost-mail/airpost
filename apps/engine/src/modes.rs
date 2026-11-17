// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Mode dispatch, in `main.cpp`'s order: `migrate` and `install-check` before the account
//! is read, `reset` before the identity, then logging setup, then `test` or `sync`.
//!
//! Derived from Mailspring-Sync `MailSync/main.cpp`, Copyright © 2017 Foundry 376, GPL-3.0.
//! See NOTICE.md.

mod import_legacy;
mod install_check;
pub(crate) mod test_auth;

use std::io::BufRead;
use std::sync::Arc;

use airpost_core::constants::{EXIT_FAILURE, EXIT_OK};
use airpost_core::{Account, Identity};
use airpost_net::{AirpostServer, HttpMetadataBackend, LocalOnly, MetadataBackend};
use serde_json::{json, Value};

use crate::cli::{Mode, Options};
use crate::output::{one_shot, one_shot_error, write_stdout};
use crate::{dispatch, stdin_loop};

pub fn run(opts: Options) -> i32 {
    match opts.mode {
        Mode::Migrate => return migrate(&opts),
        Mode::InstallCheck => return install_check::run(),
        Mode::ImportLegacy => return import_legacy::run(&opts),
        Mode::Serve => return serve(&opts),
        _ => {}
    }
    if opts.mode == Mode::Sync && opts.socket.is_some() && cfg!(not(unix)) {
        one_shot_error("--socket is not supported on this platform yet");
        return EXIT_FAILURE;
    }

    let account = match read_account(&opts) {
        Ok(a) => Arc::new(a),
        Err(code) => return code,
    };

    if opts.mode == Mode::Reset {
        return run_single(|| {
            let store =
                airpost_store::Store::open(&opts.config_dir, airpost_store::DeltaStream::stdout())?;
            store.reset_for_account(account.id(), true)
        });
    }

    let identity = match read_identity(&opts) {
        Ok(i) => i,
        Err(code) => return code,
    };

    let log_to_file = opts.mode == Mode::Sync && !opts.orphan;
    let logger = if log_to_file {
        airpost_core::log::init_file(&airpost_core::log::log_path(&opts.config_dir, account.id()))
    } else {
        airpost_core::log::init_stdout()
    };
    if let Err(e) = logger {
        one_shot_error(&format!("Setup Failed: {e}"));
        return EXIT_FAILURE;
    }
    install_panic_hook(false);

    match opts.mode {
        Mode::Test => test_auth::run(&account),
        Mode::Sync => sync(&opts, account, identity),
        Mode::Migrate | Mode::InstallCheck | Mode::ImportLegacy | Mode::Reset | Mode::Serve => {
            unreachable!()
        }
    }
}

#[cfg(unix)]
fn serve(opts: &Options) -> i32 {
    crate::serve::run(opts)
}

#[cfg(not(unix))]
fn serve(_opts: &Options) -> i32 {
    one_shot_error("--mode serve is not supported on this platform yet");
    EXIT_FAILURE
}

/// `runSingleFunctionAndExit`: `"\n" + {"error":null}` or `{"error":"<what>"}`.
fn run_single(f: impl FnOnce() -> airpost_core::Result<()>) -> i32 {
    match f() {
        Ok(()) => {
            one_shot(&json!({ "error": null }));
            EXIT_OK
        }
        Err(e) => {
            one_shot(&json!({ "error": error_what(&e) }));
            EXIT_FAILURE
        }
    }
}

/// What `std::exception::what()` gave for the same failure in C++.
fn error_what(e: &airpost_core::Error) -> String {
    match e {
        airpost_core::Error::Sync(_) => airpost_core::error::SYNC_ERROR_WHAT.to_owned(),
        other => other.to_string(),
    }
}

fn migrate(opts: &Options) -> i32 {
    run_single(|| {
        let clock = airpost_core::SystemClock;
        let mut out = std::io::stdout();
        airpost_store::migrate::migrate(&opts.config_dir, &clock, &mut out).map(|_| ())
    })
}

fn read_line_after_prompt(prompt: &str) -> String {
    write_stdout(prompt);
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim_end_matches(['\r', '\n']).to_owned()
}

fn read_account(opts: &Options) -> Result<Account, i32> {
    let text = match &opts.account {
        Some(a) => a.clone(),
        None => read_line_after_prompt("\nWaiting for Account JSON:\n"),
    };
    let account = serde_json::from_str::<Value>(&text)
        .map_err(|e| e.to_string())
        .and_then(|v| Account::from_json(v).map_err(|e| e.to_string()));
    let account = match account {
        Ok(a) => a,
        Err(e) => {
            one_shot(&json!({ "error": format!("Invalid Account JSON: {e}"), "log": text }));
            return Err(EXIT_FAILURE);
        }
    };
    if let Err(e) = account.validate() {
        one_shot_error(&e.to_string());
        return Err(EXIT_FAILURE);
    }
    Ok(account)
}

/// What the identity line selects (docs/backend-plan.md 4.1, O7).
#[derive(Debug)]
pub(crate) enum IdentityLine {
    /// `null`, or a Mailspring-shaped identity, which is validated and then ignored.
    None,
    Airpost(AirpostServer),
    /// A `airpost-server` object that failed validation: logged once logging is up, then
    /// `LocalOnly`. Mail sync carries on.
    InvalidAirpost(airpost_net::AirpostServerError),
}

/// Why an identity line was refused: the one-shot error, and the text to echo as `log`
/// (empty when it may carry the Airpost server token).
pub(crate) struct IdentityError {
    pub message: String,
    pub log: Option<String>,
}

fn read_identity(opts: &Options) -> Result<IdentityLine, i32> {
    let text = match &opts.identity {
        Some(i) => i.clone(),
        None => read_line_after_prompt("\nWaiting for Identity JSON:\n"),
    };
    parse_identity(&text).map_err(|e| {
        match e.log {
            Some(log) => one_shot(&json!({ "error": e.message, "log": log })),
            None => one_shot_error(&e.message),
        }
        EXIT_FAILURE
    })
}

/// Reads the identity line `sync` mode takes on stdin and `serve` takes in `account.add`.
pub(crate) fn parse_identity(text: &str) -> Result<IdentityLine, IdentityError> {
    if text == "null" {
        return Ok(IdentityLine::None);
    }
    let value = serde_json::from_str::<Value>(text);
    if let Ok(v) = &value {
        if AirpostServer::is_airpost_server(v) {
            return Ok(match AirpostServer::from_json(v) {
                Ok(s) => IdentityLine::Airpost(s),
                Err(e) => IdentityLine::InvalidAirpost(e),
            });
        }
    }
    let identity = value
        .map_err(|e| e.to_string())
        .and_then(|v| Identity::from_json(v).map_err(|e| e.to_string()));
    match identity {
        Ok(i) if i.valid() => Ok(IdentityLine::None),
        Ok(_) => Err(IdentityError {
            message: "ErrorIdentityMissingFields".to_owned(),
            log: None,
        }),
        Err(e) => {
            // A malformed Airpost server line carries the token: never echo it.
            let log = if text.contains("airpost-server") {
                ""
            } else {
                text
            };
            Err(IdentityError {
                message: format!("Invalid Identity JSON: {e}"),
                log: Some(log.to_owned()),
            })
        }
    }
}

/// A panic is a bug: log it as critical (which also writes stderr, where the client picks
/// it up) and abort, as the C++ top-level handler did for unexpected exceptions.
///
/// In `serve` mode a panic on one of an account's threads (they carry its id, and
/// `WorkerContext::spawn` catches the unwind) only stops that account: it is logged at
/// error to the account's log and unwinds. So does a panic on a contained worker
/// ([`crate::workers`]), which answers its call with an error. Engine-wide threads still
/// abort the process.
pub(crate) fn install_panic_hook(serve: bool) {
    std::panic::set_hook(Box::new(move |info| {
        let current = std::thread::current();
        let name = current.name().unwrap_or("").to_owned();
        // A contained worker answers the call that panicked and goes on.
        if crate::workers::is_contained(&name)
            || (serve && airpost_core::log::thread_account().is_some() && !is_engine_thread(&name))
        {
            tracing::error!("Unhandled panic on thread {name}: {info}");
            return;
        }
        airpost_core::critical!("Unhandled panic on thread {name}: {info}");
        airpost_core::log::flush();
        std::process::abort();
    }));
}

/// Threads that serve every account. `serve` mode's stdin thread carries an account id for
/// a moment while it adds one, and must still abort on a panic.
fn is_engine_thread(name: &str) -> bool {
    use airpost_core::constants::thread;
    [
        thread::STDIN,
        thread::SOCKET,
        thread::SOCKET_READER,
        thread::SOCKET_WRITER,
        thread::LIVE_QUERIES,
        thread::SIGNALS,
        thread::ACCOUNT_STOP,
        thread::ACCOUNT_RESTART,
        thread::BUILD_ID,
        thread::ENGINE_MAINTENANCE,
    ]
    .contains(&name)
}

pub(crate) fn spawn(
    name: &'static str,
    f: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    match std::thread::Builder::new().name(name.into()).spawn(f) {
        Ok(h) => h,
        Err(e) => {
            airpost_core::critical!("Could not start the {name} thread: {e}");
            airpost_core::log::flush();
            std::process::abort();
        }
    }
}

/// The HTTP backend for a valid Airpost server object, `LocalOnly` otherwise.
pub(crate) fn metadata_backend(
    config_dir: &std::path::Path,
    account: &Account,
    identity: IdentityLine,
    deltas: &Arc<airpost_store::DeltaStream>,
) -> Arc<dyn MetadataBackend> {
    use airpost_core::logmsg::{metadata_backend_airpost, metadata_backend_local};
    let server = match identity {
        IdentityLine::None => {
            tracing::info!("{}", metadata_backend_local("no server"));
            return Arc::new(LocalOnly);
        }
        IdentityLine::InvalidAirpost(e) => {
            tracing::error!("Ignoring the Airpost server identity: {e}");
            tracing::info!(
                "{}",
                metadata_backend_local(&format!("invalid {}", e.field))
            );
            return Arc::new(LocalOnly);
        }
        IdentityLine::Airpost(s) => s,
    };
    let host = server.host().to_owned();
    let deltas = deltas.clone();
    let on_refresh = Box::new(move || deltas.send_identity_refresh_needed());
    match HttpMetadataBackend::new(server, account.id(), config_dir, on_refresh) {
        Ok(b) => {
            let b = b.with_me_listener(Box::new(crate::server_features::reported));
            tracing::info!("{}", metadata_backend_airpost(&host));
            Arc::new(b)
        }
        Err(e) => {
            tracing::error!("Could not set up the Airpost server backend: {e}");
            tracing::info!("{}", metadata_backend_local("setup failed"));
            Arc::new(LocalOnly)
        }
    }
}

fn sync(opts: &Options, account: Arc<Account>, identity: IdentityLine) -> i32 {
    tracing::info!(
        "{}",
        airpost_core::logmsg::starting_sync(account.email_address())
    );

    let started = std::time::Instant::now();
    #[cfg(unix)]
    let socket = match &opts.socket {
        Some(path) => match bind_socket(opts, &account, path) {
            Ok(s) => Some(s),
            Err(code) => return code,
        },
        None => None,
    };
    #[cfg(unix)]
    let deltas = match &socket {
        Some((hub, _)) => airpost_store::DeltaStream::with_sink(hub.sink(
            account.id(),
            Box::new(airpost_store::WriteSink(std::io::stdout())),
        )),
        None => airpost_store::DeltaStream::stdout(),
    };
    #[cfg(not(unix))]
    let deltas = airpost_store::DeltaStream::stdout();
    let metadata = metadata_backend(&opts.config_dir, &account, identity, &deltas);
    let deltas_for_tokens = deltas.clone();
    let tokens = airpost_net::TokenManager::new(Some(Box::new(move |a: &Account| {
        deltas_for_tokens.send_updated_secrets(a)
    })));
    let mut ctx = airpost_sync::WorkerContext::new(
        account,
        opts.config_dir.clone(),
        deltas,
        Arc::new(tokens),
        metadata,
        Arc::new(airpost_tasks::Tasks),
        Arc::new(abort_on_failure),
    );
    ctx.verbose = opts.verbose;

    #[cfg(unix)]
    if let Some((hub, listener)) = socket {
        let path = opts.socket.clone().unwrap_or_default();
        let (account, deltas, orphan) = (ctx.account.clone(), ctx.deltas.clone(), opts.orphan);
        let config_dir = ctx.config_dir.clone();
        let dispatcher: Dispatcher = Box::new(move |ctx, rx, commands| {
            dispatch::run(ctx, rx, move || {
                let accounts = crate::socket::SingleAccount {
                    account,
                    deltas,
                    commands: commands.clone(),
                };
                crate::socket::serve(
                    listener,
                    Arc::new(crate::socket::Server {
                        hub,
                        accounts: Arc::new(accounts),
                        started,
                        config_dir,
                        database: Default::default(),
                        settings: None,
                    }),
                );
                let shown = path.to_string_lossy();
                tracing::info!("{}", airpost_core::logmsg::socket_listening(&shown));
                write_stdout(&format!("{}\n", json!({ "listening": shown })));
                if !orphan {
                    stdin_loop::spawn_reader(commands);
                }
            })
        });
        return run_account(ctx, Some(dispatcher));
    }
    #[cfg(not(unix))]
    let _ = started;

    // `--orphan` has no transport, so no `main` thread: the process waits on background.
    let dispatcher: Option<Dispatcher> = (!opts.orphan).then(|| -> Dispatcher {
        Box::new(|ctx, rx, commands| {
            dispatch::run(ctx, rx, move || stdin_loop::spawn_reader(commands))
        })
    });
    run_account(ctx, dispatcher)
}

type Dispatcher = airpost_sync::runtime::Dispatcher<dispatch::Command>;

/// Starts the account's threads (the foreground thread is started by background after its
/// first pass) and blocks until its `main` thread exits, which in practice it doesn't: the
/// stdin lifeline or a signal ends the process.
fn run_account(ctx: airpost_sync::WorkerContext, dispatcher: Option<Dispatcher>) -> i32 {
    airpost_sync::AccountRuntime::start(ctx, dispatcher).wait();
    EXIT_OK
}

/// `sync` mode's failure handler: the account is the process, so a failure ends it, as the
/// C++ worker loops and main loop did.
fn abort_on_failure(failure: &airpost_sync::AccountFailure) {
    airpost_core::critical!("{}", failure.message);
    airpost_core::log::flush();
    std::process::abort();
}

/// Binds `--socket` and writes the discovery file, before any worker starts, so a path that
/// cannot be used fails the launch with a one-shot error. Both files are removed when the
/// lifeline exits the process or SIGTERM / SIGINT ends it.
#[cfg(unix)]
fn bind_socket(
    opts: &Options,
    account: &Account,
    path: &std::path::Path,
) -> Result<(Arc<crate::socket::Hub>, std::os::unix::net::UnixListener), i32> {
    let fail = |message: String| {
        tracing::error!("-X {message}");
        one_shot_error(&message);
        EXIT_FAILURE
    };
    let listener = crate::platform::uds::bind(path).map_err(|e| fail(e.message))?;
    crate::platform::remove_on_exit(path.to_owned());
    match crate::socket::write_discovery(&opts.config_dir, account.id(), path) {
        Ok(discovery) => crate::platform::remove_on_exit(discovery),
        Err(e) => return Err(fail(format!("Could not write the discovery file: {e}"))),
    }
    if let Err(e) = crate::platform::remove_on_termination() {
        tracing::warn!("-X Could not watch for SIGTERM; the socket files may stay behind: {e}");
    }
    let hub = crate::socket::Hub::new(crate::socket::queue_limit());
    Ok((hub, listener))
}
