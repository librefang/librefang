//! OAuth2/OIDC external authentication support.
//!
//! Provides:
//! - OIDC discovery (fetches `.well-known/openid-configuration`) with caching
//! - Multi-provider support (Google, GitHub, Azure AD, Keycloak, generic OIDC)
//! - Login redirect to the external identity provider (per-provider)
//! - Authorization code callback and token exchange with CSRF protection
//! - JWT validation with JWKS caching and nonce verification
//! - Token introspection endpoint
//! - User info extraction from ID tokens
//! - Auth middleware for injecting user claims into request extensions

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use base64::Engine;
use hmac::{Hmac, Mac};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::middleware::AccountId;
use crate::routes::AppState;

type HmacSha256 = Hmac<Sha256>;

// ── OIDC Discovery ──────────────────────────────────────────────────────

/// Subset of the OpenID Connect Discovery 1.0 response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OidcDiscovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub userinfo_endpoint: Option<String>,
    pub jwks_uri: String,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    #[serde(default)]
    pub id_token_signing_alg_values_supported: Vec<String>,
}

/// JWKS key entry.
#[derive(Debug, Clone, Deserialize)]
pub struct JwksKey {
    pub kty: String,
    #[serde(default)]
    pub kid: Option<String>,
    #[serde(rename = "use", default)]
    pub key_use: Option<String>,
    #[serde(default)]
    pub alg: Option<String>,
    /// RSA modulus (base64url-encoded).
    #[serde(default)]
    pub n: Option<String>,
    /// RSA exponent (base64url-encoded).
    #[serde(default)]
    pub e: Option<String>,
    /// EC x coordinate (base64url-encoded).
    #[serde(default)]
    pub x: Option<String>,
    /// EC y coordinate (base64url-encoded).
    #[serde(default)]
    pub y: Option<String>,
    /// EC curve name.
    #[serde(default)]
    pub crv: Option<String>,
}

/// JWKS response.
#[derive(Debug, Deserialize)]
pub struct JwksResponse {
    pub keys: Vec<JwksKey>,
}

/// Claims extracted from the OIDC ID token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdTokenClaims {
    /// Subject (unique user identifier from the IdP).
    #[serde(default)]
    pub sub: String,
    /// User email (if `email` scope was granted).
    #[serde(default)]
    pub email: Option<String>,
    /// Whether the email is verified.
    #[serde(default)]
    pub email_verified: Option<bool>,
    /// User display name.
    #[serde(default)]
    pub name: Option<String>,
    /// User's picture URL.
    #[serde(default)]
    pub picture: Option<String>,
    /// Roles (from custom claims).
    #[serde(default)]
    pub roles: Vec<String>,
    /// Issuer.
    #[serde(default)]
    pub iss: String,
    /// Audience.
    #[serde(default)]
    pub aud: OidcAudience,
    /// Issued at.
    #[serde(default)]
    pub iat: Option<u64>,
    /// Expiration.
    #[serde(default)]
    pub exp: Option<u64>,
    /// Nonce (for replay protection).
    #[serde(default)]
    pub nonce: Option<String>,
}

/// OIDC `aud` claim can be a single string or an array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OidcAudience {
    Single(String),
    Multiple(Vec<String>),
}

impl Default for OidcAudience {
    fn default() -> Self {
        Self::Single(String::new())
    }
}

impl OidcAudience {
    /// Check if the audience contains the given value.
    pub fn contains(&self, value: &str) -> bool {
        match self {
            Self::Single(s) => s == value,
            Self::Multiple(v) => v.iter().any(|s| s == value),
        }
    }
}

// ── JWKS Cache ──────────────────────────────────────────────────────────

/// Cached JWKS keyset for a provider.
struct CachedJwks {
    keys: Vec<JwksKey>,
    fetched_at: std::time::Instant,
}

/// In-memory JWKS cache shared across requests. Maps JWKS URI to cached keys.
#[derive(Default)]
pub struct JwksCache {
    inner: RwLock<HashMap<String, CachedJwks>>,
}

/// JWKS cache TTL — 1 hour. Providers rotate keys infrequently.
const JWKS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Global JWKS cache instance (lazily initialized).
static JWKS_CACHE: std::sync::LazyLock<JwksCache> = std::sync::LazyLock::new(JwksCache::default);

// ── Discovery Cache ─────────────────────────────────────────────────────

/// Cached OIDC discovery document.
struct CachedDiscovery {
    doc: OidcDiscovery,
    fetched_at: std::time::Instant,
}

/// In-memory OIDC discovery cache. Maps issuer URL to cached discovery doc.
#[derive(Default)]
struct DiscoveryCache {
    inner: RwLock<HashMap<String, CachedDiscovery>>,
}

/// Discovery cache TTL — 1 hour.
const DISCOVERY_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Global OIDC discovery cache instance.
static DISCOVERY_CACHE: std::sync::LazyLock<DiscoveryCache> =
    std::sync::LazyLock::new(DiscoveryCache::default);

// ── State (CSRF) ────────────────────────────────────────────────────────

/// State parameter payload encoded as JSON and HMAC-signed.
/// Encodes the provider ID and a nonce so the callback can route correctly
/// and validate against CSRF.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthStatePayload {
    /// Provider ID (e.g. "google", "github").
    provider: String,
    /// Owning tenant account for this auth flow when initiated from a tenant context.
    account_id: Option<String>,
    /// Random nonce for CSRF protection.
    nonce: String,
    /// Timestamp (seconds since UNIX epoch) for expiry checking.
    ts: u64,
}

/// State token TTL — 10 minutes. Login flows should complete quickly.
const STATE_TOKEN_TTL_SECS: u64 = 600;

/// Build an HMAC-signed state parameter containing provider + nonce.
fn build_state_token(provider_id: &str, account_id: Option<&str>) -> String {
    let nonce = uuid::Uuid::new_v4().to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let payload = OAuthStatePayload {
        provider: provider_id.to_string(),
        account_id: account_id.map(str::to_owned),
        nonce: nonce.clone(),
        ts,
    };
    let payload_json = serde_json::to_string(&payload).unwrap_or_default();
    let payload_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());

    // HMAC-sign the payload.
    let key = state_signing_key();
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload_b64.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);

    // Format: payload.signature (both base64url)
    format!("{payload_b64}.{sig_b64}")
}

fn require_account_bound_oauth(
    multi_tenant: bool,
    account_id: Option<&str>,
) -> Result<(), &'static str> {
    if multi_tenant && account_id.is_none() {
        Err("OAuth endpoints require X-Account-Id in multi-tenant mode")
    } else {
        Ok(())
    }
}

fn validate_callback_tenant_binding(
    multi_tenant: bool,
    state_account_id: Option<&str>,
    callback_account_id: Option<&str>,
) -> Result<(), &'static str> {
    if !multi_tenant {
        if let Some(callback_account_id) = callback_account_id {
            if state_account_id != Some(callback_account_id) {
                return Err("OAuth tenant mismatch");
            }
        }
        return Ok(());
    }

    match (state_account_id, callback_account_id) {
        (Some(state_account_id), Some(callback_account_id))
            if state_account_id == callback_account_id =>
        {
            Ok(())
        }
        _ => Err("OAuth tenant mismatch"),
    }
}

