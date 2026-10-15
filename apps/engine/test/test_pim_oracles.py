"""
Self-test of the PIM oracles (harness/pim.py, the contact/event/metadata invariants): a
database written the way a correct engine writes it passes, and each kind of damage a
scenario is meant to catch is reported. No engine involved; the DAV half needs Docker.

    python3 -m pytest apps/engine/test/test_pim_oracles.py -n 0
"""
import hashlib
import json
import sqlite3

import pytest

from harness import davgen, invariants, legacy, pim
from harness.servers import dav as davmod
from harness.servers.people import PeopleState

AID = "c0ffee-harness"


def fresh_db() -> sqlite3.Connection:
    conn = sqlite3.connect(":memory:")
    for sql in legacy.cpp_schema_statements():
        conn.execute(sql)
    conn.row_factory = sqlite3.Row
    return conn


def check_invariants(conn) -> list:
    conn.commit()   # invariants.check opens its own read transaction
    return invariants.check(conn)


def _id(*parts) -> str:
    return hashlib.sha256("-".join(parts).encode()).hexdigest()[:40]


def save_contact(conn, cid: str, data: dict):
    data = {"id": cid, "aid": AID, "v": 1, "__cls": "Contact", "refs": 0, "gis": [], "info": {}, **data}
    conn.execute("INSERT INTO Contact (id, data, accountId, email, version, refs, hidden, source, bookId, etag) "
                 "VALUES (?,?,?,?,?,?,?,?,?,?)",
                 (cid, json.dumps(data), AID, data.get("email", ""), 1, 0, int(bool(data.get("h"))), data.get("s", "mail"),
                  data.get("bid", ""), data.get("etag", "")))
    conn.execute("INSERT INTO ContactSearch (content_id, content) VALUES (?, ?)", (cid, data.get("email", "")))


def save_group(conn, gid: str, data: dict):
    data = {"id": gid, "aid": AID, "v": 1, "__cls": "ContactGroup", **data}
    conn.execute("INSERT INTO ContactGroup (id, accountId, bookId, data, version, name) VALUES (?,?,?,?,?,?)",
                 (gid, AID, data.get("bid", ""), json.dumps(data), 1, data.get("name", "")))


def save_book(conn, bid: str, data: dict):
    conn.execute("INSERT INTO ContactBook (id, accountId, data, version) VALUES (?,?,?,?)",
                 (bid, AID, json.dumps({"id": bid, "aid": AID, "v": 1, "__cls": "ContactBook", **data}), 1))


def join(conn, members: dict):
    """{group id: [contact ids]} into ContactContactGroup and each contact's `gis`."""
    gis = {}
    for gid, cids in members.items():
        for cid in cids:
            conn.execute("INSERT INTO ContactContactGroup (id, value) VALUES (?, ?)", (cid, gid))
            gis.setdefault(cid, []).append(gid)
    for cid, groups in gis.items():
        row = conn.execute("SELECT data FROM Contact WHERE id = ?", (cid,)).fetchone()
        data = json.loads(row[0])
        data["gis"] = sorted(groups)
        conn.execute("UPDATE Contact SET data = ? WHERE id = ?", (json.dumps(data), cid))


# -- People API ----------------------------------------------------------------------------------------

def people_db(state: PeopleState) -> sqlite3.Connection:
    """What GoogleContactsWorker::run leaves for `state`."""
    conn = fresh_db()
    book = f"{AID}-default"
    save_book(conn, book, {"source": "gpeople", "url": ""})
    ids = {}
    for p in state.live_people():
        cid = _id("contact", p.resource_name)
        ids[p.resource_name] = cid
        save_contact(conn, cid, {"s": "gpeople", "grn": p.resource_name, "email": p.email, "name": p.name,
                                 "bid": book, "info": p.json()})
    members = {}
    for g in state.live_groups(user_only=True):
        gid = _id("group", g.resource_name)
        save_group(conn, gid, {"grn": g.resource_name, "name": g.name, "bid": book})
        members[gid] = [ids[rn] for rn in state.members(g)]
    join(conn, members)
    return conn


