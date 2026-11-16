// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Command line: `serve` (the default), `pair`, `devices`.

use std::error::Error;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use clap::parser::ValueSource;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use serde::{Deserialize, Serialize};

use crate::config::{Config, ConfigInput};
use crate::control;
use crate::devices::{DeviceFile, MAX_CODE_TTL_MS};
use crate::http::{now_ms, AppState, Server};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
    name = "airpost-gateway",
    version,
    about = "Authenticated remote access to an Airpost engine for the web client"
)]
pub struct Cli {
    /// The engine's profile directory (default: the desktop's); the gateway keeps its state
    /// in `<dir>/gateway`.
    #[arg(long, env = "AIRPOST_CONFIG_DIR", global = true)]
    config_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    serve: ServeArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the web app and the gateway API (the default).
    Serve(ServeArgs),
    /// Print a one-time code that pairs a browser with this gateway.
    Pair(PairArgs),
    /// List, rename or revoke paired devices.
    #[command(subcommand)]
    Devices(DevicesCommand),
    /// Exit 0 when the gateway at the bind address answers /healthz (for containers without
    /// curl).
    Healthcheck {
        #[arg(long, env = "AIRPOST_GATEWAY_BIND", default_value = "127.0.0.1:5180")]
        bind: SocketAddr,
    },
}

#[derive(Args, Clone)]
struct ServeArgs {
    #[arg(long, env = "AIRPOST_GATEWAY_BIND", default_value = "127.0.0.1:5180")]
    bind: SocketAddr,
    /// Where browsers reach the gateway, e.g. https://airpost.example.com. Defaults to the bind
    /// address when it is loopback.
    #[arg(long, env = "AIRPOST_GATEWAY_PUBLIC_URL")]
    public_url: Option<String>,
    /// Accept a plain-http public URL on a non-loopback host. Cookies then travel unencrypted.
    #[arg(long, env = "AIRPOST_GATEWAY_INSECURE_PUBLIC_URL")]
    insecure_public_url: bool,
    /// Another origin allowed to call the gateway, e.g. a Vite dev server. Repeatable.
    #[arg(
        long = "allow-origin",
        env = "AIRPOST_GATEWAY_ALLOW_ORIGINS",
        value_delimiter = ','
    )]
    allow_origin: Vec<String>,
    /// Proxies whose X-Forwarded-For is believed for rate limits.
    #[arg(
        long,
        env = "AIRPOST_TRUSTED_PROXIES",
        value_delimiter = ',',
        default_value = "127.0.0.0/8,::1"
    )]
    trusted_proxies: Vec<String>,
    /// The built web app (apps/web/dist).
    #[arg(long, env = "AIRPOST_GATEWAY_WEB_ROOT")]
    web_root: Option<PathBuf>,
}

#[derive(Args)]
struct PairArgs {
    /// Grant the admin scope: account, secret and engine methods from this device.
    #[arg(long)]
    admin: bool,
    /// The device's name; otherwise the browser's.
    #[arg(long)]
    name: Option<String>,
    /// How long the code stays valid: 90s, 10m, 1h (at most 1h).
    #[arg(long, default_value = "10m", value_parser = parse_ttl)]
    ttl: i64,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum DevicesCommand {
    List {
        #[arg(long)]
        json: bool,
    },
    /// Revoke a device; its open sessions close at once.
    Revoke {
        id: String,
    },
    Rename {
        id: String,
        name: String,
    },
}

fn parse_ttl(s: &str) -> std::result::Result<i64, String> {
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = n.parse().map_err(|_| format!("{s:?}: expected e.g. 10m"))?;
    let ms = match unit {
        "s" => n * 1000,
        "m" | "" => n * 60_000,
        "h" => n * 3_600_000,
        _ => return Err(format!("{s:?}: use s, m or h")),
    };
    if ms <= 0 || ms > MAX_CODE_TTL_MS {
        return Err(format!("{s:?}: between 1s and 1h"));
    }
    Ok(ms)
}

impl Cli {
    /// Parses argv, exiting on an error as `Parser::parse` does.
    pub fn from_env() -> Cli {
        Cli::try_from_args(std::env::args_os()).unwrap_or_else(|e| e.exit())
    }

    /// Like `Parser::try_parse_from`, and refuses the serve options before a subcommand
    /// (`airpost-gateway --bind X pair`), which that subcommand would ignore. clap's
    /// `args_conflicts_with_subcommands` would do that but also refuses the global
    /// `--config-dir` there, the form `airpost --config-dir X <cmd>` uses.
    pub fn try_from_args<I, T>(args: I) -> std::result::Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let mut cmd = Cli::command();
        let matches = cmd.try_get_matches_from_mut(args)?;
        if let Some(sub) = matches.subcommand_name() {
            let misplaced = cmd.get_arguments().find(|a| {
                !a.is_global_set()
                    && matches.value_source(a.get_id().as_str()) == Some(ValueSource::CommandLine)
            });
            if let Some(arg) = misplaced {
                let name = arg.get_long().unwrap_or(arg.get_id().as_str());
                return Err(cmd.error(
                    clap::error::ErrorKind::ArgumentConflict,
                    format!("--{name} is a serve option; it can't be used with `{sub}`"),
                ));
            }
        }
        Cli::from_arg_matches(&matches).map_err(|e| e.format(&mut cmd))
    }
}

