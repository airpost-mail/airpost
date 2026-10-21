"""
End-to-end test of tern-gateway (apps/gateway): a `serve` engine on
the fake IMAP server, `tern-gateway serve` on a free loopback port over the same profile, and
a browser played by http.client and the small WebSocket client below (stdlib only): pairing
with `tern-gateway pair`, the cookie and Origin checks, the relayed `hello`, reads, a task and
its Task delta, refused calls and task classes, the file routes, revoke (4401), and an engine
kill (4502) and restart.

    cargo build --release -p tern-engine -p tern-gateway
    TERN_ENGINE_BIN=target/release/tern-engine python3 -m pytest apps/engine/test/test_gateway.py -n 0

The gateway binary comes from TERN_GATEWAY_BIN, else the newer of target/release/tern-gateway
and target/debug/tern-gateway. Without one the test skips, unless CI or REQUIRE_TERN is set.
The engine always runs on `serve`, whatever TERN_ENGINE_TRANSPORT says: the gateway relays only
to a serve engine.
"""
import base64
import contextlib
import hashlib
import http.client
import json
import os
import queue
import signal
import socket
import struct
import subprocess
import threading
import time
from pathlib import Path
from typing import Optional

import pytest

from harness.engine import REPO_ROOT, EngineProcess
from harness.scenario import ScenarioRun, ServerSpec

SUBJECT = "Quarterly zebra report"
ATTACHMENT = b"binary payload"

SCENARIO = {
    "name": "gateway-end-to-end",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {"INBOX": {"messages": 3}},
    "setup": [
        {"server.append": {"mailbox": "INBOX", "messages": {"count": 1, "subject": SUBJECT,
                                                            "attachment": ["report.bin", ATTACHMENT.decode()]}}},
    ],
}

WAIT = 30.0
SECRET_MARKER = "pytest-secret-7f3c9a"   # sent as a secrets.set value; must never reach the log


def gateway_binary() -> Path:
    if os.environ.get("TERN_GATEWAY_BIN"):
        return Path(os.environ["TERN_GATEWAY_BIN"])
    candidates = [REPO_ROOT / "target" / profile / "tern-gateway" for profile in ("release", "debug")]
    built = [c for c in candidates if c.is_file() and os.access(c, os.X_OK)]
    if not built:
        msg = ("no tern-gateway binary: cargo build -p tern-gateway, or set TERN_GATEWAY_BIN. Tried: "
               + ", ".join(map(str, candidates)))
        if os.environ.get("CI") or os.environ.get("REQUIRE_TERN"):
            pytest.fail(msg)
        pytest.skip(msg)
    return max(built, key=lambda c: c.stat().st_mtime)


# -- a minimal RFC 6455 client ----------------------------------------------------------------

class WsClosed(Exception):
    pass


