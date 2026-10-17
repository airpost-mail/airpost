// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The IMAP session.
//!
//! Derived from mailcore2's `IMAPSession` (`Vendor/mailcore2/src/core/imap/
//! MCIMAPSession.cpp`), Copyright © 2001-2013 MailCore team, BSD-3-Clause, the libetpan
//! IMAP code it drives, Copyright © 2001-2005 DINH Viet Hoa, BSD-3-Clause, and
//! Mailspring-Sync's use of them (`MailUtils::configureSessionForAccount`), Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md ("mailcore2", "libEtPan!").
//!
//! Workers and tasks depend on the [`ImapSession`] trait. Behind it the crate owns the whole
//! protocol stack (transport, lenient parser, commands); the IMAP spike (plan section 10,
//! docs/spikes/imap.md) ruled out async-imap and imap-proto. The trait is synchronous:
//! [`Session`] owns a current-thread tokio runtime and blocks on it, so it must not be
//! called from inside another runtime. Errors are [`MailError`]s; callers convert them with
//! `MailError::into_sync("<call>")`, which names the failing call in the debuginfo.

mod envelope;
mod parser;
mod session;
mod transport;
mod types;

pub use session::Session;
pub use types::*;

/// Parses one server response and drops the result: the entry point of the `imap_response`
/// fuzz target (`fuzz/`), which can't reach the private parser otherwise.
#[doc(hidden)]
pub fn fuzz_parse_response(raw: &[u8]) {
    let _ = parser::parse_response(raw);
}

use ms_core::{MailError, MessageFlags, UidSet};
use ms_providers::ServerProfile;

pub type Result<T> = std::result::Result<T, MailError>;

/// One connection to the account's IMAP server. Selecting is implicit: every call that
/// takes a `path` selects it first if it is not the current folder.
pub trait ImapSession {
    // -- Connection ---------------------------------------------------------------------

    /// Connects (TLS / STARTTLS / clear per the config) and reads the greeting.
    fn connect_if_needed(&mut self) -> Result<()>;
    /// CAPABILITY, LOGIN or AUTHENTICATE XOAUTH2, COMPRESS, ENABLE QRESYNC (else CONDSTORE),
    /// ID where the profile needs it, NAMESPACE (else `LIST "" ""`). Connects first.
    fn login_if_needed(&mut self) -> Result<()>;
    fn is_disconnected(&self) -> bool;
    fn disconnect(&mut self);
    /// Replaces the credentials (a refreshed OAuth token) for the next login.
    fn set_auth(&mut self, auth: ImapAuth);

    // -- What the server is -------------------------------------------------------------

    /// CAPABILITY after login (`storedCapabilities`), with the profile's overrides applied.
    fn capabilities(&self) -> &Capabilities;
    /// The profile resolved at login. The generic profile before login.
    fn profile(&self) -> &ServerProfile;
    /// The personal namespace, or one made from the `LIST "" ""` delimiter.
    fn default_namespace(&self) -> Option<&Namespace>;
    fn is_qresync_enabled(&self) -> bool;
    /// MESSAGELIMIT=<n> (RFC 9738), if advertised.
    fn message_limit(&self) -> Option<u32>;
    /// The currently selected folder.
    fn current_folder(&self) -> Option<&str>;
    /// The TLS failure of the last connect attempt, for `test` mode advice.
    fn last_tls_error(&self) -> Option<&str>;

    // -- Folders ------------------------------------------------------------------------

