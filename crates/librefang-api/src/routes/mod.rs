//! Route handlers for the LibreFang API.
//!
//! This module is split into domain-specific sub-modules for maintainability.
//! All public handler functions are re-exported here for backward compatibility.

mod agents;
mod budget;
mod channels;
mod config;
pub mod goals;
mod inbox;
mod memory;
mod network;
mod plugins;
mod providers;
mod skills;
mod system;
mod workflows;

// Re-export everything so `routes::handler_name` still works in server.rs.
pub use agents::*;
pub use budget::*;
pub use channels::*;
pub use config::*;
pub use goals::*;
pub use inbox::*;
pub use memory::*;
pub use network::*;
pub use plugins::*;
pub use providers::*;
pub use skills::*;
pub use system::*;
pub use workflows::*;

use crate::middleware::RequestLanguage;
use dashmap::DashMap;
use librefang_kernel::LibreFangKernel;
use librefang_types::i18n::{self, ErrorTranslator};
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
    /// Probe cache for local provider health checks (ollama/vllm/lmstudio).
    /// Avoids blocking the `/api/providers` endpoint on TCP timeouts to
    /// unreachable local services. 60-second TTL.
    pub provider_probe_cache: librefang_runtime::provider_health::ProbeCache,
    /// Webhook subscription store for outbound event notifications.
    pub webhook_store: crate::webhook_store::WebhookStore,
}
