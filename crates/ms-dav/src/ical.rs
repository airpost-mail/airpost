//! The Event columns taken from an ICS resource. Derived from Mailspring-Sync
//! `MailSync/Models/Event.cpp` (`applyICSEventData`, `endOf`) and `Vendor/icalendarlib`
//! (`ICalendar::LoadFromString`, `ParseAttendee`, `Date::toUnix`), Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md. Parsing is calcard's.
//!
//! Times follow RFC 5545 §3.3.5, where icalendarlib resolved everything but UTC against the
//! machine's zone:
//! - a value ending in `Z` or carrying an offset is exact;
//! - a `TZID` value resolves through the resource's own VTIMEZONE (TZID, `X-LIC-LOCATION`,
//!   Microsoft CDO ids), an IANA or Windows zone name, or, for a custom VTIMEZONE no name
//!   matches, the offsets that VTIMEZONE defines;
//! - a floating value, a `DATE`, or a `TZID` nothing resolves uses the [`FloatingZone`].

use calcard::common::PartialDateTime;
use calcard::icalendar::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry, ICalendarFrequency,
    ICalendarParameterName, ICalendarProperty, ICalendarValue, ICalendarWeekday,
};
use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime,
    NaiveTime, Offset, TimeZone, Utc, Weekday,
};
use std::str::FromStr;

/// `endOf` for a series with no UNTIL (forever, or COUNT, which icalendarlib never
/// expanded): the client expands recurrences itself and only needs an upper bound.
pub const OPEN_RECURRENCE_END: &str = "20370000T000000";

/// [`OPEN_RECURRENCE_END`] in epoch seconds. icalendarlib's `get_time` stops at month `00`
/// and `mktime` normalises day 0 of 2037 to 2036-12-31 00:00 local time; this is that
/// instant in UTC, so the value no longer depends on the machine's zone.
pub const OPEN_RECURRENCE_END_TS: i64 = 2_114_294_400;

/// Status stored when a VEVENT has none (`Event::status`).
pub const DEFAULT_STATUS: &str = "CONFIRMED";

/// How floating times, `DATE` values and unresolvable `TZID`s become instants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FloatingZone {
    /// The machine's zone, as icalendarlib's `mktime` used.
    #[default]
    Local,
    Zone(chrono_tz::Tz),
}

/// One VEVENT's fields, in the shape `ms_store::IcsEventFields` takes them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventFields {
    pub uid: String,
    /// `RECURRENCE-ID` written as `YYYYMMDD`, `YYYYMMDDTHHMMSS` or `YYYYMMDDTHHMMSSZ`;
    /// empty for a series master or a single event.
    pub recurrence_id: String,
    /// `STATUS` as written, empty when absent; see [`EventFields::status_or_default`].
    pub status: String,
    /// `DTSTART` in epoch seconds; `None` when the VEVENT has none, which ingestion skips.
    pub start: Option<i64>,
    /// `DTSTART` is a `DATE`.
    pub all_day: bool,
    /// `rs`: `DTSTART` in epoch seconds (0 without one).
    pub recurrence_start: i64,
    /// `re` (`endOf`): without an RRULE, DTEND, else DTSTART + DURATION, else DTSTART; with
    /// one, its UNTIL (the end of that day for a `DATE` UNTIL), else
    /// [`OPEN_RECURRENCE_END_TS`].
    pub recurrence_end: i64,
    pub is_recurring: bool,
    pub has_dtstamp: bool,
    pub summary: String,
    pub description: String,
    pub location: String,
    /// `ORGANIZER` as `Name <email>` or `email`, empty when absent.
    pub organizer: String,
    /// Each `ATTENDEE` as `Name <email>` or `email` (`ParseAttendee`).
    pub attendees: Vec<String>,
}

impl EventFields {
    pub fn status_or_default(&self) -> &str {
        if self.status.is_empty() {
            DEFAULT_STATUS
        } else {
            &self.status
        }
    }

    pub fn is_recurrence_exception(&self) -> bool {
        !self.recurrence_id.is_empty()
    }

    /// The `EventSearch.participants` column: attendees joined with spaces.
    pub fn search_participants(&self) -> String {
        self.attendees.join(" ")
    }
}

