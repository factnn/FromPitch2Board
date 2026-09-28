//! The decision-cadence environment.
//!
//! Unlike the raw season runner, this is a *trajectory*: the environment stops
//! at every decision point — a user matchday or a pending transfer offer — and
//! each stop is exactly one agent step. A full season therefore produces
//! hundreds of steps, and every decision's consequences compound over time
//! (injuries, morale, finances, squad building) — the long-horizon property
//! that FromPitch2Board exists to measure.
//!
//! The whole episode is reproducible: `Episode::new(seed, horizon)` seeds the
//! world AND the entire trajectory.

use crate::env;
use domain::league::FixtureStatus;
use domain::player::{Position, TransferOfferStatus};
use ofm_core::game::Game;
use ofm_core::transfers;
use serde::{Deserialize, Serialize};

/// One decision the agent can make at a decision point.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "params")]
pub enum Action {
    /// Act on nothing and advance to the next decision point (the AI default
    /// applies to anything left unhandled).
    Continue,
    /// Set the starting XI for the upcoming matchday (slot-aligned).
    SetLineup { player_ids: Vec<String> },
    /// Set the play style for the upcoming matchday.
    SetTactics { play_style: domain::team::PlayStyle },
    /// Set lineup AND play style in one action (match preparation).
    SetMatchPlan { player_ids: Vec<String>, play_style: domain::team::PlayStyle },
    /// Accept an incoming transfer offer for one of our players.
    AcceptOffer { player_id: String, offer_id: String },
    /// Reject an incoming transfer offer.
    RejectOffer { player_id: String, offer_id: String },
    /// Counter an incoming offer with our own requested fee.
    CounterOffer { player_id: String, offer_id: String, fee: u64 },
    /// Bid for a player.
    MakeBid { player_id: String, fee: u64 },
    /// Send a scout to report on a player (reveals fuzzed rating + potential band).
    Scout { player_id: String },
    /// Transfer-list one of our players for sale — attracts incoming offers.
    ListPlayer { player_id: String },
    /// Substitute a player during the live match (L1 Match track).
    Substitute { player_out_id: String, player_in_id: String },
    /// Change tactics during the live match (L1 Match track).
    MatchTactics { play_style: Option<String>, formation: Option<String> },
}

/// An incoming transfer offer for one of our players (status `Pending`).
#[derive(Serialize, Clone, Debug)]
pub struct OfferView {
    pub offer_id: String,
    pub player_id: String,
    pub player_name: String,
    pub from_team: String,
    pub fee: u64,
    pub round: u8,
    pub suggested_counter: Option<u64>,
}

/// A player available on the market we could bid on. True attributes are NOT
/// shown (partial observability); only a scout report reveals a fuzzed rating
/// and a coarse potential band.
#[derive(Serialize, Clone, Debug)]
pub struct MarketView {
    pub player_id: String,
    pub player_name: String,
    pub position: Position,
    pub age: u8,
    pub market_value: u64,
    pub team: String,
    /// Fuzzed overall rating if a scout report exists, else None.
    pub reported_ovr: Option<u8>,
    /// Coarse potential band ("worldClass"/"strong"/"moderate"/"unclear") if scouted.
    pub potential: Option<String>,
    /// A scout is currently watching this player.
    pub scouting: bool,
}

/// A completed scout report (from the game's message inbox).
#[derive(Serialize, Clone, Debug)]
pub struct ScoutReportView {
    pub player_id: String,
    pub player_name: String,
    pub team: String,
    /// Fuzzed overall rating (1-99).
    pub avg_rating: Option<u32>,
    pub rating_desc: String,
    pub potential: String,
    pub confidence: String,
}

/// A scouting assignment in progress.
#[derive(Serialize, Clone, Debug)]
pub struct ScoutingView {
    pub player_id: String,
    pub player_name: String,
    pub days_remaining: u32,
}

/// A player as seen from the live-match (L1) observation.
#[derive(Serialize, Clone, Debug)]
pub struct LiveMatchPlayer {
    pub id: String,
    pub name: String,
    pub position: String,
    /// Live condition (drops as the match wears on).
    pub condition: u8,
}

/// The in-match observation at an L1 checkpoint stop.
#[derive(Serialize, Clone, Debug)]
pub struct LiveMatchView {
    pub minute: u8,
    pub phase: String,
    pub home_score: u8,
    pub away_score: u8,
    /// "home" or "away" — the side the agent controls.
    pub user_side: String,
    pub field: Vec<LiveMatchPlayer>,
    pub bench: Vec<LiveMatchPlayer>,
    pub subs_made: u8,
    pub subs_max: u8,
    /// Notable events since the previous stop (goals, cards, injuries).
    pub events: Vec<String>,
}

/// The full decision-point observation.
#[derive(Serialize, Clone, Debug)]
pub struct EpisodeObservation {
    pub step: u64,
    pub date: String,
    pub team_name: String,
    pub formation: String,
    pub league_position: usize,
    pub points: u32,
    pub budget: i64,
    pub is_matchday: bool,
    pub next_fixture: Option<String>,
    pub squad: Vec<env::PlayerView>,
    pub offers: Vec<OfferView>,
    pub market: Vec<MarketView>,
    pub scout_reports: Vec<ScoutReportView>,
    pub scouting_in_progress: Vec<ScoutingView>,
    pub transfer_window_open: bool,
    /// Present at an L1 in-match checkpoint stop (match_stops enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_match: Option<LiveMatchView>,
    /// Human-readable outcome of the previous action (e.g. "Bid of £5.5M for
    /// Player_12: REJECTED."). `None` on the very first observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_action_result: Option<String>,
    pub done: bool,
}

