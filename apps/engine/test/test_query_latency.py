"""
Socket latency of the thread list's paging on `serve`:
the reads the desktop sends while scrolling the inbox, replayed over the
socket against an engine that synced about 1,500 threads.

The reads are the captured `Query.toJSON()` forms in
crates/tern-store/tests/fixtures/model_queries.json (apps/desktop/tests/integration/model-query-capture.test.ts),
with the fixture's ids replaced by the synced ones. Each page is what one 25-row scroll step
sends (thread-list-engine-spec): the ids of the new rows (`inbox-ids`), their threads
(`missing-models`) and their messages (`messages-of-threads`), one after the other.

The timings go to latency.json in the run's work dir and to the test's user properties. The
bounds are loose, to catch a missing index or a per-row query rather than measure noise; the
run is parallel and the build may be a debug one.

    TERN_ENGINE_BIN=target/release/tern-engine python3 -m pytest apps/engine/test/test_query_latency.py -n 0
"""
import copy
import json
import statistics
import time
from pathlib import Path

import pytest

from harness.engine import REPO_ROOT
from harness.scenario import ScenarioRun, ServerSpec

THREADS = 1500
ROWS = 50
STEP = 25
QUERIES = REPO_ROOT / "crates" / "tern-store" / "tests" / "fixtures" / "model_queries.json"

SCENARIO = {
    "name": "query-latency-thread-list",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {"INBOX": {"messages": {"count": THREADS, "age_days": 30}}},
}


def templates() -> dict:
    captured = json.loads(QUERIES.read_text())["queries"]
    return {q["name"]: q["query"] for q in captured}


@pytest.fixture(scope="module")
def engine(request):
    binary = request.config.getoption("--engine")
    run = ScenarioRun(dict(SCENARIO), ServerSpec.parse({"fake": "dovecot"}),
                      binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    run.setup()
    run.ms.transport = "serve"
    try:
        run.ms.start()
        run.ms.wait_quiescent(timeout=300)
        yield run
    finally:
        run.teardown()


def test_thread_list_paging_latency(engine, record_property):
    ms = engine.ms
    q = templates()

    def call(query: dict) -> dict:
        return ms.rpc_call("query", query, timeout=30)

    folders = call({**copy.deepcopy(q["folders"]), "accounts": [ms.account["id"]]})["rows"]
    inbox = next(f["id"] for f in folders if f.get("role") == "inbox")

    def listing(**changes) -> dict:
        query = copy.deepcopy(q["inbox-ids"])
        query["where"]["and"][0]["value"] = [inbox]
        query.update(changes)
        return query

    total = call(listing(idsOnly=False, count=True, limit=None, offset=None))["count"]
    assert total >= THREADS * 0.95, f"{total} inbox threads synced, wanted about {THREADS}"

    def page(offset: int, limit: int) -> tuple:
        started = time.perf_counter()
        ids = call(listing(offset=offset, limit=limit))["ids"]
        threads = copy.deepcopy(q["missing-models"])
        threads["where"]["and"][0]["value"] = ids
        rows = call(threads)["rows"]
        messages = copy.deepcopy(q["messages-of-threads"])
        messages["where"]["and"][0]["value"] = ids
        msgs = call(messages)["rows"]
        return (time.perf_counter() - started) * 1000, ids, rows, msgs

    # The first screen, then 25-row steps to the bottom.
    first_ms, ids, rows, msgs = page(0, ROWS)
    assert len(ids) == ROWS and {r["id"] for r in rows} == set(ids) and len(msgs) >= ROWS
    pages = []
    for top in range(STEP, total - ROWS + 1, STEP):
        msec, ids, rows, msgs = page(top + ROWS - STEP, STEP)
        assert len(ids) == STEP and {r["id"] for r in rows} == set(ids), top
        assert {m["threadId"] for m in msgs} == set(ids), top
        pages.append(msec)

    pages_sorted = sorted(pages)
    summary = {
        "threads": total,
        "pages": len(pages),
        "first_screen_ms": round(first_ms, 2),
        "median_ms": round(statistics.median(pages), 2),
        "p95_ms": round(pages_sorted[int(len(pages_sorted) * 0.95) - 1], 2),
        "max_ms": round(pages_sorted[-1], 2),
        "reads_per_page": 3,
    }
    (engine.work / "latency.json").write_text(json.dumps({**summary, "page_ms": pages}, indent=2))
    for key, value in summary.items():
        record_property(key, value)
    print(f"thread list paging over the socket: {summary}")

    assert len(pages) > 50
    # Three round trips per page; the renderer's own cost is measured in thread-list-engine-spec.
    assert summary["median_ms"] < 50, summary
    assert summary["p95_ms"] < 250, summary
