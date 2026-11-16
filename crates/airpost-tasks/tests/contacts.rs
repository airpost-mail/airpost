// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The contact and contact-group tasks of a Gmail account against a canned People API:
//! each local phase, then the remote phase's request and what it writes back. The
//! reference is TaskProcessor.cpp (`performLocalDestroyContact` …
//! `performRemoteSyncbackContact`) and GoogleContactsWorker.cpp.

#[path = "../../airpost-net/tests/common/mod.rs"]
mod common;

use std::sync::Arc;

use airpost_core::{Account, ManualClock, Security};
use airpost_imap::{ImapAuth, ImapConfig, Session};
use airpost_net::{Endpoints, TokenManager};
use airpost_store::{Contact, ContactBook, ContactGroup, DeltaStream, Model, Query, Store, Task};
use airpost_sync::RemoteTaskRunner;
use airpost_tasks::TaskProcessor;
use common::{MockServer, Reply, Request};
use serde_json::{json, Value};

const ACCOUNT: &str = "acc1";

fn gmail() -> Arc<Account> {
    Arc::new(
        Account::from_json(json!({
            "id": ACCOUNT, "provider": "gmail", "emailAddress": "me@gmail.test",
            "settings": {"refresh_token": "r1", "refresh_client_id": "cid"},
        }))
        .unwrap(),
    )
}

/// Echoes writes back the way the People API does: a person with a resource name and a
/// fresh etag, a group with its resource name.
fn people_api(req: &Request) -> Reply {
    let ok = |v: Value| Reply::json(200, v);
    match (req.method.as_str(), req.path()) {
        ("POST", "/google/token") => ok(json!({"access_token": "atok", "expires_in": 3600})),
        ("POST", "/v1/people:createContact") => {
            let mut p = req.json();
            p["resourceName"] = json!("people/c9");
            p["etag"] = json!("e9");
            ok(p)
        }
        ("PATCH", "/v1/people/c2:updateContact") => {
            let mut p = req.json();
            p["etag"] = json!("e2-next");
            ok(p)
        }
        ("DELETE", "/v1/people/c3:deleteContact") => ok(json!({})),
        ("POST", "/v1/contactGroups") => {
            let name = req.json()["contactGroup"]["name"].clone();
            ok(
                json!({"resourceName": "contactGroups/n1", "etag": "g1", "name": name,
                      "groupType": "USER_CONTACT_GROUP"}),
            )
        }
        ("GET", "/v1/contactGroups/n1") => ok(
            json!({"resourceName": "contactGroups/n1", "etag": "g1", "name": "Book Club",
                   "groupType": "USER_CONTACT_GROUP"}),
        ),
        ("PUT", "/v1/contactGroups/n1") => ok(req.json()["contactGroup"].clone()),
        ("POST", "/v1/contactGroups/n1/members:modify") => ok(json!({})),
        ("DELETE", "/v1/contactGroups/n1") => ok(json!({})),
        _ => Reply::json(404, json!({"error": {"code": 404, "status": "NOT_FOUND"}})),
    }
}

struct Env {
    _dir: tempfile::TempDir,
    store: Store,
    account: Arc<Account>,
    tokens: Arc<TokenManager>,
    server: MockServer,
    session: Session,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    airpost_store::migrate::migrate(
        dir.path(),
        &ManualClock::new(1_700_000_000),
        &mut Vec::new(),
    )
    .unwrap();
    let store = Store::open(dir.path(), DeltaStream::new(Box::new(std::io::sink()))).unwrap();
    let server = MockServer::start(people_api);
    let tokens = Arc::new(TokenManager::new(None).with_endpoints(Endpoints::all_at(&server.url())));
    // Contact tasks never touch IMAP; the session is never connected.
    let session = Session::new(ImapConfig::new(
        "127.0.0.1",
        1,
        Security::Clear,
        "me",
        ImapAuth::Password(String::new()),
    ));
    Env {
        _dir: dir,
        store,
        account: gmail(),
        tokens,
        server,
        session,
    }
}