/// Verify and decode a state token. Returns the payload if valid.
fn verify_state_token(state: &str) -> Result<OAuthStatePayload, String> {
    let parts: Vec<&str> = state.splitn(2, '.').collect();
    if parts.len() != 2 {
        return Err("Invalid state format".to_string());
    }
    let (payload_b64, sig_b64) = (parts[0], parts[1]);

    // Verify HMAC.
    let key = state_signing_key();
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload_b64.as_bytes());
    let expected_sig = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| "Invalid state signature encoding")?;
    mac.verify_slice(&expected_sig)
        .map_err(|_| "State signature verification failed")?;

    // Decode payload.
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| "Invalid state payload encoding")?;
    let payload: OAuthStatePayload =
        serde_json::from_slice(&payload_bytes).map_err(|_| "Invalid state payload JSON")?;

    // Check expiry.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(payload.ts) > STATE_TOKEN_TTL_SECS {
        return Err("State token expired".to_string());
    }

    Ok(payload)
}

/// Derive the HMAC signing key for state tokens. Uses LIBREFANG_STATE_SECRET
/// env var if set, otherwise falls back to a random per-process key.
///
/// **Production requirement**: set `LIBREFANG_STATE_SECRET` to a stable secret.
/// Without it, OAuth state tokens are invalidated on every restart and will not
/// work across multiple server instances (load balancer / rolling deploy).
fn state_signing_key() -> String {
    static KEY: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| match std::env::var("LIBREFANG_STATE_SECRET") {
            Ok(key) if !key.trim().is_empty() => key,
            _ => {
                tracing::warn!(
                    "LIBREFANG_STATE_SECRET is not set — OAuth state tokens will not survive \
                     server restarts or load-balanced deployments. Set this environment variable \
                     to a stable secret in production."
                );
                uuid::Uuid::new_v4().to_string()
            }
        });
    KEY.clone()
}

// ── Resolved Provider ───────────────────────────────────────────────────

/// Resolved provider endpoints (after OIDC discovery or explicit config).
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedProvider {
    pub id: String,
    pub display_name: String,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub jwks_uri: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub redirect_url: String,
    #[serde(skip)]
    pub client_secret_env: String,
    #[serde(skip)]
    pub allowed_domains: Vec<String>,
    #[serde(skip)]
    pub audience: String,
}

// ── Token exchange response ─────────────────────────────────────────────

/// OAuth2 token endpoint response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

// ── Token Store ─────────────────────────────────────────────────────────

/// Stored token entry for a user session, keyed by user subject (`sub`).
#[derive(Debug, Clone)]
struct StoredTokens {
    /// The OAuth2 access token (stored for future introspection/revocation).
    #[allow(dead_code)]
    access_token: String,
    /// Optional refresh token for obtaining new access tokens.
    refresh_token: Option<String>,
    /// When the access token expires (absolute time).
    #[allow(dead_code)]
    expires_at: Option<std::time::Instant>,
    /// Provider ID that issued these tokens.
    provider_id: String,
    /// Owning tenant account for this stored token set.
    account_id: Option<String>,
    /// When this entry was stored (for TTL eviction).
    stored_at: std::time::Instant,
}

/// Token store entries older than 24 hours are evicted on access.
const TOKEN_STORE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// In-memory token store. Maps user `sub` to their stored tokens.
#[derive(Default)]
pub struct TokenStore {
    inner: RwLock<HashMap<(String, Option<String>), StoredTokens>>,
}

/// Global token store instance.
static TOKEN_STORE: std::sync::LazyLock<TokenStore> = std::sync::LazyLock::new(TokenStore::default);

impl TokenStore {
    /// Store tokens for a user.
    async fn store(&self, sub: &str, account_id: Option<&str>, tokens: StoredTokens) {
        let mut write = self.inner.write().await;
        write.insert((sub.to_string(), account_id.map(str::to_owned)), tokens);
    }

    /// Retrieve stored tokens for a user, evicting if older than TTL.
    #[allow(dead_code)]
    async fn get(&self, sub: &str, account_id: Option<&str>) -> Option<StoredTokens> {
        let mut write = self.inner.write().await;
        let key = (sub.to_string(), account_id.map(str::to_owned));
        if let Some(entry) = write.get(&key) {
            if entry.stored_at.elapsed() > TOKEN_STORE_TTL {
                debug!(sub = %sub, "Evicting expired token store entry (>24h)");
                write.remove(&key);
                return None;
            }
            return Some(entry.clone());
        }
        None
    }

    /// Remove stored tokens for a user (e.g., on logout).
    #[allow(dead_code)]
    async fn remove(&self, sub: &str, account_id: Option<&str>) {
        let mut write = self.inner.write().await;
        write.remove(&(sub.to_string(), account_id.map(str::to_owned)));
    }

    /// Find a stored entry by provider ID, evicting expired entries along the way.
    async fn find_by_provider(
        &self,
        provider_id: &str,
        account_id: Option<&str>,
    ) -> Option<(String, StoredTokens)> {
        let mut write = self.inner.write().await;
        let now = std::time::Instant::now();

        // Evict expired entries.
        write.retain(|_sub, entry| now.duration_since(entry.stored_at) <= TOKEN_STORE_TTL);

        write
            .iter()
            .find(|((_, stored_account_id), entry)| {
                entry.provider_id == provider_id
                    && stored_account_id.as_deref() == account_id
                    && entry.account_id.as_deref() == account_id
            })
            .map(|((sub, _), entry)| (sub.clone(), entry.clone()))
    }

    /// Find any stored entry with a refresh token, evicting expired entries.
    async fn find_any_with_refresh(
        &self,
        account_id: Option<&str>,
    ) -> Option<(String, StoredTokens)> {
        let mut write = self.inner.write().await;
        let now = std::time::Instant::now();

        // Evict expired entries.
        write.retain(|_sub, entry| now.duration_since(entry.stored_at) <= TOKEN_STORE_TTL);

        write
            .iter()
            .find(|((_, stored_account_id), entry)| {
                entry.refresh_token.is_some()
                    && stored_account_id.as_deref() == account_id
                    && entry.account_id.as_deref() == account_id
            })
            .map(|((sub, _), entry)| (sub.clone(), entry.clone()))
    }
}

// ── Route: GET /api/auth/providers ──────────────────────────────────────

/// GET /api/auth/providers — List available authentication providers.
#[utoipa::path(get, path = "/api/auth/providers", tag = "auth", responses((status = 200, description = "List configured OAuth/OIDC providers", body = serde_json::Value)))]
pub async fn auth_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let ext_auth = &cfg.external_auth;

    if !ext_auth.enabled {
        return Json(serde_json::json!({
            "enabled": false,
            "providers": [],
        }));
    }

    let providers = resolve_providers(ext_auth).await;
    let summary: Vec<serde_json::Value> = providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "display_name": p.display_name,
                "scopes": p.scopes,
            })
        })
        .collect();

    Json(serde_json::json!({
        "enabled": true,
        "providers": summary,
    }))
}

// ── Route: GET /api/auth/login ──────────────────────────────────────────

/// GET /api/auth/login — Redirect to the external identity provider (legacy single-provider).
#[utoipa::path(get, path = "/api/auth/login", tag = "auth", responses((status = 302, description = "Redirect to OAuth provider login")))]
pub async fn auth_login(State(state): State<Arc<AppState>>) -> Response {
    let cfg = state.kernel.config_snapshot();
    let ext_auth = &cfg.external_auth;
    if !ext_auth.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "External authentication is not configured"})),
        )
            .into_response();
    }

    if cfg.multi_tenant {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Legacy /api/auth/login is unavailable in multi-tenant mode; use an account-bound flow"
            })),
        )
            .into_response();
    }

    let providers = resolve_providers(ext_auth).await;
    let provider = match providers.first() {
        Some(p) => p,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "No auth providers configured"})),
            )
                .into_response();
        }
    };

    build_login_redirect(provider, None).into_response()
}

