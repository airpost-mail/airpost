// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Finding the running engines (docs/engine-protocol-plan.md 1.5): every engine writes
//! `<config-dir>/run/<name>.json` with its pid and socket once it listens, and removes it on
//! a clean exit. A per-account engine (`sync --socket`) writes `run/<accountId>.json`; the
//! one engine of `serve` mode writes `run/engine.json`, listing its accounts. Either way the
//! accounts are read from each engine's `hello`. Entries whose pid is gone are left behind by
//! a crash and skipped.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ms_ipc::{code, AccountInfo, Client, ClientError, HelloResult, PROTOCOL_VERSION};
use serde::Deserialize;

use crate::error::{CliError, Result};
use crate::platform;

/// How long a connect and `hello` may take before the engine counts as hung.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long any other call may take. `task.queue` answers after the local phase, which can
/// wait out the store's 60 s busy timeout.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// The snap's name in `apps/desktop/snap/snapcraft.yaml`. A snap install keeps its profile
/// in `$SNAP_USER_COMMON` (`~/snap/<name>/common`), which a CLI outside the snap can't see
/// in its environment.
const SNAP_NAME: &str = "mailspring";

/// Where the desktop keeps its profile on this OS (`setupConfigDir` in the desktop's
/// `src/browser/main.js`): Electron's appData plus `Tern`, then on Linux the snap's
/// directory.
pub fn default_config_dirs() -> Vec<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let app_data = if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    let mut dirs: Vec<PathBuf> = app_data.into_iter().map(|d| d.join("Tern")).collect();
    if cfg!(target_os = "linux") {
        if let Some(home) = env("HOME") {
            dirs.push(home.join("snap").join(SNAP_NAME).join("common"));
        }
    }
    dirs
}

/// The profile to use without `--config-dir`: the first of [`default_config_dirs`] with a
/// live engine, else the first that exists, else the first.
pub fn default_config_dir() -> Option<PathBuf> {
    let dirs = default_config_dirs();
    dirs.iter()
        .find(|d| !scan(&d.join("run")).live.is_empty())
        .or_else(|| dirs.iter().find(|d| d.is_dir()))
        .or_else(|| dirs.first())
        .cloned()
}

/// A discovery file's content.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub pid: u32,
    pub socket: PathBuf,
    #[serde(default)]
    pub protocol: u32,
    /// Per-account engines (phase A) name their account; a shared engine doesn't.
    #[serde(default)]
    pub account_id: Option<String>,
    /// A shared engine (`serve`, `run/engine.json`) lists the accounts it holds.
    #[serde(default)]
    pub accounts: Option<Vec<String>>,
}

#[derive(Debug, Default)]
pub struct Scan {
    /// Live entries, by file name.
    pub live: Vec<(PathBuf, Entry)>,
    /// Entries whose process is gone.
    pub stale: Vec<PathBuf>,
    /// Files that aren't a discovery entry, with the reason.
    pub unreadable: Vec<(PathBuf, String)>,
}

/// Reads `run_dir/*.json`. A missing directory is an empty scan.
pub fn scan(run_dir: &Path) -> Scan {
    let mut out = Scan::default();
    let Ok(dir) = std::fs::read_dir(run_dir) else {
        return out;
    };
    let mut files: Vec<PathBuf> = dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for file in files {
        let parsed = std::fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<Entry>(&b).map_err(|e| e.to_string()));
        match parsed {
            Ok(entry) if platform::pid_alive(entry.pid) => out.live.push((file, entry)),
            Ok(_) => out.stale.push(file),
            Err(e) => out.unreadable.push((file, e)),
        }
    }
    out
}

/// A connected engine.
pub struct Engine {
    pub entry: Entry,
    pub hello: HelloResult,
    pub client: Client,
}

/// The account an engine holds, by index into the engine list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub engine: usize,
    pub account_id: String,
    pub email: String,
}

fn connect(entry: &Entry) -> std::result::Result<(Client, HelloResult), String> {
    let mut client = Client::connect(&entry.socket).map_err(|e| e.to_string())?;
    let hello = client
        .set_read_timeout(Some(HELLO_TIMEOUT))
        .and_then(|()| client.hello(concat!("tern-cli/", env!("CARGO_PKG_VERSION"))));
    let hello = match hello {
        Ok(h) => h,
        Err(ClientError::Rpc(e)) if e.code == code::UNSUPPORTED_PROTOCOL => {
            return Err(format!(
            "it speaks protocol {} and this tern speaks {PROTOCOL_VERSION}; update tern or Tern \
                 so they match",
            entry.protocol
        ))
        }
        Err(e) if e.is_timeout() => return Err("no answer to hello; the engine may be hung".into()),
        Err(e) => return Err(e.to_string()),
    };
    client
        .set_read_timeout(Some(CALL_TIMEOUT))
        .map_err(|e| e.to_string())?;
    Ok((client, hello))
}

