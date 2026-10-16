// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The profile directory and what tern-mcp reads from the desktop's `config.json`: the
//! accounts' display fields (name, provider, aliases, auto-cc), the default sending
//! account, and the `core.mcp.*` keys used while there is no `mcp.json`. Credentials in
//! `config.json` are never read.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The snap's name in `apps/desktop/snap/snapcraft.yaml`; its profile is
/// `~/snap/<name>/common`.
const SNAP_NAME: &str = "mailspring";

/// Where the desktop keeps its profile on this OS, as `apps/cli` resolves it: Electron's
/// appData plus `Tern`, then on Linux the snap's directory.
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

/// The profile without `--config-dir`: the first default dir with a live engine, else the
/// first that exists, else the first.
pub fn default_config_dir() -> Option<PathBuf> {
    let dirs = default_config_dirs();
    dirs.iter()
        .find(|d| !crate::discovery::scan(&d.join("run")).live.is_empty())
        .or_else(|| dirs.iter().find(|d| d.is_dir()))
        .or_else(|| dirs.first())
        .cloned()
}

/// An account as the desktop stores it in `config.json`, without its settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountMeta {
    pub id: String,
    pub name: String,
    pub email: String,
    pub provider: Option<String>,
    /// `Name <email>` strings.
    pub aliases: Vec<String>,
    pub default_alias: Option<String>,
    /// `autoaddress`: `("cc" | "bcc", addresses)`.
    pub autoaddress: Option<(String, String)>,
}

/// The parts of the desktop's `config.json` tern-mcp uses. A missing or unreadable file
/// reads as empty: the desktop may never have run in this profile (a harness engine).
#[derive(Clone, Debug, Default)]
pub struct DesktopConfig {
    /// The `"*"` scope, where `AppEnv.config` keeps every key.
    root: Value,
}

impl DesktopConfig {
    pub fn load(config_dir: &Path) -> DesktopConfig {
        let root = std::fs::read(config_dir.join("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|mut v| v.get_mut("*").map(Value::take))
            .unwrap_or(Value::Null);
        DesktopConfig { root }
    }

    #[cfg(test)]
    pub fn from_value(root: Value) -> DesktopConfig {
        DesktopConfig { root }
    }

    /// A dotted key path such as `core.mcp.accessLevel`.
    pub fn get(&self, path: &str) -> Option<&Value> {
        let mut v = &self.root;
        for part in path.split('.') {
            v = v.get(part)?;
        }
        (!v.is_null()).then_some(v)
    }

    pub fn accounts(&self) -> Vec<AccountMeta> {
        let Some(list) = self.get("accounts").and_then(Value::as_array) else {
            return Vec::new();
        };
        list.iter()
            .filter_map(|a| {
                let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_owned);
                let auto = a.get("autoaddress");
                Some(AccountMeta {
                    id: s("id")?,
                    name: s("name").unwrap_or_default(),
                    email: s("emailAddress").unwrap_or_default(),
                    provider: s("provider"),
                    aliases: a
                        .get("aliases")
                        .and_then(Value::as_array)
                        .map(|l| {
                            l.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                    default_alias: s("defaultAlias").filter(|d| !d.is_empty()),
                    autoaddress: auto.and_then(|v| {
                        let kind = v.get("type")?.as_str()?.to_owned();
                        let value = v.get("value")?.as_str()?.to_owned();
                        (!value.trim().is_empty()).then_some((kind, value))
                    }),
                })
            })
            .collect()
    }

    /// `core.sending.defaultAccountIdForSend`.
    pub fn default_send_account(&self) -> Option<String> {
        self.get("core.sending.defaultAccountIdForSend")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_accounts_without_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("config.json"),
            json!({"*": {
                "accounts": [{"id": "a1", "name": "Ann", "emailAddress": "ann@x.test",
                              "provider": "gmail", "aliases": ["Ann W <ann@w.test>"],
                              "autoaddress": {"type": "bcc", "value": "log@x.test"},
                              "settings": {"imap_password": "secret"}}],
                "core": {"sending": {"defaultAccountIdForSend": "a1"}}
            }})
            .to_string(),
        )
        .unwrap();
        let c = DesktopConfig::load(tmp.path());
        let [a] = c.accounts().try_into().unwrap();
        assert_eq!(a.email, "ann@x.test");
        assert_eq!(a.provider.as_deref(), Some("gmail"));
        assert_eq!(a.aliases, ["Ann W <ann@w.test>"]);
        assert_eq!(
            a.autoaddress,
            Some(("bcc".to_owned(), "log@x.test".to_owned()))
        );
        assert_eq!(c.default_send_account().as_deref(), Some("a1"));
        assert!(DesktopConfig::load(&tmp.path().join("none"))
            .accounts()
            .is_empty());
    }
}
