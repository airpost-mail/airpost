"""
Scenario-end consistency checks of the engine's derived state against its canonical tables.

The engine maintains several layers incrementally rather than recomputing them:

    MessageFolder rows  ->  Message.data.folders / unread / starred / draft / labels
    Message snapshots   ->  Thread.data.folders / labels (_refs, _u), unread, starred
    Thread arrays       ->  ThreadCategory rows
    ThreadCategory rows ->  ThreadCounts
    model JSON          ->  indexed columns, ContactSearch / EventSearch rows
    ContactContactGroup ->  Contact.data.gis
    Message/Thread metadata -> ModelPluginMetadata rows

Each check recomputes one layer from the *stored* layer below it, so a bug surfaces once, at
the layer that diverged, instead of cascading. `check(conn, skip)` returns human-readable
violations; the names in CHECKS are what a scenario lists under `invariants: {skip: [...]}`.
"""
import json
import sqlite3
from collections import defaultdict

from .db import has_table

UNREAD, STARRED, DRAFT = 1, 2, 4
MAX_REPORTED_PER_CHECK = 12


# -- semantics the engine may tighten later; each lives in exactly one place ----------------

def expected_message_flags(rows, drafts_folder_ids: set):
    """unread / starred / draft as MailStore::refreshMessageFromPlacements derives them, or
    None when the message keeps whatever flags it had: OR over its rows, draft also set by a
    copy in a drafts-role folder, flags untouched with no rows (an orphan awaiting the sweep)."""
    if not rows:
        return None
    unread = any(r["unread"] for r in rows)
    starred = any(r["starred"] for r in rows)
    draft = any(r["draft"] or r["folderId"] in drafts_folder_ids for r in rows)
    return {"unread": unread, "starred": starred, "draft": draft}


# -- loading -------------------------------------------------------------------------------

class _Snapshot:
    """Every table the checks read, loaded inside one read transaction."""

    def __init__(self, conn: sqlite3.Connection):
        self.roles = {}         # folder or label id -> role (MailStore::folderById covers both)
        self.paths = {}
        for table in ("Folder", "Label"):
            for r in conn.execute(f"SELECT id, path, role FROM {table}"):
                self.roles[r["id"]] = r["role"] or ""
                self.paths[r["id"]] = r["path"] or ""
        # Per account, in MailStore::allLabelsCache's order (findAll<Label> by accountId, no
        # ORDER BY): a real database holds several accounts, each with its own `\Inbox` label.
        self.labels = defaultdict(list)
        for r in conn.execute("SELECT id, accountId, path, role FROM Label"):
            self.labels[r["accountId"]].append(dict(r))

        self.messages = {}
        for r in conn.execute("SELECT id, accountId, subject, threadId, unread, starred, draft, data FROM Message"):
            m = dict(r)
            m["data"] = json.loads(m["data"])
            self.messages[m["id"]] = m

        self.rows = defaultdict(list)
        for r in conn.execute("SELECT * FROM MessageFolder ORDER BY rowid"):
            r = dict(r)
            r["labels"] = json.loads(r["remoteXGMLabels"]) if r["remoteXGMLabels"] else []
            self.rows[r["messageId"]].append(r)

        self.orphans = {}       # message id -> since
        if has_table(conn, "MessageOrphan"):
            self.orphans = {r["messageId"]: r["since"] for r in conn.execute("SELECT messageId, since FROM MessageOrphan")}

        self.threads = {}
        for r in conn.execute("SELECT id, subject, data FROM Thread"):
            t = dict(r)
            t["data"] = json.loads(t["data"])
            self.threads[t["id"]] = t

        self.thread_categories = defaultdict(dict)   # thread id -> category id -> row
        for r in conn.execute("SELECT * FROM ThreadCategory"):
            self.thread_categories[r["id"]][r["value"]] = dict(r)

        self.thread_counts = {r["categoryId"]: (r["unread"], r["total"])
                              for r in conn.execute("SELECT categoryId, unread, total FROM ThreadCounts")}

    def folder_name(self, fid: str) -> str:
        return f"{self.paths.get(fid, '<no such folder>')} ({fid})"

    def in_all_mail(self, folder_ids) -> bool:
        """Message::inAllMail: some copy outside spam and trash; unknown folders count."""
        return any(self.roles.get(fid, "") not in ("spam", "trash") for fid in folder_ids)

    def label_for_xgm_name(self, account_id: str, name: str):
        """labelForXGMLabelName as the Rust engine has it (airpost_store label_for_xgm_label_name):
        a user label by exact path; `\\Name` by role (singular or plural), then by a
        `[Gmail]/` or `[Google Mail]/` path naming it, case-insensitively."""
        labels = self.labels.get(account_id, ())
        if not name.startswith("\\"):
            return next((l for l in labels if l["path"] == name), None)
        needle = name[1:].lower()
        names = (needle, needle + "s")
        for label in labels:
            if (label["role"] or "") in names:
                return label
        for label in labels:
            path = (label["path"] or "").lower()
            for prefix in ("[gmail]/", "[google mail]/"):
                if path.startswith(prefix) and path[len(prefix):] in names:
                    return label
        return None


