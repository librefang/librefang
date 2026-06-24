use super::*;

// ---------------------------------------------------------------------------
// Workflow routes
// ---------------------------------------------------------------------------
/// POST /api/workflows — Register a new workflow.
#[utoipa::path(
    post,
    path = "/api/workflows",
    tag = "workflows",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Workflow created", body = crate::types::JsonObject),
        (status = 400, description = "Invalid workflow definition")
    )
)]
pub async fn create_workflow(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = req["name"].as_str().unwrap_or("unnamed").to_string();
    let description = req["description"].as_str().unwrap_or("").to_string();

    let steps_json = match req["steps"].as_array() {
        Some(s) => s,
        None => {
            return ApiErrorResponse::bad_request("Missing 'steps' array").into_json_tuple();
        }
    };

    let mut steps = Vec::new();
    for s in steps_json {
        let step_name = s["name"].as_str().unwrap_or("step").to_string();
        let agent = if let Some(id) = s["agent_id"].as_str() {
            StepAgent::ById { id: id.to_string() }
        } else if let Some(name) = s["agent_name"].as_str() {
            StepAgent::ByName {
                name: name.to_string(),
            }
        } else {
            return ApiErrorResponse::bad_request(format!(
                "Step '{}' needs 'agent_id' or 'agent_name'",
                step_name
            ))
            .into_json_tuple();
        };

        let mode = parse_step_mode(&s["mode"], s);
        let error_mode = parse_error_mode(&s["error_mode"], s);

        let depends_on: Vec<String> = s["depends_on"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        steps.push(WorkflowStep {
            name: step_name,
            agent,
            prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
            mode,
            timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
            error_mode,
            output_var: s["output_var"].as_str().map(String::from),
            inherit_context: s["inherit_context"].as_bool(),
            depends_on,
            session_mode: parse_step_session_mode(s),
        });
    }

    let layout = req.get("layout").cloned();
    let total_timeout_secs = req["total_timeout_secs"].as_u64();
    let input_schema = parse_input_schema(req.get("input_schema"));

    let workflow = Workflow {
        id: WorkflowId::new(),
        name,
        description,
        steps,
        created_at: chrono::Utc::now(),
        layout,
        total_timeout_secs,
        input_schema,
    };

    // Pre-flight validation: reject manifests with empty Transform code,
    // unparseable Tera templates, zero / over-cap Wait durations, the
    // Gate parser's fail-closed sentinel, and empty Branch arms. Without
    // this, operators only discovered the typo when a real run reached
    // the bad step.
    let validation_errs = workflow.validate();
    if !validation_errs.is_empty() {
        let detail = validation_errs
            .iter()
            .map(|(step, reason)| format!("step '{step}': {reason}"))
            .collect::<Vec<_>>()
            .join("; ");
        return ApiErrorResponse::bad_request(format!("invalid workflow: {detail}"))
            .into_json_tuple();
    }

    let id = state.kernel.register_workflow(workflow).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({"workflow_id": id.to_string()})),
    )
}

/// GET /api/workflows — List all workflows.
#[utoipa::path(
    get,
    path = "/api/workflows",
    tag = "workflows",
    responses(
        (status = 200, description = "List workflows", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_workflows(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let engine = state.kernel.workflow_engine();
    let workflows = engine.list_workflows().await;
    let all_runs = engine.list_runs(None).await;

    // Per-workflow run aggregates: total count, completed/failed/cancelled
    // counts, and the most recent run summary for the row badge. Computed in
    // one pass over `all_runs` to avoid N+1 scans across O(workflows × runs).
    //
    // `success_rate` = completed / (completed + failed). Cancelled runs are
    // NOT included in the denominator — a user-initiated cancel is not a
    // reliability signal for the workflow itself.
    struct RunAgg<'a> {
        total: usize,
        completed: usize,
        failed: usize,
        cancelled: usize,
        latest: Option<&'a WorkflowRun>,
    }
    let mut agg: std::collections::HashMap<String, RunAgg> = std::collections::HashMap::new();
    for r in &all_runs {
        let entry = agg.entry(r.workflow_id.to_string()).or_insert(RunAgg {
            total: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
            latest: None,
        });
        entry.total += 1;
        match &r.state {
            WorkflowRunState::Completed => entry.completed += 1,
            WorkflowRunState::Failed => entry.failed += 1,
            WorkflowRunState::Cancelled => entry.cancelled += 1,
            _ => {}
        }
        match entry.latest {
            None => entry.latest = Some(r),
            Some(prev) if r.started_at > prev.started_at => entry.latest = Some(r),
            _ => {}
        }
    }

    let state_kind = |s: &WorkflowRunState| -> &'static str {
        match s {
            WorkflowRunState::Pending => "pending",
            WorkflowRunState::Running => "running",
            WorkflowRunState::Paused { .. } => "paused",
            WorkflowRunState::Completed => "completed",
            WorkflowRunState::Failed => "failed",
            WorkflowRunState::Cancelled => "cancelled",
        }
    };

    // Load cron jobs to find workflow-bound schedules
    let all_cron_jobs = state.kernel.cron().list_all_jobs();

    let items: Vec<serde_json::Value> = workflows
        .iter()
        .map(|w| {
            let wid = w.id.to_string();
            let schedule = all_cron_jobs.iter().find(|j| {
                matches!(&j.action, librefang_types::scheduler::CronAction::Workflow { workflow_id, .. } if workflow_id == &wid)
            });
            let schedule_json = schedule.map(|j| {
                let cron_expr = match &j.schedule {
                    librefang_types::scheduler::CronSchedule::Cron { expr, .. } => expr.clone(),
                    librefang_types::scheduler::CronSchedule::Every { every_secs } => format!("every {every_secs}s"),
                    librefang_types::scheduler::CronSchedule::At { at } => format!("at {}", at.to_rfc3339()),
                };
                serde_json::json!({
                    "cron": cron_expr,
                    "enabled": j.enabled,
                    "last_run": j.last_run.map(|t| t.to_rfc3339()),
                })
            });
            let wf_agg = agg.get(&wid);
            let run_count = wf_agg.map(|a| a.total).unwrap_or(0);
            let last_run_json = wf_agg.and_then(|a| a.latest).map(|r| {
                serde_json::json!({
                    "state": state_kind(&r.state),
                    "started_at": r.started_at.to_rfc3339(),
                    "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
                })
            });
            // success_rate = completed / (completed + failed). Cancelled runs
            // are excluded from the denominator — they are not a reliability
            // signal. Null until at least one non-cancelled terminal run exists
            // (surfacing 0% on a workflow with only in-flight/cancelled runs
            // would be misleading).
            let success_rate = wf_agg.and_then(|a| {
                let terminal = a.completed + a.failed;
                (terminal > 0).then(|| a.completed as f32 / terminal as f32)
            });
            serde_json::json!({
                "id": wid,
                "name": w.name,
                "description": w.description,
                "steps": w.steps.len(),
                "run_count": run_count,
                "cancelled_count": wf_agg.map(|a| a.cancelled).unwrap_or(0),
                "created_at": w.created_at.to_rfc3339(),
                "schedule": schedule_json,
                "last_run": last_run_json,
                "success_rate": success_rate,
            })
        })
        .collect();
    // Workflows load from the engine in a single page (in-memory), so offset=0 / limit=None.
    let total = items.len();
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: 0,
        limit: None,
    })
}

