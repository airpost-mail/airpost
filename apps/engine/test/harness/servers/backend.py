"""
The Tern server (apps/backend) as a scenario fixture: a real `tern-backend serve` on a free
loopback port with its own data dir, users and device tokens made through its CLI, and a
small API client that plays "another device" (docs/backend-plan.md 3.4, 3.5 and 6.1).

The binary is `TERN_BACKEND_BIN`, else the newest `tern-backend` under `CARGO_TARGET_DIR`,
the root `target/` or `apps/backend/target/` (debug or release), so it is found whether or
not apps/backend is in the root workspace.

Identities. The Rust engine reads a Tern server object on its identity line (4.1); the C++
engine reads a Mailspring-shaped identity and finds the server through `IDENTITY_SERVER`.
`engine_identity` builds the one a binary expects. The desktop always gives C++ `null`; the
C++ shape exists so the oracle scenarios can check wire compatibility.
"""
import base64
import json
import os
import re
import shutil
import socket
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

from ..mailsync import REPO_ROOT

BINARY_NAME = "tern-backend"
# C++ throws without `createdAt`; a date after 2018-02-15 keeps it on id schema 1
# (MailUtils::setBaseIDVersion).
CPP_IDENTITY_CREATED_AT = "2018-02-16T00:00:00.000Z"
DEFAULT_USER = "harness@example.test"
DEFAULT_PASSWORD = "harness-password"
PAGE_SIZE = 500  # ms_wire::PAGE_SIZE, the server's cap on `limit`


class BackendError(RuntimeError):
    pass


def backend_binary_candidates() -> list:
    dirs = []
    if os.environ.get("CARGO_TARGET_DIR"):
        dirs.append(Path(os.environ["CARGO_TARGET_DIR"]))
    dirs += [REPO_ROOT / "target", REPO_ROOT / "apps" / "backend" / "target"]
    return [d / profile / BINARY_NAME for d in dirs for profile in ("debug", "release")]


def find_backend_binary() -> Optional[Path]:
    """TERN_BACKEND_BIN, else the most recently built candidate; None when there is none."""
    if os.environ.get("TERN_BACKEND_BIN"):
        path = Path(os.environ["TERN_BACKEND_BIN"]).expanduser()
        return path if path.is_file() else None
    found = [c for c in backend_binary_candidates() if c.is_file() and os.access(c, os.X_OK)]
    return max(found, key=lambda c: c.stat().st_mtime) if found else None


_engine_kinds: dict = {}


def engine_kind(binary: Path) -> str:
    """`rust` or `cpp`. HARNESS_ENGINE_KIND overrides; otherwise a Rust binary is the one
    carrying std's `RUST_BACKTRACE` string."""
    if os.environ.get("HARNESS_ENGINE_KIND"):
        return os.environ["HARNESS_ENGINE_KIND"]
    real = Path(binary).resolve()
    key = (str(real), real.stat().st_mtime)
    if key not in _engine_kinds:
        needle, tail, kind = b"RUST_BACKTRACE", b"", "cpp"
        with open(real, "rb") as f:
            while chunk := f.read(1 << 20):
                if needle in tail + chunk:
                    kind = "rust"
                    break
                tail = chunk[-len(needle):]
        _engine_kinds[key] = kind
    return _engine_kinds[key]


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@dataclass
class Device:
    """One device token of a backend user."""
    name: str
    token: str
    user_id: str
    email: str


