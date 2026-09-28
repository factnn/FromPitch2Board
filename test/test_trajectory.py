"""Trajectory/scoring decoupling: run_agent.sh persists a TrajectoryRecord
(final game state + metadata) via the MCP `dump` tool, and `frompitch2board
score-trajectory` re-scores it without re-running the agent."""

import asyncio
import json
import socket
import subprocess
import time

import pytest
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

from conftest import BIN_DIR, frompitch2board


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


def test_dump_and_rescore(tmp_path, server):
    out = tmp_path / "trajectory.json"

    async def drive():
        async with streamable_http_client(server) as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                await session.call_tool("reset", {
                    "seed": 42, "scenario": "rebuild", "club": 15,
                    "world": "compact", "mode": "manager", "days": 40,
                })
                # drive a couple of steps so the state isn't the initial one
                for _ in range(3):
                    await session.call_tool("act", {
                        "action": {"action": "Continue", "params": None}
                    })
                live = json.loads((await session.call_tool("score", {})).content[0].text)
                await session.call_tool("dump", {"path": str(out)})
                return live
    live = asyncio.run(drive())

    # the trajectory file exists and carries metadata + a scoring game state
    rec = json.loads(out.read_text())
    assert rec["seed"] == 42 and rec["scenario"] == "rebuild" and rec["club"] == 15
    assert rec["mode"] == "Manager" and "final_game" in rec
    assert "initial_net_worth" in rec and "net_spend" in rec

    # re-score the saved trajectory via the CLI without re-running the agent
    r = frompitch2board("score-trajectory", str(out))
    assert r.returncode == 0, r.stderr
    # the re-scored metrics match what the live MCP score tool reported
    assert str(live["points"]) in r.stdout
    assert str(live["net_value"]) in r.stdout
    assert str(live["net_spend"]) in r.stdout