def seeded_people() -> PeopleState:
    s = PeopleState()
    s.create_group("Climbing")
    s.create_person(name="Alice Archer", email="alice@example.org", groups=["Climbing"])
    s.create_person(name="Bob Baker", email="bob@example.org")
    return s


def test_people_oracle_accepts_a_correct_database():
    s = seeded_people()
    conn = people_db(s)
    assert pim.people_matches_db(conn, s, AID) == []
    assert check_invariants(conn) == []


def test_people_oracle_reports_divergence():
    s = seeded_people()
    conn = people_db(s)
    s.create_person(name="Dan Diver", email="dan@example.org")
    s.update_person(s.person_by_email("bob@example.org"), name="Robert Baker")
    s.modify_members(s.group_by_name("Climbing"), add=[s.person_by_email("bob@example.org").resource_name])
    problems = " | ".join(pim.people_matches_db(conn, s, AID))
    assert "dan@example.org" in problems
    assert "'Bob Baker' locally vs 'Robert Baker'" in problems
    assert "members" in problems


def test_contact_invariants_catch_gis_and_search_drift():
    conn = people_db(seeded_people())
    alice = conn.execute("SELECT id FROM Contact WHERE email = 'alice@example.org'").fetchone()[0]
    data = json.loads(conn.execute("SELECT data FROM Contact WHERE id = ?", (alice,)).fetchone()[0])
    data["gis"] = []
    conn.execute("UPDATE Contact SET data = ?, hidden = 1 WHERE id = ?", (json.dumps(data), alice))
    conn.execute("DELETE FROM ContactSearch WHERE content_id = ?", (alice,))
    found = " | ".join(check_invariants(conn))
    assert "[contacts]" in found and "gis [] vs ContactContactGroup" in found
    assert "column hidden = 1, data.h = 0" in found
    assert "0 ContactSearch rows" in found


def test_contact_invariants_accept_cpp_removal_leftovers():
    """ContactGroup::afterRemove keeps the members' `gis`; Contact::afterRemove keeps the
    contact's join rows. Neither is a violation."""
    def contact_violations(conn):
        return [v for v in check_invariants(conn) if v.startswith("[contacts]")]

    conn = people_db(seeded_people())
    gid, cid = conn.execute("SELECT value, id FROM ContactContactGroup LIMIT 1").fetchone()
    conn.execute("DELETE FROM ContactGroup WHERE id = ?", (gid,))
    conn.execute("DELETE FROM ContactContactGroup WHERE value = ?", (gid,))
    assert gid in json.loads(conn.execute("SELECT data FROM Contact WHERE id = ?", (cid,)).fetchone()[0])["gis"]
    assert contact_violations(conn) == []

    conn = people_db(seeded_people())
    conn.execute("DELETE FROM Contact WHERE id = ?", (cid,))
    conn.execute("DELETE FROM ContactSearch WHERE content_id = ?", (cid,))
    assert conn.execute("SELECT COUNT(*) FROM ContactContactGroup WHERE id = ?", (cid,)).fetchone()[0] == 1
    assert contact_violations(conn) == []


def metadata_violations(conn) -> list:
    return [v for v in check_invariants(conn) if v.startswith("[plugin_metadata]")]


