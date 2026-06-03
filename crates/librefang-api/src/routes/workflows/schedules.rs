use super::*;

/// GET /api/schedules — List all scheduled jobs.
///
/// Envelope is the canonical `PaginatedResponse{items,total,offset,limit}`
/// (#3842) so the generated SDK can share one list-response type across all
/// list endpoints. The legacy `schedules` key was renamed to `items`; offset
/// is always 0 and limit is null because this endpoint returns the full set.
#[utoipa::path(
    get,
    path = "/api/schedules",
    tag = "workflows",
    responses(
        (status = 200, description = "List schedules", body = crate::types::JsonObject)
    )
)]
pub async fn list_schedules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let jobs = state.kernel.cron().list_all_jobs();
    let schedules: Vec<serde_json::Value> = jobs.iter().map(cron_job_to_schedule_json).collect();
    let total = schedules.len();
    Json(crate::types::PaginatedResponse {
        items: schedules,
        total,
        offset: 0,
        limit: None,
    })
}

/// GET /api/schedules/{id} — Get a specific schedule by ID.
#[utoipa::path(get, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule details", body = crate::types::JsonObject)))]
pub async fn get_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };
    match state.kernel.cron().get_job(job_id) {
        Some(job) => (StatusCode::OK, Json(cron_job_to_schedule_json(&job))),
        None => ApiErrorResponse::not_found(format!("Schedule '{id}' not found")).into_json_tuple(),
    }
}

/// POST /api/schedules — Create a new scheduled job (backed by CronScheduler).
#[utoipa::path(
    post,
    path = "/api/schedules",
    tag = "workflows",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Schedule created", body = crate::types::JsonObject),
        (status = 400, description = "Invalid schedule definition")
    )
)]
pub async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match req["name"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing 'name' field").into_json_tuple();
        }
    };

    let cron = match req["cron"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing 'cron' field").into_json_tuple();
        }
    };

    // Validate cron expression: must be 5 space-separated fields
    let cron_parts: Vec<&str> = cron.split_whitespace().collect();
    if cron_parts.len() != 5 {
        return ApiErrorResponse::bad_request(
            "Invalid cron expression: must have 5 fields (min hour dom mon dow)",
        )
        .into_json_tuple();
    }

    let agent_id_str = req["agent_id"].as_str().unwrap_or("").to_string();
    let workflow_id_str = req["workflow_id"].as_str().unwrap_or("").to_string();

    // Must have either agent_id or workflow_id
    if agent_id_str.is_empty() && workflow_id_str.is_empty() {
        return ApiErrorResponse::bad_request("Must provide either agent_id or workflow_id")
            .into_json_tuple();
    }

    // Resolve agent_id to a UUID
    let resolved_agent_id = if !agent_id_str.is_empty() {
        if let Ok(aid) = agent_id_str.parse::<AgentId>() {
            if state.kernel.agent_registry().get(aid).is_some() {
                aid
            } else {
                return ApiErrorResponse::not_found(format!("Agent not found: {agent_id_str}"))
                    .into_json_tuple();
            }
        } else if let Some(agent) = state
            .kernel
            .agent_registry()
            .list()
            .iter()
            .find(|a| a.name == agent_id_str)
        {
            agent.id
        } else {
            return ApiErrorResponse::not_found(format!("Agent not found: {agent_id_str}"))
                .into_json_tuple();
        }
    } else {
        // For workflow-only schedules, use a system agent ID
        AgentId(uuid::Uuid::from_bytes([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]))
    };

    // Validate workflow exists if provided
    if !workflow_id_str.is_empty() {
        if let Ok(wid) = workflow_id_str.parse::<uuid::Uuid>() {
            if state
                .kernel
                .workflow_engine()
                .get_workflow(WorkflowId(wid))
                .await
                .is_none()
            {
                return ApiErrorResponse::not_found(format!(
                    "Workflow not found: {workflow_id_str}"
                ))
                .into_json_tuple();
            }
        } else {
            return ApiErrorResponse::bad_request("Invalid workflow_id format").into_json_tuple();
        }
    }

    let message = req["message"].as_str().unwrap_or("").to_string();
    let tz = req["tz"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    // Validate timezone string if provided
    if let Some(ref tz_str) = tz {
        if tz_str != "UTC" && tz_str.parse::<chrono_tz::Tz>().is_err() {
            return ApiErrorResponse::bad_request(format!(
                "Invalid timezone '{tz_str}'. Use IANA format (e.g. 'America/New_York', 'Europe/Rome')"
            ))
            .into_json_tuple();
        }
    }

    // Build the CronJob action
    let action = if !workflow_id_str.is_empty() {
        librefang_types::scheduler::CronAction::Workflow {
            workflow_id: workflow_id_str,
            input: if message.is_empty() {
                None
            } else {
                Some(message)
            },
            timeout_secs: None,
        }
    } else {
        let msg = if message.is_empty() {
            format!("[Scheduled task '{}' triggered]", name)
        } else {
            message
        };
        librefang_types::scheduler::CronAction::AgentTurn {
            message: msg,
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        }
    };

    // Optional fan-out delivery targets. Validated up front so a bad shape
    // returns a 400 rather than silently dropping targets later.
    let delivery_targets: Vec<librefang_types::scheduler::CronDeliveryTarget> =
        match req.get("delivery_targets") {
            Some(serde_json::Value::Null) | None => Vec::new(),
            Some(v) => match serde_json::from_value(v.clone()) {
                Ok(t) => t,
                Err(e) => {
                    return ApiErrorResponse::bad_request(format!("Invalid delivery_targets: {e}"))
                        .into_json_tuple();
                }
            },
        };

    let job = librefang_types::scheduler::CronJob {
        id: librefang_types::scheduler::CronJobId::new(),
        agent_id: resolved_agent_id,
        name,
        enabled: req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        schedule: librefang_types::scheduler::CronSchedule::Cron { expr: cron, tz },
        action,
        delivery: librefang_types::scheduler::CronDelivery::None,
        delivery_targets,
        peer_id: None,
        session_mode: req["session_mode"]
            .as_str()
            .and_then(|s| serde_json::from_value(serde_json::Value::String(s.to_string())).ok()),
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };

    match state.kernel.cron().add_job(job.clone(), false) {
        Ok(job_id) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            let mut entry = cron_job_to_schedule_json(&job);
            entry["id"] = serde_json::Value::String(job_id.to_string());
            (StatusCode::CREATED, Json(entry))
        }
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}

