// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Session behaviour against a scripted in-process server: the malformed and unusual
//! responses no harness server sends (broken envelopes, unparseable lines, FETCH without
//! ENVELOPE, login error texts, stream failures). Runs without Python or Docker.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tern_core::protolog::AccumulatingLogger;
use tern_core::{ErrorCode, MessageFlags, Security, UidSet};
use tern_imap::{FetchKind, IdleInterrupt, ImapAuth, ImapConfig, ImapSession, Session};

const CAPS: &str = "IMAP4rev1 IDLE NAMESPACE UIDPLUS CONDSTORE QRESYNC ENABLE";

/// A rule: when a command contains `pattern`, answer with `reply` (lines joined by CRLF,
/// `{tag}` replaced). The first matching rule wins; `once` rules are used up.
struct Rule {
    pattern: String,
    reply: String,
    once: bool,
}

#[derive(Default)]
struct Script {
    rules: Vec<Rule>,
    /// Every command line received, tags stripped.
    seen: Vec<String>,
    /// Every synchronising literal received, as sent.
    literals: Vec<Vec<u8>>,
}

struct Server {
    port: u16,
    script: Arc<Mutex<Script>>,
}

fn rule(pattern: &str, reply: &str) -> Rule {
    Rule {
        pattern: pattern.into(),
        reply: reply.into(),
        once: false,
    }
}

fn once(pattern: &str, reply: &str) -> Rule {
    Rule {
        once: true,
        ..rule(pattern, reply)
    }
}

fn base_rules() -> Vec<Rule> {
    vec![
        rule("LOGIN", &format!("{{tag}} OK [CAPABILITY {CAPS}] Logged in")),
        rule("ENABLE", "* ENABLED QRESYNC\r\n{tag} OK Enabled"),
        rule("NAMESPACE", "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n{tag} OK done"),
        rule(
            "SELECT",
            "* 3 EXISTS\r\n* OK [UIDVALIDITY 7] x\r\n* OK [UIDNEXT 4] x\r\n* OK [HIGHESTMODSEQ 99] x\r\n{tag} OK [READ-WRITE] selected",
        ),
        rule("NOOP", "{tag} OK noop"),
        rule("LOGOUT", "* BYE\r\n{tag} OK bye"),
    ]
}

impl Server {
    fn start(greeting: &str, mut rules: Vec<Rule>) -> Server {
        rules.extend(base_rules());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let script = Arc::new(Mutex::new(Script {
            rules,
            ..Script::default()
        }));
        let s = script.clone();
        let greeting = greeting.to_owned();
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { return };
                let s = s.clone();
                let greeting = greeting.clone();
                thread::spawn(move || serve(conn, &greeting, &s));
            }
        });
        Server { port, script }
    }

    fn seen(&self) -> Vec<String> {
        self.script.lock().unwrap().seen.clone()
    }

    fn literals(&self) -> Vec<Vec<u8>> {
        self.script.lock().unwrap().literals.clone()
    }

    fn config(&self) -> ImapConfig {
        let mut c = ImapConfig::new(
            "127.0.0.1",
            self.port,
            Security::Clear,
            "user",
            ImapAuth::Password("secret pass".into()),
        );
        c.timeout = Duration::from_secs(5);
        c
    }

    fn session(&self) -> Session {
        Session::new(self.config())
    }
}

