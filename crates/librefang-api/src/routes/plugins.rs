//! Context engine plugin management endpoints.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

use super::AppState;

use crate::types::ApiErrorResponse;
/// Build routes for the context engine plugin domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/plugins/registries",
            axum::routing::get(list_plugin_registries),
        )
        .route("/plugins", axum::routing::get(list_plugins))
        .route("/plugins/install", axum::routing::post(install_plugin))
        .route("/plugins/uninstall", axum::routing::post(uninstall_plugin))
        .route("/plugins/scaffold", axum::routing::post(scaffold_plugin))
        .route("/plugins/doctor", axum::routing::get(plugin_doctor))
        .route("/plugins/{name}", axum::routing::get(get_plugin))
        .route(
            "/plugins/{name}/status",
            axum::routing::get(plugin_status),
        )
        .route(
            "/plugins/{name}/install-deps",
            axum::routing::post(install_plugin_deps),
        )
        .route(
            "/plugins/{name}/reload",
            axum::routing::post(reload_plugin),
        )
        .route(
            "/context-engine/metrics",
            axum::routing::get(context_engine_metrics),
        )
        .route(
            "/context-engine/traces",
            axum::routing::get(context_engine_traces),
        )
        .route(
            "/plugins/{name}/enable",
            axum::routing::post(enable_plugin),
        )
        .route(
            "/plugins/{name}/disable",
            axum::routing::post(disable_plugin),
        )
        .route(
            "/plugins/{name}/upgrade",
            axum::routing::post(upgrade_plugin),
        )
        .route(
            "/plugins/{name}/test-hook",
            axum::routing::post(test_plugin_hook),
        )
        .route(
            "/plugins/{name}/lint",
            axum::routing::get(lint_plugin),
        )
        .route(
            "/plugins/{name}/sign",
            axum::routing::post(sign_plugin),
        )
        .route(
            "/context-engine/health",
            axum::routing::get(context_engine_health),
        )
        .route(
            "/context-engine/chain",
            axum::routing::get(context_engine_chain),
        )
        .route(
            "/context-engine/metrics/prometheus",
            axum::routing::get(context_engine_metrics_prometheus),
        )
        .route(
            "/plugins/batch",
            axum::routing::post(batch_plugin_operation),
        )
        .route(
            "/plugins/{name}/export",
            axum::routing::get(export_plugin),
        )
        .route(
            "/plugins/{name}/update-check",
            axum::routing::get(plugin_update_check),
        )
        .route(
            "/plugins/{name}/benchmark",
            axum::routing::post(benchmark_plugin_hook),
        )
        .route(
            "/plugins/{name}/state",
            axum::routing::get(get_plugin_state).delete(reset_plugin_state),
        )
}

/// Query parameters for `GET /api/plugins`.
#[derive(serde::Deserialize, Default)]
pub struct ListPluginsQuery {
    /// Filter by enabled state: `true` = enabled only, `false` = disabled only.
    pub enabled: Option<bool>,
    /// Filter to plugins with lint errors: `true` = broken plugins only.
    pub has_errors: Option<bool>,
}

/// GET /api/plugins — List all installed context engine plugins.
#[utoipa::path(
    get,
    path = "/api/plugins",
    tag = "plugins",
    responses(
        (status = 200, description = "List installed plugins", body = serde_json::Value)
    )
)]
pub async fn list_plugins(
    Query(query): Query<ListPluginsQuery>,
) -> impl IntoResponse {
    let mut plugins = librefang_runtime::plugin_manager::list_plugins();

    // Apply enabled filter
    if let Some(enabled) = query.enabled {
        plugins.retain(|p| p.enabled == enabled);
    }

    // Apply has_errors filter (runs lint on each plugin)
    if let Some(want_errors) = query.has_errors {
        plugins.retain(|p| {
            let has_err = librefang_runtime::plugin_manager::lint_plugin(&p.manifest.name)
                .map(|r| !r.ok)
                .unwrap_or(false);
            has_err == want_errors
        });
    }
    let items: Vec<serde_json::Value> = plugins
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.manifest.name,
                "version": p.manifest.version,
                "description": p.manifest.description,
                "author": p.manifest.author,
                "hooks_valid": p.hooks_valid,
                "size_bytes": p.size_bytes,
                "path": p.path.display().to_string(),
                "enabled": p.enabled,
                "hooks": {
                    "ingest": p.manifest.hooks.ingest,
                    "after_turn": p.manifest.hooks.after_turn,
                },
            })
        })
        .collect();

    Json(serde_json::json!({
        "plugins": items,
        "total": items.len(),
        "plugins_dir": librefang_runtime::plugin_manager::plugins_dir().display().to_string(),
    }))
}

/// GET /api/plugins/:name — Get details of a specific plugin.
#[utoipa::path(
    get,
    path = "/api/plugins/{name}",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Plugin details", body = serde_json::Value),
        (status = 404, description = "Plugin not found")
    )
)]
pub async fn get_plugin(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::get_plugin_info(&name) {
        Ok(info) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": info.manifest.name,
                "version": info.manifest.version,
                "description": info.manifest.description,
                "author": info.manifest.author,
                "hooks": {
                    "ingest": info.manifest.hooks.ingest,
                    "after_turn": info.manifest.hooks.after_turn,
                },
                "hooks_valid": info.hooks_valid,
                "size_bytes": info.size_bytes,
                "path": info.path.display().to_string(),
                "enabled": info.enabled,
                "requirements": info.manifest.requirements,
                "plugin_depends": info.manifest.plugin_depends,
                "integrity_count": info.manifest.integrity.len(),
            })),
        ),
        Err(e) => ApiErrorResponse::not_found(e).into_json_tuple(),
    }
}

