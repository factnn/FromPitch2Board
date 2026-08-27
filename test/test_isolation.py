"""Isolation sandbox: agents must only see their own workspace + the env
interface (MCP / mcp_call.py), never other runs' trajectories or the env
source.

Mechanism: the repo is chmod 700 (root-only) and cc/codex/pi agents run as the
unprivileged `clubbench-agent` user via setpriv, with CWD = a private workspace
under /tmp/clubbench-ws/. llm stays root — it is a pure API loop with no shell.
"""

import json
import os
import pwd
import re
import subprocess

import pytest

REPO = "[repo]"
AGENT_USER = "clubbench-agent"


def run_as_agent(cmd: str):
    """Run a shell command as the unprivileged benchmark agent user."""
    return subprocess.run(
        ["setpriv", "--reuid", AGENT_USER, "--regid", AGENT_USER,
         "--init-groups", "bash", "-c", cmd],
        capture_output=True, text=True, timeout=60)


def test_agent_user_exists():
    pwd.getpwnam(AGENT_USER)


def test_repo_is_root_only():
    mode = oct(os.stat(REPO).st_mode & 0o777)
    assert mode == "0o700", f"repo must be root-only, got {mode}"


def test_agent_cannot_read_other_runs():
    r = run_as_agent(f"ls {REPO}/runs")
    assert r.returncode != 0
    assert "Permission denied" in r.stderr


def test_agent_cannot_read_env_source():
    r = run_as_agent(f"head -c 10 {REPO}/src/score.rs")
    assert r.returncode != 0
    assert "Permission denied" in r.stderr


def test_agent_cannot_read_trajectories():
    r = run_as_agent(f"cat {REPO}/runs/*/score.json 2>/dev/null | wc -c")
    assert r.returncode != 0 or r.stdout.strip() == "0"


def test_pi_binary_available_to_agent():
    r = run_as_agent(
        "node /opt/clubbench-tools/pi-coding-agent/dist/cli.js --version")
    assert r.returncode == 0, r.stderr


def test_run_agent_builds_isolated_workspace():
    """--skip-agent still builds the workspace; check structure + ownership.

    The workspace must contain only the interface files (prompt.md +
    mcp_call.py for pi) and be owned by the agent user; the repo must not be
    reachable from it.
    """
    log = subprocess.run(
        ["./run_agent.sh", "--agent", "pi", "--mode", "coach",
         "--scenario", "crisis", "--seed", "42", "--world", "compact",
         "--days", "15", "--skip-agent"],
        capture_output=True, text=True, timeout=300)
    assert log.returncode == 0, log.stderr[-500:]
    m = re.search(r"workspace: (\S+)", log.stdout)
    assert m, "no workspace line in output"
    run_dir = m.group(1)
    ws = os.path.join("/tmp/clubbench-ws", os.path.basename(run_dir))
    assert os.path.isdir(ws), f"workspace not created: {ws}"
    st = os.stat(ws)
    assert st.st_uid == pwd.getpwnam(AGENT_USER).pw_uid, "workspace not agent-owned"

    # Only the interface files, nothing from the repo.
    names = set(os.listdir(ws))
    assert {"prompt.md", "mcp_call.py"} <= names, names

    # The pi prompt appendix must not leak any repository path.
    prompt = open(os.path.join(ws, "prompt.md")).read()
    assert "/repo" not in prompt, "prompt leaks a repo path"

    # metadata records pi's resolved model (PI_MODEL default), not None.
    meta = json.load(open(os.path.join(run_dir, "metadata.json")))
    assert meta["model"] == "deepseek-v4-pro", meta["model"]