/// PUT /api/schedules/:id — Update a scheduled job (toggle enabled, edit fields).
#[utoipa::path(put, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Schedule updated", body = crate::types::JsonObject)))]
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };

    // Build update payload compatible with CronScheduler::update_job
    let mut updates = serde_json::Map::new();
    if let Some(enabled) = req.get("enabled") {
        updates.insert("enabled".to_string(), enabled.clone());
    }
    if let Some(name) = req.get("name") {
        updates.insert("name".to_string(), name.clone());
    }
    // Read tz from the request (if provided).  When the caller sends
    // a new `cron` expression we must carry over the timezone — otherwise
    // replacing the entire schedule object would reset tz to null.
    let req_tz = req
        .get("tz")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Validate timezone string if provided
    if let Some(ref tz_str) = req_tz {
        if tz_str != "UTC" && tz_str.parse::<chrono_tz::Tz>().is_err() {
            return ApiErrorResponse::bad_request(format!(
                "Invalid timezone '{tz_str}'. Use IANA format (e.g. 'America/New_York', 'Europe/Rome')"
            ))
            .into_json_tuple();
        }
    }

    if let Some(cron) = req.get("cron").and_then(|v| v.as_str()) {
        let cron_parts: Vec<&str> = cron.split_whitespace().collect();
        if cron_parts.len() != 5 {
            return ApiErrorResponse::bad_request("Invalid cron expression").into_json_tuple();
        }
        // If tz not in this request, preserve the existing tz from the job.
        let tz = req_tz.clone().or_else(|| {
            state.kernel.cron().get_meta(job_id).and_then(|meta| {
                if let librefang_types::scheduler::CronSchedule::Cron { tz, .. } =
                    &meta.job.schedule
                {
                    tz.clone()
                } else {
                    None
                }
            })
        });
        updates.insert(
            "schedule".to_string(),
            serde_json::json!({"kind": "cron", "expr": cron, "tz": tz}),
        );
    } else if req_tz.is_some() {
        // Caller wants to change only the timezone — read current cron expr.
        if let Some(meta) = state.kernel.cron().get_meta(job_id) {
            if let librefang_types::scheduler::CronSchedule::Cron { expr, .. } = &meta.job.schedule
            {
                updates.insert(
                    "schedule".to_string(),
                    serde_json::json!({"kind": "cron", "expr": expr, "tz": req_tz}),
                );
            }
        }
    }
    if let Some(agent_id) = req.get("agent_id") {
        updates.insert("agent_id".to_string(), agent_id.clone());
    }
    // Multi-destination fan-out targets: full replacement when supplied.
    // Validation is done on the kernel side via serde, but reject obviously
    // malformed payloads (non-array) up front to give a clearer 400.
    //
    // Semantics intentionally differ between `null` and `[]`:
    //   * field omitted        — leave existing targets untouched.
    //   * `delivery_targets:null` — same as omitted (preserves the
    //     existing list). The kernel `update_job` checks `is_null()` and
    //     skips the patch.
    //   * `delivery_targets:[]` — explicit clear; kernel deserializes the
    //     empty array and replaces the list with `Vec::new()`.
    // Callers that want to clear all targets must send `[]`, not `null`.
    if let Some(targets) = req.get("delivery_targets") {
        if !targets.is_null() && !targets.is_array() {
            return ApiErrorResponse::bad_request(
                "delivery_targets must be an array of CronDeliveryTarget objects",
            )
            .into_json_tuple();
        }
        updates.insert("delivery_targets".to_string(), targets.clone());
    }

    match state
        .kernel
        .cron()
        .update_job(job_id, &serde_json::Value::Object(updates))
    {
        Ok(_job) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "updated", "schedule_id": id})),
            )
        }
        // SSRF / shape rejections must map to 400, not the catch-all 404
        // — see the parallel branch in `update_cron_job` (#4732).
        Err(librefang_types::error::LibreFangError::InvalidInput(msg)) => {
            ApiErrorResponse::bad_request(msg).into_json_tuple()
        }
        Err(e) => ApiErrorResponse::not_found(format!("Schedule not found: {e}")).into_json_tuple(),
    }
}

