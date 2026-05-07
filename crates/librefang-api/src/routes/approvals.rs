//! Approval workflow handlers extracted from `routes/system.rs` (#3749).
//!
//! Covers the manual + per-session approval lifecycle (`/api/approvals/*`)
//! and the TOTP enrollment/confirmation/revocation surface
//! (`/api/approvals/totp/*`). Public route paths and JSON shapes match the
//! pre-extraction behaviour exactly — `routes/system.rs::router()` merges
//! this module's router so callers see no change.

use super::tools_sessions::PaginationParams;
use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build routes for the approvals + TOTP sub-domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        // Static paths must precede the `{id}` wildcard.
        .route(
            "/approvals",
            axum::routing::get(list_approvals).post(create_approval),
        )
        .route("/approvals/batch", axum::routing::post(batch_resolve))
        .route(
            "/approvals/session/{session_id}",
            axum::routing::get(list_approvals_for_session),
        )
        .route(
            "/approvals/session/{session_id}/approve_all",
            axum::routing::post(approve_all_for_session),
        )
        .route(
            "/approvals/session/{session_id}/reject_all",
            axum::routing::post(reject_all_for_session),
        )
        .route("/approvals/audit", axum::routing::get(audit_log))
        .route("/approvals/count", axum::routing::get(approval_count))
        .route("/approvals/totp/setup", axum::routing::post(totp_setup))
        .route(
            "/approvals/totp/confirm",
            axum::routing::post(totp_confirm),
        )
        .route(
            "/approvals/totp/status",
            axum::routing::get(totp_status),
        )
        .route(
            "/approvals/totp/revoke",
            axum::routing::post(totp_revoke),
        )
        .route("/approvals/{id}", axum::routing::get(get_approval))
        .route(
            "/approvals/{id}/approve",
            axum::routing::post(
                |state: State<Arc<AppState>>,
                 id: Path<String>,
                 lang: Option<axum::Extension<RequestLanguage>>,
                 body: Json<ApproveRequestBody>| async move {
                    approve_request(state, id, lang, body).await
                },
            ),
        )
        .route(
            "/approvals/{id}/reject",
            axum::routing::post(
                |state: State<Arc<AppState>>,
                 id: Path<String>,
                 lang: Option<axum::Extension<RequestLanguage>>| async move {
                    reject_request(state, id, lang).await
                },
            ),
        )
        .route(
            "/approvals/{id}/modify",
            axum::routing::post(
                |state: State<Arc<AppState>>,
                 id: Path<String>,
                 lang: Option<axum::Extension<RequestLanguage>>,
                 body: Json<ModifyRequestBody>| async move {
                    modify_request(state, id, body, lang).await
                },
            ),
        )
}

// ---------------------------------------------------------------------------
// Execution Approval System — backed by kernel.approvals()
// ---------------------------------------------------------------------------

/// Serialize an [`ApprovalRequest`] to the JSON shape expected by the dashboard.
///
/// Adds alias fields: `action` (= `action_summary`), `agent_name`, `created_at` (= `requested_at`).
fn approval_to_json(
    a: &librefang_types::approval::ApprovalRequest,
    registry_agents: &[librefang_types::agent::AgentEntry],
) -> serde_json::Value {
    let agent_name = registry_agents
        .iter()
        .find(|ag| ag.id.to_string() == a.agent_id || ag.name == a.agent_id)
        .map(|ag| ag.name.as_str())
        .unwrap_or(&a.agent_id);
    serde_json::json!({
        "id": a.id,
        "agent_id": a.agent_id,
        "agent_name": agent_name,
        "tool_name": a.tool_name,
        "description": a.description,
        "action_summary": a.action_summary,
        "action": a.action_summary,
        "risk_level": a.risk_level,
        "requested_at": a.requested_at,
        "created_at": a.requested_at,
        "timeout_secs": a.timeout_secs,
        "session_id": a.session_id,
        "status": "pending"
    })
}

/// GET /api/approvals — List pending and recent approval requests.
///
/// Transforms field names to match the dashboard template expectations:
/// `action_summary` → `action`, `agent_id` → `agent_name`, `requested_at` → `created_at`.
#[utoipa::path(
    get,
    path = "/api/approvals",
    tag = "approvals",
    params(
        ("limit" = Option<usize>, Query, description = "Max items (default 50, max 500)"),
        ("offset" = Option<usize>, Query, description = "Items to skip"),
    ),
    responses((status = 200, description = "Paginated list of pending and recent approvals", body = crate::types::JsonObject))
)]
pub async fn list_approvals(
    State(state): State<Arc<AppState>>,
    Query(pagination): Query<PaginationParams>,
) -> impl IntoResponse {
    let pending = state.kernel.approvals().list_pending();
    // Pull the full in-memory recent buffer (capped at
    // MAX_RECENT_APPROVALS = 100 by approval.rs), not a hard-coded 50.
    // The earlier limit meant `total` reported pending + 50 even when
    // the buffer held more, so a frontend asking for `offset=50` got
    // an empty page despite `total > offset` — pagination contract
    // broken (audit of #3958).  The buffer cap is the real bound;
    // surfacing the full set here lets the skip/take below paginate
    // over the actual window the server can serve.
    let recent = state.kernel.approvals().list_recent(usize::MAX);

    let registry_agents = state.kernel.agent_registry().list();
    let agent_name_for = |agent_id: &str| {
        registry_agents
            .iter()
            .find(|ag| ag.id.to_string() == agent_id || ag.name == agent_id)
            .map(|ag| ag.name.clone())
            .unwrap_or_else(|| agent_id.to_string())
    };

    let mut approvals: Vec<serde_json::Value> = pending
        .iter()
        .map(|a| approval_to_json(a, &registry_agents))
        .collect();

    approvals.extend(recent.into_iter().map(|record| {
        let request = record.request;
        let agent_name = agent_name_for(&request.agent_id);
        let status = match record.decision {
            librefang_types::approval::ApprovalDecision::Approved => "approved",
            librefang_types::approval::ApprovalDecision::Denied => "rejected",
            librefang_types::approval::ApprovalDecision::TimedOut => "expired",
            librefang_types::approval::ApprovalDecision::ModifyAndRetry { .. } => {
                "modify_and_retry"
            }
            librefang_types::approval::ApprovalDecision::Skipped => "skipped",
        };
        serde_json::json!({
            "id": request.id,
            "agent_id": request.agent_id,
            "agent_name": agent_name,
            "tool_name": request.tool_name,
            "description": request.description,
            "action_summary": request.action_summary,
            "action": request.action_summary,
            "risk_level": request.risk_level,
            "requested_at": request.requested_at,
            "created_at": request.requested_at,
            "timeout_secs": request.timeout_secs,
            "status": status,
            "decided_at": record.decided_at,
            "decided_by": record.decided_by,
        })
    }));

    approvals.sort_by(|a, b| {
        let a_pending = a["status"].as_str() == Some("pending");
        let b_pending = b["status"].as_str() == Some("pending");
        b_pending
            .cmp(&a_pending)
            .then_with(|| b["created_at"].as_str().cmp(&a["created_at"].as_str()))
    });

    let total = approvals.len();
    let offset = pagination.effective_offset();
    let limit = pagination.effective_limit();
    let items: Vec<_> = approvals.into_iter().skip(offset).take(limit).collect();

    Json(serde_json::json!({
        "approvals": items,
        "total": total,
        "offset": offset,
        "limit": limit,
    }))
}

