use super::*;

// ---------------------------------------------------------------------------
// Shared manifest resolution helper
// ---------------------------------------------------------------------------
/// Maximum manifest size (1MB) to prevent parser memory exhaustion.
const MAX_MANIFEST_SIZE: usize = 1024 * 1024;

/// Resolved manifest ready for spawning.
struct ResolvedManifest {
    manifest: AgentManifest,
    name: String,
}

/// Error from manifest resolution — carries a user-facing message.
struct ManifestError {
    message: String,
}

/// Resolve a `SpawnRequest` into a parsed `AgentManifest`.
///
/// Handles template lookup, path sanitization, size guard, signed manifest
/// verification, and TOML parsing — shared by both single and bulk spawn.
async fn resolve_manifest(
    state: &AppState,
    req: &SpawnRequest,
    lang: &'static str,
) -> Result<ResolvedManifest, ManifestError> {
    // Resolve template name → manifest_toml
    let manifest_toml = if req.manifest_toml.trim().is_empty() {
        if let Some(ref tmpl_name) = req.template {
            let safe_name: String = tmpl_name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if safe_name.is_empty() || safe_name != *tmpl_name {
                let t = ErrorTranslator::new(lang);
                return Err(ManifestError {
                    message: t.t("api-error-template-invalid-name"),
                });
            }
            let tmpl_path = state
                .kernel
                .config_ref()
                .home_dir
                .join("workspaces")
                .join("agents")
                .join(&safe_name)
                .join("agent.toml");
            // Use tokio::fs to avoid blocking in an async context
            match tokio::fs::read_to_string(&tmpl_path).await {
                Ok(content) => content,
                Err(_) => {
                    let t = ErrorTranslator::new(lang);
                    return Err(ManifestError {
                        message: t.t_args("api-error-template-not-found", &[("name", &safe_name)]),
                    });
                }
            }
        } else {
            let t = ErrorTranslator::new(lang);
            return Err(ManifestError {
                message: t.t("api-error-template-required"),
            });
        }
    } else {
        req.manifest_toml.clone()
    };

    // Size guard
    if manifest_toml.len() > MAX_MANIFEST_SIZE {
        let t = ErrorTranslator::new(lang);
        return Err(ManifestError {
            message: t.t("api-error-manifest-too-large"),
        });
    }

    // SECURITY: Verify Ed25519 signature when provided
    if let Some(ref signed_json) = req.signed_manifest {
        match state.kernel.verify_signed_manifest(signed_json) {
            Ok(verified_toml) => {
                if verified_toml.trim() != manifest_toml.trim() {
                    tracing::warn!("Signed manifest content does not match manifest_toml");
                    let t = ErrorTranslator::new(lang);
                    return Err(ManifestError {
                        message: t.t("api-error-manifest-signature-mismatch"),
                    });
                }
            }
            Err(e) => {
                tracing::warn!("Manifest signature verification failed: {e}");
                state.kernel.audit().record(
                    "system",
                    librefang_kernel::audit::AuditAction::AuthAttempt,
                    "manifest signature verification failed",
                    format!("error: {e}"),
                );
                let t = ErrorTranslator::new(lang);
                return Err(ManifestError {
                    message: t.t("api-error-manifest-signature-failed"),
                });
            }
        }
    }

    // Parse TOML
    let mut manifest: AgentManifest = match toml::from_str(&manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse agent manifest TOML: {e}");
            let t = ErrorTranslator::new(lang);
            return Err(ManifestError {
                message: t.t("api-error-manifest-invalid-format"),
            });
        }
    };

    // Allow callers to override the manifest name, enabling multiple agents
    // from the same template with distinct names.
    if let Some(ref custom_name) = req.name {
        if !custom_name.trim().is_empty() {
            manifest.name = custom_name.trim().to_string();
        }
    }

    let name = manifest.name.clone();
    Ok(ResolvedManifest { manifest, name })
}

/// POST /api/agents — Spawn a new agent.
///
/// Honours `Idempotency-Key` (#3637): when set, a duplicate request
/// with the same key + same body replays the cached response instead
/// of spawning a second agent. A different body under the same key is
/// rejected with 409 Conflict.
#[utoipa::path(
    post,
    path = "/api/agents",
    tag = "agents",
    request_body = crate::types::SpawnRequest,
    responses(
        (status = 200, description = "Agent spawned", body = crate::types::SpawnResponse),
        (status = 400, description = "Invalid manifest"),
        (status = 409, description = "Idempotency-Key was reused with a different request body")
    )
)]
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let l = super::resolve_lang(lang.as_ref());
    let key = crate::idempotency::extract_key(&headers);
    let body_bytes: Vec<u8> = body.to_vec();
    let store = Arc::clone(&state.idempotency_store);
    let inner_body = body_bytes.clone();

    crate::idempotency::run_idempotent(
        store.as_ref(),
        key.as_deref(),
        &body_bytes,
        move || async move { spawn_agent_inner(state, l, &inner_body).await },
    )
    .await
}