pub fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr())),
        )
        .init();
}

/// `<state dir>/gateway.json`, written by `serve` so `pair` can print the full URL and
/// `airpost status` can show the running gateway and its build.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Running {
    pid: u32,
    public_url: String,
    version: String,
    /// `sha256:<hex>` of the binary, as the engine's `hello.build`.
    #[serde(default)]
    build: Option<String>,
}

pub fn run(cli: Cli) -> Result<()> {
    if let Some(Command::Healthcheck { bind }) = cli.command {
        return healthcheck(bind);
    }
    let config_dir = cli
        .config_dir
        .or_else(airpost_client::discovery::default_config_dir)
        .ok_or("pass --config-dir or set AIRPOST_CONFIG_DIR to the engine's profile directory")?;
    let state_dir = config_dir.join("gateway");
    match cli.command {
        None => serve(cli.serve, config_dir),
        Some(Command::Serve(args)) => serve(args, config_dir),
        Some(Command::Pair(args)) => pair(args, &state_dir),
        Some(Command::Devices(cmd)) => devices(cmd, &state_dir),
        Some(Command::Healthcheck { .. }) => unreachable!("handled above"),
    }
}

/// The launchd `Sockets` key and systemd `FileDescriptorName=` of the HTTP socket in the unit
/// `airpost gateway install` writes.
const ACTIVATION_SOCKET: &str = "http";

fn serve(args: ServeArgs, config_dir: PathBuf) -> Result<()> {
    // Before the runtime's threads: on Linux this unsets LISTEN_FDS and friends.
    let inherited = airpost_activation::tcp_listener(ACTIVATION_SOCKET)?;
    let bind = match &inherited {
        Some(l) => l.local_addr()?,
        None => args.bind,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let config_for = |bind: SocketAddr| {
            Config::new(ConfigInput {
                bind,
                public_url: args.public_url.as_deref(),
                insecure_public_url: args.insecure_public_url,
                allow_origins: &args.allow_origin,
                trusted_proxies: &args.trusted_proxies,
                web_root: args.web_root.clone(),
                config_dir: config_dir.clone(),
            })
        };
        // Validated before anything binds, so a refused config never listens, even briefly.
        let checked = config_for(bind)?;
        // The control socket before the TCP port, so a second gateway on this profile says so
        // rather than failing on the port.
        let control_path = control::socket_path(&checked.state_dir);
        let control_listener = control::bind(&control_path)?;
        // With port 0 the public URL (and so the Host and Origin checks and `pair`'s link)
        // must name the port the system picked, so the final config is built after the bind.
        let listener = match inherited {
            Some(l) => {
                tracing::info!(listen = %bind, "listening on the service manager's socket");
                l.set_nonblocking(true)?;
                tokio::net::TcpListener::from_std(l)?
            }
            None => tokio::net::TcpListener::bind(bind).await?,
        };
        let config = config_for(listener.local_addr()?)?;
        if !config.public.https && !config.public.is_loopback() {
            tracing::warn!("serving over plain http: device cookies travel unencrypted");
        }
        let state = AppState::new(config)?;
        let server = Server::with_listener(listener, state.clone());
        let public_url = state.config().public.as_header();
        let running_path = state.devices().dir().join("gateway.json");
        std::fs::write(
            &running_path,
            serde_json::to_vec(&Running {
                pid: std::process::id(),
                public_url: public_url.clone(),
                version: env!("CARGO_PKG_VERSION").into(),
                build: airpost_activation::build_id(),
            })?,
        )?;
        let reload_state = state.clone();
        tokio::spawn(control::serve(control_listener, move || {
            reload_state.reload().map_err(|e| e.to_string())
        }));
        tracing::info!(
            listen = %server.local_addr()?,
            public_url = %public_url,
            "airpost-gateway serving"
        );
        let out = server.run(shutdown_signal()).await;
        let _ = std::fs::remove_file(&running_path);
        let _ = std::fs::remove_file(&control_path);
        out.map_err(Into::into)
    })
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutting down");
}

fn reload_running(state_dir: &Path) {
    match control::request_reload(&control::socket_path(state_dir)) {
        Ok(true) => {}
        Ok(false) => eprintln!("(no gateway is running; it reads the change when it starts)"),
        Err(e) => eprintln!("warning: couldn't tell the running gateway: {e}"),
    }
}

