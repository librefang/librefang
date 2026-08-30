use super::*;

// ---------------------------------------------------------------------------
// Agent Cloning
// ---------------------------------------------------------------------------
/// Request body for cloning an agent.
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloneAgentRequest {
    pub new_name: String,
    /// Whether to copy skills from the source agent (default: true).
    #[serde(default = "default_clone_true")]
    pub include_skills: bool,
    /// Whether to copy tools from the source agent (default: true).
    #[serde(default = "default_clone_true")]
    pub include_tools: bool,
}

fn default_clone_true() -> bool {
    true
}

fn clone_success_body(
    new_id: AgentId,
    name: String,
    warnings: Vec<&'static str>,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": new_id.to_string(),
        "name": name,
        "partial": !warnings.is_empty(),
        "warnings": warnings,
    })
}

/// POST /api/agents/{id}/clone — Clone an agent with its workspace files.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/clone",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = CloneAgentRequest, description = "New name for the cloned agent"),
    responses(
        (status = 201, description = "Agent created; response reports any partial identity-copy failures", body = crate::types::JsonObject)
    )
)]
#[allow(private_interfaces)]
pub async fn clone_agent(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    if req.new_name.len() > 256 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error": t.t_args("api-error-agent-name-too-long", &[("max", "256")])}),
            ),
        );
    }

    if req.new_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-agent-name-empty")})),
        );
    }

    let (source_manifest, source_identity) = {
        let source = match state.kernel.agent_registry().get(agent_id) {
            Some(entry) => entry,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
                );
            }
        };
        (source.manifest.clone(), source.identity.clone())
    };
    // Owner-scoping (#6753): `agent_clone` in `middleware::user_role_allows_request` deliberately lets any `User`-role caller POST `/clone` on an arbitrary agent id, unlike most other mutations, which require Admin+.
    // The clone keeps the source's `author` (not the caller's), so a non-owner still can't read it back through the agent-scoped routes above afterwards, but without this check a non-owner could still trigger unauthorized cloning of another user's agent by guessing/enumerating its UUID — spawning a duplicate agent process, copying its identity/skill/tool/workspace files onto a new instance, and consuming resources under the source owner's identity without their consent.
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    // Deep-clone manifest with new name
    let mut cloned_manifest = source_manifest.clone();
    cloned_manifest.name = req.new_name.clone();
    cloned_manifest.workspace = None; // Let kernel assign a new workspace

    // Conditionally strip skills and tools based on request flags.
    apply_clone_inclusion_flags(&mut cloned_manifest, &req);

    // Spawn the cloned agent
    let new_id = match state.kernel.spawn_agent_typed(cloned_manifest) {
        Ok(id) => id,
        Err(e) => {
            // Map AgentAlreadyExists → 409 Conflict (audit:
            // agent-not-found-returns-500). Pre-fix this branch
            // returned 500 for every `spawn_agent_typed` error
            // including the well-known duplicate-name case. The 500
            // catch-all is scrubbed via `kernel_err_body` so a clone
            // failure rooted in a kernel/SQL error never leaks the raw
            // chain.
            let status = kernel_err_to_status(&e);
            return (
                status,
                Json(serde_json::json!({"error": kernel_err_body(status, &e, &t)})),
            );
        }
    };

    // Copy workspace identity files from source to destination. Path
    // resolution and file copies are synchronous, so keep them off the Tokio
    // worker serving this request.
    let source_workspace = source_manifest.workspace;
    let destination_workspace = {
        let destination = state.kernel.agent_registry().get(new_id);
        destination.and_then(|entry| entry.manifest.workspace.clone())
    };
    drop(t);
    let mut warnings = Vec::new();
    if let Some(src_ws) = source_workspace {
        if let Some(dst_ws) = destination_workspace {
            match tokio::task::spawn_blocking(move || copy_clone_identity_files(&src_ws, &dst_ws))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(%error, %new_id, "failed to copy cloned agent identity files");
                    warnings.push("identity_files_copy_failed");
                }
                Err(error) => {
                    tracing::error!(%error, %new_id, "cloned agent identity copy task failed");
                    warnings.push("identity_files_copy_failed");
                }
            }
        } else {
            tracing::error!(%new_id, "cloned agent has no destination workspace");
            warnings.push("destination_workspace_missing");
        }
    }

    // Copy identity from source
    if let Err(e) = state
        .kernel
        .agent_registry()
        .update_identity(new_id, source_identity)
    {
        tracing::error!(error = %e, %new_id, "failed to copy cloned agent registry identity");
        warnings.push("registry_identity_copy_failed");
    }

    (
        StatusCode::CREATED,
        Json(clone_success_body(new_id, req.new_name, warnings)),
    )
}

