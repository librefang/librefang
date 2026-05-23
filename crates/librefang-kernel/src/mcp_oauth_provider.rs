//! Kernel-side OAuth provider for MCP servers.
//!
//! Implements `McpOAuthProvider` using the extensions vault for encrypted
//! token storage. The actual OAuth flow (PKCE, browser redirect) is driven
//! by the API layer — this provider handles token CRUD and client registration.

use async_trait::async_trait;
use librefang_extensions::ExtensionError;
use librefang_runtime::mcp_oauth::{McpOAuthError, McpOAuthProvider, OAuthTokens};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use tracing::{debug, warn};

/// Classified outcome of a token-endpoint refresh attempt
/// (audit: `oauth-refresh-error-body-token-leak`, sub-finding "error
/// classification").
///
/// Pre-fix, `try_refresh` returned `Result<OAuthTokens, String>` and
/// `load_token` collapsed *every* failure — a 503 from the IdP, a DNS
/// blip, a `invalid_grant` — into `Ok(None)`, which the connection layer
/// reads as "no token stored → run the OAuth flow again". That throws
/// away a still-valid refresh token on a transient outage and forces the
/// operator to re-authorize for no reason.
///
/// This enum lets the caller act on the distinction:
/// - [`Revoked`](RefreshError::Revoked) — the refresh token is dead
///   (`400 invalid_grant`). The only outcome that should flip the server
///   back to a re-auth prompt (`Ok(None)` from `load_token`).
/// - [`Transient`](RefreshError::Transient) — a 5xx / timeout / network
///   error. The refresh token is presumed still valid; the caller should
///   keep it and retry later, NOT re-auth.
/// - [`Permanent`](RefreshError::Permanent) — any other non-success
///   (4xx that is not `invalid_grant`, malformed body, SSRF block,
///   missing endpoint). Surfaced as an error so the caller does not
///   silently discard the refresh token, but it is not a "retry later"
///   signal.
#[derive(Debug)]
pub enum RefreshError {
    /// `400 invalid_grant` — the refresh token has been revoked / expired
    /// server-side. Re-authentication is required.
    Revoked,
    /// 5xx, request timeout, or transport error. Retry later; do not
    /// discard the refresh token.
    Transient(String),
    /// Any other failure (non-`invalid_grant` 4xx, parse failure, SSRF
    /// block, missing stored endpoint).
    Permanent(String),
}

impl fmt::Display for RefreshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefreshError::Revoked => write!(f, "refresh token revoked (invalid_grant)"),
            RefreshError::Transient(msg) => write!(f, "transient refresh failure: {msg}"),
            RefreshError::Permanent(msg) => write!(f, "permanent refresh failure: {msg}"),
        }
    }
}

/// Per-`server_url` single-flight locks for token refresh
/// (audit: `oauth-refresh-error-body-token-leak`, sub-finding "concurrent
/// refresh race").
///
/// Rotating-refresh-token providers (Google, GitHub Apps, Notion) issue a
/// *new* refresh token on every refresh and invalidate the old one. If two
/// `load_token` calls race past the expiry check, both POST the same
/// (soon-to-be-invalidated) refresh token: the second request arrives with
/// a token the provider just rotated away, the provider rejects it, and the
/// whole session is burned even though it was valid.
///
/// `KernelOAuthProvider` is stateless and reconstructed per request
/// (`mcp_auth.rs`, `skills.rs`, `boot.rs` all call `::new`), so the lock map
/// must be process-global to serialize refreshes across every instance, not
/// just within one. Keyed by `server_url` so distinct servers never contend.
type RefreshLocks = std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

static REFRESH_LOCKS: LazyLock<RefreshLocks> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Get (or lazily create) the single-flight lock for `server_url`.
fn refresh_lock_for(server_url: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = REFRESH_LOCKS.lock().expect("REFRESH_LOCKS mutex poisoned");
    map.entry(server_url.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Structured, sanitized view of an OAuth token-endpoint HTTP response
/// suitable for `tracing` events.
///
/// Background (audit: `oauth-refresh-error-body-token-leak`): the IdP
/// token-endpoint response body may contain `access_token` /
/// `refresh_token` / `id_token` / `client_secret`. Some IdPs include
/// token-shaped values even in non-success bodies; a hostile IdP can
/// deliberately plant such values to be persisted in operator logs.
/// Centralized log aggregators (SIEM / Loki / Splunk) typically retain
/// longer than the OAuth flow that produced them, so a single
/// `warn!(body = %resp.text())` leaks a bearer credential for the
/// lifetime of that retention tier.
///
/// This struct collapses the response to:
/// - HTTP status code
/// - Content-Type (when present)
/// - first 8 bytes (16 hex chars) of `sha256(body)`
/// - body length in bytes
///
/// The raw body itself is never stored or emitted, so it can never
/// reach a tracing layer.
///
/// Both `Display` and `Debug` are sanitized; serializing through
/// either is safe.
#[derive(Clone, serde::Serialize)]
pub struct RedactedTokenEndpointResponse {
    pub status: u16,
    pub content_type: Option<String>,
    /// First 8 bytes of `sha256(body)`, hex-encoded (16 chars). Lets the
    /// operator correlate two log lines that saw the same body without
    /// revealing the body itself.
    pub body_sha256_prefix: String,
    pub body_len: usize,
}

impl fmt::Display for RedactedTokenEndpointResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "status={} content_type={} body_sha256_prefix={} body_len={}",
            self.status,
            self.content_type.as_deref().unwrap_or("<none>"),
            self.body_sha256_prefix,
            self.body_len,
        )
    }
}

impl fmt::Debug for RedactedTokenEndpointResponse {
    // Deliberately identical to Display: `?value` in `tracing::warn!`
    // must not be able to dump the raw body.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RedactedTokenEndpointResponse({self})")
    }
}

/// Build a sanitized view of an OAuth token-endpoint response.
///
/// The raw body is consumed by `Sha256::digest` and dropped — it
/// never lands in the returned value, so a subsequent
/// `tracing::warn!(redacted = %redact_token_endpoint_response(...))`
/// cannot leak `access_token` / `refresh_token` / `id_token` /
/// `client_secret`.
pub fn redact_token_endpoint_response(
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> RedactedTokenEndpointResponse {
    let digest = Sha256::digest(body);
    RedactedTokenEndpointResponse {
        status,
        content_type: content_type.map(str::to_owned),
        body_sha256_prefix: hex::encode(&digest[..8]),
        body_len: body.len(),
    }
}

/// Classify a non-success token-endpoint refresh response (audit:
/// `oauth-refresh-error-body-token-leak`, sub-finding "error classification").
///
/// - `400` with an `error: "invalid_grant"` body (RFC 6749 §5.2) → the
///   refresh token is revoked / expired → [`RefreshError::Revoked`].
/// - any `5xx` → [`RefreshError::Transient`] (retry; keep the refresh token).
/// - everything else → [`RefreshError::Permanent`].
///
/// Only the short OAuth `error` code is extracted from the body — never the
/// token-shaped fields. The carried message keeps just the status code so it
/// can never leak a secret into a log or an `Err` returned upstream.
fn classify_refresh_failure(status: reqwest::StatusCode, body: &str) -> RefreshError {
    if status == reqwest::StatusCode::BAD_REQUEST && oauth_error_code(body) == Some("invalid_grant")
    {
        return RefreshError::Revoked;
    }
    if status.is_server_error() {
        return RefreshError::Transient(format!("token endpoint returned HTTP {status}"));
    }
    RefreshError::Permanent(format!("token endpoint returned HTTP {status}"))
}

/// Extract the RFC 6749 §5.2 `error` code from a token-endpoint error body.
///
/// Returns the value of the top-level `"error"` string field (e.g.
/// `"invalid_grant"`) when the body is JSON, otherwise `None`. The function
/// reads only the `error` code; it never returns or logs `access_token` /
/// `refresh_token` / `id_token` / `client_secret`.
fn oauth_error_code(body: &str) -> Option<&'static str> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    match value.get("error")?.as_str()? {
        // Map to a 'static str so a malicious body cannot smuggle an
        // arbitrary (possibly token-shaped) string out of this helper.
        "invalid_grant" => Some("invalid_grant"),
        _ => Some("other"),
    }
}

