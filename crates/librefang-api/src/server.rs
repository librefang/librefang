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

/// 构建 v1 API 路由树。
///
/// 每个领域子模块提供自己的 `router()` 方法，此处通过 `.merge()` 组合。
/// 路径相对于挂载点（如 `/health`、`/agents` 等），调用方将其嵌套在 `/api` 和 `/api/v1` 下。
///
/// 未来添加 v2 时只需新建 `api_v2_routes()`，挂载到 `/api/v2`，然后更新 `API_VERSION_LATEST`。
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
        // Dashboard 凭证登录（handler 定义在 server.rs 本地）
        .route(
            "/auth/dashboard-login",
            axum::routing::post(dashboard_login),
        )
        .route(
            "/auth/dashboard-check",
            axum::routing::get(dashboard_auth_check),
        )
        // OAuth/OIDC 外部认证端点
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

/// Dashboard credential login — validates username/password from config.toml
/// and returns a session token (HMAC-derived from credentials).
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
    let cfg_user = cfg_user.trim();
    let cfg_pass = resolve_dashboard_credential(
        &cfg.dashboard_pass,
        "LIBREFANG_DASHBOARD_PASS",
        &cfg.home_dir,
    );
    let cfg_pass = cfg_pass.trim();

    // If not configured, login is not needed
    if cfg_user.is_empty() || cfg_pass.is_empty() {
        return axum::response::Json(serde_json::json!({
            "ok": true, "token": "", "message": "No credentials required"
        }))
        .into_response();
    }

    let user = body.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let pass = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    // Constant-time comparison
    use subtle::ConstantTimeEq;
    let user_ok = user.as_bytes().ct_eq(cfg_user.as_bytes());
    let pass_ok = pass.as_bytes().ct_eq(cfg_pass.as_bytes());

    if user_ok.into() && pass_ok.into() {
        // Generate a deterministic token from credentials so no server-side state needed
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(cfg_pass.as_bytes()).expect("HMAC key");
        mac.update(cfg_user.as_bytes());
        mac.update(b"librefang-dashboard-session");
        let token = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();

        axum::response::Json(serde_json::json!({
            "ok": true,
            "token": token,
        }))
        .into_response()
    } else {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": "Invalid username or password"
            })),
        )
            .into_response()
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
    let has_credentials = !du.trim().is_empty() && !dp.trim().is_empty();
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

    let channels_config = kernel.config_ref().channels.clone();
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

    // Trim whitespace so `api_key = ""` or `api_key = "  "` both disable auth.
    let explicit_api_key = state.kernel.config_ref().api_key.trim().to_string();

    // Derive dashboard session token from credentials (if configured).
    let du_val = resolve_dashboard_credential(
        &state.kernel.config_ref().dashboard_user,
        "LIBREFANG_DASHBOARD_USER",
        state.kernel.home_dir(),
    );
    let dp_val = resolve_dashboard_credential(
        &state.kernel.config_ref().dashboard_pass,
        "LIBREFANG_DASHBOARD_PASS",
        state.kernel.home_dir(),
    );
    let du = du_val.trim();
    let dp = dp_val.trim();
    let dashboard_token = if !du.is_empty() && !dp.is_empty() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(dp.as_bytes()).expect("HMAC key");
        mac.update(du.as_bytes());
        mac.update(b"librefang-dashboard-session");
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    } else {
        String::new()
    };

    // Build composite key: both explicit api_key AND dashboard token are valid.
    // Middleware accepts any token matching either one.
    // Format: "key1\nkey2" — middleware splits and checks each.
    let api_key = match (explicit_api_key.is_empty(), dashboard_token.is_empty()) {
        (false, false) => format!("{explicit_api_key}\n{dashboard_token}"),
        (false, true) => explicit_api_key,
        (true, false) => dashboard_token,
        (true, true) => String::new(),
    };
    let api_key_lock = Arc::new(tokio::sync::RwLock::new(api_key));
    let gcra_limiter = rate_limiter::create_rate_limiter();

    // Build the versioned API routes. All /api/* endpoints are defined once
    // in api_v1_routes() and mounted at both /api and /api/v1 for backward
    // compatibility. Future versions (v2, v3) can be added as separate routers.
    let v1_routes = api_v1_routes();

    let app = Router::new()
        .route("/", axum::routing::get(webchat::webchat_page))
        .route(
            "/react-assets/{*path}",
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
        // Webhook 触发端点（不受版本管理 — 外部调用方使用固定 URL）
        .route("/hooks/wake", axum::routing::post(routes::webhook_wake))
        .route("/hooks/agent", axum::routing::post(routes::webhook_agent))
        // A2A 协议端点 + MCP HTTP（协议级别，不受版本管理）
        .merge(routes::network::protocol_router())
        // MCP HTTP 端点（协议级别，不受版本管理）
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
            api_key_lock,
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
        .layer(cors)
        .with_state(state.clone());

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

    // Shutdown kernel
    kernel.shutdown();

    info!("LibreFang daemon stopped");
    Ok(())
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
