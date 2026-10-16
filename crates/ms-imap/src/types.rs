// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! What an IMAP session takes and returns: configuration, capabilities, folders, fetched
//! messages, sync results and the IDLE interrupt.
//!
//! Shaped after mailcore2's IMAP types (`MCIMAPSession.h`, `MCIMAPFolder`,
//! `MCIMAPFolderStatus`, `MCIMAPNamespace`, `MCIMAPMessage`, `MCIMAPSyncResult`), Copyright ©
//! 2001-2013 MailCore team, BSD-3-Clause, and Mailspring-Sync's
//! `MailUtils::configureSessionForAccount` and `messagesRequestKindFor`, Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md ("mailcore2").

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ms_core::protolog::ProtocolLogger;
use ms_core::{Account, FolderFlags, MessageFlags, MessageHeader, Security, UidSet};

#[derive(Clone)]
pub enum ImapAuth {
    Password(String),
    /// An OAuth access token, already fetched: token refresh does blocking HTTP and must
    /// not run on the session's runtime.
    XOAuth2(String),
}

impl std::fmt::Debug for ImapAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Note: never print credentials.
        f.write_str(match self {
            ImapAuth::Password(_) => "Password(..)",
            ImapAuth::XOAuth2(_) => "XOAuth2(..)",
        })
    }
}

/// `MailUtils::configureSessionForAccount`.
#[derive(Clone)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub auth: ImapAuth,
    /// `allowObsoleteTLS` (`imap_allow_insecure_ssl`). rustls has no TLS below 1.2 and no
    /// legacy ciphers to fall back to, so it changes nothing; kept so the setting is carried.
    pub allow_obsolete_tls: bool,
    /// Never ENABLE QRESYNC, whatever the server advertises.
    pub disable_qresync: bool,
    /// Sent with ID (name, version) to the servers that require it before SELECT.
    pub client_identity: Option<(String, String)>,
    pub timeout: Duration,
    /// `sent`/`recv` transcript with AUTH payloads removed (`--verbose`, `test` mode).
    pub logger: Option<Arc<dyn ProtocolLogger>>,
    /// Replaces the platform-verifier TLS configuration (tests trusting a private CA).
    pub tls_config: Option<Arc<rustls::ClientConfig>>,
}

impl ImapConfig {
    /// mailcore's socket timeout.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn new(
        host: impl Into<String>,
        port: u16,
        security: Security,
        username: impl Into<String>,
        auth: ImapAuth,
    ) -> Self {
        ImapConfig {
            host: host.into(),
            port,
            security,
            username: username.into(),
            auth,
            allow_obsolete_tls: false,
            disable_qresync: false,
            client_identity: None,
            timeout: Self::DEFAULT_TIMEOUT,
            logger: None,
            tls_config: None,
        }
    }

    /// `MailUtils::configureSessionForAccount`. `auth` is the password, or the access token
    /// the caller fetched for an OAuth account (the refresh is blocking HTTP and must happen
    /// outside the session's runtime). iCloud's QRESYNC opt-out comes from the resolved
    /// profile at login, so it needs no flag here.
    pub fn for_account(account: &Account, auth: ImapAuth) -> Self {
        let mut c = ImapConfig::new(
            account.imap_host(),
            account.imap_port(),
            account.imap_security(),
            account.imap_username(),
            auth,
        );
        c.allow_obsolete_tls = account.imap_allow_insecure_ssl();
        c.client_identity = Some(("Tern".to_owned(), env!("CARGO_PKG_VERSION").to_owned()));
        c
    }
}

impl std::fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

/// CAPABILITY atoms, uppercased.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities(BTreeSet<String>);

impl Capabilities {
    pub fn new<S: AsRef<str>>(atoms: impl IntoIterator<Item = S>) -> Self {
        Capabilities(
            atoms
                .into_iter()
                .map(|a| a.as_ref().to_ascii_uppercase())
                .collect(),
        )
    }

    pub fn has(&self, cap: &str) -> bool {
        self.0.contains(&cap.to_ascii_uppercase())
    }

    pub fn insert(&mut self, cap: &str) {
        self.0.insert(cap.to_ascii_uppercase());
    }

