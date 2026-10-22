// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern calendars`, `tern calendar list|create|update|rsvp` and `tern contact
//! get|create|update|import|export`: the engine's `query` of Calendar, `events.*` and
//! `contacts.*`. The engine builds the ICS and vCards and queues the tasks that write them to
//! the server.

use chrono::{Local, Utc};
use serde_json::{json, Map, Value};
use tern_ipc::{feature, method, QueryResult};

use crate::app::{call, Ctx};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::mail_actions::{parse_until, require};
use crate::output;

/// The machine's IANA zone (`TZ`, else `/etc/localtime`), which floating times and all-day
/// events are read in.
pub fn local_tz_name() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':').trim();
        if !tz.is_empty() {
            return Some(tz.to_owned());
        }
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let p = target.to_string_lossy().into_owned();
    let i = p.find("zoneinfo/")?;
    Some(p[i + "zoneinfo/".len()..].to_owned())
}

fn when(s: &str, what: &str) -> Result<i64> {
    parse_until(s, Local::now())
        .map(|t| t.timestamp())
        .ok_or_else(|| {
            CliError::usage(format!(
                "can't read {what} {s:?}: give 30m, 2h, 3d, tomorrow, a date (2026-11-02) or a \
                 date and time (2026-11-02T14:30)"
            ))
        })
}

fn with_tz(mut params: Value) -> Value {
    if let Some(tz) = local_tz_name() {
        params["tz"] = json!(tz);
    }
    params
}

fn target(ctx: &Ctx, engines: &[Engine], need: &str, what: &str) -> Result<Target> {
    let t = discovery::one_target(engines, ctx.account())?;
    require(engines, &t, need, what)?;
    Ok(t)
}

fn print_writes(ctx: &Ctx, r: &Value, done: &str) {
    if ctx.json {
        output::json(r);
        return;
    }
    let writes = r["writes"].as_array().cloned().unwrap_or_default();
    for w in &writes {
        output::line(&format!(
            "{done}: {} {}",
            w["kind"].as_str().unwrap_or_default(),
            w["eventId"].as_str().unwrap_or("(new)")
        ));
    }
    if writes.is_empty() {
        output::line("Nothing to change.");
    }
}

/// `tern calendar list`: occurrences between two times, oldest first.
pub fn list(
    ctx: &Ctx,
    engines: &mut [Engine],
    from: Option<&str>,
    to: Option<&str>,
    calendars: &[String],
) -> Result<()> {
    let start = match from {
        Some(s) => when(s, "--from")?,
        None => Utc::now().timestamp(),
    };
    let end = match to {
        Some(s) => when(s, "--to")?,
        None => start + 7 * 86400,
    };
    if end < start {
        return Err(CliError::usage("--to is before --from"));
    }
    let targets = discovery::targets(engines, ctx.account())?;
    let mut all: Vec<Value> = Vec::new();
    for t in &targets {
        require(engines, t, feature::CALENDAR_EDITING, "expand events")?;
        let mut params = with_tz(json!({
            "accountIds": [t.account_id], "start": start, "end": end,
        }));
        if !calendars.is_empty() {
            params["calendarIds"] = json!(calendars);
        }
        let r: Value = call(engines, t, method::EVENTS_EXPAND, params)?;
        all.extend(r["occurrences"].as_array().cloned().unwrap_or_default());
    }
    let start_of = |o: &Value| {
        o["start"]
            .as_i64()
            .unwrap_or(o["startDate"].as_i64().unwrap_or(0) * 86400)
    };
    all.sort_by_key(start_of);
    if ctx.json {
        output::json(&json!({ "occurrences": all }));
        return Ok(());
    }
    if all.is_empty() {
        output::line("No events.");
        return Ok(());
    }
    let mut table = output::Table::new(vec!["WHEN", "TITLE", "EVENT"], 1);
    for o in &all {
        let at = if o["isAllDay"].as_bool() == Some(true) {
            format!("{} (all day)", output::list_date(start_of(o)))
        } else {
            output::full_date(start_of(o))
        };
        let title = output::one_line(o["title"].as_str().unwrap_or_default());
        let id = o["eventId"].as_str().unwrap_or_default().to_owned();
        table.row(vec![at, title, id], false);
    }
    table.print(&output::Style::detect());
    Ok(())
}

