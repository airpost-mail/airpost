// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

use std::path::Path;

use serde_json::{json, Value};

use super::fake::Fake;
use super::{call, Ctx, Output, Session};
use crate::config::DesktopConfig;
use crate::policy::Policy;

const ME: &str = "me@x.test";
const ORIGINAL_HMID: &str = "<orig-1@y.test>";

fn fake() -> Fake {
    let folder = |id: &str, path: &str, role: &str| json!({"id": id, "aid": "a1", "path": path, "role": role, "__cls": "Folder"});
    let mut f = Fake {
        accounts: vec![Fake::account("a1", ME)],
        folders: vec![
            folder("f-in", "INBOX", "inbox"),
            folder("f-dr", "Drafts", "drafts"),
            folder("f-ar", "Archive", "archive"),
            folder("f-tr", "Trash", "trash"),
            folder("f-secret", "Secret", ""),
        ],
        ..Default::default()
    };
    f.threads.push(
        json!({"id": "t1", "aid": "a1", "subject": "Plans", "unread": 1, "starred": 0,
        "lmt": 2000, "lmrt": 1000, "folders": [{"id": "f-in", "path": "INBOX", "role": "inbox"},
        {"id": "f-dr", "path": "Drafts", "role": "drafts"}], "labels": [], "participants": []}),
    );
    f.messages.push(
        json!({"id": "m1", "aid": "a1", "threadId": "t1", "hMsgId": ORIGINAL_HMID,
        "subject": "Plans", "date": 1000, "draft": false, "unread": true,
        "from": [{"name": "Bob", "email": "bob@y.test"}],
        "to": [{"name": "Me", "email": ME}, {"name": "Carol", "email": "carol@y.test"}],
        "cc": [], "bcc": [], "replyTo": [], "files": [], "folders": {"f-in": 1}}),
    );
    // The user's own unsent reply, newer than the received message.
    f.messages.push(json!({"id": "d0", "aid": "a1", "threadId": "t1", "hMsgId": "draft-0@x.test",
        "subject": "Re: Plans", "date": 2000, "draft": true, "rthMsgId": ORIGINAL_HMID,
        "from": [{"name": "Me", "email": ME}], "to": [{"name": "Bob", "email": "bob@y.test"}],
        "cc": [], "bcc": [], "replyTo": [], "folders": {"f-dr": 4},
        "files": [{"id": "file-1", "filename": "plan.pdf", "contentType": "application/pdf", "size": 10}],
        "metadata": [{"pluginId": "open-tracking", "v": 1, "value": {"uid": "u", "open_count": 0, "open_data": []}}]}));
    f.bodies
        .insert("m1".into(), "<p>original from bob</p>".into());
    f.bodies
        .insert("d0".into(), "<p>my unsent draft</p>".into());
    f
}

fn policy(level: &str, accounts: Value) -> Policy {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("mcp.json"),
        json!({"accessLevel": level, "enabledAccounts": accounts}).to_string(),
    )
    .unwrap();
    Policy::load(tmp.path(), &DesktopConfig::default()).unwrap()
}

fn run(f: &mut Fake, p: &Policy, tool: &str, args: Value) -> Output {
    run_in(&Session::default(), f, p, tool, args)
}

/// One call in a session that outlives it, as an MCP connection does.
fn run_in(session: &Session, f: &mut Fake, p: &Policy, tool: &str, args: Value) -> Output {
    let desktop = DesktopConfig::from_value(json!({"accounts": [
        {"id": "a1", "name": "Me", "emailAddress": ME, "provider": "imap", "aliases": []}
    ]}));
    let mut ctx = Ctx {
        backend: f,
        policy: p,
        desktop: &desktop,
        config_dir: Path::new("/nonexistent"),
        session,
        no_engine: "no engine running".into(),
    };
    call(&mut ctx, tool, &args)
}

fn json_of(o: &Output) -> Value {
    assert!(!o.is_error, "{}", o.first_text());
    serde_json::from_str(o.first_text()).unwrap()
}

fn rw() -> Policy {
    policy("read-write", json!({}))
}

