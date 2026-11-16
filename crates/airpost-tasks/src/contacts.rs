// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Contact and contact-group tasks: SyncbackContact, DestroyContact, SyncbackContactGroup,
//! DestroyContactGroup and ChangeContactGroupMembership.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp` (`performLocalDestroyContact`
//! through `performRemoteSyncbackContact`), Copyright © 2017 Foundry 376, GPL-3.0. See
//! NOTICE.md.
//!
//! A Gmail account's contacts live in the People API ([`GoogleContactsWorker`]); every
//! other account's in CardDAV ([`DavWorker`]), where a contact group is a hidden group card
//! plus a ContactGroup with the card's id ([`rebuild_contact_group`]).
//!
//! Differences from the C++:
//! - Where the C++ dereferenced a model it did not find (a group or contact deleted before
//!   the task ran), the task fails with `not-found` instead.
//! - A new CardDAV contact's card gets the contact's id as its UID, so the card written to
//!   `<uid>.vcf` and the row keep one identity; C++ kept whatever
//!   UID the client's card had and POSTed it.
//! - When the remote phase of DestroyContact fails, the contacts it did not delete are
//!   shown again, and a CardDAV DestroyContactGroup rebuilds the group from its card; C++
//!   left them hidden or removed although they still existed on the server.

use airpost_core::constants::{
    CARDDAV_SYNC_SOURCE, CONTACT_MAX_REFS, CONTACT_SOURCE_MAIL, GOOGLE_SYNC_SOURCE,
};
use airpost_core::{ids, Error, Result, SyncError};
use airpost_dav::VCard;
use airpost_net::people::Membership;
use airpost_store::model::json_str;
use airpost_store::{Contact, ContactGroup, Model, Query};
use airpost_sync::dav::{rebuild_contact_group, DavWorker};
use airpost_sync::google_contacts::{find_or_create_book, GoogleContactsWorker};
use serde_json::{json, Value};
use tracing::error;

use crate::json::{at, elements, get_str, has};
use crate::{unsure_how_to_process, TaskProcessor};

/// The task classes this module handles.
pub const CONTACT_TASKS: [&str; 5] = [
    "DestroyContactTask",
    "SyncbackContactTask",
    "ChangeContactGroupMembershipTask",
    "SyncbackContactGroupTask",
    "DestroyContactGroupTask",
];

/// `for (json & c : data["contacts"]) ids.push_back(c["id"].get<string>())`.
fn contact_ids(data: &Value) -> Result<Vec<String>> {
    elements(at(data, "contacts"))
        .into_iter()
        .map(|c| get_str(at(c, "id")))
        .collect()
}

/// Runs `delete` on each contact in order; on a failure, the index of the first contact
/// not deleted (the failing one) and the error.
fn delete_each(
    contacts: &mut [Contact],
    mut delete: impl FnMut(&mut Contact) -> Result<()>,
) -> std::result::Result<(), (usize, Error)> {
    for (i, c) in contacts.iter_mut().enumerate() {
        delete(c).map_err(|e| (i, e))?;
    }
    Ok(())
}

pub(crate) fn not_found(what: &str) -> Error {
    SyncError::fatal("not-found", what).into()
}

