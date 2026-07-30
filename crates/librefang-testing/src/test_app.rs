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
    ///
    /// Writes both live handles the middleware consults, matching what
    /// `server::refresh_master_credential` does in production — otherwise the
    /// transparent-upgrade path could not tell that the configured token is
    /// the master key.
    pub fn with_api_key(self, key: &str) -> Self {
        *self
            .state
            .api_key_lock
            .try_write()
            .expect("api key lock should be uncontended during test setup") = key.to_string();
        self.state
            .master_key
            .set_blocking(key.to_string(), String::new());
        self
    }

    /// Configures the master API key as an `api_key_hash` with no plaintext,
    /// the posture an operator lands in after the transparent upgrade.
    ///
    /// Clears the plaintext side of **both** live handles as well as setting the
    /// hash, because "hash-only" is the whole point of the fixture: a leftover
    /// plaintext token — from `KernelConfig.api_key`, which `build_state` seeds
    /// the handles from — would let a test authenticate without ever reaching
    /// the hash path it means to exercise.
    ///
    /// Writes the **live handles only**. It does not touch
    /// `KernelConfig.api_key_hash`, so a test that also exercises a path
    /// deriving from `auth_snapshot()` (the boot-time bind guard,
    /// `configured_user_api_keys`, `has_dashboard_credentials`) must set the
    /// config field too:
    ///
    /// ```rust,ignore
    /// let hash = librefang_api::password_hash::hash_device_token("secret-key");
    /// let h = hash.clone();
    /// TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
    ///     cfg.api_key_hash = h;
    /// }))
    /// .with_api_key_hash(&hash);
    /// ```
    ///
    /// Setting only one of the two is the failure mode worth naming: the
    /// handler sees an unconfigured daemon and the test passes for the wrong
    /// reason.
    pub fn with_api_key_hash(self, hash: &str) -> Self {
        *self
            .state
            .api_key_lock
            .try_write()
            .expect("api key lock should be uncontended during test setup") = String::new();
        self.state
            .master_key
            .set_blocking(String::new(), hash.to_string());
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

        // Passkey (#5981) — store always wired; engine built only when the
        // test config opts in via `passkey_enabled`, mirroring production.
        let passkey_store: Arc<dyn librefang_memory::passkey_store::PasskeyStore + Send + Sync> =
            Arc::new(librefang_memory::passkey_store::SqlitePasskeyStore::new(
                kernel.substrate_ref().pool(),
            ));
        let passkey_engine = {
            let cfg = kernel.config_ref();
            if cfg.passkey_enabled {
                librefang_api::passkey::PasskeyEngine::new(
                    &cfg.passkey_rp_id,
                    &cfg.passkey_rp_origin,
                    &cfg.dashboard_user,
                )
                .ok()
                .map(Arc::new)
            } else {
                None
            }
        };

        // Seed the live auth handles from the test's `KernelConfig`, which is
        // what `server::refresh_master_credential` does at boot in production
        // (#6613). Without this a test that configures `cfg.api_key` or
        // `cfg.api_key_hash` through `MockKernelBuilder::with_config` leaves
        // both handles empty, and every surface that reads them — the auth
        // middleware, the WS and terminal upgrades, `security_status`,
        // `dashboard_auth_check` — sees an unconfigured daemon while
        // `auth_snapshot()` reports a configured one. That split is exactly the
        // class of disagreement #6613 was about, so the harness must not
        // reproduce it.
        //
        // The env / `vault:` indirection is deliberately NOT applied here: a
        // fixture's credential is whatever its config literally says, and
        // resolving would make the harness depend on the developer's
        // environment and on-disk vault. `with_api_key` / `with_api_key_hash`
        // still override both handles afterwards for tests that want a
        // credential the config does not mention.
        //
        // One `config_ref()` for both fields, so the pair comes from a single
        // generation exactly as `refresh_master_credential` takes them from a
        // single `ApiAuthSnapshot`.
        let (master_plaintext, master_hash) = {
            let cfg = kernel.config_ref();
            (
                cfg.api_key.trim().to_string(),
                cfg.api_key_hash.trim().to_string(),
            )
        };
        // Rooted at the test's temp home so a transparent api_key upgrade hint
        // (#6613) lands there rather than in the process CWD.
        let master_key = Arc::new(librefang_api::middleware::MasterKeyState::new(
            tmp.path().to_path_buf(),
        ));
        master_key.set_blocking(master_plaintext.clone(), master_hash);

        Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            // The mock kernel pins no embedding provider, so readiness never
            // depends on one. Production computes this from the booted config
            // in `server::build_router`.
            readiness_requires_embedding: false,
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
            api_key_lock: Arc::new(tokio::sync::RwLock::new(master_plaintext)),
            master_key,
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
            passkey_store,
            passkey_engine,
        })
    }
}

impl Default for TestAppState {
    fn default() -> Self {
        Self::new()
    }
}
