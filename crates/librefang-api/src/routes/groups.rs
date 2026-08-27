//! User-group management endpoints (#7745).
//!
//! Groups are the second half of the identity model whose first half is `[[users]]`.
//! A permission or an ownership decision that can only name an individual does not survive contact with how teams actually work — a support rota, an on-call shift, a project team, a department all persist while the people in them join and leave — so this surface makes "the support team" a thing the system can name.
//!
//! # Shape
//!
//! Deliberately the same shape as [`crate::routes::users`], because operators reason about the two together and because the persistence, auth, and audit machinery is already correct there and worth reusing rather than reimplementing:
//!
//! * Storage is `[[groups]]` in `config.toml`, rewritten with `toml_edit` through the shared [`crate::routes::users::persist_identity_sections`] helper so comments and unrelated sections survive, the write is atomic, a `backups/config.toml.prev` is taken, and the kernel is reloaded so the change is live without a restart.
//! * Auth is **not** public. Every request goes through the authenticated middleware path, and every mutating call under `/api/groups*` is forced to `Owner` by `middleware::is_owner_only_write` — the same gate as `/api/users*`, for the same reason: group membership will confer roles (#7746), so an Admin API key that could `POST /api/groups` with `roles = ["owner"]` and add itself as a member would be a self-promotion path. `GET` stays at the generic Admin-or-above read gate so the dashboard's group list works for an Admin.
//!
//! # Nesting: groups are flat, by decision
//!
//! There is no parent, no child, and no recursive expansion.
//! The rationale is written out on [`GroupConfig`] rather than restated here; the short form is that the two consumers this entity exists for (#7744's `Principal::Group`, #7746's IdP group mapping) both want flattened effective membership, an IdP hands us exactly that on every login because it has already resolved its own hierarchy, and a locally maintained second hierarchy would drift from it.
//! Adding a `parent` field later is backwards compatible, so the decision is reversible if a concrete case ever demands it.
//!
//! # Dangling members
//!
//! A member name need not resolve to a `[[users]]` entry, and adding one is not an error.
//! #7746 refreshes membership from an IdP claim on every login, and a claim can name someone who has not authenticated here yet.
//! The `unknown_members` field on [`GroupView`] surfaces the discrepancy so the dashboard can flag it instead of the daemon rejecting the write.
//! The opposite direction *is* enforced: deleting a user strips that name from every group in the same config write (see `routes::users::delete_user`).

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::UserId;
use librefang_types::config::GroupConfig;
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::middleware::AuthenticatedApiUser;
use crate::routes::users::{persist_identity_sections, PersistError};

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/groups", axum::routing::get(list_groups).post(create_group))
        .route(
            "/groups/{name}",
            axum::routing::get(get_group)
                .put(update_group)
                .delete(delete_group),
        )
        .route(
            "/groups/{name}/members/{user}",
            axum::routing::put(add_group_member).delete(remove_group_member),
        )
        // Reverse lookup, registered here rather than in `users.rs` so the whole
        // group model lives in one file. It is the resolver #7744 and #7746 read:
        // "which teams is this person on, and what does that confer".
        .route("/users/{name}/groups", axum::routing::get(user_groups))
}

// ---------------------------------------------------------------------------
// View models
// ---------------------------------------------------------------------------

/// Wire view of a group.
///
/// `members` and `roles` are always sorted and de-duplicated — they are stored
/// that way, so this view does not re-sort; it just carries the invariant to
/// the client. `member_count` is redundant with `members.len()` and present
/// because the dashboard's list renders a count per row and should not have to
/// depend on the full membership array staying in the list payload if it is
/// ever trimmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct GroupView {
    pub name: String,
    pub description: String,
    pub members: Vec<String>,
    pub roles: Vec<String>,
    pub member_count: usize,
    /// Members that do not resolve to a `[[users]]` entry. Not an error (see
    /// the module doc-comment on dangling members) — surfaced so the dashboard
    /// can badge the row rather than the operator discovering it by accident.
    pub unknown_members: Vec<String>,
}

