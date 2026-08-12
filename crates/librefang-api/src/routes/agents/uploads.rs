use super::*;

// ---------------------------------------------------------------------------
// File Upload endpoints
// ---------------------------------------------------------------------------
/// Response body for file uploads.
#[derive(serde::Serialize)]
struct UploadResponse {
    file_id: String,
    filename: String,
    content_type: String,
    size: usize,
    /// Transcription text for audio uploads (populated via Whisper STT).
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<String>,
}

/// API-local name for the shared durable upload metadata contract.
pub(crate) type UploadMeta = librefang_types::media::UploadMetadata;

/// In-memory cache of upload metadata persisted beside each new file.
pub(crate) static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> =
    LazyLock::new(DashMap::new);

fn upload_meta_path(upload_dir: &std::path::Path, file_id: &str) -> std::path::PathBuf {
    librefang_types::media::upload_metadata_path(upload_dir, file_id)
}

pub(crate) fn persist_upload_meta_sync(
    upload_dir: &std::path::Path,
    file_id: &str,
    meta: &UploadMeta,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(meta)
        .map_err(|error| format!("failed to serialize upload metadata: {error}"))?;
    crate::atomic_write(&upload_meta_path(upload_dir, file_id), &bytes)
        .map_err(|error| format!("failed to persist upload metadata: {error}"))
}

pub(crate) async fn persist_upload_meta(
    upload_dir: &std::path::Path,
    file_id: &str,
    meta: &UploadMeta,
) -> Result<(), String> {
    let upload_dir = upload_dir.to_path_buf();
    let file_id = file_id.to_string();
    let meta = meta.clone();
    tokio::task::spawn_blocking(move || persist_upload_meta_sync(&upload_dir, &file_id, &meta))
        .await
        .map_err(|error| format!("upload metadata task failed: {error}"))?
}

pub(crate) async fn load_upload_meta(
    upload_dir: &std::path::Path,
    file_id: &str,
) -> Option<UploadMeta> {
    let path = upload_meta_path(upload_dir, file_id);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(%error, file_id, "failed to read upload metadata");
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(meta) => Some(meta),
        Err(error) => {
            tracing::warn!(%error, file_id, "failed to parse upload metadata");
            None
        }
    }
}

pub(crate) fn upload_access_allowed(
    meta: Option<&UploadMeta>,
    caller: Option<&crate::middleware::AuthenticatedApiUser>,
) -> bool {
    use crate::middleware::UserRole;

    match meta.and_then(|meta| meta.uploaded_by) {
        Some(owner_id) => {
            caller.is_some_and(|user| user.user_id == owner_id || user.role >= UserRole::Admin)
        }
        None if meta.is_some() => true,
        None => caller.is_none_or(|user| user.role >= UserRole::Admin),
    }
}

fn format_upload_limit(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * KIB;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= KIB && bytes.is_multiple_of(KIB) {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} bytes")
    }
}

