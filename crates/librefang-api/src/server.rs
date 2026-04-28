//! LibreFang daemon server — boots the kernel and serves the HTTP API.

use crate::channel_bridge;
use crate::middleware;
use crate::rate_limiter;
use crate::routes::{self, AppState};
use crate::webchat;
use axum::response::IntoResponse;
use axum::Router;
use librefang_kernel::LibreFangKernel;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
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
        .merge(routes::audit::router())
        .merge(routes::authz::router())
        .merge(routes::channels::router())
        .merge(routes::system::router())
        .merge(routes::memory::router())
        .merge(routes::workflows::router())
        .merge(routes::skills::router())
        .merge(routes::network::router())
        .merge(routes::plugins::router())
        .merge(routes::providers::router())
        .merge(routes::budget::router())
        .merge(routes::auto_dream::router())
        .merge(routes::goals::router())
        .merge(routes::inbox::router())
        .merge(routes::media::router())
        .merge(routes::prompts::routes())
        .merge(routes::terminal::router())
        .merge(routes::users::router())
        // Dashboard credential login (handler defined locally in server.rs)
        .route(
            "/auth/dashboard-login",
            axum::routing::post(dashboard_login),
        )
        .route(
            "/auth/dashboard-check",
            axum::routing::get(dashboard_auth_check),
        )
        .route("/auth/logout", axum::routing::post(dashboard_logout))
        .route(
            "/auth/change-password",
            axum::routing::post(change_password),
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
        .route(
            "/auth/refresh",
            axum::routing::post(crate::oauth::auth_refresh),
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
    let cfg = kernel.config_ref();
    let explicit_api_key = cfg.api_key.trim();
    if explicit_api_key.is_empty() {
        // No api_key configured — API is open, no auth required.
        // Dashboard login is handled separately by session cookie checks.
        return tokens;
    }
    tokens.push(explicit_api_key.to_string());
    if let Some(token) = dashboard_session_token(kernel) {
        tokens.push(token);
    }
    tokens
}

pub(crate) fn has_dashboard_credentials(kernel: &LibreFangKernel) -> bool {
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
    !username.trim().is_empty()
        && (!cfg.dashboard_pass_hash.trim().is_empty() || !password.trim().is_empty())
}

pub(crate) fn configured_user_api_keys(kernel: &LibreFangKernel) -> Vec<middleware::ApiUserAuth> {
    kernel
        .config_ref()
        .users
        .iter()
        .filter_map(|user| {
            let api_key_hash = user.api_key_hash.as_deref()?.trim();
            if api_key_hash.is_empty() {
                return None;
            }
            Some(middleware::ApiUserAuth {
                name: user.name.clone(),
                role: librefang_kernel::auth::UserRole::from_str_role(&user.role),
                api_key_hash: api_key_hash.to_string(),
                user_id: librefang_types::agent::UserId::from_name(&user.name),
            })
        })
        .collect()
}

/// Wrap each persisted paired-device api key as an `ApiUserAuth` so the
/// auth middleware can verify mobile bearers against the same in-memory
/// table it uses for config-defined users. `device:{id}` namespacing keeps
/// device entries distinguishable from regular users — `pairing_remove_device`
/// also keys on this prefix when revoking access.
pub(crate) fn paired_device_user_keys(kernel: &LibreFangKernel) -> Vec<middleware::ApiUserAuth> {
    kernel
        .pairing_ref()
        .device_api_keys()
        .into_iter()
        .map(|(device_id, api_key_hash)| {
            let name = format!("device:{device_id}");
            middleware::ApiUserAuth {
                user_id: librefang_types::agent::UserId::from_name(&name),
                role: librefang_kernel::auth::UserRole::User,
                api_key_hash,
                name,
            }
        })
        .collect()
}

/// Returns `true` if the request arrived over TLS, either directly or through
/// a reverse proxy / tunnel that sets `X-Forwarded-Proto: https` (ngrok,
/// cloudflared, traefik, nginx, …). Used to decide whether cookies should be
/// issued with the `Secure` attribute.
///
/// Handles the multi-proxy case where the header is comma-separated — RFC 7239
/// semantics put the client-facing proto first (`https, http` = HTTPS reached
/// the outermost proxy, HTTP was the back-channel), so we split and check the
/// first value only.
fn request_is_https(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

/// Build the base attribute list for the `librefang_session` cookie. `Secure`
/// is added only when the request came in over HTTPS so local-HTTP dev keeps
/// working; any public deployment should be proxied behind TLS (at which point
/// `X-Forwarded-Proto` flips the flag on automatically).
fn session_cookie_attrs(headers: &axum::http::HeaderMap) -> &'static str {
    if request_is_https(headers) {
        "Path=/dashboard; HttpOnly; SameSite=Lax; Secure"
    } else {
        "Path=/dashboard; HttpOnly; SameSite=Lax"
    }
}

/// Dashboard credential login — validates username/password using Argon2id
/// (with transparent fallback from legacy plaintext passwords) and returns
/// a randomly generated session token with expiration metadata.
async fn dashboard_login(
    axum::extract::State(state): axum::extract::State<Arc<routes::AppState>>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let cfg = state.kernel.config_snapshot();
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

            // TOTP second-factor check for login
            let policy = state.kernel.approvals().policy();
            if policy.second_factor.requires_login_totp() {
                let totp_enrolled = state
                    .kernel
                    .vault_get("totp_secret")
                    .is_some_and(|s| !s.is_empty());
                let totp_confirmed =
                    state.kernel.vault_get("totp_confirmed").as_deref() == Some("true");
                if totp_enrolled && totp_confirmed {
                    let totp_code = body.get("totp_code").and_then(|v| v.as_str()).unwrap_or("");
                    if totp_code.is_empty() {
                        // Password OK but TOTP required — ask frontend to prompt
                        return axum::response::Json(serde_json::json!({
                            "ok": false,
                            "requires_totp": true,
                        }))
                        .into_response();
                    }
                    // Replay-prevention check (#3359): reject a code already used
                    // in the last 60 seconds.
                    if state.kernel.approvals().is_totp_code_used(totp_code) {
                        return (
                            axum::http::StatusCode::UNAUTHORIZED,
                            axum::response::Json(serde_json::json!({
                                "ok": false,
                                "error": "TOTP code has already been used. Wait for the next 30-second window.",
                            })),
                        )
                            .into_response();
                    }
                    // Verify TOTP code
                    let secret = state.kernel.vault_get("totp_secret").unwrap_or_default();
                    let issuer = policy.totp_issuer.clone();
                    match librefang_kernel::approval::ApprovalManager::verify_totp_code_with_issuer(
                        &secret, totp_code, &issuer,
                    ) {
                        Ok(true) => {
                            // Mark code as used so it cannot be replayed.
                            state.kernel.approvals().record_totp_code_used(totp_code);
                        }
                        Ok(false) => {
                            return (
                                axum::http::StatusCode::UNAUTHORIZED,
                                axum::response::Json(serde_json::json!({
                                    "ok": false,
                                    "error": "Invalid TOTP code",
                                })),
                            )
                                .into_response();
                        }
                        Err(e) => {
                            tracing::warn!("TOTP verification error during login: {e}");
                            return (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                axum::response::Json(serde_json::json!({
                                    "ok": false,
                                    "error": "TOTP verification failed",
                                })),
                            )
                                .into_response();
                        }
                    }
                }
            }

            // Store the session token so the auth middleware can validate it.
            // Attach the dashboard credential identity so the middleware can
            // attribute follow-up requests to an Owner-level principal — without
            // this, the session matches but the request stays anonymous and
            // RBAC-gated handlers (audit/query, per-user budget writes) reject
            // the dashboard caller as `None`. dashboard_pass is a single
            // operator-level credential, so Owner is the right ceiling.
            let mut session = token.clone();
            session.user_name = Some(cfg_user.clone());
            session.user_role = Some("owner".to_string());
            {
                let mut sessions = state.active_sessions.write().await;
                sessions.insert(session.token.clone(), session);
                // Persist so sessions survive daemon restarts.
                save_sessions(state.kernel.home_dir(), &sessions);
            }

            // Issue a session cookie so subsequent browser navigation to
            // `/dashboard/*` authenticates without JS sending a header.
            // Scope to `Path=/dashboard` so the cookie never auto-attaches
            // to `/api/*` requests — API calls keep using the Bearer token
            // from localStorage, which neutralises cookie-borne CSRF.
            // `Secure` is added when the request is HTTPS (direct or via a
            // TLS-terminating proxy), so the cookie cannot leak across
            // plaintext fallbacks of the same host.
            let cookie = format!(
                "librefang_session={}; {}; Max-Age={}",
                token.token,
                session_cookie_attrs(&headers),
                crate::password_hash::DEFAULT_SESSION_TTL_SECS
            );
            (
                axum::http::StatusCode::OK,
                [(axum::http::header::SET_COOKIE, cookie)],
                axum::response::Json(serde_json::json!({
                    "ok": true,
                    "token": token.token,
                    "created_at": token.created_at,
                    "expires_at": token.created_at + crate::password_hash::DEFAULT_SESSION_TTL_SECS,
                })),
            )
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
    let cfg = state.kernel.config_ref();
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
    let has_user_api_keys = cfg.users.iter().any(|user| {
        user.api_key_hash
            .as_deref()
            .is_some_and(|hash| !hash.trim().is_empty())
    });
    let mode = if has_credentials && (has_api_key || has_user_api_keys) {
        "hybrid"
    } else if has_credentials {
        "credentials"
    } else if has_api_key || has_user_api_keys {
        "api_key"
    } else {
        "none"
    };

    // Intentionally do NOT echo the configured dashboard username here: the
    // endpoint is unauthenticated (the SPA calls it before the user has
    // logged in) and returning the username would hand an anonymous remote
    // caller one half of the credential pair, enabling targeted credential
    // stuffing. The `mode` field is enough for the SPA to pick the right
    // login form; the user already knows their own username.
    axum::response::Json(serde_json::json!({
        "mode": mode,
        "username": "",
    }))
}

/// Invalidate the caller's dashboard session and clear the browser cookie.
///
/// Accepts the token via the `librefang_session` cookie, `Authorization:
/// Bearer ...`, or `X-API-Key`. Always clears the cookie client-side so a
/// caller who already lost their token can still wipe it locally.
async fn dashboard_logout(
    axum::extract::State(state): axum::extract::State<Arc<routes::AppState>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let token_from_cookie = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| {
            h.split(';')
                .map(str::trim)
                .find_map(|kv| kv.strip_prefix("librefang_session="))
                .map(str::to_string)
        });
    let token_from_bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").map(str::to_string));
    let token_from_xapi = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    // Dedup token sources: the typical case is that cookie + Bearer carry the
    // same session string (SPA send both), so without the set we'd acquire the
    // sessions lock and re-persist the same file up to three times per call.
    let tokens: HashSet<String> = [token_from_cookie, token_from_bearer, token_from_xapi]
        .into_iter()
        .flatten()
        .collect();

    if !tokens.is_empty() {
        let mut sessions = state.active_sessions.write().await;
        let mut removed_any = false;
        for token in &tokens {
            if sessions.remove(token).is_some() {
                removed_any = true;
            }
        }
        if removed_any {
            save_sessions(state.kernel.home_dir(), &sessions);
        }
    }

    let expired_cookie = format!(
        "librefang_session=; {}; Max-Age=0",
        session_cookie_attrs(&headers),
    );
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::SET_COOKIE, expired_cookie)],
        axum::response::Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