#[test]
fn reply_answers_the_received_message_not_the_users_draft() {
    let mut f = fake();
    let out = json_of(&run(
        &mut f,
        &rw(),
        "reply_to_thread",
        json!({"threadId": "t1", "body": "<p>Sounds good</p>", "type": "reply-all"}),
    ));
    let task = f.queued.last().unwrap();
    assert_eq!(task["__cls"], "SyncbackDraftTask");
    let draft = &task["draft"];
    let body = draft["body"].as_str().unwrap();
    assert!(body.starts_with("<p>Sounds good</p>"), "{body}");
    assert!(
        body.contains("original from bob") && body.contains("Bob &lt;bob@y.test&gt; wrote:"),
        "{body}"
    );
    assert!(!body.contains("my unsent draft"), "{body}");
    assert_eq!(draft["rthMsgId"], ORIGINAL_HMID);
    assert_eq!(draft["threadId"], "t1");
    assert_eq!(draft["subject"], "Re: Plans");
    assert_eq!(
        draft["to"],
        json!([{"name": "Bob", "email": "bob@y.test", "__cls": "Contact"}])
    );
    assert_eq!(
        draft["cc"],
        json!([{"name": "Carol", "email": "carol@y.test", "__cls": "Contact"}])
    );
    assert_eq!(draft["from"][0]["email"], ME);
    assert_eq!(out["repliedTo"]["id"], "m1");
    assert_eq!(
        out["existingDrafts"],
        json!([{"id": "d0", "headerMessageId": "draft-0@x.test",
        "subject": "Re: Plans", "date": "1970-01-01T00:33:20.000Z", "draft": true}])
    );
    assert!(out["note"].as_str().unwrap().contains("update_draft"));

    // Naming the draft as the target is refused.
    let o = run(
        &mut f,
        &rw(),
        "reply_to_thread",
        json!({"threadId": "t1", "body": "x", "messageId": "d0"}),
    );
    assert!(
        o.is_error && o.first_text().contains("unsent draft"),
        "{}",
        o.first_text()
    );
}

#[test]
fn update_draft_changes_content_and_keeps_identity() {
    let mut f = fake();
    let out = json_of(&run(
        &mut f,
        &rw(),
        "update_draft",
        json!({
            "headerMessageId": "draft-0@x.test",
            "body": "<p>revised</p>",
            "to": [{"email": "dan@y.test"}],
            "cc": [{"name": "Carol", "email": "carol@y.test"}]
        }),
    ));
    assert_eq!(
        (&out["id"], &out["headerMessageId"], &out["threadId"]),
        (&json!("d0"), &json!("draft-0@x.test"), &json!("t1"))
    );
    let draft = &f.queued.last().unwrap()["draft"];
    assert_eq!(draft["body"], "<p>revised</p>");
    assert_eq!(
        draft["to"],
        json!([{"name": "", "email": "dan@y.test", "__cls": "Contact"}])
    );
    assert_eq!(
        (&draft["id"], &draft["hMsgId"], &draft["rthMsgId"]),
        (
            &json!("d0"),
            &json!("draft-0@x.test"),
            &json!(ORIGINAL_HMID)
        )
    );
    assert_eq!(draft["file_ids"], json!(["file-1"]));
    assert_eq!(draft["metadata"][0]["pluginId"], "open-tracking");
    assert_eq!(
        (&draft["subject"], &draft["pristine"]),
        (&json!("Re: Plans"), &json!(false))
    );

    // The stored draft changed in place: still in the thread, still one draft.
    let thread = json_of(&run(&mut f, &rw(), "get_thread", json!({"threadId": "t1"})));
    let messages = thread["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|m| (&m["id"], &m["draft"]))
            .collect::<Vec<_>>(),
        [(&json!("m1"), &json!(false)), (&json!("d0"), &json!(true))]
    );
    assert_eq!(messages[1]["body"], "<p>revised</p>");
    assert_eq!(
        messages[1]["to"],
        json!([{"name": "", "email": "dan@y.test"}])
    );

    // A subject-only edit leaves the body alone.
    run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "subject": "New subject"}),
    );
    assert!(f.queued.last().unwrap()["draft"].get("body").is_none());

    let o = run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "m1", "subject": "x"}),
    );
    assert!(o.is_error && o.first_text().contains("not found"));
    let o = run(&mut f, &rw(), "update_draft", json!({"id": "d0"}));
    assert!(o.is_error && o.first_text().contains("Nothing to change"));
}