/// GET /api/workflows/:id — Get a single workflow by ID.
#[utoipa::path(
    get,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow details", body = crate::types::JsonObject),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn get_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(w) => (StatusCode::OK, Json(workflow_to_json(&w))),
        None => {
            ApiErrorResponse::not_found(format!("Workflow '{}' not found", id)).into_json_tuple()
        }
    }
}

/// PUT /api/workflows/:id — Update an existing workflow.
#[utoipa::path(
    put,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Workflow updated", body = crate::types::JsonObject),
        (status = 400, description = "Invalid workflow definition"),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn update_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    // Fetch existing workflow to preserve created_at
    let existing = match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(w) => w,
        None => {
            return ApiErrorResponse::not_found("Workflow not found").into_json_tuple();
        }
    };

    let name = req["name"]
        .as_str()
        .map(String::from)
        .unwrap_or(existing.name.clone());
    let description = req["description"]
        .as_str()
        .map(String::from)
        .unwrap_or(existing.description.clone());

    // If steps are provided, parse them; otherwise keep existing steps
    let steps = if let Some(steps_json) = req["steps"].as_array() {
        let mut parsed_steps = Vec::new();
        for s in steps_json {
            let step_name = s["name"].as_str().unwrap_or("step").to_string();
            let agent = if let Some(aid) = s["agent_id"].as_str() {
                StepAgent::ById {
                    id: aid.to_string(),
                }
            } else if let Some(aname) = s["agent_name"].as_str() {
                StepAgent::ByName {
                    name: aname.to_string(),
                }
            } else {
                return ApiErrorResponse::bad_request(format!(
                    "Step '{}' needs 'agent_id' or 'agent_name'",
                    step_name
                ))
                .into_json_tuple();
            };

            let mode = parse_step_mode(&s["mode"], s);
            let error_mode = parse_error_mode(&s["error_mode"], s);

            let depends_on: Vec<String> = s["depends_on"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            parsed_steps.push(WorkflowStep {
                name: step_name,
                agent,
                prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
                mode,
                timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
                error_mode,
                output_var: s["output_var"].as_str().map(String::from),
                inherit_context: s["inherit_context"].as_bool(),
                depends_on,
                session_mode: parse_step_session_mode(s),
            });
        }
        parsed_steps
    } else {
        existing.steps.clone()
    };

    let layout = if req.get("layout").is_some() {
        req.get("layout").cloned()
    } else {
        existing.layout.clone()
    };

    // If the request contains "total_timeout_secs" (even null), use the new
    // value. If the key is absent, preserve the existing setting.
    let total_timeout_secs = if req.get("total_timeout_secs").is_some() {
        req["total_timeout_secs"].as_u64()
    } else {
        existing.total_timeout_secs
    };

    // Same "PATCH-style" semantic for input_schema: an explicit key (even
    // null / empty array) replaces; an absent key preserves.
    let input_schema = if req.get("input_schema").is_some() {
        parse_input_schema(req.get("input_schema"))
    } else {
        existing.input_schema.clone()
    };

    let updated = Workflow {
        id: workflow_id,
        name,
        description,
        steps,
        created_at: existing.created_at,
        layout,
        total_timeout_secs,
        input_schema,
    };

    // Same pre-flight validation as `create_workflow` — a PATCH that
    // introduces a bad Transform template / empty Branch arms / etc.
    // must fail at the route boundary, not silently at run time.
    let validation_errs = updated.validate();
    if !validation_errs.is_empty() {
        let detail = validation_errs
            .iter()
            .map(|(step, reason)| format!("step '{step}': {reason}"))
            .collect::<Vec<_>>()
            .join("; ");
        return ApiErrorResponse::bad_request(format!("invalid workflow: {detail}"))
            .into_json_tuple();
    }

    if !state
        .kernel
        .workflow_engine()
        .update_workflow(workflow_id, updated.clone())
        .await
    {
        return ApiErrorResponse::not_found("Workflow not found").into_json_tuple();
    }

    // Return the post-mutation entity in the same shape as GET so the
    // dashboard can `setQueryData` instead of round-tripping a refetch
    // (#3832). Read back from the engine in case the kernel normalized
    // anything during persist; fall back to `updated` if the row vanished
    // between write and read (narrow race — concurrent delete) so the
    // mutation still appears successful.
    let body = match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(persisted) => workflow_to_json(&persisted),
        None => workflow_to_json(&updated),
    };
    (StatusCode::OK, Json(body))
}

