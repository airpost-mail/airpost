"""
End-to-end test of the `tern` CLI (apps/cli): a real
engine on the fake IMAP server, once per account on the socket transport (`run/<id>.json`)
and once as a `serve` engine (`run/engine.json`), and the CLI pointed at its config
dir through --config-dir, so it finds the engine by its discovery file the way it finds the
desktop's engines.

    cargo build --release -p tern-engine -p tern
    TERN_ENGINE_BIN=target/release/tern-engine python3 -m pytest apps/engine/test/test_cli.py -n 0

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

from harness.engine import REPO_ROOT
from harness.scenario import ScenarioRun, ServerSpec

SUBJECT = "Quarterly zebra report"
# Older than the engine's body-prefetch window (BODY_SYNC_MAX_AGE, three months), so
# `show --body` has to ask for it with bodies.fetch.
OLD_SUBJECT = "Ancient walrus memo"
INBOX_MESSAGES = 8
LIST_SUBJECT = "Flamingo digest"

SCENARIO = {
    "name": "cli-end-to-end",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {
        "INBOX": {"messages": INBOX_MESSAGES},
        "Archive": {"messages": 3},
        "[Mailspring]/Snoozed": {},
    },
    "setup": [
        {"server.append": {"mailbox": "INBOX", "messages": {"count": 1, "subject": SUBJECT,
                                                            "attachment": ["report.bin", "binary payload"]}}},
        {"server.append": {"mailbox": "Archive", "messages": {"count": 1, "subject": OLD_SUBJECT,
                                                              "age_days": 200}}},
        {"server.append": {"mailbox": "Archive", "messages": {
            "count": 1, "subject": LIST_SUBJECT,
            "extra_headers": {"List-Unsubscribe": "<https://lists.example.net/u/3?t=abc>"}}}},
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


@pytest.fixture(scope="module", params=["socket", "serve"])
def engine(request):
    binary = request.config.getoption("--engine")
    run = ScenarioRun(dict(SCENARIO, name=f"{SCENARIO['name']}-{request.param}"), ServerSpec.parse({"fake": "dovecot"}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = request.param
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
    # The engine reads the search grammar, dates in the CLI's zone and clock.
    assert "search-grammar" in status["engines"][0]["features"]
    def subjects(*query):
        return sorted(t["subject"] for t in tern.json("search", *query)["threads"])
    assert subjects("zebra OR walrus") == [OLD_SUBJECT, SUBJECT]
    assert subjects("(zebra OR walrus) NOT subject:walrus") == [SUBJECT]
    assert subjects("subject:zebra", "in:inbox") == [SUBJECT]
    assert subjects("zebra", "before:2100-01-01") == [SUBJECT]
    assert subjects("zebra", 'after:"2 days ago"', "before:1990-01-01") == []
    assert subjects("in:trash", "zebra") == []
    r = tern.run("search", "(zebra", expect=2)
    assert "never closed" in r.stderr
    r = tern.run("search", "after:someday", expect=2)
    assert "isn't a date" in r.stderr
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

    # Settings live in the serve engine only.
    if engine.ms.transport == "serve":
        assert tern.json("settings", "get") == {"values": {}}
        assert tern.json("settings", "set", "core.sending.undoSend", "15000") == {
            "values": {"core.sending.undoSend": 15000}}
        tern.run("settings", "set", "resurfacing.snoozeBody", "Back on top")
        assert tern.json("settings", "get", "core.sending.undoSend") == {"values": {"core.sending.undoSend": 15000}}
        text = tern.run("settings", "get", json_out=False).stdout.splitlines()
        assert text == ["core.sending.undoSend = 15000", 'resurfacing.snoozeBody = "Back on top"']
        assert json.loads((engine.ms.config_dir / "settings.json").read_text())["core.sending.undoSend"] == 15000
        tern.run("settings", "set", "core.sending.undoSend", "null")
        assert "core.sending.undoSend" not in tern.json("settings", "get")["values"]
        r = tern.run("settings", "set", "core.sending.undoSend", "soon", expect=2)
        assert "core.sending.undoSend must be" in r.stderr
        tern.run("settings", "get", "core.themes", expect=2)
    else:
        r = tern.run("settings", "get", expect=1)
        assert "doesn't keep settings" in r.stderr

    # Snooze and unsnooze through the engine.
    assert "snooze" in status["engines"][0]["features"]
    def in_role(role, thread_id):
        return any(t["id"] == thread_id for t in tern.json("threads", "--role", role, "--all")["threads"])
    def wait_until(cond, what, timeout=30):
        deadline = time.monotonic() + timeout
        while not cond():
            assert time.monotonic() < deadline, what
            time.sleep(0.5)
    snoozed = tern.json("snooze", found[0]["id"], "--until", "2h")
    [group] = snoozed["accounts"]
    assert group["accountId"] == account_id and len(group["taskIds"]) == 2
    assert snoozed["until"] > time.time() + 7000
    wait_until(lambda: in_role("snoozed", found[0]["id"]), "the thread reaches Snoozed")
    assert not in_role("inbox", found[0]["id"])
    text = tern.run("unsnooze", found[0]["id"], json_out=False).stdout
    assert text.strip() == "Unsnoozed 1 thread"
    wait_until(lambda: in_role("inbox", found[0]["id"]), "the thread is back in the inbox")
    r = tern.run("snooze", found[0]["id"], "--until", "2020-01-01", expect=2)
    assert "in the past" in r.stderr
    tern.run("snooze", found[0]["id"], "--until", "whenever", expect=2)
    r = tern.run("unsnooze", "no-such-thread", expect=1)
    assert "no thread no-such-thread" in r.stderr

    # Unsubscribe: a web-only header gives the link back; a message without one fails.
    [listed] = tern.json("search", "flamingo")["threads"]
    [list_message] = tern.json("show", listed["id"])["messages"]
    r = tern.json("unsubscribe", list_message["id"], timeout=60)
    assert r == {"method": "web", "ok": True, "url": "https://lists.example.net/u/3?t=abc"}
    text = tern.run("unsubscribe", list_message["id"], json_out=False, timeout=60).stdout
    assert "open https://lists.example.net/u/3?t=abc" in text
    r = tern.run("unsubscribe", list_message["id"], "--method", "one-click", expect=1, timeout=60)
    assert "no one-click" in r.stderr
    r = tern.run("unsubscribe", message["id"], expect=1, timeout=60)
    assert "List-Unsubscribe" in r.stderr

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


def test_cli_calendar_and_contacts(engine, tmp_path):
    """`tern calendar list` and `tern contact`: the account has no calendars on the fake
    server, so the calendar half is the empty list and the refusals; contacts are imported,
    read, changed and exported through the engine."""
    tern = Tern(engine)
    assert tern.json("calendar", "list", "--from", "2026-01-01", "--to", "2026-02-01") == {"occurrences": []}
    assert "No events." in tern.run("calendar", "list", json_out=False).stdout
    r = tern.run("calendar", "create", "--calendar", "nope", "--title", "x", "--start", "2h", expect=1)
    assert "no calendar nope" in r.stderr
    tern.run("calendar", "list", "--from", "3d", "--to", "1d", expect=2)

    vcf = tmp_path / "people.vcf"
    vcf.write_text("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Zed Zebra\r\nN:Zebra;Zed;;;\r\nEMAIL:zed@example.org\r\nEND:VCARD\r\n")
    r = tern.json("contact", "import", str(vcf))
    [cid] = r["contactIds"]
    got = tern.json("contact", "get", cid)
    assert got["data"]["name"]["displayName"] == "Zed Zebra"
    r = tern.json("contact", "update", cid, "--title", "Keeper", "--company", "Zoo")
    assert "TITLE:Keeper" in r["contact"]["info"]["vcf"]
    text = tern.run("contact", "get", cid, json_out=False).stdout
    assert "title: Keeper" in text and "company: Zoo" in text
    out = tern.run("contact", "export", cid, json_out=False).stdout
    assert out.startswith("BEGIN:VCARD") and "ORG:Zoo" in out
    tern.run("contact", "update", cid, expect=2)
    r = tern.run("contact", "get", "no-such-contact", expect=1)
    assert "no contact no-such-contact" in r.stderr
