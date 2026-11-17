// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The P1-I gate: the exchanges of `apps/engine/test/conformance/probes.py`, driven through
//! [`airpost_imap::Session`] against the fake server, Dovecot and Cyrus, plus a scripted
//! CONDSTORE / QRESYNC / IDLE session and the fake's provider personalities.
//!
//! Needs `python3` (and Docker for Dovecot and Cyrus), so every test is ignored by default:
//!
//! ```sh
//! CARGO_TARGET_DIR=target/p1-i cargo test -p airpost-imap --test session -- --ignored --test-threads 4
//! cargo test -p airpost-imap --test session -- --ignored fake::     # one server kind
//! ```

mod support;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use airpost_core::protolog::AccumulatingLogger;
use airpost_core::{ErrorCode, FolderFlags, MessageFlags, Security, UidSet};
use airpost_imap::{
    FetchKind, IdleInterrupt, ImapAuth, ImapConfig, ImapSession, ListedFolder, Session, StoreOp,
};
use airpost_providers::Provider;
use serde_json::json;
use support::ServerProc;

const INBOX_COUNT: u32 = 12;

fn config(s: &ServerProc) -> ImapConfig {
    let mut c = ImapConfig::new(
        s.host.clone(),
        s.port,
        Security::Clear,
        s.user.clone(),
        ImapAuth::Password(s.password.clone()),
    );
    c.timeout = Duration::from_secs(20);
    c.client_identity = Some(("Airpost".into(), "test".into()));
    if std::env::var_os("AIRPOST_IMAP_TRACE").is_some() {
        c.logger = Some(Arc::new(StderrLogger));
    }
    c
}

/// `AIRPOST_IMAP_TRACE=1` prints every connection's transcript.
struct StderrLogger;

impl airpost_core::protolog::ProtocolLogger for StderrLogger {
    fn data(&self, direction: airpost_core::protolog::Direction, bytes: &[u8]) {
        if let Some(label) = direction.label() {
            eprint!("{label} {}", String::from_utf8_lossy(bytes));
        }
    }
}

fn session(s: &ServerProc) -> Session {
    Session::new(config(s))
}

fn plain_session(s: &ServerProc) -> Session {
    let mut c = config(s);
    c.disable_qresync = true;
    Session::new(c)
}

fn logged(s: &ServerProc, mut c: ImapConfig) -> (Session, Arc<AccumulatingLogger>) {
    let log = Arc::new(AccumulatingLogger::default());
    c.logger = Some(log.clone());
    let _ = s;
    (Session::new(c), log)
}

fn b64(data: &[u8]) -> String {
    use base64::Engine;
    format!(
        "b64:{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

fn raw_message(n: u32, subject: &str) -> Vec<u8> {
    format!(
        "From: x@example.test\r\nTo: test@example.test\r\nSubject: {subject}\r\nMessage-ID: <rust-{n}@example.test>\r\nDate: Mon, 05 Jan 2026 09:00:00 +0000\r\n\r\nbody {n}\r\n"
    )
    .into_bytes()
}

fn find<'a>(folders: &'a [ListedFolder], path: &str) -> Option<&'a ListedFolder> {
    folders.iter().find(|f| f.path == path)
}

fn uids_of(msgs: &[airpost_imap::ImapMessage]) -> Vec<u32> {
    msgs.iter().map(|m| m.uid).collect()
}

fn all() -> UidSet {
    UidSet::range(1, None)
}

/// Expunges `uids` from INBOX through `b`, as another client would.
fn expunge_via(b: &mut Session, uids: &UidSet) {
    b.store_flags("INBOX", uids, StoreOp::Add, MessageFlags::DELETED)
        .unwrap();
    b.expunge_uids("INBOX", uids).unwrap();
}

// -- The probes ----------------------------------------------------------------------------

/// probe_session_setup: greeting, CAPABILITY, LOGIN, ENABLE, NAMESPACE.
fn session_setup(s: &mut ServerProc) {
    let (mut a, log) = logged(s, config(s));
    a.login_if_needed().unwrap();
    assert!(!a.is_disconnected());
    assert!(!a.greeting().is_empty());
    assert!(a.capabilities().has("IMAP4rev1"));
    let caps_qresync = a.capabilities().has("QRESYNC");
    assert_eq!(a.is_qresync_enabled(), caps_qresync);
    // Cyrus offers COMPRESS=DEFLATE; everything after LOGIN then runs compressed.
    assert_eq!(a.is_compressed(), a.capabilities().has("COMPRESS=DEFLATE"));
    if s.kind == "cyrus" {
        assert!(a.is_compressed());
    }
    let ns = a.default_namespace().expect("namespace").clone();
    assert_eq!(ns.prefix, "");
    assert_eq!(ns.delimiter, Some('/'));
    let transcript = log.contents();
    assert!(transcript.contains("1 CAPABILITY") || transcript.contains("CAPABILITY"));
    assert!(
        !transcript.contains(&s.password),
        "the LOGIN line must not be logged"
    );
    if caps_qresync {
        assert!(transcript.contains(" ENABLE QRESYNC\r\n"), "{transcript}");
    }
    // ID is only for Coremail.
    assert!(!transcript.contains(" ID ("));
    let expected = match s.kind.as_str() {
        "cyrus" => Provider::Cyrus,
        _ => Provider::Dovecot,
    };
    assert_eq!(a.profile().provider, expected);

    // Wrong password: Authentication, connection kept for a retry.
    let mut c = config(s);
    c.auth = ImapAuth::Password("wrong".into());
    let err = Session::new(c).login_if_needed().unwrap_err();
    assert_eq!(err.code, ErrorCode::Authentication, "{err}");
}

