// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A non-local `MetadataBackend` is reached from both ends without touching the workers or
//! tasks: SyncbackMetadataTask's remote phase pushes to it (`performRemoteSyncbackMetadata`)
//! and the `metadata` thread pulls from it (`MetadataWorker::run`). The local-only default
//! does neither.

use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use ms_core::{Account, MailError, ManualClock, MessageFlags, Metadata, UidSet};
use ms_imap::{
    Capabilities, FetchKind, FetchResult, FolderStatus, IdleInterrupt, ImapAuth, ImapSession,
    ListedFolder, Namespace, SelectInfo, StoreOp, SyncResult, UidMapping,
};
use ms_net::{LocalOnly, MetadataBackend, MetadataDelta, MetadataPush, TokenManager};
use ms_providers::ServerProfile;
use ms_store::{DeltaStream, Model, Query, Store, Task, Thread};
use ms_sync::{ForegroundControl, WorkerContext};
use serde_json::json;

const ACCOUNT: &str = "acc1";
const T0: i64 = 1_700_000_000;

/// Records pushes; the first pull delivers `first_pull` and returns `"c1"`, later pulls
/// report their cursor and block, as a stream with nothing new does.
struct FakeBackend {
    pushes: Mutex<Vec<MetadataPush>>,
    first_pull: Mutex<Option<Vec<MetadataDelta>>>,
    later_pulls: Mutex<mpsc::Sender<Option<String>>>,
}

impl MetadataBackend for FakeBackend {
    fn push(&self, push: &MetadataPush) -> ms_core::Result<()> {
        self.pushes.lock().unwrap().push(push.clone());
        Ok(())
    }

    fn pull(
        &self,
        cursor: Option<&str>,
        sink: &mut dyn FnMut(MetadataDelta),
    ) -> ms_core::Result<String> {
        if let Some(deltas) = self.first_pull.lock().unwrap().take() {
            for d in deltas {
                sink(d);
            }
            return Ok("c1".into());
        }
        let _ = self
            .later_pulls
            .lock()
            .unwrap()
            .send(cursor.map(str::to_owned));
        loop {
            std::thread::park();
        }
    }
}

fn context(dir: &std::path::Path, backend: Arc<dyn MetadataBackend>) -> WorkerContext {
    let account = Account::from_json(json!({
        "id": ACCOUNT,
        "provider": "imap",
        "emailAddress": "me@example.com",
        "settings": {},
    }))
    .unwrap();
    WorkerContext {
        account: Arc::new(account),
        config_dir: dir.to_owned(),
        deltas: DeltaStream::new(Box::new(std::io::sink())),
        clock: Arc::new(ManualClock::new(T0)),
        verbose: false,
        foreground: Arc::new(ForegroundControl::new()),
        background_mark_all: Arc::new(AtomicBool::new(false)),
        tokens: Arc::new(TokenManager::new(None)),
        metadata: backend,
        tasks: Arc::new(ms_tasks::Tasks),
    }
}

fn migrated() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ms_store::migrate::migrate(dir.path(), &ManualClock::new(T0), &mut Vec::new()).unwrap();
    dir
}

fn syncback_metadata(store: &Store, ctx: &WorkerContext, thread: &Thread) -> Task {
    let mut task = Task::create(
        "SyncbackMetadataTask",
        ACCOUNT,
        &json!({
            "modelId": thread.id(),
            "modelClassName": "Thread",
            "modelHeaderMessageId": "first@x",
            "pluginId": "thread-snooze",
            "value": {"expiration": T0 + 60},
        }),
    );
    ms_tasks::TaskProcessor::for_context(ctx, store)
        .perform_local(&mut task)
        .unwrap();
    assert_eq!(task.status(), "remote");
    ctx.tasks
        .remote_runner(store, ctx)
        .perform_remote(&mut task, &mut NoImap::default())
        .unwrap();
    task
}

