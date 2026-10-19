// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `events.*` and `contacts.*` on the socket (docs/desktop-client-plan.md 3.8) on the debug
//! binary, against a seeded database and an account whose servers refuse connections: the
//! answers, and the rows the queued tasks' local phases write. The ICS and vCard edits
//! themselves are `ms-pim`'s tests.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use ms_ipc::{code, feature, method, Client, ClientError};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

const ACCOUNT_ID: &str = "c0ffee-harness";
const ACCOUNT: &str = r#"{"id":"c0ffee-harness","emailAddress":"me@example.com","name":"Me","provider":"imap","settings":{"imap_host":"127.0.0.1","imap_port":1,"imap_username":"me","imap_password":"x","imap_security":"none","smtp_host":"127.0.0.1","smtp_port":1,"smtp_username":"me","smtp_password":"x","smtp_security":"none","imap_allow_insecure_ssl":false,"smtp_allow_insecure_ssl":false}}"#;

// 2026-03-02 09:00 America/Chicago, weekly.
const MAR2: i64 = 1_772_463_600;
// The second and fourth, after the change to CDT.
const MAR9: i64 = 1_773_064_800;
const MAR23: i64 = 1_774_274_400;
const WEEK: i64 = 7 * 86400;

fn series_ics() -> String {
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:standup@test\r\nDTSTAMP:20260101T000000Z\r\nDTSTART;TZID=America/Chicago:20260302T090000\r\nDTEND;TZID=America/Chicago:20260302T093000\r\nRRULE:FREQ=WEEKLY;COUNT=10\r\nSUMMARY:Standup\r\nORGANIZER;CN=Me:mailto:me@example.com\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:guest@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into()
}

fn invite_ics() -> String {
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VEVENT\r\nUID:review@test\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20260304T150000Z\r\nDTEND:20260304T160000Z\r\nSUMMARY:Review\r\nORGANIZER;CN=Boss:mailto:boss@example.org\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:boss@example.org\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into()
}

fn mailsync(config: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mailsync"));
    c.env("CONFIG_DIR_PATH", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn insert_event(c: &Connection, id: &str, cal: &str, uid: &str, ics: &str, rs: i64, re: i64) {
    let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "Event", "v": 1, "cid": cal,
        "ics": ics, "icsuid": uid, "rid": "", "rs": rs, "re": re, "status": "CONFIRMED",
        "etag": "e1", "href": format!("/cal/{id}.ics")});
    c.execute(
        "INSERT INTO Event (id, data, accountId, etag, calendarId, recurrenceStart, recurrenceEnd, icsuid, recurrenceId) \
         VALUES (?, ?, ?, 'e1', ?, ?, ?, ?, '')",
        params![id, data.to_string(), ACCOUNT_ID, cal, rs, re, uid],
    )
    .unwrap();
}

fn seed(config: &Path) {
    let c = Connection::open(config.join("edgehill.db")).unwrap();
    for (id, read_only, owner) in [("cal-work", false, "mine"), ("cal-holidays", true, "other")] {
        let data = json!({"id": id, "aid": ACCOUNT_ID, "__cls": "Calendar", "v": 1,
            "path": format!("/cal/{id}/"), "name": id, "read_only": read_only, "owner": owner});
        c.execute(
            "INSERT INTO Calendar (id, data, accountId) VALUES (?, ?, ?)",
            params![id, data.to_string(), ACCOUNT_ID],
        )
        .unwrap();
    }
    insert_event(
        &c,
        "ev-standup",
        "cal-work",
        "standup@test",
        &series_ics(),
        MAR2,
        MAR2 + 10 * WEEK,
    );
    insert_event(
        &c,
        "ev-review",
        "cal-work",
        "review@test",
        &invite_ics(),
        1_772_636_400,
        1_772_640_000,
    );
    let holiday = series_ics()
        .replace("standup@test", "holiday@test")
        .replace(
            "PARTSTAT=ACCEPTED:mailto:me@example.com",
            "PARTSTAT=ACCEPTED:mailto:other@example.org",
        );
    insert_event(
        &c,
        "ev-holiday",
        "cal-holidays",
        "holiday@test",
        &holiday,
        MAR2,
        MAR2 + 10 * WEEK,
    );

    let vcf = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:ann\r\nFN:Ann Archer\r\nN:Archer;Ann;;;\r\nEMAIL;TYPE=WORK:ann@example.com\r\nTITLE:Engineer\r\nX-CUSTOM:kept\r\nEND:VCARD\r\n";
    let data = json!({"id": "ann", "aid": ACCOUNT_ID, "__cls": "Contact", "v": 1, "name": "Ann Archer",
        "email": "ann@example.com", "s": "carddav", "info": {"vcf": vcf, "href": "/book/ann.vcf"}, "refs": 0});
    c.execute(
        "INSERT INTO Contact (id, data, accountId, email, version, source, etag) VALUES ('ann', ?, ?, 'ann@example.com', 1, 'carddav', 'e1')",
        params![data.to_string(), ACCOUNT_ID],
    )
    .unwrap();
    c.execute(
        "INSERT INTO ContactSearch (content_id, content) VALUES ('ann', 'ann example.com Ann Archer')",
        [],
    )
    .unwrap();
}