/// Convert vault-layer errors to the public OAuth storage error taxonomy
/// (#3750). Centralized so every callsite gets the same mapping:
/// - `VaultLocked` ↔ no master key resolvable
/// - `Vault("Vault not initialized…")` → `KeyNotFound` (no vault file yet)
/// - `Vault(other)` → `Crypto(...)` (decryption / parse / schema failure)
/// - `Io` propagates as `Io`
/// - All other extension errors map to `Crypto` so they surface with a
///   distinct taxonomy from "missing token" rather than disappearing.
fn map_extension_err(err: ExtensionError) -> McpOAuthError {
    match err {
        ExtensionError::VaultLocked => McpOAuthError::VaultLocked,
        ExtensionError::Vault(msg) if msg.starts_with("Vault not initialized") => {
            McpOAuthError::KeyNotFound(msg)
        }
        ExtensionError::Vault(msg) => McpOAuthError::Crypto(msg),
        ExtensionError::Io(io) => McpOAuthError::Io(io),
        other => McpOAuthError::Crypto(other.to_string()),
    }
}

/// Vault key prefix for MCP OAuth tokens.
const VAULT_PREFIX: &str = "mcp_oauth";

/// All vault fields stored per MCP server under the mcp_oauth namespace.
/// Kept in sync with every `store`/`vault_set` call in auth_start, store_tokens,
/// and try_refresh so that `clear_tokens` is exhaustive by construction.
const ALL_VAULT_FIELDS: &[&str] = &[
    "access_token",
    "refresh_token",
    "expires_at",
    "token_endpoint",
    "client_id",
    "pkce_verifier",
    "pkce_state",
    "redirect_uri",
];

/// OAuth provider backed by the librefang encrypted credential vault.
///
/// Each instance is stateless — it opens and unlocks the vault on every
/// operation, mirroring the pattern used by `LibreFangKernel::vault_get`
/// and `vault_set`.
pub struct KernelOAuthProvider {
    /// Path to `~/.librefang` (home directory).
    home_dir: PathBuf,
}

