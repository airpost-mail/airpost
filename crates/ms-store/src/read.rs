// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Read-only queries behind the engine's read API (docs/engine-protocol-plan.md section 3,
//! docs/desktop-client-plan.md 3.2).
//!
//! A [`ReadStore`] is its own read-only connection, so a socket connection's reader thread
//! can answer reads without going through the main thread or taking the write lock. Every
//! query is scoped to the accounts it names and returns the stored model JSON (the `data`
//! column) the desktop already parses; Message JSON also carries `hidden`
//! ([`crate::models::is_hidden_json`]).

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use ms_core::constants::{DATABASE_FILE, SQLITE_BUSY_TIMEOUT_MS};
use ms_core::Result;
use rusqlite::types::Value as SqlValue;
use rusqlite::types::ValueRef;
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension, Row};
use serde_json::Value;

use crate::models::is_hidden_json;

/// Rows a read can return once, whatever the caller asks for.
pub const MAX_LIMIT: usize = 500;

/// A category's `ThreadCounts` row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub unread: i64,
    pub total: i64,
}

/// A folder or label and its counts, `None` when it has no `ThreadCounts` row.
#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub id: String,
    pub data: Value,
    pub counts: Option<Counts>,
}

/// Where `threads.list` pages from: newest first by `(lastMessageTimestamp, id)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub timestamp: i64,
    pub id: String,
}

impl Cursor {
    /// The cursor's wire form. Clients treat it as opaque.
    pub fn encode(&self) -> String {
        format!("{}:{}", self.timestamp, self.id)
    }

    pub fn decode(s: &str) -> Option<Cursor> {
        let (ts, id) = s.split_once(':')?;
        Some(Cursor {
            timestamp: ts.parse().ok()?,
            id: id.to_owned(),
        })
    }
}

/// One page of threads and the cursor of the next, `None` on the last page.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadPage {
    pub threads: Vec<Value>,
    pub next: Option<Cursor>,
}

/// The category `threads.list` filters on and its role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InCategory<'a> {
    pub id: &'a str,
    pub role: &'a str,
}

/// Which table a category lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CategoryTable {
    Folder,
    Label,
}

impl CategoryTable {
    fn name(self) -> &'static str {
        match self {
            CategoryTable::Folder => "Folder",
            CategoryTable::Label => "Label",
        }
    }
}

pub struct ReadStore {
    conn: Connection,
}

/// The stored JSON in column `idx`; `data` is TEXT on some tables and BLOB on others. A row
/// that does not parse reads as null rather than failing the whole page.
fn json_column(row: &Row<'_>, idx: usize) -> rusqlite::Result<Value> {
    Ok(match row.get_ref(idx)? {
        ValueRef::Text(b) | ValueRef::Blob(b) => serde_json::from_slice(b).unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

fn clamp(limit: usize) -> i64 {
    limit.clamp(1, MAX_LIMIT) as i64
}

/// A Message's stored JSON with `hidden` set.
pub fn with_hidden(mut message: Value) -> Value {
    let hidden = is_hidden_json(&message);
    if let Some(obj) = message.as_object_mut() {
        obj.insert("hidden".into(), Value::Bool(hidden));
    }
    message
}

fn message_column(row: &Row<'_>, idx: usize) -> rusqlite::Result<Value> {
    json_column(row, idx).map(with_hidden)
}

/// `?,?,…` for `n` parameters.
fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// What `tasks.list` keeps.
#[derive(Clone, Debug, Default)]
pub struct TaskFilter<'a> {
    pub accounts: &'a [String],
    /// `None` keeps every status.
    pub statuses: Option<&'a [String]>,
    /// The task's `__cls`.
    pub class: Option<&'a str>,
    /// Tasks naming this draft, in `headerMessageId` or `draft.hMsgId`.
    pub header_message_id: Option<&'a str>,
    pub limit: usize,
}

impl TaskFilter<'_> {
    fn keeps(&self, task: &Value) -> bool {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        self.statuses
            .is_none_or(|list| list.iter().any(|st| *st == s(task, "status")))
            && self.class.is_none_or(|c| s(task, "__cls") == c)
            && self.header_message_id.is_none_or(|h| {
                s(task, "headerMessageId") == h
                    || task.get("draft").is_some_and(|d| s(d, "hMsgId") == h)
            })
    }
}

/// The models `metadata.list` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MetadataObject {
    #[default]
    Message,
    Thread,
}

impl MetadataObject {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Message" => Some(MetadataObject::Message),
            "Thread" => Some(MetadataObject::Thread),
            _ => None,
        }
    }
}

