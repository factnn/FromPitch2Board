"""One-shot MCP tool call against the ClubBench MCP server.

Used by run_agent.sh to set up the episode (reset) and collect the final
score, so the agent launch stays a one-command flow.

Usage:
  python mcp_call.py --url http://127.0.0.1:PORT/mcp reset '{"seed": 42, ...}'
  python mcp_call.py --url http://127.0.0.1:PORT/mcp score
"""

import asyncio
import json
import sys

from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client


async def call(url: str, tool: str, args: dict):
    try:
        async with streamable_http_client(url) as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                r = await session.call_tool(tool, args)
                if r.is_error:
                    print(f"ERROR: {r.content}", file=sys.stderr)
                    sys.exit(1)
                for c in r.content:
                    print(c.text)
    except BaseException as e:
        # The streamable-http client raises on session close ("Session
        # termination failed: 202") even when the tool call succeeded; that is
        # normal for async HTTP sessions and should not fail the script.
        msg = str(e)
        if "Session termination failed" in msg or "202" in msg:
            return
        raise


def main():
    args = sys.argv[1:]
    url = "http://127.0.0.1:8890/mcp"
    if "--url" in args:
        i = args.index("--url")
        url = args[i + 1]
        args = args[:i] + args[i + 2:]
    if len(args) < 1:
        print("usage: mcp_call.py [--url URL] <tool> [json-args]", file=sys.stderr)
        sys.exit(2)
    tool = args[0]
    params = json.loads(args[1]) if len(args) > 1 else {}
    asyncio.run(call(url, tool, params))


if __name__ == "__main__":
    main()
