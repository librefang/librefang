use super::*;

#[utoipa::path(
    put,
    path = "/api/agents/{id}/model",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = crate::types::JsonObject, description = "Model name and optional provider"),
    responses(
        (status = 200, description = "Change an agent's LLM model", body = crate::types::JsonObject)
    )
)]
pub async fn set_model(
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
            )
        }
    };
    let model = match body["model"].as_str() {
        Some(m) if !m.is_empty() => m,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-missing-model")})),
            )
        }
    };
    let explicit_provider = body["provider"].as_str();
    // Check agent exists — kernel returns a generic error for missing
    // agents that the match arm below would wrap as 500. Validate up
    // front so the caller gets a 404 for the common case.
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
    match state
        .kernel
        .set_agent_model(agent_id, model, explicit_provider)
    {
        Ok(()) => {
            // Return the resolved model+provider so frontend stays in sync.
            // The model name may have been normalized (provider prefix stripped),
            // so we read it back from the registry instead of echoing the raw input.
            let (resolved_model, resolved_provider) = state
                .kernel
                .agent_registry()
                .get(agent_id)
                .map(|e| {
                    (
                        e.manifest.model.model.clone(),
                        e.manifest.model.provider.clone(),
                    )
                })
                .unwrap_or_else(|| (model.to_string(), String::new()));
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "ok", "model": resolved_model, "provider": resolved_provider}),
                ),
            )
        }
        Err(e) => {
            let status = kernel_err_to_status(&e);
            (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            )
        }
    }
}

/// GET /api/agents/{id}/tools — Get an agent's tool allowlist/blocklist.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/tools",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get an agent's tool allowlist and blocklist", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_tools(
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
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
        }
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "capabilities_tools": entry.manifest.capabilities.tools,
            "tool_allowlist": entry.manifest.tool_allowlist,
            "tool_blocklist": entry.manifest.tool_blocklist,
            "disabled": entry.manifest.tools_disabled,
        })),
    )
}

/// Request body for updating an agent's tool configuration.
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentToolsRequest {
    /// Declared tools (capabilities.tools). `None` = no change, `Some([])` = unrestricted.
    pub capabilities_tools: Option<Vec<String>>,
    /// Tool allowlist — additional filter. `None` = no change, `Some([])` = clear.
    #[serde(default)]
    pub tool_allowlist: Option<Vec<String>>,
    /// Tool blocklist — exclusion filter. `None` = no change, `Some([])` = clear.
    #[serde(default)]
    pub tool_blocklist: Option<Vec<String>>,
}

/// PUT /api/agents/{id}/tools — Update an agent's tool allowlist/blocklist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/tools",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = SetAgentToolsRequest, description = "Tool configuration fields"),
    responses(
        (status = 200, description = "Update an agent's tool allowlist and blocklist", body = crate::types::JsonObject)
    )
)]
pub async fn set_agent_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<SetAgentToolsRequest>,
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

    if body.capabilities_tools.is_none()
        && body.tool_allowlist.is_none()
        && body.tool_blocklist.is_none()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-agent-missing-tools")})),
        );
    }

    // Check agent exists — kernel returns a generic error for missing
    // agents that the match arm below would wrap as 500. Validate up
    // front so the caller gets a 404 for the common case.
    if state.kernel.agent_registry().get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    match state.kernel.set_agent_tool_filters(
        agent_id,
        body.capabilities_tools,
        body.tool_allowlist,
        body.tool_blocklist,
    ) {
        // Read the agent back so the dashboard can `setQueryData` directly
        // instead of refetching. Returns the same shape as `GET /api/agents/{id}/tools`.
        // If the registry entry vanished between the write and read (extremely
        // unlikely — would mean the agent was deleted mid-PUT) fall back to a
        // 200 ack so existing clients don't crash on the missing body.
        Ok(()) => match state.kernel.agent_registry().get(agent_id) {
            Some(entry) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "capabilities_tools": entry.manifest.capabilities.tools,
                    "tool_allowlist": entry.manifest.tool_allowlist,
                    "tool_blocklist": entry.manifest.tool_blocklist,
                    "disabled": entry.manifest.tools_disabled,
                })),
            ),
            None => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        ),
    }
}

