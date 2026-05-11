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
// Re-export `UserRole` through the api-layer auth boundary so that route
// modules (and tests) don't need to reach into `librefang_kernel::auth`
// directly. This keeps the `librefang-api` <-> `librefang-kernel` import
// surface narrow per issue #3744 — the underlying type still lives in the
// kernel; only the import path is centralized here.
pub use librefang_kernel::auth::UserRole;
use librefang_types::agent::UserId;
use librefang_types::i18n;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn, Instrument};

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
    ///
    /// Wrapped in a `RwLock` (mirroring `api_key_lock`) so the rotate-key
    /// endpoint can swap the in-memory snapshot atomically. Without a live
    /// swap, a leaked per-user bearer token could only be revoked by
    /// restarting the daemon — defeating the point of rotation.
    pub user_api_keys: Arc<tokio::sync::RwLock<Vec<ApiUserAuth>>>,
    /// When `true` and an `api_key` is configured, GET endpoints that are
    /// otherwise on the dashboard public-read allowlist (agents, config,
    /// budget, sessions, approvals, hands, skills, workflows, …) are forced
    /// through bearer authentication. Static assets, OAuth entry points, and
    /// `/api/health*` remain public so the daemon stays probeable.
    pub require_auth_for_reads: bool,
    /// Set from `LIBREFANG_ALLOW_NO_AUTH=1` to permit running without an
    /// api_key on a non-loopback bind. Off by default so empty keys
    /// fail closed for LAN/public origins (see issue #1034 port).
    pub allow_no_auth: bool,
    /// RBAC M5: optional handle to the kernel's audit log so the
    /// middleware can record `PermissionDenied` events when a request is
    /// rejected by the role gate. Wrapped in `Option` because some test
    /// harnesses construct `AuthState` without a kernel attached.
    pub audit_log: Option<Arc<librefang_kernel::audit::AuditLog>>,
}

#[derive(Clone)]
pub struct ApiUserAuth {
    pub name: String,
    pub role: UserRole,
    pub api_key_hash: String,
    /// Stable LibreFang user id derived from `name` via [`UserId::from_name`].
    /// Pre-computed at config-load so the auth middleware does not need a
    /// kernel handle to identify the caller.
    pub user_id: UserId,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedApiUser {
    pub name: String,
    pub role: UserRole,
    /// Same id stored on [`ApiUserAuth`]; downstream handlers read this
    /// from request extensions to pass the caller through to kernel
    /// `authorize()` calls and into [`librefang_kernel::audit::AuditEntry`].
    pub user_id: UserId,
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
    if matches!(
        path,
        "/api/config"
            | "/api/config/set"
            | "/api/config/reload"
            | "/api/auth/change-password"
            | "/api/shutdown"
            // #3621: TOTP enrollment is an Owner-equivalent action — a
            // confirmed enrollment hands the holder approve power for every
            // privileged tool call, so any non-Owner bearer token must not
            // be able to start, confirm, or revoke the enrollment.
            | "/api/approvals/totp/setup"
            | "/api/approvals/totp/confirm"
            | "/api/approvals/totp/revoke"
            // A2A discover registers a remote agent into the pending registry,
            // expanding the trust surface; restrict to Owner so non-Owner API keys
            // cannot fill the registry or stage impersonation attempts (#3483).
            | "/api/a2a/discover"
    ) {
        return true;
    }
    // RBAC user-management surface (M6) — every mutating call under
    // `/api/users*` (create / replace / delete / bulk import) maps to
    // `Action::ManageUsers` in the kernel, which requires `Owner`. We
    // match by prefix because the path can be `/api/users`,
    // `/api/users/{name}`, or `/api/users/import`. GET is left to the
    // generic Admin-or-above gate so the dashboard's user list and
    // permission simulator stay usable for Admins.
    if path == "/api/users" || path.starts_with("/api/users/") {
        return true;
    }
    false
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
            || path.ends_with("/approve_all")
            || path.ends_with("/reject")
            || path.ends_with("/reject_all")
            || path.ends_with("/modify");
        return agent_message || agent_clone || approval_action;
    }

    false
}

/// Pull a caller-provided token from the standard locations the auth path
/// understands. Precedence (matches the non-loopback flow at `auth(...)`):
///   1. `Authorization: Bearer <x>`
///   2. `X-API-Key: <x>`
///   3. `Sec-WebSocket-Protocol: bearer.<x>` — WS upgrade fallback.
///      Browsers cannot set custom headers on the WebSocket handshake, so
///      the dashboard encodes the token as a sub-protocol entry that starts
///      with `bearer.`. Without this branch the auth middleware (which runs
///      before any WS handler) would 401-storm every dashboard ws (terminal,
///      chat, agent stream). The matching ws handler echoes the protocol
///      back via `WebSocketUpgrade::protocols(...)` so the browser accepts
///      the handshake — see `ws::ws_bearer_protocol`.
///
/// SECURITY: `?token=` query-string auth is intentionally NOT supported.
/// Query parameters appear in server access logs, browser history, and HTTP
/// Referer headers forwarded to third parties, making them unsuitable for
/// carrying credentials.
fn extract_request_token(request: &Request<Body>) -> Option<String> {
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    if bearer.is_some() {
        return bearer;
    }
    if let Some(key) = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
    {
        return Some(key.to_string());
    }
    // WebSocket upgrade: sub-protocol entry of the form `bearer.<token>`.
    // Multiple sub-protocols may be comma-separated; pick the first that
    // starts with `bearer.`.
    request
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(',')
                .map(str::trim)
                .find(|p| p.starts_with("bearer."))
                .and_then(|p| p.strip_prefix("bearer."))
                .map(str::to_string)
        })
}

