// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost service install|uninstall|list`: service plugins, run by the OS as user units
//! ([`crate::units`]) next to the engine unit ([`crate::builtin`]) they talk to.
//!
//! A plugin declares its service in `package.json`:
//!
//! ```json
//! "airpost": {"service": {"bin": "bin/my-service", "args": ["--quiet"]}}
//! ```
//!
//! `bin` is a path inside the package, or an object keyed by `<platform>-<arch>` in Node's
//! names (`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`). Installing copies that one
//! file to `<config>/services/<package>/` and the unit runs the copy, so the binary the user
//! was shown (path and SHA-256, TD29) is the one that runs until the next install, whatever
//! later happens to the package. The unit sets `AIRPOST_CONFIG_DIR` and `AIRPOST_SERVICE_PACKAGE`.
//! What was installed, from where and with which hash is kept in `<config>/services.json`.
//! A dry run also prints `planSha256`, a digest of the binary's hash and every unit the
//! install would write; `--expect-plan` refuses an install whose plan is no longer that one.
//!
//! Units are loaded (`launchctl bootstrap`, `systemctl --user enable`) only when they are
//! written to the manager's own directory; `--unit-dir` writes them elsewhere and loads
//! nothing, as does `--no-load` or `AIRPOST_SERVICE_NO_LOAD`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::Ctx;
use crate::builtin;
use crate::error::{CliError, Result};
use crate::output;
use crate::units::{self, Manager, Names, UnitSpec};

pub const RECORDS_FILE: &str = "services.json";
pub const SERVICES_DIR: &str = "services";
const SERVICE_LOG: &str = "service.log";

/// Where units go, and whether to load them.
#[derive(Clone, Debug)]
pub struct Place {
    pub manager: Manager,
    pub dir: PathBuf,
    pub load: bool,
    /// Enable for login but don't start now (`--no-start`).
    pub no_start: bool,
    pub names: Names,
}

/// The unit options every install and uninstall command takes.
#[derive(Clone, Debug, Default)]
pub struct PlaceArgs {
    pub unit_dir: Option<PathBuf>,
    pub manager: Option<Manager>,
    pub no_load: bool,
    pub no_start: bool,
}

impl Place {
    pub fn resolve(config_dir: &Path, args: &PlaceArgs) -> Result<Place> {
        let manager = match args.manager.or_else(Manager::host) {
            Some(m) => m,
            None => {
                return Err(CliError::failed(
                    "Airpost installs services with launchd (macOS) or systemd (Linux) only",
                ))
            }
        };
        let (dir, own_dir) = match &args.unit_dir {
            Some(d) => (absolute(d)?, false),
            None => (
                manager.default_dir().ok_or_else(|| {
                    CliError::failed("HOME is not set, so there is no user unit directory")
                })?,
                true,
            ),
        };
        let no_load_env =
            std::env::var_os("AIRPOST_SERVICE_NO_LOAD").is_some_and(|v| !v.is_empty());
        let default_profile = airpost_client::discovery::default_config_dirs()
            .into_iter()
            .next();
        Ok(Place {
            manager,
            dir,
            load: own_dir && !args.no_load && !no_load_env,
            no_start: args.no_start,
            names: Names::for_profile(config_dir, default_profile.as_deref()),
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(self.manager.file_name(name))
    }

    /// The systemd `.socket` unit beside `name`'s unit.
    pub fn socket_path(&self, name: &str) -> Option<PathBuf> {
        self.manager
            .socket_file_name(name)
            .map(|f| self.dir.join(f))
    }

    pub fn unit(&self, name: &str, socket: bool) -> UnitRef {
        UnitRef {
            manager: self.manager,
            name: name.to_owned(),
            unit: self.path(name),
            socket,
        }
    }
}

pub fn absolute(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        return Ok(p.to_owned());
    }
    std::env::current_dir()
        .map(|d| d.join(p))
        .map_err(|e| CliError::failed(format!("no current directory: {e}")))
}

// -- Manifest -------------------------------------------------------------------------------

/// A package's `airpost.service`, resolved for this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub package: String,
    pub dir: PathBuf,
    /// The binary in the package, canonical.
    pub bin: PathBuf,
    pub args: Vec<String>,
}

/// `darwin-arm64`, `linux-x64`, ...: Node's `process.platform`-`process.arch`, the names
/// plugin authors know.
pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    };
    format!("{os}-{arch}")
}

/// The desktop's rule for package names (`isValidPackageName` in `src/packages/package.ts`).
pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn read_manifest(dir: &Path, platform: &str) -> std::result::Result<Manifest, String> {
    let file = dir.join("package.json");
    let text =
        fs::read_to_string(&file).map_err(|e| format!("could not read {}: {e}", file.display()))?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON: {e}", file.display()))?;
    let package = json["name"].as_str().unwrap_or_default().to_owned();
    if !valid_package_name(&package) {
        return Err(format!(
            "{} has an invalid or missing \"name\"",
            file.display()
        ));
    }
    let service = &json["airpost"]["service"];
    if !service.is_object() {
        return Err(format!("{package} declares no service (airpost.service)"));
    }
    let entry = match &service["bin"] {
        Value::String(s) => s.clone(),
        Value::Object(per_platform) => match per_platform.get(platform) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                let mut have: Vec<&str> = per_platform.keys().map(String::as_str).collect();
                have.sort();
                return Err(format!(
                    "{package} has no service binary for {platform} (it has: {})",
                    have.join(", ")
                ));
            }
        },
        _ => return Err(format!("{package}: airpost.service.bin is missing")),
    };
    let args = match &service["args"] {
        Value::Null => Vec::new(),
        Value::Array(a) if a.iter().all(Value::is_string) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => {
            return Err(format!(
                "{package}: airpost.service.args must be a list of strings"
            ))
        }
    };
    let relative = Path::new(&entry);
    if entry.is_empty() || relative.is_absolute() {
        return Err(format!(
            "{package}: airpost.service.bin must be a path inside the package"
        ));
    }
    let root = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let bin = root
        .join(relative)
        .canonicalize()
        .map_err(|e| format!("{package}: the service binary {entry}: {e}"))?;
    if !bin.starts_with(&root) {
        return Err(format!(
            "{package}: the service binary {entry} is outside the package"
        ));
    }
    let meta = fs::metadata(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    if !meta.is_file() || !is_executable(&meta) {
        return Err(format!(
            "{package}: the service binary {} is not an executable file",
            bin.display()
        ));
    }
    Ok(Manifest {
        package,
        dir: root,
        bin,
        args,
    })
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    true
}

