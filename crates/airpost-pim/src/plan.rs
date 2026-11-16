// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `events.create`, `events.update` and `events.rsvp` as the
//! calendar writes and iTIP messages to queue: what the desktop's calendar popover,
//! drag-and-drop, delete, RSVP and counter-proposal paths assembled from the edit helpers.
//!
//! An update is one revision, so SEQUENCE moves once (RFC 5546 section 2.1.4): on the
//! exception when one occurrence changed, on the master otherwise.

use crate::edit::{
    self, add_exclusion_date, apply_edits_at, bump_sequence, bump_sequence_at, counter_proposal,
    create_exception, generate_uid, is_recurring, occurrence_ics, organizer_email,
    remove_rrule_part, rrule_part, self_attendee, set_rrule_part, shift_inline_exceptions,
    update_attendee_status, update_attendees, update_occurrence_attendee_status, update_property,
    update_recurrence_rule, update_recurring_times, update_times, update_times_at, Doc, Edits,
    Guest, NewEvent, Person, Rid, Times,
};
use crate::occurrence::EventRow;
use crate::time::{duration_of, end_of, rid_of, start_of, sync_vtimezones, utc_naive, When};
use crate::{Env, Error, Result};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

/// Which occurrences of a series an update changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    This,
    Following,
    All,
}

/// A guest, as clients name one.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct GuestIn {
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

impl From<&GuestIn> for Guest {
    fn from(g: &GuestIn) -> Guest {
        Guest {
            email: g.email.clone(),
            name: g.name.clone(),
            role: g.role.clone(),
        }
    }
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// What an update changes; an absent field is left alone. `recurrenceRule: null` removes
/// the rule.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub is_all_day: Option<bool>,
    /// IANA or Windows zone to write the times in; absent, each keeps its own.
    pub timezone: Option<String>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub attendees: Option<Vec<GuestIn>>,
    pub transparency: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub recurrence_rule: Option<Option<String>>,
    #[serde(default)]
    pub deleted: bool,
}

impl Patch {
    fn moves(&self) -> bool {
        self.start.is_some()
            || self.end.is_some()
            || self.is_all_day.is_some()
            || self.timezone.is_some()
    }

    fn edits(&self, organizer: Option<&Person>) -> Edits {
        Edits {
            summary: self.summary.clone(),
            location: self.location.clone(),
            description: self.description.clone(),
            attendees: self
                .attendees
                .as_ref()
                .map(|a| a.iter().map(Guest::from).collect()),
            organizer: organizer.cloned(),
            transparency: self.transparency.clone(),
        }
    }
}

/// One calendar write to queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    /// A SyncbackEventTask for the stored row `event_id` with this resource.
    Update { event_id: String, ics: String },
    /// A SyncbackEventTask creating an event on `calendar_id`.
    Create { calendar_id: String, ics: String },
    /// A DestroyEventTask for the row (the whole resource).
    Destroy { event_id: String },
}

/// The event an update starts from: the row whose resource is written and, for one
/// occurrence of a series, the slot it fills (its RECURRENCE-ID instant).
#[derive(Clone, Debug)]
pub struct Target<'a> {
    pub row: &'a EventRow,
    pub occurrence: Option<i64>,
}

/// The slot an exception row fills: its own VEVENT's RECURRENCE-ID.
pub fn exception_slot(row: &EventRow, env: &Env) -> Option<i64> {
    let doc = Doc::parse(&row.ics).ok()?;
    let zones = doc.zones(env);
    let rids: Vec<When> = doc.vevents().filter_map(rid_of).collect();
    let wanted = row.recurrence_id.trim_end_matches('Z');
    let found = rids
        .iter()
        .find(|r| r.text().trim_end_matches('Z') == wanted)
        .or_else(|| rids.first())
        .map(|r| r.ts(&zones));
    found
}

/// Whether `occ` is an occurrence of the series in `doc`: a slot its rules draw and no
/// EXDATE removes, or the RECURRENCE-ID of one of its overrides, wherever that moved it.
pub fn is_occurrence(doc: &Doc, occ: i64, env: &Env) -> Result<bool> {
    if doc.exception_at(occ, &doc.zones(env)).is_some() {
        return Ok(true);
    }
    Ok(crate::expand::expand(doc, None, occ, occ, env)?
        .iter()
        .any(|o| o.recurrence_id == Some(occ)))
}

/// Refuses an `occurrenceStart` that names no occurrence of the series (a moved
/// occurrence's new start, a time off by a zone): an override, EXDATE or split there would
/// change nothing the user sees, or add an event.
fn check_occurrence(doc: &Doc, occ: i64, env: &Env) -> Result<()> {
    if is_occurrence(doc, occ, env)? {
        Ok(())
    } else {
        Err(Error::new(
            "not-an-occurrence",
            format!(
                "{} is not the start of an occurrence of the series",
                utc_naive(occ).format("%Y-%m-%dT%H:%M:%SZ")
            ),
        ))
    }
}

/// A new event (`events.create`).
pub fn plan_create(ev: &NewEvent, calendar_id: &str, env: &Env) -> (String, Write) {
    let doc = edit::create(ev, env);
    let uid = doc
        .first()
        .and_then(|v| v.value("UID"))
        .unwrap_or_default()
        .to_owned();
    (
        uid,
        Write::Create {
            calendar_id: calendar_id.to_owned(),
            ics: doc.to_ics(),
        },
    )
}

