//! Skills, marketplace, ClawHub, hands, and extension handlers.

/// Build routes for the skills/marketplace/hands/MCP/integrations/extensions domain.
pub fn router() -> axum::Router<std::sync::Arc<super::AppState>> {
    axum::Router::new()
        // Skills
        .route("/skills", axum::routing::get(list_skills))
        .route("/skills/registry", axum::routing::get(list_skill_registry))
        .route("/skills/install", axum::routing::post(install_skill))
        .route("/skills/uninstall", axum::routing::post(uninstall_skill))
        .route("/skills/reload", axum::routing::post(reload_skills))
        .route("/skills/create", axum::routing::post(create_skill))
        // Marketplace / ClawHub
        .route(
            "/marketplace/search",
            axum::routing::get(marketplace_search),
        )
        .route("/clawhub/search", axum::routing::get(clawhub_search))
        .route("/clawhub/browse", axum::routing::get(clawhub_browse))
        .route(
            "/clawhub/skill/{slug}",
            axum::routing::get(clawhub_skill_detail),
        )
        .route(
            "/clawhub/skill/{slug}/code",
            axum::routing::get(clawhub_skill_code),
        )
        .route("/clawhub/install", axum::routing::post(clawhub_install))
        // Skillhub marketplace
        .route(
            "/skillhub/search",
            axum::routing::get(skillhub_search),
        )
        .route(
            "/skillhub/browse",
            axum::routing::get(skillhub_browse),
        )
        .route(
            "/skillhub/skill/{slug}",
            axum::routing::get(skillhub_skill_detail),
        )
        .route(
            "/skillhub/skill/{slug}/code",
            axum::routing::get(skillhub_skill_code),
        )
        .route(
            "/skillhub/install",
            axum::routing::post(skillhub_install),
        )
        // Hands (browser automation engine)
        .route("/hands", axum::routing::get(list_hands))
        .route("/hands/install", axum::routing::post(install_hand))
        .route("/hands/{hand_id}", axum::routing::delete(uninstall_hand))
        .route("/hands/active", axum::routing::get(list_active_hands))
        .route("/hands/{hand_id}", axum::routing::get(get_hand))
        .route(
            "/hands/{hand_id}/activate",
            axum::routing::post(activate_hand),
        )
        .route(
            "/hands/{hand_id}/check-deps",
            axum::routing::post(check_hand_deps),
        )
        .route(
            "/hands/{hand_id}/install-deps",
            axum::routing::post(install_hand_deps),
        )
        .route(
            "/hands/{hand_id}/secret",
            axum::routing::post(set_hand_secret),
        )
        .route(
            "/hands/{hand_id}/settings",
            axum::routing::get(get_hand_settings).put(update_hand_settings),
        )
        .route(
            "/hands/instances/{id}/pause",
            axum::routing::post(pause_hand),
        )
        .route(
            "/hands/instances/{id}/resume",
            axum::routing::post(resume_hand),
        )
        .route(
            "/hands/instances/{id}",
            axum::routing::delete(deactivate_hand),
        )
        .route(
            "/hands/instances/{id}/stats",
            axum::routing::get(hand_stats),
        )
        .route(
            "/hands/instances/{id}/browser",
            axum::routing::get(hand_instance_browser),
        )
        .route(
            "/hands/instances/{id}/message",
            axum::routing::post(hand_send_message),
        )
        .route(
            "/hands/instances/{id}/session",
            axum::routing::get(hand_get_session),
        )
        .route(
            "/hands/instances/{id}/status",
            axum::routing::get(hand_instance_status),
        )
        .route("/hands/reload", axum::routing::post(reload_hands))
        // MCP server management
        .route(
            "/mcp/servers",
            axum::routing::get(list_mcp_servers).post(add_mcp_server),
        )
        .route(
            "/mcp/servers/{name}",
            axum::routing::get(get_mcp_server)
                .put(update_mcp_server)
                .delete(delete_mcp_server),
        )
        // MCP OAuth auth endpoints
        .route(
            "/mcp/servers/{name}/auth/status",
            axum::routing::get(super::mcp_auth::auth_status),
        )
        .route(
            "/mcp/servers/{name}/auth/start",
            axum::routing::post(super::mcp_auth::auth_start),
        )
        .route(
            "/mcp/servers/{name}/auth/callback",
            axum::routing::get(super::mcp_auth::auth_callback),
        )
        .route(
            "/mcp/servers/{name}/auth/revoke",
            axum::routing::delete(super::mcp_auth::auth_revoke),
        )
        // Integrations
        .route("/integrations", axum::routing::get(list_integrations))
        .route(
            "/integrations/available",
            axum::routing::get(list_available_integrations),
        )
        .route("/integrations/add", axum::routing::post(add_integration))
        .route(
            "/integrations/{id}",
            axum::routing::get(get_integration).delete(remove_integration),
        )
        .route(
            "/integrations/{id}/reconnect",
            axum::routing::post(reconnect_integration),
        )
        .route(
            "/integrations/health",
            axum::routing::get(integrations_health),
        )
        .route(
            "/integrations/reload",
            axum::routing::post(reload_integrations),
        )
        // Extensions
        .route("/extensions", axum::routing::get(list_extensions))
        .route(
            "/extensions/install",
            axum::routing::post(install_extension),
        )
        .route(
            "/extensions/uninstall",
            axum::routing::post(uninstall_extension),
        )
        .route("/extensions/{name}", axum::routing::get(get_extension))
}

use super::channels::FieldType;
use super::config::json_to_toml_value;
use super::AppState;
use super::RequestLanguage;
use crate::types::*;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::i18n::ErrorTranslator;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Skills endpoints
// ---------------------------------------------------------------------------

/// GET /api/skills — List installed skills.
#[utoipa::path(
    get,
    path = "/api/skills",
    tag = "skills",
    responses(
        (status = 200, description = "List installed skills", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let skills_dir = state.kernel.home_dir().join("skills");
    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir);
    if let Err(e) = registry.load_all() {
        tracing::warn!("Failed to reload skill registry: {e}");
    }

    let skills: Vec<serde_json::Value> = registry
        .list()
        .iter()
        .map(|s| {
            let source = match &s.manifest.source {
                Some(librefang_skills::SkillSource::ClawHub { slug, version }) => {
                    serde_json::json!({"type": "clawhub", "slug": slug, "version": version})
                }
                Some(librefang_skills::SkillSource::Skillhub { slug, version }) => {
                    serde_json::json!({"type": "skillhub", "slug": slug, "version": version})
                }
                Some(librefang_skills::SkillSource::OpenClaw) => {
                    serde_json::json!({"type": "openclaw"})
                }
                Some(librefang_skills::SkillSource::Local)
                | Some(librefang_skills::SkillSource::Native)
                | None => {
                    serde_json::json!({"type": "local"})
                }
            };
            serde_json::json!({
                "name": s.manifest.skill.name,
                "description": s.manifest.skill.description,
                "version": s.manifest.skill.version,
                "author": s.manifest.skill.author,
                "runtime": format!("{:?}", s.manifest.runtime.runtime_type),
                "tools_count": s.manifest.tools.provided.len(),
                "tags": s.manifest.skill.tags,
                "enabled": s.enabled,
                "source": source,
                "has_prompt_context": s.manifest.prompt_context.is_some(),
            })
        })
        .collect();

    Json(serde_json::json!({ "skills": skills, "total": skills.len() }))
}

/// POST /api/skills/install — Install a skill from FangHub (GitHub).
#[utoipa::path(
    post,
    path = "/api/skills/install",
    tag = "skills",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Install a skill from FangHub", body = serde_json::Value)
    )
)]
pub async fn install_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SkillInstallRequest>,
) -> impl IntoResponse {
    let home = state.kernel.home_dir();
    let skills_dir = if let Some(ref hand_id) = req.hand {
        let hand_dir = home.join("workspaces").join("hands").join(hand_id);
        if !hand_dir.exists() {
            return ApiErrorResponse::not_found(format!("Hand '{hand_id}' not found"))
                .into_json_tuple();
        }
        hand_dir.join("skills")
    } else {
        home.join("skills")
    };
    if let Err(e) = std::fs::create_dir_all(&skills_dir) {
        return ApiErrorResponse::internal(format!("Failed to create skills dir: {e}"))
            .into_json_tuple();
    }

    // Install from local registry (~/.librefang/registry/skills/{name}/)
    let registry_src = home.join("registry").join("skills").join(&req.name);
    if !registry_src.exists() {
        return ApiErrorResponse::not_found(format!(
            "Skill '{}' not found in local registry",
            req.name
        ))
        .into_json_tuple();
    }

    let dest = skills_dir.join(&req.name);
    if dest.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("Skill '{}' is already installed", req.name),
                "status": "already_installed",
            })),
        );
    }

    // Copy the skill directory from registry to skills
    match copy_dir_recursive(&registry_src, &dest) {
        Ok(()) => {
            // Read version from manifest
            let version = std::fs::read_to_string(dest.join("skill.toml"))
                .ok()
                .and_then(|s| toml::from_str::<librefang_skills::SkillManifest>(&s).ok())
                .map(|m| m.skill.version)
                .unwrap_or_else(|| "unknown".to_string());

            // Hot-reload so agents see the new skill immediately
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": req.name,
                    "version": version,
                    "hand": req.hand,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Skill install failed: {e}");
            // Clean up partial copy
            let _ = std::fs::remove_dir_all(&dest);
            ApiErrorResponse::internal(format!("Install failed: {e}")).into_json_tuple()
        }
    }
}

/// POST /api/skills/uninstall — Uninstall a skill.
#[utoipa::path(
    post,
    path = "/api/skills/uninstall",
    tag = "skills",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Uninstall a skill", body = serde_json::Value)
    )
)]
pub async fn uninstall_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SkillUninstallRequest>,
) -> impl IntoResponse {
    let skills_dir = state.kernel.home_dir().join("skills");
    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir);
    if let Err(e) = registry.load_all() {
        tracing::warn!("Failed to reload skill registry: {e}");
    }

    match registry.remove(&req.name) {
        Ok(()) => {
            // Hot-reload so agents stop seeing the removed skill
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "uninstalled", "name": req.name})),
            )
        }
        Err(e) => ApiErrorResponse::not_found(format!("{e}")).into_json_tuple(),
    }
}

/// POST /api/skills/reload — Rescan `~/.librefang/skills/` and refresh the
/// in-memory registry. Use this after dropping a skill directory into the
/// skills folder manually (install/uninstall via API already reload
/// automatically). Returns the new installed skill count.
#[utoipa::path(
    post,
    path = "/api/skills/reload",
    tag = "skills",
    responses(
        (status = 200, description = "Rescan the skills directory from disk", body = serde_json::Value)
    )
)]
pub async fn reload_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.kernel.reload_skills();
    let count = state
        .kernel
        .skill_registry_ref()
        .read()
        .map(|r| r.count())
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "reloaded", "count": count})),
    )
}

/// GET /api/skills/registry — List official skills from the local registry cache (~/.librefang/registry/skills).
#[utoipa::path(
    get,
    path = "/api/skills/registry",
    tag = "skills",
    responses(
        (status = 200, description = "Official skills available in the FangHub registry", body = serde_json::Value)
    )
)]
pub async fn list_skill_registry(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry_skills_dir = state.kernel.home_dir().join("registry").join("skills");

    if !registry_skills_dir.exists() {
        return Json(serde_json::json!({ "skills": [], "total": 0 }));
    }

    let mut skills: Vec<serde_json::Value> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&registry_skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                continue;
            }
            match std::fs::read_to_string(&manifest_path) {
                Ok(content) => {
                    if let Ok(manifest) =
                        toml::from_str::<librefang_skills::SkillManifest>(&content)
                    {
                        let name = manifest.skill.name.clone();
                        let installed_dir = state.kernel.home_dir().join("skills").join(&name);
                        let is_installed = installed_dir.exists();
                        skills.push(serde_json::json!({
                            "name": name,
                            "description": manifest.skill.description,
                            "version": manifest.skill.version,
                            "author": manifest.skill.author,
                            "tags": manifest.skill.tags,
                            "is_installed": is_installed,
                        }));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to read registry skill manifest {:?}: {e}",
                        manifest_path
                    );
                }
            }
        }
    }

    let total = skills.len();
    Json(serde_json::json!({ "skills": skills, "total": total }))
}

