"""Reference Calibration Set (the design notes §2): Greedy's per-cell μ/σ over many seeds
is frozen, and scoring Z uses that stable scale. Dims with no identifiable
reference variance (σ≈0) report raw + Δ only, never a forced ±1."""

import json
from pathlib import Path

from conftest import clubbench, parse_blocks

DATA = Path("data")
# Coach and Manager keep separate calibration files (different references).
CAL_FILE = DATA / "calibration-Compact-Manager.json"


def test_calibrate_writes_frozen_stats(tmp_path):
    # Write to a temp path so the shared calibration file is never touched by
    # tests (other tests read it concurrently — no torn-read races).
    out = tmp_path / "cal.json"
    r = clubbench("calibrate", "--world", "compact", "--clubs", "0",
                  "--scenarios", "rebuild", "--count", "12", "--days", "100",
                  "--mode", "manager", "--out", str(out))
    assert r.returncode == 0, r.stderr
    assert "saved" in r.stdout
    cal = json.loads(out.read_text())
    assert cal["world"] == "Compact"
    cell = cal["cells"]["rebuild:0"]
    assert cell["n"] == 12
    assert "points" in cell["dims"] and cell["dims"]["points"]["sigma"] > 0


def test_score_uses_calibration_for_z():
    # In the compact world the Greedy reference never overspends / never leaves
    # the healthy squad range → those dims have σ≈0 → Z is None (raw + Δ only).
    r = clubbench("score", "--seeds", "42,43", "--days", "100", "--world", "compact", "--club", "0")
    assert r.returncode == 0, r.stderr
    block = parse_blocks(r.stdout)[0]
    # A dimension with real variance still gets a calibration-based Z.
    z = block["rows"]["points"][4]
    assert z is not None and abs(z) < 100
    # A constant-in-calibration dim reports raw but has no Z.
    assert block["rows"]["squad_size"][4] is None


def test_score_falls_back_without_calibration():
    # Use a (scenario, club) cell that is NOT in the compact calibration file
    # (which only has rebuild:0) — the file still exists so loading succeeds,
    # but the cell lookup misses and scoring falls back to the eval-seed
    # reference std. No file moves, so no race with other tests reading the
    # shared calibration.
    r = clubbench("score", "--seeds", "42,43", "--days", "100", "--world", "compact",
                  "--club", "1", "--scenario", "crisis")
    assert r.returncode == 0, r.stderr
    block = parse_blocks(r.stdout)[0]
    # Fallback (eval-seed ref_std) gives a numeric Z even for squad_size.
    assert block["rows"]["squad_size"][4] is not None
