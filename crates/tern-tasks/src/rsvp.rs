// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! EventRSVPTask: an iTIP REPLY or COUNTER sent to the organizer as an iMIP message.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp` (`performRemoteSendRSVP`),
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! The ICS is checked against RFC 5546 before anything is sent, and the message is
//! `multipart/alternative` of a text summary and `text/calendar; method=…` (RFC 6047
//! section 2.4). Nothing is placed in Sent.

use serde_json::Value;
use tern_core::{Address, Result, SyncError};
use tern_dav::ical::parse_events;
use tern_mime::{OutgoingBody, OutgoingMessage};
use tracing::{info, warn};

use crate::json::{at, get_str, has};
use crate::TaskProcessor;

fn invalid_ics(text: impl Into<String>) -> tern_core::Error {
    SyncError::fatal("invalid-ics", text).into()
}

/// The email of an icalendar attendee or organizer, written `Name <email>` or `email`.
fn email_of(attendee: &str) -> &str {
    match (attendee.find('<'), attendee.find('>')) {
        (Some(start), Some(end)) if end > start => &attendee[start + 1..end],
        _ => attendee,
    }
}

/// Whether any content line (after unfolding, RFC 5545 section 3.1) is a UID property.
/// The event parser numbers events without one, so it cannot tell.
fn has_uid_property(ics: &str) -> bool {
    let unfolded = ics
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "");
    unfolded.lines().any(|line| {
        let upper = line.trim_start().to_ascii_uppercase();
        upper.starts_with("UID:") || upper.starts_with("UID;")
    })
}

/// The human-readable part, by RSVP status (RFC 6047 recommends one).
fn summary_text(
    data: &Value,
    method: &str,
    status: &str,
    from: &str,
    event: &str,
) -> Result<String> {
    if method == "COUNTER" {
        let mut text = format!("{from} proposed a new time for: {event}");
        if has(data, "comment") {
            let comment = get_str(at(data, "comment"))?;
            if !comment.is_empty() {
                text.push_str("\r\n\r\n");
                text.push_str(&comment);
            }
        }
        return Ok(text);
    }
    Ok(match status {
        "ACCEPTED" => format!("{from} has accepted the invitation to: {event}"),
        "DECLINED" => format!("{from} has declined the invitation to: {event}"),
        "TENTATIVE" => format!("{from} has tentatively accepted the invitation to: {event}"),
        _ => format!("{from} has responded to the invitation: {event}"),
    })
}

