#!/usr/bin/env python3
"""Runs `--mode test` and `--mode install-check` on two mailsync binaries and diffs the JSON.

    python3 apps/engine/tools/test_mode_ab.py \\
        --rust target/release/mailsync \\
        --cpp /Applications/Mailspring.app/Contents/Resources/app.asar.unpacked/mailsync \\
        [--servers fake:dovecot,dovecot:qresync,dovecot:tls,cyrus:fastmail] [--cases good,wrong-password] \\
        [--install-check] [--show-log]

Servers are the harness's (apps/engine/test/harness/servers): the in-process fake with its
SMTP sink, and Dovecot and Cyrus in Docker with the harness SMTP sink attached in front.
Each case derives an account from the harness's `account_json` and breaks one thing.

What must match between the binaries: the exit code,
`error`, `error_service`, `error_offline`, whether `account` is null, the account's JSON when
it is not, and `error_advice` with the TLS stack's reason in parentheses masked (OpenSSL and
rustls word their rejections differently) and the product name normalised (the advice says
"Tern" where the C++ said "Mailspring"). `log` is not compared: tag shapes, capability text
and the TLS notes differ by design; `--show-log` prints both.

install-check compares the key set and each check's outcome key (`success`, `error`,
`status_code`, `skipped`). The C++ binary checks against $IDENTITY_SERVER/ping (set here to
https://oauth2.googleapis.com, not id.getmailspring.com), imap.gmail.com, smtp.gmail.com
and imap.shaw.ca; the Rust one against the Google OAuth token endpoint, imap.gmail.com and
smtp.gmail.com, so both need network access.

Differences the shipped C++ build is known to produce (`known_divergence`) are listed but
not counted. Exit status is 0 when every other case matches.
"""
import argparse
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
TEST_DIR = REPO / "apps" / "engine" / "test"
sys.path.insert(0, str(TEST_DIR))
from harness.mailsync import account_json  # noqa: E402

DEFAULT_CPP = "/Applications/Mailspring.app/Contents/Resources/app.asar.unpacked/mailsync"
DEFAULT_SERVERS = "fake:dovecot,fake:gmail,dovecot:qresync,dovecot:tls,cyrus:fastmail"


def closed_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def settings(**kw):
    return lambda acct: acct["settings"].update(kw)


# (server kind, case) -> why the binaries are expected to differ. Reported, not counted.
KNOWN_DIVERGENCES = {
    ("dovecot:tls", "self-signed-allow-insecure"):
        "the shipped C++ build skips certificate validation when imap_allow_insecure_ssl is set; "
        "the C++ source (MailUtils::configureSessionForAccount) and the Rust engine validate always",
}

KNOWN_SEEN: list = []

CERTIFICATE_ADVICE_PREFIX = "Tern could not verify this server's certificate"


def known_divergence(spec: str, name: str, v_rust: dict, v_cpp: dict):
    """Why the two results may differ, or None. Applied only to differing results."""
    if (spec, name) in KNOWN_DIVERGENCES:
        return KNOWN_DIVERGENCES[(spec, name)]
    # The certificate branch of MailUtils::tlsFailureAdvice is newer than the shipped C++
    # build (1.26.0 has only the handshake branch); the Rust advice follows the source.
    rest_rust = {k: v for k, v in v_rust.items() if k not in ("error_advice", "keys")}
    rest_cpp = {k: v for k, v in v_cpp.items() if k not in ("error_advice", "keys")}
    if (rest_rust == rest_cpp and v_cpp.get("error") == "ErrorCertificate"
            and v_cpp.get("error_advice") is None
            and (v_rust.get("error_advice") or "").startswith(CERTIFICATE_ADVICE_PREFIX)):
        return "certificate advice (MailUtils.cpp:789) post-dates the shipped C++ build"
    return None