/// The writes an update makes (`events.update`). On an event that does not repeat, every
/// scope is `all`. `organizer` is the account, named on an event that gains guests.
pub fn plan_update(
    target: &Target<'_>,
    scope: Scope,
    patch: &Patch,
    organizer: Option<&Person>,
    env: &Env,
) -> Result<Vec<Write>> {
    let row = target.row;
    let mut doc = Doc::parse(&row.ics)?;
    let recurring = is_recurring(&doc);
    let scope = if recurring { scope } else { Scope::All };
    let occurrence = target.occurrence;
    if let (true, Some(occ)) = (recurring, occurrence) {
        check_occurrence(&doc, occ, env)?;
    }
    let need_occurrence = || {
        occurrence.ok_or_else(|| {
            Error::invalid("occurrenceStart is needed to change one occurrence of a series")
        })
    };
    let update = |doc: &Doc| Write::Update {
        event_id: row.id.clone(),
        ics: doc.to_ics(),
    };

    if patch.deleted {
        return match scope {
            Scope::All => Ok(vec![Write::Destroy {
                event_id: row.id.clone(),
            }]),
            Scope::This => {
                let occ = need_occurrence()?;
                delete_occurrence(&mut doc, occ, env)?;
                bump_sequence(&mut doc, None)?;
                Ok(vec![update(&doc)])
            }
            Scope::Following => {
                let occ = need_occurrence()?;
                if occ <= series_start(&doc, env)? {
                    return Ok(vec![Write::Destroy {
                        event_id: row.id.clone(),
                    }]);
                }
                truncate_before(&mut doc, occ, env)?;
                bump_sequence(&mut doc, None)?;
                Ok(vec![update(&doc)])
            }
        };
    }

    match scope {
        Scope::All => {
            edit_all(&mut doc, recurring, occurrence, patch, organizer, env)?;
            Ok(vec![update(&doc)])
        }
        Scope::This => {
            let occ = need_occurrence()?;
            edit_this(&mut doc, occ, patch, organizer, env)?;
            Ok(vec![update(&doc)])
        }
        Scope::Following => {
            let occ = need_occurrence()?;
            if occ <= series_start(&doc, env)? {
                edit_all(&mut doc, recurring, Some(occ), patch, organizer, env)?;
                return Ok(vec![update(&doc)]);
            }
            let new_series = split_at(&mut doc, occ, patch, organizer, env)?;
            bump_sequence(&mut doc, None)?;
            Ok(vec![
                update(&doc),
                Write::Create {
                    calendar_id: row.calendar_id.clone(),
                    ics: new_series.to_ics(),
                },
            ])
        }
    }
}

fn series_start(doc: &Doc, env: &Env) -> Result<i64> {
    let zones = doc.zones(env);
    doc.master()
        .and_then(start_of)
        .map(|s| s.ts(&zones))
        .ok_or_else(|| Error::invalid_ics("the event has no DTSTART"))
}

fn master_all_day(doc: &Doc) -> bool {
    doc.master().and_then(start_of).is_some_and(|s| s.is_date())
}

/// The new start and end, from the patch or, where it is silent, the occurrence as it is.
fn new_times(patch: &Patch, current: (i64, i64)) -> (i64, i64) {
    let start = patch.start.unwrap_or(current.0);
    let end = patch
        .end
        .unwrap_or_else(|| start + (current.1 - current.0).max(0));
    (start, end)
}

/// Every occurrence (`_saveAllOccurrences`, `modifyAllOccurrences`): property edits on the
/// master; a series moves by the change to the occurrence that was moved, its exceptions'
/// RECURRENCE-IDs with it; a single event takes the new times.
fn edit_all(
    doc: &mut Doc,
    recurring: bool,
    occurrence: Option<i64>,
    patch: &Patch,
    organizer: Option<&Person>,
    env: &Env,
) -> Result<()> {
    for (name, value) in [
        ("summary", &patch.summary),
        ("location", &patch.location),
        ("description", &patch.description),
    ] {
        if let Some(v) = value {
            update_property(doc, name, v, env)?;
        }
    }
    if let Some(a) = &patch.attendees {
        let guests: Vec<Guest> = a.iter().map(Guest::from).collect();
        update_attendees(doc, &guests, organizer, env)?;
    }
    if patch.moves() {
        let zones = doc.zones(env);
        let master = doc
            .master()
            .ok_or_else(|| Error::invalid_ics("Invalid ICS: no VEVENT component found"))?;
        let start =
            start_of(master).ok_or_else(|| Error::invalid_ics("the event has no DTSTART"))?;
        let s = start.ts(&zones);
        let e = end_of(master).map(|e| e.ts(&zones)).unwrap_or(s);
        let all_day = patch.is_all_day.unwrap_or(start.is_date());
        if recurring {
            let original = occurrence.unwrap_or(s);
            let current = (original, original + (e - s));
            let (ns, ne) = new_times(patch, current);
            update_recurring_times(
                doc,
                original,
                ns,
                ne,
                all_day,
                patch.timezone.as_deref(),
                env,
            )?;
            shift_inline_exceptions(doc, ns - original, env);
        } else {
            let (ns, ne) = new_times(patch, (s, e));
            update_times(
                doc,
                &Times {
                    start: ns,
                    end: ne,
                    all_day,
                    timezone: patch.timezone.clone(),
                },
                env,
            )?;
        }
    }
    if let Some(rule) = &patch.recurrence_rule {
        update_recurrence_rule(doc, rule.as_deref(), env)?;
    }
    if let Some(t) = patch.transparency.as_deref().filter(|t| !t.is_empty()) {
        update_property(doc, "transp", &t.to_ascii_uppercase(), env)?;
    }
    bump_sequence(doc, None)
}

