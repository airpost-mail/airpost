#!/usr/bin/env python3
"""Replays the C++ engine's fresh-database migration and dumps or compares the schema.

The C++ engine builds a new database by running, in order, the V1, V3, V4, V6, V7, V8 and
V9 setup queries and then V10's setup and index queries (MailStore::migrate and
_migrateToV10 with freshDatabase = true; V11 and V12 only rewrite rows). The Rust engine
creates the same final schema in one step (docs/engine-port-plan.md section 9). This
script is the reference for that DDL:

    schema_check.py dump  [--constants PATH] > crates/ms-store/tests/fixtures/v12_schema.json
    schema_check.py check DB [--constants PATH]   # compare a database made by --mode migrate

Comparison is on sqlite_master (type, name, tbl_name, sql), which is what SQLite itself
keeps: ALTER TABLE ADD COLUMN rewrites the stored CREATE TABLE text, so the replayed text
is what a migrated C++ database contains.
"""
import argparse
import json
import re
import sqlite3
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
DEFAULT_CONSTANTS = REPO / "legacy" / "mailsync-cpp" / "MailSync" / "constants.h"
FRESH_STEPS = [
    "V1_SETUP_QUERIES", "V3_SETUP_QUERIES", "V4_SETUP_QUERIES", "V6_SETUP_QUERIES",
    "V7_SETUP_QUERIES", "V8_SETUP_QUERIES", "V9_SETUP_QUERIES",
    "V10_SETUP_QUERIES", "V10_INDEX_QUERIES",
]


def strip_comments(src: str) -> str:
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == '"':
            j = i + 1
            while src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append(src[i:j + 1]); i = j + 1
        elif src.startswith("//", i):
            i = src.index("\n", i)
        elif src.startswith("/*", i):
            i = src.index("*/", i) + 2
        else:
            out.append(c); i += 1
    return "".join(out)


def unescape(lit: str) -> str:
    return re.sub(r'\\(.)', lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), lit)


def parse_arrays(constants: Path) -> dict:
    """`static vector<string> NAME = { "a" "b", "c", };` -> {NAME: ["ab", "c"]}."""
    src = strip_comments(constants.read_text())
    arrays = {}
    for m in re.finditer(r"static\s+vector<string>\s+(\w+)\s*=\s*\{(.*?)\};", src, re.S):
        items, cur = [], None
        for tok in re.finditer(r'"((?:[^"\\]|\\.)*)"|(,)', m.group(2)):
            if tok.group(2):
                if cur is not None:
                    items.append(cur)
                cur = None
            else:
                cur = (cur or "") + unescape(tok.group(1))
        if cur is not None:
            items.append(cur)
        arrays[m.group(1)] = items
    return arrays


def schema_of(conn: sqlite3.Connection) -> list:
    rows = conn.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name").fetchall()
    return [{"type": t, "name": n, "tbl_name": tb, "sql": s} for t, n, tb, s in rows]


def replay(constants: Path) -> list:
    arrays = parse_arrays(constants)
    with tempfile.TemporaryDirectory() as d:
        conn = sqlite3.connect(Path(d) / "replay.db", isolation_level=None)
        for step in FRESH_STEPS:
            for sql in arrays[step]:
                conn.execute(sql)
        schema = schema_of(conn)
        conn.close()
    return schema


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("command", choices=["dump", "check"])
    ap.add_argument("db", nargs="?")
    ap.add_argument("--constants", type=Path, default=DEFAULT_CONSTANTS)
    args = ap.parse_args()

    expected = replay(args.constants)
    if args.command == "dump":
        json.dump(expected, sys.stdout, indent=1)
        sys.stdout.write("\n")
        return 0

    if not args.db:
        ap.error("check needs the database path")
    # Not read-only: a WAL database without its -shm file cannot be opened read-only.
    conn = sqlite3.connect(args.db)
    actual = schema_of(conn)
    version = conn.execute("PRAGMA user_version").fetchone()[0]
    conn.close()
    ok = True
    if version != 12:
        print(f"user_version is {version}, expected 12"); ok = False
    exp = {(e["type"], e["name"]): e for e in expected}
    act = {(a["type"], a["name"]): a for a in actual}
    for key in sorted(exp.keys() | act.keys()):
        if key not in act:
            print(f"missing {key[0]} {key[1]}"); ok = False
        elif key not in exp:
            print(f"unexpected {key[0]} {key[1]}"); ok = False
        elif exp[key] != act[key]:
            print(f"differs {key[0]} {key[1]}:\n  expected {exp[key]['sql']}\n  actual   {act[key]['sql']}")
            ok = False
    print("schema matches the C++ V12 replay" if ok else "schema differs")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
