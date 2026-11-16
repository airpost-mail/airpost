// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

mod common;

use std::time::Duration;

use airpost_net::people::{
    is_sync_token_expired, Membership, Page, Person, PAGE_DELAY, PERSON_FIELDS,
    PERSON_UPDATE_FIELDS,
};
use airpost_net::{Endpoints, HttpClient, PeopleClient};
use common::{MockServer, Reply};
use serde_json::json;

fn client<'a>(http: &'a HttpClient, server: &MockServer) -> PeopleClient<'a> {
    let root = Endpoints::all_at(&server.url()).google_people_root;
    let mut c = PeopleClient::new(http, "atok", &root);
    assert_eq!(c.page_delay, PAGE_DELAY);
    c.page_delay = Duration::ZERO;
    c
}

fn expired_sync_token() -> Reply {
    Reply::json(
        410,
        json!({"error": {
            "code": 410,
            "message": "Sync token is expired. Clear local cache and retry call without the sync token.",
            "status": "GONE",
            "details": [{"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "EXPIRED_SYNC_TOKEN"}]
        }}),
    )
}

#[test]
fn connections_full_sync_pages() {
    let server = MockServer::start(|req| {
        assert_eq!(req.path(), "/v1/people/me/connections");
        match req.query_value("pageToken").as_deref() {
            None => Reply::json(
                200,
                json!({
                    "connections": [
                        {"resourceName": "people/c1", "etag": "e1",
                         "names": [{"displayName": "Ann"}], "emailAddresses": [{"value": "ann@x.test"}]},
                        {"resourceName": "people/c2", "etag": "e2"}
                    ],
                    "nextPageToken": "p2", "totalPeople": 3
                }),
            ),
            Some("p2") => Reply::json(
                200,
                json!({
                    "connections": [{"resourceName": "people/c3", "etag": "e3"}],
                    "nextSyncToken": "s1"
                }),
            ),
            Some(other) => panic!("unexpected page {other}"),
        }
    });
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);

    let mut pages: Vec<Page<Person>> = Vec::new();
    let outcome = people
        .list_connections(None, |p| {
            pages.push(p);
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome.next_sync_token.as_deref(), Some("s1"));
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].items.len(), 2);
    assert_eq!(pages[0].items[0].display_name, "Ann");
    assert_eq!(pages[0].items[0].email, "ann@x.test");
    assert_eq!(pages[0].next_page_token.as_deref(), Some("p2"));
    assert_eq!(pages[1].items[0].resource_name, "people/c3");

    let reqs = server.requests();
    assert_eq!(reqs.len(), 2);
    for r in &reqs {
        assert_eq!(r.method, "GET");
        assert_eq!(r.header("authorization"), Some("Bearer atok"));
        assert_eq!(
            r.query_value("personFields").as_deref(),
            Some(PERSON_FIELDS)
        );
        assert_eq!(r.query_value("pageSize").as_deref(), Some("400"));
        assert_eq!(r.query_value("requestSyncToken").as_deref(), Some("true"));
        assert_eq!(r.query_value("syncToken"), None);
    }
    assert_eq!(reqs[0].query_value("pageToken"), None);
    assert_eq!(reqs[1].query_value("pageToken").as_deref(), Some("p2"));
}

#[test]
fn connections_incremental_sync() {
    let server = MockServer::start(|_| {
        Reply::json(
            200,
            json!({
                "connections": [
                    {"resourceName": "people/c1", "etag": "e", "metadata": {"deleted": true}},
                    {"resourceName": "people/c2", "etag": "e", "photos": [{"url": "u"}]}
                ],
                "nextSyncToken": "s2"
            }),
        )
    });
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);

    let mut seen = Vec::new();
    let outcome = people
        .list_connections(Some("s+1/="), |p| {
            seen.extend(p.items);
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome.next_sync_token.as_deref(), Some("s2"));
    assert!(seen[0].deleted);
    assert!(seen[1].is_photo_only());

    let r = &server.requests()[0];
    // Tokens are query-encoded, so `+`, `/` and `=` survive.
    assert_eq!(r.query_value("syncToken").as_deref(), Some("s+1/="));
}