/// Every VEVENT in `ics` (master and exceptions), resolving floating times in the
/// machine's zone. An unparseable resource yields none, as icalendarlib's did.
pub fn parse_events(ics: &str) -> Vec<EventFields> {
    parse_events_in(ics, FloatingZone::Local)
}

pub fn parse_events_in(ics: &str, floating: FloatingZone) -> Vec<EventFields> {
    let Ok(cal) = ICalendar::parse(ics) else {
        return Vec::new();
    };
    let ctx = ZoneContext {
        cal: &cal,
        floating,
    };
    let mut uidless = 0;
    cal.components
        .iter()
        .filter(|c| c.component_type == ICalendarComponentType::VEvent)
        .map(|c| {
            let mut ev = event_fields(c, &ctx);
            // icalendarlib numbered UID-less events from 0 within the file.
            if ev.uid.is_empty() {
                ev.uid = uidless.to_string();
                uidless += 1;
            }
            ev
        })
        .collect()
}

/// The VEVENT a row with `recurrence_id` stands for, else the first one: the server may
/// return the master and its exceptions in one resource (`writeAndResyncEvent`).
pub fn matching_event<'a>(
    events: &'a [EventFields],
    recurrence_id: &str,
) -> Option<&'a EventFields> {
    events
        .iter()
        .find(|e| e.recurrence_id == recurrence_id)
        .or_else(|| events.first())
}

/// The series master (the first VEVENT without RECURRENCE-ID), else the first VEVENT
/// (`performLocalSyncbackEvent`).
pub fn series_master(events: &[EventFields]) -> Option<&EventFields> {
    events
        .iter()
        .find(|e| e.recurrence_id.is_empty())
        .or_else(|| events.first())
}

/// VEVENTs sharing `uid`: a resource holding more than one is a series with exceptions.
pub fn count_with_uid(events: &[EventFields], uid: &str) -> usize {
    events.iter().filter(|e| e.uid == uid).count()
}

fn event_fields(c: &ICalendarComponent, ctx: &ZoneContext<'_>) -> EventFields {
    let mut ev = EventFields::default();
    let mut dtstart = None;
    let mut dtend = None;
    let mut duration = None;
    let mut rrule = None;

    for entry in &c.entries {
        match &entry.name {
            ICalendarProperty::Uid => ev.uid = text(entry),
            ICalendarProperty::Summary => ev.summary = text(entry),
            ICalendarProperty::Description => ev.description = text(entry),
            ICalendarProperty::Location => ev.location = text(entry),
            ICalendarProperty::Status => ev.status = text(entry),
            ICalendarProperty::Dtstamp => ev.has_dtstamp = true,
            ICalendarProperty::Dtstart => dtstart = Some(entry),
            ICalendarProperty::Dtend => dtend = Some(entry),
            ICalendarProperty::Duration => duration = Some(entry),
            ICalendarProperty::Rrule => rrule = Some(entry),
            ICalendarProperty::RecurrenceId => {
                ev.recurrence_id = date_value(entry).map(format_ical).unwrap_or_default()
            }
            ICalendarProperty::Attendee => ev.attendees.push(participant(entry)),
            ICalendarProperty::Organizer => ev.organizer = participant(entry),
            _ => {}
        }
    }

    let start = dtstart.and_then(|e| ctx.instant(e));
    ev.start = start.map(|s| s.ts);
    ev.all_day = start.is_some_and(|s| !s.has_time);
    ev.recurrence_start = ev.start.unwrap_or(0);

    let rule = rrule.and_then(|e| match e.values.first() {
        Some(ICalendarValue::RecurrenceRule(r)) => Some(r),
        _ => None,
    });
    ev.is_recurring = rrule.is_some();
    ev.recurrence_end = match (rrule, start) {
        (Some(_), start) => rule
            .and_then(|r| r.until.as_ref())
            .and_then(|until| ctx.until(until, dtstart, start))
            .unwrap_or(OPEN_RECURRENCE_END_TS),
        (None, Some(start)) => dtend
            .and_then(|e| ctx.instant(e).map(|i| i.ts))
            .or_else(|| {
                duration
                    .and_then(|d| match d.values.first() {
                        Some(ICalendarValue::Duration(d)) => Some(d),
                        _ => None,
                    })
                    .and_then(|d| ctx.add_duration(start, d))
            })
            .unwrap_or(start.ts),
        (None, None) => 0,
    };
    ev
}

