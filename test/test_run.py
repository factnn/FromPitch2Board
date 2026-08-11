"""Benchmark runner (`clubbench run`): one command runs the whole scenario grid
× all baselines and emits a consolidated leaderboard."""

from conftest import clubbench


def test_run_produces_leaderboard():
    r = clubbench("run", "--world", "compact", "--clubs", "0", "--scenarios", "crisis",
                  "--seeds", "42", "--days", "100")
    assert r.returncode == 0, r.stderr
    out = r.stdout
    assert "ClubBench Benchmark" in out
    assert "=== scenario=crisis" in out
    assert "reference raw:" in out          # the difficulty anchor
    for cand in ("Proactive", "Selling", "Passive"):
        assert cand in out
    assert "=== overall: mean Z" in out     # the consolidated leaderboard


def test_run_coach_mode():
    r = clubbench("run", "--mode", "coach", "--world", "compact", "--clubs", "0",
                  "--scenarios", "crisis", "--seeds", "42", "--days", "150")
    assert r.returncode == 0, r.stderr
    assert "mode=Coach" in r.stdout
    assert "reference = CoachBestXI" in r.stdout
    # Coach-track candidates present; transfers frozen.
    assert "CoachBestXI" in r.stdout
    assert "CoachWorst" in r.stdout


def test_run_manager_mode_is_default():
    r = clubbench("run", "--world", "compact", "--clubs", "0", "--scenarios", "crisis",
                  "--seeds", "42", "--days", "100")
    assert r.returncode == 0, r.stderr
    assert "mode=Manager" in r.stdout
    assert "Proactive" in r.stdout and "Selling" in r.stdout


def test_run_respects_scenario_and_club():
    r1 = clubbench("run", "--world", "compact", "--clubs", "0", "--scenarios", "crisis",
                   "--seeds", "42", "--days", "100")
    r2 = clubbench("run", "--world", "compact", "--clubs", "0", "--scenarios", "rebuild",
                   "--seeds", "42", "--days", "100")
    assert r1.returncode == 0 and r2.returncode == 0
    # Scenario budget flows through: the reference's balance baseline differs.
    assert "reference raw: pts=7.0  balance=21" in r1.stdout  # crisis ≈ £21M
    assert "reference raw: pts=7.0  balance=61" in r2.stdout  # rebuild ≈ £61M
