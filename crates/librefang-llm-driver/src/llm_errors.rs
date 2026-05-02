//! LLM error classification and sanitization.
//!
//! Classifies raw LLM API errors into 8 categories using pattern matching
//! against error messages and HTTP status codes. Handles error formats from
//! all 19+ providers LibreFang supports: Anthropic, OpenAI, Gemini, Groq,
//! DeepSeek, Mistral, Together, Fireworks, Ollama, vLLM, LM Studio,
//! Perplexity, Cohere, AI21, Cerebras, SambaNova, HuggingFace, XAI, Replicate.
//!
//! Pattern matching is done via case-insensitive substring checks with no
//! external regex dependency, keeping the crate dependency graph lean.

use serde::Serialize;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Classified LLM error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum LlmErrorCategory {
    /// 429, quota exceeded, too many requests.
    RateLimit,
    /// 503, overloaded, service unavailable, high demand.
    Overloaded,
    /// Request timeout, deadline exceeded, ETIMEDOUT, ECONNRESET.
    Timeout,
    /// 402, payment required, insufficient credits/balance.
    Billing,
    /// 401/403, invalid API key, unauthorized, forbidden.
    Auth,
    /// Context length exceeded, max tokens, context window.
    ContextOverflow,
    /// Invalid request format, malformed tool_use, schema violation.
    Format,
    /// Model not found, unknown model, NOT_FOUND.
    ModelNotFound,
}

/// Classified error with metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ClassifiedError {
    /// The classified category.
    pub category: LlmErrorCategory,
    /// `true` for RateLimit, Overloaded, Timeout.
    pub is_retryable: bool,
    /// `true` only for Billing.
    pub is_billing: bool,
    /// Retry delay parsed from the error message, if available.
    pub suggested_delay_ms: Option<u64>,
    /// User-safe message (no raw API details).
    pub sanitized_message: String,
    /// Original error message for logging.
    pub raw_message: String,
    /// Provider that produced this error (e.g., "openai", "anthropic", "groq").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model that was being used when the error occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Actionable suggestion for the user to resolve this error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

// ---------------------------------------------------------------------------
// Pattern tables (case-insensitive substring checks)
// ---------------------------------------------------------------------------

/// Context overflow patterns -- checked first because they are highly specific.
const CONTEXT_OVERFLOW_PATTERNS: &[&str] = &[
    "context_length_exceeded",
    "context length",
    "context_length",
    "maximum context",
    "context window",
    "token limit",
    "too many tokens",
    "max_tokens_exceeded",
    "max tokens exceeded",
    "prompt is too long",
    "input too long",
    "context.length",
];

/// Billing patterns.
const BILLING_PATTERNS: &[&str] = &[
    "payment required",
    "insufficient credits",
    "credit balance",
    "billing",
    "insufficient balance",
    "usage limit",
];

/// Auth patterns.
///
/// NOTE: These are intentionally specific to avoid false positives.
/// "forbidden" alone is too broad (Chinese providers return 403 + "forbidden"
/// for quota/region/model-permission issues, not just invalid API keys).
/// We rely on the 401 status-code fast-path for genuine auth failures and
/// only use patterns here as a fallback for status-less classification.
const AUTH_PATTERNS: &[&str] = &[
    "invalid api key",
    "invalid api_key",
    "invalid apikey",
    "incorrect api key",
    "invalid x-api-key",
    "invalid token",
    "unauthorized",
    "invalid_auth",
    "authentication_error",
    "authentication failed",
    "api key not found",
    "api key is missing",
    "invalid credentials",
    "not authenticated",
];

/// Patterns that indicate 403 is NOT an auth issue (quota, region, model
/// permission). Checked before falling back to Auth for status 403.
const FORBIDDEN_NON_AUTH_PATTERNS: &[&str] = &[
    "quota",
    "limit",
    "balance",
    "credit",
    "billing",
    "region",
    "not available",
    "not supported",
    "not allowed",
    "access denied", // model/resource access, not API key
    "permission",    // model permission, not API key auth
    "insufficient",
    "exceeded",
    "capacity",
    "blocked",
    "restricted",
    "not enabled",
    "does not exist",
    "model", // model-level 403 (e.g., "model access forbidden")
];

/// Rate-limit patterns.
const RATE_LIMIT_PATTERNS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "too many requests",
    "exceeded quota",
    "exceeded your quota",
    "resource exhausted",
    "resource_exhausted",
    "quota exceeded",
    "tokens per minute",
    "requests per minute",
    "tpm limit",
    "rpm limit",
];

/// Model-not-found patterns.
const MODEL_NOT_FOUND_PATTERNS: &[&str] = &[
    "model not found",
    "model_not_found",
    "unknown model",
    "does not exist",
    "not_found",
    "model unavailable",
    "model_unavailable",
    "no such model",
    "invalid model",
    "is not found",
];

/// Format / bad-request patterns (catch-all for 400-class issues).
const FORMAT_PATTERNS: &[&str] = &[
    "invalid request",
    "invalid_request",
    "malformed",
    "tool_use",
    "schema",
    "validation error",
    "validation_error",
    "invalid parameter",
    "invalid_parameter",
    "missing required",
    "bad request",
    "bad_request",
];

/// Overloaded patterns.
const OVERLOADED_PATTERNS: &[&str] = &[
    "overloaded",
    "overloaded_error",
    "service unavailable",
    "service_unavailable",
    "high demand",
    "capacity",
    "server_error",
    "internal server error",
    "internal_server_error",
];

/// Timeout / network patterns.
const TIMEOUT_PATTERNS: &[&str] = &[
    "timeout",
    "timed out",
    "deadline exceeded",
    "etimedout",
    "econnreset",
    "econnrefused",
    "econnaborted",
    "epipe",
    "ehostunreach",
    "enetunreach",
    "connection reset",
    "connection refused",
    "network error",
    "fetch failed",
];

