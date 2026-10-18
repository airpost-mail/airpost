// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Date terms of the search grammar (`before:`, `after:`, `since:`), resolved against the
//! zone and clock the request names (`search.tz`, `search.now`), never the engine's own.
//! The desktop read them with chrono-node's casual parser in its own zone
//! (`search-query-backend-local.ts`, `DateUtils.getChronoPast`); the engine supports a
//! documented subset of that, and anything else is an error rather than an empty match.
//!
//! Every term names a day, and resolves to that day's first instant in the zone:
//! - ISO dates: `2024-03-01`, `2024/3/1`, `2024.03.01`; a month: `2024-03`, `2024/03`.
//! - Month names (`march`, `mar`, `sept`) alone or with a day and/or a year, either order:
//!   `march`, `march 2024`, `mar 1`, `1 mar`, `march 1st, 2024`, `1 march 2024`. A month
//!   alone is its 1st.
//! - Weekday names (`monday`, `mon`, `tues`, …).
//! - `today`, `yesterday`.
//! - `N days|weeks|months|years ago` (also the singular) and the short `Nd`, `Nw`, `Nm`,
//!   `Ny`.
//! - `last week|month|year`: one week, month or year ago.
//!
//! Names without a year, weekdays and the relative terms need `now`, and look back from it
//! as the desktop's parser did: a weekday or date without a year is the latest one on or
//! before today. Terms with spaces are quoted in the query: `after:"3 days ago"`.

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::search_grammar::GrammarError;

/// The zone and clock a request's date terms are read in.
#[derive(Clone, Copy, Debug)]
pub struct DateContext {
    pub tz: Tz,
    /// Unix seconds. `None`: only terms with a full date resolve.
    pub now: Option<i64>,
}

impl DateContext {
    /// `tz` is an IANA name; `None` is UTC.
    pub fn new(tz: Option<&str>, now: Option<i64>) -> Result<DateContext, GrammarError> {
        let tz = match tz {
            None => Tz::UTC,
            Some(name) => name.parse::<Tz>().map_err(|_| {
                GrammarError(format!("search.tz {name:?} is not an IANA time zone"))
            })?,
        };
        Ok(DateContext { tz, now })
    }

    pub fn utc(now: Option<i64>) -> DateContext {
        DateContext { tz: Tz::UTC, now }
    }

    fn today(&self, term: &str) -> Result<NaiveDate, GrammarError> {
        let now = self.now.ok_or_else(|| {
            GrammarError(format!(
                "the date {term:?} is relative to now, and the request has no search.now"
            ))
        })?;
        let utc = DateTime::<Utc>::from_timestamp(now, 0)
            .ok_or_else(|| GrammarError("search.now is out of range".into()))?;
        Ok(utc.with_timezone(&self.tz).date_naive())
    }

    /// The first instant of `day` in the zone. A day starting in a DST gap starts at the
    /// first time that exists.
    fn start_of(&self, day: NaiveDate) -> i64 {
        let mut t = day.and_time(NaiveTime::MIN);
        for _ in 0..48 {
            if let Some(dt) = self.tz.from_local_datetime(&t).earliest() {
                return dt.timestamp();
            }
            t += chrono::Duration::minutes(30);
        }
        day.and_time(NaiveTime::MIN).and_utc().timestamp()
    }
}

const MONTHS: [&[&str]; 12] = [
    &["january", "jan"],
    &["february", "feb"],
    &["march", "mar"],
    &["april", "apr"],
    &["may"],
    &["june", "jun"],
    &["july", "jul"],
    &["august", "aug"],
    &["september", "sep", "sept"],
    &["october", "oct"],
    &["november", "nov"],
    &["december", "dec"],
];

const WEEKDAYS: [&[&str]; 7] = [
    &["monday", "mon"],
    &["tuesday", "tue", "tues"],
    &["wednesday", "wed"],
    &["thursday", "thu", "thur", "thurs"],
    &["friday", "fri"],
    &["saturday", "sat"],
    &["sunday", "sun"],
];

