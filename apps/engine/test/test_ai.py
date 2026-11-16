"""
AI triage end to end: a `serve` engine on the fake IMAP
server, the fake Ollama in harness/servers/ai.py as the provider the engine keeps
(`core.ai.*` settings), and mail arriving in the inbox.

- `ai_triage_new_mail`: unread inbox mail is triaged by the engine's triage worker and
  the thread gets `airpost-ai-triage` metadata; nothing else is asked.
- `ai_triage_rate_limit_pause`: the provider answers 429 with Retry-After; the queue
  pauses, resumes and then stores the result.
- `airpost ai summarize|triage|usage` against the same engine, when a `airpost` binary is built.

    AIRPOST_ENGINE_BIN=target/release/airpost-engine python3 -m pytest apps/engine/test/test_ai.py -n 0

The engine runs as `serve` whatever AIRPOST_ENGINE_TRANSPORT says: only serve has the triage
worker. The CLI test skips without a `airpost` binary, as test_cli.py does.
"""
import json
import subprocess
from pathlib import Path

import pytest

from harness.scenario import ScenarioRun, ServerSpec
from harness.servers.ai import MODEL, FakeOllama
from test_cli import airpost_binary

PLUGIN = "airpost-ai-triage"
SENDER = ("Ann Example", "ann@example.test")
NEW_SUBJECT = "Lunch on Friday?"
STORED = r"^- AI triage stored for thread \S+$"
PAUSED = r"^-X AI triage paused for \d+ s: rate-limited$"
RESUMED = r"^AI triage resumed$"


@pytest.fixture
def ai_run(request):
    runs = []

    def start(name: str, fake: FakeOllama, features: dict) -> ScenarioRun:
        binary = request.config.getoption("--engine")
        scenario = {"name": name, "servers": [{"fake": "dovecot"}], "mailboxes": {"INBOX": {"messages": 3}}}
        run = ScenarioRun(scenario, ServerSpec.parse({"fake": "dovecot"}),
                          binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
        runs.append((run, fake))
        fake.start()
        run.setup()
        run.ms.transport = "serve"
        run.ms.start()
        run.ms.wait_quiescent(timeout=180)
        configure(run, fake, features)
        return run

    yield start
    for run, fake in runs:
        try:
            run.teardown()
        finally:
            fake.stop()


def configure(run: ScenarioRun, fake: FakeOllama, features: dict):
    run.ms.set_settings({
        "core.ai.providers": [fake.provider()],
        "core.ai.features": features,
        "core.ai.accounts": [run.ms.account["id"]],
    })


TRIAGE_ON = {"triage": {"enabled": True, "provider": "harness"}}


def deliver(run: ScenarioRun):
    run.run_step({"server.append": {"mailbox": "INBOX", "flags": [], "messages": {
        "count": 1, "sender": SENDER, "subject": NEW_SUBJECT,
        "body": "Are you free for lunch on Friday?\r\n"}}})


def wait_triaged(run: ScenarioRun, timeout: float = 60):
    run.run_step({"wait": {"log": STORED, "timeout": timeout}})
    run.run_step({"wait": {"metadata": [{"threads": {"mailbox": "INBOX", "uids": [4]}, "plugin": PLUGIN,
                                         "value": {"category": "needs-reply", "needsReply": True}}],
                           "timeout": timeout}})


def test_ai_triage_new_mail(ai_run):
    fake = FakeOllama()
    run = ai_run("ai-triage-new-mail", fake, TRIAGE_ON)
    deliver(run)
    wait_triaged(run)
    # Only the new mail: the three read messages from the first sync are not new.
    assert fake.chat_count() == 1, fake.requests
    sent = json.dumps(fake.requests[0])
    assert fake.requests[0]["model"] == MODEL
    assert "Lunch on Friday" in sent
    # core.ai.includeAddresses is off.
    assert SENDER[1] not in sent
    run.check_expectations({
        "metadata": [{"threads": {"mailbox": "INBOX", "uids": [1, 2, 3]}, "plugin": PLUGIN, "absent": True}],
        "running": True,
    })
    assert run.ms.stderr_tail() == ""


def test_ai_triage_rate_limit_pause(ai_run):
    fake = FakeOllama(script=[{"status": 429, "retry_after": 1}])
    run = ai_run("ai-triage-rate-limit-pause", fake, TRIAGE_ON)
    deliver(run)
    run.run_step({"wait": {"log": PAUSED, "timeout": 60}})
    run.run_step({"wait": {"log": RESUMED, "timeout": 30}})
    wait_triaged(run)
    assert fake.chat_count() == 2, fake.requests
    # Unavailable while paused, then available again.
    reasons = [c.get("ai", {}).get("triage", {}).get("reason") for c in run.ms.engine.capabilities]
    assert "rate-limited" in reasons, run.ms.engine.capabilities
    assert run.ms.engine.capabilities[-1]["ai"]["triage"]["available"] is True
    run.check_expectations({"running": True})


def test_airpost_ai_commands(ai_run):
    airpost = str(airpost_binary())
    fake = FakeOllama(script=[{"text": "Ann asks about lunch on Friday."}])
    run = ai_run("ai-cli", fake, {"summary": {"enabled": True, "provider": "harness"}})

    def cli(*args, expect=0):
        r = subprocess.run([airpost, "--config-dir", str(run.ms.config_dir), *args], capture_output=True,
                           text=True, timeout=120, env={"NO_COLOR": "1", "PATH": "/usr/bin:/bin"})
        assert r.returncode == expect, f"airpost {' '.join(args)}: exit {r.returncode}\n{r.stdout}\n{r.stderr}"
        assert (r.stderr == "") == (expect == 0), r.stderr
        return r.stdout if expect == 0 else r.stderr

    [thread] = run._resolve_threads({"mailbox": "INBOX", "uids": [1]})
    out = cli("ai", "summarize", thread)
    assert out.startswith("Ann asks about lunch on Friday.\n"), out
    assert MODEL in out
    # Cached: the fake is not asked again.
    doc = json.loads(cli("--json", "ai", "summarize", thread))
    assert doc["cached"] is True and doc["text"] == "Ann asks about lunch on Friday."
    assert fake.chat_count() == 1

    # Triage is off: the engine refuses, and the CLI says so.
    assert "turned off" in cli("ai", "triage", thread, expect=1)
    configure(run, fake, {"summary": {"enabled": True, "provider": "harness"}, **TRIAGE_ON})
    doc = json.loads(cli("--json", "ai", "triage", thread))
    [acct] = doc["accounts"]
    assert acct["queued"] == 1 and acct["done"] == 1, doc
    run.run_step({"wait": {"metadata": [{"threads": {"mailbox": "INBOX", "uids": [1]}, "plugin": PLUGIN,
                                         "value": {"category": "needs-reply"}}], "timeout": 30}})

    rows = json.loads(cli("--json", "ai", "usage"))
    features = {r["feature"] for r in rows if r["provider"] == "harness"}
    assert {"summary", "triage"} <= features, rows
    assert "harness" in cli("ai", "usage")
