// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `Contact`, `ContactGroup` and `ContactBook`. Derived from Mailspring-Sync
//! `MailSync/Models/Contact.*`, `ContactGroup.*` and `ContactBook.*`, Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md.

use ms_core::constants::{CONTACT_SOURCE_MAIL, FIND_LARGE_SET_CHUNK};
use ms_core::Result;
use rusqlite::Statement;
use serde_json::{json, Value};

use crate::model::{
    base_json, bind_base, bind_named, json_bool, json_i64, json_str, object_or_err,
};
use crate::{Model, Query, Store};

#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub data: Value,
}

impl Contact {
    /// `Contact(id, accountId, email, refs, source)`.
    pub fn create(id: &str, account_id: &str, email: &str, refs: i64, source: &str) -> Self {
        let mut data = base_json(id, account_id, 0);
        data["email"] = json!(email);
        data["s"] = json!(source);
        data["refs"] = json!(refs);
        data["gis"] = json!([]);
        data["info"] = json!({});
        data["name"] = json!("");
        Contact { data }
    }

    pub fn name(&self) -> &str {
        json_str(&self.data, "name")
    }

    pub fn set_name(&mut self, name: &str) {
        self.data["name"] = json!(name);
    }

    /// `grn`: the Google People resource name.
    pub fn google_resource_name(&self) -> &str {
        json_str(&self.data, "grn")
    }

    pub fn set_google_resource_name(&mut self, rn: &str) {
        self.data["grn"] = json!(rn);
    }

    pub fn email(&self) -> &str {
        json_str(&self.data, "email")
    }

    pub fn set_email(&mut self, email: &str) {
        self.data["email"] = json!(email);
    }

    /// `s`: `mail`, `gpeople` or `carddav`; `mail` when absent.
    pub fn source(&self) -> &str {
        self.data
            .get("s")
            .and_then(Value::as_str)
            .unwrap_or(CONTACT_SOURCE_MAIL)
    }

    pub fn etag(&self) -> &str {
        json_str(&self.data, "etag")
    }

    pub fn set_etag(&mut self, etag: &str) {
        self.data["etag"] = json!(etag);
    }

    /// `bid`: the ContactBook id.
    pub fn book_id(&self) -> &str {
        json_str(&self.data, "bid")
    }

    pub fn set_book_id(&mut self, id: &str) {
        self.data["bid"] = json!(id);
    }

    /// `info`: the source's own representation, e.g. `{"vcf": "BEGIN:VCARD…"}`.
    pub fn info(&self) -> &Value {
        &self.data["info"]
    }

    pub fn set_info(&mut self, info: Value) {
        self.data["info"] = info;
    }

    /// The store half of `mutateCardInInfo`: hands `info.vcf` to `mutate`, which parses the
    /// card, changes it and returns it serialized, or returns `None` when the card is
    /// incomplete, leaving `info` untouched. The vCard type lives in ms-dav, which this
    /// crate does not depend on. Returns whether `info.vcf` was rewritten.
    pub fn mutate_vcf_in_info(&mut self, mutate: impl FnOnce(&str) -> Option<String>) -> bool {
        let mut next = self.info().clone();
        let Some(vcf) = mutate(json_str(&next, "vcf")) else {
            return false;
        };
        if !next.is_object() {
            next = json!({});
        }
        next["vcf"] = json!(vcf);
        self.set_info(next);
        true
    }

    /// `h`.
    pub fn hidden(&self) -> bool {
        json_bool(&self.data, "h")
    }

    pub fn set_hidden(&mut self, h: bool) {
        self.data["h"] = json!(h);
    }

    /// `gis`: ContactGroup ids, without duplicates.
    pub fn group_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in self
            .data
            .get("gis")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !out.iter().any(|o| o == id) {
                out.push(id.to_owned());
            }
        }
        out
    }

    pub fn set_group_ids<I: IntoIterator<Item = String>>(&mut self, ids: I) {
        let mut out: Vec<String> = Vec::new();
        for id in ids {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        self.data["gis"] = json!(out);
    }

    pub fn refs(&self) -> i64 {
        json_i64(&self.data["refs"])
    }

    pub fn increment_refs(&mut self) {
        self.data["refs"] = json!(self.refs() + 1);
    }

    /// `ContactSearch.content`: the email with its `@` as a space, then the name.
    pub fn search_content(&self) -> String {
        let mut content = self.email().replacen('@', " ", 1);
        if self.data.get("name").is_some() {
            content.push(' ');
            content.push_str(self.name());
        }
        content
    }
}

