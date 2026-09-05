//! Media generation drivers — provider-agnostic abstraction for image, TTS,
//! video, and music generation.
//!
//! Architecture mirrors `crate::drivers` (LLM drivers):
//! - `MediaDriver` trait with per-modality methods and default `NotSupported` impls
//! - `MediaDriverCache` for lazy-init, thread-safe driver caching
//! - Per-provider implementations in submodules

pub mod elevenlabs;
pub mod gemini;
pub mod google_tts;
pub mod media_understanding;
pub mod minimax;
pub mod openai;

/// UTF-8-safe truncation helper. Duplicates `librefang_runtime::str_utils::safe_truncate_str`
/// (kept in `librefang-runtime` because most call sites still live there). Splitting it
/// into its own crate just for media would be over-engineering; the function is < 10 LOC.
#[inline]
pub(crate) fn safe_truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

use async_trait::async_trait;
use dashmap::DashMap;
use librefang_types::media::{
    CapabilityRouting, MediaCapability, MediaImageRequest, MediaImageResult, MediaMusicRequest,
    MediaMusicResult, MediaTaskStatus, MediaTtsRequest, MediaTtsResult, MediaVideoRequest,
    MediaVideoResult, MediaVideoSubmitResult,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use tracing::warn;

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
///
/// Holds an optional `provider_urls` map (from `KernelConfig`) so that
/// custom base URLs (e.g. OpenAI proxies, MiniMax China endpoint) are
/// respected when creating drivers.
pub struct MediaDriverCache {
    cache: DashMap<String, Arc<dyn MediaDriver>>,
    /// Provider name → custom base URL, sourced from config `[provider_urls]`.
    /// Behind RwLock for hot-reload support (update URLs via `&self`).
    provider_urls: RwLock<HashMap<String, String>>,
    /// Provider IDs that support media, in preference order.
    /// Loaded from the registry (providers/*.toml) at boot.
    media_providers: RwLock<Vec<String>>,
    /// Operator-nominated provider per capability, from the kernel-global
    /// `[capabilities]` block. Consulted *before* `media_providers` in
    /// [`MediaDriverCache::detect_for_capability`] so "use MiniMax for video,
    /// ElevenLabs for speech" is a config line rather than a code change.
    ///
    /// `BTreeMap` (not `HashMap`) because the resolved routing is logged and
    /// surfaced to operators, and this repo requires deterministic ordering
    /// for anything that gets stringified.
    capability_routing: RwLock<BTreeMap<MediaCapability, String>>,
}

fn read_media_state<'a, T>(lock: &'a RwLock<T>, state: &'static str) -> RwLockReadGuard<'a, T> {
    lock.read().unwrap_or_else(|poisoned| {
        warn!(
            state,
            "media driver state read lock poisoned; recovering inner state"
        );
        lock.clear_poison();
        poisoned.into_inner()
    })
}

fn write_media_state<'a, T>(lock: &'a RwLock<T>, state: &'static str) -> RwLockWriteGuard<'a, T> {
    lock.write().unwrap_or_else(|poisoned| {
        warn!(
            state,
            "media driver state write lock poisoned; recovering inner state"
        );
        lock.clear_poison();
        poisoned.into_inner()
    })
}

impl MediaDriverCache {
    /// Create a cache with no provider URL overrides.
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            provider_urls: RwLock::new(HashMap::new()),
            media_providers: RwLock::new(vec![
                "openai".into(),
                "gemini".into(),
                "elevenlabs".into(),
                "minimax".into(),
                "google_tts".into(),
            ]),
            capability_routing: RwLock::new(BTreeMap::new()),
        }
    }

    /// Create a cache that resolves base URLs from the given map.
    ///
    /// This mirrors how LLM drivers use `config.provider_urls` — when a
    /// caller passes `base_url: None` to [`get_or_create`], the cache
    /// checks `provider_urls` before falling back to the driver's hardcoded
    /// default.
    ///
    /// Accepts any map type that can be iterated as `(String, String)` pairs,
    /// including both `HashMap` and `BTreeMap`.
    pub fn new_with_urls(provider_urls: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            cache: DashMap::new(),
            provider_urls: RwLock::new(provider_urls.into_iter().collect()),
            media_providers: RwLock::new(vec![
                "openai".into(),
                "gemini".into(),
                "elevenlabs".into(),
                "minimax".into(),
                "google_tts".into(),
            ]),
            capability_routing: RwLock::new(BTreeMap::new()),
        }
    }

    /// Update the media provider list from registry data.
    /// Providers that declare `media_capabilities` in their TOML are included.
    /// Built-in providers are always appended as fallback.
    pub fn load_providers_from_registry(
        &self,
        providers: &[librefang_types::model_catalog::ProviderInfo],
    ) {
        let mut media_provs: Vec<String> = providers
            .iter()
            .filter(|p| !p.media_capabilities.is_empty())
            .map(|p| p.id.clone())
            .collect();
        for builtin in ["openai", "gemini", "elevenlabs", "minimax", "google_tts"] {
            if !media_provs.iter().any(|p| p == builtin) {
                media_provs.push(builtin.to_string());
            }
        }
        *write_media_state(&self.media_providers, "media_providers") = media_provs;
    }

    /// Get or create a cached driver for the given provider.
    ///
    /// If `base_url` is `None`, the cache checks its `provider_urls` map
    /// for a configured override before using the driver's default.
    pub fn get_or_create(
        &self,
        provider: &str,
        base_url: Option<&str>,
    ) -> Result<Arc<dyn MediaDriver>, MediaError> {
        // Resolve base_url: explicit arg > provider_urls map > driver default
        let resolved_url = base_url.map(|u| u.to_string()).or_else(|| {
            let urls = read_media_state(&self.provider_urls, "provider_urls");
            urls.get(provider)
                .cloned()
                // Also check the canonical name for aliases ("google" → "gemini")
                .or_else(|| {
                    let canonical = canonical_provider_name(provider);
                    if canonical != provider {
                        urls.get(canonical).cloned()
                    } else {
                        None
                    }
                })
        });
        let url_ref = resolved_url.as_deref();

        let key = format!("{}|{}", provider, url_ref.unwrap_or("default"));

        if let Some(driver) = self.cache.get(&key) {
            return Ok(Arc::clone(driver.value()));
        }

        let driver = create_media_driver(provider, url_ref)?;
        self.cache.insert(key, Arc::clone(&driver));
        Ok(driver)
    }

    /// Install the operator-nominated provider per capability from a
    /// `[capabilities]` block.
    ///
    /// Only the `provider` half is used here — the generation drivers take
    /// their model from the per-request `model` field, so a `model` in the
    /// routing block has no driver-selection meaning. Entries with no
    /// provider are skipped rather than stored as an empty preference.
    pub fn set_capability_routing(&self, routing: &CapabilityRouting) {
        let mut table = BTreeMap::new();
        for cap in CapabilityRouting::ALL {
            if let Some(provider) = routing.get(cap).and_then(|t| t.provider.as_deref()) {
                table.insert(cap, provider.to_string());
            }
        }
        *write_media_state(&self.capability_routing, "capability_routing") = table;
    }

    /// Auto-detect and return the first configured driver that supports the
    /// given capability.
    ///
    /// The operator's `[capabilities]` nomination is tried first; it only
    /// wins if that provider is actually configured *and* actually advertises
    /// the capability, so a stale or mistaken nomination degrades to the
    /// registry preference order instead of failing the call outright.
    pub fn detect_for_capability(
        &self,
        capability: MediaCapability,
    ) -> Result<Arc<dyn MediaDriver>, MediaError> {
        let nominated = read_media_state(&self.capability_routing, "capability_routing")
            .get(&capability)
            .cloned();
        if let Some(provider) = nominated {
            match self.get_or_create(&provider, None) {
                Ok(driver)
                    if driver.is_configured() && driver.capabilities().contains(&capability) =>
                {
                    return Ok(driver);
                }
                _ => warn!(
                    provider = %provider,
                    capability = %capability,
                    "[capabilities] nominates a provider that is not configured for this \
                     capability; falling back to the registry preference order"
                ),
            }
        }

        let providers = read_media_state(&self.media_providers, "media_providers");
        for provider in providers.iter() {
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

    /// Update the provider URL overrides and clear the driver cache so that
    /// drivers are recreated with the new URLs on next access.
    ///
    /// Accepts any map type that can be iterated as `(String, String)` pairs,
    /// including both `HashMap` and `BTreeMap`.
    pub fn update_provider_urls(&self, urls: impl IntoIterator<Item = (String, String)>) {
        *write_media_state(&self.provider_urls, "provider_urls") = urls.into_iter().collect();
        self.cache.clear();
    }
}

impl Default for MediaDriverCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Map provider aliases to canonical names for URL lookup.
fn canonical_provider_name(provider: &str) -> &str {
    match provider {
        "google" => "gemini",
        _ => provider,
    }
}

// ── Provider registry ──────────────────────────────────────────────────

/// Create a media driver for a given provider name.
fn create_media_driver(
    provider: &str,
    base_url: Option<&str>,
) -> Result<Arc<dyn MediaDriver>, MediaError> {
    match provider {
        "elevenlabs" => Ok(Arc::new(elevenlabs::ElevenLabsMediaDriver::new(base_url))),
        "gemini" | "google" => Ok(Arc::new(gemini::GeminiMediaDriver::new(base_url))),
        "minimax" => Ok(Arc::new(minimax::MiniMaxMediaDriver::new(base_url))),
        "openai" => Ok(Arc::new(openai::OpenAIMediaDriver::new(base_url))),
        "google_tts" => Ok(Arc::new(google_tts::GoogleTtsMediaDriver::new(base_url))),
        other => {
            // For user-defined providers with a configured base_url, use the
            // generic OpenAI-compatible driver. The API key is read from the
            // {PROVIDER_UPPER}_API_KEY environment variable.
            if let Some(url) = base_url {
                Ok(Arc::new(openai::GenericOpenAICompatMediaDriver::new(
                    other, url,
                )))
            } else {
                Err(MediaError::InvalidRequest(format!(
                    "Unknown media provider '{other}' and no base_url configured. \
                     Set provider_urls.{other} in config.toml to use this provider \
                     with the OpenAI-compatible API."
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_media_state_locks_recover_and_hot_reload_still_applies() {
        let cache = MediaDriverCache::new();
        let urls_poison = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let mut urls = cache.provider_urls.write().unwrap();
                    urls.insert("stale".to_string(), "https://stale.invalid".to_string());
                    panic!("poison media provider URLs lock");
                })
                .join()
        });
        assert!(urls_poison.is_err());
        assert!(cache.provider_urls.is_poisoned());

        cache
            .update_provider_urls([("custom".to_string(), "https://media.example/v1".to_string())]);
        assert!(!cache.provider_urls.is_poisoned());
        assert!(cache.get_or_create("custom", None).is_ok());

        let providers_poison = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let mut providers = cache.media_providers.write().unwrap();
                    providers.clear();
                    panic!("poison media providers lock");
                })
                .join()
        });
        assert!(providers_poison.is_err());
        assert!(cache.media_providers.is_poisoned());
        assert!(read_media_state(&cache.media_providers, "media_providers").is_empty());
        assert!(!cache.media_providers.is_poisoned());
        *cache.media_providers.write().unwrap() = vec!["custom".into()];
        assert_eq!(cache.media_providers.read().unwrap().as_slice(), ["custom"]);
    }

    /// Minimal driver used to exercise capability selection without needing
    /// any provider credentials in the test environment.
    struct FakeDriver {
        name: &'static str,
        caps: Vec<MediaCapability>,
    }

    #[async_trait]
    impl MediaDriver for FakeDriver {
        fn capabilities(&self) -> Vec<MediaCapability> {
            self.caps.clone()
        }
        fn provider_name(&self) -> &str {
            self.name
        }
    }

    fn cache_with_fakes(fakes: &[(&'static str, Vec<MediaCapability>)]) -> MediaDriverCache {
        let cache = MediaDriverCache::new();
        for (name, caps) in fakes {
            cache.cache.insert(
                format!("{name}|default"),
                Arc::new(FakeDriver {
                    name,
                    caps: caps.clone(),
                }) as Arc<dyn MediaDriver>,
            );
        }
        *write_media_state(&cache.media_providers, "media_providers") =
            fakes.iter().map(|(n, _)| n.to_string()).collect();
        cache
    }

    #[test]
    fn detect_for_capability_prefers_the_nominated_provider_over_registry_order() {
        let cache = cache_with_fakes(&[
            ("beta", vec![MediaCapability::ImageGeneration]),
            ("alpha", vec![MediaCapability::ImageGeneration]),
        ]);
        assert_eq!(
            cache
                .detect_for_capability(MediaCapability::ImageGeneration)
                .unwrap()
                .provider_name(),
            "beta"
        );

        let routing: CapabilityRouting =
            toml::from_str("image_generation = \"alpha\"\n").expect("parse routing");
        cache.set_capability_routing(&routing);
        assert_eq!(
            cache
                .detect_for_capability(MediaCapability::ImageGeneration)
                .unwrap()
                .provider_name(),
            "alpha",
            "[capabilities] nomination must win over the registry preference order"
        );
    }

    #[test]
    fn detect_for_capability_falls_back_when_the_nomination_lacks_the_capability() {
        let cache = cache_with_fakes(&[
            ("beta", vec![MediaCapability::ImageGeneration]),
            ("alpha", vec![MediaCapability::TextToSpeech]),
        ]);
        let routing: CapabilityRouting =
            toml::from_str("image_generation = \"alpha\"\n").expect("parse routing");
        cache.set_capability_routing(&routing);

        assert_eq!(
            cache
                .detect_for_capability(MediaCapability::ImageGeneration)
                .unwrap()
                .provider_name(),
            "beta",
            "a mistaken nomination must degrade to the preference order, not fail the call"
        );
    }

    #[test]
    fn set_capability_routing_ignores_entries_that_name_no_provider() {
        let cache = cache_with_fakes(&[("beta", vec![MediaCapability::ImageGeneration])]);
        let routing: CapabilityRouting =
            toml::from_str("image_generation = { model = \"only-a-model\" }\n")
                .expect("parse routing");
        cache.set_capability_routing(&routing);
        assert!(read_media_state(&cache.capability_routing, "capability_routing").is_empty());
        assert_eq!(
            cache
                .detect_for_capability(MediaCapability::ImageGeneration)
                .unwrap()
                .provider_name(),
            "beta"
        );
    }

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

    #[test]
    fn test_provider_urls_resolved() {
        let mut urls = HashMap::new();
        urls.insert(
            "minimax".to_string(),
            "https://custom.minimax.com/v1".to_string(),
        );
        let cache = MediaDriverCache::new_with_urls(urls);
        let driver = cache.get_or_create("minimax", None).unwrap();
        assert_eq!(driver.provider_name(), "minimax");
    }

    #[test]
    fn test_provider_urls_alias_resolution() {
        let mut urls = HashMap::new();
        urls.insert(
            "gemini".to_string(),
            "https://custom.gemini.api/v1beta".to_string(),
        );
        let cache = MediaDriverCache::new_with_urls(urls);
        // "google" is an alias for "gemini" — should resolve the URL
        let driver = cache.get_or_create("google", None).unwrap();
        assert_eq!(driver.provider_name(), "gemini");
    }

    #[test]
    fn test_explicit_base_url_overrides_config() {
        let mut urls = HashMap::new();
        urls.insert(
            "minimax".to_string(),
            "https://config-url.com/v1".to_string(),
        );
        let cache = MediaDriverCache::new_with_urls(urls);
        // Explicit base_url should take precedence over provider_urls
        let driver = cache
            .get_or_create("minimax", Some("https://explicit.com/v1"))
            .unwrap();
        assert_eq!(driver.provider_name(), "minimax");
        // Different key means a separate cache entry from the config-resolved one
        let driver2 = cache.get_or_create("minimax", None).unwrap();
        assert!(!Arc::ptr_eq(&driver, &driver2));
    }

    #[test]
    fn test_load_providers_from_registry_filters_by_media_capabilities() {
        let cache = MediaDriverCache::new();
        let providers = vec![
            librefang_types::model_catalog::ProviderInfo {
                id: "openai".into(),
                media_capabilities: vec!["image_generation".into(), "text_to_speech".into()],
                ..Default::default()
            },
            librefang_types::model_catalog::ProviderInfo {
                id: "anthropic".into(),
                media_capabilities: vec![], // no media support
                ..Default::default()
            },
            librefang_types::model_catalog::ProviderInfo {
                id: "minimax".into(),
                media_capabilities: vec!["video_generation".into()],
                ..Default::default()
            },
        ];
        cache.load_providers_from_registry(&providers);
        let list = cache.media_providers.read().unwrap();
        // openai and minimax from registry, plus builtins (gemini, elevenlabs) appended
        assert!(list.iter().any(|p| p == "openai"));
        assert!(list.iter().any(|p| p == "minimax"));
        assert!(list.iter().any(|p| p == "gemini"));
        assert!(list.iter().any(|p| p == "elevenlabs"));
        // anthropic should NOT be in the list (no media capabilities)
        assert!(!list.iter().any(|p| p == "anthropic"));
    }

    #[test]
    fn test_load_providers_from_registry_preserves_order() {
        let cache = MediaDriverCache::new();
        let providers = vec![
            librefang_types::model_catalog::ProviderInfo {
                id: "minimax".into(),
                media_capabilities: vec!["video_generation".into()],
                ..Default::default()
            },
            librefang_types::model_catalog::ProviderInfo {
                id: "openai".into(),
                media_capabilities: vec!["image_generation".into()],
                ..Default::default()
            },
        ];
        cache.load_providers_from_registry(&providers);
        let list = cache.media_providers.read().unwrap();
        // Registry order should be preserved: minimax first, then openai
        let minimax_pos = list.iter().position(|p| p == "minimax").unwrap();
        let openai_pos = list.iter().position(|p| p == "openai").unwrap();
        assert!(minimax_pos < openai_pos);
    }

    #[test]
    fn test_load_providers_empty_registry_keeps_builtins() {
        let cache = MediaDriverCache::new();
        cache.load_providers_from_registry(&[]);
        let list = cache.media_providers.read().unwrap();
        assert_eq!(list.len(), 5);
        assert!(list.contains(&"openai".to_string()));
        assert!(list.contains(&"gemini".to_string()));
        assert!(list.contains(&"elevenlabs".to_string()));
        assert!(list.contains(&"minimax".to_string()));
        assert!(list.contains(&"google_tts".to_string()));
    }
}