/// GET /api/auth/login/:provider — Redirect to a specific provider.
#[utoipa::path(get, path = "/api/auth/login/{provider}", tag = "auth", params(("provider" = String, Path, description = "OAuth provider name")), responses((status = 302, description = "Redirect to specific OAuth provider")))]
pub async fn auth_login_provider(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Path(provider_id): Path<String>,
) -> Response {
    let cfg = state.kernel.config_snapshot();
    let ext_auth = &cfg.external_auth;
    if !ext_auth.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "External authentication is not configured"})),
        )
            .into_response();
    }

    if let Err(message) = require_account_bound_oauth(cfg.multi_tenant, account.0.as_deref()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": message })),
        )
            .into_response();
    }

    let providers = resolve_providers(ext_auth).await;
    let provider = match providers.iter().find(|p| p.id == provider_id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Unknown auth provider: {provider_id}")})),
            )
                .into_response();
        }
    };

    build_login_redirect(provider, account.0.as_deref()).into_response()
}

/// Build the OAuth2 authorization redirect for the given provider.
/// Generates a signed state token encoding the provider ID and a nonce.
fn build_login_redirect(
    provider: &ResolvedProvider,
    account_id: Option<&str>,
) -> impl IntoResponse {
    let state_token = build_state_token(&provider.id, account_id);
    // Extract the nonce from the state for the OIDC nonce parameter.
    let nonce = if let Ok(payload) = verify_state_token(&state_token) {
        payload.nonce
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let scopes = provider.scopes.join(" ");

    match url::Url::parse_with_params(
        &provider.auth_url,
        &[
            ("response_type", "code"),
            ("client_id", &provider.client_id),
            ("redirect_uri", &provider.redirect_url),
            ("scope", &scopes),
            ("state", &state_token),
            ("nonce", &nonce),
        ],
    ) {
        Ok(auth_url) => {
            info!(
                provider = %provider.id,
                "Redirecting to external IdP for login"
            );
            Redirect::temporary(auth_url.as_str()).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to build auth URL: {e}")})),
        )
            .into_response(),
    }
}

// ── Route: POST /api/auth/callback ──────────────────────────────────────

/// Query params for the OAuth2 callback (GET-based callback from IdP redirect).
#[derive(Deserialize)]
pub struct CallbackQuery {
    /// Authorization code from the IdP.
    #[serde(default)]
    pub code: Option<String>,
    /// State parameter (signed CSRF token with embedded provider).
    #[serde(default)]
    pub state: Option<String>,
    /// Error from the IdP (if authorization was denied).
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// POST body for the callback (programmatic clients).
#[derive(Deserialize, utoipa::ToSchema)]
pub struct CallbackBody {
    /// Authorization code.
    pub code: String,
    /// State parameter for CSRF validation (signed token).
    pub state: String,
}

/// Callback response with session token.
#[derive(Serialize)]
struct CallbackResponse {
    /// Session token for authenticating subsequent API calls.
    token: String,
    /// Token type (always "Bearer").
    token_type: String,
    /// Token lifetime in seconds.
    expires_in: u64,
    /// Provider that authenticated the user.
    provider: String,
    /// Owning tenant account associated with this OAuth flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    /// User info extracted from the ID token.
    user: CallbackUser,
    /// Refresh token (if the provider issued one). Clients should store this
    /// and use `POST /api/auth/refresh` when the access token expires.
    ///
    /// SECURITY: Returning the refresh token to the client is acceptable here because
    /// LibreFang is a local agent system — the "client" is always the local dashboard
    /// or CLI running on the same machine, not a remote browser. The API is bound to
    /// 127.0.0.1 by default and protected by the existing API key middleware.
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

#[derive(Serialize)]
struct CallbackUser {
    sub: String,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
}

/// GET /api/auth/callback — Handle the OAuth2 authorization code callback (browser redirect).
#[utoipa::path(get, path = "/api/auth/callback", tag = "auth", responses((status = 200, description = "OAuth callback — completes login flow", body = serde_json::Value)))]
pub async fn auth_callback(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Query(query): Query<CallbackQuery>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let ext_auth = &cfg.external_auth;
    if !ext_auth.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "External authentication is not configured"})),
        )
            .into_response();
    }

    // Check for IdP errors.
    if let Some(ref err) = query.error {
        let desc = query.error_description.as_deref().unwrap_or("unknown");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": err,
                "error_description": desc
            })),
        )
            .into_response();
    }

    let code = match query.code {
        Some(ref c) if !c.is_empty() => c.clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing authorization code"})),
            )
                .into_response();
        }
    };

    // SECURITY: Validate the state parameter (CSRF protection).
    let state_str = match query.state {
        Some(ref s) if !s.is_empty() => s.clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing state parameter"})),
            )
                .into_response();
        }
    };

    let state_payload = match verify_state_token(&state_str) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "CSRF state validation failed");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid or expired state parameter"})),
            )
                .into_response();
        }
    };

    handle_code_exchange(
        ext_auth,
        cfg.multi_tenant,
        &code,
        &state_payload,
        account.0.as_deref(),
    )
    .await
}

/// POST /api/auth/callback — Handle the OAuth2 callback (programmatic clients).
#[utoipa::path(post, path = "/api/auth/callback", tag = "auth", responses((status = 200, description = "OAuth callback (POST) — completes login flow", body = serde_json::Value)))]
pub async fn auth_callback_post(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Json(body): Json<CallbackBody>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let ext_auth = &cfg.external_auth;
    if !ext_auth.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "External authentication is not configured"})),
        )
            .into_response();
    }

    // SECURITY: Validate the state parameter (CSRF protection).
    let state_payload = match verify_state_token(&body.state) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "CSRF state validation failed (POST)");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid or expired state parameter"})),
            )
                .into_response();
        }
    };

    handle_code_exchange(
        ext_auth,
        cfg.multi_tenant,
        &body.code,
        &state_payload,
        account.0.as_deref(),
    )
    .await
}