/// Request ID header name (standard).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Resolved language code extracted from the `Accept-Language` header.
///
/// Inserted into request extensions by the [`accept_language`] middleware so
/// that downstream route handlers can produce localized error messages.
#[derive(Clone, Debug)]
pub struct RequestLanguage(pub &'static str);

/// Per-request correlation id (#3639).
///
/// Inserted into [`Request::extensions_mut`] by [`request_logging`] **before**
/// the handler runs, so handlers can read the same id that ends up in the
/// `x-request-id` response header and the structured access-log line. Use
/// the [`crate::extractors::RequestId`] axum extractor on the handler side
/// — direct extension access is also supported.
#[derive(Clone, Debug)]
pub struct RequestIdExt(pub String);

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
///
/// The request_id is also published as a field on a per-request tracing
/// span that wraps the downstream handler.  Any child span opened inside
/// the handler — including the kernel orchestration spans and the
/// `llm.complete` / `llm.stream` spans on each LLM driver — inherits this
/// field automatically, so a single grep on `request_id=<uuid>` lights up
/// the full execution path (HTTP → kernel → LLM provider).  This closes
/// the propagation gap reported in #3775.
pub async fn request_logging(mut request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let method = request.method().clone();
    let uri = request.uri().path().to_string();
    let start = Instant::now();

    // #3639: stash the id in request extensions BEFORE the handler runs so
    // the [`crate::extractors::RequestId`] extractor (and any handler that
    // reads the extension directly) sees the same value that surfaces on
    // the response header and access-log span.
    request
        .extensions_mut()
        .insert(RequestIdExt(request_id.clone()));

    // Span wraps the entire downstream future so any `tracing::instrument`
    // (or manual span) opened inside the handler chain becomes a child of
    // this span and carries `request_id` for free.  `info_span!` (not
    // `debug_span!`) so the span is recorded at the default subscriber
    // level — debug-level spans get filtered out in release builds and
    // the propagation guarantee disappears with them.
    let request_span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %uri,
    );

    let mut response = next.run(request).instrument(request_span).await;

    let elapsed = start.elapsed();
    let status = response.status().as_u16();

    // Lift handler-resolved identifiers out of the response extensions and
    // onto the structured access-log line. Closes #3511 — without this,
    // tracing all requests for a specific agent/session across the kernel
    // boundary requires `RUST_LOG=debug` and string matching on raw URI
    // paths.
    let agent_id = response
        .extensions()
        .get::<crate::extensions::AgentIdField>()
        .map(|f| f.0.to_string());
    let agent_id_field = agent_id.as_deref().unwrap_or("");

    let session_id = response
        .extensions()
        .get::<crate::extensions::SessionIdField>()
        .map(|f| f.0.to_string());
    let session_id_field = session_id.as_deref().unwrap_or("");

    // 4xx/5xx elevated so auth storms and server faults surface; GET successes suppressed to avoid poll noise.
    if status >= 500 {
        error!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            agent_id = %agent_id_field,
            session_id = %session_id_field,
            "API request"
        );
    } else if status >= 400 {
        warn!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            agent_id = %agent_id_field,
            session_id = %session_id_field,
            "API request"
        );
    } else if method == axum::http::Method::GET {
        debug!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            agent_id = %agent_id_field,
            session_id = %session_id_field,
            "API request"
        );
    } else {
        info!(
            request_id = %request_id,
            method = %method,
            path = %uri,
            status = status,
            latency_ms = elapsed.as_millis() as u64,
            agent_id = %agent_id_field,
            session_id = %session_id_field,
            "API request"
        );
    }

    metrics::record_http_request(&uri, method.as_str(), status, elapsed);

    // Inject the request ID into the response header (always).
    if let Ok(header_val) = request_id.parse() {
        response.headers_mut().insert(REQUEST_ID_HEADER, header_val);
    }

    // #3639: stamp `request_id` (and a default `code` when missing) onto
    // every JSON 4xx/5xx response body so clients can correlate errors
    // with logs / support tickets without parsing the response header.
    // No-op for non-error responses, non-JSON bodies, and bodies that the
    // handler already populated with a `request_id`.
    if status >= 400 {
        response = normalize_json_error_body(response, &request_id).await;
    }

    response
}

/// Treat any `application/json` 4xx/5xx response with a `{"error": ...}`
/// body as the canonical error envelope and stamp `request_id` (#3639) plus
/// a default machine-readable `code` derived from the HTTP status when the
/// handler didn't already supply one. This centralises the contract so the
/// dozens of remaining `Json(json!({"error": "..."}))` sites in route
/// modules surface a uniform shape without per-site edits.
///
/// Bodies that fail to parse as a JSON object, or that are not JSON at all,
/// pass through untouched.
async fn normalize_json_error_body(response: Response<Body>, request_id: &str) -> Response<Body> {
    // Only touch JSON responses — leaving binary, HTML, plain-text, and
    // streaming bodies (SSE) alone is essential.
    let is_json = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if !is_json {
        return response;
    }

    let status_code = response.status();
    let (mut parts, body) = response.into_parts();

    // Cap how much of the body we'll buffer to avoid OOM if a handler
    // somehow produced a multi-megabyte error response. 256 KiB is far above
    // any realistic error envelope. `axum::body::to_bytes` enforces the cap
    // for us — anything larger is left untouched.
    const MAX_ERROR_BODY_BYTES: usize = 256 * 1024;
    let bytes = match axum::body::to_bytes(body, MAX_ERROR_BODY_BYTES).await {
        Ok(b) => b,
        // Body too large or transport error — emit empty body to avoid
        // sending a half-buffered payload, but keep the original headers
        // so callers still see status + request_id header.
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    // Try parsing as a JSON object. Anything else (top-level array,
    // primitive, or invalid JSON) is left untouched.
    let mut value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
        _ => return Response::from_parts(parts, Body::from(bytes)),
    };
    let Some(obj) = value.as_object_mut() else {
        return Response::from_parts(parts, Body::from(bytes));
    };

    // Only stamp on bodies that look like our error envelope (have an
    // `"error"` key). Non-error JSON 4xx/5xx (rare but possible — e.g.
    // structured 422 with a custom shape) is passed through as-is.
    if !obj.contains_key("error") {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let mut mutated = false;
    let default_code = default_error_code_for_status(status_code);

    if !obj.contains_key("request_id") {
        obj.insert(
            "request_id".to_string(),
            serde_json::Value::String(request_id.to_string()),
        );
        mutated = true;
    }
    if !obj.contains_key("code") {
        obj.insert(
            "code".to_string(),
            serde_json::Value::String(default_code.to_string()),
        );
        // Mirror onto the legacy `type` alias so old clients see the same token.
        obj.entry("type")
            .or_insert(serde_json::Value::String(default_code.to_string()));
        mutated = true;
    }

    // #3639 deferred — also stamp into the nested `error` object when the
    // handler emitted the new envelope shape (`error: {code, message,
    // request_id}`). Ad-hoc `Json(json!({"error": "msg"}))` sites still
    // emit `error` as a string and are left untouched here; the flat
    // top-level fields above cover them.
    if let Some(err_obj) = obj.get_mut("error").and_then(|v| v.as_object_mut()) {
        if !err_obj.contains_key("request_id") {
            err_obj.insert(
                "request_id".to_string(),
                serde_json::Value::String(request_id.to_string()),
            );
            mutated = true;
        }
        if !err_obj.contains_key("code") {
            err_obj.insert(
                "code".to_string(),
                serde_json::Value::String(default_code.to_string()),
            );
            mutated = true;
        }
    }

    if !mutated {
        return Response::from_parts(parts, Body::from(bytes));
    }

    // Re-serialize. Failure here is essentially impossible (we just parsed
    // it), but fall back to the original bytes if it ever does.
    let new_bytes = match serde_json::to_vec(&value) {
        Ok(v) => v,
        Err(_) => return Response::from_parts(parts, Body::from(bytes)),
    };
    // Update Content-Length so the framing stays correct.
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    if let Ok(len_val) = new_bytes.len().to_string().parse() {
        parts
            .headers
            .insert(axum::http::header::CONTENT_LENGTH, len_val);
    }
    Response::from_parts(parts, Body::from(new_bytes))
}

/// Map HTTP status code → default stable error code (#3639).
///
/// Only used when the handler didn't already supply a `code`. Values come
/// from [`librefang_types::error_code::ErrorCode`] so the alphabet stays in
/// one place.
fn default_error_code_for_status(status: StatusCode) -> &'static str {
    use librefang_types::error_code::ErrorCode;
    match status.as_u16() {
        400 => ErrorCode::BadRequest.as_str(),
        401 => ErrorCode::Unauthorized.as_str(),
        403 => ErrorCode::Forbidden.as_str(),
        404 => ErrorCode::NotFound.as_str(),
        409 => ErrorCode::Conflict.as_str(),
        422 => ErrorCode::InvalidInput.as_str(),
        429 => ErrorCode::RateLimited.as_str(),
        503 => ErrorCode::ServiceUnavailable.as_str(),
        s if s >= 500 => ErrorCode::InternalError.as_str(),
        _ => ErrorCode::BadRequest.as_str(),
    }
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

// ---------------------------------------------------------------------------
// Public route catalog
//
// These typed constants are the single source of truth for which routes the
// auth middleware treats as publicly reachable.  They are intentionally
// `pub` so that integration tests can enumerate them and assert that every
// path either lives here or requires an Authorization header.
//
// Sorted alphabetically by path within each slice for deterministic ordering.
// ---------------------------------------------------------------------------

/// Whether a public route is reachable on any HTTP method or GET only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicMethod {
    /// Any HTTP method is public (no token required).
    Any,
    /// Only GET requests are public; other methods require auth.
    GetOnly,
}

