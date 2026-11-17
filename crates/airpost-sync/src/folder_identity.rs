// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Folder and label identity across renames. Not in C++.
//!
//! A folder's id is the hash of the path it was first seen at and stays with the folder
//! when it is renamed, so messages, threads, rule targets and settings keep pointing at it.
//! The id is then no longer the hash of its path: rows are matched to the listing by path,
//! and a new folder created at a path whose hash a renamed folder holds gets another id
//! ([`airpost_core::ids::free_folder_id`]).
//!
//! Folder sync pairs a row the listing no longer shows with a newly listed path when a
//! rename `SyncbackCategoryTask` recorded that move ([`record_rename`]) or, failing that,
//! when they share a UIDVALIDITY ([`uidvalidity_candidates`]) and, for folders, the newest
//! messages stored for the row are at the same UIDs under the new path.

use std::collections::{BTreeSet, HashMap, HashSet};

use airpost_core::constants::FOLDER_RENAME_RECORD_TTL;
use airpost_core::Result;
use airpost_store::{Folder, Label, Message, Model, Query, Store, Thread};
use serde_json::{json, Value};

/// Messages and threads rewritten per transaction after a rename.
const REFRESH_CHUNK: usize = 100;

/// A rename the client asked for: `from` became `to`, at `at` (epoch seconds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedRename {
    pub from: String,
    pub to: String,
    pub at: i64,
}

fn state_key(account_id: &str) -> String {
    format!("folderRenames-{account_id}")
}

fn read_records(store: &Store, account_id: &str) -> Result<Vec<RecordedRename>> {
    let Some(text) = store.get_key_value(&state_key(account_id))? else {
        return Ok(Vec::new());
    };
    let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
    Ok(parsed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(RecordedRename {
                from: r["from"].as_str()?.to_owned(),
                to: r["to"].as_str()?.to_owned(),
                at: r["at"].as_i64()?,
            })
        })
        .collect())
}

fn write_records(
    store: &Store,
    account_id: &str,
    mut records: Vec<RecordedRename>,
    now: i64,
) -> Result<()> {
    records.retain(|r| now - r.at < FOLDER_RENAME_RECORD_TTL);
    let value: Vec<Value> = records
        .iter()
        .map(|r| json!({"from": r.from, "to": r.to, "at": r.at}))
        .collect();
    store.save_key_value(&state_key(account_id), &Value::from(value).to_string())
}

/// Records that `from` is being renamed to `to`. Written before the RENAME goes out, so a
/// folder listing that sees the new path before the task saves the row can pair them.
pub fn record_rename(store: &Store, account_id: &str, from: &str, to: &str) -> Result<()> {
    store.transaction("recordFolderRename", |s| {
        let now = s.now();
        let mut records = read_records(s, account_id)?;
        records.retain(|r| !(r.from == from && r.to == to));
        records.push(RecordedRename {
            from: from.to_owned(),
            to: to.to_owned(),
            at: now,
        });
        write_records(s, account_id, records, now)
    })
}

/// Drops the record of a rename the server refused.
pub fn forget_rename(store: &Store, account_id: &str, from: &str, to: &str) -> Result<()> {
    store.transaction("forgetFolderRename", |s| {
        let mut records = read_records(s, account_id)?;
        records.retain(|r| !(r.from == from && r.to == to));
        write_records(s, account_id, records, s.now())
    })
}

/// The renames recorded within [`FOLDER_RENAME_RECORD_TTL`].
pub fn recorded_renames(store: &Store, account_id: &str) -> Result<Vec<RecordedRename>> {
    let now = store.now();
    let mut records = read_records(store, account_id)?;
    records.retain(|r| now - r.at < FOLDER_RENAME_RECORD_TTL);
    Ok(records)
}

/// Where `path` is after `from` was renamed to `to`: `to` for `from` itself, and the same
/// suffix under `to` for a folder inside it (servers rename children with their parent).
pub fn renamed_path(path: &str, from: &str, to: &str, delimiter: Option<char>) -> Option<String> {
    if path == from {
        return Some(to.to_owned());
    }
    let d = delimiter?;
    let rest = path.strip_prefix(from)?.strip_prefix(d)?;
    Some(format!("{to}{d}{rest}"))
}

/// Pairs vanished rows (key, stored path) with newly listed paths (key, path, delimiter)
/// that a recorded rename links, in either direction: a listing taken just before the
/// RENAME still shows the old path while the task has already saved the new one. A pair
/// is made only when each side has exactly one candidate.
pub fn pair_by_records<V: Clone + Eq + std::hash::Hash, L: Clone + Eq + std::hash::Hash>(
    vanished: &[(V, String)],
    listed: &[(L, String, Option<char>)],
    records: &[RecordedRename],
) -> Vec<(V, L)> {
    if records.is_empty() {
        return Vec::new();
    }
    let linked = |old: &str, new: &str, d: Option<char>| {
        records.iter().any(|r| {
            renamed_path(old, &r.from, &r.to, d).as_deref() == Some(new)
                || renamed_path(old, &r.to, &r.from, d).as_deref() == Some(new)
        })
    };
    let mut candidates: Vec<(V, L)> = Vec::new();
    for (lk, lpath, d) in listed {
        for (vk, vpath) in vanished {
            if linked(vpath, lpath, *d) {
                candidates.push((vk.clone(), lk.clone()));
            }
        }
    }
    unique_pairs(candidates)
}

