// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! PROPFIND and REPORT bodies, byte for byte as `MailSync/DAVWorker.cpp` sends them
//! (Copyright © 2017 Foundry 376, GPL-3.0; see NOTICE.md), plus the calendar sync window.
//!
//! Every value interpolated into a body (hrefs, sync tokens) goes through
//! [`xml_escape`]: both come from the server.

use crate::xml::xml_escape;
use chrono::DateTime;

/// Hrefs per `addressbook-multiget` / `calendar-multiget` request.
pub const MULTIGET_CHUNK: usize = 90;

/// Deleted contacts or events per removal transaction (`ingestContactDeletions`).
pub const DELETION_CHUNK: usize = 100;

/// Pages of a sync-collection REPORT before giving up (`maxPages`).
pub const MAX_SYNC_PAGES: usize = 100;

/// `Depth` for single-resource PROPFINDs and sync-collection (RFC 6578 §3.2 requires 0).
pub const DEPTH_0: &str = "0";
/// `Depth` for collection listings and the other REPORTs.
pub const DEPTH_1: &str = "1";

/// current-user-principal (RFC 5397), sent to the service root for both CalDAV and CardDAV.
pub const PROPFIND_PRINCIPAL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><A:propfind xmlns:A=\"DAV:\"><A:prop><A:current-user-principal/><A:principal-URL/><A:resourcetype/></A:prop></A:propfind>";

/// addressbook-home-set (RFC 6352 §7.1.1), sent to the principal.
pub const PROPFIND_ADDRESSBOOK_HOME_SET: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><A:propfind xmlns:A=\"DAV:\"><A:prop><A:displayname/><A:resourcetype/><B:addressbook-home-set xmlns:B=\"urn:ietf:params:xml:ns:carddav\"/></A:prop></A:propfind>";

/// The address books in a home set, with their ctags.
pub const PROPFIND_ADDRESSBOOKS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><d:propfind xmlns:d=\"DAV:\" xmlns:cs=\"http://calendarserver.org/ns/\"><d:prop><d:resourcetype /><d:displayname /><cs:getctag /></d:prop></d:propfind>";

/// The ctag alone, at Depth 0 (`validateCachedAddressBook`).
pub const PROPFIND_CTAG: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><d:propfind xmlns:d=\"DAV:\" xmlns:cs=\"http://calendarserver.org/ns/\"><d:prop><cs:getctag/></d:prop></d:propfind>";

/// The ETag of one resource, to tell a stale `If-Match` from a deleted resource.
pub const PROPFIND_GETETAG: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:getetag/></d:prop></d:propfind>";

/// calendar-home-set (RFC 4791 §6.2.1), sent to the principal at Depth 0: at Depth 1
/// Nextcloud also answers for the principal's calendar-proxy-read/-write children.
pub const PROPFIND_CALENDAR_HOME_SET: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><d:propfind xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><c:calendar-home-set/></d:prop></d:propfind>";

/// The calendars in a home set with their metadata (`runCalendars`).
pub const PROPFIND_CALENDARS: &str = "<d:propfind xmlns:d=\"DAV:\" xmlns:cs=\"http://calendarserver.org/ns/\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:ical=\"http://apple.com/ns/ical/\"><d:prop><d:resourcetype /><d:displayname /><cs:getctag /><c:supported-calendar-component-set /><ical:calendar-color /><c:calendar-description /><d:current-user-privilege-set /><d:owner /><ical:calendar-order /></d:prop></d:propfind>";

/// Every contact's etag (`runForAddressBook`).
pub const ADDRESSBOOK_QUERY_ETAGS: &str = "<c:addressbook-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:carddav\"><d:prop><d:getetag /></d:prop></c:addressbook-query>";

/// Which collection a REPORT addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    AddressBook,
    Calendar,
}

fn hrefs_xml(hrefs: &[String]) -> String {
    hrefs
        .iter()
        .map(|h| format!("<d:href>{}</d:href>", xml_escape(h)))
        .collect()
}

/// `addressbook-multiget` for `hrefs` (RFC 6352 §8.7).
pub fn addressbook_multiget(hrefs: &[String]) -> String {
    format!(
        "<c:addressbook-multiget xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:carddav\"><d:prop><d:getetag /><c:address-data /></d:prop>{}</c:addressbook-multiget>",
        hrefs_xml(hrefs)
    )
}

/// `calendar-multiget` for `hrefs` (RFC 4791 §7.9).
pub fn calendar_multiget(hrefs: &[String]) -> String {
    format!(
        "<c:calendar-multiget xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:getetag /><c:calendar-data /></d:prop>{}</c:calendar-multiget>",
        hrefs_xml(hrefs)
    )
}

/// A multiget for the collection kind.
pub fn multiget(kind: Collection, hrefs: &[String]) -> String {
    match kind {
        Collection::AddressBook => addressbook_multiget(hrefs),
        Collection::Calendar => calendar_multiget(hrefs),
    }
}