/// POST /api/agents/{id}/upload — Upload a file attachment.
///
/// Accepts raw body bytes. The client must set:
/// - `Content-Type` header (e.g., `image/png`, `text/plain`, `application/pdf`)
/// - `X-Filename` header (original filename)
#[utoipa::path(
    post,
    path = "/api/agents/{id}/upload",
    tag = "agents",
    params(("id" = String, Path, description = "Agent ID")),
    request_body(content = String, content_type = "application/octet-stream"),
    responses(
        (status = 200, description = "Upload a file attachment for an agent", body = crate::types::JsonObject)
    )
)]
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let l = super::resolve_lang(lang.as_ref());
    let upload_limit = state.kernel.config_ref().max_upload_size_bytes;
    let upload_limit_display = format_upload_limit(upload_limit);
    let (
        err_invalid_id,
        err_unsupported_type,
        err_too_large_upload,
        err_empty_body,
        err_upload_dir_failed,
        err_upload_save_failed,
    ) = {
        let t = ErrorTranslator::new(l);
        (
            t.t("api-error-agent-invalid-id"),
            t.t("api-error-file-unsupported-type"),
            t.t_args(
                "api-error-file-too-large",
                &[("max", upload_limit_display.as_str())],
            ),
            t.t("api-error-file-empty-body"),
            t.t("api-error-file-upload-dir-failed"),
            t.t("api-error-file-save-failed"),
        )
    };
    // Validate agent ID format
    let _agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err_invalid_id})),
            );
        }
    };

    // Extract content type
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if !is_allowed_content_type(&content_type) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err_unsupported_type})),
        );
    }

    // Extract filename from header
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    // Validate size (use config override or fall back to compiled default)
    if body.len() > upload_limit {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": err_too_large_upload})),
        );
    }

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": err_empty_body})),
        );
    }

    // Generate file ID and save
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    if let Err(e) = tokio::fs::create_dir_all(&upload_dir).await {
        tracing::warn!("Failed to create upload dir: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err_upload_dir_failed})),
        );
    }

    // Persist under `<uuid>.<ext>` so the type survives at rest (#6530), while
    // the registry key / client-facing `file_id` stays a bare UUID for the
    // traversal + owner guards. Readers reconstruct the same name via
    // `on_disk_name` from the registry's stored content_type/filename.
    let on_disk = librefang_types::media::on_disk_name(&file_id, &content_type, &filename);
    let file_path = upload_dir.join(&on_disk);
    if let Err(e) = tokio::fs::write(&file_path, &body).await {
        tracing::warn!("Failed to write upload: {e}");
        if let Err(cleanup_error) = tokio::fs::remove_file(&file_path).await {
            if cleanup_error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%cleanup_error, file_id, "failed to clean up partial upload");
            }
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err_upload_save_failed})),
        );
    }

    let uploaded_by = api_user.as_ref().map(|u| u.0.user_id);
    let meta = UploadMeta {
        filename: filename.clone(),
        content_type: content_type.clone(),
        uploaded_by,
    };
    if let Err(error) = persist_upload_meta(&upload_dir, &file_id, &meta).await {
        tracing::warn!(%error, file_id, "failed to save upload metadata");
        if let Err(cleanup_error) = tokio::fs::remove_file(&file_path).await {
            tracing::warn!(%cleanup_error, file_id, "failed to clean up upload after metadata error");
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err_upload_save_failed})),
        );
    }
    UPLOAD_REGISTRY.insert(file_id.clone(), meta);
    let size = body.len();

    // Auto-transcribe audio uploads using the media engine
    let transcription = if content_type.starts_with("audio/") {
        let attachment = librefang_types::media::MediaAttachment {
            media_type: librefang_types::media::MediaType::Audio,
            mime_type: content_type.clone(),
            source: librefang_types::media::MediaSource::FilePath {
                path: file_path.to_string_lossy().to_string(),
            },
            size_bytes: size as u64,
        };
        match state
            .kernel
            .media()
            .transcribe_audio(&attachment, None, None)
            .await
        {
            Ok(result) => {
                tracing::info!(chars = result.description.len(), provider = %result.provider, "Audio transcribed");
                Some(result.description)
            }
            Err(e) => {
                tracing::warn!("Audio transcription failed: {e}");
                None
            }
        }
    } else {
        None
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!(UploadResponse {
            file_id,
            filename,
            content_type,
            size,
            transcription,
        })),
    )
}

/// Resolve the on-disk path of a persisted upload, tolerating both the
/// `<uuid>.<ext>` scheme (#6530) and the historical bare-`<uuid>` scheme.
///
/// Tries, in order: the deterministic `<uuid>.<ext>` name (from the known
/// content type / filename), the bare `<uuid>` (files written before #6530 and
/// registry misses), then a `<uuid>.*` directory probe (generated images whose
/// content type the reader may not know). Returns the first existing path.
/// `file_id` is a validated UUID, so the probe's prefix match cannot escape
/// `dir`.
pub(crate) async fn resolve_existing_upload_path_async(
    dir: &std::path::Path,
    file_id: &str,
    content_type: &str,
    filename: &str,
) -> Option<std::path::PathBuf> {
    let named = dir.join(librefang_types::media::on_disk_name(
        file_id,
        content_type,
        filename,
    ));
    if tokio::fs::try_exists(&named).await.ok()? {
        return Some(named);
    }
    let bare = dir.join(file_id);
    if tokio::fs::try_exists(&bare).await.ok()? {
        return Some(bare);
    }
    let prefix = format!("{file_id}.");
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            return Some(entry.path());
        }
    }
    None
}

