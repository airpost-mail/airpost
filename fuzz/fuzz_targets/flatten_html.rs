// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! A message's HTML body, flattened for snippets and search.

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(html) = std::str::from_utf8(data) {
        let _ = airpost_mime::flatten_html_with(html, true, true);
        let _ = airpost_mime::flatten_html_with(html, false, false);
    }
});