fn serve(conn: TcpStream, greeting: &str, script: &Mutex<Script>) {
    let mut w = conn.try_clone().unwrap();
    let mut r = BufReader::new(conn);
    let _ = write!(w, "* OK {greeting}\r\n");
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let trimmed = line.trim_end().to_owned();
        let (tag, cmd) = trimmed.split_once(' ').unwrap_or((&trimmed, ""));
        // A synchronising literal at the end of the line: continue, read it and its CRLF.
        let literal_len = cmd
            .strip_suffix('}')
            .and_then(|c| c.rsplit_once('{'))
            .and_then(|(_, n)| n.parse::<usize>().ok());
        if let Some(n) = literal_len {
            let _ = write!(w, "+ go ahead\r\n");
            let mut buf = vec![0; n + 2];
            if r.read_exact(&mut buf).is_err() {
                return;
            }
            buf.truncate(n);
            script.lock().unwrap().literals.push(buf);
        }
        let reply = {
            let mut s = script.lock().unwrap();
            s.seen.push(cmd.to_owned());
            let idx = s.rules.iter().position(|r| cmd.contains(&r.pattern));
            match idx {
                Some(i) if s.rules[i].once => Some(s.rules.remove(i).reply),
                Some(i) => Some(s.rules[i].reply.clone()),
                None => None,
            }
        };
        let reply = reply.unwrap_or_else(|| "{tag} BAD unknown command".into());
        if reply == "<close>" {
            return;
        }
        if cmd == "IDLE" {
            // `reply` is what to send before and after the continuation, split by `<+>`.
            let (before, after) = reply.split_once("<+>").unwrap_or(("", &reply));
            if !before.is_empty() {
                let _ = write!(w, "{before}\r\n");
            }
            let _ = write!(w, "+ idling\r\n");
            if !after.trim().is_empty() {
                thread::sleep(Duration::from_millis(100));
                let _ = write!(w, "{}\r\n", after.trim());
            }
            line.clear();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            script.lock().unwrap().seen.push(line.trim_end().to_owned());
            let _ = write!(w, "{tag} OK idle done\r\n");
            continue;
        }
        let _ = write!(w, "{}\r\n", reply.replace("{tag}", tag));
    }
}

const GREETING: &str = "[CAPABILITY IMAP4rev1 LOGIN] test ready";

#[test]
fn login_sequence_and_commands() {
    let srv = Server::start(GREETING, vec![]);
    let mut c = srv.config();
    let log = Arc::new(AccumulatingLogger::default());
    c.logger = Some(log.clone());
    let mut s = Session::new(c);
    let info = s.select("INBOX").unwrap();
    assert_eq!(
        (
            info.exists,
            info.uid_validity,
            info.uid_next,
            info.highest_modseq
        ),
        (3, 7, 4, 99)
    );
    assert!(s.is_qresync_enabled());
    assert_eq!(
        srv.seen(),
        vec![
            "LOGIN user \"secret pass\"",
            "ENABLE QRESYNC",
            "NAMESPACE",
            "SELECT INBOX"
        ]
    );
    let t = log.contents();
    assert!(!t.contains("secret"), "{t}");
    assert!(t.contains("2 ENABLE QRESYNC\r\n"), "{t}");
}

#[test]
fn login_errors_map_to_codes() {
    for (text, code) in [
        (
            "[ALERT] Application-specific password required",
            ErrorCode::GmailApplicationSpecificPasswordRequired,
        ),
        (
            "[AUTHENTICATIONFAILED] Invalid credentials",
            ErrorCode::Authentication,
        ),
        (
            "Your account is not enabled for IMAP use.",
            ErrorCode::GmailIMAPNotEnabled,
        ),
        (
            "Too many simultaneous connections. (Failure)",
            ErrorCode::GmailTooManySimultaneousConnections,
        ),
    ] {
        let srv = Server::start(GREETING, vec![rule("LOGIN", &format!("{{tag}} NO {text}"))]);
        let err = srv.session().login_if_needed().unwrap_err();
        assert_eq!(err.code, code, "{text}");
    }
}