def _msg(s: _Snapshot, mid: str) -> str:
    m = s.messages.get(mid)
    return f"message {mid} {m['subject']!r}" if m else f"message {mid}"


def _thread(s: _Snapshot, tid: str) -> str:
    t = s.threads.get(tid)
    return f"thread {tid} {t['subject']!r}" if t else f"thread {tid}"


# -- checks --------------------------------------------------------------------------------

def check_message_snapshot(s: _Snapshot) -> list:
    """Message.data.folders == {reported folder: OR of bits} over its rows, and
    Message.data.labels == sorted union of the rows' X-GM-LABELS (when it has rows)."""
    out = []
    for mid, m in s.messages.items():
        rows = s.rows.get(mid, [])
        want = {}
        labels = set()
        for r in rows:
            key = r["pendingFolderId"] or r["folderId"]
            want[key] = want.get(key, 0) | (UNREAD if r["unread"] else 0) | (STARRED if r["starred"] else 0) \
                | (DRAFT if r["draft"] else 0)
            labels.update(r["labels"])
        have = m["data"].get("folders")
        if not isinstance(have, dict) or {k: int(v) for k, v in have.items()} != want:
            out.append(f"{_msg(s, mid)}: folders snapshot {json.dumps(have, sort_keys=True)} != "
                       f"{json.dumps(want, sort_keys=True)} from MessageFolder rows {_rows(rows)}")
        if rows and sorted(m["data"].get("labels") or []) != sorted(labels):
            out.append(f"{_msg(s, mid)}: labels {sorted(m['data'].get('labels') or [])} != "
                       f"{sorted(labels)} from MessageFolder rows {_rows(rows)}")
    return out


def check_message_flags(s: _Snapshot) -> list:
    """Message unread / starred / draft (JSON and indexed columns) against its rows."""
    out = []
    drafts = {fid for fid, role in s.roles.items() if role == "drafts"}
    for mid, m in s.messages.items():
        rows = s.rows.get(mid, [])
        want = expected_message_flags(rows, drafts)
        for flag in ("unread", "starred", "draft"):
            data_value = bool(m["data"].get(flag, False))
            if bool(m[flag]) != data_value:
                out.append(f"{_msg(s, mid)}: column {flag}={bool(m[flag])} but data.{flag}={data_value}")
            if want is not None and data_value != want[flag]:
                out.append(f"{_msg(s, mid)}: {flag}={data_value}, expected {want[flag]} from MessageFolder rows {_rows(rows)}")
    return out