/// GET /api/approvals/{id} — Get a single approval request by ID.
#[utoipa::path(get, path = "/api/approvals/{id}", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), responses((status = 200, description = "Single approval request", body = crate::types::JsonObject), (status = 404, description = "Approval not found")))]
pub async fn get_approval(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple();
        }
    };

    match state.kernel.approvals().get_pending(uuid) {
        Some(a) => {
            let registry_agents = state.kernel.agent_registry().list();
            (StatusCode::OK, Json(approval_to_json(&a, &registry_agents)))
        }
        None => {
            ApiErrorResponse::not_found(t.t_args("api-error-approval-not-found", &[("id", &id)]))
                .into_json_tuple()
        }
    }
}

/// POST /api/approvals — Create a manual approval request (for external systems).
///
/// Note: Most approval requests are created automatically by the tool_runner
/// when an agent invokes a tool that requires approval. This endpoint exists
/// for external integrations that need to inject approval gates.
#[derive(serde::Deserialize)]
pub(crate) struct CreateApprovalRequest {
    pub agent_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub action_summary: String,
    /// Optional session ID — when set, this request participates in
    /// per-session batch resolve via `/api/approvals/session/{session_id}/approve_all`.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[utoipa::path(post, path = "/api/approvals", tag = "approvals", request_body = crate::types::JsonObject, responses((status = 200, description = "Approval created", body = crate::types::JsonObject)))]
#[allow(private_interfaces)]
pub async fn create_approval(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApprovalRequest>,
) -> impl IntoResponse {
    use librefang_types::approval::{ApprovalRequest, RiskLevel};

    let policy = state.kernel.approvals().policy();
    let id = uuid::Uuid::new_v4();
    let approval_req = ApprovalRequest {
        id,
        agent_id: req.agent_id,
        tool_name: req.tool_name.clone(),
        description: if req.description.is_empty() {
            format!("Manual approval request for {}", req.tool_name)
        } else {
            req.description
        },
        action_summary: if req.action_summary.is_empty() {
            req.tool_name.clone()
        } else {
            req.action_summary
        },
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        timeout_secs: policy.timeout_secs,
        sender_id: None,
        channel: None,
        route_to: Vec::new(),
        escalation_count: 0,
        session_id: req.session_id,
    };

    // Spawn the request in the background (it will block until resolved or timed out)
    let kernel = Arc::clone(&state.kernel);
    tokio::spawn(async move {
        kernel.approvals().request_approval(approval_req).await;
    });

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id.to_string(), "status": "pending"})),
    )
}

/// POST /api/approvals/{id}/approve — Approve a pending request.
///
/// When TOTP is enabled, the request body must include a `totp_code` field.
#[derive(serde::Deserialize, Default)]
pub(crate) struct ApproveRequestBody {
    #[serde(default)]
    totp_code: Option<String>,
}