    pub fn remove(&mut self, cap: &str) {
        self.0.remove(&cap.to_ascii_uppercase());
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// The value of a `NAME=value` capability, e.g. `MESSAGELIMIT=1000`.
    pub fn value_of(&self, name: &str) -> Option<&str> {
        let prefix = format!("{}=", name.to_ascii_uppercase());
        self.0.iter().find_map(|c| c.strip_prefix(&prefix))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Namespace {
    pub prefix: String,
    pub delimiter: Option<char>,
}

impl Namespace {
    /// The server path of a folder given by components, e.g. `["Mailspring", "Snoozed"]`:
    /// each component modified UTF-7 encoded, joined by the delimiter, after the prefix
    /// with the delimiter appended when it lacks one (mailcore's
    /// `IMAPNamespaceItem::pathForComponents`).
    pub fn path_for_components(&self, components: &[&str]) -> String {
        let delim = self.delimiter.map(String::from).unwrap_or_default();
        let encoded: Vec<String> = components
            .iter()
            .map(|c| ms_core::utf7::encode(c))
            .collect();
        let mut path = self.prefix.clone();
        if !path.is_empty() && !path.ends_with(&delim) {
            path.push_str(&delim);
        }
        path.push_str(&encoded.join(&delim));
        path
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedFolder {
    /// As returned by LIST: modified UTF-7, INBOX normalised.
    pub path: String,
    pub delimiter: Option<char>,
    pub flags: FolderFlags,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectInfo {
    pub uid_validity: u32,
    pub uid_next: u32,
    pub highest_modseq: u64,
    pub exists: u32,
}

/// STATUS results. Missing items are 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderStatus {
    pub message_count: u32,
    pub unseen_count: u32,
    pub recent_count: u32,
    pub uid_next: u32,
    pub uid_validity: u32,
    pub highest_modseq: u64,
}

/// The attribute sets of `MailUtils::messagesRequestKindFor` (plan section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchKind {
    /// FLAGS, plus X-GM-LABELS on Gmail.
    Light,
    /// ENVELOPE, `BODY.PEEK[HEADER.FIELDS (References)]`, INTERNALDATE and FLAGS, plus
    /// X-GM-LABELS, X-GM-THRID and X-GM-MSGID on Gmail.
    Heavy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOp {
    Add,
    Remove,
    Set,
}

/// One message from a UID FETCH.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImapMessage {
    pub uid: u32,
    pub flags: MessageFlags,
    /// Flags that are not system flags (`$Forwarded`, `$Junk`, …).
    pub keywords: Vec<String>,
    pub modseq: u64,
    /// INTERNALDATE, epoch seconds (Heavy only).
    pub internal_date: Option<i64>,
    pub gmail_labels: Vec<String>,
    pub gmail_thread_id: Option<u64>,
    pub gmail_message_id: Option<u64>,
    /// ENVELOPE and References (Heavy only).
    pub header: Option<MessageHeader>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchResult {
    pub messages: Vec<ImapMessage>,
    /// The server applied MESSAGELIMIT and returned only part of the range.
    pub hit_message_limit: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncResult {
    pub modified_or_added: Vec<ImapMessage>,
    /// VANISHED (EARLIER) from this response. Any others go to `take_vanished`.
    pub vanished: UidSet,
    pub hit_message_limit: bool,
}

/// COPYUID pairs, source UID → destination UID, in the server's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UidMapping(pub Vec<(u32, u32)>);

impl UidMapping {
    pub fn get(&self, source_uid: u32) -> Option<u32> {
        self.0
            .iter()
            .find(|(s, _)| *s == source_uid)
            .map(|(_, d)| *d)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Ends an IDLE from another thread (`queue-task`, `need-bodies`, `wake-workers`). Sticky:
/// an interrupt that arrives while the session is not idling ends the next IDLE at once,
/// as libetpan's interrupt pipe did.
#[derive(Clone, Default)]
pub struct IdleInterrupt {
    inner: Arc<IdleInterruptInner>,
}

#[derive(Default)]
struct IdleInterruptInner {
    pending: AtomicBool,
    notify: tokio::sync::Notify,
}

impl IdleInterrupt {
    pub fn new() -> Self {
        IdleInterrupt::default()
    }

    pub fn interrupt(&self) {
        self.inner.pending.store(true, Ordering::SeqCst);
        self.inner.notify.notify_one();
    }

    /// Clears and returns the pending flag.
    pub fn take(&self) -> bool {
        self.inner.pending.swap(false, Ordering::SeqCst)
    }

    /// Resolves once [`IdleInterrupt::interrupt`] has been called (now or earlier), and
    /// clears the flag.
    pub async fn wait(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.take() {
                return;
            }
            notified.await;
        }
    }
}

impl std::fmt::Debug for IdleInterrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdleInterrupt")
            .field("pending", &self.inner.pending.load(Ordering::SeqCst))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::Namespace;

    fn ns(prefix: &str, delimiter: Option<char>) -> Namespace {
        Namespace {
            prefix: prefix.to_owned(),
            delimiter,
        }
    }

    #[test]
    fn path_for_components_encodes_and_completes_the_prefix() {
        let parts = ["Mailspring", "Snoozed"];
        assert_eq!(
            ns("", Some('/')).path_for_components(&parts),
            "Mailspring/Snoozed"
        );
        assert_eq!(
            ns("INBOX.", Some('.')).path_for_components(&parts),
            "INBOX.Mailspring.Snoozed"
        );
        assert_eq!(
            ns("INBOX", Some('.')).path_for_components(&parts),
            "INBOX.Mailspring.Snoozed"
        );
        assert_eq!(
            ns("", None).path_for_components(&parts),
            "MailspringSnoozed"
        );
        assert_eq!(
            ns("", Some('/')).path_for_components(&["Entwürfe", "A&B"]),
            "Entw&APw-rfe/A&-B"
        );
    }
}
