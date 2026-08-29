use super::*;
use librefang_skills::registry::SkillRegistry;
use std::sync::{RwLock, RwLockReadGuard};

fn read_agent_skills_registry(
    registry: &RwLock<SkillRegistry>,
) -> RwLockReadGuard<'_, SkillRegistry> {
    registry.read().unwrap_or_else(|poisoned| {
        tracing::warn!(
            "Agent skill-assignment registry lock poisoned; recovering installed skills"
        );
        registry.clear_poison();
        poisoned.into_inner()
    })
}

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

    // #6695: refuse to change the manifest of an agent the deployment provisioned.
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        return refusal;
    }
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
        (status = 200, description = "Get an agent's tool allowlist and blocklist. `capabilities_tools` is the grant surface; `tool_allowlist` and `tool_blocklist` are applied afterwards and only narrow what it already admits, so a tool listed in `tool_allowlist` but absent from `capabilities_tools` is not granted (MCP tools excepted — they are not filtered by `capabilities_tools`). Refs #6609.", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_tools(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
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

/// The `tool_allowlist` entries that provably cannot admit any tool (#6609).
///
/// `tool_allowlist` is applied by `LibreFangKernel::available_tools`
/// (`librefang-kernel/src/kernel/tools_and_skills.rs`, Step 4) as `all_tools.retain(...)`, so it can only remove.
/// A builtin or skill tool that `capabilities.tools` did not admit in Step 1 is already gone by then, and no allowlist entry can bring it back.
/// That narrowing-only behaviour is deliberate; what this function fixes is that the write API used to accept, persist and echo an entry that could never do anything, with no signal at all to the caller.
///
/// Only entries that are *provably* inert are reported.
/// Anything that might match a tool now or later is left unreported, because a false warning on a working configuration is worse than the silence it replaces.
/// An entry is reported only when all of the following hold:
///
/// - `declared_tools` is restricted: non-empty and free of a `*` entry.
///   This mirrors the kernel's `tools_unrestricted` check; when it is false Step 1's filter is a no-op and every builtin reaches the candidate set, so any entry may legitimately narrow something.
/// - The entry contains no `*`.
///   A glob may match a tool introduced by a later skill install or MCP connect, so it is never *provably* inert; restricting the check to literals also removes any need to decide whether two glob patterns can overlap.
/// - The entry is not MCP-namespaced.
///   MCP tools join the candidate set in Step 3 *without* being filtered by `capabilities.tools`, and their names are generated at runtime from whichever servers happen to be connected.
/// - The entry is not a self-evolution tool.
///   Step 1's post-filter injects those regardless of what `capabilities.tools` declares.
/// - No declared pattern glob-matches the entry.
///   This is a glob evaluation, not a string comparison: `capabilities.tools = ["file_*"]` does admit an allowlist entry of `file_read`, and a string compare would wrongly flag it.
///
/// Together those conditions make false positives impossible: for a literal entry to survive Step 4 it must equal some candidate tool name, and every source of candidate names — builtins and skill tools admitted by `capabilities.tools`, the always-injected evolution tools, and MCP tools — is excluded above.
/// The cost is false negatives (a genuinely inert glob entry goes unreported), which is the safe direction to err in.
fn inert_tool_allowlist_entries(
    declared_tools: &[String],
    tool_allowlist: &[String],
) -> Vec<String> {
    let declared_restricted =
        !declared_tools.is_empty() && !declared_tools.iter().any(|d| d == "*");
    if !declared_restricted {
        return Vec::new();
    }
    tool_allowlist
        .iter()
        .filter(|raw| {
            let entry = raw.as_str();
            !entry.contains('*')
                && !librefang_kernel::mcp::is_mcp_tool(entry)
                && !librefang_kernel::LibreFangKernel::is_evolve_tool(entry)
                && !declared_tools
                    .iter()
                    .any(|d| librefang_types::capability::glob_matches(d, entry))
        })
        .cloned()
        .collect()
}

/// Request body for updating an agent's tool configuration.
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentToolsRequest {
    /// Declared tools (capabilities.tools) — the grant surface.
    /// A builtin or skill tool has to be admitted here to reach the agent at all.
    /// `None` = no change, `Some([])` = unrestricted.
    /// Glob patterns allowed.
    pub capabilities_tools: Option<Vec<String>>,
    /// Tool allowlist — a **narrowing** filter over what `capabilities_tools` already admits, never a grant.
    /// It is applied after `capabilities_tools` as a retain, so an entry naming a tool the declared set excludes has no effect; grant such a tool by adding it to `capabilities_tools` instead.
    /// `None` = no change, `Some([])` = clear.
    /// Glob patterns allowed (`file_*`, `mcp_*`).
    #[serde(default)]
    pub tool_allowlist: Option<Vec<String>>,
    /// Tool blocklist — exclusion filter, applied after the allowlist.
    /// `None` = no change, `Some([])` = clear.
    /// Glob patterns allowed.
    #[serde(default)]
    pub tool_blocklist: Option<Vec<String>>,
    /// `agent.toml: tools_disabled` — the master switch that removes every tool from the agent before any of the three filters above are consulted.
    /// `None` = no change, `Some(true)` = disable all tools, `Some(false)` = re-enable them.
    ///
    /// Named `disabled` to match the key `GET /api/agents/{id}/tools` already returns.
    /// Until #7742 the GET returned it and no request could write it, and every successful write forced it back to `false` — so an operator editing a blocklist re-enabled every tool without asking.
    #[serde(default)]
    pub disabled: Option<bool>,
}

/// PUT /api/agents/{id}/tools — Update an agent's tool allowlist/blocklist.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/tools",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(
        content = SetAgentToolsRequest,
        description = "Tool configuration fields. `capabilities_tools` is the grant surface; `tool_allowlist` and `tool_blocklist` only ever narrow what it already admits, because the kernel applies them afterwards as a retain. An allowlist entry naming a builtin or skill tool that `capabilities_tools` excludes therefore grants nothing — add it to `capabilities_tools` instead. MCP tools are the exception: they are not filtered by `capabilities_tools`, so an `mcp_*` allowlist entry does select among them (#6609). `disabled` is the `tools_disabled` master switch, evaluated before all three filters. Every field is a tri-state: omit it to leave the stored value alone, send it to write exactly what it says (#7742)."
    ),
    responses(
        (status = 200, description = "Updated tool configuration, echoing the stored `capabilities_tools`, `tool_allowlist`, `tool_blocklist` and `disabled` values. Carries an additional `warnings` array of strings naming each stored `tool_allowlist` entry that provably cannot admit any tool; the key is absent when there is nothing to report. The check runs whenever the request submits `tool_allowlist` or `capabilities_tools` — narrowing the grant surface is itself a way to render a stored entry inert — and is skipped for a request that submits only `tool_blocklist` or only `disabled`, and for an agent left with `tools_disabled = true`, where no allowlist entry can admit anything anyway.", body = crate::types::JsonObject)
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

    // #6695: refuse to change the manifest of an agent the deployment provisioned.
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        return refusal;
    }

    if body.capabilities_tools.is_none()
        && body.tool_allowlist.is_none()
        && body.tool_blocklist.is_none()
        && body.disabled.is_none()
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

    // Whether the inert-entry diagnostic below should run at all.
    // Captured before the fields are moved into the kernel call.
    //
    // Submitting `tool_allowlist` obviously makes the caller responsible for what it says.
    // Submitting `capabilities_tools` does too, because that is the grant surface: narrowing it can render a stored allowlist entry inert without the request mentioning the allowlist at all, and staying silent there would reproduce the exact #6609 experience — the operator's own request silences a tool and the response is a clean 200.
    // A request that touches neither (blocklist only) says nothing about either side, so it stays quiet about whatever was already stored.
    let evaluate_inert_entries = body.tool_allowlist.is_some() || body.capabilities_tools.is_some();

    match state.kernel.set_agent_tool_filters(
        agent_id,
        body.capabilities_tools,
        body.tool_allowlist,
        body.tool_blocklist,
        body.disabled,
    ) {
        // Read the agent back so the dashboard can `setQueryData` directly
        // instead of refetching. Returns the same shape as `GET /api/agents/{id}/tools`.
        // If the registry entry vanished between the write and read (extremely
        // unlikely — would mean the agent was deleted mid-PUT) fall back to a
        // 200 ack so existing clients don't crash on the missing body.
        Ok(()) => match state.kernel.agent_registry().get(agent_id) {
            Some(entry) => {
                let mut payload = serde_json::json!({
                    "capabilities_tools": entry.manifest.capabilities.tools,
                    "tool_allowlist": entry.manifest.tool_allowlist,
                    "tool_blocklist": entry.manifest.tool_blocklist,
                    "disabled": entry.manifest.tools_disabled,
                });
                // Name any allowlist entry that provably cannot admit a tool (#6609).
                // Evaluated against the entry read back *after* the write, so both the new declared set and the stored allowlist are the post-write values: a request that sets `capabilities_tools` and `tool_allowlist` together is judged against what it just wrote rather than what it replaced, and a request that only narrows `capabilities_tools` is judged against the allowlist it left in place.
                //
                // Gated on `tools_disabled` as well, since #7742: `update_tool_config` no longer force-clears that flag, so an agent can reach this arm with every tool switched off.
                // Naming an inert allowlist entry there would be advice about the second filter of a pipeline whose first stage already admits nothing — noise, and the same "your own request silenced a tool and the response was a clean 200" failure in reverse.
                if evaluate_inert_entries && !entry.manifest.tools_disabled {
                    let inert = inert_tool_allowlist_entries(
                        &entry.manifest.capabilities.tools,
                        &entry.manifest.tool_allowlist,
                    );
                    if !inert.is_empty() {
                        let warnings: Vec<String> = inert
                            .iter()
                            .map(|name| {
                                format!(
                                    "tool_allowlist entry '{name}' cannot take effect: capabilities_tools does not admit it, and tool_allowlist only narrows that set — it never grants. Add '{name}' to capabilities_tools to grant it."
                                )
                            })
                            .collect();
                        payload["warnings"] = serde_json::json!(warnings);
                    }
                }
                (StatusCode::OK, Json(payload))
            }
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
    let available = read_agent_skills_registry(state.kernel.skill_registry_ref()).skill_names();
    let mode = skill_assignment_mode(&entry.manifest);
    // `ErrorTranslator` holds a `!Send` FluentBundle, so it must not be alive across the await below (#7713 added the first await to this handler).
    drop(t);
    let pending = state
        .kernel
        .pending_skill_and_mcp_declarations(agent_id)
        .await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": mode,
            "disabled": entry.manifest.skills_disabled,
            // Declared but not installed — retained in the manifest, activates on the next skills reload (#7713).
            "pending": pending.skills,
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

    // #6695: refuse to change the manifest of an agent the deployment provisioned.
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        return refusal;
    }
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
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
    // `ErrorTranslator` holds a `!Send` FluentBundle, so it must not be alive across the await below (#7713 added the first await to this handler).
    drop(t);
    let pending = state
        .kernel
        .pending_skill_and_mcp_declarations(agent_id)
        .await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.mcp_servers,
            "available": available,
            "mode": mode,
            // Declared but with no live connection — configured-and-unreachable counts as pending (#7713).
            "pending": pending.mcp_servers,
        })),
    )
}

