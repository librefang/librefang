//! Media generation API routes — image, TTS, video, and music generation.

use super::shared::require_admin;
use super::AppState;
use crate::middleware::AccountId;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use librefang_runtime::media::{MediaDriverCache, MediaError};
use librefang_types::media::{
    MediaCapability, MediaImageRequest, MediaMusicRequest, MediaTtsRequest, MediaVideoRequest,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::LazyLock;

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
}

// ── Known media providers (mirrors MEDIA_PROVIDER_ORDER in runtime) ─────

/// Known media provider names, in preference order.
/// Keep in sync with `librefang_runtime::media::MEDIA_PROVIDER_ORDER`.
const KNOWN_MEDIA_PROVIDERS: &[&str] = &["openai", "gemini", "elevenlabs", "minimax", "google_tts"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MediaTaskMeta {
    provider: String,
    account_id: Option<String>,
}

static MEDIA_TASK_REGISTRY: LazyLock<DashMap<String, MediaTaskMeta>> = LazyLock::new(DashMap::new);

fn media_task_meta_path_for_data_dir(
    data_dir: &std::path::Path,
    task_id: &str,
) -> std::path::PathBuf {
    use base64::Engine;

    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(task_id.as_bytes());
    data_dir.join("media_tasks").join(format!("{encoded}.json"))
}

fn persist_media_task_meta_for_data_dir(
    data_dir: &std::path::Path,
    task_id: &str,
    meta: &MediaTaskMeta,
) -> Result<(), String> {
    let path = media_task_meta_path_for_data_dir(data_dir, task_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create media task directory: {e}"))?;
    }
    let payload = serde_json::to_vec(meta)
        .map_err(|e| format!("Failed to serialize media task metadata: {e}"))?;
    std::fs::write(path, payload).map_err(|e| format!("Failed to persist media task metadata: {e}"))
}

fn load_media_task_meta_for_data_dir(
    data_dir: &std::path::Path,
    task_id: &str,
) -> Option<MediaTaskMeta> {
    let path = media_task_meta_path_for_data_dir(data_dir, task_id);
    let payload = std::fs::read(path).ok()?;
    serde_json::from_slice(&payload).ok()
}

fn persist_media_task_meta(
    state: &AppState,
    task_id: &str,
    meta: &MediaTaskMeta,
) -> Result<(), String> {
    persist_media_task_meta_for_data_dir(&state.kernel.config_ref().data_dir, task_id, meta)
}

fn load_media_task_meta(state: &AppState, task_id: &str) -> Option<MediaTaskMeta> {
    if let Some(meta) = MEDIA_TASK_REGISTRY.get(task_id) {
        return Some(meta.clone());
    }

    let meta = load_media_task_meta_for_data_dir(&state.kernel.config_ref().data_dir, task_id)?;
    MEDIA_TASK_REGISTRY.insert(task_id.to_string(), meta.clone());
    Some(meta)
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Convert a `MediaError` into an [`ApiErrorResponse`].
fn media_error_response(err: MediaError) -> ApiErrorResponse {
    let (status, code) = match &err {
        MediaError::NotSupported(_) => (StatusCode::BAD_REQUEST, "not_supported"),
        MediaError::MissingKey(_) => (StatusCode::UNPROCESSABLE_ENTITY, "missing_key"),
        MediaError::Http(_) => (StatusCode::BAD_GATEWAY, "upstream_http_error"),
        MediaError::Api { status, .. } => {
            let sc = StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY);
            (sc, "upstream_api_error")
        }
        MediaError::RateLimit(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        MediaError::ContentFiltered(_) => (StatusCode::BAD_REQUEST, "content_filtered"),
        MediaError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
        MediaError::TaskNotFound(_) => (StatusCode::NOT_FOUND, "task_not_found"),
        MediaError::Other(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    };
    ApiErrorResponse {
        error: err.to_string(),
        code: Some(code.to_string()),
        r#type: None,
        details: None,
        status,
    }
}

/// Resolve a media driver from the request-level provider hint or auto-detect.
fn resolve_driver(
    cache: &MediaDriverCache,
    provider: &Option<String>,
    capability: MediaCapability,
) -> Result<Arc<dyn librefang_runtime::media::MediaDriver>, MediaError> {
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
fn save_upload(
    data: &[u8],
    filename: &str,
    content_type: &str,
    account_id: &Option<String>,
) -> Result<String, String> {
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("librefang_uploads");
    std::fs::create_dir_all(&upload_dir)
        .map_err(|e| format!("Failed to create upload directory: {e}"))?;
    std::fs::write(upload_dir.join(&file_id), data)
        .map_err(|e| format!("Failed to write upload file: {e}"))?;

    // Register metadata so serve_upload returns the correct content type.
    super::agents::UPLOAD_REGISTRY.insert(
        file_id.clone(),
        super::agents::UploadMeta {
            filename: filename.to_string(),
            content_type: content_type.to_string(),
            account_id: account_id.clone(),
        },
    );

    Ok(format!("/api/uploads/{file_id}"))
}

// ── POST /media/image ───────────────────────────────────────────────────

/// Generate one or more images from a text prompt.
pub async fn generate_image(
    account: AccountId,
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

        let filename = format!("image_{i}.png");
        match save_upload(&bytes, &filename, "image/png", &account.0) {
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
    account: AccountId,
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

    match save_upload(&result.audio_data, &filename, content_type, &account.0) {
        Ok(url) => Json(serde_json::json!({
            "url": url,
            "format": result.format,
            "provider": result.provider,
            "model": result.model,
            "duration_ms": result.duration_ms,
            "sample_rate": result.sample_rate,
        }))
        .into_response(),
        Err(e) => ApiErrorResponse::internal(format!("Failed to save audio: {e}")).into_response(),
    }
}

// ── POST /media/video ───────────────────────────────────────────────────

/// Submit a video generation task (async — returns a task ID for polling).
pub async fn submit_video(
    account: AccountId,
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

    let meta = MediaTaskMeta {
        provider: result.provider.clone(),
        account_id: account.0.clone(),
    };
    MEDIA_TASK_REGISTRY.insert(result.task_id.clone(), meta.clone());
    if let Err(e) = persist_media_task_meta(&state, &result.task_id, &meta) {
        return ApiErrorResponse::internal(e).into_response();
    }

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
    account: AccountId,
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

    match load_media_task_meta(&state, &task_id) {
        Some(meta) => {
            if meta.provider != provider {
                return ApiErrorResponse::not_found("Video task not found").into_response();
            }
            if account.0.as_deref() != meta.account_id.as_deref() {
                return ApiErrorResponse::not_found("Video task not found").into_response();
            }
        }
        None => {
            if account.0.is_some() {
                return ApiErrorResponse::not_found("Video task not found").into_response();
            }
        }
    }

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

    // Return current status for non-completed tasks
    Json(serde_json::json!({
        "status": status,
        "task_id": task_id,
    }))
    .into_response()
}

// ── POST /media/music ───────────────────────────────────────────────────

/// Generate music from a prompt and/or lyrics.
pub async fn generate_music(
    account: AccountId,
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

    match save_upload(&result.audio_data, &filename, content_type, &account.0) {
        Ok(url) => Json(serde_json::json!({
            "url": url,
            "format": result.format,
            "duration_ms": result.duration_ms,
            "provider": result.provider,
            "model": result.model,
            "sample_rate": result.sample_rate,
        }))
        .into_response(),
        Err(e) => ApiErrorResponse::internal(format!("Failed to save audio: {e}")).into_response(),
    }
}

// ── GET /media/providers ────────────────────────────────────────────────

/// List available media providers with their capabilities and config status.
pub async fn list_media_providers(
    account: AccountId,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if let Err((code, json)) = require_admin(&account, &state.kernel.config_ref().admin_accounts) {
        return (code, json).into_response();
    }

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
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_task_registry_enforces_owner_and_provider_match() {
        let task_id = uuid::Uuid::new_v4().to_string();
        MEDIA_TASK_REGISTRY.insert(
            task_id.clone(),
            MediaTaskMeta {
                provider: "minimax".to_string(),
                account_id: Some("tenant-a".to_string()),
            },
        );

        {
            let meta = MEDIA_TASK_REGISTRY
                .get(&task_id)
                .expect("task should exist");
            assert_eq!(meta.provider, "minimax");
            assert_eq!(meta.account_id.as_deref(), Some("tenant-a"));
            assert_ne!(meta.account_id.as_deref(), Some("tenant-b"));
        }

        MEDIA_TASK_REGISTRY.remove(&task_id);
    }

    #[test]
    fn media_task_metadata_persists_across_registry_reset() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let task_id = "provider-task-123";
        let meta = MediaTaskMeta {
            provider: "minimax".to_string(),
            account_id: Some("tenant-a".to_string()),
        };

        persist_media_task_meta_for_data_dir(tmp.path(), task_id, &meta).expect("persist succeeds");
        MEDIA_TASK_REGISTRY.remove(task_id);

        let loaded = load_media_task_meta_for_data_dir(tmp.path(), task_id)
            .expect("task metadata should reload from disk");
        assert_eq!(loaded, meta);
    }

    #[test]
    fn media_provider_list_requires_admin_for_scoped_tenants() {
        let account = AccountId(Some("tenant-a".to_string()));
        let admins = vec!["admin-a".to_string()];
        let result = require_admin(&account, &admins);

        assert!(result.is_err());
        let (status, json) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json["error"], "This endpoint requires admin access");
    }
}
