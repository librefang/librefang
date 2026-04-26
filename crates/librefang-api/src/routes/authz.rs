//! RBAC follow-up — admin-only effective-permissions snapshot endpoint.
//!
//! Backs the dashboard's permission simulator (RBAC M6, #3209). Returns
//! the raw RBAC inputs configured for one user across all four layers
//! (per-user `tool_policy` + `tool_categories` from M3, `memory_access`
//! from M3, `budget` from M5, `channel_tool_rules` from M3 + channel
//! bindings) so an admin debugging a denial can see every contributing
//! slice in one place without mentally walking the gate path.
//!
//! The endpoint is deliberately a **getter / serializer** — it does NOT
//! recompute the four-layer intersection that decides per-call tool
//! gates. That decision lives in the runtime + kernel gate path
//! (`AuthManager::resolve_user_tool_decision` + per-agent
//! `ToolPolicy::check_tool` + global `ApprovalPolicy.channel_rules`)
//! and is the single source of truth; reproducing it here would
//! silently drift on every gate-logic change.
//!
//! Gating mirrors the M5 `/api/audit/*` and `/api/budget/users/*`
//! endpoints: anonymous callers and Viewer/User roles are denied with a
//! `PermissionDenied` audit entry, only `Admin+` proceeds. The
//! diagnostic surfaces the same identity / policy data those endpoints
//! already expose, so the trust ceiling is identical.

use super::AppState;
use crate::middleware::AuthenticatedApiUser;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use librefang_kernel::auth::UserRole;
use librefang_types::agent::UserId;
use std::sync::Arc;

/// Build admin-gated authz / effective-permissions routes.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route(
        "/authz/effective/{user_id}",
        axum::routing::get(effective_permissions),
    )
}

/// Reject the request unless the caller is an authenticated `Admin`+.
///
/// Anonymous callers (loopback / `LIBREFANG_ALLOW_NO_AUTH=1`) are
/// denied for the same reason as `/api/audit/*`: the snapshot exposes
/// per-user policy and channel bindings — sensitive enough that we
/// don't blanket-trust an unauthenticated origin even on loopback. To
/// use this endpoint in a no-auth deployment, configure at least one
/// user with an admin api_key.
fn require_admin(state: &AppState, api_user: Option<&AuthenticatedApiUser>) -> Option<Response> {
    match api_user {
        Some(u) if u.role >= UserRole::Admin => None,
        Some(u) => {
            state.kernel.audit().record_with_context(
                "system",
                librefang_runtime::audit::AuditAction::PermissionDenied,
                format!("authz/effective endpoint denied for role {}", u.role),
                "denied",
                Some(u.user_id),
                Some("api".to_string()),
            );
            Some(
                ApiErrorResponse::forbidden("Admin role required for effective-permissions access")
                    .into_response(),
            )
        }
        None => {
            state.kernel.audit().record_with_context(
                "system",
                librefang_runtime::audit::AuditAction::PermissionDenied,
                "authz/effective endpoint denied for anonymous caller",
                "denied",
                None,
                Some("api".to_string()),
            );
            Some(
                ApiErrorResponse::forbidden(
                    "Authenticated Admin role required for effective-permissions access \
                     (configure an admin api_key)",
                )
                .into_response(),
            )
        }
    }
}

/// GET /api/authz/effective/{user_id} — admin-only effective-permissions snapshot.
///
/// `user_id` accepts either a UUID (the canonical `UserId` form) or the
/// raw configured name (re-derived via `UserId::from_name`) so operators
/// can paste a name from `config.toml` directly into the URL — same
/// semantics as `/api/budget/users/{user_id}`.
///
/// Returns 404 when no user matches; we intentionally do NOT synthesize
/// "guest defaults" because the simulator's value is showing the operator
/// what they configured, not inventing inputs.
#[utoipa::path(
    get,
    path = "/api/authz/effective/{user_id}",
    tag = "system",
    params(("user_id" = String, Path, description = "User UUID or configured name")),
    responses(
        (status = 200, description = "Effective permissions snapshot", body = serde_json::Value),
        (status = 404, description = "Unknown user"),
    )
)]
pub async fn effective_permissions(
    State(state): State<Arc<AppState>>,
    Path(user_id_param): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    let api_user_ref = api_user.as_ref().map(|e| &e.0);
    if let Some(deny) = require_admin(&state, api_user_ref) {
        return deny;
    }

    // Resolve to a canonical UserId. Try parse-as-uuid first; if that
    // fails fall back to from_name, which always succeeds.
    let user_id: UserId = user_id_param
        .parse()
        .unwrap_or_else(|_| UserId::from_name(&user_id_param));

    match state.kernel.auth_manager().effective_permissions(user_id) {
        Some(snapshot) => Json(snapshot).into_response(),
        None => ApiErrorResponse::not_found(format!(
            "no user matches '{user_id_param}' (try a configured name or canonical UUID)"
        ))
        .into_response(),
    }
}
