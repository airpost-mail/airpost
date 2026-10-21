"""
A binary-neutral picture of what an engine left behind after a scenario: every table of its
database and the set of models it streamed, with engine ids replaced by keys both engines
agree on, so two runs of the same scenario on different binaries can be diffed
(compare by headerMessageId).

Keys: Message `M:<headerMessageId>` (`#n` when one DB holds several rows with that
Message-ID), Thread `t:M:<...>` (thread ids are `t:<first message id>`, Thread.cpp:23),
Folder `F:<path>`, Label `L:<path>`, File `File:<message>:<partId>:<filename>`, Contact
`C:<source>:<email or name>`, ContactGroup `G:<name>`, ContactBook `B:<url>`, Calendar
`Cal:<path or name>`, Event `E:<icsuid>|<recurrenceId>`, Task `Task:<__cls>#<n>` (queue order).
Opaque ids inside any string are rewritten to those keys.

Absolute times become `{"$t": seconds after t0, "$e": seconds after the engine started}`:
seeded mail is dated relative to mailgen.BASE (t0), while what the harness or the engine
stamps during the run (draft dates, expirations) is relative to the launch, and server
setup takes a different time on every run. Fields that are the engine's or the server's
own clock (scans, vacuum, body fetches, UIDVALIDITY) are dropped: they are timing, not
behaviour.
"""
import json
import re
import sqlite3
from pathlib import Path

EPOCH_LO, EPOCH_HI = 1_500_000_000, 2_500_000_000
# Leaf paths (dotted, after flattening) that carry no behaviour: save counters, FTS rowids,
# the engine's wall clock.
VOLATILE = re.compile(
    r"(^|\.)(v|version|searchRowId|rowid|_sa|lastDeep|lastShallow|lastCleanup|fetchedAt|uidvalidity)$")
VOLATILE_ROWS = {("_State", "VACUUM_TIME")}
VOLATILE_COLUMNS = {("MessageOrphan", "since")}
OPAQUE_ID = re.compile(r"^[1-9A-HJ-NP-Za-km-z]{38,46}$|^[A-Za-z0-9]{40}$")
DELETED_ID = re.compile(r"deleted-[A-Za-z0-9]{10,}")
SKIP_TABLES = re.compile(r"^(sqlite_|.*Search_(data|idx|content|docsize|config)$|Account$|_?Message_v\d+$)")


def _rows(conn, table):
    try:
        cur = conn.execute(f"SELECT * FROM `{table}`")
    except sqlite3.DatabaseError:
        return []
    cols = [c[0] for c in cur.description]
    out = []
    for r in cur.fetchall():
        row = {}
        for c, v in zip(cols, r):
            if isinstance(v, bytes):
                v = v.decode("utf-8", "replace")
            if c in ("data", "value") and isinstance(v, str) and v[:1] in "{[":
                try:
                    v = json.loads(v)
                except ValueError:
                    pass
            row[c] = v
        out.append(row)
    return out


class IdMap:
    def __init__(self):
        self.ids = {}

    def add(self, engine_id, key):
        if engine_id and isinstance(engine_id, str) and engine_id not in self.ids:
            self.ids[engine_id] = key

    def compile(self):
        # Only opaque ids are rewritten inside strings; a contact id that is its email stays.
        opaque = sorted((i for i in self.ids if len(i) >= 16 and "@" not in i and i != self.ids[i]),
                        key=len, reverse=True)
        self.rx = re.compile("|".join(re.escape(i) for i in opaque)) if opaque else None

    def sub(self, s):
        if self.rx:
            s = self.rx.sub(lambda m: self.ids[m.group(0)], s)
        s = DELETED_ID.sub("deleted-*", s)
        if OPAQUE_ID.match(s):
            return "<unmapped-id>"
        return s


def _message_key_order(m, placements):
    return (m.get("subject") or "", sorted(placements.get(m["id"], [])))