/// Inner handler — produces a `(StatusCode, Vec<u8>)` snapshot suitable
/// for caching by the Idempotency-Key middleware. JSON-encodes once
/// here so the cached and replay paths share the exact same bytes.
async fn spawn_agent_inner(
    state: Arc<AppState>,
    l: &'static str,
    body_bytes: &[u8],
) -> (StatusCode, Vec<u8>) {
    let req: SpawnRequest = match serde_json::from_slice(body_bytes) {
        Ok(r) => r,
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid_json",
                format!("Invalid JSON body: {e}"),
            );
        }
    };

    let resolved = match resolve_manifest(&state, &req, l).await {
        Ok(r) => r,
        Err(e) => {
            let (status, code) = if e.message.contains("too large") {
                (StatusCode::PAYLOAD_TOO_LARGE, "manifest_too_large")
            } else if e.message.contains("not found") && e.message.contains("Template") {
                (StatusCode::NOT_FOUND, "template_not_found")
            } else if e.message.contains("signature verification failed") {
                (StatusCode::FORBIDDEN, "signature_invalid")
            } else {
                (StatusCode::BAD_REQUEST, "invalid_manifest")
            };
            return json_error(status, code, e.message);
        }
    };

    match state.kernel.spawn_agent_typed(resolved.manifest) {
        Ok(id) => {
            let body = serde_json::to_vec(&SpawnResponse {
                agent_id: id.to_string(),
                name: resolved.name,
            })
            .unwrap_or_else(|_| b"{}".to_vec());
            (StatusCode::CREATED, body)
        }
        Err(e) => {
            tracing::warn!("Spawn failed: {e}");
            let t = ErrorTranslator::new(l);
            let (status, code) = match &e {
                crate::error::KernelError::LibreFang(
                    librefang_types::error::LibreFangError::AgentAlreadyExists(_),
                ) => (StatusCode::CONFLICT, "agent_already_exists"),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "spawn_failed"),
            };
            // 409 (duplicate name) echoes the kernel message — useful
            // to the caller and free of internal detail. The 500
            // catch-all scrubs (audit: rusqlite-errors-leak): a spawn
            // failure rooted in the memory substrate would otherwise
            // leak SQL detail. Full error already logged above.
            let body = if status == StatusCode::INTERNAL_SERVER_ERROR {
                t.t("api-error-internal")
            } else {
                t.t_args("api-error-agent-error", &[("error", &e.to_string())])
            };
            json_error(status, code, body)
        }
    }
}

