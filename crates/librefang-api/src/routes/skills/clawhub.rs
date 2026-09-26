use super::*;
use librefang_skills::clawhub::{ClawHubClient, ClawHubInstallResult};
use librefang_skills::{SkillError, SkillSource};

/// How long a ClawHub **read** route waits for the hub before answering without it.
///
/// The client is patient by design — 30 s per attempt and five attempts, with backoff between them —
/// because a hub that answers slowly is worth more than a hub that is not asked twice. But patience
/// is not what a route owes its caller: `GET /api/clawhub/browse` can spend ~270 s inside the client,
/// and a caller who waits that long has learned nothing they would not have learned from the answer
/// this family already knows how to give.
///
/// Deliberately short enough to sit under the budget `route_smoke` gives every GET it walks
/// (`REQUEST_TIMEOUT`, 10 s), so an unreachable hub is a 503 rather than a failed test.
const CLAWHUB_ROUTE_BUDGET: std::time::Duration = std::time::Duration::from_secs(8);

/// How long a ClawHub **install** route lets its own work run before answering without it.
///
/// Install is a POST with side effects, and `route_smoke` walks GETs only: the read budget above
/// buys it nothing and costs it plenty. An install is a detail fetch, a download, an extraction and
/// a security scan, and cutting it off eight seconds in cancels installs that are merely slow, not
/// broken. The client already bounds a dead hub by itself — 30 s per attempt over five attempts,
/// with backoff — so this is not the first line of defence; it exists so a caller cannot be held
/// forever by work that outlives the client's retries (a hung extraction or scan). It is
/// deliberately far above [`CLAWHUB_ROUTE_BUDGET`], and expiry is safe rather than cheap: see
/// [`install_under_budget`].
const CLAWHUB_INSTALL_BUDGET: std::time::Duration = std::time::Duration::from_secs(300);

/// Run one ClawHub round trip under `budget`.
///
/// A timeout maps to [`SkillError::MarketplaceUnavailable`] and not to `Network`, which is what the
/// client raises for its own failures, because the condition is the one that variant documents: the
/// daemon is healthy, the request was well-formed, and the upstream is not answering as a marketplace.
/// That is the same `503` the rest of the family returns for a hub serving a webpage, so the dashboard
/// renders one offline state for both rather than two.
async fn within_budget<T>(
    budget: std::time::Duration,
    what: &str,
    future: impl std::future::Future<Output = Result<T, SkillError>>,
) -> Result<T, SkillError> {
    match tokio::time::timeout(budget, future).await {
        Ok(result) => result,
        Err(_) => Err(SkillError::MarketplaceUnavailable(format!(
            "{what} did not answer within {}s",
            budget.as_secs()
        ))),
    }
}

/// Run one ClawHub read round trip under [`CLAWHUB_ROUTE_BUDGET`].
async fn within_route_budget<T>(
    what: &str,
    future: impl std::future::Future<Output = Result<T, SkillError>>,
) -> Result<T, SkillError> {
    within_budget(CLAWHUB_ROUTE_BUDGET, what, future).await
}

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

/// A per-call staging directory beside the final skill directory.
///
/// The `.installing-` prefix is the one `SkillRegistry::load_all` already sweeps on every load, so
/// a daemon killed mid-install leaves behind only something the next load removes.
fn install_staging_dir(skills_dir: &std::path::Path, slug: &str) -> std::path::PathBuf {
    static INSTALL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = INSTALL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    skills_dir.join(format!(".installing-{slug}-{}-{seq}", std::process::id()))
}

/// Removes a staging directory when the install task that owns it ends — on success, on error, or
/// on unwind.
///
/// The final `<slug>` directory is only ever created by [`promote_staged_install`], so this guard
/// cannot delete an installed skill: whatever it removes is either a discarded candidate or, after
/// a successful promotion, an empty directory.
struct StagingDir(std::path::PathBuf);

impl Drop for StagingDir {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(
                path = %self.0.display(),
                %error,
                "Could not remove skill install staging directory"
            ),
        }
    }
}