/// DELETE /api/schedules/:id — Remove a scheduled job.
#[utoipa::path(delete, path = "/api/schedules/{id}", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule deleted")))]
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };

    match state.kernel.cron().remove_job(job_id) {
        Ok(_) => {
            if let Err(e) = state.kernel.cron().persist() {
                tracing::warn!("Failed to persist cron jobs: {e}");
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "removed", "schedule_id": id})),
            )
        }
        Err(e) => ApiErrorResponse::not_found(format!("Schedule not found: {e}")).into_json_tuple(),
    }
}

/// POST /api/schedules/:id/run — Manually trigger a scheduled job now.
#[utoipa::path(post, path = "/api/schedules/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Schedule ID")), responses((status = 200, description = "Schedule triggered", body = crate::types::JsonObject)))]
pub async fn run_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(jid) => jid,
        Err(e) => return e,
    };

    let job = match state.kernel.cron().get_job(job_id) {
        Some(j) => j,
        None => {
            return ApiErrorResponse::not_found("Schedule not found").into_json_tuple();
        }
    };

    let name = job.name.clone();
    let agent_id = job.agent_id;

    match &job.action {
        librefang_types::scheduler::CronAction::Workflow {
            workflow_id, input, ..
        } => {
            let wid = match workflow_id.parse::<uuid::Uuid>() {
                Ok(u) => WorkflowId(u),
                Err(_) => {
                    return ApiErrorResponse::bad_request("Invalid workflow_id").into_json_tuple();
                }
            };
            let wf_input = input
                .clone()
                .unwrap_or_else(|| format!("[Scheduled workflow '{}' triggered]", name));
            match state.kernel.run_workflow_typed(wid, wf_input).await {
                Ok((run_id, output)) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "completed",
                        "schedule_id": id,
                        "workflow_id": workflow_id,
                        "run_id": run_id.to_string(),
                        "output": output,
                    })),
                ),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "status": "failed",
                        "schedule_id": id,
                        "error": format!("{e}"),
                    })),
                ),
            }
        }
        librefang_types::scheduler::CronAction::AgentTurn { message, .. } => {
            let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone();
            match state
                .kernel
                .send_message_with_handle(agent_id, message, Some(kernel_handle))
                .await
            {
                Ok(result) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "completed",
                        "schedule_id": id,
                        "agent_id": agent_id.to_string(),
                        "response": result.response,
                    })),
                ),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "status": "failed",
                        "schedule_id": id,
                        "error": format!("{e}"),
                    })),
                ),
            }
        }
        librefang_types::scheduler::CronAction::SystemEvent { text } => {
            // Fire-and-forget system event
            let event = librefang_types::event::Event::new(
                AgentId::new(),
                librefang_types::event::EventTarget::Broadcast,
                librefang_types::event::EventPayload::Custom(text.as_bytes().to_vec()),
            );
            state.kernel.publish_typed_event(event).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "schedule_id": id,
                    "type": "system_event",
                })),
            )
        }
    }
}
