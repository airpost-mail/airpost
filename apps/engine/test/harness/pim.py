"""
Scenario vocabulary for everything that is not mail placements: contacts, contact groups,
calendars and events (DAV servers, the fake People API), plugin metadata and its expiration,
and the engine's database and files as a whole (legacy databases, generic row checks).
ScenarioRun mixes this in; test/README.md documents the vocabulary.

Model selectors. Expectations and waits address rows by matching their JSON: `{email:
alice@example.org}` matches a Contact whose `data.email` is that, falling back to the
table's indexed column of that name when the JSON has no such key. A value may be a scalar
(equality), `{re: pattern}`, `{present: bool}`, or `{in: [values]}`.
"""
import json
import re
import time
import urllib.parse
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Optional

from . import davgen

MODEL_TABLES = {"Message", "Thread", "Folder", "Label", "Contact", "ContactGroup", "ContactBook", "Calendar",
                "Event", "File", "Task"}
# DAVWorker getCalendarSyncRange: 12 months back, 18 months ahead.
WINDOW_PAST = timedelta(days=365)
WINDOW_FUTURE = timedelta(days=548)


class PimError(AssertionError):
    pass


def norm_href(href: str) -> str:
    """Path part, percent-decoded, without a trailing slash: DAVWorker normalizeHref, so an
    absolute URL and a server-relative href name the same resource."""
    path = urllib.parse.urlsplit(href or "").path or (href or "")
    return urllib.parse.unquote(path).rstrip("/")


def norm_etag(etag: str) -> str:
    etag = (etag or "").strip()
    if etag.startswith("W/"):
        etag = etag[2:]
    return etag.strip('"')


def lookup(row: dict, key: str):
    """`key` as a dotted path into the row's JSON, else the column of that name."""
    cur = row.get("data") or {}
    found = True
    for part in key.split("."):
        if isinstance(cur, dict) and part in cur:
            cur = cur[part]
        elif isinstance(cur, list) and part.isdigit() and int(part) < len(cur):
            cur = cur[int(part)]
        else:
            found = False
            break
    if found:
        return True, cur
    if key in row and key != "data":
        return True, row[key]
    return False, None


def value_matches(have_it: bool, value, want) -> bool:
    if isinstance(want, dict) and set(want) <= {"re", "present", "in", "not"}:
        if "present" in want and bool(have_it) != bool(want["present"]):
            return False
        if "re" in want and not (have_it and re.search(want["re"], str(value))):
            return False
        if "in" in want and not (have_it and value in want["in"]):
            return False
        if "not" in want and have_it and value == want["not"]:
            return False
        return True
    if not have_it:
        return want is None
    if isinstance(want, bool):
        return isinstance(value, (bool, int)) and bool(value) == want
    if isinstance(want, (int, float)) and isinstance(value, (int, float)):
        return value == want
    return value == want or str(value) == str(want)


def row_matches(row: dict, where: dict) -> bool:
    return all(value_matches(*lookup(row, k), v) for k, v in (where or {}).items())


def load_rows(conn, table: str, account_id: Optional[str] = None) -> list:
    if table not in MODEL_TABLES:
        raise PimError(f"{table} is not a model table; known: {sorted(MODEL_TABLES)}")
    rows = []
    for r in conn.execute(f"SELECT * FROM `{table}`"):
        d = dict(r)
        try:
            d["data"] = json.loads(d["data"]) if d.get("data") else {}
        except (TypeError, ValueError):
            d["data"] = {}
        if account_id and d.get("accountId") not in (None, account_id):
            continue
        rows.append(d)
    return rows


def describe_row(table: str, row: dict) -> str:
    d = row.get("data") or {}
    keys = {"Contact": ("email", "name", "s", "h"), "Event": ("icsuid", "rid", "etag", "href"),
            "Calendar": ("path", "name"), "ContactGroup": ("name", "grn"), "Task": ("__cls", "status")}
    shown = {k: d.get(k) for k in keys.get(table, ("id",)) if k in d}
    return f"{table} {row.get('id')} {json.dumps(shown, sort_keys=True)}"


