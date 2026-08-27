//! ClubBench CLI.
//!
//! `gate0`    — the original lineup/tactics experiment (single-action baselines).
//! `cadence`  — the decision-cadence experiment: the agent is consulted at every
//!              matchday and transfer offer, producing a long decision trajectory.

use clubbench::agents::{Agent, BestXIAgent, NoopAgent, RandomXIAgent, StyleProbe, WorstXIAgent};
use clubbench::episode_agents::{
    AutoManager, GreedyManager, OffersOnlyManager, PassiveManager, Policy, ProactiveManager,
    SellingManager,
};
use clubbench::run::{run_episode, run_episode_cadence_for_world};
use clubbench::score;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clubbench", about = "ClubBench environment experiments")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Original lineup/tactics experiment
    Gate0 {
        #[arg(long, default_value = "42,43,44")]
        seeds: String,
        #[arg(long, default_value_t = 400)]
        days: u64,
    },
    /// Decision-cadence experiment (matchdays + transfer offers)
    Cadence {
        #[arg(long, default_value = "42,43,44")]
        seeds: String,
        #[arg(long, default_value_t = 400)]
        days: u64,
        /// World size: medium (default) | standard | compact
        #[arg(long, default_value = "medium")]
        world: String,
        /// Scenario budget: crisis | moneyball | rebuild (default) | title
        #[arg(long, default_value = "rebuild")]
        scenario: String,
    },
    /// Paired-seed, reference-relative scoring (the benchmark evaluation protocol)
    Score {
        #[arg(long, default_value = "42,43,44,45,46,47,48,49")]
        seeds: String,
        #[arg(long, default_value_t = 400)]
        days: u64,
        /// Managed club by squad-strength rank (0 = weakest)
        #[arg(long)]
        club: Option<usize>,
        /// World size: medium (default, ~120 clubs) | standard (~440) | compact (~16)
        #[arg(long, default_value = "medium")]
        world: String,
        /// Scenario budget: crisis | moneyball | rebuild (default) | title
        #[arg(long, default_value = "rebuild")]
        scenario: String,
    },
    /// Run the full benchmark suite (scenario grid × all baselines) — one command
    Run {
        #[arg(long, default_value = "42,43")]
        seeds: String,
        #[arg(long, default_value_t = 300)]
        days: u64,
        /// World size: medium (default) | standard | compact
        #[arg(long, default_value = "medium")]
        world: String,
        /// Managed-club strength ranks (comma-separated); 0 = weakest.
        /// Omit to use each scenario's archetype rank (crisis=5, moneyball=40,
        /// rebuild=75, title=110).
        #[arg(long)]
        clubs: Option<String>,
        /// Scenario budgets (comma-separated): crisis,moneyball,rebuild,title
        #[arg(long, default_value = "crisis,rebuild")]
        scenarios: String,
        /// Track: manager (transfers+scouting+finance) | coach (lineup/tactics only)
        #[arg(long, default_value = "manager")]
        mode: String,
    },
    /// Re-score one or more saved trajectories (TrajectoryRecord JSON) without
    /// re-running the agent. Any scoring version can be layered on later.
    ScoreTrajectory {
        /// Paths to trajectory.json files
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Composition Gap (the design notes §3): given a set of coach + manager trajectory
    /// files (one per agent/seed), report BOTH the relative gap G_Z (standardized)
    /// and the raw composition delta G_Δ = Δ_coach − Δ_manager in league points,
    /// each vs the per-track Greedy reference.
    Gap {
        /// trajectory.json files (coach and manager runs for each agent)
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Build the frozen Reference Calibration Set (the design notes §2): run the Greedy
    /// reference on many seeds per (scenario, club) cell and save μ/σ to
    /// data/calibration-{world}.json. All agents' Z then use this stable scale.
    Calibrate {
        #[arg(long, default_value = "medium")]
        world: String,
        /// Club strength ranks. Omit to calibrate only each scenario's
        /// archetype rank (crisis=5, moneyball=40, rebuild=75, title=110) —
        /// the cells the main benchmark actually uses.
        #[arg(long)]
        clubs: Option<String>,
        #[arg(long, default_value = "crisis,moneyball,rebuild,title")]
        scenarios: String,
        /// Number of calibration seeds (0..count)
        #[arg(long, default_value_t = 200)]
        count: u64,
        #[arg(long, default_value_t = 400)]
        days: u64,
        /// Track whose reference to calibrate: coach (GreedyCoach) or manager
        /// (GreedyManager). Coach/Manager calibrations are kept separate.
        #[arg(long, default_value = "manager")]
        mode: String,
        /// Override the output path (default data/calibration-{world}-{mode}.json;
        /// used by tests to avoid touching the shared file).
        #[arg(long)]
        out: Option<String>,
    },
    /// Multi-season Dynasty trajectory — per-season snapshots over several seasons
    Multi {
        #[arg(long, default_value_t = 10)]
        seasons: u32,
        #[arg(long, default_value_t = 400)]
        season_days: u64,
        /// Rule policy to drive the episode (auto|proactive|selling|offersonly|passive)
        #[arg(long, default_value = "auto")]
        policy: String,
        #[arg(long, default_value = "rebuild")]
        scenario: String,
        #[arg(long, default_value_t = 15)]
        club: usize,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// World size: medium (default) | standard | compact
        #[arg(long, default_value = "compact")]
        world: String,
        /// Track: manager | coach
        #[arg(long, default_value = "manager")]
        mode: String,
        /// Also print JSON (for the curve plotter)
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Gate0 { seeds, days } => gate0(&seeds, days),
        Commands::Cadence { seeds, days, world, scenario } => cadence(&seeds, days, &world, &scenario),
        Commands::Score { seeds, days, club, world, scenario } => score_cmd(&seeds, days, club, &world, &scenario),
        Commands::Run { seeds, days, world, clubs, scenarios, mode } => {
            run_benchmark(&seeds, days, &world, clubs.as_deref().unwrap_or(""), &scenarios, &mode)
        }
        Commands::Multi { seasons, season_days, policy, scenario, club, seed, world, mode, json } => {
            multi_season(seasons, season_days, &policy, &scenario, club, seed, &world, &mode, json)
        }
        Commands::ScoreTrajectory { files } => score_trajectory(&files),
        Commands::Gap { files } => gap(&files),
        Commands::Calibrate { world, clubs, scenarios, count, days, mode, out } => {
            calibrate(&world, clubs.as_deref().unwrap_or(""), &scenarios, count, days, &mode, out.as_deref())
        }
    }
}

/// Composition Gap (the design notes §3): for each agent with BOTH a coach and a manager
/// trajectory on the same seeds, report the relative gap G_Z and the raw
/// composition delta G_Δ = Δ_coach − Δ_manager (in league points), each vs the
/// per-track Greedy reference. Example: coach LLM 75 / Greedy 68 (+7), manager
/// LLM 70 / Greedy 72 (−2) → "relative advantage drops by 9 league points".
fn gap(files: &[String]) {
    use clubbench::env::{AgentMode, ClubPick, ScenarioBudget, WorldSize};
    use clubbench::episode_agents::{GreedyCoach, GreedyManager};
    use clubbench::run::{run_episode_cadence_with_mode, TrajectoryRecord};
    use std::collections::BTreeMap;

    // load records
    let mut recs = Vec::new();
    for f in files {
        let text = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => { eprintln!("skip {f}: {e}"); continue; }
        };
        match serde_json::from_str::<TrajectoryRecord>(&text) {
            Ok(r) => recs.push(r),
            Err(e) => eprintln!("skip {f}: {e}"),
        }
    }
    // group by (world, scenario, club, agent) -> mode -> seed -> points
    let mut groups: BTreeMap<(String, String, usize, String), BTreeMap<String, BTreeMap<u64, f64>>> = BTreeMap::new();
    let mut horizons: BTreeMap<(String, String, usize, String), u64> = BTreeMap::new();
    for r in &recs {
        let m = r.metrics();
        let key = (r.world.clone(), r.scenario.clone(), r.club, r.agent.clone());
        groups.entry(key.clone())
            .or_default()
            .entry(r.mode.clone())
            .or_default()
            .insert(r.seed, m.points as f64);
        horizons.entry(key).or_insert(r.horizon_days);
    }

    fn mean(xs: &[f64]) -> f64 { if xs.is_empty() { 0.0 } else { xs.iter().sum::<f64>() / xs.len() as f64 } }
    fn std(xs: &[f64]) -> f64 {
        if xs.len() < 2 { return 0.0; }
        let m = mean(xs);
        (xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (xs.len() - 1) as f64).sqrt()
    }

    println!(
        "{:<8} {:>9} {:>8} {:>7} | {:>9} {:>8} {:>7} | {:>7} {:>8}",
        "agent", "P_coach", "Δ_coach", "Z_C", "P_mgr", "Δ_mgr", "Z_M", "G_Z", "G_Δ(pts)"
    );
    for ((world, scenario, club, agent), modes) in &groups {
        let coach = modes.get("Coach");
        let mgr = modes.get("Manager");
        let (Some(coach), Some(mgr)) = (coach, mgr) else { continue; };
        let seeds: Vec<u64> = coach.keys().filter(|s| mgr.contains_key(s)).copied().collect();
        if seeds.is_empty() { continue; }
        let world_key = world.clone();
        let world = match world.as_str() {
            "Compact" => WorldSize::Compact,
            "Standard" => WorldSize::Standard,
            _ => WorldSize::Medium,
        };
        let budget = ScenarioBudget::by_name(scenario);
        let pick = ClubPick::Strength(*club);
        let horizon = horizons.get(&(world_key, scenario.clone(), *club, agent.clone())).copied().unwrap_or(400);

        // reference runs on the same seeds (same horizon as the agent)
        let (mut g_coach, mut g_mgr) = (GreedyCoach { play_style: domain::team::PlayStyle::Balanced }, GreedyManager::new(domain::team::PlayStyle::Balanced));
        let mut ref_c: Vec<f64> = Vec::new();
        let mut ref_m: Vec<f64> = Vec::new();
        for &s in &seeds {
            ref_c.push(run_episode_cadence_with_mode(s, &pick, world, &budget, AgentMode::Coach, horizon, &mut g_coach).metrics.points as f64);
            ref_m.push(run_episode_cadence_with_mode(s, &pick, world, &budget, AgentMode::Manager, horizon, &mut g_mgr).metrics.points as f64);
        }
        let (gc_mu, gc_sd) = (mean(&ref_c), std(&ref_c));
        let (gm_mu, gm_sd) = (mean(&ref_m), std(&ref_m));

        let p_c = mean(&seeds.iter().map(|s| coach[s]).collect::<Vec<_>>());
        let p_m = mean(&seeds.iter().map(|s| mgr[s]).collect::<Vec<_>>());
        let d_c = p_c - gc_mu;
        let d_m = p_m - gm_mu;
        let zc = if gc_sd > 1e-9 { (p_c - gc_mu) / gc_sd } else { d_c.signum() };
        let zm = if gm_sd > 1e-9 { (p_m - gm_mu) / gm_sd } else { d_m.signum() };
        let gz = zc - zm;
        let gd = d_c - d_m;
        println!(
            "{:<8} {:>9.1} {:>+8.1} {:>+7.2} | {:>9.1} {:>+8.1} {:>+7.2} | {:>+7.2} {:>+8.1}",
            agent, p_c, d_c, zc, p_m, d_m, zm, gz, gd
        );
    }
}

