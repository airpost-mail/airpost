// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The running engines, found and greeted through `airpost-client`,
//! and the accounts a command acts on.

use std::path::Path;

use airpost_ipc::AccountInfo;

pub use airpost_client::discovery::default_config_dir;
pub use airpost_client::Engine;

use crate::error::{CliError, Result};

const CLIENT_NAME: &str = concat!("airpost-cli/", env!("CARGO_PKG_VERSION"));

/// The account an engine holds, by index into the engine list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub engine: usize,
    pub account_id: String,
    pub email: String,
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
    let airpost_client::Engines {
        engines, problems, ..
    } = airpost_client::Engines::connect(config_dir, CLIENT_NAME, "airpost");
    if engines.is_empty() {
        let mut msg = format!(
            "no engine running: start Airpost, or check --config-dir or AIRPOST_CONFIG_DIR (looked in \
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
            "no running engine holds account {want}: start Airpost, or check --account \
             (`airpost accounts` lists the running ones)"
        ))),
        (None, true) => Err(CliError::NoEngine(
            "the running engines hold no accounts: add one in Airpost".into(),
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
    use airpost_client::discovery::Entry;
    use airpost_ipc::{Client, HelloResult};
    use serde_json::json;
    use std::path::PathBuf;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn no_engine_is_exit_3_and_says_what_to_do() {
        let tmp = tempfile::tempdir().unwrap();
        let e = connect_all(tmp.path()).err().unwrap();
        assert_eq!(e.exit_code(), 3);
        assert!(e
            .to_string()
            .starts_with("no engine running: start Airpost"));

        // A live pid whose socket isn't there is reported, not connected.
        std::fs::create_dir(tmp.path().join("run")).unwrap();
        let entry = json!({"pid": std::process::id(), "socket": tmp.path().join("gone.sock")});
        write(&tmp.path().join("run"), "x.json", &entry.to_string());
        let e = connect_all(tmp.path()).err().unwrap();
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().contains("gone.sock"), "{e}");
    }

    fn engine(pid: u32, accounts: &[(&str, &str)]) -> Engine {
        Engine {
            file: PathBuf::from("/tmp/run/engine.json"),
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
                        ..Default::default()
                    })
                    .collect(),
                features: Vec::new(),
                capabilities: None,
                ..Default::default()
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
            ..Default::default()
        };
        assert!(matches(&a, "abc"));
        assert!(matches(&a, "me@example.test"));
        assert!(!matches(&a, "ab"));
        assert!(!matches(&a, "you@example.test"));
    }
}
