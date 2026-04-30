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
        .route("/skills/{name}", axum::routing::get(get_skill_detail))
        .route(
            "/skills/{name}/evolve/update",
            axum::routing::post(evolve_update_skill),
        )
        .route(
            "/skills/{name}/evolve/patch",
            axum::routing::post(evolve_patch_skill),
        )
        .route(
            "/skills/{name}/evolve/rollback",
            axum::routing::post(evolve_rollback_skill),
        )
        .route(
            "/skills/{name}/evolve/delete",
            axum::routing::post(evolve_delete_skill),
        )
        .route(
            "/skills/{name}/evolve/file",
            axum::routing::post(evolve_write_file).delete(evolve_remove_file),
        )
        .route("/skills/{name}/file", axum::routing::get(get_supporting_file))
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
        // ClawHub China mirror (mirror-cn.clawhub.com)
        .route("/clawhub-cn/search", axum::routing::get(clawhub_cn_search))
        .route("/clawhub-cn/browse", axum::routing::get(clawhub_cn_browse))
        .route(
            "/clawhub-cn/skill/{slug}",
            axum::routing::get(clawhub_cn_skill_detail),
        )
        .route(
            "/clawhub-cn/skill/{slug}/code",
            axum::routing::get(clawhub_cn_skill_code),
        )
        .route(
            "/clawhub-cn/install",
            axum::routing::post(clawhub_cn_install),
        )
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
            "/hands/{hand_id}/manifest",
            axum::routing::get(get_hand_manifest),
        )
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
        // Unified MCP server management — every MCP server lives as an
        // [[mcp_servers]] entry in config.toml, with an optional template_id
        // recording which catalog entry (if any) it was installed from.
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
        .route(
            "/mcp/servers/{name}/reconnect",
            axum::routing::post(reconnect_mcp_server_handler),
        )
        .route(
            "/mcp/servers/{name}/taint",
            axum::routing::patch(patch_mcp_server_taint),
        )
        // MCP OAuth auth endpoints (existing, unchanged)
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
        // Read-only catalog of installable MCP server templates
        .route("/mcp/catalog", axum::routing::get(list_mcp_catalog))
        .route(
            "/mcp/catalog/{id}",
            axum::routing::get(get_mcp_catalog_entry),
        )
        // Health + reload (covers all configured servers)
        .route("/mcp/health", axum::routing::get(mcp_health_handler))
        .route("/mcp/reload", axum::routing::post(reload_mcp_handler))
        // Read-only registry of named `[[taint_rules]]` for dashboard
        // validation (issue #3050 follow-up — typo'd rule_set names
        // would otherwise be silent no-ops in scanner).
        .route("/mcp/taint-rules", axum::routing::get(list_mcp_taint_rules))
        // Extensions — kept as dashboard-friendly aliases over the unified store.
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
pub async fn list_skills(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Use the kernel's LIVE registry so `skills.disabled` and
    // `skills.extra_dirs` from config.toml take effect on this
    // endpoint. Creating a fresh `SkillRegistry::new + load_all()`
    // here — as the code did previously — bypassed the operator
    // policy wired in `reload_skills`, making disabled skills show up
    // in the UI and extra_dirs invisible.
    let registry = state
        .kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());

    let category_filter = params.get("category").map(|s| s.as_str());

    // Collect all categories first (unaffected by the filter), then apply filter.
    // Category derivation lives in `librefang_skills::registry::derive_category`
    // so this list agrees with the kernel's prompt-builder grouping.
    let all_skills = registry.list();
    let mut categories = std::collections::BTreeSet::new();
    for s in &all_skills {
        categories.insert(librefang_skills::registry::derive_category(&s.manifest).to_string());
    }

    let skills: Vec<serde_json::Value> = all_skills
        .iter()
        .filter(|s| {
            let cat = librefang_skills::registry::derive_category(&s.manifest);
            match category_filter {
                Some(filter) => cat == filter,
                None => true,
            }
        })
        .map(|s| {
            let source = match &s.manifest.source {
                Some(librefang_skills::SkillSource::ClawHub { slug, version }) => {
                    serde_json::json!({"type": "clawhub", "slug": slug, "version": version})
                }
                Some(librefang_skills::SkillSource::ClawHubCn { slug, version }) => {
                    serde_json::json!({"type": "clawhub-cn", "slug": slug, "version": version})
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

    let categories_vec: Vec<String> = categories.into_iter().collect();
    Json(serde_json::json!({
        "skills": skills,
        "total": skills.len(),
        "categories": categories_vec,
    }))
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
            let version = "latest".to_string();

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
    // Route through the evolution module so user-initiated uninstall
    // picks up the per-skill lock and path-traversal check. The raw
    // `registry.remove()` path had neither — a concurrent evolve mid-rm
    // could see inconsistent state, and "/../" was accepted.
    let skills_dir = state.kernel.home_dir().join("skills");
    match librefang_skills::evolution::uninstall_skill(&skills_dir, &req.name) {
        Ok(result) => {
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "uninstalled",
                    "name": result.skill_name,
                    "message": result.message,
                })),
            )
        }
        Err(e) => evolution_err_to_response(e),
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
            let dir_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let skill_md_path = path.join("SKILL.md");
            if !skill_md_path.exists() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&skill_md_path) {
                if let Some(fm) = parse_skill_md_frontmatter(&content) {
                    let skill_name = if fm.name.is_empty() {
                        &dir_name
                    } else {
                        &fm.name
                    };
                    let installed_dir = state.kernel.home_dir().join("skills").join(skill_name);
                    let is_installed = installed_dir.exists();
                    skills.push(serde_json::json!({
                        "name": skill_name,
                        "description": fm.description,
                        "version": fm.version,
                        "author": fm.author,
                        "tags": fm.tags,
                        "is_installed": is_installed,
                    }));
                }
            }
        }
    }

    let total = skills.len();
    Json(serde_json::json!({ "skills": skills, "total": total }))
}

/// Parse YAML frontmatter from a SKILL.md file. Returns `(name, description)`.
/// Parsed YAML frontmatter from a SKILL.md.
///
/// Only `name` and `description` were ever required by the LibreFang
/// registry; `version` / `author` / `tags` are optional add-ons that
/// the dashboard's federated catalog UI surfaces when present. Missing
/// fields parse to `None` / `[]` rather than failing — old SKILL.md
/// files that pre-date the schema extension keep working.
#[derive(Debug, Default)]
struct SkillMdFrontmatter {
    name: String,
    description: String,
    version: Option<String>,
    author: Option<String>,
    tags: Vec<String>,
}

fn strip_yaml_value(raw: &str) -> String {
    // YAML scalar values can be wrapped in single or double quotes; strip
    // either form and trim whitespace.
    let trimmed = raw.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);
    unquoted.to_string()
}

