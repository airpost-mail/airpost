// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine's secret store: account passwords and refresh
//! tokens, integration keys and the Tern server token, held as one keyset in one item so a
//! keychain asks at most once.
//!
//! Backends: the macOS Keychain (generic password, service [`SERVICE`]), the Linux Secret
//! Service, or `secrets.enc` in the config directory, AES-256-GCM with a key from
//! `TERN_ENGINE_KEY_FILE` or systemd's `$CREDENTIALS_DIRECTORY`. There is no plaintext
//! backend: without one of these the store is unavailable. A keyset that is stored but can't
//! be read is an error, never an empty keyset, so a locked keychain or a wrong key can't lead
//! to the next write erasing every secret.
//!
//! Secrets are written and deleted by name and read back only by the engine itself; nothing
//! here hands them to a client.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

mod file;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "linux")]
mod secret_service;

pub use file::{FileBackend, KeySource};
#[cfg(target_os = "macos")]
pub use keychain::KeychainBackend;
#[cfg(target_os = "linux")]
pub use secret_service::SecretServiceBackend;

/// The keychain service and Secret Service `service` attribute of the engine's item.
pub const SERVICE: &str = "Tern Engine";
/// The item's account name (Keychain) or `account` attribute (Secret Service).
pub const ITEM_ACCOUNT: &str = "keyset";
/// The file store's data file, in the config directory.
pub const FILE_NAME: &str = "secrets.enc";
/// A key file the engine creates when the variable names one that doesn't exist yet.
pub const ENV_KEY_FILE: &str = "TERN_ENGINE_KEY_FILE";
/// systemd's credentials directory (`LoadCredential=`); the key is [`SYSTEMD_CREDENTIAL`].
pub const ENV_CREDENTIALS_DIRECTORY: &str = "CREDENTIALS_DIRECTORY";
pub const SYSTEMD_CREDENTIAL: &str = "tern-engine-key";
/// `keychain` or `secret-service`: the OS store even where the caller's default is the file
/// store (debug builds).
pub const ENV_STORE: &str = "TERN_ENGINE_SECRET_STORE";

/// Secret names.
pub mod names {
    pub const TERN_SERVER_TOKEN: &str = "tern-server-token";

