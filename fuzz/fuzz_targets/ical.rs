// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! An iCalendar object from a CalDAV server or an invitation.

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(ics) = std::str::from_utf8(data) {
        let _ = tern_dav::ical::parse_events(ics);
    }
});
