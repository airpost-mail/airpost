"""
Self-test of the services the PIM scenarios run against, with no engine involved: the fake
People API (harness/servers/people.py) and the Radicale and Baikal images
(harness/servers/dav.py). Each test sends the requests GoogleContactsWorker and DAVWorker
send, in their shapes, and checks the answers the scenarios depend on.

    python3 -m pytest apps/engine/test/test_service_fakes.py -n 0
"""
import http.client
import json
import re
import urllib.parse
import xml.etree.ElementTree as ET

import pytest

from harness import davgen
from harness.servers import dav as davmod
from harness.servers.people import CLIENT_ID, EXPIRED_TOKEN_MESSAGE, REFRESH_TOKEN, FakePeopleServer

# GoogleContactsWorker.cpp PERSON_FIELDS.
PERSON_FIELDS = ("emailAddresses,genders,names,nicknames,phoneNumbers,urls,birthdays,addresses,userDefined,"
                 "relations,occupations,organizations,photos,biographies")


# -- People API --------------------------------------------------------------------------------------

class PeopleClient:
    def __init__(self, server: FakePeopleServer):
        self.port = server.port
        self.token = ""

    def request(self, method: str, path: str, body=None, form=None, auth=True) -> tuple:
        headers = {}
        data = None
        if form is not None:
            data = urllib.parse.urlencode(form)
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        elif body is not None:
            data = json.dumps(body)
            headers["Content-Type"] = "application/json"
        if auth and self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        try:
            conn.request(method, path, body=data, headers=headers)
            resp = conn.getresponse()
            return resp.status, json.loads(resp.read() or b"{}")
        finally:
            conn.close()

    def refresh(self, refresh_token: str = REFRESH_TOKEN) -> dict:
        status, out = self.request("POST", "/token", form={"grant_type": "refresh_token", "client_id": CLIENT_ID,
                                                          "refresh_token": refresh_token}, auth=False)
        assert status == 200, out
        self.token = out["access_token"]
        return out

    def list_all(self, path: str, token_key: str, sync_token: str = "", extra: str = "") -> tuple:
        """paginateGoogleCollection: follow nextPageToken, return (items, nextSyncToken, pages)."""
        items, pages, page_token = [], 0, ""
        while True:
            url = path + extra
            if sync_token:
                url += f"&syncToken={sync_token}"
            if page_token:
                url += f"&pageToken={page_token}"
            status, out = self.request("GET", url)
            assert status == 200, out
            pages += 1
            items += out.get(token_key, [])
            page_token = out.get("nextPageToken", "")
            if not page_token:
                return items, out.get("nextSyncToken", ""), pages


@pytest.fixture
def people():
    server = FakePeopleServer(rotate_refresh_token=True).start()
    try:
        yield server
    finally:
        server.stop()


CONNECTIONS = f"/v1/people/me/connections?personFields={PERSON_FIELDS}&pageSize=400"


def test_people_oauth_and_auth(people):
    c = PeopleClient(people)
    status, out = c.request("GET", CONNECTIONS + "&requestSyncToken=true")
    assert status == 401 and out["error"]["status"] == "UNAUTHENTICATED"
    first = c.refresh()
    assert first["refresh_token"].startswith("1//harness-refresh-token-rotated"), "the first grant rotates the refresh token"
    assert isinstance(first["expires_in"], str)
    status, out = c.request("POST", "/token", form={"grant_type": "refresh_token", "client_id": CLIENT_ID,
                                                    "refresh_token": REFRESH_TOKEN}, auth=False)
    assert status == 400 and out["error"] == "invalid_grant", "the rotated-out refresh token is revoked"
    c.refresh(first["refresh_token"])
    assert c.token in people.access_tokens