#[test]
fn capability_when_not_in_greeting() {
    let srv = Server::start(
        "plain server ready",
        vec![
            rule("CAPABILITY", "* CAPABILITY IMAP4rev1 IDLE\r\n{tag} OK done"),
            rule("LOGIN", "{tag} OK Logged in"),
            rule(
                "LIST \"\" \"\"",
                "* LIST (\\Noselect) \".\" \"\"\r\n{tag} OK done",
            ),
        ],
    );
    let mut s = srv.session();
    s.login_if_needed().unwrap();
    assert_eq!(
        srv.seen(),
        vec![
            "CAPABILITY",
            "LOGIN user \"secret pass\"",
            "CAPABILITY",
            "LIST \"\" \"\""
        ]
    );
    assert!(!s.is_qresync_enabled());
    let ns = s.default_namespace().unwrap();
    assert_eq!((ns.prefix.as_str(), ns.delimiter), ("", Some('.')));
}

#[test]
fn malformed_responses_are_skipped_or_recovered() {
    let srv = Server::start(
        GREETING,
        vec![rule(
            "UID FETCH 1:3",
            concat!(
                "* 1 FETCH (UID 1 FLAGS (\\Seen) ENVELOPE (\"Mon, 5 Jan 2026 09:00:00 +0000\" \"say \"hi\" now\" ((\"Jo \"Q\" Doe\" NIL \"jo\" \"d.test\")) (NIL) NIL ((NIL NIL \"a\" \"b.test\")) NIL NIL NIL \"<one@d.test>\") INTERNALDATE \"05-Jan-2026 09:00:00 +0000\" BODY[HEADER.FIELDS (References)] {27}\r\nReferences: <zero@d.test>\r\n)\r\n",
                "* 2 FETCH garbage that is not a list\r\n",
                "* 2 FETCH (UID 2 FLAGS () ENVELOPE (NIL NIL NIL NIL NIL NIL NIL NIL NIL NIL) INTERNALDATE \"bad date\" BODY[HEADER.FIELDS (References)] NIL)\r\n",
                "* 3 FETCH (UID 3 FLAGS ())\r\n",
                "{tag} OK done"
            ),
        )],
    );
    let mut s = srv.session();
    let r = s
        .fetch_by_uid("INBOX", FetchKind::Heavy, &UidSet::range(1, Some(3)))
        .unwrap();
    // 3 lacks an ENVELOPE and is dropped; the unparseable line costs nothing else.
    assert_eq!(
        r.messages.iter().map(|m| m.uid).collect::<Vec<_>>(),
        vec![1, 2]
    );
    let h = r.messages[0].header.as_ref().unwrap();
    assert_eq!(h.subject.as_deref(), Some("say \"hi\" now"));
    assert_eq!(h.from[0].name.as_deref(), Some("Jo \"Q\" Doe"));
    assert_eq!(h.from[0].email, "jo@d.test");
    assert!(h.sender.is_empty());
    assert_eq!(h.to[0].email, "a@b.test");
    assert_eq!(h.message_id.as_deref(), Some("one@d.test"));
    assert_eq!(h.references, vec!["zero@d.test"]);
    assert_eq!(r.messages[0].internal_date, Some(1767603600));
    assert_eq!(r.messages[1].internal_date, None);
    assert!(!s.is_disconnected());
}

