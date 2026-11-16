// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Attachment file names as the desktop writes them to disk.

/// `File.safeDisplayName()` of a name: no control or bidi characters, no path separators,
/// no trailing dots or spaces, no Windows device names, never empty.
pub fn safe_file_name(display_name: &str) -> String {
    let mut name: String = display_name
        .chars()
        .filter(|c| {
            let u = *c as u32;
            // Controls and bidi overrides.
            !matches!(u, 0..=0x1F | 0x7F..=0x9F | 0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
        })
        .map(|c| if "\\/:|?*><\"#".contains(c) { '-' } else { c })
        .collect();
    while name.ends_with(['.', ' ']) {
        name.pop();
    }
    let upper = name.to_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem)
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.chars().count() == 4
            && stem
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)));
    if reserved {
        name.insert(0, '_');
    }
    if name.is_empty() {
        name.push('-');
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_on_every_platform() {
        assert_eq!(safe_file_name("re\u{202E}port:1.txt. "), "report-1.txt");
        assert_eq!(safe_file_name("../../etc/passwd"), "..-..-etc-passwd");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("lpt1"), "_lpt1");
        assert_eq!(safe_file_name(""), "-");
        assert_eq!(safe_file_name("..."), "-");
    }
}
