// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! WebDAV XML: an owned element tree, multistatus parsing (RFC 4918 §13) and text escaping.
//! Derived from Mailspring-Sync `MailSync/DavXML.*` and the XPath queries in
//! `MailSync/DAVWorker.cpp`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! The C++ engine ran XPath over libxml2. Each XPath it used is a method here, scoped the
//! same way: `./D:href` is [`DavResponse::href`], `./D:status` is [`DavResponse::status`],
//! `./D:propstat[contains(D:status,'200')]/D:prop/…` is [`DavResponse::ok_prop`]. Text is
//! the concatenation of an element's text and CDATA children, where libxml2's `text()`
//! kept only the last text node, which lost data when a server split a body over a CDATA
//! section and whitespace.

use airpost_core::{Result, SyncError};

/// XML namespaces DAV responses use.
pub mod ns {
    pub const DAV: &str = "DAV:";
    pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
    pub const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
    /// `cs:getctag` (Apple calendarserver extension).
    pub const CALENDARSERVER: &str = "http://calendarserver.org/ns/";
    /// `ical:calendar-color`, `ical:calendar-order`.
    pub const APPLE_ICAL: &str = "http://apple.com/ns/ical/";
}

/// The error key the C++ engine used for an unparseable body; the body is the debuginfo.
pub const PARSE_ERROR_KEY: &str = "Unable to parse CalDav XML";

/// An XML element with its namespace resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub ns: String,
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// The element's own text and CDATA children, concatenated in document order.
    pub text: String,
}

impl Element {
    pub fn is(&self, ns: &str, name: &str) -> bool {
        self.ns == ns && self.name == name
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The first direct child named `ns:name`.
    pub fn child(&self, ns: &str, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.is(ns, name))
    }

    pub fn children_named<'a>(
        &'a self,
        ns: &'a str,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.is(ns, name))
    }

    /// Every descendant, depth first in document order, excluding `self`.
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        let mut stack: Vec<&Element> = self.children.iter().rev().collect();
        while let Some(e) = stack.pop() {
            out.push(e);
            stack.extend(e.children.iter().rev());
        }
        out
    }

    /// Descendants named `ns:name` (XPath `.//ns:name`).
    pub fn find_all(&self, ns: &str, name: &str) -> Vec<&Element> {
        self.descendants()
            .into_iter()
            .filter(|e| e.is(ns, name))
            .collect()
    }

    pub fn find(&self, ns: &str, name: &str) -> Option<&Element> {
        self.descendants().into_iter().find(|e| e.is(ns, name))
    }

    /// Follows direct children: `path(&[(DAV, "prop"), (DAV, "owner")])` is `./D:prop/D:owner`.
    pub fn path(&self, steps: &[(&str, &str)]) -> Option<&Element> {
        steps
            .iter()
            .try_fold(self, |e, (ns, name)| e.child(ns, name))
    }

    /// The text of the last `.//ns:name` with non-empty text, as `nodeContentAtXPath(".//x/text()")`
    /// returned.
    pub fn last_text(&self, ns: &str, name: &str) -> Option<&str> {
        self.find_all(ns, name)
            .into_iter()
            .rev()
            .map(|e| e.text.as_str())
            .find(|t| !t.is_empty())
    }
}

/// Parses `body` into its root element. Leading whitespace and a BOM are skipped, and the
/// Content-Type is never consulted: iCloud and Open-Xchange answer valid XML labelled
/// `text/plain` (`DavXML.cpp`).
pub fn parse_document(body: &str) -> Result<Element> {
    let trimmed = body.trim_start_matches('\u{feff}').trim_start();
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(trimmed, opts)
        .map_err(|_| SyncError::new(PARSE_ERROR_KEY, body, false, false))?;
    Ok(convert(doc.root_element()))
}

fn convert(node: roxmltree::Node<'_, '_>) -> Element {
    let tag = node.tag_name();
    let mut el = Element {
        ns: tag.namespace().unwrap_or("").to_string(),
        name: tag.name().to_string(),
        attrs: node
            .attributes()
            .map(|a| (a.name().to_string(), a.value().to_string()))
            .collect(),
        ..Default::default()
    };
    for child in node.children() {
        if child.is_element() {
            el.children.push(convert(child));
        } else if child.is_text() {
            el.text.push_str(child.text().unwrap_or(""));
        }
    }
    el
}

/// The status code in an HTTP status line such as `HTTP/1.1 404 Not Found`.
pub fn parse_status_line(line: &str) -> Option<u16> {
    line.split_whitespace()
        .filter(|t| !t.to_ascii_uppercase().starts_with("HTTP/"))
        .find(|t| t.len() == 3 && t.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|t| t.parse().ok())
}

