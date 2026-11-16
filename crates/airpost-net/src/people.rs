// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The Google People API client side of `GoogleContactsWorker`. Derived from
//! `MailSync/GoogleContactsWorker.cpp` (Copyright © 2017 Foundry 376, GPL-3.0).
//!
//! This returns plain structs; the worker that stores them as `Contact` / `ContactGroup`
//! rows and keeps the `gsynctoken-*` keys is in airpost-sync (P4-G).

use std::time::Duration;

use airpost_core::{Error, Result};
use reqwest::Url;
use serde_json::{json, Value};

use crate::HttpClient;

/// Fields read on list and returned on writes.
pub const PERSON_FIELDS: &str = "emailAddresses,genders,names,nicknames,phoneNumbers,urls,birthdays,addresses,userDefined,relations,occupations,organizations,photos,biographies";
/// Fields an update may change; photos are read-only through `updateContact`.
pub const PERSON_UPDATE_FIELDS: &str = "emailAddresses,genders,names,nicknames,phoneNumbers,urls,birthdays,addresses,userDefined,relations,occupations,organizations,biographies";
pub const PAGE_SIZE: u32 = 400;
/// Wait before every list request: Google allows 90 requests per second per user and a
/// full sync of a large address book gets there.
pub const PAGE_DELAY: Duration = Duration::from_secs(2);

/// A `people/me/connections` entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Person {
    pub resource_name: String,
    pub etag: String,
    pub deleted: bool,
    /// `names[0].displayName`, or `""`.
    pub display_name: String,
    /// `emailAddresses[0].value`, or `""`.
    pub email: String,
    /// The whole person; the worker stores it as the contact's `info`.
    pub raw: Value,
}

impl Person {
    pub fn from_json(raw: Value) -> Person {
        let first = |list: &str, field: &str| {
            raw.get(list)
                .and_then(|l| l.get(0))
                .and_then(|e| e.get(field))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        // The People API documents `metadata.deleted`; the C++ engine read a top-level
        // `deleted`. Either one marks the person deleted.
        let deleted = raw.get("deleted").and_then(Value::as_bool) == Some(true)
            || raw.pointer("/metadata/deleted").and_then(Value::as_bool) == Some(true);
        Person {
            resource_name: str_field(&raw, "resourceName"),
            etag: str_field(&raw, "etag"),
            deleted,
            display_name: first("names", "displayName"),
            email: first("emailAddresses", "value"),
            raw,
        }
    }

    /// Sync results sometimes carry a person with only `resourceName`, `etag` and `photos`,
    /// seen after a contact is deleted. The C++ worker skipped these.
    pub fn is_photo_only(&self) -> bool {
        self.raw.as_object().is_some_and(|o| {
            o.len() == 3
                && o.contains_key("photos")
                && o.contains_key("etag")
                && o.contains_key("resourceName")
        })
    }
}

/// A `contactGroups` entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContactGroup {
    pub resource_name: String,
    pub etag: String,
    pub name: String,
    /// `USER_CONTACT_GROUP` or `SYSTEM_CONTACT_GROUP`.
    pub group_type: String,
    pub member_count: u32,
    pub deleted: bool,
    pub raw: Value,
}

impl ContactGroup {
    pub fn from_json(raw: Value) -> ContactGroup {
        ContactGroup {
            resource_name: str_field(&raw, "resourceName"),
            etag: str_field(&raw, "etag"),
            name: str_field(&raw, "name"),
            group_type: str_field(&raw, "groupType"),
            member_count: raw
                .get("memberCount")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
            deleted: raw.pointer("/metadata/deleted").and_then(Value::as_bool) == Some(true),
            raw,
        }
    }

    /// Google's built-in groups (My Contacts, Starred, …) are mostly deprecated and are
    /// not synced.
    pub fn is_system(&self) -> bool {
        self.group_type == "SYSTEM_CONTACT_GROUP"
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

/// The end of a paginated listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListOutcome {
    /// The last `nextSyncToken` seen; the worker stores it when present.
    pub next_sync_token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Membership {
    Add,
    Remove,
}

/// True when a listing failed because the stored sync token expired. The worker clears
/// the token, so the next run is a full sync, and still treats the run as failed.
pub fn is_sync_token_expired(err: &Error) -> bool {
    err.as_sync().is_some_and(|e| {
        e.debuginfo.contains("Sync token is expired") || e.debuginfo.contains("EXPIRED_SYNC_TOKEN")
    })
}

pub struct PeopleClient<'a> {
    pub http: &'a HttpClient,
    pub access_token: String,
    /// The API root, ending with `/` (`Endpoints::google_people_root`).
    pub root: String,
    /// [`PAGE_DELAY`] in production; tests set it to zero.
    pub page_delay: Duration,
}

impl<'a> PeopleClient<'a> {
    pub fn new(http: &'a HttpClient, access_token: impl Into<String>, root: &str) -> Self {
        PeopleClient {
            http,
            access_token: access_token.into(),
            root: root.to_owned(),
            page_delay: PAGE_DELAY,
        }
    }

