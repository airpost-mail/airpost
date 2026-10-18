#!/usr/bin/env python3
"""
The stack the browser run (e2e/web.spec.ts) drives: the harness's fake IMAP and SMTP servers
with two seeded accounts, a `serve` engine on a fresh profile under /tmp, and tern-gateway
serving the built web app, all on ports the system picks. Never a real account or profile.

    python3 apps/web/e2e/stack.py --web-root apps/web/dist [--work /tmp/tern-web-e2e-xyz]

Once everything runs it writes `<work>/stack.json` (origin, a pairing URL, accounts, the
control URL) and prints the same line on stdout. The control server (loopback, no auth, for
the run only) answers:

    GET  /smtp              every message the SMTP servers accepted: from, to, subject, parts
    GET  /imap?user=alice&mailbox=INBOX   subjects and flags on the fake IMAP server
    GET  /settings          the engine's settings store (`<config>/settings.json`)
    POST /settings          `{values}` to `settings.set` on a connection of the stack's own
    POST /pair              a new pairing code
    POST /engine/kill       SIGKILL the engine (the gateway closes tabs with 4502)
    POST /engine/start      a new engine on the same profile, both accounts added
    POST /deliver?user=alice&subject=…   a new message in that user's INBOX

SIGTERM or SIGINT stops everything it started.
"""
import argparse
import contextlib
import hashlib
import email
import email.policy
import json
import os
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from datetime import datetime, timedelta, timezone
from email.message import EmailMessage
from email.utils import format_datetime, make_msgid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

REPO = Path(__file__).resolve().parents[3]
TEST_DIR = REPO / "apps" / "engine" / "test"
sys.path.insert(0, str(TEST_DIR))

from fakeimap import FakeImapServer  # noqa: E402
from fakeimap.smtp import FakeSmtpServer  # noqa: E402
from harness import mailgen  # noqa: E402
from harness.mailsync import MailsyncProcess, account_json  # noqa: E402

DOMAIN = "tern.test"
PASSWORD = "ternpass"
USERS = ("alice", "bob")
NOW = datetime.now(timezone.utc).replace(microsecond=0)


def png(w: int = 24, h: int = 16, rgb=(220, 40, 40)) -> bytes:
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


INLINE_PNG = png()
REPORT_PDF = b"%PDF-1.4\n% tern e2e attachment\n" + bytes(range(256)) * 8 + b"\n%%EOF\n"


def mime(to: str, sender: tuple, subject: str, days_ago: float, text: str, html: str = None,
         attachments=(), inline=(), headers=None) -> bytes:
    m = EmailMessage(policy=email.policy.SMTP)
    m["From"] = f"{sender[0]} <{sender[1]}>"
    m["To"] = to
    m["Subject"] = subject
    m["Date"] = format_datetime(NOW - timedelta(days=days_ago))
    m["Message-ID"] = make_msgid(domain="e2e.example")
    for k, v in (headers or {}).items():
        m[k] = v
    m.set_content(text)
    if html:
        m.add_alternative(html, subtype="html")
        if inline:
            related = m.get_payload()[1]
            for cid, name, data in inline:
                related.add_related(data, maintype="image", subtype="png", cid=f"<{cid}>", filename=name)
    for name, data, mt, st in attachments:
        m.add_attachment(data, maintype=mt, subtype=st, filename=name)
    return m.as_bytes()


