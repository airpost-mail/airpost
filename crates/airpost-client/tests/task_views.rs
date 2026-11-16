// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The task JSON the CLI and MCP queue (`airpost_client::tasks`) against the task view structs
//! of `airpost_ipc::tasks` and their JSON Schema, from which the clients' TS types come.

use airpost_client::tasks;
use airpost_ipc::schema::violations;
use serde_json::{json, Value};

fn folder() -> Value {
    json!({"id": "f1", "aid": "a1", "v": 1, "__cls": "Folder", "path": "Archive", "role": "archive"})
}

fn draft() -> Value {
    json!({
        "id": "d1", "aid": "a1", "v": 1, "__cls": "Message", "subject": "Hi", "date": 1_700_000_000,
        "threadId": "", "hMsgId": "h1@airpost", "unread": false, "starred": false, "draft": true,
        "pristine": false, "plaintext": false, "body": "<p>Hi</p>", "folders": {}, "labels": [],
        "from": [{"name": "Me", "email": "me@example.com"}], "to": [{"email": "ann@example.com"}],
        "cc": [], "bcc": [], "replyTo": [], "files": [],
    })
}

#[test]
fn builders_fit_the_task_schema() {
    let built = [
        ("SyncbackDraftTaskJson", tasks::syncback_draft(draft())),
        ("SendDraftTaskJson", tasks::send_draft(draft())),
        (
            "DestroyDraftTaskJson",
            tasks::destroy_draft("a1", &["d1".to_owned()]),
        ),
        (
            "SyncbackMetadataTaskJson",
            tasks::syncback_message_metadata(
                &draft(),
                "send-later",
                json!({"expiration": 5}),
                None,
            ),
        ),
        (
            "SyncbackMetadataTaskJson",
            tasks::syncback_thread_metadata("a1", "t1", "thread-snooze", json!({})),
        ),
        (
            "GetMessageRFC2822TaskJson",
            tasks::get_message_rfc2822("a1", "m1", "/tmp/m1.eml"),
        ),
        (
            "GetManyRFC2822TaskJson",
            tasks::get_many_rfc2822("a1", &folder(), "/tmp/out"),
        ),
    ];
    let mut problems = Vec::new();
    for (def, task) in &built {
        problems.extend(violations(task, def));
        // The union every task result is typed as takes it too.
        problems.extend(violations(task, "TaskJson"));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
