"""Headless season runner (ofm-headless): drives a full season without a GUI."""

import os

import pytest

from conftest import HEADLESS, run

pytestmark = pytest.mark.skipif(
    not os.path.exists(HEADLESS),
    reason="ofm-headless not built (run: cargo build --workspace)",
)


def test_headless_runs_a_season():
    r = run(HEADLESS, "--seed", "42", "--days", "200")
    assert r.returncode == 0, r.stderr
    assert "initial state" in r.stdout
    assert "advance" in r.stdout
    # Standings are printed at the end.
    assert "Pts" in r.stdout


def test_headless_check_determinism_pass():
    r = run(HEADLESS, "--seed", "42", "--days", "100", "--check-determinism")
    assert r.returncode == 0, r.stderr
    assert "trajectory identical (content): true" in r.stdout