/// probe_list_status: LIST, LIST INBOX, STATUS of existing and missing folders.
fn list_status(s: &mut ServerProc) {
    let mut a = session(s);
    let folders = a.list_folders().unwrap();
    let inbox = find(&folders, "INBOX").expect("INBOX listed");
    assert_eq!(inbox.delimiter, Some('/'));
    assert!(find(&folders, "Archive").is_some(), "{folders:?}");
    let sent = find(&folders, "Sent").expect("Sent listed");
    assert!(sent.flags.contains(FolderFlags::SENT), "{sent:?}");
    let trash = find(&folders, "Trash").expect("Trash listed");
    assert!(trash.flags.contains(FolderFlags::TRASH));

    let st = a.status("INBOX").unwrap();
    assert_eq!(st.message_count, INBOX_COUNT);
    assert_eq!(st.unseen_count, 2);
    assert_eq!(st.uid_next, INBOX_COUNT + 1);
    assert_ne!(st.uid_validity, 0);
    if a.capabilities().has("CONDSTORE") {
        assert_ne!(st.highest_modseq, 0);
    }
    let st = a.status("Archive").unwrap();
    assert_eq!(st.message_count, 2);
    let err = a.status("Nope").unwrap_err();
    assert_eq!(err.code, ErrorCode::NonExistantFolder);
    // A NO is not a connection problem.
    assert!(!a.is_disconnected());
    a.noop().unwrap();
}

/// probe_select_fetch: SELECT, header and attribute fetches, bodies, missing UIDs.
fn select_fetch(s: &mut ServerProc) {
    let mut a = session(s);
    let info = a.select("INBOX").unwrap();
    assert_eq!(info.exists, INBOX_COUNT);
    assert_eq!(info.uid_next, INBOX_COUNT + 1);
    assert_ne!(info.uid_validity, 0);
    assert_eq!(a.is_qresync_enabled(), info.highest_modseq != 0);
    assert_eq!(a.current_folder(), Some("INBOX"));

    let heavy = a
        .fetch_by_uid("INBOX", FetchKind::Heavy, &UidSet::range(1, Some(3)))
        .unwrap();
    assert_eq!(uids_of(&heavy.messages), vec![1, 2, 3]);
    assert!(!heavy.hit_message_limit);
    for m in &heavy.messages {
        let h = m.header.as_ref().expect("header");
        assert_eq!(
            h.subject.as_deref(),
            Some(format!("Message {}", m.uid).as_str())
        );
        let id = h.message_id.as_deref().unwrap();
        assert!(id.starts_with(&format!("harness-{}-", m.uid)), "{id}");
        assert!(h.date.is_some());
        assert_eq!(h.from.len(), 1);
        assert_eq!(h.to[0].email, "test@example.test");
        assert!(m.internal_date.is_some());
    }

    let light = a.fetch_by_uid("INBOX", FetchKind::Light, &all()).unwrap();
    assert_eq!(
        uids_of(&light.messages),
        (1..=INBOX_COUNT).collect::<Vec<_>>()
    );
    for m in &light.messages {
        assert!(m.header.is_none());
        assert_eq!(
            m.flags.contains(MessageFlags::SEEN),
            !(m.uid == 2 || m.uid == 3)
        );
    }

    let body = a.fetch_message_bytes("INBOX", 2).unwrap().expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("Subject: Message 2"), "{text}");
    assert!(text.contains("This is the body of message 2."));
    assert_eq!(a.fetch_message_bytes("INBOX", 9999).unwrap(), None);
    // Fetching a body must not mark it read (BODY.PEEK).
    let m = a
        .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::single(2))
        .unwrap();
    assert!(!m.messages[0].flags.contains(MessageFlags::SEEN));

    let none = a
        .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::single(9999))
        .unwrap();
    assert!(none.messages.is_empty());
    // `20:*` matches the highest UID on the server; the filter drops it.
    let past = a
        .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::range(20, None))
        .unwrap();
    assert!(past.messages.is_empty());
    let empty = a
        .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::new())
        .unwrap();
    assert!(empty.messages.is_empty());

    // Another folder: selecting is implicit.
    let archive = a.fetch_by_uid("Archive", FetchKind::Heavy, &all()).unwrap();
    assert_eq!(archive.messages.len(), 2);
    assert_eq!(a.current_folder(), Some("Archive"));
    let err = a.select("Nope").unwrap_err();
    assert_eq!(err.code, ErrorCode::NonExistantFolder);
    assert_eq!(a.current_folder(), None);
}

