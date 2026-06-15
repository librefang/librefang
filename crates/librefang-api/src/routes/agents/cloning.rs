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

    let source = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    // Deep-clone manifest with new name
    let mut cloned_manifest = source.manifest.clone();
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

    // Copy workspace identity files from source to destination
    let new_entry = state.kernel.agent_registry().get(new_id);
    if let (Some(ref src_ws), Some(ref new_entry)) = (source.manifest.workspace, new_entry) {
        if let Some(ref dst_ws) = new_entry.manifest.workspace {
            // Security: canonicalize both paths
            if let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) {
                let src_identity = src_can.join(".identity");
                let dst_identity = dst_can.join(".identity");
                if let Err(e) = std::fs::create_dir_all(&dst_identity) {
                    tracing::warn!("Failed to create identity directory for cloned agent: {e}");
                }
                for &fname in KNOWN_IDENTITY_FILES {
                    // Source: prefer .identity/ (post-migration), fall back to workspace root
                    let src_file = if src_identity.join(fname).exists() {
                        src_identity.join(fname)
                    } else {
                        src_can.join(fname)
                    };
                    let dst_file = dst_identity.join(fname);
                    if src_file.exists() {
                        if let Err(e) = std::fs::copy(&src_file, &dst_file) {
                            tracing::warn!("Failed to copy identity file {fname}: {e}");
                        }
                    }
                }
            }
        }
    }

    // Copy identity from source
    if let Err(e) = state
        .kernel
        .agent_registry()
        .update_identity(new_id, source.identity.clone())
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
