//! MCP OAuth authentication endpoints.
//!
//! Provides auth status, flow initiation, and token revocation for
//! MCP servers that require OAuth 2.0 authorization.

use super::AppState;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_runtime::mcp_oauth::{self, McpAuthState, McpOAuthConfig};
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
        "not_required"
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "server": name,
            "auth": { "state": state_label },
        })),
    )
}

/// POST /api/mcp/servers/{name}/auth/start
///
/// Initiates an OAuth authorization flow for the specified MCP server.
/// Discovers OAuth metadata from the server URL, starts the flow via the
/// configured provider, and returns the authorization URL.
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
    let metadata = match mcp_oauth::discover_oauth_metadata(&server_url, &oauth_config).await {
        Ok(m) => m,
        Err(e) => {
            // Store error state
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(name.clone(), McpAuthState::Error { message: e.clone() });
            return ApiErrorResponse::bad_request(format!("OAuth discovery failed: {e}"))
                .into_json_tuple();
        }
    };

    // Start auth flow via provider
    let provider = state.kernel.oauth_provider_ref();
    let handle = match provider
        .start_auth_flow(&name, &metadata, &oauth_config)
        .await
    {
        Ok(h) => h,
        Err(e) => {
            let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
            auth_states.insert(name.clone(), McpAuthState::Error { message: e.clone() });
            return ApiErrorResponse::internal(format!("Failed to start OAuth flow: {e}"))
                .into_json_tuple();
        }
    };

    let auth_url = handle.auth_url.clone();

    // Store PendingAuth state
    {
        let mut auth_states = state.kernel.mcp_auth_states_ref().lock().await;
        auth_states.insert(
            name.clone(),
            McpAuthState::PendingAuth {
                auth_url: auth_url.clone(),
            },
        );
    }

    // Spawn background task to wait for completion
    let kernel = Arc::clone(&state.kernel);
    let server_name = name.clone();
    tokio::spawn(async move {
        match handle.completion.await {
            Ok(Ok(tokens)) => {
                tracing::info!(server = %server_name, "MCP OAuth flow completed successfully");
                let mut auth_states = kernel.mcp_auth_states_ref().lock().await;
                auth_states.insert(
                    server_name.clone(),
                    McpAuthState::Authorized {
                        tokens: Some(tokens),
                    },
                );
                drop(auth_states);
                kernel.retry_mcp_connection(&server_name).await;
            }
            Ok(Err(e)) => {
                tracing::warn!(server = %server_name, error = %e, "MCP OAuth flow failed");
                let mut auth_states = kernel.mcp_auth_states_ref().lock().await;
                auth_states.insert(server_name, McpAuthState::Error { message: e });
            }
            Err(_) => {
                tracing::warn!(server = %server_name, "MCP OAuth flow cancelled (sender dropped)");
                let mut auth_states = kernel.mcp_auth_states_ref().lock().await;
                auth_states.insert(
                    server_name,
                    McpAuthState::Error {
                        message: "Auth flow cancelled".to_string(),
                    },
                );
            }
        }
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "auth_url": auth_url,
            "server": name,
        })),
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
