use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use librefang_types::agent::{PromptExperiment, PromptVersion};
use sha2::{Digest, Sha256};

use super::AppState;
use librefang_runtime::kernel_handle::KernelHandle;
use std::sync::Arc;

use crate::types::ApiErrorResponse;
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
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
    match state.kernel.list_prompt_versions(agent_id) {
        Ok(versions) => Json(versions).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
    }
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
    version.agent_id = agent_id;
    version.id = uuid::Uuid::new_v4();
    version.created_at = chrono::Utc::now();
    // Compute content hash from system_prompt
    let mut hasher = Sha256::new();
    hasher.update(version.system_prompt.as_bytes());
    version.content_hash = format!("{:x}", hasher.finalize());
    match state.kernel.create_prompt_version(version.clone()) {
        Ok(_) => Json(version).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
    }
}

async fn get_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_prompt_version(&id) {
        Ok(version) => Json(version).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
    }
}

async fn delete_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.delete_prompt_version(&id) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
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
    match state.kernel.set_active_prompt_version(&id, agent_id) {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
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
    match state.kernel.list_experiments(agent_id) {
        Ok(experiments) => Json(experiments).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
    }
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
    // Assign IDs to variants
    for variant in &mut experiment.variants {
        variant.id = uuid::Uuid::new_v4();
    }
    match state.kernel.create_experiment(experiment.clone()) {
        Ok(_) => Json(experiment).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
    }
}

async fn get_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_experiment(&id) {
        Ok(experiment) => Json(experiment).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
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
        return ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response();
    }
    match state.kernel.get_experiment(id) {
        Ok(Some(experiment)) => Json(experiment).into_response(),
        Ok(None) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
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
        Err(e) => ApiErrorResponse::internal(e)
            .into_json_tuple()
            .into_response(),
    }
}
