use super::*;

/// GET /api/skills — List installed skills.
///
/// `categories` always reflects all skills regardless of the `?category=` filter.
#[utoipa::path(
    get,
    path = "/api/skills",
    tag = "skills",
    responses(
        (status = 200, description = "List installed skills", body = crate::types::JsonObject)
    )
)]
pub async fn list_skills(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListSkillsQuery>,
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

    let category_filter = params.category.as_deref();

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

    // Pagination (#3639): apply `?offset=&limit=` after the category filter
    // and category-set computation, so `categories` always reflects the
    // unfiltered registry while `items`/`total` reflect the filtered + paged
    // view. Capped server-side at PAGINATION_MAX_LIMIT.
    let pagination = crate::types::PaginationQuery {
        offset: params.offset,
        limit: params.limit,
    };
    let (items, total, offset, limit) = pagination.paginate(skills);
    let categories_vec: Vec<String> = categories.into_iter().collect();
    // Untyped JSON so `categories` can ride alongside the canonical
    // PaginatedResponse fields without a new struct.
    Json(serde_json::json!({
        "items": items,
        "total": total,
        "offset": offset,
        "limit": limit,
        "categories": categories_vec,
    }))
}

/// POST /api/skills/install — Install a skill from FangHub (GitHub).
#[utoipa::path(
    post,
    path = "/api/skills/install",
    tag = "skills",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Install a skill from FangHub", body = crate::types::JsonObject)
    )
)]
pub async fn install_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SkillInstallRequest>,
) -> impl IntoResponse {
    // Reject path-traversal payloads on BOTH `name` and `hand` before
    // letting either reach `Path::join`. Pre-fix the handler did
    // `home.join("registry").join("skills").join(&req.name)` and
    // `home.join("workspaces").join("hands").join(hand_id)` with no
    // rejection of `..` / `/` / `\`, so a payload like
    // `{"name":"../../etc/cron.daily/payload"}` would (a) leak FS
    // existence via the `.exists()` probe (200 / 404 oracle) and (b)
    // let `copy_dir_recursive` write outside `~/.librefang/skills/`
    // (full filesystem write under the daemon UID). The sibling
    // `uninstall_skill` at `librefang-skills/src/evolution.rs:1277`
    // already hardens uninstall — this brings install in line. The
    // validator below matches the project's strict pattern from
    // `agent_templates.rs:113-124` (alphanumeric + `_` + `-`, ≤ 64
    // chars, no leading `.`). (audit:
    // skill-install-path-traversal)
    if let Err(reason) = validate_skill_identifier(&req.name, "name") {
        return ApiErrorResponse::bad_request(reason).into_json_tuple();
    }
    if let Some(ref hand_id) = req.hand {
        if let Err(reason) = validate_skill_identifier(hand_id, "hand") {
            return ApiErrorResponse::bad_request(reason).into_json_tuple();
        }
    }

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
        return ApiErrorResponse::internal_scrub(e).into_json_tuple();
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
            ApiErrorResponse::internal_scrub(e).into_json_tuple()
        }
    }
}

/// POST /api/skills/uninstall — Uninstall a skill.
#[utoipa::path(
    post,
    path = "/api/skills/uninstall",
    tag = "skills",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Uninstall a skill", body = crate::types::JsonObject)
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
        (status = 200, description = "Rescan the skills directory from disk", body = crate::types::JsonObject)
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

/// GET /api/skills/pending — list skill-workshop pending candidates,
/// oldest captured first. Optionally filtered by agent.
#[utoipa::path(
    get,
    path = "/api/skills/pending",
    tag = "skills",
    params(PendingListQuery),
    responses(
        (status = 200, description = "List pending workshop candidates", body = crate::types::JsonObject)
    )
)]
pub async fn list_pending_candidates(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<PendingListQuery>,
) -> impl IntoResponse {
    let skills_root = state.kernel.home_dir().join("skills");
    let result = match q.agent.as_deref() {
        Some(agent) => librefang_kernel::skill_workshop::storage::list_pending(&skills_root, agent),
        None => librefang_kernel::skill_workshop::storage::list_pending_all(&skills_root),
    };
    match result {
        Ok(candidates) => (
            StatusCode::OK,
            Json(serde_json::json!({"candidates": candidates})),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::InvalidId(id)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("invalid agent id (must be a UUID): {id}")})),
        ),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}

