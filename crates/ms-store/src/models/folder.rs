//! `Folder` and `Label`. Derived from Mailspring-Sync `MailSync/Models/Folder.*` and
//! `Label.*`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! A Label is a Folder in a different table: same JSON shape, same columns, same hooks.
//! Both get a `ThreadCounts` row on their first save, removed with them.

use ms_core::Result;
use rusqlite::Statement;
use serde_json::Value;

use crate::model::{base_json, bind_base, bind_named, json_str, object_or_err};
use crate::{Model, Store};

fn ensure_thread_counts(id: &str, version: i64, store: &Store) -> Result<()> {
    if version == 1 {
        store
            .conn()
            .prepare_cached(
                "INSERT OR IGNORE INTO ThreadCounts (categoryId, unread, total) VALUES (?, 0, 0)",
            )?
            .execute([id])?;
    }
    Ok(())
}

fn remove_thread_counts(id: &str, store: &Store) -> Result<()> {
    store
        .conn()
        .prepare_cached("DELETE FROM ThreadCounts WHERE categoryId = ?")?
        .execute([id])?;
    Ok(())
}

macro_rules! folder_like {
    ($(#[$doc:meta])* $name:ident, $table:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            pub data: Value,
        }

        impl $name {
            /// `Folder(id, accountId, version)`: empty path and role.
            pub fn new(id: &str, account_id: &str, version: i64) -> Self {
                let mut data = base_json(id, account_id, version);
                data["path"] = Value::from("");
                data["role"] = Value::from("");
                $name { data }
            }

            pub fn path(&self) -> &str {
                json_str(&self.data, "path")
            }

            pub fn set_path(&mut self, path: &str) {
                self.data["path"] = Value::from(path);
            }

            pub fn role(&self) -> &str {
                json_str(&self.data, "role")
            }

            pub fn set_role(&mut self, role: &str) {
                self.data["role"] = Value::from(role);
            }

            /// `localStatus`: sync progress (`busy`, `syncedMinUID`, `uidnext`, `lastDeep`, …).
            /// The harness rewrites parts of it in the live database, so workers re-read it
            /// every pass instead of caching it.
            pub fn local_status(&self) -> &Value {
                &self.data["localStatus"]
            }

            /// The `localStatus` object, created empty when missing.
            pub fn local_status_mut(&mut self) -> &mut serde_json::Map<String, Value> {
                if !self.data["localStatus"].is_object() {
                    self.data["localStatus"] = Value::Object(Default::default());
                }
                self.data["localStatus"].as_object_mut().expect("object")
            }
        }

        impl Model for $name {
            model_boilerplate!($table, ["id", "data", "accountId", "version", "path", "role"]);

            fn from_json(data: Value) -> Result<Self> {
                object_or_err(&data, $table)?;
                Ok($name { data })
            }

            fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
                bind_base(self, stmt)?;
                bind_named(stmt, ":path", self.path())?;
                bind_named(stmt, ":role", self.role())?;
                Ok(())
            }

            fn before_save(&mut self, store: &Store) -> Result<()> {
                ensure_thread_counts(self.id(), self.version(), store)
            }

            fn after_remove(&mut self, store: &Store) -> Result<()> {
                remove_thread_counts(self.id(), store)
            }
        }
    };
}

folder_like!(
    /// An IMAP folder. Placements (`MessageFolder` rows) live in folders only.
    Folder,
    "Folder"
);
folder_like!(
    /// A Gmail label. Labels are never placement folders; X-GM-LABELS on the one Gmail
    /// placement say which labels a message carries.
    Label,
    "Label"
);

impl From<Label> for Folder {
    /// The folder cache holds labels as folders (`folderById` resolves both).
    fn from(label: Label) -> Folder {
        let mut f = Folder { data: label.data };
        f.ensure_cls_as("Label");
        f
    }
}

impl Folder {
    fn ensure_cls_as(&mut self, cls: &str) {
        if let Some(obj) = self.data.as_object_mut() {
            obj.entry("__cls").or_insert_with(|| Value::from(cls));
        }
    }
}
