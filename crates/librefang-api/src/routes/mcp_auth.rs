//! MCP OAuth authentication endpoints.
//!
//! Provides auth status, flow initiation (UI-driven PKCE), callback
//! handling, and token revocation for MCP servers that require OAuth 2.0
//! authorization.

use super::AppState;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::mcp_oauth_provider::KernelOAuthProvider;
use librefang_runtime::mcp_oauth::{self, McpAuthState, OAuthTokens};
use std::sync::Arc;

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
        (status = 200, description = "Auth status for the MCP server", body = serde_json::Value),
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
        (status = 200, description = "Auth flow started — returns auth URL", body = serde_json::Value),
        (status = 400, description = "Server has no HTTP transport or discovery failed"),
        (status = 404, description = "MCP server not found")
    )
)]
pub async fn auth_start(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
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
    let mut client_id = metadata
        .client_id
        .clone()
        .or_else(|| provider.vault_get(&KernelOAuthProvider::vault_key(&server_url, "client_id")));

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
                    let _ = provider.vault_set(
                        &KernelOAuthProvider::vault_key(&server_url, "client_id"),
                        &cid,
                    );
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

    // Generate PKCE challenge and state
    let (pkce_verifier, pkce_challenge) = mcp_oauth::generate_pkce();
    let pkce_state = mcp_oauth::generate_state();

    // Store PKCE state in vault for the callback to retrieve
    let store = |field: &str, value: &str| -> Result<(), String> {
        provider.vault_set(&KernelOAuthProvider::vault_key(&server_url, field), value)
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
) -> impl IntoResponse {
    // Handle error response from authorization server
    if let Some(ref error) = params.error {
        let desc = params.error_description.as_deref().unwrap_or("");
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Error {
                message: format!("{error}: {desc}"),
            },
        );
        return axum::response::Html(format!(
            "<html><body>\
             <h2>Authorization Failed</h2>\
             <p>{error}: {desc}</p>\
             <p>You can close this tab.</p>\
             </body></html>"
        ));
    }

    let code = match params.code {
        Some(ref c) => c.clone(),
        None => {
            return axum::response::Html(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>Missing authorization code.</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
                    .to_string(),
            );
        }
    };

    let received_state = match params.state {
        Some(ref s) => s.clone(),
        None => {
            return axum::response::Html(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>Missing state parameter.</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
                    .to_string(),
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
                return axum::response::Html(
                    "<html><body>\
                     <h2>Authorization Failed</h2>\
                     <p>Server has no HTTP/SSE transport.</p>\
                     <p>You can close this tab.</p>\
                     </body></html>"
                        .to_string(),
                );
            }
        },
        None => {
            return axum::response::Html(format!(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>MCP server '{}' not found.</p>\
                 <p>You can close this tab.</p>\
                 </body></html>",
                name
            ));
        }
    };

    // Load stored PKCE state from vault
    let provider = KernelOAuthProvider::new(state.kernel.home_dir().to_path_buf());
    let load =
        |field: &str| provider.vault_get(&KernelOAuthProvider::vault_key(&server_url, field));

    let stored_state = match load("pkce_state") {
        Some(s) => s,
        None => {
            tracing::error!(
                server = %name,
                server_url = %server_url,
                "PKCE state not found in vault — vault may not be initialized or \
                 LIBREFANG_VAULT_KEY not set"
            );
            return axum::response::Html(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>No pending auth flow found (PKCE state missing from vault).</p>\
                 <p>Check that LIBREFANG_VAULT_KEY is set in your environment.</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
                    .to_string(),
            );
        }
    };

    // Validate state
    if received_state != stored_state {
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Error {
                message: "OAuth state mismatch - possible CSRF".to_string(),
            },
        );
        return axum::response::Html(
            "<html><body>\
             <h2>Authorization Failed</h2>\
             <p>State parameter mismatch. This may indicate a CSRF attack.</p>\
             <p>You can close this tab.</p>\
             </body></html>"
                .to_string(),
        );
    }

    let pkce_verifier = match load("pkce_verifier") {
        Some(v) => v,
        None => {
            return axum::response::Html(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>PKCE verifier missing from vault.</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
                    .to_string(),
            );
        }
    };

    let token_endpoint = match load("token_endpoint") {
        Some(t) => t,
        None => {
            return axum::response::Html(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>Token endpoint missing from vault.</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
                    .to_string(),
            );
        }
    };

    let client_id = load("client_id");
    let redirect_uri = load("redirect_uri").unwrap_or_default();

    // Exchange authorization code for tokens
    let http_client = reqwest::Client::new();
    let mut form_params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", pkce_verifier),
    ];
    if let Some(ref cid) = client_id {
        form_params.push(("client_id", cid.clone()));
    }

    let token_resp = match http_client
        .post(&token_endpoint)
        .form(&form_params)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            let msg = format!("Token exchange request failed: {e}");
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(
                name.clone(),
                McpAuthState::Error {
                    message: msg.clone(),
                },
            );
            return axum::response::Html(format!(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>{msg}</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
            ));
        }
    };

    if !token_resp.status().is_success() {
        let status = token_resp.status();
        let body = token_resp.text().await.unwrap_or_default();
        let msg = format!("Token exchange failed (HTTP {status}): {body}");
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Error {
                message: msg.clone(),
            },
        );
        return axum::response::Html(format!(
            "<html><body>\
             <h2>Authorization Failed</h2>\
             <p>{msg}</p>\
             <p>You can close this tab.</p>\
             </body></html>"
        ));
    }

    let body = match token_resp.text().await {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("Failed to read token response body: {e}");
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(
                name.clone(),
                McpAuthState::Error {
                    message: msg.clone(),
                },
            );
            return axum::response::Html(format!(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>{msg}</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
            ));
        }
    };

    let tokens: OAuthTokens = match serde_json::from_str(&body) {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("Failed to parse token response: {e}. Body: {body}");
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(
                name.clone(),
                McpAuthState::Error {
                    message: msg.clone(),
                },
            );
            return axum::response::Html(format!(
                "<html><body>\
                 <h2>Authorization Failed</h2>\
                 <p>{msg}</p>\
                 <p>You can close this tab.</p>\
                 </body></html>"
            ));
        }
    };

    // Store tokens via the trait provider
    let trait_provider = state.kernel.oauth_provider_ref();
    if let Err(e) = trait_provider.store_tokens(&server_url, tokens).await {
        tracing::warn!(error = %e, "Failed to store OAuth tokens");
    }

    // Clean up one-time PKCE values from vault
    for field in &["pkce_verifier", "pkce_state", "redirect_uri"] {
        let _ = provider.vault_remove(&KernelOAuthProvider::vault_key(&server_url, field));
    }

    // Update auth state to Authorized
    {
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::Authorized {
                expires_at: None,
                tokens: None,
            },
        );
    }

    // Retry the MCP connection now that we have tokens.
    // Await inline so the connection is established before the response,
    // avoiding the brief "Authorized but Disconnected" state in the dashboard.
    state.kernel.retry_mcp_connection(&name).await;

    axum::response::Html(
        "<html><body>\
         <h2>Authorization Complete</h2>\
         <p>You can close this tab.</p>\
         <script>window.close()</script>\
         </body></html>"
            .to_string(),
    )
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
        (status = 200, description = "Auth revoked", body = serde_json::Value),
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

    // Clear tokens via provider (keyed by server URL, not name)
    let provider = state.kernel.oauth_provider_ref();
    if let Err(e) = provider.clear_tokens(&server_url).await {
        tracing::warn!(server = %name, error = %e, "Failed to clear OAuth tokens");
    }

    // Set auth state to NeedsAuth so the dashboard shows the Authorize button
    {
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(name.clone(), McpAuthState::NeedsAuth);
    }

    // Remove from MCP connections so next reconnect is clean
    {
        let mut conns = state.kernel.mcp_connections_ref().lock().await;
        conns.retain(|c| c.name() != name);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "server": name,
            "state": "not_required",
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};

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
}
