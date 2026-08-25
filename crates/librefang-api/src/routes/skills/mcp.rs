use super::*;

/// Snapshot the kernel's *effective* MCP server set (file `[[mcp_servers]]`
/// merged with the DB-backed `mcp_server_configs` overlay) — the list the
/// kernel actually runs. Read and existence/duplicate checks MUST use this
/// rather than `config_ref().mcp_servers` (file-only), otherwise servers
/// persisted with `mcp_runtime_store = "db"` (#6113) are invisible to the API
/// and cannot be updated or deleted. Returns an owned `Vec` so the `!Send`
/// `std::sync::RwLock` guard is never held across an `.await`.
fn effective_mcp_servers_snapshot(
    state: &Arc<AppState>,
) -> Vec<librefang_types::config::McpServerConfigEntry> {
    state
        .kernel
        .effective_mcp_servers_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Strip values out of an MCP `env` list, leaving only variable names (#6630).
///
/// `McpServerConfigEntry::env` is documented as "variables to pass through" (`["GITHUB_PERSONAL_ACCESS_TOKEN"]`) but the supported representation also accepts an inline `KEY=VALUE`, so an operator can put a live credential in there.
/// Every response that serialized the raw list therefore handed those credentials to the caller — and `GET /api/mcp/servers` is in `PUBLIC_ROUTES_DASHBOARD_READS`, so in open mode (`require_auth_for_reads` unset, the default) that caller does not even need a token.
/// The reported Viewer-role exposure is the authenticated case of the same hole.
///
/// Names are kept because they are the useful, non-secret half: the dashboard renders them, and an operator needs to know which variables a server expects.
/// The shape stays `Vec<String>` rather than becoming a list of objects so the dashboard's read-edit-write round-trip keeps working — see `merge_env_preserving_inline_values` for the other half of that contract.
fn mcp_env_names(env: &[String]) -> Vec<String> {
    env.iter()
        .map(|entry| match entry.split_once('=') {
            Some((name, _value)) => name.trim().to_string(),
            None => entry.trim().to_string(),
        })
        .collect()
}

/// Merge a submitted `env` list with what is already persisted, keeping inline values the caller could not have seen (#6630).
///
/// Redacting the read side alone would have been a data-loss bug worse than the disclosure.
/// `McpServersPage` hydrates its edit form from `GET /api/mcp/servers` and submits every field back on save, so a caller who changed only, say, `timeout_secs` would round-trip the name-only `env` list and silently wipe the inline values out of the stored config — and the server would then fail to connect for a reason nothing in the UI explains.
///
/// So a submitted bare `NAME` is treated as "unchanged" when the stored entry for that name carries an inline value: the stored form wins.
/// A submitted `NAME=value` is always taken as an explicit new value, and a name absent from the submission is dropped as an explicit removal.
///
/// Known limitation: because a bare `NAME` means "keep", an inline value cannot be *cleared* by editing it down to the bare name — remove the entry and add it back instead.
/// That is the deliberate trade for never silently destroying a credential the caller was not shown.
fn merge_env_preserving_inline_values(incoming: &[String], existing: &[String]) -> Vec<String> {
    // name → the stored entry in full (`NAME=value`).
    // Only entries that actually carry an inline value are candidates for restoration.
    let stored_inline: std::collections::HashMap<&str, &String> = existing
        .iter()
        .filter_map(|entry| entry.split_once('=').map(|(name, _)| (name.trim(), entry)))
        .collect();

    incoming
        .iter()
        .map(|entry| match entry.split_once('=') {
            // Explicit value supplied — take it verbatim.
            Some(_) => entry.clone(),
            // Bare name: restore the stored inline form if there was one.
            None => stored_inline
                .get(entry.trim())
                .map(|stored| (*stored).clone())
                .unwrap_or_else(|| entry.clone()),
        })
        .collect()
}

/// The `http_compat` counterpart of [`merge_env_preserving_inline_values`]: restore static header values the caller was never shown (#6612).
///
/// A `[[mcp_servers.transport.headers]]` entry may carry its value inline as `value`, which makes it a credential, so `http_compat_header_summary` omits it for the same reason `env` is reduced to names (#6630).
/// That leaves the same asymmetry on the write side: a client that reads the `transport` object, edits `base_url`, and submits it back would blank every static header value in the stored config, and the transport would then fail with `Missing environment variable` or send an empty header.
///
/// Semantics mirror the `env` merge exactly, keyed by header name: a submitted header carrying a `value` is an explicit new value and wins; a submitted header without a `value` restores the stored `value` for that name if there was one; a header absent from the submission is an explicit removal.
/// A submitted `value_env` is never modified — it names a variable rather than holding a secret, so it round-trips verbatim.
/// Its *presence* does not suppress the restore, though: a header may legitimately carry both, `apply_http_compat_headers` reads `value` first, and skipping on `value_env` would silently demote such a header from "send this secret" to "resolve this variable" on any read-modify-write, with nothing logged.
/// A header that only ever had a `value_env` is unaffected either way, because the stored-value lookup has no entry for a name that never carried a `value`.
///
/// The merge applies only when the submitted and stored transports are *both* `http_compat`.
/// An operator switching a server from `http_compat` to `stdio` and back must not have the old credentials silently resurrected into the new transport.
/// Under the current implementation that guard is belt-and-braces — the stored-value lookup is built from the stored `http_compat` headers, so a non-`http_compat` stored transport has nothing to restore from regardless — but it keeps the invariant explicit for any future version that sourced values from elsewhere.
///
/// Known limitation, inherited from the `env` merge: because a value-less header means "keep", a static value cannot be *cleared* in a single write.
/// Clearing takes two: one `PUT` omitting the header, which removes it and with it the stored value, then one adding it back.
/// That applies equally to clearing the `value` off a header that carries both, leaving it purely env-sourced.
/// It is the deliberate trade for never silently destroying a credential the caller was not shown.
fn merge_http_compat_secrets(
    incoming: &mut librefang_types::config::McpTransportEntry,
    existing: &librefang_types::config::McpTransportEntry,
) {
    use librefang_types::config::McpTransportEntry::HttpCompat;

    // Both sides must be `http_compat`; a transport-type change carries nothing forward.
    let (
        HttpCompat {
            headers: incoming_headers,
            ..
        },
        HttpCompat {
            headers: existing_headers,
            ..
        },
    ) = (incoming, existing)
    else {
        return;
    };

    let stored_static: std::collections::HashMap<&str, &String> = existing_headers
        .iter()
        .filter_map(|h| h.value.as_ref().map(|v| (h.name.trim(), v)))
        .collect();

    for header in incoming_headers.iter_mut() {
        // An explicitly submitted value is taken as-is; only an absent one is restored.
        // The presence of `value_env` must NOT suppress the restore: a header may carry both, and `value` wins at request time, so skipping on `value_env` would silently demote a "send this secret" header to a "resolve this variable" one on any read-modify-write.
        // A purely env-sourced header is unaffected either way, because `stored_static` only has entries for names that had a stored `value`.
        if header.value.is_some() {
            continue;
        }
        if let Some(stored) = stored_static.get(header.name.trim()) {
            header.value = Some((*stored).clone());
        }
    }
}

/// Persist an MCP server upsert according to `config.mcp_runtime_store`, then
/// make it effective without a restart. `File` (default) rewrites
/// `config.toml` and runs a full config reload — byte-for-byte the pre-#6113
/// behaviour. `Db` writes the SQLite `mcp_server_configs` table, leaving
/// `config.toml` untouched (for read-only `config.toml` deployments, the
/// #6021 motivation), then re-runs the MCP merge so the new row connects
/// immediately. Returns the reload-status string for the response body, or a
/// ready-to-return error tuple.
///
/// Managed mode (#6695) is enforced **per branch, not per route**.
/// The `File` branch is refused because it rewrites `config.toml`; the `Db` branch is left writable because it never opens the file, which is exactly what `mcp_runtime_store = "db"` was introduced for in #6021.
/// Locking the route outright would take the dashboard's whole MCP install surface away from a managed deployment even when that deployment has already moved the persistence off the managed file — over-locking a configuration that is not actually being written.
/// The refusal therefore doubles as the fix: an operator who hits `423` here sets `mcp_runtime_store = "db"` in the manifest and gets the surface back.
async fn persist_mcp_upsert(
    state: &Arc<AppState>,
    entry: &librefang_types::config::McpServerConfigEntry,
) -> Result<&'static str, (StatusCode, Json<serde_json::Value>)> {
    match state.kernel.config_ref().mcp_runtime_store {
        librefang_types::config::McpRuntimeStore::File => {
            if let Some(locked) = crate::routes::guard_config_write(state.kernel.config_path()) {
                return Err(locked);
            }
            let config_path = state.kernel.config_path().to_path_buf();
            if let Err(e) = upsert_mcp_server_config(&config_path, entry) {
                return Err(ApiErrorResponse::internal_scrub(e).into_json_tuple());
            }
            Ok(reload_via_config(state).await)
        }
        librefang_types::config::McpRuntimeStore::Db => {
            let store =
                librefang_memory::McpConfigStore::new(state.kernel.memory_substrate().pool());
            if let Err(e) = store.upsert(entry) {
                return Err(ApiErrorResponse::internal_scrub(e.to_string()).into_json_tuple());
            }
            Ok(reload_via_mcp(state).await)
        }
    }
}

/// Delete counterpart of [`persist_mcp_upsert`] — removes the entry from the
/// configured store and re-applies the effective set.
///
/// Carries the same per-branch managed-mode split, and for the stronger reason: without it a `DELETE` would strip a `[[mcp_servers]]` entry the manifest declares, and the next rollout would silently bring it back.
async fn persist_mcp_delete(
    state: &Arc<AppState>,
    name: &str,
) -> Result<&'static str, (StatusCode, Json<serde_json::Value>)> {
    match state.kernel.config_ref().mcp_runtime_store {
        librefang_types::config::McpRuntimeStore::File => {
            if let Some(locked) = crate::routes::guard_config_write(state.kernel.config_path()) {
                return Err(locked);
            }
            let config_path = state.kernel.config_path().to_path_buf();
            if let Err(e) = remove_mcp_server_config(&config_path, name) {
                return Err(ApiErrorResponse::internal_scrub(e).into_json_tuple());
            }
            Ok(reload_via_config(state).await)
        }
        librefang_types::config::McpRuntimeStore::Db => {
            let store =
                librefang_memory::McpConfigStore::new(state.kernel.memory_substrate().pool());
            if let Err(e) = store.delete(name) {
                return Err(ApiErrorResponse::internal_scrub(e.to_string()).into_json_tuple());
            }
            Ok(reload_via_mcp(state).await)
        }
    }
}

