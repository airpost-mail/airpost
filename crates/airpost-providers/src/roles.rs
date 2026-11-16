// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Folder roles. Derived from Mailspring-Sync (`MailUtils::roles`, `roleForFolder`,
//! `roleForFolderViaFlags`, `roleForFolderViaPath` in `MailUtils.cpp`, `COMMON_FOLDER_NAMES`
//! in `constants.h`), Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use airpost_core::constants::{
    CONTAINER_FOLDER_DEFAULT, MAILSPRING_FOLDER_PREFIX_V1, MAILSPRING_FOLDER_PREFIX_V2,
};
use airpost_core::FolderFlags;

/// Every role, in the order roles are assigned (`MailUtils::roles`): for each role the
/// folders' flags are tried before their names, so a `\Sent` folder wins over one named
/// "Sent".
pub const ROLES: [&str; 10] = [
    "all",
    "sent",
    "drafts",
    "spam",
    "important",
    "starred",
    "archive",
    "inbox",
    "trash",
    "snoozed",
];

/// Lowercased UTF-8 folder names and the role each implies (`COMMON_FOLDER_NAMES`).
pub const COMMON_FOLDER_NAMES: &[(&str, &str)] = &[
    ("gelöscht", "trash"),
    ("papierkorb", "trash"),
    ("корзина", "trash"),
    ("[imap]/trash", "trash"),
    ("papelera", "trash"),
    ("koš", "trash"),
    ("[imap]/кор", "trash"),
    ("зина", "trash"),
    ("deleted items", "trash"),
    ("сміття", "trash"),
    ("papierkorb/trash", "trash"),
    ("gelöschte elemente", "trash"),
    ("deleted messages", "trash"),
    ("[gmail]/trash", "trash"),
    ("trash", "trash"),
    ("удаленные", "trash"),
    ("kosz", "trash"),
    ("удалённые", "trash"),
    ("roskaposti", "spam"),
    ("skräppost", "spam"),
    ("spamverdacht", "spam"),
    ("spam", "spam"),
    ("[gmail]/spam", "spam"),
    ("[imap]/spam", "spam"),
    ("垃圾邮件", "spam"),
    ("junk", "spam"),
    ("junk mail", "spam"),
    ("junk e-mail", "spam"),
    ("junk email", "spam"),
    ("bulk mail", "spam"),
    ("спам", "spam"),
    ("inbox", "inbox"),
    ("dateneintrag", "archive"),
    ("archivio", "archive"),
    ("archive", "archive"),
    ("postausgang", "sent"),
    ("sent", "sent"),
    ("[gmail]/sent mail", "sent"),
    ("보낼편지함", "sent"),
    ("elementos enviados", "sent"),
    ("sent items", "sent"),
    ("sent messages", "sent"),
    ("odeslané", "sent"),
    ("sent-mail", "sent"),
    ("sentmail", "sent"),
    ("gesendet", "sent"),
    ("koš/sent items", "sent"),
    ("gesendete elemente", "sent"),
    ("отправленные", "sent"),
    ("sentbox", "sent"),
    ("wysłane", "sent"),
    ("drafts", "drafts"),
    ("draft", "drafts"),
    ("brouillons", "drafts"),
    ("borradores", "drafts"),
    ("черновики", "drafts"),
    ("draftbox", "drafts"),
    ("robocze", "drafts"),
    // Not in the C++ table: Gmail can list Important and Starred without their special-use
    // flag (forum topic 227), and then nothing gave them a role.
    ("[gmail]/important", "important"),
    ("[google mail]/important", "important"),
    ("[gmail]/starred", "starred"),
    ("[google mail]/starred", "starred"),
    // Note: the C++ table keys these two with capitals and looks names up lowercased, so
    // they never match; `[Mailspring]/X` and `Mailspring/X` are handled by prefix instead.
    ("Mailspring/Snoozed", "snoozed"),
    ("Mailspring.Snoozed", "snoozed"),
];

/// The prefixes under which a folder's name is its role besides the account's container
/// folder: the default container and the ones Mailspring created.
const HELPER_FOLDER_PREFIXES: [&str; 3] = [
    CONTAINER_FOLDER_DEFAULT,
    MAILSPRING_FOLDER_PREFIX_V1,
    MAILSPRING_FOLDER_PREFIX_V2,
];

/// The role a folder name implies by `COMMON_FOLDER_NAMES`. `name` is the path with the
/// namespace prefix and leading delimiter stripped and modified UTF-7 decoded; it is
/// lowercased here.
pub fn role_for_folder_name(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    COMMON_FOLDER_NAMES
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|(_, role)| *role)
}

/// `roleForFolderViaFlags`: the role special-use or XLIST flags imply.
pub fn role_for_folder_flags(flags: FolderFlags) -> Option<&'static str> {
    [
        (FolderFlags::ALL, "all"),
        (FolderFlags::SENT, "sent"),
        (FolderFlags::DRAFTS, "drafts"),
        (FolderFlags::SPAM, "spam"),
        (FolderFlags::IMPORTANT, "important"),
        (FolderFlags::STARRED, "starred"),
        (FolderFlags::INBOX, "inbox"),
        (FolderFlags::TRASH, "trash"),
        (FolderFlags::ARCHIVE, "archive"),
    ]
    .into_iter()
    .find(|(flag, _)| flags.contains(*flag))
    .map(|(_, role)| role)
}

