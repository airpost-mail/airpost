// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The message tasks: ChangeUnread, ChangeStarred, ChangeFolder (and its undo) and
//! ChangeLabels.
//!
//! Derived from Mailspring-Sync `MailSync/TaskProcessor.cpp` (`performLocalChangeOnMessages`,
//! `performRemoteChangeOnMessages`, `confirmPlacementChange`, `_movesForMessage`,
//! `_restoreMovesForMessage` and the `_apply*` functions), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! The placement rules (optimistic moves as pending placements, `undoPlacements` /
//! `restorePlacements`, the `syncedAt` hold) are in "Message Identity and Placements" in
//! Mailspring-Sync's CLAUDE.md.

use std::collections::{BTreeMap, BTreeSet};

use airpost_core::constants::ls;
use airpost_core::{logmsg, Error, MessageFlags, Result, UidSet};
use airpost_imap::{ImapSession, StoreOp};
use airpost_providers::MailboxModelKind;
use airpost_store::{Folder, Message, MessageSnapshot, Model, Placement, Query, Store, Thread};
use airpost_sync::mail_processor::{MailProcessor, UnplacedMessages};
use serde_json::{json, Value};
use tracing::{error, warn};

use crate::imap_ops::move_messages_resilient;
use crate::json::{at, get_bool, get_i64, get_str, has, str_elements};
use crate::TaskProcessor;

/// One server copy a task addresses and, after the remote phase, what became of it. The
/// row is captured before any network I/O; the confirm step reloads the message and
/// applies the outcome to the row that still matches (folder, UID).
#[derive(Clone, Debug)]
pub(crate) struct TaskPlacement {
    pub message_id: String,
    pub placement: Placement,
    /// Moves: where this copy should end up. Empty for flag and label tasks.
    pub dest_folder_id: String,
    /// The copy is now at (`dest_folder_id`, `moved_uid`).
    pub moved: bool,
    pub moved_uid: u32,
}

struct PlacementMove {
    placement: Placement,
    dest_folder_id: String,
}

/// Runs inside the local phase's transaction with the message's placements in hand.
pub(crate) type LocalChange = fn(&Store, &mut Message, &[Placement], &mut Value) -> Result<()>;

/// Runs once per server folder holding a copy, with the items there.
pub(crate) type RemoteChange =
    fn(&mut dyn ImapSession, &Store, &str, &Folder, &mut [TaskPlacement], &Value) -> Result<()>;

fn uids_of(items: &[TaskPlacement]) -> UidSet {
    UidSet::from_uids(items.iter().map(|i| i.placement.remote_uid))
}

fn client_visible_state(msg: &Message) -> Value {
    json!({
        "folders": msg.folders(),
        "labels": msg.labels(),
        "unread": msg.is_unread(),
        "starred": msg.is_starred(),
        "draft": msg.is_draft(),
    })
}

/// `inflateMessages`: the messages of `threadIds`, else of `messageIds`.
pub(crate) fn inflate_messages(store: &Store, data: &Value) -> Result<Vec<Message>> {
    if has(data, "threadIds") {
        let ids = str_elements(at(data, "threadIds"))?;
        store.find_large_set::<Message>("threadId", &ids)
    } else if has(data, "messageIds") {
        let ids = str_elements(at(data, "messageIds"))?;
        store.find_large_set::<Message>("id", &ids)
    } else {
        Ok(Vec::new())
    }
}

/// A flag task whose STORE failed leaves its copies with the user's new flags while the
/// server keeps the old ones, and on a CONDSTORE server the copy's modseq never changed,
/// so CHANGEDSINCE never reports it again. Zeroing `lastDeep` makes the background
/// worker's next pass run the full-folder attributes scan, which compares flags
/// (`star-rejected-on-condstore`). `save_folder_status` writes only this key over the
/// current row, so the reset survives unless that pass ran the full scan itself.
fn rescan_folders_for_flags<'a>(
    store: &Store,
    account_id: &str,
    folder_ids: impl Iterator<Item = &'a String>,
) -> Result<()> {
    for id in folder_ids {
        let Some(mut folder) = store.find::<Folder>(
            &Query::new()
                .equal("accountId", account_id)
                .equal("id", id.as_str()),
        )?
        else {
            continue;
        };
        let initial = folder.local_status().clone();
        folder
            .local_status_mut()
            .insert(ls::LAST_DEEP.into(), json!(0));
        store.save_folder_status(&mut folder, &initial)?;
    }
    Ok(())
}

