"""last_action_result feedback: the observation reports the outcome of the
previous action, so an agent learns that a bid was rejected/accepted instead of
blindly repeating it (the fix for the "repeat-bid loop" found in the audit)."""

import asyncio
import json
import socket
import subprocess
import time

import pytest
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

from conftest import BIN_DIR


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


def _drive(server, fn):
    async def run():
        async with streamable_http_client(server) as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                return await fn(session)
    return asyncio.run(run())


def test_first_observation_has_no_result(server):
    async def fn(session):
        await session.call_tool("reset", {
            "seed": 42, "scenario": "rebuild", "club": 15,
            "world": "compact", "mode": "manager", "days": 60,
        })
        obs = json.loads((await session.call_tool("observe", {})).content[0].text)
        assert "last_action_result" not in obs  # nothing happened yet
    _drive(server, fn)


def test_setmatchplan_reports_lineup(server):
    async def fn(session):
        await session.call_tool("reset", {
            "seed": 42, "scenario": "rebuild", "club": 15,
            "world": "compact", "mode": "coach", "days": 60,
        })
        obs = json.loads((await session.call_tool("observe", {})).content[0].text)
        ids = [p["id"] for p in obs["squad"][:11]]
        obs = json.loads((await session.call_tool("act", {
            "action": {"action": "SetMatchPlan", "params": {"player_ids": ids, "play_style": "Attacking"}}
        })).content[0].text)
        assert "last_action_result" in obs
        assert "Set lineup" in obs["last_action_result"]
    _drive(server, fn)


def test_bid_outcome_reported(server):
    async def fn(session):
        await session.call_tool("reset", {
            "seed": 42, "scenario": "rebuild", "club": 15,
            "world": "compact", "mode": "manager", "days": 60,
        })
        obs = json.loads((await session.call_tool("observe", {})).content[0].text)
        target = obs["market"][0]["player_id"]
        obs = json.loads((await session.call_tool("act", {
            "action": {"action": "MakeBid", "params": {"player_id": target, "fee": 5_000_000}}
        })).content[0].text)
        res = obs["last_action_result"]
        assert "Bid of £5000000" in res
        # It must tell the agent the outcome, never silently omit it.
        assert any(k in res for k in ("ACCEPTED", "REJECTED", "countered", "failed"))
    _drive(server, fn)


def test_continue_reports_advance(server):
    async def fn(session):
        await session.call_tool("reset", {
            "seed": 42, "scenario": "rebuild", "club": 15,
            "world": "compact", "mode": "coach", "days": 60,
        })
        obs = json.loads((await session.call_tool("act", {
            "action": {"action": "Continue", "params": None}
        })).content[0].text)
        assert "Continued" in obs["last_action_result"]
    _drive(server, fn)