/// Connects to every live engine under `config_dir`. Engines that can't be reached are
/// reported on stderr; none at all is [`CliError::NoEngine`].
pub fn connect_all(config_dir: &Path) -> Result<Vec<Engine>> {
    connect_all_with(config_dir, true)
}

/// [`connect_all`] without the warnings, for lookups that have another way to succeed.
pub fn connect_all_quiet(config_dir: &Path) -> Result<Vec<Engine>> {
    connect_all_with(config_dir, false)
}

fn connect_all_with(config_dir: &Path, warn: bool) -> Result<Vec<Engine>> {
    let run_dir = config_dir.join("run");
    let scan = scan(&run_dir);
    let mut problems: Vec<String> = scan
        .unreadable
        .iter()
        .map(|(f, e)| format!("{}: not a discovery file ({e})", f.display()))
        .collect();
    let mut engines = Vec::new();
    for (file, entry) in scan.live {
        match connect(&entry) {
            Ok((client, hello)) => engines.push(Engine {
                entry,
                hello,
                client,
            }),
            Err(e) => problems.push(format!(
                "{}: engine pid {} at {}: {e}",
                file.display(),
                entry.pid,
                entry.socket.display()
            )),
        }
    }
    if engines.is_empty() {
        let mut msg = format!(
            "no engine running: start Tern, or check --config-dir or TERN_CONFIG_DIR (looked in \
             {})",
            run_dir.display()
        );
        for p in &problems {
            msg.push_str(&format!("\n  {p}"));
        }
        return Err(CliError::NoEngine(msg));
    }
    for p in problems.iter().filter(|_| warn) {
        crate::output::warn(&format!("skipped {p}"));
    }
    Ok(engines)
}

/// Whether `account` (an id, or an email address in any case) names `info`.
pub fn matches(info: &AccountInfo, account: &str) -> bool {
    info.account_id == account || info.email.eq_ignore_ascii_case(account)
}

/// The accounts to act on: the one `account` names, or every account.
pub fn targets(engines: &[Engine], account: Option<&str>) -> Result<Vec<Target>> {
    let all = engines
        .iter()
        .enumerate()
        .flat_map(|(i, e)| e.hello.accounts.iter().map(move |a| (i, a)));
    let picked: Vec<Target> = all
        .filter(|(_, a)| account.is_none_or(|want| matches(a, want)))
        .map(|(engine, a)| Target {
            engine,
            account_id: a.account_id.clone(),
            email: a.email.clone(),
        })
        .collect();
    match (account, picked.is_empty()) {
        (Some(want), true) => Err(CliError::NoEngine(format!(
            "no running engine holds account {want}: start Tern, or check --account \
             (`tern accounts` lists the running ones)"
        ))),
        (None, true) => Err(CliError::NoEngine(
            "the running engines hold no accounts: add one in Tern".into(),
        )),
        _ => Ok(picked),
    }
}

