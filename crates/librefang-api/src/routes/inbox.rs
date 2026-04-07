//! Inbox status endpoint.

use super::AppState;
use crate::middleware::AccountId;
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
pub async fn inbox_status(
    _account: AccountId,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let status = librefang_kernel::inbox::inbox_status(&cfg.inbox, state.kernel.home_dir());
    Json(serde_json::json!(status))
}