fn parse_yaml_inline_list(raw: &str) -> Vec<String> {
    // Accept the two shapes that show up in the wild:
    //   tags: ["a", "b"]
    //   tags: [a, b]
    // Anything else (block-list `- item` form, multi-line) is left for
    // a future iteration; SKILL.md frontmatters in the registry only
    // ever use the inline form today.
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(strip_yaml_value)
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_skill_md_frontmatter(content: &str) -> Option<SkillMdFrontmatter> {
    let trimmed = content.trim();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_open = &trimmed[3..];
    let close = after_open.find("---")?;
    let frontmatter = &after_open[..close];
    let mut fm = SkillMdFrontmatter::default();
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            fm.name = strip_yaml_value(val);
        } else if let Some(val) = line.strip_prefix("description:") {
            fm.description = strip_yaml_value(val);
        } else if let Some(val) = line.strip_prefix("version:") {
            let v = strip_yaml_value(val);
            if !v.is_empty() {
                fm.version = Some(v);
            }
        } else if let Some(val) = line.strip_prefix("author:") {
            let a = strip_yaml_value(val);
            if !a.is_empty() {
                fm.author = Some(a);
            }
        } else if let Some(val) = line.strip_prefix("tags:") {
            fm.tags = parse_yaml_inline_list(val);
        }
    }
    if fm.name.is_empty() && fm.description.is_empty() {
        return None;
    }
    Some(fm)
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
// ClawHub China mirror endpoints (mirror-cn.clawhub.com)
// ---------------------------------------------------------------------------

const CLAWHUB_CN_BASE_URL: &str = "https://mirror-cn.clawhub.com/api/v1";

