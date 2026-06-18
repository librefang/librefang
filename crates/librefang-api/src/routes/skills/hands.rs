use super::*;

/// GET /api/hands — List all hand definitions (marketplace).
#[utoipa::path(
    get,
    path = "/api/hands",
    tag = "hands",
    responses(
        (status = 200, description = "List all hand definitions", body = crate::types::JsonObject)
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

    let total = hands.len();
    Json(crate::types::PaginatedResponse {
        items: hands,
        total,
        offset: 0,
        limit: None,
    })
}

/// GET /api/hands/active — List active hand instances.
#[utoipa::path(
    get,
    path = "/api/hands/active",
    tag = "hands",
    responses(
        (status = 200, description = "List active hand instances", body = crate::types::JsonObject)
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

    let total = items.len();
    Json(crate::types::PaginatedResponse {
        items,
        total,
        offset: 0,
        limit: None,
    })
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
        (status = 200, description = "Get a single hand definition with requirements", body = crate::types::JsonObject)
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
                            // manifest values; per-agent edit endpoints target AgentRegistry/agent.toml, not HAND.toml, so these remain an honest restore-default target.
                            "system_prompt": a.manifest.model.system_prompt,
                            "capabilities_tools": a.manifest.capabilities.tools,
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
                // Scrub the serialize error (audit: rusqlite-errors-leak).
                tracing::error!(error = %e, "failed to serialize hand definition");
                return ApiErrorResponse::internal_scrub(e)
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
        (status = 200, description = "Re-check dependency status for a hand", body = crate::types::JsonObject)
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
        (status = 200, description = "Auto-install missing dependencies for a hand", body = crate::types::JsonObject)
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

        // Allowlist program names + reject shell-invocation flags. See
        // `validate_install_deps_argv` for the rationale and the full
        // allow/deny tables. Returns the *reason* string so the per-dep
        // result still carries a useful message; the metacharacter guard
        // above stays as a defence-in-depth predecessor (a `python -c`
        // payload that includes `;` is caught earlier and never reaches
        // this point).
        if let Err(reason) = validate_install_deps_argv(program, args) {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "error",
                "command": final_cmd,
                "message": reason,
            }));
            continue;
        }

        tracing::info!(hand = %hand_id, dep = %req.key, cmd = %final_cmd, "Auto-installing dependency");

        // `kill_on_drop(true)` so a timeout / dropped Future SIGKILLs the
        // child instead of orphaning it. Same defect class as codex fix
        // #3 on the sidecar describe subprocess: a 300s `tokio::time::timeout`
        // without `kill_on_drop` leaves the install command running in the
        // background after the timeout fires.
        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tokio::process::Command::new(program)
                .args(args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
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
                    // Serialize the env mutation through the process-global
                    // guard (#5142). `spawn_blocking` does NOT serialize — two
                    // concurrent route handlers each get their own blocking
                    // thread and `set_var` simultaneously, the exact race the
                    // Rust 1.74+ docs forbid.
                    crate::secrets_env::set_env_var_guarded("PATH", new_path).await;
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
        (status = 200, description = "Hand uninstalled", body = crate::types::JsonObject),
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
            state.kernel.invalidate_hand_route_cache();
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
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Install a hand from TOML content", body = crate::types::JsonObject)
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
            state.kernel.invalidate_hand_route_cache();
            // Return the full canonical `HandDefinition` so dashboard /
            // SDK callers can `setQueryData` on the hands list directly
            // instead of doing a follow-up GET. The previous {id, name,
            // description, category} subset forced a refetch round-trip
            // and was inconsistent with how list_hands serializes hand
            // metadata. Refs #3832.
            //
            // We materialise as `serde_json::Value` so the OK arm and the
            // Err arm (`ApiErrorResponse::into_json_tuple()`) line up on
            // `Json<serde_json::Value>` — the tuple's match arms must
            // share a body type.
            let body = serde_json::to_value(&def).unwrap_or(serde_json::Value::Null);
            (StatusCode::OK, Json(body))
        }
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}