/// Build and save the frozen calibration set (the design notes §2), per track.
fn calibrate(world_str: &str, clubs_str: &str, scenarios_str: &str, count: u64, days: u64, mode_str: &str, out: Option<&str>) {
    use clubbench::env::AgentMode;
    use clubbench::score::build_calibration;
    let world = world_size(world_str);
    let mode = if mode_str == "coach" { AgentMode::Coach } else { AgentMode::Manager };
    let scenarios: Vec<&str> = scenarios_str.split(',').map(str::trim).collect();
    let cells: Vec<(String, usize)> = if clubs_str.trim().is_empty() {
        // Archetype-only: calibrate each scenario's own fixed club rank.
        scenarios.iter().map(|s| (s.to_string(), clubbench::env::ScenarioBudget::club_rank(s))).collect()
    } else {
        let clubs: Vec<usize> = clubs_str.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        scenarios.iter().flat_map(|s| clubs.iter().map(|&c| (s.to_string(), c))).collect()
    };
    let seeds: Vec<u64> = (0..count).collect();
    println!(
        "ClubBench Calibrate — world={world:?}, mode={mode:?}, cells={cells:?}, {count} seeds × {days} days"
    );
    let cal = build_calibration(world, &cells, &seeds, days, mode);
    let result = match out {
        Some(path) => {
            let parent = std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("."));
            std::fs::create_dir_all(parent)
                .and_then(|_| std::fs::write(path, serde_json::to_string_pretty(&cal).unwrap()))
                .map_err(|e| e.to_string())
        }
        None => cal.save(mode).map_err(|e| e.to_string()),
    };
    match result {
        Ok(()) => println!("saved ({} cells)", cal.cells.len()),
        Err(e) => eprintln!("save failed: {e}"),
    }
}

