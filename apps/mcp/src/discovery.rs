// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Finding the running engines (docs/engine-protocol-plan.md 1.5) and calling them. Every
//! engine writes `<config-dir>/run/<name>.json` once it listens: `<accountId>.json` per
//! account today, `engine.json` with an `accounts` list once one process holds every account
//! (phase C). Both are read; an account claimed by two live engines goes to the first.
//!
//! The scan is the same as `apps/cli/src/discovery.rs`; a shared crate should replace both.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ms_ipc::{code, Client, ClientError, HelloResult, PROTOCOL_VERSION};
use serde::Deserialize;
use serde_json::Value;

/// How long a connect and `hello` may take before the engine counts as hung.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// Any other call. `task.queue` answers after the local phase, which can wait out the
/// store's 60 s busy timeout.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// A discovery file's content.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub pid: u32,
    pub socket: PathBuf,
    #[serde(default)]
    pub protocol: u32,
    /// Per-account engines name their account.
    #[serde(default)]
    pub account_id: Option<String>,
    /// A shared engine lists the accounts it holds.
    #[serde(default)]
    pub accounts: Option<Vec<String>>,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub live: Vec<(PathBuf, Entry)>,
    pub stale: Vec<PathBuf>,
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
    // engine.json first, so a shared engine wins over leftover per-account entries.
    files.sort_by_key(|f| (f.file_name().is_none_or(|n| n != "engine.json"), f.clone()));
    for file in files {
        let parsed = std::fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<Entry>(&b).map_err(|e| e.to_string()));
        match parsed {
            Ok(entry) if crate::platform::pid_alive(entry.pid) => out.live.push((file, entry)),
            Ok(_) => out.stale.push(file),
            Err(e) => out.unreadable.push((file, e)),
        }
    }
    out
}

/// An account some engine holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRef {
    pub account_id: String,
    pub email: String,
}

/// A failed engine call.
#[derive(Debug)]
pub enum EngineError {
    /// No engine is running, or none holds the account.
    NoEngine(String),
    /// The engine answered with an error.
    Rpc { code: i64, message: String },
    /// The connection failed or the engine sent something unreadable.
    Transport(String),
}

impl EngineError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, EngineError::Rpc { code: c, .. } if *c == code::NOT_FOUND)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::NoEngine(m) | EngineError::Transport(m) => f.write_str(m),
            EngineError::Rpc { message, .. } => f.write_str(message),
        }
    }
}

impl From<ClientError> for EngineError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Rpc(o) => EngineError::Rpc {
                code: o.code,
                message: o.message,
            },
            other => EngineError::Transport(format!("the engine connection failed: {other}")),
        }
    }
}

/// What the tools need from the engines: the accounts they hold and a way to call a method
/// for one of them. Tests implement it over in-memory data.
pub trait Backend {
    fn accounts(&self) -> Vec<AccountRef>;
    fn call(&mut self, account_id: &str, method: &str, params: Value)
        -> Result<Value, EngineError>;
}

pub struct Engine {
    pub file: PathBuf,
    pub entry: Entry,
    pub hello: HelloResult,
    client: Client,
}

/// Every live engine under one profile, connected.
pub struct Engines {
    pub engines: Vec<Engine>,
    /// Engines that have a discovery file but couldn't be reached, as messages.
    pub problems: Vec<String>,
}

