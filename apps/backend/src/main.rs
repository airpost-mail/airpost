// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli = tern_backend::cli::Cli::parse();
    tern_backend::cli::init_logging();
    match tern_backend::cli::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tern-backend: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
