//! Consistency checks of the store's derived state against its canonical tables, ported
//! from `apps/engine/test/harness/invariants.py` so unit tests (here and in the sync and
//! task crates) can run them after store operations. Keep the two in step.
//!
//! The store maintains several layers incrementally rather than recomputing them:
//!
//! ```text
//! MessageFolder rows  ->  Message.data.folders / unread / starred / draft / labels
//! Message snapshots   ->  Thread.data.folders / labels (_refs, _u), unread, starred
//! Thread arrays       ->  ThreadCategory rows
//! ThreadCategory rows ->  ThreadCounts
//! model JSON          ->  indexed columns, ContactSearch / EventSearch rows
//! ContactContactGroup ->  Contact.data.gis
//! Message/Thread metadata -> ModelPluginMetadata rows
//! ```
//!
//! Each check recomputes one layer from the stored layer below it, so a bug surfaces once,
//! at the layer that diverged. [`check`] returns human-readable violations.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ms_core::Result;
use rusqlite::Connection;
use serde_json::Value;

use crate::model::{json_bool, json_i64, json_size, json_str};

const UNREAD: i64 = 1;
const STARRED: i64 = 2;
const DRAFT: i64 = 4;
const MAX_REPORTED_PER_CHECK: usize = 12;

type CheckFn = fn(&Snapshot) -> Vec<String>;

const CHECK_FNS: [(&str, CheckFn); 11] = [
    ("message_snapshot", check_message_snapshot),
    ("message_flags", check_message_flags),
    ("thread_refcounts", check_thread_refcounts),
    ("thread_categories", check_thread_categories),
    ("thread_counts", check_thread_counts),
    ("orphans", check_orphans),
    ("contacts", check_contacts),
    ("contact_groups", check_contact_groups),
    ("events", check_events),
    ("calendars", check_calendars),
    ("plugin_metadata", check_plugin_metadata),
];

/// The check names a caller may pass in `skip`, as in `invariants.py`'s `CHECKS`.
pub const CHECKS: [&str; 11] = [
    "message_snapshot",
    "message_flags",
    "thread_refcounts",
    "thread_categories",
    "thread_counts",
    "orphans",
    "contacts",
    "contact_groups",
    "events",
    "calendars",
    "plugin_metadata",
];

struct Row {
    folder_id: String,
    pending_folder_id: Option<String>,
    unread: bool,
    starred: bool,
    draft: bool,
    labels: Vec<String>,
}

struct MessageRow {
    account_id: String,
    unread: bool,
    starred: bool,
    draft: bool,
    data: Value,
}

struct LabelRow {
    id: String,
    path: String,
    role: String,
}

struct CategoryRow {
    in_all_mail: bool,
    unread: bool,
    lmrt: f64,
    lmst: f64,
}

struct Snapshot {
    roles: HashMap<String, String>,
    labels: HashMap<String, Vec<LabelRow>>,
    messages: BTreeMap<String, MessageRow>,
    rows: HashMap<String, Vec<Row>>,
    orphans: BTreeMap<String, i64>,
    threads: BTreeMap<String, Value>,
    thread_categories: BTreeMap<String, BTreeMap<String, CategoryRow>>,
    thread_counts: BTreeMap<String, (i64, i64)>,
    pim: Pim,
}

/// A row of a PIM table: its columns by name, with `data` parsed.
struct PimRow {
    columns: HashMap<String, Value>,
    data: Value,
}

impl PimRow {
    fn column(&self, name: &str) -> &Value {
        self.columns.get(name).unwrap_or(&Value::Null)
    }
}

struct MetadataRow {
    id: String,
    object_type: String,
    plugin_id: String,
    expiration: Option<i64>,
}

#[derive(Default)]
struct Pim {
    contacts: BTreeMap<String, PimRow>,
    groups: BTreeMap<String, PimRow>,
    books: BTreeMap<String, PimRow>,
    calendars: BTreeMap<String, PimRow>,
    events: BTreeMap<String, PimRow>,
    join: Vec<(String, String)>,
    contact_search: Vec<String>,
    event_search: Vec<String>,
    metadata_rows: Vec<MetadataRow>,
}