/// probe_store: flag changes and what another session sees.
fn store(s: &mut ServerProc) {
    let mut a = session(s);
    let mut b = session(s);
    b.select("INBOX").unwrap();
    a.store_flags(
        "INBOX",
        &UidSet::single(1),
        StoreOp::Add,
        MessageFlags::FLAGGED,
    )
    .unwrap();
    a.store_flags(
        "INBOX",
        &UidSet::single(2),
        StoreOp::Add,
        MessageFlags::FLAGGED,
    )
    .unwrap();
    a.store_flags(
        "INBOX",
        &UidSet::single(1),
        StoreOp::Remove,
        MessageFlags::FLAGGED,
    )
    .unwrap();
    a.store_flags(
        "INBOX",
        &UidSet::single(2),
        StoreOp::Add,
        MessageFlags::FLAGGED,
    )
    .unwrap();
    b.noop().unwrap();
    let m = b
        .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::range(1, Some(2)))
        .unwrap()
        .messages;
    assert!(!m[0].flags.contains(MessageFlags::FLAGGED));
    assert!(m[1].flags.contains(MessageFlags::FLAGGED));

    // Set replaces the flags; keywords survive a round trip.
    a.store_flags(
        "INBOX",
        &UidSet::single(5),
        StoreOp::Set,
        MessageFlags::SEEN | MessageFlags::FORWARDED,
    )
    .unwrap();
    let m = a
        .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::single(5))
        .unwrap()
        .messages;
    assert_eq!(m[0].flags, MessageFlags::SEEN | MessageFlags::FORWARDED);
    let err = a
        .store_flags("Nope", &UidSet::single(1), StoreOp::Add, MessageFlags::SEEN)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NonExistantFolder);
}

/// probe_bulk_store_modseq: one STORE is one modseq.
fn bulk_store_modseq(s: &mut ServerProc) {
    let mut a = plain_session(s);
    a.login_if_needed().unwrap();
    if !a.capabilities().has("CONDSTORE") {
        return;
    }
    let hms = a.select("INBOX").unwrap().highest_modseq;
    a.store_flags(
        "INBOX",
        &UidSet::range(1, Some(6)),
        StoreOp::Add,
        MessageFlags::FLAGGED,
    )
    .unwrap();
    let changed = a
        .sync_by_uid("INBOX", FetchKind::Light, &all(), hms)
        .unwrap()
        .modified_or_added;
    assert_eq!(uids_of(&changed), (1..=6).collect::<Vec<_>>());
    let st = a.status("INBOX").unwrap();
    let distinct: std::collections::BTreeSet<u64> = changed.iter().map(|m| m.modseq).collect();
    // Dovecot (and the fake) give one STORE one modseq; Cyrus gives each message its own.
    if s.kind != "cyrus" {
        assert_eq!(distinct.len(), 1, "{distinct:?}");
    }
    assert!(distinct.iter().all(|m| *m > hms));
    assert_eq!(*distinct.iter().next_back().unwrap(), st.highest_modseq);
}

/// probe_changedsince: CHANGEDSINCE with and without VANISHED.
fn changedsince(s: &mut ServerProc) {
    let mut a = session(s);
    let mut b = session(s);
    let hms = a.select("INBOX").unwrap().highest_modseq;
    if !a.is_qresync_enabled() {
        return;
    }
    b.store_flags(
        "INBOX",
        &UidSet::single(3),
        StoreOp::Add,
        MessageFlags::SEEN,
    )
    .unwrap();
    let r = a
        .sync_by_uid("INBOX", FetchKind::Light, &all(), hms)
        .unwrap();
    assert_eq!(uids_of(&r.modified_or_added), vec![3]);
    assert!(r.modified_or_added[0].flags.contains(MessageFlags::SEEN));
    assert!(r.modified_or_added[0].modseq > hms);

    a.noop().unwrap();
    let hms2 = a.status("INBOX").unwrap().highest_modseq;
    expunge_via(&mut b, &UidSet::range(4, Some(5)));
    let r = a
        .sync_by_uid("INBOX", FetchKind::Light, &all(), hms2)
        .unwrap();
    let mut vanished = r.vanished.clone();
    vanished.extend(&a.take_vanished("INBOX"));
    assert_eq!(vanished, UidSet::range(4, Some(5)), "{r:?}");
    // Cyrus answers from the session's view first: FETCH for 4 and 5 (now \Deleted), then
    // VANISHED. Whatever was fetched is also reported vanished, so applying VANISHED after
    // the changes, as the sync worker does, ends right.
    for m in &r.modified_or_added {
        if m.uid == 4 || m.uid == 5 {
            assert!(m.flags.contains(MessageFlags::DELETED), "{m:?}");
        }
    }
    // take_vanished drains.
    assert!(a.take_vanished("INBOX").is_empty());
    // Asked again with the same modseq, the server reports the expunges again (EARLIER).
    let again = a
        .sync_by_uid("INBOX", FetchKind::Light, &all(), hms2)
        .unwrap();
    assert_eq!(again.vanished, UidSet::range(4, Some(5)));

    // Heavy CHANGEDSINCE: headers for changed messages.
    b.store_flags(
        "INBOX",
        &UidSet::single(6),
        StoreOp::Add,
        MessageFlags::FLAGGED,
    )
    .unwrap();
    let r = a
        .sync_by_uid("INBOX", FetchKind::Heavy, &UidSet::range(1, Some(12)), hms2)
        .unwrap();
    let six = r.modified_or_added.iter().find(|m| m.uid == 6).unwrap();
    assert!(six.header.is_some());
}