/// Request body for POST /api/auth/change-password.
#[derive(serde::Deserialize)]
struct ChangePasswordRequest {
    current_password: String,
    /// New password — optional, omit to keep the current password.
    new_password: Option<String>,
    /// New username — optional, omit to keep the current username.
    new_username: Option<String>,
}

/// Change the dashboard password and/or username.
///
/// Verifies the current password, then updates whichever credentials are
/// provided in the request body. At least one of `new_password` or
/// `new_username` must be non-empty. All existing sessions are invalidated on success.
async fn change_password(
    axum::extract::State(state): axum::extract::State<Arc<routes::AppState>>,
    axum::Json(body): axum::Json<ChangePasswordRequest>,
) -> axum::response::Response {
    let cfg = state.kernel.config_snapshot();

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

    // Must have credential-based auth configured
    let has_password = !pass_hash.is_empty() || !cfg_pass.is_empty();
    if cfg_user.is_empty() || !has_password {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": "Password authentication is not configured"
            })),
        )
            .into_response();
    }

    // Verify current password
    let verify = crate::password_hash::verify_dashboard_password(
        &cfg_user,
        &body.current_password,
        &cfg_user,
        &cfg_pass,
        pass_hash,
    );
    if matches!(verify, crate::password_hash::VerifyResult::Denied) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": "Current password is incorrect"
            })),
        )
            .into_response();
    }

    // At least one of new_password / new_username must be provided
    let new_pass_trimmed = body
        .new_password
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let new_user_trimmed = body
        .new_username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if new_pass_trimmed.is_none() && new_user_trimmed.is_none() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": "Provide at least a new password or new username"
            })),
        )
            .into_response();
    }

    // Validate new password length if provided
    if let Some(np) = new_pass_trimmed {
        if np.len() < 8 {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::response::Json(serde_json::json!({
                    "ok": false,
                    "error": "Password must be at least 8 characters"
                })),
            )
                .into_response();
        }
    }

    // Validate new username if provided
    if let Some(nu) = new_user_trimmed {
        if nu.len() < 2 {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::response::Json(serde_json::json!({
                    "ok": false,
                    "error": "Username must be at least 2 characters"
                })),
            )
                .into_response();
        }
    }

    // Load config.toml for writing
    let config_path = state.kernel.home_dir().join("config.toml");
    let mut table: toml::value::Table = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => toml::value::Table::new(),
        }
    } else {
        toml::value::Table::new()
    };

    // Update username if requested
    if let Some(nu) = new_user_trimmed {
        table.insert(
            "dashboard_user".to_string(),
            toml::Value::String(nu.to_string()),
        );
    }

    // Update password if requested
    if let Some(np) = new_pass_trimmed {
        let new_hash = match crate::password_hash::hash_password(np) {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Failed to hash new password: {e}");
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::response::Json(serde_json::json!({
                        "ok": false,
                        "error": "Failed to hash new password"
                    })),
                )
                    .into_response();
            }
        };
        table.insert(
            "dashboard_pass_hash".to_string(),
            toml::Value::String(new_hash),
        );
        // Remove legacy plaintext password if present
        table.remove("dashboard_pass");
    }

    let toml_string = match toml::to_string_pretty(&table) {
        Ok(s) => s,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Json(serde_json::json!({
                    "ok": false,
                    "error": format!("Failed to serialize config: {e}")
                })),
            )
                .into_response();
        }
    };
    if let Err(e) = std::fs::write(&config_path, &toml_string) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::response::Json(serde_json::json!({
                "ok": false,
                "error": format!("Failed to write config: {e}")
            })),
        )
            .into_response();
    }

    // Trigger config reload so the kernel picks up the new credentials
    if let Err(e) = state.kernel.reload_config().await {
        tracing::warn!("Config reload after credential change failed: {e}");
    }

    // Update api_key_lock so the derived static token reflects new credentials immediately
    let new_api_key = valid_api_tokens(state.kernel.as_ref()).join("\n");
    *state.api_key_lock.write().await = new_api_key;

    // Invalidate all existing sessions to force re-login
    state.active_sessions.write().await.clear();
    clear_sessions_file(state.kernel.home_dir());

    tracing::info!("Dashboard credentials changed successfully");

    axum::response::Json(serde_json::json!({
        "ok": true,
        "message": "Credentials changed successfully"
    }))
    .into_response()
}

