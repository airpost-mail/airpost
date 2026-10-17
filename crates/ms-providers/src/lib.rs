// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Everything that depends on which server or provider is on the other end.
//!
//! Derived from Mailspring-Sync (the host, greeting and capability checks scattered through
//! `SyncWorker.cpp`, `MailUtils.cpp`, `TaskProcessor.cpp`, `MCIMAPSession.cpp`, and the
//! `COMMON_FOLDER_NAMES` table in `constants.h`), Copyright © 2017 Foundry 376, GPL-3.0.
//! `MCIMAPSession.cpp` and `MCSMTPSession.cpp` are mailcore2's, Copyright © 2001-2013
//! MailCore team, BSD-3-Clause. See NOTICE.md ("mailcore2").
//!
//! Two layers (docs/engine-port-plan.md section 4.4): capabilities drive the generic code
//! paths, and a [`ServerProfile`], resolved once per connection, names each quirk as a
//! field. Code outside this crate never checks a host name or greeting; it reads a profile
//! flag. Each provider is one file under `providers/` with a `detect` function and its
//! profile, matched in order with the generic default last.

pub mod discovery;
pub mod oauth;
pub mod providers;
pub mod roles;

pub use oauth::OAuthProvider;
pub use providers::office365::smtp_uses_outlook_xoauth2;
pub use providers::resolve;

/// What is known about a server once it has greeted and answered CAPABILITY (and ID, where
/// the account sends it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionInfo {
    /// The configured IMAP host, lowercased.
    pub host: String,
    /// The untagged greeting line, without `* OK`.
    pub greeting: String,
    /// The ID response's key/value pairs, when ID was sent.
    pub id_response: Vec<(String, String)>,
    /// CAPABILITY atoms, uppercased.
    pub capabilities: Vec<String>,
}

impl ConnectionInfo {
    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities
            .iter()
            .any(|c| c.eq_ignore_ascii_case(cap))
    }
}

/// The detected provider. Only for logging and the profile table; behaviour keys on the
/// [`ServerProfile`] flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Provider {
    #[default]
    Generic,
    Gmail,
    Office365,
    ICloud,
    Yahoo,
    ProtonBridge,
    Dovecot,
    Cyrus,
    Courier,
    Coremail,
    HMailServer,
    HomePl,
    Yandex,
    Hermes,
    DavMail,
    Zoho,
}

/// How folders map to the data model (`MailboxModel` in ms-sync).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MailboxModelKind {
    /// One placement per folder copy.
    #[default]
    Folders,
    /// Gmail: labels, and exactly one placement in All Mail, Spam or Trash.
    GmailLabels,
}

/// Whether labels may live in IMAP keywords on this server (docs/labels-plan.md 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeywordLabels {
    /// When the server lets clients create keywords (`\*` in PERMANENTFLAGS or FLAGS).
    #[default]
    Auto,
    /// Never, whatever the server announces.
    Off,
}

