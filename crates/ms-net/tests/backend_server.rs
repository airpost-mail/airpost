// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! The engine's `HttpMetadataBackend` (ms-net) against a real server (docs/backend-plan.md
//! 6.1, B1-E): push, stream from `0`, the outbox across a server restart, backfill, the
//! binding after a server id rotation, the thread anchor echo and 401 dormancy.
//!
//! It lives in ms-net so the MIT server crate has no GPL dependency, even a dev one. The
//! blocking reqwest client must not run inside a tokio runtime, so each server runs on its
//! own thread with its own runtime.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use ms_net::{HttpMetadataBackend, MetadataBackend, MetadataDelta, MetadataPush, TernServer};
use serde_json::json;
use tern_backend::config::{Config, FeatureSwitches, IpNet, Signup};
use tern_backend::http::Server;
use tern_backend::repo::{Repo, SqliteRepo, User};

const AID: &str = "acc1";

/// A server on its own runtime thread, restartable on the same port and data dir.
struct Backend {
    data_dir: PathBuf,
    addr: SocketAddr,
    repo: Arc<SqliteRepo>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Backend {
    fn start(data_dir: &Path, bind: SocketAddr) -> Backend {
        let repo = Arc::new(SqliteRepo::open(data_dir).unwrap());
        let config = Config {
            data_dir: data_dir.to_owned(),
            bind,
            public_url: Some("http://127.0.0.1".to_owned()),
            signup: Signup::Closed,
            features: FeatureSwitches::default(),
            trusted_proxies: vec![IpNet::parse("127.0.0.0/8").unwrap()],
        };
        let (addr_tx, addr_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server_repo: Arc<dyn Repo> = repo.clone();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let server = Server::bind(&config, server_repo).await.unwrap();
                addr_tx.send(server.local_addr().unwrap()).unwrap();
                server
                    .run(async {
                        let _ = stop_rx.await;
                    })
                    .await
                    .unwrap();
            });
        });
        let addr = addr_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Backend {
            data_dir: data_dir.to_owned(),
            addr,
            repo,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }

    fn restart(mut self) -> Backend {
        self.stop();
        Backend::start(&self.data_dir.clone(), self.addr)
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn user(&self, email: &str) -> User {
        let new = tern_backend::auth::new_user(email, "Ada", "Lovelace", "correct horse").unwrap();
        self.repo.create_user(&new, 1).unwrap()
    }

    fn token(&self, user: &User) -> String {
        tern_backend::auth::issue_token(self.repo.as_ref(), user, "device", 1).unwrap()
    }

    fn server_object(&self, user: &User, token: &str) -> TernServer {
        TernServer::from_json(&json!({
            "type": "tern-server", "version": 1, "url": self.url(), "token": token,
            "userId": user.id, "serverId": self.repo.server_id().unwrap(),
        }))
        .unwrap()
    }

    fn rows(&self, user: &User) -> Vec<tern_backend::wire::MetadataRecord> {
        self.repo.metadata_page(&user.id, AID, 500, 0).unwrap()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

/// An engine's backend with its own config dir, counting `ProcessIdentityRefreshNeeded`.
struct Device {
    backend: Arc<HttpMetadataBackend>,
    refreshes: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

fn device(server: TernServer) -> Device {
    let dir = tempfile::tempdir().unwrap();
    let refreshes = Arc::new(AtomicUsize::new(0));
    let r = refreshes.clone();
    let backend = HttpMetadataBackend::new(
        server,
        AID,
        dir.path(),
        Box::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();
    Device {
        backend: Arc::new(backend),
        refreshes,
        _dir: dir,
    }
}

fn push(class: &str, id: &str, plugin: &str, value: serde_json::Value, v: i64) -> MetadataPush {
    MetadataPush {
        account_id: AID.into(),
        model_id: id.into(),
        model_class: class.into(),
        plugin_id: plugin.into(),
        value,
        version: v,
        header_message_id: None,
    }
}

/// Pulls on a thread; deltas arrive on the returned channel, the pull's result on join.
fn pull_in_background(
    backend: Arc<HttpMetadataBackend>,
    cursor: Option<String>,
) -> (
    mpsc::Receiver<MetadataDelta>,
    std::thread::JoinHandle<ms_core::Result<String>>,
) {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        backend.pull(cursor.as_deref(), &mut |d| {
            let _ = tx.send(d);
        })
    });
    (rx, handle)
}

fn recv(rx: &mpsc::Receiver<MetadataDelta>) -> MetadataDelta {
    rx.recv_timeout(Duration::from_secs(10))
        .expect("a delta within 10 s")
}

#[test]
fn push_then_stream_from_zero_with_the_thread_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Backend::start(dir.path(), "127.0.0.1:0".parse().unwrap());
    let user = b.user("ada@example.com");
    let laptop = device(b.server_object(&user, &b.token(&user)));
    let phone = device(b.server_object(&user, &b.token(&user)));

    let mut snooze = push(
        "Thread",
        "t:abc",
        "thread-snooze",
        json!({"expiration": 9}),
        1,
    );
    snooze.header_message_id = Some("first@example.com".into());
    laptop.backend.push(&snooze).unwrap();
    laptop
        .backend
        .push(&push(
            "Message",
            "m1",
            "open-tracking",
            json!({"open_count": 0}),
            0,
        ))
        .unwrap();
    assert!(laptop.backend.outbox().is_empty(AID).unwrap());
    let rows = b.rows(&user);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].header_message_id.as_deref(),
        Some("first@example.com")
    );

    let expected = format!("{}:{}", b.repo.server_id().unwrap(), user.id);
    assert_eq!(phone.backend.binding().unwrap(), Some(expected));
    assert_eq!(phone.refreshes.load(Ordering::SeqCst), 0);

    let (rx, pull) = pull_in_background(phone.backend.clone(), None);
    match recv(&rx) {
        MetadataDelta::AnchoredThreadMetadata(m, anchor) => {
            assert_eq!(anchor, "first@example.com");
            assert_eq!(
                (m.object_type.as_str(), m.object_id.as_str(), m.version),
                ("thread", "t:abc", 1)
            );
            assert_eq!(m.value, json!({"expiration": 9}));
        }
        other => panic!("{other:?}"),
    }
    let MetadataDelta::Cursor(_) = recv(&rx) else {
        panic!("cursor after the record")
    };
    match recv(&rx) {
        MetadataDelta::Metadata(m) => {
            assert_eq!(
                (m.object_type.as_str(), m.plugin_id.as_str()),
                ("message", "open-tracking")
            );
        }
        other => panic!("{other:?}"),
    }
    let MetadataDelta::Cursor(last) = recv(&rx) else {
        panic!("cursor after the record")
    };

    // A later push from the laptop reaches the open stream.
    laptop
        .backend
        .push(&push("thread", "t:abc", "thread-snooze", json!({}), 2))
        .unwrap();
    let MetadataDelta::AnchoredThreadMetadata(m, _) = recv(&rx) else {
        panic!("the update")
    };
    assert_eq!((m.version, m.value), (2, json!({})));
    let MetadataDelta::Cursor(newest) = recv(&rx) else {
        panic!("cursor")
    };
    assert_ne!(newest, last);

    // Server shutdown ends the stream cleanly; the pull returns the last cursor.
    b.stop();
    assert_eq!(pull.join().unwrap().unwrap(), newest);
}