fn text(v: rusqlite::types::ValueRef<'_>) -> String {
    match v {
        rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
            String::from_utf8_lossy(b).into_owned()
        }
        rusqlite::types::ValueRef::Integer(i) => i.to_string(),
        rusqlite::types::ValueRef::Real(f) => f.to_string(),
        rusqlite::types::ValueRef::Null => String::new(),
    }
}

fn num(v: rusqlite::types::ValueRef<'_>) -> f64 {
    match v {
        rusqlite::types::ValueRef::Integer(i) => i as f64,
        rusqlite::types::ValueRef::Real(f) => f,
        rusqlite::types::ValueRef::Text(t) => std::str::from_utf8(t)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

impl Snapshot {
    fn load(conn: &Connection) -> Result<Snapshot> {
        let mut s = Snapshot {
            roles: HashMap::new(),
            labels: HashMap::new(),
            messages: BTreeMap::new(),
            rows: HashMap::new(),
            orphans: BTreeMap::new(),
            threads: BTreeMap::new(),
            thread_categories: BTreeMap::new(),
            thread_counts: BTreeMap::new(),
            pim: Pim::default(),
        };
        for table in ["Folder", "Label"] {
            let mut stmt = conn.prepare(&format!("SELECT id, role FROM {table}"))?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                s.roles.insert(text(r.get_ref(0)?), text(r.get_ref(1)?));
            }
        }
        {
            let mut stmt = conn.prepare("SELECT id, accountId, path, role FROM Label")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                s.labels
                    .entry(text(r.get_ref(1)?))
                    .or_default()
                    .push(LabelRow {
                        id: text(r.get_ref(0)?),
                        path: text(r.get_ref(2)?),
                        role: text(r.get_ref(3)?),
                    });
            }
        }
        {
            let mut stmt =
                conn.prepare("SELECT id, accountId, unread, starred, draft, data FROM Message")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                s.messages.insert(
                    text(r.get_ref(0)?),
                    MessageRow {
                        account_id: text(r.get_ref(1)?),
                        unread: num(r.get_ref(2)?) != 0.0,
                        starred: num(r.get_ref(3)?) != 0.0,
                        draft: num(r.get_ref(4)?) != 0.0,
                        data: serde_json::from_str(&text(r.get_ref(5)?))?,
                    },
                );
            }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT messageId, folderId, pendingFolderId, unread, starred, draft, remoteXGMLabels FROM MessageFolder ORDER BY rowid",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let labels: Vec<String> = serde_json::from_str::<Value>(&text(r.get_ref(6)?))
                    .ok()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                s.rows.entry(text(r.get_ref(0)?)).or_default().push(Row {
                    folder_id: text(r.get_ref(1)?),
                    pending_folder_id: Some(text(r.get_ref(2)?)).filter(|p| !p.is_empty()),
                    unread: num(r.get_ref(3)?) != 0.0,
                    starred: num(r.get_ref(4)?) != 0.0,
                    draft: num(r.get_ref(5)?) != 0.0,
                    labels,
                });
            }
        }
        {
            let mut stmt = conn.prepare("SELECT messageId, since FROM MessageOrphan")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                s.orphans
                    .insert(text(r.get_ref(0)?), num(r.get_ref(1)?) as i64);
            }
        }
        {
            let mut stmt = conn.prepare("SELECT id, data FROM Thread")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                s.threads.insert(
                    text(r.get_ref(0)?),
                    serde_json::from_str(&text(r.get_ref(1)?))?,
                );
            }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT id, value, inAllMail, unread, lastMessageReceivedTimestamp, lastMessageSentTimestamp FROM ThreadCategory",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                s.thread_categories
                    .entry(text(r.get_ref(0)?))
                    .or_default()
                    .insert(
                        text(r.get_ref(1)?),
                        CategoryRow {
                            in_all_mail: num(r.get_ref(2)?) != 0.0,
                            unread: num(r.get_ref(3)?) != 0.0,
                            lmrt: num(r.get_ref(4)?),
                            lmst: num(r.get_ref(5)?),
                        },
                    );
            }
        }
        {
            let mut stmt = conn.prepare("SELECT categoryId, unread, total FROM ThreadCounts")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                s.thread_counts.insert(
                    text(r.get_ref(0)?),
                    (num(r.get_ref(1)?) as i64, num(r.get_ref(2)?) as i64),
                );
            }
        }
        s.pim = load_pim(conn)?;
        Ok(s)
    }

    fn role(&self, id: &str) -> &str {
        self.roles.get(id).map(String::as_str).unwrap_or("")
    }

    /// `Message::inAllMail`: some copy outside spam and trash; unknown folders count.
    fn in_all_mail<'a>(&self, folder_ids: impl IntoIterator<Item = &'a String>) -> bool {
        folder_ids
            .into_iter()
            .any(|f| !matches!(self.role(f), "spam" | "trash"))
    }

    fn label_for_xgm_name(&self, account_id: &str, name: &str) -> Option<&LabelRow> {
        let labels = self.labels.get(account_id)?;
        if let Some(l) = labels.iter().find(|l| l.path == name) {
            return Some(l);
        }
        let needle = name.strip_prefix('\\')?.to_ascii_lowercase();
        labels.iter().find(|l| {
            let path = l.path.to_ascii_lowercase();
            let path = path.strip_prefix("[gmail]/").unwrap_or(&path);
            path == needle || l.role == needle || l.role == format!("{needle}s")
        })
    }
}