#[test]
fn full_headers_fallback_is_sticky() {
    let srv = Server::start(
        GREETING,
        vec![
            once(
                "ENVELOPE",
                "* 1 FETCH (UID 1 FLAGS () INTERNALDATE \"05-Jan-2026 09:00:00 +0000\")\r\n{tag} OK done",
            ),
            rule(
                "BODY.PEEK[HEADER.FIELDS (Date Subject From Sender Reply-To To Cc Message-ID References In-Reply-To)]",
                "* 1 FETCH (UID 1 FLAGS () BODY[HEADER.FIELDS (DATE SUBJECT FROM SENDER REPLY-TO TO CC MESSAGE-ID REFERENCES IN-REPLY-TO)] {73}\r\nSubject: mbox mail\r\nFrom: Ann <ann@a.test>\r\nMessage-ID: <mbox@a.test>\r\n\r\n INTERNALDATE \"05-Jan-2026 09:00:00 +0000\")\r\n{tag} OK done",
            ),
        ],
    );
    let mut s = srv.session();
    let one = UidSet::single(1);
    let r = s.fetch_by_uid("INBOX", FetchKind::Heavy, &one).unwrap();
    assert_eq!(r.messages.len(), 1);
    let h = r.messages[0].header.as_ref().unwrap();
    assert_eq!(h.subject.as_deref(), Some("mbox mail"));
    assert_eq!(h.message_id.as_deref(), Some("mbox@a.test"));
    // From now on the session asks for the header fields directly.
    s.fetch_by_uid("INBOX", FetchKind::Heavy, &one).unwrap();
    let fetches: Vec<String> = srv
        .seen()
        .into_iter()
        .filter(|c| c.starts_with("UID FETCH"))
        .collect();
    assert_eq!(fetches.len(), 3, "{fetches:?}");
    assert!(fetches[0].contains("ENVELOPE"));
    assert!(!fetches[1].contains("ENVELOPE") && !fetches[2].contains("ENVELOPE"));
    assert_eq!(
        fetches[0],
        "UID FETCH 1 (UID FLAGS ENVELOPE INTERNALDATE BODY.PEEK[HEADER.FIELDS (References)])"
    );
}

#[test]
fn full_headers_retry_only_for_missing_envelopes() {
    let srv = Server::start(
        GREETING,
        vec![
            // A heavy `n:*` fetch past the end gets the highest UID back, outside the filter.
            rule("UID FETCH 5:*", "* 3 FETCH (UID 3 FLAGS ())\r\n{tag} OK done"),
            // An envelope that does not tokenise as a list is a bad message, not a server
            // without ENVELOPE.
            rule(
                "UID FETCH 1 ",
                "* 1 FETCH (UID 1 FLAGS () ENVELOPE garbage INTERNALDATE \"05-Jan-2026 09:00:00 +0000\")\r\n{tag} OK done",
            ),
        ],
    );
    let mut s = srv.session();
    let r = s
        .fetch_by_uid("INBOX", FetchKind::Heavy, &UidSet::range(5, None))
        .unwrap();
    assert!(r.messages.is_empty());
    let r = s
        .fetch_by_uid("INBOX", FetchKind::Heavy, &UidSet::single(1))
        .unwrap();
    assert!(r.messages.is_empty());
    let fetches: Vec<String> = srv
        .seen()
        .into_iter()
        .filter(|c| c.starts_with("UID FETCH"))
        .collect();
    assert_eq!(fetches.len(), 2, "{fetches:?}");
    assert!(fetches.iter().all(|f| f.contains("ENVELOPE")));
}

#[test]
fn full_headers_retry_keeps_first_vanished() {
    let srv = Server::start(
        GREETING,
        vec![
            once(
                "ENVELOPE",
                "* VANISHED (EARLIER) 7:8\r\n* 1 FETCH (UID 1 FLAGS () INTERNALDATE \"05-Jan-2026 09:00:00 +0000\")\r\n{tag} OK [MESSAGELIMIT 1] done",
            ),
            rule(
                "HEADER.FIELDS (Date",
                "* 1 FETCH (UID 1 FLAGS () BODY[HEADER.FIELDS (DATE)] {20}\r\nSubject: mbox mail\r\n INTERNALDATE \"05-Jan-2026 09:00:00 +0000\")\r\n{tag} OK done",
            ),
        ],
    );
    let mut s = srv.session();
    let r = s
        .sync_by_uid("INBOX", FetchKind::Heavy, &UidSet::range(1, None), 50)
        .unwrap();
    assert_eq!(r.modified_or_added.len(), 1);
    assert_eq!(r.vanished.to_string(), "7:8");
    assert!(r.hit_message_limit);
}