// -- Flags and labels ---------------------------------------------------------------------

/// Records in `undoFlags` a message whose flag the local phase changes, with its value
/// before, which is what its undo and a revert put back.
fn record_flag(data: &mut Value, msg: &Message, before: bool, after: bool) {
    if before == after {
        return;
    }
    if !data[UNDO_FLAGS].is_object() {
        data[UNDO_FLAGS] = json!({});
    }
    data[UNDO_FLAGS][msg.id()] = json!(before);
}

/// Task data: each message a ChangeUnread or ChangeStarred local phase changed, with its
/// value before.
pub(crate) const UNDO_FLAGS: &str = "undoFlags";
/// Task data: the X-GM-LABELS each message a ChangeLabels local phase changed gained
/// (`added`) and lost (`removed`).
pub(crate) const UNDO_LABELS: &str = "undoLabels";

pub(crate) fn apply_unread(
    store: &Store,
    msg: &mut Message,
    _: &[Placement],
    data: &mut Value,
) -> Result<()> {
    let unread = get_bool(at(data, "unread"))?;
    record_flag(data, msg, msg.is_unread(), unread);
    store.set_placement_unread(msg, unread)
}

pub(crate) fn apply_unread_in_imap_folder(
    session: &mut dyn ImapSession,
    _: &Store,
    _: &str,
    source: &Folder,
    items: &mut [TaskPlacement],
    data: &Value,
) -> Result<()> {
    let op = if get_bool(at(data, "unread"))? {
        StoreOp::Remove
    } else {
        StoreOp::Add
    };
    session
        .store_flags(source.path(), &uids_of(items), op, MessageFlags::SEEN)
        .map_err(|e| e.into_sync("storeFlagsByUID"))?;
    Ok(())
}

pub(crate) fn apply_starred(
    store: &Store,
    msg: &mut Message,
    _: &[Placement],
    data: &mut Value,
) -> Result<()> {
    let starred = get_bool(at(data, "starred"))?;
    record_flag(data, msg, msg.is_starred(), starred);
    store.set_placement_starred(msg, starred)
}

pub(crate) fn apply_starred_in_imap_folder(
    session: &mut dyn ImapSession,
    _: &Store,
    _: &str,
    source: &Folder,
    items: &mut [TaskPlacement],
    data: &Value,
) -> Result<()> {
    let op = if get_bool(at(data, "starred"))? {
        StoreOp::Add
    } else {
        StoreOp::Remove
    };
    session
        .store_flags(source.path(), &uids_of(items), op, MessageFlags::FLAGGED)
        .map_err(|e| e.into_sync("storeFlagsByUID"))?;
    Ok(())
}

/// The X-GM-LABELS value of a Label JSON: the system labels by role, others by path.
/// `[Gmail]/Important` is `\Important` even without the role, which is how the messages
/// carry it; C++ sent the path and the removal failed with ErrorStore (forum topic 227).
pub(crate) fn xgm_key_for_label(label: &Value) -> Result<String> {
    let role = get_str(at(label, "role"))?;
    let path = get_str(at(label, "path"))?;
    let lower = path.to_ascii_lowercase();
    let important_path = ["[gmail]/important", "[google mail]/important"].contains(&lower.as_str());
    Ok(match role.as_str() {
        "inbox" => "\\Inbox".to_owned(),
        "important" => "\\Important".to_owned(),
        _ if important_path => "\\Important".to_owned(),
        _ => path,
    })
}

