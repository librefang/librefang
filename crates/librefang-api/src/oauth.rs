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
    ///
    /// SECURITY (#5128): `sub` is the primary key the daemon uses to bind
    /// stored tokens to a user (`TOKEN_STORE.store(&claims.sub, …)`). When
    /// this field was `#[serde(default)]`, a JWT missing the `sub` claim
    /// deserialised with `sub = ""` and every token-less login collided on
    /// the same slot — the apparent "successful login" actually carried
    /// another user's refresh token. The field is now mandatory at the
    /// serde layer, `set_required_spec_claims` enforces it at the JWT
    /// layer, and `validate_jwt_cached` rejects an explicit empty string
    /// as a defence-in-depth third gate.
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
    /// Groups the identity provider asserts this user belongs to (#7746).
    ///
    /// The standard-ish flat spelling, emitted by Okta, Authentik, Google
    /// Workspace and Entra (as object GUIDs rather than display names). It is a
    /// named field rather than being left to `extra` because it is one of the
    /// two default entries in `[external_auth] claim_paths` and typing it here
    /// keeps the common case out of the generic path resolver.
    #[serde(default)]
    pub groups: Vec<String>,
    /// OAuth2 `scope`, space-delimited per RFC 6749 (#7746).
    ///
    /// Parsed but not consulted by default: `scope` reaches `role_map` /
    /// `group_map` only when an operator names it in `[external_auth]
    /// claim_paths`. See that field for why it is held to a different
    /// standard than `roles` and `groups`.
    #[serde(default)]
    pub scope: Option<String>,
    /// Every other claim the token carried, kept so a dotted
    /// `[external_auth] claim_paths` entry can address a nested one — Keycloak's
    /// `realm_access.roles` and `resource_access.<client>.roles` have no flat
    /// spelling and cannot be reached any other way (#7746).
    ///
    /// Nothing serializes `IdTokenClaims` wholesale: `auth_userinfo` and
    /// `auth_introspect` both build their response objects field by field, so
    /// flattening the passthrough here does not widen either response.
    #[serde(flatten, default)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
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

// ── Cache invalidation (hot-reload) ─────────────────────────────────────

/// Drop every cached JWKS keyset and OIDC discovery document.
///
/// Called from the kernel hot-reload pipeline (via the
/// [`librefang_kernel::oauth_cache_invalidator::OauthCacheInvalidator`]
/// trait) whenever `[external_auth]` is reloaded with a new
/// identity-provider identity (issuer URL, JWKS URI, providers list).
/// Without this, swapping IdPs at runtime would leave the previous
/// provider's signing keys in cache until the natural 1h TTL —
/// tokens issued by the new IdP would fail JWT validation against the
/// stale keys (`No JWKS key found for kid=…`) until the entry expires.
///
/// Idempotent: clearing an already-empty cache is a no-op. Synchronous
/// from the caller's perspective: each `clear()` only briefly takes
/// the per-cache write lock to drop entries (no network I/O).
pub fn invalidate_oauth_caches() {
    // The caches are guarded by `tokio::sync::RwLock` because the
    // fetch path holds the write guard across an `.await` on the HTTP
    // round-trip. Invalidation only needs synchronous access; spawn a
    // detached task when we're on a runtime, fall back to a
    // single-threaded runtime + block_on when not (only hit from
    // synchronous tests).
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // Detached drop on the runtime — kernel `apply_hot_actions_inner`
            // already holds the config-reload write lock, so we don't
            // need the clear to complete synchronously: any subsequent
            // login attempt re-takes the cache lock and serialises
            // naturally with this task.
            let h2 = handle.clone();
            handle.spawn(async {
                JWKS_CACHE.inner.write().await.clear();
            });
            h2.spawn(async {
                DISCOVERY_CACHE.inner.write().await.clear();
            });
        }
        Err(_) => {
            // No tokio runtime — build a single-threaded one for the
            // clear and tear it down. Only exercised by synchronous
            // unit tests; the production path always has a runtime.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("transient tokio runtime for cache invalidation");
            rt.block_on(async {
                JWKS_CACHE.inner.write().await.clear();
                DISCOVERY_CACHE.inner.write().await.clear();
            });
        }
    }
}

/// Adapter that implements the kernel-facing
/// [`librefang_kernel::oauth_cache_invalidator::OauthCacheInvalidator`]
/// trait. Constructed once at API-server boot and handed to the kernel
/// via `set_oauth_cache_invalidator`.
pub struct OauthCacheInvalidatorImpl;

impl librefang_kernel::oauth_cache_invalidator::OauthCacheInvalidator
    for OauthCacheInvalidatorImpl
{
    fn invalidate(&self) {
        invalidate_oauth_caches();
    }
}

// ── State (CSRF) ────────────────────────────────────────────────────────

/// State parameter payload encoded as JSON and HMAC-signed.
/// Encodes the provider ID and a nonce so the callback can route correctly
/// and validate against CSRF.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthStatePayload {
    /// Provider ID (e.g. "google", "github").
    provider: String,
    /// Random nonce for CSRF protection.
    nonce: String,
    /// Timestamp (seconds since UNIX epoch) for expiry checking.
    ts: u64,
}

/// State token TTL — 10 minutes. Login flows should complete quickly.
const STATE_TOKEN_TTL_SECS: u64 = 600;

