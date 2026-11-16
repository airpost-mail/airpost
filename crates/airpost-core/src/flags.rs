// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Message and folder flag sets with mailcore's bit values (`MCMessageConstants.h`), shared
//! by the IMAP layer, provider role detection and the store.
//!
//! The values are mailcore2's, Copyright © 2001-2013 MailCore team, BSD-3-Clause. See
//! NOTICE.md ("mailcore2").

use bitflags::bitflags;

bitflags! {
    /// IMAP system flags of one message copy (`MessageFlag`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct MessageFlags: u32 {
        const SEEN = 1 << 0;
        const ANSWERED = 1 << 1;
        const FLAGGED = 1 << 2;
        const DELETED = 1 << 3;
        const DRAFT = 1 << 4;
        /// `$MDNSent`
        const MDN_SENT = 1 << 5;
        /// `$Forwarded`
        const FORWARDED = 1 << 6;
        /// `$SubmitPending`
        const SUBMIT_PENDING = 1 << 7;
        /// `$Submitted`
        const SUBMITTED = 1 << 8;
    }
}

impl MessageFlags {
    /// The IMAP flag atoms, e.g. `\Seen \Flagged`, in bit order.
    pub fn to_imap_atoms(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (flag, atom) in [
            (MessageFlags::SEEN, "\\Seen"),
            (MessageFlags::ANSWERED, "\\Answered"),
            (MessageFlags::FLAGGED, "\\Flagged"),
            (MessageFlags::DELETED, "\\Deleted"),
            (MessageFlags::DRAFT, "\\Draft"),
            (MessageFlags::MDN_SENT, "$MDNSent"),
            (MessageFlags::FORWARDED, "$Forwarded"),
            (MessageFlags::SUBMIT_PENDING, "$SubmitPending"),
            (MessageFlags::SUBMITTED, "$Submitted"),
        ] {
            if self.contains(flag) {
                out.push(atom);
            }
        }
        out
    }

    /// The flag for one IMAP atom (case-insensitive), or `None` for keywords outside the set.
    pub fn from_imap_atom(atom: &str) -> Option<MessageFlags> {
        Some(match atom.to_ascii_lowercase().as_str() {
            "\\seen" => MessageFlags::SEEN,
            "\\answered" => MessageFlags::ANSWERED,
            "\\flagged" => MessageFlags::FLAGGED,
            "\\deleted" => MessageFlags::DELETED,
            "\\draft" => MessageFlags::DRAFT,
            "$mdnsent" => MessageFlags::MDN_SENT,
            "$forwarded" => MessageFlags::FORWARDED,
            "$submitpending" => MessageFlags::SUBMIT_PENDING,
            "$submitted" => MessageFlags::SUBMITTED,
            _ => return None,
        })
    }
}

bitflags! {
    /// LIST / XLIST mailbox attributes, SPECIAL-USE included (`IMAPFolderFlag`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct FolderFlags: u32 {
        const MARKED = 1 << 0;
        const UNMARKED = 1 << 1;
        const NO_SELECT = 1 << 2;
        const NO_INFERIORS = 1 << 3;
        const INBOX = 1 << 4;
        const SENT = 1 << 5;
        const STARRED = 1 << 6;
        /// `\All` (and XLIST `\AllMail`)
        const ALL = 1 << 7;
        const TRASH = 1 << 8;
        const DRAFTS = 1 << 9;
        /// `\Junk` (and XLIST `\Spam`)
        const SPAM = 1 << 10;
        const IMPORTANT = 1 << 11;
        const ARCHIVE = 1 << 12;
    }
}

impl FolderFlags {
    /// The flag for one mailbox attribute (case-insensitive). `\NonExistent` (RFC 5258)
    /// counts as `\NoSelect`.
    pub fn from_attribute(attr: &str) -> Option<FolderFlags> {
        Some(match attr.to_ascii_lowercase().as_str() {
            "\\marked" => FolderFlags::MARKED,
            "\\unmarked" => FolderFlags::UNMARKED,
            "\\noselect" | "\\nonexistent" => FolderFlags::NO_SELECT,
            "\\noinferiors" => FolderFlags::NO_INFERIORS,
            "\\inbox" => FolderFlags::INBOX,
            "\\sent" => FolderFlags::SENT,
            "\\starred" | "\\flagged" => FolderFlags::STARRED,
            "\\all" | "\\allmail" => FolderFlags::ALL,
            "\\trash" => FolderFlags::TRASH,
            "\\drafts" => FolderFlags::DRAFTS,
            "\\spam" | "\\junk" => FolderFlags::SPAM,
            "\\important" => FolderFlags::IMPORTANT,
            "\\archive" => FolderFlags::ARCHIVE,
            _ => return None,
        })
    }
}

/// Bits of a placement in `Message._data.folders` (`Placement.hpp`).
pub const PLACEMENT_FLAG_UNREAD: i64 = 1;
pub const PLACEMENT_FLAG_STARRED: i64 = 2;
pub const PLACEMENT_FLAG_DRAFT: i64 = 4;

pub fn placement_flag_bits(unread: bool, starred: bool, draft: bool) -> i64 {
    (if unread { PLACEMENT_FLAG_UNREAD } else { 0 })
        | (if starred { PLACEMENT_FLAG_STARRED } else { 0 })
        | (if draft { PLACEMENT_FLAG_DRAFT } else { 0 })
}
