"""
An in-process fake of the Google endpoints a Gmail account's engine talks to besides IMAP
and SMTP: the OAuth token endpoint and the People API v1 calls GoogleContactsWorker makes
(Mailspring-Sync `MailSync/GoogleContactsWorker.cpp`, TaskProcessor's contact and
contact-group tasks), plus a CalDAV stub so the Gmail calendar pass has something to ask.

Endpoints, with the behaviour of the documented API (developers.google.com/people/api/rest):

  POST /token                                   refresh_token grant (XOAuth2TokenManager);
                                                issues access tokens the fake IMAP accepts
  GET  /v1/people/me/connections                personFields (required), pageSize (<= 1000),
                                                pageToken, requestSyncToken, syncToken
  GET  /v1/contactGroups                        pageSize, pageToken, syncToken
  GET  /v1/contactGroups/{id}?maxMembers=N      memberResourceNames
  POST /v1/contactGroups                        {"contactGroup": {"name"}}
  PUT  /v1/contactGroups/{id}                   {"contactGroup": {..., "etag"}}
  DELETE /v1/contactGroups/{id}
  POST /v1/contactGroups/{id}/members:modify    resourceNamesToAdd / resourceNamesToRemove
  POST /v1/people:createContact
  PATCH /v1/people/{id}:updateContact           updatePersonFields (required); 400
                                                FAILED_PRECONDITION when
                                                metadata.sources[].etag is stale
  DELETE /v1/people/{id}:deleteContact
  PROPFIND /caldav/v2/<email>                   a calendar home with no calendars

Sync tokens. A list request with requestSyncToken=true (connections) or any contactGroups
list returns `nextSyncToken` on its last page. A request with `syncToken` returns only what
changed after that token, deleted resources included as tombstones with
`metadata.deleted: true` ("resources deleted since the last sync will be returned as a
person with PersonMetadata.deleted set to true"). `expire_sync_tokens()` makes every token
issued so far answer 400 FAILED_PRECONDITION "Sync token is expired. Clear local cache and
retry call without the sync token.", the text GoogleContactsWorker::paginateGoogleCollection
looks for. Pages are cut from a snapshot taken at the first request of a listing, so a page
token only has to carry an offset and that snapshot's sequence number; anything changed
while a listing is paged is reported after its sync token.

Every request lands in `requests` as (method, path, query, status) for `people_requests`.
"""
import base64
import json
import threading
import time
import urllib.parse
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Optional

CLIENT_ID = "harness-client-id.apps.googleusercontent.com"
REFRESH_TOKEN = "1//harness-refresh-token"
MAX_PAGE_SIZE = 1000
DEFAULT_CONNECTIONS_PAGE = 100
DEFAULT_GROUPS_PAGE = 30
SYSTEM_GROUPS = [("myContacts", "My Contacts"), ("starred", "Starred"), ("chatBuddies", "Chat contacts"),
                 ("all", "All Contacts"), ("friends", "Friends"), ("family", "Family"),
                 ("coworkers", "Coworkers"), ("blocked", "Blocked")]
PERSON_FIELDS = {"addresses", "ageRanges", "biographies", "birthdays", "calendarUrls", "clientData",
                 "coverPhotos", "emailAddresses", "events", "externalIds", "genders", "imClients",
                 "interests", "locales", "locations", "memberships", "metadata", "miscKeywords",
                 "names", "nicknames", "occupations", "organizations", "phoneNumbers", "photos",
                 "relations", "sipAddresses", "skills", "urls", "userDefined"}
EXPIRED_TOKEN_MESSAGE = "Sync token is expired. Clear local cache and retry call without the sync token."


class ApiError(Exception):
    def __init__(self, code: int, status: str, message: str, reason: Optional[str] = None):
        super().__init__(message)
        self.code, self.status, self.message, self.reason = code, status, message, reason

    def body(self) -> dict:
        err = {"code": self.code, "message": self.message, "status": self.status}
        if self.reason:
            err["details"] = [{"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": self.reason,
                               "domain": "googleapis.com"}]
        return {"error": err}