/// Move a complete staged skill into `skills_dir`, refusing to replace an install that
/// raced it there.
///
/// Both paths are children of `skills_dir`, so the rename is atomic on the same filesystem and a
/// reader never observes a half-written `<slug>` directory: it is absent until it is complete.
/// The move takes the client's process-wide promotion lock, so two installs that both passed the
/// routes' `is_installed` probe resolve one after the other: the first lands, and the second is
/// [`SkillError::AlreadyInstalled`] — the same conflict the probe reports — instead of an
/// `ENOTEMPTY` that the handlers would answer with a scrubbed 500.
fn promote_staged_install(
    staging_dir: &std::path::Path,
    skills_dir: &std::path::Path,
    slug: &str,
) -> Result<(), SkillError> {
    let staged = staging_dir.join(slug);
    let installed = skills_dir.join(slug);
    match librefang_skills::clawhub::promote_staged_skill_if_absent(&staged, &installed) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(SkillError::AlreadyInstalled(slug.to_string()))
        }
        Err(error) => Err(SkillError::Io(std::io::Error::other(format!(
            "failed to move staged skill into {}: {error}",
            installed.display()
        )))),
    }
}

/// The answer for a slug that is already installed.
///
/// The routes' pre-install `is_installed` probe and a promotion that lost a concurrent
/// race are the same condition, so both answer alike: `409` with the `already_installed`
/// status the dashboard and CLI already key on. Without the matching arm in the install
/// handlers, the raced `AlreadyInstalled` fell into their 500 catch-all — which blames
/// the daemon for a race the caller can simply retry or ignore.
fn already_installed_response(slug: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": format!("Skill '{slug}' is already installed"),
            "status": "already_installed",
        })),
    )
}

/// The hub an install came from, resolved to a [`SkillSource`] once the version is known.
#[derive(Clone, Copy)]
enum ClawHubHub {
    ClawHub,
    ClawHubCn,
}

impl ClawHubHub {
    /// Stamp which hub a skill came from (#4689). Without this the installed manifest's `source`
    /// stays `None`, `listSkills()` surfaces it as `source.type = "local"`, and the dashboard's
    /// per-hub `isInstalledFromMarketplace(hub, slug)` check misses the freshly installed skill —
    /// the hub's "Install" button keeps showing as clickable until the user reloads.
    fn provenance(self, slug: &str, version: &str) -> SkillSource {
        match self {
            Self::ClawHub => SkillSource::ClawHub {
                slug: slug.to_string(),
                version: version.to_string(),
            },
            Self::ClawHubCn => SkillSource::ClawHubCn {
                slug: slug.to_string(),
                version: version.to_string(),
            },
        }
    }
}