/// POST /api/plugins/install — Install a plugin from registry, local path, or git URL.
///
/// Request body:
/// ```json
/// {"source": "registry", "name": "qdrant-recall"}
/// {"source": "local", "path": "/path/to/plugin"}
/// {"source": "git", "url": "https://github.com/user/plugin.git", "branch": "main"}
/// ```
#[utoipa::path(
    post,
    path = "/api/plugins/install",
    tag = "plugins",
    request_body = serde_json::Value,
    responses(
        (status = 201, description = "Plugin installed", body = serde_json::Value),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Plugin already installed")
    )
)]
pub async fn install_plugin(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let source = match body.get("source").and_then(|s| s.as_str()) {
        Some("registry") => {
            let name = match body.get("name").and_then(|n| n.as_str()) {
                Some(n) => n.to_string(),
                None => {
                    return ApiErrorResponse::bad_request("Missing 'name' for registry install")
                        .into_json_tuple()
                }
            };
            let github_repo = body
                .get("registry")
                .and_then(|r| r.as_str())
                .map(String::from);
            librefang_runtime::plugin_manager::PluginSource::Registry { name, github_repo }
        }
        Some("local") => {
            let path = match body.get("path").and_then(|p| p.as_str()) {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    return ApiErrorResponse::bad_request("Missing 'path' for local install")
                        .into_json_tuple()
                }
            };
            librefang_runtime::plugin_manager::PluginSource::Local { path }
        }
        Some("git") => {
            let url = match body.get("url").and_then(|u| u.as_str()) {
                Some(u) => u.to_string(),
                None => {
                    return ApiErrorResponse::bad_request("Missing 'url' for git install")
                        .into_json_tuple()
                }
            };
            let branch = body
                .get("branch")
                .and_then(|b| b.as_str())
                .map(String::from);
            librefang_runtime::plugin_manager::PluginSource::Git { url, branch }
        }
        _ => {
            return ApiErrorResponse::bad_request(
                "Invalid source. Use 'registry', 'local', or 'git'",
            )
            .into_json_tuple()
        }
    };

    match librefang_runtime::plugin_manager::install_plugin(&source).await {
        Ok(info) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "installed": true,
                "name": info.manifest.name,
                "version": info.manifest.version,
                "path": info.path.display().to_string(),
                "restart_required": true,
            })),
        ),
        Err(e) => {
            let status = if e.contains("already installed") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(serde_json::json!({"error": e})))
        }
    }
}

/// POST /api/plugins/uninstall — Remove an installed plugin.
///
/// Request body: `{"name": "plugin-name"}`
#[utoipa::path(
    post,
    path = "/api/plugins/uninstall",
    tag = "plugins",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Plugin removed"),
        (status = 404, description = "Plugin not found")
    )
)]
pub async fn uninstall_plugin(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let name = match body.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return ApiErrorResponse::bad_request("Missing 'name'").into_json_tuple(),
    };

    match librefang_runtime::plugin_manager::remove_plugin(name) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"removed": true, "name": name})),
        ),
        Err(e) => {
            let status = if e.contains("not installed") || e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({"error": e})))
        }
    }
}

/// POST /api/plugins/scaffold — Create a new plugin from template.
///
/// Request body:
/// ```json
/// {
///   "name": "my-plugin",
///   "description": "My custom plugin",
///   "runtime": "python"  // optional: python (default) | v | node | deno | go | native
/// }
/// ```
#[utoipa::path(
    post,
    path = "/api/plugins/scaffold",
    tag = "plugins",
    request_body = serde_json::Value,
    responses(
        (status = 201, description = "Plugin scaffolded"),
        (status = 409, description = "Plugin already exists")
    )
)]
pub async fn scaffold_plugin(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let name = match body.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return ApiErrorResponse::bad_request("Missing 'name'").into_json_tuple(),
    };
    let description = body
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("");
    // Optional runtime tag — defaults to "python" when omitted for BC.
    let runtime = body.get("runtime").and_then(|r| r.as_str());

    match librefang_runtime::plugin_manager::scaffold_plugin(name, description, runtime) {
        Ok(path) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "scaffolded": true,
                "name": name,
                "path": path.display().to_string(),
            })),
        ),
        Err(e) => {
            let status = if e.contains("already exists") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(serde_json::json!({"error": e})))
        }
    }
}

/// GET /api/plugins/doctor — Diagnose runtime availability + per-plugin readiness.
///
/// Probes every supported runtime (`python`, `node`, `go`, ...) for its
/// launcher on PATH, then cross-references with every installed plugin to
/// flag which ones will fail at hook time because their runtime is missing.
#[utoipa::path(
    get,
    path = "/api/plugins/doctor",
    tag = "plugins",
    responses(
        (status = 200, description = "Runtime availability + per-plugin diagnostics", body = serde_json::Value)
    )
)]
pub async fn plugin_doctor() -> impl IntoResponse {
    // `run_doctor` spawns subprocesses — keep it off the async runtime.
    let report = tokio::task::spawn_blocking(librefang_runtime::plugin_manager::run_doctor)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "plugin doctor task panicked");
            librefang_runtime::plugin_manager::DoctorReport {
                runtimes: Vec::new(),
                plugins: Vec::new(),
            }
        });
    Json(report)
}