/// Path to the file where active sessions are persisted across restarts.
fn sessions_path(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir.join("data").join("sessions.json")
}

/// Load persisted sessions from disk, dropping any that have already expired.
fn load_sessions(
    home_dir: &std::path::Path,
) -> std::collections::HashMap<String, crate::password_hash::SessionToken> {
    let path = sessions_path(home_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return std::collections::HashMap::new(),
    };
    let sessions: std::collections::HashMap<String, crate::password_hash::SessionToken> =
        serde_json::from_str(&content).unwrap_or_default();
    sessions
        .into_iter()
        .filter(|(_, st)| {
            !crate::password_hash::is_token_expired(
                st,
                crate::password_hash::DEFAULT_SESSION_TTL_SECS,
            )
        })
        .collect()
}

/// Persist active sessions to disk so they survive daemon restarts.
///
/// SECURITY: The file is written with owner-only permissions (0600) so that
/// bearer tokens stored in it cannot be read by other local users (#3589/#3725).
fn save_sessions(
    home_dir: &std::path::Path,
    sessions: &std::collections::HashMap<String, crate::password_hash::SessionToken>,
) {
    let path = sessions_path(home_dir);
    match serde_json::to_string(sessions) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                tracing::warn!("Failed to persist sessions: {e}");
            } else {
                // Restrict to owner-read/write only so bearer tokens are not
                // world-readable on multi-user systems.
                restrict_permissions(&path);
            }
        }
        Err(e) => tracing::warn!("Failed to serialize sessions: {e}"),
    }
}