/// probe_copy_move_append: COPY, MOVE, APPEND, EXPUNGE, CREATE / RENAME / DELETE.
fn copy_move_append(s: &mut ServerProc) {
    let mut a = session(s);
    let uidplus = {
        a.login_if_needed().unwrap();
        a.capabilities().has("UIDPLUS")
    };
    let archive_next = a.status("Archive").unwrap().uid_next;
    let copied = a
        .copy_uids("INBOX", &UidSet::range(1, Some(2)), "Archive")
        .unwrap();
    let moved = a.move_uids("INBOX", &UidSet::single(3), "Archive").unwrap();
    if uidplus {
        assert_eq!(copied.0, vec![(1, archive_next), (2, archive_next + 1)]);
        assert_eq!(moved.0, vec![(3, archive_next + 2)]);
        assert_eq!(copied.get(2), Some(archive_next + 1));
    }
    let st = a.status("Archive").unwrap();
    assert_eq!(st.message_count, 5);
    let inbox = a.fetch_by_uid("INBOX", FetchKind::Light, &all()).unwrap();
    assert!(!uids_of(&inbox.messages).contains(&3));

    let raw = raw_message(1, "appended");
    let uid = a.append("Archive", &raw, MessageFlags::SEEN).unwrap();
    if uidplus {
        assert_eq!(uid, Some(archive_next + 3));
    }
    let found = a
        .find_uids_of_recent_header_message_id("Archive", "rust-1@example.test")
        .unwrap();
    assert_eq!(found.len(), 1);
    if let Some(uid) = uid {
        assert_eq!(found, vec![uid]);
    }
    let m = a
        .fetch_by_uid("Archive", FetchKind::Heavy, &UidSet::single(found[0]))
        .unwrap()
        .messages;
    assert_eq!(m[0].flags, MessageFlags::SEEN);
    assert_eq!(
        m[0].header.as_ref().unwrap().subject.as_deref(),
        Some("appended")
    );

    let err = a
        .copy_uids("INBOX", &UidSet::single(1), "Nope")
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Copy);
    let err = a.append("Nope", &raw, MessageFlags::empty()).unwrap_err();
    assert_eq!(err.code, ErrorCode::NonExistantFolder);

    a.store_flags(
        "INBOX",
        &UidSet::single(6),
        StoreOp::Add,
        MessageFlags::DELETED,
    )
    .unwrap();
    a.expunge("INBOX").unwrap();
    assert_eq!(a.status("INBOX").unwrap().message_count, INBOX_COUNT - 2);

    a.create_folder("Temp/Sub").unwrap();
    let folders = a.list_folders().unwrap();
    assert!(find(&folders, "Temp/Sub").is_some(), "{folders:?}");
    a.rename_folder("Temp", "Temp2").unwrap();
    let folders = a.list_folders().unwrap();
    assert!(find(&folders, "Temp2/Sub").is_some(), "{folders:?}");
    assert!(find(&folders, "Temp").is_none());
    a.delete_folder("Temp2/Sub").unwrap();
    // Temp2 was only implied by its child (\Noselect); servers differ on whether it
    // outlives it.
    let _ = a.delete_folder("Temp2");
    let folders = a.list_folders().unwrap();
    assert!(find(&folders, "Temp2").is_none());
    let err = a.create_folder("INBOX").unwrap_err();
    assert_eq!(err.code, ErrorCode::Create);
    let err = a.delete_folder("Nope").unwrap_err();
    assert_eq!(err.code, ErrorCode::Delete);
    let err = a.rename_folder("Nope", "Nope2").unwrap_err();
    assert_eq!(err.code, ErrorCode::Rename);
    // A folder name that needs quoting and modified UTF-7.
    let name = airpost_core::utf7::encode("Reçus 2026");
    a.create_folder(&name).unwrap();
    let folders = a.list_folders().unwrap();
    assert!(find(&folders, &name).is_some(), "{folders:?}");
    a.append(&name, &raw_message(2, "x"), MessageFlags::empty())
        .unwrap();
    assert_eq!(a.status(&name).unwrap().message_count, 1);
}

/// probe_idle: changes reported to an idling session, with and without QRESYNC, and the
/// cross-thread interrupt.
fn idle(s: &mut ServerProc) {
    let mut a = session(s);
    let mut b = session(s);
    a.select("INBOX").unwrap();
    assert!(a.supports_idle());
    let interrupt = IdleInterrupt::new();

    // New mail ends the IDLE.
    let appender = thread::spawn(move || {
        thread::sleep(Duration::from_millis(800));
        b.append("INBOX", &raw_message(10, "during idle"), MessageFlags::SEEN)
            .unwrap();
        b
    });
    let started = Instant::now();
    a.idle("INBOX", &interrupt).unwrap();
    let mut b = appender.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(15));
    let st = a.status("INBOX").unwrap();
    assert_eq!(st.message_count, INBOX_COUNT + 1);

    // An interrupt from another thread ends it without any change.
    let i2 = interrupt.clone();
    let started = Instant::now();
    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));
        i2.interrupt();
    });
    a.idle("INBOX", &interrupt).unwrap();
    t.join().unwrap();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(500) && took < Duration::from_secs(5),
        "{took:?}"
    );
    // An interrupt that arrives before IDLE ends the next one at once.
    interrupt.interrupt();
    let started = Instant::now();
    a.idle("INBOX", &interrupt).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!interrupt.take());

    // Expunges during IDLE: VANISHED under QRESYNC, collected for take_vanished.
    if a.is_qresync_enabled() {
        let _ = a.take_vanished("INBOX");
        let t = thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            expunge_via(&mut b, &UidSet::range(7, Some(8)));
            b
        });
        let mut vanished = UidSet::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        while vanished != UidSet::range(7, Some(8)) && Instant::now() < deadline {
            a.idle("INBOX", &interrupt).unwrap();
            vanished.extend(&a.take_vanished("INBOX"));
        }
        b = t.join().unwrap();
        assert_eq!(vanished, UidSet::range(7, Some(8)));
    }

    // A session without QRESYNC sees EXPUNGE, which wakes IDLE too.
    let mut c = plain_session(s);
    c.select("INBOX").unwrap();
    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));
        expunge_via(&mut b, &UidSet::single(9));
    });
    let started = Instant::now();
    c.idle("INBOX", &interrupt).unwrap();
    t.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(c.take_vanished("INBOX").is_empty());
    // Cyrus can wake IDLE with the \Deleted flag change and report the EXPUNGE only later.
    c.noop().unwrap();
    let left = c.fetch_by_uid("INBOX", FetchKind::Light, &all()).unwrap();
    assert!(!uids_of(&left.messages).contains(&9));
}

