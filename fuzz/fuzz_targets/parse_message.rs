// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A raw RFC 822 message, as a FETCH BODY[] returns it.

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = ms_mime::parse_message(data);
});
