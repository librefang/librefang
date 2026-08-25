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

    /// Concurrent writers to the same identity file previously shared one
    /// staging path (`.{filename}.tmp`), so each `fs::write` truncated whatever
    /// the other had staged and the surviving file could hold interleaved
    /// bytes from both payloads rather than either one intact.
    ///
    /// The payloads differ in length so a torn result is detectable: a
    /// prefix-length match would still fail the whole-content comparison.
    #[test]
    fn concurrent_writes_leave_one_payload_intact() {
        const WRITERS: usize = 8;
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().to_path_buf();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(WRITERS));

        let payloads: Vec<String> = (0..WRITERS)
            .map(|index| format!("payload-{index}").repeat(index * 500 + 1))
            .collect();

        std::thread::scope(|scope| {
            for payload in &payloads {
                let workspace = workspace.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    write_identity_file(&workspace, "SOUL.md", payload)
                        .expect("write must succeed");
                });
            }
        });

        let written = std::fs::read_to_string(workspace.join(".identity/SOUL.md")).unwrap();
        assert!(
            payloads.contains(&written),
            "content must equal exactly one payload, not a mix; got {} bytes",
            written.len()
        );

        let leftovers: Vec<_> = std::fs::read_dir(workspace.join(".identity"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name != "SOUL.md")
            .collect();
        assert!(
            leftovers.is_empty(),
            "no staging file may survive a successful write; found {leftovers:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn identity_file_helpers_reject_symlink_escape() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join(".identity")).unwrap();
        let outside_file = outside.path().join("secret");
        std::fs::write(&outside_file, "secret").unwrap();
        std::os::unix::fs::symlink(&outside_file, workspace.path().join(".identity/SOUL.md"))
            .unwrap();

        assert!(matches!(
            read_identity_file(workspace.path(), "SOUL.md"),
            Err(IdentityFileMutationError::Forbidden)
        ));
        assert!(matches!(
            delete_identity_file(workspace.path(), "SOUL.md"),
            Err(IdentityFileMutationError::Forbidden)
        ));
        assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "secret");
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

    let filename_for_read = filename.clone();
    drop(t);
    let read_result =
        tokio::task::spawn_blocking(move || read_identity_file(&workspace, &filename_for_read))
            .await;
    let t = ErrorTranslator::new(resolved_lang);
    let content = match read_result {
        Ok(Ok(content)) => content,
        Ok(Err(IdentityFileMutationError::Workspace)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
        Ok(Err(IdentityFileMutationError::NotFound | IdentityFileMutationError::Io(_))) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
        Ok(Err(IdentityFileMutationError::Forbidden)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
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

#[derive(Debug)]
enum IdentityFileMutationError {
    Workspace,
    NotFound,
    Forbidden,
    Io(std::io::Error),
}

fn resolve_identity_file(
    workspace: &std::path::Path,
    filename: &str,
) -> Result<std::path::PathBuf, IdentityFileMutationError> {
    let ws_canonical = workspace
        .canonicalize()
        .map_err(|_| IdentityFileMutationError::Workspace)?;
    let identity_candidate = ws_canonical.join(".identity").join(filename);
    let file_path = if identity_candidate.exists() {
        identity_candidate
    } else {
        ws_canonical.join(filename)
    };
    let canonical = file_path
        .canonicalize()
        .map_err(|_| IdentityFileMutationError::NotFound)?;
    if !canonical.starts_with(&ws_canonical) {
        return Err(IdentityFileMutationError::Forbidden);
    }
    Ok(canonical)
}

fn read_identity_file(
    workspace: &std::path::Path,
    filename: &str,
) -> Result<String, IdentityFileMutationError> {
    let path = resolve_identity_file(workspace, filename)?;
    std::fs::read_to_string(path).map_err(IdentityFileMutationError::Io)
}

fn write_identity_file(
    workspace: &std::path::Path,
    filename: &str,
    content: &str,
) -> Result<(), IdentityFileMutationError> {
    let ws_canonical = workspace
        .canonicalize()
        .map_err(|_| IdentityFileMutationError::Workspace)?;
    let identity_dir = workspace.join(".identity");
    std::fs::create_dir_all(&identity_dir).map_err(IdentityFileMutationError::Io)?;
    let file_path = identity_dir.join(filename);

    let canonical_identity = identity_dir
        .canonicalize()
        .map_err(IdentityFileMutationError::Io)?;
    if !canonical_identity.starts_with(&ws_canonical) {
        return Err(IdentityFileMutationError::Forbidden);
    }

    // Staging through `.{filename}.tmp` gave every writer of the same identity
    // file the same staging path, so two concurrent `PUT`s truncated each
    // other's staged bytes before either rename — the surviving content was
    // whichever write happened to finish last, interleaved. The shared
    // `atomic_write` derives its temp name from the process ID and a
    // per-process counter, and additionally fsyncs the staged file before the
    // rename and the parent directory after it, which the plain
    // `fs::write` + `rename` here did neither of.
    crate::atomic_write(&file_path, content.as_bytes()).map_err(IdentityFileMutationError::Io)
}

fn delete_identity_file(
    workspace: &std::path::Path,
    filename: &str,
) -> Result<(), IdentityFileMutationError> {
    let path = resolve_identity_file(workspace, filename)?;
    std::fs::remove_file(path).map_err(IdentityFileMutationError::Io)
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
    let filename_for_write = filename.clone();
    drop(t);
    let result = tokio::task::spawn_blocking(move || {
        write_identity_file(&workspace, &filename_for_write, &req.content)
    })
    .await;
    let t = ErrorTranslator::new(resolved_lang);
    match result {
        Ok(Ok(())) => {}
        Ok(Err(IdentityFileMutationError::Workspace)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
        Ok(Err(IdentityFileMutationError::Forbidden)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
            );
        }
        Ok(Err(IdentityFileMutationError::Io(error))) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
            );
        }
        Ok(Err(IdentityFileMutationError::NotFound)) => {
            return ApiErrorResponse::internal_scrub(
                "identity-file write unexpectedly resolved as not found",
            )
            .into_json_tuple();
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
            );
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

    let filename_for_delete = filename.clone();
    drop(t);
    let result =
        tokio::task::spawn_blocking(move || delete_identity_file(&workspace, &filename_for_delete))
            .await;
    let t = ErrorTranslator::new(resolved_lang);
    match result {
        Ok(Ok(())) => {}
        Ok(Err(IdentityFileMutationError::Workspace)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": t.t("api-error-file-workspace-error")})),
            );
        }
        Ok(Err(IdentityFileMutationError::NotFound)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": t.t("api-error-file-not-found")})),
            );
        }
        Ok(Err(IdentityFileMutationError::Forbidden)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": t.t("api-error-file-path-traversal")})),
            );
        }
        Ok(Err(IdentityFileMutationError::Io(error))) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": scrub_500(&error, &t)})),
            );
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
