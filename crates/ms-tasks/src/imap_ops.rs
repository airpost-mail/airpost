// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Server-side moves and deletions shared by the message tasks.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp` (`_resolveNewUIDs`,
//! `_expungeMovedCopies`, `_moveMessagesResilient`, `_removeMessagesResilient`,
//! `_removeMessageCopiesResilient`), Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use ms_core::{logmsg, ErrorCode, MessageFlags, Result, UidSet};
use ms_imap::{FetchKind, ImapSession, StoreOp, UidMapping};
use ms_store::{Folder, Message, Model, Query, Store};
use ms_sync::mail_processor::id_for_message;
use tracing::{error, info, warn};

use crate::messages::TaskPlacement;

fn uids_of(items: &[TaskPlacement]) -> UidSet {
    UidSet::from_uids(items.iter().map(|i| i.placement.remote_uid))
}

/// The UID each item's copy received in `dest`, aligned with `items` (0 when unknown).
/// Copies are matched to UIDs by message id, which does not depend on the folder: the
/// headers of the COPYUID destination set are fetched when the server returned one, and
/// of the newest UIDs in `dest` otherwise.
///
/// COPYUID is not taken as a pairing unless the profile says so (`trust_copyuid_pairing`).
/// RFC 4315 §3 makes it one, but Yahoo answers a multi-message UID MOVE with ascending
/// source and destination ranges while assigning a permutation of the destination range
/// (observed on a live Yahoo account, 2026-09-24; `move-with-permuted-copyuid`). A
/// single-message COPYUID cannot be permuted and is used as is.
fn resolve_new_uids(
    session: &mut dyn ImapSession,
    mapping: &UidMapping,
    dest: &Folder,
    items: &[TaskPlacement],
) -> Vec<u32> {
    let mut result = vec![0u32; items.len()];

    if items.len() == 1 && mapping.0.len() == 1 {
        if let Some(uid) = mapping.get(items[0].placement.remote_uid) {
            result[0] = uid;
            return result;
        }
    }
    if session.profile().trust_copyuid_pairing && !mapping.0.is_empty() {
        for (i, item) in items.iter().enumerate() {
            result[i] = mapping.get(item.placement.remote_uid).unwrap_or(0);
        }
        if result.iter().all(|u| *u != 0) {
            return result;
        }
        result.fill(0);
    }

    let mut copied: BTreeSet<u32> = mapping.0.iter().map(|(_, d)| *d).collect();
    let candidates = if !copied.is_empty() {
        UidSet::from_uids(copied.iter().copied())
    } else {
        let Ok(status) = session.status(dest.path()) else {
            return result;
        };
        // Moves append at the top of the destination; twice the item count covers gaps in
        // UID assignment without underflowing below 1.
        let search_range = (items.len() as u32).saturating_mul(2);
        let min = if status.uid_next > search_range {
            status.uid_next - search_range
        } else {
            1
        };
        UidSet::range(min, None)
    };

    let Ok(fetched) = session.fetch_by_uid(dest.path(), FetchKind::Heavy, &candidates) else {
        return result;
    };
    // Several copies of one message (Exchange duplicates) arrive as several UIDs that hash
    // to the same id; each moved copy takes one so no two commit to the same UID.
    let mut uids_by_id: HashMap<String, VecDeque<u32>> = HashMap::new();
    for m in &fetched.messages {
        uids_by_id
            .entry(id_for_message(dest.account_id(), dest.path(), m))
            .or_default()
            .push_back(m.uid);
    }
    for uids in uids_by_id.values_mut() {
        uids.make_contiguous().sort_unstable();
    }

    let mut unmatched = Vec::new();
    let mut unpaired = 0;
    for (i, item) in items.iter().enumerate() {
        let Some(uid) = uids_by_id
            .get_mut(&item.message_id)
            .and_then(VecDeque::pop_front)
        else {
            unmatched.push(i);
            continue;
        };
        result[i] = uid;
        copied.remove(&uid);
        if mapping
            .get(item.placement.remote_uid)
            .is_some_and(|paired| paired != uid)
        {
            unpaired += 1;
        }
    }
    if unpaired > 0 {
        warn!(
            "{}",
            logmsg::copyuid_mispaired(dest.path(), unpaired, items.len())
        );
    }

    // A message without a Date header has an id that includes its folder, so it never
    // matches here; one such copy is identified by the one COPYUID it left unclaimed.
    if unmatched.len() == 1 && copied.len() == 1 {
        result[unmatched[0]] = *copied.iter().next().expect("one copied UID");
    }
    result
}