/// GET /api/clawhub-cn/search — Search ClawHub via the China mirror.
pub async fn clawhub_cn_search(
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

    let cache_key = format!("cn:search:{}:{}", query, limit);
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub-cn");
    let client = librefang_skills::clawhub::ClawHubClient::with_url(CLAWHUB_CN_BASE_URL, cache_dir);

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
            let resp = serde_json::json!({"items": items, "next_cursor": null});
            state
                .clawhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("ClawHub CN search failed: {msg}");
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

/// GET /api/clawhub-cn/browse — Browse ClawHub via the China mirror.
pub async fn clawhub_cn_browse(
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

    let cache_key = format!("cn:browse:{:?}:{}:{}", sort, limit, cursor.unwrap_or(""));
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub-cn");
    let client = librefang_skills::clawhub::ClawHubClient::with_url(CLAWHUB_CN_BASE_URL, cache_dir);

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
            tracing::warn!("ClawHub CN browse failed: {msg}");
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

/// GET /api/clawhub-cn/skill/{slug} — Skill detail via the China mirror.
pub async fn clawhub_cn_skill_detail(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub-cn");
    let client = librefang_skills::clawhub::ClawHubClient::with_url(CLAWHUB_CN_BASE_URL, cache_dir);

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

/// GET /api/clawhub-cn/skill/{slug}/code — Skill source code via the China mirror.
pub async fn clawhub_cn_skill_code(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub-cn");
    let client = librefang_skills::clawhub::ClawHubClient::with_url(CLAWHUB_CN_BASE_URL, cache_dir);

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

/// POST /api/clawhub-cn/install — Install a skill from the ClawHub China mirror.
pub async fn clawhub_cn_install(
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

    let cache_dir = state.kernel.home_dir().join(".cache").join("clawhub-cn");
    let client = librefang_skills::clawhub::ClawHubClient::with_url(CLAWHUB_CN_BASE_URL, cache_dir);

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
            // Patch source provenance to ClawHubCn so the skill registry knows
            // this skill was installed from ClawHub and can surface update/version info.
            let skill_dir = skills_dir.join(&req.slug);
            let manifest_path = skill_dir.join("skill.toml");
            if manifest_path.exists() {
                match std::fs::read_to_string(&manifest_path) {
                    Ok(toml_str) => {
                        match toml::from_str::<librefang_skills::SkillManifest>(&toml_str) {
                            Ok(mut manifest) => {
                                manifest.source = Some(librefang_skills::SkillSource::ClawHubCn {
                                    slug: req.slug.clone(),
                                    version: result.version.clone(),
                                });
                                match toml::to_string_pretty(&manifest) {
                                    Ok(updated) => {
                                        if let Err(e) = std::fs::write(&manifest_path, updated) {
                                            tracing::warn!(
                                                slug = %req.slug,
                                                path = %manifest_path.display(),
                                                "Failed to write provenance to skill.toml: {e}"
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            slug = %req.slug,
                                            "Failed to serialize skill manifest for provenance patch: {e}"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    slug = %req.slug,
                                    "Failed to parse skill.toml for provenance patch: {e}"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            slug = %req.slug,
                            path = %manifest_path.display(),
                            "Failed to read skill.toml for provenance patch: {e}"
                        );
                    }
                }
            }

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
            tracing::warn!("ClawHub CN install failed: {msg}");
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

/// GET /api/hands/{hand_id}/manifest — Return the hand's HAND.toml as text.
///
/// Reads the on-disk HAND.toml from either the registry or workspaces dir
/// so comments and original formatting survive. Falls back to serializing
/// the in-memory `HandDefinition` if the file isn't on disk (e.g. installed
/// programmatically), so the endpoint always has something to return for
/// any hand the registry knows about.
#[utoipa::path(
    get,
    path = "/api/hands/{hand_id}/manifest",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    responses(
        (status = 200, description = "HAND.toml content", content_type = "application/toml")
    )
)]
pub async fn get_hand_manifest(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    use axum::body::Body;

    // Gate the filesystem lookup on registry membership so a crafted
    // hand_id can't be used to probe for `**/HAND.toml` paths under the
    // home dir. Mirrors the `get_hand` pattern above.
    let definition = match state.kernel.hands().get_definition(&hand_id) {
        Some(def) => def,
        None => {
            return ApiErrorResponse::not_found(format!("Hand not found: {hand_id}"))
                .into_json_tuple()
                .into_response();
        }
    };

    let home = state.kernel.home_dir();
    // Two install layouts that scan_hands_dir actually walks
    // (librefang-hands/src/registry.rs:165). Anything else is a
    // codebase inconsistency that wouldn't make it into the registry,
    // so the gate above would already 404 it before we get here.
    let candidates = [
        home.join("registry")
            .join("hands")
            .join(&hand_id)
            .join("HAND.toml"),
        home.join("workspaces").join(&hand_id).join("HAND.toml"),
    ];

    let mut toml_content: Option<String> = None;
    for path in &candidates {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                toml_content = Some(content);
                break;
            }
        }
    }

    // Fall back to re-serialising the in-memory definition so hands
    // installed via API (no on-disk HAND.toml) still get a useful
    // payload. Loses comments / formatting but preserves structure.
    if toml_content.is_none() {
        match toml::to_string_pretty(&definition) {
            Ok(s) => toml_content = Some(s),
            Err(e) => {
                return ApiErrorResponse::internal(format!(
                    "Failed to serialize hand definition: {e}"
                ))
                .into_json_tuple()
                .into_response();
            }
        }
    }

    let text = toml_content.expect("toml_content set in fallback above");
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/toml")],
        Body::from(text),
    )
        .into_response()
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

        // For winget on Windows, add --accept flags to avoid interactive prompts
        let final_cmd = if cfg!(windows) && cmd.starts_with("winget ") {
            format!("{cmd} --accept-source-agreements --accept-package-agreements")
        } else {
            cmd.to_string()
        };

        // Guard against shell injection: reject commands that contain shell
        // metacharacters that are never needed in legitimate package-manager
        // install strings (semicolons, pipes, backticks, redirects, etc.).
        if final_cmd.contains(|c: char| {
            matches!(
                c,
                ';' | '|' | '&' | '$' | '`' | '>' | '<' | '(' | ')' | '{' | '}' | '\n' | '\r'
            )
        }) {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "error",
                "command": final_cmd,
                "message": "Install command contains disallowed shell metacharacters and was rejected for security reasons",
            }));
            continue;
        }

        // Split into program + arguments and exec directly — no shell involved.
        // This eliminates the sh -c / cmd /C injection vector entirely.
        let parts: Vec<&str> = final_cmd.split_whitespace().collect();
        if parts.is_empty() {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "error",
                "command": final_cmd,
                "message": "Install command is empty",
            }));
            continue;
        }
        let program = parts[0];
        let args = &parts[1..];

        tracing::info!(hand = %hand_id, dep = %req.key, cmd = %final_cmd, "Auto-installing dependency");

        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tokio::process::Command::new(program)
                .args(args)
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
                    // `std::env::set_var` is not thread-safe in an async context;
                    // push to a blocking thread to avoid UB in the tokio runtime.
                    let new_path_clone = new_path.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        // SAFETY: single mutation on a dedicated blocking thread.
                        unsafe { std::env::set_var("PATH", &new_path_clone) };
                    })
                    .await;
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

    // Set in current process.
    // `std::env::set_var` is not thread-safe in an async context; delegate to
    // a blocking thread to avoid UB in the multithreaded tokio runtime.
    {
        let env_key_clone = env_key.clone();
        let value_clone = value.clone();
        let _ = tokio::task::spawn_blocking(move || {
            // SAFETY: single mutation on a dedicated blocking thread.
            unsafe { std::env::set_var(&env_key_clone, &value_clone) };
        })
        .await;
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
                    owner_notice: result.owner_notice,
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
        (status = 200, description = "List configured named taint rule sets", body = serde_json::Value)
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
                "template_id": s.template_id,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                "env": s.env,
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
                Some(librefang_extensions::McpStatus::Ready),
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
        "template_id": entry.template_id,
        "transport": transport,
        "timeout_secs": entry.timeout_secs,
        "env": entry.env,
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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Add a new MCP server configuration", body = serde_json::Value)
    )
)]
pub async fn add_mcp_server(
    State(state): State<Arc<AppState>>,
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

        let catalog = state
            .kernel
            .mcp_catalog()
            .read()
            .unwrap_or_else(|e| e.into_inner());
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
        if state
            .kernel
            .config_ref()
            .mcp_servers
            .iter()
            .any(|s| s.name == prospective_name)
        {
            return ApiErrorResponse::conflict(format!(
                "MCP server '{prospective_name}' already exists"
            ))
            .into_json_tuple();
        }

        // Credential resolver: dotenv + vault (if unlocked)
        let home = state.kernel.home_dir().to_path_buf();
        let dotenv_path = home.join(".env");
        let vault_path = home.join("vault.enc");
        let vault = if vault_path.exists() {
            let mut v = librefang_extensions::vault::CredentialVault::new(vault_path);
            if v.unlock().is_ok() {
                Some(v)
            } else {
                None
            }
        } else {
            None
        };
        let mut resolver =
            librefang_extensions::credentials::CredentialResolver::new(vault, Some(&dotenv_path));

        // Ephemeral catalog to feed the installer (it takes &McpCatalog).
        let mut cat = librefang_extensions::catalog::McpCatalog::new(&home);
        cat.load(&home);
        let result = match librefang_extensions::installer::install_integration(
            &cat,
            &mut resolver,
            &entry.id,
            &creds,
        ) {
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

    // Establish connection to the newly added server in the background.
    let kernel = std::sync::Arc::clone(&state.kernel);
    tokio::spawn(async move { kernel.connect_mcp_servers().await });

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

    // Disconnect the old connection so connect_mcp_servers picks up the new config.
    state.kernel.disconnect_mcp_server(&name).await;
    let kernel = std::sync::Arc::clone(&state.kernel);
    tokio::spawn(async move { kernel.connect_mcp_servers().await });

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

/// PATCH /api/mcp/servers/{name}/taint — Partial update of taint settings.
///
/// Accepts a body of `{ "taint_scanning"?: bool, "taint_policy"?: McpTaintPolicy }`
/// and merges it into the existing entry. Unlike PUT this does NOT require
/// the caller to round-trip every other server field (transport, env, etc.) —
/// the dashboard taint editor in particular needs only these two fields and
/// shouldn't risk silently dropping unrelated fields it doesn't render.
// `McpTaintPolicy` (in `librefang-types`) doesn't carry `utoipa::ToSchema`,
// so deriving `ToSchema` here would fail. The OpenAPI annotation uses
// `serde_json::Value` for the body schema, which keeps the spec accurate
// without forcing a downstream derive.
#[derive(serde::Deserialize)]
pub(crate) struct PatchMcpTaintRequest {
    /// When supplied, replaces `taint_scanning` on the existing entry.
    #[serde(default)]
    pub taint_scanning: Option<bool>,
    /// When supplied, replaces `taint_policy` on the existing entry.
    /// Pass `{}` (empty object) to clear all per-tool policies; pass `null`
    /// (or omit) to leave existing policies untouched.
    #[serde(default)]
    pub taint_policy: Option<librefang_types::config::McpTaintPolicy>,
}

#[utoipa::path(
    patch,
    path = "/api/mcp/servers/{name}/taint",
    tag = "mcp",
    params(("name" = String, Path, description = "Server name")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Taint settings updated", body = serde_json::Value),
        (status = 404, description = "Server not found", body = serde_json::Value),
    )
)]
#[allow(private_interfaces)]
pub async fn patch_mcp_server_taint(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<PatchMcpTaintRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    // Locate and clone the existing entry so we mutate a fresh copy that's
    // safe to pass to upsert_mcp_server_config without touching the live
    // config until persistence succeeds.
    let mut entry = match state
        .kernel
        .config_ref()
        .mcp_servers
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

    // Reconnect so the new taint_policy snapshot reaches the live
    // `McpServerConfig.taint_policy` field. The shared `taint_rules_swap`
    // already updates via `reload_config` without a reconnect.
    state.kernel.disconnect_mcp_server(&name).await;
    let kernel = std::sync::Arc::clone(&state.kernel);
    tokio::spawn(async move { kernel.connect_mcp_servers().await });

    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("mcp_server taint updated: {name}"),
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
        // Propagate parse errors instead of silently defaulting to an empty
        // table, which would overwrite every unrelated section when we write
        // back. A malformed config.toml should surface to the caller.
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
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
        // Propagate parse errors instead of silently defaulting to an empty
        // table, which would destroy every unrelated section when we write
        // back after the retain().
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
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
    if let Some(resp) = reject_if_frozen(&state) {
        return resp;
    }
    let name = match body["name"].as_str() {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing or empty 'name' field")
                .into_json_tuple();
        }
    };

    let description = match body["description"].as_str() {
        Some(d) if !d.trim().is_empty() => d.trim().to_string(),
        _ => {
            return ApiErrorResponse::bad_request("Missing or empty 'description' field")
                .into_json_tuple();
        }
    };

    let prompt_context = body["prompt_context"].as_str().unwrap_or("").to_string();
    let tags: Vec<String> = body["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Use the evolution module for safe, validated skill creation
    let skills_dir = state.kernel.home_dir().join("skills");
    match librefang_skills::evolution::create_skill(
        &skills_dir,
        &name,
        &description,
        &prompt_context,
        tags,
        Some("dashboard"),
    ) {
        Ok(result) => {
            audit_evolve(&state, "create", &result.skill_name, &result.message);
            // Hot-reload skills so the new skill is available immediately
            state.kernel.reload_skills();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "created",
                    "name": result.skill_name,
                    "version": result.version,
                    "message": result.message,
                })),
            )
        }
        Err(e) => {
            ApiErrorResponse::bad_request(format!("Failed to create skill: {e}")).into_json_tuple()
        }
    }
}

