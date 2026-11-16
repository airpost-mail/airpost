// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The edits the desktop's `ics-event-helpers.ts` made, on a [`Doc`]: one calendar object
//! resource, its VEVENTs (a master and the inline exceptions RFC 4791 section 4.1 keeps in
//! the same resource) and VTIMEZONEs. Each edit changes the properties it is about and
//! leaves every other line as the server wrote it.
//!
//! SEQUENCE is not advanced by the edits; [`bump_sequence`] does it once per save, since a
//! save that runs several edits is one revision (RFC 5546 section 2.1.4).

use crate::ics::{self, Component, Property};
use crate::time::{
    all_day_end, duration_of, end_of, offset_in, resolve_iana, rid_of, set_when, start_of,
    sync_vtimezones, utc_naive, wall_in, zoned, When,
};
use crate::{email_from_participant_uri, Env, Error, Result};
use airpost_dav::Zones;
use chrono::{Duration, NaiveDate};

/// A calendar object being edited. A bare VEVENT is held inside a VCALENDAR and written back
/// bare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub cal: Component,
    bare: bool,
}

/// An occurrence of a series, as RECURRENCE-ID names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rid {
    /// An all-day series' occurrence: its day.
    Date(NaiveDate),
    /// A timed series' occurrence: its original start, epoch seconds.
    At(i64),
}

impl Rid {
    pub fn ts(&self, env: &Env) -> i64 {
        match self {
            Rid::Date(d) => env.day_start(*d),
            Rid::At(t) => *t,
        }
    }

    pub fn is_date(&self) -> bool {
        matches!(self, Rid::Date(_))
    }

    /// The occurrence starting at `ts`, a DATE when `all_day`.
    pub fn of(ts: i64, all_day: bool, env: &Env) -> Rid {
        if all_day {
            Rid::Date(env.local_date(ts))
        } else {
            Rid::At(ts)
        }
    }
}

/// Somebody named on an event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub email: String,
    pub name: Option<String>,
}

/// A guest to invite.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Guest {
    pub email: String,
    pub name: Option<String>,
    pub role: Option<String>,
}

/// What `events.create` builds an event from (`CreateEventOptions`).
#[derive(Clone, Debug, Default)]
pub struct NewEvent {
    pub uid: Option<String>,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// IANA or Windows zone the wall clock is written in; UTC when absent or unknown.
    pub timezone: Option<String>,
    pub organizer: Option<Person>,
    pub attendees: Vec<Guest>,
    pub recurrence_rule: Option<String>,
    /// `OPAQUE` or `TRANSPARENT` (RFC 5545 section 3.8.2.7).
    pub transparency: Option<String>,
}

/// New times for an event (`UpdateTimesOptions`).
#[derive(Clone, Debug, Default)]
pub struct Times {
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub timezone: Option<String>,
}

/// Property edits for one VEVENT; `None` leaves a property alone.
#[derive(Clone, Debug, Default)]
pub struct Edits {
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub attendees: Option<Vec<Guest>>,
    pub organizer: Option<Person>,
    pub transparency: Option<String>,
}

impl Doc {
    pub fn parse(text: &str) -> Result<Doc> {
        let root = ics::parse(text).map_err(|e| Error::invalid_ics(e.0))?;
        match root.name.as_str() {
            "VCALENDAR" => Ok(Doc {
                cal: root,
                bare: false,
            }),
            "VEVENT" => {
                let mut cal = Component::new("VCALENDAR");
                cal.children.push(root);
                Ok(Doc { cal, bare: true })
            }
            other => Err(Error::invalid_ics(format!(
                "not a calendar object: BEGIN:{other}"
            ))),
        }
    }

    /// A VCALENDAR holding `vevent`, with Airpost's PRODID.
    pub fn of_event(vevent: Component) -> Doc {
        let mut cal = Component::new_calendar();
        cal.children.push(vevent);
        Doc { cal, bare: false }
    }

    pub fn is_bare(&self) -> bool {
        self.bare
    }

    pub fn to_ics(&self) -> String {
        if self.bare && self.cal.children.len() == 1 {
            self.cal.children[0].to_ics()
        } else {
            self.cal.to_ics()
        }
    }

    pub fn zones(&self, env: &Env) -> Zones {
        env.zones(&self.cal.to_ics())
    }

    /// Indexes of the VEVENTs in [`Doc::cal`]'s children.
    pub fn vevent_ids(&self) -> Vec<usize> {
        self.cal
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name == "VEVENT")
            .map(|(i, _)| i)
            .collect()
    }

    pub fn vevents(&self) -> impl Iterator<Item = &Component> {
        self.cal.children.iter().filter(|c| c.name == "VEVENT")
    }

    /// The series master: the first VEVENT without a RECURRENCE-ID, else the first VEVENT
    /// (an invitation to one occurrence is its own master).
    pub fn master_id(&self) -> Option<usize> {
        let ids = self.vevent_ids();
        ids.iter()
            .copied()
            .find(|i| !self.cal.children[*i].has("RECURRENCE-ID"))
            .or_else(|| ids.first().copied())
    }

    pub fn master(&self) -> Option<&Component> {
        self.master_id().map(|i| &self.cal.children[i])
    }

    pub(crate) fn master_or_err(&self) -> Result<usize> {
        self.master_id()
            .ok_or_else(|| Error::invalid_ics("Invalid ICS: no VEVENT component found"))
    }

    /// The first VEVENT, as ical.js's `getFirstSubcomponent('vevent')` picks.
    pub fn first(&self) -> Option<&Component> {
        self.vevents().next()
    }

    /// The VEVENT whose RECURRENCE-ID is the instant `ts`.
    pub fn exception_at(&self, ts: i64, zones: &Zones) -> Option<usize> {
        self.vevent_ids()
            .into_iter()
            .find(|i| rid_of(&self.cal.children[*i]).is_some_and(|r| r.ts(zones) == ts))
    }

    /// The VEVENT whose RECURRENCE-ID reads as `text` once `-` and `:` go
    /// (`matchesRecurrenceId`), or failing that names the same instant.
    pub fn exception_named(&self, text: &str, zones: &Zones) -> Option<usize> {
        let wanted = rid_digits(text);
        let by_text = self.vevent_ids().into_iter().find(|i| {
            self.cal.children[*i]
                .value("RECURRENCE-ID")
                .is_some_and(|v| rid_digits(v) == wanted)
        });
        by_text.or_else(|| {
            let at = When::parse(text, None)?;
            match at {
                When::Utc(_) | When::Date(_) => self.exception_at(at.ts(zones), zones),
                When::Local(..) => None,
            }
        })
    }
}

