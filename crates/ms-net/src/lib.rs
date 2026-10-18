// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Network services other than IMAP and SMTP.
//!
//! Derived from Mailspring-Sync (`NetworkRequestUtils.*`, `XOAuth2TokenManager.*`,
//! `GoogleContactsWorker.*`, `MetadataWorker.*`), Copyright © 2017 Foundry 376, GPL-3.0.
//! See NOTICE.md.
//!
//! Everything here is blocking (reqwest's blocking client), so it runs on the calling
//! worker thread and never inside an IMAP session's runtime. The bin installs the ring
//! rustls provider at startup; [`HttpClient::new`] installs it too if nothing has.

pub mod endpoints;
pub mod http;
pub mod http_metadata;
pub mod metadata;
pub mod oauth;
pub mod outbox;
pub mod people;
pub mod unsubscribe;

pub use endpoints::Endpoints;
pub use http::HttpClient;
pub use http_metadata::HttpMetadataBackend;
pub use metadata::{LocalOnly, MetadataBackend, MetadataDelta, MetadataPush};
pub use ms_wire::{TernServer, TernServerError};
pub use oauth::{AccessToken, RotateCallback, TokenManager};
pub use people::PeopleClient;
