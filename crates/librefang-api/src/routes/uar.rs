//! UAR (Universal Agent Runtime) A2A protocol and discovery routes.
//!
//! Implements the A2A RC v1.0 endpoints required for inter-agent interoperability:
//!
//! - `GET  /.well-known/agent.json`  — AgentCard for this librefang instance
//! - `POST /a2a`                     — JSON-RPC 2.0 message dispatcher (A2A tasks)
//! - `GET  /api/uar/discovery/agents`— list all librefang agents as UAR AgentArtifacts
//!
//! The A2A handler dispatches `message/send`, `tasks/get`, and `tasks/cancel`
//! methods. A lightweight in-process task store maps A2A task IDs to agent
//! sessions so external clients can track long-running agent interactions.

use super::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use librefang_types::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Build the **root-level** A2A JSON-RPC route.
///
/// `POST /a2a` is the single JSON-RPC 2.0 entry point defined by A2A RC v1.0
/// §5. It must live at the root, NOT under `/api`. The instance-level
/// `GET /.well-known/agent.json` is served by [`crate::routes::network`]
/// which already aggregates skills across all loaded agents — we do not
/// re-register it here to avoid an overlapping-route panic at boot.
///
/// Mount this directly on the top-level [`axum::Router`] in `server.rs`.
pub fn root_router() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route("/a2a", axum::routing::post(handle_a2a_rpc))
}

/// Build the **`/api`-prefixed** UAR discovery routes.
///
/// These are merged into `api_v1_routes()` and so will be served at:
///
/// - `GET /api/uar/discovery/agents`
/// - `GET /api/uar/discovery/agents/{id}/card`
///
/// Their paths here are written *relative to* the `/api` mount point so the
/// full URL matches the documented spec exactly.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/uar/discovery/agents",
            axum::routing::get(list_agents_as_artifacts),
        )
        .route(
            "/uar/discovery/agents/{id}/card",
            axum::routing::get(get_agent_card),
        )
}

// ─────────────────────────────────────────────────────────────────────────────
// A2A types (minimal, self-contained — mirrors A2A RC v1.0 §3-5)
// ─────────────────────────────────────────────────────────────────────────────

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    fn ok(id: Option<serde_json::Value>, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::to_value(result).unwrap_or(serde_json::Value::Null)),
            error: None,
        }
    }

    fn err(id: Option<serde_json::Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

mod rpc_error {
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const TASK_NOT_FOUND: i32 = -32001;
}

/// A2A task states per RC v1.0 §4.2.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Canceled,
    Failed,
}

/// Lightweight A2A task — maps to a librefang agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct A2ATask {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_id: Option<String>,
    status: A2ATaskStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct A2ATaskStatus {
    state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<serde_json::Value>,
}

/// Params for `message/send`.
#[derive(Debug, Deserialize)]
struct MessageSendParams {
    message: serde_json::Value,
    #[serde(default)]
    task_id: Option<String>,
    /// Agent ID to route the message to. If omitted, the first available agent is used.
    #[serde(default)]
    agent_id: Option<String>,
}