def test_people_paging_sync_tokens_and_tombstones(people):
    s = people.state
    climbing = s.create_group("Climbing")
    for i in range(450):
        s.create_person(name=f"Person {i}", email=f"p{i}@example.com", groups=["Climbing"] if i < 3 else [])
    c = PeopleClient(people)
    c.refresh()

    persons, sync, pages = c.list_all(CONNECTIONS, "connections", extra="&requestSyncToken=true")
    assert pages == 2 and len(persons) == 450 and sync
    assert {p["resourceName"] for p in persons} == {p.resource_name for p in s.live_people()}
    assert all("emailAddresses" in p and "names" in p for p in persons)
    status, out = c.request("GET", "/v1/people/me/connections?pageSize=10")
    assert status == 400 and "personFields" in out["error"]["message"]

    groups, gsync, _ = c.list_all("/v1/contactGroups?pageSize=400", "contactGroups")
    user = [g for g in groups if g["groupType"] == "USER_CONTACT_GROUP"]
    assert [g["name"] for g in user] == ["Climbing"] and gsync
    assert any(g["groupType"] == "SYSTEM_CONTACT_GROUP" for g in groups)
    status, g = c.request("GET", f"/v1/{climbing.resource_name}?maxMembers={user[0]['memberCount']}")
    assert status == 200 and len(g["memberResourceNames"]) == 3

    # Incremental: one added, one changed, one deleted; nothing else.
    added = s.create_person(name="New Person", email="new@example.com")
    changed = s.person_by_email("p10@example.com")
    s.update_person(changed, name="Renamed Person")
    gone = s.person_by_email("p11@example.com")
    s.delete_person(gone)
    delta, sync2, _ = c.list_all(CONNECTIONS, "connections", sync_token=sync, extra="&requestSyncToken=true")
    by_rn = {p["resourceName"]: p for p in delta}
    assert set(by_rn) == {added.resource_name, changed.resource_name, gone.resource_name}
    assert by_rn[gone.resource_name]["metadata"]["deleted"] is True
    assert by_rn[changed.resource_name]["names"][0]["displayName"] == "Renamed Person"
    again, _, _ = c.list_all(CONNECTIONS, "connections", sync_token=sync2, extra="&requestSyncToken=true")
    assert again == []

    s.rename_group(climbing, "Bouldering")
    gdelta, _, _ = c.list_all("/v1/contactGroups?pageSize=400", "contactGroups", sync_token=gsync)
    assert [g["name"] for g in gdelta] == ["Bouldering"]

    s.expire_sync_tokens()
    status, out = c.request("GET", CONNECTIONS + f"&requestSyncToken=true&syncToken={sync2}")
    assert status == 400 and out["error"]["message"] == EXPIRED_TOKEN_MESSAGE


def test_people_writes(people):
    s = people.state
    alice = s.create_person(name="Alice Archer", email="alice@example.org")
    c = PeopleClient(people)
    c.refresh()

    status, created = c.request("POST", "/v1/people:createContact",
                                {"names": [{"givenName": "Dan", "familyName": "Diver", "displayName": "Dan Diver"}],
                                 "emailAddresses": [{"value": "dan@example.org"}]})
    assert status == 200 and created["resourceName"].startswith("people/")
    assert s.person_by_email("dan@example.org").name == "Dan Diver"

    # updateContact with the etag the listing returned (GoogleContactsWorker stores the person).
    status, listed = c.request("GET", f"/v1/{alice.resource_name}?personFields=names,emailAddresses,metadata")
    stale = dict(listed)
    listed["names"] = [{"displayName": "Alice Archer-Smith", "givenName": "Alice", "familyName": "Archer-Smith"}]
    status, out = c.request("PATCH", f"/v1/{alice.resource_name}:updateContact?updatePersonFields=names,emailAddresses",
                            listed)
    assert status == 200, out
    assert s.person_by_email("alice@example.org").name == "Alice Archer-Smith"
    status, out = c.request("PATCH", f"/v1/{alice.resource_name}:updateContact?updatePersonFields=names", stale)
    assert status == 400 and out["error"]["status"] == "FAILED_PRECONDITION", "a stale etag is refused"
    status, out = c.request("PATCH", f"/v1/{alice.resource_name}:updateContact", listed)
    assert status == 400, "updatePersonFields is required"

    status, group = c.request("POST", "/v1/contactGroups", {"contactGroup": {"name": "Book Club"}})
    assert status == 200 and group["groupType"] == "USER_CONTACT_GROUP"
    status, _ = c.request("POST", f"/v1/{group['resourceName']}/members:modify",
                          {"resourceNamesToAdd": [alice.resource_name, created["resourceName"]]})
    assert status == 200
    assert len(s.members(s.group_by_name("Book Club"))) == 2
    status, renamed = c.request("PUT", f"/v1/{group['resourceName']}",
                                {"contactGroup": {"name": "Readers", "etag": s.group_by_name("Book Club").etag}})
    assert status == 200 and renamed["name"] == "Readers"
    status, _ = c.request("DELETE", f"/v1/{group['resourceName']}")
    assert status == 200 and not [g for g in s.live_groups(user_only=True)]
    assert not s.person_by_email("alice@example.org").memberships - {"myContacts"}

    status, _ = c.request("DELETE", f"/v1/{created['resourceName']}:deleteContact")
    assert status == 200
    with pytest.raises(KeyError):
        s.person_by_email("dan@example.org")
    assert [r.method for r in people.requests_matching(path=r":deleteContact$")] == ["DELETE"]