impl Model for Contact {
    model_boilerplate!(
        "Contact",
        [
            "id",
            "data",
            "accountId",
            "version",
            "refs",
            "email",
            "hidden",
            "source",
            "etag",
            "bookId"
        ]
    );

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "Contact")?;
        Ok(Contact { data })
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_base(self, stmt)?;
        bind_named(stmt, ":refs", self.refs())?;
        bind_named(stmt, ":email", self.email())?;
        bind_named(stmt, ":hidden", self.hidden() as i64)?;
        bind_named(stmt, ":source", self.source())?;
        bind_named(stmt, ":etag", self.etag())?;
        bind_named(stmt, ":bookId", self.book_id())?;
        Ok(())
    }

    /// Mail-sourced contacts only change their refcount after creation, so their search
    /// text is written once.
    fn after_save(&mut self, store: &Store) -> Result<()> {
        let conn = store.conn();
        if self.version() == 1 {
            conn.prepare_cached("INSERT INTO ContactSearch (content_id, content) VALUES (?, ?)")?
                .execute(rusqlite::params![self.id(), self.search_content()])?;
        } else if self.source() != CONTACT_SOURCE_MAIL {
            conn.prepare_cached("UPDATE ContactSearch SET content = ? WHERE content_id = ?")?
                .execute(rusqlite::params![self.search_content(), self.id()])?;
        }
        Ok(())
    }

    fn after_remove(&mut self, store: &Store) -> Result<()> {
        store
            .conn()
            .prepare_cached("DELETE FROM ContactSearch WHERE content_id = ?")?
            .execute([self.id()])?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContactGroup {
    pub data: Value,
}

impl ContactGroup {
    pub fn create(id: &str, account_id: &str) -> Self {
        ContactGroup {
            data: base_json(id, account_id, 0),
        }
    }

    pub fn name(&self) -> &str {
        json_str(&self.data, "name")
    }

    pub fn set_name(&mut self, name: &str) {
        self.data["name"] = json!(name);
    }

    pub fn book_id(&self) -> &str {
        json_str(&self.data, "bid")
    }

    pub fn set_book_id(&mut self, id: &str) {
        self.data["bid"] = json!(id);
    }

    pub fn google_resource_name(&self) -> &str {
        json_str(&self.data, "grn")
    }

    pub fn set_google_resource_name(&mut self, rn: &str) {
        self.data["grn"] = json!(rn);
    }

    /// Contact ids in the group, from `ContactContactGroup`.
    pub fn members(&self, store: &Store) -> Result<Vec<String>> {
        let mut stmt = store
            .conn()
            .prepare_cached("SELECT id FROM ContactContactGroup WHERE value = ?")?;
        let ids = stmt
            .query_map([self.id()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// Replaces the membership (`syncMembers`) and rewrites `gis` on every contact that
    /// joined or left: the client's live queries only see membership on the models.
    pub fn sync_members(&self, store: &Store, new_contact_ids: &[String]) -> Result<()> {
        store.transaction("syncMembers", |store| {
            let old = self.members(store)?;
            let conn = store.conn();
            conn.prepare_cached("DELETE FROM ContactContactGroup WHERE value = ?")?
                .execute([self.id()])?;
            let mut insert = conn.prepare_cached(
                "INSERT OR IGNORE INTO ContactContactGroup (id, value) VALUES (?,?)",
            )?;
            for contact_id in new_contact_ids {
                insert.execute([contact_id.as_str(), self.id()])?;
            }
            drop(insert);

            let mut all = old;
            all.extend(new_contact_ids.iter().cloned());
            for chunk in all.chunks(FIND_LARGE_SET_CHUNK) {
                let contacts =
                    store.find_all::<Contact>(&Query::new().equal_any("id", chunk.iter()))?;
                for mut contact in contacts {
                    let mut groups = contact.group_ids();
                    let in_group = new_contact_ids.iter().any(|c| c == contact.id());
                    let listed = groups.iter().any(|g| g == self.id());
                    if in_group && !listed {
                        groups.push(self.id().to_owned());
                        contact.set_group_ids(groups);
                        store.save(&mut contact)?;
                    } else if !in_group && listed {
                        groups.retain(|g| g != self.id());
                        contact.set_group_ids(groups);
                        store.save(&mut contact)?;
                    }
                }
            }
            Ok(())
        })
    }
}

impl Model for ContactGroup {
    model_boilerplate!(
        "ContactGroup",
        ["id", "accountId", "version", "data", "name", "bookId"]
    );

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "ContactGroup")?;
        Ok(ContactGroup { data })
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_base(self, stmt)?;
        bind_named(stmt, ":name", self.name())?;
        bind_named(stmt, ":bookId", self.book_id())?;
        Ok(())
    }

    fn after_remove(&mut self, store: &Store) -> Result<()> {
        store
            .conn()
            .prepare_cached("DELETE FROM ContactContactGroup WHERE value = ?")?
            .execute([self.id()])?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContactBook {
    pub data: Value,
}

impl ContactBook {
    pub fn create(id: &str, account_id: &str) -> Self {
        let mut data = base_json(id, account_id, 0);
        data["source"] = json!("");
        data["url"] = json!("");
        ContactBook { data }
    }

    pub fn url(&self) -> &str {
        json_str(&self.data, "url")
    }

    pub fn set_url(&mut self, url: &str) {
        self.data["url"] = json!(url);
    }

    pub fn source(&self) -> &str {
        json_str(&self.data, "source")
    }

    pub fn set_source(&mut self, source: &str) {
        self.data["source"] = json!(source);
    }

    pub fn ctag(&self) -> &str {
        json_str(&self.data, "ctag")
    }

    pub fn set_ctag(&mut self, ctag: &str) {
        self.data["ctag"] = json!(ctag);
    }

    pub fn sync_token(&self) -> &str {
        json_str(&self.data, "syncToken")
    }

    pub fn set_sync_token(&mut self, token: &str) {
        self.data["syncToken"] = json!(token);
    }
}

impl Model for ContactBook {
    model_boilerplate!("ContactBook", ["id", "accountId", "version", "data"]);

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "ContactBook")?;
        Ok(ContactBook { data })
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_base(self, stmt)
    }
}