#[test]
fn syncback_metadata_pushes_to_a_remote_backend() {
    let dir = migrated();
    let (tx, _rx) = mpsc::channel();
    let backend = Arc::new(FakeBackend {
        pushes: Mutex::new(Vec::new()),
        first_pull: Mutex::new(None),
        later_pulls: Mutex::new(tx),
    });
    let ctx = context(dir.path(), backend.clone());
    let store = Store::open(dir.path(), ctx.deltas.clone()).unwrap();
    let mut thread = Thread::create("first", ACCOUNT, "s", 0);
    store.save(&mut thread).unwrap();

    let task = syncback_metadata(&store, &ctx, &thread);
    assert_eq!(task.status(), "complete");
    assert!(task.json().get("error").is_none_or(|e| e.is_null()));
    assert_eq!(
        *backend.pushes.lock().unwrap(),
        [MetadataPush {
            account_id: ACCOUNT.into(),
            model_id: thread.id().into(),
            model_class: "Thread".into(),
            plugin_id: "thread-snooze".into(),
            value: json!({"expiration": T0 + 60}),
            version: 1,
            header_message_id: Some("first@x".into()),
        }]
    );
}

#[test]
fn local_only_backend_neither_pushes_nor_pulls() {
    let dir = migrated();
    let ctx = context(dir.path(), Arc::new(LocalOnly));
    let store = Store::open(dir.path(), ctx.deltas.clone()).unwrap();
    let mut thread = Thread::create("first", ACCOUNT, "s", 0);
    store.save(&mut thread).unwrap();
    assert_eq!(
        syncback_metadata(&store, &ctx, &thread).status(),
        "complete"
    );
    // Returns at once: the bin does not start the thread for LocalOnly either.
    ms_sync::metadata_worker::run_metadata_worker(ctx);
}

