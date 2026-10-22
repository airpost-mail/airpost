"""
End-to-end test of the `tern` CLI's send pipeline and files on a `serve` engine: send
--at and --undo, `draft scheduled` and `cancel-send`, send reminders on drafts and threads,
tracking refused without a server, `attachment get|save` and `export` (.eml and mbox).

    cargo build --release -p tern-engine -p tern
    TERN_ENGINE_BIN=target/release/tern-engine python3 -m pytest apps/engine/test/test_cli_send.py -n 0
"""
import time
from pathlib import Path

import pytest

from harness.scenario import ScenarioRun, ServerSpec
from test_cli import Tern

FILE_SUBJECT = "Otter census"
PAYLOAD = "binary payload"
INBOX_MESSAGES = 3

SCENARIO = {
    "name": "cli-send",
    "servers": [{"fake": "dovecot", "smtp": True}],
    "account": {"extra_settings": {"save_drafts_to_server": True}},
    "env": {"TERN_DRAFT_PUSH_QUIET": "1"},
    "mailboxes": {"INBOX": {"messages": INBOX_MESSAGES - 1}, "Drafts": {}, "Sent": {}},
    "setup": [
        {"server.append": {"mailbox": "INBOX", "messages": {
            "count": 1, "subject": FILE_SUBJECT, "attachment": ["census.bin", PAYLOAD]}}},
    ],
}


