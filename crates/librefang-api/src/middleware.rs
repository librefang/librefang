//! Production middleware for the LibreFang API server.
//!
//! Provides:
//! - Request ID generation and propagation
//! - Per-endpoint structured request logging
//! - HTTP metrics recording (when telemetry feature is enabled)
//! - In-memory rate limiting (per IP)
//! - Accept-Language header parsing for i18n error responses
//! - Account identity extraction from X-Account-Id header
//! - Account signature verification (HMAC-SHA256)

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::middleware::Next;
use librefang_types::i18n;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

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
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let api_key = auth_state.api_key_lock.read().await.clone();
    // SECURITY: Capture method early for method-aware public endpoint checks.
    let method = request.method().clone();

    // Shutdown is loopback-only (CLI on same machine) — skip token auth.
    // Normalize versioned paths: /api/v1/foo → /api/foo so public endpoint
    // checks work identically for both /api/ and /api/v1/ prefixes.
    let raw_path = request.uri().path().to_string();
    let normalized;
    let path: &str = if raw_path.starts_with("/api/v1/") {
        normalized = format!("/api{}", &raw_path[7..]);
        normalized.as_str()
    } else if raw_path == "/api/v1" {
        "/api"
    } else {
        &raw_path
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
    let is_public = path == "/"
        || path == "/logo.png"
        || path == "/favicon.ico"
        || (path.starts_with("/dashboard/") && is_get)
        || (path == "/.well-known/agent.json" && is_get)
        || (path.starts_with("/a2a/") && is_get)
        || path == "/api/versions"
        || path == "/api/health"
        || path == "/api/health/detail"
        || path == "/api/status"
        || path == "/api/version"
        || (path == "/api/agents" && is_get)
        || (path == "/api/profiles" && is_get)
        || (path == "/api/config" && is_get)
        || (path == "/api/config/schema" && is_get)
        // SECURITY: /api/uploads/* removed from public endpoints — uploads
        // require authentication to prevent unauthorized file access (H1 fix).
        // Dashboard read endpoints — allow unauthenticated so the SPA can
        // render before the user enters their API key.
        || (path == "/api/models" && is_get)
        || (path == "/api/models/aliases" && is_get)
        || (path == "/api/providers" && is_get)
        || (path == "/api/budget" && is_get)
        || (path == "/api/budget/agents" && is_get)
        || (path.starts_with("/api/budget/agents/") && is_get)
        || (path == "/api/network/status" && is_get)
        || (path == "/api/a2a/agents" && is_get)
        || (path == "/api/approvals" && is_get)
        || (path.starts_with("/api/approvals/") && is_get)
        || (path == "/api/channels" && is_get)
        || (path == "/api/hands" && is_get)
        || (path == "/api/hands/active" && is_get)
        || (path.starts_with("/api/hands/") && is_get)
        || (path == "/api/skills" && is_get)
        || (path == "/api/sessions" && is_get)
        || (path == "/api/integrations" && is_get)
        || (path == "/api/integrations/available" && is_get)
        || (path == "/api/integrations/health" && is_get)
        || (path == "/api/workflows" && is_get)
        || path == "/api/logs/stream"  // SSE stream, read-only
        || (path.starts_with("/api/cron/") && is_get)
        || path.starts_with("/api/providers/github-copilot/oauth/")
        // OAuth/OIDC auth flow endpoints must be accessible without API key
        // (they are the authentication entry points themselves).
        || (path == "/api/auth/providers" && is_get)
        || (path.starts_with("/api/auth/login") && is_get)
        || path == "/api/auth/callback"
        || path == "/api/auth/dashboard-login"
        || path == "/api/auth/dashboard-check";

    if is_public {
        return next.run(request).await;
    }

    // If no API key configured (empty, whitespace-only, or missing), skip auth
    // entirely. Users who don't set api_key accept that all endpoints are open.
    // To secure the dashboard, set a non-empty api_key in config.toml.
    let api_key = api_key.trim();
    if api_key.is_empty() {
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

// ---------------------------------------------------------------------------
// Multi-tenant HMAC account signature verification (with replay protection)
// ---------------------------------------------------------------------------

/// Result of verifying an account HMAC signature.
#[derive(Debug, PartialEq)]
pub enum AccountSigResult {
    /// Signature is valid (new format with timestamp replay protection).
    Valid,
    /// Signature is valid using the legacy format (account_id only, no timestamp).
    ValidLegacy,
    /// No `X-Account-Id` header present — skip account-level auth.
    NoHeader,
    /// Timestamp is missing but signature does not match legacy format either.
    InvalidSignature,
    /// Timestamp is too old or too far in the future.
    SignatureExpired,
}

/// Maximum allowed clock skew into the future (seconds).
const HMAC_MAX_FUTURE_SECS: u64 = 60;

/// Verify an HMAC-signed account request.
///
/// **New header contract (with replay protection):**
/// - `X-Account-Id: <account_id>`
/// - `X-Account-Timestamp: <unix_epoch_seconds>`
/// - `X-Account-Sig: <hex_hmac>`
///
/// Signature input: `HMAC-SHA256(secret, "{account_id}\n{method}\n{path}\n{timestamp}")`
///
/// **Legacy fallback** (when `X-Account-Timestamp` is absent):
/// Signature input: `HMAC-SHA256(secret, account_id)`
pub fn verify_account_signature(
    secret: &str,
    account_id: &str,
    method: &str,
    path: &str,
    timestamp: Option<&str>,
    signature_hex: &str,
    hmac_max_age_secs: u64,
) -> AccountSigResult {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    // Try new format first if timestamp is provided.
    if let Some(ts_str) = timestamp {
        let ts: u64 = match ts_str.parse() {
            Ok(v) => v,
            Err(_) => return AccountSigResult::InvalidSignature,
        };

        // Staleness check
        if hmac_max_age_secs > 0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if ts + hmac_max_age_secs < now {
                return AccountSigResult::SignatureExpired;
            }
            if ts > now + HMAC_MAX_FUTURE_SECS {
                return AccountSigResult::SignatureExpired;
            }
        }

        // Build message: "{account_id}\n{method}\n{path}\n{timestamp}"
        let message = format!("{}\n{}\n{}\n{}", account_id, method, path, ts_str);
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
        mac.update(message.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());

        if constant_time_eq(expected.as_bytes(), signature_hex.as_bytes()) {
            return AccountSigResult::Valid;
        }

        // New-format signature didn't match — do NOT fall back to legacy
        // when a timestamp was explicitly provided.
        return AccountSigResult::InvalidSignature;
    }

    // Legacy fallback: signature = HMAC-SHA256(secret, account_id)
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(account_id.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    if constant_time_eq(expected.as_bytes(), signature_hex.as_bytes()) {
        warn!(
            account_id = %account_id,
            "Legacy HMAC signature format used (no timestamp). \
             Migrate to include X-Account-Timestamp for replay protection."
        );
        return AccountSigResult::ValidLegacy;
    }

    AccountSigResult::InvalidSignature
}

/// Constant-time byte-slice equality to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && a.ct_eq(b).into()
}

/// Extract and verify multi-tenant account identity from request headers.
///
/// Reads `X-Account-Id`, `X-Account-Timestamp` (optional), and `X-Account-Sig`.
/// Returns `Ok(Some(account_id))` if valid, `Ok(None)` if no account headers,
/// or `Err(Response)` with 401 error.
pub fn extract_verified_account(
    headers: &axum::http::HeaderMap,
    method: &str,
    path: &str,
    hmac_secret: &str,
    hmac_max_age_secs: u64,
) -> Result<Option<String>, Response<Body>> {
    let account_id = match headers.get("x-account-id").and_then(|v| v.to_str().ok()) {
        Some(id) => id,
        None => return Ok(None),
    };

    let signature = match headers.get("x-account-sig").and_then(|v| v.to_str().ok()) {
        Some(sig) => sig,
        None => {
            return Err(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"error":"Missing X-Account-Sig header"}"#))
                .unwrap_or_default());
        }
    };

    let timestamp = headers
        .get("x-account-timestamp")
        .and_then(|v| v.to_str().ok());

    match verify_account_signature(
        hmac_secret,
        account_id,
        method,
        path,
        timestamp,
        signature,
        hmac_max_age_secs,
    ) {
        AccountSigResult::Valid | AccountSigResult::ValidLegacy => Ok(Some(account_id.to_string())),
        AccountSigResult::NoHeader => Ok(None),
        AccountSigResult::SignatureExpired => Err(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"error":"Signature expired"}"#))
            .unwrap_or_default()),
        AccountSigResult::InvalidSignature => Err(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"error":"Invalid account signature"}"#))
            .unwrap_or_default()),
    }
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

    // -----------------------------------------------------------------------
    // HMAC account signature verification tests (replay protection)
    // -----------------------------------------------------------------------

    /// Helper: compute HMAC-SHA256 and return hex string.
    fn compute_hmac(secret: &str, message: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Helper: current unix timestamp.
    fn now_unix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn test_hmac_with_timestamp_valid() {
        let secret = "test-secret-key";
        let account_id = "acct_123";
        let method = "GET";
        let path = "/api/agents";
        let ts = now_unix().to_string();
        let message = format!("{}\n{}\n{}\n{}", account_id, method, path, ts);
        let sig = compute_hmac(secret, &message);

        let result =
            verify_account_signature(secret, account_id, method, path, Some(&ts), &sig, 300);
        assert_eq!(result, AccountSigResult::Valid);
    }

    #[test]
    fn test_hmac_with_timestamp_expired() {
        let secret = "test-secret-key";
        let account_id = "acct_123";
        let method = "GET";
        let path = "/api/agents";
        let ts = (now_unix() - 600).to_string();
        let message = format!("{}\n{}\n{}\n{}", account_id, method, path, ts);
        let sig = compute_hmac(secret, &message);

        let result =
            verify_account_signature(secret, account_id, method, path, Some(&ts), &sig, 300);
        assert_eq!(result, AccountSigResult::SignatureExpired);
    }

    #[test]
    fn test_hmac_with_timestamp_future() {
        let secret = "test-secret-key";
        let account_id = "acct_123";
        let method = "POST";
        let path = "/api/agents";
        let ts = (now_unix() + 120).to_string();
        let message = format!("{}\n{}\n{}\n{}", account_id, method, path, ts);
        let sig = compute_hmac(secret, &message);

        let result =
            verify_account_signature(secret, account_id, method, path, Some(&ts), &sig, 300);
        assert_eq!(result, AccountSigResult::SignatureExpired);
    }

    #[test]
    fn test_hmac_legacy_still_works() {
        let secret = "test-secret-key";
        let account_id = "acct_legacy";
        let sig = compute_hmac(secret, account_id);

        let result =
            verify_account_signature(secret, account_id, "GET", "/api/agents", None, &sig, 300);
        assert_eq!(result, AccountSigResult::ValidLegacy);
    }

    #[test]
    fn test_hmac_replay_different_path() {
        let secret = "test-secret-key";
        let account_id = "acct_123";
        let method = "GET";
        let original_path = "/api/agents";
        let ts = now_unix().to_string();
        let message = format!("{}\n{}\n{}\n{}", account_id, method, original_path, ts);
        let sig = compute_hmac(secret, &message);

        let result = verify_account_signature(
            secret,
            account_id,
            method,
            "/api/config",
            Some(&ts),
            &sig,
            300,
        );
        assert_eq!(result, AccountSigResult::InvalidSignature);
    }

    #[test]
    fn test_hmac_replay_different_method() {
        let secret = "test-secret-key";
        let account_id = "acct_123";
        let path = "/api/agents";
        let ts = now_unix().to_string();
        let message = format!("{}\n{}\n{}\n{}", account_id, "GET", path, ts);
        let sig = compute_hmac(secret, &message);

        let result =
            verify_account_signature(secret, account_id, "POST", path, Some(&ts), &sig, 300);
        assert_eq!(result, AccountSigResult::InvalidSignature);
    }

    #[test]
    fn test_hmac_invalid_timestamp_format() {
        let result = verify_account_signature(
            "test-secret",
            "acct_123",
            "GET",
            "/api/agents",
            Some("not-a-number"),
            "deadbeef",
            300,
        );
        assert_eq!(result, AccountSigResult::InvalidSignature);
    }

    #[test]
    fn test_hmac_wrong_secret() {
        let account_id = "acct_123";
        let method = "GET";
        let path = "/api/agents";
        let ts = now_unix().to_string();
        let message = format!("{}\n{}\n{}\n{}", account_id, method, path, ts);
        let sig = compute_hmac("correct-secret", &message);

        let result = verify_account_signature(
            "wrong-secret",
            account_id,
            method,
            path,
            Some(&ts),
            &sig,
            300,
        );
        assert_eq!(result, AccountSigResult::InvalidSignature);
    }

    #[test]
    fn test_extract_verified_account_no_headers() {
        let headers = axum::http::HeaderMap::new();
        let result = extract_verified_account(&headers, "GET", "/api/agents", "secret", 300);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_extract_verified_account_missing_sig() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-account-id", "acct_123".parse().unwrap());
        let result = extract_verified_account(&headers, "GET", "/api/agents", "secret", 300);
        assert!(result.is_err());
    }
}