/// The decision-cadence episode.
pub struct Episode {
    pub game: Game,
    pub step: u64,
    pub mode: env::AgentMode,
    pub(crate) initial_net_worth: i64,
    /// Net transfer spend over the episode (buying − selling income).
    pub(crate) net_spend: i64,
    horizon_days: u64,
    advanced_days: u64,
    /// How many seasons the episode spans (1 = classic single season, 3 = C
    /// 3Y, 10 = E 10Y). At every REAL season boundary the world rolls over
    /// (squad/finances/clock carry) until `target_seasons` is reached.
    target_seasons: u32,
    seasons_completed: u32,
    /// L1 Match track: pause user matches at fixed checkpoints (30'/HT/60'/75')
    /// so the agent can substitute and change tactics mid-game.
    match_stops: bool,
    /// The live match currently in progress (Some between the kickoff of a
    /// user matchday and the final whistle).
    stopped_match: Option<ofm_core::live_match_manager::LiveMatchSession>,
    /// Index into `MATCH_CHECKPOINTS` of the next stop.
    next_checkpoint: usize,
    /// The user's fixture (competition_index, fixture_index) whose matchday was
    /// processed with the fixture skipped — it must be played via the
    /// stopped-match flow at the next decision.
    pending_user_fixture: Option<(usize, usize)>,
    /// Metrics snapshot recorded at each season boundary (Dynasty curve).
    snapshots: Vec<crate::run::SeasonSnapshot>,
    /// Offer ids already shown to the agent — new offers (never-seen ids) are
    /// the only offer decision points, so `Continue` doesn't re-stop forever.
    seen_offers: std::collections::HashSet<String>,
    /// Outcome of the most recent action, surfaced in the next observation so
    /// the agent sees what happened (bid rejected/accepted, scout sent, ...).
    last_action_result: Option<String>,
    /// The club the agent manages for the WHOLE episode. The sim's daily loop
    /// can fire the manager for bad results (`check_manager_firing` clears
    /// `manager.team_id`, which zeroes every metric); in the benchmark the
    /// board never fires the agent mid-episode, so we re-hire each time and
    /// count the firings (a potential "would have been fired" metric).
    user_team_id: String,
    manager_firings: u32,
}

impl Episode {
    /// Start a reproducible episode managing the first club: `seed` fixes the
    /// world and trajectory; `horizon_days` bounds the episode length.
    pub fn new(seed: u64, horizon_days: u64) -> Self {
        Self::new_with_pick(seed, &env::ClubPick::Index(0), horizon_days)
    }

    /// Start a reproducible episode managing the club selected by `pick`.
    pub fn new_with_pick(seed: u64, pick: &env::ClubPick, horizon_days: u64) -> Self {
        Self::new_with_pick_and_world(seed, pick, env::WorldSize::Medium, &env::ScenarioBudget::default(), horizon_days)
    }

    /// Start a reproducible episode with an explicit world size and scenario
    /// budget (Manager track).
    pub fn new_with_pick_and_world(
        seed: u64,
        pick: &env::ClubPick,
        world: env::WorldSize,
        budget: &env::ScenarioBudget,
        horizon_days: u64,
    ) -> Self {
        Self::new_with_mode(seed, pick, world, budget, env::AgentMode::Manager, horizon_days)
    }

    /// Start a reproducible episode with an explicit world size, budget and
    /// agent mode (Coach = transfers frozen, Manager = full market).
    pub fn new_with_mode(
        seed: u64,
        pick: &env::ClubPick,
        world: env::WorldSize,
        budget: &env::ScenarioBudget,
        mode: env::AgentMode,
        horizon_days: u64,
    ) -> Self {
        Self::new_with_mode_seasons(seed, pick, world, budget, mode, horizon_days, 1)
    }

    /// Same as [`new_with_mode`] but spanning `seasons` seasons (1 = classic
    /// single season, 3 = C 3Y, 10 = E 10Y). `horizon_days` is the absolute
    /// safety cap on episode length.
    pub fn new_with_mode_seasons(
        seed: u64,
        pick: &env::ClubPick,
        world: env::WorldSize,
        budget: &env::ScenarioBudget,
        mode: env::AgentMode,
        horizon_days: u64,
        seasons: u32,
    ) -> Self {
        ofm_core::rng::set_seed(seed);
        let game = env::build_game_for_club_with(seed, pick, world, budget);
        let initial_net_worth = env::net_worth(&game);
        Self {
            user_team_id: game.manager.team_id.clone().unwrap_or_default(),
            game,
            step: 0,
            mode,
            initial_net_worth,
            net_spend: 0,
            horizon_days,
            advanced_days: 0,
            target_seasons: seasons.max(1),
            seasons_completed: 0,
            snapshots: Vec::new(),
            seen_offers: Default::default(),
            last_action_result: None,
            manager_firings: 0,
            match_stops: false,
            stopped_match: None,
            next_checkpoint: 0,
            pending_user_fixture: None,
        }
    }

