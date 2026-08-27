# ClubBench

**A Long-Horizon Benchmark for Autonomous Management Agents**

ClubBench evaluates autonomous agents — not isolated language models — inside a
headless football-club-management simulation. The central research question:

> **Can agent capabilities compose under long-horizon, multi-objective management?**

## Why football management

Professional football management is a natural lab for agent research because it is:

- **long-horizon** — a season spans hundreds of decisions across multiple domains;
- **partially observable** — hidden potential, noisy scout reports, uncertain markets;
- **multi-objective** — sporting results vs. finances vs. squad building vs. board goals;
- **objectively rewarded** — league points, trophies, wage bill, squad value are
  simulator facts. No LLM-as-judge.

The environment does **not** claim to faithfully reproduce professional football.
It instantiates the key computational challenges of long-horizon management:
delayed outcomes, partial observability, stochastic transitions, constrained
resources, hierarchical decisions, and multi-timescale planning.

## Design

A **nested task hierarchy on a single simulator** — the same world, with more
actions / state / horizon unlocked at each level:

```
L1 Match   — one match: lineup + in-match tactical adjustments
L2 Coach   — one season: squad, tactics, rotation, training (transfers frozen)
L3 Manager — one season: + transfers, contracts, scouting, finance
L4 Dynasty — 3–5 seasons: + youth, ageing, succession, long-term finances
```

Two leaderboards (methodology follows the PokéAgent Challenge):

- **Model Track** — fixed agent scaffold, only the foundation model changes
  (`Which foundation model is better at long-horizon management?`)
- **Agent Track** — open harness: rule-based / RL / raw LLM / LLM+memory /
  LLM+planning / multi-agent / MCP agents (`What agent architecture best
  manages a club?`)

## Environment

Built on [Openfoot Manager](https://github.com/openfootmanager/openfootmanager)
(GPLv3), a headless-capable football management simulator with an existing MCP
tool server (89 tools). Local fork pinned at commit `ffd7023` (v0.3.0-nightly,
branch `clubbench-dev`).

Measured with the headless season runner (`ofm-headless`):
- match engine ≈ **3,750 games/sec**;
- a full season (≈ 400 days) advances in ≈ **0.4 s**.

### Reproducibility status
- **Fully deterministic**: the simulator is patched (fork branch
  `clubbench-dev`) to thread a keyed RNG through world generation, the match
  engine and all turn subsystems. Same seed + same actions ⇒ **bit-identical
  trajectories**, verified: a 100-seed Greedy calibration re-run matches the
  frozen reference μ/σ to the last digit, and a 3Y run equals the first three
  years of a 10Y run.
- Episodes derive their RNG from `(scenario_seed, episode_idx)`; checkpoint
  snapshots resume bit-identically.

## Repository layout

ClubBench is the benchmark layer, self-contained and decoupled from the
simulator: `ofm_core` (the OpenFoot Manager game logic) is a **pinned git
dependency**, not forked into this repo.

```
/                     benchmark root (this repo)
├── Cargo.toml        env crate — depends on ofm_core via pinned git dep
├── src/              the environment: env / episode / agents / run / score
├── crates/ofm-headless/  the headless season probe
├── test/             pytest suite (one test_xx.py per feature)
├── docs/             scenario specs and design docs
├── README.md · CLAUDE.md · the design notes · baseline.md
```

The simulator is pinned at a fixed commit of `factnn/openfootmanager`
(see `Cargo.toml`); all experiments use that exact version plus identical
world snapshots and paired random seeds.

```
Claude Code / Codex / API agent
          ↓
      ClubBench (env + harness + scoring)
          ↓  ofm_core (pinned git dep)
  OpenFoot Manager simulation
```

## Status

- [x] Simulator fork + pinned version + Rust toolchain
- [x] Headless season runner / determinism probe (`ofm-headless`)
- [x] **Full determinism** — `ofm_core::rng::set_seed(seed)` reproduces the
      whole season trajectory; verified by `--check-determinism`
- [x] **Environment interface** (`crates/clubbench`) — observe / act / step,
      XI-aware match engine
- [x] **Gate-0 experiment** — lineup + play-style baselines (`clubbench`)
- [x] Scenario suite **specs** (`docs/scenarios.md`)
- [x] Model Track + Agent Track full evaluation (4 models × 4 harnesses ×
      1Y/3Y/10Y, 256×64 cells) — see `docs/experiments.md`

## Open Source & Release

**Two-repo layout**: this repo is the benchmark layer; the simulator is a
**pinned git dependency** on the public fork
[factnn/openfootmanager](https://github.com/factnn/openfootmanager)
(branch `clubbench-dev`, commit pinned in `Cargo.toml`). Both repos are GPLv3;
building this repo pulls the fork automatically.

Release day checklist: `docs/release_checklist.md`.

## License

- This repo: **GPLv3** (see `LICENSE.md`) — it links GPLv3 crates
  (`ofm_core`/`domain`/`db`).
- The simulator fork: **GPLv3**, inherited from upstream Openfoot Manager.
- Experiment results under `docs/`/`data/`: same license, or CC-BY-4.0 for
  tables/figures if preferred at publication time.