/// PUT /api/agents/{id}/mcp_servers — Update an agent's MCP server allowlist.
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentMcpServersRequest {
    /// MCP server names assigned to the agent.
    /// An empty list disables MCP servers for the agent; `["*"]` enables all connected servers.
    pub mcp_servers: Vec<String>,
}

#[utoipa::path(
    put,
    path = "/api/agents/{id}/mcp_servers",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = SetAgentMcpServersRequest, description = "Object containing the MCP server allowlist"),
    responses(
        (status = 200, description = "Update an agent's MCP server allowlist", body = crate::types::JsonObject),
        (status = 400, description = "Malformed request body", body = crate::types::JsonObject)
    )
)]
pub async fn set_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    body: Result<Json<SetAgentMcpServersRequest>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let body = match body {
        Ok(Json(body)) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error.body_text()})),
            )
        }
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };

    // #6695: refuse to change the manifest of an agent the deployment provisioned.
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        return refusal;
    }
    let servers = body.mcp_servers;
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }
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

/// Body of `PUT /api/agents/{id}/channels`.
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentChannelsRequest {
    /// `channel_type` strings the agent may receive messages from.
    /// An empty list clears the allowlist, which means "all channels" (`mode = "all"`).
    pub channels: Vec<String>,
}

/// PUT /api/agents/{id}/channels — Update an agent's channel allowlist.
///
/// Body shape mirrors the sibling `mcp_servers` route rather than the bare array the OpenAPI
/// annotation used to advertise (#7742).
/// The old handler read `body["channels"]` out of an untyped `Value` and fell back to `Vec::new()`
/// for anything it did not recognise, so the documented bare-array body — and any typo in the key —
/// silently cleared the allowlist and reopened the agent to every channel.
/// Widening an allowlist is not a defensible interpretation of a malformed request, so a body that
/// does not deserialize is now a 400.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/channels",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = SetAgentChannelsRequest, description = "Object containing the channel allowlist"),
    responses(
        (status = 200, description = "Update an agent's channel allowlist", body = crate::types::JsonObject),
        (status = 400, description = "Malformed request body", body = crate::types::JsonObject)
    )
)]
pub async fn set_agent_channels(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    body: Result<Json<SetAgentChannelsRequest>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let body = match body {
        Ok(Json(body)) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error.body_text()})),
            )
        }
    };
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            )
        }
    };

    // #6695: refuse to change the manifest of an agent the deployment provisioned.
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        return refusal;
    }
    let channels = body.channels;
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

    // #6695: refuse to change the manifest of an agent the deployment provisioned.
    if let Some(refusal) = super::guard_provisioned_agent(&state, agent_id) {
        return refusal;
    }

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
        // Read current identity, merge with provided fields.
        // The merge itself lives in `merge_agent_identity` so this handler and `PATCH /api/agents/{id}/identity` cannot drift apart again (#6608).
        let current = state
            .kernel
            .agent_registry()
            .get(agent_id)
            .map(|e| e.identity)
            .unwrap_or_default();
        let merged = merge_agent_identity(
            current,
            AgentIdentity {
                emoji: req.emoji,
                avatar_url: req.avatar_url,
                color: req.color,
                archetype: req.archetype,
                vibe: req.vibe,
                greeting_style: req.greeting_style,
            },
        );
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

    // Update model/provider through set_agent_model so provider-change semantics (prefix stripping, canonical-session cleanup, and stale per-agent credential/base URL removal) are applied uniformly.
    // Bypassing it was the root cause of #2380.
    // A provider-only PATCH keeps the stored model.
    // Ignoring that shape returned 200 without changing anything (#7765).
    let requested_model = req.model.as_deref().filter(|model| !model.is_empty());
    let explicit_provider = req
        .provider
        .as_deref()
        .filter(|provider| !provider.is_empty());
    if requested_model.is_some() || explicit_provider.is_some() {
        let model = match requested_model {
            Some(model) => model.to_string(),
            None => match state.kernel.agent_registry().get(agent_id) {
                Some(entry) => entry.manifest.model.model,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
                    );
                }
            },
        };
        if let Err(e) = state
            .kernel
            .set_agent_model(agent_id, &model, explicit_provider)
        {
            let status = kernel_err_to_status(&e);
            return (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            );
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod inert_allowlist_tests {
    use super::inert_tool_allowlist_entries;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// The reporter's case: a literal tool name allowlisted while `capabilities.tools` excludes it.
    /// Step 1 already dropped the tool, so the entry can never grant it back.
    #[test]
    fn literal_entry_outside_the_declared_set_is_inert() {
        assert_eq!(
            inert_tool_allowlist_entries(&v(&["file_read"]), &v(&["web_search"])),
            v(&["web_search"])
        );
    }

    /// The false-positive guard that a string-equality implementation fails: `file_*` does admit `file_read`, so the entry is live and must not be reported.
    /// This is why the declared side is evaluated with `glob_matches`.
    #[test]
    fn entry_admitted_by_a_declared_glob_is_not_inert() {
        assert!(inert_tool_allowlist_entries(&v(&["file_*"]), &v(&["file_read"])).is_empty());
    }

    #[test]
    fn entry_matching_a_declared_literal_is_not_inert() {
        assert!(inert_tool_allowlist_entries(&v(&["file_read"]), &v(&["file_read"])).is_empty());
    }

    /// An empty `capabilities.tools` is the kernel's "unrestricted" case: Step 1's filter is a no-op, every builtin reaches the candidate set, so no entry is provably inert.
    #[test]
    fn empty_declared_set_is_unbounded_and_yields_no_warnings() {
        assert!(inert_tool_allowlist_entries(&[], &v(&["web_search", "nope"])).is_empty());
    }

    /// Same for an explicit `*` wildcard grant.
    #[test]
    fn wildcard_declared_set_is_unbounded_and_yields_no_warnings() {
        assert!(
            inert_tool_allowlist_entries(&v(&["*", "file_read"]), &v(&["web_search"])).is_empty()
        );
    }

    /// A glob entry may match a tool a later skill install introduces, so it is never *provably* inert even against a restricted declared set.
    #[test]
    fn glob_entry_is_never_reported() {
        assert!(inert_tool_allowlist_entries(&v(&["file_read"]), &v(&["web_*"])).is_empty());
    }

    /// MCP tools join the candidate set without being filtered by `capabilities.tools`, and their names depend on which servers are connected, so an MCP-namespaced entry is never reported.
    #[test]
    fn mcp_namespaced_entry_is_never_reported() {
        assert!(
            inert_tool_allowlist_entries(&v(&["file_read"]), &v(&["mcp_github_create_issue"]))
                .is_empty()
        );
    }

    /// Self-evolution tools are injected regardless of `capabilities.tools`.
    #[test]
    fn evolve_tool_entry_is_never_reported() {
        assert!(inert_tool_allowlist_entries(
            &v(&["file_read"]),
            &v(&["skill_evolve_create", "skill_read_file"])
        )
        .is_empty());
    }

    /// Mixed input reports only the inert entries, in submission order.
    #[test]
    fn only_inert_entries_are_reported_and_order_is_preserved() {
        assert_eq!(
            inert_tool_allowlist_entries(
                &v(&["file_*", "shell_exec"]),
                &v(&["web_search", "file_write", "agent_spawn", "shell_exec"]),
            ),
            v(&["web_search", "agent_spawn"])
        );
    }
}

#[cfg(test)]
mod skill_registry_poison_tests {
    use super::read_agent_skills_registry;
    use librefang_skills::registry::SkillRegistry;
    use std::sync::RwLock;

    #[test]
    fn agent_skills_registry_recovers_after_held_write_lock_panic() {
        let registry = RwLock::new(SkillRegistry::new(std::path::PathBuf::from(
            "/tmp/agent-skills-registry-poison-test",
        )));
        let _ = std::panic::catch_unwind(|| {
            let _guard = registry.write().unwrap();
            panic!("poison agent skill-assignment registry");
        });
        assert!(registry.is_poisoned());

        assert!(read_agent_skills_registry(&registry)
            .skill_names()
            .is_empty());

        assert!(!registry.is_poisoned());
        assert!(registry.read().is_ok());
        assert!(registry.write().is_ok());
    }
}