    /// Enable the L1 Match track: user matches pause at fixed checkpoints so
    /// the agent can substitute and change tactics mid-game. Off by default —
    /// all classic episodes keep their pre-match-only decision cadence.
    pub fn with_match_stops(mut self, on: bool) -> Self {
        self.match_stops = on;
        self
    }

    /// Is a live match currently in progress (between an L1 checkpoint stop
    /// and the final whistle)? Used by the MCP layer to skip checkpoint
    /// persistence mid-match.
    pub fn in_match(&self) -> bool {
        self.stopped_match.is_some()
    }

    /// Resume an interrupted episode from a checkpoint (game state + counters).
    /// The keyed RNG is re-seeded with the ORIGINAL episode seed so the
    /// continuation is bit-identical to an uninterrupted run (day/match keys
    /// derive from the seed, not from how many steps happened before).
    #[allow(clippy::too_many_arguments)]
    pub fn resume(
        seed: u64,
        game: Game,
        mode: env::AgentMode,
        horizon_days: u64,
        target_seasons: u32,
        seasons_completed: u32,
        advanced_days: u64,
        step: u64,
        initial_net_worth: i64,
        net_spend: i64,
        snapshots: Vec<crate::run::SeasonSnapshot>,
        seen_offers: std::collections::HashSet<String>,
        manager_firings: u32,
        user_team_id: String,
    ) -> Self {
        ofm_core::rng::set_seed(seed);
        Self {
            game,
            step,
            mode,
            initial_net_worth,
            net_spend,
            horizon_days,
            advanced_days,
            target_seasons: target_seasons.max(1),
            seasons_completed,
            snapshots,
            seen_offers,
            last_action_result: None,
            user_team_id,
            manager_firings,
            match_stops: false,
            stopped_match: None,
            next_checkpoint: 0,
            pending_user_fixture: None,
        }
    }

    /// Game days advanced so far (used for per-season snapshots).
    pub fn advanced_days(&self) -> u64 {
        self.advanced_days
    }

    // Checkpoint support: expose the resume-relevant state so the MCP layer
    // can persist/restore an interrupted episode (any horizon, any track).

    pub fn checkpoint_fields(&self) -> (
        u64, u32, u32, u64, u64, i64, i64,
        Vec<crate::run::SeasonSnapshot>, std::collections::HashSet<String>, u32, String, bool,
    ) {
        (
            self.horizon_days, self.target_seasons, self.seasons_completed,
            self.advanced_days, self.step, self.initial_net_worth, self.net_spend,
            self.snapshots.clone(), self.seen_offers.clone(), self.manager_firings,
            self.user_team_id.clone(), self.match_stops,
        )
    }

    /// Per-season metric snapshots recorded during the episode (Dynasty curve).
    pub fn season_snapshots(&self) -> Vec<crate::run::SeasonSnapshot> {
        self.snapshots.clone()
    }

    pub fn step_count(&self) -> u64 {
        self.step
    }

    /// How many times the sim's board tried to fire the manager mid-episode
    /// (each one was overridden by re-hiring — a "would have been fired"
    /// signal, not yet part of the scored metrics).
    pub fn manager_firings(&self) -> u32 {
        self.manager_firings
    }

    /// True when the current league season is complete (standings final) — the
    /// natural season boundary for snapshots and multi-season rollover.
    pub fn season_complete(&self) -> bool {
        ofm_core::end_of_season::is_season_complete(&self.game)
    }

    /// Final evaluation metrics for the current game state (sport/finance/squad).
    pub fn final_metrics(&self) -> crate::run::ClubMetrics {
        crate::run::metrics_of(&self.game, self.initial_net_worth, self.net_spend)
    }

    /// Remap every club/player to an anonymous synthetic identity (the main
    /// benchmark uses this so agents can't exploit pretrained football names).
    pub fn anonymize_identities(&mut self) {
        env::anonymize_identities(&mut self.game);
    }