/// Removes the source copies of a COPY-based move. A plain EXPUNGE would also purge every
/// message another client only marked `\Deleted` (Outlook desktop's "mark for deletion"
/// mode), so with UIDPLUS it expunges exactly these UIDs (RFC 4315 §2.1), as Thunderbird
/// does; the plain EXPUNGE is the fallback when UIDPLUS is absent or the server refuses
/// UID EXPUNGE. After a dropped connection the UID EXPUNGE may already have run, so the
/// copies are left flagged `\Deleted` rather than risking a plain EXPUNGE on the
/// reconnected session (`move-without-move-connection-drop`).
fn expunge_moved_copies(session: &mut dyn ImapSession, path: &str, uids: &UidSet) -> Result<()> {
    if let Err(e) = session.store_flags(path, uids, StoreOp::Add, MessageFlags::DELETED) {
        warn!(
            "-X Could not flag moved copies \\Deleted in {path}, leaving them (error: {})",
            e.code.name()
        );
        return Ok(());
    }
    if session.capabilities().has("UIDPLUS") {
        match session.expunge_uids(path, uids) {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!("-X UID EXPUNGE failed in {path} (error: {})", e.code.name());
                if e.code != ErrorCode::Expunge {
                    return Ok(());
                }
            }
        }
    }
    info!("{}", logmsg::expunging_all_deleted(path));
    session
        .expunge(path)
        .map_err(|e| e.into_sync("moveMessages(copy cleanup)"))?;
    Ok(())
}

/// Moves the items' copies out of `path` into `dest` with UID MOVE, or COPY + `\Deleted` +
/// EXPUNGE when the server lacks MOVE, and records each copy's new UID on the item. A copy
/// whose new UID cannot be determined is logged and left as it was: its row keeps the
/// optimistic marker and the destination's next scan records it.
pub(crate) fn move_messages_resilient(
    session: &mut dyn ImapSession,
    path: &str,
    dest: &Folder,
    items: &mut [TaskPlacement],
) -> Result<()> {
    let uids = uids_of(items);
    let (mapping, must_apply_attributes) = if session.capabilities().has("MOVE") {
        let m = session
            .move_uids(path, &uids, dest.path())
            .map_err(|e| e.into_sync("moveMessages"))?;
        (m, false)
    } else {
        let m = session
            .copy_uids(path, &uids, dest.path())
            .map_err(|e| e.into_sync("moveMessages(copy)"))?;
        expunge_moved_copies(session, path, &uids)?;
        (m, true)
    };

    let new_uids = resolve_new_uids(session, &mapping, dest, items);
    for (item, new_uid) in items.iter_mut().zip(new_uids) {
        if new_uid == 0 {
            error!(
                "{}",
                logmsg::could_not_find_new_uid(&item.message_id, dest.path())
            );
            continue;
        }
        item.moved = true;
        item.moved_uid = new_uid;
    }

    if must_apply_attributes {
        for item in items.iter().filter(|i| i.moved) {
            let p = &item.placement;
            let mut flags = MessageFlags::empty();
            if p.starred {
                flags |= MessageFlags::FLAGGED;
            }
            if !p.unread {
                flags |= MessageFlags::SEEN;
            }
            if p.draft {
                flags |= MessageFlags::DRAFT;
            }
            if !flags.is_empty() {
                // Note: best effort; the destination's next scan reads whatever it has.
                let _ = session.store_flags(
                    dest.path(),
                    &UidSet::single(item.moved_uid),
                    StoreOp::Set,
                    flags,
                );
            }
        }
    }
    Ok(())
}

