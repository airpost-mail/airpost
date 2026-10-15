"""
CalDAV/CardDAV servers in Docker for the DAV scenarios: Radicale 3 (`servers/radicale/`) and
Baikal 0.10 on sabre/dav 4 (`servers/baikal/`). The mail side of these server kinds is the
in-process fake (Dovecot personality unless the entry says `imap: <personality>`), so a DAV
scenario reads `servers: [{radicale: default}, {baikal: default}]` and every IMAP step works
as on `fake:dovecot`.

The engine finds the DAV service the way the C++ DAVWorker does once it has a host name
(resolveAddressBook / resolveCalendarHomeURL): `.well-known/carddav|caldav` on https, then
http, then current-user-principal, then the home set, then the collections. The host name
itself came from `/api/resolve-dav-hosts` in C++ and comes from a DNS SRV lookup in the Rust
engine (docs/engine-port-plan.md section 2), neither of which can point at 127.0.0.1:<port>,
so the harness passes it in MAILSYNC_CALDAV_HOST / MAILSYNC_CARDDAV_HOST (host[:port], the
same shape as the resolve-dav-hosts answer) and the rest of discovery runs for real.
Authentication is Basic with the account's IMAP credentials (DAVWorker
getAuthorizationHeader), so both images carry the harness user test / pass.

Each image is seeded with one calendar and one address book, which scenarios call
`default`; population and changes by "another client" go over CalDAV/CardDAV from
DavClient, exactly what a phone or webmail would send:

  radicale  calendar /test/calendar/, address book /test/contacts/ (filesystem storage,
            sync-collection tokens, ctag, 412 on a stale If-Match)
  baikal    calendar /dav.php/calendars/test/default/, address book
            /dav.php/addressbooks/test/default/ (sabre/dav: sync tokens
            http://sabre.io/ns/sync/N, ctag = sync token, schedule inbox/outbox in the
            calendar home without a VEVENT component set)
"""
import base64
import hashlib
import http.client
import os
import re
import socket
import subprocess
import time
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Optional

from .fake import FakeServer

HERE = Path(__file__).resolve().parent
SERVERS_DIR = HERE.parents[1] / "servers"

NS = {"d": "DAV:", "c": "urn:ietf:params:xml:ns:caldav", "card": "urn:ietf:params:xml:ns:carddav",
      "cs": "http://calendarserver.org/ns/", "ical": "http://apple.com/ns/ical/"}

KINDS = {
    "radicale": dict(dir="radicale", version="3.8.3", container_port=5232, port_env=None,
                     calendar="/test/calendar/", addressbook="/test/contacts/", ready_path="/"),
    "baikal": dict(dir="baikal", version="0.10.1", container_port=None, port_env="BAIKAL_PORT",
                   calendar="/dav.php/calendars/test/default/", addressbook="/dav.php/addressbooks/test/default/",
                   ready_path="/dav.php/"),
}
PROFILES = {"default": {}}


def image_tag(kind: str) -> str:
    """Tagged by the build context's content (seed files included) so a change rebuilds it."""
    env = os.environ.get(f"HARNESS_{kind.upper()}_IMAGE")
    if env:
        return env
    root = SERVERS_DIR / KINDS[kind]["dir"]
    h = hashlib.sha1()
    for p in sorted(x for x in root.rglob("*") if x.is_file()):
        h.update(str(p.relative_to(root)).encode() + p.read_bytes())
    return f"mailsync-harness-{kind}:{KINDS[kind]['version']}-{h.hexdigest()[:8]}"