/// Remove the sessions persistence file (called on password change to force re-login).
fn clear_sessions_file(home_dir: &std::path::Path) {
    let path = sessions_path(home_dir);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!("Failed to clear sessions file: {e}");
        }
    }
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
    // Webhook-based channels (Feishu, Teams, etc.) register their routes
    // for mounting on this server instead of starting separate HTTP servers.
    let (bridge, initial_webhook_router) =
        channel_bridge::start_channel_bridge(kernel.clone()).await;

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
    let persisted_sessions = load_sessions(kernel.home_dir());
    let active_sessions = Arc::new(tokio::sync::RwLock::new(persisted_sessions));
    let webhook_router = Arc::new(tokio::sync::RwLock::new(Arc::new(initial_webhook_router)));

    // Create api_key_lock before AppState so both AppState and AuthState share the same Arc.
    let api_key = valid_api_tokens(kernel.as_ref()).join("\n");
    let api_key_lock = Arc::new(tokio::sync::RwLock::new(api_key));
    // Per-user API key snapshot is wrapped in a `RwLock` so the rotate-key
    // endpoint (`POST /api/users/{name}/rotate-key`) can swap entries live —
    // both AppState (mutator) and AuthState (reader) share the same Arc, so
    // the next request after rotation sees the new hash and the old plaintext
    // bearer token immediately fails authentication.
    let user_api_keys_lock = Arc::new(tokio::sync::RwLock::new({
        let mut keys = configured_user_api_keys(kernel.as_ref());
        keys.extend(paired_device_user_keys(kernel.as_ref()));
        keys
    }));

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
        provider_test_cache: dashmap::DashMap::new(),
        webhook_store: crate::webhook_store::WebhookStore::load(
            kernel.home_dir().join("data").join("webhooks.json"),
        ),
        active_sessions: active_sessions.clone(),
        api_key_lock: api_key_lock.clone(),
        user_api_keys: user_api_keys_lock.clone(),
        media_drivers: librefang_runtime::media::MediaDriverCache::new_with_urls(
            kernel.config_ref().provider_urls.clone(),
        ),
        webhook_router,
        config_write_lock: tokio::sync::Mutex::new(()),
        pending_a2a_agents: dashmap::DashMap::new(),
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
        let cors_cfg = state.kernel.config_ref();
        for origin in &cors_cfg.cors_origin {
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

    // AuthState shares api_key_lock + user_api_keys with AppState so
    // change_password / rotate-key can update them live without a daemon
    // restart.
    let user_api_keys_initial_len = state.user_api_keys.read().await.len();
    let dashboard_auth_enabled = has_dashboard_credentials(state.kernel.as_ref());
    let api_key_set = !state.kernel.config_ref().api_key.trim().is_empty();
    let any_auth = api_key_set || user_api_keys_initial_len > 0 || dashboard_auth_enabled;

    // Resolve the effective value of `require_auth_for_reads`.
    // - Explicit `Some(true)`  → operators are forcing the allowlist
    //   closed even if auth is misconfigured (catches an accidental
    //   `api_key = ""` redeploy).
    // - Explicit `Some(false)` → operators are deliberately keeping the
    //   reads allowlist open even when an `api_key` is set; typical
    //   for deployments fronted by an external auth proxy.
    // - `None` (default)       → derive from whether *any* authentication
    //   is configured. This makes the safe default "set an api_key and
    //   the reads allowlist closes automatically", instead of forcing
    //   operators to remember a separate flag before reads stop leaking
    //   agent IDs to the LAN.
    let configured_require_auth_for_reads = state.kernel.config_ref().require_auth_for_reads;
    let require_auth_for_reads =
        derive_require_auth_for_reads(configured_require_auth_for_reads, any_auth);
    if require_auth_for_reads && !any_auth {
        tracing::warn!(
            "require_auth_for_reads = true but no authentication is configured \
             (api_key, user_api_keys, and dashboard credentials are all empty). \
             The flag will have no effect — set an api_key or configure dashboard \
             credentials to lock down read endpoints."
        );
    }
    if require_auth_for_reads && configured_require_auth_for_reads.is_none() {
        tracing::info!(
            "require_auth_for_reads auto-enabled because authentication is configured \
             (api_key / user_api_keys / dashboard credentials). Dashboard reads now \
             require a bearer token. Set `require_auth_for_reads = false` in config.toml \
             to restore the legacy public reads allowlist."
        );
    }
    // Read LIBREFANG_ALLOW_NO_AUTH once at boot — operators flip this to
    // run intentionally open on a non-loopback bind. Without it, an empty
    // api_key on a LAN/public bind fails closed for non-loopback origins.
    let allow_no_auth = std::env::var("LIBREFANG_ALLOW_NO_AUTH")
        .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false);

    // Loud startup warning when the server is bound to a non-loopback
    // address with no authentication configured. The middleware enforces
    // fail-closed for non-loopback traffic; this warning makes the
    // operator-facing posture explicit at boot.
    //
    // The default bind address is 127.0.0.1:4545 (loopback-only). Operators
    // who change api_listen to 0.0.0.0 or a public IP without configuring auth
    // get a security warning here (#3572).
    let bind_is_loopback = listen_addr.ip().is_loopback();
    if !any_auth && !bind_is_loopback {
        if allow_no_auth {
            // LIBREFANG_ALLOW_NO_AUTH=1 means the operator knowingly accepted
            // the risk. Use error! so the message stands out in logs regardless
            // of the configured log level.
            tracing::error!(
                "SECURITY WARNING: librefang is listening on {} with no authentication. \
                 Set api_key in config.toml or use 127.0.0.1:4545 for local-only access. \
                 (LIBREFANG_ALLOW_NO_AUTH=1 — operator accepted risk; running open.)",
                listen_addr
            );
        } else {
            tracing::warn!(
                "SECURITY WARNING: librefang is listening on {} with no authentication. \
                 Set api_key in config.toml or use 127.0.0.1:4545 for local-only access. \
                 Non-loopback requests will be rejected with 401 until an api_key is set.",
                listen_addr
            );
        }
    }

    let auth_state = middleware::AuthState {
        api_key_lock: api_key_lock.clone(),
        active_sessions: active_sessions.clone(),
        dashboard_auth_enabled,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads,
        allow_no_auth,
        // RBAC M5: hand the audit log to the auth layer so role-denial
        // events land in the same hash chain as everything else.
        audit_log: Some(state.kernel.audit().clone()),
    };
    let rl_cfg = state.kernel.config_ref().rate_limit.clone();
    let gcra_limiter = rate_limiter::GcraState {
        limiter: rate_limiter::create_rate_limiter(rl_cfg.api_requests_per_minute),
        retry_after_secs: rl_cfg.retry_after_secs,
    };

    // Build the versioned API routes. All /api/* endpoints are defined once
    // in api_v1_routes() and mounted at both /api and /api/v1 for backward
    // compatibility. Future versions (v2, v3) can be added as separate routers.
    let v1_routes = api_v1_routes();

    // Upload routes are defined separately so they can share the auth/rate-limit
    // layers but bypass RequestBodyLimitLayer — the handler enforces its own
    // configurable max_upload_size_bytes (default 10 MB).
    let upload_routes = Router::new()
        .route(
            "/api/agents/{id}/upload",
            axum::routing::post(routes::agents::upload_file),
        )
        .route(
            "/api/v1/agents/{id}/upload",
            axum::routing::post(routes::agents::upload_file),
        );

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
        // A2A protocol endpoints + MCP HTTP (protocol-level, not versioned).
        // Apply an explicit body limit (1 MB) to inbound A2A task payloads so
        // that external callers cannot exhaust server memory via oversized JSON
        // bodies. This is a defence-in-depth companion to the global
        // RequestBodyLimitLayer applied further down — the global limit uses the
        // operator-configurable max_request_body_bytes value, which may be
        // raised for other endpoints (e.g. file uploads). Pinning A2A separately
        // ensures memory exhaustion DoS attacks via /a2a/tasks/send are always
        // bounded (Bug #3785).
        .merge(
            routes::network::protocol_router()
                .layer(RequestBodyLimitLayer::new(1024 * 1024)),
        )
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
        // Upload routes must be merged BEFORE the layer calls so that auth and
        // rate-limit middleware apply to them.  They are intentionally excluded
        // from RequestBodyLimitLayer (applied below) because the handler
        // enforces its own configurable limit.
        .merge(upload_routes)
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
        .layer(CompressionLayer::new())
        // INFO-level request spans so they're created even when the console
        // log level is INFO. Required for the OpenTelemetry layer to pick
        // them up and ship to the OTLP collector — at DEBUG (the default for
        // `new_for_http`) spans never exist at INFO and the OTel exporter
        // sees nothing.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(tracing::Level::INFO)),
        )
        .layer(cors);

    // Apply the global request body size limit to the full app.  Upload routes
    // were merged before the security layers above and therefore covered by
    // auth/rate-limit, but they are NOT wrapped by this layer — Axum layers
    // only apply to routes registered before the layer call, so routes merged
    // after this point (channel_routes below) are also exempt.  Upload handler
    // enforces its own max_upload_size_bytes cap instead.
    let app = app.layer(RequestBodyLimitLayer::new(
        kernel.config_ref().max_request_body_bytes,
    ));

    // NOTE: HTTP metrics are recorded inside `request_logging` middleware via
    // `librefang_telemetry::metrics::record_http_request()`.  A separate metrics
    // middleware layer is not needed (and would double-count requests).

    // Mount channel webhook routes under /channels/{adapter_name}/*.
    // These bypass auth/rate-limit layers since external platforms (Feishu,
    // Teams, etc.) handle their own signature verification.
    // The router is dynamic (behind RwLock) so hot-reload can swap routes.
    //
    // SECURITY: Apply a per-route body-size cap *before* merging so that
    // webhook handlers are not exempt from the global RequestBodyLimitLayer
    // (which was applied above to `app`). Tower layers wrap the router they
    // are attached to; a layer added to `app` after `.nest()` would not
    // cover the nested router. 1 MiB is generous for any webhook payload
    // (Slack, Teams, Feishu, Line) while capping memory-exhaustion attacks.
    const WEBHOOK_BODY_LIMIT: usize = 1024 * 1024; // 1 MiB
    let channel_webhook_state = state.webhook_router.clone();
    let channel_routes = Router::new()
        .fallback(move |req: axum::extract::Request| {
            let wr = channel_webhook_state.clone();
            async move {
                use tower::ServiceExt;
                let guard = wr.read().await;
                let router: Arc<axum::Router> = Arc::clone(&guard);
                drop(guard);
                // Unwrap the Arc — if we hold the only reference we avoid a clone,
                // otherwise Router::clone is needed (only during hot-reload overlap).
                Arc::try_unwrap(router)
                    .unwrap_or_else(|arc| (*arc).clone())
                    .into_service()
                    .oneshot(req)
                    .await
                    .unwrap_or_else(|e: std::convert::Infallible| match e {})
            }
        })
        .layer(RequestBodyLimitLayer::new(WEBHOOK_BODY_LIMIT));
    let app = app.nest("/channels", channel_routes);

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

    // Auto-start observability stack (OTLP collector + Prometheus + Grafana)
    // ONLY when the operator has opted in via `telemetry.auto_start_observability_stack`.
    // Default is off because spinning four containers on every `librefang
    // start` is a strong implicit side effect; users who only want OTel export
    // to an existing collector should keep this off and just configure
    // `otlp_endpoint`. Issue #3136.
    //
    // Done before OTLP exporter init so the exporter gate can observe the
    // actual startup outcome — if `auto_start = true` but Docker is missing
    // or a port conflict kills compose, we must NOT init the exporter at the
    // default localhost:4317, otherwise the BatchSpanProcessor spams
    // ConnectionRefused on every export interval (issue #3136 follow-up).
    let mut observability_guard: Option<ObservabilityHandle> = if kernel
        .config_ref()
        .telemetry
        .enabled
        && kernel.config_ref().telemetry.auto_start_observability_stack
    {
        let project = derive_compose_project_name(kernel.home_dir());
        match start_observability_stack(kernel.home_dir(), &project) {
            Ok(ObservabilityStartup::Started) => {
                info!(
                    "Observability stack started ({project}: OTLP :4317/:4318, Tempo :3200, Prometheus :9090, Grafana :3000)"
                );
                Some(ObservabilityHandle::new(
                    kernel.home_dir().to_path_buf(),
                    project,
                ))
            }
            Ok(ObservabilityStartup::DockerUnavailable) => {
                info!("Docker not available, skipping observability stack");
                None
            }
            Ok(ObservabilityStartup::ComposeFailed { stderr }) => {
                tracing::warn!(
                    "Observability stack failed to start (likely a port conflict on 3000/3200/4317/9090 or an existing stack): {}",
                    stderr.trim()
                );
                None
            }
            Err(e) => {
                tracing::warn!("Failed to start observability stack: {e}");
                None
            }
        }
    } else {
        None
    };

    // Initialize OpenTelemetry OTLP tracing when telemetry feature is compiled
    // in and the config has `telemetry.enabled = true`. Skip the exporter when
    // no collector is reachable: explicit empty endpoint, or default localhost
    // endpoint without a running bundled stack (auto_start off, OR auto_start
    // on but startup failed above).
    #[cfg(feature = "telemetry")]
    {
        let cfg = kernel.config_ref();
        if cfg.telemetry.enabled {
            let stack_running = observability_guard.is_some();
            if cfg.telemetry.otlp_export_disabled(stack_running) {
                tracing::info!(
                    otlp_endpoint = %cfg.telemetry.otlp_endpoint,
                    auto_start_observability_stack =
                        cfg.telemetry.auto_start_observability_stack,
                    stack_running,
                    "Telemetry OTLP exporter skipped: no collector reachable. \
                     Set telemetry.auto_start_observability_stack = true (and \
                     ensure Docker is available) or override \
                     telemetry.otlp_endpoint to point at a running collector."
                );
            } else if let Err(e) = crate::telemetry::init_otel_tracing(
                &cfg.telemetry.otlp_endpoint,
                &cfg.telemetry.service_name,
                cfg.telemetry.sample_rate,
            ) {
                tracing::warn!("Failed to initialize OpenTelemetry tracing: {e}");
            }
        }
    }

    // Track background task handles for graceful shutdown.
    // `bg_shutdown_tx` is broadcast to all looping bg_tasks so they can exit
    // cleanly before we resort to abort().
    let (bg_shutdown_tx, _bg_shutdown_rx) =
        tokio::sync::watch::channel::<bool>(false);
    let mut bg_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    let (app, state) = build_router(kernel.clone(), addr).await;

    // Sync dashboard assets in background (downloads from release if outdated)
    {
        let home = kernel.home_dir().to_path_buf();
        bg_tasks.push(tokio::spawn(async move {
            crate::webchat::sync_dashboard(&home).await;
        }));
    }

    // Background provider key validation — runs shortly after boot so the
    // dashboard shows ValidatedKey / InvalidKey instead of just Configured.
    kernel.clone().spawn_key_validation();

    // Approval expiry sweep — checks for expired pending approval requests
    // every 10 seconds and handles their resolution.
    kernel.clone().spawn_approval_sweep_task();

    // Task-board stuck-task sweep — auto-resets in_progress tasks whose worker
    // stalled without calling `task_complete` (issue #2923 / #2926). Runs on
    // `task_board.sweep_interval_secs` (default 30s).
    kernel.clone().spawn_task_board_sweep_task();

    // Session stream hub idle GC — drops broadcast entries with no live
    // receivers so the per-session sender map does not grow unbounded under
    // churn (multi-client SSE attach, PR #3078).
    kernel.clone().spawn_session_stream_hub_gc_task();

    // Config file hot-reload watcher (polls every 30 seconds).
    // Spawned after `build_router` so it can access `AppState` for bridge reload.
    {
        let k = kernel.clone();
        let st = state.clone();
        let config_path = kernel.home_dir().join("config.toml");
        let mut shutdown_rx = bg_shutdown_tx.subscribe();
        bg_tasks.push(tokio::spawn(async move {
            let mut last_modified = std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .ok();
            loop {
                tokio::select! {
                    // Graceful shutdown signal: exit the loop so the task
                    // finishes cleanly instead of being aborted mid-operation.
                    _ = shutdown_rx.wait_for(|v| *v) => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
                }
                let current = std::fs::metadata(&config_path)
                    .and_then(|m| m.modified())
                    .ok();
                if current != last_modified && current.is_some() {
                    last_modified = current;
                    tracing::info!("Config file changed, reloading...");
                    match k.reload_config().await {
                        Ok(plan) => {
                            if plan.has_changes() {
                                tracing::info!("Config hot-reload applied: {:?}", plan.hot_actions);
                            } else {
                                tracing::debug!("Config hot-reload: no actionable changes");
                            }
                            // Restart channel bridge if channel config changed
                            if plan.hot_actions.contains(
                                &librefang_kernel::config_reload::HotAction::ReloadChannels,
                            ) {
                                match crate::channel_bridge::reload_channels_from_disk(&st).await {
                                    Ok(names) => {
                                        tracing::info!(
                                            "Hot-reload: restarted channel bridge with {} adapter(s): {:?}",
                                            names.len(),
                                            names,
                                        );
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Hot-reload: failed to restart channel bridge: {e}"
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => tracing::warn!("Config hot-reload failed: {e}"),
                    }
                }
            }
        }));
    }

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
            if let Err(e) = std::fs::remove_file(info_path) {
                tracing::warn!("Failed to remove stale daemon info file: {e}");
            }
        }

        let daemon_info = DaemonInfo {
            pid: std::process::id(),
            listen_addr: addr.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&daemon_info) {
            if let Err(e) = std::fs::write(info_path, json) {
                tracing::warn!("Failed to write daemon info file: {e}");
            }
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
        let mut shutdown_rx = bg_shutdown_tx.subscribe();
        bg_tasks.push(tokio::spawn(async move {
            loop {
                let cfg = kernel.config_snapshot();
                match librefang_runtime::catalog_sync::sync_catalog_to(
                    kernel.home_dir(),
                    &cfg.registry.registry_mirror,
                )
                .await
                {
                    Ok(result) => {
                        info!(
                            "Model catalog synced: {} files downloaded",
                            result.files_downloaded
                        );
                        if let Ok(mut catalog) = kernel.model_catalog_ref().write() {
                            let cfg = kernel.config_ref();
                            catalog.load_cached_catalog_for(&cfg.home_dir);
                            if !cfg.provider_regions.is_empty() {
                                let region_urls =
                                    catalog.resolve_region_urls(&cfg.provider_regions);
                                if !region_urls.is_empty() {
                                    catalog.apply_url_overrides(&region_urls);
                                }
                            }
                            if !cfg.provider_urls.is_empty() {
                                catalog.apply_url_overrides(&cfg.provider_urls);
                            }
                            catalog.detect_auth();
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Background catalog sync failed (will use cached/builtin): {e}"
                        );
                    }
                }
                // Wait 24 hours or until shutdown signal, whichever comes first.
                tokio::select! {
                    _ = shutdown_rx.wait_for(|v| *v) => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)) => {}
                }
            }
        }));
    }

    // Background: periodic GC for API-layer caches (every 5 minutes)
    {
        let st = state.clone();
        let mut shutdown_rx = bg_shutdown_tx.subscribe();
        bg_tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
            interval.tick().await; // Skip first immediate tick
            loop {
                tokio::select! {
                    _ = shutdown_rx.wait_for(|v| *v) => break,
                    _ = interval.tick() => {}
                }

                // Evict expired clawhub/skillhub cache entries (120s TTL)
                let cache_ttl = std::time::Duration::from_secs(120);
                let before_claw = st.clawhub_cache.len();
                st.clawhub_cache
                    .retain(|_, (fetched_at, _)| fetched_at.elapsed() < cache_ttl);
                let before_skill = st.skillhub_cache.len();
                st.skillhub_cache
                    .retain(|_, (fetched_at, _)| fetched_at.elapsed() < cache_ttl);

                // Evict expired session tokens
                let expired_sessions = {
                    let mut sessions = st.active_sessions.write().await;
                    let before = sessions.len();
                    sessions.retain(|_, token| {
                        !crate::password_hash::is_token_expired(
                            token,
                            crate::password_hash::DEFAULT_SESSION_TTL_SECS,
                        )
                    });
                    before - sessions.len()
                };

                let claw_removed = before_claw - st.clawhub_cache.len();
                let skill_removed = before_skill - st.skillhub_cache.len();
                let total = claw_removed + skill_removed + expired_sessions;
                if total > 0 {
                    tracing::info!(
                        clawhub = claw_removed,
                        skillhub = skill_removed,
                        sessions = expired_sessions,
                        "API cache GC sweep completed"
                    );
                }
            }
        }));
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

    // Signal background tasks to exit their loops gracefully, then wait up to
    // 5 seconds for each to finish. Abort any that haven't exited by then so
    // we don't stall shutdown indefinitely.
    let _ = bg_shutdown_tx.send(true);
    let grace = std::time::Duration::from_secs(5);
    for handle in bg_tasks {
        let abort = handle.abort_handle();
        match tokio::time::timeout(grace, handle).await {
            Ok(_) => {}
            Err(_) => {
                // Task did not finish within the grace period — abort as a
                // last resort so a mid-operation task does not stall shutdown.
                tracing::warn!(
                    "Background task did not finish within {}s of shutdown signal; aborting",
                    grace.as_secs()
                );
                abort.abort();
            }
        }
    }
    info!("Background tasks stopped");

    // Clean up daemon info file
    if let Some(info_path) = daemon_info_path {
        if let Err(e) = std::fs::remove_file(info_path) {
            tracing::warn!("Failed to remove daemon info file on shutdown: {e}");
        }
    }

    // Stop channel bridges
    if let Some(ref mut b) = *state.bridge_manager.lock().await {
        b.stop().await;
    }

    // Stop observability stack — graceful path. `.take()` consumes the guard
    // so its Drop becomes a no-op; if we never reach this line (panic, OOM,
    // SIGTERM) the Drop impl will still attempt a best-effort `compose down`.
    if let Some(handle) = observability_guard.take() {
        match stop_observability_stack(handle.home_dir(), handle.project_name()) {
            Ok(()) => info!("Observability stack stopped ({})", handle.project_name()),
            Err(e) => tracing::warn!("Failed to stop observability stack: {e}"),
        }
        // Mark the guard's stop as already attempted so its Drop is silent.
        std::mem::forget(handle);
    }

    // Clean up tmux session so child shell processes don't linger after shutdown.
    // Read config fields and drop the Guard before any `.await`.
    let (tmux_cleanup_enabled, tmux_cleanup_path) = {
        let cfg = kernel.config_ref();
        (
            cfg.terminal.tmux_enabled,
            std::path::PathBuf::from(cfg.terminal.tmux_binary_path.as_deref().unwrap_or("tmux")),
        )
    };
    if tmux_cleanup_enabled {
        let ctrl = crate::terminal_tmux::TmuxController::new(
            tmux_cleanup_path,
            crate::terminal_tmux::DEFAULT_TMUX_SESSION_NAME.to_string(),
        );
        if let Err(e) = ctrl.kill_session().await {
            tracing::debug!("tmux session cleanup: {e}");
        } else {
            info!("tmux session cleaned up");
        }
    }

    // Shutdown kernel
    kernel.shutdown();

    info!("LibreFang daemon stopped");
    Ok(())
}

