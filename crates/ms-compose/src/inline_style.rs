// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The desktop's `InlineStyleTransformer` (`inline-style-transformer.ts`), which the main
//! process runs with juice: a quoted body with a `<style>` block gets its rules written into
//! `style` attributes, with Chromium's user-agent stylesheet first so elements keep the look
//! the default styles gave them, and the `<style>` blocks go. css-inline does the inlining
//! here, without loading anything remote. Where it differs from juice: rules it can't apply to
//! an element (`:hover`, `::-webkit-*`) are dropped, where juice keeps them in a `<style>`
//! block; at-rules (`@media`) are kept, as juice does.

use std::sync::LazyLock;

use regex::Regex;

use crate::data::DESKTOP;

/// `_requiresProcessing`: anything with a `<style`.
pub fn requires_processing(html: &str) -> bool {
    static STYLE: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)<style").unwrap());
    STYLE.is_match(html)
}

/// `_prepareHTMLForInlineStyling`: a stylesheet wrapped in `<!-- -->` unwrapped, and the
/// user-agent stylesheet put in front of the first `<style>`.
pub fn prepare(html: &str) -> String {
    static COMMENTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"<style[^>]*>[\n\r \t]*<!--([^</]*)-->[\n\r \t]*</style").unwrap()
    });
    static FIRST_STYLE: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)<style").unwrap());
    let result = COMMENTED.replace_all(html, "<style>$1</style");
    match FIRST_STYLE.find(&result) {
        Some(m) => format!(
            "{}<style>{}</style>{}",
            &result[..m.start()],
            DESKTOP.user_agent_stylesheet,
            &result[m.start()..]
        ),
        None => result.into_owned(),
    }
}

/// `InlineStyleTransformer.run`: the prepared HTML when inlining fails, as the desktop does.
pub fn inline_styles(html: &str) -> String {
    if !requires_processing(html) {
        return html.to_owned();
    }
    let prepared = prepare(html);
    let inliner = css_inline::CSSInliner::options()
        .load_remote_stylesheets(false)
        .keep_style_tags(false)
        .keep_link_tags(false)
        .keep_at_rules(true)
        .build();
    inliner.inline_fragment(&prepared, "").unwrap_or(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inlines_rules_and_user_agent_styles() {
        let got = inline_styles(
            "<style>.big { color: red } p { margin: 0 }</style><p class=\"big\">x</p>",
        );
        assert!(!got.contains("<style>") || got.contains("@media"), "{got}");
        assert!(got.contains("color: red"), "{got}");
        assert!(got.contains("display: block"), "{got}");
        assert_eq!(inline_styles("<p>plain</p>"), "<p>plain</p>");
    }
}