struct Engine {
    dir: tempfile::TempDir,
    socket: PathBuf,
    child: Child,
}

impl Engine {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let status = mailsync(dir.path())
            .args(["--mode", "migrate"])
            .output()
            .unwrap()
            .status;
        assert!(status.success());
        seed(dir.path());
        let socket = dir.path().join("run").join("e.sock");
        let mut child = mailsync(dir.path())
            .args(["--mode", "sync", "--identity", "null", "--account", ACCOUNT])
            .arg("--socket")
            .arg(&socket)
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert!(line.contains("listening"), "{line}");
        std::thread::spawn(move || std::io::copy(&mut stdout, &mut std::io::sink()));
        Engine { dir, socket, child }
    }

    fn hello(&self) -> Client {
        let mut c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        c.hello("pim-test/1").unwrap();
        c
    }

    fn relayed(&self) -> Client {
        let mut c = Client::connect(&self.socket).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        c.call(
            method::HELLO,
            json!({ "protocol": 1, "client": "tern-gateway/test", "relayed": true,
                    "scopes": [], "device": { "id": "d1", "name": "Test" } }),
        )
        .unwrap();
        c
    }

    fn db(&self) -> Connection {
        Connection::open(self.dir.path().join("edgehill.db")).unwrap()
    }

    fn event_ics(&self, id: &str) -> Option<String> {
        self.db()
            .query_row("SELECT data FROM Event WHERE id = ?", [id], |r| {
                r.get::<_, Vec<u8>>(0)
                    .or_else(|_| r.get::<_, String>(0).map(String::into_bytes))
            })
            .optional()
            .unwrap()
            .map(|b| {
                serde_json::from_slice::<Value>(&b).unwrap()["ics"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
    }

    fn task_classes(&self) -> Vec<String> {
        let db = self.db();
        let mut stmt = db.prepare("SELECT data FROM Task ORDER BY rowid").unwrap();
        let rows = stmt
            .query_map([], |r| {
                r.get::<_, Vec<u8>>(0)
                    .or_else(|_| r.get::<_, String>(0).map(String::into_bytes))
            })
            .unwrap();
        rows.map(|b| {
            let v: Value = serde_json::from_slice(&b.unwrap()).unwrap();
            v["__cls"].as_str().unwrap_or_default().to_owned()
        })
        .collect()
    }

    fn stop(mut self) {
        assert!(self.child.try_wait().unwrap().is_none(), "exited early");
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert_eq!(stderr, "");
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn key(e: &ClientError) -> Option<String> {
    match e {
        ClientError::Rpc(o) => o
            .data
            .as_ref()
            .and_then(|d| d["key"].as_str())
            .map(str::to_owned),
        _ => None,
    }
}

#[test]
fn events_expand_and_edit() {
    let e = Engine::start();
    let mut c = e.hello();
    let status = c.call(method::ENGINE_STATUS, json!({})).unwrap();
    let features = status["features"].as_array().unwrap();
    assert!(
        features.contains(&json!(feature::CALENDAR_EDITING)),
        "{status}"
    );
    assert!(
        features.contains(&json!(feature::CONTACT_EDITING)),
        "{status}"
    );

    // Two weeks of the work calendar: two standups and the review.
    let r = c
        .call(
            method::EVENTS_EXPAND,
            json!({"calendarIds": ["cal-work"], "start": MAR2 - 3600, "end": MAR2 + 2 * WEEK - 7200,
                   "tz": "America/Chicago"}),
        )
        .unwrap();
    let occ = r["occurrences"].as_array().unwrap();
    let titles: Vec<&str> = occ.iter().map(|o| o["title"].as_str().unwrap()).collect();
    assert_eq!(titles.iter().filter(|t| **t == "Standup").count(), 2, "{r}");
    assert!(titles.contains(&"Review"), "{r}");
    let standup = occ.iter().find(|o| o["title"] == "Standup").unwrap();
    assert_eq!(standup["eventId"], "ev-standup");
    assert_eq!(standup["isRecurring"], true);
    assert_eq!(standup["isMine"], true);

    // The focused occurrence and the conflicts an invitation at the second standup has.
    let r = c
        .call(
            method::EVENTS_EXPAND,
            json!({"eventIds": ["ev-standup"], "start": MAR9, "end": MAR9 + 1800,
                   "focus": {"now": MAR2 + 3 * 86400}, "conflicts": {}, "tz": "America/Chicago"}),
        )
        .unwrap();
    assert_eq!(r["focused"]["start"], MAR9, "{r}");
    assert_eq!(r["conflicts"].as_array().unwrap().len(), 1, "{r}");

    // An emailed invitation: its next occurrence.
    let r = c
        .call(
            method::EVENTS_EXPAND,
            json!({"ics": invite_ics(), "start": 0, "end": 2_000_000_000, "focus": {"now": 0}}),
        )
        .unwrap();
    assert_eq!(r["occurrences"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["upcoming"]["start"], 1_772_636_400, "{r}");
    assert_eq!(r["first"]["end"], 1_772_640_000, "{r}");
    assert!(r["first"].get("recurrenceId").is_none(), "{r}");

    // Move the second standup an hour later: an exception in the stored resource.
    let r = c
        .call(
            method::EVENTS_UPDATE,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-standup", "scope": "this",
                   "occurrenceStart": MAR9,
                   "patch": {"start": MAR9 + 3600, "end": MAR9 + 5400},
                   "tz": "America/Chicago"}),
        )
        .unwrap();
    let writes = r["writes"].as_array().unwrap();
    assert_eq!(writes.len(), 1, "{r}");
    assert_eq!(writes[0]["kind"], "update");
    assert!(writes[0]["taskId"].is_string());
    let ics = e.event_ics("ev-standup").unwrap();
    assert!(ics.contains("RECURRENCE-ID:20260309T140000Z"), "{ics}");
    assert!(
        ics.contains("DTSTART;TZID=America/Chicago:20260309T100000"),
        "{ics}"
    );

    // queue:false only describes the writes.
    let before = e.task_classes().len();
    let r = c
        .call(
            method::EVENTS_UPDATE,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-standup", "scope": "all",
                   "patch": {"summary": "Daily"}, "queue": false}),
        )
        .unwrap();
    assert!(r["writes"][0]["ics"]
        .as_str()
        .unwrap()
        .contains("SUMMARY:Daily"));
    assert!(r["writes"][0].get("taskId").is_none());
    assert_eq!(e.task_classes().len(), before);
    assert!(!e.event_ics("ev-standup").unwrap().contains("Daily"));

    // Somebody else's calendar can't be written.
    let err = c
        .call(
            method::EVENTS_UPDATE,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-holiday", "patch": {"summary": "x"}}),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::INVALID_PARAMS));
    assert_eq!(key(&err).as_deref(), Some("read-only"));

    // A new event, with a guest, organised by the account.
    let r = c
        .call(
            method::EVENTS_CREATE,
            json!({"accountId": ACCOUNT_ID, "calendarId": "cal-work", "summary": "Lunch",
                   "start": MAR2 + 3 * 3600, "end": MAR2 + 4 * 3600, "timezone": "America/Chicago",
                   "attendees": [{"email": "friend@example.org", "name": "Friend"}]}),
        )
        .unwrap();
    let id = r["writes"][0]["eventId"].as_str().unwrap().to_owned();
    let uid = r["icsuid"].as_str().unwrap();
    assert!(uid.ends_with("@tern"), "{r}");
    let ics = e.event_ics(&id).unwrap();
    assert!(
        ics.contains("SUMMARY:Lunch") && ics.contains("ORGANIZER;CN=Me:mailto:me@example.com"),
        "{ics}"
    );

    // Delete the rest of the series from the fourth standup on.
    let r = c
        .call(
            method::EVENTS_UPDATE,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-standup", "scope": "following",
                   "occurrenceStart": MAR23, "patch": {"deleted": true}}),
        )
        .unwrap();
    assert_eq!(r["writes"][0]["kind"], "update", "{r}");
    let ics = e.event_ics("ev-standup").unwrap();
    assert!(ics.contains("UNTIL=20260323T135959Z"), "{ics}");

    // Bad params.
    let err = c
        .call(
            method::EVENTS_UPDATE,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-standup", "scope": "some", "patch": {}}),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::INVALID_PARAMS));
    let err = c
        .call(
            method::EVENTS_EXPAND,
            json!({"start": 0, "end": 1, "tz": "Not/AZone"}),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::INVALID_PARAMS));

    // Reads are open to relayed devices; writes, and the tasks they queue, are not.
    let mut r = e.relayed();
    assert!(r
        .call(method::EVENTS_EXPAND, json!({"start": 0, "end": 1}))
        .is_ok());
    let err = r
        .call(
            method::EVENTS_UPDATE,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-standup", "patch": {"summary": "x"}}),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some(code::FORBIDDEN_RELAYED));
    e.stop();
}

#[test]
fn rsvp_and_counter() {
    let e = Engine::start();
    let mut c = e.hello();
    let r = c
        .call(
            method::EVENTS_RSVP,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-review", "status": "ACCEPTED"}),
        )
        .unwrap();
    assert_eq!(r["email"], "me@example.com", "{r}");
    assert!(r["replyTaskId"].is_string(), "{r}");
    assert_eq!(r["write"]["kind"], "update");
    let ics = e.event_ics("ev-review").unwrap();
    assert!(
        ics.contains("PARTSTAT=ACCEPTED:mailto:me@example.com"),
        "{ics}"
    );
    let classes = e.task_classes();
    assert!(
        classes.contains(&"SyncbackEventTask".to_owned()),
        "{classes:?}"
    );
    assert!(classes.contains(&"EventRSVPTask".to_owned()), "{classes:?}");

    // From the emailed copy: our stored copy is found by its UID.
    let r = c
        .call(
            method::EVENTS_RSVP,
            json!({"accountId": ACCOUNT_ID, "ics": invite_ics(), "status": "DECLINED",
                   "messageId": "m1", "subject": "Can't make it"}),
        )
        .unwrap();
    assert_eq!(r["write"]["eventId"], "ev-review", "{r}");
    assert!(e
        .event_ics("ev-review")
        .unwrap()
        .contains("PARTSTAT=DECLINED"));

    let r = c
        .call(
            method::EVENTS_RSVP,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-review", "status": "TENTATIVE",
                   "counter": {"start": 1_772_640_000, "end": 1_772_643_600}, "comment": "Later?"}),
        )
        .unwrap();
    assert!(r.get("write").is_none(), "{r}");
    assert!(r["replyTaskId"].is_string());

    // Not on the guest list.
    let err = c
        .call(
            method::EVENTS_RSVP,
            json!({"accountId": ACCOUNT_ID, "eventId": "ev-holiday", "status": "ACCEPTED"}),
        )
        .unwrap_err();
    assert_eq!(key(&err).as_deref(), Some("not-invited"), "{err}");
    e.stop();
}

