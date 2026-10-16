// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `ThreadCounts` deltas (docs/desktop-client-plan.md 3.2): a client that read `counts.get`
//! once keeps its counts current from these instead of re-reading the table on every Thread
//! delta. Each row is sent with its absolute values, so items merged in the delta buffer
//! keep the last one.

use ms_core::constants::{DELTA_TYPE_PERSIST, DELTA_TYPE_UNPERSIST};
use ms_core::Result;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::{DeltaItem, Store};

/// The delta class. Not a model the desktop inflates: its JSON is a `ThreadCounts` row.
pub const THREAD_COUNTS: &str = "ThreadCounts";

fn row(category_id: &str, unread: i64, total: i64) -> Value {
    json!({"id": category_id, "categoryId": category_id, "unread": unread, "total": total})
}

/// Emits the current row of each category that has one.
pub(crate) fn emit_rows<'a>(
    store: &Store,
    categories: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let mut rows = Vec::new();
    {
        let mut stmt = store
            .conn()
            .prepare_cached("SELECT unread, total FROM ThreadCounts WHERE categoryId = ?")?;
        for id in categories {
            let counts: Option<(i64, i64)> = stmt
                .query_row([id], |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                        r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    ))
                })
                .optional()?;
            if let Some((unread, total)) = counts {
                rows.push(row(id, unread, total));
            }
        }
    }
    if !rows.is_empty() {
        store.emit(DeltaItem::new(DELTA_TYPE_PERSIST, THREAD_COUNTS, rows));
    }
    Ok(())
}

/// Emits the removal of a category's row.
pub(crate) fn emit_removed(store: &Store, category_id: &str) {
    store.emit(DeltaItem::new(
        DELTA_TYPE_UNPERSIST,
        THREAD_COUNTS,
        vec![json!({"id": category_id, "categoryId": category_id})],
    ));
}