// `validate_bulk_size` lives at `routes/mod.rs` so non-agent bulk handlers
// (approvals, users, workflows) can reuse the same guard before they reach
// any `Vec::with_capacity(len)`. See
// `docs/issues/bulk-with-capacity-no-validate.md`.
/// POST /api/agents/bulk — Create multiple agents at once.
#[utoipa::path(
    post,
    path = "/api/agents/bulk",
    tag = "agents",
    request_body(content = BulkCreateRequest, description = "Array of agent spawn requests"),
    responses(
        (status = 200, description = "Create multiple agents at once", body = crate::types::JsonObject)
    )
)]
pub async fn bulk_create_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkCreateRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    if let Err(resp) = crate::validation::validate_bulk_size(req.agents.len(), BULK_LIMIT) {
        return resp;
    }

    let mut results: Vec<BulkCreateResult> = Vec::with_capacity(req.agents.len());

    for (index, spawn_req) in req.agents.iter().enumerate() {
        match resolve_manifest(&state, spawn_req, l).await {
            Err(e) => {
                results.push(BulkCreateResult {
                    index,
                    success: false,
                    agent_id: None,
                    name: None,
                    error: Some(e.message),
                });
            }
            Ok(resolved) => {
                let name = resolved.name.clone();
                match state.kernel.spawn_agent_typed(resolved.manifest) {
                    Ok(id) => {
                        results.push(BulkCreateResult {
                            index,
                            success: true,
                            agent_id: Some(id.to_string()),
                            name: Some(name),
                            error: None,
                        });
                    }
                    Err(e) => {
                        let t = ErrorTranslator::new(l);
                        results.push(BulkCreateResult {
                            index,
                            success: false,
                            agent_id: None,
                            name: None,
                            error: Some(t.t_args(
                                "api-error-agent-clone-spawn-failed",
                                &[("error", &e.to_string())],
                            )),
                        });
                    }
                }
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// DELETE /api/agents/bulk — Delete multiple agents at once.
#[utoipa::path(
    delete,
    path = "/api/agents/bulk",
    tag = "agents",
    request_body(content = BulkAgentIdsRequest, description = "Array of agent IDs to delete"),
    responses(
        (status = 200, description = "Delete multiple agents at once", body = crate::types::JsonObject)
    )
)]
pub async fn bulk_delete_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let t = ErrorTranslator::new(l);
    if let Err(resp) = crate::validation::validate_bulk_size(req.agent_ids.len(), BULK_LIMIT) {
        return resp;
    }

    let mut results: Vec<BulkActionResult> = Vec::with_capacity(req.agent_ids.len());

    for id_str in &req.agent_ids {
        let agent_id: AgentId = match id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-invalid-id")),
                });
                continue;
            }
        };
        // Same guard as the single-agent kill path: hand-spawned agents
        // must be removed by deactivating their owning hand, not directly.
        if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
            if entry.is_hand {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(
                        "Cannot delete a hand-spawned agent directly; deactivate or uninstall the owning hand instead.".to_string(),
                    ),
                });
                continue;
            }
        }
        match state.kernel.kill_agent_typed(agent_id) {
            Ok(()) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: true,
                    message: Some("Deleted".into()),
                    error: None,
                });
            }
            Err(e) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(scrub_500(&e, &t)),
                });
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// POST /api/agents/bulk/start — Set multiple agents to Full mode.
#[utoipa::path(
    post,
    path = "/api/agents/bulk/start",
    tag = "agents",
    request_body(content = BulkAgentIdsRequest, description = "Array of agent IDs to start"),
    responses(
        (status = 200, description = "Start multiple agents (set to Full mode)", body = crate::types::JsonObject)
    )
)]
pub async fn bulk_start_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    use librefang_types::agent::AgentMode;

    let l = super::resolve_lang(lang.as_ref());
    let t = ErrorTranslator::new(l);
    if let Err(resp) = crate::validation::validate_bulk_size(req.agent_ids.len(), BULK_LIMIT) {
        return resp;
    }

    let mut results: Vec<BulkActionResult> = Vec::with_capacity(req.agent_ids.len());

    for id_str in &req.agent_ids {
        let agent_id: AgentId = match id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-invalid-id")),
                });
                continue;
            }
        };
        match state
            .kernel
            .agent_registry()
            .set_mode(agent_id, AgentMode::Full)
        {
            Ok(()) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: true,
                    message: Some("Agent set to Full mode".into()),
                    error: None,
                });
            }
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-not-found")),
                });
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// POST /api/agents/bulk/stop — Stop multiple agents' current runs.
#[utoipa::path(
    post,
    path = "/api/agents/bulk/stop",
    tag = "agents",
    request_body(content = BulkAgentIdsRequest, description = "Array of agent IDs to stop"),
    responses(
        (status = 200, description = "Stop multiple agents' current runs", body = crate::types::JsonObject)
    )
)]
pub async fn bulk_stop_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<BulkAgentIdsRequest>,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let t = ErrorTranslator::new(l);
    if let Err(resp) = crate::validation::validate_bulk_size(req.agent_ids.len(), BULK_LIMIT) {
        return resp;
    }

    let mut results: Vec<BulkActionResult> = Vec::with_capacity(req.agent_ids.len());

    for id_str in &req.agent_ids {
        let agent_id: AgentId = match id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(t.t("api-error-agent-invalid-id")),
                });
                continue;
            }
        };
        match state.kernel.stop_agent_run(agent_id) {
            Ok(cancelled) => {
                let msg = if cancelled {
                    "Run cancelled"
                } else {
                    "No active run"
                };
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: true,
                    message: Some(msg.into()),
                    error: None,
                });
            }
            Err(e) => {
                results.push(BulkActionResult {
                    agent_id: id_str.clone(),
                    success: false,
                    message: None,
                    error: Some(scrub_500(&e, &t)),
                });
            }
        }
    }

    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "succeeded": succeeded,
            "failed": total - succeeded,
            "results": results,
        })),
    )
}

