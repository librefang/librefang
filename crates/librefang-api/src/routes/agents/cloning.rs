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

/// POST /api/agents/{id}/clone — Clone an agent with its workspace files.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/clone",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = CloneAgentRequest, description = "New name for the cloned agent"),
    responses(
        (status = 200, description = "Clone an agent with its workspace files", body = crate::types::JsonObject)
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
    if let (Some(src_ws), Some(dst_ws)) = (source_workspace, destination_workspace) {
        if let Err(error) =
            tokio::task::spawn_blocking(move || copy_clone_identity_files(&src_ws, &dst_ws)).await
        {
            tracing::error!(%error, "cloned agent identity copy task failed");
        }
    }

    // Copy identity from source
    if let Err(e) = state
        .kernel
        .agent_registry()
        .update_identity(new_id, source_identity)
    {
        tracing::warn!("Failed to copy agent identity: {e}");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "agent_id": new_id.to_string(),
            "name": req.new_name,
        })),
    )
}

fn copy_clone_identity_files(src_ws: &std::path::Path, dst_ws: &std::path::Path) {
    // Security: canonicalize both paths before constructing identity paths.
    let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) else {
        return;
    };
    let src_identity = src_can.join(".identity");
    let dst_identity = dst_can.join(".identity");
    if let Err(error) = std::fs::create_dir_all(&dst_identity) {
        tracing::warn!(%error, "failed to create identity directory for cloned agent");
    }
    for &filename in KNOWN_IDENTITY_FILES {
        // Source: prefer .identity/ (post-migration), fall back to workspace root.
        let migrated_source = src_identity.join(filename);
        let source = if migrated_source.exists() {
            migrated_source
        } else {
            src_can.join(filename)
        };
        if source.exists() {
            let destination = dst_identity.join(filename);
            if let Err(error) = std::fs::copy(&source, destination) {
                tracing::warn!(%error, %filename, "failed to copy cloned agent identity file");
            }
        }
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

        copy_clone_identity_files(&source, &destination);

        assert_eq!(
            std::fs::read_to_string(destination.join(".identity/SOUL.md")).unwrap(),
            "migrated soul"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join(".identity/IDENTITY.md")).unwrap(),
            "legacy identity"
        );
    }
}