class Ws:
    """A WebSocket client on a plain socket: text frames in and out, pongs answered on the
    reader thread (the gateway closes a tab that stops answering), and the close
    code the server sent."""

    GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

    def __init__(self, port: int, headers: dict, path: str = "/gateway/v1/ws"):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=WAIT)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [f"GET {path} HTTP/1.1", f"Host: 127.0.0.1:{port}", "Upgrade: websocket",
                 "Connection: Upgrade", f"Sec-WebSocket-Key: {key}", "Sec-WebSocket-Version: 13"]
        lines += [f"{k}: {v}" for k, v in headers.items()]
        self.sock.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise AssertionError(f"connection closed during the handshake: {head!r}")
            head += chunk
        head, self._buf = head.split(b"\r\n\r\n", 1)
        status, *header_lines = head.decode("latin-1").split("\r\n")
        self.status = int(status.split(" ")[1])
        if self.status != 101:
            raise AssertionError(f"upgrade refused: {head!r}")
        got = {k.lower(): v.strip() for k, v in (l.split(":", 1) for l in header_lines)}
        accept = base64.b64encode(hashlib.sha1((key + self.GUID).encode()).digest()).decode()
        assert got.get("sec-websocket-accept") == accept, got
        self.sock.settimeout(None)
        self.messages: queue.Queue = queue.Queue()
        self.close_code: Optional[int] = None
        self.close_reason = ""
        self.closed = threading.Event()
        self._send_lock = threading.Lock()
        self._sent_close = False
        self.next_id = 100
        threading.Thread(target=self._read, daemon=True).start()

    def _recv_exact(self, n: int) -> bytes:
        while len(self._buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise WsClosed("eof")
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _frame(self):
        b0, b1 = self._recv_exact(2)
        assert b1 & 0x80 == 0, "the server masked a frame"
        n = b1 & 0x7F
        if n == 126:
            n = struct.unpack(">H", self._recv_exact(2))[0]
        elif n == 127:
            n = struct.unpack(">Q", self._recv_exact(8))[0]
        return bool(b0 & 0x80), b0 & 0x0F, self._recv_exact(n)

    def _read(self):
        parts: list = []
        try:
            while True:
                fin, op, payload = self._frame()
                if op == 0x9:
                    self._send(0xA, payload)
                elif op == 0xA:
                    pass
                elif op == 0x8:
                    if len(payload) >= 2:
                        self.close_code = struct.unpack(">H", payload[:2])[0]
                        self.close_reason = payload[2:].decode("utf-8", "replace")
                    else:
                        self.close_code = 1005
                    with contextlib.suppress(OSError):
                        self._send(0x8, payload[:2])
                    break
                else:
                    parts.append(payload)
                    if fin:
                        text = b"".join(parts).decode()
                        parts = []
                        self.messages.put(json.loads(text))
        except (WsClosed, OSError):
            if self.close_code is None:
                self.close_code = 1006
        finally:
            self.closed.set()
            with contextlib.suppress(OSError):
                self.sock.close()

    def _send(self, op: int, payload: bytes):
        mask = os.urandom(4)
        n = len(payload)
        head = bytes([0x80 | op])
        if n < 126:
            head += bytes([0x80 | n])
        elif n < 1 << 16:
            head += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            head += bytes([0x80 | 127]) + struct.pack(">Q", n)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self._send_lock:
            if self._sent_close:
                return
            if op == 0x8:
                self._sent_close = True
            self.sock.sendall(head + mask + masked)

    def send_text(self, text: str):
        self._send(0x1, text.encode())

    def notify(self, method: str, params=None):
        self.send_text(json.dumps({"jsonrpc": "2.0", "method": method, "params": params}))

    def call(self, method: str, params=None, timeout: float = WAIT) -> dict:
        """The whole response (result or error); notifications before it stay queued for
        next()/wait_for()."""
        self.next_id += 1
        rid = self.next_id
        self.send_text(json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params}))
        held = []
        try:
            return self.wait_for(lambda m: m.get("id") == rid and "method" not in m, f"the answer to {method}",
                                 timeout, held)
        finally:
            for m in held:
                self.messages.put(m)

    def result(self, method: str, params=None) -> dict:
        res = self.call(method, params)
        assert "error" not in res, f"{method}: {res['error']}"
        return res["result"]

    def wait_for(self, pred, what: str, timeout: float = WAIT, held: Optional[list] = None) -> dict:
        """The first message matching `pred`; others go to `held` when given, else are dropped."""
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise AssertionError(f"timed out waiting for {what} (close {self.close_code} {self.close_reason!r})")
            try:
                msg = self.messages.get(timeout=min(left, 0.25))
            except queue.Empty:
                if self.closed.is_set() and self.messages.empty():
                    raise AssertionError(f"closed with {self.close_code} {self.close_reason!r} "
                                         f"while waiting for {what}")
                continue
            if pred(msg):
                return msg
            if held is not None:
                held.append(msg)

    def hello(self) -> dict:
        # What WsEngineClient sends (apps/web/src/engine/ws.ts).
        return self.result("hello", {"protocol": 1, "client": "tern-web/0.1.0"})

    def wait_closed(self, timeout: float = WAIT) -> int:
        assert self.closed.wait(timeout), "the gateway did not close the WebSocket"
        return self.close_code

    def close(self):
        with contextlib.suppress(OSError):
            self._send(0x8, struct.pack(">H", 1000))
        self.closed.wait(5)
        with contextlib.suppress(OSError):
            self.sock.close()