/// Permanently removes `uids` from `path`: flags them `\Deleted`, moves them to Trash when
/// the server has MOVE and the account a Trash folder (on Gmail that is what deletes a
/// message), and expunges wherever they are. Returns whether the messages have left
/// `path`.
pub(crate) fn remove_messages_resilient(
    session: &mut dyn ImapSession,
    store: &Store,
    account_id: &str,
    path: &str,
    uids: &UidSet,
) -> Result<bool> {
    if let Err(e) = session.store_flags(path, uids, StoreOp::Add, MessageFlags::DELETED) {
        info!(
            "X- removeMessages could not add deleted flag (error: {})",
            e.code.name()
        );
        return Ok(false);
    }

    let mut path = path.to_owned();
    let mut uids = uids.clone();
    let mut moved_to_trash = false;

    let trash_path = if session.capabilities().has("MOVE") {
        store
            .find::<Folder>(
                &Query::new()
                    .equal("accountId", account_id)
                    .equal("role", "trash"),
            )?
            .map(|f| f.path().to_owned())
    } else {
        None
    };

    if let Some(trash_path) = trash_path {
        match session.move_uids(&path, &uids, &trash_path) {
            Err(e) => info!(
                "X- removeMessages could not move to {trash_path} (error: {})",
                e.code.name()
            ),
            Ok(mapping) => {
                // The old UIDs mean nothing in Trash: expunge the mapped ones, or the
                // whole folder without a mapping.
                moved_to_trash = true;
                uids = UidSet::from_uids(mapping.0.iter().map(|(_, d)| *d));
                path = trash_path;
                if !uids.is_empty() {
                    // Gmail drops \Deleted when a message is moved.
                    info!("-- removeMessages re-applying deleted flag after moving to {path}");
                    if let Err(e) =
                        session.store_flags(&path, &uids, StoreOp::Add, MessageFlags::DELETED)
                    {
                        info!(
                            "X- removeMessages could not add deleted flag (error: {})",
                            e.code.name()
                        );
                    }
                }
            }
        }
    }

    let result = if !uids.is_empty() {
        info!("-- removeMessages Expunging (UIDs) from {path}");
        match session.expunge_uids(&path, &uids) {
            Ok(()) => Ok(()),
            Err(e) => {
                info!(
                    "-- removeMessages Expunge (UIDs) failed (error: {})",
                    e.code.name()
                );
                info!("-- removeMessages Expunging (Basic) from {path}");
                session.expunge(&path)
            }
        }
    } else {
        info!("-- removeMessages Expunging (Basic) from {path}");
        session.expunge(&path)
    };
    if let Err(e) = &result {
        info!(
            "X- removeMessages Expunge failed (error: {})",
            e.code.name()
        );
    }
    Ok(moved_to_trash || result.is_ok())
}

/// Deletes the message's server copies, grouped by folder. Used for drafts, whose copies
/// are never moved anywhere.
pub(crate) fn remove_message_copies_resilient(
    session: &mut dyn ImapSession,
    store: &Store,
    account_id: &str,
    msg: &Message,
) -> Result<()> {
    let mut uids_by_path: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for p in store.placements_for_message(msg.id())? {
        if p.remote_uid == 0 {
            continue;
        }
        let Some(folder) = store.folder_by_id(account_id, &p.folder_id)? else {
            continue;
        };
        uids_by_path
            .entry(folder.path().to_owned())
            .or_default()
            .push(p.remote_uid);
    }
    for (path, uids) in uids_by_path {
        let set = UidSet::from_uids(uids.iter().copied());
        info!(
            "-- Deleting {} copies of {} from {path}",
            set.iter().count(),
            msg.id()
        );
        remove_messages_resilient(session, store, account_id, &path, &set)?;
    }
    Ok(())
}