// ── Account Management ────────────────────────────────────────────────────

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Account ID extracted from the `X-Account-Id` HTTP header.
///
/// This is the **axum extractor** version of the account ID, local to the
/// api crate so it can implement `FromRequestParts` (orphan rule requires
/// the type to be crate-local for foreign trait impls).
///
/// The canonical domain model lives at `librefang_types::account::AccountId`.
/// Both types are structurally identical (`pub Option<String>`). Use the
/// `From` impls below to convert between them at crate boundaries.
///
/// - `AccountId(Some(...))` = scoped multi-tenant request
/// - `AccountId(None)` = legacy / admin / system mode
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Option<String>);

impl From<AccountId> for librefang_types::account::AccountId {
    fn from(val: AccountId) -> Self {
        librefang_types::account::AccountId(val.0)
    }
}

impl From<librefang_types::account::AccountId> for AccountId {
    fn from(val: librefang_types::account::AccountId) -> Self {
        AccountId(val.0)
    }
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for AccountId {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let account = parts
            .headers
            .get("x-account-id")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        std::future::ready(Ok(AccountId(account)))
    }
}

/// Verify that HMAC-SHA256(secret, account_id) matches the provided hex signature.
/// Uses constant-time comparison via `hmac::Mac::verify_slice`.
///
/// # Security notes
///
/// - `hex::decode` is NOT constant-time: it will return early on invalid hex
///   characters. This leaks whether the input was valid hex, but does NOT
///   leak any bits of the expected signature. Acceptable trade-off: an
///   attacker already knows the expected format is hex.
/// - `verify_slice` internally uses `subtle::ConstantTimeEq`, so the actual
///   HMAC comparison is timing-safe.
pub fn verify_account_sig(secret: &str, account_id: &str, sig_hex: &str) -> bool {
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(account_id.as_bytes());
    mac.verify_slice(&sig_bytes).is_ok()
}

