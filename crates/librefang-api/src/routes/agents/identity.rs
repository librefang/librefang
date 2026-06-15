use super::*;

// ---------------------------------------------------------------------------
// Agent Identity endpoint
// ---------------------------------------------------------------------------
/// Request body for updating agent visual identity.
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateIdentityRequest {
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub greeting_style: Option<String>,
}

/// PATCH /api/agents/{id}/identity — Update an agent's visual identity.
#[utoipa::path(
    patch,
    path = "/api/agents/{id}/identity",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = UpdateIdentityRequest, description = "Identity fields to update"),
    responses(
        (status = 200, description = "Update an agent's visual identity", body = crate::types::JsonObject)
    )
)]
#[allow(private_interfaces)]
pub async fn update_agent_identity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<UpdateIdentityRequest>,
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

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-color-invalid")})),
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-avatar-invalid")})),
            );
        }
    }

    let identity = AgentIdentity {
        emoji: req.emoji,
        avatar_url: req.avatar_url,
        color: req.color,
        archetype: req.archetype,
        vibe: req.vibe,
        greeting_style: req.greeting_style,
    };

    match state
        .kernel
        .agent_registry()
        .update_identity(agent_id, identity)
    {
        Ok(()) => {
            // Persist identity to SQLite
            if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
                if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
                    tracing::warn!("Failed to persist agent state: {e}");
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "agent_id": id})),
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Canonical agent UUID registry endpoints (refs #4614)
// ---------------------------------------------------------------------------
/// One row in the response of `GET /api/agents/identities`.
///
/// `created_at` is RFC 3339 UTC (string form rather than
/// `chrono::DateTime<Utc>` so the type implements `utoipa::ToSchema`
/// without pulling in chrono's optional `schemars` feature).
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct AgentIdentityRow {
    pub name: String,
    pub canonical_uuid: String,
    pub created_at: String,
}

/// GET /api/agents/identities — List the canonical UUID registry (refs #4614).
///
/// Returns all `name → canonical_uuid` mappings persisted at
/// `<home_dir>/agent_identities.toml`. Order is stable (sorted by name) so
/// callers can rely on the result for diagnostics / golden tests.
#[utoipa::path(
    get,
    path = "/api/agents/identities",
    tag = "agents",
    responses(
        (status = 200, description = "Canonical UUID registry contents", body = Vec<AgentIdentityRow>)
    )
)]
pub async fn list_agent_identities(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let entries = state.kernel.agent_identities().list();
    let rows: Vec<AgentIdentityRow> = entries
        .into_iter()
        .map(|(name, identity)| AgentIdentityRow {
            name,
            canonical_uuid: identity.canonical_uuid.to_string(),
            created_at: identity.created_at.to_rfc3339(),
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!(rows)))
}

/// Query parameters for `POST /api/agents/identities/{name}/reset`.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ResetIdentityQuery {
    #[serde(default)]
    pub confirm: bool,
}

const RESET_IDENTITY_WARNING: &str = "Resetting this agent's canonical UUID will orphan all sessions, memories, and audit history tied to the prior UUID. The next spawn under this name will start with a fresh UUID. This action cannot be undone. Re-issue with confirm=true to proceed.";

/// POST /api/agents/identities/{name}/reset — Drop the canonical UUID
/// binding for `name` (refs #4614).
///
/// Requires `confirm=true` (query string or JSON body) — without it the
/// request is rejected with `409 Conflict` and the data-loss warning. The
/// next spawn under the same name re-derives a fresh UUID via
/// `AgentId::from_name` and registers it as the new canonical binding.
/// The agent is **not** killed — operators can call `DELETE /api/agents/{id}`
/// (or `kill_agent`) separately if a runtime restart is also desired.
///
/// Returns `404` if no entry exists for `name`.
#[utoipa::path(
    post,
    path = "/api/agents/identities/{name}/reset",
    tag = "agents",
    params(
        ("name" = String, Path, description = "Agent name"),
        ("confirm" = Option<bool>, Query, description = "Required: confirms canonical UUID reset.")
    ),
    responses(
        (status = 200, description = "Canonical UUID purged"),
        (status = 404, description = "No canonical UUID recorded for this name"),
        (status = 409, description = "Confirmation required")
    )
)]
pub async fn reset_agent_identity(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(q): Query<ResetIdentityQuery>,
) -> impl IntoResponse {
    if !q.confirm {
        return ApiErrorResponse::conflict(RESET_IDENTITY_WARNING)
            .with_code("reset_identity_unconfirmed")
            .into_response();
    }

    match state.kernel.agent_identities().purge(&name) {
        Some(dropped) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "reset",
                "name": name,
                "previous_canonical_uuid": dropped.to_string(),
            })),
        )
            .into_response(),
        None => ApiErrorResponse::not_found(format!(
            "no canonical UUID recorded for agent name '{name}'"
        ))
        .with_code("identity_not_found")
        .into_response(),
    }
}