#[utoipa::path(post, path = "/api/approvals/{id}/approve", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Request approved", body = crate::types::JsonObject)))]
#[allow(private_interfaces)]
pub async fn approve_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<ApproveRequestBody>,
) -> axum::response::Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };

    // Verify TOTP code or recovery code if this specific tool requires it.
    // Use per-tool check so tools not in totp_tools skip TOTP (and lockout)
    // even when second_factor = totp is enabled globally.
    let totp_issuer = state.kernel.approvals().policy().totp_issuer.clone();
    let tool_requires_totp = state
        .kernel
        .approvals()
        .get_pending(uuid)
        .map(|req| {
            state
                .kernel
                .approvals()
                .policy()
                .tool_requires_totp(&req.tool_name)
        })
        .unwrap_or(false);
    let totp_verified = if tool_requires_totp {
        if state.kernel.approvals().is_totp_locked_out("api_admin") {
            return ApiErrorResponse::bad_request(
                "Too many failed TOTP attempts. Try again later.",
            )
            .into_json_tuple()
            .into_response();
        }
        match body.totp_code.as_deref() {
            Some(code) => {
                if state.kernel.approvals().recovery_code_format_matches(code) {
                    // Atomically redeem the recovery code (fixes TOCTOU #3560
                    // and vault_set-failure bypass #3633).
                    match state.kernel.vault_redeem_recovery_code(code) {
                        Ok(true) => true,
                        Ok(false) => {
                            // check_and_record_totp_failure atomically checks lockout
                            // and records the failure, fixing TOCTOU (#3584).
                            match state
                                .kernel
                                .approvals()
                                .check_and_record_totp_failure("api_admin")
                            {
                                Err(true) => {
                                    return ApiErrorResponse::bad_request(
                                        "Too many failed TOTP attempts. Try again later.",
                                    )
                                    .into_json_tuple()
                                    .into_response();
                                }
                                Err(false) => {
                                    return ApiErrorResponse::internal(
                                        "Failed to persist TOTP failure counter",
                                    )
                                    .into_json_tuple()
                                    .into_response();
                                }
                                Ok(()) => {}
                            }
                            return ApiErrorResponse::bad_request("Invalid recovery code")
                                .into_json_tuple()
                                .into_response();
                        }
                        Err(e) => {
                            return ApiErrorResponse::internal(e)
                                .into_json_tuple()
                                .into_response();
                        }
                    }
                } else {
                    let secret = match state.kernel.vault_get("totp_secret") {
                        Some(s) => s,
                        None => {
                            return ApiErrorResponse::bad_request(
                                "TOTP not configured. Run POST /api/approvals/totp/setup first.",
                            )
                            .into_json_tuple()
                            .into_response();
                        }
                    };
                    // Replay-prevention check (#3359): reject a code that was
                    // already used within the last 60 seconds (two TOTP windows).
                    if state.kernel.approvals().is_totp_code_used(code) {
                        // Atomic check + record (#3584) preserves fail-secure
                        // on DB persist failure (#3372): Err(false) = DB write
                        // dropped; Err(true) = already locked out, fall through
                        // to "already used" response so the lockout state is
                        // not leaked here.
                        if let Err(false) = state
                            .kernel
                            .approvals()
                            .check_and_record_totp_failure("api_admin")
                        {
                            return ApiErrorResponse::internal(
                                "Failed to persist TOTP failure counter",
                            )
                            .into_json_tuple()
                            .into_response();
                        }
                        return ApiErrorResponse::bad_request(
                            "TOTP code has already been used. Wait for the next 30-second window.",
                        )
                        .into_json_tuple()
                        .into_response();
                    }
                    match crate::approval::ApprovalManager::verify_totp_code_with_issuer(
                        &secret,
                        code,
                        &totp_issuer,
                    ) {
                        Ok(true) => {
                            // SECURITY (#3360): Bind the consumed code to the
                            // approval id it authorized. The replay window is
                            // still global (`is_totp_code_used` keys on the
                            // hash alone) so the code is single-use across
                            // all actions; the binding only documents *which*
                            // action used it for post-incident audit.
                            //
                            // Fail-secure (#3372 parity): if the DB write
                            // fails the code is NOT in the replay table and
                            // could be reused, so reject with 500 rather than
                            // silently approving.
                            if state
                                .kernel
                                .approvals()
                                .record_totp_code_used_for(code, Some(&format!("approval:{uuid}")))
                                .is_err()
                            {
                                return ApiErrorResponse::internal(
                                    "Failed to persist TOTP used-code record",
                                )
                                .into_json_tuple()
                                .into_response();
                            }
                            // Audit trail: write the binding alongside the
                            // approval resolution so an auditor can correlate
                            // (totp_code_hash, approval_uuid) without joining
                            // tables.
                            state.kernel.audit().record_with_context(
                                "system",
                                librefang_kernel::audit::AuditAction::AuthAttempt,
                                format!("totp_used_for_approval:{uuid}"),
                                "totp_verified",
                                None,
                                Some("api".to_string()),
                            );
                            true
                        }
                        Ok(false) => {
                            // Fail-secure: atomically check lockout + record failure (#3372/#3584).
                            match state
                                .kernel
                                .approvals()
                                .check_and_record_totp_failure("api_admin")
                            {
                                Err(true) => {
                                    return ApiErrorResponse::bad_request(
                                        "Too many failed TOTP attempts. Try again later.",
                                    )
                                    .into_json_tuple()
                                    .into_response();
                                }
                                Err(false) => {
                                    return ApiErrorResponse::internal(
                                        "Failed to persist TOTP failure counter",
                                    )
                                    .into_json_tuple()
                                    .into_response();
                                }
                                Ok(()) => {}
                            }
                            return ApiErrorResponse::bad_request("Invalid TOTP code")
                                .into_json_tuple()
                                .into_response();
                        }
                        Err(e) => {
                            return ApiErrorResponse::bad_request(e)
                                .into_json_tuple()
                                .into_response();
                        }
                    }
                }
            }
            None => false,
        }
    } else {
        false
    };

    match state
        .kernel
        .resolve_tool_approval(
            uuid,
            librefang_types::approval::ApprovalDecision::Approved,
            Some("api".to_string()),
            totp_verified,
            Some("api_admin"),
        )
        .await
    {
        Ok((resp, _deferred)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "approved", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e.to_string()).into_json_tuple().into_response(),
    }
}