/// SSL/TLS transient error patterns — these indicate a mid-stream record-layer
/// failure that is safe to retry (the server dropped the connection cleanly).
///
/// Note: `ssl handshake failure` is intentionally excluded — handshake failures
/// are configuration errors (wrong cert, protocol mismatch) that will fail again
/// on retry and are not transient network hiccups.
const SSL_TRANSIENT_PATTERNS: &[&str] = &[
    "bad record mac",
    "bad_record_mac",
    "ssl alert",
    "ssl_alert",
    "tls alert",
    "tls_alert",
    "tlsv1 alert",
    "sslv3 alert",
    "[ssl:",
];

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Check if `haystack` (lowercased) contains any pattern from `patterns`.
fn matches_any(haystack: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| haystack.contains(p))
}

/// Classify a raw error message + optional HTTP status into a category.
///
/// Priority order (most specific first):
/// 1. ContextOverflow  2. Billing (402)  3. Auth (401/403)
/// 4. RateLimit (429)  5. ModelNotFound  6. Format (400)
/// 7. Overloaded (503/500)  8. Timeout (network)
///
/// If nothing matches, falls back to `Format` for structured errors or
/// `Timeout` for network-sounding messages.
pub fn classify_error(message: &str, status: Option<u16>) -> ClassifiedError {
    let lower = message.to_lowercase();
    let delay = extract_retry_delay(message);

    // Helper to build ClassifiedError
    let build = |category: LlmErrorCategory| ClassifiedError {
        category,
        is_retryable: matches!(
            category,
            LlmErrorCategory::RateLimit | LlmErrorCategory::Overloaded | LlmErrorCategory::Timeout
        ),
        is_billing: category == LlmErrorCategory::Billing,
        suggested_delay_ms: delay,
        sanitized_message: sanitize_for_user(category, message),
        raw_message: message.to_string(),
        provider: None,
        model: None,
        suggestion: None,
    };

    // --- Status-code fast paths (some statuses are unambiguous) ---
    if let Some(code) = status {
        match code {
            429 => return build(LlmErrorCategory::RateLimit),
            402 => return build(LlmErrorCategory::Billing),
            401 => return build(LlmErrorCategory::Auth),
            403 => {
                // 403 can mean many things depending on provider:
                // - Rate limiting (Anthropic, some Chinese providers)
                // - Quota/billing exhausted
                // - Model access not enabled
                // - Region restrictions
                // Only classify as Auth if the message actually looks like an
                // API key problem; otherwise fall through to pattern matching.
                if matches_any(&lower, RATE_LIMIT_PATTERNS) {
                    return build(LlmErrorCategory::RateLimit);
                }
                if matches_any(&lower, BILLING_PATTERNS) {
                    return build(LlmErrorCategory::Billing);
                }
                if matches_any(&lower, CONTEXT_OVERFLOW_PATTERNS) {
                    return build(LlmErrorCategory::ContextOverflow);
                }
                if matches_any(&lower, MODEL_NOT_FOUND_PATTERNS) {
                    return build(LlmErrorCategory::ModelNotFound);
                }
                // If the 403 body mentions non-auth concepts (quota, region,
                // model permission, etc.), do NOT classify as Auth — fall
                // through to the general pattern-matching pipeline instead.
                if matches_any(&lower, FORBIDDEN_NON_AUTH_PATTERNS) {
                    // Don't return here — let the general pipeline classify it
                } else if matches_any(&lower, AUTH_PATTERNS) {
                    return build(LlmErrorCategory::Auth);
                } else {
                    // Generic 403 with no recognizable body: default Auth
                    return build(LlmErrorCategory::Auth);
                }
            }
            404 => return build(LlmErrorCategory::ModelNotFound),
            _ => {}
        }
    }

    // --- Pattern matching in priority order ---

    // 1. Context overflow (very specific patterns)
    if matches_any(&lower, CONTEXT_OVERFLOW_PATTERNS) {
        return build(LlmErrorCategory::ContextOverflow);
    }

    // 2. Billing
    if matches_any(&lower, BILLING_PATTERNS) {
        return build(LlmErrorCategory::Billing);
    }
    if status == Some(402) {
        return build(LlmErrorCategory::Billing);
    }

    // 3. Auth
    if matches_any(&lower, AUTH_PATTERNS) {
        return build(LlmErrorCategory::Auth);
    }
    // Note: 403 is NOT included here because it's fully handled in the
    // status-code fast-path above (where FORBIDDEN_NON_AUTH_PATTERNS can
    // redirect it to the general pipeline for non-auth 403s).
    if status == Some(401) {
        return build(LlmErrorCategory::Auth);
    }

    // 4. Rate limit
    if matches_any(&lower, RATE_LIMIT_PATTERNS) {
        return build(LlmErrorCategory::RateLimit);
    }
    if status == Some(429) {
        return build(LlmErrorCategory::RateLimit);
    }

    // 5. Model not found
    if matches_any(&lower, MODEL_NOT_FOUND_PATTERNS) {
        return build(LlmErrorCategory::ModelNotFound);
    }
    // Composite check: "model" + "not found" anywhere in the message
    if lower.contains("model") && lower.contains("not found") {
        return build(LlmErrorCategory::ModelNotFound);
    }

    // 6. Format / bad request (before overloaded, since 400 is more specific)
    if matches_any(&lower, FORMAT_PATTERNS) {
        return build(LlmErrorCategory::Format);
    }
    if status == Some(400) {
        return build(LlmErrorCategory::Format);
    }

    // 7. Overloaded
    if matches_any(&lower, OVERLOADED_PATTERNS) {
        return build(LlmErrorCategory::Overloaded);
    }
    if matches!(status, Some(500) | Some(503)) {
        return build(LlmErrorCategory::Overloaded);
    }

    // 8. Timeout / network
    if matches_any(&lower, TIMEOUT_PATTERNS) {
        return build(LlmErrorCategory::Timeout);
    }

    // --- HTML error page detection (Cloudflare etc.) ---
    if is_html_error_page(message) {
        return build(LlmErrorCategory::Overloaded);
    }

    // --- Fallback ---
    // If there's a status code in the 5xx range, treat as overloaded.
    if let Some(code) = status {
        if (500..600).contains(&code) {
            return build(LlmErrorCategory::Overloaded);
        }
        if (400..500).contains(&code) {
            return build(LlmErrorCategory::Format);
        }
    }

    // Last resort: if the message mentions network-like terms, call it timeout;
    // otherwise default to format (unknown structured error).
    if lower.contains("connect") || lower.contains("network") || lower.contains("dns") {
        build(LlmErrorCategory::Timeout)
    } else {
        build(LlmErrorCategory::Format)
    }
}