/// Outcome of attempting to bring up the observability stack.
enum ObservabilityStartup {
    /// `docker compose up -d` exited successfully.
    Started,
    /// No working `docker` CLI reachable from this process.
    DockerUnavailable,
    /// `docker compose up -d` exited non-zero (port conflict, pre-existing
    /// stack, image pull failure, etc.). `stderr` carries docker's output so
    /// the operator can see why.
    ComposeFailed { stderr: String },
}

/// Observability assets embedded at compile time. Written to
/// `<home>/observability/` on boot so Docker Desktop (macOS) can bind-mount
/// them from a path that is always in its File Sharing list (`~`). Avoids
/// the `operation not permitted` failure we hit when the daemon runs from
/// an external disk (`/Volumes/...`) that the user has not added manually.
const OBSERVABILITY_ASSETS: &[(&str, &str)] = &[
    (
        "docker-compose.observability.yml",
        include_str!("../../../deploy/docker-compose.observability.yml"),
    ),
    (
        "prometheus/prometheus.yml",
        include_str!("../../../deploy/prometheus/prometheus.yml"),
    ),
    (
        "otel-collector/config.yaml",
        include_str!("../../../deploy/otel-collector/config.yaml"),
    ),
    (
        "tempo/tempo.yaml",
        include_str!("../../../deploy/tempo/tempo.yaml"),
    ),
    (
        "grafana/provisioning/datasources/prometheus.yml",
        include_str!("../../../deploy/grafana/provisioning/datasources/prometheus.yml"),
    ),
    (
        "grafana/provisioning/datasources/tempo.yml",
        include_str!("../../../deploy/grafana/provisioning/datasources/tempo.yml"),
    ),
    (
        "grafana/provisioning/dashboards/dashboard.yml",
        include_str!("../../../deploy/grafana/provisioning/dashboards/dashboard.yml"),
    ),
    (
        "grafana/dashboards/librefang.json",
        include_str!("../../../deploy/grafana/dashboards/librefang.json"),
    ),
    (
        "grafana/dashboards/librefang-llm.json",
        include_str!("../../../deploy/grafana/dashboards/librefang-llm.json"),
    ),
    (
        "grafana/dashboards/librefang-http.json",
        include_str!("../../../deploy/grafana/dashboards/librefang-http.json"),
    ),
    (
        "grafana/dashboards/librefang-cost.json",
        include_str!("../../../deploy/grafana/dashboards/librefang-cost.json"),
    ),
    (
        "grafana/dashboards/ollama.json",
        include_str!("../../../deploy/grafana/dashboards/ollama.json"),
    ),
];

