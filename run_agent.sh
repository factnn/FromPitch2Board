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
CLUB=""
SEED=42
WORLD=compact
DAYS=400
DAYS_SET=false
SEASONS=1
PORT=0
PORT_SET=false
BUDGET=""
SKIP_AGENT=false
MAX_AGENT_TURNS=""   # unset → derived from --days (single-knob design)
ANON=true
# Resolved once so the launch line and metadata.json agree on pi's model.
# Exported: the metadata python heredoc only sees os.environ.
PI_MODEL="${PI_MODEL:-deepseek-v4-pro}"
export PI_MODEL

# ---- parse args ----
while [[ $# -gt 0 ]]; do
    case $1 in
        --agent) AGENT="$2"; shift 2 ;;
        --mode) MODE="$2"; shift 2 ;;
        --scenario) SCENARIO="$2"; shift 2 ;;
        --club) CLUB="$2"; shift 2 ;;
        --seed) SEED="$2"; shift 2 ;;
        --world) WORLD="$2"; shift 2 ;;
        --days) DAYS="$2"; DAYS_SET=true; shift 2 ;;
        --seasons) SEASONS="$2"; shift 2 ;;
        --port) PORT="$2"; PORT_SET=true; shift 2 ;;
        --budget-usd) BUDGET="$2"; shift 2 ;;
        --max-turns) MAX_AGENT_TURNS="$2"; shift 2 ;;
        --named) ANON=false; shift ;;
        --skip-agent) SKIP_AGENT=true; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ---- derived execution budgets (single knob: --days / --seasons) ----
# Multi-season: --seasons N spans N consecutive seasons (world rolls over at
# each boundary); --days is the absolute safety cap and defaults to N × 400.
if [[ "$SEASONS" -gt 1 && "$DAYS_SET" == "false" ]]; then
    DAYS=$((SEASONS * 400))
fi
# One 400-day season ≈ 43 llm steps, ≤2 agent turns (cc/pi finish in 1-2),
# ~90 min worst-case wall time. Long-horizon runs (3Y/5Y/10Y) scale these up
# automatically instead of needing per-task hardcodes.
if [[ -z "$MAX_AGENT_TURNS" ]]; then
    MAX_AGENT_TURNS=$(( (DAYS + 399) / 400 * 2 + 1 ))
fi
# llm_agent steps out its episode; ~45 steps/season + fixed headroom.
MAX_STEPS="${MAX_STEPS:-$(( DAYS / 400 * 45 + 60 ))}"
export MAX_STEPS

# pick a free port unless one was requested
if [[ "$PORT_SET" == "false" ]]; then
    PORT=$(python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()")
fi
MCP_URL="http://127.0.0.1:${PORT}/mcp"

# ---- prompt (mode) + scenario goal ----
# Archetype fusion: each scenario maps to a fixed club-strength rank unless
# --club was given explicitly (crisis=5, moneyball=40, rebuild=75, title=110).
if [[ -z "$CLUB" ]]; then
    case $SCENARIO in
        crisis) CLUB=5 ;;
        moneyball) CLUB=40 ;;
        title) CLUB=110 ;;
        *) CLUB=75 ;;
    esac
fi
PROMPT_TEMPLATE="agents/prompts/${MODE}.md"
GOAL_FILE="agents/prompts/goals/${SCENARIO}.md"
[[ -f "$PROMPT_TEMPLATE" ]] || { echo "no prompt template for mode=$MODE"; exit 1; }
[[ -f "$GOAL_FILE" ]] || { echo "no goal file for scenario=$SCENARIO"; exit 1; }

