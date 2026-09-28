"""Scoring protocol: paired-seed, reference-relative Z scores.

The first candidate (GreedyManager/Balanced) IS the frozen reference, so its
block must show all-zero *paired deltas* — a self-consistency check of the
scoring math. (Z is not necessarily 0: with a calibration file it is the
reference's position relative to its own large-seed distribution.)"""

import pytest
from conftest import frompitch2board, parse_blocks


def test_reference_vs_itself_paired_delta_zero():
    r = frompitch2board("score", "--seeds", "42,43", "--days", "150", "--world", "compact", "--club", "0")
    assert r.returncode == 0, r.stderr
    blocks = parse_blocks(r.stdout)
    assert blocks[0]["name"] == "GreedyManager"
    for dim, (cand, ref, delta, ci, z) in blocks[0]["rows"].items():
        assert cand == pytest.approx(ref, abs=1e-6)
        assert delta == pytest.approx(0.0, abs=1e-6)


def test_reference_row_present():
    """The reference's own absolute outcome is reported (interpretability)."""
    r = frompitch2board("score", "--seeds", "42", "--days", "150", "--world", "compact", "--club", "0")
    assert r.returncode == 0, r.stderr
    blocks = parse_blocks(r.stdout)
    # The reference block's "points" ref μ is a real number (the AI-default score).
    ref_points = blocks[0]["rows"]["points"][1]
    assert 0.0 < ref_points < 100.0
