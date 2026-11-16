// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Airpost's own programs as user units, the same for the desktop, agents and headless machines:
//! `airpost <engine|gateway|mcp-http> install|uninstall|start|stop|restart`, the `services` of
//! `airpost status`, `airpost logs <service>`, `airpost gateway pair` and `airpost uninstall`.
//!
//! - The engine starts at login and restarts after a failed exit. It is not socket-activated:
//!   it syncs mail with no client connected, and the desktop stops a detached engine it can't
//!   join and binds the same socket path itself, which a socket the manager held would answer
//!   (and start the unit's engine again on).
//! - The gateway and `airpost-mcp http` start on demand: launchd `Sockets` or a systemd `.socket`
//!   unit holds their TCP port and starts them on the first connection; they take the socket
//!   with `airpost_activation`. `start` runs them now; they stay running until stopped.
//!
//! The binaries are the ones beside `airpost` (the app bundle or a package install), or
//! `AIRPOST_ENGINE_BIN`, `AIRPOST_GATEWAY_BIN`, `AIRPOST_MCP_BIN`. A unit runs one binary by path and
//! serves one profile, both written in it, and is named after the profile ([`units::Names`]),
//! so two copies of Airpost (the desktop's and a package manager's) share one set of units per
//! profile. A unit running another copy's binary belongs to that copy: `install` replaces it
//! only with `--replace`, and `airpost status` names the owner.

use std::fs;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::Ctx;
use crate::discovery::Engine;
use crate::error::{CliError, Result};
use crate::output::{self, Table};
use crate::service::{
    self, load_note, print, s, Loader, Place, PlaceArgs, Records, System, UnitRecord, UnitRef,
};
use crate::units::{self, Manager, SocketSpec, UnitSpec};

pub const DEFAULT_GATEWAY_BIND: &str = "127.0.0.1:5180";
/// airpost-mcp's `DEFAULT_HTTP_PORT`, policy file and its override (apps/mcp/src/policy.rs).
const DEFAULT_MCP_PORT: u16 = 2587;
const MCP_POLICY_FILE: &str = "mcp.json";
const MCP_POLICY_ENV: &str = "AIRPOST_MCP_POLICY";
/// What `airpost-mcp http` writes while it serves (apps/mcp/src/http.rs `RUNNING_FILE`).
const MCP_RUNNING_FILE: &str = "mcp-http.json";
/// What `airpost-gateway serve` writes in `<profile>/gateway` while it serves.
const GATEWAY_RUNNING_FILE: &str = "gateway.json";
/// The serve engine's own log (`airpost_core::constants::ENGINE_LOG_FILE`).
const ENGINE_LOG_FILE: &str = "engine.log";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Engine,
    Gateway,
    McpHttp,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Engine, Kind::Gateway, Kind::McpHttp];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Engine => "engine",
            Kind::Gateway => "gateway",
            Kind::McpHttp => "mcp-http",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    fn bin_name(self) -> &'static str {
        match self {
            Kind::Engine => "airpost-engine",
            Kind::Gateway => "airpost-gateway",
            Kind::McpHttp => "airpost-mcp",
        }
    }

    fn bin_env(self) -> &'static str {
        match self {
            Kind::Engine => "AIRPOST_ENGINE_BIN",
            Kind::Gateway => "AIRPOST_GATEWAY_BIN",
            Kind::McpHttp => "AIRPOST_MCP_BIN",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Kind::Engine => "Airpost mail engine",
            Kind::Gateway => "Airpost gateway (web access)",
            Kind::McpHttp => "Airpost MCP server over HTTP",
        }
    }

    /// The variable the unit names its profile in.
    fn profile_env(self) -> &'static str {
        match self {
            Kind::Engine => "CONFIG_DIR_PATH",
            Kind::Gateway | Kind::McpHttp => "AIRPOST_CONFIG_DIR",
        }
    }

    /// Started by its socket on the first connection.
    pub fn on_demand(self) -> bool {
        self != Kind::Engine
    }

    pub fn unit_name(self, place: &Place) -> String {
        match self {
            Kind::Engine => place.names.engine(place.manager),
            Kind::Gateway => place.names.gateway(place.manager),
            Kind::McpHttp => place.names.mcp_http(place.manager),
        }
    }

    pub fn unit_ref(self, place: &Place) -> UnitRef {
        place.unit(&self.unit_name(place), self.on_demand())
    }

    /// launchd's stdout and stderr for the unit, in the profile; systemd's go to the journal.
    pub fn launchd_log(self, config_dir: &Path) -> PathBuf {
        config_dir.join(format!("launchd-{}.log", self.as_str()))
    }

    pub fn record(self, r: &Records) -> Option<&UnitRecord> {
        match self {
            Kind::Engine => r.engine.as_ref(),
            Kind::Gateway => r.gateway.as_ref(),
            Kind::McpHttp => r.mcp_http.as_ref(),
        }
    }

    fn slot(self, r: &mut Records) -> &mut Option<UnitRecord> {
        match self {
            Kind::Engine => &mut r.engine,
            Kind::Gateway => &mut r.gateway,
            Kind::McpHttp => &mut r.mcp_http,
        }
    }
}

/// This `airpost` as it was found: the path it was run by (argv[0], or its entry on PATH), not
/// resolved through links. A package manager's stable link (`/opt/homebrew/bin/airpost`, a Nix
/// profile) then names it and the binaries beside it, rather than the versioned file it points
/// to, which an upgrade removes. Units keep that path; copies are compared canonically.
pub fn this_airpost() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let found = std::env::args_os()
        .next()
        .and_then(|a| found_on_path(Path::new(&a)))
        .filter(|f| same_file(f, &exe));
    Some(found.unwrap_or(exe))
}

fn found_on_path(argv0: &Path) -> Option<PathBuf> {
    if argv0.components().count() > 1 {
        return std::path::absolute(argv0).ok().filter(|p| p.is_file());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter_map(|d| std::path::absolute(d.join(argv0)).ok())
        .find(|p| p.is_file())
}

/// Whether two paths name the same file.
fn same_file(a: &Path, b: &Path) -> bool {
    canonical(a) == canonical(b)
}

/// The binary this copy of `airpost` runs for `kind`: `$AIRPOST_*_BIN`, else the one beside `airpost`.
pub fn default_bin(kind: Kind) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(kind.bin_env()).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let p = this_airpost()?.parent()?.join(format!(
        "{}{}",
        kind.bin_name(),
        std::env::consts::EXE_SUFFIX
    ));
    p.is_file().then_some(p)
}

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_owned())
}

/// The binary the installed unit runs, read from the unit itself.
pub fn installed_bin(kind: Kind, place: &Place) -> Option<PathBuf> {
    let text = fs::read_to_string(place.path(&kind.unit_name(place))).ok()?;
    units::read_program(place.manager, &text).map(PathBuf::from)
}

