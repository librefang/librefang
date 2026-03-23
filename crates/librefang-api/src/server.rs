//! LibreFang daemon server — boots the kernel and serves the HTTP API.

use crate::channel_bridge;
use crate::middleware;
use crate::rate_limiter;
use crate::routes::{self, AppState};
use crate::webchat;
use axum::response::IntoResponse;
use axum::Router;
use librefang_kernel::LibreFangKernel;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Daemon info written to `~/.librefang/daemon.json` so the CLI can find us.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub listen_addr: String,
    pub started_at: String,
    pub version: String,
    pub platform: String,
}

/// Current API version. Bump when introducing a new version.
pub const API_VERSION_LATEST: &str = crate::versioning::CURRENT_VERSION;

/// All available API versions with their status.
pub const API_VERSIONS: &[(&str, &str)] = &[("v1", "stable")];

/// Build the v1 API route tree.
///
/// Each domain sub-module provides its own `router()` method, combined here via `.merge()`.
/// Paths are relative to the mount point (e.g. `/health`, `/agents`, etc.); the caller
/// nests them under `/api` and `/api/v1`.
///
/// To add v2 in the future, just create `api_v2_routes()`, mount it at `/api/v2`,
/// and update `API_VERSION_LATEST`.
fn api_v1_routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(routes::config::router())
        .merge(routes::agents::router())
        .merge(routes::channels::router())
        .merge(routes::system::router())
        .merge(routes::memory::router())
        .merge(routes::workflows::router())
        .merge(routes::skills::router())
        .merge(routes::network::router())
        .merge(routes::plugins::router())
        .merge(routes::providers::router())
        .merge(routes::budget::router())
        .merge(routes::goals::router())
        .merge(routes::inbox::router())
        .merge(routes::media::router())
        // Dashboard credential login (handler defined locally in server.rs)
        .route(
            "/auth/dashboard-login",
            axum::routing::post(dashboard_login),
        )
        .route(
            "/auth/dashboard-check",
            axum::routing::get(dashboard_auth_check),
        )
        // OAuth/OIDC external authentication endpoints
        .route(
            "/auth/providers",
            axum::routing::get(crate::oauth::auth_providers),
        )
        .route("/auth/login", axum::routing::get(crate::oauth::auth_login))
        .route(
            "/auth/login/{provider}",
            axum::routing::get(crate::oauth::auth_login_provider),
        )
        .route(
            "/auth/callback",
            axum::routing::get(crate::oauth::auth_callback).post(crate::oauth::auth_callback_post),
        )
        .route(
            "/auth/userinfo",
            axum::routing::get(crate::oauth::auth_userinfo),
        )
        .route(
            "/auth/introspect",
            axum::routing::post(crate::oauth::auth_introspect),
        )
}

/// Resolve a dashboard credential from: 1) env var, 2) vault:KEY syntax, 3) literal value.
fn resolve_dashboard_credential(
    config_value: &str,
    env_var: &str,
    home_dir: &std::path::Path,
) -> String {
    // 1. Environment variable takes priority
    if let Ok(val) = std::env::var(env_var) {
        if !val.trim().is_empty() {
            return val;
        }
    }

    let val = config_value.trim();

    // 2. vault:KEY_NAME syntax — read from encrypted vault
    if let Some(vault_key) = val.strip_prefix("vault:") {
        let vault_path = home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        match vault.unlock() {
            Ok(()) => {
                if let Some(secret) = vault.get(vault_key) {
                    return secret.to_string();
                }
                tracing::warn!("Vault key '{vault_key}' not found in vault");
            }
            Err(e) => {
                tracing::warn!("Could not unlock vault for dashboard credential: {e}");
            }
        }
        return String::new();
    }

    // 3. Literal value from config
    config_value.to_string()
}

#[allow(deprecated)]
pub(crate) fn dashboard_session_token(kernel: &LibreFangKernel) -> Option<String> {
    let cfg = kernel.config_ref();
    let username = resolve_dashboard_credential(
        &cfg.dashboard_user,
        "LIBREFANG_DASHBOARD_USER",
        kernel.home_dir(),
    );
    let password = resolve_dashboard_credential(
        &cfg.dashboard_pass,
        "LIBREFANG_DASHBOARD_PASS",
        kernel.home_dir(),
    );

    crate::password_hash::derive_dashboard_session_token(
        username.trim(),
        password.trim(),
        cfg.dashboard_pass_hash.trim(),
    )
}

