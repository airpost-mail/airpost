// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The rendering and file rules, asserted directly on the golden
//! corpus inputs so that re-blessing the expectations cannot quietly accept a regression.

use std::path::Path;

use tern_mime::{parse_message, ParsedMessage, ParsedPart};

fn golden(name: &str) -> ParsedMessage {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/golden")
        .join(format!("{name}.eml"));
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    parse_message(&raw).expect("parse_message")
}

fn file<'a>(m: &'a ParsedMessage, filename: &str) -> &'a ParsedPart {
    m.attachments
        .iter()
        .find(|f| f.filename == filename)
        .unwrap_or_else(|| panic!("no file {filename:?} in {:?}", names(m)))
}

fn names(m: &ParsedMessage) -> Vec<&str> {
    m.attachments.iter().map(|f| f.filename.as_str()).collect()
}

fn attachment_count(m: &ParsedMessage) -> usize {
    m.attachments
        .iter()
        .filter(|f| f.counts_as_attachment())
        .count()
}

#[test]
fn alternative_prefers_html_then_plain_then_first() {
    let m = golden("alternative-prefers-html");
    assert!(!m.plaintext);
    assert!(m.body.contains("<i>HTML</i>"));
    assert!(!m.body.contains("plain text"));

    let m = golden("alternative-html-first");
    assert_eq!(m.body.trim(), "<p>HTML first</p>");

    let m = golden("alternative-last-plain");
    assert!(m.plaintext);
    assert_eq!(m.body.trim(), "Second plain wins");
    assert!(
        m.attachments.is_empty(),
        "unchosen alternatives are not files"
    );

    let m = golden("alternative-no-text");
    assert_eq!(m.body, "");
    assert_eq!(names(&m), ["Unnamed Attachment"]);
    assert_eq!(m.attachments[0].mime_type, "application/x-first");
}

#[test]
fn alternative_appends_the_calendar_part() {
    let m = golden("exchange-calendar-invite");
    assert!(!m.plaintext);
    assert!(m.body.contains("Budget review"));
    let cal = &m.attachments[0];
    assert_eq!(cal.part_id, "1.3");
    assert_eq!(cal.mime_type, "text/calendar");
    assert_eq!(cal.filename, "Event.ics");
    assert!(String::from_utf8_lossy(&cal.data).contains("METHOD:REQUEST"));
    assert_eq!(m.attachments[1].filename, "invite.ics");
}

#[test]
fn mixed_concatenates_and_html_sibling_makes_plain_html() {
    let m = golden("mixed-plain-concatenated");
    assert!(m.plaintext);
    assert_eq!(m.body, "Part one.\r\nPart two.\r\n");

    let m = golden("mixed-html-renders-plain-as-html");
    assert!(!m.plaintext);
    assert!(m.body.starts_with("<p>Intro in HTML</p>"));
    assert!(m.body.contains("Reply line<br/>"));
    assert!(m.body.contains(
        "<blockquote type=\"cite\">quoted line<br/><blockquote type=\"cite\">deeper<br/></blockquote></blockquote>"
    ));
    assert!(m.body.contains("back &amp; &lt;done&gt;<br/>"));
}

#[test]
fn plain_bodies_are_stored_unescaped() {
    let m = golden("html-plain-escaping");
    assert!(m.plaintext);
    assert!(m.body.starts_with("<script>alert(1)</script> & \"quotes\""));
    assert_eq!(m.text, m.body, "plaintext snippets come from the raw body");
}

#[test]
fn html_is_stored_as_is() {
    let raw = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/golden/html-only.eml"),
    )
    .expect("input");
    let m = parse_message(&raw).expect("parse");
    assert!(m
        .body
        .starts_with("<html><head><style>p{color:red}</style>"));
    assert!(m.body.contains("Café"));
    assert_eq!(
        m.snippet(),
        "Café menu for today: Soup Bread & butter See you"
    );
}

