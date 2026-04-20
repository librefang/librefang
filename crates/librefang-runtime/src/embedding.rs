//! Embedding driver for vector-based semantic memory.
//!
//! Provides an `EmbeddingDriver` trait and implementations:
//! - `OpenAIEmbeddingDriver` — works with any provider offering a `/v1/embeddings`
//!   endpoint (OpenAI, Groq, Together, Fireworks, Ollama, etc.).
//! - `BedrockEmbeddingDriver` — Amazon Bedrock embedding models via SigV4-signed
//!   REST calls (no heavy `aws-sdk-*` dependency).

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, warn};
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

/// Error type for embedding operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Missing API key: {0}")]
    MissingApiKey(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Unsupported: {0}")]
    Unsupported(String),
}

/// Configuration for creating an embedding driver.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Provider name (openai, groq, together, ollama, etc.).
    pub provider: String,
    /// Model name (e.g., "text-embedding-3-small", "all-MiniLM-L6-v2").
    pub model: String,
    /// API key (resolved from env var).
    pub api_key: String,
    /// Base URL for the API.
    pub base_url: String,
    /// Optional override for embedding dimensions.
    /// When set, this value is used instead of auto-inferring from the model name.
    pub dimensions_override: Option<usize>,
}

/// Trait for computing text embeddings.
#[async_trait]
pub trait EmbeddingDriver: Send + Sync {
    /// Compute embedding vectors for a batch of texts.
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Compute embedding for a single text.
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let results = self.embed(&[text]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::Parse("Empty embedding response".to_string()))
    }

    /// Return the dimensionality of embeddings produced by this driver.
    fn dimensions(&self) -> usize;

    /// Compute an embedding vector for raw image data.
    ///
    /// Returns `Err(EmbeddingError::Unsupported)` by default — drivers that
    /// support vision/multimodal models should override this.
    async fn embed_image(&self, _image_data: &[u8]) -> Result<Vec<f32>, EmbeddingError> {
        Err(EmbeddingError::Unsupported(
            "Image embeddings not supported by this driver".into(),
        ))
    }

    /// Whether this driver supports image embeddings.
    fn supports_images(&self) -> bool {
        false
    }
}

/// OpenAI-compatible embedding driver.
///
/// Works with any provider that implements the `/v1/embeddings` endpoint:
/// OpenAI, Groq, Together, Fireworks, Ollama, vLLM, LM Studio, etc.
pub struct OpenAIEmbeddingDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    model: String,
    client: reqwest::Client,
    dims: usize,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

impl OpenAIEmbeddingDriver {
    /// Create a new OpenAI-compatible embedding driver.
    pub fn new(config: EmbeddingConfig) -> Result<Self, EmbeddingError> {
        // Use explicit override if provided, otherwise infer from model name.
        let dims = config
            .dimensions_override
            .unwrap_or_else(|| infer_dimensions(&config.model));

        Ok(Self {
            api_key: Zeroizing::new(config.api_key),
            base_url: config.base_url,
            model: config.model,
            client: crate::http_client::proxied_client(),
            dims,
        })
    }
}

/// Infer embedding dimensions from model name.
fn infer_dimensions(model: &str) -> usize {
    match model {
        // OpenAI
        "text-embedding-3-small" => 1536,
        "text-embedding-3-large" => 3072,
        "text-embedding-ada-002" => 1536,
        // Sentence Transformers / local models
        "all-MiniLM-L6-v2" => 384,
        "all-MiniLM-L12-v2" => 384,
        "all-mpnet-base-v2" => 768,
        "nomic-embed-text" => 768,
        "mxbai-embed-large" => 1024,
        // Amazon Bedrock models
        "amazon.titan-embed-text-v1" => 1536,
        "amazon.titan-embed-text-v2:0" => 1024,
        "cohere.embed-english-v3" => 1024,
        "cohere.embed-multilingual-v3" => 1024,
        // Default to 1536 (most common)
        _ => 1536,
    }
}

#[async_trait]
impl EmbeddingDriver for OpenAIEmbeddingDriver {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let url = format!("{}/embeddings", self.base_url);
        let body = EmbedRequest {
            model: &self.model,
            input: texts,
        };

        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.as_str().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key.as_str()));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| EmbeddingError::Http(e.to_string()))?;
        let status = resp.status().as_u16();

        if status != 200 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(EmbeddingError::Api {
                status,
                message: body_text,
            });
        }

        let data: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| EmbeddingError::Parse(e.to_string()))?;

        // Update dimensions from actual response if available
        let embeddings: Vec<Vec<f32>> = data.data.into_iter().map(|d| d.embedding).collect();

        debug!(
            "Embedded {} texts (dims={})",
            embeddings.len(),
            embeddings.first().map(|e| e.len()).unwrap_or(0)
        );

        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

// ---------------------------------------------------------------------------
// Cohere embedding driver (native `/v2/embed` endpoint)
// ---------------------------------------------------------------------------

