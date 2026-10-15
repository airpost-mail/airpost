//! The metadata threads against a migrated database: the expiration scan
//! (`MetadataExpirationWorker.cpp`) and the backend pull with its detached-metadata
//! hand-off (`MetadataWorker.cpp`, `MailModel::beforeSave`), the pull through an in-test
//! backend.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ms_core::{ManualClock, Metadata, Result};
use ms_net::{MetadataBackend, MetadataDelta, MetadataPush};
use ms_store::{DeltaStream, Message, Model, Query, Store, Thread};
use ms_sync::metadata_expiration::MetadataExpirationWorker;
use ms_sync::metadata_worker::{apply_metadata, MetadataWorker};
use serde_json::{json, Value};

const ACCOUNT: &str = "acc1";
const T0: i64 = 1_700_000_000;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    /// Every delta line written so far.
    fn deltas(&self) -> Vec<Value> {
        let bytes = std::mem::take(&mut *self.0.lock().unwrap());
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    deltas: Arc<DeltaStream>,
    out: Captured,
    clock: Arc<ManualClock>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(T0));
    ms_store::migrate::migrate(dir.path(), &*clock, &mut Vec::new()).unwrap();
    let out = Captured::default();
    let deltas = DeltaStream::new(Box::new(out.clone()));
    let store = Store::open(dir.path(), deltas.clone()).unwrap();
    Fixture {
        _dir: dir,
        store,
        deltas,
        out,
        clock,
    }
}

impl Fixture {
    fn thread(&self, first_message_id: &str) -> Thread {
        let mut t = Thread::create(first_message_id, ACCOUNT, "subject", 0);
        self.store.save(&mut t).unwrap();
        t
    }

    fn message(&self, id: &str) -> Message {
        let mut m = Message::new(json!({
            "id": id, "aid": ACCOUNT, "v": 0, "hMsgId": format!("{id}@x"), "threadId": "",
            "subject": "s", "date": T0, "folders": [], "labels": [],
        }));
        self.store.save(&mut m).unwrap();
        m
    }

    fn set_metadata<M: Model>(&self, model: &mut M, plugin: &str, value: Value) {
        model.upsert_metadata(plugin, &value, None);
        self.store.save(model).unwrap();
    }