/// The installed package named `name`: in `<config>/packages`, else `<config>/dev/packages`
/// (the desktop's dev mode), matched by the `name` in its package.json as the desktop does.
pub fn find_package(config_dir: &Path, name: &str) -> Result<PathBuf> {
    let roots = [
        config_dir.join("packages"),
        config_dir.join("dev").join("packages"),
    ];
    for root in &roots {
        let direct = root.join(name);
        if package_name(&direct).as_deref() == Some(name) {
            return Ok(direct);
        }
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if package_name(&entry.path()).as_deref() == Some(name) {
                return Ok(entry.path());
            }
        }
    }
    Err(CliError::failed(format!(
        "no plugin named {name} in {}; install it in Airpost first, or give --package-dir",
        roots[0].display()
    )))
}

fn package_name(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("package.json")).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    json["name"].as_str().map(str::to_owned)
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(units::hex(&hasher.finalize()))
}

// -- Records --------------------------------------------------------------------------------

/// `<config>/services.json`: what `airpost` installed, so it can be shown, checked and removed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Records {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<UnitRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<UnitRecord>,
    #[serde(default, rename = "mcpHttp", skip_serializing_if = "Option::is_none")]
    pub mcp_http: Option<UnitRecord>,
    #[serde(default)]
    pub services: BTreeMap<String, ServiceRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRecord {
    pub package: String,
    /// The package directory it was installed from.
    pub source: PathBuf,
    /// The binary in that package that was copied.
    pub source_bin: PathBuf,
    /// The copy the unit runs.
    pub bin: PathBuf,
    pub args: Vec<String>,
    pub sha256: String,
    pub manager: String,
    pub name: String,
    pub unit: PathBuf,
    pub installed_at: i64,
}

/// A unit of Airpost's own: the engine, the gateway or `airpost-mcp http`. The desktop reads the
/// engine's `name` and `unit` (`src/engine/supervisor/unit.ts`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitRecord {
    /// The binary the unit runs.
    pub bin: PathBuf,
    pub sha256: String,
    pub manager: String,
    pub name: String,
    pub unit: PathBuf,
    /// The systemd `.socket` unit of a unit that starts on demand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_unit: Option<PathBuf>,
    /// The profile the unit serves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<PathBuf>,
    /// The `airpost` that installed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_by: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub installed_at: i64,
}

impl Records {
    pub fn load(config_dir: &Path) -> Result<Records> {
        let path = config_dir.join(RECORDS_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| CliError::failed(format!("{} is not valid: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Records::default()),
            Err(e) => Err(CliError::failed(format!("{}: {e}", path.display()))),
        }
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).unwrap_or_default() + "\n";
        write_atomic(&config_dir.join(RECORDS_FILE), text.as_bytes(), false)
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let fail = |e: std::io::Error| CliError::failed(format!("{}: {e}", path.display()));
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(fail)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    fs::write(&tmp, bytes).map_err(fail)?;
    set_mode(&tmp, if executable { 0o755 } else { 0o644 }).map_err(fail)?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        fail(e)
    })
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// -- Loading --------------------------------------------------------------------------------

/// A unit as the service manager knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitRef {
    pub manager: Manager,
    /// The launchd label, or the systemd unit name without its suffix.
    pub name: String,
    pub unit: PathBuf,
    /// Started on demand by its socket (launchd `Sockets`, a systemd `.socket` unit).
    pub socket: bool,
}

/// Starts and stops units. Only [`System`] touches the service manager.
pub trait Loader {
    /// Loads the unit as written now, replacing a loaded older definition. A unit that starts
    /// at login starts now; one that starts on demand listens.
    fn load(&mut self, u: &UnitRef) -> Result<()>;
    /// Makes the unit start at the next login without starting it now.
    fn enable(&mut self, u: &UnitRef) -> Result<()>;
    /// Stops the unit and forgets it; a unit that isn't loaded is no error.
    fn unload(&mut self, u: &UnitRef) -> Result<()>;
    /// Starts the unit's program when it isn't running; `restart` stops a running one first.
    fn start(&mut self, u: &UnitRef, restart: bool) -> Result<()>;
    /// Stops the program, and the listening socket of a unit that starts on demand, until
    /// the next login or `start`.
    fn stop(&mut self, u: &UnitRef) -> Result<()>;
}

pub struct System;

impl System {
    pub fn run(program: &str, args: &[&str]) -> std::result::Result<(), String> {
        if cfg!(test) {
            return Err(format!("tests never run {program}"));
        }
        let out = std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "{program} {} failed: {}",
            args.join(" "),
            err.trim()
        ))
    }

    #[cfg(unix)]
    fn gui_domain() -> String {
        // SAFETY: getuid has no preconditions and can't fail.
        format!("gui/{}", unsafe { libc::getuid() })
    }

    #[cfg(not(unix))]
    fn gui_domain() -> String {
        String::new()
    }

    fn target(u: &UnitRef) -> String {
        format!("{}/{}", Self::gui_domain(), u.name)
    }

    fn bootstrap(u: &UnitRef) -> std::result::Result<(), String> {
        Self::run(
            "launchctl",
            &["bootstrap", &Self::gui_domain(), &u.unit.to_string_lossy()],
        )
    }

    fn systemctl(args: &[&str]) -> std::result::Result<(), String> {
        let mut all = vec!["--user"];
        all.extend_from_slice(args);
        Self::run("systemctl", &all)
    }

    /// `name.socket`, or `name.service` for a unit that starts at login.
    fn enabled_unit(u: &UnitRef) -> String {
        format!("{}.{}", u.name, if u.socket { "socket" } else { "service" })
    }
}