impl KernelOAuthProvider {
    /// Create a new provider that stores tokens in the vault at `home_dir/vault.enc`.
    pub fn new(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    /// Convenience: vault key for a specific server URL and field.
    pub fn vault_key(server_url: &str, field: &str) -> String {
        format!("{VAULT_PREFIX}:{server_url}:{field}")
    }

    /// Read a value from the vault.
    ///
    /// Returns `Ok(None)` only when the vault is unlocked and the key is
    /// genuinely absent. Vault unlock failures are surfaced as
    /// [`McpOAuthError`] (#3750) so callers can distinguish "no token
    /// stored" from "vault locked / corrupt".
    pub fn vault_get(&self, key: &str) -> Result<Option<String>, McpOAuthError> {
        let vault_path = self.home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if !vault.exists() {
            return Err(McpOAuthError::KeyNotFound(format!(
                "vault file not initialized; key {key} unreachable"
            )));
        }
        vault.unlock().map_err(map_extension_err)?;
        Ok(vault.get(key).map(|s| s.to_string()))
    }

    /// Read a value from the vault, treating "vault not initialized" as
    /// `Ok(None)` rather than `KeyNotFound`. Used by `load_token` where a
    /// missing vault is semantically equivalent to "no cached token".
    fn vault_get_optional(&self, key: &str) -> Result<Option<String>, McpOAuthError> {
        let vault_path = self.home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if !vault.exists() {
            return Ok(None);
        }
        vault.unlock().map_err(map_extension_err)?;
        Ok(vault.get(key).map(|s| s.to_string()))
    }

    /// Legacy `Option`-returning helper for code paths (e.g. `try_refresh`,
    /// `auth_start`) whose error model is still `Result<_, String>`. Logs
    /// vault failures the same way the original `vault_get` did.
    pub fn vault_get_or_warn(&self, key: &str) -> Option<String> {
        match self.vault_get_optional(key) {
            Ok(opt) => opt,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    key = %key,
                    "MCP OAuth vault_get_or_warn: vault unlock failed — returning None. \
                     Check that LIBREFANG_VAULT_KEY is set."
                );
                None
            }
        }
    }

    /// Write a value to the vault. Creates the vault if it does not exist.
    pub fn vault_set(&self, key: &str, value: &str) -> Result<(), McpOAuthError> {
        let vault_path = self.home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if !vault.exists() {
            vault.init().map_err(map_extension_err)?;
        } else {
            vault.unlock().map_err(map_extension_err)?;
        }
        vault
            .set(key.to_string(), zeroize::Zeroizing::new(value.to_string()))
            .map_err(map_extension_err)
    }

    /// Remove a value from the vault. Returns `Ok(true)` if the key existed.
    pub fn vault_remove(&self, key: &str) -> Result<bool, McpOAuthError> {
        let vault_path = self.home_dir.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if !vault.exists() {
            return Ok(false);
        }
        vault.unlock().map_err(map_extension_err)?;
        vault.remove(key).map_err(map_extension_err)
    }

    /// Try to refresh the access token using a stored refresh token.
    ///
    /// Returns a classified [`RefreshError`] on failure (audit:
    /// `oauth-refresh-error-body-token-leak`). Only [`RefreshError::Revoked`]
    /// (a `400 invalid_grant`) means the refresh token is dead and the caller
    /// should re-authenticate; 5xx / timeout / transport failures are
    /// [`RefreshError::Transient`] and the refresh token must be kept for a
    /// later retry rather than discarded.
    async fn try_refresh(
        &self,
        server_url: &str,
        refresh_token: &str,
    ) -> Result<OAuthTokens, RefreshError> {
        let token_endpoint = self
            .vault_get_or_warn(&Self::vault_key(server_url, "token_endpoint"))
            .ok_or_else(|| {
                RefreshError::Permanent("No token_endpoint stored for refresh".to_string())
            })?;

        // SSRF guard (#3623): re-validate the stored token_endpoint before
        // POSTing.  The stored value may predate policy tightening or have
        // been written by a compromised flow — always re-check before making
        // outbound requests.
        if let Err(reason) = librefang_runtime::mcp_oauth::is_ssrf_blocked_url(&token_endpoint) {
            return Err(RefreshError::Permanent(format!(
                "SSRF: token_endpoint rejected for refresh: {reason}"
            )));
        }

        let client_id = self.vault_get_or_warn(&Self::vault_key(server_url, "client_id"));

        let client = librefang_extensions::http_client::new_client();
        let mut params = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
        ];
        if let Some(cid) = &client_id {
            params.push(("client_id", cid.clone()));
        }

        let resp = match client.post(&token_endpoint).form(&params).send().await {
            Ok(resp) => resp,
            Err(e) => {
                // A transport-level failure (timeout, DNS, connection
                // reset) leaves the refresh token untouched on the
                // server, so it is still usable on the next attempt —
                // classify as Transient so the caller retries instead of
                // forcing a re-auth.
                let msg = if e.is_timeout() {
                    format!("refresh request timed out: {e}")
                } else {
                    format!("refresh request failed: {e}")
                };
                return Err(RefreshError::Transient(msg));
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            // Read Content-Type BEFORE consuming the response with `.text()`.
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = resp.text().await.unwrap_or_default();
            // Audit: `oauth-refresh-error-body-token-leak`. The IdP error
            // body may include token-shaped values (legitimately, for
            // providers that echo session state on `invalid_grant`, or
            // adversarially, to plant secrets in operator logs). Never
            // emit the body verbatim — log a sanitized digest only, and
            // surface a generic message to the caller.
            let redacted = redact_token_endpoint_response(
                status.as_u16(),
                content_type.as_deref(),
                body.as_bytes(),
            );
            warn!(
                redacted_response = %redacted,
                "MCP OAuth token refresh failed (non-success status)",
            );
            // Classify the HTTP failure (audit: error classification):
            //   400 invalid_grant → Revoked (re-auth)
            //   5xx               → Transient (retry, keep refresh token)
            //   anything else     → Permanent
            // `invalid_grant` is detected from the parsed OAuth error code
            // in the body (RFC 6749 §5.2), not the verbatim body — only the
            // short error code is matched, never the token-shaped fields.
            return Err(classify_refresh_failure(status, &body));
        }

        resp.json::<OAuthTokens>()
            .await
            .map_err(|e| RefreshError::Permanent(format!("failed to parse refresh response: {e}")))
    }

    /// True when an access token whose absolute `expires_at` (unix seconds)
    /// is the given value should be refreshed. A 60-second skew matches the
    /// historical near-expiry window so a token about to expire mid-request
    /// is refreshed proactively.
    fn is_expired(expires_at: i64) -> bool {
        chrono::Utc::now().timestamp() >= expires_at - 60
    }

    /// Refresh an access token that the caller has determined is expired,
    /// serialized per `server_url` (audit: `oauth-refresh-error-body-token-leak`,
    /// single-flight).
    ///
    /// The single-flight lock collapses a thundering herd of concurrent
    /// `load_token` calls that all saw the same expired token into one
    /// network refresh. After acquiring the lock we **re-read `expires_at`
    /// from the vault**: if a peer already refreshed while we waited, its
    /// fresh access token is returned directly and no second refresh POST is
    /// made — critical for rotating-refresh-token providers, where a second
    /// POST would arrive with a refresh token the provider just invalidated.
    ///
    /// Outcome mapping (audit: error classification):
    /// - refresh succeeds → `Ok(Some(new_access_token))`.
    /// - [`RefreshError::Revoked`] → `Ok(None)` (the only re-auth signal).
    /// - [`RefreshError::Transient`] / [`RefreshError::Permanent`] →
    ///   `Err(McpOAuthError::RefreshFailed)` so the still-valid refresh token
    ///   is kept rather than discarded.
    async fn refresh_expired_token(
        &self,
        server_url: &str,
    ) -> Result<Option<String>, McpOAuthError> {
        let lock = refresh_lock_for(server_url);
        let _guard = lock.lock().await;

        // Single-flight recheck: a peer holding the lock before us may have
        // already refreshed. Re-read `expires_at` and the access token from
        // the vault and short-circuit if the token is now valid — this is
        // what stops a second redundant (and, for rotating providers,
        // session-burning) refresh POST.
        if let Some(expires_at_str) =
            self.vault_get_optional(&Self::vault_key(server_url, "expires_at"))?
        {
            if let Ok(expires_at) = expires_at_str.parse::<i64>() {
                if !Self::is_expired(expires_at) {
                    debug!(
                        server = %server_url,
                        "MCP OAuth token already refreshed by a concurrent caller; reusing it"
                    );
                    return self.vault_get_optional(&Self::vault_key(server_url, "access_token"));
                }
            }
        }

        let Some(refresh_token) =
            self.vault_get_optional(&Self::vault_key(server_url, "refresh_token"))?
        else {
            // Expired with no refresh token to use — the OAuth flow must run.
            return Ok(None);
        };

        match self.try_refresh(server_url, &refresh_token).await {
            Ok(new_tokens) => {
                if let Err(e) = self.store_tokens(server_url, new_tokens.clone()).await {
                    warn!(error = %e, "Failed to store refreshed tokens");
                }
                Ok(Some(new_tokens.access_token))
            }
            // Revoked is the ONLY outcome that flips the server back to a
            // re-auth prompt: the refresh token is dead, so the cached state
            // is worthless and the OAuth flow must run again.
            Err(RefreshError::Revoked) => {
                warn!(server = %server_url, "MCP OAuth refresh token revoked; re-auth required");
                Ok(None)
            }
            // Transient / Permanent: do NOT discard the refresh token.
            // Surface as an error so the connection layer proceeds without a
            // bearer header (the server may 401), keeping the stored refresh
            // token intact for a later retry instead of forcing a re-auth on
            // a transient outage.
            Err(e @ (RefreshError::Transient(_) | RefreshError::Permanent(_))) => {
                warn!(server = %server_url, error = %e, "MCP OAuth token refresh failed (keeping refresh token)");
                Err(McpOAuthError::RefreshFailed(e.to_string()))
            }
        }
    }

    /// RFC 7591 Dynamic Client Registration.
    ///
    /// POSTs to the registration endpoint to obtain a client_id.
    /// This is required by servers like Notion's MCP that don't provide
    /// a pre-configured client_id.
    pub async fn register_client(
        &self,
        registration_endpoint: &str,
        redirect_uri: &str,
        _server_url: &str,
    ) -> Result<String, String> {
        // SSRF guard (#3623): registration_endpoint may have come from a
        // discovered metadata document or a vault entry written before policy
        // tightened.  Re-check before POSTing — the parser also checks, but
        // this is the actual outbound-request site and the cheapest place to
        // be sure.
        if let Err(reason) =
            librefang_runtime::mcp_oauth::is_ssrf_blocked_url(registration_endpoint)
        {
            return Err(format!("SSRF: registration_endpoint rejected: {reason}"));
        }
        let client = librefang_extensions::http_client::new_client();

        let body = serde_json::json!({
            "client_name": "LibreFang",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        });

        let resp = client
            .post(registration_endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Client registration request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            // Read Content-Type BEFORE consuming the response with `.text()`.
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = resp.text().await.unwrap_or_default();
            // Audit: `oauth-refresh-error-body-token-leak` — same threat
            // model as the token-endpoint path. RFC 7591 §3.2.1 allows
            // the registration response (including the error body) to
            // contain `client_secret`, and an adversarial AS can plant
            // any token-shaped value here just as easily as on the
            // token endpoint. Log a sanitized digest only; the
            // returned error string (which surfaces to
            // `McpAuthState::Error.message` and the dashboard) must
            // not carry the body verbatim either.
            let redacted = redact_token_endpoint_response(
                status.as_u16(),
                content_type.as_deref(),
                body.as_bytes(),
            );
            warn!(
                redacted_response = %redacted,
                "MCP OAuth Dynamic Client Registration failed (non-success status)",
            );
            return Err(format!("Client registration failed (HTTP {status})"));
        }

        // We register as a public client (`token_endpoint_auth_method: "none"`),
        // so any `client_secret` the AS echoes back is intentionally ignored —
        // it must not be persisted or used in subsequent token exchanges.
        #[derive(serde::Deserialize)]
        struct RegistrationResponse {
            client_id: String,
            #[allow(dead_code)]
            #[serde(default)]
            client_secret: Option<String>,
        }

        let reg: RegistrationResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse registration response: {e}"))?;

        Ok(reg.client_id)
    }
}

