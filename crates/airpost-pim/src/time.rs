// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! DATE and DATE-TIME values (RFC 5545 sections 3.3.4 and 3.3.5) as an edit reads and writes
//! them, and the VTIMEZONE a written TZID needs (section 3.2.19).
//!
//! A value keeps the form it was written in: a DATE, a UTC time, or a wall-clock time with
//! the TZID it is read in (none for floating). Instants come from [`airpost_dav::Zones`], which
//! resolves TZIDs as ingestion does, so an edit reads a time the way the `Event` row's
//! columns were computed. A floating time, a DATE and a TZID nothing resolves are read in
//! the user's zone, [`Env::tz`], as the desktop read them in the machine's.

use crate::ics::{self, Component, Property};
use airpost_dav::{FloatingZone, Zones};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use std::str::FromStr;

/// What every edit needs besides the event: the user's zone and the time it is now.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    /// The zone floating times, DATE values and new all-day events are read in.
    pub tz: chrono_tz::Tz,
    /// Epoch seconds, for DTSTAMP.
    pub now: i64,
}

impl Env {
    pub fn new(tz: chrono_tz::Tz, now: i64) -> Self {
        Env { tz, now }
    }

    /// `tz` by name (IANA or Windows), UTC when it names none, at the current time.
    pub fn for_zone(tz: Option<&str>) -> Self {
        let tz = tz.and_then(resolve_iana).unwrap_or(chrono_tz::UTC);
        Env::new(tz, chrono::Utc::now().timestamp())
    }

    /// The zones of one resource, its floating times in [`Env::tz`].
    pub fn zones(&self, ics: &str) -> Zones {
        Zones::new(ics, FloatingZone::Zone(self.tz))
    }

    /// The calendar day `ts` falls on in [`Env::tz`].
    pub fn local_date(&self, ts: i64) -> NaiveDate {
        self.local(ts).date()
    }

    pub fn local(&self, ts: i64) -> NaiveDateTime {
        DateTime::from_timestamp(ts, 0)
            .unwrap_or_default()
            .with_timezone(&self.tz)
            .naive_local()
    }

    /// Midnight starting `date` in [`Env::tz`].
    pub fn day_start(&self, date: NaiveDate) -> i64 {
        self.local_ts(date.and_time(NaiveTime::MIN))
    }

    /// A wall-clock time in [`Env::tz`], with `mktime`'s rules for the hours a DST change
    /// repeats (the earlier) or skips (read with the offset from before the gap).
    pub fn local_ts(&self, naive: NaiveDateTime) -> i64 {
        Zones::new("", FloatingZone::Zone(self.tz)).to_utc(None, naive)
    }

    /// DTSTAMP for now: UTC, as RFC 5545 section 3.8.7.2 requires.
    pub fn stamp(&self) -> Property {
        Property::new("DTSTAMP", utc_text(self.now))
    }
}

/// One DATE or DATE-TIME value, in the form it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum When {
    Date(NaiveDate),
    Utc(NaiveDateTime),
    /// Wall clock in a TZID, or floating when `None`.
    Local(NaiveDateTime, Option<String>),
}

impl When {
    /// Reads `value` (`20260302`, `20260302T090000`, `20260302T090000Z`, with or without
    /// `-` and `:`) under `tzid`. A DATE-TIME with no time (`20260302T`) reads as a DATE, as
    /// the desktop's `fixJCalDatesWithoutTimes` did.
    pub fn parse(value: &str, tzid: Option<&str>) -> Option<When> {
        let v: String = value
            .trim()
            .chars()
            .filter(|c| *c != '-' && *c != ':')
            .collect();
        let (date_part, rest) = match v.find(['T', 't']) {
            Some(i) => (&v[..i], Some(&v[i + 1..])),
            None => (v.as_str(), None),
        };
        if date_part.len() != 8 || !date_part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let date = NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
        let Some(rest) = rest.filter(|r| !r.is_empty()) else {
            return Some(When::Date(date));
        };
        let (digits, utc) = match rest.strip_suffix(['Z', 'z']) {
            Some(d) => (d, true),
            None => (rest, false),
        };
        let digits = if digits.len() == 4 {
            format!("{digits}00")
        } else {
            digits.to_owned()
        };
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let time = NaiveTime::parse_from_str(&digits, "%H%M%S")
            .ok()
            .or_else(|| {
                // 23:59:60, a leap second some writers emit.
                let h: u32 = digits[..2].parse().ok()?;
                let m: u32 = digits[2..4].parse().ok()?;
                NaiveTime::from_hms_opt(h, m, 59)
            })?;
        let naive = date.and_time(time);
        Some(match (utc, tzid) {
            (true, _) => When::Utc(naive),
            (false, Some(id)) if !id.is_empty() => When::Local(naive, Some(id.to_owned())),
            (false, _) => When::Local(naive, None),
        })
    }