impl Loader for System {
    fn load(&mut self, u: &UnitRef) -> Result<()> {
        let result = match u.manager {
            Manager::Launchd => {
                // A loaded job keeps its old definition until it is booted out.
                let _ = Self::run("launchctl", &["bootout", &Self::target(u)]);
                Self::bootstrap(u)
            }
            Manager::Systemd => {
                let service = format!("{}.service", u.name);
                let enabled = Self::enabled_unit(u);
                Self::systemctl(&["daemon-reload"]).and_then(|()| {
                    if u.socket {
                        // The socket may listen on another address now.
                        let _ = Self::systemctl(&["stop", &service, &enabled]);
                        Self::systemctl(&["enable", &enabled])
                            .and_then(|()| Self::systemctl(&["start", &enabled]))
                    } else {
                        Self::systemctl(&["enable", &service])
                            .and_then(|()| Self::systemctl(&["restart", &service]))
                    }
                })
            }
        };
        result.map_err(CliError::failed)
    }

    fn enable(&mut self, u: &UnitRef) -> Result<()> {
        let result = match u.manager {
            // launchd loads every plist in ~/Library/LaunchAgents at login. An older definition
            // still loaded (its program not running) is booted out, so `start` loads this one.
            Manager::Launchd => {
                let _ = Self::run("launchctl", &["bootout", &Self::target(u)]);
                Ok(())
            }
            Manager::Systemd => Self::systemctl(&["daemon-reload"])
                .and_then(|()| Self::systemctl(&["enable", &Self::enabled_unit(u)])),
        };
        result.map_err(CliError::failed)
    }

    fn unload(&mut self, u: &UnitRef) -> Result<()> {
        match u.manager {
            Manager::Launchd => {
                let _ = Self::run("launchctl", &["bootout", &Self::target(u)]);
            }
            Manager::Systemd => {
                let _ = Self::systemctl(&["disable", "--now", &Self::enabled_unit(u)]);
                if u.socket {
                    let _ = Self::systemctl(&["stop", &format!("{}.service", u.name)]);
                }
            }
        }
        Ok(())
    }

    fn start(&mut self, u: &UnitRef, restart: bool) -> Result<()> {
        let result = match u.manager {
            Manager::Launchd => {
                let target = Self::target(u);
                let kick = if restart {
                    vec!["kickstart", "-k", &target]
                } else {
                    vec!["kickstart", &target]
                };
                // A job that was booted out (or never loaded this login) has to be bootstrapped
                // first. RunAtLoad then starts it; one that starts on demand is kicked too.
                Self::run("launchctl", &kick).or_else(|_| {
                    Self::bootstrap(u).and_then(|()| {
                        if u.socket {
                            Self::run("launchctl", &["kickstart", &target])
                        } else {
                            Ok(())
                        }
                    })
                })
            }
            Manager::Systemd => {
                let service = format!("{}.service", u.name);
                let verb = if restart { "restart" } else { "start" };
                if u.socket {
                    Self::systemctl(&["start", &Self::enabled_unit(u)])
                        .and_then(|()| Self::systemctl(&[verb, &service]))
                } else {
                    Self::systemctl(&[verb, &service])
                }
            }
        };
        result.map_err(CliError::failed)
    }

    fn stop(&mut self, u: &UnitRef) -> Result<()> {
        let result = match u.manager {
            Manager::Launchd => {
                // A running job that isn't loaded is no error: there is nothing to stop.
                let _ = Self::run("launchctl", &["bootout", &Self::target(u)]);
                Ok(())
            }
            Manager::Systemd => {
                let service = format!("{}.service", u.name);
                if u.socket {
                    Self::systemctl(&["stop", &Self::enabled_unit(u), &service])
                } else {
                    Self::systemctl(&["stop", &service])
                }
            }
        };
        result.map_err(CliError::failed)
    }
}

pub fn reload_after_remove(place: &Place) {
    if place.load && place.manager == Manager::Systemd {
        let _ = System::run("systemctl", &["--user", "daemon-reload"]);
    }
}

pub fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CliError::failed(format!("{}: {e}", path.display()))),
    }
}

// -- Service units --------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ServiceArgs {
    pub package: String,
    pub package_dir: Option<PathBuf>,
    pub engine_bin: Option<PathBuf>,
    /// The engine unit's `AIRPOST_ENGINE_KEY_FILE`, when the engine unit is written.
    pub engine_key_file: Option<PathBuf>,
    pub expect_sha256: Option<String>,
    /// Refuse unless the plan (binary hash, unit texts and paths) is the one a dry run showed
    /// as `planSha256`.
    pub expect_plan: Option<String>,
    pub dry_run: bool,
}

#[derive(Clone, Debug)]
pub struct ServicePlan {
    pub manifest: Manifest,
    pub sha256: String,
    pub installed_bin: PathBuf,
    pub name: String,
    pub unit: PathBuf,
    pub text: String,
    /// The engine unit to write first; None when it is there already.
    pub engine: Option<builtin::Plan>,
    /// Digest of everything the install writes and runs: what `--expect-plan` pins.
    pub plan_sha256: String,
    engine_name: String,
    engine_unit: PathBuf,
}