#[test]
fn append_sends_crlf_literal() {
    let srv = Server::start(
        GREETING,
        vec![rule("APPEND", "{tag} OK [APPENDUID 7 42] appended")],
    );
    let mut s = srv.session();
    let uid = s
        .append(
            "Sent",
            b"Subject: a\nX: b\r\n\nbody\rend\n",
            MessageFlags::SEEN,
        )
        .unwrap();
    assert_eq!(uid, Some(42));
    let want = b"Subject: a\r\nX: b\r\n\r\nbody\r\nend\r\n";
    assert_eq!(srv.literals(), vec![want.to_vec()]);
    let append = srv
        .seen()
        .into_iter()
        .find(|c| c.starts_with("APPEND"))
        .unwrap();
    assert_eq!(append, format!("APPEND Sent (\\Seen) {{{}}}", want.len()));
}

#[test]
fn errors_and_disconnects() {
    let srv = Server::start(
        GREETING,
        vec![
            rule("UID STORE", "{tag} NO [CANNOT] read-only"),
            rule("UID FETCH 5 BODY.PEEK[]", "{tag} WHAT is this"),
            rule("UID FETCH 6", "<close>"),
            rule("UID EXPUNGE", "{tag} BAD no UIDPLUS"),
        ],
    );
    let mut s = srv.session();
    let err = s
        .store_flags(
            "INBOX",
            &UidSet::single(1),
            tern_imap::StoreOp::Add,
            MessageFlags::SEEN,
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Store);
    assert_eq!(err.detail, "[CANNOT] read-only");
    assert!(!s.is_disconnected());
    let err = s.expunge_uids("INBOX", &UidSet::single(1)).unwrap_err();
    assert_eq!(err.code, ErrorCode::Expunge);

    let err = s.fetch_message_bytes("INBOX", 5).unwrap_err();
    assert_eq!(err.code, ErrorCode::Parse);
    assert!(s.is_disconnected());

    let err = s.fetch_message_bytes("INBOX", 6).unwrap_err();
    assert_eq!(err.code, ErrorCode::Connection);
    assert!(s.is_disconnected());
    assert_eq!(s.current_folder(), None);
}

#[test]
fn bodies_vanished_and_copyuid() {
    let srv = Server::start(
        GREETING,
        vec![
            rule(
                "UID FETCH 2 BODY.PEEK[]",
                "* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n* 2 FETCH (UID 2 BODY[] {5}\r\nhello)\r\n{tag} OK done",
            ),
            rule("UID FETCH 3 BODY.PEEK[]", "{tag} OK nothing"),
            rule("UID FETCH 4 BODY.PEEK[]", "* 4 FETCH (UID 4 BODY[] NIL)\r\n{tag} OK done"),
            rule(
                "UID MOVE",
                "* OK [COPYUID 9 3,1:2 20:22] moved\r\n* VANISHED 1:3\r\n{tag} OK done",
            ),
            rule(
                "CHANGEDSINCE",
                "* VANISHED (EARLIER) 5\r\n* 1 FETCH (UID 1 FLAGS () MODSEQ (100))\r\n* VANISHED 6\r\n{tag} OK [MESSAGELIMIT 1 1] done",
            ),
        ],
    );
    let mut s = srv.session();
    assert_eq!(
        s.fetch_message_bytes("INBOX", 2).unwrap(),
        Some(b"hello".to_vec())
    );
    assert_eq!(s.fetch_message_bytes("INBOX", 3).unwrap(), None);
    assert_eq!(s.fetch_message_bytes("INBOX", 4).unwrap(), Some(vec![]));

    let m = s
        .move_uids("INBOX", &UidSet::range(1, Some(3)), "Archive")
        .unwrap();
    assert_eq!(m.0, vec![(3, 20), (1, 21), (2, 22)]);
    assert_eq!(s.take_vanished("inbox"), UidSet::range(1, Some(3)));
    assert!(srv.seen().contains(&"UID MOVE 1:3 Archive".to_owned()));

    let r = s
        .sync_by_uid("INBOX", FetchKind::Light, &UidSet::range(1, None), 50)
        .unwrap();
    assert_eq!(r.vanished, UidSet::single(5));
    assert_eq!(r.modified_or_added[0].modseq, 100);
    assert!(r.hit_message_limit);
    assert_eq!(s.take_vanished("INBOX"), UidSet::single(6));
    assert!(srv
        .seen()
        .contains(&"UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 50 VANISHED)".to_owned()));
}