/// Cohere native embedding driver.
///
/// Cohere's embed API is **not** OpenAI-compatible — it uses a different
/// endpoint (`/v2/embed` vs. `/v1/embeddings`), a different request shape
/// (`texts` + required `input_type`), and v2 returns embeddings grouped by
/// embedding type (`{ "embeddings": { "float": [[...]] } }`) rather than a
/// flat array.  A dedicated driver is therefore necessary; sending Cohere
/// requests through `OpenAIEmbeddingDriver` produces 404s.
///
/// The driver targets Cohere's **v2** API for consistency with
/// `librefang-llm-drivers` (the chat driver also uses `api.cohere.com/v2`)
/// and because v2 is the path Cohere recommends for new integrations.
pub struct CohereEmbeddingDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    model: String,
    /// `input_type` sent to Cohere v3 models. Defaults to `search_document`
    /// (correct for indexing memory/documents, the primary path today) and
    /// can be overridden via the `LIBREFANG_COHERE_INPUT_TYPE` env var for
    /// deployments whose primary path is query-time embedding or
    /// clustering.  Invalid values are ignored with a warning because
    /// Cohere returns a cryptic 400 otherwise.
    input_type: String,
    client: reqwest::Client,
    dims: usize,
}

/// Valid values for Cohere v3 `input_type`, per the official API reference.
const COHERE_VALID_INPUT_TYPES: &[&str] = &[
    "search_document",
    "search_query",
    "classification",
    "clustering",
];

/// Resolve `input_type` for Cohere v3 models, applying the
/// `LIBREFANG_COHERE_INPUT_TYPE` env var override if present and valid.
fn resolve_cohere_input_type() -> String {
    match std::env::var("LIBREFANG_COHERE_INPUT_TYPE") {
        Ok(v) if COHERE_VALID_INPUT_TYPES.contains(&v.as_str()) => v,
        Ok(v) if !v.trim().is_empty() => {
            warn!(
                invalid = %v,
                valid = ?COHERE_VALID_INPUT_TYPES,
                "LIBREFANG_COHERE_INPUT_TYPE is not one of the valid Cohere input_type values; ignoring"
            );
            "search_document".to_string()
        }
        _ => "search_document".to_string(),
    }
}

#[derive(Serialize)]
struct CohereEmbedRequest<'a> {
    texts: &'a [&'a str],
    model: &'a str,
    input_type: &'a str,
    /// Required by the v2 API — tells Cohere which numeric representation to
    /// return. We only ever want `float`; the other options (`int8`, `uint8`,
    /// `binary`, `ubinary`, `base64`) are for bandwidth-sensitive deployments
    /// that we don't support yet.
    embedding_types: &'a [&'a str],
}

/// v2 response wraps embeddings in an object keyed by embedding type —
/// we always request `float`, so we extract `embeddings.float`.
#[derive(Deserialize)]
struct CohereEmbedResponse {
    embeddings: CohereEmbeddingsByType,
}

#[derive(Deserialize)]
struct CohereEmbeddingsByType {
    float: Vec<Vec<f32>>,
}

impl CohereEmbeddingDriver {
    /// Create a new Cohere embedding driver.
    ///
    /// Returns [`EmbeddingError::MissingApiKey`] when `config.api_key` is
    /// empty, because Cohere's API rejects unauthenticated calls outright
    /// and a misleading 401 at the first real `embed` call would be harder
    /// to diagnose than failing at boot.
    pub fn new(config: EmbeddingConfig) -> Result<Self, EmbeddingError> {
        if config.api_key.is_empty() {
            return Err(EmbeddingError::MissingApiKey(
                "Cohere embedding driver requires a non-empty API key (COHERE_API_KEY)".to_string(),
            ));
        }

        let dims = config
            .dimensions_override
            .unwrap_or_else(|| infer_cohere_dimensions(&config.model));

        Ok(Self {
            api_key: Zeroizing::new(config.api_key),
            base_url: config.base_url,
            model: config.model,
            input_type: resolve_cohere_input_type(),
            client: crate::http_client::proxied_client(),
            dims,
        })
    }
}

/// Cohere embed v3 model limit. Requests over this size get rejected by the
/// API; we fail fast with a clearer error instead of letting the server return
/// a cryptic 400.
const COHERE_EMBED_MAX_BATCH: usize = 96;

/// Infer embedding dimensions from a Cohere model name.
///
/// Cohere publishes these dimensions in their official model list.  The
/// default for unknown names is 1024, which matches the most common v3
/// models; callers who need a different size should pass
/// `dimensions_override` explicitly.
fn infer_cohere_dimensions(model: &str) -> usize {
    match model {
        "embed-english-v3.0" | "embed-multilingual-v3.0" => 1024,
        "embed-english-light-v3.0" | "embed-multilingual-light-v3.0" => 384,
        "embed-english-v2.0" => 4096,
        "embed-english-light-v2.0" => 1024,
        "embed-multilingual-v2.0" => 768,
        _ => 1024,
    }
}

#[async_trait]
impl EmbeddingDriver for CohereEmbeddingDriver {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        if texts.len() > COHERE_EMBED_MAX_BATCH {
            return Err(EmbeddingError::InvalidInput(format!(
                "Cohere embed API accepts at most {} texts per request (got {})",
                COHERE_EMBED_MAX_BATCH,
                texts.len()
            )));
        }

        let url = format!("{}/embed", self.base_url);
        let body = CohereEmbedRequest {
            texts,
            model: &self.model,
            input_type: &self.input_type,
            embedding_types: &["float"],
        };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key.as_str()))
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbeddingError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status != 200 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(EmbeddingError::Api {
                status,
                message: body_text,
            });
        }

        let data: CohereEmbedResponse = resp
            .json()
            .await
            .map_err(|e| EmbeddingError::Parse(e.to_string()))?;
        let embeddings = data.embeddings.float;

        debug!(
            "Cohere embedded {} texts (dims={})",
            embeddings.len(),
            embeddings.first().map(|e| e.len()).unwrap_or(0)
        );

        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