/// Full config reload (`File` store path). Maps the plan to the response's
/// `reload` status string.
async fn reload_via_config(state: &Arc<AppState>) -> &'static str {
    match state.kernel.reload_config().await {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    }
}

/// MCP-only reload (`Db` store path) — re-runs the file+DB merge and connects
/// new servers without re-reading `config.toml`.
async fn reload_via_mcp(state: &Arc<AppState>) -> &'static str {
    match state.kernel.clone().reload_mcp_servers().await {
        Ok(_) => "applied",
        Err(_) => "saved_reload_failed",
    }
}

/// GET /api/mcp/taint-rules — List configured `[[taint_rules]]`.
///
/// Issue #3050 follow-up: the dashboard `TaintPolicyEditor` references
/// rule-set names by free-form string. Without this read-only endpoint,
/// the editor cannot tell the operator that a typed name doesn't match
/// any registered set — and the scanner silently treats unknown names
/// as no-ops (one-shot WARN in
/// `librefang_runtime_mcp::warn_unknown_rule_set_once`). The dashboard
/// uses this list to render an inline validation hint next to the
/// `rule_sets` field.
#[utoipa::path(
    get,
    path = "/api/mcp/taint-rules",
    tag = "mcp",
    responses(
        (status = 200, description = "List configured named taint rule sets", body = crate::types::JsonArray)
    )
)]
pub async fn list_mcp_taint_rules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let payload: Vec<serde_json::Value> = state
        .kernel
        .config_ref()
        .taint_rules
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "action": r.action,
                "rule_count": r.rules.len(),
            })
        })
        .collect();
    (StatusCode::OK, Json(payload))
}

