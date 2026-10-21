// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Calendar and contact tools on the engine's `events.*` and `contacts.*`.
//! The engine builds the ICS and vCards and queues the
//! tasks that write them. Anything that mails somebody (an answer to an invitation, an event
//! with guests, whom the server invites, and any change to a meeting the account organizes,
//! which the server mails to its guests) needs the send level.

use serde_json::{json, Map, Value};

use super::write::parse_send_at;
use super::{
    failed, opt_bool, opt_recipients, opt_str, opt_str_list, req_str, schema, string_list_schema,
    Ctx, Output, ToolDef, ToolError, ToolResult,
};
use crate::discovery::AccountRef;
use crate::policy::Category;

pub(super) fn definitions() -> Vec<ToolDef> {
    let guests = json!({
        "type": "array",
        "description": "Guests; the calendar server invites them by mail",
        "items": {"type": "object", "properties": {"email": {"type": "string"}, "name": {"type": "string"}}, "required": ["email"]}
    });
    vec![
        ToolDef {
            name: "list_events",
            description: "Calendar events between two times, every occurrence of a repeating event, oldest first. Each has eventId (for update_event and respond_to_event), title, start and end (ISO 8601; all-day events have startDate/endDate as YYYY-MM-DD, end exclusive), isRecurring, organizer, attendees and myStatus.",
            schema: schema(
                json!({
                    "start": {"type": "string", "description": "ISO 8601 date-time; without an offset it is local time"},
                    "end": {"type": "string", "description": "ISO 8601 date-time"},
                    "accountId": {"type": "string", "description": "Only this account (default: every enabled account)"},
                    "calendarIds": string_list_schema("Only these calendars")
                }),
                &["start", "end"],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "create_event",
            description: "Create a calendar event. With attendees the account is the organizer and the server mails them invitations, which needs the read-write-send access level.",
            schema: schema(
                json!({
                    "calendarId": {"type": "string", "description": "The calendar (a calendarId from list_events, or ask the user)"},
                    "title": {"type": "string"},
                    "start": {"type": "string", "description": "ISO 8601 date-time"},
                    "end": {"type": "string", "description": "ISO 8601 date-time"},
                    "allDay": {"type": "boolean"},
                    "location": {"type": "string"},
                    "description": {"type": "string"},
                    "attendees": guests,
                    "recurrenceRule": {"type": "string", "description": "An RFC 5545 RRULE value, e.g. FREQ=WEEKLY;BYDAY=MO;COUNT=10"},
                    "accountId": {"type": "string"}
                }),
                &["calendarId", "title", "start", "end"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "update_event",
            description: "Change or delete a calendar event. For a repeating event, scope says which occurrences: 'this' (one, named by occurrenceStart), 'following' (that one and the ones after) or 'all'. Only the fields given change; attendees replaces the guest list. Changing or deleting a meeting the account organizes that has guests, or giving attendees, needs read-write-send, as the server mails the guests.",
            schema: schema(
                json!({
                    "eventId": {"type": "string"},
                    "scope": {"type": "string", "enum": ["this", "following", "all"], "description": "Default: all"},
                    "occurrenceStart": {"type": "string", "description": "ISO 8601 start of the occurrence, for scope this or following"},
                    "title": {"type": "string"},
                    "start": {"type": "string", "description": "ISO 8601 date-time"},
                    "end": {"type": "string", "description": "ISO 8601 date-time"},
                    "location": {"type": "string"},
                    "description": {"type": "string"},
                    "attendees": guests,
                    "delete": {"type": "boolean", "description": "Delete the occurrences the scope covers"},
                    "accountId": {"type": "string"}
                }),
                &["eventId"],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "respond_to_event",
            description: "Answer an invitation (accepted, tentative or declined): the answer is written to the user's copy and mailed to the organizer. With proposedStart and proposedEnd it proposes another time to the organizer instead, and changes nothing.",
            schema: schema(
                json!({
                    "eventId": {"type": "string"},
                    "status": {"type": "string", "enum": ["accepted", "tentative", "declined"]},
                    "comment": {"type": "string"},
                    "occurrenceStart": {"type": "string", "description": "Answer only this occurrence of a repeating event (ISO 8601 start)"},
                    "proposedStart": {"type": "string", "description": "ISO 8601 date-time"},
                    "proposedEnd": {"type": "string", "description": "ISO 8601 date-time"},
                    "accountId": {"type": "string"}
                }),
                &["eventId", "status"],
            ),
            category: Category::Send,
        },
        ToolDef {
            name: "get_contact",
            description: "A contact's details: name, emailAddresses, phoneNumbers, addresses, company, title, birthdays, urls, notes, and whether it can be edited (contacts found only in mail can't).",
            schema: schema(
                json!({"contactId": {"type": "string", "description": "A contact id (search_contacts or a message's participants)"}, "accountId": {"type": "string"}}),
                &["contactId"],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "create_contact",
            description: "Add a contact to the account's address book (CardDAV, or Google Contacts on a Google account).",
            schema: schema(
                json!({
                    "name": {"type": "string", "description": "Full name, 'Given Family'"},
                    "email": {"type": "string"},
                    "phone": {"type": "string"},
                    "company": {"type": "string"},
                    "title": {"type": "string"},
                    "notes": {"type": "string"},
                    "accountId": {"type": "string", "description": "Required when several accounts are enabled"}
                }),
                &[],
            ),
            category: Category::Write,
        },
        ToolDef {
            name: "update_contact",
            description: "Change a contact; the fields given replace the stored ones (email and phone replace the whole list with that one value). Other details on the card are kept.",
            schema: schema(
                json!({
                    "contactId": {"type": "string"},
                    "name": {"type": "string"},
                    "email": {"type": "string"},
                    "phone": {"type": "string"},
                    "company": {"type": "string"},
                    "title": {"type": "string"},
                    "notes": {"type": "string"},
                    "accountId": {"type": "string"}
                }),
                &["contactId"],
            ),
            category: Category::Write,
        },
    ]
}

/// Epoch seconds for an ISO 8601 argument.
fn time_arg(args: &Value, key: &str) -> Result<Option<i64>, ToolError> {
    match opt_str(args, key)? {
        None => Ok(None),
        Some(s) => parse_send_at(&s, &chrono::Local)
            .map(|t| Some(t.timestamp()))
            .ok_or_else(|| failed(format!("Invalid {key} date: '{s}'"))),
    }
}

fn req_time(args: &Value, key: &str) -> Result<i64, ToolError> {
    time_arg(args, key)?
        .ok_or_else(|| failed(format!("Invalid arguments: '{key}' must be a string")))
}

fn tz() -> String {
    tern_compose::settings::local_tz().name().to_owned()
}

fn iso(ts: i64) -> Value {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|t| json!(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)))
        .unwrap_or(Value::Null)
}

fn day(days: i64) -> Value {
    chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
        .and_then(|d| d.checked_add_signed(chrono::Duration::days(days)))
        .map(|d| json!(d.format("%Y-%m-%d").to_string()))
        .unwrap_or(Value::Null)
}

/// An occurrence as the tools show it.
fn occurrence(o: &Value) -> Value {
    let mut out = json!({
        "eventId": o["eventId"], "calendarId": o["calendarId"], "accountId": o["accountId"],
        "title": o["title"], "isAllDay": o["isAllDay"], "isRecurring": o["isRecurring"],
        "location": o["location"], "organizer": o["organizer"], "attendees": o["attendees"],
        "myStatus": o["myStatus"], "isCancelled": o["isCancelled"],
    });
    if o["isAllDay"].as_bool() == Some(true) {
        out["startDate"] = day(o["startDate"].as_i64().unwrap_or(0));
        // The engine's endDate is the last day; an exclusive end reads like DTEND.
        out["endDate"] = day(o["endDate"].as_i64().unwrap_or(0) + 1);
    } else {
        out["start"] = iso(o["start"].as_i64().unwrap_or(0));
        out["end"] = iso(o["end"].as_i64().unwrap_or(0));
    }
    if let Some(r) = o["recurrenceIdStart"].as_i64() {
        out["occurrenceStart"] = iso(r);
    }
    out
}

fn sort_key(o: &Value) -> i64 {
    o["start"]
        .as_i64()
        .unwrap_or_else(|| o["startDate"].as_i64().unwrap_or(0) * 86400)
}

fn require(ctx: &Ctx<'_>, a: &AccountRef, feature: &str) -> Result<(), ToolError> {
    if ctx.backend.has_feature(&a.account_id, feature) {
        Ok(())
    } else {
        Err(failed(format!(
            "The Tern engine for account '{}' can't do this ({feature}); update Tern",
            a.account_id
        )))
    }
}

/// Calls `method` on `accountId`, or on each enabled account until one has the item (the
/// others answer not-found before writing anything).
fn on_holder(
    ctx: &mut Ctx<'_>,
    args: &Value,
    feature: &str,
    method: &str,
    params: Value,
    what: &str,
) -> Result<(String, Value), ToolError> {
    let accounts = ctx.accounts_for(opt_str(args, "accountId")?.as_deref())?;
    for a in &accounts {
        require(ctx, a, feature)?;
        match ctx.call(&a.account_id, method, params.clone()) {
            Ok(v) => return Ok((a.account_id.clone(), v)),
            Err(e) if e.is_not_found() => continue,
            Err(e) => return Err(engine_error(e)),
        }
    }
    Err(failed(format!("{what} not found")))
}

/// An engine refusal with its reason (`data.key`: read-only, not-invited, ...).
fn engine_error(e: crate::discovery::EngineError) -> ToolError {
    if let crate::discovery::EngineError::Rpc {
        message,
        data: Some(d),
        ..
    } = &e
    {
        if let Some(key) = d.get("key").and_then(Value::as_str) {
            return failed(format!("{message} ({key})"));
        }
    }
    e.into()
}

fn one_account(ctx: &Ctx<'_>, args: &Value) -> Result<AccountRef, ToolError> {
    let mut accounts = ctx.accounts_for(opt_str(args, "accountId")?.as_deref())?;
    match accounts.len() {
        0 => Err(failed("No enabled account")),
        1 => Ok(accounts.remove(0)),
        _ => Err(failed(
            "Several accounts are enabled; give accountId (list_accounts lists them)",
        )),
    }
}

fn guests(args: &Value) -> Result<Option<Vec<Value>>, ToolError> {
    Ok(opt_recipients(args, "attendees")?.map(|list| {
        list.into_iter()
            .map(|r| {
                let mut g = json!({ "email": r.email });
                if !r.name.is_empty() {
                    g["name"] = json!(r.name);
                }
                g
            })
            .collect()
    }))
}

fn need_send(ctx: &Ctx<'_>) -> Result<(), ToolError> {
    ctx.policy
        .check_access(Category::Send)
        .map_err(ToolError::Denied)
}

pub(super) fn list_events(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let start = req_time(args, "start")?;
    let end = req_time(args, "end")?;
    if end < start {
        return Err(failed("end is before start"));
    }
    let calendars = opt_str_list(args, "calendarIds")?;
    let accounts = ctx.accounts_for(opt_str(args, "accountId")?.as_deref())?;
    let mut all: Vec<Value> = Vec::new();
    for a in &accounts {
        if !ctx
            .backend
            .has_feature(&a.account_id, tern_ipc::feature::CALENDAR_EDITING)
        {
            continue;
        }
        let mut params = json!({
            "accountIds": [a.account_id], "start": start, "end": end, "tz": tz(),
        });
        if let Some(c) = &calendars {
            params["calendarIds"] = json!(c);
        }
        let r = ctx
            .call(&a.account_id, tern_ipc::method::EVENTS_EXPAND, params)
            .map_err(engine_error)?;
        all.extend(r["occurrences"].as_array().cloned().unwrap_or_default());
    }
    all.sort_by_key(sort_key);
    let events: Vec<Value> = all.iter().map(occurrence).collect();
    let mut out = Output::text(&json!({ "count": events.len(), "events": events }));
    if let [one] = accounts.as_slice() {
        out = out.with_account(&one.account_id);
    }
    Ok(out)
}

pub(super) fn create_event(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let start = req_time(args, "start")?;
    let end = req_time(args, "end")?;
    let attendees = guests(args)?.unwrap_or_default();
    if !attendees.is_empty() {
        need_send(ctx)?;
    }
    let account = one_account(ctx, args)?;
    require(ctx, &account, tern_ipc::feature::CALENDAR_EDITING)?;
    let mut params = json!({
        "calendarId": req_str(args, "calendarId")?, "summary": req_str(args, "title")?,
        "start": start, "end": end, "isAllDay": opt_bool(args, "allDay")?.unwrap_or(false),
        "timezone": tz(), "tz": tz(),
    });
    for (arg, key) in [
        ("location", "location"),
        ("description", "description"),
        ("recurrenceRule", "recurrenceRule"),
    ] {
        if let Some(v) = opt_str(args, arg)? {
            params[key] = json!(v);
        }
    }
    if !attendees.is_empty() {
        params["attendees"] = json!(attendees);
    }
    let r = ctx
        .call(&account.account_id, tern_ipc::method::EVENTS_CREATE, params)
        .map_err(engine_error)?;
    Ok(Output::text(&json!({
        "created": true,
        "eventId": r["writes"][0]["eventId"],
        "uid": r["icsuid"],
    }))
    .with_account(&account.account_id))
}

pub(super) fn update_event(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let event_id = req_str(args, "eventId")?;
    let mut patch = Map::new();
    for (arg, key) in [
        ("title", "summary"),
        ("location", "location"),
        ("description", "description"),
    ] {
        if let Some(v) = opt_str(args, arg)? {
            patch.insert(key.into(), json!(v));
        }
    }
    for key in ["start", "end"] {
        if let Some(t) = time_arg(args, key)? {
            patch.insert(key.into(), json!(t));
        }
    }
    if let Some(g) = guests(args)? {
        need_send(ctx)?;
        patch.insert("attendees".into(), json!(g));
    }
    if opt_bool(args, "delete")? == Some(true) {
        patch.insert("deleted".into(), json!(true));
    }
    if patch.is_empty() {
        return Err(failed("Nothing to change"));
    }
    let mut params = json!({
        "eventId": event_id, "patch": patch, "tz": tz(),
        "scope": opt_str(args, "scope")?.unwrap_or_else(|| "all".into()),
    });
    if let Some(t) = time_arg(args, "occurrenceStart")? {
        params["occurrenceStart"] = json!(t);
    }
    let what = format!("Event '{event_id}'");
    if ctx.policy.check_access(Category::Send).is_err() {
        // The server mails the guests any change to a meeting the account organizes; the
        // engine says whether this is one, writing nothing.
        let mut dry = params.clone();
        dry["queue"] = json!(false);
        let (_, r) = on_holder(
            ctx,
            args,
            tern_ipc::feature::CALENDAR_EDITING,
            tern_ipc::method::EVENTS_UPDATE,
            dry,
            &what,
        )?;
        if r["notifiesGuests"] == json!(true) {
            need_send(ctx)?;
        }
    }
    let (aid, r) = on_holder(
        ctx,
        args,
        tern_ipc::feature::CALENDAR_EDITING,
        tern_ipc::method::EVENTS_UPDATE,
        params,
        &what,
    )?;
    let writes: Vec<Value> = r["writes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| json!({"kind": w["kind"], "eventId": w["eventId"]}))
        .collect();
    Ok(Output::text(&json!({ "updated": true, "writes": writes })).with_account(&aid))
}

pub(super) fn respond_to_event(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let event_id = req_str(args, "eventId")?;
    let status = req_str(args, "status")?.to_ascii_uppercase();
    if !matches!(status.as_str(), "ACCEPTED" | "TENTATIVE" | "DECLINED") {
        return Err(failed("status must be accepted, tentative or declined"));
    }
    let mut params = json!({ "eventId": event_id, "status": status, "tz": tz() });
    if let Some(c) = opt_str(args, "comment")? {
        params["comment"] = json!(c);
    }
    if let Some(t) = time_arg(args, "occurrenceStart")? {
        params["occurrenceStart"] = json!(t);
    }
    match (
        time_arg(args, "proposedStart")?,
        time_arg(args, "proposedEnd")?,
    ) {
        (Some(s), Some(e)) => params["counter"] = json!({ "start": s, "end": e }),
        (None, None) => {}
        _ => return Err(failed("give both proposedStart and proposedEnd")),
    }
    let proposing = params.get("counter").is_some();
    let (aid, r) = on_holder(
        ctx,
        args,
        tern_ipc::feature::CALENDAR_EDITING,
        tern_ipc::method::EVENTS_RSVP,
        params,
        &format!("Event '{event_id}'"),
    )?;
    Ok(Output::text(&json!({
        "responded": !proposing,
        "proposed": proposing,
        "as": r["email"],
        "organizerNotified": r["replyTaskId"].is_string(),
    }))
    .with_account(&aid))
}

fn contact_out(r: &Value) -> Value {
    json!({
        "contactId": r["contact"]["id"],
        "readonly": r["metadata"]["readonly"],
        "source": r["metadata"]["origin"],
        "details": r["data"],
    })
}

fn contact_fields(args: &Value) -> Result<Map<String, Value>, ToolError> {
    let mut out = Map::new();
    if let Some(n) = opt_str(args, "name")? {
        let (given, family) = n.trim().split_once(' ').unwrap_or((n.trim(), ""));
        out.insert(
            "name".into(),
            json!({"displayName": n, "givenName": given, "familyName": family}),
        );
    }
    if let Some(e) = opt_str(args, "email")? {
        out.insert("emailAddresses".into(), json!([{ "value": e }]));
    }
    if let Some(p) = opt_str(args, "phone")? {
        out.insert("phoneNumbers".into(), json!([{ "value": p }]));
    }
    for key in ["company", "title", "notes"] {
        if let Some(v) = opt_str(args, key)? {
            out.insert(key.into(), json!(v));
        }
    }
    Ok(out)
}

pub(super) fn get_contact(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let id = req_str(args, "contactId")?;
    let (aid, r) = on_holder(
        ctx,
        args,
        tern_ipc::feature::CONTACT_EDITING,
        tern_ipc::method::CONTACTS_GET,
        json!({ "contactId": id }),
        &format!("Contact '{id}'"),
    )?;
    Ok(Output::text(&contact_out(&r)).with_account(&aid))
}

pub(super) fn create_contact(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let data = contact_fields(args)?;
    if data.is_empty() {
        return Err(failed("Give at least a name or an email"));
    }
    let account = one_account(ctx, args)?;
    require(ctx, &account, tern_ipc::feature::CONTACT_EDITING)?;
    let r = ctx
        .call(
            &account.account_id,
            tern_ipc::method::CONTACTS_CREATE,
            json!({ "data": data }),
        )
        .map_err(engine_error)?;
    Ok(Output::text(&contact_out(&r)).with_account(&account.account_id))
}

pub(super) fn update_contact(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let id = req_str(args, "contactId")?;
    let patch = contact_fields(args)?;
    if patch.is_empty() {
        return Err(failed("Nothing to change"));
    }
    let (aid, r) = on_holder(
        ctx,
        args,
        tern_ipc::feature::CONTACT_EDITING,
        tern_ipc::method::CONTACTS_UPDATE,
        json!({ "contactId": id, "patch": patch }),
        &format!("Contact '{id}'"),
    )?;
    Ok(Output::text(&contact_out(&r)).with_account(&aid))
}
