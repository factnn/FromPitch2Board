"""run_agent.sh orchestration (without spending agent quota).

--skip-agent runs the full pipeline except the LLM agent: sandbox workspace,
prompt filling, .mcp.json, MCP server, episode reset, and final score. This
verifies the one-command harness works end-to-end.
"""

import json
import subprocess
from pathlib import Path

from conftest import REPO


def _run_agent_skip(*args):
    cmd = [str(REPO / "run_agent.sh"), "--skip-agent", "--world", "compact", *args]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    return r


def _latest_run_dir(stdout):
    for line in stdout.splitlines():
        if "Results in:" in line:
            return Path(REPO / line.split("Results in:")[1].strip())
    raise AssertionError("no run dir found in output")


def test_skip_agent_pipeline():
    r = _run_agent_skip("--scenario", "rebuild", "--club", "15", "--seed", "42", "--days", "30")
    assert r.returncode == 0, r.stderr + r.stdout
    run_dir = _latest_run_dir(r.stdout)

    # Sandbox artifacts exist and are filled.
    prompt = (run_dir / "prompt.md").read_text()
    assert "rebuild" in prompt.lower() and "GOAL:" in prompt
    assert (run_dir / ".mcp.json").exists()

    # The episode was reset: an initial observation was captured.
    obs = json.loads((run_dir / "initial_observation.json").read_text())
    assert obs["budget"] == 50_000_000  # rebuild scenario budget
    assert len(obs["squad"]) > 0

    # The score was collected.
    score = json.loads((run_dir / "score.json").read_text())
    assert "points" in score and "net_value" in score and "squad_size" in score