fn copy_clone_identity_files(
    src_ws: &std::path::Path,
    dst_ws: &std::path::Path,
) -> std::io::Result<()> {
    // Security: canonicalize both paths before constructing identity paths.
    let src_can = src_ws.canonicalize()?;
    let dst_can = dst_ws.canonicalize()?;
    let src_identity = src_can.join(".identity");
    let dst_identity = dst_can.join(".identity");
    std::fs::create_dir_all(&dst_identity)?;
    // A `.identity` that exists but is not a directory has to be detected here, not through the per-file `try_exists()` below.
    // Unix surfaces the bad path component as ENOTDIR, but Windows reports ERROR_PATH_NOT_FOUND, which std maps to `NotFound` and `try_exists()` reduces to `Ok(false)` — the clone would then silently fall back to the legacy workspace-root copy and report success (#7547).
    let migrated_identity_is_not_a_directory = std::fs::metadata(&src_identity)
        .map(|metadata| !metadata.is_dir())
        .unwrap_or(false);
    let mut first_error = None;
    for &filename in KNOWN_IDENTITY_FILES {
        if migrated_identity_is_not_a_directory {
            first_error.get_or_insert_with(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    format!(
                        "failed to inspect migrated identity file {filename}: {} is not a directory",
                        src_identity.display()
                    ),
                )
            });
            continue;
        }
        // Source: prefer .identity/ (post-migration), fall back to workspace root.
        let migrated_source = src_identity.join(filename);
        let source = match migrated_source.try_exists() {
            Ok(true) => Some(migrated_source),
            Ok(false) => {
                let legacy_source = src_can.join(filename);
                match legacy_source.try_exists() {
                    Ok(true) => Some(legacy_source),
                    Ok(false) => None,
                    Err(error) => {
                        first_error.get_or_insert_with(|| {
                            std::io::Error::new(
                                error.kind(),
                                format!(
                                    "failed to inspect legacy identity file {filename}: {error}"
                                ),
                            )
                        });
                        continue;
                    }
                }
            }
            Err(error) => {
                first_error.get_or_insert_with(|| {
                    std::io::Error::new(
                        error.kind(),
                        format!("failed to inspect migrated identity file {filename}: {error}"),
                    )
                });
                continue;
            }
        };
        if let Some(source) = source {
            let destination = dst_identity.join(filename);
            if let Err(error) = std::fs::copy(&source, destination) {
                first_error.get_or_insert_with(|| {
                    std::io::Error::new(
                        error.kind(),
                        format!("failed to copy identity file {filename}: {error}"),
                    )
                });
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_identity_files_prefer_migrated_and_fall_back_to_legacy_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir_all(source.join(".identity")).expect("source identity dir");
        std::fs::create_dir_all(&destination).expect("destination workspace");

        std::fs::write(source.join("SOUL.md"), "legacy soul").expect("legacy soul");
        std::fs::write(source.join(".identity/SOUL.md"), "migrated soul").expect("migrated soul");
        std::fs::write(source.join("IDENTITY.md"), "legacy identity").expect("legacy identity");

        copy_clone_identity_files(&source, &destination).unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join(".identity/SOUL.md")).unwrap(),
            "migrated soul"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join(".identity/IDENTITY.md")).unwrap(),
            "legacy identity"
        );
    }

    #[test]
    fn clone_identity_files_report_missing_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).expect("source workspace");

        let error = copy_clone_identity_files(&source, &temp.path().join("missing"))
            .expect_err("missing destination must be reported");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn clone_identity_files_report_source_inspection_errors() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir_all(&source).expect("source workspace");
        std::fs::create_dir_all(&destination).expect("destination workspace");
        std::fs::write(source.join(".identity"), "not a directory")
            .expect("malformed identity path");
        std::fs::write(source.join("SOUL.md"), "legacy soul").expect("legacy soul");

        let error = copy_clone_identity_files(&source, &destination)
            .expect_err("malformed migrated identity path must be reported");
        assert!(error.to_string().contains("migrated identity file SOUL.md"));
        // The legacy root copy must not stand in for an unreadable `.identity`: a clone that quietly resurrects a pre-migration file is worse than one that reports the failure.
        assert!(
            !destination.join(".identity/SOUL.md").exists(),
            "unreadable .identity must not fall back to the legacy workspace-root file"
        );
    }

    #[test]
    fn clone_identity_files_continue_after_one_copy_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir_all(source.join(".identity/SOUL.md"))
            .expect("directory in place of identity file");
        std::fs::create_dir_all(&destination).expect("destination workspace");
        std::fs::write(source.join("IDENTITY.md"), "legacy identity").expect("legacy identity");

        copy_clone_identity_files(&source, &destination)
            .expect_err("directory copy must be reported");
        assert_eq!(
            std::fs::read_to_string(destination.join(".identity/IDENTITY.md")).unwrap(),
            "legacy identity"
        );
    }

    #[test]
    fn clone_response_distinguishes_complete_and_partial_creation() {
        let id = AgentId::new();
        let complete = clone_success_body(id, "complete".to_string(), Vec::new());
        assert_eq!(complete["partial"], false);
        assert_eq!(complete["warnings"], serde_json::json!([]));

        let partial = clone_success_body(
            id,
            "partial".to_string(),
            vec!["identity_files_copy_failed"],
        );
        assert_eq!(partial["partial"], true);
        assert_eq!(
            partial["warnings"],
            serde_json::json!(["identity_files_copy_failed"])
        );
    }
}