fn connect(entry: &Entry, client_name: &str) -> Result<(Client, HelloResult), String> {
    let mut client = Client::connect(&entry.socket).map_err(|e| e.to_string())?;
    let hello = client
        .set_read_timeout(Some(HELLO_TIMEOUT))
        .and_then(|()| client.hello(client_name));
    let hello = match hello {
        Ok(h) => h,
        Err(ClientError::Rpc(e)) if e.code == code::UNSUPPORTED_PROTOCOL => {
            return Err(format!(
                "it speaks protocol {} and tern-mcp speaks {PROTOCOL_VERSION}; update tern-mcp or \
                 Tern so they match",
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

impl Engines {
    /// Connects to every live engine under `config_dir`. `client_name` goes in `hello`.
    pub fn connect(config_dir: &Path, client_name: &str) -> Engines {
        let scan = scan(&config_dir.join("run"));
        let mut problems: Vec<String> = scan
            .unreadable
            .iter()
            .map(|(f, e)| format!("{}: not a discovery file ({e})", f.display()))
            .collect();
        let mut engines = Vec::new();
        for (file, entry) in scan.live {
            match connect(&entry, client_name) {
                Ok((client, hello)) => engines.push(Engine {
                    file,
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
        Engines { engines, problems }
    }

    /// The error a tool returns when there is no engine to talk to.
    pub fn no_engine_message(config_dir: &Path, problems: &[String]) -> String {
        let mut msg = format!(
            "Tern's mail engine isn't running, so there is no mail to read: open Tern and try \
             again (tern-mcp looked for engines in {}).",
            config_dir.join("run").display()
        );
        for p in problems {
            msg.push_str(&format!(" {p}."));
        }
        msg
    }

    fn engine_for(&mut self, account_id: &str) -> Option<&mut Engine> {
        self.engines
            .iter_mut()
            .find(|e| e.hello.accounts.iter().any(|a| a.account_id == account_id))
    }
}

impl Backend for Engines {
    fn accounts(&self) -> Vec<AccountRef> {
        let mut out: Vec<AccountRef> = Vec::new();
        for e in &self.engines {
            for a in &e.hello.accounts {
                if !out.iter().any(|o| o.account_id == a.account_id) {
                    out.push(AccountRef {
                        account_id: a.account_id.clone(),
                        email: a.email.clone(),
                    });
                }
            }
        }
        out
    }

    fn call(
        &mut self,
        account_id: &str,
        method: &str,
        mut params: Value,
    ) -> Result<Value, EngineError> {
        let Some(engine) = self.engine_for(account_id) else {
            return Err(EngineError::NoEngine(format!(
                "no running engine holds account {account_id}"
            )));
        };
        if let Value::Object(o) = &mut params {
            o.insert("accountId".into(), Value::from(account_id));
        }
        Ok(engine.client.call(method, params)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scan_reads_both_entry_shapes_and_skips_dead_pids() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path();
        let me = std::process::id();
        std::fs::write(
            run.join("acc-a.json"),
            json!({"pid": me, "socket": "/tmp/a.sock", "protocol": 1, "accountId": "acc-a"})
                .to_string(),
        )
        .unwrap();
        std::fs::write(
            run.join("engine.json"),
            json!({"pid": me, "socket": "/tmp/e.sock", "protocol": 1, "accounts": ["x", "y"]})
                .to_string(),
        )
        .unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        std::fs::write(
            run.join("dead.json"),
            json!({"pid": dead, "socket": "/tmp/d.sock"}).to_string(),
        )
        .unwrap();
        std::fs::write(run.join("junk.json"), "{").unwrap();

        let s = scan(run);
        let names: Vec<_> = s
            .live
            .iter()
            .map(|(f, _)| f.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["engine.json", "acc-a.json"]);
        assert_eq!(
            s.live[0].1.accounts.as_deref(),
            Some(&["x".to_owned(), "y".to_owned()][..])
        );
        assert_eq!(s.live[1].1.account_id.as_deref(), Some("acc-a"));
        assert_eq!(s.stale, [run.join("dead.json")]);
        assert_eq!(s.unreadable.len(), 1);
    }

    #[test]
    fn no_engine_is_a_clear_message() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engines::connect(tmp.path(), "test");
        assert!(e.engines.is_empty());
        let m = Engines::no_engine_message(tmp.path(), &e.problems);
        assert!(m.starts_with("Tern's mail engine isn't running"), "{m}");
    }
}