# -- DAV servers ----------------------------------------------------------------------------------------

NS = davmod.NS


@pytest.fixture(scope="module", params=[pytest.param(k, marks=[pytest.mark.slow, pytest.mark.docker(k)])
                                        for k in sorted(davmod.KINDS)])
def dav_server(request, tmp_path_factory):
    log = tmp_path_factory.mktemp(request.param) / "server.log"
    server = davmod.DavServer(request.param, log_path=str(log))
    server.start()
    try:
        yield server
    finally:
        server.stop()


def xml(text: str) -> ET.Element:
    return ET.fromstring(text)


def responses(doc: ET.Element) -> dict:
    """href (percent-decoded) -> {"status": str, "etag": str, "data": str} from a multistatus."""
    out = {}
    for r in doc.findall("d:response", NS):
        href = urllib.parse.unquote(r.findtext("d:href", "", NS))
        etag = r.findtext(".//d:getetag", "", NS)
        data = r.findtext(".//c:calendar-data", "", NS) or r.findtext(".//card:address-data", "", NS)
        status = r.findtext("d:status", "", NS) or r.findtext(".//d:status", "", NS)
        out[href] = {"status": status, "etag": etag, "data": data}
    return out


def raw(server, method, path, body=None, headers=None, auth=True):
    return server.dav.request(method, path, body, headers, auth=auth, expect=())


def test_dav_discovery_as_davworker(dav_server):
    """resolveAddressBook / resolveCalendarHomeURL, request by request."""
    s = dav_server
    for service, home_prop in (("caldav", "c:calendar-home-set"), ("carddav", "card:addressbook-home-set")):
        status, headers, _ = raw(s, "GET", f"/.well-known/{service}", auth=False)
        assert status in (301, 302, 303, 307, 308), status
        root = urllib.parse.urlsplit(headers["location"]).path or "/"
        assert urllib.parse.urlsplit(headers["location"]).port in (None, s.dav_port), headers["location"]
        status, _, text = raw(s, "PROPFIND", root,
                              '<?xml version="1.0" encoding="UTF-8"?><A:propfind xmlns:A="DAV:"><A:prop>'
                              '<A:current-user-principal/><A:principal-URL/><A:resourcetype/></A:prop></A:propfind>',
                              {"Depth": "0", "Content-Type": "application/xml"})
        assert status == 207
        principal = xml(text).findtext(".//d:current-user-principal/d:href", "", NS)
        assert principal
        ns = NS["c"] if service == "caldav" else NS["card"]
        tag = home_prop.split(":")[1]
        status, _, text = raw(s, "PROPFIND", principal,
                              f'<?xml version="1.0" encoding="UTF-8"?><d:propfind xmlns:d="DAV:" xmlns:x="{ns}">'
                              f'<d:prop><x:{tag}/></d:prop></d:propfind>', {"Depth": "0", "Content-Type": "application/xml"})
        assert status == 207
        home = xml(text).findtext(f".//{home_prop}/d:href", "", NS)
        assert home in (s.calendar_home, s.addressbook_home), home

    status, _, text = raw(s, "PROPFIND", s.calendar_home,
                          '<?xml version="1.0" encoding="UTF-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" '
                          'xmlns:cs="http://calendarserver.org/ns/" xmlns:ical="http://apple.com/ns/ical/"><d:prop>'
                          '<d:resourcetype/><d:displayname/><cs:getctag/><d:sync-token/><c:supported-calendar-component-set/>'
                          '<ical:calendar-color/><c:calendar-description/><d:current-user-privilege-set/><d:owner/>'
                          '<ical:calendar-order/></d:prop></d:propfind>', {"Depth": "1", "Content-Type": "application/xml"})
    assert status == 207
    cals = s.calendars()
    assert [c["href"].rstrip("/") for c in cals] == [s.cfg["calendar"].rstrip("/")]
    assert cals[0]["ctag"] and cals[0]["sync_token"]
    books = s.addressbooks()
    assert [b["href"].rstrip("/") for b in books] == [s.cfg["addressbook"].rstrip("/")]
    assert books[0]["ctag"]
    status, _, _ = raw(s, "PROPFIND", s.cfg["calendar"], "", {"Depth": "0"}, auth=False)
    assert status == 401, "Basic auth with the IMAP credentials is required"