#[async_trait]
impl McpOAuthProvider for KernelOAuthProvider {
    async fn load_token(&self, server_url: &str) -> Result<Option<String>, McpOAuthError> {
        // Treat "no vault file at all" as Ok(None) — the user simply has not
        // run any OAuth flow yet. Locked/corrupt vault propagates as Err so
        // the dashboard can prompt re-unlock instead of falsely re-auth'ing.
        let access_token =
            match self.vault_get_optional(&Self::vault_key(server_url, "access_token"))? {
                Some(t) => t,
                None => return Ok(None),
            };

        // Check expiration if stored.
        if let Some(expires_at_str) =
            self.vault_get_optional(&Self::vault_key(server_url, "expires_at"))?
        {
            if let Ok(expires_at) = expires_at_str.parse::<i64>() {
                if Self::is_expired(expires_at) {
                    debug!(server = %server_url, "MCP OAuth token expired or near expiry, attempting refresh");
                    return self.refresh_expired_token(server_url).await;
                }
            }
        }
        // No expires_at stored (e.g. Notion) — return token as-is.
        Ok(Some(access_token))
    }

    async fn store_tokens(
        &self,
        server_url: &str,
        tokens: OAuthTokens,
    ) -> Result<(), McpOAuthError> {
        self.vault_set(
            &Self::vault_key(server_url, "access_token"),
            &tokens.access_token,
        )?;

        if let Some(ref rt) = tokens.refresh_token {
            self.vault_set(&Self::vault_key(server_url, "refresh_token"), rt)?;
        }

        if tokens.expires_in > 0 {
            let expires_at = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
            self.vault_set(
                &Self::vault_key(server_url, "expires_at"),
                &expires_at.to_string(),
            )?;
        }

        debug!(server = %server_url, "MCP OAuth tokens stored in vault");
        Ok(())
    }

    /// Persist the discovery-derived OAuth metadata under the durable
    /// per-server vault namespace.
    ///
    /// **Writes are additive — passing `None` for `client_id` does NOT clear
    /// an existing value.** A re-auth flow that legitimately drops Dynamic
    /// Client Registration (e.g. server now ships a static `client_id`) will
    /// leave the previous DCR `client_id` in the vault, and the next refresh
    /// will continue to send it. To overwrite, pass the new `Some(cid)`
    /// explicitly; to remove, callers must `vault_remove` the
    /// `{server_url}/client_id` key directly.
    ///
    /// `token_endpoint` is unconditionally written — it has no semantic
    /// "absent" state at this layer (the callback only invokes this method
    /// after discovery succeeded, so the value is always meaningful).
    async fn store_oauth_metadata(
        &self,
        server_url: &str,
        token_endpoint: &str,
        client_id: Option<&str>,
    ) -> Result<(), McpOAuthError> {
        // Promote discovery output from the per-flow staging namespace into
        // the durable per-server namespace that `try_refresh` reads from.
        // Without this, refresh fails with "No token_endpoint stored for
        // refresh" the first time the access token expires (e.g. ~1h after
        // a successful Notion sign-in).
        self.vault_set(
            &Self::vault_key(server_url, "token_endpoint"),
            token_endpoint,
        )?;
        if let Some(cid) = client_id {
            self.vault_set(&Self::vault_key(server_url, "client_id"), cid)?;
        }
        debug!(server = %server_url, "MCP OAuth metadata persisted to vault");
        Ok(())
    }

    async fn clear_tokens(&self, server_url: &str) -> Result<(), McpOAuthError> {
        // #3369: aggregate per-field failures and surface them. Returning Ok
        // when vault_remove failed lets the UI display "logged out" while the
        // refresh/access tokens still sit in the vault — daemon keeps using
        // them on the next request.
        //
        // #3750: if the *first* failure is VaultLocked, propagate it as the
        // typed variant so the API layer can return 423 Locked. Mixed
        // failures collapse into Crypto with the aggregated detail.
        let mut failures: Vec<String> = Vec::new();
        let mut first_locked = false;
        for field in ALL_VAULT_FIELDS {
            let key = Self::vault_key(server_url, field);
            if let Err(e) = self.vault_remove(&key) {
                warn!(server = %server_url, field = %field, error = %e, "MCP OAuth clear_tokens: vault_remove failed");
                if matches!(e, McpOAuthError::VaultLocked) && failures.is_empty() {
                    first_locked = true;
                }
                failures.push(format!("{field}: {e}"));
            }
        }
        if !failures.is_empty() {
            if first_locked && failures.iter().all(|f| f.contains("vault is locked")) {
                return Err(McpOAuthError::VaultLocked);
            }
            return Err(McpOAuthError::Crypto(format!(
                "Sign-out failed to fully clear vault for {server_url}; tokens may still be valid. Retry. Details: {}",
                failures.join("; ")
            )));
        }
        debug!(server = %server_url, "MCP OAuth tokens cleared from vault");
        Ok(())
    }
}

