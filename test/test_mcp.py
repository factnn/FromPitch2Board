"""MCP server: exposes the headless env to any MCP-capable agent.

The server is started, then a real MCP client connects and drives an episode:
reset → observe → act (Continue) → score. Requires the `mcp` python package
and the compiled `clubbench-mcp` binary.
"""

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
        [str(BIN_DIR / "clubbench-mcp"), "--port", str(port)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    # Wait for the server to accept connections.
    for _ in range(50):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                break
        except OSError:
            time.sleep(0.1)
    yield f"http://127.0.0.1:{port}/mcp"
    proc.terminate()
    proc.wait()


def _run(session, name, args):
    return asyncio.get_event_loop().run_until_complete(session.call_tool(name, args))


def test_mcp_drives_an_episode(server):
    async def drive():
        async with streamable_http_client(server) as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                tools = await session.list_tools()
                names = [t.name for t in tools.tools]
                assert {"reset", "observe", "act", "score"} <= set(names)

                r = await session.call_tool("reset", {
                    "seed": 42, "scenario": "rebuild", "club": 15,
                    "world": "compact", "mode": "manager", "days": 60,
                })
                obs = json.loads(r.content[0].text)
                assert obs["is_matchday"] is True
                assert obs["budget"] == 50_000_000  # rebuild scenario

                r = await session.call_tool("observe", {})
                assert json.loads(r.content[0].text)["step"] == 0

                # A coach act: set the best-XI + attacking, then Continue.
                r = await session.call_tool("act", {
                    "action": {"action": "Continue", "params": None}
                })
                obs2 = json.loads(r.content[0].text)
                assert obs2["step"] == 1

                r = await session.call_tool("score", {})
                m = json.loads(r.content[0].text)
                assert "points" in m and "net_value" in m and "squad_size" in m

    asyncio.run(drive())
