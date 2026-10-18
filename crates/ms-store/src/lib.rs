// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The engine's SQLite store.
//!
//! Derived from Mailspring-Sync (`MailSync/MailStore.*`, `MailStoreTransaction.*`,
//! `Query.*`, `DeltaStream.*`, `Placement.*`, `Models/*`, `constants.h`), Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! One [`Store`] per thread, each with its own connection; `Store` is `!Send` so the
//! compiler enforces what `MailStore::assertCorrectThread` checked at runtime. Deltas go to
//! the process-wide [`DeltaStream`], which every store shares.

pub mod counts;
pub mod delta;
pub mod invariants;
pub mod legacy_import;
pub mod migrate;
pub mod model;
pub mod model_query;
pub mod models;
pub mod placement;
pub mod query;
pub mod read;
pub mod schema;
pub mod search;
pub mod store;

pub use counts::THREAD_COUNTS;
pub use delta::{
    is_socket_only, DeltaItem, DeltaSink, DeltaStream, WriteSink, MAIL_NEW, SECRETS_UPDATED,
};
pub use model::Model;
pub use models::{
    Calendar, Contact, ContactBook, ContactGroup, Event, File, FilePart, Folder, GenericModel,
    IcsEventFields, IncomingMessage, Label, Message, MessageSnapshot, Task, Thread, FILES_TMP_DIR,
};
pub use placement::{FetchableCopy, MessageAttributes, Placement};
pub use query::Query;
pub use store::Store;