/// POST /api/plugins/:name/install-deps — Install Python dependencies for a plugin.
#[utoipa::path(
    post,
    path = "/api/plugins/{name}/install-deps",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Dependencies installed"),
        (status = 400, description = "Installation failed")
    )
)]
pub async fn install_plugin_deps(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::install_requirements(&name).await {
        Ok(output) => (
            StatusCode::OK,
            Json(serde_json::json!({"success": true, "output": output})),
        ),
        Err(e) => ApiErrorResponse::bad_request(e).into_json_tuple(),
    }
}

/// POST /api/plugins/:name/reload — Re-read `plugin.toml` from disk and validate it.
///
/// Script changes (edits to hook `.py` / binary files) take effect immediately
/// because scripts are re-executed fresh on every invocation. Manifest changes
/// (adding or removing hook declarations) are reflected in the response but
/// require an **agent restart** to become active in the running context engine.
#[utoipa::path(
    post,
    path = "/api/plugins/{name}/reload",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Manifest reloaded", body = serde_json::Value),
        (status = 400, description = "Reload failed (invalid name or bad manifest)")
    )
)]
pub async fn reload_plugin(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::reload_plugin(&name) {
        Ok(info) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": info.manifest.name,
                "version": info.manifest.version,
                "hooks_valid": info.hooks_valid,
                "message": "Manifest reloaded. Script changes take effect immediately; hook additions/removals require agent restart."
            })),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// GET /api/plugins/:name/status — Current runtime status of an installed plugin.
///
/// Returns the plugin's manifest info plus whether it is currently active in the
/// running context engine (i.e. matches the configured `plugin` or appears in
/// `plugin_stack`).
#[utoipa::path(
    get,
    path = "/api/plugins/{name}/status",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Plugin status", body = serde_json::Value),
        (status = 400, description = "Plugin not found or invalid name")
    )
)]
pub async fn plugin_status(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let info = match librefang_runtime::plugin_manager::get_plugin_info(&name) {
        Ok(i) => i,
        Err(e) => return ApiErrorResponse::bad_request(e).into_response(),
    };

    let cfg = state.kernel.config_ref();
    let ctx_cfg = &cfg.context_engine;

    // Determine whether this plugin is the currently active one.
    let active_single = ctx_cfg
        .plugin
        .as_deref()
        .map(|p| p == name)
        .unwrap_or(false);
    let stack_position: Option<usize> = ctx_cfg.plugin_stack.as_ref().and_then(|stack| {
        stack.iter().position(|p| p == &name)
    });
    let is_active = active_single || stack_position.is_some();

    Json(serde_json::json!({
        "name": info.manifest.name,
        "version": info.manifest.version,
        "description": info.manifest.description,
        "hooks_valid": info.hooks_valid,
        "size_bytes": info.size_bytes,
        "path": info.path.display().to_string(),
        "enabled": info.enabled,
        "active": is_active,
        "active_as_single": active_single,
        "stack_position": stack_position,
        "min_version_required": info.manifest.librefang_min_version,
    }))
    .into_response()
}

/// GET /api/context-engine/metrics — Hook invocation metrics for the running context engine.
///
/// Returns per-hook counters (calls, successes, failures, cumulative latency in ms).
/// Returns 204 when the active context engine does not expose metrics (e.g. the
/// default engine with no plugin configured).
#[utoipa::path(
    get,
    path = "/api/context-engine/metrics",
    tag = "plugins",
    responses(
        (status = 200, description = "Hook metrics snapshot", body = serde_json::Value),
        (status = 204, description = "No metrics available (no plugin engine active)")
    )
)]
pub async fn context_engine_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.context_engine_ref().and_then(|e| e.hook_metrics()) {
        Some(metrics) => (StatusCode::OK, Json(serde_json::to_value(&metrics).unwrap_or_default())).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// GET /api/plugins/registries — List configured plugin registries and their available plugins.
#[utoipa::path(
    get,
    path = "/api/plugins/registries",
    tag = "plugins",
    responses(
        (status = 200, description = "Configured registries with available plugins", body = serde_json::Value)
    )
)]
pub async fn list_plugin_registries(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Ensure the official registry is always present.
    let mut registries = state
        .kernel
        .config_ref()
        .context_engine
        .plugin_registries
        .clone();

    // Merge registries from [plugins].plugin_registries (URL strings treated as github repos)
    let cfg = state.kernel.config_ref();
    for url in &cfg.plugins.plugin_registries {
        if !registries.iter().any(|r| r.github_repo == *url) {
            registries.push(librefang_types::config::PluginRegistrySource {
                name: url.clone(),
                github_repo: url.clone(),
            });
        }
    }
    if !registries
        .iter()
        .any(|r| r.github_repo == "librefang/librefang-registry")
    {
        registries.insert(
            0,
            librefang_types::config::PluginRegistrySource {
                name: "Official".to_string(),
                github_repo: "librefang/librefang-registry".to_string(),
            },
        );
    }

    let installed = librefang_runtime::plugin_manager::list_plugins();
    let installed_names: std::collections::HashSet<String> =
        installed.iter().map(|p| p.manifest.name.clone()).collect();

    let mut results = Vec::new();
    for reg in &registries {
        let plugins = match librefang_runtime::plugin_manager::list_registry_plugins(
            &reg.github_repo,
        )
        .await
        {
            Ok(entries) => entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "name": e.name,
                        "installed": installed_names.contains(&e.name),
                    })
                })
                .collect::<Vec<_>>(),
            Err(e) => {
                results.push(serde_json::json!({
                    "name": reg.name,
                    "github_repo": reg.github_repo,
                    "error": e,
                    "plugins": [],
                }));
                continue;
            }
        };
        results.push(serde_json::json!({
            "name": reg.name,
            "github_repo": reg.github_repo,
            "plugins": plugins,
        }));
    }

    Json(serde_json::json!({ "registries": results }))
}

