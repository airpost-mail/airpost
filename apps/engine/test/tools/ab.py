#!/usr/bin/env python3
"""
Compare two engine binaries on the scenario catalog.

Outcome mode (one binary per recording; did any scenario start failing?):

    python3 test/tools/ab.py run --engine /path/to/tern-engine-before --out test/ab/before.json
    python3 test/tools/ab.py run --engine target/release/tern-engine  --out test/ab/after.json
    python3 test/tools/ab.py compare test/ab/before.json test/ab/after.json

State mode (what did each binary leave in its database and on its delta stream?):

    # one case on one binary, then diff two such files
    python3 test/tools/ab.py case baseline-initial-sync --server fake:dovecot --engine BIN --out a.json
    python3 test/tools/ab.py diff a.json b.json
    # every case on both binaries, in parallel, with a report (test/tools/ab/report.md)
    python3 test/tools/ab.py batch            # Rust release build vs Mailspring.app's C++ engine

`batch` runs each (scenario, server) case once per binary, reruns every case with a
difference no rule in ab_known.py explains, and classifies each difference: a difference
that does not reproduce on the rerun, or that a binary shows against itself, is harness
timing. `report` reclassifies a finished batch after ab_known.py changes. Rows are matched by
headerMessageId and other binary-neutral keys (harness/snapshot.py); ids differ by design.

Outcomes per case: PASS, FAIL, XFAIL (a known bug failed as expected), XPASS (a known bug
no longer reproduces), SKIP, ERROR. `compare` lists every case whose outcome changed and
exits non-zero if any change is a regression (PASS->FAIL/ERROR, XPASS->FAIL, or a new
failure in a scenario that passed before).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
REPO = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from harness import mailgen  # noqa: E402
from harness.scenario import (ScenarioRun, ScenarioSkipped, ServerSpec, available_server_kinds,  # noqa: E402
                              cpp_binary, discover, load_scenario)
from harness.snapshot import VOLATILE, snapshot  # noqa: E402

REGRESSION = {("PASS", "FAIL"), ("PASS", "ERROR"), ("XPASS", "FAIL"), ("XPASS", "XFAIL"),
              ("XFAIL", "ERROR"), ("PASS", "XFAIL")}
# Two relative timestamps closer than this are the same time (engine clocks, seeding).
TIME_TOLERANCE = 10
CASE_TIMEOUT = 1500


# -- outcome mode -------------------------------------------------------------------------------

def outcome_of(run: ScenarioRun, failures: list) -> tuple:
    if failures:
        return "FAIL", "\n".join(failures)[:2000]
    if run.xfails:
        # XPASS only when every xfail'd expectation passed; a partial flip is still XFAIL
        return "XFAIL", "; ".join(run.xfails) + ("; XPASS: " + "; ".join(run.xpasses) if run.xpasses else "")
    if run.xpasses:
        return "XPASS", "; ".join(run.xpasses)
    return "PASS", ""


def run_all(binary, servers, only, keep):
    kinds = set(servers.split(",")) if servers else available_server_kinds()
    results = {}
    for path in discover(HERE / "scenarios"):
        sc = load_scenario(path)
        if only and only not in sc["name"]:
            continue
        for entry in sc["servers"]:
            spec = ServerSpec.parse(entry)
            key = f"{sc['name']}[{spec.id}]"
            if spec.kind not in kinds:
                results[key] = {"outcome": "SKIP", "detail": f"server kind {spec.kind} unavailable"}
                continue
            t0 = time.time()
            run = ScenarioRun(sc, spec, binary=Path(binary) if binary else None, keep=keep)
            try:
                failures = run.run()
            except ScenarioSkipped as e:
                results[key] = {"outcome": "SKIP", "detail": str(e)}
                print(f"SKIP  {key}", flush=True)
                continue
            except Exception as e:  # harness error, not an engine verdict
                results[key] = {"outcome": "ERROR", "detail": f"{type(e).__name__}: {e}"}
                print(f"ERROR {key}: {e}", flush=True)
                continue
            outcome, detail = outcome_of(run, failures)
            results[key] = {"outcome": outcome, "detail": detail, "seconds": round(time.time() - t0, 1),
                            "xfails": run.xfails, "xpasses": run.xpasses}
            print(f"{outcome:5} {key} ({results[key]['seconds']}s)", flush=True)
    return results


def cmd_run(args):
    results = run_all(args.engine, args.servers, args.scenario, args.keep)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({"binary": args.engine, "recorded": time.strftime("%Y-%m-%d %H:%M:%S"),
                               "results": results}, indent=2) + "\n")
    tally = {}
    for r in results.values():
        tally[r["outcome"]] = tally.get(r["outcome"], 0) + 1
    print(f"\nwrote {out}: " + ", ".join(f"{k} {v}" for k, v in sorted(tally.items())))
    return 0


def cmd_compare(args):
    before = json.loads(Path(args.before).read_text())
    after = json.loads(Path(args.after).read_text())
    b, a = before["results"], after["results"]
    print(f"before: {before['binary']} ({before['recorded']})\nafter:  {after['binary']} ({after['recorded']})\n")
    changed, regressions = [], []
    for key in sorted(set(b) | set(a)):
        ob = b.get(key, {}).get("outcome", "-")
        oa = a.get(key, {}).get("outcome", "-")
        if ob != oa:
            changed.append((key, ob, oa))
            if (ob, oa) in REGRESSION or (oa in ("FAIL", "ERROR") and ob not in ("FAIL", "ERROR")):
                regressions.append((key, ob, oa))
        elif ob == oa == "FAIL" and b[key].get("detail") != a[key].get("detail"):
            changed.append((key, "FAIL", "FAIL (different failure)"))
    same = len(set(b) & set(a)) - len(changed)
    print(f"{same} unchanged, {len(changed)} changed, {len(regressions)} regressions\n")
    for key, ob, oa in changed:
        flag = "REGRESSION" if (key, ob, oa) in regressions else ("fixed" if oa in ("PASS", "XPASS") else "changed")
        print(f"{flag:10} {key}: {ob} -> {oa}")
        if args.verbose or flag == "REGRESSION":
            for label, res in (("before", b.get(key)), ("after", a.get(key))):
                if res and res.get("detail"):
                    print(f"    {label}: " + res["detail"].replace("\n", "\n            ")[:800])
    if not changed:
        print("no outcome changed")
    return 1 if regressions else 0


# -- state mode: one case ------------------------------------------------------------------------

class CapturingRun(ScenarioRun):
    """A ScenarioRun that snapshots the database and the delta stream once the engine has
    stopped, before teardown removes the artifacts."""

    captured = None
    engine_t0 = None

    def setup(self):
        super().setup()
        self.engine_t0 = time.time()

    def teardown(self):
        try:
            if self.ms:
                self.ms.stop()
                deltas = [{"class": d.model_class, "type": d.type, "models": d.models}
                          for d in self.earlier_deltas + self.ms.deltas()]
                t0 = mailgen.BASE.timestamp() + 3 * 86400   # mailgen dates are relative to this
                self.captured = snapshot(self.ms.db_path, deltas, t0, self.engine_t0 or t0)
        finally:
            super().teardown()


def find_scenario(name: str) -> Path:
    p = Path(name)
    if p.suffix == ".yaml" and p.exists():
        return p
    p = HERE / "scenarios" / f"{name}.yaml"
    if not p.exists():
        raise SystemExit(f"no scenario {name}")
    return p


def run_case(scenario: Path, server: str, binary: Path, keep: bool) -> dict:
    sc = load_scenario(scenario)
    wanted = ServerSpec.parse(server)
    spec = next((ServerSpec.parse(e) for e in sc["servers"] if ServerSpec.parse(e).id == wanted.id), wanted)
    run = CapturingRun(sc, spec, binary=binary, keep=keep)
    t0 = time.time()
    result = {"scenario": sc["name"], "server": spec.id, "binary": str(binary)}
    try:
        failures = run.run()
        result["outcome"], result["detail"] = outcome_of(run, failures)
    except ScenarioSkipped as e:
        result["outcome"], result["detail"] = "SKIP", str(e)
    except Exception as e:  # harness error, not an engine verdict
        result["outcome"], result["detail"] = "ERROR", f"{type(e).__name__}: {e}"
    result["seconds"] = round(time.time() - t0, 1)
    result["artifacts"] = str(run.work) if run.work.exists() else None
    result["snapshot"] = run.captured
    return result


def cmd_case(args):
    result = run_case(find_scenario(args.scenario), args.server, Path(args.engine).resolve(), args.keep)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result) + "\n")
    print(f"{result['outcome']:5} {result['scenario']}[{result['server']}] ({result['seconds']}s) -> {out}")
    return 0


# -- state mode: diffing -------------------------------------------------------------------------

def _same(a, b) -> bool:
    if isinstance(a, dict) and isinstance(b, dict) and "$t" in a and "$t" in b:
        return (abs(a["$t"] - b["$t"]) <= TIME_TOLERANCE
                or abs(a.get("$e", a["$t"]) - b.get("$e", b["$t"])) <= TIME_TOLERANCE)
    return a == b


def _short(v, n=160):
    if isinstance(v, dict) and "$t" in v:
        return f"t0{v['$t']:+d}s/launch{v.get('$e', v['$t']):+d}s"
    s = v if isinstance(v, str) else json.dumps(v, sort_keys=True)
    s = s.replace("\r", "\\r").replace("\n", "\\n").replace("|", "\\|")
    return s if len(s) <= n else s[:n - 3] + "..."


def diff_snapshots(a: dict, b: dict) -> list:
    """Entries {table, key, field, a, b}; field None means the row exists on one side only
    (its value is a summary of that row). `table` "delta" is the streamed-model set."""
    out = []
    if not a or not b:
        return out
    for table in sorted(set(a["db"]) | set(b["db"])):
        ra, rb = a["db"].get(table, {}), b["db"].get(table, {})
        for key in sorted(set(ra) | set(rb)):
            if key not in rb:
                out.append({"table": table, "key": key, "field": None, "a": "present", "b": "absent"})
            elif key not in ra:
                out.append({"table": table, "key": key, "field": None, "a": "absent", "b": "present"})
            else:
                fa, fb = ra[key], rb[key]
                for f in sorted(set(fa) | set(fb)):
                    if VOLATILE.search(f):   # also for snapshots recorded before a field was dropped
                        continue
                    va, vb = fa.get(f, "<missing>"), fb.get(f, "<missing>")
                    if not _same(va, vb):
                        out.append({"table": table, "key": key, "field": f, "a": va, "b": vb})
    da, db = set(a["deltas"]), set(b["deltas"])
    for d in sorted(da ^ db):
        out.append({"table": "delta", "key": d, "field": None,
                    "a": "streamed" if d in da else "not streamed", "b": "streamed" if d in db else "not streamed"})
    return out


def _sig(e) -> str:
    return json.dumps([e["table"], e["key"], e["field"], e["a"], e["b"]], sort_keys=True, default=str)


def _loc(e) -> tuple:
    return (e["table"], e["key"], e["field"])


def cmd_diff(args):
    a = json.loads(Path(args.a).read_text())
    b = json.loads(Path(args.b).read_text())
    print(f"A: {a['binary']} {a['outcome']}\nB: {b['binary']} {b['outcome']}")
    entries = diff_snapshots(a.get("snapshot"), b.get("snapshot"))
    for e in entries:
        print(f"{e['table']:22} {e['key']}  {e['field'] or '(row)'}: A={_short(e['a'])}  B={_short(e['b'])}")
    print(f"\n{len(entries)} differences")
    return 1 if entries or a["outcome"] != b["outcome"] else 0


# -- state mode: batch ---------------------------------------------------------------------------

def list_cases(servers, only) -> list:
    kinds = set(servers.split(",")) if servers else available_server_kinds()
    cases = []
    for path in discover(HERE / "scenarios"):
        sc = load_scenario(path)
        if only and not re.search(only, sc["name"]):
            continue
        for entry in sc["servers"]:
            spec = ServerSpec.parse(entry)
            cases.append({"scenario": sc["name"], "path": str(path), "server": spec.id,
                          "available": spec.kind in kinds})
    return cases


def case_id(c) -> str:
    return f"{c['scenario']}[{c['server']}]"


def _durations() -> dict:
    try:
        d = json.loads((HERE / "durations.json").read_text())
    except (FileNotFoundError, ValueError):
        return {}
    return {k.removeprefix("test_scenarios.py::test_scenario[")[:-1]: v for k, v in d.items()}


def _job(case, label, binary, rep, out_dir: Path, resume: bool) -> dict:
    cid = case_id(case)
    fname = re.sub(r"[^A-Za-z0-9_.-]", "_", cid) + ".json"
    out = out_dir / "cases" / f"{label}-r{rep}" / fname
    if resume and out.exists():
        return json.loads(out.read_text())
    env = dict(os.environ, HARNESS_RUNS_DIR=str(out_dir / "runs" / f"{label}-r{rep}"))
    cmd = [sys.executable, __file__, "case", case["path"], "--server", case["server"],
           "--engine", str(binary), "--out", str(out)]
    try:
        p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=CASE_TIMEOUT)
        if out.exists():
            return json.loads(out.read_text())
        detail = (p.stdout + p.stderr)[-2000:]
    except subprocess.TimeoutExpired:
        detail = f"case timed out after {CASE_TIMEOUT}s"
    result = {"scenario": case["scenario"], "server": case["server"], "binary": str(binary),
              "outcome": "ERROR", "detail": detail, "snapshot": None, "seconds": None}
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result) + "\n")
    return result


def _run_round(cases, binaries, rep, out_dir, jobs, resume, results):
    dur = _durations()
    work = [(c, label, binary) for c in cases for label, binary in binaries.items()]
    work.sort(key=lambda w: -dur.get(case_id(w[0]), 30.0))
    done = 0
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        futs = {pool.submit(_job, c, label, b, rep, out_dir, resume): (c, label) for c, label, b in work}
        for f in as_completed(futs):
            c, label = futs[f]
            r = f.result()
            results.setdefault(case_id(c), {})[f"{label}-r{rep}"] = r
            done += 1
            print(f"[r{rep} {done:3}/{len(work)}] {r['outcome']:5} {label:4} {case_id(c)} ({r.get('seconds')}s)", flush=True)


def classify_case(cid, runs: dict, known) -> dict:
    """The case's outcome pair and every A/B difference with its category."""
    a1, b1 = runs.get("rust-r1"), runs.get("cpp-r1")
    a2, b2 = runs.get("rust-r2"), runs.get("cpp-r2")
    scenario, server = a1["scenario"], a1["server"]
    entries = diff_snapshots(a1.get("snapshot"), b1.get("snapshot"))
    case_rule = known.case_rule(scenario, server, a1["outcome"], b1["outcome"])
    rerun = bool(a2 and b2)
    if rerun:
        second = {_sig(e) for e in diff_snapshots(a2.get("snapshot"), b2.get("snapshot"))}
        unstable = {_loc(e) for e in diff_snapshots(a1.get("snapshot"), a2.get("snapshot"))}
        unstable |= {_loc(e) for e in diff_snapshots(b1.get("snapshot"), b2.get("snapshot"))}
    for e in entries:
        if case_rule:
            e["category"], e["reason"] = case_rule
            e["whole_case"] = True
            continue
        rule = known.match(scenario, server, e)
        if rule and rule[0] != "real":
            e["category"], e["reason"] = rule
        elif rerun and (_sig(e) not in second or _loc(e) in unstable):
            e["category"], e["reason"] = "timing", known.timing_reason(e)
        elif rule:
            e["category"], e["reason"] = rule
        else:
            e["category"], e["reason"] = "unexplained", ""
    outcomes = {"rust": a1["outcome"], "cpp": b1["outcome"]}
    if rerun:
        outcomes.update({"rust2": a2["outcome"], "cpp2": b2["outcome"]})
    outcome_entry = None
    if a1["outcome"] != b1["outcome"] or (rerun and a2["outcome"] != b2["outcome"]):
        flips = rerun and (a1["outcome"] != a2["outcome"] or b1["outcome"] != b2["outcome"])
        failed = [r for r in (a1, b1, a2, b2) if r and r["outcome"] in ("FAIL", "ERROR")]
        if case_rule:
            cat, reason = case_rule
        elif flips and any(known.HARNESS_ERROR.search(r.get("detail") or "") for r in failed):
            cat, reason = "timing", "the harness could not start the server (" + known.HARNESS_ERROR.search(
                next(r["detail"] for r in failed if known.HARNESS_ERROR.search(r.get("detail") or ""))).group(0) + "); passes on the rerun"
        elif flips:
            cat, reason = "timing", "outcome flips between runs of the same binary: " + "; ".join(
                f"{r['outcome']} {_short(r.get('detail') or '', 160)}" for r in failed)
        else:
            cat, reason = "unexplained", ""
        outcome_entry = {"category": cat, "reason": reason,
                         "rust_detail": "; ".join((r.get("detail") or "")[:300] for r in (a1, a2) if r),
                         "cpp_detail": "; ".join((r.get("detail") or "")[:300] for r in (b1, b2) if r)}
    return {"case": cid, "scenario": scenario, "server": server, "outcomes": outcomes,
            "outcome_diff": outcome_entry, "entries": entries,
            "artifacts": {k: v.get("artifacts") for k, v in runs.items()}}