def check_thread_refcounts(s: _Snapshot) -> list:
    """Thread folder / label refcounts and counters against its messages' snapshots
    (Thread::applyMessageAttributeChanges summed over every message of the thread)."""
    out = []
    by_thread = defaultdict(list)
    for mid, m in s.messages.items():
        tid = m["data"].get("threadId") or ""
        if tid:
            by_thread[tid].append(m)
            if tid not in s.threads:
                out.append(f"{_msg(s, mid)}: threadId {tid} names no Thread row")

    for tid, t in s.threads.items():
        msgs = by_thread.get(tid, [])
        folders = defaultdict(lambda: [0, 0])
        labels = defaultdict(lambda: [0, 0])
        unread = starred = 0
        for m in msgs:
            d = m["data"]
            unread += bool(d.get("unread"))
            starred += bool(d.get("starred"))
            snapshot = d.get("folders") or {}
            for fid, bits in snapshot.items():
                folders[fid][0] += 1
                folders[fid][1] += 1 if int(bits) & UNREAD else 0
            label_unread = 1 if d.get("unread") and s.in_all_mail(snapshot) else 0
            for name in d.get("labels") or []:
                label = s.label_for_xgm_name(m["accountId"], name)
                if label is not None:
                    labels[label["id"]][0] += 1
                    labels[label["id"]][1] += label_unread

        where = f"{_thread(s, tid)} ({len(msgs)} messages)"
        for kind, want, have_list in (("folder", folders, t["data"].get("folders") or []),
                                      ("label", labels, t["data"].get("labels") or [])):
            have = {}
            for entry in have_list:
                if entry["id"] in have:
                    out.append(f"{where}: {kind} {s.folder_name(entry['id'])} listed twice")
                have[entry["id"]] = [entry.get("_refs"), entry.get("_u")]
            for cid in sorted(set(want) | set(have)):
                w, h = want.get(cid), have.get(cid)
                if w != h:
                    out.append(f"{where}: {kind} {s.folder_name(cid)} [_refs, _u] = {h}, expected {w} "
                               f"from messages {_thread_messages(msgs)}")
        if t["data"].get("unread") != unread or t["data"].get("starred") != starred:
            out.append(f"{where}: unread/starred = {t['data'].get('unread')}/{t['data'].get('starred')}, "
                       f"expected {unread}/{starred} from messages {_thread_messages(msgs)}")
        have_folders = t["data"].get("folders") or []
        in_all_mail = len(have_folders) > sum(1 for f in have_folders if f.get("role") in ("spam", "trash"))
        if t["data"].get("inAllMail") != in_all_mail:
            out.append(f"{where}: inAllMail = {t['data'].get('inAllMail')}, expected {in_all_mail} "
                       f"from its folders {[f.get('role') or f.get('path') for f in have_folders]}")
    return out


def check_thread_categories(s: _Snapshot) -> list:
    """ThreadCategory rows == the thread's folder + label ids, unread = (_u > 0), carrying the
    thread's inAllMail and timestamps (Thread::afterSave)."""
    out = []
    for tid in sorted(set(s.threads) | set(s.thread_categories)):
        t = s.threads.get(tid)
        have = s.thread_categories.get(tid, {})
        if t is None:
            out.append(f"ThreadCategory has rows {sorted(have)} for missing thread {tid}")
            continue
        d = t["data"]
        want = {e["id"]: e.get("_u", 0) > 0 for e in (d.get("folders") or []) + (d.get("labels") or [])}
        if set(want) != set(have):
            out.append(f"{_thread(s, tid)}: ThreadCategory values {sorted(have)}, expected {sorted(want)}")
        for cid in sorted(set(want) & set(have)):
            row = have[cid]
            if bool(row["unread"]) != want[cid]:
                out.append(f"{_thread(s, tid)}: ThreadCategory {s.folder_name(cid)} unread={bool(row['unread'])}, "
                           f"expected {want[cid]}")
            if bool(row["inAllMail"]) != bool(d.get("inAllMail")):
                out.append(f"{_thread(s, tid)}: ThreadCategory {s.folder_name(cid)} inAllMail={bool(row['inAllMail'])}, "
                           f"thread says {d.get('inAllMail')}")
            for col, key in (("lastMessageReceivedTimestamp", "lmrt"), ("lastMessageSentTimestamp", "lmst")):
                if float(row[col] or 0) != float(d.get(key) or 0):
                    out.append(f"{_thread(s, tid)}: ThreadCategory {s.folder_name(cid)} {col}={row[col]}, "
                               f"thread {key}={d.get(key)}")
    return out


