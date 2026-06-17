use super::*;

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
        (status = 200, description = "Search ClawHub skills", body = crate::types::JsonObject)
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
        (status = 200, description = "Browse ClawHub skills by sort order", body = crate::types::JsonObject)
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
        (status = 200, description = "Get detailed info about a ClawHub skill", body = crate::types::JsonObject)
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
        (status = 200, description = "Fetch source code of a ClawHub skill", body = crate::types::JsonObject)
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
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Install a skill from ClawHub", body = crate::types::JsonObject)
    )
)]
pub async fn clawhub_install(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::types::ClawHubInstallRequest>,
) -> impl IntoResponse {
    let home = state.kernel.home_dir();
    // Reject path-traversal payloads in `hand` before it reaches any
    // `Path::join` below — mirrors the guard `install_skill` applies to
    // the same field. Without this, `{"hand":"../../something"}` escapes
    // `~/.librefang/workspaces/hands/`. (audit: clawhub-install-path-traversal)
    if let Some(ref hand_id) = req.hand {
        if let Err(reason) = validate_skill_identifier(hand_id, "hand") {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": reason})),
            );
        }
    }
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
            // #4689 — patch source provenance to ClawHub. Without this, the
            // installed skill's manifest.source stays None and `listSkills()`
            // surfaces it as `source.type = "local"`, which makes the
            // dashboard's per-hub `isInstalledFromMarketplace("clawhub", slug)`
            // check miss the freshly installed skill — the hub's "Install"
            // button keeps showing as clickable until the user reloads. The
            // ClawHubCn handler already does this; bringing ClawHub in line.
            let skill_dir = skills_dir.join(&req.slug);
            let manifest_path = skill_dir.join("skill.toml");
            if manifest_path.exists() {
                match std::fs::read_to_string(&manifest_path) {
                    Ok(toml_str) => {
                        match toml::from_str::<librefang_skills::SkillManifest>(&toml_str) {
                            Ok(mut manifest) => {
                                manifest.source = Some(librefang_skills::SkillSource::ClawHub {
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

            // Reload so the kernel sees the patched provenance immediately —
            // mirrors what reload_skills() does for the FangHub install path.
            state.kernel.reload_skills();

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
            // 4xx / 502 echo the actionable SkillError (security
            // block, rate limit, network); the 500 catch-all scrubs to
            // a generic body (audit: rusqlite-errors-leak). Full error
            // already logged above.
            let body = if status == StatusCode::INTERNAL_SERVER_ERROR {
                "Internal server error".to_string()
            } else {
                msg
            };
            (status, Json(serde_json::json!({"error": body})))
        }
    }
}

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
    // Reject path-traversal payloads in `hand` before it reaches any
    // `Path::join` below — mirrors the guard `install_skill` applies to
    // the same field. Without this, `{"hand":"../../something"}` escapes
    // `~/.librefang/workspaces/hands/`. (audit: clawhub-install-path-traversal)
    if let Some(ref hand_id) = req.hand {
        if let Err(reason) = validate_skill_identifier(hand_id, "hand") {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": reason})),
            );
        }
    }
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
            // See ClawHub install above: 500 catch-all scrubbed
            // (audit: rusqlite-errors-leak), actionable 4xx / 502
            // echoed. Full error already logged above.
            let body = if status == StatusCode::INTERNAL_SERVER_ERROR {
                "Internal server error".to_string()
            } else {
                msg
            };
            (status, Json(serde_json::json!({"error": body})))
        }
    }
}
