// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The file store end to end, and the OS backends behind `AIRPOST_TEST_KEYCHAIN=1`, which CI
//! and the gate don't set: they create and delete a real item ("Airpost Engine Test").

use airpost_secrets::{names, Error, FileBackend, KeySource, SecretStore};

fn file_store(dir: &std::path::Path, key: KeySource) -> SecretStore {
    SecretStore::new(Box::new(FileBackend::new(dir.join("secrets.enc"), key)))
}

#[test]
fn the_file_store_round_trips_and_never_holds_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let key = KeySource::File {
        path: dir.path().join("engine.key"),
        create: true,
    };
    let s = file_store(dir.path(), key.clone());
    s.set(&names::account_imap("a1"), "hunter2-imap").unwrap();
    s.set(names::AIRPOST_SERVER_TOKEN, "tok").unwrap();

    let raw = std::fs::read(dir.path().join("secrets.enc")).unwrap();
    assert!(raw.starts_with(b"AIRPSEC1"));
    assert!(!String::from_utf8_lossy(&raw).contains("hunter2"));

    // A new process reads it back with the same key file.
    let again = file_store(dir.path(), key);
    assert_eq!(
        again.get("account:a1:imap").unwrap().as_deref(),
        Some("hunter2-imap")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["secrets.enc", "engine.key"] {
            let mode = std::fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }
}

#[test]
fn another_key_cant_read_and_nothing_is_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let s = file_store(dir.path(), KeySource::Bytes(vec![1; 32]));
    s.set("a", "b").unwrap();
    let before = std::fs::read(dir.path().join("secrets.enc")).unwrap();

    let wrong = file_store(dir.path(), KeySource::Bytes(vec![2; 32]));
    assert!(matches!(wrong.get("a"), Err(Error::Unreadable(_))));
    assert!(wrong.set("c", "d").is_err());
    assert_eq!(
        std::fs::read(dir.path().join("secrets.enc")).unwrap(),
        before
    );
}

#[cfg(unix)]
#[test]
fn a_key_file_others_can_read_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("k");
    std::fs::write(&path, "0".repeat(64)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let s = file_store(
        dir.path(),
        KeySource::File {
            path: path.clone(),
            create: false,
        },
    );
    let err = s.set("a", "b").unwrap_err();
    assert!(matches!(err, Error::Unavailable(_)), "{err}");
    assert!(!dir.path().join("secrets.enc").exists());

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    s.set("a", "b").unwrap();
}

#[test]
fn a_missing_key_is_only_created_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let s = file_store(
        dir.path(),
        KeySource::File {
            path: dir.path().join("creds").join("airpost-engine-key"),
            create: false,
        },
    );
    assert!(matches!(s.set("a", "b"), Err(Error::Unavailable(_))));
    let short = dir.path().join("short");
    std::fs::write(&short, "abc").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&short, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let s = file_store(
        dir.path(),
        KeySource::File {
            path: short,
            create: false,
        },
    );
    assert!(matches!(s.set("a", "b"), Err(Error::Unavailable(_))));
}

fn os_tests_enabled() -> bool {
    std::env::var("AIRPOST_TEST_KEYCHAIN").as_deref() == Ok("1")
}

#[cfg(target_os = "macos")]
#[test]
fn keychain_round_trip() {
    use airpost_secrets::Backend;
    if !os_tests_enabled() {
        return;
    }
    let backend = airpost_secrets::KeychainBackend::new("Airpost Engine Test", "keyset");
    backend.delete().unwrap();
    assert_eq!(backend.load().unwrap(), None);
    let s = SecretStore::new(Box::new(airpost_secrets::KeychainBackend::new(
        "Airpost Engine Test",
        "keyset",
    )));
    s.set("a", "b").unwrap();
    s.set("c", "d").unwrap();
    let again = SecretStore::new(Box::new(airpost_secrets::KeychainBackend::new(
        "Airpost Engine Test",
        "keyset",
    )));
    assert_eq!(again.get("a").unwrap().as_deref(), Some("b"));
    backend.delete().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn secret_service_round_trip() {
    if !os_tests_enabled() {
        return;
    }
    let backend = airpost_secrets::SecretServiceBackend::new("Airpost Engine Test", "keyset");
    backend.delete().unwrap();
    let s = SecretStore::new(Box::new(airpost_secrets::SecretServiceBackend::new(
        "Airpost Engine Test",
        "keyset",
    )));
    s.set("a", "b").unwrap();
    let again = SecretStore::new(Box::new(airpost_secrets::SecretServiceBackend::new(
        "Airpost Engine Test",
        "keyset",
    )));
    assert_eq!(again.get("a").unwrap().as_deref(), Some("b"));
    backend.delete().unwrap();
}