// -- Plans ----------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub bin: Option<PathBuf>,
    /// The engine's `AIRPOST_ENGINE_KEY_FILE`.
    pub key_file: Option<PathBuf>,
    /// The gateway's address (default [`DEFAULT_GATEWAY_BIND`]).
    pub bind: Option<SocketAddr>,
    pub web_root: Option<PathBuf>,
    pub public_url: Option<String>,
    /// airpost-mcp's port (default `http.port` in mcp.json).
    pub port: Option<u16>,
    /// Take over a unit that runs another copy's binary.
    pub replace: bool,
    pub dry_run: bool,
}

/// What an install writes.
#[derive(Clone, Debug)]
pub struct Plan {
    pub kind: Kind,
    pub bin: PathBuf,
    pub sha256: String,
    pub name: String,
    pub unit: PathBuf,
    pub text: String,
    /// The systemd `.socket` unit's path and text.
    pub socket_unit: Option<(PathBuf, String)>,
    pub addr: Option<SocketAddr>,
    pub url: Option<String>,
}

impl Plan {
    /// The engine unit a service plugin's install writes when there is none.
    pub fn engine(
        config_dir: &Path,
        place: &Place,
        bin: Option<&Path>,
        key_file: Option<&Path>,
    ) -> Result<Plan> {
        let opts = Options {
            bin: bin.map(Path::to_owned),
            key_file: key_file.map(Path::to_owned),
            ..Options::default()
        };
        Plan::new(Kind::Engine, config_dir, place, &opts)
    }

    pub fn new(kind: Kind, config_dir: &Path, place: &Place, opts: &Options) -> Result<Plan> {
        let bin = match &opts.bin {
            Some(b) => service::absolute(b)?,
            None => default_bin(kind).ok_or_else(|| {
                CliError::usage(format!(
                    "no {} beside airpost; give --bin or set {}",
                    kind.bin_name(),
                    kind.bin_env()
                ))
            })?,
        };
        // Kept as found, links included: see [`this_airpost`].
        if !bin.is_file() {
            return Err(CliError::failed(format!(
                "the {} binary {} is not there",
                kind.as_str(),
                bin.display()
            )));
        }
        let sha256 = service::sha256_file(&bin)
            .map_err(|e| CliError::failed(format!("{}: {e}", bin.display())))?;
        let mut env = vec![(
            kind.profile_env().to_owned(),
            config_dir.to_string_lossy().into_owned(),
        )];
        let mut program = vec![bin.to_string_lossy().into_owned()];
        let mut socket = None;
        let mut url = None;
        match kind {
            Kind::Engine => {
                program.extend(["--mode", "serve", "--detached"].map(String::from));
                if let Some(k) = &opts.key_file {
                    env.push((
                        "AIRPOST_ENGINE_KEY_FILE".to_owned(),
                        service::absolute(k)?.to_string_lossy().into_owned(),
                    ));
                }
            }
            Kind::Gateway => {
                let addr = match opts.bind {
                    Some(a) => a,
                    None => DEFAULT_GATEWAY_BIND.parse().expect("a socket address"),
                };
                fixed_port(addr)?;
                program.extend(["serve".into(), "--bind".into(), addr.to_string()]);
                let web_root = match opts
                    .web_root
                    .as_deref()
                    .filter(|d| !d.as_os_str().is_empty())
                {
                    Some(d) => Some(web_root_dir(d)?),
                    None => default_web_root(&bin),
                };
                if let Some(d) = web_root {
                    program.extend(["--web-root".into(), d.to_string_lossy().into_owned()]);
                }
                if let Some(u) = &opts.public_url {
                    program.extend(["--public-url".into(), u.clone()]);
                }
                url = Some(match &opts.public_url {
                    Some(u) => u.trim_end_matches('/').to_owned(),
                    None => local_url(addr, ""),
                });
                socket = Some(SocketSpec {
                    name: "http".into(),
                    addr,
                });
            }
            Kind::McpHttp => {
                let policy = McpHttpPolicy::load(config_dir)?;
                if !policy.enabled {
                    return Err(CliError::failed(format!(
                        "HTTP is off for airpost-mcp; set \"http\": {{\"enabled\": true}} in {} \
                         (Preferences > MCP) first",
                        policy.path.display()
                    )));
                }
                let addr = SocketAddr::from(([127, 0, 0, 1], opts.port.unwrap_or(policy.port)));
                fixed_port(addr)?;
                program.extend(["http".into(), "--port".into(), addr.port().to_string()]);
                if let Some(p) = std::env::var_os(MCP_POLICY_ENV).filter(|v| !v.is_empty()) {
                    env.push((MCP_POLICY_ENV.to_owned(), p.to_string_lossy().into_owned()));
                }
                url = Some(local_url(addr, "/mcp"));
                socket = Some(SocketSpec {
                    name: "mcp".into(),
                    addr,
                });
            }
        }
        let name = kind.unit_name(place);
        let spec = UnitSpec {
            name: name.clone(),
            description: kind.description().to_owned(),
            program,
            env,
            after: None,
            log: Some(kind.launchd_log(config_dir)),
            socket,
        };
        let text = units::render(place.manager, &spec).map_err(CliError::failed)?;
        let socket_unit = units::render_socket(place.manager, &spec)
            .and_then(|t| place.socket_path(&name).map(|p| (p, t)));
        Ok(Plan {
            kind,
            bin,
            sha256,
            unit: place.path(&name),
            name,
            text,
            socket_unit,
            addr: spec.socket.as_ref().map(|s| s.addr),
            url,
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "service": self.kind.as_str(),
            "installed": false,
            "bin": self.bin,
            "sha256": self.sha256,
            "name": self.name,
            "path": self.unit,
            "text": self.text,
            "socketUnit": self.socket_unit.as_ref().map(|(p, _)| p),
            "socketText": self.socket_unit.as_ref().map(|(_, t)| t),
            "url": self.url,
            "port": self.addr.map(|a| a.port()),
            "onDemand": self.kind.on_demand(),
        })
    }

    pub fn apply(&self, config_dir: &Path, place: &Place, loader: &mut dyn Loader) -> Result<()> {
        service::write_atomic(&self.unit, self.text.as_bytes(), false)?;
        if let Some((path, text)) = &self.socket_unit {
            service::write_atomic(path, text.as_bytes(), false)?;
        }
        let mut records = Records::load(config_dir)?;
        *self.kind.slot(&mut records) = Some(UnitRecord {
            bin: self.bin.clone(),
            sha256: self.sha256.clone(),
            manager: place.manager.as_str().to_owned(),
            name: self.name.clone(),
            unit: self.unit.clone(),
            socket_unit: self.socket_unit.as_ref().map(|(p, _)| p.clone()),
            profile: Some(config_dir.to_owned()),
            installed_by: this_airpost(),
            url: self.url.clone(),
            port: self.addr.map(|a| a.port()),
            installed_at: service::now(),
        });
        records.save(config_dir)?;
        if place.load {
            let u = self.kind.unit_ref(place);
            if place.no_start {
                loader.enable(&u)?;
            } else {
                loader.load(&u)?;
            }
        }
        Ok(())
    }
}

fn fixed_port(addr: SocketAddr) -> Result<()> {
    if addr.port() == 0 {
        return Err(CliError::usage(format!(
            "{addr}: give a port; the unit's socket can't use port 0"
        )));
    }
    Ok(())
}