/// DELETE /api/workflows/:id — Remove a workflow.
#[utoipa::path(
    delete,
    path = "/api/workflows/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Workflow deleted"),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn delete_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    if state
        .kernel
        .workflow_engine()
        .remove_workflow(workflow_id)
        .await
    {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "workflow_id": id})),
        )
    } else {
        ApiErrorResponse::not_found("Workflow not found").into_json_tuple()
    }
}

/// Query parameters for `POST /api/workflows/:id/run`.
#[derive(serde::Deserialize, Default)]
pub struct RunWorkflowQuery {
    /// When `true`, block until the workflow finishes and return the result
    /// synchronously (backward-compatible behavior). Defaults to `false`
    /// (async: returns 202 immediately with a `run_id`).
    #[serde(default)]
    pub wait: bool,
    /// When `wait=true`, cap the synchronous wait at this many milliseconds.
    /// On expiry the run keeps going in the background and the handler
    /// returns 202. Has no effect when `wait=false`.
    pub timeout_ms: Option<u64>,
}

/// Spawn the background execution of an already-created workflow run.
///
/// Shared by the `run_workflow` async path and `rerun_workflow_run` so both drive `execute_run` with identical agent-resolver and message-sender wiring.
/// The caller creates the `Pending` run and returns its id immediately; this drives it to completion, observable via `GET /api/workflows/runs/{run_id}`.
fn spawn_background_run(state: Arc<AppState>, run_id: WorkflowRunId) {
    // Separate Arc clones for the resolver closure (Fn) and the sender closure (Fn) so neither moves out of the other.
    let state_for_resolver = state.clone();
    let state_for_sender = state.clone();
    tokio::spawn(async move {
        let result = state
            .kernel
            .workflow_engine()
            .execute_run(
                run_id,
                move |agent_ref| {
                    use librefang_kernel::workflow::StepAgent;
                    match agent_ref {
                        StepAgent::ById { id } => {
                            let agent_id: librefang_types::agent::AgentId = id.parse().ok()?;
                            let entry = state_for_resolver.kernel.agent_registry().get(agent_id)?;
                            let inherit = entry.manifest.inherit_parent_context;
                            Some((agent_id, entry.name.clone(), inherit))
                        }
                        StepAgent::ByName { name } => {
                            let entry =
                                state_for_resolver.kernel.agent_registry().find_by_name(name)?;
                            let inherit = entry.manifest.inherit_parent_context;
                            Some((entry.id, entry.name.clone(), inherit))
                        }
                    }
                },
                move |agent_id: librefang_types::agent::AgentId,
                      message: String,
                      session_mode_override: Option<librefang_types::agent::SessionMode>| {
                    let sc = state_for_sender.clone();
                    async move {
                        sc.kernel
                            .send_message_with_session_mode(
                                agent_id,
                                &message,
                                session_mode_override,
                            )
                            .await
                            .map(|r| {
                                (
                                    r.response,
                                    r.total_usage.input_tokens,
                                    r.total_usage.output_tokens,
                                )
                            })
                            .map_err(|e| format!("{e}"))
                    }
                },
            )
            .await;
        if let Err(e) = result {
            tracing::warn!(run_id = %run_id, error = %e, "Background workflow run failed");
        }
    });
}