#[test]
fn related_root_renders_and_the_rest_are_inline_files() {
    let m = golden("related-cid-inline");
    assert_eq!(
        m.body.trim(),
        "<p>Logo: <img src=\"cid:logo@example.org\"></p>"
    );
    let logo = file(&m, "logo.png");
    assert_eq!(logo.part_id, "2");
    assert_eq!(logo.unique_id, "1");
    assert!(logo.inline);
    assert_eq!(logo.content_id.as_deref(), Some("logo@example.org"));
    assert!(!logo.counts_as_attachment());
    assert_eq!(attachment_count(&m), 0);

    let m = golden("related-cid-large-counts");
    let chart = file(&m, "Unnamed Image.png");
    assert!(
        chart.inline,
        "a referenced Content-ID is inline without a disposition"
    );
    assert!(chart.size() > tern_mime::INLINE_IMAGE_COUNT_THRESHOLD);
    assert!(chart.counts_as_attachment());

    let m = golden("related-plain-root");
    assert!(m.plaintext);
    let png = file(&m, "Unnamed Image.png");
    assert!(!png.inline);
    assert_eq!(
        png.content_id, None,
        "an unreferenced Content-ID is dropped"
    );
}

#[test]
fn content_id_rules() {
    let m = golden("cid-referenced-becomes-inline");
    let sig = file(&m, "sig.gif");
    assert!(sig.inline);
    assert_eq!(sig.content_id.as_deref(), Some("sig@example.org"));
    let unref = file(&m, "unref.gif");
    assert!(!unref.inline);
    assert_eq!(unref.content_id, None);
    assert_eq!(attachment_count(&m), 1);

    let m = golden("cid-filename-fallback");
    let img = file(&m, "image001.png");
    assert_eq!(img.content_id.as_deref(), Some("image001.png"));
    assert!(!img.counts_as_attachment());
}

#[test]
fn default_names() {
    let m = golden("attachments-missing-names");
    assert_eq!(
        names(&m),
        [
            "Unnamed Attachment",
            "Unnamed Image.jpg",
            "Event.ics",
            "Unnamed Attachment",
            "Unnamed Attachment",
        ]
    );
    let m = golden("dsn-delivery-report");
    assert_eq!(names(&m), ["Delivery Status.txt"]);
    assert!(m.body.contains("User unknown"));
    assert!(
        m.body.contains("Original body"),
        "message/rfc822 renders its main part"
    );
}

#[test]
fn named_text_parts_after_the_body_are_files() {
    let m = golden("named-text-parts");
    assert_eq!(names(&m), ["notes.txt", "explicit.txt"]);
    assert!(m.body.contains("Body text."));
    assert!(m.body.contains("Named but inline."));
    assert!(m.body.contains("Unnamed trailer."));
    assert!(!m.body.contains("These are notes."));
}

#[test]
fn encoded_filenames() {
    let m = golden("encoded-filenames");
    assert_eq!(
        names(&m),
        [
            "€ rates 2026.pdf",
            "Über die Grenze.pdf",
            "Отчёт.docx",
            "résumé.zip",
            "a/b\\c:d*e?.txt",
        ]
    );
    assert_eq!(
        tern_mime::safe_filename("a/b\\c:d*e?.txt"),
        "a-b-c-d-e-.txt"
    );
}

#[test]
fn part_numbering() {
    let m = golden("exchange-outlook-related");
    let xlsx = file(&m, "Budget Q1.xlsx");
    assert_eq!((xlsx.part_id.as_str(), xlsx.unique_id.as_str()), ("2", "0"));
    let img = file(&m, "image001.png");
    assert_eq!((img.part_id.as_str(), img.unique_id.as_str()), ("1.2", "1"));
    assert_eq!(
        img.content_id.as_deref(),
        Some("image001.png@01D3A1B2.C3D4E5F0")
    );

    let m = golden("gmail-mixed-alternative-related");
    let pic = file(&m, "pic.png");
    assert_eq!(
        (pic.part_id.as_str(), pic.unique_id.as_str()),
        ("1.2.2", "3")
    );
    assert_eq!(file(&m, "terms.pdf").part_id, "2");

    let m = golden("smime-opaque");
    assert_eq!(
        m.attachments[0].part_id, "1",
        "a top-level single part is 1"
    );
}

