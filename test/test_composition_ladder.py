"""Composition Ladder: the C0 Coach / C1 Recruiter / C2 Manager rungs gate the
responsibility scope precisely. C1 unlocks scouting/buying but NOT selling.

Checks: observation visibility (market hidden for Coach, offers for
Manager only) and action gating (transfer actions rejected with a result
message instead of silently applied).
"""

import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

import pytest

REPO = str(Path(__file__).resolve().parent.parent)
BIN = os.path.join(REPO, "target/debug/clubbench-mcp")


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
def episode(tmp_path):
    if not os.path.exists(BIN):
        pytest.skip("clubbench-mcp binary not built")
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
                 "world": "compact", "days": 30, "mode": "coach",
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


def act(url, action, params=None):
    body = {"action": action, "params": params}
    return mcp(url, "act", json.dumps({"action": body}))


def test_coach_rejects_transfers_and_hides_market(episode):
    url = episode
    obs = mcp(url, "observe")
    assert obs["market"] == [], "coach must not see the market"
    r = act(url, "MakeBid", {"player_id": "x", "fee": 1000})
    assert "not available" in (r.get("last_action_result") or "")


def test_recruiter_allows_buying_but_not_selling(episode):
    url = episode
    mcp(url, "reset", json.dumps({
        "seed": 42, "scenario": "rebuild", "club": 75,
        "world": "compact", "days": 30, "mode": "recruiter", "anonymize": True}))
    obs = mcp(url, "observe")
    assert obs["offers"] == [], "recruiter must not see sell-side offers"
    r = act(url, "ListPlayer", {"player_id": "x"})
    assert "not available" in (r.get("last_action_result") or ""), "recruiter must not list players"
    # Buying side is open: Scout on a bogus id still counts as an allowed
    # action (the env reports the scout outcome, not a scope lock).
    r = act(url, "Scout", {"player_id": "bogus-id"})
    assert "not available" not in (r.get("last_action_result") or ""), "recruiter must be able to scout"


def test_manager_allows_everything(episode):
    url = episode
    mcp(url, "reset", json.dumps({
        "seed": 42, "scenario": "rebuild", "club": 75,
        "world": "compact", "days": 30, "mode": "manager", "anonymize": True}))
    r = act(url, "ListPlayer", {"player_id": "bogus-id"})
    assert "not available" not in (r.get("last_action_result") or ""), "manager must be able to list"