@dataclass
class Person:
    id: str
    fields: dict                       # personFields -> list of field objects
    memberships: set = field(default_factory=set)   # group ids
    seq: int = 0
    etag_n: int = 0
    deleted: bool = False

    @property
    def resource_name(self) -> str:
        return f"people/{self.id}"

    @property
    def etag(self) -> str:
        return "%" + base64.b64encode(f"{self.id}:{self.etag_n}".encode()).decode().rstrip("=")

    @property
    def email(self) -> str:
        return (self.fields.get("emailAddresses") or [{}])[0].get("value", "")

    @property
    def name(self) -> str:
        return (self.fields.get("names") or [{}])[0].get("displayName", "")

    def json(self, person_fields: Optional[set] = None) -> dict:
        source = {"type": "CONTACT", "id": self.id, "etag": self.etag,
                  "updateTime": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())}
        meta = {"sources": [source], "objectType": "PERSON"}
        if self.deleted:
            return {"resourceName": self.resource_name, "etag": self.etag, "metadata": {**meta, "deleted": True}}
        out = {"resourceName": self.resource_name, "etag": self.etag}
        wanted = person_fields if person_fields is not None else set(self.fields) | {"memberships", "metadata"}
        if "metadata" in wanted:
            out["metadata"] = meta
        for k, v in self.fields.items():
            if k in wanted and v:
                out[k] = [{**item, "metadata": {"primary": i == 0, "source": {"type": "CONTACT", "id": self.id}}}
                          for i, item in enumerate(v)]
        if "memberships" in wanted:
            out["memberships"] = [{"contactGroupMembership": {"contactGroupId": g, "contactGroupResourceName": f"contactGroups/{g}"},
                                   "metadata": {"source": {"type": "CONTACT", "id": self.id}}}
                                  for g in sorted(self.memberships | {"myContacts"})]
        return out


@dataclass
class Group:
    id: str
    name: str
    system: bool = False
    seq: int = 0
    etag_n: int = 0
    deleted: bool = False

    @property
    def resource_name(self) -> str:
        return f"contactGroups/{self.id}"

    @property
    def etag(self) -> str:
        return base64.b64encode(f"g{self.id}:{self.etag_n}".encode()).decode().rstrip("=")


