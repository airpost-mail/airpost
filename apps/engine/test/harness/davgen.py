"""
Deterministic iCalendar events and vCards for populating DAV servers and composing the
client's DAV tasks, the way mailgen does for messages.

Every generated item has a unique UID unless a scenario names one: the engine keys an
Event row on (calendar, UID, RECURRENCE-ID) (MailUtils::idForEvent) and a CardDAV Contact
row on the vCard UID (DAVWorker::ingestAddressDataNode), so two items sharing a UID would
collapse onto one row.

Event spec keys: uid, summary, description, location, start (ISO-8601, or "+2d" / "-3h"
relative to now), minutes (duration, default 60), all_day, rrule, attendees [emails],
organizer, sequence, status, overrides [{start, summary}] (RECURRENCE-ID instances in the
same resource).
Contact spec keys: uid, name, email (or emails), phone, org, note, version ("3.0" / "4.0").
Group spec keys: uid, name, members [contact uids], version ("3.0": X-ADDRESSBOOKSERVER-KIND,
as iCloud and Fastmail store groups; "4.0": KIND:group + MEMBER).
"""
import re
from datetime import datetime, timedelta, timezone
from typing import Optional

PRODID = "-//Airpost//engine harness//EN"
NOW = datetime.now(timezone.utc).replace(second=0, microsecond=0)
_REL = re.compile(r"^([+-]\d+(?:\.\d+)?)([dhm])$")


def when(value, base: datetime = NOW) -> datetime:
    """"+2d", "-3h", "+90m", an ISO-8601 string, an epoch number or a datetime."""
    if isinstance(value, datetime):
        return value.astimezone(timezone.utc)
    if isinstance(value, (int, float)):
        return datetime.fromtimestamp(value, timezone.utc)
    m = _REL.match(str(value))
    if m:
        n, unit = float(m.group(1)), m.group(2)
        return base + {"d": timedelta(days=n), "h": timedelta(hours=n), "m": timedelta(minutes=n)}[unit]
    dt = datetime.fromisoformat(str(value))
    return (dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)).astimezone(timezone.utc)


def _utc(dt: datetime) -> str:
    return dt.strftime("%Y%m%dT%H%M%SZ")


def _escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace(";", "\\;").replace(",", "\\,").replace("\n", "\\n")


def _fold(line: str) -> str:
    """RFC 5545 section 3.1: lines longer than 75 octets continue with a leading space."""
    raw = line.encode()
    if len(raw) <= 75:
        return line
    out, cur = [], b""
    for ch in line:
        b = ch.encode()
        if len(cur) + len(b) > (75 if not out else 74):
            out.append(cur.decode())
            cur = b""
        cur += b
    out.append(cur.decode())
    return "\r\n ".join(out)


def event_uid(n: int) -> str:
    return f"harness-event-{n}@example.test"


def contact_uid(n: int) -> str:
    return f"harness-contact-{n}"


def _vevent(spec: dict, uid: str, n: int, recurrence_id: Optional[datetime] = None) -> list:
    start = when(spec.get("start", f"+{n}d"))
    if spec.get("all_day"):
        start = start.replace(hour=0, minute=0)
    minutes = int(spec.get("minutes", 60))
    lines = ["BEGIN:VEVENT", f"UID:{uid}", f"DTSTAMP:{_utc(NOW)}"]
    if spec.get("all_day"):
        lines += [f"DTSTART;VALUE=DATE:{start.strftime('%Y%m%d')}",
                  f"DTEND;VALUE=DATE:{(start + timedelta(days=1)).strftime('%Y%m%d')}"]
    else:
        lines += [f"DTSTART:{_utc(start)}", f"DTEND:{_utc(start + timedelta(minutes=minutes))}"]
    if recurrence_id is not None:
        lines.append(f"RECURRENCE-ID:{_utc(recurrence_id)}")
    lines.append(f"SUMMARY:{_escape(spec.get('summary', f'Event {n}'))}")
    if spec.get("description"):
        lines.append(f"DESCRIPTION:{_escape(spec['description'])}")
    if spec.get("location"):
        lines.append(f"LOCATION:{_escape(spec['location'])}")
    if spec.get("status"):
        lines.append(f"STATUS:{spec['status']}")
    lines.append(f"SEQUENCE:{int(spec.get('sequence', 0))}")
    if spec.get("rrule") and recurrence_id is None:
        lines.append(f"RRULE:{spec['rrule']}")
    if spec.get("organizer"):
        lines.append(f"ORGANIZER:mailto:{spec['organizer']}")
    for a in spec.get("attendees") or []:
        lines.append(f"ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{a}")
    lines.append("END:VEVENT")
    return lines


def event(n: int, spec: Optional[dict] = None) -> tuple:
    """(uid, ics text) for event n of a series."""
    spec = dict(spec or {})
    uid = spec.get("uid") or event_uid(n)
    lines = ["BEGIN:VCALENDAR", "VERSION:2.0", f"PRODID:{PRODID}", "CALSCALE:GREGORIAN"]
    lines += _vevent(spec, uid, n)
    start = when(spec.get("start", f"+{n}d"))
    for ov in spec.get("overrides") or []:
        rid = when(ov["recurrence"], base=start) if "recurrence" in ov else start + timedelta(days=7)
        ov_spec = {**spec, **ov, "start": ov.get("start", rid)}
        lines += _vevent(ov_spec, uid, n, recurrence_id=rid)
    lines.append("END:VCALENDAR")
    return uid, "\r\n".join(_fold(l) for l in lines) + "\r\n"


