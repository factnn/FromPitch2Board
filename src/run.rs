//! Episode runner: drive a season with an agent and collect the result.

use crate::agents::Agent;
use crate::env;
use crate::episode::Episode;
use crate::episode_agents::Policy;
use ofm_core::game::Game;

/// Multi-dimension club metrics, all directly readable from the game state.
/// Sport + finance + squad are the three dimensions the Manager track scores;
/// each is reported raw AND relative to a reference distribution.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ClubMetrics {
    pub points: u32,
    pub position: usize,
    pub goal_difference: i32,
    /// Club bank balance (team.finance).
    pub balance: i64,
    pub transfer_budget: i64,
    /// Sum of weekly wages.
    pub wage_bill: u64,
    /// Sum of player market values.
    pub squad_value: u64,
    pub avg_age: f64,
    pub squad_size: usize,
    /// Value created over the episode: (squad value + balance) change.
    pub net_value: i64,
    /// Net transfer outlay (buying − selling income); positive = spent money.
    pub net_spend: i64,
}

/// Extract the metrics for the user's club from a finished game state.
pub fn metrics_of(game: &Game, initial_net_worth: i64, net_spend: i64) -> ClubMetrics {
    let user_team_id = game.manager.team_id.as_deref().unwrap_or_default();
    let mut st = game
        .league
        .as_ref()
        .map(|l| l.standings.clone())
        .unwrap_or_default();
    st.sort_by(|a, b| {
        b.points
            .cmp(&a.points)
            .then_with(|| b.goal_difference().cmp(&a.goal_difference()))
    });
    let position = st
        .iter()
        .position(|s| s.team_id == user_team_id)
        .map(|i| i + 1)
        .unwrap_or(0);
    let entry = st.iter().find(|s| s.team_id == user_team_id);

    let team = game.teams.iter().find(|t| t.id == user_team_id);
    let players: Vec<&domain::player::Player> = game
        .players
        .iter()
        .filter(|p| p.team_id.as_deref() == Some(user_team_id))
        .collect();
    let wage_bill: u64 = players.iter().map(|p| p.wage as u64).sum();
    let squad_value: u64 = players.iter().map(|p| p.market_value).sum();
    let today = game.clock.current_date.format("%Y-%m-%d").to_string();
    let ages: Vec<f64> = players
        .iter()
        .filter_map(|p| {
            let b = chrono::NaiveDate::parse_from_str(&p.date_of_birth, "%Y-%m-%d").ok()?;
            let t = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d").ok()?;
            Some(((t - b).num_days() / 365) as f64)
        })
        .collect();
    let avg_age = if ages.is_empty() { 0.0 } else { ages.iter().sum::<f64>() / ages.len() as f64 };

    ClubMetrics {
        points: entry.map(|e| e.points).unwrap_or(0),
        position,
        goal_difference: entry.map(|e| e.goal_difference()).unwrap_or(0),
        balance: team.map(|t| t.finance).unwrap_or(0),
        transfer_budget: team.map(|t| t.transfer_budget).unwrap_or(0),
        wage_bill,
        squad_value,
        avg_age,
        squad_size: players.len(),
        net_value: env::net_worth(game) - initial_net_worth,
        net_spend,
    }
}

#[derive(Debug, Clone)]
pub struct EpisodeResult {
    pub seed: u64,
    pub metrics: ClubMetrics,
    pub played: u32,
    pub won: u32,
    pub drawn: u32,
    pub lost: u32,
    pub goals_for: u32,
    pub goals_against: u32,
}

/// Run one deterministic episode: `seed` fixes both the world and the entire
/// season trajectory. The agent is consulted on every user matchday.
pub fn run_episode(seed: u64, days: u64, agent: &dyn Agent) -> EpisodeResult {
    ofm_core::rng::set_seed(seed);
    let mut game = env::build_game(seed);
    let initial_net_worth = env::net_worth(&game);

    for _ in 0..days {
        if env::user_fixture_index(&game).is_some() {
            agent.decide_tactics(&mut game);
            let lineup = agent.decide_lineup(&game);
            env::apply_lineup(&mut game, &lineup);
        }
        env::advance_day(&mut game);
    }

    ofm_core::rng::reset_random();
    result_of(&game, seed, initial_net_worth, 0)
}

