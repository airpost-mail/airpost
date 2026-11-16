// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The desktop's `SanitizeTransformer` (`sanitize-transformer.ts`), which runs DOMPurify with an
//! allowlist over a message before it is quoted. Here ammonia runs with the same lists
//! (`desktop_data.json`):
//! - only the allowlisted tags and attributes survive (any attribute on any tag, as DOMPurify's
//!   `ALLOWED_ATTR`), no `data-*`, no comments;
//! - a disallowed element keeps its content, except the ones DOMPurify drops with it
//!   (`FORBID_CONTENTS`: `<script>`, `<iframe>`, `<svg>` and so on);
//! - attribute values that are URLs take only the desktop's schemes, `data:` only on images
//!   and media, as `ALLOWED_URI_REGEXP` and DOMPurify's data-URI rule do;
//! - `@import` rules are removed from `<style>` blocks.
//!
//! DOMPurify parses the whole body (`FORCE_BODY`) and ammonia parses a fragment in a `<div>`;
//! both drop `<html>`, `<head>` and `<body>` and keep what was in them in place.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::data::DESKTOP;

/// DOMPurify's `FORBID_CONTENTS` default: disallowed elements whose content goes with them.
const FORBID_CONTENTS: [&str; 25] = [
    "annotation-xml",
    "audio",
    "colgroup",
    "desc",
    "foreignobject",
    "head",
    "iframe",
    "math",
    "mi",
    "mn",
    "mo",
    "ms",
    "mtext",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "style",
    "svg",
    "template",
    "thead",
    "title",
    "video",
    "xmp",
];

/// DOMPurify's `URI_SAFE_ATTRIBUTES`: values never checked as URLs.
const URI_SAFE_ATTRIBUTES: [&str; 14] = [
    "alt",
    "class",
    "for",
    "id",
    "label",
    "name",
    "pattern",
    "placeholder",
    "role",
    "summary",
    "title",
    "value",
    "style",
    "xmlns",
];

/// DOMPurify's `DATA_URI_TAGS`: where `src`/`href` may be a `data:` URL.
const DATA_URI_TAGS: [&str; 7] = [
    "img", "audio", "video", "source", "image", "track", "picture",
];

const URL_SCHEMES: [&str; 10] = [
    "http", "https", "ftp", "ftps", "mailto", "tel", "callto", "sms", "cid", "xmpp",
];

/// The desktop's `ALLOWED_URI_REGEXP`.
static ALLOWED_URI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:https?|ftps?|mailto|tel|callto|sms|cid|xmpp):|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))")
        .expect("uri regex")
});

/// DOMPurify's `ATTR_WHITESPACE`, removed before the URL check.
static ATTR_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("[\u{0000}-\u{0020}\u{00A0}\u{1680}\u{180E}\u{2000}-\u{2029}\u{205F}\u{3000}]")
        .expect("whitespace regex")
});

fn uri_allowed(element: &str, attribute: &str, value: &str) -> bool {
    if URI_SAFE_ATTRIBUTES.contains(&attribute) || value.is_empty() {
        return true;
    }
    let compact = ATTR_WHITESPACE.replace_all(value, "");
    if ALLOWED_URI.is_match(&compact) && !compact.to_ascii_lowercase().starts_with("file:") {
        return true;
    }
    matches!(attribute, "src" | "xlink:href" | "href")
        && value.starts_with("data:")
        && DATA_URI_TAGS.contains(&element)
}

static CLEANER: LazyLock<ammonia::Builder<'static>> = LazyLock::new(|| {
    let lists = &DESKTOP.sanitizer;
    let tags: HashSet<&'static str> = lists
        .tags
        .iter()
        .filter(|t| !t.starts_with('#'))
        .map(String::as_str)
        .collect();
    let attributes: HashSet<&'static str> = lists.attributes.iter().map(String::as_str).collect();
    let clean_content: HashSet<&'static str> = FORBID_CONTENTS
        .iter()
        .copied()
        .filter(|t| !tags.contains(t))
        .collect();
    let mut schemes: HashSet<&'static str> = URL_SCHEMES.into_iter().collect();
    // Kept past ammonia's scheme check; the filter below allows it only where DOMPurify does.
    schemes.insert("data");
    let mut b = ammonia::Builder::empty();
    b.tags(tags)
        .clean_content_tags(clean_content)
        .tag_attributes(HashMap::new())
        .generic_attributes(attributes)
        .url_schemes(schemes)
        .url_relative(ammonia::UrlRelative::PassThrough)
        .link_rel(None)
        .strip_comments(true)
        .attribute_filter(|element, attribute, value| {
            if uri_allowed(element, attribute, value) {
                Some(Cow::Borrowed(value))
            } else {
                None
            }
        });
    b
});