/// One `<propstat>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Propstat {
    pub status: Option<u16>,
    /// `<prop>`; an empty element when the propstat had none.
    pub prop: Element,
}

impl Propstat {
    /// A propstat whose status is 2xx. A missing status counts as success: the properties
    /// are there, and the C++ `.//` lookups read them regardless.
    pub fn is_ok(&self) -> bool {
        self.status.is_none_or(|s| (200..300).contains(&s))
    }
}

/// One `<response>` of a multistatus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DavResponse {
    /// The first direct `<href>` child, trimmed. Nested hrefs (an owner, a principal) never
    /// leak in here, unlike the C++ `.//D:href`, which kept the last of them.
    pub href: String,
    /// Further direct `<href>` children (RFC 4918 allows several with one status).
    pub extra_hrefs: Vec<String>,
    /// The response-level `<status>` (`./D:status`), which RFC 6578 §3.5 uses for deletions.
    pub status: Option<u16>,
    pub propstats: Vec<Propstat>,
    /// The `<response>` element itself, for queries the accessors don't cover.
    pub element: Element,
}

impl DavResponse {
    fn from_element(el: Element) -> Self {
        let mut hrefs = el
            .children_named(ns::DAV, "href")
            .map(|h| h.text.trim().to_string());
        let href = hrefs.next().unwrap_or_default();
        let extra_hrefs = hrefs.collect();
        let status = el
            .child(ns::DAV, "status")
            .and_then(|s| parse_status_line(&s.text));
        let propstats = el
            .children_named(ns::DAV, "propstat")
            .map(|ps| Propstat {
                status: ps
                    .child(ns::DAV, "status")
                    .and_then(|s| parse_status_line(&s.text)),
                prop: ps.child(ns::DAV, "prop").cloned().unwrap_or_default(),
            })
            .collect();
        DavResponse {
            href,
            extra_hrefs,
            status,
            propstats,
            element: el,
        }
    }

    /// `ns:name` as a direct child of `<prop>` in a 2xx propstat.
    pub fn ok_prop(&self, ns: &str, name: &str) -> Option<&Element> {
        self.propstats
            .iter()
            .filter(|p| p.is_ok())
            .find_map(|p| p.prop.child(ns, name))
    }

    /// `ns:name` in `<prop>` of any propstat, whatever its status.
    pub fn any_prop(&self, ns: &str, name: &str) -> Option<&Element> {
        self.propstats.iter().find_map(|p| p.prop.child(ns, name))
    }

    /// The raw text of a property: from a 2xx propstat when one carries it, else the last
    /// non-empty `.//ns:name` in the response (the C++ lookup). Empty text counts as absent.
    pub fn prop_raw(&self, ns: &str, name: &str) -> Option<&str> {
        self.ok_prop(ns, name)
            .map(|e| e.text.as_str())
            .filter(|t| !t.is_empty())
            .or_else(|| self.element.last_text(ns, name))
    }

    /// [`DavResponse::prop_raw`], trimmed, for scalar properties.
    pub fn prop_text(&self, ns: &str, name: &str) -> Option<&str> {
        self.prop_raw(ns, name)
            .map(str::trim)
            .filter(|t| !t.is_empty())
    }

    pub fn etag(&self) -> Option<&str> {
        self.prop_text(ns::DAV, "getetag")
    }

    pub fn ctag(&self) -> Option<&str> {
        self.prop_text(ns::CALENDARSERVER, "getctag")
    }

    pub fn display_name(&self) -> Option<&str> {
        self.prop_text(ns::DAV, "displayname")
    }

    /// `caldav:calendar-data`, untrimmed.
    pub fn calendar_data(&self) -> Option<&str> {
        self.prop_raw(ns::CALDAV, "calendar-data")
    }

    /// `carddav:address-data`, untrimmed.
    pub fn address_data(&self) -> Option<&str> {
        self.prop_raw(ns::CARDDAV, "address-data")
    }

    /// The href inside a property such as `current-user-principal` or `owner`, from a 2xx
    /// propstat when present.
    pub fn prop_href(&self, ns: &str, name: &str) -> Option<&str> {
        self.ok_prop(ns, name)
            .or_else(|| self.any_prop(ns, name))
            .and_then(|p| p.child(ns::DAV, "href"))
            .map(|h| h.text.trim())
            .filter(|t| !t.is_empty())
    }

    /// Whether any propstat's `resourcetype` lists `ns:name`
    /// (`./D:propstat/D:prop/D:resourcetype/ns:name`).
    pub fn has_resource_type(&self, ns: &str, name: &str) -> bool {
        self.propstats.iter().any(|p| {
            p.prop
                .child(ns::DAV, "resourcetype")
                .is_some_and(|rt| rt.child(ns, name).is_some())
        })
    }