fn check_message_snapshot(s: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    let empty = Vec::new();
    for (mid, m) in &s.messages {
        let rows = s.rows.get(mid).unwrap_or(&empty);
        let mut want: BTreeMap<String, i64> = BTreeMap::new();
        let mut labels = BTreeSet::new();
        for r in rows {
            let key = r
                .pending_folder_id
                .clone()
                .unwrap_or_else(|| r.folder_id.clone());
            let bits = (if r.unread { UNREAD } else { 0 })
                | (if r.starred { STARRED } else { 0 })
                | (if r.draft { DRAFT } else { 0 });
            *want.entry(key).or_insert(0) |= bits;
            labels.extend(r.labels.iter().cloned());
        }
        let have: Option<BTreeMap<String, i64>> = m
            .data
            .get("folders")
            .and_then(Value::as_object)
            .map(|o| o.iter().map(|(k, v)| (k.clone(), json_i64(v))).collect());
        if have.as_ref() != Some(&want) {
            out.push(format!(
                "message {mid}: folders snapshot {have:?} != {want:?} from MessageFolder rows"
            ));
        }
        if !rows.is_empty() {
            let have_labels: BTreeSet<String> = m
                .data
                .get("labels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if have_labels != labels {
                out.push(format!(
                    "message {mid}: labels {have_labels:?} != {labels:?} from MessageFolder rows"
                ));
            }
        }
    }
    out
}

fn check_message_flags(s: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    for (mid, m) in &s.messages {
        let rows = s.rows.get(mid);
        let want = rows.filter(|r| !r.is_empty()).map(|rows| {
            (
                rows.iter().any(|r| r.unread),
                rows.iter().any(|r| r.starred),
                rows.iter()
                    .any(|r| r.draft || s.role(&r.folder_id) == "drafts"),
            )
        });
        for (i, (flag, column)) in [
            ("unread", m.unread),
            ("starred", m.starred),
            ("draft", m.draft),
        ]
        .into_iter()
        .enumerate()
        {
            let data_value = json_bool(&m.data, flag);
            if column != data_value {
                out.push(format!(
                    "message {mid}: column {flag}={column} but data.{flag}={data_value}"
                ));
            }
            if let Some(w) = want {
                let expected = [w.0, w.1, w.2][i];
                if data_value != expected {
                    out.push(format!("message {mid}: {flag}={data_value}, expected {expected} from MessageFolder rows"));
                }
            }
        }
    }
    out
}

