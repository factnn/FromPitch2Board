"""Derived agent-behavior / efficiency / reliability stats from a run directory.

Everything here is computed from the lossless trajectory.jsonl (the step-level
decision trace), so it can be re-derived at any time without re-running the
agent: invalid-action rate, repeated-action rate, no-op rate, tool-category
counts, scout-before-buy, latency, and token/cost aggregates.

Also persists the full derived record as `analysis.json` in the run dir, so
every number the paper might want (cost, event types, retries, ...) survives
without re-parsing raw logs.

Usage:
  python agents/analyze_trajectory.py runs/<run_id>/...
"""

import collections
import json
import os
import sys


def cost_usd(input_tokens, output_tokens, cache_tokens):
    """Est. API cost with the same pricing token_report.py uses."""
    price = os.environ.get("MODEL_PRICE_PER_MTok", "0.27,1.10").split(",")
    p_in = float(price[0])
    p_out = float(price[1])
    return (input_tokens / 1e6 * p_in) + (output_tokens / 1e6 * p_out) + (cache_tokens / 1e6 * p_in * 0.1)


def codex_usage(run_dir: str) -> dict:
    """Recompute codex's usage.json from its raw --json event stream.

    codex emits exactly ONE top-level usage object per run (session totals);
    cached_input_tokens = cache reads; reasoning_output_tokens is reported
    separately by the OpenAI responses API. llm_calls ≈ completed turns.
    Runs before the adapter existed can be fixed post-hoc with this.
    """
    usage = {}
    turns = 0
    output_path = os.path.join(run_dir, "agent_output.jsonl")
    if not os.path.exists(output_path):
        return usage
    try:
        for line in open(output_path):
            line = line.strip()
            if not line:
                continue
            e = json.loads(line)
            if e.get("type") == "turn.completed":
                turns += 1
            if not usage and isinstance(e.get("usage"), dict):
                usage = e["usage"]
        return {
            "input_tokens": usage.get("input_tokens", 0),
            "output_tokens": usage.get("output_tokens", 0),
            "cache_read_input_tokens": usage.get("cached_input_tokens", 0),
            "reasoning_output_tokens": usage.get("reasoning_output_tokens", 0),
            "llm_calls": turns,
            "completed": True,
        }
    except Exception:
        return {"input_tokens": 0, "output_tokens": 0,
                "cache_read_input_tokens": 0, "reasoning_output_tokens": 0,
                "llm_calls": turns, "completed": True}