pub(crate) fn valid_api_tokens(kernel: &LibreFangKernel) -> Vec<String> {
    let mut tokens = Vec::new();
    let explicit_api_key = kernel.config_ref().api_key.trim();
    if !explicit_api_key.is_empty() {
        tokens.push(explicit_api_key.to_string());
    }
    if let Some(token) = dashboard_session_token(kernel) {
        tokens.push(token);
    }
    tokens
}

/// Dashboard credential login — validates username/password using Argon2id
/// (with transparent fallback from legacy plaintext passwords) and returns
/// a randomly generated session token with expiration metadata.
async fn dashboard_login(
    axum::extract::State(state): axum::extract::State<Arc<routes::AppState>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let cfg = &state.kernel.config_ref();
    let cfg_user = resolve_dashboard_credential(
        &cfg.dashboard_user,
        "LIBREFANG_DASHBOARD_USER",
        &cfg.home_dir,
    );
    let cfg_user = cfg_user.trim().to_string();
    let cfg_pass = resolve_dashboard_credential(
        &cfg.dashboard_pass,
        "LIBREFANG_DASHBOARD_PASS",
        &cfg.home_dir,
    );
    let cfg_pass = cfg_pass.trim().to_string();
    let pass_hash = cfg.dashboard_pass_hash.trim();

    // If not configured, login is not needed
    let has_password = !pass_hash.is_empty() || !cfg_pass.is_empty();
    if cfg_user.is_empty() || !has_password {
        return axum::response::Json(serde_json::json!({
            "ok": true, "token": "", "message": "No credentials required"
        }))
        .into_response();
    }

    let user = body.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let pass = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    match crate::password_hash::verify_dashboard_password(
        user, pass, &cfg_user, &cfg_pass, pass_hash,
    ) {
        crate::password_hash::VerifyResult::Ok {
            token,
            upgrade_hash,
        } => {
            // If we successfully verified via legacy plaintext, log that an
            // upgrade hash is available. The admin can persist it to config.
            if let Some(ref hash) = upgrade_hash {
                tracing::info!(
                    "Dashboard password verified via legacy plaintext. \
                     Set `dashboard_pass_hash = \"{}\"` in config.toml \
                     and remove `dashboard_pass` to complete the migration.",
                    hash
                );
            }

            // Store the session token so the auth middleware can validate it.
            state
                .active_sessions
                .write()
                .await
                .insert(token.token.clone(), token.clone());

            axum::response::Json(serde_json::json!({
                "ok": true,
                "token": token.token,
                "created_at": token.created_at,
                "expires_at": token.created_at + crate::password_hash::DEFAULT_SESSION_TTL_SECS,
            }))
            .into_response()
        }
        crate::password_hash::VerifyResult::Denied => (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": "Invalid username or password"
            })),
        )
            .into_response(),
    }
}

/// Check what auth mode the dashboard needs.
async fn dashboard_auth_check(
    axum::extract::State(state): axum::extract::State<Arc<routes::AppState>>,
) -> axum::response::Json<serde_json::Value> {
    let cfg = &state.kernel.config_ref();
    let du = resolve_dashboard_credential(
        &cfg.dashboard_user,
        "LIBREFANG_DASHBOARD_USER",
        &cfg.home_dir,
    );
    let dp = resolve_dashboard_credential(
        &cfg.dashboard_pass,
        "LIBREFANG_DASHBOARD_PASS",
        &cfg.home_dir,
    );
    let has_pass_hash = !cfg.dashboard_pass_hash.trim().is_empty();
    let has_credentials = !du.trim().is_empty() && (has_pass_hash || !dp.trim().is_empty());
    let has_api_key = !cfg.api_key.trim().is_empty();

    axum::response::Json(serde_json::json!({
        "mode": if has_credentials { "credentials" } else if has_api_key { "api_key" } else { "none" },
    }))
}