/// GET /api/marketplace/search — Search the FangHub marketplace.
#[utoipa::path(
    get,
    path = "/api/marketplace/search",
    tag = "skills",
    params(
        ("q" = Option<String>, Query, description = "Search query"),
    ),
    responses(
        (status = 200, description = "Search the FangHub marketplace", body = serde_json::Value)
    )
)]
pub async fn marketplace_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default().to_lowercase();
    let registry_dir = state.kernel.home_dir().join("registry").join("skills");

    let mut results: Vec<serde_json::Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&registry_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                if let Ok(manifest) = toml::from_str::<librefang_skills::SkillManifest>(&content) {
                    let name = &manifest.skill.name;
                    let desc = &manifest.skill.description;
                    if query.is_empty()
                        || name.to_lowercase().contains(&query)
                        || desc.to_lowercase().contains(&query)
                    {
                        results.push(serde_json::json!({
                            "name": name,
                            "description": desc,
                            "stars": 0,
                            "url": "",
                        }));
                    }
                }
            }
        }
    }

    let total = results.len();
    Json(serde_json::json!({"results": results, "total": total}))
}

// ---------------------------------------------------------------------------
// ClawHub (OpenClaw ecosystem) endpoints
// ---------------------------------------------------------------------------

/// GET /api/clawhub/search — Search ClawHub skills using vector/semantic search.
///
/// Query parameters:
/// - `q` — search query (required)
/// - `limit` — max results (default: 20, max: 50)
#[utoipa::path(
    get,
    path = "/api/clawhub/search",
    tag = "skills",
    params(
        ("q" = Option<String>, Query, description = "Search query"),
    ),
    responses(
        (status = 200, description = "Search ClawHub skills", body = serde_json::Value)
    )
)]
pub async fn clawhub_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"items": [], "next_cursor": null})),
        );
    }

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    // Check cache (120s TTL)
    let cache_key = format!("search:{}:{}", query, limit);
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub");
    let client = librefang_skills::clawhub::ClawHubClient::new(cache_dir);

    match client.search(&query, limit).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .results
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "slug": e.slug,
                        "name": e.display_name,
                        "description": e.summary,
                        "version": e.version,
                        "score": e.score,
                        "updated_at": e.updated_at,
                    })
                })
                .collect();
            let resp = serde_json::json!({
                "items": items,
                "next_cursor": null,
            });
            state
                .clawhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("ClawHub search failed: {msg}");
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::BAD_GATEWAY
            };
            (
                status,
                Json(serde_json::json!({"items": [], "next_cursor": null, "error": msg})),
            )
        }
    }
}

/// GET /api/clawhub/browse — Browse ClawHub skills by sort order.
///
/// Query parameters:
/// - `sort` — sort order: "trending", "downloads", "stars", "updated", "rating" (default: "trending")
/// - `limit` — max results (default: 20, max: 50)
/// - `cursor` — pagination cursor from previous response
#[utoipa::path(
    get,
    path = "/api/clawhub/browse",
    tag = "skills",
    params(
        ("q" = Option<String>, Query, description = "Search query"),
    ),
    responses(
        (status = 200, description = "Browse ClawHub skills by sort order", body = serde_json::Value)
    )
)]
pub async fn clawhub_browse(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sort = match params.get("sort").map(|s| s.as_str()) {
        Some("downloads") => librefang_skills::clawhub::ClawHubSort::Downloads,
        Some("stars") => librefang_skills::clawhub::ClawHubSort::Stars,
        Some("updated") => librefang_skills::clawhub::ClawHubSort::Updated,
        Some("rating") => librefang_skills::clawhub::ClawHubSort::Rating,
        _ => librefang_skills::clawhub::ClawHubSort::Trending,
    };

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let cursor = params.get("cursor").map(|s| s.as_str());

    // Check cache (120s TTL)
    let cache_key = format!("browse:{:?}:{}:{}", sort, limit, cursor.unwrap_or(""));
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub");
    let client = librefang_skills::clawhub::ClawHubClient::new(cache_dir);

    match client.browse(sort, limit, cursor).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .items
                .iter()
                .map(clawhub_browse_entry_to_json)
                .collect();
            let resp = serde_json::json!({
                "items": items,
                "next_cursor": results.next_cursor,
            });
            state
                .clawhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("ClawHub browse failed: {msg}");
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::BAD_GATEWAY
            };
            (
                status,
                Json(serde_json::json!({"items": [], "next_cursor": null, "error": msg})),
            )
        }
    }
}

/// GET /api/clawhub/skill/{slug} — Get detailed info about a ClawHub skill.
#[utoipa::path(
    get,
    path = "/api/clawhub/skill/{slug}",
    tag = "skills",
    params(
        ("slug" = String, Path, description = "Skill slug"),
    ),
    responses(
        (status = 200, description = "Get detailed info about a ClawHub skill", body = serde_json::Value)
    )
)]
pub async fn clawhub_skill_detail(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub");
    let client = librefang_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.home_dir().join("skills");
    let is_installed = client.is_installed(&slug, &skills_dir);

    match client.get_skill(&slug).await {
        Ok(detail) => {
            let version = detail
                .latest_version
                .as_ref()
                .map(|v| v.version.as_str())
                .unwrap_or("");
            let author = detail
                .owner
                .as_ref()
                .map(|o| o.handle.as_str())
                .unwrap_or("");
            let author_name = detail
                .owner
                .as_ref()
                .map(|o| o.display_name.as_str())
                .unwrap_or("");
            let author_image = detail
                .owner
                .as_ref()
                .and_then(|o| o.image.as_deref())
                .unwrap_or("");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "slug": detail.skill.slug,
                    "name": detail.skill.display_name,
                    "description": detail.skill.summary,
                    "version": version,
                    "downloads": detail.skill.stats.downloads,
                    "stars": detail.skill.stats.stars,
                    "author": author,
                    "author_name": author_name,
                    "author_image": author_image,
                    "tags": detail.skill.tags,
                    "updated_at": detail.skill.updated_at,
                    "created_at": detail.skill.created_at,
                    "is_installed": is_installed,
                    "installed": is_installed,
                })),
            )
        }
        Err(e) => {
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::NOT_FOUND
            };
            (status, Json(serde_json::json!({"error": format!("{e}")})))
        }
    }
}

/// GET /api/clawhub/skill/{slug}/code — Fetch the source code (SKILL.md) of a ClawHub skill.
#[utoipa::path(
    get,
    path = "/api/clawhub/skill/{slug}/code",
    tag = "skills",
    params(
        ("slug" = String, Path, description = "Skill slug"),
    ),
    responses(
        (status = 200, description = "Fetch source code of a ClawHub skill", body = serde_json::Value)
    )
)]
pub async fn clawhub_skill_code(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub");
    let client = librefang_skills::clawhub::ClawHubClient::new(cache_dir);

    // Try to fetch SKILL.md first, then fallback to package.json
    let mut code = String::new();
    let mut filename = String::new();

    if let Ok(content) = client.get_file(&slug, "SKILL.md").await {
        code = content;
        filename = "SKILL.md".to_string();
    } else if let Ok(content) = client.get_file(&slug, "package.json").await {
        code = content;
        filename = "package.json".to_string();
    } else if let Ok(content) = client.get_file(&slug, "skill.toml").await {
        code = content;
        filename = "skill.toml".to_string();
    }

    if code.is_empty() {
        return ApiErrorResponse::not_found("No source code found for this skill")
            .into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "slug": slug,
            "filename": filename,
            "code": code,
        })),
    )
}

/// POST /api/clawhub/install — Install a skill from ClawHub.
///
/// Runs the full security pipeline: SHA256 verification, format detection,
/// manifest security scan, prompt injection scan, and binary dependency check.
#[utoipa::path(
    post,
    path = "/api/clawhub/install",
    tag = "skills",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Install a skill from ClawHub", body = serde_json::Value)
    )
)]
pub async fn clawhub_install(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::types::ClawHubInstallRequest>,
) -> impl IntoResponse {
    let home = state.kernel.home_dir();
    let skills_dir = if let Some(ref hand_id) = req.hand {
        let hand_dir = home.join("workspaces").join("hands").join(hand_id);
        if !hand_dir.exists() {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Hand '{hand_id}' not found")})),
            );
        }
        let dir = hand_dir.join("skills");
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        home.join("skills")
    };
    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub");
    let client = librefang_skills::clawhub::ClawHubClient::new(cache_dir);

    // Check if already installed
    if client.is_installed(&req.slug, &skills_dir) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("Skill '{}' is already installed", req.slug),
                "status": "already_installed",
            })),
        );
    }

    match client.install(&req.slug, &skills_dir).await {
        Ok(result) => {
            let warnings: Vec<serde_json::Value> = result
                .warnings
                .iter()
                .map(|w| {
                    serde_json::json!({
                        "severity": format!("{:?}", w.severity),
                        "message": w.message,
                    })
                })
                .collect();

            let translations: Vec<serde_json::Value> = result
                .tool_translations
                .iter()
                .map(|(from, to)| serde_json::json!({"from": from, "to": to}))
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": result.skill_name,
                    "version": result.version,
                    "slug": result.slug,
                    "is_prompt_only": result.is_prompt_only,
                    "warnings": warnings,
                    "tool_translations": translations,
                })),
            )
        }
        Err(e) => {
            let msg = format!("{e}");
            let status = if matches!(e, librefang_skills::SkillError::SecurityBlocked(_)) {
                StatusCode::FORBIDDEN
            } else if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else if matches!(e, librefang_skills::SkillError::Network(_)) {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            tracing::warn!("ClawHub install failed: {msg}");
            (status, Json(serde_json::json!({"error": msg})))
        }
    }
}

// ---------------------------------------------------------------------------
// Skillhub marketplace endpoints
// ---------------------------------------------------------------------------

/// GET /api/skillhub/search — Search Skillhub skills.
pub async fn skillhub_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"items": [], "next_cursor": null})),
        );
    }

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    // Check cache (120s TTL)
    let cache_key = format!("sh_search:{}:{}", query, limit);
    if let Some(entry) = state.skillhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.home_dir().join(".cache").join("skillhub");
    let client = librefang_skills::skillhub::SkillhubClient::with_defaults(cache_dir);

    match client.search(&query, limit).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .results
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "slug": e.slug,
                        "name": e.display_name,
                        "description": e.summary,
                        "version": e.version,
                        "score": e.score,
                        "updated_at": e.updated_at,
                    })
                })
                .collect();
            let resp = serde_json::json!({
                "items": items,
                "next_cursor": null,
            });
            state
                .skillhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("Skillhub search failed: {msg}");
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::BAD_GATEWAY
            };
            (
                status,
                Json(serde_json::json!({"items": [], "next_cursor": null, "error": msg})),
            )
        }
    }
}

/// GET /api/skillhub/browse — Browse Skillhub skills from the static index.
pub async fn skillhub_browse(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sort = params.get("sort").map(|s| s.as_str()).unwrap_or("trending");

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    // Check cache (300s TTL)
    let cache_key = format!("sh_browse:{}:{}", sort, limit);
    if let Some(entry) = state.skillhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 300 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.home_dir().join(".cache").join("skillhub");
    let client = librefang_skills::skillhub::SkillhubClient::with_defaults(cache_dir);

    match client.browse(sort, limit).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .skills
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "slug": e.slug,
                        "name": e.name,
                        "description": e.description,
                        "version": e.version,
                        "downloads": e.downloads,
                        "stars": e.stars,
                        "categories": e.categories,
                    })
                })
                .collect();
            let resp = serde_json::json!({
                "items": items,
            });
            state
                .skillhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("Skillhub browse failed: {msg}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"items": [], "error": msg})),
            )
        }
    }
}

/// GET /api/skillhub/skill/{slug} — Get detailed info about a Skillhub skill.
pub async fn skillhub_skill_detail(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state
        .kernel
        .config_ref()
        .home_dir
        .join(".cache")
        .join("skillhub");
    let client = librefang_skills::skillhub::SkillhubClient::with_defaults(cache_dir);

    let skills_dir = state.kernel.home_dir().join("skills");
    let is_installed = client.is_installed(&slug, &skills_dir);

    match client.get_skill(&slug).await {
        Ok(detail) => {
            let version = detail
                .latest_version
                .as_ref()
                .map(|v| v.version.as_str())
                .unwrap_or("");
            let author = detail
                .owner
                .as_ref()
                .map(|o| o.handle.as_str())
                .unwrap_or("");
            let author_name = detail
                .owner
                .as_ref()
                .map(|o| o.display_name.as_str())
                .unwrap_or("");
            let author_image = detail
                .owner
                .as_ref()
                .and_then(|o| o.image.as_deref())
                .unwrap_or("");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "slug": detail.skill.slug,
                    "name": detail.skill.display_name,
                    "description": detail.skill.summary,
                    "version": version,
                    "downloads": std::cmp::max(detail.skill.stats.downloads, detail.skill.stats.installs),
                    "stars": detail.skill.stats.stars,
                    "author": author,
                    "author_name": author_name,
                    "author_image": author_image,
                    "tags": detail.skill.tags,
                    "updated_at": detail.skill.updated_at,
                    "created_at": detail.skill.created_at,
                    "is_installed": is_installed,
                    "installed": is_installed,
                    "source": "skillhub",
                })),
            )
        }
        Err(e) => {
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::NOT_FOUND
            };
            (status, Json(serde_json::json!({"error": format!("{e}")})))
        }
    }
}

