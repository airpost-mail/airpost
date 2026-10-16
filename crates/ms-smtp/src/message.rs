// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Message bytes on their way to DATA: the envelope read from the headers, the Bcc filter,
//! dot-stuffing, and the `test` mode email.
//!
//! Derived from mailcore2's `SMTPSession::dataWithFilteredBcc`, `sendMessage(Data *)` and
//! `checkAccount` (`MCSMTPSession.cpp`) and libetpan's `mailstream_send_data`
//! (`mailstream_helper.c`). mailcore2 is Copyright © 2001-2013 MailCore team and libetpan
//! Copyright © 2001-2005 DINH Viet Hoa, both BSD-3-Clause; Mailspring-Sync's changes are
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md ("mailcore2", "libEtPan!").

use lettre::message::header::{ContentType, UserAgent};
use lettre::message::Mailbox;
use lettre::Address;

/// The subject of the `test` mode email. The client and support docs refer to it by name.
pub const TEST_EMAIL_SUBJECT: &str = "Mailspring SMTP Test Email";
const TEST_EMAIL_FROM_NAME: &str = "Tern";
const TEST_EMAIL_USER_AGENT: &str = "Tern";
const TEST_EMAIL_BODY: &str = "This is an email sent by Tern while testing your account configuration.\r\n\r\nAs you've received it, everything is working.\r\n";

/// The envelope `sendMessage(Data *)` derives from a message: the first From address, and
/// every To, Cc and Bcc address in that order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HeaderEnvelope {
    pub from: Option<String>,
    pub recipients: Vec<String>,
}

