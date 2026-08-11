"""Finance scoring: value created (net_value) and net spend, not raw ending
balance.

A club that buys must show positive net_spend (money spent) and a net_value
that reflects whether that spending created or destroyed wealth. The finance
dimensions are reported relative to the reference, so a manager who burns money
without building value is penalised."""

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
    for dim in ("net_value", "net_spend"):
        assert dim in block["rows"]


def test_buying_shows_net_spend():
    r = clubbench("score", "--scenario", "rebuild", "--club", "15", "--seeds", "42",
                  "--days", "300", "--world", "compact")
    assert r.returncode == 0, r.stderr
    block = _proactive_block(r.stdout)
    _, _, delta_spend, _, _ = block["rows"]["net_spend"]
    # Proactive buys players with the £50M budget → it spends vs the reference.
    assert delta_spend > 0