/// GET /api/context-engine/traces — Recent hook invocation traces (ring buffer, last 100).
///
/// Returns per-invocation debug data: hook name, timestamp, elapsed_ms, success/failure,
/// input/output previews. Returns 204 when no plugin engine is active.
#[utoipa::path(
    get,
    path = "/api/context-engine/traces",
    tag = "plugins",
    responses(
        (status = 200, description = "Hook invocation traces", body = serde_json::Value),
        (status = 204, description = "No plugin engine active")
    )
)]
pub async fn context_engine_traces(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.context_engine_ref() {
        Some(engine) => {
            let traces = engine.hook_traces();
            let count = traces.len();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "traces": traces,
                    "count": count,
                })),
            )
                .into_response()
        }
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// POST /api/plugins/:name/enable — Enable a disabled plugin.
///
/// Removes the `.disabled` marker file. The running context engine must be
/// restarted for the change to take effect.
#[utoipa::path(
    post,
    path = "/api/plugins/{name}/enable",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Plugin enabled"),
        (status = 400, description = "Plugin not found or already enabled")
    )
)]
pub async fn enable_plugin(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::enable_plugin(&name) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "enabled": true,
                "name": name,
                "message": "Plugin enabled. Restart context engine for change to take effect.",
            })),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// POST /api/plugins/:name/disable — Disable a plugin without uninstalling it.
///
/// Creates a `.disabled` marker file. The running context engine must be
/// restarted for the change to take effect.
#[utoipa::path(
    post,
    path = "/api/plugins/{name}/disable",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Plugin disabled"),
        (status = 400, description = "Plugin not found or already disabled")
    )
)]
pub async fn disable_plugin(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::disable_plugin(&name) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "enabled": false,
                "name": name,
                "message": "Plugin disabled. Restart context engine for change to take effect.",
            })),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// POST /api/plugins/:name/upgrade — Upgrade an installed plugin from a new source.
///
/// Removes the current version and reinstalls from the given source. The
/// `.disabled` state is preserved. Requires a context engine restart.
///
/// Request body (same as install):
/// ```json
/// {"source": "registry", "name": "qdrant-recall"}
/// {"source": "local", "path": "/path/to/newer-version"}
/// {"source": "git", "url": "https://github.com/user/plugin.git", "branch": "main"}
/// ```
#[utoipa::path(
    post,
    path = "/api/plugins/{name}/upgrade",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Plugin upgraded"),
        (status = 400, description = "Plugin not installed or upgrade failed")
    )
)]
pub async fn upgrade_plugin(
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let source = match body.get("source").and_then(|s| s.as_str()) {
        Some("registry") => {
            let plugin_name = body
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(&name)
                .to_string();
            let github_repo = body
                .get("registry")
                .and_then(|r| r.as_str())
                .map(String::from);
            librefang_runtime::plugin_manager::PluginSource::Registry {
                name: plugin_name,
                github_repo,
            }
        }
        Some("local") => {
            let path = match body.get("path").and_then(|p| p.as_str()) {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    return ApiErrorResponse::bad_request("Missing 'path' for local upgrade")
                        .into_response()
                }
            };
            librefang_runtime::plugin_manager::PluginSource::Local { path }
        }
        Some("git") => {
            let url = match body.get("url").and_then(|u| u.as_str()) {
                Some(u) => u.to_string(),
                None => {
                    return ApiErrorResponse::bad_request("Missing 'url' for git upgrade")
                        .into_response()
                }
            };
            let branch = body
                .get("branch")
                .and_then(|b| b.as_str())
                .map(String::from);
            librefang_runtime::plugin_manager::PluginSource::Git { url, branch }
        }
        _ => {
            // Default to upgrading from registry using the path parameter name
            librefang_runtime::plugin_manager::PluginSource::Registry {
                name: name.clone(),
                github_repo: None,
            }
        }
    };

    match librefang_runtime::plugin_manager::upgrade_plugin(&name, &source).await {
        Ok(info) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "upgraded": true,
                "name": info.manifest.name,
                "version": info.manifest.version,
                "path": info.path.display().to_string(),
                "restart_required": true,
            })),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// POST /api/plugins/:name/test-hook — Invoke a specific hook with test input.