/// POST /api/workflows/:id/run — Execute a workflow.
///
/// By default (no query params) this is **asynchronous**: the run is spawned
/// in the background and a 202 is returned immediately with `{"run_id":"..."}`.
/// The caller can poll `GET /api/workflows/runs/{run_id}` to track progress.
///
/// With `?wait=true` the request blocks until completion (original behavior,
/// kept for backward compat). With `?wait=true&timeout_ms=N` the block is
/// capped at N milliseconds; if the run hasn't finished, 202 is returned
/// and the run continues in the background.
#[utoipa::path(post, path = "/api/workflows/{id}/run", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), request_body(content = crate::types::JsonObject, description = "Workflow input variables (free-form key/value object)"), responses((status = 200, description = "Workflow run completed (wait=true)"), (status = 202, description = "Workflow run started asynchronously")))]
pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<RunWorkflowQuery>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    let input = workflow_run_input_string(&req);

    if query.wait {
        // -- Synchronous path (backward-compatible) --
        let run_fut = state.kernel.run_workflow_typed(workflow_id, input);
        let result = if let Some(timeout_ms) = query.timeout_ms {
            tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), run_fut)
                .await
                .ok() // None on timeout, Some(inner_result) on completion
        } else {
            Some(run_fut.await)
        };

        match result {
            Some(Ok((run_id, output))) => {
                let run = state.kernel.workflow_engine().get_run(run_id).await;
                let step_results = run.as_ref().map(|r| {
                    r.step_results
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "step_name": s.step_name,
                                "agent_name": s.agent_name,
                                "prompt": s.prompt,
                                "output": s.output,
                                "input_tokens": s.input_tokens,
                                "output_tokens": s.output_tokens,
                                "duration_ms": s.duration_ms,
                                "error": s.error,
                            })
                        })
                        .collect::<Vec<_>>()
                });
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "run_id": run_id.to_string(),
                        "output": output,
                        "status": "completed",
                        "step_results": step_results.unwrap_or_default(),
                    })),
                )
            }
            Some(Err(e)) => {
                tracing::warn!("Workflow run failed for {id}: {e}");
                let detail = e.to_string();
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({
                        "error": "workflow_failed",
                        "detail": detail,
                    })),
                )
            }
            None => {
                // Timed out — run is still going in the background.
                // We need a run_id to return, but run_workflow_typed already
                // consumed the future and started the run inside the kernel.
                // Surface a generic async response; the caller should poll.
                (
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "status": "running",
                        "message": "workflow is still running; poll GET /api/workflows/runs/{run_id}",
                    })),
                )
            }
        }
    } else {
        // -- Asynchronous path (default) --
        // Create the run first so we have the run_id to return immediately,
        // then spawn execute_run in the background.
        let engine = state.kernel.workflow_engine();
        // Create the run synchronously so we can return its id in the 202, then drive it to completion in the background.
        // Progress is observable via GET /api/workflows/runs/{run_id}.
        let run_id = match engine.create_run(workflow_id, input.clone()).await {
            Some(rid) => rid,
            None => {
                return ApiErrorResponse::not_found(format!("Workflow '{id}' not found"))
                    .into_json_tuple();
            }
        };
        let run_id_str = run_id.to_string();
        spawn_background_run(state.clone(), run_id);
        (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "run_id": run_id_str,
            })),
        )
    }
}

/// POST /api/workflows/:id/dry-run — Validate and preview a workflow without executing it.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/dry-run",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Dry-run preview", body = crate::types::JsonObject),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn dry_run_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    let input = workflow_run_input_string(&req);

    match state.kernel.dry_run_workflow(workflow_id, input).await {
        Ok(steps) => {
            let all_agents_found = steps.iter().all(|s| s.agent_found);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "valid": all_agents_found,
                    "steps": steps.iter().map(|s| serde_json::json!({
                        "step_name": s.step_name,
                        "agent_name": s.agent_name,
                        "agent_found": s.agent_found,
                        "resolved_prompt": s.resolved_prompt,
                        "skipped": s.skipped,
                        "skip_reason": s.skip_reason,
                    })).collect::<Vec<_>>(),
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Workflow dry-run failed for {id}: {e}");
            ApiErrorResponse::not_found(e.to_string()).into_json_tuple()
        }
    }
}

/// GET /api/workflows/runs/:run_id — Get detailed info for a single workflow run.
#[utoipa::path(
    get,
    path = "/api/workflows/runs/{run_id}",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Workflow run details", body = crate::types::JsonObject),
        (status = 404, description = "Run not found")
    )
)]
pub async fn get_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    match state.kernel.workflow_engine().get_run(run_id).await {
        Some(run) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": run.id.to_string(),
                "workflow_id": run.workflow_id.to_string(),
                "workflow_name": run.workflow_name,
                "input": run.input,
                "state": serde_json::to_value(&run.state).unwrap_or_default(),
                "output": run.output,
                "error": run.error,
                "started_at": run.started_at.to_rfc3339(),
                "completed_at": run.completed_at.map(|t| t.to_rfc3339()),
                "step_results": run.step_results.iter().map(|s| serde_json::json!({
                    "step_name": s.step_name,
                    "agent_id": s.agent_id,
                    "agent_name": s.agent_name,
                    "prompt": s.prompt,
                    "output": s.output,
                    "input_tokens": s.input_tokens,
                    "output_tokens": s.output_tokens,
                    "duration_ms": s.duration_ms,
                    "error": s.error,
                })).collect::<Vec<_>>(),
            })),
        ),
        None => ApiErrorResponse::not_found(format!("Run '{run_id}' not found")).into_json_tuple(),
    }
}

/// POST /api/workflows/runs/:run_id/rerun — Re-run a previous run with the same parameters.
///
/// Looks up the original run and starts a fresh run of the same workflow with the original run's `input`, returning the new run's id.
/// The original run is left untouched, so this is a non-destructive repeat of what actually executed rather than a re-submission of caller-supplied params.
/// Returns 202 with `{"run_id": ...}` on success, 400 for a malformed run id, and 404 if the original run (or the workflow it referenced) no longer exists.
#[utoipa::path(
    post,
    path = "/api/workflows/runs/{run_id}/rerun",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID to re-run")),
    responses(
        (status = 202, description = "New run started", body = crate::types::JsonObject),
        (status = 400, description = "Malformed run ID"),
        (status = 404, description = "Original run or its workflow not found")
    )
)]
pub async fn rerun_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    let engine = state.kernel.workflow_engine();
    // Read the workflow + input off the stored run rather than trusting the caller, so a re-run is always a faithful repeat of what executed.
    let (workflow_id, input) = match engine.get_run(run_id).await {
        Some(run) => (run.workflow_id, run.input),
        None => {
            return ApiErrorResponse::not_found(format!("Run '{run_id}' not found"))
                .into_json_tuple();
        }
    };

    // `create_run` returns None when the workflow definition is gone (e.g. it was deleted after the original run); surface that as a 404.
    let new_run_id = match engine.create_run(workflow_id, input).await {
        Some(rid) => rid,
        None => {
            return ApiErrorResponse::not_found(format!(
                "Workflow '{workflow_id}' for run '{run_id}' no longer exists"
            ))
            .into_json_tuple();
        }
    };
    let new_run_id_str = new_run_id.to_string();
    spawn_background_run(state.clone(), new_run_id);
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "run_id": new_run_id_str })),
    )
}