/// Shared code exchange logic for both GET and POST callback handlers.
async fn handle_code_exchange(
    ext_auth: &librefang_types::config::ExternalAuthConfig,
    multi_tenant: bool,
    code: &str,
    state_payload: &OAuthStatePayload,
    callback_account_id: Option<&str>,
) -> Response {
    if let Err(message) = validate_callback_tenant_binding(
        multi_tenant,
        state_payload.account_id.as_deref(),
        callback_account_id,
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": message})),
        )
            .into_response();
    }

    let providers = resolve_providers(ext_auth).await;

    // Route to the provider encoded in the state token.
    let provider = match providers.iter().find(|p| p.id == state_payload.provider) {
        Some(p) => p,
        None => {
            // If the encoded provider is not found but there is exactly one provider,
            // use it (graceful degradation for legacy clients).
            if providers.len() == 1 {
                &providers[0]
            } else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("Unknown auth provider: {}", state_payload.provider)
                    })),
                )
                    .into_response();
            }
        }
    };

    // Resolve client secret from environment variable.
    let client_secret = std::env::var(&provider.client_secret_env).unwrap_or_default();
    if client_secret.is_empty() {
        warn!(
            env_var = %provider.client_secret_env,
            provider = %provider.id,
            "OAuth client secret env var is empty"
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "OAuth client secret not configured"})),
        )
            .into_response();
    }

    // Exchange authorization code for tokens.
    let token_resp = match exchange_code(
        &provider.token_url,
        code,
        &provider.client_id,
        &client_secret,
        &provider.redirect_url,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            // SECURITY: Log full error at debug level, return generic message to client.
            debug!("Token exchange failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Token exchange failed"})),
            )
                .into_response();
        }
    };

    // Try to validate the ID token if present.
    let claims = if let Some(ref id_token) = token_resp.id_token {
        if !id_token.is_empty() && !provider.jwks_uri.is_empty() {
            match validate_jwt_cached(id_token, &provider.jwks_uri, &provider.audience).await {
                Ok(c) => {
                    // Verify nonce matches the one from our state token.
                    if let Some(ref token_nonce) = c.nonce {
                        if token_nonce != &state_payload.nonce {
                            warn!("Nonce mismatch in ID token");
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({"error": "Nonce mismatch in ID token"})),
                            )
                                .into_response();
                        }
                    }
                    Some(c)
                }
                Err(e) => {
                    debug!(error = %e, "ID token validation failed, falling back to userinfo");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // If no claims from ID token, try the userinfo endpoint.
    let claims = match claims {
        Some(c) => c,
        None => {
            if !provider.userinfo_url.is_empty() {
                match fetch_userinfo(&provider.userinfo_url, &token_resp.access_token).await {
                    Ok(info) => IdTokenClaims {
                        sub: info["sub"]
                            .as_str()
                            .or(info["id"].as_str())
                            .unwrap_or("")
                            .to_string(),
                        email: info["email"].as_str().map(|s| s.to_string()),
                        email_verified: info["email_verified"].as_bool(),
                        name: info["name"]
                            .as_str()
                            .or(info["login"].as_str())
                            .map(|s| s.to_string()),
                        picture: info["picture"]
                            .as_str()
                            .or(info["avatar_url"].as_str())
                            .map(|s| s.to_string()),
                        roles: Vec::new(),
                        iss: provider.id.clone(),
                        aud: OidcAudience::Single(provider.client_id.clone()),
                        iat: None,
                        exp: None,
                        nonce: None,
                    },
                    Err(e) => {
                        debug!(error = %e, "Userinfo fetch failed");
                        return (
                            StatusCode::BAD_GATEWAY,
                            Json(serde_json::json!({"error": "Could not retrieve user info"})),
                        )
                            .into_response();
                    }
                }
            } else {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": "No ID token and no userinfo endpoint available"})),
                )
                    .into_response();
            }
        }
    };

    // Check allowed domains.
    if !provider.allowed_domains.is_empty() {
        if let Some(ref email) = claims.email {
            let domain = email.rsplit('@').next().unwrap_or("");
            if !provider.allowed_domains.iter().any(|d| d == domain) {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Email domain not authorized",
                        "domain": domain
                    })),
                )
                    .into_response();
            }
        } else {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "Email claim required but not present in token"})),
            )
                .into_response();
        }
    }

    info!(
        sub = %claims.sub,
        email = ?claims.email,
        provider = %provider.id,
        "External auth login successful"
    );

    let expires_in = token_resp.expires_in.unwrap_or(ext_auth.session_ttl_secs);

    // Store tokens so we can refresh later when the access token expires.
    let expires_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(expires_in));
    TOKEN_STORE
        .store(
            &claims.sub,
            state_payload.account_id.as_deref(),
            StoredTokens {
                access_token: token_resp.access_token.clone(),
                refresh_token: token_resp.refresh_token.clone(),
                expires_at,
                provider_id: provider.id.clone(),
                account_id: state_payload.account_id.clone(),
                stored_at: std::time::Instant::now(),
            },
        )
        .await;

    (
        StatusCode::OK,
        Json(CallbackResponse {
            token: token_resp.access_token,
            token_type: "Bearer".to_string(),
            expires_in,
            provider: provider.id.clone(),
            account_id: state_payload.account_id.clone(),
            user: CallbackUser {
                sub: claims.sub,
                email: claims.email,
                name: claims.name,
                picture: claims.picture,
            },
            refresh_token: token_resp.refresh_token,
        }),
    )
        .into_response()
}

// ── Route: GET /api/auth/userinfo ───────────────────────────────────────

/// GET /api/auth/userinfo — Return info about the currently authenticated user.
///
/// If a valid JWT is in the Authorization header and JWKS validation succeeds,
/// returns the decoded claims. Otherwise falls back to provider userinfo endpoint.
#[utoipa::path(get, path = "/api/auth/userinfo", tag = "auth", responses((status = 200, description = "Get authenticated user info", body = serde_json::Value), (status = 401, description = "Not authenticated")))]
pub async fn auth_userinfo(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    let kcfg = state.kernel.config_ref();
    let ext_auth = &kcfg.external_auth;

    if !ext_auth.enabled {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "auth_method": "api_key",
                "issuer": "",
            })),
        )
            .into_response();
    }

    // Try to extract and validate the Bearer token.
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let Some(token) = token else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Missing Bearer token"})),
        )
            .into_response();
    };

    let providers = resolve_providers(ext_auth).await;

    // Try JWT validation against each provider's JWKS.
    for provider in &providers {
        if provider.jwks_uri.is_empty() {
            continue;
        }
        if let Ok(claims) = validate_jwt_cached(token, &provider.jwks_uri, &provider.audience).await
        {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "auth_method": "external_oauth",
                    "provider": provider.id,
                    "sub": claims.sub,
                    "email": claims.email,
                    "name": claims.name,
                    "picture": claims.picture,
                    "roles": claims.roles,
                    "email_verified": claims.email_verified,
                })),
            )
                .into_response();
        }
    }

    // Fallback: try userinfo endpoint with the token as access token.
    for provider in &providers {
        if provider.userinfo_url.is_empty() {
            continue;
        }
        if let Ok(info) = fetch_userinfo(&provider.userinfo_url, token).await {
            return (StatusCode::OK, Json(info)).into_response();
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "Token could not be validated against any provider"})),
    )
        .into_response()
}

// ── Route: POST /api/auth/introspect ────────────────────────────────────

/// Token introspection request body.
#[derive(Deserialize)]
pub struct IntrospectRequest {
    /// The token to introspect.
    pub token: String,
    /// Optional provider hint.
    #[serde(default)]
    pub provider: Option<String>,
}

/// POST /api/auth/introspect — Validate a token and return its claims.
///
/// Follows RFC 7662 conventions: returns `{"active": true/false, ...}`.
#[utoipa::path(post, path = "/api/auth/introspect", tag = "auth", request_body = serde_json::Value, responses((status = 200, description = "Token introspection result", body = serde_json::Value)))]
pub async fn auth_introspect(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IntrospectRequest>,
) -> impl IntoResponse {
    let kcfg = state.kernel.config_ref();
    let ext_auth = &kcfg.external_auth;
    if !ext_auth.enabled {
        return Json(serde_json::json!({
            "active": false,
            "error": "External auth is not enabled"
        }));
    }

    let providers = resolve_providers(ext_auth).await;

    // If provider hint is given, only try that one.
    let candidates: Vec<&ResolvedProvider> = if let Some(ref pid) = req.provider {
        providers.iter().filter(|p| p.id == *pid).collect()
    } else {
        providers.iter().collect()
    };

    // Try JWT validation against each candidate provider's JWKS.
    for provider in &candidates {
        if provider.jwks_uri.is_empty() {
            continue;
        }
        match validate_jwt_cached(&req.token, &provider.jwks_uri, &provider.audience).await {
            Ok(claims) => {
                return Json(serde_json::json!({
                    "active": true,
                    "provider": provider.id,
                    "sub": claims.sub,
                    "email": claims.email,
                    "name": claims.name,
                    "roles": claims.roles,
                    "iss": claims.iss,
                    "exp": claims.exp,
                    "iat": claims.iat,
                }));
            }
            Err(e) => {
                debug!(provider = %provider.id, error = %e, "JWT validation failed for provider");
            }
        }
    }

    Json(serde_json::json!({
        "active": false,
        "error": "Token could not be validated against any configured provider"
    }))
}

