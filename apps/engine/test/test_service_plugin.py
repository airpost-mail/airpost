"""
The sample service plugin (examples/service-plugin)
against a serve engine on the fake IMAP server: it waits while there is no engine, connects,
and acts on the metadata-expiration deltas of its own plugin id only (stars the thread, marks
it unread, clears the expiration), leaving other plugins' due rows alone.

    cargo build -p mailsync -p tern-sample-service
    MAILSYNC_BIN=target/debug/mailsync python3 -m pytest apps/engine/test/test_service_plugin.py -n 0

The service binary comes from SAMPLE_SERVICE_BIN, else the newer of
target/release/tern-sample-service and target/debug/tern-sample-service. Without one the test
skips, unless CI or REQUIRE_SAMPLE_SERVICE is set. No unit is installed or loaded: the test
runs the binary itself, as the unit would.
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

PLUGIN_ID = "tern-sample-service"

SCENARIO = {
    "name": "service-plugin-sample",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {"INBOX": {"messages": 3}},
}


def sample_binary() -> Path:
    if os.environ.get("SAMPLE_SERVICE_BIN"):
        return Path(os.environ["SAMPLE_SERVICE_BIN"])
    candidates = [REPO_ROOT / "target" / p / "tern-sample-service" for p in ("release", "debug")]
    built = [c for c in candidates if c.is_file() and os.access(c, os.X_OK)]
    if not built:
        msg = ("no tern-sample-service binary: cargo build -p tern-sample-service, or set "
               "SAMPLE_SERVICE_BIN. Tried: " + ", ".join(map(str, candidates)))
        if os.environ.get("CI") or os.environ.get("REQUIRE_SAMPLE_SERVICE"):
            pytest.fail(msg)
        pytest.skip(msg)
    return max(built, key=lambda c: c.stat().st_mtime)


class Service:
    """The running sample, its stdout read as JSON lines on a thread."""

    def __init__(self, config_dir: Path):
        env = {k: v for k, v in os.environ.items() if k not in ("TERN_CONFIG_DIR", "TERN_SERVICE_PACKAGE")}
        self.proc = subprocess.Popen([str(sample_binary()), "--config-dir", str(config_dir)],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        self.lines: queue.Queue = queue.Queue()
        self.seen: list = []
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            self.lines.put(json.loads(line))

    def next(self, predicate, timeout: float) -> dict:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                doc = self.lines.get(timeout=max(0.05, deadline - time.monotonic()))
            except queue.Empty:
                break
            self.seen.append(doc)
            if predicate(doc):
                return doc
        raise AssertionError(f"no matching event within {timeout}s; got {self.seen[-5:]}; "
                             f"stderr: {self.stderr()}")

    def stderr(self) -> str:
        if self.proc.poll() is None:
            return "(running)"
        return self.proc.stderr.read()[-2000:]

    def stop(self):
        if self.proc.poll() is None:
            self.proc.terminate()
        self.proc.wait(timeout=10)


@pytest.fixture
def engine(request):
    binary = request.config.getoption("--mailsync")
    run = ScenarioRun(dict(SCENARIO), ServerSpec.parse({"fake": "dovecot"}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = "serve"
    try:
        run.ms.start()
        run.ms.wait_quiescent(timeout=180)
        yield run
    finally:
        run.teardown()


def thread_row(ms, thread_id):
    with ms.db() as c:
        return c.execute("SELECT unread, starred, data FROM Thread WHERE id = ?", (thread_id,)).fetchone()


def expiration(ms, thread_id, plugin_id):
    with ms.db() as c:
        row = c.execute("SELECT expiration FROM ModelPluginMetadata WHERE id = ? AND value = ?",
                        (thread_id, plugin_id)).fetchone()
    return row["expiration"] if row else "missing"


def test_sample_service_acts_on_its_own_due_rows(engine, tmp_path):
    sample_binary()
    ms = engine.ms

    # No engine in the profile: it says it is waiting and keeps running.
    idle = Service(tmp_path)
    try:
        assert idle.next(lambda e: e["event"] == "waiting", timeout=15)["configDir"] == str(tmp_path)
        time.sleep(0.5)
        assert idle.proc.poll() is None, idle.stderr()
    finally:
        idle.stop()

    with ms.db() as c:
        mine, other = [r["id"] for r in c.execute("SELECT id FROM Thread ORDER BY id LIMIT 2")]
    for t in (mine, other):
        ms.queue_task({"__cls": "ChangeUnreadTask", "threadIds": [t], "messageIds": [], "unread": False,
                       "canBeUndone": True})
    ms.wait_for(lambda: all(thread_row(ms, t)["unread"] == 0 for t in (mine, other)), timeout=60,
                what="both threads read")
    assert thread_row(ms, mine)["starred"] == 0 and thread_row(ms, other)["starred"] == 0

    service = Service(ms.config_dir)
    try:
        service.next(lambda e: e["event"] == "connected" and e["pluginId"] == PLUGIN_ID, timeout=30)
        due = int(time.time()) - 1
        for t, plugin in ((mine, PLUGIN_ID), (other, "another-plugin")):
            ms.queue_task({"__cls": "SyncbackMetadataTask", "modelId": t, "modelClassName": "thread",
                           "pluginId": plugin, "value": {"expiration": due, "note": "check"}})
        acted = service.next(lambda e: e["event"] == "acted", timeout=90)
        assert (acted["threadId"], acted["modelClass"], acted["expiration"]) == (mine, "Thread", due)
        assert acted["accountId"] == ms.account["id"]

        def done():
            row = thread_row(ms, mine)
            return row["starred"] == 1 and row["unread"] > 0 and expiration(ms, mine, PLUGIN_ID) is None
        ms.wait_for(done, timeout=60, what="the sample's thread starred, unread and its expiration cleared")
        [meta] = [m for m in json.loads(thread_row(ms, mine)["data"])["metadata"] if m["pluginId"] == PLUGIN_ID]
        assert meta["value"] == {"note": "check", "firedAt": due}

        # The other plugin's row came in the same scan, as a delta of its own, and is left alone.
        time.sleep(3)
        assert [e for e in service.seen if e["event"] == "acted"] == [acted]
        while not service.lines.empty():
            assert service.lines.get()["event"] != "acted"
        row = thread_row(ms, other)
        assert (row["starred"], row["unread"]) == (0, 0)
        assert expiration(ms, other, "another-plugin") == due
        assert service.proc.poll() is None, service.stderr()
    finally:
        service.stop()
    assert ms.running