def build_idmap(tables: dict, deltas: list) -> IdMap:
    idm = IdMap()
    for f in tables.get("Folder", []):
        idm.add(f["id"], f"F:{f['path']}")
    for f in tables.get("Label", []):
        idm.add(f["id"], f"L:{f['path']}")
    folder_key = dict(idm.ids)
    placements = {}
    for p in tables.get("MessageFolder", []):
        placements.setdefault(p["messageId"], []).append((folder_key.get(p["folderId"], p["folderId"]), p["remoteUID"]))
    by_hmid = {}
    for m in tables.get("Message", []):
        by_hmid.setdefault(m.get("headerMessageId") or "", []).append(m)
    for hmid, group in by_hmid.items():
        if len(group) == 1:
            idm.add(group[0]["id"], f"M:{hmid}")
        else:
            for n, m in enumerate(sorted(group, key=lambda m: _message_key_order(m, placements))):
                idm.add(m["id"], f"M:{hmid}#{n}")
    live = set(by_hmid)
    for d in deltas:
        if d["class"] == "Message":
            for m in d["models"]:
                h = m.get("hMsgId") or ""
                idm.add(m.get("id"), f"M:{h}" if h not in live else f"M:{h}#gone")
    for f in tables.get("File", []):
        data = f.get("data") if isinstance(f.get("data"), dict) else {}
        msg = idm.ids.get(data.get("messageId"), data.get("messageId"))
        idm.add(f["id"], f"File:{msg}:{data.get('partId')}:{data.get('filename') or f.get('filename')}")
    for c in tables.get("Contact", []):
        data = c.get("data") if isinstance(c.get("data"), dict) else {}
        idm.add(c["id"], f"C:{c.get('source') or data.get('s')}:{c.get('email') or data.get('name')}")
    for g in tables.get("ContactGroup", []):
        idm.add(g["id"], f"G:{g.get('name')}")
    for b in tables.get("ContactBook", []):
        data = b.get("data") if isinstance(b.get("data"), dict) else {}
        idm.add(b["id"], f"B:{data.get('url') or data.get('source') or b['id']}")
    for c in tables.get("Calendar", []):
        data = c.get("data") if isinstance(c.get("data"), dict) else {}
        idm.add(c["id"], f"Cal:{data.get('path') or data.get('url') or data.get('name')}")
    for e in tables.get("Event", []):
        data = e.get("data") if isinstance(e.get("data"), dict) else {}
        idm.add(e["id"], f"E:{e.get('icsuid') or data.get('icsuid')}|{e.get('recurrenceId') or data.get('rid') or ''}")
    counts = {}
    for t in tables.get("Task", []):
        data = t.get("data") if isinstance(t.get("data"), dict) else {}
        cls = data.get("__cls", "?")
        counts[cls] = counts.get(cls, 0) + 1
        idm.add(t["id"], f"Task:{cls}#{counts[cls]}")
    # Models only seen on the delta stream (deleted before the end).
    for d in deltas:
        for m in d["models"]:
            mid = m.get("id")
            if d["class"] == "Folder":
                idm.add(mid, f"F:{m.get('path')}")
            elif d["class"] == "Label":
                idm.add(mid, f"L:{m.get('path')}")
            elif d["class"] == "Event":
                idm.add(mid, f"E:{m.get('icsuid')}|{m.get('rid') or ''}")
            elif d["class"] == "ContactGroup":
                idm.add(mid, f"G:{m.get('name')}")
            elif d["class"] == "Contact":
                idm.add(mid, f"C:{m.get('s')}:{m.get('email') or m.get('name')}")
            elif d["class"] == "Task" and mid not in idm.ids:
                cls = m.get("__cls", "?")
                counts[cls] = counts.get(cls, 0) + 1
                idm.add(mid, f"Task:{cls}#{counts[cls]}")
    idm.compile()
    return idm


def _norm(value, idm: IdMap, t0: tuple):
    if isinstance(value, str):
        return idm.sub(value)
    if isinstance(value, bool) or value is None:
        return value
    if isinstance(value, (int, float)):
        if EPOCH_LO * 1000 <= value <= EPOCH_HI * 1000:
            value = value / 1000
        if EPOCH_LO <= value <= EPOCH_HI:
            return {"$t": round(value - t0[0]), "$e": round(value - t0[1])}
        return value
    if isinstance(value, list):
        return [_norm(v, idm, t0) for v in value]
    if isinstance(value, dict):
        return {idm.sub(k): _norm(v, idm, t0) for k, v in value.items()}
    return value


def flatten(value, prefix="", out=None) -> dict:
    out = {} if out is None else out
    if isinstance(value, dict) and "$t" not in value:
        if not value:
            out[prefix] = {}
        for k, v in value.items():
            flatten(v, f"{prefix}.{k}" if prefix else k, out)
    elif isinstance(value, list):
        if not value:
            out[prefix] = []
        for i, v in enumerate(value):
            flatten(v, f"{prefix}[{i}]", out)
    else:
        out[prefix] = value
    return out


def _row_key(table, row, idm):
    k = lambda v: idm.sub(v) if isinstance(v, str) else v  # noqa: E731
    if table == "MessageFolder":
        return f"{k(row['messageId'])} @ {k(row['folderId'])}:{row['remoteUID']}"
    if table == "ThreadReference":
        return f"{k(row['threadId'])} <- {row['headerMessageId']}"
    if table in ("ThreadCategory", "ContactContactGroup"):
        return f"{k(row['id'])} in {k(row['value'])}"
    if table == "ModelPluginMetadata":
        return f"{k(row['id'])} / {row['value']}"
    if table == "DetatchedPluginMetadata":
        return f"{k(row['objectId'])} / {row['pluginId']}"
    if table == "ThreadCounts":
        return k(row["categoryId"])
    if table == "MessageOrphan":
        return k(row["messageId"])
    if table.endswith("Search"):
        return k(row.get("content_id"))
    if "id" in row:
        return k(row["id"])
    return json.dumps(row, sort_keys=True, default=str)


def snapshot(db_path: Path, deltas: list, t0: float, engine_t0: float) -> dict:
    """`deltas`: [{"class", "type", "models"}] in arrival order; `t0` the mailgen anchor,
    `engine_t0` the wall time of the first launch."""
    tables = {}
    if Path(db_path).exists():
        conn = sqlite3.connect(str(db_path), timeout=10)
        try:
            names = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")]
            for name in names:
                if not SKIP_TABLES.match(name):
                    tables[name] = _rows(conn, name)
        finally:
            conn.close()
    idm = build_idmap(tables, deltas)
    db = {}
    for table, rows in tables.items():
        out = {}
        for row in rows:
            if (table, row.get("id")) in VOLATILE_ROWS:
                continue
            key = _row_key(table, row, idm)
            flat = {f: v for f, v in flatten(_norm(row, idm, (t0, engine_t0))).items()
                    if not VOLATILE.search(f) and (table, f) not in VOLATILE_COLUMNS}
            if key in out:
                key = f"{key} (dup {sum(1 for x in out if x.startswith(key))})"
            out[key] = flat
        db[table] = out
    seen = set()
    for d in deltas:
        if not d["models"]:
            seen.add(f"{d['class']} {d['type']} *")
        for m in d["models"]:
            mid = m.get("id") if isinstance(m, dict) else None
            seen.add(f"{d['class']} {d['type']} {idm.sub(mid) if isinstance(mid, str) else '*'}")
    return {"db": db, "deltas": sorted(seen)}
