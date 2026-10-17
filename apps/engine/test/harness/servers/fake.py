"""The in-process fake server behind the Server interface."""
import re
import sys
from pathlib import Path
from typing import Optional

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from fakeimap import FakeImapServer, Store  # noqa: E402
from fakeimap.smtp import FakeSmtpServer  # noqa: E402
from .base import Server


class FakeServer(Server):
    kind = "fake"

    def __init__(self, personality: str = "dovecot", log_path: Optional[str] = None, smtp: bool = False,
                 email: str = "test@example.test", reject_non_fqdn_helo: bool = False,
                 smtp_sent_copy: Optional[str] = None, smtp_sent_copy_delay: float = 0,
                 people: bool = False, people_options: Optional[dict] = None, **kw):
        self.personality_name = personality
        self.imap = FakeImapServer(personality, log_path=log_path, credentials=(self.username, self.password), **kw)
        self.gmail = self.imap.personality.gmail
        self.store: Store = self.imap.store
        self.email = email
        # Mail the user sends to themself is delivered back into INBOX, as a real MTA would.
        # `smtp_sent_copy` names a mailbox every submitted message is filed into as well
        # (Gmail's submission service saves sent mail under \Sent by itself).
        self.smtp = FakeSmtpServer(credentials=(self.username, self.password), reject_non_fqdn_helo=reject_non_fqdn_helo,
                                   deliver_to=(self.store, "INBOX", email),
                                   sent_copy=(self.store, smtp_sent_copy) if smtp_sent_copy else None,
                                   sent_copy_delay=smtp_sent_copy_delay) if smtp else None
        # `people: true` makes this a Google account: OAuth through the fake token endpoint,
        # contacts from the fake People API. IMAP then accepts exactly the access tokens that
        # endpoint issued (AUTHENTICATE XOAUTH2).
        self.people = None
        if people:
            from .people import FakePeopleServer
            self.imap.oauth_tokens = set()
            self.people = FakePeopleServer(email=email, tokens=self.imap.oauth_tokens, **(people_options or {}))

    def start(self):
        self.imap.start()
        self.port = self.imap.port
        if self.smtp:
            self.smtp.start()
        if self.people:
            self.people.start()
        return self

    def stop(self):
        self.imap.stop()
        if self.smtp:
            self.smtp.stop()
        if self.people:
            self.people.stop()

    def account_kwargs(self) -> dict:
        kw = super().account_kwargs()
        kw["email"] = self.email
        if self.smtp:
            kw["smtp_host"], kw["smtp_port"] = "127.0.0.1", self.smtp.port
        if self.people:
            kw["provider"] = "gmail"
            kw["username"] = self.email
            kw["oauth"] = self.people.account_oauth()
        return kw

    def engine_env(self) -> dict:
        return self.people.engine_env() if self.people else {}

    def sent_messages(self) -> list:
        return list(self.smtp.messages) if self.smtp else []

    def create_mailbox(self, name, special_use=None):
        self.store.create(name, [special_use] if special_use else [])

    def delete_mailbox(self, name):
        self.store.delete(name)

    def rename_mailbox(self, name, new_name):
        self.store.rename(self.store.get(name).name, new_name)

    def append(self, mailbox, raw, flags=("\\Seen",)):
        return self.store.append(mailbox, raw, flags).uid

    def _as_client(self, mailbox):
        # The Dovecot adapter performs mutations through a client session that SELECTs the
        # mailbox, which claims its \Recent messages; mirror that so both behave alike.
        self.store.get(mailbox).recent.clear()

    def expunge(self, mailbox, uids):
        self._as_client(mailbox)
        self.store.expunge(mailbox, list(uids))

    def set_flags(self, mailbox, uids, add=(), remove=(), per_message=False):
        self._as_client(mailbox)
        if add:
            self.store.store_flags(mailbox, list(uids), "add", list(add), per_message=per_message)
        if remove:
            self.store.store_flags(mailbox, list(uids), "remove", list(remove), per_message=per_message)

    def set_labels(self, mailbox, uids, add=(), remove=()):
        self._as_client(mailbox)
        if add:
            self.store.set_labels(mailbox, list(uids), "add", list(add))
        if remove:
            self.store.set_labels(mailbox, list(uids), "remove", list(remove))

    def move(self, mailbox, uids, dest):
        self._as_client(mailbox)
        self.store.move(mailbox, list(uids), dest)

    def copy(self, mailbox, uids, dest):
        self._as_client(mailbox)
        self.store.copy(mailbox, list(uids), dest)

    def set_uidvalidity(self, mailbox, value):
        self.store.set_uidvalidity(mailbox, value)

    def set_uidnext(self, mailbox, value):
        self.store.set_uidnext(mailbox, value)

    def duplicate(self, mailbox, uids, dest):
        self.store.duplicate(mailbox, list(uids), dest)

    def set_keyword_limit(self, mailbox, limit):
        """At most `limit` keywords in `mailbox` (None: no limit). A STORE past it answers
        NO [LIMIT], and PERMANENTFLAGS loses \\* once the mailbox is full, as on Cyrus."""
        self.store.get(mailbox).keyword_limit = limit

    def set_drop_keywords(self, on=True):
        """STOREs answer OK and keep no keyword."""
        self.store.drop_keywords = on

    def drop_connections(self):
        for s in list(self.imap.sessions):
            if not s.closed:
                try:
                    s.connection.shutdown(2)
                    s.connection.close()
                except OSError:
                    pass

    def truth(self):
        # The store is the truth; reading it back over IMAP is what the conformance suite
        # does to prove the two agree.
        t = self.store.truth()
        from .base import normalize_message_id
        return {mb: {uid: {**v, "message_id": normalize_message_id(v["message_id"])} for uid, v in uids.items()}
                for mb, uids in t.items()}

    # fake-only scripting
    def at(self, hook: str, fn, once: bool = True, session: Optional[str] = None,
           command: Optional[str] = None, mailbox: Optional[str] = None):
        """Run fn(session) at a protocol moment: idle_start, idle_tick, before_fetch_body,
        before_command, after_command. Filters narrow which occasion counts: `session` is
        "foreground" (a connection that has idled - the engine's IDLE worker) or "background"
        (one that never has), `command` a regex on the command line (e.g. "UID FETCH 1:\\*")
        for before/after_command, `mailbox` the session's selected mailbox."""
        rx = re.compile(command) if command else None

        def wrapped(sess, a, b):
            if session == "foreground" and not sess.has_idled:
                return False
            if session == "background" and sess.has_idled:
                return False
            if mailbox is not None and sess.selected != mailbox:
                return False
            if rx is not None:
                line = a if isinstance(a, str) else ""
                if isinstance(b, bytes):
                    line += " " + b.decode("utf-8", "replace").strip()
                if not rx.search(line):
                    return False
            fn(sess)
            return True
        self.imap.add_hook(hook, wrapped, once=once)
