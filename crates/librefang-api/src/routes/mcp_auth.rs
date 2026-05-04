//! MCP OAuth authentication endpoints.
//!
//! Provides auth status, flow initiation (UI-driven PKCE), callback
//! handling, and token revocation for MCP servers that require OAuth 2.0
//! authorization.

use super::AppState;
use crate::mcp_oauth::KernelOAuthProvider;
use crate::middleware::AuthenticatedApiUser;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use librefang_runtime::mcp_oauth::{self, McpAuthState, OAuthTokens};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use url::Url;

/// SHA-256 prefix of the caller's user_id (UUID).  Embedded into the vault
/// key + flow_id so a callback initiated by user A cannot be redeemed
/// against user B's in-flight flow even if they targeted the same server.
///
/// Truncated to 64 bits — we only need collision avoidance among concurrent
/// in-flight flows on a single daemon, not preimage resistance, so 16 hex
/// chars of SHA-256 is sufficient.
fn caller_fingerprint(user: &Option<Extension<AuthenticatedApiUser>>) -> String {
    let raw = match user {
        Some(Extension(u)) => u.user_id.to_string(),
        // No identity attached — fall back to a constant so single-user
        // deployments (no RBAC configured) still produce deterministic
        // vault keys.  The flow_id random nonce still keeps concurrent
        // anonymous flows isolated.
        None => "anon".to_string(),
    };
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())[..16].to_string()
}

fn callback_text(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

fn auth_failed(detail: impl std::fmt::Display) -> Response {
    callback_text(format!(
        "Authorization Failed\n\n{detail}\n\nYou can close this tab."
    ))
}

/// GET /api/mcp/servers/{name}/auth/status
///
/// Returns the current OAuth authentication state for an MCP server.
#[utoipa::path(
    get,
    path = "/api/mcp/servers/{name}/auth/status",
    tag = "mcp",
    params(
        ("name" = String, Path, description = "MCP server name"),
    ),
    responses(
        (status = 200, description = "Auth status for the MCP server", body = crate::types::JsonObject),
        (status = 404, description = "MCP server not found")
    )
)]
pub async fn auth_status(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Verify the server exists in config
    let cfg = state.kernel.config_snapshot();
    if !cfg.mcp_servers.iter().any(|s| s.name == name) {
        return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
            .into_json_tuple();
    }

    // Check auth state
    let auth_states = state.kernel.mcp_auth_states_ref().lock().await;
    if let Some(auth_state) = auth_states.get(&name) {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "server": name,
                "auth": auth_state,
            })),
        );
    }
    drop(auth_states);

    // No explicit auth state — check if connected (implying auth not required)
    let connections = state.kernel.mcp_connections_ref().lock().await;
    let is_connected = connections.iter().any(|c| c.name() == name);
    let state_label = if is_connected {
        "not_required"
    } else {
        "unknown"
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "server": name,
            "auth": { "state": state_label },
        })),
    )
}

/// Derive a safe OAuth callback URL from the incoming request.
///
/// The host portion is validated against `trusted_hosts` plus built-in
/// loopback aliases (`localhost`, `127.0.0.1`, `::1`). When no candidate
/// header matches the allowlist, falls back to the daemon's own listen
/// address — never echoes an untrusted `Host`/`Origin`/`X-Forwarded-Host`
/// header back as the redirect_uri. Matching is hostname-only so that a
/// bare allowlist entry (`dash.example.com`) accepts any port.
fn derive_callback_url(
    headers: &HeaderMap,
    server_name: &str,
    trusted_hosts: &[String],
    api_listen: &str,
) -> String {
    let path = format!("/api/mcp/servers/{}/auth/callback", server_name);

    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if let Some(url) = accept_origin(origin, &path, trusted_hosts) {
            return url;
        }
    }

    if let Some(fwd_host) = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
    {
        if host_is_trusted(fwd_host, trusted_hosts) {
            let proto = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .filter(|p| *p == "http" || *p == "https")
                .unwrap_or("https");
            return format!("{}://{}{}", proto, fwd_host, path);
        }
    }

    if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
        if host_is_trusted(host, trusted_hosts) {
            return format!("http://{}{}", host, path);
        }
    }

    format!("http://{}{}", listen_fallback(api_listen), path)
}

/// Return `Some(callback_url)` if `origin` is a syntactically valid
/// `scheme://host[:port]` whose host is trusted, else `None`.
fn accept_origin(origin: &str, path: &str, trusted_hosts: &[String]) -> Option<String> {
    if origin.is_empty() || origin == "null" {
        return None;
    }
    let (scheme, rest) = origin.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    // Guard against origins that smuggle a path/query/fragment.
    let host_authority = rest.split(['/', '?', '#']).next()?;
    if host_authority.is_empty() || !host_is_trusted(host_authority, trusted_hosts) {
        return None;
    }
    Some(format!("{}://{}{}", scheme, host_authority, path))
}

/// Check whether a `host[:port]` authority matches the allowlist.
///
/// Always permits loopback aliases so local development and the
/// daemon's own API port keep working even with an empty config.
fn host_is_trusted(authority: &str, trusted_hosts: &[String]) -> bool {
    let hostname = strip_port(authority);
    if hostname.is_empty() {
        return false;
    }
    if is_loopback_host(hostname) {
        return true;
    }
    trusted_hosts
        .iter()
        .any(|entry| strip_port(entry).eq_ignore_ascii_case(hostname))
}

