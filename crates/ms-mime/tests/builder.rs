// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Outgoing messages built, parsed back with the incoming parser, and compared.

use ms_core::Address;
use ms_mime::mime::{parse_tree, Part, PartKind};
use ms_mime::{
    build, build_filtering_bcc, parse_message, OutgoingAttachment, OutgoingBody, OutgoingMessage,
};

fn addr(name: &str, email: &str) -> Address {
    Address::new(Some(name).filter(|n| !n.is_empty()), email)
}

fn draft(body: OutgoingBody) -> OutgoingMessage {
    OutgoingMessage {
        from: vec![addr("Ålice Exämple", "alice@example.org")],
        to: vec![
            addr("Bob", "bob@example.net"),
            addr("", "carol@example.com"),
        ],
        cc: vec![addr("Dave, the Dev", "dave@dev.example")],
        bcc: vec![addr("Secret", "secret@example.org")],
        reply_to: vec![addr("", "replies@example.org")],
        subject: "Grüße — quarterly numbers 📈".to_owned(),
        message_id: "<draft-1@tern.example>".to_owned(),
        date: 1_767_693_600,
        body,
        ..Default::default()
    }
}

fn header_block(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let end = text.find("\r\n\r\n").expect("header end");
    text[..end].to_owned()
}

/// The values of a header field in the top-level block, unfolded.
fn header_values(raw: &[u8], name: &str) -> Vec<String> {
    let block = header_block(raw)
        .replace("\r\n ", " ")
        .replace("\r\n\t", " ");
    block
        .split("\r\n")
        .filter_map(|line| {
            let (n, v) = line.split_once(':')?;
            n.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
        .collect()
}

fn shape(p: &Part) -> String {
    match p.kind {
        PartKind::Single => p.mime_type.clone().unwrap_or_default(),
        PartKind::Message => format!(
            "message[{}]",
            p.parts.iter().map(shape).collect::<Vec<_>>().join(",")
        ),
        kind => format!(
            "{}[{}]",
            match kind {
                PartKind::Mixed => "mixed",
                PartKind::Related => "related",
                PartKind::Alternative => "alternative",
                PartKind::Signed => "signed",
                _ => unreachable!(),
            },
            p.parts.iter().map(shape).collect::<Vec<_>>().join(",")
        ),
    }
}

const HTML: &str = "<html><body><p>Hello <b>Bob</b>,</p><p>See <a href=\"https://example.org/q1\">the report</a> and the chart:</p><img src=\"cid:chart-1@tern.example\"><blockquote type=\"cite\">Earlier message</blockquote></body></html>";

#[test]
fn html_with_related_and_attachments_round_trips() {
    let png = b"\x89PNG\r\n\x1a\nfake image".to_vec();
    let pdf = (0..5000u32)
        .flat_map(|i| i.to_le_bytes())
        .collect::<Vec<u8>>();
    let mut msg = draft(OutgoingBody::Html(HTML.to_owned()));
    msg.attachments = vec![
        OutgoingAttachment {
            filename: "chart.png".to_owned(),
            mime_type: String::new(),
            content_id: Some("chart-1@tern.example".to_owned()),
            data: png.clone(),
        },
        OutgoingAttachment {
            filename: "Relatório Q1.pdf".to_owned(),
            mime_type: "application/pdf".to_owned(),
            content_id: None,
            data: pdf.clone(),
        },
    ];
    let raw = build(&msg).expect("build");

    let tree = parse_tree(&raw);
    assert_eq!(
        shape(&tree.root),
        "mixed[alternative[text/plain,related[text/html,image/png]],application/pdf]"
    );

    let parsed = parse_message(&raw).expect("parse");
    assert!(!parsed.plaintext);
    assert_eq!(parsed.body, HTML);
    assert_eq!(parsed.header.subject.as_deref(), Some(msg.subject.as_str()));
    assert_eq!(
        parsed.header.message_id.as_deref(),
        Some("draft-1@tern.example")
    );
    assert_eq!(parsed.header.from, vec![msg.from[0].clone()]);
    assert_eq!(parsed.header.to, msg.to);
    assert_eq!(parsed.header.cc, msg.cc);
    assert_eq!(parsed.header.reply_to, msg.reply_to);
    assert_eq!(parsed.header.date, Some(msg.date));

    assert_eq!(parsed.attachments.len(), 2);
    let pdf_file = &parsed.attachments[0];
    assert_eq!(pdf_file.filename, "Relatório Q1.pdf");
    assert_eq!(pdf_file.mime_type, "application/pdf");
    assert_eq!(pdf_file.data, pdf);
    assert!(!pdf_file.inline);
    assert_eq!(pdf_file.content_id, None);
    let chart = &parsed.attachments[1];
    assert_eq!(chart.filename, "chart.png");
    assert_eq!(chart.mime_type, "image/png", "type from the extension");
    assert_eq!(chart.data, png);
    assert!(chart.inline);
    assert_eq!(chart.content_id.as_deref(), Some("chart-1@tern.example"));
    assert!(!chart.counts_as_attachment());

    // The text alternative is the flattened HTML, links and quotes included.
    let alt = &tree.root.parts[0];
    let text = String::from_utf8(alt.parts[0].data.clone()).expect("utf-8 text part");
    assert!(text.contains("Hello Bob,"), "{text}");
    assert!(
        text.contains("the report (https://example.org/q1)"),
        "{text}"
    );
    assert!(text.contains("> Earlier message"), "{text}");
    assert_eq!(alt.parts[0].charset.as_deref(), Some("utf-8"));
}

#[test]
fn plaintext_draft_is_a_single_text_part() {
    let msg = draft(OutgoingBody::Text(
        "Line one\r\n> quoted\r\nÜmlaut line\r\n".to_owned(),
    ));
    let raw = build(&msg).expect("build");
    assert_eq!(shape(&parse_tree(&raw).root), "text/plain");
    let parsed = parse_message(&raw).expect("parse");
    assert!(parsed.plaintext);
    assert_eq!(
        parsed.body.replace("\r\n", "\n"),
        "Line one\n> quoted\nÜmlaut line\n"
    );
    assert!(parsed.attachments.is_empty());
}

#[test]
fn plaintext_draft_attaches_related_files_normally() {
    let mut msg = draft(OutgoingBody::Text("See the image.".to_owned()));
    msg.attachments = vec![OutgoingAttachment {
        filename: "inline.gif".to_owned(),
        mime_type: String::new(),
        content_id: Some("x@tern.example".to_owned()),
        data: b"GIF89a".to_vec(),
    }];
    let raw = build(&msg).expect("build");
    assert_eq!(shape(&parse_tree(&raw).root), "mixed[text/plain,image/gif]");
    let parsed = parse_message(&raw).expect("parse");
    assert_eq!(parsed.attachments[0].filename, "inline.gif");
}

#[test]
fn forwarded_eml_is_sent_as_octet_stream() {
    // message/rfc822 may not be base64 encoded (RFC 2046 5.2.1)
    let eml = b"From: x@example.org\r\nSubject: old\r\n\r\nBody \xff\r\n".to_vec();
    for mime_type in ["", "message/rfc822"] {
        let mut msg = draft(OutgoingBody::Text("Forwarding.".to_owned()));
        msg.attachments = vec![OutgoingAttachment {
            filename: "old.eml".to_owned(),
            mime_type: mime_type.to_owned(),
            content_id: None,
            data: eml.clone(),
        }];
        let raw = build(&msg).expect("build");
        assert_eq!(
            shape(&parse_tree(&raw).root),
            "mixed[text/plain,application/octet-stream]"
        );
        assert!(!String::from_utf8_lossy(&raw).contains("message/rfc822"));
        let parsed = parse_message(&raw).expect("parse");
        assert_eq!(parsed.attachments[0].filename, "old.eml");
        assert_eq!(parsed.attachments[0].data, eml);
    }
}

#[test]
fn calendar_reply_round_trips() {
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nUID:evt-1@example.org\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:alice@example.org\r\nSUMMARY:Planning — Q1\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let msg = OutgoingMessage {
        subject: "Accepted: Planning — Q1".to_owned(),
        body: OutgoingBody::Calendar {
            text: "Ålice has accepted: Planning — Q1".to_owned(),
            method: "REPLY".to_owned(),
            ics: ics.to_owned(),
        },
        ..draft(OutgoingBody::default())
    };
    let raw = build(&msg).expect("build");
    let tree = parse_tree(&raw);
    assert_eq!(shape(&tree.root), "alternative[text/plain,text/calendar]");
    let cal = &tree.root.parts[1];
    assert_eq!(String::from_utf8(cal.data.clone()).expect("ics"), ics);
    assert_eq!(cal.filename.as_deref(), Some("invite.ics"));
    assert!(cal.inline);

    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.contains("text/calendar; method=REPLY; charset=UTF-8")
            || text.contains("text/calendar; method=\"REPLY\"; charset=\"UTF-8\""),
        "{text}"
    );

    let parsed = parse_message(&raw).expect("parse");
    assert!(parsed.plaintext);
    assert_eq!(parsed.body, "Ålice has accepted: Planning — Q1");
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.attachments[0].mime_type, "text/calendar");
    assert_eq!(parsed.attachments[0].part_id, "2");
}