// ── Per-Agent Skill & MCP Endpoints ────────────────────────────────────
/// GET /api/agents/{id}/skills — Get an agent's skill assignment info.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/skills",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get an agent's skill assignment info", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_skills(
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
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
        }
    };
    let available = state
        .kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .skill_names();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": skill_assignment_mode(&entry.manifest),
            "disabled": entry.manifest.skills_disabled,
        })),
    )
}

/// PUT /api/agents/{id}/skills — Update an agent's skill allowlist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/skills",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = crate::types::JsonArray, description = "Array of skill names"),
    responses(
        (status = 200, description = "Update an agent's skill allowlist", body = crate::types::JsonObject)
    )
)]
pub async fn set_agent_skills(
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
            )
        }
    };
    let skills: Vec<String> = body["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state.kernel.set_agent_skills(agent_id, skills.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "skills": skills})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// GET /api/agents/{id}/mcp_servers — Get an agent's MCP server assignment info.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/mcp_servers",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get an agent's MCP server assignment info", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_mcp_servers(
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
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
        }
    };
    // Collect known MCP server names from connected tools
    let mut available: Vec<String> = Vec::new();
    if let Ok(mcp_tools) = state.kernel.mcp_tools_ref().lock() {
        let configured_servers: Vec<String> = state
            .kernel
            .effective_mcp_servers_ref()
            .read()
            .map(|servers| servers.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        for tool in mcp_tools.iter() {
            if let Some(server) = librefang_kernel::mcp::resolve_mcp_server_from_known(
                &tool.name,
                configured_servers.iter().map(String::as_str),
            ) {
                if seen.insert(server.to_string()) {
                    available.push(server.to_string());
                }
            }
        }
    }
    let mode = mcp_servers_mode(&entry.manifest.mcp_servers);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.mcp_servers,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/mcp_servers — Update an agent's MCP server allowlist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/mcp_servers",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = crate::types::JsonArray, description = "Array of MCP server names"),
    responses(
        (status = 200, description = "Update an agent's MCP server allowlist", body = crate::types::JsonObject)
    )
)]
pub async fn set_agent_mcp_servers(
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
            )
        }
    };
    let servers: Vec<String> = body["mcp_servers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state
        .kernel
        .set_agent_mcp_servers(agent_id, servers.clone())
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "mcp_servers": servers})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

/// GET /api/agents/{id}/channels — Get an agent's channel allowlist info.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/channels",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Get an agent's channel allowlist info", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_channels(
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
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            )
        }
    };
    let available: Vec<String> = state
        .kernel
        .config_ref()
        .sidecar_channels
        .iter()
        .map(|sc| sc.channel_type.clone().unwrap_or_else(|| sc.name.clone()))
        .collect();
    let mode = if entry.manifest.channels.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.channels,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/channels — Update an agent's channel allowlist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/channels",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = crate::types::JsonArray, description = "Array of channel_type strings"),
    responses(
        (status = 200, description = "Update an agent's channel allowlist", body = crate::types::JsonObject)
    )
)]
pub async fn set_agent_channels(
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
            )
        }
    };
    let channels: Vec<String> = body["channels"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state.kernel.set_agent_channels(agent_id, channels.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "channels": channels})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent Config Hot-Update
// ---------------------------------------------------------------------------
/// Request body for patching agent config (name, description, prompt, identity, model).
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct PatchAgentConfigRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    pub archetype: Option<String>,
    pub vibe: Option<String>,
    pub greeting_style: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    /// Maximum tokens for LLM response. Controls conversation window size.
    pub max_tokens: Option<u32>,
    /// Sampling temperature (0.0–2.0). Lower values are more deterministic.
    pub temperature: Option<f32>,
    #[schema(value_type = Option<Vec<serde_json::Value>>)]
    pub fallback_models: Option<Vec<librefang_types::agent::FallbackModel>>,
    /// Web search augmentation mode: "off", "auto", or "always".
    #[schema(value_type = Option<String>)]
    pub web_search_augmentation: Option<librefang_types::agent::WebSearchAugmentationMode>,
}

