"""Print a one-line summary of a run's score.json (for the grid runner)."""
import json
import sys

if __name__ == "__main__":
    d, a, m, c, s, sc = sys.argv[1:7]
    j = json.load(open(d))
    print(f"[{a}/{m} club={c} seed={s} {sc}] "
          f"pts={j['points']} pos={j['position']} "
          f"net_value={j['net_value']} net_spend={j['net_spend']}")