/// Middleware: verify `X-Account-Sig` HMAC-SHA256 signature when a shared
/// secret is configured in `AppState.account_sig_secret`.
///
/// Behaviour:
/// - No secret configured (`None`) -> pass through unconditionally.
/// - Secret configured but no `X-Account-Id` header -> pass through (legacy /
///   admin requests are allowed; `require_account_id` can gate that separately).
/// - Secret + `X-Account-Id` present, but missing / invalid `X-Account-Sig`
///   -> reject with 401 JSON error.
/// - Secret + valid HMAC -> pass through.
///
/// Must run AFTER the `auth` middleware (so legitimate auth has already been
/// checked) and BEFORE route handlers.
pub async fn account_sig_check(
    axum::extract::State(state): axum::extract::State<Arc<crate::routes::AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let secret = match state.account_sig_secret.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => return next.run(request).await,
    };

    let account_id = request
        .headers()
        .get("x-account-id")
        .and_then(|v| v.to_str().ok());
    let sig = request
        .headers()
        .get("x-account-sig")
        .and_then(|v| v.to_str().ok());

    if let Some(err_msg) = account_sig_policy(Some(secret), account_id, sig) {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"error":"{}"}}"#, err_msg)))
            .unwrap();
    }

    next.run(request).await
}

/// Core account signature policy check -- pure function, fully testable.
///
/// Returns None if the request should pass through, or Some(error_message)
/// if it must be rejected with 401.
///
/// Policy matrix:
/// | secret | account_id | sig     | Result                          |
/// |--------|------------|---------|----------------------------------|
/// | absent | any        | any     | None (pass)                      |
/// | any    | absent     | any     | None (pass)                      |
/// | present| present    | absent  | Some("Missing X-Account-Sig ...") |
/// | present| present    | invalid | Some("Invalid account signature") |
/// | present| present    | valid   | None (pass)                      |
pub(crate) fn account_sig_policy(
    secret: Option<&str>,
    account_id: Option<&str>,
    sig: Option<&str>,
) -> Option<&'static str> {
    let secret = secret?;
    let account_id = account_id?;
    match sig {
        None => Some("Missing X-Account-Sig header"),
        Some(s) if !verify_account_sig(secret, account_id, s) => Some("Invalid account signature"),
        Some(_) => None,
    }
}

