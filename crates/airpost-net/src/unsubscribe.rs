// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Unsubscribing from a list by its `List-Unsubscribe` header (RFC 2369), one-click by POST
//! (RFC 8058) or by the message a `mailto:` URI describes (RFC 6068). The choice follows the
//! desktop's `list-unsubscribe` plugin (`unsubscribe-service.ts`): one-click when
//! `List-Unsubscribe-Post` offers it and there is an https URI, else mailto, else a web link.
//!
//! The one-click URI comes from mail anyone can send, and the engine may run on a server, so
//! the POST goes only to hosts whose every address is public, connects to the address it
//! checked (directly, never through a proxy from the environment, which would look the host up
//! again), and follows no redirect.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use reqwest::Url;

/// RFC 8058 section 3.1: the exact `List-Unsubscribe-Post` value, and the POST body.
pub const ONE_CLICK_BODY: &str = "List-Unsubscribe=One-Click";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POST_TIMEOUT: Duration = Duration::from_secs(30);

/// A mailto URI with more recipients than this is refused: an unsubscribe goes to the list.
pub const MAILTO_MAX_RECIPIENTS: usize = 5;
const MAILTO_MAX_SUBJECT_CHARS: usize = 500;
const MAILTO_MAX_BODY_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    OneClick,
    Mailto,
    Web,
}

impl Method {
    /// The `messages.unsubscribe` name.
    pub fn name(self) -> &'static str {
        match self {
            Method::OneClick => "one-click",
            Method::Mailto => "mailto",
            Method::Web => "web",
        }
    }

    pub fn from_name(name: &str) -> Option<Method> {
        match name {
            "one-click" => Some(Method::OneClick),
            "mailto" => Some(Method::Mailto),
            "web" => Some(Method::Web),
            _ => None,
        }
    }
}

/// The URIs of a `List-Unsubscribe` value: each inside `<…>`, in header order, http(s) and
/// mailto only (`parseListUnsubscribeHeader`). A URI with a control character is dropped: the
/// header is stored as received, and the URI goes back to clients that may print it.
pub fn header_uris(header: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else { break };
        // Folded headers may leave whitespace inside the brackets (RFC 2369 section 2).
        let uri: String = after[..end].split_whitespace().collect();
        let lower = uri.to_ascii_lowercase();
        let scheme = ["https://", "http://", "mailto:"]
            .iter()
            .any(|p| lower.starts_with(p));
        if scheme && !uri.chars().any(char::is_control) {
            out.push(uri);
        }
        rest = &after[end + 1..];
    }
    out
}

/// Whether `List-Unsubscribe-Post` asks for one-click (RFC 8058 section 3.1).
pub fn offers_one_click(post: Option<&str>) -> bool {
    post.is_some_and(|p| p.trim() == ONE_CLICK_BODY)
}

/// The method and URI to use: `wanted`, or the preferred one the header offers. `None` when
/// the header has nothing for it. `allow_http` (tests only, with [`one_click`]'s
/// `allow_local`) lets an http URI be used for one-click.
pub fn choose(
    header: Option<&str>,
    post: Option<&str>,
    wanted: Option<Method>,
    allow_http: bool,
) -> Option<(Method, String)> {
    let uris = header_uris(header.unwrap_or(""));
    let has = |prefix: &str| {
        uris.iter()
            .find(|u| u.to_ascii_lowercase().starts_with(prefix))
            .cloned()
    };
    let one_click = || {
        offers_one_click(post)
            .then(|| has("https://").or_else(|| allow_http.then(|| has("http://")).flatten()))
            .flatten()
            .map(|u| (Method::OneClick, u))
    };
    let mailto = || has("mailto:").map(|u| (Method::Mailto, u));
    let web = || {
        has("https://")
            .or_else(|| has("http://"))
            .map(|u| (Method::Web, u))
    };
    match wanted {
        Some(Method::OneClick) => one_click(),
        Some(Method::Mailto) => mailto(),
        Some(Method::Web) => web(),
        None => one_click().or_else(mailto).or_else(web),
    }
}