    /// One page of `GET people/me/connections`. It always asks for a sync token, so the
    /// last page carries `nextSyncToken`.
    pub fn connections(
        &self,
        page_token: Option<&str>,
        sync_token: Option<&str>,
    ) -> Result<Page<Person>> {
        let mut url = self.url("people/me/connections")?;
        url.query_pairs_mut()
            .append_pair("personFields", PERSON_FIELDS)
            .append_pair("pageSize", &PAGE_SIZE.to_string());
        add_paging(&mut url, page_token, sync_token, true);
        let resp = self.get(url)?;
        Ok(page_of(&resp, "connections", Person::from_json))
    }

    /// One page of `GET contactGroups`, which returns `nextSyncToken` unasked.
    pub fn contact_groups(
        &self,
        page_token: Option<&str>,
        sync_token: Option<&str>,
    ) -> Result<Page<ContactGroup>> {
        let mut url = self.url("contactGroups")?;
        url.query_pairs_mut()
            .append_pair("pageSize", &PAGE_SIZE.to_string());
        add_paging(&mut url, page_token, sync_token, false);
        let resp = self.get(url)?;
        Ok(page_of(&resp, "contactGroups", ContactGroup::from_json))
    }

    /// Every page of connections since `sync_token` (`paginateGoogleCollection`), each
    /// passed to `on_page` as it arrives.
    pub fn list_connections(
        &self,
        sync_token: Option<&str>,
        on_page: impl FnMut(Page<Person>) -> Result<()>,
    ) -> Result<ListOutcome> {
        self.paginate(|p| self.connections(p, sync_token), on_page)
    }

    /// Every page of contact groups since `sync_token`.
    pub fn list_contact_groups(
        &self,
        sync_token: Option<&str>,
        on_page: impl FnMut(Page<ContactGroup>) -> Result<()>,
    ) -> Result<ListOutcome> {
        self.paginate(|p| self.contact_groups(p, sync_token), on_page)
    }

    fn paginate<T>(
        &self,
        fetch: impl Fn(Option<&str>) -> Result<Page<T>>,
        mut on_page: impl FnMut(Page<T>) -> Result<()>,
    ) -> Result<ListOutcome> {
        let mut outcome = ListOutcome::default();
        let mut page_token: Option<String> = None;
        loop {
            if !self.page_delay.is_zero() {
                std::thread::sleep(self.page_delay);
            }
            let page = fetch(page_token.as_deref())?;
            if page.next_sync_token.is_some() {
                outcome.next_sync_token.clone_from(&page.next_sync_token);
            }
            page_token.clone_from(&page.next_page_token);
            on_page(page)?;
            if page_token.is_none() {
                return Ok(outcome);
            }
        }
    }

