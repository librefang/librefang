//! Production middleware for the LibreFang API server.
//!
//! Provides:
//! - Request ID generation and propagation
//! - Per-endpoint structured request logging
//! - HTTP metrics recording (when telemetry feature is enabled)
//! - In-memory rate limiting (per IP)
//! - Accept-Language header parsing for i18n error responses

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::middleware::Next;
use librefang_kernel::auth::UserRole;
use librefang_types::i18n;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

use librefang_telemetry::metrics;

/// Shared state for the auth middleware.
///
/// Combines the static API key(s) with the active session store so the
/// middleware can validate both legacy deterministic tokens and the new
/// randomly generated session tokens in a single pass.
#[derive(Clone)]
pub struct AuthState {
    /// Composite key string: multiple valid tokens separated by `\n`.
    pub api_key_lock: Arc<tokio::sync::RwLock<String>>,
    /// Active sessions issued by dashboard login, keyed by token string.
    pub active_sessions:
        Arc<tokio::sync::RwLock<HashMap<String, crate::password_hash::SessionToken>>>,
    /// Whether dashboard username/password auth is configured.
    pub dashboard_auth_enabled: bool,
    /// Optional per-user API-key hashes used for role-based API access.
    pub user_api_keys: Arc<Vec<ApiUserAuth>>,
    /// When `true` and an `api_key` is configured, GET endpoints that are
    /// otherwise on the dashboard public-read allowlist (agents, config,
    /// budget, sessions, approvals, hands, skills, workflows, …) are forced
    /// through bearer authentication. Static assets, OAuth entry points, and
    /// `/api/health*` remain public so the daemon stays probeable.
    pub require_auth_for_reads: bool,
}

#[derive(Clone)]
pub struct ApiUserAuth {
    pub name: String,
    pub role: UserRole,
    pub api_key_hash: String,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedApiUser {
    pub name: String,
    pub role: UserRole,
}

/// Endpoints that mutate kernel-wide configuration, user accounts, or
/// daemon lifecycle. `librefang_kernel::auth::Action::{ModifyConfig,
/// ManageUsers}` requires `UserRole::Owner` at the kernel layer; the
/// HTTP surface must agree, otherwise an Admin API key can change
/// configuration / rotate the bearer token / reload the daemon that a
/// Owner is responsible for.
fn is_owner_only_write(method: &axum::http::Method, path: &str) -> bool {
    // Only non-GET methods are candidates — reads are handled separately.
    if *method == axum::http::Method::GET {
        return false;
    }
    // Exact-match list. These are the only routes the current codebase
    // exposes that cross the "Owner action" line; add here rather than
    // matching a prefix so a new Admin-write endpoint doesn't silently
    // get locked to Owner by accident.
    matches!(
        path,
        "/api/config"
            | "/api/config/set"
            | "/api/config/reload"
            | "/api/auth/change-password"
            | "/api/shutdown"
    )
}

/// Whitelist check for per-user API-key access.
///
/// - `Owner`: full access.
/// - `Admin`: full access **except** Owner-only writes (see
///   [`is_owner_only_write`]) — kernel-wide config, user management,
///   daemon lifecycle, and the bearer-token change endpoint.
/// - `User`: GET everything + POST to a limited set of endpoints
///   (agent messages, clone, approval actions).
/// - `Viewer`: GET only.
/// - All other methods (`PUT`/`DELETE`/`PATCH`) require `Admin`+.
///
/// The `path` must already be normalized (no trailing slash, version prefix
/// stripped) before calling this function.
fn user_role_allows_request(role: UserRole, method: &axum::http::Method, path: &str) -> bool {
    // Owner-only writes: even Admin cannot touch these.
    if is_owner_only_write(method, path) {
        return role >= UserRole::Owner;
    }

    if role >= UserRole::Admin || *method == axum::http::Method::GET {
        return true;
    }

    if role < UserRole::User {
        return false;
    }

    // User role: only specific POST endpoints are allowed.
    if *method == axum::http::Method::POST {
        let agent_message = path.starts_with("/api/agents/")
            && (path.ends_with("/message") || path.ends_with("/message/stream"));
        let agent_clone = path.starts_with("/api/agents/") && path.ends_with("/clone");
        let approval_action = path == "/api/approvals/batch"
            || path.ends_with("/approve")
            || path.ends_with("/reject")
            || path.ends_with("/modify");
        return agent_message || agent_clone || approval_action;
    }

    false
}

/// Request ID header name (standard).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Resolved language code extracted from the `Accept-Language` header.
///
/// Inserted into request extensions by the [`accept_language`] middleware so
/// that downstream route handlers can produce localized error messages.
#[derive(Clone, Debug)]
pub struct RequestLanguage(pub &'static str);

/// Middleware: parse `Accept-Language` header and store the resolved language
/// in request extensions for downstream handlers.
///
/// Also sets the `Content-Language` response header to indicate which language
/// was used.
pub async fn accept_language(mut request: Request<Body>, next: Next) -> Response<Body> {
    let lang = request
        .headers()
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .map(i18n::parse_accept_language)
        .unwrap_or(i18n::DEFAULT_LANGUAGE);

    request.extensions_mut().insert(RequestLanguage(lang));

    let mut response = next.run(request).await;

    if let Ok(header_val) = lang.parse() {
        response
            .headers_mut()
            .insert("content-language", header_val);
    }

    response
}

/// Middleware: inject a unique request ID and log the request/response.
pub async fn request_logging(request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let method = request.method().clone();
    let uri = request.uri().path().to_string();
    let start = Instant::now();

    let mut response = next.run(request).await;

    let elapsed = start.elapsed();
    let status = response.status().as_u16();

    // GET 2xx — routine polling, keep out of INFO to reduce noise
    if method == axum::http::Method::GET && status < 300 {
        debug!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            "API request"
        );
    } else {
        info!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            "API request"
        );
    }

