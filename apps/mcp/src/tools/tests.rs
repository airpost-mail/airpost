// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

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
        features: vec![
            airpost_ipc::feature::INTENTS.into(),
            airpost_ipc::feature::UNDO.into(),
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
    run_on(session, false, f, p, tool, args)
}

fn run_on(
    session: &Session,
    over_http: bool,
    f: &mut Fake,
    p: &Policy,
    tool: &str,
    args: Value,
) -> Output {
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
        over_http,
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

fn editing_engine() -> Fake {
    let mut f = fake();
    f.features.push(airpost_ipc::feature::DRAFT_PIPELINE.into());
    f.features.push(airpost_ipc::feature::DRAFT_EDITING.into());
    f
}

#[test]
fn an_engine_with_draft_editing_edits_attaches_and_deletes_drafts() {
    let mut f = editing_engine();
    let queued = f.queued.len();
    let out = json_of(&run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "body": "<p>revised</p>", "to": [{"email": "dan@y.test"}],
               "attachments": ["/home/me/report.pdf"], "removeAttachments": ["plan.pdf"]}),
    ));
    let (method, params) = f.actions.last().unwrap().clone();
    assert_eq!(method, "draft.update");
    assert_eq!(
        params,
        json!({"accountId": "a1", "headerMessageId": "draft-0@x.test",
               "fields": {"body": "<p>revised</p>",
                          "to": [{"name": "", "email": "dan@y.test", "__cls": "Contact"}]},
               "attach": ["/home/me/report.pdf"], "detach": ["plan.pdf"]})
    );
    assert_eq!(f.queued.len(), queued, "the engine saves the draft itself");
    assert_eq!((&out["id"], &out["updated"]), (&json!("d0"), &json!(true)));
    assert_eq!(
        out["attachments"],
        json!([{"id": "upload-report.pdf", "filename": "report.pdf", "size": 3}])
    );
    // A subject-only edit sends no body and no file changes.
    run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "subject": "S"}),
    );
    let (_, params) = f.actions.last().unwrap().clone();
    assert_eq!(params["fields"], json!({"subject": "S"}));
    assert!(params.get("attach").is_none() && params.get("detach").is_none());

    let out = json_of(&run(
        &mut f,
        &rw(),
        "create_draft",
        json!({"to": [{"email": "gail@y.test"}], "subject": "Hi", "body": "<p>Hello</p>",
               "attachments": ["/home/me/a.pdf"]}),
    ));
    let (method, params) = f.actions.last().unwrap().clone();
    assert_eq!(
        (method.as_str(), &params["attach"], &params["save"]),
        ("draft.create", &json!(["/home/me/a.pdf"]), &json!(false))
    );
    let draft = &f.queued.last().unwrap()["draft"];
    assert_eq!(draft["file_ids"], json!(["upload-a.pdf"]));
    assert_eq!(draft["files"][0]["filename"], "a.pdf");
    assert_eq!(out["attachments"][0]["filename"], "a.pdf");

    let out = json_of(&run(
        &mut f,
        &rw(),
        "reply_to_thread",
        json!({"threadId": "t1", "body": "<p>Here</p>", "attachments": ["/home/me/b.pdf"]}),
    ));
    assert_eq!(
        f.actions.last().unwrap().1["attach"],
        json!(["/home/me/b.pdf"])
    );
    assert_eq!(out["attachments"][0]["id"], "upload-b.pdf");

    let queued = f.queued.len();
    let out = json_of(&run(&mut f, &rw(), "delete_draft", json!({"id": "d0"})));
    assert_eq!(out["deleted"], true);
    let (method, params) = f.actions.last().unwrap().clone();
    assert_eq!(
        (method.as_str(), &params["headerMessageId"]),
        ("draft.delete", &json!("draft-0@x.test"))
    );
    assert_eq!(
        f.queued.len(),
        queued,
        "the engine queues the DestroyDraftTask"
    );
}