fn is_loopback_host(hostname: &str) -> bool {
    matches!(
        hostname.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "::1"
    )
}

/// Strip an optional trailing `:port` from a `host[:port]` authority,
/// handling bracketed IPv6 literals (`[::1]:4545` → `[::1]`).
fn strip_port(authority: &str) -> &str {
    if let Some(stripped) = authority.strip_prefix('[') {
        if let Some(end) = stripped.find(']') {
            return &authority[..end + 2];
        }
    }
    match authority.rfind(':') {
        Some(idx) if !authority[idx + 1..].contains('.') => &authority[..idx],
        _ => authority,
    }
}

/// Turn the daemon's bind address into a reachable loopback authority.
/// `0.0.0.0` / `[::]` bind to all interfaces but are not valid callback
/// targets, so substitute `127.0.0.1` while preserving the port.
fn listen_fallback(api_listen: &str) -> String {
    let (host, port) = match api_listen.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() => (h, p),
        _ => return "127.0.0.1:4545".to_string(),
    };
    let host_lower = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    let safe_host = match host_lower.as_str() {
        "" | "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        other if other == "localhost" || other == "127.0.0.1" => other,
        _ => host,
    };
    format!("{}:{}", safe_host, port)
}

/// Percent-encode a string for use as a URL query parameter value.
fn percent_encode_param(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    result
}

