"""Token + cost report for a ClubBench agent run.

Parses the run's agent_output.jsonl (Claude Code stream-json) and sums the
per-message usage: input tokens, output tokens, cache read, and an estimated
cost. Model pricing is read from MODEL_PRICE_PER_MTok if set, else a default.
"""

import json
import os
import sys
from pathlib import Path


def main():
    if len(sys.argv) < 2:
        print("usage: token_report.py runs/<dir>/agent_output.jsonl", file=sys.stderr)
        sys.exit(1)
    path = Path(sys.argv[1])
    total_in = total_out = total_cache = 0
    turns = 0
    for line in path.read_text().splitlines():
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            continue
        if e.get("type") == "assistant":
            turns += 1
        u = e.get("usage")
        if isinstance(u, dict):
            total_in += u.get("input_tokens", 0)
            total_out += u.get("output_tokens", 0)
            total_cache += u.get("cache_read_input_tokens", 0)
    if total_in == 0 and total_out == 0:
        print("no usage data found")
        return

    # Price per 1M tokens: MODEL_PRICE_PER_MTok="in,out" else default.
    price = os.environ.get("MODEL_PRICE_PER_MTok", "0.27,1.10").split(",")
    p_in = float(price[0]); p_out = float(price[1])
    cost = (total_in / 1e6 * p_in) + (total_out / 1e6 * p_out)
    cache_cost = total_cache / 1e6 * p_in * 0.1  # cache reads ~0.1x input

    print(f"turns:            {turns}")
    print(f"input tokens:     {total_in:,}")
    print(f"output tokens:    {total_out:,}")
    print(f"cache read tokens:{total_cache:,}")
    print(f"est. cost (USD):  ${cost + cache_cost:.4f}  (in@{p_in}/M, out@{p_out}/M)")


if __name__ == "__main__":
    main()