/// GET /api/mcp/servers — List configured MCP servers and their tools.
#[utoipa::path(
    get,
    path = "/api/mcp/servers",
    tag = "mcp",
    responses(
        (status = 200, description = "List configured MCP servers and their tools", body = crate::types::JsonObject)
    )
)]
pub async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Snapshot auth states so we can include them in the response
    let auth_states = state.kernel.mcp_auth_states_ref().lock().await;
    let auth_snapshot: std::collections::HashMap<String, serde_json::Value> = auth_states
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                serde_json::to_value(v).unwrap_or(serde_json::json!({"state": "not_required"})),
            )
        })
        .collect();
    drop(auth_states);

    // Effective set = file `[[mcp_servers]]` + DB overlay (#6113), so
    // DB-backed servers are listed too.
    let config_servers: Vec<serde_json::Value> = effective_mcp_servers_snapshot(&state)
        .iter()
        .map(|s| {
            let transport = s.transport.as_ref().map(serialize_mcp_transport);
            let auth_state = auth_snapshot
                .get(&s.name)
                .cloned()
                .unwrap_or(serde_json::json!({"state": "not_required"}));
            serde_json::json!({
                "name": s.name,
                "template_id": s.template_id,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                // Names only — an inline `KEY=VALUE` must never reach a caller (#6630).
                // This route is in PUBLIC_ROUTES_DASHBOARD_READS, so in open mode that caller can be unauthenticated.
                "env": mcp_env_names(&s.env),
                "auth_state": auth_state,
                // Issue #3050: surface taint config so the dashboard tree
                // editor can hydrate without a separate fetch.
                "taint_scanning": s.taint_scanning,
                "taint_policy": s.taint_policy,
            })
        })
        .collect();

    // Get connected servers and their tools from the live MCP connections.
    //
    // `connected` reflects liveness, not just vec residency: a subprocess that
    // died silently (stdio transport crash, SSE drop) leaves its McpConnection
    // in the vec until the health loop or a reconnect replaces it. Cross-check
    // with `mcp_health` so the badge/count match reality (#2738).
    let connections = state.kernel.mcp_connections_ref().lock().await;
    let health = state.kernel.mcp_health();
    let connected: Vec<serde_json::Value> = connections
        .iter()
        .map(|conn| {
            let tools: Vec<serde_json::Value> = conn
                .tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                    })
                })
                .collect();
            let is_alive = matches!(
                health.get_health(conn.name()).map(|h| h.status),
                Some(librefang_types::mcp::McpStatus::Ready),
            );
            serde_json::json!({
                "name": conn.name(),
                "tools_count": tools.len(),
                "tools": tools,
                "connected": is_alive,
            })
        })
        .collect();

    let total_connected = connected
        .iter()
        .filter(|c| {
            c.get("connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count();

    Json(serde_json::json!({
        "configured": config_servers,
        "connected": connected,
        "total_configured": config_servers.len(),
        "total_connected": total_connected,
    }))
}

/// GET /api/mcp/servers/{name} — Retrieve a single MCP server by name.
///
/// Returns the configured server entry plus live connection status and tools
/// if the server is currently connected.
#[utoipa::path(
    get,
    path = "/api/mcp/servers/{name}",
    tag = "mcp",
    params(
        ("name" = String, Path, description = "Server name"),
    ),
    responses(
        (status = 200, description = "MCP server details", body = crate::types::JsonObject),
        (status = 404, description = "MCP server not found")
    )
)]
pub async fn get_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Find the entry in the effective set (file + DB overlay, #6113). Owned
    // snapshot so the borrow is safe to hold across the .await below.
    let servers = effective_mcp_servers_snapshot(&state);
    let entry = match servers.iter().find(|s| s.name == name) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
                .into_json_tuple();
        }
    };

    let transport = entry.transport.as_ref().map(serialize_mcp_transport);

    let mut result = serde_json::json!({
        "name": entry.name,
        "template_id": entry.template_id,
        "transport": transport,
        "timeout_secs": entry.timeout_secs,
        // Names only, same contract as the list route (#6630).
        "env": mcp_env_names(&entry.env),
        "connected": false,
        // Issue #3050: surface taint config so the dashboard tree editor
        // can hydrate without a separate fetch.
        "taint_scanning": entry.taint_scanning,
        "taint_policy": entry.taint_policy,
    });

    // Check live connection status
    let connections = state.kernel.mcp_connections_ref().lock().await;
    if let Some(conn) = connections.iter().find(|c| c.name() == name) {
        let tools: Vec<serde_json::Value> = conn
            .tools()
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                })
            })
            .collect();
        if let Some(obj) = result.as_object_mut() {
            obj.insert("connected".to_string(), serde_json::json!(true));
            obj.insert("tools_count".to_string(), serde_json::json!(tools.len()));
            obj.insert("tools".to_string(), serde_json::json!(tools));
        }
    }

    (StatusCode::OK, Json(result))
}