    pub fn observe(&mut self) -> EpisodeObservation {
        let user_team_id = self.game.manager.team_id.as_deref().unwrap_or_default();
        let team = self.game.teams.iter().find(|t| t.id == user_team_id);
        let today = self.game.clock.current_date.format("%Y-%m-%d").to_string();

        // Mark the offers we're about to show as seen, so `Continue` doesn't
        // make the env re-stop on the same offers forever.
        let offers = self.pending_offers();
        for o in &offers {
            self.seen_offers.insert(o.offer_id.clone());
        }

        let is_matchday = self.user_matchday();
        let next_fixture = self
            .game
            .competitions
            .iter()
            .find_map(|league| {
                league
                    .fixtures
                    .iter()
                    .find(|f| {
                        f.status == FixtureStatus::Scheduled
                            && (f.home_team_id == user_team_id || f.away_team_id == user_team_id)
                    })
            })
            .map(|f| {
                let home = self
                    .game
                    .teams
                    .iter()
                    .find(|t| t.id == f.home_team_id)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                let away = self
                    .game
                    .teams
                    .iter()
                    .find(|t| t.id == f.away_team_id)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                format!(
                    "{} {} vs {} {}",
                    f.date,
                    home,
                    away,
                    if f.home_team_id == user_team_id { "(H)" } else { "(A)" }
                )
            });

        let (league_position, points) = self
            .game
            .league
            .as_ref()
            .map(|league| {
                let mut st = league.standings.clone();
                st.sort_by(|a, b| {
                    b.points
                        .cmp(&a.points)
                        .then_with(|| b.goal_difference().cmp(&a.goal_difference()))
                });
                let pos = st
                    .iter()
                    .position(|s| s.team_id == user_team_id)
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let pts = st
                    .iter()
                    .find(|s| s.team_id == user_team_id)
                    .map(|s| s.points)
                    .unwrap_or(0);
                (pos, pts)
            })
            .unwrap_or((0, 0));

        EpisodeObservation {
            step: self.step,
            date: today,
            team_name: team.map(|t| t.name.clone()).unwrap_or_default(),
            formation: team.map(|t| t.formation.clone()).unwrap_or_else(|| "4-4-2".into()),
            league_position,
            points,
            budget: team.map(|t| t.transfer_budget).unwrap_or(0),
            is_matchday,
            next_fixture,
            squad: self.squad_view(user_team_id),
            offers: if self.mode == env::AgentMode::Manager { offers } else { Vec::new() },
            market: if matches!(self.mode, env::AgentMode::Recruiter | env::AgentMode::Manager) { self.market_view(user_team_id) } else { Vec::new() },
            scout_reports: if matches!(self.mode, env::AgentMode::Recruiter | env::AgentMode::Manager) { self.scout_report_views() } else { Vec::new() },
            scouting_in_progress: if matches!(self.mode, env::AgentMode::Recruiter | env::AgentMode::Manager) { self.scouting_views() } else { Vec::new() },
            transfer_window_open: transfers::transfer_window_is_open(&self.game),
            live_match: self.live_match_view(),
            last_action_result: self.last_action_result.clone(),
            // The episode ends when the target number of seasons has
            // completed (standings final) or the horizon cap is hit. Ending
            // at season completion avoids the "post-season drift" problem —
            // running ~5 months past the last match would bake contract
            // expiries / post-season transfers into the end-state metrics.
            done: self.advanced_days >= self.horizon_days
                || (ofm_core::end_of_season::is_season_complete(&self.game)
                    && self.seasons_completed + 1 >= self.target_seasons),
        }
    }

    /// Apply one action and advance to the next decision point.
    pub fn step(&mut self, action: Action) -> EpisodeObservation {
        self.step += 1;
        self.apply(action);
        // Every action addresses the current decision point, so the world
        // always moves forward at least one day (which plays any match today).
        if self.stopped_match.is_some() {
            self.advance_match();
        } else {
            self.advance_to_next_decision();
        }
        self.observe()
    }

    /// In-match advancement: step the live match to the next checkpoint and
    /// present it, or — once the agent has acted at the last checkpoint (or
    /// the match finished early) — run to completion and write the result
    /// back into the season.
    fn advance_match(&mut self) {
        let finished = self
            .stopped_match
            .as_ref()
            .map(|sm| sm.is_finished())
            .unwrap_or(true);
        if !finished && self.next_checkpoint < Self::MATCH_CHECKPOINTS.len() {
            let sm = self.stopped_match.as_mut().expect("in-match step");
            sm.step_to(Self::MATCH_CHECKPOINTS[self.next_checkpoint]);
            self.next_checkpoint += 1;
            return; // present the new checkpoint observation
        }
        let sm = self.stopped_match.take().expect("session present");
        let _captures = ofm_core::turn::apply_finished_live_match(&mut self.game, sm);
        // The matchday itself was already processed (fixture skipped);
        // resume the ordinary decision cadence for the days after it.
        self.advance_to_next_decision();
    }

    /// Composition-Ladder gate: which actions the current track may perform.
    fn mode_allows(&self, action: &Action) -> bool {
        use env::AgentMode::*;
        match self.mode {
            Coach => matches!(action,
                Action::Continue | Action::SetLineup { .. }
                | Action::SetTactics { .. } | Action::SetMatchPlan { .. }
                | Action::Substitute { .. } | Action::MatchTactics { .. }),
            Recruiter => !matches!(action,
                Action::AcceptOffer { .. } | Action::RejectOffer { .. }
                | Action::CounterOffer { .. } | Action::ListPlayer { .. }),
            Manager => true,
        }
    }

