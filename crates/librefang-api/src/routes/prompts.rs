use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use librefang_types::agent::{PromptExperiment, PromptVersion};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::AppState;
use std::sync::Arc;

use crate::types::ApiErrorResponse;

/// One row of the cross-agent prompt repository overview.
///
/// Aggregates the per-agent prompt-version store into a single fleet-wide
/// summary so the dashboard repository page can list every agent's prompt
/// at a glance. `active_*` mirror the version currently flagged
/// `is_active` in the store (the agent's recorded "active" prompt
/// snapshot); `live_system_prompt` is the prompt that actually rides LLM
/// calls (`manifest.model.system_prompt`) — the two can differ until a
/// version is bound back onto the manifest.
#[derive(Debug, Serialize)]
struct PromptOverviewItem {
    agent_id: String,
    agent_name: String,
    version_count: usize,
    active_version: Option<u32>,
    active_version_id: Option<String>,
    /// The prompt text currently used in live LLM calls for this agent.
    live_system_prompt: String,
    /// Creation timestamp (RFC 3339) of the most recent version, if any.
    latest_version_at: Option<String>,
}
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Cross-agent repository overview: one summary row per non-hand
        // agent (version count + active version), so the dashboard prompt
        // repository page can render the whole fleet without N round-trips
        // to the per-agent `/agents/{id}/prompts/versions` endpoint.
        .route("/prompts/overview", get(list_prompts_overview))
        .route(
            "/agents/{agent_id}/prompts/versions",
            get(list_prompt_versions),
        )
        .route(
            "/agents/{agent_id}/prompts/versions",
            post(create_prompt_version),
        )
        .route("/prompts/versions/{id}", get(get_prompt_version))
        .route("/prompts/versions/{id}", delete(delete_prompt_version))
        .route(
            "/prompts/versions/{id}/activate",
            post(activate_prompt_version),
        )
        .route(
            "/agents/{agent_id}/prompts/experiments",
            get(list_experiments),
        )
        .route(
            "/agents/{agent_id}/prompts/experiments",
            post(create_experiment),
        )
        .route("/prompts/experiments/{id}", get(get_experiment))
        .route("/prompts/experiments/{id}/start", post(start_experiment))
        .route("/prompts/experiments/{id}/pause", post(pause_experiment))
        .route(
            "/prompts/experiments/{id}/complete",
            post(complete_experiment),
        )
        .route(
            "/prompts/experiments/{id}/metrics",
            get(get_experiment_metrics),
        )
}

/// `GET /api/prompts/overview` — fleet-wide prompt repository summary.
///
/// Walks every non-hand agent in the registry and folds its prompt-version
/// store into one summary row. Hand agents are excluded for the same
/// reason `list_agents` excludes them by default — they are managed
/// through their HAND.toml, not as standalone prompt-owning agents.
///
/// This is a read-only aggregation over existing kernel methods
/// (`agent_registry().list_arcs()` + `list_prompt_versions`); it adds no
/// new storage and no new kernel trait method. A store error for any
/// single agent degrades that agent's row to a zero-version summary
/// rather than failing the whole response.
async fn list_prompts_overview(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.agent_registry().list_arcs();
    let mut items: Vec<PromptOverviewItem> = Vec::with_capacity(agents.len());
    for entry in agents.iter().filter(|e| !e.is_hand) {
        // Store errors are non-fatal: surface the agent with zero versions
        // rather than 500-ing the whole repository view on one bad row.
        let versions = state
            .kernel
            .list_prompt_versions(entry.id)
            .unwrap_or_default();
        let active = versions.iter().find(|v| v.is_active);
        // `list_versions` orders by `version DESC`, so the first row is the
        // most recent; fall back to a max scan in case the ordering changes.
        let latest_at = versions
            .iter()
            .max_by_key(|v| v.version)
            .map(|v| v.created_at.to_rfc3339());
        items.push(PromptOverviewItem {
            agent_id: entry.id.to_string(),
            agent_name: entry.name.clone(),
            version_count: versions.len(),
            active_version: active.map(|v| v.version),
            active_version_id: active.map(|v| v.id.to_string()),
            live_system_prompt: entry.manifest.model.system_prompt.clone(),
            latest_version_at: latest_at,
        });
    }
    let total = items.len();
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: 0,
        limit: None,
    })
    .into_response()
}

