// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! A vCard from a CardDAV server.

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(vcf) = std::str::from_utf8(data) {
        let _ = airpost_dav::vcard::ContactFields::from_vcard(vcf);
    }
});
