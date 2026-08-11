"""Manager actions: scout + proactive transfers must actually change state.

The ProactiveManager scouts targets and bids ~1.4x market value; if the
transfer loop works, its squad/finance metrics differ from the reference
(buying money → squad value rises, balance drops, squad gets younger)."""

from conftest import clubbench, parse_blocks


def _proactive_block(stdout):
    for b in parse_blocks(stdout):
        if b["name"].startswith("Proactive"):
            return b
    return None


def test_proactive_buys_players():
    r = clubbench("score", "--seeds", "42", "--days", "250", "--world", "compact", "--club", "15")
    assert r.returncode == 0, r.stderr
    block = _proactive_block(r.stdout)
    assert block is not None, "Proactive block not found"
    # Buying changes the squad: squad_value rises and/or balance drops vs the
    # reference. At least one of these must be non-zero.
    _, _, delta_sv, _, _ = block["rows"]["squad_value"]
    _, _, delta_bal, _, _ = block["rows"]["balance"]
    assert abs(delta_sv) > 0.0 or abs(delta_bal) > 0.0
