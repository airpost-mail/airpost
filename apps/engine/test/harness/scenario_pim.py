"""
The ScenarioRun steps, waits and expectations for contacts, calendars, plugin metadata and
whole-database checks (harness/pim.py holds the comparisons). Mixed into ScenarioRun;
test/README.md documents the vocabulary.
"""
import contextlib
import json
import re
import sqlite3
from pathlib import Path

from . import davgen, legacy, pim


class ScenarioFailure(AssertionError):
    pass


class PimMixin:
    # -- setup ---------------------------------------------------------------------------------------

    def _pim_init(self):
        self.marks: dict = {}
        self.event_counter = 1
        self.contact_counter = 1
        self.group_counter = 1

    def _pim_seed_server(self):
        """`people:` and `calendars:` / `addressbooks:` populate the services before mailsync
        starts, as `mailboxes:` does for IMAP."""
        if self.sc.get("people"):
            people = self._people_state()
            for g in self.sc["people"].get("groups") or []:
                people.create_group(g["name"])
            for c in self.sc["people"].get("contacts") or []:
                for spec in self._expand(c, self._people_spec):
                    people.create_person(**spec)
        for name, cal in (self.sc.get("calendars") or {}).items():
            cal = cal or {}
            if cal.get("create"):
                self._dav().dav.mkcalendar(self._dav().calendar_href(name), cal.get("display_name", name),
                                           cal.get("components", ["VEVENT"]))
            for ev in cal.get("events") or []:
                for spec in self._expand(ev):
                    self._dav_add_event(name, spec)
        for name, ab in (self.sc.get("addressbooks") or {}).items():
            ab = ab or {}
            for c in ab.get("contacts") or []:
                for spec in self._expand(c):
                    self._dav_add_contact(name, spec)
            for g in ab.get("groups") or []:
                self._dav_add_group(name, g)

    @staticmethod
    def _expand(spec: dict, make=dict) -> list:
        """`{count: n, ...}` -> n copies of the rest of the spec, each through `make`; the
        generators number what the spec leaves out (UIDs, names, addresses)."""
        if "count" not in spec:
            return [spec]
        base = {k: v for k, v in spec.items() if k != "count"}
        return [make(dict(base)) for _ in range(int(spec["count"]))]

    def _people_spec(self, spec: dict) -> dict:
        i = self.contact_counter
        self.contact_counter += 1
        return {"name": spec.get("name", f"Person {i}"), "email": spec.get("email", f"person{i}@example.com"),
                "groups": spec.get("groups", [])}

    def _write_legacy_db(self):
        spec = self.sc.get("legacy_db")
        if not spec:
            return

        def resolve(mailbox: str, uid: int) -> dict:
            truth = self.server.truth()
            entry = truth.get(self.server.server_path(mailbox), {}).get(uid)
            if entry is None:
                raise ScenarioFailure(f"legacy_db names {mailbox} UID {uid}, which the server does not have")
            return {"message_id": entry["message_id"]}
        written = legacy.write_legacy_database(self.ms.config_dir, spec, self.ms.account["id"], resolve)
        self._note(f"wrote a C++-era edgehill.db{' (writes left in the WAL)' if spec.get('wal') else ''} "
                   f"and {len(written['files'])} files under files/")

    # -- services ------------------------------------------------------------------------------------

    def _people_state(self):
        people = getattr(self.server, "people", None)
        if people is None:
            raise ScenarioFailure("this scenario needs the fake People API: add `people: true` to a fake server entry")
        return people.state

    def _dav(self):
        if not hasattr(self.server, "dav_truth"):
            raise ScenarioFailure(f"this step needs a DAV server kind (radicale, baikal), not {self.spec.kind}")
        return self.server

    # -- DAV: what another client does ---------------------------------------------------------------

    def _dav_add_event(self, calendar: str, spec: dict) -> dict:
        n = self.event_counter
        self.event_counter += 1
        uid, ics = davgen.event(n, spec)
        return self._dav().put_event(calendar, uid, ics)

    def _dav_add_contact(self, addressbook: str, spec: dict) -> dict:
        n = self.contact_counter
        self.contact_counter += 1
        uid, vcf = davgen.contact(n, spec)
        return self._dav().put_card(addressbook, uid, vcf)

    def _member_uids(self, addressbook: str, members: list) -> list:
        out = []
        for m in members or []:
            if "@" in m and not m.startswith("harness-"):
                card = self._card_by_email(addressbook, m)
                out.append(card["uid"])
            else:
                out.append(m)
        return out

    def _dav_add_group(self, addressbook: str, spec: dict) -> dict:
        n = self.group_counter
        self.group_counter += 1
        spec = dict(spec)
        spec["members"] = self._member_uids(addressbook, spec.get("members"))
        uid, vcf = davgen.group(n, spec)
        return self._dav().put_card(addressbook, uid, vcf)

    def _card_by_email(self, addressbook: str, email: str) -> dict:
        srv = self._dav()
        for href, item in srv.dav.items(srv.addressbook_href(addressbook), calendar=False).items():
            f = davgen.vcard_fields(item["data"])
            if email.lower() in (e.lower() for e in f["emails"]):
                return {**item, "href": href, "uid": f["uid"]}
        raise ScenarioFailure(f"no card with EMAIL {email} in {addressbook}")

    def _card(self, arg: dict) -> dict:
        ab = arg.get("addressbook", "default")
        if arg.get("email"):
            return self._card_by_email(ab, arg["email"])
        uid = arg.get("uid") or self._group_uid(ab, arg["group"])
        card = self._dav().find_card(uid, ab)
        if card is None:
            raise ScenarioFailure(f"no card with UID {uid} in {ab}")
        return {**card, "uid": uid}

    def _group_uid(self, addressbook: str, name: str) -> str:
        srv = self._dav()
        for item in srv.dav.items(srv.addressbook_href(addressbook), calendar=False).values():
            f = davgen.vcard_fields(item["data"])
            if f["is_group"] and f["fn"] == name:
                return f["uid"]
        raise ScenarioFailure(f"no group card named {name!r} in {addressbook}")

    def _dav_step(self, op: str, arg: dict):
        srv = self._dav()
        if op == "add_event":
            r = self._dav_add_event(arg.get("calendar", "default"), arg.get("event") or {})
        elif op == "update_event":
            found = srv.find_event(arg["uid"], arg.get("calendar"))
            if found is None:
                raise ScenarioFailure(f"no event {arg['uid']} on the server")
            seq = max([int(v) for v in re.findall(r"^SEQUENCE:(\d+)", "\n".join(davgen.unfold(found["data"])), re.M)] or [0])
            ics = davgen.with_props(found["data"], "VEVENT", {"SEQUENCE": str(seq + 1), **(arg.get("changes") or {})})
            etag = srv.dav.put(found["href"], ics, "text/calendar", if_match=found["etag"])
            r = {"href": found["href"], "etag": etag, "uid": arg["uid"]}
        elif op == "delete_event":
            found = srv.find_event(arg["uid"], arg.get("calendar"))
            if found is None:
                raise ScenarioFailure(f"no event {arg['uid']} on the server")
            srv.dav.delete(found["href"], if_match=found["etag"])
            r = {"href": found["href"], "uid": arg["uid"]}
        elif op == "add_contact":
            r = self._dav_add_contact(arg.get("addressbook", "default"), arg.get("contact") or {})
        elif op == "add_group":
            r = self._dav_add_group(arg.get("addressbook", "default"), arg)
        elif op in ("update_contact", "set_group_members"):
            card = self._card(arg)
            text = card["data"]
            if op == "set_group_members":
                members = self._member_uids(arg.get("addressbook", "default"), arg.get("members"))
                lines = [l for l in davgen.unfold(text)
                         if not re.match(r"^(X-ADDRESSBOOKSERVER-MEMBER|MEMBER)[;:]", l, re.I)]
                prop = "MEMBER" if any(l.upper() == "VERSION:4.0" for l in lines) else "X-ADDRESSBOOKSERVER-MEMBER"
                end = lines.index("END:VCARD")
                lines[end:end] = [f"{prop}:urn:uuid:{m}" for m in members]
                text = "\r\n".join(lines) + "\r\n"
            else:
                text = davgen.with_props(text, "VCARD", arg.get("changes") or {})
            etag = srv.dav.put(card["href"], text, "text/vcard", if_match=card["etag"])
            r = {"href": card["href"], "etag": etag, "uid": card["uid"]}
        elif op == "delete_contact":
            card = self._card(arg)
            srv.dav.delete(card["href"], if_match=card["etag"])
            r = {"href": card["href"], "uid": card["uid"]}
        elif op == "create_calendar":
            href = srv.calendar_href(arg["name"])
            srv.dav.mkcalendar(href, arg.get("display_name", arg["name"]), arg.get("components", ["VEVENT"]))
            r = {"href": href}
        elif op == "delete_calendar":
            href = srv.calendar_href(arg["name"])
            srv.dav.remove_collection(href)
            r = {"href": href}
        else:
            raise ScenarioFailure(f"unknown dav op {op!r}")
        self._note(f"dav.{op}: {json.dumps(r)}")
        return r

    # -- People: what another client does ----------------------------------------------------------

    def _people_step(self, op: str, arg: dict):
        s = self._people_state()
        if op == "add_contact":
            p = s.create_person(name=arg.get("name", ""), email=arg.get("email", ""), groups=arg.get("groups") or [])
            return p.resource_name
        if op == "update_contact":
            p = s.person_by_email(arg["email"])
            s.update_person(p, name=arg.get("name"), email=arg.get("new_email"), phone=arg.get("phone"))
            return p.resource_name
        if op == "delete_contact":
            p = s.person_by_email(arg["email"])
            s.delete_person(p)
            return p.resource_name
        if op == "add_group":
            return s.create_group(arg["name"]).resource_name
        if op == "rename_group":
            g = s.group_by_name(arg["name"])
            s.rename_group(g, arg["to"])
            return g.resource_name
        if op == "delete_group":
            g = s.group_by_name(arg["name"])
            s.delete_group(g)
            return g.resource_name
        if op == "group_members":
            g = s.group_by_name(arg["group"])
            s.modify_members(g, [s.person_by_email(e).resource_name for e in arg.get("add") or []],
                             [s.person_by_email(e).resource_name for e in arg.get("remove") or []])
            return g.resource_name
        if op == "expire_sync_tokens":
            s.expire_sync_tokens()
            return None
        raise ScenarioFailure(f"unknown people op {op!r}")

    # -- client tasks ----------------------------------------------------------------------------------

    def _rows(self, table: str) -> list:
        with self.ms.db() as c:
            return pim.load_rows(c, table, self.ms.account["id"])

    def _contact_row(self, sel: dict) -> dict:
        rows = [r for r in self._rows("Contact") if (r["data"].get("s") or r.get("source")) != "mail"]
        if "uid" in sel:
            hits = [r for r in rows if r["id"] == sel["uid"]]
        elif "email" in sel:
            hits = [r for r in rows if (r["data"].get("email") or "").lower() == sel["email"].lower()]
        elif "name" in sel:
            hits = [r for r in rows if r["data"].get("name") == sel["name"]]
        else:
            raise ScenarioFailure(f"a contact selector needs uid, email or name: {sel}")
        if not hits:
            raise ScenarioFailure(f"no synced Contact matches {sel} in the engine's database")
        return hits[0]

    def _group_row(self, sel: dict) -> dict:
        hits = [r for r in self._rows("ContactGroup") if r["data"].get("name") == sel.get("name") or r["id"] == sel.get("id")]
        if not hits:
            raise ScenarioFailure(f"no ContactGroup matches {sel} in the engine's database")
        return hits[0]

    def _calendar_id(self, name: str) -> str:
        want = pim.norm_href(self._dav().calendar_href(name))
        for r in self._rows("Calendar"):
            if pim.norm_href(r["data"].get("path", "")) == want:
                return r["id"]
        raise ScenarioFailure(f"calendar {name} ({want}) is not in the engine's database")

    def _event_row(self, uid: str) -> dict:
        hits = [r for r in self._rows("Event") if r["data"].get("icsuid") == uid and not r["data"].get("rid")]
        if not hits:
            raise ScenarioFailure(f"event {uid} is not in the engine's database")
        return hits[0]

    def _contact_source(self) -> str:
        return "gpeople" if getattr(self.server, "people", None) else "carddav"

    def _new_contact_json(self, spec: dict) -> dict:
        source = self._contact_source()
        name, email = spec.get("name", ""), spec.get("email", "")
        base = {"__cls": "Contact", "aid": self.ms.account["id"], "v": 0, "s": source, "name": name, "email": email}
        if source == "gpeople":
            given, _, family = name.partition(" ")
            base["info"] = {"names": [{"displayName": name, "givenName": given, "familyName": family}],
                            "emailAddresses": [{"value": email}]}
        else:
            n = self.contact_counter
            self.contact_counter += 1
            _, vcf = davgen.contact(n, spec)
            base["info"] = {"vcf": vcf, "href": ""}
        return base

    def _changed_contact_json(self, row: dict, changes: dict) -> dict:
        data = json.loads(json.dumps(row["data"]))
        if self._contact_source() == "gpeople":
            info = data.setdefault("info", {})
            if "name" in changes:
                given, _, family = changes["name"].partition(" ")
                info["names"] = [{"displayName": changes["name"], "givenName": given, "familyName": family}]
                data["name"] = changes["name"]
            if "email" in changes:
                info["emailAddresses"] = [{"value": changes["email"]}]
                data["email"] = changes["email"]
        else:
            # An edit as the client's contact editor makes it: FN and EMAIL replaced, the
            # rest of the stored card kept.
            vcf = data["info"]["vcf"]
            props = {k: v for k, v in changes.items() if k.isupper()}
            if "name" in changes:
                props["FN"] = changes["name"]
                data["name"] = changes["name"]
            if "email" in changes:
                lines = [l for l in davgen.unfold(vcf) if davgen.prop(l)[0].split(".")[-1] != "EMAIL"]
                vcf = "\r\n".join(lines) + "\r\n"
                props["EMAIL;TYPE=INTERNET"] = changes["email"]
                data["email"] = changes["email"]
            data["info"]["vcf"] = davgen.with_props(vcf, "VCARD", props)
        return data

    def _resolve_pim_task(self, task: dict) -> dict:
        cls = task.get("__cls")
        task = pim.resolve_now(task)
        if cls == "SyncbackMetadataTask" and "model" in task:
            table, ids = self._resolve_model_selector(task.pop("model"))
            if len(ids) != 1:
                raise ScenarioFailure(f"SyncbackMetadataTask needs exactly one model, selector gave {ids}")
            with self.ms.db() as c:
                row = c.execute(f"SELECT data FROM `{table}` WHERE id = ?", (ids[0],)).fetchone()
            task["modelId"], task["modelClassName"] = ids[0], table
            task["modelHeaderMessageId"] = json.loads(row[0]).get("hMsgId") if table == "Message" else None
            task.setdefault("value", {})
        if cls == "SyncbackEventTask":
            ev = task.get("event") or {}
            if "uid" in ev:
                row = self._event_row(ev["uid"])
                data = json.loads(json.dumps(row["data"]))
                ics = data.get("ics", "")
                seq = max([int(v) for v in re.findall(r"^SEQUENCE:(\d+)", "\n".join(davgen.unfold(ics)), re.M)] or [0])
                data["ics"] = davgen.with_props(ics, "VEVENT", {"SEQUENCE": str(seq + 1), **(ev.get("changes") or {})})
                task["event"] = data
                task.setdefault("calendarId", data.get("cid"))
            else:
                n = self.event_counter
                self.event_counter += 1
                _, ics = davgen.event(n, ev.get("ics") or {})
                task["event"] = {"__cls": "Event", "aid": self.ms.account["id"], "ics": ics}
            if isinstance(task.get("calendar"), str):
                task["calendarId"] = self._calendar_id(task.pop("calendar"))
        if cls == "DestroyEventTask":
            task["events"] = [self._event_row(e["uid"])["data"] for e in task.get("events") or []]
        if cls == "SyncbackContactTask":
            c = task.get("contact") or {}
            task["contact"] = self._new_contact_json(c["new"]) if "new" in c else \
                self._changed_contact_json(self._contact_row(c), c.get("changes") or {})
        if cls in ("DestroyContactTask", "ChangeContactGroupMembershipTask"):
            task["contacts"] = [self._contact_row(c)["data"] for c in task.get("contacts") or []]
        if cls in ("SyncbackContactGroupTask", "DestroyContactGroupTask", "ChangeContactGroupMembershipTask"):
            g = task.get("group") or {}
            if cls == "SyncbackContactGroupTask" and g.get("new"):
                task["group"] = {"__cls": "ContactGroup", "aid": self.ms.account["id"], "name": g["name"]}
            else:
                data = json.loads(json.dumps(self._group_row(g)["data"]))
                if g.get("rename"):
                    data["name"] = g["rename"]
                task["group"] = data
        return task

    def _resolve_model_selector(self, want: dict) -> tuple:
        """{threads: {mailbox, uids}} | {messages: {mailbox, uids} | {header_message_ids}} ->
        (table, ids)."""
        if "threads" in want:
            return "Thread", self._resolve_threads(want["threads"])
        if "messages" in want:
            return "Message", self._resolve_messages(want["messages"])
        raise ScenarioFailure(f"a model selector needs threads: or messages:, got {want}")

    # -- waits ------------------------------------------------------------------------------------------

    def _pim_wait(self, arg: dict):
        timeout = float(arg.get("timeout", 90))
        if "models" in arg:
            def ok():
                with self.ms.db() as c:
                    self.wait_detail = pim.check_models(c, arg["models"], self.ms.account["id"])
                return not self.wait_detail
            self.wait_detail = []
            try:
                self.ms.wait_for(ok, timeout, f"models {json.dumps(arg['models'])}", poll=0.25)
            except TimeoutError as e:
                raise TimeoutError(f"{e}\nstill: {'; '.join(self.wait_detail[:4])}")
        if "delta" in arg:
            want = arg["delta"]
            since = self.marks.get(want.get("since"), 0) if want.get("since") else 0

            def seen():
                n = 0
                for d in self.all_deltas():
                    if d.wall < since or d.model_class != want.get("class", d.model_class) or d.type != want.get("type", d.type):
                        continue
                    n += sum(1 for m in d.models if pim.row_matches({"data": m}, want.get("where")))
                    # Signals such as ProcessIdentityRefreshNeeded carry no models.
                    n += 1 if not d.models and not want.get("where") else 0
                return n >= int(want.get("count", 1))
            self.ms.wait_for(seen, timeout, f"delta {json.dumps(want)}", poll=0.25)
        if "people_requests" in arg:
            # Same selectors as the `people_requests` expectation, polled.
            self.ms.wait_for(lambda: not self.expect_people_requests(arg["people_requests"]), timeout,
                             f"People API requests {json.dumps(arg['people_requests'])}", poll=0.25)

    def all_deltas(self) -> list:
        return self.earlier_deltas + (self.ms.deltas() if self.ms else [])

    # -- expectations -------------------------------------------------------------------------------------

    def expect_models(self, arg: dict) -> list:
        with self.ms.db() as c:
            return pim.check_models(c, arg, self.ms.account["id"])

    def expect_sql(self, arg) -> list:
        queries = arg if isinstance(arg, list) else arg.get("queries", [])

        @contextlib.contextmanager
        def conn_for(which: str):
            if which == "main":
                with self.ms.db() as c:
                    yield c
                return
            path = self.ms.config_dir / (legacy.LEGACY_DATABASE if which == "legacy" else which)
            if not path.exists():
                raise FileNotFoundError(f"{path.name} does not exist")
            # Read-only, on a copy: opening the legacy database in place would checkpoint its
            # WAL, which is part of what a scenario asserts was moved intact.
            import shutil
            import tempfile
            with tempfile.TemporaryDirectory() as d:
                for suffix in ("", "-wal", "-shm"):
                    if Path(str(path) + suffix).exists():
                        shutil.copy(str(path) + suffix, Path(d) / (path.name + suffix))
                c = sqlite3.connect(str(Path(d) / path.name))
                try:
                    yield c
                finally:
                    c.close()
        try:
            return pim.check_sql(conn_for, queries)
        except FileNotFoundError as e:
            return [str(e)]

    def expect_config_files(self, arg: dict) -> list:
        return pim.check_config_files(self.ms.config_dir, arg)

    def expect_metadata(self, arg) -> list:
        with self.ms.db() as c:
            return pim.check_metadata(c, arg, self._resolve_model_selector)

    def expect_metadata_expiration(self, arg) -> list:
        """Entries as pim.check_metadata_expiration, plus `since: <mark>` to count only deltas
        after a `mark` step (e.g. none once the client cleared the expiration)."""
        out = []
        for entry in arg:
            entry = dict(entry)
            since = entry.pop("since", None)
            if since is not None and since not in self.marks:
                out.append(f"no `mark: {since}` step ran")
                continue
            deltas = [d for d in self.all_deltas() if since is None or d.wall >= self.marks[since]]
            out += pim.check_metadata_expiration(deltas, [entry], self._resolve_model_selector)
        return out

    def expect_changed_since(self, arg: dict) -> list:
        arg = dict(arg)
        mark = arg.pop("mark")
        if mark not in self.marks:
            return [f"no `mark: {mark}` step ran"]
        return pim.check_changed_since(self.all_deltas(), self.marks[mark], arg)

    def expect_tasks(self, arg: dict) -> list:
        """{label: {status, error: null | key regex}}: how a queued task ended; `error` is the
        key of the SyncException JSON the engine stored in the task."""
        out = []
        for label, want in arg.items():
            task_id = self.labels.get(label)
            with self.ms.db() as c:
                row = c.execute("SELECT status, data FROM Task WHERE id = ?", (task_id,)).fetchone()
            if row is None:
                out.append(f"task {label} ({task_id}) is not in the Task table")
                continue
            data = json.loads(row[1])
            if "status" in want and row[0] != want["status"]:
                out.append(f"task {label}: status {row[0]!r}, expected {want['status']!r}")
            if "error" in want:
                err = data.get("error")
                if want["error"] is None:
                    if err:
                        out.append(f"task {label} failed: {json.dumps(err)[:300]}")
                elif not err or not re.search(want["error"], str(err.get("key", ""))):
                    out.append(f"task {label}: error {json.dumps(err)[:300]}, expected key /{want['error']}/")
        return out

    def expect_dav_matches_db(self, arg: dict) -> list:
        truth = self._dav().dav_truth()
        with self.ms.db() as c:
            return pim.dav_matches_db(c, truth, self.ms.account["id"], check_window=arg.get("window", True))

    def expect_people_matches_db(self, arg: dict) -> list:
        state = self._people_state()
        with self.ms.db() as c:
            return pim.people_matches_db(c, state, self.ms.account["id"])

    def expect_dav_has(self, arg: dict) -> list:
        """{events: [{uid, summary?}], no_events: [uids], contacts: [{uid | email, name?}],
        no_contacts: [uid | email], groups: {name: [member emails]}}: the server's state,
        whatever the engine believes - what proves a task's remote phase wrote it."""
        srv = self._dav()
        truth = srv.dav_truth()
        out = []
        events = {}
        for cal in truth["calendars"].values():
            for item in cal["items"].values():
                for v in davgen.vevents(item["data"]):
                    events.setdefault(v["uid"], []).append(v)
        for want in arg.get("events") or []:
            vs = events.get(want["uid"])
            if not vs:
                out.append(f"the server has no event {want['uid']}")
            elif "summary" in want and not any(v["summary"] == want["summary"] for v in vs):
                out.append(f"event {want['uid']} on the server has SUMMARY {[v['summary'] for v in vs]}, expected {want['summary']!r}")
        for uid in arg.get("no_events") or []:
            if uid in events:
                out.append(f"the server still has event {uid}")
        cards = [davgen.vcard_fields(i["data"]) for ab in truth["addressbooks"].values() for i in ab["items"].values()]

        def find(sel):
            if isinstance(sel, str):
                sel = {"email": sel} if "@" in sel else {"uid": sel}
            return [f for f in cards if (sel.get("uid") and f["uid"] == sel["uid"])
                    or (sel.get("email") and sel["email"].lower() in (e.lower() for e in f["emails"]))
                    or (sel.get("name") and not sel.get("email") and not sel.get("uid") and f["fn"] == sel["name"])]
        for want in arg.get("contacts") or []:
            hits = find(want)
            if not hits:
                out.append(f"the server has no card matching {want}")
            elif isinstance(want, dict) and "name" in want and (want.get("email") or want.get("uid")) \
                    and not any(h["fn"] == want["name"] for h in hits):
                out.append(f"card {want} on the server has FN {[h['fn'] for h in hits]}, expected {want['name']!r}")
        for sel in arg.get("no_contacts") or []:
            if find(sel):
                out.append(f"the server still has a card matching {sel}")
        by_uid = {f["uid"]: f for f in cards}
        for name, emails in (arg.get("groups") or {}).items():
            g = next((f for f in cards if f["is_group"] and f["fn"] == name), None)
            if g is None:
                out.append(f"the server has no group card {name!r}")
                continue
            got = sorted(e for m in g["members"] for e in (by_uid.get(m, {}).get("emails") or [f"<{m}>"])[:1])
            if got != sorted(emails):
                out.append(f"group {name!r} on the server has members {got}, expected {sorted(emails)}")
        return out

    def expect_people_has(self, arg: dict) -> list:
        """{contacts: [{email, name?}], no_contacts: [emails], groups: {name: [emails]},
        no_groups: [names]} in the fake People API."""
        s = self._people_state()
        out = []
        with s.lock:
            live = {p.email.lower(): p for p in s.live_people()}
            for want in arg.get("contacts") or []:
                want = {"email": want} if isinstance(want, str) else want
                p = live.get(want["email"].lower())
                if p is None:
                    out.append(f"the People API has no contact {want['email']}")
                elif "name" in want and p.name != want["name"]:
                    out.append(f"contact {want['email']} is named {p.name!r} in the People API, expected {want['name']!r}")
            for email in arg.get("no_contacts") or []:
                if email.lower() in live:
                    out.append(f"the People API still has contact {email}")
            for name, emails in (arg.get("groups") or {}).items():
                try:
                    g = s.group_by_name(name)
                except KeyError:
                    out.append(f"the People API has no contact group {name!r}")
                    continue
                got = sorted(s.person_by_resource(rn).email for rn in s.members(g) if s.person_by_resource(rn))
                if got != sorted(emails):
                    out.append(f"group {name!r} has members {got} in the People API, expected {sorted(emails)}")
            for name in arg.get("no_groups") or []:
                if any(g.name == name for g in s.live_groups()):
                    out.append(f"the People API still has contact group {name!r}")
        return out

    def expect_people_requests(self, arg: list) -> list:
        """[{method, path: regex, query: {param: value | true | false}, count | min | max,
        status}]: what the engine asked the People API, e.g. that an incremental pass sent
        its syncToken and did not re-list everything."""
        people = getattr(self.server, "people", None)
        if people is None:
            return ["people_requests needs the fake People API"]
        out = []
        for want in arg:
            hits = []
            for r in people.requests_matching(want.get("method"), want.get("path")):
                if "status" in want and r.status != want["status"]:
                    continue
                ok = True
                for k, v in (want.get("query") or {}).items():
                    if v is True and k not in r.query or v is False and k in r.query \
                            or not isinstance(v, bool) and str(r.query.get(k)) != str(v):
                        ok = False
                if ok:
                    hits.append(r)
            lo = want.get("min", want.get("count", 1))
            hi = want.get("max", want.get("count"))
            if len(hits) < lo or (hi is not None and len(hits) > hi):
                out.append(f"People API requests matching {json.dumps(want)}: {len(hits)}; all requests: "
                           + ", ".join(f"{r.method} {r.path}?{'&'.join(sorted(r.query))} {r.status}" for r in people.requests[-12:]))
        return out