fn check_thread_refcounts(s: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    let mut by_thread: BTreeMap<&str, Vec<&MessageRow>> = BTreeMap::new();
    for (mid, m) in &s.messages {
        let tid = json_str(&m.data, "threadId");
        if !tid.is_empty() {
            by_thread.entry(tid).or_default().push(m);
            if !s.threads.contains_key(tid) {
                out.push(format!("message {mid}: threadId {tid} names no Thread row"));
            }
        }
    }
    for (tid, t) in &s.threads {
        let msgs = by_thread.get(tid.as_str()).cloned().unwrap_or_default();
        let mut folders: BTreeMap<String, [i64; 2]> = BTreeMap::new();
        let mut labels: BTreeMap<String, [i64; 2]> = BTreeMap::new();
        let (mut unread, mut starred) = (0, 0);
        for m in &msgs {
            let d = &m.data;
            unread += json_bool(d, "unread") as i64;
            starred += json_bool(d, "starred") as i64;
            let snapshot = d
                .get("folders")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (fid, bits) in &snapshot {
                let e = folders.entry(fid.clone()).or_insert([0, 0]);
                e[0] += 1;
                e[1] += (json_i64(bits) & UNREAD != 0) as i64;
            }
            let label_unread = (json_bool(d, "unread") && s.in_all_mail(snapshot.keys())) as i64;
            for name in d
                .get("labels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(label) = name
                    .as_str()
                    .and_then(|n| s.label_for_xgm_name(&m.account_id, n))
                {
                    let e = labels.entry(label.id.clone()).or_insert([0, 0]);
                    e[0] += 1;
                    e[1] += label_unread;
                }
            }
        }
        let where_ = format!("thread {tid} ({} messages)", msgs.len());
        for (kind, want, key) in [
            ("folder", &folders, "folders"),
            ("label", &labels, "labels"),
        ] {
            let mut have: BTreeMap<String, [i64; 2]> = BTreeMap::new();
            for entry in t.get(key).and_then(Value::as_array).into_iter().flatten() {
                let id = json_str(entry, "id").to_owned();
                if have.contains_key(&id) {
                    out.push(format!("{where_}: {kind} {id} listed twice"));
                }
                have.insert(id, [json_i64(&entry["_refs"]), json_i64(&entry["_u"])]);
            }
            let ids: BTreeSet<&String> = want.keys().chain(have.keys()).collect();
            for id in ids {
                if want.get(id) != have.get(id) {
                    out.push(format!(
                        "{where_}: {kind} {id} [_refs, _u] = {:?}, expected {:?}",
                        have.get(id),
                        want.get(id)
                    ));
                }
            }
        }
        if json_i64(&t["unread"]) != unread || json_i64(&t["starred"]) != starred {
            out.push(format!(
                "{where_}: unread/starred = {}/{}, expected {unread}/{starred}",
                t["unread"], t["starred"]
            ));
        }
        let have_folders: Vec<&Value> = t
            .get("folders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        let spam_trash = have_folders
            .iter()
            .filter(|f| matches!(json_str(f, "role"), "spam" | "trash"))
            .count();
        let in_all_mail = have_folders.len() > spam_trash;
        if t.get("inAllMail").and_then(Value::as_bool) != Some(in_all_mail) {
            out.push(format!(
                "{where_}: inAllMail = {}, expected {in_all_mail}",
                t["inAllMail"]
            ));
        }
    }
    out
}

fn check_thread_categories(s: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    let ids: BTreeSet<&String> = s.threads.keys().chain(s.thread_categories.keys()).collect();
    let none = BTreeMap::new();
    for tid in ids {
        let have = s.thread_categories.get(tid).unwrap_or(&none);
        let Some(d) = s.threads.get(tid) else {
            out.push(format!(
                "ThreadCategory has rows {:?} for missing thread {tid}",
                have.keys().collect::<Vec<_>>()
            ));
            continue;
        };
        let want: BTreeMap<String, bool> = ["folders", "labels"]
            .iter()
            .flat_map(|k| {
                d.get(*k)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .map(|e| (json_str(&e, "id").to_owned(), json_i64(&e["_u"]) > 0))
            .collect();
        let want_ids: BTreeSet<&String> = want.keys().collect();
        let have_ids: BTreeSet<&String> = have.keys().collect();
        if want_ids != have_ids {
            out.push(format!(
                "thread {tid}: ThreadCategory values {have_ids:?}, expected {want_ids:?}"
            ));
        }
        for (cid, unread) in &want {
            let Some(row) = have.get(cid) else { continue };
            if row.unread != *unread {
                out.push(format!(
                    "thread {tid}: ThreadCategory {cid} unread={}, expected {unread}",
                    row.unread
                ));
            }
            let thread_in_all_mail = json_bool(d, "inAllMail");
            if row.in_all_mail != thread_in_all_mail {
                out.push(format!(
                    "thread {tid}: ThreadCategory {cid} inAllMail={}, thread says {thread_in_all_mail}",
                    row.in_all_mail
                ));
            }
            for (col, value, key) in [
                ("lastMessageReceivedTimestamp", row.lmrt, "lmrt"),
                ("lastMessageSentTimestamp", row.lmst, "lmst"),
            ] {
                let thread_value = d.get(key).and_then(Value::as_f64).unwrap_or(0.0);
                if value != thread_value {
                    out.push(format!("thread {tid}: ThreadCategory {cid} {col}={value}, thread {key}={thread_value}"));
                }
            }
        }
    }
    out
}

/// Only categories with a ThreadCounts row are counted: `Thread::after_save` updates
/// those rows and never inserts one.
fn check_thread_counts(s: &Snapshot) -> Vec<String> {
    let mut totals: HashMap<&str, (i64, i64)> = HashMap::new();
    for cats in s.thread_categories.values() {
        for (cid, row) in cats {
            let e = totals.entry(cid).or_insert((0, 0));
            e.0 += row.unread as i64;
            e.1 += 1;
        }
    }
    s.thread_counts
        .iter()
        .filter_map(|(cid, have)| {
            let want = totals.get(cid.as_str()).copied().unwrap_or((0, 0));
            (*have != want).then(|| format!("ThreadCounts {cid}: (unread, total) = {have:?}, expected {want:?} from ThreadCategory"))
        })
        .collect()
}

/// `MessageOrphan` lists exactly the messages with no `MessageFolder` row: the sweep finds
/// orphans only through it, so a missing record is a message that lives forever.
fn check_orphans(s: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    for mid in s.messages.keys() {
        if s.rows.get(mid).is_none_or(Vec::is_empty) && !s.orphans.contains_key(mid) {
            out.push(format!(
                "message {mid}: no MessageFolder rows and no MessageOrphan record"
            ));
        }
    }
    for (mid, since) in &s.orphans {
        if s.rows.get(mid).is_some_and(|r| !r.is_empty()) {
            out.push(format!("MessageOrphan record for message {mid} (since {since}), which has MessageFolder rows"));
        } else if !s.messages.contains_key(mid) {
            out.push(format!(
                "MessageOrphan record for message {mid} (since {since}), which does not exist"
            ));
        }
    }
    out
}

// -- contacts, calendars, plugin metadata ------------------------------------------------

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?",
        [table],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn sql_value(v: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
        ValueRef::Text(_) | ValueRef::Blob(_) => Value::String(text(v)),
    }
}

fn load_pim_table(conn: &Connection, table: &str) -> Result<BTreeMap<String, PimRow>> {
    let mut out = BTreeMap::new();
    if !has_table(conn, table)? {
        return Ok(out);
    }
    let mut stmt = conn.prepare(&format!("SELECT * FROM `{table}`"))?;
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let mut columns = HashMap::new();
        for (i, name) in names.iter().enumerate() {
            columns.insert(name.clone(), sql_value(r.get_ref(i)?));
        }
        let data = match columns.get("data") {
            Some(Value::String(d)) if !d.is_empty() => serde_json::from_str(d)?,
            _ => Value::Object(Default::default()),
        };
        let id = match columns.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        out.insert(id, PimRow { columns, data });
    }
    Ok(out)
}

fn load_content_ids(conn: &Connection, table: &str) -> Result<Vec<String>> {
    if !has_table(conn, table)? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(&format!("SELECT content_id FROM `{table}`"))?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(text(r.get_ref(0)?));
    }
    Ok(out)
}