def oauth_without_client_id(acct):
    acct["provider"] = "gmail"
    st = acct["settings"]
    st.pop("imap_password", None)
    st.pop("smtp_password", None)
    st.update(refresh_token="refresh-token", refresh_client_id="")


def reaches_smtp(srv) -> bool:
    # dovecot:tls stops every account at its self-signed certificate.
    return not srv.ssl


# name -> (applies to server?, account mutation)
CASES = {
    "good": (lambda srv: True, lambda acct: None),
    "smtp-login-only": (reaches_smtp, settings(smtp_verification="login")),
    "wrong-password": (reaches_smtp, settings(imap_password="wrong-password")),
    "wrong-port": (lambda srv: True, lambda acct: acct["settings"].update(imap_port=closed_port())),
    # Implicit TLS against a plaintext listener: the handshake fails.
    "tls-on-plain-port": (lambda srv: True, settings(imap_security="SSL / TLS")),
    # STARTTLS against a server that offers none (Dovecot and Cyrus here run without TLS).
    "starttls-unavailable": (lambda srv: not srv.ssl, settings(imap_security="STARTTLS")),
    # dovecot:tls offers STARTTLS on its one listener with a self-signed certificate for
    # CN=localhost, so its base account uses STARTTLS and every case but this one stops at
    # the certificate.
    "plaintext-on-starttls-server": (lambda srv: srv.ssl, settings(imap_security="none")),
    "self-signed-allow-insecure": (lambda srv: srv.ssl, settings(imap_allow_insecure_ssl=True)),
    "smtp-wrong-password": (reaches_smtp, settings(smtp_password="wrong-password")),
    "smtp-wrong-port": (reaches_smtp, lambda acct: acct["settings"].update(smtp_port=closed_port())),
    "smtp-login-wrong-password": (reaches_smtp,
                                  settings(smtp_verification="login", smtp_password="wrong-password")),
    "smtp-tls-on-plain-port": (reaches_smtp, settings(smtp_security="SSL / TLS")),
    "smtp-starttls-unavailable": (reaches_smtp, settings(smtp_security="STARTTLS")),
    # An OAuth account whose token cannot be refreshed: the error_offline path, before any
    # network request (the C++ binary does not read MAILSYNC_GOOGLE_TOKEN_URL, so a refresh
    # against a fake endpoint cannot be compared; apps/engine/tests/test_mode.rs covers it).
    "oauth-without-client-id": (lambda srv: True, oauth_without_client_id),
}


def start_server(spec: str):
    kind, _, profile = spec.partition(":")
    if kind == "fake":
        from harness.servers.fake import FakeServer
        srv = FakeServer(profile or "dovecot", smtp=True)
        srv.start()
        srv.ssl = False
        return srv
    if kind == "dovecot":
        from harness.servers.dovecot import DovecotServer
        srv = DovecotServer(profile or "qresync")
    elif kind == "cyrus":
        from harness.servers.cyrus import CyrusServer
        srv = CyrusServer(profile or "fastmail")
    else:
        raise SystemExit(f"unknown server kind {kind}")
    srv.start()
    wait_for_greeting(srv.port)
    srv.attach_smtp()
    if srv.ssl:
        # The harness delivers self-addressed mail with IMAP4_SSL, which this STARTTLS
        # listener does not speak; nothing here reads the delivered copy.
        srv.smtp.deliver_to = None
    return srv


def base_account(srv) -> dict:
    kw = srv.account_kwargs()
    ssl = getattr(srv, "ssl", False)
    if ssl:
        kw["imap_security"] = "STARTTLS"
    acct = account_json(**kw)
    # account_json turns allow-insecure on with any TLS; the shipped C++ build then skips
    # certificate validation, which `self-signed-allow-insecure` covers on its own.
    acct["settings"]["imap_allow_insecure_ssl"] = False
    return acct