    fn apply(&mut self, action: Action) {
        let friendly = |e: &str| e.trim_start_matches("be.error.").to_string();
        if self.stopped_match.is_some()
            && !matches!(
                action,
                Action::Continue | Action::Substitute { .. } | Action::MatchTactics { .. }
            )
        {
            self.last_action_result = Some(
                "In-match stop: only Substitute, MatchTactics or Continue are available."
                    .into(),
            );
            return;
        }
        if !self.mode_allows(&action) {
            let name = format!("{:?}", std::mem::discriminant(&action));
            self.last_action_result = Some(format!(
                "Action {name} is not available in this track (responsibility scope is locked)."
            ));
            return;
        }
        match action {
            Action::Continue => {
                self.last_action_result = Some("Continued to the next decision point (no intervention; the AI default applies where relevant).".into());
            }
            Action::SetLineup { player_ids } => {
                let valid = self.valid_squad_count(&player_ids);
                env::apply_lineup(&mut self.game, &player_ids);
                self.last_action_result = Some(format!(
                    "Set lineup ({} of {} provided ids are in your squad) for the next match.",
                    valid,
                    player_ids.len()
                ));
            }
            Action::SetTactics { play_style } => {
                env::apply_play_style(&mut self.game, play_style.clone());
                self.last_action_result = Some(format!("Set match tactics to {:?}.", play_style));
            }
            Action::SetMatchPlan { player_ids, play_style } => {
                let valid = self.valid_squad_count(&player_ids);
                env::apply_lineup(&mut self.game, &player_ids);
                env::apply_play_style(&mut self.game, play_style.clone());
                self.last_action_result = Some(format!(
                    "Set lineup ({} of {} provided ids are in your squad) and {:?} tactics for the next match.",
                    valid,
                    player_ids.len(),
                    play_style
                ));
            }
            Action::AcceptOffer { player_id, offer_id } => {
                // Record the sale income before the offer is consumed.
                let income = self
                    .game
                    .players
                    .iter()
                    .find(|p| p.id == player_id)
                    .and_then(|p| p.transfer_offers.iter().find(|o| o.id == offer_id))
                    .map(|o| o.fee as i64)
                    .unwrap_or(0);
                let name = self.player_name(&player_id);
                match transfers::respond_to_offer(&mut self.game, &player_id, &offer_id, true) {
                    Ok(_) => {
                        self.net_spend -= income;
                        self.last_action_result = Some(format!("Accepted the £{} offer for {} (they will leave the club).", income, name));
                    }
                    Err(msg) => self.last_action_result = Some(format!("Could not accept offer for {}: {}.", name, friendly(&msg))),
                }
            }
            Action::RejectOffer { player_id, offer_id } => {
                let name = self.player_name(&player_id);
                match transfers::respond_to_offer(&mut self.game, &player_id, &offer_id, false) {
                    Ok(_) => self.last_action_result = Some(format!("Rejected the offer for {}.", name)),
                    Err(msg) => self.last_action_result = Some(format!("Could not reject offer for {}: {}.", name, friendly(&msg))),
                }
            }
            Action::CounterOffer { player_id, offer_id, fee } => {
                let name = self.player_name(&player_id);
                match transfers::counter_offer(&mut self.game, &player_id, &offer_id, fee) {
                    Ok(_) => self.last_action_result = Some(format!("Countered the offer for {} at £{}.", name, fee)),
                    Err(msg) => self.last_action_result = Some(format!("Could not counter offer for {}: {}.", name, friendly(&msg))),
                }
            }
            Action::MakeBid { player_id, fee } => {
                let name = self.player_name(&player_id);
                match transfers::make_transfer_bid(&mut self.game, &player_id, fee) {
                    Ok(o) => {
                        if o.decision == transfers::TransferNegotiationDecision::Accepted {
                            self.net_spend += fee as i64;
                        }
                        let dec = match o.decision {
                            transfers::TransferNegotiationDecision::Accepted => {
                                format!("ACCEPTED (registers {})", o.registration_date.as_deref().unwrap_or("immediately"))
                            }
                            transfers::TransferNegotiationDecision::Rejected => "REJECTED".into(),
                            transfers::TransferNegotiationDecision::CounterOffer => {
                                format!("countered; suggested £{}", o.suggested_fee.unwrap_or(0))
                            }
                        };
                        self.last_action_result = Some(format!("Bid of £{} for {}: {}.", fee, name, dec));
                    }
                    Err(msg) => self.last_action_result = Some(format!("Bid for {} failed: {}.", name, friendly(&msg))),
                }
            }
            Action::Scout { player_id } => {
                let name = self.player_name(&player_id);
                match self.user_scout_id() {
                    Some(scout_id) => match ofm_core::scouting::send_scout(&mut self.game, &scout_id, &player_id) {
                        Ok(_) => self.last_action_result = Some(format!("Scout sent to assess {} (report will appear in scout_reports).", name)),
                        Err(msg) => self.last_action_result = Some(format!("Could not send scout: {}.", friendly(&msg))),
                    },
                    None => self.last_action_result = Some("No scout available to send.".into()),
                }
            }
            Action::ListPlayer { player_id } => {
                let name = self.player_name(&player_id);
                let ok = self
                    .game
                    .manager
                    .team_id
                    .as_ref()
                    .map(|team_id| {
                        self.game
                            .players
                            .iter_mut()
                            .find(|p| p.id == player_id && p.team_id.as_ref() == Some(team_id))
                            .map(|p| {
                                p.transfer_listed = true;
                            })
                            .is_some()
                    })
                    .unwrap_or(false);
                self.last_action_result = if ok {
                    Some(format!("Transfer-listed {} for sale (incoming offers may follow).", name))
                } else {
                    Some(format!("Could not transfer-list {}: not in your squad.", name))
                };
            }
            Action::Substitute { player_out_id, player_in_id } => {
                let side = self.stopped_match.as_ref().and_then(|sm| sm.user_side);
                let (out_name, in_name) =
                    (self.player_name(&player_out_id), self.player_name(&player_in_id));
                match (side, &mut self.stopped_match) {
                    (Some(side), Some(sm)) => {
                        match sm.apply_command(engine::MatchCommand::Substitute {
                            side,
                            player_off_id: player_out_id,
                            player_on_id: player_in_id,
                        }) {
                            Ok(_) => self.last_action_result =
                                Some(format!("Substituted {in_name} on for {out_name}.")),
                            Err(msg) => self.last_action_result = Some(format!(
                                "Could not substitute {in_name} for {out_name}: {msg}."
                            )),
                        }
                    }
                    _ => self.last_action_result = Some(
                        "No live match in progress — Substitute is only available at in-match stops."
                            .into(),
                    ),
                }
            }
            Action::MatchTactics { play_style, formation } => {
                let side = self.stopped_match.as_ref().and_then(|sm| sm.user_side);
                let style = play_style.as_deref().and_then(|s| match s {
                    "Balanced" => Some(engine::PlayStyle::Balanced),
                    "Attacking" => Some(engine::PlayStyle::Attacking),
                    "Defensive" => Some(engine::PlayStyle::Defensive),
                    "Possession" => Some(engine::PlayStyle::Possession),
                    "Counter" => Some(engine::PlayStyle::Counter),
                    "HighPress" => Some(engine::PlayStyle::HighPress),
                    _ => None,
                });
                match (side, &mut self.stopped_match) {
                    (Some(side), Some(sm)) => {
                        let mut results = Vec::new();
                        if let Some(style) = style {
                            match sm.apply_command(engine::MatchCommand::ChangePlayStyle { side, play_style: style }) {
                                Ok(_) => results.push("play style updated".to_string()),
                                Err(msg) => results.push(format!("play style rejected: {msg}")),
                            }
                        } else if play_style.is_some() {
                            results.push("unknown play style (Balanced/Attacking/Defensive/Possession/Counter/HighPress)".into());
                        }
                        if let Some(formation) = formation {
                            match sm.apply_command(engine::MatchCommand::ChangeFormation { side, formation }) {
                                Ok(_) => results.push("formation updated".to_string()),
                                Err(msg) => results.push(format!("formation rejected: {msg}")),
                            }
                        }
                        self.last_action_result = Some(format!("Match tactics: {}.", results.join("; ")));
                    }
                    _ => self.last_action_result = Some(
                        "No live match in progress — MatchTactics is only available at in-match stops."
                            .into(),
                    ),
                }
            }
        }
    }