def seed(imap: FakeImapServer, name: str, addr: str):
    to = f"{name.title()} <{addr}>"
    store = imap.store
    for mb, attrs in (("Archive", ["\\Archive"]),):
        if mb not in store.mailboxes:
            store.create(mb, attrs, subscribed=True)
    # Plain mail over two weeks, eight read, plus a three-message thread.
    for i, n in enumerate(range(1, 13)):
        store.append("INBOX", mailgen.message(n, to=to, age_days=14 - n, domain=f"{name}.e2e"),
                     ["\\Seen"] if i < 8 else [])
    for raw in mailgen.thread(20, 3, to=to, domain=f"{name}.e2e"):
        store.append("INBOX", raw, [])
    for n in range(30, 34):
        store.append("Archive", mailgen.message(n, to=to, age_days=40 + n, domain=f"{name}.e2e"), ["\\Seen"])
    if name == "bob":
        store.append("INBOX", mime(to, ("Gus Gardener", "gus@garden.example"), "Garden plans for spring", 0.25,
                                   "Tulips along the fence."), [])
        return
    carol = ("Carol Finance", "carol@finance.example")
    store.append("INBOX", mime(to, carol, "Quarterly zebra report", 0.2,
                               "The report is attached.\n\nCarol",
                               attachments=[("zebra-report.pdf", REPORT_PDF, "application", "pdf")]), [])
    store.append("INBOX", mime(to, ("Dana Travel", "dana@travel.example"), "Photos from the lighthouse", 0.3,
                               "A photo is inline.",
                               html='<html><body><p>Here is the lighthouse:</p>'
                                    '<p><img src="cid:lighthouse@e2e" alt="lighthouse" width="24" height="16"></p>'
                                    '</body></html>',
                               inline=[("lighthouse@e2e", "lighthouse.png", INLINE_PNG)]), [])
    store.append("INBOX", mime(to, ("Weekly Digest", "digest@news.example"), "Your weekly digest", 0.4,
                               "Top stories this week.\n",
                               headers={"List-Unsubscribe": "<mailto:leave@news.example?subject=unsubscribe>",
                                        "List-Id": "Weekly Digest <digest.news.example>"}), [])
    store.append("INBOX", mime(to, ("Offers", "offers@shop.example"), "Autumn offers", 0.5,
                               "Offers inside.\n",
                               headers={"List-Unsubscribe": "<https://shop.example/unsubscribe?u=42>"}), [])
    # Old enough for a before: search and recent enough for after:.
    store.append("INBOX", mime(to, carol, "Budget planning kickoff", 60,
                               "Planning starts next month."), ["\\Seen"])


class RoutingSmtpServer(FakeSmtpServer):
    """Delivers each recipient that is a stack user to that user's INBOX."""

    def __init__(self, stores: dict, **kw):
        super().__init__(**kw)
        self.stores = stores

    def deliver(self, msg):
        for rcpt in msg.recipients:
            store = self.stores.get(rcpt.lower())
            if store is not None:
                store.append("INBOX", msg.raw, [])


def summarize(raw: bytes) -> dict:
    m = email.message_from_bytes(raw, policy=email.policy.default)
    parts = []
    for p in m.walk():
        if p.is_multipart():
            continue
        payload = p.get_payload(decode=True) or b""
        parts.append({"type": p.get_content_type(), "filename": p.get_filename(), "size": len(payload),
                      "sha256": hashlib.sha256(payload).hexdigest()})
    return {"from": m.get("From"), "to": m.get("To"), "subject": m.get("Subject"), "parts": parts,
            "text": next((p.get_content() for p in m.walk() if p.get_content_type() == "text/plain"), None)}