/// GET /api/skillhub/skill/{slug}/code — Source code viewing is not available for Skillhub skills.
pub async fn skillhub_skill_code(Path(_slug): Path<String>) -> impl IntoResponse {
    ApiErrorResponse::not_found("Source code viewing is not available for Skillhub skills")
        .into_json_tuple()
}

/// POST /api/skillhub/install — Install a skill from Skillhub.
pub async fn skillhub_install(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::types::ClawHubInstallRequest>,
) -> impl IntoResponse {
    let home = state.kernel.home_dir();
    let skills_dir = if let Some(ref hand_id) = req.hand {
        let hand_dir = home.join("workspaces").join("hands").join(hand_id);
        if !hand_dir.exists() {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Hand '{hand_id}' not found")})),
            );
        }
        let dir = hand_dir.join("skills");
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        home.join("skills")
    };
    let cache_dir = state
        .kernel
        .config_ref()
        .home_dir
        .join(".cache")
        .join("skillhub");
    let client = librefang_skills::skillhub::SkillhubClient::with_defaults(cache_dir);

    // Check if already installed
    if client.is_installed(&req.slug, &skills_dir) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("Skill '{}' is already installed", req.slug),
                "status": "already_installed",
            })),
        );
    }

    match client.install(&req.slug, &skills_dir).await {
        Ok(result) => {
            let warnings: Vec<serde_json::Value> = result
                .warnings
                .iter()
                .map(|w| {
                    serde_json::json!({
                        "severity": format!("{:?}", w.severity),
                        "message": w.message,
                    })
                })
                .collect();

            let translations: Vec<serde_json::Value> = result
                .tool_translations
                .iter()
                .map(|(from, to)| serde_json::json!({"from": from, "to": to}))
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": result.skill_name,
                    "version": result.version,
                    "slug": result.slug,
                    "is_prompt_only": result.is_prompt_only,
                    "warnings": warnings,
                    "tool_translations": translations,
                })),
            )
        }
        Err(e) => {
            let msg = format!("{e}");
            let status = if matches!(e, librefang_skills::SkillError::SecurityBlocked(_)) {
                StatusCode::FORBIDDEN
            } else if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else if matches!(e, librefang_skills::SkillError::Network(_)) {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            tracing::warn!("Skillhub install failed: {msg}");
            (status, Json(serde_json::json!({"error": msg})))
        }
    }
}

/// Check whether a SkillError represents a ClawHub rate-limit (429).
fn is_clawhub_rate_limit(err: &librefang_skills::SkillError) -> bool {
    matches!(err, librefang_skills::SkillError::RateLimited(_))
}

/// Convert a browse entry (nested stats/tags) to a flat JSON object for the frontend.
fn clawhub_browse_entry_to_json(
    entry: &librefang_skills::clawhub::ClawHubBrowseEntry,
) -> serde_json::Value {
    let version = librefang_skills::clawhub::ClawHubClient::entry_version(entry);
    serde_json::json!({
        "slug": entry.slug,
        "name": entry.display_name,
        "description": entry.summary,
        "version": version,
        "downloads": entry.stats.downloads,
        "stars": entry.stats.stars,
        "updated_at": entry.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Hands endpoints
// ---------------------------------------------------------------------------

/// Detect the server platform for install command selection.
fn server_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// GET /api/hands — List all hand definitions (marketplace).
#[utoipa::path(
    get,
    path = "/api/hands",
    tag = "hands",
    responses(
        (status = 200, description = "List all hand definitions", body = serde_json::Value)
    )
)]
pub async fn list_hands(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let lang = headers
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(&[',', ';', '-'][..]).next())
        .unwrap_or("en");

    let defs = state.kernel.hands().list_definitions();
    let home_dir = state.kernel.home_dir().to_path_buf();
    let hands: Vec<serde_json::Value> = defs
        .iter()
        .map(|d| {
            let reqs = state
                .kernel
                .hands()
                .check_requirements(&d.id)
                .unwrap_or_default();
            let readiness = state.kernel.hands().readiness(&d.id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);

            // A hand is user-installed (uninstallable) if its HAND.toml lives
            // in `home/workspaces/{id}/`. Built-ins synced from the registry
            // live under `home/registry/hands/{id}/` and are recreated on
            // every sync, so the UI should not offer to uninstall them.
            let is_custom = home_dir
                .join("workspaces")
                .join(&d.id)
                .join("HAND.toml")
                .exists();

            let i18n_entry = d.i18n.get(lang);
            let resolved_name = i18n_entry
                .and_then(|l| l.name.as_deref())
                .unwrap_or(&d.name);
            let resolved_desc = i18n_entry
                .and_then(|l| l.description.as_deref())
                .unwrap_or(&d.description);

            serde_json::json!({
                "id": d.id,
                "name": resolved_name,
                "description": resolved_desc,
                "category": d.category,
                "icon": d.icon,
                "tools": d.tools,
                "requirements_met": requirements_met,
                "active": active,
                "degraded": degraded,
                "is_custom": is_custom,
                "requirements": reqs.iter().map(|(r, ok)| {
                    let mut req = serde_json::json!({
                        "key": r.check_value,
                        "label": r.label,
                        "satisfied": ok,
                        "optional": r.optional,
                    });
                    if *ok {
                        if let Ok(val) = std::env::var(&r.check_value) {
                            req["current_value"] = serde_json::json!(val);
                        }
                    }
                    req
                }).collect::<Vec<_>>(),
                "dashboard_metrics": d.dashboard.metrics.len(),
                "has_settings": !d.settings.is_empty(),
                "settings_count": d.settings.len(),
                "metadata": d.metadata.clone().unwrap_or_default(),
                "i18n": d.i18n,
            })
        })
        .collect();

    Json(serde_json::json!({ "hands": hands, "total": hands.len() }))
}

/// GET /api/hands/active — List active hand instances.
#[utoipa::path(
    get,
    path = "/api/hands/active",
    tag = "hands",
    responses(
        (status = 200, description = "List active hand instances", body = serde_json::Value)
    )
)]
pub async fn list_active_hands(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Split on `,`/`;` to isolate the primary tag, then try the full tag
    // ("zh-CN") before falling back to the base ("zh") so hand i18n maps with
    // region codes resolve correctly instead of silently dropping to the
    // default name.
    let primary = headers
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(&[',', ';'][..]).next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "en".to_string());
    let base = primary.split('-').next().unwrap_or("en").to_string();

    let instances = state.kernel.hands().list_instances();
    let items: Vec<serde_json::Value> = instances
        .iter()
        .map(|i| {
            let def = state.kernel.hands().get_definition(&i.hand_id);
            let hand_name = def.as_ref().map(|d| {
                d.i18n
                    .get(&primary)
                    .or_else(|| d.i18n.get(&base))
                    .and_then(|l| l.name.as_deref())
                    .unwrap_or(&d.name)
                    .to_string()
            });
            let hand_icon = def.as_ref().map(|d| d.icon.clone());

            let agent_ids: std::collections::BTreeMap<String, String> = i
                .agent_ids
                .iter()
                .map(|(role, id)| (role.clone(), id.to_string()))
                .collect();

            serde_json::json!({
                "instance_id": i.instance_id,
                "hand_id": i.hand_id,
                "hand_name": hand_name,
                "hand_icon": hand_icon,
                "status": format!("{}", i.status),
                "agent_id": i.agent_id().map(|a: librefang_types::agent::AgentId| a.to_string()),
                "agent_name": i.agent_name(),
                "agent_ids": agent_ids,
                "coordinator_role": i.coordinator_role(),
                "activated_at": i.activated_at.to_rfc3339(),
                "updated_at": i.updated_at.to_rfc3339(),
            })
        })
        .collect();

    Json(serde_json::json!({ "instances": items, "total": items.len() }))
}

/// GET /api/hands/{hand_id} — Get a single hand definition with requirements check.
#[utoipa::path(
    get,
    path = "/api/hands/{hand_id}",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    responses(
        (status = 200, description = "Get a single hand definition with requirements", body = serde_json::Value)
    )
)]
pub async fn get_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    match state.kernel.hands().get_definition(&hand_id) {
        Some(def) => {
            let lang = headers
                .get("accept-language")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(&[',', ';', '-'][..]).next())
                .unwrap_or("en");

            let i18n_entry = def.i18n.get(lang);
            let resolved_name = i18n_entry
                .and_then(|l| l.name.as_deref())
                .unwrap_or(&def.name);
            let resolved_desc = i18n_entry
                .and_then(|l| l.description.as_deref())
                .unwrap_or(&def.description);

            let reqs = state
                .kernel
                .hands()
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hands().readiness(&hand_id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            let settings_status = state
                .kernel
                .hands()
                .check_settings_availability(&hand_id, Some(lang))
                .unwrap_or_default();
            let dm = state.kernel.config_ref().default_model.clone();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": def.id,
                    "name": resolved_name,
                    "description": resolved_desc,
                    "category": def.category,
                    "icon": def.icon,
                    "tools": def.tools,
                    "requirements_met": requirements_met,
                    "active": active,
                    "degraded": degraded,
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                            "optional": r.optional,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                    "server_platform": server_platform(),
                    "agent": if let Some(agent_manifest) = def.agent() {
                        serde_json::json!({
                            "name": agent_manifest.name,
                            "description": agent_manifest.description,
                            "provider": if agent_manifest.model.provider == "default" {
                                &dm.provider
                            } else { &agent_manifest.model.provider },
                            "model": if agent_manifest.model.model == "default" {
                                &dm.model
                            } else { &agent_manifest.model.model },
                        })
                    } else {
                        serde_json::json!(null)
                    },
                    "agents": def.agents.iter().map(|(role, a)| {
                        let dm = &dm;
                        let agent_i18n = i18n_entry.and_then(|l| l.agents.get(role.as_str()));
                        let resolved_agent_name = agent_i18n
                            .and_then(|ai| ai.name.as_deref())
                            .unwrap_or(&a.manifest.name);
                        let resolved_agent_desc = agent_i18n
                            .and_then(|ai| ai.description.as_deref())
                            .unwrap_or(&a.manifest.description);
                        // Extract Phase/Step headings from system_prompt
                        let steps: Vec<&str> = a.manifest.model.system_prompt
                            .lines()
                            .filter(|line| {
                                let trimmed = line.trim();
                                trimmed.starts_with("### Phase")
                                    || trimmed.starts_with("### Step")
                                    || trimmed.starts_with("## Phase")
                                    || trimmed.starts_with("## Step")
                            })
                            .map(|line| line.trim().trim_start_matches('#').trim())
                            .collect();
                        serde_json::json!({
                            "role": role,
                            "name": resolved_agent_name,
                            "description": resolved_agent_desc,
                            "coordinator": a.coordinator,
                            "provider": if a.manifest.model.provider == "default" { &dm.provider } else { &a.manifest.model.provider },
                            "model": if a.manifest.model.model == "default" { &dm.model } else { &a.manifest.model.model },
                            "steps": steps,
                        })
                    }).collect::<Vec<_>>(),
                    "dashboard": def.dashboard.metrics.iter().map(|m| serde_json::json!({
                        "label": m.label,
                        "memory_key": m.memory_key,
                        "format": m.format,
                    })).collect::<Vec<_>>(),
                    "settings": settings_status,
                    "metadata": def.metadata.clone().unwrap_or_default(),
                    "i18n": def.i18n,
                })),
            )
        }
        None => ApiErrorResponse::not_found(format!("Hand not found: {hand_id}")).into_json_tuple(),
    }
}

