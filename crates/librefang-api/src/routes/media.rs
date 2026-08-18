//! Media generation API routes — image, TTS, video, and music generation.

use super::AppState;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::media::{MediaDriverCache, MediaError};
use librefang_types::media::{
    MediaCapability, MediaImageRequest, MediaMusicRequest, MediaTtsRequest, MediaVideoRequest,
};
use std::sync::Arc;

/// Build all routes for the Media generation domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/media/image", axum::routing::post(generate_image))
        .route("/media/speech", axum::routing::post(synthesize_speech))
        .route("/media/video", axum::routing::post(submit_video))
        .route(
            "/media/video/{task_id}",
            axum::routing::get(poll_video_task),
        )
        .route("/media/music", axum::routing::post(generate_music))
        .route("/media/providers", axum::routing::get(list_media_providers))
        .route("/media/transcribe", axum::routing::post(transcribe_audio))
}

// ── Known media providers (mirrors MEDIA_PROVIDER_ORDER in runtime) ─────

/// Built-in media provider names, in preference order, for
/// `GET /media/providers` to report configuration status against.
///
/// This is a **display list, not an allowlist**: `create_media_driver` also
/// serves user-defined providers that have `provider_urls.<name>` configured,
/// via the generic OpenAI-compatible driver. Gating a route on membership here
/// would reject those.
/// Keep in sync with `librefang_kernel::media::MEDIA_PROVIDER_ORDER`.
const KNOWN_MEDIA_PROVIDERS: &[&str] = &["openai", "gemini", "elevenlabs", "minimax", "google_tts"];

// ── Helpers ─────────────────────────────────────────────────────────────

/// Convert a `MediaError` into an [`ApiErrorResponse`].
fn media_error_response(err: MediaError) -> ApiErrorResponse {
    let (status, code) = match &err {
        MediaError::NotSupported(_) => (StatusCode::BAD_REQUEST, "not_supported"),
        MediaError::MissingKey(_) => (StatusCode::UNPROCESSABLE_ENTITY, "missing_key"),
        MediaError::Http(_) => (StatusCode::BAD_GATEWAY, "upstream_http_error"),
        MediaError::Api { status, .. } => {
            let sc = StatusCode::from_u16(*status)
                .ok()
                .filter(StatusCode::is_client_error)
                .unwrap_or(StatusCode::BAD_GATEWAY);
            (sc, "upstream_api_error")
        }
        MediaError::RateLimit(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        MediaError::ContentFiltered(_) => (StatusCode::BAD_REQUEST, "content_filtered"),
        MediaError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
        MediaError::TaskNotFound(_) => (StatusCode::NOT_FOUND, "task_not_found"),
        MediaError::Other(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    };
    let error = if status == StatusCode::INTERNAL_SERVER_ERROR {
        tracing::error!(error = %err, "media request failed");
        "Internal server error".to_string()
    } else {
        err.to_string()
    };
    ApiErrorResponse {
        error,
        code: Some(code.to_string()),
        r#type: None,
        details: None,
        request_id: None,
        status,
    }
}

fn image_content_type(data: &[u8]) -> Option<(&'static str, &'static str)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some(("image/jpeg", "jpg"))
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

/// Resolve a media driver from the request-level provider hint or auto-detect.
fn resolve_driver(
    cache: &MediaDriverCache,
    provider: &Option<String>,
    capability: MediaCapability,
) -> Result<Arc<dyn librefang_kernel::media::MediaDriver>, MediaError> {
    if let Some(ref name) = provider {
        cache.get_or_create(name, None)
    } else {
        cache.detect_for_capability(capability)
    }
}

/// Save binary data to the uploads directory and return an upload URL.
///
/// The file is registered in the shared `UPLOAD_REGISTRY` so the existing
/// `serve_upload` handler returns the correct `Content-Type`.
async fn save_upload(
    state: &AppState,
    data: &[u8],
    filename: &str,
    content_type: &str,
) -> Result<String, String> {
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| format!("Failed to create upload directory: {e}"))?;
    // Persist as `<uuid>.<ext>` (#6530); the registry entry below stores the
    // content type so serve_upload reconstructs the same name.
    let on_disk = librefang_types::media::on_disk_name(&file_id, content_type, filename);
    let file_path = upload_dir.join(&on_disk);
    tokio::fs::write(&file_path, data)
        .await
        .map_err(|e| format!("Failed to write upload file: {e}"))?;

    // Persist and register metadata so serve_upload retains the correct type and explicit shared-access policy across daemon restarts.
    let meta = super::agents::UploadMeta {
        filename: filename.to_string(),
        content_type: content_type.to_string(),
        // Daemon-generated media has no human owner — leave it shared.
        uploaded_by: None,
    };
    if let Err(error) = super::agents::persist_upload_meta(&upload_dir, &file_id, &meta).await {
        if let Err(cleanup_error) = tokio::fs::remove_file(&file_path).await {
            tracing::warn!(%cleanup_error, file_id, "failed to clean up generated upload");
        }
        return Err(error);
    }
    super::agents::UPLOAD_REGISTRY.insert(file_id.clone(), meta);

    Ok(format!("/api/uploads/{file_id}"))
}

// ── POST /media/image ───────────────────────────────────────────────────

/// Generate one or more images from a text prompt.
pub async fn generate_image(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MediaImageRequest>,
) -> impl IntoResponse {
    // Validate request
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
    }

    // Resolve driver
    let driver = match resolve_driver(
        &state.media_drivers,
        &body.provider,
        MediaCapability::ImageGeneration,
    ) {
        Ok(d) => d,
        Err(e) => return media_error_response(e).into_response(),
    };

    // Generate
    let result = match driver.generate_image(&body).await {
        Ok(r) => r,
        Err(e) => return media_error_response(e).into_response(),
    };

    // Save images to upload dir, replacing base64 data with URLs
    use base64::Engine;
    let mut image_urls: Vec<serde_json::Value> = Vec::new();
    for (i, img) in result.images.iter().enumerate() {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(&img.data_base64) {
            Ok(b) => b,
            Err(_) => {
                // If decoding fails, return the raw result as-is
                image_urls.push(serde_json::json!({
                    "data_base64": img.data_base64,
                    "url": img.url,
                }));
                continue;
            }
        };

        let Some((content_type, extension)) = image_content_type(&bytes) else {
            tracing::warn!(index = i, "generated image has an unknown byte signature");
            image_urls.push(serde_json::json!({
                "data_base64": img.data_base64,
                "url": img.url,
            }));
            continue;
        };
        let filename = format!("image_{i}.{extension}");
        match save_upload(&state, &bytes, &filename, content_type).await {
            Ok(url) => {
                image_urls.push(serde_json::json!({
                    "url": url,
                }));
            }
            Err(e) => {
                tracing::warn!("Failed to save generated image: {e}");
                image_urls.push(serde_json::json!({
                    "data_base64": img.data_base64,
                    "url": img.url,
                }));
            }
        }
    }

    Json(serde_json::json!({
        "images": image_urls,
        "model": result.model,
        "provider": result.provider,
        "revised_prompt": result.revised_prompt,
    }))
    .into_response()
}