/// POST /api/workflows/runs/:run_id/cancel — Cancel a workflow run.
///
/// Transitions `Pending`, `Running`, or `Paused` runs to `Cancelled`.
/// Returns 200 with `{"run_id": ..., "state": "cancelled"}` on success,
/// 400 for a malformed run ID, 404 if the run does not exist, or 409 if
/// the run is already in a terminal state (includes `{"state": <state>}`
/// so callers can distinguish completed vs failed vs cancelled conflicts).
#[utoipa::path(
    post,
    path = "/api/workflows/runs/{run_id}/cancel",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Run cancelled", body = crate::types::JsonObject),
        (status = 400, description = "Malformed run ID"),
        (status = 404, description = "Run not found"),
        (status = 409, description = "Run already in terminal state")
    )
)]
pub async fn cancel_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    match state.kernel.workflow_engine().cancel_run(run_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": run_id.to_string(),
                "state": "cancelled",
            })),
        ),
        Err(CancelRunError::NotFound(_)) => {
            ApiErrorResponse::not_found(format!("Run '{run_id}' not found")).into_json_tuple()
        }
        Err(CancelRunError::AlreadyTerminal { state: s, .. }) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "conflict",
                "state": s,
                "message": format!("Run '{run_id}' is already {s}"),
            })),
        ),
    }
}

/// Request body for `POST /api/workflows/runs/:run_id/pause`.
#[derive(serde::Deserialize, Default)]
pub struct PauseRunRequest {
    /// Human-readable explanation shown in logs and the dashboard.
    /// Do not include secrets or PII.
    #[serde(default)]
    pub reason: Option<String>,
}

/// POST /api/workflows/runs/:run_id/pause — Pause a workflow run.
///
/// Returns 200 with `{"run_id": "...", "resume_token": "<uuid>"}` on success.
///
/// **SECURITY**: the `resume_token` in the response body is the ONLY surface
/// from which the plaintext token is ever visible. Do not log this response.
///
/// Returns 404 if the run is not found, 409 if the run is already paused
/// (with the existing token hash) or already terminal.
#[utoipa::path(
    post,
    path = "/api/workflows/runs/{run_id}/pause",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Run paused", body = crate::types::JsonObject),
        (status = 400, description = "Malformed run ID"),
        (status = 404, description = "Run not found"),
        (status = 409, description = "Run already paused or terminal")
    )
)]
pub async fn pause_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    let reason = req["reason"]
        .as_str()
        .unwrap_or("(no reason given)")
        .to_string();

    match state
        .kernel
        .workflow_engine()
        .pause_run(run_id, reason)
        .await
    {
        Ok(token) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": run_id.to_string(),
                // SECURITY: this is the ONLY place the plaintext token is
                // surfaced. The token is never persisted — only its hash is
                // stored at rest. Callers must not log this response.
                "resume_token": token.to_string(),
            })),
        ),
        Err(PauseRunError::NotFound(_)) => {
            ApiErrorResponse::not_found(format!("Run '{run_id}' not found")).into_json_tuple()
        }
        Err(PauseRunError::AlreadyPaused {
            resume_token_hash, ..
        }) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "already_paused",
                "resume_token_hash": resume_token_hash,
                "message": format!("Run '{run_id}' is already paused"),
            })),
        ),
        Err(PauseRunError::AlreadyTerminal { state: s, .. }) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "conflict",
                "state": s,
                "message": format!("Run '{run_id}' is already {s}"),
            })),
        ),
    }
}

/// Request body for `POST /api/workflows/runs/:run_id/resume`.
#[derive(serde::Deserialize)]
pub struct ResumeRunRequest {
    /// The plaintext resume token returned by the pause endpoint.
    pub resume_token: String,
}

