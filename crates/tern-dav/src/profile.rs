// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! DAV server quirks as named flags, the DAV counterpart of `tern_providers::ServerProfile`.
//! Taken from the comments in Mailspring-Sync `MailSync/DAVWorker.cpp` (lines 196-214,
//! 446, 650, 835, 1020-1023, 1360, 1782-1801, 1968, 2112, 2335; Copyright © 2017 Foundry
//! 376, GPL-3.0, see NOTICE.md), most of which cite python-caldav's server compatibility
//! notes.
//!
//! [`DavClient`](crate::DavClient) acts on `http1_only`, `no_sync_collection` and
//! `carddav_webmail_root`. The other quirks are handled for every server by the generic
//! code paths (the etag fallback, the client-side range check, bounded time ranges, the
//! comp-filter, href normalisation, case-insensitive group kinds), so their flags document
//! where each behaviour comes from and let the workers log it.

use crate::href::{host_of, path_of};

/// Gmail's CalDAV host; the calendar home is `/caldav/v2/<email>` (`DAVWorker::DAVWorker`).
pub const GMAIL_CALDAV_HOST: &str = "apidata.googleusercontent.com";

/// Replaces [`GMAIL_CALDAV_HOST`] in the calendar home, e.g. `http://127.0.0.1:8080` for the
/// harness's fake (apps/engine/test/README.md).
pub const ENV_GOOGLE_CALDAV_ROOT: &str = "MAILSYNC_GOOGLE_CALDAV_ROOT";

/// [`GMAIL_CALDAV_HOST`], or the non-empty `MAILSYNC_GOOGLE_CALDAV_ROOT` without a trailing `/`.
pub fn gmail_caldav_host() -> String {
    std::env::var(ENV_GOOGLE_CALDAV_ROOT)
        .ok()
        .map(|v| v.trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| GMAIL_CALDAV_HOST.to_string())
}

/// Where Yahoo's CardDAV `.well-known` redirects end when the account has no CardDAV.
pub const YAHOO_WEBMAIL_ROOT: &str = "https://mail.yahoo.com/";

/// Server families with a known quirk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DavServer {
    #[default]
    Generic,
    Google,
    ICloud,
    Yahoo,
    Fastmail,
    Nextcloud,
    Baikal,
    Radicale,
    Sogo,
    Zimbra,
    Posteo,
    Synology,
    Davical,
    Bedework,
    Gmx,
    Xandikos,
    Robur,
    Cyrus,
    Confluence,
}

