// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Golden account ids. `account_ids.json` was written by `account_ids.js`, which runs the
//! desktop's `idForAccount`. An id that changes here gives every account added from then on a
//! different id than the desktop gave it, and so a second copy of its mail.

use airpost_core::ids::account_id;
use serde_json::Value;

#[test]
fn account_ids_match_the_desktop() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("account_ids.json")).expect("valid golden file");
    assert!(cases.len() >= 8);
    for c in &cases {
        let settings = c["settings"].as_object().expect("settings object");
        assert_eq!(
            account_id(c["email"].as_str().unwrap(), settings),
            c["id"].as_str().unwrap(),
            "{}",
            c["name"]
        );
    }
}