/// Stage all embedded observability assets under `<home>/observability/`,
/// overwriting on every call so upgrades ship new configs without
/// manual intervention. Returns the staged compose file path.
fn stage_observability_assets(home_dir: &Path) -> std::io::Result<std::path::PathBuf> {
    let root = home_dir.join("observability");
    for (rel, contents) in OBSERVABILITY_ASSETS {
        let target = root.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, contents)?;
    }
    Ok(root.join("docker-compose.observability.yml"))
}

/// Derive a Docker Compose project name unique to this `home_dir`. Without
/// an explicit `-p`, compose falls back to the working-dir basename
/// (`observability`) which collides between two daemons booted with
/// different home dirs and lets either tear down the other's stack. Hash
/// the absolute home_dir path and prefix with `librefang-` so the project
/// name stays scannable in `docker ps` output. Issue #3136.
fn derive_compose_project_name(home_dir: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    // Canonicalize when possible so equivalent paths (`/tmp/x` vs
    // `/private/tmp/x` on macOS) map to the same project.
    let canonical = std::fs::canonicalize(home_dir).unwrap_or_else(|_| home_dir.to_path_buf());
    canonical.hash(&mut hasher);
    format!("librefang-{:08x}", hasher.finish() as u32)
}

/// RAII guard that calls `stop_observability_stack` on Drop. The graceful
/// shutdown path consumes the guard via `mem::forget` after explicitly
/// stopping (so success can be logged at the right moment); any path that
/// skips the explicit stop — panic, early return, axum's error branch —
/// still gets a best-effort cleanup. SIGKILL is unreachable from here, so
/// operators on hostile-shutdown paths still need `docker compose -p
/// librefang-<hash> down` manually; that's acknowledged in issue #3136.
struct ObservabilityHandle {
    home_dir: std::path::PathBuf,
    project_name: String,
}