fn result_of(game: &Game, seed: u64, initial_net_worth: i64, net_spend: i64) -> EpisodeResult {
    let m = metrics_of(game, initial_net_worth, net_spend);
    let tid = game.manager.team_id.as_deref().unwrap_or_default();
    let e = game
        .league
        .as_ref()
        .and_then(|l| l.standings.iter().find(|s| s.team_id == tid));
    EpisodeResult {
        seed,
        metrics: m,
        played: e.map(|e| e.played).unwrap_or(0),
        won: e.map(|e| e.won).unwrap_or(0),
        drawn: e.map(|e| e.drawn).unwrap_or(0),
        lost: e.map(|e| e.lost).unwrap_or(0),
        goals_for: e.map(|e| e.goals_for).unwrap_or(0),
        goals_against: e.map(|e| e.goals_against).unwrap_or(0),
    }
}

/// Result of a decision-cadence episode.
#[derive(Debug, Clone)]
pub struct CadenceResult {
    pub seed: u64,
    pub steps: u64,
    pub metrics: ClubMetrics,
    pub played: u32,
    pub won: u32,
    pub drawn: u32,
    pub lost: u32,
    pub goals_for: u32,
    pub goals_against: u32,
}

/// Run the decision-cadence episode with a policy. The agent is consulted at
/// every decision point (matchdays + transfer offers); `steps` is the number of
/// decisions made over the horizon.
pub fn run_episode_cadence(
    seed: u64,
    horizon_days: u64,
    policy: &mut dyn Policy,
) -> CadenceResult {
    run_episode_cadence_for(seed, &env::ClubPick::Index(0), horizon_days, policy)
}

/// As [`run_episode_cadence`], managing the club selected by `pick`.
pub fn run_episode_cadence_for(
    seed: u64,
    pick: &env::ClubPick,
    horizon_days: u64,
    policy: &mut dyn Policy,
) -> CadenceResult {
    run_episode_cadence_for_world(seed, pick, env::WorldSize::Medium, &env::ScenarioBudget::default(), horizon_days, policy)
}

/// As [`run_episode_cadence_for`], with an explicit world size and budget.
pub fn run_episode_cadence_for_world(
    seed: u64,
    pick: &env::ClubPick,
    world: env::WorldSize,
    budget: &env::ScenarioBudget,
    horizon_days: u64,
    policy: &mut dyn Policy,
) -> CadenceResult {
    run_episode_cadence_with_mode(seed, pick, world, budget, env::AgentMode::Manager, horizon_days, policy)
}

/// As [`run_episode_cadence_for_world`], with an explicit agent mode.
pub fn run_episode_cadence_with_mode(
    seed: u64,
    pick: &env::ClubPick,
    world: env::WorldSize,
    budget: &env::ScenarioBudget,
    mode: env::AgentMode,
    horizon_days: u64,
    policy: &mut dyn Policy,
) -> CadenceResult {
    run_episode_cadence_recorded(seed, pick, world, budget, mode, budget.name(), horizon_days, policy).0
}

/// A persisted episode: the final game state plus the metadata needed to
/// (re-)score it later. Scoring is decoupled from trajectory generation — any
/// scoring version (new dimensions, calibration-set Z, …) can be applied to an
/// existing trajectory without re-running the agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrajectoryRecord {
    pub seed: u64,
    pub scenario: String,
    pub club: usize,
    pub world: String,
    /// "Coach" or "Manager"
    pub mode: String,
    /// Which agent produced this trajectory (llm / cc / codex / greedy / ...),
    /// needed to group coach + manager runs for the composition gap.
    #[serde(default = "default_agent")]
    pub agent: String,
    pub horizon_days: u64,
    pub initial_net_worth: i64,
    pub net_spend: i64,
    pub snapshots: Vec<SeasonSnapshot>,
    pub final_game: Game,
}

fn default_agent() -> String {
    "unknown".to_string()
}

impl TrajectoryRecord {
    /// The 7-dimension metrics for this trajectory (any scoring version can be
    /// layered on top of these raw values).
    pub fn metrics(&self) -> ClubMetrics {
        metrics_of(&self.final_game, self.initial_net_worth, self.net_spend)
    }
}

