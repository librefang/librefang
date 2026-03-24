use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use librefang_types::agent::{PromptExperiment, PromptVersion};

use super::AppState;
use librefang_runtime::kernel_handle::KernelHandle;
use std::sync::Arc;

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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    match state.kernel.list_prompt_versions(agent_id) {
        Ok(versions) => Json(versions).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    version.agent_id = agent_id;
    match state.kernel.create_prompt_version(version.clone()) {
        Ok(_) => Json(version).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn get_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_prompt_version(&id) {
        Ok(version) => Json(version).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn delete_prompt_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.delete_prompt_version(&id) {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "agent_id required in body"})),
            )
                .into_response()
        }
    };
    match state.kernel.set_active_prompt_version(&id, agent_id) {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    match state.kernel.list_experiments(agent_id) {
        Ok(experiments) => Json(experiments).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    experiment.agent_id = agent_id;
    match state.kernel.create_experiment(experiment.clone()) {
        Ok(_) => Json(experiment).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn get_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_experiment(&id) {
        Ok(experiment) => Json(experiment).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn start_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state
        .kernel
        .update_experiment_status(&id, librefang_types::agent::ExperimentStatus::Running)
    {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn pause_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state
        .kernel
        .update_experiment_status(&id, librefang_types::agent::ExperimentStatus::Paused)
    {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn complete_experiment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state
        .kernel
        .update_experiment_status(&id, librefang_types::agent::ExperimentStatus::Completed)
    {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn get_experiment_metrics(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.get_experiment_metrics(&id) {
        Ok(metrics) => Json(metrics).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}