/// GET /api/agents — List agents with optional filtering, pagination, and sorting.
///
/// Query parameters (all optional — omitting them returns all agents):
///   - `q`: free-text search across name and description (case-insensitive)
///   - `status`: filter by lifecycle state (e.g. "running", "suspended")
///   - `limit` / `offset`: pagination
///   - `sort`: field to sort by — "name", "created_at", "last_active", "state"
///   - `order`: "asc" (default) or "desc"
#[utoipa::path(
    get,
    path = "/api/agents",
    tag = "agents",
    params(
        ("q" = Option<String>, Query, description = "Free-text search on name/description"),
        ("status" = Option<String>, Query, description = "Filter by agent state"),
        ("limit" = Option<usize>, Query, description = "Max items to return"),
        ("offset" = Option<usize>, Query, description = "Items to skip"),
        ("sort" = Option<String>, Query, description = "Sort field: name, created_at, last_active, state"),
        ("order" = Option<String>, Query, description = "Sort order: asc or desc"),
    ),
    responses(
        (status = 200, description = "Paginated list of agents")
    )
)]
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Query(mut params): Query<AgentListQuery>,
) -> impl IntoResponse {
    // Scope agents by authenticated user: non-admin/owner callers can only
    // list agents they authored.  If the caller already supplied an explicit
    // ?owner= filter we respect it as-is; otherwise we inject the caller's
    // username automatically.
    if params.owner.is_none() {
        if let Some(ref user) = api_user {
            use crate::middleware::UserRole;
            if user.0.role < UserRole::Admin {
                params.owner = Some(user.0.name.clone());
            }
        }
    }
    let catalog_guard = state.kernel.model_catalog_ref().load();
    let catalog: Option<&librefang_kernel::model_catalog::ModelCatalog> = Some(&catalog_guard);
    let dm = {
        let dm_override = state
            .kernel
            .default_model_override_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        effective_default_model(
            &state.kernel.config_ref().default_model,
            dm_override.as_ref(),
        )
    };

    // #3569: dashboard hot path. Switch to `list_arcs()` so we share Arc
    // pointers with the registry instead of deep-cloning every manifest
    // (12+ Vecs/HashMaps) on each refresh — at 50 agents and a 20-30s
    // dashboard poll that was the dominant allocator on this handler.
    let mut agents: Vec<std::sync::Arc<librefang_types::agent::AgentEntry>> =
        state.kernel.agent_registry().list_arcs();

    // -- Filtering --
    // Exclude hand agents by default; pass ?include_hands=true to include them.
    if !params.include_hands.unwrap_or(false) {
        agents.retain(|e| !e.is_hand);
    }

    if let Some(ref q) = params.q {
        let q_lower = q.to_lowercase();
        agents.retain(|e| {
            e.name.to_lowercase().contains(&q_lower)
                || e.manifest.description.to_lowercase().contains(&q_lower)
        });
    }

    if let Some(ref status) = params.status {
        let status_lower = status.to_lowercase();
        agents.retain(|e| format!("{:?}", e.state).to_lowercase() == status_lower);
    }

    // Filter by owner (matches manifest.author). For non-admin callers this
    // is injected automatically above so they only see their own agents.
    if let Some(ref owner) = params.owner {
        let owner_lower = owner.to_lowercase();
        agents.retain(|e| e.manifest.author.to_lowercase() == owner_lower);
    }

    let total = agents.len();

    // -- Sorting --
    const VALID_SORT_FIELDS: &[&str] = &["name", "created_at", "last_active", "state"];
    let sort_field = params.sort.as_deref().unwrap_or("name");
    if !VALID_SORT_FIELDS.contains(&sort_field) {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        let msg = t.t_args(
            "api-error-agent-invalid-sort",
            &[
                ("field", sort_field),
                ("valid", &format!("{:?}", VALID_SORT_FIELDS)),
            ],
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": msg
            })),
        )
            .into_response();
    }
    let descending = params
        .order
        .as_deref()
        .map(|o| o.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);

    agents.sort_by(|a, b| {
        let cmp = match sort_field {
            "created_at" => a.created_at.cmp(&b.created_at),
            "last_active" => a.last_active.cmp(&b.last_active),
            "state" => format!("{:?}", a.state).cmp(&format!("{:?}", b.state)),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if descending {
            cmp.reverse()
        } else {
            cmp
        }
    });

    // -- Pagination --
    //
    // Audit: agent-list-limit-none-unbounded. Before, `limit = None`
    // meant "return every agent without truncation", and a
    // multi-thousand-agent deployment turned this endpoint into a
    // memory + JSON-serialization DoS sink. Now `None` defaults to
    // `DEFAULT_AGENT_LIST_LIMIT` and an explicit `Some(n)` still
    // clamps at `MAX_AGENT_LIST_LIMIT` (the historical ceiling). The
    // `total` field on the paginated response already lets callers
    // detect overflow and page.
    let offset = params.offset.unwrap_or(0);
    let limit = params
        .limit
        .unwrap_or(DEFAULT_AGENT_LIST_LIMIT)
        .min(MAX_AGENT_LIST_LIMIT);
    let agents: Vec<std::sync::Arc<librefang_types::agent::AgentEntry>> =
        agents.into_iter().skip(offset).take(limit).collect();

    // Bulk-fetch 24h sessions/cost so each row carries its own KPI without
    // forcing the dashboard to re-aggregate from /api/sessions (which is
    // pagination-clipped).
    let bulk_stats = state.kernel.memory_substrate().agents_stats_24h_bulk().ok();

    // `e` is &Arc<AgentEntry>; `as_ref()` on Arc yields the &AgentEntry the
    // helper expects without forcing a manifest deep-clone (#3569).
    let items: Vec<serde_json::Value> = agents
        .iter()
        .map(|e| enrich_agent_json(e.as_ref(), &dm, catalog, bulk_stats.as_ref()))
        .collect();

    Json(PaginatedResponse {
        items,
        total,
        offset,
        // The server-applied cap is now always finite (see the
        // pagination block above) so the response envelope reports
        // it as `Some` instead of the historical `None`.
        limit: Some(limit),
    })
    .into_response()
}