/// POST /api/mcp/servers/{name}/auth/start
///
/// Initiates a UI-driven OAuth PKCE authorization flow for the specified
/// MCP server. Discovers OAuth metadata, performs Dynamic Client
/// Registration if needed, generates PKCE challenge, and returns the
/// authorization URL for the UI to redirect to.
#[utoipa::path(
    post,
    path = "/api/mcp/servers/{name}/auth/start",
    tag = "mcp",
    params(
        ("name" = String, Path, description = "MCP server name"),
    ),
    responses(
        (status = 200, description = "Auth flow started — returns auth URL", body = crate::types::JsonObject),
        (status = 400, description = "Server has no HTTP transport or discovery failed"),
        (status = 404, description = "MCP server not found")
    )
)]
pub async fn auth_start(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    user: Option<Extension<AuthenticatedApiUser>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Find the server config
    let cfg = state.kernel.config_snapshot();
    let entry = match cfg.mcp_servers.iter().find(|s| s.name == name) {
        Some(e) => e.clone(),
        None => {
            return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
                .into_json_tuple();
        }
    };

    // Extract URL from Http or Sse transport
    let server_url = match &entry.transport {
        Some(librefang_types::config::McpTransportEntry::Http { url }) => url.clone(),
        Some(librefang_types::config::McpTransportEntry::Sse { url }) => url.clone(),
        _ => {
            return ApiErrorResponse::bad_request(
                "OAuth is only supported for HTTP/SSE transport MCP servers",
            )
            .into_json_tuple();
        }
    };

    // Discover OAuth metadata (use config.toml overrides if present)
    let oauth_config = entry.oauth.clone().unwrap_or_default();
    let metadata =
        match mcp_oauth::discover_oauth_metadata(&server_url, None, Some(&oauth_config)).await {
            Ok(m) => m,
            Err(e) => {
                let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
                auth_states.insert(name.clone(), McpAuthState::Error { message: e.clone() });
                return ApiErrorResponse::bad_request(format!("OAuth discovery failed: {e}"))
                    .into_json_tuple();
            }
        };

    // Build a KernelOAuthProvider for vault access
    let provider = KernelOAuthProvider::new(state.kernel.home_dir().to_path_buf());

    // Derive the redirect URI from the incoming request — validated
    // against `trusted_hosts` to prevent Host-header spoofing from
    // redirecting the OAuth code to an attacker origin.
    let redirect_uri = derive_callback_url(&headers, &name, &cfg.trusted_hosts, &cfg.api_listen);

    // Check vault for cached client_id, or do Dynamic Client Registration
    let mut client_id = metadata.client_id.clone().or_else(|| {
        // Use the lenient `vault_get_or_warn` here: on a fresh install
        // there is no `vault.enc` yet, and the strict `vault_get`
        // would `Err(KeyNotFound)` → emit a "vault_get failed" warning
        // on every first MCP add. DCR is the documented recovery path
        // for "no cached client_id" anyway, so collapse missing-vault /
        // missing-key into None silently. Real vault unlock failures
        // are still logged at warn! by `vault_get_or_warn`.
        provider.vault_get_or_warn(&KernelOAuthProvider::vault_key(&server_url, "client_id"))
    });

    if client_id.is_none() {
        if let Some(ref reg_endpoint) = metadata.registration_endpoint {
            tracing::info!(
                endpoint = %reg_endpoint,
                "No client_id configured, attempting Dynamic Client Registration"
            );
            match provider
                .register_client(reg_endpoint, &redirect_uri, &server_url)
                .await
            {
                Ok(cid) => {
                    tracing::info!(client_id = %cid, "Dynamic Client Registration succeeded");
                    // #3651: replaced `let _ = vault_set(...)` so a vault
                    // crypto failure here is no longer silently swallowed.
                    // Behavior is intentionally unchanged on the happy path
                    // (continue with the freshly-registered client_id even
                    // if persistence failed — the OAuth flow can still
                    // complete in-memory) but the audit trail now records
                    // every failure so operators can detect a
                    // wrong-`LIBREFANG_VAULT_KEY` boot from the logs.
                    let vault_key = KernelOAuthProvider::vault_key(&server_url, "client_id");
                    if let Err(e) = provider.vault_set(&vault_key, &cid) {
                        tracing::error!(
                            target: "audit",
                            op = "vault_set",
                            key = %vault_key,
                            error = %e,
                            "vault op failed during MCP Dynamic Client Registration persistence"
                        );
                    }
                    client_id = Some(cid);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Dynamic Client Registration failed");
                    let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
                    auth_states.insert(
                        name.clone(),
                        McpAuthState::Error {
                            message: format!("Client registration failed: {e}"),
                        },
                    );
                    return ApiErrorResponse::bad_request(format!(
                        "Dynamic Client Registration failed: {e}"
                    ))
                    .into_json_tuple();
                }
            }
        }
    }

    // Generate PKCE challenge, state, and a unique flow ID.
    //
    // #3727: Per-flow vault keys prevent concurrent auth flows to the same
    // server from clobbering each other's PKCE state.  The flow_id is embedded
    // in the OAuth `state` parameter as `{flow_id}.{random_state}` so the
    // callback can look up the correct vault entry.
    //
    // The flow_id carries a caller fingerprint prefix so a callback initiated
    // by user A is keyed under a vault entry user B's flow can never reach,
    // even if both target the same server URL — closing the multi-user
    // clobber path the issue called out.
    //
    // This supersedes the earlier per-server `{server_name}:{random}` binding
    // (#3911) — per-flow IDs subsume the per-server protection while also
    // allowing multiple concurrent flows against the same server.
    let caller_fp = caller_fingerprint(&user);
    let flow_id = format!("{caller_fp}-{}", mcp_oauth::generate_flow_id());
    let (pkce_verifier, pkce_challenge) = mcp_oauth::generate_pkce();
    let pkce_random = mcp_oauth::generate_state();
    // Combined state sent to the OAuth server: "{flow_id}.{random_state}"
    let pkce_state = format!("{flow_id}.{pkce_random}");

    // Store PKCE state in vault under per-flow keys for the callback to retrieve.
    let flow_vault_key =
        |field: &str| KernelOAuthProvider::vault_key(&format!("{server_url}:{flow_id}"), field);
    let store =
        |field: &str, value: &str| -> Result<(), librefang_runtime::mcp_oauth::McpOAuthError> {
            provider.vault_set(&flow_vault_key(field), value)
        };
    if let Err(e) = store("pkce_verifier", &pkce_verifier) {
        tracing::error!(error = %e, "Failed to store PKCE verifier in vault");
        return ApiErrorResponse::internal(format!(
            "Failed to store auth state: {e}. Ensure LIBREFANG_VAULT_KEY is set in Docker."
        ))
        .into_json_tuple();
    }
    if let Err(e) = store("pkce_state", &pkce_state) {
        tracing::error!(error = %e, "Failed to store PKCE state in vault");
        return ApiErrorResponse::internal(format!("Failed to store auth state: {e}"))
            .into_json_tuple();
    }
    if let Err(e) = store("token_endpoint", &metadata.token_endpoint) {
        tracing::warn!(error = %e, "Failed to store token_endpoint in vault");
    }
    // #3713: persist the original authorization-server host so the callback
    // can re-verify that the stored `token_endpoint` still resolves to the
    // same host the user authorized against. Without this pin, a malicious
    // (or mid-flow tampered) discovery response could redirect the
    // authorization-code exchange to an attacker-controlled endpoint and
    // exfiltrate the auth code. We pin against `server_url`'s host because
    // that is the URL the operator placed in `config.toml` — the only value
    // in the flow that the attacker cannot influence.
    if let Some(issuer_host) = url_host_lower(&server_url) {
        if let Err(e) = store("issuer_host", &issuer_host) {
            tracing::warn!(error = %e, "Failed to store issuer_host in vault");
        }
    } else {
        tracing::warn!(server_url = %server_url, "server_url has no host — cannot pin issuer for callback");
    }
    if let Err(e) = store("redirect_uri", &redirect_uri) {
        tracing::warn!(error = %e, "Failed to store redirect_uri in vault");
    }
    if let Some(ref cid) = client_id {
        if let Err(e) = store("client_id", cid) {
            tracing::warn!(error = %e, "Failed to store client_id in vault");
        }
    }

    // Build authorization URL
    let mut auth_url = format!(
        "{}?response_type=code&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
        metadata.authorization_endpoint,
        percent_encode_param(&redirect_uri),
        percent_encode_param(&pkce_challenge),
        percent_encode_param(&pkce_state),
    );
    if let Some(ref cid) = client_id {
        auth_url.push_str(&format!("&client_id={}", percent_encode_param(cid)));
    }
    if !metadata.scopes.is_empty() {
        let scope_str = metadata.scopes.join(" ");
        auth_url.push_str(&format!("&scope={}", percent_encode_param(&scope_str)));
    }
    if !metadata.user_scopes.is_empty() {
        let user_scope_str = metadata.user_scopes.join(" ");
        auth_url.push_str(&format!(
            "&user_scope={}",
            percent_encode_param(&user_scope_str)
        ));
    }

    // Update auth state
    {
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::PendingAuth {
                auth_url: auth_url.clone(),
            },
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "auth_url": auth_url,
            "server": name,
        })),
    )
}