#[test]
fn drafts_being_sent_are_not_edited() {
    let mut f = fake();
    let future = chrono::Utc::now().timestamp() + 60;
    f.messages[1]["metadata"] = json!([{"pluginId": "send-later", "v": 1,
        "value": {"expiration": future, "isUndoSend": true, "actionKey": "send"}}]);
    let o = run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "body": "x"}),
    );
    assert!(
        o.is_error && o.first_text().contains("being sent"),
        "{}",
        o.first_text()
    );
    let o = run(&mut f, &rw(), "delete_draft", json!({"id": "d0"}));
    assert!(o.is_error);
    assert!(f.queued.is_empty());
}

#[test]
fn drafts_this_session_is_sending_are_not_edited_until_the_send_ends() {
    let mut f = fake();
    let session = Session::default();
    let send = policy("read-write-send", json!({}));
    json_of(&run_in(
        &session,
        &mut f,
        &send,
        "send_draft",
        json!({"headerMessageId": "draft-0@x.test"}),
    ));
    let task = f.queued.last().unwrap().clone();
    assert_eq!(task["__cls"], "SendDraftTask");
    let task_id = task["id"].as_str().unwrap().to_owned();
    for status in ["local", "remote"] {
        f.tasks.get_mut(&task_id).unwrap()["status"] = json!(status);
        for (tool, args) in [
            ("update_draft", json!({"id": "d0", "body": "x"})),
            ("delete_draft", json!({"id": "d0"})),
            ("send_draft", json!({"headerMessageId": "draft-0@x.test"})),
        ] {
            let o = run_in(&session, &mut f, &send, tool, args);
            assert!(
                o.is_error && o.first_text().contains("being sent"),
                "{tool} while {status}: {}",
                o.first_text()
            );
        }
    }
    assert_eq!(f.queued.len(), 1);
    // Another session didn't queue the send and doesn't know about it.
    assert!(
        !run(
            &mut f,
            &send,
            "update_draft",
            json!({"id": "d0", "body": "x"})
        )
        .is_error
    );

    f.tasks.get_mut(&task_id).unwrap()["status"] = json!("complete");
    json_of(&run_in(
        &session,
        &mut f,
        &send,
        "update_draft",
        json!({"id": "d0", "body": "y"}),
    ));
    json_of(&run_in(
        &session,
        &mut f,
        &send,
        "delete_draft",
        json!({"id": "d0"}),
    ));
}

#[test]
fn a_draft_synced_from_elsewhere_is_sent_as_stored_after_its_body_downloads() {
    let mut f = fake();
    // Header-only, the way the engine stores a draft from another device until its body
    // is fetched: no `plaintext`, no files yet.
    f.messages
        .push(json!({"id": "d-phone", "aid": "a1", "threadId": "t1",
        "hMsgId": "phone-1@x.test", "subject": "From my phone", "date": 3000, "draft": true,
        "from": [{"name": "Me", "email": ME}], "to": [{"email": "bob@y.test"}],
        "cc": [], "bcc": [], "replyTo": [], "files": [], "folders": {"f-dr": 9}}));
    let file = json!({"id": "file-9", "filename": "scan.pdf", "contentType": "application/pdf", "size": 3});
    f.remote_bodies.insert(
        "d-phone".into(),
        (
            "<p>typed on the phone</p>".into(),
            json!({"plaintext": false, "files": [file]}),
        ),
    );
    let send = policy("read-write-send", json!({}));
    json_of(&run(
        &mut f,
        &send,
        "send_draft",
        json!({"headerMessageId": "phone-1@x.test"}),
    ));
    let draft = &f.queued.last().unwrap()["draft"];
    assert_eq!(
        (&draft["body"], &draft["plaintext"]),
        (&json!("<p>typed on the phone</p>"), &json!(false))
    );
    assert_eq!(draft["files"], json!([file]));
    assert_eq!(draft["file_ids"], json!(["file-9"]));

    // update_draft likewise keeps the attachments the body fetch found.
    f.messages.last_mut().unwrap()["files"] = json!([]);
    f.bodies.remove("d-phone");
    f.remote_bodies.insert(
        "d-phone".into(),
        (
            "<p>typed on the phone</p>".into(),
            json!({"plaintext": false, "files": [file]}),
        ),
    );
    json_of(&run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d-phone", "subject": "Scan"}),
    ));
    let draft = &f.queued.last().unwrap()["draft"];
    assert_eq!(
        (&draft["subject"], &draft["file_ids"]),
        (&json!("Scan"), &json!(["file-9"]))
    );
    assert!(draft.get("body").is_none());
}

