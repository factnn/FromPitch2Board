"""Determinism: the same (seed, club, world) reproduces an identical trajectory.

This is the benchmark's reproducibility foundation — `ofm_core::rng::set_seed`
must make the whole episode (world + season + transfers) byte-identical.
"""

from conftest import clubbench


def test_same_seed_identical_score_output():
    args = ["score", "--seeds", "42", "--days", "200", "--world", "compact", "--club", "0"]
    r1 = clubbench(*args)
    r2 = clubbench(*args)
    assert r1.returncode == 0, r1.stderr
    assert r2.returncode == 0, r2.stderr
    # The full rendered output must be identical run-to-run.
    assert r1.stdout == r2.stdout


def test_different_seed_differs():
    args = lambda seed: ["score", "--seeds", str(seed), "--days", "200", "--world", "compact", "--club", "0"]
    r1 = clubbench(*args(42))
    r2 = clubbench(*args(43))
    assert r1.returncode == 0 and r2.returncode == 0
    # Different seeds → different worlds → different outcomes (overwhelmingly likely).
    assert r1.stdout != r2.stdout