/// Build the full API router with all routes, middleware, and state.
///
/// This is extracted from `run_daemon()` so that embedders (e.g. librefang-desktop)
/// can create the router without starting the full daemon lifecycle.
///
/// Returns `(router, shared_state)`. The caller can use `state.bridge_manager`
/// to shut down the bridge on exit.
pub async fn build_router(
    kernel: Arc<LibreFangKernel>,
    listen_addr: SocketAddr,
) -> (Router<()>, Arc<AppState>) {
    // Start channel bridges (Telegram, etc.)
    let bridge = channel_bridge::start_channel_bridge(kernel.clone()).await;

    // Initialize Prometheus metrics recorder if telemetry feature is enabled
    // and the config has prometheus_enabled = true.
    #[cfg(feature = "telemetry")]
    let prom_handle = if kernel.config_ref().telemetry.prometheus_enabled {
        info!("Initializing Prometheus metrics recorder");
        Some(crate::telemetry::init_prometheus())
    } else {
        None
    };

    let channels_config = kernel.config_ref().channels.clone();
    let active_sessions = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let state = Arc::new(AppState {
        kernel: kernel.clone(),
        started_at: Instant::now(),
        peer_registry: kernel.peer_registry_ref().map(|r| Arc::new(r.clone())),
        bridge_manager: tokio::sync::Mutex::new(bridge),
        channels_config: tokio::sync::RwLock::new(channels_config),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        skillhub_cache: dashmap::DashMap::new(),
        provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
        webhook_store: crate::webhook_store::WebhookStore::load(
            kernel.config_ref().home_dir.join("webhooks.json"),
        ),
        active_sessions: active_sessions.clone(),
        media_drivers: librefang_runtime::media::MediaDriverCache::new(),
        #[cfg(feature = "telemetry")]
        prometheus_handle: prom_handle,
    });

    // CORS: allow localhost origins by default, plus any configured in cors_origin.
    let cors = {
        let port = listen_addr.port();
        let mut origins: Vec<axum::http::HeaderValue> = vec![
            format!("http://{listen_addr}").parse().unwrap(),
            format!("http://localhost:{port}").parse().unwrap(),
            format!("http://127.0.0.1:{port}").parse().unwrap(),
        ];
        // Also allow common dev ports
        for p in [3000u16, 8080] {
            if p != port {
                if let Ok(v) = format!("http://127.0.0.1:{p}").parse() {
                    origins.push(v);
                }
                if let Ok(v) = format!("http://localhost:{p}").parse() {
                    origins.push(v);
                }
            }
        }
        // Add explicitly configured CORS origins from config.toml
        for origin in &state.kernel.config_ref().cors_origin {
            if let Ok(v) = origin.parse::<axum::http::HeaderValue>() {
                origins.push(v);
            } else {
                tracing::warn!("Invalid CORS origin in config, skipping: {origin}");
            }
        }
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    };

    // Middleware accepts any token in this composite key.
    let api_key = valid_api_tokens(state.kernel.as_ref()).join("\n");
    let api_key_lock = Arc::new(tokio::sync::RwLock::new(api_key));
    let auth_state = middleware::AuthState {
        api_key_lock: api_key_lock.clone(),
        active_sessions: active_sessions.clone(),
    };
    let gcra_limiter = rate_limiter::create_rate_limiter();

    // Build the versioned API routes. All /api/* endpoints are defined once
    // in api_v1_routes() and mounted at both /api and /api/v1 for backward
    // compatibility. Future versions (v2, v3) can be added as separate routers.
    let v1_routes = api_v1_routes();

    let app = Router::new()
        .route("/", axum::routing::get(webchat::webchat_page))
        .route(
            "/dashboard/{*path}",
            axum::routing::get(webchat::react_asset),
        )
        .route("/logo.png", axum::routing::get(webchat::logo_png))
        .route("/favicon.ico", axum::routing::get(webchat::favicon_ico))
        .route("/locales/en.json", axum::routing::get(webchat::locale_en))
        .route("/locales/ja.json", axum::routing::get(webchat::locale_ja))
        .route(
            "/locales/zh-CN.json",
            axum::routing::get(webchat::locale_zh_cn),
        )
        // API version discovery endpoint (not versioned itself)
        .route("/api/versions", axum::routing::get(routes::api_versions))
        // Auto-generated OpenAPI specification
        .route(
            "/api/openapi.json",
            axum::routing::get(crate::openapi::openapi_spec),
        )
        // Mount v1 routes at /api/v1 (explicit version)
        .nest("/api/v1", v1_routes.clone())
        // Mount the same routes at /api (latest version alias for backward compat)
        .nest("/api", v1_routes)
        // Webhook trigger endpoints (not versioned — external callers use fixed URLs)
        .route("/hooks/wake", axum::routing::post(routes::webhook_wake))
        .route("/hooks/agent", axum::routing::post(routes::webhook_agent))
        // A2A protocol endpoints + MCP HTTP (protocol-level, not versioned)
        .merge(routes::network::protocol_router())
        // MCP HTTP endpoint (protocol-level, not versioned)
        .route("/mcp", axum::routing::post(routes::mcp_http))
        // OpenAI-compatible API (follows OpenAI versioning, not ours)
        .route(
            "/v1/chat/completions",
            axum::routing::post(crate::openai_compat::chat_completions),
        )
        .route(
            "/v1/models",
            axum::routing::get(crate::openai_compat::list_models),
        )
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .layer(axum::middleware::from_fn(middleware::accept_language))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::oauth::oidc_auth_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            gcra_limiter,
            rate_limiter::gcra_rate_limit,
        ))
        .layer(axum::middleware::from_fn(middleware::api_version_headers))
        .layer(axum::middleware::from_fn(middleware::security_headers))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(RequestBodyLimitLayer::new(
            crate::validation::MAX_REQUEST_BODY_BYTES,
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    // Add HTTP metrics middleware when telemetry feature is enabled and Prometheus is active.
    #[cfg(feature = "telemetry")]
    let app = if state.prometheus_handle.is_some() {
        app.layer(axum::middleware::from_fn(
            crate::telemetry::http_metrics_middleware,
        ))
    } else {
        app
    };

    let app = app.with_state(state.clone());

    (app, state)
}

/// Start the LibreFang daemon: boot kernel + HTTP API server.
///
/// This function blocks until Ctrl+C or a shutdown request.
pub async fn run_daemon(
    kernel: LibreFangKernel,
    listen_addr: &str,
    daemon_info_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = listen_addr.parse()?;

    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    kernel.start_background_agents().await;

    // Config file hot-reload watcher (polls every 30 seconds)
    {
        let k = kernel.clone();
        let config_path = kernel.config_ref().home_dir.join("config.toml");
        tokio::spawn(async move {
            let mut last_modified = std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .ok();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let current = std::fs::metadata(&config_path)
                    .and_then(|m| m.modified())
                    .ok();
                if current != last_modified && current.is_some() {
                    last_modified = current;
                    tracing::info!("Config file changed, reloading...");
                    match k.reload_config() {
                        Ok(plan) => {
                            if plan.has_changes() {
                                tracing::info!("Config hot-reload applied: {:?}", plan.hot_actions);
                            } else {
                                tracing::debug!("Config hot-reload: no actionable changes");
                            }
                        }
                        Err(e) => tracing::warn!("Config hot-reload failed: {e}"),
                    }
                }
            }
        });
    }

    let (app, state) = build_router(kernel.clone(), addr).await;

    // Write daemon info file
    if let Some(info_path) = daemon_info_path {
        // Check if another daemon is already running with this PID file
        if info_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(info_path) {
                if let Ok(info) = serde_json::from_str::<DaemonInfo>(&existing) {
                    // PID alive AND the health endpoint responds → truly running
                    if is_process_alive(info.pid) && is_daemon_responding(&info.listen_addr) {
                        return Err(format!(
                            "Another daemon (PID {}) is already running at {}",
                            info.pid, info.listen_addr
                        )
                        .into());
                    }
                }
            }
            // Stale PID file (process dead or different process reused PID), remove it
            info!("Removing stale daemon info file");
            let _ = std::fs::remove_file(info_path);
        }

        let daemon_info = DaemonInfo {
            pid: std::process::id(),
            listen_addr: addr.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&daemon_info) {
            let _ = std::fs::write(info_path, json);
            // SECURITY: Restrict daemon info file permissions (contains PID and port).
            restrict_permissions(info_path);
        }
    }

    info!(
        "LibreFang v{} ({}) built {} [{}]",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_SHA"),
        env!("BUILD_DATE"),
        std::env::consts::ARCH,
    );
    info!("LibreFang API server listening on http://{addr}");
    info!("WebChat UI available at http://{addr}/",);
    info!("WebSocket endpoint: ws://{addr}/api/agents/{{id}}/ws",);

    // Auto-start observability stack (Prometheus + Grafana) if Docker is available
    let observability_started = if kernel.config_ref().telemetry.enabled {
        match start_observability_stack() {
            Ok(true) => {
                info!("Observability stack started (Prometheus :9090, Grafana :3000)");
                true
            }
            Ok(false) => {
                info!("Docker not available, skipping observability stack");
                false
            }
            Err(e) => {
                tracing::warn!("Failed to start observability stack: {e}");
                false
            }
        }
    } else {
        false
    };

    // Background: sync model catalog from community repo on startup, then every 24 hours
    {
        let kernel = state.kernel.clone();
        tokio::spawn(async move {
            loop {
                match librefang_runtime::catalog_sync::sync_catalog_to(
                    &kernel.config_ref().home_dir,
                )
                .await
                {
                    Ok(result) => {
                        info!(
                            "Model catalog synced: {} files downloaded",
                            result.files_downloaded
                        );
                        if let Ok(mut catalog) = kernel.model_catalog_ref().write() {
                            catalog.load_cached_catalog_for(&kernel.config_ref().home_dir);
                            catalog.detect_auth();
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Background catalog sync failed (will use cached/builtin): {e}"
                        );
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
            }
        });
    }

    // Use SO_REUSEADDR to allow binding immediately after reboot (avoids TIME_WAIT).
    let socket = socket2::Socket::new(
        if addr.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        },
        socket2::Type::STREAM,
        None,
    )?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    let listener = tokio::net::TcpListener::from_std(std::net::TcpListener::from(socket))?;

    // Run server with graceful shutdown.
    // SECURITY: `into_make_service_with_connect_info` injects the peer
    // SocketAddr so the auth middleware can check for loopback connections.
    let api_shutdown = state.shutdown_notify.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(api_shutdown))
    .await?;

    // Clean up daemon info file
    if let Some(info_path) = daemon_info_path {
        let _ = std::fs::remove_file(info_path);
    }

    // Stop channel bridges
    if let Some(ref mut b) = *state.bridge_manager.lock().await {
        b.stop().await;
    }

    // Stop observability stack
    if observability_started {
        if let Err(e) = stop_observability_stack() {
            tracing::warn!("Failed to stop observability stack: {e}");
        } else {
            info!("Observability stack stopped");
        }
    }

    // Shutdown kernel
    kernel.shutdown();

    info!("LibreFang daemon stopped");
    Ok(())
}