/// POST /api/hands/{hand_id}/check-deps — Re-check dependency status for a hand.
#[utoipa::path(
    post,
    path = "/api/hands/{hand_id}/check-deps",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    responses(
        (status = 200, description = "Re-check dependency status for a hand", body = serde_json::Value)
    )
)]
pub async fn check_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.hands().get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hands()
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hands().readiness(&hand_id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "hand_id": def.id,
                    "requirements_met": requirements_met,
                    "active": active,
                    "degraded": degraded,
                    "server_platform": server_platform(),
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                            "optional": r.optional,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                })),
            )
        }
        None => ApiErrorResponse::not_found(format!("Hand not found: {hand_id}")).into_json_tuple(),
    }
}

/// POST /api/hands/{hand_id}/install-deps — Auto-install missing dependencies for a hand.
#[utoipa::path(
    post,
    path = "/api/hands/{hand_id}/install-deps",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    responses(
        (status = 200, description = "Auto-install missing dependencies for a hand", body = serde_json::Value)
    )
)]
pub async fn install_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    let def = match state.kernel.hands().get_definition(&hand_id) {
        Some(d) => d.clone(),
        None => {
            return ApiErrorResponse::not_found(format!("Hand not found: {hand_id}"))
                .into_json_tuple();
        }
    };

    let reqs = state
        .kernel
        .hands()
        .check_requirements(&hand_id)
        .unwrap_or_default();

    let platform = server_platform();
    let mut results = Vec::new();

    for (req, already_satisfied) in &reqs {
        if *already_satisfied {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "already_installed",
                "message": format!("{} is already available", req.label),
            }));
            continue;
        }

        let install = match &req.install {
            Some(i) => i,
            None => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "skipped",
                    "message": "No install instructions available",
                }));
                continue;
            }
        };

        // Pick the best install command for this platform
        let cmd = match platform {
            "windows" => install.windows.as_deref().or(install.pip.as_deref()),
            "macos" => install.macos.as_deref().or(install.pip.as_deref()),
            _ => install
                .linux_apt
                .as_deref()
                .or(install.linux_dnf.as_deref())
                .or(install.linux_pacman.as_deref())
                .or(install.pip.as_deref()),
        };

        let cmd = match cmd {
            Some(c) => c,
            None => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "no_command",
                    "message": format!("No install command for platform: {platform}"),
                }));
                continue;
            }
        };

        // Execute the install command
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        // For winget on Windows, add --accept flags to avoid interactive prompts
        let final_cmd = if cfg!(windows) && cmd.starts_with("winget ") {
            format!("{cmd} --accept-source-agreements --accept-package-agreements")
        } else {
            cmd.to_string()
        };

        tracing::info!(hand = %hand_id, dep = %req.key, cmd = %final_cmd, "Auto-installing dependency");

        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tokio::process::Command::new(shell)
                .arg(flag)
                .arg(&final_cmd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null())
                .output(),
        )
        .await
        {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "error",
                    "command": final_cmd,
                    "message": format!("Failed to execute: {e}"),
                }));
                continue;
            }
            Err(_) => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "timeout",
                    "command": final_cmd,
                    "message": "Installation timed out after 5 minutes",
                }));
                continue;
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if exit_code == 0 {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "installed",
                "command": final_cmd,
                "message": format!("{} installed successfully", req.label),
            }));
        } else {
            // On Windows, winget may return non-zero even on success (e.g., already installed)
            let combined = format!("{stdout}{stderr}");
            let likely_ok = combined.contains("already installed")
                || combined.contains("No applicable update")
                || combined.contains("No available upgrade");
            results.push(serde_json::json!({
                "key": req.key,
                "status": if likely_ok { "installed" } else { "error" },
                "command": final_cmd,
                "exit_code": exit_code,
                "message": if likely_ok {
                    format!("{} is already installed", req.label)
                } else {
                    let msg = stderr.chars().take(500).collect::<String>();
                    format!("Install failed (exit {}): {}", exit_code, msg.trim())
                },
            }));
        }
    }

    // On Windows, refresh PATH to pick up newly installed binaries from winget/pip
    #[cfg(windows)]
    {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        if !home.is_empty() {
            let winget_pkgs =
                std::path::Path::new(&home).join("AppData\\Local\\Microsoft\\WinGet\\Packages");
            if winget_pkgs.is_dir() {
                let mut extra_paths = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&winget_pkgs) {
                    for entry in entries.flatten() {
                        let pkg_dir = entry.path();
                        // Look for bin/ subdirectory (ffmpeg style)
                        if let Ok(sub_entries) = std::fs::read_dir(&pkg_dir) {
                            for sub in sub_entries.flatten() {
                                let bin_dir = sub.path().join("bin");
                                if bin_dir.is_dir() {
                                    extra_paths.push(bin_dir.to_string_lossy().to_string());
                                }
                            }
                        }
                        // Direct exe in package dir (yt-dlp style)
                        if std::fs::read_dir(&pkg_dir)
                            .map(|rd| {
                                rd.flatten().any(|e| {
                                    e.path().extension().map(|x| x == "exe").unwrap_or(false)
                                })
                            })
                            .unwrap_or(false)
                        {
                            extra_paths.push(pkg_dir.to_string_lossy().to_string());
                        }
                    }
                }
                // Also add pip Scripts dir
                let pip_scripts =
                    std::path::Path::new(&home).join("AppData\\Local\\Programs\\Python");
                if pip_scripts.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&pip_scripts) {
                        for entry in entries.flatten() {
                            let scripts = entry.path().join("Scripts");
                            if scripts.is_dir() {
                                extra_paths.push(scripts.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                if !extra_paths.is_empty() {
                    let current_path = std::env::var("PATH").unwrap_or_default();
                    let new_path = format!("{};{}", extra_paths.join(";"), current_path);
                    std::env::set_var("PATH", &new_path);
                    tracing::info!(
                        added = extra_paths.len(),
                        "Refreshed PATH with winget/pip directories"
                    );
                }
            }
        }
    }

    // Re-check requirements after installation
    let reqs_after = state
        .kernel
        .hands()
        .check_requirements(&hand_id)
        .unwrap_or_default();
    let all_satisfied = reqs_after.iter().all(|(_, ok)| *ok);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hand_id": def.id,
            "results": results,
            "requirements_met": all_satisfied,
            "requirements": reqs_after.iter().map(|(r, ok)| {
                serde_json::json!({
                    "key": r.key,
                    "label": r.label,
                    "satisfied": ok,
                })
            }).collect::<Vec<_>>(),
        })),
    )
}

/// DELETE /api/hands/{hand_id} — Uninstall a user-installed hand.
///
/// Only hands that live under `home_dir/workspaces/{id}/` can be removed.
/// Built-in hands (shipped by librefang-registry under `home_dir/registry/hands/`)
/// cannot be uninstalled because the next registry sync would recreate them.
/// Hands with live instances must be deactivated first.
#[utoipa::path(
    delete,
    path = "/api/hands/{hand_id}",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    responses(
        (status = 200, description = "Hand uninstalled", body = serde_json::Value),
        (status = 404, description = "Hand not found or is a built-in"),
        (status = 409, description = "Hand is still active — deactivate first"),
    )
)]
pub async fn uninstall_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    let home_dir = state.kernel.home_dir().to_path_buf();
    match state.kernel.hands().uninstall_hand(&home_dir, &hand_id) {
        Ok(()) => {
            librefang_kernel::router::invalidate_hand_route_cache();
            state.kernel.persist_hand_state();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "hand_id": hand_id,
                })),
            )
        }
        Err(librefang_hands::HandError::NotFound(id)) => {
            ApiErrorResponse::not_found(format!("Hand not found: {id}")).into_json_tuple()
        }
        Err(librefang_hands::HandError::BuiltinHand(id)) => ApiErrorResponse::not_found(format!(
            "Hand '{id}' is a built-in and cannot be uninstalled"
        ))
        .into_json_tuple(),
        Err(librefang_hands::HandError::AlreadyActive(msg)) => {
            ApiErrorResponse::conflict(msg).into_json_tuple()
        }
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}

/// POST /api/hands/install — Install a hand from TOML content.
#[utoipa::path(
    post,
    path = "/api/hands/install",
    tag = "hands",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Install a hand from TOML content", body = serde_json::Value)
    )
)]
pub async fn install_hand(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let toml_content = body["toml_content"].as_str().unwrap_or("");
    let skill_content = body["skill_content"].as_str().unwrap_or("");

    if toml_content.is_empty() {
        return ApiErrorResponse::bad_request("Missing toml_content field").into_json_tuple();
    }

    match state.kernel.hands().install_from_content_persisted(
        state.kernel.home_dir(),
        toml_content,
        skill_content,
    ) {
        Ok(def) => {
            librefang_kernel::router::invalidate_hand_route_cache();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": def.id,
                    "name": def.name,
                    "description": def.description,
                    "category": format!("{:?}", def.category),
                })),
            )
        }
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}

/// POST /api/hands/{hand_id}/activate — Activate a hand (spawns agent).
#[utoipa::path(
    post,
    path = "/api/hands/{hand_id}/activate",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Activate a hand (spawns agent)", body = serde_json::Value)
    )
)]
pub async fn activate_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    body: Option<Json<librefang_hands::ActivateHandRequest>>,
) -> impl IntoResponse {
    let config = body.map(|b| b.0.config).unwrap_or_default();

    match state.kernel.activate_hand(&hand_id, config) {
        Ok(instance) => {
            // If the hand agent has a non-reactive schedule (autonomous hands),
            // start its background loop so it begins running immediately.
            if let Some(agent_id) = instance.agent_id() {
                let entry = state
                    .kernel
                    .agent_registry()
                    .list()
                    .into_iter()
                    .find(|e| e.id == agent_id);
                if let Some(entry) = entry {
                    if !matches!(
                        entry.manifest.schedule,
                        librefang_types::agent::ScheduleMode::Reactive
                    ) {
                        state.kernel.start_background_for_agent(
                            agent_id,
                            &entry.name,
                            &entry.manifest.schedule,
                        );
                    }
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "instance_id": instance.instance_id,
                    "hand_id": instance.hand_id,
                    "status": format!("{}", instance.status),
                    "agent_id": instance.agent_id().map(|a: librefang_types::agent::AgentId| a.to_string()),
                    "agent_name": instance.agent_name(),
                    "activated_at": instance.activated_at.to_rfc3339(),
                })),
            )
        }
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}

/// POST /api/hands/instances/{id}/pause — Pause a hand instance.
#[utoipa::path(
    post,
    path = "/api/hands/instances/{id}/pause",
    tag = "hands",
    params(
        ("id" = String, Path, description = "Instance ID"),
    ),
    responses(
        (status = 200, description = "Pause a hand instance", body = serde_json::Value)
    )
)]
pub async fn pause_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.pause_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "paused", "instance_id": id})),
        ),
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}

/// POST /api/hands/instances/{id}/resume — Resume a paused hand instance.
#[utoipa::path(
    post,
    path = "/api/hands/instances/{id}/resume",
    tag = "hands",
    params(
        ("id" = String, Path, description = "Instance ID"),
    ),
    responses(
        (status = 200, description = "Resume a paused hand instance", body = serde_json::Value)
    )
)]
pub async fn resume_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.resume_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "resumed", "instance_id": id})),
        ),
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}

/// DELETE /api/hands/instances/{id} — Deactivate a hand (kills agent).
#[utoipa::path(
    delete,
    path = "/api/hands/instances/{id}",
    tag = "hands",
    params(
        ("id" = String, Path, description = "Instance ID"),
    ),
    responses(
        (status = 200, description = "Deactivate a hand (kills agent)", body = serde_json::Value)
    )
)]
pub async fn deactivate_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.deactivate_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deactivated", "instance_id": id})),
        ),
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}

/// POST /api/hands/{hand_id}/secret — Set an environment variable (secret) for a hand requirement.
#[utoipa::path(
    post,
    path = "/api/hands/{hand_id}/secret",
    tag = "hands",
    params(("hand_id" = String, Path, description = "Hand ID")),
    request_body = serde_json::Value,
    responses((status = 200, description = "Secret saved", body = serde_json::Value))
)]
pub async fn set_hand_secret(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let env_key = match body["key"].as_str() {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing 'key' field (env var name)")
                .into_json_tuple();
        }
    };
    let value = match body["value"].as_str() {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing or empty 'value' field")
                .into_json_tuple();
        }
    };

    // Verify this key belongs to a requirement of the specified hand
    let valid = {
        let defs = state.kernel.hands().list_definitions();
        defs.iter()
            .find(|d| d.id == hand_id)
            .map(|def| {
                def.requires
                    .iter()
                    .any(|r| r.check_value == env_key || r.key == env_key)
            })
            .unwrap_or(false)
    };

    if !valid {
        return ApiErrorResponse::bad_request(format!(
            "'{}' is not a requirement of hand '{}'",
            env_key, hand_id
        ))
        .into_json_tuple();
    }

    // Write to secrets.env
    let secrets_path = state.kernel.home_dir().join("secrets.env");
    if let Err(e) = write_secret_env(&secrets_path, &env_key, &value) {
        return ApiErrorResponse::internal(format!("Failed to write secret: {e}"))
            .into_json_tuple();
    }

    // Set in current process
    // SAFETY: single-threaded secret writes during user-initiated config
    unsafe {
        std::env::set_var(&env_key, &value);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "key": env_key})),
    )
}

