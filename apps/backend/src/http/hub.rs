// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! In-memory stream fan-out (docs/backend-plan.md 3.2, 3.5): a `watch` per `(user, aid)`
//! carrying the latest seq written, plus a close counter for account removal. An entry exists
//! only while a stream holds it.
//!
//! Slots are counted per `(user, aid)`, per user and per token. A token holds at most one
//! stream per account: a new stream from the same token replaces the old one, which is the
//! same device reconnecting after its old connection died without the server noticing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::{oneshot, watch};

type Key = (String, String);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Signal {
    /// Largest seq committed for the account since the entry was created.
    pub seq: i64,
    /// Bumped to close every open stream of the account.
    pub closes: u64,
}

/// Open stream limits, checked when a slot is taken.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub per_account: usize,
    pub per_user: usize,
    pub per_token: usize,
}

struct Slot {
    id: u64,
    token_id: i64,
    /// Dropped to tell the stream it was replaced.
    _evict: oneshot::Sender<()>,
}

struct Entry {
    tx: watch::Sender<Signal>,
    slots: Vec<Slot>,
}

#[derive(Default)]
struct State {
    accounts: HashMap<Key, Entry>,
    per_user: HashMap<String, usize>,
    per_token: HashMap<i64, usize>,
    next_slot: u64,
}

impl State {
    fn release(&mut self, user_id: &str, token_id: i64) {
        decrement(&mut self.per_user, user_id);
        decrement(&mut self.per_token, &token_id);
    }
}

fn decrement<K, Q>(map: &mut HashMap<K, usize>, key: &Q)
where
    K: std::borrow::Borrow<Q> + std::hash::Hash + Eq,
    Q: std::hash::Hash + Eq + ?Sized,
{
    if let Some(n) = map.get_mut(key) {
        *n -= 1;
        if *n == 0 {
            map.remove(key);
        }
    }
}

#[derive(Default)]
pub struct Hub {
    state: Mutex<State>,
}

/// One open stream's slot. Dropping it frees the slot.
pub struct Subscription {
    hub: Arc<Hub>,
    key: Key,
    slot: u64,
    token_id: i64,
    pub rx: watch::Receiver<Signal>,
    /// `Signal::closes` when the slot was taken; any other value means the account was removed.
    pub closes: u64,
    /// Resolves when a newer stream of the same token replaced this one.
    pub evicted: oneshot::Receiver<()>,
}

impl Hub {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes a stream slot for `(user, aid)` and `token_id`, replacing that token's existing
    /// stream of the account. `None` when a limit is reached.
    pub fn subscribe(
        self: &Arc<Self>,
        user_id: &str,
        aid: &str,
        token_id: i64,
        limits: Limits,
    ) -> Option<Subscription> {
        let key = (user_id.to_owned(), aid.to_owned());
        let mut guard = self.lock();
        let state = &mut *guard;
        let entry = state.accounts.entry(key.clone()).or_insert_with(|| Entry {
            tx: watch::channel(Signal::default()).0,
            slots: Vec::new(),
        });
        let before = entry.slots.len();
        entry.slots.retain(|s| s.token_id != token_id);
        let replaced = before - entry.slots.len();
        let full = entry.slots.len() >= limits.per_account;
        let rx = entry.tx.subscribe();
        for _ in 0..replaced {
            state.release(user_id, token_id);
        }
        // Replacing a stream frees one slot of every kind, so it never fails below.
        if full
            || state.per_user.get(user_id).copied().unwrap_or(0) >= limits.per_user
            || state.per_token.get(&token_id).copied().unwrap_or(0) >= limits.per_token
        {
            if state.accounts.get(&key).is_some_and(|e| e.slots.is_empty()) {
                state.accounts.remove(&key);
            }
            return None;
        }
        let slot = state.next_slot;
        state.next_slot += 1;
        let (evict_tx, evicted) = oneshot::channel();
        state
            .accounts
            .get_mut(&key)
            .expect("entry created above")
            .slots
            .push(Slot {
                id: slot,
                token_id,
                _evict: evict_tx,
            });
        *state.per_user.entry(user_id.to_owned()).or_default() += 1;
        *state.per_token.entry(token_id).or_default() += 1;
        let closes = rx.borrow().closes;
        Some(Subscription {
            hub: self.clone(),
            key,
            slot,
            token_id,
            rx,
            closes,
            evicted,
        })
    }

    /// Wakes the account's streams after a commit. Notifications can arrive out of seq order;
    /// the watch keeps the largest.
    pub fn notify(&self, user_id: &str, aid: &str, seq: i64) {
        if let Some(e) = self
            .lock()
            .accounts
            .get(&(user_id.to_owned(), aid.to_owned()))
        {
            e.tx.send_if_modified(|s| {
                let newer = seq > s.seq;
                s.seq = s.seq.max(seq);
                newer
            });
        }
    }