/// probe_stale_fetch: a selected session fetching after another expunged.
fn stale_fetch(s: &mut ServerProc) {
    let mut b = session(s);
    for (label, qresync) in [("plain", false), ("qresync", true)] {
        let mut c = if qresync {
            session(s)
        } else {
            plain_session(s)
        };
        c.select("INBOX").unwrap();
        let gone = if qresync {
            UidSet::range(8, Some(9))
        } else {
            UidSet::range(2, Some(3))
        };
        expunge_via(&mut b, &gone);
        // STATUS of the selected folder can answer from the stale view until a NOOP
        // (noop_before_selected_status).
        c.noop().unwrap();
        let st = c.status("INBOX").unwrap();
        let expected = if qresync {
            INBOX_COUNT - 4
        } else {
            INBOX_COUNT - 2
        };
        assert_eq!(st.message_count, expected, "{label}");
        c.fetch_by_uid("INBOX", FetchKind::Light, &all()).unwrap();
        let again = c.fetch_by_uid("INBOX", FetchKind::Light, &all()).unwrap();
        let uids = uids_of(&again.messages);
        assert!(gone.iter().all(|u| !uids.contains(&u)), "{label}: {uids:?}");
        if qresync && c.is_qresync_enabled() {
            assert_eq!(c.take_vanished("INBOX"), gone, "{label}");
        }
    }
}

/// A sync worker's pass over two connections: initial fetch, then CONDSTORE / QRESYNC
/// deltas, IDLE and reconnects.
fn scripted_session(s: &mut ServerProc) {
    let mut fg = session(s);
    let mut other = session(s);
    let folders = fg.list_folders().unwrap();
    assert!(folders.iter().any(|f| f.path == "INBOX"));
    let st = fg.status("INBOX").unwrap();
    let initial = fg
        .fetch_by_uid(
            "INBOX",
            FetchKind::Heavy,
            &UidSet::range(1, Some(st.uid_next - 1)),
        )
        .unwrap();
    assert_eq!(initial.messages.len() as u32, st.message_count);
    let hms = st.highest_modseq;

    // Remote changes: new mail, a flag, an expunge.
    let new_uid = other
        .append("INBOX", &raw_message(20, "scripted"), MessageFlags::empty())
        .unwrap();
    other
        .store_flags(
            "INBOX",
            &UidSet::single(1),
            StoreOp::Add,
            MessageFlags::FLAGGED,
        )
        .unwrap();
    expunge_via(&mut other, &UidSet::single(12));

    if fg.capabilities().has("CONDSTORE") {
        fg.noop().unwrap();
        let top = fg.status("INBOX").unwrap().uid_next - 1;
        let r = fg
            .sync_by_uid("INBOX", FetchKind::Heavy, &UidSet::range(1, Some(top)), hms)
            .unwrap();
        let uids = uids_of(&r.modified_or_added);
        assert!(uids.contains(&1), "{uids:?}");
        if let Some(n) = new_uid {
            assert!(uids.contains(&n));
            let m = r.modified_or_added.iter().find(|m| m.uid == n).unwrap();
            assert_eq!(
                m.header.as_ref().unwrap().message_id.as_deref(),
                Some("rust-20@example.test")
            );
        }
        if fg.is_qresync_enabled() {
            let mut v = r.vanished.clone();
            v.extend(&fg.take_vanished("INBOX"));
            assert!(v.contains(12), "{v:?}");
        }
    }

    // CONDSTORE without QRESYNC: no VANISHED, the expunge shows as a missing UID.
    let mut cs = plain_session(s);
    cs.login_if_needed().unwrap();
    assert!(!cs.is_qresync_enabled());
    if cs.capabilities().has("CONDSTORE") {
        let r = cs
            .sync_by_uid("INBOX", FetchKind::Light, &all(), hms)
            .unwrap();
        assert!(r.vanished.is_empty());
        assert!(uids_of(&r.modified_or_added).contains(&1));
    }
    let present = cs.fetch_by_uid("INBOX", FetchKind::Light, &all()).unwrap();
    assert!(!uids_of(&present.messages).contains(&12));

    // IDLE until the other connection files new mail, then fetch it.
    fg.noop().unwrap();
    let next = fg.status("INBOX").unwrap().uid_next;
    fg.select("INBOX").unwrap();
    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(600));
        other
            .append(
                "INBOX",
                &raw_message(21, "idle wake"),
                MessageFlags::empty(),
            )
            .unwrap();
        other
    });
    fg.idle("INBOX", &IdleInterrupt::new()).unwrap();
    let mut other = t.join().unwrap();
    let r = fg
        .fetch_by_uid("INBOX", FetchKind::Heavy, &UidSet::range(next, None))
        .unwrap();
    assert_eq!(r.messages.len(), 1);
    assert_eq!(
        r.messages[0].header.as_ref().unwrap().subject.as_deref(),
        Some("idle wake")
    );

    // A dropped connection is a Connection error; the next call reconnects.
    fg.disconnect();
    assert!(fg.is_disconnected());
    fg.noop().unwrap();
    assert!(
        fg.is_disconnected(),
        "NOOP on a session that was never set up does nothing"
    );
    fg.select("INBOX").unwrap();
    assert!(!fg.is_disconnected());
    other.disconnect();
}

