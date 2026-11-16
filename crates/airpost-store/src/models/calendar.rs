// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `Calendar` and `Event`. Derived from Mailspring-Sync `MailSync/Models/Calendar.*` and
//! `Event.*`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Neither table has a `version` column, so neither binds one; the JSON still carries `v`.
//! ICS parsing is the DAV library's: it hands an event's fields over as [`IcsEventFields`].

use airpost_core::Result;
use rusqlite::Statement;
use serde_json::{json, Value};

use crate::model::{base_json, bind_named, json_bool, json_i64, json_str, object_or_err};
use crate::{Model, Store};

#[derive(Clone, Debug, PartialEq)]
pub struct Calendar {
    pub data: Value,
}

impl Calendar {
    pub fn new(id: &str, account_id: &str) -> Self {
        Calendar {
            data: base_json(id, account_id, 0),
        }
    }

    pub fn path(&self) -> &str {
        json_str(&self.data, "path")
    }

    pub fn set_path(&mut self, path: &str) {
        self.data["path"] = json!(path);
    }

    pub fn name(&self) -> &str {
        json_str(&self.data, "name")
    }

    pub fn set_name(&mut self, name: &str) {
        self.data["name"] = json!(name);
    }

    pub fn ctag(&self) -> &str {
        json_str(&self.data, "ctag")
    }

    pub fn set_ctag(&mut self, ctag: &str) {
        self.data["ctag"] = json!(ctag);
    }

    pub fn sync_token(&self) -> &str {
        json_str(&self.data, "syncToken")
    }

    pub fn set_sync_token(&mut self, token: &str) {
        self.data["syncToken"] = json!(token);
    }

    /// Google Calendar blue when the server gave no colour.
    pub fn color(&self) -> &str {
        self.data
            .get("color")
            .and_then(Value::as_str)
            .unwrap_or("#4285f4")
    }

    /// Stores the colour lowercased, without the alpha channel Apple sends (`#RRGGBBAA`).
    pub fn set_color(&mut self, color: &str) {
        let color = match color.get(..7) {
            Some(rgb) if color.len() == 9 && color.starts_with('#') => rgb,
            _ => color,
        };
        self.data["color"] = json!(color.to_ascii_lowercase());
    }

    pub fn description(&self) -> &str {
        json_str(&self.data, "description")
    }

    pub fn set_description(&mut self, d: &str) {
        self.data["description"] = json!(d);
    }

    pub fn read_only(&self) -> bool {
        json_bool(&self.data, "read_only")
    }

    pub fn set_read_only(&mut self, r: bool) {
        self.data["read_only"] = json!(r);
    }

    pub fn ownership(&self) -> &str {
        json_str(&self.data, "owner")
    }

    pub fn set_ownership(&mut self, o: &str) {
        self.data["owner"] = json!(o);
    }

    pub fn order(&self) -> i64 {
        json_i64(&self.data["order"])
    }

    pub fn set_order(&mut self, o: i64) {
        self.data["order"] = json!(o);
    }
}

impl Model for Calendar {
    model_boilerplate!("Calendar", ["id", "data", "accountId"]);

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "Calendar")?;
        Ok(Calendar { data })
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_named(stmt, ":id", self.id())?;
        bind_named(stmt, ":data", serde_json::to_string(&self.to_json())?)?;
        bind_named(stmt, ":accountId", self.account_id())?;
        Ok(())
    }
}

