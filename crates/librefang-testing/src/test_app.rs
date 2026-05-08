//! TestAppState — Builds an `AppState` and `Router` suitable for axum route testing.
//!
//! Wraps the output of `MockKernelBuilder` and provides quick construction of test routers.

use crate::mock_kernel::MockKernelBuilder;
use axum::Router;
use librefang_api::middleware::ApiUserAuth;
use librefang_api::routes::AppState;
use librefang_kernel::LibreFangKernel;
use librefang_kernel::MemorySubsystemApi;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

/// Test AppState builder.
///
/// # Example
///
/// ```rust,ignore
/// // ignore: requires full kernel boot environment (temp directory, SQLite), see integration tests in tests.rs
/// use librefang_testing::TestAppState;
///
/// let test = TestAppState::new();
/// let router = test.router();
/// // Now you can use tower::ServiceExt to send test requests
/// ```
pub struct TestAppState {
    /// Shared AppState (same type as production).
    pub state: Arc<AppState>,
    /// Temp directory — must hold the reference, otherwise the directory will be deleted.
    _tmp: TempDir,
    /// Optional path to a config TOML file written to disk (for config-reload tests).
    _config_path: Option<PathBuf>,
}

impl TestAppState {
    /// Creates a TestAppState using the default mock kernel.
    pub fn new() -> Self {
        Self::with_builder(MockKernelBuilder::new())
    }

    /// Creates a TestAppState using a custom MockKernelBuilder.
    pub fn with_builder(builder: MockKernelBuilder) -> Self {
        let (kernel, tmp) = builder.build();
        let state = Self::build_state(kernel, &tmp);
        Self {
            state,
            _tmp: tmp,
            _config_path: None,
        }
    }

    /// Builds from an existing kernel (caller is responsible for holding TempDir).
    ///
    /// Wraps the kernel in `Arc` and wires `set_self_handle` so internal
    /// `kernel_handle()` lookups (used by `send_message_*`) succeed (#3652).
    pub fn from_kernel(kernel: LibreFangKernel, tmp: TempDir) -> Self {
        let kernel = Arc::new(kernel);
        kernel.set_self_handle();
        let state = Self::build_state(kernel, &tmp);
        Self {
            state,
            _tmp: tmp,
            _config_path: None,
        }
    }

    /// Builds an axum Router with common API routes (suitable for testing).
    ///
    /// The returned Router is nested under the `/api` path, matching the production setup.
    /// Covers agents CRUD, skills, config, memory, budget, system, and other main endpoints.
    pub fn router(&self) -> Router {
        use axum::routing::{get, post, put};
        use librefang_api::routes;

        let api = Router::new()
            // -- System endpoints --
            .route("/health", get(routes::health))
            .route("/health/detail", get(routes::health_detail))
            .route("/status", get(routes::status))
            .route("/version", get(routes::version))
            .route("/metrics", get(routes::prometheus_metrics))
            // -- Agents CRUD --
            .route("/agents", get(routes::list_agents).post(routes::spawn_agent))
            .route(
                "/agents/{id}",
                get(routes::get_agent)
                    .delete(routes::kill_agent)
                    .patch(routes::patch_agent),
            )
            .route("/agents/{id}/message", post(routes::send_message))
            .route("/agents/{id}/stop", post(routes::stop_agent))
            .route("/agents/{id}/model", put(routes::set_model))
            .route("/agents/{id}/mode", put(routes::set_agent_mode))
            .route("/agents/{id}/session", get(routes::get_agent_session))
            .route(
                "/agents/{id}/sessions",
                get(routes::list_agent_sessions).post(routes::create_agent_session),
            )
            .route("/agents/{id}/session/reset", post(routes::reset_session))
            .route("/agents/{id}/tools", get(routes::get_agent_tools).put(routes::set_agent_tools))
            .route("/agents/{id}/skills", get(routes::get_agent_skills).put(routes::set_agent_skills))
            .route("/agents/{id}/logs", get(routes::agent_logs))
            // -- Profiles --
            .route("/profiles", get(routes::list_profiles))
            .route("/profiles/{name}", get(routes::get_profile))
            // -- Skills --
            .route("/skills", get(routes::list_skills))
            .route("/skills/create", post(routes::create_skill))
            // -- Config --
            .route("/config", get(routes::get_config))
            .route("/config/schema", get(routes::config_schema))
            .route("/config/set", post(routes::config_set))
            .route("/config/reload", post(routes::config_reload))
            // -- Memory --
            .route("/memory/search", get(routes::memory_search))
            .route("/memory/stats", get(routes::memory_stats))
            // -- Budget / Usage --
            .route("/usage", get(routes::usage_stats))
            .route("/usage/summary", get(routes::usage_summary))
            // -- Tools & Commands --
            .route("/tools", get(routes::list_tools))
            .route("/tools/{name}", get(routes::get_tool))
            .route("/commands", get(routes::list_commands))
            // -- Models & Providers --
            .route("/models", get(routes::list_models))
            .route("/providers", get(routes::list_providers))
            // -- Sessions --
            .route("/sessions", get(routes::list_sessions));

        Router::new()
            .nest("/api", api)
            .with_state(self.state.clone())
    }