fn text(entry: &ICalendarEntry) -> String {
    entry
        .values
        .first()
        .and_then(ICalendarValue::as_text)
        .unwrap_or("")
        .to_string()
}

fn date_value(entry: &ICalendarEntry) -> Option<&PartialDateTime> {
    entry.values.first().and_then(|v| v.as_partial_date_time())
}

/// `ParseAttendee`: `CN <email>` when a CN parameter is present, else the address with any
/// `mailto:` stripped. The CN comes only from the parameters, never from the value.
fn participant(entry: &ICalendarEntry) -> String {
    let address = entry
        .values
        .first()
        .and_then(ICalendarValue::as_text)
        .map(strip_mailto)
        .unwrap_or("");
    let cn = entry
        .parameter(&ICalendarParameterName::Cn)
        .and_then(|v| v.as_text())
        .unwrap_or("");
    if cn.is_empty() {
        address.to_string()
    } else {
        format!("{cn} <{address}>")
    }
}

fn strip_mailto(value: &str) -> &str {
    match value.get(..7) {
        Some(p) if p.eq_ignore_ascii_case("mailto:") => &value[7..],
        _ => value,
    }
}

/// `YYYYMMDD[THHMMSS[Z]]`, the basic format RFC 5545 §3.3.4-5 writes.
fn format_ical(p: &PartialDateTime) -> String {
    let mut s = format!(
        "{:04}{:02}{:02}",
        p.year.unwrap_or(0),
        p.month.unwrap_or(0),
        p.day.unwrap_or(0)
    );
    if p.hour.is_some() {
        s.push_str(&format!(
            "T{:02}{:02}{:02}",
            p.hour.unwrap_or(0),
            p.minute.unwrap_or(0),
            p.second.unwrap_or(0)
        ));
        if is_utc(p) {
            s.push('Z');
        }
    }
    s
}

fn is_utc(p: &PartialDateTime) -> bool {
    p.tz_hour == Some(0) && p.tz_minute.unwrap_or(0) == 0
}

fn fixed_offset_of(p: &PartialDateTime) -> Option<FixedOffset> {
    let secs = i32::from(p.tz_hour?) * 3600 + i32::from(p.tz_minute.unwrap_or(0)) * 60;
    if p.tz_minus {
        FixedOffset::west_opt(secs)
    } else {
        FixedOffset::east_opt(secs)
    }
}

fn naive_of(p: &PartialDateTime) -> Option<(NaiveDateTime, bool)> {
    let date = NaiveDate::from_ymd_opt(i32::from(p.year?), u32::from(p.month?), u32::from(p.day?))?;
    let has_time = p.hour.is_some();
    let time = NaiveTime::from_hms_opt(
        u32::from(p.hour.unwrap_or(0)),
        u32::from(p.minute.unwrap_or(0)),
        // A leap second (60) folds into the next minute's start.
        u32::from(p.second.unwrap_or(0).min(59)),
    )?;
    Some((date.and_time(time), has_time))
}

#[derive(Clone, Copy, Debug)]
enum Zone {
    Fixed(FixedOffset),
    Named(chrono_tz::Tz),
    Floating(FloatingZone),
}

#[derive(Clone, Copy, Debug)]
struct Instant {
    ts: i64,
    has_time: bool,
    zone: Zone,
}

struct ZoneContext<'a> {
    cal: &'a ICalendar,
    floating: FloatingZone,
}