    /// The property's first value.
    pub fn of(prop: &Property) -> Option<When> {
        Self::all_of(prop).into_iter().next()
    }

    /// Every value of a multi-valued property (EXDATE, RDATE). PERIOD values are skipped.
    pub fn all_of(prop: &Property) -> Vec<When> {
        let tzid = prop.param("TZID");
        let is_date = prop
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
        prop.values()
            .iter()
            .filter(|v| !v.contains('/'))
            .filter_map(|v| When::parse(v, tzid))
            .map(|w| match (is_date, w) {
                (true, When::Utc(n) | When::Local(n, _)) => When::Date(n.date()),
                (_, w) => w,
            })
            .collect()
    }

    pub fn is_date(&self) -> bool {
        matches!(self, When::Date(_))
    }

    pub fn tzid(&self) -> Option<&str> {
        match self {
            When::Local(_, Some(id)) => Some(id),
            _ => None,
        }
    }

    /// The wall clock as written; midnight for a DATE.
    pub fn wall(&self) -> NaiveDateTime {
        match self {
            When::Date(d) => d.and_time(NaiveTime::MIN),
            When::Utc(n) | When::Local(n, _) => *n,
        }
    }

    pub fn date(&self) -> NaiveDate {
        self.wall().date()
    }

    /// Epoch seconds: a DATE at midnight in the floating zone.
    pub fn ts(&self, zones: &Zones) -> i64 {
        match self {
            When::Utc(n) => n.and_utc().timestamp(),
            When::Date(d) => zones.to_utc(None, d.and_time(NaiveTime::MIN)),
            When::Local(n, tzid) => zones.to_utc(tzid.as_deref(), *n),
        }
    }

    /// The value as written in a content line.
    pub fn text(&self) -> String {
        match self {
            When::Date(d) => d.format("%Y%m%d").to_string(),
            When::Utc(n) => format!("{}Z", n.format("%Y%m%dT%H%M%S")),
            When::Local(n, _) => n.format("%Y%m%dT%H%M%S").to_string(),
        }
    }

    /// The same form, `secs` later on its own clock (whole days for a DATE), as ical.js's
    /// `addDuration` moves a time.
    pub fn add_wall(&self, secs: i64) -> When {
        match self {
            When::Date(d) => When::Date(*d + Duration::days(secs.div_euclid(86400))),
            When::Utc(n) => When::Utc(*n + Duration::seconds(secs)),
            When::Local(n, z) => When::Local(*n + Duration::seconds(secs), z.clone()),
        }
    }

    /// A content line for this value: VALUE=DATE for a DATE, TZID for a zoned time.
    pub fn property(&self, name: &str) -> Property {
        let mut p = Property::new(name, self.text());
        self.stamp_params(&mut p);
        p
    }

    fn stamp_params(&self, p: &mut Property) {
        p.remove_param("TZID");
        p.remove_param("VALUE");
        match self {
            When::Date(_) => p.set_param("VALUE", "DATE"),
            When::Local(_, Some(id)) => p.set_param("TZID", id),
            _ => {}
        }
    }
}

/// Sets the first `name` property to `when`, keeping its other parameters (ical.js
/// `_setTime`); any further `name` properties go.
pub fn set_when(comp: &mut Component, name: &str, when: &When) {
    let mut prop = comp
        .prop(name)
        .cloned()
        .unwrap_or_else(|| Property::new(name, ""));
    prop.value = when.text();
    when.stamp_params(&mut prop);
    comp.replace(prop);
}

/// The VEVENT's DTSTART.
pub fn start_of(vevent: &Component) -> Option<When> {
    vevent.prop("DTSTART").and_then(When::of)
}

/// The VEVENT's RECURRENCE-ID.
pub fn rid_of(vevent: &Component) -> Option<When> {
    vevent.prop("RECURRENCE-ID").and_then(When::of)
}

