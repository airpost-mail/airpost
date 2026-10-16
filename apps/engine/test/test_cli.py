"""
End-to-end test of the `tern` CLI (apps/cli, docs/engine-protocol-plan.md section 3): a real
engine on the fake IMAP server with the socket transport, and the CLI pointed at its config
dir through --config-dir, so it finds the engine by its discovery file the way it finds the
desktop's engines.

    cargo build --release -p mailsync -p tern
    MAILSYNC_BIN=target/release/mailsync python3 -m pytest apps/engine/test/test_cli.py -n 0

The CLI binary comes from TERN_BIN, else the newer of target/release/tern and
target/debug/tern. Without one the test skips, unless CI or REQUIRE_TERN is set.
"""
import json
import os
import queue
import subprocess
import threading
import time
from pathlib import Path

import pytest

from harness.mailsync import REPO_ROOT
from harness.scenario import ScenarioRun, ServerSpec

SUBJECT = "Quarterly zebra report"
# Older than the engine's body-prefetch window (BODY_SYNC_MAX_AGE, three months), so
# `show --body` has to ask for it with bodies.fetch.
OLD_SUBJECT = "Ancient walrus memo"
INBOX_MESSAGES = 8

SCENARIO = {
    "name": "cli-end-to-end",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {
        "INBOX": {"messages": INBOX_MESSAGES},
        "Archive": {"messages": 3},
    },
    "setup": [
        {"server.append": {"mailbox": "INBOX", "messages": {"count": 1, "subject": SUBJECT,
                                                            "attachment": ["report.bin", "binary payload"]}}},
        {"server.append": {"mailbox": "Archive", "messages": {"count": 1, "subject": OLD_SUBJECT,
                                                              "age_days": 200}}},
    ],
}


def tern_binary() -> Path:
    if os.environ.get("TERN_BIN"):
        return Path(os.environ["TERN_BIN"])
    candidates = [REPO_ROOT / "target" / profile / "tern" for profile in ("release", "debug")]
    built = [c for c in candidates if c.is_file() and os.access(c, os.X_OK)]
    if not built:
        if os.environ.get("CI") or os.environ.get("REQUIRE_TERN"):
            pytest.fail("no tern binary: build it with cargo build -p tern, or set TERN_BIN. Tried: "
                        + ", ".join(map(str, candidates)))
        pytest.skip("no tern binary: cargo build -p tern, or set TERN_BIN. Tried: " + ", ".join(map(str, candidates)))
    # The newer one, so a stale release build doesn't shadow the debug build just made.
    return max(built, key=lambda c: c.stat().st_mtime)