def _sync_collection(s, collection, token, props):
    body = ('<?xml version="1.0" encoding="utf-8"?><D:sync-collection xmlns:D="DAV:" '
            'xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:R="urn:ietf:params:xml:ns:carddav">'
            + (f"<D:sync-token>{token}</D:sync-token>" if token else "<D:sync-token/>")
            + f"<D:sync-level>1</D:sync-level><D:prop>{props}</D:prop></D:sync-collection>")
    status, _, text = raw(s, "REPORT", collection, body, {"Depth": "0", "Content-Type": "application/xml"})
    assert status == 207, text[:500]
    doc = xml(text)
    return responses(doc), doc.findtext("d:sync-token", "", NS)


def test_caldav_sync_and_writes(dav_server):
    s = dav_server
    col = s.cfg["calendar"]
    ctag0 = s.calendars()[0]["ctag"]
    _, token = _sync_collection(s, col, "", "<D:getetag/>")
    assert token

    uid_in, ics_in = davgen.event(901, {"summary": "In window", "start": "+3d"})
    uid_out, ics_out = davgen.event(902, {"summary": "Outside", "start": "+800d"})
    put_in = s.put_event("default", uid_in, ics_in)
    s.put_event("default", uid_out, ics_out)
    assert put_in["etag"]
    status, _, _ = raw(s, "PUT", put_in["href"], ics_in, {"Content-Type": "text/calendar", "If-None-Match": "*"})
    assert status == 412, "If-None-Match * on an existing resource"
    assert s.calendars()[0]["ctag"] != ctag0, "the ctag changes with the collection"

    changes, token2 = _sync_collection(s, col, token, "<D:getetag/><C:calendar-data/>")
    assert {urllib.parse.unquote(davmod.DavServer.item_href(col, u, ".ics")) for u in (uid_in, uid_out)} <= set(changes)
    assert token2 and token2 != token

    # The etag listing DAVWorker falls back to: calendar-query with the sync window.
    start = davgen.when("-365d").strftime("%Y%m%dT%H%M%SZ")
    end = davgen.when("+548d").strftime("%Y%m%dT%H%M%SZ")
    status, _, text = raw(s, "REPORT", col,
                          '<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag /></d:prop>'
                          '<c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">'
                          f'<c:time-range start="{start}" end="{end}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>',
                          {"Depth": "1", "Content-Type": "application/xml"})
    assert status == 207
    listed = responses(xml(text))
    assert urllib.parse.unquote(put_in["href"]) in listed and not any(uid_out.split("@")[0] in h for h in listed)

    status, _, text = raw(s, "REPORT", col,
                          '<c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag />'
                          f'<c:calendar-data /></d:prop><d:href>{put_in["href"]}</d:href></c:calendar-multiget>',
                          {"Depth": "1", "Content-Type": "application/xml"})
    got = responses(xml(text))[urllib.parse.unquote(put_in["href"])]
    assert davgen.vevents(got["data"])[0]["uid"] == uid_in and bare_etag(got["etag"]) == bare_etag(put_in["etag"])

    # Another client edits it; the old etag is now stale for both PUT and DELETE.
    edited = davgen.with_props(ics_in, "VEVENT", {"SUMMARY": "Edited elsewhere", "SEQUENCE": "1"})
    new_etag = s.dav.put(put_in["href"], edited, "text/calendar", if_match=put_in["etag"])
    status, _, _ = raw(s, "PUT", put_in["href"], ics_in, {"Content-Type": "text/calendar", "If-Match": put_in["etag"]})
    assert status == 412
    status, _, _ = raw(s, "DELETE", put_in["href"], None, {"If-Match": put_in["etag"]})
    assert status == 412
    assert s.find_event(uid_in)["etag"] and davgen.vevents(s.find_event(uid_in)["data"])[0]["summary"] == "Edited elsewhere"

    s.dav.delete(put_in["href"], if_match=new_etag)
    changes, _ = _sync_collection(s, col, token2, "<D:getetag/>")
    assert "404" in changes[urllib.parse.unquote(put_in["href"])]["status"], "a deletion is reported as a 404 response"

    s.dav.mkcalendar(s.calendar_href("selftest"), "Self test")
    assert any(c["name"] == "Self test" for c in s.calendars())
    s.dav.remove_collection(s.calendar_href("selftest"))
    assert not any(c["name"] == "Self test" for c in s.calendars())


