// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The model view structs of `tern_ipc::models` against what tern-store writes: mail synced
//! through `MailProcessor` (headers, then a MIME body with an attachment, an inline image and
//! an invitation), Gmail labels, plugin metadata, contacts, groups, books, calendars and
//! events. Every stored row, every row the read paths return and every model a delta carries
//! must deserialise into its struct and fit its JSON Schema with no key the schema doesn't
//! list, so a key tern-store starts writing has to be added to the view struct (and so to the
//! generated TS types) first.

use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tern_core::{Account, Address, ManualClock, MessageFlags, MessageHeader};
use tern_imap::ImapMessage;
use tern_ipc::schema::violations;
use tern_store::model_query::{ModelQuery, QueryOutput};
use tern_store::read::ReadStore;
use tern_store::{
    Calendar, Contact, ContactBook, ContactGroup, DeltaStream, Event, Folder, IcsEventFields,
    Label, Message, Model, Query, Store, Thread,
};
use tern_sync::mail_processor::MailProcessor;

const ACCOUNT: &str = "acc1";
const T0: i64 = 1_700_000_000;

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn account(provider: &str) -> Account {
    Account::from_json(json!({
        "id": ACCOUNT,
        "provider": provider,
        "emailAddress": "me@example.com",
        "settings": {},
    }))
    .unwrap()
}

fn remote(header_id: &str, uid: u32, flags: MessageFlags, labels: &[&str]) -> ImapMessage {
    ImapMessage {
        uid,
        flags,
        internal_date: Some(1_600_000_000 + uid as i64),
        gmail_labels: labels.iter().map(|l| (*l).to_owned()).collect(),
        gmail_thread_id: Some(77),
        gmail_message_id: Some(1000 + uid as u64),
        header: Some(MessageHeader {
            date: Some(1_600_000_000 + uid as i64),
            subject: Some("Lunch".into()),
            message_id: Some(header_id.to_owned()),
            in_reply_to: if uid > 1 { vec!["m1@x".into()] } else { vec![] },
            references: if uid > 1 { vec!["m1@x".into()] } else { vec![] },
            from: vec![Address::new(Some("Ann"), "ann@example.com")],
            to: vec![Address::new(None, "me@example.com")],
            cc: vec![Address::new(Some("Bob"), "bob@example.com")],
            reply_to: vec![Address::new(None, "list@example.com")],
            extra: vec![(
                "List-Unsubscribe".into(),
                "<https://example.com/u>, <mailto:u@example.com>".into(),
            )],
            ..Default::default()
        }),
        ..Default::default()
    }
}

const RAW: &str = "From: Ann <ann@example.com>\r\n\
To: me@example.com\r\n\
Subject: Lunch\r\n\
Message-ID: <m1@x>\r\n\
List-Unsubscribe: <https://example.com/u>\r\n\
List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
\r\n\
--outer\r\n\
Content-Type: multipart/related; boundary=\"rel\"\r\n\
\r\n\
--rel\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Lunch at noon? <img src=\"cid:logo@x\"></p>\r\n\
--rel\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@x>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==\r\n\
--rel--\r\n\
--outer\r\n\
Content-Type: application/pdf; name=\"menu.pdf\"\r\n\
Content-Disposition: attachment; filename=\"menu.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQKJcOkw7zDtsOfCjIgMCBvYmoKPDwvTGVuZ3RoIDMgMCBSPj4Kc3RyZWFtCg==\r\n\
--outer\r\n\
Content-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\
\r\n\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
METHOD:REQUEST\r\n\
BEGIN:VEVENT\r\n\
UID:lunch-1\r\n\
DTSTART:20231115T120000Z\r\n\
DTEND:20231115T130000Z\r\n\
SUMMARY:Lunch\r\n\
ORGANIZER:mailto:ann@example.com\r\n\
ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.com\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n\
--outer--\r\n";