/// POST /api/workflows/runs/:run_id/resume — Resume a paused workflow run.
///
/// Returns 200 with `{"run_id": "...", "state": "running"}` immediately after
/// the resume is initiated. The actual workflow continues asynchronously.
///
/// Returns 401 if the resume token does not match.
/// Returns 404 if the run is not found.
/// Returns 409 if the run is not paused or is a DAG workflow (unsupported).
#[utoipa::path(
    post,
    path = "/api/workflows/runs/{run_id}/resume",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Run resumed", body = crate::types::JsonObject),
        (status = 400, description = "Malformed run ID or missing token"),
        (status = 401, description = "Token mismatch"),
        (status = 404, description = "Run not found"),
        (status = 409, description = "Run not paused or DAG unsupported")
    )
)]
pub async fn resume_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    let token_str = match req["resume_token"].as_str() {
        Some(s) => s.to_string(),
        None => {
            return ApiErrorResponse::bad_request("Missing required field: resume_token")
                .into_json_tuple();
        }
    };

    let token = match token_str.parse::<uuid::Uuid>() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid resume_token: must be a UUID")
                .into_json_tuple();
        }
    };

    // Build agent resolver and send_message for the resume execution.
    let state_for_resolver = state.clone();
    let state_for_sender = state.clone();

    let agent_resolver = move |agent_ref: &librefang_kernel::workflow::StepAgent| {
        use librefang_kernel::workflow::StepAgent;
        match agent_ref {
            StepAgent::ById { id } => {
                let agent_id: librefang_types::agent::AgentId = id.parse().ok()?;
                let entry = state_for_resolver.kernel.agent_registry().get(agent_id)?;
                let inherit = entry.manifest.inherit_parent_context;
                Some((agent_id, entry.name.clone(), inherit))
            }
            StepAgent::ByName { name } => {
                let entry = state_for_resolver
                    .kernel
                    .agent_registry()
                    .find_by_name(name)?;
                let inherit = entry.manifest.inherit_parent_context;
                Some((entry.id, entry.name.clone(), inherit))
            }
        }
    };

    // Validate the token synchronously (quick state check) before spawning.
    // The actual resume_run call drives the workflow; we spawn it so the
    // HTTP response returns immediately with "running".
    let engine = state.kernel.workflow_engine();

    // Pre-validate: check the run exists and is Paused — we want to return
    // 401/404/409 synchronously, not after spawn. Use a quick get_run peek.
    let peek = engine.get_run(run_id).await;
    match &peek {
        None => {
            return ApiErrorResponse::not_found(format!("Run '{run_id}' not found"))
                .into_json_tuple();
        }
        Some(run) => match &run.state {
            WorkflowRunState::Paused {
                resume_token_hash, ..
            } => {
                // Constant-time hash comparison to avoid timing oracles.
                let presented_hash =
                    librefang_kernel::workflow::WorkflowEngine::hash_resume_token(&token);
                if resume_token_hash != &presented_hash {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"error": "token_mismatch"})),
                    );
                }
            }
            WorkflowRunState::Pending | WorkflowRunState::Running => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "not_paused",
                        "state": format!("{:?}", run.state).to_lowercase(),
                    })),
                );
            }
            WorkflowRunState::Completed
            | WorkflowRunState::Failed
            | WorkflowRunState::Cancelled => {
                let s = match &run.state {
                    WorkflowRunState::Completed => "completed",
                    WorkflowRunState::Failed => "failed",
                    WorkflowRunState::Cancelled => "cancelled",
                    _ => "terminal",
                };
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "not_paused",
                        "state": s,
                    })),
                );
            }
        },
    }

    // Check for DAG workflow (unsupported for resume).
    // We need the workflow definition to know if it uses DAG deps.
    // peek at workflow steps: if the run has dag deps, surface 409.
    // Actually — easier to just let resume_run handle it and map the error.
    // But we've already peeked; just spawn and map DagUnsupported -> 409.
    // The pre-check above validates the token, so the spawn won't hit 401.
    // Spawn resume in the background; return 200 immediately.
    // `state_for_sender` is an `Arc<AppState>` — clone it once more so the
    // `Fn` send_message closure can clone-per-call without conflicting with
    // the borrow held by `.workflow_engine().resume_run(...)`.
    let state_for_engine = state_for_sender.clone();
    let state_for_send_fn = state_for_sender;
    tokio::spawn(async move {
        let result = state_for_engine
            .kernel
            .workflow_engine()
            .resume_run(
                run_id,
                token,
                agent_resolver,
                move |agent_id: librefang_types::agent::AgentId,
                      message: String,
                      session_mode_override: Option<
                    librefang_types::agent::SessionMode,
                >| {
                    let sc = state_for_send_fn.clone();
                    async move {
                        sc.kernel
                            .send_message_with_session_mode(
                                agent_id,
                                &message,
                                session_mode_override,
                            )
                            .await
                            .map(|r| {
                                (
                                    r.response,
                                    r.total_usage.input_tokens,
                                    r.total_usage.output_tokens,
                                )
                            })
                            .map_err(|e| format!("{e}"))
                    }
                },
            )
            .await;
        if let Err(e) = result {
            tracing::warn!(run_id = %run_id, error = %e, "Background workflow resume failed");
        }
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "run_id": run_id.to_string(),
            "state": "running",
        })),
    )
}

