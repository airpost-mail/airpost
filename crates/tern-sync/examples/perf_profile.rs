// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A large synthetic profile for the thin-clients read measurement (docs/thin-clients-plan.md
//! 4.7, docs/thin-clients/perf.md). It writes `<dir>/tern.db` the way sync does: every message
//! goes through `MailProcessor` (threading by References or X-GM-THRID, placements, labels,
//! ThreadCategory, ThreadCounts, ThreadSearch, mail contacts), bodies through
//! `retrieved_message_body`, plus CardDAV contacts, calendars and events through the store.
//! Nothing comes from a real mailbox. Deterministic for a given scale.
//!
//!     cargo run --release -p tern-sync --example perf_profile -- <dir> [--scale 1.0] [--busy-top]
//!
//! `<dir>/profile.json` names what the measurement needs: accounts, folder ids, the clock the
//! dates are relative to, a 50-message thread per account.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tern_core::{Account, Address, ManualClock, MessageFlags, MessageHeader};
use tern_imap::ImapMessage;
use tern_mime::ParsedMessage;
use tern_store::{
    Calendar, Contact, ContactBook, ContactGroup, DeltaStream, Event, Folder, IcsEventFields,
    Label, Model, Store,
};
use tern_sync::mail_processor::MailProcessor;

/// The clock every date is relative to: 2026-10-01T00:00:00Z.
const NOW: i64 = 1_790_812_800;
const DAY: i64 = 86_400;
const YEARS: i64 = 6;
/// Messages per batch transaction.
const BATCH: usize = 1000;
/// The length of the thread the "open a 50-message thread" measurement opens.
const LONG_THREAD: usize = 50;

struct Null;

impl Write for Null {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// xorshift64*, so the profile is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

const FIRST: &[&str] = &[
    "Ann", "Bob", "Carol", "Dan", "Eve", "Frank", "Grace", "Hal", "Ida", "Jo", "Ken", "Lea", "Mo",
    "Ned", "Ola", "Pia", "Quinn", "Rosa", "Sam", "Tara", "Uma", "Vic", "Wen", "Xavi", "Yara",
    "Zoe", "Amir", "Bea", "Cyrus", "Dina", "Emil", "Fatima", "Goran", "Hana", "Ivan", "Jun",
    "Kofi", "Lin", "Mateo", "Nia",
];
const LAST: &[&str] = &[
    "Smith", "Jones", "Lee", "Fischer", "King", "Brown", "Moss", "Polastri", "Ocean", "Hopper",
    "Abelson", "Wells", "Marsh", "Thompson", "Farah", "Kelly", "Nordmann", "Fabray", "Okafor",
    "Tanaka", "Silva", "Novak", "Haddad", "Larsen", "Murphy", "Rossi", "Kowalski", "Nguyen",
];
const DOMAINS: &[&str] = &[
    "example.com",
    "example.org",
    "example.net",
    "corp.test",
    "mail.test",
    "shop.test",
    "news.test",
    "lists.test",
    "uni.test",
    "agency.test",
];
const TOPICS: &[&str] = &[
    "budget",
    "launch",
    "roadmap",
    "invoice",
    "contract",
    "offsite",
    "hiring",
    "design review",
    "quarterly report",
    "release notes",
    "travel plans",
    "dinner",
    "migration",
    "security audit",
    "onboarding",
    "newsletter",
    "receipt",
    "shipping update",
    "flight",
    "hotel booking",
    "conference",
    "workshop",
    "interview",
    "proposal",
    "feedback",
    "retro",
    "standup notes",
    "benchmark",
    "outage",
    "postmortem",
    "renewal",
    "subscription",
    "tax documents",
    "lease",
    "school trip",
    "birthday",
    "photos",
    "recipe",
    "garden",
    "marathon",
];
const WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "this",
    "that",
    "from",
    "about",
    "please",
    "thanks",
    "next",
    "week",
    "meeting",
    "draft",
    "attached",
    "review",
    "update",
    "schedule",
    "project",
    "team",
    "client",
    "numbers",
    "deadline",
    "question",
    "answer",
    "follow",
    "up",
    "agenda",
    "notes",
    "summary",
    "plan",
    "estimate",
    "cost",
    "timeline",
    "approve",
    "sign",
    "document",
    "share",
    "folder",
    "link",
    "call",
    "tomorrow",
    "monday",
    "friday",
    "morning",
    "afternoon",
    "office",
    "remote",
    "travel",
    "booking",
    "confirm",
    "cancel",
    "reschedule",
    "invoice",
    "payment",
    "order",
    "delivery",
    "tracking",
    "package",
    "account",
    "password",
    "login",
    "support",
    "ticket",
    "issue",
    "bug",
    "fix",
    "deploy",
    "server",
    "database",
    "query",
    "latency",
    "performance",
    "metrics",
    "dashboard",
    "report",
    "quarter",
    "revenue",
    "growth",
    "market",
    "launch",
    "feature",
    "customer",
    "partner",
    "vendor",
    "contract",
    "legal",
    "policy",
    "kitchen",
    "weekend",
    "holiday",
    "family",
    "photos",
    "dinner",
    "lunch",
    "coffee",
    "zebra",
    "quokka",
    "aurora",
    "glacier",
    "harbor",
    "lantern",
    "meadow",
    "nimbus",
    "orchard",
    "pebble",
];

