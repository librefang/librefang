//! Context engine plugin management endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

use super::AppState;

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
        .route("/plugins/{name}", axum::routing::get(get_plugin))
        .route(
            "/plugins/{name}/install-deps",
            axum::routing::post(install_plugin_deps),
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
                "requirements": info.manifest.requirements,
            })),
        ),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))),
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
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Missing 'name' for registry install"})),
                    )
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
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Missing 'path' for local install"})),
                    )
                }
            };
            librefang_runtime::plugin_manager::PluginSource::Local { path }
        }
        Some("git") => {
            let url = match body.get("url").and_then(|u| u.as_str()) {
                Some(u) => u.to_string(),
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Missing 'url' for git install"})),
                    )
                }
            };
            let branch = body
                .get("branch")
                .and_then(|b| b.as_str())
                .map(String::from);
            librefang_runtime::plugin_manager::PluginSource::Git { url, branch }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "Invalid source. Use 'registry', 'local', or 'git'"}),
                ),
            )
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
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'name'"})),
            )
        }
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
/// Request body: `{"name": "my-plugin", "description": "My custom plugin"}`
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
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'name'"})),
            )
        }
    };
    let description = body
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("");

    match librefang_runtime::plugin_manager::scaffold_plugin(name, description) {
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
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
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
    for url in &state.kernel.config_ref().plugins.plugin_registries {
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