/// Query parameters for `DELETE /api/agents/{id}` (refs #4614).
///
/// `confirm = true` is required by the canonical-UUID registry design — a
/// bare DELETE is rejected with `409 Conflict` so a typo, replayed
/// request, or dashboard click-bug can't silently destroy history. When
/// `confirm=true` the agent is killed AND its `name → canonical_uuid`
/// binding is purged from `agent_identities.toml` (i.e. the next spawn
/// under the same name lands on a fresh UUID; prior sessions / memories
/// are orphaned).
#[derive(Debug, Default, serde::Deserialize)]
pub struct DeleteAgentQuery {
    #[serde(default)]
    pub confirm: bool,
}

/// Warning text shown when a DELETE arrives without confirmation. Mirrors
/// the prompt copy in the issue body so CLI / API / dashboard surface the
/// same wording.
const DELETE_AGENT_WARNING: &str = "Deleting this agent will permanently remove its canonical UUID and all associated memories and sessions. This action cannot be undone. Re-issue with confirm=true to proceed.";

/// DELETE /api/agents/:id — Kill an agent (refs #4614).
///
/// Idempotent (RFC 9110 §9.2.2 / §9.3.5): deleting an agent that is already
/// gone returns `200 OK` with `{"status": "already-deleted"}` instead of
/// `404`. `404` is reserved for the malformed-UUID case alone, so retried
/// or replayed DELETEs by clients (network blips, dashboard double-clicks)
/// no longer surface a phantom error. Refs #3509.
///
/// Refs #4614 — canonical agent UUID registry. Explicit deletes via this
/// endpoint require `confirm=true` (as a query param or JSON body field).
/// Without it the request is rejected with `409 Conflict` and the
/// data-loss warning text. With confirmation, the kernel kills the agent
/// AND purges its canonical UUID binding so the next spawn under the
/// same name lands on a fresh UUID. Internal lifecycle resets (hot
/// reload, panic restart) call `kill_agent` directly and preserve the
/// binding — the destructive purge only happens when an operator
/// explicitly asks for it.
#[utoipa::path(
    delete,
    path = "/api/agents/{id}",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("confirm" = Option<bool>, Query, description = "Required: confirms canonical UUID purge. Refs #4614.")
    ),
    responses(
        (status = 200, description = "Agent killed and canonical UUID purged"),
        (status = 400, description = "Malformed agent ID"),
        (status = 409, description = "Confirmation required, or agent is hand-owned")
    )
)]
pub async fn kill_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<DeleteAgentQuery>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .with_code("invalid_agent_id")
                .into_response();
        }
    };

    // Idempotent-no-op short-circuit: a DELETE for an already-absent agent is
    // a no-op per RFC 9110 §9.2.2, so we don't gate it on `?confirm=true` —
    // there's nothing to confirm destroying. Hand-owned and confirmation
    // checks only apply when the agent actually exists.
    match state.kernel.agent_registry().get(agent_id) {
        Some(entry) if entry.is_hand => {
            return ApiErrorResponse::conflict(
                "Cannot delete a hand-spawned agent directly; deactivate or uninstall the owning hand instead.",
            )
            .with_code("hand_agent_delete_denied")
            .into_response();
        }
        Some(_) => {
            // Refs #4614: destructive delete of an existing agent requires
            // explicit confirmation via `?confirm=true`. Without it the
            // request is rejected with 409 Conflict + the data-loss warning
            // text so a typo / replay / click-bug can't silently destroy
            // history.
            if !q.confirm {
                return ApiErrorResponse::conflict(DELETE_AGENT_WARNING)
                    .with_code("delete_confirmation_required")
                    .into_response();
            }
        }
        None => {
            // Idempotent DELETE: the agent is already gone (replayed request,
            // double-click, race with another deleter). Treat as success per
            // RFC 9110 §9.2.2 — DELETE is idempotent.
            return crate::extensions::with_agent_id(
                agent_id,
                (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "already-deleted", "agent_id": id})),
                ),
            );
        }
    }

    // Confirmed delete: kill + purge canonical UUID binding (refs #4614).
    let body = match state.kernel.kill_agent_with_purge(agent_id, true) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "killed",
                "agent_id": id,
                "identity_purged": true,
            })),
        )
            .into_response(),
        Err(e) => {
            // The agent existed when we checked above but vanished mid-flight
            // (concurrent delete). Still treat as idempotent success — the
            // caller's intent ("agent {id} should be gone") is satisfied.
            if matches!(
                e,
                crate::error::KernelError::LibreFang(
                    librefang_types::error::LibreFangError::AgentNotFound(_)
                )
            ) {
                tracing::debug!(
                    "kill_agent: agent {id} vanished mid-flight; treating as already-deleted"
                );
                return crate::extensions::with_agent_id(
                    agent_id,
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"status": "already-deleted", "agent_id": id})),
                    ),
                );
            }
            tracing::warn!("kill_agent failed for {id}: {e}");
            ApiErrorResponse::internal_scrub(e)
                .with_code("agent_kill_failed")
                .into_response()
        }
    };
    // #3511: tag response so request_logging middleware can emit
    // `agent_id` as a structured field on the access-log line.
    crate::extensions::with_agent_id(agent_id, body)
}