# -- the gateway --------------------------------------------------------------------------------

class Gateway:
    def __init__(self, config_dir: Path, log_path: Path):
        self.bin = str(gateway_binary())
        self.config_dir = config_dir
        self.log_path = log_path
        self.env = {k: v for k, v in os.environ.items()
                    if not (k.startswith("TERN_GATEWAY_") or k in ("TERN_CONFIG_DIR", "TERN_TRUSTED_PROXIES"))}
        self.env["RUST_LOG"] = "info"
        self.proc: Optional[subprocess.Popen] = None
        self.port = 0
        self.codes: list = []   # every pairing code minted, for the log check
        self.tokens: list = []  # every device token issued, for the log check

    @property
    def origin(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def start(self):
        # Port 0: the gateway names the port it got in gateway.json, as `pair` reads it.
        self.proc = subprocess.Popen([self.bin, "--config-dir", str(self.config_dir), "serve", "--bind", "127.0.0.1:0"],
                                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                     stderr=open(self.log_path, "ab"), env=self.env)
        running = self.config_dir / "gateway" / "gateway.json"
        deadline = time.monotonic() + WAIT
        while time.monotonic() < deadline and self.proc.poll() is None:
            with contextlib.suppress(OSError, ValueError):
                info = json.loads(running.read_text())
                if info["pid"] == self.proc.pid:
                    self.port = int(info["publicUrl"].rsplit(":", 1)[1])
                    if self.http("GET", "/healthz")[0] == 200:
                        return
            time.sleep(0.1)
        self.stop()
        raise AssertionError(f"tern-gateway didn't start:\n{self.log()[-3000:]}")

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()

    def log(self) -> str:
        return self.log_path.read_text(errors="replace") if self.log_path.exists() else ""

    def cli(self, *args) -> str:
        r = subprocess.run([self.bin, "--config-dir", str(self.config_dir), *args],
                           capture_output=True, text=True, timeout=WAIT, env=self.env)
        assert r.returncode == 0, f"tern-gateway {' '.join(args)}: {r.returncode}\n{r.stdout}\n{r.stderr}"
        return r.stdout

    def serve_refused(self, *args) -> str:
        """Runs a second `serve` on this profile that must exit with an error; its stderr."""
        r = subprocess.run([self.bin, "--config-dir", str(self.config_dir), "serve", *args],
                           stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=WAIT, env=self.env)
        assert r.returncode != 0, f"a second serve {args} started:\n{r.stderr}"
        return r.stderr

    def http(self, method: str, path: str, body: bytes = b"", headers: Optional[dict] = None):
        """(status, lowercased headers, body)."""
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=WAIT)
        try:
            c.request(method, path, body=body, headers=headers or {})
            r = c.getresponse()
            return r.status, {k.lower(): v for k, v in r.getheaders()}, r.read()
        finally:
            c.close()

    def pair(self, *flags, reuse: bool = False) -> tuple:
        """Mints a code with `tern-gateway pair`, redeems it as a browser would, and returns
        (Cookie header value, the pair answer's device). `reuse` also checks the code is spent;
        each attempt counts against the 5 a minute per client IP."""
        minted = json.loads(self.cli("pair", "--json", *flags))
        code = minted["code"]
        assert minted["url"] == f"{self.origin}/pair#{code}", minted
        body = json.dumps({"code": code, "deviceName": "pytest browser"}).encode()
        status, headers, out = self.http("POST", "/gateway/v1/pair", body,
                                         {"Origin": self.origin, "Content-Type": "application/json"})
        assert status == 200, (status, out)
        set_cookie = headers["set-cookie"]
        # Loopback over plain http: no Secure, so no __Host- prefix.
        assert set_cookie.startswith("tern_device="), set_cookie
        flags_ = {f.strip().lower() for f in set_cookie.split(";")[1:]}
        assert {"httponly", "samesite=strict", "path=/"} <= flags_ and "secure" not in flags_, set_cookie
        assert code not in out.decode()
        if reuse:
            again = self.http("POST", "/gateway/v1/pair", body, {"Origin": self.origin})
            assert again[0] == 401, again
        self.codes.append(code)
        self.tokens.append(set_cookie.split(";")[0].split("=", 1)[1])
        return set_cookie.split(";")[0], json.loads(out)["device"]

    def ws(self, cookie: Optional[str], origin: Optional[str] = "self") -> Ws:
        headers = {}
        if cookie:
            headers["Cookie"] = cookie
        if origin:
            headers["Origin"] = self.origin if origin == "self" else origin
        return Ws(self.port, headers)

    def session(self, cookie: str) -> tuple:
        status, _, body = self.http("GET", "/gateway/v1/session", headers={"Cookie": cookie})
        return status, (json.loads(body) if status == 200 else body)