pub(crate) fn apply_labels(
    store: &Store,
    msg: &mut Message,
    _: &[Placement],
    data: &mut Value,
) -> Result<()> {
    let before = str_elements(msg.labels())?;
    let mut labels = before.clone();
    for item in crate::json::elements(at(data, "labelsToAdd")) {
        let key = xgm_key_for_label(item)?;
        if !labels.contains(&key) {
            labels.push(key);
        }
    }
    for item in crate::json::elements(at(data, "labelsToRemove")) {
        let key = xgm_key_for_label(item)?;
        labels.retain(|l| *l != key);
    }
    let added: Vec<&String> = labels.iter().filter(|l| !before.contains(l)).collect();
    let removed: Vec<&String> = before.iter().filter(|l| !labels.contains(l)).collect();
    if !added.is_empty() || !removed.is_empty() {
        if !data[UNDO_LABELS].is_object() {
            data[UNDO_LABELS] = json!({});
        }
        data[UNDO_LABELS][msg.id()] = json!({"added": added, "removed": removed});
    }
    set_sorted_labels(store, msg, labels)
}

/// The scan sorts the labels it reads from the server and compares them positionally,
/// so an unsorted local set would register as a change and cost a no-op update + delta.
pub(crate) fn set_sorted_labels(
    store: &Store,
    msg: &mut Message,
    mut labels: Vec<String>,
) -> Result<()> {
    labels.sort();
    store.set_placement_labels(msg, &labels)
}

pub(crate) fn apply_label_change_in_imap_folder(
    session: &mut dyn ImapSession,
    _: &Store,
    _: &str,
    source: &Folder,
    items: &mut [TaskPlacement],
    data: &Value,
) -> Result<()> {
    let uids = uids_of(items);
    let to_add = crate::json::elements(at(data, "labelsToAdd"))
        .into_iter()
        .map(xgm_key_for_label)
        .collect::<Result<Vec<_>>>()?;
    let to_remove = crate::json::elements(at(data, "labelsToRemove"))
        .into_iter()
        .map(xgm_key_for_label)
        .collect::<Result<Vec<_>>>()?;
    if !to_add.is_empty() {
        session
            .store_labels(source.path(), &uids, StoreOp::Add, &to_add)
            .map_err(|e| e.into_sync("storeLabelsByUID - add"))?;
    }
    if !to_remove.is_empty() {
        session
            .store_labels(source.path(), &uids, StoreOp::Remove, &to_remove)
            .map_err(|e| e.into_sync("storeLabelsByUID - remove"))?;
    }
    Ok(())
}

// -- Moves --------------------------------------------------------------------------------

/// Undo of a ChangeFolderTask. The undo task carries `restorePlacements` (`{ messageId:
/// [{ folderId, bits }, ...] }`, the `undoPlacements` the original recorded: the folder
/// each copy it moved was shown in and that copy's flag bits) and `sourceFolderIds` = [the
/// original destination]. As many of the message's copies in that destination as there
/// are entries go back, one to each recorded folder. Copies of a message share their bytes
/// but not their flags (a self-sent message is typically unread in Inbox and read in Sent),
/// so each entry first takes a copy whose bits still equal the recorded ones, and the rest
/// are paired in order. Within both rounds the copies the original moved are preferred:
/// those still in flight to the destination, then the highest UIDs there, since a moved
/// copy lands above every UID the folder already held (RFC 3501 §2.3.1.1). A copy this
/// undo's local phase already marked for a recorded folder keeps that course, which is how
/// the remote phase finds the local phase's choice (`undo-before-remote-phase`).
fn restore_moves_for_message(
    msg: &Message,
    placements: &[Placement],
    data: &Value,
) -> Result<Vec<PlacementMove>> {
    let mut moves = Vec::new();
    let Some(entries) = at(data, "restorePlacements")
        .get(msg.id())
        .and_then(Value::as_array)
    else {
        return Ok(moves);
    };
    let Some(sources) = at(data, "sourceFolderIds")
        .as_array()
        .filter(|s| !s.is_empty())
    else {
        return Ok(moves);
    };
    let from = get_str(&sources[0])?;
    let mut targets = entries
        .iter()
        .map(|e| Ok((get_str(at(e, "folderId"))?, get_i64(at(e, "bits"))?)))
        .collect::<Result<Vec<(String, i64)>>>()?;

    let mut candidates: Vec<Placement> = Vec::new();
    for p in placements {
        if p.remote_uid == 0 {
            continue;
        }
        let mut target = None;
        if let Some(pending) = &p.pending_folder_id {
            target = targets
                .iter()
                .position(|t| t.0 == *pending && t.1 == p.flag_bits())
                .or_else(|| targets.iter().position(|t| t.0 == *pending));
        }
        if let Some(i) = target {
            moves.push(PlacementMove {
                placement: p.clone(),
                dest_folder_id: p.pending_folder_id.clone().unwrap_or_default(),
            });
            targets.remove(i);
        } else if p.reported_folder_id() == from {
            candidates.push(p.clone());
        }
    }
    candidates.sort_by(|a, b| {
        let a_in_flight = a.folder_id != from;
        let b_in_flight = b.folder_id != from;
        match (a_in_flight, b_in_flight) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => std::cmp::Ordering::Equal,
            (false, false) => b.remote_uid.cmp(&a.remote_uid),
        }
    });

    let mut target_paired = vec![false; targets.len()];
    let mut candidate_used = vec![false; candidates.len()];
    for (t, target) in targets.iter().enumerate() {
        for (c, candidate) in candidates.iter().enumerate() {
            if !candidate_used[c] && candidate.flag_bits() == target.1 {
                moves.push(PlacementMove {
                    placement: candidate.clone(),
                    dest_folder_id: target.0.clone(),
                });
                target_paired[t] = true;
                candidate_used[c] = true;
                break;
            }
        }
    }
    let mut c = 0;
    for (t, target) in targets.iter().enumerate() {
        while c < candidates.len() && candidate_used[c] {
            c += 1;
        }
        if target_paired[t] || c == candidates.len() {
            continue;
        }
        moves.push(PlacementMove {
            placement: candidates[c].clone(),
            dest_folder_id: target.0.clone(),
        });
        candidate_used[c] = true;
    }
    Ok(moves)
}

