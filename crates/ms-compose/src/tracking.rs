// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Open and link tracking on outgoing mail, in the Tern URL format (docs/backend-plan.md 3.6,
//! docs/desktop-client-plan.md T22). The engine applies it when it sends a draft whose
//! `open-tracking` or `link-tracking` metadata is on and a Tern server with tracking is
//! connected; the desktop's composer extensions (`open-tracking-composer-extension.ts`,
//! `link-tracking-composer-extension.ts`) did the same with Mailspring's URLs.
//!
//! Each recipient gets their own body: the pixel
//! `<public_url>/t/o/<ns>/<trackingId>.<rid>.png` goes before the first `.gmail_quote` (or at
//! the end), and every `http(s)` link outside `.gmail_quote` becomes
//! `<public_url>/t/l/<ns>/<trackingId>/<i>.<rid>?u=<original>`. The sent copy keeps the
//! untracked body, so the sender never trips their own pixel or links. `trackingId` (128 bits)
//! and each `rid` (64 bits) are random base64url; the recipient's address is never in a URL.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use lol_html::html_content::{ContentType, Element};
use lol_html::{element, rewrite_str, RewriteStrSettings};
use serde_json::{json, Map, Value};

pub const OPEN_TRACKING: &str = "open-tracking";
pub const LINK_TRACKING: &str = "link-tracking";

/// The server checks these before it records anything (3.6 "Input validation"): a link index
/// below 1000 and an original URL of at most 2 KiB. Links past them stay as written.
pub const MAX_LINKS: usize = 1000;
pub const MAX_URL_BYTES: usize = 2048;

/// Where the URLs point: the server's `publicURL` (no trailing slash) and the user's
/// `trackingNS`, both from `/tern/v1/me`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub public_url: String,
    pub ns: String,
}

/// What to rewrite in a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kinds {
    pub open: bool,
    pub links: bool,
}

/// A random 128-bit id, 22 base64url characters.
pub fn new_tracking_id() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>())
}

/// A random 64-bit recipient id, 11 base64url characters.
pub fn new_rid() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 8]>())
}

pub fn pixel_url(target: &Target, tracking_id: &str, rid: &str) -> String {
    format!(
        "{}/t/o/{}/{tracking_id}.{rid}.png",
        target.public_url, target.ns
    )
}

pub fn link_url(target: &Target, tracking_id: &str, index: usize, rid: &str, url: &str) -> String {
    format!(
        "{}/t/l/{}/{tracking_id}/{index}.{rid}?u={}",
        target.public_url,
        target.ns,
        encode_uri_component(url)
    )
}

/// One recipient's body. `links` is the message's list of tracked URLs, shared by every
/// recipient's body: a URL already in it keeps its index, a new one is appended, so the
/// `link-tracking` value has one entry per distinct URL. `Err` is a body lol_html can't
/// rewrite; the caller sends that message untracked.
pub fn track_body(
    html: &str,
    target: &Target,
    tracking_id: &str,
    rid: &str,
    kinds: Kinds,
    links: &mut Vec<String>,
) -> Result<String, String> {
    let links = RefCell::new(links);
    let in_quote = Cell::new(false);
    let pixel_placed = Cell::new(!kinds.open);
    // The pixel goes in as raw HTML, so its URL (from the server) is escaped here; lol_html's
    // set_attribute does that for the link hrefs.
    let pixel = format!(
        r#"<img alt="" src="{}">"#,
        escape_attribute(&pixel_url(target, tracking_id, rid))
    );

    let mut settings = RewriteStrSettings::new().with_strict(false);
    if kinds.open {
        settings = settings.append_element_content_handler(element!(
            ".gmail_quote",
            |el: &mut Element| {
                if !pixel_placed.replace(true) {
                    el.before(&pixel, ContentType::Html);
                }
                Ok(())
            }
        ));
    }
    if kinds.links {
        // Runs before the `a[href]` handler on the same element, and marks quoted links.
        settings = settings
            .append_element_content_handler(element!(
                ".gmail_quote a[href]",
                |_el: &mut Element| {
                    in_quote.set(true);
                    Ok(())
                }
            ))
            .append_element_content_handler(element!("a[href]", |el: &mut Element| {
                if in_quote.replace(false) {
                    return Ok(());
                }
                let Some(raw) = el.get_attribute("href") else {
                    return Ok(());
                };
                let url = decode_attribute(&raw).trim().to_owned();
                if !is_trackable(&url) {
                    return Ok(());
                }
                let mut links = links.borrow_mut();
                let index = match links.iter().position(|u| *u == url) {
                    Some(i) => i,
                    None if links.len() < MAX_LINKS => {
                        links.push(url.clone());
                        links.len() - 1
                    }
                    None => return Ok(()),
                };
                let tracked = link_url(target, tracking_id, index, rid, &url);
                el.set_attribute("href", &tracked.replace('&', "&amp;"))?;
                Ok(())
            }));
    }
    let mut out = rewrite_str(html, settings).map_err(|e| e.to_string())?;
    if !pixel_placed.get() {
        out.push_str(&pixel);
    }
    Ok(out)
}

