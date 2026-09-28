# Reproducibility map

The released code supports new benchmark executions and scoring. The small `results/trajectories` subset provides inspectable examples without releasing the complete paid-run archive or full result tables.

| Paper evidence | File in this release |
|---|---|
| GPT-5.6 responsibility boundary example | representative matched trajectories |
| Ten-season trajectory example | representative ten-season trajectory |
| Normalization protocol | `data/calibration-*.json`, `src/score.rs`, `scripts/summarize_score.py` |

Exact provider-side inference can vary if a served model changes. The paper therefore reports exact served model identifiers and uses fixed seeds, frozen simulator code, and stored episode outputs. Provider transcripts may contain proprietary model output and are not required to inspect simulator state transitions or final scores.

To validate the local release without an API call:

```bash
cargo build --workspace --locked
cargo test --workspace --locked
python3 -m pytest test
```

To reproduce an evaluation, export the appropriate provider credential, install the selected native scaffold, and invoke `run_agent.sh` or `scripts/run_grid.sh` as shown in the README.