impl GroupView {
    fn build(group: &GroupConfig, known_users: &BTreeSet<&str>) -> Self {
        let unknown_members: Vec<String> = group
            .members
            .iter()
            .filter(|m| !known_users.contains(m.as_str()))
            .cloned()
            .collect();
        Self {
            name: group.name.clone(),
            description: group.description.clone(),
            members: group.members.clone(),
            roles: group.roles.clone(),
            member_count: group.members.len(),
            unknown_members,
        }
    }
}

/// Payload for creating or replacing a group.
///
/// `deny_unknown_fields` matches the users surface: a typo'd key is a 422
/// rather than a silently ignored field, which is the difference between an
/// operator noticing that `member` should have been `members` and a group
/// quietly having nobody in it.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupUpsert {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

/// Response of `GET /api/users/{name}/groups`.
///
/// `roles` is the resolved union that `KernelConfig::roles_for_user` computes:
/// each group's own name plus every entry in its `roles` list. It deliberately
/// excludes `UserConfig.role` — that is the RBAC privilege level, a different
/// ladder, and #7746 is where an operator gets to connect the two under an
/// explicit mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct UserGroupsView {
    pub name: String,
    pub groups: Vec<String>,
    pub roles: Vec<String>,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Hard cap on members in one group.
///
/// `config.toml` is an operator-edited file and a group is a team, so real
/// values are in the tens. The cap exists for the same reason the bulk-import
/// limit does: a single `POST` body within the 8 MiB request cap could
/// otherwise carry a million-entry `members` array that is then sorted,
/// de-duplicated, serialized into TOML and written to disk on every subsequent
/// identity mutation.
const MAX_GROUP_MEMBERS: usize = 10_000;

/// Hard cap on roles conferred by one group. Roles are a controlled vocabulary
/// an operator types by hand; the ceiling is generous and exists only to bound
/// the same allocation path as `MAX_GROUP_MEMBERS`.
const MAX_GROUP_ROLES: usize = 256;

/// Mirrors `users::validate_name` — non-empty after trimming, at most 128
/// chars. Kept as a separate function rather than shared because the group
/// name is additionally a role string (`roles_for_user` returns it), so it is
/// the natural place for a future charset restriction that would be wrong to
/// impose on user display names.
fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if trimmed.len() > 128 {
        return Err("name too long (max 128 chars)".to_string());
    }
    Ok(trimmed.to_string())
}

/// Normalize a member / role list: trim each entry, drop empties, sort, and
/// de-duplicate.
///
/// Sorting at the write boundary rather than at the read boundary is what makes
/// the stored form canonical: `config.toml` is byte-identical for two writes
/// that differ only in the order the client listed members, and any downstream
/// stringification of a group (an ownership line in an agent prompt, once #7744
/// lands) inherits that determinism instead of having to re-establish it (#3298).
fn normalize_list(values: &[String], limit: usize, label: &str) -> Result<Vec<String>, String> {
    if values.len() > limit {
        return Err(format!(
            "too many {label} ({} supplied, max {limit})",
            values.len()
        ));
    }
    let set: BTreeSet<String> = values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect();
    Ok(set.into_iter().collect())
}

/// The `{"status": "error", ...}` envelope is forbidden repo-wide (#3505): the HTTP
/// status code is the source of truth for error-vs-ok, and a body-level `status`
/// field invites clients to branch on it instead and disagree with the code.
/// `scripts/check-error-shape.sh` enforces this.
///
/// Kept as a `(StatusCode, msg)` helper so the sixteen call sites read the same as
/// before; only the envelope changed.
fn err_response(status: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    let msg = msg.into();
    match status {
        StatusCode::BAD_REQUEST => crate::types::ApiErrorResponse::bad_request(msg),
        StatusCode::NOT_FOUND => crate::types::ApiErrorResponse::not_found(msg),
        StatusCode::CONFLICT => crate::types::ApiErrorResponse::conflict(msg),
        _ => crate::types::ApiErrorResponse::internal(msg),
    }
    .into_response()
}