def check_thread_counts(s: _Snapshot) -> list:
    """ThreadCounts (unread, total) == counts of ThreadCategory rows per category. Only
    categories with a ThreadCounts row are counted: Thread::afterSave UPDATEs, never inserts."""
    out = []
    totals = defaultdict(lambda: [0, 0])
    for cats in s.thread_categories.values():
        for cid, row in cats.items():
            totals[cid][0] += 1 if row["unread"] else 0
            totals[cid][1] += 1
    for cid, (unread, total) in sorted(s.thread_counts.items()):
        want = tuple(totals.get(cid, (0, 0)))
        if (unread, total) != want:
            out.append(f"ThreadCounts {s.folder_name(cid)}: (unread, total) = {(unread, total)}, "
                       f"expected {want} from ThreadCategory")
    return out


def check_orphans(s: _Snapshot) -> list:
    """MessageOrphan lists exactly the messages with no MessageFolder row: the end-of-pass
    sweep finds orphans only through it, so a missing record is a message that lives forever."""
    out = [f"{_msg(s, mid)}: no MessageFolder rows and no MessageOrphan record "
           f"(folders={json.dumps(m['data'].get('folders'))}, threadId={m['data'].get('threadId')})"
           for mid, m in s.messages.items() if not s.rows.get(mid) and mid not in s.orphans]
    out += [f"MessageOrphan record for {_msg(s, mid)} (since {since}), which "
            + (f"has MessageFolder rows {_rows(s.rows[mid])}" if s.rows.get(mid) else "does not exist")
            for mid, since in s.orphans.items() if s.rows.get(mid) or mid not in s.messages]
    return out


# -- contacts, calendars, plugin metadata ------------------------------------------------------

def _load_pim(conn: sqlite3.Connection) -> dict:
    out = {}
    for table in ("Contact", "ContactGroup", "ContactBook", "Calendar", "Event"):
        rows = {}
        if has_table(conn, table):
            for r in conn.execute(f"SELECT * FROM `{table}`"):
                r = dict(r)
                r["data"] = json.loads(r["data"]) if r.get("data") else {}
                rows[r["id"]] = r
        out[table] = rows
    out["join"] = [tuple(r) for r in conn.execute("SELECT id, value FROM ContactContactGroup")] \
        if has_table(conn, "ContactContactGroup") else []
    for table in ("ContactSearch", "EventSearch"):
        out[table] = [r[0] for r in conn.execute(f"SELECT content_id FROM `{table}`")] if has_table(conn, table) else []
    out["metadata_rows"] = [dict(r) for r in conn.execute("SELECT * FROM ModelPluginMetadata")] \
        if has_table(conn, "ModelPluginMetadata") else []
    return out


def _column_mismatches(table: str, row: dict, pairs) -> list:
    """pairs: (column, JSON key, default when the key is absent), as bindToQuery writes them."""
    out = []
    for col, key, default in pairs:
        want = row["data"].get(key, default)
        if isinstance(want, bool):
            want = int(want)
        have = row.get(col)
        if have != want and not (have in (None, "") and want in (None, "")):
            out.append(f"{table} {row['id']}: column {col} = {have!r}, data.{key} = {want!r}")
    return out


