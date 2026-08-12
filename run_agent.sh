#!/bin/bash
# run_agent.sh — one-command ClubBench agent evaluation.
#
# Builds a sandboxed workspace, starts the ClubBench MCP server, sets up the
# episode (reset), launches a CLI agent (Claude Code / Codex) that plays it via
# MCP tools, then records the final score.
#
# Usage:
#   ./run_agent.sh --agent cc --mode manager --scenario rebuild --club 15 --seed 42 \
#                  --world compact --days 400
#
# Options:
#   --agent cc|codex        CLI agent to run (default cc)
#   --mode manager|coach    track (default manager) — selects the prompt + env behaviour
#   --scenario <name>       crisis|moneyball|rebuild|title (default rebuild)
#   --club <rank>           managed-club strength rank, 0 = weakest (default 15)
#   --seed <n>              reproducibility seed (default 42)
#   --world <w>             compact|medium|standard (default compact)
#   --days <n>              episode horizon in game days (default 400)
#   --port <n>              MCP server port (default 8890)
#   --budget-usd <n>        optional Claude Code max budget

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ---- defaults ----
AGENT=cc
MODE=manager
SCENARIO=rebuild
CLUB=15
SEED=42
WORLD=compact
DAYS=400
PORT=0
PORT_SET=false
BUDGET=""
SKIP_AGENT=false
MAX_AGENT_TURNS=5

# ---- parse args ----
while [[ $# -gt 0 ]]; do
    case $1 in
        --agent) AGENT="$2"; shift 2 ;;
        --mode) MODE="$2"; shift 2 ;;
        --scenario) SCENARIO="$2"; shift 2 ;;
        --club) CLUB="$2"; shift 2 ;;
        --seed) SEED="$2"; shift 2 ;;
        --world) WORLD="$2"; shift 2 ;;
        --days) DAYS="$2"; shift 2 ;;
        --port) PORT="$2"; PORT_SET=true; shift 2 ;;
        --budget-usd) BUDGET="$2"; shift 2 ;;
        --max-turns) MAX_AGENT_TURNS="$2"; shift 2 ;;
        --skip-agent) SKIP_AGENT=true; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# pick a free port unless one was requested
