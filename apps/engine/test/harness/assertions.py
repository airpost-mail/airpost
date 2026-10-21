"""Comparisons between the engine's database and the server, phrased as lists of
human-readable differences so a failing scenario says exactly what diverged."""
from typing import Iterable, Optional

from .db import Placements
from .servers.base import Truth, normalize_message_id

# The engine models exactly these IMAP flags (unread, starred, draft). \Deleted, \Answered
# and keywords are not stored as flags, so they cannot be compared; label keywords are
# compared as labels (`labels: true`).
TRACKED_FLAGS = {"\\Seen", "\\Flagged", "\\Draft"}

# tern_core::keywords: which keywords are labels off Gmail.
_NON_LABEL_KEYWORDS = {"junk", "nonjunk", "notjunk", "junkrecorded", "redirected", "forwarded", "old"}
_LABEL_DOLLAR_KEYWORDS = {"$label1", "$label2", "$label3", "$label4", "$label5", "$important"}


def label_keywords(flags: Iterable[str]) -> set:
    """The label keywords among a copy's flags, lowercased, as the engine records them."""
    out = set()
    for f in flags:
        k = f.lower()
        if k.startswith("\\"):
            continue
        if k in _LABEL_DOLLAR_KEYWORDS or (not k.startswith("$") and k not in _NON_LABEL_KEYWORDS):
            out.add(k)
    return out


def compare_placements(local: Placements, truth: Truth, mailboxes: Optional[Iterable[str]] = None,
                       check_flags: bool = True, check_labels: bool = False, ignore_empty: bool = True,
                       keyword_labels: bool = False) -> list:
    """`keyword_labels`: the server's labels are its label keywords, not X-GM-LABELS."""
    diffs = []
    names = set(mailboxes) if mailboxes else set(truth) | {p for p, u in local.items() if u or not ignore_empty}
    for mb in sorted(names):
        remote = truth.get(mb)
        mine = local.get(mb, {})
        if remote is None:
            if mine:
                diffs.append(f"{mb}: engine has {len(mine)} messages in a folder the server does not have")
            continue
        missing = sorted(set(remote) - set(mine))
        extra = sorted(set(mine) - set(remote))
        if missing:
            diffs.append(f"{mb}: {len(missing)} server UIDs missing locally: {_short(missing)}")
        if extra:
            diffs.append(f"{mb}: {len(extra)} local UIDs not on server (ghosts): {_short(extra)}")
        for uid in sorted(set(remote) & set(mine)):
            r, l = remote[uid], mine[uid]
            if normalize_message_id(l.header_message_id) != normalize_message_id(r["message_id"]):
                diffs.append(f"{mb} UID {uid}: Message-ID {l.header_message_id!r} locally vs {r['message_id']!r} on server")
            if check_flags and set(l.flags) & TRACKED_FLAGS != set(r["flags"]) & TRACKED_FLAGS:
                diffs.append(f"{mb} UID {uid}: flags {sorted(set(l.flags) & TRACKED_FLAGS)} locally vs "
                             f"{sorted(set(r['flags']) & TRACKED_FLAGS)} on server")
            want = label_keywords(r["flags"]) if keyword_labels else set(r.get("labels", ()))
            if check_labels and set(l.labels) != want:
                diffs.append(f"{mb} UID {uid}: labels {sorted(l.labels)} locally vs {sorted(want)} on server")
    return diffs


def placement_changes(before: Placements, after: Placements) -> list:
    """Which (folder, uid) placements appeared, vanished or changed message between two snapshots."""
    changes = []
    for mb in sorted(set(before) | set(after)):
        b, a = before.get(mb, {}), after.get(mb, {})
        for uid in sorted(set(b) - set(a)):
            changes.append(f"{mb} UID {uid} ({b[uid].header_message_id}) vanished")
        for uid in sorted(set(a) - set(b)):
            changes.append(f"{mb} UID {uid} ({a[uid].header_message_id}) appeared")
        for uid in sorted(set(a) & set(b)):
            if a[uid].message_id != b[uid].message_id:
                changes.append(f"{mb} UID {uid} changed message {b[uid].message_id} -> {a[uid].message_id}")
    return changes


def _short(uids: list, n: int = 8) -> str:
    return ", ".join(str(u) for u in uids[:n]) + (f", … (+{len(uids) - n})" if len(uids) > n else "")
