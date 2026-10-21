"""pytest entry point: one test per (scenario, server) pair."""
from pathlib import Path

import pytest

from harness.scenario import (DOCKER_KINDS, ScenarioRun, ScenarioSkipped, ServerSpec, discover, load_scenario,
                              scenario_slow)

SCENARIOS = Path(__file__).parent / "scenarios"


def _cases(config):
    """Every (scenario, server) pair, marked for conftest's tier and server selection: `slow`
    when the scenario says so for that server or the server runs in Docker, `docker(kind)`
    for the Docker kinds."""
    only = config.getoption("--scenario")
    cases = []
    for path in discover(SCENARIOS):
        sc = load_scenario(path)
        if only and only not in sc["name"]:
            continue
        for entry in sc["servers"]:
            spec = ServerSpec.parse(entry)
            marks = []
            if spec.kind in DOCKER_KINDS:
                marks.append(pytest.mark.docker(spec.kind))
            if spec.kind in DOCKER_KINDS or scenario_slow(sc, spec):
                marks.append(pytest.mark.slow)
            cases.append(pytest.param(path, spec, id=f"{sc['name']}[{spec.id}]", marks=marks))
    return cases


def pytest_generate_tests(metafunc):
    if "scenario_path" in metafunc.fixturenames:
        metafunc.parametrize("scenario_path,server_spec", _cases(metafunc.config))


def test_scenario(scenario_path, server_spec, request):
    sc = load_scenario(scenario_path)
    binary = request.config.getoption("--engine")
    run = ScenarioRun(sc, server_spec, binary=Path(binary) if binary else None, keep=request.config.getoption("--keep"))
    # Read back by conftest.pytest_terminal_summary, which runs on the xdist controller.
    request.node.user_properties.append(("artifacts", str(run.work)))
    try:
        failures = run.run()
    except ScenarioSkipped as e:
        pytest.skip(str(e))
    if failures:
        request.node.user_properties.append(("reason", "; ".join(failures)))
        pytest.fail(f"{sc['name']} on {server_spec.id}:\n" + "\n".join(failures)
                    + f"\n\nartifacts: {run.work}\n" + "\n".join(run.report[-15:]), pytrace=False)
    for x in run.xpasses:
        print(f"XPASS {sc['name']} on {server_spec.id}: {x}")
        request.node.user_properties.append(("xpass", x))
    if run.xfails:
        pytest.xfail("; ".join(run.xfails))