class PeopleState:
    """The account's contacts and groups, shared by the HTTP handler and the scenario steps."""

    def __init__(self):
        self.lock = threading.RLock()
        self.seq = 0
        self.token_generation = 1
        self.people: dict = {}            # id -> Person
        self.groups: dict = {}            # id -> Group
        self._next_person = 100001
        self._next_group = 1
        for gid, name in SYSTEM_GROUPS:
            self.groups[gid] = Group(gid, name, system=True)

    def _bump(self) -> int:
        self.seq += 1
        return self.seq

    # -- what another client (contacts.google.com, a phone) does --------------------------------------

    def create_person(self, name: str = "", email: str = "", groups=(), **extra) -> Person:
        with self.lock:
            pid = f"c{self._next_person}"
            self._next_person += 1
            fields = {}
            if name:
                given, _, family = name.partition(" ")
                fields["names"] = [{"displayName": name, "givenName": given, "familyName": family,
                                    "displayNameLastFirst": f"{family}, {given}" if family else given}]
            emails = extra.pop("emails", None) or ([email] if email else [])
            if emails:
                fields["emailAddresses"] = [{"value": e, "type": "home"} for e in emails]
            if extra.get("phone"):
                fields["phoneNumbers"] = [{"value": extra["phone"], "type": "mobile"}]
            if extra.get("org"):
                fields["organizations"] = [{"name": extra["org"]}]
            p = Person(pid, fields, seq=self._bump(), etag_n=1)
            for g in groups:
                p.memberships.add(self.group_by_name(g).id)
            self.people[pid] = p
            for g in p.memberships:
                self._touch_group(self.groups[g])
            return p

    def update_person(self, p: Person, name: Optional[str] = None, email: Optional[str] = None, **extra):
        with self.lock:
            if name is not None:
                given, _, family = name.partition(" ")
                p.fields["names"] = [{"displayName": name, "givenName": given, "familyName": family}]
            if email is not None:
                p.fields["emailAddresses"] = [{"value": email, "type": "home"}]
            if extra.get("phone") is not None:
                p.fields["phoneNumbers"] = [{"value": extra["phone"], "type": "mobile"}]
            self._touch(p)

    def delete_person(self, p: Person):
        with self.lock:
            p.deleted = True
            for g in list(p.memberships):
                self._touch_group(self.groups[g])
            p.memberships.clear()
            self._touch(p)

    def create_group(self, name: str) -> Group:
        with self.lock:
            if any(g.name == name and not g.deleted for g in self.groups.values()):
                raise ApiError(409, "ALREADY_EXISTS", "Contact group name already exists.")
            gid = f"{0x3a5b7c00 + self._next_group:x}"
            self._next_group += 1
            g = Group(gid, name, seq=self._bump(), etag_n=1)
            self.groups[gid] = g
            return g

    def rename_group(self, g: Group, name: str):
        with self.lock:
            g.name = name
            self._touch_group(g)

    def delete_group(self, g: Group):
        with self.lock:
            g.deleted = True
            for p in self.people.values():
                if g.id in p.memberships:
                    p.memberships.discard(g.id)
                    self._touch(p)
            self._touch_group(g)

    def modify_members(self, g: Group, add=(), remove=()) -> list:
        with self.lock:
            not_found = []
            for rn in add:
                p = self.person_by_resource(rn)
                if p is None:
                    not_found.append(rn)
                elif g.id not in p.memberships:
                    p.memberships.add(g.id)
                    self._touch(p)
            for rn in remove:
                p = self.person_by_resource(rn)
                if p is None:
                    not_found.append(rn)
                elif g.id in p.memberships:
                    p.memberships.discard(g.id)
                    self._touch(p)
            self._touch_group(g)
            return not_found

    def expire_sync_tokens(self):
        with self.lock:
            self.token_generation += 1

    def _touch(self, p: Person):
        p.seq = self._bump()
        p.etag_n += 1

    def _touch_group(self, g: Group):
        g.seq = self._bump()
        g.etag_n += 1

    # -- lookups -----------------------------------------------------------------------------------------

    def live_people(self) -> list:
        return [p for p in self.people.values() if not p.deleted]

    def live_groups(self, user_only: bool = False) -> list:
        return [g for g in self.groups.values() if not g.deleted and not (user_only and g.system)]

    def members(self, g: Group) -> list:
        if g.id == "myContacts":
            return sorted(p.resource_name for p in self.live_people())
        return sorted(p.resource_name for p in self.live_people() if g.id in p.memberships)

    def person_by_email(self, email: str) -> Person:
        for p in self.live_people():
            if p.email.lower() == email.lower():
                return p
        raise KeyError(f"the fake People API has no contact {email}")

    def person_by_resource(self, rn: str) -> Optional[Person]:
        p = self.people.get(rn.split("/", 1)[-1])
        return p if p and not p.deleted else None

    def group_by_name(self, name: str) -> Group:
        for g in self.groups.values():
            if not g.deleted and (g.name == name or g.id == name):
                return g
        raise KeyError(f"the fake People API has no contact group {name}")

    def group_json(self, g: Group, max_members: int = 0) -> dict:
        out = {"resourceName": g.resource_name, "etag": g.etag,
               "metadata": {"updateTime": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())},
               "groupType": "SYSTEM_CONTACT_GROUP" if g.system else "USER_CONTACT_GROUP",
               "name": g.name, "formattedName": g.name}
        if g.deleted:
            out["metadata"]["deleted"] = True
            return out
        members = self.members(g)
        if members:
            out["memberCount"] = len(members)
        if max_members:
            out["memberResourceNames"] = members[:max_members]
        return out

    # -- sync tokens -------------------------------------------------------------------------------------

    def issue_token(self, kind: str, seq: int) -> str:
        raw = json.dumps({"k": kind, "s": seq, "g": self.token_generation}).encode()
        return base64.urlsafe_b64encode(raw).decode().rstrip("=")

    def read_token(self, kind: str, token: str) -> int:
        try:
            data = json.loads(base64.urlsafe_b64decode(token + "=" * (-len(token) % 4)))
        except Exception:
            raise ApiError(400, "INVALID_ARGUMENT", "Request contains an invalid argument.")
        if data.get("k") != kind:
            raise ApiError(400, "INVALID_ARGUMENT", "Request contains an invalid argument.")
        if data.get("g") != self.token_generation:
            raise ApiError(400, "FAILED_PRECONDITION", EXPIRED_TOKEN_MESSAGE, reason="EXPIRED_SYNC_TOKEN")
        return int(data["s"])