impl ZoneContext<'_> {
    fn instant(&self, entry: &ICalendarEntry) -> Option<Instant> {
        let p = date_value(entry)?;
        let (naive, has_time) = naive_of(p)?;
        if let Some(offset) = fixed_offset_of(p) {
            if has_time {
                let ts = naive.and_utc().timestamp() - i64::from(offset.local_minus_utc());
                return Some(Instant {
                    ts,
                    has_time,
                    zone: Zone::Fixed(offset),
                });
            }
        }
        let zone = entry
            .tz_id()
            .map(|id| self.zone_for_tzid(id, naive))
            .unwrap_or(Zone::Floating(self.floating));
        Some(Instant {
            ts: local_to_ts(zone, naive),
            has_time,
            zone,
        })
    }

    /// RRULE UNTIL: exact when UTC; otherwise in DTSTART's zone (RFC 5545 §3.3.10 makes a
    /// local UNTIL share DTSTART's form). A `DATE` UNTIL includes the whole day.
    fn until(
        &self,
        until: &PartialDateTime,
        dtstart: Option<&ICalendarEntry>,
        start: Option<Instant>,
    ) -> Option<i64> {
        let (naive, has_time) = naive_of(until)?;
        if has_time {
            if let Some(offset) = fixed_offset_of(until) {
                return Some(naive.and_utc().timestamp() - i64::from(offset.local_minus_utc()));
            }
        }
        let zone = match (start, dtstart.and_then(|e| e.tz_id())) {
            (Some(s), _) => s.zone,
            (None, Some(id)) => self.zone_for_tzid(id, naive),
            (None, None) => Zone::Floating(self.floating),
        };
        if has_time {
            Some(local_to_ts(zone, naive))
        } else {
            let next_day = naive.date().succ_opt()?.and_time(NaiveTime::MIN);
            Some(local_to_ts(zone, next_day) - 1)
        }
    }

    /// DURATION's weeks and days are nominal (RFC 5545 §3.3.6): they move the wall clock,
    /// so a day across a DST change is 23 or 25 hours.
    fn add_duration(
        &self,
        start: Instant,
        d: &calcard::icalendar::ICalendarDuration,
    ) -> Option<i64> {
        let sign = if d.neg { -1 } else { 1 };
        let days = sign * (i64::from(d.weeks) * 7 + i64::from(d.days));
        let exact =
            sign * (i64::from(d.hours) * 3600 + i64::from(d.minutes) * 60 + i64::from(d.seconds));
        let local = ts_to_local(start.zone, start.ts)?;
        let moved = local.checked_add_signed(Duration::days(days))?;
        Some(local_to_ts(start.zone, moved) + exact)
    }

    fn zone_for_tzid(&self, tzid: &str, at: NaiveDateTime) -> Zone {
        let resolver = self.cal.build_tz_resolver();
        match resolver.resolve(tzid) {
            Some(calcard::common::timezone::Tz::Tz(tz)) => return Zone::Named(tz),
            Some(calcard::common::timezone::Tz::Fixed(o)) => return Zone::Fixed(o),
            _ => {}
        }
        if let Ok(tz) = chrono_tz::Tz::from_str(tzid) {
            return Zone::Named(tz);
        }
        match self.custom_offset(tzid, at) {
            Some(o) => Zone::Fixed(o),
            None => Zone::Floating(self.floating),
        }
    }

    /// The offset a custom VTIMEZONE gives at local time `at`: the observance with the most
    /// recent onset, where onsets are each observance's DTSTART and, for the yearly
    /// `BYMONTH`/`BYDAY` rules every real VTIMEZONE uses, that rule's date in `at`'s year.
    fn custom_offset(&self, tzid: &str, at: NaiveDateTime) -> Option<FixedOffset> {
        let vtz = self.cal.timezones().find(|tz| {
            tz.entries.iter().any(|e| {
                e.name == ICalendarProperty::Tzid
                    && e.values.first().and_then(ICalendarValue::as_text) == Some(tzid)
            })
        })?;
        let mut best: Option<(NaiveDateTime, FixedOffset)> = None;
        let mut earliest: Option<(NaiveDateTime, FixedOffset)> = None;
        for id in &vtz.component_ids {
            let obs = self.cal.components.get(*id as usize)?;
            if !matches!(
                obs.component_type,
                ICalendarComponentType::Standard | ICalendarComponentType::Daylight
            ) {
                continue;
            }
            let Some(to) = obs
                .entries
                .iter()
                .find(|e| e.name == ICalendarProperty::Tzoffsetto)
                .and_then(date_value)
                .and_then(fixed_offset_of)
            else {
                continue;
            };
            let Some((start, _)) = obs
                .entries
                .iter()
                .find(|e| e.name == ICalendarProperty::Dtstart)
                .and_then(date_value)
                .and_then(naive_of)
            else {
                continue;
            };
            if earliest.is_none_or(|(t, _)| start < t) {
                earliest = Some((start, to));
            }
            let mut onsets = vec![start];
            if let Some(rule) = obs
                .entries
                .iter()
                .find(|e| e.name == ICalendarProperty::Rrule)
                .and_then(|e| match e.values.first() {
                    Some(ICalendarValue::RecurrenceRule(r)) => Some(r),
                    _ => None,
                })
            {
                if rule.freq == ICalendarFrequency::Yearly {
                    if let (Some(month), Some(day)) = (rule.bymonth.first(), rule.byday.first()) {
                        for year in [at.year() - 1, at.year()] {
                            if let Some(d) = nth_weekday(
                                year,
                                u32::from(month.month()),
                                day.ordwk.unwrap_or(1),
                                weekday(day.weekday),
                            ) {
                                let onset = d.and_time(start.time());
                                if onset >= start {
                                    onsets.push(onset);
                                }
                            }
                        }
                    }
                }
            }
            for onset in onsets {
                if onset <= at && best.is_none_or(|(t, _)| onset > t) {
                    best = Some((onset, to));
                }
            }
        }
        best.or(earliest).map(|(_, o)| o)
    }
}