/// POST /api/approvals/{id}/reject — Reject a pending request.
#[utoipa::path(post, path = "/api/approvals/{id}/reject", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), responses((status = 200, description = "Request rejected", body = crate::types::JsonObject)))]
pub async fn reject_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };

    match state
        .kernel
        .resolve_tool_approval(
            uuid,
            librefang_types::approval::ApprovalDecision::Denied,
            Some("api".to_string()),
            false,
            None,
        )
        .await
    {
        Ok((resp, _deferred)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "rejected", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        )
            .into_response(),
        // #3541: route the typed `KernelOpError` through the central
        // status-code map. The previous `not_found(_)` was wrong — a
        // `KernelOpError::Unavailable` (approval gate disabled) was
        // surfacing as 404 instead of 503, and an internal `Other`
        // failure surfaced as 404 instead of 500.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Approval — modify, batch, audit, count
// ---------------------------------------------------------------------------

/// POST /api/approvals/{id}/modify — Return a pending request with feedback for modification.
#[derive(serde::Deserialize)]
pub(crate) struct ModifyRequestBody {
    #[serde(default)]
    feedback: String,
}

#[utoipa::path(post, path = "/api/approvals/{id}/modify", tag = "approvals", params(("id" = String, Path, description = "Approval ID")), request_body = crate::types::JsonObject, responses((status = 200, description = "Request modified", body = crate::types::JsonObject)))]
#[allow(private_interfaces)]
pub async fn modify_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ModifyRequestBody>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> axum::response::Response {
    // Truncate feedback to prevent database bloat
    let feedback: String = body
        .feedback
        .chars()
        .take(librefang_types::approval::MAX_APPROVAL_FEEDBACK_LEN)
        .collect();
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(t.t("api-error-approval-invalid-id"))
                .into_json_tuple()
                .into_response();
        }
    };

    match state
        .kernel
        .resolve_tool_approval(
            uuid,
            librefang_types::approval::ApprovalDecision::ModifyAndRetry { feedback },
            Some("api".to_string()),
            false,
            None,
        )
        .await
    {
        Ok((resp, _deferred)) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "modified", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        )
            .into_response(),
        // #3541: see `reject_request` above — route through the typed
        // `KernelOpError` mapping so non-NotFound variants get the
        // status code their semantics demand.
        Err(e) => ApiErrorResponse::from(e).into_response(),
    }
}

/// POST /api/approvals/batch — Batch resolve multiple pending requests.
#[derive(serde::Deserialize)]
pub(crate) struct BatchResolveRequest {
    ids: Vec<String>,
    decision: String,
}

#[utoipa::path(post, path = "/api/approvals/batch", tag = "approvals", request_body = crate::types::JsonObject, responses((status = 200, description = "Batch resolve results", body = crate::types::JsonObject)))]
#[allow(private_interfaces)]
pub async fn batch_resolve(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BatchResolveRequest>,
) -> impl IntoResponse {
    const MAX_BATCH_SIZE: usize = 100;

    if body.ids.len() > MAX_BATCH_SIZE {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": format!("batch size {} exceeds maximum {MAX_BATCH_SIZE}", body.ids.len())}),
            ),
        );
    }

    let decision = match body.decision.as_str() {
        "approve" => librefang_types::approval::ApprovalDecision::Approved,
        "reject" => librefang_types::approval::ApprovalDecision::Denied,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": format!("invalid decision: {other}, expected 'approve' or 'reject'")}),
                ),
            );
        }
    };

    // Batch approve is incompatible with TOTP enforcement for tools that
    // require a TOTP code. Check if any of the requested IDs need TOTP;
    // if so, reject the batch so each can be approved individually.
    // Batch reject is always allowed.
    if matches!(
        decision,
        librefang_types::approval::ApprovalDecision::Approved
    ) {
        let policy = state.kernel.approvals().policy();
        let any_needs_totp = body
            .ids
            .iter()
            .filter_map(|id| uuid::Uuid::parse_str(id).ok())
            .filter_map(|uid| state.kernel.approvals().get_pending(uid))
            .any(|req| policy.tool_requires_totp(&req.tool_name));
        if any_needs_totp {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Batch approval is not available when TOTP is required for some tools. Approve those items individually with TOTP verification."
                })),
            );
        }
    }

    // Parse UUIDs, returning error entries for invalid ones
    let mut result_json: Vec<serde_json::Value> = Vec::with_capacity(body.ids.len());
    let mut valid_uuids = Vec::new();
    for id_str in &body.ids {
        match uuid::Uuid::parse_str(id_str) {
            Ok(uuid) => valid_uuids.push(uuid),
            Err(_) => {
                result_json.push(serde_json::json!({
                    "id": id_str, "status": "error", "message": "invalid UUID"
                }));
            }
        }
    }

    for uuid in valid_uuids {
        let id = uuid.to_string();
        match state
            .kernel
            .resolve_tool_approval(uuid, decision.clone(), Some("api".to_string()), false, None)
            .await
        {
            Ok((resp, _)) => result_json.push(serde_json::json!({
                "id": id,
                "status": "ok",
                "decision": resp.decision.as_str(),
                "decided_at": resp.decided_at.to_rfc3339(),
            })),
            Err(e) => result_json
                .push(serde_json::json!({"id": id, "status": "error", "message": e.to_string()})),
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"results": result_json})),
    )
}

// ---------------------------------------------------------------------------
// Per-session approval helpers
// ---------------------------------------------------------------------------

/// GET /api/approvals/session/{session_id} — List pending approvals for a session.
///
/// Mirrors `has_blocking_approval(session_key)` from Hermes-Agent: returns all
/// pending `ApprovalRequest`s whose `session_id` field matches the path param.
#[utoipa::path(
    get,
    path = "/api/approvals/session/{session_id}",
    tag = "approvals",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Pending approvals for session", body = crate::types::JsonObject)
    )
)]
pub async fn list_approvals_for_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    // Validate session_id is not empty/whitespace.
    if session_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id must not be empty or whitespace"})),
        );
    }
    // Reject excessively long session_id values to prevent DoS via memory/log amplification.
    if session_id.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id must not exceed 256 bytes"})),
        );
    }
    let registry_agents = state.kernel.agent_registry().list();
    let pending = state
        .kernel
        .approvals()
        .list_pending_for_session(&session_id);
    let items: Vec<serde_json::Value> = pending
        .iter()
        .map(|a| approval_to_json(a, &registry_agents))
        .collect();
    let count = items.len();
    let has_pending = !items.is_empty();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": session_id,
            "pending": items,
            "count": count,
            "has_pending": has_pending,
        })),
    )
}

