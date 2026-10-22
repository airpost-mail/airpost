// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern`, the command-line client of the Tern mail engine.
//! It finds the running engines through their discovery files, talks to them
//! over the socket protocol with `tern-ipc`'s blocking client, and reads nothing else of the
//! profile but the engine logs.

mod account;
mod ai;
mod app;
mod builtin;
mod cli;
mod discovery;
mod draft;
mod error;
mod logs;
mod mail_actions;
mod output;
mod pim;
mod platform;
mod read;
mod service;
mod settings;
mod skill;
mod status;
mod tasks;
mod template;
mod units;
mod watch;

use clap::Parser;

fn main() {
    let cli = match cli::Cli::try_parse() {
        Ok(cli) => cli,
        // Help and --version exit 0, usage errors 2.
        Err(e) => e.exit(),
    };
    if let Err(e) = app::run(cli) {
        let _ =
            std::io::Write::write_all(&mut std::io::stderr(), format!("tern: {e}\n").as_bytes());
        std::process::exit(e.exit_code());
    }
}
