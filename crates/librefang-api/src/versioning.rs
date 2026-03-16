//! API versioning support for the LibreFang HTTP API.
//!
//! Provides version extraction from URL path prefix (`/api/v1/...`) or the
//! `Accept` header (`Accept: application/vnd.librefang.v1+json`).
//!
//! # Adding a new API version
//!
//! 1. Add a variant to [`ApiVersion`] (e.g. `V2`).
//! 2. Create `api_v2_routes()` in `server.rs` and nest it under `/api/v2`.
//! 3. Update [`SUPPORTED_VERSIONS`], [`CURRENT_VERSION`], and `API_VERSIONS` in `server.rs`.

use std::str::FromStr;

use axum::http::Request;

// ---------------------------------------------------------------------------
// Version constants
// ---------------------------------------------------------------------------

/// The current (latest) API version string.
pub const CURRENT_VERSION: &str = "v1";

/// All versions the server actively supports.
pub const SUPPORTED_VERSIONS: &[&str] = &["v1"];

/// Versions that are deprecated but still functional.
pub const DEPRECATED_VERSIONS: &[&str] = &[];

/// Vendor media-type prefix used in the `Accept` header.
/// Full form: `application/vnd.librefang.v1+json`
const VENDOR_PREFIX: &str = "application/vnd.librefang.";

// ---------------------------------------------------------------------------
// ApiVersion enum
// ---------------------------------------------------------------------------

/// Represents a resolved API version for a given request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiVersion {
    /// Explicit version 1.
    V1,
}

impl ApiVersion {
    /// Return the canonical string representation (e.g. `"v1"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "v1",
        }
    }
}

impl FromStr for ApiVersion {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "v1" => Ok(Self::V1),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Version extraction helpers
// ---------------------------------------------------------------------------

/// Try to extract an [`ApiVersion`] from the request URL path.
///
/// Recognises paths like `/api/v1/health` and returns `Some(V1)`.
/// Paths like `/api/health` (unversioned) return `None`.
pub fn version_from_path(path: &str) -> Option<ApiVersion> {
    let rest = path.strip_prefix("/api/")?;
    let segment = rest.split('/').next()?;
    if segment.starts_with('v') {
        segment.parse::<ApiVersion>().ok()
    } else {
        None
    }
}

/// Try to extract an [`ApiVersion`] from the `Accept` header.
///
/// Supports the vendor media type `application/vnd.librefang.v1+json`.
pub fn requested_version_from_accept_header(accept: &str) -> Option<&str> {
    for part in accept.split(',') {
        let media_type = part.trim().split(';').next().unwrap_or("").trim();
        if let Some(rest) = media_type.strip_prefix(VENDOR_PREFIX) {
            let (version, suffix) = rest.rsplit_once('+')?;
            if suffix == "json" && !version.is_empty() {
                return Some(version);
            }
        }
    }
    None
}

/// Try to extract an [`ApiVersion`] from the `Accept` header.
///
/// Supports the vendor media type `application/vnd.librefang.v1+json`.
pub fn version_from_accept_header(accept: &str) -> Option<ApiVersion> {
    requested_version_from_accept_header(accept)?
        .parse::<ApiVersion>()
        .ok()
}

/// Resolve the API version for an incoming request.
///
/// Priority:
/// 1. URL path prefix (`/api/v1/...`)
/// 2. `Accept` header (`application/vnd.librefang.v1+json`)
/// 3. Falls back to latest (`V1`)
pub fn resolve_version<B>(req: &Request<B>) -> ApiVersion {
    // 1. Path-based
    if let Some(v) = version_from_path(req.uri().path()) {
        return v;
    }
    // 2. Accept header
    if let Some(accept) = req.headers().get("accept").and_then(|v| v.to_str().ok()) {
        if let Some(v) = version_from_accept_header(accept) {
            return v;
        }
    }
    // 3. Default to latest
    ApiVersion::V1
}

/// Build the JSON payload for `GET /api/versions`.
pub fn versions_payload() -> serde_json::Value {
    serde_json::json!({
        "current": CURRENT_VERSION,
        "supported": SUPPORTED_VERSIONS,
        "deprecated": DEPRECATED_VERSIONS,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_from_path_v1() {
        assert_eq!(version_from_path("/api/v1/health"), Some(ApiVersion::V1));
        assert_eq!(version_from_path("/api/v1/agents"), Some(ApiVersion::V1));
    }

    #[test]
    fn test_version_from_path_unversioned() {
        assert_eq!(version_from_path("/api/health"), None);
        assert_eq!(version_from_path("/api/agents"), None);
    }

    #[test]
    fn test_version_from_path_unknown_version() {
        assert_eq!(version_from_path("/api/v99/health"), None);
    }

    #[test]
    fn test_version_from_path_non_api() {
        assert_eq!(version_from_path("/hooks/wake"), None);
        assert_eq!(version_from_path("/a2a/agents"), None);
    }

    #[test]
    fn test_version_from_accept_header() {
        assert_eq!(
            version_from_accept_header("application/vnd.librefang.v1+json"),
            Some(ApiVersion::V1)
        );
    }

    #[test]
    fn test_version_from_accept_header_with_other_types() {
        assert_eq!(
            version_from_accept_header("text/html, application/vnd.librefang.v1+json, */*"),
            Some(ApiVersion::V1)
        );
    }

    #[test]
    fn test_version_from_accept_header_with_parameters() {
        assert_eq!(
            version_from_accept_header("application/vnd.librefang.v1+json; charset=utf-8"),
            Some(ApiVersion::V1)
        );
    }

    #[test]
    fn test_version_from_accept_header_plain_json() {
        assert_eq!(version_from_accept_header("application/json"), None);
    }

    #[test]
    fn test_version_from_accept_header_unknown_version() {
        assert_eq!(
            version_from_accept_header("application/vnd.librefang.v99+json"),
            None
        );
    }

    #[test]
    fn test_requested_version_from_accept_header_requires_json_suffix() {
        assert_eq!(
            requested_version_from_accept_header("application/vnd.librefang.v1+xml"),
            None
        );
        assert_eq!(
            requested_version_from_accept_header("application/vnd.librefang.v1"),
            None
        );
    }

    #[test]
    fn test_api_version_display() {
        assert_eq!(ApiVersion::V1.to_string(), "v1");
    }

    #[test]
    fn test_api_version_roundtrip() {
        assert_eq!(
            ApiVersion::V1.as_str().parse::<ApiVersion>(),
            Ok(ApiVersion::V1)
        );
    }

    #[test]
    fn test_versions_payload() {
        let payload = versions_payload();
        assert_eq!(payload["current"], "v1");
        assert!(payload["supported"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("v1")));
        assert!(payload["deprecated"].as_array().unwrap().is_empty());
    }
}
