//! Google Cloud Vertex AI driver.
//!
//! Uses the same Gemini generateContent API format but authenticates via
//! Google Cloud OAuth2 (service account JSON key or Application Default
//! Credentials via gcloud CLI) instead of API keys.
//!
//! Endpoint format:
//! ```text
//! https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/google/models/{model}:generateContent
//! ```
//!
//! Token acquisition supports two methods:
//! 1. **Service account JSON** — reads the key file and exchanges a JWT for a token
//! 2. **gcloud CLI** — runs `gcloud auth print-access-token` (fallback default)
//!
//! Tokens are cached with a ~50 minute TTL and auto-refreshed before expiry.

use crate::llm_driver::{
    CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError, StreamEvent,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

// ─── OAuth2 token management ────────────────────────────────────────

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

enum CredentialSource {
    ServiceAccountJson(serde_json::Value),
    GcloudCli,
    /// Pre-set static token — bypasses OAuth2 entirely. Used by test constructors.
    StaticToken(String),
}

struct TokenManager {
    credential_source: CredentialSource,
    cached: Option<CachedToken>,
}

impl TokenManager {
    fn new(credential_source: CredentialSource) -> Self {
        Self {
            credential_source,
            cached: None,
        }
    }

    async fn get_token(&mut self) -> Result<String, LlmError> {
        // Return cached token if still valid (with 5-minute margin).
        if let Some(ref cached) = self.cached {
            let margin = chrono::Duration::minutes(5);
            if chrono::Utc::now() + margin < cached.expires_at {
                return Ok(cached.access_token.clone());
            }
        }

        let token = match &self.credential_source {
            CredentialSource::ServiceAccountJson(json) => {
                Self::token_from_service_account(json).await?
            }
            CredentialSource::GcloudCli => Self::token_from_gcloud().await?,
            CredentialSource::StaticToken(t) => CachedToken {
                access_token: t.clone(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            },
        };

        self.cached = Some(token.clone());
        Ok(token.access_token)
    }

    /// Exchange a service account JWT assertion for an access token.
    async fn token_from_service_account(
        sa_json: &serde_json::Value,
    ) -> Result<CachedToken, LlmError> {
        let client_email = sa_json["client_email"]
            .as_str()
            .ok_or_else(|| LlmError::MissingApiKey("client_email missing in SA key".into()))?;
        let private_key_pem = sa_json["private_key"]
            .as_str()
            .ok_or_else(|| LlmError::MissingApiKey("private_key missing in SA key".into()))?;
        let token_uri = sa_json["token_uri"]
            .as_str()
            .unwrap_or("https://oauth2.googleapis.com/token");

        let now = chrono::Utc::now();
        let iat = now.timestamp();
        let exp = iat + 3600; // 1 hour

        // Build and sign the service-account JWT assertion.
        let claims_json = serde_json::json!({
            "iss": client_email,
            "scope": "https://www.googleapis.com/auth/cloud-platform",
            "aud": token_uri,
            "iat": iat,
            "exp": exp,
        });
        let jwt = encode_service_account_jwt(private_key_pem, &claims_json)?;

        // Exchange JWT for access token.
        let client = librefang_http::new_client();
        let resp = client
            .post(token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await
            .map_err(|e| LlmError::Http(format!("OAuth2 token request failed: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        if !status.is_success() {
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: format!("OAuth2 token exchange failed: {body}"),
                code: None,
            });
        }

        let token_resp: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| LlmError::Parse(e.to_string()))?;

        let access_token = token_resp["access_token"]
            .as_str()
            .ok_or_else(|| LlmError::Parse("Missing access_token in response".into()))?
            .to_string();

        let expires_in = token_resp["expires_in"].as_i64().unwrap_or(3600);

        Ok(CachedToken {
            access_token,
            expires_at: now + chrono::Duration::seconds(expires_in),
        })
    }

    /// Get token from `gcloud auth print-access-token`.
    async fn token_from_gcloud() -> Result<CachedToken, LlmError> {
        let output = tokio::process::Command::new("gcloud")
            .args(["auth", "print-access-token"])
            .output()
            .await
            .map_err(|e| {
                LlmError::MissingApiKey(format!(
                    "Failed to run `gcloud auth print-access-token`: {e}. \
                     Set VERTEX_AI_SERVICE_ACCOUNT_JSON or install gcloud CLI."
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(LlmError::MissingApiKey(format!(
                "gcloud auth failed: {stderr}"
            )));
        }

        let access_token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if access_token.is_empty() {
            return Err(LlmError::MissingApiKey(
                "gcloud returned empty access token".into(),
            ));
        }

        // gcloud tokens typically last 1 hour; we cache for 50 minutes.
        Ok(CachedToken {
            access_token,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(50),
        })
    }
}

// ─── RSA-SHA256 signing ────────────────────────────────────────────

fn encode_service_account_jwt(
    private_key_pem: &str,
    claims: &serde_json::Value,
) -> Result<String, LlmError> {
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .map_err(|e| LlmError::Parse(format!("Invalid service-account RSA private key: {e}")))?;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        claims,
        &key,
    )
    .map_err(|e| LlmError::Parse(format!("Failed to sign service-account JWT: {e}")))
}

// ─── Vertex AI driver ───────────────────────────────────────────────

/// Vertex AI LLM driver.
pub struct VertexAiDriver {
    project_id: String,
    region: String,
    token_manager: Arc<RwLock<TokenManager>>,
    client: reqwest::Client,
    /// Whether to emit the three `x-librefang-{agent,session,step}-id` trace
    /// headers on outbound requests. Mirrors
    /// `KernelConfig.telemetry.emit_caller_trace_headers`; when `false`, no
    /// trace headers are emitted regardless of whether `CompletionRequest`'s
    /// caller-id fields are populated.
    emit_caller_trace_headers: bool,
    /// When set, replaces the full aiplatform base URL. Used by test
    /// constructors to redirect requests to a mock server.
    base_url_override: Option<String>,
    /// Max in-driver retries for a single API call (#10). Counts re-attempts
    /// after the first try, so the request is issued at most `max_retries + 1`
    /// times. Sourced from `DriverConfig.max_retries` (default 3).
    max_retries: u32,
}

impl VertexAiDriver {
    /// Create a new Vertex AI driver.
    pub fn new(config: &DriverConfig) -> Result<Self, LlmError> {
        let credential_source = resolve_credentials(config)?;
        let project_id = resolve_project_id(config, &credential_source)?;
        let region = resolve_region(config);

        Ok(Self {
            project_id,
            region,
            token_manager: Arc::new(RwLock::new(TokenManager::new(credential_source))),
            client: librefang_http::new_client(),
            emit_caller_trace_headers: true,
            base_url_override: None,
            max_retries: config.max_retries,
        })
    }

    /// Override the trace-header emission flag (mirrors
    /// `KernelConfig.telemetry.emit_caller_trace_headers`). Default is `true`,
    /// meaning the three `x-librefang-{agent,session,step}-id` headers are
    /// emitted on every request that has those fields populated. Pass `false`
    /// to suppress them entirely.
    pub fn with_emit_caller_trace_headers(mut self, emit: bool) -> Self {
        self.emit_caller_trace_headers = emit;
        self
    }

    /// Test-only constructor: creates a driver with a pre-set access token and
    /// a custom base URL (e.g. a `wiremock::MockServer` URI). Bypasses OAuth2
    /// token exchange entirely. Requests go to
    /// `{base_url}/v1/projects/test-project/locations/us-central1/publishers/google/models/{model}:{method}`.
    #[doc(hidden)]
    pub fn new_for_test(access_token: String, base_url: String) -> Self {
        let tm = TokenManager::new(CredentialSource::StaticToken(access_token));
        Self {
            project_id: "test-project".to_string(),
            region: "us-central1".to_string(),
            token_manager: Arc::new(RwLock::new(tm)),
            client: librefang_http::proxied_client(),
            emit_caller_trace_headers: true,
            base_url_override: Some(base_url),
            max_retries: 3,
        }
    }

    /// Build the full endpoint URL for a model.
    fn endpoint_url(&self, model: &str, streaming: bool) -> String {
        // Strip "vertex-ai/" prefix if present.
        let model_name = model.strip_prefix("vertex-ai/").unwrap_or(model);
        let method = if streaming {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        if let Some(ref base) = self.base_url_override {
            return format!(
                "{base}/v1/projects/{project}/locations/{region}/publishers/google/models/{model}:{method}",
                project = self.project_id,
                region = self.region,
                model = model_name,
                method = method,
            );
        }
        format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/google/models/{model}:{method}",
            region = self.region,
            project = self.project_id,
            model = model_name,
            method = method,
        )
    }
}

fn resolve_credentials(config: &DriverConfig) -> Result<CredentialSource, LlmError> {
    // 1. Explicit config value (may contain JSON or path).
    if let Some(key) = config
        .vertex_ai
        .credentials_path
        .as_ref()
        .or(config.api_key.as_ref())
    {
        if !key.is_empty() {
            // Try parsing as JSON directly.
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(key) {
                if json.get("type").and_then(|t| t.as_str()) == Some("service_account") {
                    return Ok(CredentialSource::ServiceAccountJson(json));
                }
            }
            // Try as a file path.
            if let Ok(contents) = std::fs::read_to_string(key) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                    if json.get("type").and_then(|t| t.as_str()) == Some("service_account") {
                        return Ok(CredentialSource::ServiceAccountJson(json));
                    }
                }
            }
        }
    }

    // 2. VERTEX_AI_SERVICE_ACCOUNT_JSON env var (JSON string).
    if let Ok(json_str) = std::env::var("VERTEX_AI_SERVICE_ACCOUNT_JSON") {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
            return Ok(CredentialSource::ServiceAccountJson(json));
        }
    }

    // 3. GOOGLE_APPLICATION_CREDENTIALS env var (file path).
    if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                return Ok(CredentialSource::ServiceAccountJson(json));
            }
        }
    }

    // 4. Fall back to gcloud CLI.
    Ok(CredentialSource::GcloudCli)
}