def wait_for_greeting(port: int, timeout: float = 60):
    # Docker publishes the port before the server inside accepts, so a TCP connect alone
    # proves nothing; dovecot:tls greets in plaintext too (STARTTLS).
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=2) as s:
                if s.recv(200).startswith(b"* OK"):
                    return
        except OSError:
            pass
        time.sleep(0.3)
    raise RuntimeError(f"no IMAP greeting on {port}")


def launchable(binary: Path, work: Path) -> Path:
    # Release builds check argv[0] for the product name ("tern" / "mailspring").
    link_dir = work / "tern-mailspring-bin"
    link_dir.mkdir(parents=True, exist_ok=True)
    link = link_dir / "mailsync"
    if link.is_symlink() or link.exists():
        link.unlink()
    link.symlink_to(binary.resolve())
    return link


def run_binary(binary: Path, args: list, timeout: float = 120, env_extra: dict = None) -> dict:
    with tempfile.TemporaryDirectory(prefix="test-mode-ab-") as tmp:
        work = Path(tmp)
        config = work / "config"
        config.mkdir()
        env = dict(os.environ, CONFIG_DIR_PATH=str(config), IDENTITY_SERVER="http://127.0.0.1:9/unreachable")
        env.update(env_extra or {})
        try:
            p = subprocess.run([str(launchable(binary, work)), *args], env=env, capture_output=True,
                               text=True, timeout=timeout)
        except subprocess.TimeoutExpired as e:
            return {"exit": "timeout", "json": None, "stdout": e.stdout or "", "stderr": e.stderr or ""}
    result, deltas = None, []
    for line in p.stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "modelClass" in obj:
            deltas.append(obj)
        else:
            result = obj
    return {"exit": p.returncode, "json": result, "deltas": deltas, "stdout": p.stdout, "stderr": p.stderr}


# The TLS stack's reason, in "certificate (…). " and "connection (…). ".
ADVICE_REASON = re.compile(r"(certificate|connection) \([^()]*\)\. ")


def normalise_advice(text):
    if text is None:
        return None
    return ADVICE_REASON.sub(r"\1 (<reason>). ", text.replace("Mailspring", "Tern"))


def test_mode_view(r: dict) -> dict:
    j = r["json"]
    if j is None:
        return {"exit": r["exit"], "json": None, "stderr": r["stderr"][-500:]}
    return {
        "exit": r["exit"],
        "keys": sorted(j.keys()),
        "error": j.get("error"),
        "error_service": j.get("error_service"),
        "error_offline": j.get("error_offline"),
        "error_advice": normalise_advice(j.get("error_advice")),
        "account": j.get("account"),
    }


def run_test_mode(args):
    rust, cpp = Path(args.rust), Path(args.cpp)
    only = set(args.cases.split(",")) if args.cases else None
    mismatches, total = [], 0
    for spec in args.servers.split(","):
        print(f"== {spec}", flush=True)
        srv = start_server(spec)
        try:
            for name, (applies, mutate) in CASES.items():
                if only and name not in only:
                    continue
                if not applies(srv):
                    continue
                acct = base_account(srv)
                mutate(acct)
                argv = ["--mode", "test", "--identity", "null", "--account", json.dumps(acct)]
                r_rust = run_binary(rust, argv)
                r_cpp = run_binary(cpp, argv)
                v_rust, v_cpp = test_mode_view(r_rust), test_mode_view(r_cpp)
                total += 1
                # The client turns any stderr output into a crash report; only the Rust
                # binary is held to that (the C++ one prints OpenSSL noise there).
                same = v_rust == v_cpp and r_rust["stderr"] == ""
                if r_rust["stderr"]:
                    print(f"       rust stderr: {r_rust['stderr'][-500:]!r}")
                if r_cpp["stderr"] and args.show_log:
                    print(f"       c++ stderr: {r_cpp['stderr'][-500:]!r}")
                summary = f"{v_cpp.get('error')}/{v_cpp.get('error_service')}"
                known = None if same or r_rust["stderr"] else known_divergence(spec, name, v_rust, v_cpp)
                mark = "ok  " if same else ("KNWN" if known else "DIFF")
                print(f"  {mark} {name:28} C++ {summary} exit {v_cpp['exit']}", flush=True)
                if known:
                    KNOWN_SEEN.append((spec, name, known))
                    print(f"       known divergence: {known}")
                elif not same:
                    mismatches.append((spec, name))
                    for k in sorted(set(v_rust) | set(v_cpp)):
                        if v_rust.get(k) != v_cpp.get(k):
                            print(f"       {k}: rust={json.dumps(v_rust.get(k))}\n"
                                  f"       {' ' * len(k)}  c++ ={json.dumps(v_cpp.get(k))}")
                if args.show_log or not same:
                    for label, r in (("rust", r_rust), ("c++", r_cpp)):
                        j = r["json"] or {}
                        if j.get("error_advice"):
                            print(f"       {label} advice: {j['error_advice']}")
                        if args.show_log:
                            print(f"       --- {label} log ---")
                            print("       " + (j.get("log") or "").replace("\n", "\n       "))
        finally:
            srv.stop()
    return mismatches, total


