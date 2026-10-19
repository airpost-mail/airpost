// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Calendar and contact editing on the socket (docs/desktop-client-plan.md 3.8, stage 18,
//! `feature::CALENDAR_EDITING`, `feature::CONTACT_EDITING`).
//!
//! `events.expand`, `contacts.get` and `contacts.export` are reads, answered on the
//! connection's read-only store. The writes go to the account's `main` thread, which builds
//! the ICS or vCard with `ms_pim` from the stored row and queues the tasks a client would
//! (`SyncbackEventTask`, `DestroyEventTask`, `EventRSVPTask`, `SyncbackContactTask`) through
//! the same local phases.

use std::collections::HashMap;

use ms_core::Account;
use ms_ipc::{
    code, method, ContactDetails, ContactsCreateParams, ContactsExportParams, ContactsExportResult,
    ContactsGetParams, ContactsImportParams, ContactsImportResult, ContactsUpdateParams,
    ErrorObject, EventWrite, EventsCreateParams, EventsExpandParams, EventsExpandResult,
    EventsRsvpParams, EventsRsvpResult, EventsUpdateParams, EventsWriteResult,
    Request as RpcRequest,
};
use ms_pim::edit::{self, Doc, Guest, NewEvent, Person, Rid};
use ms_pim::occurrence::{self, EventRow, Owners};
use ms_pim::plan::{self, Patch, Rsvp, Scope, Target, Write};
use ms_pim::{contact, vcf, Env};
use ms_store::read::EventFilter;
use ms_store::{Calendar, Contact, Event, Model, Query, Store, Task};
use serde_json::{json, Map, Value};

use crate::dispatch::{Reply, Request};
use crate::drafts_api::QueueTask;
use crate::socket::Session;

pub fn is_pim_method(name: &str) -> bool {
    matches!(
        name,
        method::EVENTS_EXPAND
            | method::EVENTS_CREATE
            | method::EVENTS_UPDATE
            | method::EVENTS_RSVP
            | method::CONTACTS_GET
            | method::CONTACTS_CREATE
            | method::CONTACTS_UPDATE
            | method::CONTACTS_IMPORT
            | method::CONTACTS_EXPORT
    )
}

/// A write, for the account's `main` thread.
#[derive(Debug)]
pub enum Call {
    EventsCreate(EventsCreateParams),
    EventsUpdate(EventsUpdateParams),
    EventsRsvp(EventsRsvpParams),
    ContactsCreate(ContactsCreateParams),
    ContactsUpdate(ContactsUpdateParams),
    ContactsImport(ContactsImportParams),
}

fn invalid(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(
        code::INVALID_PARAMS,
        format!("invalid params: {}", message.into()),
    )
}

fn not_found(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(code::NOT_FOUND, message.into())
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

/// An edit `ms_pim` refused; `data.key` says why (`invalid-ics`, `read-only`,
/// `not-invited`, `no-organizer`, ...).
fn refused(e: ms_pim::Error) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, e.message).with_data(json!({ "key": e.code }))
}

fn to_reply(v: impl serde::Serialize) -> Reply {
    serde_json::to_value(v).map_err(internal)
}

/// The user's zone: `tz` by IANA or Windows name, UTC when absent.
fn env_for(tz: Option<&str>) -> Result<Env, ErrorObject> {
    match tz {
        Some(name)
            if ms_pim::time::resolve_iana(name).is_none() && !ms_pim::time::is_utc_alias(name) =>
        {
            Err(invalid(format!("tz {name:?} is not a time zone")))
        }
        _ => Ok(Env::for_zone(tz)),
    }
}

/// Whose events are whose: every account's addresses, and the calendars a server says are
/// ours by the address in their path.
fn owners(accounts: &[Account], calendars: &[Value]) -> Owners {
    let own_calendar_addresses: HashMap<String, String> = calendars
        .iter()
        .filter(|c| c["owner"].as_str() == Some("mine"))
        .filter_map(|c| {
            let id = c["id"].as_str()?;
            let address = occurrence::calendar_address(c["path"].as_str().unwrap_or_default());
            (!address.is_empty()).then(|| (id.to_owned(), address))
        })
        .collect();
    Owners {
        accounts: accounts
            .iter()
            .map(|a| {
                (
                    a.id().to_owned(),
                    a.own_addresses().map(str::to_owned).collect(),
                )
            })
            .collect(),
        own_calendar_addresses,
    }
}

// -- the connection's half ------------------------------------------------------------------