/// Get detailed information about a specific skill, including linked files,
/// tags, evolution history, and readiness status.
#[utoipa::path(
    get,
    path = "/api/skills/{name}",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    responses(
        (status = 200, description = "Skill detail with evolution history", body = serde_json::Value),
        (status = 404, description = "Skill not found")
    )
)]
pub async fn get_skill_detail(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let registry = state
        .kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());

    let skill = match registry.get(&name) {
        Some(s) => s,
        None => {
            return ApiErrorResponse::not_found(format!("Skill '{name}' not found"))
                .into_json_tuple();
        }
    };

    let manifest = &skill.manifest;

    // List linked files
    let linked_files = librefang_skills::evolution::list_supporting_files(skill);

    // Get evolution metadata
    let evolution_meta = librefang_skills::evolution::get_evolution_info(skill);

    // Build response
    let tools: Vec<serde_json::Value> = manifest
        .tools
        .provided
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": manifest.skill.name,
            "version": manifest.skill.version,
            "description": manifest.skill.description,
            "author": manifest.skill.author,
            "license": manifest.skill.license,
            "tags": manifest.skill.tags,
            "runtime": format!("{:?}", manifest.runtime.runtime_type),
            "tools": tools,
            "has_prompt_context": manifest.prompt_context.is_some(),
            "prompt_context_length": manifest.prompt_context.as_ref().map(|c| c.len()).unwrap_or(0),
            "source": manifest.source,
            "enabled": skill.enabled,
            "path": skill.path.to_string_lossy(),
            "linked_files": linked_files,
            "evolution": {
                "versions": evolution_meta.versions,
                "use_count": evolution_meta.use_count,
                "evolution_count": evolution_meta.evolution_count,
                "mutation_count": evolution_meta.mutation_count,
            },
            // Full prompt_context text so the dashboard Update modal
            // can pre-fill the editor. Capped at MAX_PROMPT_CONTEXT_CHARS
            // by the evolution module on write, so safe to inline here.
            "prompt_context": manifest.prompt_context,
        })),
    )
}

// ── Skill evolution handlers ───────────────────────────────────────────
//
// Each handler looks the skill up by name, clones the InstalledSkill
// snapshot so we don't hold the RwLock across the await, delegates to
// the evolution module, then reloads the registry so the change is
// immediately visible on subsequent requests.

fn clone_installed_skill(
    state: &Arc<AppState>,
    name: &str,
) -> Result<librefang_skills::InstalledSkill, (StatusCode, Json<serde_json::Value>)> {
    // Try the live registry first. Fall back to disk for skills that
    // exist on the filesystem but haven't been hot-reloaded into the
    // in-memory registry yet — e.g. after a just-completed
    // `skill_evolve_create` from within the same dashboard session.
    {
        let registry = state
            .kernel
            .skill_registry_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(s) = registry.get(name) {
            return Ok(s.clone());
        }
    }
    let skills_dir = state.kernel.home_dir().join("skills");
    librefang_skills::evolution::load_installed_skill_from_disk(&skills_dir, name).map_err(|e| {
        match e {
            librefang_skills::SkillError::NotFound(_) => {
                ApiErrorResponse::not_found(format!("Skill '{name}' not found")).into_json_tuple()
            }
            other => {
                ApiErrorResponse::bad_request(format!("Skill '{name}': {other}")).into_json_tuple()
            }
        }
    })
}

/// Reject dashboard/CLI evolve calls when the kernel is in Stable mode
/// (registry frozen). Mirrors the agent-tool gate in `tool_runner.rs`
/// — evolution writes to disk directly, so the frozen check on its
/// own only stops the in-memory reload. Without this guard the
/// dashboard would happily mutate skills that the operator pinned via
/// Stable mode.
fn reject_if_frozen(state: &Arc<AppState>) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let registry = state
        .kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    if registry.is_frozen() {
        Some(
            ApiErrorResponse::bad_request(
                "Skill evolution is disabled in Stable mode (registry frozen)",
            )
            .into_json_tuple(),
        )
    } else {
        None
    }
}

fn evolution_err_to_response(
    e: librefang_skills::SkillError,
) -> (StatusCode, Json<serde_json::Value>) {
    use librefang_skills::SkillError as E;
    let msg = e.to_string();
    match e {
        E::NotFound(_) => ApiErrorResponse::not_found(msg).into_json_tuple(),
        E::AlreadyInstalled(_) => ApiErrorResponse::conflict(msg).into_json_tuple(),
        E::InvalidManifest(_) | E::SecurityBlocked(_) | E::YamlParse(_) | E::TomlParse(_) => {
            ApiErrorResponse::bad_request(msg).into_json_tuple()
        }
        _ => ApiErrorResponse::internal(msg).into_json_tuple(),
    }
}

fn evolution_ok_response(
    result: librefang_skills::evolution::EvolutionResult,
) -> (StatusCode, Json<serde_json::Value>) {
    // Serialize the whole struct so dashboard consumers pick up the
    // full set of EvolutionResult fields automatically
    // (match_strategy, match_count, evolution_count, mutation_count,
    // use_count) instead of relying on this handler being updated
    // every time a new field is added.
    (
        StatusCode::OK,
        Json(serde_json::to_value(result).unwrap_or(serde_json::json!({}))),
    )
}

