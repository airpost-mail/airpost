//! Event tasks: SyncbackEvent and DestroyEvent, written through CalDAV.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp` (`performLocalSyncbackEvent`
//! through `performRemoteDestroyEvent`), Copyright © 2017 Foundry 376, GPL-3.0. See
//! NOTICE.md.
//!
//! Differences from the C++: a SyncbackEvent whose write the server refuses does not leave
//! the client's ICS in the row. A never-written event is removed; any other is re-read from
//! the server (C++ kept the refused edit until the event next changed on the server).

use ms_core::{Error, Result, SyncError};
use ms_dav::{ical, DeleteOutcome};
use ms_store::{Event, Model, Query};
use ms_sync::dav::{ics_event_fields, DavWorker};
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::contacts::not_found;
use crate::json::{at, elements, get_str, has};
use crate::TaskProcessor;

/// The task classes this module handles.
pub const EVENT_TASKS: [&str; 2] = ["SyncbackEventTask", "DestroyEventTask"];

fn invalid_ics(what: &str) -> Error {
    SyncError::fatal("invalid-ics", what).into()
}

impl TaskProcessor<'_> {
    pub(crate) fn perform_local_event_task(&self, class: &str, data: &mut Value) -> Result<()> {
        match class {
            "SyncbackEventTask" => self.perform_local_syncback_event(data),
            // Rows go in the remote phase, once the server accepts: a refused DELETE leaves
            // the ctag unchanged, so no later sync would restore rows removed here.
            _ => Ok(()),
        }
    }

    pub(crate) fn perform_remote_event_task(&self, class: &str, data: &Value) -> Result<()> {
        match class {
            "SyncbackEventTask" => self.perform_remote_syncback_event(data),
            _ => self.perform_remote_destroy_event(data),
        }
    }

    /// Applies the client's ICS to the stored event, or creates the event. A new event's id
    /// (derived from account, calendar, UID and RECURRENCE-ID) goes back into the task.
    fn perform_local_syncback_event(&self, data: &mut Value) -> Result<()> {
        let event_json = at(data, "event").clone();
        let calendar_id = get_str(at(data, "calendarId"))?;
        let event_id = if has(&event_json, "id") {
            get_str(at(&event_json, "id"))?
        } else {
            String::new()
        };
        let existing = if event_id.is_empty() {
            None
        } else {
            self.store
                .find::<Event>(&Query::new().equal("id", &event_id))?
        };

        if let Some(mut existing) = existing {
            if has(&event_json, "ics") {
                existing.set_ics_data(&get_str(at(&event_json, "ics"))?);
                let events = ical::parse_events(existing.ics_data());
                if let Some(m) = ical::matching_event(&events, existing.recurrence_id()) {
                    existing.data["rs"] = json!(m.recurrence_start);
                    existing.data["re"] = json!(m.recurrence_end);
                    existing.data["icsuid"] = json!(m.uid);
                    existing.set_recurrence_id(&m.recurrence_id);
                    existing.set_status(m.status_or_default());
                }
            }
            self.store.save(&mut existing)?;
            data["event"]["id"] = Value::from(existing.id());
            return Ok(());
        }

        let ics = get_str(at(&event_json, "ics"))?;
        let events = ical::parse_events(&ics);
        // The series master is the event's identity; the file may also carry RECURRENCE-ID
        // overrides, in any order.
        let Some(master) = ical::series_master(&events) else {
            return Err(invalid_ics("ICS data does not contain any events"));
        };
        if master.uid.is_empty() || ical::is_placeholder_uid(&master.uid) {
            return Err(invalid_ics("VEVENT has no UID"));
        }
        let fields = ics_event_fields(master);

        // Answering a synced invitation creates an event the calendar already holds: match on
        // UID and RECURRENCE-ID as the sync does, or the server answers 409 no-uid-conflict.
        // Failing that, the derived id: an earlier update can have rewritten a row's UID
        // columns, and inserting over its id would violate the primary key.
        let by_uid = Query::new()
            .equal("calendarId", calendar_id.as_str())
            .equal("icsuid", master.uid.as_str())
            .equal("recurrenceId", master.recurrence_id.as_str());
        let mut existing = self.store.find::<Event>(&by_uid)?;
        if existing.is_none() {
            let derived = Event::create("", self.account.id(), &calendar_id, &ics, &fields);
            existing = self
                .store
                .find::<Event>(&Query::new().equal("id", derived.id()))?;
        }

        let mut event = match existing {
            Some(mut e) => {
                // Each row stores the whole resource; a file with fewer VEVENTs for this UID
                // would, once PUT, erase the server's exceptions to the series.
                let stored = ical::count_with_uid(&ical::parse_events(e.ics_data()), &master.uid);
                let incoming = ical::count_with_uid(&events, &master.uid);
                if incoming < stored {
                    return Err(SyncError::fatal(
                        "ics-incomplete",
                        "The calendar already holds this series with exceptions; send the whole resource",
                    )
                    .into());
                }
                let (etag, href) = (e.etag().to_owned(), e.href().to_owned());
                e.apply_ics_event_data(&etag, &href, &ics, &fields);
                e
            }
            None => Event::create("", self.account.id(), &calendar_id, &ics, &fields),
        };
        self.store.save(&mut event)?;
        data["event"]["id"] = Value::from(event.id());
        Ok(())
    }

    fn perform_remote_syncback_event(&self, data: &Value) -> Result<()> {
        let id = get_str(at(at(data, "event"), "id"))?;
        let mut event = self
            .store
            .find::<Event>(
                &Query::new()
                    .equal("id", &id)
                    .equal("accountId", self.account.id()),
            )?
            .ok_or_else(|| not_found("Event not found for syncback"))?;
        let mut dav = self.dav()?;
        let failure = match dav.write_and_resync_event(&mut event) {
            Ok(()) => return Ok(()),
            Err(e @ Error::Database(_)) => return Err(e),
            Err(e) => e,
        };
        self.undo_refused_event_write(&mut dav, &mut event)?;
        Err(failure)
    }

    /// The local phase saved the client's ICS before the write; after a refusal the row
    /// would show it indefinitely. A never-written event (no href, no etag) is removed,
    /// since the sync ignores rows without an href; any other is re-read from its href,
    /// best effort.
    fn undo_refused_event_write(&self, dav: &mut DavWorker<'_>, event: &mut Event) -> Result<()> {
        if event.href().is_empty() && event.etag().is_empty() {
            info!(
                "Removing event {} that the server did not accept",
                event.id()
            );
            return self.store.remove(event);
        }
        match dav.restore_event_from_server(event) {
            Ok(true) => info!("Restored event {} from the server", event.id()),
            Ok(false) => {}
            Err(e @ Error::Database(_)) => return Err(e),
            Err(e) => warn!(
                "Could not restore event {} from the server: {}",
                event.id(),
                e.to_json()
            ),
        }
        Ok(())
    }

    /// Deletes each event from the server and then locally. One failure does not stop the
    /// rest, since this is the only thing that removes these rows. An event already gone
    /// from its stored href is removed locally; from a guessed href it may not be what was
    /// asked, so that is a failure.
    fn perform_remote_destroy_event(&self, data: &Value) -> Result<()> {
        let ids: Vec<String> = elements(at(data, "events"))
            .into_iter()
            .map(|e| get_str(at(e, "id")))
            .collect::<Result<_>>()?;
        let events = self.store.find_large_set::<Event>("id", &ids)?;
        if events.len() != ids.len() {
            warn!(
                "Destroying {} of {} requested events; the rest are no longer present",
                events.len(),
                ids.len()
            );
        }
        let total = events.len();
        let mut dav = self.dav()?;
        let mut failures: Vec<Error> = Vec::new();
        for mut event in events {
            let outcome = dav.delete_event(&mut event);
            let failure = match outcome {
                Ok(DeleteOutcome::Deleted) => continue,
                Ok(DeleteOutcome::AlreadyGone(_)) if !event.href().is_empty() => {
                    info!(
                        "Event {} was already gone from the server; removing locally",
                        event.id()
                    );
                    self.store.remove(&mut event)?;
                    continue;
                }
                Ok(DeleteOutcome::AlreadyGone(status)) => {
                    match ms_net::http::check_status(status, event.href(), "") {
                        Err(e) => e,
                        Ok(()) => continue,
                    }
                }
                Err(e @ Error::Sync(_)) => e,
                Err(e) => return Err(e),
            };
            error!(
                "Could not delete event {}: {}",
                event.id(),
                failure.to_json()
            );
            failures.push(failure);
        }
        match failures.len() {
            0 => Ok(()),
            // The reason itself, not a count, when there is only one.
            1 => Err(failures.remove(0)),
            n => {
                let first = failures[0]
                    .as_sync()
                    .map_or(String::new(), |s| s.key.clone());
                Err(SyncError::fatal(
                    "delete-failed",
                    format!("Could not delete {n} of {total} events; first: {first}"),
                )
                .into())
            }
        }
    }
}
