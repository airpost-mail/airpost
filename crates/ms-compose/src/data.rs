// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The lists the engine copies from the desktop, in `desktop_data.json`, which
//! `apps/desktop/tests/integration/draft-corpus-capture.test.ts` writes and checks: the sanitizer's
//! allowlist (`sanitize-transformer.ts`), the draft tracker rules (`remove-tracking-pixels`,
//! from MailTrackerBlocker, BSD-3-Clause, Copyright (c) 2020 Aaron Lee; see NOTICE.md,
//! "MailTrackerBlocker"), Blink's default stylesheet (`chrome-user-agent-stylesheet-string.ts`,
//! LGPL-2.0-or-later, from KHTML and WebKit; see NOTICE.md, "Blink default stylesheet"), the
//! salutations, name prefixes and suffixes and not-a-person address patterns (`utils.ts`,
//! `contact.ts`).

use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopData {
    pub sanitizer: SanitizerLists,
    /// Vendor rules only (no path-only ones): JavaScript regex sources, case-insensitive,
    /// tested against the start of each remote image URL.
    pub tracker_rules: Vec<String>,
    pub user_agent_stylesheet: String,
    pub salutations: Vec<String>,
    pub non_human_email_terms: Vec<String>,
    pub name_prefixes: Vec<String>,
    pub name_suffixes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SanitizerLists {
    pub tags: Vec<String>,
    pub attributes: Vec<String>,
}

pub static DESKTOP: LazyLock<DesktopData> = LazyLock::new(|| {
    serde_json::from_str(include_str!("desktop_data.json")).expect("desktop_data.json parses")
});

#[cfg(test)]
mod tests {
    #[test]
    fn the_lists_load() {
        let d = &*super::DESKTOP;
        assert!(d.sanitizer.tags.iter().any(|t| t == "signature"));
        assert!(d.tracker_rules.len() > 100);
        assert!(d.user_agent_stylesheet.contains("blockquote"));
        assert!(d.salutations.iter().any(|s| s == "mom"));
    }
}
