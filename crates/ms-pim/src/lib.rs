// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Calendar and contact editing, as the desktop's `ics-event-helpers.ts`,
//! `calendar-data-source.ts`, `calendar-utils.ts`, `ContactInfoMapping.ts` and
//! `VCFImportExport.ts` did it, so the engine socket's
//! `events.*` and `contacts.*` methods can serve every client.
//!
//! - [`ics`]: a lossless line model of an iCalendar object.
//! - [`time`]: DATE and DATE-TIME values and the zones they are read in.
//! - [`edit`]: the edits (times, recurrence exceptions, exclusion dates, guests, SEQUENCE,
//!   iTIP REPLY and COUNTER).
//! - [`expand`]: recurrence expansion through calcard, with the desktop's iteration budget.
//! - [`occurrence`]: the occurrences the calendar draws, and conflicts.
//! - [`plan`]: `events.create`, `events.update` and `events.rsvp` as writes to queue.
//! - [`contact`] and [`vcf`]: the shared contact shape over vCard and Google People, and VCF
//!   import and export.
//!
//! Nothing here touches the store or the network; the engine reads rows, calls in, and
//! queues the tasks the result describes.

pub mod contact;
pub mod edit;
pub mod expand;
pub mod ics;
pub mod occurrence;
pub mod plan;
pub mod time;
pub mod vcf;

pub use time::Env;

/// Why an edit could not be made. `code` is stable and goes out as the socket error's
/// `data.code`; `message` is for logs and the CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_ics(message: impl Into<String>) -> Self {
        Error::new("invalid-ics", message)
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Error::new("invalid-params", message)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// The address a participant URI names, lowercased: `mailto:` stripped, or a bare address,
/// or the first address inside anything else. `None` when it holds none.
pub fn email_from_participant_uri(uri: &str) -> Option<String> {
    if uri.is_empty() {
        return None;
    }
    if uri.len() >= 7 && uri[..7].eq_ignore_ascii_case("mailto:") {
        let email = uri[7..].to_lowercase();
        return email.contains('@').then_some(email);
    }
    if is_address(uri) {
        return Some(uri.to_lowercase());
    }
    let bytes: Vec<char> = uri.chars().collect();
    let at = bytes.iter().position(|c| *c == '@')?;
    let stop = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ':' | '<' | '>' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']'
            )
    };
    let mut from = at;
    while from > 0 && !stop(bytes[from - 1]) {
        from -= 1;
    }
    let mut to = at + 1;
    while to < bytes.len() && !stop(bytes[to]) {
        to += 1;
    }
    let candidate: String = bytes[from..to].iter().collect();
    is_address(&candidate).then(|| candidate.to_lowercase())
}

fn is_address(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !s.chars().any(|c| {
            c.is_whitespace()
                || matches!(c, '<' | '>' | '"' | ',' | ';' | ':' | '(' | ')' | '[' | ']')
        })
}

/// Whether two addresses are the same mailbox (case-insensitive, as `Utils.emailIsEquivalent`).
pub fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_uris() {
        assert_eq!(
            email_from_participant_uri("mailto:A@B.com").as_deref(),
            Some("a@b.com")
        );
        assert_eq!(
            email_from_participant_uri("MAILTO:x@y.org").as_deref(),
            Some("x@y.org")
        );
        assert_eq!(email_from_participant_uri("mailto:nobody"), None);
        assert_eq!(
            email_from_participant_uri("Bob@Example.com").as_deref(),
            Some("bob@example.com")
        );
        assert_eq!(
            email_from_participant_uri("invalid:user@example.com").as_deref(),
            Some("user@example.com")
        );
        assert_eq!(email_from_participant_uri("urn:uuid:1234"), None);
        assert_eq!(email_from_participant_uri(""), None);
    }
}