fn rid_digits(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_digit() || *c == 'T' || *c == 'Z')
        .collect()
}

/// A UID with the uniqueness of a UUID (RFC 7986 section 5.3).
pub fn generate_uid() -> String {
    format!("{}@airpost", uuid::Uuid::new_v4())
}

fn validate(start: i64, end: i64) -> Result<()> {
    if start < 0 || end < 0 {
        return Err(Error::invalid("Invalid timestamps: must be positive"));
    }
    if end < start {
        return Err(Error::invalid(
            "Invalid timestamps: end time must be after or equal to start time",
        ));
    }
    Ok(())
}

/// A new event's calendar object (`createICSString`).
pub fn create(ev: &NewEvent, env: &Env) -> Doc {
    let mut cal = Component::new_calendar();
    cal.props.push(Property::new("CALSCALE", "GREGORIAN"));
    let mut vevent = Component::new("VEVENT");
    vevent.props.push(Property::new(
        "UID",
        ev.uid.clone().unwrap_or_else(generate_uid),
    ));
    vevent.set_text("SUMMARY", &ev.summary);

    let zone = ev.timezone.as_deref().filter(|t| resolve_iana(t).is_some());
    match (
        ev.all_day,
        zone,
        ev.timezone.as_deref().and_then(resolve_iana),
    ) {
        (false, Some(id), Some(tz)) => {
            set_when(
                &mut vevent,
                "DTSTART",
                &When::Local(wall_in(tz, ev.start), Some(id.to_owned())),
            );
            set_when(
                &mut vevent,
                "DTEND",
                &When::Local(wall_in(tz, ev.end), Some(id.to_owned())),
            );
        }
        _ => {
            let zones = env.zones("");
            set_when(
                &mut vevent,
                "DTSTART",
                &zoned(ev.start, ev.all_day, None, &zones, env),
            );
            let end = if ev.all_day {
                all_day_end(ev.start, ev.end, env)
            } else {
                zoned(ev.end, false, None, &zones, env)
            };
            set_when(&mut vevent, "DTEND", &end);
        }
    }
    if let Some(d) = ev.description.as_deref().filter(|d| !d.is_empty()) {
        vevent.set_text("DESCRIPTION", d);
    }
    if let Some(l) = ev.location.as_deref().filter(|l| !l.is_empty()) {
        vevent.set_text("LOCATION", l);
    }
    for g in &ev.attendees {
        vevent.props.push(new_attendee(
            &g.email,
            g.name.as_deref(),
            g.role.as_deref().unwrap_or("REQ-PARTICIPANT"),
        ));
    }
    if let Some(rule) = ev.recurrence_rule.as_deref().filter(|r| !r.is_empty()) {
        vevent
            .props
            .push(Property::new("RRULE", normalize_rrule(rule)));
    }
    if let Some(t) = ev.transparency.as_deref().filter(|t| !t.is_empty()) {
        vevent
            .props
            .push(Property::new("TRANSP", t.to_ascii_uppercase()));
    }
    vevent.props.push(env.stamp());
    cal.children.push(vevent);
    if let Some(org) = &ev.organizer {
        name_organizer(&mut cal, org);
    }
    sync_vtimezones(&mut cal, ev.start, None);
    Doc { cal, bare: false }
}

/// An RRULE value as written: `RRULE:` dropped, uppercased.
fn normalize_rrule(rule: &str) -> String {
    let r = rule.trim();
    let r = if r.len() > 6 && r[..6].eq_ignore_ascii_case("RRULE:") {
        &r[6..]
    } else {
        r
    };
    r.to_ascii_uppercase()
}

fn new_attendee(email: &str, name: Option<&str>, role: &str) -> Property {
    let mut p = Property::new("ATTENDEE", format!("mailto:{email}"));
    if let Some(n) = name.filter(|n| !n.is_empty()) {
        p.set_param("CN", n);
    }
    p.set_param("PARTSTAT", "NEEDS-ACTION");
    p.set_param("ROLE", role);
    p.set_param("RSVP", "TRUE");
    p
}