///
/// Runs the hook subprocess with the given JSON input and returns the output.
/// Useful for debugging and validating hook scripts without sending real messages.
///
/// Request body:
/// ```json
/// {
///   "hook": "ingest",
///   "input": {"type": "ingest", "agent_id": "test", "message": "hello", "peer_id": null}
/// }
/// ```
#[utoipa::path(
    post,
    path = "/api/plugins/{name}/test-hook",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Hook output"),
        (status = 400, description = "Hook not declared or invocation failed")
    )
)]
pub async fn test_plugin_hook(
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let hook_name = match body.get("hook").and_then(|h| h.as_str()) {
        Some(h) => h.to_string(),
        None => return ApiErrorResponse::bad_request("Missing 'hook' field").into_response(),
    };
    let input = body.get("input").cloned().unwrap_or(serde_json::json!({}));

    // Load plugin manifest
    let info = match librefang_runtime::plugin_manager::get_plugin_info(&name) {
        Ok(i) => i,
        Err(e) => return ApiErrorResponse::not_found(e).into_response(),
    };

    // Resolve the hook script path
    let hooks = &info.manifest.hooks;
    let script_rel = match hook_name.as_str() {
        "ingest" => hooks.ingest.as_deref(),
        "after_turn" => hooks.after_turn.as_deref(),
        "assemble" => hooks.assemble.as_deref(),
        "compact" => hooks.compact.as_deref(),
        "bootstrap" => hooks.bootstrap.as_deref(),
        "prepare_subagent" => hooks.prepare_subagent.as_deref(),
        "merge_subagent" => hooks.merge_subagent.as_deref(),
        _ => {
            return ApiErrorResponse::bad_request(format!(
                "Unknown hook '{hook_name}'. Valid hooks: ingest, after_turn, assemble, compact, bootstrap, prepare_subagent, merge_subagent"
            ))
            .into_response()
        }
    };

    let script_rel = match script_rel {
        Some(s) => s.to_string(),
        None => {
            return ApiErrorResponse::bad_request(format!(
                "Hook '{hook_name}' is not declared in plugin '{name}'"
            ))
            .into_response()
        }
    };

    let script_abs = info.path.join(&script_rel);
    if !script_abs.exists() {
        return ApiErrorResponse::bad_request(format!(
            "Hook script '{script_rel}' does not exist on disk"
        ))
        .into_response();
    }

    let runtime = librefang_runtime::plugin_runtime::PluginRuntime::from_tag(
        hooks.runtime.as_deref(),
    );
    let timeout_secs = hooks.hook_timeout_secs.unwrap_or(30);

    // Build hook config and run
    // Convert manifest env (HashMap<String, String>) to Vec<(String, String)>
    // Note: env lives on PluginManifest, not on ContextEngineHooks
    let plugin_env: Vec<(String, String)> = info
        .manifest
        .env
        .iter()
        .map(|(k, v): (&String, &String)| (k.clone(), v.clone()))
        .collect();

    let config = librefang_runtime::plugin_runtime::HookConfig {
        timeout_secs,
        plugin_env,
        max_memory_mb: info.manifest.hooks.max_memory_mb,
        allow_network: info.manifest.hooks.allow_network,
        ..Default::default()
    };

    let start = std::time::Instant::now();
    match librefang_runtime::plugin_runtime::run_hook_json(
        &script_abs.to_string_lossy(),
        runtime,
        &input,
        &config,
    )
    .await
    {
        Ok(output) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "hook": hook_name,
                    "plugin": name,
                    "success": true,
                    "elapsed_ms": elapsed_ms,
                    "output": output,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "hook": hook_name,
                    "plugin": name,
                    "success": false,
                    "elapsed_ms": elapsed_ms,
                    "error": e.to_string(),
                })),
            )
                .into_response()
        }
    }
}

/// POST /api/plugins/:name/sign — Compute and write SHA-256 integrity hashes into plugin.toml.
///
/// After signing, the plugin will be verified against these hashes on every load.
/// Re-run after editing any hook scripts to keep the hashes up to date.
#[utoipa::path(
    post,
    path = "/api/plugins/{name}/sign",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Hashes written to plugin.toml", body = serde_json::Value),
        (status = 400, description = "Plugin not found or no hooks declared")
    )
)]
pub async fn sign_plugin(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::sign_plugin(&name) {
        Ok(hashes) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "signed": true,
                "plugin": name,
                "hashes": hashes,
                "count": hashes.len(),
                "message": "Integrity hashes written to plugin.toml. Re-sign after editing hook scripts.",
            })),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// GET /api/plugins/:name/lint — Validate plugin manifest and hook script structure.
///
/// Returns a lint report with errors (structural problems) and warnings
/// (best-practice suggestions). Does not execute any hook scripts.
#[utoipa::path(
    get,
    path = "/api/plugins/{name}/lint",
    tag = "plugins",
    params(("name" = String, Path, description = "Plugin name")),
    responses(
        (status = 200, description = "Lint report", body = serde_json::Value),
        (status = 400, description = "Plugin not found")
    )
)]
pub async fn lint_plugin(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::lint_plugin(&name) {
        Ok(report) => {
            let status = if report.ok {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (status, Json(serde_json::to_value(&report).unwrap_or_default())).into_response()
        }
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// GET /api/context-engine/health — Lightweight smoke test of the active plugin engine.
///
/// Verifies that all declared hook scripts exist on disk and are executable
/// (for native runtime). Does not invoke any hook subprocess.
/// Returns 200 when healthy, 503 when degraded.
#[utoipa::path(
    get,
    path = "/api/context-engine/health",
    tag = "plugins",
    responses(
        (status = 200, description = "Engine healthy"),
        (status = 204, description = "No plugin engine configured"),
        (status = 503, description = "Engine degraded — hook scripts missing or invalid")
    )
)]
pub async fn context_engine_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let ctx_cfg = &cfg.context_engine;

    // Determine which plugins are active
    let mut active_plugins: Vec<String> = Vec::new();
    if let Some(ref p) = ctx_cfg.plugin {
        active_plugins.push(p.clone());
    }
    if let Some(ref stack) = ctx_cfg.plugin_stack {
        for p in stack {
            if !active_plugins.contains(p) {
                active_plugins.push(p.clone());
            }
        }
    }

    if active_plugins.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let mut issues: Vec<serde_json::Value> = Vec::new();
    let mut all_ok = true;

    for plugin_name in &active_plugins {
        match librefang_runtime::plugin_manager::lint_plugin(plugin_name) {
            Ok(report) => {
                if !report.ok {
                    all_ok = false;
                    issues.push(serde_json::json!({
                        "plugin": plugin_name,
                        "errors": report.errors,
                    }));
                }
            }
            Err(e) => {
                all_ok = false;
                issues.push(serde_json::json!({
                    "plugin": plugin_name,
                    "errors": [e],
                }));
            }
        }
    }

    let status = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(serde_json::json!({
            "healthy": all_ok,
            "active_plugins": active_plugins,
            "issues": issues,
        })),
    )
        .into_response()
}