impl TaskProcessor<'_> {
    /// `performRemoteSendRSVP`.
    pub(crate) fn perform_remote_send_rsvp(&self, data: &Value) -> Result<()> {
        if !has(data, "ics") || !has(data, "subject") || !has(data, "to") {
            return Err(SyncError::fatal(
                "missing-json",
                "Missing required fields: ics, subject, or to",
            )
            .into());
        }
        let ics = get_str(at(data, "ics"))?;
        let subject = get_str(at(data, "subject"))?;
        let organizer = get_str(at(data, "to"))?;
        let status = if has(data, "icsRSVPStatus") {
            get_str(at(data, "icsRSVPStatus"))?
        } else {
            "ACCEPTED".to_owned()
        };
        // REPLY answers the invitation; COUNTER proposes a different time (RFC 5546
        // section 3.2.7). Absent means REPLY.
        let method = if has(data, "method") {
            get_str(at(data, "method"))?
        } else {
            "REPLY".to_owned()
        };
        if method != "REPLY" && method != "COUNTER" {
            return Err(invalid_ics(format!("Unsupported iTIP method: {method}")));
        }

        // RFC 5546: the ICS declares the method it is sent as.
        if !ics.contains(&format!("METHOD:{method}")) {
            return Err(invalid_ics(format!(
                "ICS data must contain METHOD:{method}"
            )));
        }
        let events = parse_events(&ics);
        let Some(event) = events.first() else {
            return Err(invalid_ics(
                "ICS data does not contain any VEVENT components",
            ));
        };
        // RFC 5546 section 3.2.3: UID (matching the REQUEST), DTSTAMP and ORGANIZER.
        if !has_uid_property(&ics) {
            return Err(invalid_ics(format!(
                "ICS {method} must contain UID property matching the original invitation"
            )));
        }
        if !event.has_dtstamp {
            return Err(invalid_ics(format!(
                "ICS {method} must contain DTSTAMP property"
            )));
        }
        if event.organizer.is_empty() {
            return Err(invalid_ics(format!(
                "ICS {method} must contain ORGANIZER property"
            )));
        }
        // A REPLY carries exactly one ATTENDEE, the person replying (section 3.2.3). A
        // COUNTER carries the proposing attendee and may repeat the guest list (3.2.7).
        if method == "REPLY" && event.attendees.len() != 1 {
            return Err(invalid_ics(format!(
                "ICS REPLY must contain exactly one ATTENDEE (the replying user), found {}",
                event.attendees.len()
            )));
        }
        if method == "COUNTER" && event.attendees.is_empty() {
            return Err(invalid_ics(
                "ICS COUNTER must contain the ATTENDEE making the proposal",
            ));
        }
        // A REPLY states a participation status (RFC 5545 section 3.2.12); a COUNTER states
        // a time instead.
        if method == "REPLY" {
            let has_partstat = [
                "PARTSTAT=ACCEPTED",
                "PARTSTAT=DECLINED",
                "PARTSTAT=TENTATIVE",
            ]
            .iter()
            .any(|p| ics.contains(p));
            if !has_partstat {
                return Err(invalid_ics(
                    "ATTENDEE must have valid PARTSTAT parameter (ACCEPTED, DECLINED, or TENTATIVE)",
                ));
            }
        } else if event.start.is_none() {
            return Err(invalid_ics("ICS COUNTER must contain the proposed DTSTART"));
        }

        // RFC 6047: From should be the ATTENDEE, or the organizer's calendar may reject the
        // reply. Aliases and forwarding make a mismatch legitimate, so it only warns. In a
        // COUNTER the sender is whichever attendee matches the account.
        let from_email = self.account.email_address().to_owned();
        let lower_from = from_email.to_lowercase();
        let attendee_email = event
            .attendees
            .iter()
            .map(|a| email_of(a))
            .find(|e| e.to_lowercase() == lower_from)
            .unwrap_or_else(|| email_of(&event.attendees[0]));
        if attendee_email.to_lowercase() != lower_from {
            warn!(
                "RSVP From address ({from_email}) does not match ATTENDEE email in ICS ({attendee_email}). This may cause the RSVP to be rejected."
            );
        }

        let event_summary = if event.summary.is_empty() {
            "Calendar Event"
        } else {
            &event.summary
        };
        let text = summary_text(data, &method, &status, &from_email, event_summary)?;

        let me = Address::new(None, &from_email);
        let message = OutgoingMessage {
            from: vec![me.clone()],
            to: vec![Address::new(None, &organizer)],
            reply_to: vec![me],
            subject,
            date: self.store.now(),
            body: OutgoingBody::Calendar {
                text,
                method: method.clone(),
                ics,
            },
            ..Default::default()
        };
        let raw = tern_mime::build(&message)?;

        let mut mailer = (self.mailer)(&self.account)?;
        let what = if method == "REPLY" {
            format!("{method} ({status})")
        } else {
            method.clone()
        };
        info!("-- Sending RFC 6047-compliant {what} to organizer {organizer}");
        if let Err(e) = mailer.send(&raw) {
            info!(
                "-X An SMTP error occurred: {} SMTP code: {} ({})",
                e.code.name(),
                mailer.last_response_code(),
                e.detail
            );
            return Err(SyncError::fatal("send-failed", e.code.name()).into());
        }
        info!("-- {method} sent successfully");
        Ok(())
    }
}
