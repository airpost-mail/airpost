//! `MailProcessor`: turns fetched headers and bodies into Message, Thread, Contact and File
//! rows, and runs the orphan sweep.

use ms_core::{Account, Result};
use ms_imap::ImapMessage;
use ms_mime::ParsedMessage;
use ms_store::{Folder, Message, Store};

pub struct MailProcessor<'a> {
    pub account: &'a Account,
    pub store: &'a Store,
    pub is_gmail: bool,
}

/// What to do with messages that end up with no placement after a refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnplacedMessages {
    Keep,
    Remove,
}

impl<'a> MailProcessor<'a> {
    pub fn new(account: &'a Account, store: &'a Store) -> Self {
        MailProcessor {
            account,
            store,
            is_gmail: false,
        }
    }

    /// Inserts a newly seen message, or updates the existing one with this copy's
    /// placement and flags (`insertFallbackToUpdateMessage`).
    pub fn insert_fallback_to_update_message(
        &self,
        remote: &ImapMessage,
        folder: &Folder,
        sync_data_timestamp: i64,
    ) -> Result<Option<Message>> {
        let _ = (remote, folder, sync_data_timestamp);
        todo!("P2-B: MailProcessor::insertFallbackToUpdateMessage")
    }

    pub fn delete_vanished_placements(&self, folder: &Folder, uids: &[u32]) -> Result<()> {
        let _ = (folder, uids);
        todo!("P2-B: MailProcessor::deleteVanishedPlacements")
    }

    pub fn delete_unassigned_placements(&self, folder: &Folder) -> Result<()> {
        let _ = folder;
        todo!("P2-B: MailProcessor::deleteUnassignedPlacements")
    }

    pub fn detach_messages_from_folder(&self, folder: &Folder) -> Result<()> {
        let _ = folder;
        todo!("P2-B: MailProcessor::detachMessagesFromFolder")
    }

    pub fn refresh_messages(
        &self,
        message_ids: &[String],
        unplaced: UnplacedMessages,
        reason: &str,
    ) -> Result<()> {
        let _ = (message_ids, unplaced, reason);
        todo!("P2-B: MailProcessor::refreshMessages")
    }

    /// Removes messages orphaned before `sweep_before` (see "Orphan sweep" in
    /// legacy/mailsync-cpp/CLAUDE.md).
    pub fn sweep_expired_orphans(&self, sweep_before: i64, pass_started_at: i64) -> Result<()> {
        let _ = (sweep_before, pass_started_at);
        todo!("P2-B: MailProcessor::sweepExpiredOrphans")
    }

    /// Stores a fetched body, its files and its snippet (`retrievedMessageBody`).
    pub fn retrieved_message_body(
        &self,
        message: &mut Message,
        parsed: &ParsedMessage,
    ) -> Result<()> {
        let _ = (message, parsed);
        todo!("P2-F: MailProcessor::retrievedMessageBody")
    }
}
