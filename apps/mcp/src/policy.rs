// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The access policy, `<config-dir>/mcp.json`, with the semantics of the desktop plugin's
//! `mcp-access-control.ts`:
//!
//! - `accessLevel`: `read-only` (the default), `read-write` or `read-write-send`. Read tools
//!   always run; write tools need `read-write` or more; send tools `read-write-send`. Any
//!   other value allows reads only.
//! - `enabledAccounts`: `{accountId: {enabled, excludedFolderIds}}`. Empty means every
//!   account is enabled. An account that isn't enabled is invisible: tools treat its ids as
//!   unknown. A thread or message with a copy in an excluded folder or label is blocked, even
//!   when it is also somewhere allowed.
//! - `http`: `{enabled, port}` for `tern-mcp http`; off by default. Over HTTP, `http.enabled`
//!   is what turns every tool on or off ([`Policy::for_http`]).
//! - `enabled`: `false` turns every tool off for stdio clients (the desktop's "Local apps"
//!   switch). Absent means on, so files written before the switch keep working.
//!
//! Without `mcp.json`, the desktop's `core.mcp.accessLevel` and `core.mcp.enabledAccounts`
//! in `config.json` are read instead, so settings made in the old preferences pane keep
//! applying; HTTP stays off in that case.
//! The policy is read again for every tool call. `TERN_MCP_POLICY` names another file in place
//! of `<config-dir>/mcp.json`, for a server whose operator keeps the policy read-only outside
//! the profile (deploy/server).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::DesktopConfig;

pub const POLICY_FILE: &str = "mcp.json";
pub const ENV_POLICY: &str = "TERN_MCP_POLICY";
pub const DEFAULT_HTTP_PORT: u16 = 2587;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Read,
    Write,
    Send,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Read => "read",
            Category::Write => "write",
            Category::Send => "send",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub excluded_folder_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    DEFAULT_HTTP_PORT
}

fn default_true() -> bool {
    true
}

impl Default for HttpPolicy {
    fn default() -> Self {
        HttpPolicy {
            enabled: false,
            port: DEFAULT_HTTP_PORT,
        }
    }
}

/// Where the policy came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    PolicyFile(PathBuf),
    /// `config.json`'s `core.mcp.*`, while there is no `mcp.json`.
    DesktopConfig,
    /// Neither: the defaults.
    Defaults,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolicyFile {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    access_level: Option<String>,
    #[serde(default)]
    enabled_accounts: BTreeMap<String, AccountPolicy>,
    #[serde(default)]
    http: HttpPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub enabled: bool,
    /// As configured; [`Policy::check_access`] treats an unknown value as read-only.
    pub access_level: String,
    pub enabled_accounts: BTreeMap<String, AccountPolicy>,
    pub http: HttpPolicy,
    pub source: Source,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            enabled: true,
            access_level: "read-only".into(),
            enabled_accounts: BTreeMap::new(),
            http: HttpPolicy::default(),
            source: Source::Defaults,
        }
    }
}

/// The policy file: [`ENV_POLICY`] when set, else `<config-dir>/mcp.json`.
pub fn policy_path(config_dir: &Path) -> PathBuf {
    policy_path_with(config_dir, std::env::var_os(ENV_POLICY))
}

fn policy_path_with(config_dir: &Path, env: Option<std::ffi::OsString>) -> PathBuf {
    match env.filter(|v| !v.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => config_dir.join(POLICY_FILE),
    }
}

impl Policy {
    /// Reads `mcp.json`, else the desktop's keys. A `mcp.json` that doesn't parse is an
    /// error rather than a fallback, so a typo can't widen access.
    pub fn load(config_dir: &Path, desktop: &DesktopConfig) -> Result<Policy, String> {
        let path = policy_path(config_dir);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let f: PolicyFile = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("{} is not a valid policy file: {e}", path.display()))?;
                Ok(Policy {
                    enabled: f.enabled,
                    access_level: f.access_level.unwrap_or_else(|| "read-only".into()),
                    enabled_accounts: f.enabled_accounts,
                    http: f.http,
                    source: Source::PolicyFile(path),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::from_desktop(desktop),
            Err(e) => Err(format!("could not read {}: {e}", path.display())),
        }
    }

