// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The occurrences the calendar draws (`occurrencesForEvents` and `occurrenceFromICS` in the
//! desktop's `calendar-data-source.ts`), the one to focus for an event opened from outside
//! the calendar (`focusedEventInfoForEvents`), and the busy time an invitation would clash
//! with (`findConflicts`).
//!
//! An occurrence goes out in the shape the desktop's `EventOccurrence` has: `startDate` and
//! `endDate` are calendar days counted from 1970-01-01, both inclusive; `start` and `end`
//! (timed occurrences only) are epoch seconds.

use crate::edit::{is_recurring, recurrence_info, Doc};
use crate::expand::{expand, Occ};
use crate::time::{end_of, rid_of, start_of};
use crate::{email_from_participant_uri, same_address, Env};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// An `Event` row, in its stored JSON keys.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct EventRow {
    pub id: String,
    #[serde(rename = "aid", default)]
    pub account_id: String,
    #[serde(rename = "cid", default)]
    pub calendar_id: String,
    #[serde(default)]
    pub ics: String,
    #[serde(default)]
    pub icsuid: String,
    #[serde(rename = "rid", default)]
    pub recurrence_id: String,
    #[serde(rename = "rs", default)]
    pub recurrence_start: Option<f64>,
    #[serde(rename = "re", default)]
    pub recurrence_end: Option<f64>,
}

impl EventRow {
    pub fn from_json(v: &Value) -> Option<EventRow> {
        serde_json::from_value(v.clone()).ok()
    }

    pub fn is_exception(&self) -> bool {
        !self.recurrence_id.is_empty()
    }
}

/// Whose an event is: each account's own addresses, and the address of each calendar the
/// server says is ours (Google writes a secondary calendar's own id as ORGANIZER).
#[derive(Clone, Debug, Default)]
pub struct Owners {
    /// Account id and its addresses (address and aliases), in account order.
    pub accounts: Vec<(String, Vec<String>)>,
    /// Calendar id to the address in its collection path.
    pub own_calendar_addresses: HashMap<String, String>,
}

impl Owners {
    /// The account an address belongs to (`AccountStore.accountForEmail`).
    pub fn account_for(&self, email: &str) -> Option<&str> {
        self.accounts
            .iter()
            .find(|(_, addrs)| addrs.iter().any(|a| same_address(a, email)))
            .map(|(id, _)| id.as_str())
    }

    fn is_account(&self, email: &str, account_id: &str) -> bool {
        self.account_for(email) == Some(account_id)
    }
}

/// The address in a calendar's collection path, which on Google is the calendar's own id
/// (`/caldav/v2/c_1a2b%40group.calendar.google.com/events/`); empty when it carries none.
pub fn calendar_address(path: &str) -> String {
    path.rsplit('/')
        .map(|seg| percent_decode(seg).unwrap_or_else(|| seg.to_owned()))
        .find(|seg| seg.contains('@'))
        .unwrap_or_default()
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Days since 1970-01-01.
pub fn calendar_date(d: NaiveDate) -> i64 {
    (d - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or_default()).num_days()
}

fn strip_mailto(s: &str) -> String {
    if s.len() >= 7 && s[..7].eq_ignore_ascii_case("mailto:") {
        s[7..].to_owned()
    } else {
        s.to_owned()
    }
}

fn round(x: f64) -> i64 {
    x.round() as i64
}

/// The id an occurrence of `event_id` is drawn under; a move gives it a new one.
pub fn occurrence_id(event_id: &str, start: i64) -> String {
    format!("{event_id}-e{start}")
}

struct Built<'a> {
    id: String,
    row: &'a EventRow,
    doc: &'a Doc,
    occ: &'a Occ,
    is_recurring: bool,
    is_exception: Option<bool>,
}

