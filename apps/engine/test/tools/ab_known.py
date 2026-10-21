"""
Classification of the differences `ab.py batch` finds between the Rust engine and the C++
reference binary. Each rule names where the expected behaviour is written down.

Categories:
  deliberate  a documented divergence says the engines differ
  cpp-older   the reference binary (Mailspring.app 1.26.0) predates an upstream Mailspring-Sync
              fix the Rust engine has
  timing      the harness snapshot caught the engines at different moments (also detected
              automatically: a difference that does not repeat on a rerun)
  real        a behaviour difference; the reason names the C++ source line that defines it

A rule is (scenario regex, server regex, table regex, key regex, field regex, category,
reason). `None` matches anything; a field regex never matches a whole-row entry unless it
is "(row)".
"""
import re

CPP_NO_OVERRIDES = ("The C++ binary does not read the harness endpoint overrides (TERN_ENGINE_CALDAV_HOST, "
                    "TERN_ENGINE_CARDDAV_HOST, TERN_ENGINE_GOOGLE_*), so it never reaches the fake People API or "
                    "the DAV servers.")
CPP_NO_IMPORT = ("`--mode import-legacy` exists only in the Rust engine; "
                 "the C++ binary rejects the mode.")
FRESH_DB = ("The Rust `migrate` moves a C++ database aside and starts a fresh V12 database "
            "on purpose; C++ keeps and upgrades it.")
SNIPPET = ("Snippet/body/search-text drift from replacing libtidy+libxml2 with html5ever.")

# Whole-case rules: (scenario regex, server regex, category, reason). They apply when the C++
# outcome is not PASS, and cover every difference in the case.
CASE_RULES = [
    (r"^(dav-|google-people-)", None, "deliberate", CPP_NO_OVERRIDES),
    (r"^import-legacy-", None, "deliberate", CPP_NO_IMPORT),
    (r"^fresh-db-replaces-legacy$", None, "deliberate", FRESH_DB),
]

RULES = [
    (None, None, r"^Message$", None, r"^data\.snippet$", "deliberate", SNIPPET),
    (None, None, r"^Thread$", None, r"^(snippet|data\.snippet)$", "deliberate", SNIPPET),
    (None, None, r"^MessageBody$", None, r"^value$", "deliberate", SNIPPET),
    (None, None, r"^_State$", r"^engine$", "(row)", "deliberate",
     "`_State.engine = rust` marks a Rust-written database so `migrate` can tell it from a C++ one."),
]

REPORT_NOTES = """
| Not compared | Why |
|---|---|
| Mail state in the `dav-*` and `google-people-*` cases on C++ | The C++ binary never reaches the fake Google OAuth/People endpoints or the DAV hosts, so the whole case is classified by its case rule; the Rust side is checked by the scenarios' own expectations |
| `--mode test` / `install-check` output | `apps/engine/tools/test_mode_ab.py` (52 match, 2 known `dovecot:tls` divergences) |
| Log lines and IMAP traffic | Tag shapes, capability text and wording differ by design; scenarios assert the lines they depend on |
| Save counters (`v`, `version`), FTS rowids, scan/vacuum/body-fetch clocks, UIDVALIDITY | Timing or server clock, not behaviour (`harness/snapshot.py` VOLATILE) |
| How often a model was streamed | Coalescing depends on timing; the report compares which models were persisted or unpersisted |

| Seen in an earlier batch, not in this one | Classification |
|---|---|
| `connection-dropped-during-command[fake:dovecot]` failed once on Rust (`expected connectionError to be reported, ProcessState deltas were [False]`), passed on its other run | Timing. `server.drop_connections` also drops the foreground's IDLE; the foreground reconnects within ~3 ms and its successful iteration calls `endConnectionError`, which coalesces with the background's `beginConnectionError` before the flush thread writes. C++ has the same race (main.cpp:150 and :218, DeltaStream.cpp:216-229). The Rust engine no longer has it: the foreground leaves the error in place after an IDLE that ended in an error (`tern-sync/src/foreground.rs`), and begin/end change the flag and queue the delta under one lock (`tern-store/src/delta.rs`) |
| `flag-change-on-second-placement[cyrus:fastmail]` ERROR on C++ (`docker run` exit 125) | Infrastructure; passed on the rerun |
"""


def _m(rx, value) -> bool:
    return rx is None or re.search(rx, value or "") is not None


def case_rule(scenario, server, rust_outcome, cpp_outcome):
    if cpp_outcome == "PASS" and rust_outcome == "PASS":
        return None
    for sc, sv, cat, reason in CASE_RULES:
        if _m(sc, scenario) and _m(sv, server):
            return cat, reason
    return None


def match(scenario, server, entry):
    field = entry["field"] if entry["field"] is not None else "(row)"
    for sc, sv, table, key, f, cat, reason in RULES:
        if f is not None and f != "(row)" and entry["field"] is None:
            continue
        if f == "(row)" and entry["field"] is not None:
            continue
        if (_m(sc, scenario) and _m(sv, server) and _m(table, entry["table"]) and _m(key, entry["key"])
                and (f in (None, "(row)") or _m(f, field))):
            return cat, reason
    return None

# A failed case whose detail matches this never reached the engine (Docker refused the
# container, a port was taken).
HARNESS_ERROR = re.compile(r"docker', 'run'[^\]]*\]' returned non-zero exit status \d+|Address already in use")

# Why a difference the rerun shows to be timing moves, where that is known.
TIMING_REASONS = [
    (r"^ThreadSearch$", r"^body$",
     "The thread's search body appends each message body as it is fetched (MailProcessor.cpp:497 "
     "`retrievedMessageBody` -> `appendToThreadSearchContent`, :725-762), and the background and foreground "
     "workers race to fetch bodies on Docker servers; the order changes between runs of either binary."),
]


def timing_reason(entry) -> str:
    for table, field, reason in TIMING_REASONS:
        if _m(table, entry["table"]) and _m(field, entry["field"]):
            return reason
    return "differs between two runs of the same binary, or not on the rerun"
