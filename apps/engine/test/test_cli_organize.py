"""
End-to-end test of the `tern` commands that change mailboxes (apps/cli/src/organize.rs):
archive, trash, move, mark-read/unread, star, label add/remove and label/folder
create/rename/delete, by thread id and by --query, against a `serve` engine on the fake
IMAP server, once with plain folders and once on the Gmail fake with labels.

    cargo build --release -p tern-engine -p tern
    TERN_ENGINE_BIN=target/release/tern-engine python3 -m pytest apps/engine/test/test_cli_organize.py -n 0

The CLI binary is found as in test_cli.py.
"""
import time
from pathlib import Path

import pytest

from harness.scenario import ScenarioRun, ServerSpec
from test_cli import Tern

INVOICES = [f"Invoice {n} from the shop" for n in (1001, 1002, 1003)]
SHOP = ["Shop Billing", "billing@shop.example.net"]


def scenario(name: str, mailboxes: dict) -> dict:
    return {
        "name": name,
        "mailboxes": mailboxes,
        "setup": [{"server.append": {"mailbox": "INBOX", "messages": {"count": 1, "subject": s, "sender": SHOP}}}
                  for s in INVOICES],
    }


def start(request, sc: dict, server: dict):
    binary = request.config.getoption("--engine")
    run = ScenarioRun(sc, ServerSpec.parse(server), binary=Path(binary) if binary else None,
                      keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = "serve"
    run.ms.start()
    run.ms.wait_quiescent(timeout=180)
    return run


@pytest.fixture(scope="module")
def imap(request):
    run = start(request, scenario("cli-organize-imap", {
        "INBOX": {"messages": 3},
        "Archive": {"special_use": "\\Archive"},
        "Trash": {"special_use": "\\Trash"},
        "Sent": {"special_use": "\\Sent"},
    }), {"fake": "dovecot"})
    try:
        yield run
    finally:
        run.teardown()


@pytest.fixture(scope="module")
def gmail(request):
    run = start(request, scenario("cli-organize-gmail", {"INBOX": {"messages": 3}}), {"fake": "gmail"})
    try:
        yield run
    finally:
        run.teardown()


def in_mailbox(run, name: str) -> list:
    return list(run.server.truth().get(name, {}).values())


def eventually(what: str, check, timeout: float = 30):
    deadline = time.monotonic() + timeout
    while not check():
        assert time.monotonic() < deadline, what
        time.sleep(0.3)


def invoice_ids(tern) -> list:
    threads = tern.json("search", "subject:invoice")["threads"]
    assert sorted(t["subject"] for t in threads) == INVOICES
    return [t["id"] for t in threads]


def test_cli_organize_folders(imap):
    """Plain IMAP: a folder made, filled by --query (dry run first, then for real, then again
    with nothing left to change), threads marked, starred, archived and trashed by id and by
    query, the folder refused for deletion while it holds mail, renamed, and deleted."""
    run = imap
    tern = Tern(run)
    ids = invoice_ids(tern)

    r = tern.run("label", "create", "Receipts", expect=1)
    assert "folders, not labels" in r.stderr
    r = tern.run("label", "add", "Receipts", ids[0], expect=1)
    assert "folders, not labels" in r.stderr
    r = tern.run("move", "Receipts", ids[0], expect=1)
    assert "tern folder create Receipts" in r.stderr
    tern.run("archive", ids[0], "--limit", "3", expect=2)

    made = tern.json("folder", "create", "Receipts")
    assert made["status"] == "complete" and made["folder"]["path"].endswith("Receipts")
    assert any(f["id"] == made["folder"]["id"] for f in tern.json("folders")["folders"])
    r = tern.run("folder", "create", "receipts", expect=1)
    assert "already has a folder" in r.stderr

    dry = tern.json("move", "Receipts", "--query", "subject:invoice", "--dry-run")
    assert dry["dryRun"] is True and dry["action"] == "move"
    [acct] = dry["accounts"]
    assert sorted(acct["threadIds"]) == sorted(ids) and acct["taskIds"] == []
    assert sorted(t["subject"] for t in dry["threads"]) == INVOICES
    text = tern.run("move", "Receipts", "-q", "subject:invoice", "--dry-run", json_out=False).stdout
    assert text.startswith("Would move 3 threads to Receipts:") and INVOICES[0] in text
    time.sleep(1)
    assert in_mailbox(run, made["folder"]["path"]) == [], "a dry run changes nothing"

    moved = tern.json("move", "Receipts", "--query", "subject:invoice", "--wait", "--timeout", "60", timeout=90)
    [acct] = moved["accounts"]
    assert sorted(acct["threadIds"]) == sorted(ids) and len(acct["taskIds"]) == 1
    [task] = moved["tasks"]
    assert task["__cls"] == "ChangeFolderTask" and task["status"] == "complete" and task.get("error") is None
    assert len(in_mailbox(run, made["folder"]["path"])) == 3
    assert len(in_mailbox(run, "INBOX")) == 3

    again = tern.json("move", "Receipts", "--query", "subject:invoice")
    [acct] = again["accounts"]
    assert acct["threadIds"] == [] and acct["taskIds"] == [] and len(acct["unchangedThreadIds"]) == 3
    text = tern.run("move", "Receipts", "--query", "subject:invoice", json_out=False).stdout
    assert text.strip() == "Nothing to change (3 threads already there)"

    tern.json("mark-unread", ids[0], "--wait", timeout=90)
    tern.json("star", ids[0], ids[1], "--wait", timeout=90)
    flags = {m["message_id"]: m["flags"] for m in in_mailbox(run, made["folder"]["path"])}
    assert sum("\\Seen" not in f for f in flags.values()) == 1
    assert sum("\\Flagged" in f for f in flags.values()) == 2
    starred = tern.json("star", "--query", "subject:invoice")["accounts"][0]
    assert len(starred["threadIds"]) == 1 and len(starred["unchangedThreadIds"]) == 2
    tern.json("mark-read", "--query", "subject:invoice is:unread", "--wait", timeout=90)
    eventually("all read on the server",
               lambda: all("\\Seen" in m["flags"] for m in in_mailbox(run, made["folder"]["path"])))

    tern.json("archive", ids[0], "--wait", timeout=90)
    assert len(in_mailbox(run, "Archive")) == 1
    trashed = tern.json("trash", "--query", "subject:invoice in:receipts", "--limit", "1", "--wait", timeout=90)
    assert len(trashed["accounts"][0]["threadIds"]) == 1
    assert len(in_mailbox(run, "Trash")) == 1
    assert len(in_mailbox(run, made["folder"]["path"])) == 1

    r = tern.run("folder", "delete", "Receipts", expect=1)
    assert "holds 1 thread" in r.stderr and "--force" in r.stderr
    r = tern.run("folder", "rename", "INBOX", "Elsewhere", expect=1)
    assert "inbox folder" in r.stderr

    renamed = tern.json("folder", "rename", "Receipts", "Bills")
    assert renamed["status"] == "complete" and renamed["folder"]["id"] == made["folder"]["id"]
    bills = renamed["folder"]["path"]
    assert bills.endswith("Bills") and bills in run.server.mailboxes()
    assert made["folder"]["path"] not in run.server.mailboxes()
    gone = tern.json("folder", "delete", "Bills", "--force")
    assert gone["status"] == "complete"
    assert bills not in run.server.mailboxes()

    r = tern.run("archive", "no-such-thread", expect=1)
    assert "no thread no-such-thread" in r.stderr
    r = tern.run("archive", "--query", "(x", expect=2)
    assert "--query" in r.stderr


def test_cli_organize_labels(gmail):
    """Gmail: applying a missing label refused, the label created, a backfill by --query with
    a dry run first, rerun with nothing to do, archive dropping the Inbox label, a label taken
    off, renamed and deleted."""
    run = gmail
    tern = Tern(run)
    ids = invoice_ids(tern)

    r = tern.run("label", "add", "Receipts", "--query", "subject:invoice", expect=1)
    assert "no label Receipts" in r.stderr and "tern label create Receipts" in r.stderr
    r = tern.run("folder", "create", "Receipts", expect=1)
    assert "Gmail" in r.stderr

    made = tern.json("label", "create", "Receipts")
    assert made["status"] == "complete" and made["label"]["path"] == "Receipts"
    assert any(l["id"] == made["label"]["id"] for l in tern.json("labels")["labels"])

    dry = tern.json("label", "add", "Receipts", "--query", "subject:invoice", "--dry-run")
    assert sorted(dry["accounts"][0]["threadIds"]) == sorted(ids) and dry["accounts"][0]["taskIds"] == []

    def labelled(label):
        return [m for m in in_mailbox(run, "[Gmail]/All Mail") if label in m["labels"]]

    assert labelled("Receipts") == []
    added = tern.json("label", "add", "Receipts", "--query", "subject:invoice", "--wait", timeout=90)
    [task] = added["tasks"]
    assert task["__cls"] == "ChangeLabelsTask" and task.get("error") is None
    assert len(labelled("Receipts")) == 3
    again = tern.json("label", "add", "Receipts", "--query", "subject:invoice")["accounts"][0]
    assert again["threadIds"] == [] and len(again["unchangedThreadIds"]) == 3
    assert len(tern.json("search", "in:receipts")["threads"]) == 3

    archived = tern.json("archive", "--query", "subject:invoice", "--wait", timeout=90)
    assert archived["accounts"][0]["category"]["role"] == "inbox"
    assert [m for m in labelled("\\Inbox") if m in labelled("Receipts")] == []
    assert len(labelled("\\Inbox")) == 3

    tern.json("label", "remove", "Receipts", ids[0], "--wait", timeout=90)
    assert len(labelled("Receipts")) == 2

    renamed = tern.json("label", "rename", "Receipts", "Bills")
    assert renamed["label"]["id"] == made["label"]["id"] and renamed["label"]["path"] == "Bills"
    assert len(labelled("Bills")) == 2
    tern.json("label", "delete", "Bills")
    assert "Bills" not in run.server.mailboxes()
    tern.run("sync")
    eventually("the label leaves `tern labels`",
               lambda: all(l["path"] != "Bills" for l in tern.json("labels")["labels"]), timeout=60)
