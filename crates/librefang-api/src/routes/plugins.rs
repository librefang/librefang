//! Context engine plugin management endpoints.

use axum::extract::{Path, State};
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
pub async fn list_plugins() -> impl IntoResponse {
    let plugins = librefang_runtime::plugin_manager::list_plugins();
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
