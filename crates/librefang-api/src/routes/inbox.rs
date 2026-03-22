//! Inbox status endpoint.

use super::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// GET /api/inbox/status — Return inbox configuration and file counts.
#[utoipa::path(
    get,
    path = "/api/inbox/status",
    tag = "inbox",
    responses(
        (status = 200, description = "Inbox status", body = serde_json::Value)
    )
)]
pub async fn inbox_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = librefang_kernel::inbox::inbox_status(
        &state.kernel.config.inbox,
        &state.kernel.config.home_dir,
    );
    Json(serde_json::json!(status))
}