    fn from_desktop(desktop: &DesktopConfig) -> Result<Policy, String> {
        let level = desktop.get("core.mcp.accessLevel").and_then(Value::as_str);
        let accounts = match desktop.get("core.mcp.enabledAccounts") {
            None | Some(Value::Null) => None,
            Some(v) => Some(desktop_accounts(v).map_err(|e| {
                format!("config.json's core.mcp.enabledAccounts is not valid ({e})")
            })?),
        };
        if level.is_none() && accounts.is_none() {
            return Ok(Policy::default());
        }
        Ok(Policy {
            enabled: true,
            access_level: level.unwrap_or("read-only").to_owned(),
            enabled_accounts: accounts.unwrap_or_default(),
            http: HttpPolicy::default(),
            source: Source::DesktopConfig,
        })
    }

    /// The policy for a call that came over HTTP: the HTTP switch stands in for `enabled`, so
    /// the two transports are turned on and off separately, and a server still running after
    /// HTTP was turned off refuses every call.
    pub fn for_http(mut self) -> Policy {
        self.enabled = self.http.enabled;
        self
    }

    /// `checkAccessLevel`: `Err` with the message the plugin returned.
    pub fn check_access(&self, category: Category) -> Result<(), String> {
        if !self.enabled {
            return Err(
                "Access denied: apps on this computer are not allowed to use Tern. \
                        Turn it on in Tern's Preferences > MCP."
                    .into(),
            );
        }
        let level = self.access_level.as_str();
        let ok = match category {
            Category::Read => true,
            Category::Write => matches!(level, "read-write" | "read-write-send"),
            Category::Send => level == "read-write-send",
        };
        if ok {
            return Ok(());
        }
        let needed = if category == Category::Send {
            "read-write-send"
        } else {
            "read-write"
        };
        Err(format!(
            "Access denied: '{}' operations require access level '{needed}', but current level \
             is '{level}'.",
            category.name()
        ))
    }

    pub fn account_allowed(&self, account_id: &str) -> bool {
        self.enabled
            && (self.enabled_accounts.is_empty()
                || self
                    .enabled_accounts
                    .get(account_id)
                    .is_some_and(|a| a.enabled))
    }

    fn excluded(&self, account_id: &str) -> &[String] {
        self.enabled_accounts
            .get(account_id)
            .map(|a| a.excluded_folder_ids.as_slice())
            .unwrap_or(&[])
    }

    pub fn folder_allowed(&self, account_id: &str, folder_id: &str) -> bool {
        self.account_allowed(account_id)
            && !self.excluded(account_id).iter().any(|f| f == folder_id)
    }

    /// `isThreadAllowed` over the stored Thread JSON: any excluded folder or label blocks.
    pub fn thread_allowed(&self, thread: &Value) -> bool {
        let account = crate::model::str_of(thread, "aid");
        if !self.account_allowed(account) {
            return false;
        }
        let excluded = self.excluded(account);
        excluded.is_empty()
            || !crate::model::thread_category_ids(thread).any(|id| excluded.iter().any(|e| e == id))
    }

    /// `isMessageAllowed` over the stored Message JSON: any copy in an excluded folder blocks.
    pub fn message_allowed(&self, message: &Value) -> bool {
        let account = crate::model::str_of(message, "aid");
        if !self.account_allowed(account) {
            return false;
        }
        let excluded = self.excluded(account);
        excluded.is_empty()
            || !crate::model::message_folder_ids(message).any(|id| excluded.iter().any(|e| e == id))
    }

    pub fn summary(&self) -> Value {
        let source = match &self.source {
            Source::PolicyFile(p) => p.display().to_string(),
            Source::DesktopConfig => "config.json (core.mcp.*)".into(),
            Source::Defaults => "defaults".into(),
        };
        serde_json::json!({
            "source": source,
            "enabled": self.enabled,
            "accessLevel": self.access_level,
            "enabledAccounts": self.enabled_accounts,
            "http": self.http,
        })
    }
}