fn has_keyword(m: &airpost_imap::ImapMessage, k: &str) -> bool {
    m.keywords.iter().any(|x| x.eq_ignore_ascii_case(k))
}

fn keywords_of(s: &mut Session, path: &str, uid: u32) -> airpost_imap::ImapMessage {
    let mut r = s
        .fetch_by_uid(path, FetchKind::Light, &UidSet::single(uid))
        .unwrap()
        .messages;
    assert_eq!(r.len(), 1, "{path} {uid}");
    r.remove(0)
}

/// probe_keywords: keyword STORE, what another session sees, the modseq bump, the FLAGS list
/// at SELECT, and COPY keeping keywords.
fn keywords(s: &mut ServerProc) {
    let mut a = session(s);
    let mut b = session(s);
    let info = a.select("INBOX").unwrap();
    assert!(info.allows_new_keywords(), "{info:?}");
    assert!(info
        .permanent_flags
        .as_ref()
        .unwrap()
        .iter()
        .any(|f| f == "\\*"));
    assert_eq!(a.allows_new_keywords("INBOX"), Some(true));
    assert_eq!(a.allows_new_keywords("Archive"), None);
    let hms = info.highest_modseq;

    let kws = ["work".to_owned(), "$label1".to_owned()];
    a.store_keywords("INBOX", &UidSet::range(1, Some(2)), StoreOp::Add, &kws)
        .unwrap();
    let m = keywords_of(&mut b, "INBOX", 1);
    assert!(
        has_keyword(&m, "work") && has_keyword(&m, "$label1"),
        "{m:?}"
    );
    assert!(
        m.flags.contains(MessageFlags::SEEN),
        "system flags untouched"
    );
    if a.is_qresync_enabled() {
        let r = b
            .sync_by_uid("INBOX", FetchKind::Light, &all(), hms)
            .unwrap();
        assert_eq!(uids_of(&r.modified_or_added), vec![1, 2]);
    }
    let info = b.select("INBOX").unwrap();
    assert!(
        info.flags.iter().any(|f| f.eq_ignore_ascii_case("work")),
        "{info:?}"
    );

    a.store_keywords(
        "INBOX",
        &UidSet::single(1),
        StoreOp::Remove,
        &["WORK".to_owned()],
    )
    .unwrap();
    let m = keywords_of(&mut b, "INBOX", 1);
    assert!(
        !has_keyword(&m, "work") && has_keyword(&m, "$label1"),
        "{m:?}"
    );
    assert!(m.flags.contains(MessageFlags::SEEN));

    a.copy_uids("INBOX", &UidSet::single(2), "Archive").unwrap();
    let archive = a.fetch_by_uid("Archive", FetchKind::Light, &all()).unwrap();
    let copied = archive.messages.last().unwrap();
    assert!(
        has_keyword(copied, "work") && has_keyword(copied, "$label1"),
        "{copied:?}"
    );

    let err = a
        .store_keywords("INBOX", &UidSet::single(1), StoreOp::Set, &kws)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Store);
    let err = a
        .store_keywords(
            "INBOX",
            &UidSet::single(1),
            StoreOp::Add,
            &["two words".to_owned()],
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Store);
}