/// RFC 6578 sync-collection. The initial sync (no token) asks for etags only and fetches
/// the data by multiget in chunks; an incremental one asks for the data too, since the
/// change set is expected to be small.
pub fn sync_collection(kind: Collection, token: &str, initial: bool) -> String {
    let token_el = if token.is_empty() {
        "<D:sync-token/>".to_string()
    } else {
        format!("<D:sync-token>{}</D:sync-token>", xml_escape(token))
    };
    let (ns, data) = match kind {
        Collection::AddressBook => ("urn:ietf:params:xml:ns:carddav", "<C:address-data/>"),
        Collection::Calendar => ("urn:ietf:params:xml:ns:caldav", "<C:calendar-data/>"),
    };
    let props = if initial {
        "<D:getetag/>".to_string()
    } else {
        format!("<D:getetag/>{data}")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:sync-collection xmlns:D=\"DAV:\" xmlns:C=\"{ns}\">{token_el}<D:sync-level>1</D:sync-level><D:prop>{props}</D:prop></D:sync-collection>"
    )
}

/// Every event etag in `range` (`runForCalendar`).
///
/// The VEVENT comp-filter is always sent. RFC 4791 doesn't require it, but servers fail
/// without it: SOGo and Xandikos 0.3+ return nothing; Nextcloud, Cyrus, Posteo and Robur
/// return errors; Zimbra, DAViCal, Synology and GMX are inconsistent. Both bounds are always
/// sent because Radicale and GMX fail open-ended time ranges (python-caldav research).
pub fn calendar_query_etags(range: &CalendarSyncRange) -> String {
    format!(
        "<c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:getetag /></d:prop><c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\"><c:time-range start=\"{}\" end=\"{}\"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>",
        range.start_str, range.end_str
    )
}

/// Months of events kept before now.
pub const CALENDAR_SYNC_PAST_MONTHS: i64 = 12;
/// Months of events kept after now.
pub const CALENDAR_SYNC_FUTURE_MONTHS: i64 = 18;

/// The window of events the engine keeps (`getCalendarSyncRange`); a month is 30 days.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarSyncRange {
    pub start: i64,
    pub end: i64,
    /// `YYYYMMDDTHHMMSSZ` for the time-range filter (RFC 4791 §9.9).
    pub start_str: String,
    pub end_str: String,
}

impl CalendarSyncRange {
    pub fn around(now: i64) -> Self {
        const MONTH: i64 = 30 * 24 * 60 * 60;
        let start = now - CALENDAR_SYNC_PAST_MONTHS * MONTH;
        let end = now + CALENDAR_SYNC_FUTURE_MONTHS * MONTH;
        CalendarSyncRange {
            start,
            end,
            start_str: format_utc(start),
            end_str: format_utc(end),
        }
    }

    /// `eventOverlapsRange`. Bedework and SOGo return events outside the requested range,
    /// so events new to the sync-token path are checked client side. Both ends inclusive.
    pub fn overlaps(&self, event_start: i64, event_end: i64) -> bool {
        event_end >= self.start && event_start <= self.end
    }
}

/// `MailUtils::formatDateTimeUTC`.
pub fn format_utc(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_collection_bodies() {
        assert_eq!(
            sync_collection(Collection::Calendar, "", true),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:sync-collection xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\"><D:sync-token/><D:sync-level>1</D:sync-level><D:prop><D:getetag/></D:prop></D:sync-collection>"
        );
        assert_eq!(
            sync_collection(Collection::AddressBook, "http://x/ns/sync/5&6", false),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:sync-collection xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:carddav\"><D:sync-token>http://x/ns/sync/5&amp;6</D:sync-token><D:sync-level>1</D:sync-level><D:prop><D:getetag/><C:address-data/></D:prop></D:sync-collection>"
        );
    }

    #[test]
    fn multiget_escapes_hrefs() {
        let body = calendar_multiget(&["/c/a.ics".into(), "/c/</d:href><x>".into()]);
        assert_eq!(
            body,
            "<c:calendar-multiget xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:getetag /><c:calendar-data /></d:prop><d:href>/c/a.ics</d:href><d:href>/c/&lt;/d:href&gt;&lt;x&gt;</d:href></c:calendar-multiget>"
        );
    }

    #[test]
    fn sync_range() {
        let r = CalendarSyncRange::around(1_700_000_000);
        assert_eq!(r.start, 1_700_000_000 - 360 * 86400);
        assert_eq!(r.end, 1_700_000_000 + 540 * 86400);
        assert_eq!(r.start_str, "20221119T221320Z");
        assert_eq!(r.end_str, "20250507T221320Z");
        assert!(r.overlaps(r.end, r.end + 10));
        assert!(r.overlaps(r.start - 10, r.start));
        assert!(!r.overlaps(r.end + 1, r.end + 10));
        assert!(calendar_query_etags(&r)
            .contains("<c:time-range start=\"20221119T221320Z\" end=\"20250507T221320Z\"/>"));
    }
}
