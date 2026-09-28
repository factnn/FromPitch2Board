"""Group A: a standardized LLM harness — the same thin scaffold, any model.

This is the "Model Track" subject: a fixed loop (observe → prompt → LLM →
one action → act) with a model-swappable backbone. Unlike Claude Code / Codex
(off-the-shelf, opaque), this harness is transparent and controlled: the prompt,
tools and loop are identical for every model, so differences in results isolate
the foundation model's agentic management capability.

It drives a ClubBench MCP server (which run_agent.sh has already reset). The
LLM is called via the Anthropic-compatible messages API, configured by env:
  ANTHROPIC_BASE_URL, ANTHROPIC_AUTH_TOKEN, ANTHROPIC_MODEL
(defaults point at DeepSeek). Set `--model` to override.

Usage (normally via run_agent.sh --agent llm):
  python agents/llm_agent.py --url http://127.0.0.1:PORT/mcp --prompt prompt.md [--model ...]
"""

import json
import os
import sys
import time
import urllib.request

from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

try:
    from exceptiongroup import BaseExceptionGroup  # Python < 3.11
except ImportError:
    pass


def _is_session_noise(exc: BaseException) -> bool:
    """True if exc is (possibly nested) streamable-http session-close noise.

    The client raises "Session termination failed: 202" inside an
    ExceptionGroup when tearing down the async HTTP session; only suppress it
    when every sub-exception is noise so real errors still surface.
    """
    if isinstance(exc, BaseExceptionGroup):
        return all(_is_session_noise(e) for e in exc.exceptions)
    msg = str(exc)
    return "Session termination failed" in msg or "202" in msg


def call_llm(prompt: str, model: str) -> tuple[str, dict, str]:
    """Call the Anthropic-compatible messages API.

    Returns (text, usage, stop_reason). Following the same-ceiling principle
    max_tokens is a generous ceiling so the models'
    actual token consumption is their own choice; finish_reason and reasoning
    tokens are recorded so the paper can answer "does Pro win by thinking
    more" with data. Env knobs: MAX_TOKENS / THINKING_BUDGET (defaults are
    deliberately loose; the old 2048/8192 cap handicapped deep thinkers).
    """
    base = os.environ.get("ANTHROPIC_BASE_URL", "https://api.deepseek.com/anthropic")
    # Some proxies ship their key as ANTHROPIC_API_KEY (zyapi); accept either.
    token = os.environ.get("ANTHROPIC_AUTH_TOKEN") or os.environ.get("ANTHROPIC_API_KEY", "")
    url = base.rstrip("/") + "/v1/messages"
    max_tokens = int(os.environ.get("MAX_TOKENS", "32768"))
    thinking_budget = int(os.environ.get("THINKING_BUDGET", "16384"))
    body = {
        "model": model,
        "max_tokens": max_tokens,
        "thinking": {"type": "enabled", "budget_tokens": thinking_budget},
        "messages": [{"role": "user", "content": prompt}],
    }
    # Default browser UA: some model proxies sit behind Cloudflare, which
    # hard-blocks the "Python-urllib" UA with error 1010 (the Opus 5 proxy
    # does exactly this — curl passes, urllib gets 403). Harmless on direct
    # APIs; override via LLM_USER_AGENT if a provider demands a specific one.
    ua = os.environ.get(
        "LLM_USER_AGENT",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
    )
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode(),
        headers={
            "Content-Type": "application/json",
            "User-Agent": ua,
            "x-api-key": token,
            "Authorization": f"Bearer {token}",
            "anthropic-version": "2023-06-01",
        },
    )
    # Transient API failures (timeouts, 429/5xx, connection resets) must not
    # kill a 3-hour 10Y run. Escalating backoff: 8 parallel cells hammering a
    # rate-limited proxy (the Opus grid hit 429 storms) recover instead of
    # dying mid-season — each dead turn used to cost a harness relaunch and
    # zeroed this turn's usage.
    for attempt, backoff in enumerate((10, 20, 40, 60), start=1):
        try:
            with urllib.request.urlopen(req, timeout=180) as r:
                resp = json.load(r)
            break
        except Exception as e:
            if attempt == 5:
                raise
            print(f"  ! transient API error (attempt {attempt}): {e} — retry in {backoff}s", flush=True)
            time.sleep(backoff)
    text = "".join(b.get("text", "") for b in resp.get("content", []) if b.get("type") == "text")
    usage = resp.get("usage", {})
    # The endpoint does NOT report reasoning tokens in usage (probe-verified);
    # the thinking block content is the only signal — record its length and
    # estimate tokens (≈ 4 chars/token) so the paper can measure thinking.
    thinking_chars = sum(len(b.get("thinking", "")) for b in resp.get("content", []) if b.get("type") == "thinking")
    usage["thinking_chars"] = thinking_chars
    usage["reasoning_tokens"] = thinking_chars // 4
    stop_reason = resp.get("stop_reason", "")
    return text, usage, stop_reason