/// POST /api/hands/{hand_id}/activate — Activate a hand (spawns agent).
///
/// Honours `Idempotency-Key` (#3637): when set, a duplicate request
/// with the same key + same body replays the cached response instead
/// of activating a second hand instance. A different body under the
/// same key is rejected with 409 Conflict.
#[utoipa::path(
    post,
    path = "/api/hands/{hand_id}/activate",
    tag = "hands",
    params(
        ("hand_id" = String, Path, description = "Hand ID"),
    ),
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Activate a hand (spawns agent)", body = crate::types::JsonObject),
        (status = 409, description = "Idempotency-Key was reused with a different request body")
    )
)]
pub async fn activate_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let key = crate::idempotency::extract_key(&headers);
    let body_bytes: Vec<u8> = body.to_vec();
    let store = Arc::clone(&state.idempotency_store);
    let inner_body = body_bytes.clone();

    crate::idempotency::run_idempotent(
        store.as_ref(),
        key.as_deref(),
        &body_bytes,
        move || async move { activate_hand_inner(state, hand_id, &inner_body).await },
    )
    .await
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
        (status = 200, description = "Pause a hand instance", body = crate::types::JsonObject)
    )
)]
pub async fn pause_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.pause_hand(id) {
        Ok(()) => match state.kernel.hands().get_instance(id) {
            // #3832: return the post-mutation entity instead of an ack envelope
            // so the dashboard can setQueryData without a follow-up GET.
            Some(instance) => (StatusCode::OK, Json(hand_instance_to_json(&instance))),
            None => {
                ApiErrorResponse::internal(format!("hand instance {id} disappeared after pause"))
                    .into_json_tuple()
            }
        },
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
        (status = 200, description = "Resume a paused hand instance", body = crate::types::JsonObject)
    )
)]
pub async fn resume_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.resume_hand(id) {
        Ok(()) => match state.kernel.hands().get_instance(id) {
            // #3832: return the post-mutation entity instead of an ack envelope
            // so the dashboard can setQueryData without a follow-up GET.
            Some(instance) => (StatusCode::OK, Json(hand_instance_to_json(&instance))),
            None => {
                ApiErrorResponse::internal(format!("hand instance {id} disappeared after resume"))
                    .into_json_tuple()
            }
        },
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
        (status = 200, description = "Deactivate a hand (kills agent)", body = crate::types::JsonObject)
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
    request_body = crate::types::JsonObject,
    responses((status = 200, description = "Secret saved", body = crate::types::JsonObject))
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
        return ApiErrorResponse::internal_scrub(e).into_json_tuple();
    }

    // Set in current process. Serialized through the process-global env
    // write guard (#5142) — `spawn_blocking` does NOT serialize concurrent
    // env mutations, it fans out across the blocking pool.
    crate::secrets_env::set_env_var_guarded(env_key.clone(), value.clone()).await;

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
        (status = 200, description = "Get settings schema and current values", body = crate::types::JsonObject)
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
    request_body = crate::types::JsonObject,
    responses(
        (status = 200, description = "Update settings for a hand instance", body = crate::types::JsonObject)
    )
)]
pub async fn update_hand_settings(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    Json(config): Json<std::collections::HashMap<String, serde_json::Value>>,
) -> impl IntoResponse {
    let instance = state
        .kernel
        .hands()
        .list_instances()
        .into_iter()
        .find(|i| i.hand_id == hand_id);

    match instance {
        Some(inst) => {
            let id = inst.instance_id;
            // Merge over existing config so untouched keys keep their saved values.
            let mut merged = inst.config;
            merged.extend(config);
            match state.kernel.hands().update_config(id, merged.clone()) {
                Ok(()) => {
                    state.kernel.persist_hand_state();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "ok",
                            "hand_id": hand_id,
                            "instance_id": id,
                            "config": merged,
                        })),
                    )
                }
                Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
            }
        }
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
        (status = 200, description = "Reload hand definitions from disk", body = crate::types::JsonObject)
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
        (status = 200, description = "Get dashboard stats for a hand instance", body = crate::types::JsonObject)
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
        (status = 200, description = "Get live browser state for a hand instance", body = crate::types::JsonObject)
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
            librefang_kernel::browser::BrowserCommand::ReadPage,
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
            librefang_kernel::browser::BrowserCommand::Screenshot,
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

    // Reject oversized messages — see check_message_size for the
    // byte/char split. Audit: message-byte-vs-char-cap.
    if let Err(e) = crate::validation::check_message_size(&req.message) {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": e.message})),
        );
    }

    // Resolve file attachments. Hand/skill calls do not carry a channel
    // sender context (the hand call is the channel from the kernel's
    // POV), so we route the injection through the helper with
    // `sender_context = None` and the agent's persistent registry
    // session id as the fallback. This matches the
    // `send_message_with_handle` path below, which is itself a
    // `Persistent`-mode dispatch landing on `entry.session_id`. Without
    // this signature change the helper would silently land on the same
    // session anyway, but it would do so via the kernel's "agent
    // default" branch — the very fallback we are removing for
    // channel-scoped callers. Threading the explicit fallback keeps the
    // hand path's behaviour byte-identical while closing the cross-chat
    // leak on the agent message path.
    if !req.attachments.is_empty() {
        let image_blocks = super::agents::resolve_attachments(&state, &req.attachments);
        if !image_blocks.is_empty() {
            let fallback_session_id = state
                .kernel
                .agent_registry()
                .get(agent_id)
                .map(|e| e.session_id)
                .unwrap_or_else(librefang_types::agent::SessionId::new);
            super::agents::inject_attachments_into_session(
                state.kernel.as_ref(),
                agent_id,
                None,
                None,
                fallback_session_id,
                image_blocks,
            );
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
            .send_message_ephemeral(agent_id, &effective_message, None)
            .await
    } else {
        let kernel_handle: Arc<dyn librefang_kernel::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn librefang_kernel::kernel_handle::KernelHandle>;
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
                    // Hands do not surface an auto-pinnable session id via
                    // this body (#5199 is dashboard-chat-only). Field
                    // omitted when None via `skip_serializing_if`.
                    session_id: None,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("hand_send_message failed for instance {id}: {e}");
            ApiErrorResponse::internal_scrub(e).into_json_tuple()
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
        Err(e) => ApiErrorResponse::internal_scrub(e).into_json_tuple(),
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

/// POST /api/hands/marketplace/install — Install a hand from the remote
/// HandsHub marketplace.
///
/// Runs the same security pipeline as the local installer plus a network
/// download: a caller-supplied `registry_url` is SSRF-checked and its
/// validated address is pinned onto a redirect-disabled HTTP client (#5954
/// F1); SHA-256 verification of the bundle against the registry digest — and
/// for a third-party registry a digest is *required* (#5954 F4); the shared
/// `librefang_skills::supply_chain::scan` audit (reused from the skills
/// marketplace); and a bundle-id == requested-id assertion (#5954 F3). The
/// verified content is funneled into the existing persisted-install path, so
/// the on-disk layout is identical to a local install. See
/// `HandRegistry::install_from_remote`.
///
/// Status contract: 200 with the installed `HandDefinition`; 409 when the hand
/// is already installed; 422 when the supply-chain audit blocks the bundle;
/// 400 for everything else (bad id, rejected `registry_url`, download /
/// checksum failure, bundle-id mismatch, missing checksum from a third-party
/// registry, parse error).
#[utoipa::path(
    post,
    path = "/api/hands/marketplace/install",
    tag = "hands",
    request_body = crate::types::HandsHubInstallRequest,
    responses(
        (status = 200, description = "Install a hand from the HandsHub marketplace", body = crate::types::JsonObject),
        (status = 409, description = "Hand already installed"),
        (status = 422, description = "Bundle blocked by the supply-chain audit")
    )
)]
pub async fn install_hand_from_marketplace(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::types::HandsHubInstallRequest>,
) -> impl IntoResponse {
    if req.hand_id.trim().is_empty() {
        return ApiErrorResponse::bad_request("Missing hand_id field").into_json_tuple();
    }

    // `require_checksum` is the F4 trust gate: a caller-supplied third-party
    // `registry_url` MUST advertise a SHA-256 (rejected otherwise); the
    // compiled-in default registry (`hands.librefang.ai`) keeps its existing
    // best-effort behaviour (WARN + install when the index omits a digest).
    let (hub, require_checksum) = match req.registry_url.as_deref() {
        // Caller-supplied registry URL: validate against the SSRF guard before
        // pointing the daemon at it. Without this an authenticated caller could
        // aim `download_bundle` at loopback / private / cloud-metadata addresses
        // (e.g. http://169.254.169.254/...). The operator allowlist
        // (`config.toml: [hands] registry_allowed_hosts`) exempts self-hosted
        // mirrors on internal networks; it defaults empty (public-only) and can
        // never exempt cloud-metadata ranges. `check_ssrf` resolves DNS
        // synchronously, so it runs on a blocking thread (no sync I/O in the
        // async handler).
        //
        // The resolution it returns is NOT discarded: the validated hostname +
        // IPs are pinned onto the HandsHub client so the address we checked is
        // the address the bundle/index fetch connects to (closing the
        // DNS-rebind TOCTOU). Auto-redirects are disabled inside the client, so
        // a registry that passed the check cannot 302 the fetch to an internal
        // target.
        Some(url) => {
            let allowed_hosts = state
                .kernel
                .config_snapshot()
                .hands
                .registry_allowed_hosts
                .clone();
            let url = url.to_string();
            let check = tokio::task::spawn_blocking(move || {
                librefang_kernel::web_fetch::check_ssrf(&url, &allowed_hosts)
                    .map(|resolution| (url, resolution))
            })
            .await;
            let (validated_url, resolution) = match check {
                Ok(Ok(pair)) => pair,
                Ok(Err(e)) => {
                    return ApiErrorResponse::bad_request(format!("registry_url rejected: {e}"))
                        .into_json_tuple();
                }
                Err(e) => {
                    return ApiErrorResponse::bad_request(format!(
                        "registry_url validation failed: {e}"
                    ))
                    .into_json_tuple();
                }
            };
            let pinned_ips: Vec<std::net::IpAddr> =
                resolution.resolved.iter().map(|addr| addr.ip()).collect();
            (
                librefang_hands::HandsHubClient::with_pinned_url(
                    &validated_url,
                    &resolution.hostname,
                    &pinned_ips,
                ),
                true,
            )
        }
        // Default registry (`hands.librefang.ai`) is trusted — skip the check
        // and keep the best-effort checksum policy.
        None => (librefang_hands::HandsHubClient::new(), false),
    };

    let home_dir = state.kernel.home_dir().to_path_buf();
    match state
        .kernel
        .hands()
        .install_from_remote(&home_dir, &req.hand_id, &hub, require_checksum)
        .await
    {
        Ok(result) => {
            state.kernel.invalidate_hand_route_cache();
            // Return the freshly installed canonical `HandDefinition` (plus the
            // resolved version and checksum-verification flag) so dashboard /
            // SDK callers can `setQueryData` without a follow-up GET — same
            // contract as the local `install_hand` handler.
            let def = state.kernel.hands().get_definition(&result.hand_id);
            let body = serde_json::json!({
                "hand_id": result.hand_id,
                "version": result.version,
                "checksum_verified": result.checksum_verified,
                "definition": def,
            });
            (StatusCode::OK, Json(body))
        }
        Err(e @ librefang_hands::HandError::AlreadyActive(_))
        | Err(e @ librefang_hands::HandError::AlreadyRegistered(_)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
        Err(e @ librefang_hands::HandError::SecurityBlocked(_)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
        Err(e) => ApiErrorResponse::bad_request(format!("{e}")).into_json_tuple(),
    }
}