#[test]
fn priority_headers() {
    for (importance, want) in [
        ("high", ["high", "1 (Highest)", "High"]),
        ("low", ["low", "5 (Lowest)", "Low"]),
    ] {
        let msg = OutgoingMessage {
            importance: Some(importance.to_owned()),
            ..draft(OutgoingBody::Text("x".to_owned()))
        };
        let raw = build(&msg).expect("build");
        assert_eq!(header_values(&raw, "Importance"), [want[0]]);
        assert_eq!(header_values(&raw, "X-Priority"), [want[1]]);
        assert_eq!(header_values(&raw, "X-MSMail-Priority"), [want[2]]);
        let parsed = parse_message(&raw).expect("parse");
        assert_eq!(parsed.importance.as_deref(), Some(importance));
    }
    let msg = OutgoingMessage {
        importance: Some("normal".to_owned()),
        ..draft(OutgoingBody::Text("x".to_owned()))
    };
    let raw = build(&msg).expect("build");
    assert!(header_values(&raw, "Importance").is_empty());
    assert!(header_values(&raw, "X-Priority").is_empty());
}

#[test]
fn reply_and_forward_threading() {
    let reply = OutgoingMessage {
        in_reply_to: Some("<orig-1@example.net>".to_owned()),
        ..draft(OutgoingBody::Text("x".to_owned()))
    };
    let raw = build(&reply).expect("build");
    let parsed = parse_message(&raw).expect("parse");
    assert_eq!(parsed.header.in_reply_to, ["orig-1@example.net"]);
    assert_eq!(parsed.header.references, ["orig-1@example.net"]);

    let forward = OutgoingMessage {
        in_reply_to: Some("orig-1@example.net".to_owned()),
        forwarded_message_id: Some("<fwd-2@example.net>".to_owned()),
        ..draft(OutgoingBody::Text("x".to_owned()))
    };
    let parsed = parse_message(&build(&forward).expect("build")).expect("parse");
    assert_eq!(parsed.header.in_reply_to, ["orig-1@example.net"]);
    assert_eq!(
        parsed.header.references,
        ["fwd-2@example.net"],
        "forward replaces References"
    );

    let fresh = draft(OutgoingBody::Text("x".to_owned()));
    let raw = build(&fresh).expect("build");
    assert!(header_values(&raw, "In-Reply-To").is_empty());
    assert!(header_values(&raw, "References").is_empty());
}

