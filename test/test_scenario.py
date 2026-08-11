"""Scenario budget: the managed club's financial starting state is a first-class
scenario lever that directly gates buying power.

With a `crisis` budget (£5M transfer) ProactiveManager cannot afford anyone →
squad unchanged. With a `rebuild` budget (£50M) it buys → squad grows."""

from conftest import clubbench, parse_blocks


def _proactive_squad_size_delta(stdout):
    for b in parse_blocks(stdout):
        if b["name"].startswith("Proactive"):
            return b["rows"]["squad_size"][2]  # Δ column
    raise AssertionError("Proactive block not found")


def test_crisis_budget_blocks_buying():
    r = clubbench("score", "--scenario", "crisis", "--club", "15", "--seeds", "42",
                  "--days", "300", "--world", "compact")
    assert r.returncode == 0, r.stderr
    # £5M budget → can't afford a meaningful transfer → squad unchanged.
    assert _proactive_squad_size_delta(r.stdout) == 0.0


def test_rebuild_budget_enables_buying():
    r = clubbench("score", "--scenario", "rebuild", "--club", "15", "--seeds", "42",
                  "--days", "300", "--world", "compact")
    assert r.returncode == 0, r.stderr
    # £50M budget → buys players → squad grows vs the reference.
    assert _proactive_squad_size_delta(r.stdout) > 0.0


def test_scenario_name_in_header():
    r = clubbench("score", "--scenario", "moneyball", "--seeds", "42", "--days", "60",
                  "--world", "compact", "--club", "0")
    assert r.returncode == 0, r.stderr
    assert "scenario=moneyball" in r.stdout
