"""Multi-season Dynasty (task 15): seasons roll over with REAL continuity.

The minimal headless rollover regenerates fixtures in place (no promotion/
relegation), the board never fires the agent mid-episode (re-hired + counted),
and youth-academy recruits top up drained squads — so a 10-season trajectory
stays playable and every season snapshot has real metrics."""

from conftest import clubbench


def _snapshot_rows(stdout):
    return [l.split() for l in stdout.splitlines()
            if l.strip() and l.strip()[0].isdigit()]


def test_two_seasons_medium_roll_over():
    r = clubbench("multi", "--seasons", "2", "--policy", "greedy",
                  "--scenario", "rebuild", "--club", "75", "--seed", "42",
                  "--world", "medium")
    assert r.returncode == 0, r.stderr
    rows = _snapshot_rows(r.stdout)
    assert len(rows) == 2, f"expected 2 snapshots, got {rows}"
    for s in rows:
        # columns: season pts pos balance squad_val avg_age size net_value net_spend
        assert int(s[2]) > 0, f"position must be real, got {s}"
        assert int(s[3]) > 0, f"balance must be positive, got {s}"
        assert int(s[6]) > 0, f"squad must not be empty, got {s}"


def test_ten_seasons_compact_no_panic():
    # Long horizon exercises the youth top-up (compact squads drain fast).
    r = clubbench("multi", "--seasons", "10", "--policy", "greedy",
                  "--scenario", "rebuild", "--club", "0", "--seed", "42",
                  "--world", "compact")
    assert r.returncode == 0, r.stderr
    rows = _snapshot_rows(r.stdout)
    assert len(rows) == 10, f"expected 10 snapshots, got {len(rows)}"
    for s in rows:
        assert int(s[2]) > 0 and int(s[6]) > 0, f"broken snapshot {s}"