impl ObservabilityHandle {
    fn new(home_dir: std::path::PathBuf, project_name: String) -> Self {
        Self {
            home_dir,
            project_name,
        }
    }

    fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    fn project_name(&self) -> &str {
        &self.project_name
    }
}

impl Drop for ObservabilityHandle {
    fn drop(&mut self) {
        // Best-effort: log the failure but never panic from Drop.
        if let Err(e) = stop_observability_stack(&self.home_dir, &self.project_name) {
            tracing::warn!(
                project = %self.project_name,
                "non-graceful exit: failed to tear down observability stack: {e}"
            );
        }
    }
}

/// Check if Docker is available and start the observability stack under
/// `project_name` so two daemons with different home dirs don't fight over
/// the same compose project.
fn start_observability_stack(
    home_dir: &Path,
    project_name: &str,
) -> Result<ObservabilityStartup, Box<dyn std::error::Error>> {
    // Check if docker CLI exists and daemon is reachable
    let docker_check = std::process::Command::new("docker")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match docker_check {
        Ok(status) if status.success() => {}
        _ => return Ok(ObservabilityStartup::DockerUnavailable),
    }

    let compose_file = stage_observability_assets(home_dir)
        .map_err(|e| format!("failed to stage observability assets: {e}"))?;

    let output = std::process::Command::new("docker")
        .args(["compose", "-p", project_name, "-f"])
        .arg(&compose_file)
        .args(["up", "-d"])
        .output()
        .map_err(|e| format!("docker compose up failed to spawn: {e}"))?;

    if output.status.success() {
        Ok(ObservabilityStartup::Started)
    } else {
        Ok(ObservabilityStartup::ComposeFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Stop the observability stack identified by `project_name`. Idempotent:
/// returns `Ok(())` when the compose file is missing (already torn down or
/// never started) or when `compose down` succeeds with no containers.
fn stop_observability_stack(
    home_dir: &Path,
    project_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let compose_file = home_dir
        .join("observability")
        .join("docker-compose.observability.yml");
    if !compose_file.exists() {
        return Ok(());
    }

    std::process::Command::new("docker")
        .args(["compose", "-p", project_name, "-f"])
        .arg(&compose_file)
        .args(["down"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("docker compose down failed: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod observability_tests {
    use super::*;

    #[test]
    fn derive_project_name_is_stable_for_same_home() {
        let p = std::path::PathBuf::from("/tmp/librefang-test-home-a");
        let a = derive_compose_project_name(&p);
        let b = derive_compose_project_name(&p);
        assert_eq!(a, b, "same home_dir must produce the same project name");
        assert!(
            a.starts_with("librefang-"),
            "project name must be operator-recognisable in `docker ps`: {a}"
        );
    }

    #[test]
    fn derive_project_name_differs_for_different_homes() {
        let a = derive_compose_project_name(std::path::Path::new("/tmp/librefang-home-A"));
        let b = derive_compose_project_name(std::path::Path::new("/tmp/librefang-home-B"));
        assert_ne!(
            a, b,
            "two daemons with distinct home_dirs must NOT share a compose project"
        );
    }
}

/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
/// On non-Unix platforms this is a no-op.
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!("Failed to restrict permissions on {}: {e}", path.display());
    }
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
        let _ = pid; // suppress unused variable warning on unsupported platforms
        false
    }
}

/// Resolve the effective value of `require_auth_for_reads` from the explicit
/// config option and whether any authentication method is configured.
///
/// - `Some(explicit)` preserves the operator's stated intent verbatim.
/// - `None` derives the value from `any_auth` so that setting any form of
///   auth (api_key / user keys / dashboard credentials) automatically closes
///   the dashboard reads allowlist.
fn derive_require_auth_for_reads(configured: Option<bool>, any_auth: bool) -> bool {
    match configured {
        Some(explicit) => explicit,
        None => any_auth,
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

#[cfg(test)]
mod derive_require_auth_for_reads_tests {
    use super::derive_require_auth_for_reads;

    #[test]
    fn none_with_auth_enables() {
        assert!(derive_require_auth_for_reads(None, true));
    }

    #[test]
    fn none_without_auth_disables() {
        assert!(!derive_require_auth_for_reads(None, false));
    }

    #[test]
    fn some_false_is_preserved_even_when_auth_configured() {
        assert!(!derive_require_auth_for_reads(Some(false), true));
    }

    #[test]
    fn some_true_is_preserved_even_when_no_auth_configured() {
        assert!(derive_require_auth_for_reads(Some(true), false));
    }
}
