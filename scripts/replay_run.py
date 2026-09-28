#!/usr/bin/env python3
"""Replay a recorded episode through the deterministic environment.

The step-level trace (`trajectory.jsonl`) stores the action, the agent reply and
the environment's response, but not the observation the agent saw. That is
enough to re-drive the episode: the simulator is deterministic, so feeding the
recorded action sequence back in reproduces the same decision points. This
script does that and records, for every decision point, which event triggered
the stop — a matchday, a fresh incoming offer, or a transfer-window market day.
The trigger follows the environment's own precedence (matchday > fresh offer >
market day).

Used to attribute skipped decisions (`Continue`) to event type.

Usage:
  python scripts/replay_run.py runs/<run_id> [--mcp target/debug/clubbench-mcp]
"""

import argparse
import asyncio
import json
import socket
import subprocess
import sys
import time
from pathlib import Path

from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

ROOT = Path(__file__).resolve().parents[1]


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_for_port(port: int, tries: int = 200) -> bool:
    for _ in range(tries):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.25):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def original_names(run_dir: Path) -> dict:
    """id -> stable entity name for the original run.

    World generation is seeded, but entity UUIDs are assigned per process
    (`ofm-headless`: "same seed reproduces the same world ... entity UUIDs still
    differ"), so recorded actions reference ids that do not exist in a fresh
    replay. Names are stable, so we translate through them.
    """
    path = run_dir / "trajectory.json"
    if not path.exists():
        return {}
    game = json.loads(path.read_text()).get("final_game", {})
    out = {}
    for p in game.get("players") or []:
        out[p["id"]] = p.get("match_name") or p.get("full_name")
    for t in game.get("teams") or []:
        out[t["id"]] = t.get("name")
    return out


def retarget(action: dict, obs: dict, names: dict) -> dict:
    """Rewrite a recorded action's entity ids onto the replay's ids."""
    a = json.loads(json.dumps(action))
    if a.get("params") is None:  # unit variants (e.g. Continue) carry no params
        return a
    params = a.get("params") or {}
    by_name = {}
    for key in ("squad", "market", "scout_reports"):
        for item in obs.get(key) or []:
            name = item.get("player_name") or item.get("name")
            pid = item.get("player_id") or item.get("id")
            if name and pid:
                by_name[name] = pid

    def remap(pid):
        name = names.get(pid)
        return by_name.get(name, pid) if name else pid

    if "player_ids" in params:
        params["player_ids"] = [remap(p) for p in params["player_ids"]]
    for key in ("player_id", "player_out_id", "player_in_id"):
        if key in params:
            params[key] = remap(params[key])
    if "offer_id" in params:
        target = params.get("player_id")
        for offer in obs.get("offers") or []:
            if offer.get("player_id") == target:
                params["offer_id"] = offer["offer_id"]
                break
    a["params"] = params
    return a


async def replay(run_dir: Path, mcp_bin: Path, out_path: Path, cp_dir: Path | None = None,
                 days: int | None = None):
    md = json.loads((run_dir / "metadata.json").read_text())
    recs = [json.loads(l) for l in (run_dir / "trajectory.jsonl").read_text().splitlines() if l.strip()]
    names = original_names(run_dir)
    manager = md.get("track") == "manager"

    port = free_port()
    log = open(out_path.with_suffix(".mcplog"), "wb")
    proc = subprocess.Popen([str(mcp_bin), "--port", str(port)], stdout=log, stderr=log)
    rows, score = [], None
    try:
        if not wait_for_port(port):
            raise RuntimeError("mcp server did not start")
        async with streamable_http_client(f"http://127.0.0.1:{port}/mcp") as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                await session.call_tool(
                    "reset",
                    {
                        "seed": md["seed"],
                        "scenario": md["scenario"],
                        "club": md["club"],
                        "world": md["world"],
                        "mode": md["track"],
                        "days": days or md["horizon_days"],
                        "seasons": md["seasons"],
                        "anonymize": md.get("anonymized", True),
                        "match_stops": False,
                        **({"cp_dir": str(cp_dir)} if cp_dir else {}),
                    },
                )
                seen_offers = set()
                for i, rec in enumerate(recs):
                    obs = json.loads((await session.call_tool("observe", {})).content[0].text)
                    offers = obs.get("offers") or []
                    fresh = [o for o in offers if o.get("id") not in seen_offers]
                    if obs.get("is_matchday"):
                        kind = "matchday"
                    elif manager and fresh:
                        kind = "offer"
                    else:
                        kind = "market"
                    seen_offers.update(o.get("id") for o in offers)
                    name = (rec.get("action") or {}).get("action")
                    rows.append(
                        {
                            "step": i + 1,
                            "date": obs.get("date"),
                            "kind": kind,
                            "action": name,
                            "noop": name == "Continue",
                            "n_offers": len(offers),
                            "window_open": obs.get("transfer_window_open"),
                            "done": obs.get("done"),
                        }
                    )
                    await session.call_tool("act", {"action": retarget(rec["action"], obs, names)})
                    if obs.get("done"):
                        break
                score = json.loads((await session.call_tool("score", {})).content[0].text)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        log.close()
    return rows, score


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run_dir", type=Path)
    ap.add_argument("--mcp", default=str(ROOT / "target/debug/clubbench-mcp"))
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--cp-dir", type=Path, default=None,
                    help="write simulator checkpoints here while replaying, so a "
                         "run truncated by its day cap can be resumed (--days "
                         "raises the cap stored in the checkpoint)")
    ap.add_argument("--days", type=int, default=None)
    args = ap.parse_args()

    out = args.out or (args.run_dir / "replay_steps.jsonl")
    rows, score = asyncio.run(replay(args.run_dir, Path(args.mcp), out, args.cp_dir, args.days))
    out.write_text("\n".join(json.dumps(r) for r in rows) + "\n")

    recorded = json.loads((args.run_dir / "score.json").read_text())
    same = all(recorded.get(k) == score.get(k) for k in ("points", "position", "goal_difference"))
    print(json.dumps({"run": args.run_dir.name, "steps": len(rows), "replay_matches": same,
                      "recorded": {k: recorded.get(k) for k in ("points", "position")},
                      "replayed": {k: score.get(k) for k in ("points", "position")}}))
    return 0 if same else 1


if __name__ == "__main__":
    sys.exit(main())