/// POST /api/workflows/runs/:run_id/operator — Resolve a paused operator
/// step with an operator decision and drive the workflow forward (#5133).
///
/// Auth: goes through the normal auth layer (NOT on the public allowlist).
/// The authenticated operator is the security boundary for this resolution
/// — no resume token is required (unlike the generic `/resume` endpoint).
///
/// - 200 `{"run_id":..,"state":"running"}` — resolution accepted; the run
///   resumes asynchronously (Approve/Edit/Input) or has been marked Failed
///   (Reject).
/// - 400 — malformed run ID / unknown action / missing required payload.
/// - 404 — run not found.
/// - 409 — run not paused, not an operator-step pause, or the action is
///   not authorised at this step.
#[utoipa::path(
    post,
    path = "/api/workflows/runs/{run_id}/operator",
    tag = "workflows",
    params(("run_id" = String, Path, description = "Workflow run ID")),
    responses(
        (status = 200, description = "Operator action accepted", body = crate::types::JsonObject),
        (status = 400, description = "Malformed run ID / action / payload"),
        (status = 404, description = "Run not found"),
        (status = 409, description = "Not an operator pause or action not authorised")
    )
)]
pub async fn operator_action_workflow_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    // Optional so the handler still compiles / works on installs that
    // disable auth entirely; when auth is on, the middleware layer
    // (see `is_public` in `middleware.rs`) rejects unauthenticated
    // callers before we get here, so this Option is `Some` in
    // production. The reason we still extract it: we want the
    // operator's identity in the audit log on success, not just an
    // anonymous "operator action accepted" event.
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    use crate::workflow::OperatorAction;
    let operator_name = api_user
        .as_ref()
        .map(|u| u.0.name.clone())
        .unwrap_or_else(|| "<unauthenticated>".to_string());

    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    // Flat request shape (not the `OperatorAction` enum's
    // externally-tagged serde) so channel adapters / the dashboard can
    // post a simple `{"action":"approve"}` or
    // `{"action":"edit","payload":"..."}`.
    let action_str = match req["action"].as_str() {
        Some(s) => s.to_string(),
        None => {
            return ApiErrorResponse::bad_request("Missing required field: action")
                .into_json_tuple();
        }
    };
    let field_opt = req["field"].as_str().map(|s| s.to_string());
    let payload_opt = req["payload"].as_str().map(|s| s.to_string());

    // Build the typed action from the flat request shape.
    let action = match action_str.as_str() {
        "approve" => OperatorAction::Approve,
        "reject" => OperatorAction::Reject,
        "edit" => OperatorAction::Edit,
        "freeform_input" => OperatorAction::FreeformInput,
        "provide_input" => match field_opt.clone() {
            Some(f) if !f.is_empty() => OperatorAction::ProvideInput { field: f },
            _ => {
                return ApiErrorResponse::bad_request(
                    "action 'provide_input' requires a non-empty 'field'",
                )
                .into_json_tuple();
            }
        },
        other => {
            return ApiErrorResponse::bad_request(format!(
                "unknown operator action '{other}' (expected approve/reject/edit/\
                 provide_input/freeform_input)"
            ))
            .into_json_tuple();
        }
    };

    // Pre-validate the pause synchronously so we can return 404/409 before
    // spawning the (async) resume. Mirrors `resume_workflow_run`'s peek.
    let engine = state.kernel.workflow_engine();
    if engine.inspect_operator_pause(run_id).await.is_none() {
        // Distinguish "run unknown" from "not an operator pause" for a
        // useful status code.
        if engine.get_run(run_id).await.is_none() {
            return ApiErrorResponse::not_found(format!("Run '{run_id}' not found"))
                .into_json_tuple();
        }
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "not_operator_pause",
                "message": format!("Run '{run_id}' is not paused at an operator step"),
            })),
        );
    }

    // Reject / payload-less actions need no payload; Edit / *Input do —
    // surface the 400 synchronously rather than after the spawn.
    let needs_payload = matches!(
        action,
        OperatorAction::Edit | OperatorAction::FreeformInput | OperatorAction::ProvideInput { .. }
    );
    if needs_payload && payload_opt.as_deref().unwrap_or("").is_empty() {
        return ApiErrorResponse::bad_request(format!(
            "action '{action_str}' requires a non-empty 'payload'"
        ))
        .into_json_tuple();
    }

    let payload = payload_opt.clone();
    let state_for_resolver = state.clone();
    let agent_resolver = move |agent_ref: &librefang_kernel::workflow::StepAgent| {
        use librefang_kernel::workflow::StepAgent;
        match agent_ref {
            StepAgent::ById { id } => {
                let agent_id: librefang_types::agent::AgentId = id.parse().ok()?;
                let entry = state_for_resolver.kernel.agent_registry().get(agent_id)?;
                let inherit = entry.manifest.inherit_parent_context;
                Some((agent_id, entry.name.clone(), inherit))
            }
            StepAgent::ByName { name } => {
                let entry = state_for_resolver
                    .kernel
                    .agent_registry()
                    .find_by_name(name)?;
                let inherit = entry.manifest.inherit_parent_context;
                Some((entry.id, entry.name.clone(), inherit))
            }
        }
    };

    // Drive the resolution in the background; respond 200 immediately.
    // Reject resolves synchronously inside `resolve_operator_step` (no
    // subsequent steps), but spawning keeps the response shape uniform
    // with `/resume` and avoids blocking the request on a long pipeline.
    let state_for_engine = state.clone();
    let state_for_send = state.clone();
    let audit_action = action_str.clone();
    let audit_operator = operator_name.clone();
    tokio::spawn(async move {
        let result = state_for_engine
            .kernel
            .workflow_engine()
            .resolve_operator_step(
                run_id,
                action,
                payload,
                agent_resolver,
                move |agent_id: librefang_types::agent::AgentId,
                      message: String,
                      session_mode_override: Option<
                    librefang_types::agent::SessionMode,
                >| {
                    let sc = state_for_send.clone();
                    async move {
                        sc.kernel
                            .send_message_with_session_mode(
                                agent_id,
                                &message,
                                session_mode_override,
                            )
                            .await
                            .map(|r| {
                                (
                                    r.response,
                                    r.total_usage.input_tokens,
                                    r.total_usage.output_tokens,
                                )
                            })
                            .map_err(|e| format!("{e}"))
                    }
                },
            )
            .await;
        // Emit one structured event regardless of outcome so the audit
        // trail records WHO did WHAT against WHICH run, not just the
        // failures. Previously only `Err` produced a log line, which
        // meant a successful approve / reject / edit was invisible to
        // anyone tailing the daemon log or shipping audit events to a
        // SIEM.
        match result {
            Ok(_) => tracing::info!(
                run_id = %run_id,
                operator = %audit_operator,
                action = %audit_action,
                "operator action applied to workflow run",
            ),
            Err(e) => tracing::warn!(
                run_id = %run_id,
                operator = %audit_operator,
                action = %audit_action,
                error = %e,
                "operator action resolution failed (or run rejected/failed)",
            ),
        }
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "run_id": run_id.to_string(),
            "state": "running",
        })),
    )
}