def cmd_batch(args):
    import ab_known as known
    out_dir = Path(args.out_dir).resolve()
    binaries = {"rust": Path(args.a).resolve(), "cpp": Path(args.b).resolve()}
    for label, b in binaries.items():
        if not b.is_file():
            raise SystemExit(f"{label} binary not found: {b}")
    cases = list_cases(args.servers, args.scenario)
    runnable = [c for c in cases if c["available"]]
    print(f"{len(runnable)} cases ({len(cases) - len(runnable)} unavailable server kinds), "
          f"rust={binaries['rust']} cpp={binaries['cpp']}, {args.jobs} jobs", flush=True)
    t_start = time.time()
    if not args.resume and (out_dir / "cases").exists():
        shutil.rmtree(out_dir / "cases")
        shutil.rmtree(out_dir / "runs", ignore_errors=True)
    results: dict = {}
    _run_round(runnable, binaries, 1, out_dir, args.jobs, args.resume, results)
    if not args.no_rerun:
        # Rerun the cases with a difference no rule explains, to tell timing from behaviour.
        differing = []
        for c in runnable:
            first = classify_case(case_id(c), results[case_id(c)], known)
            if any(e.get("category") == "unexplained" for e in first["entries"] + [first["outcome_diff"] or {}]):
                differing.append(c)
        print(f"\nrerunning {len(differing)} cases with unexplained differences on both binaries", flush=True)
        _run_round(differing, binaries, 2, out_dir, args.jobs, args.resume, results)
    report = [classify_case(cid, runs, known) for cid, runs in sorted(results.items())]
    summary = {"recorded": time.strftime("%Y-%m-%d %H:%M"), "minutes": round((time.time() - t_start) / 60),
               "command": " ".join(["python3", "apps/engine/test/tools/ab.py", *sys.argv[1:]]),
               "binaries": {k: str(v) for k, v in binaries.items()},
               "unavailable": [case_id(c) for c in cases if not c["available"]], "cases": report}
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=1, default=str) + "\n")
    Path(args.report).write_text(render_report(summary, known))
    tally = {}
    for c in report:
        for e in c["entries"] + ([c["outcome_diff"]] if c["outcome_diff"] else []):
            tally[e["category"]] = tally.get(e["category"], 0) + 1
    print(f"\nwrote {args.report} and {out_dir / 'summary.json'}: " + ", ".join(f"{k} {v}" for k, v in sorted(tally.items())))
    return 1 if tally.get("unexplained") or tally.get("real") else 0