/// Middleware: require `X-Account-Id` header in multi-tenant mode.
///
/// When multi-tenant isolation is enabled, requests without a valid
/// `X-Account-Id` header must be rejected to prevent `AccountId(None)` from
/// bypassing all tenant isolation (the `check_account()` function passes
/// everything through when the account ID is `None`).
///
/// This middleware is **opt-in** -- add it to the middleware stack only when
/// `config.multi_tenant.enabled = true`:
///
/// ```ignore
/// .layer(axum::middleware::from_fn(require_account_id))
/// ```
///
/// A small set of infrastructure endpoints (health, version, OpenAPI spec)
/// are exempt because they are not tenant-scoped.
pub async fn require_account_id(request: Request<Body>, next: Next) -> Response<Body> {
    let path = request.uri().path();

    // Allow health/version/public/auth endpoints without account ID.
    // Auth endpoints are exempt because the user is in the process of
    // authenticating and cannot yet carry an X-Account-Id header.
    let is_exempt = path == "/api/health"
        || path == "/api/version"
        || path.starts_with("/api/auth/")
        || path.starts_with("/api/v1/auth/")
        || path == "/openapi.json";

    if !is_exempt {
        let has_account = request
            .headers()
            .get("x-account-id")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.trim().is_empty())
            .is_some();

        if !has_account {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"error":"X-Account-Id header required in multi-tenant mode"}"#,
                ))
                .unwrap();
        }
    }

    next.run(request).await
}