/// Query parameters for the OAuth callback.
#[derive(serde::Deserialize)]
pub struct AuthCallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// GET /api/mcp/servers/{name}/auth/callback
///
/// OAuth callback endpoint. The authorization server redirects here after
/// the user authorizes. Exchanges the authorization code for tokens using
/// the stored PKCE verifier, stores the tokens, and retries the MCP
/// connection.
pub async fn auth_callback(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<AuthCallbackParams>,
) -> Response {
    // #3730: Validate the `state` parameter before doing anything else.
    //
    // The state must be present and in the form "{flow_id}.{random_nonce}".
    // This proves the caller initiated a flow via auth_start — the nonce is
    // stored in the vault keyed by flow_id and is never sent over any
    // side-channel.  Validating here prevents unauthenticated callers from
    // probing the endpoint or mutating server-side auth state.
    let received_state = match params.state {
        Some(ref s) => s.clone(),
        None => {
            return auth_failed("Missing state parameter.");
        }
    };

    // #3727: Extract the flow_id from the state parameter.
    // The state format set by auth_start is "{flow_id}.{random_state}".
    // A missing dot means this callback was not initiated by this daemon.
    let flow_id = match received_state.split_once('.') {
        Some((fid, _)) if !fid.is_empty() => fid.to_string(),
        _ => {
            return auth_failed(
                "Malformed state parameter — no valid flow ID found. \
                 This callback may not have been initiated by this server.",
            );
        }
    };

    // Find server config to get URL
    let cfg = state.kernel.config_snapshot();
    let server_url = match cfg.mcp_servers.iter().find(|s| s.name == name) {
        Some(entry) => match &entry.transport {
            Some(librefang_types::config::McpTransportEntry::Http { url }) => url.clone(),
            Some(librefang_types::config::McpTransportEntry::Sse { url }) => url.clone(),
            _ => {
                return auth_failed("Server has no HTTP/SSE transport.");
            }
        },
        None => {
            return auth_failed(format!("MCP server '{name}' not found."));
        }
    };

    // Load stored PKCE state from vault using the per-flow key (#3727).
    let provider = KernelOAuthProvider::new(state.kernel.home_dir().to_path_buf());
    let flow_key_prefix = format!("{server_url}:{flow_id}");
    // #3750: collapse vault Result into Option for callers below — a vault
    // storage failure during callback is logged and treated the same as
    // "value missing", since the recovery path (retry from dashboard) is
    // identical for both cases.
    let load = |field: &str| -> Option<String> {
        match provider.vault_get(&KernelOAuthProvider::vault_key(&flow_key_prefix, field)) {
            Ok(opt) => opt,
            Err(e) => {
                tracing::warn!(
                    field = %field,
                    error = %e,
                    "vault_get failed during OAuth callback"
                );
                None
            }
        }
    };

    let stored_state = match load("pkce_state") {
        Some(s) => s,
        None => {
            tracing::error!(
                server = %name,
                server_url = %server_url,
                flow_id = %flow_id,
                "PKCE state not found in vault — vault may not be initialized, \
                 LIBREFANG_VAULT_KEY not set, or unknown flow_id"
            );
            return auth_failed(
                "No pending auth flow found for this flow ID (PKCE state missing from vault). \
                 Check that LIBREFANG_VAULT_KEY is set in your environment.",
            );
        }
    };

    // #3730: The stored state encodes the flow_id and a random nonce; the
    // received state must match exactly, which proves the caller initiated this
    // specific flow.  This subsumes the earlier server-name prefix binding
    // (#3911) — the per-flow vault key + exact-match state already binds each
    // callback to one specific in-flight authorization.
    // Use constant-time comparison to prevent timing attacks.
    let received_bytes = received_state.as_bytes();
    let stored_bytes = stored_state.as_bytes();
    let nonce_match = received_bytes.len() == stored_bytes.len()
        && bool::from(received_bytes.ct_eq(stored_bytes));
    if !nonce_match {
        tracing::warn!(
            server = %name,
            "OAuth state validation failed — possible CSRF or cross-flow replay"
        );
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Error {
                message: "OAuth state mismatch - possible CSRF".to_string(),
            },
        );
        return auth_failed("State parameter mismatch. This may indicate a CSRF attack.");
    }

    // State is valid — now safe to inspect the OAuth response from the provider.
    // Handling the `error` param here (after state validation) prevents
    // unauthenticated callers from injecting arbitrary error messages into
    // auth state (#3730).
    if let Some(ref error) = params.error {
        let desc = params.error_description.as_deref().unwrap_or("");
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Error {
                message: format!("{error}: {desc}"),
            },
        );
        return auth_failed(format!("{error}: {desc}"));
    }

    let code = match params.code {
        Some(ref c) => c.clone(),
        None => {
            return auth_failed("Missing authorization code.");
        }
    };

    let pkce_verifier = match load("pkce_verifier") {
        Some(v) => v,
        None => {
            return auth_failed("PKCE verifier missing from vault.");
        }
    };

    let token_endpoint = match load("token_endpoint") {
        Some(t) => t,
        None => {
            return auth_failed("Token endpoint missing from vault.");
        }
    };
    // SSRF guard (#3623): re-validate the stored token_endpoint before the
    // outbound code exchange.  The parser checks at discovery time, but the
    // value sat in the vault between then and now and may predate a tightening
    // of the SSRF policy — the kernel's `try_refresh` already does this; this
    // is the matching guard for the auth-code path.
    if let Err(reason) = mcp_oauth::is_ssrf_blocked_url(&token_endpoint) {
        return auth_failed(format!(
            "SSRF: token_endpoint rejected for code exchange: {reason}"
        ));
    }

    // #3713: pin the token-exchange target to the authorization server's
    // original host. The discovery metadata's `token_endpoint` is attacker-
    // influenced data; it must not be trusted to point anywhere outside the
    // host the user originally authorized against. If the stored issuer host
    // is missing (e.g. an in-flight flow predating this guard) or does not
    // match `token_endpoint.host()`, refuse the exchange — never POST the
    // code to an unverified host.
    let issuer_host = match load("issuer_host") {
        Some(h) if !h.is_empty() => h,
        _ => {
            tracing::error!(
                server = %name,
                token_endpoint = %token_endpoint,
                "issuer_host missing from vault — refusing token exchange (#3713)"
            );
            return auth_failed(
                "Authorization server host pin missing from vault — refusing to exchange the auth code. Please retry the sign-in from the dashboard.",
            );
        }
    };
    if !token_endpoint_host_matches(&token_endpoint, &issuer_host) {
        let token_host = url_host_lower(&token_endpoint).unwrap_or_default();
        tracing::error!(
            server = %name,
            token_endpoint = %token_endpoint,
            issuer_host = %issuer_host,
            token_host = %token_host,
            "token_endpoint host does not match authorization server host — refusing token exchange (possible metadata-tamper attack, #3713)"
        );
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Error {
                message: "token_endpoint host mismatch — refused to exchange auth code".to_string(),
            },
        );
        return auth_failed(
            "Token endpoint host does not match the authorization server host. Refusing to exchange the auth code.",
        );
    }

    let client_id = load("client_id");
    let redirect_uri = match load("redirect_uri") {
        Some(r) if !r.is_empty() => r,
        _ => {
            return auth_failed(
                "Redirect URI missing from vault — auth flow state was lost. \
                 Please retry from the dashboard.",
            );
        }
    };

    // Exchange authorization code for tokens.
    // Use the proxy-aware client so token endpoint requests respect proxy config
    // and inherit default connect/read timeouts (prevents hung token exchanges).
    let http_client = librefang_runtime::http_client::proxied_client();
    let mut form_params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", pkce_verifier),
    ];
    if let Some(ref cid) = client_id {
        form_params.push(("client_id", cid.clone()));
    }

    // #3730: user-visible errors must NOT leak the token endpoint URL or the
    // raw response body — both can include internal hostnames, query
    // parameters, or provider error payloads that contain sensitive context.
    // Detailed diagnostics go to tracing (operator-only); the user/dashboard
    // sees a generic message.
    const GENERIC_TOKEN_EXCHANGE_FAILED: &str =
        "Token exchange failed. Check the daemon logs for details.";

    let token_resp = match http_client
        .post(&token_endpoint)
        .form(&form_params)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(
                server = %name,
                token_endpoint = %token_endpoint,
                error = %e,
                "OAuth token exchange request failed"
            );
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(
                name.clone(),
                McpAuthState::Error {
                    message: GENERIC_TOKEN_EXCHANGE_FAILED.to_string(),
                },
            );
            return auth_failed(GENERIC_TOKEN_EXCHANGE_FAILED);
        }
    };

    if !token_resp.status().is_success() {
        let status = token_resp.status();
        let body_raw = token_resp.text().await.unwrap_or_default();
        // Truncate operator-visible body for tracing; user gets generic msg.
        let body_preview: String = body_raw.chars().take(500).collect();
        tracing::error!(
            server = %name,
            token_endpoint = %token_endpoint,
            status = %status,
            body_preview = %body_preview,
            "OAuth token exchange returned non-success status"
        );
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Error {
                message: GENERIC_TOKEN_EXCHANGE_FAILED.to_string(),
            },
        );
        return auth_failed(GENERIC_TOKEN_EXCHANGE_FAILED);
    }

    let body = match token_resp.text().await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(
                server = %name,
                token_endpoint = %token_endpoint,
                error = %e,
                "Failed to read OAuth token response body"
            );
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(
                name.clone(),
                McpAuthState::Error {
                    message: GENERIC_TOKEN_EXCHANGE_FAILED.to_string(),
                },
            );
            return auth_failed(GENERIC_TOKEN_EXCHANGE_FAILED);
        }
    };

    let tokens: OAuthTokens = match serde_json::from_str(&body) {
        Ok(t) => t,
        Err(e) => {
            let body_preview: String = body.chars().take(500).collect();
            tracing::error!(
                server = %name,
                token_endpoint = %token_endpoint,
                error = %e,
                body_preview = %body_preview,
                "Failed to parse OAuth token response"
            );
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(
                name.clone(),
                McpAuthState::Error {
                    message: GENERIC_TOKEN_EXCHANGE_FAILED.to_string(),
                },
            );
            return auth_failed(GENERIC_TOKEN_EXCHANGE_FAILED);
        }
    };

    // Store tokens via the trait provider
    let trait_provider = state.kernel.oauth_provider_ref();
    if let Err(e) = trait_provider.store_tokens(&server_url, tokens).await {
        tracing::warn!(error = %e, "Failed to store OAuth tokens");
    }

    // Promote `token_endpoint` (and `client_id` if registered via DCR) from
    // the per-flow staging namespace into the durable per-server namespace
    // BEFORE the PKCE cleanup loop deletes them. The kernel's `try_refresh`
    // path reads these from `{server_url}/...`, not the per-flow keys; if
    // we skip this step the refresh fails on the first access-token expiry
    // with `No token_endpoint stored for refresh` and the user is bounced
    // back through a fresh OAuth flow each session.
    //
    // NOTE: concurrent OAuth flows for the same `server_url` race here on
    // the bare per-server namespace — last write wins. This matches the
    // existing race in DCR persistence at L345-346 and is acceptable: two
    // simultaneous sign-ins for one server are an unusual operator action
    // and both arrive at the same metadata under normal use. No locking.
    //
    // Fail-open on `store_oauth_metadata` error (parity with the
    // `store_tokens` warn-and-continue at L823-825): the user has just
    // completed an interactive OAuth flow and we already have the access
    // token in hand. Returning a 5xx here would force them through another
    // browser round-trip — much worse UX than silently breaking refresh
    // until the next interactive sign-in, which is the *existing* failure
    // mode pre-#4547 anyway. The recovery path is: when refresh fails
    // (`McpOAuthError::MissingTokenEndpoint`), the daemon flips back to
    // `NeedsAuth` and the next message kicks off a fresh OAuth flow,
    // which re-runs this block. Operators see the failure in the
    // `Failed to persist OAuth metadata for refresh` log line above the
    // expected `No token_endpoint stored for refresh` later — that
    // pairing is the diagnostic signal.
    if let Err(e) = trait_provider
        .store_oauth_metadata(&server_url, &token_endpoint, client_id.as_deref())
        .await
    {
        tracing::warn!(error = %e, "Failed to persist OAuth metadata for refresh");
    }

    // Clean up one-time PKCE values from vault (per-flow key — #3727).
    //
    // #3651: replaced `let _ = vault_remove(...)` so vault crypto failures
    // during PKCE cleanup are no longer silently dropped. Behavior is
    // intentionally unchanged on success (one-time cleanup, errors don't
    // abort the OAuth callback path), but every failure now produces an
    // `audit` log line so operators can correlate stale PKCE entries with
    // a misconfigured `LIBREFANG_VAULT_KEY`.
    for field in &[
        "pkce_verifier",
        "pkce_state",
        "redirect_uri",
        "token_endpoint",
        "client_id",
    ] {
        let vault_key = KernelOAuthProvider::vault_key(&flow_key_prefix, field);
        if let Err(e) = provider.vault_remove(&vault_key) {
            tracing::error!(
                target: "audit",
                op = "vault_remove",
                key = %vault_key,
                error = %e,
                "vault op failed during PKCE cleanup"
            );
        }
    }

    // Retry the MCP connection now that we have tokens.
    // Awaited inline (blocks the browser tab up to the kernel's 60s tool-discovery timeout)
    // to avoid a transient "Authorized but Disconnected" state in the dashboard —
    // the UX cost is worth the state consistency.
    // The kernel's retry_mcp_connection is the single source of truth for setting
    // Authorized (on Ok) or Error (on Err) in mcp_auth_states.
    state.kernel.retry_mcp_connection(&name).await;

    callback_text("Authorization Complete\n\nYou can close this tab.".to_string())
}