def cmd_report(args):
    import ab_known as known
    out_dir = Path(args.out_dir).resolve()
    old = json.loads((out_dir / "summary.json").read_text())
    results: dict = {}
    for d in sorted((out_dir / "cases").iterdir()):
        for f in d.glob("*.json"):
            r = json.loads(f.read_text())
            results.setdefault(f"{r['scenario']}[{r['server']}]", {})[d.name] = r
    old["cases"] = [classify_case(cid, runs, known) for cid, runs in sorted(results.items())]
    (out_dir / "summary.json").write_text(json.dumps(old, indent=1, default=str) + "\n")
    Path(args.report).write_text(render_report(old, known))
    print(f"wrote {args.report}")
    return 0


# -- report --------------------------------------------------------------------------------------

CATEGORY_TITLES = {
    "real": "Real behaviour differences",
    "unexplained": "Unexplained (not yet classified)",
    "cpp-older": "C++ reference binary older than an upstream fix",
    "deliberate": "Deliberate (a documented divergence)",
    "timing": "Harness timing or infrastructure",
}
CATEGORY_ORDER = ["real", "unexplained", "cpp-older", "deliberate", "timing"]


def _field_pattern(f):
    return re.sub(r"\[\d+\]", "[*]", f) if f else "(row)"


def _key_pattern(k):
    k = re.sub(r"M:[^ ]*?@[^ #]+(#\w+)?", "M:*", k)
    return re.sub(r"(Task:\w+)#\d+", r"\1#*", k)


