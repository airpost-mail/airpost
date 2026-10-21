"""
Drives a real mailsync process the way the Mailspring client does: account JSON on the
command line, newline-delimited JSON commands on stdin, the delta stream on stdout, and
the engine log in CONFIG_DIR_PATH. Nothing here touches the engine's source; every signal
the harness relies on is one the client already depends on or one that is logged today.

MAILSYNC_TRANSPORT=socket runs the Rust engine with `--socket` instead:
commands become JSON-RPC requests on a Unix
socket and deltas arrive as `delta` notifications, recorded exactly as the stdout ones are.
stdin stays open as the lifeline and still carries test-crash and test-segfault.

MAILSYNC_TRANSPORT=serve runs one `--mode serve` engine (`ServeEngine`)
and adds each account to it with `account.add` on stdin; commands and deltas go over the
socket as with `socket`, and each account's deltas are picked out by their `accountId`.
A MailsyncProcess is then one account's view of that engine: `running` also needs its
`account.state` to be starting or running, and `held()` stops the whole engine.

Quiescence is inferred, not signalled. The engine has no "done" event, so a process is
considered quiescent when all of these hold at once:

  - every Folder's localStatus has busy == false and syncedMinUID <= 1 (initial sync done)
  - the background worker's last log line is "Sync loop complete." and it is older than
    the settle window (syncNow() logs that line once per iteration and loops immediately
    while there is more to do, so a stale one means the worker went to sleep)
  - the foreground worker's last IMAP command is IDLE (verbose log) or its last engine
    line is "Idling on folder ..." (non-verbose)
  - no delta has arrived within the settle window
"""
import contextlib
import itertools
import json
import os
import re
import shutil
import signal
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Iterable, Optional

# apps/engine; the monorepo root is two levels further up.
ENGINE_DIR = Path(__file__).resolve().parents[2]
REPO_ROOT = ENGINE_DIR.parents[1]


def describe_exit(code: Optional[int]) -> str:
    """A negative returncode is a signal, not an exit status: a migrate or sync the OS killed
    (e.g. SIGKILL under memory pressure) must not read like an engine error."""
    if code is None or code >= 0:
        return f"exit {code}"
    try:
        name = signal.Signals(-code).name
    except ValueError:
        name = "unknown signal"
    return f"killed by signal {-code} ({name})"

LOG_LINE = re.compile(
    r"^(?P<pid>\d+) \[(?P<ts>[^\]]+)\] \[(?P<thread>[^\]]+)\] \[(?P<level>[^\]]+)\] (?P<msg>.*)$"
)
IMAP_LINE = re.compile(r"^(?P<dir>sent|recv|sent-private|error-parse|error-received) (?P<line>.*)$")
# The command line of an IDLE, whatever the tag looks like.
IDLE_COMMAND = re.compile(r"^\S+ IDLE$")


_minute_epochs: dict = {}


def _log_epoch(ts: str) -> float:
    """`2026-10-15 15:37:28.943` (local time, both engines' log format) as epoch seconds.
    The minute prefix is converted once; a log holds tens of thousands of lines."""
    try:
        minute, seconds = ts[:16], float(ts[17:])
        base = _minute_epochs.get(minute)
        if base is None:
            base = _minute_epochs[minute] = time.mktime(time.strptime(minute, "%Y-%m-%d %H:%M"))
        return base + seconds
    except ValueError:
        return 0.0


_task_seq = itertools.count(1)

TRANSPORTS = ("stdio", "socket", "serve")
# The protocol version the socket transport speaks (tern-ipc PROTOCOL_VERSION).
SOCKET_PROTOCOL = 1
# The engine's TASK_REJECTED: the task's local phase failed. The Task delta carries the same
# error, as on stdio, so it is an outcome rather than a transport problem.
RPC_TASK_REJECTED = -32002
RPC_ACCOUNT_EXISTS = -32006
# The Account JSON settings that are secrets (KeyManager, the engine's registry).
SECRET_SETTINGS = ("imap_password", "smtp_password", "refresh_token")
# The stdin commands the socket transport sends as requests; anything else goes over stdin.
STDIN_TO_METHOD = {
    "wake-workers": "sync.wake",
    "queue-task": "task.queue",
    "cancel-task": "task.cancel",
    "need-bodies": "bodies.fetch",
    "sync-calendar": "calendar.sync",
}
# The one delta that stays on stdout in socket mode: the account JSON after
# an OAuth refresh, which must not reach socket subscribers.
STDOUT_ONLY_DELTA = "ProcessAccountSecretsUpdated"
# `account.state` values that mean the account's threads are gone (serve).
ACCOUNT_DOWN = ("stopped", "removed")
# How long serve's `account.add` / `account.remove` may take: the engine joins a stopping
# account's threads for up to 5 s (ACCOUNT_STOP_TIMEOUT_MS) and waits 10 s for a start.
SERVE_ACCOUNT_TIMEOUT = 30.0


def default_transport() -> str:
    t = os.environ.get("MAILSYNC_TRANSPORT") or "stdio"
    if t not in TRANSPORTS:
        raise MailsyncError(f"MAILSYNC_TRANSPORT={t!r}: expected one of {', '.join(TRANSPORTS)}")
    return t


class MailsyncError(RuntimeError):
    pass


# The engine's database in its config dir: the Rust engine's is tern.db, the C++ engine's
# edgehill.db. The desktop's Import from Mailspring copies a C++ database to tern.db, where the
# Rust engine's migrate moves it aside.
RUST_DATABASE = "tern.db"
CPP_DATABASE = "edgehill.db"


def is_cpp_engine(binary: Optional[Path]) -> bool:
    """Whether `binary` is the C++ engine: MAILSYNC_CPP_BIN, or a build in legacy/mailsync-cpp,
    Xcode's MailSync-* DerivedData or a Mailspring.app bundle."""
    if binary is None:
        return False
    path = Path(binary).expanduser().resolve()
    cpp = os.environ.get("MAILSYNC_CPP_BIN")
    if cpp and Path(cpp).expanduser().resolve() == path:
        return True
    parts = path.parts
    return ("mailsync-cpp" in parts or any(p.endswith(".app") and p.startswith("Mailspring") for p in parts)
            or any(p.startswith("MailSync-") for p in parts))


def find_binary() -> Path:
    """MAILSYNC_BIN, else the Rust release build, else the C++ engine's Linux cmake output,
    else Xcode's."""
    candidates = []
    if os.environ.get("MAILSYNC_BIN"):
        candidates.append(Path(os.environ["MAILSYNC_BIN"]))
    candidates.append(REPO_ROOT / "target" / "release" / "mailsync")
    candidates.append(REPO_ROOT / "legacy" / "mailsync-cpp" / "mailsync")
    derived = Path.home() / "Library/Developer/Xcode/DerivedData"
    if derived.exists():
        candidates += sorted(derived.glob("MailSync-*/Build/Products/*/mailsync"), reverse=True)
    for c in candidates:
        if c.is_file() and os.access(c, os.X_OK):
            return c.resolve()
    raise MailsyncError(
        "No mailsync binary found. Build it (see BUILDING.md) or set MAILSYNC_BIN. Tried: "
        + ", ".join(str(c) for c in candidates)
    )