def ensure_image(kind: str) -> str:
    tag = image_tag(kind)
    if os.environ.get(f"HARNESS_{kind.upper()}_IMAGE"):
        return tag
    if subprocess.run(["docker", "image", "inspect", tag], capture_output=True).returncode != 0:
        r = subprocess.run(["docker", "build", "-q", "-t", tag, str(SERVERS_DIR / KINDS[kind]["dir"])],
                           capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(f"docker build of {kind} failed: {r.stderr[-2000:]}")
    return tag


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class DavError(RuntimeError):
    def __init__(self, method: str, url: str, status: int, body: str):
        super().__init__(f"{method} {url} -> {status}: {body[:300]}")
        self.status = status
        self.body = body


def _text(el, path: str) -> str:
    found = el.find(path, NS)
    return (found.text or "").strip() if found is not None and found.text else ""


class DavClient:
    """Just enough WebDAV for another client's changes and for reading the server's state
    back. Every request is logged into `requests` for the self-test."""

    def __init__(self, host: str, port: int, username: str, password: str):
        self.host, self.port = host, port
        self.auth = "Basic " + base64.b64encode(f"{username}:{password}".encode()).decode()
        self.requests: list = []

    def request(self, method: str, path: str, body: Optional[str] = None, headers: Optional[dict] = None,
                auth: bool = True, expect: tuple = (200, 201, 204, 207)) -> tuple:
        h = {"User-Agent": "mailsync-harness"}
        if auth:
            h["Authorization"] = self.auth
        h.update(headers or {})
        data = body.encode() if isinstance(body, str) else body
        conn = http.client.HTTPConnection(self.host, self.port, timeout=30)
        try:
            conn.request(method, path, body=data, headers=h)
            resp = conn.getresponse()
            text = resp.read().decode("utf-8", "replace")
            status, rheaders = resp.status, {k.lower(): v for k, v in resp.getheaders()}
        finally:
            conn.close()
        self.requests.append((method, path, status))
        if expect and status not in expect:
            raise DavError(method, path, status, text)
        return status, rheaders, text

    # -- discovery, as DAVWorker::resolveAddressBook / resolveCalendarHomeURL --------------------

    def well_known(self, service: str) -> str:
        status, headers, _ = self.request("GET", f"/.well-known/{service}", auth=False, expect=())
        if status not in (301, 302, 303, 307, 308):
            raise DavError("GET", f"/.well-known/{service}", status, "expected a redirect (RFC 6764 section 5)")
        return urllib.parse.urlsplit(headers["location"]).path or "/"

    def propfind(self, path: str, props: str, depth: str = "0") -> ET.Element:
        body = f'<?xml version="1.0" encoding="UTF-8"?><d:propfind xmlns:d="DAV:" xmlns:c="{NS["c"]}" ' \
               f'xmlns:card="{NS["card"]}" xmlns:cs="{NS["cs"]}" xmlns:ical="{NS["ical"]}"><d:prop>{props}</d:prop></d:propfind>'
        _, _, text = self.request("PROPFIND", path, body, {"Depth": depth, "Content-Type": "application/xml; charset=utf-8"})
        return ET.fromstring(text)

    def principal(self, root: str) -> str:
        doc = self.propfind(root, "<d:current-user-principal/>")
        return _text(doc, ".//d:current-user-principal/d:href")

    def home_set(self, principal: str, service: str) -> str:
        prop = "c:calendar-home-set" if service == "caldav" else "card:addressbook-home-set"
        doc = self.propfind(principal, f"<{prop}/>")
        return _text(doc, f".//{prop}/d:href")

    def collections(self, home: str) -> list:
        """Children of a home set: [{href, name, ctag, sync_token, calendar, addressbook, components}]."""
        doc = self.propfind(home, "<d:resourcetype/><d:displayname/><cs:getctag/><d:sync-token/>"
                                  "<c:supported-calendar-component-set/>", depth="1")
        out = []
        for resp in doc.findall("d:response", NS):
            href = _text(resp, "d:href")
            if href.rstrip("/") == home.rstrip("/"):
                continue
            rt = resp.find(".//d:resourcetype", NS)
            comps = [c.get("name") for c in resp.findall(".//c:supported-calendar-component-set/c:comp", NS)]
            out.append({"href": href, "name": _text(resp, ".//d:displayname"), "ctag": _text(resp, ".//cs:getctag"),
                        "sync_token": _text(resp, ".//d:sync-token"),
                        "calendar": rt is not None and rt.find("c:calendar", NS) is not None,
                        "addressbook": rt is not None and rt.find("card:addressbook", NS) is not None,
                        "components": comps})
        return out

    # -- items --------------------------------------------------------------------------------------

    def put(self, href: str, text: str, content_type: str, if_match: Optional[str] = None,
            create: bool = False) -> str:
        headers = {"Content-Type": f"{content_type}; charset=utf-8"}
        if if_match:
            headers["If-Match"] = if_match
        if create:
            headers["If-None-Match"] = "*"
        _, rh, _ = self.request("PUT", href, text, headers, expect=(200, 201, 204))
        return rh.get("etag") or self.etag(href)

    def delete(self, href: str, if_match: Optional[str] = None):
        self.request("DELETE", href, None, {"If-Match": if_match} if if_match else None, expect=(200, 204))

    def get(self, href: str) -> tuple:
        status, rh, text = self.request("GET", href, expect=(200,))
        return rh.get("etag", ""), text

    def etag(self, href: str) -> str:
        doc = self.propfind(href, "<d:getetag/>")
        return _text(doc, ".//d:getetag")

    def items(self, collection: str, calendar: bool) -> dict:
        """{href: {"etag", "data"}} for every resource in a collection, from a query REPORT with
        no filter beyond the component (calendars: VCALENDAR, so events outside the engine's
        sync window are listed too)."""
        if calendar:
            body = ('<?xml version="1.0" encoding="UTF-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">'
                    '<d:prop><d:getetag/><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"/></c:filter>'
                    '</c:calendar-query>')
            data_path = ".//c:calendar-data"
        else:
            body = ('<?xml version="1.0" encoding="UTF-8"?><card:addressbook-query xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">'
                    '<d:prop><d:getetag/><card:address-data/></d:prop></card:addressbook-query>')
            data_path = ".//card:address-data"
        _, _, text = self.request("REPORT", collection, body, {"Depth": "1", "Content-Type": "application/xml; charset=utf-8"})
        out = {}
        for resp in ET.fromstring(text).findall("d:response", NS):
            href = urllib.parse.unquote(_text(resp, "d:href"))
            if href.rstrip("/") == collection.rstrip("/"):
                continue
            out[href] = {"etag": _text(resp, ".//d:getetag"), "data": _text(resp, data_path)}
        return out

    def mkcalendar(self, href: str, name: str, components=("VEVENT",)):
        comps = "".join(f'<c:comp name="{c}"/>' for c in components)
        body = ('<?xml version="1.0" encoding="UTF-8"?><c:mkcalendar xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">'
                f'<d:set><d:prop><d:displayname>{name}</d:displayname><c:supported-calendar-component-set>{comps}'
                '</c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>')
        self.request("MKCALENDAR", href, body, {"Content-Type": "application/xml; charset=utf-8"}, expect=(201,))

    def remove_collection(self, href: str):
        self.request("DELETE", href, expect=(200, 204))


class DavServer(FakeServer):
    """A DAV container plus the in-process fake IMAP server."""

    def __init__(self, kind: str, profile: str = "default", log_path: Optional[str] = None,
                 imap: str = "dovecot", **kw):
        if kind not in KINDS:
            raise ValueError(f"unknown DAV server kind {kind}; known: {sorted(KINDS)}")
        if profile not in PROFILES:
            raise ValueError(f"unknown {kind} profile {profile}; known: {sorted(PROFILES)}")
        super().__init__(imap, log_path=log_path, **kw)
        self.kind = kind
        self.profile = profile
        self.cfg = KINDS[kind]
        self.dav_port = 0
        self.container: Optional[str] = None
        self.dav_log_path = str(Path(log_path).with_name("dav.log")) if log_path else None
        self.dav: Optional[DavClient] = None
        self.calendar_home = ""
        self.addressbook_home = ""

    # -- lifecycle ----------------------------------------------------------------------------------

    def start(self):
        super().start()
        try:
            self._start_container()
        except Exception:
            super().stop()
            raise
        return self

    def _start_container(self):
        tag = ensure_image(self.kind)
        self.dav_port = _free_port()
        name = f"harness-{self.kind}-{os.getpid()}-{int(time.time() * 1000) % 100000}"
        inner = self.cfg["container_port"] or self.dav_port
        args = ["docker", "run", "-d", "--name", name, "-p", f"127.0.0.1:{self.dav_port}:{inner}"]
        if self.cfg["port_env"]:
            args += ["-e", f"{self.cfg['port_env']}={self.dav_port}"]
        subprocess.run(args + [tag], check=True, capture_output=True)
        self.container = name
        self.dav = DavClient("127.0.0.1", self.dav_port, self.username, self.password)
        try:
            self._wait_ready()
            self.calendar_home = self.dav.home_set(self.dav.principal(self.dav.well_known("caldav")), "caldav")
            self.addressbook_home = self.dav.home_set(self.dav.principal(self.dav.well_known("carddav")), "carddav")
        except Exception:
            self._stop_container()
            raise

    def _wait_ready(self, timeout: float = 60):
        deadline = time.time() + timeout
        last = None
        while time.time() < deadline:
            try:
                status, _, _ = self.dav.request("PROPFIND", self.cfg["ready_path"], "", {"Depth": "0"}, expect=())
                if status in (207, 401):
                    return
                last = f"HTTP {status}"
            except OSError as e:
                last = e
            time.sleep(0.3)
        raise RuntimeError(f"{self.kind} did not become ready on {self.dav_port}: {last}\n{self.logs()[-2000:]}")

    def stop(self):
        self._stop_container()
        super().stop()

    def _stop_container(self):
        if not self.container:
            return
        if self.dav_log_path:
            try:
                Path(self.dav_log_path).write_text(self.logs())
            except Exception as e:
                Path(self.dav_log_path).write_text(f"(could not collect the {self.kind} log: {e})\n")
        subprocess.run(["docker", "rm", "-f", self.container], capture_output=True)
        self.container = None

    def logs(self) -> str:
        if not self.container:
            return ""
        r = subprocess.run(["docker", "logs", self.container], capture_output=True, text=True)
        return r.stdout + r.stderr

    def engine_env(self) -> dict:
        host = f"127.0.0.1:{self.dav_port}"
        return {**super().engine_env(), "MAILSYNC_CALDAV_HOST": host, "MAILSYNC_CARDDAV_HOST": host}

    # -- names --------------------------------------------------------------------------------------

    def calendar_href(self, name: str) -> str:
        """`default` is the seeded calendar; any other name is a collection in the home set."""
        if name == "default":
            return self.cfg["calendar"]
        return name if name.startswith("/") else self.calendar_home.rstrip("/") + "/" + name + "/"

    def addressbook_href(self, name: str) -> str:
        if name == "default":
            return self.cfg["addressbook"]
        return name if name.startswith("/") else self.addressbook_home.rstrip("/") + "/" + name + "/"

    @staticmethod
    def item_href(collection: str, uid: str, ext: str) -> str:
        safe = re.sub(r"[^A-Za-z0-9@._-]", "-", uid)
        return collection + urllib.parse.quote(safe) + ext

    # -- what another client does ---------------------------------------------------------------------

    def put_event(self, calendar: str, uid: str, ics: str, href: Optional[str] = None, if_match: Optional[str] = None) -> dict:
        """Creates (no `if_match`: If-None-Match *) or replaces a calendar resource."""
        col = self.calendar_href(calendar)
        href = href or self.item_href(col, uid, ".ics")
        etag = self.dav.put(href, ics, "text/calendar", if_match=if_match, create=not if_match)
        return {"href": href, "etag": etag, "uid": uid}

    def put_card(self, addressbook: str, uid: str, vcf: str, href: Optional[str] = None, if_match: Optional[str] = None) -> dict:
        col = self.addressbook_href(addressbook)
        href = href or self.item_href(col, uid, ".vcf")
        etag = self.dav.put(href, vcf, "text/vcard", if_match=if_match, create=not if_match)
        return {"href": href, "etag": etag, "uid": uid}

    def find_event(self, uid: str, calendar: Optional[str] = None) -> Optional[dict]:
        """{href, etag, data, calendar} of the resource holding a UID."""
        from .. import davgen
        cals = [self.calendar_href(calendar)] if calendar else [c["href"] for c in self.calendars()]
        for col in cals:
            for href, item in self.dav.items(col, calendar=True).items():
                if any(v["uid"] == uid for v in davgen.vevents(item["data"])):
                    return {**item, "href": href, "calendar": col}
        return None

    def find_card(self, uid: str, addressbook: str = "default") -> Optional[dict]:
        from .. import davgen
        col = self.addressbook_href(addressbook)
        for href, item in self.dav.items(col, calendar=False).items():
            if davgen.vcard_fields(item["data"])["uid"] == uid:
                return {**item, "href": href, "addressbook": col}
        return None

    def calendars(self) -> list:
        """The calendar collections the engine is meant to sync: resourcetype calendar with
        VEVENT in the component set (DAVWorker::runCalendars)."""
        return [c for c in self.dav.collections(self.calendar_home) if c["calendar"] and "VEVENT" in c["components"]]

    def addressbooks(self) -> list:
        return [c for c in self.dav.collections(self.addressbook_home) if c["addressbook"]]

    def dav_truth(self) -> dict:
        return {
            "calendars": {c["href"]: {**c, "items": self.dav.items(c["href"], calendar=True)} for c in self.calendars()},
            "addressbooks": {a["href"]: {**a, "items": self.dav.items(a["href"], calendar=False)} for a in self.addressbooks()},
        }