fn month_of(word: &str) -> Option<u32> {
    MONTHS
        .iter()
        .position(|names| names.contains(&word))
        .map(|i| i as u32 + 1)
}

fn weekday_of(word: &str) -> Option<u32> {
    WEEKDAYS
        .iter()
        .position(|names| names.contains(&word))
        .map(|i| i as u32)
}

/// `1`, `01`, `1st`, `2nd`, `3rd`, `4th` as a day of the month.
fn day_of(word: &str) -> Option<u32> {
    let digits = word
        .strip_suffix("st")
        .or_else(|| word.strip_suffix("nd"))
        .or_else(|| word.strip_suffix("rd"))
        .or_else(|| word.strip_suffix("th"))
        .unwrap_or(word);
    let d: u32 = digits.parse().ok()?;
    (digits.len() <= 2 && (1..=31).contains(&d)).then_some(d)
}

fn year_of(word: &str) -> Option<i32> {
    (word.len() == 4)
        .then(|| word.parse().ok())
        .flatten()
        .filter(|y| (1900..=9999).contains(y))
}

/// `2024-03-01`, `2024/3/1`, `2024.03.01`, `2024-03`.
fn iso(term: &str) -> Option<NaiveDate> {
    let sep = term.chars().find(|c| matches!(c, '-' | '/' | '.'))?;
    let parts: Vec<&str> = term.split(sep).collect();
    let num = |s: &str, max_len: usize| -> Option<u32> {
        (!s.is_empty() && s.len() <= max_len && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let year = i32::try_from(num(parts[0], 4)?)
        .ok()
        .filter(|_| parts[0].len() == 4)?;
    match parts[1..] {
        [m] => NaiveDate::from_ymd_opt(year, num(m, 2)?, 1),
        [m, d] => NaiveDate::from_ymd_opt(year, num(m, 2)?, num(d, 2)?),
        _ => None,
    }
}

/// The latest date on or before `today` with `month` and `day` (the 1st when `None`).
fn latest_month_day(today: NaiveDate, month: u32, day: Option<u32>) -> Option<NaiveDate> {
    let day = day.unwrap_or(1);
    (0..8)
        .filter_map(|back| NaiveDate::from_ymd_opt(today.year() - back, month, day))
        .find(|d| *d <= today)
}

enum Unit {
    Day,
    Week,
    Month,
    Year,
}

fn unit_of(word: &str) -> Option<Unit> {
    Some(match word.strip_suffix('s').unwrap_or(word) {
        "day" | "d" => Unit::Day,
        "week" | "w" => Unit::Week,
        "month" | "m" => Unit::Month,
        "year" | "y" => Unit::Year,
        _ => return None,
    })
}

fn back(today: NaiveDate, n: u32, unit: Unit) -> Option<NaiveDate> {
    match unit {
        Unit::Day => today.checked_sub_days(Days::new(n.into())),
        Unit::Week => today.checked_sub_days(Days::new(u64::from(n) * 7)),
        Unit::Month => today.checked_sub_months(Months::new(n)),
        Unit::Year => today.checked_sub_months(Months::new(n.checked_mul(12)?)),
    }
}

fn unsupported(term: &str) -> GrammarError {
    GrammarError(format!(
        "{term:?} isn't a date the search understands: use a date like 2024-03-01 or \
         \"march 1 2024\", a month or weekday name, today, yesterday, \"3 days ago\" (or 3d, \
         2w, 6m, 1y) or \"last week|month|year\""
    ))
}

/// The day `term` names, without the zone.
fn day(term: &str, ctx: &DateContext) -> Result<NaiveDate, GrammarError> {
    let lower = term.trim().to_lowercase().replace(',', " ");
    let words: Vec<&str> = lower.split_whitespace().collect();
    let none = || unsupported(term);
    let found = match words.as_slice() {
        [] => None,
        [w] => {
            if let Some(d) = iso(w) {
                Some(d)
            } else if *w == "today" {
                Some(ctx.today(term)?)
            } else if *w == "yesterday" {
                back(ctx.today(term)?, 1, Unit::Day)
            } else if let Some(m) = month_of(w) {
                latest_month_day(ctx.today(term)?, m, None)
            } else if let Some(wd) = weekday_of(w) {
                let today = ctx.today(term)?;
                let since = (7 + today.weekday().num_days_from_monday() - wd) % 7;
                back(today, since, Unit::Day)
            } else {
                // `3d`, `2w`, `6m`, `1y`.
                let split = w.find(|c: char| !c.is_ascii_digit()).unwrap_or(w.len());
                let (n, u) = w.split_at(split);
                match (n.parse::<u32>(), u) {
                    (Ok(n), "d" | "w" | "m" | "y") => {
                        back(ctx.today(term)?, n, unit_of(u).ok_or_else(none)?)
                    }
                    _ => None,
                }
            }
        }
        ["last", u] if matches!(*u, "week" | "month" | "year") => {
            back(ctx.today(term)?, 1, unit_of(u).ok_or_else(none)?)
        }
        [n, u, "ago"] => match (n.parse::<u32>(), unit_of(u)) {
            (Ok(n), Some(unit)) if u.len() > 1 => back(ctx.today(term)?, n, unit),
            _ => None,
        },
        [a, b] => match (month_of(a), month_of(b)) {
            (Some(m), _) => {
                if let Some(y) = year_of(b) {
                    NaiveDate::from_ymd_opt(y, m, 1)
                } else {
                    let d = day_of(b).ok_or_else(none)?;
                    latest_month_day(ctx.today(term)?, m, Some(d))
                }
            }
            (None, Some(m)) => {
                let d = day_of(a).ok_or_else(none)?;
                latest_month_day(ctx.today(term)?, m, Some(d))
            }
            _ => None,
        },
        [a, b, c] => {
            let (m, d) = match (month_of(a), month_of(b)) {
                (Some(m), _) => (m, day_of(b)),
                (None, Some(m)) => (m, day_of(a)),
                _ => return Err(none()),
            };
            match (d, year_of(c)) {
                (Some(d), Some(y)) => NaiveDate::from_ymd_opt(y, m, d),
                _ => None,
            }
        }
        _ => None,
    };
    found.ok_or_else(none)
}

/// The first instant (unix seconds) of the day `term` names, in `ctx`'s zone.
pub fn resolve(term: &str, ctx: &DateContext) -> Result<i64, GrammarError> {
    Ok(ctx.start_of(day(term, ctx)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-14 15:00 UTC, a Wednesday.
    const NOW: i64 = 1_791_990_000;

    fn day_in(term: &str, tz: &str) -> String {
        let ctx = DateContext::new(Some(tz), Some(NOW)).unwrap();
        day(term, &ctx)
            .unwrap_or_else(|e| panic!("{term}: {e}"))
            .to_string()
    }

    fn lisbon(term: &str) -> String {
        day_in(term, "Europe/Lisbon")
    }

    #[test]
    fn now_is_a_wednesday() {
        let d = DateTime::<Utc>::from_timestamp(NOW, 0).unwrap();
        assert_eq!(d.to_rfc3339(), "2026-10-14T15:00:00+00:00");
        assert_eq!(d.weekday(), chrono::Weekday::Wed);
    }

    #[test]
    fn absolute_dates() {
        assert_eq!(lisbon("2024-03-01"), "2024-03-01");
        assert_eq!(lisbon("2024/3/1"), "2024-03-01");
        assert_eq!(lisbon("2024.03.01"), "2024-03-01");
        assert_eq!(lisbon("2024-03"), "2024-03-01");
        assert_eq!(lisbon("March 2024"), "2024-03-01");
        assert_eq!(lisbon("march 1st, 2024"), "2024-03-01");
        assert_eq!(lisbon("1 Mar 2024"), "2024-03-01");
        assert_eq!(lisbon("Sept 30 2025"), "2025-09-30");
    }

    #[test]
    fn names_look_back() {
        assert_eq!(lisbon("today"), "2026-10-14");
        assert_eq!(lisbon("yesterday"), "2026-10-13");
        assert_eq!(lisbon("wednesday"), "2026-10-14");
        assert_eq!(lisbon("mon"), "2026-10-12");
        assert_eq!(lisbon("thursday"), "2026-10-08");
        assert_eq!(lisbon("october"), "2026-10-01");
        assert_eq!(lisbon("november"), "2025-11-01");
        assert_eq!(lisbon("oct 14"), "2026-10-14");
        assert_eq!(lisbon("15 oct"), "2025-10-15");
        assert_eq!(lisbon("feb 29"), "2024-02-29");
    }

    #[test]
    fn relative_terms() {
        assert_eq!(lisbon("3 days ago"), "2026-10-11");
        assert_eq!(lisbon("1 day ago"), "2026-10-13");
        assert_eq!(lisbon("2 weeks ago"), "2026-09-30");
        assert_eq!(lisbon("1 month ago"), "2026-09-14");
        assert_eq!(lisbon("2 years ago"), "2024-10-14");
        assert_eq!(lisbon("7d"), "2026-10-07");
        assert_eq!(lisbon("2w"), "2026-09-30");
        assert_eq!(lisbon("6m"), "2026-04-14");
        assert_eq!(lisbon("1y"), "2025-10-14");
        assert_eq!(lisbon("last week"), "2026-10-07");
        assert_eq!(lisbon("last month"), "2026-09-14");
        assert_eq!(lisbon("last year"), "2025-10-14");
    }

    #[test]
    fn the_zone_picks_today() {
        // 15:00 UTC on the 14th is already the 15th in Auckland and still the 14th in
        // Honolulu.
        assert_eq!(day_in("today", "Pacific/Auckland"), "2026-10-15");
        assert_eq!(day_in("today", "Pacific/Honolulu"), "2026-10-14");
        assert_eq!(day_in("yesterday", "Pacific/Auckland"), "2026-10-14");
    }

    #[test]
    fn start_of_day_in_the_zone() {
        let at = |tz: &str, term: &str| {
            resolve(term, &DateContext::new(Some(tz), Some(NOW)).unwrap()).unwrap()
        };
        // Lisbon is UTC+1 in summer: 2024-07-01 00:00 is 2024-06-30 23:00 UTC.
        assert_eq!(at("Europe/Lisbon", "2024-07-01"), 1_719_788_400);
        assert_eq!(at("UTC", "2024-07-01"), 1_719_792_000);
        assert_eq!(at("America/New_York", "2024-07-01"), 1_719_806_400);
        // Santiago's clocks skipped from 00:00 to 01:00 on 2024-09-08.
        assert_eq!(at("America/Santiago", "2024-09-08"), 1_725_768_000);
    }

    #[test]
    fn errors() {
        let ctx = DateContext::new(Some("Europe/Lisbon"), Some(NOW)).unwrap();
        for term in [
            "",
            "someday",
            "next week",
            "3",
            "3 days",
            "3 d ago",
            "2024-13-01",
            "2024-02-30",
            "24-03-01",
            "march 32",
            "march 1 24",
            "noon",
            "3x",
        ] {
            assert!(day(term, &ctx).is_err(), "{term:?} resolved");
        }
        let no_now = DateContext::new(None, None).unwrap();
        assert!(day("2024-03-01", &no_now).is_ok());
        assert!(day("march 2024", &no_now).is_ok());
        let e = day("yesterday", &no_now).unwrap_err();
        assert!(e.0.contains("search.now"), "{e}");
        assert!(DateContext::new(Some("Mars/Olympus"), None).is_err());
    }
}
