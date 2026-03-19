//! Skills, marketplace, ClawHub, hands, and extension handlers.

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
    let skills_dir = state.kernel.config.home_dir.join("skills");
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
                Some(librefang_skills::SkillSource::OpenClaw) => {
                    serde_json::json!({"type": "openclaw"})
                }
                Some(librefang_skills::SkillSource::Bundled) => {
                    serde_json::json!({"type": "bundled"})
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
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let config = librefang_skills::marketplace::MarketplaceConfig::default();
    let client = librefang_skills::marketplace::MarketplaceClient::new(config);

    match client.install(&req.name, &skills_dir).await {
        Ok(version) => {
            // Hot-reload so agents see the new skill immediately
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": req.name,
                    "version": version,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Skill install failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Install failed: {e}")})),
            )
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
    let skills_dir = state.kernel.config.home_dir.join("skills");
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
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
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
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return Json(serde_json::json!({"results": [], "total": 0}));
    }

    let config = librefang_skills::marketplace::MarketplaceConfig::default();
    let client = librefang_skills::marketplace::MarketplaceClient::new(config);

    match client.search(&query).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "name": r.name,
                        "description": r.description,
                        "stars": r.stars,
                        "url": r.url,
                    })
                })
                .collect();
            Json(serde_json::json!({"results": items, "total": items.len()}))
        }
        Err(e) => {
            tracing::warn!("Marketplace search failed: {e}");
            Json(serde_json::json!({"results": [], "total": 0, "error": format!("{e}")}))
        }
    }
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

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
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
                StatusCode::OK
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

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
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
                StatusCode::OK
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
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = librefang_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
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
                .map(|o| o.image.as_str())
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
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
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
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No source code found for this skill"})),
        );
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
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
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
pub async fn list_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let defs = state.kernel.hand_registry.list_definitions();
    let hands: Vec<serde_json::Value> = defs
        .iter()
        .map(|d| {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&d.id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&d.id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "description": d.description,
                "category": d.category,
                "icon": d.icon,
                "tools": d.tools,
                "requirements_met": requirements_met,
                "active": active,
                "degraded": degraded,
                "requirements": reqs.iter().map(|(r, ok)| serde_json::json!({
                    "key": r.key,
                    "label": r.label,
                    "satisfied": ok,
                    "optional": r.optional,
                })).collect::<Vec<_>>(),
                "dashboard_metrics": d.dashboard.metrics.len(),
                "has_settings": !d.settings.is_empty(),
                "settings_count": d.settings.len(),
                "metadata": d.metadata.clone().unwrap_or_default(),
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
pub async fn list_active_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let instances = state.kernel.hand_registry.list_instances();
    let items: Vec<serde_json::Value> = instances
        .iter()
        .map(|i| {
            serde_json::json!({
                "instance_id": i.instance_id,
                "hand_id": i.hand_id,
                "status": format!("{}", i.status),
                "agent_id": i.agent_id.map(|a| a.to_string()),
                "agent_name": i.agent_name,
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
) -> impl IntoResponse {
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&hand_id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            let settings_status = state
                .kernel
                .hand_registry
                .check_settings_availability(&hand_id)
                .unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": def.id,
                    "name": def.name,
                    "description": def.description,
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
                    "agent": {
                        "name": def.agent.name,
                        "description": def.agent.description,
                        "provider": if def.agent.provider == "default" {
                            &state.kernel.config.default_model.provider
                        } else { &def.agent.provider },
                        "model": if def.agent.model == "default" {
                            &state.kernel.config.default_model.model
                        } else { &def.agent.model },
                    },
                    "dashboard": def.dashboard.metrics.iter().map(|m| serde_json::json!({
                        "label": m.label,
                        "memory_key": m.memory_key,
                        "format": m.format,
                    })).collect::<Vec<_>>(),
                    "settings": settings_status,
                    "metadata": def.metadata.clone().unwrap_or_default(),
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
        ),
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
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&hand_id);
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
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
        ),
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
    let def = match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(d) => d.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
            );
        }
    };

    let reqs = state
        .kernel
        .hand_registry
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
        .hand_registry
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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing toml_content field"})),
        );
    }

    match state
        .kernel
        .hand_registry
        .install_from_content(toml_content, skill_content)
    {
        Ok(def) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": def.id,
                "name": def.name,
                "description": def.description,
                "category": format!("{:?}", def.category),
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
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
            if let Some(agent_id) = instance.agent_id {
                let entry = state
                    .kernel
                    .registry
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
                    "agent_id": instance.agent_id.map(|a| a.to_string()),
                    "agent_name": instance.agent_name,
                    "activated_at": instance.activated_at.to_rfc3339(),
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
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
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
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
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
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
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
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
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    let settings_status = match state
        .kernel
        .hand_registry
        .check_settings_availability(&hand_id)
    {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
            );
        }
    };

    // Find active instance config values (if any)
    let instance_config: std::collections::HashMap<String, serde_json::Value> = state
        .kernel
        .hand_registry
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
        .hand_registry
        .list_instances()
        .iter()
        .find(|i| i.hand_id == hand_id)
        .map(|i| i.instance_id);

    match instance_id {
        Some(id) => match state.kernel.hand_registry.update_config(id, config.clone()) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "hand_id": hand_id,
                    "instance_id": id,
                    "config": config,
                })),
            ),
            Err(e) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("{e}")})),
            ),
        },
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": format!("No active instance for hand: {hand_id}. Activate the hand first.")}),
            ),
        ),
    }
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
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Instance not found"})),
            );
        }
    };

    let def = match state.kernel.hand_registry.get_definition(&instance.hand_id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Hand definition not found"})),
            );
        }
    };

    let agent_id = match instance.agent_id {
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
            .memory
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
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Instance not found"})),
            );
        }
    };

    // 2. Get agent_id
    let agent_id = match instance.agent_id {
        Some(aid) => aid,
        None => {
            return (StatusCode::OK, Json(serde_json::json!({"active": false})));
        }
    };

    let agent_id_str = agent_id.to_string();

    // 3. Check if a browser session exists (without creating one)
    if !state.kernel.browser_ctx.has_session(&agent_id_str) {
        return (StatusCode::OK, Json(serde_json::json!({"active": false})));
    }

    // 4. Send ReadPage command to get page info
    let mut url = String::new();
    let mut title = String::new();
    let mut content = String::new();

    match state
        .kernel
        .browser_ctx
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
        .browser_ctx
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
    // Get configured servers from config
    let config_servers: Vec<serde_json::Value> = state
        .kernel
        .config
        .mcp_servers
        .iter()
        .map(|s| {
            let transport = serialize_mcp_transport(&s.transport);
            serde_json::json!({
                "name": s.name,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                "env": s.env,
            })
        })
        .collect();

    // Get connected servers and their tools from the live MCP connections
    let connections = state.kernel.mcp_connections.lock().await;
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
    // Find the configured entry by name
    let entry = state
        .kernel
        .config
        .mcp_servers
        .iter()
        .find(|s| s.name == name);

    let entry = match entry {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("MCP server '{}' not found", name)})),
            );
        }
    };

    let transport = serialize_mcp_transport(&entry.transport);

    let mut result = serde_json::json!({
        "name": entry.name,
        "transport": transport,
        "timeout_secs": entry.timeout_secs,
        "env": entry.env,
        "connected": false,
    });

    // Check live connection status
    let connections = state.kernel.mcp_connections.lock().await;
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'name' field"})),
            );
        }
    };

    if body.get("transport").is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'transport' field"})),
        );
    }

    // Validate by deserializing the body into McpServerConfigEntry
    let entry: librefang_types::config::McpServerConfigEntry = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid MCP server config: {e}")})),
            );
        }
    };

    // Check for duplicate name
    if state
        .kernel
        .config
        .mcp_servers
        .iter()
        .any(|s| s.name == name)
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("MCP server '{}' already exists", name)})),
        );
    }

    // Persist to config.toml
    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = upsert_mcp_server_config(&config_path, &entry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write config: {e}")})),
        );
    }

    // Trigger config reload
    let reload_status = match state.kernel.reload_config() {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit_log.record(
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
        .config
        .mcp_servers
        .iter()
        .any(|s| s.name == name)
    {
        return (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": t.t_args("api-error-mcp-not-found", &[("name", &name)])}),
            ),
        );
    }

    // Force the name in body to match the path parameter
    if let Some(obj) = body.as_object_mut() {
        obj.insert("name".to_string(), serde_json::json!(name));
    }

    if body.get("transport").is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-mcp-missing-transport")})),
        );
    }

    // Validate by deserializing
    let entry: librefang_types::config::McpServerConfigEntry = match serde_json::from_value(body) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": t.t_args("api-error-mcp-invalid-config", &[("error", &e.to_string())])}),
                ),
            );
        }
    };

    // Persist — upsert replaces an existing entry with the same name
    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = upsert_mcp_server_config(&config_path, &entry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-config-write-failed", &[("error", &e.to_string())])}),
            ),
        );
    }

    let reload_status = match state.kernel.reload_config() {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit_log.record(
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
        .config
        .mcp_servers
        .iter()
        .any(|s| s.name == name)
    {
        return (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": t.t_args("api-error-mcp-not-found", &[("name", &name)])}),
            ),
        );
    }

    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = remove_mcp_server_config(&config_path, &name) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({"error": t.t_args("api-error-config-write-failed", &[("error", &e.to_string())])}),
            ),
        );
    }

    let reload_status = match state.kernel.reload_config() {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit_log.record(
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
    if path.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'name' field"})),
            );
        }
    };

    // Validate name (alphanumeric + hyphens only)
    if !is_safe_component_name(&name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Skill name must contain only letters, numbers, hyphens, and underscores"}),
            ),
        );
    }

    let description = body["description"].as_str().unwrap_or("").to_string();
    let runtime = body["runtime"].as_str().unwrap_or("prompt_only");
    let prompt_context = body["prompt_context"].as_str().unwrap_or("").to_string();

    // Only allow prompt_only skills from the web UI for safety
    if runtime != "prompt_only" {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Only prompt_only skills can be created from the web UI"}),
            ),
        );
    }

    // Write skill.toml to ~/.librefang/skills/{name}/
    let skill_dir = state.kernel.config.home_dir.join("skills").join(&name);
    if skill_dir.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Skill '{}' already exists", name)})),
        );
    }

    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create skill directory: {e}")})),
        );
    }

    let toml_content = format!(
        "[skill]\nname = \"{}\"\ndescription = \"{}\"\nruntime = \"prompt_only\"\n\n[prompt]\ncontext = \"\"\"\n{}\n\"\"\"\n",
        name,
        description.replace('"', "\\\""),
        prompt_context
    );

    let toml_path = skill_dir.join("skill.toml");
    if let Err(e) = std::fs::write(&toml_path, &toml_content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write skill.toml: {e}")})),
        );
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
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_health;

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
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_health;

    // Look up the template first
    let template = match registry.get_template(&id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Integration '{}' not found", id)})),
            )
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
        .extension_registry
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'id' field"})),
            );
        }
    };

    // Scope the write lock so it's dropped before any .await
    let install_err = {
        let mut registry = state
            .kernel
            .extension_registry
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

    state.kernel.extension_health.register(&id);

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
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());
        registry.uninstall(&id).err()
    };

    if let Some(e) = uninstall_err {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }

    state.kernel.extension_health.unregister(&id);

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
            .extension_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        registry.is_installed(&id)
    };

    if !is_installed {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Integration '{}' not installed", id)})),
        );
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
    let health_entries = state.kernel.extension_health.all_health();
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
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
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_health;

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
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());

    let template = match registry.get_template(&name) {
        Some(t) => t.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Extension '{}' not found", name)})),
            );
        }
    };

    let installed = registry.get_installed(&name).cloned();
    let health = state.kernel.extension_health.get_health(&name);

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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing or empty 'name' field"})),
        );
    }

    // Scope the write lock so it's dropped before any .await
    let install_err = {
        let mut registry = state
            .kernel
            .extension_registry
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

    state.kernel.extension_health.register(&name);

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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing or empty 'name' field"})),
        );
    }

    // Scope the write lock
    let uninstall_err = {
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());
        registry.uninstall(&name).err()
    };

    if let Some(e) = uninstall_err {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }

    state.kernel.extension_health.unregister(&name);

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