/// The message a mailto URI describes. Header fields other than `to`, `cc`, `subject` and
/// `body` are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// Parses a `mailto:` URI (RFC 6068). `Err` says why it can't be sent.
pub fn parse_mailto(uri: &str) -> Result<Mailto, String> {
    let rest = uri
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("mailto:"))
        .map(|_| &uri[7..])
        .ok_or("not a mailto URI")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut m = Mailto::default();
    let add = |list: &mut Vec<String>, raw: &str| -> Result<(), String> {
        for addr in percent_decode(raw)?.split(',') {
            let addr = addr.trim();
            if !addr.is_empty() {
                list.push(valid_address(addr)?);
            }
        }
        Ok(())
    };
    add(&mut m.to, path)?;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key.to_ascii_lowercase().as_str() {
            "to" => add(&mut m.to, value)?,
            "cc" => add(&mut m.cc, value)?,
            "subject" => m.subject = percent_decode(value)?,
            "body" => m.body = percent_decode(value)?,
            _ => {}
        }
    }
    if m.to.is_empty() {
        return Err("the mailto URI names no recipient".into());
    }
    if m.to.len() + m.cc.len() > MAILTO_MAX_RECIPIENTS {
        return Err(format!(
            "the mailto URI names more than {MAILTO_MAX_RECIPIENTS} recipients"
        ));
    }
    // A subject is one header line.
    m.subject = m
        .subject
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAILTO_MAX_SUBJECT_CHARS)
        .collect::<String>()
        .trim()
        .to_owned();
    if m.body.len() > MAILTO_MAX_BODY_BYTES {
        return Err("the mailto URI's body is too long".into());
    }
    Ok(m)
}

fn valid_address(addr: &str) -> Result<String, String> {
    let ok = addr.len() <= 254
        && addr
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
        && addr.matches('@').count() == 1
        && !addr
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>()[],;:\"\\".contains(c));
    if ok {
        Ok(addr.to_owned())
    } else {
        Err(format!("{addr:?} is not an email address"))
    }
}

/// `%XX` decoding as UTF-8. RFC 6068 has no `+` for space.
fn percent_decode(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or("the mailto URI has a broken %-escape")?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "the mailto URI is not UTF-8".to_owned())
}

/// Whether a one-click POST may go to `ip`: not loopback, private, link-local, shared,
/// multicast, documentation or otherwise reserved space.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => is_public_v4(a),
        IpAddr::V6(a) => is_public_v6(a),
    }
}

fn is_public_v4(a: Ipv4Addr) -> bool {
    let [b0, b1, b2, _] = a.octets();
    !(a.is_private()
        || a.is_loopback()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || a.is_documentation()
        || b0 == 0
        || (b0 == 100 && (64..128).contains(&b1))
        || (b0 == 192 && b1 == 0 && b2 == 0)
        || (b0 == 198 && (18..20).contains(&b1))
        || b0 >= 240)
}

fn is_public_v6(a: Ipv6Addr) -> bool {
    if let Some(v4) = a.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let s = a.segments();
    let v4 = |hi: u16, lo: u16| {
        let [x, y] = [hi.to_be_bytes(), lo.to_be_bytes()];
        Ipv4Addr::new(x[0], x[1], y[0], y[1])
    };
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) embed an IPv4 address.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return is_public_v4(v4(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return is_public_v4(v4(s[1], s[2]));
    }
    !(a.is_loopback()
        || a.is_unspecified()
        || a.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00
        // Link-local fe80::/10 and the deprecated site-local fec0::/10.
        || (s[0] & 0xff80) == 0xfe80
        || (s[0] == 0x2001 && s[1] == 0x0db8)
        // Teredo (2001::/32) tunnels to an IPv4 address.
        || (s[0] == 0x2001 && s[1] == 0)
        // Local-use NAT64 (64:ff9b:1::/48).
        || s[..3] == [0x64, 0xff9b, 1]
        // Discard-only (100::/64).
        || s[..4] == [0x100, 0, 0, 0]
        // IPv4-compatible (::/96) and IPv4-translated (::ffff:0:0/96).
        || s[..6] == [0, 0, 0, 0, 0, 0]
        || s[..6] == [0, 0, 0, 0, 0xffff, 0])
}

