"""The fake server must answer like Dovecot for every exchange the engine performs.
Needs Docker (or HARNESS_SERVERS naming dovecot with a local dovecot); skipped otherwise.
Slow tier: conftest skips the `docker` marker's kinds when they are not available."""
import pytest

from conformance.compare import compare_all
from conformance.probes import PROBES

pytestmark = [pytest.mark.slow, pytest.mark.docker("dovecot")]


@pytest.mark.parametrize("probe", PROBES, ids=lambda p: p.__name__)
def test_fake_matches_dovecot(probe):
    results = compare_all(probes=[probe])
    bad = {k: v for k, (status, v) in results.items() if status == "DIFFERENT"}
    if bad:
        msg = "\n\n".join(f"{k[0]}/{k[1]}:\n" + "\n".join(diff[2:]) for k, diff in bad.items())
        pytest.fail(f"fake differs from Dovecot in {len(bad)} response(s):\n{msg}", pytrace=False)
