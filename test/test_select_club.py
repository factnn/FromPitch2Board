"""select_club: the managed club's strength affects the difficulty baseline.

A clearly stronger club (top-division squad-strength rank) must have a higher
reference (AI-default) baseline than a clearly weaker club (bottom-division) on
the same world/seed — otherwise scenarios (relegation vs title race) are not
distinguishable. We compare the two extremes of the strength ranking (the
Greedy reference reinvests budgets, so adjacent ranks are too noisy)."""

from conftest import frompitch2board, parse_blocks


def _reference_points(stdout):
    return parse_blocks(stdout)[0]["rows"]["points"][1]


def test_stronger_club_higher_baseline():
    # Single-seed reference points are noisy (division structure + transfer
    # variance), so compare the MEAN over 3 seeds. A mid-strong club (rank 8)
    # must outscore the weakest club (rank 0) on average.
    args = lambda club: ["score", "--seeds", "42,43,44", "--days", "200", "--world", "compact", "--club", str(club)]
    weakest = frompitch2board(*args(0))
    stronger = frompitch2board(*args(8))
    assert weakest.returncode == 0 and stronger.returncode == 0
    assert _reference_points(stronger.stdout) > _reference_points(weakest.stdout)
