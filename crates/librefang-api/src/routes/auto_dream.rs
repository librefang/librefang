//! Auto-dream HTTP endpoints.
//!
//! * `GET /api/auto-dream/status` — Global config + per-enrolled-agent
//!   status (last-consolidated timestamp, next eligible time, sessions
//!   touched since). Cheap: one stat + one count per enrolled agent.
//! * `POST /api/auto-dream/agents/{id}/trigger` — Manually fire a
//!   consolidation for a specific agent. Bypasses time and session gates
//!   but still respects the per-agent lock.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentId;

use super::AppState;

/// Build routes for the auto-dream domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/auto-dream/status", axum::routing::get(auto_dream_status))
        .route(
            "/auto-dream/agents/{id}/trigger",
            axum::routing::post(auto_dream_trigger),
        )
}

/// GET /api/auto-dream/status — Global auto-dream status + per-agent status.
#[utoipa::path(
    get,
    path = "/api/auto-dream/status",
    tag = "auto_dream",
    responses((status = 200, description = "Auto-dream status", body = serde_json::Value))
)]
pub async fn auto_dream_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = librefang_kernel::auto_dream::current_status(&state.kernel).await;
    Json(status)
}

/// POST /api/auto-dream/agents/{id}/trigger — Manually trigger a
/// consolidation for a specific agent.
#[utoipa::path(
    post,
    path = "/api/auto-dream/agents/{id}/trigger",
    tag = "auto_dream",
    params(("id" = String, Path, description = "Agent UUID")),
    responses(
        (status = 200, description = "Trigger outcome", body = serde_json::Value),
        (status = 400, description = "Invalid agent id"),
    )
)]
pub async fn auto_dream_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match id.parse::<AgentId>() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid agent id"})),
            )
                .into_response();
        }
    };
    let outcome =
        librefang_kernel::auto_dream::trigger_manual(Arc::clone(&state.kernel), agent_id).await;
    Json(outcome).into_response()
}
