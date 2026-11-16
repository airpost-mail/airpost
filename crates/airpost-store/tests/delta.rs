// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Delta coalescing and flush timing.

mod common;

use std::thread::sleep;
use std::time::{Duration, Instant};

use airpost_core::constants::{
    DELTA_TYPE_METADATA_EXPIRATION, DELTA_TYPE_PERSIST, DELTA_TYPE_UNPERSIST,
};
use airpost_store::{DeltaItem, DeltaStream, Message, Model, Query, Task};
use common::{fixture, Sink, ACCOUNT};
use serde_json::json;

fn stream() -> (Sink, std::sync::Arc<DeltaStream>) {
    let sink = Sink::default();
    let stream = DeltaStream::new(Box::new(sink.clone()));
    (sink, stream)
}

fn item(kind: &str, class: &str, jsons: Vec<serde_json::Value>) -> DeltaItem {
    DeltaItem::new(kind, class, jsons)
}

/// Waits up to `limit` for the sink to hold `n` lines.
fn wait_lines(sink: &Sink, n: usize, limit: Duration) -> Vec<serde_json::Value> {
    let start = Instant::now();
    while start.elapsed() < limit {
        let lines = sink.lines();
        if lines.len() >= n {
            return lines;
        }
        sleep(Duration::from_millis(2));
    }
    sink.lines()
}

#[test]
fn line_format_is_sorted_and_compact() {
    let i = item(
        DELTA_TYPE_PERSIST,
        "Message",
        vec![json!({"z": 1, "id": "a", "b": {"y": 2, "x": [1, {"d": 0, "c": 0}]}})],
    );
    assert_eq!(
        i.dump(),
        r#"{"modelClass":"Message","modelJSONs":[{"b":{"x":[1,{"c":0,"d":0}],"y":2},"id":"a","z":1}],"type":"persist"}"#
    );
}

#[test]
fn persist_unpersist_persist_yields_three_items() {
    let (sink, s) = stream();
    s.emit(
        vec![
            item(
                DELTA_TYPE_PERSIST,
                "Thread",
                vec![json!({"id": "t1", "v": 1})],
            ),
            item(
                DELTA_TYPE_UNPERSIST,
                "Thread",
                vec![json!({"id": "t1", "v": 1})],
            ),
            item(
                DELTA_TYPE_PERSIST,
                "Thread",
                vec![json!({"id": "t1", "v": 2})],
            ),
        ],
        10_000,
    );
    s.flush();
    let lines = sink.lines();
    let kinds: Vec<&str> = lines.iter().map(|l| l["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["persist", "unpersist", "persist"]);
}

#[test]
fn same_id_merge_keeps_earlier_keys() {
    let (sink, s) = stream();
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Message",
            vec![json!({"id": "m1", "v": 1, "body": "<p>hi</p>", "rulesReady": true})],
        )],
        10_000,
    );
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Message",
            vec![json!({"id": "m1", "v": 2}), json!({"id": "m2", "v": 1})],
        )],
        10_000,
    );
    s.flush();
    let lines = sink.lines();
    assert_eq!(lines.len(), 1);
    let jsons = lines[0]["modelJSONs"].as_array().unwrap();
    assert_eq!(jsons.len(), 2);
    assert_eq!(
        jsons[0],
        json!({"id": "m1", "v": 2, "body": "<p>hi</p>", "rulesReady": true})
    );
    assert_eq!(jsons[1]["id"], "m2");
}

#[test]
fn classes_flush_in_alphabetical_order() {
    let (sink, s) = stream();
    for class in ["Thread", "Contact", "Message", "Folder"] {
        s.emit(
            vec![item(DELTA_TYPE_PERSIST, class, vec![json!({"id": class})])],
            10_000,
        );
    }
    s.flush();
    let classes: Vec<String> = sink
        .lines()
        .iter()
        .map(|l| l["modelClass"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(classes, ["Contact", "Folder", "Message", "Thread"]);
}

#[test]
fn worker_delay_holds_deltas_for_500ms() {
    let (sink, s) = stream();
    let start = Instant::now();
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Message",
            vec![json!({"id": "a"})],
        )],
        500,
    );
    sleep(Duration::from_millis(300));
    assert!(
        sink.lines().is_empty(),
        "flushed before the 500 ms deadline"
    );
    let lines = wait_lines(&sink, 1, Duration::from_secs(3));
    assert_eq!(lines.len(), 1);
    assert!(start.elapsed() >= Duration::from_millis(500));
}

