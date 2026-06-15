use super::*;

// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------
/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
#[utoipa::path(get, path = "/api/cron/jobs", tag = "workflows", responses((status = 200, description = "List cron jobs", body = Vec<serde_json::Value>)))]
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                state.kernel.cron().list_jobs(aid)
            }
            Err(_) => {
                return ApiErrorResponse::bad_request("Invalid agent_id").into_json_tuple();
            }
        }
    } else {
        state.kernel.cron().list_all_jobs()
    };
    let total = jobs.len();
    let jobs_json: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|j| serde_json::to_value(&j).unwrap_or_default())
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({"jobs": jobs_json, "total": total})),
    )
}

/// POST /api/cron/jobs — Create a new cron job.
#[utoipa::path(post, path = "/api/cron/jobs", tag = "workflows", request_body = crate::types::JsonObject, responses((status = 200, description = "Cron job created", body = crate::types::JsonObject)))]
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    match state.kernel.cron_create(agent_id, body.clone()).await {
        Ok(result) => {
            // cron_create returns a JSON string — parse it so the response
            // is a proper JSON object instead of a stringified blob.
            let parsed: serde_json::Value =
                serde_json::from_str(&result).unwrap_or(serde_json::json!({"id": result}));
            (StatusCode::CREATED, Json(parsed))
        }
        // #3541: route structured KernelOpError through the centralized
        // From impl so the status-code contract is consistent across all
        // routes. The earlier inline match mapped `Unavailable` to 500
        // (should be 503) and `Other` to 400 (should be 500), both fixed
        // here because the From impl is the single source of truth.
        Err(e) => ApiErrorResponse::from(e).into_json_tuple(),
    }
}

/// DELETE /api/cron/jobs/{id} — Delete a cron job.
///
/// Idempotent (RFC 9110 §9.2.2): deleting a cron job that is already gone
/// returns `200 OK` with `{"status": "already-deleted"}` instead of `404`.
/// `400` is reserved for the malformed-UUID case alone (Refs #3509). Returns
/// `500` if the in-memory removal succeeds but persistence to disk fails —
/// without persistence, the deletion would silently revert on daemon restart
/// (issue #3515).
#[utoipa::path(
    delete,
    path = "/api/cron/jobs/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Cron job ID")),
    responses(
        (status = 200, description = "Cron job deleted (or was already absent — idempotent)"),
        (status = 400, description = "Malformed cron job ID"),
        (status = 500, description = "Persist failed; change will not survive restart")
    )
)]
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    };
    let job_id = librefang_types::scheduler::CronJobId(uuid);
    match state.kernel.cron().remove_job(job_id) {
        Ok(_) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::error!("Failed to persist cron scheduler state after delete: {e}");
                return cron_persist_failed_response("delete", &e.to_string());
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "deleted", "job_id": id})),
            )
        }
        Err(_) => {
            // Idempotent DELETE — the cron job is already gone (replayed
            // request, double-click, or removed by another deleter). Treat
            // as success so clients don't have to special-case 404.
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "already-deleted", "job_id": id})),
            )
        }
    }
}

/// PUT /api/cron/jobs/{id} — Update a cron job's configuration.
///
/// Returns 500 if the in-memory update succeeds but persistence to disk
/// fails — without persistence, the new schedule runs in-memory until the
/// next restart, then silently reverts to the old schedule (issue #3515).
#[utoipa::path(put, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Cron job updated", body = crate::types::JsonObject), (status = 500, description = "Persist failed; change will not survive restart")))]
pub async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().update_job(job_id, &body) {
                Ok(job) => {
                    if let Err(e) = state.kernel.cron().persist() {
                        tracing::error!("Failed to persist cron scheduler state after update: {e}");
                        return cron_persist_failed_response("update", &e.to_string());
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::to_value(&job).unwrap_or_default()),
                    )
                }
                // SSRF / shape rejections from `validate_cron_delivery*`
                // surface as `InvalidInput` and must map to 400, not the
                // catch-all 404 (#4732). 404 here would silently mask a
                // refused webhook host as "schedule not found", letting
                // attacker-controlled clients confuse the failure mode.
                Err(librefang_types::error::LibreFangError::InvalidInput(msg)) => {
                    ApiErrorResponse::bad_request(msg).into_json_tuple()
                }
                Err(e) => ApiErrorResponse::not_found(format!("{e}")).into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}

/// PUT /api/cron/jobs/{id}/enable — Enable or disable a cron job.
///
/// Returns 500 if the in-memory toggle succeeds but persistence to disk
/// fails — without persistence, the new enabled state would silently
/// revert on daemon restart (issue #3515).
#[utoipa::path(put, path = "/api/cron/jobs/{id}/enable", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Cron job toggled", body = crate::types::JsonObject), (status = 500, description = "Persist failed; change will not survive restart")))]
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().set_enabled(job_id, enabled) {
                Ok(()) => {
                    if let Err(e) = state.kernel.cron().persist() {
                        tracing::error!("Failed to persist cron scheduler state after toggle: {e}");
                        return cron_persist_failed_response("toggle", &e.to_string());
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"id": id, "enabled": enabled})),
                    )
                }
                Err(e) => ApiErrorResponse::not_found(format!("{e}")).into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}

/// GET /api/cron/jobs/{id} — Get a single cron job by ID.
///
/// Response carries the cron `JobMeta` plus two #3693 observability
/// fields:
/// - `session_message_count` (`usize`): messages in the persistent
///   `(agent, "cron")` session.
/// - `session_token_count` (`u64`): kernel-estimated tokens for those
///   messages (system prompt and tools excluded — same accounting as
///   the prune path).
#[utoipa::path(get, path = "/api/cron/jobs/{id}", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job details", body = crate::types::JsonObject), (status = 404, description = "Job not found")))]
pub async fn get_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(cron_job_response_with_metrics(&state, &meta)),
                ),
                None => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}

/// GET /api/cron/jobs/{id}/status — Get status of a specific cron job.
///
/// Same response shape as `GET /api/cron/jobs/{id}`, including the
/// #3693 `session_message_count` / `session_token_count` fields.
#[utoipa::path(get, path = "/api/cron/jobs/{id}/status", tag = "workflows", params(("id" = String, Path, description = "Cron job ID")), responses((status = 200, description = "Cron job status", body = crate::types::JsonObject)))]
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(cron_job_response_with_metrics(&state, &meta)),
                ),
                None => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}
