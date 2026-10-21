// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! GoogleContactsWorker against a canned People API: the full listing, the incremental pass
//! that follows it, tombstones, photo-only entries, system groups and expired sync tokens.
//! `GoogleContactsWorker.cpp` is the reference; every case ends in `invariants::check`.

#[path = "../../tern-net/tests/common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{MockServer, Reply, Request};
use serde_json::{json, Value};
use tern_core::{Account, ManualClock};
use tern_net::{Endpoints, TokenManager};
use tern_store::{Contact, ContactBook, ContactGroup, DeltaStream, Model, Query, Store};
use tern_sync::google_contacts::{
    contacts_sync_token_key, groups_sync_token_key, GoogleContactsWorker,
};

const ACCOUNT: &str = "acc1";

fn account() -> Arc<Account> {
    Arc::new(
        Account::from_json(json!({
            "id": ACCOUNT, "provider": "gmail", "emailAddress": "me@gmail.test",
            "settings": {"refresh_token": "r1", "refresh_client_id": "cid"},
        }))
        .unwrap(),
    )
}

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    tern_store::migrate::migrate(
        dir.path(),
        &ManualClock::new(1_700_000_000),
        &mut Vec::new(),
    )
    .unwrap();
    let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    (dir, store)
}

fn tokens(server: &MockServer) -> Arc<TokenManager> {
    Arc::new(TokenManager::new(None).with_endpoints(Endpoints::all_at(&server.url())))
}

fn token_reply() -> Reply {
    Reply::json(200, json!({"access_token": "atok", "expires_in": 3600}))
}

fn person(id: &str, name: &str, email: &str) -> Value {
    json!({
        "resourceName": format!("people/{id}"), "etag": format!("e-{id}"),
        "metadata": {"sources": [{"type": "CONTACT", "id": id, "etag": format!("e-{id}")}]},
        "names": [{"displayName": name}],
        "emailAddresses": [{"value": email}],
    })
}

fn tombstone(id: &str) -> Value {
    json!({"resourceName": format!("people/{id}"), "etag": "x", "metadata": {"deleted": true}})
}

fn group(id: &str, name: &str, system: bool, members: u32) -> Value {
    json!({
        "resourceName": format!("contactGroups/{id}"), "etag": "g", "name": name,
        "groupType": if system { "SYSTEM_CONTACT_GROUP" } else { "USER_CONTACT_GROUP" },
        "memberCount": members,
    })
}

/// The account's People API: a full listing without a sync token, then one change set.
fn people_api(req: &Request) -> Reply {
    if req.path() == "/google/token" {
        return token_reply();
    }
    let sync = req.query_value("syncToken");
    match (req.path(), sync.as_deref()) {
        ("/v1/people/me/connections", None) => match req.query_value("pageToken").as_deref() {
            None => Reply::json(
                200,
                json!({
                    "connections": [person("c1", "Ann Archer", "ann@x.test"), person("c2", "Bob Baker", "bob@x.test")],
                    "nextPageToken": "p2",
                }),
            ),
            Some(_) => Reply::json(
                200,
                json!({
                    "connections": [
                        {"resourceName": "people/c3", "etag": "e3", "photos": [{"url": "u"}]},
                        tombstone("c4"),
                    ],
                    "nextSyncToken": "s1",
                }),
            ),
        },
        ("/v1/people/me/connections", Some("s1")) => Reply::json(
            200,
            json!({
                "connections": [tombstone("c2"), person("c1", "Ann Archer-Smith", "ann@x.test")],
                "nextSyncToken": "s2",
            }),
        ),
        ("/v1/contactGroups", None) => Reply::json(
            200,
            json!({
                "contactGroups": [group("myContacts", "My Contacts", true, 2), group("f", "Friends", false, 2)],
                "nextSyncToken": "g1",
            }),
        ),
        ("/v1/contactGroups", Some("g1")) => Reply::json(
            200,
            json!({
                "contactGroups": [{"resourceName": "contactGroups/f", "metadata": {"deleted": true},
                                   "groupType": "USER_CONTACT_GROUP", "name": "Friends"}],
                "nextSyncToken": "g2",
            }),
        ),
        ("/v1/contactGroups/f", None) => Reply::json(
            200,
            json!({"resourceName": "contactGroups/f", "memberResourceNames": ["people/c1", "people/gone"]}),
        ),
        _ => Reply::json(404, json!({"error": {"code": 404}})),
    }
}

fn worker<'a>(store: &'a Store, server: &MockServer) -> GoogleContactsWorker<'a> {
    GoogleContactsWorker::new(account(), store, tokens(server))
        .unwrap()
        .with_page_delay(Duration::ZERO)
}

fn gpeople(store: &Store) -> Vec<Contact> {
    store
        .find_all::<Contact>(
            &Query::new()
                .equal("accountId", ACCOUNT)
                .equal("source", "gpeople"),
        )
        .unwrap()
}

fn key(store: &Store, k: &str) -> Option<String> {
    store.get_key_value(k).unwrap()
}