    /// A 2xx `resourcetype` that does not list `caldav:calendar`. Nextcloud keeps a deleted
    /// calendar in the home for 30 days as `<nc:deleted-calendar/>`.
    pub fn is_positively_not_calendar(&self) -> bool {
        self.propstats.iter().filter(|p| p.is_ok()).any(|p| {
            p.prop
                .child(ns::DAV, "resourcetype")
                .is_some_and(|rt| rt.child(ns::CALDAV, "calendar").is_none())
        })
    }

    pub fn is_calendar(&self) -> bool {
        self.has_resource_type(ns::CALDAV, "calendar")
    }

    /// `.//carddav:addressbook` anywhere in the response, as `resolveAddressBook` matched.
    pub fn is_address_book(&self) -> bool {
        self.element.find(ns::CARDDAV, "addressbook").is_some()
    }

    /// `supported-calendar-component-set` lists `<comp name="{component}"/>`.
    pub fn supports_component(&self, component: &str) -> bool {
        self.propstats.iter().any(|p| {
            p.prop
                .child(ns::CALDAV, "supported-calendar-component-set")
                .is_some_and(|set| {
                    set.children_named(ns::CALDAV, "comp")
                        .any(|c| c.attr("name") == Some(component))
                })
        })
    }

    /// The privileges in a 2xx `current-user-privilege-set`, or `None` when the server did
    /// not supply the property (RFC 3744 §5.4 makes it optional).
    pub fn privileges(&self) -> Option<Vec<(String, String)>> {
        let set = self.ok_prop(ns::DAV, "current-user-privilege-set")?;
        Some(
            set.descendants()
                .into_iter()
                .filter(|e| e.ns == ns::DAV && e.name != "privilege")
                .map(|e| (e.ns.clone(), e.name.clone()))
                .collect(),
        )
    }
}

/// A parsed `207 Multi-Status` body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Multistatus {
    pub responses: Vec<DavResponse>,
    /// The `<sync-token>` of a sync-collection REPORT (`//D:sync-token/text()`).
    pub sync_token: Option<String>,
    pub root: Element,
}

impl Multistatus {
    /// Any XML parses; a root other than `<multistatus>` yields no responses, as the C++
    /// XPath found none.
    pub fn parse(body: &str) -> Result<Multistatus> {
        Ok(Multistatus::from_root(parse_document(body)?))
    }

    pub fn from_root(root: Element) -> Multistatus {
        let responses = root
            .find_all(ns::DAV, "response")
            .into_iter()
            .cloned()
            .map(DavResponse::from_element)
            .collect();
        let sync_token = root
            .child(ns::DAV, "sync-token")
            .map(|t| t.text.trim())
            .filter(|t| !t.is_empty())
            .or_else(|| root.last_text(ns::DAV, "sync-token").map(str::trim))
            .map(String::from);
        Multistatus {
            responses,
            sync_token,
            root,
        }
    }

    /// The last non-empty `//ns:name/D:href` in the document, as `nodeContentAtXPath` read
    /// `current-user-principal` and the home sets.
    pub fn last_prop_href(&self, ns: &str, name: &str) -> Option<&str> {
        self.root
            .find_all(ns, name)
            .into_iter()
            .rev()
            .filter_map(|p| p.child(ns::DAV, "href"))
            .map(|h| h.text.trim())
            .find(|t| !t.is_empty())
    }
}

/// Escapes text bound for an XML text node, so an href taken from a server response can't
/// close its element and add siblings to a multiget body (`xmlEscape`).
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        assert_eq!(parse_status_line("HTTP/1.1 200 OK"), Some(200));
        assert_eq!(parse_status_line(" HTTP/1.1 404 Not Found\n"), Some(404));
        assert_eq!(
            parse_status_line("HTTP/1.1 507 Insufficient Storage"),
            Some(507)
        );
        assert_eq!(parse_status_line("garbage"), None);
    }

    #[test]
    fn unparseable_body_is_a_sync_error_with_the_body() {
        let err = Multistatus::parse("<html><body>oops").unwrap_err();
        let s = err.as_sync().unwrap();
        assert_eq!(s.key, PARSE_ERROR_KEY);
        assert_eq!(s.debuginfo, "<html><body>oops");
        assert!(!s.retryable);
    }

    #[test]
    fn text_concatenates_cdata_and_text() {
        let root =
            parse_document("<a xmlns='DAV:'><b>one<![CDATA[<two>]]>three</b>\u{20}</a>").unwrap();
        assert_eq!(root.child(ns::DAV, "b").unwrap().text, "one<two>three");
    }

    #[test]
    fn escape() {
        assert_eq!(xml_escape("a&b<c>\"'"), "a&amp;b&lt;c&gt;&quot;&apos;");
    }
}
