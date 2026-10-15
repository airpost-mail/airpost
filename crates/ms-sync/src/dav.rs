//! CalDAV and CardDAV sync and writes for one account (`DAVWorker`): discovery, the
//! calendar and address book passes of the calContacts thread, and the event and contact
//! writes of the task remote phases.
//!
//! Derived from Mailspring-Sync `MailSync/DAVWorker.cpp` and `.hpp` (the worker side; the
//! protocol side is ms-dav's `DavClient`), `MailSync/DAVUtils.cpp` and the CardDAV group
//! handling of `MailSync/Models/ContactGroup.cpp`, Copyright © 2017 Foundry 376, GPL-3.0.
//! See NOTICE.md.
//!
//! A worker borrows its thread's store: the calContacts thread's for sync, the foreground's
//! for task remote phases (C++ opened a second `MailStore` for each DAVWorker). Discovery
//! results are cached for the worker's lifetime, as in C++.
//!
//! Differences from the C++:
//! - A new card is PUT to `<address book>/<uid>.vcf` with `If-None-Match: *`; C++ POSTed to
//!   the collection, which RFC 6352 doesn't define (plan section 6 "DAV").
//! - A DELETE answered 404 or 410 removes the contact locally; C++ failed the task.
//! - The etag listing fallback (`runForCalendar`) sends the server's own hrefs in its
//!   multigets rather than their decoded form, as the C++ comment on `normalizeHref` says
//!   requests should.
//! - A calendar colour is compared with the stored one after the same normalisation the
//!   store applies, so a `#RRGGBBAA` colour does not re-save the calendar every pass.
//! - A new event is PUT with `If-None-Match: *`, as new cards are; C++ sent no precondition
//!   and could overwrite a resource it had not synced yet.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use ms_core::constants::{CARDDAV_SYNC_SOURCE, CONTACT_MAX_REFS};
use ms_core::{ids, Account, Error, Result, SyncError};
use ms_dav::discovery::email_domain;
use ms_dav::href::{href_for_new_event, normalize_href, replace_path};
use ms_dav::ical::{self, EventFields};
use ms_dav::requests::{DELETION_CHUNK, MULTIGET_CHUNK};
use ms_dav::{
    http_status, CalendarHome, CalendarSyncRange, Collection, DavAuth, DavClient, DavObject,
    DavService, DeleteOutcome, DnsResolver, HostResolver, ReqwestTransport, ResourceKind,
    SyncCollection, SyncCollectionResult, VCard,
};
use ms_net::{HttpClient, TokenManager};
use ms_store::model::json_str;
use ms_store::IcsEventFields;
use ms_store::{Calendar, Contact, ContactBook, ContactGroup, Event, Model, Query, Store};
use serde_json::json;
use tracing::{info, warn};

/// Retries of a sync-collection whose token the server rejected (`maxRetries`).
const MAX_SYNC_TOKEN_RETRIES: u32 = 1;

/// The ms-store shape of one parsed VEVENT.
pub fn ics_event_fields(e: &EventFields) -> IcsEventFields {
    IcsEventFields {
        uid: e.uid.clone(),
        recurrence_id: e.recurrence_id.clone(),
        status: e.status.clone(),
        recurrence_start: e.recurrence_start,
        recurrence_end: e.recurrence_end,
        summary: e.summary.clone(),
        description: e.description.clone(),
        location: e.location.clone(),
        attendees: e.attendees.clone(),
    }
}

/// A card read from a response, before it touches the store.
struct ParsedContact {
    id: String,
    etag: String,
    href: String,
    vcf: String,
    email: String,
    name: String,
    is_group: bool,
}

/// One VEVENT of a fetched resource.
struct ParsedEvent {
    etag: String,
    href: String,
    ics: String,
    fields: EventFields,
}

/// `ContactGroup` support for servers whose groups are cards (iCloud, Fastmail, vCard 4
/// `KIND:group`): the hidden group contact keeps the card synced by etag, and a ContactGroup
/// with the same id carries the name and the members (`rebuildContactGroup`).
pub fn rebuild_contact_group(store: &Store, account_id: &str, contact: &Contact) -> Result<()> {
    let mut group = store
        .find::<ContactGroup>(&Query::new().equal("id", contact.id()))?
        .unwrap_or_else(|| ContactGroup::create(contact.id(), account_id));
    group.set_name(contact.name());
    group.set_book_id(contact.book_id());
    store.save(&mut group)?;

    let card = VCard::parse(json_str(contact.info(), "vcf"));
    if card.incomplete() {
        return Ok(());
    }
    group.sync_members(store, &card.member_ids())
}

pub struct DavWorker<'a> {
    account: Arc<Account>,
    store: &'a Store,
    client: DavClient,
    resolver: Box<dyn HostResolver>,
    /// Set for Gmail: calendars live at a fixed home and contacts go through the People API.
    gmail_home: Option<CalendarHome>,

    contacts_discovery_complete: bool,
    cached_address_book: Option<ContactBook>,
    contacts_validation_failures: u32,

    calendars_discovery_complete: bool,
    cached_calendar_home: Option<CalendarHome>,
}

impl<'a> DavWorker<'a> {
    /// The worker for `account` on this thread's `store`. Requests authenticate with an OAuth
    /// Bearer token from `tokens` when the account has a refresh token, else with Basic and
    /// the IMAP credentials (`getAuthorizationHeader`).
    pub fn new(
        account: Arc<Account>,
        store: &'a Store,
        tokens: Arc<TokenManager>,
        verbose: bool,
    ) -> Result<Self> {
        let http = HttpClient::new()?;
        let token_account = account.clone();
        let auth = DavAuth::for_account(&account, move || {
            Ok(tokens.token_for(&http, &token_account)?.access_token)
        });
        let mut client = DavClient::new(Box::new(ReqwestTransport::new()?), auth);
        client.set_verbose(verbose);
        Ok(DavWorker::with_client(
            account,
            store,
            client,
            Box::new(DnsResolver),
        ))
    }