/// POST /api/approvals/session/{session_id}/approve_all — Approve all pending
/// approvals for the given session atomically.
///
/// Mirrors Hermes-Agent's `resolve_gateway_approval(session_key, "once",
/// resolve_all=True)`.  TOTP pre-check is enforced — if any pending request
/// requires TOTP, the entire batch is rejected before any mutation.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub(crate) struct ApproveAllForSessionRequest {
    /// Optional count of approvals the caller expects to be pending.
    /// If provided, the server verifies the actual pending count matches
    /// before approving.  Returns 409 Conflict if the count changed.
    #[serde(default)]
    pub expected_count: Option<usize>,
    /// Optional list of approval IDs the caller expects to be pending.
    /// If provided, the server verifies the actual pending set matches before
    /// approving.  Returns 409 Conflict if a new high-risk approval landed
    /// between the operator viewing the list and clicking approve_all.
    #[serde(default)]
    #[schema(value_type = Option<Vec<String>>)]
    pub expected_ids: Option<Vec<uuid::Uuid>>,
}

/// POST /api/approvals/session/{session_id}/approve_all — Approve all pending
/// approvals for the given session atomically.
#[utoipa::path(
    post,
    path = "/api/approvals/session/{session_id}/approve_all",
    tag = "approvals",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = ApproveAllForSessionRequest,
    responses(
        (status = 200, description = "All pending session approvals approved", body = crate::types::JsonObject),
        (status = 400, description = "TOTP required for one or more items", body = crate::types::JsonObject),
        (status = 409, description = "Pending set changed since request was issued", body = crate::types::JsonObject),
    )
)]
#[allow(private_interfaces)]
pub async fn approve_all_for_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    body: Option<Json<ApproveAllForSessionRequest>>,
) -> impl IntoResponse {
    let req = body
        .map(|Json(r)| r)
        .unwrap_or(ApproveAllForSessionRequest {
            expected_count: None,
            expected_ids: None,
        });
    // Validate session_id is not empty/whitespace.
    if session_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id must not be empty or whitespace"})),
        );
    }
    // Reject excessively long session_id values to prevent DoS via memory/log amplification.
    if session_id.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id must not exceed 256 bytes"})),
        );
    }

    // Collect pending IDs and pre-check for TOTP blockers.
    let pending = state
        .kernel
        .approvals()
        .list_pending_for_session(&session_id);

    // Confirmation check: verify pending count matches expected_count if provided.
    if let Some(expected_count) = req.expected_count {
        if pending.len() != expected_count {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "Pending approval count has changed since this request was issued. Refresh and try again.",
                    "pending_count": pending.len(),
                    "expected_count": expected_count,
                })),
            );
        }
    }

    // Confirmation check: verify pending set matches expected_ids if provided.
    // Always validate when expected_ids is Some(…), even for an empty slice —
    // a caller asserting "there are zero pending approvals" must be protected too.
    if let Some(ref expected) = req.expected_ids {
        let pending_ids: std::collections::HashSet<_> = pending.iter().map(|r| r.id).collect();
        let expected_set: std::collections::HashSet<_> = expected.iter().cloned().collect();
        if pending_ids != expected_set {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "Pending approval set has changed since this request was issued. Refresh and try again.",
                    "pending_ids": pending_ids,
                    "expected_ids": expected_set,
                })),
            );
        }
    }

    // TOTP pre-check: reject entire batch if any item requires TOTP.
    let policy = state.kernel.approvals().policy();
    if pending
        .iter()
        .any(|req| policy.tool_requires_totp(&req.tool_name))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Session contains approvals that require TOTP. Approve those individually.",
            })),
        );
    }

    // Resolve each pending request through the kernel so deferred executions
    // are properly spawned (resolve_tool_approval calls handle_approval_resolution
    // for each deferred payload).
    // Reuse the `pending` list collected above — avoids a TOCTOU race where the
    // set could change between the pre-check and the resolve loop.
    let mut resolved = 0usize;
    for pending_req in pending {
        if state
            .kernel
            .resolve_tool_approval(
                pending_req.id,
                librefang_types::approval::ApprovalDecision::Approved,
                Some("api".to_string()),
                false,
                None,
            )
            .await
            .is_ok()
        {
            // Non-existent / already-resolved items are skipped silently.
            resolved += 1;
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": session_id,
            "resolved": resolved,
            "decision": "approved",
        })),
    )
}

/// POST /api/approvals/session/{session_id}/reject_all — Reject all pending
/// approvals for the given session atomically.
///
/// Mirrors Hermes-Agent's `resolve_gateway_approval(session_key, "deny",
/// resolve_all=True)`.
#[utoipa::path(
    post,
    path = "/api/approvals/session/{session_id}/reject_all",
    tag = "approvals",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "All pending session approvals rejected", body = crate::types::JsonObject)
    )
)]
pub async fn reject_all_for_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    // Validate session_id is not empty/whitespace.
    if session_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id must not be empty or whitespace"})),
        );
    }
    // Reject excessively long session_id values to prevent DoS via memory/log amplification.
    if session_id.len() > 256 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id must not exceed 256 bytes"})),
        );
    }

    // Route through resolve_tool_approval for each request so deferred
    // executions are properly handled (even though rejection means the deferred
    // will never run, this keeps the code path consistent).
    let mut resolved = 0usize;
    for pending_req in state
        .kernel
        .approvals()
        .list_pending_for_session(&session_id)
    {
        if state
            .kernel
            .resolve_tool_approval(
                pending_req.id,
                librefang_types::approval::ApprovalDecision::Denied,
                Some("api".to_string()),
                false,
                None,
            )
            .await
            .is_ok()
        {
            resolved += 1;
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": session_id,
            "resolved": resolved,
            "decision": "rejected",
        })),
    )
}

/// GET /api/approvals/audit — Query the persistent approval audit log.
#[derive(serde::Deserialize)]
pub struct AuditQueryParams {
    #[serde(default = "default_audit_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
    agent_id: Option<String>,
    tool_name: Option<String>,
}

