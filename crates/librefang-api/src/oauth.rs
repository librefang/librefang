//! OAuth2/OIDC external authentication support.
//!
//! Provides:
//! - OIDC discovery (fetches `.well-known/openid-configuration`)
//! - Login redirect to the external identity provider
//! - Authorization code callback and token exchange
//! - JWT validation for session tokens
//! - User info extraction from ID tokens

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

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

/// JWKS key entry (RSA public key).
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

/// Response listing available auth providers.
#[derive(Serialize)]
struct AuthProvidersResponse {
    /// Whether external auth is configured and enabled.
    enabled: bool,
    /// The OIDC issuer URL (empty if not configured).
    issuer_url: String,
    /// Whether the basic API key auth is also available.
    api_key_auth: bool,
}

/// GET /api/auth/providers — List available authentication methods.
pub async fn auth_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let ext_auth = &state.kernel.config.external_auth;
    let api_key_set = !state.kernel.config.api_key.trim().is_empty();

    Json(AuthProvidersResponse {
        enabled: ext_auth.enabled,
        issuer_url: if ext_auth.enabled {
            ext_auth.issuer_url.clone()
        } else {
            String::new()
        },
        api_key_auth: api_key_set,
    })
}

// ── Route: GET /api/auth/login ──────────────────────────────────────────

/// Query params for the login redirect.
#[derive(Deserialize)]
pub struct LoginQuery {
    /// Optional `state` parameter for CSRF protection (passed through to callback).
    #[serde(default)]
    pub state: Option<String>,
}

/// GET /api/auth/login — Redirect to the external identity provider.
pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LoginQuery>,
) -> Response {
    let ext_auth = &state.kernel.config.external_auth;
    if !ext_auth.enabled || ext_auth.issuer_url.is_empty() || ext_auth.client_id.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "External authentication is not configured"})),
        )
            .into_response();
    }

    // Discover OIDC configuration
    let discovery = match discover_oidc(&ext_auth.issuer_url).await {
        Ok(d) => d,
        Err(e) => {
            warn!("OIDC discovery failed: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("OIDC discovery failed: {e}")})),
            )
                .into_response();
        }
    };

    // Build authorization URL
    let state_param = query
        .state
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let scopes = ext_auth.scopes.join(" ");
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
        discovery.authorization_endpoint,
        urlencoding(&ext_auth.client_id),
        urlencoding(&ext_auth.redirect_url),
        urlencoding(&scopes),
        urlencoding(&state_param),
    );

    info!(issuer = %ext_auth.issuer_url, "Redirecting to external IdP for login");
    Redirect::temporary(&auth_url).into_response()
}

// ── Route: GET /api/auth/callback ───────────────────────────────────────

/// Query params for the OAuth2 callback.
#[derive(Deserialize)]
pub struct CallbackQuery {
    /// Authorization code from the IdP.
    #[serde(default)]
    pub code: Option<String>,
    /// State parameter (CSRF token).
    #[serde(default)]
    #[allow(dead_code)]
    pub state: Option<String>,
    /// Error from the IdP (if authorization was denied).
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// Callback response with session token.
#[derive(Serialize)]
struct CallbackResponse {
    /// Session token (JWT or opaque) for authenticating subsequent API calls.
    token: String,
    /// Token type (always "Bearer").
    token_type: String,
    /// Token lifetime in seconds.
    expires_in: u64,
    /// User info extracted from the ID token.
    user: CallbackUser,
}

#[derive(Serialize)]
struct CallbackUser {
    sub: String,
    email: Option<String>,
    name: Option<String>,
}

/// GET /api/auth/callback — Handle the OAuth2 authorization code callback.
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

    // Check for IdP errors
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

    // Discover OIDC configuration
    let discovery = match discover_oidc(&ext_auth.issuer_url).await {
        Ok(d) => d,
        Err(e) => {
            warn!("OIDC discovery failed during callback: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("OIDC discovery failed: {e}")})),
            )
                .into_response();
        }
    };

