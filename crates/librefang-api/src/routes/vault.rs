//! Credential-vault write surface (#8164).
//!
//! * `GET /api/vault/keys` — which writable keys the vault holds, and where the daemon actually resolves each one from. Names, a boolean and a [`KeySource`]; never a value.
//! * `PUT /api/vault/keys/{key}` — store a secret under a writable key.
//! * `DELETE /api/vault/keys/{key}` — remove it.
//!
//! There is deliberately no read-back endpoint. A secret that the API can hand back is a secret one leaked log line, one browser cache entry, or one over-broad token away from being public; the vault's only consumers are in-process (`resolve_github_token` and friends), so nothing needs the value over HTTP.
//!
//! # Why an allowlist rather than arbitrary key names
//!
//! The vault is a single flat namespace shared with the MCP OAuth flow, which stores `mcp-oauth:{server_url}:client_secret` entries there. An endpoint that accepted any key would let an authenticated caller overwrite another server's OAuth client secret, and a listing that returned every key would disclose the set of MCP servers an operator has authenticated against. [`WRITABLE_KEYS`] therefore names exactly the keys a surface is allowed to manage; extending it is a one-line change plus the reasoning for why that key belongs on an operator-facing form.
//!
//! # Why the listing reports a source and not just presence
//!
//! The daemon reads its own process environment before it touches the vault ([`resolve_key`]), so a listing built from the vault alone describes storage rather than behaviour. On a deployment that exports `GITHUB_TOKEN`, a vault-only flag says "not set" while skill proposal and agent-type promotion work, and says "not set" again after a delete that revoked nothing. [`KeySource`] is the field that lets a surface say "overridden by the environment" instead of either lie; `set` keeps its narrow meaning so the operator can still tell a landed-but-inert write from an empty vault.
//!
//! # Hot reload
//!
//! Writes go through [`librefang_kernel::KernelApi::vault_set`], which mutates the same lazily-unlocked `CredentialVault` that [`librefang_kernel::KernelApi::vault_get`] reads from — the `Arc<RwLock<…>>` cached on the kernel by `vault_handle()` (#3598). `CredentialVault::set` inserts into that in-memory map *and* re-encrypts to disk, so the next request that calls `vault_get` observes the new value with no restart and no cache to invalidate.
//!
//! Routing through the kernel accessor is not by itself enough, because the cached map is not the only writer: `KernelOAuthProvider::vault_set` opens its own `CredentialVault` for every `mcp-oauth:*` entry, and `librefang vault set` runs in a separate process. `CredentialVault::save` re-encrypts the *whole* file from one instance's map, so a `PUT` here would erase every OAuth client secret stored since the kernel unlocked — dropping those servers back to `NeedsAuth` — and a `DELETE` would report a revocation it never performed. `LibreFangKernel::vault_set` / `vault_remove` therefore re-read `vault.enc` under the write guard before mutating; this endpoint is the operator-facing trigger that made that reconciliation load-bearing.
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

/// Where the daemon actually resolves a key's value from, in the precedence order [`resolve_key`] applies.
///
/// This exists because "is there a value in the vault" is not the question an operator is asking. The daemon reads its own process environment first, so on a host that exports `GITHUB_TOKEN` a vault-only flag reports "not set" while skill proposal and agent-type promotion work fine — and reports "not set" again after a delete that revoked nothing, because the environment still supplies the token. Both readings are wrong in a way that costs the operator a debugging session.
///
/// The variants stay the *effective* answer, never the stored one: [`KeySource::Environment`] means the environment is what a request would use, whatever the vault also holds. The separate `set` flag on each listing entry keeps the narrow vault-presence meaning, so a surface can tell "your write landed but is inert" from "there is nothing stored at all".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    /// Neither the environment nor the vault holds a non-empty value.
    Unset,
    /// The vault supplies the value the daemon uses.
    Vault,
    /// The daemon's own environment supplies it, overriding any vault entry.
    Environment,
}

/// Resolve a vault key the way the daemon does: process environment first, vault second, values that are empty or whitespace-only treated as absent.
///
/// The environment variable name is the vault key name. Every entry in [`WRITABLE_KEYS`] is a credential the daemon already accepts from its own environment under that exact name, which is what makes one lookup rule correct for the whole allowlist; a future key that does not follow the convention needs its own mapping here rather than a second precedence order somewhere else.
///
/// `routes::skills::resolve_github_token` and [`key_source`] both go through this so the order can only be defined once. A listing that computed presence independently is exactly how the two drifted apart in the first place.
pub(crate) fn resolve_key(state: &AppState, key: &str) -> Option<(String, KeySource)> {
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            return Some((value, KeySource::Environment));
        }
    }
    state
        .kernel
        .vault_get(key)
        .filter(|v| !v.trim().is_empty())
        .map(|value| (value, KeySource::Vault))
}

/// [`resolve_key`] with the value dropped. Nothing on the HTTP surface holds a secret longer than it takes to decide where it came from.
pub(crate) fn key_source(state: &AppState, key: &str) -> KeySource {
    resolve_key(state, key).map_or(KeySource::Unset, |(_, source)| source)
}

/// Whether the vault itself holds a non-empty entry, ignoring the environment.
fn stored_in_vault(state: &AppState, key: &str) -> bool {
    state
        .kernel
        .vault_get(key)
        .is_some_and(|v| !v.trim().is_empty())
}

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

