// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Writes the protocol's JSON Schema to `packages/client/schema/tern-ipc.json` and the
//! query allowlist to `crates/tern-store/src/model_query_allowlist.json`. With `--check` it
//! writes nothing and exits 1 when either file differs.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let outputs: [(PathBuf, String); 2] = [
        (
            root.join("packages/client/schema/tern-ipc.json"),
            tern_ipc::schema::protocol_text(),
        ),
        (
            root.join("crates/tern-store/src/model_query_allowlist.json"),
            tern_ipc::query_attrs::allowlist_text(),
        ),
    ];
    let mut stale = false;
    for (path, text) in &outputs {
        let shown = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        let current = std::fs::read_to_string(path).unwrap_or_default();
        if current == *text {
            continue;
        }
        if check {
            eprintln!("{shown} is out of date: run `cargo run -p tern-ipc --bin tern-ipc-schema`");
            stale = true;
        } else {
            if let Some(dir) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    eprintln!("{shown}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            if let Err(e) = std::fs::write(path, text) {
                eprintln!("{shown}: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!("wrote {shown}");
        }
    }
    if stale {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