/// Which copies a ChangeFolderTask moves, and where. Both phases derive this from the task
/// data and the message's current rows, so the remote phase finds the same copies after
/// the local phase (which only marks them) and after an earlier task has moved them on. A
/// copy at UID 0 is never selected: no scan can report where it went, so a marker on it
/// would show it in the destination forever.
///
/// A move to Trash or Spam takes every copy; any other move takes the copies in
/// `sourceFolderIds` when the client named the folder it was looking at, else every copy
/// outside Sent and Drafts. A copy is matched by the folder it is in or the one it is
/// optimistically shown in, a copy already marked for this destination is always included
/// so a re-run finishes it, and a copy already in the destination is only touched to pull
/// it back from a pending move elsewhere. A copy is moved even when the destination
/// already holds one (`move-into-folder-holding-a-copy`).
fn moves_for_message(
    store: &Store,
    msg: &Message,
    placements: &[Placement],
    data: &Value,
) -> Result<Vec<PlacementMove>> {
    if at(data, "restorePlacements").is_object() {
        return restore_moves_for_message(msg, placements, data);
    }

    let folder = at(data, "folder");
    let dest = get_str(at(folder, "id"))?;
    let dest_role = at(folder, "role").as_str().unwrap_or("");
    let every_copy = dest_role == "trash" || dest_role == "spam";
    let sources: BTreeSet<String> = match at(data, "sourceFolderIds") {
        Value::Array(a) => a.iter().map(get_str).collect::<Result<_>>()?,
        _ => BTreeSet::new(),
    };
    let matches = |folder_id: &str| -> Result<bool> {
        if every_copy {
            return Ok(true);
        }
        if !sources.is_empty() {
            return Ok(sources.contains(folder_id));
        }
        let role = msg.folder_role(store, folder_id)?;
        Ok(role != "sent" && role != "drafts")
    };

    let mut moves = Vec::new();
    for p in placements {
        if p.remote_uid == 0 {
            continue;
        }
        let pending = p.pending_folder_id.as_deref().unwrap_or("");
        let pending_elsewhere = !pending.is_empty() && pending != dest;
        let selected = if pending == dest {
            true
        } else if p.folder_id == dest {
            pending_elsewhere && matches(pending)?
        } else {
            matches(&p.folder_id)? || (pending_elsewhere && matches(pending)?)
        };
        if selected {
            moves.push(PlacementMove {
                placement: p.clone(),
                dest_folder_id: dest.clone(),
            });
        }
    }
    Ok(moves)
}

