//! FromPitch2Board MCP server — exposes the headless environment to any MCP-capable
//! agent (Claude Code, Codex, Gemini CLI, …) via standard tools.
//!
//! Tools: `reset` (start an episode), `observe` (current state JSON),
//! `act` (apply one action), `score` (final evaluation metrics).
//!
//! IMPORTANT: the server must run on a **single-threaded** tokio runtime — the
//! environment's determinism relies on a thread-local seeded RNG, which breaks
//! if tools run on different threads.

use std::future::Future;
use std::sync::{Arc, Mutex};

use rmcp::model::{CallToolResult, Implementation, PaginatedRequestParams};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use serde_json::{json, Value};

use crate::env::{AgentMode, ClubPick, ScenarioBudget, WorldSize};
use crate::episode::{Action, Episode};

#[derive(Clone)]
pub struct FromPitch2BoardMcp {
    episode: Arc<Mutex<Option<Episode>>>,
    /// Metadata of the most recent reset, needed to build a TrajectoryRecord
    /// when `dump` is called.
    meta: Arc<Mutex<Option<crate::run::TrajectoryMeta>>>,
    /// Per-step tool-call log for the current episode (action, date, result,
    /// server latency, success). This is the lossless decision trace for
    /// off-the-shelf agents (cc / codex), whose LLM internals we don't control.
    step_log: Arc<Mutex<Option<Vec<serde_json::Value>>>>,
    /// Checkpoint directory of the current episode (game db + counters), set
    /// by reset. Every CHECKPOINT_STEPS acts the server persists the episode
    /// here so any interrupted run — single-season or 10Y — can resume.
    cp_dir: Arc<Mutex<Option<String>>>,
}

/// How often (in acts) the episode is checkpointed to disk.
pub const CHECKPOINT_STEPS: u64 = 8;