def parse_action(text: str):
    """Extract the first balanced JSON object from the model's reply."""
    start = text.find("{")
    if start < 0:
        raise ValueError(f"no JSON object in reply: {text[:120]}")
    depth = 0
    in_str = False
    for i in range(start, len(text)):
        ch = text[i]
        if ch == '"' and (i == 0 or text[i - 1] != "\\"):
            in_str = not in_str
        elif not in_str:
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    return json.loads(text[start : i + 1])
    raise ValueError(f"unbalanced JSON in reply: {text[:120]}")


async def run(url: str, briefing: str, model: str, out_dir: str, max_steps: int = 400):
    import time
    steps = 0
    usage_totals = {"input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0,
                    "reasoning_tokens": 0, "llm_calls": 0}
    # The harness relaunches this script once per "turn" until the season is
    # done; a fresh relaunch used to overwrite usage.json with this turn's
    # zeros before the first call. Accumulate across turns instead — cost data
    # for the paper must survive a mid-season restart.
    prev_usage = os.path.join(out_dir, "usage.json")
    if os.path.exists(prev_usage):
        try:
            with open(prev_usage) as uf:
                for k, v in json.load(uf).items():
                    usage_totals[k] = v
        except (ValueError, OSError):
            pass
    start_wall = time.time()
    traj_path = os.path.join(out_dir, "trajectory.jsonl")
    async with streamable_http_client(url) as conn:
        read, write = conn[0], conn[1]
        async with ClientSession(read, write) as session:
            await session.initialize()
            r = await session.call_tool("observe", {})
            obs = json.loads(r.content[0].text)
            while not obs.get("done") and steps < max_steps:
                steps += 1
                prompt = (
                    briefing
                    + "\n\n## CURRENT STATE\n```json\n"
                    + json.dumps(obs)
                    + "\n```\n\nReturn exactly ONE action as a JSON object "
                    'of the form {"action": "...", "params": {...}}. '
                    "No prose."
                )
                t0 = time.time()
                reply, usage, stop_reason = call_llm(prompt, model)
                latency_ms = int((time.time() - t0) * 1000)
                final_reply = reply
                try:
                    action = parse_action(reply)
                except ValueError as e:
                    # The model sometimes spent its whole output budget on a
                    # thinking block and returned an empty text block. Retry
                    # once with a directive nudge before falling back, so a
                    # long-horizon episode doesn't waste steps on Continues.
                    retry = (
                        prompt
                        + '\n\nReply with ONLY a JSON object of the form '
                        '{"action": "..", "params": {...}}. No thinking, no prose.'
                    )
                    print(f"  ! step {steps}: parse failed ({e}) — retrying", flush=True)
                    t1 = time.time()
                    reply2, usage2, stop_reason2 = call_llm(retry, model)
                    latency_ms += int((time.time() - t1) * 1000)
                    usage = {
                        "input_tokens": usage.get("input_tokens", 0) + usage2.get("input_tokens", 0),
                        "output_tokens": usage.get("output_tokens", 0) + usage2.get("output_tokens", 0),
                        "cache_read_input_tokens": usage.get("cache_read_input_tokens", 0) + usage2.get("cache_read_input_tokens", 0),
                        "reasoning_tokens": usage.get("reasoning_tokens", 0) + usage2.get("reasoning_tokens", 0),
                    }
                    stop_reason = stop_reason2
                    usage_totals["llm_calls"] += 1  # the retry is an extra call
                    final_reply = reply2
                    try:
                        action = parse_action(reply2)
                    except ValueError as e2:
                        print(f"  ! step {steps}: retry also failed ({e2}) — continue", flush=True)
                        action = {"action": "Continue", "params": None}

                for k in ("input_tokens", "output_tokens", "cache_read_input_tokens", "reasoning_tokens"):
                    usage_totals[k] += usage.get(k, 0)
                usage_totals["llm_calls"] += 1

                date = obs.get("date", "")
                r = await session.call_tool("act", {"action": action})
                tool_success = not getattr(r, "isError", False)
                obs = json.loads(r.content[0].text)
                result = obs.get("last_action_result")

                # Lossless step-level trajectory — including the RAW model reply
                # so failure analysis can see exactly what the model produced
                # (not just the parsed action), like cc's stream-json output.
                with open(traj_path, "a") as tf:
                    tf.write(json.dumps({
                        "step": steps,
                        "date": date,
                        "action": action,
                        "reply": final_reply,
                        "result": result,
                        "latency_ms": latency_ms,
                        "input_tokens": usage.get("input_tokens", 0),
                        "output_tokens": usage.get("output_tokens", 0),
                        "cache_read_tokens": usage.get("cache_read_input_tokens", 0),
                        "reasoning_tokens": usage.get("reasoning_tokens", 0),
                        "stop_reason": stop_reason,
                        "tool_success": tool_success,
                    }, ensure_ascii=False) + "\n")

                # stdout stream keeps the live view + token_report working.
                print(f"  ▶ {json.dumps(action)[:160]}", flush=True)
                # Live raw-reply event (stream_view appends it to
                # agent_output.jsonl, so you can watch the model output while
                # the episode is still running).
                print(json.dumps({"type": "llm_reply", "step": steps, "reply": final_reply}, ensure_ascii=False), flush=True)
                print(json.dumps({"type": "assistant", "usage": {
                    "input_tokens": usage.get("input_tokens", 0),
                    "output_tokens": usage.get("output_tokens", 0),
                    "cache_read_input_tokens": usage.get("cache_read_input_tokens", 0),
                }}), flush=True)

            # Aggregated usage + episode summary (input/output kept separate).
            usage_totals["steps"] = steps
            usage_totals["completed"] = bool(obs.get("done"))
            usage_totals["wall_time_s"] = round(time.time() - start_wall, 2)
            with open(os.path.join(out_dir, "usage.json"), "w") as uf:
                json.dump(usage_totals, uf, indent=2)
            print(f"done after {steps} steps", flush=True)
            r = await session.call_tool("score", {})
            print("SCORE:", r.content[0].text[:400], flush=True)