/// GET /api/skills/pending/{id} — return a single pending candidate by id.
#[utoipa::path(
    get,
    path = "/api/skills/pending/{id}",
    tag = "skills",
    params(
        ("id" = String, Path, description = "Candidate UUID")
    ),
    responses(
        (status = 200, description = "Pending candidate detail", body = crate::types::JsonObject),
        (status = 404, description = "Candidate not found", body = crate::types::JsonObject)
    )
)]
pub async fn show_pending_candidate(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let skills_root = state.kernel.home_dir().join("skills");
    match librefang_kernel::skill_workshop::storage::load_candidate(&skills_root, &id) {
        Ok(candidate) => (
            StatusCode::OK,
            Json(serde_json::json!({"candidate": candidate})),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::InvalidId(_)) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": format!("invalid candidate id (must be a UUID): {id}")}),
            ),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::NotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("candidate '{id}' not found")})),
        ),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}

/// POST /api/skills/pending/{id}/approve — promote a pending candidate
/// into the active skill registry via `evolution::create_skill`.
#[utoipa::path(
    post,
    path = "/api/skills/pending/{id}/approve",
    tag = "skills",
    params(
        ("id" = String, Path, description = "Candidate UUID")
    ),
    responses(
        (status = 200, description = "Candidate promoted to active skill", body = crate::types::JsonObject),
        (status = 404, description = "Candidate not found", body = crate::types::JsonObject),
        (status = 409, description = "Promotion blocked (security scan or naming collision)", body = crate::types::JsonObject),
        (status = 422, description = "Update candidate targets a skill that no longer exists", body = crate::types::JsonObject)
    )
)]
pub async fn approve_pending_candidate(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    // `skills_root` is still needed by the `AlreadyInstalled` phantom-vs-collision
    // disambiguation branch below, which reads the candidate + active skill body
    // directly off disk.
    let skills_root = state.kernel.home_dir().join("skills");
    // Capture the creating agent id BEFORE promotion: the kernel approve
    // deletes the pending TOML on success, and the phantom-recovery arm
    // below (AlreadyInstalled) still needs it to converge the best-effort
    // auto-assign. A read failure here must not block the approve.
    let creator_agent_id =
        match librefang_kernel::skill_workshop::storage::load_candidate(&skills_root, &id) {
            Ok(c) => Some(c.agent_id),
            Err(_) => None,
        };
    // Promote through the kernel so a CREATE is also auto-assigned to the
    // creating agent's allowlist (#5844 / #5989): the kernel method loads
    // the candidate, calls approve_candidate (re-running the injection
    // scan), reloads the registry, and appends the new skill to the owner
    // agent's allowlist when the candidate is a create, clearing
    // skills_disabled so it goes live. The AlreadyInstalled arm below
    // handles phantom recovery, where the kernel errored and did not
    // assign, so it re-runs the route-level best-effort assign itself.
    match state.kernel.approve_pending_skill(&id) {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "approved",
                "candidate_id": id,
                "skill_name": result.skill_name,
                "version": result.version,
                "message": result.message,
            })),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::InvalidId(_)) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": format!("invalid candidate id (must be a UUID): {id}")}),
            ),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::NotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("candidate '{id}' not found")})),
        ),
        Err(e @ librefang_kernel::skill_workshop::WorkshopError::SecurityBlocked(_)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::Skill(
            librefang_skills::SkillError::AlreadyInstalled(skill_name),
        )) => {
            // `AlreadyInstalled` from `evolution::create_skill` is
            // ambiguous and we MUST NOT collapse the two cases:
            //
            //   * Phantom pending — a previous approve of THIS candidate
            //     promoted the skill but the pending-file cleanup failed
            //     transiently (Windows AV holding a handle, read-only
            //     mount mid-clean-up). The active body is byte-identical
            //     to the candidate's `prompt_context`. Idempotent
            //     recovery: drop the pending row, return 200
            //     `already_promoted`.
            //   * Name collision — the user already has an unrelated
            //     skill with the same name (manual install, marketplace,
            //     prior `evolve`, or a `synth_name` fallback collision).
            //     The active body differs from the candidate body.
            //     Silently dropping the pending row in this case would
            //     destroy the candidate the user wanted reviewed without
            //     them ever seeing it — a real data-loss bug. Return 409
            //     and KEEP the pending file so the reviewer can rename
            //     and retry.
            //
            // Decide by reading the active skill's `prompt_context.md`
            // and comparing byte-for-byte against the candidate's stored
            // `prompt_context` (`evolution::create_skill` writes the
            // string verbatim — no trim, no normalisation — so equality
            // is well-defined). If we cannot load the candidate (e.g.
            // it was already cleaned up by a concurrent reject), the
            // recovery target state is reached anyway → 200.
            let candidate = match librefang_kernel::skill_workshop::storage::load_candidate(
                &skills_root,
                &id,
            ) {
                Ok(c) => Some(c),
                Err(librefang_kernel::skill_workshop::WorkshopError::NotFound(_)) => None,
                Err(e) => {
                    // Scrub the raw storage error (audit:
                    // rusqlite-errors-leak) — keep the operator-useful
                    // context in the log, return a generic body.
                    tracing::error!(error = %e, %skill_name, "failed to read candidate to disambiguate phantom vs collision");
                    return ApiErrorResponse::internal_scrub(e).into_json_tuple();
                }
            };
            let bodies_match = match &candidate {
                None => true, // Concurrent cleanup beat us — terminal state already reached.
                Some(cand) => {
                    let active_body_path = skills_root.join(&skill_name).join("prompt_context.md");
                    match std::fs::read_to_string(&active_body_path) {
                        Ok(active) => active == cand.prompt_context,
                        // If we can't read the active body we cannot prove
                        // it's a phantom — fall through to the collision
                        // branch so we never drop the pending file.
                        Err(_) => false,
                    }
                }
            };
            if bodies_match {
                // Phantom recovery. `NotFound` from the nested reject is
                // the desired terminal state (a concurrent reject / CLI
                // cleanup beat us to the row), not a failure.
                match librefang_kernel::skill_workshop::storage::reject_candidate(&skills_root, &id)
                {
                    Ok(()) | Err(librefang_kernel::skill_workshop::WorkshopError::NotFound(_)) => {
                        // Idempotent re-approval: the skill was already
                        // promoted, but the creating agent may not have been
                        // assigned on the first pass (e.g. it didn't exist
                        // yet, or assignment failed). Re-run the best-effort
                        // assign so phantom recovery converges to the same
                        // end state — `assign_skill_to_creator` is a no-op
                        // when the skill is already live on the allowlist.
                        assign_skill_to_creator(&state, creator_agent_id.as_deref(), &skill_name);
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "status": "already_promoted",
                                "candidate_id": id,
                                "skill_name": skill_name,
                                "message": format!(
                                    "Active skill '{skill_name}' already exists with the same body; pending entry cleared.",
                                ),
                            })),
                        )
                    }
                    Err(e) => {
                        // Scrub the raw storage error (audit:
                        // rusqlite-errors-leak); operator context in
                        // the log, generic body to the client.
                        tracing::error!(error = %e, %skill_name, "failed to clear pending entry after phantom recovery");
                        ApiErrorResponse::internal_scrub(e).into_json_tuple()
                    }
                }
            } else {
                // Real name collision. Pending file is intentionally
                // left in place so the reviewer can rename and retry
                // without losing their candidate.
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": format!(
                            "Skill '{skill_name}' already exists with different content. \
                             Edit the candidate's `name` field in its pending TOML \
                             (or reject it and capture again under a different rule) and retry."
                        ),
                        "kind": "name_collision",
                        "candidate_id": id,
                        "skill_name": skill_name,
                    })),
                )
            }
        }
        Err(librefang_kernel::skill_workshop::WorkshopError::Skill(
            librefang_skills::SkillError::NotFound(detail),
        )) => {
            // Update candidates can outlive their target: the skill may be
            // deleted between candidate capture and approval. 409 would
            // mislead the client into a rename-and-retry; the candidate is
            // unprocessable until the target skill exists again.
            // `NotFound`'s payload is a human-readable detail string (not a
            // bare skill name), so embed it as-is.
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!(
                        "target skill no longer exists ({detail}); \
                         reject this candidate and re-capture once the skill is reinstalled"
                    ),
                    "kind": "target_skill_missing",
                    "candidate_id": id,
                })),
            )
        }
        Err(librefang_kernel::skill_workshop::WorkshopError::Skill(e)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("promotion rejected: {e}")})),
        ),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}