/// Classify an error with provider and model context for richer diagnostics.
///
/// This is the preferred entry point when provider/model information is available.
/// It enriches the classified error with actionable suggestions based on the
/// error category and provider context.
pub fn classify_error_with_context(
    message: &str,
    status: Option<u16>,
    provider: Option<&str>,
    model: Option<&str>,
) -> ClassifiedError {
    let mut classified = classify_error(message, status);
    classified.provider = provider.map(|s| s.to_string());
    classified.model = model.map(|s| s.to_string());
    classified.suggestion = generate_suggestion(classified.category, provider, model);

    // Enrich sanitized message with context
    if provider.is_some() || model.is_some() {
        classified.sanitized_message =
            enrich_message(&classified.sanitized_message, provider, model);
    }

    classified
}

/// Generate an actionable suggestion based on the error category and context.
fn generate_suggestion(
    category: LlmErrorCategory,
    provider: Option<&str>,
    model: Option<&str>,
) -> Option<String> {
    let suggestion = match category {
        LlmErrorCategory::RateLimit => {
            if let Some(p) = provider {
                format!(
                    "Consider upgrading your {} plan or reducing request frequency.",
                    p
                )
            } else {
                "Consider upgrading your API plan or reducing request frequency.".to_string()
            }
        }
        LlmErrorCategory::Billing => {
            if let Some(p) = provider {
                format!("Check your {} account balance and billing settings.", p)
            } else {
                "Check your account balance and billing settings.".to_string()
            }
        }
        LlmErrorCategory::Auth => {
            if let Some(p) = provider {
                format!(
                    "Verify your {} API key in config.toml or environment variables.",
                    p
                )
            } else {
                "Verify your API key in config.toml or environment variables.".to_string()
            }
        }
        LlmErrorCategory::ModelNotFound => {
            if let Some(m) = model {
                if let Some(p) = provider {
                    format!(
                        "Model '{}' may not be available on {}. Check available models with `librefang models list`.",
                        m, p
                    )
                } else {
                    format!(
                        "Model '{}' was not found. Check available models with `librefang models list`.",
                        m
                    )
                }
            } else {
                "Check the model name and availability with `librefang models list`.".to_string()
            }
        }
        LlmErrorCategory::ContextOverflow => {
            if let Some(m) = model {
                format!(
                    "The conversation exceeds {}'s context window. Try clearing history or using a model with a larger context.",
                    m
                )
            } else {
                "The conversation exceeds the model's context window. Try clearing history or switching to a model with a larger context.".to_string()
            }
        }
        LlmErrorCategory::Overloaded => {
            if let Some(p) = provider {
                format!(
                    "{} is temporarily overloaded. The request will be retried automatically.",
                    p
                )
            } else {
                "The provider is temporarily overloaded. The request will be retried automatically."
                    .to_string()
            }
        }
        LlmErrorCategory::Timeout => {
            return Some(
                "Check your network connection. If the issue persists, try a different provider."
                    .to_string(),
            );
        }
        LlmErrorCategory::Format => {
            return Some(
                "This may be a compatibility issue. Try updating LibreFang or switching models."
                    .to_string(),
            );
        }
    };
    Some(suggestion)
}

/// Enrich a sanitized error message with provider/model context.
fn enrich_message(base: &str, provider: Option<&str>, model: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(p) = provider {
        parts.push(format!("provider={}", p));
    }
    if let Some(m) = model {
        parts.push(format!("model={}", m));
    }
    if parts.is_empty() {
        base.to_string()
    } else {
        format!("{} [{}]", base, parts.join(", "))
    }
}

// ---------------------------------------------------------------------------
// Sanitization
// ---------------------------------------------------------------------------

/// Produce a user-friendly error message that includes a sanitized excerpt
/// of the raw provider error so users can actually diagnose problems.
///
/// Previous versions returned only a generic category message ("Verify your
/// API key") which made it impossible for users to tell what was wrong when
/// their keys were actually valid (issue #493).
pub fn sanitize_for_user(category: LlmErrorCategory, raw: &str) -> String {
    let prefix = match category {
        LlmErrorCategory::RateLimit => "Rate limited",
        LlmErrorCategory::Overloaded => "Provider overloaded",
        LlmErrorCategory::Timeout => "Request timed out",
        LlmErrorCategory::Billing => "Billing issue",
        LlmErrorCategory::Auth => "Auth error",
        LlmErrorCategory::ContextOverflow => "Context too long",
        LlmErrorCategory::Format => "Request failed",
        LlmErrorCategory::ModelNotFound => "Model not found",
    };

    let detail = sanitize_raw_excerpt(raw);
    if detail.is_empty() {
        // Fall back to a helpful generic message when there is no raw detail.
        match category {
            LlmErrorCategory::RateLimit => "Rate limited — retrying shortly.".to_string(),
            LlmErrorCategory::Overloaded => {
                "Provider temporarily overloaded — retrying.".to_string()
            }
            LlmErrorCategory::Timeout => {
                "Request timed out. Check your network connection.".to_string()
            }
            LlmErrorCategory::Billing => {
                "Billing issue. Check your API plan and balance.".to_string()
            }
            LlmErrorCategory::Auth => "Auth error. Check your API key configuration.".to_string(),
            LlmErrorCategory::ContextOverflow => {
                "Context too long for the model's context window.".to_string()
            }
            LlmErrorCategory::Format => {
                "Request failed. Check API key and model config.".to_string()
            }
            LlmErrorCategory::ModelNotFound => "Model not found. Check the model name.".to_string(),
        }
    } else {
        // Include the sanitized detail — cap total at 300 chars.
        let full = format!("{prefix}: {detail}");
        cap_message(&full, 300)
    }
}