/// RFC 8058 one-click: a POST of [`ONE_CLICK_BODY`] to `url`, without cookies or other
/// credentials and without following redirects. `allow_local` lets tests use plain http and
/// local addresses. `Err` says why it didn't work.
pub fn one_click(url: &str, allow_local: bool) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|e| format!("the URI is not valid: {e}"))?;
    match parsed.scheme() {
        "https" => {}
        "http" if allow_local => {}
        _ => return Err("one-click unsubscribe needs an https URI".into()),
    }
    let host = parsed
        .host_str()
        .ok_or("the URI has no host")?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("could not resolve {host}"));
    }
    if !allow_local && !addrs.iter().all(|a| is_public(a.ip())) {
        return Err(format!(
            "{host} is a local or private address; one-click unsubscribe only goes to public hosts"
        ));
    }
    crate::http::ensure_crypto_provider();
    let mut builder = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(POST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .tls_backend_rustls();
    if host.parse::<IpAddr>().is_err() {
        // Connect to the addresses checked above, not to a second lookup's.
        builder = builder.resolve_to_addrs(&host, &addrs);
    }
    let client = builder
        .build()
        .map_err(|e| format!("could not create the HTTP client: {e}"))?;
    let resp = client
        .post(parsed)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(USER_AGENT, "Airpost")
        .body(ONE_CLICK_BODY)
        .send()
        .map_err(|e| format!("the request failed: {}", without_url(&e)))?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else if status.is_redirection() {
        Err(format!(
            "the server answered {} (a redirect), which one-click unsubscribe doesn't follow",
            status.as_u16()
        ))
    } else {
        Err(format!("the server answered {status}"))
    }
}

