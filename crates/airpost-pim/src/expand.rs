// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Recurrence expansion, as the desktop's ical-expander use did it: every occurrence of a
//! resource's VEVENTs that overlaps a window, each series' exceptions substituted at the
//! slots their RECURRENCE-IDs name.
//!
//! calcard generates a series' slots (RRULE, RDATE) from the master alone; the exceptions
//! are matched here by instant, since calcard keys them by SEQUENCE as well and a master
//! revised after its exceptions would show both. EXDATEs are applied here too, so they
//! exclude RDATEs as well, by instant whatever zone they are written in.

use crate::edit::{rrule_part, set_rrule_part, Doc};
use crate::time::{duration_of, end_of, rid_of, start_of, When};
use crate::{Env, Error, Result};
use chrono::{Duration, NaiveDate};

/// One occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occ {
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// For an all-day occurrence, its first day and the exclusive end date, as written.
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    /// Index into the [`Doc`]'s children of the VEVENT whose properties it shows: the
    /// master, or the exception that overrides this slot.
    pub vevent: usize,
    /// The slot it fills, for an occurrence of a series (its RECURRENCE-ID).
    pub recurrence_id: Option<i64>,
}

const STEP_SECONDS: [(&str, i64); 7] = [
    ("SECONDLY", 1),
    ("MINUTELY", 60),
    ("HOURLY", 3600),
    ("DAILY", 86400),
    ("WEEKLY", 604_800),
    // The shortest month and year: underestimating the step overestimates the budget, which
    // errs towards expanding a legitimate series rather than truncating it.
    ("MONTHLY", 28 * 86400),
    ("YEARLY", 365 * 86400),
];
const MIN_ITERATIONS: usize = 1000;
const MAX_ITERATIONS: usize = 50000;

/// How many slots a series may step through (`expansionIterationBudget`): the steps of its
/// own frequency from where it starts to the end of the window, plus slack, between 1000
/// and 50000. Generation runs forward from DTSTART, so a fixed cap would make a long-running
/// series vanish; no cap would let `FREQ=SECONDLY` from 1970 spin forever.
pub fn budget(rule: Option<&str>, series_start: Option<i64>, window_end: i64) -> usize {
    let (Some(rule), Some(series_start)) = (rule, series_start) else {
        return MIN_ITERATIONS;
    };
    let freq = rrule_part(rule, "FREQ")
        .unwrap_or_default()
        .to_ascii_uppercase();
    let interval = rrule_part(rule, "INTERVAL")
        .and_then(|i| i.parse::<i64>().ok())
        .filter(|i| *i > 0)
        .unwrap_or(1);
    let step = STEP_SECONDS
        .iter()
        .find(|(f, _)| *f == freq)
        .map(|(_, s)| *s)
        .unwrap_or(86400)
        * interval;
    let span = (window_end - series_start).max(0);
    let steps = usize::try_from((span + step - 1) / step).unwrap_or(MAX_ITERATIONS) + 100;
    steps.clamp(MIN_ITERATIONS, MAX_ITERATIONS)
}

/// The budget for the first VEVENT's RRULE (`expansionIterationBudget(ics, …)`).
pub fn budget_for(ics: &str, series_start: Option<i64>, window_end: i64) -> usize {
    let rule = Doc::parse(ics)
        .ok()
        .and_then(|d| d.first().and_then(|v| v.value("RRULE")).map(str::to_owned));
    budget(rule.as_deref(), series_start, window_end)
}

/// [`expand`] on text.
pub fn expand_text(
    ics: &str,
    series_start: i64,
    after: i64,
    before: i64,
    env: &Env,
) -> Result<Vec<Occ>> {
    expand(&Doc::parse(ics)?, Some(series_start), after, before, env)
}