/// `roleForFolderViaPath`. Under `Airpost/`, `[Mailspring]/`, `Mailspring/` or the account's
/// container folder the rest of the name is the role (`None` when it is not one); otherwise the name
/// is looked up in [`COMMON_FOLDER_NAMES`].
pub fn role_for_folder_path(
    container_folder_path: &str,
    namespace_prefix: &str,
    path: &str,
    delimiter: Option<char>,
) -> Option<&'static str> {
    let mut path = path;
    if !namespace_prefix.is_empty()
        && path.len() > namespace_prefix.len()
        && path.starts_with(namespace_prefix)
    {
        path = &path[namespace_prefix.len()..];
    }
    if let Some(d) = delimiter {
        if path.chars().count() > 1 && path.starts_with(d) {
            path = &path[d.len_utf8()..];
        }
    }

    if let Some(d) = delimiter {
        let lower = path.to_ascii_lowercase();
        let container = (!container_folder_path.is_empty()).then_some(container_folder_path);
        for prefix in HELPER_FOLDER_PREFIXES.into_iter().chain(container) {
            let prefix = format!("{prefix}{d}").to_ascii_lowercase();
            if lower.len() > prefix.len() && lower.starts_with(&prefix) {
                let rest = &lower[prefix.len()..];
                return ROLES.into_iter().find(|r| *r == rest);
            }
        }
    }

    // LIST returns non-ASCII names in modified UTF-7 (RFC 3501 5.1.3), e.g. "Ko&AWE-".
    let decoded;
    let name = if path.contains('&') {
        decoded = airpost_core::utf7::decode_or_raw(path);
        decoded.as_str()
    } else {
        path
    };
    role_for_folder_name(name)
}

/// `roleForFolder`: flags first, then the path.
pub fn role_for_folder(
    container_folder_path: &str,
    namespace_prefix: &str,
    path: &str,
    delimiter: Option<char>,
    flags: FolderFlags,
) -> Option<&'static str> {
    role_for_folder_flags(flags)
        .or_else(|| role_for_folder_path(container_folder_path, namespace_prefix, path, delimiter))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_role(path: &str) -> Option<&'static str> {
        role_for_folder_path("", "", path, Some('/'))
    }

    #[test]
    fn flags_order() {
        assert_eq!(role_for_folder_flags(FolderFlags::empty()), None);
        assert_eq!(
            role_for_folder_flags(FolderFlags::ALL | FolderFlags::SENT),
            Some("all")
        );
        assert_eq!(
            role_for_folder_flags(FolderFlags::INBOX | FolderFlags::TRASH),
            Some("inbox")
        );
        assert_eq!(role_for_folder_flags(FolderFlags::SPAM), Some("spam"));
        assert_eq!(role_for_folder_flags(FolderFlags::ARCHIVE), Some("archive"));
    }

    #[test]
    fn names() {
        assert_eq!(path_role("INBOX"), Some("inbox"));
        assert_eq!(path_role("Sent Items"), Some("sent"));
        assert_eq!(path_role("[Gmail]/Sent Mail"), Some("sent"));
        assert_eq!(path_role("Junk E-mail"), Some("spam"));
        assert_eq!(path_role("Work"), None);
        assert_eq!(path_role("[Gmail]/Important"), Some("important"));
        assert_eq!(path_role("[Google Mail]/Important"), Some("important"));
        assert_eq!(path_role("[Gmail]/Starred"), Some("starred"));
        // A user label named Important is not the system one.
        assert_eq!(path_role("Important"), None);
        // Modified UTF-7 names (folder-roles-by-localized-name).
        assert_eq!(path_role("Ko&AWE-"), Some("trash"));
        assert_eq!(path_role("&BBoEPgRABDcEOAQ9BDA-"), Some("trash"));
        assert_eq!(path_role("Gel&APY-scht"), Some("trash"));
    }

    #[test]
    fn prefixes() {
        assert_eq!(path_role("Airpost/Snoozed"), Some("snoozed"));
        assert_eq!(path_role("[Mailspring]/Snoozed"), Some("snoozed"));
        assert_eq!(path_role("Mailspring/Snoozed"), Some("snoozed"));
        assert_eq!(path_role("Mailspring/Other"), None);
        assert_eq!(
            role_for_folder_path("", "", "Mailspring.Snoozed", Some('.')),
            Some("snoozed")
        );
        assert_eq!(
            role_for_folder_path("Airpost", "", "Airpost/Trash", Some('/')),
            Some("trash")
        );
        assert_eq!(
            role_for_folder_path("", "INBOX.", "INBOX.Sent", Some('.')),
            Some("sent")
        );
        assert_eq!(
            role_for_folder_path("", "INBOX", "INBOX.Drafts", Some('.')),
            Some("drafts")
        );
        // The prefix is only stripped when the path is longer than it.
        assert_eq!(
            role_for_folder_path("", "INBOX", "INBOX", Some('.')),
            Some("inbox")
        );
    }

    #[test]
    fn flags_win() {
        assert_eq!(
            role_for_folder("", "", "Trash", Some('/'), FolderFlags::SENT),
            Some("sent")
        );
        assert_eq!(
            role_for_folder("", "", "Trash", Some('/'), FolderFlags::empty()),
            Some("trash")
        );
    }
}
