//! OAuth2/OIDC external authentication support.
//!
//! Provides:
//! - OIDC discovery (fetches `.well-known/openid-configuration`)
//! - Multi-provider support (Google, GitHub, Azure AD, Keycloak, generic OIDC)
//! - Login redirect to the external identity provider (per-provider)
//! - Authorization code callback and token exchange
//! - JWT validation with JWKS caching
//! - Token introspection endpoint
//! - User info extraction from ID tokens
//! - Auth middleware for injecting user claims into request extensions

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::routes::AppState;

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
pub struct JwksCache {
    inner: RwLock<HashMap<String, CachedJwks>>,
}

impl JwksCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for JwksCache {
    fn default() -> Self {
        Self::new()
    }
}

/// JWKS cache TTL — 1 hour. Providers rotate keys infrequently.
const JWKS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Global JWKS cache instance (lazily initialized).
static JWKS_CACHE: std::sync::LazyLock<JwksCache> = std::sync::LazyLock::new(JwksCache::new);

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
    #[allow(dead_code)]
    refresh_token: Option<String>,
}

// ── Route: GET /api/auth/providers ──────────────────────────────────────

/// GET /api/auth/providers — List available authentication providers.
pub async fn auth_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let ext_auth = &state.kernel.config.external_auth;
    let api_key_set = !state.kernel.config.api_key.trim().is_empty();

    if !ext_auth.enabled {
        return Json(serde_json::json!({
            "enabled": false,
            "providers": [],
            "api_key_auth": api_key_set,
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
        "api_key_auth": api_key_set,
    }))
}

// ── Route: GET /api/auth/login ──────────────────────────────────────────

/// Query params for the login redirect.
#[derive(Deserialize)]
pub struct LoginQuery {
    /// Optional `state` parameter for CSRF protection (passed through to callback).
    #[serde(default)]
    pub state: Option<String>,
}

/// GET /api/auth/login — Redirect to the external identity provider (legacy single-provider).
pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LoginQuery>,
) -> Response {
    let ext_auth = &state.kernel.config.external_auth;
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

    build_login_redirect(provider, query.state).into_response()
}

/// GET /api/auth/login/:provider — Redirect to a specific provider.
pub async fn auth_login_provider(
    State(state): State<Arc<AppState>>,
    Path(provider_id): Path<String>,
    Query(query): Query<LoginQuery>,
) -> Response {
    let ext_auth = &state.kernel.config.external_auth;
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

    build_login_redirect(provider, query.state).into_response()
}

/// Build the OAuth2 authorization redirect for the given provider.
fn build_login_redirect(
    provider: &ResolvedProvider,
    state_param: Option<String>,
) -> impl IntoResponse {
    let state_param = state_param.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let scopes = provider.scopes.join(" ");

    match url::Url::parse_with_params(
        &provider.auth_url,
        &[
            ("response_type", "code"),
            ("client_id", &provider.client_id),
            ("redirect_uri", &provider.redirect_url),
            ("scope", &scopes),
            ("state", &state_param),
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
    /// State parameter (CSRF token).
    #[serde(default)]
    pub state: Option<String>,
    /// Provider identifier (passed through from login redirect).
    #[serde(default)]
    pub provider: Option<String>,
    /// Error from the IdP (if authorization was denied).
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// POST body for the callback (programmatic clients).
#[derive(Deserialize)]
pub struct CallbackBody {
    /// Authorization code.
    pub code: String,
    /// State parameter for CSRF validation.
    #[serde(default)]
    pub state: Option<String>,
    /// Provider ID.
    #[serde(default)]
    pub provider: Option<String>,
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
}

#[derive(Serialize)]
struct CallbackUser {
    sub: String,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
}

/// GET /api/auth/callback — Handle the OAuth2 authorization code callback (browser redirect).
pub async fn auth_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
) -> impl IntoResponse {
    let ext_auth = &state.kernel.config.external_auth;
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

    let provider_id = query.provider.as_deref();
    handle_code_exchange(ext_auth, &code, provider_id).await
}

/// POST /api/auth/callback — Handle the OAuth2 callback (programmatic clients).
pub async fn auth_callback_post(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CallbackBody>,
) -> impl IntoResponse {
    let ext_auth = &state.kernel.config.external_auth;
    if !ext_auth.enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "External authentication is not configured"})),
        )
            .into_response();
    }

    let provider_id = body.provider.as_deref();
    handle_code_exchange(ext_auth, &body.code, provider_id).await
}

/// Shared code exchange logic for both GET and POST callback handlers.
async fn handle_code_exchange(
    ext_auth: &librefang_types::config::ExternalAuthConfig,
    code: &str,
    provider_id: Option<&str>,
) -> Response {
    let providers = resolve_providers(ext_auth).await;

    // Find the requested provider, or default to the first one.
    let provider = if let Some(id) = provider_id {
        match providers.iter().find(|p| p.id == id) {
            Some(p) => p,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Unknown auth provider: {}", id)})),
                )
                    .into_response();
            }
        }
    } else {
        match providers.first() {
            Some(p) => p,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({"error": "No auth providers configured"})),
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
            warn!("Token exchange failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("Token exchange failed: {e}")})),
            )
                .into_response();
        }
    };

    // Try to validate the ID token if present.
    let claims = if let Some(ref id_token) = token_resp.id_token {
        if !id_token.is_empty() && !provider.jwks_uri.is_empty() {
            match validate_jwt_cached(id_token, &provider.jwks_uri, &provider.audience).await {
                Ok(c) => Some(c),
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
                    },
                    Err(e) => {
                        warn!(error = %e, "Userinfo fetch failed");
                        return (
                            StatusCode::BAD_GATEWAY,
                            Json(serde_json::json!({"error": format!("Could not retrieve user info: {e}")})),
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
            let domain = email.split('@').next_back().unwrap_or("");
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
        }),
    )
        .into_response()
}

// ── Route: GET /api/auth/userinfo ───────────────────────────────────────

/// GET /api/auth/userinfo — Return info about the currently authenticated user.
///
/// If a valid JWT is in the Authorization header and JWKS validation succeeds,
/// returns the decoded claims. Otherwise falls back to provider userinfo endpoint.
pub async fn auth_userinfo(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    let ext_auth = &state.kernel.config.external_auth;

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
pub async fn auth_introspect(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IntrospectRequest>,
) -> impl IntoResponse {
    let ext_auth = &state.kernel.config.external_auth;
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
    let config = &state.kernel.config.external_auth;
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
                // Check allowed domains.
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
/// For providers with an `issuer_url`, performs OIDC discovery.
/// For providers with explicit URLs, uses those directly.
/// Falls back to legacy single-provider config if no explicit providers are defined.
async fn resolve_providers(
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
        match discover_oidc(&config.issuer_url).await {
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

    // Use OIDC discovery.
    if provider.issuer_url.is_empty() {
        return Err(format!(
            "Provider '{}' has no issuer_url and no explicit auth_url/token_url",
            provider.id
        ));
    }

    let disc = discover_oidc(&provider.issuer_url).await?;
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

/// Fetch the OIDC discovery document from `{issuer}/.well-known/openid-configuration`.
async fn discover_oidc(issuer_url: &str) -> Result<OidcDiscovery, String> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );
    let resp = reqwest::get(&url)
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
    let client = reqwest::Client::new();
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
        return Err(format!("Token endpoint returned HTTP {status}: {body}"));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))
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
    let resp = reqwest::get(jwks_uri)
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
    let client = reqwest::Client::new();
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
}