/// Every occurrence that overlaps `[after, before]`, inclusive at both ends as
/// ical-expander's `between` is (an all-day end, being exclusive, is not). A series stops at
/// the first occurrence that starts after `before`. `series_start` is the row's `rs`, which
/// sizes the [`budget`]; `None` reads it from DTSTART.
pub fn expand(
    doc: &Doc,
    series_start: Option<i64>,
    after: i64,
    before: i64,
    env: &Env,
) -> Result<Vec<Occ>> {
    let zones = doc.zones(env);
    let in_range = |s: i64, e: i64, end_is_date: bool| {
        let end_ok = if end_is_date && e > s {
            e > after
        } else {
            e >= after
        };
        end_ok && s <= before
    };
    let overrides: Vec<(usize, i64, String)> = doc
        .vevent_ids()
        .into_iter()
        .filter_map(|i| {
            let v = &doc.cal.children[i];
            let rid = rid_of(v)?;
            Some((
                i,
                rid.ts(&zones),
                v.value("UID").unwrap_or_default().to_owned(),
            ))
        })
        .collect();

    let mut out = Vec::new();
    for mi in doc.vevent_ids() {
        let master = &doc.cal.children[mi];
        if master.has("RECURRENCE-ID") {
            continue;
        }
        let start =
            start_of(master).ok_or_else(|| Error::invalid_ics("a VEVENT has no DTSTART"))?;
        let start_ts = start.ts(&zones);
        if !master.has("RRULE") && !master.has("RDATE") {
            let end = end_of(master).unwrap_or_else(|| start.clone());
            let e = end.ts(&zones);
            if in_range(start_ts, e, end.is_date()) {
                out.push(occ(start_ts, e, &start, &end, mi, None));
            }
            continue;
        }

        let uid = master.value("UID").unwrap_or_default();
        let excluded: Vec<i64> = master
            .props_named("EXDATE")
            .flat_map(When::all_of)
            .map(|w| match (&w, &start) {
                // A DATE EXDATE on a timed series excludes that day's occurrence.
                (When::Date(d), s) if !s.is_date() => {
                    When::Local(d.and_time(s.wall().time()), s.tzid().map(str::to_owned)).ts(&zones)
                }
                _ => w.ts(&zones),
            })
            .collect();
        let limit = budget(
            master.value("RRULE"),
            series_start.or(Some(start_ts)),
            before,
        );
        let mut slots = slots_of(doc, mi, &start, limit, env)?;
        slots.sort_unstable();
        slots.dedup();
        let duration = duration_of(master);

        for slot in slots {
            if excluded.contains(&slot) {
                continue;
            }
            let found = overrides
                .iter()
                .find(|(_, rid, ouid)| *rid == slot && ouid == uid);
            let item = match found {
                Some((oi, _, _)) => {
                    let ov = &doc.cal.children[*oi];
                    let Some(os) = start_of(ov) else { continue };
                    let oe = end_of(ov).unwrap_or_else(|| os.clone());
                    (os.ts(&zones), oe.ts(&zones), os, oe, *oi)
                }
                None => {
                    let s = match &start {
                        When::Date(_) => When::Date(env.local_date(slot)),
                        When::Utc(_) => When::Utc(crate::time::utc_naive(slot)),
                        When::Local(_, tz) => {
                            When::Local(zones.to_local(tz.as_deref(), slot), tz.clone())
                        }
                    };
                    let e = s.add_wall(duration);
                    let e_ts = match &e {
                        When::Utc(_) => slot + duration,
                        other => other.ts(&zones),
                    };
                    (slot, e_ts, s, e, mi)
                }
            };
            let (s, e, sw, ew, vi) = item;
            if s > before {
                break;
            }
            if in_range(s, e, ew.is_date()) {
                out.push(occ(s, e, &sw, &ew, vi, Some(slot)));
            }
        }
    }
    Ok(out)
}

fn occ(start: i64, end: i64, sw: &When, ew: &When, vevent: usize, rid: Option<i64>) -> Occ {
    let start_date = sw.date();
    let end_date = if ew.is_date() {
        ew.date()
    } else {
        start_date + Duration::days(1)
    };
    Occ {
        start,
        end,
        all_day: sw.is_date(),
        start_date,
        end_date,
        vevent,
        recurrence_id: rid,
    }
}