/// Persist the episode (game db + counters) into `cp_dir` so an interrupted
/// run can resume bit-identically. Fails loudly to stderr but never breaks
/// the agent loop — a lost checkpoint only costs the last few steps.
fn write_checkpoint(cp_dir: &str, seed: u64, ep: &Episode) -> Result<(), String> {
    let dir = std::path::Path::new(cp_dir);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let db_path = dir.join("game.db");
    // Start from an empty database: the persistence writer inserts a whole game
    // snapshot, so rewriting an existing file violates its primary keys and
    // every checkpoint after the first would fail.
    if db_path.exists() {
        std::fs::remove_file(&db_path).map_err(|e| e.to_string())?;
    }
    let db = db::game_database::GameDatabase::open(&db_path)?;
    db::game_persistence::GamePersistenceWriter::write_game(&db, &ep.game, "checkpoint", "checkpoint")?;
    let (horizon_days, target_seasons, seasons_completed, advanced_days, step,
         initial_net_worth, net_spend, snapshots, seen_offers, manager_firings,
         user_team_id, match_stops) = ep.checkpoint_fields();
    let state = json!({
        "seed": seed,
        "horizon_days": horizon_days,
        "target_seasons": target_seasons,
        "seasons_completed": seasons_completed,
        "advanced_days": advanced_days,
        "step": step,
        "initial_net_worth": initial_net_worth,
        "net_spend": net_spend,
        "snapshots": snapshots,
        "seen_offers": seen_offers,
        "manager_firings": manager_firings,
        "user_team_id": user_team_id,
        "match_stops": match_stops,
    });
    std::fs::write(dir.join("state.json"), serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Load a checkpoint if one exists; None = start fresh.
fn read_checkpoint(
    cp_dir: &str,
    seed: u64,
    mode: crate::env::AgentMode,
) -> Option<Episode> {
    let dir = std::path::Path::new(cp_dir);
    let state_path = dir.join("state.json");
    let db_path = dir.join("game.db");
    if !state_path.exists() || !db_path.exists() {
        return None;
    }
    let state: Value = serde_json::from_str(&std::fs::read_to_string(&state_path).ok()?).ok()?;
    if state.get("seed").and_then(Value::as_u64) != Some(seed) {
        eprintln!("[mcp] checkpoint seed mismatch — starting fresh");
        return None;
    }
    let db = db::game_database::GameDatabase::open(&db_path).ok()?;
    let game = db::game_persistence::GamePersistenceReader::read_game(&db).ok()?;
    let snapshots: Vec<crate::run::SeasonSnapshot> =
        serde_json::from_value(state.get("snapshots")?.clone()).ok()?;
    let seen_offers: std::collections::HashSet<String> =
        serde_json::from_value(state.get("seen_offers")?.clone()).ok()?;
    // Note: match_stops is applied by the reset caller (the reset args are
    // the single source of truth for the track); the checkpoint stores it
    // only for observability of resumed runs.
    Some(Episode::resume(
        seed,
        game,
        mode,
        state.get("horizon_days")?.as_u64()?,
        state.get("target_seasons")?.as_u64()? as u32,
        state.get("seasons_completed")?.as_u64()? as u32,
        state.get("advanced_days")?.as_u64()?,
        state.get("step")?.as_u64()?,
        state.get("initial_net_worth")?.as_i64()?,
        state.get("net_spend")?.as_i64()?,
        snapshots,
        seen_offers,
        state.get("manager_firings")?.as_u64()? as u32,
        state.get("user_team_id")?.as_str()?.to_string(),
    ))
}

impl Default for FromPitch2BoardMcp {
    fn default() -> Self {
        Self::new()
    }
}

impl FromPitch2BoardMcp {
    pub fn new() -> Self {
        Self {
            episode: Arc::new(Mutex::new(None)),
            meta: Arc::new(Mutex::new(None)),
            step_log: Arc::new(Mutex::new(None)),
            cp_dir: Arc::new(Mutex::new(None)),
        }
    }

    fn tools() -> Vec<Tool> {
        fn schema(v: Value) -> Arc<serde_json::Map<String, Value>> {
            Arc::new(v.as_object().cloned().unwrap_or_default())
        }
        vec![
            Tool::new(
                "reset",
                "Start a new episode: build the world, pick the club and scenario, seed the RNG. Returns the initial observation.",
                schema(json!({
                    "type": "object",
                    "properties": {
                        "seed": { "type": "number", "description": "reproducibility seed" },
                        "scenario": { "type": "string", "enum": ["crisis", "moneyball", "rebuild", "title"] },
                        "club": { "type": "number", "description": "managed-club strength rank, 0 = weakest" },
                        "world": { "type": "string", "enum": ["compact", "medium", "standard"] },
                        "mode": { "type": "string", "enum": ["coach", "manager"] },
                        "days": { "type": "number", "description": "episode horizon in game days" },
                        "match_stops": { "type": "boolean", "description": "L1 Match track: pause user matches at 30'/HT/60'/75' for live substitutions and tactic changes (default false)" },
                        "anonymize": { "type": "boolean", "description": "remap clubs/players to synthetic ids (default true)" }
                    },
                    "required": ["seed"]
                })),
            ),
            Tool::new(
                "observe",
                "Get the current decision-point observation (squad, offers, market, scout reports, standings, budget, …) as JSON.",
                schema(json!({ "type": "object", "properties": {} })),
            ),
            Tool::new(
                "act",
                "Apply one action and advance to the next decision point. Returns the next observation.",
                schema(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "object",
                            "description": "one of: Continue | SetMatchPlan | SetLineup | SetTactics | AcceptOffer | RejectOffer | CounterOffer | MakeBid | Scout | ListPlayer"
                        }
                    },
                    "required": ["action"]
                })),
            ),
            Tool::new(
                "score",
                "Final evaluation metrics for the current episode (sport / finance / squad), as JSON.",
                schema(json!({ "type": "object", "properties": {} })),
            ),
            Tool::new(
                "snapshots",
                "Per-season metric snapshots recorded so far in the episode (the Dynasty trajectory curve).",
                schema(json!({ "type": "object", "properties": {} })),
            ),
            Tool::new(
                "dump",
                "Persist the current episode (final game state + metadata) to a TrajectoryRecord JSON file at the given path, so it can be re-scored later without re-running the agent.",
                schema(json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "absolute output path for trajectory.json" }
                    },
                    "required": ["path"]
                })),
            ),
        ]
    }

    fn tool_reset(&self, args: Value) -> Result<String, String> {
        let seed = args.get("seed").and_then(Value::as_u64).ok_or("missing seed")?;
        let scenario = args.get("scenario").and_then(Value::as_str).unwrap_or("rebuild");
        let club = args.get("club").and_then(Value::as_u64).unwrap_or(0) as usize;
        let world = args.get("world").and_then(Value::as_str).unwrap_or("medium");
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("manager");
        let days = args.get("days").and_then(Value::as_u64).unwrap_or(400);
        let seasons = args.get("seasons").and_then(Value::as_u64).unwrap_or(1) as u32;
        let cp_dir = args.get("cp_dir").and_then(Value::as_str).map(String::from);
        let match_stops = args.get("match_stops").and_then(Value::as_bool).unwrap_or(false);

        let pick = ClubPick::Strength(club);
        let world = match world {
            "compact" => WorldSize::Compact,
            "standard" => WorldSize::Standard,
            _ => WorldSize::Medium,
        };
        let budget = ScenarioBudget::by_name(scenario);
        let mode = match mode {
            "coach" => AgentMode::Coach,
            "recruiter" => AgentMode::Recruiter,
            _ => AgentMode::Manager,
        };
        let anonymize = args.get("anonymize").and_then(Value::as_bool).unwrap_or(true);

        let mut ep = match &cp_dir {
            // Checkpoint resume: an interrupted run continues bit-identically
            // (keyed RNG is re-seeded from the ORIGINAL episode seed).
            Some(dir) => match read_checkpoint(dir, seed, mode) {
                Some(e) => {
                    eprintln!("[mcp] resumed episode from checkpoint {dir} (step {})", e.step_count());
                    e
                }
                None => Episode::new_with_mode_seasons(seed, &pick, world, &budget, mode, days, seasons),
            },
            None => Episode::new_with_mode_seasons(seed, &pick, world, &budget, mode, days, seasons),
        };
        ep = ep.with_match_stops(match_stops);
        if anonymize {
            ep.anonymize_identities();
        }
        let obs = ep.observe();
        *self.cp_dir.lock().unwrap() = cp_dir;
        *self.episode.lock().unwrap() = Some(ep);
        *self.step_log.lock().unwrap() = Some(Vec::new());
        *self.meta.lock().unwrap() = Some(crate::run::TrajectoryMeta {
            seed,
            scenario: scenario.to_string(),
            club,
            world: format!("{world:?}"),
            mode: format!("{mode:?}"),
            agent: "unknown".to_string(), // dump overrides this via its args
            horizon_days: days,
        });
        serde_json::to_string_pretty(&obs).map_err(|e| e.to_string())
    }

    fn tool_dump(&self, args: Value) -> Result<String, String> {
        use crate::run::TrajectoryRecord;
        let path = args.get("path").and_then(Value::as_str).ok_or("missing path")?;
        let agent = args.get("agent").and_then(Value::as_str).unwrap_or("unknown");
        let (ep_guard, meta_guard) = (self.episode.lock().unwrap(), self.meta.lock().unwrap());
        let ep = ep_guard.as_ref().ok_or("no episode — call reset first")?;
        let meta = meta_guard.as_ref().ok_or("no episode — call reset first")?;
        let record = TrajectoryRecord {
            seed: meta.seed,
            scenario: meta.scenario.clone(),
            club: meta.club,
            world: meta.world.clone(),
            mode: meta.mode.clone(),
            agent: agent.to_string(),
            horizon_days: meta.horizon_days,
            initial_net_worth: ep.initial_net_worth,
            net_spend: ep.net_spend,
            snapshots: ep.season_snapshots(),
            final_game: ep.game.clone(),
        };
        let json = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("write {path}: {e}"))?;
        // Lossless step-level tool trace → trajectory.jsonl (sibling of the
        // trajectory.json the caller asked for). The harness (llm_agent.py)
        // writes its own richer trajectory.jsonl with LLM token/latency data;
        // if it already exists, keep it — don't overwrite with server-side
        // timings. This file is only authoritative for off-the-shelf agents
        // (cc / codex) whose LLM internals we don't control.
        let jsonl_path = std::path::Path::new(path).with_extension("jsonl");
        if jsonl_path.exists() {
            return Ok(format!("trajectory written to {path} (trajectory.jsonl kept)"));
        }
        if let Some(log) = self.step_log.lock().unwrap().as_ref() {
            let lines: String = log
                .iter()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&jsonl_path, lines + "\n").map_err(|e| format!("write {jsonl_path:?}: {e}"))?;
        }
        Ok(format!("trajectory written to {path}"))
    }

    fn tool_observe(&self) -> Result<String, String> {
        let mut guard = self.episode.lock().unwrap();
        let ep = guard.as_mut().ok_or("no episode — call reset first")?;
        let obs = ep.observe();
        serde_json::to_string_pretty(&obs).map_err(|e| e.to_string())
    }

    fn tool_act(&self, args: Value) -> Result<String, String> {
        let action: Action = serde_json::from_value(
            args.get("action").ok_or("missing action")?.clone(),
        )
        .map_err(|e| format!("invalid action JSON: {e}"))?;
        let mut guard = self.episode.lock().unwrap();
        let ep = guard.as_mut().ok_or("no episode — call reset first")?;
        let log_action = action.clone();
        let obs = ep.step(action);
        // Periodic checkpoint: every CHECKPOINT_STEPS acts, persist the
        // episode so an interrupted run (any horizon) can resume instead of
        // restarting from season 1. Skipped mid-match: the live-match session
        // is not part of the game db, so a mid-match checkpoint would resume
        // with the fixture lost.
        if obs.step % CHECKPOINT_STEPS == 0 && !ep.in_match() {
            if let Some(dir) = self.cp_dir.lock().unwrap().as_ref() {
                let meta = self.meta.lock().unwrap();
                let seed = meta.as_ref().map(|m| m.seed).unwrap_or(0);
                drop(meta);
                if let Err(e) = write_checkpoint(dir, seed, ep) {
                    eprintln!("[mcp] checkpoint write failed: {e}");
                }
            }
        }
        if let Some(log) = self.step_log.lock().unwrap().as_mut() {
            log.push(json!({
                "step": obs.step,
                "date": obs.date,
                "action": log_action,
                "result": obs.last_action_result,
                // Per-step LLM latency/tokens are written by the harness that
                // controls the model (llm_agent.py); the env server cannot
                // measure them, so they stay null for off-the-shelf agents.
                "latency_ms": null,
                "input_tokens": null,
                "output_tokens": null,
                "cache_read_tokens": null,
                "tool_success": true,
            }));
        }
        serde_json::to_string_pretty(&obs).map_err(|e| e.to_string())
    }

    fn tool_score(&self) -> Result<String, String> {
        let guard = self.episode.lock().unwrap();
        let ep = guard.as_ref().ok_or("no episode — call reset first")?;
        let m = ep.final_metrics();
        serde_json::to_string_pretty(&m).map_err(|e| e.to_string())
    }

    fn tool_snapshots(&self) -> Result<String, String> {
        let guard = self.episode.lock().unwrap();
        let ep = guard.as_ref().ok_or("no episode — call reset first")?;
        serde_json::to_string_pretty(&ep.season_snapshots()).map_err(|e| e.to_string())
    }
}