def check_models(conn, spec: dict, account_id: str) -> list:
    """{Table: {where, count | {min, max}, match: [selectors], absent: [selectors]}}"""
    out = []
    for table, want in spec.items():
        want = want or {}
        rows = [r for r in load_rows(conn, table, account_id) if row_matches(r, want.get("where"))]
        if "count" in want:
            c = want["count"]
            lo, hi = (c, c) if isinstance(c, int) else (c.get("min", 0), c.get("max"))
            if len(rows) < lo or (hi is not None and len(rows) > hi):
                out.append(f"{table}{' where ' + json.dumps(want['where']) if want.get('where') else ''}: "
                           f"{len(rows)} rows, expected {c}: " + "; ".join(describe_row(table, r) for r in rows[:6]))
        for sel in want.get("match") or []:
            hits = [r for r in rows if row_matches(r, sel)]
            if not hits:
                out.append(f"{table}: no row matches {json.dumps(sel)}; rows: "
                           + "; ".join(describe_row(table, r) for r in rows[:8]))
        for sel in want.get("absent") or []:
            hits = [r for r in rows if row_matches(r, sel)]
            if hits:
                out.append(f"{table}: {len(hits)} rows match {json.dumps(sel)}, expected none: {describe_row(table, hits[0])}")
    return out


def _in_window(ics: str, now: datetime) -> bool:
    """Whether DAVWorker would hold a row for some VEVENT of this resource: any instance
    overlapping 12 months back .. 18 months ahead. Recurring events count as overlapping."""
    for line in davgen.unfold(ics):
        name, params, value = davgen.prop(line)
        if name == "RRULE":
            return True
    for v in davgen.vevents(ics):
        start = v["dtstart"]
        try:
            dt = datetime.strptime(start[:8], "%Y%m%d").replace(tzinfo=timezone.utc)
        except ValueError:
            return True
        if now - WINDOW_PAST <= dt <= now + WINDOW_FUTURE:
            return True
    return False