/// Load saved trajectory records and print their metrics (the decoupled
/// re-scoring entry point — no agent re-run needed).
fn score_trajectory(files: &[String]) {
    use clubbench::run::TrajectoryRecord;
    println!("{:<10} {:>5} {:>6} {:>6} {:>5} {:>12} {:>12} {:>7} {:>6} {:>12} {:>12}",
        "file", "seed", "club", "mode", "pts", "balance", "squad_val", "avg_age", "size", "net_value", "net_spend");
    for f in files {
        let json = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => { eprintln!("skip {f}: {e}"); continue; }
        };
        let rec: TrajectoryRecord = match serde_json::from_str(&json) {
            Ok(r) => r,
            Err(e) => { eprintln!("skip {f}: {e}"); continue; }
        };
        let m = rec.metrics();
        println!("{:<10} {:>5} {:>6} {:>6} {:>5} {:>12} {:>12} {:>7.1} {:>6} {:>12} {:>12}",
            std::path::Path::new(f).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            rec.seed, rec.club, rec.mode, m.points, m.balance, m.squad_value, m.avg_age, m.squad_size, m.net_value, m.net_spend);
    }
}

/// Run a multi-season Dynasty trajectory and print per-season snapshots.
fn multi_season(
    seasons: u32,
    season_days: u64,
    policy_name: &str,
    scenario: &str,
    club: usize,
    seed: u64,
    world_str: &str,
    mode_str: &str,
    json: bool,
) {
    use clubbench::env::{AgentMode, ClubPick, ScenarioBudget, WorldSize};
    use clubbench::run::run_multi_season;

    let world = match world_str {
        "standard" => WorldSize::Standard,
        "compact" => WorldSize::Compact,
        _ => WorldSize::Medium,
    };
    let mode = if mode_str == "coach" { AgentMode::Coach } else { AgentMode::Manager };
    let budget = ScenarioBudget::by_name(scenario);
    let pick = ClubPick::Strength(club);

    let mut policy: Box<dyn Policy> = match policy_name {
        "greedy" => Box::new(GreedyManager::new(domain::team::PlayStyle::Balanced)),
        "proactive" => Box::new(ProactiveManager::new(domain::team::PlayStyle::Attacking)),
        "selling" => Box::new(SellingManager::new(domain::team::PlayStyle::Attacking)),
        "offersonly" => Box::new(OffersOnlyManager),
        "passive" => Box::new(PassiveManager),
        _ => Box::new(AutoManager::new(domain::team::PlayStyle::Attacking)),
    };

    let snaps = run_multi_season(seed, &pick, world, &budget, mode, seasons, season_days, policy.as_mut());

    if json {
        println!("{}", serde_json::to_string_pretty(&snaps).unwrap_or_default());
        return;
    }
    println!("ClubBench Dynasty — {} seasons, {} days/season, policy={} scenario={} club={} seed={} world={:?} mode={:?}",
        seasons, season_days, policy_name, scenario, club, seed, world, mode);
    println!("{:<7} {:>6} {:>5} {:>12} {:>12} {:>7} {:>6} {:>12} {:>12}", "season", "pts", "pos", "balance", "squad_val", "avg_age", "size", "net_value", "net_spend");
    for s in &snaps {
        println!("{:<7} {:>6} {:>5} {:>12} {:>12} {:>7.1} {:>6} {:>12} {:>12}",
            s.season, s.points, s.position, s.balance, s.squad_value, s.avg_age, s.squad_size, s.net_value, s.net_spend);
    }
}

