"""
Live queries and `task.done` on `serve` against the fake server: the thread list held as a
`query.subscribe` window, a star queued with `task.queue {notify: true}`, and the
`task.done` and `query.changed` that follow once the server took it.

    AIRPOST_ENGINE_BIN=target/release/airpost-engine python3 -m pytest apps/engine/test/test_live_queries.py -n 0
"""
import time
from pathlib import Path

import pytest

from harness.scenario import ScenarioRun, ServerSpec

WAIT = 60
WINDOW = 10

SCENARIO = {
    "name": "live-queries",
    "servers": [{"fake": "dovecot"}],
    "mailboxes": {"INBOX": {"messages": {"count": 30, "age_days": 10}}},
}


@pytest.fixture(scope="module")
def run(request):
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


def wait_for(ms, method: str, match, what: str) -> dict:
    deadline = time.monotonic() + WAIT
    while True:
        found = [p for p in ms.notifications(method) if match(p)]
        if found:
            return found[0]
        assert time.monotonic() < deadline, f"no {what}; got {ms.notifications(method)}"
        time.sleep(0.1)


def test_a_star_reaches_task_done_and_the_window(run):
    ms = run.ms
    aid = ms.account["id"]
    sub = ms.rpc_call("query.subscribe", {"query": {"class": "Thread", "accounts": [aid]},
                                          "window": {"offset": 0, "limit": WINDOW}})
    assert len(sub["rows"]) == WINDOW and sub["total"] >= 25, sub["total"]
    thread = sub["rows"][3]
    assert not thread.get("starred"), thread

    task = {"id": "live-star-1", "__cls": "ChangeStarredTask", "aid": aid, "status": "local", "v": 0,
            "threadIds": [thread["id"]], "messageIds": [], "starred": True}
    ms.rpc_call("task.queue", {"accountId": aid, "task": task, "notify": True})

    done = wait_for(ms, "task.done", lambda p: p["taskId"] == "live-star-1", "task.done")
    assert (done["accountId"], done["status"], done["error"]) == (aid, "complete", None), done
    assert done["task"]["id"] == "live-star-1"

    def starred(p):
        return p["subId"] == sub["subId"] and any(
            op["op"] == "update" and op["row"]["id"] == thread["id"] and op["row"].get("starred")
            for op in p.get("ops") or [])
    changed = wait_for(ms, "query.changed", starred, "query.changed starring the thread")
    assert changed["version"] > sub["version"]
    versions = [p["version"] for p in ms.notifications("query.changed") if p["subId"] == sub["subId"]]
    assert versions == list(range(sub["version"] + 1, sub["version"] + 1 + len(versions))), versions
    # Only one task.done, on the connection that asked.
    assert len([p for p in ms.notifications("task.done") if p["taskId"] == "live-star-1"]) == 1

    ms.rpc_call("query.unsubscribe", {"subId": sub["subId"]})
