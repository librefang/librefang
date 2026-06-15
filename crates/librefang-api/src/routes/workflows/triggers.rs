use super::*;

// ---------------------------------------------------------------------------
// Trigger routes
// ---------------------------------------------------------------------------
/// POST /api/triggers — Register a new event trigger.
#[utoipa::path(
    post,
    path = "/api/triggers",
    tag = "workflows",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Trigger created", body = crate::types::JsonObject),
        (status = 400, description = "Invalid trigger definition")
    )
)]
pub async fn create_trigger(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id_str = match req["agent_id"].as_str() {
        Some(id) => id,
        None => {
            return ApiErrorResponse::bad_request("Missing 'agent_id'").into_json_tuple();
        }
    };

    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid agent_id").into_json_tuple();
        }
    };

    let pattern: TriggerPattern = match req.get("pattern") {
        Some(p) => {
            // Legacy clients send `"task_posted"` as a bare string, but the
            // variant now carries an optional `assignee_match` field and
            // expects the struct form `{"task_posted": {...}}`. Rewrite the
            // bare strings to `{"<variant>": {}}` so both shapes parse.
            let normalized = normalize_pattern_json(p.clone());
            match serde_json::from_value(normalized) {
                Ok(pat) => pat,
                Err(e) => {
                    tracing::warn!("Invalid trigger pattern: {e}");
                    return ApiErrorResponse::bad_request("Invalid trigger pattern")
                        .into_json_tuple();
                }
            }
        }
        None => {
            return ApiErrorResponse::bad_request("Missing 'pattern'").into_json_tuple();
        }
    };

    let prompt_template = req["prompt_template"]
        .as_str()
        .unwrap_or("Event: {{event}}")
        .to_string();
    let max_fires = req["max_fires"].as_u64().unwrap_or(0);

    // Optional cross-session target: route triggered message to a different agent.
    // If the caller supplied a value but it is malformed, reject explicitly —
    // otherwise the trigger would silently register without any target and the
    // caller would assume the routing was accepted.
    let target_agent: Option<AgentId> = match req.get("target_agent_id").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => match s.parse() {
            Ok(id) => Some(id),
            Err(_) => {
                return ApiErrorResponse::bad_request(format!(
                    "Invalid 'target_agent_id': '{s}' is not a valid UUID"
                ))
                .into_json_tuple();
            }
        },
    };

    let cooldown_secs: Option<u64> = req["cooldown_secs"].as_u64();

    let session_mode: Option<librefang_types::agent::SessionMode> =
        match req.get("session_mode").and_then(|v| v.as_str()) {
            None => None,
            Some(s) => match serde_json::from_value(serde_json::json!(s)) {
                Ok(m) => Some(m),
                Err(_) => {
                    return ApiErrorResponse::bad_request(format!(
                        "Invalid 'session_mode': '{s}' (expected 'persistent' or 'new')"
                    ))
                    .into_json_tuple();
                }
            },
        };

    // Optional workflow_id: if set, the trigger fires a workflow run instead
    // of dispatching a message to an agent via send_message_full.
    let workflow_id: Option<String> = match req.get("workflow_id").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => {
            if s.is_empty() {
                return ApiErrorResponse::bad_request(
                    "workflow_id must not be empty when provided",
                )
                .into_json_tuple();
            }
            if s.len() > librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN {
                return ApiErrorResponse::bad_request(format!(
                    "workflow_id too long ({} chars, max {})",
                    s.len(),
                    librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN
                ))
                .into_json_tuple();
            }
            Some(s.to_string())
        }
    };

    match state.kernel.register_trigger_with_target(
        agent_id,
        pattern,
        prompt_template,
        max_fires,
        target_agent,
        cooldown_secs,
        session_mode,
        workflow_id.clone(),
    ) {
        Ok(trigger_id) => {
            let mut resp = serde_json::json!({
                "trigger_id": trigger_id.to_string(),
                "agent_id": agent_id.to_string(),
            });
            if let Some(target) = target_agent {
                resp["target_agent_id"] = serde_json::json!(target.to_string());
            }
            if let Some(wid) = workflow_id {
                resp["workflow_id"] = serde_json::json!(wid);
            }
            (StatusCode::CREATED, Json(resp))
        }
        Err(e) => {
            tracing::warn!("Trigger registration failed: {e}");
            // The per-agent cap (audit: trigger-engine-no-per-agent-cap)
            // and other client-side rejections surface as `InvalidInput`
            // — those are 400, not "agent not found". Only a genuine
            // missing-owner/target maps to 404. Mirrors the parallel
            // branch in `update_schedule` above.
            use crate::error::KernelError;
            use librefang_types::error::LibreFangError;
            match e {
                KernelError::LibreFang(LibreFangError::InvalidInput(msg)) => {
                    ApiErrorResponse::bad_request(msg).into_json_tuple()
                }
                other => {
                    ApiErrorResponse::not_found(format!("Trigger registration failed: {other}"))
                        .into_json_tuple()
                }
            }
        }
    }
}