/// POST /api/mcp/servers — Add a new MCP server configuration.
///
/// Expects a JSON body matching `McpServerConfigEntry` (name, transport, timeout_secs, env).
/// Persists to config.toml and triggers a config reload.
#[utoipa::path(
    post,
    path = "/api/mcp/servers",
    tag = "mcp",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Add a new MCP server configuration", body = crate::types::JsonObject)
    )
)]
pub async fn add_mcp_server(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Two accepted shapes:
    //   (A) Template install: { "template_id": "github", "credentials": { ... } }
    //   (B) Raw entry:        { "name": "...", "transport": { ... }, ... }
    let (entry, name) = if let Some(tid) = body
        .get("template_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        // Template install path
        let creds: std::collections::HashMap<String, String> = body
            .get("credentials")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let catalog = state.kernel.mcp_catalog_load();
        let entry = match catalog.get(&tid) {
            Some(e) => e.clone(),
            None => {
                return ApiErrorResponse::not_found(format!("MCP catalog entry '{tid}' not found"))
                    .into_json_tuple();
            }
        };
        drop(catalog);

        // Duplicate-name check BEFORE running the installer. `install_integration`
        // stores provided credentials in the vault as a side effect, so if we
        // returned 409 from the check below (which used to run after install)
        // the vault would already hold credentials for a server the caller never
        // managed to register. Reject first, side-effect second.
        let prospective_name = entry.id.clone();
        if effective_mcp_servers_snapshot(&state)
            .iter()
            .any(|s| s.name == prospective_name)
        {
            return ApiErrorResponse::conflict(format!(
                "MCP server '{prospective_name}' already exists"
            ))
            .into_json_tuple();
        }

        // Route through the kernel facade: cached vault (no per-request
        // Argon2id KDF) + cached catalog snapshot (#3598).
        let result = match state.kernel.install_integration(&entry.id, &creds) {
            Ok(r) => r,
            Err(e) => {
                return ApiErrorResponse::bad_request(format!("Install failed: {e}"))
                    .into_json_tuple();
            }
        };
        (result.server, result.id)
    } else {
        // Raw entry path
        let name = match body.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => {
                return ApiErrorResponse::bad_request("Missing or empty 'name' field")
                    .into_json_tuple();
            }
        };

        if body.get("transport").is_none() {
            return ApiErrorResponse::bad_request("Missing 'transport' field").into_json_tuple();
        }

        let entry: librefang_types::config::McpServerConfigEntry =
            match serde_json::from_value(body) {
                Ok(e) => e,
                Err(e) => {
                    return ApiErrorResponse::bad_request(format!(
                        "Invalid MCP server config: {e}"
                    ))
                    .into_json_tuple();
                }
            };
        (entry, name)
    };

    // Check for duplicate name
    if effective_mcp_servers_snapshot(&state)
        .iter()
        .any(|s| s.name == name)
    {
        return ApiErrorResponse::conflict(format!("MCP server '{}' already exists", name))
            .into_json_tuple();
    }

    // Persist per `mcp_runtime_store` (config.toml or DB) and apply.
    let reload_status = match persist_mcp_upsert(&state, &entry).await {
        Ok(status) => status,
        Err(resp) => return resp,
    };

    // Establish connection to the newly added server in the background.
    // Wrap in `spawn_supervised` so a panic inside `connect_mcp_servers`
    // (e.g. parse failure, OAuth handshake, tool list deserialization) is
    // logged at `error!` rather than silently aborting the detached task
    // and leaving the new server stuck in a half-connecting state.
    let kernel = std::sync::Arc::clone(&state.kernel);
    librefang_kernel::supervised_spawn::spawn_supervised(
        "connect_mcp_servers_after_add",
        async move { kernel.connect_mcp_servers().await },
    );

    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("mcp_server added: {name}"),
        "completed",
        user_id,
        Some("api".to_string()),
    );

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "added",
            "name": name,
            "template_id": entry.template_id,
            "reload": reload_status,
        })),
    )
}