    pub fn account_prefix(account_id: &str) -> String {
        format!("account:{account_id}:")
    }
    pub fn account_imap(account_id: &str) -> String {
        format!("account:{account_id}:imap")
    }
    pub fn account_smtp(account_id: &str) -> String {
        format!("account:{account_id}:smtp")
    }
    pub fn account_refresh_token(account_id: &str) -> String {
        format!("account:{account_id}:refresh-token")
    }
    pub fn integration(provider: &str) -> String {
        format!("integration:{provider}")
    }
    /// What every [`ai_provider`] name starts with.
    pub const AI_PROVIDER_PREFIX: &str = "integration:ai-";
    /// An AI provider's key, by the provider's id in `core.ai.providers`;
    /// `credentials.import` maps the desktop's `integration-ai-<id>-key` to it.
    pub fn ai_provider(provider_id: &str) -> String {
        format!("{AI_PROVIDER_PREFIX}{provider_id}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    Keychain,
    SecretService,
    File,
}

impl StoreKind {
    /// The name `hello.credentials.store` reports.
    pub fn as_str(self) -> &'static str {
        match self {
            StoreKind::Keychain => "keychain",
            StoreKind::SecretService => "secret-service",
            StoreKind::File => "file",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No backend can be used here, or the chosen one can't be reached.
    #[error("no secret store is available: {0}")]
    Unavailable(String),
    /// Something is stored but can't be read: a wrong key, a damaged file, a locked
    /// keychain that refused.
    #[error("the stored secrets can't be read: {0}")]
    Unreadable(String),
    #[error("{0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Where the keyset blob lives.
pub trait Backend: Send + Sync {
    fn kind(&self) -> StoreKind;
    /// The stored blob; `None` when nothing was ever stored.
    fn load(&self) -> Result<Option<Vec<u8>>>;
    fn save(&self, blob: &[u8]) -> Result<()>;
}

pub type Keyset = BTreeMap<String, String>;

/// The keyset in a backend, read once and cached; writes go through to the backend before
/// the cache changes.
pub struct SecretStore {
    backend: Box<dyn Backend>,
    cache: Mutex<Option<Keyset>>,
}

/// Which backend [`select`] picked, before any of it is touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Keychain,
    SecretService,
    File { data: PathBuf, key: KeySource },
}

impl Selection {
    pub fn kind(&self) -> StoreKind {
        match self {
            Selection::Keychain => StoreKind::Keychain,
            Selection::SecretService => StoreKind::SecretService,
            Selection::File { .. } => StoreKind::File,
        }
    }
}

/// Picks the backend for `config_dir` from the environment (`env` reads a variable):
/// `TERN_ENGINE_KEY_FILE`, then `$CREDENTIALS_DIRECTORY`, then `TERN_ENGINE_SECRET_STORE`,
/// then `default_key_file` (the caller's dev default, a key file it may create), then the
/// OS store of this platform. Touches nothing, so it can be reported before the store opens.
pub fn select(
    config_dir: &Path,
    env: &dyn Fn(&str) -> Option<String>,
    default_key_file: Option<PathBuf>,
) -> Result<Selection> {
    let data = config_dir.join(FILE_NAME);
    let var = |k: &str| env(k).filter(|v| !v.is_empty());
    if let Some(path) = var(ENV_KEY_FILE) {
        return Ok(Selection::File {
            data,
            key: KeySource::File {
                path: PathBuf::from(path),
                create: true,
            },
        });
    }
    if let Some(dir) = var(ENV_CREDENTIALS_DIRECTORY) {
        return Ok(Selection::File {
            data,
            key: KeySource::File {
                path: Path::new(&dir).join(SYSTEMD_CREDENTIAL),
                create: false,
            },
        });
    }
    match var(ENV_STORE).as_deref() {
        Some("keychain") => return os_store(StoreKind::Keychain),
        Some("secret-service") => return os_store(StoreKind::SecretService),
        Some(other) => {
            return Err(Error::Unavailable(format!(
                "{ENV_STORE}={other:?}; expected \"keychain\" or \"secret-service\""
            )))
        }
        None => {}
    }
    if let Some(path) = default_key_file {
        return Ok(Selection::File {
            data,
            key: KeySource::File { path, create: true },
        });
    }
    if cfg!(target_os = "macos") {
        os_store(StoreKind::Keychain)
    } else if cfg!(target_os = "linux") {
        os_store(StoreKind::SecretService)
    } else {
        Err(Error::Unavailable(format!(
            "this platform has no supported keychain; set {ENV_KEY_FILE} to a key file"
        )))
    }
}

fn os_store(kind: StoreKind) -> Result<Selection> {
    match kind {
        StoreKind::Keychain if cfg!(target_os = "macos") => Ok(Selection::Keychain),
        StoreKind::SecretService if cfg!(target_os = "linux") => Ok(Selection::SecretService),
        _ => Err(Error::Unavailable(format!(
            "the {} store doesn't exist on this platform",
            kind.as_str()
        ))),
    }
}

impl SecretStore {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        SecretStore {
            backend,
            cache: Mutex::new(None),
        }
    }

    /// The store `selection` names. Opening reads nothing; the first read or write does.
    pub fn open(selection: &Selection) -> Result<Self> {
        let backend: Box<dyn Backend> = match selection {
            Selection::File { data, key } => Box::new(FileBackend::new(data.clone(), key.clone())),
            #[cfg(target_os = "macos")]
            Selection::Keychain => Box::new(KeychainBackend::new(SERVICE, ITEM_ACCOUNT)),
            #[cfg(target_os = "linux")]
            Selection::SecretService => Box::new(SecretServiceBackend::new(SERVICE, ITEM_ACCOUNT)),
            #[allow(unreachable_patterns)]
            other => {
                return Err(Error::Unavailable(format!(
                    "the {} store doesn't exist on this platform",
                    other.kind().as_str()
                )))
            }
        };
        Ok(SecretStore::new(backend))
    }

    pub fn kind(&self) -> StoreKind {
        self.backend.kind()
    }

    fn lock(&self) -> MutexGuard<'_, Option<Keyset>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn loaded<'a>(&self, guard: &'a mut MutexGuard<'_, Option<Keyset>>) -> Result<&'a mut Keyset> {
        if guard.is_none() {
            let keys = match self.backend.load()? {
                Some(blob) => decode(&blob)?,
                None => Keyset::new(),
            };
            **guard = Some(keys);
        }
        Ok(guard.as_mut().expect("loaded above"))
    }

    pub fn get(&self, name: &str) -> Result<Option<String>> {
        let mut guard = self.lock();
        Ok(self.loaded(&mut guard)?.get(name).cloned())
    }

    /// Whether each of `names` has a value.
    pub fn has<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, bool>> {
        let mut guard = self.lock();
        let keys = self.loaded(&mut guard)?;
        Ok(names
            .into_iter()
            .map(|n| (n.to_owned(), keys.contains_key(n)))
            .collect())
    }

    /// The names that have a value and start with `prefix`.
    pub fn names_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let mut guard = self.lock();
        Ok(self
            .loaded(&mut guard)?
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }

    /// Applies `change` to a copy of the keyset and stores it; the cache follows only once
    /// the backend took it. A change that leaves the keyset as it was writes nothing.
    pub fn update(&self, change: impl FnOnce(&mut Keyset)) -> Result<()> {
        let mut guard = self.lock();
        let current = self.loaded(&mut guard)?;
        let mut next = current.clone();
        change(&mut next);
        if next == *current {
            return Ok(());
        }
        self.backend.save(&encode(&next))?;
        *current = next;
        Ok(())
    }

    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        self.update(|k| {
            k.insert(name.to_owned(), value.to_owned());
        })
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        self.update(|k| {
            k.remove(name);
        })
    }

