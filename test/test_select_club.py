"""select_club: the managed club's strength affects the difficulty baseline.

A stronger club (higher squad-strength rank) must have a higher reference
(AI-default) baseline than a weaker club on the same world/seed — otherwise
scenarios (relegation vs title race) are not distinguishable."""

from conftest import clubbench, parse_blocks


def _reference_points(stdout):
    return parse_blocks(stdout)[0]["rows"]["points"][1]


def test_stronger_club_higher_baseline():
    args = lambda club: ["score", "--seeds", "42", "--days", "200", "--world", "compact", "--club", str(club)]
    weakest = clubbench(*args(0))
    strongest = clubbench(*args(1))  # compact world has 2 divisions; rank 1 is a top-division club
    assert weakest.returncode == 0 and strongest.returncode == 0
    # The stronger club's AI-default (reference) outcome exceeds the weaker's.
    assert _reference_points(strongest.stdout) > _reference_points(weakest.stdout)
