import faulthandler
import json
import os
import signal
import sys
from pathlib import Path

import pytest

# `kill -USR1 <pid>` dumps every thread's stack, for when a wait looks stuck. Under xdist the
# pid is the worker's, which names its artifacts directory: test/runs/session-<pid>/.
faulthandler.register(signal.SIGUSR1, all_threads=True)

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

# Per-case wall time (setup + call + teardown) from the last `--record-durations` run, keyed
# by node id relative to this directory. It orders the run longest first; a case missing
# from it counts as DEFAULT_DURATION.
DURATIONS_FILE = HERE / "durations.json"
DEFAULT_DURATION = 30.0
# A quick-tier case slower than this is listed at the end of the run as a slow candidate
# (README "Tiers").
SLOW_THRESHOLD = 20.0


def pytest_addoption(parser):
    parser.addoption("--servers", default=None, help="comma-separated server kinds to run (fake,dovecot,cyrus,radicale,baikal); naming a Docker kind runs its cases whatever the tier")
    parser.addoption("--scenario", default=None, help="only scenarios whose name contains this")
    parser.addoption("--mailsync", default=None, help="path to the mailsync binary (else MAILSYNC_BIN / auto)")
    parser.addoption("--keep", action="store_true", help="keep run artifacts for passing scenarios too")
    parser.addoption("--slow", action="store_true", help="run only the slow tier (Docker servers, heavy seeding, long waits)")
    parser.addoption("--all", action="store_true", help="run both tiers")
    parser.addoption("--record-durations", action="store_true", help=f"merge this run's per-case durations into {DURATIONS_FILE.name}")


def _tier(config) -> str:
    if config.getoption("--all"):
        return "all"
    return "slow" if config.getoption("--slow") else "quick"


def explicit_servers(config):
    value = config.getoption("--servers")
    return set(value.split(",")) if value else None


def _key(config, nodeid: str) -> str:
    """Node id relative to this directory, whatever the rootdir."""
    prefix = os.path.relpath(HERE, config.rootpath)
    return nodeid if prefix == "." else nodeid.removeprefix(prefix.replace(os.sep, "/") + "/")


def _load_durations() -> dict:
    try:
        return json.loads(DURATIONS_FILE.read_text())
    except (FileNotFoundError, ValueError):
        return {}


def _docker_kinds(item) -> set:
    return {k for m in item.iter_markers("docker") for k in m.args}


def pytest_collection_modifyitems(session, config, items):
    """Select the tier, drop cases of server kinds `--servers` leaves out, skip Docker cases
    when Docker is missing, and order what is left longest first."""
    from harness.scenario import DOCKER_KINDS, available_server_kinds
    tier = _tier(config)
    servers = explicit_servers(config)
    keep, dropped = [], []
    for item in items:
        kinds = _docker_kinds(item)
        spec = getattr(item, "callspec", None) and item.callspec.params.get("server_spec")
        needed = ({spec.kind} if spec else set()) | kinds
        if servers is not None and needed and not needed & servers:
            dropped.append(item)
        elif servers is not None and kinds & servers & DOCKER_KINDS:
            keep.append(item)   # `--servers dovecot` runs dovecot's cases in any tier
        elif tier == "all" or (item.get_closest_marker("slow") is not None) == (tier == "slow"):
            keep.append(item)
        else:
            dropped.append(item)
    if dropped:
        config.hook.pytest_deselected(items=dropped)
    if any(_docker_kinds(item) for item in keep):
        available = available_server_kinds()
        for item in keep:
            missing = sorted(_docker_kinds(item) - available)
            if missing:
                item.add_marker(pytest.mark.skip(reason=f"server kind {', '.join(missing)} not available (needs Docker)"))
    durations = _load_durations()
    keep.sort(key=lambda item: -durations.get(_key(config, item.nodeid), DEFAULT_DURATION))
    items[:] = keep


