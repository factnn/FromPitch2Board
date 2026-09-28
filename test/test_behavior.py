"""P0 logging protocol: every run persists metadata.json + trajectory.jsonl
(step-level decision trace) + usage.json, and analyze_trajectory.py derives
behavior/reliability stats from them — all re-runnable without the agent."""

import asyncio
import json
import socket
import subprocess
import sys
import time

import pytest
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

from conftest import BIN_DIR
from pathlib import Path


def _free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


@pytest.fixture(scope="module")
def server():
    port = _free_port()
    proc = subprocess.Popen(
        [str(BIN_DIR / "frompitch2board-mcp"), "--port", str(port)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    for _ in range(50):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                break
        except OSError:
            time.sleep(0.1)
    yield f"http://127.0.0.1:{port}/mcp"
    proc.terminate()
    proc.wait()


def test_mcp_step_trace_written(tmp_path, server):
    async def drive():
        async with streamable_http_client(server) as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                await session.call_tool("reset", {
                    "seed": 42, "scenario": "rebuild", "club": 5,
                    "world": "compact", "mode": "manager", "days": 20,
                })
                for _ in range(3):
                    await session.call_tool("act", {"action": {"action": "Continue", "params": None}})
                await session.call_tool("dump", {"path": str(tmp_path / "trajectory.json"), "agent": "cc"})
    asyncio.run(drive())

    # trajectory.jsonl has the step-level decision trace
    traj = tmp_path / "trajectory.jsonl"
    assert traj.exists()
    steps = [json.loads(l) for l in traj.read_text().splitlines() if l.strip()]
    assert len(steps) >= 3
    assert steps[0]["action"]["action"] == "Continue"
    assert "latency_ms" in steps[0] and "tool_success" in steps[0]

    # the trajectory record itself is re-scoreable
    rec = json.loads((tmp_path / "trajectory.json").read_text())
    assert rec["agent"] == "cc" and rec["mode"] == "Manager"


def test_analyze_trajectory_derived_stats(tmp_path, server):
    # a minimal run dir: trajectory.jsonl + metadata + usage
    (tmp_path / "trajectory.jsonl").write_text(json.dumps({
        "step": 1, "date": "2026-07-01", "action": {"action": "MakeBid", "params": {"player_id": "p1", "fee": 1}},
        "latency_ms": 100, "input_tokens": 10, "output_tokens": 2, "tool_success": True,
    }) + "\n" + json.dumps({
        "step": 2, "date": "2026-07-02", "action": {"action": "MakeBid", "params": {"player_id": "p1", "fee": 1}},
        "latency_ms": 100, "input_tokens": 10, "output_tokens": 2, "tool_success": False,
    }) + "\n")
    (tmp_path / "metadata.json").write_text(json.dumps({"agent": "test", "track": "manager", "scenario": "rebuild", "seed": 42}))
    (tmp_path / "usage.json").write_text(json.dumps({"input_tokens": 20, "output_tokens": 4, "llm_calls": 2, "completed": True}))

    r = subprocess.run(
        [sys.executable, "agents/analyze_trajectory.py", str(tmp_path)],
        capture_output=True, text=True,
    )
    assert r.returncode == 0, r.stderr
    # the repeated MakeBid with identical params is flagged as a repeat
    assert "repeat=50.0%" in r.stdout
    # the tool_failure on step 2 is flagged as invalid
    assert "invalid=50.0%" in r.stdout
    assert "tools={'MakeBid': 2}" in r.stdout
    # the full derived record is persisted as analysis.json
    analysis = json.loads((tmp_path / "analysis.json").read_text())
    assert "cost_usd" in analysis and analysis["cost_usd"] >= 0
    assert analysis["invalid_count"] == 1 and analysis["repeat_count"] == 1
    assert analysis["input_tokens"] == 20