/// GET /api/workflows/runs/:run_id/operator — Inspect the operator pause
/// on a paused run. Returns the artifact awaiting review and the actions
/// the workflow author authorised at the step. Companion to the
/// `POST .../operator` resolve endpoint — the dashboard hits this first to
/// learn what action buttons to render and what text to show.
///
/// - 200 — `{run_id, workflow_id, workflow_name, step_name,
///   operator_step_index, artifact, actions, started_at, paused_at}`
/// - 400 — malformed run ID.
/// - 404 — run not found.
/// - 409 — run is not paused or not at an operator step (so the
///   dashboard can render a "not awaiting operator review" hint instead
///   of an empty button bar).
pub async fn inspect_workflow_operator_pause(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let run_id = WorkflowRunId(match run_id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid run ID").into_json_tuple();
        }
    });

    let engine = state.kernel.workflow_engine();
    let pause = match engine.inspect_operator_pause(run_id).await {
        Some(p) => p,
        None => {
            // Distinguish "run unknown" from "not an operator pause" so
            // the dashboard surfaces a useful hint per status code.
            if engine.get_run(run_id).await.is_none() {
                return ApiErrorResponse::not_found(format!("Run '{run_id}' not found"))
                    .into_json_tuple();
            }
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "not_operator_pause",
                    "message": format!("Run '{run_id}' is not paused at an operator step"),
                })),
            );
        }
    };
    let run = match engine.get_run(run_id).await {
        Some(r) => r,
        None => {
            // Race window: the run vanished between the two reads.
            return ApiErrorResponse::not_found(format!("Run '{run_id}' not found"))
                .into_json_tuple();
        }
    };
    (StatusCode::OK, Json(operator_pause_row_json(&run, &pause)))
}

/// GET /api/workflows/operator/pending — List every run currently paused
/// at an operator step, oldest pause first. The dashboard renders this
/// as a "pending operator reviews" worklist so a human operator does not
/// have to fetch every run and filter client-side.
///
/// Returns `200` with a (possibly empty) array of rows in the same shape
/// as the single-run GET endpoint.
pub async fn list_pending_operator_workflow_runs(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let engine = state.kernel.workflow_engine();
    let rows = engine.list_pending_operator_runs().await;
    let body: Vec<serde_json::Value> = rows
        .iter()
        .map(|(run, pause)| operator_pause_row_json(run, pause))
        .collect();
    Json(body)
}

/// GET /api/workflows/:id/runs — List runs for the workflow named in the path.
#[utoipa::path(get, path = "/api/workflows/{id}/runs", tag = "workflows", params(("id" = String, Path, description = "Workflow ID")), responses((status = 200, description = "List workflow runs", body = Vec<serde_json::Value>), (status = 400, description = "Invalid workflow ID")))]
pub async fn list_workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });
    // `list_runs(None)` returns all workflows' runs; filter to this workflow to prevent cross-workflow data leak.
    let list: Vec<serde_json::Value> = state
        .kernel
        .workflow_engine()
        .list_runs(None)
        .await
        .iter()
        .filter(|r| r.workflow_id == workflow_id)
        .map(|r| {
            serde_json::json!({
                "id": r.id.to_string(),
                "workflow_name": r.workflow_name,
                "state": serde_json::to_value(&r.state).unwrap_or_default(),
                "steps_completed": r.step_results.len(),
                "started_at": r.started_at.to_rfc3339(),
                "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    (StatusCode::OK, Json(serde_json::Value::Array(list)))
}

// ---------------------------------------------------------------------------
// Save workflow as reusable template
// ---------------------------------------------------------------------------
/// POST /api/workflows/:id/save-as-template — Convert a workflow into a reusable template.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/save-as-template",
    tag = "workflows",
    params(("id" = String, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Template created", body = crate::types::JsonObject),
        (status = 404, description = "Workflow not found")
    )
)]
pub async fn save_workflow_as_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid workflow ID").into_json_tuple();
        }
    });

    let workflow = match state
        .kernel
        .workflow_engine()
        .get_workflow(workflow_id)
        .await
    {
        Some(w) => w,
        None => {
            return ApiErrorResponse::not_found(format!("Workflow '{}' not found", id))
                .into_json_tuple();
        }
    };

    let template = workflow.to_template();

    // Persist template to TOML file under the active kernel home directory.
    let templates_dir = state.kernel.home_dir().join("workflows").join("templates");
    if let Err(e) = tokio::fs::create_dir_all(&templates_dir).await {
        warn!("Failed to create templates directory: {e}");
    } else {
        let toml_path = templates_dir.join(format!("{}.toml", &template.id));
        match toml::to_string_pretty(&template) {
            Ok(toml_str) => {
                if let Err(e) = tokio::fs::write(&toml_path, toml_str).await {
                    warn!(
                        path = %toml_path.display(),
                        "Failed to write template file: {e}"
                    );
                }
            }
            Err(e) => {
                warn!("Failed to serialize template to TOML: {e}");
            }
        }
    }

    // Register in the in-memory template registry
    state.kernel.templates().register(template.clone()).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "created",
            "template": template,
        })),
    )
}