impl Env {
    fn processor(&self) -> TaskProcessor<'_> {
        TaskProcessor::new(self.account.clone(), &self.store).with_tokens(self.tokens.clone())
    }

    fn run(&mut self, class: &str, data: Value) -> Task {
        let mut task = Task::create(class, ACCOUNT, &data);
        self.processor().perform_local(&mut task).unwrap();
        assert_eq!(
            task.status(),
            "remote",
            "{class} local: {}",
            task.data["error"]
        );
        let mut p =
            TaskProcessor::new(self.account.clone(), &self.store).with_tokens(self.tokens.clone());
        p.perform_remote(&mut task, &mut self.session).unwrap();
        task
    }

    fn contact(&self, id: &str) -> Option<Contact> {
        self.store
            .find::<Contact>(&Query::new().equal("id", id))
            .unwrap()
    }

    fn group(&self, id: &str) -> Option<ContactGroup> {
        self.store
            .find::<ContactGroup>(&Query::new().equal("id", id))
            .unwrap()
    }

    fn requests(&self, method: &str, path: &str) -> Vec<Request> {
        self.server
            .requests()
            .into_iter()
            .filter(|r| r.method == method && r.path() == path)
            .collect()
    }

    fn synced_contact(&self, id: &str, rn: &str, name: &str, email: &str) -> Contact {
        let mut c = Contact::create(id, ACCOUNT, email, 100_000, "gpeople");
        c.set_name(name);
        c.set_google_resource_name(rn);
        c.set_info(json!({
            "resourceName": rn, "etag": format!("e-{id}"),
            "names": [{"displayName": name}], "emailAddresses": [{"value": email}],
        }));
        self.store.save(&mut c).unwrap();
        c
    }

    fn check_invariants(&self) {
        assert_eq!(
            airpost_store::invariants::check(self.store.conn(), &[]).unwrap(),
            Vec::<String>::new()
        );
    }
}

fn assert_complete(task: &Task) {
    assert_eq!(task.status(), "complete");
    assert!(task.data["error"].is_null(), "{}", task.data["error"]);
}

#[test]
fn contacts_are_created_updated_and_destroyed_through_the_people_api() {
    let mut e = env();
    let bob = e.synced_contact("bob", "people/c2", "Bob Baker", "bob@x.test");
    let carol = e.synced_contact("carol", "people/c3", "Carol Cook", "carol@x.test");

    let create = e.run(
        "SyncbackContactTask",
        json!({"contact": {"__cls": "Contact", "aid": ACCOUNT, "s": "gpeople",
                           "name": "Dan Diver", "email": "dan@x.test",
                           "info": {"names": [{"displayName": "Dan Diver"}],
                                    "emailAddresses": [{"value": "dan@x.test"}]}}}),
    );
    assert_complete(&create);
    let dan_id = create.data["contact"]["id"].as_str().unwrap().to_owned();
    let dan = e.contact(&dan_id).unwrap();
    assert_eq!(
        (
            dan.google_resource_name(),
            dan.name(),
            dan.email(),
            dan.source()
        ),
        ("people/c9", "Dan Diver", "dan@x.test", "gpeople")
    );
    assert_eq!(dan.info()["etag"], "e9");
    let sent = &e.requests("POST", "/v1/people:createContact")[0];
    assert_eq!(sent.json()["names"][0]["displayName"], "Dan Diver");

    // The client sends the contact it has, with the stored person (and its etag) as info.
    let mut edited = bob.to_json();
    edited["name"] = json!("Robert Baker");
    edited["info"]["names"] = json!([{"displayName": "Robert Baker"}]);
    edited["gis"] = json!(["stale"]);
    let update = e.run("SyncbackContactTask", json!({"contact": edited}));
    assert_complete(&update);
    let patch = &e.requests("PATCH", "/v1/people/c2:updateContact")[0];
    assert_eq!(patch.json()["etag"], "e-bob");
    assert!(patch
        .query_value("updatePersonFields")
        .is_some_and(|f| f.contains("names") && !f.contains("photos")));
    let bob = e.contact("bob").unwrap();
    assert_eq!(
        (bob.name(), bob.info()["etag"].as_str()),
        ("Robert Baker", Some("e2-next"))
    );
    // Only info, name and email are taken from the client's copy.
    assert!(bob.group_ids().is_empty());

    let destroy = e.run("DestroyContactTask", json!({"contacts": [carol.to_json()]}));
    assert_complete(&destroy);
    assert!(e.contact("carol").is_none());
    assert_eq!(e.requests("DELETE", "/v1/people/c3:deleteContact").len(), 1);
    e.check_invariants();
}

#[test]
fn destroy_hides_the_contact_until_the_remote_phase() {
    let e = env();
    let carol = e.synced_contact("carol", "people/c3", "Carol Cook", "carol@x.test");
    let mut task = Task::create(
        "DestroyContactTask",
        ACCOUNT,
        &json!({"contacts": [carol.to_json()]}),
    );
    e.processor().perform_local(&mut task).unwrap();
    assert!(e.contact("carol").unwrap().hidden());
    assert!(e.server.requests().is_empty());
}

