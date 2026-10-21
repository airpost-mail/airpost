// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Paired devices and pending pairing codes.
//!
//! Everything lives in `<config>/gateway/devices.json` (0600), rewritten atomically under an
//! exclusive `flock` on `devices.lock`, so the running gateway and the `pair` / `devices`
//! subcommands can both change it. Every change re-reads the file under the lock first;
//! nobody writes back a stale copy. Only sha256 of codes and tokens is stored.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCOPE_ADMIN: &str = "admin";
pub const TOKEN_PREFIX: &str = "tern_gw_";
/// Device names are cut to this many characters.
pub const MAX_NAME_CHARS: usize = 64;
pub const DEFAULT_CODE_TTL_MS: i64 = 10 * 60 * 1000;
pub const MAX_CODE_TTL_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 3600 * 1000;
/// Devices unused for this long are dropped; admin devices sooner (2.3).
pub const IDLE_EXPIRY_MS: i64 = 180 * DAY_MS;
pub const ADMIN_IDLE_EXPIRY_MS: i64 = 30 * DAY_MS;

const FILE_VERSION: u32 = 1;
/// RFC 4648 base32. Codes are 12 characters, 60 bits.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const CODE_LEN: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    /// Hex sha256 of the token.
    token_hash: String,
    pub paired_at: i64,
    pub last_used_at: i64,
}

impl Device {
    pub fn is_admin(&self) -> bool {
        self.scopes.iter().any(|s| s == SCOPE_ADMIN)
    }

    fn idle_limit(&self) -> i64 {
        if self.is_admin() {
            ADMIN_IDLE_EXPIRY_MS
        } else {
            IDLE_EXPIRY_MS
        }
    }