/// DELETE /api/mcp/servers/{name}/auth/revoke
///
/// Revokes OAuth tokens for an MCP server and clears auth state.
#[utoipa::path(
    delete,
    path = "/api/mcp/servers/{name}/auth/revoke",
    tag = "mcp",
    params(
        ("name" = String, Path, description = "MCP server name"),
    ),
    responses(
        (status = 200, description = "Auth revoked", body = crate::types::JsonObject),
        (status = 404, description = "MCP server not found")
    )
)]
pub async fn auth_revoke(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Find server and resolve its URL
    let cfg = state.kernel.config_snapshot();
    let server_url = match cfg.mcp_servers.iter().find(|s| s.name == name) {
        Some(entry) => match &entry.transport {
            Some(librefang_types::config::McpTransportEntry::Http { url }) => url.clone(),
            Some(librefang_types::config::McpTransportEntry::Sse { url }) => url.clone(),
            _ => name.clone(), // fallback to name for non-HTTP transports
        },
        None => {
            return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
                .into_json_tuple();
        }
    };

    // #3369: clear in-memory state first so the current process can no longer
    // use the cached tokens, even if the persistent vault wipe fails. Then
    // attempt the vault wipe; if that fails, surface the error so the UI can
    // tell the user the sign-out is incomplete on disk and prompt a retry.
    {
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(name.clone(), McpAuthState::NeedsAuth);
    }
    {
        let mut conns = state.kernel.mcp_connections_ref().lock().await;
        conns.retain(|c| c.name() != name);
    }

    let provider = state.kernel.oauth_provider_ref();
    if let Err(e) = provider.clear_tokens(&server_url).await {
        tracing::error!(server = %name, error = %e, "auth_revoke: vault clear failed");
        // #3750: surface VaultLocked / KeyNotFound / Io / Crypto distinctly so
        // the dashboard can render the right recovery prompt instead of a
        // generic 500.
        use librefang_runtime::mcp_oauth::McpOAuthError;
        let resp = match e {
            McpOAuthError::VaultLocked => ApiErrorResponse::bad_request(
                "Vault is locked — set LIBREFANG_VAULT_KEY before retrying sign-out.",
            )
            .with_status(axum::http::StatusCode::LOCKED)
            .with_code("vault_locked"),
            McpOAuthError::KeyNotFound(detail) => ApiErrorResponse::not_found(format!(
                "No stored tokens to clear: {detail}"
            ))
            .with_code("vault_key_not_found"),
            McpOAuthError::Io(io) => ApiErrorResponse::internal(format!(
                "Sign-out failed due to vault I/O error: {io}. Tokens may still be valid. Retry."
            ))
            .with_code("vault_io"),
            McpOAuthError::Crypto(detail) => ApiErrorResponse::internal(format!(
                "Sign-out partially failed: in-memory session cleared but stored tokens may remain in the vault. Retry. Details: {detail}"
            ))
            .with_code("vault_crypto"),
        };
        return resp.into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "server": name,
            "state": "not_required",
        })),
    )
}