async fn list_prompt_versions(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let agent_id: librefang_types::agent::AgentId = match agent_id.parse() {
        Ok(id) => id,
        Err(e) => {
            return ApiErrorResponse::bad_request(e.to_string())
                .into_json_tuple()
                .into_response()
        }
    };
    let body = match state.kernel.list_prompt_versions(agent_id) {
        Ok(versions) => {
            let total = versions.len();
            Json(crate::types::PaginatedResponse {
                items: versions,
                total,
                offset: 0,
                limit: None,
            })
            .into_response()
        }
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    };
    // #3511: tag response so request_logging middleware can emit `agent_id`.
    crate::extensions::with_agent_id(agent_id, body)
}

async fn create_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(mut version): Json<PromptVersion>,
) -> impl IntoResponse {
    let agent_id: librefang_types::agent::AgentId = match agent_id.parse() {
        Ok(id) => id,
        Err(e) => {
            return ApiErrorResponse::bad_request(e.to_string())
                .into_json_tuple()
                .into_response()
        }
    };
    // Audit: `docs/issues/prompt-version-system-prompt-no-cap.md`.
    // Reject oversize `system_prompt` BEFORE any write. Once a version is
    // activated, its `system_prompt` rides every LLM call — an uncapped
    // field is a direct token-cost amplification vector. Cheked against
    // both a byte cap (memory) and a character cap (token / billing).
    if let Err(e) = crate::validation::check_system_prompt_size(&version.system_prompt) {
        return e.into_response();
    }
    version.agent_id = agent_id;
    version.id = uuid::Uuid::new_v4();
    version.created_at = chrono::Utc::now();
    // Audit: ignore client-supplied `is_active`. The create endpoint MUST
    // NOT side-channel activation: the only legitimate path to flip a
    // version active is `POST /prompts/versions/{id}/activate`, which
    // additionally invariant-checks against the existing active version.
    version.is_active = false;
    // Audit: ignore client-supplied `version`. Versions are monotonic
    // per agent and the server is the single source of truth — a client
    // picking `version = 999` would break monotonicity assumptions
    // downstream (active-version selection, list ordering, audit log).
    // Compute `prev_max + 1` from the existing rows for this agent.
    let next_version = match state.kernel.list_prompt_versions(agent_id) {
        Ok(existing) => existing.iter().map(|v| v.version).max().unwrap_or(0) + 1,
        Err(e) => return ApiErrorResponse::from(e).into_response(),
    };
    version.version = next_version;
    // Compute content hash from system_prompt
    let mut hasher = Sha256::new();
    hasher.update(version.system_prompt.as_bytes());
    version.content_hash = format!("{:x}", hasher.finalize());
    let body = match state.kernel.create_prompt_version(&version) {
        // Issue #3832: POST /versions creates a new resource — 201 Created.
        Ok(_) => (StatusCode::CREATED, Json(version)).into_response(),
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    };
    // #3511: tag response so request_logging middleware can emit `agent_id`.
    crate::extensions::with_agent_id(agent_id, body)
}

async fn get_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_prompt_version(&id) {
        Ok(version) => Json(version).into_response(),
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}

async fn delete_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.delete_prompt_version(&id) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}

async fn activate_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match body.get("agent_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ApiErrorResponse::bad_request("agent_id required in body")
                .into_json_tuple()
                .into_response()
        }
    };
    if let Err(e) = state.kernel.set_active_prompt_version(&id, agent_id) {
        // #3541: typed KernelOpError → status-code via From impl.
        return ApiErrorResponse::from(e).into_response();
    }
    // Read back the activated version so the caller can patch caches in place
    // without an extra round-trip. If the version vanished between write and
    // read (narrow race — concurrent delete) or the kernel implementation
    // doesn't expose it (e.g. mock kernels in tests, or stores that accept
    // activate without persisting versions), fall back to the legacy ack
    // envelope so the activation still appears successful.
    match state.kernel.get_prompt_version(&id) {
        Ok(Some(version)) => Json(version).into_response(),
        Ok(None) | Err(_) => Json(serde_json::json!({"success": true})).into_response(),
    }
}

