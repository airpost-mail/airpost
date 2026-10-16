// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `mailsync`: one process per account, plus one-shot modes (docs/engine-port-plan.md
//! section 3).
//!
//! Derived from Mailspring-Sync (`MailSync/main.cpp`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.

mod cli;
mod modes;
mod output;
mod platform;
mod stdin_loop;

use std::process::ExitCode;

use ms_core::constants::{EXIT_FAILURE, EXIT_OK, EXIT_PATH_CHECK};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    // Release builds only run under a path naming the product (the C++ checked
    // "mailspring"). The desktop app still ships as Mailspring.app and launches the engine
    // from `Mailspring.app/.../app.asar.unpacked/`, so "mailspring" is accepted alongside
    // "tern" until the desktop rebrand lands; then drop it.
    if !cfg!(debug_assertions) {
        let exe = args.first().map(|a| a.to_lowercase()).unwrap_or_default();
        if !exe.contains("tern") && !exe.contains("mailspring") {
            return code(EXIT_PATH_CHECK);
        }
    }

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