impl ServicePlan {
    pub fn new(config_dir: &Path, place: &Place, args: &ServiceArgs) -> Result<ServicePlan> {
        if !valid_package_name(&args.package) {
            return Err(CliError::usage(format!(
                "{:?} is not a plugin name",
                args.package
            )));
        }
        let dir = match &args.package_dir {
            Some(d) => absolute(d)?,
            None => find_package(config_dir, &args.package)?,
        };
        let manifest = read_manifest(&dir, &platform_key()).map_err(CliError::failed)?;
        if manifest.package != args.package {
            return Err(CliError::failed(format!(
                "{} holds the plugin {}, not {}",
                dir.display(),
                manifest.package,
                args.package
            )));
        }
        let sha256 = sha256_file(&manifest.bin)
            .map_err(|e| CliError::failed(format!("{}: {e}", manifest.bin.display())))?;
        check_expected(&sha256, args.expect_sha256.as_deref(), &manifest.bin)?;
        let service_dir = config_dir.join(SERVICES_DIR).join(&manifest.package);
        let installed_bin = service_dir.join(manifest.bin.file_name().unwrap_or_default());
        let engine_name = place.names.engine(place.manager);
        let engine_unit = place.path(&engine_name);
        let engine = if engine_unit.exists() {
            None
        } else {
            Some(builtin::Plan::engine(
                config_dir,
                place,
                args.engine_bin.as_deref(),
                args.engine_key_file.as_deref(),
            )?)
        };
        let name = place.names.service(place.manager, &manifest.package);
        let mut program = vec![installed_bin.to_string_lossy().into_owned()];
        program.extend(manifest.args.iter().cloned());
        let spec = UnitSpec {
            name: name.clone(),
            description: format!("Airpost service plugin {}", manifest.package),
            program,
            env: vec![
                (
                    "AIRPOST_CONFIG_DIR".to_owned(),
                    config_dir.to_string_lossy().into_owned(),
                ),
                (
                    "AIRPOST_SERVICE_PACKAGE".to_owned(),
                    manifest.package.clone(),
                ),
            ],
            after: Some(engine_name.clone()),
            log: Some(service_dir.join(SERVICE_LOG)),
            socket: None,
        };
        let text = units::render(place.manager, &spec).map_err(CliError::failed)?;
        let unit = place.path(&name);
        let plan_sha256 = plan_digest(&sha256, &installed_bin, &unit, &text, engine.as_ref());
        if let Some(want) = args.expect_plan.as_deref() {
            if !want.eq_ignore_ascii_case(&plan_sha256) {
                return Err(CliError::failed(format!(
                    "the install of {} changed since it was shown (plan {plan_sha256}, not \
                     {want}); check it and install again",
                    manifest.package
                )));
            }
        }
        Ok(ServicePlan {
            unit,
            manifest,
            sha256,
            installed_bin,
            name,
            text,
            engine,
            plan_sha256,
            engine_name,
            engine_unit,
        })
    }

    pub fn to_json(&self, place: &Place) -> Value {
        let engine = match &self.engine {
            Some(e) => e.to_json(),
            None => json!({
                "installed": true,
                "name": self.engine_name,
                "path": self.engine_unit,
                "text": fs::read_to_string(&self.engine_unit).unwrap_or_default(),
            }),
        };
        json!({
            "package": self.manifest.package,
            "source": self.manifest.dir,
            "bin": self.manifest.bin,
            "sha256": self.sha256,
            "planSha256": self.plan_sha256,
            "installedBin": self.installed_bin,
            "args": self.manifest.args,
            "manager": place.manager.as_str(),
            "load": place.load,
            "unit": {"name": self.name, "path": self.unit, "text": self.text},
            "engine": engine,
        })
    }

    pub fn apply(
        &self,
        config_dir: &Path,
        place: &Place,
        loader: &mut dyn Loader,
    ) -> Result<ServiceRecord> {
        if let Some(engine) = &self.engine {
            engine.apply(config_dir, place, loader)?;
        }
        let bytes = fs::read(&self.manifest.bin)
            .map_err(|e| CliError::failed(format!("{}: {e}", self.manifest.bin.display())))?;
        // Hashed again as read: the file may have changed since the plan was shown.
        let copied = units::hex(&Sha256::digest(&bytes));
        check_expected(&copied, Some(&self.sha256), &self.manifest.bin)?;
        write_atomic(&self.installed_bin, &bytes, true)?;
        write_atomic(&self.unit, self.text.as_bytes(), false)?;
        let record = ServiceRecord {
            package: self.manifest.package.clone(),
            source: self.manifest.dir.clone(),
            source_bin: self.manifest.bin.clone(),
            bin: self.installed_bin.clone(),
            args: self.manifest.args.clone(),
            sha256: copied,
            manager: place.manager.as_str().to_owned(),
            name: self.name.clone(),
            unit: self.unit.clone(),
            installed_at: now(),
        };
        let mut records = Records::load(config_dir)?;
        records
            .services
            .insert(record.package.clone(), record.clone());
        records.save(config_dir)?;
        if place.load {
            loader.load(&place.unit(&self.name, false))?;
        }
        Ok(record)
    }
}

/// SHA-256 over what a service install writes and runs: the service binary's hash, where it
/// is copied, the service unit's path and text, and the engine unit's binary hash, path and
/// text when the install writes it. Each part is length-prefixed so no two plans collide.
fn plan_digest(
    bin_sha256: &str,
    installed_bin: &Path,
    unit: &Path,
    text: &str,
    engine: Option<&builtin::Plan>,
) -> String {
    let mut h = Sha256::new();
    let mut part = |bytes: &[u8]| {
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    };
    part(b"airpost-service-plan-1");
    part(bin_sha256.as_bytes());
    part(installed_bin.to_string_lossy().as_bytes());
    part(unit.to_string_lossy().as_bytes());
    part(text.as_bytes());
    match engine {
        Some(e) => {
            part(b"engine");
            part(e.sha256.as_bytes());
            part(e.unit.to_string_lossy().as_bytes());
            part(e.text.as_bytes());
        }
        None => part(b"engine-kept"),
    }
    units::hex(&h.finalize())
}