/// One occurrence (`_saveOccurrenceException`, `createOccurrenceException`): its inline
/// exception is edited where it has one, else one is made from the master at that slot.
fn edit_this(
    doc: &mut Doc,
    occ: i64,
    patch: &Patch,
    organizer: Option<&Person>,
    env: &Env,
) -> Result<()> {
    let zones = doc.zones(env);
    let edits = patch.edits(organizer);
    let index = match doc.exception_at(occ, &zones) {
        Some(i) => {
            if patch.moves() {
                let v = &doc.cal.children[i];
                let start = start_of(v)
                    .ok_or_else(|| Error::invalid_ics("the exception has no DTSTART"))?;
                let s = start.ts(&zones);
                let e = end_of(v).map(|e| e.ts(&zones)).unwrap_or(s);
                let (ns, ne) = new_times(patch, (s, e));
                update_times_at(
                    doc,
                    i,
                    &Times {
                        start: ns,
                        end: ne,
                        all_day: patch.is_all_day.unwrap_or(start.is_date()),
                        timezone: patch.timezone.clone(),
                    },
                    false,
                    env,
                )?;
            }
            i
        }
        None => {
            let master = doc
                .master()
                .ok_or_else(|| Error::invalid_ics("Invalid ICS: no VEVENT component found"))?;
            let all_day = patch.is_all_day.unwrap_or_else(|| master_all_day(doc));
            let current = (occ, occ + slot_length(master, occ, &zones));
            let (ns, ne) = new_times(patch, current);
            create_exception(doc, occ, ns, ne, all_day, env)?;
            let i = doc
                .exception_at(occ, &doc.zones(env))
                .ok_or_else(|| Error::invalid_ics("the new exception could not be found"))?;
            if let Some(tz) = patch.timezone.as_deref().filter(|_| !all_day) {
                update_times_at(
                    doc,
                    i,
                    &Times {
                        start: ns,
                        end: ne,
                        all_day,
                        timezone: Some(tz.to_owned()),
                    },
                    false,
                    env,
                )?;
            }
            i
        }
    };
    // Syncing VTIMEZONEs can drop one ahead of the VEVENT.
    let index = doc.exception_at(occ, &doc.zones(env)).unwrap_or(index);
    apply_edits_at(doc, index, &edits, env);
    bump_sequence_at(doc, Some(occ), env)
}

/// How long the occurrence at `slot` lasts on the master's clock.
fn slot_length(master: &crate::ics::Component, slot: i64, zones: &airpost_dav::Zones) -> i64 {
    let duration = duration_of(master);
    match start_of(master) {
        Some(When::Local(_, tz)) => {
            let wall = zones.to_local(tz.as_deref(), slot);
            When::Local(wall, tz.clone()).add_wall(duration).ts(zones) - slot
        }
        _ => duration,
    }
}

/// Cancels the occurrence at slot `occ`: its inline exception goes and the slot is excluded.
fn delete_occurrence(doc: &mut Doc, occ: i64, env: &Env) -> Result<()> {
    let zones = doc.zones(env);
    if let Some(i) = doc.exception_at(occ, &zones) {
        let text = doc.cal.children[i]
            .value("RECURRENCE-ID")
            .unwrap_or_default()
            .to_owned();
        if edit::remove_inline_exception(doc, &text, env) {
            return Ok(());
        }
    }
    add_exclusion_date(doc, occ, master_all_day(doc), env)
}

/// Ends the series before slot `occ` (RFC 5545 section 3.3.10 UNTIL, one second before it,
/// in UTC); COUNT goes, exceptions and EXDATE/RDATE values from `occ` on go.
fn truncate_before(doc: &mut Doc, occ: i64, env: &Env) -> Result<()> {
    let zones = doc.zones(env);
    let all_day = master_all_day(doc);
    let mi = doc.master_or_err()?;
    let until = if all_day {
        When::Date(env.local_date(occ) - chrono::Duration::days(1))
    } else {
        When::Utc(utc_naive(occ - 1))
    };
    let keep: Vec<bool> = doc
        .cal
        .children
        .iter()
        .map(|c| c.name != "VEVENT" || rid_of(c).is_none_or(|r| r.ts(&zones) < occ))
        .collect();
    let mut i = 0;
    doc.cal.children.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
    let mi = mi - keep[..mi].iter().filter(|k| !**k).count();
    let master = &mut doc.cal.children[mi];
    if let Some(rule) = master.prop_mut("RRULE") {
        let r = remove_rrule_part(&rule.value, "COUNT");
        rule.value = set_rrule_part(&r, "UNTIL", &until.text());
    }
    for name in ["EXDATE", "RDATE"] {
        let mut kept = Vec::new();
        for p in master.props.iter().filter(|p| p.name == name) {
            let values: Vec<When> = When::all_of(p)
                .into_iter()
                .filter(|w| w.ts(&zones) < occ)
                .collect();
            if !values.is_empty() {
                let mut q = p.clone();
                q.value = values.iter().map(When::text).collect::<Vec<_>>().join(",");
                kept.push(q);
            }
        }
        master.remove_all(name);
        master.props.extend(kept);
    }
    master.replace(env.stamp());
    sync_vtimezones(&mut doc.cal, occ, None);
    Ok(())
}