def test_plugin_metadata_invariant():
    conn = fresh_db()
    meta = [{"pluginId": "thread-snooze", "value": {"expiration": 1900000000}, "v": 1},
            {"pluginId": "empty", "value": {}, "v": 2}]
    data = {"id": "t1", "aid": AID, "v": 1, "__cls": "Thread", "folders": [], "labels": [], "metadata": meta}
    conn.execute("INSERT INTO Thread (id, accountId, version, data, subject, unread, starred, firstMessageTimestamp, "
                 "lastMessageTimestamp, lastMessageReceivedTimestamp, lastMessageSentTimestamp, inAllMail) "
                 "VALUES ('t1', ?, 1, ?, '', 0, 0, 0, 0, 0, 0, 1)", (AID, json.dumps(data)))
    assert any("no ModelPluginMetadata row" in v for v in metadata_violations(conn))
    conn.execute("INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) "
                 "VALUES ('t1', ?, 'Thread', 'thread-snooze', 1900000000)", (AID,))
    assert metadata_violations(conn) == []
    conn.execute("UPDATE ModelPluginMetadata SET expiration = NULL")
    assert any("(objectType, expiration)" in v for v in metadata_violations(conn))
    conn.execute("UPDATE ModelPluginMetadata SET expiration = 1900000000")
    # A scalar value has nlohmann size() 1, so MailModel::afterSave gives even `false` a row.
    data["metadata"] = meta + [{"pluginId": "flag", "value": False, "v": 1}]
    conn.execute("UPDATE Thread SET data = ? WHERE id = 't1'", (json.dumps(data),))
    assert any("t1 carries flag metadata" in v for v in metadata_violations(conn))
    conn.execute("INSERT INTO ModelPluginMetadata (id, accountId, objectType, value, expiration) "
                 "VALUES ('t1', ?, 'Thread', 'flag', NULL)", (AID,))
    assert metadata_violations(conn) == []


def test_check_models_and_selectors():
    conn = people_db(seeded_people())
    assert pim.check_models(conn, {"Contact": {"where": {"s": "gpeople"}, "count": 2,
                                               "match": [{"email": "alice@example.org", "grn": {"re": "^people/"}, "h": {"not": True}}],
                                               "absent": [{"email": "nobody@example.org"}]}}, AID) == []
    out = pim.check_models(conn, {"Contact": {"count": 3}, "ContactGroup": {"match": [{"name": "Nope"}]}}, AID)
    assert len(out) == 2


def test_resolve_now():
    out = pim.resolve_now({"value": {"expiration": {"now_plus": 10}}, "list": [{"now_plus": 0}]})
    assert isinstance(out["value"]["expiration"], int) and isinstance(out["list"][0], int)


# -- DAV ------------------------------------------------------------------------------------------------

def dav_db(truth: dict) -> sqlite3.Connection:
    """What DAVWorker leaves for the server's state: Calendar per VEVENT collection, an Event
    per in-window VEVENT, a ContactBook per address book, a Contact per card (id = UID),
    group cards hidden plus a ContactGroup with members."""
    import time
    from datetime import datetime, timezone
    conn = fresh_db()
    now = datetime.now(timezone.utc)
    for href, cal in truth["calendars"].items():
        cid = _id("cal", href)
        conn.execute("INSERT INTO Calendar (id, data, accountId) VALUES (?,?,?)",
                     (cid, json.dumps({"id": cid, "aid": AID, "v": 1, "__cls": "Calendar", "path": href, "name": cal["name"]}), AID))
        for ihref, item in cal["items"].items():
            if not pim._in_window(item["data"], now):
                continue
            for v in davgen.vevents(item["data"]):
                eid = _id("event", cid, v["uid"], v["recurrence_id"])
                rs = int(time.time())
                data = {"id": eid, "aid": AID, "v": 1, "__cls": "Event", "cid": cid, "icsuid": v["uid"], "rid": v["recurrence_id"],
                        "etag": item["etag"], "href": ihref, "ics": item["data"], "rs": rs, "re": rs + 3600}
                conn.execute("INSERT INTO Event (id, data, accountId, etag, calendarId, recurrenceStart, recurrenceEnd, icsuid, "
                             "recurrenceId) VALUES (?,?,?,?,?,?,?,?,?)",
                             (eid, json.dumps(data), AID, item["etag"], cid, rs, rs + 3600, v["uid"], v["recurrence_id"]))
                conn.execute("INSERT INTO EventSearch (content_id, title, description, location, participants) VALUES (?,?,?,?,?)",
                             (eid, v["summary"], "", "", ""))
    for href, ab in truth["addressbooks"].items():
        bid = _id("book", href)
        save_book(conn, bid, {"source": "carddav", "url": f"http://127.0.0.1{href}"})
        cards = {}
        for ihref, item in ab["items"].items():
            f = davgen.vcard_fields(item["data"])
            cards[f["uid"]] = f
            save_contact(conn, f["uid"], {"s": "carddav", "email": f["emails"][0] if f["emails"] else "", "name": f["fn"],
                                          "h": f["is_group"], "bid": bid, "etag": item["etag"],
                                          "info": {"vcf": item["data"], "href": ihref}})
        members = {}
        for uid, f in cards.items():
            if f["is_group"]:
                save_group(conn, uid, {"name": f["fn"], "bid": bid})
                members[uid] = [m for m in f["members"] if m in cards and not cards[m]["is_group"]]
        join(conn, members)
    return conn


