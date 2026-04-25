//! TestAppState — Builds an `AppState` and `Router` suitable for axum route testing.
//!
//! Wraps the output of `MockKernelBuilder` and provides quick construction of test routers.

use crate::mock_kernel::MockKernelBuilder;
use axum::Router;
use librefang_api::routes::AppState;
use librefang_kernel::LibreFangKernel;
use std::path::Path;
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
        Self { state, _tmp: tmp }
    }

    /// Builds from an existing kernel (caller is responsible for holding TempDir).
    pub fn from_kernel(kernel: LibreFangKernel, tmp: TempDir) -> Self {
        let state = Self::build_state(kernel, &tmp);
        Self { state, _tmp: tmp }
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
            .route("/agents/{id}/clone", post(routes::clone_agent))
            // -- Profiles --
            .route("/profiles", get(routes::list_profiles))
            .route("/profiles/{name}", get(routes::get_profile))
            // -- Skills --
            .route("/skills", get(routes::list_skills))
            .route("/skills/registry", get(routes::list_skill_registry))
            .route("/skills/install", post(routes::install_skill))
            .route("/skills/uninstall", post(routes::uninstall_skill))
            .route("/skills/reload", post(routes::reload_skills))
            .route("/skills/create", post(routes::create_skill))
            .route("/skills/{name}", get(routes::get_skill_detail))
            // -- Workflows --
            .route("/workflows", get(routes::list_workflows).post(routes::create_workflow))
            .route("/workflows/{id}", get(routes::get_workflow))
            .route("/workflows/{id}/dry-run", post(routes::dry_run_workflow))
            .route("/workflows/{id}/run", post(routes::run_workflow))
            .route("/workflows/{id}/runs", get(routes::list_workflow_runs))
            .route("/workflows/runs/{run_id}", get(routes::get_workflow_run))
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
            .route("/sessions", get(routes::list_sessions))
            // -- Audit --
            .route("/audit/recent", get(routes::audit_recent))
            .route("/audit/verify", get(routes::audit_verify));

        Router::new()
            .nest("/api", api)
            .with_state(self.state.clone())
    }

    /// Returns an Arc reference to the AppState.
    pub fn app_state(&self) -> Arc<AppState> {
        self.state.clone()
    }

    /// Returns the temp LibreFang home directory used by this test app.
    pub fn home_dir(&self) -> &Path {
        self._tmp.path()
    }

    /// Internal: builds AppState from a kernel.
    fn build_state(kernel: LibreFangKernel, tmp: &TempDir) -> Arc<AppState> {
        let kernel = Arc::new(kernel);
        let channels_config = kernel.config_ref().channels.clone();

        Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
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
            prometheus_handle: None,
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(axum::Router::new()))),
            config_write_lock: tokio::sync::Mutex::new(()),
        })
    }
}

impl Default for TestAppState {
    fn default() -> Self {
        Self::new()
    }
}