fn occurrence_json(b: Built<'_>, owners: &Owners, env: &Env) -> Value {
    let item = &b.doc.cal.children[b.occ.vevent];
    let zones = b.doc.zones(env);
    let status = item
        .value("STATUS")
        .unwrap_or_default()
        .to_ascii_uppercase();
    let attendees: Vec<Value> = item
        .props_named("ATTENDEE")
        .map(|a| {
            json!({
                "email": strip_mailto(&a.value),
                "name": a.param("CN").unwrap_or_default(),
                "partstat": a.param("PARTSTAT").unwrap_or("NEEDS-ACTION"),
            })
        })
        .collect();
    let is_this_account = |email: &str| owners.is_account(email, &b.row.account_id);
    let mine = item
        .props_named("ATTENDEE")
        .map(|a| {
            (
                strip_mailto(&a.value),
                a.param("PARTSTAT").unwrap_or("NEEDS-ACTION"),
            )
        })
        .find(|(email, _)| !email.is_empty() && is_this_account(email));
    let awaiting = mine.as_ref().is_some_and(|(_, ps)| {
        let ps = ps.to_ascii_uppercase();
        ps != "ACCEPTED" && ps != "DECLINED"
    });
    let organizer = item
        .value("ORGANIZER")
        .and_then(email_from_participant_uri)
        .map(|e| strip_mailto(&e))
        .unwrap_or_default();
    let own_address = owners.own_calendar_addresses.get(&b.row.calendar_id);
    let i_organize = !organizer.is_empty()
        && (is_this_account(&organizer)
            || own_address.is_some_and(|a| !a.is_empty() && same_address(&organizer, a)));

    let o = b.occ;
    let (start_date, end_date) = if o.all_day {
        let s = calendar_date(o.start_date);
        (s, (calendar_date(o.end_date) - 1).max(s))
    } else {
        (
            calendar_date(env.local_date(o.start)),
            calendar_date(env.local_date((o.end - 1).max(o.start))),
        )
    };
    let rid = rid_of(item).map(|r| r.ts(&zones));
    let doc_rule = recurrence_info(b.doc).map(|(rule, _)| rule);

    let mut m = Map::new();
    m.insert("id".into(), json!(b.id));
    m.insert("eventId".into(), json!(b.row.id));
    m.insert("icsuid".into(), json!(b.row.icsuid));
    m.insert("accountId".into(), json!(b.row.account_id));
    m.insert("calendarId".into(), json!(b.row.calendar_id));
    m.insert("title".into(), json!(item.text("SUMMARY")));
    m.insert("location".into(), json!(item.text("LOCATION")));
    m.insert("description".into(), json!(item.text("DESCRIPTION")));
    m.insert("startDate".into(), json!(start_date));
    m.insert("endDate".into(), json!(end_date));
    m.insert("isCancelled".into(), json!(status == "CANCELLED"));
    m.insert("isPending".into(), json!(status == "TENTATIVE" || awaiting));
    m.insert("isMine".into(), json!(organizer.is_empty() || i_organize));
    m.insert(
        "isFree".into(),
        json!(item.value("TRANSP") == Some("TRANSPARENT")),
    );
    m.insert(
        "isException".into(),
        json!(b.is_exception.unwrap_or(rid.is_some())),
    );
    if let Some(r) = rid {
        m.insert("recurrenceIdStart".into(), json!(r));
    }
    m.insert("isRecurring".into(), json!(b.is_recurring));
    m.insert(
        "organizer".into(),
        if organizer.is_empty() {
            Value::Null
        } else {
            json!({ "email": organizer })
        },
    );
    m.insert("attendees".into(), Value::Array(attendees));
    m.insert(
        "myStatus".into(),
        mine.map(|(_, ps)| json!(ps.to_ascii_uppercase()))
            .unwrap_or(Value::Null),
    );
    m.insert(
        "rrule".into(),
        doc_rule.map(Value::from).unwrap_or(Value::Null),
    );
    m.insert(
        "timezone".into(),
        crate::edit::event_timezone(b.doc, env)
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    m.insert("isAllDay".into(), json!(o.all_day));
    if !o.all_day {
        m.insert("start".into(), json!(o.start));
        m.insert("end".into(), json!(o.end));
    }
    Value::Object(m)
}

/// The occurrences of `rows` that overlap `[start, end]` (`occurrencesForEvents`). Rows are
/// grouped by UID: the master's resource is expanded with its inline exceptions, and an
/// exception stored as a row of its own is added unless the expansion already drew its
/// slot. A master that cannot be expanded is drawn once from its columns, titled
/// `(Error expanding event)`, so it does not vanish.
pub fn occurrences(
    rows: &[EventRow],
    start: i64,
    end: i64,
    owners: &Owners,
    env: &Env,
) -> Vec<Value> {
    let mut groups: Vec<(&str, Vec<&EventRow>)> = Vec::new();
    for row in rows {
        match groups.iter_mut().find(|(uid, _)| *uid == row.icsuid) {
            Some((_, g)) => g.push(row),
            None => groups.push((&row.icsuid, vec![row])),
        }
    }
    let mut out = Vec::new();
    for (_, group) in groups {
        let master = group.iter().find(|r| !r.is_exception());
        let mut expanded_starts: Vec<i64> = Vec::new();
        if let Some(master) = master {
            let result = Doc::parse(&master.ics).and_then(|doc| {
                let rs = master.recurrence_start.filter(|f| f.is_finite()).map(round);
                let occs = expand(&doc, rs, start, end, env)?;
                Ok((doc, occs))
            });
            match result {
                Ok((doc, occs)) => {
                    let recurring = is_recurring(&doc);
                    for o in &occs {
                        expanded_starts.push(o.start);
                        out.push(occurrence_json(
                            Built {
                                id: occurrence_id(&master.id, o.start),
                                row: master,
                                doc: &doc,
                                occ: o,
                                is_recurring: recurring,
                                is_exception: None,
                            },
                            owners,
                            env,
                        ));
                    }
                }
                Err(err) => {
                    tracing::warn!(event = %master.id, error = %err, "could not expand an event");
                    if let Some(v) = error_occurrence(master, env) {
                        out.push(v);
                    }
                }
            }
        }
        for ex in group.iter().filter(|r| r.is_exception()) {
            let rs = ex.recurrence_start.map(round);
            if rs.is_some_and(|s| expanded_starts.contains(&s)) {
                continue;
            }
            let Ok(doc) = Doc::parse(&ex.ics) else {
                continue;
            };
            let zones = doc.zones(env);
            let Some(vi) = doc.vevent_ids().first().copied() else {
                continue;
            };
            let v = &doc.cal.children[vi];
            let Some(s) = start_of(v) else { continue };
            let e = end_of(v).unwrap_or_else(|| s.clone());
            let (st, et) = (s.ts(&zones), e.ts(&zones));
            if et < start || st > end {
                continue;
            }
            let o = Occ {
                start: st,
                end: et,
                all_day: s.is_date(),
                start_date: s.date(),
                end_date: if e.is_date() { e.date() } else { s.date() },
                vevent: vi,
                recurrence_id: None,
            };
            out.push(occurrence_json(
                Built {
                    id: format!("{}-e0", ex.id),
                    row: ex,
                    doc: &doc,
                    occ: &o,
                    is_recurring: true,
                    is_exception: Some(true),
                },
                owners,
                env,
            ));
        }
    }
    out
}

fn error_occurrence(master: &EventRow, env: &Env) -> Option<Value> {
    let rs = master.recurrence_start.filter(|f| f.is_finite())?;
    let re = master.recurrence_end.filter(|f| f.is_finite())?;
    let (rs, re) = (round(rs), round(re));
    let all_day = re - rs >= 82800;
    let start_date = calendar_date(env.local_date(rs));
    let end_date = if all_day {
        calendar_date(env.local_date(re)) - 1
    } else {
        calendar_date(env.local_date((re - 1).max(rs)))
    }
    .max(start_date);
    let mut v = json!({
        "id": format!("{}-e0", master.id),
        "eventId": master.id,
        "icsuid": master.icsuid,
        "accountId": master.account_id,
        "calendarId": master.calendar_id,
        "title": "(Error expanding event)",
        "location": "",
        "description": "",
        "startDate": start_date,
        "endDate": end_date,
        "isCancelled": false,
        "isPending": false,
        "isMine": true,
        "isException": false,
        "isRecurring": false,
        "organizer": null,
        "attendees": [],
        "myStatus": null,
        "rrule": null,
        "timezone": null,
        "isAllDay": all_day,
        "expansionError": true,
    });
    if !all_day {
        v["start"] = json!(rs);
        v["end"] = json!(re);
    }
    Some(v)
}

fn occurrence_start(o: &Value, env: &Env) -> i64 {
    match o["start"].as_i64() {
        Some(s) => s,
        None => {
            let days = o["startDate"].as_i64().unwrap_or(0);
            let d = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or_default()
                + chrono::Duration::days(days);
            env.day_start(d)
        }
    }
}

const UPCOMING_LOOKAHEAD: i64 = 366 * 86400;

/// The occurrence to focus for an event opened from outside the calendar
/// (`focusedEventInfoForEvents`): the one `recurrence_id_start` names, else the next from
/// `now`, else the first. `rows` are every row sharing one UID.
pub fn focused(
    rows: &[EventRow],
    now: i64,
    recurrence_id_start: Option<i64>,
    owners: &Owners,
    env: &Env,
) -> Option<Value> {
    if rows.is_empty() {
        return None;
    }
    let by_start = |list: &mut Vec<Value>| {
        list.sort_by_key(|o| occurrence_start(o, env));
    };
    if let Some(rid) = recurrence_id_start {
        let master = rows.iter().find(|r| !r.is_exception()).unwrap_or(&rows[0]);
        let mut at = rid;
        if let Ok(doc) = Doc::parse(&master.ics) {
            let zones = doc.zones(env);
            for v in doc.vevents() {
                if rid_of(v).is_some_and(|r| r.ts(&zones) == rid) {
                    if let Some(s) = start_of(v) {
                        at = s.ts(&zones);
                    }
                }
            }
        }
        let found = occurrences(rows, at, at + 86400, owners, env)
            .into_iter()
            .find(|o| {
                o["recurrenceIdStart"]
                    .as_i64()
                    .unwrap_or_else(|| occurrence_start(o, env))
                    == rid
            });
        if found.is_some() {
            return found;
        }
    }
    let mut upcoming = occurrences(rows, now, now + UPCOMING_LOOKAHEAD, owners, env);
    by_start(&mut upcoming);
    if let Some(o) = upcoming.into_iter().next() {
        return Some(o);
    }
    let first = rows
        .iter()
        .filter_map(|r| r.recurrence_start.filter(|f| f.is_finite()))
        .map(round)
        .min()?;
    occurrences(rows, first, first + 86400, owners, env)
        .into_iter()
        .next()
}

/// The occurrence of an invitation that is next from `now` (`upcomingOccurrence`): the
/// event itself unless it is a series, then the first occurrence still to end. `None` once
/// a series is over. Returns start, end, the slot it fills, and whether it is all-day.
pub fn upcoming(doc: &Doc, now: i64, env: &Env) -> Option<(i64, i64, Option<i64>, bool)> {
    let zones = doc.zones(env);
    let first = doc.first()?;
    if !(first.has("RRULE") || first.has("RDATE")) {
        let s = start_of(first)?;
        let e = end_of(first)?;
        return Some((s.ts(&zones), e.ts(&zones), None, s.is_date()));
    }
    let series_start = start_of(first).map(|s| s.ts(&zones));
    let mut occs = expand(doc, series_start, now, now + UPCOMING_LOOKAHEAD, env).ok()?;
    occs.sort_by_key(|o| o.start);
    let o = occs.into_iter().next()?;
    Some((o.start, o.end, o.recurrence_id, o.all_day))
}

/// An invitation's own times, read off its first VEVENT: start, end, the slot it fills
/// when it is about one occurrence of a series, and whether it is all-day.
pub fn first_times(doc: &Doc, env: &Env) -> Option<(i64, i64, Option<i64>, bool)> {
    let zones = doc.zones(env);
    let first = doc.first()?;
    let s = start_of(first)?;
    let st = s.ts(&zones);
    let et = end_of(first).map(|e| e.ts(&zones)).unwrap_or(st);
    let rid = rid_of(first).map(|r| r.ts(&zones));
    Some((st, et, rid, s.is_date()))
}

/// Cancelled, marked free, or an invitation one of `addresses` has declined.
fn is_free_time(item: &crate::ics::Component, addresses: &[String]) -> bool {
    if item
        .value("STATUS")
        .is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED"))
    {
        return true;
    }
    if item
        .value("TRANSP")
        .is_some_and(|t| t.eq_ignore_ascii_case("TRANSPARENT"))
    {
        return true;
    }
    for a in item.props_named("ATTENDEE") {
        if let Some(email) = email_from_participant_uri(&a.value) {
            if addresses.iter().any(|x| x.to_lowercase() == email) {
                return a
                    .param("PARTSTAT")
                    .is_some_and(|p| p.eq_ignore_ascii_case("DECLINED"));
            }
        }
    }
    false
}

/// One busy occurrence overlapping a window.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub event_id: String,
    pub calendar_id: String,
    pub icsuid: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
}