/// Params for `tasks/get` and `tasks/cancel`.
#[derive(Debug, Deserialize)]
struct TaskRefParams {
    id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// AgentCard types (A2A RC v1.0 §3)
// ─────────────────────────────────────────────────────────────────────────────

/// AgentCard — machine-readable capability declaration.
#[derive(Debug, Clone, Serialize)]
struct AgentCard {
    name: String,
    description: String,
    url: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation_url: Option<String>,
    capabilities: AgentCapabilities,
    skills: Vec<AgentSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    default_input_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    default_output_modes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AgentCapabilities {
    streaming: bool,
    push_notifications: bool,
    state_transition_history: bool,
}

#[derive(Debug, Clone, Serialize)]
struct AgentSkill {
    id: String,
    name: String,
    description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    output_modes: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// In-process task store (DashMap, ephemeral)
// ─────────────────────────────────────────────────────────────────────────────

use dashmap::DashMap;
use std::sync::LazyLock;

/// Ephemeral in-memory A2A task store.
///
/// Tasks are keyed by UUID string. Lost on restart — callers that need
/// durability should implement their own task persistence on top.
static TASK_STORE: LazyLock<DashMap<String, A2ATask>> = LazyLock::new(DashMap::new);

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Derive the public base URL for A2A routes.
///
/// Resolution order:
/// 1. `LIBREFANG_A2A_BASE_URL` environment variable
/// 2. `[uar].base_url` from kernel config
/// 3. `http://localhost:4545` (default API port)
fn a2a_base_url(state: &AppState) -> String {
    if let Ok(url) = std::env::var("LIBREFANG_A2A_BASE_URL") {
        if !url.is_empty() {
            return url;
        }
    }
    state
        .kernel
        .config_ref()
        .uar
        .as_ref()
        .and_then(|u| u.base_url.clone())
        .unwrap_or_else(|| "http://localhost:4545".into())
}

// ─────────────────────────────────────────────────────────────────────────────
// Route handlers
// ─────────────────────────────────────────────────────────────────────────────

/// `POST /a2a` — JSON-RPC 2.0 A2A dispatcher.
async fn handle_a2a_rpc(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::err(
            req.id,
            rpc_error::INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        ));
    }

    let resp = match req.method.as_str() {
        "message/send" => dispatch_message_send(&state, req.id.clone(), req.params).await,
        "tasks/get" => dispatch_tasks_get(req.id.clone(), req.params),
        "tasks/cancel" => dispatch_tasks_cancel(req.id.clone(), req.params),
        other => {
            tracing::warn!(method = other, "unknown A2A method");
            JsonRpcResponse::err(
                req.id,
                rpc_error::METHOD_NOT_FOUND,
                format!("method '{other}' not found"),
            )
        }
    };

    Json(resp)
}

/// `GET /api/uar/discovery/agents` — list all agents as UAR `AgentArtifact`s.
async fn list_agents_as_artifacts(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let entries = state.kernel.agent_registry().list();
    let mut artifacts = Vec::with_capacity(entries.len());

    for entry in entries {
        if entry.is_hand {
            continue;
        }
        let manifest = match state.kernel.memory_substrate().load_agent(entry.id) {
            Ok(Some(record)) => record.manifest,
            _ => continue,
        };
        let art = librefang_uar_spec::translator::manifest_to_artifact(&manifest);
        artifacts.push(art);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"agents": artifacts})),
    )
}

/// `GET /api/uar/discovery/agents/{id}/card` — per-agent AgentCard.
async fn get_agent_card(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid agent id"})),
            )
                .into_response();
        }
    };

    let record = match state.kernel.memory_substrate().load_agent(agent_id) {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "agent not found"})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    let base_url = a2a_base_url(&state);
    let card = build_agent_card_from_manifest(&record.manifest, &base_url);
    (StatusCode::OK, Json(card)).into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
// A2A method implementations
// ─────────────────────────────────────────────────────────────────────────────

