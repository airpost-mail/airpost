// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Folder and label tasks: SyncbackCategory, DestroyCategory, ChangeRoleMapping and
//! ExpungeAllInFolder.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp`
//! (`performRemoteSyncbackCategory`, `performRemoteDestroyCategory`,
//! `performLocalChangeRoleMapping`, `performRemoteExpungeAllInFolder`) and
//! `MailUtils::namespacePrefixOrBlank`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::time::Duration;

use airpost_core::{MessageFlags, Result, SyncError, UidSet};
use airpost_imap::{ImapSession, StoreOp};
use airpost_store::{Folder, Label, Model, Query, Store};
use airpost_sync::folder_identity;
use airpost_sync::mail_processor::MailProcessor;
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

    fn path(&self) -> &str {
        match self {
            Category::Folder(f) => f.path(),
            Category::Label(l) => l.path(),
        }
    }

    fn id(&self) -> &str {
        match self {
            Category::Folder(f) => f.id(),
            Category::Label(l) => l.id(),
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
    /// The local row is found by path, since after an earlier rename its id is no longer
    /// the hash of the path. A renamed row keeps its id.
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

        let renaming = !existing_path.is_empty();
        if renaming {
            folder_identity::record_rename(self.store, account_id, &existing_path, &path)?;
        }
        let result = if renaming {
            session.rename_folder(&existing_path, &path)
        } else {
            session.create_folder(&path)
        };
        if let Err(e) = result {
            if renaming {
                if let Err(forget) =
                    folder_identity::forget_rename(self.store, account_id, &existing_path, &path)
                {
                    error!("-X could not drop the rename record: {forget}");
                }
            }
            data["created"] = Value::Null;
            error!("Syncback of folder/label '{path}' failed.");
            return Err(e.into_sync("create/renameFolder").into());
        }

        let gmail = is_gmail(session);
        let by_path = |p: &str| -> Result<Option<Category>> {
            let query = Query::new().equal("accountId", account_id).equal("path", p);
            Ok(if gmail {
                self.store.find::<Label>(&query)?.map(Category::Label)
            } else {
                self.store.find::<Folder>(&query)?.map(Category::Folder)
            })
        };
        let mut local = if renaming {
            by_path(&existing_path)?
        } else {
            None
        };
        let found = local.is_some();
        if local.is_none() {
            local = by_path(&path)?;
        }
        let mut local = match local {
            Some(c) => c,
            None => {
                let id =
                    airpost_core::ids::free_folder_id(account_id, &path, |id| self.id_taken(id));
                if gmail {
                    Category::Label(Label::new(&id, account_id, 0))
                } else {
                    Category::Folder(Folder::new(&id, account_id, 0))
                }
            }
        };
        local.set_path(&path);
        data["created"] = local.to_json();
        let d = delimiter.chars().next();
        self.store
            .transaction("performRemoteSyncbackCategory", |s| {
                local.save(s)?;
                if renaming && found {
                    folder_identity::queue_refresh(s, account_id, local.id(), &existing_path)?;
                    self.rename_children(s, account_id, &existing_path, &path, d, gmail)?;
                }
                Ok(())
            })?;
        folder_identity::run_pending_refreshes(self.store, account_id)?;
        info!("Syncback of folder/label '{path}' succeeded.");
        Ok(())
    }

    /// Whether a Folder or Label row holds `id`.
    fn id_taken(&self, id: &str) -> bool {
        let q = Query::new().equal("id", id);
        // Note: a failed lookup counts as free; a clash then fails the save, not the loop.
        self.store.count::<Folder>(&q).unwrap_or(0) > 0
            || self.store.count::<Label>(&q).unwrap_or(0) > 0
    }

    /// Gives the rows inside a renamed folder their new paths, as the server renamed them
    /// with it, keeping their ids.
    fn rename_children(
        &self,
        s: &Store,
        account_id: &str,
        from: &str,
        to: &str,
        delimiter: Option<char>,
        gmail: bool,
    ) -> Result<()> {
        let Some(d) = delimiter else {
            return Ok(());
        };
        let query = Query::new().equal("accountId", account_id);
        let children: Vec<Category> = if gmail {
            s.find_all::<Label>(&query)?
                .into_iter()
                .map(Category::Label)
                .collect()
        } else {
            s.find_all::<Folder>(&query)?
                .into_iter()
                .map(Category::Folder)
                .collect()
        };
        let prefix = format!("{from}{d}");
        for mut child in children {
            let old_path = child.path().to_owned();
            if !old_path.starts_with(&prefix) {
                continue;
            }
            let Some(new_path) = folder_identity::renamed_path(&old_path, from, to, Some(d)) else {
                continue;
            };
            child.set_path(&new_path);
            child.save(s)?;
            folder_identity::queue_refresh(s, account_id, child.id(), &old_path)?;
        }
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