// ── POST /media/speech ──────────────────────────────────────────────────

/// Synthesize speech from text (TTS).
pub async fn synthesize_speech(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MediaTtsRequest>,
) -> impl IntoResponse {
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
    }

    let driver = match resolve_driver(
        &state.media_drivers,
        &body.provider,
        MediaCapability::TextToSpeech,
    ) {
        Ok(d) => d,
        Err(e) => return media_error_response(e).into_response(),
    };

    let result = match driver.synthesize_speech(&body).await {
        Ok(r) => r,
        Err(e) => return media_error_response(e).into_response(),
    };

    // Save audio to upload dir
    let content_type = match result.format.as_str() {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "aac" => "audio/aac",
        _ => "audio/mpeg",
    };
    let filename = format!("speech.{}", result.format);

    match save_upload(&state, &result.audio_data, &filename, content_type).await {
        Ok(url) => Json(serde_json::json!({
            "url": url,
            "format": result.format,
            "provider": result.provider,
            "model": result.model,
            "duration_ms": result.duration_ms,
            "sample_rate": result.sample_rate,
        }))
        .into_response(),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_response(),
    }
}

// ── POST /media/video ───────────────────────────────────────────────────

/// Submit a video generation task (async — returns a task ID for polling).
pub async fn submit_video(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MediaVideoRequest>,
) -> impl IntoResponse {
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
    }

    let driver = match resolve_driver(
        &state.media_drivers,
        &body.provider,
        MediaCapability::VideoGeneration,
    ) {
        Ok(d) => d,
        Err(e) => return media_error_response(e).into_response(),
    };

    let result = match driver.submit_video(&body).await {
        Ok(r) => r,
        Err(e) => return media_error_response(e).into_response(),
    };

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "task_id": result.task_id,
            "provider": result.provider,
        })),
    )
        .into_response()
}

// ── GET /media/video/{task_id} ──────────────────────────────────────────

