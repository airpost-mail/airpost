// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

fn main() -> std::process::ExitCode {
    let cli = airpost_gateway::cli::Cli::from_env();
    airpost_gateway::cli::init_logging();
    match airpost_gateway::cli::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("airpost-gateway: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