fn check_expected(actual: &str, expected: Option<&str>, bin: &Path) -> Result<()> {
    match expected {
        Some(want) if !want.eq_ignore_ascii_case(actual) => Err(CliError::failed(format!(
            "{} changed: its SHA-256 is {actual}, not {want}; check it and install again",
            bin.display()
        ))),
        _ => Ok(()),
    }
}

pub fn service_install(
    config_dir: &Path,
    place: &Place,
    args: &ServiceArgs,
    loader: &mut dyn Loader,
) -> Result<Value> {
    let plan = ServicePlan::new(config_dir, place, args)?;
    let mut out = plan.to_json(place);
    out["installed"] = json!(false);
    if !args.dry_run {
        let record = plan.apply(config_dir, place, loader)?;
        out["installed"] = json!(true);
        out["record"] = json!(record);
    }
    Ok(out)
}

pub fn service_uninstall(
    config_dir: &Path,
    place: &Place,
    package: &str,
    loader: &mut dyn Loader,
) -> Result<Value> {
    if !valid_package_name(package) {
        return Err(CliError::usage(format!("{package:?} is not a plugin name")));
    }
    let mut records = Records::load(config_dir)?;
    let name = place.names.service(place.manager, package);
    let unit = place.path(&name);
    let record = records.services.remove(package);
    if record.is_none() && !unit.exists() {
        return Err(CliError::failed(format!(
            "no service installed for {package}"
        )));
    }
    if place.load {
        loader.unload(&place.unit(&name, false))?;
    }
    remove_file(&unit)?;
    reload_after_remove(place);
    let dir = config_dir.join(SERVICES_DIR).join(package);
    match fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(CliError::failed(format!("{}: {e}", dir.display()))),
    }
    records.save(config_dir)?;
    Ok(json!({"package": package, "name": name, "path": unit, "removed": true}))
}

/// The records, each with whether its unit file is still there and its binary unchanged.
pub fn service_list(config_dir: &Path) -> Result<Value> {
    let records = Records::load(config_dir)?;
    let services: Vec<Value> = records
        .services
        .values()
        .map(|r| {
            let mut v = json!(r);
            v["unitPresent"] = json!(r.unit.exists());
            v["binUnchanged"] = json!(sha256_file(&r.bin).ok().as_deref() == Some(&r.sha256));
            v
        })
        .collect();
    let engine = records.engine.as_ref().map(|r| {
        let mut v = json!(r);
        v["unitPresent"] = json!(r.unit.exists());
        v
    });
    Ok(json!({"engine": engine, "services": services}))
}

// -- Commands -------------------------------------------------------------------------------

pub fn print(ctx: &Ctx, v: &Value, text: &[String]) {
    if ctx.json {
        output::json(v);
    } else {
        for line in text {
            output::line(line);
        }
    }
}

pub fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn load_note(v: &Value, done: &str) -> String {
    if v["load"].as_bool() == Some(true) {
        format!("{done} and started it")
    } else {
        format!("{done}; not loaded (--unit-dir, --no-load or AIRPOST_SERVICE_NO_LOAD)")
    }
}

pub fn install_command(ctx: &Ctx, place: &PlaceArgs, args: &ServiceArgs) -> Result<()> {
    let place = Place::resolve(&ctx.config_dir, place)?;
    let v = service_install(&ctx.config_dir, &place, args, &mut System)?;
    let mut text = vec![
        format!("Plugin:   {} ({})", s(&v["package"]), s(&v["source"])),
        format!("Binary:   {}", s(&v["bin"])),
        format!("SHA-256:  {}", s(&v["sha256"])),
        format!("Copied to {}", s(&v["installedBin"])),
        format!("Plan:     {} (--expect-plan)", s(&v["planSha256"])),
    ];
    if v["engine"]["installed"].as_bool() == Some(false) {
        text.push(format!(
            "Engine unit {} (runs {}):",
            s(&v["engine"]["path"]),
            s(&v["engine"]["bin"])
        ));
        text.push(s(&v["engine"]["text"]));
    }
    text.push(format!("Unit {}:", s(&v["unit"]["path"])));
    text.push(s(&v["unit"]["text"]));
    text.push(if args.dry_run {
        "Dry run: nothing written.".to_owned()
    } else {
        load_note(&v, "Installed the service")
    });
    print(ctx, &v, &text);
    Ok(())
}

pub fn uninstall_command(ctx: &Ctx, place: &PlaceArgs, package: &str) -> Result<()> {
    let place = Place::resolve(&ctx.config_dir, place)?;
    let v = service_uninstall(&ctx.config_dir, &place, package, &mut System)?;
    print(ctx, &v, &[format!("Removed {}", s(&v["path"]))]);
    Ok(())
}

