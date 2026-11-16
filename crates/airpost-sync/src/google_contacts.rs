// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Google contacts and contact groups through the People API (`GoogleContactsWorker`).
//!
//! Derived from Mailspring-Sync `MailSync/GoogleContactsWorker.cpp` and `.hpp`, Copyright ©
//! 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! [`GoogleContactsWorker::run`] is the contacts half of a calContacts pass for a Gmail
//! account; the write methods are the remote phases of the contact and contact-group tasks,
//! called by tern-tasks on the foreground thread with that thread's store. Every call fetches
//! its access token from the shared [`TokenManager`] first, so a rotated refresh token
//! reaches the client as `ProcessAccountSecretsUpdated` whichever thread refreshed it.
//!
//! Differences from the C++:
//! - A person or group with `metadata.deleted: true` is a tombstone: it removes the local
//!   row and never creates one. C++ read a top-level `deleted` for people (the API never
//!   sends it) and fell through to an upsert for a tombstone it had no row for.
//! - Membership changes skip contacts without a resource name instead of sending `""`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tern_core::constants::{CONTACT_MAX_REFS, GOOGLE_SYNC_SOURCE};
use tern_core::{ids, Account, Error, Result};
use tern_net::people::{self, Membership, Person};
use tern_net::{HttpClient, PeopleClient, TokenManager};
use tern_store::{Contact, ContactBook, ContactGroup, Model, Query, Store};
use tracing::{info, warn};

/// The `gsynctoken-contacts-<aid>` / `gsynctoken-groups-<aid>` key-value entries.
pub fn contacts_sync_token_key(account_id: &str) -> String {
    format!("gsynctoken-contacts-{account_id}")
}

pub fn groups_sync_token_key(account_id: &str) -> String {
    format!("gsynctoken-groups-{account_id}")
}

/// The account's ContactBook, created with source `gpeople` if missing. Its existence is how
/// the client knows contact sync is enabled.
pub fn find_or_create_book(store: &Store, account: &Account) -> Result<ContactBook> {
    let existing = store.find::<ContactBook>(&Query::new().equal("accountId", account.id()))?;
    if let Some(book) = existing {
        return Ok(book);
    }
    let mut book = ContactBook::create(&format!("{}-default", account.id()), account.id());
    book.set_source(GOOGLE_SYNC_SOURCE);
    store.save(&mut book)?;
    Ok(book)
}

pub struct GoogleContactsWorker<'a> {
    account: Arc<Account>,
    store: &'a Store,
    tokens: Arc<TokenManager>,
    http: HttpClient,
    root: String,
    page_delay: Duration,
}

impl<'a> GoogleContactsWorker<'a> {
    /// The worker for a Gmail account; any other provider is a caller bug.
    pub fn new(account: Arc<Account>, store: &'a Store, tokens: Arc<TokenManager>) -> Result<Self> {
        if account.provider() != "gmail" {
            return Err(Error::other(
                "GoogleContactsWorker created with non-gmail account",
            ));
        }
        let root = tokens.endpoints().google_people_root.clone();
        Ok(GoogleContactsWorker {
            account,
            store,
            tokens,
            http: HttpClient::new()?,
            root,
            page_delay: people::PAGE_DELAY,
        })
    }

    /// Replaces the wait before every list request (tests).
    pub fn with_page_delay(mut self, delay: Duration) -> Self {
        self.page_delay = delay;
        self
    }