fn persist_error_response(state: &Arc<AppState>, e: PersistError) -> axum::response::Response {
    match e {
        PersistError::BadRequest(m) => err_response(StatusCode::BAD_REQUEST, m),
        PersistError::Conflict(m) => err_response(StatusCode::CONFLICT, m),
        PersistError::NotFound(m) => err_response(StatusCode::NOT_FOUND, m),
        PersistError::Internal(m) => err_response(StatusCode::INTERNAL_SERVER_ERROR, m),
        PersistError::Managed => crate::routes::managed_config_response(state.kernel.config_path()),
    }
}

/// Build a `GroupView` from the **live** config, so the response body reflects
/// what the kernel reloaded rather than what the handler intended to write.
fn view_from_live(state: &Arc<AppState>, name: &str) -> Option<GroupView> {
    let cfg = state.kernel.config_ref();
    let known: BTreeSet<&str> = cfg.users.iter().map(|u| u.name.as_str()).collect();
    cfg.group(name).map(|g| GroupView::build(g, &known))
}

/// `persist_identity_sections` records this against the audit log so a group
/// edit is distinguishable from a user edit without diffing `config.toml`.
const GROUP_AUDIT_DETAIL: &str = "groups updated";

fn caller_id(caller: &Option<Extension<AuthenticatedApiUser>>) -> Option<UserId> {
    caller.as_ref().map(|c| c.0.user_id)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/groups",
    tag = "groups",
    responses(
        (status = 200, description = "List of configured groups, ordered by name", body = [GroupView])
    )
)]
pub async fn list_groups(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let known: BTreeSet<&str> = cfg.users.iter().map(|u| u.name.as_str()).collect();
    // Ordered by name rather than by declaration order in `config.toml`: the
    // dashboard renders the list as-is, and a list that reshuffles when an
    // unrelated group is appended to the file is a list nobody can scan.
    let mut groups: Vec<&librefang_types::config::GroupConfig> = cfg.groups.iter().collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    let views: Vec<GroupView> = groups
        .into_iter()
        .map(|g| GroupView::build(g, &known))
        .collect();
    Json(views).into_response()
}

