//! Tool profile + agent template endpoints — extracted from `system.rs` per #3749.
//!
//! Mounts `/profiles`, `/profiles/{name}`, `/templates`, `/templates/{name}`,
//! and `/templates/{name}/toml`. Public route paths are unchanged; this module
//! is a sibling under `routes::` and is mounted via
//! `.merge(crate::routes::agent_templates::router())` from `system::router()`.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentManifest;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build routes for the tool-profile + agent-template domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/profiles", axum::routing::get(list_profiles))
        .route("/profiles/{name}", axum::routing::get(get_profile))
        .route("/templates", axum::routing::get(list_agent_templates))
        .route("/templates/{name}", axum::routing::get(get_agent_template))
        .route(
            "/templates/{name}/toml",
            axum::routing::get(get_agent_template_toml),
        )
}

// ---------------------------------------------------------------------------
// Profile + Mode endpoints
// ---------------------------------------------------------------------------

/// GET /api/profiles — List all tool profiles and their tool lists.
#[utoipa::path(
    get,
    path = "/api/profiles",
    tag = "system",
    responses(
        (status = 200, description = "List tool profiles", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_profiles() -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let profiles = [
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    let result: Vec<serde_json::Value> = profiles
        .iter()
        .map(|(name, profile)| {
            serde_json::json!({
                "name": name,
                "tools": profile.tools(),
            })
        })
        .collect();

    Json(result)
}

/// GET /api/profiles/:name — Get a single profile by name.
#[utoipa::path(get, path = "/api/profiles/{name}", tag = "system", params(("name" = String, Path, description = "Profile name")), responses((status = 200, description = "Profile details", body = crate::types::JsonObject)))]
pub async fn get_profile(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    let profiles: &[(&str, ToolProfile)] = &[
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    match profiles.iter().find(|(n, _)| *n == name) {
        Some((n, profile)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": n,
                "tools": profile.tools(),
            })),
        ),
        None => {
            ApiErrorResponse::not_found(t.t_args("api-error-profile-not-found", &[("name", &name)]))
                .into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// Validate a template name supplied via URL path before joining it onto the
/// templates directory. Only permits `[A-Za-z0-9_-]` to guarantee the result
/// cannot escape the base directory through `..`, absolute paths, or platform
/// separators (`/`, `\`). Rejects empty names and anything longer than 64
/// chars to cap log noise.
fn validate_template_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 64 {
        return Err("invalid template name");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid template name");
    }
    Ok(())
}

#[cfg(test)]
mod template_name_validation_tests {
    use super::validate_template_name;

    #[test]
    fn accepts_simple_names() {
        assert!(validate_template_name("assistant").is_ok());
        assert!(validate_template_name("customer-support").is_ok());
        assert!(validate_template_name("coder_v2").is_ok());
        assert!(validate_template_name("a1").is_ok());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_template_name("..").is_err());
        assert!(validate_template_name("../../etc").is_err());
        assert!(validate_template_name("foo/../bar").is_err());
        assert!(validate_template_name("..\\..\\tmp").is_err());
    }

    #[test]
    fn rejects_separators_and_absolute_paths() {
        assert!(validate_template_name("foo/bar").is_err());
        assert!(validate_template_name("foo\\bar").is_err());
        assert!(validate_template_name("/etc/passwd").is_err());
        assert!(validate_template_name("C:\\Windows").is_err());
    }

    #[test]
    fn rejects_empty_and_oversized() {
        assert!(validate_template_name("").is_err());
        assert!(validate_template_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn rejects_null_and_special_chars() {
        assert!(validate_template_name("foo\0bar").is_err());
        assert!(validate_template_name("foo bar").is_err());
        assert!(validate_template_name("foo.bar").is_err());
        assert!(validate_template_name("foo%2fbar").is_err());
    }
}

/// GET /api/templates — List available agent templates.
#[utoipa::path(get, path = "/api/templates", tag = "system", operation_id = "list_agent_templates", responses((status = 200, description = "List templates", body = Vec<serde_json::Value>)))]
pub async fn list_agent_templates() -> impl IntoResponse {
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    let mut templates = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("agent.toml");
                if manifest_path.exists() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let description = std::fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|content| toml::from_str::<AgentManifest>(&content).ok())
                        .map(|m| m.description)
                        .unwrap_or_default();

                    templates.push(serde_json::json!({
                        "name": name,
                        "description": description,
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "templates": templates,
        "total": templates.len(),
    }))
}

/// GET /api/templates/:name — Get template details.
#[utoipa::path(get, path = "/api/templates/{name}", tag = "system", operation_id = "get_agent_template", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template details", body = crate::types::JsonObject)))]
pub async fn get_agent_template(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_template_name(&name).is_err() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    if !manifest_path.exists() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "name": name,
                    "manifest": {
                        "name": manifest.name,
                        "description": manifest.description,
                        "module": manifest.module,
                        "tags": manifest.tags,
                        "model": {
                            "provider": manifest.model.provider,
                            "model": manifest.model.model,
                        },
                        "capabilities": {
                            "tools": manifest.capabilities.tools,
                            "network": manifest.capabilities.network,
                        },
                    },
                    "manifest_toml": content,
                })),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                ApiErrorResponse::internal(t.t("api-error-template-invalid-manifest"))
                    .into_json_tuple()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            ApiErrorResponse::internal(t.t("api-error-template-read-failed")).into_json_tuple()
        }
    }
}

/// GET /api/templates/:name/toml — Get the raw TOML content of a template.
#[utoipa::path(get, path = "/api/templates/{name}/toml", tag = "system", operation_id = "get_agent_template_toml", params(("name" = String, Path, description = "Template name")), responses((status = 200, description = "Template TOML content as plain text", body = String)))]
pub async fn get_agent_template_toml(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_template_name(&name).is_err() {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            t.t("api-error-template-not-found"),
        )
            .into_response();
    }
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    if !manifest_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            t.t("api-error-template-not-found"),
        )
            .into_response();
    }

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            content,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                t.t("api-error-template-read-failed"),
            )
                .into_response()
        }
    }
}