/// Lowercased host component of a URL string, or None if the URL is
/// unparseable or has no host. Used to pin the OAuth flow's token endpoint
/// to the original authorization server's host (#3713).
fn url_host_lower(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// True iff `token_endpoint` parses to a URL whose host equals
/// `expected_host` (case-insensitive). A token endpoint with no host, an
/// unparseable URL, or a different host all return false — the caller MUST
/// refuse the code exchange in that case (#3713).
fn token_endpoint_host_matches(token_endpoint: &str, expected_host: &str) -> bool {
    match url_host_lower(token_endpoint) {
        Some(h) => h == expected_host.to_ascii_lowercase(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::UserRole;
    use axum::body::to_bytes;
    use axum::http::{HeaderName, HeaderValue};
    use librefang_types::agent::UserId;

    #[test]
    fn caller_fingerprint_is_stable_per_user() {
        let user = AuthenticatedApiUser {
            name: "alice".into(),
            role: UserRole::Owner,
            user_id: UserId::from_name("alice"),
        };
        let fp1 = caller_fingerprint(&Some(Extension(user.clone())));
        let fp2 = caller_fingerprint(&Some(Extension(user)));
        assert_eq!(fp1, fp2, "same user must produce identical fingerprint");
    }

    #[test]
    fn caller_fingerprint_differs_across_users() {
        let alice = AuthenticatedApiUser {
            name: "alice".into(),
            role: UserRole::Owner,
            user_id: UserId::from_name("alice"),
        };
        let bob = AuthenticatedApiUser {
            name: "bob".into(),
            role: UserRole::Owner,
            user_id: UserId::from_name("bob"),
        };
        let fp_a = caller_fingerprint(&Some(Extension(alice)));
        let fp_b = caller_fingerprint(&Some(Extension(bob)));
        assert_ne!(fp_a, fp_b, "distinct users must produce distinct prefixes");
    }

    #[test]
    fn caller_fingerprint_anonymous_is_stable() {
        let fp1 = caller_fingerprint(&None);
        let fp2 = caller_fingerprint(&None);
        assert_eq!(fp1, fp2);
    }

    fn hdrs(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    const LISTEN: &str = "0.0.0.0:4545";

    /// Helper: extract the Content-Type header value from a Response as a String.
    fn content_type(resp: &Response) -> String {
        resp.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    }

    #[tokio::test]
    async fn auth_failed_sets_plain_text_content_type() {
        let resp = auth_failed("some error");
        assert_eq!(
            content_type(&resp),
            "text/plain; charset=utf-8",
            "auth_failed must return text/plain, not HTML"
        );
    }

    #[tokio::test]
    async fn auth_callback_error_param_is_not_html() {
        // Simulate a crafted XSS payload in the error detail.
        // The detail passes through as plain text — no HTML parsing/execution,
        // because the Content-Type is text/plain, not text/html.
        let payload = "<script>alert(1)</script>";
        let resp = auth_failed(payload);

        // Content-Type must be text/plain — this is the key XSS mitigation.
        assert_eq!(
            content_type(&resp),
            "text/plain; charset=utf-8",
            "Content-Type must be text/plain, not text/html"
        );

        // The body must NOT be wrapped in HTML tags — confirm no <html>/<body> shell.
        let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            !body_str.contains("<html"),
            "Body must not contain HTML wrapper, got: {body_str}"
        );
        assert!(
            !body_str.contains("<body"),
            "Body must not contain HTML wrapper, got: {body_str}"
        );
    }

    #[tokio::test]
    async fn callback_text_sets_plain_text_content_type() {
        let resp = callback_text("Hello".to_string());
        assert_eq!(content_type(&resp), "text/plain; charset=utf-8");
    }

    #[test]
    fn spoofed_host_falls_back_to_loopback_when_allowlist_empty() {
        let h = hdrs(&[("host", "attacker.example")]);
        let url = derive_callback_url(&h, "srv", &[], LISTEN);
        assert!(
            !url.contains("attacker.example"),
            "spoofed Host must not appear in redirect_uri, got {url}"
        );
        assert!(url.starts_with("http://127.0.0.1:4545/"), "got {url}");
        assert!(url.ends_with("/api/mcp/servers/srv/auth/callback"));
    }

    #[test]
    fn spoofed_origin_falls_back_to_loopback_when_allowlist_empty() {
        let h = hdrs(&[("origin", "https://attacker.example")]);
        let url = derive_callback_url(&h, "srv", &[], LISTEN);
        assert!(!url.contains("attacker.example"), "got {url}");
        assert!(url.starts_with("http://127.0.0.1:4545/"), "got {url}");
    }

    #[test]
    fn spoofed_forwarded_host_falls_back_to_loopback_when_allowlist_empty() {
        let h = hdrs(&[
            ("x-forwarded-host", "attacker.example"),
            ("x-forwarded-proto", "https"),
        ]);
        let url = derive_callback_url(&h, "srv", &[], LISTEN);
        assert!(!url.contains("attacker.example"), "got {url}");
        assert!(url.starts_with("http://127.0.0.1:4545/"), "got {url}");
    }

    #[test]
    fn localhost_always_allowed_even_with_empty_allowlist() {
        let h = hdrs(&[("host", "localhost:4545")]);
        let url = derive_callback_url(&h, "srv", &[], LISTEN);
        assert_eq!(
            url,
            "http://localhost:4545/api/mcp/servers/srv/auth/callback"
        );
    }

    #[test]
    fn loopback_ip_always_allowed_even_with_empty_allowlist() {
        let h = hdrs(&[("host", "127.0.0.1:4545")]);
        let url = derive_callback_url(&h, "srv", &[], LISTEN);
        assert_eq!(
            url,
            "http://127.0.0.1:4545/api/mcp/servers/srv/auth/callback"
        );
    }

    #[test]
    fn allowlisted_origin_is_used_verbatim() {
        let trusted = vec!["dash.example.com".to_string()];
        let h = hdrs(&[("origin", "https://dash.example.com")]);
        let url = derive_callback_url(&h, "srv", &trusted, LISTEN);
        assert_eq!(
            url,
            "https://dash.example.com/api/mcp/servers/srv/auth/callback"
        );
    }

    #[test]
    fn allowlisted_forwarded_host_uses_forwarded_proto() {
        let trusted = vec!["dash.example.com".to_string()];
        let h = hdrs(&[
            ("x-forwarded-host", "dash.example.com"),
            ("x-forwarded-proto", "https"),
        ]);
        let url = derive_callback_url(&h, "srv", &trusted, LISTEN);
        assert_eq!(
            url,
            "https://dash.example.com/api/mcp/servers/srv/auth/callback"
        );
    }

    #[test]
    fn allowlisted_host_with_port_matches_bare_entry() {
        let trusted = vec!["dash.example.com".to_string()];
        let h = hdrs(&[("host", "dash.example.com:8080")]);
        let url = derive_callback_url(&h, "srv", &trusted, LISTEN);
        assert_eq!(
            url,
            "http://dash.example.com:8080/api/mcp/servers/srv/auth/callback"
        );
    }

    #[test]
    fn attacker_header_ignored_even_when_other_trusted_hosts_configured() {
        let trusted = vec!["dash.example.com".to_string()];
        let h = hdrs(&[
            ("origin", "https://attacker.example"),
            ("host", "attacker.example"),
        ]);
        let url = derive_callback_url(&h, "srv", &trusted, LISTEN);
        assert!(!url.contains("attacker.example"), "got {url}");
        assert!(url.starts_with("http://127.0.0.1:4545/"), "got {url}");
    }

    #[test]
    fn server_name_is_percent_encoded_into_path() {
        let h = hdrs(&[("host", "localhost:4545")]);
        let url = derive_callback_url(&h, "my-server", &[], LISTEN);
        assert!(url.contains("/api/mcp/servers/my-server/auth/callback"));
    }

    #[test]
    fn ipv6_listen_addr_collapses_to_ipv4_loopback_fallback() {
        let h = hdrs(&[("host", "attacker.example")]);
        let url = derive_callback_url(&h, "srv", &[], "[::]:4545");
        assert!(url.starts_with("http://127.0.0.1:4545/"), "got {url}");
    }

    #[test]
    fn origin_null_is_rejected() {
        let h = hdrs(&[("origin", "null")]);
        let url = derive_callback_url(&h, "srv", &[], LISTEN);
        assert!(url.starts_with("http://127.0.0.1:4545/"), "got {url}");
    }

    #[test]
    fn token_endpoint_matching_issuer_host_is_accepted() {
        assert!(token_endpoint_host_matches(
            "https://auth.example.com/oauth/token",
            "auth.example.com"
        ));
    }

    #[test]
    fn token_endpoint_with_different_host_is_rejected() {
        // The vulnerability scenario: discovery advertises a token endpoint
        // pointed at an attacker host. The callback must refuse.
        assert!(!token_endpoint_host_matches(
            "https://attacker.example/oauth/token",
            "auth.example.com"
        ));
    }

    #[test]
    fn token_endpoint_subdomain_is_rejected() {
        // Defense-in-depth: a sibling/child of the issuer host is still a
        // different origin and must not be trusted.
        assert!(!token_endpoint_host_matches(
            "https://evil.auth.example.com.attacker.example/oauth/token",
            "auth.example.com"
        ));
        assert!(!token_endpoint_host_matches(
            "https://api.auth.example.com/oauth/token",
            "auth.example.com"
        ));
    }

    #[test]
    fn token_endpoint_host_match_is_case_insensitive() {
        assert!(token_endpoint_host_matches(
            "https://AUTH.Example.COM/oauth/token",
            "auth.example.com"
        ));
    }

    #[test]
    fn unparseable_token_endpoint_is_rejected() {
        assert!(!token_endpoint_host_matches(
            "not a url",
            "auth.example.com"
        ));
    }

    #[test]
    fn url_host_lower_extracts_lowercased_host() {
        assert_eq!(
            url_host_lower("https://Auth.Example.COM/path"),
            Some("auth.example.com".to_string())
        );
        assert_eq!(url_host_lower("not a url"), None);
    }
}
