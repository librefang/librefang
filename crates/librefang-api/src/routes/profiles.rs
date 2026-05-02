//! Tool-profile read endpoints.
//!
//! Extracted from `routes::system` (issue #3749) so each subdomain owns its
//! own router and handlers. Public paths and JSON shapes are unchanged.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::ToolProfile;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build the `/profiles` router.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/profiles", axum::routing::get(list_profiles))
        .route("/profiles/{name}", axum::routing::get(get_profile))
}

/// All built-in tool profiles, in display order.
fn builtin_profiles() -> &'static [(&'static str, ToolProfile)] {
    &[
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ]
}

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
    let result: Vec<serde_json::Value> = builtin_profiles()
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
#[utoipa::path(
    get,
    path = "/api/profiles/{name}",
    tag = "system",
    params(("name" = String, Path, description = "Profile name")),
    responses((status = 200, description = "Profile details", body = crate::types::JsonObject))
)]
pub async fn get_profile(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    match builtin_profiles().iter().find(|(n, _)| *n == name) {
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
