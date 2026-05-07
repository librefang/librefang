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

pub mod agent_templates;
pub mod agents;
pub mod approvals;
pub mod audit;
pub mod authz;
pub mod auto_dream;
pub mod backup;
pub mod bindings;
pub mod budget;
pub mod channels;
pub mod commands;
pub mod config;
pub mod goals;
pub mod inbox;
pub mod logs;
pub mod mcp_auth;
pub mod media;
pub mod memory;
pub mod network;
pub mod pairing;
pub mod plugins;
pub mod prompts;
pub mod providers;
pub mod registry;
pub mod skills;
pub mod system;
pub mod task_queue;
pub mod terminal;
pub mod tools_sessions;
pub mod users;
pub mod webhooks;
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
pub use agent_templates::*;
pub use agents::*;
pub use approvals::*;
pub use audit::*;
pub use authz::*;
pub use auto_dream::*;
pub use backup::*;
pub use bindings::*;
pub use budget::*;
pub use channels::*;
pub use commands::*;
pub use config::*;
pub use goals::*;
pub use inbox::*;
pub use logs::*;
pub use mcp_auth::*;
pub use media::*;
pub use memory::*;
pub use network::*;
pub use pairing::*;
pub use plugins::*;
pub use providers::*;
// `registry::*` is intentionally not re-exported: every handler in
// `routes::registry` is a private `async fn`, so the glob resolves to zero
// items and would trip `-D unused-imports`. The module is reached via the
// qualified `crate::routes::registry::router()` call inside `system.rs`.
pub use skills::*;
pub use system::*;
pub use task_queue::*;
pub use terminal::*;
pub use tools_sessions::*;
pub use users::*;
pub use webhooks::*;
pub use workflows::*;

use crate::middleware::RequestLanguage;
use crate::rate_limiter::KeyedRateLimiter;
use dashmap::DashMap;
use librefang_kernel::KernelApi;
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
/// `kernel` is `Arc<dyn KernelApi>` (#3566) — routes interact with the
/// kernel exclusively through the [`KernelApi`] trait surface, not via
/// the concrete `LibreFangKernel` struct. This is the single explicit
/// contract between the HTTP layer and the kernel; widening it is an
/// explicit choice rather than a side-effect of adding a new inherent
/// method on the kernel struct.
pub struct AppState {
    pub kernel: Arc<dyn KernelApi>,
    pub started_at: Instant,
    /// Channel bridge manager — held in an `ArcSwap` for lock-free reads and atomic
    /// swap on hot-reload. Write sites use `store(Arc::new(new_value))`; the stop
    /// path uses `swap` + `Arc::try_unwrap` to obtain ownership for `stop()`.
    pub bridge_manager: arc_swap::ArcSwap<Option<librefang_channels::bridge::BridgeManager>>,
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
    pub provider_probe_cache: librefang_kernel::provider_health::ProbeCache,
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
    /// Shared per-user API key snapshot — same Arc the auth middleware
    /// reads from, so swapping the inner Vec via `rotate_user_key` (or any
    /// future user-mutation endpoint) makes the change visible to the very
    /// next request without a daemon restart.
    pub user_api_keys: Arc<tokio::sync::RwLock<Vec<crate::middleware::ApiUserAuth>>>,
    /// Media generation driver cache for image/TTS/video/music.
    pub media_drivers: librefang_kernel::media::MediaDriverCache,
    /// Dynamic webhook router for channel webhook endpoints.
    /// Mounted under `/channels` on the main server. Updated on hot-reload.
    pub webhook_router: Arc<tokio::sync::RwLock<Arc<axum::Router>>>,
    /// Mutex for serializing config file writes — prevents concurrent config_set
    /// calls from reading the same file and overwriting each other's changes.
    pub config_write_lock: tokio::sync::Mutex<()>,
    /// Pending A2A agents awaiting operator approval (Bug #3786).
    /// Maps discovery URL → AgentCard. Agents here are NOT trusted yet and
    /// cannot receive tasks. Use POST /api/a2a/agents/{url}/approve to promote
    /// them into the kernel's trusted external-agent list.
    pub pending_a2a_agents: DashMap<String, librefang_kernel::a2a::AgentCard>,
    /// Per-IP brute-force limiter for authentication endpoints.
    /// Shared between the auth-endpoint middleware layer and the background
    /// prune task so stale entries are reclaimed every 5 minutes.
    pub auth_login_limiter: Arc<crate::rate_limiter::AuthLoginLimiter>,
    /// GCRA rate limiter — shared with the middleware layer so the background GC
    /// task can call `retain_recent()` to evict stale per-IP entries and prevent
    /// the DashMap from growing unbounded over a long-running daemon. See #3668.
    pub gcra_limiter: Arc<KeyedRateLimiter>,
    /// Compiled `trusted_proxies` allowlist — built once at boot and shared with
    /// the GCRA + auth-login middlewares (see `server.rs`). Re-used by WS
    /// upgrade handlers (`ws::agent_ws`, `routes::terminal::terminal_ws`) to
    /// resolve the real client IP for per-IP slot keying without re-parsing
    /// the raw config strings (and re-emitting the malformed-entry warning)
    /// on every upgrade.
    pub trusted_proxies: Arc<crate::client_ip::TrustedProxies>,
    /// Master switch matching `KernelConfig::trust_forwarded_for`. Cached at
    /// boot alongside `trusted_proxies` so WS handlers don't have to hold a
    /// `config_ref()` guard just to read this single bool.
    pub trust_forwarded_for: bool,
    /// Persistent Idempotency-Key replay cache (#3637). Reuses the
    /// substrate's SQLite connection so replays survive daemon
    /// restarts within the 24h TTL window.
    pub idempotency_store: Arc<dyn librefang_memory::idempotency::IdempotencyStore + Send + Sync>,
}
