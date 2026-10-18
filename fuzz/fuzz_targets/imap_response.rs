// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! An IMAP server response line, literals included.

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    ms_imap::fuzz_parse_response(data);
});