/// Fills a database the way sync does and returns it with every delta line it wrote.
fn synced(provider: &str) -> (tempfile::TempDir, Store, Sink) {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let mut out = Vec::new();
    tern_store::migrate::migrate(dir.path(), &clock, &mut out).unwrap();
    let sink = Sink::default();
    let stream = DeltaStream::new(Box::new(sink.clone()));
    let store = Store::open(dir.path(), stream.clone()).unwrap();
    store.set_clock(Arc::new(ManualClock::new(T0)));
    let acct = account(provider);
    let gmail = provider == "gmail";

    let folder = |path: &str, role: &str| {
        let mut f = Folder::new(&tern_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
        f.set_path(path);
        f.set_role(role);
        f.local_status_mut().insert("uidvalidity".into(), json!(1));
        store.save(&mut f).unwrap();
        f
    };
    let inbox = if gmail {
        folder("[Gmail]/All Mail", "all")
    } else {
        folder("INBOX", "inbox")
    };
    folder("Sent", "sent");
    if gmail {
        for (path, role) in [("\\Inbox", "inbox"), ("Work", "")] {
            let mut l = Label::new(&tern_core::ids::folder_id(ACCOUNT, path), ACCOUNT, 0);
            l.set_path(path);
            l.set_role(role);
            store.save(&mut l).unwrap();
        }
    }

    let mut processor = MailProcessor::new(&acct, &store);
    processor.is_gmail = gmail;
    let labels: &[&str] = if gmail { &["\\Inbox", "Work"] } else { &[] };
    let first = processor
        .insert_fallback_to_update_message(
            &remote("m1@x", 1, MessageFlags::empty(), labels),
            &inbox,
            T0,
        )
        .unwrap();
    processor
        .insert_fallback_to_update_message(
            &remote(
                "m2@x",
                2,
                MessageFlags::SEEN | MessageFlags::FLAGGED,
                labels,
            ),
            &inbox,
            T0,
        )
        .unwrap();

    let parsed = tern_mime::parse_message(RAW.as_bytes()).unwrap();
    let mut m = store
        .find::<Message>(&Query::new().equal("id", first.id()))
        .unwrap()
        .unwrap();
    processor.retrieved_message_body(&mut m, &parsed).unwrap();

    let mut m = store
        .find::<Message>(&Query::new().equal("id", first.id()))
        .unwrap()
        .unwrap();
    m.upsert_metadata("send-reminders", &json!({"expiration": T0 + 60}), None);
    store.save(&mut m).unwrap();
    let mut t = store
        .find::<Thread>(&Query::new().equal("id", m.thread_id()))
        .unwrap()
        .unwrap();
    t.upsert_metadata("notes", &json!({"text": "hi"}), None);
    store.save(&mut t).unwrap();

    let mut book = ContactBook::create("b1", ACCOUNT);
    book.set_url("https://dav.example.com/book/");
    book.set_source("carddav");
    store.save(&mut book).unwrap();
    let mut group = ContactGroup::create("g1", ACCOUNT);
    group.set_name("Friends");
    group.set_book_id("b1");
    store.save(&mut group).unwrap();
    let mut c = Contact::create("c1", ACCOUNT, "ann@example.com", 2, "carddav");
    c.set_name("Ann");
    c.set_book_id("b1");
    c.set_etag("e1");
    store.save(&mut c).unwrap();
    let mut mail_contact = Contact::create("c2", ACCOUNT, "bob@example.com", 1, "mail");
    store.save(&mut mail_contact).unwrap();
    group.sync_members(&store, &["c1".into()]).unwrap();

    let mut cal = Calendar::new("cal1", ACCOUNT);
    cal.set_name("Home");
    cal.set_path("/cal/home/");
    cal.set_color("#AABBCC");
    store.save(&mut cal).unwrap();
    let fields = IcsEventFields {
        uid: "uid-1".into(),
        recurrence_start: T0,
        recurrence_end: T0 + 3600,
        summary: "Lunch".into(),
        attendees: vec!["ann@example.com".into()],
        ..Default::default()
    };
    let mut e = Event::create("etag1", ACCOUNT, "cal1", "BEGIN:VCALENDAR", &fields);
    e.set_href("/cal/home/uid-1.ics");
    store.save(&mut e).unwrap();
    stream.begin_connection_error(ACCOUNT);
    stream.end_connection_error(ACCOUNT);
    stream.flush();

    (dir, store, sink)
}

/// Deserialises `json` as `T`, checks it against `def`'s schema, and that serialising it
/// back gives the same JSON (a null and an absent key are the same).
fn check_as<T: DeserializeOwned + Serialize>(def: &str, json: &Value, failures: &mut Vec<String>) {
    let id = json["id"].as_str().unwrap_or("?");
    match serde_json::from_value::<T>(json.clone()) {
        Ok(view) => {
            let back = serde_json::to_value(&view).unwrap();
            if without_nulls(&back) != without_nulls(json) {
                failures.push(format!(
                    "{def} {id}: changed in a round trip\n  {json}\n  {back}"
                ));
            }
        }
        Err(e) => failures.push(format!("{def} {id}: {e}\n  {json}")),
    }
    for v in violations(json, def) {
        failures.push(format!("{def} {id}: {v}"));
    }
}

fn without_nulls(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), without_nulls(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(without_nulls).collect()),
        other => other.clone(),
    }
}

