"""Finance scoring (the design notes §1): value created (net_value) is directional;
net_spend and wage_bill are *constraints* — only budget violations are scored,
never "spend as little as possible". avg_age is a diagnostic (raw only)."""

from conftest import clubbench, parse_blocks


def _proactive_block(stdout):
    for b in parse_blocks(stdout):
        if b["name"].startswith("Proactive"):
            return b
    return None


def test_finance_dimensions_reported():
    r = clubbench("score", "--scenario", "rebuild", "--club", "15", "--seeds", "42",
                  "--days", "200", "--world", "compact")
    assert r.returncode == 0, r.stderr
    block = _proactive_block(r.stdout)
    assert block is not None
    # Directional + constraint dims are scored (have a Z).
    for dim in ("net_value", "squad_value", "transfer_budget_violation", "wage_budget_violation", "squad_size"):
        assert dim in block["rows"], f"missing scored dim {dim}"
    # Diagnostic dims (avg_age) are raw-only → Z = None.
    assert block["rows"]["avg_age"][4] is None


def test_within_budget_is_zero_violation():
    """A candidate that spends inside the £50M transfer budget has no
    transfer-budget violation (0 = healthy), regardless of how much it spends."""
    r = clubbench("score", "--scenario", "rebuild", "--club", "15", "--seeds", "42",
                  "--days", "300", "--world", "compact")
    assert r.returncode == 0, r.stderr
    block = _proactive_block(r.stdout)
    # Proactive spends ~1.4× value on targets but stays inside the budget.
    cand_violation = block["rows"]["transfer_budget_violation"][0]
    assert cand_violation == 0.0