/// Run the full benchmark: for every (club, scenario) cell, score every
/// baseline candidate vs the frozen reference, and print a consolidated
/// leaderboard (per-cell Z table + overall mean Z per dimension).
fn run_benchmark(
    seeds_str: &str,
    days: u64,
    world_str: &str,
    clubs_str: &str,
    scenarios_str: &str,
    mode_str: &str,
) {
    use clubbench::env::{AgentMode, ClubPick, ScenarioBudget};
    use clubbench::score::{collect_paired_for_mode, DimReport};
    use std::collections::BTreeMap;

    let mode = match mode_str.trim().to_lowercase().as_str() {
        "coach" => AgentMode::Coach,
        _ => AgentMode::Manager,
    };
    let seeds: Vec<u64> = seeds_str.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let world = world_size(world_str);
    let explicit_clubs: Option<Vec<usize>> = if clubs_str.trim().is_empty() {
        None
    } else {
        Some(clubs_str.split(',').filter_map(|s| s.trim().parse().ok()).collect())
    };
    let scenarios: Vec<&str> = scenarios_str.split(',').map(str::trim).collect();
    // Archetype fusion: each scenario runs on its own fixed club rank unless
    // the caller explicitly overrides with --clubs.
    let clubs_of = |scenario: &str| -> Vec<usize> {
        explicit_clubs.clone().unwrap_or_else(|| vec![clubbench::env::ScenarioBudget::club_rank(scenario)])
    };

    let clubs_label = explicit_clubs
        .as_ref()
        .map(|c| format!("{c:?}"))
        .unwrap_or_else(|| "archetype-per-scenario".to_string());
    println!(
        "ClubBench Benchmark — mode={:?}, world={:?}, {} seeds, clubs={}, scenarios={:?}",
        mode,
        world,
        seeds.len(),
        clubs_label,
        scenarios
    );
    println!(
        "reference = {}",
        if mode == AgentMode::Coach { "GreedyCoach (frozen, Balanced)" } else { "ClubBench-Greedy-v1 (frozen GreedyManager, Balanced)" }
    );
    if seeds.len() < 8 {
        println!("note: Z is noisy with <8 seeds (reference σ is estimated from few samples); use 8+ seeds for stable leaderboards.\n");
    } else {
        println!();
    }

    // Coach scores only points (+ avg_age diagnostic); Manager all seven.
    let dims: Vec<&str> = clubbench::score::dimensions_for(mode)
        .iter()
        .map(|d| d.name)
        .collect();
    let mut candidates: Vec<Box<dyn Policy>> = if mode == AgentMode::Coach {
        vec![
            Box::new(clubbench::episode_agents::GreedyCoach { play_style: domain::team::PlayStyle::Balanced }),
            Box::new(clubbench::episode_agents::CoachBestXI { play_style: domain::team::PlayStyle::Attacking }),
            Box::new(clubbench::episode_agents::CoachBestXI { play_style: domain::team::PlayStyle::Balanced }),
            Box::new(clubbench::episode_agents::CoachBestXI { play_style: domain::team::PlayStyle::Defensive }),
            Box::new(clubbench::episode_agents::CoachRandom),
            Box::new(clubbench::episode_agents::CoachWorst),
        ]
    } else {
        vec![
            Box::new(clubbench::episode_agents::RandomManager),
            Box::new(PassiveManager),
            Box::new(clubbench::episode_agents::GreedyManager::new(domain::team::PlayStyle::Balanced)),
            Box::new(ProactiveManager::new(domain::team::PlayStyle::Attacking)),
            Box::new(SellingManager::new(domain::team::PlayStyle::Attacking)),
            Box::new(AutoManager::new(domain::team::PlayStyle::Balanced)),
            Box::new(OffersOnlyManager),
        ]
    };

    // overall: candidate -> dim -> (sum of Z, count of non-diagnostic cells)
    let mut overall: BTreeMap<String, Vec<(f64, usize)>> = BTreeMap::new();
    let mut cells = 0usize;

    for scenario in &scenarios {
        let budget = ScenarioBudget::by_name(scenario);
        for &club in &clubs_of(scenario) {
            let pick = ClubPick::Strength(club);
            println!("=== scenario={}  club-rank={} ===", scenario, club);

            // Reference raw baseline (difficulty anchor): its own mean metrics.
            let ref_m = clubbench::score::reference_mean_metrics(&pick, world, &budget, mode, &seeds, days);
            println!(
                "  reference raw: pts={:.1}  balance={:.0}  net_value={:.0}  squad_value={:.0}  squad_size={:.1}",
                ref_m.points, ref_m.balance, ref_m.net_value, ref_m.squad_value, ref_m.squad_size
            );

            println!("  {:<12} {}", "candidate", dims.iter().map(|d| format!("{:>9}", format!("{}(Z)", d))).collect::<Vec<_>>().join(" "));
            for candidate in candidates.iter_mut() {
                let (_, reports) = collect_paired_for_mode(&pick, world, &budget, mode, &seeds, days, candidate.as_mut());
                println!(
                    "  {:<12} {}",
                    candidate.name(),
                    reports.iter().map(|r: &DimReport| fmt_z(r.z)).collect::<Vec<_>>().join(" ")
                );
                let e = overall.entry(candidate.name().to_string()).or_insert_with(|| vec![(0.0, 0); dims.len()]);
                for (i, r) in reports.iter().enumerate() {
                    if let Some(z) = r.z {
                        e[i].0 += z;
                        e[i].1 += 1;
                    }
                }
            }
            cells += 1;
            println!();
        }
    }

    println!("=== overall: mean Z across {} cells ===", cells);
    println!("{:<12} {}", "candidate", dims.iter().map(|d| format!("{:>9}", format!("{}(Z)", d))).collect::<Vec<_>>().join(" "));
    for (name, sums) in &overall {
        let means: Vec<String> = sums
            .iter()
            .map(|(s, n)| {
                if *n > 0 { format!("{:>9.2}", s / *n as f64) } else { format!("{:>9}", "—") }
            })
            .collect();
        println!("{:<12} {}", name, means.join(" "));
    }
}