struct Folders {
    /// Thread home for archived mail: Archive, or All Mail on Gmail.
    archive: Folder,
    inbox: Option<Folder>,
    sent: Option<Folder>,
    trash: Folder,
    spam: Folder,
    /// User folders (IMAP) a thread may be filed in instead of the archive.
    custom: Vec<Folder>,
    /// Gmail: the label paths a thread may carry.
    user_labels: Vec<String>,
}

struct Spec {
    id: &'static str,
    email: &'static str,
    gmail: bool,
    messages: usize,
    inbox_share: f64,
}

fn person(rng: &mut Rng) -> Address {
    let first = *rng.pick(FIRST);
    let last = *rng.pick(LAST);
    let domain = *rng.pick(DOMAINS);
    let n = rng.below(40);
    Address::new(
        Some(&format!("{first} {last}")),
        &format!(
            "{}.{}{}@{domain}",
            first.to_lowercase(),
            last.to_lowercase(),
            if n > 0 { n.to_string() } else { String::new() }
        ),
    )
}

fn paragraph(rng: &mut Rng, words: usize) -> String {
    let mut out = Vec::with_capacity(words);
    for _ in 0..words {
        out.push(*rng.pick(WORDS));
    }
    let mut s = out.join(" ");
    if let Some(first) = s.get(0..1) {
        s.replace_range(0..1, &first.to_uppercase());
    }
    s.push('.');
    s
}

/// Thread lengths: mostly single messages, a long tail of long ones.
fn thread_length(rng: &mut Rng) -> usize {
    let r = rng.unit();
    if r < 0.68 {
        1
    } else if r < 0.88 {
        2 + rng.below(2) as usize
    } else if r < 0.96 {
        4 + rng.below(7) as usize
    } else if r < 0.99 {
        11 + rng.below(20) as usize
    } else {
        31 + rng.below(90) as usize
    }
}

fn save_folder(store: &Store, aid: &str, path: &str, role: &str) -> Folder {
    let mut f = Folder::new(&tern_core::ids::folder_id(aid, path), aid, 0);
    f.set_path(path);
    f.set_role(role);
    store.save(&mut f).unwrap();
    f
}

fn save_label(store: &Store, aid: &str, path: &str, role: &str) -> String {
    let mut l = Label::new(&tern_core::ids::folder_id(aid, path), aid, 0);
    l.set_path(path);
    l.set_role(role);
    store.save(&mut l).unwrap();
    l.id().to_owned()
}