#[test]
fn delete_draft_queues_destroy() {
    let mut f = fake();
    json_of(&run(
        &mut f,
        &rw(),
        "delete_draft",
        json!({"headerMessageId": "draft-0@x.test"}),
    ));
    let t = f.queued.last().unwrap();
    assert_eq!(
        (&t["__cls"], &t["messageIds"]),
        (&json!("DestroyDraftTask"), &json!(["d0"]))
    );
}

#[test]
fn create_draft_then_find_it_by_header_message_id() {
    let mut f = fake();
    let out = json_of(&run(
        &mut f,
        &rw(),
        "create_draft",
        json!({"to": [{"email": "bob@y.test"}], "subject": "Hi", "body": "<p>x</p>"}),
    ));
    assert_eq!(out["draft"], true);
    let hmid = out["headerMessageId"].as_str().unwrap().to_owned();
    assert!(hmid.ends_with("@x.test"));
    assert!(f.queued[0]["draft"].get("threadId").is_none());
    let o = json_of(&run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"headerMessageId": hmid, "subject": "Hello"}),
    ));
    assert_eq!(o["id"], out["id"]);
}

/// The default sending account comes from the engine's settings when it keeps them, and an
/// unset key there means none, whatever `config.json` says.
#[test]
fn new_drafts_use_the_engine_default_sending_account() {
    let draft_account = |f: &mut Fake| {
        let out = json_of(&run(
            f,
            &rw(),
            "create_draft",
            json!({"to": [{"email": "bob@y.test"}], "subject": "Hi", "body": "<p>x</p>"}),
        ));
        out["accountId"].as_str().unwrap().to_owned()
    };
    let mut f = fake();
    f.accounts.push(Fake::account("a2", "two@x.test"));
    assert_eq!(draft_account(&mut f), "a1");
    f.features.push(ms_ipc::feature::SETTINGS.into());
    f.settings
        .insert("core.sending.defaultAccountIdForSend".into(), json!("a2"));
    assert_eq!(draft_account(&mut f), "a2");
    f.settings.clear();
    assert_eq!(draft_account(&mut f), "a1");
}

#[test]
fn messages_are_marked_as_drafts_or_not() {
    let mut f = fake();
    let m = json_of(&run(
        &mut f,
        &rw(),
        "get_message",
        json!({"messageId": "d0"}),
    ));
    assert_eq!(m["draft"], true);
    let m = json_of(&run(
        &mut f,
        &rw(),
        "get_message",
        json!({"messageId": "m1"}),
    ));
    assert_eq!(
        (&m["draft"], &m["body"]),
        (&json!(false), &json!("<p>original from bob</p>"))
    );
    assert_eq!(m["folders"], json!([{"id": "f-in", "name": "Inbox"}]));
}

#[test]
fn policy_is_enforced() {
    let mut f = fake();
    let ro = policy("read-only", json!({}));
    let o = run(
        &mut f,
        &ro,
        "set_starred",
        json!({"threadIds": ["t1"], "starred": true}),
    );
    assert!(o.is_error && o.denied);
    assert!(o.first_text().contains("require access level 'read-write'"));
    let o = run(
        &mut f,
        &rw(),
        "send_draft",
        json!({"headerMessageId": "draft-0@x.test"}),
    );
    assert!(o.is_error && o.denied && o.first_text().contains("'read-write-send'"));
    assert!(f.queued.is_empty());

    // A disabled account is invisible.
    let hidden = policy(
        "read-write",
        json!({"a1": {"enabled": false}, "other": {"enabled": true}}),
    );
    assert_eq!(
        json_of(&run(&mut f, &hidden, "list_accounts", json!({}))),
        json!([])
    );
    let o = run(&mut f, &hidden, "get_thread", json!({"threadId": "t1"}));
    assert!(o.is_error && o.first_text().contains("Thread 't1' not found"));
    let o = run(&mut f, &hidden, "list_folders", json!({"accountId": "a1"}));
    assert!(o.first_text().contains("Account 'a1' not found"));

    // An excluded folder hides the thread and the folder.
    let excl = policy(
        "read-write",
        json!({"a1": {"enabled": true, "excludedFolderIds": ["f-dr"]}}),
    );
    let o = run(
        &mut f,
        &excl,
        "set_starred",
        json!({"threadIds": ["t1"], "starred": true}),
    );
    assert!(o.is_error && o.denied);
    let folders = json_of(&run(
        &mut f,
        &excl,
        "list_folders",
        json!({"accountId": "a1"}),
    ));
    assert!(!folders
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "f-dr"));
}