/// A draft's send with tracking applied.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackedSend {
    /// `SendDraftTask.perRecipientBodies`: `self` (the sent copy) and one body per recipient.
    pub bodies: Map<String, Value>,
    /// The new `open-tracking` / `link-tracking` values for the draft's metadata, which the
    /// send moves to the sent message and pushes to the server.
    pub values: Vec<(&'static str, Value)>,
}

/// Whether `value` (a plugin's metadata value on a draft) turns tracking on: the composer's
/// toggles write an object when on and `null` when off.
pub fn is_on(value: Option<&Value>) -> bool {
    value.is_some_and(|v| v.as_object().is_some_and(|o| !o.is_empty()))
}

fn plugin_value<'a>(draft: &'a Value, plugin_id: &str) -> Option<&'a Value> {
    draft
        .get("metadata")?
        .as_array()?
        .iter()
        .find(|m| m.get("pluginId").and_then(Value::as_str) == Some(plugin_id))
        .map(|m| &m["value"])
}

/// What `draft` (its Message JSON) tracks: `None` for a plaintext draft or one with neither
/// toggle on.
pub fn kinds_of(draft: &Value) -> Option<Kinds> {
    if draft.get("plaintext").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let kinds = Kinds {
        open: is_on(plugin_value(draft, OPEN_TRACKING)),
        links: is_on(plugin_value(draft, LINK_TRACKING)),
    };
    (kinds.open || kinds.links).then_some(kinds)
}

/// The draft's To, Cc and Bcc addresses, each once (compared case-insensitively).
fn recipients(draft: &Value) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    ["to", "cc", "bcc"]
        .iter()
        .filter_map(|k| draft.get(*k).and_then(Value::as_array))
        .flatten()
        .filter_map(|c| c.get("email").and_then(Value::as_str))
        .map(|e| e.trim().to_owned())
        .filter(|e| !e.is_empty() && seen.insert(e.to_ascii_lowercase()))
        .collect()
}

/// Tracks a send of `draft` whose sent copy is `self_body`. `per_recipient` holds the bodies a
/// client's send hooks made (keys exactly the recipients, checked by the caller), which are
/// tracked in place of `self_body`. `None` when the draft tracks nothing or has no recipients;
/// `Err` when a body can't be rewritten.
pub fn track_send(
    draft: &Value,
    self_body: &str,
    per_recipient: &BTreeMap<String, String>,
    target: &Target,
) -> Option<Result<TrackedSend, String>> {
    let kinds = kinds_of(draft)?;
    let addresses: Vec<String> = if per_recipient.is_empty() {
        recipients(draft)
    } else {
        per_recipient
            .keys()
            .filter(|k| k.as_str() != "self")
            .cloned()
            .collect()
    };
    if addresses.is_empty() {
        return None;
    }
    let tracking_id = new_tracking_id();
    let mut links = Vec::new();
    let mut by_rid = Map::new();
    let mut bodies = Map::new();
    bodies.insert("self".into(), json!(self_body));
    for address in &addresses {
        let rid = new_rid();
        let base = per_recipient.get(address).map_or(self_body, String::as_str);
        match track_body(base, target, &tracking_id, &rid, kinds, &mut links) {
            Ok(body) => bodies.insert(address.clone(), json!(body)),
            Err(e) => return Some(Err(e)),
        };
        by_rid.insert(rid, json!(address));
    }
    let uid = draft.get("hMsgId").cloned().unwrap_or(Value::Null);
    let mut values = Vec::new();
    let updated = |plugin_id: &str| {
        let mut v = plugin_value(draft, plugin_id)
            .cloned()
            .unwrap_or_else(|| json!({}));
        v["trackingId"] = json!(tracking_id);
        v["recipients"] = Value::Object(by_rid.clone());
        v["uid"] = uid.clone();
        v
    };
    if kinds.open {
        let mut v = updated(OPEN_TRACKING);
        if v.get("open_count").is_none() {
            v["open_count"] = json!(0);
        }
        if v.get("open_data").is_none() {
            v["open_data"] = json!([]);
        }
        values.push((OPEN_TRACKING, v));
    }
    if kinds.links {
        let mut v = updated(LINK_TRACKING);
        v["links"] = links
            .iter()
            .map(|url| json!({"url": url, "click_count": 0, "click_data": []}))
            .collect();
        values.push((LINK_TRACKING, v));
    }
    Some(Ok(TrackedSend { bodies, values }))
}

