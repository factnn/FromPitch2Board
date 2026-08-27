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

try:
    from exceptiongroup import BaseExceptionGroup  # Python < 3.11
except ImportError:
    pass


def _is_session_noise(exc: BaseException) -> bool:
    """True if exc is (possibly nested) streamable-http session-close noise.

    The client raises "Session termination failed: 202" inside an
    ExceptionGroup when tearing down an async HTTP session after the tool call
    already succeeded. The group's own message is the generic "unhandled
    errors in a TaskGroup (...)", so we must recurse into .exceptions. Only
    suppress when *every* sub-exception is noise; a real error must surface.
    """
    if isinstance(exc, BaseExceptionGroup):
        return all(_is_session_noise(e) for e in exc.exceptions)
    msg = str(exc)
    return "Session termination failed" in msg or "202" in msg


async def call(url: str, tool: str, args: dict) -> int:
    try:
        async with streamable_http_client(url) as conn:
            read, write = conn[0], conn[1]
            async with ClientSession(read, write) as session:
                await session.initialize()
                r = await session.call_tool(tool, args)
                if getattr(r, "isError", False):
                    print(f"ERROR: {r.content}", file=sys.stderr, flush=True)
                    return 1
                for c in r.content:
                    print(c.text, flush=True)
    except BaseException as e:
        # Session-close noise must not fail the script; any other error is
        # re-raised so the traceback points at the real cause.
        if _is_session_noise(e):
            return 0
        raise
    return 0


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
    sys.exit(asyncio.run(call(url, tool, params)))


if __name__ == "__main__":
    main()