/// GET /api/skills/{name}/file?path=... — return the contents of a
/// supporting file so the dashboard can render it. Share the same
/// security rules as `skill_read_file` (no absolute paths, no traversal,
/// must resolve within the skill directory, size-capped).
#[utoipa::path(
    get,
    path = "/api/skills/{name}/file",
    tag = "skills",
    params(
        ("name" = String, Path, description = "Skill name"),
        ("path" = String, Query, description = "Relative file path inside the skill directory")
    ),
    responses(
        (status = 200, description = "File contents", body = serde_json::Value),
        (status = 400, description = "Invalid path"),
        (status = 404, description = "Skill or file not found")
    )
)]
pub async fn get_supporting_file(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(rel_path) = params.get("path") else {
        return ApiErrorResponse::bad_request("Missing 'path' query parameter").into_json_tuple();
    };
    // Reject absolute paths and traversal early — defense in depth even
    // before canonicalisation runs. Check by `Path::Component` rather
    // than a substring scan: the old `contains("..")` rejected legit
    // names like `config..bak.md` and `..prefix.txt`, while still
    // missing the bare Windows-style `foo\..\bar` (components are
    // resolved differently).
    if rel_path.is_empty() || std::path::Path::new(rel_path).is_absolute() {
        return ApiErrorResponse::bad_request(format!("Invalid path: {rel_path}"))
            .into_json_tuple();
    }
    if std::path::Path::new(rel_path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return ApiErrorResponse::bad_request(format!(
            "Path traversal ('..') is not allowed: {rel_path}"
        ))
        .into_json_tuple();
    }

    let skill = match clone_installed_skill(&state, &name) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let requested = skill.path.join(rel_path);
    let Ok(canonical) = requested.canonicalize() else {
        return ApiErrorResponse::not_found(format!("File not found: {rel_path}"))
            .into_json_tuple();
    };
    let Ok(root) = skill.path.canonicalize() else {
        return ApiErrorResponse::internal("Skill directory missing").into_json_tuple();
    };
    if !canonical.starts_with(&root) {
        return ApiErrorResponse::bad_request(format!(
            "'{rel_path}' is outside the skill directory"
        ))
        .into_json_tuple();
    }

    // Size cap: even supporting files up to 1 MiB can exceed response
    // limits in the browser. Truncate and advertise.
    const MAX_BYTES: usize = 256 * 1024;
    let content = match std::fs::read_to_string(&canonical) {
        Ok(s) => s,
        Err(e) => {
            return ApiErrorResponse::internal(format!("Failed to read file: {e}"))
                .into_json_tuple();
        }
    };
    let (truncated, body) = if content.len() > MAX_BYTES {
        let cut = content
            .char_indices()
            .take_while(|(i, _)| *i < MAX_BYTES)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        (true, content[..cut].to_string())
    } else {
        (false, content)
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": name,
            "path": rel_path,
            "content": body,
            "truncated": truncated,
        })),
    )
}

/// Record a successful skill evolution in the audit trail. All
/// dashboard-initiated mutations go through this so the audit log has a
/// tamper-evident record of every `/api/skills/.../evolve/*` action.
fn audit_evolve(state: &Arc<AppState>, action: &str, skill_name: &str, detail: &str) {
    state.kernel.audit().record(
        // Dashboard calls don't have an agent_id — use a distinctive
        // actor so audit readers can tell user actions from agent ones.
        "dashboard".to_string(),
        librefang_runtime::audit::AuditAction::AgentMessage,
        format!("skill_evolve:{action}:{skill_name}"),
        detail.to_string(),
    );
}

/// POST /api/skills/{name}/evolve/update — full-rewrite prompt_context.
#[utoipa::path(
    post,
    path = "/api/skills/{name}/evolve/update",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Skill updated", body = serde_json::Value),
        (status = 400, description = "Invalid request / security-blocked content"),
        (status = 404, description = "Skill not found")
    )
)]
pub async fn evolve_update_skill(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_frozen(&state) {
        return resp;
    }
    let Some(prompt_context) = body["prompt_context"].as_str() else {
        return ApiErrorResponse::bad_request("Missing 'prompt_context' field").into_json_tuple();
    };
    let changelog = body["changelog"].as_str().unwrap_or("").trim();
    if changelog.is_empty() {
        return ApiErrorResponse::bad_request("Missing 'changelog' field").into_json_tuple();
    }
    let skill = match clone_installed_skill(&state, &name) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match librefang_skills::evolution::update_skill(
        &skill,
        prompt_context,
        changelog,
        Some("dashboard"),
    ) {
        Ok(r) => {
            audit_evolve(&state, "update", &r.skill_name, changelog);
            state.kernel.reload_skills();
            evolution_ok_response(r)
        }
        Err(e) => evolution_err_to_response(e),
    }
}

/// POST /api/skills/{name}/evolve/patch — fuzzy find-and-replace.
#[utoipa::path(
    post,
    path = "/api/skills/{name}/evolve/patch",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Skill patched", body = serde_json::Value),
        (status = 400, description = "Invalid request / fuzzy match failed"),
        (status = 404, description = "Skill not found")
    )
)]
pub async fn evolve_patch_skill(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_frozen(&state) {
        return resp;
    }
    let Some(old_string) = body["old_string"].as_str() else {
        return ApiErrorResponse::bad_request("Missing 'old_string' field").into_json_tuple();
    };
    let Some(new_string) = body["new_string"].as_str() else {
        return ApiErrorResponse::bad_request("Missing 'new_string' field").into_json_tuple();
    };
    let changelog = body["changelog"].as_str().unwrap_or("").trim();
    if changelog.is_empty() {
        return ApiErrorResponse::bad_request("Missing 'changelog' field").into_json_tuple();
    }
    let replace_all = body["replace_all"].as_bool().unwrap_or(false);
    let skill = match clone_installed_skill(&state, &name) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match librefang_skills::evolution::patch_skill(
        &skill,
        old_string,
        new_string,
        changelog,
        replace_all,
        Some("dashboard"),
    ) {
        Ok(r) => {
            audit_evolve(&state, "patch", &r.skill_name, changelog);
            state.kernel.reload_skills();
            evolution_ok_response(r)
        }
        Err(e) => evolution_err_to_response(e),
    }
}

/// POST /api/skills/{name}/evolve/rollback — roll back to previous version.
#[utoipa::path(
    post,
    path = "/api/skills/{name}/evolve/rollback",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    responses(
        (status = 200, description = "Skill rolled back", body = serde_json::Value),
        (status = 404, description = "Skill or snapshot not found")
    )
)]
pub async fn evolve_rollback_skill(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_frozen(&state) {
        return resp;
    }
    let skill = match clone_installed_skill(&state, &name) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match librefang_skills::evolution::rollback_skill(&skill, Some("dashboard")) {
        Ok(r) => {
            audit_evolve(
                &state,
                "rollback",
                &r.skill_name,
                "rolled back to previous version",
            );
            state.kernel.reload_skills();
            evolution_ok_response(r)
        }
        Err(e) => evolution_err_to_response(e),
    }
}

/// POST /api/skills/{name}/evolve/delete — delete a locally-evolved skill.
#[utoipa::path(
    post,
    path = "/api/skills/{name}/evolve/delete",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    responses(
        (status = 200, description = "Skill deleted", body = serde_json::Value),
        (status = 400, description = "Non-local skill — deletion refused"),
        (status = 404, description = "Skill not found")
    )
)]
pub async fn evolve_delete_skill(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_frozen(&state) {
        return resp;
    }
    let skills_dir = state.kernel.home_dir().join("skills");
    match librefang_skills::evolution::delete_skill(&skills_dir, &name) {
        Ok(r) => {
            audit_evolve(&state, "delete", &r.skill_name, &r.message);
            state.kernel.reload_skills();
            evolution_ok_response(r)
        }
        Err(e) => evolution_err_to_response(e),
    }
}

/// POST /api/skills/{name}/evolve/file — add a supporting file.
#[utoipa::path(
    post,
    path = "/api/skills/{name}/evolve/file",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "File written", body = serde_json::Value),
        (status = 400, description = "Invalid path / over size limit"),
        (status = 404, description = "Skill not found")
    )
)]
pub async fn evolve_write_file(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_frozen(&state) {
        return resp;
    }
    let Some(path) = body["path"].as_str() else {
        return ApiErrorResponse::bad_request("Missing 'path' field").into_json_tuple();
    };
    let Some(content) = body["content"].as_str() else {
        return ApiErrorResponse::bad_request("Missing 'content' field").into_json_tuple();
    };
    let skill = match clone_installed_skill(&state, &name) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match librefang_skills::evolution::write_supporting_file(&skill, path, content) {
        Ok(r) => {
            audit_evolve(&state, "write_file", &r.skill_name, path);
            state.kernel.reload_skills();
            evolution_ok_response(r)
        }
        Err(e) => evolution_err_to_response(e),
    }
}