pub fn envelope_from_headers(raw: &[u8]) -> HeaderEnvelope {
    let Some(msg) = mail_parser::MessageParser::default().parse_headers(raw) else {
        return HeaderEnvelope::default();
    };
    let addresses = |a: Option<&mail_parser::Address>| -> Vec<String> {
        a.map(|a| {
            a.iter()
                .filter_map(|addr| addr.address())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
    };
    let mut recipients = addresses(msg.to());
    recipients.extend(addresses(msg.cc()));
    recipients.extend(addresses(msg.bcc()));
    HeaderEnvelope {
        from: addresses(msg.from()).into_iter().next(),
        recipients,
    }
}

/// `dataWithFilteredBcc`: the wire copy carries no Bcc field, and a message left with no
/// To or Cc gets `To: Undisclosed recipients:;` so it is still well formed. The message is
/// returned unchanged when neither applies.
pub fn filter_bcc(raw: &[u8]) -> Vec<u8> {
    let (head, sep, body) = split_header(raw);
    let fields = header_fields(head);

    let mut has_recipient = false;
    let mut has_bcc = false;
    for field in &fields {
        match field_name(field).to_ascii_lowercase().as_str() {
            "to" | "cc" => has_recipient = true,
            "bcc" => has_bcc = true,
            _ => {}
        }
    }
    if has_recipient && !has_bcc {
        return raw.to_vec();
    }

    let eol: &[u8] = if head.windows(2).any(|w| w == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let mut out = Vec::with_capacity(raw.len() + 40);
    for field in fields {
        if !field_name(field).eq_ignore_ascii_case("bcc") {
            out.extend_from_slice(field);
        }
    }
    if !has_recipient {
        out.extend_from_slice(b"To: Undisclosed recipients:;");
        out.extend_from_slice(eol);
    }
    out.extend_from_slice(sep);
    out.extend_from_slice(body);
    out
}

/// Splits at the blank line ending the header. The header keeps its last line ending; `sep`
/// is the blank line itself.
fn split_header(raw: &[u8]) -> (&[u8], &[u8], &[u8]) {
    let mut line_start = 0;
    while line_start < raw.len() {
        let rest = &raw[line_start..];
        let sep_len = if rest.starts_with(b"\r\n") {
            2
        } else if rest.starts_with(b"\n") {
            1
        } else {
            0
        };
        if sep_len > 0 {
            return (
                &raw[..line_start],
                &raw[line_start..line_start + sep_len],
                &raw[line_start + sep_len..],
            );
        }
        match rest.iter().position(|&b| b == b'\n') {
            Some(p) => line_start += p + 1,
            None => break,
        }
    }
    (raw, b"", b"")
}

/// Header fields with their folded continuation lines and line endings.
fn header_fields(head: &[u8]) -> Vec<&[u8]> {
    let mut fields: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    while pos < head.len() {
        let end = head[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(head.len(), |p| pos + p + 1);
        let continuation = matches!(head[pos], b' ' | b'\t');
        if !continuation && pos > start {
            fields.push(&head[start..pos]);
            start = pos;
        }
        pos = end;
    }
    if pos > start {
        fields.push(&head[start..pos]);
    }
    fields
}

fn field_name(field: &[u8]) -> String {
    let name = field.split(|&b| b == b':').next().unwrap_or_default();
    String::from_utf8_lossy(name).trim().to_owned()
}

/// The DATA payload: CR, LF and CRLF line endings become CRLF, lines starting with `.` get
/// another, and the terminating `.` line follows (`mailstream_send_data`).
pub fn dot_stuff(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 50 + 5);
    let mut at_line_start = true;
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if at_line_start && b == b'.' {
            out.push(b'.');
        }
        at_line_start = false;
        match b {
            b'\r' => {
                out.extend_from_slice(b"\r\n");
                if raw.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                at_line_start = true;
            }
            b'\n' => {
                out.extend_from_slice(b"\r\n");
                at_line_start = true;
            }
            _ => out.push(b),
        }
        i += 1;
    }
    if !at_line_start {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

/// The email `checkAccount` sends to the account's own address. `None` when the address
/// cannot be written into a header.
pub fn build_test_email(address: &str) -> Option<Vec<u8>> {
    let addr: Address = address.parse().ok()?;
    let me = Mailbox::new(None, addr.clone());
    let msg = lettre::Message::builder()
        .from(Mailbox::new(Some(TEST_EMAIL_FROM_NAME.to_owned()), addr))
        .reply_to(me.clone())
        .to(me)
        .subject(TEST_EMAIL_SUBJECT)
        .date_now()
        // MessageHeader::init(true, true) always generated one (MCMessageHeader.cpp:29-31).
        .message_id(None)
        .header(UserAgent::from(TEST_EMAIL_USER_AGENT.to_owned()))
        .header(ContentType::TEXT_PLAIN)
        .body(TEST_EMAIL_BODY.to_owned())
        .ok()?;
    Some(msg.formatted())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcc_is_removed_with_its_continuation() {
        let raw = b"From: a@x\r\nTo: b@x\r\nBcc: c@x,\r\n d@x\r\nSubject: s\r\n\r\nbody\r\nBcc: not-a-header\r\n";
        assert_eq!(
            filter_bcc(raw),
            b"From: a@x\r\nTo: b@x\r\nSubject: s\r\n\r\nbody\r\nBcc: not-a-header\r\n"
        );
    }

    #[test]
    fn bcc_only_gets_undisclosed_recipients() {
        let raw = b"From: a@x\r\nBcc: c@x\r\nSubject: s\r\n\r\nbody";
        assert_eq!(
            filter_bcc(raw),
            b"From: a@x\r\nSubject: s\r\nTo: Undisclosed recipients:;\r\n\r\nbody"
        );
    }

    #[test]
    fn untouched_without_bcc() {
        let raw = b"From: a@x\r\nCc: b@x\r\n\r\nbody";
        assert_eq!(filter_bcc(raw), raw);
    }

    #[test]
    fn envelope_reads_to_cc_bcc_in_order() {
        let raw = b"From: \"A\" <a@x.test>\r\nTo: b@x.test, \"C\" <c@x.test>\r\nCc: grp: d@x.test;\r\nBcc: e@x.test\r\n\r\nbody";
        let env = envelope_from_headers(raw);
        assert_eq!(env.from.as_deref(), Some("a@x.test"));
        assert_eq!(
            env.recipients,
            vec!["b@x.test", "c@x.test", "d@x.test", "e@x.test"]
        );
    }

    #[test]
    fn dot_stuffing_and_line_endings() {
        assert_eq!(
            dot_stuff(b".a\nb\r.c\r\n..d"),
            b"..a\r\nb\r\n..c\r\n...d\r\n.\r\n"
        );
        assert_eq!(dot_stuff(b"x\r\n"), b"x\r\n.\r\n");
        assert_eq!(dot_stuff(b""), b".\r\n");
    }

    #[test]
    fn test_email_shape() {
        let raw = String::from_utf8(build_test_email("me@example.test").unwrap()).unwrap();
        assert!(raw.contains("Subject: Mailspring SMTP Test Email\r\n"));
        assert!(raw.contains("To: me@example.test\r\n"));
        assert!(raw.contains("Reply-To: me@example.test\r\n"));
        assert!(raw.contains("From: Tern <me@example.test>\r\n"));
        assert!(raw.contains("User-Agent: Tern\r\n"));
        assert!(raw.contains("\r\nMessage-ID: <"));
        assert!(build_test_email("not an address").is_none());
    }
}