/// Sets `plugin_id`'s value in a Message JSON's `metadata`, adding the entry when missing.
pub fn set_plugin_value(draft: &mut Value, plugin_id: &str, value: Value) {
    if !draft.get("metadata").is_some_and(Value::is_array) {
        draft["metadata"] = json!([]);
    }
    let list = draft["metadata"].as_array_mut().expect("an array");
    match list
        .iter_mut()
        .find(|m| m.get("pluginId").and_then(Value::as_str) == Some(plugin_id))
    {
        Some(entry) => entry["value"] = value,
        None => list.push(json!({"pluginId": plugin_id, "value": value, "v": 0})),
    }
}

/// `http:` and `https:` links the server will redirect to.
fn is_trackable(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")) && url.len() <= MAX_URL_BYTES
}

/// `encodeURIComponent`.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// An attribute value as lol_html returns it (entities kept), decoded the way a browser's
/// `getAttribute` would, by the HTML tokenizer.
fn escape_attribute(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn decode_attribute(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_owned();
    }
    let quote = if raw.contains('"') { '\'' } else { '"' };
    let sink = AttrSink(RefCell::new(None));
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from_slice(&format!(
        "<a href={quote}{raw}{quote}>"
    )));
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    tokenizer
        .sink
        .0
        .into_inner()
        .unwrap_or_else(|| raw.to_owned())
}

struct AttrSink(RefCell<Option<String>>);

