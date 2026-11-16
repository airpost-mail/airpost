// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `secrets.enc`: the keyset sealed with AES-256-GCM, for machines without a keychain
//! (headless Linux, systemd services) and for dev builds, whose unsigned binaries a keychain
//! would ask about on every rebuild.
//!
//! The AES key is HKDF-SHA256 of a key file: `AIRPOST_ENGINE_KEY_FILE` (created with 32 random
//! bytes, hex, when missing) or systemd's `$CREDENTIALS_DIRECTORY/airpost-engine-key`. The key
//! file must be the engine user's and closed to group and others. A key file on the same disk
//! as `secrets.enc` keeps the secrets out of backups and casual reads, no more; a keychain is
//! stronger, and the docs say so.
//!
//! Layout: `AIRPSEC1`, a 12-byte nonce, then the ciphertext and tag; the magic is the
//! additional data. Written to a temporary file and renamed, at 0600.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

use crate::{Backend, Error, Result, StoreKind};

const MAGIC: &[u8; 8] = b"AIRPSEC1";
const HKDF_SALT: &[u8] = b"airpost-engine-secrets";
const HKDF_INFO: &[u8] = b"secrets.enc v1";
/// The shortest key file accepted, in bytes.
const MIN_KEY_BYTES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// A key file; `create` makes one when it is missing.
    File { path: PathBuf, create: bool },
    /// The key material itself (tests).
    Bytes(Vec<u8>),
}

pub struct FileBackend {
    data: PathBuf,
    key: KeySource,
}

impl FileBackend {
    pub fn new(data: PathBuf, key: KeySource) -> Self {
        FileBackend { data, key }
    }

    fn sealing_key(&self) -> Result<LessSafeKey> {
        let material = match &self.key {
            KeySource::Bytes(b) => b.clone(),
            KeySource::File { path, create } => read_key_file(path, *create)?,
        };
        derive(&material)
    }
}

impl Backend for FileBackend {
    fn kind(&self) -> StoreKind {
        StoreKind::File
    }

    fn load(&self) -> Result<Option<Vec<u8>>> {
        let sealed = match fs::read(&self.data) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::Unreadable(format!("{}: {e}", self.data.display()))),
        };
        let key = self.sealing_key()?;
        open(&key, &sealed).map(Some).ok_or_else(|| {
            Error::Unreadable(format!(
                "{} can't be decrypted with this key (a different key file, or a damaged file)",
                self.data.display()
            ))
        })
    }

    fn save(&self, blob: &[u8]) -> Result<()> {
        let key = self.sealing_key()?;
        let sealed = seal(&key, blob)?;
        write_private(&self.data, &sealed)
            .map_err(|e| Error::Backend(format!("writing {}: {e}", self.data.display())))
    }
}

fn derive(material: &[u8]) -> Result<LessSafeKey> {
    let salt = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, HKDF_SALT);
    let prk = salt.extract(material);
    let okm = prk
        .expand(&[HKDF_INFO], &AES_256_GCM)
        .map_err(|_| Error::Backend("key derivation failed".into()))?;
    let mut bytes = [0u8; 32];
    okm.fill(&mut bytes)
        .map_err(|_| Error::Backend("key derivation failed".into()))?;
    let key = UnboundKey::new(&AES_256_GCM, &bytes)
        .map_err(|_| Error::Backend("key derivation failed".into()))?;
    Ok(LessSafeKey::new(key))
}

fn seal(key: &LessSafeKey, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| Error::Backend("no randomness for the nonce".into()))?;
    let mut buf = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(MAGIC),
        &mut buf,
    )
    .map_err(|_| Error::Backend("encryption failed".into()))?;
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + buf.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&buf);
    Ok(out)
}

fn open(key: &LessSafeKey, sealed: &[u8]) -> Option<Vec<u8>> {
    let rest = sealed.strip_prefix(MAGIC.as_slice())?;
    if rest.len() < NONCE_LEN + aead::MAX_TAG_LEN {
        return None;
    }
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
    let mut buf = ciphertext.to_vec();
    let plain = key.open_in_place(nonce, Aad::from(MAGIC), &mut buf).ok()?;
    Some(plain.to_vec())
}

/// The key file's bytes, whitespace trimmed. A missing file is created when `create`.
fn read_key_file(path: &Path, create: bool) -> Result<Vec<u8>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            create_key_file(path)?;
        }
        Err(e) => {
            return Err(Error::Unavailable(format!(
                "key file {}: {e}",
                path.display()
            )))
        }
    }
    check_key_file(path)?;
    let bytes = fs::read(path)
        .map_err(|e| Error::Unavailable(format!("key file {}: {e}", path.display())))?;
    let trimmed = bytes.trim_ascii();
    if trimmed.len() < MIN_KEY_BYTES {
        return Err(Error::Unavailable(format!(
            "key file {} holds {} bytes; it needs at least {MIN_KEY_BYTES}",
            path.display(),
            trimmed.len()
        )));
    }
    Ok(trimmed.to_vec())
}

fn create_key_file(path: &Path) -> Result<()> {
    let mut raw = [0u8; 32];
    SystemRandom::new()
        .fill(&mut raw)
        .map_err(|_| Error::Backend("no randomness for the key".into()))?;
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let fail = |e: std::io::Error| {
        Error::Unavailable(format!("creating key file {}: {e}", path.display()))
    };
    let mut f = create_new_private(path).map_err(fail)?;
    f.write_all(format!("{hex}\n").as_bytes()).map_err(fail)?;
    f.sync_all().map_err(fail)
}

#[cfg(unix)]
fn check_key_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path)
        .map_err(|e| Error::Unavailable(format!("key file {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::Unavailable(format!(
            "key file {} is not a regular file",
            path.display()
        )));
    }
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    if meta.uid() != me {
        return Err(Error::Unavailable(format!(
            "key file {} belongs to another user",
            path.display()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(Error::Unavailable(format!(
            "key file {} is readable by other users (mode {:o}); chmod 600 it",
            path.display(),
            meta.mode() & 0o777
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_key_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_new_private(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_new_private(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Replaces `path` with `bytes` through a temporary file in the same directory, at 0600.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("enc.tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = create_new_private(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open() {
        let key = derive(&[7u8; 32]).unwrap();
        let sealed = seal(&key, b"{\"x\":1}").unwrap();
        assert!(sealed.starts_with(MAGIC));
        assert_eq!(open(&key, &sealed).unwrap(), b"{\"x\":1}");
        let other = derive(&[8u8; 32]).unwrap();
        assert!(open(&other, &sealed).is_none());
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(open(&key, &tampered).is_none());
        assert!(open(&key, b"AIRPSEC1short").is_none());
    }
}