/// Build an HMAC-signed state parameter containing provider + nonce.
fn build_state_token(provider_id: &str) -> String {
    let nonce = uuid::Uuid::new_v4().to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let payload = OAuthStatePayload {
        provider: provider_id.to_string(),
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
fn state_signing_key() -> String {
    static KEY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        std::env::var("LIBREFANG_STATE_SECRET").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string())
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
    /// Whether to require `email_verified: true` in the ID token / userinfo
    /// response before allowing login.  Defaults to `true` (#3703).
    #[serde(skip)]
    pub require_email_verified: bool,
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
    /// Load-bearing since #6629: `find_by_access_token` matches on this to prove a refresh caller owns the session it is refreshing, so it is no longer dead code.
    access_token: String,
    /// Optional refresh token for obtaining new access tokens.
    refresh_token: Option<String>,
    /// When the access token expires (absolute time).
    #[allow(dead_code)]
    expires_at: Option<std::time::Instant>,
    /// Provider ID that issued these tokens.
    provider_id: String,
    /// When this entry was stored (for TTL eviction).
    stored_at: std::time::Instant,
}

/// Token store entries older than 24 hours are evicted on access.
const TOKEN_STORE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// In-memory token store. Maps user `sub` to their stored tokens.
#[derive(Default)]
pub struct TokenStore {
    inner: RwLock<HashMap<String, StoredTokens>>,
}

/// Global token store instance.
static TOKEN_STORE: std::sync::LazyLock<TokenStore> = std::sync::LazyLock::new(TokenStore::default);

impl TokenStore {
    /// Store tokens for a user.
    async fn store(&self, sub: &str, tokens: StoredTokens) {
        let mut write = self.inner.write().await;
        write.insert(sub.to_string(), tokens);
    }

    /// Retrieve stored tokens for a user, evicting if older than TTL.
    #[allow(dead_code)]
    async fn get(&self, sub: &str) -> Option<StoredTokens> {
        let mut write = self.inner.write().await;
        if let Some(entry) = write.get(sub) {
            if entry.stored_at.elapsed() > TOKEN_STORE_TTL {
                debug!(sub = %sub, "Evicting expired token store entry (>24h)");
                write.remove(sub);
                return None;
            }
            return Some(entry.clone());
        }
        None
    }

    /// Remove stored tokens for a user (e.g., on logout).
    #[allow(dead_code)]
    async fn remove(&self, sub: &str) {
        let mut write = self.inner.write().await;
        write.remove(sub);
    }

    /// Find the stored entry whose access token the caller presented (#6629).
    ///
    /// This replaced `find_by_provider` and `find_any_with_refresh`, which selected *an* entry matching a provider — or literally any entry with a refresh token — without checking that it belonged to the caller.
    /// Since the route is reachable by any Admin, that let one caller refresh another local user's upstream session and receive their credentials, with every scope that token had been granted.
    ///
    /// Ownership is proven by presenting the access token the callback returned to that client.
    /// There is no lesser predicate available here: the store is keyed by upstream OIDC subject with no record of which local user owns an entry, and a caller-supplied `sub` or `provider` is an assertion rather than proof.
    ///
    /// Comparison is constant-time.
    /// A short-circuiting `==` over a `HashMap` scan leaks how many leading bytes matched, which for a remotely-guessable-in-principle credential is a distinguisher worth denying; `subtle::ConstantTimeEq` costs nothing at these lengths.
    ///
    /// Returns the refresh token as a non-optional `String` so callers cannot reach for `.unwrap()` on `entry.refresh_token` (audit: `oauth-refresh-error-body-token-leak`, sub-finding "unwrap on refresh_token").
    /// The `Some(..)` arm is itself the proof that a refresh token is present — the invariant lives in the type, not in a comment.
    async fn find_by_access_token(
        &self,
        access_token: &str,
    ) -> Option<(String, String, StoredTokens)> {
        use subtle::ConstantTimeEq;

        // An empty needle must never match, and the length check below cannot save us: `StoredTokens::access_token` is taken verbatim from the provider's token response, so a provider that returned an empty string would leave an entry whose access token is `""` — and `{"access_token": ""}` would then compare equal to it and hand back that session's refresh token to a caller who proved nothing.
        // Fail closed on the empty needle rather than relying on every IdP to be well-behaved.
        // The handler rejects blank input too (#6629); this is the invariant at the primitive, so a future caller inherits it.
        if access_token.is_empty() {
            return None;
        }

        let mut write = self.inner.write().await;
        let now = std::time::Instant::now();

        // Evict expired entries.
        write.retain(|_sub, entry| now.duration_since(entry.stored_at) <= TOKEN_STORE_TTL);

        write.iter().find_map(|(sub, entry)| {
            // Length differs → cannot match.
            // `ct_eq` requires equal lengths anyway, and the length of an IdP-issued token is not a secret.
            if entry.access_token.len() != access_token.len() {
                return None;
            }
            if !bool::from(entry.access_token.as_bytes().ct_eq(access_token.as_bytes())) {
                return None;
            }
            entry
                .refresh_token
                .clone()
                .map(|rt| (sub.clone(), rt, entry.clone()))
        })
    }
}

// ── Route: GET /api/auth/providers ──────────────────────────────────────

/// GET /api/auth/providers — List available authentication providers.
///
/// Gated by `require_auth_for_reads` at the middleware layer (it lives in
/// `PUBLIC_ROUTES_DASHBOARD_READS`): in strict mode an unauthenticated caller
/// gets 401 before reaching this handler. In open mode the route is reachable
/// without a token.
///
/// The response is intentionally names-only (`id` + `display_name`) for **every**
/// caller — the minimum the login UI needs to render its provider buttons. The
/// per-provider OAuth `scopes` (and any other configuration detail) are never
/// returned here, so the IdP scope configuration cannot be enumerated through
/// this endpoint regardless of the caller's privilege. (An earlier revision
/// keyed `scopes` exposure on the `AuthenticatedApiUser` extension, but the
/// static `api_key` — the highest-privilege credential — is identity-less and
/// never carries that extension, so the gate both under-disclosed to the admin
/// key and inverted the privilege ordering. Scopes aren't security-sensitive,
/// but they also aren't needed here; admins read full provider config from the
/// auth-gated `/api/config` surface instead.)
#[utoipa::path(get, path = "/api/auth/providers", tag = "auth", responses((status = 200, description = "List configured OAuth/OIDC providers", body = crate::types::JsonObject)))]
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
            // Names-only for all callers: omit `scopes` (and any other
            // configuration detail) so the IdP scope set is never enumerable
            // through this endpoint.
            serde_json::json!({
                "id": p.id,
                "display_name": p.display_name,
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

    build_login_redirect(provider).into_response()
}

/// GET /api/auth/login/:provider — Redirect to a specific provider.
#[utoipa::path(get, path = "/api/auth/login/{provider}", tag = "auth", params(("provider" = String, Path, description = "OAuth provider name")), responses((status = 302, description = "Redirect to specific OAuth provider")))]
pub async fn auth_login_provider(
    State(state): State<Arc<AppState>>,
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

    build_login_redirect(provider).into_response()
}

/// Build the OAuth2 authorization redirect for the given provider.
/// Generates a signed state token encoding the provider ID and a nonce.
fn build_login_redirect(provider: &ResolvedProvider) -> impl IntoResponse {
    let state_token = build_state_token(&provider.id);
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
        Err(error) => {
            tracing::error!(
                provider = %provider.id,
                %error,
                "failed to build OAuth authorization URL"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to build authorization URL"})),
            )
                .into_response()
        }
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
#[utoipa::path(get, path = "/api/auth/callback", tag = "auth", responses((status = 200, description = "OAuth callback — completes login flow", body = crate::types::JsonObject)))]
pub async fn auth_callback(
    State(state): State<Arc<AppState>>,
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

    if let Err(resp) = consume_oauth_nonce(&state, &state_payload.nonce) {
        return resp;
    }

    handle_code_exchange(ext_auth, &code, &state_payload).await
}

/// POST /api/auth/callback — Handle the OAuth2 callback (programmatic clients).
#[utoipa::path(post, path = "/api/auth/callback", tag = "auth", responses((status = 200, description = "OAuth callback (POST) — completes login flow", body = crate::types::JsonObject)))]
pub async fn auth_callback_post(
    State(state): State<Arc<AppState>>,
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

    if let Err(resp) = consume_oauth_nonce(&state, &state_payload.nonce) {
        return resp;
    }

    handle_code_exchange(ext_auth, &body.code, &state_payload).await
}

/// Atomically reject + consume an OAuth state nonce.
///
/// #3944 verified that the nonce in the id_token matched the one we
/// signed into `state`, but never marked the nonce as redeemed.  A
/// callback URL captured from browser history, Referer, or proxy logs
/// could be replayed against the daemon repeatedly until the IdP
/// rejected the authorization code.  This helper enforces single-use
/// at the daemon by checking + recording the nonce as consumed before
/// the code exchange runs.  Subsequent requests with the same `state`
/// are rejected with HTTP 400.
///
/// The nonce is consumed eagerly (before code exchange).  Failed
/// downstream verification (token-endpoint reject, JWT signature fail)
/// still leaves the nonce marked used — the legitimate user must
/// restart the auth flow if anything goes wrong, which is exactly the
/// fail-closed shape we want for credential flows.
//
// `axum::http::Response<Body>` is ~128 bytes, which trips clippy's
// `result_large_err` lint.  The whole point of this helper is to
// short-circuit the callback handler with a fully-formed Response
// when the nonce was already redeemed — boxing the Err just to
// satisfy the lint would force every caller to `.map_err(|b| *b)`
// at the call site for no real benefit (the helper isn't on a hot
// path; one allocation per OAuth callback is fine, and the Err
// path is the rare-branch).  Suppress the lint here.
#[allow(clippy::result_large_err)]
fn consume_oauth_nonce(state: &Arc<AppState>, nonce: &str) -> Result<(), Response> {
    if state.kernel.approvals().is_oauth_nonce_used(nonce) {
        warn!("OIDC nonce replay rejected (state.nonce already redeemed)");
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "OAuth callback already redeemed; please restart the sign-in flow"
            })),
        )
            .into_response());
    }
    state.kernel.approvals().record_oauth_nonce_used(nonce);
    Ok(())
}

/// Outcome of validating the `nonce` claim in a JWT id_token against the
/// nonce the relying party signed into the OAuth `state` parameter.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NonceCheck {
    /// id_token nonce matches the state nonce.
    Ok,
    /// id_token nonce is present but differs from the state nonce.
    Mismatch,
    /// id_token has no nonce claim — must reject (#3364).
    Missing,
}