    // Exchange authorization code for tokens
    let client_secret = std::env::var(&ext_auth.client_secret_env).unwrap_or_default();
    let token_resp = match exchange_code(
        &discovery.token_endpoint,
        &code,
        &ext_auth.client_id,
        &client_secret,
        &ext_auth.redirect_url,
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

    // Validate the ID token
    let id_token_str = match token_resp.id_token {
        Some(ref t) if !t.is_empty() => t.clone(),
        _ => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "No ID token in token response. Provider may not support OIDC."})),
            )
                .into_response();
        }
    };

    let claims = match validate_id_token(&id_token_str, &discovery.jwks_uri, ext_auth).await {
        Ok(c) => c,
        Err(e) => {
            warn!("ID token validation failed: {e}");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": format!("ID token validation failed: {e}")})),
            )
                .into_response();
        }
    };

    // Check allowed domains
    if !ext_auth.allowed_domains.is_empty() {
        if let Some(ref email) = claims.email {
            let domain = email.split('@').next_back().unwrap_or("");
            if !ext_auth.allowed_domains.iter().any(|d| d == domain) {
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
        "External auth login successful"
    );

    // Return the access token (or ID token) as the session token.
    // The middleware will validate this on subsequent requests.
    let expires_in = token_resp.expires_in.unwrap_or(ext_auth.session_ttl_secs);
    (
        StatusCode::OK,
        Json(CallbackResponse {
            token: token_resp.access_token.clone(),
            token_type: "Bearer".to_string(),
            expires_in,
            user: CallbackUser {
                sub: claims.sub,
                email: claims.email,
                name: claims.name,
            },
        }),
    )
        .into_response()
}

// ── Route: GET /api/auth/userinfo ───────────────────────────────────────

/// GET /api/auth/userinfo — Return info about the currently authenticated user.
///
/// Requires a valid Bearer token (either API key or OAuth session token).
pub async fn auth_userinfo(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let ext_auth = &state.kernel.config.external_auth;

    Json(serde_json::json!({
        "auth_method": if ext_auth.enabled { "external_oauth" } else { "api_key" },
        "issuer": if ext_auth.enabled { &ext_auth.issuer_url } else { "" },
    }))
}

// ── OIDC Discovery helper ───────────────────────────────────────────────

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

/// Fetch JWKS from the given URI.
async fn fetch_jwks(jwks_uri: &str) -> Result<Vec<JwksKey>, String> {
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
    Ok(jwks.keys)
}

/// Validate an OIDC ID token using the JWKS from the provider.
async fn validate_id_token(
    id_token: &str,
    jwks_uri: &str,
    config: &librefang_types::config::ExternalAuthConfig,
) -> Result<IdTokenClaims, String> {
    // Decode the header to find the key ID
    let header =
        jsonwebtoken::decode_header(id_token).map_err(|e| format!("Invalid JWT header: {e}"))?;

    let keys = fetch_jwks(jwks_uri).await?;

    // Find the matching key
    let key = if let Some(ref kid) = header.kid {
        keys.iter()
            .find(|k| k.kid.as_deref() == Some(kid))
            .ok_or_else(|| format!("No JWKS key found for kid={kid}"))?
    } else {
        // If no kid in header, use the first RSA key
        keys.iter()
            .find(|k| k.kty == "RSA")
            .ok_or("No RSA key found in JWKS")?
    };

    // Build the decoding key from RSA components
    let n = key.n.as_deref().ok_or("JWKS key missing 'n' component")?;
    let e = key.e.as_deref().ok_or("JWKS key missing 'e' component")?;
    let decoding_key = DecodingKey::from_rsa_components(n, e)
        .map_err(|err| format!("Invalid RSA key components: {err}"))?;

    // Configure validation
    let algorithm = match header.alg {
        jsonwebtoken::Algorithm::RS256 => Algorithm::RS256,
        jsonwebtoken::Algorithm::RS384 => Algorithm::RS384,
        jsonwebtoken::Algorithm::RS512 => Algorithm::RS512,
        other => return Err(format!("Unsupported JWT algorithm: {other:?}")),
    };

    let mut validation = Validation::new(algorithm);
    // Set the expected audience
    let expected_aud = if config.audience.is_empty() {
        &config.client_id
    } else {
        &config.audience
    };
    validation.set_audience(&[expected_aud]);
    validation.set_issuer(&[&config.issuer_url]);

    let token_data = decode::<IdTokenClaims>(id_token, &decoding_key, &validation)
        .map_err(|e| format!("JWT validation failed: {e}"))?;

    Ok(token_data.claims)
}

/// Validate an access/session token against the external auth provider's JWKS.
///
/// This is used by the auth middleware to verify OAuth session tokens
/// on protected endpoints.
pub async fn validate_external_token(
    token: &str,
    config: &librefang_types::config::ExternalAuthConfig,
) -> Result<IdTokenClaims, String> {
    let discovery = discover_oidc(&config.issuer_url).await?;
    validate_id_token(token, &discovery.jwks_uri, config).await
}

/// Simple URL encoding helper.
fn urlencoding(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
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
    fn test_urlencoding() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("a=b&c=d"), "a%3Db%26c%3Dd");
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
    }
}