/// GET /api/context-engine/chain — Show the active context engine topology.
///
/// Describes whether the engine is the default (no plugin), a single plugin,
/// or a stacked chain of plugins, and lists hook coverage for each.
#[utoipa::path(
    get,
    path = "/api/context-engine/chain",
    tag = "plugins",
    responses(
        (status = 200, description = "Engine chain topology", body = serde_json::Value)
    )
)]
pub async fn context_engine_chain(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let ctx_cfg = &cfg.context_engine;

    let single = ctx_cfg.plugin.as_deref();
    let stack = ctx_cfg.plugin_stack.as_deref().unwrap_or(&[]);

    let mode = if !stack.is_empty() {
        "stacked"
    } else if single.is_some() {
        "single"
    } else {
        "default"
    };

    // Build per-plugin hook coverage info
    let mut chain: Vec<serde_json::Value> = Vec::new();

    let plugins_to_describe: Vec<&str> = if !stack.is_empty() {
        stack.iter().map(|s| s.as_str()).collect()
    } else if let Some(p) = single {
        vec![p]
    } else {
        vec![]
    };

    for plugin_name in &plugins_to_describe {
        let hooks_info = match librefang_runtime::plugin_manager::get_plugin_info(plugin_name) {
            Ok(info) => {
                let hooks = &info.manifest.hooks;
                serde_json::json!({
                    "ingest": hooks.ingest.is_some(),
                    "after_turn": hooks.after_turn.is_some(),
                    "assemble": hooks.assemble.is_some(),
                    "compact": hooks.compact.is_some(),
                    "bootstrap": hooks.bootstrap.is_some(),
                    "prepare_subagent": hooks.prepare_subagent.is_some(),
                    "merge_subagent": hooks.merge_subagent.is_some(),
                    "runtime": hooks.runtime,
                    "enabled": info.enabled,
                    "hooks_valid": info.hooks_valid,
                    "cache_ttl_secs": hooks.hook_cache_ttl_secs,
                })
            }
            Err(e) => serde_json::json!({ "error": e }),
        };
        chain.push(serde_json::json!({
            "plugin": plugin_name,
            "hooks": hooks_info,
        }));
    }

    Json(serde_json::json!({
        "mode": mode,
        "chain": chain,
        "chain_length": chain.len(),
        "fallback": "default engine (embedding recall)",
    }))
    .into_response()
}

/// GET /api/context-engine/metrics/prometheus — Hook metrics in Prometheus text format.
///
/// Returns metrics for direct scraping by Prometheus or Grafana.
/// Returns 204 when no plugin engine is active.
pub async fn context_engine_metrics_prometheus(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Some(metrics) = state.kernel.context_engine_ref().and_then(|e| e.hook_metrics()) else {
        return (StatusCode::NO_CONTENT, String::new()).into_response();
    };

    let mut output = String::new();
    output.push_str("# HELP librefang_hook_calls_total Total hook invocations\n");
    output.push_str("# TYPE librefang_hook_calls_total counter\n");
    output.push_str("# HELP librefang_hook_errors_total Total hook failures\n");
    output.push_str("# TYPE librefang_hook_errors_total counter\n");
    output.push_str("# HELP librefang_hook_latency_ms_total Cumulative hook latency in milliseconds\n");
    output.push_str("# TYPE librefang_hook_latency_ms_total counter\n");

    let hook_pairs = [
        ("ingest", &metrics.ingest),
        ("after_turn", &metrics.after_turn),
        ("bootstrap", &metrics.bootstrap),
        ("assemble", &metrics.assemble),
        ("compact", &metrics.compact),
        ("prepare_subagent", &metrics.prepare_subagent),
        ("merge_subagent", &metrics.merge_subagent),
    ];
    for (hook, stats) in hook_pairs {
        output.push_str(&format!(
            "librefang_hook_calls_total{{hook=\"{}\"}} {}\n",
            hook, stats.calls
        ));
        output.push_str(&format!(
            "librefang_hook_errors_total{{hook=\"{}\"}} {}\n",
            hook, stats.failures
        ));
        output.push_str(&format!(
            "librefang_hook_latency_ms_total{{hook=\"{}\"}} {}\n",
            hook, stats.total_ms
        ));
        if stats.calls > 0 {
            let avg = stats.total_ms / stats.calls;
            output.push_str(&format!(
                "librefang_hook_latency_ms_avg{{hook=\"{}\"}} {}\n",
                hook, avg
            ));
        }
    }

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        output,
    )
        .into_response()
}