/// How long the VEVENT lasts on its own clock (ical.js `Event.duration`): DTEND less
/// DTSTART, else DURATION, else a day for a DATE and nothing for a DATE-TIME.
pub fn duration_of(vevent: &Component) -> i64 {
    let Some(start) = start_of(vevent) else {
        return 0;
    };
    if let Some(end) = vevent.prop("DTEND").and_then(When::of) {
        return (end.wall() - start.wall()).num_seconds();
    }
    if let Some(d) = vevent.value("DURATION").and_then(parse_duration) {
        return d;
    }
    if start.is_date() {
        86400
    } else {
        0
    }
}

/// The VEVENT's end: DTEND, else DTSTART plus [`duration_of`].
pub fn end_of(vevent: &Component) -> Option<When> {
    if let Some(end) = vevent.prop("DTEND").and_then(When::of) {
        return Some(end);
    }
    let start = start_of(vevent)?;
    Some(start.add_wall(duration_of(vevent)))
}

/// A DURATION value (RFC 5545 section 3.3.6) in seconds; weeks and days count as 86400.
pub fn parse_duration(value: &str) -> Option<i64> {
    let v = value.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix(['P', 'p'])?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut in_time = false;
    for c in v.chars() {
        match c.to_ascii_uppercase() {
            'T' => in_time = true,
            d if d.is_ascii_digit() => num.push(d),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += n * match (unit, in_time) {
                    ('W', _) => 7 * 86400,
                    ('D', _) => 86400,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    num.is_empty().then_some(sign * total)
}

/// UTC and the names that mean it; these are written with a trailing Z rather than a TZID.
pub fn is_utc_alias(tzid: &str) -> bool {
    matches!(
        tzid.trim().to_ascii_lowercase().as_str(),
        "utc" | "gmt" | "etc/utc" | "etc/gmt" | "z"
    )
}

/// The IANA zone a TZID describes, IANA or Windows name (`resolveIanaZone`); `None` for UTC
/// and its aliases and for names nothing maps (Outlook display names, private ids).
pub fn resolve_iana(tzid: &str) -> Option<chrono_tz::Tz> {
    if tzid.is_empty() || is_utc_alias(tzid) {
        return None;
    }
    match calcard::common::timezone::Tz::from_str(tzid) {
        Ok(calcard::common::timezone::Tz::Tz(tz)) => Some(tz),
        _ => None,
    }
}

/// Epoch seconds as a UTC DATE-TIME value.
pub fn utc_text(ts: i64) -> String {
    When::Utc(utc_naive(ts)).text()
}

pub fn utc_naive(ts: i64) -> NaiveDateTime {
    DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .naive_utc()
}

/// The value an edit writes for instant `ts` (ical.js `createICALTime`): a DATE on the
/// user's calendar day for an all-day event; the wall clock in `preserve` when that TZID
/// resolves and is not UTC; UTC otherwise.
pub fn zoned(ts: i64, all_day: bool, preserve: Option<&str>, zones: &Zones, env: &Env) -> When {
    if all_day {
        return When::Date(env.local_date(ts));
    }
    match preserve {
        Some(id) if !is_utc_alias(id) && zones.knows(id) => {
            When::Local(zones.to_local(Some(id), ts), Some(id.to_owned()))
        }
        _ => When::Utc(utc_naive(ts)),
    }
}

/// An all-day DTEND, exclusive (RFC 5545 section 3.6.1): the day after the last one `end`
/// covers, never before the day after `start`.
pub fn all_day_end(start: i64, end: i64, env: &Env) -> When {
    let last_covered = (end - 1).max(start);
    When::Date(env.local_date(last_covered) + Duration::days(1))
}

/// The wall clock of `ts` in IANA zone `tz`.
pub fn wall_in(tz: chrono_tz::Tz, ts: i64) -> NaiveDateTime {
    DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .with_timezone(&tz)
        .naive_local()
}

/// Seconds east of UTC in `tz` at `ts`.
pub fn offset_in(tz: chrono_tz::Tz, ts: i64) -> i64 {
    (wall_in(tz, ts) - utc_naive(ts)).num_seconds()
}

/// A VTIMEZONE for `tzid` covering a year before `reference` to ten after, from chrono-tz
/// through calcard; `None` for UTC and for names nothing maps.
pub fn vtimezone(tzid: &str, reference: i64) -> Option<Component> {
    resolve_iana(tzid)?;
    let mut cal = calcard::icalendar::ICalendar::parse(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Airpost//Calendar//EN\r\nEND:VCALENDAR\r\n",
    )
    .ok()?;
    let year = 366 * 86400;
    cal.add_timezone(tzid, reference - year, reference + 10 * year)?;
    let parsed = ics::parse(&cal.to_string()).ok()?;
    parsed.children.into_iter().find(|c| c.name == "VTIMEZONE")
}

/// Brings a VCALENDAR's VTIMEZONEs in line with the TZIDs its components reference: one
/// still referenced keeps the server's own, one nothing references goes, and one missing is
/// taken from `donor` (the other calendar an edit copies from) or built. TZIDs nothing maps
/// get none (`syncVTimezones`).
pub fn sync_vtimezones(cal: &mut Component, reference: i64, donor: Option<&Component>) {
    if cal.name != "VCALENDAR" {
        return;
    }
    let mut referenced: Vec<String> = Vec::new();
    for child in cal.children.iter().filter(|c| c.name != "VTIMEZONE") {
        collect_tzids(child, &mut referenced);
    }
    let tzid_of = |c: &Component| c.value("TZID").unwrap_or_default().to_owned();
    cal.children
        .retain(|c| c.name != "VTIMEZONE" || referenced.contains(&tzid_of(c)));
    let present: Vec<String> = cal
        .children
        .iter()
        .filter(|c| c.name == "VTIMEZONE")
        .map(tzid_of)
        .collect();
    // Appended, as ical.js did, so the indexes of the components already there hold.
    let mut added = Vec::new();
    for id in referenced.iter().filter(|id| !present.contains(id)) {
        let from_donor = donor.and_then(|d| {
            d.children_named("VTIMEZONE")
                .find(|c| tzid_of(c) == *id)
                .cloned()
        });
        if let Some(vtz) = from_donor.or_else(|| vtimezone(id, reference)) {
            added.push(vtz);
        }
    }
    cal.children.extend(added);
}

fn collect_tzids(comp: &Component, out: &mut Vec<String>) {
    for p in &comp.props {
        if let Some(id) = p.param("TZID") {
            if !out.iter().any(|o| o == id) {
                out.push(id.to_owned());
            }
        }
    }
}

/// `YYYYMMDD` for a DATE, as RECURRENCE-ID texts are compared.
pub fn date_text(d: NaiveDate) -> String {
    format!("{:04}{:02}{:02}", d.year(), d.month(), d.day())
}

/// Seconds since local midnight.
pub fn seconds_into_day(n: NaiveDateTime) -> i64 {
    i64::from(n.time().num_seconds_from_midnight())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env::new(chrono_tz::America::Chicago, 1_767_225_600)
    }

    #[test]
    fn parses_forms() {
        assert_eq!(
            When::parse("20260302", None),
            Some(When::Date(NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()))
        );
        assert!(matches!(
            When::parse("20260302T090000Z", None),
            Some(When::Utc(_))
        ));
        assert!(matches!(
            When::parse("20260302T090000", Some("America/Chicago")),
            Some(When::Local(_, Some(_)))
        ));
        assert!(matches!(
            When::parse("2026-03-02T", None),
            Some(When::Date(_))
        ));
        assert_eq!(When::parse("garbage", None), None);
    }

    #[test]
    fn instants_through_zones() {
        let env = env();
        let zones = env.zones("");
        let w = When::parse("20260302T090000", Some("America/New_York")).unwrap();
        assert_eq!(w.ts(&zones), 1_772_460_000);
        let d = When::parse("20260302", None).unwrap();
        assert_eq!(d.ts(&zones), 1_772_431_200);
        assert_eq!(
            zoned(1_772_460_000, false, Some("America/New_York"), &zones, &env),
            w
        );
        assert_eq!(
            zoned(1_772_460_000, false, Some("UTC"), &zones, &env).text(),
            "20260302T140000Z"
        );
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1W"), Some(604800));
        assert_eq!(parse_duration("-P1DT2H"), Some(-93600));
        assert_eq!(parse_duration("1H"), None);
    }

    #[test]
    fn builds_vtimezones() {
        let vtz = vtimezone("America/Chicago", 1_772_460_000).unwrap();
        assert_eq!(vtz.value("TZID"), Some("America/Chicago"));
        assert!(vtz.children.iter().any(|c| c.name == "DAYLIGHT"));
        let windows = vtimezone("Central Standard Time", 1_772_460_000).unwrap();
        assert_eq!(windows.value("TZID"), Some("Central Standard Time"));
        assert!(vtimezone("UTC", 0).is_none());
        assert!(vtimezone("Customized Time Zone", 0).is_none());
    }
}
