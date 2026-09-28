"""Manager actions: scout + proactive transfers must actually change state.

The ProactiveManager scouts targets and bids ~1.4x market value; if the
transfer loop works, its squad/finance metrics differ from the reference
(buying money → squad value rises, balance drops, squad gets younger)."""

from conftest import frompitch2board, parse_blocks


def _proactive_block(stdout):
    for b in parse_blocks(stdout):
        if b["name"].startswith("Proactive"):
            return b
    return None


def test_proactive_buys_players():
    r = frompitch2board("score", "--scenario", "rebuild", "--seeds", "42", "--days", "250", "--world", "compact", "--club", "15")
    assert r.returncode == 0, r.stderr
    block = _proactive_block(r.stdout)
    assert block is not None, "Proactive block not found"
    # With a £50M budget Proactive buys players → the squad is worth more than
    # the reference's, while staying inside the budget (no violation).
    _, _, delta_sv, _, _ = block["rows"]["squad_value"]
    assert delta_sv > 0.0
    assert block["rows"]["transfer_budget_violation"][0] == 0.0
