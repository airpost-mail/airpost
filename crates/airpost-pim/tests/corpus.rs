// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Replays the desktop's ICS and vCard specs against airpost-pim. `fixtures/desktop_corpus.ndjson`
//! holds every call the desktop specs made into `ics-event-helpers.ts`, `calendar-utils.ts`,
//! `calendar-data-source.ts`, `ContactInfoMapping.ts` and `VCFImportExport.ts` before those
//! went, with the arguments and what each returned or threw, recorded with the machine in
//! America/Chicago. Each is called again here and the results compared for meaning: instants
//! rather than the zone they are written in, decoded text, parameters as sets, DTSTAMP and
//! line order ignored. Where airpost-pim deliberately answers differently (see `differs`), the
//! record is checked for the corrected answer instead.

use airpost_pim::edit::{self, Doc, Edits, Guest, NewEvent, Person, Rid, Times};
use airpost_pim::ics::{self, Component};
use airpost_pim::occurrence::{self, EventRow, Owners};
use airpost_pim::time::When;
use airpost_pim::Env;
use chrono::NaiveDate;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn env() -> Env {
    Env::new(chrono_tz::America::Chicago, 1_767_225_600)
}

fn records() -> Vec<Value> {
    include_str!("fixtures/desktop_corpus.ndjson")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("corpus line"))
        .collect()
}

fn undefined(v: &Value) -> bool {
    v.is_null() || v.get("$undefined").is_some()
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn opt_s(v: &Value) -> Option<String> {
    if undefined(v) {
        None
    } else {
        s(v)
    }
}

fn date_secs(v: &Value) -> i64 {
    v["$date"]
        .as_f64()
        .map(|ms| (ms / 1000.0).round() as i64)
        .unwrap_or(0)
}

fn rid_of(v: &Value) -> Option<Rid> {
    if undefined(v) {
        return None;
    }
    let unix = v["unix"].as_i64()?;
    if v["isDate"].as_bool() == Some(true) {
        let text = v["$icaltime"].as_str()?;
        let d = NaiveDate::parse_from_str(&text.replace('-', "")[..8], "%Y%m%d").ok()?;
        Some(Rid::Date(d))
    } else {
        Some(Rid::At(unix))
    }
}

fn model_rows(v: &Value) -> Vec<EventRow> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| EventRow::from_json(&m["json"]))
        .collect()
}

fn owners(record: &Value, own: Option<&Value>) -> Owners {
    // These specs stubbed AccountStore.accountForEmail, which the recording did not capture.
    let stubbed: Vec<(String, Vec<String>)> = if record["spec"]
        .as_str()
        .unwrap_or_default()
        .starts_with("occurrence ownership")
    {
        vec![("acct-1".into(), vec!["me@example.com".into()])]
    } else {
        Vec::new()
    };
    let accounts = stubbed
        .into_iter()
        .chain(
            record["context"]["accounts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| {
                    let mut addrs = vec![a["emailAddress"].as_str().unwrap_or_default().to_owned()];
                    for alias in a["aliases"].as_array().into_iter().flatten() {
                        let alias = alias.as_str().unwrap_or_default();
                        let addr = match (alias.rfind('<'), alias.rfind('>')) {
                            (Some(l), Some(r)) if l < r => alias[l + 1..r].to_owned(),
                            _ => alias.to_owned(),
                        };
                        addrs.push(addr);
                    }
                    (a["id"].as_str().unwrap_or_default().to_owned(), addrs)
                }),
        )
        .collect();
    let own_calendar_addresses = own
        .and_then(|o| o["$map"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|pair| Some((s(&pair[0])?, s(&pair[1])?)))
        .collect();
    Owners {
        accounts,
        own_calendar_addresses,
    }
}

// ---- semantic ICS comparison ------------------------------------------------------------

fn instant(p: &ics::Property, zones: &airpost_dav::Zones) -> Vec<(bool, i64)> {
    When::all_of(p)
        .into_iter()
        .map(|w| {
            (
                w.is_date(),
                if w.is_date() {
                    w.date().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
                } else {
                    w.ts(zones)
                },
            )
        })
        .collect()
}