#[test]
fn templates_are_listed_and_write_drafts() {
    let mut f = editing_engine();
    f.templates
        .push(json!({"name": "Follow up", "subject": "Checking in",
        "body": "<div>Hi <span data-tvar=\"name\">Name</span></div>"}));
    let old = run(&mut f, &rw(), "list_templates", json!({}));
    assert!(old.is_error && old.first_text().contains("update Airpost"));
    let o = run(
        &mut f,
        &rw(),
        "create_draft",
        json!({"to": [{"email": "a@y.test"}], "template": "Follow up"}),
    );
    assert!(o.is_error && o.first_text().contains("update Airpost"));
    f.features.push(airpost_ipc::feature::TEMPLATES.into());

    let list = json_of(&run(&mut f, &rw(), "list_templates", json!({})));
    assert_eq!(list[0]["name"], "Follow up");
    assert_eq!(list[0]["variables"], json!(["name"]));
    let one = json_of(&run(
        &mut f,
        &rw(),
        "list_templates",
        json!({"name": "Follow up"}),
    ));
    assert!(one["body"].as_str().unwrap().contains("data-tvar"));
    let o = run(&mut f, &rw(), "list_templates", json!({"name": "Nope"}));
    assert!(o.is_error && o.first_text().contains("not found"));

    let out = json_of(&run(
        &mut f,
        &rw(),
        "create_draft",
        json!({"to": [{"email": "a@y.test"}], "template": "Follow up"}),
    ));
    assert_eq!(f.actions.last().unwrap().1["template"], "Follow up");
    assert_eq!(out["subject"], "Checking in");
    let draft = &f.queued.last().unwrap()["draft"];
    assert!(draft["body"].as_str().unwrap().starts_with("<div>Hi <span"));

    let out = json_of(&run(
        &mut f,
        &rw(),
        "reply_to_thread",
        json!({"threadId": "t1", "template": "Follow up"}),
    ));
    assert_eq!(out["subject"], "Re: Plans");
    let body = f.queued.last().unwrap()["draft"]["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        body.starts_with("<div>Hi <span") && body.contains("original from bob"),
        "{body}"
    );

    for args in [
        json!({"to": [{"email": "a@y.test"}], "subject": "s", "body": "b", "template": "Follow up"}),
        json!({"to": [{"email": "a@y.test"}], "subject": "s"}),
    ] {
        let o = run(&mut f, &rw(), "create_draft", args);
        assert!(
            o.is_error && o.first_text().contains("'body'"),
            "{}",
            o.first_text()
        );
    }
}

#[test]
fn message_warnings_from_the_engine_show_in_get_message_and_get_thread() {
    let mut f = fake();
    let warning = json!([{"code": "reply-to-mismatch", "message": "This message looks suspicious! It originates from bob@y.test but replies will go to eve@z.test."}]);
    f.messages[0]["warnings"] = warning.clone();
    let m = json_of(&run(
        &mut f,
        &rw(),
        "get_message",
        json!({"messageId": "m1"}),
    ));
    assert_eq!(m["warnings"], warning);
    let t = json_of(&run(&mut f, &rw(), "get_thread", json!({"threadId": "t1"})));
    assert_eq!(t["messages"][0]["warnings"], warning);
    assert!(t["messages"][1].get("warnings").is_none());
}

#[test]
fn attachments_need_draft_editing_absolute_paths_and_a_local_client() {
    let pdf = json!(["/home/me/report.pdf"]);
    for features in [vec![], vec![airpost_ipc::feature::DRAFT_PIPELINE]] {
        let mut f = fake();
        f.features = features.iter().map(|s| s.to_string()).collect();
        for (tool, args) in [
            ("update_draft", json!({"id": "d0", "attachments": pdf})),
            (
                "update_draft",
                json!({"id": "d0", "removeAttachments": ["plan.pdf"]}),
            ),
            (
                "create_draft",
                json!({"to": [{"email": "a@y.test"}], "subject": "s", "body": "b", "attachments": pdf}),
            ),
            (
                "reply_to_thread",
                json!({"threadId": "t1", "body": "b", "attachments": pdf}),
            ),
        ] {
            let o = run(&mut f, &rw(), tool, args);
            assert!(
                o.is_error && o.first_text().contains("draft-editing"),
                "{tool}: {}",
                o.first_text()
            );
        }
        assert!(f.queued.is_empty(), "{features:?}");
    }

    let mut f = editing_engine();
    let o = run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "attachments": ["report.pdf"]}),
    );
    assert!(
        o.is_error && o.first_text().contains("absolute"),
        "{}",
        o.first_text()
    );
    let o = run_on(
        &Session::default(),
        true,
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "attachments": pdf}),
    );
    assert!(o.is_error && o.denied, "{}", o.first_text());
    assert!(f.actions.is_empty());
    // Removing one isn't reading a file: fine over HTTP too.
    let o = run_on(
        &Session::default(),
        true,
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "removeAttachments": ["file-1"]}),
    );
    assert_eq!(json_of(&o).get("attachments"), None);
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
    f.features.push(airpost_ipc::feature::SETTINGS.into());
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
fn thread_changes_are_the_engine_intents() {
    let mut f = fake();
    let starred = json_of(&run(
        &mut f,
        &rw(),
        "set_starred",
        json!({"threadIds": ["t1"], "starred": true}),
    ));
    assert_eq!(starred["undo"], "undo-1");
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
    let moved = json_of(&run(
        &mut f,
        &rw(),
        "move_to_folder",
        json!({"threadIds": ["t1"], "folderId": "f-ar"}),
    ));
    assert_eq!(moved["moved"], 1);
    let o = run(
        &mut f,
        &rw(),
        "move_to_folder",
        json!({"threadIds": ["t1"], "folderId": "nope"}),
    );
    assert!(o.first_text().contains("Folder 'nope' not found"));
    let o = run(
        &mut f,
        &rw(),
        "set_labels",
        json!({"threadIds": ["t1"], "labelsToAdd": ["x"]}),
    );
    assert!(o.first_text().contains("folders, not labels"));
    let calls: Vec<_> = f
        .actions
        .iter()
        .map(|(m, p)| {
            let mut p = p.clone();
            p.as_object_mut().unwrap().remove("accountId");
            (m.as_str(), p)
        })
        .collect();
    assert_eq!(
        calls,
        [
            (
                "threads.star",
                json!({"threadIds": ["t1"], "starred": true})
            ),
            ("threads.archive", json!({"threadIds": ["t1"]})),
            ("threads.trash", json!({"threadIds": ["t1"]})),
            ("threads.move", json!({"threadIds": ["t1"], "to": "f-ar"})),
            (
                "threads.label",
                json!({"threadIds": ["t1"], "add": ["x"], "remove": []})
            ),
        ]
    );
    assert!(f.queued.is_empty());
    let o = run(
        &mut f,
        &rw(),
        "set_starred",
        json!({"threadIds": ["nope"], "starred": true}),
    );
    assert!(o.first_text().contains("Thread 'nope' not found"));

    // An engine without the intents says so.
    f.features.clear();
    let o = run(
        &mut f,
        &rw(),
        "archive_threads",
        json!({"threadIds": ["t1"]}),
    );
    assert!(o.is_error && o.first_text().contains("update Airpost"));
}