impl TokenSink for AttrSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        if let Token::TagToken(tag) = token {
            if tag.kind == TagKind::StartTag && self.0.borrow().is_none() {
                *self.0.borrow_mut() = tag
                    .attrs
                    .iter()
                    .find(|a| &*a.name.local == "href")
                    .map(|a| a.value.to_string());
            }
        }
        TokenSinkResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target {
            public_url: "https://t.example".into(),
            ns: "N".repeat(22),
        }
    }

    const BOTH: Kinds = Kinds {
        open: true,
        links: true,
    };

    fn track(html: &str, kinds: Kinds, links: &mut Vec<String>) -> String {
        track_body(html, &target(), "TID", "RID", kinds, links).unwrap()
    }

    #[test]
    fn ids_have_the_server_lengths() {
        let id = new_tracking_id();
        assert_eq!(id.len(), 22);
        assert!(id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(new_rid().len(), 11);
        assert_ne!(new_tracking_id(), new_tracking_id());
    }

    #[test]
    fn the_pixel_goes_before_the_first_quote() {
        let html = r#"<p>Hi</p><div class="gmail_quote_attribution">On Monday</div><blockquote class="gmail_quote"><div class="gmail_quote">x</div></blockquote>"#;
        let out = track(
            html,
            Kinds {
                open: true,
                links: false,
            },
            &mut Vec::new(),
        );
        let pixel = format!(
            r#"<img alt="" src="https://t.example/t/o/{}/TID.RID.png">"#,
            "N".repeat(22)
        );
        assert_eq!(
            out,
            format!(
                r#"<p>Hi</p><div class="gmail_quote_attribution">On Monday</div>{pixel}<blockquote class="gmail_quote"><div class="gmail_quote">x</div></blockquote>"#
            )
        );
    }

    #[test]
    fn a_hostile_public_url_stays_inside_the_pixel_src() {
        let hostile = Target {
            public_url: r#"https://t.example/a"><a/href="https://evil">Pay/now</a><img/src="x"#
                .into(),
            ns: "N".repeat(22),
        };
        let none = Kinds {
            open: true,
            links: false,
        };
        let out = track_body("<p>hi</p>", &hostile, "TID", "RID", none, &mut vec![]).unwrap();
        assert_eq!(
            out,
            "<p>hi</p><img alt=\"\" src=\"https://t.example/a&quot;&gt;&lt;a/href=&quot;https://evil&quot;&gt;Pay/now&lt;/a&gt;&lt;img/src=&quot;x/t/o/NNNNNNNNNNNNNNNNNNNNNN/TID.RID.png\">"
        );
        let quoted = r#"<p>hi</p><div class="gmail_quote">q</div>"#;
        let out = track_body(quoted, &hostile, "TID", "RID", none, &mut vec![]).unwrap();
        assert!(!out.contains("<a/href"), "{out}");
    }

    #[test]
    fn without_a_quote_the_pixel_goes_at_the_end() {
        let out = track(
            "<p>Hi</p>",
            Kinds {
                open: true,
                links: false,
            },
            &mut Vec::new(),
        );
        assert!(out.starts_with("<p>Hi</p><img alt=\"\" src=\"https://t.example/t/o/"));
        assert!(out.ends_with("/TID.RID.png\">"));
    }

    #[test]
    fn links_outside_the_quote_are_rewritten_and_numbered_by_url() {
        let html = concat!(
            r#"<p><a href="https://a.example/x?q=1&amp;r=2">a</a> <a href="mailto:x@y">m</a> "#,
            r#"<a href="HTTP://b.example">b</a> <a href="https://a.example/x?q=1&amp;r=2">again</a></p>"#,
            r#"<blockquote class="gmail_quote"><p><a href="https://quoted.example">q</a></p></blockquote>"#,
            r#"<a href="https://after.example">after</a>"#,
        );
        let mut links = Vec::new();
        let out = track(html, BOTH, &mut links);
        assert_eq!(
            links,
            [
                "https://a.example/x?q=1&r=2",
                "HTTP://b.example",
                "https://after.example"
            ]
        );
        let ns = "N".repeat(22);
        assert!(out.contains(&format!(
            r#"href="https://t.example/t/l/{ns}/TID/0.RID?u=https%3A%2F%2Fa.example%2Fx%3Fq%3D1%26r%3D2""#
        )));
        assert_eq!(out.matches("/TID/0.RID?u=").count(), 2);
        assert!(out.contains("/TID/1.RID?u=HTTP%3A%2F%2Fb.example"));
        assert!(out.contains("/TID/2.RID?u=https%3A%2F%2Fafter.example"));
        assert!(out.contains(r#"<a href="mailto:x@y">m</a>"#));
        assert!(out.contains(r#"<a href="https://quoted.example">q</a>"#));
        // The pixel sits before the quote, untouched markup stays byte for byte.
        assert!(out.contains(r#".png"><blockquote class="gmail_quote"><p>"#));
    }

    #[test]
    fn a_second_recipient_shares_the_link_indices() {
        let mut links = Vec::new();
        track(r#"<a href="https://a.example">a</a>"#, BOTH, &mut links);
        let out = track_body(
            r#"<a href="https://b.example">b</a><a href="https://a.example">a</a>"#,
            &target(),
            "TID",
            "R2",
            BOTH,
            &mut links,
        )
        .unwrap();
        assert_eq!(links, ["https://a.example", "https://b.example"]);
        assert!(out.contains("/TID/1.R2?u=https%3A%2F%2Fb.example"));
        assert!(out.contains("/TID/0.R2?u=https%3A%2F%2Fa.example"));
    }

    #[test]
    fn long_urls_and_links_past_the_limit_stay() {
        let long = format!("https://l.example/{}", "x".repeat(MAX_URL_BYTES));
        let mut links: Vec<String> = (0..MAX_LINKS).map(|i| format!("https://{i}")).collect();
        let html = format!(r#"<a href="{long}">l</a><a href="https://new.example">n</a>"#);
        let out = track(
            &html,
            Kinds {
                open: false,
                links: true,
            },
            &mut links,
        );
        assert_eq!(out, html);
        assert_eq!(links.len(), MAX_LINKS);
    }

    #[test]
    fn uri_components_are_encoded_like_javascript() {
        assert_eq!(
            encode_uri_component("https://x.example/a b?é=1&c=(d)*"),
            "https%3A%2F%2Fx.example%2Fa%20b%3F%C3%A9%3D1%26c%3D(d)*"
        );
    }

    fn tracked_draft(open: Value, links: Value) -> Value {
        json!({
            "hMsgId": "m1@example.test",
            "plaintext": false,
            "to": [{"email": "ann@example.net"}, {"email": "ANN@example.net"}],
            "cc": [{"email": "bob@example.net"}],
            "bcc": [],
            "metadata": [
                {"pluginId": OPEN_TRACKING, "value": open, "v": 2},
                {"pluginId": LINK_TRACKING, "value": links, "v": 1},
            ],
        })
    }

    #[test]
    fn a_send_gets_a_body_per_recipient_and_the_values_name_them_by_rid() {
        let draft = tracked_draft(
            json!({"open_count": 0, "open_data": []}),
            json!({"tracked": true}),
        );
        let body = r#"<p><a href="https://a.example">a</a></p>"#;
        let sent = track_send(&draft, body, &BTreeMap::new(), &target())
            .unwrap()
            .unwrap();
        assert_eq!(sent.bodies["self"], body);
        let keys: Vec<&String> = sent.bodies.keys().collect();
        assert_eq!(keys, ["ann@example.net", "bob@example.net", "self"]);
        let (plugin, open) = &sent.values[0];
        assert_eq!(*plugin, OPEN_TRACKING);
        let tid = open["trackingId"].as_str().unwrap();
        assert_eq!(tid.len(), 22);
        assert_eq!(open["uid"], "m1@example.test");
        let recipients = open["recipients"].as_object().unwrap();
        assert_eq!(recipients.len(), 2);
        for (rid, email) in recipients {
            let html = sent.bodies[email.as_str().unwrap()].as_str().unwrap();
            assert!(html.contains(&format!("/t/o/{}/{tid}.{rid}.png", "N".repeat(22))));
            assert!(html.contains(&format!("/{tid}/0.{rid}?u=https%3A%2F%2Fa.example")));
        }
        let (plugin, link) = &sent.values[1];
        assert_eq!(*plugin, LINK_TRACKING);
        assert_eq!(link["trackingId"], open["trackingId"]);
        assert_eq!(link["tracked"], true);
        assert_eq!(
            link["links"],
            json!([{"url": "https://a.example", "click_count": 0, "click_data": []}])
        );
    }

    #[test]
    fn hook_bodies_are_tracked_in_place_of_the_stored_one() {
        let draft = tracked_draft(json!({"open_count": 0}), Value::Null);
        let per = BTreeMap::from([
            ("ann@example.net".to_owned(), "<p>for ann</p>".to_owned()),
            ("bob@example.net".to_owned(), "<p>for bob</p>".to_owned()),
        ]);
        let sent = track_send(&draft, "<p>self</p>", &per, &target())
            .unwrap()
            .unwrap();
        assert!(sent.bodies["ann@example.net"]
            .as_str()
            .unwrap()
            .starts_with("<p>for ann</p><img"));
        assert_eq!(sent.bodies["self"], "<p>self</p>");
        assert_eq!(sent.values.len(), 1);
        assert_eq!(sent.values[0].1["open_data"], json!([]));
    }

    #[test]
    fn nothing_to_track() {
        let off = tracked_draft(Value::Null, Value::Null);
        assert!(track_send(&off, "x", &BTreeMap::new(), &target()).is_none());
        let mut plain = tracked_draft(json!({"open_count": 0}), Value::Null);
        plain["plaintext"] = json!(true);
        assert!(track_send(&plain, "x", &BTreeMap::new(), &target()).is_none());
        let mut nobody = tracked_draft(json!({"open_count": 0}), Value::Null);
        nobody["to"] = json!([]);
        nobody["cc"] = json!([]);
        assert!(track_send(&nobody, "x", &BTreeMap::new(), &target()).is_none());
    }

    #[test]
    fn set_plugin_value_replaces_or_adds() {
        let mut d = tracked_draft(json!({"open_count": 0}), Value::Null);
        set_plugin_value(&mut d, OPEN_TRACKING, json!({"x": 1}));
        assert_eq!(d["metadata"][0]["value"], json!({"x": 1}));
        assert_eq!(d["metadata"][0]["v"], 2);
        set_plugin_value(&mut d, "other", json!(true));
        assert_eq!(d["metadata"][2]["pluginId"], "other");
    }
}
