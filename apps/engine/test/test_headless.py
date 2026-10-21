"""
The headless server path (deploy/server, docs/self-hosting.md "Tern server"): a detached `serve`
engine with no desktop, whose secrets go to the key-file store, an account added through the
`tern` CLI, and the account coming back after a restart. A different key file can't read the
stored secrets, and the account then stays stopped rather than starting without them.

    cargo build --release -p tern-engine -p tern
    TERN_ENGINE_BIN=target/release/tern-engine python3 -m pytest apps/engine/test/test_headless.py -n 0

The CLI binary comes from TERN_BIN, as in test_cli.py.
"""
import copy
import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

import pytest

from fakeimap.smtp import FakeSmtpServer
from harness.engine import find_binary
from harness.scenario import ScenarioRun, ServerSpec
from test_cli import tern_binary

INBOX_MESSAGES = 4
SCENARIO = {
    "name": "headless-server",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {"INBOX": {"messages": INBOX_MESSAGES}},
}
WAIT = 60.0


class Server:
    """A detached engine on its own profile and key file, and `tern` pointed at it."""

    def __init__(self, work: Path, binary: Path):
        self.work = work
        self.profile = work / "server-profile"
        self.profile.mkdir(mode=0o700)
        keys = work / "keys"
        keys.mkdir(mode=0o700)
        self.key = keys / "engine.key"
        # Release builds start only when argv[0] names the product.
        self.binary = work / "tern-engine"
        self.binary.symlink_to(binary)
        self.tern = str(tern_binary())
        # macOS caps socket paths at 104 bytes; the work dir is longer.
        self.sock_dir = Path(tempfile.mkdtemp(prefix="ms-", dir="/tmp"))
        self.proc = None

    def env(self, key: Path) -> dict:
        env = {k: v for k, v in os.environ.items() if not k.startswith("TERN_")}
        env.update(CONFIG_DIR_PATH=str(self.profile), TERN_ENGINE_KEY_FILE=str(key), NO_COLOR="1")
        return env

    def start(self, key: Path = None):
        args = [str(self.binary), "--mode", "serve", "--detached", "--socket", str(self.sock_dir / "s")]
        with open(self.work / "engine-stdout.log", "ab") as out:
            self.proc = subprocess.Popen(args, env=self.env(key or self.key), stdin=subprocess.DEVNULL,
                                         stdout=out, stderr=subprocess.STDOUT)

        def answers():
            assert self.proc.poll() is None, f"the engine exited {self.proc.returncode}"
            return self.run("status", check=False).returncode == 0
        self.wait(answers, "the engine to answer")

    def stop(self):
        if self.proc is None:
            return
        self.json("rpc", "engine.shutdown", "{}")
        assert self.proc.wait(timeout=30) == 0
        self.proc = None
        assert not (self.profile / "run" / "engine.json").exists()

    def kill(self):
        if self.proc is not None and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(timeout=10)
        shutil.rmtree(self.sock_dir, ignore_errors=True)

    def run(self, *args, input=None, check=True):
        r = subprocess.run([self.tern, "--config-dir", str(self.profile), "--json", *args],
                           capture_output=True, text=True, input=input, timeout=60,
                           env=self.env(self.key))
        if check:
            assert r.returncode == 0, f"tern {' '.join(args)}: exit {r.returncode}\n{r.stdout}\n{r.stderr}"
        return r

    def json(self, *args, **kw):
        return json.loads(self.run(*args, **kw).stdout)

    def accounts(self) -> list:
        [engine] = self.json("status")["engines"]
        return engine["accounts"]

    @staticmethod
    def wait(cond, what: str, timeout: float = WAIT):
        deadline = time.monotonic() + timeout
        while not cond():
            assert time.monotonic() < deadline, f"timed out waiting for {what}"
            time.sleep(0.2)


@pytest.fixture
def setup(request):
    binary = request.config.getoption("--engine")
    run = ScenarioRun(SCENARIO, ServerSpec.parse({"fake": "dovecot"}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    # The harness's own engine stays unstarted: the server's engine is the one under test.
    account = copy.deepcopy(run.ms.account)
    run.ms = None
    # `account.add {test}` checks SMTP too, which the quick tier's fake server doesn't run.
    smtp = FakeSmtpServer().start()
    account["settings"]["smtp_port"] = smtp.port
    server = Server(run.work, Path(binary).resolve() if binary else find_binary())
    try:
        yield run, server, account
    finally:
        server.kill()
        smtp.stop()
        run.teardown()


def test_headless_account_add_survives_a_restart(setup):
    run, server, account = setup
    settings = account["settings"]
    password = settings.pop("imap_password")
    settings.pop("smtp_password")
    account_id = account["id"]

    server.start()
    assert server.accounts() == []
    assert not server.key.exists(), "the key is made on the first secret, not at startup"

    added = server.json("rpc", "account.add", json.dumps({
        "account": account, "identity": None, "test": True,
        "secrets": {"imap_password": password, "smtp_password": password},
    }))
    assert added == {"accountId": account_id, "state": "running", "stored": True}, added

    def inbox_synced():
        return len(server.json("threads", "--folder", "inbox", "-n", "50")["threads"]) == INBOX_MESSAGES
    server.wait(inbox_synced, "the inbox to sync")

    assert server.key.stat().st_mode & 0o777 == 0o600
    for name in ("accounts.json", "secrets.enc"):
        assert password.encode() not in (server.profile / name).read_bytes(), name
    registered = json.loads((server.profile / "accounts.json").read_text())
    assert "imap_password" not in json.dumps(registered)

    # Back after a restart, from accounts.json and the key-file store.
    server.stop()
    server.start()
    server.wait(lambda: [(a["accountId"], a["state"]) for a in server.accounts()] == [(account_id, "running")],
                "the registered account to start")
    assert inbox_synced()
    server.stop()

    # Another key can't open secrets.enc: the account is listed but doesn't run.
    server.start(key=server.work / "keys" / "other.key")
    server.wait(lambda: [(a["accountId"], a["state"]) for a in server.accounts()] == [(account_id, "stopped")],
                "the account to stay stopped")
    [stopped] = server.accounts()
    assert stopped["error"]["key"] == "ErrorAccountNotStarted", stopped
    server.stop()

    # ...and left it alone: the right key still opens it.
    server.start()
    server.wait(lambda: [(a["accountId"], a["state"]) for a in server.accounts()] == [(account_id, "running")],
                "the account to start with its own key again")
    server.stop()