    fn player_name(&self, player_id: &str) -> String {
        self.game
            .players
            .iter()
            .find(|p| p.id == player_id)
            .map(|p| p.match_name.clone())
            .unwrap_or_else(|| "unknown player".into())
    }

    /// How many of the given ids are actually in the managed squad. Invalid or
    /// foreign ids are silently ignored by the lineup engine, so the agent
    /// should be told the real count rather than assume all ids were accepted.
    fn valid_squad_count(&self, ids: &[String]) -> usize {
        let team_id = self.game.manager.team_id.as_deref();
        ids.iter()
            .filter(|id| {
                self.game.players.iter().any(|p| {
                    &p.id == *id && team_id.is_some() && p.team_id.as_deref() == team_id
                })
            })
            .count()
    }

    /// Move the world forward one day (expiring stale offers, running the turn
    /// loop, and playing any user matchday via the XI-aware engine). Records a
    /// per-season metrics snapshot whenever a season boundary is crossed.
    fn advance_one_day(&mut self) {
        transfers::expire_stale_transfer_offers(&mut self.game);
        if self.match_stops && self.user_matchday() && self.stopped_match.is_none() {
            // L1 matchday: process the day with the user's fixture left
            // unplayed — the stopped-match flow plays it at the next decision.
            match self.resolve_user_fixture() {
                Some((competition_index, fixture_index)) => {
                    ofm_core::turn::process_day_skipping_fixture(
                        &mut self.game,
                        competition_index,
                        fixture_index,
                    );
                    self.pending_user_fixture = Some((competition_index, fixture_index));
                }
                None => ofm_core::turn::process_day(&mut self.game),
            }
        } else {
            ofm_core::turn::process_day(&mut self.game);
        }
        // The sim may fire the manager for bad results; in the benchmark the
        // board never fires the agent mid-episode, so re-hire (and count).
        if self.game.manager.team_id.is_none() && !self.user_team_id.is_empty() {
            self.game.manager.hire(self.user_team_id.clone());
            self.manager_firings += 1;
        }
        self.advanced_days += 1;
        // Season snapshots are recorded at REAL season boundaries in
        // advance_to_next_decision (the old 365-day cadence misaligned with
        // the actual ~283-day season and never fired for MCP episodes).
    }

    /// A transfer-window "market day": the agent gets a chance to scout and bid
    /// roughly every three days while the window is open.
    fn market_day(&self) -> bool {
        transfers::transfer_window_is_open(&self.game) && self.advanced_days % 3 == 0
    }

    /// Pending offers the agent has never been shown (never-seen ids).
    fn fresh_offers(&self) -> Vec<OfferView> {
        self.pending_offers()
            .into_iter()
            .filter(|o| !self.seen_offers.contains(&o.offer_id))
            .collect()
    }