#[test]
fn groups_are_created_filled_renamed_and_destroyed() {
    let mut e = env();
    let bob = e.synced_contact("bob", "people/c2", "Bob Baker", "bob@x.test");
    let dan = e.synced_contact("dan", "people/c9", "Dan Diver", "dan@x.test");

    // No contacts pass has run yet: the book is created with the id that pass would use.
    let create = e.run(
        "SyncbackContactGroupTask",
        json!({"group": {"__cls": "ContactGroup", "aid": ACCOUNT, "name": "Book Club"}}),
    );
    assert_complete(&create);
    let gid = create.data["group"]["id"].as_str().unwrap().to_owned();
    let group = e.group(&gid).unwrap();
    assert_eq!(
        (group.name(), group.google_resource_name(), group.book_id()),
        ("Book Club", "contactGroups/n1", "acc1-default")
    );
    let book = e
        .store
        .find::<ContactBook>(&Query::new().equal("accountId", ACCOUNT))
        .unwrap()
        .unwrap();
    assert_eq!(book.source(), "gpeople");

    let add = e.run(
        "ChangeContactGroupMembershipTask",
        json!({"direction": "add", "group": group.to_json(),
               "contacts": [bob.to_json(), dan.to_json()]}),
    );
    assert_complete(&add);
    let mut members = group.members(&e.store).unwrap();
    members.sort();
    assert_eq!(members, ["bob", "dan"]);
    assert_eq!(
        e.contact("dan").unwrap().group_ids(),
        std::slice::from_ref(&gid)
    );

    let remove = e.run(
        "ChangeContactGroupMembershipTask",
        json!({"direction": "remove", "group": group.to_json(), "contacts": [dan.to_json()]}),
    );
    assert_complete(&remove);
    assert_eq!(group.members(&e.store).unwrap(), ["bob"]);
    assert!(e.contact("dan").unwrap().group_ids().is_empty());
    let modify = e.requests("POST", "/v1/contactGroups/n1/members:modify");
    assert_eq!(
        modify[0].json(),
        json!({"resourceNamesToAdd": ["people/c2", "people/c9"]})
    );
    assert_eq!(
        modify[1].json(),
        json!({"resourceNamesToRemove": ["people/c9"]})
    );

    let mut renamed = e.group(&gid).unwrap().to_json();
    renamed["name"] = json!("Bouldering");
    let rename = e.run("SyncbackContactGroupTask", json!({"group": renamed}));
    assert_complete(&rename);
    let put = &e.requests("PUT", "/v1/contactGroups/n1")[0];
    assert_eq!(
        (
            put.json()["contactGroup"]["name"].as_str(),
            put.json()["contactGroup"]["etag"].as_str()
        ),
        (Some("Bouldering"), Some("g1"))
    );
    assert_eq!(e.group(&gid).unwrap().name(), "Bouldering");

    let destroy = e.run(
        "DestroyContactGroupTask",
        json!({"group": e.group(&gid).unwrap().to_json()}),
    );
    assert_complete(&destroy);
    assert_eq!(destroy.data["googleResourceName"], "contactGroups/n1");
    assert!(e.group(&gid).is_none());
    assert_eq!(e.requests("DELETE", "/v1/contactGroups/n1").len(), 1);
    e.check_invariants();
}

#[test]
fn a_task_for_a_missing_model_fails_with_not_found() {
    let e = env();
    let mut task = Task::create(
        "ChangeContactGroupMembershipTask",
        ACCOUNT,
        &json!({"direction": "add", "group": {"id": "nope"}, "contacts": []}),
    );
    e.processor().perform_local(&mut task).unwrap();
    assert_eq!(task.status(), "complete");
    assert_eq!(task.data["error"]["key"], "not-found");

    let mut task = Task::create(
        "SyncbackContactTask",
        ACCOUNT,
        &json!({"contact": {"id": "nope", "s": "gpeople", "name": "", "email": "", "info": {}}}),
    );
    e.processor().perform_local(&mut task).unwrap();
    assert_eq!(task.data["error"]["key"], "not-found");
    assert!(e.server.requests().is_empty());
}

#[test]
fn people_api_errors_end_the_task_with_the_error() {
    let mut e = env();
    // people/c5 is unknown to the canned API: updateContact answers 404.
    let mut c = e.synced_contact("eve", "people/c5", "Eve", "eve@x.test");
    c.set_name("Eve E");
    let task = e.run("SyncbackContactTask", json!({"contact": c.to_json()}));
    assert_eq!(task.status(), "complete");
    assert_eq!(task.data["error"]["key"], "Invalid Response Code: 404");
}
