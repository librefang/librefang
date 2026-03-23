//! Media generation drivers — provider-agnostic abstraction for image, TTS,
//! video, and music generation.
//!
//! Architecture mirrors `crate::drivers` (LLM drivers):
//! - `MediaDriver` trait with per-modality methods and default `NotSupported` impls
//! - `MediaDriverCache` for lazy-init, thread-safe driver caching
//! - Per-provider implementations in submodules

pub mod minimax;
pub mod openai;

use async_trait::async_trait;
use dashmap::DashMap;
use librefang_types::media::{
    MediaCapability, MediaImageRequest, MediaImageResult, MediaMusicRequest, MediaMusicResult,
    MediaTaskStatus, MediaTtsRequest, MediaTtsResult, MediaVideoRequest, MediaVideoResult,
    MediaVideoSubmitResult,
};
use std::fmt;
use std::sync::Arc;

// ── Error type ─────────────────────────────────────────────────────────

/// Errors from media generation drivers.
#[derive(Debug, Clone)]
pub enum MediaError {
    /// The requested capability is not supported by this driver.
    NotSupported(String),
    /// API key is missing.
    MissingKey(String),
    /// HTTP or network error.
    Http(String),
    /// Provider returned an error response.
    Api { status: u16, message: String },
    /// Rate limited.
    RateLimit(String),
    /// Content was rejected (e.g. safety filter).
    ContentFiltered(String),
    /// Invalid request parameters.
    InvalidRequest(String),
    /// Task not found (for async operations).
    TaskNotFound(String),
    /// Generic error.
    Other(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::NotSupported(cap) => write!(f, "{cap} not supported by this driver"),
            MediaError::MissingKey(key) => write!(f, "API key not set: {key}"),
            MediaError::Http(e) => write!(f, "HTTP error: {e}"),
            MediaError::Api { status, message } => {
                write!(f, "API error (HTTP {status}): {message}")
            }
            MediaError::RateLimit(msg) => write!(f, "Rate limited: {msg}"),
            MediaError::ContentFiltered(msg) => write!(f, "Content filtered: {msg}"),
            MediaError::InvalidRequest(msg) => write!(f, "Invalid request: {msg}"),
            MediaError::TaskNotFound(id) => write!(f, "Task not found: {id}"),
            MediaError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for MediaError {}

// ── Driver trait ───────────────────────────────────────────────────────

/// Provider-agnostic media generation driver.
///
/// Each provider implements only the modalities it supports. Unimplemented
/// methods return `MediaError::NotSupported` by default (same pattern as
/// `KernelHandle`).
#[async_trait]
pub trait MediaDriver: Send + Sync {
    /// Which capabilities this driver provides.
    fn capabilities(&self) -> Vec<MediaCapability>;

    /// Whether the driver has valid credentials configured.
    fn is_configured(&self) -> bool {
        true
    }

    /// Provider name (e.g. "openai", "minimax").
    fn provider_name(&self) -> &str;

    // ── Image generation (sync) ────────────────────────────────────

    async fn generate_image(
        &self,
        _request: &MediaImageRequest,
    ) -> Result<MediaImageResult, MediaError> {
        Err(MediaError::NotSupported("image generation".into()))
    }

    // ── Text-to-speech (sync) ──────────────────────────────────────

    async fn synthesize_speech(
        &self,
        _request: &MediaTtsRequest,
    ) -> Result<MediaTtsResult, MediaError> {
        Err(MediaError::NotSupported("text-to-speech".into()))
    }

    // ── Video generation (async: submit → poll → result) ───────────

    async fn submit_video(
        &self,
        _request: &MediaVideoRequest,
    ) -> Result<MediaVideoSubmitResult, MediaError> {
        Err(MediaError::NotSupported("video generation".into()))
    }

    async fn poll_video(&self, _task_id: &str) -> Result<MediaTaskStatus, MediaError> {
        Err(MediaError::NotSupported("video generation".into()))
    }

    async fn get_video_result(&self, _task_id: &str) -> Result<MediaVideoResult, MediaError> {
        Err(MediaError::NotSupported("video generation".into()))
    }

    // ── Music generation (sync, but slow) ──────────────────────────

    async fn generate_music(
        &self,
        _request: &MediaMusicRequest,
    ) -> Result<MediaMusicResult, MediaError> {
        Err(MediaError::NotSupported("music generation".into()))
    }
}

// ── Driver cache ───────────────────────────────────────────────────────

/// Thread-safe, lazy-initializing cache for media drivers.
pub struct MediaDriverCache {
    cache: DashMap<String, Arc<dyn MediaDriver>>,
}

impl MediaDriverCache {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
        }
    }

