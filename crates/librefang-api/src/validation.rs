//! Centralized input validation for the LibreFang API.
//!
//! Provides:
//! - `ValidatedJson<T>` extractor that enforces common validation rules
//! - Request body size limiting via tower `RequestBodyLimitLayer`
//! - Consistent JSON error responses for all validation failures

use axum::extract::rejection::JsonRejection;
use axum::extract::FromRequest;
use axum::http::StatusCode;
use axum::Json;
use serde::de::DeserializeOwned;

use crate::types::ApiErrorResponse;

/// Maximum allowed request body size (1 MB).
///
/// Individual endpoints may enforce tighter limits (e.g. message size),
/// but this provides a global safety net against oversized payloads.
pub const MAX_REQUEST_BODY_BYTES: usize = 1_024 * 1_024;

/// Maximum allowed length for a single string field (in characters).
/// Fields that legitimately need more (e.g. manifest_toml, message bodies)
/// should use endpoint-specific checks instead.
pub const MAX_STRING_FIELD_LEN: usize = 10_000;

/// Maximum nesting depth allowed in arbitrary JSON values.
pub const MAX_JSON_DEPTH: usize = 20;

/// Maximum chat-message body size in UTF-8 bytes.
///
/// Memory-safety cap: prevents a single oversized request from
/// allocating an arbitrarily-large `String` inside the handler.
/// Tuned to 256 KiB (4× the historical 64 KiB) so the
/// complementary character cap (see [`MAX_MESSAGE_CHARS`]) governs
/// LLM-cost protection without unfairly clipping CJK users, who
/// pay ~3 bytes per glyph.
///
/// Audit: message-byte-vs-char-cap. Historical `MAX_MESSAGE_SIZE
/// = 64 * 1024` was 64 KiB regardless of script; CJK users hit
/// the cap at ~21 K characters while ASCII users got 64 K.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// Maximum chat-message body size in unicode scalar values
/// (`str::chars().count()`).
///
/// LLM-cost protection: each character roughly maps to a token (or
/// a small multiple in CJK / emoji), so this cap stops a single
/// request from dominating an agent's token budget regardless of
/// the encoding. 100 K characters sits well above any realistic
/// human-typed prompt and below LLM context windows.
///
/// Used alongside [`MAX_MESSAGE_BYTES`]: both must be satisfied.
pub const MAX_MESSAGE_CHARS: usize = 100_000;

/// Maximum size of a `PromptVersion.system_prompt` field in UTF-8 bytes.
///
/// LLM-cost-amplification guard: once a `PromptVersion` is activated,
/// every subsequent LLM call carries its `system_prompt` verbatim, so an
/// uncapped field is a direct money-loss vector against the operator's
/// LLM bill. The 1 MB `RequestBodyLimitLayer` is too loose to act as a
/// safety net here — a 32 KiB cap sits well above any realistic hand-
/// authored prompt and well below LLM context-window saturation.
///
/// Audit: `docs/issues/prompt-version-system-prompt-no-cap.md`.
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 32 * 1024;

/// Maximum size of a `PromptVersion.system_prompt` field in unicode
/// scalar values (`str::chars().count()`).
///
/// Complementary cap to [`MAX_SYSTEM_PROMPT_BYTES`]: ASCII prompts hit
/// the byte cap first, but a CJK-heavy prompt encodes at ~3 bytes per
/// glyph and a billable character is closer to one token regardless of
/// script. This cap ensures the per-call token cost is bounded in both
/// encodings.
pub const MAX_SYSTEM_PROMPT_CHARS: usize = 16 * 1024;