/// Reject the request unless the caller is an authenticated `Owner`.
///
/// `Admin` is deliberately not enough. `middleware::is_owner_only_write` already keeps `/api/config/set` and the `/api/users/{name}/provider-keys` writes at Owner, and `min_role_for_privileged_get` keeps even the names-only provider-key listing there, on the reasoning that Admin is "config write" by design rather than "custody of the credentials the daemon presents as itself". A vault key is exactly the latter: an Admin who could replace `GITHUB_TOKEN` would make skill proposal and agent-type promotion push under a token they chose, and one who could delete it would break both for everyone.
///
/// The three vault paths are registered in those two middleware tables so the policy stays in one place; this check is the in-handler half, and is what covers the code paths that reach the handler without the per-user API-key middleware verdict. The trusted loopback / `LIBREFANG_ALLOW_NO_AUTH=1` path still passes because the middleware injects a synthetic Owner there, which is what keeps the dashboard usable on a single-user install.
fn require_owner(state: &AppState, api_user: Option<&AuthenticatedApiUser>) -> Option<Response> {
    match api_user {
        Some(u) if u.role >= UserRole::Owner => None,
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
                ApiErrorResponse::forbidden("Owner role required for vault access").into_response(),
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
                ApiErrorResponse::unauthorized("Owner credential required for vault access")
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

/// Reject a key outside [`WRITABLE_KEYS`], and record the attempt.
///
/// This is the branch the allowlist exists for — `PUT /api/vault/keys/mcp-oauth:…:client_secret`, or a `DELETE` aimed at `totp_secret` — so it is the one an operator most needs to find in the hash-chained log afterwards. Left unrecorded it was the only rejection on this surface that produced no audit entry, while the plain role denial in [`require_owner`] recorded a `PermissionDenied` for a far less interesting request.
fn not_a_writable_key(
    state: &AppState,
    api_user: Option<&AuthenticatedApiUser>,
    key: &str,
) -> Response {
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::PermissionDenied,
        format!("vault write rejected for key outside the allowlist: {key}"),
        "denied",
        api_user.map(|u| u.user_id),
        Some("api".to_string()),
    );
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
        (status = 200, description = "Writable vault keys, whether the vault holds each one, and the effective source the daemon resolves it from (`unset` / `vault` / `environment`)", body = crate::types::JsonObject),
        (status = 401, description = "Owner credential required"),
        (status = 403, description = "Owner role required"),
    )
)]
pub async fn vault_list_keys(
    State(state): State<Arc<AppState>>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    if let Some(deny) = require_owner(&state, api_user.as_ref().map(|e| &e.0)) {
        return deny;
    }
    let keys: Vec<serde_json::Value> = WRITABLE_KEYS
        .iter()
        .map(|key| {
            serde_json::json!({
                "key": key,
                // `set` is vault presence alone; `source` is what a request would actually use.
                "set": stored_in_vault(&state, key),
                "source": key_source(&state, key),
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
        (status = 200, description = "Secret stored; `source` reports whether the daemon will actually use it or the process environment still overrides it", body = crate::types::JsonObject),
        (status = 400, description = "Empty or oversized value"),
        (status = 401, description = "Owner credential required"),
        (status = 403, description = "Owner role required"),
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
    if let Some(deny) = require_owner(&state, api_user.as_ref().map(|e| &e.0)) {
        return deny;
    }
    let Some(key) = writable_key(&key) else {
        return not_a_writable_key(&state, api_user.as_ref().map(|e| &e.0), &key);
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
    // The write landed, but it is inert while the daemon's environment carries the same key — report the effective source rather than a bare success the surface would render as "configured".
    Json(serde_json::json!({ "key": key, "set": true, "source": key_source(&state, key) }))
        .into_response()
}

#[utoipa::path(
    delete,
    path = "/api/vault/keys/{key}",
    tag = "vault",
    params(("key" = String, Path, description = "Vault key name")),
    responses(
        (status = 200, description = "Vault copy removed (or already absent); `source` still reports `environment` when the process environment continues to supply the key", body = crate::types::JsonObject),
        (status = 401, description = "Owner credential required"),
        (status = 403, description = "Owner role required"),
        (status = 404, description = "Key is not writable over HTTP"),
        (status = 503, description = "Vault could not be unlocked or written"),
    )
)]
pub async fn vault_delete_key(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    if let Some(deny) = require_owner(&state, api_user.as_ref().map(|e| &e.0)) {
        return deny;
    }
    let Some(key) = writable_key(&key) else {
        return not_a_writable_key(&state, api_user.as_ref().map(|e| &e.0), &key);
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
    // A delete revokes the vault copy and nothing else. When the environment still supplies the key, `source` is what stops the response from reading as "revoked".
    Json(serde_json::json!({
        "key": key,
        "set": false,
        "removed": removed,
        "source": key_source(&state, key),
    }))
    .into_response()
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

    /// The wire strings a surface branches on. Renaming a variant silently breaks the dashboard and TUI wording, which have no compiler to catch it.
    #[test]
    fn key_source_serializes_to_the_documented_wire_strings() {
        let json = |s: KeySource| serde_json::to_value(s).expect("KeySource must serialize");
        assert_eq!(json(KeySource::Unset), "unset");
        assert_eq!(json(KeySource::Vault), "vault");
        assert_eq!(json(KeySource::Environment), "environment");
    }
}