fn accounts_of(session: &Session) -> Vec<Account> {
    session
        .server
        .accounts
        .statuses()
        .into_iter()
        .filter_map(|s| s.account.and_then(|a| Account::from_json(a).ok()))
        .collect()
}

/// `events.*` and `contacts.*` from a connection.
pub fn call(session: &mut Session, req: &RpcRequest) -> Reply {
    let check = |s: &Session, id: &str| s.server.accounts.check(id);
    match req.method.as_str() {
        method::EVENTS_EXPAND => expand(session, req.params_as::<EventsExpandParams>()?),
        method::CONTACTS_GET => {
            let p = req.params_as::<ContactsGetParams>()?;
            check(session, &p.account_id)?;
            let store = session.reads.store()?;
            let found = store
                .contacts(&p.account_id, std::slice::from_ref(&p.contact_id))
                .map_err(internal)?;
            let c = found
                .into_iter()
                .next()
                .ok_or_else(|| not_found(format!("no contact {}", p.contact_id)))?;
            to_reply(details(c, None))
        }
        method::CONTACTS_EXPORT => {
            let p = req.params_as::<ContactsExportParams>()?;
            check(session, &p.account_id)?;
            let contacts = session
                .reads
                .store()?
                .contacts(&p.account_id, &p.ids)
                .map_err(internal)?;
            to_reply(ContactsExportResult {
                vcf: vcf::export(&contacts),
            })
        }
        name => {
            let (account_id, call) = match name {
                method::EVENTS_CREATE => {
                    let p = req.params_as::<EventsCreateParams>()?;
                    (p.account_id.clone(), Call::EventsCreate(p))
                }
                method::EVENTS_UPDATE => {
                    let p = req.params_as::<EventsUpdateParams>()?;
                    (p.account_id.clone(), Call::EventsUpdate(p))
                }
                method::EVENTS_RSVP => {
                    let p = req.params_as::<EventsRsvpParams>()?;
                    (p.account_id.clone(), Call::EventsRsvp(p))
                }
                method::CONTACTS_CREATE => {
                    let p = req.params_as::<ContactsCreateParams>()?;
                    (p.account_id.clone(), Call::ContactsCreate(p))
                }
                method::CONTACTS_UPDATE => {
                    let p = req.params_as::<ContactsUpdateParams>()?;
                    (p.account_id.clone(), Call::ContactsUpdate(p))
                }
                method::CONTACTS_IMPORT => {
                    let p = req.params_as::<ContactsImportParams>()?;
                    (p.account_id.clone(), Call::ContactsImport(p))
                }
                _ => {
                    return Err(ErrorObject::new(
                        code::METHOD_NOT_FOUND,
                        "not a calendar or contact method",
                    ))
                }
            };
            check(session, &account_id)?;
            session.dispatch(&account_id, Request::Pim(Box::new(call)))
        }
    }
}