fn weekday(w: ICalendarWeekday) -> Weekday {
    match w {
        ICalendarWeekday::Sunday => Weekday::Sun,
        ICalendarWeekday::Monday => Weekday::Mon,
        ICalendarWeekday::Tuesday => Weekday::Tue,
        ICalendarWeekday::Wednesday => Weekday::Wed,
        ICalendarWeekday::Thursday => Weekday::Thu,
        ICalendarWeekday::Friday => Weekday::Fri,
        ICalendarWeekday::Saturday => Weekday::Sat,
    }
}

/// The `n`th `wd` of a month; negative `n` counts from the end.
fn nth_weekday(year: i32, month: u32, n: i16, wd: Weekday) -> Option<NaiveDate> {
    if n > 0 {
        NaiveDate::from_weekday_of_month_opt(year, month, wd, u8::try_from(n).ok()?)
    } else {
        let first_next = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1)?
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)?
        };
        let last = first_next.pred_opt()?;
        let back = (7 + last.weekday().num_days_from_monday() - wd.num_days_from_monday()) % 7;
        let last_wd = last - Duration::days(i64::from(back));
        let d = last_wd - Duration::weeks(i64::from(-n - 1));
        (d.month() == month).then_some(d)
    }
}

/// A wall-clock time in `zone` as epoch seconds. An ambiguous time (the repeated hour when
/// clocks go back) takes the earlier instant; a time in the gap when they go forward is
/// read with the offset from before the gap, as `mktime` does, so 02:30 becomes 03:30.
fn local_to_ts(zone: Zone, naive: NaiveDateTime) -> i64 {
    fn pick<Tz: TimeZone>(tz: &Tz, naive: NaiveDateTime) -> i64 {
        match tz.from_local_datetime(&naive) {
            LocalResult::Single(t) => t.timestamp(),
            LocalResult::Ambiguous(a, _) => a.timestamp(),
            LocalResult::None => {
                let before = naive - Duration::hours(3);
                let offset = match tz.from_local_datetime(&before) {
                    LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => {
                        t.offset().fix().local_minus_utc()
                    }
                    LocalResult::None => 0,
                };
                naive.and_utc().timestamp() - i64::from(offset)
            }
        }
    }
    match zone {
        Zone::Fixed(o) => naive.and_utc().timestamp() - i64::from(o.local_minus_utc()),
        Zone::Named(tz) => pick(&tz, naive),
        Zone::Floating(FloatingZone::Zone(tz)) => pick(&tz, naive),
        Zone::Floating(FloatingZone::Local) => pick(&Local, naive),
    }
}