impl ServerHandler for FromPitch2BoardMcp {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("FromPitch2Board MCP Server", env!("CARGO_PKG_VERSION")))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        let tools = Self::tools();
        std::future::ready(Ok(rmcp::model::ListToolsResult {
            tools,
            ..Default::default()
        }))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        Self::tools().into_iter().find(|t| t.name == name)
    }

    fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + MaybeSendFuture + '_ {
        let name = request.name.clone().into_owned();
        let args = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let this = self.clone();
        async move {
            let result = match name.as_str() {
                "reset" => this.tool_reset(args),
                "observe" => this.tool_observe(),
                "act" => this.tool_act(args),
                "score" => this.tool_score(),
                "snapshots" => this.tool_snapshots(),
                "dump" => this.tool_dump(args),
                other => return Err(McpError::invalid_params(format!("unknown tool: {other}"), None)),
            };
            match result {
                Ok(text) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)])),
                Err(e) => Err(McpError::invalid_params(e, None)),
            }
        }
    }
}

/// Start the MCP server over Streamable HTTP on `127.0.0.1:port`.
pub async fn serve(port: u16) -> Result<(), String> {
    let handler = FromPitch2BoardMcp::new();
    let session_manager =
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default();
    let service_factory = move || Ok::<_, std::io::Error>(handler.clone());
    let config = rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
        .with_stateful_mode(true);
    let service =
        rmcp::transport::streamable_http_server::StreamableHttpService::new(service_factory, Arc::new(session_manager), config);
    let app = axum::Router::new().fallback(axum::routing::any(move |req| {
        let service = service.clone();
        async move { service.handle(req).await }
    }));
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .map_err(|e| format!("bind failed: {e}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}