fn default_audit_limit() -> usize {
    50
}

#[utoipa::path(get, path = "/api/approvals/audit", tag = "approvals", params(("limit" = Option<usize>, Query, description = "Max entries"), ("offset" = Option<usize>, Query, description = "Offset"), ("agent_id" = Option<String>, Query, description = "Filter by agent"), ("tool_name" = Option<String>, Query, description = "Filter by tool")), responses((status = 200, description = "Audit log entries", body = crate::types::JsonObject)))]
pub async fn audit_log(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuditQueryParams>,
) -> impl IntoResponse {
    const MAX_AUDIT_LIMIT: usize = 500;
    let limit = params.limit.min(MAX_AUDIT_LIMIT);
    let entries = state.kernel.approvals().query_audit(
        limit,
        params.offset,
        params.agent_id.as_deref(),
        params.tool_name.as_deref(),
    );
    let total = state
        .kernel
        .approvals()
        .audit_count(params.agent_id.as_deref(), params.tool_name.as_deref());

    Json(serde_json::json!({
        "items": entries,
        "total": total,
        "offset": params.offset,
        "limit": limit,
    }))
}

/// GET /api/approvals/count — Lightweight pending count for notification badges.
#[utoipa::path(get, path = "/api/approvals/count", tag = "approvals", responses((status = 200, description = "Pending approval count", body = crate::types::JsonObject)))]
pub async fn approval_count(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = state.kernel.approvals().pending_count();
    Json(serde_json::json!({"pending": pending}))
}

// ---------------------------------------------------------------------------
// TOTP setup endpoints
// ---------------------------------------------------------------------------

/// POST /api/approvals/totp/setup — Generate a new TOTP secret and return a provisioning URI.
///
/// The secret is stored in the vault but not yet active. The user must call
/// `/api/approvals/totp/confirm` with a valid code to activate TOTP.
///
/// If TOTP is already confirmed, the request body must include a valid
/// `current_code` (TOTP or recovery code) to authorize the reset.
#[derive(serde::Deserialize, Default)]
pub struct TotpSetupBody {
    /// Required when resetting an already-confirmed TOTP enrollment.
    #[serde(default)]
    current_code: Option<String>,
}

pub async fn totp_setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TotpSetupBody>,
) -> impl IntoResponse {
    // #3621: setup uses its own lockout bucket so a hostile actor cannot
    // exhaust the shared `api_admin` lockout (used by every other TOTP entry
    // surface) just by spamming setup attempts. The owner-only middleware
    // gate (see `is_owner_only_write`) already keeps non-Owner roles out.
    const SETUP_LOCKOUT_KEY: &str = "api_admin_totp_setup";
    let totp_issuer = state.kernel.approvals().policy().totp_issuer.clone();
    // If TOTP is already confirmed, require verification of the old code
    let already_confirmed = state.kernel.vault_get("totp_confirmed").as_deref() == Some("true");

    if already_confirmed {
        if state
            .kernel
            .approvals()
            .is_totp_locked_out(SETUP_LOCKOUT_KEY)
        {
            return ApiErrorResponse::bad_request(
                "Too many failed TOTP attempts. Try again later.",
            )
            .into_json_tuple();
        }
        match body.current_code.as_deref() {
            None => {
                return ApiErrorResponse::bad_request(
                    "TOTP is already enrolled. Provide current_code (TOTP or recovery code) to reset.",
                )
                .into_json_tuple();
            }
            Some(code) => {
                let verified = if state.kernel.approvals().recovery_code_format_matches(code) {
                    // Atomically redeem the recovery code (fixes TOCTOU #3560 / #3633).
                    match state.kernel.vault_redeem_recovery_code(code) {
                        Ok(matched) => matched,
                        Err(e) => {
                            return ApiErrorResponse::internal(e).into_json_tuple();
                        }
                    }
                } else {
                    // TOTP code — check replay before verifying (#3359).
                    if state.kernel.approvals().is_totp_code_used(code) {
                        // Atomic check + record (#3584) preserves fail-secure
                        // on DB persist failure (#3372): Err(false) = DB write
                        // dropped; Err(true) = already locked out, fall through
                        // to "already used" response so the lockout state is
                        // not leaked here.
                        if let Err(false) = state
                            .kernel
                            .approvals()
                            .check_and_record_totp_failure(SETUP_LOCKOUT_KEY)
                        {
                            return ApiErrorResponse::internal(
                                "Failed to persist TOTP failure counter",
                            )
                            .into_json_tuple();
                        }
                        return ApiErrorResponse::bad_request(
                            "TOTP code has already been used. Wait for the next 30-second window.",
                        )
                        .into_json_tuple();
                    }
                    match state.kernel.vault_get("totp_secret") {
                        Some(secret) => {
                            let ok =
                                crate::approval::ApprovalManager::verify_totp_code_with_issuer(
                                    &secret,
                                    code,
                                    &totp_issuer,
                                )
                                .unwrap_or(false);
                            if ok {
                                state.kernel.approvals().record_totp_code_used(code);
                            }
                            ok
                        }
                        None => false,
                    }
                };
                if !verified {
                    // Fail-secure: atomically check lockout + record failure (#3372/#3584).
                    match state
                        .kernel
                        .approvals()
                        .check_and_record_totp_failure(SETUP_LOCKOUT_KEY)
                    {
                        Err(true) => {
                            return ApiErrorResponse::bad_request(
                                "Too many failed TOTP attempts. Try again later.",
                            )
                            .into_json_tuple();
                        }
                        Err(false) => {
                            return ApiErrorResponse::internal(
                                "Failed to persist TOTP failure counter",
                            )
                            .into_json_tuple();
                        }
                        Ok(()) => {}
                    }
                    return ApiErrorResponse::bad_request(
                        "Invalid current_code. Provide a valid TOTP or recovery code to reset.",
                    )
                    .into_json_tuple();
                }
            }
        }
    }

    // Reject overwrite of a pending (not yet confirmed) TOTP enrollment.
    // `totp_secret` present but `totp_confirmed` != "true" means a setup
    // was initiated by a previous call but the confirm step was never completed.
    // Allowing a second setup call here would silently discard the first QR
    // code, making the first caller's authenticator app permanently invalid
    // without any indication.
    let pending_setup = state.kernel.vault_get("totp_secret").is_some()
        && state.kernel.vault_get("totp_confirmed").as_deref() != Some("true");
    if pending_setup {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "TOTP enrollment already in progress — confirm the existing setup or revoke it first",
                "status": "pending_confirmation",
            })),
        );
    }

    let (secret_base32, otpauth_uri, qr_base64) = match state
        .kernel
        .approvals()
        .new_totp_secret(&totp_issuer, "admin")
    {
        Ok(v) => v,
        Err(e) => {
            return ApiErrorResponse::internal(e).into_json_tuple();
        }
    };
    let qr_data_uri = format!("data:image/png;base64,{qr_base64}");

    // Generate recovery codes
    let recovery_codes = state.kernel.approvals().new_recovery_codes();
    let recovery_json = serde_json::to_string(&recovery_codes).unwrap_or_default();

    // Store secret and recovery codes in vault (not yet active — totp_confirmed = false)
    if let Err(e) = state.kernel.vault_set("totp_secret", &secret_base32) {
        return ApiErrorResponse::internal(e).into_json_tuple();
    }
    if let Err(e) = state.kernel.vault_set("totp_confirmed", "false") {
        return ApiErrorResponse::internal(e).into_json_tuple();
    }
    if let Err(e) = state
        .kernel
        .vault_set("totp_recovery_codes", &recovery_json)
    {
        return ApiErrorResponse::internal(e).into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "otpauth_uri": otpauth_uri,
            "secret": secret_base32,
            "qr_code": qr_data_uri,
            "recovery_codes": recovery_codes,
            "message": "Scan the QR code or enter the secret in your authenticator app, then call POST /api/approvals/totp/confirm with a valid code. Save your recovery codes in a safe place."
        })),
    )
}