#[test]
fn main_thread_delay_flushes_within_5ms() {
    let (sink, s) = stream();
    let start = Instant::now();
    s.emit(
        vec![item(DELTA_TYPE_PERSIST, "Task", vec![json!({"id": "a"})])],
        5,
    );
    let lines = wait_lines(&sink, 1, Duration::from_secs(2));
    assert_eq!(lines.len(), 1);
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "took {:?}",
        start.elapsed()
    );
}

#[test]
fn zero_delay_flushes_at_once() {
    let (sink, s) = stream();
    let start = Instant::now();
    s.begin_connection_error(ACCOUNT);
    let lines = wait_lines(&sink, 1, Duration::from_secs(2));
    assert!(start.elapsed() < Duration::from_millis(250));
    assert_eq!(
        lines[0],
        json!({"modelClass": "ProcessState", "type": "persist", "modelJSONs": [{"accountId": ACCOUNT, "id": ACCOUNT, "connectionError": true}]})
    );
}

#[test]
fn earlier_deadline_flushes_everything_now() {
    let (sink, s) = stream();
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Message",
            vec![json!({"id": "slow"})],
        )],
        5_000,
    );
    sleep(Duration::from_millis(50));
    assert!(sink.lines().is_empty());
    let start = Instant::now();
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Task",
            vec![json!({"id": "fast"})],
        )],
        5,
    );
    let lines = wait_lines(&sink, 2, Duration::from_secs(2));
    assert_eq!(
        lines.len(),
        2,
        "the pending Message delta goes out with the Task one"
    );
    assert!(start.elapsed() < Duration::from_millis(250));
}

#[test]
fn later_deadline_does_not_delay_the_scheduled_flush() {
    let (sink, s) = stream();
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Message",
            vec![json!({"id": "a"})],
        )],
        50,
    );
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Message",
            vec![json!({"id": "b"})],
        )],
        10_000,
    );
    let lines = wait_lines(&sink, 1, Duration::from_secs(2));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["modelJSONs"].as_array().unwrap().len(), 2);
}

#[test]
fn connection_error_end_is_sent_only_after_begin() {
    let (sink, s) = stream();
    s.end_connection_error(ACCOUNT);
    sleep(Duration::from_millis(50));
    assert!(sink.lines().is_empty());
    s.begin_connection_error(ACCOUNT);
    s.end_connection_error(ACCOUNT);
    wait_lines(&sink, 1, Duration::from_secs(2));
    sleep(Duration::from_millis(100));
    // Either one merged item or two, depending on whether the flush ran in between; the
    // last state the client sees is the end.
    let states: Vec<bool> = sink
        .lines()
        .iter()
        .flat_map(|l| l["modelJSONs"].as_array().unwrap().clone())
        .map(|j| j["connectionError"].as_bool().unwrap())
        .collect();
    assert_eq!(states.last(), Some(&false));
    assert!(states.len() <= 2);
}

#[test]
fn secrets_and_identity_refresh() {
    let (sink, s) = stream();
    let account = airpost_core::Account::from_json(json!({"id": ACCOUNT, "settings": {}})).unwrap();
    s.send_updated_secrets(&account);
    s.send_identity_refresh_needed();
    let lines = wait_lines(&sink, 2, Duration::from_secs(2));
    assert_eq!(lines[0]["modelClass"], "ProcessAccountSecretsUpdated");
    assert_eq!(lines[0]["modelJSONs"][0]["id"], ACCOUNT);
    assert_eq!(lines[0]["modelJSONs"][0]["__cls"], "Account");
    assert_eq!(
        lines[1],
        json!({"modelClass": "ProcessIdentityRefreshNeeded", "modelJSONs": [], "type": "persist"})
    );
}

