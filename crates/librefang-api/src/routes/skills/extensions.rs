use super::*;

/// GET /api/extensions — List catalog entries annotated with installed state.
#[utoipa::path(
    get,
    path = "/api/extensions",
    tag = "extensions",
    responses(
        (status = 200, description = "List catalog entries with install/health status", body = crate::types::JsonObject)
    )
)]
pub async fn list_extensions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let installed_map = installed_servers_by_template(&cfg.mcp_servers);
    let health = state.kernel.mcp_health();

    let catalog = state.kernel.mcp_catalog_load();

    let mut extensions = Vec::new();
    for entry in catalog.list() {
        let status = status_str_for_catalog(&entry.id, &installed_map, health);
        let installed_entry = installed_map.get(&entry.id);
        let tool_count = installed_entry
            .and_then(|srv| health.get_health(&srv.name))
            .map(|h| h.tool_count)
            .unwrap_or(0);
        extensions.push(serde_json::json!({
            "name": entry.id,
            "display_name": entry.name,
            "description": entry.description,
            "icon": entry.icon,
            "category": entry.category.to_string(),
            "status": status,
            "tags": entry.tags,
            "installed": installed_entry.is_some(),
            "tool_count": tool_count,
            "installed_at": serde_json::Value::Null,
        }));
    }

    Json(serde_json::json!({
        "extensions": extensions,
        "total": extensions.len(),
    }))
}

/// GET /api/extensions/:name — Get details for a single catalog entry.
#[utoipa::path(
    get,
    path = "/api/extensions/{name}",
    tag = "extensions",
    params(
        ("name" = String, Path, description = "Catalog entry id"),
    ),
    responses(
        (status = 200, description = "Catalog entry detail + install status", body = crate::types::JsonObject)
    )
)]
pub async fn get_extension(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let installed_map = installed_servers_by_template(&cfg.mcp_servers);
    let catalog = state.kernel.mcp_catalog_load();

    let entry = match catalog.get(&name) {
        Some(t) => t.clone(),
        None => {
            return ApiErrorResponse::not_found(format!("Extension '{}' not found", name))
                .into_json_tuple();
        }
    };
    drop(catalog);

    let installed_entry = installed_map.get(&entry.id);
    let health = state.kernel.mcp_health();
    let health_snapshot = installed_entry.and_then(|srv| health.get_health(&srv.name));

    let status = status_str_for_catalog(&entry.id, &installed_map, health);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": entry.id,
            "display_name": entry.name,
            "description": entry.description,
            "icon": entry.icon,
            "category": entry.category.to_string(),
            "status": status,
            "tags": entry.tags,
            "installed": installed_entry.is_some(),
            "tool_count": health_snapshot.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed_at": serde_json::Value::Null,
            "required_env": entry.required_env.iter().map(|e| serde_json::json!({
                "name": e.name,
                "label": e.label,
                "help": e.help,
                "is_secret": e.is_secret,
                "get_url": e.get_url,
            })).collect::<Vec<_>>(),
            "has_oauth": entry.oauth.is_some(),
            "setup_instructions": entry.setup_instructions,
            "health": health_snapshot.as_ref().map(|h| serde_json::json!({
                "last_ok": h.last_ok.map(|t| t.to_rfc3339()),
                "last_error": h.last_error,
                "consecutive_failures": h.consecutive_failures,
                "reconnecting": h.reconnecting,
            })),
        })),
    )
}

/// POST /api/extensions/install — Install a catalog entry (alias for
/// POST /api/mcp/servers with template_id).
#[utoipa::path(
    post,
    path = "/api/extensions/install",
    tag = "extensions",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Install a catalog entry", body = crate::types::JsonObject)
    )
)]
pub async fn install_extension(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExtensionInstallRequest>,
) -> impl IntoResponse {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return ApiErrorResponse::bad_request("Missing or empty 'name' field").into_json_tuple();
    }

    let already_installed = state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .any(|s| s.template_id.as_deref() == Some(name.as_str()) || s.name == name);
    if already_installed {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("Extension '{}' already installed", name),
            })),
        );
    }

    // Route through the kernel facade: cached vault + cached catalog (#3598).
    let result = match state
        .kernel
        .install_integration(&name, &std::collections::HashMap::new())
    {
        Ok(r) => r,
        Err(e) => {
            let err_str = e.to_string();
            let status = match e {
                librefang_types::integration::IntegrationError::NotFound(_) => {
                    StatusCode::NOT_FOUND
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            // 404 echoes the "not found" message (caller-useful); the
            // 500 catch-all scrubs (audit: rusqlite-errors-leak) and
            // logs the full error for operators.
            let body = if status == StatusCode::INTERNAL_SERVER_ERROR {
                tracing::error!(error = %err_str, extension = %name, "extension install failed");
                "Internal server error".to_string()
            } else {
                err_str
            };
            return (status, Json(serde_json::json!({"error": body})));
        }
    };

    // Persist to the database config store (source = runtime) and sync into the
    // kernel; the helper triggers the reconnect. config.toml stays read-only
    // bootstrap (so this works when config.toml is a read-only ConfigMap).
    let server_name = result.server.name.clone();
    if let Err(resp) =
        super::mcp::apply_mcp_mutation(&state, super::mcp::McpMutation::Upsert(result.server)).await
    {
        return resp;
    }

    state.kernel.mcp_health().register(&server_name);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "installed",
            "name": name,
            // The reconnect runs via apply_mcp_mutation (background for the
            // SurrealDB path, synchronous for the legacy config.toml path).
            "connected": true,
        })),
    )
}

/// POST /api/extensions/uninstall — Uninstall by catalog id (template_id).
#[utoipa::path(
    post,
    path = "/api/extensions/uninstall",
    tag = "extensions",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Uninstall a catalog-backed MCP server", body = crate::types::JsonObject)
    )
)]
pub async fn uninstall_extension(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExtensionUninstallRequest>,
) -> impl IntoResponse {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return ApiErrorResponse::bad_request("Missing or empty 'name' field").into_json_tuple();
    }

    // Resolve template_id -> server name (may differ for raw-authored entries).
    let server_name = state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .find(|s| s.template_id.as_deref() == Some(name.as_str()) || s.name == name)
        .map(|s| s.name.clone());

    let server_name = match server_name {
        Some(n) => n,
        None => {
            return ApiErrorResponse::not_found(format!("Extension '{}' not installed", name))
                .into_json_tuple();
        }
    };

    // Disconnect + unregister first, then remove from the store and sync into
    // the kernel via the shared helper (config.toml stays read-only bootstrap).
    state.kernel.mcp_health().unregister(&server_name);
    state.kernel.disconnect_mcp_server(&server_name).await;
    if let Err(resp) =
        super::mcp::apply_mcp_mutation(&state, super::mcp::McpMutation::Remove(server_name.clone()))
            .await
    {
        return resp;
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "uninstalled",
            "name": name,
        })),
    )
}