def analyze(run_dir: str) -> dict:
    steps = []
    traj = os.path.join(run_dir, "trajectory.jsonl")
    if os.path.exists(traj):
        for line in open(traj):
            line = line.strip()
            if line:
                steps.append(json.loads(line))

    usage = {}
    usage_path = os.path.join(run_dir, "usage.json")
    if os.path.exists(usage_path):
        usage = json.load(open(usage_path))

    meta = {}
    meta_path = os.path.join(run_dir, "metadata.json")
    if os.path.exists(meta_path):
        meta = json.load(open(meta_path))

    # codex usage is ALWAYS re-derived from its lossless raw event stream —
    # early runs were aggregated with the wrong field names (cache/calls = 0).
    if meta.get("agent") == "codex":
        usage = codex_usage(run_dir)
        try:
            json.dump(usage, open(usage_path, "w"), indent=2)
        except OSError:
            pass

    n = len(steps)
    acts = [(s.get("action") or {}).get("action", "?") for s in steps]
    cats = collections.Counter(acts)

    invalid = sum(1 for s in steps if not s.get("tool_success"))
    noop = sum(1 for a in acts if a == "Continue")
    # Repeated = same action name AND same params as the previous step
    # (a classic loop signature; approximate window = immediate predecessor).
    repeat = 0
    for i in range(1, n):
        a, b = steps[i].get("action"), steps[i - 1].get("action")
        if (a and b and a.get("action") == b.get("action")
                and a.get("params") == b.get("params")):
            repeat += 1

    # Scout-before-buy: a MakeBid whose target was never scouted in any earlier step.
    scouted = {s["action"]["params"]["player_id"]
               for s in steps if s.get("action", {}).get("action") == "Scout"}
    buys = [s for s in steps if s.get("action", {}).get("action") == "MakeBid"]
    buys_unscouted = sum(1 for s in buys if s["action"]["params"]["player_id"] not in scouted)

    latencies = [s["latency_ms"] for s in steps if isinstance(s.get("latency_ms"), (int, float))]
    total_latency = sum(latencies)

    # Raw agent-output events (off-the-shelf agents like cc/codex): count event
    # types + subtypes generically — compaction/retry signatures, whatever the
    # harness emits, are recorded for later analysis.
    event_types = collections.Counter()
    event_subtypes = collections.Counter()
    output_path = os.path.join(run_dir, "agent_output.jsonl")
    if os.path.exists(output_path):
        for line in open(output_path):
            line = line.strip()
            if not line:
                continue
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if isinstance(e, dict):
                event_types[e.get("type", "?")] += 1
                if e.get("subtype"):
                    event_subtypes[e.get("subtype")] += 1

    in_tok = usage.get("input_tokens", 0)
    out_tok = usage.get("output_tokens", 0)
    cache_tok = usage.get("cache_read_input_tokens", 0)

    return {
        "run_id": os.path.basename(run_dir),
        "agent": meta.get("agent"), "track": meta.get("track"), "scenario": meta.get("scenario"),
        "seed": meta.get("seed"),
        "steps": n,
        "llm_calls": usage.get("llm_calls", 0),
        "input_tokens": in_tok,
        "output_tokens": out_tok,
        "cache_read_tokens": cache_tok,
        "cost_usd": round(cost_usd(in_tok, out_tok, cache_tok), 4),
        "wall_time_s": usage.get("wall_time_s") or meta.get("wall_time_s"),
        "completed": usage.get("completed") if "completed" in usage else meta.get("completed"),
        "invalid_count": invalid,
        "invalid_rate": (invalid / n) if n else 0.0,
        "noop_count": noop,
        "noop_rate": (noop / n) if n else 0.0,
        "repeat_count": repeat,
        "repeat_rate": (repeat / n) if n else 0.0,
        "unscouted_purchase_count": buys_unscouted,
        "unscouted_purchase_rate": (buys_unscouted / len(buys)) if buys else None,
        "tool_categories": dict(cats),
        "avg_latency_ms": (sum(latencies) / len(latencies)) if latencies else None,
        "total_llm_latency_s": round(total_latency / 1000, 1) if total_latency else None,
        "agent_output_events": dict(event_types),
        "agent_output_subtypes": dict(event_subtypes),
    }


def render(r: dict) -> str:
    unscouted = (f"{r['unscouted_purchase_rate']:.1%}"
                 if r['unscouted_purchase_rate'] is not None else "n/a")
    return (
        f"[{r['run_id']}] agent={r['agent']} track={r['track']} scenario={r['scenario']} seed={r['seed']} "
        f"completed={r['completed']}\n"
        f"  steps={r['steps']} llm_calls={r['llm_calls']} in={r['input_tokens']:,} out={r['output_tokens']:,} "
        f"cache={r['cache_read_tokens']:,} cost=${r['cost_usd']:.4f} wall={r['wall_time_s']}s\n"
        f"  invalid={r['invalid_rate']:.1%} noop={r['noop_rate']:.1%} repeat={r['repeat_rate']:.1%} "
        f"unscouted_buy={unscouted}\n"
        f"  avg_latency={r['avg_latency_ms']}ms  tools={r['tool_categories']}"
    )


def main():
    if len(sys.argv) < 2:
        print("usage: analyze_trajectory.py runs/<run_id>/...")
        sys.exit(2)
    for d in sys.argv[1:]:
        r = analyze(d)
        # Persist the full derived record so analysis data (cost, event types,
        # retry counts, ...) survives without re-parsing raw logs.
        try:
            with open(os.path.join(d, "analysis.json"), "w") as f:
                json.dump(r, f, indent=2, ensure_ascii=False)
        except OSError as e:
            print(f"  ! analysis.json write failed: {e}", file=sys.stderr)
        print(render(r))
        print()


if __name__ == "__main__":
    main()
