"""L1 Match track: with match_stops the episode pauses user matches at
30'/HT/60'/75' for live substitutions and tactic changes, then resumes the
ordinary decision cadence. Off by default — classic episodes never see a
live_match block.

Checks: checkpoint flow, in-match action gating, per-match determinism under
the keyed RNG, and the unchanged default cadence.
"""

import json
import os
from pathlib import Path
import socket
import subprocess
import time

import pytest

REPO = str(Path(__file__).resolve().parent.parent)
BIN = os.path.join(REPO, "target/debug/frompitch2board-mcp")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def mcp(url, tool, args="{}"):
    r = subprocess.run(
        ["python3", os.path.join(REPO, "agents/mcp_call.py"), "--url", url,
         tool, args],
        capture_output=True, text=True, timeout=120)
    return json.loads(r.stdout)


@pytest.fixture()
def server(tmp_path):
    if not os.path.exists(BIN):
        pytest.skip("frompitch2board-mcp binary not built")
    port = free_port()
    url = f"http://127.0.0.1:{port}/mcp"
    proc = subprocess.Popen([BIN, "--port", str(port)],
                            stdout=open(str(tmp_path / "s.log"), "w"),
                            stderr=subprocess.STDOUT)
    for _ in range(100):
        r = subprocess.run(
            ["python3", os.path.join(REPO, "agents/mcp_call.py"), "--url", url,
             "reset", json.dumps({
                 "seed": 42, "scenario": "rebuild", "club": 75,
                 "world": "compact", "days": 60, "mode": "coach",
                 "anonymize": True})],
            capture_output=True, text=True, timeout=30)
        try:
            json.loads(r.stdout)
            break
        except ValueError:
            time.sleep(0.3)
    yield url
    proc.terminate()
    proc.wait()


def reset(url, **overrides):
    args = {"seed": 42, "scenario": "rebuild", "club": 75,
            "world": "compact", "days": 60, "mode": "coach",
            "anonymize": True}
    args.update(overrides)
    return mcp(url, "reset", json.dumps(args))


def act(url, action, params=None):
    body = {"action": action, "params": params}
    return mcp(url, "act", json.dumps({"action": body}))


def play_until_match_stop(url):
    """Set a match plan, then step until an in-match stop is reached."""
    obs = act(url, "SetMatchPlan", {"player_ids": [], "play_style": "Attacking"})
    guard = 0
    while obs.get("live_match") is None and not obs.get("done") and guard < 200:
        obs = act(url, "Continue")
        guard += 1
    return obs


def test_match_stops_pause_at_checkpoints_and_resume(server):
    url = server
    reset(url, match_stops=True)
    obs = play_until_match_stop(url)
    assert obs.get("live_match") is not None, "match_stops episode must pause in-match"
    minutes = [obs["live_match"]["minute"]]
    assert obs["live_match"]["minute"] == 30, "first stop must be at 30'"
    for _ in range(3):  # HT / 60' / 75'
        obs = act(url, "Continue")
        if obs.get("live_match") is not None:
            minutes.append(obs["live_match"]["minute"])
        else:
            break
    assert minutes == [30, 45, 60, 75], f"checkpoint minutes wrong: {minutes}"
    # One more action finishes the match and resumes the normal cadence.
    obs = act(url, "Continue")
    assert obs.get("live_match") is None, "live_match must clear after the whistle"


def test_match_actions_gate_at_match_stops(server):
    url = server
    reset(url, match_stops=True)
    obs = play_until_match_stop(url)
    lm = obs["live_match"]
    field_ids = [p["id"] for p in lm["field"]]
    bench_ids = [p["id"] for p in lm["bench"]]
    # Substitutions are only allowed when a bench option exists.
    if bench_ids:
        r = act(url, "Substitute", {"player_out_id": field_ids[0],
                                    "player_in_id": bench_ids[0]})
        assert "Substituted" in (r.get("last_action_result") or "")
        assert r["live_match"]["subs_made"] == 1
    r = act(url, "MatchTactics", {"play_style": "Defensive"})
    assert "play style" in (r.get("last_action_result") or "")
    # Non-match actions are locked while the ball is in play.
    r = act(url, "SetMatchPlan", {"player_ids": [], "play_style": "Balanced"})
    assert "only Substitute" in (r.get("last_action_result") or "")


def test_match_stops_replay_deterministically(server):
    url = server
    scores = []
    for seed in (42, 42):
        reset(url, seed=seed, match_stops=True)
        obs = mcp(url, "observe")
        while not obs.get("done"):
            if obs.get("live_match") is not None:
                # A fixed policy: swap the first bench player on at every stop
                # when possible, else hold.
                lm = obs["live_match"]
                if lm["bench"] and lm["subs_made"] < lm["subs_max"]:
                    obs = act(url, "Substitute",
                              {"player_out_id": lm["field"][0]["id"],
                               "player_in_id": lm["bench"][0]["id"]})
                else:
                    obs = act(url, "Continue")
            else:
                obs = act(url, "SetMatchPlan",
                          {"player_ids": [], "play_style": "Attacking"})
        scores.append(mcp(url, "score")["points"])
    assert scores[0] == scores[1], \
        f"same seed + same policy must replay bit-identically, got {scores}"


def test_match_stops_off_keeps_classic_cadence(server):
    url = server
    reset(url, match_stops=False)
    obs = mcp(url, "observe")
    guard = 0
    while not obs.get("done") and guard < 200:
        obs = act(url, "SetMatchPlan",
                  {"player_ids": [], "play_style": "Attacking"})
        guard += 1
        assert obs.get("live_match") is None, \
            "match_stops off must never expose a live_match block"
