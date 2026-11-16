// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli = airpost_backend::cli::Cli::parse();
    airpost_backend::cli::init_logging();
    match airpost_backend::cli::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("airpost-backend: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