/// What folder sync knows of one side of a UIDVALIDITY pairing: the stored values of a
/// vanished row, or the STATUS of a newly listed path. 0 is unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Validity {
    pub uid_validity: u32,
    pub uid_next: u32,
}

/// The (vanished, listed) pairs that share a UIDVALIDITY. A listed UIDNEXT below the
/// stored one is a different mailbox: UIDs never go backwards. Some servers give every
/// folder the same UIDVALIDITY, or one per second of creation, so a value shared by several
/// candidates proves nothing alone ([`unique_pairs`]).
pub fn uidvalidity_candidates<V: Clone, L: Clone>(
    vanished: &[(V, Validity)],
    listed: &[(L, Validity)],
) -> Vec<(V, L)> {
    let mut candidates = Vec::new();
    for (lk, l) in listed {
        if l.uid_validity == 0 {
            continue;
        }
        for (vk, v) in vanished {
            let uids_forward = v.uid_next == 0 || l.uid_next == 0 || l.uid_next >= v.uid_next;
            if v.uid_validity == l.uid_validity && uids_forward {
                candidates.push((vk.clone(), lk.clone()));
            }
        }
    }
    candidates
}

/// The candidate pairs whose two sides appear in no other candidate.
pub fn unique_pairs<V: Clone + Eq + std::hash::Hash, L: Clone + Eq + std::hash::Hash>(
    candidates: Vec<(V, L)>,
) -> Vec<(V, L)> {
    let mut by_v: HashMap<&V, usize> = HashMap::new();
    let mut by_l: HashMap<&L, usize> = HashMap::new();
    for (v, l) in &candidates {
        *by_v.entry(v).or_default() += 1;
        *by_l.entry(l).or_default() += 1;
    }
    candidates
        .iter()
        .filter(|(v, l)| by_v[v] == 1 && by_l[l] == 1)
        .cloned()
        .collect()
}

fn refresh_key(account_id: &str) -> String {
    format!("folderRenameRefresh-{account_id}")
}

fn read_refreshes(store: &Store, account_id: &str) -> Result<Vec<(String, String)>> {
    let Some(text) = store.get_key_value(&refresh_key(account_id))? else {
        return Ok(Vec::new());
    };
    let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
    Ok(parsed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some((
                r["id"].as_str()?.to_owned(),
                r["oldPath"].as_str()?.to_owned(),
            ))
        })
        .collect())
}

fn write_refreshes(store: &Store, account_id: &str, list: &[(String, String)]) -> Result<()> {
    let value: Vec<Value> = list
        .iter()
        .map(|(id, old)| json!({"id": id, "oldPath": old}))
        .collect();
    store.save_key_value(&refresh_key(account_id), &Value::from(value).to_string())
}

/// Notes, in the transaction that gives the Folder or Label `id` a new path, that what
/// embeds it still shows `old_path`. Kept until [`run_pending_refreshes`] has caught up, so
/// a quit in between leaves the work for the next pass.
pub fn queue_refresh(store: &Store, account_id: &str, id: &str, old_path: &str) -> Result<()> {
    let mut list = read_refreshes(store, account_id)?;
    if !list.iter().any(|(i, p)| i == id && p == old_path) {
        list.push((id.to_owned(), old_path.to_owned()));
    }
    write_refreshes(store, account_id, &list)
}

/// Brings what embeds each folder or label [`queue_refresh`] noted up to date with its row,
/// then forgets the note.
pub fn run_pending_refreshes(store: &Store, account_id: &str) -> Result<()> {
    for (id, old_path) in read_refreshes(store, account_id)? {
        let q = Query::new().equal("id", &id);
        let row = match store.find::<Label>(&q)? {
            Some(l) => Some((l.to_json(), true)),
            None => store.find::<Folder>(&q)?.map(|f| (f.to_json(), false)),
        };
        if let Some((json, is_label)) = row {
            refresh_after_rename(store, account_id, &json, &old_path, is_label)?;
        }
        store.transaction("forgetFolderRenameRefresh", |s| {
            let mut list = read_refreshes(s, account_id)?;
            list.retain(|(i, p)| !(*i == id && *p == old_path));
            write_refreshes(s, account_id, &list)
        })?;
    }
    Ok(())
}