    /// A worker on a given client and host resolver (tests).
    pub fn with_client(
        account: Arc<Account>,
        store: &'a Store,
        client: DavClient,
        resolver: Box<dyn HostResolver>,
    ) -> Self {
        let gmail_home =
            (account.provider() == "gmail").then(|| CalendarHome::gmail(account.email_address()));
        DavWorker {
            account,
            store,
            client,
            resolver,
            gmail_home,
            contacts_discovery_complete: false,
            cached_address_book: None,
            contacts_validation_failures: 0,
            calendars_discovery_complete: false,
            cached_calendar_home: None,
        }
    }

    fn aid(&self) -> &str {
        self.account.id()
    }

    /// `DAVWorker::run`: contacts, then calendars.
    pub fn run(&mut self) -> Result<()> {
        self.run_contacts()?;
        self.run_calendars()
    }

    // -- CardDAV ---------------------------------------------------------------------------

    /// `runContacts`. A server without CardDAV, or one unreachable during discovery, is
    /// skipped for the worker's lifetime.
    pub fn run_contacts(&mut self) -> Result<()> {
        if self.gmail_home.is_some() {
            return Ok(());
        }
        if self.contacts_discovery_complete && self.cached_address_book.is_none() {
            return Ok(());
        }
        if !self.contacts_discovery_complete {
            info!("Performing full CardDAV address book discovery");
            match self.resolve_address_book() {
                Ok(book) => {
                    self.cached_address_book = book;
                    self.contacts_discovery_complete = true;
                    self.contacts_validation_failures = 0;
                }
                Err(e) if matches!(http_status(&e), Some(404..=406)) => {
                    info!(
                        "CardDAV not supported by server ({}), skipping contact sync",
                        error_key(&e)
                    );
                    self.contacts_discovery_complete = true;
                    self.cached_address_book = None;
                    return Ok(());
                }
                Err(e) if e.is_offline() => {
                    info!(
                        "CardDAV server unreachable during discovery ({}), skipping contact sync",
                        error_key(&e)
                    );
                    self.contacts_discovery_complete = true;
                    self.cached_address_book = None;
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
            if self.cached_address_book.is_none() {
                return Ok(());
            }
        }

        if !self.validate_cached_address_book()? {
            info!("Cached address book URL invalid, will rediscover on next sync");
            self.cached_address_book = None;
            self.contacts_discovery_complete = false;
            return Ok(());
        }
        let Some(mut book) = self.cached_address_book.take() else {
            return Ok(());
        };
        let result = self.sync_address_book(&mut book);
        self.cached_address_book = Some(book);
        result
    }

    fn sync_address_book(&mut self, book: &mut ContactBook) -> Result<()> {
        let existing = self
            .store
            .find::<ContactBook>(&Query::new().equal("accountId", self.aid()))?;
        let old_ctag = existing.as_ref().map_or("", |b| b.ctag()).to_string();
        let new_ctag = book.ctag().to_string();
        if old_ctag == new_ctag && !new_ctag.is_empty() {
            info!("Address book unchanged (ctag: {new_ctag}), skipping sync");
            return Ok(());
        }
        if new_ctag.is_empty() {
            info!("Syncing address book (server doesn't provide ctag)");
        } else {
            info!("Syncing address book (ctag: {old_ctag} -> {new_ctag})");
        }

        // sync-collection first; the etag listing covers servers without it (Robur, GMX),
        // those refusing it with HTTP errors (Zimbra, Posteo) and unreliable ones (Synology,
        // DAViCal, Bedework).
        if !self.run_for_address_book_with_sync_token(book, 0)? {
            self.run_for_address_book(book)?;
        }
        // Only now: discovery saved the book without its ctag, so a pass that fails before
        // here is retried rather than skipped as unchanged.
        if !new_ctag.is_empty() {
            book.set_ctag(&new_ctag);
            self.store.save(book)?;
        }
        Ok(())
    }

    /// `validateCachedAddressBook`: a Depth 0 ctag PROPFIND. False when the URL is gone (404,
    /// 406, 410) or after three authentication failures in a row; other errors propagate and
    /// keep the cache.
    fn validate_cached_address_book(&mut self) -> Result<bool> {
        let Some(url) = self
            .cached_address_book
            .as_ref()
            .map(|b| b.url().to_string())
        else {
            return Ok(false);
        };
        match self.client.address_book_ctag(&url) {
            Ok(ctag) => {
                if let (Some(ctag), Some(book)) = (ctag, self.cached_address_book.as_mut()) {
                    book.set_ctag(&ctag);
                }
                self.contacts_validation_failures = 0;
                Ok(true)
            }
            Err(e) => {
                self.contacts_validation_failures += 1;
                match http_status(&e) {
                    Some(404 | 406 | 410) => {
                        info!(
                            "Address book URL returned {}, invalidating cache",
                            error_key(&e)
                        );
                        Ok(false)
                    }
                    Some(401 | 403) if self.contacts_validation_failures >= 3 => {
                        warn!(
                            "Multiple auth failures ({}), invalidating discovery cache",
                            self.contacts_validation_failures
                        );
                        Ok(false)
                    }
                    _ => Err(e),
                }
            }
        }
    }

    /// `resolveAddressBook`: the account's ContactBook, its URL refreshed from discovery on
    /// every call since users have no way to change it. `None` when the account syncs
    /// contacts some other way or the server has no CardDAV.
    pub fn resolve_address_book(&mut self) -> Result<Option<ContactBook>> {
        let mut existing = self
            .store
            .find::<ContactBook>(&Query::new().equal("accountId", self.aid()))?;
        if existing
            .as_ref()
            .is_some_and(|b| b.source() != CARDDAV_SYNC_SOURCE)
        {
            return Ok(None);
        }
        let domain = email_domain(self.account.email_address()).to_string();
        let Some(host) =
            self.resolver
                .dav_host(DavService::CardDav, &domain, self.account.imap_host())?
        else {
            return Ok(existing);
        };
        let Some(books) = self.client.discover_address_books(&host)? else {
            return Ok(existing);
        };
        // The C++ kept the last address book listed ("TODO: Pick the primary one").
        for info in books {
            let book = existing.get_or_insert_with(|| {
                ContactBook::create(&format!("{}-default", self.account.id()), self.account.id())
            });
            book.set_source(CARDDAV_SYNC_SOURCE);
            book.set_url(&info.url);
            // Saved without the new ctag: it is stored after a successful sync, and saving
            // it here would make the first sync see it unchanged.
            self.store.save(book)?;
            book.set_ctag(&info.ctag);
        }
        Ok(existing)
    }

    /// RFC 6578 sync-collection for an address book (`runForAddressBookWithSyncToken`).
    /// False when the caller should fall back to the etag listing.
    fn run_for_address_book_with_sync_token(
        &mut self,
        book: &mut ContactBook,
        retry: u32,
    ) -> Result<bool> {
        let token = book.sync_token().to_string();
        let result = match self
            .client
            .sync_collection(book.url(), Collection::AddressBook, &token)
        {
            SyncCollection::Complete(r) => r,
            SyncCollection::Unsupported(_) => return Ok(false),
            SyncCollection::TokenRejected(_) => {
                if token.is_empty() || retry >= MAX_SYNC_TOKEN_RETRIES {
                    if retry >= MAX_SYNC_TOKEN_RETRIES {
                        warn!("Max retries ({MAX_SYNC_TOKEN_RETRIES}) exceeded for address book sync token, falling back to legacy sync");
                    }
                    info!(
                        "sync-collection not supported or failed for contacts, using legacy sync"
                    );
                    return Ok(false);
                }
                info!(
                    "Sync token expired for address book, retrying with full sync (attempt {})",
                    retry + 1
                );
                book.set_sync_token("");
                self.store.save(book)?;
                return self.run_for_address_book_with_sync_token(book, retry + 1);
            }
        };
        self.apply_address_book_changes(book, &result)?;
        Ok(true)
    }

    fn apply_address_book_changes(
        &mut self,
        book: &mut ContactBook,
        result: &SyncCollectionResult,
    ) -> Result<()> {
        let mut groups: Vec<Contact> = Vec::new();

        let direct: Vec<ParsedContact> = result
            .data()
            .iter()
            .filter_map(|o| self.parse_card(o))
            .collect();
        if !direct.is_empty() {
            self.store.transaction("syncToken:contacts:direct", |s| {
                for p in &direct {
                    self.upsert_contact(s, p, book.id(), &mut groups)?;
                }
                Ok(())
            })?;
        }

        let mut needed = result.needed_hrefs();
        needed.reverse();
        let mut empty_multiget = false;
        for chunk in needed.chunks(MULTIGET_CHUNK) {
            let got = self
                .client
                .multiget(book.url(), Collection::AddressBook, chunk)?;
            if got.responses == 0 {
                empty_multiget = true;
            }
            let parsed: Vec<ParsedContact> = got
                .objects
                .iter()
                .filter_map(|o| self.parse_card(o))
                .collect();
            self.store.transaction("syncToken:contacts:multiget", |s| {
                for p in &parsed {
                    self.upsert_contact(s, p, book.id(), &mut groups)?;
                }
                Ok(())
            })?;
        }

        let deleted = result.deleted_hrefs();
        if !deleted.is_empty() {
            let gone: HashSet<String> = deleted.iter().map(|h| normalize_href(h)).collect();
            self.store
                .transaction("syncToken:contacts:deletions", |s| {
                    // The href is in the JSON, so the book's contacts are loaded once and matched.
                    for mut c in s.find_all::<Contact>(&Query::new().equal("bookId", book.id()))? {
                        let href = json_str(c.info(), "href");
                        if href.is_empty() || !gone.contains(&normalize_href(href)) {
                            continue;
                        }
                        if let Some(mut g) =
                            s.find::<ContactGroup>(&Query::new().equal("id", c.id()))?
                        {
                            s.remove(&mut g)?;
                        }
                        s.remove(&mut c)?;
                    }
                    Ok(())
                })?;
        }

        // Groups last, so the member contacts they reference exist.
        self.save_groups(groups)?;

        // An empty multiget is a server anomaly; storing the token would skip those cards
        // in every later incremental sync.
        if !result.sync_token.is_empty() && result.sync_token != book.sync_token() {
            if empty_multiget {
                warn!("Not storing sync-token because multiget returned empty responses - will retry contacts on next sync");
            } else {
                book.set_sync_token(&result.sync_token);
                self.store.save(book)?;
                info!("Stored new sync-token for address book");
            }
        }
        Ok(())
    }

    /// The etag listing (`runForAddressBook`): etags change with content, so cards are only
    /// added (new etags) and deleted (missing etags).
    fn run_for_address_book(&mut self, book: &ContactBook) -> Result<()> {
        let mut remote: BTreeMap<String, String> = BTreeMap::new();
        for he in self.client.address_book_etags(book.url())? {
            remote.insert(he.etag, he.href);
        }
        let local: HashSet<String> = {
            let mut stmt = self
                .store
                .conn()
                .prepare_cached("SELECT etag FROM Contact WHERE bookId = ?")?;
            let rows = stmt
                .query_map([book.id()], |r| r.get::<_, Option<String>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter().map(Option::unwrap_or_default).collect()
        };
        let mut needed: Vec<String> = remote
            .iter()
            .filter(|(etag, _)| !local.contains(*etag))
            .map(|(_, href)| href.clone())
            .collect();
        let mut deleted: Vec<String> = local
            .iter()
            .filter(|etag| !remote.contains_key(*etag))
            .cloned()
            .collect();
        deleted.sort();
        needed.reverse();
        if !deleted.is_empty() || !needed.is_empty() {
            info!("{}", book.url());
            info!("  remote: {} etags", remote.len());
            info!("   local: {} etags", local.len());
            info!(" deleted: {}", deleted.len());
            info!("  needed: {}", needed.len());
        }

        let mut groups: Vec<Contact> = Vec::new();
        for chunk in needed.chunks(MULTIGET_CHUNK) {
            let got = self
                .client
                .multiget(book.url(), Collection::AddressBook, chunk)?;
            let parsed: Vec<ParsedContact> = got
                .objects
                .iter()
                .filter_map(|o| self.parse_card(o))
                .collect();
            // Deletions ride the first chunk's transaction, so a replaced card usually
            // disappears and reappears in one commit.
            let pending = std::mem::take(&mut deleted);
            self.store.transaction("runForAddressBook", |s| {
                self.ingest_contact_deletions(s, book, &pending)?;
                for p in &parsed {
                    self.upsert_contact(s, p, book.id(), &mut groups)?;
                }
                Ok(())
            })?;
        }
        if !deleted.is_empty() {
            self.store.transaction("runForAddressBook:deletions", |s| {
                self.ingest_contact_deletions(s, book, &deleted)
            })?;
        }
        // Saved last and all together, so an interrupted sync leaves whole groups or none.
        self.save_groups(groups)
    }

    /// `ingestContactDeletions`: removes the book's contacts with these etags and the groups
    /// built from them.
    fn ingest_contact_deletions(
        &self,
        s: &Store,
        book: &ContactBook,
        etags: &[String],
    ) -> Result<()> {
        for chunk in etags.chunks(DELETION_CHUNK) {
            let q = Query::new()
                .equal("bookId", book.id())
                .equal_any("etag", chunk.iter());
            let mut group_ids = Vec::new();
            for mut c in s.find_all::<Contact>(&q)? {
                group_ids.push(c.id().to_string());
                s.remove(&mut c)?;
            }
            if group_ids.is_empty() {
                continue;
            }
            for mut g in
                s.find_all::<ContactGroup>(&Query::new().equal_any("id", group_ids.iter()))?
            {
                s.remove(&mut g)?;
            }
        }
        Ok(())
    }

    fn save_groups(&self, groups: Vec<Contact>) -> Result<()> {
        for mut contact in groups {
            rebuild_contact_group(self.store, self.aid(), &contact)?;
            self.store.save(&mut contact)?;
        }
        Ok(())
    }

    /// The fields of one card (`ingestAddressDataNode` before the store). A card without a
    /// UID is keyed by its href.
    fn parse_card(&self, o: &DavObject) -> Option<ParsedContact> {
        if o.data.is_empty() {
            info!("Received addressbook entry {} with an empty body", o.etag);
            return None;
        }
        if VCard::parse(&o.data).incomplete() {
            info!("Unable to decode vcard ({} bytes)", o.data.len());
            return None;
        }
        let fields = ms_dav::ContactFields::from_vcard(&o.data);
        let id = if fields.uid.is_empty() {
            ids::calendar_id(self.aid(), &o.href)
        } else {
            fields.uid
        };
        Some(ParsedContact {
            id,
            etag: o.etag.clone(),
            href: o.href.clone(),
            vcf: o.data.clone(),
            email: fields.email,
            name: fields.name,
            is_group: fields.is_group,
        })
    }

    /// The stored contact for a card, updated from it and not yet saved. A group card's
    /// contact is hidden.
    fn contact_for_card(&self, s: &Store, p: &ParsedContact, book_id: &str) -> Result<Contact> {
        let mut contact = s
            .find::<Contact>(&Query::new().equal("id", &p.id))?
            .unwrap_or_else(|| {
                Contact::create(
                    &p.id,
                    self.aid(),
                    &p.email,
                    CONTACT_MAX_REFS,
                    CARDDAV_SYNC_SOURCE,
                )
            });
        contact.set_info(json!({"vcf": p.vcf, "href": p.href}));
        contact.set_name(&p.name);
        contact.set_email(&p.email);
        contact.set_etag(&p.etag);
        contact.set_book_id(book_id);
        if p.is_group {
            contact.set_hidden(true);
        }
        Ok(contact)
    }

    /// Saves a card's contact, or defers it to `groups` when it is a group card.
    fn upsert_contact(
        &self,
        s: &Store,
        p: &ParsedContact,
        book_id: &str,
        groups: &mut Vec<Contact>,
    ) -> Result<()> {
        let mut contact = self.contact_for_card(s, p, book_id)?;
        if p.is_group {
            groups.push(contact);
        } else {
            s.save(&mut contact)?;
        }
        Ok(())
    }

    /// `writeAndResyncContact`: writes the card (a new one to `<uid>.vcf`, an existing one
    /// with `If-Match`), then reads it back for the server's version and etag. A 403 on an
    /// update is ignored so the read-back reverts the refused card.
    pub fn write_and_resync_contact(&mut self, contact: &mut Contact) -> Result<()> {
        let Some(book) = self
            .store
            .find::<ContactBook>(&Query::new().equal("accountId", self.aid()))?
        else {
            warn!("No address book found.");
            return Ok(());
        };
        let vcf = json_str(contact.info(), "vcf").to_string();
        let mut href = json_str(contact.info(), "href").to_string();

        if href.is_empty() {
            let card = VCard::parse(&vcf);
            let uid = match card.value("UID") {
                "" => contact.id().to_string(),
                uid => uid.to_string(),
            };
            href = ms_dav::client::new_card_href(book.url(), &uid, contact.id());
            self.client.create_vcard(book.url(), &href, &vcf)?;
        } else {
            let url = replace_path(book.url(), &href);
            match self.client.put_vcard(&url, &vcf, contact.etag()) {
                Ok(_) => {}
                Err(e) if http_status(&e) == Some(403) => {}
                Err(e) => return Err(e),
            }
        }

        // The read-back is a new copy of the contact, possibly with server-side changes; the
        // caller's copy is stale from here on.
        let got = self.client.multiget(
            book.url(),
            Collection::AddressBook,
            std::slice::from_ref(&href),
        )?;
        for o in &got.objects {
            let Some(p) = self.parse_card(o) else {
                warn!("Not able to inflate contact from REPORT response after sending.");
                continue;
            };
            let mut serverside = self.contact_for_card(self.store, &p, book.id())?;
            if p.is_group {
                rebuild_contact_group(self.store, self.aid(), &serverside)?;
            }
            info!("Contact ETAG is now {}", serverside.etag());
            self.store.save(&mut serverside)?;
            // A server that gave the card a new UID (Fastmail) made it a different row.
            if contact.id() != serverside.id() {
                self.store.remove(contact)?;
            }
        }
        Ok(())
    }

    /// `deleteContact`: DELETE with `If-Match`, then removes the row. A card already gone
    /// from the server (404, 410) is removed too.
    pub fn delete_contact(&mut self, contact: &mut Contact) -> Result<()> {
        if contact.source() != CARDDAV_SYNC_SOURCE {
            info!("Deleted contact not synced via CardDAV");
            return Ok(());
        }
        let Some(book) = self
            .store
            .find::<ContactBook>(&Query::new().equal("accountId", self.aid()))?
        else {
            warn!("No address book found.");
            return Ok(());
        };
        let Some(href) = contact.info().get("href").and_then(|h| h.as_str()) else {
            warn!("No href in info.");
            return Ok(());
        };
        if href.is_empty() {
            warn!("Blank href in info.");
            return Ok(());
        }
        let url = replace_path(book.url(), href);
        if let DeleteOutcome::AlreadyGone(status) =
            self.client
                .delete(ResourceKind::VCard, &url, contact.etag())?
        {
            info!("{url}: already gone from the server ({status})");
        }
        self.store.remove(contact)
    }

    // -- CalDAV ----------------------------------------------------------------------------

    /// `resolveCalendarHomeURL`: `None` when the account has no CalDAV.
    fn resolve_calendar_home(&mut self) -> Result<Option<CalendarHome>> {
        let domain = email_domain(self.account.email_address()).to_string();
        let Some(host) =
            self.resolver
                .dav_host(DavService::CalDav, &domain, self.account.imap_host())?
        else {
            return Ok(None);
        };
        self.client.discover_calendar_home(&host)
    }

    /// The calendar home, from discovery on first use. `Ok(None)` means skip calendars.
    fn calendar_home(&mut self) -> Result<Option<CalendarHome>> {
        if let Some(home) = &self.gmail_home {
            return Ok(Some(home.clone()));
        }
        if !self.calendars_discovery_complete {
            info!("Performing CalDAV calendar home-set discovery");
            match self.resolve_calendar_home() {
                Ok(home) => {
                    self.cached_calendar_home = home;
                    self.calendars_discovery_complete = true;
                }
                Err(e) if matches!(http_status(&e), Some(404..=406)) => {
                    info!(
                        "CalDAV not supported by server ({}), skipping calendar sync",
                        error_key(&e)
                    );
                    self.calendars_discovery_complete = true;
                    self.cached_calendar_home = None;
                    return Ok(None);
                }
                Err(e) if e.is_offline() => {
                    // Not marked complete: a network failure must not switch CalDAV off for
                    // the process lifetime.
                    info!(
                        "CalDAV server unreachable during discovery ({}), will retry next cycle",
                        error_key(&e)
                    );
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(self.cached_calendar_home.clone())
    }

    /// `resolvedCalendarURL`: the full URL of a calendar path, discovering the home first
    /// when this worker has not (a task's fresh worker).
    fn resolved_calendar_url(&mut self, path: &str) -> Result<String> {
        if let Some(home) = &self.gmail_home {
            return Ok(home.calendar_url(path));
        }
        if path.contains("://") {
            return Ok(path.to_string());
        }
        if !self.calendars_discovery_complete {
            self.cached_calendar_home = self.resolve_calendar_home()?;
            self.calendars_discovery_complete = true;
        }
        match &self.cached_calendar_home {
            Some(home) => Ok(replace_path(&home.url, path)),
            None => Err(
                SyncError::fatal("no-caldav", "CalDAV is not available for this account").into(),
            ),
        }
    }

    /// `runCalendars`: lists the home's VEVENT calendars, saves their metadata, syncs each
    /// whose ctag changed, and drops calendars the server no longer lists.
    pub fn run_calendars(&mut self) -> Result<()> {
        let Some(home) = self.calendar_home()? else {
            return Ok(());
        };
        let listing = match self.client.list_calendars(&home) {
            Ok(l) => l,
            Err(e) if matches!(http_status(&e), Some(404..=406)) => {
                info!(
                    "CalDAV not supported by server ({}), skipping calendar sync",
                    error_key(&e)
                );
                return Ok(());
            }
            Err(e) if e.is_offline() => {
                info!(
                    "CalDAV server unreachable during discovery ({}), skipping calendar sync",
                    error_key(&e)
                );
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        let mut local: HashMap<String, Calendar> = self
            .store
            .find_all_map::<Calendar>(&Query::new().equal("accountId", self.aid()), "id")?;
        let held_before: Vec<String> = local.keys().cloned().collect();
        let listed: HashSet<String> = listing
            .listed_paths
            .iter()
            .map(|p| ids::calendar_id(self.aid(), p))
            .collect();

        for info in &listing.calendars {
            let id = ids::calendar_id(self.aid(), &info.path);
            let ownership = info.ownership.as_str();
            let (mut calendar, needs_sync) = match local.remove(&id) {
                Some(mut calendar) => {
                    let unchanged = calendar.ctag() == info.ctag && !info.ctag.is_empty();
                    if unchanged {
                        info!(
                            "Calendar '{}' unchanged (ctag: {}), skipping sync",
                            info.name, info.ctag
                        );
                    }
                    if self.update_calendar_metadata(&mut calendar, info) {
                        self.store.save(&mut calendar)?;
                    }
                    (calendar, !unchanged)
                }
                None => {
                    let mut calendar = Calendar::new(&id, self.aid());
                    calendar.set_path(&info.path);
                    calendar.set_name(&info.name);
                    if !info.color.is_empty() {
                        calendar.set_color(&info.color);
                    }
                    calendar.set_description(&info.description);
                    calendar.set_read_only(info.read_only);
                    calendar.set_ownership(ownership);
                    let writability = if info.read_only {
                        "read-only"
                    } else if info.privileges_advertised {
                        "writable"
                    } else {
                        "writable, server advertises no privilege set"
                    };
                    info!("Discovered calendar '{}' ({writability})", info.name);
                    if let Some(order) = info.order {
                        calendar.set_order(order);
                    }
                    self.store.save(&mut calendar)?;
                    (calendar, true)
                }
            };

            if needs_sync {
                if info.ctag.is_empty() {
                    info!(
                        "Syncing calendar '{}' (server doesn't provide ctag)",
                        info.name
                    );
                } else {
                    info!(
                        "Syncing calendar '{}' (ctag: {} -> {})",
                        info.name,
                        calendar.ctag(),
                        info.ctag
                    );
                }
                let url = home.calendar_url(&info.path);
                if !self.run_for_calendar_with_sync_token(&id, &url, &mut calendar, 0)? {
                    self.run_for_calendar(&id, &url)?;
                }
                if !info.ctag.is_empty() && calendar.ctag() != info.ctag {
                    calendar.set_ctag(&info.ctag);
                    self.store.save(&mut calendar)?;
                }
            }
        }

        self.prune_calendars(&held_before, &listed)
    }

    /// Applies a listing's metadata to a known calendar; true when anything changed.
    fn update_calendar_metadata(
        &self,
        calendar: &mut Calendar,
        info: &ms_dav::CalendarInfo,
    ) -> bool {
        let mut changed = false;
        if calendar.name() != info.name {
            calendar.set_name(&info.name);
            changed = true;
        }
        if !info.color.is_empty() {
            let mut probe = Calendar::new("", "");
            probe.set_color(&info.color);
            if calendar.color() != probe.color() {
                calendar.set_color(&info.color);
                changed = true;
            }
        }
        if calendar.description() != info.description {
            calendar.set_description(&info.description);
            changed = true;
        }
        if calendar.read_only() != info.read_only {
            calendar.set_read_only(info.read_only);
            changed = true;
            info!(
                "Calendar '{}' is now {}",
                info.name,
                if info.read_only {
                    "read-only"
                } else {
                    "writable"
                }
            );
        }
        // Only a known answer replaces a known answer: a server that stops returning
        // DAV:owner shouldn't erase what it said before.
        let ownership = info.ownership.as_str();
        if !ownership.is_empty() && calendar.ownership() != ownership {
            calendar.set_ownership(ownership);
            changed = true;
        }
        if let Some(order) = info.order {
            if calendar.order() != order {
                calendar.set_order(order);
                changed = true;
            }
        }
        changed
    }

    /// Drops calendars the server stopped listing, with their events, but only when the
    /// listing names at least one calendar held before the pass: one that matches none is
    /// far more likely a change in how the server spells hrefs (encoding, trailing slash,
    /// absolute URLs), which changes every id at once, than the user removing every
    /// calendar, and acting on it would erase events not yet written to the server.
    fn prune_calendars(&self, held_before: &[String], listed: &HashSet<String>) -> Result<()> {
        if !held_before.iter().any(|id| listed.contains(id)) {
            return Ok(());
        }
        for id in held_before.iter().filter(|id| !listed.contains(*id)) {
            let Some(mut calendar) = self.store.find::<Calendar>(&Query::new().equal("id", id))?
            else {
                continue;
            };
            let events = self
                .store
                .find_all::<Event>(&Query::new().equal("calendarId", id))?;
            let count = events.len();
            self.store.transaction("pruneCalendar", |s| {
                for mut e in events {
                    s.remove(&mut e)?;
                }
                s.remove(&mut calendar)
            })?;
            info!(
                "Removed calendar '{}' and its {count} events; the server no longer lists it",
                calendar.name()
            );
        }
        Ok(())
    }

    /// The VEVENTs of fetched resources that have a start time.
    fn parse_events(objects: &[DavObject], log_missing_start: bool) -> Vec<ParsedEvent> {
        let mut out = Vec::new();
        for o in objects {
            if o.data.is_empty() || o.etag.is_empty() {
                continue;
            }
            for fields in ical::parse_events(&o.data) {
                if fields.start.is_none() {
                    if log_missing_start {
                        info!(
                            "Received calendar event with no start time: {} ({} bytes)",
                            o.href,
                            o.data.len()
                        );
                    }
                    continue;
                }
                out.push(ParsedEvent {
                    etag: o.etag.clone(),
                    href: o.href.clone(),
                    ics: o.data.clone(),
                    fields,
                });
            }
        }
        out
    }

    /// RFC 6578 sync-collection for a calendar (`runForCalendarWithSyncToken`). False when
    /// the caller should fall back to the etag listing. Events new to this path are only
    /// created inside the sync window: servers return events outside it (Bedework, SOGo).
    fn run_for_calendar_with_sync_token(
        &mut self,
        calendar_id: &str,
        url: &str,
        calendar: &mut Calendar,
        retry: u32,
    ) -> Result<bool> {
        let token = calendar.sync_token().to_string();
        let result = match self
            .client
            .sync_collection(url, Collection::Calendar, &token)
        {
            SyncCollection::Complete(r) => r,
            SyncCollection::Unsupported(_) => return Ok(false),
            SyncCollection::TokenRejected(_) => {
                if token.is_empty() || retry >= MAX_SYNC_TOKEN_RETRIES {
                    if retry >= MAX_SYNC_TOKEN_RETRIES {
                        warn!("Max retries ({MAX_SYNC_TOKEN_RETRIES}) exceeded for calendar sync token, falling back to legacy sync");
                    }
                    info!("sync-collection not supported or failed, using legacy sync");
                    return Ok(false);
                }
                info!(
                    "Sync token expired for calendar, retrying with full sync (attempt {})",
                    retry + 1
                );
                calendar.set_sync_token("");
                self.store.save(calendar)?;
                return self.run_for_calendar_with_sync_token(
                    calendar_id,
                    url,
                    calendar,
                    retry + 1,
                );
            }
        };
        // Yahoo never returns a sync-token, so an empty answer without one says nothing.
        if result.is_empty_without_token() {
            info!("sync-collection reported 0 change and no token, falling back to full sync");
            return Ok(false);
        }

        let mut existing: HashMap<(String, String), Event> = HashMap::new();
        for e in self
            .store
            .find_all::<Event>(&Query::new().equal("calendarId", calendar_id))?
        {
            existing.insert((e.ics_uid().to_string(), e.recurrence_id().to_string()), e);
        }
        let range = CalendarSyncRange::around(self.store.now());

        let direct = Self::parse_events(&result.data(), false);
        if !direct.is_empty() {
            self.store.transaction("syncToken:direct", |s| {
                self.upsert_events_in_range(s, calendar_id, &direct, &range, &mut existing)
            })?;
        }

        let mut needed = result.needed_hrefs();
        needed.reverse();
        for chunk in needed.chunks(MULTIGET_CHUNK) {
            let got = self.client.multiget(url, Collection::Calendar, chunk)?;
            let parsed = Self::parse_events(&got.objects, false);
            self.store.transaction("syncToken:multiget", |s| {
                self.upsert_events_in_range(s, calendar_id, &parsed, &range, &mut existing)
            })?;
        }

        let deleted = result.deleted_hrefs();
        if !deleted.is_empty() {
            let gone: HashSet<String> = deleted.iter().map(|h| normalize_href(h)).collect();
            let mut doomed: Vec<Event> = Vec::new();
            let mut deleted_masters: HashSet<String> = HashSet::new();
            for e in existing.values() {
                if gone.contains(&normalize_href(e.href())) {
                    if e.recurrence_id().is_empty() {
                        deleted_masters.insert(e.ics_uid().to_string());
                    }
                    doomed.push(e.clone());
                }
            }
            // A deleted master takes its exceptions with it.
            for e in existing.values() {
                if !gone.contains(&normalize_href(e.href()))
                    && !e.recurrence_id().is_empty()
                    && deleted_masters.contains(e.ics_uid())
                {
                    doomed.push(e.clone());
                }
            }
            self.store.transaction("syncToken:deletions", |s| {
                for mut e in doomed {
                    s.remove(&mut e)?;
                }
                Ok(())
            })?;
        }

        if !result.sync_token.is_empty() && result.sync_token != calendar.sync_token() {
            calendar.set_sync_token(&result.sync_token);
            self.store.save(calendar)?;
            info!("Stored new sync-token for calendar");
        }
        Ok(true)
    }

    fn upsert_events_in_range(
        &self,
        s: &Store,
        calendar_id: &str,
        parsed: &[ParsedEvent],
        range: &CalendarSyncRange,
        existing: &mut HashMap<(String, String), Event>,
    ) -> Result<()> {
        for pe in parsed {
            let key = (pe.fields.uid.clone(), pe.fields.recurrence_id.clone());
            let fields = ics_event_fields(&pe.fields);
            if let Some(e) = existing.get_mut(&key) {
                e.apply_ics_event_data(&pe.etag, &pe.href, &pe.ics, &fields);
                s.save(e)?;
            } else if range.overlaps(fields.recurrence_start, fields.recurrence_end) {
                let mut e = Event::create(&pe.etag, self.aid(), calendar_id, &pe.ics, &fields);
                e.set_href(&pe.href);
                s.save(&mut e)?;
                existing.insert(key, e);
            }
        }
        Ok(())
    }

    /// The etag listing for a calendar (`runForCalendar`): the server's events in the sync
    /// window against the stored ones by href. Stored events the server no longer lists in
    /// the window (deleted, or now outside it) are removed with every row of their UID.
    fn run_for_calendar(&mut self, calendar_id: &str, url: &str) -> Result<()> {
        let range = CalendarSyncRange::around(self.store.now());
        // normalised href -> (the server's href, etag)
        let mut remote: BTreeMap<String, (String, String)> = BTreeMap::new();
        for he in self.client.calendar_etags(url, &range)? {
            remote.insert(normalize_href(&he.href), (he.href, he.etag));
        }
        // normalised href -> (icsuid, etag)
        let mut local: BTreeMap<String, (String, String)> = BTreeMap::new();
        for e in self
            .store
            .find_all::<Event>(&Query::new().equal("calendarId", calendar_id))?
        {
            if !e.href().is_empty() {
                local.insert(
                    normalize_href(e.href()),
                    (e.ics_uid().to_string(), e.etag().to_string()),
                );
            }
        }

        let mut needed: Vec<String> = remote
            .iter()
            .filter(|(href, (_, etag))| local.get(*href).is_none_or(|(_, l)| l != etag))
            .map(|(_, (original, _))| original.clone())
            .collect();
        let removed: Vec<String> = local
            .iter()
            .filter(|(href, _)| !remote.contains_key(*href))
            .map(|(_, (uid, _))| uid.clone())
            .collect();
        needed.reverse();
        if !removed.is_empty() || !needed.is_empty() {
            info!(
                "{url} (time range: {} to {})",
                range.start_str, range.end_str
            );
            info!("  remote: {} events in range", remote.len());
            info!("   local: {} events", local.len());
            info!(" removed: {} (deleted or out of range)", removed.len());
            info!("  needed: {}", needed.len());
        }

        for chunk in removed.chunks(DELETION_CHUNK) {
            self.store.transaction("runForCalendar:deletions", |s| {
                let q = Query::new()
                    .equal("calendarId", calendar_id)
                    .equal_any("icsuid", chunk.iter());
                for mut e in s.find_all::<Event>(&q)? {
                    s.remove(&mut e)?;
                }
                Ok(())
            })?;
        }

        for chunk in needed.chunks(MULTIGET_CHUNK) {
            let got = self.client.multiget(url, Collection::Calendar, chunk)?;
            let parsed = Self::parse_events(&got.objects, true);
            self.store.transaction("runForCalendar:insertions", |s| {
                for pe in &parsed {
                    let fields = ics_event_fields(&pe.fields);
                    let q = Query::new()
                        .equal("calendarId", calendar_id)
                        .equal("icsuid", fields.uid.as_str())
                        .equal("recurrenceId", fields.recurrence_id.as_str());
                    match s.find::<Event>(&q)? {
                        Some(mut e) => {
                            e.apply_ics_event_data(&pe.etag, &pe.href, &pe.ics, &fields);
                            s.save(&mut e)?;
                        }
                        None => {
                            let mut e =
                                Event::create(&pe.etag, self.aid(), calendar_id, &pe.ics, &fields);
                            e.set_href(&pe.href);
                            s.save(&mut e)?;
                        }
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn calendar_of(&self, event: &Event, action: &str) -> Result<Calendar> {
        self.store
            .find::<Calendar>(&Query::new().equal("id", event.calendar_id()))?
            .ok_or_else(|| {
                SyncError::fatal(
                    "no-calendar",
                    format!("Calendar not found for event {action}"),
                )
                .into()
            })
    }

    /// `writeAndResyncEvent`: PUTs the event's resource (with `If-Match` when it has an etag,
    /// `If-None-Match: *` when it was never written; a 412 is `etag-conflict`), then reads it
    /// back for the server's version and etag.
    pub fn write_and_resync_event(&mut self, event: &mut Event) -> Result<()> {
        let calendar = self.calendar_of(event, "syncback")?;
        let calendar_url = self.resolved_calendar_url(calendar.path())?;
        let href = match event.href() {
            "" => href_for_new_event(calendar.path(), event.ics_uid(), event.id()),
            h => h.to_string(),
        };
        let url = replace_path(&calendar_url, &href);
        if event.href().is_empty() && event.etag().is_empty() {
            self.client.create_ics(&url, event.ics_data())?;
        } else {
            self.client.put_ics(&url, event.ics_data(), event.etag())?;
        }

        let got = self.client.multiget(
            &calendar_url,
            Collection::Calendar,
            std::slice::from_ref(&href),
        )?;
        if got.objects.is_empty() {
            warn!("Event syncback: received empty response from server");
        }
        if self.apply_read_back(event, &got.objects)? {
            info!("Event syncback complete. New etag: {}", event.etag());
        }
        Ok(())
    }

    /// Re-reads an event's stored resource after a refused write, so the row shows the
    /// server's copy again: the server's etag and ctag did not change, so no sync would.
    /// True when the server returned it.
    pub fn restore_event_from_server(&mut self, event: &mut Event) -> Result<bool> {
        if event.href().is_empty() {
            return Ok(false);
        }
        let calendar = self.calendar_of(event, "restore")?;
        let calendar_url = self.resolved_calendar_url(calendar.path())?;
        let href = event.href().to_string();
        let got = self.client.multiget(
            &calendar_url,
            Collection::Calendar,
            std::slice::from_ref(&href),
        )?;
        self.apply_read_back(event, &got.objects)
    }

    /// Applies the VEVENT matching `event` from a multiget of its resource and saves it.
    fn apply_read_back(&self, event: &mut Event, objects: &[DavObject]) -> Result<bool> {
        let mut applied = false;
        for o in objects {
            let events = ical::parse_events(&o.data);
            // The resource may hold the master and its exceptions.
            let Some(matching) = ical::matching_event(&events, event.recurrence_id()) else {
                continue;
            };
            let fields = ics_event_fields(matching);
            event.apply_ics_event_data(&o.etag, &o.href, &o.data, &fields);
            event.data["icsuid"] = json!(matching.uid);
            self.store.save(event)?;
            applied = true;
        }
        Ok(applied)
    }

    /// `deleteEvent`: DELETE with `If-Match`, then removes the row. A single exception of a
    /// series is refused: the series shares its resource, and one occurrence is removed by
    /// a PUT adding an EXDATE. A resource already gone is returned as
    /// [`DeleteOutcome::AlreadyGone`] with the row kept; the caller decides.
    pub fn delete_event(&mut self, event: &mut Event) -> Result<DeleteOutcome> {
        let calendar = self.calendar_of(event, "deletion")?;
        let href = match event.href() {
            "" => href_for_new_event(calendar.path(), event.ics_uid(), event.id()),
            h => h.to_string(),
        };
        if !event.recurrence_id().is_empty() {
            let vevents =
                ical::count_with_uid(&ical::parse_events(event.ics_data()), event.ics_uid());
            // Sibling rows too, in case ingestion skipped a VEVENT without DTSTART.
            let siblings = self.store.count::<Event>(
                &Query::new()
                    .equal("calendarId", event.calendar_id())
                    .equal("icsuid", event.ics_uid()),
            )?;
            if vevents > 1 || siblings > 1 {
                return Err(SyncError::fatal(
                    "shared-resource",
                    "Cannot delete a single occurrence by removing its calendar resource; the rest of the series shares it",
                )
                .into());
            }
        }
        let calendar_url = self.resolved_calendar_url(calendar.path())?;
        let url = replace_path(&calendar_url, &href);
        let outcome = self
            .client
            .delete(ResourceKind::ICalendar, &url, event.etag())?;
        if outcome == DeleteOutcome::Deleted {
            // Only once the server has accepted.
            self.store.remove(event)?;
            info!("Event deleted successfully: {href}");
        }
        Ok(outcome)
    }
}

fn error_key(e: &Error) -> String {
    match e.as_sync() {
        Some(s) => s.key.clone(),
        None => e.to_string(),
    }
}