#[test]
fn bcc_kept_for_sent_copy_and_filtered_for_the_wire() {
    let msg = draft(OutgoingBody::Text("x".to_owned()));
    let sent_copy = build(&msg).expect("build");
    let parsed = parse_message(&sent_copy).expect("parse");
    assert_eq!(parsed.header.bcc, msg.bcc);

    let wire = build_filtering_bcc(&msg, true).expect("build");
    assert!(header_values(&wire, "Bcc").is_empty());
    let parsed = parse_message(&wire).expect("parse");
    assert!(parsed.header.bcc.is_empty());
    assert_eq!(parsed.header.to, msg.to);
}

#[test]
fn user_agent_and_message_id() {
    let msg = OutgoingMessage {
        message_id: String::new(),
        ..draft(OutgoingBody::Text("x".to_owned()))
    };
    let raw = build(&msg).expect("build");
    assert_eq!(header_values(&raw, "X-Mailer"), ["Tern"]);
    assert!(!String::from_utf8_lossy(&raw).contains("Mailspring"));
    let parsed = parse_message(&raw).expect("parse");
    let id = parsed.header.message_id.expect("generated Message-ID");
    let (left, host) = id.split_once('@').expect("uuid@host");
    assert_eq!(left.len(), 36, "{id}");
    assert!(!host.is_empty());
}

#[test]
fn extra_headers_are_written() {
    let msg = OutgoingMessage {
        extra_headers: vec![("X-Tern-Thread".to_owned(), "abc".to_owned())],
        ..draft(OutgoingBody::Text("x".to_owned()))
    };
    let raw = build(&msg).expect("build");
    assert_eq!(header_values(&raw, "X-Tern-Thread"), ["abc"]);
    let parsed = parse_message(&raw).expect("parse");
    assert_eq!(parsed.header.extra_value("X-Tern-Thread"), Some("abc"));
}

#[test]
fn lines_stay_short_and_crlf_terminated() {
    let long_html = format!("<p>{}</p>", "word ".repeat(2000));
    let mut msg = draft(OutgoingBody::Html(long_html.clone()));
    msg.attachments = vec![OutgoingAttachment {
        filename: "blob.bin".to_owned(),
        mime_type: String::new(),
        content_id: None,
        data: vec![0xAB; 10_000],
    }];
    let raw = build(&msg).expect("build");
    let text = String::from_utf8(raw.clone()).expect("7-bit/utf-8 output");
    for line in text.split("\r\n") {
        assert!(line.len() <= 998, "line of {} bytes", line.len());
        assert!(!line.contains('\n'), "bare LF");
    }
    let parsed = parse_message(&raw).expect("parse");
    assert_eq!(parsed.body, long_html);
    assert_eq!(parsed.attachments[0].mime_type, "application/octet-stream");
    assert_eq!(parsed.attachments[0].data, vec![0xAB; 10_000]);
}