def main():
    args = sys.argv[1:]
    url = "http://127.0.0.1:8890/mcp"
    prompt_file = None
    model = os.environ.get("ANTHROPIC_MODEL", "deepseek-v4-flash")
    while args:
        if args[0] == "--url":
            url = args[1]; args = args[2:]
        elif args[0] == "--prompt":
            prompt_file = args[1]; args = args[2:]
        elif args[0] == "--model":
            model = args[1]; args = args[2:]
        else:
            print(f"unknown arg: {args[0]}", file=sys.stderr); sys.exit(2)
    briefing = open(prompt_file).read() if prompt_file else (
        "You are an autonomous football club manager. Use the ClubBench MCP "
        "tools (observe / act / score) to manage the club through the season."
    )
    # Logging lives next to the prompt (run_agent.sh's RUN_DIR).
    out_dir = os.path.dirname(os.path.abspath(prompt_file)) if prompt_file else os.getcwd()
    import asyncio
    # Long-horizon runs need more steps than a single 400-day season (~43):
    # 3Y ≈ 130, 10Y ≈ 430+. MAX_STEPS env override (default 400 unchanged).
    max_steps = int(os.environ.get("MAX_STEPS", "400"))
    try:
        asyncio.run(run(url, briefing, model, out_dir, max_steps))
    except BaseException as e:
        # ignore the MCP client's session-close noise (which arrives wrapped in
        # an ExceptionGroup); any real error must surface.
        if _is_session_noise(e):
            pass
        else:
            raise


if __name__ == "__main__":
    main()