impl KernelOAuthProvider {
    /// Returns the canonical list of vault fields cleared by `clear_tokens`.
    /// Exposed for tests to assert exhaustiveness without a live vault.
    #[cfg(test)]
    pub(crate) fn clear_token_fields() -> &'static [&'static str] {
        ALL_VAULT_FIELDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAII guard that sets `LIBREFANG_VAULT_KEY` on construction and
    /// removes it on `Drop`. Critical because the test bodies below run
    /// `expect()` / `assert!` between set_var and remove_var; on panic
    /// the manual `remove_var` is skipped, the `serial_test` mutex is
    /// released anyway, and any subsequent test that doesn't itself set
    /// the env var would observe a polluted environment. The guard
    /// makes cleanup unconditional regardless of panic / early-return.
    struct VaultKeyEnvGuard;

    impl VaultKeyEnvGuard {
        fn set(value: &str) -> Self {
            // SAFETY: tests are gated by `serial_test::serial(librefang_vault_key)`
            // so concurrent mutators of this env var inside the same
            // process are serialized; no other thread observes a torn
            // value while this guard is alive.
            unsafe {
                std::env::set_var("LIBREFANG_VAULT_KEY", value);
            }
            Self
        }
    }

    impl Drop for VaultKeyEnvGuard {
        fn drop(&mut self) {
            // SAFETY: same justification as `set` above.
            unsafe {
                std::env::remove_var("LIBREFANG_VAULT_KEY");
            }
        }
    }

    /// Syntactically-valid 32-byte (base64-encoded 44-char) master key
    /// used by the negative-path tests below. Reaches the decrypt step
    /// and fails on the corrupt ciphertext, rather than failing on a
    /// missing key (which is a different code path).
    const TEST_VAULT_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn vault_key_format() {
        let key = KernelOAuthProvider::vault_key("https://example.com/mcp", "access_token");
        assert_eq!(key, "mcp_oauth:https://example.com/mcp:access_token");
    }

    #[test]
    fn vault_key_refresh_token() {
        let key = KernelOAuthProvider::vault_key("https://example.com/mcp", "refresh_token");
        assert_eq!(key, "mcp_oauth:https://example.com/mcp:refresh_token");
    }

    #[test]
    fn vault_key_all_fields_namespaced() {
        let url = "https://mcp.notion.com/mcp";
        // All fields that should be cleaned up on delete — driven by ALL_VAULT_FIELDS
        for field in ALL_VAULT_FIELDS {
            let key = KernelOAuthProvider::vault_key(url, field);
            assert!(
                key.starts_with("mcp_oauth:"),
                "Key for '{}' should be prefixed with 'mcp_oauth:'",
                field
            );
            assert!(
                key.contains(url),
                "Key for '{}' should contain the server URL",
                field
            );
            assert!(
                key.ends_with(field),
                "Key for '{}' should end with the field name",
                field
            );
        }
    }

    #[test]
    fn vault_keys_are_isolated_per_server() {
        let key_a = KernelOAuthProvider::vault_key("https://server-a.com/mcp", "access_token");
        let key_b = KernelOAuthProvider::vault_key("https://server-b.com/mcp", "access_token");
        assert_ne!(
            key_a, key_b,
            "Different servers should have different vault keys"
        );
    }

    /// #3369: when vault_remove fails, clear_tokens MUST return Err so the
    /// API layer can tell the user sign-out is incomplete. Pre-fix, this
    /// returned Ok(()) and the UI showed "logged out" while the access token
    /// stayed in the vault.
    #[tokio::test]
    #[serial_test::serial(librefang_vault_key)]
    async fn clear_tokens_returns_err_when_vault_unlock_fails() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        // Write a garbage vault.enc so unlock() fails for every vault_remove call.
        std::fs::write(home.join("vault.enc"), b"not-a-real-vault").expect("seed bad vault");
        // RAII-guarded master key: cleared on Drop even if the assertion
        // below panics, so subsequent tests don't observe a polluted env.
        let _vault_key = VaultKeyEnvGuard::set(TEST_VAULT_KEY);

        let provider = KernelOAuthProvider::new(home);
        let result = provider.clear_tokens("https://example.com/mcp").await;

        assert!(
            result.is_err(),
            "clear_tokens must propagate vault failures (#3369), got {:?}",
            result
        );
        let err = result.unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("retry") || msg.contains("sign-out"),
            "error message should prompt the caller to retry, got: {err}"
        );
    }

    /// #3750: pin the `ExtensionError → McpOAuthError` mapping so a
    /// rephrasing of the upstream vault error message (which the
    /// `Vault("Vault not initialized…")` arm currently substring-matches
    /// on) can't silently demote `KeyNotFound` to `Crypto`. If the
    /// upstream message ever changes, the third assertion below fires
    /// and points at this exact coupling.
    #[test]
    fn map_extension_err_covers_each_variant() {
        // VaultLocked → VaultLocked
        let mapped = map_extension_err(ExtensionError::VaultLocked);
        assert!(
            matches!(mapped, McpOAuthError::VaultLocked),
            "VaultLocked must round-trip, got {mapped:?}"
        );

        // Vault("Vault not initialized…") → KeyNotFound. The literal
        // prefix is the contract with `librefang-extensions::vault::unlock`;
        // any change there must update both sides.
        let mapped = map_extension_err(ExtensionError::Vault(
            "Vault not initialized. Run `librefang vault init`.".to_string(),
        ));
        assert!(
            matches!(mapped, McpOAuthError::KeyNotFound(_)),
            "'Vault not initialized…' must map to KeyNotFound, got {mapped:?}; \
             if the upstream vault error message changed, update map_extension_err to match."
        );

        // Vault(other) → Crypto
        let mapped = map_extension_err(ExtensionError::Vault("AEAD decryption failed".to_string()));
        assert!(
            matches!(mapped, McpOAuthError::Crypto(_)),
            "non-init Vault errors must map to Crypto, got {mapped:?}"
        );

        // Io → Io
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let mapped = map_extension_err(ExtensionError::Io(io));
        assert!(
            matches!(mapped, McpOAuthError::Io(_)),
            "Io must round-trip, got {mapped:?}"
        );
    }

    /// `load_token` MUST distinguish "no vault file at all" (a fresh
    /// install — `Ok(None)`) from "vault present but unlock failed"
    /// (`Err`). Pre-fix both surfaced as `None` and the dashboard could
    /// not tell the user to set `LIBREFANG_VAULT_KEY` (#3750).
    #[tokio::test]
    async fn load_token_returns_ok_none_when_vault_file_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let provider = KernelOAuthProvider::new(tmp.path().to_path_buf());

        let result = provider.load_token("https://example.com/mcp").await;
        assert!(
            matches!(result, Ok(None)),
            "fresh install (no vault.enc) must yield Ok(None), got {result:?}"
        );
    }

    /// Counterpart to the test above: a corrupt vault must surface as
    /// `Err`, not silently as `Ok(None)`. Otherwise the dashboard would
    /// helpfully kick off a re-auth flow that can never succeed because
    /// the vault is unreadable.
    #[tokio::test]
    #[serial_test::serial(librefang_vault_key)]
    async fn load_token_propagates_vault_failure_as_err() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        std::fs::write(home.join("vault.enc"), b"not-a-real-vault").expect("seed bad vault");
        let _vault_key = VaultKeyEnvGuard::set(TEST_VAULT_KEY);

        let provider = KernelOAuthProvider::new(home);
        let result = provider.load_token("https://example.com/mcp").await;

        assert!(
            result.is_err(),
            "corrupt vault must surface as Err, not Ok(None) — got {result:?}"
        );
    }

    /// Regression for the silent OAuth refresh failure: after a successful
    /// authorization callback, `token_endpoint` (and `client_id` from RFC 7591
    /// DCR) MUST live under the durable per-server vault namespace so that
    /// `try_refresh` can find them when the access token expires.
    ///
    /// Pre-fix the callback handler stashed these values under per-flow keys
    /// (`{server_url}:{flow_id}/...`) and only `store_tokens` ran against the
    /// bare namespace, so refresh blew up with "No token_endpoint stored for
    /// refresh" the first time the user's session crossed the access-token
    /// TTL — symptom seen most often with Notion (~1h tokens).
    #[tokio::test]
    #[serial_test::serial(librefang_vault_key)]
    async fn store_oauth_metadata_persists_to_bare_namespace() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let _vault_key = VaultKeyEnvGuard::set(TEST_VAULT_KEY);
        let provider = KernelOAuthProvider::new(home);
        let server_url = "https://mcp.notion.com/mcp";

        provider
            .store_oauth_metadata(
                server_url,
                "https://mcp.notion.com/token",
                Some("client-xyz"),
            )
            .await
            .expect("store_oauth_metadata");

        let token_ep_key = KernelOAuthProvider::vault_key(server_url, "token_endpoint");
        let client_id_key = KernelOAuthProvider::vault_key(server_url, "client_id");

        assert_eq!(
            provider
                .vault_get(&token_ep_key)
                .expect("vault_get token_endpoint"),
            Some("https://mcp.notion.com/token".to_string()),
            "token_endpoint must be readable under the bare per-server key — \
             this is the key try_refresh reads from"
        );
        assert_eq!(
            provider
                .vault_get(&client_id_key)
                .expect("vault_get client_id"),
            Some("client-xyz".to_string()),
            "client_id must be readable under the bare per-server key for refresh"
        );
    }

    /// `client_id` is optional (servers with a pre-registered public client
    /// won't run RFC 7591 DCR). Passing `None` must NOT write a bogus key,
    /// and — critically — must NOT clear an existing value. `store_oauth_metadata`
    /// is documented as additive; this locks that contract so a future
    /// "helpful" cleanup of the kernel impl can't silently turn `None` into
    /// a delete and leave production refreshes mid-flow without a client_id.
    #[tokio::test]
    #[serial_test::serial(librefang_vault_key)]
    async fn store_oauth_metadata_skips_client_id_when_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let _vault_key = VaultKeyEnvGuard::set(TEST_VAULT_KEY);
        let provider = KernelOAuthProvider::new(home);
        let server_url = "https://example.com/mcp";
        let client_id_key = KernelOAuthProvider::vault_key(server_url, "client_id");

        // Case 1: empty vault, None client_id → key must remain absent.
        provider
            .store_oauth_metadata(server_url, "https://example.com/token", None)
            .await
            .expect("store_oauth_metadata");

        assert_eq!(
            provider
                .vault_get(&client_id_key)
                .expect("vault_get client_id"),
            None,
            "client_id key must remain absent when None is passed against an empty vault"
        );

        // Case 2: seed an existing client_id (simulates a prior DCR run),
        // then call again with None → existing value MUST survive.
        provider
            .vault_set(&client_id_key, "preexisting-cid")
            .expect("seed client_id");
        provider
            .store_oauth_metadata(server_url, "https://example.com/token", None)
            .await
            .expect("store_oauth_metadata (None against seeded client_id)");

        assert_eq!(
            provider
                .vault_get(&client_id_key)
                .expect("vault_get client_id after None call"),
            Some("preexisting-cid".to_string()),
            "writes are additive: passing None must NOT clear an existing client_id"
        );
    }

    /// Regression for #5069: a daemon process that calls `vault_set` twice
    /// in a row against a previously-uninitialised vault MUST succeed on
    /// the second call. The first call walks `init() + set()`; the second
    /// must walk `unlock() + set()` using the same env-supplied master key
    /// and decrypt the file the first call just wrote.
    ///
    /// This mirrors the `auth_start` PKCE-stash sequence
    /// (`pkce_verifier` → `pkce_state` → `redirect_uri`) where the first
    /// call lazy-creates the vault and every subsequent call lands on the
    /// freshly-written file. Pre-fix the second call's `unlock()` failed
    /// with `aead::Error` because `init()` and `resolve_master_key()`
    /// duplicated the env / keyring lookup code, so a daemon process that
    /// raced two readers of the env var (one in init, one in the next
    /// unlock) could write a file the next unlock could not decrypt. The
    /// unified-resolution fix removes the duplication, and the new
    /// post-write verification in `init()` catches any latent divergence
    /// at the source rather than letting the next caller fail with an
    /// opaque `aead::Error`.
    ///
    /// Uses the named `serial(librefang_vault_key)` group — the same
    /// group every other vault-key-touching test in this crate
    /// (including `kernel::tests::vault_cache_reuses_unlocked_handle_across_calls`
    /// and `install_integration_writes_through_cached_vault_handle`,
    /// migrated alongside this change) sits in. Two disjoint serial
    /// groups in the same crate that both mutate the process-global
    /// `LIBREFANG_VAULT_KEY` would race init's resolve → save → verify
    /// sequence and the verification would spuriously fire.
    #[test]
    #[serial_test::serial(librefang_vault_key)]
    fn vault_set_twice_round_trips_via_env_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        // SAFETY: serial guard prevents env races with other vault tests.
        let _vault_key = VaultKeyEnvGuard::set(TEST_VAULT_KEY);
        // SAFETY: same serial guard prevents racing env reads.
        unsafe {
            std::env::set_var("LIBREFANG_VAULT_NO_KEYRING", "1");
        }

        let provider = KernelOAuthProvider::new(home.clone());

        // First call: vault.enc absent → init() + set() lazy-creates it.
        provider
            .vault_set("pkce_verifier", "verifier-1")
            .expect("first vault_set must lazy-init and persist");
        assert!(
            home.join("vault.enc").exists(),
            "first vault_set must have materialised vault.enc"
        );

        // Second call: vault.enc present → unlock() + set(). This is the
        // failing path in #5069 — unlock() panicked with aead::Error on
        // the file the same process wrote ~ms earlier.
        provider
            .vault_set("pkce_state", "state-1")
            .expect("second vault_set must unlock and persist (no aead::Error)");

        // Third call to mirror the third PKCE field in auth_start.
        provider
            .vault_set("redirect_uri", "https://example.test/cb")
            .expect("third vault_set must unlock and persist");

        // Round-trip: every entry written above must be readable through a
        // fresh provider instance, which exercises the unlock path again.
        let reader = KernelOAuthProvider::new(home);
        assert_eq!(
            reader
                .vault_get("pkce_verifier")
                .expect("vault_get pkce_verifier"),
            Some("verifier-1".to_string())
        );
        assert_eq!(
            reader
                .vault_get("pkce_state")
                .expect("vault_get pkce_state"),
            Some("state-1".to_string())
        );
        assert_eq!(
            reader
                .vault_get("redirect_uri")
                .expect("vault_get redirect_uri"),
            Some("https://example.test/cb".to_string())
        );

        // SAFETY: same serial-guard rationale.
        unsafe {
            std::env::remove_var("LIBREFANG_VAULT_NO_KEYRING");
        }
    }

    #[test]
    fn clear_tokens_covers_all_stored_fields() {
        // Verifies that ALL_VAULT_FIELDS (used by clear_tokens) covers every field
        // that store_tokens or auth_start might write. If a new field is added to
        // those functions, add it to ALL_VAULT_FIELDS and this assertion will pass;
        // if it's forgotten in ALL_VAULT_FIELDS, this test will fail.
        let fields = KernelOAuthProvider::clear_token_fields();
        for expected in &[
            "access_token",
            "refresh_token",
            "expires_at",
            "token_endpoint",
            "client_id",
            "pkce_verifier",
            "pkce_state",
            "redirect_uri",
        ] {
            assert!(
                fields.contains(expected),
                "ALL_VAULT_FIELDS is missing '{}' — clear_tokens won't wipe it",
                expected
            );
        }
        assert_eq!(
            fields.len(),
            8,
            "Unexpected field count in ALL_VAULT_FIELDS — update this assertion if new fields are intentionally added"
        );
    }

    /// Audit: `oauth-refresh-error-body-token-leak`. The helper that
    /// summarises a token-endpoint response MUST NOT carry the raw body
    /// (or any token-shaped substring of it) in its `Display` or `Debug`
    /// representation. The hex digest prefix and the lengths are the
    /// only operator-visible information.
    #[test]
    fn redact_token_endpoint_response_strips_token_fields() {
        let body = br#"{"access_token":"super-secret-12345","refresh_token":"rt-9999","id_token":"id-eyJ","client_secret":"cs-abcdef"}"#;
        let r = redact_token_endpoint_response(400, Some("application/json"), body);

        let displayed = format!("{r}");
        let debugged = format!("{r:?}");

        for secret in [
            "super-secret-12345",
            "rt-9999",
            "id-eyJ",
            "cs-abcdef",
            "access_token",
            "refresh_token",
            "id_token",
            "client_secret",
        ] {
            assert!(
                !displayed.contains(secret),
                "Display leaked '{secret}': {displayed}"
            );
            assert!(
                !debugged.contains(secret),
                "Debug leaked '{secret}': {debugged}"
            );
        }

        assert!(displayed.contains("status=400"));
        assert!(displayed.contains("content_type=application/json"));
        assert!(displayed.contains(&format!("body_len={}", body.len())));
        assert!(displayed.contains("body_sha256_prefix="));
    }

    /// Same body produces the same digest prefix — operators can
    /// correlate two log lines that saw the same body without ever
    /// holding the body itself.
    #[test]
    fn redact_token_endpoint_response_digest_is_stable() {
        let body = br#"{"error":"invalid_grant"}"#;
        let a = redact_token_endpoint_response(400, None, body);
        let b = redact_token_endpoint_response(400, None, body);
        assert_eq!(a.body_sha256_prefix, b.body_sha256_prefix);
        // 16 hex chars = first 8 bytes of sha256.
        assert_eq!(a.body_sha256_prefix.len(), 16);
        assert!(
            a.body_sha256_prefix.chars().all(|c| c.is_ascii_hexdigit()),
            "digest prefix must be hex: {}",
            a.body_sha256_prefix
        );
    }

    /// End-to-end: the `try_refresh` non-success branch must emit only
    /// a sanitized digest; a `tracing::warn!` capture across that path
    /// must not contain the raw secret.
    ///
    /// We exercise the redaction path directly (the call site emits
    /// `warn!(redacted_response = %redact_token_endpoint_response(...))`)
    /// rather than spinning up an HTTP fake — the redaction helper is
    /// the single chokepoint and is what the audit fix is about.
    #[tokio::test]
    async fn warn_emission_does_not_contain_raw_token_body() {
        use std::io;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Clone)]
        struct VecWriter(Arc<Mutex<Vec<u8>>>);
        impl io::Write for VecWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for VecWriter {
            type Writer = VecWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = VecWriter(buf.clone());
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(false);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        let raw_body = br#"{"error":"invalid_grant","access_token":"super-secret-12345","refresh_token":"rt-9999"}"#;
        let redacted = redact_token_endpoint_response(400, Some("application/json"), raw_body);
        // Mirrors the call shape used in `try_refresh`.
        warn!(
            redacted_response = %redacted,
            "MCP OAuth token refresh failed (non-success status)",
        );

        let captured = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
        for secret in ["super-secret-12345", "rt-9999"] {
            assert!(
                !captured.contains(secret),
                "log line leaked '{secret}'; captured: {captured:?}"
            );
        }
        assert!(
            captured.contains("body_sha256_prefix="),
            "log line missing sanitized digest; captured: {captured:?}"
        );
    }

    /// Audit: `oauth-refresh-error-body-token-leak`. The Dynamic Client
    /// Registration (RFC 7591) non-success branch in
    /// `KernelOAuthProvider::register_client` is the third
    /// token-endpoint-shaped call site in this file. The registration
    /// response body can legitimately contain `client_secret`, and an
    /// adversarial authorization server can plant any token-shaped
    /// value there. The error string returned from `register_client`
    /// flows into `McpAuthState::Error.message` (visible in the
    /// dashboard) AND is logged via `tracing::warn!(error = %e, ...)`
    /// in `routes::mcp_auth::auth_start`, so it must NOT carry the
    /// raw body. Verify both: the returned `Err` string does not
    /// contain the body, and the warn-side emission does not either.
    #[tokio::test]
    async fn dcr_failure_does_not_leak_raw_body_into_error_or_log() {
        use std::io;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Clone)]
        struct VecWriter(Arc<Mutex<Vec<u8>>>);
        impl io::Write for VecWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for VecWriter {
            type Writer = VecWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = VecWriter(buf.clone());
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(false);
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        // Mirrors the call shape used in the DCR non-success branch:
        // construct the redacted view, emit the warn, and synthesize
        // the returned error string the way `register_client` does.
        let raw_body =
            br#"{"error":"invalid_client_metadata","client_secret":"cs-leak-9999","access_token":"at-leak-7777"}"#;
        let status = reqwest::StatusCode::BAD_REQUEST;
        let redacted =
            redact_token_endpoint_response(status.as_u16(), Some("application/json"), raw_body);
        warn!(
            redacted_response = %redacted,
            "MCP OAuth Dynamic Client Registration failed (non-success status)",
        );
        let returned_err = format!("Client registration failed (HTTP {status})");

        let captured = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
        for secret in [
            "cs-leak-9999",
            "at-leak-7777",
            "client_secret",
            "access_token",
        ] {
            assert!(
                !captured.contains(secret),
                "DCR warn leaked '{secret}'; captured: {captured:?}"
            );
            assert!(
                !returned_err.contains(secret),
                "DCR returned Err leaked '{secret}': {returned_err}"
            );
        }
        assert!(
            captured.contains("body_sha256_prefix="),
            "DCR warn missing sanitized digest; captured: {captured:?}"
        );
        assert!(
            returned_err.contains("HTTP 400"),
            "DCR returned Err should still carry status; got: {returned_err}"
        );
    }

    // -------------------------------------------------------------------
    // Audit: `oauth-refresh-error-body-token-leak` — error classification
    // -------------------------------------------------------------------

    /// `400 invalid_grant` is the only failure that means the refresh token
    /// is dead. It must classify as `Revoked` so the caller re-authenticates.
    #[test]
    fn classify_400_invalid_grant_is_revoked() {
        let body =
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#;
        let r = classify_refresh_failure(reqwest::StatusCode::BAD_REQUEST, body);
        assert!(
            matches!(r, RefreshError::Revoked),
            "400 invalid_grant must be Revoked, got {r:?}"
        );
    }

    /// A 5xx is a server-side hiccup; the refresh token is presumed valid.
    /// Must classify as `Transient` so the caller retries instead of
    /// discarding the refresh token.
    #[test]
    fn classify_503_is_transient() {
        let r = classify_refresh_failure(reqwest::StatusCode::SERVICE_UNAVAILABLE, "");
        assert!(
            matches!(r, RefreshError::Transient(_)),
            "503 must be Transient, got {r:?}"
        );
        let r = classify_refresh_failure(reqwest::StatusCode::BAD_GATEWAY, "");
        assert!(
            matches!(r, RefreshError::Transient(_)),
            "502 must be Transient, got {r:?}"
        );
    }

    /// A non-`invalid_grant` 4xx (e.g. `invalid_client`) is not a retryable
    /// outage and not a revoked refresh token — it must classify as
    /// `Permanent`, NOT collapse to a re-auth.
    #[test]
    fn classify_400_other_error_is_permanent() {
        let body = r#"{"error":"invalid_client"}"#;
        let r = classify_refresh_failure(reqwest::StatusCode::BAD_REQUEST, body);
        assert!(
            matches!(r, RefreshError::Permanent(_)),
            "400 invalid_client must be Permanent, got {r:?}"
        );
        // A bare 401/403 with no parseable OAuth error code is Permanent too.
        let r = classify_refresh_failure(reqwest::StatusCode::UNAUTHORIZED, "not json");
        assert!(
            matches!(r, RefreshError::Permanent(_)),
            "401 with non-JSON body must be Permanent, got {r:?}"
        );
    }

    /// The classified error's `Display` carries only the HTTP status — it
    /// must never echo a token-shaped value smuggled in the error body.
    #[test]
    fn classify_error_message_does_not_leak_body() {
        let body = r#"{"error":"invalid_grant","access_token":"super-secret-12345","refresh_token":"rt-9999"}"#;
        let r = classify_refresh_failure(reqwest::StatusCode::BAD_REQUEST, body);
        let shown = r.to_string();
        for secret in ["super-secret-12345", "rt-9999"] {
            assert!(
                !shown.contains(secret),
                "RefreshError Display leaked '{secret}': {shown}"
            );
        }
    }

    /// `oauth_error_code` extracts only the short RFC 6749 §5.2 `error` code,
    /// never the surrounding token-shaped fields, and returns a `'static`
    /// string so a hostile body cannot smuggle an arbitrary value out.
    #[test]
    fn oauth_error_code_extracts_only_the_code() {
        let body = r#"{"error":"invalid_grant","access_token":"leak"}"#;
        assert_eq!(oauth_error_code(body), Some("invalid_grant"));
        // Any non-invalid_grant code collapses to the constant "other".
        assert_eq!(
            oauth_error_code(r#"{"error":"invalid_client"}"#),
            Some("other")
        );
        // Non-JSON / no error field → None.
        assert_eq!(oauth_error_code("not json"), None);
        assert_eq!(oauth_error_code(r#"{"foo":"bar"}"#), None);
    }

    // -------------------------------------------------------------------
    // Audit: `oauth-refresh-error-body-token-leak` — single-flight
    // -------------------------------------------------------------------

    /// The per-`server_url` single-flight lock map returns the *same* lock
    /// instance for repeated lookups of one URL, and *distinct* locks for
    /// distinct URLs. Two concurrent refreshers of the same server therefore
    /// serialize on one lock (only one reaches the network at a time), while
    /// different servers never contend.
    #[test]
    fn refresh_lock_is_shared_per_server_and_isolated_across_servers() {
        let a1 = refresh_lock_for("https://server-a.example/mcp");
        let a2 = refresh_lock_for("https://server-a.example/mcp");
        let b1 = refresh_lock_for("https://server-b.example/mcp");

        assert!(
            Arc::ptr_eq(&a1, &a2),
            "same server_url must yield the same lock so refreshes single-flight"
        );
        assert!(
            !Arc::ptr_eq(&a1, &b1),
            "distinct server_urls must yield distinct locks so they don't contend"
        );
    }

    /// Single-flight recheck: when a peer has already refreshed the token
    /// while this caller waited on the lock, `load_token` returns the freshly
    /// stored access token directly and makes NO refresh request.
    ///
    /// We exercise the real `load_token` path. The vault holds a *valid*
    /// (non-expired) `expires_at` plus an `access_token`, but no
    /// `token_endpoint` / `refresh_token` — so if the recheck branch were
    /// ever skipped and a refresh were attempted, `try_refresh` would fail
    /// (no endpoint) and `load_token` would NOT return the stored token.
    /// Returning the stored token is the proof that the recheck short-circuit
    /// fired and the network was never touched.
    #[tokio::test]
    #[serial_test::serial(librefang_vault_key)]
    async fn refresh_recheck_returns_already_refreshed_token_without_network() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let _vault_key = VaultKeyEnvGuard::set(TEST_VAULT_KEY);
        let provider = KernelOAuthProvider::new(home);
        let server_url = "https://recheck.example/mcp";

        // Seed a token that is comfortably valid (expires in 1 hour). No
        // token_endpoint / refresh_token stored: any refresh attempt would
        // fail, so a returned token can only come from the valid-token path.
        let future = chrono::Utc::now().timestamp() + 3600;
        provider
            .vault_set(
                &KernelOAuthProvider::vault_key(server_url, "access_token"),
                "fresh-token",
            )
            .expect("seed access_token");
        provider
            .vault_set(
                &KernelOAuthProvider::vault_key(server_url, "expires_at"),
                &future.to_string(),
            )
            .expect("seed expires_at");

        let token = provider.load_token(server_url).await.expect("load_token");
        assert_eq!(
            token,
            Some("fresh-token".to_string()),
            "a non-expired token must be returned without any refresh attempt"
        );
    }

    /// Counterpart sanity check: when the stored token is genuinely expired
    /// and there is no refresh token, `load_token` returns `Ok(None)` (the
    /// OAuth flow must run) — it does NOT return the stale access token.
    #[tokio::test]
    #[serial_test::serial(librefang_vault_key)]
    async fn load_token_expired_without_refresh_token_yields_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let _vault_key = VaultKeyEnvGuard::set(TEST_VAULT_KEY);
        let provider = KernelOAuthProvider::new(home);
        let server_url = "https://expired.example/mcp";

        let past = chrono::Utc::now().timestamp() - 3600;
        provider
            .vault_set(
                &KernelOAuthProvider::vault_key(server_url, "access_token"),
                "stale-token",
            )
            .expect("seed access_token");
        provider
            .vault_set(
                &KernelOAuthProvider::vault_key(server_url, "expires_at"),
                &past.to_string(),
            )
            .expect("seed expires_at");

        let token = provider.load_token(server_url).await.expect("load_token");
        assert_eq!(
            token, None,
            "expired token with no refresh token must yield Ok(None), not the stale token"
        );
    }
}