/// What the server said about itself, from the `Server` and `DAV` response headers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerHints {
    pub server: Option<String>,
    pub dav: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DavProfile {
    pub server: DavServer,
    /// Calendars live at a fixed URL rather than behind discovery (Google, `DAVWorker.cpp:446`).
    pub fixed_caldav_host: Option<&'static str>,
    /// Contacts sync through the People API, not CardDAV (Google).
    pub contacts_via_people_api: bool,
    /// The CardDAV `.well-known` chain can end on this webmail page, which means no CardDAV
    /// (Yahoo: `https://mail.yahoo.com/`, `DAVWorker.cpp:650`).
    pub carddav_webmail_root: Option<&'static str>,
    /// The server may give a written card a new UID and href, so the stored contact is
    /// replaced rather than updated (Fastmail, `DAVWorker.cpp:835`).
    pub reassigns_contact_uid: bool,
    /// Groups are cards with `X-ADDRESSBOOKSERVER-KIND:group` and member properties
    /// (iCloud, `DAVWorker.cpp:1360`).
    pub vcard3_contact_groups: bool,
    /// No RFC 6578 sync-collection at all; go straight to the etag listing (GMX, Robur,
    /// `DAVWorker.cpp:1020`).
    pub no_sync_collection: bool,
    /// sync-collection is refused with an HTTP error rather than declined (Zimbra, Posteo,
    /// `DAVWorker.cpp:1021`); the etag fallback covers it.
    pub sync_collection_errors: bool,
    /// Sync tokens are unpredictable (Synology, DAViCal, Bedework; Nextcloud before
    /// nextcloud/server#44130, `DAVWorker.cpp:1022, 1968`).
    pub unreliable_sync_token: bool,
    /// Tokens have one-second precision and can miss changes within a second (SOGo,
    /// `DAVWorker.cpp:1023`); harmless at the engine's polling interval.
    pub second_precision_sync_token: bool,
    /// sync-collection never returns a `<sync-token>` (Yahoo, `DAVWorker.cpp:2112`); an
    /// empty answer without a token falls back to the etag listing.
    pub sync_collection_omits_token: bool,
    /// Time-range queries return events outside the range (Bedework, SOGo,
    /// `DAVWorker.cpp:1793-1797`); new events from sync-collection are range-checked.
    pub time_range_returns_extra: bool,
    /// Open-ended time ranges fail (Radicale, GMX, `DAVWorker.cpp:1791`); both bounds are
    /// always sent.
    pub requires_bounded_time_range: bool,
    /// calendar-query fails or returns nothing without a VEVENT comp-filter (SOGo,
    /// Xandikos, Nextcloud, Cyrus, Posteo, Robur, Zimbra, DAViCal, Synology, GMX,
    /// `DAVWorker.cpp:1782-1784`); the filter is always sent.
    pub requires_comp_filter: bool,
    /// DURATION is ignored, so such events look zero-length to time ranges (Xandikos,
    /// `DAVWorker.cpp:1798`).
    pub ignores_duration: bool,
    /// Hrefs come back percent-encoded twice (`@` as `%2540`; Confluence,
    /// `DAVWorker.cpp:196-214`); `normalize_href` decodes until stable for every server.
    pub double_encoded_hrefs: bool,
    /// A stored vCard 4 group is served to vCard 3 clients as
    /// `X-ADDRESSBOOKSERVER-KIND:GROUP` (sabre/vobject: Baikal, Nextcloud); `VCard::is_group`
    /// ignores the value's case for every server.
    pub uppercase_group_kind: bool,
    /// Authentication fails under HTTP/2 multiplexing with 401s despite correct
    /// credentials (Baikal's nginx image, `DAVWorker.cpp:2335`); requests use HTTP/1.1.
    pub http1_only: bool,
}

impl DavProfile {
    /// The profile for a DAV URL, refined by what the server said about itself.
    pub fn detect(url: &str, hints: &ServerHints) -> DavProfile {
        DavProfile::for_server(classify(url, hints))
    }

    pub fn for_server(server: DavServer) -> DavProfile {
        use DavServer::*;
        let mut p = DavProfile {
            server,
            requires_comp_filter: true,
            ..Default::default()
        };
        match server {
            Google => {
                p.fixed_caldav_host = Some(GMAIL_CALDAV_HOST);
                p.contacts_via_people_api = true;
            }
            ICloud => p.vcard3_contact_groups = true,
            Yahoo => {
                p.carddav_webmail_root = Some(YAHOO_WEBMAIL_ROOT);
                p.sync_collection_omits_token = true;
            }
            Fastmail => {
                p.reassigns_contact_uid = true;
                p.vcard3_contact_groups = true;
            }
            Nextcloud => {
                p.unreliable_sync_token = true;
                p.uppercase_group_kind = true;
            }
            Baikal => {
                p.http1_only = true;
                p.uppercase_group_kind = true;
            }
            Radicale => p.requires_bounded_time_range = true,
            Sogo => {
                p.second_precision_sync_token = true;
                p.time_range_returns_extra = true;
            }
            Zimbra | Posteo => p.sync_collection_errors = true,
            Synology | Davical => p.unreliable_sync_token = true,
            Bedework => {
                p.unreliable_sync_token = true;
                p.time_range_returns_extra = true;
            }
            Gmx => {
                p.no_sync_collection = true;
                p.requires_bounded_time_range = true;
            }
            Xandikos => p.ignores_duration = true,
            Robur => p.no_sync_collection = true,
            Confluence => p.double_encoded_hrefs = true,
            // Cyrus fails calendar-query without a VEVENT comp-filter (`DAVWorker.cpp:1783`),
            // which every server gets.
            Cyrus | Generic => {}
        }
        p
    }
}