def pytest_xdist_make_scheduler(config, log):
    """Longest first: the collection is already sorted, so hand it out one case at a time,
    in order, to whichever worker is free (see LongestFirstScheduling)."""
    if config.getoption("dist") != "load":
        return None
    from xdist.scheduler import LoadScheduling

    class LongestFirstScheduling(LoadScheduling):
        """xdist's load scheduler sends each worker a contiguous chunk of at least two cases
        up front, which would give one worker the two longest. Here every worker gets one
        case from the head of the list, then one more in reverse worker order (a worker runs
        a case only once it holds the next one), and from then on one case each time it
        finishes one."""

        def schedule(self):
            assert self.collection_is_completed
            if self.collection is not None:
                for node in self.nodes:
                    self.check_schedule(node)
                return
            if not self._check_nodes_have_same_collection():
                self.log("**Different tests collected, aborting run**")
                return
            self.collection = next(iter(self.node2collection.values()))
            self.pending[:] = range(len(self.collection))
            if not self.collection:
                return
            self.maxschedchunk = 1
            for node in list(self.nodes) + list(reversed(self.nodes)):
                if not self.pending:
                    break
                self._send_tests(node, 1)
            if not self.pending:
                for node in self.nodes:
                    node.shutdown()

    return LongestFirstScheduling(config, log)


_case_seconds: dict = {}
_case_slow: dict = {}


def pytest_runtest_logreport(report):
    """Runs on the xdist controller for every phase of every case."""
    _case_seconds[report.nodeid] = _case_seconds.get(report.nodeid, 0.0) + report.duration
    if report.when == "call" or report.skipped:
        _case_slow.setdefault(report.nodeid, "slow" in report.keywords)
        if report.skipped and not hasattr(report, "wasxfail"):
            _case_seconds.pop(report.nodeid, None)
            _case_slow.pop(report.nodeid, None)


def pytest_sessionfinish(session):
    config = session.config
    if hasattr(config, "workerinput") or not config.getoption("--record-durations") or not _case_seconds:
        return
    durations = _load_durations()
    durations.update({_key(config, nodeid): round(s, 1) for nodeid, s in _case_seconds.items() if nodeid in _case_slow})
    DURATIONS_FILE.write_text(json.dumps(dict(sorted(durations.items())), indent=1) + "\n")


def pytest_terminal_summary(terminalreporter):
    """One list of every scenario that needs a look, with the directory holding its engine log,
    database and server transcript; then the quick-tier cases that took longer than the slow
    threshold."""
    rows = []
    for status in ("failed", "error", "xfailed", "passed"):
        for rep in terminalreporter.stats.get(status, []):
            props = dict(rep.user_properties)
            if status == "passed" and "xpass" not in props:
                continue
            label = {"failed": "FAILED", "error": "ERROR", "xfailed": "XFAIL", "passed": "XPASS"}[status]
            reason = props.get("reason") or getattr(rep, "wasxfail", "") or props.get("xpass") or str(rep.longrepr or "").split("\n")[-1]
            reason = " ".join(reason.split())[:300]
            rows.append(f"{label} {rep.nodeid}\n    reason: {reason}\n    artifacts: {props.get('artifacts', '-')}")
    if rows:
        terminalreporter.section("scenarios needing attention")
        for row in rows:
            terminalreporter.line(row)
    long_quick = sorted(((s, n) for n, s in _case_seconds.items() if _case_slow.get(n) is False and s > SLOW_THRESHOLD),
                        reverse=True)
    if long_quick:
        terminalreporter.section("quick-tier cases over the slow threshold")
        terminalreporter.line(f"each took more than {SLOW_THRESHOLD:.0f} s; mark it `slow: true` (README \"Tiers\") "
                              "if it does so on its own, not only under load")
        for s, n in long_quick:
            terminalreporter.line(f"{s:6.1f}s {n}")
    deselected = len(terminalreporter.stats.get("deselected", []))
    tier = _tier(terminalreporter.config)
    if deselected and tier != "all":
        terminalreporter.line(f"{tier} tier: {deselected} cases of the other tier or other servers deselected "
                              f"(--slow / --all / --servers select them)")