    fn scanner(&self) -> MetadataExpirationWorker<'_> {
        MetadataExpirationWorker::new(
            ACCOUNT,
            &self.store,
            self.deltas.clone(),
            self.clock.clone(),
        )
        .with_chunk_pause(Duration::ZERO)
    }

    /// The `metadata-expiration` deltas written since the last call, as (class, id).
    fn expirations(&self) -> Vec<(String, String)> {
        self.deltas.flush();
        self.out
            .deltas()
            .into_iter()
            .filter(|d| d["type"] == "metadata-expiration")
            .flat_map(|d| {
                let class = d["modelClass"].as_str().unwrap().to_owned();
                d["modelJSONs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|j| (class.clone(), j["id"].as_str().unwrap().to_owned()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn count(&self, sql: &str) -> i64 {
        self.store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

#[test]
fn scan_emits_due_models_until_the_metadata_is_cleared() {
    let f = fixture();
    let mut snoozed = f.thread("m1");
    let mut later = f.thread("m2");
    let mut tracked = f.message("m3");
    let mut scheduled = f.message("m4");
    f.set_metadata(
        &mut snoozed,
        "thread-snooze",
        json!({"expiration": T0 + 10}),
    );
    f.set_metadata(&mut later, "thread-snooze", json!({"expiration": T0 + 500}));
    f.set_metadata(&mut tracked, "open-tracking", json!({"uid": "u"}));
    f.set_metadata(
        &mut scheduled,
        "send-later",
        json!({"expiration": (T0 + 10) as f64}),
    );
    f.expirations();

    // Nothing due: the next expiration is 10 s away, above the 5 s floor.
    assert_eq!(f.scanner().scan().unwrap(), T0 + 10);
    assert!(f.expirations().is_empty());

    f.clock.set(T0 + 10);
    // Due: both kinds go out; the next wake is the remaining expiration.
    assert_eq!(f.scanner().scan().unwrap(), T0 + 500);
    let mut sent = f.expirations();
    sent.sort();
    assert_eq!(
        sent,
        [
            ("Message".to_owned(), scheduled.id().to_owned()),
            ("Thread".to_owned(), snoozed.id().to_owned()),
        ]
    );

    // Not cleared yet: sent again. 15 s after emitting is the soonest the next scan runs.
    f.clock.set(T0 + 488);
    assert_eq!(f.scanner().scan().unwrap(), T0 + 503);
    assert_eq!(f.expirations().len(), 2);

    // The client clears both, one with `{}` and one with a null expiration.
    let mut snoozed = f
        .store
        .find::<Thread>(&Query::new().equal("id", snoozed.id()))
        .unwrap()
        .unwrap();
    f.set_metadata(&mut snoozed, "thread-snooze", json!({}));
    let mut scheduled = f
        .store
        .find::<Message>(&Query::new().equal("id", scheduled.id()))
        .unwrap()
        .unwrap();
    f.set_metadata(&mut scheduled, "send-later", json!({"expiration": null}));
    f.expirations();
    f.clock.set(T0 + 489);
    assert_eq!(f.scanner().scan().unwrap(), T0 + 500);
    assert!(f.expirations().is_empty());
    // Nothing emitted, so the floor is 5 s.
    f.clock.set(T0 + 498);
    assert_eq!(f.scanner().scan().unwrap(), T0 + 503);

    // With nothing left after emitting, the worker sleeps two hours.
    f.clock.set(T0 + 600);
    assert_eq!(f.scanner().scan().unwrap(), T0 + 600 + 2 * 60 * 60);
    assert_eq!(
        f.expirations(),
        [("Thread".to_owned(), later.id().to_owned())]
    );
    let mut later = f
        .store
        .find::<Thread>(&Query::new().equal("id", later.id()))
        .unwrap()
        .unwrap();
    f.set_metadata(&mut later, "thread-snooze", json!({}));
    assert_eq!(f.scanner().scan().unwrap(), T0 + 600 + 2 * 60 * 60);
    assert_eq!(
        f.count("SELECT count(*) FROM ModelPluginMetadata WHERE expiration IS NOT NULL"),
        0
    );
}

#[test]
fn scan_reads_only_its_account_and_survives_a_damaged_row() {
    let f = fixture();
    let mut t = f.thread("m1");
    f.set_metadata(&mut t, "thread-snooze", json!({"expiration": T0 - 1}));
    f.store
        .conn()
        .execute_batch(&format!(
            "INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) VALUES
               ('x', 'other', 'Thread', 'thread-snooze', {T0}),
               ('y', '{ACCOUNT}', 'Folder', 'p', {T0})"
        ))
        .unwrap();
    f.expirations();
    f.scanner().scan().unwrap();
    assert_eq!(f.expirations(), [("Thread".to_owned(), t.id().to_owned())]);
}

#[test]
fn metadata_for_a_missing_model_waits_and_attaches_on_first_save() {
    let f = fixture();
    let pending = Metadata {
        object_type: "Message".into(),
        object_id: "later".into(),
        account_id: ACCOUNT.into(),
        plugin_id: "send-reminders".into(),
        version: 4,
        value: json!({"expiration": T0 + 100}),
    };
    apply_metadata(&f.store, &pending).unwrap();
    assert_eq!(f.count("SELECT count(*) FROM DetatchedPluginMetadata"), 1);

    let m = f.message("later");
    assert_eq!(m.json()["metadata"][0]["pluginId"], "send-reminders");
    assert_eq!(m.json()["metadata"][0]["v"], 4);
    assert_eq!(f.count("SELECT count(*) FROM DetatchedPluginMetadata"), 0);
    let expiration: i64 = f
        .store
        .conn()
        .query_row(
            "SELECT expiration FROM ModelPluginMetadata WHERE id = 'later'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(expiration, T0 + 100);

    // An older or equal version is ignored; a newer one replaces the value.
    apply_metadata(
        &f.store,
        &Metadata {
            version: 4,
            value: json!({}),
            ..pending.clone()
        },
    )
    .unwrap();
    let stored = |f: &Fixture| {
        f.store
            .find::<Message>(&Query::new().equal("id", "later"))
            .unwrap()
            .unwrap()
            .json()["metadata"][0]
            .clone()
    };
    assert_eq!(stored(&f)["value"]["expiration"], T0 + 100);
    apply_metadata(
        &f.store,
        &Metadata {
            version: 5,
            value: json!({}),
            ..pending.clone()
        },
    )
    .unwrap();
    assert_eq!(stored(&f)["v"], 5);
    assert_eq!(f.count("SELECT count(*) FROM ModelPluginMetadata"), 0);

    // A class findGeneric does not know is dropped, not fatal.
    apply_metadata(
        &f.store,
        &Metadata {
            object_type: "Folder".into(),
            ..pending
        },
    )
    .unwrap();
}

/// A backend that serves scripted pulls and records pushes.
#[derive(Default)]
struct FakeBackend {
    pulls: Mutex<VecDeque<(Vec<MetadataDelta>, Result<String>)>>,
    cursors_seen: Mutex<Vec<Option<String>>>,
    pushes: Mutex<Vec<MetadataPush>>,
}

impl MetadataBackend for FakeBackend {
    fn push(&self, push: &MetadataPush) -> Result<()> {
        self.pushes.lock().unwrap().push(push.clone());
        Ok(())
    }

    fn pull(&self, cursor: Option<&str>, sink: &mut dyn FnMut(MetadataDelta)) -> Result<String> {
        self.cursors_seen
            .lock()
            .unwrap()
            .push(cursor.map(str::to_owned));
        let (deltas, result) = self
            .pulls
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted pull");
        for d in deltas {
            sink(d);
        }
        result
    }
}

#[test]
fn pull_applies_deltas_and_resumes_from_the_saved_cursor() {
    let f = fixture();
    let t = f.thread("m1");
    let metadata = |id: &str, class: &str, v: i64| {
        MetadataDelta::Metadata(Metadata {
            object_type: class.into(),
            object_id: id.into(),
            account_id: ACCOUNT.into(),
            plugin_id: "thread-snooze".into(),
            version: v,
            value: json!({"expiration": T0 + 30}),
        })
    };
    let backend = Arc::new(FakeBackend::default());
    backend.pulls.lock().unwrap().extend([
        (
            vec![
                MetadataDelta::Cursor("head".into()),
                metadata(t.id(), "Thread", 3),
                metadata("not-yet", "Message", 1),
                MetadataDelta::IdentityRefreshNeeded,
            ],
            Ok("c1".into()),
        ),
        (
            vec![MetadataDelta::Cursor("c2".into())],
            Err(ms_core::SyncError::offline("pull", "stream dropped").into()),
        ),
    ]);
    let worker = MetadataWorker::new(ACCOUNT, &f.store, backend.clone());
    assert_eq!(worker.cursor().unwrap(), None);
    worker.pull_once().unwrap();
    assert_eq!(worker.cursor().unwrap().as_deref(), Some("c1"));

    let t = f
        .store
        .find::<Thread>(&Query::new().equal("id", t.id()))
        .unwrap()
        .unwrap();
    assert_eq!(t.json()["metadata"][0]["v"], 3);
    assert_eq!(
        f.count("SELECT count(*) FROM ModelPluginMetadata WHERE expiration IS NOT NULL"),
        1
    );
    assert_eq!(
        f.count("SELECT count(*) FROM DetatchedPluginMetadata WHERE objectId = 'not-yet'"),
        1
    );
    f.deltas.flush();
    assert!(f
        .out
        .deltas()
        .iter()
        .any(|d| d["modelClass"] == "ProcessIdentityRefreshNeeded"));

    // A dropped stream keeps the cursor reached before it dropped.
    let err = worker.pull_once().unwrap_err();
    assert!(err.is_retryable());
    assert_eq!(worker.cursor().unwrap().as_deref(), Some("c2"));
    assert_eq!(
        *backend.cursors_seen.lock().unwrap(),
        [None, Some("c1".to_owned())]
    );
}