/// Splits the series at slot `occ` for a "this and following" edit: `doc` keeps the
/// occurrences before it, and the returned series, under a new UID, starts there with the
/// patch applied. The old series' exceptions from `occ` on are not carried over.
fn split_at(
    doc: &mut Doc,
    occ: i64,
    patch: &Patch,
    organizer: Option<&Person>,
    env: &Env,
) -> Result<Doc> {
    let zones = doc.zones(env);
    let master = doc
        .master()
        .ok_or_else(|| Error::invalid_ics("Invalid ICS: no VEVENT component found"))?
        .clone();
    let before_count = if rrule_part(master.value("RRULE").unwrap_or_default(), "COUNT").is_some() {
        let start = series_start(doc, env)?;
        crate::expand::expand(doc, Some(start), start, occ - 1, env)?
            .iter()
            .filter(|o| o.recurrence_id.unwrap_or(o.start) < occ)
            .count()
    } else {
        0
    };

    let mut fresh = master.clone();
    fresh.set_value("UID", generate_uid());
    fresh.remove_all("SEQUENCE");
    fresh.remove_all("RECURRENCE-ID");
    if let Some(rule) = fresh.prop_mut("RRULE") {
        if let Some(count) = rrule_part(&rule.value, "COUNT").and_then(|c| c.parse::<usize>().ok())
        {
            let left = count.saturating_sub(before_count).max(1);
            rule.value = set_rrule_part(&rule.value, "COUNT", &left.to_string());
        }
    }
    for name in ["EXDATE", "RDATE"] {
        let kept: Vec<_> = fresh
            .props
            .iter()
            .filter(|p| p.name == name)
            .filter_map(|p| {
                let values: Vec<When> = When::all_of(p)
                    .into_iter()
                    .filter(|w| w.ts(&zones) >= occ)
                    .collect();
                (!values.is_empty()).then(|| {
                    let mut q = p.clone();
                    q.value = values.iter().map(When::text).collect::<Vec<_>>().join(",");
                    q
                })
            })
            .collect();
        fresh.remove_all(name);
        fresh.props.extend(kept);
    }
    let all_day = master_all_day(doc);
    let length = slot_length(&master, occ, &zones);
    let mut new_doc = Doc::of_event(fresh);
    new_doc.cal.children.splice(
        0..0,
        doc.cal
            .children
            .iter()
            .filter(|c| c.name == "VTIMEZONE")
            .cloned(),
    );
    let (ns, ne) = new_times(patch, (occ, occ + length));
    let mi = new_doc.master_or_err()?;
    update_times_at(
        &mut new_doc,
        mi,
        &Times {
            start: ns,
            end: ne,
            all_day: patch.is_all_day.unwrap_or(all_day),
            timezone: patch.timezone.clone(),
        },
        false,
        env,
    )?;
    // Syncing VTIMEZONEs can drop one ahead of the VEVENT.
    let mi = new_doc.master_or_err()?;
    let edits = patch.edits(organizer);
    apply_edits_at(&mut new_doc, mi, &edits, env);
    if let Some(rule) = &patch.recurrence_rule {
        update_recurrence_rule(&mut new_doc, rule.as_deref(), env)?;
    }
    truncate_before(doc, occ, env)?;
    Ok(new_doc)
}

/// The answer to an invitation, as the SyncbackEventTask that records it carries it, so a
/// write refused by a changed copy can be made again (`RSVPAnswer`).
pub fn answer_json(
    email: &str,
    status: &str,
    occurrence_ics: Option<&str>,
    retried: bool,
) -> Value {
    let mut v = json!({ "email": email, "status": status });
    if let Some(o) = occurrence_ics {
        v["occurrenceIcs"] = json!(o);
    }
    if retried {
        v["retried"] = json!(true);
    }
    v
}

/// An iTIP message to email the organizer (EventRSVPTask).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub to: String,
    pub subject: String,
    pub ics: String,
    pub method: &'static str,
    pub status: Option<String>,
    pub comment: Option<String>,
}

/// What an RSVP writes and sends.
#[derive(Clone, Debug, Default)]
pub struct RsvpPlan {
    pub write: Option<(Write, Value)>,
    pub message: Option<Message>,
    /// Our address on the guest list.
    pub me: String,
}

/// An RSVP's inputs; see `events.rsvp`.
#[derive(Clone, Debug, Default)]
pub struct Rsvp<'a> {
    /// Our stored copy, when there is one.
    pub stored: Option<&'a EventRow>,
    /// Whether our copy may be written (not a read-only calendar, not somebody else's).
    pub writable: bool,
    /// The emailed invitation, when answering from a message.
    pub invite: Option<&'a str>,
    /// Where to add an invitation we have no copy of and are attending.
    pub add_to: Option<&'a str>,
    pub status: &'a str,
    /// The occurrence of a stored series to answer alone.
    pub occurrence: Option<Rid>,
    /// A note to the organizer, sent with the REPLY.
    pub comment: Option<&'a str>,
    pub reply: bool,
    pub write: bool,
    pub retried: bool,
}

