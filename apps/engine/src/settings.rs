// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `serve` mode's settings store:
//! `$CONFIG_DIR_PATH/settings.json`, one flat JSON object of the keys in
//! [`airpost_ipc::SETTINGS_KEYS`], read once at start and rewritten atomically on every change.
//!
//! Callers hold a [`SettingsGuard`] while they answer and notify, so a connection sees its own
//! `settings.get` / `settings.set` answers and other connections' `settings.changed` in the
//! order the store applied them. Keys the store doesn't know that are already in the file
//! (written by a newer engine) are kept on rewrite but never served.
//!
//! The file also holds a random store id, made when the store starts without one (no file, or
//! a corrupt one moved aside) and returned with `settings.get`, so a client that copies its own
//! values into a store once can tell a store that lost them from one that cleared them.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use airpost_ipc::{code, setting_error, setting_kind, ErrorObject};
use serde_json::{Map, Value};
use tracing::warn;

pub const SETTINGS_FILE: &str = "settings.json";

/// The store id's key in the file; not a setting, so never served as a value or set.
const STORE_ID_KEY: &str = "$storeId";

pub struct Settings {
    path: PathBuf,
    values: Mutex<Map<String, Value>>,
    /// Why the file couldn't be read, when it exists: reads and writes are refused rather
    /// than answering with, or replacing it by, what little the store holds.
    unreadable: Option<String>,
}

pub struct SettingsGuard<'a> {
    path: &'a Path,
    unreadable: Option<&'a str>,
    values: MutexGuard<'a, Map<String, Value>>,
}

impl Settings {
    /// Loads `settings.json` from `config_dir`. A file that isn't a JSON object is moved
    /// aside to `settings.json.bad` and the store starts empty, with a new store id.
    pub fn open(config_dir: &Path) -> Settings {
        let path = config_dir.join(SETTINGS_FILE);
        let mut unreadable = None;
        let mut values = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(map)) => map,
                Ok(_) | Err(_) => {
                    let aside = path.with_extension("json.bad");
                    warn!(
                        "-X {} is not a JSON object; moving it to {} and starting empty",
                        path.display(),
                        aside.display()
                    );
                    if let Err(e) = std::fs::rename(&path, &aside) {
                        warn!("-X Could not move {}: {e}", path.display());
                        unreadable = Some(format!(
                            "{} is not a JSON object and could not be moved aside: {e}",
                            path.display()
                        ));
                    }
                    Map::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => {
                warn!(
                    "-X Could not read {}: {e}; settings are empty and can't be changed",
                    path.display()
                );
                unreadable = Some(format!("could not read {}: {e}", path.display()));
                Map::new()
            }
        };
        if unreadable.is_none() && !values.get(STORE_ID_KEY).is_some_and(Value::is_string) {
            values.insert(
                STORE_ID_KEY.to_owned(),
                Value::String(uuid::Uuid::new_v4().to_string()),
            );
            // Written now so the id outlives a run that stores nothing; if this fails the id
            // lasts until the first write that succeeds.
            if let Err(e) = write_atomic(&path, &Value::Object(values.clone())) {
                warn!("-X Could not write {}: {e}", path.display());
            }
        }
        Settings {
            path,
            values: Mutex::new(values),
            unreadable,
        }
    }

    pub fn lock(&self) -> SettingsGuard<'_> {
        SettingsGuard {
            path: &self.path,
            unreadable: self.unreadable.as_deref(),
            values: self.values.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }
}

/// The executor reads a key's stored value; an unreadable file reads as unset, so it
/// applies the defaults.
impl airpost_sync::EngineSettings for Settings {
    fn get(&self, key: &str) -> Option<Value> {
        self.lock()
            .get(Some(&[key.to_owned()]))
            .ok()
            .and_then(|mut values| values.remove(key))
    }
}

fn invalid(message: String) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, message)
}

/// Refuses the values [`SettingsGuard::set`] would refuse for their keys or shapes.
pub fn check(values: &Map<String, Value>) -> Result<(), ErrorObject> {
    match values.iter().find_map(|(k, v)| setting_error(k, v)) {
        Some(e) => Err(invalid(e)),
        None => Ok(()),
    }
}