def dav_matches_db(conn, truth: dict, account_id: str, check_window: bool = True) -> list:
    """Calendars, events, CardDAV contacts and groups locally == on the DAV server."""
    out = []
    now = datetime.now(timezone.utc)
    cal_rows = load_rows(conn, "Calendar", account_id)
    by_path = {norm_href(r["data"].get("path", "")): r for r in cal_rows}
    events = load_rows(conn, "Event", account_id)
    for href, cal in truth["calendars"].items():
        row = by_path.get(norm_href(href))
        if row is None:
            out.append(f"calendar {href} ({cal['name']!r}) is on the server but not in Calendar "
                       f"(have {sorted(by_path)})")
            continue
        if cal["name"] and row["data"].get("name") != cal["name"]:
            out.append(f"calendar {href}: name {row['data'].get('name')!r} locally vs {cal['name']!r} on the server")
        cid = row["id"]
        local = [e for e in events if (e["data"].get("cid") or e.get("calendarId")) == cid]
        expected = {}   # (uid, recurrence?) -> (href, etag)
        for ihref, item in cal["items"].items():
            if check_window and not _in_window(item["data"], now):
                continue
            for v in davgen.vevents(item["data"]):
                if not v["dtstart"]:
                    continue
                expected.setdefault((v["uid"], bool(v["recurrence_id"])), []).append((ihref, item["etag"], v))
        have = {}
        for e in local:
            have.setdefault((e["data"].get("icsuid") or e.get("icsuid"), bool(e["data"].get("rid") or e.get("recurrenceId"))), []).append(e)
        for key, items in expected.items():
            rows = have.get(key, [])
            uid, is_override = key
            label = f"event {uid}{' (an override)' if is_override else ''} in {href}"
            if len(rows) != len(items):
                out.append(f"{label}: {len(items)} on the server, {len(rows)} Event rows locally")
                continue
            for (ihref, etag, v), e in zip(items, rows):
                if norm_etag(e["data"].get("etag") or e.get("etag")) != norm_etag(etag):
                    out.append(f"{label}: etag {e['data'].get('etag')!r} locally vs {etag!r} on the server")
                if norm_href(e["data"].get("href", "")) != norm_href(ihref):
                    out.append(f"{label}: href {e['data'].get('href')!r} locally vs {ihref!r} on the server")
                ics = e["data"].get("ics") or ""
                if v["summary"] and f"SUMMARY:{v['summary']}" not in "\n".join(davgen.unfold(ics)):
                    out.append(f"{label}: the stored ICS does not carry the server's SUMMARY {v['summary']!r}")
        for key, rows in have.items():
            if key not in expected:
                out.append(f"event {key[0]}{' (an override)' if key[1] else ''}: {len(rows)} Event rows in {href} "
                           f"for nothing on the server")
    known_cids = {r["id"] for r in cal_rows if norm_href(r["data"].get("path", "")) in {norm_href(h) for h in truth["calendars"]}}
    for r in cal_rows:
        if r["id"] not in known_cids:
            out.append(f"Calendar {r['data'].get('path')!r} ({r['data'].get('name')!r}) is not on the server")
    stray = [e for e in events if (e["data"].get("cid") or e.get("calendarId")) not in known_cids]
    if stray:
        out.append(f"{len(stray)} Event rows in calendars the server does not list: {describe_row('Event', stray[0])}")

    contacts = {r["id"]: r for r in load_rows(conn, "Contact", account_id)
                if (r["data"].get("s") or r.get("source")) == "carddav"}
    groups = {r["id"]: r for r in load_rows(conn, "ContactGroup", account_id)}
    members = {}
    for cid, gid in conn.execute("SELECT id, value FROM ContactContactGroup"):
        members.setdefault(gid, set()).add(cid)
    books = load_rows(conn, "ContactBook", account_id)
    seen_contacts, seen_groups = set(), set()
    for ab_href, ab in truth["addressbooks"].items():
        if not any((b["data"].get("source") == "carddav" and norm_href(b["data"].get("url", "")).endswith(norm_href(ab_href)))
                   for b in books):
            out.append(f"address book {ab_href}: no ContactBook with source carddav and that url "
                       f"(have {[(b['data'].get('source'), b['data'].get('url')) for b in books]})")
        cards = {}
        for ihref, item in ab["items"].items():
            f = davgen.vcard_fields(item["data"])
            cards[f["uid"]] = (ihref, item["etag"], f)
        for uid, (ihref, etag, f) in cards.items():
            row = contacts.get(uid)
            label = f"{'group' if f['is_group'] else 'contact'} card {uid} ({f['fn']!r})"
            if row is None:
                out.append(f"{label} is on the server but not in Contact")
                continue
            seen_contacts.add(uid)
            d = row["data"]
            if norm_etag(d.get("etag")) != norm_etag(etag):
                out.append(f"{label}: etag {d.get('etag')!r} locally vs {etag!r} on the server")
            if norm_href((d.get("info") or {}).get("href", "")) != norm_href(ihref):
                out.append(f"{label}: info.href {(d.get('info') or {}).get('href')!r} locally vs {ihref!r}")
            want_email = f["emails"][0] if f["emails"] else ""
            if not f["is_group"] and (d.get("email") or "") != want_email:
                out.append(f"{label}: email {d.get('email')!r} locally vs {want_email!r} on the server")
            if (d.get("name") or "") != (f["fn"] or ""):
                out.append(f"{label}: name {d.get('name')!r} locally vs FN {f['fn']!r} on the server")
            if bool(d.get("h", False)) != f["is_group"]:
                out.append(f"{label}: hidden is {bool(d.get('h', False))}, expected {f['is_group']} "
                           "(group cards are hidden contacts)")
            if f["is_group"]:
                g = groups.get(uid)
                if g is None:
                    out.append(f"{label}: no ContactGroup row with the card's UID")
                    continue
                seen_groups.add(uid)
                if g["data"].get("name") != f["fn"]:
                    out.append(f"{label}: ContactGroup name {g['data'].get('name')!r} vs FN {f['fn']!r}")
                want_members = {m for m in f["members"] if m in cards and not cards[m][2]["is_group"]}
                got = members.get(uid, set()) & set(contacts)
                if got != want_members:
                    out.append(f"{label}: members {sorted(got)} locally vs {sorted(want_members)} on the server")
    for cid, row in contacts.items():
        if cid not in seen_contacts:
            out.append(f"{describe_row('Contact', row)} is a CardDAV contact the server does not have")
    for gid, row in groups.items():
        # Groups with a Google resource name belong to the People API, not this server.
        if gid not in seen_groups and not row["data"].get("grn"):
            out.append(f"{describe_row('ContactGroup', row)} has no group card on the server")
    return out