    pub fn expired(&self, now: i64) -> bool {
        now - self.last_used_at > self.idle_limit()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pending {
    code_hash: String,
    #[serde(default)]
    name: Option<String>,
    scopes: Vec<String>,
    expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Data {
    version: u32,
    #[serde(default)]
    devices: Vec<Device>,
    #[serde(default)]
    pending: Vec<Pending>,
}

impl Default for Data {
    fn default() -> Self {
        Data {
            version: FILE_VERSION,
            devices: Vec::new(),
            pending: Vec::new(),
        }
    }
}

impl Data {
    fn prune(&mut self, now: i64) {
        self.pending.retain(|p| p.expires_at > now);
        self.devices.retain(|d| !d.expired(now));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DevicesError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: not a devices file this gateway understands: {message}")]
    Parse { path: PathBuf, message: String },
}

/// What the running gateway checks requests against, rebuilt from the file on every change.
#[derive(Debug, Default)]
pub struct Snapshot {
    by_token: HashMap<String, Device>,
    /// The latest expiry of a pending code, for the pairing limit that only applies while a
    /// code is waiting.
    pending_until: Option<i64>,
}

impl Snapshot {
    pub fn authenticate(&self, token: &str) -> Option<&Device> {
        self.by_token.get(&hash_hex(token))
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.by_token.values().find(|d| d.id == id)
    }

    pub fn code_pending(&self, now: i64) -> bool {
        self.pending_until.is_some_and(|t| t > now)
    }
}

/// `<config>/gateway/`: `devices.json`, its lock, and the control socket.
#[derive(Clone, Debug)]
pub struct DeviceFile {
    dir: PathBuf,
}

impl DeviceFile {
    /// Creates `dir` (0700) when missing.
    pub fn open(dir: &Path) -> Result<DeviceFile, DevicesError> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|source| DevicesError::Io {
                path: dir.to_owned(),
                source,
            })?;
        Ok(DeviceFile {
            dir: dir.to_owned(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self) -> PathBuf {
        self.dir.join("devices.json")
    }

    fn io(&self, path: PathBuf) -> impl FnOnce(io::Error) -> DevicesError {
        move |source| DevicesError::Io { path, source }
    }

    fn lock(&self) -> Result<Lock, DevicesError> {
        let path = self.dir.join("devices.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(self.io(path.clone()))?;
        // SAFETY: flock on a descriptor we own; it is released when `file` closes.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(DevicesError::Io {
                path,
                source: io::Error::last_os_error(),
            });
        }
        Ok(Lock(file))
    }

    fn read(&self) -> Result<Data, DevicesError> {
        let path = self.path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Data::default()),
            Err(source) => return Err(DevicesError::Io { path, source }),
        };
        let data: Data = serde_json::from_str(&text).map_err(|e| DevicesError::Parse {
            path: path.clone(),
            message: e.to_string(),
        })?;
        if data.version != FILE_VERSION {
            return Err(DevicesError::Parse {
                path,
                message: format!("version {}", data.version),
            });
        }
        Ok(data)
    }

    fn write(&self, data: &Data) -> Result<(), DevicesError> {
        let path = self.path();
        let tmp = self.dir.join("devices.json.tmp");
        let json = serde_json::to_vec_pretty(data).expect("devices serialize");
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(self.io(tmp.clone()))?;
        f.write_all(&json)
            .and_then(|()| f.sync_all())
            .map_err(self.io(tmp.clone()))?;
        fs::rename(&tmp, &path).map_err(self.io(path))
    }

    /// Re-reads the file under the lock, prunes it, applies `f` and writes it back.
    fn update<T>(&self, now: i64, f: impl FnOnce(&mut Data) -> T) -> Result<T, DevicesError> {
        let _lock = self.lock()?;
        let mut data = self.read()?;
        data.prune(now);
        let out = f(&mut data);
        self.write(&data)?;
        Ok(out)
    }

    pub fn snapshot(&self, now: i64) -> Result<Snapshot, DevicesError> {
        let _lock = self.lock()?;
        let mut data = self.read()?;
        data.prune(now);
        Ok(Snapshot {
            pending_until: data.pending.iter().map(|p| p.expires_at).max(),
            by_token: data
                .devices
                .into_iter()
                .map(|d| (d.token_hash.clone(), d))
                .collect(),
        })
    }

    pub fn list(&self, now: i64) -> Result<Vec<Device>, DevicesError> {
        let _lock = self.lock()?;
        let mut data = self.read()?;
        data.prune(now);
        Ok(data.devices)
    }

    /// Adds a pending code and returns it, formatted for display (`ABCD-EFGH-JKLM`).
    pub fn mint_code(
        &self,
        name: Option<&str>,
        admin: bool,
        ttl_ms: i64,
        now: i64,
    ) -> Result<String, DevicesError> {
        let code = new_code();
        let pending = Pending {
            code_hash: hash_hex(&code),
            name: name.map(clean_name).filter(|n| !n.is_empty()),
            scopes: scopes(admin),
            expires_at: now + ttl_ms.clamp(1, MAX_CODE_TTL_MS),
        };
        self.update(now, |d| d.pending.push(pending))?;
        Ok(format_code(&code))
    }

    /// Consumes the pending code `code` (any spelling [`normalize_code`] accepts) and pairs a
    /// device. Returns it and its token, or None for an unknown or expired code. The name
    /// given to `pair --name` wins over `device_name`.
    pub fn redeem(
        &self,
        code: &str,
        device_name: &str,
        now: i64,
    ) -> Result<Option<(Device, String)>, DevicesError> {
        let Some(code) = normalize_code(code) else {
            return Ok(None);
        };
        let hash = hash_hex(&code);
        self.update(now, |d| {
            let i = d.pending.iter().position(|p| p.code_hash == hash)?;
            let pending = d.pending.swap_remove(i);
            let token = new_token();
            let name = pending
                .name
                .unwrap_or_else(|| clean_name(device_name))
                .chars()
                .take(MAX_NAME_CHARS)
                .collect::<String>();
            let device = Device {
                id: new_device_id(),
                name: if name.is_empty() {
                    "Browser".into()
                } else {
                    name
                },
                scopes: pending.scopes,
                token_hash: hash_hex(&token),
                paired_at: now,
                last_used_at: now,
            };
            d.devices.push(device.clone());
            Some((device, token))
        })
    }

    pub fn revoke(&self, id: &str, now: i64) -> Result<bool, DevicesError> {
        self.update(now, |d| {
            let before = d.devices.len();
            d.devices.retain(|x| x.id != id);
            d.devices.len() != before
        })
    }

    pub fn rename(&self, id: &str, name: &str, now: i64) -> Result<bool, DevicesError> {
        let name = clean_name(name);
        self.update(now, |d| match d.devices.iter_mut().find(|x| x.id == id) {
            Some(x) if !name.is_empty() => {
                x.name = name;
                true
            }
            _ => false,
        })
    }

    /// Records last-used times kept in memory by the running gateway (by device id).
    pub fn touch(&self, used: &HashMap<String, i64>, now: i64) -> Result<(), DevicesError> {
        if used.is_empty() {
            return Ok(());
        }
        self.update(now, |d| {
            for x in &mut d.devices {
                if let Some(&t) = used.get(&x.id) {
                    x.last_used_at = x.last_used_at.max(t);
                }
            }
        })
    }
}

struct Lock(#[allow(dead_code)] File);

fn scopes(admin: bool) -> Vec<String> {
    if admin {
        vec![SCOPE_ADMIN.to_owned()]
    } else {
        Vec::new()
    }
}

/// Trims, drops control characters and cuts to [`MAX_NAME_CHARS`].
pub fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .collect()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    rand::fill(&mut buf[..]);
    buf
}

fn hash_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn new_code() -> String {
    let n = u64::from_le_bytes(random_bytes::<8>());
    (0..CODE_LEN)
        .map(|i| CODE_ALPHABET[((n >> (5 * i)) & 31) as usize] as char)
        .collect()
}

fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(random_bytes::<32>())
    )
}

fn new_device_id() -> String {
    hex(&random_bytes::<6>())
}