if [[ "$PORT_SET" == "false" ]]; then
    PORT=$(python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()")
fi
MCP_URL="http://127.0.0.1:${PORT}/mcp"

# ---- prompt (mode) + scenario goal ----
PROMPT_TEMPLATE="agents/prompts/${MODE}.md"
GOAL_FILE="agents/prompts/goals/${SCENARIO}.md"
[[ -f "$PROMPT_TEMPLATE" ]] || { echo "no prompt template for mode=$MODE"; exit 1; }
[[ -f "$GOAL_FILE" ]] || { echo "no goal file for scenario=$SCENARIO"; exit 1; }

# ---- workspace (sandbox) ----
TS=$(date +%Y%m%d_%H%M%S)
RUN_DIR="runs/${SCENARIO}-club${CLUB}-seed${SEED}-${AGENT}-${MODE}-${TS}"
mkdir -p "$RUN_DIR"

# ---- .mcp.json for Claude Code / Codex ----
cat > "$RUN_DIR/.mcp.json" <<JSON
{
  "mcpServers": {
    "clubbench": {
      "type": "http",
      "url": "http://127.0.0.1:${PORT}/mcp",
      "enabled": true
    }
  }
}
JSON

echo "=================================================="
echo "ClubBench agent run"
echo "  agent=$AGENT mode=$MODE scenario=$SCENARIO club=$CLUB seed=$SEED world=$WORLD days=$DAYS"
echo "  workspace: $RUN_DIR"
echo "=================================================="

# ---- start MCP server ----
./target/debug/clubbench-mcp --port "$PORT" > "$RUN_DIR/mcp.log" 2>&1 &
MCP_PID=$!
trap 'kill $MCP_PID 2>/dev/null' EXIT

# wait for the server to accept connections
for _ in $(seq 1 50); do
    if curl -s -o /dev/null --max-time 1 "${MCP_URL}"; then break; fi
    sleep 0.2
done
echo "[setup] MCP server on ${MCP_URL}"

# ---- set up the episode (reset) ----
echo "[setup] resetting episode..."
python3 agents/mcp_call.py --url "${MCP_URL}" reset \
  "{\"seed\": ${SEED}, \"scenario\": \"${SCENARIO}\", \"club\": ${CLUB}, \"world\": \"${WORLD}\", \"mode\": \"${MODE}\", \"days\": ${DAYS}}" \
  2>/dev/null > "$RUN_DIR/initial_observation.json"

# ---- build prompt with the REAL club name (from the reset observation) ----
CLUB_NAME=$(python3 -c "import json; print(json.load(open('$RUN_DIR/initial_observation.json')).get('team_name','${SCENARIO} club'))")
python3 - "$PROMPT_TEMPLATE" "$GOAL_FILE" "$SCENARIO" "$CLUB_NAME" "$RUN_DIR/prompt.md" <<'PY'
import sys
tmpl = open(sys.argv[1]).read()
goal = open(sys.argv[2]).read().strip()
prompt = (tmpl
    .replace("{{SCENARIO_GOAL}}", goal)
    .replace("{{SCENARIO_NAME}}", sys.argv[3])
    .replace("{{CLUB}}", sys.argv[4]))
open(sys.argv[5], "w").write(prompt)
PY
echo "[setup] prompt built for club: $CLUB_NAME"

# ---- resolve the agent binary ----
CLAUDE_BIN=$(command -v claude 2>/dev/null || echo /root/.npm-global/bin/claude)

# ---- launch the agent (skipped with --skip-agent for CI/tests) ----
if [[ "$SKIP_AGENT" == "true" ]]; then
    echo "[agent] skipped (--skip-agent)"
else
echo "[agent] launching $AGENT..."
AGENT_CMD=()
if [[ "$AGENT" == "cc" ]]; then
    AGENT_CMD=("$CLAUDE_BIN" -p "$(cat "$RUN_DIR/prompt.md")" --dangerously-skip-permissions --output-format stream-json --verbose)
    if [[ -n "$BUDGET" ]]; then AGENT_CMD+=(--max-budget-usd "$BUDGET"); fi
elif [[ "$AGENT" == "codex" ]]; then
    AGENT_CMD=(codex exec --sandbox read-only -C "$RUN_DIR" "$(cat "$RUN_DIR/prompt.md")")
else
    echo "unknown agent: $AGENT"; exit 1
fi

# IS_SANDBOX=1 tells Claude Code it is running in a sandbox, which permits
# --dangerously-skip-permissions even under root (the flagbench pattern).
# `claude -p` produces one autonomous response; if the season isn't over it may
# stop early, so loop a "continue" prompt until the episode reports done.
CONTINUE_PROMPT="The season is not over yet. Continue managing the club: observe, act, and keep playing until the observation reports \"done\": true. Do not summarise or stop early."
for turn in $(seq 1 $MAX_AGENT_TURNS); do
    echo "[agent] turn $turn: launching $AGENT..."
    (cd "$RUN_DIR" && IS_SANDBOX=1 "${AGENT_CMD[@]}") >> "$RUN_DIR/agent_output.jsonl" 2>> "$RUN_DIR/agent.log" || true

    # Check whether the episode is done.
    if python3 agents/mcp_call.py --url "${MCP_URL}" observe 2>/dev/null | grep -q '"done": true'; then
        echo "[agent] season complete after turn $turn"
        break
    fi
    if [[ "$turn" == "$MAX_AGENT_TURNS" ]]; then
        echo "[agent] hit max turns without finishing the season"
        break
    fi
    AGENT_CMD=("$CLAUDE_BIN" -p "$CONTINUE_PROMPT" --dangerously-skip-permissions --output-format stream-json --verbose)
    if [[ -n "$BUDGET" ]]; then AGENT_CMD+=(--max-budget-usd "$BUDGET"); fi
done
fi  # end SKIP_AGENT

# ---- collect the final score + per-season snapshots ----
echo "[score] collecting result..."
python3 agents/mcp_call.py --url "${MCP_URL}" score 2>/dev/null > "$RUN_DIR/score.json" || echo "score failed" > "$RUN_DIR/score.json"
python3 agents/mcp_call.py --url "${MCP_URL}" snapshots 2>/dev/null > "$RUN_DIR/snapshots.json" || true

echo ""
echo "Done. Results in: $RUN_DIR/"
echo "  prompt:      $RUN_DIR/prompt.md"
echo "  agent output: $RUN_DIR/agent_output.jsonl"
echo "  score:       $RUN_DIR/score.json"