/// PUT /api/mcp/servers/{name} — Update an existing MCP server configuration.
///
/// Replaces the existing entry with the provided JSON body. The `name` path
/// parameter identifies which server to update; the body's `name` field (if
/// present) is ignored in favour of the path parameter.
#[utoipa::path(
    put,
    path = "/api/mcp/servers/{name}",
    tag = "mcp",
    params(
        ("name" = String, Path, description = "Server name"),
    ),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Update an existing MCP server configuration", body = crate::types::JsonObject)
    )
)]
pub async fn update_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Json(mut body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Ensure the entry exists (effective set = file + DB overlay, #6113)
    if !effective_mcp_servers_snapshot(&state)
        .iter()
        .any(|s| s.name == name)
    {
        return ApiErrorResponse::not_found(
            t.t_args("api-error-mcp-not-found", &[("name", &name)]),
        )
        .into_json_tuple();
    }

    // Force the name in body to match the path parameter
    if let Some(obj) = body.as_object_mut() {
        obj.insert("name".to_string(), serde_json::json!(name));
    }

    if body.get("transport").is_none() {
        return ApiErrorResponse::bad_request(t.t("api-error-mcp-missing-transport"))
            .into_json_tuple();
    }

    // Validate by deserializing
    let mut entry: librefang_types::config::McpServerConfigEntry =
        match serde_json::from_value(body) {
            Ok(e) => e,
            Err(e) => {
                return ApiErrorResponse::bad_request(
                    t.t_args("api-error-mcp-invalid-config", &[("error", &e.to_string())]),
                )
                .into_json_tuple();
            }
        };

    // Reads return `env` as names only (#6630), so a client that hydrated its form from GET and submitted every field back would otherwise wipe the inline values it was never shown.
    // Restore them for any bare name.
    // Static `http_compat` header values are redacted on read for the same reason and get the same treatment (#6612).
    if let Some(existing) = effective_mcp_servers_snapshot(&state)
        .into_iter()
        .find(|s| s.name == name)
    {
        entry.env = merge_env_preserving_inline_values(&entry.env, &existing.env);
        if let (Some(incoming_transport), Some(existing_transport)) =
            (entry.transport.as_mut(), existing.transport.as_ref())
        {
            merge_http_compat_secrets(incoming_transport, existing_transport);
        }
    }

    // Drop ErrorTranslator before .await — FluentBundle is !Send and cannot
    // be held across an async suspension point.
    drop(t);

    // Persist per `mcp_runtime_store` (config.toml or DB) and apply.
    let reload_status = match persist_mcp_upsert(&state, &entry).await {
        Ok(status) => status,
        Err(resp) => return resp,
    };

    // Disconnect the old connection so connect_mcp_servers picks up the new config.
    state.kernel.disconnect_mcp_server(&name).await;
    let kernel = std::sync::Arc::clone(&state.kernel);
    librefang_kernel::supervised_spawn::spawn_supervised(
        "connect_mcp_servers_after_update",
        async move { kernel.connect_mcp_servers().await },
    );

    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("mcp_server updated: {name}"),
        "completed",
        user_id,
        Some("api".to_string()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "name": name,
            "reload": reload_status,
        })),
    )
}