/// Validate that a `PromptVersion.system_prompt` fits inside both
/// [`MAX_SYSTEM_PROMPT_BYTES`] and [`MAX_SYSTEM_PROMPT_CHARS`]. Returns
/// a `ValidationError` whose body includes both counts plus the
/// configured caps so operators can diagnose which limit triggered.
/// Returns `Ok(())` when both checks pass.
///
/// Audit: `docs/issues/prompt-version-system-prompt-no-cap.md`.
pub fn check_system_prompt_size(system_prompt: &str) -> Result<(), ValidationError> {
    let bytes = system_prompt.len();
    if bytes > MAX_SYSTEM_PROMPT_BYTES {
        let chars = system_prompt.chars().count();
        return Err(ValidationError {
            status: StatusCode::BAD_REQUEST,
            message: format!(
                "system_prompt exceeds maximum byte length: {bytes} bytes / {chars} chars \
                 exceeds byte cap of {MAX_SYSTEM_PROMPT_BYTES} bytes"
            ),
        });
    }
    let chars = system_prompt.chars().count();
    if chars > MAX_SYSTEM_PROMPT_CHARS {
        return Err(ValidationError {
            status: StatusCode::BAD_REQUEST,
            message: format!(
                "system_prompt exceeds maximum character length: {chars} chars / {bytes} bytes \
                 exceeds character cap of {MAX_SYSTEM_PROMPT_CHARS} chars"
            ),
        });
    }
    Ok(())
}

/// Validate that a chat-message body fits inside both
/// [`MAX_MESSAGE_BYTES`] and [`MAX_MESSAGE_CHARS`]. Returns a
/// `ValidationError` whose body includes both counts plus the
/// configured caps so operators can diagnose which limit triggered.
/// Returns `Ok(())` when both checks pass.
///
/// Audit: message-byte-vs-char-cap.
pub fn check_message_size(message: &str) -> Result<(), ValidationError> {
    let bytes = message.len();
    if bytes > MAX_MESSAGE_BYTES {
        let chars = message.chars().count();
        return Err(ValidationError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: format!(
                "message body too large: {bytes} bytes / {chars} chars exceeds byte cap \
                 of {MAX_MESSAGE_BYTES} bytes"
            ),
        });
    }
    let chars = message.chars().count();
    if chars > MAX_MESSAGE_CHARS {
        return Err(ValidationError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: format!(
                "message body too long: {chars} chars / {bytes} bytes exceeds character cap \
                 of {MAX_MESSAGE_CHARS} chars"
            ),
        });
    }
    Ok(())
}

/// Validate that a bulk-endpoint array is non-empty and within `limit`.
///
/// Bulk handlers must call this **before** allocating any per-item
/// buffer (e.g. `Vec::with_capacity(req.ids.len())`). Even within the
/// 8 MiB global request-body cap, an attacker can craft an array of
/// empty strings that would otherwise cause millions of pre-allocated
/// entries. Issue `docs/issues/bulk-with-capacity-no-validate.md`.
///
/// Each call site supplies its own per-route `limit` constant — the
/// approvals lane historically allows 100, agents bulk allows 50, etc.
/// Lowering caps silently to a single shared value would break operator
/// workflows, so the cap is the caller's choice.
///
/// Returns a tuple-shape error rather than a [`ValidationError`] so
/// existing handlers that already build `(StatusCode, Json<Value>)`
/// can `let Err(resp) = ...; return resp;` without further conversion.
pub fn validate_bulk_size(
    len: usize,
    limit: usize,
) -> Result<(), (StatusCode, axum::Json<serde_json::Value>)> {
    if len == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "bulk array is empty"})),
        ));
    }
    if len > limit {
        return Err((
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": format!("bulk array size {len} exceeds maximum {limit}"),
            })),
        ));
    }
    Ok(())
}

// ── Error type ──────────────────────────────────────────────────────

/// A validation error returned as a consistent JSON body.
///
/// This is a thin wrapper around [`ApiErrorResponse`] that always includes
/// `code: "validation_error"` for backward compatibility.
#[derive(Debug)]
pub struct ValidationError {
    pub status: StatusCode,
    pub message: String,
}

impl axum::response::IntoResponse for ValidationError {
    fn into_response(self) -> axum::response::Response {
        ApiErrorResponse {
            error: self.message,
            code: Some("validation_error".to_string()),
            r#type: Some("validation_error".to_string()),
            details: None,
            request_id: None,
            status: self.status,
        }
        .into_response()
    }
}