/// GET /api/hands/{hand_id}/settings — Get settings schema and current values for a hand.
#[utoipa::path(
    get,
    path = "/api/hands/{hand_id}/settings",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    responses(
        (status = 200, description = "Get settings schema and current values", body = serde_json::Value)
    )
)]
pub async fn get_hand_settings(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    let lang = headers
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(&[',', ';', '-'][..]).next());

    let settings_status = match state
        .kernel
        .hands()
        .check_settings_availability(&hand_id, lang)
    {
        Ok(s) => s,
        Err(_) => {
            return ApiErrorResponse::not_found(format!("Hand not found: {hand_id}"))
                .into_json_tuple();
        }
    };

    // Find active instance config values (if any)
    let instance_config: std::collections::HashMap<String, serde_json::Value> = state
        .kernel
        .hands()
        .list_instances()
        .iter()
        .find(|i| i.hand_id == hand_id)
        .map(|i| i.config.clone())
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hand_id": hand_id,
            "settings": settings_status,
            "current_values": instance_config,
        })),
    )
}

/// PUT /api/hands/{hand_id}/settings — Update settings for a hand instance.
#[utoipa::path(
    put,
    path = "/api/hands/{hand_id}/settings",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Update settings for a hand instance", body = serde_json::Value)
    )
)]
pub async fn update_hand_settings(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    Json(config): Json<std::collections::HashMap<String, serde_json::Value>>,
) -> impl IntoResponse {
    // Find active instance for this hand
    let instance_id = state
        .kernel
        .hands()
        .list_instances()
        .iter()
        .find(|i| i.hand_id == hand_id)
        .map(|i| i.instance_id);

    match instance_id {
        Some(id) => match state.kernel.hands().update_config(id, config.clone()) {
            Ok(()) => {
                state.kernel.persist_hand_state();
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "hand_id": hand_id,
                        "instance_id": id,
                        "config": config,
                    })),
                )
            }
            Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
        },
        None => ApiErrorResponse::not_found(format!(
            "No active instance for hand: {hand_id}. Activate the hand first."
        ))
        .into_json_tuple(),
    }
}

/// POST /api/hands/reload — Reload hand definitions from disk.
#[utoipa::path(
    post,
    path = "/api/hands/reload",
    tag = "hands",
    responses(
        (status = 200, description = "Reload hand definitions from disk", body = serde_json::Value)
    )
)]
pub async fn reload_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (added, updated) = state.kernel.reload_hands();
    let total = state.kernel.hands().list_definitions().len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "added": added,
            "updated": updated,
            "total": total,
        })),
    )
}

/// GET /api/hands/instances/{id}/stats — Get dashboard stats for a hand instance.
#[utoipa::path(
    get,
    path = "/api/hands/instances/{id}/stats",
    tag = "hands",
    params(
        ("id" = String, Path, description = "Instance ID"),
    ),
    responses(
        (status = 200, description = "Get dashboard stats for a hand instance", body = serde_json::Value)
    )
)]
pub async fn hand_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let instance = match state.kernel.hands().get_instance(id) {
        Some(i) => i,
        None => {
            return ApiErrorResponse::not_found("Instance not found").into_json_tuple();
        }
    };

    let def = match state.kernel.hands().get_definition(&instance.hand_id) {
        Some(d) => d,
        None => {
            return ApiErrorResponse::not_found("Hand definition not found").into_json_tuple();
        }
    };

    let agent_id = match instance.agent_id() {
        Some(aid) => aid,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "instance_id": id,
                    "hand_id": instance.hand_id,
                    "metrics": {},
                })),
            );
        }
    };

    // Read dashboard metrics from agent's structured memory
    let mut metrics = serde_json::Map::new();
    for metric in &def.dashboard.metrics {
        let value = state
            .kernel
            .memory_substrate()
            .structured_get(agent_id, &metric.memory_key)
            .ok()
            .flatten()
            .unwrap_or(serde_json::Value::Null);
        metrics.insert(
            metric.label.clone(),
            serde_json::json!({
                "value": value,
                "format": metric.format,
            }),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "instance_id": id,
            "hand_id": instance.hand_id,
            "status": format!("{}", instance.status),
            "agent_id": agent_id.to_string(),
            "metrics": metrics,
        })),
    )
}

/// GET /api/hands/instances/{id}/browser — Get live browser state for a hand instance.
#[utoipa::path(
    get,
    path = "/api/hands/instances/{id}/browser",
    tag = "hands",
    params(
        ("id" = String, Path, description = "Instance ID"),
    ),
    responses(
        (status = 200, description = "Get live browser state for a hand instance", body = serde_json::Value)
    )
)]
pub async fn hand_instance_browser(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    // 1. Look up instance
    let instance = match state.kernel.hands().get_instance(id) {
        Some(i) => i,
        None => {
            return ApiErrorResponse::not_found("Instance not found").into_json_tuple();
        }
    };

    // 2. Get agent_id
    let agent_id = match instance.agent_id() {
        Some(aid) => aid,
        None => {
            return (StatusCode::OK, Json(serde_json::json!({"active": false})));
        }
    };

    let agent_id_str = agent_id.to_string();

    // 3. Check if a browser session exists (without creating one)
    if !state.kernel.browser().has_session(&agent_id_str) {
        return (StatusCode::OK, Json(serde_json::json!({"active": false})));
    }

    // 4. Send ReadPage command to get page info
    let mut url = String::new();
    let mut title = String::new();
    let mut content = String::new();

    match state
        .kernel
        .browser()
        .send_command(
            &agent_id_str,
            librefang_runtime::browser::BrowserCommand::ReadPage,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                url = data["url"].as_str().unwrap_or("").to_string();
                title = data["title"].as_str().unwrap_or("").to_string();
                content = data["content"].as_str().unwrap_or("").to_string();
                // Truncate content to avoid huge payloads (UTF-8 safe)
                if content.len() > 2000 {
                    content = format!(
                        "{}... (truncated)",
                        librefang_types::truncate_str(&content, 2000)
                    );
                }
            }
        }
        Ok(_) => {}  // Non-success: leave defaults
        Err(_) => {} // Error: leave defaults
    }

    // 5. Send Screenshot command to get visual state
    let mut screenshot_base64 = String::new();

    match state
        .kernel
        .browser()
        .send_command(
            &agent_id_str,
            librefang_runtime::browser::BrowserCommand::Screenshot,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                screenshot_base64 = data["image_base64"].as_str().unwrap_or("").to_string();
            }
        }
        Ok(_) => {}
        Err(_) => {}
    }

    // 6. Return combined state
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "active": true,
            "url": url,
            "title": title,
            "content": content,
            "screenshot_base64": screenshot_base64,
        })),
    )
}

// ---------------------------------------------------------------------------
// Hand instance proxy endpoints — users interact with hands, not raw agents
// ---------------------------------------------------------------------------

/// Helper: resolve a hand instance UUID → its linked AgentId.
/// Returns an error response tuple if the instance is missing or has no agent.
fn resolve_hand_agent(
    state: &AppState,
    instance_id: uuid::Uuid,
) -> Result<
    (
        librefang_hands::HandInstance,
        librefang_types::agent::AgentId,
    ),
    (StatusCode, Json<serde_json::Value>),
> {
    let instance = state
        .kernel
        .hands()
        .get_instance(instance_id)
        .ok_or_else(|| ApiErrorResponse::not_found("Hand instance not found").into_json_tuple())?;
    let agent_id = instance.agent_id().ok_or_else(|| {
        (
            StatusCode::OK,
            Json(serde_json::json!({"error": "Hand instance is not active", "active": false})),
        )
    })?;
    Ok((instance, agent_id))
}

/// POST /api/hands/instances/:id/message — Send a message to a hand.
///
/// This is the primary user-facing chat endpoint.  Internally it proxies to
/// the underlying agent, but users never need to know the agent ID.
pub async fn hand_send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    let (_instance, agent_id) = match resolve_hand_agent(&state, id) {
        Ok(v) => v,
        Err(e) => return e,
    };

    // Reject oversized messages
    const MAX_MESSAGE_SIZE: usize = 64 * 1024;
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        );
    }

    // Resolve file attachments
    if !req.attachments.is_empty() {
        let image_blocks = super::agents::resolve_attachments(&req.attachments);
        if !image_blocks.is_empty() {
            super::agents::inject_attachments_into_session(&state.kernel, agent_id, image_blocks);
        }
    }

    // Detect ephemeral mode
    let (effective_message, is_ephemeral) = if req.ephemeral {
        (req.message.clone(), true)
    } else if let Some(stripped) = req.message.strip_prefix("/btw ") {
        (stripped.to_string(), true)
    } else {
        (req.message.clone(), false)
    };

    let result = if is_ephemeral {
        state
            .kernel
            .send_message_ephemeral(agent_id, &effective_message)
            .await
    } else {
        let kernel_handle: Arc<dyn librefang_runtime::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn librefang_runtime::kernel_handle::KernelHandle>;
        state
            .kernel
            .send_message_with_handle(agent_id, &effective_message, Some(kernel_handle))
            .await
    };

    match result {
        Ok(result) => {
            let cleaned = crate::ws::strip_think_tags(&result.response);
            let response = if cleaned.trim().is_empty() {
                format!(
                    "[Hand completed processing but returned no text. ({} in / {} out | {} iter)]",
                    result.total_usage.input_tokens,
                    result.total_usage.output_tokens,
                    result.iterations,
                )
            } else {
                cleaned
            };
            (
                StatusCode::OK,
                Json(serde_json::json!(MessageResponse {
                    response,
                    input_tokens: result.total_usage.input_tokens,
                    output_tokens: result.total_usage.output_tokens,
                    iterations: result.iterations,
                    cost_usd: result.cost_usd,
                    decision_traces: result.decision_traces,
                    memories_saved: result.memories_saved,
                    memories_used: result.memories_used,
                    memory_conflicts: result.memory_conflicts,
                    thinking: None,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("hand_send_message failed for instance {id}: {e}");
            ApiErrorResponse::internal(format!("Message delivery failed: {e}")).into_json_tuple()
        }
    }
}

/// GET /api/hands/instances/:id/session — Get hand conversation history.
pub async fn hand_get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let (_instance, agent_id) = match resolve_hand_agent(&state, id) {
        Ok(v) => v,
        Err(e) => return e,
    };

    // Delegate to the existing agent session logic
    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found("Linked agent not found").into_json_tuple();
        }
    };

    match state
        .kernel
        .memory_substrate()
        .get_session(entry.session_id)
    {
        Ok(Some(session)) => {
            let messages: Vec<serde_json::Value> = session
                .messages
                .iter()
                .map(|m| {
                    let (content, blocks) = match &m.content {
                        librefang_types::message::MessageContent::Text(t) => (t.clone(), None),
                        librefang_types::message::MessageContent::Blocks(blocks) => {
                            // Text-only content for backward compatibility
                            let text = blocks
                                .iter()
                                .filter_map(|b| match b {
                                    librefang_types::message::ContentBlock::Text {
                                        text, ..
                                    } => Some(text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            // Structured blocks for rich rendering
                            let structured: Vec<serde_json::Value> = blocks
                                .iter()
                                .filter_map(|b| match b {
                                    librefang_types::message::ContentBlock::Text {
                                        text, ..
                                    } => Some(serde_json::json!({
                                        "type": "text", "text": text
                                    })),
                                    librefang_types::message::ContentBlock::ToolUse {
                                        id,
                                        name,
                                        input,
                                        ..
                                    } => Some(serde_json::json!({
                                        "type": "tool_use", "id": id, "name": name, "input": input
                                    })),
                                    librefang_types::message::ContentBlock::ToolResult {
                                        tool_use_id,
                                        tool_name,
                                        content,
                                        is_error,
                                        ..
                                    } => Some(serde_json::json!({
                                        "type": "tool_result",
                                        "tool_use_id": tool_use_id,
                                        "name": tool_name,
                                        "content": content,
                                        "is_error": is_error,
                                    })),
                                    _ => None,
                                })
                                .collect();
                            let has_non_text = structured
                                .iter()
                                .any(|b| b["type"].as_str() != Some("text"));
                            (text, if has_non_text { Some(structured) } else { None })
                        }
                    };
                    let mut msg = serde_json::json!({
                        "role": format!("{:?}", m.role).to_lowercase(),
                        "content": content,
                    });
                    if let Some(blocks) = blocks {
                        msg["blocks"] = serde_json::Value::Array(blocks);
                    }
                    msg
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "messages": messages })),
            )
        }
        Ok(None) => (StatusCode::OK, Json(serde_json::json!({ "messages": [] }))),
        Err(e) => {
            ApiErrorResponse::internal(format!("Failed to load session: {e}")).into_json_tuple()
        }
    }
}

/// GET /api/hands/instances/:id/status — Combined hand + agent status.
///
/// Returns everything the dashboard needs in one call: hand metadata,
/// activation state, agent runtime info, and model details.
pub async fn hand_instance_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let lang = headers
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(&[',', ';', '-'][..]).next())
        .unwrap_or("en");

    let instance = match state.kernel.hands().get_instance(id) {
        Some(i) => i,
        None => {
            return ApiErrorResponse::not_found("Hand instance not found").into_json_tuple();
        }
    };

    // Hand-level info (always available)
    let hand_def = state
        .kernel
        .hands()
        .list_definitions()
        .into_iter()
        .find(|d| d.id == instance.hand_id);

    let resolved_name: Option<String> = hand_def.as_ref().map(|d| {
        d.i18n
            .get(lang)
            .and_then(|l| l.name.as_deref())
            .unwrap_or(&d.name)
            .to_string()
    });

    let mut resp = serde_json::json!({
        "instance_id": instance.instance_id,
        "hand_id": instance.hand_id,
        "hand_name": resolved_name,
        "hand_icon": hand_def.as_ref().map(|d| d.icon.as_str()),
        "status": format!("{:?}", instance.status),
        "activated_at": instance.activated_at.to_rfc3339(),
        "config": instance.config,
    });

    // Agent-level info (only when active)
    if let Some(agent_id) = instance.agent_id() {
        if let Some(entry) = state.kernel.agent_registry().get(agent_id) {
            resp["agent"] = serde_json::json!({
                "id": agent_id.to_string(),
                "name": entry.manifest.name,
                "state": format!("{:?}", entry.state),
                "model": {
                    "provider": entry.manifest.model.provider,
                    "model": entry.manifest.model.model,
                },
                "iterations_total": entry.manifest.autonomous.as_ref().map(|a| a.max_iterations),
                "session_id": entry.session_id.to_string(),
            });
        }
    }

    (StatusCode::OK, Json(resp))
}

