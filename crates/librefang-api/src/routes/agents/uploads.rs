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

    let file_path = upload_dir.join(&file_id);
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

    let file_path = state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir()
        .join(&file_id);

    // Look up metadata from registry; fall back to disk probe for generated images
    // (image_generate saves files without registering in UPLOAD_REGISTRY).
    let (content_type, owner) = match UPLOAD_REGISTRY.get(&file_id) {
        Some(m) => (m.content_type.clone(), m.uploaded_by),
        None => {
            // Infer content type from file magic bytes
            if !file_path.exists() {
                return (
                    StatusCode::NOT_FOUND,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/json".to_string(),
                    )],
                    b"{\"error\":\"File not found\"}".to_vec(),
                );
            }
            ("image/png".to_string(), None)
        }
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