#[test]
fn contacts_edit_import_export() {
    let e = Engine::start();
    let mut c = e.hello();
    let r = c
        .call(
            method::CONTACTS_GET,
            json!({"accountId": ACCOUNT_ID, "contactId": "ann"}),
        )
        .unwrap();
    assert_eq!(r["data"]["title"], "Engineer", "{r}");
    assert_eq!(r["metadata"]["origin"], "CardDAV");

    let r = c
        .call(
            method::CONTACTS_UPDATE,
            json!({"accountId": ACCOUNT_ID, "contactId": "ann",
                   "patch": {"title": "Lead", "phoneNumbers": [{"value": "+1 555 0100", "type": "cell"}]}}),
        )
        .unwrap();
    let vcf = r["contact"]["info"]["vcf"].as_str().unwrap();
    assert!(vcf.contains("TITLE:Lead"), "{vcf}");
    assert!(
        vcf.contains("TEL;TYPE=cell:+1 555 0100") || vcf.contains("TEL;TYPE=CELL:+1 555 0100"),
        "{vcf}"
    );
    assert!(vcf.contains("X-CUSTOM:kept"), "{vcf}");
    assert!(r["taskId"].is_string());

    let r = c
        .call(
            method::CONTACTS_CREATE,
            json!({"accountId": ACCOUNT_ID,
                   "data": {"name": {"givenName": "Bo", "familyName": "Brown"},
                            "emailAddresses": [{"value": "bo@example.com"}]}}),
        )
        .unwrap();
    assert_eq!(r["contact"]["email"], "bo@example.com", "{r}");
    assert!(r["contact"]["id"].is_string());

    let file = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Cy\r\nEMAIL:cy@example.com\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Di\r\nEMAIL:di@example.com\r\nEND:VCARD\r\n";
    let r = c
        .call(
            method::CONTACTS_IMPORT,
            json!({"accountId": ACCOUNT_ID, "vcf": file}),
        )
        .unwrap();
    let ids: Vec<String> = r["contactIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2, "{r}");

    let mut wanted = vec!["ann".to_owned()];
    wanted.extend(ids);
    let r = c
        .call(
            method::CONTACTS_EXPORT,
            json!({"accountId": ACCOUNT_ID, "ids": wanted}),
        )
        .unwrap();
    let out = r["vcf"].as_str().unwrap();
    assert_eq!(out.matches("BEGIN:VCARD").count(), 3, "{out}");
    assert!(out.contains("TITLE:Lead") && out.contains("FN:Di"), "{out}");
    e.stop();
}
