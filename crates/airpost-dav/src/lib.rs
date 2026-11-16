// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The protocol side of calendar and contact sync; the CalDAV/CardDAV workers that drive
//! it are in airpost-sync (P5).
//!
//! Derived from Mailspring-Sync (`DAVWorker.cpp` protocol code, `DavXML.*`, `DAVUtils.*`,
//! `VCard.*`, `Models/Event.cpp` ICS handling), Copyright © 2017 Foundry 376, GPL-3.0, and
//! iCalendarLib (`Vendor/icalendarlib`), Copyright © Juliusz Gonera, MIT. See NOTICE.md
//! ("iCalendarLib").
//!
//! - [`DavClient`] makes the requests: discovery, listings, sync-collection, multiget,
//!   writes. It is blocking (reqwest) and owned by the calContacts thread; it may be
//!   called with or without a tokio runtime entered on that thread.
//! - [`xml`], [`requests`] and [`href`] are the pieces it is built from, usable alone.
//! - [`ical`] and [`vcard`] read the Event and Contact fields from resources.
//! - [`DavProfile`] names each server quirk the C++ comments describe.

pub mod client;
pub mod discovery;
pub mod href;
pub mod ical;
pub mod profile;
pub mod ratelimit;
pub mod requests;
mod runtime;
pub mod transport;
pub mod vcard;
pub mod xml;

pub use client::{
    http_status, AddressBookInfo, CalendarHome, CalendarInfo, CalendarListing, DavAuth, DavClient,
    DavObject, DeleteOutcome, HrefEtag, MultigetResult, Ownership, ResourceKind, SyncChange,
    SyncCollection, SyncCollectionResult, TokenProvider, WriteResult, ETAG_CONFLICT,
};
pub use discovery::{DavHost, DavService, DnsResolver, HostResolver};
pub use ical::{EventFields, FloatingZone, Zones};
pub use profile::{DavProfile, DavServer};
pub use requests::{CalendarSyncRange, Collection};
pub use transport::{HttpRequest, HttpResponse, ReqwestTransport, Transport};
pub use vcard::{ContactFields, VCard};
pub use xml::{DavResponse, Multistatus};
