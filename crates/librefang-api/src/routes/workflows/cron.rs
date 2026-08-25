use super::*;

// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------
/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
///
/// Owner-scoping (#6753 follow-up): non-admins can't see cron jobs for agents they don't author — same leak class this PR closed for `/api/triggers`, since `JobMeta`/`CronJob` carries `prompt_template` and other user-authored content.
/// Mirrors `list_triggers` in `triggers.rs`: an explicit `?agent_id=` for an unowned agent returns an empty list rather than 404 (avoids leaking existence), and an unfiltered list is post-filtered down to jobs on agents the caller authors.
#[utoipa::path(get, path = "/api/cron/jobs", tag = "workflows", responses((status = 200, description = "List cron jobs", body = Vec<serde_json::Value>)))]
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let restrict_to: Option<String> = match api_user.as_ref() {
        Some(u) if u.0.role < crate::middleware::UserRole::Admin => Some(u.0.name.clone()),
        _ => None,
    };
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                if !super::super::can_access_agent(&state, aid, api_user.as_ref()) {
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({"jobs": [], "total": 0})),
                    );
                }
                state.kernel.cron().list_jobs(aid)
            }
            Err(_) => {
                return ApiErrorResponse::bad_request("Invalid agent_id").into_json_tuple();
            }
        }
    } else {
        state.kernel.cron().list_all_jobs()
    };
    let jobs: Vec<_> = if let Some(ref user_name) = restrict_to {
        let owned_ids: std::collections::HashSet<AgentId> = state
            .kernel
            .agent_registry()
            .list()
            .iter()
            .filter(|e| e.manifest.author.eq_ignore_ascii_case(user_name))
            .map(|e| e.id)
            .collect();
        jobs.into_iter()
            .filter(|j| owned_ids.contains(&j.agent_id))
            .collect()
    } else {
        jobs
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    // #7744: the job belongs to the authenticated caller, read from the auth
    // extension and never from `body` — which is forwarded to `cron_create`
    // whole, so a body-readable owner would be a caller-chosen owner.
    let owner = api_user
        .as_ref()
        .and_then(|u| u.0.owner_principal())
        .or_else(|| state.kernel.config_ref().default_owner_principal());
    match state
        .kernel
        .cron_create(agent_id, body.clone(), owner)
        .await
    {
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

#[derive(Debug, PartialEq, Eq)]
enum CronJobErrorClass<'a> {
    NotFound,
    BadRequest(&'a str),
    Internal,
}

fn classify_cron_job_error<'a>(
    error: &'a librefang_types::error::LibreFangError,
    job_id: librefang_types::scheduler::CronJobId,
) -> CronJobErrorClass<'a> {
    use librefang_types::error::LibreFangError;

    match error {
        LibreFangError::ResourceNotFound { kind, id }
            if kind.eq_ignore_ascii_case("cron job") && id == &job_id.to_string() =>
        {
            CronJobErrorClass::NotFound
        }
        LibreFangError::Internal(message) if message == &format!("Cron job {job_id} not found") => {
            CronJobErrorClass::NotFound
        }
        LibreFangError::InvalidInput(message) => CronJobErrorClass::BadRequest(message),
        LibreFangError::Internal(message)
            if [
                "Invalid agent_id:",
                "Invalid schedule:",
                "Invalid action:",
                "Invalid delivery:",
                "Invalid delivery_targets:",
            ]
            .iter()
            .any(|prefix| message.starts_with(prefix)) =>
        {
            CronJobErrorClass::BadRequest(message)
        }
        _ => CronJobErrorClass::Internal,
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
        Err(error) if classify_cron_job_error(&error, job_id) == CronJobErrorClass::NotFound => {
            // Idempotent DELETE — the cron job is already gone (replayed
            // request, double-click, or removed by another deleter). Treat
            // as success so clients don't have to special-case 404.
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "already-deleted", "job_id": id})),
            )
        }
        Err(error) => {
            tracing::error!(%job_id, error = %error, "Failed to remove cron job");
            ApiErrorResponse::internal("Failed to delete cron job").into_json_tuple()
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
                Err(error) => match classify_cron_job_error(&error, job_id) {
                    CronJobErrorClass::NotFound => {
                        ApiErrorResponse::not_found("Cron job not found").into_json_tuple()
                    }
                    CronJobErrorClass::BadRequest(message) => {
                        ApiErrorResponse::bad_request(message).into_json_tuple()
                    }
                    CronJobErrorClass::Internal => {
                        tracing::error!(%job_id, error = %error, "Failed to update cron job");
                        ApiErrorResponse::internal("Failed to update cron job").into_json_tuple()
                    }
                },
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta)
                    if super::super::can_access_agent(
                        &state,
                        meta.job.agent_id,
                        api_user.as_ref(),
                    ) =>
                {
                    (
                        StatusCode::OK,
                        Json(cron_job_response_with_metrics(&state, &meta)),
                    )
                }
                _ => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = librefang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron().get_meta(job_id) {
                Some(meta)
                    if super::super::can_access_agent(
                        &state,
                        meta.job.agent_id,
                        api_user.as_ref(),
                    ) =>
                {
                    (
                        StatusCode::OK,
                        Json(cron_job_response_with_metrics(&state, &meta)),
                    )
                }
                _ => ApiErrorResponse::not_found("Job not found").into_json_tuple(),
            }
        }
        Err(_) => ApiErrorResponse::bad_request("Invalid job ID").into_json_tuple(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::error::LibreFangError;

    #[test]
    fn cron_job_error_classification_is_narrow() {
        let job_id = librefang_types::scheduler::CronJobId::new();
        let missing = LibreFangError::Internal(format!("Cron job {job_id} not found"));
        assert_eq!(
            classify_cron_job_error(&missing, job_id),
            CronJobErrorClass::NotFound
        );

        let other_id = librefang_types::scheduler::CronJobId::new();
        assert_eq!(
            classify_cron_job_error(&missing, other_id),
            CronJobErrorClass::Internal,
            "a failure for a different job must not become idempotent success"
        );

        let storage = LibreFangError::Internal("scheduler storage unavailable".to_string());
        assert_eq!(
            classify_cron_job_error(&storage, job_id),
            CronJobErrorClass::Internal
        );
    }

    #[test]
    fn cron_job_update_parse_errors_are_bad_requests() {
        let job_id = librefang_types::scheduler::CronJobId::new();
        for message in [
            "Invalid agent_id: malformed UUID",
            "Invalid schedule: missing field",
            "Invalid action: unknown variant",
            "Invalid delivery: unknown variant",
            "Invalid delivery_targets: expected array",
        ] {
            let error = LibreFangError::Internal(message.to_string());
            assert_eq!(
                classify_cron_job_error(&error, job_id),
                CronJobErrorClass::BadRequest(message)
            );
        }
    }

    #[test]
    fn typed_cron_not_found_is_supported() {
        let job_id = librefang_types::scheduler::CronJobId::new();
        let missing = LibreFangError::ResourceNotFound {
            kind: "Cron job".to_string(),
            id: job_id.to_string(),
        };
        assert_eq!(
            classify_cron_job_error(&missing, job_id),
            CronJobErrorClass::NotFound
        );
    }
}