#[test]
fn undo_takes_back_only_this_assistants_changes() {
    let mut f = fake();
    let entry = |token: &str, client: &str, account: &str| {
        json!({"token": token, "client": client, "action": "archive", "label": "Archived 1 thread",
               "count": 1, "at": 0, "accountIds": [account], "taskIds": ["t"], "state": "done",
               "canUndo": true})
    };
    let o = run(&mut f, &rw(), "undo", json!({}));
    assert!(o.is_error && o.first_text().contains("Nothing to undo"));
    f.undo_log = vec![
        entry("u-desktop", "airpost-desktop", "a1"),
        entry("u-mine", "airpost-mcp", "a1"),
    ];
    let o = run(&mut f, &rw(), "undo", json!({"token": "u-desktop"}));
    assert!(o.is_error && o.first_text().contains("No change 'u-desktop'"));
    let out = json_of(&run(&mut f, &rw(), "undo", json!({})));
    assert_eq!(
        (&out["undone"], &out["token"], &out["canRedo"]),
        (&json!("Archived 1 thread"), &json!("u-mine"), &json!(true))
    );
    let o = run(&mut f, &rw(), "undo", json!({}));
    assert!(o.first_text().contains("Nothing to undo"));
    let out = json_of(&run(&mut f, &rw(), "undo", json!({"redo": true})));
    assert_eq!(out["redone"], "Archived 1 thread");
    let applied: Vec<_> = f
        .actions
        .iter()
        .map(|(m, p)| (m.as_str(), p["token"].clone()))
        .collect();
    assert_eq!(
        applied,
        [
            ("undo.apply", json!("u-mine")),
            ("undo.redo", json!("u-mine"))
        ]
    );

    // A change on an account the policy now excludes stays.
    let hidden = policy(
        "read-write",
        json!({"a1": {"enabled": true}, "a2": {"enabled": false}}),
    );
    f.accounts.push(Fake::account("a2", "two@x.test"));
    f.undo_log = vec![entry("u-two", "airpost-mcp", "a2")];
    let o = run(&mut f, &hidden, "undo", json!({}));
    assert!(o.is_error && o.denied, "{}", o.first_text());
    let o = run(&mut f, &policy("read-only", json!({})), "undo", json!({}));
    assert!(o.is_error && o.denied);
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
fn schedule_send_says_who_does_the_send() {
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
    f.features
        .push(airpost_ipc::feature::EXPIRATION_EXECUTOR.into());
    let out = json_of(&run(
        &mut f,
        &send,
        "schedule_send",
        json!({"headerMessageId": "draft-0@x.test", "sendAt": at}),
    ));
    assert!(out["note"].as_str().unwrap().contains("engine sends"));
    let want = chrono::NaiveDateTime::parse_from_str(&at, "%Y-%m-%dT%H:%M")
        .unwrap()
        .and_local_timezone(chrono::Local)
        .earliest()
        .unwrap()
        .timestamp();
    let task = f.queued.last().unwrap();
    assert_eq!(task["value"]["expiration"], want, "{task}");
    assert_eq!(task["value"]["clientFinalize"], Value::Null, "{task}");

    // A plugin's send action: the engine would decline it, so the row says the app sends it.
    let out = json_of(&run(
        &mut f,
        &send,
        "schedule_send",
        json!({"headerMessageId": "draft-0@x.test", "sendAt": at, "actionKey": "send-and-snooze"}),
    ));
    assert!(
        out["note"].as_str().unwrap().contains("desktop app"),
        "{out}"
    );
    let task = f.queued.last().unwrap();
    assert_eq!(task["value"]["actionKey"], "send-and-snooze", "{task}");
    assert_eq!(task["value"]["clientFinalize"], true, "{task}");
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

#[test]
fn search_hands_the_grammar_to_the_engine() {
    let mut f = fake();
    f.features = vec![airpost_ipc::feature::SEARCH_GRAMMAR.into()];
    f.threads.push(
        json!({"id": "t2", "aid": "a1", "subject": "Other", "unread": 0,
        "starred": 0, "lmt": 3000, "lmrt": 3000, "labels": [], "participants": [],
        "folders": [{"id": "f-secret", "path": "Secret", "role": ""}]}),
    );
    let q = "(from:bob OR from:carol) NOT in:trash after:yesterday";
    let r = json_of(&run(&mut f, &rw(), "search_mail", json!({"query": q})));
    // No filters of airpost-mcp's own: the engine's answer, newest first.
    assert_eq!(r.as_array().unwrap().len(), 2);
    assert_eq!((&r[0]["id"], &r[1]["id"]), (&json!("t2"), &json!("t1")));
    let sent = &f.searches[0];
    assert_eq!(sent["query"], q);
    assert_eq!(sent["grammar"], true);
    assert!(sent["now"].as_i64().unwrap() > 1_700_000_000, "{sent}");
    // The access policy still applies.
    let excl = policy(
        "read-write",
        json!({"a1": {"enabled": true, "excludedFolderIds": ["f-secret"]}}),
    );
    let r = json_of(&run(&mut f, &excl, "search_mail", json!({"query": "x"})));
    assert_eq!(r.as_array().unwrap().len(), 1);
    assert_eq!(r[0]["id"], "t1");
    let o = run(&mut f, &rw(), "search_mail", json!({"query": "(x"}));
    assert!(
        o.is_error && o.first_text().contains("never closed"),
        "{}",
        o.first_text()
    );
}

/// Two sent messages with tracking, a scheduled draft and one whose schedule has passed.
fn tracked() -> Fake {
    let mut f = fake();
    f.features = vec![airpost_ipc::feature::METADATA_LIST.into()];
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
        o.is_error && o.first_text().contains("needs a newer Airpost engine"),
        "{}",
        o.first_text()
    );
}

#[test]
fn scheduled_sends_include_due_and_failed_ones() {
    let mut f = tracked();
    let mut failed = f.messages[1].clone();
    failed["id"] = json!("d-failed");
    failed["hMsgId"] = json!("failed@x.test");
    failed["metadata"] = json!([{"pluginId": "send-later", "value": {"expiration": null,
        "actionKey": "send", "error": "The draft's From address isn't one of this account's."}}]);
    f.messages.push(failed);
    let mut cancelled = f.messages[1].clone();
    cancelled["id"] = json!("d-cancelled");
    cancelled["hMsgId"] = json!("cancelled@x.test");
    cancelled["metadata"] = json!([{"pluginId": "send-later", "value": {"expiration": null}}]);
    f.messages.push(cancelled);

    let r = json_of(&run(&mut f, &rw(), "list_scheduled_sends", json!({})));
    let r = r.as_array().unwrap();
    let states: Vec<(&str, &str)> = r
        .iter()
        .map(|s| {
            (
                s["headerMessageId"].as_str().unwrap(),
                s["state"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        states,
        [
            ("failed@x.test", "failed"),
            ("past@x.test", "due"),
            ("draft-0@x.test", "scheduled")
        ],
        "{r:?}"
    );
    assert!(r[0]["error"].as_str().unwrap().contains("From address"));
    assert_eq!(r[0]["sendAt"], Value::Null);
    assert_eq!(r[1]["sendAt"], "1970-01-01T00:01:40.000Z");
    assert_eq!(r[2]["actionKey"], "send");
    assert_eq!(r[2]["to"], json!([{"name": "Bob", "email": "bob@y.test"}]));
    assert!(r[2]["sendAt"].as_str().unwrap().ends_with('Z'));
    assert_eq!(r[2].get("error"), None);
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
    f.features = vec![airpost_ipc::feature::TASKS_LIST.into()];
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

#[test]
fn snooze_and_unsnooze_go_through_the_engine() {
    let mut f = fake();
    let at = (chrono::Local::now() + chrono::Duration::hours(3))
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    let args = json!({"threadIds": ["t1"], "until": at});
    // An engine without the methods says so.
    let o = run(&mut f, &rw(), "snooze_threads", args.clone());
    assert!(
        o.is_error && o.first_text().contains("update Airpost"),
        "{}",
        o.first_text()
    );
    f.features.push(airpost_ipc::feature::SNOOZE.into());
    let out = json_of(&run(&mut f, &rw(), "snooze_threads", args));
    assert_eq!(out["snoozed"], 1);
    let (m, params) = f.actions.last().unwrap();
    assert_eq!(m, "threads.snooze");
    assert_eq!(params["threadIds"], json!(["t1"]));
    let want = chrono::NaiveDateTime::parse_from_str(&at, "%Y-%m-%dT%H:%M")
        .unwrap()
        .and_local_timezone(chrono::Local)
        .earliest()
        .unwrap()
        .timestamp();
    assert_eq!(params["until"], want);

    let past = run(
        &mut f,
        &rw(),
        "snooze_threads",
        json!({"threadIds": ["t1"], "until": "2020-01-01T09:00"}),
    );
    assert!(past.is_error && past.first_text().contains("future"));
    let ro = run(
        &mut f,
        &policy("read-only", json!({})),
        "snooze_threads",
        json!({"threadIds": ["t1"], "until": at}),
    );
    assert!(ro.is_error && ro.denied);

    let out = json_of(&run(
        &mut f,
        &rw(),
        "unsnooze_threads",
        json!({"threadIds": ["t1"]}),
    ));
    assert_eq!(out["unsnoozed"], 1);
    assert_eq!(f.actions.last().unwrap().0, "threads.unsnooze");
    let missing = run(
        &mut f,
        &rw(),
        "unsnooze_threads",
        json!({"threadIds": ["nope"]}),
    );
    assert!(missing.is_error && missing.first_text().contains("not found"));
}

#[test]
fn unsubscribe_needs_send_access_and_says_what_to_do() {
    let mut f = fake();
    f.features.push(airpost_ipc::feature::UNSUBSCRIBE.into());
    f.messages[0]["hListUnsub"] = json!("<https://lists.y.test/u/1>");
    let send = policy("read-write-send", json!({}));
    let o = run(&mut f, &rw(), "unsubscribe", json!({"messageId": "m1"}));
    assert!(o.is_error && o.denied, "{}", o.first_text());
    let out = json_of(&run(
        &mut f,
        &send,
        "unsubscribe",
        json!({"messageId": "m1", "method": "web"}),
    ));
    assert_eq!(out["url"], "https://lists.y.test/u/1");
    assert!(out["note"].as_str().unwrap().contains("browser"));
    let (_, params) = f.actions.last().unwrap();
    assert_eq!(params["method"], "web");
    let o = run(&mut f, &send, "unsubscribe", json!({"messageId": "d0"}));
    assert!(
        o.is_error && o.first_text().contains("List-Unsubscribe"),
        "{}",
        o.first_text()
    );
}

#[test]
fn an_engine_with_the_draft_pipeline_builds_and_sends_the_drafts() {
    let mut f = fake();
    f.features.push(airpost_ipc::feature::DRAFT_PIPELINE.into());
    json_of(&run(
        &mut f,
        &rw(),
        "reply_to_thread",
        json!({"threadId": "t1", "body": "<p>Sounds good</p>"}),
    ));
    let (method, params) = f.actions.last().unwrap().clone();
    assert_eq!(method, "draft.create");
    assert_eq!(
        (&params["kind"], &params["messageId"], &params["save"]),
        (&json!("reply"), &json!("m1"), &json!(false))
    );
    let draft = &f.queued.last().unwrap()["draft"];
    let body = draft["body"].as_str().unwrap();
    assert!(
        body.starts_with("<p>Sounds good</p><!-- engine -->"),
        "{body}"
    );
    assert!(body.contains("Bob &lt;bob@y.test&gt; wrote:"), "{body}");
    assert_eq!(
        (&draft["pristine"], &draft["threadId"]),
        (&json!(false), &json!("t1"))
    );

    json_of(&run(
        &mut f,
        &rw(),
        "create_draft",
        json!({"to": [{"email": "gail@y.test"}], "subject": "Hi", "body": "<p>Hello</p>"}),
    ));
    let (method, params) = f.actions.last().unwrap().clone();
    assert_eq!(
        (method.as_str(), &params["kind"]),
        ("draft.create", &json!("new"))
    );
    let draft = &f.queued.last().unwrap()["draft"];
    assert_eq!(draft["subject"], "Hi");
    assert_eq!(draft["to"][0]["email"], "gail@y.test");

    let queued = f.queued.len();
    json_of(&run(
        &mut f,
        &policy("read-write-send", json!({})),
        "send_draft",
        json!({"headerMessageId": "draft-0@x.test"}),
    ));
    let (method, params) = f.actions.last().unwrap().clone();
    assert_eq!(
        (method.as_str(), &params["headerMessageId"]),
        ("draft.send", &json!("draft-0@x.test"))
    );
    assert_eq!(f.queued.len(), queued, "the engine queues the send itself");
}

/// The tools' bodies are HTML: drafts stay HTML when the composer's default is plaintext, with
/// the pipeline and without.
#[test]
fn drafts_are_html_when_the_composer_default_is_plaintext() {
    for pipeline in [false, true] {
        let mut f = fake();
        f.features.push(airpost_ipc::feature::SETTINGS.into());
        f.settings
            .insert("core.composing.html".into(), json!(false));
        if pipeline {
            f.features.push(airpost_ipc::feature::DRAFT_PIPELINE.into());
        }
        json_of(&run(
            &mut f,
            &rw(),
            "create_draft",
            json!({"to": [{"email": "bob@y.test"}], "subject": "Hi", "body": "<p>Hello</p>"}),
        ));
        json_of(&run(
            &mut f,
            &rw(),
            "reply_to_thread",
            json!({"threadId": "t1", "body": "<p>Sounds good</p>"}),
        ));
        for queued in &f.queued {
            let draft = &queued["draft"];
            assert_eq!(draft["plaintext"], false, "pipeline {pipeline}: {draft}");
            assert!(
                draft["body"].as_str().unwrap().starts_with("<p>"),
                "pipeline {pipeline}: {draft}"
            );
        }
        if pipeline {
            let (_, params) = f.actions.last().unwrap();
            assert_eq!(params["html"], true);
        }
    }
}

#[test]
fn calendar_and_contact_tools() {
    let mut f = fake();
    f.features = vec![
        airpost_ipc::feature::CALENDAR_EDITING.into(),
        airpost_ipc::feature::CONTACT_EDITING.into(),
    ];
    let ro = policy("read-only", json!({}));
    let send = policy("read-write-send", json!({}));

    let out = json_of(&run(
        &mut f,
        &ro,
        "list_events",
        json!({"start": "2026-03-01T00:00:00Z", "end": "2026-03-08T00:00:00Z"}),
    ));
    assert_eq!(out["count"], 2);
    // Oldest first; all-day ones by their day, end exclusive.
    assert_eq!(out["events"][0]["eventId"], "ev1");
    assert_eq!(out["events"][0]["startDate"], "1970-01-01");
    assert_eq!(out["events"][0]["endDate"], "1970-01-02");
    assert_eq!(out["events"][1]["start"], "1970-01-01T00:33:20Z");
    let (_, p) = f.actions.last().unwrap();
    assert_eq!(p["start"], 1_772_323_200);
    assert!(p["tz"].is_string());

    // Writes need read-write; guests, whom the server mails, read-write-send.
    let create = json!({"calendarId": "cal", "title": "Lunch",
        "start": "2026-03-02T12:00:00Z", "end": "2026-03-02T13:00:00Z"});
    let o = run(&mut f, &ro, "create_event", create.clone());
    assert!(o.is_error && o.denied);
    let out = json_of(&run(&mut f, &rw(), "create_event", create.clone()));
    assert_eq!(out["eventId"], "ev-new");
    let mut with_guests = create.clone();
    with_guests["attendees"] = json!([{"email": "g@y.test"}]);
    let o = run(&mut f, &rw(), "create_event", with_guests.clone());
    assert!(o.is_error && o.denied, "{}", o.first_text());
    assert!(!run(&mut f, &send, "create_event", with_guests).is_error);
    let (m, p) = f.actions.last().unwrap();
    assert_eq!(m, "events.create");
    assert_eq!(p["attendees"], json!([{"email": "g@y.test"}]));

    let out = json_of(&run(
        &mut f,
        &rw(),
        "update_event",
        json!({"eventId": "ev1", "scope": "this", "occurrenceStart": "2026-03-02T09:00:00Z",
               "title": "Moved", "start": "2026-03-02T10:00:00Z"}),
    ));
    assert_eq!(out["writes"][0]["kind"], "update");
    let (_, p) = f.actions.last().unwrap();
    assert_eq!(
        p["patch"],
        json!({"summary": "Moved", "start": 1_772_445_600})
    );
    assert_eq!(
        (p["scope"].as_str(), p["occurrenceStart"].as_i64()),
        (Some("this"), Some(1_772_442_000))
    );
    let o = run(
        &mut f,
        &rw(),
        "update_event",
        json!({"eventId": "missing", "title": "x"}),
    );
    assert!(
        o.is_error && o.first_text().contains("not found"),
        "{}",
        o.first_text()
    );
    assert!(run(&mut f, &rw(), "update_event", json!({"eventId": "ev1"})).is_error);

    // A meeting the account organizes mails its guests any change: read-write asks the engine
    // without writing and is refused; read-write-send writes at once.
    for args in [
        json!({"eventId": "meeting", "start": "2026-03-02T10:00:00Z"}),
        json!({"eventId": "meeting", "description": "agenda"}),
        json!({"eventId": "meeting", "delete": true, "scope": "this",
               "occurrenceStart": "2026-03-02T09:00:00Z"}),
    ] {
        let before = f.actions.len();
        let o = run(&mut f, &rw(), "update_event", args.clone());
        assert!(o.is_error && o.denied, "{}", o.first_text());
        let asked: Vec<&Value> = f.actions[before..].iter().map(|(_, p)| p).collect();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0]["queue"], false);
        let before = f.actions.len();
        assert!(!run(&mut f, &send, "update_event", args).is_error);
        assert_eq!(f.actions.len(), before + 1);
        assert!(f.actions.last().unwrap().1.get("queue").is_none());
    }
    // One that mails nobody is written after the check.
    let before = f.actions.len();
    assert!(
        !run(
            &mut f,
            &rw(),
            "update_event",
            json!({"eventId": "ev1", "title": "x"})
        )
        .is_error
    );
    assert_eq!(f.actions.len(), before + 2);
    assert!(f.actions.last().unwrap().1.get("queue").is_none());

    // An answer mails the organizer.
    let o = run(
        &mut f,
        &rw(),
        "respond_to_event",
        json!({"eventId": "ev1", "status": "accepted"}),
    );
    assert!(o.is_error && o.denied);
    let out = json_of(&run(
        &mut f,
        &send,
        "respond_to_event",
        json!({"eventId": "ev1", "status": "accepted"}),
    ));
    assert_eq!(
        (
            out["responded"].as_bool(),
            out["organizerNotified"].as_bool()
        ),
        (Some(true), Some(true))
    );
    let out = json_of(&run(
        &mut f,
        &send,
        "respond_to_event",
        json!({"eventId": "ev1", "status": "tentative",
               "proposedStart": "2026-03-02T15:00:00Z", "proposedEnd": "2026-03-02T16:00:00Z"}),
    ));
    assert_eq!(out["proposed"], true);
    assert!(
        run(
            &mut f,
            &send,
            "respond_to_event",
            json!({"eventId": "ev1", "status": "maybe"})
        )
        .is_error
    );

    let out = json_of(&run(&mut f, &ro, "get_contact", json!({"contactId": "c1"})));
    assert_eq!(out["source"], "CardDAV");
    let out = json_of(&run(
        &mut f,
        &rw(),
        "update_contact",
        json!({"contactId": "c1", "title": "Lead", "phone": "+1 555"}),
    ));
    assert_eq!(out["details"]["title"], "Lead");
    assert_eq!(out["details"]["phoneNumbers"], json!([{"value": "+1 555"}]));
    let out = json_of(&run(
        &mut f,
        &rw(),
        "create_contact",
        json!({"name": "Ann Archer", "email": "ann@y.test"}),
    ));
    assert_eq!(out["details"]["name"]["familyName"], "Archer");
    assert!(run(&mut f, &rw(), "create_contact", json!({})).is_error);

    // Accounts the policy disables don't exist.
    let only_other = policy("read-write", json!({"a1": {"enabled": false}}));
    let o = run(
        &mut f,
        &only_other,
        "get_contact",
        json!({"contactId": "c1"}),
    );
    assert!(o.is_error);
}

#[test]
fn create_label_is_the_engine_intent_and_returns_the_new_id() {
    let mut f = fake();
    let o = run(
        &mut f,
        &rw(),
        "create_label",
        json!({"accountId": "a1", "name": "Receipts"}),
    );
    assert!(o.first_text().contains("does not use labels"));
    assert!(f.actions.is_empty());

    // Not among the desktop's accounts, so its labels say it is Gmail.
    f.accounts.push(Fake::account("g1", "me@gmail.test"));
    f.labels.push(
        json!({"id": "l-inbox", "aid": "g1", "path": "INBOX", "role": "inbox", "__cls": "Label"}),
    );
    let o = run(
        &mut f,
        &policy("read-only", json!({})),
        "create_label",
        json!({"accountId": "g1", "name": "Receipts"}),
    );
    assert!(o.is_error && o.denied);
    let r = json_of(&run(
        &mut f,
        &rw(),
        "create_label",
        json!({"accountId": "g1", "name": "Reçus"}),
    ));
    assert_eq!(r["labelId"], "l-Re&AOc-us");
    assert_eq!(
        (r["name"].as_str(), r["path"].as_str()),
        (Some("Reçus"), Some("Re&AOc-us"))
    );
    let [(m, params)] = f.actions.as_slice() else {
        panic!("one call")
    };
    assert_eq!(
        (m.as_str(), &params["accountId"], &params["name"]),
        ("categories.create", &json!("g1"), &json!("Reçus"))
    );
    let o = run(
        &mut f,
        &rw(),
        "create_label",
        json!({"accountId": "g1", "name": "Reçus"}),
    );
    assert!(
        o.first_text().contains("already a folder or label"),
        "{}",
        o.first_text()
    );
    let o = run(
        &mut f,
        &rw(),
        "create_label",
        json!({"accountId": "nope", "name": "X"}),
    );
    assert!(o.first_text().contains("not found"));
    assert!(f.queued.is_empty());
}

fn metadata_tasks(f: &Fake) -> Vec<Value> {
    f.queued
        .iter()
        .filter(|t| t["__cls"] == "SyncbackMetadataTask")
        .cloned()
        .collect()
}

#[test]
fn draft_tools_set_reminders_as_the_composer_does() {
    let mut f = fake();
    let out = json_of(&run(
        &mut f,
        &rw(),
        "create_draft",
        json!({"to": [{"email": "gail@y.test"}], "subject": "Hi", "body": "<p>Hello</p>",
               "remindInDays": 2}),
    ));
    let [task]: [Value; 1] = metadata_tasks(&f).try_into().unwrap();
    assert_eq!(task["pluginId"], "thread:send-reminders");
    assert_eq!(
        (&task["modelId"], &task["modelClassName"]),
        (&out["id"], &json!("message"))
    );
    assert_eq!(task["value"]["sentHeaderMessageId"], out["headerMessageId"]);
    let at = task["value"]["expiration"].as_i64().unwrap();
    let expected = chrono::Utc::now().timestamp() + 2 * 86_400;
    assert!((at - expected).abs() < 60, "{at}");
    assert!(out["remindAt"].is_string());

    let out = run(
        &mut f,
        &rw(),
        "create_draft",
        json!({"to": [{"email": "g@y.test"}], "subject": "x", "body": "y", "remindInDays": 0}),
    );
    assert!(out.is_error && out.first_text().contains("remindInDays"));

    // update_draft with only a reminder writes the metadata and leaves the draft alone.
    let drafts_saved = f.queued.len() - metadata_tasks(&f).len();
    json_of(&run(
        &mut f,
        &rw(),
        "update_draft",
        json!({"id": "d0", "remindInDays": 1.5}),
    ));
    assert_eq!(f.queued.len() - metadata_tasks(&f).len(), drafts_saved);
    let last = metadata_tasks(&f).pop().unwrap();
    assert_eq!(
        (&last["modelId"], &last["value"]["sentHeaderMessageId"]),
        (&json!("d0"), &json!("draft-0@x.test"))
    );
}

#[test]
fn tracking_needs_a_server_that_offers_it() {
    let mut f = fake();
    let args = json!({"to": [{"email": "g@y.test"}], "subject": "x", "body": "<p>y</p>",
                      "trackOpens": true, "trackLinks": true});
    let o = run(&mut f, &rw(), "create_draft", args.clone());
    assert!(
        o.is_error && o.first_text().contains("update Airpost"),
        "{}",
        o.first_text()
    );
    f.features.push(airpost_ipc::feature::TRACKING.into());
    let o = run(&mut f, &rw(), "create_draft", args.clone());
    assert!(
        o.is_error && o.first_text().contains("needs an Airpost server"),
        "{}",
        o.first_text()
    );
    assert!(
        f.queued.is_empty(),
        "nothing is written when tracking can't be sent"
    );
    f.capabilities = json!({"server": {"readReceipts": {"available": true},
                                       "linkTracking": {"available": true}}});
    let out = json_of(&run(&mut f, &rw(), "create_draft", args));
    assert_eq!(out["tracking"], json!({"opens": true, "links": true}));
    let values: Vec<(Value, Value)> = metadata_tasks(&f)
        .iter()
        .map(|t| (t["pluginId"].clone(), t["value"].clone()))
        .collect();
    assert_eq!(
        values,
        [
            (
                json!("open-tracking"),
                json!({"open_count": 0, "open_data": []})
            ),
            (json!("link-tracking"), json!({"tracked": true})),
        ]
    );
}

#[test]
fn set_reminder_writes_the_threads_reminder() {
    let mut f = fake();
    f.messages.push(
        json!({"id": "m2", "aid": "a1", "threadId": "t1", "hMsgId": "mine@x.test",
        "subject": "Re: Plans", "date": 1500, "draft": false,
        "from": [{"name": "Me", "email": ME}], "to": [], "cc": [], "bcc": [], "replyTo": [],
        "files": [], "folders": {"f-in": 2}}),
    );
    let out = json_of(&run(
        &mut f,
        &rw(),
        "set_reminder",
        json!({"threadId": "t1", "remindInDays": 3}),
    ));
    assert_eq!(
        (&out["threadId"], &out["cleared"]),
        (&json!("t1"), &json!(false))
    );
    let [task]: [Value; 1] = metadata_tasks(&f).try_into().unwrap();
    assert_eq!(
        (&task["pluginId"], &task["modelClassName"], &task["modelId"]),
        (&json!("send-reminders"), &json!("thread"), &json!("t1"))
    );
    let v = &task["value"];
    assert_eq!(
        (
            &v["sentHeaderMessageId"],
            &v["lastReplyTimestamp"],
            &v["shouldNotify"]
        ),
        (&json!("mine@x.test"), &json!(1000), &json!(false))
    );
    json_of(&run(
        &mut f,
        &rw(),
        "set_reminder",
        json!({"threadId": "t1", "clear": true}),
    ));
    assert_eq!(metadata_tasks(&f).pop().unwrap()["value"], json!({}));
    for bad in [
        json!({"threadId": "t1"}),
        json!({"threadId": "t1", "remindInDays": 1, "clear": true}),
    ] {
        let o = run(&mut f, &rw(), "set_reminder", bad);
        assert!(o.is_error && o.first_text().contains("remindInDays"));
    }
    let o = run(
        &mut f,
        &policy("read-only", json!({})),
        "set_reminder",
        json!({"threadId": "t1", "clear": true}),
    );
    assert!(o.is_error && o.denied);
}