fn ts_to_local(zone: Zone, ts: i64) -> Option<NaiveDateTime> {
    let utc: DateTime<Utc> = DateTime::from_timestamp(ts, 0)?;
    Some(match zone {
        Zone::Fixed(o) => utc.with_timezone(&o).naive_local(),
        Zone::Named(tz) => utc.with_timezone(&tz).naive_local(),
        Zone::Floating(FloatingZone::Zone(tz)) => utc.with_timezone(&tz).naive_local(),
        Zone::Floating(FloatingZone::Local) => utc.with_timezone(&Local).naive_local(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc() -> FloatingZone {
        FloatingZone::Zone(chrono_tz::UTC)
    }

    fn one(ics: &str) -> EventFields {
        let mut v = parse_events_in(ics, utc());
        assert_eq!(v.len(), 1, "{v:?}");
        v.remove(0)
    }

    fn wrap(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//t//EN\r\nBEGIN:VEVENT\r\n{body}END:VEVENT\r\nEND:VCALENDAR\r\n")
    }

    #[test]
    fn open_end_constant_is_dec_31_2036() {
        assert_eq!(
            DateTime::from_timestamp(OPEN_RECURRENCE_END_TS, 0)
                .unwrap()
                .to_rfc3339(),
            "2036-12-31T00:00:00+00:00"
        );
    }

    #[test]
    fn utc_tzid_and_floating_times() {
        let e = one(&wrap(
            "UID:a\r\nDTSTART:20240115T100000Z\r\nDTEND:20240115T110000Z\r\n",
        ));
        assert_eq!(e.start, Some(1_705_312_800));
        assert_eq!(e.recurrence_end, 1_705_316_400);

        let e = one(&wrap(
            "UID:a\r\nDTSTART;TZID=America/New_York:20240715T100000\r\nDTEND;TZID=America/New_York:20240715T110000\r\n",
        ));
        assert_eq!(e.start, Some(1_721_052_000)); // 14:00Z, EDT
        let e = one(&wrap("UID:a\r\nDTSTART:20240115T100000\r\n"));
        assert_eq!(e.start, Some(1_705_312_800));
        assert_eq!(e.recurrence_end, e.recurrence_start);
    }

    #[test]
    fn duration_and_rrule_ends() {
        let e = one(&wrap(
            "UID:a\r\nDTSTART:20240115T100000Z\r\nDURATION:PT90M\r\n",
        ));
        assert_eq!(e.recurrence_end - e.recurrence_start, 5400);

        let e = one(&wrap(
            "UID:a\r\nDTSTART:20240115T100000Z\r\nRRULE:FREQ=WEEKLY;UNTIL=20240301T100000Z\r\n",
        ));
        assert!(e.is_recurring);
        assert_eq!(e.recurrence_end, 1_709_287_200);

        let e = one(&wrap(
            "UID:a\r\nDTSTART:20240115T100000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
        ));
        assert_eq!(e.recurrence_end, OPEN_RECURRENCE_END_TS);

        let e = one(&wrap(
            "UID:a\r\nDTSTART;VALUE=DATE:20240115\r\nRRULE:FREQ=DAILY;UNTIL=20240120\r\n",
        ));
        assert!(e.all_day);
        assert_eq!(e.recurrence_end, 1_705_795_199); // 2024-01-20T23:59:59Z
    }

    #[test]
    fn dst_gap_reads_forward() {
        let e = one(&wrap(
            "UID:a\r\nDTSTART;TZID=Europe/Berlin:20240331T023000\r\n",
        ));
        assert_eq!(e.start, Some(1_711_848_600)); // 01:30Z == 03:30 CEST
    }

    #[test]
    fn participants() {
        let e = one(&wrap(
            "UID:a\r\nDTSTART:20240115T100000Z\r\nORGANIZER;CN=\"Boss, The\":MAILTO:boss@x.com\r\nATTENDEE;ROLE=REQ-PARTICIPANT;CN=Ann:mailto:ann@x.com\r\nATTENDEE:mailto:bob@x.com\r\n",
        ));
        assert_eq!(e.organizer, "Boss, The <boss@x.com>");
        assert_eq!(e.attendees, vec!["Ann <ann@x.com>", "bob@x.com"]);
        assert_eq!(e.search_participants(), "Ann <ann@x.com> bob@x.com");
    }

    #[test]
    fn nth_weekday_rules() {
        assert_eq!(
            nth_weekday(2024, 3, -1, Weekday::Sun),
            NaiveDate::from_ymd_opt(2024, 3, 31)
        );
        assert_eq!(
            nth_weekday(2024, 11, 1, Weekday::Sun),
            NaiveDate::from_ymd_opt(2024, 11, 3)
        );
        assert_eq!(
            nth_weekday(2024, 3, 2, Weekday::Sun),
            NaiveDate::from_ymd_opt(2024, 3, 10)
        );
    }
}
