// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `serve`'s registered accounts and their secrets:
//! `<config>/accounts.json` holds the non-secret Account JSON, the [`SecretStore`] the
//! passwords and refresh tokens. An account comes back together only when it starts.
//!
//! The store opens on first use, so an engine with no registered account never touches a
//! keychain. Debug builds default to the file store keyed by `<config>/engine-dev.key`: an
//! unsigned binary would make the keychain ask after every rebuild.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::{json, Map, Value};
use tern_ipc::{
    CredentialsImportParams, CredentialsImportResult, CredentialsStatus, ImportFailure,
};
use tern_secrets::{names, SecretStore, Selection};
use tracing::{info, warn};

pub const ACCOUNTS_FILE: &str = "accounts.json";
/// The Tern server of `server.set`; its token is the secret `tern-server-token`.
pub const SERVER_FILE: &str = "server.json";
/// The key file debug builds use when the environment names no store.
pub const DEV_KEY_FILE: &str = "engine-dev.key";

/// A secret's store name for an account id.
type SecretName = fn(&str) -> String;

/// The settings that are secrets, with the store name each goes to.
const SECRET_SETTINGS: [(&str, SecretName); 3] = [
    ("imap_password", names::account_imap),
    ("smtp_password", names::account_smtp),
    ("refresh_token", names::account_refresh_token),
];

/// The keyset suffix KeyManager gave each secret setting (`<email>-imap`, ...).
const KEYSET_SUFFIXES: [(&str, &str); 3] = [
    ("imap_password", "-imap"),
    ("smtp_password", "-smtp"),
    ("refresh_token", "-refresh-token"),
];

pub struct Registry {
    config_dir: PathBuf,
    selection: Result<Selection, String>,
    store: OnceLock<Result<SecretStore, String>>,
    /// Non-secret Account JSON, in the order the accounts were first registered.
    accounts: Mutex<Vec<Value>>,
}

impl Registry {
    pub fn open(config_dir: &Path) -> Self {
        let dev_key = cfg!(debug_assertions).then(|| config_dir.join(DEV_KEY_FILE));
        let selection = tern_secrets::select(config_dir, &|k| std::env::var(k).ok(), dev_key)
            .map_err(|e| e.to_string());
        Self::with_selection(config_dir, selection)
    }