#[test]
fn thread_changes_queue_the_desktop_tasks() {
    let mut f = fake();
    json_of(&run(
        &mut f,
        &rw(),
        "set_starred",
        json!({"threadIds": ["t1"], "starred": true}),
    ));
    json_of(&run(
        &mut f,
        &rw(),
        "archive_threads",
        json!({"threadIds": ["t1"]}),
    ));
    json_of(&run(
        &mut f,
        &rw(),
        "trash_threads",
        json!({"threadIds": ["t1"]}),
    ));
    let o = run(
        &mut f,
        &rw(),
        "set_labels",
        json!({"threadIds": ["t1"], "labelsToAdd": ["x"]}),
    );
    assert!(o.first_text().contains("does not use labels"));
    let classes: Vec<_> = f
        .queued
        .iter()
        .map(|t| (t["__cls"].clone(), t["folder"]["id"].clone()))
        .collect();
    assert_eq!(
        classes,
        [
            (json!("ChangeStarredTask"), Value::Null),
            (json!("ChangeFolderTask"), json!("f-ar")),
            (json!("ChangeFolderTask"), json!("f-tr")),
        ]
    );
    assert_eq!(f.queued[0]["threadIds"], json!(["t1"]));
    let o = run(
        &mut f,
        &rw(),
        "set_starred",
        json!({"threadIds": ["nope"], "starred": true}),
    );
    assert!(o.first_text().contains("Thread 'nope' not found"));
}

#[test]
fn arguments_are_validated() {
    let mut f = fake();
    for (tool, args, needle) in [
        (
            "set_starred",
            json!({"threadIds": "t1", "starred": true}),
            "'threadIds' must be an array of strings",
        ),
        (
            "set_starred",
            json!({"threadIds": ["t1"]}),
            "'starred' must be a boolean",
        ),
        (
            "search_mail",
            json!({"query": "a", "limit": -1}),
            "'limit' must be a non-negative number",
        ),
        (
            "create_draft",
            json!({"to": [{"name": "x"}], "subject": "s", "body": "b"}),
            "'to' must be an array",
        ),
        (
            "reply_to_thread",
            json!({"threadId": "t1", "body": "b", "type": "forward"}),
            "'type' must be",
        ),
        (
            "schedule_send",
            json!({"headerMessageId": "h", "sendAt": "tomorrow"}),
            "Invalid sendAt",
        ),
        ("nope", json!({}), "Unknown tool"),
    ] {
        let p = policy("read-write-send", json!({}));
        let o = run(&mut f, &p, tool, args);
        assert!(
            o.is_error && o.first_text().contains(needle),
            "{tool}: {}",
            o.first_text()
        );
    }
}

#[test]
fn send_at_without_an_offset_is_local_time() {
    use super::write::parse_send_at;
    let utc_minus_7 = chrono::FixedOffset::west_opt(7 * 3600).unwrap();
    let at = |s: &str| {
        parse_send_at(s, &utc_minus_7).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    };
    for (input, want) in [
        ("2026-10-17T09:00:00", "2026-10-17T16:00:00Z"),
        ("2026-10-17T09:00", "2026-10-17T16:00:00Z"),
        ("2026-10-17T09:00:30.500", "2026-10-17T16:00:30Z"),
        ("2026-10-17T09:00:00Z", "2026-10-17T09:00:00Z"),
        ("2026-10-17T09:00Z", "2026-10-17T09:00:00Z"),
        ("2026-10-17T09:00+02:00", "2026-10-17T07:00:00Z"),
        // A date alone is UTC, as in JavaScript.
        ("2026-10-17", "2026-10-17T00:00:00Z"),
    ] {
        assert_eq!(at(input).as_deref(), Some(want), "{input}");
    }
    assert_eq!(at("tomorrow"), None);
    assert_eq!(at("2026-13-01T09:00"), None);
}

