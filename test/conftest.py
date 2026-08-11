"""Shared test helpers for ClubBench.

Each feature has its own test_*.py that runs the compiled Rust binaries via
subprocess and asserts on their output. The binaries must be built first:

    cd openfootmanager/src-tauri && cargo build -p clubbench -p ofm-headless
"""

import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BIN_DIR = REPO / "openfootmanager" / "src-tauri" / "target" / "debug"
CLUBBENCH = BIN_DIR / "clubbench"
HEADLESS = BIN_DIR / "ofm-headless"


def run(binary, *args, timeout=600):
    """Run a binary and return the CompletedProcess."""
    return subprocess.run(
        [str(binary), *map(str, args)],
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def clubbench(*args, timeout=600):
    """Run the `clubbench` binary."""
    return run(CLUBBENCH, *args, timeout=timeout)


def parse_blocks(stdout):
    """Parse `clubbench score` output into [(candidate, {dim: (cand,ref,delta,ci,z)})].

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
            # dim, cand μ, ref μ, Δ, Δ±CI, Z  →  6 tokens
            if len(parts) == 6:
                current["rows"][parts[0]] = tuple(float(x) for x in parts[1:])
    if current is not None:
        blocks.append(current)
    return blocks
