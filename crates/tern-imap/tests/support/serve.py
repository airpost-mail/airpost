"""
Starts one of the harness's IMAP servers for the tern-imap integration tests.

    python3 serve.py <kind> [profile] [--populate] [--tls | --starttls]

kind is fake, dovecot or cyrus (Docker for the last two); profile is the fake personality or
the Dovecot / Cyrus profile name. --populate fills INBOX and Archive the way the conformance
probes do (apps/engine/test/conformance/compare.py). --tls gives the fake implicit TLS with a
fresh self-signed certificate for `localhost`, whose PEM path is printed as `cert`;
--starttls offers the same certificate through STARTTLS instead.

Prints one JSON line {"host", "port", "user", "password", "cert"} when the server is ready,
then reads JSON lines {"op": <Server method>, "args": [...]} and answers each with
{"ok": result} or {"error": text}. Stops the server when stdin closes.
"""
import base64
import json
import os
import ssl
import subprocess
import sys
import tempfile
from pathlib import Path

TEST = Path(__file__).resolve().parents[4] / "apps" / "engine" / "test"
sys.path.insert(0, str(TEST))

from harness import mailgen  # noqa: E402
from harness.servers.fake import FakeServer  # noqa: E402


def populate(server):
    server.populate("INBOX", mailgen.messages(3) + [mailgen.message(4, html=True, attachment=("a.txt", b"hello"))]
                    + mailgen.messages(8, start=5))
    server.set_flags("INBOX", [2, 3], remove=["\\Seen"])
    archive = mailgen.messages(2, start=100)
    try:
        server.populate("Archive", archive)
    except KeyError:
        # Personalities without an Archive folder (gmail, courier, …).
        server.create_mailbox("Archive")
        server.populate("Archive", archive)


def self_signed(dirname: str):
    """A throwaway CA and a `localhost` certificate it signed. Returns the server context and
    the CA's PEM path, which is what a client must trust."""
    path = lambda n: os.path.join(dirname, n)  # noqa: E731
    run = lambda *a: subprocess.run(list(a), check=True, capture_output=True)  # noqa: E731
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", path("ca.key"), "-out", path("ca.pem"),
        "-days", "2", "-subj", "/CN=tern-imap test CA", "-addext", "basicConstraints=critical,CA:TRUE",
        "-addext", "keyUsage=critical,keyCertSign")
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", path("key.pem"), "-out", path("req.csr"),
        "-subj", "/CN=localhost")
    with open(path("ext.cnf"), "w") as f:
        f.write("subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n")
    run("openssl", "x509", "-req", "-in", path("req.csr"), "-CA", path("ca.pem"), "-CAkey", path("ca.key"),
        "-CAcreateserial", "-out", path("cert.pem"), "-days", "2", "-extfile", path("ext.cnf"))
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(path("cert.pem"), path("key.pem"))
    return ctx, path("ca.pem")


def _fail_dns_fast(module):
    """Cyrus looks up the client's reverse DNS before it greets; where the container's resolver
    never answers that costs 10 s per connection, longer than the harness waits for the
    greeting. A resolver that refuses at once makes the lookup fail immediately."""
    real_run = module.subprocess.run

    class _Subprocess:
        def __getattr__(self, name):
            return getattr(subprocess, name)

        @staticmethod
        def run(cmd, *a, **kw):
            if cmd[:2] == ["docker", "run"]:
                cmd = cmd[:2] + ["--dns", "127.0.0.1"] + cmd[2:]
            return real_run(cmd, *a, **kw)

    module.subprocess = _Subprocess()


def make_server(kind: str, profile: str, tls: str, workdir: str):
    if kind == "fake":
        kw = {}
        cert = None
        if tls:
            ctx, cert = self_signed(workdir)
            kw = dict(tls_context=ctx, implicit_tls=(tls == "implicit"))
        return FakeServer(profile or "dovecot", **kw), cert
    if kind == "dovecot":
        from harness.servers.dovecot import DovecotServer
        return DovecotServer(profile or "qresync"), None
    if kind == "cyrus":
        from harness.servers import cyrus
        _fail_dns_fast(cyrus)
        return cyrus.CyrusServer(profile or "fastmail"), None
    raise SystemExit(f"unknown server kind {kind}")


def decode_arg(value):
    if isinstance(value, str) and value.startswith("b64:"):
        return base64.b64decode(value[4:])
    if isinstance(value, list):
        return [decode_arg(v) for v in value]
    return value


def jsonable(value):
    if isinstance(value, bytes):
        return base64.b64encode(value).decode()
    if isinstance(value, (set, tuple)):
        return [jsonable(v) for v in value]
    if isinstance(value, list):
        return [jsonable(v) for v in value]
    if isinstance(value, dict):
        return {str(k): jsonable(v) for k, v in value.items()}
    return value


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}
    kind = args[0]
    profile = args[1] if len(args) > 1 else ""
    workdir = tempfile.mkdtemp(prefix="tern-imap-test-")
    tls = "implicit" if "--tls" in flags else "starttls" if "--starttls" in flags else ""
    server, cert = make_server(kind, profile, tls, workdir)
    server.start()
    try:
        if "--populate" in flags:
            populate(server)
        print(json.dumps({"host": server.host, "port": server.port, "user": server.username,
                          "password": server.password, "cert": cert}), flush=True)
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
                call_args = [decode_arg(a) for a in req.get("args", [])]
                if req["op"] == "oauth_tokens":
                    # Makes the fake accept exactly these XOAUTH2 access tokens.
                    server.imap.oauth_tokens = set(call_args[0])
                    result = None
                else:
                    result = getattr(server, req["op"])(*call_args)
                print(json.dumps({"ok": jsonable(result)}), flush=True)
            except Exception as e:  # report and keep serving
                print(json.dumps({"error": repr(e)}), flush=True)
    finally:
        server.stop()


if __name__ == "__main__":
    main()