    metrics::record_http_request(&uri, method.as_str(), status, elapsed);

    // Inject the request ID into the response
    if let Ok(header_val) = request_id.parse() {
        response.headers_mut().insert(REQUEST_ID_HEADER, header_val);
    }

    response
}

/// API version headers middleware.
///
/// Adds `X-API-Version` to every response so clients always know which version
/// they are talking to. When a request targets `/api/v1/...` the header reflects
/// `v1`; for the unversioned `/api/...` alias it returns the latest version.
///
/// Also performs content-type negotiation: if the `Accept` header contains
/// `application/vnd.librefang.<version>+json` the response version header
/// reflects the negotiated version. If the requested version is unknown the
/// server returns `406 Not Acceptable`.
pub async fn api_version_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let path = request.uri().path().to_string();

    let path_version = crate::versioning::version_from_path(&path);
    let accept_version = request
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::versioning::version_from_accept_header);

    // Check Accept header for version negotiation
    let requested_accept_version = request
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::versioning::requested_version_from_accept_header);

    // Validate negotiated version if provided
    if path_version.is_none() {
        if let Some(ver) = requested_accept_version {
            let known = crate::server::API_VERSIONS.iter().any(|(v, _)| *v == ver);
            if !known {
                return Response::builder()
                    .status(StatusCode::NOT_ACCEPTABLE)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "error": format!("Unsupported API version: {ver}"),
                            "available": crate::server::API_VERSIONS
                                .iter()
                                .map(|(v, _)| *v)
                                .collect::<Vec<_>>(),
                        })
                        .to_string(),
                    ))
                    .unwrap_or_default();
            }
        }
    }

    let mut response = next.run(request).await;

    // Determine the version to report. Explicit path versions win over headers.
    let version = if let Some(ver) = path_version {
        ver.to_string()
    } else if let Some(ver) = accept_version {
        ver.to_string()
    } else {
        crate::server::API_VERSION_LATEST.to_string()
    };

    if let Ok(val) = version.parse() {
        response.headers_mut().insert("x-api-version", val);
    } else {
        tracing::warn!("Failed to set X-API-Version header: {:?}", version);
    }

    response
}