fn sequence_of(vevent: &Component) -> i64 {
    vevent
        .value("SEQUENCE")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Advances SEQUENCE once for a revision the organizer made (absent means zero, RFC 5545
/// section 3.7.4): on the inline exception `rid` names, else on the master.
pub fn bump_sequence(doc: &mut Doc, rid: Option<&str>) -> Result<()> {
    let zones = doc.zones(&Env::new(chrono_tz::UTC, 0));
    let target = match rid {
        Some(r) => doc.exception_named(r, &zones).ok_or_else(|| {
            Error::invalid_ics(format!("No VEVENT to revise for RECURRENCE-ID {r}"))
        })?,
        None => doc
            .master_id()
            .ok_or_else(|| Error::invalid_ics("No VEVENT to revise"))?,
    };
    let v = &mut doc.cal.children[target];
    let next = sequence_of(v) + 1;
    v.set_value("SEQUENCE", next.to_string());
    Ok(())
}

/// Bumps the VEVENT an edit of occurrence `rid` (instant) touched: its exception when there
/// is one, else the master.
pub fn bump_sequence_at(doc: &mut Doc, rid: Option<i64>, env: &Env) -> Result<()> {
    let zones = doc.zones(env);
    let target = match rid.and_then(|r| doc.exception_at(r, &zones)) {
        Some(i) => i,
        None => doc
            .master_id()
            .ok_or_else(|| Error::invalid_ics("No VEVENT to revise"))?,
    };
    let v = &mut doc.cal.children[target];
    let next = sequence_of(v) + 1;
    v.set_value("SEQUENCE", next.to_string());
    Ok(())
}

// Google stores a RECURRENCE-ID sent in UTC back in the event's zone, so the instants compare.
fn same_occurrence(a: &Component, za: &Zones, b: &Component, zb: &Zones) -> bool {
    match (rid_of(a), rid_of(b)) {
        (Some(x), Some(y)) => x.ts(za) == y.ts(zb),
        (None, None) => !a.has("RECURRENCE-ID") && !b.has("RECURRENCE-ID"),
        _ => false,
    }
}

// Order-insensitive, since a server may write the lines of an unchanged VEVENT in another order.
fn content_apart_from_revision(vevent: &Component) -> (Vec<String>, Vec<String>) {
    let mut props: Vec<String> = vevent
        .props
        .iter()
        .filter(|p| !matches!(p.name.as_str(), "SEQUENCE" | "DTSTAMP" | "RECURRENCE-ID"))
        .map(|p| {
            let mut params: Vec<String> = p
                .params
                .iter()
                .map(|q| format!("{}={}", q.name, q.values.join(",")))
                .collect();
            params.sort();
            format!("{};{}:{}", p.name, params.join(";"), p.value)
        })
        .collect();
    props.sort();
    let children = vevent.children.iter().map(Component::to_ics).collect();
    (props, children)
}

/// Re-sends an older copy (undo, redo) over `current`, the copy last written
/// (`matchEventSequence`): a VEVENT that differs from its counterpart takes the higher of the
/// two SEQUENCEs and a fresh DTSTAMP, since Google refuses a lower SEQUENCE with 409 but
/// accepts a tie; one that matches apart from SEQUENCE and DTSTAMP takes `current`'s, so
/// guests see no revision of an occurrence nobody changed.
pub fn match_sequence(doc: &mut Doc, current: &Doc, env: &Env) {
    let zd = doc.zones(env);
    let zc = current.zones(env);
    for i in doc.vevent_ids() {
        let counterpart = current
            .vevents()
            .find(|c| same_occurrence(c, &zc, &doc.cal.children[i], &zd));
        let v = &mut doc.cal.children[i];
        match counterpart {
            Some(c) if content_apart_from_revision(c) == content_apart_from_revision(v) => {
                for name in ["SEQUENCE", "DTSTAMP"] {
                    v.remove_all(name);
                    if let Some(p) = c.prop(name) {
                        v.props.push(p.clone());
                    }
                }
            }
            other => {
                let seq = sequence_of(v).max(other.map(sequence_of).unwrap_or(0));
                v.set_value("SEQUENCE", seq.to_string());
                v.replace(env.stamp());
            }
        }
    }
}

/// Google keeps an override that a PUT leaves out, so re-sending an older copy would not
/// revert an occurrence changed since (`revertAddedOverrides`). Each override in `current`
/// that `before` lacks goes back in at its own slot with the master's content; a slot the
/// series no longer has is skipped.
pub fn revert_added_overrides(before: &Doc, current: &Doc, env: &Env) -> Result<Doc> {
    let Some(master) = before.vevents().find(|v| !v.has("RECURRENCE-ID")) else {
        return Ok(before.clone());
    };
    let duration = duration_of(master);
    let zb = before.zones(env);
    let zc = current.zones(env);
    let series_start = start_of(master).map(|s| s.ts(&zb)).unwrap_or(0);
    let before_text = before.to_ics();
    let series_has = |slot: i64| {
        crate::expand::expand_text(&before_text, series_start, slot, slot + 1, env)
            .map(|occs| {
                occs.iter()
                    .any(|o| o.recurrence_id.unwrap_or(o.start) == slot)
            })
            .unwrap_or(false)
    };

    let mut reverted = before.clone();
    for ov in current.vevents() {
        let Some(slot) = rid_of(ov) else { continue };
        if before.vevents().any(|v| same_occurrence(v, &zb, ov, &zc)) {
            continue;
        }
        let at = slot.ts(&zc);
        if !series_has(at) {
            continue;
        }
        let end = slot.add_wall(duration).ts(&zc);
        create_exception(&mut reverted, at, at, end, slot.is_date(), env)?;
    }
    Ok(reverted)
}

/// New start and end for the master (`updateEventTimes`). Given an IANA or Windows
/// `timezone`, a timed event is written in that zone's wall clock and the series' anchors
/// (EXDATE, RDATE, UNTIL, RECURRENCE-IDs) move with it; otherwise each time keeps the zone
/// it had, or goes to UTC when it had none.
pub fn update_times(doc: &mut Doc, t: &Times, env: &Env) -> Result<()> {
    let mi = doc.master_or_err()?;
    update_times_at(doc, mi, t, true, env)
}

/// [`update_times`] on the VEVENT at child index `i`; `realign` moves the series' anchors
/// when the zone changes, which only a master has.
pub fn update_times_at(doc: &mut Doc, i: usize, t: &Times, realign: bool, env: &Env) -> Result<()> {
    validate(t.start, t.end)?;
    let zones = doc.zones(env);
    let update_zone = t
        .timezone
        .as_deref()
        .and_then(|id| resolve_iana(id).map(|tz| (id.to_owned(), tz)));
    match (t.all_day, update_zone) {
        (false, Some((id, tz))) => {
            let previous = start_of(&doc.cal.children[i]);
            let v = &mut doc.cal.children[i];
            set_when(
                v,
                "DTSTART",
                &When::Local(wall_in(tz, t.start), Some(id.clone())),
            );
            v.remove_all("DURATION");
            set_when(
                v,
                "DTEND",
                &When::Local(wall_in(tz, t.end), Some(id.clone())),
            );
            if let Some(prev) = previous.filter(|p| realign && !p.is_date()) {
                realign_anchors(doc, i, &prev, &id, tz, &zones, env);
            }
        }
        _ => {
            let v = &mut doc.cal.children[i];
            let start_zone = v
                .prop("DTSTART")
                .and_then(|p| p.param("TZID"))
                .map(str::to_owned);
            let end_zone = v
                .prop("DTEND")
                .and_then(|p| p.param("TZID"))
                .map(str::to_owned)
                .or_else(|| start_zone.clone());
            set_when(
                v,
                "DTSTART",
                &zoned(t.start, t.all_day, start_zone.as_deref(), &zones, env),
            );
            let end = if t.all_day {
                all_day_end(t.start, t.end, env)
            } else {
                zoned(t.end, false, end_zone.as_deref(), &zones, env)
            };
            v.remove_all("DURATION");
            set_when(v, "DTEND", &end);
        }
    }
    doc.cal.children[i].replace(env.stamp());
    sync_vtimezones(&mut doc.cal, t.start, None);
    Ok(())
}

/// Moves each date-time that pins an occurrence (EXDATE, RDATE, RRULE UNTIL, every
/// RECURRENCE-ID) by the wall-clock difference between the old zone and the new one, so it
/// still names an instance of the rezoned rule (`realignOccurrenceAnchors`).
fn realign_anchors(
    doc: &mut Doc,
    mi: usize,
    previous: &When,
    tzid: &str,
    tz: chrono_tz::Tz,
    zones: &Zones,
    env: &Env,
) {
    let prev_instant = previous.ts(zones);
    let prev_tzid = previous.tzid().map(str::to_owned);
    let prev_offset = (previous.wall() - utc_naive(prev_instant)).num_seconds();
    let wall_delta = offset_in(tz, prev_instant) - prev_offset;
    let in_old_zone = |w: &When| -> chrono::NaiveDateTime {
        match (&prev_tzid, previous) {
            (_, When::Utc(_)) => utc_naive(w.ts(zones)),
            (Some(_), _) | (None, _) => zones.to_local(prev_tzid.as_deref(), w.ts(zones)),
        }
    };
    let realigned = |w: &When, zoned: bool| -> When {
        if w.is_date() {
            return w.clone();
        }
        let wall = in_old_zone(w) + Duration::seconds(wall_delta);
        if zoned {
            When::Local(wall, Some(tzid.to_owned()))
        } else {
            When::Utc(utc_naive(
                Zones::new("", airpost_dav::FloatingZone::Zone(tz)).to_utc(None, wall),
            ))
        }
    };
    let _ = env;

    let master = &mut doc.cal.children[mi];
    for p in master
        .props
        .iter_mut()
        .filter(|p| p.name == "EXDATE" || p.name == "RDATE")
    {
        let zoned = p.has_param("TZID");
        let values = When::all_of(p);
        if values.is_empty() || values.len() != p.values().len() {
            continue;
        }
        let moved: Vec<When> = values.iter().map(|w| realigned(w, zoned)).collect();
        p.value = moved.iter().map(When::text).collect::<Vec<_>>().join(",");
        if zoned {
            p.set_param("TZID", tzid);
        }
    }
    if let Some(rule) = master.prop_mut("RRULE") {
        if let Some(until) = rrule_part(&rule.value, "UNTIL").and_then(|u| When::parse(&u, None)) {
            let until = match until {
                When::Local(n, None) => When::Local(n, prev_tzid.clone()),
                w => w,
            };
            if !until.is_date() {
                let moved = realigned(&until, false);
                rule.value = set_rrule_part(&rule.value, "UNTIL", &moved.text());
            }
        }
    }
    for i in doc.vevent_ids() {
        let v = &mut doc.cal.children[i];
        let Some(p) = v.prop("RECURRENCE-ID").cloned() else {
            continue;
        };
        let zoned = p.has_param("TZID");
        let Some(w) = When::of(&p) else { continue };
        let moved = realigned(&w, zoned);
        set_when(v, "RECURRENCE-ID", &moved);
    }
}

/// One `KEY=value` part of an RRULE.
pub fn rrule_part(rule: &str, key: &str) -> Option<String> {
    rule.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(key)
            .then(|| v.trim().to_owned())
    })
}