    /// Get or create a cached driver for the given provider.
    pub fn get_or_create(
        &self,
        provider: &str,
        base_url: Option<&str>,
    ) -> Result<Arc<dyn MediaDriver>, MediaError> {
        let key = format!("{}|{}", provider, base_url.unwrap_or("default"));

        if let Some(driver) = self.cache.get(&key) {
            return Ok(Arc::clone(driver.value()));
        }

        let driver = create_media_driver(provider, base_url)?;
        self.cache.insert(key, Arc::clone(&driver));
        Ok(driver)
    }

    /// Auto-detect and return the first configured driver that supports the
    /// given capability.
    pub fn detect_for_capability(
        &self,
        capability: MediaCapability,
    ) -> Result<Arc<dyn MediaDriver>, MediaError> {
        // Try providers in preference order
        for provider in MEDIA_PROVIDER_ORDER {
            if let Ok(driver) = self.get_or_create(provider, None) {
                if driver.is_configured() && driver.capabilities().contains(&capability) {
                    return Ok(driver);
                }
            }
        }
        Err(MediaError::MissingKey(format!(
            "No configured provider found for {capability}"
        )))
    }

    /// Clear all cached drivers (for hot-reload).
    pub fn clear(&self) {
        self.cache.clear();
    }
}

impl Default for MediaDriverCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── Provider registry ──────────────────────────────────────────────────

/// Provider preference order for auto-detection.
static MEDIA_PROVIDER_ORDER: &[&str] = &["openai", "minimax", "elevenlabs"];

/// Create a media driver for a given provider name.
fn create_media_driver(
    provider: &str,
    base_url: Option<&str>,
) -> Result<Arc<dyn MediaDriver>, MediaError> {
    match provider {
        "minimax" => Ok(Arc::new(minimax::MiniMaxMediaDriver::new(base_url))),
        "openai" => Ok(Arc::new(openai::OpenAIMediaDriver::new(base_url))),
        other => Err(MediaError::Other(format!(
            "Unknown media provider: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_error_display() {
        let err = MediaError::NotSupported("video".into());
        assert_eq!(err.to_string(), "video not supported by this driver");

        let err = MediaError::Api {
            status: 429,
            message: "too many requests".into(),
        };
        assert_eq!(err.to_string(), "API error (HTTP 429): too many requests");
    }

    #[test]
    fn test_cache_creation() {
        let cache = MediaDriverCache::new();
        // MiniMax driver should be creatable (even without API key)
        let driver = cache.get_or_create("minimax", None);
        assert!(driver.is_ok());
    }

    #[test]
    fn test_cache_reuse() {
        let cache = MediaDriverCache::new();
        let d1 = cache.get_or_create("minimax", None).unwrap();
        let d2 = cache.get_or_create("minimax", None).unwrap();
        assert!(Arc::ptr_eq(&d1, &d2));
    }

    #[test]
    fn test_cache_clear() {
        let cache = MediaDriverCache::new();
        let _ = cache.get_or_create("minimax", None);
        cache.clear();
        // After clear, new instance is created
        let d = cache.get_or_create("minimax", None).unwrap();
        assert_eq!(d.provider_name(), "minimax");
    }

    #[test]
    fn test_unknown_provider() {
        let cache = MediaDriverCache::new();
        let result = cache.get_or_create("nonexistent", None);
        assert!(result.is_err());
    }
}