def people_matches_db(conn, state, account_id: str) -> list:
    """Contacts and user contact groups locally == in the fake People API."""
    out = []
    contacts = [r for r in load_rows(conn, "Contact", account_id) if (r["data"].get("s") or r.get("source")) == "gpeople"]
    by_grn = {}
    for r in contacts:
        by_grn.setdefault(r["data"].get("grn", ""), []).append(r)
    with state.lock:
        live = {p.resource_name: p for p in state.live_people()}
        user_groups = {g.resource_name: g for g in state.live_groups(user_only=True)}
        members = {rn: set(state.members(g)) for rn, g in user_groups.items()}
    for rn, p in live.items():
        rows = by_grn.get(rn, [])
        if len(rows) != 1:
            out.append(f"person {rn} ({p.email}): {len(rows)} Contact rows locally, expected 1")
            continue
        d = rows[0]["data"]
        if (d.get("email") or "") != p.email:
            out.append(f"person {rn}: email {d.get('email')!r} locally vs {p.email!r}")
        if (d.get("name") or "") != p.name:
            out.append(f"person {rn}: name {d.get('name')!r} locally vs {p.name!r}")
        if d.get("h"):
            out.append(f"person {rn} ({p.email}) is hidden locally")
    for grn, rows in by_grn.items():
        if grn not in live:
            out.append(f"{len(rows)} gpeople Contact rows for {grn or '<no resourceName>'}, which the server does not have: "
                       f"{describe_row('Contact', rows[0])}")
    groups = load_rows(conn, "ContactGroup", account_id)
    g_by_grn = {}
    for g in groups:
        g_by_grn.setdefault(g["data"].get("grn", ""), []).append(g)
    join = {}
    for cid, gid in conn.execute("SELECT id, value FROM ContactContactGroup"):
        join.setdefault(gid, set()).add(cid)
    contact_grn = {r["id"]: r["data"].get("grn") for r in contacts}
    for rn, g in user_groups.items():
        rows = g_by_grn.get(rn, [])
        if len(rows) != 1:
            out.append(f"contact group {rn} ({g.name!r}): {len(rows)} ContactGroup rows locally, expected 1")
            continue
        if rows[0]["data"].get("name") != g.name:
            out.append(f"contact group {rn}: name {rows[0]['data'].get('name')!r} locally vs {g.name!r}")
        got = {contact_grn.get(cid) for cid in join.get(rows[0]["id"], set())}
        if got != members[rn]:
            out.append(f"contact group {g.name!r}: members {sorted(x or '?' for x in got)} locally vs {sorted(members[rn])}")
    for grn, rows in g_by_grn.items():
        if grn and grn not in user_groups:
            out.append(f"ContactGroup {rows[0]['data'].get('name')!r} ({grn}) is not a user group on the server")
    books = [b for b in load_rows(conn, "ContactBook", account_id) if b["data"].get("source") == "gpeople"]
    if len(books) != 1:
        out.append(f"{len(books)} ContactBook rows with source gpeople, expected 1 (how the client knows contact sync is on)")
    return out


def check_metadata(conn, entries: list, resolve) -> list:
    """[{threads|messages: selector, plugin, value: {subset}, expiration: true|false|n}]: the
    model carries that plugin's metadata in its JSON and in ModelPluginMetadata."""
    out = []
    for want in entries:
        table, ids = resolve(want)
        for mid in ids:
            row = conn.execute(f"SELECT data FROM `{table}` WHERE id = ?", (mid,)).fetchone()
            if row is None:
                out.append(f"{table} {mid} does not exist")
                continue
            data = json.loads(row[0])
            entry = next((m for m in data.get("metadata") or [] if m.get("pluginId") == want["plugin"]), None)
            if want.get("absent"):
                if entry and entry.get("value"):
                    out.append(f"{table} {mid} carries {want['plugin']} metadata {json.dumps(entry['value'])}, expected none")
                continue
            if entry is None:
                out.append(f"{table} {mid} has no {want['plugin']} metadata (plugins: "
                           f"{[m.get('pluginId') for m in data.get('metadata') or []]})")
                continue
            for k, v in (want.get("value") or {}).items():
                if entry["value"].get(k) != v:
                    out.append(f"{table} {mid} {want['plugin']}.{k} = {entry['value'].get(k)!r}, expected {v!r}")
            pm = conn.execute("SELECT expiration FROM ModelPluginMetadata WHERE id = ? AND value = ? AND objectType = ?",
                              (mid, want["plugin"], table)).fetchall()
            if len(pm) != 1:
                out.append(f"{table} {mid}: {len(pm)} ModelPluginMetadata rows for {want['plugin']}, expected 1")
            elif "expiration" in want:
                exp = pm[0][0]
                if want["expiration"] is True and exp is None:
                    out.append(f"{table} {mid} {want['plugin']}: ModelPluginMetadata.expiration is NULL")
                elif want["expiration"] is False and exp is not None:
                    out.append(f"{table} {mid} {want['plugin']}: ModelPluginMetadata.expiration is {exp}, expected NULL")
    return out


