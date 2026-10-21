// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Tracking images left out of a new draft's quoted text (`remove-tracking-pixels`,
//! `removeTrackingPixelsFromDraft`): images from known trackers and hidden remote images. The
//! tracker rules are the desktop's vendor rules (from MailTrackerBlocker, see `data`); Rust's
//! regex engine doesn't backtrack, so the rules' `.+` stay uncapped where the desktop caps
//! them at 256 characters.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::data::DESKTOP;

const MAX_TESTED_URL: usize = 2048;

static DRAFT_TRACKERS: LazyLock<Regex> = LazyLock::new(|| {
    let joined = DESKTOP
        .tracker_rules
        .iter()
        .filter(|r| Regex::new(r).is_ok())
        .map(|r| format!("(?:{r})"))
        .collect::<Vec<_>>()
        .join("|");
    regex::RegexBuilder::new(&joined)
        .case_insensitive(true)
        .size_limit(64 << 20)
        .build()
        .expect("tracker rules")
});

static IMAGE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<(?:img|image|source|input)\b(?:[^>"']|"[^"]*"|'[^']*')*>"#).unwrap()
});
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([^\s"'<>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?"#).unwrap()
});

fn parse_tag(tag: &str) -> (String, HashMap<String, String>) {
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^<([a-z]+)").unwrap());
    let name_match = NAME.captures(tag);
    let name = name_match
        .as_ref()
        .map(|c| c[1].to_lowercase())
        .unwrap_or_default();
    let start = name_match.and_then(|c| c.get(0)).map_or(0, |m| m.end());
    let mut inner = &tag[start..];
    inner = inner.strip_suffix('>').unwrap_or(inner);
    inner = inner.strip_suffix('/').unwrap_or(inner);
    let mut attrs = HashMap::new();
    for c in ATTRIBUTE.captures_iter(inner) {
        let attr = c[1].to_lowercase();
        let value = c
            .get(2)
            .or_else(|| c.get(3))
            .or_else(|| c.get(4))
            .map_or("", |m| m.as_str());
        attrs.entry(attr).or_insert_with(|| value.to_owned());
    }
    (name, attrs)
}

/// The URLs of a `srcset`, per the HTML candidate grammar (`srcsetURLs`).
pub fn srcset_urls(srcset: &str) -> Vec<String> {
    let chars: Vec<char> = srcset.chars().collect();
    let mut urls = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') {
            i += 1;
        }
        let mut j = i;
        while j < chars.len() && !chars[j].is_whitespace() {
            j += 1;
        }
        let mut url: String = chars[i..j].iter().collect();
        if url.ends_with(',') {
            url = url.trim_end_matches(',').to_owned();
        } else {
            while j < chars.len() && chars[j] != ',' {
                j += 1;
            }
        }
        if !url.is_empty() {
            urls.push(url);
        }
        i = j;
    }
    urls
}

fn image_urls(name: &str, attrs: &HashMap<String, String>) -> Vec<String> {
    if name == "input"
        && attrs
            .get("type")
            .map(|t| t.trim().to_lowercase())
            .as_deref()
            != Some("image")
    {
        return Vec::new();
    }
    let mut urls = attrs
        .get("srcset")
        .map(|s| srcset_urls(s))
        .unwrap_or_default();
    if let Some(src) = attrs.get("src") {
        urls.insert(0, src.clone());
    }
    urls
}

/// `isRemoteURL`: anything but inline and app schemes.
pub fn is_remote_url(url: &str) -> bool {
    static LOCAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*(?:cid|data|mailspring|tern-file|file):").unwrap());
    !LOCAL.is_match(url) && !url.trim().is_empty()
}

fn parse_style(style: &str) -> HashMap<String, String> {
    static IMPORTANT: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)!important").unwrap());
    let mut decls = HashMap::new();
    for decl in style.split(';') {
        let Some(idx) = decl.find(':') else { continue };
        let prop = decl[..idx].trim().to_lowercase();
        let value = IMPORTANT
            .replace(&decl[idx + 1..], "")
            .trim()
            .to_lowercase();
        if !prop.is_empty() {
            decls.insert(prop, value);
        }
    }
    decls
}