/// PATCH /api/agents/{id}/config — Hot-update agent name, description, system prompt, and identity.
#[utoipa::path(
    patch,
    path = "/api/agents/{id}/config",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = PatchAgentConfigRequest, description = "Agent config fields to update"),
    responses(
        (status = 200, description = "Hot-update agent name, description, system prompt, identity, and model", body = crate::types::JsonObject)
    )
)]
#[allow(private_interfaces)]
pub async fn patch_agent_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<PatchAgentConfigRequest>,
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

    // Input length limits
    const MAX_NAME_LEN: usize = 256;
    const MAX_DESC_LEN: usize = 4096;
    const MAX_PROMPT_LEN: usize = 65_536;

    if let Some(ref name) = req.name {
        if name.len() > MAX_NAME_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-name-too-long", &[("max", &MAX_NAME_LEN.to_string())])}),
                ),
            );
        }
    }
    if let Some(ref desc) = req.description {
        if desc.len() > MAX_DESC_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-desc-too-long", &[("max", &MAX_DESC_LEN.to_string())])}),
                ),
            );
        }
    }
    if let Some(ref prompt) = req.system_prompt {
        if prompt.len() > MAX_PROMPT_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-agent-prompt-too-long", &[("max", &MAX_PROMPT_LEN.to_string())])}),
                ),
            );
        }
    }

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

    // Update name
    if let Some(ref new_name) = req.name {
        if !new_name.is_empty() {
            if let Err(e) = state
                .kernel
                .agent_registry()
                .update_name(agent_id, new_name.clone())
            {
                return (
                    StatusCode::CONFLICT,
                    Json(
                        serde_json::json!({"error": t.t_args("api-error-generic", &[("error", &e.to_string())])}),
                    ),
                );
            }
        }
    }

    // Update description
    if let Some(ref new_desc) = req.description {
        if state
            .kernel
            .agent_registry()
            .update_description(agent_id, new_desc.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update system prompt (hot-swap — takes effect on next message)
    if let Some(ref new_prompt) = req.system_prompt {
        if state
            .kernel
            .agent_registry()
            .update_system_prompt(agent_id, new_prompt.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update identity fields (merge — only overwrite provided fields)
    let has_identity_field = req.emoji.is_some()
        || req.avatar_url.is_some()
        || req.color.is_some()
        || req.archetype.is_some()
        || req.vibe.is_some()
        || req.greeting_style.is_some();

    if has_identity_field {
        // Read current identity, merge with provided fields
        let current = state
            .kernel
            .agent_registry()
            .get(agent_id)
            .map(|e| e.identity)
            .unwrap_or_default();
        let merged = AgentIdentity {
            emoji: req.emoji.or(current.emoji),
            avatar_url: req.avatar_url.or(current.avatar_url),
            color: req.color.or(current.color),
            archetype: req.archetype.or(current.archetype),
            vibe: req.vibe.or(current.vibe),
            greeting_style: req.greeting_style.or(current.greeting_style),
        };
        if state
            .kernel
            .agent_registry()
            .update_identity(agent_id, merged)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update model/provider — always go through set_agent_model so that
    // provider-change semantics (prefix stripping, canonical-session cleanup,
    // and clearing of stale per-agent api_key_env / base_url overrides) are
    // applied uniformly. Bypassing it via update_model_and_provider was the
    // root cause of #2380: switching to a non-default provider via the
    // dashboard left stale CLOUDVERSE_API_KEY / cloudverse base_url on the
    // manifest, so the new provider's request was sent to the old URL with
    // the old credentials and rejected with "Missing Authentication header".
    if let Some(ref new_model) = req.model {
        if !new_model.is_empty() {
            let explicit_provider = req.provider.as_deref().filter(|p| !p.is_empty());
            if let Err(e) = state
                .kernel
                .set_agent_model(agent_id, new_model, explicit_provider)
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": scrub_500(&e, &t)})),
                );
            }
        }
    }

    // Apply per-agent api_key_env / base_url overrides. The OpenAPI schema for
    // this endpoint advertises both fields, but before this they were read only
    // by the hand-runtime-config endpoint and silently dropped here — switching
    // a non-hand agent to a custom provider plus its credential env var / base
    // URL in one `PATCH /config` call returned `200` while the two fields were
    // discarded (the #2380 class of failure). `set_agent_model` above clears
    // any stale api_key_env / base_url on a provider change, so the override is
    // re-applied AFTER it has run.
    //
    // Tri-state per field: `Some(non-empty)` sets it, `Some(empty/whitespace)`
    // clears it, `None` leaves it unchanged — merged against the current value
    // so sending only one field does not wipe the other.
    if req.api_key_env.is_some() || req.base_url.is_some() {
        let entry = match state.kernel.agent_registry().get(agent_id) {
            Some(e) => e,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
                );
            }
        };
        let resolve = |incoming: Option<&String>, current: Option<String>| -> Option<String> {
            match incoming {
                Some(s) if s.trim().is_empty() => None,
                Some(s) => Some(s.trim().to_string()),
                None => current,
            }
        };
        let new_api_key_env = resolve(
            req.api_key_env.as_ref(),
            entry.manifest.model.api_key_env.clone(),
        );
        let new_base_url = resolve(req.base_url.as_ref(), entry.manifest.model.base_url.clone());
        if state
            .kernel
            .agent_registry()
            .update_model_provider_config(
                agent_id,
                entry.manifest.model.model.clone(),
                entry.manifest.model.provider.clone(),
                new_api_key_env,
                new_base_url,
            )
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Validate and update temperature (sampling randomness)
    if let Some(temperature) = req.temperature {
        if !(0.0..=2.0).contains(&temperature) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "temperature must be between 0.0 and 2.0"})),
            );
        }
        if state
            .kernel
            .agent_registry()
            .update_temperature(agent_id, temperature)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update max_tokens (response length / conversation window limit)
    if let Some(max_tokens) = req.max_tokens {
        if max_tokens == 0 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "max_tokens must be greater than 0"})),
            );
        }
        if state
            .kernel
            .agent_registry()
            .update_max_tokens(agent_id, max_tokens)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update fallback model chain
    if let Some(fallbacks) = req.fallback_models {
        if state
            .kernel
            .agent_registry()
            .update_fallback_models(agent_id, fallbacks)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Update web search augmentation mode
    if let Some(mode) = req.web_search_augmentation {
        if state
            .kernel
            .agent_registry()
            .update_web_search_augmentation(agent_id, mode)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    }

    // Persist updated manifest to database so changes survive restart
    if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
        if let Err(e) = state.kernel.memory_substrate().save_agent(&entry) {
            tracing::warn!("Failed to persist agent config update: {e}");
        }
    }

    // Write updated manifest to agent.toml on disk so disk doesn't override
    // dashboard changes on next boot (#996, #1018).
    state.kernel.persist_manifest_to_disk(agent_id);

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "agent_id": id})),
    )
}