/// Whether the path must match exactly or may be a prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicMatch {
    /// The normalised request path must equal `path` exactly.
    Exact,
    /// The normalised request path must start with `path`.
    Prefix,
}

/// A single entry in the public-route allowlist.
#[derive(Debug, Clone, Copy)]
pub struct PublicRoute {
    pub method: PublicMethod,
    pub path: &'static str,
    pub match_kind: PublicMatch,
}

impl PublicRoute {
    const fn exact_any(path: &'static str) -> Self {
        Self {
            method: PublicMethod::Any,
            path,
            match_kind: PublicMatch::Exact,
        }
    }
    const fn exact_get(path: &'static str) -> Self {
        Self {
            method: PublicMethod::GetOnly,
            path,
            match_kind: PublicMatch::Exact,
        }
    }
    const fn prefix_any(path: &'static str) -> Self {
        Self {
            method: PublicMethod::Any,
            path,
            match_kind: PublicMatch::Prefix,
        }
    }
    const fn prefix_get(path: &'static str) -> Self {
        Self {
            method: PublicMethod::GetOnly,
            path,
            match_kind: PublicMatch::Prefix,
        }
    }
}

/// Routes that are public on **any** HTTP method, regardless of auth config.
///
/// These are either static assets needed to render the login screen, auth
/// flow entry points, or minimal liveness probes that leak nothing sensitive.
///
/// Ordering note: entries here are grouped by semantic role (assets /
/// auth-flow / pairing / liveness / OAuth) rather than sorted alphabetically,
/// for readability. `PUBLIC_ROUTES_GET_ONLY` and `PUBLIC_ROUTES_DASHBOARD_READS`
/// are sorted alphabetically by path. Maintain the chosen ordering when adding
/// new entries to each slice.
pub const PUBLIC_ROUTES_ALWAYS: &[PublicRoute] = &[
    // Static assets / shell
    PublicRoute::exact_any("/"),
    PublicRoute::exact_any("/favicon.ico"),
    PublicRoute::exact_any("/logo.png"),
    // Auth flow entry points (method-free so POST also works)
    PublicRoute::exact_any("/api/auth/callback"),
    PublicRoute::exact_any("/api/auth/dashboard-check"),
    PublicRoute::exact_any("/api/auth/dashboard-login"),
    // Mobile pairing — phone has no API key yet
    PublicRoute::exact_any("/api/pairing/complete"),
    // Minimal liveness probes
    PublicRoute::exact_any("/api/health"),
    // NOTE: `/api/health/detail` is intentionally NOT public here. Its
    // payload includes `panic_count`, `restart_count`, `agent_count`,
    // embedding / extraction model ids, `config_warnings` from
    // `KernelConfig::validate()`, budget percentages, and LLM latency —
    // i.e. operational telemetry that should not be reachable from a
    // cold probe. The dashboard's `<OfflineBanner />` previously polled
    // this endpoint pre-auth and #4893 worked around the 401 spam by
    // exposing the detail payload publicly; the correct fix is for the
    // banner to poll the genuinely minimal `/api/health` instead, which
    // is what it does now. The middleware-internal comment block below
    // (covering the dashboard-read group) has long explained this
    // contract; this PR restores it (#4868 review).
    PublicRoute::exact_any("/api/version"),
    PublicRoute::exact_any("/api/versions"),
    // GitHub Copilot OAuth — prefix, any method
    PublicRoute::prefix_any("/api/providers/github-copilot/oauth/"),
];

/// Routes that are public on **GET only**, regardless of auth config.
pub const PUBLIC_ROUTES_GET_ONLY: &[PublicRoute] = &[
    PublicRoute::exact_get("/.well-known/agent.json"),
    // A2A: agent listing is public so external callers can discover agents
    // without a bearer token (A2A spec intent). All other /a2a/* paths require
    // auth (Bug #3781).
    PublicRoute::exact_get("/a2a/agents"),
    PublicRoute::exact_get("/api/auth/providers"),
    // Auth login prefix
    PublicRoute::prefix_get("/api/auth/login"),
    // Config schema
    PublicRoute::exact_get("/api/config/schema"),
    // Dashboard assets (JS/CSS/fonts) — always public, SPA needs them for login page
    PublicRoute::prefix_get("/dashboard/assets/"),
    // PWA siblings of the dashboard shell — static bytes baked into the binary
    // via `include_dir!` (see `webchat.rs::resolve_dashboard_file`), identical
    // for every user and leaking nothing sensitive. They MUST be reachable
    // unauthenticated because:
    //   * the W3C App Manifest spec mandates `credentials="omit"` for
    //     `<link rel="manifest">` fetches absent `crossorigin="use-credentials"`,
    //     so the session cookie is intentionally not sent;
    //   * the service-worker register fetch and PWA icons are likewise issued
    //     before/around the login flow.
    // Without the exemption every authenticated dashboard load would log a
    // stream of WARN 401s for these paths.
    //
    // Source of truth for the asset set is `dashboard/public/` (bundled by
    // Vite into `dist/`). Adding a new PWA asset there means: (1) reference
    // it from `dashboard/index.html` (or `manifest.json`) and (2) add an
    // exact-match entry here. The rate limiter exempts the whole `/dashboard/`
    // tree via prefix in `rate_limiter.rs::is_rate_limit_exempt`, so no
    // change is needed there.
    PublicRoute::exact_get("/dashboard/icon-192.png"),
    PublicRoute::exact_get("/dashboard/icon-512.png"),
    PublicRoute::exact_get("/dashboard/manifest.json"),
    PublicRoute::exact_get("/dashboard/sw.js"),
    // i18n locale bundles — static, fetched before auth flow
    PublicRoute::prefix_get("/locales/"),
];

