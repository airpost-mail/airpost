"""
End-to-end test of tern-mcp (apps/mcp, docs/mcp.md): a real engine on the fake IMAP server
with the socket transport, and tern-mcp speaking MCP over stdio to a minimal JSON-RPC client,
pointed at the engine's config dir with --config-dir.

    cargo build --release -p mailsync -p tern-mcp
    MAILSYNC_BIN=target/release/mailsync python3 -m pytest apps/engine/test/test_mcp.py -n 0

The server binary comes from TERN_MCP_BIN, else the newer of target/release/tern-mcp and
target/debug/tern-mcp. Without one the test skips, unless CI or REQUIRE_TERN_MCP is set.
"""
import base64
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
ATTACHMENT = b"binary payload"

SCENARIO = {
    "name": "mcp-end-to-end",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {"INBOX": {"messages": 3}},
    "setup": [
        {"server.append": {"mailbox": "INBOX", "messages": {"count": 1, "subject": SUBJECT,
                                                            "attachment": ["report.bin", ATTACHMENT.decode()]}}},
    ],
}


def mcp_binary() -> Path:
    if os.environ.get("TERN_MCP_BIN"):
        return Path(os.environ["TERN_MCP_BIN"])
    candidates = [REPO_ROOT / "target" / profile / "tern-mcp" for profile in ("release", "debug")]
    built = [c for c in candidates if c.is_file() and os.access(c, os.X_OK)]
    if not built:
        msg = "no tern-mcp binary: cargo build -p tern-mcp, or set TERN_MCP_BIN. Tried: " + ", ".join(map(str, candidates))
        if os.environ.get("CI") or os.environ.get("REQUIRE_TERN_MCP"):
            pytest.fail(msg)
        pytest.skip(msg)
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


class McpError(AssertionError):
    pass


class Mcp:
    """tern-mcp on stdio, with a JSON-RPC client over its newline-delimited stdout."""

    def __init__(self, config_dir: Path, protocol: str = "2025-06-18"):
        env = {**os.environ}
        env.pop("TERN_CONFIG_DIR", None)
        self.proc = subprocess.Popen([str(mcp_binary()), "--config-dir", str(config_dir)],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True, env=env)
        self.lines: queue.Queue = queue.Queue()
        self.stderr: list = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=lambda: self.stderr.extend(self.proc.stderr), daemon=True).start()
        self.next_id = 0
        init = self.request("initialize", {"protocolVersion": protocol, "capabilities": {},
                                           "clientInfo": {"name": "pytest", "version": "1"}})
        self.server_info = init
        self.notify("notifications/initialized")

    def _read(self):
        for line in self.proc.stdout:
            self.lines.put(line)

    def send(self, msg: dict):
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def notify(self, method: str, params=None):
        self.send({"jsonrpc": "2.0", "method": method, **({"params": params} if params is not None else {})})

    def request(self, method: str, params=None, timeout: float = 90):
        self.next_id += 1
        rid = self.next_id
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, **({"params": params} if params is not None else {})})
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise McpError(f"no answer to {method} within {timeout}s; stderr: {''.join(self.stderr)[-2000:]}")
            try:
                msg = json.loads(self.lines.get(timeout=left))
            except queue.Empty:
                continue
            if msg.get("id") != rid:
                continue   # notifications, or answers to requests this client didn't make
            if "error" in msg:
                raise McpError(f"{method}: {msg['error']}")
            return msg["result"]

    def call(self, tool: str, args: dict = None, expect_error: bool = False):
        result = self.request("tools/call", {"name": tool, "arguments": args or {}})
        text = result["content"][0]["text"]
        if expect_error:
            assert result.get("isError") is True, f"{tool} should have failed: {text}"
            return json.loads(text)["error"]
        assert not result.get("isError"), f"{tool}: {text}"
        return json.loads(text)

    def close(self):
        if self.proc.poll() is None:
            self.proc.stdin.close()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=10)


def write_policy(config_dir: Path, policy: dict):
    (config_dir / "mcp.json").write_text(json.dumps(policy))


def wait_for(predicate, what: str, timeout: float = 60):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.25)
    raise AssertionError(f"timed out waiting for {what}; last: {last}")


def drafts_in(mcp: Mcp, thread_id: str) -> list:
    return [m for m in mcp.call("get_thread", {"threadId": thread_id})["messages"] if m["draft"]]


