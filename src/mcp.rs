//! ClubBench MCP server — exposes the headless environment to any MCP-capable
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
pub struct ClubBenchMcp {
    episode: Arc<Mutex<Option<Episode>>>,
}

impl Default for ClubBenchMcp {
    fn default() -> Self {
        Self::new()
    }
}

impl ClubBenchMcp {
    pub fn new() -> Self {
        Self {
            episode: Arc::new(Mutex::new(None)),
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
                        "days": { "type": "number", "description": "episode horizon in game days" }
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
        ]
    }

    fn tool_reset(&self, args: Value) -> Result<String, String> {
        let seed = args.get("seed").and_then(Value::as_u64).ok_or("missing seed")?;
        let scenario = args.get("scenario").and_then(Value::as_str).unwrap_or("rebuild");
        let club = args.get("club").and_then(Value::as_u64).unwrap_or(0) as usize;
        let world = args.get("world").and_then(Value::as_str).unwrap_or("medium");
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("manager");
        let days = args.get("days").and_then(Value::as_u64).unwrap_or(400);

        let pick = ClubPick::Strength(club);
        let world = match world {
            "compact" => WorldSize::Compact,
            "standard" => WorldSize::Standard,
            _ => WorldSize::Medium,
        };
        let budget = ScenarioBudget::by_name(scenario);
        let mode = if mode == "coach" { AgentMode::Coach } else { AgentMode::Manager };

        let mut ep = Episode::new_with_mode(seed, &pick, world, &budget, mode, days);
        let obs = ep.observe();
        *self.episode.lock().unwrap() = Some(ep);
        serde_json::to_string_pretty(&obs).map_err(|e| e.to_string())
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
        let obs = ep.step(action);
        serde_json::to_string_pretty(&obs).map_err(|e| e.to_string())
    }

    fn tool_score(&self) -> Result<String, String> {
        let guard = self.episode.lock().unwrap();
        let ep = guard.as_ref().ok_or("no episode — call reset first")?;
        let m = ep.final_metrics();
        serde_json::to_string_pretty(&m).map_err(|e| e.to_string())
    }
}

impl ServerHandler for ClubBenchMcp {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("ClubBench MCP Server", env!("CARGO_PKG_VERSION")))
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
    let handler = ClubBenchMcp::new();
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
