use super::*;

#[derive(Debug)]
enum IdentityFileError {
    Workspace,
    NotFound,
    Traversal,
    Io(std::io::Error),
}

fn resolve_identity_file(
    workspace: &std::path::Path,
    filename: &str,
) -> Result<std::path::PathBuf, IdentityFileError> {
    let workspace = workspace
        .canonicalize()
        .map_err(|_| IdentityFileError::Workspace)?;
    let identity_candidate = workspace.join(".identity").join(filename);
    let candidate = if identity_candidate.exists() {
        identity_candidate
    } else {
        workspace.join(filename)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|_| IdentityFileError::NotFound)?;
    if !canonical.starts_with(&workspace) {
        return Err(IdentityFileError::Traversal);
    }
    Ok(canonical)
}

fn read_identity_file(
    workspace: &std::path::Path,
    filename: &str,
) -> Result<String, IdentityFileError> {
    let path = resolve_identity_file(workspace, filename)?;
    std::fs::read_to_string(path).map_err(IdentityFileError::Io)
}

fn write_identity_file(
    workspace: &std::path::Path,
    filename: &str,
    content: &[u8],
) -> Result<(), IdentityFileError> {
    let workspace = workspace
        .canonicalize()
        .map_err(|_| IdentityFileError::Workspace)?;
    let identity_dir = workspace.join(".identity");
    std::fs::create_dir_all(&identity_dir).map_err(IdentityFileError::Io)?;
    let identity_dir = identity_dir.canonicalize().map_err(IdentityFileError::Io)?;
    if !identity_dir.starts_with(&workspace) {
        return Err(IdentityFileError::Traversal);
    }
    crate::atomic_write(&identity_dir.join(filename), content).map_err(IdentityFileError::Io)
}

fn delete_identity_file(
    workspace: &std::path::Path,
    filename: &str,
) -> Result<(), IdentityFileError> {
    let path = resolve_identity_file(workspace, filename)?;
    std::fs::remove_file(path).map_err(IdentityFileError::Io)
}

#[cfg(test)]
mod identity_file_io_tests {
    use super::*;

    #[test]
    fn identity_file_helpers_round_trip_current_layout() {
        let temp = tempfile::tempdir().unwrap();

        write_identity_file(temp.path(), "SOUL.md", b"hello").unwrap();
        assert_eq!(read_identity_file(temp.path(), "SOUL.md").unwrap(), "hello");
        assert!(temp.path().join(".identity/SOUL.md").is_file());
        assert_eq!(
            std::fs::read_dir(temp.path().join(".identity"))
                .unwrap()
                .count(),
            1
        );

        delete_identity_file(temp.path(), "SOUL.md").unwrap();
        assert!(matches!(
            read_identity_file(temp.path(), "SOUL.md"),
            Err(IdentityFileError::NotFound)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn identity_file_helpers_reject_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".identity")).unwrap();
        std::fs::write(outside.path().join("secret"), "secret").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret"),
            temp.path().join(".identity/SOUL.md"),
        )
        .unwrap();

        assert!(matches!(
            read_identity_file(temp.path(), "SOUL.md"),
            Err(IdentityFileError::Traversal)
        ));
        assert!(matches!(
            delete_identity_file(temp.path(), "SOUL.md"),
            Err(IdentityFileError::Traversal)
        ));
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret")).unwrap(),
            "secret"
        );
    }
}

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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    Path(id): Path<String>,
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

    let entry = match state.kernel.agent_registry().get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
            );
        }
    };
    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-agent-no-workspace")})),
            );
        }
    };

    // `ErrorTranslator` is `!Send`, so it must be dropped before the
    // `.await` and re-created afterwards, matching the established
    // pattern in `get_agent_file` below (#3579).
    drop(t);
    let files = match tokio::task::spawn_blocking(move || list_identity_files(&workspace)).await {
        Ok(files) => files,
        Err(error) => {
            let t = ErrorTranslator::new(resolved_lang);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
            );
        }
    };

    (StatusCode::OK, Json(serde_json::json!({ "files": files })))
}

fn list_identity_files(workspace: &std::path::Path) -> Vec<serde_json::Value> {
    KNOWN_IDENTITY_FILES
        .iter()
        .map(|&name| {
            // Check .identity/ first (current layout), then workspace root (pre-migration fallback)
            let identity_path = workspace.join(".identity").join(name);
            let path = if identity_path.exists() {
                identity_path
            } else {
                workspace.join(name)
            };
            let metadata = std::fs::metadata(path).ok();
            serde_json::json!({
                "name": name,
                "exists": metadata.is_some(),
                "size_bytes": metadata.map(|value| value.len()).unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
mod identity_file_list_tests {
    use super::*;

    #[test]
    fn list_identity_files_prefers_current_layout_and_reports_sizes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".identity")).unwrap();
        std::fs::write(temp.path().join("SOUL.md"), "legacy").unwrap();
        std::fs::write(temp.path().join(".identity/SOUL.md"), "current").unwrap();

        let files = list_identity_files(temp.path());
        let soul = files.iter().find(|file| file["name"] == "SOUL.md").unwrap();
        assert_eq!(soul["exists"], true);
        assert_eq!(soul["size_bytes"], 7);
        assert!(files.iter().any(|file| file["exists"] == false));
    }
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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

    if !super::super::can_access_agent(&state, agent_id, api_user.as_ref()) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": t.t("api-error-agent-not-found")})),
        );
    }

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

    drop(t);
    let filename_for_task = filename.clone();
    let read_result =
        tokio::task::spawn_blocking(move || read_identity_file(&workspace, &filename_for_task))
            .await;
    let t = ErrorTranslator::new(resolved_lang);
    let content = match read_result {
        Ok(Ok(content)) => content,
        Ok(Err(IdentityFileError::Workspace)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
        Ok(Err(IdentityFileError::Traversal)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
            );
        }
        Ok(Err(IdentityFileError::NotFound | IdentityFileError::Io(_))) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
        Err(error) => {
            return ApiErrorResponse::internal_scrub(format!(
                "agent identity file read task failed: {error}"
            ))
            .into_json_tuple();
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

    let size_bytes = req.content.len();
    let content = req.content.into_bytes();
    let filename_for_task = filename.clone();
    drop(t);
    let result = tokio::task::spawn_blocking(move || {
        write_identity_file(&workspace, &filename_for_task, &content)
    })
    .await;
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(IdentityFileError::Workspace)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
        Ok(Err(IdentityFileError::Traversal)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
            );
        }
        Ok(Err(IdentityFileError::Io(error))) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
            );
        }
        Ok(Err(IdentityFileError::NotFound)) => unreachable!(),
        Err(error) => {
            return ApiErrorResponse::internal_scrub(format!(
                "agent identity file write task failed: {error}"
            ))
            .into_json_tuple();
        }
    }
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

    let filename_for_task = filename.clone();
    drop(t);
    let result =
        tokio::task::spawn_blocking(move || delete_identity_file(&workspace, &filename_for_task))
            .await;
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(IdentityFileError::Workspace)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
        Ok(Err(IdentityFileError::NotFound)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
        Ok(Err(IdentityFileError::Traversal)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
            );
        }
        Ok(Err(IdentityFileError::Io(error))) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
            );
        }
        Err(error) => {
            return ApiErrorResponse::internal_scrub(format!(
                "agent identity file delete task failed: {error}"
            ))
            .into_json_tuple();
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
        })),
    )
}
