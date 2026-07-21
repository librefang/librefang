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

/// Metadata stored alongside uploaded files.
pub(crate) struct UploadMeta {
    #[allow(dead_code)]
    pub(crate) filename: String,
    pub(crate) content_type: String,
    /// User who uploaded the file (#3361). `None` means "anonymous /
    /// pre-auth daemon" — readable by any authenticated caller for
    /// backwards compatibility with content saved before owner-binding
    /// was introduced. New uploads from authenticated users always set
    /// this so `serve_upload` can reject cross-user UUID guessing.
    pub(crate) uploaded_by: Option<librefang_types::agent::UserId>,
}

/// In-memory upload metadata registry.
pub(crate) static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> =
    LazyLock::new(DashMap::new);

/// Maximum upload size: 10 MB.
#[allow(dead_code)]
const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;

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
            t.t_args("api-error-file-too-large", &[("max", "10MB")]),
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
    let upload_limit = state.kernel.config_ref().max_upload_size_bytes;
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err_upload_save_failed})),
        );
    }

    let size = body.len();
    let uploaded_by = api_user.as_ref().map(|u| u.0.user_id);
    UPLOAD_REGISTRY.insert(
        file_id.clone(),
        UploadMeta {
            filename: filename.clone(),
            content_type: content_type.clone(),
            uploaded_by,
        },
    );

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
        match state.kernel.media().transcribe_audio(&attachment).await {
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
pub(crate) fn resolve_existing_upload_path(
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
    if named.exists() {
        return Some(named);
    }
    let bare = dir.join(file_id);
    if bare.exists() {
        return Some(bare);
    }
    let prefix = format!("{file_id}.");
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
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

    // The registry carries the content_type/filename needed to reconstruct the
    // `<uuid>.<ext>` on-disk name (#6530) and the owner for the access check. A
    // miss means a generated image / pre-registry file — the resolver's
    // `<uuid>.*` probe still finds it, and the content type defaults to PNG (the
    // only un-registered producer today is image_generate).
    let (content_type, filename, owner) = match UPLOAD_REGISTRY.get(&file_id) {
        Some(m) => (m.content_type.clone(), m.filename.clone(), m.uploaded_by),
        None => ("image/png".to_string(), String::new(), None),
    };

    let Some(file_path) =
        resolve_existing_upload_path(&upload_dir, &file_id, &content_type, &filename)
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

    // SECURITY (#3361): Bind uploads to their uploader. A bare UUID is not
    // access control — UUIDs leak through audit logs, dashboard responses,
    // tracing output, and message history. Owner-bound files are readable
    // only by the uploader or by Admin/Owner callers; un-owned entries (pre-
    // #3361 uploads, generator output) stay readable for compatibility.
    if let Some(owner_id) = owner {
        use crate::middleware::UserRole;
        let allowed = match api_user.as_ref().map(|u| &u.0) {
            Some(u) => u.user_id == owner_id || u.role >= UserRole::Admin,
            None => false,
        };
        if !allowed {
            tracing::warn!(
                file_id = %file_id,
                caller = ?api_user.as_ref().map(|u| u.0.name.clone()),
                "upload access denied: caller is not the uploader"
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
    }

    match std::fs::read(&file_path) {
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
    use super::resolve_existing_upload_path;

    #[test]
    fn resolver_finds_named_bare_and_generated_image_schemes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();

        // 1. Registry hit: producer wrote `<uuid>.png`, reader knows the type.
        let named_id = "11111111-1111-1111-1111-111111111111";
        std::fs::write(p.join(format!("{named_id}.png")), b"png").unwrap();
        assert_eq!(
            resolve_existing_upload_path(p, named_id, "image/png", "shot.png"),
            Some(p.join(format!("{named_id}.png")))
        );

        // 2. Legacy file written bare `<uuid>` before #6530: still served, even
        //    though the reader now computes a `.png` name that does not exist.
        let bare_id = "22222222-2222-2222-2222-222222222222";
        std::fs::write(p.join(bare_id), b"legacy").unwrap();
        assert_eq!(
            resolve_existing_upload_path(p, bare_id, "image/png", "old.png"),
            Some(p.join(bare_id))
        );

        // 3. Generated image not in the registry: the reader defaults the type,
        //    but the `<uuid>.*` probe finds the real extension.
        let gen_id = "33333333-3333-3333-3333-333333333333";
        std::fs::write(p.join(format!("{gen_id}.jpg")), b"jpg").unwrap();
        assert_eq!(
            resolve_existing_upload_path(p, gen_id, "application/octet-stream", ""),
            Some(p.join(format!("{gen_id}.jpg")))
        );

        // 4. Nothing on disk → None.
        assert_eq!(
            resolve_existing_upload_path(p, "44444444-4444-4444-4444-444444444444", "", ""),
            None
        );
    }
}
