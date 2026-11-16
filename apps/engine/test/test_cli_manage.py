"""
End-to-end test of `airpost spam`, `not-spam`, `folder empty`, `signature`, `rules` and
`calendars` (apps/cli/src/organize.rs, signature.rs, rules.rs, pim.rs) against a `serve`
engine on the fake IMAP server.

    cargo build --release -p airpost-engine -p airpost
    AIRPOST_ENGINE_BIN=target/release/airpost-engine python3 -m pytest apps/engine/test/test_cli_manage.py -n 0

The CLI binary is found as in test_cli.py.
"""
import pytest

from test_cli import Airpost
from test_cli_organize import INVOICES, eventually, in_mailbox, invoice_ids, scenario, start


@pytest.fixture(scope="module")
def imap(request):
    run = start(request, scenario("cli-manage-imap", {
        "INBOX": {"messages": 3},
        "Junk": {"special_use": "\\Junk"},
        "Trash": {"special_use": "\\Trash", "messages": 2},
        "Archive": {"special_use": "\\Archive"},
        "Sent": {"special_use": "\\Sent"},
    }), {"fake": "dovecot"})
    try:
        yield run
    finally:
        run.teardown()


def test_cli_spam_and_empty(imap):
    """A thread to spam and back, Trash emptied after a dry run and a refused unconfirmed run."""
    run = imap
    airpost = Airpost(run)
    ids = invoice_ids(airpost)
    inbox = len(in_mailbox(run, "INBOX"))

    spammed = airpost.json("spam", ids[0], "--wait", timeout=90)
    assert spammed["accounts"][0]["threadIds"] == [ids[0]]
    assert len(in_mailbox(run, "Junk")) == 1 and len(in_mailbox(run, "INBOX")) == inbox - 1
    back = airpost.json("not-spam", ids[0], "--wait", timeout=90)
    [task] = back["tasks"]
    assert task["__cls"] == "ChangeFolderTask" and task.get("error") is None
    assert in_mailbox(run, "Junk") == [] and len(in_mailbox(run, "INBOX")) == inbox
    again = airpost.json("not-spam", ids[0])["accounts"][0]
    assert again["threadIds"] == [] and again["unchangedThreadIds"] == [ids[0]]

    dry = airpost.json("folder", "empty", "trash", "--dry-run")
    [acct] = dry["accounts"]
    assert dry["dryRun"] is True and acct["folder"]["role"] == "trash" and acct["threads"] == 2
    r = airpost.run("folder", "empty", "trash", expect=2)
    assert "--yes" in r.stderr
    assert len(in_mailbox(run, "Trash")) == 2
    r = airpost.run("folder", "empty", "INBOX", "--yes", expect=1)
    assert "can be emptied" in r.stderr
    done = airpost.json("folder", "empty", "trash", "--yes", "--wait", timeout=90)
    [acct] = done["accounts"]
    assert acct["task"]["__cls"] == "ExpungeAllInFolderTask" and acct["task"].get("error") is None
    eventually("Trash is empty on the server", lambda: in_mailbox(run, "Trash") == [])


def test_cli_signatures(imap):
    airpost = Airpost(imap)
    email = imap.ms.account["emailAddress"]
    made = airpost.json("signature", "set", "Work", "--body", "Ann\nShop")["signature"]
    assert made["title"] == "Work" and "<br" in made["body"]
    airpost.json("signature", "default", "work")
    [sig] = [s for s in airpost.json("signature", "list")["signatures"] if s["id"] == made["id"]]
    assert sig["defaultFor"] == [email]
    airpost.json("signature", "set", made["id"], "--title", "Office")
    gone = airpost.json("signature", "delete", "Office")
    assert gone["defaultFor"] == [email]
    assert airpost.json("signature", "list")["defaults"][email] is None


def test_cli_rules(imap):
    """A bad rule refused, a rule added, listed, turned off and on, and run over the inbox."""
    run = imap
    airpost = Airpost(run)
    r = airpost.run("rules", "add", "--if", "subject matches (x", "--then", "star", expect=2)
    assert "not an expression" in r.stderr
    r = airpost.run("rules", "add", "--if", "subject contains invoice", "--then", "move Nowhere", expect=1)
    assert "no folder Nowhere" in r.stderr
    folder = airpost.json("folder", "create", "Receipts")["folder"]

    rule = airpost.json("rules", "add", "--name", "Receipts", "--if", "subject contains invoice",
                     "--then", "move Receipts")["rule"]
    assert rule["actions"] == [{"templateKey": "changeFolder", "value": folder["id"], "valueName": "Receipts"}]
    assert [x["id"] for x in airpost.json("rules", "list")["rules"]] == [rule["id"]]
    assert airpost.json("rules", "disable", "Receipts")["rule"]["disabled"] is True
    r = airpost.run("rules", "reprocess", expect=1)
    assert "no rule that is on" in r.stderr
    airpost.json("rules", "enable", rule["id"])

    ran = airpost.json("rules", "reprocess", "--wait", "--timeout", "120", timeout=150)
    [acct] = ran["accounts"]
    assert acct["stopped"] is False and acct["done"] == acct["total"] > 0
    assert ran["disabledRules"] == []
    eventually("the invoices reach Receipts",
               lambda: len(in_mailbox(run, folder["path"])) == len(INVOICES), timeout=60)
    airpost.json("rules", "remove", "Receipts")
    assert airpost.json("rules", "list")["rules"] == []


def test_cli_calendars(imap):
    assert isinstance(Airpost(imap).json("calendars")["calendars"], list)
