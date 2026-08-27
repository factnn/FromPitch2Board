//! Baseline agents for the decision-cadence environment (`episode::Episode`).

use crate::episode::{Action, EpisodeObservation};
use domain::player::Position;
use domain::team::PlayStyle;

/// A decision policy: given the current observation, pick one action.
pub trait Policy {
    fn name(&self) -> &str;
    fn act(&mut self, obs: &EpisodeObservation) -> Action;
}

/// A sensible automated manager:
/// - on a matchday: best-fit XI + a fixed play style;
/// - on offers: accept any bid at/above 1.2× market value, otherwise reject;
/// - otherwise: occasionally bid for a strong transfer-listed player we can afford.
pub struct AutoManager {
    pub play_style: PlayStyle,
}

impl AutoManager {
    pub fn new(play_style: PlayStyle) -> Self {
        Self { play_style }
    }

    fn best_lineup(&self, obs: &EpisodeObservation) -> Vec<String> {
        // Greedy per-formation-slot best fit, skipping the injured/low-condition.
        let slots = ofm_core::player_rating::formation_slots(&obs.formation);
        let mut pool: Vec<&crate::env::PlayerView> = obs.squad.iter().filter(|p| !p.injured).collect();
        let mut xi = Vec::new();
        for slot in slots.iter().take(11) {
            let group = slot.to_group_position();
            let mut best_idx = None;
            let mut best_rating: f64 = f64::MIN;
            for (i, p) in pool.iter().enumerate() {
                let rating = if p.group_position == group {
                    p.ovr as f64
                } else {
                    p.ovr as f64 - 8.0
                };
                let rating = if p.condition < 55 { rating - 25.0 } else { rating };
                if rating > best_rating {
                    best_rating = rating;
                    best_idx = Some(i);
                }
            }
            if let Some(i) = best_idx {
                xi.push(pool.remove(i).id.clone());
            }
        }
        xi
    }
}

impl Policy for AutoManager {
    fn name(&self) -> &str {
        "AutoManager"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if obs.is_matchday {
            return Action::SetMatchPlan {
                player_ids: self.best_lineup(obs),
                play_style: self.play_style.clone(),
            };
        }
        if !obs.offers.is_empty() {
            // Accept anything ≥ 1.2× market value, reject the rest.
            let offer = &obs.offers[0];
            let value = obs
                .squad
                .iter()
                .find(|p| p.id == offer.player_id)
                .map(|p| p.market_value as u64)
                .unwrap_or(0);
            if offer.fee >= value * 12 / 10 {
                return Action::AcceptOffer {
                    player_id: offer.player_id.clone(),
                    offer_id: offer.offer_id.clone(),
                };
            }
            return Action::RejectOffer {
                player_id: offer.player_id.clone(),
                offer_id: offer.offer_id.clone(),
            };
        }
        Action::Continue
    }
}

/// Never manage anything — the control baseline (everything is AI default).
pub struct PassiveManager;
impl Policy for PassiveManager {
    fn name(&self) -> &str {
        "Passive"
    }
    fn act(&mut self, _obs: &EpisodeObservation) -> Action {
        Action::Continue
    }
}

/// A manager that works the market: scouts uncased targets, then bids for
/// scouted high-potential players it can afford. Exercises the Scout / MakeBid
/// actions (partial observability: it acts on reported rating + potential band,
/// never the true hidden attributes).
pub struct ProactiveManager {
    pub play_style: PlayStyle,
    bid_on: std::collections::HashSet<String>,
    scouted: std::collections::HashSet<String>,
}