@pytest.fixture(scope="module", params=[pytest.param(k, marks=[pytest.mark.slow, pytest.mark.docker(k)])
                                        for k in sorted(davmod.KINDS)])
def seeded_dav(request, tmp_path_factory):
    server = davmod.DavServer(request.param, log_path=str(tmp_path_factory.mktemp(request.param) / "server.log"))
    server.start()
    try:
        for n, spec in enumerate([{"summary": "Soon", "start": "+1d"},
                                  {"uid": "weekly@example.test", "rrule": "FREQ=WEEKLY;COUNT=4", "overrides": [{"summary": "moved"}]},
                                  {"uid": "old@example.test", "start": "-900d"}], start=1):
            uid, ics = davgen.event(n, spec)
            server.put_event("default", uid, ics)
        for n, spec in enumerate([{"uid": "alice-uid", "name": "Alice", "email": "alice@example.org"},
                                  {"uid": "bob-uid", "name": "Bob", "email": "bob@example.org", "version": "4.0"}], start=1):
            uid, vcf = davgen.contact(n, spec)
            server.put_card("default", uid, vcf)
        uid, vcf = davgen.group(1, {"uid": "g-uid", "name": "Climbing", "members": ["alice-uid", "bob-uid"]})
        server.put_card("default", uid, vcf)
        yield server
    finally:
        server.stop()


def test_dav_oracle_accepts_a_correct_database(seeded_dav):
    conn = dav_db(seeded_dav.dav_truth())
    assert pim.dav_matches_db(conn, seeded_dav.dav_truth(), AID) == []
    assert check_invariants(conn) == []
    assert not any("old@example.test" == r[0] for r in conn.execute("SELECT icsuid FROM Event"))


def test_dav_oracle_reports_divergence(seeded_dav):
    truth = seeded_dav.dav_truth()
    conn = dav_db(truth)
    conn.execute("DELETE FROM Event WHERE icsuid = 'weekly@example.test' AND recurrenceId != ''")
    conn.execute("UPDATE Contact SET data = json_set(data, '$.etag', 'stale') WHERE id = 'alice-uid'")
    conn.execute("DELETE FROM ContactContactGroup WHERE id = 'bob-uid'")
    problems = " | ".join(pim.dav_matches_db(conn, truth, AID))
    assert "weekly@example.test (an override)" in problems
    assert "etag 'stale' locally" in problems
    assert "members ['alice-uid'] locally vs ['alice-uid', 'bob-uid']" in problems
    # And a change on the server that the database has not seen.
    found = seeded_dav.find_event("weekly@example.test")
    seeded_dav.dav.put(found["href"], davgen.with_props(found["data"], "VEVENT", {"SUMMARY": "Renamed"}), "text/calendar",
                       if_match=found["etag"])
    problems = " | ".join(pim.dav_matches_db(dav_db(truth), seeded_dav.dav_truth(), AID))
    assert "weekly@example.test" in problems and "etag" in problems