/// POST /api/plugins/batch — Apply an operation to multiple plugins at once.
///
/// Request body:
/// ```json
/// {"operation": "enable", "plugins": ["plugin-a", "plugin-b"]}
/// {"operation": "disable", "plugins": ["plugin-a"]}
/// {"operation": "lint", "plugins": ["plugin-a", "plugin-b"]}
/// {"operation": "sign", "plugins": ["plugin-a"]}
/// ```
pub async fn batch_plugin_operation(
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let operation = match body.get("operation").and_then(|o| o.as_str()) {
        Some(op) => op.to_string(),
        None => return ApiErrorResponse::bad_request("Missing 'operation' field").into_response(),
    };
    let plugins: Vec<String> = match body.get("plugins").and_then(|p| p.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        None => return ApiErrorResponse::bad_request("Missing 'plugins' array").into_response(),
    };

    if plugins.is_empty() {
        return ApiErrorResponse::bad_request("'plugins' array is empty").into_response();
    }

    let mut results = Vec::new();
    for name in &plugins {
        let result = match operation.as_str() {
            "enable" => librefang_runtime::plugin_manager::enable_plugin(name)
                .map(|_| serde_json::json!({"ok": true}))
                .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e})),
            "disable" => librefang_runtime::plugin_manager::disable_plugin(name)
                .map(|_| serde_json::json!({"ok": true}))
                .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e})),
            "lint" => librefang_runtime::plugin_manager::lint_plugin(name)
                .map(|r| serde_json::to_value(&r).unwrap_or_default())
                .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e})),
            "sign" => librefang_runtime::plugin_manager::sign_plugin(name)
                .map(|h| serde_json::json!({"ok": true, "hashes": h}))
                .unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e})),
            _ => serde_json::json!({"ok": false, "error": format!("Unknown operation '{operation}'")}),
        };
        results.push(serde_json::json!({"plugin": name, "result": result}));
    }

    let all_ok = results.iter().all(|r| r["result"]["ok"].as_bool().unwrap_or(false));
    Json(serde_json::json!({
        "operation": operation,
        "results": results,
        "all_ok": all_ok,
    }))
    .into_response()
}

/// GET /api/plugins/:name/export — Download a plugin as a tar archive.
///
/// Returns a gzip-compressed tar of the plugin directory, suitable for
/// backup or transfer to another installation.
pub async fn export_plugin(Path(name): Path<String>) -> impl IntoResponse {
    use axum::body::Body;

    let info = match librefang_runtime::plugin_manager::get_plugin_info(&name) {
        Ok(i) => i,
        Err(e) => return ApiErrorResponse::not_found(e).into_response(),
    };

    // Build tar in memory
    let tar_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut tar = tar::Builder::new(enc);
            tar.append_dir_all(&info.manifest.name, &info.path)
                .map_err(|e| format!("Failed to create tar: {e}"))?;
            tar.finish().map_err(|e| format!("Failed to finalize tar: {e}"))?;
        }
        Ok(buf)
    })
    .await
    .unwrap_or_else(|e| Err(format!("Task panicked: {e}")));

    match tar_bytes {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "application/gzip"),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    &format!("attachment; filename=\"{name}.tar.gz\"") as &str,
                ),
            ],
            Body::from(bytes),
        )
            .into_response(),
        Err(e) => ApiErrorResponse::bad_request(e).into_response(),
    }
}

/// GET /api/plugins/:name/update-check — Check if a newer version is available in the registry.
///
/// Compares the installed version against the registry manifest. Uses the
/// configured default registry.
pub async fn plugin_update_check(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let info = match librefang_runtime::plugin_manager::get_plugin_info(&name) {
        Ok(i) => i,
        Err(e) => return ApiErrorResponse::not_found(e).into_response(),
    };

    let registry = state
        .kernel
        .config_ref()
        .context_engine
        .plugin_registries
        .first()
        .map(|r| r.github_repo.clone())
        .unwrap_or_else(|| "librefang/librefang-registry".to_string());

    // Fetch registry manifest for this plugin
    let client = match reqwest::Client::builder()
        .user_agent("librefang-plugin-updater/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": format!("HTTP client error: {e}")})),
            )
                .into_response();
        }
    };

    let manifest_url = format!(
        "https://raw.githubusercontent.com/{registry}/main/plugins/{name}/plugin.toml"
    );

    match client.get(&manifest_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.text().await {
                Ok(text) => {
                    let registry_version = toml::from_str::<toml::Value>(&text)
                        .ok()
                        .and_then(|v| v.get("version")?.as_str().map(str::to_string));

                    let installed_version = &info.manifest.version;
                    let update_available = registry_version
                        .as_deref()
                        .map(|rv| rv != installed_version)
                        .unwrap_or(false);

                    Json(serde_json::json!({
                        "plugin": name,
                        "installed_version": installed_version,
                        "registry_version": registry_version,
                        "update_available": update_available,
                        "registry": registry,
                    }))
                    .into_response()
                }
                Err(e) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("Failed to read registry response: {e}")})),
                )
                    .into_response(),
            }
        }
        Ok(resp) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "plugin": name,
                "registry": registry,
                "error": format!("Not found in registry (HTTP {})", resp.status()),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": format!("Registry unreachable: {e}")})),
        )
            .into_response(),
    }
}