def account_json(
    imap_host: str,
    imap_port: int,
    *,
    account_id: str = "c0ffee-harness",
    email: str = "test@example.test",
    username: str = "test",
    password: str = "pass",
    provider: str = "imap",
    smtp_host: str = "127.0.0.1",
    smtp_port: int = 10025,
    imap_security: str = "none",
    smtp_security: str = "none",
    container_folder: str = "",
    extra_settings: Optional[dict] = None,
    oauth: Optional[dict] = None,
) -> dict:
    """The minimum Account the engine's Account::valid() accepts, matching what the client's
    onboarding produces for a generic IMAP account. Account ids starting with c-f get a
    startDelay() of 0 so the background worker starts immediately.

    `oauth: {refresh_token, refresh_client_id}` makes it an OAuth account as onboarding
    stores one (Gmail, Office 365): no passwords, so IMAP, SMTP and DAV authenticate with an
    access token the engine fetches with the refresh token (XOAuth2TokenManager)."""
    settings = {
        "imap_host": imap_host,
        "imap_port": imap_port,
        "imap_username": username,
        "imap_password": password,
        "imap_security": imap_security,
        "imap_allow_insecure_ssl": imap_security != "none",
        "smtp_host": smtp_host,
        "smtp_port": smtp_port,
        "smtp_username": username,
        "smtp_password": password,
        "smtp_security": smtp_security,
        "smtp_allow_insecure_ssl": smtp_security != "none",
        "container_folder": container_folder,
        "create_helper_folders": False,
    }
    if oauth:
        settings.pop("imap_password")
        settings.pop("smtp_password")
        settings["refresh_token"] = oauth["refresh_token"]
        settings["refresh_client_id"] = oauth["refresh_client_id"]
    settings.update(extra_settings or {})
    # Onboarding records the "send password unencrypted" confirmation as the host it was
    # given for; the engine refuses `none` for a non-loopback host (an /etc/hosts alias)
    # without it.
    for proto in ("imap", "smtp"):
        if settings.get(f"{proto}_security") == "none":
            settings.setdefault(f"{proto}_allow_cleartext", settings.get(f"{proto}_host"))
    return {
        "id": account_id,
        "__cls": "Account",
        "provider": provider,
        "emailAddress": email,
        "name": "Harness Test",
        "settings": settings,
        "aliases": [],
        "label": email,
    }


@dataclass
class Delta:
    t: float
    type: str          # persist | unpersist
    model_class: str   # Message, Thread, Folder, Label, Task, ...
    models: list
    raw: dict
    wall: float = 0.0  # time.time() on receipt; comparable across engine processes and with expirations

    def ids(self) -> list:
        return [m.get("id") for m in self.models]


# The background worker's sleep between passes (`WORKER_SLEEP`, SyncWorker.cpp).
WORKER_SLEEP_SECONDS = 120.0
# How long the background worker's `Sync loop complete.` and the last delta must be old for
# quiescence. The worker loops into its next pass within milliseconds when it has more to
# do; the bound is the workers' delta stream delay (500 ms, `STREAM_DELAY_WORKER_MS`), so
# a change written just before the line still reaches stdout inside the window, with as
# much again for scheduling under a full parallel run.
QUIESCENCE_SETTLE = 1.0


@dataclass
class LogLine:
    t: float           # seconds since process start (from our clock, not the engine's)
    thread: str
    level: str
    msg: str
    imap_dir: Optional[str] = None   # sent / recv / sent-private when the line is IMAP traffic
    imap_line: Optional[str] = None
    wall: float = 0.0  # epoch seconds from the line's own timestamp; comparable with `mark` and across processes


@dataclass
class ProcessState:
    deltas: list = field(default_factory=list)
    log: list = field(default_factory=list)
    stdout_noise: list = field(default_factory=list)   # non-JSON stdout, e.g. {"error": ...}
    rpc_errors: list = field(default_factory=list)     # socket: (t, method, error object) of failed requests
    folder_status: dict = field(default_factory=dict)  # folder id -> latest localStatus
    folder_paths: dict = field(default_factory=dict)   # folder id -> path
    last_delta_t: float = 0.0
    mail_new: list = field(default_factory=list)       # socket: (t, params) of each `mail.new`


class RpcChannel:
    """Numbered JSON-RPC 2.0 requests over one newline-delimited stream (a socket, or serve's
    stdin), paired with their responses by the stream's reader, which hands each response
    to `response()`. `failed()` says why waiting for one is pointless (the engine exited,
    the transport broke), or None."""

    def __init__(self, write: Callable[[bytes], None], failed: Callable[[], Optional[str]]):
        self._write = write
        self._failed = failed
        self._seq = itertools.count(1)
        self._pending: dict = {}   # id -> (method, owner), for requests not yet answered
        self._replies: dict = {}   # id -> response, for requests call() waits on
        self._cond = threading.Condition()

    def request(self, method: str, params: dict, owner=None, keep_reply: bool = False) -> int:
        rid = next(self._seq)
        line = json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params}) + "\n"
        with self._cond:
            self._pending[rid] = (method, owner)
            if keep_reply:
                self._replies[rid] = None
        try:
            self._write(line.encode())
        except OSError as e:
            raise MailsyncError(f"{method}: write failed ({e}); {self._failed() or 'engine running'}")
        return rid

    def call(self, method: str, params: dict, timeout: float, owner=None):
        """The request's result; a JSON-RPC error raises MailsyncError."""
        deadline = time.monotonic() + timeout
        rid = self.request(method, params, owner, keep_reply=True)
        with self._cond:
            while self._replies.get(rid) is None:
                left = deadline - time.monotonic()
                why = self._failed()
                if left <= 0 or why:
                    self._replies.pop(rid, None)
                    raise MailsyncError(f"no answer to {method} ({why or f'nothing after {timeout}s'})")
                self._cond.wait(min(left, 0.1))
            reply = self._replies.pop(rid)
        if "error" in reply:
            err = MailsyncError(f"{method} failed: {reply['error']}")
            err.rpc_error = reply["error"]
            raise err
        return reply.get("result")

    def response(self, msg: dict) -> Optional[tuple]:
        """Delivers a response to its call(); returns (method, owner) when nobody waits on it."""
        with self._cond:
            method, owner = self._pending.pop(msg.get("id"), ("?", None))
            if msg.get("id") in self._replies:
                self._replies[msg["id"]] = msg
                self._cond.notify_all()
                return None
        return method, owner