/// Render a dimension's Z, or `—` for diagnostic dims (raw only).
fn fmt_z(z: Option<f64>) -> String {
    match z {
        Some(v) => format!("{:>9.2}", v),
        None => format!("{:>9}", "—"),
    }
}

fn world_size(s: &str) -> clubbench::env::WorldSize {
    match s.trim().to_lowercase().as_str() {
        "compact" => clubbench::env::WorldSize::Compact,
        "standard" => clubbench::env::WorldSize::Standard,
        _ => clubbench::env::WorldSize::Medium,
    }
}

fn score_cmd(seeds_str: &str, days: u64, club: Option<usize>, world: &str, scenario: &str) {
    let seeds: Vec<u64> = seeds_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let pick = club.map(|rank| clubbench::env::ClubPick::Strength(rank));
    let world = world_size(world);
    let budget = clubbench::env::ScenarioBudget::by_name(scenario);
    println!(
        "ClubBench Score — paired-seed, reference-relative ({} seeds, {} days, club={:?}, world={:?}, scenario={})",
        seeds.len(),
        days,
        club,
        world,
        scenario
    );
    println!("reference = ClubBench-Greedy-v1 (frozen GreedyManager, Balanced)\n");

    let mut candidates: Vec<Box<dyn Policy>> = vec![
        Box::new(GreedyManager::new(domain::team::PlayStyle::Balanced)),
        Box::new(AutoManager::new(domain::team::PlayStyle::Attacking)),
        Box::new(AutoManager::new(domain::team::PlayStyle::Balanced)),
        Box::new(ProactiveManager::new(domain::team::PlayStyle::Attacking)),
        Box::new(SellingManager::new(domain::team::PlayStyle::Attacking)),
        Box::new(OffersOnlyManager),
        Box::new(PassiveManager),
        Box::new(clubbench::episode_agents::RandomManager),
    ];

    for candidate in candidates.iter_mut() {
        let (_, reports) = match &pick {
            Some(p) => score::collect_paired_for_world(p, world, &budget, &seeds, days, candidate.as_mut()),
            None => score::collect_paired(&seeds, days, candidate.as_mut()),
        };
        println!("=== {} vs reference ===", candidate.name());
        print!("{}", score::render_reports(&reports));
        println!();
    }
}