/// The reset metadata needed to build a [`TrajectoryRecord`] (used by the MCP
/// server's `dump` tool, which serializes the live episode).
#[derive(Debug, Clone)]
pub struct TrajectoryMeta {
    pub seed: u64,
    pub scenario: String,
    pub club: usize,
    pub world: String,
    pub mode: String,
    pub agent: String,
    pub horizon_days: u64,
}

/// Like [`run_episode_cadence_with_mode`], but also returns the persisted
/// trajectory record so the run can be re-scored without re-running.
pub fn run_episode_cadence_recorded(
    seed: u64,
    pick: &env::ClubPick,
    world: env::WorldSize,
    budget: &env::ScenarioBudget,
    mode: env::AgentMode,
    scenario: &str,
    horizon_days: u64,
    policy: &mut dyn Policy,
) -> (CadenceResult, TrajectoryRecord) {
    let mut ep = Episode::new_with_mode(seed, pick, world, budget, mode, horizon_days);
    let mut obs = ep.observe();
    let mut guard = 0u64;
    while !obs.done && guard < 200_000 {
        let action = policy.act(&obs);
        obs = ep.step(action);
        guard += 1;
    }

    let r = result_of(&ep.game, seed, ep.initial_net_worth, ep.net_spend);
    let club = match pick {
        env::ClubPick::Strength(rank) => *rank,
        env::ClubPick::Index(i) => *i,
    };
    let record = TrajectoryRecord {
        seed,
        scenario: scenario.to_string(),
        club,
        world: format!("{world:?}"),
        mode: format!("{mode:?}"),
        agent: policy.name().to_string(),
        horizon_days,
        initial_net_worth: ep.initial_net_worth,
        net_spend: ep.net_spend,
        snapshots: ep.season_snapshots(),
        final_game: ep.game.clone(),
    };
    let result = CadenceResult {
        seed,
        steps: ep.step_count(),
        metrics: r.metrics,
        played: r.played,
        won: r.won,
        drawn: r.drawn,
        lost: r.lost,
        goals_for: r.goals_for,
        goals_against: r.goals_against,
    };
    (result, record)
}

/// One season's end-of-season snapshot, for the multi-season curve.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SeasonSnapshot {
    pub season: u32,
    pub points: u32,
    pub position: usize,
    pub balance: i64,
    pub squad_value: u64,
    pub avg_age: f64,
    pub squad_size: usize,
    pub net_value: i64,
    pub net_spend: i64,
}

/// Run several seasons in one continuous episode, snapshotting the club's
/// metrics at every REAL season boundary (league complete) and rolling the
/// world over via [`env::rollover_season`]. The squad, finances and clock carry
/// over between seasons — this is the long-horizon Dynasty trajectory.
/// `season_days` is only a safety cap on the total episode length.
pub fn run_multi_season(
    seed: u64,
    pick: &env::ClubPick,
    world: env::WorldSize,
    budget: &env::ScenarioBudget,
    mode: env::AgentMode,
    seasons: u32,
    season_days: u64,
    policy: &mut dyn Policy,
) -> Vec<SeasonSnapshot> {
    let total_days = season_days * seasons as u64; // outer safety cap
    let mut ep = Episode::new_with_mode(seed, pick, world, budget, mode, total_days);
    let mut obs = ep.observe();
    let mut snapshots = Vec::new();
    let mut guard = 0u64;

    while ep.advanced_days() < total_days && guard < 5_000_000 {
        let action = policy.act(&obs);
        obs = ep.step(action);
        guard += 1;
        if ep.season_complete() {
            let m = ep.final_metrics();
            snapshots.push(SeasonSnapshot {
                season: snapshots.len() as u32 + 1,
                points: m.points,
                position: m.position,
                balance: m.balance,
                squad_value: m.squad_value,
                avg_age: m.avg_age,
                squad_size: m.squad_size,
                net_value: m.net_value,
                net_spend: m.net_spend,
            });
            if snapshots.len() >= seasons as usize {
                break;
            }
            env::rollover_season(&mut ep.game);
        }
    }
    snapshots
}