/// reqwest's error text without the URL, which the caller already has.
fn without_url(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    if let Some(u) = e.url() {
        text = text.replace(&format!(" for url ({u})"), "");
    }
    if let Some(source) = std::error::Error::source(e) {
        text = format!("{text}: {source}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str =
        "<mailto:leave@lists.example.com?subject=unsubscribe>, \n <https://example.com/u/1>";

    #[test]
    fn header_uris_keep_order_and_drop_other_schemes() {
        assert_eq!(
            header_uris(LIST),
            [
                "mailto:leave@lists.example.com?subject=unsubscribe",
                "https://example.com/u/1"
            ]
        );
        assert_eq!(
            header_uris("<ftp://x>, <HTTP://a.example/u>, <https://b.example/\n  long>"),
            ["HTTP://a.example/u", "https://b.example/long"]
        );
        assert!(header_uris("no brackets").is_empty());
        // A URI carrying a terminal escape (here an OSC 8 hyperlink) is dropped.
        assert_eq!(
            header_uris(
                "<https://a.example/\x1b]8;;https://evil.example\x1b\\x>, <https://b.example/u\u{9b}2J>, <https://c.example/u>"
            ),
            ["https://c.example/u"]
        );
        assert!(header_uris("<https://unterminated").is_empty());
    }

    #[test]
    fn choice_follows_the_desktop_order() {
        let post = Some(" List-Unsubscribe=One-Click ");
        assert_eq!(
            choose(Some(LIST), post, None, false),
            Some((Method::OneClick, "https://example.com/u/1".into()))
        );
        assert_eq!(
            choose(Some(LIST), None, None, false),
            Some((
                Method::Mailto,
                "mailto:leave@lists.example.com?subject=unsubscribe".into()
            ))
        );
        assert_eq!(
            choose(Some(LIST), post, Some(Method::Web), false),
            Some((Method::Web, "https://example.com/u/1".into()))
        );
        // One-click needs https.
        assert_eq!(
            choose(Some("<http://example.com/u>"), post, None, false),
            Some((Method::Web, "http://example.com/u".into()))
        );
        assert_eq!(
            choose(
                Some("<http://example.com/u>"),
                post,
                Some(Method::OneClick),
                false
            ),
            None
        );
        assert_eq!(
            choose(
                Some("<https://x.example>"),
                Some("List-Unsubscribe=Yes"),
                Some(Method::OneClick),
                false
            ),
            None
        );
        assert_eq!(choose(None, post, None, false), None);
        assert_eq!(
            choose(Some("<http://127.0.0.1:9/u>"), post, None, true),
            Some((Method::OneClick, "http://127.0.0.1:9/u".into()))
        );
        assert_eq!(Method::from_name("one-click"), Some(Method::OneClick));
        assert_eq!(Method::from_name("body-link"), None);
    }

    #[test]
    fn mailto_fields() {
        let m = parse_mailto(
            "MAILTO:leave%2Bme@lists.example.com,two@example.org?Subject=Please%20remove%0D%0ABcc:%20x@y.z&body=line%201%0Aline%202&cc=c@example.net&in-reply-to=x",
        )
        .unwrap();
        assert_eq!(m.to, ["leave+me@lists.example.com", "two@example.org"]);
        assert_eq!(m.cc, ["c@example.net"]);
        assert_eq!(m.subject, "Please remove  Bcc: x@y.z");
        assert_eq!(m.body, "line 1\nline 2");
        let m = parse_mailto("mailto:?to=a@example.com").unwrap();
        assert_eq!(m.to, ["a@example.com"]);
        assert!(m.subject.is_empty());
    }

    #[test]
    fn bad_mailtos() {
        for uri in [
            "https://example.com",
            "mailto:",
            "mailto:nobody",
            "mailto:a@b",
            "mailto:a@example.com%0D%0ABcc:b@example.com",
            "mailto:a%ZZ@example.com",
            "mailto:a@example.com?body=%FF",
            "mailto:a@x.io,b@x.io,c@x.io,d@x.io,e@x.io,f@x.io",
        ] {
            assert!(parse_mailto(uri).is_err(), "{uri}");
        }
    }

    #[test]
    fn public_addresses() {
        for ip in [
            "93.184.216.34",
            "2606:2800:220:1:248:1893:25c8:1946",
            "64:ff9b::5db8:d822",
            "2002:5db8:d822::1",
            "2003::1",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "::7f00:1",
            "2002:7f00:1::",
            "2002:a00:1::1",
            "2002:c0a8:101::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001::1",
            "::ffff:0:7f00:1",
            "::ffff:0:a00:1",
            "fec0::1",
            "feff::1",
            "100::1",
            "64:ff9b:1::5db8:d822",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn one_click_refuses_before_connecting() {
        assert!(one_click("http://example.com/u", false)
            .unwrap_err()
            .contains("https"));
        assert!(one_click("https://127.0.0.1/u", false)
            .unwrap_err()
            .contains("local"));
        assert!(one_click("https://[::1]:8443/u", false)
            .unwrap_err()
            .contains("local"));
        assert!(one_click("https://localhost/u", false)
            .unwrap_err()
            .contains("local"));
        assert!(one_click("mailto:a@b.example", true).is_err());
        assert!(one_click("not a url", false).is_err());
    }

    /// A one-request HTTP server answering `status`; returns its URL and the request it read.
    fn serve_once(status: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/u/1?list=7", listener.local_addr().unwrap());
        let status = status.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = conn.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let reply = format!("HTTP/1.1 {status}\r\nLocation: http://127.0.0.1:1/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            conn.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn one_click_posts_the_rfc_8058_body() {
        let (url, server) = serve_once("200 OK");
        one_click(&url, true).unwrap();
        let request = server.join().unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(
            request.starts_with("POST /u/1?list=7 HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            lower.contains("content-type: application/x-www-form-urlencoded\r\n"),
            "{request}"
        );
        assert!(
            request.ends_with("\r\n\r\nList-Unsubscribe=One-Click"),
            "{request}"
        );
        assert!(
            !lower.contains("cookie") && !lower.contains("authorization"),
            "{request}"
        );
    }

    #[test]
    fn one_click_reports_redirects_and_errors() {
        let (url, server) = serve_once("302 Found");
        assert!(one_click(&url, true).unwrap_err().contains("redirect"));
        server.join().unwrap();
        let (url, server) = serve_once("500 Internal Server Error");
        assert!(one_click(&url, true).unwrap_err().contains("500"));
        server.join().unwrap();
        // Local addresses are refused before any connection when not allowed.
        let (url, _server) = serve_once("200 OK");
        assert!(one_click(&url.replace("http:", "https:"), false)
            .unwrap_err()
            .contains("local"));
    }
}
