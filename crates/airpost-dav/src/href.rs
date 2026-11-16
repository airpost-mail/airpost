// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! URL and href handling for DAV requests. Derived from Mailspring-Sync
//! `MailSync/DAVWorker.cpp` (`replacePath`, `urlDecode`, `normalizeHref`, `hrefForNewEvent`),
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Hrefs from a server are echoed back unchanged in requests (multiget bodies, PUT URLs);
//! [`normalize_href`] is only for comparing them.

/// Prefixes `https://` unless the URL already starts with `http` (`performXMLRequest`).
/// Discovery hands around bare `host/path` strings, and Gmail's CalDAV host has no scheme.
pub fn with_scheme(url: &str) -> String {
    if url.starts_with("http") {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

/// Replaces everything after the authority of `url` with `path` (`replacePath`). A path
/// without a leading slash gets one. A URL with no path keeps all of itself as the base.
pub fn replace_path(url: &str, path: &str) -> String {
    let host_end = match url.find("://") {
        None => url.find('/'),
        Some(start) => url[start + 3..].find('/').map(|i| i + start + 3),
    };
    let base = match host_end {
        Some(end) => &url[..end],
        None => url,
    };
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// An href from a response as a full URL: absolute hrefs as they are, paths on `base`'s
/// authority.
pub fn resolve(base: &str, href: &str) -> String {
    if href.contains("://") {
        href.to_string()
    } else {
        replace_path(base, href)
    }
}

/// One pass of percent-decoding; `+` becomes a space as in query strings (`urlDecode`).
/// Invalid escapes are kept as they are. Bytes that don't form UTF-8 are replaced.
pub fn url_decode(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(if b == b'+' { b' ' } else { b });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// The comparable form of an href (`normalizeHref`, after python-caldav):
/// - scheme and host are dropped, since servers answer absolute URLs, absolute paths or both;
/// - percent-decoding repeats until stable (at most 5 passes), because some servers encode
///   twice (Confluence: `@` → `%2540`);
/// - trailing slashes are stripped, since collections come back with and without them.
pub fn normalize_href(href: &str) -> String {
    let mut result = href.to_string();
    if let Some(scheme_end) = result.find("://") {
        if let Some(path_start) = result[scheme_end + 3..].find('/') {
            result = result[scheme_end + 3 + path_start..].to_string();
        }
    }
    for _ in 0..5 {
        let decoded = url_decode(&result);
        if decoded == result {
            break;
        }
        result = decoded;
    }
    while result.len() > 1 && result.ends_with('/') {
        result.pop();
    }
    result
}

/// The resource href for an event that has none stored yet (`hrefForNewEvent`): exceptions
/// live inside the master's resource (RFC 4791 §4.1), so every event is `{uid}.ics`.
///
/// The UID comes from an ICS the user did not write (any invitation can be stored on a
/// calendar) and lands in a request path and a multiget body. A UID that is empty, longer
/// than 200 bytes, outside `[A-Za-z0-9._@-]` or containing `..` could address another
/// resource once the path is normalised, so the event's row id names it instead. RFC 4791
/// §5.3.2 leaves the resource name to the client. The name must be stable per event: two
/// names for one UID is a `no-uid-conflict`.
pub fn href_for_new_event(calendar_path: &str, ics_uid: &str, event_id: &str) -> String {
    let unreserved = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@');
    let safe = !ics_uid.is_empty()
        && ics_uid.len() <= 200
        && ics_uid.chars().all(unreserved)
        && !ics_uid.contains("..");
    let name = if safe { ics_uid } else { event_id };
    format!("{calendar_path}{name}.ics")
}

/// The host of a URL or bare `host/path`, lowercased, without port or credentials.
pub fn host_of(url: &str) -> String {
    let rest = url.find("://").map_or(url, |i| &url[i + 3..]);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    host.to_ascii_lowercase()
}

/// The path of a URL or bare `host/path` (`/` when there is none).
pub fn path_of(url: &str) -> String {
    let rest = url.find("://").map_or(url, |i| &url[i + 3..]);
    match rest.find('/') {
        Some(i) => rest[i..].to_string(),
        None => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_path_cases() {
        assert_eq!(
            replace_path("https://dav.example.com/root/x/", "/cal/home/"),
            "https://dav.example.com/cal/home/"
        );
        assert_eq!(
            replace_path("https://dav.example.com", "cal"),
            "https://dav.example.com/cal"
        );
        assert_eq!(replace_path("dav.example.com/", "/p"), "dav.example.com/p");
        assert_eq!(
            replace_path("apidata.googleusercontent.com", "/caldav/v2/x"),
            "apidata.googleusercontent.com/caldav/v2/x"
        );
    }

    #[test]
    fn normalize() {
        assert_eq!(
            normalize_href("https://h.example/dav/a%2540b.com/cal/"),
            "/dav/a@b.com/cal"
        );
        assert_eq!(normalize_href("/dav/a%40b.com/"), "/dav/a@b.com");
        assert_eq!(normalize_href("/"), "/");
        assert_eq!(normalize_href("/a//"), "/a");
        assert_eq!(normalize_href("/x%zz"), "/x%zz");
        assert_eq!(normalize_href("/x%4"), "/x%4");
        assert_eq!(normalize_href("/a+b"), "/a b");
    }

    #[test]
    fn new_event_hrefs() {
        assert_eq!(
            href_for_new_event("/cal/", "abc-123@host.com", "rowid"),
            "/cal/abc-123@host.com.ics"
        );
        assert_eq!(
            href_for_new_event("/cal/", "../x", "rowid"),
            "/cal/rowid.ics"
        );
        assert_eq!(
            href_for_new_event("/cal/", "a/b", "rowid"),
            "/cal/rowid.ics"
        );
        assert_eq!(href_for_new_event("/cal/", "", "rowid"), "/cal/rowid.ics");
        assert_eq!(
            href_for_new_event("/cal/", "a..b", "rowid"),
            "/cal/rowid.ics"
        );
        let long = "a".repeat(201);
        assert_eq!(
            href_for_new_event("/cal/", &long, "rowid"),
            "/cal/rowid.ics"
        );
        assert_eq!(href_for_new_event("/cal/", "ü", "rowid"), "/cal/rowid.ics");
    }

    #[test]
    fn hosts_and_paths() {
        assert_eq!(
            host_of("https://User@Caldav.Example.com:8443/x"),
            "caldav.example.com"
        );
        assert_eq!(
            host_of("apidata.googleusercontent.com/caldav"),
            "apidata.googleusercontent.com"
        );
        assert_eq!(host_of("http://[::1]:80/"), "::1");
        assert_eq!(path_of("https://h/remote.php/dav/"), "/remote.php/dav/");
        assert_eq!(path_of("https://h"), "/");
        assert_eq!(with_scheme("h/x"), "https://h/x");
        assert_eq!(with_scheme("http://h/x"), "http://h/x");
    }
}
