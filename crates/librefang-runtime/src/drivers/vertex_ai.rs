//! Google Cloud Vertex AI driver.
//!
//! Uses the same Gemini generateContent API format but authenticates via
//! Google Cloud OAuth2 (service account JSON key or Application Default
//! Credentials) instead of API keys.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::llm_driver::{
    CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError, StreamEvent,
};

/// Vertex AI LLM driver.
pub struct VertexAiDriver {
    project_id: String,
    region: String,
    token_manager: Arc<RwLock<TokenManager>>,
    client: reqwest::Client,
}

// ─── OAuth2 token management ────────────────────────────────────────

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

enum CredentialSource {
    ServiceAccountJson(serde_json::Value),
    GcloudCli,
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

        // Build JWT header + claims.
        let header = base64_url_encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims_json = serde_json::json!({
            "iss": client_email,
            "scope": "https://www.googleapis.com/auth/cloud-platform",
            "aud": token_uri,
            "iat": iat,
            "exp": exp,
        });
        let claims = base64_url_encode(claims_json.to_string().as_bytes());
        let signing_input = format!("{header}.{claims}");

        // Sign with RS256.
        let signature = rsa_sha256_sign(private_key_pem, signing_input.as_bytes())?;
        let sig_b64 = base64_url_encode(&signature);
        let jwt = format!("{signing_input}.{sig_b64}");