#[test]
fn list_inbox_fallbacks_and_literals() {
    let srv = Server::start(
        GREETING,
        vec![
            rule(
                "LIST \"\" \"*\"",
                "* LIST (\\HasNoChildren \\Inbox) \"/\" \"Posteingang\"\r\n* LIST () NIL {6}\r\nNo\"Dlm\r\n* LIST (\\Noselect \\HasChildren) \"/\" Parent\r\n{tag} OK done",
            ),
        ],
    );
    let mut s = srv.session();
    let folders = s.list_folders().unwrap();
    assert_eq!(folders[0].path, "INBOX");
    assert!(folders[0].flags.contains(tern_core::FolderFlags::INBOX));
    assert_eq!(folders[1].path, "No\"Dlm");
    assert_eq!(folders[1].delimiter, None);
    assert!(folders[2].flags.contains(tern_core::FolderFlags::NO_SELECT));

    let srv = Server::start(
        GREETING,
        vec![
            rule("LIST \"\" \"*\"", "* LIST () \"/\" Work\r\n{tag} OK done"),
            rule(
                "LIST \"\" \"INBOX\"",
                "* LIST () \"/\" inbox\r\n{tag} OK done",
            ),
        ],
    );
    let folders = srv.session().list_folders().unwrap();
    assert_eq!(
        folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        vec!["Work", "INBOX"]
    );
}

#[test]
fn idle_paths() {
    let srv = Server::start(
        GREETING,
        vec![
            once("IDLE", "* 4 EXISTS<+>"),
            once("IDLE", "<+>* 5 EXISTS"),
            rule("IDLE", "<+>"),
        ],
    );
    let mut s = srv.session();
    let i = IdleInterrupt::new();
    // EXISTS before the continuation: no wait at all.
    s.idle("INBOX", &i).unwrap();
    // EXISTS during IDLE.
    s.idle("INBOX", &i).unwrap();
    // Nothing from the server: the interrupt ends it.
    let i2 = i.clone();
    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        i2.interrupt();
    });
    s.idle("INBOX", &i).unwrap();
    t.join().unwrap();
    let dones = srv.seen().iter().filter(|c| *c == "DONE").count();
    assert_eq!(dones, 3);
}

#[test]
fn coremail_tags_and_unsolicited_capability() {
    let srv = Server::start(
        "Coremail System IMap Server Ready(163com[10.0.0.1])",
        vec![
            rule(
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 ID XLIST\r\n{tag} OK done",
            ),
            rule("LOGIN", "{tag} OK Logged in"),
            rule(
                "ID (",
                "* ID (\"name\" \"Coremail\")\r\n{tag} OK ID completed",
            ),
            rule(
                "LIST \"\" \"\"",
                "* LIST (\\Noselect) \"/\" \"\"\r\n{tag} OK done",
            ),
            rule(
                "XLIST",
                "* XLIST (\\Inbox) \"/\" \"INBOX\"\r\n{tag} OK done",
            ),
        ],
    );
    let mut c = srv.config();
    c.client_identity = Some(("Tern".into(), "1.0".into()));
    let log = Arc::new(AccumulatingLogger::default());
    c.logger = Some(log.clone());
    let mut s = Session::new(c);
    s.select("INBOX").unwrap();
    s.list_folders().unwrap();
    let t = log.contents();
    assert!(t.contains("C1 CAPABILITY\r\n"), "{t}");
    assert!(
        t.contains("C4 ID (\"name\" \"Tern\" \"version\" \"1.0\")\r\n"),
        "{t}"
    );
    assert!(t.contains(" XLIST \"\" \"*\"\r\n"), "{t}");
}

#[test]
fn session_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Session>();
}