/// Marks the selected copies so the client sees them in the destination immediately, and
/// records as `undoPlacements` the folder each one was shown in and its flag bits, which
/// is where and how its undo sends a copy back.
pub(crate) fn apply_folder(
    store: &Store,
    msg: &mut Message,
    placements: &[Placement],
    data: &mut Value,
) -> Result<()> {
    let mut shown_in = Vec::new();
    for mv in moves_for_message(store, msg, placements, data)? {
        let p = &mv.placement;
        if p.reported_folder_id() != mv.dest_folder_id {
            shown_in.push(json!({"folderId": p.reported_folder_id(), "bits": p.flag_bits()}));
        }
        store.begin_placement_move(msg, &p.folder_id, p.remote_uid, &mv.dest_folder_id)?;
    }
    if !shown_in.is_empty() {
        if !data["undoPlacements"].is_object() {
            data["undoPlacements"] = json!({});
        }
        data["undoPlacements"][msg.id()] = Value::Array(shown_in);
    }
    Ok(())
}

fn move_destination(
    store: &Store,
    account_id: &str,
    folder_id: &str,
    data: &Value,
) -> Result<Folder> {
    if let Some(f) = store.folder_by_id(account_id, folder_id)? {
        return Ok((*f).clone());
    }
    let folder = at(data, "folder");
    if folder.is_object() && get_str(at(folder, "id"))? == folder_id {
        return Folder::from_json(folder.clone());
    }
    Err(airpost_core::SyncError::fatal(
        "no-matching-folder",
        "The destination folder no longer exists.",
    )
    .into())
}

/// Moves the items' copies out of `source`, one MOVE per destination. A copy already in
/// its destination (an undo settling a copy it found at home) is only confirmed in place.
pub(crate) fn apply_folder_move_in_imap_folder(
    session: &mut dyn ImapSession,
    store: &Store,
    account_id: &str,
    source: &Folder,
    items: &mut [TaskPlacement],
    data: &Value,
) -> Result<()> {
    let mut by_dest: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, item) in items.iter_mut().enumerate() {
        if item.placement.folder_id == item.dest_folder_id {
            item.moved = true;
            item.moved_uid = item.placement.remote_uid;
        } else {
            by_dest
                .entry(item.dest_folder_id.clone())
                .or_default()
                .push(i);
        }
    }
    for (dest_id, idxs) in by_dest {
        let dest = move_destination(store, account_id, &dest_id, data)?;
        let mut group: Vec<TaskPlacement> = idxs.iter().map(|&i| items[i].clone()).collect();
        let result = move_messages_resilient(session, source.path(), &dest, &mut group);
        for (item, &i) in group.into_iter().zip(&idxs) {
            items[i] = item;
        }
        result?;
    }
    Ok(())
}

// -- The two phases -----------------------------------------------------------------------