fn load_pim(conn: &Connection) -> Result<Pim> {
    let mut p = Pim {
        contacts: load_pim_table(conn, "Contact")?,
        groups: load_pim_table(conn, "ContactGroup")?,
        books: load_pim_table(conn, "ContactBook")?,
        calendars: load_pim_table(conn, "Calendar")?,
        events: load_pim_table(conn, "Event")?,
        contact_search: load_content_ids(conn, "ContactSearch")?,
        event_search: load_content_ids(conn, "EventSearch")?,
        ..Pim::default()
    };
    if has_table(conn, "ContactContactGroup")? {
        let mut stmt = conn.prepare("SELECT id, value FROM ContactContactGroup")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            p.join.push((text(r.get_ref(0)?), text(r.get_ref(1)?)));
        }
    }
    if has_table(conn, "ModelPluginMetadata")? {
        let mut stmt =
            conn.prepare("SELECT id, objectType, value, expiration FROM ModelPluginMetadata")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let expiration = r.get_ref(3)?;
            p.metadata_rows.push(MetadataRow {
                id: text(r.get_ref(0)?),
                object_type: text(r.get_ref(1)?),
                plugin_id: text(r.get_ref(2)?),
                expiration: (expiration != rusqlite::types::ValueRef::Null)
                    .then(|| num(expiration) as i64),
            });
        }
    }
    Ok(p)
}