/// Check if Docker is available and start the observability stack.
/// Returns Ok(true) if started, Ok(false) if Docker not available.
fn start_observability_stack() -> Result<bool, Box<dyn std::error::Error>> {
    // Check if docker CLI exists
    let docker_check = std::process::Command::new("docker")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match docker_check {
        Ok(status) if status.success() => {}
        _ => return Ok(false),
    }

    // Find the compose file relative to the executable or well-known paths
    let compose_file = find_compose_file()?;

    std::process::Command::new("docker")
        .args(["compose", "-f"])
        .arg(&compose_file)
        .args(["up", "-d"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("docker compose up failed: {e}"))?;

    Ok(true)
}

/// Stop the observability stack.
fn stop_observability_stack() -> Result<(), Box<dyn std::error::Error>> {
    let compose_file = find_compose_file()?;

    std::process::Command::new("docker")
        .args(["compose", "-f"])
        .arg(&compose_file)
        .args(["down"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("docker compose down failed: {e}"))?;

    Ok(())
}

/// Locate the observability docker-compose file.
fn find_compose_file() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    // Try relative to current exe
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Binary might be in target/release or target/debug
            for ancestor in dir.ancestors().take(4) {
                let candidate = ancestor.join("deploy/docker-compose.observability.yml");
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
    }

    // Try current working directory
    let cwd_candidate = std::path::PathBuf::from("deploy/docker-compose.observability.yml");
    if cwd_candidate.exists() {
        return Ok(cwd_candidate);
    }

    Err("Could not find deploy/docker-compose.observability.yml".into())
}

/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
/// On non-Unix platforms this is a no-op.
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

/// Read daemon info from the standard location.
pub fn read_daemon_info(home_dir: &Path) -> Option<DaemonInfo> {
    let info_path = home_dir.join("daemon.json");
    let contents = std::fs::read_to_string(info_path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Wait for an OS termination signal OR an API shutdown request.
///
/// On Unix: listens for SIGINT, SIGTERM, and API notify.
/// On Windows: listens for Ctrl+C and API notify.
async fn shutdown_signal(api_shutdown: Arc<tokio::sync::Notify>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to listen for SIGINT");
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");

        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), shutting down...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down...");
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Ctrl+C received, shutting down...");
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
            }
        }
    }
}

/// Check if a process with the given PID is still alive.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Use kill -0 to check if process exists without sending a signal
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        // tasklist /FI "PID eq N" returns "INFO: No tasks..." when no match,
        // or a table row with the PID when found. Check exit code and that
        // "INFO:" is NOT in the output to confirm the process exists.
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| {
                o.status.success() && {
                    let out = String::from_utf8_lossy(&o.stdout);
                    !out.contains("INFO:") && out.contains(&pid.to_string())
                }
            })
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Check if an LibreFang daemon is actually responding at the given address.
/// This avoids false positives where a different process reused the same PID
/// after a system reboot.
fn is_daemon_responding(addr: &str) -> bool {
    // Quick TCP connect check — don't make a full HTTP request to avoid delays
    let addr_only = addr
        .strip_prefix("http://")
        .or_else(|| addr.strip_prefix("https://"))
        .unwrap_or(addr);
    if let Ok(sock_addr) = addr_only.parse::<std::net::SocketAddr>() {
        std::net::TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_millis(500))
            .is_ok()
    } else {
        // Fallback: try connecting to hostname
        std::net::TcpStream::connect(addr_only)
            .map(|_| true)
            .unwrap_or(false)
    }
}