# ---- workspace (sandbox) ----
START_TS=$(date +%s)
TS=$(date +%Y%m%d_%H%M%S)
# Absolute so it survives the agent's `cd "$RUN_DIR"` below. TS is
# second-resolution; two runs of the same cell in the same second must not
# share a run dir (that collided once and killed both) — add a random suffix.
RUN_DIR="${SCRIPT_DIR}/runs/${SCENARIO}-club${CLUB}-seed${SEED}-${AGENT}-${MODE}-${TS}-$RANDOM"
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
# Retry a few times: under heavy parallel launch the MCP handshake can race
# and return an empty response, which used to kill the whole run early
# (money wasted on a dead job).
echo "[setup] resetting episode..."
for _ in 1 2 3; do
    python3 agents/mcp_call.py --url "${MCP_URL}" reset \
      "{\"seed\": ${SEED}, \"scenario\": \"${SCENARIO}\", \"club\": ${CLUB}, \"world\": \"${WORLD}\", \"mode\": \"${MODE}\", \"days\": ${DAYS}, \"seasons\": ${SEASONS}, \"anonymize\": ${ANON}, \"cp_dir\": \"$RUN_DIR/checkpoint\"}" \
      2>"$RUN_DIR/reset_err.txt" > "$RUN_DIR/initial_observation.json" \
      || echo "[setup] reset call failed rc=$? (attempt $_): $(head -c 120 "$RUN_DIR/initial_observation.json")"
    [[ -s "$RUN_DIR/initial_observation.json" ]] && break
    echo "[setup] reset returned empty — retrying"
    sleep 2
done

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

# ---- isolated agent workspace (path resolved BEFORE the pi prompt appendix,
#      which embeds it) ----
WS_DIR="/tmp/clubbench-ws/$(basename "$RUN_DIR")"

# Pi has no MCP integration out of the box; it reaches the env through its
# built-in bash tool calling mcp_call.py (pi's minimal-tools philosophy).
# NOTE: paths are workspace-relative — the agent runs in an isolated workspace
# and must not see any repository path (cross-run/source leakage prevention).
if [[ "$AGENT" == "pi" ]]; then
    cat >> "$RUN_DIR/prompt.md" <<PROMPT

## HOW TO ACCESS THE ENVIRONMENT (Pi)

You have no MCP tools in this harness. Interact with the environment through
your built-in bash tool, calling the helper script in your workspace:

IMPORTANT: You are in an isolated sandbox. The game lives behind the helper
script below and is reachable ONLY through it. The filesystem around you
contains nothing about the game — do not waste time searching it (no repo, no
source code, no other players' data). Observe, decide, act through the script.

  python3 ${WS_DIR}/mcp_call.py --url ${MCP_URL} observe
  python3 ${WS_DIR}/mcp_call.py --url ${MCP_URL} act '<action-json>'
  python3 ${WS_DIR}/mcp_call.py --url ${MCP_URL} score

- observe prints the full observation JSON (squad, market, offers, date, step,
  done, last_action_result, ...). Start every decision by observing.
- act takes ONE action, e.g.:
    python3 ${WS_DIR}/mcp_call.py --url ${MCP_URL} act '{"action": {"action": "Continue", "params": null}}'
    python3 ${WS_DIR}/mcp_call.py --url ${MCP_URL} act '{"action": {"action": "SetMatchPlan", "params": {"player_ids": ["..."], "play_style": "Attacking"}}}'
- Every act advances the game to the next decision point and prints the new
  observation (read last_action_result to learn what your previous action did).
- Keep observing and acting until an observation shows "done": true, then call
  score once. Never stop early.
PROMPT
fi

# The agent must only see ITS OWN workspace + the env interface (MCP /
# mcp_call.py). Everything else — the repository, other runs' trajectories, the
# env source — is hidden by running the agent as the unprivileged
# `clubbench-agent` user while the repo is root-only (chmod 700). llm has no
# shell access (pure API harness) so it keeps running as root; its API key is
# injected through env as before.
mkdir -p "$WS_DIR"
cp -f "$RUN_DIR/prompt.md" "$WS_DIR/prompt.md"
if [[ "$AGENT" == "pi" ]]; then
    cp -f agents/mcp_call.py "$WS_DIR/mcp_call.py"
elif [[ "$AGENT" == "llm" ]]; then
    # llm runs in the WS too; it writes trajectory.jsonl/usage.json next to
    # its prompt and reads the harness script from here.
    cp agents/llm_agent.py "$WS_DIR/llm_agent.py"
elif [[ "$AGENT" == "cc" ]]; then
    cp "$RUN_DIR/.mcp.json" "$WS_DIR/.mcp.json"
elif [[ "$AGENT" == "codex" ]]; then
    # Per-run CODEX_HOME inside the WS: DeepSeek provider config (official
    # setup) + model catalog. The API key is substituted at run time from the
    # environment and lives only in this transient workspace file.
    mkdir -p "$WS_DIR/.codex"
    sed -e "s|{{DEEPSEEK_API_KEY}}|${DEEPSEEK_API_KEY:-}|g" \
        -e "s|{{CODEX_MODEL}}|${CODEX_MODEL:-deepseek-v4-pro}|g" \
        agents/codex/config.toml > "$WS_DIR/.codex/config.toml"
    cp agents/codex/models.json "$WS_DIR/.codex/models.json"
    cp "$RUN_DIR/.mcp.json" "$WS_DIR/.mcp.json"
fi
chown -R clubbench-agent:clubbench-agent "$WS_DIR" 2>/dev/null || true

# ---- resolve the agent binary ----
# Find the claude CLI: $CLAUDE_BIN override > PATH > the shared claude_tool
# conda env's node + @anthropic-ai/claude-code/cli.js > the legacy npm-global
# location (npm churn has removed that one before). CLAUDE_CMD is an array so
# "node /path/cli.js" works as well as a plain binary.
CLAUDE_BIN="${CLAUDE_BIN:-}"
CC_CLI_JS=[local-tool]
CC_NODE=[local-tool]
if [[ -n "$CLAUDE_BIN" ]]; then
    CLAUDE_CMD=("$CLAUDE_BIN")
elif [[ -f "$CC_CLI_JS" && -x "$CC_NODE" ]]; then
    # Preferred BEFORE `command -v claude`: cc runs unprivileged as
    # clubbench-agent, whose PATH may lack a root-only `claude` binary; the
    # claude_tool env is world-readable so node+cli.js works for the agent too.
    CLAUDE_CMD=("$CC_NODE" "$CC_CLI_JS")
elif command -v claude >/dev/null 2>&1; then
    CLAUDE_CMD=(claude)
else
    CLAUDE_CMD=(/root/.npm-global/bin/claude)
fi

# ---- launch the agent (skipped with --skip-agent for CI/tests) ----
if [[ "$SKIP_AGENT" == "true" ]]; then
    echo "[agent] skipped (--skip-agent)"
else
echo "[agent] launching $AGENT..."
AGENT_CMD=()
if [[ "$AGENT" == "cc" ]]; then
    AGENT_CMD=("${CLAUDE_CMD[@]}" -p "$(cat "$RUN_DIR/prompt.md")" --dangerously-skip-permissions --output-format stream-json --verbose)
    if [[ -n "$BUDGET" ]]; then AGENT_CMD+=(--max-budget-usd "$BUDGET"); fi
elif [[ "$AGENT" == "codex" ]]; then
    # Codex + DeepSeek via the official responses-wire config in CODEX_HOME.
    # --json = stream-json equivalent; workspace-write sandbox on top of our
    # own isolation; --skip-git-repo-check (WS is not a git repo).
    CODEX_MODEL="${CODEX_MODEL:-deepseek-v4-pro}"
    export CODEX_MODEL CODEX_HOME="$WS_DIR/.codex"
    AGENT_CMD=(codex exec --json --skip-git-repo-check --sandbox workspace-write \
        -C "$WS_DIR" -m "$CODEX_MODEL" "$(cat "$RUN_DIR/prompt.md")")
elif [[ "$AGENT" == "llm" ]]; then
    # Group A: standardized harness — direct API call, model-swappable.
    # Runs inside the isolated WS (script copied there) like every agent.
    AGENT_CMD=(python3 "$WS_DIR/llm_agent.py" --url "${MCP_URL}" --prompt "$WS_DIR/prompt.md")
elif [[ "$AGENT" == "pi" ]]; then
    # Group B: Pi harness. Native DeepSeek provider (env DEEPSEEK_API_KEY).
    # The env is reached through pi's built-in bash tool calling mcp_call.py —
    # pi's minimal-tools philosophy: read/write/edit/bash + extensions.
    # /opt/clubbench-tools/pi is the world-readable copy — the agent runs as
    # clubbench-agent and cannot traverse /root (where npm-global lives).
    PI_BIN="${PI_BIN:-/opt/clubbench-tools/pi}"
    # pi's native default is minimal (= no thinking); the protocol aligns all
    # harnesses at max reasoning, so default to xhigh (pi maps xhigh→"max").
    # Override with PI_THINKING=off|minimal|low|medium|high|xhigh.
    AGENT_CMD=("$PI_BIN" -p "$(cat "$RUN_DIR/prompt.md")" --provider deepseek \
        --model "$PI_MODEL" --mode json \
        --thinking "${PI_THINKING:-xhigh}" \
        --session-dir "$WS_DIR/.pi-sessions")
else
    echo "unknown agent: $AGENT"; exit 1
fi

# IS_SANDBOX=1 tells Claude Code it is running in a sandbox, which permits
# --dangerously-skip-permissions even under root (the flagbench pattern).
# `claude -p` produces one autonomous response; if the season isn't over it may
# stop early, so loop a "continue" prompt until the episode reports done.
# ALL agents run unprivileged (clubbench-agent) inside their isolated
# workspace — same sandbox surface for every harness, no exceptions.
AGENT_PATH="/opt/clubbench-tools:[local-tool]"
launch_agent() {
    (export HOME="$WS_DIR" PATH="$AGENT_PATH" IS_SANDBOX=1
     setpriv --reuid clubbench-agent --regid clubbench-agent --init-groups \
         bash -c 'cd "$0" && exec "$@"' "$WS_DIR" "${AGENT_CMD[@]}")
}
CONTINUE_PROMPT="The season is not over yet. Continue managing the club: observe, act, and keep playing until the observation reports \"done\": true. Do not summarise or stop early."
for turn in $(seq 1 $MAX_AGENT_TURNS); do
    echo "[agent] turn $turn: launching $AGENT..."
    # < /dev/null: codex exec appends piped stdin to its prompt and BLOCKS
    # waiting for EOF (background jobs have an open pipe); nobody needs stdin.
    launch_agent 2>> "$RUN_DIR/agent.log" < /dev/null \
      | python3 "$SCRIPT_DIR/agents/stream_view.py" "$RUN_DIR/agent_output.jsonl" || true

    # Check whether the episode is done.
    if python3 agents/mcp_call.py --url "${MCP_URL}" observe 2>/dev/null | grep -q '"done": true'; then
        echo "[agent] season complete after turn $turn"
        break
    fi
    if [[ "$turn" == "$MAX_AGENT_TURNS" ]]; then
        echo "[agent] hit max turns without finishing the season"
        break
    fi
    if [[ "$AGENT" == "llm" ]]; then
        # The standardized harness loops internally; just re-run it to continue.
        AGENT_CMD=(python3 "$WS_DIR/llm_agent.py" --url "${MCP_URL}" --prompt "$WS_DIR/prompt.md")
    elif [[ "$AGENT" == "pi" ]]; then
        # Resume pi's session (same --session-dir) so memory carries across turns.
        AGENT_CMD=("$PI_BIN" --continue -p "$CONTINUE_PROMPT" --provider deepseek \
            --model "$PI_MODEL" --mode json \
            --thinking "${PI_THINKING:-xhigh}" \
            --session-dir "$WS_DIR/.pi-sessions")
    elif [[ "$AGENT" == "codex" ]]; then
        AGENT_CMD=(codex exec resume --last --json --skip-git-repo-check \
            --sandbox workspace-write -C "$WS_DIR" -m "$CODEX_MODEL" "$CONTINUE_PROMPT")
    else
        # cc must play the WHOLE episode in ONE session, same as pi (--continue)
        # and codex (resume --last): resume the session id captured from the
        # previous turn's stream-json result event. Without this, each turn was
        # a fresh claude -p with no memory — not comparable with the other
        # off-the-shelf harnesses (B table).
        CC_SID=$(python3 - "$RUN_DIR/agent_output.jsonl" <<'PY'
import json, sys
sid = None
for line in open(sys.argv[1]):
    if not line.strip().startswith("{"):
        continue
    try:
        e = json.loads(line)
    except ValueError:
        continue
    if e.get("type") == "result" and e.get("session_id"):
        sid = e["session_id"]
print(sid or "")
PY
)
        AGENT_CMD=("${CLAUDE_CMD[@]}" -p "$CONTINUE_PROMPT" --dangerously-skip-permissions --output-format stream-json --verbose)
        # CC_NO_RESUME=1 reverts to the pre-fix no-memory protocol — used as a
        # controlled ablation arm (is the session-memory effect real, or just
        # provider/date drift after the 8-17 DS price hike?).
        if [[ -n "$CC_SID" && -z "$CC_NO_RESUME" ]]; then AGENT_CMD+=(--resume "$CC_SID"); fi
        if [[ -n "$BUDGET" ]]; then AGENT_CMD+=(--max-budget-usd "$BUDGET"); fi
    fi
done
fi  # end SKIP_AGENT

# ---- archive the agent's isolated workspace for analysis ----
# (pi sessions, files the agent wrote; the host side reads it back as root)
if [[ -d "$WS_DIR" ]]; then
    mkdir -p "$RUN_DIR/ws"
    cp -a "$WS_DIR/." "$RUN_DIR/ws/" 2>/dev/null || true
    # llm_agent writes its step trace + usage next to its prompt (in the WS);
    # the analysis scripts expect them at the run-dir root level.
    if [[ "$AGENT" == "llm" ]]; then
        cp -a "$WS_DIR/trajectory.jsonl" "$RUN_DIR/trajectory.jsonl" 2>/dev/null || true
        cp -a "$WS_DIR/usage.json" "$RUN_DIR/usage.json" 2>/dev/null || true
    fi
fi

# ---- collect the final score + per-season snapshots ----
echo "[score] collecting result..."
python3 agents/mcp_call.py --url "${MCP_URL}" score 2>/dev/null > "$RUN_DIR/score.json" || echo "score failed" > "$RUN_DIR/score.json"
python3 agents/mcp_call.py --url "${MCP_URL}" snapshots 2>/dev/null > "$RUN_DIR/snapshots.json" || true
# Persist the trajectory (final game state + metadata) so the run can be
# re-scored later without re-running the agent (score-trajectory decoupling).
# The MCP dump also writes the step-level tool trace to trajectory.jsonl.
python3 agents/mcp_call.py --url "${MCP_URL}" dump "{\"path\": \"$RUN_DIR/trajectory.json\", \"agent\": \"$AGENT\"}" 2>/dev/null || echo "dump failed" > "$RUN_DIR/trajectory.json"

# ---- run metadata (identity + env version + completion) ----
python3 - "$RUN_DIR" "$AGENT" "$MODE" "$SCENARIO" "$CLUB" "$SEED" "$WORLD" "$DAYS" "$SEASONS" "$ANON" "$START_TS" <<'PY' 2>/dev/null || true
import json, os, subprocess, sys, time
out_dir, agent, mode, scenario, club, seed, world, days, seasons, anon, start_ts = sys.argv[1:12]
def git_rev(path):
    try:
        return subprocess.run(["git", "-C", path, "rev-parse", "--short", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
    except Exception:
        return "?"
score_ok = os.path.exists(os.path.join(out_dir, "score.json"))
usage_ok = os.path.exists(os.path.join(out_dir, "usage.json"))
usage_completed = False
if usage_ok:
    try:
        usage_completed = bool(json.load(open(os.path.join(out_dir, "usage.json"))).get("completed"))
    except Exception:
        pass
if not score_ok:
    reason = "no_score"
elif usage_ok and not usage_completed:
    reason = "incomplete"  # hit max turns / stopped early
else:
    reason = "normal"

meta = {
    "run_id": os.path.basename(out_dir),
    "agent": agent,
    # Per-agent model env: pi → PI_MODEL, codex → CODEX_MODEL, the rest →
    # ANTHROPIC_MODEL. Kept per-agent so an exported var in some tmux can
    # never leak into another agent's metadata.
    "model": (os.environ.get("PI_MODEL") if agent == "pi"
              else os.environ.get("CODEX_MODEL") if agent == "codex"
              else os.environ.get("ANTHROPIC_MODEL") or ""),
    "seed": int(seed), "scenario": scenario, "club": int(club),
    "world": world, "track": mode, "horizon_days": int(days), "seasons": int(seasons),
    "anonymized": anon == "true",
    "env_commit_clubbench": git_rev("[repo]"),
    "started_ts": start_ts,
    "wall_time_s": round(time.time() - float(start_ts), 2),
    "completed": score_ok,
    "termination_reason": reason,
}
# llm_agent writes usage.json itself; for cc/codex aggregate from the raw
# stream-json output. `llm_calls` = number of assistant (model-turn) events —
# the same "model turns" semantics as the llm harness's call counter.
usage_path = os.path.join(out_dir, "usage.json")
if not os.path.exists(usage_path):
    agg = {"input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0, "llm_calls": 0}
    if agent == "pi":
        # pi's stream events carry no usage; its session journal does
        # (OpenAI-style usage per assistant message).
        import glob as _glob
        for sess in _glob.glob(os.path.join(out_dir, "ws", ".pi-sessions", "*.jsonl")):
            try:
                for line in open(sess):
                    line = line.strip()
                    if not line:
                        continue
                    e = json.loads(line)
                    if e.get("type") != "message":
                        continue
                    u = (e.get("message") or {}).get("usage") or {}
                    if u:
                        agg["input_tokens"] += u.get("input", 0)
                        agg["output_tokens"] += u.get("output", 0)
                        agg["cache_read_input_tokens"] += u.get("cacheRead", 0)
                        agg["llm_calls"] += 1
            except Exception:
                pass
    elif agent == "codex":
        # codex --json emits exactly ONE top-level usage object per run
        # (session totals): cached_input_tokens = cache reads, and the
        # OpenAI responses API reports reasoning_output_tokens separately.
        # llm_calls ≈ number of completed turns.
        try:
            u = None
            turns = 0
            for line in open(os.path.join(out_dir, "agent_output.jsonl")):
                line = line.strip()
                if not line:
                    continue
                e = json.loads(line)
                if e.get("type") == "turn.completed":
                    turns += 1
                if u is None and isinstance(e.get("usage"), dict):
                    u = e["usage"]
            agg = {
                "input_tokens": (u or {}).get("input_tokens", 0),
                "output_tokens": (u or {}).get("output_tokens", 0),
                "cache_read_input_tokens": (u or {}).get("cached_input_tokens", 0),
                "reasoning_output_tokens": (u or {}).get("reasoning_output_tokens", 0),
                "llm_calls": turns,
                "completed": True,
            }
        except Exception:
            pass
    else:
        try:
            for line in open(os.path.join(out_dir, "agent_output.jsonl")):
                e = json.loads(line)
                if e.get("type") == "assistant":
                    agg["llm_calls"] += 1
                u = e.get("usage")
                if isinstance(u, dict):
                    agg["input_tokens"] += u.get("input_tokens", 0)
                    agg["output_tokens"] += u.get("output_tokens", 0)
                    agg["cache_read_input_tokens"] += u.get("cache_read_input_tokens", 0)
        except Exception:
            pass
    json.dump(agg, open(usage_path, "w"), indent=2)
json.dump(meta, open(os.path.join(out_dir, "metadata.json"), "w"), indent=2)
PY

# ---- token + cost report ----
echo ""
echo "[tokens]"
python3 "$SCRIPT_DIR/agents/token_report.py" "$RUN_DIR/agent_output.jsonl" 2>/dev/null || true

# ---- derived agent-behavior / reliability stats (also runnable standalone) ----
echo ""
echo "[behavior]"
python3 "$SCRIPT_DIR/agents/analyze_trajectory.py" "$RUN_DIR" 2>/dev/null || true

echo ""
echo "Done. Results in: $RUN_DIR/"
echo "  metadata:    $RUN_DIR/metadata.json   (identity + env version + completion)"
echo "  trajectory:  $RUN_DIR/trajectory.jsonl  (step-level decision trace)"
echo "  usage:       $RUN_DIR/usage.json      (input/output/cache tokens, cost)"
echo "  score:       $RUN_DIR/score.json      (football outcome)"
echo "  snapshots:   $RUN_DIR/snapshots.json  (per-season)"