def install_check_view(r: dict) -> dict:
    j = r["json"]
    if j is None:
        return {"exit": r["exit"], "json": None}
    checks = {}
    for k, v in j.items():
        if k.endswith("_check"):
            checks[k] = sorted(v.keys()) if isinstance(v, dict) else v
    return {
        "keys": sorted(j.keys()),
        "error": j.get("error"),
        "checks": checks,
        "log_is_string": isinstance(j.get("log"), str),
        "exit": r["exit"],
    }


def run_install_check(args):
    argv = ["--mode", "install-check"]
    # The C++ binary GETs $IDENTITY_SERVER/ping; any HTTP status passes, so it is pointed at
    # the same Google host the Rust check uses rather than id.getmailspring.com.
    r_rust = run_binary(Path(args.rust), argv)
    r_cpp = run_binary(Path(args.cpp), argv, env_extra={"IDENTITY_SERVER": "https://oauth2.googleapis.com"})
    v_rust, v_cpp = install_check_view(r_rust), install_check_view(r_cpp)
    print("== install-check")
    for label, r in (("rust", r_rust), ("c++", r_cpp)):
        j = dict(r["json"] or {})
        j.pop("log", None)
        print(f"  {label}: exit {r['exit']} {json.dumps(j)}")
    same = v_rust == v_cpp
    if not same:
        for k in sorted(set(v_rust) | set(v_cpp)):
            if v_rust.get(k) != v_cpp.get(k):
                print(f"  DIFF {k}: rust={json.dumps(v_rust.get(k))} c++={json.dumps(v_cpp.get(k))}")
    else:
        print("  ok   shape and outcomes match")
    return [] if same else [("install-check", "")]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--rust", default=os.environ.get("MAILSYNC_BIN", str(REPO / "target" / "release" / "mailsync")))
    ap.add_argument("--cpp", default=DEFAULT_CPP)
    ap.add_argument("--servers", default=DEFAULT_SERVERS)
    ap.add_argument("--cases", default=None, help=f"comma-separated subset of {', '.join(CASES)}")
    ap.add_argument("--install-check", action="store_true", help="also diff install-check")
    ap.add_argument("--only-install-check", action="store_true")
    ap.add_argument("--show-log", action="store_true")
    args = ap.parse_args()

    mismatches, total = [], 0
    if not args.only_install_check:
        mismatches, total = run_test_mode(args)
    if args.install_check or args.only_install_check:
        mismatches += run_install_check(args)
        total += 1
    print(f"\n{total - len(mismatches) - len(KNOWN_SEEN)}/{total} match, "
          f"{len(KNOWN_SEEN)} known divergences, {len(mismatches)} mismatches")
    for spec, name in mismatches:
        print(f"  mismatch: {spec} {name}")
    sys.exit(1 if mismatches else 0)


if __name__ == "__main__":
    main()
