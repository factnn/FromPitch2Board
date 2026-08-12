"""Stream a readable view of a Claude Code run while appending the raw JSONL.

Usage (inside run_agent.sh):
  claude -p ... | python3 agents/stream_view.py runs/<dir>/agent_output.jsonl

Each JSONL event is appended to the output file (the machine-readable
trajectory) and a human-readable line is printed for assistant messages and
tool calls, so you can watch the agent play live.
"""

import json
import sys


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else "agent_output.jsonl"
    with open(out_path, "a") as out:
        for line in sys.stdin:
            line = line.rstrip("\n")
            if not line.strip():
                continue
            out.write(line + "\n")
            out.flush()
            try:
                e = json.loads(line)
            except json.JSONDecodeError:
                continue
            t = e.get("type")
            if t == "assistant":
                for c in e.get("message", {}).get("content", []):
                    if c.get("type") == "tool_use":
                        name = c.get("name", "?")
                        inp = json.dumps(c.get("input", {}))
                        if len(inp) > 160:
                            inp = inp[:157] + "..."
                        print(f"  ▶ {name} {inp}", flush=True)
                    elif c.get("type") == "text" and c.get("text"):
                        txt = c["text"].replace("\n", " ")
                        print(f"  💬 {txt[:160]}", flush=True)
            elif t == "result":
                usage = e.get("usage", {})
                print(f"\n  ✅ result (turns={e.get('num_turns')}) "
                      f"in_tok={usage.get('input_tokens', 0)} out_tok={usage.get('output_tokens', 0)} "
                      f"cache_read={usage.get('cache_read_input_tokens', 0)}", flush=True)
                r = e.get("result", "")
                if r:
                    print(f"  └ {r[:300].replace(chr(10), ' ')}", flush=True)


if __name__ == "__main__":
    main()