/// The instants of a series' slots, generating at most `limit`.
///
/// calcard steps the rule on the wall clock of DTSTART's own zone (RFC 5545 section 3.3.10),
/// given as floating time read in UTC; each wall-clock slot then becomes an instant here,
/// through the same zone resolution ingestion uses. calcard alone would read a UTC DTSTART's
/// slots in its default zone, and a custom VTIMEZONE's not at all.
fn slots_of(doc: &Doc, mi: usize, start: &When, limit: usize, env: &Env) -> Result<Vec<i64>> {
    let zones = doc.zones(env);
    let original = &doc.cal.children[mi];
    let tzid = start.tzid().map(str::to_owned);
    // The wall clock of an instant in DTSTART's frame.
    let wall_of = |ts: i64| match start {
        When::Utc(_) => crate::time::utc_naive(ts),
        When::Date(_) => env.local(ts),
        When::Local(_, tz) => zones.to_local(tz.as_deref(), ts),
    };
    let slot_ts = |wall: chrono::NaiveDateTime| match start {
        When::Utc(_) => wall.and_utc().timestamp(),
        When::Date(_) => When::Date(wall.date()).ts(&zones),
        When::Local(..) => When::Local(wall, tzid.clone()).ts(&zones),
    };

    let mut master = crate::ics::Component::new("VEVENT");
    master.props.push(crate::ics::Property::new(
        "UID",
        original.value("UID").unwrap_or("series"),
    ));
    let floating_start = match start {
        When::Date(d) => When::Date(*d),
        other => When::Local(other.wall(), None),
    };
    master.props.push(floating_start.property("DTSTART"));
    let mut rdates: Vec<i64> = Vec::new();
    for p in original.props_named("RDATE") {
        rdates.extend(When::all_of(p).iter().map(|w| w.ts(&zones)));
    }
    if let Some(rule) = original.prop("RRULE") {
        let mut value = rule.value.clone();
        if let Some(until) = rrule_part(&value, "UNTIL").and_then(|u| When::parse(&u, None)) {
            let rewritten = match (&until, start) {
                (When::Date(d), When::Date(_)) => d.format("%Y%m%d").to_string(),
                // A DATE UNTIL on a timed series includes that whole day.
                (When::Date(d), _) => format!("{}T235959", d.format("%Y%m%d")),
                (When::Utc(_), _) => wall_of(until.ts(&zones))
                    .format("%Y%m%dT%H%M%S")
                    .to_string(),
                (When::Local(n, _), When::Date(_)) => n.format("%Y%m%dT%H%M%S").to_string(),
                (When::Local(n, _), _) => n.format("%Y%m%dT%H%M%S").to_string(),
            };
            value = set_rrule_part(&value, "UNTIL", &rewritten);
        }
        master.props.push(crate::ics::Property::new("RRULE", value));
    }
    let mut cal = crate::ics::Component::new("VCALENDAR");
    cal.props.push(crate::ics::Property::new("VERSION", "2.0"));
    cal.children.push(master);

    let mut slots = vec![start.ts(&zones)];
    if original.has("RRULE") {
        let parsed = calcard::icalendar::ICalendar::parse(cal.to_ics())
            .map_err(|_| Error::invalid_ics("the series could not be read"))?;
        let expanded = parsed.expand_dates(calcard::common::timezone::Tz::UTC, limit);
        if expanded.events.is_empty() {
            if let Some(err) = expanded.errors.first() {
                let detail = format!("{:?}", err.error);
                // A series whose UNTIL precedes its start has just its first occurrence.
                if !detail.contains("UntilBeforeStart") {
                    return Err(Error::invalid_ics(format!(
                        "the series could not be expanded: {detail}"
                    )));
                }
            }
        }
        slots = expanded
            .events
            .iter()
            .map(|e| slot_ts(e.start.naive_utc()))
            .collect();
        if slots.is_empty() {
            slots.push(start.ts(&zones));
        }
    }
    slots.extend(rdates);
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env::new(chrono_tz::America::Chicago, 1_767_225_600)
    }

    #[test]
    fn budgets() {
        assert_eq!(budget(None, Some(0), 100), 1000);
        assert_eq!(budget(Some("FREQ=SECONDLY"), Some(0), 10_000_000), 50000);
        assert_eq!(budget(Some("FREQ=WEEKLY"), Some(0), 604_800 * 2000), 2100);
        assert_eq!(budget(Some("FREQ=WEEKLY"), None, 1), 1000);
    }

    #[test]
    fn weekly_with_exception_and_exdate() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=America/Chicago:20260302T090000\r\nDTEND;TZID=America/Chicago:20260302T100000\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE:20260316T140000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:w\r\nRECURRENCE-ID:20260309T140000Z\r\nDTSTART;TZID=America/Chicago:20260309T120000\r\nDTEND;TZID=America/Chicago:20260309T130000\r\nSUMMARY:moved\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let doc = Doc::parse(ics).unwrap();
        let occs = expand(&doc, None, 0, 2_000_000_000, &env()).unwrap();
        let starts: Vec<i64> = occs.iter().map(|o| o.start).collect();
        // 03-02 09:00 CST, 03-09 12:00 CDT (moved), 03-23 09:00 CDT
        assert_eq!(starts, vec![1_772_463_600, 1_773_075_600, 1_774_274_400]);
        assert_eq!(occs[1].vevent, 1);
        assert_eq!(occs[1].recurrence_id, Some(1_773_064_800));
        assert_eq!(occs[0].end - occs[0].start, 3600);
    }

    #[test]
    fn until_under_a_zoned_start_is_exact() {
        // UNTIL one second before the 03-09 occurrence (14:00Z) stops the series there.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=America/Chicago:20260302T090000\r\nDTEND;TZID=America/Chicago:20260302T100000\r\nRRULE:FREQ=WEEKLY;UNTIL=20260309T135959Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let occs = expand(&Doc::parse(ics).unwrap(), None, 0, 2_000_000_000, &env()).unwrap();
        assert_eq!(occs.len(), 1);
    }

    #[test]
    fn all_day_window_edges() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:d\r\nDTSTART;VALUE=DATE:20260622\r\nDTEND;VALUE=DATE:20260623\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let doc = Doc::parse(ics).unwrap();
        let env = env();
        let next_midnight = env.day_start(NaiveDate::from_ymd_opt(2026, 6, 23).unwrap());
        assert!(expand(&doc, None, next_midnight, next_midnight + 10, &env)
            .unwrap()
            .is_empty());
        let occs = expand(&doc, None, next_midnight - 1, next_midnight, &env).unwrap();
        assert_eq!(occs.len(), 1);
        assert!(occs[0].all_day);
        assert_eq!(
            occs[0].end_date,
            NaiveDate::from_ymd_opt(2026, 6, 23).unwrap()
        );
    }
}