async fn list_experiments(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let agent_id: librefang_types::agent::AgentId = match agent_id.parse() {
        Ok(id) => id,
        Err(e) => {
            return ApiErrorResponse::bad_request(e.to_string())
                .into_json_tuple()
                .into_response()
        }
    };
    let body = match state.kernel.list_experiments(agent_id) {
        Ok(experiments) => {
            let total = experiments.len();
            Json(crate::types::PaginatedResponse {
                items: experiments,
                total,
                offset: 0,
                limit: None,
            })
            .into_response()
        }
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    };
    // #3511: tag response so request_logging middleware can emit `agent_id`.
    crate::extensions::with_agent_id(agent_id, body)
}

async fn create_experiment(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(mut experiment): Json<PromptExperiment>,
) -> impl IntoResponse {
    let agent_id: librefang_types::agent::AgentId = match agent_id.parse() {
        Ok(id) => id,
        Err(e) => {
            return ApiErrorResponse::bad_request(e.to_string())
                .into_json_tuple()
                .into_response()
        }
    };
    experiment.agent_id = agent_id;
    experiment.id = uuid::Uuid::new_v4();
    experiment.created_at = chrono::Utc::now();
    // Audit: `docs/issues/prompt-version-system-prompt-no-cap.md` (same
    // defensive pattern applied to experiments). The state machine —
    // `status`, `started_at`, `ended_at` — is server-owned and can only
    // advance through `/start`, `/pause`, `/complete`. Ignore any
    // client-supplied values on create so an experiment cannot be
    // posted as already-Running with backdated `started_at`.
    experiment.status = librefang_types::agent::ExperimentStatus::default();
    experiment.started_at = None;
    experiment.ended_at = None;
    // Assign IDs to variants
    for variant in &mut experiment.variants {
        variant.id = uuid::Uuid::new_v4();
    }
    let body = match state.kernel.create_experiment(&experiment) {
        // Issue #3832: POST /experiments creates a new resource — 201 Created.
        Ok(_) => (StatusCode::CREATED, Json(experiment)).into_response(),
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    };
    // #3511: tag response so request_logging middleware can emit `agent_id`.
    crate::extensions::with_agent_id(agent_id, body)
}

async fn get_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_experiment(&id) {
        Ok(experiment) => Json(experiment).into_response(),
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}

// Apply a status transition and return the post-mutation `PromptExperiment` so
// callers (dashboard React Query hooks, SDK consumers) can `setQueryData`
// directly instead of doing a follow-up GET. If the experiment vanished
// between the status write and the snapshot read (narrow race — e.g. a
// concurrent delete), fall back to the legacy `{"success": true}` ack so the
// call still appears successful. Refs #3832.
async fn transition_experiment(
    state: &AppState,
    id: &str,
    status: librefang_types::agent::ExperimentStatus,
) -> axum::response::Response {
    if let Err(e) = state.kernel.update_experiment_status(id, status) {
        // #3541: typed KernelOpError → status-code via From impl.
        return ApiErrorResponse::from(e).into_response();
    }
    match state.kernel.get_experiment(id) {
        Ok(Some(experiment)) => Json(experiment).into_response(),
        Ok(None) => Json(serde_json::json!({"success": true})).into_response(),
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}

async fn start_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    transition_experiment(
        &state,
        &id,
        librefang_types::agent::ExperimentStatus::Running,
    )
    .await
}

async fn pause_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    transition_experiment(
        &state,
        &id,
        librefang_types::agent::ExperimentStatus::Paused,
    )
    .await
}

async fn complete_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    transition_experiment(
        &state,
        &id,
        librefang_types::agent::ExperimentStatus::Completed,
    )
    .await
}

async fn get_experiment_metrics(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_experiment_metrics(&id) {
        Ok(metrics) => Json(metrics).into_response(),
        // #3541: PromptStore returns typed `KernelOpError`; route through
        // the central status-code map so a `NotFound { kind: "prompt_version" }`
        // surfaces as 404 instead of being flattened to 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}