#[cfg(test)]
mod account_tests {
    use super::*;
    use axum::http::request::Parts;

    fn make_parts_with_header(name: &str, value: &str) -> Parts {
        let req = axum::http::Request::builder()
            .method(axum::http::Method::GET)
            .uri("/")
            .header(name, value)
            .body(())
            .unwrap();
        req.into_parts().0
    }

    fn make_parts() -> Parts {
        let req = axum::http::Request::builder()
            .method(axum::http::Method::GET)
            .uri("/")
            .body(())
            .unwrap();
        req.into_parts().0
    }

    // ─── Header Extraction (4 tests) ───

    #[tokio::test]
    async fn test_account_id_from_x_account_id_header() {
        let mut parts = make_parts_with_header("x-account-id", "tenant-123");
        let account =
            <AccountId as axum::extract::FromRequestParts<()>>::from_request_parts(&mut parts, &())
                .await
                .unwrap();
        assert_eq!(account, AccountId(Some("tenant-123".to_string())));
    }

    #[tokio::test]
    async fn test_account_id_filters_empty_header() {
        let mut parts = make_parts_with_header("x-account-id", "");
        let account =
            <AccountId as axum::extract::FromRequestParts<()>>::from_request_parts(&mut parts, &())
                .await
                .unwrap();
        assert_eq!(account, AccountId(None));
    }