class Stack:
    def __init__(self, work: Path, web_root: Path, mailsync: Path, gateway: Path):
        self.work = work
        self.web_root = web_root
        self.mailsync = mailsync
        self.gateway_bin = gateway
        self.imaps = {}
        self.smtps = {}
        self.accounts = {}
        self.handles = []
        self.gateway = None
        self.engine_starts = 0
        self.lock = threading.Lock()

    # -- mail servers ---------------------------------------------------------------------
    def start_mail(self):
        stores = {}
        for name in USERS:
            addr = f"{name}@{DOMAIN}"
            imap = FakeImapServer("dovecot", port=0, credentials=(addr, PASSWORD))
            seed(imap, name, addr)
            stores[addr] = imap.store
            imap.start()
            self.imaps[name] = imap
        for name in USERS:
            addr = f"{name}@{DOMAIN}"
            smtp = RoutingSmtpServer(stores, port=0, credentials=(addr, PASSWORD))
            smtp.start()
            self.smtps[name] = smtp
            # Ids starting c-f skip the engine's start delay.
            self.accounts[name] = account_json(
                "127.0.0.1", self.imaps[name].port, account_id=f"e2e{name}", email=addr, username=addr,
                password=PASSWORD, smtp_port=smtp.port, extra_settings={"create_helper_folders": True})
            self.accounts[name]["name"] = name.title()
            self.accounts[name]["label"] = addr

    # -- engine ---------------------------------------------------------------------------
    def start_engine(self):
        with self.lock:
            self.engine_starts += 1
            env = {"RUST_LOG": "info"}
            first = None
            handles = []
            for name in USERS:
                h = MailsyncProcess(self.accounts[name], self.work, binary=self.mailsync, verbose=True, env=env,
                                    transport="serve", name=f"{name}-{self.engine_starts}", peer=first)
                h.scoped_db = True
                h.start()
                handles.append(h)
                first = first or h
            for h in handles:
                h.wait_quiescent(timeout=180)
            self.handles = handles

    def kill_engine(self):
        with self.lock:
            if not self.handles:
                return
            owner = self.handles[0]
            with contextlib.suppress(ProcessLookupError):
                os.kill(owner.engine.proc.pid, signal.SIGKILL)
            owner.engine.proc.wait()
            for h in self.handles:
                with contextlib.suppress(Exception):
                    h.stop()
            self.handles = []

    def engine_call(self, method: str, params: dict) -> dict:
        """One request on a connection of its own, as the desktop's would be: the harness's
        connection treats notifications it doesn't know (`settings.changed`) as errors."""
        with self.lock:
            path = str(self.handles[0].engine._sock_dir / "s")
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(30)
            sock.connect(path)
            reader = sock.makefile("rb")
            for i, (m, p) in enumerate((("hello", {"protocol": 1, "client": "tern-web-e2e"}), (method, params)), 1):
                sock.sendall(json.dumps({"jsonrpc": "2.0", "id": i, "method": m, "params": p}).encode() + b"\n")
                while True:
                    msg = json.loads(reader.readline())
                    if msg.get("id") == i:
                        break
                if "error" in msg:
                    raise RuntimeError(f"{m}: {msg['error']}")
            return msg["result"]

    @property
    def config_dir(self) -> Path:
        return self.work / "config"

    # -- gateway --------------------------------------------------------------------------
    def start_gateway(self):
        env = {k: v for k, v in os.environ.items()
               if not (k.startswith("TERN_GATEWAY_") or k in ("TERN_CONFIG_DIR", "TERN_TRUSTED_PROXIES"))}
        env["RUST_LOG"] = "info"
        env["TERN_GATEWAY_WEB_ROOT"] = str(self.web_root)
        self.gateway_env = env
        self.gateway = subprocess.Popen(
            [str(self.gateway_bin), "--config-dir", str(self.config_dir), "serve", "--bind", "127.0.0.1:0"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=open(self.work / "gateway.log", "ab"),
            env=env)
        running = self.config_dir / "gateway" / "gateway.json"
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and self.gateway.poll() is None:
            with contextlib.suppress(OSError, ValueError, KeyError):
                info = json.loads(running.read_text())
                if info["pid"] == self.gateway.pid:
                    self.origin = info["publicUrl"].rstrip("/")
                    return
            time.sleep(0.1)
        raise RuntimeError("tern-gateway didn't start; see " + str(self.work / "gateway.log"))

    def pair_code(self) -> dict:
        r = subprocess.run([str(self.gateway_bin), "--config-dir", str(self.config_dir), "pair", "--json",
                            "--name", "e2e browser"], capture_output=True, text=True, env=self.gateway_env,
                           timeout=30)
        if r.returncode != 0:
            raise RuntimeError(f"pair failed: {r.stderr}")
        return json.loads(r.stdout)

    def stop(self):
        if self.gateway and self.gateway.poll() is None:
            self.gateway.terminate()
            try:
                self.gateway.wait(10)
            except subprocess.TimeoutExpired:
                self.gateway.kill()
        with contextlib.suppress(Exception):
            self.kill_engine()
        for s in list(self.imaps.values()) + list(self.smtps.values()):
            with contextlib.suppress(Exception):
                s.stop()

    # -- control --------------------------------------------------------------------------
    def smtp_log(self) -> list:
        out = []
        for name, smtp in self.smtps.items():
            for msg in list(smtp.messages):
                out.append({"server": name, "sender": msg.sender, "recipients": msg.recipients,
                            **summarize(msg.raw)})
        return out

    def imap_state(self, user: str, mailbox: str) -> list:
        mb = self.imaps[user].store.get(mailbox)
        if mb is None:
            return []
        return [{"uid": m.uid, "subject": m.header("Subject"), "flags": sorted(m.flags)} for m in mb.messages]


def serve_control(stack: Stack) -> ThreadingHTTPServer:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def answer(self, code: int, body):
            data = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            u = urlparse(self.path)
            q = {k: v[0] for k, v in parse_qs(u.query).items()}
            if u.path == "/smtp":
                return self.answer(200, stack.smtp_log())
            if u.path == "/imap":
                return self.answer(200, stack.imap_state(q.get("user", "alice"), q.get("mailbox", "INBOX")))
            if u.path == "/settings":
                path = stack.config_dir / "settings.json"
                return self.answer(200, json.loads(path.read_text()) if path.exists() else {})
            self.answer(404, {"error": "no such route"})

        def do_POST(self):
            u = urlparse(self.path)
            q = {k: v[0] for k, v in parse_qs(u.query).items()}
            try:
                if u.path == "/pair":
                    return self.answer(200, stack.pair_code())
                if u.path == "/settings":
                    body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
                    return self.answer(200, stack.engine_call("settings.set", {"values": body["values"]}))
                if u.path == "/engine/kill":
                    stack.kill_engine()
                    return self.answer(200, {"ok": True})
                if u.path == "/engine/start":
                    stack.start_engine()
                    return self.answer(200, {"ok": True, "starts": stack.engine_starts})
                if u.path == "/deliver":
                    user = q.get("user", "alice")
                    raw = mime(f"{user.title()} <{user}@{DOMAIN}>", ("Eve Later", "eve@later.example"),
                               q.get("subject", "A new message"), 0, "Delivered during the run.")
                    stack.imaps[user].store.append("INBOX", raw, [])
                    return self.answer(200, {"ok": True})
            except Exception as e:  # the run reports it
                return self.answer(500, {"error": repr(e)})
            self.answer(404, {"error": "no such route"})

    srv = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--web-root", default=str(REPO / "apps" / "web" / "dist"))
    ap.add_argument("--work", help="a fresh directory (default: a new one under /tmp)")
    ap.add_argument("--mailsync", default=os.environ.get("MAILSYNC_BIN", str(REPO / "target" / "release" / "mailsync")))
    ap.add_argument("--gateway", default=os.environ.get("TERN_GATEWAY_BIN",
                                                        str(REPO / "target" / "release" / "tern-gateway")))
    a = ap.parse_args()
    work = Path(a.work) if a.work else Path(tempfile.mkdtemp(prefix="tern-web-e2e-", dir="/tmp"))
    work.mkdir(parents=True, exist_ok=True)
    home = Path.home().resolve()
    if work.resolve() == home / "tern-trial" or home / "tern-trial" in work.resolve().parents:
        sys.exit("refusing to run on ~/tern-trial")
    if not (Path(a.web_root) / "index.html").is_file():
        sys.exit(f"no built web app in {a.web_root}: VITE_TERN_ENGINE=gateway npm run build")

    stack = Stack(work, Path(a.web_root).resolve(), Path(a.mailsync).resolve(), Path(a.gateway).resolve())
    done = threading.Event()

    def stop(*_):
        done.set()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    control = None
    try:
        stack.start_mail()
        stack.start_engine()
        stack.start_gateway()
        control = serve_control(stack)
        info = {
            "work": str(work),
            "origin": stack.origin,
            "pair": stack.pair_code(),
            "control": f"http://127.0.0.1:{control.server_address[1]}",
            "accounts": {n: {"id": acc["id"], "email": acc["emailAddress"]} for n, acc in stack.accounts.items()},
            "inlinePngSha256": hashlib.sha256(INLINE_PNG).hexdigest(),
            "reportPdfSha256": hashlib.sha256(REPORT_PDF).hexdigest(),
            "pid": os.getpid(),
        }
        (work / "stack.json").write_text(json.dumps(info, indent=2))
        print(json.dumps(info), flush=True)
        done.wait()
    finally:
        if control:
            control.shutdown()
        stack.stop()


if __name__ == "__main__":
    main()