/// Bearer token authentication middleware.
///
/// When `api_key` is non-empty (after trimming), requests to non-public
/// endpoints must include `Authorization: Bearer <api_key>`.
/// If the key is empty or whitespace-only, auth is disabled entirely
/// (public/local development mode).
///
/// Also validates randomly generated session tokens from the active
/// session store, cleaning up expired sessions on each check.
pub async fn auth(
    axum::extract::State(auth_state): axum::extract::State<AuthState>,
    mut request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let api_key = auth_state.api_key_lock.read().await.clone();
    // SECURITY: Capture method early for method-aware public endpoint checks.
    let method = request.method().clone();

    // Shutdown is loopback-only (CLI on same machine) — skip token auth.
    // Normalize versioned paths: /api/v1/foo → /api/foo so public endpoint
    // checks work identically for both /api/ and /api/v1/ prefixes.
    let raw_path = request.uri().path().to_string();
    // Normalize: strip version prefix and trailing slashes so ACL checks
    // work consistently (e.g. "/api/v1/agents/" → "/api/agents").
    let after_version: String = if raw_path.starts_with("/api/v1/") {
        format!("/api{}", &raw_path[7..])
    } else if raw_path == "/api/v1" {
        "/api".to_string()
    } else {
        raw_path.clone()
    };
    // Strip a trailing slash for consistent ACL matching, but preserve the
    // root path "/" itself — otherwise stripping turns it into the empty
    // string, and `is_public` checks that compare against "/" (e.g. for the
    // dashboard HTML) silently miss, returning 401 for GET /.
    let path: &str = if after_version == "/" {
        "/"
    } else {
        after_version.strip_suffix('/').unwrap_or(&after_version)
    };
    if path == "/api/shutdown" {
        let is_loopback = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false); // SECURITY: default-deny — unknown origin is NOT loopback
        if is_loopback {
            return next.run(request).await;
        }
    }

    // Public endpoints that don't require auth (dashboard needs these).
    // SECURITY: /api/agents is GET-only (listing). POST (spawn) requires auth.
    // SECURITY: Public endpoints are GET-only unless explicitly noted.
    // POST/PUT/DELETE to any endpoint ALWAYS requires auth to prevent
    // unauthenticated writes (cron job creation, skill install, etc.).
    let is_get = method == axum::http::Method::GET;

    // "Always public" endpoints stay reachable with no token even when
    // `require_auth_for_reads` is on. These are either (a) static assets
    // needed to render the login screen, (b) auth flow entry points, or
    // (c) minimal liveness probes that leak nothing sensitive.
    //
    // `/api/status` intentionally stays out of this set: its handler returns
    // the full agent listing (id + name + model + profile) plus `home_dir`,
    // `api_listen`, and session count, which is exactly the enumeration
    // surface `require_auth_for_reads` exists to close. It lives in the
    // `dashboard_read_*` group below so it gets locked down with the flag.
    //
    // `/api/health/detail` is **not** in any public set — its own doc comment
    // at routes/config.rs:317 says it "requires auth", and it returns
    // `panic_count`, `restart_count`, `agent_count`, embedding/extraction
    // model IDs, `config_warnings` from `KernelConfig::validate()`, and the
    // event-bus drop count. All operational data that should not be reachable
    // from a cold probe. Unlike the dashboard read group, this endpoint
    // requires auth unconditionally regardless of `require_auth_for_reads`,
    // so the middleware contract finally matches the handler's own docs.
    // `/api/health` stays public because its payload is genuinely minimal
    // (status + version + a two-item checks array) and load balancers /
    // orchestrators need it for probing.
    let always_public_method_free = matches!(
        path,
        "/" | "/logo.png"
            | "/favicon.ico"
            | "/api/versions"
            | "/api/health"
            | "/api/version"
            | "/api/auth/callback"
            | "/api/auth/dashboard-login"
            | "/api/auth/dashboard-check"
    ) || path.starts_with("/api/providers/github-copilot/oauth/");
    // MCP OAuth callback — browser redirect from OAuth provider, no API key.
    // Pattern: /api/mcp/servers/{name}/auth/callback — GET only.
    let is_mcp_oauth_callback =
        is_get && path.starts_with("/api/mcp/servers/") && path.ends_with("/auth/callback");
    let always_public_get_only = is_get
        && (matches!(
            path,
            "/.well-known/agent.json" | "/api/config/schema" | "/api/auth/providers"
        ) || path.starts_with("/dashboard/")
            || path.starts_with("/a2a/")
            || path.starts_with("/api/uploads/")
            || path.starts_with("/api/auth/login"));
    let always_public =
        always_public_method_free || always_public_get_only || is_mcp_oauth_callback;

    // "Dashboard reads" — the legacy public allowlist that lets the SPA
    // render before the user enters credentials. Downgraded to authenticated
    // when `require_auth_for_reads` is enabled AND an `api_key` is configured,
    // so a remote attacker can no longer enumerate agents, config, budget,
    // sessions, approvals, hands, skills, or workflows.
    let dashboard_read_exact = matches!(
        path,
        "/api/agents"
            | "/api/profiles"
            | "/api/config"
            | "/api/status"
            | "/api/models"
            | "/api/models/aliases"
            | "/api/providers"
            | "/api/budget"
            | "/api/budget/agents"
            | "/api/network/status"
            | "/api/a2a/agents"
            | "/api/approvals"
            | "/api/channels"
            | "/api/hands"
            | "/api/hands/active"
            | "/api/skills"
            | "/api/sessions"
            | "/api/mcp/servers"
            | "/api/mcp/catalog"
            | "/api/mcp/health"
            | "/api/workflows"
    );
    let dashboard_read_prefix = path.starts_with("/api/budget/agents/")
        || path.starts_with("/api/approvals/")
        || path.starts_with("/api/hands/")
        || path.starts_with("/api/cron/");
    let dashboard_read_public =
        (is_get && (dashboard_read_exact || dashboard_read_prefix)) || path == "/api/logs/stream"; // SSE stream, read-only

    // The flag only engages when *some* form of auth is actually configured.
    // Gating on `api_key.is_empty()` alone would silently no-op the flag
    // whenever an operator configures only per-user keys or dashboard
    // username/password auth — which is exactly the setup most production
    // deployments use. Mirror the "auth configured?" check below so every
    // auth mode participates.
    let auth_configured = !api_key.trim().is_empty()
        || !auth_state.user_api_keys.is_empty()
        || auth_state.dashboard_auth_enabled;
    let enforce_auth_on_reads = auth_state.require_auth_for_reads && auth_configured;

    let is_public = always_public || (dashboard_read_public && !enforce_auth_on_reads);

    if is_public {
        return next.run(request).await;
    }

    // If no API key configured (empty, whitespace-only, or missing), skip auth
    // entirely. Users who don't set api_key accept that all endpoints are open.
    // To secure the dashboard, set a non-empty api_key in config.toml.
    let api_key = api_key.trim();
    if api_key.is_empty()
        && auth_state.user_api_keys.is_empty()
        && !auth_state.dashboard_auth_enabled
    {
        return next.run(request).await;
    }

    // Check Authorization: Bearer <token> header, then fallback to X-API-Key
    let bearer_token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let api_token = bearer_token.or_else(|| {
        request
            .headers()
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
    });

    // Split composite key (supports multiple valid tokens separated by \n).
    let valid_keys: Vec<&str> = api_key.split('\n').filter(|k| !k.is_empty()).collect();

    // Helper: constant-time check against any valid key
    let matches_any = |token: &str| -> bool {
        use subtle::ConstantTimeEq;
        valid_keys
            .iter()
            .any(|key| key.len() == token.len() && token.as_bytes().ct_eq(key.as_bytes()).into())
    };

    // SECURITY: Use constant-time comparison to prevent timing attacks.
    let header_auth = api_token.map(&matches_any);

    // Also check ?token= query parameter (for EventSource/SSE clients that
    // cannot set custom headers, same approach as WebSocket auth).
    let query_token = request
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("token=")));

    // SECURITY: Use constant-time comparison to prevent timing attacks.
    let query_auth = query_token.map(&matches_any);

    // Accept if either auth method matches a static API key or legacy token
    if header_auth == Some(true) || query_auth == Some(true) {
        return next.run(request).await;
    }

    // Check the active session store for randomly generated dashboard tokens.
    // Also prune expired sessions opportunistically.
    let provided_token = api_token.or(query_token);
    if let Some(token_str) = provided_token {
        let mut sessions = auth_state.active_sessions.write().await;
        // Remove expired sessions while we hold the lock
        sessions.retain(|_, st| {
            !crate::password_hash::is_token_expired(
                st,
                crate::password_hash::DEFAULT_SESSION_TTL_SECS,
            )
        });
        if sessions.contains_key(token_str) {
            drop(sessions);
            return next.run(request).await;
        }
        drop(sessions);

        if let Some(user) = auth_state
            .user_api_keys
            .iter()
            .find(|user| crate::password_hash::verify_password(token_str, &user.api_key_hash))
            .cloned()
        {
            if !user_role_allows_request(user.role, &method, path) {
                let lang = request
                    .extensions()
                    .get::<RequestLanguage>()
                    .map(|rl| rl.0)
                    .unwrap_or(i18n::DEFAULT_LANGUAGE);
                return Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .header("content-type", "application/json")
                    .header("content-language", lang)
                    .body(Body::from(
                        serde_json::json!({
                            "error": format!(
                                "Role '{}' is not allowed to access this endpoint",
                                user.role
                            )
                        })
                        .to_string(),
                    ))
                    .unwrap_or_default();
            }

            request.extensions_mut().insert(AuthenticatedApiUser {
                name: user.name,
                role: user.role,
            });
            return next.run(request).await;
        }
    }

    // Determine error message: was a credential provided but wrong, or missing entirely?
    // Use the request language (set by accept_language middleware) for i18n.
    let lang = request
        .extensions()
        .get::<RequestLanguage>()
        .map(|rl| rl.0)
        .unwrap_or(i18n::DEFAULT_LANGUAGE);
    let translator = i18n::ErrorTranslator::new(lang);

    let credential_provided = header_auth.is_some() || query_auth.is_some();
    let error_msg = if credential_provided {
        translator.t("api-error-auth-invalid-key")
    } else {
        translator.t("api-error-auth-missing-header")
    };

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("www-authenticate", "Bearer")
        .header("content-language", lang)
        .body(Body::from(
            serde_json::json!({"error": error_msg}).to_string(),
        ))
        .unwrap_or_default()
}