#[test]
fn nested_messages() {
    let m = golden("forwarded-rfc822-inline");
    assert!(m.body.contains("Forwarding this."));
    assert!(m.body.contains("<p>Inner <b>HTML</b></p>"));
    assert!(m.attachments.is_empty());

    let m = golden("forwarded-rfc822-base64");
    let eml = file(&m, "Original message.eml");
    assert_eq!(eml.mime_type, "message/rfc822");
    assert!(String::from_utf8_lossy(&eml.data).contains("Subject: Original message"));

    let m = golden("digest");
    assert!(m.body.contains("First digest entry"));
    assert!(m.body.contains("<p>Second entry</p>"));
}

#[test]
fn signed_is_mixed() {
    let m = golden("signed-pgp");
    assert!(m.body.contains("Signed text."));
    assert_eq!(names(&m), ["signature.asc"]);

    let m = golden("signed-smime");
    assert!(m.body.contains("<u>html</u>"));
    assert_eq!(names(&m), ["smime.p7s"]);
}

#[test]
fn snippet_rules() {
    let m = golden("snippet-links-quotes-scripts");
    let s = m.snippet();
    assert!(s.contains("document (https://example.org/doc)"), "{s}");
    // flattenHTML() keeps cited blockquotes (MCString.cpp:2162)
    assert!(
        s.contains("> On Monday Bob wrote: > quoted text that the snippet keeps"),
        "{s}"
    );
    assert!(!s.contains("hidden"), "{s}");
    assert!(!s.contains("display:none"), "{s}");
    assert!(s.contains("A plain blockquote stays"), "{s}");
    assert!(s.contains("& entities <ok>"), "{s}");
    assert!(!s.contains("  "), "whitespace is collapsed: {s:?}");

    let m = golden("list-unsubscribe");
    assert!(m.snippet().contains("an article (https://example.org/a)"));
}

#[test]
fn snippet_and_search_text_lengths() {
    let m = golden("snippet-surrogate-boundary");
    assert_eq!(
        m.snippet().encode_utf16().count(),
        399,
        "never splits a surrogate pair"
    );
    let m = golden("snippet-long-utf16");
    assert!(m.snippet().encode_utf16().count() <= tern_mime::SNIPPET_LENGTH);
    let m = golden("search-text-long");
    assert_eq!(
        m.snippet().encode_utf16().count(),
        tern_mime::SNIPPET_LENGTH
    );
    assert_eq!(
        m.search_text().encode_utf16().count(),
        tern_mime::SEARCH_TEXT_LENGTH
    );
}

#[test]
fn list_unsubscribe_and_importance() {
    let m = golden("list-unsubscribe");
    assert_eq!(
        m.list_unsubscribe.as_deref(),
        Some("<mailto:unsub@lists.example.org?subject=unsubscribe>, <https://lists.example.org/u/abc123>")
    );
    assert_eq!(
        m.list_unsubscribe_post.as_deref(),
        Some("List-Unsubscribe=One-Click")
    );
    assert_eq!(m.importance, None);

    let cases = [
        ("importance-high", Some("high")),
        ("importance-x-priority", Some("low")),
        ("importance-msmail-fallback", Some("high")),
        ("importance-precedence", Some("normal")),
        ("plain-simple", None),
    ];
    for (name, want) in cases {
        assert_eq!(golden(name).importance.as_deref(), want, "{name}");
    }
}

#[test]
fn charsets() {
    let cases = [
        (
            "charset-latin1-labelled-utf8",
            "une forêt où vivait un garçon très naïf",
        ),
        (
            "charset-utf8-labelled-latin1",
            "Grüße aus München — schönen Tag!",
        ),
        ("charset-unknown-label", "Привет, как дела?"),
        ("charset-utf7", "Hi Mom -☺-! 1 + 1 = 2"),
        ("charset-utf16-base64", "日本語のテキスト"),
        ("charset-html-meta", "Привет, мир!"),
        ("charset-gbk-raw-headers", "会议通知：明天上午十点"),
        ("headers-iso-2022-jp", "こんにちは、世界"),
        (
            "qp-html-soft-breaks",
            "with soft breaks, and some “smart quotes” and an em dash — plus a euro €.",
        ),
        ("uuencoded-body", "Hello world"),
    ];
    for (name, want) in cases {
        let m = golden(name);
        assert!(m.body.contains(want), "{name}: {:?}", m.body);
        assert!(!m.body.starts_with('\u{feff}'), "{name}: BOM kept");
    }
}