// ── Route: POST /api/auth/refresh ────────────────────────────────────────

/// Request body for the refresh token endpoint.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct RefreshRequest {
    /// The refresh token obtained from the initial login callback.
    /// If omitted, the server looks up a stored refresh token from the token store.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Optional provider hint (if the user logged in with a specific provider).
    #[serde(default)]
    pub provider: Option<String>,
}

/// Response from the refresh token endpoint.
#[derive(Serialize)]
struct RefreshResponse {
    /// New access token.
    token: String,
    /// Token type (always "Bearer").
    token_type: String,
    /// Token lifetime in seconds.
    expires_in: u64,
    /// New refresh token (if the provider rotated it).
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

/// POST /api/auth/refresh — Exchange a refresh token for a new access token.
///
/// When the access token expires, clients can call this endpoint with the
/// refresh token received during login instead of forcing a full re-authorization.
/// If the request body omits `refresh_token`, the server looks up the token store
/// for a previously stored refresh token (from the OAuth callback).
#[utoipa::path(post, path = "/api/auth/refresh", tag = "auth", request_body = RefreshRequest, responses((status = 200, description = "New access token", body = serde_json::Value), (status = 400, description = "Missing or invalid refresh token"), (status = 502, description = "Token refresh failed")))]
pub async fn auth_refresh(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    Json(req): Json<RefreshRequest>,
) -> impl IntoResponse {
    let kcfg = state.kernel.config_ref();
    let ext_auth = &kcfg.external_auth;
    if !ext_auth.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "External authentication is not configured"})),
        )
            .into_response();
    }

    if let Err(message) = require_account_bound_oauth(kcfg.multi_tenant, account.0.as_deref()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": message })),
        )
            .into_response();
    }

    let providers = resolve_providers(ext_auth).await;

    // Resolve the refresh token: prefer the request body, fall back to TOKEN_STORE.
    let (refresh_token, stored_sub, provider) = if let Some(ref rt) = req.refresh_token {
        // Client supplied a refresh token explicitly.
        let provider = if let Some(ref pid) = req.provider {
            providers.iter().find(|p| p.id == *pid)
        } else if providers.len() == 1 {
            providers.first()
        } else {
            None
        };
        (rt.clone(), None::<String>, provider.cloned())
    } else if let Some(ref pid) = req.provider {
        // No refresh token in request, but provider given — look up from store.
        match TOKEN_STORE
            .find_by_provider(pid, account.0.as_deref())
            .await
        {
            Some((sub, entry)) => match entry.refresh_token {
                Some(rt) => {
                    let provider = providers.iter().find(|p| p.id == *pid).cloned();
                    (rt, Some(sub), provider)
                }
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "No refresh token stored for this provider"
                        })),
                    )
                        .into_response();
                }
            },
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "No stored session found for this provider"
                    })),
                )
                    .into_response();
            }
        }
    } else {
        // Neither refresh token nor provider — try to find any stored refresh token.
        match TOKEN_STORE
            .find_any_with_refresh(account.0.as_deref())
            .await
        {
            Some((sub, entry)) => {
                let provider = providers
                    .iter()
                    .find(|p| p.id == entry.provider_id)
                    .cloned();
                // refresh_token is guaranteed Some by find_any_with_refresh
                (entry.refresh_token.unwrap(), Some(sub), provider)
            }
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "No refresh token provided and none found in token store"
                    })),
                )
                    .into_response();
            }
        }
    };

    let Some(provider) = provider else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Multiple providers configured; please specify 'provider' in the request"
            })),
        )
            .into_response();
    };

    // Resolve client secret.
    let client_secret = std::env::var(&provider.client_secret_env).unwrap_or_default();
    if client_secret.is_empty() {
        warn!(
            env_var = %provider.client_secret_env,
            provider = %provider.id,
            "OAuth client secret env var is empty"
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "OAuth client secret not configured"})),
        )
            .into_response();
    }

    // Exchange the refresh token for new tokens.
    let token_resp = match exchange_refresh_token(
        &provider.token_url,
        &refresh_token,
        &provider.client_id,
        &client_secret,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            debug!("Refresh token exchange failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Token refresh failed"})),
            )
                .into_response();
        }
    };

    let expires_in = token_resp.expires_in.unwrap_or(ext_auth.session_ttl_secs);

    // Update TOKEN_STORE with new tokens so subsequent refreshes work.
    if let Some(ref sub) = stored_sub {
        let expires_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(expires_in));
        TOKEN_STORE
            .store(
                sub,
                account.0.as_deref(),
                StoredTokens {
                    access_token: token_resp.access_token.clone(),
                    refresh_token: token_resp.refresh_token.clone(),
                    expires_at,
                    provider_id: provider.id.clone(),
                    account_id: account.0.clone(),
                    stored_at: std::time::Instant::now(),
                },
            )
            .await;
    }

    info!(provider = %provider.id, "Token refresh successful");

    (
        StatusCode::OK,
        Json(RefreshResponse {
            token: token_resp.access_token,
            token_type: "Bearer".to_string(),
            expires_in,
            refresh_token: token_resp.refresh_token,
        }),
    )
        .into_response()
}

// ── Auth Middleware ──────────────────────────────────────────────────────

/// OIDC auth middleware that extracts and validates Bearer JWT tokens.
///
/// If external auth is disabled, this is a no-op.
/// If enabled, attempts to validate the Bearer token against configured providers
/// and injects `IdTokenClaims` into request extensions for downstream handlers.
/// Does NOT block requests — the existing api_key middleware handles access control.
pub async fn oidc_auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let kcfg = state.kernel.config_ref();
    let config = &kcfg.external_auth;
    if !config.enabled {
        return next.run(request).await;
    }

    // Extract Bearer token.
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    let Some(token) = token else {
        return next.run(request).await;
    };

    // Resolve providers and try to validate.
    let providers = resolve_providers(config).await;
    for provider in &providers {
        if provider.jwks_uri.is_empty() {
            continue;
        }
        match validate_jwt_cached(&token, &provider.jwks_uri, &provider.audience).await {
            Ok(claims) => {
                // SECURITY: Check allowed domains. When allowed_domains is non-empty,
                // tokens without an email claim MUST be rejected.
                if !provider.allowed_domains.is_empty() {
                    if let Some(ref email) = claims.email {
                        let domain = email.rsplit('@').next().unwrap_or("");
                        if !provider.allowed_domains.iter().any(|d| d == domain) {
                            debug!(email = %email, "Email domain not in allowed list");
                            return (
                                StatusCode::FORBIDDEN,
                                Json(serde_json::json!({"error": "Email domain not authorized"})),
                            )
                                .into_response();
                        }
                    } else {
                        // SECURITY: No email claim but domain filtering is required — reject.
                        debug!("Token has no email claim but allowed_domains is configured");
                        return (
                            StatusCode::FORBIDDEN,
                            Json(serde_json::json!({"error": "Email claim required for domain authorization"})),
                        )
                            .into_response();
                    }
                }
                // Inject claims into request extensions.
                request.extensions_mut().insert(claims);
                break;
            }
            Err(e) => {
                debug!(provider = %provider.id, error = %e, "JWT validation failed in middleware");
            }
        }
    }

    next.run(request).await
}