def test_mcp_end_to_end(engine, tmp_path):
    config_dir = Path(engine.ms.config_dir)
    account_id = engine.ms.account["id"]
    email = engine.ms.account["emailAddress"]
    policy_file = config_dir / "mcp.json"
    policy_file.unlink(missing_ok=True)

    # No engine under the profile: a tool error the model can read, not a protocol error.
    lonely = Mcp(tmp_path)
    try:
        assert "isn't running" in lonely.call("list_accounts", expect_error=True)
    finally:
        lonely.close()

    mcp = Mcp(config_dir)
    try:
        assert mcp.server_info["serverInfo"]["name"] == "tern"
        assert mcp.server_info["protocolVersion"] == "2025-06-18"
        names = {t["name"] for t in mcp.request("tools/list")["tools"]}
        assert {"list_accounts", "search_mail", "get_thread", "reply_to_thread", "update_draft",
                "delete_draft", "send_draft", "get_attachment"} <= names
        assert not any("mailspring" in n for n in names)

        [account] = mcp.call("list_accounts")
        assert (account["id"], account["email"], account["usesLabels"]) == (account_id, email, False)
        folders = {f["path"]: f for f in mcp.call("list_folders", {"accountId": account_id})}
        assert folders["INBOX"]["role"] == "inbox" and "Drafts" in folders

        [found] = mcp.call("search_mail", {"query": "zebra"})
        assert found["subject"] == SUBJECT
        # The engine's search grammar: fields, OR, NOT and dates, which tern-mcp passes on.
        [same] = mcp.call("search_mail", {"query": "(subject:zebra OR nothing-like-this) NOT in:trash"})
        assert same["id"] == found["id"]
        assert mcp.call("search_mail", {"query": "zebra before:1990-01-01"}) == []
        assert "never closed" in mcp.call("search_mail", {"query": "(zebra"}, expect_error=True)
        thread_id = found["id"]
        thread = mcp.call("get_thread", {"threadId": thread_id})
        [received] = thread["messages"]
        assert received["draft"] is False
        assert "This is the body of message" in received["body"]
        assert mcp.call("get_message", {"messageId": received["id"]})["draft"] is False

        # The attachment, as a resource link and through resources/read.
        [file] = received["files"]
        att = mcp.request("tools/call", {"name": "get_attachment",
                                         "arguments": {"messageId": received["id"], "fileId": file["id"]}})
        assert not att.get("isError"), att
        meta = json.loads(att["content"][0]["text"])
        assert meta["filename"] == "report.bin"
        assert Path(meta["path"]).read_bytes() == ATTACHMENT
        link = att["content"][-1]
        assert link["type"] == "resource_link"
        [contents] = mcp.request("resources/read", {"uri": link["uri"]})["contents"]
        assert base64.b64decode(contents["blob"]) == ATTACHMENT

        # Without mcp.json or core.mcp.* keys the policy is read-only.
        err = mcp.call("set_starred", {"threadIds": [thread_id], "starred": True}, expect_error=True)
        assert "read-write" in err
        write_policy(config_dir, {"accessLevel": "read-write"})

        assert mcp.call("set_starred", {"threadIds": [thread_id], "starred": True})["updated"] == 1
        wait_for(lambda: mcp.call("get_thread", {"threadId": thread_id})["starred"], "the star")
        err = mcp.call("send_draft", {"headerMessageId": "x@y"}, expect_error=True)
        assert "read-write-send" in err

        # Replying twice: the second reply quotes the received message, not the first draft,
        # and says the thread already has a draft.
        first = mcp.call("reply_to_thread", {"threadId": thread_id, "body": "<p>first answer</p>"})
        assert first["draft"] is True and first["threadId"] == thread_id
        assert first["repliedTo"]["id"] == received["id"] and first["existingDrafts"] == []
        [d1] = wait_for(lambda: drafts_in(mcp, thread_id), "the first draft")
        assert d1["id"] == first["id"] and d1["headerMessageId"] == first["headerMessageId"]
        assert "first answer" in d1["body"]

        second = mcp.call("reply_to_thread", {"threadId": thread_id, "body": "<p>second answer</p>"})
        assert second["repliedTo"]["id"] == received["id"]
        assert [d["id"] for d in second["existingDrafts"]] == [first["id"]]
        assert "update_draft" in second["note"]
        drafts = {d["id"]: d for d in wait_for(lambda: (lambda d: d if len(d) == 2 else None)(drafts_in(mcp, thread_id)),
                                               "the second draft")}
        assert "first answer" not in drafts[second["id"]]["body"]
        assert "This is the body of message" in drafts[second["id"]]["body"]
        err = mcp.call("reply_to_thread", {"threadId": thread_id, "body": "x", "messageId": first["id"]},
                       expect_error=True)
        assert "draft" in err

        # update_draft edits the first draft in place.
        updated = mcp.call("update_draft", {"headerMessageId": first["headerMessageId"],
                                            "body": "<p>revised answer</p>", "subject": "Re: zebra, revised"})
        assert (updated["id"], updated["headerMessageId"], updated["threadId"]) == \
            (first["id"], first["headerMessageId"], thread_id)
        drafts = {d["id"]: d for d in drafts_in(mcp, thread_id)}
        assert set(drafts) == {first["id"], second["id"]}
        assert drafts[first["id"]]["body"] == "<p>revised answer</p>"
        assert drafts[first["id"]]["subject"] == "Re: zebra, revised"
        assert drafts[first["id"]]["to"] == received["from"]
        err = mcp.call("update_draft", {"id": received["id"], "body": "x"}, expect_error=True)
        assert "not found" in err

        # delete_draft removes the second.
        mcp.call("delete_draft", {"id": second["id"]})
        wait_for(lambda: [d["id"] for d in drafts_in(mcp, thread_id)] == [first["id"]], "the deleted draft to go")

        # A disabled account is invisible.
        write_policy(config_dir, {"accessLevel": "read-write", "enabledAccounts": {account_id: {"enabled": False}}})
        assert mcp.call("list_accounts") == []
        assert "not found" in mcp.call("get_thread", {"threadId": thread_id}, expect_error=True)
        assert mcp.call("search_mail", {"query": "zebra"}) == []
        # A policy file that doesn't parse fails closed.
        policy_file.write_text("{not json")
        assert "mcp.json" in mcp.call("list_accounts", expect_error=True)
    finally:
        mcp.close()
        policy_file.unlink(missing_ok=True)

    # Every call is in the audit log, with bodies redacted.
    audit = [json.loads(line) for line in (config_dir / "mcp-audit.log").read_text().splitlines()]
    tools = [a["toolName"] for a in audit]
    assert "reply_to_thread" in tools and "update_draft" in tools and "resources/read" in tools
    assert {a["outcome"] for a in audit} >= {"ok", "denied", "error"}
    assert all(a["client"].startswith("pytest") for a in audit)
    text = (config_dir / "mcp-audit.log").read_text()
    assert "revised answer" not in text and "second answer" not in text
    update = next(a for a in audit if a["toolName"] == "update_draft")
    assert update["accountId"] == account_id and '"body":"<' in update["params"]
    if os.name == "posix":
        assert (config_dir / "mcp-audit.log").stat().st_mode & 0o777 == 0o600


