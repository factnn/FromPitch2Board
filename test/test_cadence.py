"""Decision cadence: the episode stops at matchdays/offers/market and each stop
is one agent step — a season is a decision trajectory, not a fast-forward."""

from conftest import frompitch2board, parse_blocks


def test_cadence_produces_trajectory():
    r = frompitch2board("cadence", "--seeds", "42", "--days", "200", "--world", "compact")
    assert r.returncode == 0, r.stderr
    assert "steps" in r.stdout
    # The managed agent takes a non-trivial number of decisions over 200 days
    # (matchdays + transfer offers). Passive still advances day-by-day.
    assert "avg_steps" in r.stdout


def test_score_produces_dimensions():
    r = frompitch2board("score", "--seeds", "42", "--days", "150", "--world", "compact", "--club", "0")
    assert r.returncode == 0, r.stderr
    blocks = parse_blocks(r.stdout)
    assert blocks, "no candidate blocks found"
    # Dimension structure: directional (points/net_value/squad_value),
    # budget-constraint (violations), and target-range (squad_size) are scored;
    # avg_age is diagnostic — reported raw with Z = None.
    for dim in ("points", "net_value", "squad_value", "transfer_budget_violation", "wage_budget_violation", "squad_size"):
        assert dim in blocks[0]["rows"], f"missing dimension {dim}"
    assert blocks[0]["rows"]["avg_age"][4] is None  # diagnostic → no Z