#[test]
fn expired_sync_token_is_recognised() {
    let server = MockServer::start(|_| expired_sync_token());
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);

    let mut called = false;
    let err = people
        .list_connections(Some("old"), |_| {
            called = true;
            Ok(())
        })
        .unwrap_err();
    assert!(is_sync_token_expired(&err));
    assert!(!called);
    assert_eq!(err.as_sync().unwrap().key, "Invalid Response Code: 410");

    let err = people
        .list_contact_groups(Some("old"), |_| Ok(()))
        .unwrap_err();
    assert!(is_sync_token_expired(&err));
}

#[test]
fn other_failures_are_not_token_expiry() {
    let server = MockServer::start(|req| match req.path() {
        "/v1/contactGroups" => Reply::json(
            429,
            json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED"}}),
        ),
        _ => Reply::json(
            401,
            json!({"error": {"code": 401, "status": "UNAUTHENTICATED"}}),
        ),
    });
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);

    let err = people.list_connections(None, |_| Ok(())).unwrap_err();
    assert!(!is_sync_token_expired(&err));
    assert!(!err.is_retryable());
    let err = people.list_contact_groups(None, |_| Ok(())).unwrap_err();
    assert!(!is_sync_token_expired(&err));
    assert!(err.is_retryable());
}

#[test]
fn a_failing_page_handler_stops_paging() {
    let server = MockServer::start(|_| {
        Reply::json(200, json!({"connections": [], "nextPageToken": "again"}))
    });
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);
    let err = people
        .list_connections(None, |_| Err(airpost_core::Error::other("stop")))
        .unwrap_err();
    assert_eq!(err.to_string(), "stop");
    assert_eq!(server.count(), 1);
}

#[test]
fn contact_groups_pages_and_members() {
    let server = MockServer::start(|req| match req.path() {
        "/v1/contactGroups" => match req.query_value("pageToken").as_deref() {
            None => Reply::json(
                200,
                json!({
                    "contactGroups": [
                        {"resourceName": "contactGroups/myContacts", "groupType": "SYSTEM_CONTACT_GROUP"},
                        {"resourceName": "contactGroups/a1", "groupType": "USER_CONTACT_GROUP",
                         "name": "Friends", "memberCount": 2, "etag": "ge"}
                    ],
                    "nextPageToken": "g2", "nextSyncToken": "gs-partial"
                }),
            ),
            _ => Reply::json(
                200,
                json!({
                    "contactGroups": [{"resourceName": "contactGroups/a2", "metadata": {"deleted": true}}],
                    "nextSyncToken": "gs1"
                }),
            ),
        },
        "/v1/contactGroups/a1" => Reply::json(
            200,
            json!({"resourceName": "contactGroups/a1",
                   "memberResourceNames": ["people/c1", "people/c2"]}),
        ),
        other => panic!("unexpected {other}"),
    });
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);

    let mut groups = Vec::new();
    let outcome = people
        .list_contact_groups(None, |p| {
            groups.extend(p.items);
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome.next_sync_token.as_deref(), Some("gs1"));
    assert_eq!(groups.len(), 3);
    assert!(groups[0].is_system());
    assert_eq!(groups[1].name, "Friends");
    assert_eq!(groups[1].member_count, 2);
    assert!(groups[2].deleted);

    let members = people.group_members("contactGroups/a1", 2).unwrap();
    assert_eq!(members, ["people/c1", "people/c2"]);

    let reqs = server.requests();
    assert_eq!(reqs[0].query_value("pageSize").as_deref(), Some("400"));
    assert_eq!(reqs[0].query_value("requestSyncToken"), None);
    assert_eq!(reqs[1].query_value("pageToken").as_deref(), Some("g2"));
    assert_eq!(reqs[2].query_value("maxMembers").as_deref(), Some("2"));
}