// ---------------------------------------------------------------------------
// MCP server endpoints
// ---------------------------------------------------------------------------

fn http_compat_header_summary(
    header: &librefang_types::config::HttpCompatHeaderConfig,
) -> serde_json::Value {
    serde_json::json!({
        "name": header.name,
        "value_env": header.value_env,
        "source": if header.value_env.is_some() {
            "env"
        } else if header.value.is_some() {
            "static"
        } else {
            "unset"
        },
    })
}

fn http_compat_tool_summary(
    tool: &librefang_types::config::HttpCompatToolConfig,
) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "path": tool.path,
        "method": serde_json::to_value(&tool.method).unwrap_or(serde_json::json!("post")),
        "request_mode": serde_json::to_value(&tool.request_mode)
            .unwrap_or(serde_json::json!("json_body")),
        "response_mode": serde_json::to_value(&tool.response_mode)
            .unwrap_or(serde_json::json!("json")),
    })
}

fn serialize_mcp_transport(
    transport: &librefang_types::config::McpTransportEntry,
) -> serde_json::Value {
    match transport {
        librefang_types::config::McpTransportEntry::Stdio { command, args } => {
            serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
            })
        }
        librefang_types::config::McpTransportEntry::Sse { url } => {
            serde_json::json!({
                "type": "sse",
                "url": url,
            })
        }
        librefang_types::config::McpTransportEntry::Http { url } => {
            serde_json::json!({
                "type": "http",
                "url": url,
            })
        }
        librefang_types::config::McpTransportEntry::HttpCompat {
            base_url,
            headers,
            tools,
        } => {
            let tool_summaries: Vec<serde_json::Value> =
                tools.iter().map(http_compat_tool_summary).collect();
            let header_summaries: Vec<serde_json::Value> =
                headers.iter().map(http_compat_header_summary).collect();
            serde_json::json!({
                "type": "http_compat",
                "base_url": base_url,
                "headers": header_summaries,
                "tools_count": tool_summaries.len(),
                "tools": tool_summaries,
            })
        }
    }
}

/// GET /api/mcp/servers — List configured MCP servers and their tools.
#[utoipa::path(
    get,
    path = "/api/mcp/servers",
    tag = "mcp",
    responses(
        (status = 200, description = "List configured MCP servers and their tools", body = serde_json::Value)
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

    // Get configured servers from config
    let config_servers: Vec<serde_json::Value> = state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .map(|s| {
            let transport = s.transport.as_ref().map(serialize_mcp_transport);
            let auth_state = auth_snapshot
                .get(&s.name)
                .cloned()
                .unwrap_or(serde_json::json!({"state": "not_required"}));
            serde_json::json!({
                "name": s.name,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                "env": s.env,
                "auth_state": auth_state,
            })
        })
        .collect();

    // Get connected servers and their tools from the live MCP connections
    let connections = state.kernel.mcp_connections_ref().lock().await;
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
            serde_json::json!({
                "name": conn.name(),
                "tools_count": tools.len(),
                "tools": tools,
                "connected": true,
            })
        })
        .collect();

    Json(serde_json::json!({
        "configured": config_servers,
        "connected": connected,
        "total_configured": config_servers.len(),
        "total_connected": connected.len(),
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
        (status = 200, description = "MCP server details", body = serde_json::Value),
        (status = 404, description = "MCP server not found")
    )
)]
pub async fn get_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Find the configured entry by name — use config_snapshot() because
    // the result is held across an .await below.
    let cfg = state.kernel.config_snapshot();
    let entry = cfg.mcp_servers.iter().find(|s| s.name == name);

    let entry = match entry {
        Some(e) => e,
        None => {
            return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
                .into_json_tuple();
        }
    };

    let transport = entry.transport.as_ref().map(serialize_mcp_transport);

    let mut result = serde_json::json!({
        "name": entry.name,
        "transport": transport,
        "timeout_secs": entry.timeout_secs,
        "env": entry.env,
        "connected": false,
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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Add a new MCP server configuration", body = serde_json::Value)
    )
)]
pub async fn add_mcp_server(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Validate required fields
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

    // Validate by deserializing the body into McpServerConfigEntry
    let entry: librefang_types::config::McpServerConfigEntry = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => {
            return ApiErrorResponse::bad_request(format!("Invalid MCP server config: {e}"))
                .into_json_tuple();
        }
    };

    // Check for duplicate name
    if state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .any(|s| s.name == name)
    {
        return ApiErrorResponse::conflict(format!("MCP server '{}' already exists", name))
            .into_json_tuple();
    }

    // Persist to config.toml
    let config_path = state.kernel.home_dir().join("config.toml");
    if let Err(e) = upsert_mcp_server_config(&config_path, &entry) {
        return ApiErrorResponse::internal(format!("Failed to write config: {e}"))
            .into_json_tuple();
    }

    // Trigger config reload
    let reload_status = match state.kernel.reload_config().await {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("mcp_server added: {name}"),
        "completed",
    );

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "added",
            "name": name,
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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Update an existing MCP server configuration", body = serde_json::Value)
    )
)]
pub async fn update_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(mut body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Ensure the entry exists
    if !state
        .kernel
        .config_ref()
        .mcp_servers
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
    let entry: librefang_types::config::McpServerConfigEntry = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => {
            return ApiErrorResponse::bad_request(
                t.t_args("api-error-mcp-invalid-config", &[("error", &e.to_string())]),
            )
            .into_json_tuple();
        }
    };

    // Persist — upsert replaces an existing entry with the same name
    let config_path = state.kernel.home_dir().join("config.toml");
    if let Err(e) = upsert_mcp_server_config(&config_path, &entry) {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-config-write-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }
    // Drop ErrorTranslator before .await — FluentBundle is !Send and cannot
    // be held across an async suspension point.
    drop(t);

    let reload_status = match state.kernel.reload_config().await {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("mcp_server updated: {name}"),
        "completed",
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
        (status = 200, description = "Remove an MCP server configuration", body = serde_json::Value)
    )
)]
pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Ensure the entry exists
    if !state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .any(|s| s.name == name)
    {
        return ApiErrorResponse::not_found(
            t.t_args("api-error-mcp-not-found", &[("name", &name)]),
        )
        .into_json_tuple();
    }

    // Resolve server URL before removing config (needed for vault cleanup)
    let server_url = state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .find(|s| s.name == name)
        .and_then(|s| match &s.transport {
            Some(librefang_types::config::McpTransportEntry::Http { url }) => Some(url.clone()),
            Some(librefang_types::config::McpTransportEntry::Sse { url }) => Some(url.clone()),
            _ => None,
        });

    let config_path = state.kernel.home_dir().join("config.toml");
    if let Err(e) = remove_mcp_server_config(&config_path, &name) {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-config-write-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }
    drop(t);

    // Clean up OAuth vault tokens, auth state, and live connections
    if let Some(ref url) = server_url {
        let provider = librefang_kernel::mcp_oauth_provider::KernelOAuthProvider::new(
            state.kernel.home_dir().to_path_buf(),
        );
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
            let _ = provider.vault_remove(
                &librefang_kernel::mcp_oauth_provider::KernelOAuthProvider::vault_key(url, field),
            );
        }
    }
    state
        .kernel
        .mcp_auth_states_ref()
        .lock()
        .await
        .remove(&name);
    state
        .kernel
        .mcp_connections_ref()
        .lock()
        .await
        .retain(|c| c.name() != name);

    let reload_status = match state.kernel.reload_config().await {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("mcp_server removed: {name}"),
        "completed",
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

/// Upsert an MCP server entry in config.toml's `[[mcp_servers]]` array.
///
/// If an entry with the same name already exists it is replaced; otherwise a
/// new entry is appended.
fn upsert_mcp_server_config(
    config_path: &std::path::Path,
    entry: &librefang_types::config::McpServerConfigEntry,
) -> Result<(), String> {
    validate_static_file_path(config_path, "config.toml")?;
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        toml::value::Table::new()
    };

    // Serialize the entry to a TOML value via JSON round-trip
    let entry_json = serde_json::to_value(entry).map_err(|e| e.to_string())?;
    let entry_toml = json_to_toml_value(&entry_json);

    let servers = table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));

    if let toml::Value::Array(ref mut arr) = servers {
        // Remove existing entry with same name (if any)
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != entry.name)
                .unwrap_or(true)
        });
        // Append new/updated entry
        arr.push(entry_toml);
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove an MCP server entry from config.toml's `[[mcp_servers]]` array by name.
fn remove_mcp_server_config(config_path: &std::path::Path, name: &str) -> Result<(), String> {
    validate_static_file_path(config_path, "config.toml")?;
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        return Ok(());
    };

    if let Some(toml::Value::Array(ref mut arr)) = table.get_mut("mcp_servers") {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != name)
                .unwrap_or(true)
        });
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

fn is_safe_component_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && std::path::Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            == Some(name)
}