#[utoipa::path(
    get,
    path = "/api/groups/{name}",
    tag = "groups",
    params(("name" = String, Path, description = "Group name (case-sensitive)")),
    responses(
        (status = 200, description = "Group detail", body = GroupView),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_group(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match view_from_live(&state, &name) {
        Some(view) => Json(view).into_response(),
        None => err_response(StatusCode::NOT_FOUND, format!("group '{name}' not found")),
    }
}

#[utoipa::path(
    post,
    path = "/api/groups",
    tag = "groups",
    request_body = GroupUpsert,
    responses(
        (status = 201, description = "Group created", body = GroupView),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Group already exists"),
    )
)]
pub async fn create_group(
    State(state): State<Arc<AppState>>,
    caller: Option<Extension<AuthenticatedApiUser>>,
    Json(req): Json<GroupUpsert>,
) -> impl IntoResponse {
    let name = match validate_name(&req.name) {
        Ok(n) => n,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let members = match normalize_list(&req.members, MAX_GROUP_MEMBERS, "members") {
        Ok(m) => m,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let roles = match normalize_list(&req.roles, MAX_GROUP_ROLES, "roles") {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };

    let new_group = GroupConfig {
        name: name.clone(),
        description: req.description.trim().to_string(),
        members,
        roles,
    };

    // Pre-check against the live snapshot so the obvious conflict does not have
    // to take the config write lock. The persist closure re-checks under the
    // lock, which is what actually makes it race-free.
    if state.kernel.config_ref().group(&name).is_some() {
        return err_response(
            StatusCode::CONFLICT,
            format!("group '{name}' already exists"),
        );
    }

    let to_push = new_group.clone();
    match persist_identity_sections(
        &state,
        caller_id(&caller),
        GROUP_AUDIT_DETAIL,
        move |_users, groups| {
            if groups.iter().any(|g| g.name == to_push.name) {
                return Err(PersistError::Conflict(format!(
                    "group '{}' already exists",
                    to_push.name
                )));
            }
            groups.push(to_push);
            Ok(())
        },
    )
    .await
    {
        Ok(()) => match view_from_live(&state, &name) {
            Some(view) => (StatusCode::CREATED, Json(view)).into_response(),
            None => err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("group '{name}' vanished between write and reload"),
            ),
        },
        Err(e) => persist_error_response(&state, e),
    }
}

#[utoipa::path(
    put,
    path = "/api/groups/{name}",
    tag = "groups",
    params(("name" = String, Path, description = "Group name (case-sensitive)")),
    request_body = GroupUpsert,
    responses(
        (status = 200, description = "Group updated", body = GroupView),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Rename collides with an existing group"),
    )
)]
pub async fn update_group(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    caller: Option<Extension<AuthenticatedApiUser>>,
    Json(req): Json<GroupUpsert>,
) -> impl IntoResponse {
    // Same convention as `update_user`: the URL path identifies the group being
    // edited and the body's `name` is the desired final name, so a rename is a
    // PUT rather than a delete-and-recreate that would lose the membership list.
    let final_name = match validate_name(&req.name) {
        Ok(n) => n,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let members = match normalize_list(&req.members, MAX_GROUP_MEMBERS, "members") {
        Ok(m) => m,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let roles = match normalize_list(&req.roles, MAX_GROUP_ROLES, "roles") {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let description = req.description.trim().to_string();

    let target = name.clone();
    let renamed_to = final_name.clone();
    match persist_identity_sections(
        &state,
        caller_id(&caller),
        GROUP_AUDIT_DETAIL,
        move |_users, groups| {
            let idx = groups
                .iter()
                .position(|g| g.name == target)
                .ok_or_else(|| PersistError::NotFound(format!("group '{target}' not found")))?;
            if renamed_to != target && groups.iter().any(|g| g.name == renamed_to) {
                return Err(PersistError::Conflict(format!(
                    "another group named '{renamed_to}' already exists"
                )));
            }
            groups[idx] = GroupConfig {
                name: renamed_to,
                description,
                members,
                roles,
            };
            Ok(())
        },
    )
    .await
    {
        Ok(()) => match view_from_live(&state, &final_name) {
            Some(view) => (StatusCode::OK, Json(view)).into_response(),
            None => err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("group '{final_name}' vanished between write and reload"),
            ),
        },
        Err(e) => persist_error_response(&state, e),
    }
}

#[utoipa::path(
    delete,
    path = "/api/groups/{name}",
    tag = "groups",
    params(("name" = String, Path, description = "Group name (case-sensitive)")),
    responses(
        (status = 204, description = "Group deleted"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_group(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    caller: Option<Extension<AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let target = name.clone();
    // Deleting a group deletes the membership with it — there is no cascade to
    // run in the other direction, because a `[[users]]` entry never names the
    // groups it belongs to. Membership lives on exactly one side of the
    // relation, which is the reason the many-to-many needs no join table.
    match persist_identity_sections(
        &state,
        caller_id(&caller),
        GROUP_AUDIT_DETAIL,
        move |_users, groups| {
            let before = groups.len();
            groups.retain(|g| g.name != target);
            if groups.len() == before {
                Err(PersistError::NotFound(format!(
                    "group '{target}' not found"
                )))
            } else {
                Ok(())
            }
        },
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => persist_error_response(&state, e),
    }
}

#[utoipa::path(
    put,
    path = "/api/groups/{name}/members/{user}",
    tag = "groups",
    params(
        ("name" = String, Path, description = "Group name (case-sensitive)"),
        ("user" = String, Path, description = "User name to add (case-sensitive)"),
    ),
    responses(
        (status = 200, description = "Membership after the add", body = GroupView),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Group not found"),
    )
)]
pub async fn add_group_member(
    State(state): State<Arc<AppState>>,
    Path((name, user)): Path<(String, String)>,
    caller: Option<Extension<AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let member = match validate_name(&user) {
        Ok(m) => m,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let target = name.clone();
    // Idempotent: adding a member who is already in the group is a 200 with the
    // unchanged membership, not a 409. The caller that matters here is #7746's
    // per-login sync, which replays the IdP's claim on every authentication and
    // would otherwise have to diff before writing.
    match persist_identity_sections(
        &state,
        caller_id(&caller),
        GROUP_AUDIT_DETAIL,
        move |_users, groups| {
            let group = groups
                .iter_mut()
                .find(|g| g.name == target)
                .ok_or_else(|| PersistError::NotFound(format!("group '{target}' not found")))?;
            if group.has_member(&member) {
                return Ok(());
            }
            if group.members.len() >= MAX_GROUP_MEMBERS {
                return Err(PersistError::BadRequest(format!(
                    "group '{}' already has the maximum of {MAX_GROUP_MEMBERS} members",
                    group.name
                )));
            }
            group.members.push(member);
            // Re-establish the sorted invariant the stored form carries.
            group.members.sort();
            Ok(())
        },
    )
    .await
    {
        Ok(()) => match view_from_live(&state, &name) {
            Some(view) => (StatusCode::OK, Json(view)).into_response(),
            None => err_response(StatusCode::NOT_FOUND, format!("group '{name}' not found")),
        },
        Err(e) => persist_error_response(&state, e),
    }
}

#[utoipa::path(
    delete,
    path = "/api/groups/{name}/members/{user}",
    tag = "groups",
    params(
        ("name" = String, Path, description = "Group name (case-sensitive)"),
        ("user" = String, Path, description = "User name to remove (case-sensitive)"),
    ),
    responses(
        (status = 200, description = "Membership after the removal", body = GroupView),
        (status = 404, description = "Group not found"),
    )
)]
pub async fn remove_group_member(
    State(state): State<Arc<AppState>>,
    Path((name, user)): Path<(String, String)>,
    caller: Option<Extension<AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let target = name.clone();
    let member = user.trim().to_string();
    // Also idempotent, and for a sharper reason than the add: a revocation that
    // 404s when the membership is already absent invites the caller to treat the
    // error as "nothing to do" and stop checking, which is exactly the habit that
    // makes a real revocation failure invisible. Removing an absent member is a
    // successful revocation.
    match persist_identity_sections(
        &state,
        caller_id(&caller),
        GROUP_AUDIT_DETAIL,
        move |_users, groups| {
            let group = groups
                .iter_mut()
                .find(|g| g.name == target)
                .ok_or_else(|| PersistError::NotFound(format!("group '{target}' not found")))?;
            group.members.retain(|m| m != &member);
            Ok(())
        },
    )
    .await
    {
        Ok(()) => match view_from_live(&state, &name) {
            Some(view) => (StatusCode::OK, Json(view)).into_response(),
            None => err_response(StatusCode::NOT_FOUND, format!("group '{name}' not found")),
        },
        Err(e) => persist_error_response(&state, e),
    }
}

#[utoipa::path(
    get,
    path = "/api/users/{name}/groups",
    tag = "groups",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    responses(
        (status = 200, description = "Groups the user belongs to and the roles that confers", body = UserGroupsView),
    )
)]
pub async fn user_groups(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    // No 404 for an unregistered name. A user can be named in a group before a
    // `[[users]]` entry exists (see the module doc-comment), so "this name is in
    // no group" and "this name has no user row" are different facts and the
    // caller asked about the first one. An empty answer is the correct answer.
    let groups: Vec<String> = cfg
        .groups_for_user(&name)
        .into_iter()
        .map(|g| g.name.clone())
        .collect();
    let roles: Vec<String> = cfg.roles_for_user(&name).into_iter().collect();
    Json(UserGroupsView {
        name,
        groups,
        roles,
    })
    .into_response()
}