/// One VEVENT as the ICS parser read it, for [`Event::create`] and
/// [`Event::apply_ics_event_data`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IcsEventFields {
    pub uid: String,
    /// `RECURRENCE-ID`, "" for a master.
    pub recurrence_id: String,
    /// `STATUS`, "" when absent (stored as `CONFIRMED`).
    pub status: String,
    /// `rs`: DTSTART in epoch seconds.
    pub recurrence_start: i64,
    /// `re`: the end of the last occurrence (`endOf`): DTEND (or DTSTART) without an RRULE,
    /// the RRULE's UNTIL, else `20370000T000000`.
    pub recurrence_end: i64,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub attendees: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub data: Value,
    // `EventSearch` columns from the last applied ICS. Not persisted: an event loaded from
    // the database or client JSON has none and leaves its search row alone on save.
    search_title: String,
    search_description: String,
    search_location: String,
    search_participants: String,
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Event {
    pub fn new(data: Value) -> Self {
        Event {
            data,
            search_title: String::new(),
            search_description: String::new(),
            search_location: String::new(),
            search_participants: String::new(),
        }
    }

    /// `Event(etag, accountId, calendarId, ics, event)`.
    pub fn create(
        etag: &str,
        account_id: &str,
        calendar_id: &str,
        ics: &str,
        fields: &IcsEventFields,
    ) -> Self {
        let id = airpost_core::ids::event_id(
            account_id,
            calendar_id,
            &fields.uid,
            &fields.recurrence_id,
        );
        let mut e = Event::new(base_json(&id, account_id, 0));
        e.data["cid"] = json!(calendar_id);
        e.data["icsuid"] = json!(fields.uid);
        e.apply_ics_event_data(etag, "", ics, fields);
        e
    }

    pub fn calendar_id(&self) -> &str {
        json_str(&self.data, "cid")
    }

    pub fn set_calendar_id(&mut self, id: &str) {
        self.data["cid"] = json!(id);
    }

    pub fn href(&self) -> &str {
        json_str(&self.data, "href")
    }

    pub fn set_href(&mut self, href: &str) {
        self.data["href"] = json!(href);
    }

    pub fn ics_data(&self) -> &str {
        json_str(&self.data, "ics")
    }

    pub fn set_ics_data(&mut self, ics: &str) {
        self.data["ics"] = json!(ics);
    }

    pub fn ics_uid(&self) -> &str {
        json_str(&self.data, "icsuid")
    }

    /// "" on an event from client JSON, which carries no etag.
    pub fn etag(&self) -> &str {
        json_str(&self.data, "etag")
    }

    pub fn set_etag(&mut self, etag: &str) {
        self.data["etag"] = json!(etag);
    }

    pub fn recurrence_id(&self) -> &str {
        json_str(&self.data, "rid")
    }

    pub fn set_recurrence_id(&mut self, rid: &str) {
        self.data["rid"] = json!(rid);
    }

    pub fn status(&self) -> &str {
        self.data
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("CONFIRMED")
    }

    pub fn set_status(&mut self, status: &str) {
        self.data["status"] = json!(status);
    }

    pub fn is_recurrence_exception(&self) -> bool {
        !self.recurrence_id().is_empty()
    }

    pub fn recurrence_start(&self) -> i64 {
        json_i64(&self.data["rs"])
    }

    pub fn recurrence_end(&self) -> i64 {
        json_i64(&self.data["re"])
    }

    /// `applyICSEventData`: the fields of a fetched or updated VEVENT, plus the search text
    /// the next save writes to `EventSearch`.
    pub fn apply_ics_event_data(
        &mut self,
        etag: &str,
        href: &str,
        ics: &str,
        fields: &IcsEventFields,
    ) {
        self.set_etag(etag);
        if !href.is_empty() {
            self.set_href(href);
        }
        self.set_ics_data(ics);
        self.set_recurrence_id(&fields.recurrence_id);
        self.set_status(if fields.status.is_empty() {
            "CONFIRMED"
        } else {
            &fields.status
        });
        self.data["rs"] = json!(fields.recurrence_start);
        self.data["re"] = json!(fields.recurrence_end);
        self.search_title = fields.summary.clone();
        self.search_description = fields.description.clone();
        self.search_location = fields.location.clone();
        self.search_participants = fields.attendees.join(" ");
    }

    fn has_search_content(&self) -> bool {
        !(self.search_title.is_empty()
            && self.search_description.is_empty()
            && self.search_location.is_empty()
            && self.search_participants.is_empty())
    }
}

impl Model for Event {
    model_boilerplate!(
        "Event",
        [
            "id",
            "data",
            "icsuid",
            "recurrenceId",
            "accountId",
            "etag",
            "calendarId",
            "recurrenceStart",
            "recurrenceEnd"
        ]
    );

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "Event")?;
        Ok(Event::new(data))
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_named(stmt, ":id", self.id())?;
        bind_named(stmt, ":data", serde_json::to_string(&self.to_json())?)?;
        bind_named(stmt, ":icsuid", self.ics_uid())?;
        bind_named(stmt, ":recurrenceId", self.recurrence_id())?;
        bind_named(stmt, ":accountId", self.account_id())?;
        bind_named(stmt, ":etag", self.etag())?;
        bind_named(stmt, ":calendarId", self.calendar_id())?;
        bind_named(stmt, ":recurrenceStart", self.recurrence_start())?;
        bind_named(stmt, ":recurrenceEnd", self.recurrence_end())?;
        Ok(())
    }

    fn before_save(&mut self, store: &Store) -> Result<()> {
        if self.has_search_content() {
            let id = self.id().to_owned();
            let is_new = self.version() <= 1;
            super::search_row::assign(store, "EventSearch", &id, is_new, &mut self.data)?;
        }
        Ok(())
    }

    fn after_save(&mut self, store: &Store) -> Result<()> {
        if !self.has_search_content() {
            return Ok(());
        }
        let rowid = json_i64(&self.data["searchRowId"]);
        super::search_row::write(
            store,
            "EventSearch",
            rowid,
            self.id(),
            &["title", "description", "location", "participants"],
            &[
                &self.search_title,
                &self.search_description,
                &self.search_location,
                &self.search_participants,
            ],
        )
    }

    fn after_remove(&mut self, store: &Store) -> Result<()> {
        super::search_row::remove(store, "EventSearch", self.id(), &self.data)
    }
}