@pytest.fixture(scope="module")
def setup(request):
    gateway_binary()   # skip before starting anything
    binary = request.config.getoption("--engine")
    run = ScenarioRun(dict(SCENARIO), ServerSpec.parse({"fake": "dovecot"}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = "serve"
    gw = None
    try:
        run.ms.start()
        run.ms.wait_quiescent(timeout=180)
        gw = Gateway(Path(run.ms.config_dir), run.work / "gateway.log")
        gw.start()
        yield run, gw
    finally:
        if gw:
            gw.stop()
        run.teardown()


def restart_engine(run: ScenarioRun):
    """A new serve engine on the same profile, as the desktop starts one after a crash."""
    old = run.ms
    old.stop()
    run.ms = EngineProcess(old.account, run.work, binary=old.binary, verbose=True,
                             env=run._engine_env(), transport="serve")
    run.ms.start()
    run.ms.wait_quiescent(timeout=180)


def test_gateway_end_to_end(setup):
    run, gw = setup
    account_id = run.ms.account["id"]

    # -- auth at the door --------------------------------------------------------------------
    assert gw.http("GET", "/gateway/v1/session")[0] == 401
    assert gw.ws(None).wait_closed() == 4401
    assert gw.ws("tern_device=not-a-token").wait_closed() == 4401
    cookie, device = gw.pair(reuse=True)

    # -- a second serve on the profile: config refused before any bind, then the control socket
    # checked before the TCP port --------------------------------------------------------------
    assert "public_url" in gw.serve_refused("--bind", "0.0.0.0:0")
    assert "another gateway is running" in gw.serve_refused("--bind", f"127.0.0.1:{gw.port}")
    assert gw.http("GET", "/healthz")[0] == 200
    assert device["scopes"] == [] and device["name"] == "pytest browser"
    assert gw.ws(cookie, origin=None).wait_closed() == 4403
    assert gw.ws(cookie, origin="http://evil.example").wait_closed() == 4403
    status, session = gw.session(cookie)
    assert status == 200 and session["engine"]["state"] == "up", session
    assert session["device"]["id"] == device["id"]

    # -- the relayed session -----------------------------------------------------------------
    ws = gw.ws(cookie)
    hello = ws.hello()
    assert "relayed" in hello["features"] and "files" in hello["features"], hello
    assert hello["gateway"]["scopes"] == []
    assert [a["accountId"] for a in hello["accounts"]] == [account_id]
    assert ws.result("subscribe", {"accounts": None, "classes": ["Task", "Thread"]}) == {}

    listed = ws.call("account.list")
    password = run.ms.account["settings"]["imap_password"]
    assert json.dumps(password) not in json.dumps(listed), "account.list leaked the password"
    [acct] = listed["result"]["accounts"]
    assert acct["accountId"] == account_id
    settings = acct["account"]["settings"]
    assert settings["imap_host"] == run.ms.account["settings"]["imap_host"]
    assert not {"imap_password", "smtp_password", "refresh_token"} & set(settings), settings

    folders = ws.result("folders.list", {"accountId": account_id})["folders"]
    assert any(f["role"] == "inbox" for f in folders), folders
    threads = ws.result("threads.list", {"accountId": account_id, "role": "inbox", "limit": 10})["threads"]
    assert len(threads) == 4, [t.get("subject") for t in threads]
    zebra = next(t for t in threads if t["subject"] == SUBJECT)
    queried = ws.result("query", {"class": "Thread", "accounts": [account_id],
                                  "search": {"text": "zebra"}, "limit": 5})
    assert [t["id"] for t in queried["rows"]] == [zebra["id"]], queried
    counted = ws.result("query", {"class": "Thread", "accounts": [account_id], "count": True})
    assert counted["count"] == 4, counted
    [message] = ws.result("threads.get", {"accountId": account_id, "id": zebra["id"]})["messages"]
    got = ws.result("messages.get", {"accountId": account_id, "id": message["id"]})
    [att] = got["files"]
    assert att["filename"] == "report.bin"

    # -- a task and its Task delta ------------------------------------------------------------
    task = {"id": "gw-star-1", "__cls": "ChangeStarredTask", "aid": account_id, "status": "local", "v": 0,
            "threadIds": [zebra["id"]], "messageIds": [], "starred": True}
    queued = ws.call("task.queue", {"accountId": account_id, "task": task})
    assert "error" not in queued, queued

    def task_done(m):
        p = m.get("params") or {}
        return (m.get("method") == "delta" and p.get("modelClass") == "Task"
                and any(j.get("id") == "gw-star-1" and j.get("status") == "complete" for j in p.get("modelJSONs", [])))
    ws.wait_for(task_done, "the starred task to complete")
    starred = ws.result("threads.list", {"accountId": account_id, "role": "inbox", "limit": 10})["threads"]
    assert next(t for t in starred if t["id"] == zebra["id"])["starred"]   # a count, as Thread JSON has it
    # ...and on the server.
    deadline = time.monotonic() + WAIT
    while not any("\\Flagged" in m["flags"] for m in run.server.truth()["INBOX"].values()):
        assert time.monotonic() < deadline, "the star never reached the IMAP server"
        time.sleep(0.2)

    # -- refusals: never reach the engine ------------------------------------------------------
    refused = {
        "secrets.set": {"name": "integration:pytest", "value": SECRET_MARKER},
        "files.get": {"accountId": account_id, "fileId": att["id"]},
        "files.put": {"accountId": account_id, "name": "x", "path": "/etc/passwd"},
        "engine.shutdown": {},
        "account.add": {"account": run.ms.account, "identity": None},
        "test.crash": {"accountId": account_id},
        "no.such.method": {},
    }
    for method, params in refused.items():
        assert ws.call(method, params)["error"]["code"] == -32008, method
    for cls, extra in [("GetMessageRFC2822Task", {"messageId": message["id"], "filepath": "/tmp/gw-export.eml"}),
                       ("NoSuchTask", {})]:
        t = {"id": f"gw-{cls}", "__cls": cls, "aid": account_id, **extra}
        res = ws.call("task.queue", {"accountId": account_id, "task": t})
        assert res["error"]["code"] == -32008, (cls, res)
    assert not Path("/tmp/gw-export.eml").exists()
    # A refused call leaves the session usable.
    assert ws.result("engine.status", {}) is not None

    # -- files ------------------------------------------------------------------------------------
    url = f"/gateway/v1/files/{account_id}/{att['id']}"
    assert gw.http("GET", url)[0] == 401
    status, headers, body = gw.http("GET", url, headers={"Cookie": cookie})
    assert status == 200 and body == ATTACHMENT, (status, body[:200])
    assert headers["content-type"] == "application/octet-stream"
    assert headers["content-security-policy"] == "sandbox"
    assert headers["content-disposition"].startswith('attachment; filename="report.bin"')
    status, headers, body = gw.http("GET", url, headers={"Cookie": cookie, "Range": "bytes=0-5"})
    assert status == 206 and body == ATTACHMENT[:6]

    upload = b"uploaded from the browser\n" * 100
    path = f"/gateway/v1/files/{account_id}?name=notes%20one.txt"
    assert gw.http("POST", path, upload, {"Cookie": cookie, "Content-Type": "text/plain"})[0] == 403
    status, _, out = gw.http("POST", path, upload,
                             {"Cookie": cookie, "Origin": gw.origin, "Content-Type": "text/plain"})
    assert status == 200, (status, out)
    put = json.loads(out)
    assert put["file"]["filename"] == "notes one.txt" and put["file"]["size"] == len(upload)
    status, _, body = gw.http("GET", f"/gateway/v1/files/{account_id}/{put['fileId']}", headers={"Cookie": cookie})
    assert status == 200 and body == upload
    incoming = Path(run.ms.config_dir) / "gateway" / "incoming"
    assert not any(incoming.iterdir()), "the upload's temp file stayed"

    # -- an admin device: local-only calls pass, parent-only and paths still don't ----------------
    admin_cookie, admin = gw.pair("--admin", "--name", "admin browser")
    assert admin["scopes"] == ["admin"] and admin["name"] == "admin browser"
    aws = gw.ws(admin_cookie)
    assert aws.hello()["gateway"]["scopes"] == ["admin"]
    files_get = aws.call("files.get", {"accountId": account_id, "fileId": att["id"]})
    assert "error" not in files_get, files_get
    # account.* and secrets.* are local-only (stage 12): an admin device may call them.
    has = aws.call("secrets.has", {"names": ["integration:deepl"]})
    assert has.get("result") == {"integration:deepl": False}, has
    for method, params in [("engine.shutdown", {}), ("credentials.import", {"accounts": [], "secrets": {}}),
                           ("files.put", {"accountId": account_id, "name": "x", "path": "/etc/passwd"})]:
        assert aws.call(method, params)["error"]["code"] == -32008, method
    aws.close()

    # -- revoke closes with 4401 ----------------------------------------------------------------
    devices = json.loads(gw.cli("devices", "list", "--json"))["devices"]
    assert {d["id"] for d in devices} == {device["id"], admin["id"]}
    gw.cli("devices", "revoke", device["id"])
    assert ws.wait_closed() == 4401
    assert gw.session(cookie)[0] == 401
    assert gw.ws(cookie).wait_closed() == 4401
    assert gw.http("GET", url, headers={"Cookie": cookie})[0] == 401

    # -- engine kill closes with 4502; after a restart a new socket works -------------------------
    cookie, device = gw.pair("--name", "second browser")
    ws = gw.ws(cookie)
    ws.hello()
    os.kill(run.ms.proc.pid, signal.SIGKILL)
    run.ms.proc.wait(10)
    assert ws.wait_closed() == 4502
    status, session = gw.session(cookie)
    assert status == 200 and session["engine"]["state"] == "down", session
    assert gw.ws(cookie).wait_closed() == 4502

    restart_engine(run)
    deadline = time.monotonic() + WAIT
    while gw.session(cookie)[1]["engine"]["state"] != "up":
        assert time.monotonic() < deadline, "the gateway never saw the restarted engine"
        time.sleep(0.2)
    ws = gw.ws(cookie)
    hello = ws.hello()
    assert [a["accountId"] for a in hello["accounts"]] == [account_id]
    ws.result("subscribe", {"accounts": None, "classes": ["Task"]})
    again = ws.result("threads.list", {"accountId": account_id, "role": "inbox", "limit": 10})["threads"]
    assert {t["id"] for t in again} == {t["id"] for t in threads}
    ws.close()
    assert ws.close_code == 1000

    # -- the log: no secrets, codes, tokens or params ---------------------------------------------
    log = gw.log()
    assert "tern-gateway serving" in log
    assert len(gw.tokens) == 3 and len(gw.codes) == 3
    for secret in [*gw.tokens, *gw.codes, SECRET_MARKER]:
        assert secret not in log, f"the gateway log holds a secret: {secret[:4]}..."
    # The upload name reaches the gateway URL-encoded, so check that form too.
    assert "/etc/passwd" not in log and "notes one" not in log and "notes%20one" not in log
