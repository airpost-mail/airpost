"""
End-to-end test of the `airpost` CLI (apps/cli): a real
engine on the fake IMAP server, once per account on the socket transport (`run/<id>.json`)
and once as a `serve` engine (`run/engine.json`), and the CLI pointed at its config
dir through --config-dir, so it finds the engine by its discovery file the way it finds the
desktop's engines.

    cargo build --release -p airpost-engine -p airpost
    AIRPOST_ENGINE_BIN=target/release/airpost-engine python3 -m pytest apps/engine/test/test_cli.py -n 0

The CLI binary comes from AIRPOST_BIN, else the newer of target/release/airpost and
target/debug/airpost. Without one the test skips, unless CI or REQUIRE_AIRPOST is set.
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


def airpost_binary() -> Path:
    if os.environ.get("AIRPOST_BIN"):
        return Path(os.environ["AIRPOST_BIN"])
    candidates = [REPO_ROOT / "target" / profile / "airpost" for profile in ("release", "debug")]
    built = [c for c in candidates if c.is_file() and os.access(c, os.X_OK)]
    if not built:
        if os.environ.get("CI") or os.environ.get("REQUIRE_AIRPOST"):
            pytest.fail("no airpost binary: build it with cargo build -p airpost, or set AIRPOST_BIN. Tried: "
                        + ", ".join(map(str, candidates)))
        pytest.skip("no airpost binary: cargo build -p airpost, or set AIRPOST_BIN. Tried: " + ", ".join(map(str, candidates)))
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


class Airpost:
    def __init__(self, run: ScenarioRun):
        self.bin = str(airpost_binary())
        self.config_dir = str(run.ms.config_dir)
        self.env = {**os.environ, "NO_COLOR": "1"}
        self.env.pop("AIRPOST_CONFIG_DIR", None)

    def args(self, *args, json_out=True, config_dir=None) -> list:
        return [self.bin, "--config-dir", config_dir or self.config_dir] + (["--json"] if json_out else []) + list(args)

    def run(self, *args, json_out=True, input=None, timeout=60, expect=0, config_dir=None):
        r = subprocess.run(self.args(*args, json_out=json_out, config_dir=config_dir), capture_output=True,
                           text=True, input=input, timeout=timeout, env=self.env)
        assert r.returncode == expect, f"airpost {' '.join(args)}: exit {r.returncode}\nstdout: {r.stdout[-2000:]}\nstderr: {r.stderr[-2000:]}"
        return r

    def json(self, *args, **kw):
        r = self.run(*args, **kw)
        assert r.stderr == "", r.stderr
        return json.loads(r.stdout)

    def popen(self, *args) -> "Stream":
        return Stream(subprocess.Popen(self.args(*args), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       text=True, env=self.env))


class Stream:
    """A running `airpost` whose stdout lines are read on a thread."""

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
    airpost = Airpost(engine)
    account_id = engine.ms.account["id"]
    email = engine.ms.account["emailAddress"]

    # No engine: exit 3, and the message says what to do.
    r = airpost.run("status", config_dir=str(tmp_path), expect=3)
    assert "no engine running: start Airpost" in r.stderr

    status = airpost.json("status")
    [eng] = status["engines"]
    assert eng["pid"] == engine.ms.proc.pid
    assert [(a["accountId"], a["email"], a["connectionError"]) for a in eng["accounts"]] == [(account_id, email, False)]
    text = airpost.run("status", json_out=False).stdout
    assert text.splitlines()[0].split()[:3] == ["ACCOUNT", "ID", "STATE"]
    assert email in text and account_id in text

    accounts = airpost.json("accounts")["accounts"]
    assert [(a["accountId"], a["email"], a["state"]) for a in accounts] == [(account_id, email, "running")]
    # --account by email or id; an unknown one has no engine.
    assert airpost.json("--account", email.upper(), "accounts")["accounts"][0]["accountId"] == account_id
    r = airpost.run("--account", "nobody@example.test", "accounts", expect=3)
    assert "airpost accounts" in r.stderr

    folders = airpost.json("folders")
    paths = {f["path"]: f for f in folders["folders"]}
    assert {"INBOX", "Archive"} <= set(paths)
    inbox = paths["INBOX"]
    assert folders["counts"][inbox["id"]]["total"] == INBOX_MESSAGES + 1
    assert "INBOX" in airpost.run("folders", json_out=False).stdout

    # Paging: by folder path, a short page with a cursor, then --all.
    page = airpost.json("threads", "--folder", "inbox", "-n", "3")
    assert len(page["threads"]) == 3 and page["next"] and page["more"] is True
    page2 = airpost.json("threads", "--folder", "inbox", "-n", "3", "--before", page["next"])
    assert len(page2["threads"]) == 3
    assert not {t["id"] for t in page2["threads"]} & {t["id"] for t in page["threads"]}
    every = airpost.json("threads", "--folder", "INBOX", "--all", "-n", "4")
    assert len(every["threads"]) == INBOX_MESSAGES + 1 and every["next"] is None and every["more"] is False
    lmts = [t["lmt"] for t in every["threads"]]
    assert lmts == sorted(lmts, reverse=True)
    assert [t["id"] for t in every["threads"][:3]] == [t["id"] for t in page["threads"]]
    assert len(airpost.json("threads", "--role", "inbox", "--all")["threads"]) == INBOX_MESSAGES + 1
    r = airpost.run("threads", "--folder", "Nope", expect=1)
    assert "airpost folders" in r.stderr
    airpost.run("threads", "--folder", "INBOX", "--label", "x", expect=2)
    listing = airpost.run("threads", "--folder", "INBOX", json_out=False).stdout.splitlines()
    assert listing[0].split() == ["DATE", "FROM", "SUBJECT", "ID"]
    assert len(listing) == INBOX_MESSAGES + 2

    # Search finds the thread; show on its id lists the message, on the message id shows it.
    found = airpost.json("search", "zebra", "QUARTERLY")["threads"]
    assert [t["subject"] for t in found] == [SUBJECT]
    assert airpost.json("search", "zebra", "nothing-like-this")["threads"] == []
    # The engine reads the search grammar, dates in the CLI's zone and clock.
    assert "search-grammar" in status["engines"][0]["features"]
    def subjects(*query):
        return sorted(t["subject"] for t in airpost.json("search", *query)["threads"])
    assert subjects("zebra OR walrus") == [OLD_SUBJECT, SUBJECT]
    assert subjects("(zebra OR walrus) NOT subject:walrus") == [SUBJECT]
    assert subjects("subject:zebra", "in:inbox") == [SUBJECT]
    assert subjects("zebra", "before:2100-01-01") == [SUBJECT]
    assert subjects("zebra", 'after:"2 days ago"', "before:1990-01-01") == []
    assert subjects("in:trash", "zebra") == []
    r = airpost.run("search", "(zebra", expect=2)
    assert "never closed" in r.stderr
    r = airpost.run("search", "after:someday", expect=2)
    assert "isn't a date" in r.stderr
    thread = airpost.json("show", found[0]["id"])
    assert thread["kind"] == "thread"
    [message] = thread["messages"]
    assert message["subject"] == SUBJECT
    shown = airpost.json("show", message["id"], "--body")
    assert shown["kind"] == "message"
    assert shown["message"]["id"] == message["id"]
    assert "This is the body of message" in shown["body"]
    assert [f["filename"] for f in shown["files"]] == ["report.bin"]
    text = airpost.run("show", message["id"], "--body", json_out=False).stdout
    assert f"Subject: {SUBJECT}" in text and "report.bin" in text and "This is the body of message" in text
    # A body outside the prefetch window: not stored until show --body asks for it.
    [old] = airpost.json("search", "walrus")["threads"]
    [old_message] = airpost.json("show", old["id"])["messages"]
    assert airpost.json("rpc", "messages.get", json.dumps({"accountId": account_id, "id": old_message["id"],
                                                         "body": True}))["body"] is None
    old_shown = airpost.json("show", old_message["id"], "--body", timeout=60)
    assert "This is the body of message" in old_shown["body"]
    r = airpost.run("show", "no-such-id", expect=1)
    assert "no message or thread no-such-id" in r.stderr

    contacts = airpost.json("contacts", "alice")
    assert isinstance(contacts["contacts"], list)

    raw = airpost.json("rpc", "threads.get", json.dumps({"accountId": account_id, "id": found[0]["id"]}))
    assert raw["thread"]["id"] == found[0]["id"]
    assert airpost.json("rpc", "engine.status")["pid"] == engine.ms.proc.pid
    airpost.run("rpc", "threads.get", "{not json", expect=2)

    assert airpost.json("sync")["accounts"] == [account_id]

    # A task, queued and waited for, while `watch` streams its deltas.
    watch = airpost.popen("watch", "--class", "Task")
    try:
        time.sleep(1.0)   # let watch subscribe; nothing it prints says when it has
        task = {"__cls": "ChangeStarredTask", "messageIds": [message["id"]], "starred": True}
        done = airpost.json("task", "queue", "-", "--wait", "--timeout", "60", input=json.dumps(task), timeout=90)
        assert done["status"] == "complete" and done.get("error") is None, done
        delta = watch.next_json(lambda d: d["modelClass"] == "Task"
                                and any(m["id"] == done["id"] and m["status"] == "complete" for m in d["modelJSONs"]),
                                timeout=30)
        assert delta["accountId"] == account_id
    finally:
        watch.stop()
    assert airpost.json("show", message["id"])["message"]["starred"] is True

    # task wait after the task was queued: reported at once when it has already finished,
    # and an unknown id fails instead of waiting.
    task = {"__cls": "ChangeStarredTask", "id": "cli-unstar-1", "messageIds": [message["id"]], "starred": False}
    queued = airpost.json("task", "queue", json.dumps(task))
    assert queued["id"] == "cli-unstar-1"
    final = airpost.json("task", "wait", "cli-unstar-1", "--timeout", "60", timeout=90)
    assert final["id"] == "cli-unstar-1" and final["status"] == "complete"
    again = airpost.json("task", "wait", "cli-unstar-1", timeout=30)
    assert again["status"] == "complete"
    r = airpost.run("task", "wait", "no-such-task", timeout=30, expect=1)
    assert "no task no-such-task" in r.stderr
    airpost.run("task", "queue", '{"starred": true}', expect=2)

    # Settings live in the serve engine only.
    if engine.ms.transport == "serve":
        assert airpost.json("settings", "get") == {"values": {}}
        assert airpost.json("settings", "set", "core.sending.undoSend", "15000") == {
            "values": {"core.sending.undoSend": 15000}}
        airpost.run("settings", "set", "resurfacing.snoozeBody", "Back on top")
        assert airpost.json("settings", "get", "core.sending.undoSend") == {"values": {"core.sending.undoSend": 15000}}
        text = airpost.run("settings", "get", json_out=False).stdout.splitlines()
        assert text == ["core.sending.undoSend = 15000", 'resurfacing.snoozeBody = "Back on top"']
        assert json.loads((engine.ms.config_dir / "settings.json").read_text())["core.sending.undoSend"] == 15000
        airpost.run("settings", "set", "core.sending.undoSend", "null")
        assert "core.sending.undoSend" not in airpost.json("settings", "get")["values"]
        r = airpost.run("settings", "set", "core.sending.undoSend", "soon", expect=2)
        assert "core.sending.undoSend must be" in r.stderr
        airpost.run("settings", "get", "core.themes", expect=2)
    else:
        r = airpost.run("settings", "get", expect=1)
        assert "doesn't keep settings" in r.stderr

    # Snooze and unsnooze through the engine.
    assert "snooze" in status["engines"][0]["features"]
    def in_role(role, thread_id):
        return any(t["id"] == thread_id for t in airpost.json("threads", "--role", role, "--all")["threads"])
    def wait_until(cond, what, timeout=30):
        deadline = time.monotonic() + timeout
        while not cond():
            assert time.monotonic() < deadline, what
            time.sleep(0.5)
    snoozed = airpost.json("snooze", found[0]["id"], "--until", "2h")
    [group] = snoozed["accounts"]
    assert group["accountId"] == account_id and len(group["taskIds"]) == 2
    assert snoozed["until"] > time.time() + 7000
    wait_until(lambda: in_role("snoozed", found[0]["id"]), "the thread reaches Snoozed")
    assert not in_role("inbox", found[0]["id"])
    text = airpost.run("unsnooze", found[0]["id"], json_out=False).stdout
    assert text.strip() == "Unsnoozed 1 thread"
    wait_until(lambda: in_role("inbox", found[0]["id"]), "the thread is back in the inbox")
    r = airpost.run("snooze", found[0]["id"], "--until", "2020-01-01", expect=2)
    assert "in the past" in r.stderr
    airpost.run("snooze", found[0]["id"], "--until", "whenever", expect=2)
    r = airpost.run("unsnooze", "no-such-thread", expect=1)
    assert "no thread no-such-thread" in r.stderr

    # Unsubscribe: a web-only header gives the link back; a message without one fails.
    [listed] = airpost.json("search", "flamingo")["threads"]
    [list_message] = airpost.json("show", listed["id"])["messages"]
    r = airpost.json("unsubscribe", list_message["id"], timeout=60)
    assert r == {"method": "web", "ok": True, "url": "https://lists.example.net/u/3?t=abc"}
    text = airpost.run("unsubscribe", list_message["id"], json_out=False, timeout=60).stdout
    assert "open https://lists.example.net/u/3?t=abc" in text
    r = airpost.run("unsubscribe", list_message["id"], "--method", "one-click", expect=1, timeout=60)
    assert "no one-click" in r.stderr
    r = airpost.run("unsubscribe", message["id"], expect=1, timeout=60)
    assert "List-Unsubscribe" in r.stderr

    log = airpost.run("logs", account_id, "-n", "5", json_out=False).stdout.splitlines()
    assert len(log) == 5
    log = airpost.run("logs", email, "-n", "3").stdout.splitlines()
    assert [set(json.loads(l)) for l in log] == [{"line"}] * 3
    # Without an account argument: --account, else the one account with a log.
    assert len(airpost.run("--account", account_id, "logs", "-n", "2", json_out=False).stdout.splitlines()) == 2
    assert len(airpost.run("logs", "-n", "2", json_out=False).stdout.splitlines()) == 2
    r = airpost.run("logs", "nobody", expect=1)
    assert account_id in r.stderr

    assert engine.ms.running


def test_cli_calendar_and_contacts(engine, tmp_path):
    """`airpost calendar list` and `airpost contact`: the account has no calendars on the fake
    server, so the calendar half is the empty list and the refusals; contacts are imported,
    read, changed and exported through the engine."""
    airpost = Airpost(engine)
    assert airpost.json("calendar", "list", "--from", "2026-01-01", "--to", "2026-02-01") == {"occurrences": []}
    assert "No events." in airpost.run("calendar", "list", json_out=False).stdout
    r = airpost.run("calendar", "create", "--calendar", "nope", "--title", "x", "--start", "2h", expect=1)
    assert "no calendar nope" in r.stderr
    airpost.run("calendar", "list", "--from", "3d", "--to", "1d", expect=2)

    vcf = tmp_path / "people.vcf"
    vcf.write_text("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Zed Zebra\r\nN:Zebra;Zed;;;\r\nEMAIL:zed@example.org\r\nEND:VCARD\r\n")
    r = airpost.json("contact", "import", str(vcf))
    [cid] = r["contactIds"]
    got = airpost.json("contact", "get", cid)
    assert got["data"]["name"]["displayName"] == "Zed Zebra"
    r = airpost.json("contact", "update", cid, "--title", "Keeper", "--company", "Zoo")
    assert "TITLE:Keeper" in r["contact"]["info"]["vcf"]
    text = airpost.run("contact", "get", cid, json_out=False).stdout
    assert "title: Keeper" in text and "company: Zoo" in text
    out = airpost.run("contact", "export", cid, json_out=False).stdout
    assert out.startswith("BEGIN:VCARD") and "ORG:Zoo" in out
    airpost.run("contact", "update", cid, expect=2)
    r = airpost.run("contact", "get", "no-such-contact", expect=1)
    assert "no contact no-such-contact" in r.stderr


DRAFT_SUBJECT = "Picnic plans"
PHISH_SUBJECT = "Verify your walrus account"
DRAFT_SCENARIO = {
    "name": "cli-drafts",
    "servers": [{"fake": "dovecot", "smtp": True}],
    "account": {"extra_settings": {"save_drafts_to_server": True}},
    "env": {"AIRPOST_DRAFT_PUSH_QUIET": "1"},
    "mailboxes": {"INBOX": {"messages": 2}, "Drafts": {}, "Sent": {}},
    "setup": [
        {"server.append": {"mailbox": "INBOX", "messages": {
            "message_id": "<picnic-1@sender.example.net>", "subject": DRAFT_SUBJECT,
            "sender": ["Bob Sender", "bob@sender.example.net"], "body": "Saturday at the lake?"}}},
        {"server.append": {"mailbox": "INBOX", "messages": {
            "subject": PHISH_SUBJECT, "sender": ["Bank", "alerts@bank.example.com"],
            "extra_headers": {"Reply-To": "help@bank-support.example.org"}}}},
    ],
}


@pytest.fixture(scope="module")
def serve_engine(request):
    binary = request.config.getoption("--engine")
    run = ScenarioRun(dict(DRAFT_SCENARIO), ServerSpec.parse({"fake": "dovecot", "smtp": True}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = "serve"
    try:
        run.ms.start()
        run.ms.wait_quiescent(timeout=180)
        yield run
    finally:
        run.teardown()


def test_cli_drafts(serve_engine):
    """`airpost draft` on serve: a new draft and a reply written through draft.create, the
    reply revised by a second call, both listed and pushed to the server's Drafts, then the
    reply sent."""
    run = serve_engine
    airpost = Airpost(run)
    [thread] = airpost.json("search", "picnic")["threads"]

    new = airpost.json("draft", "new", "--to", '"Ann A" <ann@example.net>, carol@example.net',
                    "--subject", "Hello", "--body", "Line one\nLine <two>")
    assert new["updated"] is False
    d = new["draft"]
    assert [(c["name"], c["email"]) for c in d["to"]] == [("Ann A", "ann@example.net"), ("", "carol@example.net")]
    assert d["subject"] == "Hello" and d["pristine"] is False
    assert d["body"].startswith("Line one<br/>Line &lt;two&gt;")

    first = airpost.json("draft", "reply", thread["id"], "--body", "Count me in")["draft"]
    assert first["subject"] == f"Re: {DRAFT_SUBJECT}" and first["pristine"] is False
    assert [c["email"] for c in first["to"]] == ["bob@sender.example.net"]
    assert first["threadId"] == thread["id"]
    assert first["body"].startswith("Count me in") and "gmail_quote" in first["body"]
    assert "Saturday at the lake?" in first["body"]
    # Again: the same draft, its text replaced and its quote kept.
    second = airpost.json("draft", "reply", thread["id"], "--body-file", "-", input="Sunday works better\n")
    assert second["updated"] is True
    again = second["draft"]
    assert again["id"] == first["id"] and again["hMsgId"] == first["hMsgId"]
    assert again["body"].startswith("Sunday works better") and "Count me in" not in again["body"]
    assert "Saturday at the lake?" in again["body"]

    listed = airpost.json("draft", "list")["drafts"]
    assert {d["id"] for d in listed} == {d["id"], first["id"]}
    assert [d["date"] for d in listed] == sorted((d["date"] for d in listed), reverse=True)
    text = airpost.run("draft", "list", json_out=False).stdout.splitlines()
    assert text[0].split() == ["DATE", "TO", "SUBJECT", "FILES", "THREAD", "ID"] and len(text) == 3

    # Written, so pushed to the server's Drafts.
    deadline = time.monotonic() + 60
    while len(run.server.truth().get("Drafts", {})) < 2:
        assert time.monotonic() < deadline, "both drafts reach the server's Drafts"
        time.sleep(0.5)

    sent = airpost.json("draft", "send", first["hMsgId"], "--wait", "--timeout", "60", timeout=90)
    assert sent["status"] == "complete" and sent.get("error") is None, sent
    [mail] = run.server.smtp.messages
    assert mail.recipients == ["bob@sender.example.net"]
    assert b"Sunday works better" in mail.raw
    assert [x["id"] for x in airpost.json("draft", "list")["drafts"]] == [d["id"]]

    r = airpost.run("draft", "send", "no-such-draft", expect=1)
    assert "no draft no-such-draft" in r.stderr
    r = airpost.run("draft", "reply", d["id"], expect=2)
    assert "is a draft" in r.stderr


def test_cli_draft_edit_attach_delete(serve_engine, tmp_path):
    """`airpost draft edit` and `delete` over draft.update and draft.delete, and --attach: a
    new draft with a file, edited in place (text replaced, another file attached, the same
    one not twice, one detached), sent with its attachment; a reply edited with its quote
    kept, then deleted."""
    run = serve_engine
    airpost = Airpost(run)
    report = tmp_path / "report.pdf"
    report.write_bytes(b"%PDF-1.4 quarterly numbers")
    notes = tmp_path / "notes.txt"
    notes.write_text("bring snacks")

    new = airpost.json("draft", "new", "--to", "ann@example.net", "--subject", "Agenda",
                    "--body", "First take", "--attach", str(report))["draft"]
    assert [(f["filename"], f["size"], f["contentType"]) for f in new["files"]] == [
        ("report.pdf", 26, "application/pdf")]
    assert new["pristine"] is False

    edited = airpost.json("draft", "edit", new["id"], "--subject", "Agenda v2", "--body", "Second take",
                       "--attach", str(notes), "--attach", str(report))
    d = edited["draft"]
    assert d["id"] == new["id"] and d["hMsgId"] == new["hMsgId"]
    assert d["subject"] == "Agenda v2"
    assert d["body"].startswith("Second take") and "First take" not in d["body"]
    assert sorted(f["filename"] for f in d["files"]) == ["notes.txt", "report.pdf"]

    d = airpost.json("draft", "edit", new["hMsgId"], "--detach", "report.pdf")["draft"]
    assert [f["filename"] for f in d["files"]] == ["notes.txt"]
    text = airpost.run("draft", "list", json_out=False).stdout.splitlines()
    assert text[0].split() == ["DATE", "TO", "SUBJECT", "FILES", "THREAD", "ID"]
    assert any(new["id"] in line and "notes.txt" in line for line in text[1:])
    assert "notes.txt" in airpost.run("show", new["id"], json_out=False).stdout

    [thread] = airpost.json("search", "picnic")["threads"]
    reply = airpost.json("draft", "reply", thread["id"], "--no-prefer-existing", "--body", "Old words")["draft"]
    again = airpost.json("draft", "edit", reply["id"], "--body", "New words")["draft"]
    assert again["id"] == reply["id"]
    assert again["body"].startswith("New words") and "Old words" not in again["body"]
    assert "gmail_quote" in again["body"]

    r = airpost.run("draft", "edit", new["id"], expect=2)
    assert "nothing to change" in r.stderr
    r = airpost.run("draft", "edit", new["id"], "--attach", str(tmp_path / "missing.pdf"), expect=2)
    assert "missing.pdf" in r.stderr
    r = airpost.run("draft", "edit", new["id"], "--detach", "nope.doc", expect=1)
    assert "no file" in r.stderr

    deleted = airpost.json("draft", "delete", reply["id"])
    assert deleted["id"] == reply["id"] and deleted["cancelledSend"] is False
    assert reply["id"] not in {x["id"] for x in airpost.json("draft", "list")["drafts"]}
    r = airpost.run("draft", "delete", reply["id"], expect=1)
    assert "no draft" in r.stderr

    checked = airpost.json("draft", "validate", new["id"])
    assert checked["ok"] is True and checked["errors"] == [] and checked["id"] == new["id"]
    nobody = airpost.json("draft", "new", "--subject", "Nobody", "--body", "x")["draft"]
    r = airpost.run("draft", "validate", nobody["id"], expect=1)
    assert json.loads(r.stdout)["ok"] is False and "can't be sent" in r.stderr
    airpost.json("draft", "delete", nobody["id"])

    before = len(run.server.smtp.messages)
    sent = airpost.json("draft", "send", new["id"], "--wait", "--timeout", "60", timeout=90)
    assert sent["status"] == "complete" and sent.get("error") is None, sent
    assert len(run.server.smtp.messages) == before + 1
    mail = run.server.smtp.messages[-1]
    assert mail.recipients == ["ann@example.net"]
    assert b"Second take" in mail.raw and b"notes.txt" in mail.raw and b"report.pdf" not in mail.raw


def test_cli_templates_and_warnings(serve_engine, tmp_path):
    """`airpost template` over templates.* on the desktop's template files, `--template` on new
    and reply drafts with draft.validate flagging the unfilled variable, a template saved
    from a draft; and the phishing warning in `airpost show`."""
    run = serve_engine
    airpost = Airpost(run)
    folder = Path(airpost.config_dir) / "templates"
    folder.mkdir(exist_ok=True)
    # As the desktop writes them.
    (folder / "Follow up.html").write_text(
        '<meta name="subject" content="Checking in"/>\n'
        '<div>Hi <span data-tvar="first_name" class="template-variable" title="first_name">First</span>,'
        ' any news?</div>')
    (folder / "Plain.html").write_text("<div>No subject here</div>")

    listed = airpost.json("template", "list")["templates"]
    assert [(t["name"], t["subject"], t["variables"]) for t in listed] == [
        ("Follow up", "Checking in", ["first_name"]), ("Plain", "", [])]
    shown = airpost.json("template", "show", "Follow up")["template"]
    assert 'data-tvar="first_name"' in shown["body"]
    assert "First, any news?" in airpost.run("template", "show", "Follow up", json_out=False).stdout

    new = airpost.json("draft", "new", "--to", "ann@example.net", "--template", "Follow up")["draft"]
    assert new["subject"] == "Checking in" and new["body"].startswith("<div>Hi <span")
    checked = airpost.json("draft", "validate", new["id"])
    assert [w["code"] for w in checked["warnings"]] == ["template-variable"]
    airpost.json("draft", "edit", new["id"], "--body", "Hi Ann, any news?")
    assert airpost.json("draft", "validate", new["id"])["warnings"] == []

    [thread] = airpost.json("search", "picnic")["threads"]
    reply = airpost.json("draft", "reply", thread["id"], "--no-prefer-existing", "--template", "Follow up")["draft"]
    assert reply["subject"] == f"Re: {DRAFT_SUBJECT}"
    assert reply["body"].startswith("<div>Hi <span") and "Saturday at the lake?" in reply["body"]
    r = airpost.run("draft", "new", "--template", "Nope", expect=1)
    assert "no template" in r.stderr

    saved = airpost.json("template", "save", "--from-draft", new["id"])["template"]
    assert saved["name"] == "Checking in" and saved["subject"] == "Checking in"
    assert (folder / "Checking in.html").read_text().startswith(
        '<meta name="subject" content="Checking in"/>\nHi Ann, any news?')
    typed = airpost.json("template", "save", "Quick", "--body", "Thanks {{name}}!")["template"]
    assert typed["variables"] == ["name"]
    assert airpost.json("template", "save", "Quick", "--body", "x")["template"]["name"] == "Quick 1"
    airpost.json("template", "delete", "Quick 1")
    assert not (folder / "Quick 1.html").exists()
    r = airpost.run("template", "save", "../etc", "--body", "x", expect=1)
    assert "Invalid template name" in r.stderr
    for d in (new, reply):
        airpost.json("draft", "delete", d["id"])

    [phish] = airpost.json("search", "walrus")["threads"]
    thread = airpost.json("show", phish["id"])
    [m] = thread["messages"]
    assert [w["code"] for w in m["warnings"]] == ["reply-to-mismatch"]
    text = airpost.run("show", m["id"], json_out=False).stdout
    assert ("Warning: This message looks suspicious! It originates from alerts@bank.example.com "
            "but replies will go to help@bank-support.example.org.") in " ".join(text.split())
    picnic = airpost.json("show", airpost.json("search", "picnic")["threads"][0]["id"])
    assert all("warnings" not in x for x in picnic["messages"])


def test_cli_skill(tmp_path):
    """`airpost skill install|print|uninstall` in a temporary HOME and project, and `airpost status`
    warning about an installed skill older than the binary. No engine needed."""
    home, project = tmp_path / "home", tmp_path / "project"
    home.mkdir()
    project.mkdir()
    env = {**os.environ, "HOME": str(home), "NO_COLOR": "1"}
    env.pop("AIRPOST_CONFIG_DIR", None)
    binary = str(airpost_binary())

    def airpost(*args, expect=0):
        r = subprocess.run([binary, *args], capture_output=True, text=True, timeout=30, env=env,
                           cwd=project)
        assert r.returncode == expect, f"airpost {' '.join(args)}: exit {r.returncode}\n{r.stdout}\n{r.stderr}"
        return r

    version = airpost("--version").stdout.split()[-1]
    source = (REPO_ROOT / "skills" / "airpost" / "SKILL.md").read_text()
    assert airpost("skill", "print").stdout == source

    done = json.loads(airpost("skill", "install", "--json").stdout)["skills"]
    assert [(d["agent"], d["result"], d["version"]) for d in done] == [
        ("claude", "installed", version), ("codex", "installed", version)]
    claude = home / ".claude" / "skills" / "airpost" / "SKILL.md"
    codex = home / ".agents" / "skills" / "airpost" / "SKILL.md"
    installed = claude.read_text()
    assert f'\nmetadata:\n  airpost-version: "{version}"\n---\n' in installed
    assert codex.read_text() == installed
    assert installed.replace(f'\nmetadata:\n  airpost-version: "{version}"', "") == source

    no_profile = str(tmp_path / "profile")
    r = airpost("--config-dir", no_profile, "status", expect=3)
    assert "warning" not in r.stderr
    claude.write_text(installed.replace(version, "0.0.0-old"))
    r = airpost("--config-dir", no_profile, "status", expect=3)
    assert f"{claude.parent} is from airpost 0.0.0-old" in r.stderr and "airpost skill install`" in r.stderr
    skills = json.loads(airpost("--config-dir", no_profile, "--json", "status", expect=3).stdout)["skills"]
    assert sorted((s["agent"], s["stale"]) for s in skills) == [("claude", True), ("codex", False)]

    r = airpost("skill", "install", "--agent", "claude")
    assert "Updated the airpost skill 0.0.0-old to" in r.stdout
    assert "warning" not in airpost("--config-dir", no_profile, "status", expect=3).stderr

    theirs = project / ".claude" / "skills" / "airpost"
    theirs.mkdir(parents=True)
    (theirs / "SKILL.md").write_text("---\nname: airpost\n---\nhand-written\n")
    r = airpost("skill", "install", "--project", expect=1)
    assert "isn't a skill airpost installed" in r.stderr and "--force" in r.stderr
    assert not (project / ".agents").exists()
    airpost("skill", "install", "--project", "--force")
    assert "airpost-version" in (theirs / "SKILL.md").read_text()
    projects = [s for s in json.loads(airpost("--config-dir", no_profile, "--json", "status", expect=3).stdout)["skills"]
                if s["project"]]
    assert len(projects) == 2

    done = json.loads(airpost("skill", "uninstall", "--json").stdout)["skills"]
    assert [d["result"] for d in done] == ["removed", "removed"]
    assert not claude.parent.exists() and not codex.parent.exists()
    assert [d["result"] for d in json.loads(airpost("skill", "uninstall", "--json").stdout)["skills"]] == [
        "absent", "absent"]
    airpost("skill", "uninstall", "--project", "--agent", "codex")
    assert (theirs / "SKILL.md").exists() and not (project / ".agents" / "skills" / "airpost").exists()