/// DELETE /api/skills/{name}/evolve/file — remove a supporting file.
/// Path is supplied via the `?path=` query string since axum's DELETE
/// body handling is inconsistent across clients.
#[utoipa::path(
    delete,
    path = "/api/skills/{name}/evolve/file",
    tag = "skills",
    params(
        ("name" = String, Path, description = "Skill name"),
        ("path" = String, Query, description = "Relative path of the file to remove")
    ),
    responses(
        (status = 200, description = "File removed", body = serde_json::Value),
        (status = 400, description = "Missing 'path' parameter"),
        (status = 404, description = "Skill or file not found")
    )
)]
pub async fn evolve_remove_file(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_frozen(&state) {
        return resp;
    }
    let Some(path) = params.get("path") else {
        return ApiErrorResponse::bad_request("Missing 'path' query parameter").into_json_tuple();
    };
    let skill = match clone_installed_skill(&state, &name) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match librefang_skills::evolution::remove_supporting_file(&skill, path) {
        Ok(r) => {
            audit_evolve(&state, "remove_file", &r.skill_name, path);
            state.kernel.reload_skills();
            evolution_ok_response(r)
        }
        Err(e) => evolution_err_to_response(e),
    }
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

/// Escape a value for safe storage in a `.env` file.
///
/// If a value contains literal newlines the raw `KEY=value\nEXTRA=junk` text
/// would be parsed as two separate keys by every dotenv reader. Backslashes
/// must be doubled so they are not misread as escape sequences on read-back.
fn escape_env_value(value: &str) -> String {
    value
        .replace('\\', "\\\\") // must come first to avoid double-escaping
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Write or update a key in the secrets.env file.
/// File format: one `KEY=value` per line. Existing keys are overwritten.
/// Values containing newlines or backslashes are escaped so they stay on a
/// single line and round-trip correctly through dotenv parsers.
pub(crate) fn write_secret_env(
    path: &std::path::Path,
    key: &str,
    value: &str,
) -> Result<(), std::io::Error> {
    validate_static_file_path(path, "secrets.env")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    if key.contains('\n') || key.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret key must not contain newline characters",
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret value must not contain newline characters",
        ));
    }
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

    // Add new line — escape the value so embedded newlines/backslashes cannot
    // corrupt the file structure.
    let escaped = escape_env_value(value);
    lines.push(format!("{key}={escaped}"));

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
///
/// Uses `toml_edit::DocumentMut` to preserve comments, key ordering, and
/// formatting of unrelated sections (providers, agents, etc.). The previous
/// `toml::Value` round-trip silently rewrote the entire file on every
/// channel write — see issue #3183. Callers must hold
/// `AppState::config_write_lock` to serialize against `POST /api/config/set`,
/// which performs an asymmetric read-modify-write on the same file.
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

    let mut doc: toml_edit::DocumentMut = if content.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        content.parse()?
    };

    // Ensure [channels] table exists
    if !doc.contains_table("channels") {
        doc["channels"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let channels_table = doc["channels"]
        .as_table_mut()
        .ok_or("channels is not a table")?;

    // Build channel sub-table with correct TOML types
    let mut ch_table = toml_edit::Table::new();
    for (k, (v, ft)) in fields {
        let item = match ft {
            FieldType::Number => {
                if let Ok(n) = v.parse::<i64>() {
                    toml_edit::value(n)
                } else {
                    toml_edit::value(v.clone())
                }
            }
            FieldType::List => {
                // Always store list items as strings so that numeric IDs
                // (e.g. Discord guild snowflakes, Telegram user IDs) are
                // deserialized correctly into Vec<String> config fields.
                let mut arr = toml_edit::Array::new();
                for s in v.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    arr.push(s);
                }
                toml_edit::value(arr)
            }
            _ => toml_edit::value(v.clone()),
        };
        ch_table.insert(k, item);
    }
    channels_table.insert(channel_name, toml_edit::Item::Table(ch_table));

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(config_path, doc.to_string())?;
    Ok(())
}

/// Remove a `[channels.<name>]` section from config.toml.
///
/// Mirrors `upsert_channel_config`: format-preserving via `toml_edit`, and
/// callers must hold `AppState::config_write_lock`.
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

    let mut doc: toml_edit::DocumentMut = content.parse()?;

    if let Some(channels) = doc.get_mut("channels").and_then(|i| i.as_table_mut()) {
        channels.remove(channel_name);
    }

    std::fs::write(config_path, doc.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// MCP catalog + reconnect + health + reload endpoints
// ---------------------------------------------------------------------------

/// Serialize a single catalog transport for API output.
fn serialize_catalog_transport(t: &librefang_extensions::McpCatalogTransport) -> serde_json::Value {
    match t {
        librefang_extensions::McpCatalogTransport::Stdio { command, args } => {
            serde_json::json!({ "type": "stdio", "command": command, "args": args })
        }
        librefang_extensions::McpCatalogTransport::Sse { url } => {
            serde_json::json!({ "type": "sse", "url": url })
        }
        librefang_extensions::McpCatalogTransport::Http { url } => {
            serde_json::json!({ "type": "http", "url": url })
        }
    }
}

/// Collect catalog ids that are "already installed" for the purposes of
/// the catalog list/detail endpoints. Includes both `template_id` matches
/// (server was installed via the template) and `name` matches (manually
/// configured server occupies the catalog entry's id), so the endpoints
/// agree with `add_mcp_server`'s 409 name-collision guard and the UI
/// doesn't offer Install on entries that will definitely fail.
fn collect_installed_catalog_ids(state: &Arc<AppState>) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for s in state.kernel.config_ref().mcp_servers.iter() {
        if let Some(tid) = s.template_id.clone() {
            ids.insert(tid);
        }
        ids.insert(s.name.clone());
    }
    ids
}

fn render_catalog_entry(
    entry: &librefang_extensions::McpCatalogEntry,
    installed_template_ids: &std::collections::HashSet<String>,
    lang: &str,
) -> serde_json::Value {
    // Pick the localized override (with `zh-TW` → `zh` soft fallback) and
    // fall back to the English fields per-string when no entry / field is
    // present.
    let i18n_entry = entry.i18n.get(lang).or_else(|| {
        lang.split_once('-')
            .and_then(|(base, _)| entry.i18n.get(base))
    });
    let name = i18n_entry
        .and_then(|e| e.name.as_deref())
        .unwrap_or(&entry.name);
    let description = i18n_entry
        .and_then(|e| e.description.as_deref())
        .unwrap_or(&entry.description);
    let setup_instructions = i18n_entry
        .and_then(|e| e.setup_instructions.as_deref())
        .unwrap_or(&entry.setup_instructions);

    serde_json::json!({
        "id": entry.id,
        "name": name,
        "description": description,
        "icon": entry.icon,
        "category": entry.category.to_string(),
        "installed": installed_template_ids.contains(&entry.id),
        "tags": entry.tags,
        "transport": serialize_catalog_transport(&entry.transport),
        "required_env": entry.required_env.iter().map(|e| serde_json::json!({
            "name": e.name,
            "label": e.label,
            "help": e.help,
            "is_secret": e.is_secret,
            "get_url": e.get_url,
        })).collect::<Vec<_>>(),
        "has_oauth": entry.oauth.is_some(),
        "setup_instructions": setup_instructions,
    })
}

