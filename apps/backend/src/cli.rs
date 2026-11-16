// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Command line (docs/backend-plan.md 3.2). Logs go to stderr; command output to stdout.

use std::error::Error;
use std::io::{BufRead, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::auth;
use crate::backup;
use crate::config::{Config, Signup};
use crate::http::Server;
use crate::ids;
use crate::repo::{Repo, SqliteRepo, User, SCHEMA_VERSION};

#[derive(Debug, Parser)]
#[command(
    name = "airpost-backend",
    version,
    about = "Self-hostable Airpost server"
)]
pub struct Cli {
    /// TOML config file. AIRPOST_* environment variables override it (docs/self-hosting.md).
    #[arg(long, global = true, env = "AIRPOST_CONFIG")]
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
    /// Exit 0 if the server answers `GET /ping` on its bind address. For container health
    /// checks, where the image has no shell or curl.
    Healthcheck {
        /// Seconds to wait for the connection and the answer.
        #[arg(long, default_value_t = 5)]
        timeout: u64,
    },
    /// Write a consistent, owner-only copy of the database to PATH (`VACUUM INTO`). The copy
    /// gets a new server id, so engines re-sync from scratch if it is ever restored.
    Backup { path: PathBuf },
    /// Replace the database with the one at PATH (a backup, a pre-migration copy, or a copy
    /// of a stopped server's airpost.db), with a new server id so clients re-sync. The server
    /// must be stopped.
    Restore { path: PathBuf },
    /// Give the database a new server id. Run it, with the server stopped, after restoring
    /// the data dir by other means than `restore` (a filesystem snapshot, say).
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

/// Text logs on stderr, or one JSON object per line with `AIRPOST_LOG_FORMAT=json`. `RUST_LOG`
/// sets the level (default `info`).
pub fn init_logging() {
    use std::io::IsTerminal;
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let format = std::env::var("AIRPOST_LOG_FORMAT").unwrap_or_default();
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    if format == "json" {
        builder.json().init();
    } else {
        builder.with_ansi(std::io::stderr().is_terminal()).init();
        if !format.is_empty() && format != "text" {
            tracing::warn!("AIRPOST_LOG_FORMAT={format:?} is not \"text\" or \"json\"; using text");
        }
    }
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
        Command::Healthcheck { timeout } => {
            let addr = healthcheck(&config, Duration::from_secs(timeout.max(1)))?;
            writeln!(out, "ok {addr}")?;
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
        Command::Restore { path } => {
            let _lock = SqliteRepo::lock_data_dir(&config.data_dir)?;
            let id = SqliteRepo::restore(&config.data_dir, &path)?;
            writeln!(
                out,
                "restored {} as {} (server id {id}); clients re-sync their metadata",
                path.display(),
                config.db_path().display()
            )?;
            Ok(())
        }
        Command::RotateServerId => {
            let _lock = SqliteRepo::lock_data_dir(&config.data_dir)?;
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
        let _lock = SqliteRepo::lock_data_dir(&config.data_dir)?;
        let repo: Arc<dyn Repo> = Arc::new(SqliteRepo::open(&config.data_dir)?);
        if config.signup == Signup::Closed && repo.list_users()?.is_empty() {
            tracing::warn!(
                "no users yet and signup is closed; create one with `airpost-backend user add <email>`"
            );
        }
        let server = Server::bind(&config, repo.clone()).await?;
        tracing::info!(
            addr = %server.local_addr()?,
            public_url = %public_url,
            data_dir = %config.data_dir.display(),
            "listening"
        );
        let backups = config.backup.clone().map(|schedule| {
            tracing::info!(
                dir = %schedule.dir.display(),
                every_secs = schedule.interval.as_secs(),
                keep = schedule.keep,
                "scheduled backups on"
            );
            tokio::spawn(backup::run(repo, schedule))
        });
        server.run(shutdown_signal()).await?;
        if let Some(task) = backups {
            task.abort();
        }
        tracing::info!("stopped");
        Ok(())
    })
}

/// `GET /ping` on the bind address (loopback when it binds every interface). Returns the
/// address that answered.
fn healthcheck(config: &Config, timeout: Duration) -> Result<std::net::SocketAddr, Box<dyn Error>> {
    let mut addr = config.bind;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr.ip() {
            IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
        });
    }
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("connecting to {addr}: {e}"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(
        stream,
        "GET /ping HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let mut status = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut status)
        .map_err(|e| format!("reading from {addr}: {e}"))?;
    if status.split_whitespace().nth(1) == Some("200") {
        Ok(addr)
    } else {
        Err(format!("unexpected answer from {addr}: {:?}", status.trim_end()).into())
    }
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
    ms.map(ids::format_time).unwrap_or_else(|| "-".to_owned())
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
