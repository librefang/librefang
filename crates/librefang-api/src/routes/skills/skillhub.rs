use super::*;

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
