// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! An in-memory cache with a per-entry TTL and an entry bound, keyed by a SHA-256 of the
//! provider, endpoint, key and normalised request. Neither keys nor request text are held
//! in clear.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

pub(crate) type CacheKey = [u8; 32];

/// Builds a [`CacheKey`] from parts. Each part is length-prefixed, so `("ab", "c")` and
/// `("a", "bc")` differ.
pub(crate) struct KeyBuilder(Sha256);

impl KeyBuilder {
    pub fn new(kind: &str) -> Self {
        KeyBuilder(Sha256::new()).part(kind)
    }

    pub fn part(mut self, value: &str) -> Self {
        self.0.update((value.len() as u64).to_le_bytes());
        self.0.update(value.as_bytes());
        self
    }

    pub fn finish(self) -> CacheKey {
        self.0.finalize().into()
    }
}

struct Entry<V> {
    expires: Instant,
    inserted: u64,
    value: V,
}

struct Inner<V> {
    entries: HashMap<CacheKey, Entry<V>>,
    next: u64,
}

pub(crate) struct TtlCache<V> {
    inner: Mutex<Inner<V>>,
    max_entries: usize,
}

impl<V: Clone> TtlCache<V> {
    pub fn new(max_entries: usize) -> Self {
        TtlCache {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                next: 0,
            }),
            max_entries,
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<V> {
        let mut inner = self.lock();
        let entry = inner.entries.get(key)?;
        if entry.expires <= Instant::now() {
            inner.entries.remove(key);
            return None;
        }
        Some(entry.value.clone())
    }

    /// When full, drops expired entries and then the oldest insertion.
    pub fn insert(&self, key: CacheKey, value: V, ttl: Duration) {
        if self.max_entries == 0 || ttl.is_zero() {
            return;
        }
        let now = Instant::now();
        let mut inner = self.lock();
        if inner.entries.len() >= self.max_entries && !inner.entries.contains_key(&key) {
            inner.entries.retain(|_, e| e.expires > now);
            if inner.entries.len() >= self.max_entries {
                let oldest = inner
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.inserted)
                    .map(|(k, _)| *k);
                if let Some(oldest) = oldest {
                    inner.entries.remove(&oldest);
                }
            }
        }
        let inserted = inner.next;
        inner.next += 1;
        inner.entries.insert(
            key,
            Entry {
                expires: now + ttl,
                inserted,
                value,
            },
        );
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn clear(&self) {
        self.lock().entries.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<V>> {
        // A panic while holding the lock leaves the map consistent (every mutation is a
        // single HashMap call), so a poisoned lock is safe to reuse.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> CacheKey {
        KeyBuilder::new("t").part(s).finish()
    }

    #[test]
    fn parts_are_delimited() {
        let a = KeyBuilder::new("k").part("ab").part("c").finish();
        let b = KeyBuilder::new("k").part("a").part("bc").finish();
        assert_ne!(a, b);
    }

    #[test]
    fn expiry() {
        let cache = TtlCache::new(10);
        cache.insert(key("a"), 1, Duration::from_millis(30));
        assert_eq!(cache.get(&key("a")), Some(1));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(cache.get(&key("a")), None);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn bound_evicts_oldest() {
        let cache = TtlCache::new(2);
        let hour = Duration::from_secs(3600);
        cache.insert(key("a"), 1, hour);
        cache.insert(key("b"), 2, hour);
        cache.insert(key("c"), 3, hour);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&key("a")), None);
        assert_eq!(cache.get(&key("b")), Some(2));
        assert_eq!(cache.get(&key("c")), Some(3));
        // Replacing an existing key doesn't evict.
        cache.insert(key("b"), 20, hour);
        assert_eq!(cache.get(&key("c")), Some(3));
        assert_eq!(cache.get(&key("b")), Some(20));
    }

    #[test]
    fn bound_prefers_expired() {
        let cache = TtlCache::new(2);
        cache.insert(key("old"), 1, Duration::from_secs(3600));
        cache.insert(key("short"), 2, Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        cache.insert(key("new"), 3, Duration::from_secs(3600));
        assert_eq!(cache.get(&key("old")), Some(1));
        assert_eq!(cache.get(&key("new")), Some(3));
    }
}