/// `http://<addr><path>`, naming loopback for an unspecified address.
fn local_url(addr: SocketAddr, path: &str) -> String {
    let mut a = addr;
    if a.ip().is_unspecified() {
        a.set_ip(if a.is_ipv4() {
            [127, 0, 0, 1].into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    format!("http://{a}{path}")
}

fn web_root_dir(d: &Path) -> Result<PathBuf> {
    let d = service::absolute(d)?;
    if !d.is_dir() {
        return Err(CliError::failed(format!(
            "the web root {} is not a directory",
            d.display()
        )));
    }
    Ok(d)
}

/// The built web client beside the gateway binary: `$AIRPOST_GATEWAY_WEB_ROOT`, a package's
/// `../share/airpost-gateway/web` (packaging/archive.sh), or the desktop bundle's `airpost-web`.
fn default_web_root(bin: &Path) -> Option<PathBuf> {
    let dir = bin.parent()?;
    let env = std::env::var_os("AIRPOST_GATEWAY_WEB_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let package = dir
        .parent()
        .map(|prefix| prefix.join("share").join("airpost-gateway").join("web"));
    env.into_iter()
        .chain(package)
        .chain([dir.join("airpost-web")])
        .find(|d| d.join("index.html").is_file())
}

/// `http` in airpost-mcp's policy file: what `airpost-mcp http` checks before it serves.
struct McpHttpPolicy {
    path: PathBuf,
    enabled: bool,
    port: u16,
}

impl McpHttpPolicy {
    fn load(config_dir: &Path) -> Result<McpHttpPolicy> {
        let path = std::env::var_os(MCP_POLICY_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join(MCP_POLICY_FILE));
        let http = match fs::read(&path) {
            Ok(bytes) => {
                let v: Value = serde_json::from_slice(&bytes).map_err(|e| {
                    CliError::failed(format!("{} is not valid JSON: {e}", path.display()))
                })?;
                v["http"].clone()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
            Err(e) => return Err(CliError::failed(format!("{}: {e}", path.display()))),
        };
        let port = http["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(DEFAULT_MCP_PORT);
        Ok(McpHttpPolicy {
            enabled: http["enabled"] == true,
            port,
            path,
        })
    }
}

// -- Install, uninstall, start, stop --------------------------------------------------------

pub fn install(
    kind: Kind,
    config_dir: &Path,
    place: &Place,
    opts: &Options,
    loader: &mut dyn Loader,
) -> Result<Value> {
    let mut place = place.clone();
    // The engine the desktop runs holds the socket: the unit's engine would exit at once and
    // be restarted every few seconds. It starts when the desktop quits (`airpost engine start`).
    let attached = kind == Kind::Engine && attached_engine(config_dir).is_some();
    place.no_start |= attached;
    let plan = Plan::new(kind, config_dir, &place, opts)?;
    let mut out = plan.to_json();
    out["manager"] = json!(place.manager.as_str());
    out["load"] = json!(place.load);
    out["started"] = json!(place.load && !place.no_start);
    if let Some(theirs) = installed_bin(kind, &place).filter(|b| !same_file(b, &plan.bin)) {
        out["replaces"] = json!(theirs);
        if !opts.replace {
            return Err(CliError::failed(format!(
                "the {kind} unit runs {}, not {} as this airpost ({}) would: it belongs to another \
                 copy of Airpost. Give --replace to switch it to this copy",
                theirs.display(),
                plan.bin.display(),
                this_airpost().unwrap_or_default().display(),
                kind = kind.as_str()
            )));
        }
    }
    if opts.dry_run {
        return Ok(out);
    }
    if let Some(addr) = plan.addr.filter(|_| place.load && !plan.unit.exists()) {
        check_free(addr)?;
    }
    plan.apply(config_dir, &place, loader)?;
    out["installed"] = json!(true);
    Ok(out)
}

/// launchd and systemd bind the socket after `airpost` returns and only log a failure, so a
/// port taken by another program is refused here.
fn check_free(addr: SocketAddr) -> Result<()> {
    match std::net::TcpListener::bind(addr) {
        Ok(_) => Ok(()),
        Err(e) => Err(CliError::failed(format!(
            "can't listen on {addr} ({e}); is something else using it?"
        ))),
    }
}

/// The pid of a live engine for this profile that isn't detached: the desktop's own.
fn attached_engine(config_dir: &Path) -> Option<u32> {
    let file = config_dir
        .join("run")
        .join(airpost_client::discovery::ENGINE_FILE);
    let v: Value = serde_json::from_slice(&fs::read(file).ok()?).ok()?;
    let pid = u32::try_from(v["pid"].as_u64()?).ok()?;
    (v["detached"] != true && airpost_client::pid_alive(pid)).then_some(pid)
}

pub fn uninstall(
    kind: Kind,
    config_dir: &Path,
    place: &Place,
    loader: &mut dyn Loader,
) -> Result<Value> {
    let mut records = Records::load(config_dir)?;
    if kind == Kind::Engine && !records.services.is_empty() {
        let names: Vec<&str> = records.services.keys().map(String::as_str).collect();
        return Err(CliError::failed(format!(
            "these services run on the engine unit; uninstall them first: {}",
            names.join(", ")
        )));
    }
    let u = kind.unit_ref(place);
    if !u.unit.exists() && kind.record(&records).is_none() {
        return Err(CliError::failed(format!(
            "the {} unit is not installed",
            kind.as_str()
        )));
    }
    if place.load {
        loader.unload(&u)?;
    }
    service::remove_file(&u.unit)?;
    if let Some(socket) = place.socket_path(&u.name) {
        service::remove_file(&socket)?;
    }
    service::reload_after_remove(place);
    *kind.slot(&mut records) = None;
    records.save(config_dir)?;
    Ok(json!({"service": kind.as_str(), "name": u.name, "path": u.unit, "removed": true}))
}

fn installed_unit(kind: Kind, place: &Place) -> Result<UnitRef> {
    let u = kind.unit_ref(place);
    if !u.unit.exists() {
        return Err(CliError::failed(format!(
            "the {} unit is not installed ({} is missing)",
            kind.as_str(),
            u.unit.display()
        )));
    }
    Ok(u)
}

/// Starts the installed unit's program, or restarts it so it runs the binary now at its path
/// (the desktop does this after an update instead of stopping the unit's engine, which would
/// then stay stopped: the engine unit restarts only after a failed exit).
pub fn start(kind: Kind, place: &Place, restart: bool, loader: &mut dyn Loader) -> Result<Value> {
    let u = installed_unit(kind, place)?;
    if place.load {
        loader.start(&u, restart)?;
    }
    Ok(json!({
        "service": kind.as_str(), "name": u.name, "path": u.unit, "load": place.load,
        "restarted": restart,
    }))
}

pub fn stop(kind: Kind, place: &Place, loader: &mut dyn Loader) -> Result<Value> {
    let u = installed_unit(kind, place)?;
    if place.load {
        loader.stop(&u)?;
    }
    Ok(json!({
        "service": kind.as_str(), "name": u.name, "path": u.unit, "load": place.load,
        "stopped": place.load,
    }))
}

// -- Commands -------------------------------------------------------------------------------

pub enum Action {
    Install(Options),
    Uninstall,
    Start,
    Stop,
    Restart,
}

pub fn command(ctx: &Ctx, kind: Kind, place: &PlaceArgs, action: Action) -> Result<()> {
    let place = Place::resolve(&ctx.config_dir, place)?;
    let not_loaded = "--unit-dir, --no-load or AIRPOST_SERVICE_NO_LOAD";
    match action {
        Action::Install(opts) => {
            let v = install(kind, &ctx.config_dir, &place, &opts, &mut System)?;
            let mut text = vec![
                format!("Binary:   {}", s(&v["bin"])),
                format!("SHA-256:  {}", s(&v["sha256"])),
            ];
            if let Some(url) = v["url"].as_str() {
                text.push(format!("URL:      {url}"));
            }
            if let Some(theirs) = v["replaces"].as_str() {
                text.push(format!("Replaces: {theirs}"));
            }
            text.push(format!("Unit {}:", s(&v["path"])));
            text.push(s(&v["text"]));
            if let Some(path) = v["socketUnit"].as_str() {
                text.push(format!("Socket unit {path}:"));
                text.push(s(&v["socketText"]));
            }
            text.push(if opts.dry_run {
                "Dry run: nothing written.".to_owned()
            } else if v["load"] == true && v["started"] == false {
                format!(
                    "Installed the {} unit; it starts at login or with `airpost {} start`",
                    kind.as_str(),
                    kind.as_str()
                )
            } else if v["load"] == true && kind.on_demand() {
                format!(
                    "Installed the {} unit; it starts on the first connection",
                    kind.as_str()
                )
            } else {
                load_note(&v, &format!("Installed the {} unit", kind.as_str()))
            });
            print(ctx, &v, &text);
        }
        Action::Uninstall => {
            let v = uninstall(kind, &ctx.config_dir, &place, &mut System)?;
            print(ctx, &v, &[format!("Removed {}", s(&v["path"]))]);
        }
        Action::Start | Action::Restart => {
            let restart = matches!(action, Action::Restart);
            let v = start(kind, &place, restart, &mut System)?;
            let line = if place.load {
                format!(
                    "{} {}",
                    if restart { "Restarted" } else { "Started" },
                    s(&v["name"])
                )
            } else {
                format!("Not started {}: {not_loaded}", s(&v["name"]))
            };
            print(ctx, &v, &[line]);
        }
        Action::Stop => {
            let v = stop(kind, &place, &mut System)?;
            let line = if place.load {
                format!("Stopped {}", s(&v["name"]))
            } else {
                format!("Not stopped {}: {not_loaded}", s(&v["name"]))
            };
            print(ctx, &v, &[line]);
        }
    }
    Ok(())
}

// -- Status ---------------------------------------------------------------------------------

/// A running service, from what it reports about itself.
#[derive(Debug, Default)]
struct Live {
    pid: u32,
    url: Option<String>,
    port: Option<u16>,
    /// `sha256:<hex>` of the binary it runs.
    build: Option<String>,
}

fn live(kind: Kind, config_dir: &Path, engines: &[Engine]) -> Option<Live> {
    let is_serve = |f: &Path| {
        f.file_name()
            .is_some_and(|n| n == airpost_client::discovery::ENGINE_FILE)
    };
    let from_file = |path: PathBuf, url_key: &str| -> Option<Live> {
        let v: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        let pid = u32::try_from(v["pid"].as_u64()?).ok()?;
        if !airpost_client::pid_alive(pid) {
            return None;
        }
        let url = v[url_key].as_str().map(str::to_owned);
        let port = v["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .or_else(|| url.as_deref().and_then(url_port));
        Some(Live {
            pid,
            url,
            port,
            build: v["build"].as_str().map(str::to_owned),
        })
    };
    match kind {
        Kind::Engine => engines
            .iter()
            .find(|e| is_serve(&e.file))
            .map(|e| Live {
                pid: e.entry.pid,
                build: e.hello.build.clone(),
                ..Live::default()
            })
            .or_else(|| {
                let scan = airpost_client::discovery::scan(&config_dir.join("run"));
                let (_, entry) = scan.live.into_iter().find(|(f, _)| is_serve(f))?;
                Some(Live {
                    pid: entry.pid,
                    ..Live::default()
                })
            }),
        Kind::Gateway => from_file(
            config_dir.join("gateway").join(GATEWAY_RUNNING_FILE),
            "publicUrl",
        ),
        Kind::McpHttp => from_file(config_dir.join(MCP_RUNNING_FILE), "url"),
    }
}

fn url_port(url: &str) -> Option<u16> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split('/').next()?;
    host.rsplit_once(':')?.1.parse().ok()
}

/// The place `airpost status` reads units from: the manager's directory, or `$AIRPOST_UNIT_DIR`.
pub fn status_place(config_dir: &Path) -> Option<Place> {
    let args = PlaceArgs {
        unit_dir: std::env::var_os("AIRPOST_UNIT_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        ..PlaceArgs::default()
    };
    Place::resolve(config_dir, &args).ok()
}

/// `airpost status`'s `services`: each of Airpost's units, installed or not, and its process.
pub fn status_rows(config_dir: &Path, place: Option<&Place>, engines: &[Engine]) -> Vec<Value> {
    let records = Records::load(config_dir).unwrap_or_default();
    Kind::ALL
        .into_iter()
        .map(|kind| status_row(kind, config_dir, place, &records, engines))
        .collect()
}

fn status_row(
    kind: Kind,
    config_dir: &Path,
    place: Option<&Place>,
    records: &Records,
    engines: &[Engine],
) -> Value {
    let record = kind.record(records);
    let unit = place.map(|p| kind.unit_ref(p));
    let text = unit.as_ref().and_then(|u| fs::read_to_string(&u.unit).ok());
    let installed = text.is_some();
    let (unit_bin, unit_profile) = match (place, &text) {
        (Some(p), Some(t)) => (
            units::read_program(p.manager, t).map(PathBuf::from),
            units::read_env(p.manager, t, kind.profile_env()).map(PathBuf::from),
        ),
        _ => (None, None),
    };
    let ours = default_bin(kind);
    let owner = unit_bin.as_ref().map(|b| {
        if ours.as_deref().is_some_and(|o| same_file(b, o)) {
            "this"
        } else {
            "other"
        }
    });
    let live = live(kind, config_dir, engines);
    let (url, port) = match (&live, record) {
        (Some(l), _) => (l.url.clone(), l.port),
        (None, Some(r)) if installed => (r.url.clone(), r.port),
        _ => (None, None),
    };

    let mut warnings = Vec::new();
    if let (Some(l), Some(ours)) = (&live, &ours) {
        let skewed = match (&l.build, service::sha256_file(ours)) {
            (Some(running), Ok(hex)) => running.strip_prefix("sha256:") != Some(hex.as_str()),
            _ => false,
        };
        if skewed && owner == Some("this") {
            warnings.push(format!(
                "the running {k} (pid {}) is another build than {}; `airpost {k} restart` runs it",
                l.pid,
                ours.display(),
                k = kind.as_str()
            ));
        } else if skewed {
            warnings.push(format!(
                "the running {} (pid {}) is another build than {} beside this airpost",
                kind.as_str(),
                l.pid,
                ours.display()
            ));
        }
    }
    if let Some(p) = unit_profile.filter(|p| p != config_dir) {
        warnings.push(format!(
            "the {} unit serves the profile {}, not {}",
            kind.as_str(),
            p.display(),
            config_dir.display()
        ));
    }
    if let Some(r) = record.filter(|_| !installed) {
        warnings.push(format!(
            "services.json records a {} unit at {}, but it isn't there",
            kind.as_str(),
            r.unit.display()
        ));
    }
    json!({
        "name": kind.as_str(),
        "installed": installed,
        "running": live.is_some(),
        "pid": live.as_ref().map(|l| l.pid),
        "url": url,
        "port": port,
        "unit": unit.filter(|_| installed).map(|u| u.unit),
        "bin": unit_bin.or(ours),
        "onDemand": installed && kind.on_demand(),
        "owner": owner,
        "build": live.and_then(|l| l.build),
        "warnings": warnings,
    })
}

/// The services as a table, then their warnings on stderr.
pub fn print_status_rows(ctx: &Ctx, rows: &[Value]) {
    let mut table = Table::new(vec!["SERVICE", "STATE", "PID", "URL", "BINARY"], 4);
    for r in rows {
        let state = match (r["running"] == true, r["installed"] == true) {
            (true, _) => "running",
            (false, true) if r["onDemand"] == true => "on demand",
            (false, true) => "stopped",
            (false, false) => "not installed",
        };
        let bin = r["bin"].as_str().unwrap_or("-");
        let binary = match r["owner"].as_str() {
            Some("other") => format!("{bin} (another copy)"),
            Some(_) => bin.to_owned(),
            None => "-".to_owned(),
        };
        let cell = |v: &Value| match v {
            Value::Null => "-".to_owned(),
            v => s(v),
        };
        table.row(
            vec![
                s(&r["name"]),
                state.to_owned(),
                cell(&r["pid"]),
                cell(&r["url"]),
                binary,
            ],
            false,
        );
        if r["warnings"].as_array().is_some_and(|w| !w.is_empty()) {
            table.alert(0);
        }
    }
    table.print(&ctx.style);
    for w in rows
        .iter()
        .flat_map(|r| r["warnings"].as_array().cloned().unwrap_or_default())
    {
        output::warn(&s(&w));
    }
}

// -- Logs -----------------------------------------------------------------------------------

/// `airpost logs engine|gateway|mcp-http`: the serve engine's log; the gateway's and airpost-mcp's
/// launchd log file, or their unit's journal.
pub fn logs(ctx: &Ctx, kind: Kind, follow: bool, lines: usize) -> Result<()> {
    if kind == Kind::Engine {
        let path = ctx.config_dir.join(ENGINE_LOG_FILE);
        return crate::logs::run_file(&path, follow, lines, ctx.json);
    }
    let place = Place::resolve(&ctx.config_dir, &PlaceArgs::default())?;
    match place.manager {
        Manager::Launchd => {
            let path = kind.launchd_log(&ctx.config_dir);
            if !path.is_file() {
                return Err(CliError::failed(format!(
                    "no {} log at {}: its unit hasn't run yet",
                    kind.as_str(),
                    path.display()
                )));
            }
            crate::logs::run_file(&path, follow, lines, ctx.json)
        }
        Manager::Systemd => journal(&kind.unit_name(&place), follow, lines, ctx.json),
    }
}

fn journal(name: &str, follow: bool, lines: usize, json_out: bool) -> Result<()> {
    let mut cmd = Command::new("journalctl");
    cmd.args([
        "--user",
        "--no-pager",
        "-o",
        "cat",
        "-n",
        &lines.to_string(),
    ])
    .args([
        "-u",
        &format!("{name}.service"),
        "-u",
        &format!("{name}.socket"),
    ]);
    if follow {
        cmd.arg("-f");
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| CliError::failed(format!("could not run journalctl: {e}")))?;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(std::io::Result::ok) {
            if json_out {
                output::json(&json!({ "line": line }));
            } else {
                output::line(&line);
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| CliError::failed(format!("journalctl: {e}")))?;
    if !status.success() {
        return Err(CliError::failed(format!("journalctl exited with {status}")));
    }
    Ok(())
}

// -- Pairing --------------------------------------------------------------------------------

pub struct PairArgs<'a> {
    pub admin: bool,
    pub name: Option<&'a str>,
    pub ttl: Option<&'a str>,
}

/// `airpost gateway pair`: `airpost-gateway pair` for this profile, with the installed unit's URL
/// when the gateway hasn't started yet (its first connection starts it).
pub fn pair(ctx: &Ctx, args: &PairArgs) -> Result<()> {
    let place = status_place(&ctx.config_dir);
    let bin = place
        .as_ref()
        .and_then(|p| installed_bin(Kind::Gateway, p))
        .or_else(|| default_bin(Kind::Gateway))
        .ok_or_else(|| {
            CliError::usage(
                "no airpost-gateway beside airpost; install it or set AIRPOST_GATEWAY_BIN",
            )
        })?;
    let mut cmd = Command::new(&bin);
    cmd.arg("--config-dir")
        .arg(&ctx.config_dir)
        .args(["pair", "--json"]);
    if args.admin {
        cmd.arg("--admin");
    }
    if let Some(n) = args.name {
        cmd.args(["--name", n]);
    }
    if let Some(t) = args.ttl {
        cmd.args(["--ttl", t]);
    }
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| CliError::failed(format!("could not run {}: {e}", bin.display())))?;
    if !out.status.success() {
        return Err(CliError::failed(format!(
            "{} pair failed ({})",
            bin.display(),
            out.status
        )));
    }
    let mut v: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| CliError::failed(format!("{} pair printed no JSON: {e}", bin.display())))?;
    if v["url"].is_null() {
        let records = Records::load(&ctx.config_dir).unwrap_or_default();
        let base = records.gateway.and_then(|r| r.url);
        if let (Some(base), Some(code)) = (base, v["code"].as_str()) {
            v["url"] = json!(format!("{base}/pair#{code}"));
        }
    }
    if ctx.json {
        output::json(&v);
        return Ok(());
    }
    let scope = if v["admin"] == true { " (admin)" } else { "" };
    let left_ms = v["expiresAt"].as_i64().unwrap_or(0) - chrono::Utc::now().timestamp_millis();
    let minutes = (left_ms.max(0) + 59_999) / 60_000;
    output::line(&format!("Pairing code{scope}: {}", s(&v["code"])));
    output::line(&match v["url"].as_str() {
        Some(url) => format!("Open {url} within {minutes} min."),
        None => format!("Open /pair on the gateway and enter the code within {minutes} min."),
    });
    Ok(())
}

// -- Uninstall everything -------------------------------------------------------------------

pub struct UninstallAll {
    pub purge_profile: bool,
    pub yes: bool,
    /// Where to look for the desktop's `airpost` link; the OS's places when empty.
    pub link_paths: Vec<PathBuf>,
}

/// Where the desktop links `airpost` (`linkLocations` in apps/desktop/src/main/cli-link.ts).
fn default_link_paths() -> Vec<PathBuf> {
    let user = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".local").join("bin").join("airpost"));
    let mut paths = Vec::new();
    if cfg!(target_os = "macos") {
        paths.push(PathBuf::from("/usr/local/bin/airpost"));
    }
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        paths.extend(user);
    }
    paths
}

/// A link the desktop made: a symlink to the `airpost` inside a packaged app
/// (`.../app.asar.unpacked/airpost`). Links into a package manager's tree are that manager's.
fn is_desktop_link(path: &Path) -> bool {
    if !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return false;
    }
    let Ok(target) = fs::read_link(path) else {
        return false;
    };
    let named_airpost = target
        .file_name()
        .is_some_and(|n| n == "airpost" || n == "airpost.exe");
    named_airpost
        && target
            .components()
            .any(|c| c.as_os_str() == "app.asar.unpacked")
}

fn remove_links(paths: &[PathBuf]) -> Vec<Value> {
    paths
        .iter()
        .filter(|p| is_desktop_link(p))
        .map(|p| match fs::remove_file(p) {
            Ok(()) => json!({"path": p, "removed": true}),
            Err(e) => json!({
                "path": p, "removed": false,
                "error": format!("{e}; remove it with: sudo rm {}", p.display()),
            }),
        })
        .collect()
}

fn confirm_purge(ctx: &Ctx, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    let dir = ctx.config_dir.display();
    if ctx.json || !std::io::stdin().is_terminal() {
        return Err(CliError::usage(format!(
            "--purge-profile deletes {dir}; confirm with --yes"
        )));
    }
    let mut err = std::io::stderr();
    let _ = write!(
        err,
        "Delete {dir} and everything in it (the mail cache, accounts and settings)? [y/N] "
    );
    let _ = err.flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::failed("kept the profile; nothing removed"))
    }
}