/// Poll video generation task status and retrieve result when complete.
///
/// Query parameter `provider` is required to route the poll to the correct
/// driver (the task ID is provider-specific).
pub async fn poll_video_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let provider = match params.get("provider") {
        Some(p) => p.clone(),
        None => {
            return ApiErrorResponse::bad_request("Missing required query parameter: provider")
                .into_response();
        }
    };

    let driver = match state.media_drivers.get_or_create(&provider, None) {
        Ok(d) => d,
        Err(e) => return media_error_response(e).into_response(),
    };

    // Poll status first
    let status = match driver.poll_video(&task_id).await {
        Ok(s) => s,
        Err(e) => return media_error_response(e).into_response(),
    };

    // If completed, fetch the full result
    if status == librefang_types::media::MediaTaskStatus::Completed {
        match driver.get_video_result(&task_id).await {
            Ok(video) => {
                return Json(serde_json::json!({
                    "status": "completed",
                    "result": {
                        "file_url": video.file_url,
                        "width": video.width,
                        "height": video.height,
                        "duration_secs": video.duration_secs,
                        "provider": video.provider,
                        "model": video.model,
                    }
                }))
                .into_response();
            }
            Err(e) => return media_error_response(e).into_response(),
        }
    }

    // Keep the HTTP contract flat and stable. `MediaTaskStatus` itself is a
    // tagged enum (`{"state":"failed","error":"..."}`), but dashboard
    // consumers expect a string status plus an optional sibling error.
    Json(video_task_status_json(status, &task_id)).into_response()
}

fn video_task_status_json(
    status: librefang_types::media::MediaTaskStatus,
    task_id: &str,
) -> serde_json::Value {
    match status {
        librefang_types::media::MediaTaskStatus::Failed { error } => {
            serde_json::json!({
                "status": "failed",
                "task_id": task_id,
                "error": error,
            })
        }
        status => serde_json::json!({
            "status": status.to_string(),
            "task_id": task_id,
        }),
    }
}

// ── POST /media/music ───────────────────────────────────────────────────

/// Generate music from a prompt and/or lyrics.
pub async fn generate_music(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MediaMusicRequest>,
) -> impl IntoResponse {
    if let Err(e) = body.validate() {
        return ApiErrorResponse::bad_request(e).into_response();
    }

    let driver = match resolve_driver(
        &state.media_drivers,
        &body.provider,
        MediaCapability::MusicGeneration,
    ) {
        Ok(d) => d,
        Err(e) => return media_error_response(e).into_response(),
    };

    let result = match driver.generate_music(&body).await {
        Ok(r) => r,
        Err(e) => return media_error_response(e).into_response(),
    };

    // Save audio to upload dir
    let content_type = match result.format.as_str() {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        _ => "audio/mpeg",
    };
    let filename = format!("music.{}", result.format);

    match save_upload(&state, &result.audio_data, &filename, content_type).await {
        Ok(url) => Json(serde_json::json!({
            "url": url,
            "format": result.format,
            "duration_ms": result.duration_ms,
            "provider": result.provider,
            "model": result.model,
            "sample_rate": result.sample_rate,
        }))
        .into_response(),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_response(),
    }
}

// ── POST /media/transcribe ──────────────────────────────────────────────

/// Transcribe audio to text (STT).
///
/// Accepts raw audio bytes with `Content-Type` set to the audio MIME type
/// (e.g. `audio/webm`, `audio/wav`). Returns the transcribed text.
pub async fn transcribe_audio(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Strip parameters (e.g. "audio/webm;codecs=opus" -> "audio/webm")
    // so MediaEngine's mime_to_ext can match the base type.
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/webm")
        .split(';')
        .next()
        .unwrap_or("audio/webm")
        .trim()
        .to_string();

    if !content_type.starts_with("audio/") {
        return ApiErrorResponse::bad_request("Content-Type must be an audio type").into_response();
    }

    if body.is_empty() {
        return ApiErrorResponse::bad_request("Empty audio body").into_response();
    }

    // 10 MB limit
    if body.len() > 10 * 1024 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Audio too large (max 10MB)"})),
        )
            .into_response();
    }

    // Save to temp file so MediaEngine can read it
    let upload_dir = state
        .kernel
        .config_ref()
        .channels
        .effective_file_download_dir();
    if let Err(e) = tokio::fs::create_dir_all(&upload_dir).await {
        return ApiErrorResponse::internal_scrub(e).into_response();
    }
    let file_id = uuid::Uuid::new_v4().to_string();
    // Transient file read straight back by the media engine via `file_path`;
    // still named `<uuid>.<ext>` so all producers share one scheme (#6530).
    let file_path = upload_dir.join(librefang_types::media::on_disk_name(
        &file_id,
        &content_type,
        "",
    ));
    if let Err(e) = tokio::fs::write(&file_path, &body).await {
        return ApiErrorResponse::internal_scrub(e).into_response();
    }
    let mut temp_file = TempUploadGuard(Some(file_path.clone()));

    let attachment = librefang_types::media::MediaAttachment {
        media_type: librefang_types::media::MediaType::Audio,
        mime_type: content_type,
        source: librefang_types::media::MediaSource::FilePath {
            path: file_path.to_string_lossy().to_string(),
        },
        size_bytes: body.len() as u64,
    };

    let response = match state
        .kernel
        .media()
        .transcribe_audio(&attachment, None, None)
        .await
    {
        Ok(result) => Json(serde_json::json!({
            "text": result.description,
            "provider": result.provider,
            "model": result.model,
        }))
        .into_response(),
        Err(e) => ApiErrorResponse::internal_scrub(e).into_response(),
    };
    if tokio::fs::remove_file(&file_path).await.is_ok() {
        temp_file.0 = None;
    }
    response
}

