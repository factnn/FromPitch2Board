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
- Raw world generation **is** seed-reproducible (`generate_world_data_seeded`).
- Entity IDs and the game-build glue use ambient RNG → the current reset
  strategy is **generate-once → save snapshot → load per episode**.
- The season trajectory is not yet deterministic (ambient RNG in the match
  engine / turn subsystems). Full per-episode determinism requires threading a
  seeded RNG through the engine/turn loop — planned.

## Repository layout

```
/                     benchmark root (this repo)
├── README.md
├── CLAUDE.md         working notes for AI agents
├── the design notes        full design discussion (background, methodology, validity, legal, naming)
└── openfootmanager/  (NOT tracked here — separate fork repo, pinned ffd7023)
```

## Status

- [x] Simulator fork + pinned version + Rust toolchain
- [x] Headless season runner / determinism probe (`ofm-headless`)
- [ ] Full determinism (seeded RNG through engine/turn)
- [ ] `observe()/act()/step()` environment interface
- [ ] Gate-0 experiment: heuristic vs. strong-LLM baseline
- [ ] Scenario suite (relegation / rebuild / moneyball / dynasty)
- [ ] Model Track + Agent Track baselines, leaderboard

## License

ClubBench (this repo) and all ClubBench-authored code: TBD — see the licensing
discussion in `the design notes`. The simulator fork is GPLv3 (upstream Openfoot Manager).
