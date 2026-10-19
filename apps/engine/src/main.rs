// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `mailsync`: one process per account (`sync`), or every account in one process (`serve`,
//! docs/engine-protocol-plan.md section 4), plus one-shot modes (docs/engine-port-plan.md
//! section 3).
//!
//! Derived from Mailspring-Sync (`MailSync/main.cpp`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.

#[cfg(unix)]
mod accounts_api;
#[cfg(unix)]
mod ai_api;
#[cfg(unix)]
mod ai_triage;
mod cli;
mod dispatch;
mod drafts_api;
#[cfg(unix)]
mod files;
#[cfg(unix)]
mod integrations_api;
mod modes;
mod output;
mod platform;
#[cfg(unix)]
mod read_api;
#[cfg(unix)]
mod registry;
#[cfg(unix)]
mod restart;
#[cfg(unix)]
mod rules;
#[cfg(unix)]
mod serve;
#[cfg(unix)]
mod settings;
#[cfg(unix)]
mod socket;
mod stdin_loop;
#[cfg(unix)]
mod unsubscribe;
mod workers;

use std::process::ExitCode;

use ms_core::constants::{EXIT_FAILURE, EXIT_OK, EXIT_PATH_CHECK};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    // Release builds only run under a path naming the product (the C++ checked
    // "mailspring"), e.g. `Tern.app/.../app.asar.unpacked/mailsync`.
    if !cfg!(debug_assertions) {
        let exe = args.first().map(|a| a.to_lowercase()).unwrap_or_default();
        if !exe.contains("tern") {
            return code(EXIT_PATH_CHECK);
        }
    }

    platform::restrict_umask();

    // reqwest and lettre are built without a default provider (aws-lc is not
    // GPL-compatible); ring is installed process-wide before any TLS client exists.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let opts = match cli::parse(&args[1.min(args.len())..]) {
        cli::Parsed::Run(opts) => opts,
        cli::Parsed::Usage(exit) => {
            output::write_stdout(&cli::usage());
            return code(exit);
        }
    };
    code(modes::run(opts))
}

fn code(c: i32) -> ExitCode {
    match c {
        EXIT_OK => ExitCode::SUCCESS,
        EXIT_FAILURE => ExitCode::FAILURE,
        other => ExitCode::from(u8::try_from(other).unwrap_or(1)),
    }
}
