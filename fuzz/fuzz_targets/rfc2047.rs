// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! An RFC 2047 encoded header value.

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = ms_core::rfc2047::decode(data);
});