    /// `LIST "" *` (or XLIST): every folder, INBOX normalised to `INBOX`.
    fn list_folders(&mut self) -> Result<Vec<ListedFolder>>;
    /// A plain SELECT, even if `path` is already selected.
    fn select(&mut self, path: &str) -> Result<SelectInfo>;
    /// Whether the last SELECT of `path` on this connection allowed new keywords
    /// (`SelectInfo::allows_new_keywords`); `None` before one.
    fn allows_new_keywords(&self, _path: &str) -> Option<bool> {
        None
    }
    fn select_if_needed(&mut self, path: &str) -> Result<()>;
    fn noop(&mut self) -> Result<()>;
    /// STATUS (UNSEEN MESSAGES RECENT UIDNEXT UIDVALIDITY [HIGHESTMODSEQ]).
    fn status(&mut self, path: &str) -> Result<FolderStatus>;
    fn create_folder(&mut self, path: &str) -> Result<()>;
    fn rename_folder(&mut self, path: &str, new_path: &str) -> Result<()>;
    fn delete_folder(&mut self, path: &str) -> Result<()>;

    // -- Messages -----------------------------------------------------------------------

    /// UID FETCH of `uids` with the attributes of `kind`. Messages missing a requested
    /// attribute, or with UID 0, are dropped. `hit_message_limit` reports a
    /// `[MESSAGELIMIT n uid]` response code.
    fn fetch_by_uid(&mut self, path: &str, kind: FetchKind, uids: &UidSet) -> Result<FetchResult>;
    /// UID FETCH … (CHANGEDSINCE modseq [VANISHED]).
    fn sync_by_uid(
        &mut self,
        path: &str,
        kind: FetchKind,
        uids: &UidSet,
        modseq: u64,
    ) -> Result<SyncResult>;
    /// Every untagged VANISHED seen for `path` since the last call, from any response.
    fn take_vanished(&mut self, path: &str) -> UidSet;
    /// `UID FETCH <uid> BODY.PEEK[]`. `None` when the message no longer exists.
    fn fetch_message_bytes(&mut self, path: &str, uid: u32) -> Result<Option<Vec<u8>>>;
    fn store_flags(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        flags: MessageFlags,
    ) -> Result<()>;
    /// `+FLAGS.SILENT` / `-FLAGS.SILENT` of keywords, which must be keyword atoms
    /// (`ms_core::keywords::is_keyword_atom`). `StoreOp::Set` is refused: it would replace
    /// the system flags too.
    fn store_keywords(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        keywords: &[String],
    ) -> Result<()>;
    /// X-GM-LABELS. Labels are passed as the client names them; `\Inbox` etc. as-is.
    fn store_labels(
        &mut self,
        path: &str,
        uids: &UidSet,
        op: StoreOp,
        labels: &[String],
    ) -> Result<()>;
    /// UID MOVE. The mapping comes from COPYUID and is empty without UIDPLUS.
    fn move_uids(&mut self, path: &str, uids: &UidSet, dest_path: &str) -> Result<UidMapping>;
    fn copy_uids(&mut self, path: &str, uids: &UidSet, dest_path: &str) -> Result<UidMapping>;
    /// EXPUNGE of every `\Deleted` message in `path`.
    fn expunge(&mut self, path: &str) -> Result<()>;
    /// UID EXPUNGE (UIDPLUS).
    fn expunge_uids(&mut self, path: &str, uids: &UidSet) -> Result<()>;
    /// APPEND; returns the new UID from APPENDUID when the server reports one.
    fn append(&mut self, path: &str, data: &[u8], flags: MessageFlags) -> Result<Option<u32>>;
    /// Re-selects `path` and fetches the headers of its last few messages by sequence
    /// number, returning the UIDs whose Message-ID is `header_message_id`
    /// (`findUIDsOfRecentHeaderMessageID`). Used to find the provider's Sent copy.
    fn find_uids_of_recent_header_message_id(
        &mut self,
        path: &str,
        header_message_id: &str,
    ) -> Result<Vec<u32>>;

    // -- IDLE ---------------------------------------------------------------------------

    /// IDLE is advertised (or forced by the profile).
    fn supports_idle(&self) -> bool;
    /// IDLEs on `path` until the server reports a change, 28 minutes pass, or `interrupt`
    /// fires.
    fn idle(&mut self, path: &str, interrupt: &IdleInterrupt) -> Result<()>;
}