/// Extract a safe excerpt from the raw error for display to the user.
///
/// Strips potential API key fragments (sk-xxx, key-xxx, Bearer xxx) and
/// truncates to avoid dumping huge HTML error pages.
fn sanitize_raw_excerpt(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }

    // If it looks like an HTML error page, don't show HTML to the user.
    if is_html_error_page(raw) {
        return "provider returned an error page (possible outage)".to_string();
    }

    // Try to extract the "message" field from JSON error bodies.
    let excerpt = extract_json_message(raw).unwrap_or_else(|| raw.to_string());

    // Strip anything that looks like a secret.
    let cleaned = redact_secrets(&excerpt);

    // Strip the "LLM driver error: API error (NNN): " wrapper if present —
    // the status code is already captured by the classifier.
    let cleaned = strip_llm_wrapper(&cleaned);

    // Cap length.
    cap_message(&cleaned, 200)
}

/// Try to pull `.error.message` or `.message` from a JSON error body.
fn extract_json_message(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    // OpenAI / most providers: {"error": {"message": "..."}}
    if let Some(msg) = v.pointer("/error/message").and_then(|v| v.as_str()) {
        return Some(msg.to_string());
    }
    // Anthropic: {"error": {"type": "...", "message": "..."}}
    if let Some(msg) = v.pointer("/message").and_then(|v| v.as_str()) {
        return Some(msg.to_string());
    }
    // Some providers: {"detail": "..."}
    if let Some(msg) = v.pointer("/detail").and_then(|v| v.as_str()) {
        return Some(msg.to_string());
    }
    None
}

/// Redact anything that looks like an API key or bearer token.
fn redact_secrets(s: &str) -> String {
    let mut result = s.to_string();
    // Common key prefixes: sk-..., key-..., Bearer ...
    // Replace sequences that look like keys (long alphanumeric after prefix).
    for prefix in &["sk-", "key-", "Bearer ", "bearer "] {
        while let Some(start) = result.find(prefix) {
            let end = result[start + prefix.len()..]
                .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .map(|i| start + prefix.len() + i)
                .unwrap_or(result.len());
            if end > start + prefix.len() + 4 {
                result.replace_range(start..end, "<redacted>");
            } else {
                break; // Avoid infinite loop on short matches
            }
        }
    }
    result
}

/// Strip the "LLM driver error: API error (NNN): " prefix if present.
fn strip_llm_wrapper(s: &str) -> String {
    // Pattern: "LLM driver error: API error (NNN): actual message"
    if let Some(idx) = s.find("API error (") {
        if let Some(close) = s[idx..].find("): ") {
            return s[idx + close + 3..].to_string();
        }
    }
    if let Some(rest) = s.strip_prefix("LLM driver error: ") {
        return rest.to_string();
    }
    s.to_string()
}

/// Cap a message at `max` chars, adding "..." if truncated.
fn cap_message(msg: &str, max: usize) -> String {
    if msg.chars().count() <= max {
        msg.to_string()
    } else {
        let take = max.saturating_sub(3);
        let end = msg
            .char_indices()
            .nth(take)
            .map(|(i, _)| i)
            .unwrap_or(msg.len());
        // Defense in depth: walk back to the nearest char boundary so we never
        // panic on multi-byte UTF-8 (CJK, emoji) input.
        let mut safe = end.min(msg.len());
        while safe > 0 && !msg.is_char_boundary(safe) {
            safe -= 1;
        }
        format!("{}...", &msg[..safe])
    }
}

// ---------------------------------------------------------------------------
// Retry-After extraction
// ---------------------------------------------------------------------------

