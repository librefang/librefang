//! Auto-dream HTTP endpoints.
//!
//! * `GET /api/auto-dream/status` — Current enable state, target agent,
//!   last-consolidated timestamp, and when the next consolidation is
//!   eligible to fire. Cheap: one stat of the lock file.
//! * `POST /api/auto-dream/trigger` — Manually fire a consolidation. Bypasses
//!   the time gate but still respects the process-lock so two triggers
//!   cannot double-fire. Returns immediately; the dream runs detached.

use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

use super::AppState;

/// Build routes for the auto-dream domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/auto-dream/status", axum::routing::get(auto_dream_status))
        .route(
            "/auto-dream/trigger",
            axum::routing::post(auto_dream_trigger),
        )
}

/// GET /api/auto-dream/status — Current auto-dream status.
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

/// POST /api/auto-dream/trigger — Manually trigger a consolidation.
#[utoipa::path(
    post,
    path = "/api/auto-dream/trigger",
    tag = "auto_dream",
    responses((status = 200, description = "Trigger outcome", body = serde_json::Value))
)]
pub async fn auto_dream_trigger(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let outcome = librefang_kernel::auto_dream::trigger_manual(Arc::clone(&state.kernel)).await;
    Json(outcome)
}
