// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Calendar and contact editing (docs/desktop-client-plan.md 3.8, stage 18,
//! [`feature::CALENDAR_EDITING`](crate::feature::CALENDAR_EDITING) and
//! [`feature::CONTACT_EDITING`](crate::feature::CONTACT_EDITING)): occurrences expanded by
//! the engine, and edits built by it as ICS and vCards and queued as the tasks a client
//! would queue (`SyncbackEventTask`, `DestroyEventTask`, `EventRSVPTask`,
//! `SyncbackContactTask`).
//!
//! Times are epoch seconds. `tz` is the IANA (or Windows) zone floating times, DATE values
//! and all-day events are read in, the user's; UTC when absent.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `events.expand`: the occurrences of stored events that overlap `[start, end]`, or of an
/// ICS that is not stored (`ics`, an emailed invitation). Rows are chosen by `eventIds`,
/// else `calendarIds`, else every calendar of `accountIds` (every account by default); an
/// event id brings every row of its UID.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsExpandParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ids: Option<Vec<String>>,
    /// An ICS to expand instead of stored rows, read as the account `accountIds` names
    /// first would have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ics: Option<String>,
    pub start: i64,
    pub end: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// With `eventIds`: also answer `focused`, the occurrence to show for an event opened
    /// from outside the calendar. With `ics`: also answer `upcoming` and `first`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<EventsFocus>,
    /// Also answer `conflicts`: the timed occurrences of the chosen rows that overlap
    /// `[start, end)` and that the account attends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<EventsConflicts>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsFocus {
    /// The time to look forward from.
    pub now: i64,
    /// The occurrence to focus, by the slot it fills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence_id_start: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsConflicts {
    /// The invitation being answered, left out of its own conflicts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_icsuid: Option<String>,
}

/// `events.expand`'s answer. An occurrence has the desktop's `EventOccurrence` keys
/// (`id`, `eventId`, `accountId`, `calendarId`, `title`, `start`, `end`, `startDate`,
/// `endDate` as days since 1970-01-01, `isAllDay`, `isRecurring`, `isException`,
/// `recurrenceIdStart`, `organizer`, `attendees`, `isMine`, `myStatus`, ...).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsExpandResult {
    pub occurrences: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<Value>,
    /// `{start, end, isAllDay, recurrenceId?}`; null once a series is over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upcoming: Option<Value>,
    /// With `ics` and `focus`: the invitation's own times, from its first VEVENT, in the
    /// same shape; `recurrenceId` when it is about one occurrence of a series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<Vec<Value>>,
}

/// A guest on an event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventGuest {
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `REQ-PARTICIPANT` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// `events.create`: a new event on a calendar, organised by the account when it has guests.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsCreateParams {
    pub account_id: String,
    pub calendar_id: String,
    pub summary: String,
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub is_all_day: bool,
    /// The zone the times are written in; UTC when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<EventGuest>,
    /// An RRULE value (`FREQ=WEEKLY;BYDAY=MO`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence_rule: Option<String>,
    /// `OPAQUE` or `TRANSPARENT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// False: answer with the writes without queueing them (a client that queues them
    /// itself, for undo). True by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<bool>,
}

/// `events.update`: `patch` on one occurrence, that one and the ones after, or the whole
/// series. On an event that does not repeat every scope is `all`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsUpdateParams {
    pub account_id: String,
    pub event_id: String,
    /// `start`, `end`, `isAllDay`, `timezone`, `summary`, `location`, `description`,
    /// `attendees` (`[{email, name?, role?}]`, the whole list), `transparency`,
    /// `recurrenceRule` (null removes it), `deleted: true` (removes the occurrences the
    /// scope covers).
    pub patch: Map<String, Value>,
    /// `this`, `following` or `all` (default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// For `this` and `following`: the occurrence, by the slot it fills (its
    /// `recurrenceIdStart`, else its start). Not needed for an exception row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_start: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<bool>,
}

/// One calendar write an edit made: `update` and `destroy` name the row, `create` the
/// calendar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventWrite {
    /// `create`, `update` or `destroy`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ics: Option<String>,
    /// The task queued for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// `events.create` and `events.update`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsWriteResult {
    pub writes: Vec<EventWrite>,
    /// `events.create`: the new event's UID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icsuid: Option<String>,
}

/// A counter-proposal's time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsCounter {
    pub start: i64,
    pub end: i64,
}

/// `events.rsvp`: our answer to an invitation, on our copy (`eventId`) or on an emailed
/// one (`ics`, with `messageId`), written to our copy and emailed to the organizer as a
/// REPLY; or, with `counter`, a COUNTER proposing another time, with nothing written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsRsvpParams {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ics: Option<String>,
    /// The message the invitation came in; the REPLY answers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// `ACCEPTED`, `TENTATIVE` or `DECLINED`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<EventsCounter>,
    /// The one occurrence of a series to answer, by the slot it fills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_start: Option<i64>,
    /// With `ics` and no stored copy: the calendar to add it to when attending; the
    /// account's first writable calendar by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
    /// False: don't email the organizer. True by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<bool>,
    /// False: don't write our copy. True by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<bool>,
    /// The subject; `Accepted: <title>` (or `New time proposed: <title>`) by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// Set on an answer made again on a copy that changed under the first; not retried a
    /// second time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retried: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsRsvpResult {
    /// Our address on the guest list.
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<EventWrite>,
    /// The `EventRSVPTask`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_task_id: Option<String>,
}

/// `contacts.get`: a contact's details in the shape the contact editor shows (`name`,
/// `emailAddresses`, `phoneNumbers`, `addresses`, `company`, `title`, `birthdays`, `urls`,
/// `nicknames`, `relations`, `notes`, `photoURL`), and where they come from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsGetParams {
    pub account_id: String,
    pub contact_id: String,
}

/// `contacts.get`'s answer, and `contacts.create`'s and `contacts.update`'s.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDetails {
    /// The Contact JSON.
    pub contact: Value,
    /// `{origin, readonly}`.
    pub metadata: Value,
    pub data: Value,
    /// The `SyncbackContactTask`, for a write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// `contacts.create`: a new contact in the account's address book (CardDAV, or Google
/// People on a Google account), with `data` in the `contacts.get` shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsCreateParams {
    pub account_id: String,
    pub data: Map<String, Value>,
}

/// `contacts.update`: the fields of `patch` (the `contacts.get` shape, top-level keys
/// replacing the stored ones) written to the contact's vCard or Google person.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsUpdateParams {
    pub account_id: String,
    pub contact_id: String,
    pub patch: Map<String, Value>,
}

/// `contacts.import`: the vCards of a .vcf file as new CardDAV contacts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsImportParams {
    pub account_id: String,
    pub vcf: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsImportResult {
    pub contact_ids: Vec<String>,
    pub task_ids: Vec<String>,
}

/// `contacts.export`: the contacts as one .vcf file, CardDAV cards as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsExportParams {
    pub account_id: String,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsExportResult {
    pub vcf: String,
}