// ── Provider Resolution ─────────────────────────────────────────────────

/// Resolve all configured providers to their endpoints.
///
/// For providers with an `issuer_url`, performs OIDC discovery (cached).
/// For providers with explicit URLs, uses those directly.
/// Falls back to legacy single-provider config if no explicit providers are defined.
pub(crate) async fn resolve_providers(
    config: &librefang_types::config::ExternalAuthConfig,
) -> Vec<ResolvedProvider> {
    let mut resolved = Vec::new();

    // Multi-provider mode.
    for provider in &config.providers {
        match resolve_single_provider(provider).await {
            Ok(p) => resolved.push(p),
            Err(e) => warn!(
                provider_id = %provider.id,
                error = %e,
                "Failed to resolve OIDC provider"
            ),
        }
    }

    // Legacy single-provider fallback.
    if resolved.is_empty() && !config.issuer_url.is_empty() && !config.client_id.is_empty() {
        match discover_oidc_cached(&config.issuer_url).await {
            Ok(disc) => {
                resolved.push(ResolvedProvider {
                    id: "default".to_string(),
                    display_name: "SSO".to_string(),
                    auth_url: disc.authorization_endpoint,
                    token_url: disc.token_endpoint,
                    userinfo_url: disc.userinfo_endpoint.unwrap_or_default(),
                    jwks_uri: disc.jwks_uri,
                    client_id: config.client_id.clone(),
                    scopes: config.scopes.clone(),
                    redirect_url: config.redirect_url.clone(),
                    client_secret_env: config.client_secret_env.clone(),
                    allowed_domains: config.allowed_domains.clone(),
                    audience: if config.audience.is_empty() {
                        config.client_id.clone()
                    } else {
                        config.audience.clone()
                    },
                });
            }
            Err(e) => warn!(error = %e, "Failed to resolve legacy OIDC provider"),
        }
    }

    resolved
}

async fn resolve_single_provider(
    provider: &librefang_types::config::OidcProvider,
) -> Result<ResolvedProvider, String> {
    let display_name = if provider.display_name.is_empty() {
        provider.id.clone()
    } else {
        provider.display_name.clone()
    };

    let audience = if provider.audience.is_empty() {
        provider.client_id.clone()
    } else {
        provider.audience.clone()
    };

    // If explicit URLs are provided, use them directly (e.g., GitHub).
    if !provider.auth_url.is_empty() && !provider.token_url.is_empty() {
        return Ok(ResolvedProvider {
            id: provider.id.clone(),
            display_name,
            auth_url: provider.auth_url.clone(),
            token_url: provider.token_url.clone(),
            userinfo_url: provider.userinfo_url.clone(),
            jwks_uri: provider.jwks_uri.clone(),
            client_id: provider.client_id.clone(),
            scopes: provider.scopes.clone(),
            redirect_url: provider.redirect_url.clone(),
            client_secret_env: provider.client_secret_env.clone(),
            allowed_domains: provider.allowed_domains.clone(),
            audience,
        });
    }

    // Use OIDC discovery (cached).
    if provider.issuer_url.is_empty() {
        return Err(format!(
            "Provider '{}' has no issuer_url and no explicit auth_url/token_url",
            provider.id
        ));
    }

    let disc = discover_oidc_cached(&provider.issuer_url).await?;
    Ok(ResolvedProvider {
        id: provider.id.clone(),
        display_name,
        auth_url: if provider.auth_url.is_empty() {
            disc.authorization_endpoint
        } else {
            provider.auth_url.clone()
        },
        token_url: if provider.token_url.is_empty() {
            disc.token_endpoint
        } else {
            provider.token_url.clone()
        },
        userinfo_url: if provider.userinfo_url.is_empty() {
            disc.userinfo_endpoint.unwrap_or_default()
        } else {
            provider.userinfo_url.clone()
        },
        jwks_uri: if provider.jwks_uri.is_empty() {
            disc.jwks_uri
        } else {
            provider.jwks_uri.clone()
        },
        client_id: provider.client_id.clone(),
        scopes: provider.scopes.clone(),
        redirect_url: provider.redirect_url.clone(),
        client_secret_env: provider.client_secret_env.clone(),
        allowed_domains: provider.allowed_domains.clone(),
        audience,
    })
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Fetch the OIDC discovery document with caching.
async fn discover_oidc_cached(issuer_url: &str) -> Result<OidcDiscovery, String> {
    let key = issuer_url.trim_end_matches('/').to_string();

    // Check cache first.
    {
        let read = DISCOVERY_CACHE.inner.read().await;
        if let Some(cached) = read.get(&key) {
            if cached.fetched_at.elapsed() < DISCOVERY_CACHE_TTL {
                return Ok(cached.doc.clone());
            }
        }
    }

    // Fetch fresh.
    let doc = discover_oidc(issuer_url).await?;

    // Update cache.
    {
        let mut write = DISCOVERY_CACHE.inner.write().await;
        write.insert(
            key,
            CachedDiscovery {
                doc: doc.clone(),
                fetched_at: std::time::Instant::now(),
            },
        );
    }

    Ok(doc)
}

/// Fetch the OIDC discovery document from `{issuer}/.well-known/openid-configuration`.
async fn discover_oidc(issuer_url: &str) -> Result<OidcDiscovery, String> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );
    let resp = librefang_runtime::http_client::new_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OIDC discovery: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "OIDC discovery returned HTTP {}",
            resp.status().as_u16()
        ));
    }
    resp.json::<OidcDiscovery>()
        .await
        .map_err(|e| format!("Failed to parse OIDC discovery: {e}"))
}

/// Exchange an authorization code for tokens at the token endpoint.
async fn exchange_code(
    token_endpoint: &str,
    code: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, String> {
    let client = librefang_runtime::http_client::new_client();
    let resp = client
        .post(token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
        ])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Token request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        // SECURITY: Full error body is returned to caller (which logs at debug level),
        // but caller should NOT forward this to the end user.
        return Err(format!("Token endpoint returned HTTP {status}: {body}"));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))
}

/// Exchange a refresh token for new access/refresh tokens at the token endpoint.
async fn exchange_refresh_token(
    token_endpoint: &str,
    refresh_token: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<TokenResponse, String> {
    let client = librefang_runtime::http_client::new_client();
    let resp = client
        .post(token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Refresh token request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Token endpoint returned HTTP {status} for refresh: {body}"
        ));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| format!("Failed to parse refresh token response: {e}"))
}

/// Fetch JWKS from a URI using the global cache.
async fn fetch_jwks_cached(jwks_uri: &str) -> Result<Vec<JwksKey>, String> {
    // Check cache.
    {
        let read = JWKS_CACHE.inner.read().await;
        if let Some(cached) = read.get(jwks_uri) {
            if cached.fetched_at.elapsed() < JWKS_CACHE_TTL {
                return Ok(cached.keys.clone());
            }
        }
    }

    // Fetch fresh keys.
    debug!(jwks_uri, "Fetching JWKS keys");
    let resp = librefang_runtime::http_client::new_client()
        .get(jwks_uri)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch JWKS: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("JWKS endpoint returned HTTP {}", resp.status()));
    }
    let jwks: JwksResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse JWKS: {e}"))?;

    // Update cache.
    {
        let mut write = JWKS_CACHE.inner.write().await;
        write.insert(
            jwks_uri.to_string(),
            CachedJwks {
                keys: jwks.keys.clone(),
                fetched_at: std::time::Instant::now(),
            },
        );
    }

    Ok(jwks.keys)
}