#[utoipa::path(get, path = "/api/triggers", tag = "workflows", params(("agent_id" = Option<String>, Query, description = "Filter by agent ID")), responses((status = 200, description = "List triggers", body = crate::types::JsonObject)))]
pub async fn list_triggers(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let agent_filter = params
        .get("agent_id")
        .and_then(|id| id.parse::<AgentId>().ok());

    // Owner-scoping: non-admins can't see triggers for agents they don't
    // author. Two enforcement points:
    //   1. With ?agent_id=... — verify the caller owns that agent.
    //   2. Without — post-filter the trigger list by author.
    let restrict_to: Option<String> = match api_user.as_ref() {
        Some(u) if u.0.role < crate::middleware::UserRole::Admin => Some(u.0.name.clone()),
        _ => None,
    };
    if let (Some(user_name), Some(aid)) = (restrict_to.as_ref(), agent_filter) {
        let owns = state
            .kernel
            .agent_registry()
            .get(aid)
            .as_ref()
            .map(|e| e.manifest.author.eq_ignore_ascii_case(user_name))
            .unwrap_or(false);
        if !owns {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"triggers": [], "total": 0})),
            )
                .into_response();
        }
    }

    let triggers = state.kernel.list_triggers(agent_filter);
    let list: Vec<serde_json::Value> = if let Some(ref user_name) = restrict_to {
        // No explicit agent_id — fall back to per-trigger owner check.
        let owned_ids: std::collections::HashSet<librefang_types::agent::AgentId> = state
            .kernel
            .agent_registry()
            .list()
            .iter()
            .filter(|e| e.manifest.author.eq_ignore_ascii_case(user_name))
            .map(|e| e.id)
            .collect();
        triggers
            .iter()
            .filter(|tr| owned_ids.contains(&tr.agent_id))
            .map(trigger_to_json)
            .collect()
    } else {
        triggers.iter().map(trigger_to_json).collect()
    };
    let total = list.len();
    Json(serde_json::json!({"triggers": list, "total": total})).into_response()
}

#[utoipa::path(get, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), responses((status = 200, description = "Trigger detail", body = crate::types::JsonObject), (status = 404, description = "Not found")))]
/// GET /api/triggers/:id — Fetch a single trigger by ID.
pub async fn get_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => return ApiErrorResponse::bad_request("Invalid trigger ID").into_json_tuple(),
    });
    match state.kernel.get_trigger(trigger_id) {
        Some(t) => (StatusCode::OK, Json(trigger_to_json(&t))),
        None => ApiErrorResponse::not_found("Trigger not found").into_json_tuple(),
    }
}

/// DELETE /api/triggers/:id — Remove a trigger.
///
/// Idempotent (RFC 9110 §9.2.2): deleting a trigger that is already gone
/// returns `200 OK` with `{"status": "already-deleted"}` instead of `404`.
/// `400` is reserved for the malformed-UUID case alone. Refs #3509.
#[utoipa::path(
    delete,
    path = "/api/triggers/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Trigger ID")),
    responses(
        (status = 200, description = "Trigger deleted (or was already absent — idempotent)"),
        (status = 400, description = "Malformed trigger ID")
    )
)]
pub async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid trigger ID").into_json_tuple();
        }
    });

    if state.kernel.remove_trigger(trigger_id) {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "trigger_id": id})),
        )
    } else {
        // Idempotent DELETE — replayed request, double-click, or already
        // removed by another caller. Surface success so clients don't have
        // to special-case 404 on a successful-state outcome.
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "already-deleted", "trigger_id": id})),
        )
    }
}

