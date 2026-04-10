//! Test helper functions — HTTP request building and response assertions.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};

/// Builds a test HTTP request.
///
/// # Parameters
/// - `method` — HTTP method (GET, POST, etc.)
/// - `path` — Request path (e.g. "/api/health")
/// - `body` — Request body (None for empty body)
///
/// # Example
///
/// ```rust
/// use librefang_testing::test_request;
/// use axum::http::Method;
///
/// let req = test_request(Method::GET, "/api/health", None);
/// let req_with_body = test_request(
///     Method::POST,
///     "/api/agents",
///     Some(r#"{"name": "test"}"#),
/// );
/// ```
pub fn test_request(method: Method, path: &str, body: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);

    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }

    let body = match body {
        Some(b) => Body::from(b.to_string()),
        None => Body::empty(),
    };

    builder.body(body).expect("failed to build test request")
}

/// Builds a tenant-scoped test HTTP request with `X-Account-Id`.
pub fn test_tenant_request(
    method: Method,
    path: &str,
    body: Option<&str>,
    account_id: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-account-id", account_id);

    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }

    let body = match body {
        Some(b) => Body::from(b.to_string()),
        None => Body::empty(),
    };

    builder.body(body).expect("failed to build test request")
}

/// Asserts the response status is 200 and the body is valid JSON.
///
/// Returns the parsed `serde_json::Value`.
///
/// # Panics
///
/// Panics if the status code is not 200 or the body is not valid JSON.
pub async fn assert_json_ok(response: axum::http::Response<Body>) -> serde_json::Value {
    let status = response.status();
    let body = read_body(response).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "expected status 200, got {status}. Response body: {body}"
    );

    serde_json::from_str(&body).unwrap_or_else(|e| {
        panic!("response body is not valid JSON: {e}. Raw content: {body}");
    })
}

/// Asserts the response status matches the expected error code and the body is valid JSON.
///
/// Returns the parsed `serde_json::Value`.
///
/// # Panics
///
/// Panics if the status code does not match or the body is not valid JSON.
pub async fn assert_json_error(
    response: axum::http::Response<Body>,
    expected_status: StatusCode,
) -> serde_json::Value {
    let status = response.status();
    let body = read_body(response).await;

    assert_eq!(
        status, expected_status,
        "expected status {expected_status}, got {status}. Response body: {body}"
    );

    serde_json::from_str(&body).unwrap_or_else(|e| {
        panic!("response body is not valid JSON: {e}. Raw content: {body}");
    })
}

/// Reads the response body as a string.
async fn read_body(response: axum::http::Response<Body>) -> String {
    use http_body_util::BodyExt;
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("failed to read response body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8")
}