/// PUT /api/agents/:id/suspend — Suspend an agent (stops cron, keeps in registry).
#[utoipa::path(put, path = "/api/agents/{id}/suspend", tag = "agents", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Agent suspended")))]
pub async fn suspend_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid agent ID")
                .with_code("invalid_agent_id")
                .into_response();
        }
    };
    let body = match state.kernel.suspend_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "suspended", "agent_id": id})),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::not_found(e.to_string())
            .with_code("agent_not_found")
            .into_response(),
    };
    crate::extensions::with_agent_id(agent_id, body)
}

/// PUT /api/agents/:id/resume — Resume a suspended agent.
#[utoipa::path(put, path = "/api/agents/{id}/resume", tag = "agents", params(("id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Agent resumed")))]
pub async fn resume_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request("Invalid agent ID")
                .with_code("invalid_agent_id")
                .into_response();
        }
    };
    let body = match state.kernel.resume_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "running", "agent_id": id})),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::not_found(e.to_string())
            .with_code("agent_not_found")
            .into_response(),
    };
    crate::extensions::with_agent_id(agent_id, body)
}

/// PUT /api/agents/:id/mode — Change an agent's operational mode.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/mode",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = SetModeRequest, description = "New agent mode"),
    responses(
        (status = 200, description = "Change an agent's operational mode", body = crate::types::JsonObject)
    )
)]
pub async fn set_agent_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<SetModeRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .with_code("invalid_agent_id")
                .into_response();
        }
    };

    let body = match state.kernel.agent_registry().set_mode(agent_id, body.mode) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "agent_id": id,
                "mode": body.mode,
            })),
        )
            .into_response(),
        Err(_) => ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
            .with_code("agent_not_found")
            .into_response(),
    };
    crate::extensions::with_agent_id(agent_id, body)
}