/// The RRULE with part `key` set to `value` (added at the end when absent).
pub fn set_rrule_part(rule: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut parts: Vec<String> = rule
        .split(';')
        .filter(|p| !p.is_empty())
        .map(|part| match part.split_once('=') {
            Some((k, _)) if k.trim().eq_ignore_ascii_case(key) => {
                found = true;
                format!("{}={value}", key.to_ascii_uppercase())
            }
            _ => part.to_owned(),
        })
        .collect();
    if !found {
        parts.push(format!("{}={value}", key.to_ascii_uppercase()));
    }
    parts.join(";")
}

/// The RRULE without part `key`.
pub fn remove_rrule_part(rule: &str, key: &str) -> String {
    rule.split(';')
        .filter(|part| {
            !part
                .split_once('=')
                .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Embeds an exception for the occurrence that started at `original` inline in the
/// master's VCALENDAR (`createRecurrenceException`): a copy of the master without RRULE,
/// RDATE and EXDATE, RECURRENCE-ID in UTC (a DATE for an all-day series), at the new times.
/// An exception already at that slot is replaced. Returns the RECURRENCE-ID text.
pub fn create_exception(
    doc: &mut Doc,
    original: i64,
    new_start: i64,
    new_end: i64,
    all_day: bool,
    env: &Env,
) -> Result<String> {
    validate(new_start, new_end)?;
    let zones = doc.zones(env);
    let mi = doc.master_or_err()?;
    let rid = if all_day {
        When::Date(env.local_date(original))
    } else {
        When::Utc(utc_naive(original))
    };
    let rid_text = rid.text();

    if !doc.bare {
        if let Some(existing) = doc.exception_at(original, &zones) {
            doc.cal.children.remove(existing);
        }
    }
    let mi = if doc
        .cal
        .children
        .get(mi)
        .is_some_and(|c| !c.has("RECURRENCE-ID"))
    {
        mi
    } else {
        doc.master_or_err()?
    };

    let master = &doc.cal.children[mi];
    let start_zone = master
        .prop("DTSTART")
        .and_then(|p| p.param("TZID"))
        .map(str::to_owned);
    let mut exception = master.clone();
    for name in ["RRULE", "RDATE", "EXDATE"] {
        exception.remove_all(name);
    }
    set_when(&mut exception, "RECURRENCE-ID", &rid);
    set_when(
        &mut exception,
        "DTSTART",
        &zoned(new_start, all_day, start_zone.as_deref(), &zones, env),
    );
    let end = if all_day {
        all_day_end(new_start, new_end, env)
    } else {
        zoned(new_end, false, start_zone.as_deref(), &zones, env)
    };
    exception.remove_all("DURATION");
    set_when(&mut exception, "DTEND", &end);
    exception.replace(env.stamp());
    doc.cal.children[mi].replace(env.stamp());
    if !doc.bare {
        doc.cal.children.push(exception);
    }
    Ok(rid_text)
}

/// Applies property edits to the inline exception `rid` names (`applyEditsToException`).
pub fn apply_edits_to_exception(doc: &mut Doc, rid: &str, edits: &Edits, env: &Env) -> Result<()> {
    if doc.bare {
        return Err(Error::invalid_ics("Invalid ICS: expected VCALENDAR root"));
    }
    let zones = doc.zones(env);
    let i = doc.exception_named(rid, &zones).ok_or_else(|| {
        Error::invalid_ics(format!(
            "No exception VEVENT found with RECURRENCE-ID matching {rid}"
        ))
    })?;
    apply_edits_at(doc, i, edits, env);
    Ok(())
}

/// Applies property edits to the VEVENT at child index `i`.
pub fn apply_edits_at(doc: &mut Doc, i: usize, edits: &Edits, env: &Env) {
    let v = &mut doc.cal.children[i];
    if let Some(s) = &edits.summary {
        v.set_text("SUMMARY", s);
    }
    if let Some(d) = &edits.description {
        v.set_text("DESCRIPTION", d);
    }
    if let Some(l) = &edits.location {
        v.set_text("LOCATION", l);
    }
    if let Some(a) = &edits.attendees {
        reconcile_attendees(v, a);
    }
    if let Some(t) = edits.transparency.as_deref().filter(|t| !t.is_empty()) {
        v.set_value("TRANSP", t.to_ascii_uppercase());
    }
    v.replace(env.stamp());
    if let Some(org) = &edits.organizer {
        name_organizer(&mut doc.cal, org);
    }
}

/// Moves each inline exception's RECURRENCE-ID, and the master's EXDATEs, by `delta`
/// seconds after the master moved by it (`shiftInlineExceptions`): whole days for a DATE, a
/// plain offset otherwise, a zoned value staying in its zone. Exception DTSTART and DTEND
/// stay where the user put them.
pub fn shift_inline_exceptions(doc: &mut Doc, delta: i64, env: &Env) {
    if delta == 0 || doc.bare {
        return;
    }
    let zones = doc.zones(env);
    let shift = |w: &When| -> When {
        match w {
            When::Date(d) => {
                let days = (delta as f64 / 86400.0).round() as i64;
                let start = env.day_start(*d);
                When::Date(env.local_date(start) + Duration::days(days))
            }
            other => {
                let shifted = other.ts(&zones) + delta;
                zoned(shifted, false, other.tzid(), &zones, env)
            }
        }
    };
    for i in doc.vevent_ids() {
        let v = &mut doc.cal.children[i];
        match v.prop("RECURRENCE-ID").cloned() {
            None => {
                let mut touched = false;
                for p in v.props.iter_mut().filter(|p| p.name == "EXDATE") {
                    touched = true;
                    let values = When::all_of(p);
                    if values.is_empty() || values.len() != p.values().len() {
                        continue;
                    }
                    let moved: Vec<When> = values.iter().map(shift).collect();
                    p.value = moved.iter().map(When::text).collect::<Vec<_>>().join(",");
                    match moved.first() {
                        Some(When::Utc(_)) => p.remove_param("TZID"),
                        Some(When::Local(_, Some(id))) => p.set_param("TZID", id),
                        _ => {}
                    }
                }
                if touched {
                    v.replace(env.stamp());
                }
            }
            Some(p) => {
                let Some(w) = When::of(&p) else { continue };
                set_when(v, "RECURRENCE-ID", &shift(&w));
                v.replace(env.stamp());
            }
        }
    }
}

/// Moves the whole series by the move of one occurrence (`updateRecurringEventTimes`):
/// the master's start shifts by the occurrence's delta (whole days for an all-day series)
/// and takes the occurrence's new length.
pub fn update_recurring_times(
    doc: &mut Doc,
    original: i64,
    new_start: i64,
    new_end: i64,
    all_day: bool,
    timezone: Option<&str>,
    env: &Env,
) -> Result<()> {
    let zones = doc.zones(env);
    let mi = doc.master_or_err()?;
    let current = start_of(&doc.cal.children[mi])
        .map(|s| s.ts(&zones))
        .ok_or_else(|| Error::invalid_ics("Invalid ICS: the event has no DTSTART"))?;
    if all_day {
        let days = (env.local_date(new_start) - env.local_date(original)).num_days();
        let span = (env.local_date(new_end) - env.local_date(new_start)).num_days();
        let master_start = env.day_start(env.local_date(current) + Duration::days(days));
        let master_end = env.day_start(env.local_date(master_start) + Duration::days(span));
        return update_times(
            doc,
            &Times {
                start: master_start,
                end: master_end,
                all_day,
                timezone: None,
            },
            env,
        );
    }
    let master_start = current + (new_start - original);
    update_times(
        doc,
        &Times {
            start: master_start,
            end: master_start + (new_end - new_start),
            all_day,
            timezone: timezone.map(str::to_owned),
        },
        env,
    )
}

/// Whether the first VEVENT repeats (RRULE or RDATE).
pub fn is_recurring(doc: &Doc) -> bool {
    doc.first()
        .is_some_and(|v| v.has("RRULE") || v.has("RDATE"))
}

/// The first VEVENT's RRULE and its FREQ, when it has one.
pub fn recurrence_info(doc: &Doc) -> Option<(String, String)> {
    let rule = doc.first()?.value("RRULE")?.to_owned();
    let freq = rrule_part(&rule, "FREQ")
        .unwrap_or_default()
        .to_ascii_uppercase();
    Some((rule, freq))
}

/// Excludes the occurrence starting at `occurrence` (`addExclusionDate`), in the zone of
/// the master's DTSTART.
pub fn add_exclusion_date(doc: &mut Doc, occurrence: i64, all_day: bool, env: &Env) -> Result<()> {
    let zones = doc.zones(env);
    let mi = doc.master_or_err()?;
    let v = &mut doc.cal.children[mi];
    let zone = v
        .prop("DTSTART")
        .and_then(|p| p.param("TZID"))
        .map(str::to_owned);
    let ex = zoned(occurrence, all_day, zone.as_deref(), &zones, env);
    v.props.push(ex.property("EXDATE"));
    v.replace(env.stamp());
    Ok(())
}

/// Cancels the occurrence an inline exception overrides (`removeInlineException`): the
/// VEVENT goes and its slot is excluded on the master, with the RECURRENCE-ID's own value
/// and zone. False when no exception matches.
pub fn remove_inline_exception(doc: &mut Doc, rid: &str, env: &Env) -> bool {
    if doc.bare {
        return false;
    }
    let zones = doc.zones(env);
    let wanted: String = rid.chars().filter(|c| *c != '-' && *c != ':').collect();
    let wanted_ts = match When::parse(&wanted, None) {
        Some(w @ When::Utc(_)) => Some(w.ts(&zones)),
        _ => None,
    };
    let mut master = None;
    let mut exception = None;
    for i in doc.vevent_ids() {
        let v = &doc.cal.children[i];
        let Some(p) = v.prop("RECURRENCE-ID") else {
            if master.is_none() {
                master = Some(i);
            }
            continue;
        };
        let Some(w) = When::of(p) else { continue };
        if w.text() == wanted || wanted_ts.is_some_and(|t| w.ts(&zones) == t) {
            exception = Some((i, w));
        }
    }
    let (Some(mi), Some((ei, slot))) = (master, exception) else {
        return false;
    };
    doc.cal.children.remove(ei);
    let mi = if ei < mi { mi - 1 } else { mi };
    let v = &mut doc.cal.children[mi];
    v.props.push(slot.property("EXDATE"));
    v.replace(env.stamp());
    true
}

/// Sets or removes the RRULE (`updateRecurrenceRule`); removing it drops EXDATE and RDATE,
/// which mean nothing without one.
pub fn update_recurrence_rule(doc: &mut Doc, rule: Option<&str>, env: &Env) -> Result<()> {
    let mi = doc.master_or_err()?;
    let v = &mut doc.cal.children[mi];
    v.remove_all("RRULE");
    match rule.filter(|r| !r.trim().is_empty()) {
        Some(r) => v.props.push(Property::new("RRULE", normalize_rrule(r))),
        None => {
            v.remove_all("EXDATE");
            v.remove_all("RDATE");
        }
    }
    v.replace(env.stamp());
    Ok(())
}

/// Sets the guest list to `attendees`. Guests already there are left exactly as they are, so
/// parameters the editor never sees (CUTYPE=RESOURCE, ROLE=CHAIR, DELEGATED-TO) survive.
fn reconcile_attendees(vevent: &mut Component, attendees: &[Guest]) {
    let wanted: Vec<String> = attendees.iter().map(|a| a.email.to_lowercase()).collect();
    let existing: Vec<Option<String>> = vevent
        .props
        .iter()
        .filter(|p| p.name == "ATTENDEE")
        .map(|p| email_from_participant_uri(&p.value))
        .collect();
    vevent.props.retain(|p| {
        p.name != "ATTENDEE"
            || email_from_participant_uri(&p.value).is_some_and(|e| wanted.contains(&e))
    });
    for a in attendees {
        let email = a.email.to_lowercase();
        if existing
            .iter()
            .any(|e| e.as_deref() == Some(email.as_str()))
        {
            continue;
        }
        vevent
            .props
            .push(new_attendee(&a.email, a.name.as_deref(), "REQ-PARTICIPANT"));
    }
}

/// With guests a VEVENT must name its ORGANIZER (RFC 5545 section 3.8.4.3), and a CalDAV
/// server sends invitations only for an organizer it hosts (RFC 6638 section 3.2.1); one
/// already named is kept. The organizer is also an accepted attendee, as Google writes it.
fn name_organizer(cal: &mut Component, organizer: &Person) {
    let ids: Vec<usize> = cal
        .children
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name == "VEVENT")
        .map(|(i, _)| i)
        .collect();
    let with_guests: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|i| cal.children[*i].has("ATTENDEE"))
        .collect();
    if with_guests.is_empty() || ids.iter().any(|i| cal.children[*i].has("ORGANIZER")) {
        return;
    }
    for i in &ids {
        let mut p = Property::new("ORGANIZER", format!("mailto:{}", organizer.email));
        if let Some(n) = organizer.name.as_deref().filter(|n| !n.is_empty()) {
            p.set_param("CN", n);
        }
        cal.children[*i].props.push(p);
    }
    let email = organizer.email.to_lowercase();
    for i in with_guests {
        let v = &mut cal.children[i];
        let pos = v.props.iter().position(|p| {
            p.name == "ATTENDEE" && email_from_participant_uri(&p.value).as_deref() == Some(&email)
        });
        let pos = match pos {
            Some(p) => p,
            None => {
                let mut p = Property::new("ATTENDEE", format!("mailto:{}", organizer.email));
                if let Some(n) = organizer.name.as_deref().filter(|n| !n.is_empty()) {
                    p.set_param("CN", n);
                }
                v.props.push(p);
                v.props.len() - 1
            }
        };
        v.props[pos].set_param("PARTSTAT", "ACCEPTED");
        v.props[pos].remove_param("RSVP");
    }
}

/// Sets the master's guest list (`updateAttendees`).
pub fn update_attendees(
    doc: &mut Doc,
    attendees: &[Guest],
    organizer: Option<&Person>,
    env: &Env,
) -> Result<()> {
    let mi = doc.master_or_err()?;
    reconcile_attendees(&mut doc.cal.children[mi], attendees);
    if let Some(org) = organizer {
        name_organizer(&mut doc.cal, org);
    }
    doc.cal.children[mi].replace(env.stamp());
    Ok(())
}

/// Drops the iTIP METHOD so a scheduling message can be stored (RFC 4791 section 4.1).
pub fn strip_method(doc: &mut Doc) {
    doc.cal.remove_all("METHOD");
}

/// Sets a text property on the master, or TRANSP (`updateEventProperty`).
pub fn update_property(doc: &mut Doc, name: &str, value: &str, env: &Env) -> Result<()> {
    let mi = doc.master_or_err()?;
    let v = &mut doc.cal.children[mi];
    match name.to_ascii_lowercase().as_str() {
        "summary" => v.set_text("SUMMARY", value),
        "description" => v.set_text("DESCRIPTION", value),
        "location" => v.set_text("LOCATION", value),
        "transp" => v.set_value("TRANSP", value),
        other => return Err(Error::invalid(format!("not an editable property: {other}"))),
    }
    v.replace(env.stamp());
    Ok(())
}

/// The TZID of the master's DTSTART when it names a zone (not UTC or floating).
pub fn event_timezone(doc: &Doc, env: &Env) -> Option<String> {
    let zones = doc.zones(env);
    let id = doc.first()?.prop("DTSTART")?.param("TZID")?;
    (!crate::time::is_utc_alias(id) && zones.knows(id)).then(|| id.to_owned())
}

/// Writes RECURRENCE-ID for the occurrence `rid` in the form the series writes DTSTART
/// (RFC 5545 section 3.8.4.4): a DATE for an all-day series, local time with the same TZID
/// for a zoned one, UTC otherwise (`nameOccurrence`).
fn name_occurrence(
    vevent: &mut Component,
    dtstart: &Property,
    rid: &Rid,
    zones: &Zones,
    env: &Env,
) {
    let Some(start) = When::of(dtstart) else {
        return;
    };
    let tzid = dtstart.param("TZID").map(str::to_owned);
    let zoned_tz =
        tzid.filter(|id| !start.is_date() && !crate::time::is_utc_alias(id) && zones.knows(id));
    let value = if start.is_date() {
        match rid {
            Rid::Date(d) => When::Date(*d),
            Rid::At(t) => When::Date(env.local_date(*t)),
        }
    } else if let Some(id) = &zoned_tz {
        When::Local(zones.to_local(Some(id), rid.ts(env)), Some(id.clone()))
    } else {
        When::Utc(utc_naive(rid.ts(env)))
    };
    vevent.remove_all("RECURRENCE-ID");
    vevent.props.push(value.property("RECURRENCE-ID"));
}

/// The iTIP COUNTER proposing another time (RFC 5546 section 3.2.7): our ATTENDEE line alone
/// at PARTSTAT=TENTATIVE, the one occurrence `rid` names, no recurrence. `None` when `email`
/// is not invited.
pub fn counter_proposal(
    doc: &Doc,
    email: &str,
    start: i64,
    end: i64,
    comment: Option<&str>,
    rid: Option<&Rid>,
    env: &Env,
) -> Result<Option<String>> {
    let zones = doc.zones(env);
    let master = doc.vevents().find(|v| !v.has("RECURRENCE-ID"));
    let exception = rid.and_then(|r| {
        let at = r.ts(env);
        doc.vevents()
            .find(|v| rid_of(v).is_some_and(|w| w.ts(&zones) == at))
    });
    let source = exception
        .or(master)
        .or_else(|| doc.first())
        .ok_or_else(|| Error::invalid_ics("Invalid ICS: no VEVENT component found"))?;
    if source.has("RRULE") && rid.is_none() {
        return Err(Error::invalid(
            "A counter-proposal for a series must name the occurrence it counters",
        ));
    }
    let target = email.to_lowercase();
    let Some(mine) = source
        .props_named("ATTENDEE")
        .find(|p| email_from_participant_uri(&p.value).as_deref() == Some(&target))
        .cloned()
    else {
        return Ok(None);
    };

    let mut proposed = source.clone();
    for name in ["RRULE", "RDATE", "EXDATE", "ATTENDEE"] {
        proposed.remove_all(name);
    }
    if let (Some(r), None) = (rid, exception) {
        if let Some(dtstart) = source.prop("DTSTART") {
            name_occurrence(&mut proposed, dtstart, r, &zones, env);
        }
    }
    let mut attendee = mine;
    attendee.set_param("PARTSTAT", "TENTATIVE");
    attendee.remove_param("RSVP");
    proposed.props.push(attendee);

    let is_date = start_of(source).is_some_and(|s| s.is_date());
    let (s, e) = if is_date {
        (
            When::Date(env.local_date(start)),
            When::Date(env.local_date(end)),
        )
    } else {
        (When::Utc(utc_naive(start)), When::Utc(utc_naive(end)))
    };
    set_when(&mut proposed, "DTSTART", &s);
    proposed.remove_all("DURATION");
    set_when(&mut proposed, "DTEND", &e);
    proposed.replace(env.stamp());
    if let Some(c) = comment.filter(|c| !c.is_empty()) {
        proposed.set_text("COMMENT", c);
    }

    let mut counter = Component::new_calendar();
    counter.props.push(Property::new("CALSCALE", "GREGORIAN"));
    counter.props.push(Property::new("METHOD", "COUNTER"));
    counter.children.push(proposed);
    Ok(Some(counter.to_ics()))
}

/// A VCALENDAR holding just the VEVENT for one occurrence (`occurrenceIcs`): its exception
/// where the series has one, else the occurrence cut from the master at its slot. What a
/// REPLY for one occurrence is built from (RFC 5546 section 3.2.3).
pub fn occurrence_ics(doc: &Doc, rid: &Rid, env: &Env) -> Result<Doc> {
    let zones = doc.zones(env);
    let named = rid.ts(env);
    let vevent = match doc
        .vevents()
        .find(|v| rid_of(v).is_some_and(|w| w.ts(&zones) == named))
    {
        Some(v) => v.clone(),
        None => {
            let master = doc
                .first()
                .ok_or_else(|| Error::invalid_ics("Invalid ICS: no VEVENT component found"))?;
            let duration = duration_of(master);
            let mut v = master.clone();
            for name in ["RRULE", "RDATE", "EXDATE"] {
                v.remove_all(name);
            }
            if let Some(dtstart) = master.prop("DTSTART") {
                name_occurrence(&mut v, dtstart, rid, &zones, env);
            }
            if let Some(start) = rid_of(&v) {
                set_when(&mut v, "DTSTART", &start);
                v.remove_all("DURATION");
                set_when(&mut v, "DTEND", &start.add_wall(duration));
            }
            v
        }
    };
    let mut out = Doc::of_event(vevent);
    sync_vtimezones(&mut out.cal, named, Some(&doc.cal));
    Ok(out)
}

/// Sets one attendee's PARTSTAT on a VEVENT, every other parameter and attendee left as they
/// are (RFC 6638 section 3.2.5); RSVP goes, the answer having been given. Only a changed
/// VEVENT gets a new DTSTAMP, which breaks ties at equal SEQUENCE (RFC 5546 section 3.2).
fn set_attendee_partstat(vevent: &mut Component, email: &str, partstat: &str, env: &Env) -> bool {
    let mut changed = false;
    for p in vevent.props.iter_mut().filter(|p| p.name == "ATTENDEE") {
        if email_from_participant_uri(&p.value).as_deref() != Some(email) {
            continue;
        }
        p.set_param("PARTSTAT", partstat);
        p.remove_param("RSVP");
        changed = true;
    }
    if changed {
        vevent.replace(env.stamp());
    }
    changed
}

/// Our answer in every VEVENT: to a whole series or a single event. False when `email` is
/// not an attendee.
pub fn update_attendee_status(
    doc: &mut Doc,
    email: &str,
    partstat: &str,
    env: &Env,
) -> Result<bool> {
    let ids = doc.vevent_ids();
    if ids.is_empty() {
        return Err(Error::invalid_ics("Invalid ICS: no VEVENT component found"));
    }
    let target = email.to_lowercase();
    let mut matched = false;
    for i in ids {
        if set_attendee_partstat(&mut doc.cal.children[i], &target, partstat, env) {
            matched = true;
        }
    }
    Ok(matched)
}

/// Our answer to one occurrence on our stored copy (`updateOccurrenceAttendeeStatus`): on
/// the VEVENT that occurrence's RECURRENCE-ID names, or, when the copy has none, the
/// invitation's VEVENT stored as its exception. False when `email` is not invited to it.
pub fn update_occurrence_attendee_status(
    cal: &mut Doc,
    occurrence: &Doc,
    email: &str,
    partstat: &str,
    env: &Env,
) -> Result<bool> {
    let invited = occurrence
        .first()
        .ok_or_else(|| Error::invalid_ics("Invalid ICS: no VEVENT component found"))?;
    let rid = rid_of(invited).ok_or_else(|| {
        Error::invalid("The invitation names no occurrence: it has no RECURRENCE-ID")
    })?;
    if cal.bare {
        return Err(Error::invalid_ics(
            "Invalid ICS: a stored event must be a VCALENDAR",
        ));
    }
    let target = email.to_lowercase();
    let named = rid.ts(&occurrence.zones(env));
    let zones = cal.zones(env);
    if let Some(i) = cal.exception_at(named, &zones) {
        return Ok(set_attendee_partstat(
            &mut cal.cal.children[i],
            &target,
            partstat,
            env,
        ));
    }
    let mut exception = invited.clone();
    if !set_attendee_partstat(&mut exception, &target, partstat, env) {
        return Ok(false);
    }
    cal.cal.children.push(exception);
    sync_vtimezones(&mut cal.cal, named, Some(&occurrence.cal));
    Ok(true)
}

/// The iTIP REPLY to an invitation (RFC 5546 section 3.2.3, `EventRSVPTask.forReplying`):
/// METHOD:REPLY, and in every VEVENT our ATTENDEE alone, at `partstat`.
pub fn reply(doc: &Doc, email: &str, partstat: &str) -> String {
    let mut out = doc.clone();
    out.cal.set_value("METHOD", "REPLY");
    let me = email.to_lowercase();
    for i in out.vevent_ids() {
        let v = &mut out.cal.children[i];
        v.props.retain(|p| {
            p.name != "ATTENDEE" || email_from_participant_uri(&p.value).as_deref() == Some(&me)
        });
        for p in v.props.iter_mut().filter(|p| p.name == "ATTENDEE") {
            p.set_param("PARTSTAT", partstat);
            p.remove_param("RSVP");
        }
    }
    out.cal.to_ics()
}

/// Our address on the first VEVENT's guest list: the first attendee `is_mine` accepts.
pub fn self_attendee(doc: &Doc, is_mine: impl Fn(&str) -> bool) -> Option<String> {
    doc.first()?
        .props_named("ATTENDEE")
        .filter_map(|p| email_from_participant_uri(&p.value))
        .find(|e| is_mine(e))
}

/// The first VEVENT's ORGANIZER address.
pub fn organizer_email(doc: &Doc) -> Option<String> {
    email_from_participant_uri(doc.first()?.value("ORGANIZER")?)
}

/// The first VEVENT's start and end instants and whether it is all-day.
pub fn span_of(doc: &Doc, env: &Env) -> Option<(i64, i64, bool)> {
    let zones = doc.zones(env);
    let v = doc.first()?;
    let start = start_of(v)?;
    let end = end_of(v)?;
    Some((start.ts(&zones), end.ts(&zones), start.is_date()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env::new(chrono_tz::America::Chicago, 1_767_225_600)
    }

    const WEEKLY: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//t//EN\r\nBEGIN:VEVENT\r\nUID:w@t\r\nDTSTART;TZID=America/Chicago:20260302T090000\r\nDTEND;TZID=America/Chicago:20260302T100000\r\nRRULE:FREQ=WEEKLY\r\nSUMMARY:Standup\r\nDTSTAMP:20260101T000000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn exception_round_trip() {
        let env = env();
        let mut doc = Doc::parse(WEEKLY).unwrap();
        // 2026-03-09 09:00 CDT = 14:00Z
        let slot = 1_773_064_800;
        let rid = create_exception(&mut doc, slot, slot + 3600, slot + 7200, false, &env).unwrap();
        assert_eq!(rid, "20260309T140000Z");
        bump_sequence(&mut doc, Some(&rid)).unwrap();
        let ex = &doc.cal.children[1];
        assert_eq!(ex.value("SEQUENCE"), Some("1"));
        assert!(!ex.has("RRULE"));
        assert_eq!(ex.value("DTSTART"), Some("20260309T100000"));
        assert!(remove_inline_exception(&mut doc, &rid, &env));
        assert_eq!(doc.vevents().count(), 1);
        assert_eq!(
            doc.master().unwrap().value("EXDATE"),
            Some("20260309T140000Z")
        );
    }

    #[test]
    fn create_writes_zone_and_vtimezone() {
        let env = env();
        let doc = create(
            &NewEvent {
                uid: Some("u@t".into()),
                summary: "Lunch, maybe".into(),
                start: 1_772_460_000,
                end: 1_772_463_600,
                timezone: Some("America/New_York".into()),
                attendees: vec![Guest {
                    email: "g@x.com".into(),
                    ..Default::default()
                }],
                organizer: Some(Person {
                    email: "me@x.com".into(),
                    name: Some("Me".into()),
                }),
                ..Default::default()
            },
            &env,
        );
        let text = doc.to_ics();
        assert!(text.contains("DTSTART;TZID=America/New_York:20260302T090000\r\n"));
        assert!(text.contains("SUMMARY:Lunch\\, maybe\r\n"));
        assert!(text.contains("BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\n"));
        assert!(text.contains("ORGANIZER;CN=Me:mailto:me@x.com\r\n"));
        assert!(text.contains("ATTENDEE;CN=Me;PARTSTAT=ACCEPTED:mailto:me@x.com\r\n"));
    }

    #[test]
    fn reply_names_only_me() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260302T090000Z\r\nATTENDEE;RSVP=TRUE:mailto:me@x.com\r\nATTENDEE:mailto:other@x.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let out = reply(&Doc::parse(ics).unwrap(), "ME@x.com", "ACCEPTED");
        assert!(out.contains("METHOD:REPLY\r\n"));
        assert!(out.contains("ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@x.com\r\n"));
        assert!(!out.contains("other@x.com"));
    }
}
