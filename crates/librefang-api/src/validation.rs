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
    }
}
