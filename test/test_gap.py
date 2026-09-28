"""Composition Gap: given a coach + manager trajectory pair for the
same agent, report BOTH the relative gap G_Z and the raw composition delta
G_Δ = Δ_coach − Δ_manager (in league points), each vs the per-track Greedy
reference."""

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


async def _drive_and_dump(session, mode, path):
    await session.call_tool("reset", {
        "seed": 42, "scenario": "rebuild", "club": 5, "world": "compact",
        "mode": mode, "days": 30,
    })
    obs = json.loads((await session.call_tool("observe", {})).content[0].text)
    guard = 0
    while not obs.get("done") and guard < 200:
        obs = json.loads((await session.call_tool(
            "act", {"action": {"action": "Continue", "params": None}})).content[0].text)
        guard += 1
    await session.call_tool("dump", {"path": path, "agent": "llm"})


def test_gap_computes_both_metrics(tmp_path, server):
    async def run():
        async with streamable_http_client(server) as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                coach = str(tmp_path / "coach.json")
                mgr = str(tmp_path / "manager.json")
                await _drive_and_dump(session, "coach", coach)
                await _drive_and_dump(session, "manager", mgr)
    asyncio.run(run())

    r = frompitch2board("gap", str(tmp_path / "coach.json"), str(tmp_path / "manager.json"))
    assert r.returncode == 0, r.stderr
    assert "G_Δ(pts)" in r.stdout  # header
    assert "llm" in r.stdout       # the agent row is reported