/// GET /api/mcp/catalog — List all installable MCP catalog entries.
#[utoipa::path(
    get,
    path = "/api/mcp/catalog",
    tag = "mcp",
    responses(
        (status = 200, description = "MCP catalog entries", body = serde_json::Value)
    )
)]
pub async fn list_mcp_catalog(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let installed_ids = collect_installed_catalog_ids(&state);

    let catalog = state
        .kernel
        .mcp_catalog()
        .read()
        .unwrap_or_else(|e| e.into_inner());
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
        (status = 200, description = "Catalog entry detail", body = serde_json::Value),
        (status = 404, description = "Catalog entry not found", body = serde_json::Value),
    )
)]
pub async fn get_mcp_catalog_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let lang = super::resolve_lang(lang.as_ref());
    let installed_ids = collect_installed_catalog_ids(&state);

    let catalog = state
        .kernel
        .mcp_catalog()
        .read()
        .unwrap_or_else(|e| e.into_inner());
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
        (status = 200, description = "Reconnect an MCP server", body = serde_json::Value),
        (status = 404, description = "MCP server not configured", body = serde_json::Value),
    )
)]
pub async fn reconnect_mcp_server_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let configured = state
        .kernel
        .config_ref()
        .mcp_servers
        .iter()
        .any(|s| s.name == name);
    if !configured {
        return ApiErrorResponse::not_found(format!("MCP server '{}' not configured", name))
            .into_json_tuple();
    }

    match state.kernel.reconnect_mcp_server(&name).await {
        Ok(tool_count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": name,
                "status": "connected",
                "tool_count": tool_count,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "id": name,
                "status": "error",
                "error": e,
            })),
        ),
    }
}

/// GET /api/mcp/health — Health snapshot across all configured MCP servers.
#[utoipa::path(
    get,
    path = "/api/mcp/health",
    tag = "mcp",
    responses(
        (status = 200, description = "Health snapshot for all configured MCP servers", body = serde_json::Value)
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
        (status = 200, description = "Reload catalog and reconnect MCP servers", body = serde_json::Value)
    )
)]
pub async fn reload_mcp_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Sync the in-memory config with config.toml before reconnecting.
    // `reload_mcp_servers` reads from `self.config.load_full()`, so if the
    // caller just edited config.toml out-of-band (the CLI's `librefang mcp
    // add/remove` does this, then POSTs /api/mcp/reload) the reload would
    // otherwise run against the stale snapshot and miss the change.
    if let Err(e) = state.kernel.reload_config().await {
        tracing::warn!("Failed to reload config before MCP reload: {e}");
    }

    match state.kernel.reload_mcp_servers().await {
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
// Extension management endpoints — kept as dashboard-friendly aliases over
// the unified store. Installed state comes from config.mcp_servers with
// `template_id` set; catalog-only entries come from the McpCatalog.
// ---------------------------------------------------------------------------

fn installed_servers_by_template(
    servers: &[librefang_types::config::McpServerConfigEntry],
) -> std::collections::HashMap<String, &librefang_types::config::McpServerConfigEntry> {
    let mut map = std::collections::HashMap::new();
    for s in servers {
        if let Some(tid) = &s.template_id {
            map.insert(tid.clone(), s);
        }
    }
    map
}

fn status_str_for_catalog(
    template_id: &str,
    installed_by_template: &std::collections::HashMap<
        String,
        &librefang_types::config::McpServerConfigEntry,
    >,
    health: &librefang_extensions::health::HealthMonitor,
) -> &'static str {
    match installed_by_template.get(template_id) {
        Some(srv) => match health.get_health(&srv.name).as_ref().map(|h| &h.status) {
            Some(librefang_extensions::McpStatus::Ready) => "ready",
            Some(librefang_extensions::McpStatus::Error(_)) => "error",
            _ => "installed",
        },
        None => "available",
    }
}

/// GET /api/extensions — List catalog entries annotated with installed state.
#[utoipa::path(
    get,
    path = "/api/extensions",
    tag = "extensions",
    responses(
        (status = 200, description = "List catalog entries with install/health status", body = serde_json::Value)
    )
)]
pub async fn list_extensions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let installed_map = installed_servers_by_template(&cfg.mcp_servers);
    let health = state.kernel.mcp_health();

    let catalog = state
        .kernel
        .mcp_catalog()
        .read()
        .unwrap_or_else(|e| e.into_inner());

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
        (status = 200, description = "Catalog entry detail + install status", body = serde_json::Value)
    )
)]
pub async fn get_extension(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let installed_map = installed_servers_by_template(&cfg.mcp_servers);
    let catalog = state
        .kernel
        .mcp_catalog()
        .read()
        .unwrap_or_else(|e| e.into_inner());

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
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Install a catalog entry", body = serde_json::Value)
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

    // Reuse the installer via an ephemeral catalog load.
    let home = state.kernel.home_dir().to_path_buf();
    let dotenv_path = home.join(".env");
    let vault_path = home.join("vault.enc");
    let vault = if vault_path.exists() {
        let mut v = librefang_extensions::vault::CredentialVault::new(vault_path);
        if v.unlock().is_ok() {
            Some(v)
        } else {
            None
        }
    } else {
        None
    };
    let mut resolver =
        librefang_extensions::credentials::CredentialResolver::new(vault, Some(&dotenv_path));
    let mut catalog = librefang_extensions::catalog::McpCatalog::new(&home);
    catalog.load(&home);

    let result = match librefang_extensions::installer::install_integration(
        &catalog,
        &mut resolver,
        &name,
        &std::collections::HashMap::new(),
    ) {
        Ok(r) => r,
        Err(e) => {
            let err_str = e.to_string();
            let status = match e {
                librefang_extensions::ExtensionError::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return (status, Json(serde_json::json!({"error": err_str})));
        }
    };

    let config_path = state.kernel.home_dir().join("config.toml");
    if let Err(e) = upsert_mcp_server_config(&config_path, &result.server) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to write config: {e}"),
            })),
        );
    }

    // Sync the in-memory config with the freshly-written config.toml before
    // reload_mcp_servers runs. `reload_mcp_servers` reads from
    // `self.config.load_full()`, so skipping this step means the just-added
    // [[mcp_servers]] entry is invisible and the endpoint reports "installed"
    // without actually connecting anything.
    if let Err(e) = state.kernel.reload_config().await {
        tracing::warn!("Failed to reload config after extension install: {e}");
    }

    state.kernel.mcp_health().register(&result.server.name);
    let connected = state.kernel.reload_mcp_servers().await.unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "installed",
            "name": name,
            "connected": connected > 0,
        })),
    )
}