/// Append a freshly-promoted workshop skill to its creating agent's
/// allowlist (#5989). Best-effort and idempotent:
///
/// * `agent_id` is `None` / unparseable / no longer in the registry →
///   log a WARN and return; the caller's approve still succeeds.
/// * agent's allowlist is empty (which already grants every skill) →
///   no-op, mirroring the kernel's `assign_skill_to_agent_allowlist`;
///   appending here would narrow an all-skills agent to just this one.
/// * skill already on the allowlist AND not hidden by `skills_disabled`
///   → no-op (no redundant DB write).
/// * otherwise append the skill and route through `set_agent_skills`,
///   which clears `skills_disabled` so the new skill is live, persists
///   the manifest, and invalidates the agent's cached tool list.
fn assign_skill_to_creator(state: &Arc<AppState>, agent_id: Option<&str>, skill_name: &str) {
    let Some(agent_id) = agent_id else {
        tracing::warn!(
            %skill_name,
            "skill_workshop: approved candidate had no recoverable agent_id; \
             skipping auto-assign (skill promoted, assign manually if needed)"
        );
        return;
    };
    let Ok(agent_id) = agent_id.parse::<librefang_types::agent::AgentId>() else {
        tracing::warn!(
            %agent_id,
            %skill_name,
            "skill_workshop: candidate agent_id is not a valid AgentId; skipping auto-assign"
        );
        return;
    };
    let Some(entry) = state.kernel.agent_registry().get(agent_id) else {
        tracing::warn!(
            %agent_id,
            %skill_name,
            "skill_workshop: creating agent no longer exists; skipping auto-assign \
             (skill promoted successfully)"
        );
        return;
    };

    // An empty allowlist already grants the agent every skill; appending
    // would narrow it to just this one. Match the kernel's
    // `assign_skill_to_agent_allowlist` and leave all-skills agents
    // untouched, so this phantom-recovery path behaves identically to the
    // create path (which skips the same case kernel-side).
    if entry.manifest.skills.is_empty() {
        return;
    }

    let already_listed = entry.manifest.skills.iter().any(|s| s == skill_name);
    // Re-run the assign when the skill is present but hidden by
    // `skills_disabled` so the flag is cleared and the skill goes live.
    if already_listed && !entry.manifest.skills_disabled {
        return;
    }

    let mut skills = entry.manifest.skills.clone();
    if !already_listed {
        skills.push(skill_name.to_string());
    }
    if let Err(e) = state.kernel.set_agent_skills(agent_id, skills) {
        tracing::warn!(
            %agent_id,
            %skill_name,
            error = %e,
            "skill_workshop: failed to auto-assign promoted skill to creating agent \
             (skill promoted successfully; assign manually)"
        );
    }
}