fn expand(session: &mut Session, p: EventsExpandParams) -> Reply {
    let env = env_for(p.tz.as_deref())?;
    if p.end < p.start {
        return Err(invalid("end is before start"));
    }
    let all = accounts_of(session);
    let account_ids: Vec<String> = match &p.account_ids {
        Some(ids) => {
            for id in ids {
                session.server.accounts.check(id)?;
            }
            ids.clone()
        }
        None => all.iter().map(|a| a.id().to_owned()).collect(),
    };
    let store = session.reads.store()?;
    let calendars = store.calendars(&account_ids).map_err(internal)?;
    let owners = owners(&all, &calendars);
    let mut result = EventsExpandResult::default();

    if let Some(ics) = &p.ics {
        let doc = Doc::parse(ics).map_err(refused)?;
        let row = EventRow {
            id: String::new(),
            account_id: account_ids.first().cloned().unwrap_or_default(),
            icsuid: doc
                .first()
                .and_then(|v| v.value("UID"))
                .unwrap_or_default()
                .to_owned(),
            ics: ics.clone(),
            ..Default::default()
        };
        result.occurrences =
            occurrence::occurrences(std::slice::from_ref(&row), p.start, p.end, &owners, &env);
        if let Some(focus) = &p.focus {
            let times = |t: Option<(i64, i64, Option<i64>, bool)>| match t {
                Some((start, end, rid, all_day)) => {
                    let mut v = json!({ "start": start, "end": end, "isAllDay": all_day });
                    if let Some(r) = rid {
                        v["recurrenceId"] = json!(r);
                    }
                    v
                }
                None => Value::Null,
            };
            result.upcoming = Some(times(occurrence::upcoming(&doc, focus.now, &env)));
            result.first = Some(times(occurrence::first_times(&doc, &env)));
        }
        return to_reply(result);
    }

    let window = p.event_ids.is_none().then_some((p.start, p.end));
    let rows: Vec<EventRow> = store
        .events(&EventFilter {
            account_ids: &account_ids,
            calendar_ids: p.calendar_ids.as_deref(),
            event_ids: p.event_ids.as_deref(),
            window,
        })
        .map_err(internal)?
        .iter()
        .filter_map(EventRow::from_json)
        .collect();
    result.occurrences = occurrence::occurrences(&rows, p.start, p.end, &owners, &env);
    if let Some(focus) = &p.focus {
        result.focused = Some(
            occurrence::focused(&rows, focus.now, focus.recurrence_id_start, &owners, &env)
                .unwrap_or(Value::Null),
        );
    }
    if let Some(c) = &p.conflicts {
        let addresses: Vec<String> = all
            .iter()
            .filter(|a| account_ids.iter().any(|id| id == a.id()))
            .flat_map(|a| a.own_addresses().map(str::to_owned).collect::<Vec<_>>())
            .collect();
        let found = occurrence::conflicts(
            &rows,
            p.start,
            p.end,
            &addresses,
            c.exclude_icsuid.as_deref(),
            &env,
        );
        result.conflicts = Some(
            found
                .iter()
                .filter_map(|c| serde_json::to_value(c).ok())
                .collect(),
        );
    }
    to_reply(result)
}

fn details(contact: Value, task_id: Option<String>) -> ContactDetails {
    let parsed = contact::parse(&contact);
    ContactDetails {
        metadata: json!({ "origin": parsed.origin, "readonly": parsed.readonly }),
        data: parsed.data,
        contact,
        task_id,
    }
}

// -- the account's half ---------------------------------------------------------------------

/// The account's `main` thread: its store, account and task queue.
pub struct Here<'a> {
    pub store: &'a Store,
    pub account: &'a Account,
    pub queue: &'a dyn QueueTask,
}

