// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The persisted models (`Models/*.cpp`). Each wraps its JSON and exposes typed accessors
//! over it; the save/remove hooks keep the derived tables (thread refcounts,
//! ThreadCategory, ThreadCounts, ThreadSearch, ContactSearch, EventSearch,
//! ModelPluginMetadata) in step.
//!
//! Derived from Mailspring-Sync `MailSync/Models/*`, Copyright © 2017 Foundry 376, GPL-3.0.
//! See NOTICE.md.

/// The `Model` items every JSON-only model implements the same way.
macro_rules! model_boilerplate {
    ($table:literal, [$($col:literal),* $(,)?]) => {
        const TABLE: &'static str = $table;

        fn columns() -> &'static [&'static str] {
            &[$($col),*]
        }

        fn json(&self) -> &serde_json::Value {
            &self.data
        }

        fn json_mut(&mut self) -> &mut serde_json::Value {
            &mut self.data
        }
    };
}

mod calendar;
mod contact;
mod file;
mod folder;
pub(crate) mod generic;
mod message;
mod task;
mod thread;

pub use calendar::{Calendar, Event, IcsEventFields};
pub use contact::{Contact, ContactBook, ContactGroup};
pub use file::{File, FilePart, TMP_DIR as FILES_TMP_DIR};
pub use folder::{Folder, Label};
pub use generic::GenericModel;
pub use message::{is_hidden_json, IncomingMessage, Message, MessageSnapshot};
pub use task::Task;
pub use thread::{label_for_xgm_label_name, Thread, SEND_REMINDERS_PLUGIN};