/// POST /api/skills/pending/{id}/reject — drop a pending candidate
/// without promoting.
#[utoipa::path(
    post,
    path = "/api/skills/pending/{id}/reject",
    tag = "skills",
    params(
        ("id" = String, Path, description = "Candidate UUID")
    ),
    responses(
        (status = 200, description = "Candidate dropped", body = crate::types::JsonObject),
        (status = 404, description = "Candidate not found", body = crate::types::JsonObject)
    )
)]
pub async fn reject_pending_candidate(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let skills_root = state.kernel.home_dir().join("skills");
    match librefang_kernel::skill_workshop::storage::reject_candidate(&skills_root, &id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "rejected", "candidate_id": id})),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::InvalidId(_)) => (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": format!("invalid candidate id (must be a UUID): {id}")}),
            ),
        ),
        Err(librefang_kernel::skill_workshop::WorkshopError::NotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("candidate '{id}' not found")})),
        ),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
    }
}

/// GET /api/skills/registry — List official skills from the local registry cache (~/.librefang/registry/skills).
#[utoipa::path(
    get,
    path = "/api/skills/registry",
    tag = "skills",
    responses(
        (status = 200, description = "Official skills available in the FangHub registry", body = crate::types::JsonObject)
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

/// GET /api/marketplace/search — Search the FangHub marketplace.
#[utoipa::path(
    get,
    path = "/api/marketplace/search",
    tag = "skills",
    params(
        ("q" = Option<String>, Query, description = "Search query"),
    ),
    responses(
        (status = 200, description = "Search the FangHub marketplace", body = crate::types::JsonObject)
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

#[utoipa::path(
    post,
    path = "/api/skills/create",
    tag = "skills",
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Create a new prompt-only skill", body = crate::types::JsonObject)
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
        (status = 200, description = "Skill detail with evolution history", body = crate::types::JsonObject),
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

/// POST /api/skills/{name}/propose — open a PR contributing this skill
/// to the configured public skill registry.
///
/// Forks the registry repo under the authenticated GitHub user, pushes
/// the skill files to a fresh branch, and opens a pull request with an
/// auto-generated description (metadata + evolution changelog). Requires
/// a `GITHUB_TOKEN` (env or vault).
#[utoipa::path(
    post,
    path = "/api/skills/{name}/propose",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    responses(
        (status = 200, description = "PR opened against the registry", body = crate::types::JsonObject),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "No GitHub token configured"),
        (status = 404, description = "Skill not found"),
        (status = 502, description = "GitHub request failed")
    )
)]
pub async fn propose_skill_to_registry(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let skill = match clone_installed_skill(&state, &name) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let Some(token) = resolve_github_token(&state) else {
        return ApiErrorResponse::unauthorized(
            "No GitHub token configured. Connect GitHub in Settings or set GITHUB_TOKEN.",
        )
        .into_json_tuple();
    };

    let registry_repo = state
        .kernel
        .config_snapshot()
        .skills
        .registry_repo
        .clone()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| librefang_skills::registry_pr::DEFAULT_REGISTRY_REPO.to_string());

    let evolution = librefang_skills::evolution::get_evolution_info(&skill);

    run_registry_proposal(&skill, &evolution, &registry_repo, &token).await
}