impl Here<'_> {
    pub fn handle(&self, call: Call) -> Reply {
        match call {
            Call::EventsCreate(p) => self.create_event(p),
            Call::EventsUpdate(p) => self.update_event(p),
            Call::EventsRsvp(p) => self.rsvp(p),
            Call::ContactsCreate(p) => self.create_contact(p),
            Call::ContactsUpdate(p) => self.update_contact(p),
            Call::ContactsImport(p) => self.import_contacts(p),
        }
    }

    /// Runs the task's local phase; answers with the task as it left it.
    fn queue_task(&self, class: &str, data: &Value) -> Result<(String, Value), ErrorObject> {
        let task = Task::create(class, self.account.id(), data);
        let id = task.id().to_owned();
        let done = self.queue.queue_task(task.data)?;
        Ok((id, done))
    }

    fn event(&self, id: &str) -> Result<Event, ErrorObject> {
        self.store
            .find::<Event>(&Query::new().equal("id", id))
            .map_err(internal)?
            .filter(|e| e.account_id() == self.account.id())
            .ok_or_else(|| not_found(format!("no event {id} in account {}", self.account.id())))
    }

    fn calendar(&self, id: &str) -> Result<Calendar, ErrorObject> {
        self.store
            .find::<Calendar>(&Query::new().equal("id", id))
            .map_err(internal)?
            .filter(|c| c.account_id() == self.account.id())
            .ok_or_else(|| not_found(format!("no calendar {id} in account {}", self.account.id())))
    }

    /// Whether the calendar's events may be written: not read-only, not somebody else's.
    fn writable(cal: &Calendar) -> bool {
        !cal.read_only() && cal.ownership() != "other"
    }

    fn check_writable(&self, cal: &Calendar) -> Result<(), ErrorObject> {
        if Self::writable(cal) {
            Ok(())
        } else {
            Err(refused(ms_pim::Error::new(
                "read-only",
                format!("calendar {} can't be written", cal.id()),
            )))
        }
    }

    fn organizer(&self) -> Person {
        let name = self.account.as_json()["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        Person {
            email: self.account.email_address().to_owned(),
            name,
        }
    }

    /// Whether `email` is this account's, or the address of the calendar (one the server
    /// says is ours) the event is on.
    fn is_mine(&self, email: &str, cal: Option<&Calendar>) -> bool {
        self.account
            .own_addresses()
            .any(|a| ms_pim::same_address(a, email))
            || cal.is_some_and(|c| {
                c.ownership() == "mine"
                    && ms_pim::same_address(&occurrence::calendar_address(c.path()), email)
            })
    }

    /// Queues one write, or with `queue` false only describes it.
    fn write(
        &self,
        w: &Write,
        rsvp: Option<&Value>,
        queue: bool,
    ) -> Result<EventWrite, ErrorObject> {
        let (class, data, mut out) = match w {
            Write::Update { event_id, ics } => {
                let stored = self.event(event_id)?;
                let mut event = stored.data.clone();
                event["ics"] = json!(ics);
                let mut data = json!({ "event": event, "calendarId": stored.calendar_id() });
                if let Some(a) = rsvp.filter(|a| !a.is_null()) {
                    data["rsvp"] = a.clone();
                }
                let out = EventWrite {
                    kind: "update".into(),
                    event_id: Some(event_id.clone()),
                    calendar_id: Some(stored.calendar_id().to_owned()),
                    ics: Some(ics.clone()),
                    task_id: None,
                };
                ("SyncbackEventTask", data, out)
            }
            Write::Create { calendar_id, ics } => {
                let event = json!({
                    "__cls": "Event",
                    "aid": self.account.id(),
                    "cid": calendar_id,
                    "ics": ics,
                });
                let data = json!({ "event": event, "calendarId": calendar_id });
                let out = EventWrite {
                    kind: "create".into(),
                    event_id: None,
                    calendar_id: Some(calendar_id.clone()),
                    ics: Some(ics.clone()),
                    task_id: None,
                };
                ("SyncbackEventTask", data, out)
            }
            Write::Destroy { event_id } => {
                let stored = self.event(event_id)?;
                let data = json!({ "events": [stored.data] });
                let out = EventWrite {
                    kind: "destroy".into(),
                    event_id: Some(event_id.clone()),
                    calendar_id: Some(stored.calendar_id().to_owned()),
                    ics: None,
                    task_id: None,
                };
                ("DestroyEventTask", data, out)
            }
        };
        if queue {
            let (id, done) = self.queue_task(class, &data)?;
            if let Some(eid) = done["event"]["id"].as_str() {
                out.event_id = Some(eid.to_owned());
            }
            out.task_id = Some(id);
        }
        Ok(out)
    }

    // -- events ---------------------------------------------------------------------------

    fn create_event(&self, p: EventsCreateParams) -> Reply {
        let env = env_for(p.tz.as_deref())?;
        if p.end < p.start {
            return Err(invalid("end is before start"));
        }
        let cal = self.calendar(&p.calendar_id)?;
        self.check_writable(&cal)?;
        let attendees: Vec<Guest> = p
            .attendees
            .iter()
            .map(|g| Guest {
                email: g.email.clone(),
                name: g.name.clone(),
                role: g.role.clone(),
            })
            .collect();
        let ev = NewEvent {
            uid: None,
            summary: p.summary,
            description: p.description,
            location: p.location,
            start: p.start,
            end: p.end,
            all_day: p.is_all_day,
            timezone: p.timezone,
            organizer: (!attendees.is_empty()).then(|| self.organizer()),
            attendees,
            recurrence_rule: p.recurrence_rule,
            transparency: p.transparency,
        };
        let (uid, w) = plan::plan_create(&ev, cal.id(), &env);
        let out = self.write(&w, None, p.queue.unwrap_or(true))?;
        to_reply(EventsWriteResult {
            writes: vec![out],
            icsuid: Some(uid),
        })
    }

    fn row(&self, id: &str) -> Result<(Event, EventRow), ErrorObject> {
        let e = self.event(id)?;
        let row = EventRow::from_json(&e.data)
            .ok_or_else(|| internal(format!("event {id} could not be read")))?;
        Ok((e, row))
    }

    fn update_event(&self, p: EventsUpdateParams) -> Reply {
        let env = env_for(p.tz.as_deref())?;
        let scope = match p.scope.as_deref().unwrap_or("all") {
            "this" => Scope::This,
            "following" => Scope::Following,
            "all" => Scope::All,
            other => {
                return Err(invalid(format!(
                    "scope must be this, following or all, not {other:?}"
                )))
            }
        };
        let patch: Patch = serde_json::from_value(Value::Object(p.patch.clone()))
            .map_err(|e| invalid(format!("patch: {e}")))?;
        let (event, row) = self.row(&p.event_id)?;
        let cal = self.calendar(event.calendar_id())?;
        self.check_writable(&cal)?;
        let occurrence = p.occurrence_start.or_else(|| {
            row.is_exception()
                .then(|| plan::exception_slot(&row, &env))
                .flatten()
        });
        let target = Target {
            row: &row,
            occurrence,
        };
        let organizer = self.organizer();
        let writes =
            plan::plan_update(&target, scope, &patch, Some(&organizer), &env).map_err(refused)?;
        let queue = p.queue.unwrap_or(true);
        let mut out = Vec::with_capacity(writes.len());
        for w in &writes {
            out.push(self.write(w, None, queue)?);
        }
        to_reply(EventsWriteResult {
            writes: out,
            icsuid: None,
        })
    }

    /// Our copy of an emailed invitation: the master row with its UID on our calendars.
    fn stored_copy(&self, ics: &str) -> Result<Option<Event>, ErrorObject> {
        let uid = Doc::parse(ics)
            .map_err(refused)?
            .first()
            .and_then(|v| v.value("UID"))
            .unwrap_or_default()
            .to_owned();
        if uid.is_empty() {
            return Ok(None);
        }
        let rows = self
            .store
            .find_all::<Event>(
                &Query::new()
                    .equal("accountId", self.account.id())
                    .equal("icsuid", uid.as_str()),
            )
            .map_err(internal)?;
        Ok(rows
            .iter()
            .find(|e| e.recurrence_id().is_empty())
            .or_else(|| rows.first())
            .cloned())
    }

    /// The account's first calendar an invitation may be added to.
    fn default_calendar(&self) -> Result<Option<String>, ErrorObject> {
        let mut cals = self
            .store
            .find_all::<Calendar>(&Query::new().equal("accountId", self.account.id()))
            .map_err(internal)?;
        cals.sort_by_key(|c| c.order());
        Ok(cals
            .into_iter()
            .find(Self::writable)
            .map(|c| c.id().to_owned()))
    }

    fn rsvp(&self, p: EventsRsvpParams) -> Reply {
        let env = env_for(p.tz.as_deref())?;
        let stored_event = match (&p.event_id, &p.ics) {
            (Some(id), _) => Some(self.event(id)?),
            (None, Some(ics)) => self.stored_copy(ics)?,
            (None, None) => return Err(invalid("events.rsvp needs eventId or ics")),
        };
        let row = stored_event
            .as_ref()
            .and_then(|e| EventRow::from_json(&e.data));
        let cal = match &stored_event {
            Some(e) => Some(self.calendar(e.calendar_id())?),
            None => None,
        };
        let is_mine = |email: &str| self.is_mine(email, cal.as_ref());
        let all_day = row
            .as_ref()
            .and_then(|r| Doc::parse(&r.ics).ok())
            .and_then(|d| d.master().and_then(ms_pim::time::start_of))
            .is_some_and(|s| s.is_date());
        let occurrence = p.occurrence_start.map(|t| Rid::of(t, all_day, &env));

        if let Some(counter) = &p.counter {
            let base;
            let row = match (&row, &p.ics) {
                (_, Some(ics)) => {
                    base = EventRow {
                        ics: ics.clone(),
                        ..Default::default()
                    };
                    &base
                }
                (Some(r), None) => r,
                (None, None) => unreachable!("checked above"),
            };
            let mut msg = plan::plan_counter(
                row,
                counter.start,
                counter.end,
                p.comment.as_deref(),
                occurrence,
                is_mine,
                &env,
            )
            .map_err(refused)?;
            if let Some(s) = &p.subject {
                msg.subject = s.clone();
            }
            let email = self_address(&row.ics, &|e| self.is_mine(e, cal.as_ref()));
            let task_id = self.queue_message(&msg, p.message_id.as_deref())?;
            return to_reply(EventsRsvpResult {
                email,
                write: None,
                reply_task_id: Some(task_id),
            });
        }

        let add_to = match (&p.calendar_id, &stored_event) {
            (Some(id), None) => {
                let c = self.calendar(id)?;
                self.check_writable(&c)?;
                Some(id.clone())
            }
            (None, None) => self.default_calendar()?,
            _ => None,
        };
        let writable = cal.as_ref().is_some_and(Self::writable);
        let input = Rsvp {
            stored: row.as_ref(),
            writable,
            invite: p.ics.as_deref(),
            add_to: add_to.as_deref(),
            status: &p.status,
            occurrence,
            reply: p.reply.unwrap_or(true),
            write: p.write.unwrap_or(true),
            retried: p.retried.unwrap_or(false),
        };
        let mut planned = plan::plan_rsvp(&input, is_mine, &env).map_err(refused)?;
        let write = match &planned.write {
            Some((w, answer)) => Some(self.write(w, Some(answer), true)?),
            None => None,
        };
        let mut reply_task_id = None;
        if let Some(msg) = planned.message.as_mut() {
            if let Some(s) = &p.subject {
                msg.subject = s.clone();
            }
            reply_task_id = Some(self.queue_message(msg, p.message_id.as_deref())?);
        }
        to_reply(EventsRsvpResult {
            email: planned.me,
            write,
            reply_task_id,
        })
    }

    /// An `EventRSVPTask` emailing `msg` to the organizer.
    fn queue_message(
        &self,
        msg: &plan::Message,
        message_id: Option<&str>,
    ) -> Result<String, ErrorObject> {
        let mut data = json!({
            "to": msg.to,
            "subject": msg.subject,
            "ics": msg.ics,
            "method": msg.method,
        });
        if let Some(s) = &msg.status {
            data["icsRSVPStatus"] = json!(s);
        }
        if let Some(c) = &msg.comment {
            data["comment"] = json!(c);
        }
        if let Some(m) = message_id {
            data["messageId"] = json!(m);
        }
        Ok(self.queue_task("EventRSVPTask", &data)?.0)
    }

    // -- contacts -------------------------------------------------------------------------

    fn google(&self) -> bool {
        self.account.provider() == "gmail"
    }

    fn contact(&self, id: &str) -> Result<Contact, ErrorObject> {
        self.store
            .find::<Contact>(&Query::new().equal("id", id))
            .map_err(internal)?
            .filter(|c| c.account_id() == self.account.id())
            .ok_or_else(|| not_found(format!("no contact {id} in account {}", self.account.id())))
    }

    /// Saves the contact through SyncbackContactTask; answers with it as stored.
    fn save_contact(&self, json: Value) -> Result<ContactDetails, ErrorObject> {
        let (id, done) = self.queue_task("SyncbackContactTask", &json!({ "contact": json }))?;
        let mut saved = done["contact"].clone();
        if let Some(cid) = saved["id"].as_str() {
            if let Ok(c) = self.contact(cid) {
                saved = c.data;
            }
        }
        Ok(details(saved, Some(id)))
    }

    fn create_contact(&self, p: ContactsCreateParams) -> Reply {
        let empty = contact::empty_contact(self.account.id(), self.google());
        let built = contact::apply(&empty, &Value::Object(p.data)).map_err(refused)?;
        to_reply(self.save_contact(built)?)
    }

    fn update_contact(&self, p: ContactsUpdateParams) -> Reply {
        let stored = self.contact(&p.contact_id)?;
        let mut next = contact::parse(&stored.data).data;
        if !next.is_object() {
            next = Value::Object(Map::new());
        }
        for (k, v) in p.patch {
            next[k] = v;
        }
        let updated = contact::apply(&stored.data, &next).map_err(refused)?;
        to_reply(self.save_contact(updated)?)
    }

    fn import_contacts(&self, p: ContactsImportParams) -> Reply {
        let contacts: Vec<Value> = if self.google() {
            // A Google account keeps its contacts as People, so each card is read into the
            // editor's shape and written as a new person.
            vcf::import(&p.vcf, self.account.id())
                .iter()
                .filter_map(|c| {
                    let data = contact::parse(c).data;
                    let empty = contact::empty_contact(self.account.id(), true);
                    contact::apply(&empty, &data).ok()
                })
                .collect()
        } else {
            vcf::import(&p.vcf, self.account.id())
        };
        let mut result = ContactsImportResult::default();
        for c in contacts {
            let saved = self.save_contact(c)?;
            if let Some(id) = saved.contact["id"].as_str() {
                result.contact_ids.push(id.to_owned());
            }
            result.task_ids.extend(saved.task_id);
        }
        to_reply(result)
    }
}

/// Our address on an event's guest list, for a counter-proposal's answer.
fn self_address(ics: &str, is_mine: &dyn Fn(&str) -> bool) -> String {
    Doc::parse(ics)
        .ok()
        .and_then(|d| edit::self_attendee(&d, is_mine))
        .unwrap_or_default()
}