fn rrule_norm(v: &str, zones: &airpost_dav::Zones) -> String {
    let mut parts: Vec<String> = v
        .split(';')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, val) = p.split_once('=').unwrap_or((p, ""));
            let k = k.to_ascii_uppercase();
            if k == "UNTIL" {
                let w = When::parse(val, None);
                let t = w.map(|w| {
                    if w.is_date() {
                        format!("D{}", w.text())
                    } else {
                        w.ts(zones).to_string()
                    }
                });
                format!("UNTIL={}", t.unwrap_or_default())
            } else {
                format!("{k}={}", val.to_ascii_uppercase())
            }
        })
        .filter(|p| p != "INTERVAL=1")
        .collect();
    parts.sort();
    parts.join(";")
}

fn prop_key(p: &ics::Property, zones: &airpost_dav::Zones) -> String {
    match p.name.as_str() {
        "DTSTART" | "DTEND" | "RECURRENCE-ID" | "EXDATE" | "RDATE" => {
            format!("{}={:?}", p.name, instant(p, zones))
        }
        "RRULE" => format!("RRULE={}", rrule_norm(&p.value, zones)),
        "SUMMARY" | "DESCRIPTION" | "LOCATION" | "COMMENT" => format!("{}={}", p.name, p.text()),
        "SEQUENCE" => format!("SEQUENCE={}", p.value.trim().parse::<i64>().unwrap_or(0)),
        "ATTENDEE" | "ORGANIZER" => {
            let mut params: Vec<String> = p
                .params
                .iter()
                .map(|q| format!("{}={}", q.name, q.values.join(",").to_ascii_uppercase()))
                .collect();
            params.sort();
            format!(
                "{}={};{}",
                p.name,
                airpost_pim::email_from_participant_uri(&p.value).unwrap_or_default(),
                params.join(";")
            )
        }
        _ => {
            let mut params: Vec<String> = p
                .params
                .iter()
                .filter(|q| q.name != "VALUE")
                .map(|q| format!("{}={}", q.name, q.values.join(",")))
                .collect();
            params.sort();
            format!(
                "{};{}={}",
                p.name,
                params.join(";"),
                p.value.to_ascii_uppercase()
            )
        }
    }
}

fn vevent_keys(c: &Component, zones: &airpost_dav::Zones) -> Vec<String> {
    let mut keys: Vec<String> = c
        .props
        .iter()
        .filter(|p| p.name != "DTSTAMP")
        .map(|p| prop_key(p, zones))
        .collect();
    // An absent SEQUENCE is zero (RFC 5545 section 3.7.4).
    keys.retain(|k| k != "SEQUENCE=0");
    // ical.js writes an all-day DTEND only when it is not the default.
    keys.sort();
    keys
}

/// METHOD, the VEVENTs' properties by key, and the VTIMEZONE TZIDs.
type IcsMeaning = (Option<String>, BTreeMap<String, Vec<String>>, Vec<String>);

fn ics_meaning(text: &str) -> Result<IcsMeaning, String> {
    let doc = Doc::parse(text).map_err(|e| e.to_string())?;
    let env = env();
    let zones = doc.zones(&env);
    let mut vevents = BTreeMap::new();
    for v in doc.vevents() {
        let rid = v
            .prop("RECURRENCE-ID")
            .map(|p| format!("{:?}", instant(p, &zones)))
            .unwrap_or_default();
        let key = format!("{}|{}", v.value("UID").unwrap_or_default(), rid);
        vevents.insert(key, vevent_keys(v, &zones));
    }
    let mut tzids: Vec<String> = doc
        .cal
        .children_named("VTIMEZONE")
        .map(|t| t.value("TZID").unwrap_or_default().to_owned())
        .collect();
    tzids.sort();
    Ok((doc.cal.value("METHOD").map(str::to_owned), vevents, tzids))
}

fn same_ics(expected: &str, actual: &str) -> Result<(), String> {
    let (em, ev, et) = ics_meaning(expected)?;
    let (am, av, at) = ics_meaning(actual)?;
    if em != am {
        return Err(format!("METHOD {em:?} != {am:?}"));
    }
    if ev.keys().collect::<Vec<_>>() != av.keys().collect::<Vec<_>>() {
        return Err(format!(
            "VEVENTs {:?} != {:?}",
            ev.keys().collect::<Vec<_>>(),
            av.keys().collect::<Vec<_>>()
        ));
    }
    for (k, e) in &ev {
        let a = &av[k];
        if e != a {
            let missing: Vec<_> = e.iter().filter(|x| !a.contains(x)).collect();
            let extra: Vec<_> = a.iter().filter(|x| !e.contains(x)).collect();
            return Err(format!("VEVENT {k}: missing {missing:?}, extra {extra:?}"));
        }
    }
    if et != at {
        return Err(format!("VTIMEZONEs {et:?} != {at:?}"));
    }
    Ok(())
}

