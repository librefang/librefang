//! Auto-dream HTTP endpoints.
//!
//! * `GET /api/auto-dream/status` — Global config + per-agent status
//!   (opt-in flag, last-consolidated timestamp, next eligible time,
//!   sessions touched since, optional live progress).
//! * `POST /api/auto-dream/agents/{id}/trigger` — Manually fire a
//!   consolidation. Bypasses time and session gates, respects the lock.
//! * `POST /api/auto-dream/agents/{id}/abort` — Cancel an in-flight
//!   manual dream. Scheduled dreams cannot be aborted.
//! * `PUT /api/auto-dream/agents/{id}/enabled` — Toggle the agent's
//!   `auto_dream_enabled` opt-in flag. Body: `{"enabled": bool}`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use super::AppState;
use crate::extractors::AgentIdPath;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/auto-dream/status", axum::routing::get(auto_dream_status))
        .route(
            "/auto-dream/agents/{id}/trigger",
            axum::routing::post(auto_dream_trigger),
        )
        .route(
            "/auto-dream/agents/{id}/abort",
            axum::routing::post(auto_dream_abort),
        )
        .route(
            "/auto-dream/agents/{id}/enabled",
            axum::routing::put(auto_dream_set_enabled),
        )
}

#[utoipa::path(
    get,
    path = "/api/auto-dream/status",
    tag = "auto_dream",
    responses((status = 200, description = "Auto-dream status", body = crate::types::JsonObject))
)]
pub async fn auto_dream_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = state.kernel.auto_dream_status().await;
    Json(status)
}

#[utoipa::path(
    post,
    path = "/api/auto-dream/agents/{id}/trigger",
    tag = "auto_dream",
    params(("id" = String, Path, description = "Agent UUID")),
    responses(
        (status = 200, description = "Trigger outcome", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent id"),
    )
)]
pub async fn auto_dream_trigger(
    State(state): State<Arc<AppState>>,
    AgentIdPath(agent_id): AgentIdPath,
) -> impl IntoResponse {
    let outcome = Arc::clone(&state.kernel)
        .auto_dream_trigger_manual(agent_id)
        .await;
    // #3511: tag response so request_logging middleware can emit
    // `agent_id` as a structured field on the access-log line.
    crate::extensions::with_agent_id(agent_id, Json(outcome))
}

#[utoipa::path(
    post,
    path = "/api/auto-dream/agents/{id}/abort",
    tag = "auto_dream",
    params(("id" = String, Path, description = "Agent UUID")),
    responses(
        (status = 200, description = "Abort outcome", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent id"),
    )
)]
pub async fn auto_dream_abort(
    State(state): State<Arc<AppState>>,
    AgentIdPath(agent_id): AgentIdPath,
) -> impl IntoResponse {
    let outcome = state.kernel.auto_dream_abort(agent_id).await;
    // #3511: tag response with agent_id for the access-log middleware.
    crate::extensions::with_agent_id(agent_id, Json(outcome))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetEnabledRequest {
    pub enabled: bool,
}

#[utoipa::path(
    put,
    path = "/api/auto-dream/agents/{id}/enabled",
    tag = "auto_dream",
    params(("id" = String, Path, description = "Agent UUID")),
    responses(
        (status = 200, description = "Opt-in toggled", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent id"),
        (status = 404, description = "Agent not found"),
    )
)]
pub async fn auto_dream_set_enabled(
    State(state): State<Arc<AppState>>,
    AgentIdPath(agent_id): AgentIdPath,
    Json(req): Json<SetEnabledRequest>,
) -> impl IntoResponse {
    let body = match state.kernel.auto_dream_set_enabled(agent_id, req.enabled) {
        Ok(()) => Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "enabled": req.enabled,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    };
    // #3511: tag response with agent_id for the access-log middleware.
    crate::extensions::with_agent_id(agent_id, body)
}