fn pair(args: PairArgs, state_dir: &Path) -> Result<()> {
    let devices = DeviceFile::open(state_dir)?;
    let now = now_ms();
    let code = devices.mint_code(args.name.as_deref(), args.admin, args.ttl, now)?;
    reload_running(state_dir);
    let url = std::fs::read(state_dir.join("gateway.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Running>(&b).ok())
        .map(|r| format!("{}/pair#{code}", r.public_url));
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "code": code, "url": url, "admin": args.admin, "expiresAt": now + args.ttl,
            })
        );
        return Ok(());
    }
    let minutes = (args.ttl + 59_999) / 60_000;
    let scope = if args.admin { " (admin)" } else { "" };
    println!("Pairing code{scope}: {code}");
    match url {
        Some(url) => println!("Open {url} within {minutes} min."),
        None => println!("Open /pair on the gateway and enter the code within {minutes} min."),
    }
    Ok(())
}

fn devices(cmd: DevicesCommand, state_dir: &Path) -> Result<()> {
    let devices = DeviceFile::open(state_dir)?;
    let now = now_ms();
    match cmd {
        DevicesCommand::List { json } => {
            let list = devices.list(now)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "devices": list.iter().map(|d| serde_json::json!({
                    "id": d.id, "name": d.name, "scopes": d.scopes,
                    "pairedAt": d.paired_at, "lastUsedAt": d.last_used_at,
                })).collect::<Vec<_>>() })
                );
                return Ok(());
            }
            if list.is_empty() {
                println!("No paired devices. Run `airpost-gateway pair` to add one.");
            }
            for d in list {
                let scope = if d.is_admin() { "admin" } else { "-" };
                println!(
                    "{}  {:<5}  paired {}  used {}  {}",
                    d.id,
                    scope,
                    day(d.paired_at),
                    day(d.last_used_at),
                    d.name
                );
            }
        }
        DevicesCommand::Revoke { id } => {
            if !devices.revoke(&id, now)? {
                return Err(format!("no device {id}").into());
            }
            reload_running(state_dir);
            println!("Revoked {id}.");
        }
        DevicesCommand::Rename { id, name } => {
            if !devices.rename(&id, &name, now)? {
                return Err(format!("no device {id}, or an empty name").into());
            }
            reload_running(state_dir);
        }
    }
    Ok(())
}

fn healthcheck(bind: SocketAddr) -> Result<()> {
    use std::io::{Read, Write};
    let mut addr = bind;
    if addr.ip().is_unspecified() {
        addr.set_ip(if addr.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    let timeout = std::time::Duration::from_secs(3);
    let mut s = std::net::TcpStream::connect_timeout(&addr, timeout)?;
    s.set_read_timeout(Some(timeout))?;
    write!(
        s,
        "GET {} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n",
        crate::http::HEALTH_PATH
    )?;
    let mut head = [0u8; 12];
    s.read_exact(&mut head)?;
    if &head[9..12] != b"200" {
        return Err(format!("{addr} answered {}", String::from_utf8_lossy(&head)).into());
    }
    Ok(())
}

fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%d")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_goes_before_or_after_the_subcommand() {
        for args in [
            &["airpost-gateway", "--config-dir", "/p", "pair"][..],
            &["airpost-gateway", "pair", "--config-dir", "/p"],
            &[
                "airpost-gateway",
                "--config-dir",
                "/p",
                "serve",
                "--bind",
                "127.0.0.1:1",
            ],
            &[
                "airpost-gateway",
                "--config-dir",
                "/p",
                "--bind",
                "127.0.0.1:1",
            ],
            &["airpost-gateway", "--config-dir", "/p", "devices", "list"],
        ] {
            let cli = Cli::try_from_args(args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
            assert_eq!(cli.config_dir.as_deref(), Some(Path::new("/p")), "{args:?}");
        }
        let cli = Cli::try_from_args(["airpost-gateway", "--bind", "127.0.0.1:7"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.serve.bind.port(), 7);
        let Some(Command::Serve(args)) =
            Cli::try_from_args(["airpost-gateway", "serve", "--bind", "127.0.0.1:8"])
                .unwrap()
                .command
        else {
            panic!("expected serve");
        };
        assert_eq!(args.bind.port(), 8);
    }

    #[test]
    fn serve_options_before_a_subcommand_are_refused() {
        for args in [
            &["airpost-gateway", "--bind", "127.0.0.1:1", "pair"][..],
            &["airpost-gateway", "--web-root", "/w", "serve"],
            &[
                "airpost-gateway",
                "--insecure-public-url",
                "devices",
                "list",
            ],
        ] {
            let err = Cli::try_from_args(args)
                .err()
                .unwrap_or_else(|| panic!("{args:?} parsed"));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{args:?}"
            );
        }
    }

    #[test]
    fn ttls() {
        assert_eq!(parse_ttl("90s"), Ok(90_000));
        assert_eq!(parse_ttl("10m"), Ok(600_000));
        assert_eq!(parse_ttl("10"), Ok(600_000));
        assert_eq!(parse_ttl("1h"), Ok(3_600_000));
        assert!(parse_ttl("2h").is_err());
        assert!(parse_ttl("0m").is_err());
        assert!(parse_ttl("5d").is_err());
    }
}
