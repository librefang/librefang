//! Route handlers for the LibreFang API.
//!
//! Each domain sub-module exports a `router()` function that builds its own route tree.
//! `server.rs` combines all sub-routers via `.merge()`, avoiding hundreds of route
//! registrations in a single function.
//!
//! Handler functions are still exposed via glob re-export to maintain
//! `routes::handler_name` backward compatibility (in particular, the utoipa macros
//! in openapi.rs require this path format).

// All modules export a `router()` function; glob re-export causes a name ambiguity
// warning, but `router()` is only accessed via qualified paths (e.g.
// `routes::agents::router()`), so there is no actual conflict.
#![allow(ambiguous_glob_reexports)]

pub mod agents;
pub mod audit;
pub mod auto_dream;
pub mod budget;
pub mod channels;
pub mod config;
pub mod goals;
pub mod inbox;
pub mod mcp_auth;
pub mod media;
pub mod memory;
pub mod network;
pub mod plugins;
pub mod prompts;
pub mod providers;
pub mod skills;
pub mod system;
pub mod terminal;
pub mod workflows;

// Glob re-export to keep `routes::handler_name` backward compatible
// (utoipa macros in openapi.rs, ws.rs, etc. all depend on this path format).
//
// Previously both system.rs and workflows.rs exported `list_templates` / `get_template`,
// causing E0659 name ambiguity. The workflows.rs versions have been renamed to
// `list_workflow_templates` / `get_workflow_template` to resolve the conflict.
//
// All modules export a `router()` function; glob re-export produces an ambiguity
// warning, but `router()` is only accessed via qualified paths (e.g.
// `routes::agents::router()`), so there is no actual conflict.
pub use agents::*;
pub use audit::*;
pub use auto_dream::*;
pub use budget::*;
pub use channels::*;
pub use config::*;
pub use goals::*;
pub use inbox::*;
pub use mcp_auth::*;
pub use media::*;
pub use memory::*;
pub use network::*;
pub use plugins::*;
pub use providers::*;
pub use skills::*;
pub use system::*;
pub use terminal::*;
pub use workflows::*;

use crate::middleware::RequestLanguage;
use dashmap::DashMap;
use librefang_kernel::LibreFangKernel;
use librefang_types::i18n::{self, ErrorTranslator};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Extract an [`ErrorTranslator`] from the request extensions.
///
/// Uses the language resolved by the `accept_language` middleware, or falls
/// back to English if the middleware hasn't run (e.g. in tests).
#[allow(dead_code)]
pub(crate) fn translator_from_extensions(extensions: &axum::http::Extensions) -> ErrorTranslator {
    let lang = extensions
        .get::<RequestLanguage>()
        .map(|rl| rl.0)
        .unwrap_or(i18n::DEFAULT_LANGUAGE);
    ErrorTranslator::new(lang)
}

/// Resolve the client language from an optional `RequestLanguage` extension.
#[allow(dead_code)]
pub(crate) fn resolve_lang(lang: Option<&axum::Extension<RequestLanguage>>) -> &'static str {
    lang.map(|l| l.0 .0).unwrap_or(i18n::DEFAULT_LANGUAGE)
}

/// Shared application state.
///
/// The kernel is wrapped in Arc so it can serve as both the main kernel
/// and the KernelHandle for inter-agent tool access.
pub struct AppState {
    pub kernel: Arc<LibreFangKernel>,
    pub started_at: Instant,
    /// Optional peer registry for OFP mesh networking status.
    pub peer_registry: Option<Arc<librefang_wire::registry::PeerRegistry>>,
    /// Channel bridge manager — held behind a Mutex so it can be swapped on hot-reload.
    pub bridge_manager: tokio::sync::Mutex<Option<librefang_channels::bridge::BridgeManager>>,
    /// Live channel config — updated on every hot-reload so list_channels() reflects reality.
    pub channels_config: tokio::sync::RwLock<librefang_types::config::ChannelsConfig>,
    /// Notify handle to trigger graceful HTTP server shutdown from the API.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
    /// ClawHub response cache — prevents 429 rate limiting on rapid dashboard refreshes.
    /// Maps cache key → (fetched_at, response_json) with 120s TTL.
    pub clawhub_cache: DashMap<String, (Instant, serde_json::Value)>,
    /// Skillhub response cache — prevents rate limiting on rapid dashboard refreshes.
    /// Maps cache key → (fetched_at, response_json) with TTL.
    pub skillhub_cache: DashMap<String, (Instant, serde_json::Value)>,
    /// Probe cache for local provider health checks (ollama/vllm/lmstudio).
    /// Avoids blocking the `/api/providers` endpoint on TCP timeouts to
    /// unreachable local services. 60-second TTL.
    pub provider_probe_cache: librefang_runtime::provider_health::ProbeCache,
    /// Cache for manual provider test results (latency, timestamp, reachable).
    /// Populated by POST /api/providers/{name}/test, consumed by GET /api/providers.
    pub provider_test_cache: DashMap<String, (Instant, u128, String, bool)>,
    /// Webhook subscription store for outbound event notifications.
    pub webhook_store: crate::webhook_store::WebhookStore,
    /// Active session tokens issued by dashboard login.
    /// Maps token string -> SessionToken (with creation timestamp for expiry checks).
    pub active_sessions:
        Arc<tokio::sync::RwLock<HashMap<String, crate::password_hash::SessionToken>>>,
    /// Shared api_key_lock from the auth middleware — updated on password/api_key change
    /// so the new credentials take effect immediately without restart.
    pub api_key_lock: Arc<tokio::sync::RwLock<String>>,
    /// Media generation driver cache for image/TTS/video/music.
    pub media_drivers: librefang_runtime::media::MediaDriverCache,
    /// Dynamic webhook router for channel webhook endpoints.
    /// Mounted under `/channels` on the main server. Updated on hot-reload.
    pub webhook_router: Arc<tokio::sync::RwLock<Arc<axum::Router>>>,
    /// Mutex for serializing config file writes — prevents concurrent config_set
    /// calls from reading the same file and overwriting each other's changes.
    pub config_write_lock: tokio::sync::Mutex<()>,
    /// Prometheus metrics handle (only set when `telemetry` feature + config enabled).
    #[cfg(feature = "telemetry")]
    pub prometheus_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
}