// ---------------------------------------------------------------------------
// Amazon Bedrock embedding driver (SigV4-signed REST calls)
// ---------------------------------------------------------------------------

/// Amazon Bedrock embedding driver.
///
/// Uses manual AWS SigV4 signing so we avoid pulling in the full `aws-sdk-*`
/// dependency tree.  Bedrock's embedding API is invoked per-text because the
/// Titan `/invoke` endpoint accepts a single `inputText` at a time.
pub struct BedrockEmbeddingDriver {
    client: reqwest::Client,
    region: String,
    model_id: String,
    access_key: Zeroizing<String>,
    secret_key: Zeroizing<String>,
    session_token: Option<Zeroizing<String>>,
    dims: usize,
}

/// Bedrock Titan invoke request body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BedrockEmbedRequest<'a> {
    input_text: &'a str,
}

/// Bedrock Titan invoke response body.
#[derive(Deserialize)]
struct BedrockEmbedResponse {
    embedding: Vec<f32>,
}

impl BedrockEmbeddingDriver {
    /// Create a new Bedrock embedding driver.
    ///
    /// Reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_REGION`
    /// from the environment (or the supplied overrides).
    pub fn new(
        model_id: String,
        region: Option<String>,
        dimensions_override: Option<usize>,
    ) -> Result<Self, EmbeddingError> {
        let access_key = std::env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| EmbeddingError::MissingApiKey("AWS_ACCESS_KEY_ID not set".to_string()))?;
        let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
            EmbeddingError::MissingApiKey("AWS_SECRET_ACCESS_KEY not set".to_string())
        })?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok().map(Zeroizing::new);
        let region = region
            .or_else(|| std::env::var("AWS_REGION").ok())
            .unwrap_or_else(|| "us-east-1".to_string());

        let dims = dimensions_override.unwrap_or_else(|| infer_dimensions(&model_id));

        Ok(Self {
            client: crate::http_client::proxied_client(),
            region,
            model_id,
            access_key: Zeroizing::new(access_key),
            secret_key: Zeroizing::new(secret_key),
            session_token,
            dims,
        })
    }

    /// Build the Bedrock invoke URL for the configured model and region.
    fn invoke_url(&self) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke",
            self.region, self.model_id
        )
    }
}

// ── Minimal AWS SigV4 helpers ───────────────────────────────────────────

/// Compute SHA-256 hex digest.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// HMAC-SHA256.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Derive the SigV4 signing key.
fn sigv4_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Build the full `Authorization` header value for an AWS SigV4 signed request.
///
/// This is a *minimal* implementation that covers the Bedrock invoke use-case
/// (POST, JSON body, no query-string parameters).
#[allow(clippy::too_many_arguments)]
fn sigv4_auth_header(
    access_key: &str,
    secret_key: &str,
    session_token: Option<&str>,
    region: &str,
    service: &str,
    host: &str,
    uri_path: &str,
    payload: &[u8],
    now: &chrono::DateTime<chrono::Utc>,
) -> (String, String, String) {
    let date_stamp = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

    let payload_hash = sha256_hex(payload);

    // Canonical headers (must be sorted). Include security token if present.
    let (canonical_headers, signed_headers) = if let Some(token) = session_token {
        (
            format!("content-type:application/json\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\nx-amz-security-token:{token}\n"),
            "content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
        )
    } else {
        (
            format!("content-type:application/json\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n"),
            "content-type;host;x-amz-content-sha256;x-amz-date",
        )
    };

    // Canonical request.
    let canonical_request =
        format!("POST\n{uri_path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

    let credential_scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let signing_key = sigv4_signing_key(secret_key, &date_stamp, region, service);
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    let auth = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    (auth, amz_date, payload_hash)
}

#[async_trait]
impl EmbeddingDriver for BedrockEmbeddingDriver {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let url = self.invoke_url();
        // Parse host and path from URL for signing.
        let parsed: url::Url = url
            .parse()
            .map_err(|e: url::ParseError| EmbeddingError::Http(e.to_string()))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| EmbeddingError::Http("no host in Bedrock URL".into()))?
            .to_string();
        let uri_path = parsed.path().to_string();

        let mut embeddings = Vec::with_capacity(texts.len());

        for &text in texts {
            let body = serde_json::to_vec(&BedrockEmbedRequest { input_text: text })
                .map_err(|e| EmbeddingError::Parse(e.to_string()))?;

            let now = chrono::Utc::now();
            let (auth, amz_date, payload_hash) = sigv4_auth_header(
                &self.access_key,
                &self.secret_key,
                self.session_token.as_ref().map(|s| s.as_str()),
                &self.region,
                "bedrock",
                &host,
                &uri_path,
                &body,
                &now,
            );

            let mut req = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Host", &host)
                .header("X-Amz-Date", &amz_date)
                .header("X-Amz-Content-Sha256", &payload_hash)
                .header("Authorization", &auth);
            if let Some(ref token) = self.session_token {
                req = req.header("X-Amz-Security-Token", token.as_str());
            }
            let resp = req
                .body(body)
                .send()
                .await
                .map_err(|e| EmbeddingError::Http(e.to_string()))?;

            let status = resp.status().as_u16();
            if status != 200 {
                let body_text = resp.text().await.unwrap_or_default();
                return Err(EmbeddingError::Api {
                    status,
                    message: body_text,
                });
            }

            let data: BedrockEmbedResponse = resp
                .json()
                .await
                .map_err(|e| EmbeddingError::Parse(e.to_string()))?;

            embeddings.push(data.embedding);
        }

        debug!(
            "Bedrock embedded {} texts (dims={})",
            embeddings.len(),
            embeddings.first().map(|e| e.len()).unwrap_or(0)
        );

        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

/// Probe environment variables and local services to detect an available
/// embedding provider.
///
/// Checks in priority order:
/// 1. `OPENAI_API_KEY`    → `"openai"`
/// 2. `GROQ_API_KEY`      → `"groq"`
/// 3. `MISTRAL_API_KEY`   → `"mistral"`
/// 4. `TOGETHER_API_KEY`  → `"together"`
/// 5. `FIREWORKS_API_KEY` → `"fireworks"`
/// 6. `COHERE_API_KEY`    → `"cohere"`
/// 7. `OLLAMA_HOST` set, or Ollama running on localhost → `"ollama"`
/// 8. `None` if nothing is available
pub fn detect_embedding_provider() -> Option<&'static str> {
    // Cloud providers — check API key env vars in priority order.
    let cloud_providers: &[(&str, &str)] = &[
        ("OPENAI_API_KEY", "openai"),
        ("OPENROUTER_API_KEY", "openrouter"),
        ("GROQ_API_KEY", "groq"),
        ("MISTRAL_API_KEY", "mistral"),
        ("TOGETHER_API_KEY", "together"),
        ("FIREWORKS_API_KEY", "fireworks"),
        ("COHERE_API_KEY", "cohere"),
    ];
    for &(env_var, provider) in cloud_providers {
        if let Ok(val) = std::env::var(env_var) {
            if !val.trim().is_empty() {
                return Some(provider);
            }
        }
    }

    // Local Ollama — available if OLLAMA_HOST is set and non-empty. We don't
    // attempt a live TCP probe here (that would be async and would require a
    // runtime); a non-empty env var is sufficient signal.
    if std::env::var("OLLAMA_HOST").is_ok_and(|v| !v.trim().is_empty()) {
        return Some("ollama");
    }

    None
}