    /// Returns the path to the temporary directory.
    pub fn tmp_path(&self) -> &std::path::Path {
        self._tmp.path()
    }

    /// Returns an Arc reference to the AppState.
    pub fn app_state(&self) -> Arc<AppState> {
        self.state.clone()
    }

    /// Sets the global API key so auth middleware accepts it.
    pub fn with_api_key(self, key: &str) -> Self {
        *self
            .state
            .api_key_lock
            .try_write()
            .expect("api key lock should be uncontended during test setup") = key.to_string();
        self
    }

    /// Pre-populates the per-user API key list for auth middleware.
    pub fn with_user_api_keys(self, keys: Vec<ApiUserAuth>) -> Self {
        *self
            .state
            .user_api_keys
            .try_write()
            .expect("user API key lock should be uncontended during test setup") = keys;
        self
    }

    /// Serializes the kernel config to a TOML file at `path`.
    ///
    /// Useful for tests that exercise config-reload endpoints which read
    /// from disk.
    ///
    /// Note: this snapshots the kernel's internal `KernelConfig` only.
    /// Values set via [`with_api_key`](Self::with_api_key) /
    /// [`with_user_api_keys`](Self::with_user_api_keys) live on the
    /// `AppState` runtime locks and are NOT written to disk — bake them
    /// into the kernel config via `MockKernelBuilder::with_config` if
    /// the test reloads from this file.
    pub fn with_config_path(mut self, path: PathBuf) -> Self {
        let config_str =
            toml::to_string_pretty(&*self.state.kernel.config_ref()).expect("serialize config");
        std::fs::write(&path, config_str).expect("write config file");
        self._config_path = Some(path);
        self
    }

    /// Consumes `TestAppState`, returning the components a test may need
    /// to hold onto directly.
    pub fn into_parts(self) -> (Arc<AppState>, TempDir, Option<PathBuf>) {
        (self.state, self._tmp, self._config_path)
    }

    /// Internal: builds AppState from a kernel.
    fn build_state(kernel: Arc<LibreFangKernel>, tmp: &TempDir) -> Arc<AppState> {
        let channels_config = kernel.config_ref().channels.clone();

        // Idempotency-Key replay store (#3637) — wired against the
        // substrate's shared SQLite connection so tests exercise the
        // same persistence path as production.
        let idempotency_store: Arc<
            dyn librefang_memory::idempotency::IdempotencyStore + Send + Sync,
        > = Arc::new(librefang_memory::idempotency::SqliteIdempotencyStore::new(
            kernel.substrate_ref().pool(),
        ));

        Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            bridge_manager: arc_swap::ArcSwap::new(std::sync::Arc::new(None)),
            channels_config: tokio::sync::RwLock::new(channels_config),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            skillhub_cache: dashmap::DashMap::new(),
            provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
            provider_test_cache: dashmap::DashMap::new(),
            webhook_store: librefang_api::webhook_store::WebhookStore::load(
                tmp.path().join("test_webhooks.json"),
            ),
            active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            config_write_lock: tokio::sync::Mutex::new(()),
            pending_a2a_agents: dashmap::DashMap::new(),
            auth_login_limiter: std::sync::Arc::new(
                librefang_api::rate_limiter::AuthLoginLimiter::new(),
            ),
            gcra_limiter: librefang_api::rate_limiter::create_rate_limiter(0),
            // Tests run with header-trust off (the production default) so
            // per-IP rate-limiter / WS slot keying always uses the TCP peer.
            trusted_proxies: Arc::new(librefang_api::client_ip::TrustedProxies::default()),
            trust_forwarded_for: false,
            idempotency_store,
        })
    }
}

impl Default for TestAppState {
    fn default() -> Self {
        Self::new()
    }
}