/// Routes in the "dashboard reads" group — public when `require_auth_for_reads`
/// is NOT enabled (or no auth is configured), authenticated otherwise.
///
/// All entries are GET-only. Prefix entries are marked `PublicMatch::Prefix`.
pub const PUBLIC_ROUTES_DASHBOARD_READS: &[PublicRoute] = &[
    PublicRoute::exact_get("/api/a2a/agents"),
    PublicRoute::exact_get("/api/agents"),
    PublicRoute::exact_get("/api/auto-dream/status"),
    PublicRoute::exact_get("/api/budget"),
    PublicRoute::exact_get("/api/budget/agents"),
    PublicRoute::prefix_get("/api/budget/agents/"),
    PublicRoute::exact_get("/api/channels"),
    PublicRoute::exact_get("/api/config"),
    PublicRoute::prefix_get("/api/cron/"),
    PublicRoute::exact_get("/api/hands"),
    PublicRoute::exact_get("/api/hands/active"),
    PublicRoute::prefix_get("/api/hands/"),
    PublicRoute::exact_get("/api/mcp/catalog"),
    PublicRoute::exact_get("/api/mcp/health"),
    PublicRoute::exact_get("/api/mcp/servers"),
    PublicRoute::exact_get("/api/models"),
    PublicRoute::exact_get("/api/models/aliases"),
    PublicRoute::exact_get("/api/network/status"),
    PublicRoute::exact_get("/api/profiles"),
    PublicRoute::exact_get("/api/providers"),
    PublicRoute::exact_get("/api/sessions"),
    PublicRoute::exact_get("/api/skills"),
    PublicRoute::exact_get("/api/status"),
    PublicRoute::exact_get("/api/workflows"),
];