    #[tokio::test]
    async fn test_account_id_filters_whitespace_header() {
        let mut parts = make_parts_with_header("x-account-id", "   ");
        let account =
            <AccountId as axum::extract::FromRequestParts<()>>::from_request_parts(&mut parts, &())
                .await
                .unwrap();
        assert_eq!(account, AccountId(None));
    }

    #[tokio::test]
    async fn test_account_id_defaults_to_none_when_absent() {
        let mut parts = make_parts();
        let account =
            <AccountId as axum::extract::FromRequestParts<()>>::from_request_parts(&mut parts, &())
                .await
                .unwrap();
        assert_eq!(account, AccountId(None));
    }

    // ─── Signature Verification (7 tests) ───

    #[test]
    fn test_verify_account_sig_valid_hmac() {
        let secret = "my-secret";
        let account_id = "tenant-123";
        let sig_hex = {
            let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(account_id.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };
        assert!(verify_account_sig(secret, account_id, &sig_hex));
    }

    #[test]
    fn test_verify_account_sig_invalid_hmac() {
        assert!(!verify_account_sig("secret", "tenant-123", "deadbeef"));
    }

    #[test]
    fn test_verify_account_sig_malformed_hex() {
        assert!(!verify_account_sig("secret", "tenant-123", "not-hex"));
    }

    #[test]
    fn test_verify_account_sig_empty_hex() {
        assert!(!verify_account_sig("secret", "tenant-123", ""));
    }

    #[test]
    fn test_account_sig_policy_passes_when_no_secret() {
        assert_eq!(account_sig_policy(None, Some("tenant-123"), None), None);
    }

    #[test]
    fn test_account_sig_policy_passes_when_no_account_id() {
        assert_eq!(account_sig_policy(Some("secret"), None, Some("sig")), None);
    }

    #[test]
    fn test_account_sig_policy_requires_sig_when_secret_and_account_present() {
        assert_eq!(
            account_sig_policy(Some("secret"), Some("tenant-123"), None),
            Some("Missing X-Account-Sig header")
        );
    }

    #[test]
    fn test_account_sig_policy_rejects_invalid_sig() {
        assert_eq!(
            account_sig_policy(Some("secret"), Some("tenant-123"), Some("invalid")),
            Some("Invalid account signature")
        );
    }

    #[test]
    fn test_account_sig_policy_accepts_valid_sig() {
        let secret = "my-secret";
        let account_id = "tenant-123";
        let sig_hex = {
            let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(account_id.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };
        assert_eq!(
            account_sig_policy(Some(secret), Some(account_id), Some(&sig_hex)),
            None
        );
    }

    #[test]
    fn test_account_sig_policy_matrix_row_1_no_secret() {
        // Row 1: no secret configured -> always pass regardless of other headers
        assert_eq!(
            account_sig_policy(None, Some("acc-123"), Some("any-sig")),
            None
        );
        assert_eq!(account_sig_policy(None, Some("acc-123"), None), None);
        assert_eq!(account_sig_policy(None, None, None), None);
    }

    #[test]
    fn test_account_sig_policy_matrix_row_2_no_account_id() {
        // Row 2: no account_id header -> pass regardless of secret/sig
        assert_eq!(
            account_sig_policy(Some("secret"), None, Some("any-sig")),
            None
        );
        assert_eq!(account_sig_policy(Some("secret"), None, None), None);
    }

    // ─── From conversion between middleware and types-crate AccountId ───

    #[test]
    fn test_account_id_roundtrip_to_types_crate() {
        let middleware_id = AccountId(Some("tenant-42".to_string()));
        let types_id: librefang_types::account::AccountId = middleware_id.clone().into();
        assert_eq!(types_id.0, Some("tenant-42".to_string()));
        let back: AccountId = types_id.into();
        assert_eq!(back, middleware_id);
    }

    #[test]
    fn test_account_id_none_roundtrip_to_types_crate() {
        let middleware_id = AccountId(None);
        let types_id: librefang_types::account::AccountId = middleware_id.clone().into();
        assert_eq!(types_id.0, None);
        let back: AccountId = types_id.into();
        assert_eq!(back, middleware_id);
    }

    // ─── require_account_id middleware (multi-tenant gate) ───

    #[tokio::test]
    async fn test_require_account_id_rejects_when_multi_tenant_enabled() {
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let app = Router::new()
            .route("/api/agents", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(require_account_id));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_require_account_id_allows_when_multi_tenant_disabled() {
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let app = Router::new().route("/api/agents", get(|| async { "ok" }));

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

    #[tokio::test]
    async fn test_require_account_id_passes_with_header() {
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let app = Router::new()
            .route("/api/agents", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(require_account_id));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/agents")
                    .header("x-account-id", "tenant-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_require_account_id_exempts_health_endpoint() {
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(require_account_id));

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

    #[tokio::test]
    async fn test_require_account_id_rejects_upload_without_header() {
        use axum::routing::get;
        use axum::Router;
        use tower::ServiceExt;

        let app = Router::new()
            .route("/api/uploads/some-file-id", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(require_account_id));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/uploads/some-file-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Upload endpoints are NOT exempt — they require X-Account-Id
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

// ── account_sig_check middleware integration tests ────────────────────────

#[cfg(test)]
mod account_sig_check_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Build a minimal `AppState` with an optional HMAC secret.
    ///
    /// Boots a real (but lightweight) kernel so the `AppState` struct is fully
    /// populated.  Tests that only touch `account_sig_secret` are unaffected
    /// by the kernel's internal state.
    fn test_app_state(secret: Option<String>) -> (Arc<crate::routes::AppState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("sig-test");
        std::fs::create_dir_all(&home).unwrap();
        let config = librefang_types::config::KernelConfig {
            home_dir: home.clone(),
            data_dir: home.join("data"),
            ..Default::default()
        };
        let kernel = Arc::new(librefang_kernel::LibreFangKernel::boot_with_config(config).unwrap());

        let state = Arc::new(crate::routes::AppState {
            kernel,
            started_at: std::time::Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            skillhub_cache: dashmap::DashMap::new(),
            provider_probe_cache: librefang_runtime::provider_health::ProbeCache::new(),
            provider_test_cache: dashmap::DashMap::new(),
            webhook_store: crate::webhook_store::WebhookStore::load(home.join("webhooks.json")),
            active_sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            #[cfg(feature = "telemetry")]
            prometheus_handle: None,
            media_drivers: librefang_runtime::media::MediaDriverCache::new(),
            webhook_router: Arc::new(tokio::sync::RwLock::new(Arc::new(Router::new()))),
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            account_sig_secret: secret,
        });
        (state, tmp)
    }

    /// Compute a valid HMAC-SHA256 hex signature for testing.
    fn sign(secret: &str, account_id: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(account_id.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sig_check_passes_when_no_secret_configured() {
        let (state, _tmp) = test_app_state(None);
        let app = Router::new().route("/test", get(|| async { "ok" })).layer(
            axum::middleware::from_fn_with_state(state, account_sig_check),
        );

        // No secret configured -- any request should pass, even with a random
        // X-Account-Id and no sig.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("x-account-id", "tenant-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sig_check_rejects_missing_sig_when_secret_set() {
        let (state, _tmp) = test_app_state(Some("my-secret".into()));
        let app = Router::new().route("/test", get(|| async { "ok" })).layer(
            axum::middleware::from_fn_with_state(state, account_sig_check),
        );

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("x-account-id", "tenant-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "Missing X-Account-Sig header");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sig_check_accepts_valid_hmac() {
        let secret = "test-secret-key";
        let account = "tenant-42";
        let sig = sign(secret, account);

        let (state, _tmp) = test_app_state(Some(secret.into()));
        let app = Router::new().route("/test", get(|| async { "ok" })).layer(
            axum::middleware::from_fn_with_state(state, account_sig_check),
        );

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("x-account-id", account)
                    .header("x-account-sig", sig)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }
}