fn gate0(seeds_str: &str, days: u64) {
    let seeds: Vec<u64> = seeds_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let agents: Vec<Box<dyn Agent>> = vec![
        Box::new(BestXIAgent::new(55)),
        Box::new(StyleProbe { style: domain::team::PlayStyle::Balanced }),
        Box::new(StyleProbe { style: domain::team::PlayStyle::Attacking }),
        Box::new(StyleProbe { style: domain::team::PlayStyle::HighPress }),
        Box::new(StyleProbe { style: domain::team::PlayStyle::Defensive }),
        Box::new(NoopAgent),
        Box::new(RandomXIAgent),
        Box::new(WorstXIAgent),
    ];

    println!("ClubBench Gate 0 — lineup/tactics effectiveness ({} days/episode)", days);
    println!("{:<18} {:>5} {:>4} {:>4} {:>4} {:>4} {:>5} {:>8}", "agent", "seed", "pos", "P", "W", "D", "pts", "GF:GA");

    let mut agg: std::collections::BTreeMap<String, (f64, f64, f64)> = std::collections::BTreeMap::new();
    for seed in &seeds {
        for agent in &agents {
            let r = run_episode(*seed, days, agent.as_ref());
            println!(
                "{:<18} {:>5} {:>4} {:>4} {:>4} {:>4} {:>5} {:>3}:{:<3}",
                agent.name(), seed, r.metrics.position, r.played, r.won, r.drawn, r.metrics.points, r.goals_for, r.goals_against
            );
            let e = agg.entry(agent.name().to_string()).or_insert((0.0, 0.0, 0.0));
            e.0 += r.metrics.position as f64;
            e.1 += r.metrics.points as f64;
            e.2 += 1.0;
        }
        println!();
    }
    println!("=== averages over {} seeds ===", seeds.len());
    for (name, (pos, pts, n)) in &agg {
        println!("{:<18} avg_pos {:5.2}   avg_pts {:5.2}", name, pos / n, pts / n);
    }
}

