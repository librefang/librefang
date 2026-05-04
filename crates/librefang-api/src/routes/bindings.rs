//! Agent bindings (key-binding manifest) endpoints (#3749 11/N).

use super::AppState;
use crate::middleware::RequestLanguage;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/bindings",
            axum::routing::get(list_bindings).post(add_binding),
        )
        .route("/bindings/{index}", axum::routing::delete(remove_binding))
}

/// GET /api/bindings — List all agent bindings.
#[utoipa::path(get, path = "/api/bindings", tag = "system", responses((status = 200, description = "List key bindings", body = Vec<serde_json::Value>)))]
pub async fn list_bindings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bindings = state.kernel.list_bindings();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "bindings": bindings })),
    )
}

/// POST /api/bindings — Add a new agent binding.
///
/// Validation is **advisory**: a binding referencing an unknown agent is
/// still accepted (kernel-side `add_binding` is infallible and binding
/// state may be primed before the target agent spawns), but a `WARN` is
/// logged so misconfiguration is visible. Both name lookup and UUID
/// resolution check the registry — the previous form passed any
/// well-formed UUID even if no such agent existed, so the warn would
/// never fire on a UUID typo.
#[utoipa::path(
    post,
    path = "/api/bindings",
    tag = "system",
    request_body = crate::types::JsonObject,
    responses((status = 201, description = "Binding added", body = crate::types::JsonObject))
)]
pub async fn add_binding(
    State(state): State<Arc<AppState>>,
    Json(binding): Json<librefang_types::config::AgentBinding>,
) -> impl IntoResponse {
    let registry = state.kernel.agent_registry();
    let agent_exists = registry.list().iter().any(|e| e.name == binding.agent)
        || binding
            .agent
            .parse::<librefang_types::agent::AgentId>()
            .ok()
            .is_some_and(|id| registry.get(id).is_some());
    if !agent_exists {
        tracing::warn!(agent = %binding.agent, "Binding references unknown agent");
    }

    state.kernel.add_binding(binding);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "status": "created" })),
    )
}

/// DELETE /api/bindings/:index — Remove a binding by index.
#[utoipa::path(delete, path = "/api/bindings/{index}", tag = "system", params(("index" = u32, Path, description = "Binding index")), responses((status = 200, description = "Binding removed")))]
pub async fn remove_binding(
    State(state): State<Arc<AppState>>,
    Path(index): Path<usize>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match state.kernel.remove_binding(index) {
        Some(_) => (StatusCode::NO_CONTENT, Json(serde_json::json!(null))),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": t.t("api-error-binding-index-out-of-range") })),
        ),
    }
}