/// The timed occurrences of `rows` that overlap `[start, end)` (`findConflicts`), half-open
/// so back-to-back meetings do not clash; all-day events are not busy time.
pub fn conflicts(
    rows: &[EventRow],
    start: i64,
    end: i64,
    addresses: &[String],
    exclude_icsuid: Option<&str>,
    env: &Env,
) -> Vec<Conflict> {
    if end <= start {
        return Vec::new();
    }
    let mut out = Vec::new();
    for row in rows {
        if exclude_icsuid.is_some_and(|u| u == row.icsuid) || row.is_exception() {
            continue;
        }
        let Ok(doc) = Doc::parse(&row.ics) else {
            continue;
        };
        let rs = row.recurrence_start.filter(|f| f.is_finite()).map(round);
        let Ok(occs) = expand(&doc, rs, start, end, env) else {
            continue;
        };
        for o in occs {
            let item = &doc.cal.children[o.vevent];
            if o.all_day || is_free_time(item, addresses) {
                continue;
            }
            if o.end <= start || o.start >= end {
                continue;
            }
            out.push(Conflict {
                event_id: row.id.clone(),
                calendar_id: row.calendar_id.clone(),
                icsuid: row.icsuid.clone(),
                title: item.text("SUMMARY"),
                start: o.start,
                end: o.end,
            });
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.title.cmp(&b.title)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_addresses() {
        assert_eq!(
            calendar_address("/caldav/v2/c_1a2b%40group.calendar.google.com/events/"),
            "c_1a2b@group.calendar.google.com"
        );
        assert_eq!(calendar_address("/dav/calendars/user/home/"), "");
    }

    #[test]
    fn invitation_awaiting_me_is_pending() {
        let env = Env::new(chrono_tz::America::Chicago, 0);
        let row = EventRow {
            id: "e1".into(),
            account_id: "a1".into(),
            calendar_id: "c1".into(),
            icsuid: "u".into(),
            ics: "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u\r\nDTSTART:20260302T150000Z\r\nDTEND:20260302T160000Z\r\nORGANIZER:mailto:boss@x.com\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:Me@x.com\r\nSUMMARY:Review\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into(),
            ..Default::default()
        };
        let owners = Owners {
            accounts: vec![("a1".into(), vec!["me@x.com".into()])],
            ..Default::default()
        };
        let occs = occurrences(&[row], 0, 2_000_000_000, &owners, &env);
        assert_eq!(occs.len(), 1);
        let o = &occs[0];
        assert_eq!(o["id"], "e1-e1772463600");
        assert_eq!(o["isPending"], true);
        assert_eq!(o["isMine"], false);
        assert_eq!(o["myStatus"], "NEEDS-ACTION");
        assert_eq!(o["attendees"][0]["email"], "Me@x.com");
        assert_eq!(o["organizer"]["email"], "boss@x.com");
    }
}