/// Map a DTO `Option<String>` into the `Option<Option<String>>` semantics
/// required by [`librefang_hands::HandAgentRuntimeOverride`] for nullable
/// secret-like fields (`api_key_env`, `base_url`).
///
/// - `None`            (field absent in JSON)        → `None`            (leave unchanged)
/// - `Some("")`        (empty string sent in JSON)   → `Some(None)`      (clear the override)
/// - `Some(non_empty)` (string value sent)           → `Some(Some(_))`   (set the override)
///
/// Whitespace is trimmed before the empty-string check so values like `"   "`
/// are treated as a clear, matching the `/config` endpoint's existing
/// length-bounded semantics for these fields.
fn hand_override_nullable_string(raw: Option<String>) -> Option<Option<String>> {
    raw.map(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// PATCH /api/agents/{id}/hand-runtime-config — Runtime-only config override for hand agents.
#[utoipa::path(
    patch,
    path = "/api/agents/{id}/hand-runtime-config",
    tag = "agents",
    params(("id" = String, Path, description = "Hand agent ID")),
    request_body(
        content = PatchAgentConfigRequest,
        description = "Runtime override fields. Whitespace is trimmed on all string fields. For `model` and `provider` an empty (or whitespace-only) string is ignored ('leave unchanged'); for the nullable secrets `api_key_env` and `base_url` an empty (or whitespace-only) string clears the override."
    ),
    responses(
        (status = 200, description = "Runtime override applied to the live manifest and persisted to hand_state.json", body = crate::types::JsonObject),
        (status = 400, description = "Invalid agent id or target agent is not managed by a hand", body = crate::types::JsonObject),
        (status = 404, description = "Agent not found", body = crate::types::JsonObject),
        (status = 409, description = "Hand role not found for the agent (hand registry inconsistency)", body = crate::types::JsonObject),
        (status = 500, description = "Internal kernel error", body = crate::types::JsonObject),
    )
)]
pub async fn patch_hand_agent_runtime_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid agent id"})),
            );
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "agent not found"})),
            );
        }
    };
    if !entry.is_hand {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "agent is not managed by a hand"})),
        );
    }

    // Field semantics:
    // - `model` / `provider`: plain `Option<String>`. Empty string is
    //   ignored (dashboard sends empty strings for "leave unchanged" on
    //   free-text inputs); the kernel merges any `Some(value)` onto the
    //   existing override.
    // - `api_key_env` / `base_url`: tri-state via `Option<Option<String>>`.
    //   See `hand_override_nullable_string` for the empty-string = clear
    //   convention.
    // - `max_tokens` / `temperature` / `web_search_augmentation`: pass
    //   through as-is; `None` means "do not change".
    let override_config = librefang_hands::HandAgentRuntimeOverride {
        model: req
            .model
            .map(|s| s.trim().to_string())
            .filter(|v| !v.is_empty()),
        provider: req
            .provider
            .map(|s| s.trim().to_string())
            .filter(|v| !v.is_empty()),
        api_key_env: hand_override_nullable_string(req.api_key_env),
        base_url: hand_override_nullable_string(req.base_url),
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        web_search_augmentation: req.web_search_augmentation,
    };

    match state
        .kernel
        .update_hand_agent_runtime_override(agent_id, override_config)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "agent_id": id})),
        ),
        Err(e) => {
            let (status, msg) = map_hand_runtime_override_err(&e);
            (status, Json(serde_json::json!({"error": msg})))
        }
    }
}