impl ValidationError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    pub fn payload_too_large(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: msg.into(),
        }
    }
}

// ── ValidatedJson extractor ─────────────────────────────────────────

/// Drop-in replacement for `axum::Json<T>` that returns a consistent
/// `{"error": "...", "type": "validation_error"}` on deserialization failure
/// instead of axum's default plain-text rejection.
///
/// Route handlers can swap `Json<T>` → `ValidatedJson<T>` with no other changes.
pub struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + 'static,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
{
    type Rejection = ValidationError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ValidatedJson(value)),
            Err(rejection) => {
                let message = match rejection {
                    JsonRejection::JsonDataError(ref e) => {
                        format!("Invalid JSON data: {e}")
                    }
                    JsonRejection::JsonSyntaxError(ref e) => {
                        format!("Malformed JSON: {e}")
                    }
                    JsonRejection::MissingJsonContentType(_) => {
                        "Missing Content-Type: application/json header".to_string()
                    }
                    JsonRejection::BytesRejection(_) => "Failed to read request body".to_string(),
                    other => {
                        format!("JSON rejection: {other}")
                    }
                };
                Err(ValidationError::bad_request(message))
            }
        }
    }
}

// ── Validation helpers ──────────────────────────────────────────────

/// Validate that a string field does not exceed `max_len` characters.
/// Returns `Ok(())` or a `ValidationError` naming the offending field.
pub fn check_string_length(
    field_name: &str,
    value: &str,
    max_len: usize,
) -> Result<(), ValidationError> {
    if value.chars().count() > max_len {
        Err(ValidationError::bad_request(format!(
            "Field '{field_name}' exceeds maximum length of {max_len} characters"
        )))
    } else {
        Ok(())
    }
}

/// Validate that a string field is not empty (after trimming whitespace).
pub fn check_not_empty(field_name: &str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(ValidationError::bad_request(format!(
            "Field '{field_name}' must not be empty"
        )))
    } else {
        Ok(())
    }
}

