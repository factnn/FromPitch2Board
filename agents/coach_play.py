#!/usr/bin/env python3
"""Play a ClubBench coach-mode episode via MCP, applying a condition-aware
best-XI policy.

Selection: effective_rating = ovr * condition/100 (matches the engine's
player_rating::effective_rating_for_assignment). Pick top-N per position
group for the current formation (4-4-2 -> 1 GK / 4 DEF / 4 MID / 2 FWD).
Play style: Attacking.
"""

import json
import subprocess
import sys
import time

URL = "http://127.0.0.1:53705/mcp"
SCRIPT = str(Path(__file__).resolve().parent / "mcp_call.py")
PLAY_STYLE = "Attacking"


def needed_for(formation: str) -> dict:
    """Parse '4-4-2' style formation into group counts."""
    parts = [int(x) for x in formation.replace(" ", "").split("-") if x.isdigit()]
    counts = {"Goalkeeper": 1}
    if len(parts) >= 3:
        # e.g. 4-4-2 -> 4 DEF, 4 MID, 2 FWD
        counts["Defender"] = parts[0]
        counts["Midfielder"] = parts[1]
        counts["Forward"] = parts[2]
    else:
        counts.update({"Defender": 4, "Midfielder": 4, "Forward": 2})
    return counts


def mcp(tool: str, args: dict = None) -> dict:
    cmd = [sys.executable, SCRIPT, "--url", URL, tool]
    if args is not None:
        cmd.append(json.dumps(args))
    for attempt in range(5):
        try:
            out = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
            if out.returncode != 0:
                raise RuntimeError(out.stderr[-2000:])
            return json.loads(out.stdout)
        except Exception as e:
            time.sleep(2 * (attempt + 1))
    raise RuntimeError(f"mcp {tool} failed after retries")


def pick_xi(squad, formation):
    by_group = {}
    for p in squad:
        if p.get("injured"):
            continue
        by_group.setdefault(p["group_position"], []).append(p)
    xi = []
    for group, n in needed_for(formation).items():
        players = by_group.get(group, [])
        players.sort(key=lambda p: p["ovr"] * p["condition"] / 100.0, reverse=True)
        xi.extend(p["id"] for p in players[:n])
    return xi


def main():
    step = 0
    results = []
    while True:
        obs = mcp("observe")
        step = obs.get("step", step)
        date = obs.get("date", "?")
        if obs.get("done"):
            print(f"DONE at step {step} {date}", flush=True)
            break
        if obs.get("is_matchday"):
            xi = pick_xi(obs["squad"], obs.get("formation", "4-4-2"))
            action = {"action": "SetMatchPlan",
                      "params": {"player_ids": xi, "play_style": PLAY_STYLE}}
            act_obs = mcp("act", {"action": action})
            results.append({
                "date": date,
                "fixture": obs.get("next_fixture"),
                "position": act_obs.get("league_position"),
                "points": act_obs.get("points"),
            })
            if step % 5 == 0:
                print(f"step {step} {date}: pos={act_obs.get('league_position')} "
                      f"pts={act_obs.get('points')} xi_ovr={sum(p['ovr'] for p in obs['squad'] if p['id'] in xi)}",
                      flush=True)
        else:
            mcp("act", {"action": {"action": "Continue", "params": None}})

    # Final summary
    score = mcp("score")
    print("SCORE:", json.dumps(score, indent=2))
    # Write trajectory
    traj = sys.argv[1]
    try:
        mcp("dump", {"path": traj, "agent": "cc-coach"})
        print(f"trajectory -> {traj}")
    except Exception as e:
        print(f"dump failed: {e}")
    with open(Path(traj).with_name("match_log.json"), "w") as f:
        json.dump(results, f, indent=2)


if __name__ == "__main__":
    main()
