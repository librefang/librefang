use super::*;

/// GET /api/agents/{id}/files — List workspace identity files.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/files",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "List workspace identity files for an agent", body = crate::types::JsonObject)
    )
)]
pub async fn list_agent_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
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

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
            );
        }
    };

    let mut files = Vec::new();
    for &name in KNOWN_IDENTITY_FILES {
        // Check .identity/ first (current layout), then workspace root (pre-migration fallback)
        let identity_path = workspace.join(".identity").join(name);
        let path = if identity_path.exists() {
            identity_path
        } else {
            workspace.join(name)
        };
        let (exists, size_bytes) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (true, size)
        } else {
            (false, 0u64)
        };
        files.push(serde_json::json!({
            "name": name,
            "exists": exists,
            "size_bytes": size_bytes,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!({ "files": files })))
}

/// GET /api/agents/{id}/files/{filename} — Read a workspace identity file.
#[utoipa::path(
    get,
    path = "/api/agents/{id}/files/{filename}",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("filename" = String, Path, description = "Identity file name"),
    ),
    responses(
        (status = 200, description = "Read a workspace identity file", body = crate::types::JsonObject)
    )
)]
pub async fn get_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let resolved_lang = super::resolve_lang(lang.as_ref());
    let t = ErrorTranslator::new(resolved_lang);
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": t.t("api-error-agent-invalid-id")})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-file-not-in-whitelist")})),
        );
    }

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
            );
        }
    };

    // Resolve canonical path: prefer .identity/ (current layout), fall back to workspace root
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
    };

    let identity_path = workspace.join(".identity").join(&filename);
    let file_path = if identity_path.exists() {
        identity_path
    } else {
        workspace.join(&filename)
    };

    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
    };

    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
        );
    }

    // Off-runtime read so this axum handler never parks a tokio worker
    // thread on a slow disk (#3579). `ErrorTranslator` is `!Send`, so it
    // must be dropped before the `.await` and re-created afterwards or
    // axum's `Handler` bound fails to compile.
    drop(t);
    let read_result = tokio::fs::read_to_string(&canonical).await;
    let t = ErrorTranslator::new(resolved_lang);
    let content = match read_result {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
    };

    let size_bytes = content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": filename,
            "content": content,
            "size_bytes": size_bytes,
        })),
    )
}

/// Request body for writing a workspace identity file.
#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetAgentFileRequest {
    pub content: String,
}

/// PUT /api/agents/{id}/files/{filename} — Write a workspace identity file.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/files/{filename}",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("filename" = String, Path, description = "Identity file name"),
    ),
    request_body(content = SetAgentFileRequest, description = "File content to write"),
    responses(
        (status = 200, description = "Write a workspace identity file", body = crate::types::JsonObject)
    )
)]
#[allow(private_interfaces)]
pub async fn set_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<SetAgentFileRequest>,
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

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-file-not-in-whitelist")})),
        );
    }

    // Max 32KB content
    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": t.t("api-error-file-too-large")})),
        );
    }

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
            );
        }
    };

    // Security: verify workspace path and target stays inside it
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
    };

    // Always write to .identity/ (current layout)
    let identity_dir = workspace.join(".identity");
    if let Err(e) = std::fs::create_dir_all(&identity_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        );
    }
    let file_path = identity_dir.join(&filename);

    // Security: ensure .identity/ is inside the workspace
    let check_path = identity_dir
        .canonicalize()
        .map(|p| p.join(&filename))
        .unwrap_or_else(|_| file_path.clone());
    if !check_path.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
        );
    }

    // Atomic write: write to .tmp, then rename
    let tmp_path = identity_dir.join(format!(".{filename}.tmp"));
    if let Err(e) = std::fs::write(&tmp_path, &req.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        );
    }
    if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
        if let Err(e) = std::fs::remove_file(&tmp_path) {
            tracing::warn!("Failed to remove temporary file: {e}");
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        );
    }

    let size_bytes = req.content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
            "size_bytes": size_bytes,
        })),
    )
}

/// DELETE /api/agents/{id}/files/{filename} — Delete a workspace identity file.
#[utoipa::path(
    delete,
    path = "/api/agents/{id}/files/{filename}",
    tag = "agents",
    params(
        ("id" = String, Path, description = "Agent ID"),
        ("filename" = String, Path, description = "Identity file name"),
    ),
    responses(
        (status = 200, description = "File deleted successfully", body = crate::types::JsonObject),
        (status = 404, description = "File not found", body = crate::types::JsonObject)
    )
)]
pub async fn delete_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    lang: Option<axum::Extension<RequestLanguage>>,
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

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": t.t("api-error-file-not-in-whitelist")})),
        );
    }

    let workspace = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => match e.manifest.workspace {
            Some(ref ws) => ws.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
                );
            }
        },
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };

    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
    };

    // Resolve path: prefer .identity/ (current layout), fall back to workspace root
    let identity_candidate = workspace.join(".identity").join(&filename);
    let file_path = if identity_candidate.exists() {
        identity_candidate
    } else {
        workspace.join(&filename)
    };

    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
        );
    }

    if let Err(e) = std::fs::remove_file(&canonical) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": scrub_500(&e, &t)})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
        })),
    )
}