/// Create an embedding driver from kernel config.
///
/// Pass `"auto"` as `provider` to invoke [`detect_embedding_provider`] and
/// pick the first available provider automatically.  Returns
/// `Err(EmbeddingError::MissingApiKey)` when `"auto"` is requested but no
/// provider can be detected.
pub fn create_embedding_driver(
    provider: &str,
    model: &str,
    api_key_env: &str,
    custom_base_url: Option<&str>,
    dimensions_override: Option<usize>,
) -> Result<Box<dyn EmbeddingDriver + Send + Sync>, EmbeddingError> {
    // Resolve "auto" to the first available provider.
    if provider == "auto" {
        let detected = detect_embedding_provider().ok_or_else(|| {
            EmbeddingError::MissingApiKey(
                "No embedding provider available. Set one of: OPENAI_API_KEY, GROQ_API_KEY, \
                 MISTRAL_API_KEY, TOGETHER_API_KEY, FIREWORKS_API_KEY, COHERE_API_KEY, \
                 or configure Ollama."
                    .to_string(),
            )
        })?;
        // Determine the API key env var for the detected provider.
        let resolved_key_env = if api_key_env.is_empty() {
            provider_default_key_env(detected)
        } else {
            api_key_env
        };
        return create_embedding_driver(
            detected,
            model,
            resolved_key_env,
            custom_base_url,
            dimensions_override,
        );
    }

    // Bedrock uses its own auth (SigV4) and endpoint format — handle early.
    if provider == "bedrock" {
        warn!(
            provider = %provider,
            model = %model,
            "Embedding driver configured to send data to AWS Bedrock — text content will leave this machine"
        );
        let region = custom_base_url
            .filter(|u| !u.is_empty())
            .map(|s| s.to_string());
        let driver = BedrockEmbeddingDriver::new(model.to_string(), region, dimensions_override)?;
        return Ok(Box::new(driver));
    }

    // Cohere uses its native `/v2/embed` endpoint with a different request
    // shape than OpenAI's `/v1/embeddings`. Handle it before the OpenAI-
    // compatible fall-through so a generic OpenAI driver doesn't 404.
    if provider == "cohere" {
        let resolved_key_env = if api_key_env.is_empty() {
            "COHERE_API_KEY"
        } else {
            api_key_env
        };
        let api_key = std::env::var(resolved_key_env).unwrap_or_default();
        if api_key.is_empty() {
            return Err(EmbeddingError::MissingApiKey(format!(
                "Cohere embedding driver requires {resolved_key_env} (currently unset or empty)"
            )));
        }

        // Model name fallback: auto-detect hands us the model from config,
        // which is often OpenAI-shaped ("text-embedding-3-small"). Cohere
        // rejects those with a 404 at request time, so transparently
        // substitute a sensible default and log a warn so the user notices
        // and can pin a real Cohere model in config.
        let cohere_model = if model.starts_with("embed-") {
            model.to_string()
        } else {
            warn!(
                provider = "cohere",
                requested_model = %model,
                fallback_model = "embed-english-v3.0",
                "Requested model is not a Cohere embed-* model; falling back to embed-english-v3.0. \
                 Set `[memory].embedding_model` in config.toml to pick a specific Cohere model."
            );
            "embed-english-v3.0".to_string()
        };

        let base_url = custom_base_url
            .filter(|u| !u.is_empty())
            .map(|u| u.trim_end_matches('/').to_string())
            .unwrap_or_else(|| "https://api.cohere.com/v2".to_string());

        warn!(
            provider = "cohere",
            base_url = %base_url,
            "Embedding driver configured to send data to external API — text content will leave this machine"
        );

        let config = EmbeddingConfig {
            provider: "cohere".to_string(),
            model: cohere_model,
            api_key,
            base_url,
            dimensions_override,
        };
        let driver = CohereEmbeddingDriver::new(config)?;
        return Ok(Box::new(driver));
    }

    let api_key = if api_key_env.is_empty() {
        String::new()
    } else {
        std::env::var(api_key_env).unwrap_or_default()
    };

    let base_url = custom_base_url
        .filter(|u| !u.is_empty())
        .map(|u| {
            let trimmed = u.trim_end_matches('/');
            // All OpenAI-compatible embedding providers need /v1 in the path.
            // If the user supplied a bare host URL (e.g. "http://192.168.0.1:11434"),
            // append /v1 so the final request hits {base}/v1/embeddings.
            let needs_v1 = matches!(
                provider,
                "openai"
                    | "openrouter"
                    | "groq"
                    | "together"
                    | "fireworks"
                    | "mistral"
                    | "ollama"
                    | "vllm"
                    | "lmstudio"
            );
            if needs_v1 && !trimmed.ends_with("/v1") {
                format!("{trimmed}/v1")
            } else {
                trimmed.to_string()
            }
        })
        .unwrap_or_else(|| match provider {
            "openai" => "https://api.openai.com/v1".to_string(),
            "openrouter" => "https://openrouter.ai/api/v1".to_string(),
            "groq" => "https://api.groq.com/openai/v1".to_string(),
            "together" => "https://api.together.xyz/v1".to_string(),
            "fireworks" => "https://api.fireworks.ai/inference/v1".to_string(),
            "mistral" => "https://api.mistral.ai/v1".to_string(),
            "ollama" => "http://localhost:11434/v1".to_string(),
            "vllm" => "http://localhost:8000/v1".to_string(),
            "lmstudio" => "http://localhost:1234/v1".to_string(),
            other => {
                warn!("Unknown embedding provider '{other}', using OpenAI-compatible format");
                format!("https://{other}/v1")
            }
        });

    // SECURITY: Warn when embedding requests will be sent to an external API
    let is_local = base_url.contains("localhost")
        || base_url.contains("127.0.0.1")
        || base_url.contains("[::1]");
    if !is_local {
        warn!(
            provider = %provider,
            base_url = %base_url,
            "Embedding driver configured to send data to external API — text content will leave this machine"
        );
    }

    let config = EmbeddingConfig {
        provider: provider.to_string(),
        model: model.to_string(),
        api_key,
        base_url,
        dimensions_override,
    };

    let driver = OpenAIEmbeddingDriver::new(config)?;
    Ok(Box::new(driver))
}