/// Shared core for the two "propose to registry" routes: the installed-skill
/// route (`/api/skills/{name}/propose`) and the pending-candidate route
/// (`/api/skills/pending/{id}/propose-to-registry`). Both build an
/// [`librefang_skills::InstalledSkill`] (the candidate route stages one in a
/// temp directory) and delegate the fork/push/PR work to
/// `registry_pr::propose_skill_to_registry`, mapping its result onto the same
/// JSON shape so dashboard consumers get an identical response either way.
async fn run_registry_proposal(
    skill: &librefang_skills::InstalledSkill,
    evolution: &librefang_skills::evolution::SkillEvolutionMeta,
    registry_repo: &str,
    token: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    let result = librefang_skills::registry_pr::propose_skill_to_registry(
        librefang_skills::registry_pr::ProposeRequest {
            skill,
            evolution,
            registry_repo,
            token,
        },
    )
    .await;

    match result {
        Ok(pr) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "pr_url": pr.pr_url,
                "repo": pr.repo,
                "branch": pr.branch,
            })),
        ),
        Err(librefang_skills::SkillError::SecurityBlocked(msg)) => {
            ApiErrorResponse::unauthorized(msg).into_json_tuple()
        }
        Err(librefang_skills::SkillError::InvalidManifest(msg)) => {
            ApiErrorResponse::bad_request(msg).into_json_tuple()
        }
        Err(librefang_skills::SkillError::NotFound(msg)) => {
            ApiErrorResponse::not_found(msg).into_json_tuple()
        }
        // Network / GitHub failures surface as 502 Bad Gateway — the
        // request was well-formed but the upstream dependency failed.
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// POST /api/skills/pending/{id}/propose-to-registry — open a PR
/// contributing a *pending* candidate to the configured public skill
/// registry, without first approving it into the active registry (#5819).
///
/// The candidate carries everything a contribution needs (`name`,
/// `description`, `prompt_context`). We stage it as an
/// [`librefang_skills::InstalledSkill`] in a throwaway temp directory and
/// reuse the same fork/push/PR machinery as the installed-skill route
/// (`run_registry_proposal`). For an update candidate (`kind = update`,
/// `target_skill_id` set), the target skill's evolution history is pulled in
/// so the PR description carries the version diff / changelog. The temp
/// directory is removed once the proposal returns.
#[utoipa::path(
    post,
    path = "/api/skills/pending/{id}/propose-to-registry",
    tag = "skills",
    params(("id" = String, Path, description = "Candidate UUID")),
    responses(
        (status = 200, description = "PR opened against the registry", body = crate::types::JsonObject),
        (status = 400, description = "Invalid candidate id"),
        (status = 401, description = "No GitHub token configured"),
        (status = 404, description = "Candidate not found"),
        (status = 502, description = "GitHub request failed")
    )
)]
pub async fn propose_pending_to_registry(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let skills_root = state.kernel.home_dir().join("skills");
    let candidate =
        match librefang_kernel::skill_workshop::storage::load_candidate(&skills_root, &id) {
            Ok(c) => c,
            Err(librefang_kernel::skill_workshop::WorkshopError::InvalidId(_)) => {
                return ApiErrorResponse::bad_request(format!(
                    "invalid candidate id (must be a UUID): {id}"
                ))
                .into_json_tuple();
            }
            Err(librefang_kernel::skill_workshop::WorkshopError::NotFound(_)) => {
                return ApiErrorResponse::not_found(format!("candidate '{id}' not found"))
                    .into_json_tuple();
            }
            Err(e) => return ApiErrorResponse::internal_scrub(e).into_json_tuple(),
        };

    let Some(token) = resolve_github_token(&state) else {
        return ApiErrorResponse::unauthorized(
            "No GitHub token configured. Connect GitHub in Settings or set GITHUB_TOKEN.",
        )
        .into_json_tuple();
    };

    let registry_repo = state
        .kernel
        .config_snapshot()
        .skills
        .registry_repo
        .clone()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| librefang_skills::registry_pr::DEFAULT_REGISTRY_REPO.to_string());

    // For an update candidate, carry the target skill's evolution history
    // into the PR description (version diff / changelog). For a create, or
    // when the target no longer exists, start from an empty history.
    let evolution = candidate
        .target_skill_id
        .as_deref()
        .and_then(|target| clone_installed_skill(&state, target).ok())
        .map(|s| librefang_skills::evolution::get_evolution_info(&s))
        .unwrap_or_default();

    // Stage the candidate as an InstalledSkill in a throwaway temp dir.
    // The version shown in the PR reflects the proposed bump when known.
    let version = candidate
        .proposed_version
        .clone()
        .or_else(|| candidate.current_version.clone())
        .unwrap_or_else(|| "0.1.0".to_string());

    let staged = match stage_candidate_skill(&skills_root, &candidate, &version) {
        Ok(s) => s,
        Err(e) => return ApiErrorResponse::internal_scrub(e.to_string()).into_json_tuple(),
    };

    let response = run_registry_proposal(&staged.skill, &evolution, &registry_repo, &token).await;

    // Best-effort cleanup of the staging directory — the proposal is done
    // (success or failure) and the temp tree is no longer needed.
    let _ = std::fs::remove_dir_all(&staged.dir);

    response
}