    pub fn with_selection(config_dir: &Path, selection: Result<Selection, String>) -> Self {
        Registry {
            config_dir: config_dir.to_owned(),
            selection,
            store: OnceLock::new(),
            accounts: Mutex::new(load_accounts(&config_dir.join(ACCOUNTS_FILE))),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Value>> {
        self.accounts.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn store(&self) -> Result<&SecretStore, String> {
        self.store
            .get_or_init(|| {
                let selection = self.selection.clone()?;
                let store = SecretStore::open(&selection).map_err(|e| e.to_string())?;
                info!("Secret store: {}", store.kind().as_str());
                Ok(store)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// `hello.credentials`, without touching the store.
    pub fn status(&self) -> CredentialsStatus {
        CredentialsStatus {
            store: self
                .selection
                .as_ref()
                .ok()
                .map(|s| s.kind().as_str().to_owned()),
            accounts: self.lock().len(),
            error: match (&self.selection, self.store.get()) {
                (Err(e), _) | (_, Some(Err(e))) => Some(e.clone()),
                _ => None,
            },
        }
    }

    /// The registered accounts' non-secret JSON.
    pub fn accounts(&self) -> Vec<Value> {
        self.lock().clone()
    }

    /// The registered accounts' ids, in their order.
    pub fn ids(&self) -> Vec<String> {
        self.lock()
            .iter()
            .filter_map(|a| account_id(a).map(str::to_owned))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Value> {
        self.lock()
            .iter()
            .find(|a| account_id(a) == Some(id))
            .cloned()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// `account` with its stored secrets in place of any it lacks, when it is registered
    /// with the same servers and usernames ([`same_servers`]). An account that names other
    /// servers gets none: a client that can reach the engine must not be able to send a
    /// stored password or token somewhere else.
    /// Returns the account and whether it is registered with other servers.
    pub fn with_secrets_for_same_servers(&self, account: Value) -> Result<(Value, bool), String> {
        let Some(id) = account_id(&account) else {
            return Ok((account, false));
        };
        match self.get(id) {
            Some(registered) if same_servers(&registered, &account) => {
                Ok((self.with_secrets(account)?, false))
            }
            Some(_) => Ok((account, true)),
            None => Ok((account, false)),
        }
    }

    /// `account` with the secrets the store holds for it in place of any it lacks.
    pub fn with_secrets(&self, mut account: Value) -> Result<Value, String> {
        let Some(id) = account_id(&account).map(str::to_owned) else {
            return Err("the account has no id".into());
        };
        let missing: Vec<_> = SECRET_SETTINGS
            .iter()
            .filter(|(k, _)| setting_str(&account, k).is_none())
            .collect();
        if missing.is_empty() {
            return Ok(account);
        }
        let store = self.store()?;
        if let Some(settings) = account.get_mut("settings").and_then(Value::as_object_mut) {
            for (key, name) in missing {
                if let Some(v) = store.get(&name(&id)).map_err(|e| e.to_string())? {
                    settings.insert((*key).to_owned(), Value::String(v));
                }
            }
        }
        Ok(account)
    }

    /// Registers `account` (with or without secrets): the secrets it carries replace the
    /// stored ones, the rest goes to `accounts.json`. Returns the non-secret JSON and whether
    /// the registry changed. A store that can't take the secrets leaves the account
    /// unregistered.
    pub fn register(&self, account: &Value) -> Result<(Value, bool), String> {
        let id = account_id(account)
            .ok_or("the account has no id")?
            .to_owned();
        let secrets: Vec<(String, String)> = SECRET_SETTINGS
            .iter()
            .filter_map(|(k, name)| setting_str(account, k).map(|v| (name(&id), v.to_owned())))
            .collect();
        if !secrets.is_empty() {
            let prefix = names::account_prefix(&id);
            self.store()?
                .update(|keys| {
                    keys.retain(|n, _| !n.starts_with(&prefix));
                    keys.extend(secrets);
                })
                .map_err(|e| e.to_string())?;
        }
        let public = stored_form(account);
        let changed = self.upsert(&id, public.clone())?;
        Ok((public, changed))
    }

    /// Replaces a registered account's JSON with `public` (secrets left out), deleting its
    /// stored secrets when they were for other servers ([`same_servers`]). Returns whether
    /// they were deleted.
    pub fn update(&self, id: &str, public: &Value) -> Result<bool, String> {
        let public = stored_form(public);
        let Some(before) = self.get(id) else {
            return Err(format!("unknown account {id}"));
        };
        let moved = !same_servers(&before, &public);
        if moved {
            self.store()?
                .delete_prefix(&names::account_prefix(id))
                .map_err(|e| e.to_string())?;
        }
        self.upsert(id, public)?;
        Ok(moved)
    }

    /// Puts the named accounts first, in that order; the others follow as they were.
    pub fn reorder(&self, ids: &[String]) -> Result<(), String> {
        let mut accounts = self.lock();
        let mut next: Vec<Value> = ids
            .iter()
            .filter_map(|id| accounts.iter().find(|a| account_id(a) == Some(id)).cloned())
            .collect();
        for a in accounts.iter() {
            if !account_id(a).is_some_and(|id| ids.iter().any(|i| i == id)) {
                next.push(a.clone());
            }
        }
        if next == *accounts {
            return Ok(());
        }
        save_accounts(&self.config_dir.join(ACCOUNTS_FILE), &next)?;
        *accounts = next;
        Ok(())
    }

    /// A secret that isn't an account's, for the engine's own use (integration keys, the
    /// Tern server token). Never sent to a client.
    pub fn secret(&self, name: &str) -> Result<Option<String>, String> {
        self.store()?.get(name).map_err(|e| e.to_string())
    }

    pub fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        self.store()?.set(name, value).map_err(|e| e.to_string())
    }

    pub fn delete_secret(&self, name: &str) -> Result<(), String> {
        self.store()?.delete(name).map_err(|e| e.to_string())
    }

    /// [`Registry::delete_secret`], except that a store that can't be opened or read
    /// counts as holding nothing: the engine can't read a value from it to send anywhere,
    /// and refusing would leave the client unable to drop what the secret was for.
    pub fn forget_secret(&self, name: &str) -> Result<(), String> {
        match self.delete_secret(name) {
            Err(e) if self.secret(name).is_err() => {
                warn!("-X Could not delete the secret {name} from a store that can't be read: {e}");
                Ok(())
            }
            r => r,
        }
    }

    pub fn has_secrets(&self, names: &[String]) -> Result<BTreeMap<String, bool>, String> {
        self.store()?
            .has(names.iter().map(String::as_str))
            .map_err(|e| e.to_string())
    }

    /// The Tern server `server.set` stored, `{url, userId, serverId}`.
    pub fn server(&self) -> Option<Value> {
        let bytes = std::fs::read(self.config_dir.join(SERVER_FILE)).ok()?;
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .filter(Value::is_object)
    }

    pub fn set_server(&self, server: &Value) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(server).map_err(|e| e.to_string())?;
        let path = self.config_dir.join(SERVER_FILE);
        crate::platform::uds::write_private_file(&path, &text)
            .map_err(|e| format!("writing {}: {e}", path.display()))
    }

    /// Forgets the server and deletes its token.
    pub fn clear_server(&self) -> Result<(), String> {
        match std::fs::remove_file(self.config_dir.join(SERVER_FILE)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
        if let Ok(store) = self.store() {
            store
                .delete(names::TERN_SERVER_TOKEN)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn upsert(&self, id: &str, public: Value) -> Result<bool, String> {
        let mut accounts = self.lock();
        let mut next = accounts.clone();
        match next.iter_mut().find(|a| account_id(a) == Some(id)) {
            Some(existing) if *existing == public => return Ok(false),
            Some(existing) => *existing = public,
            None => next.push(public),
        }
        save_accounts(&self.config_dir.join(ACCOUNTS_FILE), &next)?;
        *accounts = next;
        Ok(true)
    }

    /// Stores a rotated refresh token of a registered account. False when the account isn't
    /// registered.
    pub fn store_refresh_token(&self, id: &str, token: &str) -> Result<bool, String> {
        if !self.contains(id) {
            return Ok(false);
        }
        self.store()?
            .set(&names::account_refresh_token(id), token)
            .map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Unregisters the account and deletes its secrets. False when it wasn't registered.
    pub fn forget(&self, id: &str) -> Result<bool, String> {
        let mut accounts = self.lock();
        let Some(at) = accounts.iter().position(|a| account_id(a) == Some(id)) else {
            return Ok(false);
        };
        if let Ok(store) = self.store() {
            store
                .delete_prefix(&names::account_prefix(id))
                .map_err(|e| e.to_string())?;
        }
        let mut next = accounts.clone();
        next.remove(at);
        save_accounts(&self.config_dir.join(ACCOUNTS_FILE), &next)?;
        *accounts = next;
        Ok(true)
    }

    /// `credentials.import`: registers each account with its secrets, taken from the account
    /// itself or else from the keyset's `<email>-*` entries (so several accounts with one
    /// email each get them), and stores `integration-<provider>-key` and
    /// `tern-server-token`. Accounts are not started.
    pub fn import(&self, p: CredentialsImportParams) -> CredentialsImportResult {
        let mut out = CredentialsImportResult::default();
        for mut account in p.accounts {
            let id = account_id(&account).unwrap_or_default().to_owned();
            let email = account
                .get("emailAddress")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let fail = |error: String| ImportFailure {
                account_id: id.clone(),
                email: email.clone(),
                error,
            };
            if let Some(settings) = account.get_mut("settings").and_then(Value::as_object_mut) {
                for (key, suffix) in KEYSET_SUFFIXES {
                    let has = settings
                        .get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty());
                    if !has {
                        if let Some(v) = p.secrets.get(&format!("{email}{suffix}")) {
                            settings.insert(key.to_owned(), Value::String(v.clone()));
                        }
                    }
                }
            }
            let checked = tern_core::Account::from_json(account.clone())
                .and_then(|a| a.validate().map(|()| a));
            match checked {
                Err(e) => out.failed.push(fail(e)),
                Ok(_) => match self.register(&account) {
                    Ok(_) => out.imported.push(id.clone()),
                    Err(e) => out.failed.push(fail(e)),
                },
            }
        }
        let others: BTreeMap<String, String> = p
            .secrets
            .iter()
            .filter_map(|(k, v)| other_secret_name(k).map(|n| (n, v.clone())))
            .collect();
        if !others.is_empty() {
            match self.store().and_then(|s| {
                s.update(|keys| keys.extend(others.clone()))
                    .map_err(|e| e.to_string())
            }) {
                Ok(()) => out.secrets = others.into_keys().collect(),
                Err(e) => warn!("-X Could not store the imported secrets: {e}"),
            }
        }
        info!(
            "Imported {} accounts ({} failed) and {} other secrets",
            out.imported.len(),
            out.failed.len(),
            out.secrets.len()
        );
        out
    }
}

/// The store name of a keyset entry that isn't an account's: `integration-<p>-key` becomes
/// `integration:<p>`; `tern-server-token` keeps its name.
fn other_secret_name(key: &str) -> Option<String> {
    if key == names::TERN_SERVER_TOKEN {
        return Some(key.to_owned());
    }
    let provider = key.strip_prefix("integration-")?.strip_suffix("-key")?;
    (!provider.is_empty()).then(|| names::integration(provider))
}

pub fn account_id(account: &Value) -> Option<&str> {
    account
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn setting_str<'a>(account: &'a Value, key: &str) -> Option<&'a str> {
    account
        .get("settings")
        .and_then(|s| s.get(key))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// The settings that say where an account's secrets go: its servers, usernames, how it
/// connects, and for OAuth the client the refresh token belongs to.
const CONNECTION_SETTINGS: [&str; 13] = [
    "refresh_client_id",
    "imap_host",
    "imap_port",
    "imap_username",
    "imap_security",
    "imap_allow_insecure_ssl",
    "imap_allow_cleartext",
    "smtp_host",
    "smtp_port",
    "smtp_username",
    "smtp_security",
    "smtp_allow_insecure_ssl",
    "smtp_allow_cleartext",
];

/// Whether `a` and `b` send their secrets to the same place: same provider and
/// [`CONNECTION_SETTINGS`]. A port given as a number or a string counts as the same.
pub fn same_servers(a: &Value, b: &Value) -> bool {
    let text = |v: Option<&Value>| match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(other) => other.to_string(),
    };
    let setting = |v: &Value, k: &str| text(v.get("settings").and_then(|s| s.get(k)));
    text(a.get("provider")) == text(b.get("provider"))
        && CONNECTION_SETTINGS
            .iter()
            .all(|k| setting(a, k) == setting(b, k))
}

/// Whether `account` carries any secret setting.
pub fn has_secrets(account: &Value) -> bool {
    SECRET_SETTINGS
        .iter()
        .any(|(k, _)| setting_str(account, k).is_some())
}

/// The secret settings `account` carries; an empty one counts as absent.
pub fn secrets_of(account: &Value) -> Vec<Option<&str>> {
    SECRET_SETTINGS
        .iter()
        .map(|(k, _)| setting_str(account, k))
        .collect()
}

/// `account` without its secret settings.
pub fn strip_secrets(account: &Value) -> Value {
    let mut out = account.clone();
    if let Some(settings) = out.get_mut("settings").and_then(Value::as_object_mut) {
        for (key, _) in SECRET_SETTINGS {
            settings.remove(key);
        }
    }
    out
}

/// Fields of the desktop's Account JSON that record its own view of the account's sync,
/// not the account: they are neither stored nor compared.
const CLIENT_STATE_FIELDS: [&str; 3] = ["syncState", "syncError", "sync_error"];

/// `account` as the registry keeps it: no secrets and no [`CLIENT_STATE_FIELDS`].
pub fn stored_form(account: &Value) -> Value {
    let mut out = strip_secrets(account);
    if let Some(fields) = out.as_object_mut() {
        for key in CLIENT_STATE_FIELDS {
            fields.remove(key);
        }
    }
    out
}

/// `account` with `patch` applied: its top-level fields replace the account's, and the keys
/// of its `settings` object replace the account's settings (null removes one). `id` and the
/// secrets can't be patched.
pub fn patched(account: &Value, patch: &Value) -> Result<Value, String> {
    let Some(patch) = patch.as_object() else {
        return Err("the patch must be an object".into());
    };
    let mut out = account.clone();
    let fields = out.as_object_mut().ok_or("the account is not an object")?;
    for (key, value) in patch {
        match key.as_str() {
            "id" => {
                if value.as_str() != fields.get("id").and_then(Value::as_str) {
                    return Err("an account's id can't change".into());
                }
            }
            "settings" => {
                let changes = value.as_object().ok_or("settings must be an object")?;
                let settings = fields
                    .entry("settings")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or("the account's settings are not an object")?;
                for (k, v) in changes {
                    if SECRET_SETTINGS.iter().any(|(s, _)| s == k) {
                        return Err(format!("{k} is a secret: use account.setSecrets"));
                    }
                    if v.is_null() {
                        settings.remove(k);
                    } else {
                        settings.insert(k.clone(), v.clone());
                    }
                }
            }
            _ if value.is_null() => {
                fields.remove(key);
            }
            _ => {
                fields.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(out)
}

/// Whether a secret name may be written with `secrets.set` / `secrets.delete`: lowercase
/// words and dashes, optionally `:` and an id (`integration:deepl`, `ai:openai-1`,
/// `tern-server-token`), at most 128 bytes. An account's secrets go through `account.*`, so
/// the rule that changing its servers clears them can't be side-stepped.
pub fn writable_secret_name(name: &str) -> bool {
    let (kind, id) = name.split_once(':').unwrap_or((name, ""));
    let word = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    name.len() <= 128
        && word(kind)
        && kind != "account"
        && (!name.contains(':')
            || (!id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))))
}

/// The most a secret value may hold.
pub const MAX_SECRET_BYTES: usize = 64 * 1024;

/// The accounts in `accounts.json`. A file that can't be read is set aside, so the next
/// write doesn't replace what may still be recovered from it.
fn load_accounts(path: &Path) -> Vec<Value> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warn!("-X Could not read {}: {e}", path.display());
            return Vec::new();
        }
    };
    let parsed = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v.get("accounts").and_then(Value::as_array).cloned());
    match parsed {
        Some(list) => {
            let mut seen = BTreeSet::new();
            list.into_iter()
                .filter(|a| account_id(a).is_some_and(|id| seen.insert(id.to_owned())))
                .collect()
        }
        None => {
            let aside = path.with_extension(format!("json.unreadable-{}", std::process::id()));
            warn!(
                "-X {} is not an account list; moved to {}",
                path.display(),
                aside.display()
            );
            let _ = std::fs::rename(path, aside);
            Vec::new()
        }
    }
}

fn save_accounts(path: &Path, accounts: &[Value]) -> Result<(), String> {
    let mut body = Map::new();
    body.insert("version".into(), json!(1));
    body.insert("accounts".into(), Value::Array(accounts.to_vec()));
    let text = serde_json::to_vec_pretty(&Value::Object(body)).map_err(|e| e.to_string())?;
    crate::platform::uds::write_private_file(path, &text)
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_secrets::KeySource;

    fn registry(dir: &Path) -> Registry {
        Registry::with_selection(
            dir,
            Ok(Selection::File {
                data: dir.join(tern_secrets::FILE_NAME),
                key: KeySource::Bytes(vec![3; 32]),
            }),
        )
    }

    fn account(id: &str, email: &str, password: Option<&str>) -> Value {
        let mut settings = json!({
            "imap_host": "127.0.0.1", "imap_port": 993, "imap_username": email,
            "imap_security": "SSL / TLS", "smtp_host": "127.0.0.1", "smtp_port": 465,
            "smtp_username": email, "smtp_security": "SSL / TLS",
            "imap_allow_insecure_ssl": false, "smtp_allow_insecure_ssl": false,
        });
        if let Some(p) = password {
            settings["imap_password"] = json!(p);
            settings["smtp_password"] = json!(p);
        }
        json!({"id": id, "emailAddress": email, "provider": "imap", "settings": settings})
    }

    #[test]
    fn register_strips_and_restores_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        let (public, changed) = r.register(&account("a1", "a@x.test", Some("pw"))).unwrap();
        assert!(changed);
        assert!(public["settings"].get("imap_password").is_none());
        let text = std::fs::read_to_string(dir.path().join(ACCOUNTS_FILE)).unwrap();
        assert!(!text.contains("pw\""), "{text}");
        let again = registry(dir.path());
        assert_eq!(again.accounts().len(), 1);
        let full = again.with_secrets(again.get("a1").unwrap()).unwrap();
        assert_eq!(full["settings"]["imap_password"], "pw");
        assert!(again.forget("a1").unwrap());
        assert!(again.accounts().is_empty());
        let r = registry(dir.path());
        let full = r.with_secrets(account("a1", "a@x.test", None)).unwrap();
        assert!(full["settings"].get("imap_password").is_none());
    }

    #[test]
    fn import_maps_email_keys_to_every_account_with_that_email() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        let secrets = BTreeMap::from([
            ("same@x.test-imap".to_owned(), "pw".to_owned()),
            ("same@x.test-smtp".to_owned(), "pw".to_owned()),
            ("integration-deepl-key".to_owned(), "k".to_owned()),
            ("tern-server-token".to_owned(), "t".to_owned()),
            ("unrelated".to_owned(), "u".to_owned()),
        ]);
        let result = r.import(CredentialsImportParams {
            accounts: vec![
                account("a1", "same@x.test", None),
                account("a2", "same@x.test", None),
                account("a3", "nokeys@x.test", None),
            ],
            secrets,
        });
        assert_eq!(result.imported, vec!["a1", "a2"]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].account_id, "a3");
        assert_eq!(
            result.secrets,
            vec!["integration:deepl", "tern-server-token"]
        );
        for id in ["a1", "a2"] {
            let full = r.with_secrets(r.get(id).unwrap()).unwrap();
            assert_eq!(full["settings"]["imap_password"], "pw");
        }
        let store = r.store().unwrap();
        assert_eq!(
            store.get("integration:deepl").unwrap().as_deref(),
            Some("k")
        );
        assert_eq!(store.get("unrelated").unwrap(), None);
    }

    #[test]
    fn a_rotated_token_is_kept_for_registered_accounts_only() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        r.register(&account("a1", "a@x.test", Some("pw"))).unwrap();
        assert!(r.store_refresh_token("a1", "rotated").unwrap());
        assert!(!r.store_refresh_token("nope", "rotated").unwrap());
        let full = r.with_secrets(r.get("a1").unwrap()).unwrap();
        assert_eq!(full["settings"]["refresh_token"], "rotated");
    }

    #[test]
    fn stored_secrets_fill_in_only_for_the_same_servers() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        r.register(&account("a1", "a@x.test", Some("pw"))).unwrap();
        let (same, other) = r
            .with_secrets_for_same_servers(account("a1", "a@x.test", None))
            .unwrap();
        assert!(!other);
        assert_eq!(same["settings"]["imap_password"], "pw");
        let mut moved = account("a1", "a@x.test", None);
        moved["settings"]["smtp_host"] = json!("mail.evil.test");
        let (moved, other) = r.with_secrets_for_same_servers(moved).unwrap();
        assert!(other);
        assert!(moved["settings"].get("smtp_password").is_none());
        let (unknown, other) = r
            .with_secrets_for_same_servers(account("a2", "a@x.test", None))
            .unwrap();
        assert!(!other);
        assert!(unknown["settings"].get("imap_password").is_none());

        let mut port = account("a1", "a@x.test", None);
        port["settings"]["imap_port"] = json!("993");
        port["label"] = json!("Work");
        assert!(same_servers(&account("a1", "a@x.test", None), &port));
    }

    #[test]
    fn an_unreadable_accounts_file_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(ACCOUNTS_FILE), "{not json").unwrap();
        let r = registry(dir.path());
        assert!(r.accounts().is_empty());
        assert!(!dir.path().join(ACCOUNTS_FILE).exists());
    }

    #[test]
    fn forgetting_needs_a_store_only_when_it_can_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let closed = Registry::with_selection(dir.path(), Err("access denied".to_owned()));
        assert!(closed.delete_secret("integration:ai-claude").is_err());
        assert!(closed.forget_secret("integration:ai-claude").is_ok());

        let r = registry(dir.path());
        r.set_secret("integration:ai-claude", "sk-synthetic")
            .unwrap();
        r.forget_secret("integration:ai-claude").unwrap();
        assert_eq!(r.secret("integration:ai-claude").unwrap(), None);

        // A store whose file no longer reads.
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join(tern_secrets::FILE_NAME), b"not a keyset").unwrap();
        let broken = registry(broken.path());
        assert!(broken.secret("integration:ai-claude").is_err());
        assert!(broken.forget_secret("integration:ai-claude").is_ok());
    }
}
