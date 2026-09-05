//! Credential-vault write surface (#8164).
//!
//! * `GET /api/vault/keys` — which writable keys are currently set. Names and a boolean only.
//! * `PUT /api/vault/keys/{key}` — store a secret under a writable key.
//! * `DELETE /api/vault/keys/{key}` — remove it.
//!
//! There is deliberately no read-back endpoint. A secret that the API can hand back is a secret one leaked log line, one browser cache entry, or one over-broad token away from being public; the vault's only consumers are in-process (`resolve_github_token` and friends), so nothing needs the value over HTTP.
//!
//! # Why an allowlist rather than arbitrary key names
//!
//! The vault is a single flat namespace shared with the MCP OAuth flow, which stores `mcp-oauth:{server_url}:client_secret` entries there. An endpoint that accepted any key would let an authenticated caller overwrite another server's OAuth client secret, and a listing that returned every key would disclose the set of MCP servers an operator has authenticated against. [`WRITABLE_KEYS`] therefore names exactly the keys a surface is allowed to manage; extending it is a one-line change plus the reasoning for why that key belongs on an operator-facing form.
//!
//! # Hot reload
//!
//! Writes go through [`librefang_kernel::KernelApi::vault_set`], which mutates the same lazily-unlocked `CredentialVault` that [`librefang_kernel::KernelApi::vault_get`] reads from — the `Arc<RwLock<…>>` cached on the kernel by `vault_handle()` (#3598). `CredentialVault::set` inserts into that in-memory map *and* re-encrypts to disk, so the next request that calls `vault_get` observes the new value with no restart and no cache to invalidate.
//!
//! The stale-cache hazard the issue describes is real but belongs to a different path: `KernelOAuthProvider::vault_set` and the `librefang vault set` CLI each construct their own `CredentialVault`, so a write there updates the file while the running daemon keeps serving its cached map. Routing this endpoint through the kernel accessor is what avoids reproducing that surprise in-process.
//!
//! # Hosts where the vault cannot be unlocked
//!
//! With no OS keyring and no `LIBREFANG_VAULT_KEY`, `vault_set` fails and this endpoint returns `503` naming the failure. It deliberately does not fall back to writing `~/.librefang/secrets.env`: that would answer "store this secret securely" by putting it on disk in cleartext, and the operator would have no way to tell from the `200` which of the two happened.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use super::AppState;
use crate::middleware::{AuthenticatedApiUser, UserRole};
use crate::types::ApiErrorResponse;

/// Vault keys that may be written or deleted over HTTP, sorted (#3298).
///
/// `GITHUB_TOKEN` is the fallback `routes::skills::resolve_github_token` consults for `POST /api/skills/{name}/propose` and `POST /api/templates/{name}/promote`.
pub const WRITABLE_KEYS: &[&str] = &["GITHUB_TOKEN"];

/// Longest secret accepted. Comfortably above any provider token; a body larger than this is a mistake, not a credential.
const MAX_SECRET_LEN: usize = 8192;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/vault/keys", axum::routing::get(vault_list_keys))
        .route(
            "/vault/keys/{key}",
            axum::routing::put(vault_put_key).delete(vault_delete_key),
        )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VaultSetRequest {
    /// The secret to store. Surrounding whitespace is trimmed — a token pasted with a trailing newline is the common case and would otherwise be sent in an `Authorization` header verbatim.
    pub value: String,
}

/// Reject the request unless the caller is an authenticated `Admin`+.
///
/// Writing a daemon-wide credential is an operator action. The trusted loopback / `LIBREFANG_ALLOW_NO_AUTH=1` path is accepted because the middleware injects a synthetic Owner there, which is what keeps the dashboard usable on a single-user install.
fn require_admin(state: &AppState, api_user: Option<&AuthenticatedApiUser>) -> Option<Response> {
    match api_user {
        Some(u) if u.role >= UserRole::Admin => None,
        Some(u) => {
            state.kernel.audit().record_with_context(
                "system",
                librefang_kernel::audit::AuditAction::PermissionDenied,
                format!("vault endpoint denied for role {}", u.role),
                "denied",
                Some(u.user_id),
                Some("api".to_string()),
            );
            Some(
                ApiErrorResponse::forbidden("Admin role required for vault access").into_response(),
            )
        }
        None => {
            state.kernel.audit().record_with_context(
                "system",
                librefang_kernel::audit::AuditAction::PermissionDenied,
                "vault endpoint denied for anonymous caller",
                "denied",
                None,
                Some("api".to_string()),
            );
            Some(
                ApiErrorResponse::unauthorized("Admin credential required for vault access")
                    .into_response(),
            )
        }
    }
}

/// Resolve a path-supplied key against [`WRITABLE_KEYS`].
///
/// Returns the `&'static str` from the allowlist rather than the caller's string, so nothing downstream can be reached with a key this module never vetted.
fn writable_key(key: &str) -> Option<&'static str> {
    WRITABLE_KEYS.iter().copied().find(|c| *c == key)
}