    /// Advance until the next decision point (a user matchday, fresh pending
    /// offers, a transfer-window market day, season completion, or the
    /// horizon), always moving forward at least one day first.
    fn advance_to_next_decision(&mut self) {
        self.advance_one_day();
        let mut guard = 0u32;
        loop {
            guard += 1;
            if guard > 10_000 {
                break; // safety: a rollover that never clears season state
            }
            if self.stopped_match.is_some() {
                break; // in-match checkpoint — observe the live state
            }
            if self.pending_user_fixture.is_some() {
                self.begin_stopped_match();
                break;
            }
            if self.advanced_days >= self.horizon_days {
                break;
            }
            // Stop the instant the league season completes — otherwise the
            // final step would keep advancing to the next market day (weeks
            // after the last match) and bake post-season drift into the
            // end-state metrics. For multi-season runs the boundary is a
            // rollover instead: snapshot the finished season and regenerate
            // the next one (squad, finances and clock carry over).
            if ofm_core::end_of_season::is_season_complete(&self.game) {
                let m = crate::run::metrics_of(&self.game, self.initial_net_worth, self.net_spend);
                self.snapshots.push(crate::run::SeasonSnapshot {
                    season: self.seasons_completed + 1,
                    points: m.points,
                    position: m.position,
                    balance: m.balance,
                    squad_value: m.squad_value,
                    avg_age: m.avg_age,
                    squad_size: m.squad_size,
                    net_value: m.net_value,
                    net_spend: m.net_spend,
                });
                if self.seasons_completed + 1 >= self.target_seasons {
                    break;
                }
                env::rollover_season(&mut self.game);
                self.seasons_completed += 1;
                continue;
            }
            if self.user_matchday() {
                break; // user matchday — lineup/tactics decision
            }
            if self.mode == env::AgentMode::Manager && !self.fresh_offers().is_empty() {
                break; // fresh transfer offer — accept/reject/counter decision
            }
            if matches!(self.mode, env::AgentMode::Recruiter | env::AgentMode::Manager)
                && self.market_day()
            {
                break; // transfer-window market — scout/bid decision
            }
            self.advance_one_day();
        }
    }

    /// Does the user's club have a scheduled match today (any competition)?
    fn user_matchday(&self) -> bool {
        let today = self.game.clock.current_date.format("%Y-%m-%d").to_string();
        self.game.user_has_scheduled_match_on(&today)
    }

    /// L1 match checkpoints (game minutes). The agent decides at each stop;
    /// after the last one the match runs to completion.
    const MATCH_CHECKPOINTS: [u8; 4] = [30, 45, 60, 75];

    /// Locate today's scheduled user fixture across all competitions, as
    /// (competition_index, fixture_index) — the same resolution the GUI's
    /// live-match start uses.
    fn resolve_user_fixture(&self) -> Option<(usize, usize)> {
        let today = self.game.clock.current_date.format("%Y-%m-%d").to_string();
        let team_id = self.game.manager.team_id.as_deref()?;
        for (competition_index, competition) in self.game.competitions.iter().enumerate() {
            for (fixture_index, fixture) in competition.fixtures.iter().enumerate() {
                if fixture.date == today
                    && fixture.status == domain::league::FixtureStatus::Scheduled
                    && (fixture.home_team_id == team_id || fixture.away_team_id == team_id)
                {
                    return Some((competition_index, fixture_index));
                }
            }
        }
        None
    }

    /// Create the live-match session for the pending user fixture, swap its
    /// competition into the legacy mirror (create_live_match reads it), and
    /// step to the first checkpoint.
    fn begin_stopped_match(&mut self) {
        let (competition_index, fixture_index) =
            self.pending_user_fixture.take().expect("pending fixture set");
        if let Some(competition) = self.game.competitions.get(competition_index).cloned() {
            self.game.league = Some(competition);
        }
        // Match-keyed sub-stream: same draw source the whole-match engine
        // path uses, so both arms stay deterministic under (seed, fixture).
        ofm_core::rng::set_domain("match", &fixture_index.to_le_bytes());
        match ofm_core::live_match_manager::create_live_match(
            &self.game,
            fixture_index,
            ofm_core::live_match_manager::MatchMode::Live,
            false,
        ) {
            Ok(mut session) => {
                session.step_to(Self::MATCH_CHECKPOINTS[0]);
                self.next_checkpoint = 1;
                self.stopped_match = Some(session);
                self.last_action_result = Some(format!(
                    "Kickoff — live match control enabled (stops at 30'/HT/60'/75')."
                ));
            }
            Err(msg) => {
                self.last_action_result = Some(format!("Could not start live match: {msg}."));
                // The fixture stays unplayed this round; the season continues.
            }
        }
    }

    /// Build the live-match observation view from the session snapshot.
    fn live_match_view(&self) -> Option<LiveMatchView> {
        let sm = self.stopped_match.as_ref()?;
        let snap = sm.snapshot();
        let user_side = sm.user_side?;
        let (user_team, user_bench, subs_made) = match user_side {
            engine::Side::Home => (&snap.home_team, &snap.home_bench, snap.home_subs_made),
            engine::Side::Away => (&snap.away_team, &snap.away_bench, snap.away_subs_made),
        };
        let to_view = |p: &engine::PlayerData| LiveMatchPlayer {
            id: p.id.clone(),
            name: p.name.clone(),
            position: format!("{:?}", p.position),
            condition: p.condition,
        };
        Some(LiveMatchView {
            minute: snap.current_minute,
            phase: format!("{:?}", snap.phase),
            home_score: snap.home_score,
            away_score: snap.away_score,
            user_side: format!("{user_side:?}").to_lowercase(),
            field: user_team.players.iter().map(to_view).collect(),
            bench: user_bench.iter().map(to_view).collect(),
            subs_made,
            subs_max: snap.max_subs,
            events: snap
                .events
                .iter()
                .map(|e| format!("{:?}", e))
                .collect(),
        })
    }