/// Check JSON nesting depth does not exceed `max_depth`.
/// Useful for `serde_json::Value` payloads that could be arbitrarily nested.
pub fn check_json_depth(
    value: &serde_json::Value,
    max_depth: usize,
) -> Result<(), ValidationError> {
    // Iterative depth check using an explicit stack to avoid stack overflow
    // on adversarial input with extreme nesting.
    let mut stack: Vec<(&serde_json::Value, usize)> = vec![(value, 0)];
    let mut max_seen: usize = 0;

    while let Some((v, current_depth)) = stack.pop() {
        if current_depth > max_seen {
            max_seen = current_depth;
        }
        // Early exit as soon as we exceed the limit.
        if max_seen > max_depth {
            return Err(ValidationError::bad_request(format!(
                "JSON nesting depth exceeds maximum of {max_depth}"
            )));
        }
        match v {
            serde_json::Value::Array(arr) => {
                for item in arr {
                    stack.push((item, current_depth + 1));
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values() {
                    stack.push((item, current_depth + 1));
                }
            }
            _ => {}
        }
    }

    if max_seen > max_depth {
        Err(ValidationError::bad_request(format!(
            "JSON nesting depth exceeds maximum of {max_depth}"
        )))
    } else {
        Ok(())
    }
}

/// Validate that a filesystem path supplied by an API caller falls inside an
/// allowlist of permitted roots, after canonicalization.
///
/// Audit: `docs/issues/migrate-arbitrary-paths.md`. `POST /api/migrate` used
/// to consume `source_dir` / `target_dir` via `PathBuf::from(...)` with no
/// containment check, and the sibling `POST /api/migrate/scan` had the same
/// flaw via `req.path` — the 200-vs-400 branch on the latter is a pure
/// `.exists()` oracle even without the write primitive. Admin is dev/ops,
/// not the trust ceiling: a compromised Admin token (leaked CI env,
/// phishing) became a full daemon-UID write primitive and a `.exists()`
/// oracle for arbitrary filesystem paths.
///
/// Behaviour:
/// - `require_exists = true` (source paths): the input MUST already exist on
///   disk. We canonicalize it (resolving any `..` or symlink) and reject if
///   the result is not a descendant of any allowed root.
/// - `require_exists = false` (target paths): the input MAY be nonexistent
///   because the migration will create it. We walk up to the nearest existing
///   ancestor, canonicalize THAT, then re-append the unresolved suffix. The
///   composed path must still be a descendant of an allowed root. This blocks
///   `/etc/cron.d/foo` (no existing ancestor under home) and
///   `~/.librefang/../etc/foo` (canonical ancestor is `/etc`, not home).
///
/// Returns the canonicalized path on success.
pub fn validate_path_containment(
    field_name: &str,
    input: &std::path::Path,
    allowed_roots: &[&std::path::Path],
    require_exists: bool,
) -> Result<std::path::PathBuf, ValidationError> {
    // Canonicalize each allowed root once. A root that fails to canonicalize
    // is a daemon-config bug, not user input, so we surface that as a server
    // error rather than silently dropping the root.
    let mut canon_roots: Vec<std::path::PathBuf> = Vec::with_capacity(allowed_roots.len());
    for root in allowed_roots {
        match root.canonicalize() {
            Ok(c) => canon_roots.push(c),
            Err(e) => {
                return Err(ValidationError {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: format!(
                        "allowed migration root '{}' could not be canonicalized: {e}",
                        root.display()
                    ),
                });
            }
        }
    }

    let resolved = if require_exists {
        input.canonicalize().map_err(|e| {
            ValidationError::bad_request(format!(
                "Field '{field_name}': path '{}' could not be canonicalized: {e}",
                input.display()
            ))
        })?
    } else {
        canonicalize_nonexistent(input).map_err(|e| {
            ValidationError::bad_request(format!(
                "Field '{field_name}': path '{}' could not be resolved: {e}",
                input.display()
            ))
        })?
    };

    if canon_roots.iter().any(|root| resolved.starts_with(root)) {
        Ok(resolved)
    } else {
        Err(ValidationError::bad_request(format!(
            "Field '{field_name}': path '{}' is outside the allowed migration roots",
            input.display()
        )))
    }
}

/// Resolve a path that may not yet exist by canonicalizing the nearest
/// existing ancestor and re-appending the unresolved suffix. This is the
/// `target_dir` path that migration will create.
///
/// We deliberately do not use `std::path::absolute` alone: that would not
/// resolve symlinks in the parent chain, leaving a TOCTOU window where
/// `~/.librefang/legit` is a symlink to `/etc/cron.d`.
fn canonicalize_nonexistent(input: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    // Make the input absolute first, so relative paths get anchored to the
    // process cwd before we walk up. `absolute` is purely lexical (no FS
    // access) so it can't fail on missing components.
    let abs = std::path::absolute(input)?;

    // Find the nearest existing ancestor.
    let mut existing = abs.as_path();
    let mut suffix_components: Vec<&std::ffi::OsStr> = Vec::new();
    loop {
        if existing.exists() {
            break;
        }
        let file_name = existing.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "no existing ancestor for path '{}' — cannot resolve symlinks",
                    input.display()
                ),
            )
        })?;
        suffix_components.push(file_name);
        existing = existing.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("path '{}' has no existing ancestor", input.display()),
            )
        })?;
    }

    let mut resolved = existing.canonicalize()?;
    for component in suffix_components.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