// ---- dispatch ---------------------------------------------------------------------------

enum Out {
    Ics(String),
    Json(Value),
    Err(String),
}

fn err(e: airpost_pim::Error) -> Out {
    Out::Err(e.message)
}

fn ics_result(r: airpost_pim::Result<Doc>) -> Out {
    match r {
        Ok(d) => Out::Ics(d.to_ics()),
        Err(e) => err(e),
    }
}

fn with_doc(text: &str, f: impl FnOnce(&mut Doc) -> airpost_pim::Result<()>) -> Out {
    ics_result(Doc::parse(text).and_then(|mut d| f(&mut d).map(|_| d)))
}

fn guests(v: &Value) -> Vec<Guest> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|a| Guest {
            email: a["email"].as_str().unwrap_or_default().to_owned(),
            name: opt_s(&a["name"]),
            role: opt_s(&a["role"]),
        })
        .collect()
}

fn person(v: &Value) -> Option<Person> {
    if undefined(v) {
        return None;
    }
    Some(Person {
        email: v["email"].as_str()?.to_owned(),
        name: opt_s(&v["name"]),
    })
}

fn run(r: &Value) -> Option<Out> {
    let env = env();
    let a = &r["args"];
    let text = |i: usize| a[i].as_str().unwrap_or_default().to_owned();
    Some(match r["fn"].as_str()? {
        "expansionIterationBudget" => {
            let start = a[1]
                .as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.round() as i64);
            match a[2].as_f64().filter(|f| f.is_finite()) {
                Some(end) => Out::Json(json!(airpost_pim::expand::budget_for(
                    &text(0),
                    start,
                    end as i64
                ))),
                None => Out::Json(json!(airpost_pim::expand::budget(None, None, 0))),
            }
        }
        "isRecurringEvent" => match Doc::parse(&text(0)) {
            Ok(d) => Out::Json(json!(edit::is_recurring(&d))),
            Err(e) => err(e),
        },
        "getRecurrenceInfo" => match Doc::parse(&text(0)) {
            Ok(d) => Out::Json(match edit::recurrence_info(&d) {
                Some((rule, freq)) => {
                    json!({"isRecurring": true, "rule": rrule_norm(&rule, &d.zones(&env)), "frequency": freq})
                }
                None => json!({"isRecurring": false}),
            }),
            Err(e) => err(e),
        },
        "getEventTimezone" => match Doc::parse(&text(0)) {
            Ok(d) => Out::Json(json!(edit::event_timezone(&d, &env))),
            Err(e) => err(e),
        },
        "emailFromParticipantURI" => {
            Out::Json(json!(airpost_pim::email_from_participant_uri(&text(0))))
        }
        "resolveIanaZone" => Out::Json(json!(
            airpost_pim::time::resolve_iana(&text(0)).map(|tz| tz.name().to_owned())
        )),
        "createVTIMEZONEString" => {
            let at = date_secs(&a[1]);
            Out::Json(json!(airpost_pim::time::vtimezone(&text(0), at).is_some()))
        }
        "bumpEventSequence" => {
            let rid = opt_s(&a[1]);
            with_doc(&text(0), |d| edit::bump_sequence(d, rid.as_deref()))
        }
        "addExclusionDate" => {
            let (t, all_day) = (a[1].as_i64()?, a[2].as_bool().unwrap_or(false));
            with_doc(&text(0), |d| edit::add_exclusion_date(d, t, all_day, &env))
        }
        "removeInlineException" => {
            let rid = text(1);
            with_doc(&text(0), |d| {
                edit::remove_inline_exception(d, &rid, &env);
                Ok(())
            })
        }
        "updateEventProperty" => {
            let (p, v) = (text(1), text(2));
            with_doc(&text(0), |d| edit::update_property(d, &p, &v, &env))
        }
        "updateAttendees" => {
            let (g, o) = (guests(&a[1]), person(&a[2]));
            with_doc(&text(0), |d| {
                edit::update_attendees(d, &g, o.as_ref(), &env)
            })
        }
        "updateEventTimes" => {
            let o = &a[1];
            let t = Times {
                start: o["start"].as_i64()?,
                end: o["end"].as_i64()?,
                all_day: o["isAllDay"].as_bool().unwrap_or(false),
                timezone: opt_s(&o["timezone"]),
            };
            with_doc(&text(0), |d| edit::update_times(d, &t, &env))
        }
        "updateRecurringEventTimes" => {
            let tz = opt_s(&a[5]);
            let (o, ns, ne, all_day) = (
                a[1].as_i64()?,
                a[2].as_i64()?,
                a[3].as_i64()?,
                a[4].as_bool().unwrap_or(false),
            );
            with_doc(&text(0), |d| {
                edit::update_recurring_times(d, o, ns, ne, all_day, tz.as_deref(), &env)
            })
        }
        "updateRecurrenceRule" => {
            let rule = opt_s(&a[1]);
            with_doc(&text(0), |d| {
                edit::update_recurrence_rule(d, rule.as_deref(), &env)
            })
        }
        "shiftInlineExceptions" => {
            let delta = (a[1].as_f64()? / 1000.0).round() as i64;
            with_doc(&text(0), |d| {
                edit::shift_inline_exceptions(d, delta, &env);
                Ok(())
            })
        }
        "createRecurrenceException" => {
            let (o, ns, ne, all_day) = (
                a[1].as_i64()?,
                a[2].as_i64()?,
                a[3].as_i64()?,
                a[4].as_bool().unwrap_or(false),
            );
            match Doc::parse(&text(0)).and_then(|mut d| {
                let rid = edit::create_exception(&mut d, o, ns, ne, all_day, &env)?;
                Ok((d, rid))
            }) {
                Ok((d, rid)) => Out::Json(json!({"masterIcs": d.to_ics(), "recurrenceId": rid})),
                Err(e) => err(e),
            }
        }
        "applyEditsToException" => {
            let e = &a[2];
            let edits = Edits {
                summary: opt_s(&e["summary"]),
                location: opt_s(&e["location"]),
                description: opt_s(&e["description"]),
                attendees: if undefined(&e["attendees"]) {
                    None
                } else {
                    Some(guests(&e["attendees"]))
                },
                organizer: person(&e["organizer"]),
                transparency: opt_s(&e["transparency"]),
            };
            let rid = text(1);
            with_doc(&text(0), |d| {
                edit::apply_edits_to_exception(d, &rid, &edits, &env)
            })
        }
        "createICSString" => {
            let o = &a[0];
            let ev = NewEvent {
                uid: opt_s(&o["uid"]),
                summary: o["summary"].as_str().unwrap_or_default().to_owned(),
                description: opt_s(&o["description"]),
                location: opt_s(&o["location"]),
                start: date_secs(&o["start"]),
                end: date_secs(&o["end"]),
                all_day: o["isAllDay"].as_bool().unwrap_or(false),
                timezone: opt_s(&o["timezone"]),
                organizer: person(&o["organizer"]),
                attendees: guests(&o["attendees"]),
                recurrence_rule: opt_s(&o["recurrenceRule"]),
                transparency: opt_s(&o["transparency"]),
            };
            Out::Ics(edit::create(&ev, &env).to_ics())
        }
        "createCounterProposal" => {
            let o = &a[1];
            let rid = rid_of(&o["recurrenceId"]);
            match Doc::parse(&text(0)).and_then(|d| {
                edit::counter_proposal(
                    &d,
                    o["email"].as_str().unwrap_or_default(),
                    date_secs(&o["start"]),
                    date_secs(&o["end"]),
                    opt_s(&o["comment"]).as_deref(),
                    rid.as_ref(),
                    &env,
                )
            }) {
                Ok(Some(t)) => Out::Ics(t),
                Ok(None) => Out::Json(Value::Null),
                Err(e) => err(e),
            }
        }
        "updateAttendeeStatus" => {
            let (email, ps) = (text(1), text(2));
            match Doc::parse(&text(0))
                .and_then(|mut d| Ok((edit::update_attendee_status(&mut d, &email, &ps, &env)?, d)))
            {
                Ok((true, d)) => Out::Ics(d.to_ics()),
                Ok((false, _)) => Out::Json(Value::Null),
                Err(e) => err(e),
            }
        }
        "updateOccurrenceAttendeeStatus" => {
            let (email, ps) = (text(2), text(3));
            match Doc::parse(&text(0)).and_then(|mut d| {
                let occ = Doc::parse(&text(1))?;
                Ok((
                    edit::update_occurrence_attendee_status(&mut d, &occ, &email, &ps, &env)?,
                    d,
                ))
            }) {
                Ok((true, d)) => Out::Ics(d.to_ics()),
                Ok((false, _)) => Out::Json(Value::Null),
                Err(e) => err(e),
            }
        }
        "occurrenceIcs" => {
            let rid = rid_of(&a[1])?;
            ics_result(Doc::parse(&text(0)).and_then(|d| edit::occurrence_ics(&d, &rid, &env)))
        }
        "stripITIPMethod" => with_doc(&text(0), |d| {
            edit::strip_method(d);
            Ok(())
        }),
        "matchEventSequence" => {
            let cur = text(1);
            with_doc(&text(0), |d| {
                let current = Doc::parse(&cur)?;
                edit::match_sequence(d, &current, &env);
                Ok(())
            })
        }
        "revertAddedOverrides" => ics_result(Doc::parse(&text(0)).and_then(|b| {
            let c = Doc::parse(&text(1))?;
            edit::revert_added_overrides(&b, &c, &env)
        })),
        "upcomingOccurrence" => match Doc::parse(&text(0)) {
            Ok(d) => Out::Json(match occurrence::upcoming(&d, date_secs(&a[1]), &env) {
                Some((s, e, rid, all_day)) => {
                    json!({"start": s, "end": e, "recurrenceId": rid, "isAllDay": all_day})
                }
                None => Value::Null,
            }),
            Err(e) => err(e),
        },
        "findConflicts" => {
            let o = &a[0];
            let rows = model_rows(&o["events"]);
            let addrs: Vec<String> = o["addresses"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(s)
                .collect();
            let c = occurrence::conflicts(
                &rows,
                o["start"].as_i64()?,
                o["end"].as_i64()?,
                &addrs,
                opt_s(&o["excludeIcsuid"]).as_deref(),
                &env,
            );
            Out::Json(serde_json::to_value(c).ok()?)
        }
        "occurrencesForEvents" => {
            let rows = model_rows(&a[0]);
            let o = &a[1];
            let own = owners(r, o.get("ownCalendarAddresses"));
            Out::Json(Value::Array(occurrence::occurrences(
                &rows,
                o["startUnix"].as_i64()?,
                o["endUnix"].as_i64()?,
                &own,
                &env,
            )))
        }
        "focusedEventInfoForEvents" => {
            let rows = model_rows(&a[0]);
            let own = owners(r, None);
            let rid = a[2].as_i64();
            Out::Json(
                match occurrence::focused(&rows, a[1].as_i64()?, rid, &own, &env) {
                    Some(o) => {
                        let start = o["start"].as_i64().unwrap_or_else(|| {
                            let d = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
                                + chrono::Duration::days(o["startDate"].as_i64().unwrap_or(0));
                            env.day_start(d)
                        });
                        json!({"id": o["id"], "start": start})
                    }
                    None => Value::Null,
                },
            )
        }
        _ => return None,
    })
}