@dataclass
class Request:
    method: str
    path: str
    query: dict
    status: int
    body: Optional[dict] = None


class FakePeopleServer:
    def __init__(self, email: str = "test@example.test", rotate_refresh_token: bool = False,
                 expires_in="3599", tokens: Optional[set] = None):
        self.state = PeopleState()
        self.email = email
        self.client_id = CLIENT_ID
        self.refresh_tokens = {REFRESH_TOKEN}
        self.rotate_refresh_token = rotate_refresh_token
        # A string, as some token proxies return it; XOAuth2TokenManager accepts either.
        self.expires_in = expires_in
        self.access_tokens: set = tokens if tokens is not None else set()
        self.requests: list = []
        self.port = 0
        self._httpd: Optional[ThreadingHTTPServer] = None
        self._thread: Optional[threading.Thread] = None
        self._issued = 0
        # Listings in progress: page tokens refer to these snapshots.
        self._snapshots: dict = {}

    # -- lifecycle -----------------------------------------------------------------------------------

    def start(self):
        owner = self

        class Handler(_Handler):
            server_owner = owner
        self._httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._httpd.daemon_threads = True
        self.port = self._httpd.server_address[1]
        self._thread = threading.Thread(target=self._httpd.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True)
        self._thread.start()
        return self

    def stop(self):
        if self._httpd:
            self._httpd.shutdown()
            self._httpd.server_close()
            self._httpd = None

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def engine_env(self) -> dict:
        """The endpoint overrides the engine reads (TERN_ENGINE_* environment, see README). The
        fakes are plain http on 127.0.0.1; every https request goes to a dead proxy, so a build
        that ignores the overrides fails fast instead of sending the harness's refresh token
        to Google (curl and most HTTP clients honour https_proxy)."""
        dead = "http://127.0.0.1:9"
        return {"TERN_ENGINE_GOOGLE_TOKEN_URL": f"{self.base}/token",
                "TERN_ENGINE_GOOGLE_PEOPLE_ROOT": f"{self.base}/v1/",
                "TERN_ENGINE_GOOGLE_CALDAV_ROOT": self.base,
                "https_proxy": dead, "HTTPS_PROXY": dead}

    def account_oauth(self) -> dict:
        return {"refresh_token": REFRESH_TOKEN, "refresh_client_id": self.client_id}

    # -- request handling -----------------------------------------------------------------------------

    def token(self, form: dict) -> tuple:
        if form.get("grant_type") != "refresh_token" or form.get("client_id") != self.client_id:
            return 400, {"error": "invalid_client", "error_description": "The OAuth client was not found."}
        if form.get("refresh_token") not in self.refresh_tokens:
            return 400, {"error": "invalid_grant", "error_description": "Token has been expired or revoked."}
        self._issued += 1
        access = f"ya29.harness-{self._issued}-{int(time.time())}"
        self.access_tokens.add(access)
        out = {"access_token": access, "expires_in": self.expires_in, "token_type": "Bearer",
               "scope": "https://mail.google.com/ https://www.googleapis.com/auth/contacts"}
        if self.rotate_refresh_token and self._issued == 1:
            rotated = f"1//harness-refresh-token-rotated-{int(time.time())}"
            self.refresh_tokens = {rotated}
            out["refresh_token"] = rotated
        return 200, out

    def handle(self, method: str, path: str, query: dict, body: Optional[dict], auth: str) -> tuple:
        if path == "/token":
            return self.token(body or {})
        if path.startswith("/caldav/"):
            return self._caldav(method, path)
        if not path.startswith("/v1/"):
            return 404, {"error": {"code": 404, "message": "Not Found", "status": "NOT_FOUND"}}
        token = auth[len("Bearer "):] if auth.startswith("Bearer ") else ""
        if token not in self.access_tokens:
            raise ApiError(401, "UNAUTHENTICATED", "Request had invalid authentication credentials. Expected OAuth 2 "
                                                   "access token, login cookie or other valid authentication credential.")
        rest = path[len("/v1/"):]
        s = self.state
        with s.lock:
            if rest == "people/me/connections" and method == "GET":
                return 200, self._connections(query)
            if rest == "contactGroups" and method == "GET":
                return 200, self._groups(query)
            if rest == "contactGroups" and method == "POST":
                name = ((body or {}).get("contactGroup") or {}).get("name", "")
                if not name:
                    raise ApiError(400, "INVALID_ARGUMENT", "Contact group name is required.")
                return 200, s.group_json(s.create_group(name))
            if rest == "people:createContact" and method == "POST":
                return 200, self._create_contact(body or {}).json()
            if rest.startswith("contactGroups/"):
                return self._group_resource(method, rest[len("contactGroups/"):], query, body)
            if rest.startswith("people/"):
                return self._person_resource(method, rest[len("people/"):], query, body)
        return 404, {"error": {"code": 404, "message": "Requested entity was not found.", "status": "NOT_FOUND"}}

    def _page_size(self, query: dict, default: int) -> int:
        try:
            n = int(query.get("pageSize", default))
        except ValueError:
            raise ApiError(400, "INVALID_ARGUMENT", "Invalid pageSize.")
        if n < 0 or n > MAX_PAGE_SIZE:
            raise ApiError(400, "INVALID_ARGUMENT", f"pageSize must be between 0 and {MAX_PAGE_SIZE}.")
        return n or default

    def _listing(self, kind: str, query: dict, items_since) -> tuple:
        """(page items, next page token, next sync token or None, total) for a paged listing.
        `items_since(since_seq or None, upto_seq)` returns the snapshot's items in order."""
        s = self.state
        sync_token = query.get("syncToken")
        since = s.read_token(kind, sync_token) if sync_token else None
        page_token = query.get("pageToken")
        if page_token:
            snap = self._snapshots.get((kind, page_token))
            if snap is None:
                raise ApiError(400, "INVALID_ARGUMENT", "Request contains an invalid argument.")
            offset, upto, since = snap
        else:
            offset, upto = 0, s.seq
        items = items_since(since, upto)
        size = self._page_size(query, DEFAULT_CONNECTIONS_PAGE if kind == "people" else DEFAULT_GROUPS_PAGE)
        page = items[offset:offset + size]
        next_page = None
        if offset + size < len(items):
            next_page = base64.urlsafe_b64encode(f"{kind}:{upto}:{offset + size}:{time.time()}".encode()).decode()
            self._snapshots[(kind, next_page)] = (offset + size, upto, since)
        return page, next_page, s.issue_token(kind, upto), len(items)

    def _connections(self, query: dict) -> dict:
        fields = query.get("personFields")
        if not fields:
            raise ApiError(400, "INVALID_ARGUMENT", "personFields mask is required. Please specify one or more valid paths.")
        wanted = set(fields.split(","))
        unknown = wanted - PERSON_FIELDS
        if unknown:
            raise ApiError(400, "INVALID_ARGUMENT", f"Invalid personFields mask path: \"{sorted(unknown)[0]}\".")
        s = self.state

        def items(since, upto):
            if since is None:
                chosen = [p for p in s.people.values() if not p.deleted and p.seq <= upto]
            else:
                chosen = [p for p in s.people.values() if since < p.seq <= upto]
            return sorted(chosen, key=lambda p: p.seq)
        page, next_page, sync, total = self._listing("people", query, items)
        out = {"connections": [p.json(wanted) for p in page], "totalPeople": total, "totalItems": total}
        if next_page:
            out["nextPageToken"] = next_page
        elif query.get("syncToken") or query.get("requestSyncToken") == "true":
            out["nextSyncToken"] = sync
        if not out["connections"]:
            del out["connections"]
        return out

    def _groups(self, query: dict) -> dict:
        s = self.state

        def items(since, upto):
            if since is None:
                chosen = [g for g in s.groups.values() if not g.deleted and g.seq <= upto]
            else:
                chosen = [g for g in s.groups.values() if since < g.seq <= upto]
            return sorted(chosen, key=lambda g: (not g.system, g.seq, g.id))
        page, next_page, sync, total = self._listing("groups", query, items)
        out = {"contactGroups": [s.group_json(g) for g in page], "totalItems": total}
        if next_page:
            out["nextPageToken"] = next_page
        else:
            out["nextSyncToken"] = sync
        return out

    def _group_resource(self, method: str, rest: str, query: dict, body: Optional[dict]) -> tuple:
        s = self.state
        gid, _, verb = rest.partition("/")
        g = s.groups.get(gid)
        if g is None or g.deleted:
            raise ApiError(404, "NOT_FOUND", "Requested entity was not found.")
        if verb == "members:modify" and method == "POST":
            if g.id in ("myContacts",) or (g.system and g.id not in ("starred",)):
                raise ApiError(400, "FAILED_PRECONDITION", "Cannot modify members of a system contact group.")
            body = body or {}
            not_found = s.modify_members(g, body.get("resourceNamesToAdd") or [], body.get("resourceNamesToRemove") or [])
            return 200, ({"notFoundResourceNames": not_found} if not_found else {})
        if verb:
            raise ApiError(404, "NOT_FOUND", "Requested entity was not found.")
        if method == "GET":
            return 200, s.group_json(g, max_members=int(query.get("maxMembers", 0) or 0))
        if method == "PUT":
            cg = (body or {}).get("contactGroup") or {}
            if g.system:
                raise ApiError(400, "FAILED_PRECONDITION", "Cannot update a system contact group.")
            if cg.get("etag") and cg["etag"] != g.etag:
                raise ApiError(400, "FAILED_PRECONDITION", "Request contactGroup.etag is different than the current "
                                                           "contactGroup.etag. Clear local cache and get the latest contactGroup.",
                               reason="failedPrecondition")
            if cg.get("name"):
                s.rename_group(g, cg["name"])
            return 200, s.group_json(g)
        if method == "DELETE":
            if g.system:
                raise ApiError(400, "FAILED_PRECONDITION", "Cannot delete a system contact group.")
            s.delete_group(g)
            return 200, {}
        raise ApiError(405, "UNIMPLEMENTED", f"{method} is not supported here.")

    def _person_resource(self, method: str, rest: str, query: dict, body: Optional[dict]) -> tuple:
        s = self.state
        pid, _, verb = rest.partition(":")
        p = s.people.get(pid)
        if p is None or p.deleted:
            raise ApiError(404, "NOT_FOUND", "Requested entity was not found.")
        if verb == "updateContact" and method == "PATCH":
            fields = query.get("updatePersonFields")
            if not fields:
                raise ApiError(400, "INVALID_ARGUMENT", "updatePersonFields mask is required. Please specify one or more valid paths.")
            body = body or {}
            # The etag the client read: metadata.sources[CONTACT].etag as documented, or the
            # person's top-level etag, which is what a client that stored the listed person
            # (GoogleContactsWorker stores `info` = the connection JSON) sends back.
            sources = ((body.get("metadata") or {}).get("sources")) or []
            contact = next((x for x in sources if x.get("type") == "CONTACT"), None)
            etag = contact.get("etag") if contact else body.get("etag")
            if not etag:
                raise ApiError(400, "INVALID_ARGUMENT", "Request must set person.etag or person.metadata.sources "
                                                        "for the contact to update.")
            if etag != p.etag:
                raise ApiError(400, "FAILED_PRECONDITION", "Request person.etag is different than the current person.etag. "
                                                           "Clear local cache and get the latest person.",
                               reason="failedPrecondition")
            for f in fields.split(","):
                if f not in PERSON_FIELDS:
                    raise ApiError(400, "INVALID_ARGUMENT", f"Invalid updatePersonFields mask path: \"{f}\".")
                if f in ("memberships", "metadata", "photos"):
                    continue
                p.fields[f] = [{k: v for k, v in item.items() if k != "metadata"} for item in body.get(f) or []]
            s._touch(p)
            return 200, p.json()
        if verb == "deleteContact" and method == "DELETE":
            s.delete_person(p)
            return 200, {}
        if not verb and method == "GET":
            return 200, p.json(set(query.get("personFields", "names,emailAddresses").split(",")))
        raise ApiError(404, "NOT_FOUND", "Requested entity was not found.")

    def _create_contact(self, body: dict):
        s = self.state
        fields = {k: [{kk: vv for kk, vv in item.items() if kk != "metadata"} for item in v]
                  for k, v in body.items() if k in PERSON_FIELDS and k not in ("metadata", "memberships", "photos") and isinstance(v, list)}
        p = s.create_person()
        p.fields.update(fields)
        s._touch(p)
        return p

    def _caldav(self, method: str, path: str) -> tuple:
        # The Gmail calendar pass PROPFINDs apidata.googleusercontent.com/caldav/v2/<email>;
        # answering with the home alone gives it no calendars to sync.
        if method != "PROPFIND":
            return 405, {"error": "method not allowed"}
        xml = ('<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:"><D:response>'
               f'<D:href>{path}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype>'
               '</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>')
        return 207, xml

    def requests_matching(self, method: Optional[str] = None, path: Optional[str] = None) -> list:
        import re
        rx = re.compile(path) if path else None
        return [r for r in list(self.requests) if (method is None or r.method == method) and (rx is None or rx.search(r.path))]


class _Handler(BaseHTTPRequestHandler):
    server_owner: FakePeopleServer = None
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def _dispatch(self):
        owner = self.server_owner
        url = urllib.parse.urlsplit(self.path)
        query = {k: v[-1] for k, v in urllib.parse.parse_qs(url.query, keep_blank_values=True).items()}
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        body = None
        if raw:
            ctype = self.headers.get("Content-Type", "")
            if "x-www-form-urlencoded" in ctype or url.path == "/token":
                body = {k: v[-1] for k, v in urllib.parse.parse_qs(raw.decode()).items()}
            else:
                try:
                    body = json.loads(raw)
                except ValueError:
                    body = None
        try:
            status, payload = owner.handle(self.command, url.path, query, body, self.headers.get("Authorization", ""))
        except ApiError as e:
            status, payload = e.code, e.body()
        except KeyError as e:
            status, payload = 404, {"error": {"code": 404, "message": str(e), "status": "NOT_FOUND"}}
        owner.requests.append(Request(self.command, url.path, query, status, body))
        if isinstance(payload, str):
            data, ctype = payload.encode(), "application/xml; charset=utf-8"
        else:
            data, ctype = json.dumps(payload).encode(), "application/json; charset=UTF-8"
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_PROPFIND = _dispatch
