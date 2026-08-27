#!/usr/bin/env python3
"""Completeness check for a finished run dir (used by run_grid.sh's skip logic).

score.json alone is NOT proof of completion — run_agent.sh writes it even when
the agent hit max-turns mid-season. A run counts as complete only if the
horizon matches AND the trajectory clock reached the season end (the 400-day
medium season ends 2027-03-18; truncated runs stay far behind it).

Usage: check_complete.py <run_dir> <horizon_days>
Prints "ok" or "stale"; exit 0 either way (the caller decides).
"""

import json
import sys


def main() -> int:
    run_dir, horizon = sys.argv[1], sys.argv[2]
    try:
        md = json.load(open(run_dir + "/metadata.json"))
        tr = json.load(open(run_dir + "/trajectory.json"))
        d0 = md.get("horizon_days", 0)
        seasons = int(md.get("seasons", 1) or 1)
        # Old multi-season runs predate the metadata seasons field; infer it
        # from the horizon cap so a 1-season-complete "10Y" run isn't skipped.
        if seasons == 1 and int(d0) >= 800:
            seasons = max(int(d0) // 400, 1)
        if str(d0) != horizon:
            print("stale (horizon mismatch)")
            return 0
        # Boundary snapshots are the authoritative multi-season signal: a
        # complete N-season run records N snapshots. A 10Y run stopped after
        # Y1 records 1 → redo. Single-season runs from before the snapshot
        # fix fall back to the clock rule (season ends 2027-03-18).
        snaps = tr.get("snapshots") or []
        if seasons > 1:
            print("ok" if len(snaps) >= seasons else "stale")
            return 0
        if snaps and len(snaps) >= 1:
            print("ok")
            return 0
        c1 = tr["final_game"]["clock"]["current_date"][:10]
        print("ok" if c1 >= "2027-03-01" else "stale")
    except Exception as e:
        print(f"stale ({e})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