    /// The group's member resource names (`GET contactGroups/x?maxMembers=n`).
    pub fn group_members(&self, resource_name: &str, max_members: u32) -> Result<Vec<String>> {
        let mut url = self.url(resource_name)?;
        url.query_pairs_mut()
            .append_pair("maxMembers", &max_members.to_string());
        let resp = self.get(url)?;
        Ok(resp
            .get("memberResourceNames")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `POST contactGroups`.
    pub fn create_group(&self, name: &str) -> Result<ContactGroup> {
        let body = json!({ "contactGroup": { "name": name } });
        let resp = self.send("POST", self.url("contactGroups")?, Some(&body))?;
        Ok(ContactGroup::from_json(resp))
    }

    /// Renames a group: reads it for the current etag, then `PUT`s it back with the name.
    pub fn rename_group(&self, resource_name: &str, name: &str) -> Result<ContactGroup> {
        let mut data = self.get(self.url(resource_name)?)?;
        if let Some(o) = data.as_object_mut() {
            o.insert("name".into(), Value::String(name.to_owned()));
        }
        let body = json!({ "contactGroup": data });
        let resp = self.send("PUT", self.url(resource_name)?, Some(&body))?;
        Ok(ContactGroup::from_json(resp))
    }

    /// `DELETE contactGroups/x`.
    pub fn delete_group(&self, resource_name: &str) -> Result<()> {
        self.send("DELETE", self.url(resource_name)?, None)
            .map(drop)
    }

    /// `POST contactGroups/x/members:modify`.
    pub fn modify_group_members(
        &self,
        group_resource_name: &str,
        direction: Membership,
        contact_resource_names: &[String],
    ) -> Result<()> {
        let key = match direction {
            Membership::Add => "resourceNamesToAdd",
            Membership::Remove => "resourceNamesToRemove",
        };
        let body = json!({ key: contact_resource_names });
        let url = self.url(&format!("{group_resource_name}/members:modify"))?;
        self.send("POST", url, Some(&body)).map(drop)
    }

    /// `POST people:createContact` with the contact's `info` as the person.
    pub fn create_contact(&self, person: &Value) -> Result<Person> {
        let resp = self.send("POST", self.url("people:createContact")?, Some(person))?;
        Ok(Person::from_json(resp))
    }

    /// `PATCH people/x:updateContact`. `person` must carry the current `etag`.
    pub fn update_contact(&self, resource_name: &str, person: &Value) -> Result<Person> {
        let mut url = self.url(&format!("{resource_name}:updateContact"))?;
        url.query_pairs_mut()
            .append_pair("updatePersonFields", PERSON_UPDATE_FIELDS);
        let resp = self.send("PATCH", url, Some(person))?;
        Ok(Person::from_json(resp))
    }

    /// `DELETE people/x:deleteContact`.
    pub fn delete_contact(&self, resource_name: &str) -> Result<()> {
        let url = self.url(&format!("{resource_name}:deleteContact"))?;
        self.send("DELETE", url, None).map(drop)
    }

    fn url(&self, path: &str) -> Result<Url> {
        let s = format!("{}{}", self.root, path);
        Url::parse(&s).map_err(|e| {
            airpost_core::SyncError::fatal(
                "URL using bad/illegal format or missing URL",
                format!("{s}: {e}"),
            )
            .into()
        })
    }

    fn get(&self, url: Url) -> Result<Value> {
        self.send("GET", url, None)
    }

    fn send(&self, method: &str, url: Url, body: Option<&Value>) -> Result<Value> {
        self.http
            .json(method, url.as_str(), Some(&self.access_token), body)
    }
}

/// `requestSyncToken` is sent on incremental syncs too, as Google's sync guide does, so
/// every completed listing ends with a fresh token.
fn add_paging(
    url: &mut Url,
    page_token: Option<&str>,
    sync_token: Option<&str>,
    request_sync_token: bool,
) {
    let mut q = url.query_pairs_mut();
    if let Some(p) = page_token.filter(|p| !p.is_empty()) {
        q.append_pair("pageToken", p);
    }
    if let Some(s) = sync_token.filter(|s| !s.is_empty()) {
        q.append_pair("syncToken", s);
    }
    if request_sync_token {
        q.append_pair("requestSyncToken", "true");
    }
}

fn page_of<T>(resp: &Value, key: &str, item: impl Fn(Value) -> T) -> Page<T> {
    let token = |k: &str| {
        resp.get(k)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    Page {
        items: resp
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().cloned().map(&item).collect())
            .unwrap_or_default(),
        next_page_token: token("nextPageToken"),
        next_sync_token: token("nextSyncToken"),
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn person_fields() {
        let p = Person::from_json(json!({
            "resourceName": "people/c1", "etag": "e",
            "names": [{"displayName": "Ann"}, {"displayName": "B"}],
            "emailAddresses": [{"value": "ann@x.test"}],
        }));
        assert_eq!(p.display_name, "Ann");
        assert_eq!(p.email, "ann@x.test");
        assert!(!p.deleted && !p.is_photo_only());

        let gone =
            Person::from_json(json!({"resourceName": "people/c2", "metadata": {"deleted": true}}));
        assert!(gone.deleted);
        assert!(Person::from_json(json!({"resourceName": "r", "deleted": true})).deleted);
        let photo = Person::from_json(json!({"resourceName": "r", "etag": "e", "photos": []}));
        assert!(photo.is_photo_only());
    }

    #[test]
    fn group_fields() {
        let g = ContactGroup::from_json(json!({
            "resourceName": "contactGroups/a", "name": "Friends",
            "groupType": "USER_CONTACT_GROUP", "memberCount": 3,
        }));
        assert_eq!((g.name.as_str(), g.member_count), ("Friends", 3));
        assert!(!g.is_system() && !g.deleted);
        let s = ContactGroup::from_json(
            json!({"groupType": "SYSTEM_CONTACT_GROUP", "metadata": {"deleted": true}}),
        );
        assert!(s.is_system() && s.deleted);
    }
}