/// Python truthiness of a JSON value: null, false, 0, "" and empty containers are false.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Python `==` between a column value and a JSON value: numbers compare by value.
fn py_eq(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn blank(v: &Value) -> bool {
    matches!(v, Value::Null) || v.as_str() == Some("")
}

/// `pairs`: (column, JSON key, default when the key is absent), as `bindToQuery` writes them.
fn column_mismatches(
    table: &str,
    id: &str,
    row: &PimRow,
    pairs: &[(&str, &str, Value)],
) -> Vec<String> {
    let mut out = Vec::new();
    for (col, key, default) in pairs {
        let mut want = row
            .data
            .get(*key)
            .cloned()
            .unwrap_or_else(|| default.clone());
        if let Value::Bool(b) = want {
            want = Value::from(b as i64);
        }
        let have = row.column(col);
        let same = py_eq(have, &want) || (blank(have) && blank(&want));
        if !same {
            out.push(format!(
                "{table} {id}: column {col} = {have}, data.{key} = {want}"
            ));
        }
    }
    out
}

/// Contact columns == its JSON (`Contact::bindToQuery`); one ContactSearch row per contact
/// and none for a removed one (afterSave / afterRemove); a non-mail contact's book exists;
/// and, between a contact and a group that both exist, `gis` names the group exactly when
/// a ContactContactGroup row joins them (`ContactGroup::syncMembers` writes both).
fn check_contacts(s: &Snapshot) -> Vec<String> {
    let p = &s.pim;
    let mut out = Vec::new();
    let pairs = [
        ("email", "email", Value::from("")),
        ("hidden", "h", Value::from(false)),
        ("source", "s", Value::from("mail")),
        ("etag", "etag", Value::from("")),
        ("bookId", "bid", Value::from("")),
        ("accountId", "aid", Value::Null),
    ];
    for (id, c) in &p.contacts {
        out.extend(column_mismatches("Contact", id, c, &pairs));
        let source = c
            .data
            .get("s")
            .cloned()
            .unwrap_or_else(|| Value::from("mail"));
        let bid = c.data.get("bid").unwrap_or(&Value::Null);
        if source != "mail" && truthy(bid) && !bid.as_str().is_some_and(|b| p.books.contains_key(b))
        {
            out.push(format!(
                "Contact {id} ({}): book {bid} does not exist",
                c.data.get("email").unwrap_or(&Value::Null)
            ));
        }
    }
    let mut search: BTreeMap<&str, usize> = BTreeMap::new();
    for cid in &p.contact_search {
        *search.entry(cid).or_default() += 1;
    }
    for cid in p.contacts.keys() {
        let n = search.get(cid.as_str()).copied().unwrap_or(0);
        if n != 1 {
            out.push(format!("Contact {cid}: {n} ContactSearch rows, expected 1"));
        }
    }
    for cid in search.keys() {
        if !p.contacts.contains_key(*cid) {
            out.push(format!(
                "ContactSearch row for {cid}, which is not a Contact"
            ));
        }
    }
    // Only pairs where both the contact and the group exist: ContactGroup::afterRemove
    // deletes the join rows but leaves members' `gis`, and Contact::afterRemove leaves its
    // join rows (ContactGroup.cpp:75-82, Contact.cpp), so dangling references are expected.
    let mut joined: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (cid, gid) in &p.join {
        if p.groups.contains_key(gid) {
            joined.entry(cid).or_default().insert(gid);
        }
    }
    let none = BTreeSet::new();
    for (id, c) in &p.contacts {
        let gis: BTreeSet<&str> = c
            .data
            .get("gis")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|g| p.groups.contains_key(*g))
            .collect();
        let have = joined.get(id.as_str()).unwrap_or(&none);
        if &gis != have {
            out.push(format!(
                "Contact {id} ({}): gis {gis:?} vs ContactContactGroup {have:?}",
                c.data.get("email").unwrap_or(&Value::Null)
            ));
        }
    }
    out
}