#[test]
fn full_listing_then_incremental_pass() {
    let server = MockServer::start(people_api);
    let (_dir, store) = store();

    worker(&store, &server).run().unwrap();

    let book = store
        .find::<ContactBook>(&Query::new().equal("accountId", ACCOUNT))
        .unwrap()
        .unwrap();
    assert_eq!((book.id(), book.source()), ("acc1-default", "gpeople"));

    // The photo-only entry and the tombstone without a local row create nothing.
    let mut contacts = gpeople(&store);
    contacts.sort_by(|a, b| a.email().cmp(b.email()));
    assert_eq!(contacts.len(), 2);
    let ann = &contacts[0];
    assert_eq!(
        (
            ann.email(),
            ann.name(),
            ann.google_resource_name(),
            ann.book_id()
        ),
        ("ann@x.test", "Ann Archer", "people/c1", "acc1-default")
    );
    assert_eq!(ann.info()["etag"], "e-c1");
    assert_eq!(ann.refs(), tern_core::constants::CONTACT_MAX_REFS);
    assert_eq!(ann.id().len(), 40);
    assert!(!ann.hidden());

    // System groups are skipped; the user group gets the members the API resolves.
    let groups = store
        .find_all::<ContactGroup>(&Query::new().equal("accountId", ACCOUNT))
        .unwrap();
    assert_eq!(groups.len(), 1);
    let friends = &groups[0];
    assert_eq!(
        (
            friends.name(),
            friends.google_resource_name(),
            friends.book_id()
        ),
        ("Friends", "contactGroups/f", "acc1-default")
    );
    assert_eq!(friends.members(&store).unwrap(), [ann.id().to_owned()]);
    let ann_now = store
        .find::<Contact>(&Query::new().equal("id", ann.id()))
        .unwrap()
        .unwrap();
    assert_eq!(ann_now.group_ids(), [friends.id().to_owned()]);

    assert_eq!(
        key(&store, &contacts_sync_token_key(ACCOUNT)).as_deref(),
        Some("s1")
    );
    assert_eq!(
        key(&store, &groups_sync_token_key(ACCOUNT)).as_deref(),
        Some("g1")
    );

    let reqs = server.requests();
    let first = reqs
        .iter()
        .find(|r| r.path() == "/v1/people/me/connections")
        .unwrap();
    assert_eq!(
        first.query_value("requestSyncToken").as_deref(),
        Some("true")
    );
    assert_eq!(first.query_value("pageSize").as_deref(), Some("400"));
    assert!(first.query_value("syncToken").is_none());
    assert_eq!(first.header("authorization"), Some("Bearer atok"));
    let members = reqs
        .iter()
        .filter(|r| r.path().starts_with("/v1/contactGroups/"))
        .collect::<Vec<_>>();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].query_value("maxMembers").as_deref(), Some("2"));
    assert_eq!(
        tern_store::invariants::check(store.conn(), &[]).unwrap(),
        Vec::<String>::new()
    );

    // The second pass sends both tokens and applies only the changes.
    worker(&store, &server).run().unwrap();
    let contacts = gpeople(&store);
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].name(), "Ann Archer-Smith");
    assert!(store
        .find_all::<ContactGroup>(&Query::new().equal("accountId", ACCOUNT))
        .unwrap()
        .is_empty());
    assert_eq!(
        key(&store, &contacts_sync_token_key(ACCOUNT)).as_deref(),
        Some("s2")
    );
    assert_eq!(
        key(&store, &groups_sync_token_key(ACCOUNT)).as_deref(),
        Some("g2")
    );
    let synced: Vec<_> = server
        .requests()
        .into_iter()
        .filter_map(|r| r.query_value("syncToken"))
        .collect();
    assert_eq!(synced, ["s1", "g1"]);
    assert_eq!(
        tern_store::invariants::check(store.conn(), &[]).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn an_expired_sync_token_is_cleared_and_fails_the_pass() {
    let server = MockServer::start(|req| {
        if req.path() == "/google/token" {
            return token_reply();
        }
        Reply::json(
            400,
            json!({"error": {"code": 400, "status": "FAILED_PRECONDITION",
                   "message": "Sync token is expired. Clear local cache and retry call without the sync token."}}),
        )
    });
    let (_dir, store) = store();
    store
        .save_key_value(&contacts_sync_token_key(ACCOUNT), "old")
        .unwrap();
    store
        .save_key_value(&groups_sync_token_key(ACCOUNT), "old-groups")
        .unwrap();

    let err = worker(&store, &server).run().unwrap_err();
    assert_eq!(err.as_sync().unwrap().key, "Invalid Response Code: 400");
    assert!(err.is_retryable());
    assert_eq!(
        key(&store, &contacts_sync_token_key(ACCOUNT)).as_deref(),
        Some("")
    );
    // The pass stopped at the connections listing.
    assert_eq!(
        key(&store, &groups_sync_token_key(ACCOUNT)).as_deref(),
        Some("old-groups")
    );
    let listed: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.path().starts_with("/v1/"))
        .collect();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].query_value("syncToken").as_deref(), Some("old"));
}

#[test]
fn only_gmail_accounts_get_a_worker() {
    let (_dir, store) = store();
    let imap = Arc::new(Account::from_json(json!({"id": ACCOUNT, "provider": "imap"})).unwrap());
    assert!(GoogleContactsWorker::new(imap, &store, Arc::new(TokenManager::new(None))).is_err());
}