/// Validate that a provider name supplied on the
/// `POST/DELETE /api/providers/{name}/key` path is well-shaped before
/// it is used to derive an environment-variable name.
///
/// Contract: `^[a-z0-9-]{1,64}$`.
///
/// Why: `set_provider_key` / `delete_provider_key` derive
/// `env_var = "{NAME}_API_KEY"` from the path segment when the
/// provider is not in the catalog, then write that env var into the
/// live `std::env` and persist it to `secrets.env`. Without a charset
/// + length cap an Admin can plant arbitrary process-wide env vars
///   (`STRIPE_API_KEY`, `AWS_SECRET_ACCESS_KEY`, …) — silently
///   re-targeting any third-party crate that reads them — or submit
///   `name = "a".repeat(1_000_000)` to plant a 1 MB env var. See
///   `docs/issues/set-provider-key-arbitrary-names.md`.
pub fn check_provider_name_shape(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("provider name must not be empty".to_string());
    }
    if name.len() > 64 {
        return Err(format!(
            "provider name too long: {} chars exceeds 64-char cap",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("provider name contains invalid characters (allowed: [a-z0-9-])".to_string());
    }
    Ok(())
}

/// Validate that a derived env-var name is safe to plant into the
/// process environment + `secrets.env`. Used in
/// `set_provider_key` / `delete_provider_key` when the supplied
/// provider name is NOT in the catalog (i.e. an unknown / custom
/// provider) and the env var is therefore derived from path input
/// rather than catalog metadata.
///
/// Contract: `^[A-Z][A-Z0-9_]{0,63}_API_KEY$`.
///
/// The `_API_KEY` suffix requirement is the bright-line trust
/// boundary: even a maximally permissive admin can only plant
/// process-env vars whose name ends in `_API_KEY`, which third-party
/// crates that read e.g. `AWS_SECRET_ACCESS_KEY` or `STRIPE_API_KEY`
/// will not match (`STRIPE_API_KEY` matches; the audit catalogues why
/// that is still considered safer than the unbounded prior behaviour
/// — operator intent is "I'm registering a custom provider", and the
/// catalog is the canonical safe path for known third-party providers).
pub fn check_derived_env_var(env_var: &str) -> Result<(), String> {
    if env_var.len() > 64 {
        return Err(format!(
            "derived env var name too long: {} chars exceeds 64-char cap",
            env_var.len()
        ));
    }
    if !env_var.ends_with("_API_KEY") {
        return Err("derived env var name must end with _API_KEY".to_string());
    }
    let mut chars = env_var.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => {
            return Err(
                "derived env var name must start with an uppercase ASCII letter".to_string(),
            );
        }
    }
    if !env_var
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(
            "derived env var name contains invalid characters (allowed: [A-Z0-9_])".to_string(),
        );
    }
    Ok(())
}

