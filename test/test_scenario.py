"""Scenario budget: the managed club's financial starting state is a first-class
scenario lever that directly gates buying power.

With a `crisis` budget (£5M transfer) ProactiveManager cannot afford anyone →
squad value unchanged. With a `rebuild` budget (£50M) it buys → squad value
grows (measured by squad_value, since buying into the healthy 22-26 range keeps
the squad_size distance at 0)."""

from conftest import frompitch2board, parse_blocks


def _proactive_squad_value_delta(stdout):
    for b in parse_blocks(stdout):
        if b["name"].startswith("Proactive"):
            return b["rows"]["squad_value"][2]  # Δ column
    raise AssertionError("Proactive block not found")


def test_crisis_budget_blocks_buying():
    r = frompitch2board("score", "--scenario", "crisis", "--club", "15", "--seeds", "42",
                  "--days", "300", "--world", "compact")
    assert r.returncode == 0, r.stderr
    # £5M budget → can't afford a meaningful transfer → squad value unchanged.
    assert _proactive_squad_value_delta(r.stdout) == 0.0


def test_rebuild_budget_enables_buying():
    r = frompitch2board("score", "--scenario", "rebuild", "--club", "15", "--seeds", "42",
                  "--days", "300", "--world", "compact")
    assert r.returncode == 0, r.stderr
    # £50M budget → buys players → squad value grows vs the reference.
    assert _proactive_squad_value_delta(r.stdout) > 0.0


def test_scenario_name_in_header():
    r = frompitch2board("score", "--scenario", "moneyball", "--seeds", "42", "--days", "60",
                  "--world", "compact", "--club", "0")
    assert r.returncode == 0, r.stderr
    assert "scenario=moneyball" in r.stdout