/// `SanitizeTransformer.run`.
pub fn sanitize(html: &str) -> String {
    let cleaned = CLEANER.clean(html).to_string();
    strip_style_imports(&cleaned)
}

static STYLE_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)(<style[^>]*>)(.*?)(</style>)").expect("style regex"));
static AT_KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@((?:[A-Za-z_\-]|\\[0-9A-Fa-f]{1,6}[ \t\n\r\f]?|\\[^0-9A-Fa-f\n])+)").unwrap()
});

/// An at-keyword's name with CSS escapes resolved, lowercased.
fn unescape_ident(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.push(chars.next().unwrap_or_default());
        }
        if hex.is_empty() {
            if let Some(c) = chars.next() {
                out.push(c);
            }
        } else {
            out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
        }
    }
    out.to_lowercase()
}

/// Removes `@import` rules (escaped spellings too) from a stylesheet: each runs to its `;`
/// outside quotes and parentheses, or to the end.
pub fn strip_imports(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(m) = AT_KEYWORD.find(rest) {
        let name = unescape_ident(&rest[m.start() + 1..m.end()]);
        if name != "import" {
            out.push_str(&rest[..m.end()]);
            rest = &rest[m.end()..];
            continue;
        }
        out.push_str(&rest[..m.start()]);
        let after = &rest[m.end()..];
        let (mut depth, mut quote, mut end) = (0i32, None::<char>, after.len());
        let mut escaped = false;
        for (i, c) in after.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match (quote, c) {
                (_, '\\') => escaped = true,
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(c),
                (None, '(') => depth += 1,
                (None, ')') => depth -= 1,
                (None, ';') if depth <= 0 => {
                    end = i + 1;
                    break;
                }
                _ => {}
            }
        }
        rest = &after[end.min(after.len())..];
    }
    out.push_str(rest);
    out
}

fn strip_style_imports(html: &str) -> String {
    if !html.contains("<style") {
        return html.to_owned();
    }
    STYLE_BLOCK
        .replace_all(html, |c: &regex::Captures| {
            format!("{}{}{}", &c[1], strip_imports(&c[2]), &c[3])
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_scripts_and_bad_urls() {
        let got = sanitize(
            "<p onclick=\"x()\">Hi</p><script>alert(1)</script><iframe src=\"https://e.example\">in</iframe>\
             <a href=\"javascript:alert(1)\">bad</a><a href=\"https://ok.example/\" target=\"_blank\">ok</a>\
             <div data-secret=\"1\" title=\"t\">d</div><img src=\"data:image/png;base64,AA\"><a href=\"data:text/html,x\">d</a>\
             <a href=\"file:///etc/passwd\">f</a><a href=\"/rel\">r</a><blink>kept</blink>",
        );
        assert_eq!(
            got,
            "<p>Hi</p><a>bad</a><a href=\"https://ok.example/\" target=\"_blank\">ok</a>\
             <div title=\"t\">d</div><img src=\"data:image/png;base64,AA\"><a>d</a><a>f</a><a href=\"/rel\">r</a>kept"
        );
    }

    #[test]
    fn keeps_head_styles_without_imports() {
        let got = sanitize(
            "<html><head><title>T</title><style>@import url(\"https://t.example/x.css\"); @\\69mport 'y.css'; td { padding: 4px }</style></head><body><p>x</p></body></html>",
        );
        assert_eq!(
            got,
            "<title>T</title><style>  td { padding: 4px }</style><p>x</p>"
        );
        assert_eq!(
            strip_imports("@media print { a { b: c } }"),
            "@media print { a { b: c } }"
        );
    }
}
