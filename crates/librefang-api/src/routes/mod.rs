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
pub mod groups;
pub mod inbox;
pub mod logs;
pub mod mcp_auth;
pub mod media;
pub mod memory;
pub mod network;
pub mod pairing;
pub mod passkey;
pub mod plugins;
pub mod prompts;
pub mod providers;
pub mod provisioning;
pub mod registry;
pub mod secrets_env;
pub mod sidecar_describe;
pub mod sidecar_toml;
pub mod skills;
pub mod system;
pub mod task_queue;
pub mod terminal;
pub mod tools_sessions;
pub mod users;
pub mod webhooks;
pub mod workflows;

/// Boot a concrete kernel for route unit tests without scattering concrete
/// kernel references across production route modules. The import-surface gate
/// explicitly allowlists this boundary module (#3744).
#[cfg(test)]
pub(crate) fn boot_test_kernel(
    config: librefang_types::config::KernelConfig,
) -> librefang_kernel::LibreFangKernel {
    librefang_kernel::LibreFangKernel::boot_with_config(config).expect("test kernel boots")
}

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
use librefang_types::i18n;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Resolve the client language from an optional `RequestLanguage` extension.
pub(crate) fn resolve_lang(lang: Option<&axum::Extension<RequestLanguage>>) -> &'static str {
    lang.map(|l| l.0 .0).unwrap_or(i18n::DEFAULT_LANGUAGE)
}

const PROVIDER_TEST_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const PENDING_A2A_AGENT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Result of a user-triggered provider connectivity test.
pub struct ProviderTestResult {
    pub(crate) tested_at: Instant,
    pub(crate) latency_ms: u128,
    pub(crate) tested_rfc3339: String,
    pub(crate) reachable: bool,
}

impl ProviderTestResult {
    pub(crate) fn new(latency_ms: u128, reachable: bool) -> Self {
        Self {
            tested_at: Instant::now(),
            latency_ms,
            tested_rfc3339: chrono::Utc::now().to_rfc3339(),
            reachable,
        }
    }

    pub(crate) fn is_fresh(&self) -> bool {
        self.is_fresh_at(Instant::now())
    }

    fn is_fresh_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.tested_at) < PROVIDER_TEST_CACHE_TTL
    }
}

/// An unapproved A2A discovery and the time its lease was last refreshed.
pub struct PendingA2aAgent {
    pub(crate) card: librefang_kernel::a2a::AgentCard,
    pub(crate) discovered_at: Instant,
}

impl PendingA2aAgent {
    pub(crate) fn new(card: librefang_kernel::a2a::AgentCard) -> Self {
        Self {
            card,
            discovered_at: Instant::now(),
        }
    }

    fn is_fresh_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.discovered_at) < PENDING_A2A_AGENT_TTL
    }
}

pub(crate) struct RouteCachePruneCounts {
    pub(crate) provider_tests: usize,
    pub(crate) pending_a2a_agents: usize,
}

pub(crate) fn prune_route_caches(
    provider_tests: &DashMap<String, ProviderTestResult>,
    pending_a2a_agents: &DashMap<String, PendingA2aAgent>,
) -> RouteCachePruneCounts {
    prune_route_caches_at(provider_tests, pending_a2a_agents, Instant::now())
}

fn prune_route_caches_at(
    provider_tests: &DashMap<String, ProviderTestResult>,
    pending_a2a_agents: &DashMap<String, PendingA2aAgent>,
    now: Instant,
) -> RouteCachePruneCounts {
    let provider_before = provider_tests.len();
    provider_tests.retain(|_, result| result.is_fresh_at(now));
    let pending_before = pending_a2a_agents.len();
    pending_a2a_agents.retain(|_, pending| pending.is_fresh_at(now));
    RouteCachePruneCounts {
        provider_tests: provider_before.saturating_sub(provider_tests.len()),
        pending_a2a_agents: pending_before.saturating_sub(pending_a2a_agents.len()),
    }
}

