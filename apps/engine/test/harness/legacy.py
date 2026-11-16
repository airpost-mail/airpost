"""
A database as the C++ engine leaves it, for the scenarios that hand one to the Rust engine
(fresh-db-replaces-legacy, import-legacy-carries-metadata).
It is written at `airpost.db`, where the desktop's Import from Mailspring puts
Mailspring's `edgehill.db` (or at `edgehill.db` for the C++ engine itself). The Rust engine
never reads such a database in place: `--mode migrate` moves `airpost.db` and its `-wal`/`-shm` to
`airpost.legacy.db*`, creates a fresh V12 database and clears `files/`; `--mode import-legacy`
later reads the moved copy.

The schema is the V12 fixture the Rust store is checked against
(`crates/airpost-store/tests/fixtures/v12_schema.json`, the C++ fresh-database replay that
`apps/engine/tools/schema_check.py dump` writes). Rows are written the way the C++ models serialise
(`MailModel::toJSON`, `bindToQuery`), with C++-era ids: the Rust engine's message ids differ
by design (D4), so anything carried over has to be re-keyed by headerMessageId.

    legacy_db:
      wal: true                 # leave the writes in the -wal file, as a killed engine does
      folders: {Drafts: drafts, INBOX: inbox}
      messages:
        m1: {mailbox: INBOX, uid: 2, thread: t1}           # a message the server also has
        d1: {folder: Drafts, draft: {hMsgId: local-1@example.test, subject: Plans,
             body: "<p>draft</p>"}, files: [{filename: notes.txt, content: "..."}]}
        d2: {draft: {hMsgId: reply-1@example.test, reply_to: m1}}  # a reply in m1's thread
        d3: {draft: {hMsgId: reply-2@example.test, rthMsgId: gone@example.net}}  # a reply to mail not here
      metadata:
        - {model: t1, plugin: thread-snooze, value: {expiration: 1900000000}}
      state: {cursor-c0ffee-harness: "42"}
      files: {"ab/cd/abcdef/stray.bin": "x"}               # extra files under files/
"""
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path
from typing import Callable, Optional

ENGINE_DIR = Path(__file__).resolve().parents[2]
REPO_ROOT = ENGINE_DIR.parents[1]
V12_FIXTURE = REPO_ROOT / "crates" / "airpost-store" / "tests" / "fixtures" / "v12_schema.json"

DATABASE = "airpost.db"
LEGACY_DATABASE = "airpost.legacy.db"
FTS_SHADOW_SUFFIXES = ("_config", "_content", "_data", "_docsize", "_idx")

# ResolveMessage(mailbox, uid) -> {"message_id", "subject", "date"} for a message on the server.
ResolveMessage = Callable[[str, int], dict]


def cpp_schema_statements() -> list:
    """The C++ engine's fresh-database DDL, in execution order."""
    entries = json.loads(V12_FIXTURE.read_text())
    virtual = {e["name"] for e in entries if e["sql"] and e["sql"].startswith("CREATE VIRTUAL TABLE")}
    shadow = {v + s for v in virtual for s in FTS_SHADOW_SUFFIXES}
    order = {"table": 0, "index": 1, "trigger": 2, "view": 3}
    return [e["sql"] for e in sorted(entries, key=lambda e: order.get(e["type"], 9))
            if e["sql"] and e["name"] not in shadow]


def _hash_id(*parts: str) -> str:
    return hashlib.sha256("-".join(parts).encode()).hexdigest()[:40]