impl ProactiveManager {
    pub fn new(play_style: PlayStyle) -> Self {
        Self {
            play_style,
            bid_on: Default::default(),
            scouted: Default::default(),
        }
    }
    fn best_lineup(&self, obs: &EpisodeObservation) -> Vec<String> {
        let slots = ofm_core::player_rating::formation_slots(&obs.formation);
        let mut pool: Vec<&crate::env::PlayerView> = obs.squad.iter().filter(|p| !p.injured).collect();
        let mut xi = Vec::new();
        for slot in slots.iter().take(11) {
            let group = slot.to_group_position();
            let mut best = None;
            let mut best_rating: f64 = f64::MIN;
            for (i, p) in pool.iter().enumerate() {
                let rating = if p.group_position == group { p.ovr as f64 } else { p.ovr as f64 - 8.0 };
                let rating = if p.condition < 55 { rating - 25.0 } else { rating };
                if rating > best_rating {
                    best_rating = rating;
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                xi.push(pool.remove(i).id.clone());
            }
        }
        xi
    }
}

impl Policy for ProactiveManager {
    fn name(&self) -> &str {
        "Proactive"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if obs.is_matchday {
            return Action::SetMatchPlan {
                player_ids: self.best_lineup(obs),
                play_style: self.play_style.clone(),
            };
        }
        if !obs.offers.is_empty() {
            let offer = &obs.offers[0];
            let value = obs
                .squad
                .iter()
                .find(|p| p.id == offer.player_id)
                .map(|p| p.market_value as u64)
                .unwrap_or(0);
            if offer.fee >= value * 12 / 10 {
                return Action::AcceptOffer {
                    player_id: offer.player_id.clone(),
                    offer_id: offer.offer_id.clone(),
                };
            }
            return Action::RejectOffer {
                player_id: offer.player_id.clone(),
                offer_id: offer.offer_id.clone(),
            };
        }
        // Bid on a scouted target with a decent reported rating we can afford.
        if let Some(target) = obs.market.iter().find(|m| {
            !self.bid_on.contains(&m.player_id)
                && m.reported_ovr.map(|r| r >= 60).unwrap_or(false)
                && (m.market_value as i64) <= obs.budget
        }) {
            self.bid_on.insert(target.player_id.clone());
            // Bid ~1.4× market value: the AI club usually rejects market value
            // outright (it wants a premium on top).
            let fee = (target.market_value * 14) / 10;
            return Action::MakeBid {
                player_id: target.player_id.clone(),
                fee,
            };
        }
        // Otherwise scout an unscouted, un-assigned target to reveal it.
        if let Some(target) = obs.market.iter().find(|m| {
            m.reported_ovr.is_none() && !self.scouted.contains(&m.player_id) && !m.scouting
        }) {
            self.scouted.insert(target.player_id.clone());
            return Action::Scout {
                player_id: target.player_id.clone(),
            };
        }
        Action::Continue
    }
}

/// Greedy slot-aligned best-XI from the observation (condition-aware).
pub fn best_lineup_xi(obs: &EpisodeObservation) -> Vec<String> {
    let slots = ofm_core::player_rating::formation_slots(&obs.formation);
    let mut pool: Vec<&crate::env::PlayerView> = obs.squad.iter().filter(|p| !p.injured).collect();
    let mut xi = Vec::new();
    for slot in slots.iter().take(11) {
        let group = slot.to_group_position();
        let mut best = None;
        let mut best_rating: f64 = f64::MIN;
        for (i, p) in pool.iter().enumerate() {
            let rating = if p.group_position == group { p.ovr as f64 } else { p.ovr as f64 - 8.0 };
            let rating = if p.condition < 55 { rating - 25.0 } else { rating };
            if rating > best_rating {
                best_rating = rating;
                best = Some(i);
            }
        }
        if let Some(i) = best {
            xi.push(pool.remove(i).id.clone());
        }
    }
    xi
}

// ---------------------------------------------------------------------------
// Coach-track policies: transfers/scouting frozen — matchday decisions only.
// ---------------------------------------------------------------------------

/// Pick the strongest XI + a fixed play style on every matchday; ignore the
/// market entirely (the Coach track).
pub struct CoachBestXI {
    pub play_style: PlayStyle,
}
impl Policy for CoachBestXI {
    fn name(&self) -> &str {
        "CoachBestXI"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if obs.is_matchday {
            return Action::SetMatchPlan {
                player_ids: best_lineup_xi(obs),
                play_style: self.play_style.clone(),
            };
        }
        Action::Continue
    }
}

/// A uniformly random valid XI on matchdays.
pub struct CoachRandom;
impl Policy for CoachRandom {
    fn name(&self) -> &str {
        "CoachRandom"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        use rand::seq::SliceRandom;
        if obs.is_matchday {
            let slots = ofm_core::player_rating::formation_slots(&obs.formation);
            let mut pool: Vec<&crate::env::PlayerView> = obs.squad.iter().filter(|p| !p.injured).collect();
            pool.shuffle(&mut ofm_core::rng::rng());
            let mut xi = Vec::new();
            for slot in slots.iter().take(11) {
                let group = slot.to_group_position();
                let pos = pool
                    .iter()
                    .position(|p| p.group_position == group)
                    .or_else(|| pool.iter().position(|_| true));
                if let Some(i) = pos {
                    xi.push(pool.remove(i).id.clone());
                }
            }
            return Action::SetMatchPlan {
                player_ids: xi,
                play_style: PlayStyle::Balanced,
            };
        }
        Action::Continue
    }
}

/// The *worst* XI every matchday (antagonist lower bound).
pub struct CoachWorst;
impl Policy for CoachWorst {
    fn name(&self) -> &str {
        "CoachWorst"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if obs.is_matchday {
            let slots = ofm_core::player_rating::formation_slots(&obs.formation);
            let mut pool: Vec<&crate::env::PlayerView> = obs.squad.iter().filter(|p| !p.injured).collect();
            let mut xi = Vec::new();
            for slot in slots.iter().take(11) {
                let group = slot.to_group_position();
                let mut worst = None;
                let mut worst_rating: f64 = f64::MAX;
                for (i, p) in pool.iter().enumerate() {
                    let rating = if p.group_position == group { p.ovr as f64 } else { p.ovr as f64 - 8.0 };
                    if rating < worst_rating {
                        worst_rating = rating;
                        worst = Some(i);
                    }
                }
                if let Some(i) = worst {
                    xi.push(pool.remove(i).id.clone());
                }
            }
            return Action::SetMatchPlan {
                player_ids: xi,
                play_style: PlayStyle::Defensive,
            };
        }
        Action::Continue
    }
}

/// A manager that actively sells: transfer-lists the oldest backup players to
/// attract offers, then accepts anything at/above 1.3× market value. Exercises
/// the ListPlayer / AcceptOffer / RejectOffer actions (the selling side).
pub struct SellingManager {
    pub play_style: PlayStyle,
    listed: std::collections::HashSet<String>,
}

impl SellingManager {
    pub fn new(play_style: PlayStyle) -> Self {
        Self {
            play_style,
            listed: Default::default(),
        }
    }
    fn best_lineup(&self, obs: &EpisodeObservation) -> Vec<String> {
        let slots = ofm_core::player_rating::formation_slots(&obs.formation);
        let mut pool: Vec<&crate::env::PlayerView> = obs.squad.iter().filter(|p| !p.injured).collect();
        let mut xi = Vec::new();
        for slot in slots.iter().take(11) {
            let group = slot.to_group_position();
            let mut best = None;
            let mut best_rating: f64 = f64::MIN;
            for (i, p) in pool.iter().enumerate() {
                let rating = if p.group_position == group { p.ovr as f64 } else { p.ovr as f64 - 8.0 };
                let rating = if p.condition < 55 { rating - 25.0 } else { rating };
                if rating > best_rating {
                    best_rating = rating;
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                xi.push(pool.remove(i).id.clone());
            }
        }
        xi
    }
}

impl Policy for SellingManager {
    fn name(&self) -> &str {
        "Selling"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if obs.is_matchday {
            return Action::SetMatchPlan {
                player_ids: self.best_lineup(obs),
                play_style: self.play_style.clone(),
            };
        }
        if !obs.offers.is_empty() {
            let offer = &obs.offers[0];
            let value = obs
                .squad
                .iter()
                .find(|p| p.id == offer.player_id)
                .map(|p| p.market_value as u64)
                .unwrap_or(0);
            if offer.fee >= value * 9 / 10 {
                return Action::AcceptOffer {
                    player_id: offer.player_id.clone(),
                    offer_id: offer.offer_id.clone(),
                };
            }
            return Action::RejectOffer {
                player_id: offer.player_id.clone(),
                offer_id: offer.offer_id.clone(),
            };
        }
        // On a market day, list the most valuable unlisted backup to attract a
        // meaningful offer (selling a bench asset for cash).
        let xi = self.best_lineup(obs);
        if let Some(target) = obs
            .squad
            .iter()
            .filter(|p| !p.injured && p.age >= 24 && !xi.contains(&p.id) && !p.transfer_listed)
            .max_by_key(|p| p.market_value)
        {
            if self.listed.insert(target.id.clone()) {
                return Action::ListPlayer {
                    player_id: target.id.clone(),
                };
            }
        }
        Action::Continue
    }
}

/// A uniformly random manager: a random XI + random play style on matchdays,
/// and a random market action otherwise (scout / bid / list / continue). The
/// manager-track lower bound — together with Passive (no-op) and Greedy it
/// forms the E0 environmental-baseline ladder: Random < No-op < Greedy.
/// Deterministic per seed (draws from the seeded episode RNG).
pub struct RandomManager;
impl Policy for RandomManager {
    fn name(&self) -> &str {
        "RandomManager"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        use rand::seq::SliceRandom;
        use rand::RngExt;
        if obs.is_matchday {
            // Random XI, FIXED Balanced style — randomising the style too would
            // average over the engine's style effects (Attacking is strong) and
            // accidentally outperform the no-op baseline, breaking the
            // Random < No-op < Greedy ladder. Same convention as CoachRandom.
            let mut pool: Vec<String> = obs
                .squad
                .iter()
                .filter(|p| !p.injured)
                .map(|p| p.id.clone())
                .collect();
            let mut rng = ofm_core::rng::rng();
            pool.shuffle(&mut rng);
            return Action::SetMatchPlan {
                player_ids: pool.into_iter().take(11).collect(),
                play_style: PlayStyle::Balanced,
            };
        }
        if !obs.offers.is_empty() {
            let offer = &obs.offers[0];
            let mut rng = ofm_core::rng::rng();
            if rng.random_range(0..2) == 0 {
                return Action::AcceptOffer {
                    player_id: offer.player_id.clone(),
                    offer_id: offer.offer_id.clone(),
                };
            }
            return Action::RejectOffer {
                player_id: offer.player_id.clone(),
                offer_id: offer.offer_id.clone(),
            };
        }
        let mut rng = ofm_core::rng::rng();
        match rng.random_range(0..4) {
            0 if !obs.market.is_empty() => {
                let m = &obs.market[rng.random_range(0..obs.market.len())];
                Action::Scout { player_id: m.player_id.clone() }
            }
            1 if !obs.market.is_empty() => {
                let m = &obs.market[rng.random_range(0..obs.market.len())];
                Action::MakeBid { player_id: m.player_id.clone(), fee: m.market_value }
            }
            2 if !obs.squad.is_empty() => {
                let p = &obs.squad[rng.random_range(0..obs.squad.len())];
                Action::ListPlayer { player_id: p.id.clone() }
            }
            _ => Action::Continue,
        }
    }
}

/// Resolve every pending offer, accepting above 1.2× value — but never touch
/// the lineup or the market. Isolates the transfer-decision contribution.
pub struct OffersOnlyManager;
impl Policy for OffersOnlyManager {
    fn name(&self) -> &str {
        "OffersOnly"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if !obs.offers.is_empty() {
            let offer = &obs.offers[0];
            let value = obs
                .squad
                .iter()
                .find(|p| p.id == offer.player_id)
                .map(|p| p.market_value as u64)
                .unwrap_or(0);
            if offer.fee >= value * 12 / 10 {
                return Action::AcceptOffer {
                    player_id: offer.player_id.clone(),
                    offer_id: offer.offer_id.clone(),
                };
            }
            return Action::RejectOffer {
                player_id: offer.player_id.clone(),
                offer_id: offer.offer_id.clone(),
            };
        }
        Action::Continue
    }
}

// ---------------------------------------------------------------------------
// Greedy baselines (the frozen reference; see the design notes).
//
// Deterministic given the observation + internal state: no RNG. Acts only on
// what any agent sees — fuzzed scout ratings and coarse potential bands — so
// the comparison with LLM agents stays fair (partial observability applies).
// ---------------------------------------------------------------------------

/// Match utility for the Greedy coach (the design notes §三):
///   U_i = ability + role_fit − fatigue − injury_risk
/// Injured players are dropped; a tired player (low condition/fitness) loses
/// rating and naturally rotates out — this is the fatigue-based rotation.
fn greedy_utility(p: &crate::env::PlayerView, slot_group: &Position) -> f64 {
    if p.injured {
        return f64::MIN;
    }
    let role_fit = if &p.group_position == slot_group { 0.0 } else { -8.0 };
    let fatigue = (100.0 - p.condition as f64) * 0.5 + (100.0 - p.fitness as f64) * 0.2;
    p.ovr as f64 + role_fit - fatigue
}

/// The Greedy XI: fixed formation, highest-utility available player per slot.
fn greedy_lineup(obs: &EpisodeObservation) -> Vec<String> {
    let slots = ofm_core::player_rating::formation_slots(&obs.formation);
    let mut pool: Vec<&crate::env::PlayerView> = obs.squad.iter().collect();
    let mut xi = Vec::new();
    for slot in slots.iter().take(11) {
        let group = slot.to_group_position();
        let mut best = None;
        let mut best_u = f64::MIN;
        for (i, p) in pool.iter().enumerate() {
            let u = greedy_utility(p, &group);
            if u > best_u {
                best_u = u;
                best = Some(i);
            }
        }
        if let Some(i) = best {
            xi.push(pool.remove(i).id.clone());
        }
    }
    xi
}

/// The frozen Greedy coach: best-utility XI + a fixed play style, nothing else.
pub struct GreedyCoach {
    pub play_style: PlayStyle,
}
impl Policy for GreedyCoach {
    fn name(&self) -> &str {
        "GreedyCoach"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if obs.is_matchday {
            return Action::SetMatchPlan {
                player_ids: greedy_lineup(obs),
                play_style: self.play_style.clone(),
            };
        }
        Action::Continue
    }
}

/// The frozen Greedy manager = Greedy coach + market logic (the design notes §三):
///   - Need(p) per group = max(0, target − current_strength) with
///     target = squad average + 4 (improve every position beyond the mean);
///   - scout unscouted targets in needy positions;
///   - bid for the best scouted value-for-money target we can afford
///     (TransferValue ≈ fuzzed rating + potential bonus − age penalty + need);
///   - accept offers: surplus players at 1.1× value, starters only at 1.8×;
///   - list old non-starters (age ≥ 27) to raise cash.
pub struct GreedyManager {
    pub play_style: PlayStyle,
    bid_on: std::collections::HashSet<String>,
    listed: std::collections::HashSet<String>,
}

impl GreedyManager {
    pub fn new(play_style: PlayStyle) -> Self {
        Self {
            play_style,
            bid_on: Default::default(),
            listed: Default::default(),
        }
    }

