// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! UID sets: mailcore's `IndexSet` as the workers use it (mailcore2 `MCIndexSet.cpp`,
//! Copyright © 2001-2013 MailCore team, BSD-3-Clause; see NOTICE.md ("mailcore2")).
//!
//! A set is a sorted list of disjoint, non-adjacent ranges. A range may be open-ended
//! (`12:*`, as VANISHED and `1:*` fetches produce), so a set is not always enumerable;
//! [`UidSet::iter`] walks only the bounded part and [`UidSet::open_from`] reports the rest.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UidRange {
    pub start: u32,
    /// Inclusive upper bound; `None` is `*`.
    pub end: Option<u32>,
}

impl UidRange {
    pub fn contains(&self, uid: u32) -> bool {
        uid >= self.start && self.end.is_none_or(|e| uid <= e)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UidSet {
    ranges: Vec<UidRange>,
}

impl UidSet {
    pub fn new() -> Self {
        UidSet::default()
    }

    pub fn single(uid: u32) -> Self {
        Self::range(uid, Some(uid))
    }

    /// `start:end`, or `start:*` for `None`. Bounds are swapped if reversed.
    pub fn range(start: u32, end: Option<u32>) -> Self {
        let mut s = UidSet::new();
        s.insert_range(start, end);
        s
    }

    pub fn from_uids<I: IntoIterator<Item = u32>>(uids: I) -> Self {
        let mut v: Vec<u32> = uids.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        let mut s = UidSet::new();
        for uid in v {
            s.insert(uid);
        }
        s
    }

    pub fn insert(&mut self, uid: u32) {
        self.insert_range(uid, Some(uid));
    }

    pub fn insert_range(&mut self, start: u32, end: Option<u32>) {
        let (start, end) = match end {
            Some(e) if e < start => (e, Some(start)),
            _ => (start, end),
        };
        self.ranges.push(UidRange { start, end });
        self.normalize();
    }

    pub fn extend(&mut self, other: &UidSet) {
        self.ranges.extend_from_slice(&other.ranges);
        self.normalize();
    }

    fn normalize(&mut self) {
        self.ranges.sort_by_key(|r| r.start);
        let mut merged: Vec<UidRange> = Vec::with_capacity(self.ranges.len());
        for r in self.ranges.drain(..) {
            if let Some(last) = merged.last_mut() {
                let touches = match last.end {
                    None => true,
                    Some(e) => r.start <= e.saturating_add(1),
                };
                if touches {
                    last.end = match (last.end, r.end) {
                        (None, _) | (_, None) => None,
                        (Some(a), Some(b)) => Some(a.max(b)),
                    };
                    continue;
                }
            }
            merged.push(r);
        }
        self.ranges = merged;
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn ranges(&self) -> &[UidRange] {
        &self.ranges
    }

    pub fn contains(&self, uid: u32) -> bool {
        self.ranges.iter().any(|r| r.contains(uid))
    }

    /// Number of UIDs, or `None` when the set is open-ended.
    pub fn len(&self) -> Option<u64> {
        self.ranges.iter().try_fold(0u64, |acc, r| {
            r.end.map(|e| acc + (e as u64 - r.start as u64 + 1))
        })
    }

    pub fn min(&self) -> Option<u32> {
        self.ranges.first().map(|r| r.start)
    }

    /// The start of the open-ended range, if the set has one.
    pub fn open_from(&self) -> Option<u32> {
        self.ranges
            .last()
            .filter(|r| r.end.is_none())
            .map(|r| r.start)
    }

    /// The UIDs of the bounded ranges, ascending.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.ranges
            .iter()
            .filter_map(|r| r.end.map(|e| r.start..=e))
            .flatten()
    }

    /// Parses an IMAP sequence set such as `1:5,7,9:*`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut set = UidSet::new();
        for part in s.trim().split(',') {
            let (a, b) = match part.split_once(':') {
                Some((a, b)) => (a, Some(b)),
                None => (part, None),
            };
            let num = |x: &str| -> Option<Option<u32>> {
                if x == "*" {
                    Some(None)
                } else {
                    x.parse::<u32>().ok().map(Some)
                }
            };
            let start = num(a)?;
            let end = match b {
                Some(b) => Some(num(b)?),
                None => None,
            };
            match (start, end) {
                (Some(s), None) => set.insert(s),
                (Some(s), Some(Some(e))) => set.insert_range(s, Some(e)),
                (Some(s), Some(None)) => set.insert_range(s, None),
                // `*:n` is `n:*`; a lone `*` is the highest UID, unknown here.
                (None, Some(Some(e))) => set.insert_range(e, None),
                (None, _) => set.insert_range(u32::MAX, None),
            }
        }
        Some(set)
    }
}

impl fmt::Display for UidSet {
    /// The IMAP sequence-set form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, r) in self.ranges.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            match r.end {
                Some(e) if e == r.start => write!(f, "{}", r.start)?,
                Some(e) => write!(f, "{}:{}", r.start, e)?,
                None => write!(f, "{}:*", r.start)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_and_format() {
        let s = UidSet::from_uids([5, 1, 2, 3, 9, 10]);
        assert_eq!(s.to_string(), "1:3,5,9:10");
        assert_eq!(s.len(), Some(6));
        let mut o = s.clone();
        o.insert_range(8, None);
        assert_eq!(o.to_string(), "1:3,5,8:*");
        assert_eq!(o.len(), None);
        assert_eq!(o.open_from(), Some(8));
        assert!(o.contains(4_000_000));
        assert_eq!(o.iter().collect::<Vec<_>>(), vec![1, 2, 3, 5]);
    }

    #[test]
    fn parsing() {
        let s = UidSet::parse("1:5,7,9:*").unwrap();
        assert_eq!(s.to_string(), "1:5,7,9:*");
        assert_eq!(UidSet::parse("5:3").unwrap().to_string(), "3:5");
        assert!(UidSet::parse("a:b").is_none());
        assert!(UidSet::new().is_empty());
    }
}