#[test]
fn metadata_thread_pulls_into_the_store_and_keeps_the_cursor() {
    let dir = migrated();
    let (tx, rx) = mpsc::channel();
    let metadata = |id: &str, class: &str| Metadata {
        object_type: class.into(),
        object_id: id.into(),
        account_id: ACCOUNT.into(),
        plugin_id: "thread-snooze".into(),
        version: 7,
        value: json!({"expiration": T0 + 60}),
    };
    let store = {
        let deltas = DeltaStream::new(Box::new(std::io::sink()));
        Store::open(dir.path(), deltas).unwrap()
    };
    let mut thread = Thread::create("first", ACCOUNT, "s", 0);
    store.save(&mut thread).unwrap();
    let backend = Arc::new(FakeBackend {
        pushes: Mutex::new(Vec::new()),
        first_pull: Mutex::new(Some(vec![
            MetadataDelta::Cursor("head".into()),
            MetadataDelta::Metadata(metadata(thread.id(), "Thread")),
            MetadataDelta::Metadata(metadata("arrives-later", "Message")),
        ])),
        later_pulls: Mutex::new(tx),
    });
    let ctx = context(dir.path(), backend);
    std::thread::Builder::new()
        .name(ms_core::constants::thread::METADATA.into())
        .spawn(move || ms_sync::metadata_worker::run_metadata_worker(ctx))
        .unwrap();

    let resumed_from = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(resumed_from.as_deref(), Some("c1"));
    assert_eq!(
        store
            .get_key_value(&format!("cursor-{ACCOUNT}"))
            .unwrap()
            .as_deref(),
        Some("c1")
    );
    let thread = store
        .find::<Thread>(&Query::new().equal("id", thread.id()))
        .unwrap()
        .unwrap();
    assert_eq!(thread.json()["metadata"][0]["v"], 7);
    let (pending, indexed): (i64, i64) = store
        .conn()
        .query_row(
            "SELECT (SELECT count(*) FROM DetatchedPluginMetadata WHERE objectId = 'arrives-later'),
                    (SELECT count(*) FROM ModelPluginMetadata WHERE expiration = ?)",
            [T0 + 60],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((pending, indexed), (1, 1));
}

/// SyncbackMetadataTask never talks IMAP.
#[derive(Default)]
struct NoImap {
    caps: Capabilities,
    profile: ServerProfile,
}

fn unused<T>() -> ms_imap::Result<T> {
    Err(MailError::new(
        ms_core::ErrorCode::Connection,
        "this task does not use IMAP",
    ))
}

impl ImapSession for NoImap {
    fn connect_if_needed(&mut self) -> ms_imap::Result<()> {
        unused()
    }
    fn login_if_needed(&mut self) -> ms_imap::Result<()> {
        unused()
    }
    fn is_disconnected(&self) -> bool {
        true
    }
    fn disconnect(&mut self) {}
    fn set_auth(&mut self, _: ImapAuth) {}
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn profile(&self) -> &ServerProfile {
        &self.profile
    }
    fn default_namespace(&self) -> Option<&Namespace> {
        None
    }
    fn is_qresync_enabled(&self) -> bool {
        false
    }
    fn message_limit(&self) -> Option<u32> {
        None
    }
    fn current_folder(&self) -> Option<&str> {
        None
    }
    fn last_tls_error(&self) -> Option<&str> {
        None
    }
    fn list_folders(&mut self) -> ms_imap::Result<Vec<ListedFolder>> {
        unused()
    }
    fn select(&mut self, _: &str) -> ms_imap::Result<SelectInfo> {
        unused()
    }
    fn select_if_needed(&mut self, _: &str) -> ms_imap::Result<()> {
        unused()
    }
    fn noop(&mut self) -> ms_imap::Result<()> {
        unused()
    }
    fn status(&mut self, _: &str) -> ms_imap::Result<FolderStatus> {
        unused()
    }
    fn create_folder(&mut self, _: &str) -> ms_imap::Result<()> {
        unused()
    }
    fn rename_folder(&mut self, _: &str, _: &str) -> ms_imap::Result<()> {
        unused()
    }
    fn delete_folder(&mut self, _: &str) -> ms_imap::Result<()> {
        unused()
    }
    fn fetch_by_uid(&mut self, _: &str, _: FetchKind, _: &UidSet) -> ms_imap::Result<FetchResult> {
        unused()
    }
    fn sync_by_uid(
        &mut self,
        _: &str,
        _: FetchKind,
        _: &UidSet,
        _: u64,
    ) -> ms_imap::Result<SyncResult> {
        unused()
    }
    fn take_vanished(&mut self, _: &str) -> UidSet {
        UidSet::default()
    }
    fn fetch_message_bytes(&mut self, _: &str, _: u32) -> ms_imap::Result<Option<Vec<u8>>> {
        unused()
    }
    fn store_flags(
        &mut self,
        _: &str,
        _: &UidSet,
        _: StoreOp,
        _: MessageFlags,
    ) -> ms_imap::Result<()> {
        unused()
    }
    fn store_labels(
        &mut self,
        _: &str,
        _: &UidSet,
        _: StoreOp,
        _: &[String],
    ) -> ms_imap::Result<()> {
        unused()
    }
    fn move_uids(&mut self, _: &str, _: &UidSet, _: &str) -> ms_imap::Result<UidMapping> {
        unused()
    }
    fn copy_uids(&mut self, _: &str, _: &UidSet, _: &str) -> ms_imap::Result<UidMapping> {
        unused()
    }
    fn expunge(&mut self, _: &str) -> ms_imap::Result<()> {
        unused()
    }
    fn expunge_uids(&mut self, _: &str, _: &UidSet) -> ms_imap::Result<()> {
        unused()
    }
    fn append(&mut self, _: &str, _: &[u8], _: MessageFlags) -> ms_imap::Result<Option<u32>> {
        unused()
    }
    fn find_uids_of_recent_header_message_id(
        &mut self,
        _: &str,
        _: &str,
    ) -> ms_imap::Result<Vec<u32>> {
        unused()
    }
    fn supports_idle(&self) -> bool {
        false
    }
    fn idle(&mut self, _: &str, _: &IdleInterrupt) -> ms_imap::Result<()> {
        unused()
    }
}