fn check_class(class: &str, json: &Value, failures: &mut Vec<String>) {
    use tern_ipc::models::*;
    match class {
        "Thread" => check_as::<ThreadJson>("ThreadJson", json, failures),
        "Message" => check_as::<MessageJson>("MessageJson", json, failures),
        "Folder" => check_as::<FolderJson>("FolderJson", json, failures),
        "Label" => check_as::<LabelJson>("LabelJson", json, failures),
        "Contact" => check_as::<ContactJson>("ContactJson", json, failures),
        "ContactGroup" => check_as::<ContactGroupJson>("ContactGroupJson", json, failures),
        "ContactBook" => check_as::<ContactBookJson>("ContactBookJson", json, failures),
        "Calendar" => check_as::<CalendarJson>("CalendarJson", json, failures),
        "Event" => check_as::<EventJson>("EventJson", json, failures),
        "File" => check_as::<FileJson>("FileJson", json, failures),
        "ThreadCounts" => check_as::<ThreadCountsJson>("ThreadCountsJson", json, failures),
        "ProcessState" => check_as::<ProcessStateJson>("ProcessStateJson", json, failures),
        other => failures.push(format!("no view struct for {other}")),
    }
}

const TABLES: &[&str] = &[
    "Thread",
    "Message",
    "Folder",
    "Label",
    "Contact",
    "ContactGroup",
    "ContactBook",
    "Calendar",
    "Event",
    "File",
];

fn run(provider: &str) {
    let (dir, store, sink) = synced(provider);
    let mut failures = Vec::new();
    let mut seen = std::collections::BTreeMap::<String, usize>::new();

    // Stored rows.
    for table in TABLES {
        let mut stmt = store
            .conn()
            .prepare(&format!("SELECT data FROM {table}"))
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for row in rows {
            let json: Value = serde_json::from_str(&row).unwrap();
            // Stored rows carry no `__cls` but Task's; `toJSON` adds it.
            let mut json = json;
            json.as_object_mut()
                .unwrap()
                .entry("__cls")
                .or_insert_with(|| json!(table));
            check_class(table, &json, &mut failures);
            *seen.entry(format!("stored {table}")).or_default() += 1;
        }
    }

    // What the read paths send: `query` rows (with the derived fields, `body` and `summary`
    // when asked), and the thread and message reads.
    let reads = ReadStore::open(dir.path()).unwrap();
    for table in TABLES {
        let include = match *table {
            "Message" => json!(["body"]),
            "Thread" => json!(["summary"]),
            _ => json!([]),
        };
        let q: ModelQuery =
            serde_json::from_value(json!({"class": table, "include": include})).unwrap();
        let QueryOutput::Rows { rows, .. } = reads.model_query(&q).unwrap() else {
            panic!("rows");
        };
        for row in rows {
            if *table == "Thread" {
                assert!(row["summary"]["messageCount"].as_i64() > Some(0), "{row}");
            }
            check_class(table, &row, &mut failures);
            *seen.entry(format!("query {table}")).or_default() += 1;
        }
    }
    for t in reads.threads(ACCOUNT, None, 50, None).unwrap().threads {
        check_class("Thread", &t, &mut failures);
        for m in reads
            .thread_messages(ACCOUNT, t["id"].as_str().unwrap())
            .unwrap()
        {
            check_class("Message", &m, &mut failures);
            *seen.entry("read Message".into()).or_default() += 1;
        }
    }

    // Deltas, as stdout writes them and with the fields the socket adds.
    let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    for line in text.lines() {
        let delta: Value = serde_json::from_str(line).unwrap();
        let class = delta["modelClass"].as_str().unwrap();
        for m in delta["modelJSONs"].as_array().unwrap() {
            check_class(class, m, &mut failures);
            check_class(
                class,
                &tern_store::derived::annotated(class, m.clone()),
                &mut failures,
            );
            *seen.entry(format!("delta {class}")).or_default() += 1;
        }
    }

    assert!(
        failures.is_empty(),
        "{provider}: {} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // The fixture reaches every class, so a struct can't pass by not being tried.
    for table in TABLES {
        if *table == "Label" && provider != "gmail" {
            continue;
        }
        for source in ["stored", "query", "delta"] {
            assert!(
                seen.contains_key(&format!("{source} {table}")),
                "{provider}: no {source} {table} rows; seen {seen:?}"
            );
        }
    }
    assert!(seen.contains_key("delta ThreadCounts"), "{seen:?}");
}

#[test]
fn imap_rows_fit_their_view_structs() {
    run("imap");
}

#[test]
fn gmail_rows_fit_their_view_structs() {
    run("gmail");
}