/// Whether the current API principal may access an agent-scoped resource.
///
/// Admin and Owner roles can inspect every agent.
/// Lower roles are limited to agents whose manifest author matches their authenticated name.
/// `None` remains allowed for the explicitly trusted loopback/no-auth deployment mode, matching the existing API compatibility contract.
///
/// An agent that is not in the registry is denied to every principal, including Admin: an agent-scoped resource addressed by an id that resolves to nothing is reported as 404 rather than served, so id enumeration cannot distinguish "exists but not yours" from "does not exist".
pub(crate) fn can_access_agent(
    state: &AppState,
    agent_id: librefang_types::agent::AgentId,
    api_user: Option<&axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> bool {
    let Some(entry) = state.kernel.agent_registry().get(agent_id) else {
        return false;
    };
    let Some(user) = api_user else {
        return true;
    };
    if user.0.role >= crate::middleware::UserRole::Admin {
        return true;
    }
    entry.manifest.author.eq_ignore_ascii_case(&user.0.name)
}

/// Shared application state.
///
/// `kernel` is `Arc<dyn KernelApi>` (#3566) — method calls into the kernel
/// go through the [`KernelApi`] trait rather than the concrete
/// `LibreFangKernel` struct, so adding a method to that struct does not
/// silently widen what the HTTP layer can reach.
///
/// The boundary is narrower than "single contract", though: several
/// `KernelApi` accessors hand back concrete runtime types by reference
/// (e.g. `browser()`, `media()`, `processes()`, `tts()`, `web_tools()`
/// return `&librefang_runtime::*`), and other modules in this crate import
/// `librefang_kernel::*` module-level types directly (triggers, trajectory,
/// errors, auth, config, agent_loop, …). So the trait governs kernel method
/// dispatch, but the API crate still depends on runtime / kernel concrete
/// types in places — the contract is not a hermetic seal over the whole
/// kernel surface.
pub struct AppState {
    pub kernel: Arc<dyn KernelApi>,
    pub started_at: Instant,
    /// Whether a working embedding driver was a *requirement* of the config
    /// this process booted with — snapshotted at boot, deliberately not read
    /// live (#6633).
    ///
    /// `GET /api/ready` compares this against `kernel.embedding()`, which
    /// reflects the driver built once during boot. Reading the requirement
    /// from the live `config_ref()` instead would compare two different points
    /// in time: `POST /api/config/reload` swaps the whole `KernelConfig` when
    /// the plan carries any hot action, so an edit that adds
    /// `memory.embedding_provider` alongside any hot-reloadable field lands a
    /// new requirement against a driver that is never rebuilt (a `[memory]`
    /// change is `restart_required`). Readiness would then report 503 forever
    /// while the daemon is perfectly able to serve traffic, and Kubernetes
    /// would hold the pod out of Service endpoints with no automatic recovery
    /// — turning a config-file edit into an outage.
    ///
    /// Snapshotting keeps the probe an answer about *this* process.
    pub readiness_requires_embedding: bool,
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
    pub provider_test_cache: DashMap<String, ProviderTestResult>,
    /// Webhook subscription store for outbound event notifications.
    pub webhook_store: crate::webhook_store::WebhookStore,
    /// Active session tokens issued by dashboard login.
    /// Maps token string -> SessionToken (with creation timestamp for expiry checks).
    pub active_sessions:
        Arc<tokio::sync::RwLock<HashMap<String, crate::password_hash::SessionToken>>>,
    /// Shared api_key_lock from the auth middleware — updated on password/api_key change
    /// so the new credentials take effect immediately without restart.
    pub api_key_lock: Arc<tokio::sync::RwLock<String>>,
    /// Shared master-credential state from the auth middleware (#6613): the
    /// resolved plaintext `api_key` and its `api_key_hash`. Same Arc the
    /// middleware verifies against, so a config reload or credential change
    /// takes effect on the next request. Always refreshed together with
    /// `api_key_lock` via `crate::server::refresh_master_credential`.
    pub master_key: Arc<crate::middleware::MasterKeyState>,
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
    // NOTE: taking this lock is NOT the same as being allowed to write.
    // Managed mode (#6695) is enforced by `guard_config_write` below, which every config-persisting handler must call before it starts building a new file.
    /// Pending A2A agents awaiting operator approval (Bug #3786).
    /// Maps discovery URL → pending card plus lease timestamp. Agents here are
    /// NOT trusted yet and cannot receive tasks. Use
    /// POST /api/a2a/agents/{url}/approve to promote them into the kernel's
    /// trusted external-agent list.
    pub pending_a2a_agents: DashMap<String, PendingA2aAgent>,
    /// Per-IP brute-force limiter for authentication endpoints.
    /// Shared between the auth-endpoint middleware layer and the background
    /// prune task so stale entries are reclaimed every 5 minutes.
    pub auth_login_limiter: Arc<crate::rate_limiter::AuthLoginLimiter>,
    /// GCRA rate limiter — shared with the middleware layer so the background GC
    /// task can call `retain_recent()` to evict stale per-IP entries and prevent
    /// the DashMap from growing unbounded over a long-running daemon. See #3668.
    pub gcra_limiter: Arc<KeyedRateLimiter>,
    /// Effective quota captured with `gcra_limiter` at server boot.
    /// A mixed config reload can update `config_ref()` without rebuilding this
    /// restart-required limiter, so status routes must use this applied value.
    pub gcra_tokens_per_minute: u32,
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
    /// Registered passkey (WebAuthn/FIDO2) credentials (#5981). Reuses the
    /// substrate's SQLite connection so passkeys persist across restarts.
    pub passkey_store: Arc<dyn librefang_memory::passkey_store::PasskeyStore + Send + Sync>,
    /// Passkey ceremony engine — `Some` only when `passkey_enabled` is set and
    /// the RP config built successfully at boot; `None` otherwise (the
    /// `/api/auth/passkey/*` routes then answer `503`).
    pub passkey_engine: Option<Arc<crate::passkey::PasskeyEngine>>,
}

/// Refuse a configuration write when the file is owned by the deployment (#6695).
///
/// Returns `Some(response)` — a `423 Locked` carrying a structured body — when [`librefang_kernel::config::ConfigMode::Managed`] is in effect, and `None` when the caller may proceed.
///
/// Every handler that persists into `config.toml` calls this **before** it reads the existing file, so a refused write never opens, truncates, or rewrites anything.
/// Enforcement lives here rather than relying on the mount being read-only: a filesystem `EACCES` surfaces as a 500 with an errno, which tells an operator nothing about *why* the write is not allowed, and it does not apply at all when the deployment leaves the file writable but still expects the manifest to be the source of truth.
///
/// The mode is read from the process environment on every call rather than cached at boot.
/// It costs one `std::env::var`, and it means a mode set by an orchestrator that rewrites the environment mid-life cannot be stale.
///
/// `source` must be the kernel's own [`config_path`](librefang_kernel::LibreFangKernel::config_path) — `state.kernel.config_path()` at every call site.
/// Re-deriving it here would let the refusal name a different file from the one the handler would have written, which is precisely the confusion the `423` body exists to remove (#6695).
pub fn guard_config_write(
    source: &std::path::Path,
) -> Option<(
    axum::http::StatusCode,
    axum::response::Json<serde_json::Value>,
)> {
    let mode = librefang_kernel::config::config_mode();
    if mode.is_writable() {
        return None;
    }

    Some((
        axum::http::StatusCode::LOCKED,
        axum::response::Json(serde_json::json!({
            "ok": false,
            "error": "configuration is managed by the deployment",
            "code": "config_managed",
            "source": source.display().to_string(),
        })),
    ))
}

/// Refuse a write to a resource the deployment's provisioning tree owns (#6695).
///
/// The resource-level counterpart of [`guard_config_write`], and deliberately the same status code and envelope shape so a client can handle both with one branch: `423 Locked` carrying `{ok:false, error, code, kind, name, source}`.
/// The `code` differs — `resource_provisioned` rather than `config_managed` — because the remedy differs: one is fixed by editing `config.toml`, the other by editing a file in the provisioning tree and rolling the daemon.
///
/// Returns `None` for every runtime-created resource, which is the whole point of provisioning being per-resource rather than a global switch: an operator keeps full control of everything the deployment did not declare.
///
/// This is a lock on the resource's *definition*, not on operating it. Suspending, resuming, messaging, resetting a session, or reading anything about a provisioned agent all stay available — the RFC's "operational actions and mutable runtime state remain usable" criterion.
pub fn guard_provisioned_write(
    resource: Option<&librefang_kernel::provisioning::ResourceProvenance>,
) -> Option<(
    axum::http::StatusCode,
    axum::response::Json<serde_json::Value>,
)> {
    let provenance = resource?;
    Some((
        axum::http::StatusCode::LOCKED,
        axum::response::Json(serde_json::json!({
            "ok": false,
            "error": "this resource is provisioned by the deployment",
            "code": "resource_provisioned",
            "kind": provenance.kind.as_str(),
            "name": provenance.name,
            "source": provenance.source,
        })),
    ))
}

/// The `423 Locked` body `guard_config_write` produces, as a ready `Response`.
///
/// Handlers whose error type cannot carry a `Response` (the `PersistError` / `PersistBudgetError` enums are `Clone`-free but also `Debug`-matched in several places) store a unit `Managed` variant and call this at the point of conversion, so there is still exactly one place that decides the status code and the body shape.
pub fn managed_config_response(source: &std::path::Path) -> axum::response::Response {
    use axum::response::IntoResponse;
    match guard_config_write(source) {
        Some(parts) => parts.into_response(),
        // Unreachable in practice: only called from a `Managed` error arm, which is only constructed when the guard fired.
        // Falling back to the same body keeps the wire contract stable if that ever stops holding.
        None => (
            axum::http::StatusCode::LOCKED,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": "configuration is managed by the deployment",
                "code": "config_managed",
                "source": source.display().to_string(),
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_card(name: &str) -> librefang_kernel::a2a::AgentCard {
        librefang_kernel::a2a::AgentCard {
            name: name.to_string(),
            description: String::new(),
            url: format!("https://{name}.example/a2a"),
            version: "1".to_string(),
            capabilities: Default::default(),
            skills: Vec::new(),
            default_input_modes: Vec::new(),
            default_output_modes: Vec::new(),
        }
    }

    #[test]
    fn route_cache_prune_removes_expired_entries_and_keeps_fresh_entries() {
        let provider_tests = DashMap::new();
        provider_tests.insert("stale".to_string(), ProviderTestResult::new(34, false));

        let pending_agents = DashMap::new();
        pending_agents.insert(
            "https://stale.example/a2a".to_string(),
            PendingA2aAgent::new(agent_card("stale")),
        );

        let sweep_at = Instant::now()
            .checked_add(PENDING_A2A_AGENT_TTL + Duration::from_secs(1))
            .expect("24-hour test offset must fit in Instant");
        let mut fresh_provider = ProviderTestResult::new(12, true);
        fresh_provider.tested_at = sweep_at;
        provider_tests.insert("fresh".to_string(), fresh_provider);
        let mut fresh_pending = PendingA2aAgent::new(agent_card("fresh"));
        fresh_pending.discovered_at = sweep_at;
        pending_agents.insert("https://fresh.example/a2a".to_string(), fresh_pending);

        let removed = prune_route_caches_at(&provider_tests, &pending_agents, sweep_at);

        assert_eq!(removed.provider_tests, 1);
        assert_eq!(removed.pending_a2a_agents, 1);
        assert!(provider_tests.contains_key("fresh"));
        assert!(!provider_tests.contains_key("stale"));
        assert!(pending_agents.contains_key("https://fresh.example/a2a"));
        assert!(!pending_agents.contains_key("https://stale.example/a2a"));
    }

    #[test]
    fn named_provider_result_preserves_field_meaning() {
        let result = ProviderTestResult::new(27, false);

        assert_eq!(result.latency_ms, 27);
        assert!(!result.reachable);
        assert!(!result.tested_rfc3339.is_empty());
    }
}