macro_rules! server_tests {
    ($kind:ident, $profile:expr) => {
        mod $kind {
            use super::*;

            fn run(probe: fn(&mut ServerProc)) {
                let mut s = ServerProc::start(stringify!($kind), $profile, true);
                probe(&mut s);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn session_setup() {
                run(super::session_setup);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn list_status() {
                run(super::list_status);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn select_fetch() {
                run(super::select_fetch);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn store() {
                run(super::store);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn keywords() {
                run(super::keywords);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn bulk_store_modseq() {
                run(super::bulk_store_modseq);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn changedsince() {
                run(super::changedsince);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn copy_move_append() {
                run(super::copy_move_append);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn idle() {
                run(super::idle);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn stale_fetch() {
                run(super::stale_fetch);
            }

            #[test]
            #[ignore = "needs python3 and the harness servers"]
            fn scripted_session() {
                run(super::scripted_session);
            }
        }
    };
}

server_tests!(fake, "dovecot");
server_tests!(dovecot, "qresync");
server_tests!(cyrus, "fastmail");

// -- Provider personalities and transports (fake server) -----------------------------------

mod personalities {
    use super::*;

    fn fake(personality: &str) -> ServerProc {
        ServerProc::start("fake", personality, true)
    }

    #[test]
    #[ignore = "needs python3"]
    fn gmail() {
        let s = fake("gmail");
        let (mut a, log) = logged(&s, config(&s));
        let folders = a.list_folders().unwrap();
        assert_eq!(a.profile().provider, Provider::Gmail);
        assert!(!a.capabilities().has("XLIST"), "XLIST is off for Gmail");
        assert!(log.contents().contains(" LIST \"\" \"*\""));
        let all_mail = folders
            .iter()
            .find(|f| f.flags.contains(FolderFlags::ALL))
            .expect("All Mail")
            .path
            .clone();
        let r = a.fetch_by_uid(&all_mail, FetchKind::Heavy, &all()).unwrap();
        assert!(!r.messages.is_empty());
        for m in &r.messages {
            assert!(m.gmail_thread_id.is_some() && m.gmail_message_id.is_some());
        }
        let first = r.messages[0].uid;
        a.store_labels(
            &all_mail,
            &UidSet::single(first),
            StoreOp::Add,
            &["Work Stuff".to_owned(), "\\Starred".to_owned()],
        )
        .unwrap();
        let m = a
            .fetch_by_uid(&all_mail, FetchKind::Light, &UidSet::single(first))
            .unwrap()
            .messages;
        assert!(
            m[0].gmail_labels.iter().any(|l| l == "Work Stuff"),
            "{:?}",
            m[0].gmail_labels
        );
        assert!(log
            .contents()
            .contains("+X-GM-LABELS.SILENT (\"Work Stuff\" \"\\\\Starred\")"));
    }

    #[test]
    #[ignore = "needs python3"]
    fn coremail_tags_and_id() {
        let s = fake("netease-coremail");
        let (mut a, log) = logged(&s, config(&s));
        a.select("INBOX").unwrap();
        assert_eq!(a.profile().provider, Provider::Coremail);
        let t = log.contents();
        // The greeting carried the capabilities, so the first command is LOGIN (not logged).
        assert!(t.contains("C1 OK"), "{t}");
        assert!(!t.contains("\n1 "), "{t}");
        let id = t
            .find(" ID (\"name\" \"Airpost\" \"version\" \"test\")")
            .expect("ID sent");
        let select = t.find(" SELECT INBOX").unwrap();
        assert!(id < select);
        // XLIST is advertised and used.
        a.list_folders().unwrap();
        assert!(log.contents().contains(" XLIST \"\" \"*\""));
    }

    #[test]
    #[ignore = "needs python3"]
    fn yahoo_message_limit() {
        let mut s = ServerProc::start("fake", "yahoo-messagelimit", false);
        let raws: Vec<String> = (0..60).map(|i| b64(&raw_message(i, "m"))).collect();
        s.call("populate", json!(["INBOX", raws]));
        let mut a = session(&s);
        a.login_if_needed().unwrap();
        assert_eq!(a.message_limit(), Some(50));
        assert_eq!(a.profile().provider, Provider::Yahoo);
        assert!(!a.profile().trust_copyuid_pairing);
        let r = a.fetch_by_uid("INBOX", FetchKind::Light, &all()).unwrap();
        assert!(r.hit_message_limit);
        assert_eq!(r.messages.len(), 50);
        let r = a
            .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::range(1, Some(10)))
            .unwrap();
        assert!(!r.hit_message_limit);
    }

    #[test]
    #[ignore = "needs python3"]
    fn courier() {
        let s = fake("courier");
        let mut a = session(&s);
        a.login_if_needed().unwrap();
        assert_eq!(a.profile().provider, Provider::Courier);
        assert!(a.supports_idle());
        assert!(a.profile().reselect_after_append);
        a.list_folders().unwrap();
    }

    #[test]
    #[ignore = "needs python3"]
    fn hmailserver_and_proton() {
        let s = fake("hmailserver");
        let mut a = session(&s);
        a.status("INBOX").unwrap();
        assert!(a.profile().status_uidnext_unreliable);
        let s = fake("proton-bridge");
        let mut a = session(&s);
        a.list_folders().unwrap();
        assert_eq!(a.profile().provider, Provider::ProtonBridge);
    }

    #[test]
    #[ignore = "needs python3"]
    fn no_keywords_without_star() {
        for p in ["outlook", "yahoo", "proton-bridge"] {
            let s = fake(p);
            let mut a = session(&s);
            let info = a.select("INBOX").unwrap();
            assert!(!info.allows_new_keywords(), "{p}: {info:?}");
            assert_eq!(a.allows_new_keywords("INBOX"), Some(false));
        }
        let s = fake("keywords-in-flags");
        let mut a = session(&s);
        let info = a.select("INBOX").unwrap();
        assert!(!info
            .permanent_flags
            .as_ref()
            .unwrap()
            .iter()
            .any(|f| f == "\\*"));
        assert!(info.allows_new_keywords(), "\\* in FLAGS counts: {info:?}");
    }

    #[test]
    #[ignore = "needs python3"]
    fn keyword_limit_and_drop() {
        let mut s = fake("dovecot");
        s.call("set_keyword_limit", json!(["INBOX", 2]));
        let mut a = session(&s);
        let one = |k: &str| vec![k.to_owned()];
        a.store_keywords("INBOX", &UidSet::single(1), StoreOp::Add, &one("a"))
            .unwrap();
        a.store_keywords("INBOX", &UidSet::single(2), StoreOp::Add, &one("b"))
            .unwrap();
        // A keyword already defined still fits.
        a.store_keywords("INBOX", &UidSet::single(3), StoreOp::Add, &one("A"))
            .unwrap();
        let err = a
            .store_keywords("INBOX", &UidSet::single(3), StoreOp::Add, &one("c"))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Store);
        assert!(err.detail.starts_with("[LIMIT]"), "{err:?}");
        assert!(!a.select("INBOX").unwrap().allows_new_keywords());
        assert_eq!(a.allows_new_keywords("INBOX"), Some(false));

        let mut s = fake("dovecot");
        s.call("set_drop_keywords", json!([true]));
        let mut a = session(&s);
        a.store_keywords("INBOX", &UidSet::single(1), StoreOp::Add, &one("work"))
            .unwrap();
        let m = a
            .fetch_by_uid("INBOX", FetchKind::Light, &UidSet::single(1))
            .unwrap()
            .messages;
        assert!(m[0].keywords.is_empty(), "{m:?}");
    }

    #[test]
    #[ignore = "needs python3"]
    fn inbox_only_listing() {
        let s = fake("inbox-only");
        let mut a = session(&s);
        let folders = a.list_folders().unwrap();
        assert!(folders.iter().any(|f| f.path == "INBOX"), "{folders:?}");
    }

    #[test]
    #[ignore = "needs python3"]
    fn xoauth2() {
        let mut s = fake("gmail");
        s.call("oauth_tokens", json!([["good-token"]]));
        let mut c = config(&s);
        c.auth = ImapAuth::XOAuth2("good-token".into());
        let log = Arc::new(AccumulatingLogger::default());
        c.logger = Some(log.clone());
        let mut a = Session::new(c.clone());
        a.select("INBOX").unwrap();
        assert!(
            !log.contents().contains("AUTHENTICATE"),
            "AUTH lines are private"
        );
        c.auth = ImapAuth::XOAuth2("bad-token".into());
        let err = Session::new(c).login_if_needed().unwrap_err();
        assert_eq!(err.code, ErrorCode::Authentication);
    }

    fn trusting(s: &ServerProc, mut c: ImapConfig) -> ImapConfig {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::CertificateDer;
        let cert = CertificateDer::from_pem_file(s.cert.as_ref().unwrap()).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        c.tls_config = Some(Arc::new(tls));
        c.host = "localhost".into();
        c
    }

    #[test]
    #[ignore = "needs python3 and openssl"]
    fn implicit_tls() {
        let s = ServerProc::start_with("fake", "dovecot", true, "tls");
        let mut c = config(&s);
        c.security = Security::Tls;
        let mut a = Session::new(trusting(&s, c.clone()));
        assert_eq!(a.select("INBOX").unwrap().exists, INBOX_COUNT);
        a.idle("INBOX", &{
            let i = IdleInterrupt::new();
            i.interrupt();
            i
        })
        .unwrap();

        // Untrusted: a certificate error, not a TLS advice case.
        c.host = "localhost".into();
        let mut untrusted = Session::new(c);
        let err = untrusted.login_if_needed().unwrap_err();
        assert_eq!(err.code, ErrorCode::Certificate, "{err}");
        assert_eq!(untrusted.last_tls_error(), None);
        assert!(untrusted.is_disconnected());
    }

    #[test]
    #[ignore = "needs python3 and openssl"]
    fn starttls() {
        let s = ServerProc::start_with("fake", "dovecot", true, "starttls");
        let mut c = config(&s);
        c.security = Security::StartTls;
        let log = Arc::new(AccumulatingLogger::default());
        c.logger = Some(log.clone());
        let mut a = Session::new(trusting(&s, c));
        assert_eq!(a.select("INBOX").unwrap().exists, INBOX_COUNT);
        let t = log.contents();
        let starttls = t.find("1 STARTTLS").expect("STARTTLS sent");
        let cap = t.rfind(" CAPABILITY\r\n").unwrap();
        assert!(
            cap > starttls,
            "capabilities are fetched again after STARTTLS"
        );

        // Implicit TLS against a plain server is a connection failure with the reason kept.
        let mut c = config(&s);
        c.security = Security::Tls;
        let mut wrong = Session::new(trusting(&s, c));
        let err = wrong.login_if_needed().unwrap_err();
        assert_eq!(err.code, ErrorCode::Connection, "{err}");
    }

    #[test]
    #[ignore = "needs python3"]
    fn connection_drop_and_reconnect() {
        let mut s = fake("dovecot");
        let mut a = session(&s);
        a.select("INBOX").unwrap();
        s.call("drop_connections", json!([]));
        thread::sleep(Duration::from_millis(200));
        let err = a.noop().unwrap_err();
        assert_eq!(err.code, ErrorCode::Connection, "{err}");
        assert!(a.is_disconnected());
        assert_eq!(a.current_folder(), None);
        // The next call reconnects and logs in again.
        a.select("INBOX").unwrap();
        assert!(!a.is_disconnected());
    }
}

/// Cyrus's Debian layout: folders under `INBOX.` with `.` as the separator.
#[test]
#[ignore = "needs Docker"]
fn cyrus_namespace_prefix() {
    let s = ServerProc::start("cyrus", "default-ns", true);
    let mut a = session(&s);
    let folders = a.list_folders().unwrap();
    let ns = a.default_namespace().unwrap().clone();
    assert_eq!(ns.prefix, "INBOX.");
    assert_eq!(ns.delimiter, Some('.'));
    assert!(find(&folders, "INBOX").is_some(), "{folders:?}");
    assert!(find(&folders, "INBOX.Archive").is_some(), "{folders:?}");
    let st = a.status("INBOX.Archive").unwrap();
    assert_eq!(st.message_count, 2);
    let path = ns.path_for_components(&["Airpost", "Snoozed"]);
    assert_eq!(path, "INBOX.Airpost.Snoozed");
    a.create_folder(&path).unwrap();
    assert!(find(&a.list_folders().unwrap(), &path).is_some());
}