    pub fn delete_prefix(&self, prefix: &str) -> Result<()> {
        self.update(|k| k.retain(|name, _| !name.starts_with(prefix)))
    }
}

/// `{"version":1,"secrets":{...}}`, the blob every backend stores.
fn encode(keys: &Keyset) -> Vec<u8> {
    serde_json::json!({ "version": 1, "secrets": keys })
        .to_string()
        .into_bytes()
}

fn decode(blob: &[u8]) -> Result<Keyset> {
    let v: serde_json::Value = serde_json::from_slice(blob)
        .map_err(|e| Error::Unreadable(format!("the keyset is not JSON: {e}")))?;
    let Some(secrets) = v.get("secrets").and_then(serde_json::Value::as_object) else {
        return Err(Error::Unreadable("the keyset has no secrets object".into()));
    };
    Ok(secrets
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Memory {
        blob: Mutex<Option<Vec<u8>>>,
        fail_save: bool,
    }

    impl Backend for Memory {
        fn kind(&self) -> StoreKind {
            StoreKind::File
        }
        fn load(&self) -> Result<Option<Vec<u8>>> {
            Ok(self.blob.lock().unwrap().clone())
        }
        fn save(&self, blob: &[u8]) -> Result<()> {
            if self.fail_save {
                return Err(Error::Backend("refused".into()));
            }
            *self.blob.lock().unwrap() = Some(blob.to_vec());
            Ok(())
        }
    }

    fn memory(fail_save: bool) -> SecretStore {
        SecretStore::new(Box::new(Memory {
            blob: Mutex::new(None),
            fail_save,
        }))
    }

    #[test]
    fn set_get_delete() {
        let s = memory(false);
        assert_eq!(s.get("a").unwrap(), None);
        s.set(&names::account_imap("a1"), "pw").unwrap();
        s.set(&names::account_smtp("a1"), "pw2").unwrap();
        s.set(&names::integration("deepl"), "k").unwrap();
        assert_eq!(s.get("account:a1:imap").unwrap().as_deref(), Some("pw"));
        let has = s
            .has(["account:a1:imap", "account:a1:refresh-token"])
            .unwrap();
        assert!(has["account:a1:imap"]);
        assert!(!has["account:a1:refresh-token"]);
        s.delete_prefix(&names::account_prefix("a1")).unwrap();
        assert_eq!(
            s.names_with_prefix("account:").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(s.get("integration:deepl").unwrap().as_deref(), Some("k"));
    }

    #[test]
    fn a_refused_write_leaves_the_cache_alone() {
        let s = memory(true);
        assert!(s.set("a", "b").is_err());
        assert_eq!(s.get("a").unwrap(), None);
    }

    #[test]
    fn selection_order() {
        let dir = Path::new("/cfg");
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let s = select(
            dir,
            &env(&[(ENV_KEY_FILE, "/k"), (ENV_CREDENTIALS_DIRECTORY, "/run/c")]),
            None,
        )
        .unwrap();
        assert_eq!(
            s,
            Selection::File {
                data: dir.join(FILE_NAME),
                key: KeySource::File {
                    path: "/k".into(),
                    create: true
                }
            }
        );
        let s = select(dir, &env(&[(ENV_CREDENTIALS_DIRECTORY, "/run/c")]), None).unwrap();
        assert_eq!(
            s,
            Selection::File {
                data: dir.join(FILE_NAME),
                key: KeySource::File {
                    path: Path::new("/run/c").join(SYSTEMD_CREDENTIAL),
                    create: false
                }
            }
        );
        let s = select(dir, &env(&[]), Some("/dev.key".into())).unwrap();
        assert_eq!(s.kind(), StoreKind::File);
        assert!(select(dir, &env(&[(ENV_STORE, "plaintext")]), None).is_err());
        let os = select(dir, &env(&[]), None);
        if cfg!(target_os = "macos") {
            assert_eq!(os.unwrap(), Selection::Keychain);
        } else if cfg!(target_os = "linux") {
            assert_eq!(os.unwrap(), Selection::SecretService);
        } else {
            assert!(os.is_err());
        }
    }

    #[test]
    fn an_unreadable_blob_is_an_error_not_an_empty_keyset() {
        let s = SecretStore::new(Box::new(Memory {
            blob: Mutex::new(Some(b"garbage".to_vec())),
            fail_save: false,
        }));
        assert!(matches!(s.get("a"), Err(Error::Unreadable(_))));
        assert!(s.set("a", "b").is_err());
    }
}