#[test]
fn pushes_during_an_outage_wait_in_the_outbox() {
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::start(dir.path(), "127.0.0.1:0".parse().unwrap());
    let user = b.user("ada@example.com");
    let laptop = device(b.server_object(&user, &b.token(&user)));
    let mut b = b;
    b.stop();

    let p = push(
        "Thread",
        "t:1",
        "send-reminders",
        json!({"expiration": 5}),
        3,
    );
    laptop.backend.push(&p).unwrap();
    laptop
        .backend
        .push(&push(
            "Thread",
            "t:1",
            "send-reminders",
            json!({"expiration": 6}),
            4,
        ))
        .unwrap();
    assert_eq!(
        laptop.backend.outbox().len(AID).unwrap(),
        1,
        "latest per key"
    );
    assert!(laptop.backend.binding().unwrap_err().is_retryable());
    assert!(laptop.backend.drain_outbox().unwrap_err().is_retryable());
    assert!(laptop
        .backend
        .pull(None, &mut |_| {})
        .unwrap_err()
        .is_retryable());

    let b = b.restart();
    laptop.backend.binding().unwrap();
    laptop.backend.drain_outbox().unwrap();
    assert!(laptop.backend.outbox().is_empty(AID).unwrap());
    let rows = b.rows(&user);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].v, &rows[0].value), (4, &json!({"expiration": 6})));
}

#[test]
fn a_direct_push_supersedes_the_queued_value_for_its_key() {
    // The server keeps the last value written, so the stale queued value must not be sent
    // after the newer one that went straight through.
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::start(dir.path(), "127.0.0.1:0".parse().unwrap());
    let user = b.user("ada@example.com");
    let laptop = device(b.server_object(&user, &b.token(&user)));
    let mut b = b;
    b.stop();
    let old = push(
        "Thread",
        "t:1",
        "thread-snooze",
        json!({"expiration": 1}),
        1,
    );
    let other = push(
        "Thread",
        "t:2",
        "thread-snooze",
        json!({"expiration": 2}),
        1,
    );
    laptop.backend.push(&old).unwrap();
    laptop.backend.push(&other).unwrap();
    assert_eq!(laptop.backend.outbox().len(AID).unwrap(), 2);

    let b = b.restart();
    let new = push(
        "Thread",
        "t:1",
        "thread-snooze",
        json!({"expiration": 9}),
        2,
    );
    laptop.backend.push(&new).unwrap();
    assert_eq!(laptop.backend.outbox().len(AID).unwrap(), 1, "t:1 dropped");
    laptop.backend.drain_outbox().unwrap();
    assert!(laptop.backend.outbox().is_empty(AID).unwrap());
    let mut rows = b.rows(&user);
    rows.sort_by(|a, b| a.object_id.cmp(&b.object_id));
    let values: Vec<_> = rows
        .iter()
        .map(|r| (r.object_id.as_str(), &r.value))
        .collect();
    assert_eq!(
        values,
        [
            ("t:1", &json!({"expiration": 9})),
            ("t:2", &json!({"expiration": 2}))
        ]
    );
}