#[test]
fn schedule_send_says_the_desktop_does_the_send() {
    let mut f = fake();
    let send = policy("read-write-send", json!({}));
    let at = (chrono::Local::now() + chrono::Duration::hours(2))
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    let out = json_of(&run(
        &mut f,
        &send,
        "schedule_send",
        json!({"headerMessageId": "draft-0@x.test", "sendAt": at}),
    ));
    assert!(out["note"].as_str().unwrap().contains("desktop app"));
    let want = chrono::NaiveDateTime::parse_from_str(&at, "%Y-%m-%dT%H:%M")
        .unwrap()
        .and_local_timezone(chrono::Local)
        .earliest()
        .unwrap()
        .timestamp();
    let task = f.queued.last().unwrap();
    assert_eq!(task["value"]["expiration"], want, "{task}");
}

#[test]
fn no_engine_is_an_error_the_model_can_read() {
    let mut f = Fake::default();
    let o = run(&mut f, &rw(), "list_accounts", json!({}));
    assert!(o.is_error && o.first_text().contains("no engine running"));
}

#[test]
fn search_and_listing() {
    let mut f = fake();
    let r = json_of(&run(
        &mut f,
        &rw(),
        "search_mail",
        json!({"query": "plans in:inbox is:unread"}),
    ));
    assert_eq!(r.as_array().unwrap().len(), 1);
    assert_eq!(r[0]["unread"], true);
    let r = json_of(&run(
        &mut f,
        &rw(),
        "search_mail",
        json!({"query": "in:trash"}),
    ));
    assert_eq!(r, json!([]));
    let r = json_of(&run(
        &mut f,
        &rw(),
        "list_threads",
        json!({"folderId": "f-in", "accountId": "a1", "unread": false}),
    ));
    assert_eq!(r, json!([]));
    let r = json_of(&run(
        &mut f,
        &rw(),
        "list_threads",
        json!({"folderId": "f-in", "accountId": "a1"}),
    ));
    assert_eq!(r[0]["id"], "t1");
    let o = run(
        &mut f,
        &rw(),
        "list_threads",
        json!({"folderId": "f-nope", "accountId": "a1"}),
    );
    assert!(o.first_text().contains("Folder 'f-nope' not found"));
}

/// Two sent messages with tracking, a scheduled draft and one whose schedule has passed.
fn tracked() -> Fake {
    let mut f = fake();
    f.features = vec![ms_ipc::feature::METADATA_LIST.into()];
    let ev = |r: &str, ts: i64| json!({"recipient": r, "timestamp": ts});
    let sent = |id: &str, date: i64, metadata: Value| {
        json!({"id": id, "aid": "a1", "threadId": "t1", "hMsgId": format!("{id}@x.test"),
               "subject": format!("Sent {id}"), "date": date, "draft": false,
               "from": [{"name": "Me", "email": ME}], "to": [{"name": "Bob", "email": "bob@y.test"}],
               "folders": {"f-in": 9}, "metadata": metadata})
    };
    f.messages.push(sent(
        "s1",
        3000,
        json!([
            {"pluginId": "open-tracking", "value": {"open_count": 2,
                "open_data": [ev("bob@y.test", 3100), ev("bob@y.test", 5000)]}},
            {"pluginId": "link-tracking", "value": {"links": [
                {"url": "https://a.test", "click_count": 3,
                 "click_data": [ev("x", 3200), ev("y", 3300), ev("x", 3400)]},
                {"url": "https://b.test", "click_count": 1, "click_data": [ev("x", 3500)]}]}}
        ]),
    ));
    f.messages.push(sent(
        "s2",
        2000,
        json!([{"pluginId": "link-tracking", "value": {"links": [
            {"url": "https://c.test", "click_count": 5, "click_data": [ev("z", 2100)]}]}}]),
    ));
    let future = chrono::Utc::now().timestamp() + 3600;
    f.messages[1]["metadata"] = json!([{"pluginId": "send-later",
        "value": {"expiration": future, "actionKey": "send"}}]);
    let mut past = f.messages[1].clone();
    past["id"] = json!("d-past");
    past["hMsgId"] = json!("past@x.test");
    past["metadata"] = json!([{"pluginId": "send-later", "value": {"expiration": 100}}]);
    f.messages.push(past);
    f
}