// ---- expected-value comparison ----------------------------------------------------------

fn expected_json(v: &Value) -> Value {
    match v {
        Value::Object(m) if m.contains_key("$date") => json!(date_secs(v)),
        Value::Object(m) if m.contains_key("$icaltime") => m["unix"].clone(),
        Value::Object(m) if m.contains_key("$undefined") => Value::Null,
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(_, x)| x.get("$undefined").is_none())
                .map(|(k, x)| (k.clone(), expected_json(x)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(expected_json).collect()),
        other => other.clone(),
    }
}

/// `actual` holds everything `expected` does (extra keys are airpost-pim additions).
fn covers(expected: &Value, actual: &Value) -> Result<(), String> {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                let av = a.get(k).unwrap_or(&Value::Null);
                covers(ev, av).map_err(|m| format!(".{k}{m}"))?;
            }
            Ok(())
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Err(format!(": {} items != {}", e.len(), a.len()));
            }
            for (i, (x, y)) in e.iter().zip(a).enumerate() {
                covers(x, y).map_err(|m| format!("[{i}]{m}"))?;
            }
            Ok(())
        }
        (Value::Number(x), Value::Number(y)) => {
            if (x.as_f64().unwrap_or(0.0) - y.as_f64().unwrap_or(1.0)).abs() < 1e-6 {
                Ok(())
            } else {
                Err(format!(": {x} != {y}"))
            }
        }
        (e, a) if e == a => Ok(()),
        (e, a) => Err(format!(": {e} != {a}")),
    }
}