/// Return the default API-key environment variable name for a given provider.
fn provider_default_key_env(provider: &str) -> &'static str {
    match provider {
        "openai" => "OPENAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "groq" => "GROQ_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "together" => "TOGETHER_API_KEY",
        "fireworks" => "FIREWORKS_API_KEY",
        "cohere" => "COHERE_API_KEY",
        // Local providers don't need a key.
        _ => "",
    }
}

/// Compute cosine similarity between two vectors.
///
/// Returns a value in [-1.0, 1.0] where 1.0 = identical direction.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

/// Serialize an embedding vector to bytes (for SQLite BLOB storage).
pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize an embedding vector from bytes.
pub fn embedding_from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_real_vectors() {
        let a = vec![0.1, 0.2, 0.3, 0.4];
        let b = vec![0.1, 0.2, 0.3, 0.4];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-5);

        let c = vec![0.4, 0.3, 0.2, 0.1];
        let sim2 = cosine_similarity(&a, &c);
        assert!(sim2 > 0.0 && sim2 < 1.0); // Similar but not identical
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let sim = cosine_similarity(&[], &[]);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_cosine_similarity_length_mismatch() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_embedding_roundtrip() {
        let embedding = vec![0.1, -0.5, 1.23456, 0.0, -1e10, 1e10];
        let bytes = embedding_to_bytes(&embedding);
        let recovered = embedding_from_bytes(&bytes);
        assert_eq!(embedding.len(), recovered.len());
        for (a, b) in embedding.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn test_embedding_bytes_empty() {
        let bytes = embedding_to_bytes(&[]);
        assert!(bytes.is_empty());
        let recovered = embedding_from_bytes(&bytes);
        assert!(recovered.is_empty());
    }

    #[test]
    fn test_infer_dimensions() {
        assert_eq!(infer_dimensions("text-embedding-3-small"), 1536);
        assert_eq!(infer_dimensions("all-MiniLM-L6-v2"), 384);
        assert_eq!(infer_dimensions("nomic-embed-text"), 768);
        assert_eq!(infer_dimensions("unknown-model"), 1536); // default
    }

    #[test]
    fn test_create_embedding_driver_ollama() {
        // Should succeed even without API key (ollama is local)
        let driver = create_embedding_driver("ollama", "all-MiniLM-L6-v2", "", None, None);
        assert!(driver.is_ok());
        assert_eq!(driver.unwrap().dimensions(), 384);
    }

    #[test]
    fn test_create_embedding_driver_custom_url_with_v1() {
        // Custom URL already containing /v1 should be used as-is
        let driver = create_embedding_driver(
            "ollama",
            "nomic-embed-text",
            "",
            Some("http://192.168.0.1:11434/v1"),
            None,
        );
        assert!(driver.is_ok());
    }

    #[test]
    fn test_create_embedding_driver_custom_url_without_v1() {
        // Custom URL missing /v1 should get it appended for known providers
        let driver = create_embedding_driver(
            "ollama",
            "nomic-embed-text",
            "",
            Some("http://192.168.0.1:11434"),
            None,
        );
        assert!(driver.is_ok());
    }

    #[test]
    fn test_create_embedding_driver_custom_url_trailing_slash() {
        // Trailing slash should be trimmed before appending /v1
        let driver = create_embedding_driver(
            "ollama",
            "nomic-embed-text",
            "",
            Some("http://192.168.0.1:11434/"),
            None,
        );
        assert!(driver.is_ok());
    }

    #[test]
    fn test_create_embedding_driver_dimensions_override() {
        // Explicit dimensions override should take precedence over model inference
        let driver = create_embedding_driver("ollama", "all-MiniLM-L6-v2", "", None, Some(768));
        assert!(driver.is_ok());
        // all-MiniLM-L6-v2 normally infers 384, but override says 768
        assert_eq!(driver.unwrap().dimensions(), 768);
    }

    #[test]
    fn test_create_embedding_driver_dimensions_override_none() {
        // No override should fall back to model inference
        let driver = create_embedding_driver("ollama", "nomic-embed-text", "", None, None);
        assert!(driver.is_ok());
        assert_eq!(driver.unwrap().dimensions(), 768);
    }

    // ── Bedrock / SigV4 tests ──────────────────────────────────────────

    #[test]
    fn test_infer_dimensions_bedrock_models() {
        assert_eq!(infer_dimensions("amazon.titan-embed-text-v1"), 1536);
        assert_eq!(infer_dimensions("amazon.titan-embed-text-v2:0"), 1024);
        assert_eq!(infer_dimensions("cohere.embed-english-v3"), 1024);
        assert_eq!(infer_dimensions("cohere.embed-multilingual-v3"), 1024);
    }

    #[test]
    fn test_sha256_hex_empty() {
        // SHA-256 of empty string is a well-known constant.
        let hash = sha256_hex(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hex_hello() {
        let hash = sha256_hex(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_hmac_sha256_known_vector() {
        // RFC 4231 test case 2: key = "Jefe", data = "what do ya want for nothing?"
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let result = hmac_sha256(key, data);
        assert_eq!(
            hex::encode(&result),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // AWS example credentials from official documentation — NOT real secrets.
    // https://docs.aws.amazon.com/IAM/latest/UserGuide/id_credentials_access-keys.html
    const TEST_AWS_ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const TEST_AWS_SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    #[test]
    fn test_sigv4_signing_key_deterministic() {
        // Ensure the signing key derivation is deterministic.
        let key1 = sigv4_signing_key(TEST_AWS_SECRET_KEY, "20260322", "us-east-1", "bedrock");
        let key2 = sigv4_signing_key(TEST_AWS_SECRET_KEY, "20260322", "us-east-1", "bedrock");
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 32); // HMAC-SHA256 output is 32 bytes
    }

    #[test]
    fn test_sigv4_auth_header_format() {
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2026, 3, 22, 12, 0, 0).unwrap();
        let (auth, amz_date, payload_hash) = sigv4_auth_header(
            TEST_AWS_ACCESS_KEY,
            TEST_AWS_SECRET_KEY,
            None,
            "us-east-1",
            "bedrock",
            "bedrock-runtime.us-east-1.amazonaws.com",
            "/model/amazon.titan-embed-text-v2:0/invoke",
            b"{\"inputText\":\"hello\"}",
            &now,
        );

        let expected_prefix = format!("AWS4-HMAC-SHA256 Credential={TEST_AWS_ACCESS_KEY}/20260322/us-east-1/bedrock/aws4_request");
        assert!(auth.starts_with(&expected_prefix));
        assert!(auth.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date"));
        assert!(auth.contains("Signature="));
        assert_eq!(amz_date, "20260322T120000Z");
        assert_eq!(payload_hash, sha256_hex(b"{\"inputText\":\"hello\"}"));
    }

    #[test]
    fn test_create_embedding_driver_bedrock_missing_keys() {
        // Without AWS env vars set, bedrock driver creation should fail.
        // Temporarily ensure the vars are unset for this test.
        let had_key = std::env::var("AWS_ACCESS_KEY_ID").ok();
        let had_secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");

        let result =
            create_embedding_driver("bedrock", "amazon.titan-embed-text-v2:0", "", None, None);
        let err_msg = result.err().expect("expected Err").to_string();
        assert!(err_msg.contains("AWS_ACCESS_KEY_ID"));

        // Restore env vars if they were set.
        if let Some(v) = had_key {
            std::env::set_var("AWS_ACCESS_KEY_ID", v);
        }
        if let Some(v) = had_secret {
            std::env::set_var("AWS_SECRET_ACCESS_KEY", v);
        }
    }

    #[test]
    fn test_create_embedding_driver_bedrock_with_keys() {
        // Set fake AWS keys for this test.
        let had_key = std::env::var("AWS_ACCESS_KEY_ID").ok();
        let had_secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();
        let had_region = std::env::var("AWS_REGION").ok();
        std::env::set_var("AWS_ACCESS_KEY_ID", TEST_AWS_ACCESS_KEY);
        std::env::set_var("AWS_SECRET_ACCESS_KEY", TEST_AWS_SECRET_KEY);
        std::env::set_var("AWS_REGION", "us-west-2");

        let result =
            create_embedding_driver("bedrock", "amazon.titan-embed-text-v2:0", "", None, None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().dimensions(), 1024);

        // Restore env vars.
        match had_key {
            Some(v) => std::env::set_var("AWS_ACCESS_KEY_ID", v),
            None => std::env::remove_var("AWS_ACCESS_KEY_ID"),
        }
        match had_secret {
            Some(v) => std::env::set_var("AWS_SECRET_ACCESS_KEY", v),
            None => std::env::remove_var("AWS_SECRET_ACCESS_KEY"),
        }
        match had_region {
            Some(v) => std::env::set_var("AWS_REGION", v),
            None => std::env::remove_var("AWS_REGION"),
        }
    }

    #[test]
    fn test_bedrock_region_override_via_custom_base_url() {
        // When custom_base_url is passed for bedrock, it's treated as a region override.
        let had_key = std::env::var("AWS_ACCESS_KEY_ID").ok();
        let had_secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();
        std::env::set_var("AWS_ACCESS_KEY_ID", TEST_AWS_ACCESS_KEY);
        std::env::set_var("AWS_SECRET_ACCESS_KEY", TEST_AWS_SECRET_KEY);

        let result = create_embedding_driver(
            "bedrock",
            "amazon.titan-embed-text-v1",
            "",
            Some("eu-west-1"),
            None,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap().dimensions(), 1536);

        match had_key {
            Some(v) => std::env::set_var("AWS_ACCESS_KEY_ID", v),
            None => std::env::remove_var("AWS_ACCESS_KEY_ID"),
        }
        match had_secret {
            Some(v) => std::env::set_var("AWS_SECRET_ACCESS_KEY", v),
            None => std::env::remove_var("AWS_SECRET_ACCESS_KEY"),
        }
    }

    // ── Cohere native driver tests ─────────────────────────────────────

    #[test]
    fn test_infer_cohere_dimensions_v3() {
        assert_eq!(infer_cohere_dimensions("embed-english-v3.0"), 1024);
        assert_eq!(infer_cohere_dimensions("embed-multilingual-v3.0"), 1024);
        assert_eq!(infer_cohere_dimensions("embed-english-light-v3.0"), 384);
        assert_eq!(
            infer_cohere_dimensions("embed-multilingual-light-v3.0"),
            384
        );
    }

    #[test]
    fn test_infer_cohere_dimensions_v2_and_default() {
        assert_eq!(infer_cohere_dimensions("embed-english-v2.0"), 4096);
        assert_eq!(infer_cohere_dimensions("embed-english-light-v2.0"), 1024);
        assert_eq!(infer_cohere_dimensions("embed-multilingual-v2.0"), 768);
        // Unknown Cohere model names fall back to 1024 (the v3 default).
        assert_eq!(infer_cohere_dimensions("embed-some-future-model"), 1024);
    }

    #[test]
    fn test_cohere_driver_requires_api_key() {
        let cfg = EmbeddingConfig {
            provider: "cohere".to_string(),
            model: "embed-english-v3.0".to_string(),
            api_key: String::new(),
            base_url: "https://api.cohere.com/v2".to_string(),
            dimensions_override: None,
        };
        let err = CohereEmbeddingDriver::new(cfg).unwrap_err();
        assert!(
            matches!(err, EmbeddingError::MissingApiKey(_)),
            "expected MissingApiKey, got {err:?}"
        );
    }

    #[test]
    fn test_cohere_driver_dims_from_model() {
        let cfg = EmbeddingConfig {
            provider: "cohere".to_string(),
            model: "embed-english-light-v3.0".to_string(),
            api_key: "bogus".to_string(),
            base_url: "https://api.cohere.com/v2".to_string(),
            dimensions_override: None,
        };
        let driver = CohereEmbeddingDriver::new(cfg).unwrap();
        assert_eq!(driver.dimensions(), 384);
    }

    #[test]
    fn test_cohere_driver_dimensions_override() {
        let cfg = EmbeddingConfig {
            provider: "cohere".to_string(),
            model: "embed-english-v3.0".to_string(),
            api_key: "bogus".to_string(),
            base_url: "https://api.cohere.com/v2".to_string(),
            dimensions_override: Some(512),
        };
        let driver = CohereEmbeddingDriver::new(cfg).unwrap();
        assert_eq!(driver.dimensions(), 512);
    }

    #[tokio::test]
    async fn test_cohere_driver_empty_input_returns_empty() {
        let cfg = EmbeddingConfig {
            provider: "cohere".to_string(),
            model: "embed-english-v3.0".to_string(),
            api_key: "bogus".to_string(),
            base_url: "https://api.cohere.com/v2".to_string(),
            dimensions_override: None,
        };
        let driver = CohereEmbeddingDriver::new(cfg).unwrap();
        // Empty batch must not hit the network.
        let out = driver.embed(&[]).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_cohere_driver_batch_over_limit_fails_fast() {
        let cfg = EmbeddingConfig {
            provider: "cohere".to_string(),
            model: "embed-english-v3.0".to_string(),
            api_key: "bogus".to_string(),
            base_url: "https://api.cohere.com/v2".to_string(),
            dimensions_override: None,
        };
        let driver = CohereEmbeddingDriver::new(cfg).unwrap();
        let texts: Vec<&str> = vec!["x"; COHERE_EMBED_MAX_BATCH + 1];
        let err = driver.embed(&texts).await.unwrap_err();
        assert!(
            matches!(err, EmbeddingError::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
    }

    #[test]
    fn test_create_embedding_driver_cohere_missing_key() {
        let had = std::env::var("COHERE_API_KEY").ok();
        std::env::remove_var("COHERE_API_KEY");

        let result = create_embedding_driver("cohere", "embed-english-v3.0", "", None, None);
        assert!(matches!(result, Err(EmbeddingError::MissingApiKey(_))));

        if let Some(v) = had {
            std::env::set_var("COHERE_API_KEY", v);
        }
    }

    #[test]
    fn test_create_embedding_driver_cohere_with_key() {
        let had = std::env::var("COHERE_API_KEY").ok();
        std::env::set_var("COHERE_API_KEY", "test-cohere-key");

        let driver = create_embedding_driver("cohere", "embed-english-v3.0", "", None, None)
            .expect("cohere driver should build with key present");
        assert_eq!(driver.dimensions(), 1024);

        match had {
            Some(v) => std::env::set_var("COHERE_API_KEY", v),
            None => std::env::remove_var("COHERE_API_KEY"),
        }
    }

    #[test]
    fn test_create_embedding_driver_cohere_remaps_openai_model_name() {
        // When auto-detect sends us an OpenAI-flavored model name, the Cohere
        // branch should fall back to `embed-english-v3.0` rather than passing
        // the unknown name through (which would 404 at request time).
        let had = std::env::var("COHERE_API_KEY").ok();
        std::env::set_var("COHERE_API_KEY", "test-cohere-key");

        let driver =
            create_embedding_driver("cohere", "text-embedding-3-small", "", None, None).unwrap();
        // After remap, dims should match embed-english-v3.0 (1024), NOT the
        // 1536 that text-embedding-3-small would have inferred.
        assert_eq!(driver.dimensions(), 1024);

        match had {
            Some(v) => std::env::set_var("COHERE_API_KEY", v),
            None => std::env::remove_var("COHERE_API_KEY"),
        }
    }

    #[test]
    fn test_resolve_cohere_input_type_default() {
        let had = std::env::var("LIBREFANG_COHERE_INPUT_TYPE").ok();
        std::env::remove_var("LIBREFANG_COHERE_INPUT_TYPE");

        assert_eq!(resolve_cohere_input_type(), "search_document");

        if let Some(v) = had {
            std::env::set_var("LIBREFANG_COHERE_INPUT_TYPE", v);
        }
    }

    #[test]
    fn test_resolve_cohere_input_type_valid_override() {
        let had = std::env::var("LIBREFANG_COHERE_INPUT_TYPE").ok();
        std::env::set_var("LIBREFANG_COHERE_INPUT_TYPE", "search_query");

        assert_eq!(resolve_cohere_input_type(), "search_query");

        match had {
            Some(v) => std::env::set_var("LIBREFANG_COHERE_INPUT_TYPE", v),
            None => std::env::remove_var("LIBREFANG_COHERE_INPUT_TYPE"),
        }
    }

    #[test]
    fn test_resolve_cohere_input_type_invalid_falls_back() {
        let had = std::env::var("LIBREFANG_COHERE_INPUT_TYPE").ok();
        std::env::set_var("LIBREFANG_COHERE_INPUT_TYPE", "not-a-real-type");

        // Invalid values must not leak through to the Cohere API (it would
        // 400 with a cryptic message). Fall back to search_document.
        assert_eq!(resolve_cohere_input_type(), "search_document");

        match had {
            Some(v) => std::env::set_var("LIBREFANG_COHERE_INPUT_TYPE", v),
            None => std::env::remove_var("LIBREFANG_COHERE_INPUT_TYPE"),
        }
    }

    #[test]
    fn test_create_embedding_driver_cohere_custom_base_url() {
        // Users running behind a proxy / Cohere-compatible gateway should be
        // able to override the base URL.
        let had = std::env::var("COHERE_API_KEY").ok();
        std::env::set_var("COHERE_API_KEY", "test-cohere-key");

        let driver = create_embedding_driver(
            "cohere",
            "embed-english-v3.0",
            "",
            Some("https://cohere-proxy.internal/v2/"),
            None,
        )
        .unwrap();
        // Trailing slash must be stripped so `{base}/embed` is well-formed.
        assert_eq!(driver.dimensions(), 1024);

        match had {
            Some(v) => std::env::set_var("COHERE_API_KEY", v),
            None => std::env::remove_var("COHERE_API_KEY"),
        }
    }
}
