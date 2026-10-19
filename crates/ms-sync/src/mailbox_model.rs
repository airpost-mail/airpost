// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Folders versus Gmail labels (plan 4.3 and 4.4). Gmail is the one provider that changes
//! the data model: labels instead of folders, exactly one placement (All Mail, Spam or
//! Trash), and an All Mail placement on send.
//!
//! Derived from Mailspring-Sync (`SyncWorker::syncFoldersAndLabels`, the Gmail branch of
//! `MailProcessor::updateMessage`, the sent-folder lookup of
//! `TaskProcessor::performRemoteSendDraft`), Copyright © 2017 Foundry 376, GPL-3.0. See
//! NOTICE.md.

use ms_core::{FolderFlags, Result, SyncError};
use ms_imap::ListedFolder;
use ms_providers::MailboxModelKind;
use ms_store::{Folder, Label, Message, Model, Query, Store};

pub trait MailboxModel {
    /// Splits LIST output into the folders that hold placements and the labels. Both come
    /// back new (version 0) with id and path set; the caller matches them to stored rows by
    /// path, since a renamed row keeps the id of its first path.
    fn classify_folders(&self, listed: &[ListedFolder]) -> (Vec<Folder>, Vec<Label>);
    /// Called after a placement is inserted or updated, e.g. to apply X-GM-LABELS.
    fn on_placement_upserted(
        &self,
        store: &Store,
        msg: &mut Message,
        folder: &Folder,
    ) -> Result<()>;
    /// Where a sent message is placed: Sent, or All Mail on Gmail.
    fn sent_placement_folder(&self, store: &Store) -> Result<Folder>;
}

/// The model a session's profile calls for.
pub fn for_profile(kind: MailboxModelKind, account_id: &str) -> Box<dyn MailboxModel> {
    match kind {
        MailboxModelKind::Folders => Box::new(FolderModel {
            account_id: account_id.to_owned(),
        }),
        MailboxModelKind::GmailLabels => Box::new(GmailLabelModel {
            account_id: account_id.to_owned(),
        }),
    }
}

fn new_folder(account_id: &str, l: &ListedFolder) -> Folder {
    let mut f = Folder::new(&ms_core::ids::folder_id(account_id, &l.path), account_id, 0);
    f.set_path(&l.path);
    f
}

fn new_label(account_id: &str, l: &ListedFolder) -> Label {
    let mut f = Label::new(&ms_core::ids::folder_id(account_id, &l.path), account_id, 0);
    f.set_path(&l.path);
    f
}

fn find_by_role(store: &Store, account_id: &str, role: &str) -> Result<Option<Folder>> {
    let q = Query::new()
        .equal("accountId", account_id)
        .equal("role", role);
    if let Some(f) = store.find::<Folder>(&q)? {
        return Ok(Some(f));
    }
    Ok(store.find::<Label>(&q)?.map(Folder::from))
}

/// IMAP folders (every non-Gmail server).
pub struct FolderModel {
    pub account_id: String,
}

impl MailboxModel for FolderModel {
    fn classify_folders(&self, listed: &[ListedFolder]) -> (Vec<Folder>, Vec<Label>) {
        let folders = listed
            .iter()
            .map(|l| new_folder(&self.account_id, l))
            .collect();
        (folders, Vec::new())
    }

    fn on_placement_upserted(&self, _: &Store, _: &mut Message, _: &Folder) -> Result<()> {
        Ok(())
    }

    fn sent_placement_folder(&self, store: &Store) -> Result<Folder> {
        find_by_role(store, &self.account_id, "sent")?.ok_or_else(|| {
            SyncError::fatal(
                "no-sent-folder",
                "There is no folder for sent mail. Assign a sent folder in Preferences > Folders.",
            )
            .into()
        })
    }
}

/// Gmail labels. Only All Mail, Spam and Trash are folders; everything else LIST returns
/// is a view of All Mail and becomes a Label.
pub struct GmailLabelModel {
    pub account_id: String,
}

impl GmailLabelModel {
    fn is_folder(flags: FolderFlags) -> bool {
        flags.intersects(FolderFlags::ALL | FolderFlags::SPAM | FolderFlags::TRASH)
    }
}

impl MailboxModel for GmailLabelModel {
    fn classify_folders(&self, listed: &[ListedFolder]) -> (Vec<Folder>, Vec<Label>) {
        let mut folders = Vec::new();
        let mut labels = Vec::new();
        for l in listed {
            if Self::is_folder(l.flags) {
                folders.push(new_folder(&self.account_id, l));
            } else {
                labels.push(new_label(&self.account_id, l));
            }
        }
        (folders, labels)
    }

    /// All Mail, Spam and Trash are mutually exclusive on Gmail, which has no QRESYNC to
    /// VANISH the old copy promptly, so a copy recorded in one removes the other two.
    fn on_placement_upserted(
        &self,
        store: &Store,
        msg: &mut Message,
        folder: &Folder,
    ) -> Result<()> {
        if matches!(folder.role(), "all" | "spam" | "trash") {
            store.remove_placements_outside_folder(msg, folder.id())?;
        }
        Ok(())
    }

    fn sent_placement_folder(&self, store: &Store) -> Result<Folder> {
        find_by_role(store, &self.account_id, "all")?.ok_or_else(|| {
            SyncError::fatal(
                "no-all-folder",
                "There is no All Mail folder to place sent mail in.",
            )
            .into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(path: &str, flags: FolderFlags) -> ListedFolder {
        ListedFolder {
            path: path.into(),
            delimiter: Some('/'),
            flags,
        }
    }

    #[test]
    fn gmail_keeps_three_folders() {
        let m = GmailLabelModel {
            account_id: "a".into(),
        };
        let (folders, labels) = m.classify_folders(&[
            listed("INBOX", FolderFlags::empty()),
            listed("[Gmail]/All Mail", FolderFlags::ALL),
            listed("[Gmail]/Spam", FolderFlags::SPAM),
            listed("[Gmail]/Trash", FolderFlags::TRASH),
            listed("Work", FolderFlags::empty()),
        ]);
        let paths: Vec<_> = folders.iter().map(|f| f.path().to_owned()).collect();
        assert_eq!(paths, ["[Gmail]/All Mail", "[Gmail]/Spam", "[Gmail]/Trash"]);
        let labels: Vec<_> = labels.iter().map(|f| f.path().to_owned()).collect();
        assert_eq!(labels, ["INBOX", "Work"]);
    }

    #[test]
    fn folders_keep_everything() {
        let m = FolderModel {
            account_id: "a".into(),
        };
        let (folders, labels) = m.classify_folders(&[
            listed("INBOX", FolderFlags::empty()),
            listed("Archive", FolderFlags::ARCHIVE),
        ]);
        assert_eq!(folders.len(), 2);
        assert!(labels.is_empty());
        assert_eq!(folders[0].id(), ms_core::ids::folder_id("a", "INBOX"));
    }
}