/// Try to extract a retry delay (in milliseconds) from the error message.
///
/// Recognizes patterns like:
/// - `retry after 30` (seconds)
/// - `retry-after: 30` (seconds)
/// - `try again in 30` (seconds)
/// - `retry after 500ms` (milliseconds)
///
/// Returns `None` if no recognizable delay is found.
pub fn extract_retry_delay(message: &str) -> Option<u64> {
    let lower = message.to_lowercase();

    // Patterns to search for, each followed by a number.
    const PREFIXES: &[&str] = &["retry after ", "retry-after: ", "try again in "];

    for prefix in PREFIXES {
        if let Some(start) = lower.find(prefix) {
            let after = &lower[start + prefix.len()..];
            // Parse the leading digits.
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(value) = num_str.parse::<u64>() {
                if value == 0 {
                    continue;
                }
                // Check for "ms" suffix (milliseconds).
                let rest = &after[num_str.len()..];
                if rest.starts_with("ms") {
                    return Some(value);
                }
                // Default: treat as seconds, convert to ms.
                return Some(value.saturating_mul(1000));
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Transient error detection
// ---------------------------------------------------------------------------

/// Check if an error is likely transient (network hiccup, temporary overload).
///
/// This is a quick heuristic that does not require full classification.
pub fn is_transient(message: &str) -> bool {
    let lower = message.to_lowercase();
    matches_any(&lower, TIMEOUT_PATTERNS)
        || matches_any(&lower, OVERLOADED_PATTERNS)
        || matches_any(&lower, RATE_LIMIT_PATTERNS)
        || matches_any(&lower, SSL_TRANSIENT_PATTERNS)
}

// ---------------------------------------------------------------------------
// HTML / Cloudflare error detection
// ---------------------------------------------------------------------------

/// Detect if the response body is a Cloudflare error page or raw HTML
/// instead of expected JSON.
///
/// Checks for: `<!DOCTYPE`, `<html`, Cloudflare error codes (521-530),
/// `cf-error-code`.
pub fn is_html_error_page(body: &str) -> bool {
    let lower = body.to_lowercase();

    // HTML markers
    if lower.contains("<!doctype") || lower.contains("<html") {
        return true;
    }

    // Cloudflare error code header/attribute
    if lower.contains("cf-error-code") || lower.contains("cf-error-type") {
        return true;
    }

    // Cloudflare error status codes in text (e.g., "Error 522" or "522:")
    for code in 521..=530 {
        let code_str = code.to_string();
        if lower.contains(&code_str) && lower.contains("cloudflare") {
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// FailoverReason — provider-switching taxonomy
// ---------------------------------------------------------------------------

/// Why an LLM API call failed, used to decide the provider-switching strategy.
///
/// Mirrors the `FailoverReason` taxonomy from Hermes-Agent's `error_classifier.py`
/// but simplified to the eight variants that drive distinct recovery actions in
/// `librefang_llm_drivers::FallbackChain`:
///
/// | Variant           | HTTP hint            | Recovery                            |
/// |-------------------|----------------------|-------------------------------------|
/// | `RateLimit`       | 429 / "rate limit"   | sleep (optional hint ms), retry      |
/// | `CreditExhausted` | 402 / "credit"       | skip to next provider               |
/// | `ModelUnavailable`| 503 / 404            | skip to next provider               |
/// | `ContextTooLong`  | 413 / "context"      | propagate (caller must compress)   |
/// | `Timeout`         | network timeout      | skip to next provider               |
/// | `HttpError`       | other 4xx/5xx        | skip to next provider               |
/// | `AuthError`       | 401 / bad API key    | skip to next provider               |
/// | `Unknown`         | anything else        | propagate immediately               |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum FailoverReason {
    /// 429 or quota-based throttling — back off then retry same provider.
    /// Carries an optional retry delay hint (milliseconds).
    RateLimit(Option<u64>),
    /// 402 or confirmed credit exhaustion — rotate to next provider immediately.
    CreditExhausted,
    /// 413 or context window overflow — caller must compress, not retried here.
    ContextTooLong,
    /// 503 / 404 — provider or model temporarily unavailable, try next provider.
    ModelUnavailable,
    /// Connection or read timeout — try next provider.
    Timeout,
    /// Other HTTP 4xx/5xx error — skip to next provider.
    HttpError,
    /// 401 or invalid API key — skip to next provider (another slot may have a
    /// valid key).
    AuthError,
    /// Unclassifiable error — propagate to caller.
    Unknown,
}

// ---------------------------------------------------------------------------
// ProviderErrorCode — typed classification carried on `LlmError::Api`
// ---------------------------------------------------------------------------

/// Typed classification of a provider-returned API error, populated by the
/// driver when it parses the provider's structured error response (JSON
/// `error.code`, `error.type`, etc.). Carrying this lets `failover_reason()`
/// classify the error without doing case-insensitive substring matching on
/// the raw `message`, which silently breaks every time a provider rewords or
/// localizes its error strings (#3745).
///
/// `LlmError::Api { code: None, .. }` keeps the legacy substring-free
/// status-code-only fallback path in `failover_reason()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[non_exhaustive]
pub enum ProviderErrorCode {
    /// 429-equivalent — rate limit / quota throttling. Retry same provider.
    RateLimit,
    /// 402-equivalent — credit / billing / quota exhausted. Skip provider.
    CreditExhausted,
    /// Context window / token limit overflow. Caller must compress.
    ContextLengthExceeded,
    /// Requested model is not available on this provider slot.
    ModelNotFound,
    /// 401/403 authentication/permission failure on this slot.
    AuthError,
    /// Provider/model temporarily unavailable (503-equivalent).
    ServerUnavailable,
    /// Other server-side error (500-class) — skip to next provider.
    ServerError,
    /// Other client-side error (400-class) — skip to next provider.
    BadRequest,
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Classification tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_rate_limit() {
        // Standard 429
        let e = classify_error("Too Many Requests", Some(429));
        assert_eq!(e.category, LlmErrorCategory::RateLimit);
        assert!(e.is_retryable);

        // Pattern: "rate limit"
        let e = classify_error("You have hit the rate limit for this API", None);
        assert_eq!(e.category, LlmErrorCategory::RateLimit);

        // Pattern: "quota exceeded"
        let e = classify_error("Resource exhausted: quota exceeded", None);
        assert_eq!(e.category, LlmErrorCategory::RateLimit);

        // Pattern: "tokens per minute"
        let e = classify_error("You exceeded your tokens per minute limit", None);
        assert_eq!(e.category, LlmErrorCategory::RateLimit);

        // Pattern: "RPM"
        let e = classify_error("RPM limit reached, slow down", None);
        assert_eq!(e.category, LlmErrorCategory::RateLimit);
    }

    #[test]
    fn test_classify_overloaded() {
        let e = classify_error("The server is currently overloaded", None);
        assert_eq!(e.category, LlmErrorCategory::Overloaded);
        assert!(e.is_retryable);

        let e = classify_error("Service unavailable due to high demand", None);
        assert_eq!(e.category, LlmErrorCategory::Overloaded);

        // Status 503
        let e = classify_error("Please try again later", Some(503));
        assert_eq!(e.category, LlmErrorCategory::Overloaded);

        // Status 500
        let e = classify_error("Something went wrong", Some(500));
        assert_eq!(e.category, LlmErrorCategory::Overloaded);
    }

    #[test]
    fn test_classify_timeout() {
        let e = classify_error("ETIMEDOUT: request timed out", None);
        assert_eq!(e.category, LlmErrorCategory::Timeout);
        assert!(e.is_retryable);

        let e = classify_error("ECONNRESET", None);
        assert_eq!(e.category, LlmErrorCategory::Timeout);

        let e = classify_error("ECONNREFUSED: connection refused", None);
        assert_eq!(e.category, LlmErrorCategory::Timeout);

        let e = classify_error("fetch failed: network error", None);
        assert_eq!(e.category, LlmErrorCategory::Timeout);

        let e = classify_error("deadline exceeded while waiting for response", None);
        assert_eq!(e.category, LlmErrorCategory::Timeout);
    }

    #[test]
    fn test_classify_billing() {
        let e = classify_error("Payment required", Some(402));
        assert_eq!(e.category, LlmErrorCategory::Billing);
        assert!(e.is_billing);
        assert!(!e.is_retryable);

        let e = classify_error("Insufficient credits in your account", None);
        assert_eq!(e.category, LlmErrorCategory::Billing);

        let e = classify_error("Your credit balance is too low", None);
        assert_eq!(e.category, LlmErrorCategory::Billing);
    }

    #[test]
    fn test_classify_auth() {
        let e = classify_error("Invalid API key provided", Some(401));
        assert_eq!(e.category, LlmErrorCategory::Auth);
        assert!(!e.is_retryable);

        let e = classify_error("Forbidden: you do not have access", Some(403));
        assert_eq!(e.category, LlmErrorCategory::Auth);

        let e = classify_error("Incorrect API key format", None);
        assert_eq!(e.category, LlmErrorCategory::Auth);

        let e = classify_error("Authentication failed for this endpoint", None);
        assert_eq!(e.category, LlmErrorCategory::Auth);
    }

    #[test]
    fn test_classify_context_overflow() {
        let e = classify_error("This model's maximum context length is 128000 tokens", None);
        assert_eq!(e.category, LlmErrorCategory::ContextOverflow);

        let e = classify_error("context_length_exceeded", Some(400));
        assert_eq!(e.category, LlmErrorCategory::ContextOverflow);

        let e = classify_error("prompt is too long for the context window", None);
        assert_eq!(e.category, LlmErrorCategory::ContextOverflow);

        let e = classify_error("input too long: exceeds maximum context", None);
        assert_eq!(e.category, LlmErrorCategory::ContextOverflow);
    }

    #[test]
    fn test_classify_format() {
        let e = classify_error("Invalid request: missing 'messages' field", None);
        assert_eq!(e.category, LlmErrorCategory::Format);

        let e = classify_error("Malformed JSON in request body", None);
        assert_eq!(e.category, LlmErrorCategory::Format);

        let e = classify_error("Validation error: tool_use block missing id", None);
        assert_eq!(e.category, LlmErrorCategory::Format);

        // Status 400 without more specific patterns
        let e = classify_error("Something is wrong with your request", Some(400));
        assert_eq!(e.category, LlmErrorCategory::Format);
    }

    #[test]
    fn test_classify_model_not_found() {
        let e = classify_error("Model 'gpt-5-ultra' not found", None);
        assert_eq!(e.category, LlmErrorCategory::ModelNotFound);

        let e = classify_error("The model does not exist or you lack access", None);
        assert_eq!(e.category, LlmErrorCategory::ModelNotFound);

        let e = classify_error("Unknown model: claude-99", None);
        assert_eq!(e.category, LlmErrorCategory::ModelNotFound);
    }

    #[test]
    fn test_status_code_override() {
        // Even though message says "overloaded", status 429 wins
        let e = classify_error("server overloaded", Some(429));
        assert_eq!(e.category, LlmErrorCategory::RateLimit);

        // Status 402 overrides message
        let e = classify_error("something generic happened", Some(402));
        assert_eq!(e.category, LlmErrorCategory::Billing);

        // Status 401 overrides message
        let e = classify_error("generic error text", Some(401));
        assert_eq!(e.category, LlmErrorCategory::Auth);
    }

    #[test]
    fn test_retryable_categories() {
        // Retryable
        assert!(classify_error("rate limit", None).is_retryable);
        assert!(classify_error("overloaded", None).is_retryable);
        assert!(classify_error("timeout", None).is_retryable);

        // Not retryable
        assert!(!classify_error("", Some(402)).is_retryable); // Billing
        assert!(!classify_error("", Some(401)).is_retryable); // Auth
        assert!(!classify_error("context_length_exceeded", None).is_retryable); // ContextOverflow
        assert!(!classify_error("model not found", None).is_retryable); // ModelNotFound
    }

    #[test]
    fn test_billing_flag() {
        let e = classify_error("payment required", Some(402));
        assert!(e.is_billing);

        let e = classify_error("rate limit exceeded", None);
        assert!(!e.is_billing);

        let e = classify_error("insufficient credits", None);
        assert!(e.is_billing);
    }

    #[test]
    fn test_sanitize_messages() {
        // With raw detail — should include the raw excerpt in the message
        let msg = sanitize_for_user(LlmErrorCategory::RateLimit, "raw error details here");
        assert!(msg.contains("Rate limited"));
        assert!(msg.contains("raw error details here"));

        // Auth with API key in raw — key should be redacted
        let msg = sanitize_for_user(LlmErrorCategory::Auth, "sk-abc123xyz invalid key");
        assert!(msg.contains("Auth error"));
        assert!(!msg.contains("sk-abc123xyz"));
        assert!(msg.contains("<redacted>"));

        // Empty raw — fallback to generic
        let msg = sanitize_for_user(LlmErrorCategory::ContextOverflow, "");
        assert!(msg.contains("Context too long"));

        let msg = sanitize_for_user(LlmErrorCategory::ModelNotFound, "");
        assert!(msg.contains("Model not found"));

        // JSON error body — should extract the message field
        let msg = sanitize_for_user(
            LlmErrorCategory::Auth,
            r#"{"error":{"message":"Your API key is invalid","type":"auth_error"}}"#,
        );
        assert!(msg.contains("Your API key is invalid"));

        // All fallback messages (empty raw) should be under 300 chars
        for cat in [
            LlmErrorCategory::RateLimit,
            LlmErrorCategory::Overloaded,
            LlmErrorCategory::Timeout,
            LlmErrorCategory::Billing,
            LlmErrorCategory::Auth,
            LlmErrorCategory::ContextOverflow,
            LlmErrorCategory::Format,
            LlmErrorCategory::ModelNotFound,
        ] {
            let m = sanitize_for_user(cat, "");
            assert!(
                m.len() <= 300,
                "Fallback message for {:?} too long: {}",
                cat,
                m.len()
            );
        }
    }

    #[test]
    fn test_sanitize_redacts_secrets() {
        let msg = sanitize_raw_excerpt("Invalid key: sk-proj-abcdefg12345");
        assert!(!msg.contains("sk-proj-abcdefg12345"));
        assert!(msg.contains("<redacted>"));

        let msg = sanitize_raw_excerpt("Bearer eyJhbGciOiJIUzI1NiJ9 was rejected");
        assert!(!msg.contains("eyJhbGciOiJIUzI1NiJ9"));
    }

    #[test]
    fn test_sanitize_extracts_json_message() {
        let msg = sanitize_raw_excerpt(
            r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit"}}"#,
        );
        assert_eq!(msg, "Rate limit exceeded");
    }

    #[test]
    fn test_sanitize_html_page() {
        let msg = sanitize_raw_excerpt("<!DOCTYPE html><html><body>502 Bad Gateway</body></html>");
        assert!(msg.contains("error page"));
        assert!(!msg.contains("<html>"));
    }

    #[test]
    fn test_strip_llm_wrapper() {
        assert_eq!(
            strip_llm_wrapper("LLM driver error: API error (403): quota exceeded"),
            "quota exceeded"
        );
        assert_eq!(
            strip_llm_wrapper("LLM driver error: some other error"),
            "some other error"
        );
        assert_eq!(strip_llm_wrapper("plain error"), "plain error");
    }

    #[test]
    fn test_extract_retry_delay() {
        assert_eq!(
            extract_retry_delay("Rate limited. Retry after 30 seconds"),
            Some(30_000)
        );
        assert_eq!(extract_retry_delay("retry-after: 5"), Some(5_000));
        assert_eq!(
            extract_retry_delay("Please try again in 10 seconds"),
            Some(10_000)
        );
        assert_eq!(extract_retry_delay("Retry after 500ms"), Some(500));
    }

    #[test]
    fn test_extract_retry_delay_none() {
        assert_eq!(extract_retry_delay("Something went wrong"), None);
        assert_eq!(extract_retry_delay(""), None);
        assert_eq!(extract_retry_delay("rate limit exceeded"), None);
    }

    #[test]
    fn test_is_transient() {
        assert!(is_transient("Connection reset by peer"));
        assert!(is_transient("ECONNRESET"));
        assert!(is_transient("Request timed out after 30s"));
        assert!(is_transient("Service unavailable"));
        assert!(is_transient("rate limit exceeded"));

        // Non-transient
        assert!(!is_transient("invalid api key"));
        assert!(!is_transient("model not found"));
        assert!(!is_transient("context_length_exceeded"));
    }

    #[test]
    fn test_is_html_error_page() {
        assert!(is_html_error_page(
            "<!DOCTYPE html><html><body>Error</body></html>"
        ));
        assert!(is_html_error_page("<html lang='en'>502 Bad Gateway</html>"));
        assert!(!is_html_error_page(r#"{"error": "rate limit"}"#));
        assert!(!is_html_error_page("plain text error message"));
    }

    #[test]
    fn test_cloudflare_detection() {
        assert!(is_html_error_page(
            "<!DOCTYPE html><html><body>cloudflare 522 connection timed out</body></html>"
        ));
        assert!(is_html_error_page(
            "<html><head><meta cf-error-code='1015'></head></html>"
        ));
    }

    #[test]
    fn test_unknown_error_defaults() {
        // An error with no recognizable pattern and no status code
        let e = classify_error("??? something unknown ???", None);
        // Should default to Format (unknown structured error)
        assert_eq!(e.category, LlmErrorCategory::Format);

        // Network-sounding message without explicit pattern
        let e = classify_error("failed to connect to host", None);
        assert_eq!(e.category, LlmErrorCategory::Timeout);
    }

    #[test]
    fn test_gemini_specific_errors() {
        // Gemini model not found format
        let e = classify_error(
            "models/gemini-ultra is not found for API version v1beta",
            None,
        );
        assert_eq!(e.category, LlmErrorCategory::ModelNotFound);

        // Gemini overloaded
        let e = classify_error("The model is overloaded. Please try again later.", None);
        assert_eq!(e.category, LlmErrorCategory::Overloaded);

        // Gemini resource exhausted (rate limit)
        let e = classify_error("Resource exhausted: request rate limit exceeded", None);
        assert_eq!(e.category, LlmErrorCategory::RateLimit);
    }

    #[test]
    fn test_403_non_auth_classification() {
        // Chinese providers often return 403 for quota/region/model issues,
        // not auth problems. These should NOT be classified as Auth.

        // Quota exceeded with 403
        let e = classify_error("Quota exceeded for this model", Some(403));
        assert_ne!(e.category, LlmErrorCategory::Auth);
        assert_eq!(e.category, LlmErrorCategory::RateLimit);

        // Region restriction with 403
        let e = classify_error("This model is not available in your region", Some(403));
        assert_ne!(e.category, LlmErrorCategory::Auth);

        // Insufficient balance with 403 (e.g., Qwen/ZhiPu)
        let e = classify_error("Insufficient balance in your account", Some(403));
        assert_ne!(e.category, LlmErrorCategory::Auth);
        assert_eq!(e.category, LlmErrorCategory::Billing);

        // Model access not enabled with 403
        let e = classify_error("Model access is not enabled for your account", Some(403));
        assert_ne!(e.category, LlmErrorCategory::Auth);

        // Rate limit via 403 (some providers)
        let e = classify_error("Rate limit exceeded", Some(403));
        assert_eq!(e.category, LlmErrorCategory::RateLimit);

        // Genuine auth failure with 403
        let e = classify_error("Invalid API key or unauthorized access", Some(403));
        assert_eq!(e.category, LlmErrorCategory::Auth);

        // Generic 403 with no clues — defaults to Auth
        let e = classify_error("Forbidden", Some(403));
        assert_eq!(e.category, LlmErrorCategory::Auth);
    }

    #[test]
    fn test_anthropic_specific_errors() {
        // Anthropic overloaded_error
        let e = classify_error(
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            Some(529),
        );
        assert_eq!(e.category, LlmErrorCategory::Overloaded);

        // Anthropic rate limit
        let e = classify_error(
            "rate_limit_error: Number of request tokens has exceeded your per-minute rate limit",
            Some(429),
        );
        assert_eq!(e.category, LlmErrorCategory::RateLimit);

        // Anthropic invalid API key
        let e = classify_error(
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
            Some(401),
        );
        assert_eq!(e.category, LlmErrorCategory::Auth);
    }

    // -----------------------------------------------------------------------
    // Rich error context tests (issue #1008)
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_with_provider_context() {
        let e = classify_error_with_context(
            "rate limit exceeded",
            Some(429),
            Some("openai"),
            Some("gpt-4"),
        );
        assert_eq!(e.category, LlmErrorCategory::RateLimit);
        assert_eq!(e.provider, Some("openai".to_string()));
        assert_eq!(e.model, Some("gpt-4".to_string()));
        assert!(e.suggestion.is_some());
        assert!(e.sanitized_message.contains("openai"));
    }

    #[test]
    fn test_classify_with_model_not_found_context() {
        let e = classify_error_with_context(
            "model not found",
            Some(404),
            Some("anthropic"),
            Some("claude-99"),
        );
        assert_eq!(e.category, LlmErrorCategory::ModelNotFound);
        assert!(e.suggestion.unwrap().contains("claude-99"));
        assert!(e.sanitized_message.contains("anthropic"));
    }

    #[test]
    fn test_classify_without_context_backward_compatible() {
        let e = classify_error("rate limit exceeded", Some(429));
        assert_eq!(e.category, LlmErrorCategory::RateLimit);
        assert!(e.provider.is_none());
        assert!(e.model.is_none());
        assert!(e.suggestion.is_none());
    }

    #[test]
    fn test_enrich_message() {
        let msg = enrich_message("Rate limited", Some("openai"), Some("gpt-4"));
        assert_eq!(msg, "Rate limited [provider=openai, model=gpt-4]");

        let msg = enrich_message("Error", Some("groq"), None);
        assert_eq!(msg, "Error [provider=groq]");

        let msg = enrich_message("Error", None, None);
        assert_eq!(msg, "Error");
    }

    #[test]
    fn test_generate_suggestion_all_categories() {
        // Verify every category produces a suggestion
        for (cat, provider, model) in [
            (LlmErrorCategory::RateLimit, Some("openai"), Some("gpt-4")),
            (LlmErrorCategory::Billing, Some("anthropic"), None),
            (LlmErrorCategory::Auth, None, None),
            (
                LlmErrorCategory::ModelNotFound,
                Some("groq"),
                Some("llama-99"),
            ),
            (LlmErrorCategory::ContextOverflow, None, Some("gpt-4")),
            (LlmErrorCategory::Overloaded, Some("openai"), None),
            (LlmErrorCategory::Timeout, None, None),
            (LlmErrorCategory::Format, None, None),
        ] {
            let suggestion = generate_suggestion(cat, provider, model);
            assert!(suggestion.is_some(), "No suggestion for {:?}", cat);
        }
    }

    // -----------------------------------------------------------------------
    // cap_message — UTF-8 boundary safety
    // -----------------------------------------------------------------------

    #[test]
    fn cap_message_handles_cjk_without_panic() {
        // Each CJK char is 3 bytes; 200 chars in is mid-string.
        let input: String = "\u{4f60}\u{597d}\u{4e16}\u{754c}".repeat(100);
        // Must not panic and must end at a char boundary.
        let out = cap_message(&input, 50);
        assert!(out.ends_with("..."));
        let prefix = out.trim_end_matches("...");
        // The prefix must itself be valid UTF-8 (any slice on a non-boundary
        // would have panicked above before reaching this assertion).
        assert!(!prefix.is_empty());
        assert!(prefix.chars().count() <= 50);
    }

    #[test]
    fn cap_message_handles_emoji_without_panic() {
        let input: String = "\u{1f600}\u{1f601}\u{1f602}".repeat(50);
        let out = cap_message(&input, 30);
        assert!(out.ends_with("..."));
        assert!(out.trim_end_matches("...").chars().count() <= 30);
    }

    #[test]
    fn cap_message_short_max_does_not_underflow() {
        // max < 3 used to underflow `max - 3` and panic.
        let input = "hello world";
        let out = cap_message(input, 2);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn cap_message_within_limit_returns_unchanged() {
        let input = "\u{4f60}\u{597d}";
        assert_eq!(cap_message(input, 10), input);
    }
}