/// `core.mcp.enabledAccounts` read the way the plugin read it, where a null `enabled` or
/// `excludedFolderIds` is false or `[]`. Anything else that doesn't fit is an error, so a
/// value the plugin would have read as a restriction never turns into "every account".
fn desktop_accounts(v: &Value) -> Result<BTreeMap<String, AccountPolicy>, String> {
    let Value::Object(map) = v else {
        return Err("not an object".into());
    };
    let mut out = BTreeMap::new();
    for (id, a) in map {
        let policy = match a {
            Value::Null => AccountPolicy::default(),
            Value::Object(a) => {
                let enabled = match a.get("enabled") {
                    None | Some(Value::Null) => false,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => return Err(format!("{id}.enabled is not a boolean")),
                };
                let excluded_folder_ids = match a.get("excludedFolderIds") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(ids)) => ids
                        .iter()
                        .map(|f| f.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| format!("{id}.excludedFolderIds has a non-string id"))?,
                    Some(_) => return Err(format!("{id}.excludedFolderIds is not an array")),
                };
                AccountPolicy {
                    enabled,
                    excluded_folder_ids,
                }
            }
            _ => return Err(format!("{id} is not an object")),
        };
        out.insert(id.clone(), policy);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_env_names_another_policy_file() {
        let dir = Path::new("/profile");
        assert_eq!(policy_path_with(dir, None), dir.join(POLICY_FILE));
        assert_eq!(
            policy_path_with(dir, Some("".into())),
            dir.join(POLICY_FILE)
        );
        assert_eq!(
            policy_path_with(dir, Some("/etc/tern/mcp.json".into())),
            PathBuf::from("/etc/tern/mcp.json")
        );
    }

    fn policy(v: Value) -> Policy {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(POLICY_FILE), v.to_string()).unwrap();
        Policy::load(tmp.path(), &DesktopConfig::default()).unwrap()
    }

    #[test]
    fn access_levels() {
        let p = Policy::default();
        assert!(p.check_access(Category::Read).is_ok());
        let e = p.check_access(Category::Write).unwrap_err();
        assert_eq!(
            e,
            "Access denied: 'write' operations require access level 'read-write', but current \
             level is 'read-only'."
        );
        let p = policy(json!({"accessLevel": "read-write"}));
        assert!(p.check_access(Category::Write).is_ok());
        assert!(p
            .check_access(Category::Send)
            .unwrap_err()
            .contains("'read-write-send'"));
        let p = policy(json!({"accessLevel": "read-write-send"}));
        assert!(p.check_access(Category::Send).is_ok());
        let p = policy(json!({"accessLevel": "everything"}));
        assert!(p.check_access(Category::Read).is_ok());
        assert!(p.check_access(Category::Write).is_err());
    }

    #[test]
    fn accounts_and_folder_exclusion() {
        let p = Policy::default();
        assert!(p.account_allowed("any"));
        let p = policy(json!({"enabledAccounts": {
            "a": {"enabled": true, "excludedFolderIds": ["secret"]},
            "b": {"enabled": false}
        }}));
        assert!(p.account_allowed("a"));
        assert!(!p.account_allowed("b"));
        assert!(!p.account_allowed("c"));
        assert!(p.folder_allowed("a", "inbox"));
        assert!(!p.folder_allowed("a", "secret"));
        assert!(!p.folder_allowed("b", "inbox"));

        let thread = |cats: &[&str]| {
            json!({"aid": "a", "folders": cats.iter().map(|c| json!({"id": c})).collect::<Vec<_>>(),
                   "labels": []})
        };
        assert!(p.thread_allowed(&thread(&["inbox"])));
        // Exclusion wins over an allowed category on the same thread.
        assert!(!p.thread_allowed(&thread(&["inbox", "secret"])));
        let msg = json!({"aid": "a", "folders": {"inbox": 0, "secret": 1}});
        assert!(!p.message_allowed(&msg));
        assert!(p.message_allowed(&json!({"aid": "a", "folders": {"inbox": 0}})));
        assert!(!p.message_allowed(&json!({"aid": "b", "folders": {}})));
    }

    #[test]
    fn enabled_false_turns_every_tool_and_account_off() {
        assert!(policy(json!({})).enabled);
        let p = policy(json!({"enabled": false, "accessLevel": "read-write-send"}));
        let e = p.check_access(Category::Read).unwrap_err();
        assert!(e.contains("Preferences > MCP"), "{e}");
        assert!(p.check_access(Category::Send).is_err());
        assert!(!p.account_allowed("a"));
        assert!(!p.thread_allowed(&json!({"aid": "a", "folders": [], "labels": []})));
        assert_eq!(p.summary()["enabled"], false);
    }

    #[test]
    fn http_calls_follow_the_http_switch() {
        let p = policy(json!({"enabled": false, "http": {"enabled": true}})).for_http();
        assert!(p.check_access(Category::Read).is_ok());
        assert!(p.account_allowed("a"));
        let p = policy(json!({"http": {"enabled": false}})).for_http();
        assert!(p.check_access(Category::Read).is_err());
        assert!(!p.account_allowed("a"));
    }

    #[test]
    fn falls_back_to_the_desktop_keys_and_fails_closed_on_a_bad_file() {
        let tmp = tempfile::tempdir().unwrap();
        let desktop = DesktopConfig::from_value(json!({"core": {"mcp": {
            "accessLevel": "read-write", "enabled": true, "port": 9999,
            "enabledAccounts": {"a": {"enabled": true}}
        }}}));
        let p = Policy::load(tmp.path(), &desktop).unwrap();
        assert_eq!(p.source, Source::DesktopConfig);
        assert_eq!(p.access_level, "read-write");
        assert!(p.account_allowed("a") && !p.account_allowed("b"));
        // The plugin's own HTTP server owns core.mcp.enabled/port.
        assert!(!p.http.enabled);

        assert_eq!(
            Policy::load(tmp.path(), &DesktopConfig::default())
                .unwrap()
                .source,
            Source::Defaults
        );
        // Nulls read as the plugin read them, and keep the restriction.
        let nulls = DesktopConfig::from_value(json!({"core": {"mcp": {"enabledAccounts": {
            "work": {"enabled": true, "excludedFolderIds": null},
            "home": {"enabled": null}, "old": null
        }}}}));
        let p = Policy::load(tmp.path(), &nulls).unwrap();
        assert!(p.account_allowed("work") && p.folder_allowed("work", "x"));
        assert!(!p.account_allowed("home") && !p.account_allowed("old"));
        assert!(!p.account_allowed("other"));
        // Anything else that doesn't fit fails closed, as a bad mcp.json does.
        for bad in [
            json!({"work": {"enabled": true, "excludedFolderIds": "secret"}}),
            json!({"work": {"enabled": "yes"}}),
            json!({"work": {"enabled": true, "excludedFolderIds": [1]}}),
            json!({"work": true}),
            json!(["work"]),
        ] {
            let d = DesktopConfig::from_value(json!({"core": {"mcp": {"enabledAccounts": bad}}}));
            let e = Policy::load(tmp.path(), &d).unwrap_err();
            assert!(e.contains("core.mcp.enabledAccounts"), "{e}");
        }

        std::fs::write(tmp.path().join(POLICY_FILE), "{not json").unwrap();
        assert!(Policy::load(tmp.path(), &desktop).is_err());
        std::fs::write(
            tmp.path().join(POLICY_FILE),
            json!({"http": {"enabled": true}}).to_string(),
        )
        .unwrap();
        let p = Policy::load(tmp.path(), &desktop).unwrap();
        assert_eq!(p.access_level, "read-only");
        assert_eq!(
            p.http,
            HttpPolicy {
                enabled: true,
                port: DEFAULT_HTTP_PORT
            }
        );
    }
}