impl TaskProcessor<'_> {
    /// Applies `change` to every message of the task in one transaction and holds each
    /// against server data for 24 hours (`_sa` / `_suc`), so a scan cannot revert the
    /// change before the remote phase has made it on the server.
    pub(crate) fn perform_local_change_on_messages(
        &self,
        data: &mut Value,
        change: LocalChange,
    ) -> Result<()> {
        self.store.transaction("performLocalChangeOnMessages", |s| {
            let mut messages = inflate_messages(s, data)?;
            let recompute_thread_attributes = has(data, "threadIds");
            let hold_until = s.now() + 24 * 60 * 60;

            for msg in messages.iter_mut() {
                if recompute_thread_attributes {
                    msg.skip_thread_updates_after_save = true;
                }
                let placements = s.placements_for_message(msg.id())?;
                change(s, msg, &placements, data)?;
                msg.set_sync_unsaved_changes(msg.sync_unsaved_changes() + 1);
                msg.set_synced_at(hold_until);
                s.save(msg)?;
            }

            // With the thread's messages in hand, recount the threads from scratch, which
            // also repairs refcounts that drifted.
            if recompute_thread_attributes {
                let thread_ids = str_elements(at(data, "threadIds"))?;
                for chunk in thread_ids.chunks(500) {
                    let mut threads: BTreeMap<String, Thread> = s
                        .find_all::<Thread>(&Query::new().equal_any("id", chunk.iter()))?
                        .into_iter()
                        .map(|t| (t.id().to_owned(), t))
                        .collect();
                    for t in threads.values_mut() {
                        t.reset_counted_attributes();
                    }
                    for msg in &messages {
                        if let Some(t) = threads.get_mut(msg.thread_id()) {
                            t.apply_message_attribute_changes(
                                &MessageSnapshot::empty(),
                                Some(msg),
                                s,
                            )?;
                        }
                    }
                    for t in threads.values_mut() {
                        s.save(t)?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Runs the server side of a message task: every server copy the task addresses is
    /// grouped by the folder holding it and `apply` runs once per folder. The network I/O
    /// happens outside any transaction, so the messages are reloaded afterwards and the
    /// outcome applied to their rows then, together with releasing the `_sa` hold. The
    /// confirm save usually changes nothing the client can see (the optimistic marker
    /// already reported the destination), so its deltas are dropped unless the
    /// client-visible state changed.
    ///
    /// A failing folder ends the server work, and the error is returned only after the
    /// outcome is applied: copies already moved are committed, copies that were not go
    /// back to the folder the server has them in, and the hold is released either way.
    /// Flags a failed flag task wrote locally are left for the folders' next full scan, and
    /// `reached` gets the messages a copy of which the server changed, so a revert
    /// ([`TaskProcessor::revert_local_change`]) leaves those to the scan. When the task will
    /// run again, what reached the server is recorded and the rest keeps its optimistic
    /// state and the hold on the message's changes.
    pub(crate) fn perform_remote_change_on_messages(
        &self,
        data: &Value,
        session: &mut dyn ImapSession,
        is_move: bool,
        apply: RemoteChange,
        may_retry: bool,
        reached: &mut BTreeSet<String>,
    ) -> Result<()> {
        let store = self.store;
        let account_id = self.account.id();

        let mut items: Vec<TaskPlacement> = Vec::new();
        for msg in inflate_messages(store, data)? {
            let placements = store.placements_for_message(msg.id())?;
            if is_move {
                for mv in moves_for_message(store, &msg, &placements, data)? {
                    items.push(TaskPlacement {
                        message_id: msg.id().to_owned(),
                        placement: mv.placement,
                        dest_folder_id: mv.dest_folder_id,
                        moved: false,
                        moved_uid: 0,
                    });
                }
            } else {
                for p in placements.into_iter().filter(|p| p.remote_uid > 0) {
                    items.push(TaskPlacement {
                        message_id: msg.id().to_owned(),
                        placement: p,
                        dest_folder_id: String::new(),
                        moved: false,
                        moved_uid: 0,
                    });
                }
            }
        }

        let mut by_folder: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, item) in items.iter().enumerate() {
            by_folder
                .entry(item.placement.folder_id.clone())
                .or_default()
                .push(i);
        }

        let mut failure: Option<Error> = None;
        for (folder_id, idxs) in &by_folder {
            let Some(folder) = store.folder_by_id(account_id, folder_id)? else {
                warn!(
                    "-- {} copies are in a folder ({folder_id}) that no longer exists, skipping",
                    idxs.len()
                );
                continue;
            };
            let mut group: Vec<TaskPlacement> = idxs.iter().map(|&i| items[i].clone()).collect();
            let result = apply(session, store, account_id, &folder, &mut group, data);
            for (item, &i) in group.into_iter().zip(idxs) {
                items[i] = item;
            }
            if !is_move && result.is_ok() {
                reached.extend(idxs.iter().map(|&i| items[i].message_id.clone()));
            }
            match result {
                Ok(()) => {}
                // Leaves the task queued (see perform_remote), so nothing is released twice.
                Err(e @ Error::Database(_)) => return Err(e),
                Err(e) => {
                    error!("{}", logmsg::changing_copies_failed(folder.path()));
                    failure = Some(e);
                    break;
                }
            }
        }

        if is_move {
            reached.extend(
                items
                    .iter()
                    .filter(|i| i.moved)
                    .map(|i| i.message_id.clone()),
            );
        }
        let failed = failure.is_some();
        let retrying = may_retry && failure.as_ref().is_some_and(Error::is_retryable);
        store.transaction("performRemoteChangeOnMessages", |s| {
            let mut client_visible_change = false;
            let mut displaced_ids = Vec::new();
            let now = s.now();
            for mut safe in inflate_messages(s, data)? {
                let before = client_visible_state(&safe);
                let rows = s.placements_for_message(safe.id())?;
                let safe_id = safe.id().to_owned();
                for item in items.iter().filter(|i| i.message_id == safe_id) {
                    if retrying && !item.moved && !item.dest_folder_id.is_empty() {
                        continue;
                    }
                    if failed && !item.moved && !item.dest_folder_id.is_empty() {
                        s.abandon_placement_move(
                            &mut safe,
                            &item.placement.folder_id,
                            item.placement.remote_uid,
                            &item.dest_folder_id,
                        )?;
                        continue;
                    }
                    if let Some(d) = confirm_placement_change(s, &mut safe, item, &rows)? {
                        displaced_ids.push(d);
                    }
                }
                if safe.placements_changed() {
                    s.refresh_message_from_placements(&mut safe)?;
                }
                if client_visible_state(&safe) != before {
                    client_visible_change = true;
                }
                if retrying {
                    s.save(&mut safe)?;
                    continue;
                }
                let suc = safe.sync_unsaved_changes() - 1;
                safe.set_sync_unsaved_changes(suc);
                if suc == 0 {
                    safe.set_synced_at(now);
                }
                s.save(&mut safe)?;
            }
            // A message that lost a stale (folder, UID) row to a moved copy has a snapshot
            // listing a copy it no longer has. Without another row it is an orphan, swept
            // at the end of the pass like any other vanished copy.
            for id in &displaced_ids {
                warn!("-- Message {id} lost a placement to a moved copy at the same UID");
                client_visible_change = true;
            }
            let processor = MailProcessor::new(&self.account, s);
            processor.refresh_messages_in_open_transaction(
                s,
                &displaced_ids,
                UnplacedMessages::Keep,
                true,
            )?;
            if !client_visible_change {
                s.unsafe_erase_transaction_deltas();
            }
            Ok(())
        })?;

        if let Some(e) = failure {
            if !is_move && !retrying {
                rescan_folders_for_flags(store, account_id, by_folder.keys())?;
            }
            return Err(e);
        }
        Ok(())
    }
}

/// Applies one moved copy's new location to the reloaded message. A row that no longer
/// matches its captured (folder, UID), reset to UID 0 by a UIDVALIDITY change while the
/// task ran, is left alone with its marker; the folder's next scan records where the copy
/// is. A marker a later task's local phase put on the row (an undo issued before this move
/// reached the server) is carried over to the new row, so the client keeps seeing the copy
/// where the later task is sending it. Returns the id of another message displaced from
/// the UID the copy claimed.
fn confirm_placement_change(
    store: &Store,
    msg: &mut Message,
    item: &TaskPlacement,
    rows: &[Placement],
) -> Result<Option<String>> {
    if !item.moved {
        return Ok(None);
    }
    let p = &item.placement;
    let Some(current) = rows
        .iter()
        .find(|r| r.folder_id == p.folder_id && r.remote_uid == p.remote_uid)
    else {
        warn!(
            "-- Message {} no longer has a placement at ({}, {}); leaving its move to be re-derived",
            msg.id(),
            p.folder_id,
            p.remote_uid
        );
        return Ok(None);
    };
    let later_pending = current.pending_folder_id.clone();
    let displaced = store.commit_placement_move(
        msg,
        &p.folder_id,
        p.remote_uid,
        &item.dest_folder_id,
        item.moved_uid,
    )?;
    if let Some(later) = later_pending.filter(|l| *l != item.dest_folder_id) {
        store.begin_placement_move(msg, &item.dest_folder_id, item.moved_uid, &later)?;
    }
    Ok(displaced)
}

/// Whether the session's server is Gmail, from the profile resolved at login.
pub(crate) fn is_gmail(session: &dyn ImapSession) -> bool {
    session.profile().mailbox_model == MailboxModelKind::GmailLabels
}