/// JavaScript's `parseFloat`: the longest number at the start, after whitespace.
fn parse_float(s: &str) -> Option<f64> {
    static NUM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[+-]?(?:\d+\.?\d*(?:[eE][+-]?\d+)?|\.\d+(?:[eE][+-]?\d+)?)").unwrap()
    });
    NUM.find(s.trim_start())
        .and_then(|m| m.as_str().parse().ok())
}

/// `pixelSize`: `None` when unset, infinite in units other than pixels, negative as 0.
fn pixel_size(value: Option<&String>) -> Option<f64> {
    static PX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*(-?\d*\.?\d+)\s*(px)?\s*$").unwrap());
    let value = value?;
    Some(match PX.captures(value) {
        Some(c) => c[1].parse::<f64>().map_or(f64::INFINITY, |v| v.max(0.0)),
        None => f64::INFINITY,
    })
}

fn is_hidden(attrs: &HashMap<String, String>) -> bool {
    let style = parse_style(attrs.get("style").map_or("", String::as_str));
    if style.get("display").is_some_and(|v| v == "none")
        || style.get("visibility").is_some_and(|v| v == "hidden")
        || style.get("opacity").and_then(|v| parse_float(v)) == Some(0.0)
        || attrs.contains_key("hidden")
    {
        return true;
    }
    let dimension = |prop: &str| -> Option<f64> {
        let size = pixel_size(style.get(prop).or_else(|| attrs.get(prop)));
        let max = pixel_size(style.get(&format!("max-{prop}")));
        match max {
            None => size,
            Some(m) if m.is_infinite() => size,
            Some(m) => Some(size.unwrap_or(m).min(m)),
        }
    };
    let (width, height) = (dimension("width"), dimension("height"));
    if width == Some(0.0) || height == Some(0.0) {
        return true;
    }
    if width.is_none() && height.is_none() {
        return false;
    }
    width.unwrap_or(1.0) <= 1.0 && height.unwrap_or(1.0) <= 1.0
}

fn tested_prefix(url: &str) -> &str {
    match url.char_indices().nth(MAX_TESTED_URL) {
        Some((i, _)) => &url[..i],
        None => url,
    }
}

/// `isTrackingImage` with the draft rules.
pub fn is_tracking_image(tag: &str) -> bool {
    let (name, attrs) = parse_tag(tag);
    let remote: Vec<String> = image_urls(&name, &attrs)
        .into_iter()
        .filter(|u| is_remote_url(u))
        .collect();
    if remote.is_empty() {
        return false;
    }
    remote
        .iter()
        .any(|u| DRAFT_TRACKERS.is_match(tested_prefix(u)))
        || is_hidden(&attrs)
}

/// `removeTrackingPixelsFromDraft`.
pub fn remove_tracking_pixels(body: &str) -> String {
    IMAGE_TAG
        .replace_all(body, |c: &regex::Captures| {
            if is_tracking_image(&c[0]) {
                String::new()
            } else {
                c[0].to_owned()
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_compiles() {
        let bad: Vec<&String> = DESKTOP
            .tracker_rules
            .iter()
            .filter(|r| Regex::new(r).is_err())
            .collect();
        assert!(bad.is_empty(), "{bad:?}");
    }

    #[test]
    fn removes_trackers_and_hidden_images() {
        let body = "<p>News</p><img src=\"https://list-manage.com/track/open.php?u=1\" width=\"1\" height=\"1\">\
                    <img src=\"https://cdn.example/hidden.gif\" style=\"display:none\">\
                    <img src=\"https://cdn.example/photo.jpg\" width=\"300\"><img src=\"cid:x\" width=\"1\">";
        assert_eq!(
            remove_tracking_pixels(body),
            "<p>News</p><img src=\"https://cdn.example/photo.jpg\" width=\"300\"><img src=\"cid:x\" width=\"1\">"
        );
        assert_eq!(
            srcset_urls("a.png 1x, data:x,y 2x,b.png"),
            ["a.png", "data:x,y", "b.png"]
        );
    }
}