/// POST /api/approvals/totp/confirm — Confirm TOTP enrollment by verifying a code.
#[derive(serde::Deserialize)]
pub struct TotpConfirmBody {
    code: String,
}

pub async fn totp_confirm(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TotpConfirmBody>,
) -> impl IntoResponse {
    let totp_issuer = state.kernel.approvals().policy().totp_issuer.clone();
    if state.kernel.approvals().is_totp_locked_out("api_admin") {
        return ApiErrorResponse::bad_request("Too many failed TOTP attempts. Try again later.")
            .into_json_tuple();
    }

    let secret = match state.kernel.vault_get("totp_secret") {
        Some(s) => s,
        None => {
            return ApiErrorResponse::bad_request(
                "No TOTP secret found. Run POST /api/approvals/totp/setup first.",
            )
            .into_json_tuple();
        }
    };

    // Replay-prevention check (#3359): reject a code already used in the last 60 s.
    if state.kernel.approvals().is_totp_code_used(&body.code) {
        // Atomic check + record (#3584) preserves fail-secure on DB persist
        // failure (#3372): Err(false) = DB write dropped; Err(true) = already
        // locked out, fall through to "already used" response so the lockout
        // state is not leaked here.
        if let Err(false) = state
            .kernel
            .approvals()
            .check_and_record_totp_failure("api_admin")
        {
            return ApiErrorResponse::internal("Failed to persist TOTP failure counter")
                .into_json_tuple();
        }
        return ApiErrorResponse::bad_request(
            "TOTP code has already been used. Wait for the next 30-second window.",
        )
        .into_json_tuple();
    }
    match crate::approval::ApprovalManager::verify_totp_code_with_issuer(
        &secret,
        &body.code,
        &totp_issuer,
    ) {
        Ok(true) => {
            state.kernel.approvals().record_totp_code_used(&body.code);
            if let Err(e) = state.kernel.vault_set("totp_confirmed", "true") {
                return ApiErrorResponse::internal(e).into_json_tuple();
            }
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "confirmed", "message": "TOTP is now active. Set second_factor = \"totp\" in your config to enforce it."}),
                ),
            )
        }
        Ok(false) => {
            // Fail-secure: atomically check lockout + record failure (#3372/#3584).
            match state
                .kernel
                .approvals()
                .check_and_record_totp_failure("api_admin")
            {
                Err(true) => {
                    return ApiErrorResponse::bad_request(
                        "Too many failed TOTP attempts. Try again later.",
                    )
                    .into_json_tuple();
                }
                Err(false) => {
                    return ApiErrorResponse::internal("Failed to persist TOTP failure counter")
                        .into_json_tuple();
                }
                Ok(()) => {}
            }
            ApiErrorResponse::bad_request(
                "Invalid TOTP code. Check your authenticator app and try again.",
            )
            .into_json_tuple()
        }
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

/// GET /api/approvals/totp/status — Check TOTP enrollment status.
pub async fn totp_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let has_secret = state
        .kernel
        .vault_get("totp_secret")
        .is_some_and(|s| !s.is_empty());
    let confirmed = state.kernel.vault_get("totp_confirmed").as_deref() == Some("true");
    let policy = state.kernel.approvals().policy();
    let sf = policy.second_factor;
    let enforced = sf != librefang_types::approval::SecondFactor::None;

    let remaining_recovery = state
        .kernel
        .vault_get("totp_recovery_codes")
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| v.len())
        .unwrap_or(0);

    Json(serde_json::json!({
        "enrolled": has_secret,
        "confirmed": confirmed,
        "enforced": enforced,
        "scope": serde_json::to_value(sf).unwrap_or(serde_json::json!("none")),
        "remaining_recovery_codes": remaining_recovery,
    }))
}

