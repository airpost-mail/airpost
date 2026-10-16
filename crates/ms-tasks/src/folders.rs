// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Folder and label tasks: SyncbackCategory, DestroyCategory, ChangeRoleMapping and
//! ExpungeAllInFolder.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp`
//! (`performRemoteSyncbackCategory`, `performRemoteDestroyCategory`,
//! `performLocalChangeRoleMapping`, `performRemoteExpungeAllInFolder`) and
//! `MailUtils::namespacePrefixOrBlank`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::time::Duration;

use ms_core::{MessageFlags, Result, SyncError, UidSet};
use ms_imap::{ImapSession, StoreOp};
use ms_store::{Folder, Label, Model, Query, Store};
use ms_sync::mail_processor::MailProcessor;
use serde_json::Value;
use tracing::{error, info};

use crate::json::{at, get_str, has};
use crate::messages::is_gmail;
use crate::TaskProcessor;

/// The client keeps up with a mass deletion when the chunks are spaced out.
const EXPUNGE_ALL_CHUNK_PAUSE: Duration = Duration::from_millis(300);

/// A Folder or, on Gmail, a Label: the two tables share a shape.
enum Category {
    Folder(Folder),
    Label(Label),
}

impl Category {
    fn find(store: &Store, query: &Query) -> Result<Option<Category>> {
        if let Some(f) = store.find::<Folder>(query)? {
            return Ok(Some(Category::Folder(f)));
        }
        Ok(store.find::<Label>(query)?.map(Category::Label))
    }

    fn set_role(&mut self, role: &str) {
        match self {
            Category::Folder(f) => f.set_role(role),
            Category::Label(l) => l.set_role(role),
        }
    }

    fn set_path(&mut self, path: &str) {
        match self {
            Category::Folder(f) => f.set_path(path),
            Category::Label(l) => l.set_path(path),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Category::Folder(f) => f.to_json(),
            Category::Label(l) => l.to_json(),
        }
    }

    fn save(&mut self, store: &Store) -> Result<()> {
        match self {
            Category::Folder(f) => store.save(f),
            Category::Label(l) => store.save(l),
        }
    }
}

impl TaskProcessor<'_> {
    pub(crate) fn perform_remote_syncback_category(
        &self,
        account_id: &str,
        data: &mut Value,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let mut path = get_str(at(data, "path"))?;
        let existing_path = if has(data, "existingPath") {
            get_str(at(data, "existingPath"))?
        } else {
            String::new()
        };

        // The client writes `/` between components and may leave out the namespace prefix.
        let (prefix, delimiter) = match session.default_namespace() {
            Some(ns) => (ns.prefix.clone(), ns.delimiter),
            None => (String::new(), None),
        };
        let delimiter = delimiter.map(String::from).unwrap_or_default();
        if !delimiter.is_empty() {
            path = path.replace('/', &delimiter);
        }
        if !prefix.is_empty() && !path.starts_with(&prefix) {
            path = if !delimiter.is_empty() && prefix.ends_with(&delimiter) {
                format!("{prefix}{path}")
            } else {
                format!("{prefix}{delimiter}{path}")
            };
        }

        let result = if existing_path.is_empty() {
            session.create_folder(&path)
        } else {
            session.rename_folder(&existing_path, &path)
        };
        if let Err(e) = result {
            data["created"] = Value::Null;
            error!("Syncback of folder/label '{path}' failed.");
            return Err(e.into_sync("create/renameFolder").into());
        }

        let gmail = is_gmail(session);
        let mut local = None;
        if !existing_path.is_empty() {
            let query = Query::new()
                .equal("accountId", account_id)
                .equal("id", ms_core::ids::folder_id(account_id, &existing_path));
            local = if gmail {
                self.store.find::<Label>(&query)?.map(Category::Label)
            } else {
                self.store.find::<Folder>(&query)?.map(Category::Folder)
            };
        }
        let mut local = local.unwrap_or_else(|| {
            let id = ms_core::ids::folder_id(account_id, &path);
            if gmail {
                Category::Label(Label::new(&id, account_id, 0))
            } else {
                Category::Folder(Folder::new(&id, account_id, 0))
            }
        });
        local.set_path(&path);
        data["created"] = local.to_json();
        local.save(self.store)?;
        info!("Syncback of folder/label '{path}' succeeded.");
        Ok(())
    }

    pub(crate) fn perform_remote_destroy_category(
        &self,
        data: &Value,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let path = get_str(at(data, "path"))?;
        session
            .delete_folder(&path)
            .map_err(|e| e.into_sync("deleteFolder"))?;
        info!("Deletion of folder/label '{path}' succeeded.");
        Ok(())
    }

    pub(crate) fn perform_local_change_role_mapping(
        &self,
        account_id: &str,
        data: &Value,
    ) -> Result<()> {
        let path = get_str(at(data, "path"))?;
        let role = get_str(at(data, "role"))?;
        self.store
            .transaction("performLocalChangeRoleMapping", |s| {
                let query = Query::new()
                    .equal("accountId", account_id)
                    .equal("path", path.as_str());
                let Some(mut category) = Category::find(s, &query)? else {
                    return Err(SyncError::fatal("no-matching-folder", "").into());
                };
                // Only one category holds a role.
                let existing_query = Query::new()
                    .equal("accountId", account_id)
                    .equal("role", role.as_str());
                if let Some(mut existing) = Category::find(s, &existing_query)? {
                    existing.set_role("");
                    existing.save(s)?;
                }
                category.set_role(&role);
                category.save(s)
            })
    }

    pub(crate) fn perform_remote_expunge_all_in_folder(
        &self,
        data: &Value,
        session: &mut dyn ImapSession,
    ) -> Result<()> {
        let folder = at(data, "folder");
        let path = get_str(at(folder, "path"))?;
        let id = get_str(at(folder, "id"))?;

        session
            .store_flags(
                &path,
                &UidSet::range(1, None),
                StoreOp::Add,
                MessageFlags::DELETED,
            )
            .map_err(|e| e.into_sync("storeFlagsByUID"))?;
        session.expunge(&path).map_err(|e| e.into_sync("expunge"))?;
        info!("-- Expunged {path}");

        // Runs here rather than in the local phase because it takes too long there.
        let mut processor = MailProcessor::new(&self.account, self.store);
        processor.is_gmail = is_gmail(session);
        processor.detach_messages_from_folder(&id, EXPUNGE_ALL_CHUNK_PAUSE)
    }
}