/// Install `slug` under `budget`, writing only to a staging directory until the skill is complete.
///
/// The install runs in its own task, and the staged skill is promoted only if the caller is still
/// waiting when the task finishes. That is what makes a budget expiry safe: the work that gets cut
/// off cannot leave a partial skill behind (the candidate reaches `skills_dir/<slug>` only via one
/// `rename`), and it cannot leave staging junk either (the task's [`StagingDir`] guard removes the
/// candidate once the task ends, after it has stopped writing). A timeout — or an HTTP caller that
/// hangs up — therefore answers without side effects. The only race left is an install that
/// completes in the same instant the budget expires, which lands complete or not at all, never
/// truncated.
async fn install_under_budget(
    client: ClawHubClient,
    slug: String,
    skills_dir: std::path::PathBuf,
    hub: ClawHubHub,
    budget: std::time::Duration,
) -> Result<ClawHubInstallResult, SkillError> {
    let staging_dir = install_staging_dir(&skills_dir, &slug);
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();

    tokio::spawn({
        let staging_dir = staging_dir.clone();
        async move {
            let _staging = StagingDir(staging_dir.clone());
            let result = client.install(&slug, &staging_dir).await;

            match result {
                // The route stopped waiting (budget expired, or the caller hung up): drop the
                // finished candidate instead of promoting a skill whose caller was already told
                // the install failed.
                Ok(_) if result_tx.is_closed() => {}
                Ok(result) => {
                    // Stamp provenance while the skill is still staged, so the directory that
                    // lands in `skills_dir` is complete at rename time. A patch failure stays
                    // non-fatal, exactly as it was when this ran after installation.
                    let manifest = staging_dir.join(&slug).join("skill.toml");
                    let source = hub.provenance(&slug, &result.version);
                    if let Err(error) = patch_skill_provenance_off_thread(manifest, source).await {
                        tracing::warn!(
                            slug = %slug,
                            "Failed to patch provenance in skill.toml: {error}"
                        );
                    }

                    if let Err(error) = promote_staged_install(&staging_dir, &skills_dir, &slug) {
                        let _ = result_tx.send(Err(error));
                    } else {
                        let _ = result_tx.send(Ok(result));
                    }
                }
                Err(error) => {
                    let _ = result_tx.send(Err(error));
                }
            }
        }
    });

    match tokio::time::timeout(budget, result_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_abandoned)) => Err(SkillError::Io(std::io::Error::other(
            "install task ended without reporting a result",
        ))),
        Err(_) => Err(SkillError::MarketplaceUnavailable(format!(
            "ClawHub install did not finish within {}s",
            budget.as_secs()
        ))),
    }
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
        match within_route_budget("ClawHub file", client.get_file(slug, filename)).await {
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

    match within_route_budget("ClawHub search", client.search(&query, limit)).await {
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

    match within_route_budget("ClawHub browse", client.browse(sort, limit, cursor)).await {
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

    match within_route_budget("ClawHub skill detail", client.get_skill(&slug)).await {
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
        return already_installed_response(&req.slug);
    }

    match install_under_budget(
        client,
        req.slug.clone(),
        skills_dir.clone(),
        ClawHubHub::ClawHub,
        CLAWHUB_INSTALL_BUDGET,
    )
    .await
    {
        Ok(result) => {
            // #4689 — provenance was stamped while the skill was staged (see `ClawHubHub::
            // provenance`), so the directory that landed in `skills/` is complete.
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
        // Two concurrent installs of the same slug can both pass the `is_installed`
        // probe above; the one that loses the promotion lock gets the answer the probe
        // would have given it, not an `ENOTEMPTY` 500.
        Err(librefang_skills::SkillError::AlreadyInstalled(_)) => {
            already_installed_response(&req.slug)
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

    match within_route_budget("ClawHub search", client.search(&query, limit)).await {
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

    match within_route_budget("ClawHub browse", client.browse(sort, limit, cursor)).await {
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

    match within_route_budget("ClawHub skill detail", client.get_skill(&slug)).await {
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
        return already_installed_response(&req.slug);
    }

    match install_under_budget(
        client,
        req.slug.clone(),
        skills_dir.clone(),
        ClawHubHub::ClawHubCn,
        CLAWHUB_INSTALL_BUDGET,
    )
    .await
    {
        Ok(result) => {
            // Provenance to ClawHubCn was stamped while the skill was staged, so the registry can
            // surface update/version info and the directory in `skills/` is complete.
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
        // Same conflict as the standard hub: a lost promotion race is the
        // `is_installed` probe's condition, answered as 409, never a 500.
        Err(librefang_skills::SkillError::AlreadyInstalled(_)) => {
            already_installed_response(&req.slug)
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

#[cfg(test)]
mod route_budget_tests {
    use super::{
        already_installed_response, install_under_budget, within_budget, within_route_budget,
        ClawHubHub, CLAWHUB_INSTALL_BUDGET, CLAWHUB_ROUTE_BUDGET,
    };
    use axum::http::StatusCode;
    use librefang_skills::clawhub::ClawHubClient;
    use librefang_skills::SkillError;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A loopback hub that answers every request after `delay`, or never answers at all.
    struct StubHub {
        base_url: String,
        served: Arc<AtomicUsize>,
    }

    fn stub_hub(delay: std::time::Duration, answer: bool) -> StubHub {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub hub");
        let address = listener.local_addr().expect("stub hub address");
        let served = Arc::new(AtomicUsize::new(0));

        let counter = served.clone();
        let _ = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let counter = counter.clone();
                let _ = std::thread::spawn(move || {
                    // Read the request head only: waiting for EOF would block on a client that
                    // is itself waiting for the answer.
                    let mut request = Vec::new();
                    let mut byte = [0_u8; 1];
                    while stream.read_exact(&mut byte).is_ok() {
                        request.push(byte[0]);
                        if request.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }

                    if !answer {
                        // Accept and stall. Long enough to outlast the test, short enough that
                        // the detached thread does not linger long past it.
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        return;
                    }

                    std::thread::sleep(delay);

                    let request = String::from_utf8_lossy(&request);
                    let (content_type, body) = if request.contains("/download") {
                        ("text/markdown", SKILL_MD)
                    } else {
                        ("application/json", DETAIL_JSON)
                    };
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body.as_bytes());
                    let _ = stream.flush();
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    counter.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        StubHub {
            base_url: format!("http://{address}/api/v1"),
            served,
        }
    }

    const SLOW_SLUG: &str = "slow-skill";

    /// A minimal valid detail response. `expectedSha256` is absent, which is the unverified path
    /// the installer already warns about.
    const DETAIL_JSON: &str =
        r#"{"skill":{"slug":"slow-skill"},"latestVersion":{"version":"1.0.0"},"owner":null}"#;

    /// A prompt-only skill body that satisfies conversion and the security pipeline.
    const SKILL_MD: &str =
        "---\nname: slow-skill\ndescription: slow test skill\nversion: 1.0.0\n---\n# Slow\nBody\n";

    fn entries(dir: &Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(dir)
            .expect("read skills dir")
            .map(|entry| entry.expect("dir entry").file_name())
            .collect()
    }

    /// A round trip that never settles must give up on the route's clock, not on the caller's.
    ///
    /// A hub that accepts the connection and then says nothing is the case this exists for: before the
    /// budget, the only bound was the client's own patience — 30 s per attempt across five attempts,
    /// with backoff between them — so the caller learned the hub was unreachable ~270 s later, or never,
    /// because the test that walks every GET route gives up at ten.
    #[tokio::test]
    async fn a_round_trip_that_never_answers_gives_up_inside_the_budget() {
        let started = std::time::Instant::now();
        // A round trip that is never bounded does not fail this test, it hangs it, so the call is
        // given a hard ceiling of its own: a helper that stopped bounding would then fail here with a
        // message that says which property broke, rather than stalling the suite until CI kills it.
        let result = tokio::time::timeout(
            CLAWHUB_ROUTE_BUDGET * 4,
            within_route_budget(
                "test hub",
                std::future::pending::<Result<(), librefang_skills::SkillError>>(),
            ),
        )
        .await
        .expect("within_route_budget never gave up: it must bound the round trip");
        let elapsed = started.elapsed();

        assert!(
            matches!(
                result,
                Err(librefang_skills::SkillError::MarketplaceUnavailable(_))
            ),
            "a hub that never answers is unavailable, not a network fault: {result:?}"
        );
        assert!(
            elapsed >= CLAWHUB_ROUTE_BUDGET,
            "gave up before the budget had elapsed: {elapsed:?}"
        );
        assert!(
            elapsed < CLAWHUB_ROUTE_BUDGET + std::time::Duration::from_secs(3),
            "gave up long after the budget: {elapsed:?}"
        );
    }

    /// The budget has to stay under the bound `route_smoke` gives every GET it walks.
    ///
    /// That relationship is the whole point — it is what makes an unreachable hub a `503` instead of a
    /// failed test — and it is invisible to the compiler, so it is asserted here rather than left in a
    /// comment on the constant. Raising it past ten seconds silently restores the failure this removes.
    #[test]
    fn the_budget_stays_under_the_smoke_tests_own_bound() {
        assert!(
            CLAWHUB_ROUTE_BUDGET < std::time::Duration::from_secs(10),
            "the route budget ({CLAWHUB_ROUTE_BUDGET:?}) must stay under route_smoke's REQUEST_TIMEOUT (10s)"
        );
    }

    /// Install and read budgets are separate, and install's is the larger one.
    ///
    /// Install is a POST that `route_smoke` does not walk, so the eight-second read bound is not
    /// its bound. Collapsing the two constants back together silently restores the cancellation
    /// these tests exist for.
    #[test]
    fn the_install_budget_is_its_own_and_not_the_read_budget() {
        assert!(
            CLAWHUB_INSTALL_BUDGET > CLAWHUB_ROUTE_BUDGET,
            "install ({CLAWHUB_INSTALL_BUDGET:?}) must not share the read budget ({CLAWHUB_ROUTE_BUDGET:?})"
        );
    }

    /// An install that takes longer than the read budget still completes.
    ///
    /// The read budget is a property of the GET routes `route_smoke` walks; applying it to install
    /// cancels a merely slow hub mid-install. This drives one stub at a pace the read budget
    /// refuses and the install budget accepts.
    #[tokio::test]
    async fn an_install_slower_than_the_read_budget_still_completes() {
        let hub = stub_hub(std::time::Duration::from_millis(300), true);
        let client = ClawHubClient::with_url(&hub.base_url, PathBuf::new());
        let skills_dir = tempfile::tempdir().unwrap();

        // The read budget, applied the way a read route applies it, gives up on this hub...
        let read = within_budget(
            std::time::Duration::from_millis(120),
            "ClawHub test read",
            client.get_skill(SLOW_SLUG),
        )
        .await;
        assert!(
            matches!(read, Err(SkillError::MarketplaceUnavailable(_))),
            "the read budget must cut a 300 ms answer: {read:?}"
        );

        // ...while installing the same slug from the same hub completes.
        let result = install_under_budget(
            client,
            SLOW_SLUG.to_string(),
            skills_dir.path().to_path_buf(),
            ClawHubHub::ClawHub,
            std::time::Duration::from_secs(5),
        )
        .await
        .expect("an install that is only slow must not be cut by the read budget");

        assert_eq!(result.skill_name, SLOW_SLUG);
        let manifest_path = skills_dir.path().join(SLOW_SLUG).join("skill.toml");
        assert!(manifest_path.is_file(), "the install did not land complete");
        let manifest: librefang_skills::SkillManifest =
            toml::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert!(
            matches!(
                manifest.source,
                Some(librefang_skills::SkillSource::ClawHub { ref slug, .. }) if slug == SLOW_SLUG
            ),
            "provenance must be stamped before promotion: {:?}",
            manifest.source
        );
        assert_eq!(
            entries(skills_dir.path()),
            vec![std::ffi::OsString::from(SLOW_SLUG)],
            "a completed install must not leave staging behind"
        );
    }

    /// Two concurrent installs of one slug cannot both land in `skills/`.
    ///
    /// The test drives both installs straight into [`install_under_budget`], so no route's
    /// `is_installed` probe filters either of them: both download and stage a candidate, and
    /// both reach the promotion lock — the race the lock exists for. One install must win; the
    /// other must be answered as [`SkillError::AlreadyInstalled`] — the condition the routes
    /// turn into the same 409 the probe produces — and `skills/` must hold exactly one complete
    /// install, never an `ENOTEMPTY` failure and never a `.backup-` or `.installing-` residue.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_installs_of_one_slug_leave_one_install_and_one_conflict() {
        let hub = stub_hub(std::time::Duration::from_millis(150), true);
        let skills_dir = tempfile::tempdir().unwrap();

        let first = install_under_budget(
            ClawHubClient::with_url(&hub.base_url, PathBuf::new()),
            SLOW_SLUG.to_string(),
            skills_dir.path().to_path_buf(),
            ClawHubHub::ClawHub,
            std::time::Duration::from_secs(5),
        );
        let second = install_under_budget(
            ClawHubClient::with_url(&hub.base_url, PathBuf::new()),
            SLOW_SLUG.to_string(),
            skills_dir.path().to_path_buf(),
            ClawHubHub::ClawHub,
            std::time::Duration::from_secs(5),
        );
        let (first, second) = tokio::join!(first, second);
        let outcomes = [first, second];

        let mut installed = 0;
        let mut conflicts = 0;
        for outcome in &outcomes {
            match outcome {
                Ok(_) => installed += 1,
                Err(SkillError::AlreadyInstalled(_)) => conflicts += 1,
                Err(other) => panic!("a raced install answered something else: {other:?}"),
            }
        }
        assert_eq!(
            (installed, conflicts),
            (1, 1),
            "one install wins and the other gets the conflict: {outcomes:?}"
        );

        // The losing task removes its staging directory after it reports the conflict; wait
        // for both cleanup guards to land so the assertions below see the settled state.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while entries(skills_dir.path())
            .iter()
            .any(|name| name.to_string_lossy().starts_with(".installing-"))
        {
            assert!(
                std::time::Instant::now() < deadline,
                "install staging directories outlived the installs: {:?}",
                entries(skills_dir.path())
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(
            entries(skills_dir.path()),
            vec![std::ffi::OsString::from(SLOW_SLUG)],
            "exactly one complete install, with no staging or backup residue"
        );
        assert!(
            skills_dir
                .path()
                .join(SLOW_SLUG)
                .join("skill.toml")
                .is_file(),
            "the winning install must be complete"
        );
    }

    /// A lost promotion race answers exactly like the up-front `is_installed` probe.
    ///
    /// Both install routes return this tuple for a slug that is already installed, so a
    /// caller cannot tell the race from the probe — and in particular never sees the
    /// generic "Internal server error" the 500 catch-all produces.
    #[test]
    fn a_raced_install_answers_the_same_conflict_as_the_is_installed_probe() {
        let (status, body) = already_installed_response(SLOW_SLUG);

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["status"], "already_installed");
        assert!(
            body["error"]
                .as_str()
                .unwrap_or("")
                .contains("already installed"),
            "the conflict must name the condition: {body:?}"
        );
    }

    /// A budget that expires mid-install leaves `skills/` exactly as it found it.
    ///
    /// The hub here is slow but healthy, so the install keeps running after the route stops
    /// waiting, finishes the download, and would have succeeded — which is exactly the case where
    /// a naive implementation promotes a skill whose caller was told the install failed. The
    /// candidate must be discarded instead, and its staging directory removed.
    #[tokio::test]
    async fn a_cancelled_install_discards_its_candidate_without_residue() {
        // Each leg takes ten times the budget, so a late timer under CI load still fires while
        // the install is in flight rather than after it has already answered.
        let hub = stub_hub(std::time::Duration::from_millis(500), true);
        let client = ClawHubClient::with_url(&hub.base_url, PathBuf::new());
        let skills_dir = tempfile::tempdir().unwrap();
        // The staging directory lives for only a couple of milliseconds — measured at ~2 ms here
        // — so polling for it can slide right past it and mistakenly read the empty directory as
        // "already drained". The directory's mtime is the durable witness: creating the staging
        // directory and removing it both touch it. Snapshot it before the install starts.
        let baseline_mtime = std::fs::metadata(skills_dir.path())
            .expect("stat skills dir")
            .modified()
            .expect("skills dir mtime");

        let error = install_under_budget(
            client,
            SLOW_SLUG.to_string(),
            skills_dir.path().to_path_buf(),
            ClawHubHub::ClawHub,
            std::time::Duration::from_millis(50),
        )
        .await
        .expect_err("a budget that expires mid-download must answer");
        assert!(
            matches!(error, SkillError::MarketplaceUnavailable(_)),
            "budget expiry is the marketplace being unavailable, not a network fault: {error:?}"
        );
        assert!(entries(skills_dir.path()).is_empty());

        // The install task is not cancelled; it keeps going and only then learns the route is
        // gone. Wait for the download to have been served, then — under a deadline, not a fixed
        // window a saturated CI can outlast — for the task to have reached its staging phase,
        // witnessed by the mtime change. Only then may the drain below run: starting it while
        // the staging has not even been created yet would let it pass on an empty `skills_dir`
        // and mask a promotion or residue regression.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while hub.served.load(Ordering::Relaxed) < 2 && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            hub.served.load(Ordering::Relaxed) >= 2,
            "the stub hub never served the download"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::fs::metadata(skills_dir.path())
            .expect("stat skills dir")
            .modified()
            .expect("skills dir mtime")
            == baseline_mtime
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the timed-out install never created its staging directory"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let discard_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !entries(skills_dir.path()).is_empty() {
            assert!(
                !skills_dir.path().join(SLOW_SLUG).exists(),
                "a timed-out install was promoted into skills/"
            );
            assert!(
                std::time::Instant::now() < discard_deadline,
                "a timed-out install left staging residue: {:?}",
                entries(skills_dir.path())
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            entries(skills_dir.path()).is_empty(),
            "a timed-out install must leave skills/ exactly as it found it"
        );
    }

    /// The hub the budget exists for — accepts and then says nothing — leaves no residue either.
    #[tokio::test]
    async fn an_install_from_a_hub_that_never_answers_leaves_no_residue() {
        let hub = stub_hub(std::time::Duration::ZERO, false);
        let client = ClawHubClient::with_url(&hub.base_url, PathBuf::new());
        let skills_dir = tempfile::tempdir().unwrap();

        let error = install_under_budget(
            client,
            SLOW_SLUG.to_string(),
            skills_dir.path().to_path_buf(),
            ClawHubHub::ClawHub,
            std::time::Duration::from_millis(200),
        )
        .await
        .expect_err("a hub that never answers must hit the budget");
        assert!(
            matches!(error, SkillError::MarketplaceUnavailable(_)),
            "{error:?}"
        );

        // Nothing can appear while the stalled download is still open.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(entries(skills_dir.path()).is_empty());
    }
}