        // Exchange JWT for access token.
        let client = crate::http_client::new_client();
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

// ─── RSA-SHA256 signing (pure Rust, no extra crates) ────────────────

/// URL-safe base64 encoding without padding.
fn base64_url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Parse a PEM-encoded PKCS#8 private key and sign data with RS256.
fn rsa_sha256_sign(pem: &str, data: &[u8]) -> Result<Vec<u8>, LlmError> {
    // Extract DER from PEM.
    let der = pem_to_der(pem)?;
    // Parse PKCS#8 to get RSA private key components.
    let (n, d) = parse_pkcs8_rsa(&der)?;
    // Hash with SHA-256.
    let digest = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(data);
        hasher.finalize()
    };
    // PKCS#1 v1.5 padding for SHA-256.
    let padded = pkcs1_v15_pad(&digest, n.len())?;
    // RSA raw sign: signature = padded^d mod n.
    let sig = modpow(&padded, &d, &n);
    Ok(sig)
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, LlmError> {
    use base64::Engine;
    let b64: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .map_err(|e| LlmError::Parse(format!("Failed to decode PEM base64: {e}")))
}

/// Minimal ASN.1/DER parser to extract RSA n and d from PKCS#8.
fn parse_pkcs8_rsa(der: &[u8]) -> Result<(Vec<u8>, Vec<u8>), LlmError> {
    // PKCS#8 structure:
    //   SEQUENCE {
    //     INTEGER (version)
    //     SEQUENCE { OID, ... }
    //     OCTET STRING -> PKCS#1 RSAPrivateKey
    //   }
    let parse_err = |msg: &str| LlmError::Parse(format!("PKCS#8 parse error: {msg}"));

    let inner = asn1_unwrap_sequence(der).map_err(|e| parse_err(&e))?;
    // Skip version INTEGER.
    let (_, rest) = asn1_read_element(inner).map_err(|e| parse_err(&e))?;
    // Skip algorithm SEQUENCE.
    let (_, rest) = asn1_read_element(rest).map_err(|e| parse_err(&e))?;
    // Read OCTET STRING containing PKCS#1.
    let (octet_bytes, _) = asn1_read_element(rest).map_err(|e| parse_err(&e))?;

    // Now parse PKCS#1 RSAPrivateKey:
    //   SEQUENCE { version, n, e, d, p, q, dp, dq, qinv }
    let rsa_inner = asn1_unwrap_sequence(octet_bytes).map_err(|e| parse_err(&e))?;
    // version
    let (_, rest) = asn1_read_element(rsa_inner).map_err(|e| parse_err(&e))?;
    // n (modulus)
    let (n_bytes, rest) = asn1_read_integer(rest).map_err(|e| parse_err(&e))?;
    // e (public exponent) - skip
    let (_, rest) = asn1_read_element(rest).map_err(|e| parse_err(&e))?;
    // d (private exponent)
    let (d_bytes, _) = asn1_read_integer(rest).map_err(|e| parse_err(&e))?;

    Ok((n_bytes, d_bytes))
}

fn asn1_unwrap_sequence(data: &[u8]) -> Result<&[u8], String> {
    if data.is_empty() || data[0] != 0x30 {
        return Err("Expected SEQUENCE tag (0x30)".into());
    }
    let (content, _) = asn1_read_content(&data[1..])?;
    Ok(content)
}

fn asn1_read_element(data: &[u8]) -> Result<(&[u8], &[u8]), String> {
    if data.is_empty() {
        return Err("Unexpected end of data".into());
    }
    let (content, rest) = asn1_read_content(&data[1..])?;
    Ok((content, rest))
}

fn asn1_read_integer(data: &[u8]) -> Result<(Vec<u8>, &[u8]), String> {
    if data.is_empty() || data[0] != 0x02 {
        return Err("Expected INTEGER tag (0x02)".into());
    }
    let (content, rest) = asn1_read_content(&data[1..])?;
    // Strip leading zero byte (sign byte).
    let trimmed = if !content.is_empty() && content[0] == 0x00 {
        &content[1..]
    } else {
        content
    };
    Ok((trimmed.to_vec(), rest))
}

fn asn1_read_content(data: &[u8]) -> Result<(&[u8], &[u8]), String> {
    if data.is_empty() {
        return Err("Unexpected end of data reading length".into());
    }
    let (len, offset) = if data[0] & 0x80 == 0 {
        (data[0] as usize, 1)
    } else {
        let num_bytes = (data[0] & 0x7F) as usize;
        if num_bytes > 4 || data.len() < 1 + num_bytes {
            return Err("Invalid ASN.1 length encoding".into());
        }
        let mut len: usize = 0;
        for i in 0..num_bytes {
            len = (len << 8) | (data[1 + i] as usize);
        }
        (len, 1 + num_bytes)
    };

    if data.len() < offset + len {
        return Err("ASN.1 content exceeds available data".into());
    }
    Ok((&data[offset..offset + len], &data[offset + len..]))
}

/// PKCS#1 v1.5 signature padding for SHA-256.
fn pkcs1_v15_pad(digest: &[u8], modulus_len: usize) -> Result<Vec<u8>, LlmError> {
    // DigestInfo prefix for SHA-256.
    let prefix: &[u8] = &[
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    let t_len = prefix.len() + digest.len();
    if modulus_len < t_len + 11 {
        return Err(LlmError::Parse("RSA modulus too short for signing".into()));
    }
    let ps_len = modulus_len - t_len - 3;
    let mut em = Vec::with_capacity(modulus_len);
    em.push(0x00);
    em.push(0x01);
    em.extend(std::iter::repeat_n(0xFF, ps_len));
    em.push(0x00);
    em.extend_from_slice(prefix);
    em.extend_from_slice(digest);
    Ok(em)
}

// ─── Big-integer modular exponentiation ─────────────────────────────

/// Compute base^exp mod modulus using big-integer arithmetic (byte arrays).
fn modpow(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    let n = BigUint::from_bytes(modulus);
    let b = BigUint::from_bytes(base);
    let e = BigUint::from_bytes(exp);
    let result = b.modpow(&e, &n);
    result.to_bytes(modulus.len())
}

/// Minimal big-unsigned-integer for RSA operations.
#[derive(Clone, Debug)]
struct BigUint {
    /// Digits stored in little-endian order, base 2^32.
    digits: Vec<u32>,
}

impl BigUint {
    fn zero() -> Self {
        Self { digits: vec![0] }
    }

    fn one() -> Self {
        Self { digits: vec![1] }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::zero();
        }
        let mut digits = Vec::with_capacity(bytes.len().div_ceil(4));
        let mut i = bytes.len();
        while i > 0 {
            let start = i.saturating_sub(4);
            let mut word: u32 = 0;
            for &b in &bytes[start..i] {
                word = (word << 8) | (b as u32);
            }
            digits.push(word);
            i = start;
        }
        // Trim leading zeros.
        while digits.len() > 1 && *digits.last().unwrap() == 0 {
            digits.pop();
        }
        Self { digits }
    }

    fn to_bytes(&self, min_len: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        for &d in self.digits.iter().rev() {
            bytes.push((d >> 24) as u8);
            bytes.push((d >> 16) as u8);
            bytes.push((d >> 8) as u8);
            bytes.push(d as u8);
        }
        // Strip leading zeros.
        let first_nonzero = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
        let mut result = bytes[first_nonzero..].to_vec();
        // Pad to min_len.
        while result.len() < min_len {
            result.insert(0, 0);
        }
        result
    }

    fn is_zero(&self) -> bool {
        self.digits.iter().all(|&d| d == 0)
    }

    fn bit_len(&self) -> usize {
        if self.is_zero() {
            return 0;
        }
        let top = *self.digits.last().unwrap();
        (self.digits.len() - 1) * 32 + (32 - top.leading_zeros() as usize)
    }

    fn bit(&self, i: usize) -> bool {
        let word_idx = i / 32;
        let bit_idx = i % 32;
        if word_idx >= self.digits.len() {
            false
        } else {
            (self.digits[word_idx] >> bit_idx) & 1 == 1
        }
    }

    /// self * other
    fn mul(&self, other: &BigUint) -> BigUint {
        let mut result = vec![0u32; self.digits.len() + other.digits.len()];
        for (i, &a) in self.digits.iter().enumerate() {
            let mut carry: u64 = 0;
            for (j, &b) in other.digits.iter().enumerate() {
                let prod = (a as u64) * (b as u64) + (result[i + j] as u64) + carry;
                result[i + j] = prod as u32;
                carry = prod >> 32;
            }
            result[i + other.digits.len()] += carry as u32;
        }
        while result.len() > 1 && *result.last().unwrap() == 0 {
            result.pop();
        }
        BigUint { digits: result }
    }

    /// self % modulus using shift-and-subtract.
    fn rem(&self, modulus: &BigUint) -> BigUint {
        if modulus.is_zero() {
            panic!("division by zero");
        }
        let mut remainder = self.clone();
        let mod_bits = modulus.bit_len();
        let self_bits = self.bit_len();
        if self_bits < mod_bits {
            return remainder;
        }

        let shift = self_bits - mod_bits;
        let mut shifted = modulus.shl(shift);
        for i in (0..=shift).rev() {
            if remainder.gte(&shifted) {
                remainder = remainder.sub(&shifted);
            }
            if i > 0 {
                shifted = shifted.shr1();
            }
        }
        remainder
    }

    /// Modular exponentiation: self^exp mod modulus.
    fn modpow(&self, exp: &BigUint, modulus: &BigUint) -> BigUint {
        if modulus.is_zero() {
            panic!("modulus cannot be zero");
        }
        let mut result = BigUint::one();
        let base = self.rem(modulus);
        let exp_bits = exp.bit_len();

        for i in (0..exp_bits).rev() {
            result = result.mul(&result).rem(modulus);
            if exp.bit(i) {
                result = result.mul(&base).rem(modulus);
            }
        }
        result
    }

    fn shl(&self, shift: usize) -> BigUint {
        let word_shift = shift / 32;
        let bit_shift = shift % 32;
        let mut digits = vec![0u32; self.digits.len() + word_shift + 1];
        for (i, &d) in self.digits.iter().enumerate() {
            digits[i + word_shift] |= d << bit_shift;
            if bit_shift > 0 {
                digits[i + word_shift + 1] |= d >> (32 - bit_shift);
            }
        }
        while digits.len() > 1 && *digits.last().unwrap() == 0 {
            digits.pop();
        }
        BigUint { digits }
    }

    fn shr1(&self) -> BigUint {
        let mut digits = vec![0u32; self.digits.len()];
        for i in 0..self.digits.len() {
            digits[i] = self.digits[i] >> 1;
            if i + 1 < self.digits.len() {
                digits[i] |= self.digits[i + 1] << 31;
            }
        }
        while digits.len() > 1 && *digits.last().unwrap() == 0 {
            digits.pop();
        }
        BigUint { digits }
    }

    fn gte(&self, other: &BigUint) -> bool {
        if self.digits.len() != other.digits.len() {
            return self.digits.len() > other.digits.len();
        }
        for i in (0..self.digits.len()).rev() {
            if self.digits[i] != other.digits[i] {
                return self.digits[i] > other.digits[i];
            }
        }
        true // equal
    }

    fn sub(&self, other: &BigUint) -> BigUint {
        let mut digits = vec![0u32; self.digits.len()];
        let mut borrow: i64 = 0;
        for i in 0..self.digits.len() {
            let a = self.digits[i] as i64;
            let b = if i < other.digits.len() {
                other.digits[i] as i64
            } else {
                0
            };
            let diff = a - b - borrow;
            if diff < 0 {
                digits[i] = (diff + (1i64 << 32)) as u32;
                borrow = 1;
            } else {
                digits[i] = diff as u32;
                borrow = 0;
            }
        }
        while digits.len() > 1 && *digits.last().unwrap() == 0 {
            digits.pop();
        }
        BigUint { digits }
    }
}

// ─── Driver construction ────────────────────────────────────────────

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
            client: crate::http_client::new_client(),
        })
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
        );

        let mut last_error = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500 * (1 << attempt))).await;
            }

            let token = self.token_manager.write().await.get_token().await?;

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status();

            if status.is_success() {
                let resp_body = resp
                    .text()
                    .await
                    .map_err(|e| LlmError::Http(e.to_string()))?;
                return super::gemini::parse_and_convert_response(&resp_body);
            }

            let resp_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                last_error = Some(LlmError::RateLimited {
                    retry_after_ms: 1000 * (1 << attempt),
                });
                continue;
            }
            if status.as_u16() == 503 {
                last_error = Some(LlmError::Overloaded {
                    retry_after_ms: 1000 * (1 << attempt),
                });
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
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::Http("Max retries exceeded".into())))
    }

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
        );

        let mut last_error = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500 * (1 << attempt))).await;
            }

            let token = self.token_manager.write().await.get_token().await?;

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;

            let status = resp.status();

            if status.is_success() {
                return super::gemini::stream_gemini_sse(resp, tx).await;
            }

            let resp_body = resp.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                last_error = Some(LlmError::RateLimited {
                    retry_after_ms: 1000 * (1 << attempt),
                });
                continue;
            }
            if status.as_u16() == 503 {
                last_error = Some(LlmError::Overloaded {
                    retry_after_ms: 1000 * (1 << attempt),
                });
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
            });
        }

        Err(last_error.unwrap_or_else(|| LlmError::Http("Max retries exceeded".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_url_encode() {
        let result = base64_url_encode(b"hello world");
        assert_eq!(result, "aGVsbG8gd29ybGQ");
        // No padding characters.
        assert!(!result.contains('='));
    }

    #[test]
    fn test_biguint_basic() {
        let zero = BigUint::zero();
        assert!(zero.is_zero());

        let one = BigUint::one();
        assert!(!one.is_zero());
        assert_eq!(one.bit_len(), 1);

        let n = BigUint::from_bytes(&[0x01, 0x00]);
        assert_eq!(n.bit_len(), 9);
        let bytes = n.to_bytes(2);
        assert_eq!(bytes, vec![0x01, 0x00]);
    }

    #[test]
    fn test_biguint_modpow() {
        // 3^4 mod 5 = 81 mod 5 = 1
        let base = BigUint::from_bytes(&[3]);
        let exp = BigUint::from_bytes(&[4]);
        let modulus = BigUint::from_bytes(&[5]);
        let result = base.modpow(&exp, &modulus);
        assert_eq!(result.to_bytes(1), vec![1]);
    }

    #[test]
    fn test_endpoint_url() {
        let driver = VertexAiDriver {
            project_id: "my-project".to_string(),
            region: "us-central1".to_string(),
            token_manager: Arc::new(RwLock::new(TokenManager::new(CredentialSource::GcloudCli))),
            client: crate::http_client::new_client(),
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
            skip_permissions: true,
        };
        let region = resolve_region(&config);
        assert_eq!(region, "us-central1");
    }

    #[test]
    fn test_pkcs1_v15_pad_length() {
        let digest = [0u8; 32]; // SHA-256 output
        let padded = pkcs1_v15_pad(&digest, 256).unwrap();
        assert_eq!(padded.len(), 256);
        assert_eq!(padded[0], 0x00);
        assert_eq!(padded[1], 0x01);
    }
}