def bare_etag(e: str) -> str:
    return (e or "").strip().removeprefix("W/").strip('"')


def test_carddav_sync_groups_and_writes(dav_server):
    s = dav_server
    col = s.cfg["addressbook"]
    _, token = _sync_collection(s, col, "", "<D:getetag/>")
    alice_uid, alice = davgen.contact(901, {"name": "Alice Archer", "email": "alice@example.org"})
    bob_uid, bob = davgen.contact(902, {"name": "Bob Baker", "email": "bob@example.org", "version": "4.0"})
    a = s.put_card("default", alice_uid, alice)
    s.put_card("default", bob_uid, bob)
    gid, group = davgen.group(901, {"name": "Climbing", "members": [alice_uid, bob_uid]})
    s.put_card("default", gid, group)

    changes, token2 = _sync_collection(s, col, token, "<D:getetag/><R:address-data/>")
    cards = {davgen.vcard_fields(v["data"])["uid"]: davgen.vcard_fields(v["data"]) for v in changes.values() if v["data"]}
    assert set(cards) == {alice_uid, bob_uid, gid}
    assert cards[gid]["is_group"] and sorted(cards[gid]["members"]) == sorted([alice_uid, bob_uid])

    status, _, text = raw(s, "REPORT", col,
                          '<c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag />'
                          '</d:prop></c:addressbook-query>', {"Depth": "1", "Content-Type": "application/xml"})
    assert status == 207 and urllib.parse.unquote(a["href"]) in responses(xml(text))
    status, _, text = raw(s, "REPORT", col,
                          '<c:addressbook-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag />'
                          f'<c:address-data /></d:prop><d:href>{a["href"]}</d:href></c:addressbook-multiget>',
                          {"Depth": "1", "Content-Type": "application/xml"})
    assert davgen.vcard_fields(responses(xml(text))[urllib.parse.unquote(a["href"])]["data"])["fn"] == "Alice Archer"

    edited = davgen.with_props(alice, "VCARD", {"FN": "Alice from the phone"})
    s.dav.put(a["href"], edited, "text/vcard", if_match=a["etag"])
    status, _, _ = raw(s, "PUT", a["href"], alice, {"Content-Type": "text/vcard", "If-Match": a["etag"]})
    assert status == 412
    status, _, _ = raw(s, "DELETE", a["href"], None, {"If-Match": a["etag"]})
    assert status == 412
    card = s.find_card(alice_uid)
    s.dav.delete(card["href"], if_match=card["etag"])
    changes, _ = _sync_collection(s, col, token2, "<D:getetag/>")
    assert "404" in changes[urllib.parse.unquote(a["href"])]["status"]
    truth = s.dav_truth()
    assert set(truth["addressbooks"]) and set(truth["calendars"])


