// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! stdout writes. The client parses stdout, and a write to a closed pipe must not panic
//! (Rust ignores SIGPIPE, so `println!` would), so every write goes through here and
//! errors are dropped.

use std::io::Write;

use serde_json::Value;

pub fn write_stdout(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// One-shot result: `"\n" + {"error":…}` (plus any other keys of `resp`), no newline after.
pub fn one_shot(resp: &Value) {
    write_stdout(&format!("\n{resp}"));
}

pub fn one_shot_error(message: &str) {
    one_shot(&serde_json::json!({ "error": message }));
}