/// GET /api/uploads/{file_id} — Serve an uploaded file.
#[utoipa::path(
    get,
    path = "/api/uploads/{file_id}",
    tag = "agents",
    params(("file_id" = String, Path, description = "Upload file ID (UUID)")),
    responses(
        (status = 200, description = "Serve an uploaded file by ID", body = crate::types::JsonObject)
    )
)]
pub async fn serve_upload(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<String>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    // Validate file_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&file_id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"Invalid file ID\"}".to_vec(),
        );
    }

    let upload_dir = state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();

    // The registry caches content_type/filename/owner metadata.
    // Reload the persisted sidecar after a daemon restart; a true miss is legacy content with unknown ownership and is restricted to Admin/Owner in auth mode.
    let meta = match UPLOAD_REGISTRY.get(&file_id) {
        Some(meta) => Some(meta.clone()),
        None => {
            let loaded = load_upload_meta(&upload_dir, &file_id).await;
            if let Some(meta) = loaded.as_ref() {
                UPLOAD_REGISTRY.insert(file_id.clone(), meta.clone());
            }
            loaded
        }
    };
    if !upload_access_allowed(meta.as_ref(), api_user.as_ref().map(|user| &user.0)) {
        tracing::warn!(
            file_id = %file_id,
            caller = ?api_user.as_ref().map(|user| user.0.name.clone()),
            "upload access denied: owner metadata missing or caller is not the uploader"
        );
        return (
            StatusCode::FORBIDDEN,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"You are not authorized to access this upload\"}".to_vec(),
        );
    }
    let (content_type, filename) = meta.as_ref().map_or_else(
        || ("image/png".to_string(), String::new()),
        |meta| (meta.content_type.clone(), meta.filename.clone()),
    );

    let Some(file_path) =
        resolve_existing_upload_path_async(&upload_dir, &file_id, &content_type, &filename).await
    else {
        return (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"File not found\"}".to_vec(),
        );
    };

    match tokio::fs::read(&file_path).await {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            data,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"File not found on disk\"}".to_vec(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_upload_limit, load_upload_meta, persist_upload_meta_sync,
        resolve_existing_upload_path_async, upload_access_allowed, UploadMeta,
    };
    use crate::middleware::{AuthenticatedApiUser, UserRole};
    use librefang_types::agent::UserId;

    #[tokio::test]
    async fn async_resolver_finds_named_bare_and_generated_image_schemes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();

        let named_id = "51111111-1111-1111-1111-111111111111";
        tokio::fs::write(p.join(format!("{named_id}.png")), b"png")
            .await
            .unwrap();
        assert_eq!(
            resolve_existing_upload_path_async(p, named_id, "image/png", "shot.png").await,
            Some(p.join(format!("{named_id}.png")))
        );

        let bare_id = "52222222-2222-2222-2222-222222222222";
        tokio::fs::write(p.join(bare_id), b"legacy").await.unwrap();
        assert_eq!(
            resolve_existing_upload_path_async(p, bare_id, "image/png", "old.png").await,
            Some(p.join(bare_id))
        );

        let generated_id = "53333333-3333-3333-3333-333333333333";
        tokio::fs::write(p.join(format!("{generated_id}.jpg")), b"jpg")
            .await
            .unwrap();
        assert_eq!(
            resolve_existing_upload_path_async(p, generated_id, "application/octet-stream", "",)
                .await,
            Some(p.join(format!("{generated_id}.jpg")))
        );
    }

    #[tokio::test]
    async fn upload_metadata_survives_registry_loss() {
        let dir = tempfile::tempdir().unwrap();
        let file_id = "61111111-1111-1111-1111-111111111111";
        let owner = UserId::from_name("alice");
        let expected = UploadMeta {
            filename: "private.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            uploaded_by: Some(owner),
        };

        persist_upload_meta_sync(dir.path(), file_id, &expected).unwrap();
        let loaded = load_upload_meta(dir.path(), file_id)
            .await
            .expect("persisted metadata must reload");
        assert_eq!(loaded.filename, expected.filename);
        assert_eq!(loaded.content_type, expected.content_type);
        assert_eq!(loaded.uploaded_by, Some(owner));
    }

    #[test]
    fn unknown_upload_metadata_fails_closed_for_authenticated_non_admins() {
        let owner_id = UserId::from_name("alice");
        let owner = AuthenticatedApiUser {
            name: "alice".to_string(),
            role: UserRole::User,
            user_id: owner_id,
        };
        let stranger = AuthenticatedApiUser {
            name: "eve".to_string(),
            role: UserRole::Viewer,
            user_id: UserId::from_name("eve"),
        };
        let admin = AuthenticatedApiUser {
            name: "admin".to_string(),
            role: UserRole::Admin,
            user_id: UserId::from_name("admin"),
        };
        let owned = UploadMeta {
            filename: "private.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            uploaded_by: Some(owner_id),
        };
        let generated = UploadMeta {
            filename: "generated.png".to_string(),
            content_type: "image/png".to_string(),
            uploaded_by: None,
        };

        assert!(upload_access_allowed(Some(&owned), Some(&owner)));
        assert!(!upload_access_allowed(Some(&owned), Some(&stranger)));
        assert!(upload_access_allowed(Some(&owned), Some(&admin)));
        assert!(upload_access_allowed(Some(&generated), Some(&stranger)));
        assert!(!upload_access_allowed(None, Some(&stranger)));
        assert!(upload_access_allowed(None, Some(&admin)));
        assert!(upload_access_allowed(None, None));
    }

    #[test]
    fn upload_limit_display_uses_the_runtime_byte_value() {
        assert_eq!(format_upload_limit(3 * 1024 * 1024), "3 MiB");
        assert_eq!(format_upload_limit(7 * 1024), "7 KiB");
        assert_eq!(format_upload_limit(1537), "1537 bytes");
    }
}
