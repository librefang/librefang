use super::*;

fn patch_skill_provenance(
    manifest_path: &std::path::Path,
    source: librefang_skills::SkillSource,
) -> Result<(), String> {
    if !manifest_path.exists() {
        return Ok(());
    }

    let toml_str = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
    let mut manifest = toml::from_str::<librefang_skills::SkillManifest>(&toml_str)
        .map_err(|e| format!("parse {}: {e}", manifest_path.display()))?;
    manifest.source = Some(source);
    let updated = toml::to_string_pretty(&manifest)
        .map_err(|e| format!("serialize {}: {e}", manifest_path.display()))?;
    crate::atomic_write(manifest_path, updated.as_bytes())
        .map_err(|e| format!("write {}: {e}", manifest_path.display()))
}

async fn patch_skill_provenance_off_thread(
    manifest_path: std::path::PathBuf,
    source: librefang_skills::SkillSource,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || patch_skill_provenance(&manifest_path, source))
        .await
        .map_err(|e| format!("provenance patch task failed: {e}"))?
}

#[cfg(test)]
mod provenance_tests {
    use super::patch_skill_provenance;

    #[test]
    fn patches_manifest_atomically_without_staging_residue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skill.toml");
        std::fs::write(
            &path,
            "[skill]\nname = \"example\"\nversion = \"1.0.0\"\ndescription = \"test\"\n",
        )
        .unwrap();

        patch_skill_provenance(
            &path,
            librefang_skills::SkillSource::ClawHub {
                slug: "example-skill".to_string(),
                version: "1.2.3".to_string(),
            },
        )
        .unwrap();

        let manifest: librefang_skills::SkillManifest =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(matches!(
            manifest.source,
            Some(librefang_skills::SkillSource::ClawHub { slug, version })
                if slug == "example-skill" && version == "1.2.3"
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_manifest_remains_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");

        patch_skill_provenance(
            &path,
            librefang_skills::SkillSource::ClawHubCn {
                slug: "missing".to_string(),
                version: "1.0.0".to_string(),
            },
        )
        .unwrap();

        assert!(!path.exists());
    }
}

/// Fetch the first source file a ClawHub-family hub actually has for `slug`.
///
/// Returns `Err` only when the hub is not serving marketplace data at all.
/// Every candidate name is fetched from the same host, so once that host answers with its webpage instead of a file, walking to the next name just re-reads the same page — and the old `if let Ok` chain then reported the result as "no source code found", a `404` about the skill for a fault in the hub (#7387).
/// Any other per-file failure keeps the original try-the-next-name behaviour and still ends as that `404` when none of the three exist.
async fn fetch_skill_source(
    client: &librefang_skills::clawhub::ClawHubClient,
    slug: &str,
) -> Result<Option<(String, String)>, librefang_skills::SkillError> {
    for filename in ["SKILL.md", "package.json", "skill.toml"] {
        match client.get_file(slug, filename).await {
            Ok(content) if !content.is_empty() => return Ok(Some((filename.to_string(), content))),
            Ok(_) => continue,
            Err(error) if is_marketplace_unavailable(&error) => return Err(error),
            Err(_) => continue,
        }
    }
    Ok(None)
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
            let status = marketplace_error_status(&e, StatusCode::BAD_GATEWAY);
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
            let status = marketplace_error_status(&e, StatusCode::BAD_GATEWAY);
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
            // `404` is only honest for a slug the hub says it does not have.
            // When the hub itself is not answering as a marketplace, saying "not found" invents a fact about the skill (#7387).
            let status = marketplace_error_status(&e, StatusCode::NOT_FOUND);
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
    let (filename, code) = match fetch_skill_source(&client, &slug).await {
        Ok(Some(found)) => found,
        Ok(None) => {
            return ApiErrorResponse::not_found("No source code found for this skill")
                .into_json_tuple()
        }
        Err(error) => {
            tracing::warn!("ClawHub skill code fetch failed: {error}");
            return (
                marketplace_error_status(&error, StatusCode::BAD_GATEWAY),
                Json(serde_json::json!({"error": format!("{error}")})),
            );
        }
    };

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
            let manifest_path = skills_dir.join(&req.slug).join("skill.toml");
            let source = librefang_skills::SkillSource::ClawHub {
                slug: req.slug.clone(),
                version: result.version.clone(),
            };
            if let Err(e) = patch_skill_provenance_off_thread(manifest_path.clone(), source).await {
                tracing::warn!(
                    slug = %req.slug,
                    path = %manifest_path.display(),
                    "Failed to patch provenance in skill.toml: {e}"
                );
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
            } else {
                // A dead marketplace used to fall through to the `500`, whose body is then scrubbed to "Internal server error" — the one case where the operator most needs the text (#7387).
                marketplace_error_status(
                    &e,
                    if matches!(e, librefang_skills::SkillError::Network(_)) {
                        StatusCode::BAD_GATEWAY
                    } else {
                        StatusCode::INTERNAL_SERVER_ERROR
                    },
                )
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
    let client =
        librefang_skills::clawhub::ClawHubClient::with_url(&clawhub_cn_base_url(), cache_dir);

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
            let status = marketplace_error_status(&e, StatusCode::BAD_GATEWAY);
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
    let client =
        librefang_skills::clawhub::ClawHubClient::with_url(&clawhub_cn_base_url(), cache_dir);

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
            let status = marketplace_error_status(&e, StatusCode::BAD_GATEWAY);
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
    let client =
        librefang_skills::clawhub::ClawHubClient::with_url(&clawhub_cn_base_url(), cache_dir);

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
            // `404` is only honest for a slug the hub says it does not have.
            // When the hub itself is not answering as a marketplace, saying "not found" invents a fact about the skill (#7387).
            let status = marketplace_error_status(&e, StatusCode::NOT_FOUND);
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
    let client =
        librefang_skills::clawhub::ClawHubClient::with_url(&clawhub_cn_base_url(), cache_dir);

    let (filename, code) = match fetch_skill_source(&client, &slug).await {
        Ok(Some(found)) => found,
        Ok(None) => {
            return ApiErrorResponse::not_found("No source code found for this skill")
                .into_json_tuple()
        }
        Err(error) => {
            tracing::warn!("ClawHub skill code fetch failed: {error}");
            return (
                marketplace_error_status(&error, StatusCode::BAD_GATEWAY),
                Json(serde_json::json!({"error": format!("{error}")})),
            );
        }
    };

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
    let client =
        librefang_skills::clawhub::ClawHubClient::with_url(&clawhub_cn_base_url(), cache_dir);

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
            let manifest_path = skills_dir.join(&req.slug).join("skill.toml");
            let source = librefang_skills::SkillSource::ClawHubCn {
                slug: req.slug.clone(),
                version: result.version.clone(),
            };
            if let Err(e) = patch_skill_provenance_off_thread(manifest_path.clone(), source).await {
                tracing::warn!(
                    slug = %req.slug,
                    path = %manifest_path.display(),
                    "Failed to patch provenance in skill.toml: {e}"
                );
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
            } else {
                // A dead marketplace used to fall through to the `500`, whose body is then scrubbed to "Internal server error" — the one case where the operator most needs the text (#7387).
                marketplace_error_status(
                    &e,
                    if matches!(e, librefang_skills::SkillError::Network(_)) {
                        StatusCode::BAD_GATEWAY
                    } else {
                        StatusCode::INTERNAL_SERVER_ERROR
                    },
                )
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