/// `ABCDEFGHJKLM` → `ABCD-EFGH-JKLM`.
pub fn format_code(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).expect("ascii"))
        .collect::<Vec<_>>()
        .join("-")
}

/// Accepts a code typed by hand: any case, with dashes or spaces, and the digits people
/// read for look-alike letters (0 for O, 1 for I, 8 for B).
pub fn normalize_code(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .map(|c| match c.to_ascii_uppercase() {
            '0' => 'O',
            '1' => 'I',
            '8' => 'B',
            c => c,
        })
        .collect();
    (code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.contains(&b))).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn file() -> (tempfile::TempDir, DeviceFile) {
        let dir = tempfile::tempdir().unwrap();
        let f = DeviceFile::open(&dir.path().join("gateway")).unwrap();
        (dir, f)
    }

    #[test]
    fn codes() {
        let c = new_code();
        assert_eq!(c.len(), 12);
        assert_eq!(normalize_code(&c).as_deref(), Some(c.as_str()));
        assert_eq!(normalize_code(&format_code(&c).to_lowercase()), Some(c));
        assert_eq!(
            normalize_code("abc0-1fg8 2345").as_deref(),
            Some("ABCOIFGB2345")
        );
        assert_eq!(normalize_code("ABCD-EFGH-JKL"), None);
        assert_eq!(normalize_code("ABCD-EFGH-JKL9"), None);
        assert_ne!(new_code(), new_code());
    }

    #[test]
    fn pair_once() {
        let (_d, f) = file();
        let code = f.mint_code(None, false, DEFAULT_CODE_TTL_MS, NOW).unwrap();
        assert!(f.snapshot(NOW).unwrap().code_pending(NOW));
        let (dev, token) = f.redeem(&code, "Firefox on Linux", NOW).unwrap().unwrap();
        assert_eq!(dev.name, "Firefox on Linux");
        assert!(!dev.is_admin());
        assert!(token.starts_with(TOKEN_PREFIX));
        assert!(f.redeem(&code, "again", NOW).unwrap().is_none());

        let snap = f.snapshot(NOW).unwrap();
        assert!(!snap.code_pending(NOW));
        assert_eq!(snap.authenticate(&token).map(|d| &d.id), Some(&dev.id));
        assert!(snap.authenticate("tern_gw_nope").is_none());

        let text = fs::read_to_string(f.path()).unwrap();
        assert!(!text.contains(&token));
        assert!(!text.contains(&code.replace('-', "")));
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(f.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn code_expires() {
        let (_d, f) = file();
        let code = f.mint_code(None, true, 1000, NOW).unwrap();
        assert!(f.redeem(&code, "x", NOW + 1000).unwrap().is_none());
        assert!(!f.snapshot(NOW + 1000).unwrap().code_pending(NOW + 1000));
    }

    #[test]
    fn named_admin_device() {
        let (_d, f) = file();
        let code = f
            .mint_code(Some("laptop\u{7}"), true, DEFAULT_CODE_TTL_MS, NOW)
            .unwrap();
        let (dev, _) = f.redeem(&code, "ignored", NOW).unwrap().unwrap();
        assert_eq!(dev.name, "laptop");
        assert!(dev.is_admin());
    }

    #[test]
    fn revoke_rename_touch_expire() {
        let (_d, f) = file();
        let code = f.mint_code(None, false, DEFAULT_CODE_TTL_MS, NOW).unwrap();
        let (dev, token) = f.redeem(&code, "a", NOW).unwrap().unwrap();
        let code = f.mint_code(None, true, DEFAULT_CODE_TTL_MS, NOW).unwrap();
        let (admin, _) = f.redeem(&code, "b", NOW).unwrap().unwrap();

        assert!(f.rename(&dev.id, " desk ", NOW).unwrap());
        assert!(!f.rename(&dev.id, "  ", NOW).unwrap());
        assert_eq!(f.list(NOW).unwrap()[0].name, "desk");

        // Admin devices expire after 30 idle days, others after 180.
        let later = NOW + ADMIN_IDLE_EXPIRY_MS + 1;
        let ids: Vec<_> = f.list(later).unwrap().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![dev.id.clone()]);
        assert!(f.list(later).unwrap().iter().all(|d| d.id != admin.id));

        f.touch(&HashMap::from([(dev.id.clone(), later)]), later)
            .unwrap();
        let much_later = NOW + IDLE_EXPIRY_MS + 10;
        assert!(f
            .snapshot(much_later)
            .unwrap()
            .authenticate(&token)
            .is_some());

        assert!(f.revoke(&dev.id, much_later).unwrap());
        assert!(!f.revoke(&dev.id, much_later).unwrap());
        assert!(f
            .snapshot(much_later)
            .unwrap()
            .authenticate(&token)
            .is_none());
    }

    #[test]
    fn foreign_version_refused() {
        let (_d, f) = file();
        fs::write(f.path(), r#"{"version":2,"devices":[]}"#).unwrap();
        assert!(matches!(f.snapshot(NOW), Err(DevicesError::Parse { .. })));
    }
}