#[test]
fn backfill_keeps_server_values_and_adds_missing_ones() {
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::start(dir.path(), "127.0.0.1:0".parse().unwrap());
    let user = b.user("ada@example.com");
    let laptop = device(b.server_object(&user, &b.token(&user)));
    let phone = device(b.server_object(&user, &b.token(&user)));
    laptop
        .backend
        .push(&push(
            "Thread",
            "t:1",
            "thread-snooze",
            json!({"expiration": 1}),
            3,
        ))
        .unwrap();
    // Stale local state on the phone, plus something only it has, and one invalid value.
    phone
        .backend
        .outbox()
        .put(&push(
            "Thread",
            "t:old",
            "thread-snooze",
            json!({"x": 1}),
            1,
        ))
        .unwrap();
    let mut huge = push("Message", "m2", "p", json!("x".repeat(70 * 1024)), 1);
    huge.header_message_id = Some("m2@x".into());
    phone
        .backend
        .rebind(&[
            push(
                "Thread",
                "t:1",
                "thread-snooze",
                json!({"expiration": 99}),
                1,
            ),
            push("Contact", "c1", "notes", json!({"n": "hi"}), 2),
            huge,
        ])
        .unwrap();
    assert_eq!(
        phone.backend.outbox().len(AID).unwrap(),
        3,
        "old pushes dropped"
    );
    phone.backend.drain_outbox().unwrap();
    assert!(
        phone.backend.outbox().is_empty(AID).unwrap(),
        "rejected ones dropped"
    );

    let rows = b.rows(&user);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].value, json!({"expiration": 1}), "server value kept");
    assert_eq!(rows[1].object_id, "c1");
}

#[test]
fn binding_follows_the_server_after_a_restore() {
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::start(dir.path(), "127.0.0.1:0".parse().unwrap());
    let user = b.user("ada@example.com");
    let laptop = device(b.server_object(&user, &b.token(&user)));
    let first = laptop.backend.binding().unwrap().unwrap();

    let rotated = b.repo.rotate_server_id().unwrap();
    let b = b.restart();
    let second = laptop.backend.binding().unwrap().unwrap();
    assert_eq!(second, format!("{rotated}:{}", user.id));
    assert_ne!(first, second);
    // The client's copy is stale: told once, however often the binding is read.
    laptop.backend.binding().unwrap();
    assert_eq!(laptop.refreshes.load(Ordering::SeqCst), 1);
    drop(b);
}

#[test]
fn a_revoked_token_makes_the_backend_dormant() {
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::start(dir.path(), "127.0.0.1:0".parse().unwrap());
    let user = b.user("ada@example.com");
    let laptop = device(b.server_object(&user, &b.token(&user)));
    laptop.backend.binding().unwrap();
    let id = b.repo.list_tokens(Some(&user.id)).unwrap()[0].id;
    b.repo.revoke_token(id, 2).unwrap();

    let p = push(
        "Thread",
        "t:1",
        "thread-snooze",
        json!({"expiration": 1}),
        1,
    );
    laptop.backend.push(&p).unwrap();
    assert!(laptop.backend.is_dormant());
    assert_eq!(laptop.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(laptop.backend.outbox().len(AID).unwrap(), 1);

    // No more requests: everything fails retryably, from the dormant flag alone, even once
    // the server is gone.
    let mut b = b;
    b.stop();
    for e in [
        laptop.backend.binding().unwrap_err(),
        laptop.backend.drain_outbox().unwrap_err(),
        laptop.backend.pull(Some("5"), &mut |_| {}).unwrap_err(),
    ] {
        assert!(e.is_retryable(), "{e}");
        assert!(e.to_string().contains("rejected"), "{e}");
    }
    laptop.backend.push(&p).unwrap();
    assert_eq!(laptop.refreshes.load(Ordering::SeqCst), 1, "told once");
    assert_eq!(laptop.backend.outbox().len(AID).unwrap(), 1);
}

#[test]
fn a_server_under_a_path_prefix_is_unsupported_but_harmless() {
    // The base URL may carry a path prefix; requests go under it. Against a server mounted
    // at `/`, a prefixed URL gets 404s, which must never abort the engine.
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::start(dir.path(), "127.0.0.1:0".parse().unwrap());
    let user = b.user("ada@example.com");
    let token = b.token(&user);
    let mut server = b.server_object(&user, &token);
    server.url = format!("{}/prefix", b.url());
    let laptop = device(server);
    let e = laptop.backend.binding().unwrap_err();
    assert!(e.is_retryable(), "{e}");
    assert!(laptop
        .backend
        .pull(None, &mut |_| {})
        .unwrap_err()
        .is_retryable());
    assert!(!laptop.backend.is_dormant());
}
