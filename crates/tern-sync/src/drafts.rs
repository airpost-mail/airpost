// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Server-synced drafts (`docs/drafts-sync-design.md`), the part the sync workers share with
//! the tasks: which server copies are drafts and the id they map to.
//!
//! With [`Account::save_drafts_to_server`] on, every copy of a draft that has a Message-ID
//! is keyed by `draft_id(accountId, Message-ID)`, the id a client-saved draft gets. Our own
//! APPENDed revision, copies another device or webmail saved, and the leftovers of an
//! interrupted push all collapse onto the one row the composer edits. Off, a draft copy is an
//! ordinary message with the header-hash id, as in the C++ engine.

use tern_core::{Account, Result};
use tern_imap::ImapMessage;
use tern_store::{Folder, MessageAttributes, Model, Store};

use crate::mail_processor::{attributes_of, CopyLabels};

/// A copy is a draft when it is in the drafts-role folder, or flagged or labelled `\Draft`
/// anywhere but Trash and Spam (a draft deleted elsewhere keeps its flag there).
pub fn is_draft_copy(folder: &Folder, attrs: &MessageAttributes) -> bool {
    match folder.role() {
        "trash" | "spam" => false,
        "drafts" => true,
        _ => attrs.draft,
    }
}

/// The Message-ID of a fetched draft copy when the account syncs drafts; `None` for every
/// other copy, which keeps its header-hash id.
pub fn synced_draft_message_id<'m>(
    account: &Account,
    folder: &Folder,
    m: &'m ImapMessage,
    labels: CopyLabels,
) -> Option<&'m str> {
    if !account.save_drafts_to_server() {
        return None;
    }
    let mid = m.header.as_ref()?.message_id.as_deref()?;
    let mid = tern_core::ids::strip_angle_brackets(mid);
    if mid.is_empty() || !is_draft_copy(folder, &attributes_of(m, labels)) {
        return None;
    }
    Some(mid)
}

/// A server copy of a draft that was sent or deleted here: the foreground deletes it
/// instead of the scan syncing it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TombstonedCopy {
    pub folder_id: String,
    pub uid: u32,
    pub header_message_id: String,
}

/// The fetched copy as a [`TombstonedCopy`] when it is a draft copy whose Message-ID has a
/// tombstone.
pub fn tombstoned_copy(
    store: &Store,
    account: &Account,
    folder: &Folder,
    m: &ImapMessage,
    labels: CopyLabels,
) -> Result<Option<TombstonedCopy>> {
    let Some(mid) = synced_draft_message_id(account, folder, m, labels) else {
        return Ok(None);
    };
    if !store.is_draft_tombstoned(account.id(), mid)? {
        return Ok(None);
    }
    Ok(Some(TombstonedCopy {
        folder_id: folder.id().to_owned(),
        uid: m.uid,
        header_message_id: mid.to_owned(),
    }))
}

/// The id of a fetched draft copy when the account syncs drafts ([`synced_draft_message_id`]).
pub fn synced_draft_id(
    account: &Account,
    folder: &Folder,
    m: &ImapMessage,
    labels: CopyLabels,
) -> Option<String> {
    synced_draft_message_id(account, folder, m, labels)
        .map(|mid| tern_core::ids::draft_id(account.id(), mid))
}

/// Whether a draft of an account that syncs drafts has a server copy the foreground has
/// not reconciled with it yet.
pub fn has_unreconciled_copies(store: &Store, account: &Account) -> Result<bool> {
    if !account.save_drafts_to_server() {
        return Ok(false);
    }
    let Some(home) = store.draft_home_folder(account.id())? else {
        return Ok(false);
    };
    Ok(!store
        .drafts_with_unreconciled_copies(account.id(), home.id())?
        .is_empty())
}