class BackendServer:
    """`start` runs `serve`; `stop` ends it and keeps the data dir, so `stop` then `start` is
    an outage the engine has to ride out, on the same port."""

    def __init__(self, work_dir: Path, binary: Optional[Path] = None, log_path: Optional[Path] = None):
        self.binary = Path(binary) if binary else find_backend_binary()
        if self.binary is None:
            raise BackendError(
                f"no {BINARY_NAME} binary: build apps/backend or set TERN_BACKEND_BIN. Tried: "
                + ", ".join(str(c) for c in backend_binary_candidates()))
        self.data_dir = Path(work_dir) / "backend-data"
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.log_path = Path(log_path) if log_path else Path(work_dir) / "backend.log"
        self.port = 0
        self.proc: Optional[subprocess.Popen] = None
        self.server_id = ""
        self.devices: dict = {}   # name -> Device
        self.users: dict = {}     # email -> user id
        self.backups: dict = {}   # name -> (path, device names it holds tokens for)

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    @property
    def running(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def _env(self) -> dict:
        env = {k: v for k, v in os.environ.items() if not k.startswith("TERN_")}
        env.update(TERN_DATA_DIR=str(self.data_dir), TERN_BIND=f"127.0.0.1:{self.port}", TERN_SIGNUP="closed",
                   RUST_LOG=os.environ.get("HARNESS_BACKEND_LOG", "info"))
        return env

    # -- lifecycle -----------------------------------------------------------------------------

    def start(self, timeout: float = 20):
        if self.running:
            return
        if not self.port:
            self.port = free_port()
        log = open(self.log_path, "ab")
        self.proc = subprocess.Popen([str(self.binary), "serve"], env=self._env(), stdin=subprocess.DEVNULL,
                                     stdout=log, stderr=log)
        log.close()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise BackendError(f"{BINARY_NAME} serve exited {self.proc.returncode}: {self.log_tail()}")
            try:
                if self.request("GET", "/ping", timeout=1)[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.05)
        else:
            self.stop()
            raise BackendError(f"{BINARY_NAME} did not answer /ping within {timeout}s: {self.log_tail()}")
        self.server_id = self._read_server_id()

    def stop(self, timeout: float = 10):
        if not self.proc:
            return
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self.proc = None

    def log_tail(self, n: int = 20) -> str:
        try:
            return "\n".join(self.log_path.read_text(errors="replace").splitlines()[-n:])
        except FileNotFoundError:
            return ""

    def _read_server_id(self) -> str:
        status, _, body = self.request("GET", "/tern/v1/server")
        if status != 200:
            raise BackendError(f"GET /tern/v1/server: HTTP {status} {body[:200]!r}")
        return json.loads(body)["serverId"]

    def me(self, device: str) -> dict:
        """`GET /tern/v1/me` as `device`: the `(serverId, userId)` the engine binds its cursor to."""
        status, _, body = self.request("GET", "/tern/v1/me", device=device)
        if status != 200:
            raise BackendError(f"GET /tern/v1/me as {device}: HTTP {status} {body[:200]!r}")
        return json.loads(body)

    # -- CLI -------------------------------------------------------------------------------------

    def cli(self, *args: str, stdin: Optional[str] = None) -> str:
        r = subprocess.run([str(self.binary), *args], env=self._env(), input=stdin, capture_output=True,
                           text=True, timeout=60)
        if r.returncode != 0:
            raise BackendError(f"{BINARY_NAME} {' '.join(args[:2])} failed ({r.returncode}): {r.stderr[-500:]}")
        return r.stdout

    def add_user(self, email: str = DEFAULT_USER, password: str = DEFAULT_PASSWORD) -> str:
        if email in self.users:
            return self.users[email]
        out = self.cli("user", "add", email, "--password-stdin", stdin=password + "\n")
        m = re.search(r"^created user (\S+) ", out, re.M)
        if not m:
            raise BackendError(f"unexpected `user add` output: {out!r}")
        self.users[email] = m.group(1)
        return self.users[email]

    def device(self, name: str, email: str = DEFAULT_USER) -> Device:
        """The device token `name` of `email`, minted on first use. One token per running engine:
        a second stream from the same token replaces the first (3.5)."""
        if name not in self.devices:
            user_id = self.add_user(email)
            token = self.cli("token", "create", email, "--name", name).strip().splitlines()[-1]
            self.devices[name] = Device(name, token, user_id, email)
        return self.devices[name]

    def revoke(self, name: str) -> int:
        """Revokes every live token named `name`; returns how many."""
        dev = self.devices[name]
        out = self.cli("token", "list", "--user", dev.email)
        ids = [int(cols[0]) for cols in (line.split("\t") for line in out.splitlines()[1:])
               if len(cols) >= 6 and cols[2] == name and cols[5] == "-"]
        for token_id in ids:
            self.cli("token", "revoke", str(token_id))
        return len(ids)

    def backup(self, name: str) -> Path:
        """`tern-backend backup`: a copy with a new server id for when it is restored."""
        path = self.data_dir.parent / f"backend-backup-{name}.db"
        self.cli("backup", str(path))
        self.backups[name] = (path, set(self.devices))
        return path

    def restore(self, name: str):
        """Restores backup `name` as docs/self-hosting.md says: server stopped, the copy as
        `tern.db`, the old `-wal` and `-shm` removed. Tokens minted after the backup are gone
        from the server, so those devices are forgotten and minted again on next use."""
        if self.running:
            raise BackendError("restore needs the server stopped (backend.stop)")
        path, devices = self.backups[name]
        db = self.data_dir / "tern.db"
        for side in ("-wal", "-shm"):
            db.with_name(db.name + side).unlink(missing_ok=True)
        shutil.copyfile(path, db)
        self.devices = {k: v for k, v in self.devices.items() if k in devices}
        self.users = {d.email: d.user_id for d in self.devices.values()}

    # -- HTTP ------------------------------------------------------------------------------------

    def request(self, method: str, path: str, body=None, device: Optional[str] = None, timeout: float = 10,
                headers: Optional[dict] = None) -> tuple:
        """(status, headers, body bytes). `device` authenticates as that device token, with
        Basic `base64("<token>:")` as both engines send it. `path` may also be a full URL."""
        url = path if path.startswith("http") else self.url + path
        data = None
        hdrs = dict(headers or {})
        if body is not None:
            data = json.dumps(body).encode()
            hdrs.setdefault("Content-Type", "application/json")
        if device:
            cred = base64.b64encode(f"{self.devices[device].token}:".encode()).decode()
            hdrs["Authorization"] = f"Basic {cred}"
        req = urllib.request.Request(url, data=data, method=method, headers=hdrs)
        opener = urllib.request.build_opener(_NoRedirect, urllib.request.ProxyHandler({}))
        try:
            with opener.open(req, timeout=timeout) as resp:
                return resp.status, dict(resp.headers), resp.read()
        except urllib.error.HTTPError as e:
            return e.code, dict(e.headers), e.read()

    def push(self, device: str, aid: str, object_type: str, object_id: str, plugin_id: str, value,
             version: Optional[int] = None, header_message_id: Optional[str] = None) -> int:
        """`POST /metadata/<aid>/<objectId>/<pluginId>` as another device; returns the new `v`."""
        body = {"objectType": object_type, "version": version, "value": value}
        if header_message_id:
            body["headerMessageId"] = header_message_id
        path = "/metadata/{}/{}/{}".format(*(urllib.parse.quote(p, safe="") for p in (aid, object_id, plugin_id)))
        status, _, out = self.request("POST", path, body, device=device)
        if status != 200:
            raise BackendError(f"push {object_type} {object_id} {plugin_id}: HTTP {status} {out[:200]!r}")
        return json.loads(out)["v"]

    def rows(self, device: str, aid: str) -> list:
        """Every metadata row of `aid` visible to the device's user, tombstones included,
        through the pages endpoint (3.5)."""
        rows, offset = [], 0
        while True:
            status, _, out = self.request("GET", f"/metadata/{urllib.parse.quote(aid, safe='')}?limit={PAGE_SIZE}&offset={offset}",
                                          device=device)
            if status != 200:
                raise BackendError(f"metadata page of {aid}: HTTP {status} {out[:200]!r}")
            page = json.loads(out)
            rows += page
            if len(page) < PAGE_SIZE:
                return rows
            offset += len(page)

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *exc):
        self.stop()


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """A redirect is an answer to check (tracking links, 3.6), not something to follow."""

    def redirect_request(self, *args, **kwargs):
        return None


def tern_server_identity(backend: BackendServer, device: Device) -> dict:
    """The identity line the Rust engine reads (docs/backend-plan.md 4.1)."""
    return {"type": "tern-server", "version": 1, "url": backend.url, "token": device.token,
            "userId": device.user_id, "serverId": backend.server_id}


def cpp_identity(device: Device) -> dict:
    """The Mailspring-shaped identity C++ needs; the server comes from `IDENTITY_SERVER`."""
    return {"id": device.user_id, "emailAddress": device.email, "firstName": "", "lastName": "",
            "token": device.token, "createdAt": CPP_IDENTITY_CREATED_AT}


def engine_identity(backend: BackendServer, device: Device, binary: Path) -> tuple:
    """(identity JSON, extra environment) for the engine `binary`."""
    if engine_kind(binary) == "cpp":
        return cpp_identity(device), {"IDENTITY_SERVER": backend.url}
    return tern_server_identity(backend, device), {}