/// A candidate skill staged on disk for a registry proposal.
struct StagedCandidate {
    /// The temp directory root to remove after proposing.
    dir: std::path::PathBuf,
    /// The InstalledSkill pointing at the staged files.
    skill: librefang_skills::InstalledSkill,
}

/// Write a pending candidate's content (`skill.toml` + `prompt_context.md`)
/// into a fresh temp directory under `<skills>/.propose-tmp/<id>/<name>/` and
/// return an [`librefang_skills::InstalledSkill`] pointing at it. The caller
/// removes the temp tree once the proposal returns. Staging under the skills
/// root (rather than the OS temp dir) keeps the files on the same volume and
/// inside the librefang home, and avoids pulling in a new runtime dependency.
fn stage_candidate_skill(
    skills_root: &std::path::Path,
    candidate: &librefang_kernel::skill_workshop::candidate::CandidateSkill,
    version: &str,
) -> std::io::Result<StagedCandidate> {
    let root = skills_root.join(".propose-tmp").join(&candidate.id);
    // Clear any leftovers from a previous failed attempt for this candidate.
    let _ = std::fs::remove_dir_all(&root);
    let skill_dir = root.join(&candidate.name);
    std::fs::create_dir_all(&skill_dir)?;

    let manifest = librefang_skills::SkillManifest {
        skill: librefang_skills::SkillMeta {
            name: candidate.name.clone(),
            version: version.to_string(),
            description: candidate.description.clone(),
            author: "agent-evolved".to_string(),
            license: String::new(),
            tags: Vec::new(),
        },
        runtime: librefang_skills::SkillRuntimeConfig {
            runtime_type: librefang_skills::SkillRuntime::PromptOnly,
            entry: String::new(),
        },
        tools: librefang_skills::SkillTools::default(),
        requirements: Default::default(),
        prompt_context: None, // body lives in prompt_context.md
        source: Some(librefang_skills::SkillSource::Local),
        config: HashMap::new(),
        config_vars: Vec::new(),
        env_passthrough: Vec::new(),
    };

    let toml_str = toml::to_string_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(skill_dir.join("skill.toml"), toml_str)?;
    std::fs::write(
        skill_dir.join("prompt_context.md"),
        &candidate.prompt_context,
    )?;

    Ok(StagedCandidate {
        dir: root,
        skill: librefang_skills::InstalledSkill {
            manifest,
            path: skill_dir,
            enabled: true,
        },
    })
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
        (status = 200, description = "File contents", body = crate::types::JsonObject),
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
            return ApiErrorResponse::internal_scrub(e).into_json_tuple();
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

/// POST /api/skills/{name}/evolve/update — full-rewrite prompt_context.
#[utoipa::path(
    post,
    path = "/api/skills/{name}/evolve/update",
    tag = "skills",
    params(("name" = String, Path, description = "Skill name")),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Skill updated", body = crate::types::JsonObject),
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
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Skill patched", body = crate::types::JsonObject),
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
        (status = 200, description = "Skill rolled back", body = crate::types::JsonObject),
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
        (status = 200, description = "Skill deleted", body = crate::types::JsonObject),
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
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "File written", body = crate::types::JsonObject),
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
        (status = 200, description = "File removed", body = crate::types::JsonObject),
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