class ServeEngine:
    """One `--mode serve` process shared by every account of a scenario.
   stdin and stdout are the parent channel: `account.add`
    / `account.remove` / `engine.shutdown` requests, their responses, and each account's
    `ProcessAccountSecretsUpdated` as a `delta` notification. Everything else uses one socket
    connection, subscribed to every account; its `delta` and `account.state` notifications
    are routed to the MailsyncProcess registered for their `accountId`."""

    def __init__(self, binary: Path, config_dir: Path, env: dict, verbose: bool, stderr_path: Path):
        self.binary = binary
        self.config_dir = config_dir
        self.env = env
        self.verbose = verbose
        self.stderr_path = stderr_path
        self.proc: Optional[subprocess.Popen] = None
        self.handles: dict = {}        # accountId -> the MailsyncProcess it was last added for
        # Lines for accounts the engine started from accounts.json before their handle was
        # added, replayed to it then.
        self._registered: set = set()
        self._early: dict = {}
        self._route_lock = threading.RLock()
        self.errors: list = []         # engine-wide transport problems; every handle reports them
        self.stdout_noise: list = []
        self.capabilities: list = []   # capabilities.changed params, in order
        # Engine-wide notifications a scenario waits for (rules.progress, rules.disabled, and
        # settings.changed the engine made itself), as (method, params) in order.
        self.notifications: list = []
        self._t0 = 0.0
        self._threads: list = []
        self._stdin_lock = threading.Lock()
        self._sock: Optional[socket.socket] = None
        self._sock_lock = threading.Lock()
        self._sock_dir: Optional[Path] = None
        self._closing = False
        self.parent = RpcChannel(self._write_stdin, self._failed)
        self.rpc = RpcChannel(self._write_socket, self._failed)

    def start(self, timeout: float = 30.0) -> "ServeEngine":
        # macOS caps socket paths at 104 bytes (see MailsyncProcess.start).
        self._sock_dir = Path(tempfile.mkdtemp(prefix="ms-", dir="/tmp"))
        path = str(self._sock_dir / "s")
        args = [str(self.binary), "--mode", "serve"] + (["--verbose"] if self.verbose else []) + ["--socket", path]
        self._t0 = time.monotonic()
        self.proc = subprocess.Popen(args, env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=open(self.stderr_path, "wb"))
        self._spawn(self._read_stdout)
        # stdin is read once the socket is bound, so the first answer means it listens.
        self.parent.call("engine.status", {}, timeout)
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.connect(path)
        self._sock = sock
        self._spawn(self._read_socket)
        hello = self.rpc.call("hello", {"protocol": SOCKET_PROTOCOL, "client": "tern-harness"}, timeout)
        # An engine restarted on the same config dir starts the accounts it registered.
        registered = {a.get("id") for a in self.registered_accounts()}
        self._registered = registered
        unexpected = [a for a in hello.get("accounts") or [] if a.get("accountId") not in registered]
        if hello.get("protocol") != SOCKET_PROTOCOL or unexpected:
            raise MailsyncError(f"unexpected hello result from a serve engine: {hello}")
        self.rpc.call("subscribe", {"accounts": None, "classes": None}, timeout)
        return self

    def registered_accounts(self) -> list:
        """The non-secret Account JSON in the config dir's accounts.json."""
        try:
            return json.loads((self.config_dir / "accounts.json").read_text()).get("accounts") or []
        except (OSError, ValueError):
            return []

    def _spawn(self, target):
        t = threading.Thread(target=target, daemon=True)
        t.start()
        self._threads.append(t)

    @property
    def running(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def _failed(self) -> Optional[str]:
        if self.proc is not None and self.proc.poll() is not None:
            return f"serve engine {describe_exit(self.proc.returncode)}"
        if self.errors:
            return "transport errors: " + "; ".join(self.errors)
        return None

    def _error(self, msg: str):
        self.errors.append(f"[{time.monotonic() - self._t0:.1f}s] {msg}")

    def _write_stdin(self, data: bytes):
        with self._stdin_lock:
            self.proc.stdin.write(data)
            self.proc.stdin.flush()

    def _write_socket(self, data: bytes):
        with self._sock_lock:
            self._sock.sendall(data)

    # -- accounts -------------------------------------------------------------------------

    def add(self, handle: "MailsyncProcess") -> dict:
        """`account.add` for `handle`, which from then on gets that account's deltas and
        states. Answers once the account runs (or stopped while starting)."""
        aid = handle.account["id"]
        with self._route_lock:
            self.handles[aid] = handle
            for line in self._early.pop(aid, []):
                self._on_socket_line(line)
        account = handle.account
        if aid in self._registered:
            # As the desktop does once the engine stored them: the engine's secrets may be
            # newer (a rotated refresh token) than the ones the scenario started with.
            account = dict(account, settings={k: v for k, v in (account.get("settings") or {}).items()
                                              if k not in SECRET_SETTINGS})
        try:
            result = self.parent.call("account.add", {"account": account, "identity": handle.identity},
                                      SERVE_ACCOUNT_TIMEOUT)
        except MailsyncError as e:
            # Registered on an earlier engine, the account was started by this one already.
            err = getattr(e, "rpc_error", None) or {}
            if err.get("code") != RPC_ACCOUNT_EXISTS or "already" not in str(err.get("message")):
                raise
            listed = self.rpc.call("account.list", {}, SERVE_ACCOUNT_TIMEOUT).get("accounts") or []
            mine = next((a for a in listed if a.get("accountId") == aid), {})
            result = {"accountId": aid, "state": mine.get("state"), "error": mine.get("error"), "autostarted": True}
        handle._on_account_state({"accountId": aid, "state": result.get("state"), "error": result.get("error")},
                                 from_reply=True)
        return result

    def remove(self, handle: "MailsyncProcess") -> dict:
        """`account.remove`; returns once its `removed` notification arrived."""
        aid = handle.account["id"]
        result = self.parent.call("account.remove", {"accountId": aid}, SERVE_ACCOUNT_TIMEOUT)
        deadline = time.monotonic() + 10
        while handle.account_state != "removed" and time.monotonic() < deadline and self.running:
            time.sleep(0.05)
        if handle.account_state != "removed":
            self._error(f"no `removed` account.state for {aid} after account.remove")
        return result or {}

    # -- readers --------------------------------------------------------------------------

    def _read_stdout(self):
        for raw in self.proc.stdout:
            line = raw.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                msg = None
            if not isinstance(msg, dict) or msg.get("jsonrpc") != "2.0":
                self.stdout_noise.append(line[:200])
                self._error(f"non-JSON-RPC line on serve stdout: {line[:200]!r}")
                continue
            if msg.get("method") == "delta":
                params = dict(msg.get("params") or {})
                aid = params.pop("accountId", None)
                handle = self.handles.get(aid)
                if params.get("modelClass") != STDOUT_ONLY_DELTA or handle is None:
                    self._error(f"unexpected delta on serve stdout: {line[:200]!r}")
                    continue
                handle._record_delta(handle.elapsed(), params)
            elif "id" in msg and "method" not in msg:
                unclaimed = self.parent.response(msg)
                if unclaimed and msg.get("error"):
                    self._error(f"{unclaimed[0]} on stdin failed: {msg['error']}")
            else:
                self._error(f"unexpected message on serve stdout: {line[:200]!r}")

    def _read_socket(self):
        f = self._sock.makefile("rb")
        try:
            for raw in f:
                line = raw.strip()
                if line:
                    self._on_socket_line(line)
        except OSError:
            pass
        if self._closing:
            return
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            if not self._closing:
                self._error("the serve engine closed the socket while still running")

    def _on_socket_line(self, line: bytes):
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            self._error(f"unparseable socket line: {line[:200]!r}")
            return
        method = msg.get("method")
        params = msg.get("params") or {}
        if method in ("delta", "account.state"):
            aid = params.get("accountId")
            with self._route_lock:
                handle = self.handles.get(aid)
                if handle is None and aid in self._registered:
                    self._early.setdefault(aid, []).append(line)
                    return
            if handle is None:
                self._error(f"{method} for an account never added ({aid!r}): {line[:200]!r}")
            elif method == "account.state":
                handle._on_account_state(params)
            elif params.get("modelClass") == STDOUT_ONLY_DELTA:
                self._error(f"{STDOUT_ONLY_DELTA} on the socket: account secrets must stay on stdout")
            elif handle.account_state == "removed" or "modelClass" not in params:
                # A removed account's DeltaStream is closed. Engine-wide, since the
                # scenario drops a removed account's handle.
                self._error(f"delta for account {aid} in state {handle.account_state}: {line[:200]!r}")
            else:
                d = dict(params)
                d.pop("accountId", None)
                # Every model in an account's stream belongs to that account (MailModel `aid`).
                strays = {m.get("aid") for m in d.get("modelJSONs") or [] if isinstance(m, dict)} - {None, aid}
                if strays:
                    handle._transport_error(f"{d['modelClass']} delta for {aid} carries models of {sorted(strays)}")
                handle._record_delta(handle.elapsed(), d)
        elif method == "mail.new":
            with self._route_lock:
                handle = self.handles.get(params.get("accountId"))
                if handle is None and params.get("accountId") in self._registered:
                    self._early.setdefault(params.get("accountId"), []).append(line)
                    return
            if handle is None:
                self._error(f"mail.new for an account never added: {line[:200]!r}")
            else:
                handle._record_mail_new(handle.elapsed(), params)
        elif method == "account.changed":
            # The registry's non-secret Account JSON: a secret here would reach every client.
            settings = (params.get("account") or {}).get("settings") or {}
            leaked = sorted(k for k in SECRET_SETTINGS if k in settings)
            if leaked:
                self._error(f"account.changed carries {', '.join(leaked)}")
        elif method == "capabilities.changed":
            # Engine-wide: an AI provider's pause, a settings change.
            self.capabilities.append(params)
        elif method in ("rules.progress", "rules.disabled", "settings.changed"):
            self.notifications.append((method, dict(params)))
        elif method == "disconnect":
            self._error(f"engine sent disconnect: {params}")
        elif "id" in msg:
            unclaimed = self.rpc.response(msg)
            if unclaimed and msg.get("error"):
                name, owner = unclaimed
                if owner is not None:
                    owner._rpc_failed(name, msg["error"])
                else:
                    self._error(f"{name} failed: {msg['error']}")
        else:
            self._error(f"unexpected socket message: {line[:200]!r}")

    # -- shutdown -------------------------------------------------------------------------

    def stop(self, graceful: bool = False, timeout: float = 20.0) -> Optional[int]:
        """SIGTERM, as the client kills the engine on quit (what a `restart` reproduces), or
        with `graceful` `engine.shutdown` (every account stopped, deltas flushed, exit 0).
        SIGKILL if it does not exit."""
        if not self.proc:
            return None
        if self.proc.poll() is None:
            if graceful:
                with contextlib.suppress(MailsyncError):
                    self.parent.request("engine.shutdown", {})
            else:
                self.proc.terminate()
            try:
                self.proc.wait(timeout)
            except subprocess.TimeoutExpired:
                self._error(f"the serve engine did not exit within {timeout}s ({'engine.shutdown' if graceful else 'SIGTERM'})")
                self.proc.terminate()
                try:
                    self.proc.wait(5)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                    self.proc.wait()
        self._closing = True
        if self._sock is not None:
            with contextlib.suppress(OSError):
                self._sock.shutdown(socket.SHUT_RDWR)
            with contextlib.suppress(OSError):
                self._sock.close()
        with contextlib.suppress(Exception):
            self.proc.stdin.close()
        for t in self._threads:
            t.join(timeout=2)
        if self._sock_dir is not None:
            shutil.rmtree(self._sock_dir, ignore_errors=True)
        return self.proc.returncode


class MailsyncProcess:
    def __init__(
        self,
        account: dict,
        work_dir: Path,
        binary: Optional[Path] = None,
        verbose: bool = True,
        identity: Optional[dict] = None,
        env: Optional[dict] = None,
        pre_sync_modes: Iterable[str] = (),
        transport: Optional[str] = None,
        name: Optional[str] = None,
        peer: Optional["MailsyncProcess"] = None,
    ):
        """`name` tells a scenario's further accounts apart (their stderr file). `peer` is an
        account already running in the same config dir: this one then skips `--mode migrate`
        (the client migrates once per launch, not per account) and on `serve` joins its
        engine instead of starting one."""
        self.account = account
        self.name = name
        self.peer = peer
        self.work_dir = Path(work_dir)
        self.config_dir = self.work_dir / "config"
        self.config_dir.mkdir(parents=True, exist_ok=True)
        self.verbose = verbose
        self.identity = identity
        self.extra_env = env or {}
        self.binary = self._launchable_binary(Path(binary).resolve() if binary else find_binary())
        self.proc: Optional[subprocess.Popen] = None
        self.state = ProcessState()
        self._lock = threading.Lock()
        self._t0 = 0.0
        self._threads: list = []
        self._log_pos = 0
        self._log_lock = threading.Lock()
        self._stopped = threading.Event()
        self.wait_reason = ""
        self.migrate_output = ""
        # One-shot modes run after migrate and before sync, as the client would run them
        # (`import-legacy`), and what each printed.
        self.pre_sync_modes = list(pre_sync_modes)
        self.mode_outputs: dict = {}
        self.stderr_path = self.work_dir / (f"stderr-{name}.log" if name else "stderr.log")
        self.transport = transport or default_transport()
        if self.transport not in TRANSPORTS:
            raise MailsyncError(f"unknown transport {self.transport!r}")
        # Socket transport state. Problems on the socket fail the next send or wait, and the
        # scenario's expectations (transport_errors).
        self._transport_errors: list = []
        self._sock: Optional[socket.socket] = None
        self._sock_dir: Optional[Path] = None
        self._sock_lock = threading.Lock()
        self._rpc: Optional[RpcChannel] = None
        self._listening: Optional[str] = None
        self._listening_event = threading.Event()
        self._closing = False
        # serve: the shared engine, whether this account started it, and the account's
        # `account.state` history as (t, state, error).
        self.engine: Optional[ServeEngine] = None
        self.owns_engine = False
        self.account_state: Optional[str] = None
        self.account_error: Optional[dict] = None
        self.account_states: list = []
        # Queries see only this account's rows (temp views over every table with an
        # accountId column), for scenarios with several accounts in one database.
        self.scoped_db = False

    # -- lifecycle ----------------------------------------------------------------------

    def _launchable_binary(self, binary: Path) -> Path:
        # Release builds refuse to start unless argv[0] contains the product name: "tern" for
        # the Rust engine, "mailspring" for the C++ one (main.cpp, the executable-path check).
        # A symlink inside the work dir whose name has both satisfies either binary,
        # regardless of where the real one lives.
        link_dir = self.work_dir / "tern-mailspring-bin"
        link_dir.mkdir(parents=True, exist_ok=True)
        link = link_dir / "mailsync"
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(binary)
        return link

    def _env(self) -> dict:
        env = dict(os.environ)
        env.update(
            CONFIG_DIR_PATH=str(self.config_dir),
            # Only the metadata worker talks to the identity server, and with --identity
            # null it exits immediately. The variable must still be set or main() bails.
            IDENTITY_SERVER="http://127.0.0.1:9/unreachable",
        )
        # Never the OS keychain (conftest sets one key file for every case; run.py doesn't).
        env.setdefault("TERN_ENGINE_KEY_FILE", str(self.work_dir / "engine.key"))
        env.setdefault("TERN_ENGINE_RESTART_DELAY_MS", "600000")
        env.update(self.extra_env)
        return env

    def _identity_arg(self) -> str:
        return json.dumps(self.identity) if self.identity else "null"

    def run_mode(self, mode: str, timeout: float = 120) -> subprocess.CompletedProcess:
        """Run a one-shot mode (migrate, test, reset) and return its result."""
        return subprocess.run(
            [str(self.binary), "--mode", mode, "--identity", self._identity_arg(),
             "--account", json.dumps(self.account)],
            env=self._env(), capture_output=True, text=True, timeout=timeout,
        )

    def migrate(self):
        r = self.run_mode("migrate")
        # e.g. "Migration V10: 376 placements created, 10 messages without a copy"
        self.migrate_output = " ".join(r.stdout.split())
        if r.returncode < 0:
            raise MailsyncError(f"migrate {describe_exit(r.returncode)}: {r.stdout[-500:]} {r.stderr[-500:]}")
        if r.returncode != 0 or '"error":null' not in r.stdout.replace(" ", ""):
            hint = " (exit 2 with no output is the executable-path check: argv[0] must contain 'tern' for the Rust engine, 'mailspring' for the C++ one)" if r.returncode == 2 else ""
            raise MailsyncError(f"migrate failed ({r.returncode}){hint}: {r.stdout[-500:]} {r.stderr[-500:]}")

    def run_one_shot(self, mode: str, timeout: float = 120) -> str:
        """Runs a one-shot mode that must succeed: exit 0 and `{"error":null}` on stdout, the
        contract every one-shot mode shares."""
        r = self.run_mode(mode, timeout=timeout)
        out = " ".join(r.stdout.split())
        self.mode_outputs[mode] = out
        if r.returncode != 0 or '"error":null' not in r.stdout.replace(" ", ""):
            raise MailsyncError(f"--mode {mode} failed ({describe_exit(r.returncode)}): {r.stdout[-500:]} {r.stderr[-500:]}")
        return out

    def start(self):
        # The client runs `--mode migrate` before every launch (application.ts), and it is
        # the only mode that upgrades the schema: a build started on an older database
        # without it would run against tables it does not have.
        if self.peer is None:
            self.migrate()
        for mode in self.pre_sync_modes:
            self.run_one_shot(mode)
        if self.transport == "serve":
            self._start_serve()
            return
        args = [str(self.binary), "--mode", "sync"]
        if self.verbose:
            args.append("--verbose")
        args += ["--identity", self._identity_arg(), "--account", json.dumps(self.account)]
        if self.transport == "socket":
            # macOS caps socket paths at 104 bytes and pytest's work dirs under /var/folders
            # are longer, so the socket lives in a fresh short dir (0700 from mkdtemp).
            self._sock_dir = Path(tempfile.mkdtemp(prefix="ms-", dir="/tmp"))
            args += ["--socket", str(self._sock_dir / "s")]
        self._t0 = time.monotonic()
        self.proc = subprocess.Popen(
            args, env=self._env(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=open(self.stderr_path, "wb"), bufsize=1, text=True,
        )
        for target in (self._read_stdout, self._tail_log):
            t = threading.Thread(target=target, daemon=True)
            t.start()
            self._threads.append(t)
        if self.transport == "socket":
            self._connect()

    def start_refused(self, timeout: float = 15.0) -> Optional[str]:
        """Starts the account expecting the engine to refuse it before it connects anywhere:
        returns the engine's error (serve: `account.add`'s error message; stdio and socket:
        the one-shot `{"error": ...}` line before the process exits), or None when the
        account was accepted, in which case it is removed or stopped again."""
        try:
            self.start()
        except MailsyncError as e:
            err = getattr(e, "rpc_error", None)
            if self.transport == "serve":
                if err is None:
                    raise
                self.engine.handles.pop(self.account["id"], None)
                self._stopped.set()
                for t in self._threads:
                    t.join(timeout=2)
                return str(err.get("message") or json.dumps(err))
        if self.transport == "serve":
            self.remove_account()
            return None
        deadline = time.monotonic() + timeout
        while self.proc.poll() is None and time.monotonic() < deadline:
            time.sleep(0.05)
        exited = self.proc.poll() is not None
        self.stop()
        if not exited:
            return None
        for _, line in self.state.stdout_noise:
            with contextlib.suppress(json.JSONDecodeError):
                d = json.loads(line)
                if isinstance(d, dict) and d.get("error"):
                    return str(d["error"])
        return f"exited ({describe_exit(self.exit_code)}) with no error line"

    def _start_serve(self):
        if self.peer is not None:
            if self.peer.engine is None or not self.peer.engine.running:
                raise MailsyncError("the serve engine this account joins is not running")
            self.engine = self.peer.engine
        else:
            self.engine = ServeEngine(self.binary, self.config_dir, self._env(), self.verbose, self.stderr_path)
            self.owns_engine = True
            self.engine.start()
        self.proc = self.engine.proc
        self._t0 = time.monotonic()
        t = threading.Thread(target=self._tail_log, daemon=True)
        t.start()
        self._threads.append(t)
        self.engine.add(self)

    def _on_account_state(self, params: dict, from_reply: bool = False):
        """An `account.state` notification, or `account.add`'s answer before any arrived."""
        if from_reply and self.account_state is not None:
            return
        with self._lock:
            self.account_state = params.get("state")
            self.account_error = params.get("error")
            self.account_states.append((self.elapsed(), self.account_state, self.account_error))

    def remove_account(self) -> dict:
        """serve: `account.remove`, the engine keeps running for the other accounts."""
        if self.transport != "serve":
            raise MailsyncError("remove_account is serve-only; stop() the process instead")
        result = self.engine.remove(self)
        self._stopped.set()
        for t in self._threads:
            t.join(timeout=2)
        self._drain_log()
        return result

    @property
    def transport_errors(self) -> list:
        own = list(self._transport_errors)
        return own + (list(self.engine.errors) if self.engine else [])

    # -- socket transport -----------------------------------------------------------------

    def _connect(self, timeout: float = 30.0):
        """Waits for `{"listening": path}`, connects, and sends hello and subscribe. Returns
        once subscribed, so no delta is missed (the engine also holds the deltas it flushed
        before the first subscribe and replays them)."""
        deadline = time.monotonic() + timeout
        while not self._listening_event.wait(0.1):
            if self.proc.poll() is not None or time.monotonic() > deadline:
                what = describe_exit(self.exit_code) if self.proc.poll() is not None else f"nothing after {timeout}s"
                raise MailsyncError(f"no listening line from mailsync --socket ({what})"
                                    f"\n--- stderr ---\n{self.stderr_tail()}")
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.connect(self._listening)
        self._sock = sock
        self._rpc = RpcChannel(self._write_socket, self._rpc_wait_failed)
        t = threading.Thread(target=self._read_socket, args=(sock,), daemon=True)
        t.start()
        self._threads.append(t)
        left = max(0.1, deadline - time.monotonic())
        hello = self._rpc.call("hello", {"protocol": SOCKET_PROTOCOL, "client": "tern-harness"}, left)
        ids = [a.get("accountId") for a in hello.get("accounts", [])]
        if hello.get("protocol") != SOCKET_PROTOCOL or self.account["id"] not in ids:
            raise MailsyncError(f"unexpected hello result: {hello}")
        self._rpc.call("subscribe", {"accounts": None, "classes": None}, max(0.1, deadline - time.monotonic()))

    def _write_socket(self, data: bytes):
        with self._sock_lock:
            self._sock.sendall(data)

    def _rpc_wait_failed(self) -> Optional[str]:
        if self.proc.poll() is not None:
            return f"mailsync {describe_exit(self.exit_code)}"
        if self._transport_errors:
            return f"transport errors: {self._transport_errors}"
        return None

    def _rpc_failed(self, method: str, err: dict):
        """A request whose answer nobody waited on failed. A task whose local phase failed is
        an outcome (its Task delta carries the error), anything else a transport problem."""
        with self._lock:
            self.state.rpc_errors.append((self.elapsed(), method, err))
        if err.get("code") != RPC_TASK_REJECTED:
            self._transport_error(f"{method} failed: {err}")

    def _transport_error(self, msg: str):
        with self._lock:
            self._transport_errors.append(f"[{self.elapsed():.1f}s] {msg}")

    def _read_socket(self, sock: socket.socket):
        f = sock.makefile("rb")
        try:
            for raw in f:
                line = raw.strip()
                if line:
                    self._on_socket_line(line)
        except OSError:
            pass   # stop() closed it
        if self._closing:
            return
        # The engine closes the socket when it exits; give an exit in progress a moment.
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            if not self._closing:
                self._transport_error("the engine closed the socket while still running")

    def _on_socket_line(self, line: bytes):
        t = self.elapsed()
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            self._transport_error(f"unparseable socket line: {line[:200]!r}")
            return
        method = msg.get("method")
        if method == "delta":
            params = dict(msg.get("params") or {})
            account_id = params.pop("accountId", None)
            if account_id != self.account["id"] or "modelClass" not in params:
                self._transport_error(f"delta for account {account_id!r}: {line[:200]!r}")
                return
            self._record_delta(t, params)
        elif method == "mail.new":
            params = msg.get("params") or {}
            if params.get("accountId") != self.account["id"]:
                self._transport_error(f"mail.new for account {params.get('accountId')!r}: {line[:200]!r}")
                return
            self._record_mail_new(t, params)
        elif method == "disconnect":
            self._transport_error(f"engine sent disconnect: {msg.get('params')}")
        elif "id" in msg:
            unclaimed = self._rpc.response(msg)
            if unclaimed and msg.get("error"):
                self._rpc_failed(unclaimed[0], msg["error"])
        else:
            self._transport_error(f"unexpected socket message: {line[:200]!r}")

    def _close_socket(self):
        self._closing = True
        if self._sock is not None:
            with contextlib.suppress(OSError):
                self._sock.shutdown(socket.SHUT_RDWR)
            with contextlib.suppress(OSError):
                self._sock.close()

    def stop(self, timeout: float = 5.0, graceful: bool = False) -> Optional[int]:
        """Closes stdin and SIGTERMs the engine, as the client does on quit. On serve,
        `graceful` shuts the engine down with `engine.shutdown` instead (scenario teardown)."""
        if not self.proc:
            return None
        if self.transport == "serve":
            return self._stop_serve(graceful)
        self._close_socket()
        if self.proc.poll() is None:
            try:
                self.proc.stdin.close()
            except Exception:
                pass
            self.proc.terminate()
            try:
                self.proc.wait(timeout)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self._stopped.set()
        for t in self._threads:
            t.join(timeout=2)
        self._drain_log()
        if self._sock_dir is not None:
            shutil.rmtree(self._sock_dir, ignore_errors=True)
        return self.proc.returncode

    def _stop_serve(self, graceful: bool) -> Optional[int]:
        """The account that started the engine stops it (every other account with it); any
        other one is removed from it if it is still there."""
        if self.owns_engine:
            code = self.engine.stop(graceful=graceful)
        else:
            if self.engine.running and self.account_state != "removed":
                with contextlib.suppress(MailsyncError):
                    self.engine.remove(self)
            code = self.engine.proc.poll()
        self._stopped.set()
        for t in self._threads:
            t.join(timeout=2)
        self._drain_log()
        return code

    @contextlib.contextmanager
    def held(self):
        """SIGSTOP the engine for the duration, SIGCONT after: the server changes made inside
        land as one batch however long they take, instead of being split by a worker pass
        that wakes from its 120 s sleep halfway through. Its sockets stay open; anything the
        server sends meanwhile is read on resume. POSIX only.

        On `serve` this stops the one engine, so every account is held; scenarios that use it
        have one account.

        If the background worker's sleep ran out while held, its pass starts on resume; a
        `wake-workers` sent during that pass is not seen until the next sleep ends, so this
        waits for the pass to finish before returning."""
        if not self.running:
            raise MailsyncError(f"mailsync is not running ({self.describe_stop()})")
        done = [l for l in self.log("background", engine_only=True) if l.msg == "Sync loop complete."]
        asleep_since = done[-1].t if done else None
        self.proc.send_signal(signal.SIGSTOP)
        try:
            yield
        finally:
            resumed = self.elapsed()
            if self.proc.poll() is None:
                self.proc.send_signal(signal.SIGCONT)
        if asleep_since is not None and resumed - asleep_since >= WORKER_SLEEP_SECONDS - 5:
            self.wait_for_log(r"^Syncing folder list", timeout=30, since=asleep_since + 0.001)
            self.wait_quiescent(timeout=600)

    @property
    def exit_code(self) -> Optional[int]:
        return self.proc.poll() if self.proc else None

    @property
    def running(self) -> bool:
        if self.proc is None or self.proc.poll() is not None:
            return False
        return self.transport != "serve" or self.account_state not in ACCOUNT_DOWN

    def describe_stop(self) -> str:
        """Why `running` is false: the exit, or on serve the account's state and error key."""
        if self.transport == "serve" and self.exit_code is None and self.account_state in ACCOUNT_DOWN:
            key = (self.account_error or {}).get("key")
            return f"account {self.account_state}" + (f" with {key}" if key else "")
        return describe_exit(self.exit_code)

    def stopped_error(self) -> Optional[dict]:
        """The SyncError JSON that stopped this account: `account.state`'s error on serve; on
        stdio and socket the one the process logged as critical before it aborted."""
        if self.transport == "serve":
            return self.account_error if self.account_state == "stopped" else None
        self._drain_log()
        for l in reversed(self.log()):
            if l.level == "critical":
                m = re.search(r"(\{.*\})\s*$", l.msg)
                if m:
                    with contextlib.suppress(json.JSONDecodeError):
                        return json.loads(m.group(1))
                return {"key": None, "message": l.msg}
        return None

    def elapsed(self) -> float:
        return time.monotonic() - self._t0

    # -- stdin commands -------------------------------------------------------------------

    def send(self, packet: dict):
        """A stdin command. On the socket transport the ones with a method become requests;
        their answers are read asynchronously, as stdin has none."""
        if not self.running:
            raise MailsyncError(f"mailsync is not running ({self.describe_stop()})")
        if self.transport_errors:
            raise MailsyncError("socket transport failed: " + "; ".join(self.transport_errors))
        method = STDIN_TO_METHOD.get(packet.get("type")) if self.transport != "stdio" else None
        aid = self.account["id"]
        if self.transport == "serve" and method is None:
            # serve's stdin speaks JSON-RPC only; `test.crash` is the per-account test-crash
            # (debug builds only).
            if packet.get("type") != "test-crash":
                raise MailsyncError(f"{packet.get('type')!r} has no serve equivalent")
            self.engine.parent.call("test.crash", {"accountId": aid}, SERVE_ACCOUNT_TIMEOUT)
            return
        if method is None:
            self.proc.stdin.write(json.dumps(packet) + "\n")
            self.proc.stdin.flush()
            return
        params = {
            "sync.wake": lambda: {"accountId": aid},
            "task.queue": lambda: {"accountId": aid, "task": packet["task"]},
            "task.cancel": lambda: {"accountId": aid, "taskId": packet["taskId"]},
            "bodies.fetch": lambda: {"accountId": aid, "ids": packet["ids"]},
            "calendar.sync": lambda: {"accountId": aid},
        }[method]()
        if self.transport == "serve":
            self.engine.rpc.request(method, params, owner=self)
        else:
            self._rpc.request(method, params)

    def wake(self):
        """Ends the background worker's 120s sleep and re-marks folders busy, exactly as the
        client does when the user clicks Sync Mail."""
        self.send({"type": "wake-workers"})

    def queue_task(self, task: dict) -> str:
        """Queue a task the way Actions.queueTask does. `task` needs __cls and the fields
        that class's performLocal/performRemote read; id/accountId/status are filled in."""
        task = dict(task)
        # The sequence number keeps tasks queued within one millisecond apart.
        task.setdefault("id", f"task-{int(time.time() * 1000)}-{len(self.state.deltas)}-{next(_task_seq)}")
        task.setdefault("aid", self.account["id"])   # MailModel's JSON key for accountId
        task.setdefault("status", "local")
        task.setdefault("v", 0)
        task.setdefault("metadata", [])
        self.send({"type": "queue-task", "task": task})
        return task["id"]

    def rpc_call(self, method: str, params: dict, timeout: float = SERVE_ACCOUNT_TIMEOUT):
        """One socket request and its result (socket and serve); an error raises MailsyncError
        with `rpc_error`."""
        if self.transport == "serve":
            return self.engine.rpc.call(method, params, timeout)
        if self.transport == "socket":
            return self._rpc.call(method, params, timeout)
        raise MailsyncError(f"{method} needs the socket; this scenario runs on {self.transport}")

    def set_settings(self, values: dict) -> dict:
        """serve: `settings.set`, as the desktop's settings mirror writes the engine's store."""
        if self.transport != "serve":
            raise MailsyncError("settings.set is serve-only")
        return self.engine.rpc.call("settings.set", {"values": values}, SERVE_ACCOUNT_TIMEOUT)

    def need_bodies(self, message_ids: Iterable[str]):
        self.send({"type": "need-bodies", "ids": list(message_ids)})

    def sync_calendar(self):
        """The client's calendar refresh: one calendars-only DAV pass on the `calendar` thread,
        never two at once (main.cpp `sync-calendar`). Contacts keep their own cadence."""
        self.send({"type": "sync-calendar"})

    # -- observation ----------------------------------------------------------------------

    def _read_stdout(self):
        socket_mode = self.transport == "socket"
        for line in self.proc.stdout:
            line = line.strip()
            if not line:
                continue
            t = self.elapsed()
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                d = None
            if socket_mode and isinstance(d, dict) and "listening" in d and not self._listening_event.is_set():
                self._listening = d["listening"]
                self._listening_event.set()
                continue
            if not isinstance(d, dict) or "modelClass" not in d:
                with self._lock:
                    self.state.stdout_noise.append((t, line))
                continue
            if socket_mode and d["modelClass"] != STDOUT_ONLY_DELTA:
                self._transport_error(f"delta on stdout in socket mode: {line[:200]}")
                continue
            self._record_delta(t, d)

    def _record_delta(self, t: float, d: dict):
        delta = Delta(t, d.get("type", ""), d["modelClass"], d.get("modelJSONs", []), d, wall=time.time())
        with self._lock:
            self.state.deltas.append(delta)
            self.state.last_delta_t = t
            if delta.model_class == "Folder":
                for m in delta.models:
                    if delta.type == "persist":
                        self.state.folder_status[m["id"]] = m.get("localStatus", {})
                        self.state.folder_paths[m["id"]] = m.get("path")
                    else:
                        self.state.folder_status.pop(m["id"], None)

    def _record_mail_new(self, t: float, params: dict):
        with self._lock:
            self.state.mail_new.append((t, dict(params)))

    def mail_new(self) -> list:
        """The `mail.new` notifications so far, as (t, params). Socket transports only."""
        with self._lock:
            return list(self.state.mail_new)

    def notifications(self, method: str) -> list:
        """serve: the params of the engine-wide `method` notifications so far, in order."""
        if self.transport != "serve":
            return []
        return [p for m, p in list(self.engine.notifications) if m == method]

    @property
    def log_path(self) -> Path:
        return self.config_dir / f"mailsync-{self.account['id']}.log"

    def _tail_log(self):
        while not self._stopped.is_set():
            self._drain_log()
            time.sleep(0.1)

    def _drain_log(self):
        if not self.log_path.exists():
            return
        with self._log_lock:   # the tailer thread and waiters both drain
            # spdlog rotates at 5 MB (mailsync-<id>.log -> mailsync-<id>.1.log). --verbose
            # logs every IMAP line, so a large mailbox rotates several times per run.
            if self.log_path.stat().st_size < self._log_pos:
                rotated = self.log_path.with_name(self.log_path.name + ".1")
                if rotated.exists():
                    with open(rotated, "rb") as f:
                        f.seek(self._log_pos)
                        tail = f.read()
                    cut = tail.rfind(b"\n")
                    if cut >= 0:
                        self._ingest_log(tail[:cut])
                self._log_pos = 0
            with open(self.log_path, "rb") as f:
                f.seek(self._log_pos)
                chunk = f.read()
            # only consume complete lines; a line the engine is still writing waits for next time
            cut = chunk.rfind(b"\n")
            if cut < 0:
                return
            self._log_pos += cut + 1
            self._ingest_log(chunk[:cut])

    def _ingest_log(self, chunk: bytes):
        t = self.elapsed()
        new = []
        for raw in chunk.decode("utf-8", "replace").split("\n"):
            m = LOG_LINE.match(raw.rstrip("\r"))
            if not m:
                continue  # continuation lines of multi-line IMAP literals
            ll = LogLine(t, m["thread"], m["level"], m["msg"], wall=_log_epoch(m["ts"]))
            im = IMAP_LINE.match(ll.msg)
            if im:
                ll.imap_dir, ll.imap_line = im["dir"], im["line"]
            new.append(ll)
        with self._lock:
            self.state.log.extend(new)

    def deltas(self, model_class: Optional[str] = None, since: float = 0.0) -> list:
        with self._lock:
            return [d for d in self.state.deltas
                    if d.t >= since and (model_class is None or d.model_class == model_class)]

    def log(self, thread: Optional[str] = None, engine_only: bool = False, since: float = 0.0) -> list:
        with self._lock:
            return [l for l in self.state.log
                    if l.t >= since and (thread is None or l.thread == thread)
                    and (not engine_only or l.imap_dir is None)]

    def imap_transcript(self, thread: Optional[str] = None) -> list:
        """(thread, sent|recv, line) for every IMAP line the engine logged (needs --verbose)."""
        return [(l.thread, l.imap_dir, l.imap_line) for l in self.log(thread) if l.imap_dir]

    def log_contains(self, pattern: str, since: float = 0.0) -> bool:
        rx = re.compile(pattern)
        return any(rx.search(l.msg) for l in self.log(since=since))

    def grep(self, pattern: str, since: float = 0.0) -> list:
        rx = re.compile(pattern)
        return [l for l in self.log(since=since) if rx.search(l.msg)]

    # -- database -------------------------------------------------------------------------

    @property
    def db_path(self) -> Path:
        return self.config_dir / (CPP_DATABASE if is_cpp_engine(self.binary) else RUST_DATABASE)

    def db(self, scoped: Optional[bool] = None):
        """`with proc.db() as conn:` - closed on exit (sqlite3's own context manager only
        commits, and a lingering connection re-creates the WAL files after teardown).
        The engine owns writes; the harness only reads. query_only enforces that at the
        connection level (mode=ro cannot be used: a WAL database needs the -shm file, which a
        read-only opener may not create before the engine has opened it).

        With `scoped_db` (or `scoped=True`) every table with an accountId column is shadowed
        by a temp view of this account's rows, so the harness's unqualified queries see one
        account of a shared database. Tables without one (ThreadCounts, MessageBody, ...)
        stay whole; the invariants check the whole database (`scoped=False`)."""
        conn = sqlite3.connect(str(self.db_path), timeout=10)
        conn.row_factory = sqlite3.Row
        if self.scoped_db if scoped is None else scoped:
            aid = self.account["id"].replace("'", "''")
            tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type = 'table'")]
            for t in tables:
                cols = [r[1] for r in conn.execute(f'PRAGMA main.table_info("{t}")')]
                if "accountId" in cols:
                    conn.execute(f'CREATE TEMP VIEW "{t}" AS SELECT * FROM main."{t}" WHERE accountId = \'{aid}\'')
        conn.execute("PRAGMA query_only = 1")
        return contextlib.closing(conn)

    def db_folders(self) -> dict:
        """path -> {id, role, localStatus} straight from the database."""
        with self.db() as c:
            rows = c.execute("SELECT id, path, role, data FROM Folder").fetchall()
        out = {}
        for r in rows:
            data = json.loads(r["data"])
            out[r["path"]] = {"id": r["id"], "role": r["role"], "localStatus": data.get("localStatus", {})}
        return out

    # -- waiting --------------------------------------------------------------------------

    def wait_for(self, predicate: Callable[[], bool], timeout: float, what: str = "condition",
                 poll: float = 0.1) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if not self.running:
                raise MailsyncError(f"mailsync stopped ({self.describe_stop()}) while waiting for {what}"
                                    f"\n--- stderr ---\n{self.stderr_tail()}")
            if self.transport_errors:
                raise MailsyncError(f"socket transport failed while waiting for {what}: "
                                    + "; ".join(self.transport_errors))
            if predicate():
                return
            time.sleep(poll)
        raise TimeoutError(f"timed out after {timeout}s waiting for {what}\n{self.describe()}")

    def wait_for_log(self, pattern: str, timeout: float = 60, since: float = 0.0) -> LogLine:
        rx = re.compile(pattern)
        found = []

        def check():
            for l in self.log(since=since):
                if rx.search(l.msg):
                    found.append(l)
                    return True
            return False

        self.wait_for(check, timeout, f"log line /{pattern}/")
        return found[0]

    def _folders_idle(self, ignore_busy: Iterable[str] = ()) -> bool:
        folders = self.db_folders()
        if not folders:
            return False
        for path, f in folders.items():
            if path in ignore_busy:
                continue
            ls = f["localStatus"]
            if ls.get("busy") is True:   # folders the engine never syncs (\All) have no busy key
                return False
            if ls.get("syncedMinUID", 0) > 1:
                return False
        return True

    def _background_asleep(self, settle: float) -> bool:
        bg = self.log("background", engine_only=True)
        if not bg:
            return False
        last = bg[-1]
        return last.msg == "Sync loop complete." and self.elapsed() - last.t >= settle

    def _foreground_idling(self) -> bool:
        fg = self.log("foreground")
        if not fg:
            return False
        # Without IDLE (NetEase) the foreground parks here instead, which is its idle state.
        if fg[-1].imap_dir is None and fg[-1].msg.startswith("Connection does not support idling"):
            return True
        if self.verbose:
            sent = [l for l in fg if l.imap_dir == "sent"]
            # Tag-agnostic: mailcore numbers tags, Coremail servers need `C`-prefixed ones.
            return bool(sent) and IDLE_COMMAND.match(sent[-1].imap_line or "") is not None
        engine = [l for l in fg if l.imap_dir is None]
        return bool(engine) and engine[-1].msg.startswith("Idling on folder")

    def wait_quiescent(self, timeout: float = 120, settle: float = QUIESCENCE_SETTLE, require_idle: bool = True,
                       ignore_busy: Iterable[str] = ()) -> None:
        """Block until the engine has nothing left to do. See the module docstring.
        ignore_busy: folder paths whose busy flag is known never to clear (the skipped \\All
        mailbox), so they do not hold up the wait."""

        def check():
            self._drain_log()
            if not self._folders_idle(ignore_busy):
                self.wait_reason = "a folder is busy or has syncedMinUID > 1"
                return False
            if not self._background_asleep(settle):
                self.wait_reason = "background worker is not asleep"
                return False
            if require_idle and not self._foreground_idling():
                self.wait_reason = "foreground worker is not idling"
                return False
            with self._lock:
                last_delta = self.state.last_delta_t
            if self.elapsed() - last_delta < settle:
                self.wait_reason = "deltas still arriving"
                return False
            self.wait_reason = ""
            return True

        self.wait_for(check, timeout, "quiescence", poll=0.2)

    def sync_pass(self, timeout: float = 120, settle: float = QUIESCENCE_SETTLE, ignore_busy: Iterable[str] = ()) -> float:
        """wake-workers, then wait for the resulting background pass to finish. Returns the
        timestamp of the wake so callers can scope delta/log queries to this pass."""
        t = self.elapsed()
        self.wake()
        self.wait_for_log(r"^Marking all folders as `busy`", timeout=30, since=t)
        self.wait_quiescent(timeout=timeout, settle=settle, ignore_busy=ignore_busy)
        return t

    # -- diagnostics ----------------------------------------------------------------------

    def stderr_tail(self, n: int = 40) -> str:
        try:
            return "\n".join(self.stderr_path.read_text(errors="replace").splitlines()[-n:])
        except FileNotFoundError:
            return ""

    def describe(self) -> str:
        bg = self.log("background", engine_only=True)[-3:]
        fg = self.log("foreground", engine_only=True)[-3:]
        folders = {p: {k: v for k, v in f["localStatus"].items() if k in ("busy", "syncedMinUID", "uidnext")}
                   for p, f in self.db_folders().items()}
        return (
            f"elapsed={self.elapsed():.1f}s exit={self.describe_stop()} deltas={len(self.state.deltas)}\n"
            f"folders={json.dumps(folders)}\n"
            f"bg tail: {[l.msg[:100] for l in bg]}\nfg tail: {[l.msg[:100] for l in fg]}\n"
            f"fg idling={self._foreground_idling()} waiting because: {self.wait_reason!r} "
            f"stdout noise={self.state.stdout_noise[-3:]}"
            + (f" transport={self.transport} errors={self.transport_errors[-3:]}" if self.transport != "stdio" else "")
            + (f" account.state={self.account_states[-3:]}" if self.transport == "serve" else "")
        )

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *exc):
        self.stop()
