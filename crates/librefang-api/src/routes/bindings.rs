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

/// Push the kernel's binding list into the running channel bridge's router.
///
/// The router copies the bindings once when the bridge starts, so without this an added or removed binding would not route a single message until the next restart or channel reload.
/// `resolved` is the name the new binding uses and the agent it names: the router resolves a binding through its own name cache, which is seeded only at bridge start (and only when bindings existed then), so a binding to an agent defined or spawned since then would match and still route nowhere.
/// With no bridge running (no channels configured) there is no router to update, and the next bridge start loads the kernel list itself.
fn sync_live_router(state: &AppState, resolved: Option<(String, librefang_types::agent::AgentId)>) {
    let guard = state.bridge_manager.load();
    let Some(bridge) = &**guard else {
        return;
    };
    let router = bridge.router();
    if let Some((name, id)) = resolved {
        router.register_agent(name, id);
    }
    router.load_bindings(&state.kernel.list_bindings());
}

/// The agent a binding's `agent` field names, whether or not it has spawned yet.
///
/// A spawned agent's live registry entry wins, and the canonical identity registry covers every defined agent that has not spawned (one that first runs on a cron fire, or one left Suspended), the same order the bridge uses to seed the router at start.
/// Without the identity lookup a binding to such an agent would load into the router and still route nowhere until a restart.
/// A UUID is accepted when either registry knows it.
fn resolve_binding_agent(state: &AppState, agent: &str) -> Option<librefang_types::agent::AgentId> {
    let registry = state.kernel.agent_registry();
    let identities = state.kernel.agent_identities();
    if let Some(id) = registry
        .list()
        .iter()
        .find(|e| e.name == agent)
        .map(|e| e.id)
        .or_else(|| identities.get(agent))
    {
        return Some(id);
    }
    let id = agent.parse::<librefang_types::agent::AgentId>().ok()?;
    let known = registry.get(id).is_some()
        || identities
            .list()
            .values()
            .any(|record| record.canonical_uuid == id);
    known.then_some(id)
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
    let resolved = resolve_binding_agent(&state, &binding.agent);
    if resolved.is_none() {
        tracing::warn!(agent = %binding.agent, "Binding references unknown agent");
    }

    let agent = binding.agent.clone();
    state.kernel.add_binding(binding);
    sync_live_router(&state, resolved.map(|id| (agent, id)));
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "status": "created" })),
    )
}

/// DELETE /api/bindings/:index — Remove a binding by index.
#[utoipa::path(delete, path = "/api/bindings/{index}", tag = "system", params(("index" = u32, Path, description = "Binding index")), responses((status = 204, description = "Binding removed")))]
pub async fn remove_binding(
    State(state): State<Arc<AppState>>,
    Path(index): Path<u32>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match state.kernel.remove_binding(index as usize) {
        // A bodyless 204: a generated client decoding a no-content response trips over both a
        // serialized JSON `null` and the `content-type` that shipping one implies.
        // `bindings_routes_test.rs` drives the mounted router and asserts both.
        Some(_) => {
            sync_live_router(&state, None);
            StatusCode::NO_CONTENT.into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": t.t("api-error-binding-index-out-of-range") })),
        )
            .into_response(),
    }
}