fn resolve_project_id(
    config: &DriverConfig,
    credential_source: &CredentialSource,
) -> Result<String, LlmError> {
    if let Some(project_id) = config
        .vertex_ai
        .project_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(project_id.clone());
    }

    // Check env vars.
    for var in [
        "VERTEX_AI_PROJECT_ID",
        "GOOGLE_CLOUD_PROJECT",
        "GCLOUD_PROJECT",
    ] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return Ok(val);
            }
        }
    }

    // Extract from base_url if it contains a project.
    if let Some(ref url) = config.base_url {
        // e.g., https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/...
        if let Some(idx) = url.find("/projects/") {
            let after = &url[idx + 10..];
            if let Some(end) = after.find('/') {
                let project = &after[..end];
                if !project.is_empty() {
                    return Ok(project.to_string());
                }
            }
        }
    }

    // Extract from service account JSON.
    if let CredentialSource::ServiceAccountJson(ref json) = credential_source {
        if let Some(project) = json["project_id"].as_str() {
            if !project.is_empty() {
                return Ok(project.to_string());
            }
        }
    }

    Err(LlmError::MissingApiKey(
        "Vertex AI project ID not found. Set VERTEX_AI_PROJECT_ID, \
         GOOGLE_CLOUD_PROJECT, or provide a service account key with project_id."
            .into(),
    ))
}