    fn client(&self) -> Result<PeopleClient<'_>> {
        let token = self.tokens.token_for(&self.http, &self.account)?;
        let mut client = PeopleClient::new(&self.http, token.access_token, &self.root);
        client.page_delay = self.page_delay;
        Ok(client)
    }

    /// One contacts pass: every connection changed since the stored sync token (all of
    /// them without one), then every contact group, re-reading the members of each group
    /// the listing returns. An expired sync token is cleared and the pass fails; the next
    /// pass lists everything.
    pub fn run(&self) -> Result<()> {
        let client = self.client()?;
        let store = self.store;
        let aid = self.account.id();
        let book = find_or_create_book(store, &self.account)?;

        let mut contacts_local: HashMap<String, Contact> = HashMap::new();
        let query = Query::new()
            .equal("accountId", aid)
            .equal("source", GOOGLE_SYNC_SOURCE);
        for c in store.find_all::<Contact>(&query)? {
            contacts_local.insert(c.google_resource_name().to_owned(), c);
        }

        let key = contacts_sync_token_key(aid);
        let token = self.sync_token(&key)?;
        let listed = client.list_connections(token.as_deref(), |page| {
            store.transaction("GoogleContactsWorker::run", |s| {
                for person in page.items {
                    self.apply_connection(s, &book, &mut contacts_local, person)?;
                }
                Ok(())
            })
        });
        self.finish_listing(&key, listed)?;

        let mut groups_local =
            store.find_all::<ContactGroup>(&Query::new().equal("accountId", aid))?;
        let key = groups_sync_token_key(aid);
        let token = self.sync_token(&key)?;
        let listed = client.list_contact_groups(token.as_deref(), |page| {
            for group in page.items {
                self.apply_group(&client, &book, &mut groups_local, &contacts_local, group)?;
            }
            Ok(())
        });
        self.finish_listing(&key, listed)
    }

    fn sync_token(&self, key: &str) -> Result<Option<String>> {
        Ok(self.store.get_key_value(key)?.filter(|t| !t.is_empty()))
    }

    /// Stores the listing's `nextSyncToken`, or clears the stored token when the server
    /// says it expired (`paginateGoogleCollection`).
    fn finish_listing(&self, key: &str, listed: Result<people::ListOutcome>) -> Result<()> {
        match listed {
            Ok(outcome) => {
                if let Some(t) = outcome.next_sync_token {
                    self.store.save_key_value(key, &t)?;
                }
                Ok(())
            }
            Err(e) => {
                if people::is_sync_token_expired(&e) {
                    self.store.save_key_value(key, "")?;
                }
                Err(e)
            }
        }
    }

    fn apply_connection(
        &self,
        store: &Store,
        book: &ContactBook,
        contacts_local: &mut HashMap<String, Contact>,
        person: Person,
    ) -> Result<()> {
        if person.resource_name.is_empty() {
            warn!("- Google connection without a resourceName, skipping");
            return Ok(());
        }
        if person.deleted {
            if let Some(mut local) = contacts_local.remove(&person.resource_name) {
                store.remove(&mut local)?;
            }
            return Ok(());
        }
        // Sent after a contact is deleted, without any other data.
        if person.is_photo_only() {
            return Ok(());
        }
        let local = contacts_local
            .entry(person.resource_name.clone())
            .or_insert_with(|| {
                let mut c = Contact::create(
                    &ids::random_id(),
                    self.account.id(),
                    "",
                    CONTACT_MAX_REFS,
                    GOOGLE_SYNC_SOURCE,
                );
                c.set_google_resource_name(&person.resource_name);
                c.set_book_id(book.id());
                c
            });
        apply_person_to_contact(local, person);
        store.save(local)
    }

    fn apply_group(
        &self,
        client: &PeopleClient<'_>,
        book: &ContactBook,
        groups_local: &mut Vec<ContactGroup>,
        contacts_local: &HashMap<String, Contact>,
        group: people::ContactGroup,
    ) -> Result<()> {
        let store = self.store;
        let existing = groups_local
            .iter()
            .position(|g| g.google_resource_name() == group.resource_name);
        if group.deleted {
            if let Some(i) = existing {
                let mut local = groups_local.remove(i);
                store.remove(&mut local)?;
            }
            return Ok(());
        }
        // Google's built-in groups are mostly deprecated.
        if group.is_system() {
            return Ok(());
        }

        let mut local = match existing {
            Some(i) => groups_local[i].clone(),
            None => {
                let mut g = ContactGroup::create(&ids::random_id(), self.account.id());
                g.set_google_resource_name(&group.resource_name);
                g
            }
        };
        local.set_name(&group.name);
        local.set_book_id(book.id());
        store.save(&mut local)?;

        // Whenever a group changes, its membership is read again.
        let mut members = Vec::new();
        if group.member_count > 0 {
            for rn in client.group_members(&group.resource_name, group.member_count)? {
                match contacts_local.get(&rn) {
                    Some(c) => members.push(c.id().to_owned()),
                    None => warn!("Google group references resourceName not found: {rn}"),
                }
            }
        }
        local.sync_members(store, &members)?;
        match existing {
            Some(i) => groups_local[i] = local,
            None => groups_local.push(local),
        }
        Ok(())
    }

    /// SyncbackContactTask's remote phase: `updateContact` with the stored person (it
    /// carries the etag) when the contact has a resource name, `createContact` otherwise;
    /// the response replaces the contact's `info`, name and email.
    pub fn upsert_contact(&self, contact: &mut Contact) -> Result<()> {
        let client = self.client()?;
        let body = contact.info().clone();
        info!("Upserting contact with data: {body}");
        let resp = if contact.google_resource_name().is_empty() {
            let resp = client.create_contact(&body)?;
            contact.set_google_resource_name(require_resource_name(&resp.resource_name)?);
            resp
        } else {
            client.update_contact(contact.google_resource_name(), &body)?
        };
        info!("Upserting contact complete: {}", resp.raw);
        apply_person_to_contact(contact, resp);
        self.store.save(contact)
    }

    /// DestroyContactTask's remote phase for one contact. Contacts that did not come from
    /// the People API, or never reached it, are left alone.
    pub fn delete_contact(&self, contact: &mut Contact) -> Result<()> {
        if contact.source() != GOOGLE_SYNC_SOURCE || contact.google_resource_name().is_empty() {
            return Ok(());
        }
        let client = self.client()?;
        client.delete_contact(contact.google_resource_name())?;
        self.store.remove(contact)
    }

    /// SyncbackContactGroupTask's remote phase: renames the group when it has a resource
    /// name, creates it otherwise, and stores the resource name the server answers with.
    pub fn upsert_contact_group(&self, group: &mut ContactGroup) -> Result<()> {
        let client = self.client()?;
        let resp = if group.google_resource_name().is_empty() {
            client.create_group(group.name())?
        } else {
            client.rename_group(group.google_resource_name(), group.name())?
        };
        info!("Response {}", resp.raw);
        group.set_google_resource_name(require_resource_name(&resp.resource_name)?);
        self.store.save(group)
    }

    /// DestroyContactGroupTask's remote phase.
    pub fn delete_contact_group(&self, group_resource_name: &str) -> Result<()> {
        self.client()?.delete_group(group_resource_name)?;
        info!("- Deleted contact group {group_resource_name}");
        Ok(())
    }

    /// ChangeContactGroupMembershipTask's remote phase (`members:modify`).
    pub fn update_contact_group_membership(
        &self,
        group: &ContactGroup,
        contacts: &[Contact],
        direction: Membership,
    ) -> Result<()> {
        let resource_names: Vec<String> = contacts
            .iter()
            .map(|c| c.google_resource_name().to_owned())
            .filter(|rn| !rn.is_empty())
            .collect();
        self.client()?.modify_group_members(
            group.google_resource_name(),
            direction,
            &resource_names,
        )
    }
}

/// `applyJSONToContact`: the first name and email become the contact's, and the whole
/// person is kept as `info`, which the client edits and SyncbackContactTask sends back.
fn apply_person_to_contact(contact: &mut Contact, person: Person) {
    contact.set_email(&person.email);
    contact.set_name(&person.display_name);
    contact.set_info(person.raw);
}

/// `resp["resourceName"].get<string>()`, which throws on a response without one.
fn require_resource_name(rn: &str) -> Result<&str> {
    if rn.is_empty() {
        return Err(Error::other(
            "[json.exception.type_error.302] type must be string, but is null",
        ));
    }
    Ok(rn)
}