#[utoipa::path(
    patch,
    path = "/api/mcp/servers/{name}/taint",
    tag = "mcp",
    params(("name" = String, Path, description = "Server name")),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Taint settings updated", body = crate::types::JsonObject),
        (status = 404, description = "Server not found", body = crate::types::JsonObject),
    )
)]
#[allow(private_interfaces)]
pub async fn patch_mcp_server_taint(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Json(body): Json<PatchMcpTaintRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    // Locate and clone the existing entry so we mutate a fresh copy that's
    // safe to pass to persist_mcp_upsert without touching the live config
    // until persistence succeeds. Effective set = file + DB overlay (#6113).
    let mut entry = match effective_mcp_servers_snapshot(&state)
        .iter()
        .find(|s| s.name == name)
        .cloned()
    {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(
                t.t_args("api-error-mcp-not-found", &[("name", &name)]),
            )
            .into_json_tuple();
        }
    };

    if let Some(scanning) = body.taint_scanning {
        entry.taint_scanning = scanning;
    }
    if let Some(policy) = body.taint_policy {
        entry.taint_policy = Some(policy);
    }

    // Drop ErrorTranslator before .await — FluentBundle is !Send and cannot
    // be held across an async suspension point.
    drop(t);

    // Persist per `mcp_runtime_store` (config.toml or DB) and apply.
    let reload_status = match persist_mcp_upsert(&state, &entry).await {
        Ok(status) => status,
        Err(resp) => return resp,
    };

    // Reconnect so the new taint_policy snapshot reaches the live
    // `McpServerConfig.taint_policy` field. The shared `taint_rules_swap`
    // already updates via `reload_config` without a reconnect.
    state.kernel.disconnect_mcp_server(&name).await;
    let kernel = std::sync::Arc::clone(&state.kernel);
    librefang_kernel::supervised_spawn::spawn_supervised(
        "connect_mcp_servers_after_taint_patch",
        async move { kernel.connect_mcp_servers().await },
    );

    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("mcp_server taint updated: {name}"),
        "completed",
        user_id,
        Some("api".to_string()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "updated",
            "name": name,
            "reload": reload_status,
        })),
    )
}