def check_metadata_expiration(deltas: list, entries: list, resolve) -> list:
    """[{threads|messages: selector, plugin, count: n | {min, max}}]: how many
    `metadata-expiration` deltas carried each model, and that none arrived before the
    expiration its metadata names (MetadataExpirationWorker fires at `expiration <= now`)."""
    out = []
    for want in entries:
        table, ids = resolve(want)
        for mid in ids:
            hits = [(d, m) for d in deltas if d.type == "metadata-expiration" and d.model_class == table
                    for m in d.models if m.get("id") == mid]
            c = want.get("count", {"min": 1})
            lo, hi = (c, c) if isinstance(c, int) else (c.get("min", 0), c.get("max"))
            if len(hits) < lo or (hi is not None and len(hits) > hi):
                out.append(f"{table} {mid}: {len(hits)} metadata-expiration deltas, expected {c}")
            for d, m in hits:
                entry = next((x for x in m.get("metadata") or [] if x.get("pluginId") == want.get("plugin")), None)
                if want.get("plugin") and entry is None:
                    out.append(f"{table} {mid}: the metadata-expiration delta carries no {want['plugin']} metadata")
                    continue
                exp = (entry or {}).get("value", {}).get("expiration")
                if isinstance(exp, (int, float)) and d.wall + 1 < exp:
                    out.append(f"{table} {mid}: metadata-expiration delta at {d.wall:.0f}, "
                               f"{exp - d.wall:.0f}s before its expiration {exp}")
    return out


def check_changed_since(deltas: list, since: float, spec: dict) -> list:
    """{Table: {key: path, where: selector, persist: [keys], unpersist: [keys]}}: exactly
    which models (by `key`, default id) were streamed since a mark, distinct per type. What
    proves a pass was incremental: an unchanged model is never re-sent."""
    out = []
    for table, want in spec.items():
        key = want.get("key", "id")
        for typ in ("persist", "unpersist"):
            if typ not in want:
                continue
            got = set()
            for d in deltas:
                if d.wall < since or d.model_class != table or d.type != typ:
                    continue
                for m in d.models:
                    row = {"data": m}
                    if want.get("where") and not row_matches(row, want["where"]):
                        continue
                    have, v = lookup(row, key)
                    got.add(str(v) if have else f"<no {key}>")
            expected = {str(x) for x in want[typ]}
            if got != expected:
                out.append(f"{table} {typ} since mark: {sorted(got)}, expected {sorted(expected)}"
                           + (f" (extra {sorted(got - expected)})" if got - expected else "")
                           + (f" (missing {sorted(expected - got)})" if expected - got else ""))
    return out


def check_config_files(config_dir: Path, spec: dict) -> list:
    """{present: [paths], absent: [paths], empty: [dirs]} relative to CONFIG_DIR_PATH; a
    directory listed under `empty` may also be missing."""
    out = []
    for rel in spec.get("present") or []:
        if not (config_dir / rel).exists():
            out.append(f"{rel} is missing from the config directory "
                       f"(has {sorted(p.name for p in config_dir.iterdir())})")
    for rel in spec.get("absent") or []:
        if (config_dir / rel).exists():
            out.append(f"{rel} exists in the config directory, expected it gone")
    for rel in spec.get("empty") or []:
        d = config_dir / rel
        if d.exists():
            left = sorted(str(p.relative_to(d)) for p in d.rglob("*") if p.is_file())
            if left:
                out.append(f"{rel}/ still holds {len(left)} files: {left[:5]}")
    return out


def check_sql(conn_for, queries: list) -> list:
    """[{db: main | legacy, query, equals: value | [rows]}] for what no model expectation
    covers: pragmas, `_State` keys, a moved legacy database."""
    out = []
    for q in queries:
        with conn_for(q.get("db", "main")) as c:
            try:
                rows = [tuple(r) for r in c.execute(q["query"]).fetchall()]
            except Exception as e:
                out.append(f"{q.get('db', 'main')}: {q['query']!r} failed: {e}")
                continue
        want = q.get("equals")
        got = rows[0][0] if len(rows) == 1 and len(rows[0]) == 1 and not isinstance(want, list) else [list(r) for r in rows]
        if got != want and str(got) != str(want):
            out.append(f"{q.get('db', 'main')}: {q['query']!r} returned {got!r}, expected {want!r}")
    return out


def resolve_now(value):
    """{now_plus: seconds} anywhere in a task -> epoch seconds, so expirations can be relative."""
    if isinstance(value, dict):
        if set(value) == {"now_plus"}:
            return int(time.time() + float(value["now_plus"]))
        return {k: resolve_now(v) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve_now(v) for v in value]
    return value