fn cadence(seeds_str: &str, days: u64, world: &str, scenario: &str) {
    let seeds: Vec<u64> = seeds_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let world = world_size(world);
    let budget = clubbench::env::ScenarioBudget::by_name(scenario);
    let mut policies: Vec<Box<dyn Policy>> = vec![
        Box::new(AutoManager::new(domain::team::PlayStyle::Attacking)),
        Box::new(OffersOnlyManager),
        Box::new(PassiveManager),
    ];

    println!("ClubBench Cadence — long decision trajectory ({} days/episode, world={:?}, scenario={})", days, world, scenario);
    println!(
        "{:<14} {:>5} {:>6} {:>4} {:>4} {:>4} {:>4} {:>5} {:>8}",
        "policy", "seed", "steps", "pos", "P", "W", "D", "pts", "GF:GA"
    );

    let mut agg: std::collections::BTreeMap<String, (f64, f64, f64, f64)> = std::collections::BTreeMap::new();
    for seed in &seeds {
        for policy in policies.iter_mut() {
            let r = run_episode_cadence_for_world(*seed, &clubbench::env::ClubPick::Index(0), world, &budget, days, policy.as_mut());
            println!(
                "{:<14} {:>5} {:>6} {:>4} {:>4} {:>4} {:>4} {:>5} {:>3}:{:<3}",
                policy.name(), seed, r.steps, r.metrics.position, r.played, r.won, r.drawn, r.metrics.points, r.goals_for, r.goals_against
            );
            let e = agg.entry(policy.name().to_string()).or_insert((0.0, 0.0, 0.0, 0.0));
            e.0 += r.steps as f64;
            e.1 += r.metrics.position as f64;
            e.2 += r.metrics.points as f64;
            e.3 += 1.0;
        }
        println!();
    }
    println!("=== averages over {} seeds ===", seeds.len());
    for (name, (steps, pos, pts, n)) in &agg {
        println!(
            "{:<14} avg_steps {:7.1}   avg_pos {:5.2}   avg_pts {:5.2}",
            name, steps / n, pos / n, pts / n
        );
    }
}