impl TaskProcessor<'_> {
    fn uses_google_contacts(&self) -> bool {
        self.account.provider() == "gmail"
    }

    fn google_contacts(&self) -> Result<GoogleContactsWorker<'_>> {
        GoogleContactsWorker::new(self.account.clone(), self.store, self.tokens.clone())
    }

    pub(crate) fn dav(&self) -> Result<DavWorker<'_>> {
        if let Some(connect) = &self.dav_connector {
            let (client, resolver) = connect();
            return Ok(DavWorker::with_client(
                self.account.clone(),
                self.store,
                client,
                resolver,
            ));
        }
        DavWorker::new(
            self.account.clone(),
            self.store,
            self.tokens.clone(),
            self.verbose,
        )
    }

    /// `find<Contact>(id, accountId)`, which the C++ dereferenced unchecked.
    fn find_account_contact(&self, id: &str, what: &str) -> Result<Contact> {
        self.store
            .find::<Contact>(
                &Query::new()
                    .equal("id", id)
                    .equal("accountId", self.account.id()),
            )?
            .ok_or_else(|| not_found(what))
    }

    fn find_group(&self, id: &str) -> Result<ContactGroup> {
        self.store
            .find::<ContactGroup>(&Query::new().equal("id", id))?
            .ok_or_else(|| not_found("ContactGroup not found"))
    }

    pub(crate) fn perform_local_contact_task(&self, class: &str, data: &mut Value) -> Result<()> {
        match class {
            "DestroyContactTask" => self.perform_local_destroy_contact(data),
            "SyncbackContactTask" => self.perform_local_syncback_contact(data),
            "SyncbackContactGroupTask" => self.perform_local_syncback_contact_group(data),
            "DestroyContactGroupTask" => self.perform_local_destroy_contact_group(data),
            "ChangeContactGroupMembershipTask" => {
                self.perform_local_change_contact_group_membership(data)
            }
            other => Err(Error::other(unsure_how_to_process(other))),
        }
    }

    pub(crate) fn perform_remote_contact_task(&self, class: &str, data: &Value) -> Result<()> {
        match class {
            "DestroyContactTask" => self.perform_remote_destroy_contact(data),
            "SyncbackContactTask" => self.perform_remote_syncback_contact(data),
            "SyncbackContactGroupTask" => self.perform_remote_syncback_contact_group(data),
            "DestroyContactGroupTask" => self.perform_remote_destroy_contact_group(data),
            "ChangeContactGroupMembershipTask" => {
                self.perform_remote_change_contact_group_membership(data)
            }
            other => Err(Error::other(unsure_how_to_process(other))),
        }
    }

    /// Hides the contacts until the remote phase deletes them.
    fn perform_local_destroy_contact(&self, data: &Value) -> Result<()> {
        let ids = contact_ids(data)?;
        self.store.transaction("performLocalDestroyContact", |s| {
            for mut c in s.find_large_set::<Contact>("id", &ids)? {
                c.set_hidden(true);
                s.save(&mut c)?;
            }
            Ok(())
        })
    }

    /// Deletes the contacts in order and stops at the first failure, as the C++ did. The
    /// contacts not deleted by then are unhidden: they still exist on the server, and
    /// nothing else would show them again.
    fn perform_remote_destroy_contact(&self, data: &Value) -> Result<()> {
        let ids = contact_ids(data)?;
        let mut contacts = self.store.find_large_set::<Contact>("id", &ids)?;
        let result = if self.uses_google_contacts() {
            let worker = self.google_contacts()?;
            delete_each(&mut contacts, |c| worker.delete_contact(c))
        } else {
            let mut dav = self.dav()?;
            delete_each(&mut contacts, |c| dav.delete_contact(c))
        };
        let Err((first_kept, e)) = result else {
            return Ok(());
        };
        if !matches!(e, Error::Database(_)) {
            let kept: Vec<String> = contacts[first_kept..]
                .iter()
                .map(|c| c.id().to_string())
                .collect();
            self.unhide_contacts(&kept)?;
        }
        Err(e)
    }

    fn unhide_contacts(&self, ids: &[String]) -> Result<()> {
        self.store.transaction("unhideContacts", |s| {
            for mut c in s.find_large_set::<Contact>("id", ids)? {
                // A group card stays hidden: its ContactGroup is what the client shows.
                let is_group_card = c.source() == CARDDAV_SYNC_SOURCE
                    && VCard::parse(json_str(c.info(), "vcf")).is_group();
                if c.hidden() && !is_group_card {
                    c.set_hidden(false);
                    s.save(&mut c)?;
                }
            }
            Ok(())
        })
    }

    /// Creates or updates the contact from the client's copy. The client may not know every
    /// key the engine stores on a contact, so only `info`, `name` and `email` are taken from
    /// it. A new contact's id goes back into the task for the remote phase.
    fn perform_local_syncback_contact(&self, data: &mut Value) -> Result<()> {
        let json = at(data, "contact").clone();
        let clientside = Contact::from_json(json.clone())?;
        let source = if self.uses_google_contacts() {
            GOOGLE_SYNC_SOURCE
        } else {
            CARDDAV_SYNC_SOURCE
        };
        if !clientside.source().is_empty() && clientside.source() != source {
            error!(
                "performLocalSyncbackContact: Client picked incorrect source for new contact: {source} != {}",
                clientside.source()
            );
            return Ok(());
        }

        let is_new = !has(&json, "id");
        let mut local = if is_new {
            Contact::create(
                &ids::random_id(),
                self.account.id(),
                "",
                CONTACT_MAX_REFS,
                source,
            )
        } else {
            let id = get_str(at(&json, "id"))?;
            self.store
                .find::<Contact>(&Query::new().equal("id", &id))?
                .ok_or_else(|| not_found("Contact not found for syncback"))?
        };
        local.set_info(at(&json, "info").clone());
        if is_new && source == CARDDAV_SYNC_SOURCE {
            let id = local.id().to_owned();
            local.mutate_vcf_in_info(|vcf| {
                if vcf.is_empty() {
                    return None;
                }
                let mut card = VCard::parse(vcf);
                card.set_value("UID", &id);
                Some(card.serialize())
            });
        }
        local.set_name(&get_str(at(&json, "name"))?);
        local.set_email(&get_str(at(&json, "email"))?);
        self.store.save(&mut local)?;

        data["contact"]["id"] = Value::from(local.id());
        Ok(())
    }

    fn perform_remote_syncback_contact(&self, data: &Value) -> Result<()> {
        let id = get_str(at(at(data, "contact"), "id"))?;
        let query = Query::new()
            .equal("id", &id)
            .equal("accountId", self.account.id());
        let mut contact = self
            .store
            .find::<Contact>(&query)?
            .ok_or_else(|| not_found("Contact not found for syncback"))?;
        if contact.source() == CONTACT_SOURCE_MAIL {
            return Ok(());
        }
        if self.uses_google_contacts() {
            self.google_contacts()?.upsert_contact(&mut contact)
        } else {
            self.dav()?.write_and_resync_contact(&mut contact)
        }
    }

    /// Creates or renames the group locally. A new group's id goes back into the task.
    fn perform_local_syncback_contact_group(&self, data: &mut Value) -> Result<()> {
        if !self.uses_google_contacts() {
            return self.perform_local_syncback_dav_contact_group(data);
        }
        let group = at(data, "group");
        let mut id = if has(group, "id") {
            get_str(at(group, "id"))?
        } else {
            String::new()
        };
        let name = get_str(at(group, "name"))?;
        // The C++ read a book the contacts pass had created; a task queued before the first
        // pass finds it missing, so it is created here with the same id.
        let book = find_or_create_book(self.store, &self.account)?;

        if id.is_empty() {
            id = ids::random_id();
            data["group"]["id"] = Value::from(id.as_str());
        }
        let mut local = match self
            .store
            .find::<ContactGroup>(&Query::new().equal("id", &id))?
        {
            Some(g) => g,
            None => ContactGroup::create(&id, self.account.id()),
        };
        local.set_book_id(book.id());
        local.set_name(&name);
        self.store.save(&mut local)
    }

    fn perform_remote_syncback_contact_group(&self, data: &Value) -> Result<()> {
        let group = at(data, "group");
        let id = if has(group, "id") {
            get_str(at(group, "id"))?
        } else {
            String::new()
        };
        if id.is_empty() {
            error!("performRemoteSyncbackContactGroup: Group did not get assigned an ID.");
            return Ok(());
        }
        if self.uses_google_contacts() {
            let mut group = self.find_group(&id)?;
            self.google_contacts()?.upsert_contact_group(&mut group)
        } else {
            let mut contact =
                self.find_account_contact(&id, "Contact not found for group syncback")?;
            self.dav()?.write_and_resync_contact(&mut contact)
        }
    }

    /// The CardDAV group: a rename updates the ContactGroup and its card; a new group is a
    /// new hidden group card, whose UID is the group's id.
    fn perform_local_syncback_dav_contact_group(&self, data: &mut Value) -> Result<()> {
        let group = at(data, "group");
        let id = if has(group, "id") {
            get_str(at(group, "id"))?
        } else {
            String::new()
        };
        let name = get_str(at(group, "name"))?;

        if !id.is_empty() {
            let Some(mut existing) = self
                .store
                .find::<ContactGroup>(&Query::new().equal("id", &id))?
            else {
                return Ok(());
            };
            existing.set_name(&name);
            self.store.save(&mut existing)?;

            let mut contact = self
                .store
                .find::<Contact>(&Query::new().equal("id", &id))?
                .ok_or_else(|| not_found("Contact not found for group syncback"))?;
            contact.set_name(&name);
            contact.mutate_vcf_in_info(|vcf| {
                let mut card = VCard::parse(vcf);
                if card.incomplete() {
                    return None;
                }
                card.rename(&name);
                Some(card.serialize())
            });
            return self.store.save(&mut contact);
        }

        let uid = ids::random_id();
        let mut contact = Contact::create(
            &uid,
            self.account.id(),
            "",
            CONTACT_MAX_REFS,
            CARDDAV_SYNC_SOURCE,
        );
        contact.set_info(json!({"vcf": VCard::new_group(&uid, &name).serialize(), "href": ""}));
        contact.set_hidden(true);
        contact.set_name(&name);
        // No bookId until the read-back: the etag listing deletes every card of the book
        // whose etag the server does not list, and this card's is still empty.
        data["group"]["id"] = Value::from(uid.as_str());
        self.store.save(&mut contact)?;
        rebuild_contact_group(self.store, self.account.id(), &contact)
    }

    /// Removes the group locally and keeps its resource name in the task: the remote phase
    /// has no row left to read it from.
    fn perform_local_destroy_contact_group(&self, data: &mut Value) -> Result<()> {
        let id = get_str(at(at(data, "group"), "id"))?;
        let Some(mut deleted) = self
            .store
            .find::<ContactGroup>(&Query::new().equal("id", &id))?
        else {
            return Ok(());
        };
        if self.uses_google_contacts() {
            data["googleResourceName"] = Value::from(deleted.google_resource_name());
        }
        self.store.remove(&mut deleted)
    }

    fn perform_remote_destroy_contact_group(&self, data: &Value) -> Result<()> {
        let id = get_str(at(at(data, "group"), "id"))?;
        if !self.uses_google_contacts() {
            let Some(mut contact) = self.store.find::<Contact>(&Query::new().equal("id", &id))?
            else {
                return Ok(());
            };
            return match self.dav()?.delete_contact(&mut contact) {
                Err(e) if !matches!(e, Error::Database(_)) => {
                    // The card is still on the server; show the group again.
                    rebuild_contact_group(self.store, self.account.id(), &contact)?;
                    Err(e)
                }
                other => other,
            };
        }
        if !has(data, "googleResourceName") {
            error!("performRemoteDestroyContactGroup: Group did not have a googleResourceName.");
            return Ok(());
        }
        let resource_name = get_str(at(data, "googleResourceName"))?;
        self.google_contacts()?.delete_contact_group(&resource_name)
    }

    fn perform_local_change_contact_group_membership(&self, data: &Value) -> Result<()> {
        let ids = contact_ids(data)?;
        let direction = get_str(at(data, "direction"))?;
        let group_id = get_str(at(at(data, "group"), "id"))?;
        if !self.uses_google_contacts() {
            // The members are the group card's; the ContactGroup is rebuilt from it.
            let contacts = self.store.find_large_set::<Contact>("id", &ids)?;
            let member_ids: Vec<&str> = contacts.iter().map(|c| c.id()).collect();
            let mut card_contact =
                self.find_account_contact(&group_id, "Contact not found for group membership")?;
            card_contact.mutate_vcf_in_info(|vcf| {
                let mut card = VCard::parse(vcf);
                if card.incomplete() {
                    return None;
                }
                if direction == "add" {
                    card.add_members(&member_ids);
                } else {
                    card.remove_members(&member_ids);
                }
                Some(card.serialize())
            });
            rebuild_contact_group(self.store, self.account.id(), &card_contact)?;
            return self.store.save(&mut card_contact);
        }
        let group = self.find_group(&group_id)?;
        let mut members = group.members(self.store)?;
        if direction == "add" {
            members.extend(ids);
        } else {
            for id in &ids {
                if let Some(pos) = members.iter().position(|m| m == id) {
                    members.remove(pos);
                }
            }
        }
        group.sync_members(self.store, &members)
    }

    fn perform_remote_change_contact_group_membership(&self, data: &Value) -> Result<()> {
        let group_id = get_str(at(at(data, "group"), "id"))?;
        if !self.uses_google_contacts() {
            let mut card_contact =
                self.find_account_contact(&group_id, "Contact not found for group membership")?;
            return self.dav()?.write_and_resync_contact(&mut card_contact);
        }
        let direction = match get_str(at(data, "direction"))?.as_str() {
            "add" => Membership::Add,
            _ => Membership::Remove,
        };
        let ids = contact_ids(data)?;
        let contacts = self.store.find_large_set::<Contact>("id", &ids)?;
        let group = self.find_group(&group_id)?;
        self.google_contacts()?
            .update_contact_group_membership(&group, &contacts, direction)
    }
}