def contact(n: int, spec: Optional[dict] = None) -> tuple:
    """(uid, vcf text) for contact n of a series."""
    spec = dict(spec or {})
    uid = spec.get("uid") or contact_uid(n)
    name = spec.get("name", f"Contact {n}")
    emails = spec.get("emails") or [spec.get("email", f"contact{n}@example.org")]
    version = str(spec.get("version", "3.0"))
    given, _, family = name.partition(" ")
    lines = ["BEGIN:VCARD", f"VERSION:{version}", f"PRODID:{PRODID}", f"UID:{uid}",
             f"FN:{_escape(name)}", f"N:{_escape(family)};{_escape(given)};;;"]
    for i, e in enumerate(emails):
        lines.append(f"EMAIL;TYPE=INTERNET{',pref' if i == 0 and version == '3.0' else ''}:{e}")
    if spec.get("phone"):
        lines.append(f"TEL;TYPE=CELL:{spec['phone']}")
    if spec.get("org"):
        lines.append(f"ORG:{_escape(spec['org'])}")
    if spec.get("note"):
        lines.append(f"NOTE:{_escape(spec['note'])}")
    lines.append(f"REV:{_utc(NOW)}")
    lines.append("END:VCARD")
    return uid, "\r\n".join(_fold(l) for l in lines) + "\r\n"


def group(n: int, spec: Optional[dict] = None) -> tuple:
    """(uid, vcf text) for a contact group card."""
    spec = dict(spec or {})
    uid = spec.get("uid") or f"harness-group-{n}"
    name = spec.get("name", f"Group {n}")
    version = str(spec.get("version", "3.0"))
    lines = ["BEGIN:VCARD", f"VERSION:{version}", f"PRODID:{PRODID}", f"UID:{uid}",
             f"FN:{_escape(name)}", f"N:{_escape(name)};;;;"]
    if version == "4.0":
        lines.append("KIND:group")
        lines += [f"MEMBER:urn:uuid:{m}" for m in spec.get("members") or []]
    else:
        lines.append("X-ADDRESSBOOKSERVER-KIND:group")
        lines += [f"X-ADDRESSBOOKSERVER-MEMBER:urn:uuid:{m}" for m in spec.get("members") or []]
    lines.append(f"REV:{_utc(NOW)}")
    lines.append("END:VCARD")
    return uid, "\r\n".join(_fold(l) for l in lines) + "\r\n"


# -- reading back ------------------------------------------------------------------------------

def unfold(text: str) -> list:
    out = []
    for line in text.replace("\r\n", "\n").split("\n"):
        if line[:1] in (" ", "\t") and out:
            out[-1] += line[1:]
        elif line:
            out.append(line)
    return out


def prop(line: str) -> tuple:
    """("NAME", {params}, value) for one unfolded content line."""
    head, _, value = line.partition(":")
    name, *params = head.split(";")
    pd = {}
    for p in params:
        k, _, v = p.partition("=")
        pd[k.upper()] = v
    return name.upper(), pd, value


def vevents(ics: str) -> list:
    """[{uid, recurrence_id, summary, dtstart}] for each VEVENT in an iCalendar object."""
    out, cur = [], None
    for line in unfold(ics):
        name, params, value = prop(line)
        if name == "BEGIN" and value.upper() == "VEVENT":
            cur = {"uid": "", "recurrence_id": "", "summary": "", "dtstart": ""}
        elif name == "END" and value.upper() == "VEVENT" and cur is not None:
            out.append(cur)
            cur = None
        elif cur is not None and name in ("UID", "RECURRENCE-ID", "SUMMARY", "DTSTART"):
            cur[name.lower().replace("-", "_")] = value
    return out


def vcard_fields(vcf: str) -> dict:
    """{uid, fn, emails, is_group, members} of one vCard, as DAVWorker reads it
    (DAVUtils::isGroupCard; members from MEMBER and X-ADDRESSBOOKSERVER-MEMBER, urn:uuid:
    stripped)."""
    out = {"uid": "", "fn": "", "n": "", "emails": [], "is_group": False, "members": []}
    for line in unfold(vcf):
        name, params, value = prop(line)
        name = name.split(".")[-1]   # item1.EMAIL grouping
        if name == "UID":
            out["uid"] = value
        elif name == "FN":
            out["fn"] = value.replace("\\,", ",").replace("\\;", ";")
        elif name == "N":
            out["n"] = value
        elif name == "EMAIL":
            out["emails"].append(value)
        elif (name == "KIND" or name == "X-ADDRESSBOOKSERVER-KIND") and value.lower() == "group":
            out["is_group"] = True
        elif name in ("MEMBER", "X-ADDRESSBOOKSERVER-MEMBER"):
            out["members"].append(value[len("urn:uuid:"):] if value.startswith("urn:uuid:") else value)
    return out


def with_props(text: str, component: str, changes: dict) -> str:
    """`text` with properties of the first `component` (VEVENT or VCARD) replaced, e.g.
    {"SUMMARY": "Moved", "SEQUENCE": "1"}; a property not present is added before its END.
    How another client edits an item: everything else, the UID included, stays."""
    changes = {k.upper(): v for k, v in changes.items()}
    lines = unfold(text)
    out, inside, done, seen = [], False, False, set()
    for line in lines:
        name, params, value = prop(line)
        if name == "BEGIN" and value.upper() == component and not done:
            inside = True
        if inside and name == "END" and value.upper() == component:
            for k, v in changes.items():
                if k not in seen and v is not None:
                    out.append(f"{k}:{v}")
            inside, done = False, True
        if inside and name in changes:
            seen.add(name)
            v = changes[name]
            if v is not None:
                out.append(f"{name}:{v}")
            continue
        out.append(line)
    return "\r\n".join(_fold(l) for l in out) + "\r\n"