def test_install_writes_temp_configs_only(tmp_path):
    claude = tmp_path / "claude_desktop_config.json"
    mailspring = {"command": "/Applications/Mailspring.app/Contents/MacOS/Mailspring", "args": ["bridge.js"]}
    claude.write_text(json.dumps({"mcpServers": {"mailspring": mailspring}}))
    profile = tmp_path / "profile"
    profile.mkdir()
    env = {**os.environ, "HOME": str(tmp_path / "home"), "CODEX_HOME": str(tmp_path / "codex")}
    run = lambda *a: subprocess.run([str(mcp_binary()), "--config-dir", str(profile), *a],
                                    capture_output=True, text=True, timeout=30, env=env)
    r = run("install", "--client", "claude-desktop", "--config-file", str(claude))
    assert r.returncode == 0, r.stderr
    servers = json.loads(claude.read_text())["mcpServers"]
    assert servers["mailspring"] == mailspring
    assert servers["tern"]["args"] == ["--config-dir", str(profile)]
    r = run("install", "--client", "codex")
    assert r.returncode == 0, r.stderr
    assert "[mcp_servers.tern]" in (tmp_path / "codex" / "config.toml").read_text()
    assert run("uninstall", "--client", "claude-desktop", "--config-file", str(claude)).returncode == 0
    assert json.loads(claude.read_text())["mcpServers"] == {"mailspring": mailspring}
    status = json.loads(run("status", "--json").stdout)
    assert status["engines"] == [] and status["policy"]["accessLevel"] == "read-only"


def test_install_replaces_the_old_plugin_bridge_entry(tmp_path):
    claude = tmp_path / "claude_desktop_config.json"
    profile = tmp_path / "profile"
    profile.mkdir()
    bridge = lambda dir: {"command": "/Applications/Tern.app/Contents/MacOS/Tern",
                          "args": ["/x/app.asar.unpacked/mcp-stdio-bridge.js", "--config-dir", str(dir)],
                          "env": {"ELECTRON_RUN_AS_NODE": "1"}}
    env = {**os.environ, "HOME": str(tmp_path / "home")}
    run = lambda *a: subprocess.run([str(mcp_binary()), "--config-dir", str(profile), "install",
                                     "--client", "claude-desktop", "--config-file", str(claude), *a],
                                    capture_output=True, text=True, timeout=30, env=env)
    # Another profile's bridge entry is left alone by --if-stale.
    other = bridge(tmp_path / "other")
    claude.write_text(json.dumps({"mcpServers": {"tern": other}}))
    r = run("--if-stale")
    assert r.returncode == 0 and r.stdout.startswith("No stale"), r.stdout + r.stderr
    assert json.loads(claude.read_text())["mcpServers"]["tern"] == other
    # This profile's is replaced, without --force.
    claude.write_text(json.dumps({"mcpServers": {"tern": bridge(profile)}}))
    r = run("--if-stale")
    assert r.returncode == 0, r.stderr
    assert json.loads(claude.read_text())["mcpServers"]["tern"]["args"] == ["--config-dir", str(profile)]
    # A foreign entry is still refused, and --if-stale with --force is a usage error.
    claude.write_text(json.dumps({"mcpServers": {"tern": {"command": "/usr/bin/other"}}}))
    r = run()
    assert r.returncode != 0 and "--force" in r.stderr
    assert run("--if-stale", "--force").returncode != 0