// ---------------------------------------------------------------------------
// Trigger update endpoint
// ---------------------------------------------------------------------------
#[utoipa::path(patch, path = "/api/triggers/{id}", tag = "workflows", params(("id" = String, Path, description = "Trigger ID")), request_body(content = crate::types::JsonObject, description = "Partial trigger fields: pattern, prompt_template, enabled, max_fires, cooldown_secs, session_mode, target_agent_id"), responses((status = 200, description = "Updated trigger", body = crate::types::JsonObject), (status = 404, description = "Not found")))]
/// PATCH /api/triggers/:id — Partially update a trigger.
///
/// All body fields are optional. Only provided fields are changed.
/// Supported fields: `pattern`, `prompt_template`, `enabled`, `max_fires`,
/// `cooldown_secs` (pass `null` to clear), `session_mode` (pass `null` to clear),
/// `target_agent_id` (pass `null` to clear, omit to leave unchanged).
pub async fn update_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => return ApiErrorResponse::bad_request("Invalid trigger ID").into_json_tuple(),
    });

    // Parse pattern if provided
    let pattern = if req.get("pattern").is_some() && !req["pattern"].is_null() {
        let normalized = normalize_pattern_json(req["pattern"].clone());
        match serde_json::from_value::<TriggerPattern>(normalized) {
            Ok(p) => Some(p),
            Err(e) => {
                return ApiErrorResponse::bad_request(format!("Invalid pattern: {e}"))
                    .into_json_tuple()
            }
        }
    } else {
        None
    };

    // Parse session_mode: absent = no change, null = clear, string = set
    let session_mode: Option<Option<librefang_types::agent::SessionMode>> =
        if req.get("session_mode").is_none() {
            None
        } else if req["session_mode"].is_null() {
            Some(None)
        } else {
            match serde_json::from_value(req["session_mode"].clone()) {
                Ok(m) => Some(Some(m)),
                Err(e) => {
                    return ApiErrorResponse::bad_request(format!("Invalid session_mode: {e}"))
                        .into_json_tuple()
                }
            }
        };

    // Parse cooldown_secs: absent = no change, null = clear, number = set
    let cooldown_secs: Option<Option<u64>> = if req.get("cooldown_secs").is_none() {
        None
    } else if req["cooldown_secs"].is_null() {
        Some(None)
    } else {
        match req["cooldown_secs"].as_u64() {
            Some(n) => Some(Some(n)),
            None => {
                return ApiErrorResponse::bad_request(
                    "cooldown_secs must be a non-negative integer",
                )
                .into_json_tuple()
            }
        }
    };

    // Parse target_agent_id: absent = no change, null = clear, string = set
    let target_agent: Option<Option<AgentId>> = if req.get("target_agent_id").is_none() {
        None
    } else if req["target_agent_id"].is_null() {
        Some(None)
    } else {
        match req["target_agent_id"].as_str().and_then(|s| s.parse().ok()) {
            Some(id) => Some(Some(id)),
            None => {
                return ApiErrorResponse::bad_request("Invalid 'target_agent_id'").into_json_tuple()
            }
        }
    };

    // Validate target agent exists when being set (mirrors POST validation)
    if let Some(Some(target_id)) = target_agent {
        if state.kernel.agent_registry().get(target_id).is_none() {
            return ApiErrorResponse::bad_request(format!(
                "target_agent_id '{target_id}' does not exist"
            ))
            .into_json_tuple();
        }
    }

    // Parse workflow_id: absent = no change, null = clear, string = set
    let workflow_id: Option<Option<String>> = if req.get("workflow_id").is_none() {
        None
    } else if req["workflow_id"].is_null() {
        Some(None)
    } else {
        match req["workflow_id"].as_str() {
            Some(s) => {
                if s.is_empty() {
                    return ApiErrorResponse::bad_request(
                        "workflow_id must not be empty when provided",
                    )
                    .into_json_tuple();
                }
                if s.len() > librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN {
                    return ApiErrorResponse::bad_request(format!(
                        "workflow_id too long ({} chars, max {})",
                        s.len(),
                        librefang_kernel::triggers::MAX_WORKFLOW_ID_LEN
                    ))
                    .into_json_tuple();
                }
                Some(Some(s.to_string()))
            }
            None => {
                return ApiErrorResponse::bad_request("workflow_id must be a string or null")
                    .into_json_tuple()
            }
        }
    };

    let patch = TriggerPatch {
        pattern,
        prompt_template: req["prompt_template"].as_str().map(|s| s.to_string()),
        enabled: req["enabled"].as_bool(),
        max_fires: req["max_fires"].as_u64(),
        cooldown_secs,
        session_mode,
        target_agent,
        workflow_id,
    };

    match state.kernel.update_trigger(trigger_id, patch) {
        Some(t) => (StatusCode::OK, Json(trigger_to_json(&t))),
        None => ApiErrorResponse::not_found("Trigger not found").into_json_tuple(),
    }
}