#[test]
fn nothing_is_flushed_on_drop() {
    let (sink, s) = stream();
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Message",
            vec![json!({"id": "a"})],
        )],
        10_000,
    );
    drop(s);
    sleep(Duration::from_millis(50));
    assert!(sink.lines().is_empty());
}

#[test]
fn remove_with_no_matches_emits_none() {
    let f = fixture();
    f.store
        .remove_all::<Task>(&Query::new().equal("accountId", "nobody"))
        .unwrap();
    f.stream.flush();
    assert_eq!(
        f.sink.lines(),
        [json!({"modelClass": "none", "modelJSONs": [], "type": "unpersist"})]
    );
}

#[test]
fn remove_all_emits_full_json() {
    let f = fixture();
    let mut t = Task::create("ChangeStarredTask", ACCOUNT, &json!({"starred": true}));
    f.store.save(&mut t).unwrap();
    f.stream.flush();
    f.sink.clear();
    f.store
        .remove_all::<Task>(&Query::new().equal("accountId", ACCOUNT))
        .unwrap();
    f.stream.flush();
    let lines = f.sink.lines();
    assert_eq!(lines[0]["type"], "unpersist");
    assert_eq!(lines[0]["modelClass"], "Task");
    assert_eq!(lines[0]["modelJSONs"][0]["starred"], true);
    assert_eq!(lines[0]["modelJSONs"][0]["__cls"], "ChangeStarredTask");
    assert_eq!(f.store.count::<Task>(&Query::new()).unwrap(), 0);
}

#[test]
fn message_dispatch_adds_body_and_sync_flags() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let mut msg = f.insert("m1@x", &inbox, 1, common::attrs(1, true), None);
    f.stream.flush();
    let first = f
        .sink
        .lines()
        .into_iter()
        .find(|l| l["modelClass"] == "Message")
        .unwrap();
    // Version 1: headers are in.
    assert_eq!(first["modelJSONs"][0]["headersSyncComplete"], true);
    assert!(first["modelJSONs"][0].get("body").is_none());
    assert!(first["modelJSONs"][0].get("rulesReady").is_none());

    // The body arrives: rr goes to HasBody, and with an Inbox copy the save is rules-ready.
    f.sink.clear();
    f.store
        .conn()
        .execute(
            "INSERT INTO MessageBody (id, value, fetchedAt) VALUES (?, ?, 0)",
            [msg.id(), "<p>body</p>"],
        )
        .unwrap();
    msg.set_body_for_dispatch("<p>body</p>");
    msg.mark_body_stored();
    f.store.save(&mut msg).unwrap();
    f.stream.flush();
    let j = f
        .sink
        .lines()
        .into_iter()
        .find(|l| l["modelClass"] == "Message")
        .unwrap()["modelJSONs"][0]
        .clone();
    assert_eq!(j["body"], "<p>body</p>");
    assert_eq!(j["fullSyncComplete"], true);
    assert_eq!(j["rulesReady"], true);
    assert!(j.get("headersSyncComplete").is_none());
    assert_eq!(j["rr"], 2);

    // Later saves are not rules-ready again.
    f.sink.clear();
    let mut again = f.load(msg.id());
    f.store.save(&mut again).unwrap();
    f.stream.flush();
    let j = f
        .sink
        .lines()
        .into_iter()
        .find(|l| l["modelClass"] == "Message")
        .unwrap()["modelJSONs"][0]
        .clone();
    assert!(
        j.get("rulesReady").is_none()
            && j.get("body").is_none()
            && j.get("fullSyncComplete").is_none()
    );
}

