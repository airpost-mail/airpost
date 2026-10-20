// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Finding the running engines (docs/engine-protocol-plan.md 1.5): every engine writes
//! `<config-dir>/run/<name>.json` with its pid and socket once it listens, and removes it on
//! a clean exit. A per-account engine (`sync --socket`) writes `run/<accountId>.json`; the
//! one engine of `serve` mode writes `run/engine.json`, listing its accounts. Either way the
//! accounts are read from each engine's `hello`. Entries whose pid is gone are left behind by
//! a crash and skipped.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::platform;

/// The snap's name in `apps/desktop/snap/snapcraft.yaml`. A snap install keeps its profile
/// in `$SNAP_USER_COMMON` (`~/snap/<name>/common`), which a client outside the snap can't see
/// in its environment.
const SNAP_NAME: &str = "tern";

/// The discovery file of a `serve` engine.
pub const ENGINE_FILE: &str = "engine.json";

/// Where the desktop keeps its profile on this OS (`setupConfigDir` in the desktop's
/// `src/main/main.js`): Electron's appData plus `Tern`, then on Linux the snap's
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
    /// Live entries with their file: `engine.json` first, so a shared engine wins over
    /// per-account entries left behind, then by file name.
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
    files.sort_by_key(|f| (f.file_name().is_none_or(|n| n != ENGINE_FILE), f.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn scan_reads_both_shapes_and_skips_stale_pids_and_garbage() {
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
        assert_eq!(s.live[0].0, run.join("engine.json"));
        assert_eq!(s.live[1].0, run.join("acc-a.json"));
        assert_eq!(s.live[0].1.account_id, None);
        assert_eq!(
            s.live[0].1.accounts,
            Some(vec!["acc-c".to_owned(), "acc-d".to_owned()])
        );
        assert_eq!(
            s.live[1].1,
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
}