// ---------------------------------------------------------------------------
// Single agent detail + SSE streaming
// ---------------------------------------------------------------------------
/// GET /api/agents/:id — Get a single agent's detailed info.
#[utoipa::path(
    get,
    path = "/api/agents/{id}",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Agent details", body = crate::types::JsonObject),
        (status = 404, description = "Agent not found")
    )
)]
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
                .with_code("invalid_agent_id")
                .into_response();
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(t.t("api-error-agent-not-found"))
                .with_code("agent_not_found")
                .into_response();
        }
    };

    let dm = {
        let dm_override = state
            .kernel
            .default_model_override_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        effective_default_model(
            &state.kernel.config_ref().default_model,
            dm_override.as_ref(),
        )
    };
    let resolved_provider =
        if entry.manifest.model.provider.is_empty() || entry.manifest.model.provider == "default" {
            dm.provider.as_str()
        } else {
            entry.manifest.model.provider.as_str()
        };
    let resolved_model =
        if entry.manifest.model.model.is_empty() || entry.manifest.model.model == "default" {
            dm.model.as_str()
        } else {
            entry.manifest.model.model.as_str()
        };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "is_hand": entry.is_hand,
            "state": format!("{:?}", entry.state),
            "mode": entry.mode,
            "profile": entry.manifest.profile,
            "created_at": entry.created_at.to_rfc3339(),
            "last_active": entry.last_active.to_rfc3339(),
            "session_id": entry.session_id.0.to_string(),
            "model": {
                "provider": resolved_provider,
                "model": resolved_model,
                "max_tokens": entry.manifest.model.max_tokens,
                "temperature": entry.manifest.model.temperature,
            },
            "capabilities": {
                "tools": entry.manifest.capabilities.tools,
                "network": entry.manifest.capabilities.network,
            },
            "system_prompt": entry.manifest.model.system_prompt,
            "description": entry.manifest.description,
            "tags": entry.manifest.tags,
            "identity": {
                "emoji": entry.identity.emoji,
                "avatar_url": entry.identity.avatar_url,
                "color": entry.identity.color,
            },
            "skills": entry.manifest.skills,
            "skills_mode": skill_assignment_mode(&entry.manifest),
            "schedule": format_schedule_mode(&entry.manifest.schedule),
            "skills_disabled": entry.manifest.skills_disabled,
            "tools_disabled": entry.manifest.tools_disabled,
            "mcp_servers": entry.manifest.mcp_servers,
            "mcp_servers_mode": mcp_servers_mode(&entry.manifest.mcp_servers),
            "fallback_models": entry.manifest.fallback_models,
            "auto_evolve": entry.manifest.auto_evolve,
            "web_search_augmentation": entry.manifest.web_search_augmentation,
        })),
    )
        .into_response()
}

/// POST /api/agents/{id}/stop — Cancel an agent's current LLM run.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/stop",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Cancel an agent's current LLM run", body = crate::types::JsonObject)
    )
)]
pub async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    match state.kernel.stop_agent_run(agent_id) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Run cancelled"})),
        ),
        Ok(false) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "No active run"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        ),
    }
}

/// GET /api/agents/{id}/runtime — Snapshot of in-flight loops for the agent.
///
/// Returns one entry per `(agent, session)` pair that's currently executing.
/// Empty array when the agent is idle.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/runtime",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "List of in-flight sessions for the agent", body = crate::types::JsonArray)
    )
)]
pub async fn list_agent_runtime(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };
    let snapshots = state.kernel.list_running_sessions(agent_id);
    (StatusCode::OK, Json(serde_json::json!(snapshots)))
}