/// Validate a JWT token against cached JWKS keys.
async fn validate_jwt_cached(
    token: &str,
    jwks_uri: &str,
    expected_audience: &str,
) -> Result<IdTokenClaims, String> {
    let header =
        jsonwebtoken::decode_header(token).map_err(|e| format!("Invalid JWT header: {e}"))?;

    let keys = fetch_jwks_cached(jwks_uri).await?;

    // Find the matching key.
    let key = if let Some(ref kid) = header.kid {
        keys.iter()
            .find(|k| k.kid.as_deref() == Some(kid))
            .ok_or_else(|| format!("No JWKS key found for kid={kid}"))?
    } else {
        // No kid — match by key type.
        let kty = match header.alg {
            Algorithm::ES256 | Algorithm::ES384 => "EC",
            _ => "RSA",
        };
        keys.iter()
            .find(|k| k.kty == kty)
            .ok_or_else(|| format!("No {kty} key found in JWKS"))?
    };

    // Build decoding key.
    let decoding_key = build_decoding_key(key, &header.alg)?;

    // Configure validation.
    let mut validation = Validation::new(header.alg);
    if expected_audience.is_empty() {
        validation.validate_aud = false;
    } else {
        validation.set_audience(&[expected_audience]);
    }
    validation.validate_exp = true;

    let token_data = decode::<IdTokenClaims>(token, &decoding_key, &validation)
        .map_err(|e| format!("JWT validation failed: {e}"))?;

    Ok(token_data.claims)
}

/// Build a `DecodingKey` from a JWK entry.
fn build_decoding_key(jwk: &JwksKey, alg: &Algorithm) -> Result<DecodingKey, String> {
    match alg {
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 => {
            let n = jwk.n.as_deref().ok_or("JWKS key missing 'n' component")?;
            let e = jwk.e.as_deref().ok_or("JWKS key missing 'e' component")?;
            DecodingKey::from_rsa_components(n, e)
                .map_err(|err| format!("Invalid RSA key components: {err}"))
        }
        Algorithm::ES256 | Algorithm::ES384 => {
            let x = jwk.x.as_deref().ok_or("EC JWK missing 'x' field")?;
            let y = jwk.y.as_deref().ok_or("EC JWK missing 'y' field")?;
            DecodingKey::from_ec_components(x, y)
                .map_err(|err| format!("Invalid EC key components: {err}"))
        }
        _ => Err(format!("Unsupported JWT algorithm: {alg:?}")),
    }
}

/// Fetch user info from a userinfo endpoint using an access token.
async fn fetch_userinfo(
    userinfo_url: &str,
    access_token: &str,
) -> Result<serde_json::Value, String> {
    let client = librefang_runtime::http_client::new_client();
    let resp = client
        .get(userinfo_url)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Userinfo fetch failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Userinfo endpoint returned HTTP {status}: {body}"));
    }

    resp.json()
        .await
        .map_err(|e| format!("Userinfo parse failed: {e}"))
}

