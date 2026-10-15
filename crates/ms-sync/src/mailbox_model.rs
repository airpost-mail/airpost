//! Folders versus Gmail labels (plan 4.3 and 4.4). Gmail is the one provider that changes
//! the data model: labels instead of folders, exactly one placement (All Mail, Spam or
//! Trash), and an All Mail placement on send.

use ms_core::Result;
use ms_imap::ListedFolder;
use ms_store::{Folder, Label, Message, Store};

pub trait MailboxModel {
    /// Splits LIST output into the folders that hold placements and the labels.
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

/// IMAP folders (every non-Gmail server).
pub struct FolderModel {
    pub account_id: String,
}

impl MailboxModel for FolderModel {
    fn classify_folders(&self, listed: &[ListedFolder]) -> (Vec<Folder>, Vec<Label>) {
        let _ = listed;
        todo!("P2-B: FolderModel::classify_folders")
    }

    fn on_placement_upserted(
        &self,
        store: &Store,
        msg: &mut Message,
        folder: &Folder,
    ) -> Result<()> {
        let _ = (store, msg, folder);
        todo!("P2-B: FolderModel::on_placement_upserted")
    }

    fn sent_placement_folder(&self, store: &Store) -> Result<Folder> {
        let _ = store;
        todo!("P2-B: FolderModel::sent_placement_folder")
    }
}

/// Gmail labels.
pub struct GmailLabelModel {
    pub account_id: String,
}

impl MailboxModel for GmailLabelModel {
    fn classify_folders(&self, listed: &[ListedFolder]) -> (Vec<Folder>, Vec<Label>) {
        let _ = listed;
        todo!("P2-B: GmailLabelModel::classify_folders")
    }

    fn on_placement_upserted(
        &self,
        store: &Store,
        msg: &mut Message,
        folder: &Folder,
    ) -> Result<()> {
        let _ = (store, msg, folder);
        todo!("P2-B: GmailLabelModel::on_placement_upserted")
    }

    fn sent_placement_folder(&self, store: &Store) -> Result<Folder> {
        let _ = store;
        todo!("P2-B: GmailLabelModel::sent_placement_folder")
    }
}
