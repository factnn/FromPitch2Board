"""Shared test helpers for FromPitch2Board.

Each feature has its own test_*.py that runs the compiled Rust binaries via
subprocess and asserts on their output. The binaries must be built first
(they live in this repo, depending on ofm_core via a pinned git dependency):

    cd frompitch2board-repo && cargo build -p frompitch2board -p ofm-headless
"""

import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BIN_DIR = REPO / "target" / "debug"
FROMPITCH2BOARD = BIN_DIR / "frompitch2board"
HEADLESS = BIN_DIR / "ofm-headless"


def run(binary, *args, timeout=600):
    """Run a binary and return the CompletedProcess."""
    return subprocess.run(
        [str(binary), *map(str, args)],
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def frompitch2board(*args, timeout=600):
    """Run the `frompitch2board` binary."""
    return run(FROMPITCH2BOARD, *args, timeout=timeout)


def parse_blocks(stdout):
    """Parse `frompitch2board score` output into [(candidate, {dim: (cand,ref,delta,ci,z)})].

    Columns in the rendered table: dim, cand μ, ref μ, Δ, Δ±CI, Z.
    """
    blocks = []
    current = None
    for line in stdout.splitlines():
        line = line.strip()
        if line.startswith("==="):
            if current is not None:
                blocks.append(current)
            current = {"name": line.strip("= ").split(" vs ")[0], "rows": {}}
        elif current is not None and line and not line.startswith("dim"):
            parts = line.split()
            # dim, cand μ, ref μ, Δ, Δ±CI, Z  →  6 tokens. Diagnostic dims (or
            # dims with no identifiable calibration variance) have Z = "—",
            # stored as None; the raw/Δ columns are still floats.
            if len(parts) == 6:
                if parts[5] == "—":
                    current["rows"][parts[0]] = tuple(float(x) for x in parts[1:5]) + (None,)
                else:
                    current["rows"][parts[0]] = tuple(float(x) for x in parts[1:])
    if current is not None:
        blocks.append(current)
    return blocks
