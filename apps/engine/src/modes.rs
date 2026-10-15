//! Mode dispatch, in `main.cpp`'s order: `migrate` and `install-check` before the account
//! is read, `reset` before the identity, then logging setup, then `test` or `sync`.

use std::io::BufRead;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use ms_core::constants::{thread, EXIT_FAILURE, EXIT_OK};
use ms_core::{Account, Identity};
use serde_json::{json, Value};

use crate::cli::{Mode, Options};
use crate::output::{one_shot, one_shot_error, write_stdout};
use crate::stdin_loop;

pub fn run(opts: Options) -> i32 {
    match opts.mode {
        Mode::Migrate => return migrate(&opts),
        Mode::InstallCheck => {
            one_shot_error("install-check is not implemented in this build");
            return EXIT_FAILURE;
        }
        Mode::ImportLegacy => {
            one_shot_error("import-legacy is not implemented in this build");
            return EXIT_FAILURE;
        }
        _ => {}
    }

    let account = match read_account(&opts) {
        Ok(a) => Arc::new(a),
        Err(code) => return code,
    };

    if opts.mode == Mode::Reset {
        return run_single(|| {
            let store = ms_store::Store::open(&opts.config_dir, ms_store::DeltaStream::stdout())?;
            store.reset_for_account(account.id())
        });
    }

    let identity = match read_identity(&opts) {
        Ok(i) => i.map(Arc::new),
        Err(code) => return code,
    };

    let log_to_file = opts.mode == Mode::Sync && !opts.orphan;
    let logger = if log_to_file {
        ms_core::log::init_file(&ms_core::log::log_path(&opts.config_dir, account.id()))
    } else {
        ms_core::log::init_stdout()
    };
    if let Err(e) = logger {
        one_shot_error(&format!("Setup Failed: {e}"));
        return EXIT_FAILURE;
    }
    install_panic_hook();

    match opts.mode {
        Mode::Test => {
            one_shot(&json!({
                "error": "test mode is not implemented in this build",
                "error_service": "imap",
                "log": "",
                "account": null,
            }));
            EXIT_FAILURE
        }
        Mode::Sync => sync(&opts, account, identity),
        Mode::Migrate | Mode::InstallCheck | Mode::ImportLegacy | Mode::Reset => unreachable!(),
    }
}

/// `runSingleFunctionAndExit`: `"\n" + {"error":null}` or `{"error":"<what>"}`.
fn run_single(f: impl FnOnce() -> ms_core::Result<()>) -> i32 {
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
fn error_what(e: &ms_core::Error) -> String {
    match e {
        ms_core::Error::Sync(_) => ms_core::error::SYNC_ERROR_WHAT.to_owned(),
        other => other.to_string(),
    }
}

fn migrate(opts: &Options) -> i32 {
    run_single(|| {
        let clock = ms_core::SystemClock;
        let mut out = std::io::stdout();
        ms_store::migrate::migrate(&opts.config_dir, &clock, &mut out).map(|_| ())
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
        .and_then(Account::from_json);
    let account = match account {
        Ok(a) => a,
        Err(e) => {
            one_shot(&json!({ "error": format!("Invalid Account JSON: {e}"), "log": text }));
            return Err(EXIT_FAILURE);
        }
    };
    let missing = account.valid();
    if !missing.is_empty() {
        one_shot_error(&format!("Account is missing required fields:{missing}"));
        return Err(EXIT_FAILURE);
    }
    Ok(account)
}

fn read_identity(opts: &Options) -> Result<Option<Identity>, i32> {
    let text = match &opts.identity {
        Some(i) => i.clone(),
        None => read_line_after_prompt("\nWaiting for Identity JSON:\n"),
    };
    if text == "null" {
        return Ok(None);
    }
    let identity = serde_json::from_str::<Value>(&text)
        .map_err(|e| e.to_string())
        .and_then(Identity::from_json);
    match identity {
        Ok(i) if i.valid() => Ok(Some(i)),
        Ok(_) => {
            one_shot_error("ErrorIdentityMissingFields");
            Err(EXIT_FAILURE)
        }
        Err(e) => {
            one_shot(&json!({ "error": format!("Invalid Identity JSON: {e}"), "log": text }));
            Err(EXIT_FAILURE)
        }
    }
}

/// A panic is a bug: log it as critical (which also writes stderr, where the client picks
/// it up) and abort, as the C++ top-level handler did for unexpected exceptions.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let name = std::thread::current().name().unwrap_or("").to_owned();
        ms_core::critical!("Unhandled panic on thread {name}: {info}");
        ms_core::log::flush();
        std::process::abort();
    }));
}

fn spawn(name: &'static str, f: impl FnOnce() + Send + 'static) -> std::thread::JoinHandle<()> {
    match std::thread::Builder::new().name(name.into()).spawn(f) {
        Ok(h) => h,
        Err(e) => {
            ms_core::critical!("Could not start the {name} thread: {e}");
            ms_core::log::flush();
            std::process::abort();
        }
    }
}

fn sync(opts: &Options, account: Arc<Account>, identity: Option<Arc<Identity>>) -> i32 {
    tracing::info!(
        "{}",
        ms_core::logmsg::starting_sync(account.email_address())
    );

    let deltas = ms_store::DeltaStream::stdout();
    let deltas_for_tokens = deltas.clone();
    let tokens = ms_net::TokenManager::new(Some(Box::new(move |a: &Account| {
        deltas_for_tokens.send_updated_secrets(a)
    })));
    let ctx = ms_sync::WorkerContext {
        account: account.clone(),
        identity,
        config_dir: opts.config_dir.clone(),
        deltas,
        clock: ms_core::clock::system(),
        verbose: opts.verbose,
        foreground: Arc::new(ms_sync::ForegroundControl::new()),
        background_mark_all: Arc::new(AtomicBool::new(false)),
        tokens: Arc::new(tokens),
        metadata: Arc::new(ms_net::LocalOnly),
        tasks: Arc::new(ms_tasks::Tasks),
    };

    // The foreground thread is started by background after its first pass.
    let c = ctx.clone();
    let background = spawn(thread::BACKGROUND, move || {
        ms_sync::background::run_background(c)
    });
    let c = ctx.clone();
    spawn(thread::CAL_CONTACTS, move || {
        ms_sync::cal_contacts::run_cal_contacts(c)
    });
    if !ctx.metadata.is_local_only() {
        let c = ctx.clone();
        spawn(thread::METADATA, move || {
            ms_sync::metadata_worker::run_metadata_worker(c)
        });
    }
    let c = ctx.clone();
    spawn(thread::METADATA_EXPIRATION, move || {
        ms_sync::metadata_expiration::run_metadata_expiration(c)
    });

    if opts.orphan {
        // Blocks forever: the worker never returns.
        let _ = background.join();
        EXIT_OK
    } else {
        stdin_loop::run(ctx)
    }
}