impl SettingsGuard<'_> {
    /// The stored values of `keys` (every key when `None`). A key without a value, or one
    /// this engine doesn't keep, is left out. Refused when the file couldn't be read, so a
    /// client doesn't take the empty store for cleared settings.
    pub fn get(&self, keys: Option<&[String]>) -> Result<Map<String, Value>, ErrorObject> {
        if let Some(e) = self.unreadable {
            return Err(ErrorObject::new(code::INTERNAL_ERROR, e.to_owned()));
        }
        let wanted =
            |k: &str| setting_kind(k).is_some() && keys.is_none_or(|ks| ks.iter().any(|w| w == k));
        Ok(self
            .values
            .iter()
            .filter(|(k, _)| wanted(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    /// The store's id; `None` only when the file couldn't be read.
    pub fn store_id(&self) -> Option<String> {
        self.values
            .get(STORE_ID_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// Stores `values` (null clears a key) and returns the ones that changed, null for a
    /// cleared key. Nothing is stored when a key or value is invalid or the file can't be
    /// written.
    pub fn set(&mut self, values: Map<String, Value>) -> Result<Map<String, Value>, ErrorObject> {
        check(&values)?;
        if let Some(e) = self.unreadable {
            return Err(ErrorObject::new(code::INTERNAL_ERROR, e.to_owned()));
        }
        let changed: Map<String, Value> = values
            .into_iter()
            .filter(|(k, v)| match self.values.get(k) {
                Some(old) => old != v,
                None => !v.is_null(),
            })
            .collect();
        if changed.is_empty() {
            return Ok(changed);
        }
        let mut next = self.values.clone();
        for (k, v) in &changed {
            if v.is_null() {
                next.remove(k);
            } else {
                next.insert(k.clone(), v.clone());
            }
        }
        write_atomic(self.path, &Value::Object(next.clone())).map_err(|e| {
            ErrorObject::new(
                code::INTERNAL_ERROR,
                format!("could not write {}: {e}", self.path.display()),
            )
        })?;
        *self.values = next;
        Ok(changed)
    }
}

/// Writes `value` to a temporary file beside `path`, then renames it over `path`.
fn write_atomic(path: &Path, value: &Value) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn read_file(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join(SETTINGS_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::open(dir.path());
        let changed = s
            .lock()
            .set(map(
                json!({"core.sending.undoSend": 15000, "signatures": {"s1": {"body": "x"}}}),
            ))
            .unwrap();
        assert_eq!(changed.len(), 2);
        // The same value again changes nothing; null clears.
        let again = s
            .lock()
            .set(map(json!({"core.sending.undoSend": 15000})))
            .unwrap();
        assert!(again.is_empty());
        let cleared = s.lock().set(map(json!({"signatures": null}))).unwrap();
        assert_eq!(cleared, map(json!({"signatures": null})));
        let none = s.lock().set(map(json!({"signatures": null}))).unwrap();
        assert!(none.is_empty());

        let reopened = Settings::open(dir.path());
        assert_eq!(
            reopened.lock().get(None).unwrap(),
            map(json!({"core.sending.undoSend": 15000}))
        );
        assert_eq!(
            reopened
                .lock()
                .get(Some(&["signatures".to_owned()]))
                .unwrap(),
            Map::new()
        );
        assert!(!dir.path().join("settings.json.tmp").exists());
    }

    #[test]
    fn invalid_values_store_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::open(dir.path());
        let e = s
            .lock()
            .set(map(
                json!({"core.composing.html": false, "core.sending.undoSend": "soon"}),
            ))
            .unwrap_err();
        assert_eq!(e.code, code::INVALID_PARAMS);
        let e = s
            .lock()
            .set(map(json!({"core.themes": ["x"]})))
            .unwrap_err();
        assert!(e.message.contains("core.themes"), "{}", e.message);
        assert!(s.lock().get(None).unwrap().is_empty());
        assert_eq!(read_file(dir.path()).as_object().unwrap().len(), 1);
    }

    #[test]
    fn unknown_keys_in_the_file_are_kept_but_not_served() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            json!({"future.key": 1, "core.composing.html": true}).to_string(),
        )
        .unwrap();
        let s = Settings::open(dir.path());
        assert_eq!(
            s.lock().get(None).unwrap(),
            map(json!({"core.composing.html": true}))
        );
        s.lock()
            .set(map(json!({"core.composing.html": false})))
            .unwrap();
        let mut file = read_file(dir.path());
        assert!(file[STORE_ID_KEY].is_string());
        file.as_object_mut().unwrap().remove(STORE_ID_KEY);
        assert_eq!(file, json!({"future.key": 1, "core.composing.html": false}));
    }

    #[test]
    fn the_store_id_lasts_until_the_file_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let id = Settings::open(dir.path()).lock().store_id().unwrap();
        let s = Settings::open(dir.path());
        assert_eq!(s.lock().store_id().as_deref(), Some(id.as_str()));
        s.lock()
            .set(map(json!({"core.composing.html": true})))
            .unwrap();
        assert_eq!(
            Settings::open(dir.path()).lock().store_id(),
            Some(id.clone())
        );
        // Not a setting: neither served nor settable.
        assert!(!s.lock().get(None).unwrap().contains_key(STORE_ID_KEY));
        assert!(s.lock().set(map(json!({STORE_ID_KEY: "x"}))).is_err());

        std::fs::remove_file(dir.path().join(SETTINGS_FILE)).unwrap();
        let fresh = Settings::open(dir.path());
        assert_ne!(fresh.lock().store_id(), Some(id));
        assert!(fresh.lock().get(None).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let id = Settings::open(dir.path()).lock().store_id();
        std::fs::write(dir.path().join(SETTINGS_FILE), "{not json").unwrap();
        let s = Settings::open(dir.path());
        assert!(s.lock().get(None).unwrap().is_empty());
        assert!(s.lock().store_id().is_some());
        assert_ne!(s.lock().store_id(), id);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.json.bad")).unwrap(),
            "{not json"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_refuses_reads_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        // A directory in the file's place: it exists but can't be read as a file.
        std::fs::create_dir(dir.path().join(SETTINGS_FILE)).unwrap();
        let s = Settings::open(dir.path());
        assert_eq!(s.lock().get(None).unwrap_err().code, code::INTERNAL_ERROR);
        assert_eq!(s.lock().store_id(), None);
        let e = s
            .lock()
            .set(map(json!({"core.composing.html": true})))
            .unwrap_err();
        assert_eq!(e.code, code::INTERNAL_ERROR);
    }
}
