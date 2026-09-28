//! Paired-seed, reference-relative scoring — the FromPitch2Board evaluation protocol.
//!
//! Protocol (see docs/scenarios.md):
//!   1. Every agent plays the SAME set of evaluation seeds (paired design).
//!   2. On each seed, a frozen reference policy also plays the same world.
//!   3. Per dimension we report:
//!        - raw mean (candidate and reference),
//!        - paired Δ = mean over seeds of (candidate_i − reference_i), with a
//!          CI, and
//!        - Z = (candidate_mean − reference_mean) / reference_std, signed so
//!          that higher is always better.
//!   4. Raw football/finance numbers are always shown alongside Z so the
//!      leaderboard stays interpretable ("took 46 points, spent £2M less than
//!      the reference manager").

use crate::env::{AgentMode, ClubPick, ScenarioBudget, WorldSize};
use crate::episode_agents::{GreedyCoach, GreedyManager, Policy};
use crate::run::{run_episode_cadence_with_mode, CadenceResult, ClubMetrics};
use domain::team::PlayStyle;

/// Frozen reference calibration: the Greedy reference's per-cell,
/// per-dimension mean and std over a LARGE seed set (100-200 seeds), so Z uses
/// a stable scale instead of the few evaluation seeds — which can degenerate to
/// `sign(Δ)` when `ref_std ≈ 0` and lose all magnitude.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Calibration {
    pub world: String,
    pub cells: std::collections::HashMap<String, CalibrationCell>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CalibrationCell {
    pub n: usize,
    pub dims: std::collections::HashMap<String, CalibDim>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CalibDim {
    pub mu: f64,
    pub sigma: f64,
}

impl Calibration {
    pub fn cell_key(scenario: &str, club: usize) -> String {
        format!("{scenario}:{club}")
    }
    /// The calibration file name for a world + mode. Coach and Manager use
    /// DIFFERENT references (GreedyCoach vs GreedyManager), so their reference
    /// distributions are kept separate.
    pub fn file_name(world: &str, mode: AgentMode) -> String {
        let m = match mode {
            AgentMode::Coach => "Coach",
            AgentMode::Recruiter => "Manager", // ladder rung scores on Manager dims/reference
            AgentMode::Manager => "Manager",
        };
        format!("data/calibration-{world}-{m}.json")
    }
    /// Load the frozen calibration for a world + mode (None if not built yet →
    /// scoring falls back to the evaluation-seed reference statistics).
    pub fn load(world: &str, mode: AgentMode) -> Option<Calibration> {
        let text = std::fs::read_to_string(Self::file_name(world, mode)).ok()?;
        serde_json::from_str(&text).ok()
    }
    pub fn save(&self, mode: AgentMode) -> std::io::Result<()> {
        std::fs::create_dir_all("data")?;
        std::fs::write(
            Self::file_name(&self.world, mode),
            serde_json::to_string_pretty(self).unwrap(),
        )
    }
    pub fn get(&self, scenario: &str, club: usize) -> Option<&CalibrationCell> {
        self.cells.get(&Self::cell_key(scenario, club))
    }
}

/// The frozen reference policy: FromPitch2Board-Greedy-v1 (a transparent, simple,
/// deterministic greedy manager). Code is fixed and
/// public; the leaderboard is anchored on this, never on the current SOTA.
/// The greedy baseline doubles as the difficulty anchor: Z=0 ≈ the simple
/// heuristic, Z>0 = beats it.
pub fn reference_v1() -> GreedyManager {
    GreedyManager::new(PlayStyle::Balanced)
}

/// The mode-appropriate reference: the Manager track anchors on GreedyManager
/// (coach + transfers/scouting/finance), the Coach track on GreedyCoach (the
/// same greedy matchday logic with the market frozen).
pub fn reference_for(mode: AgentMode) -> Box<dyn crate::episode_agents::Policy> {
    match mode {
        AgentMode::Manager | AgentMode::Recruiter => Box::new(reference_v1()),
        AgentMode::Coach => Box::new(GreedyCoach {
            play_style: PlayStyle::Balanced,
        }),
    }
}

/// How a dimension enters the score: only a few metrics are
/// genuinely "higher is better"; finance dims are *constraints* (only
/// overspending is penalised), and squad-shape dims are diagnostics or a
/// target range — never "as low as possible".
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DimKind {
    /// Higher value is better (points, net_value, squad_value).
    Higher,
    /// Lower value is better.
    Lower,
    /// A budget constraint: `max(0, metric − budget)`, 0 = within budget,
    /// lower is better (0 is the healthy value).
    Cap(CapSource),
    /// Reported raw only, no directional Z.
    Diagnostic,
    /// Squad-size target range [22, 26]: 0 inside it, distance outside it,
    /// lower is better.
    SquadSizeRange,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CapSource {
    TransferBudget,
    WageBudget,
}

/// A named evaluation dimension with its scoring kind.
pub struct Dimension {
    pub name: &'static str,
    pub kind: DimKind,
}

/// One seed's candidate vs reference value for a dimension.
#[derive(Clone, Copy)]
pub struct PairedSample {
    pub seed: u64,
    pub candidate: f64,
    pub reference: f64,
}

/// Per-dimension report. `z` is `None` for diagnostic dims (raw only).
pub struct DimReport {
    pub name: String,
    pub candidate_mean: f64,
    pub reference_mean: f64,
    pub delta_mean: f64,
    pub delta_ci: f64,
    pub z: Option<f64>,
    pub kind: DimKind,
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn std(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n < 2 {
        return 0.0;
    }
    let m = mean(xs);
    let var = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1) as f64;
    var.sqrt()
}

fn t95(n: usize) -> f64 {
    // Approximate two-sided 95% t critical values for df = n-1.
    match n {
        2 => 12.71,
        3 => 4.30,
        4 => 3.18,
        5 => 2.78,
        6 => 2.57,
        7 => 2.45,
        8 => 2.37,
        9 => 2.31,
        10 => 2.26,
        12 => 2.20,
        15 => 2.13,
        20 => 2.09,
        30 => 2.04,
        _ => 1.96,
    }
}

/// Score one dimension over paired seeds. `cal` is the frozen calibration
/// statistic for this (scenario, club) cell when available:
///   - Z = (cand_mean − cal.mu) / cal.sigma  — a stable, cross-agent-comparable
///     scale from the Greedy reference's large-seed distribution;
///   - if cal.sigma ≈ 0 → the dim has no identifiable reference variance → no
///     Z (raw + Δ only), never a forced ±1;
///   - if `cal` is None (no calibration file) → fall back to the evaluation
///     seeds' reference std, with the legacy sign(Δ) guard.
pub fn score_dimension(samples: &[PairedSample], dim: &Dimension, cal: Option<&CalibDim>) -> DimReport {
    let cand: Vec<f64> = samples.iter().map(|s| s.candidate).collect();
    let refs: Vec<f64> = samples.iter().map(|s| s.reference).collect();
    let deltas: Vec<f64> = samples.iter().map(|s| s.candidate - s.reference).collect();

    let cand_mean = mean(&cand);
    let ref_mean = mean(&refs);
    let ref_std = std(&refs);
    let delta_mean = mean(&deltas);
    let delta_std = std(&deltas);
    let delta_ci = t95(samples.len()) * delta_std / (samples.len() as f64).sqrt();

    let z = match dim.kind {
        // Diagnostics are reported raw; no directional Z.
        DimKind::Diagnostic => None,
        _ => {
            let z_raw = match cal {
                Some(c) if c.sigma > 1e-9 => Some((cand_mean - c.mu) / c.sigma),
                Some(_) => None, // no identifiable reference variance → no Z
                None => {
                    if ref_std > 1e-9 {
                        Some((cand_mean - ref_mean) / ref_std)
                    } else if delta_mean.abs() < 1e-9 {
                        Some(0.0)
                    } else {
                        Some(delta_mean.signum())
                    }
                }
            };
            // Constraint/target dims are lower-better on the *penalty* value
            // (overspend, distance outside the healthy range): 0 = healthy.
            match (z_raw, dim.kind) {
                (Some(v), DimKind::Higher) => Some(v),
                (Some(v), _) => Some(-v),
                (None, _) => None,
            }
        }
    };

    DimReport {
        name: dim.name.to_string(),
        candidate_mean: cand_mean,
        reference_mean: ref_mean,
        delta_mean,
        delta_ci,
        z,
        kind: dim.kind,
    }
}

/// The dimensions scored per track. Coach freezes transfers, so its squad /
/// finance dims are not the coach's lever — the coach leaderboard is points
/// (with avg_age reported raw as a diagnostic). Manager scores all seven.
pub fn dimensions_for(mode: AgentMode) -> &'static [Dimension] {
    match mode {
        AgentMode::Coach => &DIMENSIONS_COACH,
        // Recruiter runs active transfers (scout/buy) — full Manager dims.
        AgentMode::Recruiter | AgentMode::Manager => &DIMENSIONS,
    }
}

/// Coach-track dimensions: points (directional) + avg_age (diagnostic).
const DIMENSIONS_COACH: [Dimension; 2] = [
    Dimension { name: "points", kind: DimKind::Higher },
    Dimension { name: "avg_age", kind: DimKind::Diagnostic },
];

/// Build the frozen calibration for a set of (scenario, club) cells: run the
/// track's Greedy reference on a large seed set and record each dimension's μ
/// and σ. This is `frompitch2board calibrate --mode`; the result is saved to
/// `data/calibration-{world}-{Mode}.json` and reused by every agent.
pub fn build_calibration(
    world: WorldSize,
    cells: &[(String, usize)],
    seeds: &[u64],
    days: u64,
    mode: AgentMode,
) -> Calibration {
    let dims = dimensions_for(mode);
    let mut cal = Calibration { world: format!("{world:?}"), cells: Default::default() };
    for (scenario, club) in cells {
        let budget = ScenarioBudget::by_name(scenario);
        let pick = ClubPick::Strength(*club);
        let mut per_dim: Vec<Vec<f64>> = dims.iter().map(|_| Vec::new()).collect();
        for &seed in seeds {
            let mut reference = reference_for(mode);
            let r = run_episode_cadence_with_mode(seed, &pick, world, &budget, mode, days, reference.as_mut());
            for (i, dim) in dims.iter().enumerate() {
                per_dim[i].push(metric_value(&r.metrics, &budget, dim.name));
            }
        }
        let mut cell_dims = std::collections::HashMap::new();
        for (i, dim) in dims.iter().enumerate() {
            cell_dims.insert(
                dim.name.to_string(),
                CalibDim { mu: mean(&per_dim[i]), sigma: std(&per_dim[i]) },
            );
        }
        cal.cells.insert(Calibration::cell_key(scenario, *club), CalibrationCell { n: seeds.len(), dims: cell_dims });
    }
    cal
}

/// Distance of a squad size from the healthy range [22, 26]: 0 inside it,
/// positive outside (lack of depth < 22, hoarding > 26).
fn squad_size_distance(size: usize) -> f64 {
    if size < 22 {
        (22 - size) as f64
    } else if size > 26 {
        (size - 26) as f64
    } else {
        0.0
    }
}

/// The dimensions scored by FromPitch2Board (sport + finance + squad), following the
/// The dimension structure:
///   - directional higher: points, net_value (net worth change), squad_value;
///   - finance as *constraints*: only budget violations are penalised
///     (transfer over-spend, wage over-budget) — not "spend as little as
///     possible", which would reward fire-selling the squad for cash;
///   - squad shape as diagnostic / target-range: avg_age raw-only, squad_size
///     scored by distance from a healthy [22, 26] range.
pub const DIMENSIONS: [Dimension; 7] = [
    Dimension { name: "points", kind: DimKind::Higher },
    Dimension { name: "net_value", kind: DimKind::Higher },
    Dimension { name: "squad_value", kind: DimKind::Higher },
    Dimension { name: "transfer_budget_violation", kind: DimKind::Cap(CapSource::TransferBudget) },
    Dimension { name: "wage_budget_violation", kind: DimKind::Cap(CapSource::WageBudget) },
    Dimension { name: "avg_age", kind: DimKind::Diagnostic },
    Dimension { name: "squad_size", kind: DimKind::SquadSizeRange },
];

fn metric_value(m: &ClubMetrics, budget: &ScenarioBudget, name: &str) -> f64 {
    match name {
        "points" => m.points as f64,
        "net_value" => m.net_value as f64,
        "squad_value" => m.squad_value as f64,
        // Constraint dims score the *violation* (0 = within budget / healthy).
        "transfer_budget_violation" => (m.net_spend - budget.transfer_budget).max(0) as f64,
        "wage_budget_violation" => (m.wage_bill as i64 - budget.wage_budget).max(0) as f64,
        "avg_age" => m.avg_age,
        "squad_size" => squad_size_distance(m.squad_size),
        _ => 0.0,
    }
}

/// Collect paired (candidate, reference) samples for every dimension over a
/// fixed set of seeds. The reference and candidate both play each seed.
pub fn collect_paired(
    seeds: &[u64],
    horizon_days: u64,
    candidate: &mut dyn Policy,
) -> (Vec<PairedSample>, Vec<DimReport>) {
    collect_paired_for(&ClubPick::Index(0), seeds, horizon_days, candidate)
}

/// As [`collect_paired`], managing the club selected by `pick`.
pub fn collect_paired_for(
    pick: &ClubPick,
    seeds: &[u64],
    horizon_days: u64,
    candidate: &mut dyn Policy,
) -> (Vec<PairedSample>, Vec<DimReport>) {
    collect_paired_for_world(pick, WorldSize::Medium, &ScenarioBudget::default(), seeds, horizon_days, candidate)
}

/// As [`collect_paired_for`], with an explicit world size and budget.
pub fn collect_paired_for_world(
    pick: &ClubPick,
    world: WorldSize,
    budget: &ScenarioBudget,
    seeds: &[u64],
    horizon_days: u64,
    candidate: &mut dyn Policy,
) -> (Vec<PairedSample>, Vec<DimReport>) {
    collect_paired_for_mode(pick, world, budget, AgentMode::Manager, seeds, horizon_days, candidate)
}

/// As [`collect_paired_for_world`], with an explicit agent mode.
pub fn collect_paired_for_mode(
    pick: &ClubPick,
    world: WorldSize,
    budget: &ScenarioBudget,
    mode: AgentMode,
    seeds: &[u64],
    horizon_days: u64,
    candidate: &mut dyn Policy,
) -> (Vec<PairedSample>, Vec<DimReport>) {
    let mut reference = reference_for(mode);
    let dims = dimensions_for(mode);
    // Frozen calibration for this (scenario, club) cell, if built.
    // Coach and Manager use separate calibration files (different references).
    let scenario = budget.name();
    let club = match pick {
        ClubPick::Strength(rank) => *rank,
        ClubPick::Index(i) => *i,
    };
    let calibration = Calibration::load(&format!("{world:?}"), mode)
        .and_then(|c| c.get(scenario, club).cloned());
    // per dimension -> Vec<PairedSample>
    let mut per_dim: Vec<Vec<PairedSample>> = dims.iter().map(|_| Vec::new()).collect();

    for &seed in seeds {
        let ref_res: CadenceResult = run_episode_cadence_with_mode(seed, pick, world, budget, mode, horizon_days, reference.as_mut());
        let cand_res: CadenceResult = run_episode_cadence_with_mode(seed, pick, world, budget, mode, horizon_days, candidate);
        for (i, dim) in dims.iter().enumerate() {
            per_dim[i].push(PairedSample {
                seed,
                candidate: metric_value(&cand_res.metrics, budget, dim.name),
                reference: metric_value(&ref_res.metrics, budget, dim.name),
            });
        }
    }

    let reports = dims
        .iter()
        .enumerate()
        .map(|(i, dim)| {
            let cal_dim = calibration.as_ref().and_then(|c| c.dims.get(dim.name));
            score_dimension(&per_dim[i], dim, cal_dim)
        })
        .collect();
    let samples: Vec<PairedSample> = per_dim.into_iter().next().unwrap_or_default();
    (samples, reports)
}

/// Mean raw metrics of the reference policy over `seeds` — the difficulty
/// anchor for a scenario cell (its points, ending balance, squad, etc.).
pub fn reference_mean_metrics(
    pick: &ClubPick,
    world: WorldSize,
    budget: &ScenarioBudget,
    mode: AgentMode,
    seeds: &[u64],
    horizon_days: u64,
) -> ClubMetrics {
    let mut reference = reference_for(mode);
    let mut sum: Option<ClubMetrics> = None;
    for &seed in seeds {
        let r = run_episode_cadence_with_mode(seed, pick, world, budget, mode, horizon_days, reference.as_mut());
        let m = r.metrics;
        sum = Some(match sum {
            None => m.clone(),
            Some(s) => ClubMetrics {
                points: s.points + m.points,
                position: s.position + m.position,
                goal_difference: s.goal_difference + m.goal_difference,
                balance: s.balance + m.balance,
                transfer_budget: s.transfer_budget + m.transfer_budget,
                wage_bill: s.wage_bill + m.wage_bill,
                squad_value: s.squad_value + m.squad_value,
                avg_age: s.avg_age + m.avg_age,
                squad_size: s.squad_size + m.squad_size,
                net_value: s.net_value + m.net_value,
                net_spend: s.net_spend + m.net_spend,
            },
        });
    }
    let s = sum.unwrap_or_default();
    let n = seeds.len().max(1) as f64;
    ClubMetrics {
        points: (s.points as f64 / n) as u32,
        position: (s.position as f64 / n) as usize,
        goal_difference: (s.goal_difference as f64 / n) as i32,
        balance: (s.balance as f64 / n) as i64,
        transfer_budget: (s.transfer_budget as f64 / n) as i64,
        wage_bill: (s.wage_bill as f64 / n) as u64,
        squad_value: (s.squad_value as f64 / n) as u64,
        avg_age: s.avg_age / n,
        squad_size: (s.squad_size as f64 / n) as usize,
        net_value: (s.net_value as f64 / n) as i64,
        net_spend: (s.net_spend as f64 / n) as i64,
    }
}

/// Render the dimension reports as a compact table. Diagnostic dims report
/// raw values with `—` for Z (no directional score).
pub fn render_reports(reports: &[DimReport]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<24} {:>10} {:>10} {:>10} {:>8} {:>7}\n",
        "dim", "cand μ", "ref μ", "Δ", "Δ±CI", "Z"
    ));
    for r in reports {
        let z = match r.z {
            Some(v) => format!("{v:>7.2}"),
            None => format!("{:>7}", "—"),
        };
        out.push_str(&format!(
            "{:<24} {:>10.1} {:>10.1} {:>10.1} {:>8.1} {}\n",
            r.name,
            r.candidate_mean,
            r.reference_mean,
            r.delta_mean,
            r.delta_ci,
            z
        ));
    }
    out
}
