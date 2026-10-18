// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The running engines, through `tern-client`'s discovery and router, and the [`Backend`]
//! seam the tools call through so the tests can run them over in-memory data.

use std::path::Path;

use serde_json::{Map, Value};

pub use tern_client::{AccountRef, EngineError, Engines};

/// What the tools need from the engines: the accounts they hold, a way to call a method for
/// one of them, and the features each one announces.
pub trait Backend {
    fn accounts(&self) -> Vec<AccountRef>;
    fn call(&mut self, account_id: &str, method: &str, params: Value)
        -> Result<Value, EngineError>;
    /// Whether the engine holding `account_id` lists `feature` in `hello.features`.
    fn has_feature(&self, account_id: &str, feature: &str) -> bool;
    /// The stored values of `keys` from the engine that keeps the settings, or `None` when
    /// no engine does (they are then only in the desktop's `config.json`).
    fn settings(&mut self, keys: &[&str]) -> Option<Map<String, Value>>;
}

/// Connects to every live engine under `config_dir`. `client_name` goes in `hello`.
pub fn connect(config_dir: &Path, client_name: &str) -> Engines {
    Engines::connect(config_dir, client_name, "tern-mcp")
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

impl Backend for Engines {
    fn accounts(&self) -> Vec<AccountRef> {
        Engines::accounts(self)
    }

    fn call(
        &mut self,
        account_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, EngineError> {
        Engines::call(self, account_id, method, params)
    }

    fn has_feature(&self, account_id: &str, feature: &str) -> bool {
        self.engines
            .iter()
            .find(|e| e.holds(account_id))
            .is_some_and(|e| e.has_feature(feature))
    }

    fn settings(&mut self, keys: &[&str]) -> Option<Map<String, Value>> {
        let engine = self.settings_engine()?;
        let keys = keys.iter().map(|k| (*k).to_owned()).collect();
        // An engine that fails the call keeps nothing tern-mcp can use.
        Some(
            engine
                .settings_get(Some(keys))
                .map(|v| v.values)
                .unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_engine_is_a_clear_message() {
        let tmp = tempfile::tempdir().unwrap();
        let e = connect(tmp.path(), "test");
        assert!(e.engines.is_empty());
        let m = no_engine_message(tmp.path(), &e.problems);
        assert!(m.starts_with("Tern's mail engine isn't running"), "{m}");
    }
}