#[test]
fn tracked_messages_and_top_links() {
    let mut f = tracked();
    let r = json_of(&run(&mut f, &rw(), "list_tracked_messages", json!({})));
    assert_eq!(r["total"], 2);
    let ids: Vec<&str> = r["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["s1", "s2"], "newest first, drafts left out");
    assert_eq!(r["messages"][0]["opens"]["count"], 2);
    assert_eq!(
        r["messages"][0]["to"],
        json!([{"name": "Bob", "email": "bob@y.test"}])
    );
    assert_eq!(r["messages"][1]["opens"], Value::Null);
    let count = |args: Value, f: &mut Fake| {
        json_of(&run(f, &rw(), "list_tracked_messages", args))["total"].clone()
    };
    assert_eq!(count(json!({"trackingType": "opens"}), &mut f), 1);
    assert_eq!(count(json!({"minClicks": 5}), &mut f), 1);
    assert_eq!(count(json!({"minOpens": 1}), &mut f), 1);
    assert_eq!(
        count(json!({"sentAfter": "1970-01-01T00:40:00Z"}), &mut f),
        1,
        "s1 was sent at 00:50, s2 at 00:33"
    );
    assert_eq!(
        count(json!({"sentBefore": "1970-01-01T00:40:00Z"}), &mut f),
        1
    );
    assert_eq!(
        count(json!({"openedAfter": "1970-01-01T01:20:00Z"}), &mut f),
        1,
        "only s1's 5000 s open is after 4800 s"
    );
    let o = run(
        &mut f,
        &rw(),
        "list_tracked_messages",
        json!({"trackingType": "clicks"}),
    );
    assert!(o.is_error, "{}", o.first_text());
    let o = run(
        &mut f,
        &rw(),
        "list_tracked_messages",
        json!({"sentAfter": "not a date"}),
    );
    assert!(o.first_text().contains("Invalid sentAfter date"));

    let top = json_of(&run(&mut f, &rw(), "get_top_links", json!({"limit": 2})));
    assert_eq!(
        top,
        json!([
            {"url": "https://c.test", "clickCount": 5, "uniqueRecipients": 1, "messageId": "s2",
             "threadId": "t1", "subject": "Sent s2", "date": "1970-01-01T00:33:20.000Z",
             "accountId": "a1"},
            {"url": "https://a.test", "clickCount": 3, "uniqueRecipients": 2, "messageId": "s1",
             "threadId": "t1", "subject": "Sent s1", "date": "1970-01-01T00:50:00.000Z",
             "accountId": "a1"}
        ])
    );

    f.features.clear();
    let o = run(&mut f, &rw(), "get_top_links", json!({}));
    assert!(
        o.is_error && o.first_text().contains("needs a newer Tern engine"),
        "{}",
        o.first_text()
    );
}

#[test]
fn scheduled_sends_are_the_future_ones() {
    let mut f = tracked();
    let r = json_of(&run(&mut f, &rw(), "list_scheduled_sends", json!({})));
    let r = r.as_array().unwrap();
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0]["headerMessageId"], "draft-0@x.test");
    assert_eq!(r[0]["actionKey"], "send");
    assert_eq!(r[0]["to"], json!([{"name": "Bob", "email": "bob@y.test"}]));
    assert!(r[0]["sendAt"].as_str().unwrap().ends_with('Z'));
    let o = run(
        &mut f,
        &rw(),
        "list_scheduled_sends",
        json!({"accountId": "nope"}),
    );
    assert!(o.first_text().contains("Account 'nope' not found"));
}

#[test]
fn a_send_queued_elsewhere_blocks_edits_when_the_engine_lists_tasks() {
    let mut f = fake();
    f.features = vec![ms_ipc::feature::TASKS_LIST.into()];
    f.tasks.insert(
        "desktop-send".into(),
        json!({"id": "desktop-send", "aid": "a1", "__cls": "SendDraftTask",
               "status": "remote", "headerMessageId": "draft-0@x.test"}),
    );
    let o = run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "body": "x"}),
    );
    assert!(
        o.is_error && o.first_text().contains("being sent"),
        "{}",
        o.first_text()
    );
    f.tasks.get_mut("desktop-send").unwrap()["status"] = json!("complete");
    json_of(&run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "body": "x"}),
    ));
}