fn folders_for(store: &Store, spec: &Spec, ids: &mut BTreeMap<String, String>) -> Folders {
    let aid = spec.id;
    if spec.gmail {
        let all = save_folder(store, aid, "[Gmail]/All Mail", "all");
        let trash = save_folder(store, aid, "[Gmail]/Trash", "trash");
        let spam = save_folder(store, aid, "[Gmail]/Spam", "spam");
        ids.insert("all".into(), all.id().into());
        ids.insert("trash".into(), trash.id().into());
        ids.insert("spam".into(), spam.id().into());
        ids.insert("inbox".into(), save_label(store, aid, "INBOX", "inbox"));
        ids.insert(
            "sent".into(),
            save_label(store, aid, "[Gmail]/Sent Mail", "sent"),
        );
        ids.insert(
            "drafts".into(),
            save_label(store, aid, "[Gmail]/Drafts", "drafts"),
        );
        ids.insert(
            "important".into(),
            save_label(store, aid, "[Gmail]/Important", "important"),
        );
        ids.insert(
            "starred".into(),
            save_label(store, aid, "[Gmail]/Starred", "starred"),
        );
        let mut user_labels = Vec::new();
        for name in [
            "Travel",
            "Receipts",
            "Projects",
            "Family",
            "Newsletters",
            "Finance",
            "Clients",
            "Hiring",
            "Events",
            "Reading",
            "Archive/2021",
            "Archive/2022",
        ] {
            let id = save_label(store, aid, name, "");
            if name == "Travel" {
                ids.insert("label".into(), id);
            }
            user_labels.push(name.to_owned());
        }
        Folders {
            archive: all,
            inbox: None,
            sent: None,
            trash,
            spam,
            custom: Vec::new(),
            user_labels,
        }
    } else {
        let inbox = save_folder(store, aid, "INBOX", "inbox");
        let sent = save_folder(store, aid, "Sent", "sent");
        save_folder(store, aid, "Drafts", "drafts");
        let trash = save_folder(store, aid, "Trash", "trash");
        let spam = save_folder(store, aid, "Junk", "spam");
        let archive = save_folder(store, aid, "Archive", "archive");
        let custom: Vec<Folder> = ["Projects", "Projects/Alpha", "Receipts", "Travel", "Lists"]
            .iter()
            .map(|p| save_folder(store, aid, p, ""))
            .collect();
        for (k, f) in [
            ("inbox", &inbox),
            ("sent", &sent),
            ("trash", &trash),
            ("spam", &spam),
            ("archive", &archive),
            ("folder", &custom[0]),
        ] {
            ids.insert(k.into(), f.id().into());
        }
        Folders {
            archive,
            inbox: Some(inbox),
            sent: Some(sent),
            trash,
            spam,
            custom,
            user_labels: Vec::new(),
        }
    }
}

struct Pending {
    remote: ImapMessage,
    folder: Folder,
    body: Option<ParsedMessage>,
}

fn html_body(rng: &mut Rng, topic: &str, from: &Address) -> (String, String) {
    let mut html = String::from("<div dir=\"ltr\">");
    let mut text = String::new();
    let greet = format!("Hi, about the {topic}:");
    html.push_str(&format!("<p>{greet}</p>"));
    text.push_str(&greet);
    for _ in 0..(2 + rng.below(6)) {
        let words = 25 + rng.below(50) as usize;
        let p = paragraph(rng, words);
        html.push_str(&format!("<p>{p}</p>"));
        text.push(' ');
        text.push_str(&p);
    }
    let sig = from.name.clone().unwrap_or_default();
    html.push_str(&format!("<p>-- <br>{sig}</p></div>"));
    text.push_str(&format!(" -- {sig}"));
    (html, text)
}