fn not_a_writable_key(key: &str) -> Response {
    ApiErrorResponse::not_found(format!(
        "'{key}' is not a vault key this API may write; writable keys: {}",
        WRITABLE_KEYS.join(", ")
    ))
    .into_response()
}

fn vault_unavailable(error: &str) -> Response {
    ApiErrorResponse::internal(format!("Vault unavailable: {error}"))
        .with_status(StatusCode::SERVICE_UNAVAILABLE)
        .into_response()
}

#[utoipa::path(
    get,
    path = "/api/vault/keys",
    tag = "vault",
    responses(
        (status = 200, description = "Writable vault keys and whether each is set", body = crate::types::JsonObject),
        (status = 401, description = "Admin credential required"),
        (status = 403, description = "Admin role required"),
    )
)]
pub async fn vault_list_keys(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    if let Some(deny) = require_admin(&state, api_user.as_ref().map(|e| &e.0)) {
        return deny;
    }
    let keys: Vec<serde_json::Value> = WRITABLE_KEYS
        .iter()
        .map(|key| {
            serde_json::json!({
                "key": key,
                "set": state.kernel.vault_get(key).is_some_and(|v| !v.trim().is_empty()),
            })
        })
        .collect();
    Json(serde_json::json!({ "keys": keys })).into_response()
}

#[utoipa::path(
    put,
    path = "/api/vault/keys/{key}",
    tag = "vault",
    params(("key" = String, Path, description = "Vault key name")),
    request_body = VaultSetRequest,
    responses(
        (status = 200, description = "Secret stored", body = crate::types::JsonObject),
        (status = 400, description = "Empty or oversized value"),
        (status = 401, description = "Admin credential required"),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Key is not writable over HTTP"),
        (status = 503, description = "Vault could not be unlocked or written"),
    )
)]
pub async fn vault_put_key(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
    Json(req): Json<VaultSetRequest>,
) -> Response {
    if let Some(deny) = require_admin(&state, api_user.as_ref().map(|e| &e.0)) {
        return deny;
    }
    let Some(key) = writable_key(&key) else {
        return not_a_writable_key(&key);
    };
    let value = req.value.trim();
    if value.is_empty() {
        return ApiErrorResponse::bad_request(format!(
            "secret value for '{key}' must not be empty; use DELETE to clear it"
        ))
        .into_response();
    }
    if value.len() > MAX_SECRET_LEN {
        return ApiErrorResponse::bad_request(format!(
            "secret value for '{key}' exceeds {MAX_SECRET_LEN} bytes"
        ))
        .into_response();
    }

    if let Err(error) = state.kernel.vault_set(key, value) {
        tracing::error!(%key, %error, "vault write failed");
        return vault_unavailable(&error);
    }
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("vault key {key} set"),
        "ok",
        api_user.as_ref().map(|e| e.0.user_id),
        Some("api".to_string()),
    );
    Json(serde_json::json!({ "key": key, "set": true })).into_response()
}

#[utoipa::path(
    delete,
    path = "/api/vault/keys/{key}",
    tag = "vault",
    params(("key" = String, Path, description = "Vault key name")),
    responses(
        (status = 200, description = "Secret removed (or already absent)", body = crate::types::JsonObject),
        (status = 401, description = "Admin credential required"),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Key is not writable over HTTP"),
        (status = 503, description = "Vault could not be unlocked or written"),
    )
)]
pub async fn vault_delete_key(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    if let Some(deny) = require_admin(&state, api_user.as_ref().map(|e| &e.0)) {
        return deny;
    }
    let Some(key) = writable_key(&key) else {
        return not_a_writable_key(&key);
    };
    let removed = match state.kernel.vault_remove(key) {
        Ok(removed) => removed,
        Err(error) => {
            tracing::error!(%key, %error, "vault delete failed");
            return vault_unavailable(&error);
        }
    };
    if removed {
        state.kernel.audit().record_with_context(
            "system",
            librefang_kernel::audit::AuditAction::ConfigChange,
            format!("vault key {key} removed"),
            "ok",
            api_user.as_ref().map(|e| e.0.user_id),
            Some("api".to_string()),
        );
    }
    Json(serde_json::json!({ "key": key, "set": false, "removed": removed })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_keys_are_sorted_and_unique() {
        let mut sorted = WRITABLE_KEYS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted, WRITABLE_KEYS,
            "WRITABLE_KEYS must stay sorted (#3298)"
        );
    }

    #[test]
    fn writable_key_rejects_names_outside_the_allowlist() {
        assert_eq!(writable_key("GITHUB_TOKEN"), Some("GITHUB_TOKEN"));
        assert_eq!(
            writable_key("mcp-oauth:https://evil.example:client_secret"),
            None
        );
        assert_eq!(writable_key("__sentinel__"), None);
        assert_eq!(writable_key("github_token"), None, "matching must be exact");
    }
}
