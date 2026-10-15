// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! Command line (docs/backend-plan.md 3.2). Logs go to stderr; command output to stdout.

use std::error::Error;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use crate::auth;
use crate::config::Config;
use crate::http::Server;
use crate::ids;
use crate::repo::{Repo, SqliteRepo, User, SCHEMA_VERSION};

#[derive(Debug, Parser)]
#[command(name = "tern-backend", version, about = "Self-hostable Tern server")]
pub struct Cli {
    /// TOML config file. TERN_DATA_DIR, TERN_BIND, TERN_PUBLIC_URL and TERN_SIGNUP override it.
    #[arg(long, global = true, env = "TERN_CONFIG")]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the HTTP server.
    Serve {
        /// Allow a plain-http public URL on a non-loopback host. Tokens then cross the network
        /// in clear text.
        #[arg(long)]
        insecure_public_url: bool,
    },
    /// Create or upgrade the database, then exit.
    Migrate,
    /// Write a consistent, owner-only copy of the database to PATH (`VACUUM INTO`). The copy
    /// gets a new server id, so engines re-sync from scratch if it is ever restored.
    Backup { path: PathBuf },
    /// Give the database a new server id. Run it, with the server stopped, after restoring
    /// the data dir from anything other than `backup` (a filesystem snapshot, say).
    RotateServerId,
    #[command(subcommand)]
    User(UserCommand),
    #[command(subcommand)]
    Token(TokenCommand),
}

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// Create a user. Without --password-stdin a random password is generated and printed.
    Add {
        email: String,
        #[arg(long, default_value = "")]
        first_name: String,
        #[arg(long, default_value = "")]
        last_name: String,
        /// Read the password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
    },
    List,
    /// Block logins and every token of the user, keeping their data.
    Disable {
        email: String,
    },
    Enable {
        email: String,
    },
    /// Delete the user and every row they own.
    Delete {
        email: String,
        /// Confirm the deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    /// Mint a device token for a user and print it.
    Create {
        email: String,
        #[arg(long, default_value = "cli")]
        name: String,
    },
    List {
        /// Only this user's tokens.
        #[arg(long)]
        user: Option<String>,
    },
    Revoke {
        id: i64,
    },
}

type CliResult = Result<(), Box<dyn Error>>;

pub fn init_logging() {
    use std::io::IsTerminal;
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

pub fn run(cli: Cli) -> CliResult {
    let config = Config::load(cli.config.as_deref(), |k| std::env::var(k).ok())?;
    let mut out = std::io::stdout().lock();
    match cli.command {
        Command::Serve {
            insecure_public_url,
        } => serve(config, insecure_public_url),
        Command::Migrate => {
            let repo = SqliteRepo::open(&config.data_dir)?;
            writeln!(
                out,
                "{}: schema version {SCHEMA_VERSION}, server id {}",
                config.db_path().display(),
                repo.server_id()?
            )?;
            Ok(())
        }
        Command::Backup { path } => {
            let id = SqliteRepo::open(&config.data_dir)?.backup(&path)?;
            writeln!(
                out,
                "wrote {} (server id {id} once restored)",
                path.display()
            )?;
            Ok(())
        }
        Command::RotateServerId => {
            let id = SqliteRepo::open(&config.data_dir)?.rotate_server_id()?;
            writeln!(out, "server id {id}")?;
            Ok(())
        }
        Command::User(cmd) => user(&SqliteRepo::open(&config.data_dir)?, cmd, &mut out),
        Command::Token(cmd) => token(&SqliteRepo::open(&config.data_dir)?, cmd, &mut out),
    }
}

fn serve(config: Config, insecure_public_url: bool) -> CliResult {
    let public_url = config.public_url()?;
    if let Err(e) = config.check_public_url_is_safe() {
        if !insecure_public_url {
            return Err(e.into());
        }
        tracing::warn!("{e}");
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let repo = Arc::new(SqliteRepo::open(&config.data_dir)?);
        let server = Server::bind(&config, repo).await?;
        tracing::info!(
            addr = %server.local_addr()?,
            public_url = %public_url,
            data_dir = %config.data_dir.display(),
            "listening"
        );
        server.run(shutdown_signal()).await?;
        tracing::info!("stopped");
        Ok(())
    })
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
}

fn find_user(repo: &dyn Repo, email: &str) -> Result<User, Box<dyn Error>> {
    repo.user_by_email(email)?
        .ok_or_else(|| format!("no user with email {email}").into())
}

fn time(ms: Option<i64>) -> String {
    ms.map(ids::format_created_at)
        .unwrap_or_else(|| "-".to_owned())
}

fn user(repo: &dyn Repo, cmd: UserCommand, out: &mut impl Write) -> CliResult {
    let now = ids::now_ms();
    match cmd {
        UserCommand::Add {
            email,
            first_name,
            last_name,
            password_stdin,
        } => {
            let (password, generated) = if password_stdin {
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                (line.trim_end_matches(['\r', '\n']).to_owned(), false)
            } else {
                (ids::new_password(), true)
            };
            let new = auth::new_user(&email, &first_name, &last_name, &password)?;
            let user = repo.create_user(&new, now)?;
            writeln!(out, "created user {} {}", user.id, user.email)?;
            if generated {
                writeln!(out, "password: {password}")?;
            }
        }
        UserCommand::List => {
            writeln!(out, "id\temail\tname\tcreated\tdisabled")?;
            for u in repo.list_users()? {
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}",
                    u.id,
                    u.email,
                    format!("{} {}", u.first_name, u.last_name).trim(),
                    time(Some(u.created_at)),
                    time(u.disabled_at)
                )?;
            }
        }
        UserCommand::Disable { email } => {
            let u = find_user(repo, &email)?;
            repo.set_user_disabled(&u.id, true, now)?;
            writeln!(out, "disabled {}", u.email)?;
        }
        UserCommand::Enable { email } => {
            let u = find_user(repo, &email)?;
            repo.set_user_disabled(&u.id, false, now)?;
            writeln!(out, "enabled {}", u.email)?;
        }
        UserCommand::Delete { email, yes } => {
            let u = find_user(repo, &email)?;
            if !yes {
                return Err(format!(
                    "this deletes {} and all of their data; pass --yes to confirm",
                    u.email
                )
                .into());
            }
            repo.delete_user(&u.id)?;
            writeln!(out, "deleted {}", u.email)?;
        }
    }
    Ok(())
}

fn token(repo: &dyn Repo, cmd: TokenCommand, out: &mut impl Write) -> CliResult {
    let now = ids::now_ms();
    match cmd {
        TokenCommand::Create { email, name } => {
            let u = find_user(repo, &email)?;
            if u.disabled_at.is_some() {
                return Err(format!("{} is disabled", u.email).into());
            }
            let token = auth::issue_token(repo, &u, &name, now)?;
            writeln!(out, "{token}")?;
        }
        TokenCommand::List { user } => {
            let user_id = match user {
                Some(email) => Some(find_user(repo, &email)?.id),
                None => None,
            };
            writeln!(out, "id\temail\tname\tcreated\tlast used\trevoked")?;
            for t in repo.list_tokens(user_id.as_deref())? {
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    t.id,
                    t.user_email,
                    t.name,
                    time(Some(t.created_at)),
                    time(t.last_used_at),
                    time(t.revoked_at)
                )?;
            }
        }
        TokenCommand::Revoke { id } => {
            if !repo.revoke_token(id, now)? {
                return Err(format!("no live token with id {id}").into());
            }
            writeln!(out, "revoked token {id}")?;
        }
    }
    Ok(())
}