struct TempUploadGuard(Option<std::path::PathBuf>);

impl Drop for TempUploadGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ── GET /media/providers ────────────────────────────────────────────────

/// List available media providers with their capabilities and config status.
pub async fn list_media_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut providers = Vec::new();

    for &name in KNOWN_MEDIA_PROVIDERS {
        match state.media_drivers.get_or_create(name, None) {
            Ok(driver) => {
                providers.push(serde_json::json!({
                    "name": driver.provider_name(),
                    "configured": driver.is_configured(),
                    "capabilities": driver.capabilities(),
                }));
            }
            Err(_) => {
                // Provider could not be instantiated (should not happen for known providers)
                providers.push(serde_json::json!({
                    "name": name,
                    "configured": false,
                    "capabilities": [],
                    "error": "driver instantiation failed",
                }));
            }
        }
    }

    Json(serde_json::json!({
        "providers": providers,
    }))
}

#[cfg(test)]
mod audit_tests {
    use super::*;

    #[test]
    fn generated_image_signatures_select_the_real_mime_type() {
        assert_eq!(
            image_content_type(b"\x89PNG\r\n\x1a\nrest"),
            Some(("image/png", "png"))
        );
        assert_eq!(
            image_content_type(b"\xff\xd8\xffrest"),
            Some(("image/jpeg", "jpg"))
        );
        assert_eq!(
            image_content_type(b"GIF89arest"),
            Some(("image/gif", "gif"))
        );
        assert_eq!(
            image_content_type(b"RIFF1234WEBPrest"),
            Some(("image/webp", "webp"))
        );
        assert_eq!(image_content_type(b"not an image"), None);
    }

    #[test]
    fn upstream_statuses_only_preserve_client_errors() {
        let client = media_error_response(MediaError::Api {
            status: 422,
            message: "invalid".to_string(),
        });
        let server = media_error_response(MediaError::Api {
            status: 503,
            message: "unavailable".to_string(),
        });
        assert_eq!(client.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(server.status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn temp_upload_guard_removes_an_abandoned_file() {
        let path =
            std::env::temp_dir().join(format!("librefang-media-guard-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"audio").unwrap();
        {
            let _guard = TempUploadGuard(Some(path.clone()));
        }
        assert!(!path.exists());
    }

    #[test]
    fn unknown_video_poll_provider_keeps_the_invalid_request_code() {
        let err = media_error_response(MediaError::InvalidRequest(
            "Unknown media provider 'definitely_not_a_provider' and no base_url configured."
                .to_string(),
        ));
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code.as_deref(), Some("invalid_request"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::media::MediaTaskStatus;

    #[test]
    fn video_task_status_json_flattens_active_status() {
        assert_eq!(
            video_task_status_json(MediaTaskStatus::Processing, "task-1"),
            serde_json::json!({"status": "processing", "task_id": "task-1"}),
        );
    }

    #[test]
    fn video_task_status_json_flattens_failure_error() {
        assert_eq!(
            video_task_status_json(
                MediaTaskStatus::Failed {
                    error: "provider failed".to_string(),
                },
                "task-2",
            ),
            serde_json::json!({
                "status": "failed",
                "task_id": "task-2",
                "error": "provider failed",
            }),
        );
    }

    #[test]
    fn internal_media_errors_are_scrubbed_but_client_errors_are_preserved() {
        let internal = media_error_response(MediaError::Other(
            "failed to read /srv/librefang/media.key".to_string(),
        ));
        assert_eq!(internal.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(internal.code.as_deref(), Some("internal_error"));
        assert_eq!(internal.error, "Internal server error");
        assert!(!internal.error.contains("/srv/librefang"));

        let invalid = media_error_response(MediaError::InvalidRequest(
            "prompt must not be empty".to_string(),
        ));
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid.error, "Invalid request: prompt must not be empty");
    }
}