/// Security headers middleware — applied to ALL API responses.
pub async fn security_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("x-xss-protection", "1; mode=block".parse().unwrap());
    // All JS/CSS is bundled inline — only external resource is Google Fonts.
    headers.insert(
        "content-security-policy",
        "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com https://fonts.gstatic.com; img-src 'self' data: blob:; connect-src 'self' ws://localhost:* ws://127.0.0.1:* wss://localhost:* wss://127.0.0.1:*; font-src 'self' https://fonts.gstatic.com; media-src 'self' blob:; frame-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'"
            .parse()
            .unwrap(),
    );
    headers.insert(
        "referrer-policy",
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    headers.insert(
        "cache-control",
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    headers.insert(
        "strict-transport-security",
        "max-age=63072000; includeSubDomains".parse().unwrap(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    #[test]
    fn test_request_id_header_constant() {
        assert_eq!(REQUEST_ID_HEADER, "x-request-id");
    }

    #[test]
    fn test_user_role_admin_cannot_modify_config() {
        // Admin must be blocked from kernel-wide config mutations.
        let post = axum::http::Method::POST;
        for path in [
            "/api/config",
            "/api/config/set",
            "/api/config/reload",
            "/api/auth/change-password",
            "/api/shutdown",
        ] {
            assert!(
                !user_role_allows_request(UserRole::Admin, &post, path),
                "Admin must NOT be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_owner_still_allowed_on_config_writes() {
        let post = axum::http::Method::POST;
        for path in [
            "/api/config",
            "/api/config/set",
            "/api/config/reload",
            "/api/auth/change-password",
            "/api/shutdown",
        ] {
            assert!(
                user_role_allows_request(UserRole::Owner, &post, path),
                "Owner must be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_admin_can_still_spawn_agents_and_install_skills() {
        let post = axum::http::Method::POST;
        for path in ["/api/agents", "/api/skills/install"] {
            assert!(
                user_role_allows_request(UserRole::Admin, &post, path),
                "Admin must still be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_user_still_limited_to_message_endpoints() {
        let post = axum::http::Method::POST;
        assert!(user_role_allows_request(
            UserRole::User,
            &post,
            "/api/agents/123/message"
        ));
        // Users still can't touch spawn, skill install, or config.
        for path in ["/api/agents", "/api/skills/install", "/api/config/set"] {
            assert!(
                !user_role_allows_request(UserRole::User, &post, path),
                "User must NOT be allowed to POST {path}"
            );
        }
    }

    #[test]
    fn test_user_role_viewer_still_get_only() {
        let get = axum::http::Method::GET;
        let post = axum::http::Method::POST;
        assert!(user_role_allows_request(
            UserRole::Viewer,
            &get,
            "/api/agents"
        ));
        assert!(!user_role_allows_request(
            UserRole::Viewer,
            &post,
            "/api/agents/123/message"
        ));
    }

    #[tokio::test]
    async fn test_api_version_header_prefers_explicit_path_version() {
        let app = Router::new()
            .route("/api/v1/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .header("accept", "application/vnd.librefang.v99+json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_api_version_header_rejects_unknown_vendor_version_on_alias() {
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("accept", "application/vnd.librefang.v99+json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    }

    #[tokio::test]
    async fn test_api_version_header_accepts_vendor_media_type_with_parameters() {
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("accept", "application/vnd.librefang.v1+json; charset=utf-8")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_api_version_header_ignores_non_json_vendor_media_type() {
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("accept", "application/vnd.librefang.v1+xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_api_version_header_is_added_to_unauthorized_responses() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route("/api/private", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth))
            .layer(axum::middleware::from_fn(api_version_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/private")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["x-api-version"], "v1");
    }

    #[tokio::test]
    async fn test_user_api_key_can_post_agent_messages() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
            }]),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route(
                "/api/agents/123/message",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents/123/message")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_user_api_key_cannot_spawn_agents() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
            }]),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route(
                "/api/agents",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_viewer_api_key_cannot_post_anything() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![ApiUserAuth {
                name: "ReadOnly".to_string(),
                role: UserRole::Viewer,
                api_key_hash: crate::password_hash::hash_password("viewer-key").unwrap(),
            }]),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route(
                "/api/agents/123/message",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents/123/message")
                    .header("authorization", "Bearer viewer-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_viewer_api_key_can_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![ApiUserAuth {
                name: "ReadOnly".to_string(),
                role: UserRole::Viewer,
                api_key_hash: crate::password_hash::hash_password("viewer-key").unwrap(),
            }]),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route("/api/budget", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/budget")
                    .header("authorization", "Bearer viewer-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_trailing_slash_does_not_bypass_acl() {
        // Verify that a User-role key trying to POST /api/agents/ (with
        // trailing slash) still gets FORBIDDEN, not allowed through because
        // the path normalization strips the slash before the ACL check.
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
            }]),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route(
                "/api/agents",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .route(
                "/api/agents/",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents/")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // After normalization "/api/agents/" → "/api/agents", which User
        // role is not allowed to POST to → FORBIDDEN.
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// Regression for #2305: GET / must stay public. Earlier path
    /// normalization stripped the trailing slash from "/" producing an
    /// empty string, so the `path == "/"` public-endpoint check missed
    /// and the dashboard HTML returned 401 instead of the SPA.
    #[tokio::test]
    async fn test_root_path_is_public_even_with_api_key_set() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("somekey".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![]),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route("/", get(|| async { "dashboard html" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET / must serve the dashboard HTML without auth so the SPA can render"
        );
    }

    #[tokio::test]
    async fn test_forbidden_response_has_json_content_type() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
            }]),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route(
                "/api/agents",
                get(|| async { "ok" }).post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/agents")
                    .header("authorization", "Bearer user-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["content-type"], "application/json");
    }

    /// With an api_key configured and `require_auth_for_reads = true`,
    /// GET /api/agents must stop being public — otherwise a remote caller
    /// on a 0.0.0.0 listener can enumerate agents without a token.
    #[tokio::test]
    async fn test_require_auth_for_reads_blocks_unauthenticated_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: true,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "require_auth_for_reads=true must make dashboard read endpoints \
             require a bearer token"
        );
    }

    /// With `require_auth_for_reads = true` the correct bearer still goes
    /// through, so legitimate dashboard clients keep working.
    #[tokio::test]
    async fn test_require_auth_for_reads_allows_authenticated_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: true,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// `/api/health` must stay reachable without a token even when
    /// `require_auth_for_reads = true` so probes, load balancers, and
    /// orchestrators can keep working.
    #[tokio::test]
    async fn test_require_auth_for_reads_keeps_health_public() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: true,
        };
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Default (flag off) behaviour must be preserved bit-for-bit: an
    /// unauthenticated GET /api/agents still succeeds so existing
    /// dashboards keep rendering.
    #[tokio::test]
    async fn test_require_auth_for_reads_off_preserves_public_get() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: false,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// `/api/health/detail`'s own doc comment says "requires auth" and its
    /// payload includes panic counts, agent counts, model IDs, and
    /// `config_warnings` from `KernelConfig::validate()`. Unlike the
    /// dashboard-read group, this endpoint requires auth **unconditionally**
    /// — even when `require_auth_for_reads` is off — because its handler
    /// doc contract said so all along and the middleware was just wrong.
    /// `/api/health` stays public either way for load balancers.
    #[tokio::test]
    async fn test_api_health_detail_always_requires_auth() {
        // Flag OFF: /api/health is still public, /api/health/detail still
        // requires auth. This is the contract fix — it used to be in the
        // always-public set.
        let auth_state_off = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: false,
        };
        let app_off = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .route("/api/health/detail", get(|| async { "detail" }))
            .layer(axum::middleware::from_fn_with_state(auth_state_off, auth));

        let health = app_off
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            health.status(),
            StatusCode::OK,
            "/api/health must stay public regardless of the flag"
        );

        let detail = app_off
            .oneshot(
                Request::builder()
                    .uri("/api/health/detail")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            detail.status(),
            StatusCode::UNAUTHORIZED,
            "/api/health/detail must require auth even when the flag is off — \
             its doc comment has always said so"
        );

        // Flag ON: contract unchanged.
        let auth_state_on = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: true,
        };
        let app_on = Router::new()
            .route("/api/health/detail", get(|| async { "detail" }))
            .layer(axum::middleware::from_fn_with_state(auth_state_on, auth));

        let detail = app_on
            .oneshot(
                Request::builder()
                    .uri("/api/health/detail")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail.status(), StatusCode::UNAUTHORIZED);
    }

    /// `/api/status` used to be in the always-public set, but its handler
    /// returns the full agents listing + home_dir + api_listen — exactly
    /// the enumeration surface the flag exists to close. It must be locked
    /// down when the flag is on.
    #[tokio::test]
    async fn test_require_auth_for_reads_blocks_api_status() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: true,
        };
        let app = Router::new()
            .route("/api/status", get(|| async { "status" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "/api/status leaks the agent list; must require auth when the flag is on"
        );
    }

    /// The flag must gate on any configured auth method, not just `api_key`.
    /// An operator with only per-user API keys (and empty `api_key`) must
    /// still get dashboard reads locked down when they enable the flag —
    /// gating on `api_key_present` alone would silently no-op here.
    #[tokio::test]
    async fn test_require_auth_for_reads_engages_with_user_api_keys_only() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(vec![ApiUserAuth {
                name: "alice".into(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("alice-key").unwrap(),
            }]),
            require_auth_for_reads: true,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        // Unauthenticated → must be rejected.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "flag must engage when auth is configured via user_api_keys alone"
        );

        // Valid per-user key → must succeed.
        let ok = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .header("authorization", "Bearer alice-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }

    /// Flag is set but no auth of any kind is configured → must not
    /// accidentally start returning 401 for unauthenticated reads. The
    /// startup warning in server.rs covers operator-visible feedback; the
    /// middleware preserves the open-development default.
    #[tokio::test]
    async fn test_require_auth_for_reads_is_noop_without_any_auth() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(Vec::new()),
            require_auth_for_reads: true,
        };
        let app = Router::new()
            .route("/api/agents", get(|| async { "agents listing" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "flag must not block unauthenticated reads when no auth is configured — \
             the startup warning handles operator feedback"
        );
    }
}