/// OIDC nonce validation. The relying party always includes a `nonce` in
/// the auth request; the IDP MUST echo it back in the id_token. A missing
/// claim is rejected, never silently accepted (which would let an attacker
/// replay an id_token captured from another login session).
pub(crate) fn check_id_token_nonce(token_nonce: Option<&str>, state_nonce: &str) -> NonceCheck {
    match token_nonce {
        Some(n) if n == state_nonce => NonceCheck::Ok,
        Some(_) => NonceCheck::Mismatch,
        None => NonceCheck::Missing,
    }
}

/// Shared code exchange logic for both GET and POST callback handlers.
async fn handle_code_exchange(
    ext_auth: &librefang_types::config::ExternalAuthConfig,
    code: &str,
    state_payload: &OAuthStatePayload,
) -> Response {
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

    // Validate the ID token if the provider supplies one.  Three rules,
    // all hard rejects:
    //   1. Provider sent an id_token AND we have a jwks_uri to verify it
    //      against → JWT validation MUST succeed.  A malformed / forged /
    //      expired id_token is a strong signal of replay or token swap;
    //      falling through to userinfo (which carries no nonce) lets
    //      the attack succeed.  This was the path #3944 left open.
    //   2. id_token validates → nonce claim MUST be present and equal
    //      to the nonce we signed into `state`.
    //   3. id_token validates with mismatched nonce → reject.
    //
    // The "no id_token at all" path (some OAuth2 providers genuinely
    // don't emit one for non-OIDC flows) still falls through to
    // userinfo by design; that path was always nonce-less.
    let claims = if let Some(ref id_token) = token_resp.id_token {
        if !id_token.is_empty() && !provider.jwks_uri.is_empty() {
            match validate_jwt_cached(id_token, &provider.jwks_uri, &provider.audience).await {
                Ok(c) => {
                    // Verify nonce claim against the nonce we sent in the auth request.
                    match check_id_token_nonce(c.nonce.as_deref(), &state_payload.nonce) {
                        NonceCheck::Ok => {}
                        NonceCheck::Mismatch => {
                            warn!("Nonce mismatch in ID token");
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({"error": "Nonce mismatch in ID token"})),
                            )
                                .into_response();
                        }
                        NonceCheck::Missing => {
                            // #3364: OIDC requires the IDP echo the nonce we sent.
                            // Falling back to userinfo here lets a replayed id_token
                            // from a different login session sign in as the captured user.
                            warn!(
                                "ID token is missing the nonce claim — \
                                 rejecting to prevent nonce-bypass attack"
                            );
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({
                                    "error": "ID token missing required nonce claim"
                                })),
                            )
                                .into_response();
                        }
                    }
                    Some(c)
                }
                Err(e) => {
                    // The provider sent an id_token AND we had keys to
                    // verify it — verification must succeed or the
                    // request is rejected.  Returning the error message
                    // as-is is fine: it's our own validation diagnostic
                    // (kid mismatch, expired, sig fail), not provider
                    // body.
                    warn!(error = %e, "ID token validation failed — rejecting OAuth callback");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "ID token signature or expiry validation failed"
                        })),
                    )
                        .into_response();
                }
            }
        } else {
            // Provider sent a non-empty id_token but no jwks_uri configured
            // for this provider, OR the id_token field came back empty.
            // The empty-token case is benign (no token to verify); the
            // missing-jwks-uri case means this provider was added to the
            // config without OIDC keys, which only makes sense for pure
            // OAuth2 — accept and rely on userinfo.
            if !id_token.is_empty() {
                warn!(
                    "Provider supplied id_token but jwks_uri is unset; \
                     skipping JWT validation and falling back to userinfo. \
                     Configure jwks_uri to enforce OIDC nonce binding."
                );
            }
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
                        // #7746: the userinfo document is the only identity
                        // statement a pure-OAuth2 provider makes, and hardcoding
                        // these empty meant an operator's `role_map` / `group_map`
                        // silently never matched for exactly those providers.
                        roles: string_list_claim(&info["roles"]),
                        groups: string_list_claim(&info["groups"]),
                        scope: info["scope"].as_str().map(|s| s.to_string()),
                        extra: userinfo_passthrough(&info),
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

    // SECURITY (#3703): Require email_verified = true before allowing login.
    // Without this check, a provider that supports unverified email addresses
    // can be exploited to claim an address in `allowed_domains` without actually
    // owning that address.
    if provider.require_email_verified {
        match claims.email_verified {
            Some(true) => {} // verified — allow login to proceed
            Some(false) => {
                warn!(
                    sub = %claims.sub,
                    provider = %provider.id,
                    "OIDC login rejected: email_verified = false"
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Email address not verified by identity provider"
                    })),
                )
                    .into_response();
            }
            None => {
                warn!(
                    sub = %claims.sub,
                    provider = %provider.id,
                    "OIDC login rejected: email_verified claim absent"
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Email address not verified by identity provider"
                    })),
                )
                    .into_response();
            }
        }
    }

    // Check allowed domains.
    if !provider.allowed_domains.is_empty() {
        if let Some(ref email) = claims.email {
            let domain = email_domain(email);
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

    // SECURITY (#5128): Refuse to bind a session to an empty `sub`. JWT
    // validation in `validate_jwt_cached` already rejects this case, but
    // the userinfo-fallback branch above synthesises `sub` from
    // `info["sub"].or(info["id"]).unwrap_or("")` — a provider that omits
    // both fields would otherwise land here with an empty primary key
    // and silently collide every token-less login on the same slot in
    // `TOKEN_STORE`.
    if claims.sub.is_empty() {
        warn!(
            provider = %provider.id,
            "External auth login rejected: empty `sub` after claim resolution"
        );
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "Identity provider did not return a usable subject identifier"
            })),
        )
            .into_response();
    }

    // SECURITY: log only the email domain at INFO — full email is PII and
    // production INFO logs typically ship to aggregators with longer retention
    // than DEBUG. The domain alone preserves the diagnostic value (which IdP
    // tenant signed in) without leaking the user identifier.
    info!(
        sub = %claims.sub,
        domain = %claims.email.as_deref().map(email_domain).unwrap_or(""),
        provider = %provider.id,
        "External auth login successful"
    );

    let expires_in = token_resp.expires_in.unwrap_or(ext_auth.session_ttl_secs);

    // Store tokens so we can refresh later when the access token expires.
    let expires_at = Some(std::time::Instant::now() + std::time::Duration::from_secs(expires_in));
    TOKEN_STORE
        .store(
            &claims.sub,
            StoredTokens {
                access_token: token_resp.access_token.clone(),
                refresh_token: token_resp.refresh_token.clone(),
                expires_at,
                provider_id: provider.id.clone(),
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
#[utoipa::path(get, path = "/api/auth/userinfo", tag = "auth", responses((status = 200, description = "Get authenticated user info", body = crate::types::JsonObject), (status = 401, description = "Not authenticated")))]
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
#[utoipa::path(post, path = "/api/auth/introspect", tag = "auth", request_body = crate::types::JsonObject, responses((status = 200, description = "Token introspection result", body = crate::types::JsonObject)))]
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
    ///
    /// When omitted, `access_token` must be supplied instead — the server no longer searches the token store for "some" stored session (#6629).
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// The access token this caller received from its own login callback, proving which stored session it owns (#6629).
    ///
    /// **Prefer `refresh_token` when the client still has it.**
    /// `/api/auth/callback` returns both values — see the `SECURITY` note on `CallbackResponse::refresh_token`, which hands the refresh token to the client deliberately — so this field is a convenience for a client that kept only the access token it authenticates with, not the only credential a legitimate caller can present.
    /// Presenting it resolves the matching entry without letting the caller name someone else's session: the value is high-entropy, issued by the identity provider, and never disclosed by any read route.
    ///
    /// The trade this accepts: an access token travels on every request as a `Bearer` header, so it is the more exposed of the two credentials, and exchanging one for a refresh token extends a leaked short-lived credential past its own lifetime.
    /// It is bounded by the store's 24 h TTL and does not widen what a holder of that access token can already do while it is live, which is why the path is offered at all — but a client that holds its refresh token should send that instead.
    ///
    /// An expired access token still matches — entries live in the store for 24 h regardless of the access token's own lifetime, which is exactly the state a caller is in when it needs to refresh.
    #[serde(default)]
    pub access_token: Option<String>,
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
/// When the access token expires, clients call this instead of forcing a full re-authorization, presenting either:
///
/// * `refresh_token` — the one `/api/auth/callback` returned to it, which is the preferred path, or
/// * `access_token` — also returned by that callback, identifying the stored session whose server-side refresh token should be used, for a client that kept only this half.
///
/// There is no third path.
/// A blank string in either field counts as absent, so it reaches neither the store nor the provider.
/// The endpoint used to fall back to scanning the token store for any entry matching a `provider` hint, or for literally any entry with a refresh token, and it is reachable by any Admin — so a caller could refresh a *different* local user's upstream session and be handed their credentials, with every scope that token carried.
/// Both fallbacks are gone (#6629); a request that proves nothing gets a 400.
#[utoipa::path(post, path = "/api/auth/refresh", tag = "auth", request_body = RefreshRequest, responses((status = 200, description = "New access token", body = crate::types::JsonObject), (status = 400, description = "Missing or invalid refresh token"), (status = 502, description = "Token refresh failed")))]
pub async fn auth_refresh(
    State(state): State<Arc<AppState>>,
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

    let providers = resolve_providers(ext_auth).await;

    // A field present but blank proves nothing, so treat it as absent instead of letting it select a branch (#6629).
    // Left as `Some("")`, an empty `refresh_token` would fan out a doomed request to the provider's token endpoint, and an empty `access_token` would reach the store scan — where it is rejected, but the primitive should not be the only thing standing between a blank body and a credential lookup.
    // Filtering on the trimmed form while passing the value through untrimmed keeps the comparison an exact match against what the provider issued.
    let supplied_refresh = req
        .refresh_token
        .as_deref()
        .filter(|rt| !rt.trim().is_empty());
    let supplied_access = req
        .access_token
        .as_deref()
        .filter(|at| !at.trim().is_empty());

    // Resolve the refresh token from whichever credential the caller proved.
    let (refresh_token, stored_sub, provider) = if let Some(rt) = supplied_refresh {
        // Client supplied a refresh token explicitly.
        let provider = if let Some(ref pid) = req.provider {
            providers.iter().find(|p| p.id == *pid)
        } else if providers.len() == 1 {
            providers.first()
        } else {
            None
        };
        (rt.to_string(), None::<String>, provider.cloned())
    } else if let Some(at) = supplied_access {
        // No refresh token, but the caller presented the access token it was issued — resolve the entry that token belongs to, and only that one (#6629).
        // The pre-fix code took a `provider` hint (or nothing at all) and selected an arbitrary matching entry, so an Admin could refresh another local user's upstream session and receive their credentials.
        match TOKEN_STORE.find_by_access_token(at).await {
            Some((sub, refresh_token, entry)) => {
                let provider = providers
                    .iter()
                    .find(|p| p.id == entry.provider_id)
                    .cloned();
                (refresh_token, Some(sub), provider)
            }
            None => {
                // Deliberately one message for "no such session", "that session has no refresh token", and "it expired out of the store".
                // Distinguishing them would turn this route into an oracle for which access tokens the daemon has seen.
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "No refreshable session matches the supplied access_token"
                    })),
                )
                    .into_response();
            }
        }
    } else {
        // Neither credential supplied, or both blank.
        // There is deliberately no fallback: any store lookup that is not keyed on something the caller proved it owns hands out someone else's tokens (#6629).
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Provide either 'refresh_token', or the 'access_token' \
                          issued to this session so the server can identify it"
            })),
        )
            .into_response();
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
                StoredTokens {
                    access_token: token_resp.access_token.clone(),
                    refresh_token: token_resp.refresh_token.clone(),
                    expires_at,
                    provider_id: provider.id.clone(),
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
///
/// # The `roles` claim (#7744)
///
/// `IdTokenClaims` has parsed a `roles: Vec<String>` claim since the type was written, and until now every downstream handler ignored the extension entirely, so a validated claim about what the caller is allowed to do reached no authorization decision at all.
/// This middleware now also resolves those claims through `[external_auth.role_map]` and injects a [`crate::middleware::OidcRoleGrant`] when they map to a LibreFang role.
/// Resolution happens here rather than in [`crate::middleware::auth`] because this is the layer that holds both the validated claims and the `ResolvedProvider` the token authenticated against — the provider is what decides whether the token was audience-bound and whether its email was verified, and neither fact survives into the claims struct.
///
/// The grant is strictly additive. It is injected only when an operator wrote a `role_map`, and [`crate::middleware::auth`] consults it only where it was about to reject the request, so no request that succeeds today changes outcome or role.
/// Axum runs the last-added layer first, and `server.rs` adds `middleware::auth` after this one, which is what puts the grant in extensions before the credential path looks for it.
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
                        let domain = email_domain(email);
                        if !provider.allowed_domains.iter().any(|d| d == domain) {
                            // SECURITY: log only the domain — the full email is PII even at
                            // DEBUG. `domain` is already extracted just above for the check.
                            debug!(domain = %domain, "Email domain not in allowed list");
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
                // Resolve `[external_auth]` claim mappings before the claims are
                // moved into extensions. See `provider_grant_gates_pass` for the
                // two provider-level gates that have no representation in the
                // claims struct and therefore cannot be checked downstream, and
                // `identity_claim_values` for why one resolved set of claim
                // values feeds both maps (#7746).
                let claim_values =
                    identity_claim_values(&claims, &config.claim_paths, &provider.client_id);
                if let Some(grant) =
                    role_grant_from_claims(&claims, provider, &config.role_map, &claim_values)
                {
                    request.extensions_mut().insert(grant);
                }
                if let Some(membership) = group_membership_from_claims(
                    &claims,
                    provider,
                    &config.group_map,
                    &kcfg.groups,
                    &claim_values,
                ) {
                    request.extensions_mut().insert(membership);
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

/// The two provider-level conditions a validated token must also satisfy before any of its claims become a LibreFang grant (#7744, extended to group membership in #7746).
///
/// Neither has a representation in [`IdTokenClaims`], which is why the check lives at the provider loop and not anywhere downstream.
///
/// **The provider must be audience-bound.** `validate_jwt_cached` sets `validation.validate_aud = false` when `expected_audience` is empty, so with no audience configured *any* token signed by that issuer's JWKS validates — including one minted for an unrelated OAuth client in the same tenant.
/// That is tolerable while the claims are inert and not tolerable as a grant of privilege or membership, so an unbound provider grants nothing.
/// `resolve_single_provider` falls back to `client_id` when `audience` is unset, so this only bites a provider configured with neither.
///
/// **The email must be verified when the provider requires it.** `require_email_verified` is the #3703 mitigation, and the callback route enforces it before minting anything; the middleware path never did, because it had nothing to mint.
///
/// Neither condition rejects the request. An unverified or audience-unbound token is simply not a credential, and the caller falls through to whatever the rest of the auth chain makes of it; turning either into a 403 here would change the outcome of requests that authenticate by some *other* means and merely happen to carry a JWT.
fn provider_grant_gates_pass(
    claims: &IdTokenClaims,
    provider: &ResolvedProvider,
    what: &'static str,
) -> bool {
    if provider.audience.is_empty() {
        debug!(
            provider = %provider.id,
            grant = what,
            "external_auth claim mapping is configured but this provider has no audience \
             to bind tokens to; refusing to derive a grant from an unbound token"
        );
        return false;
    }
    if provider.require_email_verified && claims.email_verified != Some(true) {
        debug!(
            provider = %provider.id,
            grant = what,
            "refusing to derive a grant from a token whose email is unverified"
        );
        return false;
    }
    true
}

/// Every string value at `path` in `claims`, where `path` is a dotted path into the token (#7746).
///
/// The first segment is matched against the typed fields first (`roles`, `groups`, `scope`) and then against the flattened passthrough, so the common flat claims cost no JSON traversal and a provider that also emits them nested still resolves.
/// A path resolving to an array contributes each string element; a path resolving to a single string contributes its whitespace-separated words, which is what makes a space-delimited `scope` work without a second config knob.
/// Anything else — a missing key, a number, an object — contributes nothing and is not an error: providers differ in which claims they emit, and a claim this deployment does not use is the normal case.
fn resolve_claim_path(claims: &IdTokenClaims, path: &str) -> Vec<String> {
    let mut segments = path.split('.');
    let Some(head) = segments.next() else {
        return Vec::new();
    };
    let rest: Vec<&str> = segments.collect();
    if rest.is_empty() {
        match head {
            "roles" => return claims.roles.clone(),
            "groups" => return claims.groups.clone(),
            "scope" => {
                return claims
                    .scope
                    .as_deref()
                    .map(|s| s.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default()
            }
            _ => {}
        }
    }
    let Some(mut value) = claims.extra.get(head) else {
        return Vec::new();
    };
    for segment in rest {
        match value.get(segment) {
            Some(next) => value = next,
            None => return Vec::new(),
        }
    }
    flatten_claim_value(value)
}

/// Array of strings → its elements; single string → its whitespace-separated words; anything else → nothing.
fn flatten_claim_value(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect(),
        serde_json::Value::String(s) => s.split_whitespace().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// The identity attributes this token asserts, as one de-duplicated ordered set of claim values, resolved through `[external_auth] claim_paths` (#7746).
///
/// One vocabulary for both maps, deliberately. `[external_auth.role_map]` has always documented its key as a "role/group claim value" and only ever read `claims.roles`, so an operator who wrote a group name into it got silence; drawing both maps from the same resolved set makes the documented behaviour the real one and means an operator maps a claim value once, in whichever map confers the thing they want.
///
/// The literal `<client>` in a path is substituted with the provider's `client_id`, so `resource_access.<client>.roles` — Keycloak's per-client role location — is one config entry rather than one per provider.
///
/// `BTreeSet` so the result does not depend on claim ordering (the IdP's business, and not stable between logins) or on `claim_paths` ordering, which is the #3298 rule applied at the point where these values become a grant.
fn identity_claim_values(
    claims: &IdTokenClaims,
    claim_paths: &[String],
    client_id: &str,
) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for path in claim_paths {
        let path = if path.contains("<client>") {
            path.replace("<client>", client_id)
        } else {
            path.clone()
        };
        out.extend(resolve_claim_path(claims, &path));
    }
    out.into_iter().collect()
}

/// Resolve a validated token's claim values into the local `[[groups]]` the caller belongs to for this request, or `None` when they belong to none (#7746).
///
/// Independent of [`role_grant_from_claims`] rather than a field on it, because the two grants answer different questions and an operator may want either alone: `group_map` with no `role_map` is a deployment that uses SSO identity for ownership and channel-binding roles while keeping API privilege on local keys, and it must not be silently disabled by the absence of the other map.
///
/// The membership is inserted into request extensions and dies with the request. Nothing writes it to `[[groups]]` — see `ExternalAuthConfig::group_map` for why persisting IdP state into operator-owned config would make a revocation unremovable rather than propagating it.
fn group_membership_from_claims(
    claims: &IdTokenClaims,
    provider: &ResolvedProvider,
    group_map: &std::collections::BTreeMap<String, String>,
    declared: &[librefang_types::config::GroupConfig],
    claim_values: &[String],
) -> Option<crate::middleware::IdpGroupMembership> {
    if group_map.is_empty() {
        return None;
    }
    if !provider_grant_gates_pass(claims, provider, "group") {
        return None;
    }
    let groups = librefang_kernel::auth::translate_oidc_groups(group_map, declared, claim_values);
    if groups.is_empty() {
        return None;
    }
    debug!(
        provider = %provider.id,
        count = groups.len(),
        "OIDC claims resolved to local group membership via external_auth.group_map"
    );
    Some(crate::middleware::IdpGroupMembership { groups })
}

/// Every string in a userinfo field that is an array of strings, or the whitespace-separated words of one that is a string.
fn string_list_claim(value: &serde_json::Value) -> Vec<String> {
    flatten_claim_value(value)
}

/// The userinfo document minus the keys [`IdTokenClaims`] types explicitly, so `extra` carries the same passthrough set a flattened ID token would and a dotted `claim_paths` entry resolves identically on both paths.
fn userinfo_passthrough(
    info: &serde_json::Value,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    const TYPED: &[&str] = &[
        "sub",
        "email",
        "email_verified",
        "name",
        "picture",
        "roles",
        "groups",
        "scope",
        "iss",
        "aud",
        "iat",
        "exp",
        "nonce",
    ];
    info.as_object()
        .map(|map| {
            map.iter()
                .filter(|(k, _)| !TYPED.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve a validated ID token into a [`crate::middleware::OidcRoleGrant`], or `None` when the token authorizes nothing (#7744).
///
/// The two provider-level conditions that must also hold — audience binding and email verification — live in [`provider_grant_gates_pass`], which #7746 shares with the group-membership resolver so the two grants can never disagree about which tokens are trustworthy.
///
/// `claim_values` is the resolved identity-attribute set from [`identity_claim_values`], not `claims.roles`.
/// #7906 read `roles` alone while `role_map`'s documentation described its key as a "role/group claim value", so an operator who wrote a group name into the map got silence; drawing from the resolved set makes the documented behaviour real without widening what an identity provider can assert, since every value still has to appear in a map the operator wrote.
fn role_grant_from_claims(
    claims: &IdTokenClaims,
    provider: &ResolvedProvider,
    role_map: &std::collections::BTreeMap<String, String>,
    claim_values: &[String],
) -> Option<crate::middleware::OidcRoleGrant> {
    if role_map.is_empty() {
        return None;
    }
    if !provider_grant_gates_pass(claims, provider, "role") {
        return None;
    }
    let role = librefang_kernel::auth::translate_oidc_roles(role_map, claim_values)?;
    // `email` is the operator-recognisable identity and the one `[[users]]`
    // entries are normally named after; `sub` is the fallback for providers
    // that issue no email claim. Either way the id is `UserId::from_name`, the
    // same derivation every other credential path uses, so an OIDC caller and
    // a declared user of the same name are one principal rather than two.
    let name = claims
        .email
        .clone()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| claims.sub.clone());
    let user_id = librefang_types::agent::UserId::from_name(&name);
    debug!(
        provider = %provider.id,
        role = %role,
        "OIDC role claim resolved to a LibreFang role"
    );
    Some(crate::middleware::OidcRoleGrant {
        name,
        role,
        user_id,
    })
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
        match resolve_single_provider(provider, config.require_email_verified).await {
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
                    require_email_verified: config.require_email_verified,
                });
            }
            Err(e) => warn!(error = %e, "Failed to resolve legacy OIDC provider"),
        }
    }

    resolved
}

async fn resolve_single_provider(
    provider: &librefang_types::config::OidcProvider,
    global_require_email_verified: bool,
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

    // Per-provider override takes precedence over the global setting.
    let require_email_verified = provider
        .require_email_verified
        .unwrap_or(global_require_email_verified);

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
            require_email_verified,
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
        require_email_verified,
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
    let resp = librefang_kernel::http_client::new_client()
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
    let client = librefang_kernel::http_client::new_client();
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
    let client = librefang_kernel::http_client::new_client();
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
    let resp = librefang_kernel::http_client::new_client()
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
    // SECURITY (#5128): `sub` MUST be required at the JWT layer. Without
    // this, a token missing the claim would still decode successfully and
    // the empty-string `sub` would become the primary key in
    // `TOKEN_STORE`, causing different users' sessions to collide on the
    // same slot. We additionally keep `exp` (lifetime enforcement) and
    // `aud` (only when one is configured — see below) in the required
    // set.
    if expected_audience.is_empty() {
        validation.validate_aud = false;
        validation.set_required_spec_claims(&["sub", "exp"]);
    } else {
        validation.set_audience(&[expected_audience]);
        validation.set_required_spec_claims(&["sub", "exp", "aud"]);
    }
    validation.validate_exp = true;

    let token_data = decode::<IdTokenClaims>(token, &decoding_key, &validation)
        .map_err(|e| format!("JWT validation failed: {e}"))?;

    // Defence in depth (#5128): a JWT with an explicit empty `sub` (e.g.
    // `"sub": ""`) is structurally valid at the serde + required-claims
    // layers but is still unusable as a primary key in `TOKEN_STORE`.
    // Reject here so every caller of `validate_jwt_cached` is protected
    // uniformly — the OAuth callback, /userinfo, /introspect, and the
    // auth middleware all funnel through this function.
    if token_data.claims.sub.is_empty() {
        return Err("JWT validation failed: `sub` claim is empty".to_string());
    }

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
    let client = librefang_kernel::http_client::new_client();
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

/// Return the domain portion (everything after the last `@`) of an email.
///
/// SECURITY: this is the only form of an email address allowed into logs —
/// the local part is the user identifier (PII), the domain is a non-PII
/// diagnostic anchor (which IdP tenant signed in). Used both for
/// `allowed_domains` authorization checks and for redacted log fields.
///
/// Malformed inputs never panic: no `@` returns the whole string (a token
/// that isn't an address has no local part to leak), an empty or
/// trailing-`@` value returns `""`.
fn email_domain(email: &str) -> &str {
    email.rsplit('@').next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use base64::Engine;

    #[tokio::test]
    async fn login_redirect_scrubs_authorization_url_parse_errors() {
        let provider = ResolvedProvider {
            id: "broken-provider".to_string(),
            display_name: "Broken".to_string(),
            auth_url: "https://[invalid-host".to_string(),
            token_url: "https://idp.example/token".to_string(),
            userinfo_url: String::new(),
            jwks_uri: String::new(),
            client_id: "client".to_string(),
            client_secret_env: "TEST_SECRET".to_string(),
            redirect_url: "https://app.example/callback".to_string(),
            scopes: vec!["openid".to_string()],
            allowed_domains: vec![],
            audience: String::new(),
            require_email_verified: false,
        };

        let response = build_login_redirect(&provider).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("Failed to build authorization URL"));
        assert!(!body.contains("invalid-host"));
        assert!(!body.contains("IPv6"));
    }

    #[test]
    fn email_domain_redacts_local_part_and_handles_malformed_input() {
        // Normal address: only the domain survives, never the local part (PII).
        assert_eq!(email_domain("user@example.com"), "example.com");
        // Multiple `@`: take everything after the last one; no panic.
        assert_eq!(email_domain("a@b@corp.example.com"), "corp.example.com");
        // No `@` at all: returns the whole string (not an address, no local
        // part to leak) — must not panic.
        assert_eq!(email_domain("noatsign"), "noatsign");
        // Trailing `@`: empty domain, never leaks the local part.
        assert_eq!(email_domain("user@"), "");
        // Leading `@`: domain only.
        assert_eq!(email_domain("@example.com"), "example.com");
        // Empty input: empty domain.
        assert_eq!(email_domain(""), "");
        // The local part is never present in the output for a valid address.
        assert!(!email_domain("secret-user@example.com").contains("secret-user"));
    }

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
        let token = build_state_token("google");
        let payload = verify_state_token(&token).unwrap();
        assert_eq!(payload.provider, "google");
        assert!(!payload.nonce.is_empty());
    }

    #[test]
    fn test_state_token_rejects_tampered_payload() {
        let token = build_state_token("google");
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

    // ── resolve_providers tests ─────────────────────────────────────────

    // #3703: ensure the default is to require verified email — operators
    // must opt out explicitly per provider, not the other way around.
    #[test]
    fn require_email_verified_defaults_true() {
        let config = librefang_types::config::ExternalAuthConfig::default();
        assert!(
            config.require_email_verified,
            "default OIDC config must require email_verified to keep allowed_domains meaningful"
        );
    }

    // #3703: per-provider override `None` must inherit the global setting
    // so the secure default is not silently bypassed by configs written
    // before the field existed. Drives `resolve_providers` directly so a
    // future refactor that drops `unwrap_or(global_require_email_verified)`
    // breaks this test instead of silently re-opening the bypass.
    #[tokio::test]
    async fn require_email_verified_per_provider_inherits_global() {
        fn provider(id: &str, require: Option<bool>) -> librefang_types::config::OidcProvider {
            librefang_types::config::OidcProvider {
                id: id.into(),
                display_name: id.into(),
                issuer_url: String::new(),
                auth_url: "https://idp/auth".into(),
                token_url: "https://idp/token".into(),
                userinfo_url: "https://idp/userinfo".into(),
                jwks_uri: String::new(),
                client_id: "c".into(),
                client_secret_env: "S".into(),
                redirect_url: "http://localhost/cb".into(),
                scopes: vec![],
                allowed_domains: vec![],
                audience: String::new(),
                require_email_verified: require,
            }
        }

        // Global=true, provider=None → inherit true.
        let cfg_inherit_true = librefang_types::config::ExternalAuthConfig {
            enabled: true,
            require_email_verified: true,
            providers: vec![provider("inherit-true", None)],
            ..Default::default()
        };
        let resolved = resolve_providers(&cfg_inherit_true).await;
        assert_eq!(resolved.len(), 1);
        assert!(
            resolved[0].require_email_verified,
            "provider with None override must inherit global=true"
        );

        // Global=false, provider=None → inherit false (negative direction).
        let cfg_inherit_false = librefang_types::config::ExternalAuthConfig {
            enabled: true,
            require_email_verified: false,
            providers: vec![provider("inherit-false", None)],
            ..Default::default()
        };
        let resolved = resolve_providers(&cfg_inherit_false).await;
        assert_eq!(resolved.len(), 1);
        assert!(
            !resolved[0].require_email_verified,
            "provider with None override must inherit global=false"
        );

        // Global=true, provider=Some(false) → explicit override wins.
        let cfg_override = librefang_types::config::ExternalAuthConfig {
            enabled: true,
            require_email_verified: true,
            providers: vec![provider("override", Some(false))],
            ..Default::default()
        };
        let resolved = resolve_providers(&cfg_override).await;
        assert_eq!(resolved.len(), 1);
        assert!(
            !resolved[0].require_email_verified,
            "explicit per-provider override must beat global"
        );
    }

    // ── #6629: refresh must only ever resolve the caller's own session ──
    //
    // `TOKEN_STORE` is a process-global `LazyLock<HashMap>` keyed by upstream OIDC subject, with no record of which local user owns an entry.
    // The two lookups that `auth_refresh` used to fall back on — `find_by_provider` and `find_any_with_refresh` — therefore selected *an* entry rather than the caller's, and the route is reachable by any Admin.
    // These tests pin the replacement: the only way to reach a stored refresh token is to present the access token that session was issued.
    //
    // Two subjects are seeded into the one global store, which is exactly the "two users" condition the issue asks to be proven — the interesting property is cross-selection, and that lives in this map.

    fn seeded_tokens(access: &str, refresh: &str, provider: &str) -> StoredTokens {
        StoredTokens {
            access_token: access.to_string(),
            refresh_token: Some(refresh.to_string()),
            expires_at: None,
            provider_id: provider.to_string(),
            stored_at: std::time::Instant::now(),
        }
    }

    // Token values are per-test locals, never shared constants.
    // `TOKEN_STORE` is a process-global `LazyLock` and libtest runs these on parallel threads, so two tests seeding the SAME access-token value under different subjects would race: `find_by_access_token` does a `find_map` over a `HashMap` and could return either subject, failing whichever test asserted on the other one.
    // Distinct values per test keep them independent without serializing the suite.
    //
    // Within a test, values share a length so the constant-time comparison is actually exercised rather than short-circuited on the length check, and differ in their prefix so a mismatch cannot pass by coincidence.

    #[tokio::test]
    async fn refresh_lookup_resolves_only_the_presented_sessions_own_tokens() {
        let victim_access = "t1-victim-access-aaaaaaaaaaaaaaaaaaaa";
        let victim_refresh = "t1-victim-refresh-aaaaaaaaaaaaaaaaaaa";
        let attacker_access = "t1-attackr-access-bbbbbbbbbbbbbbbbbbb";
        let attacker_refresh = "t1-attackr-refresh-bbbbbbbbbbbbbbbbbb";

        TOKEN_STORE
            .store(
                "sub-t1-victim-6629",
                seeded_tokens(victim_access, victim_refresh, "google"),
            )
            .await;
        TOKEN_STORE
            .store(
                "sub-t1-attacker-6629",
                seeded_tokens(attacker_access, attacker_refresh, "google"),
            )
            .await;

        // Each side resolves its own refresh token.
        let (sub, refresh, _) = TOKEN_STORE
            .find_by_access_token(victim_access)
            .await
            .expect("the victim's own access token must resolve its session");
        assert_eq!(sub, "sub-t1-victim-6629");
        assert_eq!(refresh, victim_refresh);

        let (sub, refresh, _) = TOKEN_STORE
            .find_by_access_token(attacker_access)
            .await
            .expect("the attacker's own session still resolves — that is fine");
        assert_eq!(sub, "sub-t1-attacker-6629");
        // The whole point: presenting your own access token never yields someone else's refresh token, even with both in one store under the same provider.
        assert_eq!(refresh, attacker_refresh);
        assert_ne!(
            refresh, victim_refresh,
            "cross-user selection — this is the #6629 vulnerability"
        );

        TOKEN_STORE.remove("sub-t1-victim-6629").await;
        TOKEN_STORE.remove("sub-t1-attacker-6629").await;
    }

    #[tokio::test]
    async fn refresh_lookup_rejects_an_unknown_or_guessed_access_token() {
        let real_access = "t2-real-access-cccccccccccccccccccccc";

        TOKEN_STORE
            .store(
                "sub-t2-lonely-6629",
                seeded_tokens(real_access, "t2-real-refresh-ccccccccccccccccccc", "google"),
            )
            .await;

        for wrong in [
            // Nothing like it.
            "completely-unrelated-token-value-xxxxx",
            // A near-miss of the same length: the shared prefix must not help.
            "t2-real-access-ccccccccccccccccccccZ",
            // A prefix of the real value.
            "t2-real-access",
            // Empty.
            "",
        ] {
            assert!(
                TOKEN_STORE.find_by_access_token(wrong).await.is_none(),
                "access token {wrong:?} must not resolve any session"
            );
        }

        // Sanity: the real value still resolves, so the loop above is rejecting wrong inputs rather than the lookup being broken outright.
        assert!(
            TOKEN_STORE
                .find_by_access_token(real_access)
                .await
                .is_some(),
            "the seeded access token must still resolve — otherwise the \
             negative assertions above prove nothing"
        );

        TOKEN_STORE.remove("sub-t2-lonely-6629").await;
    }

    /// The empty needle is a fail-open the length check cannot catch.
    ///
    /// `StoredTokens::access_token` is whatever the provider's token response carried, so an IdP that returned an empty `access_token` leaves an entry with `access_token: ""` — and `""` compares equal to `""`, both in length and under `ct_eq`.
    /// Without the explicit guard in `find_by_access_token`, `{"access_token": ""}` would resolve that session and hand back its refresh token to a caller who proved nothing.
    /// The refresh token here is deliberately non-empty: what must be rejected is the *lookup*, not the entry's usability.
    #[tokio::test]
    async fn refresh_lookup_rejects_an_empty_access_token_even_against_an_empty_stored_value() {
        TOKEN_STORE
            .store(
                "sub-t4-emptyaccess-6629",
                seeded_tokens("", "t4-real-refresh-eeeeeeeeeeeeeeeeeeee", "google"),
            )
            .await;

        assert!(
            TOKEN_STORE.find_by_access_token("").await.is_none(),
            "an empty access token must never resolve a session, not even one \
             whose stored access token is also empty"
        );

        TOKEN_STORE.remove("sub-t4-emptyaccess-6629").await;
    }

    /// An entry with no refresh token must not be resolvable — otherwise the caller gets an entry it cannot use and the handler would have to unwrap an `Option` that is only sometimes populated.
    #[tokio::test]
    async fn refresh_lookup_skips_sessions_without_a_refresh_token() {
        let access = "t3-norefresh-access-dddddddddddddddddd";

        TOKEN_STORE
            .store(
                "sub-t3-norefresh-6629",
                StoredTokens {
                    access_token: access.to_string(),
                    refresh_token: None,
                    expires_at: None,
                    provider_id: "google".to_string(),
                    stored_at: std::time::Instant::now(),
                },
            )
            .await;

        assert!(
            TOKEN_STORE.find_by_access_token(access).await.is_none(),
            "a session with no stored refresh token has nothing to hand back"
        );

        TOKEN_STORE.remove("sub-t3-norefresh-6629").await;
    }

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
                require_email_verified: None,
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
                require_email_verified: None,
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

    /// Regression: #3364 — OIDC id_token MUST carry the nonce we signed into
    /// `state`. A missing claim must be rejected outright; previously the
    /// callback fell through to userinfo (no nonce binding), letting a
    /// captured id_token from a different login session sign in as that user.
    #[test]
    fn id_token_missing_nonce_is_rejected_not_fallback() {
        assert_eq!(
            check_id_token_nonce(None, "state-nonce-abc"),
            NonceCheck::Missing,
            "no nonce claim => reject (no userinfo fallback)"
        );
    }

    #[test]
    fn id_token_mismatched_nonce_is_rejected() {
        assert_eq!(
            check_id_token_nonce(Some("attacker-nonce"), "state-nonce-abc"),
            NonceCheck::Mismatch
        );
    }

    #[test]
    fn id_token_matching_nonce_is_accepted() {
        assert_eq!(
            check_id_token_nonce(Some("state-nonce-abc"), "state-nonce-abc"),
            NonceCheck::Ok
        );
    }

    #[test]
    fn id_token_empty_nonce_does_not_match_nonempty_state() {
        // Empty-string is "present but empty" — must not be treated as a
        // wildcard match against any state nonce.
        assert_eq!(
            check_id_token_nonce(Some(""), "state-nonce-abc"),
            NonceCheck::Mismatch
        );
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
                    require_email_verified: None,
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
                    require_email_verified: None,
                },
            ],
            ..Default::default()
        };
        let providers = resolve_providers(&config).await;
        // Only the explicit-URL provider should succeed.
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "good");
    }

    // ── OAuth cache invalidation (refs jwks-cache-no-reload-evict.md) ───
    //
    // These tests pin the contract that `invalidate_oauth_caches()`
    // empties both the JWKS and OIDC discovery caches. We seed the
    // module-level statics directly because the cache types are
    // private — there is no public mutator beyond the fetch path.
    // The test runs serially via a process-wide mutex so two
    // concurrent cases don't observe each other's writes (the caches
    // are global to the process).

    static OAUTH_CACHE_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // The `OAUTH_CACHE_TEST_MUTEX` guard is deliberately held across the
    // `.await`s below: it serializes these cases against the process-global
    // caches, so it must stay locked for the whole test body, awaits
    // included.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_invalidate_oauth_caches_clears_jwks_entries() {
        let _g = OAUTH_CACHE_TEST_MUTEX
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        // Seed: pretend we've already cached IdP-A's signing keys.
        {
            let mut write = JWKS_CACHE.inner.write().await;
            write.insert(
                "https://idp-a.example.com/.well-known/jwks.json".to_string(),
                CachedJwks {
                    keys: vec![JwksKey {
                        kty: "RSA".to_string(),
                        kid: Some("idp-a-key-1".to_string()),
                        key_use: Some("sig".to_string()),
                        alg: Some("RS256".to_string()),
                        n: Some("AQAB".to_string()),
                        e: Some("AQAB".to_string()),
                        x: None,
                        y: None,
                        crv: None,
                    }],
                    fetched_at: std::time::Instant::now(),
                },
            );
            assert_eq!(write.len(), 1, "seed must populate cache");
        }

        // Invalidate as the hot-reload pipeline would.
        invalidate_oauth_caches();

        // Wait for the detached invalidation task to land — it runs
        // on the same multi-thread runtime, so a single yield is not
        // sufficient under heavier test concurrency. Bounded retry.
        for _ in 0..50 {
            if JWKS_CACHE.inner.read().await.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            JWKS_CACHE.inner.read().await.is_empty(),
            "invalidate_oauth_caches must drop all JWKS entries so a \
             subsequent token validation re-fetches from the new IdP"
        );
    }

    // Guard held across awaits on purpose — see the JWKS test above.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_invalidate_oauth_caches_clears_discovery_entries() {
        let _g = OAUTH_CACHE_TEST_MUTEX
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        // Seed the discovery cache with a stale entry.
        {
            let mut write = DISCOVERY_CACHE.inner.write().await;
            write.insert(
                "https://idp-a.example.com".to_string(),
                CachedDiscovery {
                    doc: OidcDiscovery {
                        issuer: "https://idp-a.example.com".to_string(),
                        authorization_endpoint: "https://idp-a.example.com/authorize".to_string(),
                        token_endpoint: "https://idp-a.example.com/token".to_string(),
                        userinfo_endpoint: None,
                        jwks_uri: "https://idp-a.example.com/.well-known/jwks.json".to_string(),
                        scopes_supported: vec![],
                        response_types_supported: vec![],
                        id_token_signing_alg_values_supported: vec![],
                    },
                    fetched_at: std::time::Instant::now(),
                },
            );
            assert_eq!(write.len(), 1, "seed must populate cache");
        }

        invalidate_oauth_caches();

        for _ in 0..50 {
            if DISCOVERY_CACHE.inner.read().await.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            DISCOVERY_CACHE.inner.read().await.is_empty(),
            "invalidate_oauth_caches must drop all discovery entries so a \
             subsequent OIDC handshake re-fetches the new IdP's document"
        );
    }

    // Guard held across awaits on purpose — see the JWKS test above.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_invalidate_oauth_caches_is_idempotent_on_empty() {
        let _g = OAUTH_CACHE_TEST_MUTEX
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        // Ensure both caches are empty up front.
        JWKS_CACHE.inner.write().await.clear();
        DISCOVERY_CACHE.inner.write().await.clear();

        // Should not panic, should not deadlock.
        invalidate_oauth_caches();
        // Wait a tick for the detached tasks to complete.
        tokio::task::yield_now().await;

        assert!(JWKS_CACHE.inner.read().await.is_empty());
        assert!(DISCOVERY_CACHE.inner.read().await.is_empty());
    }

    // ── claim-path resolution (#7746) ───────────────────────────────────

    fn claims_from(json: serde_json::Value) -> IdTokenClaims {
        serde_json::from_value(json).expect("claims deserialize")
    }

    #[test]
    fn flat_claim_paths_read_the_typed_fields() {
        let claims = claims_from(serde_json::json!({
            "sub": "s",
            "roles": ["librefang-admins"],
            "groups": ["platform-oncall"],
        }));
        assert_eq!(
            identity_claim_values(
                &claims,
                &["roles".to_string(), "groups".to_string()],
                "client-a"
            ),
            vec![
                "librefang-admins".to_string(),
                "platform-oncall".to_string()
            ],
        );
    }

    #[test]
    fn nested_claim_paths_reach_keycloaks_realm_and_client_roles() {
        // The reason paths exist at all: neither of these has a flat spelling,
        // and Keycloak is the named conformance target on #7746.
        let claims = claims_from(serde_json::json!({
            "sub": "s",
            "realm_access": { "roles": ["librefang-operators"] },
            "resource_access": {
                "librefang": { "roles": ["workflow-author"] },
                "some-other-client": { "roles": ["not-ours"] },
            },
        }));
        let values = identity_claim_values(
            &claims,
            &[
                "realm_access.roles".to_string(),
                "resource_access.<client>.roles".to_string(),
            ],
            "librefang",
        );
        assert_eq!(
            values,
            vec![
                "librefang-operators".to_string(),
                "workflow-author".to_string()
            ],
        );
        // `<client>` is substituted with *this* provider's client id, so another
        // client's roles in the same token are not picked up.
        assert!(!values.contains(&"not-ours".to_string()));
    }

    #[test]
    fn scope_is_split_on_whitespace_and_only_when_asked_for() {
        let claims = claims_from(serde_json::json!({
            "sub": "s",
            "scope": "openid email librefang:oncall",
        }));
        // Default paths do not include `scope` — see `claim_paths` for why the
        // "what a client app was granted" assertion is held to a different
        // standard than "who the user is".
        assert!(identity_claim_values(
            &claims,
            &["roles".to_string(), "groups".to_string()],
            "client-a"
        )
        .is_empty());
        assert_eq!(
            identity_claim_values(&claims, &["scope".to_string()], "client-a"),
            vec![
                "email".to_string(),
                "librefang:oncall".to_string(),
                "openid".to_string(),
            ],
        );
    }

    #[test]
    fn a_missing_or_wrongly_shaped_claim_contributes_nothing() {
        let claims = claims_from(serde_json::json!({
            "sub": "s",
            "department_id": 42,
            "realm_access": { "roles": "single-role" },
        }));
        assert!(identity_claim_values(
            &claims,
            &[
                "groups".to_string(),
                "absent".to_string(),
                "absent.nested.deeply".to_string(),
                "department_id".to_string(),
            ],
            "client-a"
        )
        .is_empty());
        // A single string resolves to its words, so a provider that emits one
        // role as a bare string still works.
        assert_eq!(
            identity_claim_values(&claims, &["realm_access.roles".to_string()], "client-a"),
            vec!["single-role".to_string()],
        );
    }

    #[test]
    fn claim_values_are_deduplicated_and_ordered_independently_of_the_token() {
        // #3298 at the point where claims become a grant: the IdP's claim
        // ordering is not stable between logins and must not reach anything
        // downstream.
        let a = claims_from(serde_json::json!({
            "sub": "s",
            "roles": ["zulu", "alpha"],
            "groups": ["alpha", "mike"],
        }));
        let b = claims_from(serde_json::json!({
            "sub": "s",
            "roles": ["alpha", "zulu"],
            "groups": ["mike", "alpha"],
        }));
        let paths = ["roles".to_string(), "groups".to_string()];
        assert_eq!(
            identity_claim_values(&a, &paths, "c"),
            vec!["alpha".to_string(), "mike".to_string(), "zulu".to_string()],
        );
        assert_eq!(
            identity_claim_values(&a, &paths, "c"),
            identity_claim_values(&b, &paths, "c"),
        );
    }

    #[test]
    fn userinfo_fallback_carries_roles_groups_and_nested_claims() {
        // Before #7746 this path hardcoded `roles: Vec::new()`, so an operator's
        // maps silently never matched for a provider that issues no ID token.
        let info = serde_json::json!({
            "sub": "s",
            "email": "a@corp.example",
            "roles": ["librefang-admins"],
            "groups": ["platform-oncall"],
            "scope": "openid librefang:oncall",
            "realm_access": { "roles": ["librefang-operators"] },
        });
        assert_eq!(string_list_claim(&info["roles"]), vec!["librefang-admins"]);
        assert_eq!(string_list_claim(&info["groups"]), vec!["platform-oncall"]);
        let passthrough = userinfo_passthrough(&info);
        assert!(
            passthrough.contains_key("realm_access"),
            "a nested claim must survive into `extra` so a dotted claim path resolves on the userinfo path too"
        );
        // Typed keys are not duplicated into the passthrough, so `extra` holds
        // the same set a flattened ID token would.
        for typed in ["sub", "email", "roles", "groups", "scope"] {
            assert!(!passthrough.contains_key(typed), "`{typed}` is typed");
        }
    }
}