#[test]
fn headers() {
    let m = golden("headers-rfc2047");
    assert_eq!(
        m.header.subject.as_deref(),
        Some("🎉 Party für alle — Samstag")
    );
    assert_eq!(m.header.from[0].name.as_deref(), Some("André Müller"));

    let m = golden("charset-gbk-raw-headers");
    assert_eq!(m.header.subject.as_deref(), Some("会议通知 － 明天上午"));
    assert_eq!(m.header.from[0].name.as_deref(), Some("张三"));

    let m = golden("charset-raw-utf8-headers");
    assert_eq!(
        m.header.subject.as_deref(),
        Some("Grüße – Rückmeldung zum Angebot")
    );
    assert_eq!(m.header.from[0].name.as_deref(), Some("Jürgen"));

    let m = golden("headers-iso-2022-jp");
    assert_eq!(m.header.subject.as_deref(), Some("こんにちは"));

    let m = golden("mbox-from-line");
    assert_eq!(m.header.subject.as_deref(), Some("With an mbox From line"));
}

#[test]
fn malformed_structure() {
    let m = golden("multipart-without-boundary");
    assert_eq!(m.body, "");
    assert_eq!(m.attachments[0].mime_type, "multipart/mixed");

    let m = golden("multipart-unterminated");
    assert!(m.body.contains("The HTML part runs to the end"));

    let m = golden("no-content-type");
    assert!(m.plaintext);
    assert!(m.body.contains("Second line & <tags> stay as text."));

    let m = golden("empty-body");
    assert!(m.plaintext);
    assert_eq!(m.body, "");
    assert_eq!(m.snippet(), "");
}

#[test]
fn exchange_quirks() {
    let m = golden("exchange-winmail-dat");
    let tnef = file(&m, "winmail.dat");
    assert_eq!(tnef.mime_type, "application/ms-tnef");
    assert!(tnef.counts_as_attachment());

    let m = golden("exchange-outlook-related");
    assert!(!m.plaintext);
    assert!(m
        .body
        .contains("src=\"cid:image001.png@01D3A1B2.C3D4E5F0\""));
    assert_eq!(m.snippet(), "Hi team,");
    assert_eq!(attachment_count(&m), 1);
    assert_eq!(m.header.from[0].name.as_deref(), Some("Doe, Jane"));

    let m = golden("apple-mail-inline-image");
    assert_eq!(
        m.body,
        "<html><body><div>Look at this:</div></body></html><html><body><div>Nice, right?</div></body></html>"
    );
    let photo = file(&m, "photo.png");
    assert!(photo.inline);
    assert_eq!(photo.content_id.as_deref(), Some("photo-1@apple.example"));
}

#[test]
fn deeply_quoted_plain_text_does_not_overflow_the_stack() {
    // A plain part rendered as HTML next to an HTML sibling, with one line of 20K '>'.
    // mailcore recursed per quote level; run on a small stack to catch a regression.
    let quotes = ">".repeat(20_000);
    let raw = format!(
        "From: a@example.org\r\nSubject: deep\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/html\r\n\r\n<p>hi</p>\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\n{quotes}x\r\n--b--\r\n"
    );
    let m = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || parse_message(raw.as_bytes()).expect("parse_message"))
        .unwrap()
        .join()
        .expect("no stack overflow");
    assert_eq!(m.body.matches("</blockquote>").count(), 20_000);
    assert!(
        m.snippet().starts_with("hi > > >"),
        "{}",
        &m.snippet()[..40]
    );
}