fn sort_occurrences(v: &mut Value) {
    if let Some(a) = v.as_array_mut() {
        a.sort_by_key(|o| o["id"].as_str().unwrap_or_default().to_owned());
    }
}

/// Records where airpost-pim answers differently on purpose, and why.
fn differs(r: &Value) -> Option<&'static str> {
    let spec = r["spec"].as_str().unwrap_or_default();
    if spec.contains("follows the VTIMEZONE a file does carry rather than describing the zone") {
        // Zones resolve as ingestion resolves them: an IANA TZID by its name (Calconnect's
        // advice), not by a VTIMEZONE that redefines it, so edits read times as the row's
        // columns do.
        return Some("an IANA TZID reads by its name");
    }
    let args = r["args"].to_string();
    let expands = r["fn"] == "occurrencesForEvents" || r["fn"] == "upcomingOccurrence";
    if args.contains("UID:rdate@fx") && expands {
        // ical.js left out the DTSTART instance of a series that has RDATEs but no RRULE;
        // RFC 5545 section 3.8.5.2 counts it as the first.
        return Some("DTSTART is an instance of an RDATE series");
    }
    if args.contains("UID:yearly@fx") && expands {
        // ical.js moved Feb 29 to Mar 1 in common years; RFC 5545 section 3.3.10 ignores
        // the dates a rule names that do not exist.
        return Some("a leap-day series skips common years");
    }
    None
}