/// DELETE /api/agents/{id}/hand-runtime-config — Drop all runtime overrides
/// for the hand agent's role, restoring the live manifest to the HAND.toml
/// defaults and persisting the cleared state to `hand_state.json`.
///
/// Returns 204 No Content on success (idempotent — a second call against an
/// already-clean role is also 204).
#[utoipa::path(
    delete,
    path = "/api/agents/{id}/hand-runtime-config",
    tag = "agents",
    params(("id" = String, Path, description = "Hand agent ID")),
    responses(
        (status = 204, description = "Runtime overrides cleared; manifest restored to HAND.toml defaults"),
        (status = 400, description = "Invalid agent id or target agent is not managed by a hand", body = crate::types::JsonObject),
        (status = 404, description = "Agent not found", body = crate::types::JsonObject),
        (status = 409, description = "Hand role not found for the agent (hand registry inconsistency)", body = crate::types::JsonObject),
        (status = 500, description = "Internal kernel error", body = crate::types::JsonObject),
    )
)]
pub async fn delete_hand_agent_runtime_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid agent id"})),
            )
                .into_response();
        }
    };

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "agent not found"})),
            )
                .into_response();
        }
    };
    if !entry.is_hand {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "agent is not managed by a hand"})),
        )
            .into_response();
    }

    match state.kernel.clear_hand_agent_runtime_override(agent_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let (status, msg) = map_hand_runtime_override_err(&e);
            (status, Json(serde_json::json!({"error": msg}))).into_response()
        }
    }
}