def test_davgen_round_trip():
    uid, ics = davgen.event(1, {"summary": "A, b; c", "rrule": "FREQ=DAILY;COUNT=3", "overrides": [{"summary": "moved"}]})
    vs = davgen.vevents(ics)
    assert [v["uid"] for v in vs] == [uid, uid] and vs[0]["summary"] == "A\\, b\\; c" and vs[1]["recurrence_id"]
    assert all(len(l.encode()) <= 75 for l in ics.split("\r\n"))
    edited = davgen.with_props(ics, "VEVENT", {"summary": "Moved"})
    assert davgen.vevents(edited)[0]["summary"] == "Moved" and davgen.vevents(edited)[1]["summary"] == "moved"
    _, vcf = davgen.group(1, {"name": "G", "members": ["x"], "version": "4.0"})
    f = davgen.vcard_fields(vcf)
    assert f["is_group"] and f["members"] == ["x"]
    assert re.search(r"^KIND:group\r$", vcf, re.M)


# -- Airpost server (apps/backend) ------------------------------------------------------------------------

def _backend(tmp_path):
    from harness.servers import backend as backendmod
    if backendmod.find_backend_binary() is None:
        pytest.skip("no airpost-backend binary (build apps/backend or set AIRPOST_BACKEND_BIN)")
    return backendmod, backendmod.BackendServer(tmp_path)


def test_backend_fixture(tmp_path):
    """The `backend` fixture: users and tokens through the CLI, `serverId`, a push read back
    through the pages, the rows surviving an outage, a restore, and a revoked token answered 401."""
    backendmod, b = _backend(tmp_path)
    with b:
        engine = b.device("engine")
        other = b.device("other-device")
        assert engine.user_id == other.user_id and engine.token != other.token
        assert engine.token.startswith("airpost_") and b.server_id
        ident = backendmod.airpost_server_identity(b, engine)
        assert ident["url"] == b.url and ident["serverId"] == b.server_id and ident["userId"] == engine.user_id
        assert backendmod.cpp_identity(engine)["createdAt"] == backendmod.CPP_IDENTITY_CREATED_AT
        # The engine binds its cursor to what /me answers; the fixture's identity must agree.
        me = b.me("engine")
        assert (me["serverId"], me["userId"]) == (b.server_id, engine.user_id)

        assert b.push("other-device", "acc1", "thread", "t:abc", "thread-snooze", {"expiration": 1}) == 1
        assert b.push("other-device", "acc1", "thread", "t:abc", "thread-snooze", {"expiration": 2}, version=1) == 2
        rows = b.rows("engine", "acc1")
        assert [(r["object_id"], r["plugin_id"], r["v"], r["value"]) for r in rows] == \
            [("t:abc", "thread-snooze", 2, {"expiration": 2})]

        server_id, port = b.server_id, b.port
        b.stop()
        with pytest.raises(OSError):
            b.request("GET", "/ping", timeout=1)
        b.start()
        assert (b.server_id, b.port) == (server_id, port)
        assert len(b.rows("engine", "acc1")) == 1

        # A restored backup: a new server id, writes after the backup gone, later tokens too.
        b.backup("one-row")
        b.device("late-device")
        b.push("other-device", "acc1", "thread", "t:def", "thread-snooze", {"expiration": 3})
        b.stop()
        b.restore("one-row")
        b.start()
        assert b.server_id != server_id and b.me("engine")["serverId"] == b.server_id
        assert [r["object_id"] for r in b.rows("engine", "acc1")] == ["t:abc"]
        assert "late-device" not in b.devices

        assert b.revoke("engine") == 1
        assert b.request("GET", "/metadata/acc1", device="engine")[0] == 401
        assert b.request("GET", "/metadata/acc1", device="other-device")[0] == 200


def test_engine_kind(tmp_path, monkeypatch):
    from harness.servers import backend as backendmod
    monkeypatch.delenv("HARNESS_ENGINE_KIND", raising=False)
    fake = tmp_path / "airpost-engine"
    fake.write_bytes(b"\0" * ((1 << 20) - 5) + b"RUST_BACKTRACE" + b"\0" * 10)   # straddles a read
    assert backendmod.engine_kind(fake) == "rust"
    other = tmp_path / "mailsync"
    other.write_bytes(b"\0mailcore\0" * 100)
    assert backendmod.engine_kind(other) == "cpp"