/// Removes every unit `airpost` installed for the profile, then the desktop's `airpost` link, then
/// (with `purge_profile`) the profile.
pub fn uninstall_all(
    ctx: &Ctx,
    place: &Place,
    args: &UninstallAll,
    loader: &mut dyn Loader,
) -> Result<Value> {
    let config_dir = &ctx.config_dir;
    if args.purge_profile {
        if let Some(pid) = attached_engine(config_dir) {
            return Err(CliError::failed(format!(
                "Airpost's engine (pid {pid}) is using {}; quit Airpost first",
                config_dir.display()
            )));
        }
        confirm_purge(ctx, args.yes)?;
    }
    let mut removed = Vec::new();
    let plugins: Vec<String> = Records::load(config_dir)?.services.into_keys().collect();
    for package in plugins {
        removed.push(service::service_uninstall(
            config_dir, place, &package, loader,
        )?);
    }
    for kind in [Kind::McpHttp, Kind::Gateway, Kind::Engine] {
        let recorded = kind.record(&Records::load(config_dir)?).is_some();
        if recorded || kind.unit_ref(place).unit.exists() {
            removed.push(uninstall(kind, config_dir, place, loader)?);
        }
    }
    let link_paths = if args.link_paths.is_empty() {
        default_link_paths()
    } else {
        args.link_paths.clone()
    };
    let links = remove_links(&link_paths);
    let mut profile = json!({"path": config_dir, "removed": false});
    if args.purge_profile {
        wait_for_engine_exit(config_dir, Duration::from_secs(15));
        match fs::remove_dir_all(config_dir) {
            Ok(()) => profile["removed"] = json!(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(CliError::failed(format!("{}: {e}", config_dir.display()))),
        }
    }
    Ok(json!({"units": removed, "links": links, "profile": profile}))
}

/// The unit's engine exits after `bootout` / `disable --now`, but not at once.
fn wait_for_engine_exit(config_dir: &Path, limit: Duration) {
    let run = config_dir.join("run");
    let start = Instant::now();
    while start.elapsed() < limit {
        let scan = airpost_client::discovery::scan(&run);
        if scan.live.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn uninstall_all_command(ctx: &Ctx, place: &PlaceArgs, args: &UninstallAll) -> Result<()> {
    let place = Place::resolve(&ctx.config_dir, place)?;
    let v = uninstall_all(ctx, &place, args, &mut System)?;
    let mut text: Vec<String> = v["units"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| format!("Removed {}", s(&u["path"])))
        .collect();
    for l in v["links"].as_array().into_iter().flatten() {
        text.push(match l["removed"].as_bool() {
            Some(true) => format!("Removed the link {}", s(&l["path"])),
            _ => format!("Could not remove {}: {}", s(&l["path"]), s(&l["error"])),
        });
    }
    if v["profile"]["removed"] == true {
        text.push(format!("Deleted {}", s(&v["profile"]["path"])));
    }
    if text.is_empty() {
        text.push("Nothing to remove.".to_owned());
    }
    print(ctx, &v, &text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::fake::{executable, Fake};
    use crate::units::Names;

    struct Env {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        config: PathBuf,
        units: PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        for bin in ["airpost-engine", "airpost-gateway", "airpost-mcp"] {
            executable(
                &root.join("a").join(bin),
                &format!("#!/bin/sh\n# {bin} a\n"),
            );
            executable(
                &root.join("b").join(bin),
                &format!("#!/bin/sh\n# {bin} b\n"),
            );
        }
        let config = root.join("profile");
        fs::create_dir_all(&config).unwrap();
        Env {
            config,
            units: root.join("units"),
            root,
            _tmp: tmp,
        }
    }

    fn place(env: &Env, manager: Manager) -> Place {
        Place {
            manager,
            dir: env.units.clone(),
            load: false,
            no_start: false,
            names: Names::for_profile(&env.config, Some(&env.config)),
        }
    }

    fn opts(env: &Env, copy: &str, kind: Kind) -> Options {
        Options {
            bin: Some(env.root.join(copy).join(kind.bin_name())),
            ..Options::default()
        }
    }

    fn enable_mcp_http(env: &Env, port: u16) {
        let policy = json!({"http": {"enabled": true, "port": port}});
        fs::write(env.config.join(MCP_POLICY_FILE), policy.to_string()).unwrap();
    }

    #[test]
    fn the_gateway_unit_listens_for_its_first_connection() {
        let env = env();
        let web = env.root.join("a").join("airpost-web");
        fs::create_dir_all(&web).unwrap();
        fs::write(web.join("index.html"), "<html>").unwrap();
        let p = place(&env, Manager::Launchd);
        let mut o = opts(&env, "a", Kind::Gateway);
        // The desktop passes an empty --web-root when its build has none.
        o.web_root = Some(PathBuf::new());
        let v = install(Kind::Gateway, &env.config, &p, &o, &mut Fake::default()).unwrap();
        assert_eq!(v["installed"], true);
        assert_eq!(v["url"], "http://127.0.0.1:5180");
        let plist =
            fs::read_to_string(env.units.join("com.airpost.airpost.gateway.plist")).unwrap();
        assert!(plist.contains("<key>Sockets</key>") && !plist.contains("RunAtLoad"));
        assert!(plist.contains(&format!(
            "<string>serve</string>\n\t\t<string>--bind</string>\n\t\t\
             <string>127.0.0.1:5180</string>\n\t\t<string>--web-root</string>\n\t\t\
             <string>{}</string>",
            web.display()
        )));
        assert!(plist.contains(&format!(
            "<key>AIRPOST_CONFIG_DIR</key>\n\t\t<string>{}</string>",
            env.config.display()
        )));
        let r = Records::load(&env.config).unwrap().gateway.unwrap();
        assert_eq!(
            (r.port, r.url.as_deref()),
            (Some(5180), Some("http://127.0.0.1:5180"))
        );
        assert_eq!(r.profile.as_deref(), Some(env.config.as_path()));

        let p = place(&env, Manager::Systemd);
        o.bind = Some("0.0.0.0:8080".parse().unwrap());
        o.public_url = Some("https://airpost.example.com/".into());
        install(Kind::Gateway, &env.config, &p, &o, &mut Fake::default()).unwrap();
        let service = fs::read_to_string(env.units.join("airpost-gateway.service")).unwrap();
        assert!(service.contains("\"--public-url\" \"https://airpost.example.com/\""));
        assert!(!service.contains("[Install]"));
        let socket = fs::read_to_string(env.units.join("airpost-gateway.socket")).unwrap();
        assert!(socket.contains("ListenStream=0.0.0.0:8080\nFileDescriptorName=http\n"));
        let r = Records::load(&env.config).unwrap().gateway.unwrap();
        assert_eq!(r.url.as_deref(), Some("https://airpost.example.com"));

        uninstall(Kind::Gateway, &env.config, &p, &mut Fake::default()).unwrap();
        assert!(!env.units.join("airpost-gateway.service").exists());
        assert!(!env.units.join("airpost-gateway.socket").exists());
        assert!(Records::load(&env.config).unwrap().gateway.is_none());
        o.bind = Some("127.0.0.1:0".parse().unwrap());
        let e = install(Kind::Gateway, &env.config, &p, &o, &mut Fake::default()).unwrap_err();
        assert!(e.to_string().contains("port 0"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn a_package_managers_links_stay_in_the_unit() {
        // Homebrew's layout: bin/ links into a versioned Cellar, share/ beside bin/.
        let env = env();
        let prefix = env.root.join("prefix");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        let link = prefix.join("bin").join("airpost-gateway");
        std::os::unix::fs::symlink(env.root.join("a/airpost-gateway"), &link).unwrap();
        let web = prefix.join("share").join("airpost-gateway").join("web");
        fs::create_dir_all(&web).unwrap();
        fs::write(web.join("index.html"), "<html>").unwrap();
        let p = place(&env, Manager::Systemd);
        let o = Options {
            bin: Some(link.clone()),
            ..Options::default()
        };
        install(Kind::Gateway, &env.config, &p, &o, &mut Fake::default()).unwrap();
        let unit = fs::read_to_string(env.units.join("airpost-gateway.service")).unwrap();
        assert!(
            unit.contains(&format!(
                "ExecStart=\"{}\" \"serve\" \"--bind\" \"127.0.0.1:5180\" \"--web-root\" \"{}\"",
                link.display(),
                web.display()
            )),
            "{unit}"
        );
        // The file the link points to is the same binary, not another copy's.
        let mut same = opts(&env, "a", Kind::Gateway);
        install(Kind::Gateway, &env.config, &p, &same, &mut Fake::default()).unwrap();
        same.bin = Some(link);
        install(Kind::Gateway, &env.config, &p, &same, &mut Fake::default()).unwrap();
    }

    #[test]
    fn another_copys_unit_is_replaced_only_when_asked() {
        let env = env();
        let p = place(&env, Manager::Launchd);
        let mut fake = Fake::default();
        let a = opts(&env, "a", Kind::Engine);
        install(Kind::Engine, &env.config, &p, &a, &mut fake).unwrap();
        // The same copy installs again.
        install(Kind::Engine, &env.config, &p, &a, &mut fake).unwrap();
        let mut b = opts(&env, "b", Kind::Engine);
        let e = install(Kind::Engine, &env.config, &p, &b, &mut fake).unwrap_err();
        let msg = e.to_string();
        assert!(
            msg.contains("another copy") && msg.contains("--replace"),
            "{msg}"
        );
        assert!(msg.contains(&env.root.join("a/airpost-engine").display().to_string()));
        assert_eq!(
            installed_bin(Kind::Engine, &p).unwrap(),
            env.root.join("a/airpost-engine")
        );
        b.replace = true;
        let v = install(Kind::Engine, &env.config, &p, &b, &mut fake).unwrap();
        assert_eq!(v["replaces"], json!(env.root.join("a/airpost-engine")));
        assert_eq!(
            installed_bin(Kind::Engine, &p).unwrap(),
            env.root.join("b/airpost-engine")
        );
    }

    #[test]
    fn mcp_http_needs_http_on_and_takes_its_port() {
        let env = env();
        let p = place(&env, Manager::Systemd);
        let mut o = opts(&env, "a", Kind::McpHttp);
        let e = install(Kind::McpHttp, &env.config, &p, &o, &mut Fake::default()).unwrap_err();
        assert!(e.to_string().contains("HTTP is off"), "{e}");
        enable_mcp_http(&env, 2601);
        let v = install(Kind::McpHttp, &env.config, &p, &o, &mut Fake::default()).unwrap();
        assert_eq!(v["url"], "http://127.0.0.1:2601/mcp");
        let unit = fs::read_to_string(env.units.join("airpost-mcp-http.service")).unwrap();
        assert!(unit.contains("\"http\" \"--port\" \"2601\""), "{unit}");
        let socket = fs::read_to_string(env.units.join("airpost-mcp-http.socket")).unwrap();
        assert!(socket.contains("ListenStream=127.0.0.1:2601\nFileDescriptorName=mcp\n"));
        o.port = Some(2602);
        o.dry_run = true;
        let v = install(Kind::McpHttp, &env.config, &p, &o, &mut Fake::default()).unwrap();
        assert_eq!(
            (v["port"].as_u64(), v["installed"].as_bool()),
            (Some(2602), Some(false))
        );
        assert!(
            fs::read_to_string(env.units.join("airpost-mcp-http.socket"))
                .unwrap()
                .contains(":2601")
        );
    }

    #[test]
    fn the_manager_is_told_what_kind_of_unit_it_runs() {
        let env = env();
        let mut p = place(&env, Manager::Launchd);
        let mut fake = Fake::default();
        assert!(start(Kind::Gateway, &p, false, &mut fake).is_err());
        install(
            Kind::Gateway,
            &env.config,
            &p,
            &opts(&env, "a", Kind::Gateway),
            &mut fake,
        )
        .unwrap();
        p.load = true;
        start(Kind::Gateway, &p, false, &mut fake).unwrap();
        stop(Kind::Gateway, &p, &mut fake).unwrap();
        p.no_start = true;
        install(
            Kind::Engine,
            &env.config,
            &p,
            &opts(&env, "a", Kind::Engine),
            &mut fake,
        )
        .unwrap();
        start(Kind::Engine, &p, true, &mut fake).unwrap();
        p.no_start = false;
        install(
            Kind::Engine,
            &env.config,
            &p,
            &opts(&env, "a", Kind::Engine),
            &mut fake,
        )
        .unwrap();
        uninstall(Kind::Gateway, &env.config, &p, &mut fake).unwrap();
        assert_eq!(
            fake.calls,
            [
                "start launchd com.airpost.airpost.gateway (socket)",
                "stop launchd com.airpost.airpost.gateway (socket)",
                "enable launchd com.airpost.airpost.engine",
                "restart launchd com.airpost.airpost.engine",
                "load launchd com.airpost.airpost.engine",
                "unload launchd com.airpost.airpost.gateway (socket)",
            ]
        );
    }

    #[test]
    fn status_rows_show_units_and_running_services() {
        let env = env();
        let p = place(&env, Manager::Launchd);
        enable_mcp_http(&env, 2604);
        let o = opts(&env, "a", Kind::McpHttp);
        install(Kind::McpHttp, &env.config, &p, &o, &mut Fake::default()).unwrap();
        // airpost-mcp http's running file, for this live process.
        let running = json!({"pid": std::process::id(), "url": "http://127.0.0.1:2604/mcp",
                             "port": 2604, "build": "sha256:00"});
        fs::write(env.config.join(MCP_RUNNING_FILE), running.to_string()).unwrap();
        let gw = env.config.join("gateway");
        fs::create_dir_all(&gw).unwrap();
        let dead = json!({"pid": u32::MAX, "publicUrl": "http://127.0.0.1:5180"});
        fs::write(gw.join(GATEWAY_RUNNING_FILE), dead.to_string()).unwrap();

        let rows = status_rows(&env.config, Some(&p), &[]);
        let names: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
        assert_eq!(names, ["engine", "gateway", "mcp-http"]);
        for r in &rows {
            let keys = ["installed", "running", "pid", "url", "port", "unit", "bin"];
            assert!(keys.iter().all(|k| r.get(*k).is_some()), "{r}");
        }
        let (engine, gateway, mcp) = (&rows[0], &rows[1], &rows[2]);
        assert_eq!(
            (engine["installed"].as_bool(), engine["running"].as_bool()),
            (Some(false), Some(false))
        );
        assert_eq!(engine["unit"], Value::Null);
        assert_eq!(gateway["running"], false);
        assert_eq!(gateway["url"], Value::Null);
        assert_eq!(
            (mcp["installed"].as_bool(), mcp["running"].as_bool()),
            (Some(true), Some(true))
        );
        assert_eq!(mcp["pid"], std::process::id());
        assert_eq!(mcp["url"], "http://127.0.0.1:2604/mcp");
        assert_eq!(mcp["port"], 2604);
        assert_eq!(
            mcp["unit"],
            json!(env.units.join("com.airpost.airpost.mcp-http.plist"))
        );
        assert_eq!(mcp["bin"], json!(env.root.join("a/airpost-mcp")));
        // No airpost-mcp beside the test binary: the unit belongs to another copy.
        assert_eq!(mcp["owner"], "other");
        assert_eq!(mcp["onDemand"], true);

        // Not running: the URL the unit listens on.
        fs::remove_file(env.config.join(MCP_RUNNING_FILE)).unwrap();
        let rows = status_rows(&env.config, Some(&p), &[]);
        assert_eq!(rows[2]["running"], false);
        assert_eq!(rows[2]["port"], 2604);

        // A unit file that went away is reported.
        fs::remove_file(env.units.join("com.airpost.airpost.mcp-http.plist")).unwrap();
        let rows = status_rows(&env.config, Some(&p), &[]);
        assert_eq!(rows[2]["installed"], false);
        assert!(rows[2]["warnings"][0]
            .as_str()
            .unwrap()
            .contains("isn't there"));
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_removes_every_unit_and_only_the_desktops_link() {
        use std::os::unix::fs::symlink;
        let env = env();
        let p = place(&env, Manager::Systemd);
        let mut fake = Fake::default();
        for kind in [Kind::Engine, Kind::Gateway] {
            install(kind, &env.config, &p, &opts(&env, "a", kind), &mut fake).unwrap();
        }
        let links = env.root.join("links");
        fs::create_dir_all(&links).unwrap();
        let desktop = links.join("airpost");
        symlink(
            "/Applications/Airpost.app/Contents/Resources/app.asar.unpacked/airpost",
            &desktop,
        )
        .unwrap();
        let brew = links.join("airpost-brew");
        symlink("/opt/homebrew/Cellar/airpost/1.0/bin/airpost", &brew).unwrap();
        let file = links.join("airpost-file");
        fs::write(&file, "#!/bin/sh\n").unwrap();
        let ctx = Ctx {
            config_dir: env.config.clone(),
            account: None,
            json: true,
            style: output::Style::default(),
        };
        let args = UninstallAll {
            purge_profile: false,
            yes: false,
            link_paths: vec![desktop.clone(), brew.clone(), file.clone()],
        };
        let v = uninstall_all(&ctx, &p, &args, &mut fake).unwrap();
        assert_eq!(v["units"].as_array().unwrap().len(), 2);
        assert_eq!(v["links"], json!([{"path": desktop, "removed": true}]));
        assert!(fs::symlink_metadata(&desktop).is_err());
        assert!(fs::symlink_metadata(&brew).is_ok() && file.exists());
        assert_eq!(fs::read_dir(&env.units).unwrap().count(), 0);
        assert!(env.config.exists());

        let purge = UninstallAll {
            purge_profile: true,
            yes: false,
            link_paths: vec![file.clone()],
        };
        // --json and no --yes: nothing to ask on.
        let e = uninstall_all(&ctx, &p, &purge, &mut fake).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        let purge = UninstallAll { yes: true, ..purge };
        let v = uninstall_all(&ctx, &p, &purge, &mut fake).unwrap();
        assert_eq!(v["profile"]["removed"], true);
        assert!(!env.config.exists());
    }

    #[test]
    fn urls_and_ports() {
        assert_eq!(url_port("http://127.0.0.1:5180"), Some(5180));
        assert_eq!(url_port("http://[::1]:9/mcp"), Some(9));
        assert_eq!(url_port("https://airpost.example.com"), None);
        assert_eq!(
            local_url("0.0.0.0:80".parse().unwrap(), "/x"),
            "http://127.0.0.1:80/x"
        );
        assert_eq!(local_url("[::]:80".parse().unwrap(), ""), "http://[::1]:80");
        assert_eq!(Kind::parse("mcp-http"), Some(Kind::McpHttp));
        assert_eq!(Kind::parse("service"), None);
    }
}
