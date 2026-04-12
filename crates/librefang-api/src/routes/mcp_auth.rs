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
use librefang_types::config::McpOAuthConfig;
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

/// Derive the OAuth callback URL from the incoming request headers.
fn derive_callback_url(headers: &HeaderMap, server_name: &str) -> String {
    // Try Origin header first
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if !origin.is_empty() && origin != "null" {
            return format!("{}/api/mcp/servers/{}/auth/callback", origin, server_name);
        }
    }
    // Try X-Forwarded-Host + X-Forwarded-Proto
    if let Some(fwd_host) = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
    {
        let proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("https");
        return format!(
            "{}://{}/api/mcp/servers/{}/auth/callback",
            proto, fwd_host, server_name
        );
    }
    // Fall back to Host header
    if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
        return format!(
            "http://{}/api/mcp/servers/{}/auth/callback",
            host, server_name
        );
    }
    // Last resort
    format!(
        "http://localhost:4545/api/mcp/servers/{}/auth/callback",
        server_name
    )
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

    // Discover OAuth metadata
    let oauth_config = McpOAuthConfig::default();
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

    // Derive the redirect URI from the incoming request
    let redirect_uri = derive_callback_url(&headers, &name);

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
    let _ = store("token_endpoint", &metadata.token_endpoint);
    let _ = store("redirect_uri", &redirect_uri);
    if let Some(ref cid) = client_id {
        let _ = store("client_id", cid);
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

    let tokens: OAuthTokens = match token_resp.json().await {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("Failed to parse token response: {e}");
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

    // Retry the MCP connection now that we have tokens
    let kernel = Arc::clone(&state.kernel);
    let server_name = name.clone();
    tokio::spawn(async move {
        kernel.retry_mcp_connection(&server_name).await;
    });

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
    // Verify server exists
    let cfg = state.kernel.config_snapshot();
    if !cfg.mcp_servers.iter().any(|s| s.name == name) {
        return ApiErrorResponse::not_found(format!("MCP server '{}' not found", name))
            .into_json_tuple();
    }

    // Clear tokens via provider
    let provider = state.kernel.oauth_provider_ref();
    if let Err(e) = provider.clear_tokens(&name).await {
        tracing::warn!(server = %name, error = %e, "Failed to clear OAuth tokens");
    }

    // Remove auth state
    {
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.remove(&name);
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
