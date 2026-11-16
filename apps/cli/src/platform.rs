// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The OS-specific bits: terminal width, file identity.

use std::fs::Metadata;

/// The width of the terminal on stdout, if stdout is one.
#[cfg(unix)]
pub fn terminal_width() -> Option<usize> {
    // SAFETY: TIOCGWINSZ fills a winsize; the struct is plain data and zeroed first.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
    (ok && ws.ws_col > 0).then_some(usize::from(ws.ws_col))
}

#[cfg(not(unix))]
pub fn terminal_width() -> Option<usize> {
    None
}

/// Identifies the file behind a path, to notice when the log was rotated under it.
#[cfg(unix)]
pub fn file_id(meta: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
pub fn file_id(meta: &Metadata) -> Option<(u64, u64)> {
    let _ = meta;
    None
}