/// POST /api/plugins/:name/benchmark — Run a hook N times and report latency stats.
///
/// Request body:
/// ```json
/// {
///   "hook": "ingest",
///   "input": {"type": "ingest", "agent_id": "test", "message": "hello", "peer_id": null},
///   "runs": 10
/// }
/// ```
pub async fn benchmark_plugin_hook(
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let hook_name = match body.get("hook").and_then(|h| h.as_str()) {
        Some(h) => h.to_string(),
        None => return ApiErrorResponse::bad_request("Missing 'hook' field").into_response(),
    };
    let input = body.get("input").cloned().unwrap_or(serde_json::json!({}));
    let runs = body.get("runs").and_then(|r| r.as_u64()).unwrap_or(5).min(50) as usize;

    let info = match librefang_runtime::plugin_manager::get_plugin_info(&name) {
        Ok(i) => i,
        Err(e) => return ApiErrorResponse::not_found(e).into_response(),
    };

    let hooks = &info.manifest.hooks;
    let script_rel = match hook_name.as_str() {
        "ingest" => hooks.ingest.as_deref(),
        "after_turn" => hooks.after_turn.as_deref(),
        "assemble" => hooks.assemble.as_deref(),
        "compact" => hooks.compact.as_deref(),
        "bootstrap" => hooks.bootstrap.as_deref(),
        _ => return ApiErrorResponse::bad_request(format!("Unknown hook '{hook_name}'")).into_response(),
    };
    let script_rel = match script_rel {
        Some(s) => s.to_string(),
        None => return ApiErrorResponse::bad_request(format!("Hook '{hook_name}' not declared")).into_response(),
    };

    let script_abs = info.path.join(&script_rel);
    let runtime = librefang_runtime::plugin_runtime::PluginRuntime::from_tag(hooks.runtime.as_deref());
    let plugin_env: Vec<(String, String)> = info.manifest.env.iter()
        .map(|(k, v): (&String, &String)| (k.clone(), v.clone()))
        .collect();
    let config = librefang_runtime::plugin_runtime::HookConfig {
        timeout_secs: hooks.hook_timeout_secs.unwrap_or(30),
        plugin_env,
        max_memory_mb: hooks.max_memory_mb,
        allow_network: hooks.allow_network,
        ..Default::default()
    };

    let mut latencies_ms: Vec<u64> = Vec::with_capacity(runs);
    let mut errors = 0u64;

    for _ in 0..runs {
        let start = std::time::Instant::now();
        match librefang_runtime::plugin_runtime::run_hook_json(
            &script_abs.to_string_lossy(),
            runtime,
            &input,
            &config,
        )
        .await
        {
            Ok(_) => latencies_ms.push(start.elapsed().as_millis() as u64),
            Err(_) => {
                errors += 1;
                latencies_ms.push(start.elapsed().as_millis() as u64);
            }
        }
    }

    latencies_ms.sort_unstable();
    let total: u64 = latencies_ms.iter().sum();
    let avg = if runs > 0 { total / runs as u64 } else { 0 };
    let p50 = latencies_ms.get(runs / 2).copied().unwrap_or(0);
    let p95 = latencies_ms.get(runs * 95 / 100).copied().unwrap_or(0);
    let p99 = latencies_ms.get(runs * 99 / 100).copied().unwrap_or(0);
    let min = latencies_ms.first().copied().unwrap_or(0);
    let max = latencies_ms.last().copied().unwrap_or(0);

    Json(serde_json::json!({
        "hook": hook_name,
        "plugin": name,
        "runs": runs,
        "errors": errors,
        "latency_ms": {
            "min": min,
            "max": max,
            "avg": avg,
            "p50": p50,
            "p95": p95,
            "p99": p99,
            "all": latencies_ms,
        }
    }))
    .into_response()
}

/// GET /api/plugins/:name/state — Read the plugin's shared state JSON file.
///
/// Returns `{}` when the state file doesn't exist or shared state is not enabled.
pub async fn get_plugin_state(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::validate_plugin_name(&name) {
        Ok(()) => {}
        Err(e) => return ApiErrorResponse::bad_request(e).into_response(),
    }
    let state_path = librefang_runtime::plugin_manager::plugins_dir()
        .join(&name)
        .join(".state.json");

    let content = if state_path.exists() {
        std::fs::read_to_string(&state_path).unwrap_or_else(|_| "{}".to_string())
    } else {
        "{}".to_string()
    };

    let value: serde_json::Value = serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
    (StatusCode::OK, Json(value)).into_response()
}

/// DELETE /api/plugins/:name/state — Reset the plugin's shared state to `{}`.
pub async fn reset_plugin_state(Path(name): Path<String>) -> impl IntoResponse {
    match librefang_runtime::plugin_manager::validate_plugin_name(&name) {
        Ok(()) => {}
        Err(e) => return ApiErrorResponse::bad_request(e).into_response(),
    }
    let state_path = librefang_runtime::plugin_manager::plugins_dir()
        .join(&name)
        .join(".state.json");

    match std::fs::write(&state_path, "{}") {
        Ok(()) => Json(serde_json::json!({"reset": true, "plugin": name})).into_response(),
        Err(e) => ApiErrorResponse::bad_request(format!("Failed to reset state: {e}")).into_response(),
    }
}
