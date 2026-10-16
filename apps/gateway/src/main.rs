// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli = tern_gateway::cli::Cli::parse();
    tern_gateway::cli::init_logging();
    match tern_gateway::cli::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tern-gateway: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