/// Brings what embeds a renamed folder or label up to date with its row (`category`, the
/// saved JSON): on Gmail, the X-GM-LABELS name `old_path` on its messages' copies, then
/// recounting their threads; for both, the entry threads keep for it. Chunked, so a large
/// folder doesn't hold the write lock for long; each step can run again.
fn refresh_after_rename(
    store: &Store,
    account_id: &str,
    category: &Value,
    old_path: &str,
    is_label: bool,
) -> Result<()> {
    let new_path = category["path"].as_str().unwrap_or("");
    if is_label && old_path != new_path {
        // Messages Gmail already re-sent with the new name were counted against a label
        // set that couldn't resolve it, so they are recounted too.
        let mut message_ids = store.message_ids_with_label(account_id, old_path)?;
        let mut seen: HashSet<String> = message_ids.iter().cloned().collect();
        for id in store.message_ids_with_label(account_id, new_path)? {
            if seen.insert(id.clone()) {
                message_ids.push(id);
            }
        }
        for chunk in message_ids.chunks(REFRESH_CHUNK) {
            store.transaction("relabelRenamedLabel", |s| {
                s.rename_label_in_placements(chunk, old_path, new_path)?;
                let mut threads = BTreeSet::new();
                for mut m in s.find_all::<Message>(&Query::new().equal_any("id", chunk.iter()))? {
                    m.set_placements_changed(true);
                    // The recount below rebuilds the thread; a diff against the snapshot
                    // can't resolve the old name any more.
                    m.skip_thread_updates_after_save = true;
                    s.save(&mut m)?;
                    if !m.thread_id().is_empty() {
                        threads.insert(m.thread_id().to_owned());
                    }
                }
                Thread::recount_from_messages(s, &threads.into_iter().collect::<Vec<_>>())
            })?;
        }
    }
    let id = category["id"].as_str().unwrap_or("");
    let thread_ids = Thread::ids_in_category(store, id)?;
    for chunk in thread_ids.chunks(REFRESH_CHUNK) {
        store.transaction("refreshRenamedCategory", |s| {
            for mut t in s.find_all::<Thread>(&Query::new().equal_any("id", chunk.iter()))? {
                if t.refresh_category_entry(category) {
                    s.save(&mut t)?;
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(from: &str, to: &str) -> RecordedRename {
        RecordedRename {
            from: from.into(),
            to: to.into(),
            at: 0,
        }
    }

    #[test]
    fn renamed_paths_follow_the_parent() {
        assert_eq!(renamed_path("A", "A", "B", Some('/')).as_deref(), Some("B"));
        assert_eq!(
            renamed_path("A/x/y", "A", "B", Some('/')).as_deref(),
            Some("B/x/y")
        );
        assert_eq!(renamed_path("AB", "A", "B", Some('/')), None);
        assert_eq!(renamed_path("A/x", "A", "B", None), None);
        assert_eq!(
            renamed_path("A.x", "A", "C.A", Some('.')).as_deref(),
            Some("C.A.x")
        );
    }

    #[test]
    fn records_pair_both_directions_and_children() {
        let vanished = vec![
            (1, "A".to_owned()),
            (2, "A/x".to_owned()),
            (3, "Z".to_owned()),
        ];
        let listed = vec![
            (10, "B".to_owned(), Some('/')),
            (11, "B/x".to_owned(), Some('/')),
            (12, "Y".to_owned(), Some('/')),
        ];
        let pairs = pair_by_records(&vanished, &listed, &[rec("A", "B")]);
        assert_eq!(pairs, vec![(1, 10), (2, 11)]);
        // The listing predates the RENAME: the row already has the new path.
        let pairs = pair_by_records(
            &[(1, "B".to_owned())],
            &[(10, "A".to_owned(), Some('/'))],
            &[rec("A", "B")],
        );
        assert_eq!(pairs, vec![(1, 10)]);
        assert!(pair_by_records(&vanished, &listed, &[]).is_empty());
    }

    #[test]
    fn records_pair_nothing_ambiguous() {
        // A renamed to B, then B renamed to C before the listing: two candidates for C.
        let vanished = vec![(1, "A".to_owned()), (2, "B".to_owned())];
        let listed = vec![(10, "C".to_owned(), Some('/'))];
        let pairs = pair_by_records(&vanished, &listed, &[rec("A", "C"), rec("B", "C")]);
        assert!(pairs.is_empty());
    }

    fn v(uid_validity: u32, uid_next: u32) -> Validity {
        Validity {
            uid_validity,
            uid_next,
        }
    }

    #[test]
    fn uidvalidity_pairs_unique_values_only() {
        let vanished = vec![
            (1, v(100, 50)),
            (2, v(7, 10)),
            (3, v(7, 12)),
            (4, v(300, 9)),
        ];
        let listed = vec![
            (10, v(100, 52)),
            (11, v(7, 12)),
            (12, v(300, 2)),
            (13, v(0, 0)),
        ];
        let candidates = uidvalidity_candidates(&vanished, &listed);
        // 300's UIDNEXT went backwards.
        assert_eq!(candidates, vec![(1, 10), (2, 11), (3, 11)]);
        // 7 is shared by two vanished rows.
        assert_eq!(unique_pairs(candidates), vec![(1, 10)]);
        // Unknown UIDNEXT on either side doesn't block a pair.
        assert_eq!(
            uidvalidity_candidates(&[(1, v(5, 0))], &[(10, v(5, 3))]),
            vec![(1, 10)]
        );
        assert!(uidvalidity_candidates(&[(1, v(0, 0))], &[(10, v(0, 0))]).is_empty());
    }
}