/// POST /api/extensions/uninstall — Uninstall by catalog id (template_id).
#[utoipa::path(
    post,
    path = "/api/extensions/uninstall",
    tag = "extensions",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Uninstall a catalog-backed MCP server", body = serde_json::Value)
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

    let config_path = state.kernel.home_dir().join("config.toml");
    if let Err(e) = remove_mcp_server_config(&config_path, &server_name) {
        return ApiErrorResponse::internal(format!("Failed to update config: {e}"))
            .into_json_tuple();
    }

    // Sync the in-memory config before reload_mcp_servers runs. Otherwise
    // `self.config.load_full()` still returns the stale snapshot with the
    // removed entry and `reload_mcp_servers` happily reconnects the server
    // we just deleted.
    if let Err(e) = state.kernel.reload_config().await {
        tracing::warn!("Failed to reload config after extension uninstall: {e}");
    }

    state.kernel.mcp_health().unregister(&server_name);
    state.kernel.disconnect_mcp_server(&server_name).await;
    if let Err(e) = state.kernel.reload_mcp_servers().await {
        tracing::warn!("Failed to reload MCP servers after uninstall: {e}");
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
            template_id: None,
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
            taint_scanning: true,
            taint_policy: None,
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
            template_id: None,
            transport: Some(McpTransportEntry::Http {
                url: "http://old:8080/mcp".to_string(),
            }),
            timeout_secs: 10,
            env: vec![],
            headers: vec![],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
        };
        upsert_mcp_server_config(&config_path, &v1).unwrap();

        let v2 = McpServerConfigEntry {
            name: "nocodb".to_string(),
            template_id: None,
            transport: Some(McpTransportEntry::Http {
                url: "http://new:9090/mcp".to_string(),
            }),
            timeout_secs: 60,
            env: vec![],
            headers: vec![],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
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

    /// Regression for #3183: writing a channel section must not destroy
    /// unrelated provider settings (or the user's comments and key order)
    /// in `config.toml`. The previous `toml::Value` round-trip rebuilt the
    /// entire document on every channel write, which dropped comments and
    /// — combined with the missing `config_write_lock` — could clobber a
    /// concurrent provider write from `POST /api/config/set`.
    #[test]
    fn upsert_channel_config_preserves_unrelated_sections_and_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        let original = "\
# Top-of-file comment that must survive channel writes
api_port = 4545

[providers.nim]
# NVIDIA NIM provider — issue #3183 repro
kind = \"openai-compat\"
base_url = \"https://integrate.api.nvidia.com/v1\"
api_key_env = \"NIM_API_KEY\"

[channels.discord]
bot_token_env = \"OLD_DISCORD_TOKEN\"
";
        std::fs::write(&config_path, original).unwrap();

        let mut fields: HashMap<String, (String, FieldType)> = HashMap::new();
        fields.insert(
            "bot_token_env".to_string(),
            ("DISCORD_BOT_TOKEN".to_string(), FieldType::Text),
        );
        fields.insert(
            "guild_ids".to_string(),
            ("123, 456".to_string(), FieldType::List),
        );

        upsert_channel_config(&config_path, "discord", &fields).expect("upsert should succeed");

        let raw = std::fs::read_to_string(&config_path).unwrap();

        // Provider section must be intact — this is the original bug.
        assert!(
            raw.contains("[providers.nim]"),
            "[providers.nim] section was dropped — got:\n{raw}"
        );
        assert!(
            raw.contains("base_url = \"https://integrate.api.nvidia.com/v1\""),
            "NIM base_url was dropped — got:\n{raw}"
        );

        // Comments and the top-level scalar must survive the rewrite.
        assert!(
            raw.contains("# Top-of-file comment that must survive channel writes"),
            "top-level comment was dropped — got:\n{raw}"
        );
        assert!(
            raw.contains("# NVIDIA NIM provider"),
            "in-section comment was dropped — got:\n{raw}"
        );
        assert!(
            raw.contains("api_port = 4545"),
            "top-level scalar was dropped — got:\n{raw}"
        );

        // The new channel fields must be written with correct TOML types
        // (list of strings, not list of integers — see the FieldType::List
        // comment about Discord guild snowflakes).
        #[derive(serde::Deserialize)]
        struct Discord {
            bot_token_env: String,
            guild_ids: Vec<String>,
        }
        #[derive(serde::Deserialize)]
        struct Channels {
            discord: Discord,
        }
        #[derive(serde::Deserialize)]
        struct Wrapper {
            channels: Channels,
        }
        let parsed: Wrapper = toml::from_str(&raw).expect("config must round-trip");
        assert_eq!(parsed.channels.discord.bot_token_env, "DISCORD_BOT_TOKEN");
        assert_eq!(parsed.channels.discord.guild_ids, vec!["123", "456"]);
    }

    /// Companion to the upsert test: removing a channel must also leave
    /// every other section untouched.
    #[test]
    fn remove_channel_config_preserves_unrelated_sections_and_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        let original = "\
# keep me
[providers.nim]
kind = \"openai-compat\"
base_url = \"https://integrate.api.nvidia.com/v1\"

[channels.discord]
bot_token_env = \"DISCORD_BOT_TOKEN\"
";
        std::fs::write(&config_path, original).unwrap();

        remove_channel_config(&config_path, "discord").expect("remove should succeed");

        let raw = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            raw.contains("[providers.nim]"),
            "[providers.nim] was dropped — got:\n{raw}"
        );
        assert!(
            raw.contains("# keep me"),
            "top-level comment was dropped — got:\n{raw}"
        );
        assert!(
            !raw.contains("[channels.discord]"),
            "channel section should have been removed — got:\n{raw}"
        );
    }

    // ── escape_env_value tests (Bug #3790) ─────────────────────────────────

    #[test]
    fn escape_env_value_plain_value_unchanged() {
        assert_eq!(escape_env_value("hello"), "hello");
        assert_eq!(escape_env_value("sk-abc123"), "sk-abc123");
    }

    #[test]
    fn escape_env_value_newline_becomes_backslash_n() {
        let raw = "line1\nline2";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, "line1\\nline2");
        // Must not contain a literal newline character.
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn escape_env_value_carriage_return_becomes_backslash_r() {
        let raw = "val\r\nend";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, "val\\r\\nend");
        assert!(!escaped.contains('\r'));
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn escape_env_value_backslash_is_doubled() {
        let raw = r"C:\Users\secret";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, r"C:\\Users\\secret");
    }

    #[test]
    fn escape_env_value_backslash_before_newline_double_escapes_correctly() {
        // "\\\n" → the backslash must be doubled before the newline is escaped,
        // producing "\\\\n" (a literal backslash-backslash-n), not "\\n".
        let raw = "\\\n";
        let escaped = escape_env_value(raw);
        assert_eq!(escaped, "\\\\\\n");
        assert!(!escaped.contains('\n'));
    }

    #[test]
    fn write_secret_env_value_with_newline_is_rejected() {
        // Implementation tightened to reject newlines in the value rather
        // than escape them — escape-into-single-line was the old behaviour
        // (see this test's previous name) but it left a real injection
        // surface for callers that didn't expect dotenv parsers to honour
        // backslash sequences.  Now we fail-closed: caller must sanitise
        // before passing.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secrets.env");
        let err = write_secret_env(&path, "API_KEY", "val\nwith\nnewlines").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("newline"),
            "error should mention newlines, got: {err}"
        );
        // No file should have been written.
        assert!(
            !path.exists(),
            "secrets.env must not be created on validation error"
        );
    }
}