def check_contacts(s: _Snapshot) -> list:
    """Contact columns == its JSON (Contact::bindToQuery); one ContactSearch row per contact
    and none for a removed one (afterSave / afterRemove); a non-mail contact's book exists;
    and, between a contact and a group that both exist, `gis` names the group exactly when a
    ContactContactGroup row joins them (ContactGroup::syncMembers writes both).

    Dangling references are not checked: in C++ removing a group (ContactGroup::afterRemove)
    deletes its join rows but leaves the members' `gis`, and removing a contact
    (Contact::afterRemove) leaves its join rows, so a faithful port keeps both."""
    p = s.pim
    out = []
    for c in p["Contact"].values():
        out += _column_mismatches("Contact", c, [("email", "email", ""), ("hidden", "h", False), ("source", "s", "mail"),
                                                 ("etag", "etag", ""), ("bookId", "bid", ""), ("accountId", "aid", None)])
        if c["data"].get("s", "mail") != "mail" and c["data"].get("bid") and c["data"]["bid"] not in p["ContactBook"]:
            out.append(f"Contact {c['id']} ({c['data'].get('email')}): book {c['data']['bid']} does not exist")
    search = defaultdict(int)
    for cid in p["ContactSearch"]:
        search[cid] += 1
    for cid in p["Contact"]:
        if search.get(cid, 0) != 1:
            out.append(f"Contact {cid}: {search.get(cid, 0)} ContactSearch rows, expected 1")
    for cid in search:
        if cid not in p["Contact"]:
            out.append(f"ContactSearch row for {cid}, which is not a Contact")
    groups = p["ContactGroup"]
    joined = defaultdict(set)
    for cid, gid in p["join"]:
        if gid in groups:
            joined[cid].add(gid)
    for c in p["Contact"].values():
        gis = {g for g in (c["data"].get("gis") or []) if g in groups}
        if gis != joined.get(c["id"], set()):
            out.append(f"Contact {c['id']} ({c['data'].get('email')}): gis {sorted(gis)} vs "
                       f"ContactContactGroup {sorted(joined.get(c['id'], set()))}")
    return out


def check_contact_groups(s: _Snapshot) -> list:
    """ContactGroup columns == its JSON (ContactGroup::bindToQuery); its book exists."""
    p = s.pim
    out = []
    for g in p["ContactGroup"].values():
        out += _column_mismatches("ContactGroup", g, [("name", "name", ""), ("bookId", "bid", ""), ("accountId", "aid", None)])
        if g["data"].get("bid") and g["data"]["bid"] not in p["ContactBook"]:
            out.append(f"ContactGroup {g['id']} ({g['data'].get('name')!r}): book {g['data']['bid']} does not exist")
    return out


def check_events(s: _Snapshot) -> list:
    """Event columns == its JSON (Event::bindToQuery); its calendar exists; at most one row
    per (calendar, UID, RECURRENCE-ID), the identity MailUtils::idForEvent hashes; EventSearch
    has at most one row per event and none for a removed one."""
    p = s.pim
    out = []
    keys = defaultdict(list)
    for e in p["Event"].values():
        out += _column_mismatches("Event", e, [("etag", "etag", ""), ("calendarId", "cid", None), ("icsuid", "icsuid", None),
                                               ("recurrenceId", "rid", ""), ("recurrenceStart", "rs", None),
                                               ("recurrenceEnd", "re", None), ("accountId", "aid", None)])
        if e["data"].get("cid") not in p["Calendar"]:
            out.append(f"Event {e['id']} ({e['data'].get('icsuid')}): calendar {e['data'].get('cid')} does not exist")
        keys[(e["data"].get("cid"), e["data"].get("icsuid"), e["data"].get("rid", ""))].append(e["id"])
    for (cid, uid, rid), ids in keys.items():
        if len(ids) > 1:
            out.append(f"{len(ids)} Event rows for {uid}{' ' + rid if rid else ''} in calendar {cid}: {ids}")
    search = defaultdict(int)
    for eid in p["EventSearch"]:
        search[eid] += 1
    for eid, n in search.items():
        if eid not in p["Event"]:
            out.append(f"EventSearch row for {eid}, which is not an Event")
        elif n > 1:
            out.append(f"Event {eid}: {n} EventSearch rows")
    return out


def check_calendars(s: _Snapshot) -> list:
    """Calendar.accountId == data.aid, and no two calendars of an account share a path."""
    out = []
    paths = defaultdict(list)
    for c in s.pim["Calendar"].values():
        out += _column_mismatches("Calendar", c, [("accountId", "aid", None)])
        paths[(c.get("accountId"), c["data"].get("path"))].append(c["id"])
    for (aid, path), ids in paths.items():
        if len(ids) > 1:
            out.append(f"{len(ids)} Calendar rows for {path} in account {aid}: {ids}")
    return out