/// Named quirk flags (plan 4.4). Defaults are the generic-server behaviour, which is the
/// C++ behaviour for every server: where the C++ applies a workaround unconditionally, the
/// flag defaults to on and a provider turns it off only when it must.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerProfile {
    pub provider: Provider,
    pub mailbox_model: MailboxModelKind,
    pub keyword_labels: KeywordLabels,
    /// The OAuth token endpoint family, when the account authenticates with OAuth.
    pub oauth: Option<OAuthProvider>,

    // -- Capability overrides
    /// Ignore XLIST (Gmail, Yandex, IdeaImapServer).
    pub xlist_disabled: bool,
    /// Never ENABLE QRESYNC (iCloud).
    pub disable_qresync: bool,
    /// Assume IDLE even if not advertised (Courier).
    pub force_idle: bool,
    /// Assume NAMESPACE even if not advertised (Courier).
    pub force_namespace: bool,
    /// Ignore an advertised NAMESPACE (Hermes).
    pub ignore_namespace_capability: bool,

    // -- Commands
    /// SEARCH with LITERAL+ (Gmail).
    pub literal_plus_search: bool,
    /// SEARCH without a CHARSET argument (Yahoo).
    pub search_charset_null: bool,
    /// Send ID right after login and before any SELECT (Coremail / NetEase).
    pub id_before_select: bool,
    /// The server tags with a `C` prefix (Coremail, detected from the greeting).
    pub c_prefixed_tags: bool,
    /// Honour XYMHIGHESTMODSEQ (Yahoo).
    pub xymhighestmodseq: bool,
    /// NOOP before STATUS of the selected folder, or STATUS returns stale values. Seen on
    /// Dovecot; the C++ does it for every server, so it is on by default.
    pub noop_before_selected_status: bool,
    /// Re-SELECT the folder after APPEND before searching it. Needed on Courier; the C++
    /// send path does it for every server (`TaskProcessor.cpp:2438`), so it is on by default.
    pub reselect_after_append: bool,

    // -- Responses that cannot be taken at face value
    /// STATUS UIDNEXT may be missing, 0 or stale (Coremail, hMailServer, home.pl). A
    /// UIDNEXT of 0 is treated this way on every server.
    pub status_uidnext_unreliable: bool,
    /// VANISHED ranges can extend past UIDNEXT; clamp them (cyrus-imapd #6071). On by
    /// default, as the C++ clamps on every server.
    pub vanished_upper_bound_uidnext: bool,
    /// Take a multi-message COPYUID as a source → destination pairing. Yahoo permutes the
    /// pairs, and the C++ re-verifies new UIDs by header on every server
    /// (`TaskProcessor.cpp:128-216`; a single-message COPYUID is used as is), so it is off
    /// by default.
    pub trust_copyuid_pairing: bool,
    /// The server says BYE when UIDVALIDITY changes on a selected folder (Dovecot).
    pub bye_on_uidvalidity_change: bool,
    /// A NO to LIST is a connection problem, not a fatal error (O365; the C++ applies it
    /// to every server).
    pub list_no_is_connection_error: bool,
    /// Abrupt disconnects during IDLE are normal. Seen on Yandex; the C++ swallows IDLE
    /// errors on every server (`SyncWorker.cpp:302-313`), so it is on by default.
    pub tolerate_idle_disconnect: bool,

    // -- Folders
    /// Skip the \All folder, which duplicates every message (ProtonMail Bridge). Every
    /// server but Gmail, where All Mail is the one folder that is synced.
    pub skip_all_mail_folder: bool,
    /// LIST can return the same folder twice (ProtonMail Bridge, DavMail, Zoho). Merging is
    /// harmless elsewhere, so it is on by default.
    pub merge_duplicate_list_entries: bool,

    // -- Sent mail
    /// The provider files its own Sent copy, sometimes minutes later (O365). The cleanup
    /// runs for every server in the C++, so it is on by default.
    pub late_sent_copies: bool,

    // -- Contacts and calendars
    /// Contacts come from the Google People API instead of CardDAV.
    pub people_api_contacts: bool,
    /// A fixed CalDAV URL, `{email}` substituted (Gmail).
    pub caldav_url_template: Option<&'static str>,
}

impl Default for ServerProfile {
    fn default() -> Self {
        ServerProfile {
            provider: Provider::Generic,
            mailbox_model: MailboxModelKind::Folders,
            keyword_labels: KeywordLabels::Auto,
            oauth: None,
            xlist_disabled: false,
            disable_qresync: false,
            force_idle: false,
            force_namespace: false,
            ignore_namespace_capability: false,
            literal_plus_search: false,
            search_charset_null: false,
            id_before_select: false,
            c_prefixed_tags: false,
            xymhighestmodseq: false,
            noop_before_selected_status: true,
            reselect_after_append: true,
            status_uidnext_unreliable: false,
            vanished_upper_bound_uidnext: true,
            trust_copyuid_pairing: false,
            bye_on_uidvalidity_change: false,
            list_no_is_connection_error: true,
            tolerate_idle_disconnect: true,
            skip_all_mail_folder: true,
            merge_duplicate_list_entries: true,
            late_sent_copies: true,
            people_api_contacts: false,
            caldav_url_template: None,
        }
    }
}

impl ServerProfile {
    /// The fixed CalDAV URL for `email`, when the provider has one.
    pub fn caldav_url(&self, email: &str) -> Option<String> {
        self.caldav_url_template
            .map(|t| t.replace("{email}", email))
    }
}