/// Validate that a string looks like a plausible identifier (agent name, key name, etc.).
/// Allows alphanumeric characters, hyphens, underscores, and dots.
pub fn check_identifier(field_name: &str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::bad_request(format!(
            "Field '{field_name}' must not be empty"
        )));
    }
    if value.len() > 256 {
        return Err(ValidationError::bad_request(format!(
            "Field '{field_name}' exceeds maximum identifier length of 256 characters"
        )));
    }
    if !value
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ValidationError::bad_request(format!(
            "Field '{field_name}' contains invalid characters (allowed: alphanumeric, -, _, .)"
        )));
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_string_length_within_limit() {
        assert!(check_string_length("name", "hello", 10).is_ok());
    }

    #[test]
    fn test_check_string_length_at_limit() {
        assert!(check_string_length("name", "12345", 5).is_ok());
    }

    #[test]
    fn test_check_string_length_exceeds_limit() {
        let err = check_string_length("name", "123456", 5).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("name"));
        assert!(err.message.contains("5"));
    }

    #[test]
    fn test_check_not_empty_with_content() {
        assert!(check_not_empty("field", "hello").is_ok());
    }

    #[test]
    fn test_check_not_empty_empty_string() {
        let err = check_not_empty("field", "").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("field"));
    }

    #[test]
    fn test_check_not_empty_whitespace_only() {
        let err = check_not_empty("field", "   ").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_check_json_depth_shallow() {
        let v = serde_json::json!({"a": "b"});
        assert!(check_json_depth(&v, 20).is_ok());
    }

    #[test]
    fn test_check_json_depth_nested_within_limit() {
        let v = serde_json::json!({"a": {"b": {"c": 1}}});
        assert!(check_json_depth(&v, 5).is_ok());
    }

    #[test]
    fn test_check_json_depth_exceeds_limit() {
        // Build a deeply nested JSON value
        let mut v = serde_json::json!(1);
        for _ in 0..25 {
            v = serde_json::json!({"nested": v});
        }
        let err = check_json_depth(&v, 20).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("depth"));
    }

    #[test]
    fn test_check_json_depth_flat_array() {
        let v = serde_json::json!([1, 2, 3, 4, 5]);
        assert!(check_json_depth(&v, 2).is_ok());
    }

    #[test]
    fn test_check_identifier_valid() {
        assert!(check_identifier("id", "my-agent_v1.0").is_ok());
    }

    #[test]
    fn test_check_identifier_empty() {
        let err = check_identifier("id", "").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_check_identifier_invalid_chars() {
        let err = check_identifier("id", "my agent").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("invalid characters"));
    }

    #[test]
    fn test_check_identifier_too_long() {
        let long = "a".repeat(257);
        let err = check_identifier("id", &long).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_check_identifier_path_traversal() {
        let err = check_identifier("id", "../etc/passwd").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    // Refs docs/issues/set-provider-key-arbitrary-names.md.
    // The provider-name shape gate stops path-derived env var planting.

    #[test]
    fn provider_name_accepts_canonical_catalog_names() {
        for ok in [
            "openai",
            "anthropic",
            "google",
            "gemini",
            "groq",
            "openrouter",
            "claude-code",
            "gemini-cli",
            "codex-cli",
            "qwen-code",
            "ollama",
            "x", // 1 char minimum
        ] {
            assert!(
                check_provider_name_shape(ok).is_ok(),
                "must accept canonical provider name {ok:?}"
            );
        }
        // Exactly 64 chars must pass.
        let max = "a".repeat(64);
        assert!(check_provider_name_shape(&max).is_ok());
    }

    #[test]
    fn provider_name_rejects_empty() {
        assert!(check_provider_name_shape("").is_err());
    }

    #[test]
    fn provider_name_rejects_oversize() {
        let long = "a".repeat(65);
        assert!(check_provider_name_shape(&long).is_err());
        let huge = "a".repeat(1_000_000);
        let err = check_provider_name_shape(&huge).unwrap_err();
        assert!(
            err.contains("too long"),
            "diagnostic must say too long: {err}"
        );
    }

    #[test]
    fn provider_name_rejects_uppercase() {
        // Uppercase breaks the catalog naming contract.
        assert!(check_provider_name_shape("OpenAI").is_err());
        assert!(check_provider_name_shape("STRIPE").is_err());
    }

    #[test]
    fn provider_name_rejects_path_traversal_and_separators() {
        for bad in [
            "../../etc",
            "ab/cd",
            "..",
            ".",
            "./foo",
            "a.b",
            "a b",
            "a_b",
        ] {
            assert!(
                check_provider_name_shape(bad).is_err(),
                "must reject {bad:?} — path/charset escape"
            );
        }
    }

    #[test]
    fn provider_name_rejects_null_and_control_chars() {
        assert!(check_provider_name_shape("a\0b").is_err());
        assert!(check_provider_name_shape("a\nb").is_err());
        assert!(check_provider_name_shape("a\tb").is_err());
    }

    #[test]
    fn derived_env_var_accepts_canonical_shapes() {
        for ok in ["MY_PROVIDER_API_KEY", "X_API_KEY", "FOO123_API_KEY"] {
            assert!(
                check_derived_env_var(ok).is_ok(),
                "must accept derived env var {ok:?}"
            );
        }
    }

    #[test]
    fn derived_env_var_rejects_missing_suffix() {
        for bad in ["MY_PROVIDER", "AWS_SECRET_ACCESS_KEY", "PATH", ""] {
            let err = check_derived_env_var(bad).unwrap_err();
            assert!(
                err.contains("_API_KEY"),
                "must mention suffix in diagnostic for {bad:?}: {err}"
            );
        }
    }

    #[test]
    fn derived_env_var_rejects_oversize() {
        // 65-char name (still ends in _API_KEY) — past the 64-cap.
        let long = format!("{}_API_KEY", "A".repeat(57));
        assert_eq!(long.len(), 65);
        assert!(check_derived_env_var(&long).is_err());
    }

    #[test]
    fn derived_env_var_rejects_lowercase_or_leading_digit() {
        assert!(check_derived_env_var("my_provider_API_KEY").is_err());
        assert!(check_derived_env_var("1FOO_API_KEY").is_err());
        assert!(check_derived_env_var("_FOO_API_KEY").is_err());
    }

    #[test]
    fn test_validation_error_response_format() {
        let err = ValidationError::bad_request("test error");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.message, "test error");
    }

    #[test]
    fn test_validation_error_payload_too_large() {
        let err = ValidationError::payload_too_large("too big");
        assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(err.message, "too big");
    }

    #[test]
    fn test_constants_are_reasonable() {
        const { assert!(MAX_REQUEST_BODY_BYTES >= 1024) };
        const { assert!(MAX_STRING_FIELD_LEN >= 1000) };
        const { assert!(MAX_JSON_DEPTH >= 10) };
        const { assert!(MAX_MESSAGE_BYTES >= 64 * 1024) };
        const { assert!(MAX_MESSAGE_CHARS >= 10_000) };
    }

    /// Audit: message-byte-vs-char-cap. The byte cap is the
    /// memory-safety limit; the char cap is the LLM-cost protection.
    /// Both must apply. These tests pin the fairness contract.
    #[test]
    fn check_message_size_accepts_ascii_under_caps() {
        let msg = "a".repeat(1000);
        assert!(check_message_size(&msg).is_ok());
    }

    #[test]
    fn check_message_size_accepts_cjk_text_that_byte_only_cap_would_have_rejected() {
        // Pre-fix MAX_MESSAGE_SIZE = 64 KiB rejected ~21K CJK chars
        // because each glyph encodes to 3 bytes in UTF-8. A 30K-char
        // CJK message is 90 KiB — over the historical 64 KiB limit
        // but well under the new 256 KiB byte cap AND 100 K char
        // cap. Must accept now.
        let msg: String = std::iter::repeat_n('文', 30_000).collect();
        assert!(
            msg.len() > 64 * 1024,
            "test fixture must exceed historical 64 KiB cap"
        );
        assert!(
            check_message_size(&msg).is_ok(),
            "CJK message at 30K chars / ~90 KiB must pass under the new fair caps"
        );
    }

    #[test]
    fn check_message_size_rejects_oversize_byte_payload_with_both_counts_in_error() {
        // Pure ASCII payload past the byte cap. Error must report
        // both byte count and char count so operators can diagnose.
        let msg = "a".repeat(MAX_MESSAGE_BYTES + 1);
        let err = check_message_size(&msg).expect_err("must reject past byte cap");
        assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(
            err.message.contains("bytes") && err.message.contains("chars"),
            "error must include both byte and char counts: {}",
            err.message
        );
    }

    #[test]
    fn check_message_size_rejects_oversize_char_payload() {
        // A multi-byte char payload that satisfies the byte cap but
        // exceeds the char cap — synthetic but pins the
        // independent-cap contract. Use a 2-byte char (Cyrillic) so
        // we hit the char cap before the byte cap.
        // 100_001 × 2 bytes = ~200 KiB (under MAX_MESSAGE_BYTES);
        // 100_001 chars (over MAX_MESSAGE_CHARS).
        let msg: String = std::iter::repeat_n('а', MAX_MESSAGE_CHARS + 1).collect();
        assert!(
            msg.len() < MAX_MESSAGE_BYTES,
            "fixture must respect byte cap"
        );
        let err = check_message_size(&msg).expect_err("must reject past char cap");
        assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(
            err.message.contains("character cap"),
            "error must distinguish char-cap from byte-cap path: {}",
            err.message
        );
    }
}
