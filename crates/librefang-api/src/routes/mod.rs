//! Route handlers for the LibreFang API.
//!
//! This module is split into domain-specific sub-modules for maintainability.
//! All public handler functions are re-exported here for backward compatibility.

mod agents;
mod budget;
mod channels;
mod config;
mod network;
mod providers;
mod skills;
mod system;
mod workflows;

// Re-export everything so `routes::handler_name` still works in server.rs.
pub use agents::*;
pub use budget::*;
pub use channels::*;
pub use config::*;
pub use network::*;
pub use providers::*;
pub use skills::*;
pub use system::*;
pub use workflows::*;

use dashmap::DashMap;
use librefang_kernel::LibreFangKernel;
use std::sync::Arc;
use std::time::Instant;

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
}