def check_plugin_metadata(s: _Snapshot) -> list:
    """ModelPluginMetadata == the non-empty metadata entries of every Message and Thread,
    with `expiration` = value.expiration when it is a number (MailModel::afterSave); the
    expiration worker reads only this table, so a missing row is a snooze that never wakes."""
    want = {}
    for table, models in (("Message", s.messages), ("Thread", s.threads)):
        for m in models.values():
            for entry in m["data"].get("metadata") or []:
                value = entry.get("value")
                # nlohmann size() == 0: only null, {} and [] are skipped; false, 0 and ""
                # still get a row in MailModel::afterSave.
                if value is None or (isinstance(value, (dict, list)) and not value):
                    continue
                exp = value.get("expiration") if isinstance(value, dict) else None
                exp = exp if isinstance(exp, (int, float)) and not isinstance(exp, bool) else None
                want[(m["id"], entry.get("pluginId"))] = (table, None if exp is None else int(exp))
    have = {}
    out = []
    for r in s.pim["metadata_rows"]:
        exp = r["expiration"]
        have[(r["id"], r["value"])] = (r["objectType"], None if exp is None else int(exp))
    for key, (table, exp) in want.items():
        if key not in have:
            out.append(f"{table} {key[0]} carries {key[1]} metadata but has no ModelPluginMetadata row")
        elif have[key] != (table, exp):
            out.append(f"ModelPluginMetadata ({key[0]}, {key[1]}): (objectType, expiration) = {have[key]}, "
                       f"expected {(table, exp)}")
    for key, (table, _) in have.items():
        if key not in want:
            out.append(f"ModelPluginMetadata ({key[0]}, {key[1]}, {table}) has no metadata entry on the model")
    return out


CHECKS = {
    "message_snapshot": check_message_snapshot,
    "message_flags": check_message_flags,
    "thread_refcounts": check_thread_refcounts,
    "thread_categories": check_thread_categories,
    "thread_counts": check_thread_counts,
    "orphans": check_orphans,
    "contacts": check_contacts,
    "contact_groups": check_contact_groups,
    "events": check_events,
    "calendars": check_calendars,
    "plugin_metadata": check_plugin_metadata,
}


def check(conn: sqlite3.Connection, skip=()) -> list:
    unknown = set(skip) - set(CHECKS)
    if unknown:
        raise ValueError(f"unknown invariants {sorted(unknown)}; known: {sorted(CHECKS)}")
    if not has_table(conn, "MessageFolder"):
        return []   # a pre-placements database: nothing here applies
    conn.execute("BEGIN")   # one snapshot across every table, even if the engine commits meanwhile
    try:
        s = _Snapshot(conn)
        s.pim = _load_pim(conn)
    finally:
        conn.execute("ROLLBACK")
    out = []
    for name, fn in CHECKS.items():
        if name in skip:
            continue
        found = fn(s)
        out += [f"[{name}] {v}" for v in found[:MAX_REPORTED_PER_CHECK]]
        if len(found) > MAX_REPORTED_PER_CHECK:
            out.append(f"[{name}] ... and {len(found) - MAX_REPORTED_PER_CHECK} more")
    return out


def _rows(rows) -> str:
    return "[" + ", ".join(
        f"{{folder={r['folderId']} uid={r['remoteUID']} u={r['unread']} s={r['starred']} d={r['draft']}"
        + (f" pending={r['pendingFolderId']}" if r["pendingFolderId"] else "")
        + (f" labels={r['labels']}" if r["labels"] else "") + "}"
        for r in rows) + "]"


def _thread_messages(msgs) -> str:
    return "[" + ", ".join(
        f"{m['id'][:12]} u={bool(m['data'].get('unread'))} folders={json.dumps(m['data'].get('folders'), sort_keys=True)}"
        + (f" labels={m['data'].get('labels')}" if m["data"].get("labels") else "")
        for m in msgs[:6]) + (", ..." if len(msgs) > 6 else "") + "]"