fn host_is(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn classify(url: &str, hints: &ServerHints) -> DavServer {
    use DavServer::*;
    let host = host_of(url);
    let path = path_of(url).to_ascii_lowercase();
    let server = hints.server.as_deref().unwrap_or("").to_ascii_lowercase();
    let dav = hints.dav.as_deref().unwrap_or("").to_ascii_lowercase();

    let by_host: &[(&str, DavServer)] = &[
        ("googleusercontent.com", Google),
        ("google.com", Google),
        ("icloud.com", ICloud),
        ("me.com", ICloud),
        ("yahoo.com", Yahoo),
        ("fastmail.com", Fastmail),
        ("messagingengine.com", Fastmail),
        ("posteo.de", Posteo),
        ("gmx.net", Gmx),
        ("gmx.com", Gmx),
        ("gmx.de", Gmx),
    ];
    if let Some((_, s)) = by_host.iter().find(|(d, _)| host_is(&host, d)) {
        return *s;
    }
    let by_header: &[(&str, DavServer)] = &[
        ("radicale", Radicale),
        ("xandikos", Xandikos),
        ("davical", Davical),
        ("sogo", Sogo),
        ("zimbra", Zimbra),
        ("bedework", Bedework),
        ("synology", Synology),
        ("robur", Robur),
        ("baikal", Baikal),
        ("nextcloud", Nextcloud),
        ("cyrus", Cyrus),
        ("confluence", Confluence),
    ];
    if let Some((_, s)) = by_header
        .iter()
        .find(|(n, _)| server.contains(n) || host.contains(n))
    {
        return *s;
    }
    if dav.contains("nextcloud") || dav.contains("nc-calendar") {
        return Nextcloud;
    }
    let by_path: &[(&str, DavServer)] = &[
        ("/remote.php/dav", Nextcloud),
        ("/dav.php", Baikal),
        ("/cal.php", Baikal),
        ("/card.php", Baikal),
        ("/sogo/dav", Sogo),
        ("/caldav.php", Davical),
        ("/ucaldav", Bedework),
    ];
    by_path
        .iter()
        .find(|(p, _)| path.contains(p))
        .map_or(Generic, |(_, s)| *s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(url: &str) -> DavProfile {
        DavProfile::detect(url, &ServerHints::default())
    }

    #[test]
    fn detection() {
        assert_eq!(
            detect("https://p52-caldav.icloud.com/123/calendars/").server,
            DavServer::ICloud
        );
        assert_eq!(
            detect("apidata.googleusercontent.com/caldav/v2/a@b.com").server,
            DavServer::Google
        );
        assert_eq!(
            detect("https://cloud.example.org/remote.php/dav/calendars/u/").server,
            DavServer::Nextcloud
        );
        assert_eq!(
            detect("http://localhost:8800/dav.php/calendars/u/").server,
            DavServer::Baikal
        );
        assert_eq!(
            detect("https://caldav.gmx.net/begenda/dav/u/calendar").server,
            DavServer::Gmx
        );
        assert_eq!(
            detect("https://dav.example.com/").server,
            DavServer::Generic
        );
        let radicale = DavProfile::detect(
            "http://localhost:5232/user/",
            &ServerHints {
                server: Some("Radicale/3.2".into()),
                dav: None,
            },
        );
        assert_eq!(radicale.server, DavServer::Radicale);
        assert!(radicale.requires_bounded_time_range);
    }

    #[test]
    fn flags() {
        assert!(DavProfile::for_server(DavServer::Baikal).http1_only);
        assert!(DavProfile::for_server(DavServer::Gmx).no_sync_collection);
        assert_eq!(
            DavProfile::for_server(DavServer::Yahoo).carddav_webmail_root,
            Some("https://mail.yahoo.com/")
        );
        assert!(DavProfile::for_server(DavServer::Baikal).uppercase_group_kind);
        assert!(DavProfile::for_server(DavServer::Generic).requires_comp_filter);
        assert!(!DavProfile::for_server(DavServer::Generic).no_sync_collection);
        assert_eq!(
            DavProfile::for_server(DavServer::Google).fixed_caldav_host,
            Some(GMAIL_CALDAV_HOST)
        );
    }
}
