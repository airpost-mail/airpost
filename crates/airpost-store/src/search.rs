// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Thread search and thread references: the `ThreadSearch` FTS row and the
//! `ThreadReference` rows that find a thread by Message-ID or References. Derived from
//! Mailspring-Sync `MailSync/MailProcessor.cpp` (`appendToThreadSearchContent`,
//! `upsertThreadReferences` and the thread lookup in `insertMessage`), Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md.

use airpost_core::constants::SEARCH_TEXT_LENGTH;
use airpost_core::Result;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde_json::Value;

use crate::{Message, Model, Store, Thread};

/// At most this many References are matched when looking up a thread: a rogue client
/// can put any number in the header.
const MAX_LOOKUP_REFERENCES: usize = 50;
/// References indexed per message: the first (the thread root) and the last 99.
const MAX_INDEXED_REFERENCES: usize = 100;

/// `stringByAppendingOrSkipping`: appends ` value` unless it is already there.
fn append_or_skip(input: &mut String, value: &str) {
    let with_space = format!(" {value}");
    if !input.contains(&with_space) {
        input.push_str(&with_space);
    }
}

/// The first `n` UTF-16 code units of `s`, as mailcore's `substringToIndex` cut them.
pub fn truncate_utf16(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (idx, ch) in s.char_indices() {
        units += ch.len_utf16();
        if units > n {
            return &s[..idx];
        }
    }
    s
}

impl Store {
    /// Adds a message's participants, and optionally body text (cut to 5000 UTF-16 units),
    /// to the thread's `ThreadSearch` row, creating it on first use and refreshing the
    /// categories. A new row's rowid is stored on the thread, which the caller then saves.
    pub fn append_to_thread_search_content(
        &self,
        thread: &mut Thread,
        message: Option<&Message>,
        body: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        let categories = thread.categories_search_string();
        let (mut to, mut from, mut text) = (String::new(), String::new(), String::new());
        if thread.search_row_id() != 0 {
            let existing: Option<(String, String, String)> = conn
                .prepare_cached("SELECT to_, from_, body FROM ThreadSearch WHERE rowid = ?")?
                .query_row([thread.search_row_id()], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ))
                })
                .optional()?;
            if let Some((t, f, b)) = existing {
                (to, from, text) = (t, f, b);
            }
        }

        if let Some(msg) = message {
            let add = |target: &mut String, list: &Value| {
                for c in list.as_array().into_iter().flatten() {
                    for key in ["email", "name"] {
                        if let Some(v) = c.get(key).and_then(Value::as_str) {
                            append_or_skip(target, v);
                        }
                    }
                }
            };
            add(&mut to, msg.to());
            add(&mut to, msg.cc());
            add(&mut to, msg.bcc());
            add(&mut from, msg.from());
        }
        if let Some(b) = body {
            text = format!("{text} {}", truncate_utf16(b, SEARCH_TEXT_LENGTH));
        }

        if thread.search_row_id() != 0 {
            conn.prepare_cached("UPDATE ThreadSearch SET to_ = ?, from_ = ?, body = ?, categories = ? WHERE rowid = ?")?
                .execute(params![to, from, text, categories, thread.search_row_id()])?;
        } else {
            conn.prepare_cached(
                "INSERT INTO ThreadSearch (subject, to_, from_, body, categories, content_id) VALUES (?, ?, ?, ?, ?, ?)",
            )?
            .execute(params![thread.subject(), to, from, text, categories, thread.id()])?;
            thread.set_search_row_id(conn.last_insert_rowid());
        }
        Ok(())
    }

    /// Makes the thread findable by the message's Message-ID and its References: the first
    /// reference (the thread root) and the last 99 (the most recent).
    pub fn upsert_thread_references(
        &self,
        thread_id: &str,
        account_id: &str,
        header_message_id: &str,
        references: &[String],
    ) -> Result<()> {
        let mut stmt = self
            .conn()
            .prepare_cached("INSERT OR IGNORE INTO ThreadReference (threadId, accountId, headerMessageId) VALUES (?,?,?)")?;
        stmt.execute(params![thread_id, account_id, header_message_id])?;
        let count = references.len();
        if count == 0 {
            return Ok(());
        }
        stmt.execute(params![thread_id, account_id, references[0]])?;
        let last_n = (MAX_INDEXED_REFERENCES - 1).min(count - 1);
        for r in &references[(count - last_n).max(1)..] {
            stmt.execute(params![thread_id, account_id, r])?;
        }
        Ok(())
    }

    /// The thread holding the message's own Message-ID or any of its first 50 References.
    pub fn find_thread_by_references(
        &self,
        account_id: &str,
        header_message_id: &str,
        references: &[String],
    ) -> Result<Option<Thread>> {
        let refs = &references[..references.len().min(MAX_LOOKUP_REFERENCES)];
        let sql = format!(
            "SELECT Thread.data FROM Thread INNER JOIN ThreadReference ON ThreadReference.threadId = Thread.id \
             WHERE ThreadReference.accountId = ? AND ThreadReference.headerMessageId IN ({}) LIMIT 1",
            vec!["?"; 1 + refs.len()].join(",")
        );
        let mut binds = vec![
            SqlValue::Text(account_id.to_owned()),
            SqlValue::Text(header_message_id.to_owned()),
        ];
        binds.extend(refs.iter().map(|r| SqlValue::Text(r.clone())));
        let data: Option<String> = self
            .conn()
            .prepare_cached(&sql)?
            .query_row(params_from_iter(binds), |r| r.get(0))
            .optional()?;
        match data {
            Some(d) => Ok(Some(Thread::from_json(serde_json::from_str(&d)?)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_by_utf16_units() {
        assert_eq!(truncate_utf16("abc", 2), "ab");
        assert_eq!(truncate_utf16("a😀b", 2), "a");
        assert_eq!(truncate_utf16("a😀b", 3), "a😀");
        assert_eq!(truncate_utf16("abc", 10), "abc");
    }

    #[test]
    fn append_skips_present_values() {
        let mut s = String::new();
        append_or_skip(&mut s, "a@b");
        append_or_skip(&mut s, "a@b");
        append_or_skip(&mut s, "Ann");
        assert_eq!(s, " a@b Ann");
    }
}