/// Check whether a normalised path matches a [`PublicRoute`] entry.
fn matches_route(route: &PublicRoute, path: &str, is_get: bool) -> bool {
    let method_ok = match route.method {
        PublicMethod::Any => true,
        PublicMethod::GetOnly => is_get,
    };
    if !method_ok {
        return false;
    }
    match route.match_kind {
        PublicMatch::Exact => path == route.path,
        PublicMatch::Prefix => path.starts_with(route.path),
    }
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
    // Snapshot the per-user API key list once per request — `user_api_keys`
    // is now an `Arc<RwLock<Vec<…>>>` so the rotate-key endpoint can swap
    // entries live. The snapshot is cheap (small Vec of role records, no
    // hash work) and lets every downstream read avoid re-acquiring the
    // lock, including the constant-time `verify_password` loop below.
    let user_api_keys: Vec<ApiUserAuth> = auth_state.user_api_keys.read().await.clone();
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
    // SECURITY: Loopback requests go through the same auth check as all other
    // connections. The unconditional loopback bypass has been removed — any
    // process on the same host must supply a valid token just like a remote
    // caller (see bug #3558).
    //
    // We still perform early token attribution here so that RBAC-gated
    // handlers (audit, per-user budget write, …) that require an
    // AuthenticatedApiUser extension work correctly for loopback callers that
    // carry a valid session or per-user API key (e.g. the CLI, a Vite
    // dev-proxy). After attribution the request falls through to the normal
    // is_public / token-verification flow below — there is no early return.
    {
        let is_loopback = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        if is_loopback {
            if let Some(token_str) = extract_request_token(&request) {
                // First try active dashboard sessions (random hex token exact
                // match) — the SPA proxied through Vite at 127.0.0.1 presents
                // a session cookie that must retain its role attribution.
                let session_attribution = {
                    let sessions = auth_state.active_sessions.read().await;
                    sessions.get(&token_str).cloned()
                };
                if let Some(session) = session_attribution {
                    if let (Some(name), Some(role_str)) = (session.user_name, session.user_role) {
                        let role = UserRole::from_str_role(&role_str);
                        let user_id = UserId::from_name(&name);
                        request.extensions_mut().insert(AuthenticatedApiUser {
                            name,
                            role,
                            user_id,
                        });
                    }
                    // Fall through to normal auth — the session token will be
                    // validated again in the main token-check path below.
                }
                // Try per-user API keys (Argon2 verify against api_key_hash).
                // Use the local `user_api_keys` snapshot taken at the top of
                // `auth()` — single source of truth for this request.
                else if let Some(user) = user_api_keys
                    .iter()
                    .find(|user| {
                        crate::password_hash::verify_password(&token_str, &user.api_key_hash)
                    })
                    .cloned()
                {
                    // Apply the role gate so a Viewer/User key on loopback
                    // cannot smuggle a write it would be denied over the LAN.
                    if !user_role_allows_request(user.role, &method, path) {
                        if let Some(ref audit) = auth_state.audit_log {
                            audit.record_with_context(
                                "system",
                                librefang_kernel::audit::AuditAction::PermissionDenied,
                                format!("{} {}", method, path),
                                format!("role={}", user.role),
                                Some(user.user_id),
                                Some("api".to_string()),
                            );
                        }
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
                        user_id: user.user_id,
                    });
                    // Fall through to normal auth — the token will be
                    // re-verified in the main token-check path below.
                }
            }
            // No early return — loopback requests continue through the
            // standard is_public check and token verification below.
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
    // Walk PUBLIC_ROUTES_ALWAYS: public on any HTTP method regardless of auth config.
    let always_public_method_free = PUBLIC_ROUTES_ALWAYS
        .iter()
        .any(|r| matches_route(r, path, is_get));

    // MCP OAuth callback — browser redirect from OAuth provider, no API key.
    // Pattern: /api/mcp/servers/{name}/auth/callback — GET only.
    // This is the sole public entry point for the MCP OAuth flow; the prefix
    // "/api/mcp/servers/" is NOT in the PUBLIC_ROUTES_* slices so that
    // /api/mcp/servers/{name} and /auth/status remain auth-protected.
    let is_mcp_oauth_callback =
        is_get && path.starts_with("/api/mcp/servers/") && path.ends_with("/auth/callback");

    // Path has been trimmed of trailing slashes above, so `/dashboard/` is
    // normalized to `/dashboard`. Match the bare root as well as any
    // descendant so the login gate (and cookie session lookup below) don't
    // silently miss the root navigation.
    let is_dashboard_path = path == "/dashboard" || path.starts_with("/dashboard/");

    // Compute `auth_configured` early so we can decide whether the SPA
    // shell at `/dashboard/*` stays publicly reachable. When *any* form of
    // auth is configured, shell access goes behind the session cookie and
    // an unauthenticated browser gets a minimal inline login page
    // (see the 401 handler below). When no auth is configured the shell
    // stays public so the out-of-the-box dev experience still works.
    let auth_configured = !api_key.trim().is_empty()
        || !user_api_keys.is_empty()
        || auth_state.dashboard_auth_enabled;
    // The inline login page (`login_page.html`) only speaks username/password,
    // so only gate the shell when *that* mode is actually enabled. API-key-only
    // deployments keep a public shell so the SPA can load its own API-key
    // entry UI; the individual `/api/*` endpoints still require a Bearer
    // token, which is the real security boundary.
    //
    // Dashboard assets (JS/CSS/font chunks) and locale bundles are in
    // PUBLIC_ROUTES_GET_ONLY; the dashboard shell is conditionally public
    // based on dashboard_auth_enabled (handled below).
    let dashboard_shell_public = !auth_state.dashboard_auth_enabled && is_dashboard_path;

    // Walk PUBLIC_ROUTES_GET_ONLY: public on GET only regardless of auth config.
    // MCP OAuth callbacks are handled separately by is_mcp_oauth_callback above
    // (prefix + suffix check), not via a PUBLIC_ROUTES_GET_ONLY prefix entry.
    let always_public_get_only = is_get
        && (PUBLIC_ROUTES_GET_ONLY
            .iter()
            .any(|r| matches_route(r, path, is_get))
            || dashboard_shell_public);

    let always_public =
        always_public_method_free || always_public_get_only || is_mcp_oauth_callback;

    // "Dashboard reads" — the legacy public allowlist that lets the SPA
    // render before the user enters credentials. Downgraded to authenticated
    // when `require_auth_for_reads` is enabled AND an `api_key` is configured,
    // so a remote attacker can no longer enumerate agents, config, budget,
    // sessions, approvals, hands, skills, or workflows.
    //
    // SECURITY #3367 + post-merge audit of #3941: /api/approvals/* is
    // intentionally absent — every read path there exposes `action_summary`
    // (the pending shell command). The dashboard attaches credentials on every
    // request via its api helper, so this is not a UX regression.
    //
    // NOTE: /api/logs/stream (SSE) is also intentionally excluded — it
    // streams real-time audit/log events and must require auth the same way
    // every other sensitive read endpoint does. (#3593/#3680)
    let dashboard_read_public = is_get
        && PUBLIC_ROUTES_DASHBOARD_READS
            .iter()
            .any(|r| matches_route(r, path, is_get));

    let enforce_auth_on_reads = auth_state.require_auth_for_reads && auth_configured;

    let is_public = always_public || (dashboard_read_public && !enforce_auth_on_reads);

    if is_public {
        return next.run(request).await;
    }

    // If no API key configured (empty/whitespace) and no other auth method is
    // active, fail closed for any request that did NOT come from loopback —
    // unless the operator explicitly opted in via LIBREFANG_ALLOW_NO_AUTH=1.
    //
    // SECURITY: This closes the openfang #1034 hole where an empty api_key
    // bypassed auth for every origin (LAN/public), exposing agent config,
    // channel tokens, and LLM keys to anyone reachable on the bind address.
    // Loopback already short-circuits above for the single-user dev UX, so
    // reaching this branch means the caller is on the LAN/WAN.
    let api_key = api_key.trim();
    if api_key.is_empty() && user_api_keys.is_empty() && !auth_state.dashboard_auth_enabled {
        // Re-check ConnectInfo defensively — if it is missing for any reason
        // we MUST treat the origin as non-loopback (fail closed, never open).
        let is_loopback = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        if is_loopback || auth_state.allow_no_auth {
            return next.run(request).await;
        }
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("www-authenticate", "Bearer")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "error": "API key required for non-loopback requests. Set api_key in config.toml, bind to 127.0.0.1, or set LIBREFANG_ALLOW_NO_AUTH=1 to opt out."
                })
                .to_string(),
            ))
            .unwrap_or_default();
    }

    // Check Authorization: Bearer <token> header, then fallback to X-API-Key,
    // then fallback to Sec-WebSocket-Protocol: bearer.<token> for WS upgrades.
    // Browsers cannot set custom headers on WebSocket handshakes, so the
    // dashboard encodes the session token as a sub-protocol entry — this must
    // be checked here for non-loopback connections (Docker bridge, LAN) where
    // the loopback fast-path above is not taken.
    let bearer_token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let api_token = bearer_token
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
        })
        .or_else(|| {
            // WS upgrade fallback: Sec-WebSocket-Protocol: bearer.<token>
            request
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.split(',')
                        .map(str::trim)
                        .find(|p| p.starts_with("bearer."))
                        .and_then(|p| p.strip_prefix("bearer."))
                })
        });

    // Cookie-based session token — only accepted for SPA shell navigation
    // (`/dashboard/*`). API endpoints still require a Bearer/header token so
    // a cross-site request that auto-forwards the cookie cannot trigger a
    // write. Pair with `SameSite=Lax` on the Set-Cookie (issued by
    // `dashboard_login`) for the usual CSRF posture.
    let cookie_session_token = if is_dashboard_path {
        request
            .headers()
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .and_then(|header| {
                header
                    .split(';')
                    .map(str::trim)
                    .find_map(|kv| kv.strip_prefix("librefang_session="))
                    .map(str::to_string)
            })
    } else {
        None
    };

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

    // SECURITY: ?token= query-string auth is deliberately NOT checked here.
    // Query parameters are written to server access logs, retained in browser
    // history, and forwarded in HTTP Referer headers to third parties. Tokens
    // must only arrive via Authorization: Bearer or X-API-Key headers, or via
    // the session cookie. WebSocket upgrades are the sole exception (browsers
    // cannot set custom headers on WebSocket connections); they authenticate
    // via crate::ws::ws_auth_token, which never passes through this middleware.

    // Accept if header auth matches a static API key or legacy token
    if header_auth == Some(true) {
        return next.run(request).await;
    }

    // Check the active session store for randomly generated dashboard tokens.
    // Also prune expired sessions opportunistically. Cookie token is only
    // consulted for `/dashboard/*` navigation (filtered upstream).
    let provided_token = api_token.or(cookie_session_token.as_deref());
    if let Some(token_str) = provided_token {
        let mut sessions = auth_state.active_sessions.write().await;
        // Remove expired sessions while we hold the lock
        sessions.retain(|_, st| {
            !crate::password_hash::is_token_expired(
                st,
                crate::password_hash::DEFAULT_SESSION_TTL_SECS,
            )
        });
        if let Some(session) = sessions.get(token_str).cloned() {
            drop(sessions);
            // If the session was issued by a credential flow that carried
            // identity (dashboard_login attaches `user_name` + `user_role`),
            // rebuild the AuthenticatedApiUser extension so RBAC-gated
            // handlers (audit/query, per-user budget writes) can see the
            // role. Legacy sessions persisted before attribution was added
            // load with both fields `None` and continue through as
            // trusted-anonymous — preserves the pre-fix behaviour for any
            // session sitting in `~/.librefang/sessions.json` from older
            // builds.
            if let (Some(name), Some(role_str)) = (session.user_name, session.user_role) {
                let role = UserRole::from_str_role(&role_str);
                let user_id = UserId::from_name(&name);
                request.extensions_mut().insert(AuthenticatedApiUser {
                    name,
                    role,
                    user_id,
                });
            }
            return next.run(request).await;
        }
        drop(sessions);

        if let Some(user) = user_api_keys
            .iter()
            .find(|user| crate::password_hash::verify_password(token_str, &user.api_key_hash))
            .cloned()
        {
            if !user_role_allows_request(user.role, &method, path) {
                // RBAC M5: surface the denial in the hash-chained audit
                // log so an operator can correlate 403s with the user
                // who tripped them. Best-effort — we do not have a
                // direct kernel handle in the middleware extension so
                // we read it back via the `audit_log_handle` injected
                // into AuthState at server build time.
                if let Some(ref audit) = auth_state.audit_log {
                    audit.record_with_context(
                        "system",
                        librefang_kernel::audit::AuditAction::PermissionDenied,
                        format!("{} {}", method, path),
                        format!("role={}", user.role),
                        Some(user.user_id),
                        Some("api".to_string()),
                    );
                }
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
                user_id: user.user_id,
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

    let credential_provided = header_auth.is_some();
    let error_msg = if credential_provided {
        translator.t("api-error-auth-invalid-key")
    } else {
        translator.t("api-error-auth-missing-header")
    };

    // Browser navigation to `/dashboard/*` with no valid session — serve a
    // minimal self-contained login page instead of a JSON error, so the SPA
    // bundle (and whatever it imports) never reaches an unauthenticated
    // caller.
    if is_get && is_dashboard_path && auth_state.dashboard_auth_enabled {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "text/html; charset=utf-8")
            .header("cache-control", "no-store")
            .body(Body::from(LOGIN_PAGE_HTML))
            .unwrap_or_default();
    }

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("www-authenticate", "Bearer")
        .header("content-language", lang)
        .body(Body::from(
            serde_json::json!({"error": error_msg}).to_string(),
        ))
        .unwrap_or_default()
}

const LOGIN_PAGE_HTML: &str = include_str!("login_page.html");

/// Security headers middleware — applied to ALL API responses.
pub async fn security_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("x-xss-protection", "1; mode=block".parse().unwrap());
    // All JS/CSS is bundled inline — only external resource is Google Fonts.
    // SECURITY: 'unsafe-eval' removed from script-src (#3732). 'unsafe-inline'
    // removed from script-src as well; the bundled SPA does not need it.
    // 'unsafe-inline' is kept in style-src only because the React/Vite bundle
    // injects CSS-in-JS style tags at runtime and removing it would break the
    // dashboard UI until a nonce-based approach is wired through the build.
    headers.insert(
        "content-security-policy",
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com https://fonts.gstatic.com; img-src 'self' data: blob:; connect-src 'self' ws://localhost:* ws://127.0.0.1:* wss://localhost:* wss://127.0.0.1:*; font-src 'self' https://fonts.gstatic.com; media-src 'self' blob:; frame-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'"
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

    // #3621: TOTP enrollment must be Owner-only. Without this gate, any
    // bearer token (including a Viewer or User role) could overwrite the
    // unconfirmed `totp_secret` and hijack enrollment, or wipe a confirmed
    // enrollment via `revoke` and silently disable 2FA on login.
    #[test]
    fn test_totp_enrollment_is_owner_only() {
        let post = axum::http::Method::POST;
        for role in [UserRole::Viewer, UserRole::User, UserRole::Admin] {
            for path in [
                "/api/approvals/totp/setup",
                "/api/approvals/totp/confirm",
                "/api/approvals/totp/revoke",
            ] {
                assert!(
                    !user_role_allows_request(role, &post, path),
                    "{role:?} must NOT be allowed to POST {path}"
                );
            }
        }
        // Owner still has access.
        for path in [
            "/api/approvals/totp/setup",
            "/api/approvals/totp/confirm",
            "/api/approvals/totp/revoke",
        ] {
            assert!(
                user_role_allows_request(UserRole::Owner, &post, path),
                "Owner must be allowed to POST {path}"
            );
        }

        // Regression for over-gating: GET /api/approvals/totp/status is a
        // read-only enrollment-status probe and must remain reachable for
        // every authenticated role, including non-Owner ones.
        let get = axum::http::Method::GET;
        for role in [
            UserRole::Viewer,
            UserRole::User,
            UserRole::Admin,
            UserRole::Owner,
        ] {
            assert!(
                user_role_allows_request(role, &get, "/api/approvals/totp/status"),
                "{role:?} must be allowed to GET /api/approvals/totp/status"
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
    fn test_user_role_admin_cannot_mutate_users_endpoints() {
        // RBAC M6: every mutating call under /api/users* maps to
        // Action::ManageUsers, which requires Owner. Without this gate an
        // Admin per-user API key could promote itself to Owner via
        // POST /api/users.
        for method in [
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ] {
            for path in ["/api/users", "/api/users/alice", "/api/users/import"] {
                assert!(
                    !user_role_allows_request(UserRole::Admin, &method, path),
                    "Admin must NOT be allowed to {method} {path}"
                );
                assert!(
                    user_role_allows_request(UserRole::Owner, &method, path),
                    "Owner must be allowed to {method} {path}"
                );
            }
        }
    }

    #[test]
    fn test_user_role_viewer_can_still_list_users_for_simulator() {
        // GET on /api/users* stays at the generic Admin-or-above gate (the
        // permission simulator needs the list). Viewer/User remain GET-only
        // by the existing user_role_allows_request rules.
        let get = axum::http::Method::GET;
        assert!(user_role_allows_request(
            UserRole::Admin,
            &get,
            "/api/users"
        ));
        assert!(user_role_allows_request(
            UserRole::Owner,
            &get,
            "/api/users"
        ));
        // GET is universally allowed by the role-allows logic, so even
        // Viewer can read — middleware-level filtering of PII is a
        // separate concern (UserView already redacts api_key_hash).
        assert!(user_role_allows_request(
            UserRole::Viewer,
            &get,
            "/api/users"
        ));
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
        // Session-scoped approval endpoints are also denied for Viewer.
        assert!(!user_role_allows_request(
            UserRole::Viewer,
            &post,
            "/api/approvals/session/sess-1/approve_all"
        ));
        assert!(!user_role_allows_request(
            UserRole::Viewer,
            &post,
            "/api/approvals/session/sess-1/reject_all"
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "ReadOnly".to_string(),
                role: UserRole::Viewer,
                api_key_hash: crate::password_hash::hash_password("viewer-key").unwrap(),
                user_id: UserId::from_name("ReadOnly"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "ReadOnly".to_string(),
                role: UserRole::Viewer,
                api_key_hash: crate::password_hash::hash_password("viewer-key").unwrap(),
                user_id: UserId::from_name("ReadOnly"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "Guest".to_string(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("user-key").unwrap(),
                user_id: UserId::from_name("Guest"),
            }])),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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

    /// `/api/auto-dream/status` is a dashboard read — same shape as
    /// `/api/agents` etc.: GET returns the global toggle + per-agent
    /// state, drives the Settings page's Dream Mode card. Must not 401
    /// when no auth is configured (default install) so the SPA renders.
    /// POST endpoints under `/api/auto-dream/agents/*` (trigger / abort /
    /// enabled) stay write-protected — they are not added to the
    /// allowlist.
    #[tokio::test]
    async fn test_auto_dream_status_get_is_dashboard_read_public() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };
        let app = Router::new()
            .route("/api/auto-dream/status", get(|| async { "status" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/auto-dream/status")
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(vec![ApiUserAuth {
                name: "alice".into(),
                role: UserRole::User,
                api_key_hash: crate::password_hash::hash_password("alice-key").unwrap(),
                user_id: UserId::from_name("alice"),
            }])),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
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
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: true,
            allow_no_auth: false,
            audit_log: None,
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

    // ---- openfang #1034 port: empty-api_key fail-closed coverage --------
    //
    // Helper builders + 6 scenarios specified by the security port:
    //   (a) loopback + no key      → 200
    //   (b) LAN IP + no key        → 401
    //   (c) public IP + no key     → 401
    //   (d) allow_no_auth=1        → 200 from any origin
    //   (e) configured key         → still does normal Bearer validation
    //   (f) missing ConnectInfo    → 401 (fail-closed, never open)

    fn no_auth_state() -> AuthState {
        AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(String::new())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        }
    }

    fn with_key_state(key: &str) -> AuthState {
        AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new(key.to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        }
    }

    fn protected_router(state: AuthState) -> Router {
        Router::new()
            .route("/api/agents/1", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state, auth))
    }

    fn req_with_addr(ip: &str) -> Request<Body> {
        let addr: std::net::SocketAddr = format!("{ip}:40000").parse().unwrap();
        let mut req = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        req
    }

    /// (a) Empty api_key + loopback origin → 200. Single-user dev UX kept.
    #[tokio::test]
    async fn empty_key_allows_loopback() {
        let app = protected_router(no_auth_state());
        let resp = app.oneshot(req_with_addr("127.0.0.1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// (b) Empty api_key + LAN origin → 401. Closes the #1034 hole where a
    /// 192.168.x caller could hit every non-public endpoint.
    #[tokio::test]
    async fn empty_key_blocks_lan_origin() {
        let app = protected_router(no_auth_state());
        let resp = app.oneshot(req_with_addr("192.168.1.50")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// (c) Empty api_key + public IP origin → 401.
    #[tokio::test]
    async fn empty_key_blocks_public_origin() {
        let app = protected_router(no_auth_state());
        let resp = app.oneshot(req_with_addr("203.0.113.5")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// (d) `allow_no_auth = true` (i.e. LIBREFANG_ALLOW_NO_AUTH=1 at boot)
    /// opens the door from any origin. Operators must opt in explicitly.
    #[tokio::test]
    async fn empty_key_with_allow_no_auth_opens_lan() {
        let mut s = no_auth_state();
        s.allow_no_auth = true;
        let app = protected_router(s);
        let resp = app.oneshot(req_with_addr("10.0.0.9")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// (e) With an api_key configured, missing token → 401, valid bearer → 200.
    /// Confirms the new branch only fires on the no-auth code path.
    #[tokio::test]
    async fn configured_key_still_validates_bearer() {
        let app = protected_router(with_key_state("secret"));
        let resp = app
            .clone()
            .oneshot(req_with_addr("203.0.113.5"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let addr: std::net::SocketAddr = "203.0.113.5:40000".parse().unwrap();
        let mut authed = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        authed
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        let ok = app.oneshot(authed).await.unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }

    /// (f) ConnectInfo extension is missing → fail closed. The middleware
    /// must never treat unknown origin as loopback. Defense in depth in case
    /// upstream wiring changes (e.g. a future router skips
    /// `into_make_service_with_connect_info`).
    #[tokio::test]
    async fn empty_key_blocks_when_connect_info_missing() {
        let app = protected_router(no_auth_state());
        // No ConnectInfo extension inserted.
        let req = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ---- Regression tests for bug #3558: loopback bypass removed -----------

    /// Regression #3558: when an api_key IS configured, a loopback request
    /// with NO token must be rejected. The old code unconditionally let any
    /// loopback caller through; the fix removes that bypass so loopback goes
    /// through the same token check as every other origin.
    #[tokio::test]
    async fn configured_key_loopback_no_token_is_rejected() {
        let app = protected_router(with_key_state("secret"));
        let resp = app.oneshot(req_with_addr("127.0.0.1")).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "loopback with a configured api_key but no token must be 401, not bypassed"
        );
    }

    /// Regression #3558: when an api_key IS configured, a loopback request
    /// WITH the correct token must still succeed (the fix must not break
    /// legitimate loopback callers that present credentials).
    #[tokio::test]
    async fn configured_key_loopback_valid_token_is_allowed() {
        let app = protected_router(with_key_state("secret"));
        let addr: std::net::SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let mut req = Request::builder()
            .method("GET")
            .uri("/api/agents/1")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "loopback with a valid bearer token must still be allowed through"
        );
    }

    // ---- Bug #3781: GET /a2a/tasks/{id} must require auth ---------------
    //
    // Before the fix, `path.starts_with("/a2a/")` in the always_public_get_only
    // block let any caller read full task transcripts (agent prompts + LLM
    // outputs) without a bearer token. Only `/a2a/agents` (capability discovery)
    // should remain public; task-level resources contain sensitive data.

    /// GET /a2a/agents (the capability listing) must stay public — external
    /// A2A peers call this to discover what skills a local agent exposes.
    #[tokio::test]
    async fn a2a_agents_listing_is_always_public() {
        let app = Router::new()
            .route("/a2a/agents", get(|| async { "agent list" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/a2a/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET /a2a/agents must be public so external A2A peers can discover local agents"
        );
    }

    /// GET /a2a/tasks/{id} must require auth (Bug #3781). Task transcripts
    /// contain full agent prompts and LLM outputs — sensitive operational data.
    #[tokio::test]
    async fn a2a_task_transcript_requires_auth() {
        let app = Router::new()
            .route("/a2a/tasks/{id}", get(|| async { "full task transcript" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        // Unauthenticated → must be rejected.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/a2a/tasks/some-uuid-1234")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "GET /a2a/tasks/{{id}} must require auth — it returns full task transcripts"
        );
    }

    /// Regression for #3473 (dup of #3781): GET /a2a/tasks/{id}/status must
    /// also require auth. The status endpoint exposes per-task progress
    /// signals usable for side-channel inference even before the full
    /// transcript is fetched, so it has to share the auth gate.
    #[tokio::test]
    async fn a2a_task_status_requires_auth() {
        let app = Router::new()
            .route("/a2a/tasks/{id}/status", get(|| async { "task status" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/a2a/tasks/some-uuid-1234/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "GET /a2a/tasks/{{id}}/status must require auth (#3473 dup of #3781)"
        );
    }

    /// GET /a2a/tasks/{id} must allow access with a valid bearer token.
    #[tokio::test]
    async fn a2a_task_transcript_accessible_with_valid_token() {
        let app = Router::new()
            .route("/a2a/tasks/{id}", get(|| async { "full task transcript" }))
            .layer(axum::middleware::from_fn_with_state(
                with_key_state("secret"),
                auth,
            ));

        let addr: std::net::SocketAddr = "203.0.113.5:40000".parse().unwrap();
        let mut req = Request::builder()
            .uri("/a2a/tasks/some-uuid-1234")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));

        let response = app.oneshot(req).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "valid bearer token must allow access to /a2a/tasks/{{id}}"
        );
    }

    // ---- Bug #3680: GET /api/logs/stream must require auth even when
    // ---- require_auth_for_reads = false -------------------------------
    //
    // Before #3909 the SSE endpoint was unconditionally appended to
    // `dashboard_read_public` (`|| path == "/api/logs/stream"`) so any
    // operator who explicitly set `require_auth_for_reads = false` (the
    // documented escape hatch for an external auth proxy) lost auth on
    // the log stream. The stream emits real-time tracing fields that can
    // contain prompts, OAuth callback codes, MCP stderr, and bearer
    // prefixes — a continuous credential leak. The fix removed the
    // path from every public allowlist; this test locks that contract
    // so a future refactor cannot silently re-introduce it.

    /// GET /api/logs/stream must return 401 when `require_auth_for_reads`
    /// is OFF — the SSE log stream is sensitive enough that the
    /// "loosen reads" escape hatch must NOT apply to it.
    #[tokio::test]
    async fn logs_stream_requires_auth_even_when_reads_are_loosened() {
        // Reproduce the deployment posture that exposed the bug:
        // an api_key is configured, but the operator has opted out of
        // auth-gating dashboard reads (e.g. fronting with an external
        // auth proxy). /api/logs/stream MUST still require auth.
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };

        let app = Router::new()
            .route("/api/logs/stream", get(|| async { "sse stream" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        // Simulate a remote (non-loopback) caller so the loopback
        // short-circuit cannot mask the bug.
        let addr: std::net::SocketAddr = "203.0.113.5:53000".parse().unwrap();
        let mut req = Request::builder()
            .uri("/api/logs/stream")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));

        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "GET /api/logs/stream must require auth — SSE leaks tracing \
             fields with prompts, OAuth codes, and bearer prefixes"
        );
    }

    /// Sanity check: /api/logs/stream with a valid bearer DOES go through.
    /// Without this counter-test the regression test above could pass by
    /// accident (e.g. if the route were globally blocked).
    #[tokio::test]
    async fn logs_stream_allows_authenticated_caller() {
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };

        let app = Router::new()
            .route("/api/logs/stream", get(|| async { "sse stream" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        let addr: std::net::SocketAddr = "203.0.113.5:53000".parse().unwrap();
        let mut req = Request::builder()
            .uri("/api/logs/stream")
            .header("authorization", "Bearer secret")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "valid bearer token must allow access to /api/logs/stream"
        );
    }

    /// Regression: #3367 — GET /api/approvals/session/{id} used to be
    /// publicly readable via the `/api/approvals/` prefix in
    /// `dashboard_read_prefix`. That endpoint returns pending approval
    /// details including shell commands, so it must require authentication
    /// even when `require_auth_for_reads` is off.
    ///
    /// Updated post-#3941 audit: every approvals read endpoint exposes
    /// the same `action_summary` (pending shell command), so the entire
    /// `/api/approvals/*` surface must be auth-gated, not just the
    /// `/session/` sub-tree.
    #[tokio::test]
    async fn approvals_reads_require_auth() {
        // Auth state: api_key configured, require_auth_for_reads OFF — this
        // is the scenario where the bug was exploitable.
        let auth_state = AuthState {
            api_key_lock: Arc::new(tokio::sync::RwLock::new("secret".to_string())),
            active_sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dashboard_auth_enabled: false,
            user_api_keys: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            require_auth_for_reads: false,
            allow_no_auth: false,
            audit_log: None,
        };

        let app = Router::new()
            .route("/api/approvals", get(|| async { "list" }))
            .route(
                "/api/approvals/session/{id}",
                get(|| async { "pending approvals" }),
            )
            .route("/api/approvals/audit", get(|| async { "audit log" }))
            .route("/api/approvals/{id}", get(|| async { "approval detail" }))
            .layer(axum::middleware::from_fn_with_state(auth_state, auth));

        for path in &[
            "/api/approvals",
            "/api/approvals/session/sess-abc-123",
            "/api/approvals/audit",
            "/api/approvals/some-approval-id",
        ] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(*path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{path} must be auth-gated (returns action_summary)"
            );
        }
    }

    /// Regression for #4860: the inline login page must redirect to `/`
    /// (the SPA shell) when it was itself served at `/`, `/dashboard`, or
    /// `/dashboard/`. The router only registers `/` and
    /// `/dashboard/{*path}`, so redirecting back to `/dashboard` or
    /// `/dashboard/` after a successful sign-in lands on a 404.
    #[test]
    fn login_page_redirects_dashboard_root_to_spa_shell() {
        let html = super::LOGIN_PAGE_HTML;
        // Pin the full collapse condition so neither the bare `/dashboard`
        // case nor the trailing-slash case can be silently dropped — a
        // substring like `path === '/dashboard'` would also match
        // `path === '/dashboard/'` and let one half regress unnoticed.
        assert!(
            html.contains("path === '/dashboard' || path === '/dashboard/'"),
            "login page must collapse both /dashboard and /dashboard/ to the SPA shell at /"
        );
        assert!(
            !html.contains("target = '/dashboard/';"),
            "login page must not redirect to /dashboard/ — that path 404s (#4860)"
        );
    }
}