pub fn list_command(ctx: &Ctx) -> Result<()> {
    let v = service_list(&ctx.config_dir)?;
    let mut text = Vec::new();
    match &v["engine"] {
        Value::Null => text.push("Engine unit: not installed".to_owned()),
        e => text.push(format!(
            "Engine unit: {} ({})",
            s(&e["name"]),
            s(&e["unit"])
        )),
    }
    for svc in v["services"].as_array().into_iter().flatten() {
        let mut problems = Vec::new();
        if svc["unitPresent"].as_bool() != Some(true) {
            problems.push("unit file missing");
        }
        if svc["binUnchanged"].as_bool() != Some(true) {
            problems.push("binary changed since install");
        }
        text.push(format!(
            "{}  {}  sha256 {}{}",
            s(&svc["package"]),
            s(&svc["bin"]),
            s(&svc["sha256"]),
            if problems.is_empty() {
                String::new()
            } else {
                format!("  ({})", problems.join(", "))
            }
        ));
    }
    if v["services"].as_array().is_none_or(Vec::is_empty) {
        text.push("No services installed.".to_owned());
    }
    print(ctx, &v, &text);
    Ok(())
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// Records what would have been loaded; nothing reaches launchctl or systemctl.
    #[derive(Default)]
    pub struct Fake {
        pub calls: Vec<String>,
    }

    impl Fake {
        fn push(&mut self, verb: &str, u: &UnitRef) -> Result<()> {
            let socket = if u.socket { " (socket)" } else { "" };
            self.calls
                .push(format!("{verb} {} {}{socket}", u.manager.as_str(), u.name));
            Ok(())
        }
    }

    impl Loader for Fake {
        fn load(&mut self, u: &UnitRef) -> Result<()> {
            self.push("load", u)
        }
        fn enable(&mut self, u: &UnitRef) -> Result<()> {
            self.push("enable", u)
        }
        fn unload(&mut self, u: &UnitRef) -> Result<()> {
            self.push("unload", u)
        }
        fn start(&mut self, u: &UnitRef, restart: bool) -> Result<()> {
            self.push(if restart { "restart" } else { "start" }, u)
        }
        fn stop(&mut self, u: &UnitRef) -> Result<()> {
            self.push("stop", u)
        }
    }

    pub fn executable(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        set_mode(path, 0o755).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{executable, Fake};
    use super::*;
    use crate::builtin::{self, Kind};

    fn engine_install(
        config: &Path,
        place: &Place,
        bin: Option<&Path>,
        key_file: Option<&Path>,
        dry_run: bool,
        loader: &mut dyn Loader,
    ) -> Result<Value> {
        let opts = builtin::Options {
            bin: bin.map(Path::to_owned),
            key_file: key_file.map(Path::to_owned),
            dry_run,
            ..builtin::Options::default()
        };
        builtin::install(Kind::Engine, config, place, &opts, loader)
    }

    fn engine_uninstall(config: &Path, place: &Place, loader: &mut dyn Loader) -> Result<Value> {
        builtin::uninstall(Kind::Engine, config, place, loader)
    }

    fn engine_start(place: &Place, restart: bool, loader: &mut dyn Loader) -> Result<Value> {
        builtin::start(Kind::Engine, place, restart, loader)
    }

    struct Env {
        _tmp: tempfile::TempDir,
        config: PathBuf,
        units: PathBuf,
        engine: PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let config = root.join("profile");
        let units = root.join("units");
        let engine = root.join("bin").join("airpost-engine");
        executable(&engine, "#!/bin/sh\nexit 0\n");
        Env {
            _tmp: tmp,
            config,
            units,
            engine,
        }
    }

    fn package(env: &Env, name: &str, service: Value) -> PathBuf {
        let dir = env.config.join("packages").join(name);
        fs::create_dir_all(&dir).unwrap();
        let manifest = json!({
            "name": name,
            "engines": {"airpost": "^1.27.0"},
            "airpost": {"service": service},
        });
        fs::write(dir.join("package.json"), manifest.to_string()).unwrap();
        dir
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

    fn args(name: &str, env: &Env) -> ServiceArgs {
        ServiceArgs {
            package: name.into(),
            engine_bin: Some(env.engine.clone()),
            ..Default::default()
        }
    }

    #[test]
    fn manifest_takes_a_path_or_one_per_platform() {
        let env = env();
        let dir = package(&env, "p", json!({"bin": "bin/svc", "args": ["-q"]}));
        executable(&dir.join("bin/svc"), "x");
        let m = read_manifest(&dir, "linux-x64").unwrap();
        assert_eq!(m.bin, dir.join("bin/svc"));
        assert_eq!(m.args, ["-q"]);

        let dir = package(
            &env,
            "q",
            json!({"bin": {"darwin-arm64": "mac/svc", "linux-x64": "linux/svc"}}),
        );
        executable(&dir.join("mac/svc"), "x");
        executable(&dir.join("linux/svc"), "x");
        assert_eq!(
            read_manifest(&dir, "darwin-arm64").unwrap().bin,
            dir.join("mac/svc")
        );
        let e = read_manifest(&dir, "win32-x64").unwrap_err();
        assert!(e.contains("darwin-arm64, linux-x64"), "{e}");
    }

    #[test]
    fn manifest_refuses_binaries_outside_the_package_and_bad_blocks() {
        let env = env();
        let outside = env.config.join("outside");
        executable(&outside, "x");
        let dir = package(&env, "p", json!({"bin": "../../outside"}));
        assert!(read_manifest(&dir, "linux-x64")
            .unwrap_err()
            .contains("outside the package"));
        let dir = package(&env, "abs", json!({"bin": outside}));
        assert!(read_manifest(&dir, "linux-x64")
            .unwrap_err()
            .contains("inside the package"));
        #[cfg(unix)]
        {
            let dir = package(&env, "link", json!({"bin": "svc"}));
            std::os::unix::fs::symlink(&outside, dir.join("svc")).unwrap();
            assert!(read_manifest(&dir, "linux-x64")
                .unwrap_err()
                .contains("outside the package"));
            let dir = package(&env, "noexec", json!({"bin": "svc"}));
            fs::write(dir.join("svc"), "x").unwrap();
            assert!(read_manifest(&dir, "linux-x64")
                .unwrap_err()
                .contains("not an executable"));
        }
        let dir = package(&env, "args", json!({"bin": "svc", "args": [1]}));
        assert!(read_manifest(&dir, "linux-x64")
            .unwrap_err()
            .contains("list of strings"));
        let dir = env.config.join("packages").join("none");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("package.json"), r#"{"name":"none"}"#).unwrap();
        assert!(read_manifest(&dir, "linux-x64")
            .unwrap_err()
            .contains("no service"));
    }

    #[test]
    fn dry_run_shows_the_binary_and_units_and_writes_nothing() {
        let env = env();
        let dir = package(&env, "sample", json!({"bin": "svc", "args": ["--x"]}));
        executable(&dir.join("svc"), "service v1");
        let place = place(&env, Manager::Launchd);
        let mut fake = Fake::default();
        let mut a = args("sample", &env);
        a.dry_run = true;
        let v = service_install(&env.config, &place, &a, &mut fake).unwrap();
        assert_eq!(v["installed"], false);
        assert_eq!(v["bin"], json!(dir.join("svc")));
        assert_eq!(v["sha256"], sha256_file(&dir.join("svc")).unwrap());
        assert_eq!(
            v["installedBin"],
            json!(env.config.join("services/sample/svc"))
        );
        assert_eq!(v["unit"]["name"], "com.airpost.airpost.service.sample");
        let text = v["unit"]["text"].as_str().unwrap();
        assert!(text.contains(&format!(
            "<string>{}</string>",
            env.config.join("services/sample/svc").display()
        )));
        assert!(text.contains("<key>AIRPOST_SERVICE_PACKAGE</key>\n\t\t<string>sample</string>"));
        assert_eq!(v["engine"]["installed"], false);
        assert!(v["engine"]["text"]
            .as_str()
            .unwrap()
            .contains("<string>--detached</string>"));
        assert!(!env.units.exists());
        assert!(!env.config.join(RECORDS_FILE).exists());
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn install_copies_the_binary_records_it_and_writes_both_units() {
        let env = env();
        let dir = package(&env, "sample", json!({"bin": "svc"}));
        executable(&dir.join("svc"), "service v1");
        let hash = sha256_file(&dir.join("svc")).unwrap();
        let mut place = place(&env, Manager::Systemd);
        let mut fake = Fake::default();
        let mut a = args("sample", &env);
        a.expect_sha256 = Some(hash.to_uppercase());
        let v = service_install(&env.config, &place, &a, &mut fake).unwrap();
        assert_eq!(v["installed"], true);

        let copy = env.config.join("services/sample/svc");
        assert_eq!(fs::read_to_string(&copy).unwrap(), "service v1");
        let unit = fs::read_to_string(env.units.join("airpost-service-sample.service")).unwrap();
        assert!(unit.contains("After=airpost-engine.service\n"));
        assert!(unit.contains(&format!("ExecStart=\"{}\"\n", copy.display())));
        let engine = fs::read_to_string(env.units.join("airpost-engine.service")).unwrap();
        assert!(engine.contains(&format!(
            "ExecStart=\"{}\" \"--mode\" \"serve\" \"--detached\"",
            env.engine.display()
        )));
        assert!(engine.contains(&format!(
            "Environment=\"CONFIG_DIR_PATH={}\"",
            env.config.display()
        )));

        let records = Records::load(&env.config).unwrap();
        let r = &records.services["sample"];
        assert_eq!((r.sha256.as_str(), &r.source), (hash.as_str(), &dir));
        assert_eq!(r.source_bin, dir.join("svc"));
        assert_eq!(records.engine.as_ref().unwrap().bin, env.engine);
        assert!(fake.calls.is_empty(), "nothing loads outside the unit dir");

        // A changed plugin binary isn't what the unit runs until it is installed again, and
        // the list says the copy is intact.
        executable(&dir.join("svc"), "service v2");
        let listed = service_list(&env.config).unwrap();
        assert_eq!(listed["services"][0]["binUnchanged"], true);
        assert_eq!(listed["services"][0]["unitPresent"], true);
        let e = service_install(&env.config, &place, &a, &mut fake).unwrap_err();
        assert!(e.to_string().contains("changed"), "{e}");
        assert_eq!(fs::read_to_string(&copy).unwrap(), "service v1");

        // Loading goes engine first, then the service; an existing engine unit is kept.
        place.load = true;
        a.expect_sha256 = None;
        service_install(&env.config, &place, &a, &mut fake).unwrap();
        assert_eq!(fake.calls, ["load systemd airpost-service-sample"]);
        assert_eq!(fs::read_to_string(&copy).unwrap(), "service v2");
    }

    #[test]
    fn a_missing_engine_unit_is_loaded_before_the_service() {
        let env = env();
        let dir = package(&env, "sample", json!({"bin": "svc"}));
        executable(&dir.join("svc"), "x");
        let mut place = place(&env, Manager::Launchd);
        place.load = true;
        let mut fake = Fake::default();
        service_install(&env.config, &place, &args("sample", &env), &mut fake).unwrap();
        assert_eq!(
            fake.calls,
            [
                "load launchd com.airpost.airpost.engine",
                "load launchd com.airpost.airpost.service.sample"
            ]
        );
    }

    #[test]
    fn uninstall_removes_the_unit_copy_and_record_and_keeps_the_engine() {
        let env = env();
        let dir = package(&env, "sample", json!({"bin": "svc"}));
        executable(&dir.join("svc"), "x");
        let mut place = place(&env, Manager::Launchd);
        let mut fake = Fake::default();
        service_install(&env.config, &place, &args("sample", &env), &mut fake).unwrap();
        let e = engine_uninstall(&env.config, &place, &mut fake).unwrap_err();
        assert!(e.to_string().contains("sample"), "{e}");

        place.load = true;
        service_uninstall(&env.config, &place, "sample", &mut fake).unwrap();
        assert!(!env
            .units
            .join("com.airpost.airpost.service.sample.plist")
            .exists());
        assert!(!env.config.join("services/sample").exists());
        assert!(Records::load(&env.config).unwrap().services.is_empty());
        assert!(env.units.join("com.airpost.airpost.engine.plist").exists());
        assert!(service_uninstall(&env.config, &place, "sample", &mut fake).is_err());
        assert!(service_uninstall(&env.config, &place, "../x", &mut fake).is_err());

        engine_uninstall(&env.config, &place, &mut fake).unwrap();
        assert!(!env.units.join("com.airpost.airpost.engine.plist").exists());
        assert!(Records::load(&env.config).unwrap().engine.is_none());
        assert_eq!(
            fake.calls,
            [
                "unload launchd com.airpost.airpost.service.sample",
                "unload launchd com.airpost.airpost.engine"
            ]
        );
    }

    #[test]
    fn install_refuses_a_plan_that_changed_after_the_dry_run() {
        let env = env();
        let dir = package(&env, "sample", json!({"bin": "svc", "args": ["--a"]}));
        executable(&dir.join("svc"), "service v1");
        let place = place(&env, Manager::Launchd);
        let mut fake = Fake::default();
        let mut a = args("sample", &env);
        a.dry_run = true;
        let shown = service_install(&env.config, &place, &a, &mut fake).unwrap();
        let plan = shown["planSha256"].as_str().unwrap().to_owned();
        assert_eq!(plan.len(), 64);
        a.dry_run = false;
        a.expect_sha256 = Some(shown["sha256"].as_str().unwrap().to_owned());
        a.expect_plan = Some(plan.clone());

        // Same binary, other args: the unit would differ from the one shown.
        package(&env, "sample", json!({"bin": "svc", "args": ["--b"]}));
        let e = service_install(&env.config, &place, &a, &mut fake).unwrap_err();
        assert!(e.to_string().contains("changed since it was shown"), "{e}");
        // Another engine binary for the engine unit.
        package(&env, "sample", json!({"bin": "svc", "args": ["--a"]}));
        let other_engine = env.engine.with_file_name("airpost-engine-other");
        executable(&other_engine, "#!/bin/sh\nexit 1\n");
        let mut b = a.clone();
        b.engine_bin = Some(other_engine);
        assert!(service_install(&env.config, &place, &b, &mut fake).is_err());
        assert!(!env.units.exists());
        assert!(!env.config.join(SERVICES_DIR).exists());
        assert!(!env.config.join(RECORDS_FILE).exists());

        // The plan shown, unchanged, installs.
        let v = service_install(&env.config, &place, &a, &mut fake).unwrap();
        assert_eq!(
            (v["installed"].as_bool(), v["planSha256"].as_str()),
            (Some(true), Some(plan.as_str()))
        );
        // With the engine unit there now, the same plan is no longer the one a new dry run
        // would show: that one leaves the engine unit alone.
        let e = service_install(&env.config, &place, &a, &mut fake).unwrap_err();
        assert!(e.to_string().contains("changed since it was shown"), "{e}");
    }

    #[test]
    fn engine_start_and_restart_go_to_the_installed_unit_only() {
        let env = env();
        let mut place = place(&env, Manager::Launchd);
        let mut fake = Fake::default();
        assert!(engine_start(&place, true, &mut fake)
            .unwrap_err()
            .to_string()
            .contains("not installed"));
        engine_install(
            &env.config,
            &place,
            Some(&env.engine),
            None,
            false,
            &mut fake,
        )
        .unwrap();
        let v = engine_start(&place, true, &mut fake).unwrap();
        assert_eq!(v["load"], false);
        assert!(fake.calls.is_empty(), "nothing loads outside the unit dir");
        place.load = true;
        engine_start(&place, true, &mut fake).unwrap();
        engine_start(&place, false, &mut fake).unwrap();
        place.manager = Manager::Systemd;
        engine_install(
            &env.config,
            &place,
            Some(&env.engine),
            None,
            false,
            &mut fake,
        )
        .unwrap();
        engine_start(&place, true, &mut fake).unwrap();
        assert_eq!(
            fake.calls,
            [
                "restart launchd com.airpost.airpost.engine",
                "start launchd com.airpost.airpost.engine",
                "load systemd airpost-engine",
                "restart systemd airpost-engine"
            ]
        );
    }

    #[test]
    fn engine_install_takes_a_key_file() {
        let env = env();
        let place = place(&env, Manager::Systemd);
        let key = env.config.join("engine.key");
        let v = engine_install(
            &env.config,
            &place,
            Some(&env.engine),
            Some(&key),
            false,
            &mut Fake::default(),
        )
        .unwrap();
        assert_eq!(v["installed"], true);
        let unit = fs::read_to_string(env.units.join("airpost-engine.service")).unwrap();
        assert!(unit.contains(&format!(
            "Environment=\"AIRPOST_ENGINE_KEY_FILE={}\"",
            key.display()
        )));
        assert_eq!(
            Records::load(&env.config).unwrap().engine.unwrap().sha256,
            sha256_file(&env.engine).unwrap()
        );
    }

    #[test]
    fn packages_are_found_by_their_manifest_name() {
        let env = env();
        let dev = env.config.join("dev/packages/checkout-dir");
        fs::create_dir_all(&dev).unwrap();
        fs::write(dev.join("package.json"), r#"{"name":"devpkg"}"#).unwrap();
        assert_eq!(find_package(&env.config, "devpkg").unwrap(), dev);
        assert!(find_package(&env.config, "missing").is_err());
        let dir = package(&env, "real", json!({"bin": "svc"}));
        executable(&dir.join("svc"), "x");
        let mut a = args("real", &env);
        a.package_dir = Some(dev);
        let e = ServicePlan::new(&env.config, &place(&env, Manager::Launchd), &a).unwrap_err();
        assert!(e.to_string().contains("devpkg"), "{e}");
    }

    #[test]
    fn units_load_only_from_the_managers_own_directory() {
        let env = env();
        let given = PlaceArgs {
            unit_dir: Some(env.units.clone()),
            manager: Some(Manager::Systemd),
            no_load: false,
            no_start: false,
        };
        assert!(!Place::resolve(&env.config, &given).unwrap().load);
        let own = PlaceArgs {
            unit_dir: None,
            manager: Some(Manager::Systemd),
            no_load: true,
            no_start: false,
        };
        assert!(!Place::resolve(&env.config, &own).unwrap().load);
        assert!(valid_package_name("a.b_c-1"));
        assert!(!valid_package_name("a/b") && !valid_package_name("..") && !valid_package_name(""));
        assert!(
            ["darwin-arm64", "darwin-x64", "linux-x64", "linux-arm64"]
                .contains(&platform_key().as_str())
                || cfg!(not(any(target_os = "macos", target_os = "linux")))
        );
    }
}
