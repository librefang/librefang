//! Declarative resource provisioning surface (#6695).
//!
//! `GET /api/config/status` answers "who owns `config.toml`". This module answers the same question one level down, for the resources a deployment declares outside that file — agents today.
//!
//! There is deliberately no write route here. A provisioning tree is deployment-owned by definition, so the only way to change it is to change the tree and roll the daemon, exactly as managed configuration is rollout-only.

use super::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

/// Build routes for the provisioning domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route(
        "/provisioning/status",
        axum::routing::get(provisioning_status),
    )
}

/// GET /api/provisioning/status — what the deployment's provisioning tree owns, and whether it has drifted.
///
/// `resources[].drifted` is true when the declaring file on disk no longer hashes to what was applied, which is the resource-level equivalent of comparing a ConfigMap's `checksum/config` annotation against `GET /api/config/status`.
/// A drifted resource means the tree moved and the daemon has not been rolled; it does **not** mean the running resource is broken.
///
/// `failures[]` carries one entry per file the last reconcile refused, with the reason already formatted for an operator.
/// A malformed manifest never fails the boot, so this endpoint is the only place that record survives after the boot log has scrolled.
///
/// Authenticated like every other `/api/*` route. It exposes paths and checksums, never manifest contents.
#[utoipa::path(
    get,
    path = "/api/provisioning/status",
    tag = "system",
    responses(
        (status = 200, description = "Provisioned resources, their provenance, and reconcile failures", body = crate::types::JsonObject)
    )
)]
pub async fn provisioning_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    axum::Json(state.kernel.provisioning_status())
}