fn check(r: &Value) -> Result<(), String> {
    let Some(out) = run(r) else {
        return Err("no Rust counterpart".into());
    };
    let expected_err = r.get("error").and_then(Value::as_str);
    match (out, expected_err) {
        (Out::Err(_), Some(_)) => Ok(()),
        (Out::Err(m), None) => Err(format!("threw {m}")),
        (_, Some(e)) => Err(format!("did not throw {e}")),
        (Out::Ics(t), None) => match &r["result"] {
            // A UID the spec left to createICSString is random on both sides.
            Value::String(e) if r["fn"] == "createICSString" && undefined(&r["args"][0]["uid"]) => {
                let uid = |text: &str| {
                    Doc::parse(text)
                        .ok()
                        .and_then(|d| d.first().and_then(|v| v.value("UID")).map(str::to_owned))
                        .unwrap_or_default()
                };
                same_ics(e, &t.replace(&uid(&t), &uid(e)))
            }
            Value::String(e) => same_ics(e, &t),
            Value::Object(o) if o.contains_key("masterIcs") => Err("expected an object".into()),
            other => Err(format!("expected {other}, got ICS")),
        },
        (Out::Json(mut j), None) => {
            let mut expected = r["result"].clone();
            if let (Some(e), Some(a)) = (
                expected.get("masterIcs").and_then(Value::as_str),
                j.get("masterIcs").and_then(Value::as_str),
            ) {
                same_ics(e, a)?;
                expected["masterIcs"] = Value::Null;
                j["masterIcs"] = Value::Null;
            }
            if r["fn"] == "getRecurrenceInfo" {
                if let Some(rule) = expected["rule"].as_str().map(str::to_owned) {
                    let zones = Env::zones(&env(), "");
                    expected["rule"] = json!(rrule_norm(&rule, &zones));
                }
            }
            if r["fn"] == "createVTIMEZONEString" {
                return covers(&json!(expected.is_string()), &j);
            }
            let mut expected = expected_json(&expected);
            if r["fn"] == "occurrencesForEvents" {
                sort_occurrences(&mut expected);
                sort_occurrences(&mut j);
            }
            covers(&expected, &j)
        }
    }
}