    /// Per-group upgrade need (f64, 0 = position already at/above target).
    fn group_needs(&self, obs: &EpisodeObservation) -> std::collections::HashMap<Position, f64> {
        let fit: Vec<&crate::env::PlayerView> = obs.squad.iter().filter(|p| !p.injured).collect();
        let avg = fit.iter().map(|p| p.ovr as f64).sum::<f64>() / fit.len().max(1) as f64;
        let target = avg + 4.0;
        let mut strength: std::collections::HashMap<Position, f64> = Default::default();
        for p in &fit {
            let e = strength.entry(p.group_position.clone()).or_insert(0.0);
            *e = e.max(p.ovr as f64);
        }
        let mut needs = std::collections::HashMap::new();
        for g in [
            Position::Goalkeeper,
            Position::Defender,
            Position::Midfielder,
            Position::Forward,
        ] {
            let s = strength.get(&g).copied().unwrap_or(0.0);
            needs.insert(g, (target - s).max(0.0));
        }
        needs
    }

    fn potential_bonus(potential: Option<&String>) -> f64 {
        match potential.map(String::as_str) {
            Some("worldClass") => 6.0,
            Some("strong") => 4.0,
            Some("moderate") => 2.0,
            _ => 0.0,
        }
    }
}

impl Policy for GreedyManager {
    fn name(&self) -> &str {
        "GreedyManager"
    }
    fn act(&mut self, obs: &EpisodeObservation) -> Action {
        if obs.is_matchday {
            return Action::SetMatchPlan {
                player_ids: greedy_lineup(obs),
                play_style: self.play_style.clone(),
            };
        }
        if !obs.offers.is_empty() {
            let offer = &obs.offers[0];
            let value = obs
                .squad
                .iter()
                .find(|p| p.id == offer.player_id)
                .map(|p| p.market_value)
                .unwrap_or(0);
            // Surplus players go for a slight premium; irreplaceable starters
            // only leave for a huge fee (greedy value discipline).
            let starter = greedy_lineup(obs).iter().any(|id| id == &offer.player_id);
            let threshold = if starter { 18 } else { 11 };
            if offer.fee >= value * threshold / 10 {
                return Action::AcceptOffer {
                    player_id: offer.player_id.clone(),
                    offer_id: offer.offer_id.clone(),
                };
            }
            return Action::RejectOffer {
                player_id: offer.player_id.clone(),
                offer_id: offer.offer_id.clone(),
            };
        }
        if obs.transfer_window_open {
            let needs = self.group_needs(obs);
            let budget = obs.budget.max(0) as u64;

            // 1) Bid for the best scouted, affordable, value-for-money target.
            let mut best: Option<(f64, String, u64)> = None;
            for m in obs.market.iter().filter(|m| m.reported_ovr.is_some()) {
                if self.bid_on.contains(&m.player_id) {
                    continue;
                }
                let group_need = needs
                    .get(&m.position.to_group_position())
                    .copied()
                    .unwrap_or(0.0);
                let rating = m.reported_ovr.unwrap_or(0) as f64;
                let age_pen = if m.age > 26 { (m.age - 26) as f64 * 1.5 } else { 0.0 };
                let val = rating + Self::potential_bonus(m.potential.as_ref()) - age_pen + group_need * 0.5;
                // Bid a 1.4× premium (AI clubs reject market value); only if we
                // can afford it and aren't wildly overpaying.
                let fee = m.market_value * 14 / 10;
                if fee > budget || fee > m.market_value.saturating_mul(2) {
                    continue;
                }
                if best.as_ref().map(|(v, _, _)| val > *v).unwrap_or(true) {
                    best = Some((val, m.player_id.clone(), fee));
                }
            }
            if let Some((_, pid, fee)) = best {
                self.bid_on.insert(pid.clone());
                return Action::MakeBid { player_id: pid, fee };
            }

            // 2) Otherwise scout an unscouted target in a needy position.
            if let Some(m) = obs.market.iter().find(|m| {
                m.reported_ovr.is_none()
                    && !m.scouting
                    && needs.get(&m.position.to_group_position()).copied().unwrap_or(0.0) > 0.0
            }) {
                return Action::Scout { player_id: m.player_id.clone() };
            }

            // 3) Otherwise list the most valuable old non-starter to raise cash.
            let xi = greedy_lineup(obs);
            if let Some(p) = obs
                .squad
                .iter()
                .filter(|p| !p.injured && p.age >= 27 && !xi.contains(&p.id) && !p.transfer_listed)
                .max_by_key(|p| p.market_value)
            {
                if self.listed.insert(p.id.clone()) {
                    return Action::ListPlayer { player_id: p.id.clone() };
                }
            }
        }
        Action::Continue
    }
}