/// Validate an access/session token against the external auth provider's JWKS.
///
/// Public API for the auth middleware to verify OAuth session tokens.
pub async fn validate_external_token(
    token: &str,
    config: &librefang_types::config::ExternalAuthConfig,
) -> Result<IdTokenClaims, String> {
    let providers = resolve_providers(config).await;
    for provider in &providers {
        if provider.jwks_uri.is_empty() {
            continue;
        }
        match validate_jwt_cached(token, &provider.jwks_uri, &provider.audience).await {
            Ok(claims) => return Ok(claims),
            Err(e) => debug!(provider = %provider.id, error = %e, "Token validation failed"),
        }
    }
    Err("Token could not be validated against any configured provider".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn test_oidc_audience_single() {
        let aud = OidcAudience::Single("my-app".to_string());
        assert!(aud.contains("my-app"));
        assert!(!aud.contains("other"));
    }

    #[test]
    fn test_oidc_audience_multiple() {
        let aud = OidcAudience::Multiple(vec!["app1".to_string(), "app2".to_string()]);
        assert!(aud.contains("app1"));
        assert!(aud.contains("app2"));
        assert!(!aud.contains("app3"));
    }

    #[test]
    fn test_default_external_auth_config() {
        let config = librefang_types::config::ExternalAuthConfig::default();
        assert!(!config.enabled);
        assert!(config.issuer_url.is_empty());
        assert!(config.client_id.is_empty());
        assert_eq!(config.client_secret_env, "LIBREFANG_OAUTH_CLIENT_SECRET");
        assert_eq!(config.scopes.len(), 3);
        assert_eq!(config.session_ttl_secs, 86400);
        assert!(config.providers.is_empty());
    }

    #[test]
    fn test_build_decoding_key_missing_rsa_components() {
        let jwk = JwksKey {
            kty: "RSA".to_string(),
            kid: None,
            key_use: None,
            alg: None,
            n: None,
            e: None,
            x: None,
            y: None,
            crv: None,
        };
        let result = build_decoding_key(&jwk, &Algorithm::RS256);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_decoding_key_missing_ec_components() {
        let jwk = JwksKey {
            kty: "EC".to_string(),
            kid: None,
            key_use: None,
            alg: None,
            n: None,
            e: None,
            x: None,
            y: None,
            crv: None,
        };
        let result = build_decoding_key(&jwk, &Algorithm::ES256);
        assert!(result.is_err());
    }

    #[test]
    fn test_unsupported_algorithm() {
        let jwk = JwksKey {
            kty: "oct".to_string(),
            kid: None,
            key_use: None,
            alg: None,
            n: None,
            e: None,
            x: None,
            y: None,
            crv: None,
        };
        let result = build_decoding_key(&jwk, &Algorithm::HS256);
        assert!(result.is_err());
        assert!(result.err().unwrap().contains("Unsupported"));
    }

    // ── State token tests ───────────────────────────────────────────────

    #[test]
    fn test_build_and_verify_state_token() {
        let token = build_state_token("google", None);
        let payload = verify_state_token(&token).unwrap();
        assert_eq!(payload.provider, "google");
        assert_eq!(payload.account_id, None);
        assert!(!payload.nonce.is_empty());
    }

    #[test]
    fn test_build_and_verify_state_token_with_account_id() {
        let token = build_state_token("google", Some("tenant-a"));
        let payload = verify_state_token(&token).unwrap();
        assert_eq!(payload.provider, "google");
        assert_eq!(payload.account_id.as_deref(), Some("tenant-a"));
        assert!(!payload.nonce.is_empty());
    }

    #[test]
    fn test_state_token_rejects_tampered_payload() {
        let token = build_state_token("google", None);
        // Tamper with the payload part.
        let parts: Vec<&str> = token.splitn(2, '.').collect();
        let tampered = format!("{}.{}", "dGFtcGVyZWQ", parts[1]);
        assert!(verify_state_token(&tampered).is_err());
    }

    #[test]
    fn test_state_token_rejects_missing_signature() {
        assert!(verify_state_token("just-payload-no-dot").is_err());
    }

    #[test]
    fn test_state_token_rejects_expired() {
        // Build a token with an old timestamp.
        let payload = OAuthStatePayload {
            provider: "test".to_string(),
            account_id: None,
            nonce: "nonce".to_string(),
            ts: 0, // epoch = very expired
        };
        let payload_json = serde_json::to_string(&payload).unwrap();
        let payload_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        let key = state_signing_key();
        let mut mac = HmacSha256::new_from_slice(key.as_bytes()).unwrap();
        mac.update(payload_b64.as_bytes());
        let sig = mac.finalize().into_bytes();
        let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
        let token = format!("{payload_b64}.{sig_b64}");

        let result = verify_state_token(&token);
        assert!(result.is_err());
        assert!(result.err().unwrap().contains("expired"));
    }

    #[tokio::test]
    async fn test_handle_code_exchange_rejects_tenant_mismatch() {
        let ext_auth = librefang_types::config::ExternalAuthConfig::default();
        let state_payload = OAuthStatePayload {
            provider: "google".to_string(),
            account_id: Some("tenant-a".to_string()),
            nonce: "nonce".to_string(),
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        let response =
            handle_code_exchange(&ext_auth, true, "code", &state_payload, Some("tenant-b")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_require_account_bound_oauth_rejects_missing_account_in_multi_tenant() {
        let err = require_account_bound_oauth(true, None).unwrap_err();
        assert!(err.contains("X-Account-Id"));
    }

    #[test]
    fn test_validate_callback_tenant_binding_rejects_missing_callback_account_in_multi_tenant() {
        let err = validate_callback_tenant_binding(true, Some("tenant-a"), None).unwrap_err();
        assert_eq!(err, "OAuth tenant mismatch");
    }

    #[test]
    fn test_validate_callback_tenant_binding_allows_matching_callback_account_in_multi_tenant() {
        assert!(validate_callback_tenant_binding(true, Some("tenant-a"), Some("tenant-a")).is_ok());
    }

    #[tokio::test]
    async fn test_token_store_is_scoped_by_account_id() {
        let sub = format!("subject-{}", uuid::Uuid::new_v4());
        let tenant_a = "tenant-a";
        let tenant_b = "tenant-b";

        TOKEN_STORE
            .store(
                &sub,
                Some(tenant_a),
                StoredTokens {
                    access_token: "access-a".to_string(),
                    refresh_token: Some("refresh-a".to_string()),
                    expires_at: None,
                    provider_id: "google".to_string(),
                    account_id: Some(tenant_a.to_string()),
                    stored_at: std::time::Instant::now(),
                },
            )
            .await;
        TOKEN_STORE
            .store(
                &sub,
                Some(tenant_b),
                StoredTokens {
                    access_token: "access-b".to_string(),
                    refresh_token: Some("refresh-b".to_string()),
                    expires_at: None,
                    provider_id: "google".to_string(),
                    account_id: Some(tenant_b.to_string()),
                    stored_at: std::time::Instant::now(),
                },
            )
            .await;

        let stored_a = TOKEN_STORE
            .find_by_provider("google", Some(tenant_a))
            .await
            .expect("tenant-a entry");
        let stored_b = TOKEN_STORE
            .find_by_provider("google", Some(tenant_b))
            .await
            .expect("tenant-b entry");

        assert_eq!(stored_a.0, sub);
        assert_eq!(stored_a.1.refresh_token.as_deref(), Some("refresh-a"));
        assert_eq!(stored_b.0, sub);
        assert_eq!(stored_b.1.refresh_token.as_deref(), Some("refresh-b"));

        TOKEN_STORE.remove(&sub, Some(tenant_a)).await;
        TOKEN_STORE.remove(&sub, Some(tenant_b)).await;
    }

    // ── resolve_providers tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_resolve_providers_empty_config() {
        let config = librefang_types::config::ExternalAuthConfig::default();
        let providers = resolve_providers(&config).await;
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_providers_explicit_urls() {
        let config = librefang_types::config::ExternalAuthConfig {
            enabled: true,
            providers: vec![librefang_types::config::OidcProvider {
                id: "github".to_string(),
                display_name: "GitHub".to_string(),
                issuer_url: String::new(),
                auth_url: "https://github.com/login/oauth/authorize".to_string(),
                token_url: "https://github.com/login/oauth/access_token".to_string(),
                userinfo_url: "https://api.github.com/user".to_string(),
                jwks_uri: String::new(),
                client_id: "test-client".to_string(),
                client_secret_env: "GH_SECRET".to_string(),
                redirect_url: "http://localhost/callback".to_string(),
                scopes: vec!["read:user".to_string()],
                allowed_domains: vec![],
                audience: String::new(),
            }],
            ..Default::default()
        };
        let providers = resolve_providers(&config).await;
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "github");
        assert_eq!(
            providers[0].auth_url,
            "https://github.com/login/oauth/authorize"
        );
    }

    #[tokio::test]
    async fn test_resolve_providers_discovery_failure_does_not_panic() {
        // Provider with an issuer_url that will fail discovery (no server).
        let config = librefang_types::config::ExternalAuthConfig {
            enabled: true,
            providers: vec![librefang_types::config::OidcProvider {
                id: "bad".to_string(),
                display_name: "Bad".to_string(),
                issuer_url: "http://127.0.0.1:1/nonexistent".to_string(),
                auth_url: String::new(),
                token_url: String::new(),
                userinfo_url: String::new(),
                jwks_uri: String::new(),
                client_id: "test".to_string(),
                client_secret_env: "SECRET".to_string(),
                redirect_url: "http://localhost/callback".to_string(),
                scopes: vec!["openid".to_string()],
                allowed_domains: vec![],
                audience: String::new(),
            }],
            ..Default::default()
        };
        let providers = resolve_providers(&config).await;
        // Should return empty (discovery failed) without panicking.
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_providers_legacy_fallback_no_issuer() {
        // Legacy config with client_id but no issuer_url — should not resolve.
        let config = librefang_types::config::ExternalAuthConfig {
            enabled: true,
            client_id: "legacy-client".to_string(),
            issuer_url: String::new(),
            ..Default::default()
        };
        let providers = resolve_providers(&config).await;
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_providers_multi_provider_mixed() {
        // One provider with explicit URLs (succeeds) and one with bad issuer (fails).
        let config = librefang_types::config::ExternalAuthConfig {
            enabled: true,
            providers: vec![
                librefang_types::config::OidcProvider {
                    id: "good".to_string(),
                    display_name: "Good".to_string(),
                    issuer_url: String::new(),
                    auth_url: "https://auth.example.com/authorize".to_string(),
                    token_url: "https://auth.example.com/token".to_string(),
                    userinfo_url: String::new(),
                    jwks_uri: String::new(),
                    client_id: "good-client".to_string(),
                    client_secret_env: "GOOD_SECRET".to_string(),
                    redirect_url: "http://localhost/callback".to_string(),
                    scopes: vec!["openid".to_string()],
                    allowed_domains: vec![],
                    audience: String::new(),
                },
                librefang_types::config::OidcProvider {
                    id: "bad".to_string(),
                    display_name: "Bad".to_string(),
                    issuer_url: "http://127.0.0.1:1/nonexistent".to_string(),
                    auth_url: String::new(),
                    token_url: String::new(),
                    userinfo_url: String::new(),
                    jwks_uri: String::new(),
                    client_id: "bad-client".to_string(),
                    client_secret_env: "BAD_SECRET".to_string(),
                    redirect_url: "http://localhost/callback".to_string(),
                    scopes: vec!["openid".to_string()],
                    allowed_domains: vec![],
                    audience: String::new(),
                },
            ],
            ..Default::default()
        };
        let providers = resolve_providers(&config).await;
        // Only the explicit-URL provider should succeed.
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "good");
    }
}