fn validate_static_file_path(
    path: &std::path::Path,
    expected_file_name: &str,
) -> Result<(), String> {
    let actual = path.file_name().and_then(|name| name.to_str());
    if actual != Some(expected_file_name) {
        return Err(format!(
            "invalid file path '{}': expected file '{}'",
            path.display(),
            expected_file_name
        ));
    }
    // Block path-traversal components (`..`). We intentionally do NOT reject
    // `Component::Prefix` — on Windows every absolute path contains a drive-
    // letter prefix (e.g. `C:`), and the paths passed here are constructed
    // server-side via `home_dir().join(file)`, so the prefix is legitimate.
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("unsafe path '{}'", path.display()));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/skills/create",
    tag = "skills",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Create a new prompt-only skill", body = serde_json::Value)
    )
)]
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match body["name"].as_str() {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing or empty 'name' field")
                .into_json_tuple();
        }
    };

    // Validate name (alphanumeric + hyphens only)
    if !is_safe_component_name(&name) {
        return ApiErrorResponse::bad_request(
            "Skill name must contain only letters, numbers, hyphens, and underscores",
        )
        .into_json_tuple();
    }

    let description = body["description"].as_str().unwrap_or("").to_string();
    let runtime = body["runtime"].as_str().unwrap_or("prompt_only");
    let prompt_context = body["prompt_context"].as_str().unwrap_or("").to_string();

    // Only allow prompt_only skills from the web UI for safety
    if runtime != "prompt_only" {
        return ApiErrorResponse::bad_request(
            "Only prompt_only skills can be created from the web UI",
        )
        .into_json_tuple();
    }

    // Write skill.toml to ~/.librefang/skills/{name}/
    let skill_dir = state.kernel.home_dir().join("skills").join(&name);
    if skill_dir.exists() {
        return ApiErrorResponse::conflict(format!("Skill '{}' already exists", name))
            .into_json_tuple();
    }

    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        return ApiErrorResponse::internal(format!("Failed to create skill directory: {e}"))
            .into_json_tuple();
    }

    let toml_content = format!(
        "[skill]\nname = \"{}\"\ndescription = \"{}\"\nruntime = \"prompt_only\"\n\n[prompt]\ncontext = \"\"\"\n{}\n\"\"\"\n",
        name,
        description.replace('"', "\\\""),
        prompt_context
    );

    let toml_path = skill_dir.join("skill.toml");
    if let Err(e) = std::fs::write(&toml_path, &toml_content) {
        return ApiErrorResponse::internal(format!("Failed to write skill.toml: {e}"))
            .into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "created",
            "name": name,
            "note": "Restart the daemon to load the new skill, or it will be available on next boot."
        })),
    )
}

// ── Helper functions for secrets.env management ────────────────────────

/// Denylist of critical system environment variables that must not be overwritten.
const DENIED_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "TERM",
    "LANG",
    "PWD",
];

/// Maximum allowed length for an environment variable value.
const ENV_VALUE_MAX_LEN: usize = 4096;

/// Validate an environment variable name and value before setting them.
///
/// Rules:
/// - Name must match `^[A-Za-z_][A-Za-z0-9_]*$`
/// - Name must not be in the system denylist
/// - Value length must not exceed [`ENV_VALUE_MAX_LEN`]
pub(crate) fn validate_env_var(name: &str, value: &str) -> Result<(), String> {
    // Check name format: must start with letter or underscore, then alphanumeric/underscore
    if name.is_empty() {
        return Err("Environment variable name must not be empty".to_string());
    }
    let first = name.as_bytes()[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return Err(format!(
            "Environment variable name '{}' must start with a letter or underscore",
            name
        ));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!(
            "Environment variable name '{}' contains invalid characters (only A-Z, a-z, 0-9, _ allowed)",
            name
        ));
    }

    // Check denylist
    let upper = name.to_ascii_uppercase();
    if DENIED_ENV_VARS.iter().any(|&d| d == upper) {
        return Err(format!(
            "Environment variable '{}' is a protected system variable and cannot be overwritten",
            name
        ));
    }

    // Check value length
    if value.len() > ENV_VALUE_MAX_LEN {
        return Err(format!(
            "Environment variable value exceeds maximum length of {} bytes",
            ENV_VALUE_MAX_LEN
        ));
    }

    Ok(())
}

/// Write or update a key in the secrets.env file.
/// File format: one `KEY=value` per line. Existing keys are overwritten.
pub(crate) fn write_secret_env(
    path: &std::path::Path,
    key: &str,
    value: &str,
) -> Result<(), std::io::Error> {
    validate_static_file_path(path, "secrets.env")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Remove existing line for this key
    lines.retain(|l| !l.starts_with(&format!("{key}=")));

    // Add new line
    lines.push(format!("{key}={value}"));

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, lines.join("\n") + "\n")?;

    // SECURITY: Restrict file permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!("Failed to set file permissions: {e}");
        }
    }

    Ok(())
}

/// Remove a key from the secrets.env file.
pub(crate) fn remove_secret_env(path: &std::path::Path, key: &str) -> Result<(), std::io::Error> {
    validate_static_file_path(path, "secrets.env")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    if !path.exists() {
        return Ok(());
    }

    let lines: Vec<String> = std::fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.starts_with(&format!("{key}=")))
        .map(|l| l.to_string())
        .collect();

    std::fs::write(path, lines.join("\n") + "\n")?;

    Ok(())
}

// ── Config.toml channel management helpers ──────────────────────────

/// Upsert a `[channels.<name>]` section in config.toml with the given non-secret fields.
pub(crate) fn upsert_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
    fields: &HashMap<String, (String, FieldType)>,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_static_file_path(config_path, "config.toml")
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    // Ensure [channels] table exists
    if !root.contains_key("channels") {
        root.insert(
            "channels".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let channels_table = root
        .get_mut("channels")
        .and_then(|v| v.as_table_mut())
        .ok_or("channels is not a table")?;

    // Build channel sub-table with correct TOML types
    let mut ch_table = toml::map::Map::new();
    for (k, (v, ft)) in fields {
        let toml_val = match ft {
            FieldType::Number => {
                if let Ok(n) = v.parse::<i64>() {
                    toml::Value::Integer(n)
                } else {
                    toml::Value::String(v.clone())
                }
            }
            FieldType::List => {
                // Always store list items as strings so that numeric IDs
                // (e.g. Discord guild snowflakes, Telegram user IDs) are
                // deserialized correctly into Vec<String> config fields.
                let items: Vec<toml::Value> = v
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| toml::Value::String(s.to_string()))
                    .collect();
                toml::Value::Array(items)
            }
            _ => toml::Value::String(v.clone()),
        };
        ch_table.insert(k.clone(), toml_val);
    }
    channels_table.insert(channel_name.to_string(), toml::Value::Table(ch_table));

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

/// Remove a `[channels.<name>]` section from config.toml.
pub(crate) fn remove_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_static_file_path(config_path, "config.toml")
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    if !config_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(());
    }

    let mut doc: toml::Value = toml::from_str(&content)?;

    if let Some(channels) = doc
        .as_table_mut()
        .and_then(|r| r.get_mut("channels"))
        .and_then(|c| c.as_table_mut())
    {
        channels.remove(channel_name);
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Integration management endpoints
// ---------------------------------------------------------------------------

/// Derive a human-readable status string for an integration.
fn integration_status_str(
    installed: Option<&librefang_extensions::InstalledIntegration>,
    health: Option<&librefang_extensions::health::IntegrationHealth>,
) -> &'static str {
    match installed {
        Some(inst) if !inst.enabled => "disabled",
        Some(_) => match health.map(|h| &h.status) {
            Some(librefang_extensions::IntegrationStatus::Ready) => "ready",
            Some(librefang_extensions::IntegrationStatus::Error(_)) => "error",
            _ => "installed",
        },
        None => "available",
    }
}

/// GET /api/integrations — List installed integrations with status.
#[utoipa::path(
    get,
    path = "/api/integrations",
    tag = "integrations",
    responses(
        (status = 200, description = "List installed integrations with status", body = serde_json::Value)
    )
)]
pub async fn list_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extensions()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_monitor();

    let mut entries = Vec::new();
    for info in registry.list_all_info() {
        let h = health.get_health(&info.template.id);
        let status = integration_status_str(info.installed.as_ref(), h.as_ref());
        if status == "available" {
            continue; // Only show installed
        }
        entries.push(serde_json::json!({
            "id": info.template.id,
            "name": info.template.name,
            "icon": info.template.icon,
            "category": info.template.category.to_string(),
            "status": status,
            "tool_count": h.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed_at": info.installed.as_ref().map(|i| i.installed_at.to_rfc3339()),
        }));
    }

    Json(serde_json::json!({
        "installed": entries,
        "count": entries.len(),
    }))
}

/// GET /api/integrations/:id — Get a single integration by ID.
#[utoipa::path(
    get,
    path = "/api/integrations/{id}",
    tag = "integrations",
    params(
        ("id" = String, Path, description = "Integration ID"),
    ),
    responses(
        (status = 200, description = "Integration detail", body = serde_json::Value),
        (status = 404, description = "Integration not found", body = serde_json::Value),
    )
)]
pub async fn get_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let registry = state
        .kernel
        .extensions()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_monitor();

    // Look up the template first
    let template = match registry.get_template(&id) {
        Some(t) => t,
        None => {
            return ApiErrorResponse::not_found(format!("Integration '{}' not found", id))
                .into_json_tuple()
                .into_response();
        }
    };

    let installed = registry.get_installed(&id);
    let h = health.get_health(&id);

    let status = integration_status_str(installed, h.as_ref());

    let error_message = h.as_ref().and_then(|h| match &h.status {
        librefang_extensions::IntegrationStatus::Error(msg) => Some(msg.clone()),
        _ => None,
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": template.id,
            "name": template.name,
            "description": template.description,
            "icon": template.icon,
            "category": template.category.to_string(),
            "status": status,
            "tags": template.tags,
            "tool_count": h.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed": installed.is_some(),
            "enabled": installed.map(|i| i.enabled).unwrap_or(false),
            "installed_at": installed.map(|i| i.installed_at.to_rfc3339()),
            "has_oauth": template.oauth.is_some(),
            "setup_instructions": template.setup_instructions,
            "required_env": template.required_env.iter().map(|e| serde_json::json!({
                "name": e.name,
                "label": e.label,
                "help": e.help,
                "is_secret": e.is_secret,
                "get_url": e.get_url,
            })).collect::<Vec<_>>(),
            "error": error_message,
        })),
    )
        .into_response()
}

/// GET /api/integrations/available — List all available templates.
#[utoipa::path(
    get,
    path = "/api/integrations/available",
    tag = "integrations",
    responses(
        (status = 200, description = "List all available integration templates", body = serde_json::Value)
    )
)]
pub async fn list_available_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extensions()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let templates: Vec<serde_json::Value> = registry
        .list_templates()
        .iter()
        .map(|t| {
            let installed = registry.is_installed(&t.id);
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "description": t.description,
                "icon": t.icon,
                "category": t.category.to_string(),
                "installed": installed,
                "tags": t.tags,
                "required_env": t.required_env.iter().map(|e| serde_json::json!({
                    "name": e.name,
                    "label": e.label,
                    "help": e.help,
                    "is_secret": e.is_secret,
                    "get_url": e.get_url,
                })).collect::<Vec<_>>(),
                "has_oauth": t.oauth.is_some(),
                "setup_instructions": t.setup_instructions,
            })
        })
        .collect();

    Json(serde_json::json!({
        "integrations": templates,
        "count": templates.len(),
    }))
}

/// POST /api/integrations/add — Install an integration.
#[utoipa::path(
    post,
    path = "/api/integrations/add",
    tag = "integrations",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Install an integration", body = serde_json::Value)
    )
)]
pub async fn add_integration(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = match req.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return ApiErrorResponse::bad_request("Missing 'id' field").into_json_tuple();
        }
    };

    // Scope the write lock so it's dropped before any .await
    let install_err = {
        let mut registry = state
            .kernel
            .extensions()
            .write()
            .unwrap_or_else(|e| e.into_inner());

        if registry.is_installed(&id) {
            Some((
                StatusCode::CONFLICT,
                format!("Integration '{}' already installed", id),
            ))
        } else if registry.get_template(&id).is_none() {
            Some((
                StatusCode::NOT_FOUND,
                format!("Unknown integration: '{}'", id),
            ))
        } else {
            let entry = librefang_extensions::InstalledIntegration {
                id: id.clone(),
                installed_at: chrono::Utc::now(),
                enabled: true,
                oauth_provider: None,
                config: std::collections::HashMap::new(),
            };
            match registry.install(entry) {
                Ok(_) => None,
                Err(e) => Some((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
    }; // write lock dropped here

    if let Some((status, error)) = install_err {
        return (status, Json(serde_json::json!({"error": error})));
    }

    state.kernel.extension_monitor().register(&id);

    // Hot-connect the new MCP server
    let connected = state.kernel.reload_extension_mcps().await.unwrap_or(0);

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "status": "installed",
            "connected": connected > 0,
            "message": format!("Integration '{}' installed", id),
        })),
    )
}