async fn dispatch_message_send(
    state: &AppState,
    id: Option<serde_json::Value>,
    params: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let params: MessageSendParams = match params
        .ok_or_else(|| "params required".to_string())
        .and_then(|p| serde_json::from_value(p).map_err(|e| e.to_string()))
    {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::err(id, rpc_error::INVALID_PARAMS, e),
    };

    // Extract text content from the A2A message.
    let user_text = extract_text_from_message(&params.message);

    // Resolve agent — use explicit agent_id or pick the first available one.
    let agent_id: AgentId = if let Some(ref aid) = params.agent_id {
        match aid.parse() {
            Ok(id) => id,
            Err(_) => {
                return JsonRpcResponse::err(id, rpc_error::INVALID_PARAMS, "invalid agent_id");
            }
        }
    } else {
        match state
            .kernel
            .agent_registry()
            .list()
            .into_iter()
            .find(|e| !e.is_hand)
        {
            Some(e) => e.id,
            None => {
                return JsonRpcResponse::err(id, rpc_error::INVALID_PARAMS, "no agents available");
            }
        }
    };

    // Create or continue an A2A task.
    let task_id = params
        .task_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Build submitted task.
    let task = A2ATask {
        id: task_id.clone(),
        context_id: Some(agent_id.to_string()),
        status: A2ATaskStatus {
            state: TaskState::Working,
            message: None,
        },
        artifacts: vec![],
    };
    TASK_STORE.insert(task_id.clone(), task.clone());

    // Send message to the librefang agent via kernel.
    match state.kernel.send_message(agent_id, &user_text).await {
        Ok(result) => {
            let response_text = result.response;
            let completed_task = A2ATask {
                id: task_id.clone(),
                context_id: Some(agent_id.to_string()),
                status: A2ATaskStatus {
                    state: TaskState::Completed,
                    message: Some(serde_json::json!({
                        "role": "agent",
                        "parts": [{"type": "text", "text": response_text}]
                    })),
                },
                artifacts: vec![serde_json::json!({
                    "artifact_id": uuid::Uuid::new_v4().to_string(),
                    "name": "response",
                    "parts": [{"type": "text", "text": response_text}]
                })],
            };
            TASK_STORE.insert(task_id, completed_task.clone());
            JsonRpcResponse::ok(id, completed_task)
        }
        Err(e) => {
            let failed_task = A2ATask {
                id: task_id.clone(),
                status: A2ATaskStatus {
                    state: TaskState::Failed,
                    message: Some(serde_json::json!({
                        "role": "agent",
                        "parts": [{"type": "text", "text": e.to_string()}]
                    })),
                },
                context_id: Some(agent_id.to_string()),
                artifacts: vec![],
            };
            TASK_STORE.insert(task_id, failed_task.clone());
            JsonRpcResponse::ok(id, failed_task)
        }
    }
}

fn dispatch_tasks_get(
    id: Option<serde_json::Value>,
    params: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let params: TaskRefParams = match params
        .ok_or_else(|| "params required".to_string())
        .and_then(|p| serde_json::from_value(p).map_err(|e| e.to_string()))
    {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::err(id, rpc_error::INVALID_PARAMS, e),
    };

    match TASK_STORE.get(&params.id) {
        Some(task) => JsonRpcResponse::ok(id, task.clone()),
        None => JsonRpcResponse::err(id, rpc_error::TASK_NOT_FOUND, "task not found"),
    }
}

fn dispatch_tasks_cancel(
    id: Option<serde_json::Value>,
    params: Option<serde_json::Value>,
) -> JsonRpcResponse {
    let params: TaskRefParams = match params
        .ok_or_else(|| "params required".to_string())
        .and_then(|p| serde_json::from_value(p).map_err(|e| e.to_string()))
    {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::err(id, rpc_error::INVALID_PARAMS, e),
    };

    match TASK_STORE.get_mut(&params.id) {
        Some(mut entry) => {
            entry.status.state = TaskState::Canceled;
            JsonRpcResponse::ok(id, entry.clone())
        }
        None => JsonRpcResponse::err(id, rpc_error::TASK_NOT_FOUND, "task not found"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AgentCard builders
// ─────────────────────────────────────────────────────────────────────────────

fn build_agent_card_from_manifest(
    manifest: &librefang_types::agent::AgentManifest,
    base_url: &str,
) -> AgentCard {
    let skills: Vec<AgentSkill> = manifest
        .skills
        .iter()
        .map(|s| AgentSkill {
            id: s.clone(),
            name: s.clone(),
            description: String::new(),
            tags: vec![],
            input_modes: vec!["text".into()],
            output_modes: vec!["text".into()],
        })
        .collect();

    AgentCard {
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        url: format!("{base_url}/a2a"),
        version: "1.0.0".into(),
        documentation_url: None,
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
            state_transition_history: true,
        },
        skills,
        default_input_modes: vec!["text".into()],
        default_output_modes: vec!["text".into()],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Extract the first text string from an A2A message JSON value.
fn extract_text_from_message(message: &serde_json::Value) -> String {
    if let Some(parts) = message.get("parts").and_then(|p| p.as_array()) {
        for part in parts {
            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    return text.to_string();
                }
            }
        }
    }
    // Fallback: treat the whole value as a text message if no parts
    message.as_str().unwrap_or_default().to_string()
}
