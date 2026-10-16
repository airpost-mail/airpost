#!/usr/bin/env python3
"""
Fake mail accounts for local development (docs/deployment-plan.md 8.2): the harness's fake
IMAP and SMTP servers, in memory, seeded with mail, for pointing a dev desktop, CLI or
headless engine at. Never a real account.

    python3 apps/engine/test/tools/dev_mail.py [--host 127.0.0.1] [--users alice,bob]

Each user gets an IMAP port (1143, 1144, …) and an SMTP port (1025, 1026, …), no TLS,
username = the address, password `ternpass`. Mail sent to any dev user's address over any of
the SMTP ports lands in that user's INBOX, so alice can mail bob. Mail to other addresses
is accepted and dropped. Everything is lost on exit and reseeded on start.
"""
import argparse
import signal
import sys
import time
from pathlib import Path

TEST_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TEST_DIR))

from fakeimap import FakeImapServer  # noqa: E402
from fakeimap.smtp import FakeSmtpServer  # noqa: E402
from harness import mailgen  # noqa: E402

DOMAIN = "tern.test"
PASSWORD = "ternpass"


class RoutingSmtpServer(FakeSmtpServer):
    """Delivers each recipient that is a dev user to that user's INBOX."""

    def __init__(self, stores: dict, **kw):
        super().__init__(**kw)
        self.stores = stores

    def deliver(self, msg):
        for rcpt in msg.recipients:
            store = self.stores.get(rcpt.lower())
            if store is not None:
                store.append("INBOX", msg.raw, [])
        print(f"smtp {msg.sender} -> {', '.join(msg.recipients)} ({len(msg.raw)} bytes)", flush=True)


def seed(imap: FakeImapServer, name: str, email: str):
    to = f"{name.title()} <{email}>"
    inbox = [mailgen.message(n, to=to, age_days=14 - n) for n in range(1, 13)]
    for i, raw in enumerate(inbox):
        imap.store.append("INBOX", raw, ["\\Seen"] if i < 8 else [])
    for raw in mailgen.thread(20, 3, to=to):
        imap.store.append("INBOX", raw, [])
    for n in range(30, 34):
        imap.store.append("Archive", mailgen.message(n, to=to, age_days=40 + n), ["\\Seen"])
    me = mailgen.self_addressed(40, me=to)
    imap.store.append("INBOX", me, [])
    imap.store.append("Sent", me, ["\\Seen"])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="127.0.0.1", help="address to listen on (0.0.0.0 in a container)")
    ap.add_argument("--users", default="alice,bob", help="comma-separated local parts")
    ap.add_argument("--imap-port", type=int, default=1143, help="first user's IMAP port")
    ap.add_argument("--smtp-port", type=int, default=1025, help="first user's SMTP port")
    ap.add_argument("--personality", default="dovecot")
    a = ap.parse_args()

    names = [u.strip() for u in a.users.split(",") if u.strip()]
    stores = {}
    servers = []
    for i, name in enumerate(names):
        email = f"{name}@{DOMAIN}"
        imap = FakeImapServer(a.personality, host=a.host, port=a.imap_port + i, credentials=(email, PASSWORD))
        if "Archive" not in imap.store.mailboxes:
            imap.store.create("Archive", ["\\Archive"], subscribed=True)
        seed(imap, name, email)
        stores[email] = imap.store
        servers.append((email, imap))
    smtps = []
    for i, (email, imap) in enumerate(servers):
        smtp = RoutingSmtpServer(stores, host=a.host, port=a.smtp_port + i, credentials=(email, PASSWORD))
        smtps.append(smtp)
        imap.start()
        smtp.start()
        print(f"{email}: IMAP {a.host}:{imap.port}, SMTP {a.host}:{smtp.port}, password {PASSWORD}, no TLS",
              flush=True)

    def stop(*_):
        for _, imap in servers:
            imap.stop()
        for smtp in smtps:
            smtp.stop()
        sys.exit(0)

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    while True:
        time.sleep(3600)


if __name__ == "__main__":
    main()
