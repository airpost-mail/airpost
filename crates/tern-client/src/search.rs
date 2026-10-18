// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! What a client adds to a grammar search (`search {grammar: true}`, `query.search`): the
//! engine reads date terms in the zone and at the time the request names, never its own, so
//! a client sends where and when the person searching is.

use std::time::{SystemTime, UNIX_EPOCH};

use ms_ipc::SearchParams;

/// The local IANA zone: `TZ` when set (a zoneinfo path is read from its `zoneinfo/` part),
/// else the system's. `None` when neither is known; the engine then reads dates in UTC.
pub fn local_time_zone() -> Option<String> {
    if let Some(tz) = std::env::var("TZ").ok().filter(|t| !t.is_empty()) {
        let tz = tz.trim_start_matches(':');
        let name = tz.rsplit_once("zoneinfo/").map_or(tz, |(_, n)| n);
        if !name.starts_with('/') && !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    iana_time_zone::get_timezone().ok()
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
}