#[test]
fn rules_ready_reads_body_back_when_the_incoming_copy_arrives_second() {
    let f = fixture();
    let sent = f.folder("Sent", "sent");
    let inbox = f.folder("INBOX", "inbox");
    let mut msg = f.insert("self@x", &sent, 1, common::attrs(1, false), None);
    f.store
        .conn()
        .execute(
            "INSERT INTO MessageBody (id, value, fetchedAt) VALUES (?, ?, 0)",
            [msg.id(), "stored body"],
        )
        .unwrap();
    msg.mark_body_stored();
    f.store.save(&mut msg).unwrap();
    f.stream.flush();
    let j = f
        .sink
        .lines()
        .into_iter()
        .rev()
        .find(|l| l["modelClass"] == "Message")
        .unwrap()["modelJSONs"][0]
        .clone();
    assert!(
        j.get("rulesReady").is_none(),
        "only a Sent copy: not rules-ready"
    );

    f.sink.clear();
    let mut msg = f.load(msg.id());
    f.store
        .upsert_placement(&mut msg, &inbox, 7, &common::attrs(7, true))
        .unwrap();
    f.store.save(&mut msg).unwrap();
    f.stream.flush();
    let j = f
        .sink
        .lines()
        .into_iter()
        .find(|l| l["modelClass"] == "Message")
        .unwrap()["modelJSONs"][0]
        .clone();
    assert_eq!(j["rulesReady"], true);
    assert_eq!(j["body"], "stored body");
    assert!(
        j.get("fullSyncComplete").is_none(),
        "read back, not fetched"
    );
    f.assert_invariants();
}

#[test]
fn metadata_expiration_delta() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let msg = f.insert("m1@x", &inbox, 1, common::attrs(1, true), None);
    let found = f
        .store
        .find_all_generic("message", &Query::new().equal_any("id", [msg.id()]))
        .unwrap();
    f.stream.flush();
    f.sink.clear();
    let items: Vec<DeltaItem> = found
        .iter()
        .map(|m| {
            DeltaItem::new(
                DELTA_TYPE_METADATA_EXPIRATION,
                m.table(),
                vec![m.delta_json()],
            )
        })
        .collect();
    f.stream.emit(items, 0);
    let lines = wait_lines(&f.sink, 1, Duration::from_secs(2));
    assert_eq!(lines[0]["type"], "metadata-expiration");
    assert_eq!(lines[0]["modelClass"], "Message");
    assert_eq!(lines[0]["modelJSONs"][0]["id"], msg.id());
    let via_model = DeltaItem::for_model(DELTA_TYPE_METADATA_EXPIRATION, &msg);
    assert_eq!(via_model.model_class, "Message");
}

#[test]
fn transaction_emits_after_commit_only() {
    let f = fixture();
    let s = &f.store;
    s.transaction("t", |s| {
        let mut t = Task::create("A", ACCOUNT, &json!({}));
        s.save(&mut t)?;
        s.delta_stream().flush();
        assert!(f.sink.lines().is_empty(), "deltas wait for the commit");
        Ok(())
    })
    .unwrap();
    s.delta_stream().flush();
    assert_eq!(f.sink.lines().len(), 1);

    f.sink.clear();
    let r: airpost_core::Result<()> = s.transaction("t", |s| {
        let mut t = Task::create("B", ACCOUNT, &json!({}));
        s.save(&mut t)?;
        Err(airpost_core::Error::other("boom"))
    });
    assert!(r.is_err());
    s.delta_stream().flush();
    assert!(f.sink.lines().is_empty(), "a rollback drops its deltas");
    assert_eq!(s.count::<Task>(&Query::new()).unwrap(), 1);
}

