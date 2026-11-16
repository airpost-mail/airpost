// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! What a client adds to a grammar search (`search {grammar: true}`, `query.search`): the
//! engine reads date terms in the zone and at the time the request names, never its own, so
//! a client sends where and when the person searching is.

use std::time::{SystemTime, UNIX_EPOCH};

use airpost_ipc::SearchParams;

/// The local IANA zone: `TZ` when it names one (a zoneinfo path is read from its
/// `zoneinfo/` part), else the system's. A POSIX `TZ` rule such as `UTC0` or
/// `EST5EDT,M3.2.0,M11.1.0` isn't a name the engine reads, so the system's zone is used.
/// `None` when neither is known; the engine then reads dates in UTC.
pub fn local_time_zone() -> Option<String> {
    std::env::var("TZ")
        .ok()
        .and_then(|tz| zone_from_tz(&tz))
        .or_else(|| iana_time_zone::get_timezone().ok())
}

/// The IANA name in a `TZ` value, if it looks like one: `Area/City` (`Etc/GMT+5` too) or
/// a name of letters only (`UTC`, `GMT`).
fn zone_from_tz(tz: &str) -> Option<String> {
    let tz = tz.trim_start_matches(':');
    let name = tz.rsplit_once("zoneinfo/").map_or(tz, |(_, n)| n);
    let iana_chars = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'));
    let looks_iana = if name.contains('/') {
        !name.starts_with('/')
            && !name.ends_with('/')
            && name.starts_with(|c: char| c.is_ascii_alphabetic())
    } else {
        name.chars().all(|c| c.is_ascii_alphabetic())
    };
    (!name.is_empty() && iana_chars && looks_iana).then(|| name.to_owned())
}

/// Now, in unix seconds.
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `search` params for `query` in the grammar, with the local zone and clock.
pub fn grammar_search(query: &str, limit: Option<usize>) -> SearchParams {
    SearchParams {
        query: query.to_owned(),
        limit,
        grammar: true,
        tz: local_time_zone(),
        now: Some(now_secs()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_search_carries_zone_and_clock() {
        let p = grammar_search("from:ann", Some(5));
        assert!(p.grammar && p.now.is_some_and(|n| n > 1_700_000_000));
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["query"], "from:ann");
        assert!(v.get("accountId").is_none());
    }

    #[test]
    fn tz_names_and_rules() {
        for (tz, name) in [
            ("Europe/Lisbon", Some("Europe/Lisbon")),
            (":America/New_York", Some("America/New_York")),
            ("/usr/share/zoneinfo/Asia/Tokyo", Some("Asia/Tokyo")),
            ("Etc/GMT+5", Some("Etc/GMT+5")),
            ("UTC", Some("UTC")),
            ("", None),
            ("/etc/localtime", None),
            ("UTC0", None),
            ("EST5EDT,M3.2.0,M11.1.0", None),
            ("<+03>-3", None),
        ] {
            assert_eq!(zone_from_tz(tz).as_deref(), name, "{tz:?}");
        }
    }
}
