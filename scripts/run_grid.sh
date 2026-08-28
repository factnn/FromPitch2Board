#!/bin/bash
# run_grid.sh — run the ClubBench agent grid with a concurrency cap.
#
# Each job is one full episode (run_agent.sh), which picks its own free MCP
# port, so N jobs run in parallel. Results land in runs/<...>/score.json.
#
# Usage:
#   scripts/run_grid.sh --agents llm,cc --modes coach,manager \
#       --clubs auto --seeds 42,43 --scenarios crisis,rebuild \
#       --world medium --days 400 --parallel 8
#
# --clubs auto = each scenario's archetype club (crisis=5 moneyball=40
# rebuild=75 title=110), derived by run_agent.sh.
# Jobs whose previous run left a score.json are skipped (safe to re-run a
# killed grid without paying twice).
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SCRIPT_DIR"

AGENTS="llm"
MODES="manager"
CLUBS="auto"
SEEDS="42"
SCENARIOS="rebuild"
WORLD="medium"
DAYS=""
SEASONS="1"
PARALLEL=8
TAG=""
MATCH_STOPS=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --agents) AGENTS="$2"; shift 2 ;;
        --modes) MODES="$2"; shift 2 ;;
        --clubs) CLUBS="$2"; shift 2 ;;
        --seeds) SEEDS="$2"; shift 2 ;;
        --scenarios) SCENARIOS="$2"; shift 2 ;;
        --world) WORLD="$2"; shift 2 ;;
        --days) DAYS="$2"; shift 2 ;;
        --seasons) SEASONS="$2"; shift 2 ;;
        --parallel) PARALLEL="$2"; shift 2 ;;
        --tag) TAG="$2"; shift 2 ;;
        --match-stops) MATCH_STOPS="--match-stops"; shift ;;
        *) echo "unknown: $1"; exit 1 ;;
    esac
done

# DAYS unset → derive from SEASONS (single-knob design). Passing --days 200
# by default silently truncated every multi-season cell at 200 game-days —
# the E/C grids all died at 2027-01-17 once because of that.
if [[ -z "$DAYS" ]]; then
    if [[ "$SEASONS" -gt 1 ]]; then
        DAYS=$((SEASONS * 400))
    else
        DAYS=400
    fi
fi

# Enumerate all jobs.
JOBS=()
for a in ${AGENTS//,/ }; do
    for m in ${MODES//,/ }; do
        for s in ${SEEDS//,/ }; do
            for sc in ${SCENARIOS//,/ }; do
                if [[ "$CLUBS" == "auto" ]]; then
                    JOBS+=("$a $m auto $s $sc $WORLD $DAYS")
                else
                    for c in ${CLUBS//,/ }; do
                        JOBS+=("$a $m $c $s $sc $WORLD $DAYS")
                    done
                fi
            done
        done
    done
done

echo "grid: ${#JOBS[@]} jobs, parallel=$PARALLEL, world=$WORLD days=$DAYS clubs=$CLUBS"

# One worker per job (xargs replaces {} with the job's fields).
run_one_job() {
    set -- $1
    A=$1; M=$2; C=$3; S=$4; SC=$5; W=$6; D=$7
    # --tag keeps grids of the same cells but different models/configs apart
    # (e.g. pro vs flash); the skip check reads this same log file.
    OUT="runs/${SC}-seed${S}-${A}-${M}-grid${TAG:+-${TAG}}.ts"
    PREV=$(grep -oE "[repo]/runs/[^ ]+" "$OUT.log" 2>/dev/null | head -1)
    if [[ -n "$PREV" && -f "$PREV/score.json" ]]; then
        # Skip only if the previous run REALLY finished (score.json alone is
        # written even when the agent hit max-turns mid-season). The check
        # lives in a standalone script — heredocs inside exported functions
        # break under xargs, which silently skipped/redid the wrong cells.
        if [[ "$(python3 scripts/check_complete.py "$PREV" "$D" 2>/dev/null)" == "ok" ]]; then
            echo "[$(date +%H:%M:%S)] $A/$M club=$C seed=$S $SC skip (already done)"
            return 0
        fi
        echo "[$(date +%H:%M:%S)] $A/$M club=$C seed=$S $SC redo (previous run truncated)"
    fi
    echo "[$(date +%H:%M:%S)] $A/$M club=$C seed=$S $SC start"
    # Per-job wall-clock bound: derived from --days (1h/season + 30 min), so
    # an agent stuck in an infinite bash/exploration loop must not burn tokens
    # for a day (pi did 22h once) and 10Y runs are never killed early.
    # --max-turns is intentionally NOT passed: run_agent.sh derives it from
    # --days too (single-knob design).
    JOB_TIMEOUT="${JOB_TIMEOUT:-$(( (D + 399) / 400 * 3600 + 1800 ))}"
    if [[ "$C" == "auto" ]]; then
        timeout --kill-after=60 "$JOB_TIMEOUT" ./run_agent.sh --agent "$A" --mode "$M" --scenario "$SC" \
            --seed "$S" --world "$W" --days "$D" --seasons "$SEASONS" $MATCH_STOPS \
            > "$OUT.log" 2>&1
    else
        timeout --kill-after=60 "$JOB_TIMEOUT" ./run_agent.sh --agent "$A" --mode "$M" --scenario "$SC" --club "$C" \
            --seed "$S" --world "$W" --days "$D" --seasons "$SEASONS" $MATCH_STOPS \
            > "$OUT.log" 2>&1
    fi
    D2=$(grep -oE "[repo]/runs/[^ ]+" "$OUT.log" | head -1)
    if [[ -n "$D2" && -f "$D2/score.json" ]]; then
        python3 scripts/summarize_score.py "$D2/score.json" "$A" "$M" "$C" "$S" "$SC"
    else
        echo "[$A/$M club=$C seed=$S $SC] NO SCORE (see $OUT.log)"
    fi
    echo "[$(date +%H:%M:%S)] $A/$M club=$C seed=$S $SC done"
}
# TAG/JOB_TIMEOUT/SEASONS must be exported alongside the function: xargs
# spawns `bash -c`, which only inherits exported variables. Missing TAG made
# the tag silently vanish (skip logic read the previous grid's log and
# skipped all jobs); missing SEASONS made the reset JSON malformed
# (`"seasons": ,`) and every job died instantly.
export TAG JOB_TIMEOUT SEASONS MATCH_STOPS
export -f run_one_job

printf '%s\n' "${JOBS[@]}" | xargs -P "$PARALLEL" -I{} bash -c 'run_one_job "$1"' _ {}

echo "=== grid complete ==="