/// The one account a command acts on: `account`, or the only account there is.
pub fn one_target(engines: &[Engine], account: Option<&str>) -> Result<Target> {
    let mut t = targets(engines, account)?;
    if t.len() > 1 {
        let names: Vec<&str> = t.iter().map(|t| t.email.as_str()).collect();
        return Err(CliError::usage(format!(
            "several accounts are running ({}); pick one with --account",
            names.join(", ")
        )));
    }
    Ok(t.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn scan_skips_stale_pids_and_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path();
        let live = json!({"pid": std::process::id(), "socket": "/tmp/a.sock", "protocol": 1,
                          "accountId": "acc-a"});
        write(run, "acc-a.json", &live.to_string());
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                &["/C", "exit"][..]
            } else {
                &[][..]
            })
            .spawn()
            .unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        let dead = json!({"pid": dead_pid, "socket": "/tmp/b.sock", "protocol": 1});
        write(run, "acc-b.json", &dead.to_string());
        let shared = json!({"pid": std::process::id(), "socket": "/tmp/engine.sock",
                            "protocol": 1, "accounts": ["acc-c", "acc-d"]});
        write(run, "engine.json", &shared.to_string());
        write(run, "garbage.json", "{not json");
        write(run, "wrong-shape.json", r#"{"socket": "/tmp/c.sock"}"#);
        write(run, "acc-a.sock", "");
        write(run, "notes.txt", "{}");

        let s = scan(run);
        assert_eq!(s.live.len(), 2);
        assert_eq!(s.live[0].0, run.join("acc-a.json"));
        assert_eq!(s.live[1].0, run.join("engine.json"));
        assert_eq!(s.live[1].1.account_id, None);
        assert_eq!(
            s.live[1].1.accounts,
            Some(vec!["acc-c".to_owned(), "acc-d".to_owned()])
        );
        assert_eq!(
            s.live[0].1,
            Entry {
                pid: std::process::id(),
                socket: "/tmp/a.sock".into(),
                protocol: 1,
                account_id: Some("acc-a".into()),
                accounts: None,
            }
        );
        if cfg!(unix) {
            assert_eq!(s.stale, [run.join("acc-b.json")]);
        }
        let bad: Vec<_> = s.unreadable.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(
            bad,
            [run.join("garbage.json"), run.join("wrong-shape.json")]
        );

        assert!(scan(&run.join("missing")).live.is_empty());
    }

    #[test]
    fn no_engine_is_exit_3_and_says_what_to_do() {
        let tmp = tempfile::tempdir().unwrap();
        let e = connect_all(tmp.path()).err().unwrap();
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().starts_with("no engine running: start Tern"));

        // A live pid whose socket isn't there is reported, not connected.
        std::fs::create_dir(tmp.path().join("run")).unwrap();
        let entry = json!({"pid": std::process::id(), "socket": tmp.path().join("gone.sock")});
        write(&tmp.path().join("run"), "x.json", &entry.to_string());
        let e = connect_all(tmp.path()).err().unwrap();
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().contains("gone.sock"), "{e}");
    }

    #[test]
    fn default_config_dir_is_the_production_profile() {
        let dirs = default_config_dirs();
        assert_eq!(dirs[0].file_name().unwrap(), "Tern");
        if cfg!(target_os = "macos") {
            assert!(dirs[0].ends_with("Library/Application Support/Tern"));
        }
        if cfg!(target_os = "linux") {
            assert!(dirs[1].ends_with("snap/mailspring/common"));
        }
        assert!(default_config_dir().is_some_and(|d| dirs.contains(&d)));
    }

    fn engine(pid: u32, accounts: &[(&str, &str)]) -> Engine {
        Engine {
            entry: Entry {
                pid,
                socket: PathBuf::from("/tmp/x.sock"),
                protocol: 1,
                account_id: None,
                accounts: None,
            },
            hello: HelloResult {
                protocol: 1,
                engine: "0.1.0".into(),
                pid,
                accounts: accounts
                    .iter()
                    .map(|(id, email)| AccountInfo {
                        account_id: (*id).into(),
                        email: (*email).into(),
                        state: "running".into(),
                        error: None,
                    })
                    .collect(),
            },
            client: Client::from_streams(Box::new(std::io::empty()), Box::new(std::io::sink())),
        }
    }

    #[test]
    fn targets_fan_out_or_pick_one() {
        let engines = [
            engine(1, &[("a1", "ann@x.test")]),
            engine(2, &[("b1", "bob@x.test"), ("b2", "bea@x.test")]),
        ];
        let all = targets(&engines, None).unwrap();
        let ids: Vec<(usize, &str)> = all
            .iter()
            .map(|t| (t.engine, t.account_id.as_str()))
            .collect();
        assert_eq!(ids, [(0, "a1"), (1, "b1"), (1, "b2")]);
        assert_eq!(
            one_target(&engines, Some("BEA@x.test")).unwrap().account_id,
            "b2"
        );
        assert_eq!(one_target(&engines, None).unwrap_err().exit_code(), 2);
        assert_eq!(
            targets(&engines, Some("nobody")).unwrap_err().exit_code(),
            3
        );
        assert_eq!(one_target(&engines[..1], None).unwrap().email, "ann@x.test");
    }

    #[test]
    fn accounts_match_by_id_or_email() {
        let a = AccountInfo {
            account_id: "abc".into(),
            email: "Me@Example.test".into(),
            state: "running".into(),
            error: None,
        };
        assert!(matches(&a, "abc"));
        assert!(matches(&a, "me@example.test"));
        assert!(!matches(&a, "ab"));
        assert!(!matches(&a, "you@example.test"));
    }
}