fn title_case(status: &str) -> String {
    let lower = status.to_lowercase();
    let mut c = lower.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn summary_of(doc: &Doc) -> String {
    doc.first().map(|v| v.text("SUMMARY")).unwrap_or_default()
}

/// Our answer to an invitation (`respondToCalendarEvent`, `planRSVPWrite`,
/// `EventRSVPTask.forReplying`): PARTSTAT on our copy (RFC 6638 section 3.2.5), on the one
/// occurrence answered where only one is, or the invitation added to a calendar when we are
/// attending and have no copy; and a REPLY to the organizer (RFC 5546 section 3.2.3), which
/// goes even when the copy cannot be written.
pub fn plan_rsvp(r: &Rsvp<'_>, is_mine: impl Fn(&str) -> bool, env: &Env) -> Result<RsvpPlan> {
    let status = r.status.to_ascii_uppercase();
    if !matches!(status.as_str(), "ACCEPTED" | "DECLINED" | "TENTATIVE") {
        return Err(Error::invalid(format!("not an answer: {}", r.status)));
    }
    let mut plan = RsvpPlan::default();
    match (r.invite, r.stored) {
        (Some(invite_text), stored) => {
            let invite = Doc::parse(invite_text)?;
            let me = self_attendee(&invite, &is_mine).ok_or_else(not_invited)?;
            let names_occurrence = invite.first().is_some_and(|v| v.has("RECURRENCE-ID"));
            if r.write {
                match stored {
                    Some(row) if r.writable => {
                        let mut doc = Doc::parse(&row.ics)?;
                        let changed = if names_occurrence {
                            update_occurrence_attendee_status(&mut doc, &invite, &me, &status, env)?
                        } else {
                            update_attendee_status(&mut doc, &me, &status, env)?
                        };
                        if changed {
                            let occ = names_occurrence.then_some(invite_text);
                            plan.write = Some((
                                Write::Update {
                                    event_id: row.id.clone(),
                                    ics: doc.to_ics(),
                                },
                                answer_json(&me, &status, occ, r.retried),
                            ));
                        }
                    }
                    Some(_) => {}
                    None => {
                        let attending = status == "ACCEPTED" || status == "TENTATIVE";
                        if let (true, Some(calendar_id)) = (attending, r.add_to) {
                            let mut answered = invite.clone();
                            if update_attendee_status(&mut answered, &me, &status, env)? {
                                edit::strip_method(&mut answered);
                                plan.write = Some((
                                    Write::Create {
                                        calendar_id: calendar_id.to_owned(),
                                        ics: answered.to_ics(),
                                    },
                                    Value::Null,
                                ));
                            }
                        }
                    }
                }
            }
            if r.reply {
                if let Some(org) = organizer_email(&invite) {
                    plan.message = Some(Message {
                        to: org,
                        subject: format!("{}: {}", title_case(&status), summary_of(&invite)),
                        ics: edit::reply(&invite, &me, &status),
                        method: "REPLY",
                        status: Some(status.clone()),
                        comment: r.comment.map(str::to_owned),
                    });
                }
            }
            plan.me = me;
        }
        (None, Some(row)) => {
            let doc = Doc::parse(&row.ics)?;
            let me = self_attendee(&doc, &is_mine).ok_or_else(not_invited)?;
            let this_only = r
                .occurrence
                .filter(|_| is_recurring(&doc) && !row.is_exception());
            if let Some(rid) = &this_only {
                check_occurrence(&doc, rid.ts(env), env)?;
            }
            let answered = match &this_only {
                Some(rid) => occurrence_ics(&doc, rid, env)?,
                None => doc.clone(),
            };
            if r.write && r.writable {
                let mut copy = doc.clone();
                let changed = if this_only.is_some() {
                    update_occurrence_attendee_status(&mut copy, &answered, &me, &status, env)?
                } else {
                    update_attendee_status(&mut copy, &me, &status, env)?
                };
                if changed {
                    let occ_text = this_only.map(|_| answered.to_ics());
                    plan.write = Some((
                        Write::Update {
                            event_id: row.id.clone(),
                            ics: copy.to_ics(),
                        },
                        answer_json(&me, &status, occ_text.as_deref(), r.retried),
                    ));
                }
            }
            if r.reply {
                if let Some(org) = organizer_email(&doc) {
                    plan.message = Some(Message {
                        to: org,
                        subject: format!("{}: {}", title_case(&status), summary_of(&answered)),
                        ics: edit::reply(&answered, &me, &status),
                        method: "REPLY",
                        status: Some(status.clone()),
                        comment: r.comment.map(str::to_owned),
                    });
                }
            }
            plan.me = me;
        }
        (None, None) => {
            return Err(Error::invalid("events.rsvp needs eventId or ics"));
        }
    }
    Ok(plan)
}

fn not_invited() -> Error {
    Error::new(
        "not-invited",
        "this account is not on the event's guest list",
    )
}

/// A counter-proposal for another time (`proposeNewTimeForCalendarEvent`): a COUNTER to the
/// organizer, nothing written, since the event changes only when the organizer accepts.
pub fn plan_counter(
    row: &EventRow,
    start: i64,
    end: i64,
    comment: Option<&str>,
    occurrence: Option<Rid>,
    is_mine: impl Fn(&str) -> bool,
    env: &Env,
) -> Result<Message> {
    if end < start {
        return Err(Error::invalid("the proposed end is before its start"));
    }
    let doc = Doc::parse(&row.ics)?;
    let me = self_attendee(&doc, &is_mine).ok_or_else(not_invited)?;
    let organizer = organizer_email(&doc).ok_or_else(|| {
        Error::new(
            "no-organizer",
            "the event has no organizer to propose a new time to",
        )
    })?;
    let rid = occurrence.filter(|_| is_recurring(&doc) || row.is_exception());
    if let (Some(r), true) = (&rid, is_recurring(&doc) && !row.is_exception()) {
        check_occurrence(&doc, r.ts(env), env)?;
    }
    let ics = counter_proposal(&doc, &me, start, end, comment, rid.as_ref(), env)?
        .ok_or_else(not_invited)?;
    Ok(Message {
        to: organizer,
        subject: format!("New time proposed: {}", summary_of(&doc)),
        ics,
        method: "COUNTER",
        status: None,
        comment: comment.map(str::to_owned),
    })
}

/// An undo or redo of an event write (SyncbackEventTask `restore`): `snapshot` re-sent over
/// `current`, the copy stored now, with the overrides added since put back to the series'
/// own occurrences and SEQUENCE no lower than `current`'s.
pub fn restore(snapshot: &str, current: &str, env: &Env) -> Result<String> {
    let before = Doc::parse(snapshot)?;
    let now = Doc::parse(current)?;
    let mut reverted = edit::revert_added_overrides(&before, &now, env)?;
    edit::match_sequence(&mut reverted, &now, env);
    Ok(reverted.to_ics())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env::new(chrono_tz::America::Chicago, 1_767_225_600)
    }

    fn weekly() -> EventRow {
        EventRow {
            id: "e1".into(),
            account_id: "a".into(),
            calendar_id: "c".into(),
            icsuid: "w@t".into(),
            ics: "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:w@t\r\nDTSTART;TZID=America/Chicago:20260302T090000\r\nDTEND;TZID=America/Chicago:20260302T100000\r\nRRULE:FREQ=WEEKLY;COUNT=10\r\nSUMMARY:Standup\r\nORGANIZER:mailto:me@x.com\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:me@x.com\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:g@x.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into(),
            ..Default::default()
        }
    }

    const MAR9: i64 = 1_773_064_800;

    #[test]
    fn this_occurrence_then_again_edits_in_place() {
        let env = env();
        let row = weekly();
        let target = Target {
            row: &row,
            occurrence: Some(MAR9),
        };
        let patch = Patch {
            start: Some(MAR9 + 3600),
            end: Some(MAR9 + 7200),
            ..Default::default()
        };
        let w = plan_update(&target, Scope::This, &patch, None, &env).unwrap();
        let Write::Update { ics, .. } = &w[0] else {
            panic!()
        };
        let row2 = EventRow {
            ics: ics.clone(),
            ..row.clone()
        };
        let target = Target {
            row: &row2,
            occurrence: Some(MAR9),
        };
        let patch = Patch {
            summary: Some("Moved standup".into()),
            ..Default::default()
        };
        let w = plan_update(&target, Scope::This, &patch, None, &env).unwrap();
        let Write::Update { ics, .. } = &w[0] else {
            panic!()
        };
        let doc = Doc::parse(ics).unwrap();
        assert_eq!(doc.vevents().count(), 2);
        let ex = doc.vevents().nth(1).unwrap();
        assert_eq!(ex.text("SUMMARY"), "Moved standup");
        assert_eq!(ex.value("DTSTART"), Some("20260309T100000"));
        assert_eq!(ex.value("SEQUENCE"), Some("2"));
        assert_eq!(doc.master().unwrap().value("SEQUENCE"), None);
    }

    #[test]
    fn following_splits_the_series() {
        let env = env();
        let row = weekly();
        let target = Target {
            row: &row,
            occurrence: Some(MAR9),
        };
        let patch = Patch {
            summary: Some("New standup".into()),
            ..Default::default()
        };
        let w = plan_update(&target, Scope::Following, &patch, None, &env).unwrap();
        assert_eq!(w.len(), 2);
        let (
            Write::Update { ics: old, .. },
            Write::Create {
                ics: new,
                calendar_id,
            },
        ) = (&w[0], &w[1])
        else {
            panic!("{w:?}")
        };
        assert_eq!(calendar_id, "c");
        let old = Doc::parse(old).unwrap();
        assert_eq!(
            old.master().unwrap().value("RRULE"),
            Some("FREQ=WEEKLY;UNTIL=20260309T135959Z")
        );
        let new = Doc::parse(new).unwrap();
        let m = new.master().unwrap();
        assert_ne!(m.value("UID"), Some("w@t"));
        assert_eq!(m.value("RRULE"), Some("FREQ=WEEKLY;COUNT=9"));
        assert_eq!(m.value("DTSTART"), Some("20260309T090000"));
        assert_eq!(m.text("SUMMARY"), "New standup");
        let occs = crate::expand::expand(&new, None, 0, 2_000_000_000, &env).unwrap();
        assert_eq!(occs.len(), 9);
    }

    #[test]
    fn delete_one_and_all() {
        let env = env();
        let row = weekly();
        let target = Target {
            row: &row,
            occurrence: Some(MAR9),
        };
        let del = Patch {
            deleted: true,
            ..Default::default()
        };
        let w = plan_update(&target, Scope::This, &del, None, &env).unwrap();
        let Write::Update { ics, .. } = &w[0] else {
            panic!()
        };
        assert!(ics.contains("EXDATE;TZID=America/Chicago:20260309T090000\r\n"));
        let w = plan_update(&target, Scope::All, &del, None, &env).unwrap();
        assert_eq!(
            w,
            vec![Write::Destroy {
                event_id: "e1".into()
            }]
        );
    }

    #[test]
    fn rsvp_one_occurrence_and_reply() {
        let env = env();
        let mut row = weekly();
        row.ics = row
            .ics
            .replace("ORGANIZER:mailto:me@x.com", "ORGANIZER:mailto:boss@x.com");
        let r = Rsvp {
            stored: Some(&row),
            writable: true,
            status: "declined",
            occurrence: Some(Rid::At(MAR9)),
            reply: true,
            write: true,
            ..Default::default()
        };
        let plan = plan_rsvp(&r, |e| e == "g@x.com", &env).unwrap();
        assert_eq!(plan.me, "g@x.com");
        let (Write::Update { ics, .. }, answer) = plan.write.unwrap() else {
            panic!()
        };
        let doc = Doc::parse(&ics).unwrap();
        assert_eq!(doc.vevents().count(), 2);
        assert!(answer["occurrenceIcs"].is_string());
        let msg = plan.message.unwrap();
        assert_eq!(msg.to, "boss@x.com");
        assert_eq!(msg.subject, "Declined: Standup");
        assert!(msg.ics.contains("METHOD:REPLY\r\n"));
        assert!(msg
            .ics
            .contains("RECURRENCE-ID;TZID=America/Chicago:20260309T090000\r\n"));
        assert!(!msg.ics.contains("me@x.com\r\n") || msg.ics.contains("ORGANIZER"));
    }

    #[test]
    fn counter_names_the_occurrence() {
        let env = env();
        let mut row = weekly();
        row.ics = row
            .ics
            .replace("ORGANIZER:mailto:me@x.com", "ORGANIZER:mailto:boss@x.com");
        let msg = plan_counter(
            &row,
            MAR9 + 3600,
            MAR9 + 7200,
            Some("later?"),
            Some(Rid::At(MAR9)),
            |e| e == "g@x.com",
            &env,
        )
        .unwrap();
        assert_eq!(msg.method, "COUNTER");
        assert!(msg.ics.contains("METHOD:COUNTER\r\n"));
        assert!(msg.ics.contains("DTSTART:20260309T150000Z\r\n"));
        assert!(msg.ics.contains("COMMENT:later?\r\n"));
        assert!(!msg.ics.contains("RRULE"));
    }

    #[test]
    fn an_occurrence_start_must_name_an_occurrence() {
        let env = env();
        let row = weekly();
        let key = |r: Result<Vec<Write>>| r.unwrap_err().code;
        let patch = Patch {
            summary: Some("x".into()),
            ..Default::default()
        };
        let del = Patch {
            deleted: true,
            ..Default::default()
        };
        // An hour off the slot (a zone read wrong), before the series, after its COUNT.
        for bad in [MAR9 + 3600, MAR9 - 7 * 86400 - 3600, MAR9 + 70 * 86400] {
            let target = Target {
                row: &row,
                occurrence: Some(bad),
            };
            for scope in [Scope::This, Scope::Following, Scope::All] {
                assert_eq!(
                    key(plan_update(&target, scope, &patch, None, &env)),
                    "not-an-occurrence"
                );
                assert_eq!(
                    key(plan_update(&target, scope, &del, None, &env)),
                    "not-an-occurrence"
                );
            }
        }

        // Moved an hour later: the override is named by its slot, not by its new start.
        let target = Target {
            row: &row,
            occurrence: Some(MAR9),
        };
        let moved = Patch {
            start: Some(MAR9 + 3600),
            end: Some(MAR9 + 7200),
            ..Default::default()
        };
        let Write::Update { ics, .. } =
            &plan_update(&target, Scope::This, &moved, None, &env).unwrap()[0]
        else {
            panic!()
        };
        let row2 = EventRow {
            ics: ics.clone(),
            ..row.clone()
        };
        let at = |occ| Target {
            row: &row2,
            occurrence: Some(occ),
        };
        assert!(plan_update(&at(MAR9), Scope::This, &patch, None, &env).is_ok());
        assert_eq!(
            key(plan_update(
                &at(MAR9 + 3600),
                Scope::This,
                &patch,
                None,
                &env
            )),
            "not-an-occurrence"
        );

        // A deleted occurrence is gone.
        let Write::Update { ics, .. } =
            &plan_update(&at(MAR9 + 7 * 86400), Scope::This, &del, None, &env).unwrap()[0]
        else {
            panic!()
        };
        let row3 = EventRow {
            ics: ics.clone(),
            ..row.clone()
        };
        let target = Target {
            row: &row3,
            occurrence: Some(MAR9 + 7 * 86400),
        };
        assert_eq!(
            key(plan_update(&target, Scope::This, &patch, None, &env)),
            "not-an-occurrence"
        );

        // An RSVP or counter for one occurrence alike.
        let mut invited = weekly();
        invited.ics = invited
            .ics
            .replace("ORGANIZER:mailto:me@x.com", "ORGANIZER:mailto:boss@x.com");
        let r = Rsvp {
            stored: Some(&invited),
            writable: true,
            status: "ACCEPTED",
            occurrence: Some(Rid::At(MAR9 + 60)),
            reply: true,
            write: true,
            ..Default::default()
        };
        assert_eq!(
            plan_rsvp(&r, |e| e == "g@x.com", &env).unwrap_err().code,
            "not-an-occurrence"
        );
        let err = plan_counter(
            &invited,
            MAR9,
            MAR9 + 3600,
            None,
            Some(Rid::At(MAR9 + 60)),
            |e| e == "g@x.com",
            &env,
        )
        .unwrap_err();
        assert_eq!(err.code, "not-an-occurrence");
    }

    #[test]
    fn rsvp_comment_goes_with_the_reply() {
        let env = env();
        let mut row = weekly();
        row.ics = row
            .ics
            .replace("ORGANIZER:mailto:me@x.com", "ORGANIZER:mailto:boss@x.com");
        let r = Rsvp {
            stored: Some(&row),
            writable: true,
            status: "TENTATIVE",
            comment: Some("Might be late"),
            reply: true,
            write: true,
            ..Default::default()
        };
        let plan = plan_rsvp(&r, |e| e == "g@x.com", &env).unwrap();
        assert_eq!(
            plan.message.unwrap().comment.as_deref(),
            Some("Might be late")
        );
        let from_mail = Rsvp {
            invite: Some(OCCURRENCE_INVITE),
            ..r
        };
        let plan = plan_rsvp(&from_mail, |e| e == "g@x.com", &env).unwrap();
        assert_eq!(
            plan.message.unwrap().comment.as_deref(),
            Some("Might be late")
        );
    }

    // An invitation to one moved occurrence, answered from the mail view (event-header-spec's
    // cases): the REPLY is that occurrence alone, and the answer lands on its override.
    const OCCURRENCE_INVITE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:w@t\r\nRECURRENCE-ID:20260309T140000Z\r\nDTSTART:20260309T150000Z\r\nDTEND:20260309T151500Z\r\nSUMMARY:Standup\r\nORGANIZER:mailto:boss@x.com\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:boss@x.com\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:g@x.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn my_partstat(vevent: &str) -> &str {
        let line = vevent
            .lines()
            .find(|l| l.starts_with("ATTENDEE") && l.ends_with("g@x.com"))
            .unwrap();
        line.split(';')
            .find_map(|p| p.strip_prefix("PARTSTAT="))
            .and_then(|p| p.split(':').next())
            .unwrap()
    }

    #[test]
    fn rsvp_from_an_emailed_occurrence() {
        let env = env();
        let mut row = weekly();
        row.ics = row
            .ics
            .replace("ORGANIZER:mailto:me@x.com", "ORGANIZER:mailto:boss@x.com");
        let r = Rsvp {
            stored: Some(&row),
            writable: true,
            invite: Some(OCCURRENCE_INVITE),
            status: "ACCEPTED",
            reply: true,
            write: true,
            ..Default::default()
        };
        let plan = plan_rsvp(&r, |e| e == "g@x.com", &env).unwrap();
        let msg = plan.message.unwrap();
        assert_eq!(msg.to, "boss@x.com");
        assert_eq!(msg.ics.matches("BEGIN:VEVENT").count(), 1);
        assert!(msg.ics.contains("RECURRENCE-ID:20260309T140000Z\r\n"));
        assert!(!msg.ics.contains("RRULE"));

        // Our copy had no override for it: one is added, answered, and the series is not.
        let (Write::Update { ics, .. }, _) = plan.write.unwrap() else {
            panic!()
        };
        let vevents: Vec<&str> = ics.split("BEGIN:VEVENT").skip(1).collect();
        assert_eq!(vevents.len(), 2);
        assert_eq!(my_partstat(vevents[0]), "NEEDS-ACTION");
        assert!(vevents[1].contains("RECURRENCE-ID"));
        assert!(vevents[1].contains("DTSTART:20260309T150000Z\r\n"));
        assert_eq!(my_partstat(vevents[1]), "ACCEPTED");

        // Answered again: the override is changed, not added twice.
        let answered = EventRow { ics, ..row.clone() };
        let again = Rsvp {
            stored: Some(&answered),
            status: "DECLINED",
            ..r
        };
        let (Write::Update { ics, .. }, _) = plan_rsvp(&again, |e| e == "g@x.com", &env)
            .unwrap()
            .write
            .unwrap()
        else {
            panic!()
        };
        let vevents: Vec<&str> = ics.split("BEGIN:VEVENT").skip(1).collect();
        assert_eq!(vevents.len(), 2);
        assert_eq!(my_partstat(vevents[0]), "NEEDS-ACTION");
        assert_eq!(my_partstat(vevents[1]), "DECLINED");
    }

    #[test]
    fn rsvp_adds_an_invitation_we_have_no_copy_of_only_when_attending() {
        let env = env();
        let invite = OCCURRENCE_INVITE.replace("RECURRENCE-ID:20260309T140000Z\r\n", "");
        let r = Rsvp {
            invite: Some(&invite),
            add_to: Some("c"),
            status: "DECLINED",
            reply: true,
            write: true,
            ..Default::default()
        };
        let plan = plan_rsvp(&r, |e| e == "g@x.com", &env).unwrap();
        assert!(plan.write.is_none());
        assert!(plan.message.is_some());

        let accept = Rsvp {
            status: "ACCEPTED",
            ..r
        };
        let plan = plan_rsvp(&accept, |e| e == "g@x.com", &env).unwrap();
        let (Write::Create { calendar_id, ics }, _) = plan.write.unwrap() else {
            panic!()
        };
        assert_eq!(calendar_id, "c");
        assert!(!ics.contains("METHOD"));
        assert_eq!(my_partstat(&ics), "ACCEPTED");
    }

    // Undo and redo (change-mail-tasks-spec's SyncbackEventTask cases): a snapshot re-sent over
    // the stored copy.
    fn series(overrides: &[(&str, &str, u32)], exdate: &str, summary: &str) -> String {
        let mut out = String::from(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:s@t\r\nDTSTART:20261008T130000Z\r\nDTEND:20261008T140000Z\r\nRRULE:FREQ=DAILY\r\n",
        );
        if !exdate.is_empty() {
            out.push_str(&format!("EXDATE:{exdate}\r\n"));
        }
        out.push_str(&format!(
            "SUMMARY:{summary}\r\nSEQUENCE:0\r\nEND:VEVENT\r\n"
        ));
        for (rid, start, seq) in overrides {
            out.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:s@t\r\nRECURRENCE-ID:{rid}\r\nDTSTART:{start}\r\nSUMMARY:Test series\r\nSEQUENCE:{seq}\r\nEND:VEVENT\r\n"
            ));
        }
        out.push_str("END:VCALENDAR\r\n");
        out
    }

    fn single(start: &str, seq: u32) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:u@t\r\nDTSTART:20261008T{start}00Z\r\nDTEND:20261008T140000Z\r\nSEQUENCE:{seq}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n")
    }

    fn vevents(ics: &str) -> Vec<(Option<String>, String, String)> {
        Doc::parse(ics)
            .unwrap()
            .vevents()
            .map(|v| {
                (
                    v.value("RECURRENCE-ID").map(str::to_owned),
                    v.text("SUMMARY"),
                    v.value("SEQUENCE").unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn an_undo_or_redo_is_never_below_the_stored_sequence() {
        let env = env();
        let (original, moved) = (single("1300", 1), single("1315", 2));
        let undo = restore(&original, &moved, &env).unwrap();
        assert!(undo.contains("DTSTART:20261008T130000Z"), "{undo}");
        assert!(undo.contains("SEQUENCE:2"), "{undo}");
        let redo = restore(&moved, &undo, &env).unwrap();
        assert!(
            redo.contains("DTSTART:20261008T131500Z") && redo.contains("SEQUENCE:2"),
            "{redo}"
        );
        // Two moves undone in turn: the first undo goes out at the later move's SEQUENCE.
        let undo_second = restore(&moved, &single("1330", 3), &env).unwrap();
        let undo_first = restore(&original, &undo_second, &env).unwrap();
        assert!(
            undo_first.contains("DTSTART:20261008T130000Z"),
            "{undo_first}"
        );
        assert!(undo_first.contains("SEQUENCE:3"), "{undo_first}");
    }

    #[test]
    fn an_undo_puts_back_an_override_added_since() {
        // Rename the series, then move one occurrence; undoing both brings that occurrence back
        // to the old title, though the rename never touched it.
        let env = env();
        let rid = "20261009T130000Z";
        let original = series(&[], "", "Test series");
        let renamed = series(&[], "", "Renamed series");
        let moved = series(&[(rid, "20261009T150000Z", 1)], "", "Renamed series");
        let undo_move = restore(&renamed, &moved, &env).unwrap();
        let undo_rename = restore(&original, &undo_move, &env).unwrap();
        let got = vevents(&undo_rename);
        assert_eq!(got[0].1, "Test series", "{undo_rename}");
        assert!(
            got.iter()
                .skip(1)
                .all(|v| v.0.as_deref() == Some(rid) && v.1 == "Test series"),
            "{undo_rename}"
        );
    }
}
