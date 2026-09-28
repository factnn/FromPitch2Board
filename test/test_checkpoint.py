"""Checkpoint/resume: an interrupted episode resumes bit-identically.

The MCP server checkpoints the episode every CHECKPOINT_STEPS acts (game db +
counters). Killing the server mid-episode and resetting from the checkpoint
must continue deterministically — the keyed RNG re-seeds from the original
episode seed, so day/match draws are identical to an uninterrupted run.
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
BIN = os.path.join(REPO, "target/debug/frompitch2board-mcp")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def start_server(port, log_path):
    return subprocess.Popen([BIN, "--port", str(port)],
                            stdout=open(log_path, "w"), stderr=subprocess.STDOUT)


def mcp(url, tool, args="{}"):
    r = subprocess.run(
        ["python3", os.path.join(REPO, "agents/mcp_call.py"), "--url", url,
         tool, args],
        capture_output=True, text=True, timeout=120)
    return json.loads(r.stdout)


def wait_server(url, proc, reset_args):
    """Wait for the server AND reset it (transport probe = a real reset)."""
    for _ in range(100):
        if proc.poll() is not None:
            pytest.fail("server died at startup")
        r = subprocess.run(
            ["python3", os.path.join(REPO, "agents/mcp_call.py"), "--url", url,
             "reset", reset_args],
            capture_output=True, text=True, timeout=30)
        try:
            return json.loads(r.stdout)
        except ValueError:
            time.sleep(0.3)
    pytest.fail("server never became ready")


def play(url, n_acts):
    """Scripted deterministic policy: Continue, Continue, ... (engine auto-XI)."""
    obs = mcp(url, "observe")
    for _ in range(n_acts):
        obs = mcp(url, "act",
                  json.dumps({"action": {"action": "Continue", "params": None}}))
    return obs, mcp(url, "score")


def test_checkpoint_resume_is_deterministic(tmp_path):
    if not os.path.exists(BIN):
        pytest.skip("frompitch2board-mcp binary not built")
    cp_dir = str(tmp_path / "checkpoint")

    # --- interrupted run: 12 acts, then kill the server ---
    port1 = free_port()
    url1 = f"http://127.0.0.1:{port1}/mcp"
    s1 = start_server(port1, str(tmp_path / "s1.log"))
    args1 = json.dumps({
        "seed": 42, "scenario": "crisis", "club": 5, "world": "compact",
        "mode": "coach", "days": 60, "anonymize": True, "cp_dir": cp_dir,
    })
    wait_server(url1, s1, args1)
    obs_mid, _ = play(url1, 12)
    assert os.path.exists(os.path.join(cp_dir, "state.json")), "checkpoint not written"
    assert os.path.exists(os.path.join(cp_dir, "game.db")), "game db not written"
    s1.terminate()
    s1.wait()

    # --- resume on a FRESH server from the same cp_dir ---
    port2 = free_port()
    url2 = f"http://127.0.0.1:{port2}/mcp"
    s2 = start_server(port2, str(tmp_path / "s2.log"))
    obs_resumed = wait_server(url2, s2, json.dumps({
        "seed": 42, "scenario": "crisis", "club": 5, "world": "compact",
        "mode": "coach", "days": 60, "anonymize": True, "cp_dir": cp_dir,
    }))
    # Resume restarts from the LAST checkpoint (every 8 acts), not the exact
    # crash point: 12 acts played → checkpoint at step 8 is the resume point.
    assert obs_resumed["step"] == 8, obs_resumed["step"]
    _, score_resumed = play(url2, 22)  # 8 + 22 = 30 acts total
    s2.terminate()
    s2.wait()

    # --- uninterrupted reference run, same 30 acts, fresh cp_dir ---
    port3 = free_port()
    url3 = f"http://127.0.0.1:{port3}/mcp"
    s3 = start_server(port3, str(tmp_path / "s3.log"))
    wait_server(url3, s3, json.dumps({
        "seed": 42, "scenario": "crisis", "club": 5, "world": "compact",
        "mode": "coach", "days": 60, "anonymize": True,
        "cp_dir": str(tmp_path / "cp_ref"),
    }))
    _, score_ref = play(url3, 30)
    s3.terminate()
    s3.wait()

    # Bit-identical continuation: the interrupted run's final state equals the
    # uninterrupted one's.
    assert score_resumed == score_ref, (score_resumed, score_ref)
