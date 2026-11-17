#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Airpost contributors
"""
The thin-clients read measurement: the desktop's read paths with `core.engine.reads` on `sqlite` and on
`engine`, against a large synthetic profile and a release `serve` engine.

1. Builds the profile once with `cargo run --release -p airpost-sync --example perf_profile`
   (about 100k messages, 33k threads, 3 accounts, one of them Gmail-style) into
   <dir>/seed, unless it is there. No real mailbox is read.
2. Copies it to <dir>/run, starts `airpost-engine --mode serve` on the copy and adds the
   profile's accounts with an IMAP server that refuses connections, so the engine serves
   reads and runs task local phases but syncs nothing.
3. Runs apps/desktop/tests/integration/thin-perf.test.ts in the desktop's spec window, which
   writes <dir>/results.json, and prints the p50/p95 table.

    python3 apps/engine/test/tools/thin_perf.py [--dir /tmp/airpost-shots/thin-perf]
        [--engine target/release/airpost-engine] [--iterations 20] [--scale 1.0]
        [--busy-top] [--table-only]

--busy-top builds and measures the stress profile instead (<dir>/seed-busy, results-busy.json):
the same mail with each thread's messages held at the clock, so the long threads lead every list
and a 200-thread first page carries about 20 messages per thread instead of about 2.
"""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
DESKTOP = REPO / "apps" / "desktop"


class Engine:
    """A serve engine with JSON-RPC over its stdin (the parent channel)."""

    def __init__(self, binary: Path, config_dir: Path):
        self.sock_dir = Path(tempfile.mkdtemp(prefix="tp-", dir="/tmp"))
        self.socket_path = str(self.sock_dir / "s")
        env = dict(os.environ)
        env.update(
            CONFIG_DIR_PATH=str(config_dir),
            IDENTITY_SERVER="http://127.0.0.1:9/unreachable",
            AIRPOST_ENGINE_KEY_FILE=str(config_dir / "engine.key"),
            AIRPOST_ENGINE_RESTART_DELAY_MS="600000",
        )
        self.stderr = open(config_dir / "engine-stderr.log", "wb")
        self.proc = subprocess.Popen(
            [str(binary), "--mode", "serve", "--socket", self.socket_path],
            env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
        )
        self.next_id = 1
        self.answers = {}
        self.cond = threading.Condition()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if isinstance(msg, dict) and "id" in msg and msg.get("method") is None:
                with self.cond:
                    self.answers[msg["id"]] = msg
                    self.cond.notify_all()

    def call(self, method: str, params: dict, timeout: float = 60):
        rid = self.next_id
        self.next_id += 1
        line = json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        self.proc.stdin.write(line.encode() + b"\n")
        self.proc.stdin.flush()
        deadline = time.monotonic() + timeout
        with self.cond:
            while rid not in self.answers:
                left = deadline - time.monotonic()
                if left <= 0 or self.proc.poll() is not None:
                    raise RuntimeError(f"{method}: no answer (engine exit {self.proc.poll()})")
                self.cond.wait(left)
            msg = self.answers.pop(rid)
        if msg.get("error"):
            raise RuntimeError(f"{method}: {msg['error']}")
        return msg.get("result")

    def stop(self):
        try:
            self.call("engine.shutdown", {}, timeout=20)
            self.proc.wait(timeout=30)
        except Exception:
            self.proc.terminate()
            self.proc.wait(timeout=30)
        shutil.rmtree(self.sock_dir, ignore_errors=True)


def offline_account(a: dict) -> dict:
    # Port 9 on loopback refuses: the workers fail to connect and back off.
    settings = {
        "imap_host": "127.0.0.1", "imap_port": 9, "imap_username": a["emailAddress"],
        "imap_password": "x", "imap_security": "none", "imap_allow_insecure_ssl": False,
        "imap_allow_cleartext": "127.0.0.1",
        "smtp_host": "127.0.0.1", "smtp_port": 9, "smtp_username": a["emailAddress"],
        "smtp_password": "x", "smtp_security": "none", "smtp_allow_insecure_ssl": False,
        "smtp_allow_cleartext": "127.0.0.1",
        "container_folder": "", "create_helper_folders": False, "save_drafts_to_server": False,
    }
    return {
        "id": a["id"], "__cls": "Account", "provider": "imap", "emailAddress": a["emailAddress"],
        "name": "Perf User", "settings": settings, "aliases": [], "label": a["emailAddress"],
    }


def percentile(values, p):
    s = sorted(values)
    if not s:
        return None
    k = max(0, min(len(s) - 1, int(round(p / 100 * (len(s) - 1)))))
    return s[k]


def table(results: dict) -> str:
    rows = ["| Shape | rows | SQLite p50 / p95 ms | engine p50 / p95 ms | n |",
            "|---|---:|---:|---:|---:|"]
    for name, entry in results["results"].items():
        samples = entry["samples"]
        cells = []
        n = 0
        for mode in ("sqlite", "engine"):
            v = samples.get(mode) or []
            n = max(n, len(v))
            cells.append(f"{percentile(v, 50):.1f} / {percentile(v, 95):.1f}" if v else "-")
        rows.append(f"| {name} | {entry.get('rows', '')} | {cells[0]} | {cells[1]} | {n} |")
    return "\n".join(rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default="/tmp/airpost-shots/thin-perf")
    target = Path(os.environ.get("CARGO_TARGET_DIR") or REPO / "target")
    ap.add_argument("--engine", default=str(target / "release" / "airpost-engine"))
    ap.add_argument("--iterations", type=int, default=20)
    ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--busy-top", action="store_true",
                    help="the stress profile: the long threads at the top of every list")
    ap.add_argument("--table-only", action="store_true", help="print the table of the last run")
    args = ap.parse_args()
    base = Path(args.dir)
    suffix = "-busy" if args.busy_top else ""
    results_path = base / f"results{suffix}.json"
    if args.table_only:
        print(table(json.loads(results_path.read_text())))
        return

    seed = base / f"seed{suffix}"
    if not (seed / "airpost.db").exists():
        subprocess.run(["cargo", "run", "--release", "-p", "airpost-sync", "--example", "perf_profile",
                        "--", str(seed), "--scale", str(args.scale)]
                       + (["--busy-top"] if args.busy_top else []), cwd=REPO, check=True)
    run = base / "run"
    shutil.rmtree(run, ignore_errors=True)
    run.mkdir(parents=True)
    for name in ("airpost.db", "profile.json"):
        shutil.copy2(seed / name, run / name)
    profile = json.loads((run / "profile.json").read_text())

    engine = Engine(Path(args.engine), run)
    try:
        engine.call("engine.status", {}, timeout=60)
        for a in profile["accounts"]:
            engine.call("account.add", {"account": offline_account(a), "identity": None}, timeout=60)
        env = dict(os.environ)
        env.update(
            AIRPOST_PERF_PROFILE=str(run),
            AIRPOST_PERF_SOCKET=engine.socket_path,
            AIRPOST_PERF_OUT=str(results_path),
            AIRPOST_PERF_ITERATIONS=str(args.iterations),
        )
        results_path.unlink(missing_ok=True)
        subprocess.run(["npm", "test", "--", "--spec-file-pattern=thin-perf"], cwd=DESKTOP, env=env,
                       check=False)
    finally:
        engine.stop()
    if not results_path.exists():
        sys.exit("the spec wrote no results")
    print(table(json.loads(results_path.read_text())))


if __name__ == "__main__":
    main()