def render_report(summary: dict, known) -> str:
    cases = summary["cases"]
    L = ["# Engine A/B report", "",
         "Rust release build against the C++ reference binary on the harness scenario catalog. "
         "Generated by `ab.py batch`; do not edit by hand. "
         "Classification rules: `apps/engine/test/tools/ab_known.py`.", "",
         "| | |", "|---|---|",
         f"| Command | `{summary['command']}` |",
         f"| Recorded | {summary['recorded']} ({summary['minutes']} min) |",
         f"| Rust | `{summary['binaries']['rust']}` |",
         f"| C++ | `{summary['binaries']['cpp']}` (Mailspring 1.26.0) |",
         f"| Cases | {len(cases)} run, {len(summary['unavailable'])} not run (server kind unavailable) |",
         "| Matching | rows by headerMessageId and other binary-neutral keys (`harness/snapshot.py`); times relative to the run start, within "
         f"{TIME_TOLERANCE} s; save counters, FTS rowids and the engine's own scan/vacuum/fetch clocks dropped |",
         "| Timing | every case with a difference no rule explains is run a second time on both binaries; a difference that does not repeat, or that one binary shows against itself, is timing |",
         ""]
    tally = {k: 0 for k in CATEGORY_ORDER}
    groups = {}
    for c in cases:
        if c["outcome_diff"]:
            tally[c["outcome_diff"]["category"]] += 1
        for e in c["entries"]:
            tally[e["category"]] += 1
            if e.get("whole_case"):
                # One line per scenario its case rule covers whole.
                g = (e["category"], c["scenario"], "(whole case)", "*", "*")
            elif e["category"] == "deliberate":
                # A field rule applies to every scenario alike: one line per rule and field.
                g = (e["category"], "*", e["table"], _key_pattern(e["key"]), _field_pattern(e["field"]))
            else:
                g = (e["category"], c["scenario"], e["table"], _key_pattern(e["key"]), _field_pattern(e["field"]))
            groups.setdefault(g, {"servers": set(), "scenarios": set(), "tables": set(), "n": 0, "example": e})
            groups[g]["tables"].add(f"delta {e['key'].split()[0]}" if e["table"] == "delta" else e["table"])
            groups[g]["servers"].add(c["server"])
            groups[g]["scenarios"].add(c["scenario"])
            groups[g]["n"] += 1
    L += ["## Totals", "", "| Category | Differences |", "|---|---|"]
    L += [f"| {CATEGORY_TITLES[k]} | {tally[k]} |" for k in CATEGORY_ORDER]
    L += ["", "Outcome differences count once per case; state differences once per field and row.", ""]

    L += ["## Cases", "", "| Case | Rust | C++ | Rerun (Rust / C++) | Outcome difference | " + " | ".join(CATEGORY_ORDER) + " |",
          "|---|---|---|---|---|" + "---|" * len(CATEGORY_ORDER)]
    for c in cases:
        o = c["outcomes"]
        per = {k: 0 for k in CATEGORY_ORDER}
        for e in c["entries"]:
            per[e["category"]] += 1
        od = c["outcome_diff"]["category"] if c["outcome_diff"] else ""
        rr = f"{o['rust2']} / {o['cpp2']}" if "rust2" in o else ""
        L.append(f"| {c['case']} | {o['rust']} | {o['cpp']} | {rr} | {od} | " + " | ".join(str(per[k] or "") for k in CATEGORY_ORDER) + " |")
    for cid in summary["unavailable"]:
        L.append(f"| {cid} | not run | not run | |" + " |" * (len(CATEGORY_ORDER) + 1))
    L.append("")

    L += ["## Outcome differences", "", "| Case | Rust (runs) | C++ (runs) | Category | Reason |", "|---|---|---|---|---|"]
    for c in cases:
        od = c["outcome_diff"]
        if od:
            o = c["outcomes"]
            L.append(f"| {c['case']} | {' / '.join(v for k, v in o.items() if k.startswith('rust'))} | "
                     f"{' / '.join(v for k, v in o.items() if k.startswith('cpp'))} | {od['category']} | "
                     f"{_short(od['reason'] or od['cpp_detail'] or od['rust_detail'], 300)} |")
    L.append("")

    for cat in CATEGORY_ORDER:
        rows = [(g, v) for g, v in groups.items() if g[0] == cat]
        if not rows:
            continue
        L += [f"## {CATEGORY_TITLES[cat]}", "",
              "| Scenario | Servers | Model | Key | Field | Rows | Rust (example) | C++ (example) | Reason |",
              "|---|---|---|---|---|---|---|---|---|"]
        for (cat_, scen, table, key, field), v in sorted(rows):
            e = v["example"]
            if scen == "*":
                scen = f"{len(v['scenarios'])} scenarios" if len(v["scenarios"]) > 3 else ", ".join(sorted(v["scenarios"]))
            servers = f"{len(v['servers'])} servers" if len(v["servers"]) > 4 else ", ".join(sorted(v["servers"]))
            if table == "(whole case)":
                L.append(f"| {scen} | {servers} | {', '.join(sorted(v['tables']))} | * | * | {v['n']} | | | {_short(e['reason'], 400)} |")
                continue
            L.append(f"| {scen} | {servers} | {table} | {_short(key, 80)} | {field} | {v['n']} | "
                     f"{_short(e['a'], 100)} | {_short(e['b'], 100)} | {_short(e['reason'], 400)} |")
        L.append("")
    if hasattr(known, "REPORT_NOTES"):
        L += ["## Notes", "", known.REPORT_NOTES.strip(), ""]
    return "\n".join(L)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="outcomes of every case on one binary")
    r.add_argument("--engine", help="binary to test (default: auto-discovered)")
    r.add_argument("--out", required=True)
    r.add_argument("--servers", help="fake,dovecot (default: whatever is available)")
    r.add_argument("--scenario", help="only scenarios whose name contains this")
    r.add_argument("--keep", action="store_true")
    r.set_defaults(fn=cmd_run)
    c = sub.add_parser("compare", help="outcome changes between two `run` recordings")
    c.add_argument("before")
    c.add_argument("after")
    c.add_argument("-v", "--verbose", action="store_true")
    c.set_defaults(fn=cmd_compare)
    k = sub.add_parser("case", help="one case on one binary: outcome plus DB/delta snapshot")
    k.add_argument("scenario", help="scenario name or .yaml path")
    k.add_argument("--server", required=True, help="kind:profile, e.g. fake:dovecot")
    k.add_argument("--engine", required=True)
    k.add_argument("--out", required=True)
    k.add_argument("--keep", action="store_true")
    k.set_defaults(fn=cmd_case)
    d = sub.add_parser("diff", help="state differences between two `case` files")
    d.add_argument("a")
    d.add_argument("b")
    d.set_defaults(fn=cmd_diff)
    b = sub.add_parser("batch", help="every case on both binaries, classified, with a report")
    b.add_argument("--a", default=str(REPO / "target" / "release" / "tern-engine"), help="Rust binary (default: target/release/tern-engine)")
    b.add_argument("--b", default=str(cpp_binary()), help="C++ binary (default: the harness's cpp_binary())")
    b.add_argument("--servers", help="server kinds (default: whatever is available)")
    b.add_argument("--scenario", help="regex on scenario names")
    b.add_argument("-j", "--jobs", type=int, default=16)
    b.add_argument("--out-dir", default=str(HERE / "ab" / "batch"))
    b.add_argument("--report", default=str(HERE / "ab" / "report.md"))
    b.add_argument("--resume", action="store_true", help="reuse case files already in --out-dir")
    b.add_argument("--no-rerun", action="store_true", help="skip the second run of differing cases")
    b.set_defaults(fn=cmd_batch)
    p = sub.add_parser("report", help="reclassify a finished batch (after editing ab_known.py) and rewrite the report")
    p.add_argument("--out-dir", default=str(HERE / "ab" / "batch"))
    p.add_argument("--report", default=str(HERE / "ab" / "report.md"))
    p.set_defaults(fn=cmd_report)
    args = ap.parse_args()
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
