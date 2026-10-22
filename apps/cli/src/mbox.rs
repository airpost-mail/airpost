// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! mbox framing, as the desktop's `apps/desktop/src/mail/mbox.ts` writes it: the mboxrd
//! variant, with LF line endings and every body line matching `^>*From ` quoted with one
//! more `>`, each message after a `From <sender> <asctime UTC>` line and followed by an empty
//! line. Messages are bytes throughout (the desktop reads them as latin1), so any charset or
//! transfer encoding passes through unchanged.

use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};

const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `mboxFromLine`: `From <sender> Thu Jan  1 00:00:00 2026\n`, the day of month padded with a
/// space.
pub fn from_line(sender: &str, date: DateTime<Utc>) -> String {
    use chrono::{Datelike, Timelike};
    format!(
        "From {sender} {} {} {:>2} {:02}:{:02}:{:02} {}\n",
        DAY_NAMES[date.weekday().num_days_from_sunday() as usize],
        MONTH_NAMES[date.month0() as usize],
        date.day(),
        date.hour(),
        date.minute(),
        date.second(),
        date.year()
    )
}

/// JavaScript's `\s` on a latin1 character.
fn is_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ' | 0xa0)
}

/// `raw` with every `\r\n` as `\n`.
fn lf(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' && raw.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

/// The first run of `class` characters that has an `@` with at least one character on each
/// side, the whole run; with `bracketed`, only a run right after `<` and right before `>`.
/// These are `/<([^<>\s]+@[^<>\s]+)>/` and `/([^\s<>,;:"'()]+@[^\s<>,;:"'()]+)/`.
fn find_address(value: &[u8], bracketed: bool) -> Option<&[u8]> {
    let in_class =
        |b: u8| !is_space(b) && !b"<>".contains(&b) && (bracketed || !b",;:\"'()".contains(&b));
    let mut i = 0;
    while i < value.len() {
        let start = if bracketed {
            if value[i] != b'<' {
                i += 1;
                continue;
            }
            i + 1
        } else if in_class(value[i]) {
            i
        } else {
            i += 1;
            continue;
        };
        let mut end = start;
        while end < value.len() && in_class(value[end]) {
            end += 1;
        }
        let run = &value[start..end];
        let has_at = run.len() >= 3 && run[1..run.len() - 1].contains(&b'@');
        if has_at && (!bracketed || value.get(end) == Some(&b'>')) {
            return Some(run);
        }
        i = end.max(i + 1);
    }
    None
}

/// `senderFromEml`: the envelope sender for the `From ` line, from Return-Path, else From;
/// `MAILER-DAEMON` when neither has an address.
pub fn sender_from_eml(raw: &[u8]) -> String {
    let normalized = lf(raw);
    let header_end = normalized
        .windows(2)
        .position(|w| w == b"\n\n")
        .unwrap_or(normalized.len());
    // Unfolded: a newline and the spaces and tabs after it become one space.
    let mut headers = Vec::with_capacity(header_end);
    let mut i = 0;
    while i < header_end {
        let b = normalized[i];
        if b == b'\n' && i + 1 < header_end && matches!(normalized[i + 1], b' ' | b'\t') {
            headers.push(b' ');
            i += 1;
            while i < header_end && matches!(normalized[i], b' ' | b'\t') {
                i += 1;
            }
            continue;
        }
        headers.push(b);
        i += 1;
    }
    // `^` and `$` with the `m` flag: lines end at `\n` and `\r`.
    let lines: Vec<&[u8]> = headers.split(|b| *b == b'\n' || *b == b'\r').collect();
    for name in [&b"return-path:"[..], b"from:"] {
        let Some(line) = lines
            .iter()
            .find(|l| l.len() >= name.len() && l[..name.len()].eq_ignore_ascii_case(name))
        else {
            continue;
        };
        let value = &line[name.len()..];
        if let Some(addr) = find_address(value, true).or_else(|| find_address(value, false)) {
            // Control characters would confuse mbox parsers on the separator line.
            return addr
                .iter()
                .filter(|b| !matches!(**b, 0x00..=0x1f | 0x7f))
                .map(|b| *b as char)
                .collect();
        }
    }
    "MAILER-DAEMON".into()
}

/// `transformEmlToMbox`: LF line endings, `>`-quoted From lines, a final newline.
pub fn eml_to_mbox(raw: &[u8]) -> Vec<u8> {
    let body = lf(raw);
    let mut out = Vec::with_capacity(body.len() + 16);
    let mut line_start = true;
    for (i, &b) in body.iter().enumerate() {
        if line_start {
            let rest = &body[i..];
            let quotes = rest.iter().take_while(|c| **c == b'>').count();
            if rest[quotes..].starts_with(b"From ") {
                out.push(b'>');
            }
        }
        out.push(b);
        line_start = b == b'\n' || b == b'\r';
    }
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    out
}

/// One message as it goes into the mbox, dated `date` (the .eml file's modification time,
/// which the export sets to the message's date).
pub fn mbox_message(raw: &[u8], date: DateTime<Utc>) -> Vec<u8> {
    let mut out = from_line(&sender_from_eml(raw), date).into_bytes();
    out.extend(eml_to_mbox(raw));
    out.push(b'\n');
    out
}

/// Appends every `.eml` file of `dir`, in name order (the export's index order), to `out`.
/// Returns how many.
pub fn assemble(dir: &Path, out: &mut impl Write) -> std::io::Result<usize> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("eml"))
        })
        .collect();
    files.sort();
    for path in &files {
        let raw = std::fs::read(path)?;
        let modified: DateTime<Utc> = std::fs::metadata(path)?.modified()?.into();
        out.write_all(&mbox_message(&raw, modified))?;
    }
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_lines_use_asctime_in_utc() {
        let d = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            from_line("a@b.c", d),
            "From a@b.c Thu Jan  1 00:00:00 2026\n"
        );
        let d = DateTime::parse_from_rfc3339("2026-10-22T09:05:07+02:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(from_line("x@y", d), "From x@y Thu Oct 22 07:05:07 2026\n");
    }

    #[test]
    fn the_sender_prefers_return_path_then_from() {
        let s = |raw: &str| sender_from_eml(raw.as_bytes());
        assert_eq!(
            s("Return-Path: <bounce@lists.example>\r\nFrom: Ann <ann@example.com>\r\n\r\nbody"),
            "bounce@lists.example"
        );
        assert_eq!(s("From: Ann <ann@example.com>\r\n\r\n"), "ann@example.com");
        assert_eq!(s("from: ann@example.com (Ann)\n\n"), "ann@example.com");
        // Folded header.
        assert_eq!(
            s("From: Ann\r\n  <ann@example.com>\r\n\r\n"),
            "ann@example.com"
        );
        // An empty Return-Path falls back to From.
        assert_eq!(s("Return-Path: <>\nFrom: b@x.y\n\n"), "b@x.y");
        assert_eq!(s("Subject: hi\n\nFrom: body@x.y\n"), "MAILER-DAEMON");
        // Only the headers count, and `@` needs something on each side.
        assert_eq!(s("From: @x\n\n"), "MAILER-DAEMON");
        assert_eq!(s("From: \"a\" <a@b\x01c>\n\n"), "a@bc");
    }

    #[test]
    fn bodies_are_quoted_and_end_with_a_newline() {
        let raw = b"Subject: x\r\n\r\nFrom here\r\n>From there\r\nnot From\r\nend";
        assert_eq!(
            eml_to_mbox(raw),
            b"Subject: x\n\n>From here\n>>From there\nnot From\nend\n".to_vec()
        );
        let m = mbox_message(b"From: a@b.c\n\nhi\n", DateTime::UNIX_EPOCH);
        assert_eq!(
            String::from_utf8(m).unwrap(),
            "From a@b.c Thu Jan  1 00:00:00 1970\nFrom: a@b.c\n\nhi\n\n"
        );
    }
}