/// DELETE /api/integrations/:id — Remove an integration.
#[utoipa::path(
    delete,
    path = "/api/integrations/{id}",
    tag = "integrations",
    params(
        ("id" = String, Path, description = "Integration ID"),
    ),
    responses(
        (status = 200, description = "Remove an integration", body = serde_json::Value)
    )
)]
pub async fn remove_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Scope the write lock
    let uninstall_err = {
        let mut registry = state
            .kernel
            .extensions()
            .write()
            .unwrap_or_else(|e| e.into_inner());
        registry.uninstall(&id).err()
    };

    if let Some(e) = uninstall_err {
        return ApiErrorResponse::not_found(e.to_string()).into_json_tuple();
    }

    state.kernel.extension_monitor().unregister(&id);

    // Hot-disconnect the removed MCP server
    if let Err(e) = state.kernel.reload_extension_mcps().await {
        tracing::warn!("Failed to reload MCP extensions: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": id,
            "status": "removed",
        })),
    )
}

/// POST /api/integrations/:id/reconnect — Reconnect an MCP server.
#[utoipa::path(
    post,
    path = "/api/integrations/{id}/reconnect",
    tag = "integrations",
    params(
        ("id" = String, Path, description = "Integration ID"),
    ),
    responses(
        (status = 200, description = "Reconnect an integration MCP server", body = serde_json::Value)
    )
)]
pub async fn reconnect_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let is_installed = {
        let registry = state
            .kernel
            .extensions()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        registry.is_installed(&id)
    };

    if !is_installed {
        return ApiErrorResponse::not_found(format!("Integration '{}' not installed", id))
            .into_json_tuple();
    }

    match state.kernel.reconnect_extension_mcp(&id).await {
        Ok(tool_count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": id,
                "status": "connected",
                "tool_count": tool_count,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "id": id,
                "status": "error",
                "error": e,
            })),
        ),
    }
}

/// GET /api/integrations/health — Health status for all integrations.
#[utoipa::path(
    get,
    path = "/api/integrations/health",
    tag = "integrations",
    responses(
        (status = 200, description = "Health status for all integrations", body = serde_json::Value)
    )
)]
pub async fn integrations_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health_entries = state.kernel.extension_monitor().all_health();
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

/// POST /api/integrations/reload — Hot-reload integration configs and reconnect MCP.
#[utoipa::path(
    post,
    path = "/api/integrations/reload",
    tag = "integrations",
    responses(
        (status = 200, description = "Hot-reload integration configs and reconnect MCP", body = serde_json::Value)
    )
)]
pub async fn reload_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.reload_extension_mcps().await {
        Ok(connected) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "reloaded",
                "new_connections": connected,
            })),
        ),
        Err(e) => ApiErrorResponse::internal(e).into_json_tuple(),
    }
}

// ---------------------------------------------------------------------------
// Extension management endpoints
// ---------------------------------------------------------------------------

/// GET /api/extensions — List all installed extensions (integrations) with status.
#[utoipa::path(
    get,
    path = "/api/extensions",
    tag = "extensions",
    responses(
        (status = 200, description = "List all installed extensions with status", body = serde_json::Value)
    )
)]
pub async fn list_extensions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extensions()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_monitor();

    let mut extensions = Vec::new();
    for info in registry.list_all_info() {
        let h = health.get_health(&info.template.id);
        let status = match &info.installed {
            Some(inst) if !inst.enabled => "disabled",
            Some(_) => match h.as_ref().map(|h| &h.status) {
                Some(librefang_extensions::IntegrationStatus::Ready) => "ready",
                Some(librefang_extensions::IntegrationStatus::Error(_)) => "error",
                _ => "installed",
            },
            None => "available",
        };
        extensions.push(serde_json::json!({
            "name": info.template.id,
            "display_name": info.template.name,
            "description": info.template.description,
            "icon": info.template.icon,
            "category": info.template.category.to_string(),
            "status": status,
            "tags": info.template.tags,
            "installed": info.installed.is_some(),
            "tool_count": h.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed_at": info.installed.as_ref().map(|i| i.installed_at.to_rfc3339()),
        }));
    }

    Json(serde_json::json!({
        "extensions": extensions,
        "total": extensions.len(),
    }))
}

/// GET /api/extensions/:name — Get details for a single extension by name.
#[utoipa::path(
    get,
    path = "/api/extensions/{name}",
    tag = "extensions",
    params(
        ("name" = String, Path, description = "Extension name"),
    ),
    responses(
        (status = 200, description = "Get details for a single extension", body = serde_json::Value)
    )
)]
pub async fn get_extension(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let registry = state
        .kernel
        .extensions()
        .read()
        .unwrap_or_else(|e| e.into_inner());

    let template = match registry.get_template(&name) {
        Some(t) => t.clone(),
        None => {
            return ApiErrorResponse::not_found(format!("Extension '{}' not found", name))
                .into_json_tuple();
        }
    };

    let installed = registry.get_installed(&name).cloned();
    let health = state.kernel.extension_monitor().get_health(&name);

    let status = match &installed {
        Some(inst) if !inst.enabled => "disabled",
        Some(_) => match health.as_ref().map(|h| &h.status) {
            Some(librefang_extensions::IntegrationStatus::Ready) => "ready",
            Some(librefang_extensions::IntegrationStatus::Error(_)) => "error",
            _ => "installed",
        },
        None => "available",
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": template.id,
            "display_name": template.name,
            "description": template.description,
            "icon": template.icon,
            "category": template.category.to_string(),
            "status": status,
            "tags": template.tags,
            "installed": installed.is_some(),
            "tool_count": health.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed_at": installed.as_ref().map(|i| i.installed_at.to_rfc3339()),
            "required_env": template.required_env.iter().map(|e| serde_json::json!({
                "name": e.name,
                "label": e.label,
                "help": e.help,
                "is_secret": e.is_secret,
                "get_url": e.get_url,
            })).collect::<Vec<_>>(),
            "has_oauth": template.oauth.is_some(),
            "setup_instructions": template.setup_instructions,
            "health": health.as_ref().map(|h| serde_json::json!({
                "last_ok": h.last_ok.map(|t| t.to_rfc3339()),
                "last_error": h.last_error,
                "consecutive_failures": h.consecutive_failures,
                "reconnecting": h.reconnecting,
            })),
        })),
    )
}

/// POST /api/extensions/install — Install an extension by name.
#[utoipa::path(
    post,
    path = "/api/extensions/install",
    tag = "extensions",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Install an extension by name", body = serde_json::Value)
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

    // Scope the write lock so it's dropped before any .await
    let install_err = {
        let mut registry = state
            .kernel
            .extensions()
            .write()
            .unwrap_or_else(|e| e.into_inner());

        if registry.is_installed(&name) {
            Some((
                StatusCode::CONFLICT,
                format!("Extension '{}' already installed", name),
            ))
        } else if registry.get_template(&name).is_none() {
            Some((
                StatusCode::NOT_FOUND,
                format!("Unknown extension: '{}'", name),
            ))
        } else {
            let entry = librefang_extensions::InstalledIntegration {
                id: name.clone(),
                installed_at: chrono::Utc::now(),
                enabled: true,
                oauth_provider: None,
                config: std::collections::HashMap::new(),
            };
            match registry.install(entry) {
                Ok(_) => None,
                Err(e) => Some((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
    }; // write lock dropped here

    if let Some((status, error)) = install_err {
        return (status, Json(serde_json::json!({"error": error})));
    }

    state.kernel.extension_monitor().register(&name);

    // Hot-connect the new MCP server
    let connected = state.kernel.reload_extension_mcps().await.unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "installed",
            "name": name,
            "connected": connected > 0,
        })),
    )
}

/// POST /api/extensions/uninstall — Uninstall an extension by name.
#[utoipa::path(
    post,
    path = "/api/extensions/uninstall",
    tag = "extensions",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Uninstall an extension by name", body = serde_json::Value)
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

    // Scope the write lock
    let uninstall_err = {
        let mut registry = state
            .kernel
            .extensions()
            .write()
            .unwrap_or_else(|e| e.into_inner());
        registry.uninstall(&name).err()
    };

    if let Some(e) = uninstall_err {
        return ApiErrorResponse::not_found(e.to_string()).into_json_tuple();
    }

    state.kernel.extension_monitor().unregister(&name);

    // Hot-disconnect the removed MCP server
    if let Err(e) = state.kernel.reload_extension_mcps().await {
        tracing::warn!("Failed to reload MCP extensions after uninstall: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "uninstalled",
            "name": name,
        })),
    )
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::config::{McpServerConfigEntry, McpTransportEntry};

    /// Regression for #2319: adding an MCP server through the UI wrote each
    /// entry as a JSON-stringified blob inside `mcp_servers = ['{"name":...}']`
    /// instead of a `[[mcp_servers]]` TOML table, because the top-level object
    /// hit the catch-all in `json_to_toml_value` and got stringified. After
    /// the fix, the on-disk file must round-trip back into a real
    /// `McpServerConfigEntry` via `toml::from_str`.
    #[test]
    fn upsert_mcp_server_writes_inline_table_not_stringified_json() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "").unwrap();

        let entry = McpServerConfigEntry {
            name: "nocodb".to_string(),
            transport: Some(McpTransportEntry::Stdio {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "mcp-remote".to_string(),
                    "http://nocodb:8080/mcp/abc".to_string(),
                ],
            }),
            timeout_secs: 30,
            env: vec![],
            headers: vec!["xc-mcp-token: secret".to_string()],
            oauth: None,
        };

        upsert_mcp_server_config(&config_path, &entry).expect("upsert should succeed");

        let raw = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            !raw.contains("mcp_servers = ['{"),
            "mcp_servers must not be written as stringified JSON — got:\n{raw}"
        );
        assert!(
            !raw.contains("mcp_servers = [\"{"),
            "mcp_servers must not be written as stringified JSON — got:\n{raw}"
        );

        #[derive(serde::Deserialize)]
        struct Wrapper {
            mcp_servers: Vec<McpServerConfigEntry>,
        }
        let parsed: Wrapper =
            toml::from_str(&raw).expect("config.toml must deserialize into McpServerConfigEntry");
        assert_eq!(parsed.mcp_servers.len(), 1);
        let roundtripped = &parsed.mcp_servers[0];
        assert_eq!(roundtripped.name, "nocodb");
        assert_eq!(roundtripped.timeout_secs, 30);
        assert_eq!(roundtripped.headers, vec!["xc-mcp-token: secret"]);
        match &roundtripped.transport {
            Some(McpTransportEntry::Stdio { command, args }) => {
                assert_eq!(command, "npx");
                assert_eq!(args, &["-y", "mcp-remote", "http://nocodb:8080/mcp/abc"]);
            }
            other => panic!("expected stdio transport, got {other:?}"),
        }
    }

    /// A second upsert for the same name must replace the entry in-place,
    /// not produce a second row — this is how the user ended up with three
    /// stale duplicate blobs in the bug report.
    #[test]
    fn upsert_mcp_server_replaces_existing_entry_with_same_name() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "").unwrap();

        let v1 = McpServerConfigEntry {
            name: "nocodb".to_string(),
            transport: Some(McpTransportEntry::Http {
                url: "http://old:8080/mcp".to_string(),
            }),
            timeout_secs: 10,
            env: vec![],
            headers: vec![],
            oauth: None,
        };
        upsert_mcp_server_config(&config_path, &v1).unwrap();

        let v2 = McpServerConfigEntry {
            name: "nocodb".to_string(),
            transport: Some(McpTransportEntry::Http {
                url: "http://new:9090/mcp".to_string(),
            }),
            timeout_secs: 60,
            env: vec![],
            headers: vec![],
            oauth: None,
        };
        upsert_mcp_server_config(&config_path, &v2).unwrap();

        #[derive(serde::Deserialize)]
        struct Wrapper {
            mcp_servers: Vec<McpServerConfigEntry>,
        }
        let raw = std::fs::read_to_string(&config_path).unwrap();
        let parsed: Wrapper = toml::from_str(&raw).unwrap();
        assert_eq!(
            parsed.mcp_servers.len(),
            1,
            "upsert must replace, not append"
        );
        assert_eq!(parsed.mcp_servers[0].timeout_secs, 60);
        match &parsed.mcp_servers[0].transport {
            Some(McpTransportEntry::Http { url }) => assert_eq!(url, "http://new:9090/mcp"),
            other => panic!("expected http transport, got {other:?}"),
        }
    }
}
