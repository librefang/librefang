//! Inbox status endpoint.

use super::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// 构建 inbox 领域的路由。
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route("/inbox/status", axum::routing::get(inbox_status))
}

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
        &state.kernel.config_ref().inbox,
        &state.kernel.config_ref().home_dir,
    );
    Json(serde_json::json!(status))
}
