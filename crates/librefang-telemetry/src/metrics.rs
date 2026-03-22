//! HTTP metrics utilities for LibreFang telemetry.
//!
//! This module provides a thin wrapper around the `metrics` crate macros
//! (`metrics::counter!`, `metrics::histogram!`).  The public API
//! (`record_http_request`, `normalize_path`, `get_http_metrics_summary`) is
//! kept for backward compatibility, but all recording now delegates to the
//! standard `metrics` crate whose recorder is installed by
//! `crates/librefang-api/src/telemetry.rs`.

use std::time::Duration;

/// Normalize an HTTP path by replacing dynamic segments (UUIDs, hex IDs) with
/// `{id}` so that high-cardinality metric labels are collapsed.
pub fn normalize_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let mut normalized = Vec::new();
    let mut i = 0;

    while i < segments.len() {
        let seg = segments[i];

        if seg.is_empty() {
            normalized.push(seg);
            i += 1;
            continue;
        }

        if seg == "api" || seg == "v1" || seg == "v2" || seg == "a2a" {
            normalized.push(seg);
            i += 1;
            continue;
        }

        if i + 1 < segments.len() {
            let next_seg = segments[i + 1];
            if is_dynamic_segment(next_seg) {
                normalized.push(seg);
                normalized.push("{id}");
                i += 2;
                continue;
            }
        }

        normalized.push(seg);
        i += 1;
    }

    normalized.join("/")
}

/// Determine whether a path segment is a dynamic identifier that should be
/// collapsed.
///
/// Matches:
///   - Standard UUIDs: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (8-4-4-4-12 hex)
///   - Pure hex strings of length 8..64 (e.g. SHA-256 hashes, short IDs)
///
/// Does NOT match general words that happen to contain hyphens (like
/// `well-known` or `my-agent`).
fn is_dynamic_segment(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    // Check for UUID pattern: 8-4-4-4-12 hex digits separated by hyphens
    if is_uuid(s) {
        return true;
    }

    // Pure hex string between 8 and 64 characters (no hyphens).
    if s.len() >= 8 && s.len() <= 64 && !s.contains('-') && s.chars().all(|c| c.is_ascii_hexdigit())
    {
        return true;
    }

    false
}

/// Check if a string matches the UUID format: 8-4-4-4-12 hex groups.
fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected_lengths = [8, 4, 4, 4, 12];
    for (part, &expected_len) in parts.iter().zip(expected_lengths.iter()) {
        if part.len() != expected_len || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    true
}

/// Record an HTTP request using `metrics` crate macros.
///
/// This is the main entry point called by the request-logging middleware in
/// `librefang-api`.  It delegates to `metrics::counter!` and
/// `metrics::histogram!` so that data flows through whichever recorder has been
/// installed (typically the Prometheus exporter).
pub fn record_http_request(path: &str, method: &str, status: u16, duration: Duration) {
    let normalized = normalize_path(path);
    let status_str = status.to_string();

    metrics::counter!(
        "librefang_http_requests_total",
        "method" => method.to_string(),
        "path" => normalized.clone(),
        "status" => status_str,
    )
    .increment(1);

    metrics::histogram!(
        "librefang_http_request_duration_seconds",
        "method" => method.to_string(),
        "path" => normalized,
    )
    .record(duration.as_secs_f64());
}

/// Render an HTTP metrics summary.
///
/// Delegates to the global `PrometheusHandle` installed by
/// `crates/librefang-api/src/telemetry.rs`.  If no recorder has been installed
/// yet (e.g. telemetry feature disabled) this returns a comment explaining why
/// the output is empty.
pub fn get_http_metrics_summary() -> String {
    // The PrometheusHandle lives in librefang-api::telemetry and is rendered
    // directly in the /api/metrics route handler.  This function is kept for
    // backward-compatibility but callers that need the full Prometheus output
    // should use the handle directly.
    "# HTTP metrics are exported via the Prometheus recorder.\n\
     # Use the /api/metrics endpoint or the PrometheusHandle for full output.\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_simple_path() {
        assert_eq!(normalize_path("/api/health"), "/api/health");
    }

    #[test]
    fn test_normalize_uuid_segment() {
        assert_eq!(
            normalize_path("/api/agents/550e8400-e29b-41d4-a716-446655440000/message"),
            "/api/agents/{id}/message"
        );
    }

    #[test]
    fn test_normalize_hex_segment() {
        assert_eq!(
            normalize_path("/api/agents/deadbeef01234567/message"),
            "/api/agents/{id}/message"
        );
    }

    #[test]
    fn test_normalize_preserves_well_known() {
        // "well-known" is NOT a UUID/hex — it must not be replaced.
        assert_eq!(
            normalize_path("/.well-known/agent.json"),
            "/.well-known/agent.json"
        );
    }

    #[test]
    fn test_normalize_preserves_regular_hyphenated_words() {
        assert_eq!(
            normalize_path("/api/my-agent/status"),
            "/api/my-agent/status"
        );
    }

    #[test]
    fn test_is_dynamic_uuid() {
        assert!(is_dynamic_segment("550e8400-e29b-41d4-a716-446655440000"));
    }

    #[test]
    fn test_is_dynamic_hex() {
        assert!(is_dynamic_segment("deadbeef01234567"));
    }

    #[test]
    fn test_not_dynamic_well_known() {
        assert!(!is_dynamic_segment("well-known"));
    }

    #[test]
    fn test_not_dynamic_short_string() {
        assert!(!is_dynamic_segment("abc"));
    }

    #[test]
    fn test_not_dynamic_regular_word() {
        assert!(!is_dynamic_segment("agents"));
    }
}