    /// Ends every open stream of the account.
    pub fn close_account(&self, user_id: &str, aid: &str) {
        if let Some(e) = self
            .lock()
            .accounts
            .get(&(user_id.to_owned(), aid.to_owned()))
        {
            e.tx.send_modify(|s| s.closes += 1);
        }
    }

    pub fn open_streams(&self, user_id: &str, aid: &str) -> usize {
        self.lock()
            .accounts
            .get(&(user_id.to_owned(), aid.to_owned()))
            .map_or(0, |e| e.slots.len())
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut state = self.hub.lock();
        let Some(e) = state.accounts.get_mut(&self.key) else {
            return;
        };
        let before = e.slots.len();
        e.slots.retain(|s| s.id != self.slot);
        let removed = before != e.slots.len();
        if e.slots.is_empty() {
            state.accounts.remove(&self.key);
        }
        // An evicted slot was already released by the stream that replaced it.
        if removed {
            state.release(&self.key.0, self.token_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Limits = Limits {
        per_account: 2,
        per_user: 3,
        per_token: 2,
    };

    fn is_empty(hub: &Hub) -> bool {
        let s = hub.lock();
        s.accounts.is_empty() && s.per_user.is_empty() && s.per_token.is_empty()
    }

    #[test]
    fn slots_and_signals() {
        let hub = Arc::new(Hub::default());
        hub.notify("u", "a", 5); // nobody listening: no entry is created
        assert_eq!(hub.open_streams("u", "a"), 0);

        let mut s1 = hub.subscribe("u", "a", 1, L).unwrap();
        let s2 = hub.subscribe("u", "a", 2, L).unwrap();
        assert!(hub.subscribe("u", "a", 3, L).is_none());
        let other = hub.subscribe("u", "b", 3, L).unwrap();
        assert_eq!(hub.open_streams("u", "a"), 2);

        hub.notify("u", "a", 7);
        hub.notify("u", "a", 6);
        assert!(s1.rx.has_changed().unwrap());
        assert_eq!(s1.rx.borrow_and_update().seq, 7);
        hub.notify("u", "a", 7);
        assert!(!s1.rx.has_changed().unwrap(), "not newer");
        assert!(!other.rx.has_changed().unwrap());

        hub.close_account("u", "a");
        assert_eq!(s1.rx.borrow_and_update().closes, 1);

        drop(s2);
        assert_eq!(hub.open_streams("u", "a"), 1);
        let s3 = hub.subscribe("u", "a", 2, L).unwrap();
        drop(s1);
        drop(s3);
        assert_eq!(hub.open_streams("u", "a"), 0);
        assert!(!hub.lock().accounts.contains_key(&("u".into(), "a".into())));
        drop(other);
        assert!(is_empty(&hub));
    }

    #[test]
    fn same_token_replaces_its_stream() {
        let hub = Arc::new(Hub::default());
        let mut s1 = hub.subscribe("u", "a", 1, L).unwrap();
        let s2 = hub.subscribe("u", "a", 2, L).unwrap();
        // Full for a third device, but token 1 reconnecting replaces its own stream.
        assert!(hub.subscribe("u", "a", 3, L).is_none());
        let mut s1b = hub.subscribe("u", "a", 1, L).unwrap();
        assert_eq!(
            s1.evicted.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        );
        assert_eq!(
            s1b.evicted.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        assert_eq!(hub.open_streams("u", "a"), 2);
        // Dropping the replaced stream does not free the new one's slot.
        drop(s1);
        assert_eq!(hub.open_streams("u", "a"), 2);
        assert!(hub.subscribe("u", "a", 3, L).is_none());
        drop((s1b, s2));
        assert!(is_empty(&hub));
    }

    #[test]
    fn per_user_and_per_token_limits() {
        let hub = Arc::new(Hub::default());
        let a = hub.subscribe("u", "a", 1, L).unwrap();
        let b = hub.subscribe("u", "b", 1, L).unwrap();
        assert!(hub.subscribe("u", "c", 1, L).is_none(), "per token");
        let c = hub.subscribe("u", "c", 2, L).unwrap();
        assert!(hub.subscribe("u", "d", 3, L).is_none(), "per user");
        assert!(!hub.lock().accounts.contains_key(&("u".into(), "d".into())));
        // Replacing still works at the user limit.
        let b2 = hub.subscribe("u", "b", 1, L).unwrap();
        // Another user has its own budget; token ids are global.
        let v = hub.subscribe("v", "a", 9, L).unwrap();
        drop((a, b, c, b2, v));
        assert!(is_empty(&hub));
    }
}