@pytest.fixture(scope="module")
def engine(request):
    binary = request.config.getoption("--engine")
    run = ScenarioRun(dict(SCENARIO), ServerSpec.parse({"fake": "dovecot", "smtp": True}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = "serve"
    try:
        run.ms.start()
        run.ms.wait_quiescent(timeout=180)
        yield run
    finally:
        run.teardown()


def plugin(model: dict, plugin_id: str):
    return next((m["value"] for m in model.get("metadata") or [] if m["pluginId"] == plugin_id), None)


def test_cli_send_later_undo_and_cancel(engine):
    """`send --at` writes the send-later metadata the desktop shows; `send --undo` goes
    through draft.send's delay; both are listed by `draft scheduled` and stopped by
    `cancel-send`, which leaves the draft a draft."""
    tern = Tern(engine)
    later = tern.json("draft", "new", "--to", "ann@example.net", "--subject", "Later", "--body", "x")["draft"]
    before = time.time()
    r = tern.json("draft", "send", later["id"], "--at", "2d")
    assert r["id"] == later["id"] and abs(r["sendAt"] - (before + 2 * 86400)) < 60
    value = plugin(tern.json("show", later["id"])["message"], "send-later")
    assert value["expiration"] == r["sendAt"] and value["actionKey"] == "send"

    undo = tern.json("draft", "new", "--to", "bob@example.net", "--subject", "Undo me", "--body", "y")["draft"]
    r = tern.json("draft", "send", undo["id"], "--undo", "600")
    assert r["undo"] is True and r["sendAt"] > time.time() + 500

    rows = {x["id"]: x for x in tern.json("draft", "scheduled")["scheduled"]}
    assert rows[later["id"]]["state"] == "scheduled" and rows[undo["id"]]["state"] == "undo"
    text = tern.run("draft", "scheduled", json_out=False).stdout.splitlines()
    assert text[0].split() == ["SEND", "AT", "STATE", "TO", "SUBJECT", "ID"] and len(text) == 3

    for d in (later, undo):
        assert tern.json("draft", "cancel-send", d["id"])["cancelled"] is True
    assert tern.json("draft", "scheduled")["scheduled"] == []
    assert {later["id"], undo["id"]} <= {x["id"] for x in tern.json("draft", "list")["drafts"]}
    r = tern.run("draft", "cancel-send", later["id"], expect=1)
    assert "no pending send" in r.stderr
    assert engine.server.smtp.messages == []

    tern.run("draft", "send", later["id"], "--at", "2d", "--undo", "5", expect=2)
    tern.run("draft", "send", later["id"], "--at", "yesterday", expect=2)
    for d in (later, undo):
        tern.json("draft", "delete", d["id"])


def test_cli_reminders_and_tracking(engine):
    """--remind-in rides on the draft as `thread:send-reminders`; `tern remind` sets and
    clears a thread's `send-reminders`; tracking is refused with no Tern server."""
    tern = Tern(engine)
    out = tern.json("draft", "new", "--to", "ann@example.net", "--subject", "Nudge", "--body", "z",
                    "--remind-in", "3d")
    d = out["draft"]
    assert abs(out["remindAt"] - (time.time() + 3 * 86400)) < 60
    value = plugin(tern.json("show", d["id"])["message"], "thread:send-reminders")
    assert value == {"expiration": out["remindAt"], "sentHeaderMessageId": d["hMsgId"]}

    [thread] = tern.json("search", "otter")["threads"]
    r = tern.json("remind", thread["id"], "--in", "2d")
    assert r["threadId"] == thread["id"] and r["remindAt"] > time.time()
    value = plugin(tern.json("show", thread["id"])["thread"], "send-reminders")
    assert value["expiration"] == r["remindAt"] and value["shouldNotify"] is False
    assert "lastReplyTimestamp" in value
    assert tern.json("remind", thread["id"], "--clear")["remindAt"] is None
    assert plugin(tern.json("show", thread["id"])["thread"], "send-reminders") == {}

    r = tern.run("draft", "edit", d["id"], "--track-opens", expect=1)
    assert "Tern server" in r.stderr or "update Tern" in r.stderr
    tern.run("draft", "new", "--to", "a@example.net", "--plain", "--track-links", "--body", "q", expect=2)
    assert tern.json("tracking", "list")["messages"] == []
    tern.json("draft", "delete", d["id"])


def test_cli_attachments_and_export(engine, tmp_path):
    """`show` lists file ids; `attachment get` and `save` write them without overwriting;
    `export` writes a message's .eml and a folder's mbox."""
    tern = Tern(engine)
    [thread] = tern.json("search", "otter")["threads"]
    shown = tern.json("show", thread["id"])
    [message] = [m for m in shown["messages"] if m["subject"] == FILE_SUBJECT]
    [f] = message["files"]
    assert f"id {f['id']}" in tern.run("show", message["id"], json_out=False).stdout

    out = tmp_path / "got.bin"
    got = tern.json("attachment", "get", f["id"], "-o", str(out))
    assert got["path"] == str(out) and out.read_text() == PAYLOAD and got["size"] == len(PAYLOAD)
    r = tern.run("attachment", "get", f["id"], "-o", str(out), expect=1)
    assert "--force" in r.stderr
    tern.json("attachment", "get", f["id"], "-o", str(out), "--force")
    raw = tern.run("attachment", "get", f["id"], "-o", "-", json_out=False).stdout
    assert raw == PAYLOAD

    saved = tern.json("attachment", "save", message["id"], "--dir", str(tmp_path / "files"))
    [s] = saved["files"]
    assert Path(s["path"]).name == "census.bin" and Path(s["path"]).read_text() == PAYLOAD
    tern.run("attachment", "save", message["id"], "--dir", str(tmp_path / "files"), expect=1)

    eml = tmp_path / "otter.eml"
    r = tern.json("export", message["id"], "-o", str(eml), timeout=150)
    assert r["path"] == str(eml) and FILE_SUBJECT.encode() in eml.read_bytes()
    tern.run("export", message["id"], "-o", str(eml), expect=1)

    mbox = tmp_path / "inbox.mbox"
    r = tern.json("export", "--mbox", "--folder", "inbox", "-o", str(mbox), timeout=150)
    assert r["messages"] == INBOX_MESSAGES, r
    lines = mbox.read_bytes().split(b"\n")
    assert lines[0].startswith(b"From ")
    assert sum(1 for line in lines if line.startswith(b"From ")) == INBOX_MESSAGES
    assert FILE_SUBJECT.encode() in mbox.read_bytes()
    assert not list(tmp_path.glob("inbox.mbox.*"))
