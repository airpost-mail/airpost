// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `mailto:` links as `DraftFactory.createDraftForMailto` reads them, malformed ones included:
//! many links carry an unescaped URL in `body=`, so the query is split only at the parameters a
//! draft takes, and a value is decoded only when it looks entirely escaped.

use std::sync::LazyLock;

use regex::Regex;

use crate::contacts::{parse_contacts_in_string, Contact};

/// What a `mailto:` link asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: Vec<Contact>,
    pub cc: Vec<Contact>,
    pub bcc: Vec<Contact>,
    pub subject: Option<String>,
    /// As written in the link: plain text, newlines and all.
    pub body: Option<String>,
}

/// JavaScript's `decodeURI` (`reserved` keeps escapes of `;/?:@&=+$,#`) and
/// `decodeURIComponent`. `None` where JavaScript throws.
fn decode(s: &str, keep_reserved: bool) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while i < bytes.len() {
        if bytes[i] != b'%' {
            let c = s[i..].chars().next()?;
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let byte_at = |j: usize| -> Option<u8> {
            if bytes.get(j) != Some(&b'%') {
                return None;
            }
            Some(hex(*bytes.get(j + 1)?)? << 4 | hex(*bytes.get(j + 2)?)?)
        };
        let first = byte_at(i)?;
        let len = match first {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => return None,
        };
        let mut buf = vec![first];
        for k in 1..len {
            buf.push(byte_at(i + 3 * k)?);
        }
        let decoded = std::str::from_utf8(&buf).ok()?;
        if keep_reserved && len == 1 && ";/?:@&=+$,#".contains(decoded) {
            out.push_str(&s[i..i + 3]);
        } else {
            out.push_str(decoded);
        }
        i += 3 * len;
    }
    Some(out)
}

pub fn decode_uri(s: &str) -> Option<String> {
    decode(s, true)
}

pub fn decode_uri_component(s: &str) -> Option<String> {
    decode(s, false)
}

/// Parses a `mailto:` link; `None` when it isn't one.
pub fn parse_mailto(url: &str) -> Option<Mailto> {
    static MAILTO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)mailto:/*([^?&]*)(.*)").unwrap());
    static SPLIT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)[&|?](subject|body|cc|to|from|bcc)+\s*=").unwrap());
    let url = decode_uri(url).unwrap_or_else(|| url.to_owned());
    let caps = MAILTO.captures(&url)?;
    let mut to = caps[1].to_owned();
    let query = caps.get(2).map_or("", |m| m.as_str());
    if !to.is_empty() && !to.contains('@') {
        to = decode_uri_component(&to)?;
    }

    let mut fields: Vec<(String, String)> = vec![("to".into(), to)];
    let matches: Vec<(usize, usize, String)> = SPLIT
        .captures_iter(query)
        .map(|c| {
            let m = c.get(0).expect("match");
            (m.start(), m.end(), c[1].to_lowercase())
        })
        .collect();
    for (n, (_, value_start, key)) in matches.iter().enumerate() {
        let value_end = matches.get(n + 1).map_or(query.len(), |m| m.0);
        let mut value = query[*value_start..value_end].to_owned();
        if !value.contains('?') && !value.contains('&') {
            if let Some(d) = decode_uri_component(&value) {
                value = d;
            }
        }
        fields.retain(|(k, _)| k != key);
        fields.push((key.clone(), value));
    }
    let get = |k: &str| {
        fields
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
    };
    let contacts = |k: &str| {
        get(k)
            .filter(|v| !v.is_empty())
            .map(|v| parse_contacts_in_string(&v))
            .unwrap_or_default()
    };
    Some(Mailto {
        to: contacts("to"),
        cc: contacts("cc"),
        bcc: contacts("bcc"),
        subject: get("subject"),
        body: get("body"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_like_javascript() {
        assert_eq!(decode_uri("a%20b%3Fc%C3%A9").as_deref(), Some("a b%3Fcé"));
        assert_eq!(decode_uri_component("a%20b%3Fc").as_deref(), Some("a b?c"));
        assert_eq!(decode_uri_component("100%"), None);
        assert_eq!(decode_uri_component("%C3"), None);
    }

    #[test]
    fn parses_links() {
        let m = parse_mailto("mailto:someone@example.net?subject=Hello%20there&body=Line%201%0ALine%202&cc=a%40b.example").unwrap();
        assert_eq!(m.to[0].email, "someone@example.net");
        assert_eq!(m.cc[0].email, "a@b.example");
        assert_eq!(m.subject.as_deref(), Some("Hello there"));
        assert_eq!(m.body.as_deref(), Some("Line 1\nLine 2"));
        let m = parse_mailto(
            "mailto:x@y.example?body=https://github.com/x/issues?utf8=&q=is%3Aopen&subject=Bug",
        )
        .unwrap();
        assert_eq!(
            m.body.as_deref(),
            Some("https://github.com/x/issues?utf8=&q=is%3Aopen")
        );
        assert_eq!(m.subject.as_deref(), Some("Bug"));
        assert!(parse_mailto("https://example.com").is_none());
    }
}
