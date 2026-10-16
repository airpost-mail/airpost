// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! tern-mcp: an MCP server for the mail in a Tern profile. It reads through the running
//! engine's read API and changes mail by queueing the desktop's tasks, so it works with
//! the desktop closed as long as an engine runs. stdout carries MCP; stderr is the log.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serde_json::{json, Value};

mod audit;
mod config;
mod discovery;
mod drafts;
mod http;
mod install;
mod model;
mod platform;
mod policy;
mod search;
mod server;
mod tasks;
mod tools;

/// Writes a line to stderr, which MCP clients keep as the server's log.
pub fn log(msg: &str) {
    eprintln!("[tern-mcp] {msg}");
}

#[derive(Parser, Debug)]
#[command(name = "tern-mcp", version, about = "MCP server for the mail in Tern")]
struct Cli {
    /// The Tern profile directory (default: the desktop's).
    #[arg(long, global = true, env = "TERN_CONFIG_DIR", value_name = "DIR")]
    config_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Serve MCP on stdin/stdout (what running without a subcommand does).
    Stdio,
    /// Serve MCP over HTTP on 127.0.0.1 with a bearer token; needs http.enabled in mcp.json.
    Http {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Register tern-mcp as the `tern` server in an MCP client.
    Install {
        #[arg(long, value_enum)]
        client: install::McpClient,
        /// Edit this file instead of the client's own config.
        #[arg(long, value_name = "PATH")]
        config_file: Option<PathBuf>,
        /// Replace a `tern` entry that runs something other than tern-mcp.
        #[arg(long)]
        force: bool,
        /// Only replace the old desktop plugin's `mcp-stdio-bridge.js` entry for this
        /// profile; do nothing when there is none.
        #[arg(long, conflicts_with = "force")]
        if_stale: bool,
    },
    /// Remove the `tern` server from an MCP client.
    Uninstall {
        #[arg(long, value_enum)]
        client: install::McpClient,
        #[arg(long, value_name = "PATH")]
        config_file: Option<PathBuf>,
    },
    /// Show the engines, the policy and the files tern-mcp uses.
    Status {
        #[arg(long)]
        json: bool,
    },
}

fn config_dir(cli: &Cli) -> Result<PathBuf, String> {
    let dir = cli
        .config_dir
        .clone()
        .or_else(config::default_config_dir)
        .ok_or(
        "could not work out the Tern profile directory; give --config-dir or set TERN_CONFIG_DIR",
    )?;
    // Absolute, so an installed entry doesn't depend on the client's working directory.
    Ok(std::path::absolute(&dir).unwrap_or(dir))
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start the async runtime: {e}"))
}

fn status(dir: &Path) -> Value {
    let desktop = config::DesktopConfig::load(dir);
    let policy = match policy::Policy::load(dir, &desktop) {
        Ok(p) => p.summary(),
        Err(e) => json!({"error": e}),
    };
    let engines = discovery::connect(
        dir,
        &format!("tern-mcp/{} (status)", env!("CARGO_PKG_VERSION")),
    );
    let list: Vec<Value> = engines
        .engines
        .iter()
        .map(|e| {
            json!({
                "file": e.file.display().to_string(),
                "pid": e.hello.pid,
                "socket": e.entry.socket.display().to_string(),
                "engine": e.hello.engine,
                "protocol": e.hello.protocol,
                "accounts": e.hello.accounts.iter().map(|a| json!({
                    "accountId": a.account_id, "email": a.email, "state": a.state,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "configDir": dir.display().to_string(),
        "engines": list,
        "problems": engines.problems,
        "policy": policy,
        "auditLog": dir.join(audit::AUDIT_FILE).display().to_string(),
        "tokenFile": http::token_path(dir).display().to_string(),
    })
}

fn print_status(s: &Value) {
    println!("Profile:   {}", s["configDir"].as_str().unwrap_or(""));
    let engines = s["engines"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if engines.is_empty() {
        println!("Engines:   none running");
    }
    for e in engines {
        println!(
            "Engine:    pid {} ({} {}), protocol {}",
            e["pid"],
            e["engine"].as_str().unwrap_or(""),
            e["socket"].as_str().unwrap_or(""),
            e["protocol"]
        );
        for a in e["accounts"].as_array().into_iter().flatten() {
            println!(
                "           {} {} ({})",
                a["accountId"].as_str().unwrap_or(""),
                a["email"].as_str().unwrap_or(""),
                a["state"].as_str().unwrap_or("")
            );
        }
    }
    for p in s["problems"].as_array().into_iter().flatten() {
        println!("Problem:   {}", p.as_str().unwrap_or(""));
    }
    let p = &s["policy"];
    if let Some(e) = p["error"].as_str() {
        println!("Policy:    error: {e}");
    } else {
        println!(
            "Policy:    {} (from {})",
            p["accessLevel"].as_str().unwrap_or(""),
            p["source"].as_str().unwrap_or("")
        );
        let accounts = p["enabledAccounts"].as_object().filter(|o| !o.is_empty());
        match accounts {
            None => println!("Accounts:  all"),
            Some(o) => {
                for (id, a) in o {
                    let state = if a["enabled"] == true {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    let excluded = a["excludedFolderIds"].as_array().map_or(0, Vec::len);
                    println!("Accounts:  {id} {state}, {excluded} excluded folders");
                }
            }
        }
        let http = &p["http"];
        println!(
            "HTTP:      {} (port {})",
            if http["enabled"] == true {
                "enabled"
            } else {
                "off"
            },
            http["port"]
        );
    }
    println!("Audit log: {}", s["auditLog"].as_str().unwrap_or(""));
    println!("Token:     {}", s["tokenFile"].as_str().unwrap_or(""));
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        None | Some(Command::Stdio) => {
            let dir = config_dir(&cli)?;
            let app = Arc::new(server::App::new(dir));
            runtime()?.block_on(server::serve_stdio(app))
        }
        Some(Command::Http { port }) => {
            let dir = config_dir(&cli)?;
            let desktop = config::DesktopConfig::load(&dir);
            let policy = policy::Policy::load(&dir, &desktop)?;
            if !policy.http.enabled {
                return Err(format!(
                    "HTTP is off; set \"http\": {{\"enabled\": true}} in {}",
                    dir.join(policy::POLICY_FILE).display()
                ));
            }
            let port = port.unwrap_or(policy.http.port);
            let app = Arc::new(server::App::new(dir));
            runtime()?.block_on(http::serve(app, port))
        }
        Some(Command::Install {
            client,
            ref config_file,
            force,
            if_stale,
        }) => {
            let dir = config_dir(&cli)?;
            let exe = std::env::current_exe()
                .and_then(std::fs::canonicalize)
                .map_err(|e| format!("could not find this binary's path: {e}"))?;
            let entry = install::ServerEntry::for_profile(exe, &dir);
            let replace = match (force, if_stale) {
                (true, _) => install::Replace::Any,
                (_, true) => install::Replace::OnlyStale,
                _ => install::Replace::Ours,
            };
            println!(
                "{}",
                install::install(client, config_file.as_deref(), &entry, replace)?
            );
            Ok(())
        }
        Some(Command::Uninstall {
            client,
            ref config_file,
        }) => {
            println!("{}", install::uninstall(client, config_file.as_deref())?);
            Ok(())
        }
        Some(Command::Status { json }) => {
            let dir = config_dir(&cli)?;
            let s = status(&dir);
            if json {
                println!("{}", serde_json::to_string_pretty(&s).expect("JSON"));
            } else {
                print_status(&s);
            }
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log(&e);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subcommands() {
        let cli = Cli::try_parse_from(["tern-mcp", "--config-dir", "/tmp/x"]).unwrap();
        assert!(cli.command.is_none());
        let cli = Cli::try_parse_from([
            "tern-mcp",
            "install",
            "--client",
            "claude-code",
            "--config-file",
            "/tmp/c.json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Install {
                client: install::McpClient::ClaudeCode,
                ..
            })
        ));
        assert!(Cli::try_parse_from(["tern-mcp", "install", "--client", "mailspring"]).is_err());
        assert!(Cli::try_parse_from([
            "tern-mcp",
            "install",
            "--client",
            "claude-desktop",
            "--if-stale",
            "--force",
        ])
        .is_err());
    }

    #[test]
    fn status_of_an_empty_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let s = status(tmp.path());
        assert_eq!(s["engines"], json!([]));
        assert_eq!(s["policy"]["accessLevel"], "read-only");
    }
}