/// ContactGroup columns == its JSON (`ContactGroup::bindToQuery`); its book exists.
fn check_contact_groups(s: &Snapshot) -> Vec<String> {
    let p = &s.pim;
    let mut out = Vec::new();
    let pairs = [
        ("name", "name", Value::from("")),
        ("bookId", "bid", Value::from("")),
        ("accountId", "aid", Value::Null),
    ];
    for (id, g) in &p.groups {
        out.extend(column_mismatches("ContactGroup", id, g, &pairs));
        let bid = g.data.get("bid").unwrap_or(&Value::Null);
        if truthy(bid) && !bid.as_str().is_some_and(|b| p.books.contains_key(b)) {
            out.push(format!(
                "ContactGroup {id} ({}): book {bid} does not exist",
                g.data.get("name").unwrap_or(&Value::Null)
            ));
        }
    }
    out
}

/// Event columns == its JSON (`Event::bindToQuery`); its calendar exists; at most one row
/// per (calendar, UID, RECURRENCE-ID), the identity `MailUtils::idForEvent` hashes;
/// EventSearch has at most one row per event and none for a removed one.
fn check_events(s: &Snapshot) -> Vec<String> {
    let p = &s.pim;
    let mut out = Vec::new();
    let pairs = [
        ("etag", "etag", Value::from("")),
        ("calendarId", "cid", Value::Null),
        ("icsuid", "icsuid", Value::Null),
        ("recurrenceId", "rid", Value::from("")),
        ("recurrenceStart", "rs", Value::Null),
        ("recurrenceEnd", "re", Value::Null),
        ("accountId", "aid", Value::Null),
    ];
    let mut keys: BTreeMap<(String, String, String), Vec<&str>> = BTreeMap::new();
    for (id, e) in &p.events {
        out.extend(column_mismatches("Event", id, e, &pairs));
        let cid = e.data.get("cid").unwrap_or(&Value::Null);
        if !cid.as_str().is_some_and(|c| p.calendars.contains_key(c)) {
            out.push(format!(
                "Event {id} ({}): calendar {cid} does not exist",
                e.data.get("icsuid").unwrap_or(&Value::Null)
            ));
        }
        let rid = e
            .data
            .get("rid")
            .cloned()
            .unwrap_or_else(|| Value::from(""));
        keys.entry((
            cid.to_string(),
            e.data.get("icsuid").unwrap_or(&Value::Null).to_string(),
            rid.to_string(),
        ))
        .or_default()
        .push(id);
    }
    for ((cid, uid, rid), ids) in &keys {
        if ids.len() > 1 {
            out.push(format!(
                "{} Event rows for {uid} {rid} in calendar {cid}: {ids:?}",
                ids.len()
            ));
        }
    }
    let mut search: BTreeMap<&str, usize> = BTreeMap::new();
    for eid in &p.event_search {
        *search.entry(eid).or_default() += 1;
    }
    for (eid, n) in search {
        if !p.events.contains_key(eid) {
            out.push(format!("EventSearch row for {eid}, which is not an Event"));
        } else if n > 1 {
            out.push(format!("Event {eid}: {n} EventSearch rows"));
        }
    }
    out
}