fn seed_account(
    store: &Store,
    config_dir: &Path,
    spec: &Spec,
    rng: &mut Rng,
    busy_top: bool,
    manifest: &mut Value,
) {
    let account = Account::from_json(json!({
        "id": spec.id, "provider": "imap", "emailAddress": spec.email, "settings": {},
    }))
    .unwrap();
    let mut ids = BTreeMap::new();
    let folders = folders_for(store, spec, &mut ids);
    let me = Address::new(Some("Perf User"), spec.email);
    let mut uids: BTreeMap<String, u32> = BTreeMap::new();
    let mut next_uid = |folder: &Folder| -> u32 {
        let n = uids.entry(folder.id().to_owned()).or_insert(0);
        *n += 1;
        *n
    };
    let mut processor = MailProcessor::new(&account, store).with_config_dir(config_dir);
    processor.is_gmail = spec.gmail;

    let mut pending: Vec<Pending> = Vec::new();
    let mut written = 0usize;
    let mut thread_no = 0u64;
    let mut long_thread_root: Option<String> = None;
    let started = Instant::now();

    while written + pending.len() < spec.messages {
        thread_no += 1;
        let len = if long_thread_root.is_none() && thread_no == 7 {
            LONG_THREAD
        } else {
            thread_length(rng)
        };
        // Dense near NOW: u^2 puts most threads in the last two years. Messages are minutes
        // to days apart, back from the last one; with --busy-top forward from the first and
        // held at NOW, which puts the long threads at the top of every list.
        let u = rng.unit();
        let anchor =
            NOW - (u * u * (YEARS * 365 * DAY) as f64) as i64 - rng.below(DAY as u64) as i64;
        let mut dates: Vec<i64> = vec![anchor];
        for _ in 1..len {
            let gap = 600 + rng.below(3 * DAY as u64) as i64;
            let prev = *dates.last().unwrap();
            dates.push(if busy_top {
                (prev + gap).min(NOW - rng.below(3600) as i64)
            } else {
                prev - gap
            });
        }
        if !busy_top {
            dates.reverse();
        }
        let recent = NOW - dates[if busy_top { 0 } else { len - 1 }] < 120 * DAY;
        let topic = *rng.pick(TOPICS);
        let subject = format!("{} {}", topic[..1].to_uppercase() + &topic[1..], thread_no);
        let mut people: Vec<Address> = (0..(1 + rng.below(4))).map(|_| person(rng)).collect();
        if rng.chance(0.15) {
            people.extend((0..rng.below(8)).map(|_| person(rng)));
        }
        let place = rng.unit();
        let (in_trash, in_spam) = (place < 0.01, (0.01..0.02).contains(&place));
        let in_inbox = !in_trash
            && !in_spam
            && (rng.chance(spec.inbox_share) || (recent && rng.chance(0.5)) || len == LONG_THREAD);
        let custom = (!in_trash && !in_spam && !folders.custom.is_empty() && rng.chance(0.12))
            .then(|| rng.pick(&folders.custom).clone());
        let mut thread_labels: Vec<String> = Vec::new();
        if spec.gmail && !in_trash && !in_spam {
            if in_inbox {
                thread_labels.push("\\Inbox".into());
            }
            if rng.chance(0.2) {
                thread_labels.push("\\Important".into());
            }
            if rng.chance(0.3) {
                thread_labels.push(rng.pick(&folders.user_labels).clone());
            }
        }
        let starred_thread = rng.chance(0.03);
        let gm_thrid = 1_000_000 + thread_no;
        let root_id = format!("t{}-{thread_no}@perf.test", spec.id);
        let mut references: Vec<String> = Vec::new();
        for (k, &date) in dates.iter().enumerate() {
            let from_me = (k % 2 == 1 && rng.chance(0.6)) || (k == 0 && rng.chance(0.12));
            let sender = if from_me {
                me.clone()
            } else {
                people[k % people.len()].clone()
            };
            let mut to: Vec<Address> = if from_me {
                people.clone()
            } else {
                vec![me.clone()]
            };
            let cc: Vec<Address> = if !from_me && people.len() > 1 {
                people
                    .iter()
                    .filter(|p| p.email != sender.email)
                    .take(3)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            to.truncate(10);
            let mid = if k == 0 {
                root_id.clone()
            } else {
                format!("m{}-{thread_no}-{k}@perf.test", spec.id)
            };
            let last = k + 1 == len;
            let unread = !from_me && last && ((in_inbox && rng.chance(0.3)) || rng.chance(0.02));
            let mut flags = MessageFlags::empty();
            if !unread {
                flags |= MessageFlags::SEEN;
            }
            if starred_thread && last {
                flags |= MessageFlags::FLAGGED;
            }
            let subj = if k == 0 {
                subject.clone()
            } else {
                format!("Re: {subject}")
            };
            let header = MessageHeader {
                date_raw: None,
                date: Some(date),
                subject_raw: Some(subj.clone().into_bytes()),
                subject: Some(subj),
                message_id: Some(mid.clone()),
                in_reply_to: references.last().cloned().into_iter().collect(),
                references: references.iter().rev().take(20).rev().cloned().collect(),
                from: vec![sender.clone()],
                to,
                cc,
                ..Default::default()
            };
            let folder = if in_trash {
                folders.trash.clone()
            } else if in_spam {
                folders.spam.clone()
            } else if spec.gmail {
                folders.archive.clone()
            } else if from_me {
                folders.sent.clone().unwrap()
            } else if in_inbox {
                folders.inbox.clone().unwrap()
            } else if let Some(c) = &custom {
                c.clone()
            } else {
                folders.archive.clone()
            };
            let mut gmail_labels = thread_labels.clone();
            if spec.gmail && from_me {
                gmail_labels.push("\\Sent".into());
            }
            let remote = ImapMessage {
                uid: next_uid(&folder),
                flags,
                internal_date: Some(date),
                gmail_labels: if spec.gmail { gmail_labels } else { Vec::new() },
                gmail_thread_id: spec.gmail.then_some(gm_thrid),
                gmail_message_id: spec.gmail.then_some(gm_thrid * 1000 + k as u64),
                header: Some(header.clone()),
                ..Default::default()
            };
            // Bodies: what sync would have fetched, the last year and some older mail.
            let has_body = NOW - date < 365 * DAY || rng.chance(0.15) || len == LONG_THREAD;
            let body = has_body.then(|| {
                let (html, text) = html_body(rng, topic, &sender);
                ParsedMessage {
                    header,
                    body: html,
                    plaintext: false,
                    text,
                    ..Default::default()
                }
            });
            pending.push(Pending {
                remote,
                folder,
                body,
            });
            references.push(mid);
        }
        if len == LONG_THREAD && long_thread_root.is_none() {
            long_thread_root = Some(root_id);
        }
        if pending.len() >= BATCH {
            written += flush(store, &processor, &mut pending);
            eprintln!(
                "{}: {written} messages, {:.0}/s",
                spec.id,
                written as f64 / started.elapsed().as_secs_f64()
            );
        }
    }
    written += flush(store, &processor, &mut pending);

    // The thread the open measurement uses.
    let long_thread: Option<String> = long_thread_root.and_then(|root| {
        store
            .conn()
            .query_row(
                "SELECT threadId FROM Message WHERE headerMessageId = ? AND accountId = ?",
                [root.as_str(), spec.id],
                |r| r.get(0),
            )
            .ok()
    });

    let (calendars, events) = seed_calendars(store, spec, rng);
    let carddav = seed_contacts(store, spec, rng);
    manifest["accounts"].as_array_mut().unwrap().push(json!({
        "id": spec.id,
        "emailAddress": spec.email,
        "gmail": spec.gmail,
        "messages": written,
        "threads": thread_no,
        "categories": ids,
        "longThread": long_thread,
        "calendars": calendars,
        "events": events,
        "carddavContacts": carddav,
    }));
}

fn flush(store: &Store, processor: &MailProcessor<'_>, pending: &mut Vec<Pending>) -> usize {
    let batch = std::mem::take(pending);
    let n = batch.len();
    store
        .transaction("perfSeed", |_| {
            for p in batch {
                let mut msg =
                    processor.insert_fallback_to_update_message(&p.remote, &p.folder, NOW)?;
                if let Some(body) = &p.body {
                    processor.retrieved_message_body(&mut msg, body)?;
                }
            }
            Ok(())
        })
        .unwrap();
    n
}

fn ics_time(t: i64) -> String {
    let days = t.div_euclid(DAY);
    let secs = t.rem_euclid(DAY);
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

fn seed_calendars(store: &Store, spec: &Spec, rng: &mut Rng) -> (Vec<String>, usize) {
    let names = if spec.gmail {
        vec!["Personal", "Work", "Holidays"]
    } else {
        vec!["Calendar", "Team"]
    };
    let mut ids = Vec::new();
    let mut count = 0;
    store
        .transaction("perfSeedEvents", |s| {
            for name in &names {
                let cid = tern_core::ids::calendar_id(spec.id, &format!("/cal/{}/{name}/", spec.id));
                let mut cal = Calendar::new(&cid, spec.id);
                cal.set_name(name);
                cal.set_path(&format!("/cal/{}/{name}/", spec.id));
                s.save(&mut cal)?;
                ids.push(cid.clone());
                for n in 0..800 {
                    // Three years around NOW, an hour or so each, a tenth weekly for a year.
                    let start = NOW - 2 * 365 * DAY + rng.below(3 * 365 * DAY as u64) as i64;
                    let start = start - start % 900;
                    let end = start + 1800 + 900 * rng.below(6) as i64;
                    let weekly = rng.chance(0.1);
                    let uid = format!("ev-{}-{name}-{n}@perf.test", spec.id);
                    let summary = format!("{} {n}", rng.pick(TOPICS));
                    let rrule = if weekly { "RRULE:FREQ=WEEKLY;COUNT=52\r\n" } else { "" };
                    let ics = format!(
                        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Tern//perf//EN\r\nBEGIN:VEVENT\r\n\
                         UID:{uid}\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nDTEND:{}\r\n{rrule}SUMMARY:{summary}\r\n\
                         LOCATION:Room {}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
                        ics_time(start),
                        ics_time(start),
                        ics_time(end),
                        rng.below(20),
                    );
                    let fields = IcsEventFields {
                        uid: uid.clone(),
                        recurrence_id: String::new(),
                        status: String::new(),
                        recurrence_start: start,
                        recurrence_end: if weekly { end + 51 * 7 * DAY } else { end },
                        summary,
                        description: String::new(),
                        location: String::new(),
                        attendees: Vec::new(),
                    };
                    let mut e = Event::create(&format!("etag-{n}"), spec.id, &cid, &ics, &fields);
                    s.save(&mut e)?;
                    count += 1;
                }
            }
            Ok(())
        })
        .unwrap();
    (ids, count)
}

fn seed_contacts(store: &Store, spec: &Spec, rng: &mut Rng) -> usize {
    let book_id = format!("book-{}", spec.id);
    let mut n = 0;
    store
        .transaction("perfSeedContacts", |s| {
            let mut book = ContactBook::create(&book_id, spec.id);
            book.set_source("carddav");
            book.set_url(&format!("/dav/{}/contacts/", spec.id));
            s.save(&mut book)?;
            let groups: Vec<ContactGroup> = (0..8)
                .map(|g| {
                    let id = format!("grp-{}-{g}", spec.id);
                    let mut group = ContactGroup::create(&id, spec.id);
                    group.set_name(&format!("Group {g}"));
                    group.set_book_id(&book_id);
                    s.save(&mut group)?;
                    Ok(group)
                })
                .collect::<tern_core::Result<_>>()?;
            let mut members: Vec<Vec<String>> = vec![Vec::new(); groups.len()];
            for i in 0..1500 {
                let p = person(rng);
                let id = format!("card-{}-{i}", spec.id);
                let mut c = Contact::create(&id, spec.id, &p.email, 0, "carddav");
                c.set_name(p.name.as_deref().unwrap_or(""));
                c.set_book_id(&book_id);
                s.save(&mut c)?;
                if rng.chance(0.3) {
                    members[rng.below(groups.len() as u64) as usize].push(id);
                }
                n += 1;
            }
            for (group, ids) in groups.iter().zip(&members) {
                group.sync_members(s, ids)?;
            }
            Ok(())
        })
        .unwrap();
    n
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(
        args.next()
            .expect("usage: perf_profile <dir> [--scale N] [--busy-top]"),
    );
    let mut scale = 1.0f64;
    let mut busy_top = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).expect("--scale N"),
            "--busy-top" => busy_top = true,
            other => panic!("unknown argument {other}"),
        }
    }
    if dir.join("tern.db").exists() {
        panic!("{} already has a tern.db", dir.display());
    }
    std::fs::create_dir_all(&dir).unwrap();
    let mut out = Vec::new();
    tern_store::migrate::migrate(&dir, &ManualClock::new(NOW), &mut out).unwrap();
    let store = Store::open(&dir, DeltaStream::new(Box::new(Null))).unwrap();
    store.set_clock(Arc::new(ManualClock::new(NOW)));
    let n = |m: usize| (m as f64 * scale) as usize;
    let specs = [
        Spec {
            id: "perfgml1",
            email: "perf.user@gmail.test",
            gmail: true,
            messages: n(55_000),
            inbox_share: 0.04,
        },
        Spec {
            id: "perfwrk1",
            email: "perf.user@work.test",
            gmail: false,
            messages: n(30_000),
            inbox_share: 0.08,
        },
        Spec {
            id: "perfhom1",
            email: "perf.user@home.test",
            gmail: false,
            messages: n(15_000),
            inbox_share: 0.1,
        },
    ];
    let mut rng = Rng(0x7e57_7e57_0d15_ea5e);
    let mut manifest = json!({"now": NOW, "scale": scale, "busyTop": busy_top, "accounts": []});
    let started = Instant::now();
    for spec in &specs {
        seed_account(&store, &dir, spec, &mut rng, busy_top, &mut manifest);
    }
    let violations = tern_store::invariants::check(store.conn(), &[]).unwrap();
    assert!(violations.is_empty(), "{violations:#?}");
    store.conn().execute_batch("ANALYZE").unwrap();
    let count = |sql: &str| -> i64 { store.conn().query_row(sql, [], |r| r.get(0)).unwrap() };
    manifest["totals"] = json!({
        "messages": count("SELECT COUNT(*) FROM Message"),
        "threads": count("SELECT COUNT(*) FROM Thread"),
        "bodies": count("SELECT COUNT(*) FROM MessageBody WHERE value IS NOT NULL"),
        "contacts": count("SELECT COUNT(*) FROM Contact"),
        "events": count("SELECT COUNT(*) FROM Event"),
        "seedSeconds": started.elapsed().as_secs(),
    });
    std::fs::write(
        dir.join("profile.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    println!("{}", manifest["totals"]);
}