/// DELETE /api/mcp/servers/{name} — Remove an MCP server configuration.
#[utoipa::path(
    delete,
    path = "/api/mcp/servers/{name}",
    tag = "mcp",
    params(
        ("name" = String, Path, description = "Server name"),
    ),
    responses(
        (status = 200, description = "Remove an MCP server configuration", body = crate::types::JsonObject)
    )
)]
pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Ensure the entry exists in the effective set (file + DB overlay, #6113),
    // and resolve its URL before removal (needed for vault cleanup).
    let server_url = match effective_mcp_servers_snapshot(&state)
        .iter()
        .find(|s| s.name == name)
    {
        Some(s) => match &s.transport {
            Some(librefang_types::config::McpTransportEntry::Http { url }) => Some(url.clone()),
            Some(librefang_types::config::McpTransportEntry::Sse { url }) => Some(url.clone()),
            _ => None,
        },
        None => {
            return ApiErrorResponse::not_found(
                t.t_args("api-error-mcp-not-found", &[("name", &name)]),
            )
            .into_json_tuple();
        }
    };
    drop(t);

    // Persist the removal per `mcp_runtime_store` (config.toml or DB) and apply.
    let reload_status = match persist_mcp_delete(&state, &name).await {
        Ok(status) => status,
        Err(resp) => return resp,
    };

    // Clean up OAuth vault tokens, auth state, and live connections.
    //
    // #3651: replaced `let _ = vault_remove(...)` so vault crypto failures
    // during MCP server uninstall are no longer silently dropped. Behavior
    // is intentionally unchanged on success (uninstall continues even if a
    // few vault entries can't be wiped — the auth state is reset
    // unconditionally below) but each failure now produces an `audit` log
    // line so operators can detect leftover credentials after a wrong-key
    // boot.
    if let Some(ref url) = server_url {
        let provider = KernelOAuthProvider::new(state.kernel.home_dir().to_path_buf());
        for field in &[
            "access_token",
            "refresh_token",
            "expires_at",
            "token_endpoint",
            "client_id",
            "pkce_verifier",
            "pkce_state",
            "redirect_uri",
        ] {
            let vault_key = KernelOAuthProvider::vault_key(url, field);
            if let Err(e) = provider.vault_remove(&vault_key) {
                tracing::error!(
                    target: "audit",
                    op = "vault_remove",
                    key = %vault_key,
                    error = %e,
                    "vault op failed during MCP server uninstall"
                );
            }
        }
    }
    state
        .kernel
        .mcp_auth_states_ref()
        .lock()
        .await
        .remove(&name);
    state.kernel.disconnect_mcp_server(&name).await;

    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("mcp_server removed: {name}"),
        "completed",
        user_id,
        Some("api".to_string()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "removed",
            "name": name,
            "reload": reload_status,
        })),
    )
}

/// GET /api/mcp/catalog — List all installable MCP catalog entries.
#[utoipa::path(
    get,
    path = "/api/mcp/catalog",
    tag = "mcp",
    responses(
        (status = 200, description = "MCP catalog entries", body = crate::types::JsonObject)
    )
)]
pub async fn list_mcp_catalog(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let installed_ids = collect_installed_catalog_ids(&state);

    let catalog = state.kernel.mcp_catalog_load();
    let entries: Vec<serde_json::Value> = catalog
        .list()
        .iter()
        .map(|e| render_catalog_entry(e, &installed_ids, lang))
        .collect();
    Json(serde_json::json!({
        "entries": entries,
        "count": entries.len(),
    }))
}

