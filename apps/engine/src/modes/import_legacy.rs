// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `--mode import-legacy`: imports what only the C++
//! engine's database held from the `tern.legacy.db` that `migrate` moved aside. The
//! work is `tern_store::legacy_import`; this is the one-shot wrapper.
//!
//! The client runs it after `migrate` on every launch, before any account process, without
//! `--account` or `--identity` (both are ignored: the legacy database holds every account).
//! It is a no-op when there is no legacy database or it has been imported already, so
//! running it every launch costs one file check and one `_State` read.
//!
//! Output is the one-shot `"\n" + {"error":null}`, after `info:` log lines naming what was
//! imported; a failure prints `{"error":"<what>"}` and exits 1. Nothing is written to the
//! new database unless the whole import commits.

use serde_json::json;
use tern_core::constants::{EXIT_FAILURE, EXIT_OK};

use crate::cli::Options;
use crate::output::{one_shot, one_shot_error};

pub fn run(opts: &Options) -> i32 {
    if let Err(e) = tern_core::log::init_stdout() {
        one_shot_error(&format!("Setup Failed: {e}"));
        return EXIT_FAILURE;
    }
    // No client listens to a one-shot mode's deltas.
    let deltas = tern_store::DeltaStream::new(Box::new(std::io::sink()));
    match tern_store::legacy_import::import_legacy(&opts.config_dir, deltas) {
        Ok(_) => {
            tern_core::log::flush();
            one_shot(&json!({ "error": null }));
            EXIT_OK
        }
        Err(e) => {
            tracing::error!("-X Legacy import failed: {e}");
            tern_core::log::flush();
            one_shot(&json!({ "error": super::error_what(&e) }));
            EXIT_FAILURE
        }
    }
}