    fn squad_view(&self, team_id: &str) -> Vec<env::PlayerView> {
        self.game
            .players
            .iter()
            .filter(|p| p.team_id.as_deref() == Some(team_id))
            .map(|p| env::PlayerView {
                id: p.id.clone(),
                name: p.match_name.clone(),
                position: p.position.clone(),
                group_position: p.position.to_group_position(),
                ovr: p.ovr,
                age: age_from_dob(&p.date_of_birth, &self.game.clock.current_date.format("%Y-%m-%d").to_string()),
                condition: p.condition,
                fitness: p.fitness,
                morale: p.morale,
                injured: p.injury.is_some(),
                transfer_listed: p.transfer_listed,
                wage: p.wage,
                market_value: p.market_value,
            })
            .collect()
    }

    fn pending_offers(&self) -> Vec<OfferView> {
        let team_id = self.game.manager.team_id.as_deref().unwrap_or_default();
        let mut out = Vec::new();
        for player in self.game.players.iter().filter(|p| p.team_id.as_deref() == Some(team_id)) {
            for offer in &player.transfer_offers {
                if offer.status != TransferOfferStatus::Pending {
                    continue;
                }
                let from_team = self
                    .game
                    .teams
                    .iter()
                    .find(|t| t.id == offer.from_team_id)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                out.push(OfferView {
                    offer_id: offer.id.clone(),
                    player_id: player.id.clone(),
                    player_name: player.match_name.clone(),
                    from_team,
                    fee: offer.fee,
                    round: offer.negotiation_round,
                    suggested_counter: offer.suggested_counter_fee,
                });
            }
        }
        out
    }

    fn market_view(&self, own_team_id: &str) -> Vec<MarketView> {
        // Scout reports keyed by player id, for the market view.
        let reports: std::collections::HashMap<&str, &domain::message::ScoutReportData> = self
            .game
            .messages
            .iter()
            .filter_map(|m| m.context.scout_report.as_ref())
            .map(|r| (r.player_id.as_str(), r))
            .collect();
        let scouting: std::collections::HashSet<&str> = self
            .game
            .scouting_assignments
            .iter()
            .map(|a| a.player_id.as_str())
            .collect();

        // Market = a scouted shortlist of targets outside the squad: the most
        // valuable players in the world (transfer-listed or not). Ratings are
        // hidden until scouted (partial observability).
        let mut out: Vec<MarketView> = self
            .game
            .players
            .iter()
            .filter(|p| p.team_id.as_deref() != Some(own_team_id) && p.injury.is_none())
            .map(|p| {
                let report = reports.get(p.id.as_str());
                MarketView {
                    player_id: p.id.clone(),
                    player_name: p.match_name.clone(),
                    position: p.position.clone(),
                    age: age_from_dob(&p.date_of_birth, &self.game.clock.current_date.format("%Y-%m-%d").to_string()),
                    market_value: p.market_value,
                    team: self
                        .game
                        .teams
                        .iter()
                        .find(|t| Some(&t.id) == p.team_id.as_ref())
                        .map(|t| t.name.clone())
                        .unwrap_or_default(),
                    reported_ovr: report.and_then(|r| r.avg_rating).map(|v| v as u8),
                    potential: report.map(|r| r.potential_key.clone()),
                    scouting: scouting.contains(p.id.as_str()),
                }
            })
            .collect();
        // The market shortlist = the most valuable targets outside the squad,
        // so unscouted high-value players remain visible and scoutable.
        out.sort_by(|a, b| b.market_value.cmp(&a.market_value));
        out.truncate(30);
        out
    }

    /// Completed scout reports pulled from the game's message inbox.
    fn scout_report_views(&self) -> Vec<ScoutReportView> {
        self.game
            .messages
            .iter()
            .filter_map(|m| m.context.scout_report.as_ref())
            .map(|r| {
                let team = r
                    .team_name
                    .clone()
                    .or_else(|| {
                        self.game.players.iter().find(|p| p.id == r.player_id)
                            .and_then(|p| p.team_id.as_ref())
                            .and_then(|tid| self.game.teams.iter().find(|t| &t.id == tid))
                            .map(|t| t.name.clone())
                    })
                    .unwrap_or_default();
                ScoutReportView {
                    player_id: r.player_id.clone(),
                    player_name: r.player_name.clone(),
                    team,
                    avg_rating: r.avg_rating,
                    rating_desc: r.rating_key.clone(),
                    potential: r.potential_key.clone(),
                    confidence: r.confidence_key.clone(),
                }
            })
            .collect()
    }

    /// Scouting assignments in progress.
    fn scouting_views(&self) -> Vec<ScoutingView> {
        self.game
            .scouting_assignments
            .iter()
            .map(|a| ScoutingView {
                player_id: a.player_id.clone(),
                player_name: self
                    .game
                    .players
                    .iter()
                    .find(|p| p.id == a.player_id)
                    .map(|p| p.match_name.clone())
                    .unwrap_or_default(),
                days_remaining: a.days_remaining,
            })
            .collect()
    }

    /// The user's first scout (needed by `send_scout`).
    fn user_scout_id(&self) -> Option<String> {
        let user_team_id = self.game.manager.team_id.as_deref()?;
        self.game
            .staff
            .iter()
            .find(|s| s.team_id.as_deref() == Some(user_team_id) && s.role == domain::staff::StaffRole::Scout)
            .map(|s| s.id.clone())
    }
}

fn age_from_dob(dob: &str, today: &str) -> u8 {
    let Ok(b) = chrono::NaiveDate::parse_from_str(dob, "%Y-%m-%d") else {
        return 0;
    };
    let Ok(t) = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d") else {
        return 0;
    };
    ((t - b).num_days() / 365) as u8
}
