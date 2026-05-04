//! Cross-cutting metadata + sub-router composition (#3749 final).
//!
//! After the #3749 series, this module is intentionally thin: it composes the
//! per-domain routers (`agent_templates`, `tools_sessions`, `approvals`,
//! `pairing`, `registry`, `backup`, `commands`, `bindings`, `logs`) and
//! exposes only the workspace-level `/api/versions` metadata endpoint plus
//! two cross-crate path helpers (`librefang_home`, `hostname_string`) that
//! sibling routers still call into.
//!
//! Per-handler history (where each was extracted to):
//!   - tool profiles, agent templates  -> `agent_templates`
//!   - device pairing                  -> `pairing`
//!   - tools, sessions                 -> `tools_sessions`
//!   - approvals, TOTP                 -> `approvals`
//!   - registry schema/content         -> `registry`
//!   - backup / restore                -> `backup`
//!   - audit                           -> `audit`
//!   - task queue                      -> `task_queue`
//!   - webhook subscriptions           -> `webhooks`
//!   - agent KV / memory export-import -> `memory`
//!   - external trigger webhooks       -> `webhooks` (#3749 11/N)
//!   - log SSE stream                  -> `logs`     (#3749 11/N)
//!   - chat command catalog            -> `commands` (#3749 11/N)
//!   - agent bindings                  -> `bindings` (#3749 11/N)
//!   - queue lane occupancy            -> `task_queue` (#3749 11/N)

use super::AppState;
use axum::response::IntoResponse;
use axum::Json;
use std::path::PathBuf;

/// Build the system router by composing every extracted sub-router and
/// exposing the workspace-level `/api/versions` metadata endpoint.
pub fn router() -> axum::Router<std::sync::Arc<AppState>> {
    axum::Router::new()
        .merge(crate::routes::agent_templates::router())
        .merge(crate::routes::tools_sessions::router())
        .merge(crate::routes::approvals::router())
        .merge(crate::routes::pairing::router())
        .merge(crate::routes::registry::router())
        .merge(crate::routes::backup::router())
        // #3749 11/N — final extraction wave.
        .merge(crate::routes::commands::router())
        .merge(crate::routes::bindings::router())
        .merge(crate::routes::logs::router())
}

/// Resolve the LibreFang home directory without depending on the kernel crate.
///
/// Mirrors `librefang_kernel::config::librefang_home`:
/// `LIBREFANG_HOME` env var takes priority, otherwise `~/.librefang`
/// (falling back to the system temp dir if no home directory is available).
///
/// Kept here (rather than in `librefang_kernel`) because the API layer must
/// not import kernel internals at the path-helper level (#3744).
pub(super) fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

/// Get the machine hostname (best-effort).
pub(crate) fn hostname_string() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .or_else(|_| {
            std::process::Command::new("hostname")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .map_err(|_| std::env::VarError::NotPresent)
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// API versioning metadata — workspace-level, unrelated to any sub-domain.
// ---------------------------------------------------------------------------

/// GET /api/versions — List supported API versions and negotiation info.
#[utoipa::path(
    get,
    path = "/api/versions",
    tag = "system",
    responses(
        (status = 200, description = "API version info", body = crate::types::JsonObject)
    )
)]
pub async fn api_versions() -> impl IntoResponse {
    let supported: Vec<&str> = crate::versioning::SUPPORTED_VERSIONS.to_vec();
    let deprecated: Vec<&str> = crate::versioning::DEPRECATED_VERSIONS.to_vec();

    let details: Vec<serde_json::Value> = crate::server::API_VERSIONS
        .iter()
        .map(|(ver, status)| {
            serde_json::json!({
                "version": ver,
                "status": status,
                "url_prefix": format!("/api/{ver}"),
            })
        })
        .collect();

    Json(serde_json::json!({
        "current": crate::versioning::CURRENT_VERSION,
        "supported": supported,
        "deprecated": deprecated,
        "details": details,
        "negotiation": {
            "header": "Accept",
            "media_type_pattern": "application/vnd.librefang.<version>+json",
            "example": "application/vnd.librefang.v1+json",
        },
    }))
}