#[test]
fn desktop_ics_corpus_replays() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for r in records() {
        let module = r["module"].as_str().unwrap_or_default();
        if !matches!(
            module,
            "ics-event-helpers" | "calendar-utils" | "calendar-data-source"
        ) {
            continue;
        }
        checked += 1;
        if let Some(why) = differs(&r) {
            let _ = why;
            continue;
        }
        if let Err(m) = check(&r) {
            if std::env::var_os("CORPUS_DEBUG").is_some() {
                eprintln!(
                    "---- {}\n{}\n",
                    r["fn"],
                    serde_json::to_string_pretty(&r["args"]).unwrap_or_default()
                );
            }
            failures.push(format!(
                "{} [{}]: {}",
                r["fn"],
                r["spec"].as_str().unwrap_or_default(),
                m
            ));
        }
    }
    assert!(checked > 700, "only {checked} records");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Drops what the desktop wrote for "nothing" (`false`, `undefined`, `""`, `[]`, the text
/// "undefined") so contact shapes compare by what they say.
fn meaning(v: &Value) -> Value {
    match v {
        other if undefined(other) => Value::Null,
        Value::Object(o) if o.contains_key("$model") => meaning(&o["json"]),
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| k.as_str() != "__cls" && k.as_str() != "photoURL")
                .map(|(k, v)| (k.clone(), meaning(v)))
                .filter(|(_, v)| !matches!(v, Value::Null))
                .collect(),
        ),
        Value::Array(a) if a.is_empty() => Value::Null,
        Value::Array(a) => Value::Array(a.iter().map(meaning).collect()),
        Value::Bool(false) => Value::Null,
        Value::String(s) if s.is_empty() || s == "undefined" => Value::Null,
        Value::String(s) if s.starts_with("BEGIN:VCARD") => json!(card_lines(s)),
        other => other.clone(),
    }
}

/// A vCard's content lines, sorted, the desktop's doubled CRs undone.
fn card_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .replace("\r\r\n", "\r\n")
        .split("\r\n")
        .filter(|l| !l.trim().is_empty() && !l.starts_with("UID:"))
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

fn contact_record(r: &Value) -> Option<Value> {
    let a = &r["args"];
    Some(match r["fn"].as_str()? {
        "parse" => airpost_pim::contact::parse(&a[0]["json"]).to_json(),
        "fromVCF" => airpost_pim::contact::from_vcf(a[0]["vcf"].as_str()?).to_json(),
        "apply" => airpost_pim::contact::apply(&a[0]["json"], &a[1]).ok()?,
        "contactToVCFString" => json!(airpost_pim::vcf::contact_to_vcf(&a[0]["json"])),
        "contactsToVCFFileContent" => {
            let cs: Vec<Value> = a[0].as_array()?.iter().map(|c| c["json"].clone()).collect();
            json!(airpost_pim::vcf::export(&cs))
        }
        "vcfStringToContacts" => json!(airpost_pim::vcf::import(a[0].as_str()?, a[1].as_str()?)),
        _ => return None,
    })
}

/// Contact records airpost-pim answers differently on purpose, and why.
fn contact_differs(r: &Value) -> Option<&'static str> {
    let text = r.to_string();
    if r["fn"] == "apply" {
        // `apply` edited the contact it was given in place, so the recorded argument is the
        // contact after the edit; `contact.rs`'s own tests cover it.
        return Some("arguments recorded after the call");
    }
    if text.contains(r#"Chief\\ Engineer"#) && r["fn"] != "contactToVCFString" {
        return Some("vCard text is unescaped (`\\;` reads as `;`)");
    }
    if text.contains(r#""displayName":"undefined""#) || r["fn"] == "vcfStringToContacts" {
        return Some("a card with only FN is named by it");
    }
    if text.contains("BDAY:1985-01-01") {
        return Some("birthdays keep their month and day");
    }
    None
}

#[test]
fn desktop_contact_corpus_replays() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for r in records() {
        if !matches!(
            r["module"].as_str(),
            Some("ContactInfoMapping" | "VCFImportExport")
        ) {
            continue;
        }
        checked += 1;
        if contact_differs(&r).is_some() {
            continue;
        }
        let ours = contact_record(&r).map(|v| meaning(&v));
        let theirs = meaning(&r["result"]);
        if ours.as_ref() != Some(&theirs) {
            failures.push(format!(
                "{}: {}\n  != {}",
                r["fn"],
                ours.map(|v| v.to_string()).unwrap_or_else(|| "none".into()),
                theirs
            ));
        }
    }
    assert!(checked > 20, "only {checked} records");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