#[derive(Debug, Default)]
pub struct NewEvent<'a> {
    pub calendar: &'a str,
    pub title: &'a str,
    pub start: &'a str,
    pub end: Option<&'a str>,
    pub all_day: bool,
    pub location: Option<&'a str>,
    pub description: Option<&'a str>,
    pub guests: &'a [String],
    pub rrule: Option<&'a str>,
}

/// `tern calendars`: the stored calendars of every account (or --account's), through `query`.
pub fn calendars(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let mut rows: Vec<Value> = Vec::new();
    let mut asked: Vec<usize> = Vec::new();
    for t in &targets {
        if asked.contains(&t.engine) {
            continue;
        }
        asked.push(t.engine);
        require(engines, t, feature::QUERY, "list calendars")?;
        let accounts: Vec<&str> = targets
            .iter()
            .filter(|x| x.engine == t.engine)
            .map(|x| x.account_id.as_str())
            .collect();
        let r: QueryResult = call(
            engines,
            t,
            method::QUERY,
            json!({ "class": "Calendar", "accounts": accounts }),
        )?;
        for mut row in r.rows {
            let aid = row["aid"].as_str().unwrap_or_default().to_owned();
            if let Some(owner) = targets.iter().find(|x| x.account_id == aid) {
                row["email"] = json!(owner.email);
            }
            rows.push(row);
        }
    }
    let key = |c: &Value| {
        (
            c["email"].as_str().unwrap_or_default().to_owned(),
            c["order"].as_i64().unwrap_or(0),
            c["name"].as_str().unwrap_or_default().to_lowercase(),
        )
    };
    rows.sort_by_key(key);
    if ctx.json {
        output::json(&json!({ "calendars": rows }));
        return Ok(());
    }
    if rows.is_empty() {
        output::line("No calendars.");
        return Ok(());
    }
    let mut table = output::Table::new(vec!["NAME", "ACCOUNT", "READ-ONLY", "ID"], 0);
    for c in &rows {
        table.row(
            vec![
                output::one_line(c["name"].as_str().unwrap_or_default()),
                c["email"].as_str().unwrap_or_default().to_owned(),
                if c["read_only"].as_bool() == Some(true) {
                    "yes".into()
                } else {
                    String::new()
                },
                c["id"].as_str().unwrap_or_default().to_owned(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

/// `tern calendar create`.
pub fn create(ctx: &Ctx, engines: &mut [Engine], e: NewEvent<'_>) -> Result<()> {
    let t = target(ctx, engines, feature::CALENDAR_EDITING, "create events")?;
    let start = when(e.start, "--start")?;
    let end = match e.end {
        Some(s) => when(s, "--end")?,
        None if e.all_day => start + 86400,
        None => start + 3600,
    };
    let mut params = with_tz(json!({
        "accountId": t.account_id, "calendarId": e.calendar, "summary": e.title,
        "start": start, "end": end, "isAllDay": e.all_day,
    }));
    if let Some(tz) = local_tz_name() {
        params["timezone"] = json!(tz);
    }
    for (k, v) in [
        ("location", e.location),
        ("description", e.description),
        ("recurrenceRule", e.rrule),
    ] {
        if let Some(v) = v {
            params[k] = json!(v);
        }
    }
    if !e.guests.is_empty() {
        params["attendees"] = json!(e
            .guests
            .iter()
            .map(|g| json!({ "email": g }))
            .collect::<Vec<_>>());
    }
    let r: Value = call(engines, &t, method::EVENTS_CREATE, params)?;
    print_writes(ctx, &r, "Queued");
    Ok(())
}

#[derive(Debug, Default)]
pub struct EventEdit<'a> {
    pub event: &'a str,
    pub scope: &'a str,
    pub occurrence: Option<&'a str>,
    pub start: Option<&'a str>,
    pub end: Option<&'a str>,
    pub title: Option<&'a str>,
    pub location: Option<&'a str>,
    pub description: Option<&'a str>,
    pub delete: bool,
}

/// `tern calendar update`.
pub fn update(ctx: &Ctx, engines: &mut [Engine], e: EventEdit<'_>) -> Result<()> {
    let t = target(ctx, engines, feature::CALENDAR_EDITING, "edit events")?;
    let mut patch = Map::new();
    if let Some(s) = e.start {
        patch.insert("start".into(), json!(when(s, "--start")?));
    }
    if let Some(s) = e.end {
        patch.insert("end".into(), json!(when(s, "--end")?));
    }
    for (k, v) in [
        ("summary", e.title),
        ("location", e.location),
        ("description", e.description),
    ] {
        if let Some(v) = v {
            patch.insert(k.into(), json!(v));
        }
    }
    if e.delete {
        patch.insert("deleted".into(), json!(true));
    }
    if patch.is_empty() {
        return Err(CliError::usage(
            "nothing to change: give --start, --end, --title, --location, --description or --delete",
        ));
    }
    let mut params = with_tz(json!({
        "accountId": t.account_id, "eventId": e.event, "scope": e.scope, "patch": patch,
    }));
    if let Some(o) = e.occurrence {
        params["occurrenceStart"] = json!(when(o, "--occurrence")?);
    }
    let r: Value = call(engines, &t, method::EVENTS_UPDATE, params)?;
    print_writes(ctx, &r, "Queued");
    Ok(())
}

/// `tern calendar rsvp`.
pub fn rsvp(
    ctx: &Ctx,
    engines: &mut [Engine],
    event: &str,
    status: &str,
    comment: Option<&str>,
    propose: Option<(&str, &str)>,
) -> Result<()> {
    let t = target(
        ctx,
        engines,
        feature::CALENDAR_EDITING,
        "answer invitations",
    )?;
    let mut params = with_tz(json!({
        "accountId": t.account_id, "eventId": event, "status": status.to_ascii_uppercase(),
    }));
    if let Some(c) = comment {
        params["comment"] = json!(c);
    }
    if let Some((s, e)) = propose {
        params["counter"] =
            json!({ "start": when(s, "--propose-start")?, "end": when(e, "--propose-end")? });
    }
    let r: Value = call(engines, &t, method::EVENTS_RSVP, params)?;
    if ctx.json {
        output::json(&r);
    } else if propose.is_some() {
        output::line("Proposed a new time to the organizer.");
    } else {
        output::line(&format!(
            "Answered {} as {}{}",
            status.to_ascii_lowercase(),
            r["email"].as_str().unwrap_or_default(),
            if r["replyTaskId"].is_string() {
                "; the organizer is told by mail"
            } else {
                ""
            }
        ));
    }
    Ok(())
}

// -- contacts -----------------------------------------------------------------------------

fn print_contact(ctx: &Ctx, r: &Value) {
    if ctx.json {
        output::json(r);
        return;
    }
    let d = &r["data"];
    let name = d["name"]["displayName"].as_str().unwrap_or_default();
    output::line(&output::one_line(if name.is_empty() {
        r["contact"]["name"].as_str().unwrap_or_default()
    } else {
        name
    }));
    for (label, key) in [
        ("email", "emailAddresses"),
        ("phone", "phoneNumbers"),
        ("url", "urls"),
    ] {
        for v in d[key].as_array().into_iter().flatten() {
            output::line(&format!(
                "  {label}: {}",
                output::one_line(v["value"].as_str().unwrap_or_default())
            ));
        }
    }
    for (label, key) in [
        ("company", "company"),
        ("title", "title"),
        ("notes", "notes"),
    ] {
        if let Some(v) = d[key].as_str().filter(|v| !v.is_empty()) {
            output::line(&format!("  {label}: {}", output::one_line(v)));
        }
    }
    output::line(&format!(
        "  ({}{})",
        r["metadata"]["origin"].as_str().unwrap_or_default(),
        if r["metadata"]["readonly"].as_bool() == Some(true) {
            ", read-only"
        } else {
            ""
        }
    ));
}

pub fn contact_get(ctx: &Ctx, engines: &mut [Engine], id: &str) -> Result<()> {
    let t = target(ctx, engines, feature::CONTACT_EDITING, "read contacts")?;
    let r: Value = call(
        engines,
        &t,
        method::CONTACTS_GET,
        json!({ "accountId": t.account_id, "contactId": id }),
    )?;
    print_contact(ctx, &r);
    Ok(())
}

/// The fields a contact command sets: `--patch` JSON, then the flags over it.
pub fn contact_fields(
    patch: Option<&str>,
    name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    company: Option<&str>,
    title: Option<&str>,
    notes: Option<&str>,
) -> Result<Map<String, Value>> {
    let mut out = match patch {
        Some(p) => match serde_json::from_str::<Value>(p) {
            Ok(Value::Object(m)) => m,
            _ => return Err(CliError::usage("--patch must be a JSON object")),
        },
        None => Map::new(),
    };
    if let Some(n) = name {
        let (given, family) = n.trim().split_once(' ').unwrap_or((n.trim(), ""));
        out.insert(
            "name".into(),
            json!({ "displayName": n, "givenName": given, "familyName": family }),
        );
    }
    if let Some(e) = email {
        out.insert("emailAddresses".into(), json!([{ "value": e }]));
    }
    if let Some(p) = phone {
        out.insert("phoneNumbers".into(), json!([{ "value": p }]));
    }
    for (k, v) in [("company", company), ("title", title), ("notes", notes)] {
        if let Some(v) = v {
            out.insert(k.into(), json!(v));
        }
    }
    Ok(out)
}

pub fn contact_create(ctx: &Ctx, engines: &mut [Engine], data: Map<String, Value>) -> Result<()> {
    if data.is_empty() {
        return Err(CliError::usage("give --name, --email or --patch"));
    }
    let t = target(ctx, engines, feature::CONTACT_EDITING, "create contacts")?;
    let r: Value = call(
        engines,
        &t,
        method::CONTACTS_CREATE,
        json!({ "accountId": t.account_id, "data": data }),
    )?;
    print_contact(ctx, &r);
    Ok(())
}

pub fn contact_update(
    ctx: &Ctx,
    engines: &mut [Engine],
    id: &str,
    patch: Map<String, Value>,
) -> Result<()> {
    if patch.is_empty() {
        return Err(CliError::usage("nothing to change"));
    }
    let t = target(ctx, engines, feature::CONTACT_EDITING, "edit contacts")?;
    let r: Value = call(
        engines,
        &t,
        method::CONTACTS_UPDATE,
        json!({ "accountId": t.account_id, "contactId": id, "patch": patch }),
    )?;
    print_contact(ctx, &r);
    Ok(())
}

pub fn contact_import(ctx: &Ctx, engines: &mut [Engine], file: &str) -> Result<()> {
    let vcf = if file == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
            .map_err(|e| CliError::failed(format!("can't read stdin: {e}")))?;
        s
    } else {
        std::fs::read_to_string(file)
            .map_err(|e| CliError::failed(format!("can't read {file}: {e}")))?
    };
    let t = target(ctx, engines, feature::CONTACT_EDITING, "import contacts")?;
    let r: Value = call(
        engines,
        &t,
        method::CONTACTS_IMPORT,
        json!({ "accountId": t.account_id, "vcf": vcf }),
    )?;
    if ctx.json {
        output::json(&r);
    } else {
        let n = r["contactIds"].as_array().map_or(0, Vec::len);
        output::line(&format!(
            "Imported {n} contact{} into {}",
            if n == 1 { "" } else { "s" },
            t.email
        ));
    }
    Ok(())
}

pub fn contact_export(ctx: &Ctx, engines: &mut [Engine], ids: &[String]) -> Result<()> {
    let t = target(ctx, engines, feature::CONTACT_EDITING, "export contacts")?;
    let r: Value = call(
        engines,
        &t,
        method::CONTACTS_EXPORT,
        json!({ "accountId": t.account_id, "ids": ids }),
    )?;
    if ctx.json {
        output::json(&r);
    } else {
        output::text(r["vcf"].as_str().unwrap_or_default());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_fields_merge_flags_over_patch() {
        let m = contact_fields(
            Some(r#"{"title": "x", "urls": [{"value": "https://a"}]}"#),
            Some("Ann Archer"),
            None,
            Some("+1 555"),
            None,
            Some("Lead"),
            None,
        )
        .unwrap();
        assert_eq!(m["title"], "Lead");
        assert_eq!(m["name"]["familyName"], "Archer");
        assert_eq!(m["phoneNumbers"][0]["value"], "+1 555");
        assert_eq!(m["urls"][0]["value"], "https://a");
        assert!(contact_fields(Some("[1]"), None, None, None, None, None, None).is_err());
    }
}