@pytest.fixture(scope="module")
def engine(request):
    binary = request.config.getoption("--mailsync")
    run = ScenarioRun(dict(SCENARIO), ServerSpec.parse({"fake": "dovecot"}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = "socket"
    try:
        run.ms.start()
        run.ms.wait_quiescent(timeout=180)
        yield run
    finally:
        run.teardown()


class Tern:
    def __init__(self, run: ScenarioRun):
        self.bin = str(tern_binary())
        self.config_dir = str(run.ms.config_dir)
        self.env = {**os.environ, "NO_COLOR": "1"}
        self.env.pop("TERN_CONFIG_DIR", None)

    def args(self, *args, json_out=True, config_dir=None) -> list:
        return [self.bin, "--config-dir", config_dir or self.config_dir] + (["--json"] if json_out else []) + list(args)

    def run(self, *args, json_out=True, input=None, timeout=60, expect=0, config_dir=None):
        r = subprocess.run(self.args(*args, json_out=json_out, config_dir=config_dir), capture_output=True,
                           text=True, input=input, timeout=timeout, env=self.env)
        assert r.returncode == expect, f"tern {' '.join(args)}: exit {r.returncode}\nstdout: {r.stdout[-2000:]}\nstderr: {r.stderr[-2000:]}"
        return r

    def json(self, *args, **kw):
        r = self.run(*args, **kw)
        assert r.stderr == "", r.stderr
        return json.loads(r.stdout)

    def popen(self, *args) -> "Stream":
        return Stream(subprocess.Popen(self.args(*args), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       text=True, env=self.env))


class Stream:
    """A running `tern` whose stdout lines are read on a thread."""

    def __init__(self, proc: subprocess.Popen):
        self.proc = proc
        self.lines: queue.Queue = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            self.lines.put(line)

    def next_json(self, predicate, timeout: float) -> dict:
        deadline = time.monotonic() + timeout
        seen = []
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=max(0.05, deadline - time.monotonic()))
            except queue.Empty:
                break
            doc = json.loads(line)
            seen.append(doc)
            if predicate(doc):
                return doc
        raise AssertionError(f"no matching line within {timeout}s; got {seen[-5:]}")

    def stop(self):
        if self.proc.poll() is None:
            self.proc.terminate()
        self.proc.wait(timeout=10)


def test_cli_end_to_end(engine, tmp_path):
    tern = Tern(engine)
    account_id = engine.ms.account["id"]
    email = engine.ms.account["emailAddress"]

    # No engine: exit 3, and the message says what to do.
    r = tern.run("status", config_dir=str(tmp_path), expect=3)
    assert "no engine running: start Tern" in r.stderr

    status = tern.json("status")
    [eng] = status["engines"]
    assert eng["pid"] == engine.ms.proc.pid
    assert [(a["accountId"], a["email"], a["connectionError"]) for a in eng["accounts"]] == [(account_id, email, False)]
    text = tern.run("status", json_out=False).stdout
    assert text.splitlines()[0].split()[:3] == ["ACCOUNT", "ID", "STATE"]
    assert email in text and account_id in text

    accounts = tern.json("accounts")["accounts"]
    assert [(a["accountId"], a["email"], a["state"]) for a in accounts] == [(account_id, email, "running")]
    # --account by email or id; an unknown one has no engine.
    assert tern.json("--account", email.upper(), "accounts")["accounts"][0]["accountId"] == account_id
    r = tern.run("--account", "nobody@example.test", "accounts", expect=3)
    assert "tern accounts" in r.stderr

    folders = tern.json("folders")
    paths = {f["path"]: f for f in folders["folders"]}
    assert {"INBOX", "Archive"} <= set(paths)
    inbox = paths["INBOX"]
    assert folders["counts"][inbox["id"]]["total"] == INBOX_MESSAGES + 1
    assert "INBOX" in tern.run("folders", json_out=False).stdout

    # Paging: by folder path, a short page with a cursor, then --all.
    page = tern.json("threads", "--folder", "inbox", "-n", "3")
    assert len(page["threads"]) == 3 and page["next"] and page["more"] is True
    page2 = tern.json("threads", "--folder", "inbox", "-n", "3", "--before", page["next"])
    assert len(page2["threads"]) == 3
    assert not {t["id"] for t in page2["threads"]} & {t["id"] for t in page["threads"]}
    every = tern.json("threads", "--folder", "INBOX", "--all", "-n", "4")
    assert len(every["threads"]) == INBOX_MESSAGES + 1 and every["next"] is None and every["more"] is False
    lmts = [t["lmt"] for t in every["threads"]]
    assert lmts == sorted(lmts, reverse=True)
    assert [t["id"] for t in every["threads"][:3]] == [t["id"] for t in page["threads"]]
    assert len(tern.json("threads", "--role", "inbox", "--all")["threads"]) == INBOX_MESSAGES + 1
    r = tern.run("threads", "--folder", "Nope", expect=1)
    assert "tern folders" in r.stderr
    tern.run("threads", "--folder", "INBOX", "--label", "x", expect=2)
    listing = tern.run("threads", "--folder", "INBOX", json_out=False).stdout.splitlines()
    assert listing[0].split() == ["DATE", "FROM", "SUBJECT", "ID"]
    assert len(listing) == INBOX_MESSAGES + 2

    # Search finds the thread; show on its id lists the message, on the message id shows it.
    found = tern.json("search", "zebra", "QUARTERLY")["threads"]
    assert [t["subject"] for t in found] == [SUBJECT]
    assert tern.json("search", "zebra", "nothing-like-this")["threads"] == []
    thread = tern.json("show", found[0]["id"])
    assert thread["kind"] == "thread"
    [message] = thread["messages"]
    assert message["subject"] == SUBJECT
    shown = tern.json("show", message["id"], "--body")
    assert shown["kind"] == "message"
    assert shown["message"]["id"] == message["id"]
    assert "This is the body of message" in shown["body"]
    assert [f["filename"] for f in shown["files"]] == ["report.bin"]
    text = tern.run("show", message["id"], "--body", json_out=False).stdout
    assert f"Subject: {SUBJECT}" in text and "report.bin" in text and "This is the body of message" in text
    # A body outside the prefetch window: not stored until show --body asks for it.
    [old] = tern.json("search", "walrus")["threads"]
    [old_message] = tern.json("show", old["id"])["messages"]
    assert tern.json("rpc", "messages.get", json.dumps({"accountId": account_id, "id": old_message["id"],
                                                         "body": True}))["body"] is None
    old_shown = tern.json("show", old_message["id"], "--body", timeout=60)
    assert "This is the body of message" in old_shown["body"]
    r = tern.run("show", "no-such-id", expect=1)
    assert "no message or thread no-such-id" in r.stderr

    contacts = tern.json("contacts", "alice")
    assert isinstance(contacts["contacts"], list)

    raw = tern.json("rpc", "threads.get", json.dumps({"accountId": account_id, "id": found[0]["id"]}))
    assert raw["thread"]["id"] == found[0]["id"]
    assert tern.json("rpc", "engine.status")["pid"] == engine.ms.proc.pid
    tern.run("rpc", "threads.get", "{not json", expect=2)

    assert tern.json("sync")["accounts"] == [account_id]

    # A task, queued and waited for, while `watch` streams its deltas.
    watch = tern.popen("watch", "--class", "Task")
    try:
        time.sleep(1.0)   # let watch subscribe; nothing it prints says when it has
        task = {"__cls": "ChangeStarredTask", "messageIds": [message["id"]], "starred": True}
        done = tern.json("task", "queue", "-", "--wait", "--timeout", "60", input=json.dumps(task), timeout=90)
        assert done["status"] == "complete" and done.get("error") is None, done
        delta = watch.next_json(lambda d: d["modelClass"] == "Task"
                                and any(m["id"] == done["id"] and m["status"] == "complete" for m in d["modelJSONs"]),
                                timeout=30)
        assert delta["accountId"] == account_id
    finally:
        watch.stop()
    assert tern.json("show", message["id"])["message"]["starred"] is True

    # task wait after the task was queued: reported at once when it has already finished,
    # and an unknown id fails instead of waiting.
    task = {"__cls": "ChangeStarredTask", "id": "cli-unstar-1", "messageIds": [message["id"]], "starred": False}
    queued = tern.json("task", "queue", json.dumps(task))
    assert queued["id"] == "cli-unstar-1"
    final = tern.json("task", "wait", "cli-unstar-1", "--timeout", "60", timeout=90)
    assert final["id"] == "cli-unstar-1" and final["status"] == "complete"
    again = tern.json("task", "wait", "cli-unstar-1", timeout=30)
    assert again["status"] == "complete"
    r = tern.run("task", "wait", "no-such-task", timeout=30, expect=1)
    assert "no task no-such-task" in r.stderr
    tern.run("task", "queue", '{"starred": true}', expect=2)

    log = tern.run("logs", account_id, "-n", "5", json_out=False).stdout.splitlines()
    assert len(log) == 5
    log = tern.run("logs", email, "-n", "3").stdout.splitlines()
    assert [set(json.loads(l)) for l in log] == [{"line"}] * 3
    # Without an account argument: --account, else the one account with a log.
    assert len(tern.run("--account", account_id, "logs", "-n", "2", json_out=False).stdout.splitlines()) == 2
    assert len(tern.run("logs", "-n", "2", json_out=False).stdout.splitlines()) == 2
    r = tern.run("logs", "nobody", expect=1)
    assert account_id in r.stderr

    assert engine.ms.running