/// `Calendar.accountId == data.aid`, and no two calendars of an account share a path.
fn check_calendars(s: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    let mut paths: BTreeMap<(String, String), Vec<&str>> = BTreeMap::new();
    for (id, c) in &s.pim.calendars {
        out.extend(column_mismatches(
            "Calendar",
            id,
            c,
            &[("accountId", "aid", Value::Null)],
        ));
        paths
            .entry((
                c.column("accountId").to_string(),
                c.data.get("path").unwrap_or(&Value::Null).to_string(),
            ))
            .or_default()
            .push(id);
    }
    for ((aid, path), ids) in &paths {
        if ids.len() > 1 {
            out.push(format!(
                "{} Calendar rows for {path} in account {aid}: {ids:?}",
                ids.len()
            ));
        }
    }
    out
}

/// ModelPluginMetadata == the non-empty metadata entries of every Message and Thread, with
/// `expiration` = `value.expiration` when it is a number (`MailModel::afterSave`); the
/// expiration worker reads only this table, so a missing row is a snooze that never wakes.
fn check_plugin_metadata(s: &Snapshot) -> Vec<String> {
    let mut want: BTreeMap<(String, String), (&str, Option<i64>)> = BTreeMap::new();
    let models = s
        .messages
        .iter()
        .map(|(id, m)| ("Message", id, &m.data))
        .chain(s.threads.iter().map(|(id, t)| ("Thread", id, t)));
    for (table, id, data) in models {
        for entry in data
            .get("metadata")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            // `m["value"].size() == 0` in MailModel::afterSave: only null, {} and [] are
            // skipped; a scalar such as false, 0 or "" still gets a row.
            let value = entry.get("value").unwrap_or(&Value::Null);
            if json_size(value) == 0 {
                continue;
            }
            let exp = value
                .get("expiration")
                .filter(|e| e.is_number())
                .map(json_i64);
            want.insert(
                (id.clone(), json_str(entry, "pluginId").to_owned()),
                (table, exp),
            );
        }
    }
    let have: BTreeMap<(String, String), (&str, Option<i64>)> = s
        .pim
        .metadata_rows
        .iter()
        .map(|r| {
            (
                (r.id.clone(), r.plugin_id.clone()),
                (r.object_type.as_str(), r.expiration),
            )
        })
        .collect();
    let mut out = Vec::new();
    for (key, w) in &want {
        match have.get(key) {
            None => out.push(format!(
                "{} {} carries {} metadata but has no ModelPluginMetadata row",
                w.0, key.0, key.1
            )),
            Some(h) if h != w => out.push(format!(
                "ModelPluginMetadata ({}, {}): (objectType, expiration) = {h:?}, expected {w:?}",
                key.0, key.1
            )),
            Some(_) => {}
        }
    }
    for (key, (table, _)) in &have {
        if !want.contains_key(key) {
            out.push(format!(
                "ModelPluginMetadata ({}, {}, {table}) has no metadata entry on the model",
                key.0, key.1
            ));
        }
    }
    out
}

/// Every violation, prefixed with its check's name, skipping the checks in `skip`.
pub fn check(conn: &Connection, skip: &[&str]) -> Result<Vec<String>> {
    if let Some(unknown) = skip.iter().find(|n| !CHECKS.contains(n)) {
        return Err(ms_core::Error::Other(format!(
            "unknown invariant {unknown}; known: {CHECKS:?}"
        )));
    }
    if !has_table(conn, "MessageFolder")? {
        return Ok(Vec::new()); // a pre-placements database: nothing here applies
    }
    // One read snapshot across every table, even if another connection commits meanwhile.
    conn.execute_batch("SAVEPOINT invariants")?;
    let s = Snapshot::load(conn);
    let _ = conn.execute_batch("ROLLBACK TO invariants; RELEASE invariants");
    let s = s?;
    let mut out = Vec::new();
    for (name, f) in CHECK_FNS {
        if skip.contains(&name) {
            continue;
        }
        let found = f(&s);
        let more = found.len().saturating_sub(MAX_REPORTED_PER_CHECK);
        out.extend(
            found
                .into_iter()
                .take(MAX_REPORTED_PER_CHECK)
                .map(|v| format!("[{name}] {v}")),
        );
        if more > 0 {
            out.push(format!("[{name}] ... and {more} more"));
        }
    }
    Ok(out)
}