/// What `metadata.list` keeps; see `ms_ipc::MetadataListParams`.
#[derive(Clone, Debug, Default)]
pub struct MetadataFilter<'a> {
    pub accounts: &'a [String],
    pub plugin_id: &'a str,
    pub object: MetadataObject,
    /// Messages only.
    pub draft: Option<bool>,
    pub expires_before: Option<i64>,
    pub has_expiration: Option<bool>,
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub limit: usize,
    pub offset: usize,
}

impl ReadStore {
    /// Opens `$CONFIG_DIR_PATH/edgehill.db` read-only, with the engine's busy timeout. The
    /// database must exist; WAL lets these reads run while a writer holds the lock.
    pub fn open(config_dir: &Path) -> Result<ReadStore> {
        let conn = Connection::open_with_flags(
            config_dir.join(DATABASE_FILE),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))?;
        conn.execute_batch("PRAGMA query_only = 1;")?;
        Ok(ReadStore { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The account's folders or labels by path, with their `ThreadCounts`.
    pub fn categories(&self, table: CategoryTable, account_id: &str) -> Result<Vec<Category>> {
        let sql = format!(
            "SELECT c.id, c.data, n.unread, n.total FROM {t} c \
             LEFT JOIN ThreadCounts n ON n.categoryId = c.id \
             WHERE c.accountId = ? ORDER BY c.path, c.id",
            t = table.name()
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([account_id], |r| {
            let unread: Option<i64> = r.get(2)?;
            let total: Option<i64> = r.get(3)?;
            Ok(Category {
                id: r.get(0)?,
                data: json_column(r, 1)?,
                counts: match (unread, total) {
                    (Some(unread), Some(total)) => Some(Counts { unread, total }),
                    _ => None,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The role of the account's folder (or label) with this id, `""` when it has none and
    /// `None` when there is no such category.
    pub fn category_role(
        &self,
        table: CategoryTable,
        account_id: &str,
        id: &str,
    ) -> Result<Option<String>> {
        let sql = format!(
            "SELECT COALESCE(role, '') FROM {} WHERE id = ? AND accountId = ?",
            table.name()
        );
        Ok(self
            .conn
            .prepare_cached(&sql)?
            .query_row([id, account_id], |r| r.get(0))
            .optional()?)
    }

    /// The id of the account's folder with `role`, or else its label with it. Gmail keeps
    /// `all`, `spam` and `trash` as folders and the other roles as labels.
    pub fn category_for_role(&self, account_id: &str, role: &str) -> Result<Option<String>> {
        for table in [CategoryTable::Folder, CategoryTable::Label] {
            let sql = format!(
                "SELECT id FROM {} WHERE accountId = ? AND role = ? ORDER BY id LIMIT 1",
                table.name()
            );
            let id: Option<String> = self
                .conn
                .prepare_cached(&sql)?
                .query_row([account_id, role], |r| r.get(0))
                .optional()?;
            if id.is_some() {
                return Ok(id);
            }
        }
        Ok(None)
    }

    /// One page of the account's threads, newest first, in `category` when given, starting
    /// after `before`. Stable while threads are added: the order is total and the cursor is
    /// the last row's key. Outside spam and trash, a category's threads are only those in
    /// all mail, as in the desktop's CategoryMailboxPerspective.threads(): a thread whose
    /// messages are all in spam or trash keeps its labels but is not listed under them.
    pub fn threads(
        &self,
        account_id: &str,
        category: Option<InCategory<'_>>,
        limit: usize,
        before: Option<&Cursor>,
    ) -> Result<ThreadPage> {
        let limit = clamp(limit);
        let (ts, id) = match before {
            Some(c) => (Some(c.timestamp), Some(c.id.as_str())),
            None => (None, None),
        };
        let page = "(?3 IS NULL OR t.lastMessageTimestamp < ?3 \
                     OR (t.lastMessageTimestamp = ?3 AND t.id < ?4)) \
                    ORDER BY t.lastMessageTimestamp DESC, t.id DESC LIMIT ?5";
        let sql = match category {
            Some(_) => format!(
                "SELECT t.data, t.lastMessageTimestamp, t.id FROM Thread t \
                 INNER JOIN ThreadCategory c ON c.id = t.id \
                 WHERE t.accountId = ?1 AND c.value = ?2 AND (?6 OR c.inAllMail = 1) AND {page}"
            ),
            // ?2 and ?6 are bound either way, so the statement takes them.
            None => format!(
                "SELECT t.data, t.lastMessageTimestamp, t.id FROM Thread t \
                 WHERE t.accountId = ?1 AND ?2 IS NULL AND ?6 AND {page}"
            ),
        };
        let category_id = category.map(|c| c.id);
        let outside_all_mail = category.is_none_or(|c| matches!(c.role, "spam" | "trash"));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut rows: Vec<(Value, Cursor)> = stmt
            .query_map(
                params![account_id, category_id, ts, id, limit + 1, outside_all_mail],
                |r| {
                    Ok((
                        json_column(r, 0)?,
                        Cursor {
                            timestamp: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            id: r.get(2)?,
                        },
                    ))
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        let next = if rows.len() as i64 > limit {
            rows.truncate(limit as usize);
            rows.last().map(|(_, c)| c.clone())
        } else {
            None
        };
        Ok(ThreadPage {
            threads: rows.into_iter().map(|(v, _)| v).collect(),
            next,
        })
    }

    pub fn thread(&self, account_id: &str, id: &str) -> Result<Option<Value>> {
        Ok(self
            .conn
            .prepare_cached("SELECT data FROM Thread WHERE id = ? AND accountId = ?")?
            .query_row([id, account_id], |r| json_column(r, 0))
            .optional()?)
    }

    /// The thread's messages, oldest first, without bodies.
    pub fn thread_messages(&self, account_id: &str, thread_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT data FROM Message WHERE threadId = ? AND accountId = ? ORDER BY date ASC, id ASC",
        )?;
        let rows = stmt.query_map([thread_id, account_id], |r| message_column(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn message(&self, account_id: &str, id: &str) -> Result<Option<Value>> {
        Ok(self
            .conn
            .prepare_cached("SELECT data FROM Message WHERE id = ? AND accountId = ?")?
            .query_row([id, account_id], |r| message_column(r, 0))
            .optional()?)
    }

    /// The `ThreadCounts` rows of the accounts' folders and labels, by category id. Rows of a
    /// category that no longer exists are left out.
    pub fn counts(&self, accounts: &[String]) -> Result<Vec<(String, Counts)>> {
        let wanted: HashSet<&str> = accounts.iter().map(String::as_str).collect();
        let mut stmt = self.conn.prepare_cached(
            "SELECT n.categoryId, n.unread, n.total, c.accountId FROM ThreadCounts n \
             INNER JOIN (SELECT id, accountId FROM Folder UNION ALL SELECT id, accountId FROM Label) c \
             ON c.id = n.categoryId ORDER BY n.categoryId",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Counts {
                    unread: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    total: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                },
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, counts, account) = row?;
            if wanted.contains(account.as_str()) {
                out.push((id, counts));
            }
        }
        Ok(out)
    }

    /// The stored tasks `filter` keeps, at most `filter.limit` (clamped to [`MAX_LIMIT`]):
    /// the pending ones (`local`, `remote`) first, oldest queued first, then the finished ones,
    /// newest first. Pending tasks come first so the hundred finished tasks an account keeps
    /// can't crowd them out of an unfiltered list.
    pub fn tasks(&self, filter: &TaskFilter<'_>) -> Result<Vec<Value>> {
        let wanted: HashSet<&str> = filter.accounts.iter().map(String::as_str).collect();
        let limit = clamp(filter.limit) as usize;
        // Note: the table holds the pending tasks plus the newest hundred finished ones per
        // account, so the filters run here rather than as SQL over JSON in a BLOB column.
        let mut stmt = self
            .conn
            .prepare_cached("SELECT accountId, data FROM Task ORDER BY rowid ASC")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                json_column(r, 1)?,
            ))
        })?;
        let mut pending = Vec::new();
        let mut finished = Vec::new();
        for row in rows {
            let (account, task) = row?;
            if !wanted.contains(account.as_str()) || !filter.keeps(&task) {
                continue;
            }
            match task.get("status").and_then(Value::as_str) {
                Some("local" | "remote") => pending.push(task),
                _ => finished.push(task),
            }
        }
        // Rows are updated in place, so rowid order is queue order.
        finished.reverse();
        pending.extend(finished);
        pending.truncate(limit);
        Ok(pending)
    }

    /// The messages or threads carrying `filter.plugin_id`'s metadata, newest first.
    pub fn metadata_models(&self, filter: &MetadataFilter<'_>) -> Result<Vec<Value>> {
        if filter.accounts.is_empty() {
            return Ok(Vec::new());
        }
        let (table, date) = match filter.object {
            MetadataObject::Message => ("Message", "date"),
            MetadataObject::Thread => ("Thread", "lastMessageTimestamp"),
        };
        let mut sql = format!(
            "SELECT m.data FROM ModelPluginMetadata p INNER JOIN {table} m ON m.id = p.id \
             WHERE p.value = ? AND p.objectType = ? AND p.accountId IN ({})",
            placeholders(filter.accounts.len())
        );
        let mut args: Vec<SqlValue> = vec![
            SqlValue::Text(filter.plugin_id.to_owned()),
            SqlValue::Text(table.to_owned()),
        ];
        args.extend(filter.accounts.iter().cloned().map(SqlValue::Text));
        if let (Some(draft), MetadataObject::Message) = (filter.draft, filter.object) {
            sql.push_str(" AND m.draft = ?");
            args.push(SqlValue::Integer(draft.into()));
        }
        if let Some(t) = filter.expires_before {
            sql.push_str(" AND p.expiration IS NOT NULL AND p.expiration < ?");
            args.push(SqlValue::Integer(t));
        }
        match filter.has_expiration {
            Some(true) => sql.push_str(" AND p.expiration IS NOT NULL"),
            Some(false) => sql.push_str(" AND p.expiration IS NULL"),
            None => {}
        }
        if let Some(t) = filter.after {
            sql.push_str(&format!(" AND m.{date} > ?"));
            args.push(SqlValue::Integer(t));
        }
        if let Some(t) = filter.before {
            sql.push_str(&format!(" AND m.{date} < ?"));
            args.push(SqlValue::Integer(t));
        }
        sql.push_str(&format!(
            " ORDER BY m.{date} DESC, m.id DESC LIMIT ? OFFSET ?"
        ));
        args.push(SqlValue::Integer(clamp(filter.limit)));
        args.push(SqlValue::Integer(
            filter.offset.min(i64::MAX as usize) as i64
        ));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let object = filter.object;
        let rows = stmt.query_map(params_from_iter(args), |r| match object {
            MetadataObject::Message => message_column(r, 0),
            MetadataObject::Thread => json_column(r, 0),
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn task(&self, account_id: &str, id: &str) -> Result<Option<Value>> {
        Ok(self
            .conn
            .prepare_cached("SELECT data FROM Task WHERE id = ? AND accountId = ?")?
            .query_row([id, account_id], |r| json_column(r, 0))
            .optional()?)
    }

    pub fn file(&self, account_id: &str, id: &str) -> Result<Option<Value>> {
        Ok(self
            .conn
            .prepare_cached("SELECT data FROM File WHERE id = ? AND accountId = ?")?
            .query_row([id, account_id], |r| json_column(r, 0))
            .optional()?)
    }

    /// The stored body, `None` when it has not been fetched (no row, or the NULL row a
    /// pending fetch leaves).
    pub fn message_body(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached("SELECT value FROM MessageBody WHERE id = ? AND value IS NOT NULL")?
            .query_row([id], |r| r.get(0))
            .optional()?)
    }

    /// Threads matching `query` in `ThreadSearch`, newest first. See [`fts_query`] for how
    /// the text is read; a query with no terms matches nothing.
    pub fn search_threads(
        &self,
        account_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let Some(expr) = fts_query(query, false) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.data FROM ThreadSearch s INNER JOIN Thread t ON t.id = s.content_id \
             WHERE ThreadSearch MATCH ?1 AND t.accountId = ?2 \
             ORDER BY t.lastMessageTimestamp DESC, t.id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![expr, account_id, clamp(limit)], |r| {
            json_column(r, 0)
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Visible contacts whose email or name has words starting with every term of `query`,
    /// most referenced first.
    pub fn search_contacts(
        &self,
        account_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let Some(expr) = fts_query(query, true) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.data FROM ContactSearch s INNER JOIN Contact c ON c.id = s.content_id \
             WHERE ContactSearch MATCH ?1 AND c.accountId = ?2 AND c.hidden = 0 \
             ORDER BY c.refs DESC, c.id ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![expr, account_id, clamp(limit)], |r| {
            json_column(r, 0)
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// An FTS5 query for user text: every whitespace-separated term becomes a quoted string
/// (inner quotes doubled), joined by implicit AND, so no character the user types is read
/// as FTS syntax (`AND`, `NEAR`, `-`, `:`, `*`, `^`, parentheses). `prefix` makes every term
/// a prefix match. `None` when the text has no terms.
pub fn fts_query(text: &str, prefix: bool) -> Option<String> {
    let star = if prefix { "*" } else { "" };
    let terms: Vec<String> = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"{star}", t.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_quotes_every_term() {
        assert_eq!(fts_query("  ", false), None);
        assert_eq!(fts_query("a b", false).unwrap(), r#""a" "b""#);
        assert_eq!(
            fts_query(r#"say "hi" NEAR(x) -y"#, true).unwrap(),
            r#""say"* """hi"""* "NEAR(x)"* "-y"*"#
        );
        assert_eq!(fts_query("a\0b", false).unwrap(), r#""a" "b""#);
    }

    #[test]
    fn cursor_round_trip() {
        let c = Cursor {
            timestamp: 1_600_000_000,
            id: "t:ab:c".into(),
        };
        assert_eq!(Cursor::decode(&c.encode()), Some(c));
        assert_eq!(Cursor::decode("x:y"), None);
        assert_eq!(Cursor::decode("nocolon"), None);
    }
}
