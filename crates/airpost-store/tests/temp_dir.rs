// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `Store::open` points SQLite's process-wide temp directory at the config dir, as
//! `main.cpp` did before every mode. In its own test binary because the setting is global.

use airpost_core::ManualClock;
use airpost_store::{schema, DeltaStream, Store};

#[test]
fn open_sets_the_temp_directory_before_reset_vacuums() {
    let config = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    airpost_store::migrate::migrate(config.path(), &ManualClock::new(1), &mut out).unwrap();
    let probe = schema::open_connection(other.path()).unwrap();
    schema::set_temp_directory(&probe, other.path()).unwrap();

    let store = Store::open(config.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    let dir: String = store
        .conn()
        .query_row("PRAGMA temp_store_directory", [], |r| r.get(0))
        .unwrap();
    assert_eq!(dir, config.path().to_string_lossy());
    store.reset_for_account("acc1", true).unwrap();
}