// ---------------------------------------------------------------------------
// Agent update endpoint
// ---------------------------------------------------------------------------
//
// The legacy `PUT /api/agents/{id}/update` endpoint was removed in #3748 —
// callers should send `{"manifest_toml": "..."}` to `PATCH /api/agents/{id}`
// instead, which now also handles full-manifest replacement.
#[utoipa::path(
    patch,
    path = "/api/agents/{id}",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = crate::types::JsonObject, description = "Partial agent fields to update"),
    responses(
        (status = 200, description = "Partially update an agent (name, description, model, system prompt)", body = crate::types::JsonObject)
    )
)]
pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    // Full-manifest replacement path (folded in from the now-removed
    // PUT /agents/{id}/update endpoint, #3748). When the caller supplies
    // `manifest_toml`, parse it and run the kernel's `update_manifest`
    // routine that preserves workspace/name/tags, re-grants capabilities,
    // refreshes scheduler quotas, persists to SQLite, and writes
    // agent.toml. Per-agent concurrency caps and session_mode caches
    // still require kill+respawn.
    if let Some(manifest_toml) = body.get("manifest_toml").and_then(|v| v.as_str()) {
        let manifest: AgentManifest = match toml::from_str(manifest_toml) {
            Ok(m) => m,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::json!({"error": t.t_args("api-error-agent-invalid-manifest", &[("error", &e.to_string())])}),
                    ),
                );
            }
        };
        // Localize the scrubbed internal-error message before dropping the
        // translator (`ErrorTranslator` is `!Send`, so it must not survive
        // across the `update_manifest` call site). The detailed cause still
        // reaches tracing::error! below; only the generic, localized text
        // is surfaced to the client.
        let internal_error_msg = t.t("api-error-internal");
        drop(t);
        return match state.kernel.update_manifest(agent_id, manifest) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "agent_id": id,
                    "note": "Manifest persisted; capabilities and scheduler quotas refreshed in place. Per-agent concurrency caps and session-mode changes take effect after the agent is killed and respawned.",
                })),
            ),
            // Memory/kernel error scrubbed before response (audit:
            // rusqlite-errors-leak). The full chain (column names,
            // constraint identifiers, lock state) still reaches
            // tracing::error! for ops; the response body is the
            // generic, localized "Internal server error" so the client
            // sees no schema details. Surrounding match arm shape is
            // `(StatusCode, Json<Value>)` so we hand-construct the
            // scrubbed pair here rather than detour through
            // `ApiErrorResponse::into_response()`.
            Err(e) => {
                tracing::error!(error = %e, "agent manifest update failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": internal_error_msg})),
                )
            }
        };
    }

    // Apply partial updates using dedicated registry methods
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .agent_registry()
            .update_name(agent_id, name.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .agent_registry()
            .update_description(agent_id, desc.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
        let explicit_provider = body.get("provider").and_then(|v| v.as_str());
        if let Err(e) = state
            .kernel
            .set_agent_model(agent_id, model, explicit_provider)
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(system_prompt) = body.get("system_prompt").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .agent_registry()
            .update_system_prompt(agent_id, system_prompt.to_string())
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }
    if let Some(mcp_servers) = match patch_agent_mcp_servers(&body) {
        Ok(servers) => servers,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error})),
            );
        }
    } {
        if let Err(e) = state.kernel.set_agent_mcp_servers(agent_id, mcp_servers) {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                ),
            );
        }
    }

    // Track whether `set_agent_schedule` already persisted (SQLite + disk).
    // Branches above only mutate the in-memory registry, so the generic
    // persist block at the end of this handler is required for them. The
    // schedule branch, however, routes through `set_agent_schedule` which
    // saves to SQLite and writes `agent.toml` internally — picking up any
    // earlier partial updates already applied to the registry entry. Calling
    // `save_agent` + `persist_manifest_to_disk` again here would be a
    // redundant double-write on every schedule PATCH.
    let mut schedule_persisted = false;
    if let Some(schedule_val) = body.get("schedule") {
        match serde_json::from_value::<librefang_types::agent::ScheduleMode>(schedule_val.clone()) {
            Ok(schedule) => {
                // Go through `set_agent_schedule` (not `agent_registry()
                // .update_schedule`) so the background loop is stopped /
                // restarted to match — otherwise a Reactive→Continuous
                // (or Continuous→Reactive) toggle from the dashboard
                // would return 200 but the runtime would keep running
                // the previous schedule until the daemon restarts
                // (#4984).
                if let Err(e) = state.kernel.clone().set_agent_schedule(agent_id, schedule) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(
                            serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                        ),
                    );
                }
                schedule_persisted = true;
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                    ),
                );
            }
        }
    }

    if let Some(auto_evolve) = body.get("auto_evolve").and_then(|v| v.as_bool()) {
        let _ = state
            .kernel
            .agent_registry()
            .update_auto_evolve(agent_id, auto_evolve);
    }

    // Persist updated entry to SQLite (skipped when the schedule branch
    // already handled it — see `schedule_persisted` above).
    if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
        if !schedule_persisted {
            if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
                tracing::warn!("Failed to persist agent state: {e}");
            }

            // Write updated manifest to agent.toml on disk so disk doesn't override
            // dashboard changes on next boot (#996, #1018).
            state.kernel.persist_manifest_to_disk(agent_id);
        }

        (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "agent_id": entry.id.to_string(), "name": entry.name}),
            ),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": t.t("api-error-agent-vanished")})),
        )
    }
}

/// POST /api/agents/{id}/reload — Re-read the agent's agent.toml from disk.
///
/// Picks up manual edits to fields like `skills`, `mcp_servers`, `tools`,
/// or `system_prompt` without restarting the daemon. Runtime-only fields
/// (workspace path, tags) are preserved.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/reload",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Agent manifest reloaded from agent.toml", body = crate::types::JsonObject)
    )
)]
pub async fn reload_agent_manifest(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };
    match state.kernel.reload_agent_from_disk(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "reloaded", "agent_id": id})),
        ),
        // Mirror clone/push error mapping for consistency: AgentNotFound → 404
        // (an unknown id is a missing resource, not a malformed request), and
        // on-disk config faults (missing/unreadable/invalid agent.toml) → 500
        // (the request is well-formed; the server-side state is the problem).
        Err(e) => {
            let status = kernel_err_to_status(&e);
            (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            )
        }
    }
}