/// GET /api/mcp/catalog/{id} — Single catalog entry detail.
#[utoipa::path(
    get,
    path = "/api/mcp/catalog/{id}",
    tag = "mcp",
    params(("id" = String, Path, description = "Catalog entry id")),
    responses(
        (status = 200, description = "Catalog entry detail", body = crate::types::JsonObject),
        (status = 404, description = "Catalog entry not found", body = crate::types::JsonObject),
    )
)]
pub async fn get_mcp_catalog_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let installed_ids = collect_installed_catalog_ids(&state);

    let catalog = state.kernel.mcp_catalog_load();
    match catalog.get(&id) {
        Some(entry) => (
            StatusCode::OK,
            Json(render_catalog_entry(entry, &installed_ids, lang)),
        ),
        None => ApiErrorResponse::not_found(format!("MCP catalog entry '{}' not found", id))
            .into_json_tuple(),
    }
}

/// POST /api/mcp/servers/{name}/reconnect — Force a reconnect of an MCP server.
#[utoipa::path(
    post,
    path = "/api/mcp/servers/{name}/reconnect",
    tag = "mcp",
    params(("name" = String, Path, description = "Server name")),
    responses(
        (status = 200, description = "Reconnect an MCP server", body = crate::types::JsonObject),
        (status = 404, description = "MCP server not configured", body = crate::types::JsonObject),
    )
)]
pub async fn reconnect_mcp_server_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let configured = effective_mcp_servers_snapshot(&state)
        .iter()
        .any(|s| s.name == name);
    if !configured {
        return ApiErrorResponse::not_found(format!("MCP server '{}' not configured", name))
            .into_json_tuple();
    }

    match state.kernel.clone().reconnect_mcp_server(&name).await {
        Ok(tool_count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": name,
                "status": "connected",
                "tool_count": tool_count,
            })),
        ),
        Err(e) => {
            // Scrub the raw reconnect error (audit:
            // rusqlite-errors-leak); operators keep the detail in the
            // log, the client sees the generic body alongside the
            // structured status fields.
            tracing::error!(error = %e, server = %name, "MCP reconnect failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "id": name,
                    "status": "error",
                    "error": "Internal server error",
                })),
            )
        }
    }
}

/// GET /api/mcp/health — Health snapshot across all configured MCP servers.
#[utoipa::path(
    get,
    path = "/api/mcp/health",
    tag = "mcp",
    responses(
        (status = 200, description = "Health snapshot for all configured MCP servers", body = crate::types::JsonObject)
    )
)]
pub async fn mcp_health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health_entries = state.kernel.mcp_health().all_health();
    let entries: Vec<serde_json::Value> = health_entries
        .iter()
        .map(|h| {
            serde_json::json!({
                "id": h.id,
                "status": h.status.to_string(),
                "tool_count": h.tool_count,
                "last_ok": h.last_ok.map(|t| t.to_rfc3339()),
                "last_error": h.last_error,
                "consecutive_failures": h.consecutive_failures,
                "reconnecting": h.reconnecting,
                "reconnect_attempts": h.reconnect_attempts,
                "connected_since": h.connected_since.map(|t| t.to_rfc3339()),
            })
        })
        .collect();

    Json(serde_json::json!({
        "health": entries,
        "count": entries.len(),
    }))
}

/// POST /api/mcp/reload — Re-read the catalog and reconnect MCP servers.
#[utoipa::path(
    post,
    path = "/api/mcp/reload",
    tag = "mcp",
    responses(
        (status = 200, description = "Reload catalog and reconnect MCP servers", body = crate::types::JsonObject)
    )
)]
pub async fn reload_mcp_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Sync the in-memory config with config.toml before reconnecting.
    // `reload_mcp_servers` reads from `self.config.load_full()`, so if the
    // caller just edited config.toml out-of-band (the CLI's `librefang mcp
    // add/remove` does this, then POSTs /api/mcp/reload) the reload would
    // otherwise run against the stale snapshot and miss the change.
    if let Err(e) = state.kernel.reload_config().await {
        return ApiErrorResponse::internal_scrub(e).into_json_tuple();
    }

    match state.kernel.clone().reload_mcp_servers().await {
        Ok(connected) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "reloaded",
                "new_connections": connected,
            })),
        ),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}