fn resolve_region(config: &DriverConfig) -> String {
    if let Some(region) = config
        .vertex_ai
        .region
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        return region.clone();
    }

    // Check env vars.
    for var in ["VERTEX_AI_REGION", "GOOGLE_CLOUD_REGION"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }

    // Extract from base_url if provided.
    if let Some(ref url) = config.base_url {
        // e.g., https://us-central1-aiplatform.googleapis.com/...
        if let Some(region) = url.strip_prefix("https://").and_then(|s| {
            s.strip_suffix("-aiplatform.googleapis.com")
                .or_else(|| s.split("-aiplatform.googleapis.com").next())
        }) {
            // Only take the region part (before any path).
            let region = region.split('/').next().unwrap_or(region);
            if !region.is_empty() && region.contains('-') {
                return region.to_string();
            }
        }
    }

    "us-central1".to_string()
}

// ─── LlmDriver implementation ──────────────────────────────────────

#[async_trait]
impl LlmDriver for VertexAiDriver {
    #[tracing::instrument(
        name = "llm.complete",
        skip_all,
        fields(provider = "vertex_ai", model = %request.model)
    )]
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let url = self.endpoint_url(&request.model, false);

        let (contents, system_instruction) =
            super::gemini::convert_messages(&request.messages, &request.system);
        let tools = super::gemini::convert_tools(&request);
        let body = super::gemini::build_request(
            contents,
            system_instruction,
            tools,
            Some(request.temperature),
            Some(request.max_tokens),
            request.response_format.as_ref(),
        );

        // Configurable in-driver retry cap (#10); default 3 (four total
        // attempts, including transport-error retries below).
        let max_retries = self.max_retries as u64;
        let mut last_error = None;
        for attempt in 0..=max_retries {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500 * (1 << attempt))).await;
            }

            let token = self.token_manager.write().await.get_token().await?;
            debug!(url = %url, attempt, "Sending Vertex AI request");

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .headers(super::trace_headers::build_trace_header_map(
                    &[],
                    &request,
                    self.emit_caller_trace_headers,
                ))
                .json(&body)
                .send()
                .await;
            // #10: route transport-layer errors (connection refused, TLS,
            // read timeout) through the same retry decision as 429/503 rather
            // than returning immediately via `?`.
            let resp = match resp {
                Ok(resp) => resp,
                Err(e) => {
                    if attempt < max_retries && crate::backoff::transport_error_is_retryable(&e) {
                        tracing::warn!(error = %e, attempt, "Vertex AI transport error, retrying");
                        last_error = Some(LlmError::Http(e.to_string()));
                        continue;
                    }
                    return Err(LlmError::Http(e.to_string()));
                }
            };

            let status = resp.status();

            if status.is_success() {
                let resp_body = resp
                    .text()
                    .await
                    .map_err(|e| LlmError::Http(e.to_string()))?;
                return super::gemini::parse_and_convert_response(&resp_body);
            }

            // Read Retry-After before the body consumes the response.
            let retry_after_ms =
                crate::retry_after::parse_retry_after_ms(resp.headers(), 1000 * (1 << attempt));
            let resp_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                last_error = Some(LlmError::RateLimited {
                    retry_after_ms,
                    message: None,
                });
                continue;
            }
            if status.as_u16() == 503 {
                last_error = Some(LlmError::Overloaded { retry_after_ms });
                continue;
            }
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(LlmError::AuthenticationFailed(
                    super::gemini::parse_gemini_error(&resp_body),
                ));
            }
            if status.as_u16() == 404 {
                return Err(LlmError::ModelNotFound(super::gemini::parse_gemini_error(
                    &resp_body,
                )));
            }

            return Err(LlmError::Api {
                status: status.as_u16(),
                message: super::gemini::parse_gemini_error(&resp_body),
                code: None,
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::Http("Max retries exceeded".into())))
    }

    #[tracing::instrument(
        name = "llm.stream",
        skip_all,
        fields(provider = "vertex_ai", model = %request.model)
    )]
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let url = self.endpoint_url(&request.model, true);

        let (contents, system_instruction) =
            super::gemini::convert_messages(&request.messages, &request.system);
        let tools = super::gemini::convert_tools(&request);
        let body = super::gemini::build_request(
            contents,
            system_instruction,
            tools,
            Some(request.temperature),
            Some(request.max_tokens),
            request.response_format.as_ref(),
        );

        // Configurable in-driver retry cap (#10); default 3 (four total
        // attempts, including transport-error retries below).
        let max_retries = self.max_retries as u64;
        let mut last_error = None;
        for attempt in 0..=max_retries {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500 * (1 << attempt))).await;
            }

            let token = self.token_manager.write().await.get_token().await?;
            debug!(url = %url, attempt, "Sending Vertex AI streaming request");

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .headers(super::trace_headers::build_trace_header_map(
                    &[],
                    &request,
                    self.emit_caller_trace_headers,
                ))
                .json(&body)
                .send()
                .await;
            // #10: route transport-layer errors (connection refused, TLS,
            // read timeout) through the same retry decision as 429/503 rather
            // than returning immediately via `?`.
            let resp = match resp {
                Ok(resp) => resp,
                Err(e) => {
                    if attempt < max_retries && crate::backoff::transport_error_is_retryable(&e) {
                        tracing::warn!(error = %e, attempt, "Vertex AI transport error, retrying");
                        last_error = Some(LlmError::Http(e.to_string()));
                        continue;
                    }
                    return Err(LlmError::Http(e.to_string()));
                }
            };

            let status = resp.status();

            if status.is_success() {
                return super::gemini::stream_gemini_sse(resp, tx).await;
            }

            // Read Retry-After before the body consumes the response.
            let retry_after_ms =
                crate::retry_after::parse_retry_after_ms(resp.headers(), 1000 * (1 << attempt));
            let resp_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                last_error = Some(LlmError::RateLimited {
                    retry_after_ms,
                    message: None,
                });
                continue;
            }
            if status.as_u16() == 503 {
                last_error = Some(LlmError::Overloaded { retry_after_ms });
                continue;
            }
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(LlmError::AuthenticationFailed(
                    super::gemini::parse_gemini_error(&resp_body),
                ));
            }
            if status.as_u16() == 404 {
                return Err(LlmError::ModelNotFound(super::gemini::parse_gemini_error(
                    &resp_body,
                )));
            }

            return Err(LlmError::Api {
                status: status.as_u16(),
                message: super::gemini::parse_gemini_error(&resp_body),
                code: None,
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::Http("Max retries exceeded".into())))
    }

    fn family(&self) -> crate::llm_driver::LlmFamily {
        crate::llm_driver::LlmFamily::Google
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_account_jwt_uses_rs256_and_verifies_with_public_key() {
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};

        let mut rng = rsa::rand_core::OsRng;
        let private_key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
        let public_key = rsa::RsaPublicKey::from(&private_key);
        let private_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("encode PKCS#8 key");
        let public_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .expect("encode public key");
        let claims = serde_json::json!({
            "iss": "service-account@example.test",
            "scope": "https://www.googleapis.com/auth/cloud-platform",
            "aud": "https://oauth2.googleapis.com/token",
            "iat": 1_700_000_000_i64,
            "exp": 1_700_003_600_i64,
        });

        let jwt = encode_service_account_jwt(private_pem.as_str(), &claims).expect("sign JWT");
        let header = jsonwebtoken::decode_header(&jwt).expect("decode JWT header");
        assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);

        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        validation.validate_exp = false;
        validation.validate_aud = false;
        let decoded = jsonwebtoken::decode::<serde_json::Value>(
            &jwt,
            &jsonwebtoken::DecodingKey::from_rsa_pem(public_pem.as_bytes())
                .expect("parse public key"),
            &validation,
        )
        .expect("verify JWT");
        assert_eq!(decoded.claims, claims);
    }

    #[test]
    fn service_account_jwt_rejects_invalid_private_key() {
        let error = encode_service_account_jwt("not a PEM key", &serde_json::json!({}))
            .expect_err("invalid key must fail");
        assert!(error
            .to_string()
            .contains("Invalid service-account RSA private key"));
    }

    #[test]
    fn test_endpoint_url() {
        let driver = VertexAiDriver {
            project_id: "my-project".to_string(),
            region: "us-central1".to_string(),
            token_manager: Arc::new(RwLock::new(TokenManager::new(CredentialSource::GcloudCli))),
            client: librefang_http::new_client(),
            emit_caller_trace_headers: true,
            base_url_override: None,
            max_retries: 3,
        };

        let url = driver.endpoint_url("vertex-ai/gemini-2.5-pro", false);
        assert_eq!(
            url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.5-pro:generateContent"
        );

        let stream_url = driver.endpoint_url("gemini-2.5-flash", true);
        assert!(stream_url.contains("streamGenerateContent?alt=sse"));
        assert!(stream_url.contains("gemini-2.5-flash"));
    }

    #[test]
    fn test_resolve_region_default() {
        // With no env vars set, should default to us-central1.
        let config = DriverConfig {
            provider: "vertex-ai".to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig::default(),
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
            request_timeout_secs: None,
            emit_caller_trace_headers: true,
            max_retries: 3,
        };
        let region = resolve_region(&config);
        assert_eq!(region, "us-central1");
    }

    #[test]
    fn test_resolve_region_explicit() {
        let config = DriverConfig {
            provider: "vertex-ai".to_string(),
            api_key: None,
            base_url: None,
            vertex_ai: librefang_types::config::VertexAiConfig {
                region: Some("europe-west4".to_string()),
                ..Default::default()
            },
            azure_openai: librefang_types::config::AzureOpenAiConfig::default(),
            skip_permissions: true,
            message_timeout_secs: 300,
            mcp_bridge: None,
            proxy_url: None,
            request_timeout_secs: None,
            emit_caller_trace_headers: true,
            max_retries: 3,
        };
        let region = resolve_region(&config);
        assert_eq!(region, "europe-west4");
    }
}