/// POST /api/approvals/totp/revoke — Revoke TOTP enrollment.
///
/// Requires a valid TOTP or recovery code to authorize revocation.
#[derive(serde::Deserialize)]
pub struct TotpRevokeBody {
    code: String,
}

pub async fn totp_revoke(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TotpRevokeBody>,
) -> impl IntoResponse {
    // #3621: revoke uses its own lockout bucket so failed code attempts on
    // this path cannot exhaust the shared `api_admin` lockout (used by every
    // other TOTP entry surface) and DoS legitimate approve/login flows.
    const REVOKE_LOCKOUT_KEY: &str = "api_admin_totp_revoke";
    let totp_issuer = state.kernel.approvals().policy().totp_issuer.clone();
    if state
        .kernel
        .approvals()
        .is_totp_locked_out(REVOKE_LOCKOUT_KEY)
    {
        return ApiErrorResponse::bad_request("Too many failed TOTP attempts. Try again later.")
            .into_json_tuple();
    }

    let confirmed = state.kernel.vault_get("totp_confirmed").as_deref() == Some("true");

    if !confirmed {
        return ApiErrorResponse::bad_request("TOTP is not enrolled.").into_json_tuple();
    }

    // Verify the provided code (recovery codes are consumed on use).
    // For recovery codes, use the atomic vault_redeem_recovery_code path
    // (fixes TOCTOU #3560 and vault_set-failure bypass #3633).
    let verified = if state
        .kernel
        .approvals()
        .recovery_code_format_matches(&body.code)
    {
        match state.kernel.vault_redeem_recovery_code(&body.code) {
            Ok(matched) => matched,
            Err(e) => {
                return ApiErrorResponse::internal(e).into_json_tuple();
            }
        }
    } else {
        // TOTP replay check first (#3952).  Most damaging path of all:
        // a single replayed code disables 2FA entirely.  approve_request
        // and totp_confirm both check this; totp_revoke was missed.
        if state.kernel.approvals().is_totp_code_used(&body.code) {
            // Don't count toward the lockout — the code itself isn't
            // wrong, it's already-spent.  Return the same 400 shape so
            // the caller can't distinguish "already used" from "wrong".
            return ApiErrorResponse::bad_request("TOTP code already used. Wait for a new code.")
                .into_json_tuple();
        }
        match state.kernel.vault_get("totp_secret") {
            Some(secret) => {
                let ok = crate::approval::ApprovalManager::verify_totp_code_with_issuer(
                    &secret,
                    &body.code,
                    &totp_issuer,
                )
                .unwrap_or(false);
                if ok {
                    // Mark consumption only after a true verify.
                    state.kernel.approvals().record_totp_code_used(&body.code);
                }
                ok
            }
            None => false,
        }
    };

    if !verified {
        // Fail-secure: atomically check lockout + record failure (#3372/#3584).
        match state
            .kernel
            .approvals()
            .check_and_record_totp_failure(REVOKE_LOCKOUT_KEY)
        {
            Err(true) => {
                return ApiErrorResponse::bad_request(
                    "Too many failed TOTP attempts. Try again later.",
                )
                .into_json_tuple();
            }
            Err(false) => {
                return ApiErrorResponse::internal("Failed to persist TOTP failure counter")
                    .into_json_tuple();
            }
            Ok(()) => {}
        }
        return ApiErrorResponse::bad_request(
            "Invalid code. Provide a valid TOTP or recovery code.",
        )
        .into_json_tuple();
    }

    // #3633: clearing must not be best-effort and must avoid creating a
    // partial state that BYPASSES 2FA on login. The login gate
    // (server.rs ~334) reads `if totp_enrolled && totp_confirmed` to decide
    // whether to prompt for a TOTP code, so:
    //   * `totp_confirmed=false` alone is enough to disable 2FA on login,
    //     even if `totp_secret` is still present.
    // An earlier fail-fast version cleared `totp_confirmed` first and
    // returned 500 if `totp_secret` then failed to wipe — that
    // simultaneously told the user "TOTP is still active, retry" while
    // actually disabling 2FA. To prevent that, we:
    //   1. Wipe `totp_secret` and `totp_recovery_codes` FIRST so the
    //      verify path is dead before we ever flip the `totp_confirmed`
    //      flag. Even if writing the flag later fails, secret/recovery are
    //      already gone, so a retry is purely a state-flag fix and 2FA is
    //      effectively disabled either way.
    //   2. Attempt every write (collect-all, not fail-fast) so a transient
    //      failure on field N doesn't leave fields >N untouched on retry.
    //   3. Aggregate failures into a single 500 with all field errors.
    let mut failures: Vec<String> = Vec::new();
    if let Err(e) = state.kernel.vault_set("totp_secret", "") {
        tracing::error!("totp_revoke: failed to clear totp_secret: {e}");
        failures.push(format!("totp_secret: {e}"));
    }
    if let Err(e) = state.kernel.vault_set("totp_recovery_codes", "[]") {
        tracing::error!("totp_revoke: failed to clear totp_recovery_codes: {e}");
        failures.push(format!("totp_recovery_codes: {e}"));
    }
    if let Err(e) = state.kernel.vault_set("totp_confirmed", "false") {
        tracing::error!("totp_revoke: failed to clear totp_confirmed: {e}");
        failures.push(format!("totp_confirmed: {e}"));
    }
    if !failures.is_empty() {
        return ApiErrorResponse::internal(format!(
            "TOTP revocation partially failed; the secret and recovery codes have been wiped first so 2FA is no longer verifiable, but vault state is inconsistent. Retry. Details: {}",
            failures.join("; ")
        ))
        .into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "revoked",
            "message": "TOTP has been revoked. Set second_factor = \"none\" in config to disable enforcement."
        })),
    )
}