#[test]
fn nested_transaction_failure_rolls_back_only_its_part() {
    let f = fixture();
    let s = &f.store;
    s.transaction("outer", |s| {
        let mut a = Task::create("A", ACCOUNT, &json!({}));
        s.save(&mut a)?;
        let inner: airpost_core::Result<()> = s.transaction("inner", |s| {
            let mut b = Task::create("B", ACCOUNT, &json!({}));
            s.save(&mut b)?;
            Err(airpost_core::Error::other("constraint race"))
        });
        assert!(inner.is_err());
        let mut c = Task::create("C", ACCOUNT, &json!({}));
        s.save(&mut c)
    })
    .unwrap();
    s.delta_stream().flush();
    let classes: Vec<String> = s
        .find_all::<Task>(&Query::new())
        .unwrap()
        .iter()
        .map(|t| t.class().to_owned())
        .collect();
    assert_eq!(classes.len(), 2);
    assert!(classes.contains(&"A".to_owned()) && classes.contains(&"C".to_owned()));
    let jsons = f.sink.lines()[0]["modelJSONs"].as_array().unwrap().len();
    assert_eq!(jsons, 2, "B's delta went with its savepoint");
}

#[test]
fn unsafe_erase_drops_transaction_deltas() {
    let f = fixture();
    f.store
        .transaction("t", |s| {
            let mut t = Task::create("A", ACCOUNT, &json!({}));
            s.save(&mut t)?;
            s.unsafe_erase_transaction_deltas();
            Ok(())
        })
        .unwrap();
    f.stream.flush();
    assert!(f.sink.lines().is_empty());
}

#[test]
fn unpersist_carries_full_json() {
    let f = fixture();
    let inbox = f.folder("INBOX", "inbox");
    let mut msg = f.insert("m1@x", &inbox, 1, common::attrs(1, true), None);
    f.stream.flush();
    f.sink.clear();
    let mut msg2 = Message::new(msg.data.clone());
    msg2.data["v"] = msg.data["v"].clone();
    f.store.remove(&mut msg).unwrap();
    f.stream.flush();
    let lines = f.sink.lines();
    let m = lines.iter().find(|l| l["modelClass"] == "Message").unwrap();
    assert_eq!(m["type"], "unpersist");
    assert_eq!(m["modelJSONs"][0]["subject"], msg2.subject());
    assert_eq!(m["modelJSONs"][0]["__cls"], "Message");
    // The thread had only this message, so it goes too.
    let t = lines.iter().find(|l| l["modelClass"] == "Thread").unwrap();
    assert_eq!(t["type"], "unpersist");
    f.assert_invariants();
}

/// A custom sink gets each flushed item's class with the same line the stdout sink writes.
#[test]
fn custom_sink_gets_class_and_stdout_line() {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Pairs(Arc<Mutex<Vec<(String, String)>>>);
    impl airpost_store::DeltaSink for Pairs {
        fn write_item(&mut self, model_class: &str, line: &str) {
            self.0
                .lock()
                .unwrap()
                .push((model_class.to_owned(), line.to_owned()));
        }
    }

    let pairs = Pairs::default();
    let s = DeltaStream::with_sink(Box::new(pairs.clone()));
    let (sink, stdout) = stream();
    for st in [&s, &stdout] {
        st.emit(
            vec![
                item(DELTA_TYPE_PERSIST, "Thread", vec![json!({"id": "t1"})]),
                item(DELTA_TYPE_UNPERSIST, "Message", vec![json!({"id": "m1"})]),
            ],
            0,
        );
        st.flush();
    }
    let got = pairs.0.lock().unwrap().clone();
    assert_eq!(
        got.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
        ["Message", "Thread"]
    );
    let expected: String = got.iter().map(|(_, l)| format!("{l}\n")).collect();
    assert_eq!(sink.text(), expected);
}

/// `close` delivers what is buffered and drops what comes after, a connection error
/// included.
#[test]
fn close_flushes_then_drops_later_items() {
    let (sink, s) = stream();
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Thread",
            vec![json!({"id": "t1"})],
        )],
        500,
    );
    s.close();
    assert_eq!(sink.lines().len(), 1);
    s.emit(
        vec![item(
            DELTA_TYPE_PERSIST,
            "Thread",
            vec![json!({"id": "t2"})],
        )],
        0,
    );
    s.begin_connection_error(ACCOUNT);
    s.flush();
    sleep(Duration::from_millis(50));
    assert_eq!(sink.lines().len(), 1);
}