#[test]
fn group_writes() {
    let server = MockServer::start(|req| match (req.method.as_str(), req.path()) {
        ("POST", "/v1/contactGroups") => Reply::json(
            200,
            json!({"resourceName": "contactGroups/new", "name": req.json()["contactGroup"]["name"]}),
        ),
        ("GET", "/v1/contactGroups/g1") => Reply::json(
            200,
            json!({"resourceName": "contactGroups/g1", "etag": "etag-1", "name": "Old",
                   "groupType": "USER_CONTACT_GROUP"}),
        ),
        ("PUT", "/v1/contactGroups/g1") => Reply::json(200, req.json()["contactGroup"].clone()),
        ("DELETE", "/v1/contactGroups/g1") => Reply::json(200, json!({})),
        ("POST", "/v1/contactGroups/g1/members:modify") => Reply::json(200, json!({})),
        other => panic!("unexpected {other:?}"),
    });
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);

    let g = people.create_group("Team").unwrap();
    assert_eq!(g.resource_name, "contactGroups/new");
    assert_eq!(g.name, "Team");

    let g = people.rename_group("contactGroups/g1", "New").unwrap();
    assert_eq!(g.name, "New");
    assert_eq!(g.etag, "etag-1");

    people.delete_group("contactGroups/g1").unwrap();
    let members = vec!["people/c1".to_string(), "people/c2".to_string()];
    people
        .modify_group_members("contactGroups/g1", Membership::Add, &members)
        .unwrap();
    people
        .modify_group_members("contactGroups/g1", Membership::Remove, &members[..1])
        .unwrap();

    let reqs = server.requests();
    assert_eq!(reqs[0].json(), json!({"contactGroup": {"name": "Team"}}));
    assert_eq!(reqs[2].json()["contactGroup"]["etag"], "etag-1");
    assert_eq!(reqs[2].json()["contactGroup"]["name"], "New");
    assert_eq!(reqs[3].method, "DELETE");
    assert_eq!(
        reqs[4].json(),
        json!({"resourceNamesToAdd": ["people/c1", "people/c2"]})
    );
    assert_eq!(
        reqs[5].json(),
        json!({"resourceNamesToRemove": ["people/c1"]})
    );
}

#[test]
fn contact_writes() {
    let server = MockServer::start(|req| match (req.method.as_str(), req.path()) {
        ("POST", "/v1/people:createContact") => {
            let mut p = req.json();
            p["resourceName"] = json!("people/c9");
            p["etag"] = json!("e1");
            Reply::json(200, p)
        }
        ("PATCH", "/v1/people/c9:updateContact") => {
            let mut p = req.json();
            p["etag"] = json!("e2");
            Reply::json(200, p)
        }
        ("DELETE", "/v1/people/c9:deleteContact") => Reply::json(200, json!({})),
        other => panic!("unexpected {other:?}"),
    });
    let http = HttpClient::new().unwrap();
    let people = client(&http, &server);

    let info =
        json!({"names": [{"displayName": "Zed"}], "emailAddresses": [{"value": "z@x.test"}]});
    let p = people.create_contact(&info).unwrap();
    assert_eq!(p.resource_name, "people/c9");
    assert_eq!(
        (p.display_name.as_str(), p.email.as_str()),
        ("Zed", "z@x.test")
    );

    let mut changed = p.raw.clone();
    changed["names"][0]["displayName"] = json!("Zed Z");
    let p = people.update_contact("people/c9", &changed).unwrap();
    assert_eq!(p.display_name, "Zed Z");
    assert_eq!(p.etag, "e2");

    people.delete_contact("people/c9").unwrap();

    let reqs = server.requests();
    assert_eq!(reqs[0].json(), info);
    assert_eq!(reqs[0].header("content-type"), Some("application/json"));
    assert_eq!(
        reqs[1].query_value("updatePersonFields").as_deref(),
        Some(PERSON_UPDATE_FIELDS)
    );
    assert_eq!(reqs[1].json()["etag"], "e1");
    assert_eq!(reqs[2].header("authorization"), Some("Bearer atok"));
}