class _Writer:
    def __init__(self, conn: sqlite3.Connection, account_id: str, resolve: Optional[ResolveMessage]):
        self.c = conn
        self.aid = account_id
        self.resolve = resolve
        self.folder_ids: dict = {}      # path -> id
        self.message_ids: dict = {}     # scenario key -> id
        self.thread_ids: dict = {}      # scenario key -> id
        self.message_threads: dict = {} # message key -> thread key
        self.messages: dict = {}        # id -> data
        self.threads: dict = {}         # id -> data

    def folder(self, path: str, role: str = "") -> str:
        if path in self.folder_ids:
            return self.folder_ids[path]
        fid = _hash_id(self.aid, path)
        data = {"id": fid, "aid": self.aid, "v": 1, "__cls": "Folder", "path": path, "role": role,
                "localStatus": {"uidvalidity": 1, "uidnext": 1, "syncedMinUID": 1, "busy": False}}
        self.c.execute("INSERT INTO Folder (id, accountId, version, data, path, role) VALUES (?,?,?,?,?,?)",
                       (fid, self.aid, 1, json.dumps(data), path, role))
        self.folder_ids[path] = fid
        return fid

    def message(self, key: str, spec: dict):
        draft = spec.get("draft")
        if draft:
            hmid = draft["hMsgId"].strip("<>")
            subject = draft.get("subject", "")
            date = int(draft.get("date", time.time()))
            folder_path, uid = spec.get("folder", "Drafts"), 0
        else:
            if self.resolve is None:
                raise ValueError("legacy_db messages that name a server message need a running server")
            found = self.resolve(spec["mailbox"], int(spec["uid"]))
            hmid, subject, date = found["message_id"].strip("<>"), found.get("subject", ""), int(found.get("date", 0))
            folder_path, uid = spec["mailbox"], int(spec["uid"])
        mid = spec.get("id") or _hash_id("cpp", self.aid, hmid)
        fid = self.folder(folder_path, spec.get("role", ""))
        # The client saves a reply draft in the thread of the message it answers.
        reply_to = draft.get("reply_to") if draft else None
        tkey = spec.get("thread", self.message_threads[reply_to] if reply_to else f"thread-of-{key}")
        self.message_threads[key] = tkey
        tid = self.thread_ids.setdefault(tkey, "t:" + mid)
        unread = bool(spec.get("unread", False))
        bits = (1 if unread else 0) | (4 if draft else 0)
        data = {
            "id": mid, "aid": self.aid, "v": 1, "__cls": "Message", "hMsgId": hmid, "subject": subject,
            "date": date, "threadId": tid, "folders": {fid: bits}, "labels": [], "unread": unread,
            "starred": False, "draft": bool(draft), "metadata": [], "files": [],
            "from": draft.get("from", [{"name": "Harness Test", "email": "test@example.test"}]) if draft else [],
            "to": draft.get("to", []) if draft else [], "cc": [], "bcc": [], "replyTo": [],
            "snippet": "", "plaintext": bool(draft.get("plaintext", False)) if draft else False,
        }
        if reply_to:
            data["rthMsgId"] = self.messages[self.message_ids[reply_to]]["hMsgId"]
        elif draft and draft.get("rthMsgId"):
            data["rthMsgId"] = draft["rthMsgId"].strip("<>")
        for f in spec.get("files") or []:
            data["files"].append(self.file(mid, f))
        self.c.execute(
            "INSERT INTO Message (id, accountId, version, data, headerMessageId, subject, date, draft, unread, "
            "starred, threadId) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
            (mid, self.aid, 1, None, hmid, subject, date, int(bool(draft)), int(unread), 0, tid))
        self.c.execute("INSERT INTO MessageFolder (accountId, messageId, folderId, remoteUID, unread, starred, draft) "
                       "VALUES (?,?,?,?,?,?,?)", (self.aid, mid, fid, uid, int(unread), 0, int(bool(draft))))
        if draft and draft.get("body") is not None:
            self.c.execute("INSERT INTO MessageBody (id, value, fetchedAt) VALUES (?,?,?)",
                           (mid, draft["body"], int(time.time())))
        self.message_ids[key] = mid
        self.messages[mid] = data
        thread = self.threads.setdefault(tid, {
            "id": tid, "aid": self.aid, "v": 1, "__cls": "Thread", "subject": subject, "folders": [],
            "labels": [], "unread": 0, "starred": 0, "inAllMail": True, "metadata": [],
            "lmrt": date, "lmst": 0, "fmt": date, "lmt": date, "participants": [], "attachmentCount": 0,
        })
        self.thread_ids[tkey] = tid
        if not any(f["id"] == fid for f in thread["folders"]):
            thread["folders"].append({"id": fid, "path": folder_path, "role": spec.get("role", ""), "_refs": 1,
                                      "_u": int(unread)})

    def file(self, message_id: str, spec: dict) -> dict:
        fid = _hash_id("file", message_id, self.aid, spec["filename"])
        content = spec.get("content", "").encode()
        data = {"id": fid, "aid": self.aid, "v": 1, "__cls": "File", "messageId": message_id,
                "filename": spec["filename"], "contentType": spec.get("content_type", "text/plain"),
                "size": len(content), "partId": spec.get("part_id", "2"), "contentId": None}
        self.c.execute("INSERT INTO File (id, version, data, accountId, filename) VALUES (?,?,?,?,?)",
                       (fid, 1, json.dumps(data), self.aid, spec["filename"]))
        self.pending_files = getattr(self, "pending_files", {})
        lower = fid.lower()
        self.pending_files[f"{lower[0:2]}/{lower[2:4]}/{lower}/{spec['filename']}"] = content
        return data

    def metadata(self, spec: dict):
        key = spec["model"]
        if key in self.message_ids:
            mid, table, model = self.message_ids[key], "Message", self.messages[self.message_ids[key]]
        elif key in self.thread_ids:
            mid, table, model = self.thread_ids[key], "Thread", self.threads[self.thread_ids[key]]
        else:
            raise ValueError(f"legacy_db metadata names {key!r}, which is neither a message nor a thread key")
        value = spec.get("value") or {}
        version = int(spec.get("version", 1))
        model["metadata"].append({"pluginId": spec["plugin"], "value": value, "v": version})
        exp = value.get("expiration") if isinstance(value.get("expiration"), (int, float)) else None
        self.c.execute("INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) VALUES (?,?,?,?,?)",
                       (mid, self.aid, table, spec["plugin"], exp))

    def finish(self):
        for mid, data in self.messages.items():
            self.c.execute("UPDATE Message SET data = ? WHERE id = ?", (json.dumps(data), mid))
        for tid, data in self.threads.items():
            self.c.execute(
                "INSERT INTO Thread (id, accountId, version, data, subject, unread, starred, firstMessageTimestamp, "
                "lastMessageTimestamp, lastMessageReceivedTimestamp, lastMessageSentTimestamp, inAllMail) "
                "VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                (tid, self.aid, 1, json.dumps(data), data["subject"], 0, 0, data["fmt"], data["lmt"], data["lmrt"], 0, 1))


def _write(db_path: str, spec: dict, account_id: str, resolved: dict, keep_wal: bool):
    conn = sqlite3.connect(db_path, isolation_level=None)
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA page_size = 4096")
    if keep_wal:
        conn.execute("PRAGMA wal_autocheckpoint = 0")
    for sql in cpp_schema_statements():
        conn.execute(sql)
    conn.execute("PRAGMA user_version = 12")
    conn.execute("BEGIN")
    w = _Writer(conn, account_id, (lambda mb, uid: resolved[f"{mb}:{uid}"]) if resolved else None)
    for path, role in (spec.get("folders") or {}).items():
        w.folder(path, role or "")
    for key, mspec in (spec.get("messages") or {}).items():
        w.message(key, mspec)
    for mspec in spec.get("metadata") or []:
        w.metadata(mspec)
    w.finish()
    for k, v in (spec.get("state") or {}).items():
        conn.execute("REPLACE INTO _State (id, value) VALUES (?, ?)", (k, str(v)))
    # The C++ engine records its last vacuum (MailStore.cpp VACUUM_TIME_KEY); it never wrote
    # an `engine` key.
    conn.execute("REPLACE INTO _State (id, value) VALUES ('VACUUM_TIME', ?)", (str(int(time.time())),))
    for sql in spec.get("sql") or []:
        conn.execute(sql)
    conn.execute("COMMIT")
    files_dir = Path(db_path).parent / "files"
    for rel, content in {**getattr(w, "pending_files", {}), **(spec.get("files") or {})}.items():
        target = files_dir.joinpath(*rel.split("/"))
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content if isinstance(content, bytes) else str(content).encode())
    if keep_wal:
        # Exit without closing: the committed frames stay in the -wal file with its -shm,
        # which is what a C++ engine killed mid-run leaves for migrate to move.
        sys.stdout.flush()
        os._exit(0)
    conn.close()


def write_legacy_database(config_dir: Path, spec: dict, account_id: str,
                          resolve: Optional[ResolveMessage] = None, database: str = DATABASE) -> dict:
    """Writes `config_dir/<database>` as the C++ engine would have left it. Returns what was
    written: {"messages": {key: id}, "files": [relative paths]}."""
    config_dir = Path(config_dir)
    config_dir.mkdir(parents=True, exist_ok=True)
    db_path = config_dir / database
    if db_path.exists():
        raise FileExistsError(f"{db_path} already exists")
    resolved = {}
    for mspec in (spec.get("messages") or {}).values():
        if "mailbox" in mspec:
            resolved[f"{mspec['mailbox']}:{int(mspec['uid'])}"] = resolve(mspec["mailbox"], int(mspec["uid"]))
    payload = json.dumps({"db": str(db_path), "spec": spec, "aid": account_id, "resolved": resolved,
                          "wal": bool(spec.get("wal", False))})
    # A child process, so that `wal: true` can exit without the close-time checkpoint.
    r = subprocess.run([sys.executable, "-I", __file__], input=payload, text=True, capture_output=True, timeout=60)
    if r.returncode != 0:
        raise RuntimeError(f"writing the legacy database failed: {r.stderr[-2000:]}")
    files = sorted(str(p.relative_to(config_dir / "files")) for p in (config_dir / "files").rglob("*") if p.is_file()) \
        if (config_dir / "files").exists() else []
    return {"files": files}


if __name__ == "__main__":
    args = json.loads(sys.stdin.read())
    _write(args["db"], args["spec"], args["aid"], args["resolved"], args["wal"])
