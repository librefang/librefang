//! User RBAC management endpoints (Phase 4 / RBAC M6).
//!
//! These endpoints expose CRUD over `[[users]]` entries in `config.toml`,
//! plus a bulk-import endpoint used by the dashboard CSV-import wizard.
//!
//! Auth: NOT in the public allowlist — every request goes through the
//! authenticated middleware path. Mutating calls (`POST` / `PUT` /
//! `DELETE` under `/api/users*`) are additionally gated to `Owner` via
//! `middleware::is_owner_only_write` because they map to
//! `Action::ManageUsers` in the kernel — without that gate an Admin
//! per-user API key could `POST /api/users` to create a `role: "owner"`
//! user with a chosen `api_key_hash` and self-promote. `GET` stays
//! Admin-or-above so the permission simulator's user list keeps working.
//! Static api_key / dashboard session callers bypass the per-user role
//! check by design (they are owner-equivalent shared secrets).
//!
//! Persistence model: we read the live `KernelConfig`, mutate the `users`
//! vector, then rewrite the `[[users]]` array-of-tables in `config.toml`
//! using `toml_edit` so unrelated comments/sections are preserved. After
//! every successful write we trigger a kernel reload so the in-memory
//! `AuthManager` picks up the change without restart.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_kernel::auth::UserRole;
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use librefang_types::user_policy::{
    ChannelToolPolicy, UserMemoryAccess, UserToolCategories, UserToolPolicy,
};
use serde::{Deserialize, Serialize};

use super::AppState;
use crate::middleware::{ApiUserAuth, AuthenticatedApiUser};

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/users", axum::routing::get(list_users).post(create_user))
        .route(
            "/users/{name}",
            axum::routing::get(get_user)
                .put(update_user)
                .delete(delete_user),
        )
        .route("/users/import", axum::routing::post(import_users))
        .route(
            "/users/{name}/policy",
            axum::routing::get(get_user_policy).put(update_user_policy),
        )
        .route(
            "/users/{name}/rotate-key",
            axum::routing::post(rotate_user_key),
        )
}

// ---------------------------------------------------------------------------
// View models
// ---------------------------------------------------------------------------

/// Sanitized user view returned over the wire — never echoes the
/// `api_key_hash` value, nor the contents of `tool_policy`,
/// `memory_access`, `budget`, etc. The list view only needs presence
/// flags so the dashboard can show a "this user is policy-customized"
/// badge; the per-user detail endpoints already surface the bodies.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct UserView {
    pub name: String,
    pub role: String,
    pub channel_bindings: HashMap<String, String>,
    pub has_api_key: bool,
    /// True when the user has any per-user tool policy configured —
    /// either an allow/deny list, tool-category overrides, or
    /// per-channel rules. Summary only; the contents stay behind
    /// `/api/users/{name}/policy`.
    pub has_policy: bool,
    /// True when the user has a custom memory namespace ACL.
    pub has_memory_access: bool,
    /// True when the user has a per-user budget cap configured.
    pub has_budget: bool,
}

impl From<&UserConfig> for UserView {
    fn from(cfg: &UserConfig) -> Self {
        let has_policy = cfg.tool_policy.is_some()
            || cfg.tool_categories.is_some()
            || !cfg.channel_tool_rules.is_empty();
        Self {
            name: cfg.name.clone(),
            role: cfg.role.clone(),
            channel_bindings: cfg.channel_bindings.clone(),
            has_api_key: cfg
                .api_key_hash
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            has_policy,
            has_memory_access: cfg.memory_access.is_some(),
            has_budget: cfg.budget.is_some(),
        }
    }
}

/// Payload for creating or replacing a user. `api_key_hash` is accepted
/// pre-hashed (Argon2 phc string) — the dashboard hashes locally before
/// sending. `None` clears any existing hash on update; absent on create.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct UserUpsert {
    pub name: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub channel_bindings: HashMap<String, String>,
    #[serde(default)]
    pub api_key_hash: Option<String>,
}

fn default_role() -> String {
    "user".to_string()
}

/// Bulk-import payload. `rows` are pre-parsed by the frontend (drag-drop
/// CSV, dialect-aware). `dry_run = true` returns counts without persisting.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct BulkImportRequest {
    #[serde(default)]
    pub rows: Vec<UserUpsert>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct BulkImportRow {
    pub index: usize,
    pub name: String,
    pub status: String, // "created" | "updated" | "failed"
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct BulkImportResult {
    pub created: usize,
    pub updated: usize,
    pub failed: usize,
    pub dry_run: bool,
    pub rows: Vec<BulkImportRow>,
}

/// Response payload for `POST /api/users/{name}/rotate-key`.
///
/// `new_api_key` is the **plaintext** rotated key — this is the only time
/// the server will surface it. Operators must copy and store it now;
/// nothing else (audit log included) records the plaintext.
///
/// `sessions_invalidated` reports how many in-memory per-user API key
/// records were swapped — typically `1`. See the doc-comment on
/// [`rotate_user_key`] for why dashboard sessions are NOT included in
/// this count.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RotateKeyResponse {
    pub status: String,
    pub new_api_key: String,
    pub sessions_invalidated: usize,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

const VALID_ROLES: &[&str] = &["owner", "admin", "user", "viewer"];

fn validate_role(role: &str) -> Result<String, String> {
    let normalized = role.trim().to_lowercase();
    if VALID_ROLES.iter().any(|r| *r == normalized) {
        Ok(normalized)
    } else {
        Err(format!(
            "invalid role '{role}' — expected one of: {}",
            VALID_ROLES.join(", ")
        ))
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if trimmed.len() > 128 {
        return Err("name too long (max 128 chars)".to_string());
    }
    Ok(())
}

/// Reject anything that isn't a parseable Argon2id PHC string.
///
/// The dashboard hashes locally before sending so the daemon never sees
/// the plaintext, but the wire shape is still `String` — without this
/// check an Owner could paste an arbitrary value (a hash exfiltrated
/// from a different database, a constant, an empty-after-trim string)
/// into `api_key_hash` and silently grant whoever knows that hash's
/// preimage a working API key. `password_hash::PasswordHash::new` parses
/// the PHC structure (algorithm / params / salt / hash segments) without
/// running the verifier, so we get format validation for free.
///
/// `None` and trim-empty strings are treated as "clear the existing hash"
/// and accepted unchanged.
fn validate_api_key_hash(hash: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = hash else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    argon2::password_hash::PasswordHash::new(trimmed).map_err(|e| {
        format!(
            "api_key_hash is not a valid Argon2 PHC string: {e} \
             (expected `$argon2id$v=19$m=…,t=…,p=…$<salt>$<hash>`)"
        )
    })?;
    Ok(Some(trimmed.to_string()))
}

fn err_response(status: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({ "status": "error", "error": msg.into() })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/users",
    tag = "users",
    responses(
        (status = 200, description = "List of registered users", body = [UserView])
    )
)]
pub async fn list_users(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    let users: Vec<UserView> = cfg.users.iter().map(UserView::from).collect();
    Json(users).into_response()
}

#[utoipa::path(
    get,
    path = "/api/users/{name}",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    responses(
        (status = 200, description = "User detail", body = UserView),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_user(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    match cfg.users.iter().find(|u| u.name == name) {
        Some(u) => Json(UserView::from(u)).into_response(),
        None => err_response(StatusCode::NOT_FOUND, format!("user '{name}' not found")),
    }
}

#[utoipa::path(
    post,
    path = "/api/users",
    tag = "users",
    request_body = UserUpsert,
    responses(
        (status = 201, description = "User created", body = UserView),
        (status = 400, description = "Validation error"),
        (status = 409, description = "User already exists"),
    )
)]
pub async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UserUpsert>,
) -> impl IntoResponse {
    if let Err(e) = validate_name(&req.name) {
        return err_response(StatusCode::BAD_REQUEST, e);
    }
    let role = match validate_role(&req.role) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let api_key_hash = match validate_api_key_hash(req.api_key_hash.as_deref()) {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };

    let new_cfg = UserConfig {
        name: req.name.trim().to_string(),
        role,
        channel_bindings: req.channel_bindings,
        api_key_hash,
        // RBAC M5 (#3203) per-user budget — read-only display data in
        // this slice (no write endpoint, no per-user enforcement; the
        // metering pipeline still only enforces global / per-agent /
        // per-provider caps). For now budget is set by editing
        // config.toml directly; a follow-up adds the write path.
        budget: None,
        // RBAC M3 (#3205) per-user policy fields. M6's create endpoint
        // doesn't accept them yet — the dashboard's matrix editor
        // (`/users/{name}/policy`) is the future home, ships as a stub
        // page for now. Default to "no opinion" so the kernel's role-
        // based defaults (and existing channel/category rules) decide.
        tool_policy: None,
        tool_categories: None,
        memory_access: None,
        channel_tool_rules: HashMap::new(),
    };

    // Pre-check duplicates so we can map them to 409 cleanly. The persist
    // closure does its own check too (in case of a race), but the live
    // snapshot here lets us avoid acquiring the write lock for an obvious
    // conflict.
    if state
        .kernel
        .config_ref()
        .users
        .iter()
        .any(|u| u.name == new_cfg.name)
    {
        return err_response(
            StatusCode::CONFLICT,
            format!("user '{}' already exists", new_cfg.name),
        );
    }

    let to_push = new_cfg.clone();
    match persist_users(&state, move |users| {
        if users.iter().any(|u| u.name == to_push.name) {
            return Err(PersistError::Conflict(format!(
                "user '{}' already exists",
                to_push.name
            )));
        }
        users.push(to_push);
        Ok(())
    })
    .await
    {
        Ok(()) => (StatusCode::CREATED, Json(UserView::from(&new_cfg))).into_response(),
        Err(PersistError::Conflict(m)) => err_response(StatusCode::CONFLICT, m),
        Err(PersistError::BadRequest(m)) => err_response(StatusCode::BAD_REQUEST, m),
        Err(PersistError::NotFound(m)) => err_response(StatusCode::NOT_FOUND, m),
        Err(PersistError::Internal(m)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, m),
    }
}

#[utoipa::path(
    put,
    path = "/api/users/{name}",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    request_body = UserUpsert,
    responses(
        (status = 200, description = "User updated", body = UserView),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn update_user(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<UserUpsert>,
) -> impl IntoResponse {
    if let Err(e) = validate_name(&req.name) {
        return err_response(StatusCode::BAD_REQUEST, e);
    }
    let role = match validate_role(&req.role) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };

    let api_key_hash = match validate_api_key_hash(req.api_key_hash.as_deref()) {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };

    // The PUT body's `name` is treated as the desired final name; the URL
    // path identifies the user being updated. Allow rename so the dashboard
    // can edit the display name without a delete-and-recreate dance.
    let renamed_to = req.name.trim().to_string();
    let new_role = role;
    let new_bindings = req.channel_bindings;
    let new_api_key_hash = api_key_hash;

    let target_existing = name.clone();
    let renamed_to_for_closure = renamed_to.clone();
    // The closure returns the final `UserConfig` so the response body
    // can serialize the post-merge view (incl. preserved RBAC M3 policy
    // fields). `persist_users` is generic over the closure's `Ok` type,
    // so this avoids the Arc<Mutex> capture pattern earlier drafts used.
    match persist_users(&state, move |users| -> Result<UserConfig, PersistError> {
        let idx = users
            .iter()
            .position(|u| u.name == target_existing)
            .ok_or_else(|| PersistError::NotFound(format!("user '{target_existing}' not found")))?;
        // If renaming, ensure no collision with another existing user.
        if renamed_to_for_closure != target_existing
            && users.iter().any(|u| u.name == renamed_to_for_closure)
        {
            return Err(PersistError::Conflict(format!(
                "another user named '{}' already exists",
                renamed_to_for_closure
            )));
        }
        // RBAC M3 (#3205) + M5 (#3203): preserve per-user `tool_policy`,
        // `tool_categories`, `memory_access`, `channel_tool_rules`, and
        // `budget` across the rename/role/binding edit. The M6 dashboard
        // only exposes name/role/bindings/api_key_hash today; clobbering
        // the RBAC fields here would silently disable a Viewer's PII
        // redaction the moment an admin retitles their account. `budget`
        // is currently set via config.toml (no write endpoint yet, full
        // per-user enforcement lands in an M5 follow-up), and the same
        // preserve-across-edit rule applies.
        let preserved = users[idx].clone();
        users[idx] = UserConfig {
            name: renamed_to_for_closure.clone(),
            role: new_role.clone(),
            channel_bindings: new_bindings.clone(),
            api_key_hash: new_api_key_hash.clone(),
            budget: preserved.budget,
            tool_policy: preserved.tool_policy,
            tool_categories: preserved.tool_categories,
            memory_access: preserved.memory_access,
            channel_tool_rules: preserved.channel_tool_rules,
        };
        Ok(users[idx].clone())
    })
    .await
    {
        Ok(final_cfg) => (StatusCode::OK, Json(UserView::from(&final_cfg))).into_response(),
        Err(PersistError::Conflict(m)) => err_response(StatusCode::CONFLICT, m),
        Err(PersistError::NotFound(m)) => err_response(StatusCode::NOT_FOUND, m),
        Err(PersistError::BadRequest(m)) => err_response(StatusCode::BAD_REQUEST, m),
        Err(PersistError::Internal(m)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, m),
    }
}

#[utoipa::path(
    delete,
    path = "/api/users/{name}",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    responses(
        (status = 200, description = "User deleted"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let target = name.clone();
    match persist_users(&state, move |users| {
        let before = users.len();
        users.retain(|u| u.name != target);
        if users.len() == before {
            Err(PersistError::NotFound(format!("user '{target}' not found")))
        } else {
            Ok(())
        }
    })
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(PersistError::NotFound(m)) => err_response(StatusCode::NOT_FOUND, m),
        Err(PersistError::BadRequest(m)) => err_response(StatusCode::BAD_REQUEST, m),
        Err(PersistError::Conflict(m)) => err_response(StatusCode::CONFLICT, m),
        Err(PersistError::Internal(m)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, m),
    }
}

#[utoipa::path(
    post,
    path = "/api/users/import",
    tag = "users",
    request_body = BulkImportRequest,
    responses(
        (status = 200, description = "Import result", body = BulkImportResult),
    )
)]
pub async fn import_users(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BulkImportRequest>,
) -> impl IntoResponse {
    // Validate every row first so the preview can surface errors without
    // mutating state.
    let mut prepared: Vec<(usize, Result<UserConfig, String>)> = Vec::with_capacity(req.rows.len());
    for (i, row) in req.rows.iter().enumerate() {
        let prepared_row = (|| -> Result<UserConfig, String> {
            validate_name(&row.name)?;
            let role = validate_role(&row.role)?;
            let api_key_hash = validate_api_key_hash(row.api_key_hash.as_deref())?;
            Ok(UserConfig {
                name: row.name.trim().to_string(),
                role,
                channel_bindings: row.channel_bindings.clone(),
                api_key_hash,
                // CSV import doesn't carry RBAC M5 (#3203) budget or
                // M3 (#3205) policy fields — start blank for new rows;
                // the per-row update path below preserves existing
                // values for rows that match an already-registered name.
                budget: None,
                tool_policy: None,
                tool_categories: None,
                memory_access: None,
                channel_tool_rules: HashMap::new(),
            })
        })();
        prepared.push((i, prepared_row));
    }

    if req.dry_run {
        // Compute the would-be counts without writing.
        let cfg = state.kernel.config_ref();
        let existing_names: std::collections::HashSet<&str> =
            cfg.users.iter().map(|u| u.name.as_str()).collect();
        let mut rows_out = Vec::with_capacity(prepared.len());
        let mut created = 0usize;
        let mut updated = 0usize;
        let mut failed = 0usize;
        for (i, prepared_row) in &prepared {
            match prepared_row {
                Ok(u) => {
                    let status = if existing_names.contains(u.name.as_str()) {
                        updated += 1;
                        "updated"
                    } else {
                        created += 1;
                        "created"
                    };
                    rows_out.push(BulkImportRow {
                        index: *i,
                        name: u.name.clone(),
                        status: status.to_string(),
                        error: None,
                    });
                }
                Err(e) => {
                    failed += 1;
                    rows_out.push(BulkImportRow {
                        index: *i,
                        name: req.rows[*i].name.clone(),
                        status: "failed".to_string(),
                        error: Some(e.clone()),
                    });
                }
            }
        }
        return Json(BulkImportResult {
            created,
            updated,
            failed,
            dry_run: true,
            rows: rows_out,
        })
        .into_response();
    }

    // Commit phase. Snapshot existing names BEFORE persisting so we can
    // classify each applied row as created vs updated. Failed rows already
    // have entries in `rows_out`; valid rows are appended after persist
    // succeeds (so the order in `rows_out` matches the input).
    let mut rows_out: Vec<BulkImportRow> = Vec::new();
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut failed = 0usize;

    let pre_existing: std::collections::HashSet<String> = state
        .kernel
        .config_ref()
        .users
        .iter()
        .map(|u| u.name.clone())
        .collect();

    let mut to_apply: Vec<(usize, UserConfig)> = Vec::new();
    for (i, prepared_row) in prepared.into_iter() {
        match prepared_row {
            Ok(u) => to_apply.push((i, u)),
            Err(e) => {
                failed += 1;
                rows_out.push(BulkImportRow {
                    index: i,
                    name: req.rows[i].name.clone(),
                    status: "failed".to_string(),
                    error: Some(e),
                });
            }
        }
    }

    let payload: Vec<UserConfig> = to_apply.iter().map(|(_, u)| u.clone()).collect();
    let result = persist_users(&state, move |users| {
        for new_u in &payload {
            if let Some(idx) = users.iter().position(|u| u.name == new_u.name) {
                // RBAC M3 (#3205) + M5 (#3203): preserve existing per-
                // user policy and budget when a CSV row updates an
                // existing user — same reasoning as `update_user`.
                let preserved = users[idx].clone();
                users[idx] = UserConfig {
                    budget: preserved.budget,
                    tool_policy: preserved.tool_policy,
                    tool_categories: preserved.tool_categories,
                    memory_access: preserved.memory_access,
                    channel_tool_rules: preserved.channel_tool_rules,
                    ..new_u.clone()
                };
            } else {
                users.push(new_u.clone());
            }
        }
        Ok(())
    })
    .await;

    match result {
        Ok(()) => {
            for (i, u) in to_apply {
                let status = if pre_existing.contains(&u.name) {
                    updated += 1;
                    "updated"
                } else {
                    created += 1;
                    "created"
                };
                rows_out.push(BulkImportRow {
                    index: i,
                    name: u.name,
                    status: status.to_string(),
                    error: None,
                });
            }
            // Stable ordering for callers that diff against the input row
            // index — failures may have been pushed first.
            rows_out.sort_by_key(|r| r.index);
            Json(BulkImportResult {
                created,
                updated,
                failed,
                dry_run: false,
                rows: rows_out,
            })
            .into_response()
        }
        Err(PersistError::BadRequest(m)) => err_response(StatusCode::BAD_REQUEST, m),
        Err(PersistError::Conflict(m)) => err_response(StatusCode::CONFLICT, m),
        Err(PersistError::NotFound(m)) => err_response(StatusCode::NOT_FOUND, m),
        Err(PersistError::Internal(m)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, m),
    }
}

/// Rotate a user's API key.
///
/// Generates a fresh 32-byte random plaintext key, hashes it with Argon2id,
/// stores the hash in `config.toml`, and **swaps the live in-memory snapshot
/// the auth middleware reads from** so the next request that presents the
/// old plaintext token immediately fails authentication. Without the live
/// swap a leaked key could only be revoked by restarting the daemon — which
/// defeats the point of rotation. See the `user_api_keys` field on
/// [`AppState`] for the shared `Arc<RwLock<…>>` that makes this work.
///
/// Owner-only, gated by `is_owner_only_write` in `middleware.rs` — same
/// blast radius as user create/delete. The request takes no body; the new
/// plaintext key is returned in the response and is **never written to the
/// audit log or echoed again**.
///
/// Dashboard sessions (`AppState.active_sessions`) are intentionally NOT
/// invalidated here. The active-sessions store is keyed by an opaque token
/// and the stored [`crate::password_hash::SessionToken`] does not carry a
/// `user_id` — it tracks the single shared dashboard credential pair
/// (`dashboard_user` / `dashboard_pass`), not the per-user API keys this
/// endpoint rotates. The two auth surfaces are independent: a per-user
/// bearer-token caller never lands in `active_sessions` at all (see
/// `middleware.rs:618`), so the per-user key swap completes the kill on
/// its own. If a future change ties dashboard sessions to a `UserId`, we
/// can extend `sessions_invalidated` to include those evictions; today
/// the count reflects the per-user-bearer-token kill, which is the actual
/// revocation that matters for this surface.
#[utoipa::path(
    post,
    path = "/api/users/{name}/rotate-key",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    responses(
        (status = 200, description = "Key rotated. `new_api_key` is the only time the plaintext is exposed — the server cannot reproduce it later.", body = RotateKeyResponse),
        (status = 404, description = "Not found"),
    )
)]
pub async fn rotate_user_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    caller: Option<Extension<AuthenticatedApiUser>>,
) -> impl IntoResponse {
    // 32-byte random plaintext token, hex-encoded (64 chars). Mirrors the
    // shape of `password_hash::generate_session_token` so operator tooling
    // that already knows how to handle session tokens accepts these too.
    let new_plaintext = generate_api_key_plaintext();
    let new_hash = match crate::password_hash::hash_password(&new_plaintext) {
        Ok(h) => h,
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("hash failed: {e}"),
            );
        }
    };

    let target = name.clone();
    // Capture the OLD `api_key_hash` (pre-rotation) so we can compute a
    // short audit fingerprint AFTER the persist succeeds — without it
    // an audit entry like `"api_key rotated by alice for user bob"`
    // is forensically useless when correlating with an authentication
    // log line "auth failed with leaked key X". The fingerprint is the
    // first 8 hex chars of `sha256(old_argon2_hash)` — a hash-of-hash —
    // so we never write any plaintext or any reversibly-related material
    // into the audit log; the operator just gets a stable correlation
    // ID for the just-revoked credential.
    let result = persist_users(
        &state,
        move |users| -> Result<Option<String>, PersistError> {
            let idx = users
                .iter()
                .position(|u| u.name == target)
                .ok_or_else(|| PersistError::NotFound(format!("user '{target}' not found")))?;
            let old_hash = users[idx].api_key_hash.clone();
            // Preserve every other field — rotation only swaps `api_key_hash`,
            // it must not zero out budget / RBAC M3 policy / channel bindings.
            users[idx].api_key_hash = Some(new_hash);
            Ok(old_hash)
        },
    )
    .await;

    // The post-rotation `UserConfig` isn't part of the response shape (no
    // leakage of the new hash) but we still need to drive `persist_users`
    // through the on-disk write + reload + middleware-snapshot refresh
    // before serializing the success body.
    let old_hash = match result {
        Ok(h) => h,
        Err(e) => {
            return match e {
                PersistError::NotFound(m) => err_response(StatusCode::NOT_FOUND, m),
                PersistError::BadRequest(m) => err_response(StatusCode::BAD_REQUEST, m),
                PersistError::Conflict(m) => err_response(StatusCode::CONFLICT, m),
                PersistError::Internal(m) => err_response(StatusCode::INTERNAL_SERVER_ERROR, m),
            };
        }
    };

    // `persist_users` already refreshed `state.user_api_keys` after the
    // kernel reload — count the swapped entry for the wire response so
    // operators can see the kill happened in the same hop. We resolve the
    // count by inspecting the live snapshot for the rotated user instead
    // of trusting `persist_users` to return it (the in-place refresh is
    // best-effort with respect to ordering).
    let sessions_invalidated = state
        .user_api_keys
        .read()
        .await
        .iter()
        .filter(|u| u.name == name)
        .count();

    // Audit-record the rotation. Detail names the actor (caller) so the
    // hash-chained log answers "who rotated whose key" without echoing
    // the plaintext — that stays in the response body only. The
    // `(old: <fp>)` fragment is the first 8 hex chars of
    // `sha256(old_argon2_hash)` so operators can correlate this audit
    // entry with prior authentication-failure log lines mentioning the
    // same fingerprint after a leaked key is reported. The fingerprint
    // is a hash-of-hash, so it leaks no key material — the underlying
    // value is already an Argon2id PHC string, and the truncated SHA-256
    // is one-way over that. If the user had no prior key (first-time
    // assignment via rotate) we record `(old: none)` so the field is
    // always present and parseable downstream.
    let old_fp = old_hash
        .as_deref()
        .map(api_key_hash_fingerprint)
        .unwrap_or_else(|| "none".to_string());
    let actor = caller
        .as_ref()
        .map(|c| c.0.name.clone())
        .unwrap_or_else(|| "system".to_string());
    let actor_user_id = caller.as_ref().map(|c| c.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_runtime::audit::AuditAction::RoleChange,
        format!("api_key rotated by {actor} for user {name} (old: {old_fp})"),
        "completed",
        actor_user_id,
        Some("api".to_string()),
    );

    Json(RotateKeyResponse {
        status: "ok".to_string(),
        new_api_key: new_plaintext,
        sessions_invalidated,
    })
    .into_response()
}

/// Generate a 32-byte (256-bit) random API key plaintext.
///
/// Hex-encoded so the result is URL-safe and matches the existing
/// `generate_session_token` shape (64 chars). We don't reuse
/// `generate_session_token` directly because that returns a
/// [`crate::password_hash::SessionToken`] with a creation timestamp, which
/// is the wrong type — API keys don't carry a TTL.
fn generate_api_key_plaintext() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Short forensic fingerprint of an Argon2 PHC `api_key_hash` string.
///
/// Returns the first 8 hex chars of `sha256(input)` — a hash-of-hash so
/// nothing key-related is reversible from the result, even with the
/// rotated key in hand. Used by the rotate-key audit detail to tag the
/// just-revoked credential, so an operator chasing a "leaked key"
/// authentication-failure line can correlate the failed token's
/// fingerprint (computed the same way at the auth layer when a future
/// telemetry change adds it) against the rotation entry that revoked it.
///
/// Length is 8 hex chars (32 bits) — enough to distinguish individual
/// rotations within a normal-sized user table without making the audit
/// detail noisy. Truncation is safe here because the value is for human
/// pattern-matching, not a cryptographic identifier.
fn api_key_hash_fingerprint(api_key_hash: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(api_key_hash.as_bytes());
    let mut s = String::with_capacity(8);
    for b in digest.iter().take(4) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Rebuild the `ApiUserAuth` records from the kernel's current `[[users]]`
/// table. Mirrors `server.rs::configured_user_api_keys`, kept private to
/// `users.rs` so the persistence path can call it without exposing
/// `server.rs` internals across the route boundary.
fn rebuild_api_user_records(state: &AppState) -> Vec<ApiUserAuth> {
    state
        .kernel
        .config_ref()
        .users
        .iter()
        .filter_map(|user| {
            let api_key_hash = user.api_key_hash.as_deref()?.trim();
            if api_key_hash.is_empty() {
                return None;
            }
            Some(ApiUserAuth {
                name: user.name.clone(),
                role: UserRole::from_str_role(&user.role),
                api_key_hash: api_key_hash.to_string(),
                user_id: UserId::from_name(&user.name),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

pub(crate) enum PersistError {
    BadRequest(String),
    Conflict(String),
    NotFound(String),
    Internal(String),
}

/// Read `config.toml`, run `mutate` on a clone of the current `users`
/// vector, then rewrite the `[[users]]` array-of-tables and reload the
/// kernel. The mutator returns a `PersistError` to abort the write with a
/// chosen status code, or any `R` to be threaded back to the caller —
/// `update_user` uses this to surface the post-merge `UserConfig`
/// (including preserved RBAC M3 policy fields) without an out-of-band
/// `Arc<Mutex>` capture.
pub(crate) async fn persist_users<F, R>(state: &Arc<AppState>, mutate: F) -> Result<R, PersistError>
where
    F: FnOnce(&mut Vec<UserConfig>) -> Result<R, PersistError>,
{
    let _guard = state.config_write_lock.lock().await;

    let mut users: Vec<UserConfig> = state.kernel.config_ref().users.clone();
    let captured = mutate(&mut users)?;

    let config_path = state.kernel.home_dir().join("config.toml");
    if config_path.file_name().and_then(|n| n.to_str()) != Some("config.toml")
        || config_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(PersistError::BadRequest(
            "invalid config file path".to_string(),
        ));
    }

    // Read the existing file. A read failure on an existing file (permission
    // denied, hardware fault, …) MUST abort — falling back to "" would
    // silently drop every other section in `config.toml` (agents, providers,
    // taint rules, etc.) on the next write. The caller's
    // `backups/config.toml.prev` from the previous successful write is the
    // recovery point of last resort.
    let raw = if config_path.exists() {
        std::fs::read_to_string(&config_path).map_err(|e| {
            PersistError::Internal(format!("could not read existing config.toml: {e}"))
        })?
    } else {
        String::new()
    };
    // Parse with `toml_edit` so we preserve comments / formatting / unrelated
    // sections. A parse failure means the on-disk file is already corrupt;
    // refuse to write rather than overwriting with an empty document, which
    // would clobber every other section the operator is hand-editing.
    let mut doc: toml_edit::DocumentMut = raw.parse().map_err(|e| {
        PersistError::Internal(format!(
            "config.toml is not valid TOML — refusing to overwrite: {e}"
        ))
    })?;

    // Replace the entire `users` key with a freshly built array-of-tables
    // (or remove it when the vector is empty so we don't leave a stranded
    // `users = []` behind).
    if users.is_empty() {
        doc.remove("users");
    } else {
        let mut aot = toml_edit::ArrayOfTables::new();
        for u in &users {
            // Serialize the whole UserConfig via serde so RBAC M3 (#3205)
            // fields (`tool_policy` / `tool_categories` / `memory_access`
            // / `channel_tool_rules`) survive the round-trip. Earlier
            // drafts hand-emitted the four M6 fields and silently dropped
            // the M3 ones — the per-user policy got reset every time the
            // dashboard edited a name or role. The `#[serde(skip_serializing_if)]`
            // on each optional field keeps the on-disk shape minimal.
            //
            // We go through `toml_edit::ser::to_document` (NOT `toml::to_string`)
            // because the source struct interleaves scalar fields with
            // nested tables (`channel_bindings` table sits before the
            // `api_key_hash` scalar), which the strict `toml` serializer
            // rejects with `ValueAfterTable`. `toml_edit` reorders for us.
            let single = toml_edit::ser::to_document(u)
                .map_err(|e| PersistError::Internal(format!("serialize user '{}': {e}", u.name)))?;
            aot.push(single.as_table().clone());
        }
        doc.insert("users", toml_edit::Item::ArrayOfTables(aot));
    }

    let new_toml = doc.to_string();
    let mut parsed: librefang_types::config::KernelConfig = toml::from_str(&new_toml)
        .map_err(|e| PersistError::Internal(format!("invalid config after edit: {e}")))?;
    parsed.clamp_bounds();
    if let Err(errors) = librefang_kernel::config_reload::validate_config_for_reload(&parsed) {
        return Err(PersistError::BadRequest(format!(
            "invalid config: {}",
            errors.join("; ")
        )));
    }

    if config_path.exists() {
        if let Some(home_dir) = config_path.parent() {
            let backups_dir = home_dir.join("backups");
            if std::fs::create_dir_all(&backups_dir).is_ok() {
                let _ = std::fs::copy(&config_path, backups_dir.join("config.toml.prev"));
            }
        }
    }

    // Acquire the auth-snapshot write lock BEFORE the disk write so the
    // middleware can never observe an intermediate state where the new
    // hash is already on disk (and reachable through `kernel.config_ref()`)
    // but the `state.user_api_keys` Vec the middleware actually verifies
    // against still holds the OLD record. Earlier ordering was
    // `write file → reload → acquire lock → swap`, which left a small
    // race window: any request landing between the file write and the
    // snapshot swap would hit the stale `Vec<ApiUserAuth>` and pass auth
    // with the just-rotated plaintext — bounded by `reload_config`
    // latency (a few ms under load) but exploitable. Holding the lock
    // across persist + reload + swap means every concurrent auth check
    // either sees the pre-rotation snapshot OR blocks on this writer
    // until the swap completes; never an in-between read where the
    // on-disk hash and the live `Vec` disagree. Auth blocking during
    // rotation is the correct behavior (the operator just rotated;
    // concurrent requests with the old key SHOULD fail).
    let mut user_keys_guard = state.user_api_keys.write().await;

    std::fs::write(&config_path, &new_toml)
        .map_err(|e| PersistError::Internal(format!("write failed: {e}")))?;

    if let Err(e) = state.kernel.reload_config().await {
        // The file is on disk; surface a soft error so the dashboard can
        // show the reason without rolling back. The next manual reload (or
        // restart) will pick it up. Drop the guard so subsequent reads
        // aren't blocked on a failed-write path — the on-disk state has
        // moved forward, and a stale `Vec<ApiUserAuth>` is no worse than
        // pre-fix behaviour for this failure mode.
        drop(user_keys_guard);
        tracing::warn!(error = %e, "user config reload failed after write");
        return Err(PersistError::Internal(format!("reload failed: {e}")));
    }

    // Refresh the in-memory `ApiUserAuth` snapshot the auth middleware
    // reads from. Without this swap, mutations to `users[].api_key_hash`
    // (rotate-key, update_user, import_users) only become effective after
    // a daemon restart — the bug rotate-key exists to fix. Done in the
    // shared helper so every user-mutation path benefits, not only the
    // rotation endpoint. Still holding `user_keys_guard` here so the swap
    // is atomic with the persist + reload above.
    *user_keys_guard = rebuild_api_user_records(state.as_ref());
    drop(user_keys_guard);

    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        "users updated".to_string(),
        "completed",
    );

    Ok(captured)
}

// ---------------------------------------------------------------------------
// RBAC M3 — per-user policy GET / PUT (#3054 Phase 2 wiring)
// ---------------------------------------------------------------------------

/// View / wire shape for the per-user RBAC M3 policy slice.
///
/// Each top-level field is independently nullable so the dashboard can edit
/// one section at a time without restating the others. On PUT a `None`
/// clears that field; a missing key preserves the existing value (see
/// [`update_user_policy`]). On GET every field is always present (`null`
/// when the user has no opinion configured) so the client can render a
/// stable form.
// `utoipa::ToSchema` is intentionally NOT derived: the inner types
// (`UserToolPolicy`, `UserMemoryAccess`, `ChannelToolPolicy`, …) live in
// `librefang-types` and don't implement `ToSchema` to keep that crate
// free of OpenAPI deps. The handler attribute below points utoipa at
// `serde_json::Value` for documentation; the wire shape is still pinned
// by serde so callers see the real JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPolicyView {
    #[serde(default)]
    pub tool_policy: Option<UserToolPolicy>,
    #[serde(default)]
    pub tool_categories: Option<UserToolCategories>,
    #[serde(default)]
    pub memory_access: Option<UserMemoryAccess>,
    #[serde(default)]
    pub channel_tool_rules: HashMap<String, ChannelToolPolicy>,
}

impl From<&UserConfig> for UserPolicyView {
    fn from(cfg: &UserConfig) -> Self {
        Self {
            tool_policy: cfg.tool_policy.clone(),
            tool_categories: cfg.tool_categories.clone(),
            memory_access: cfg.memory_access.clone(),
            channel_tool_rules: cfg.channel_tool_rules.clone(),
        }
    }
}

/// PUT body decoder. We accept the request as a raw JSON object so we can
/// distinguish three states per key:
///   * key absent       → preserve existing value
///   * key present null → clear
///   * key present obj  → replace
///
/// `Option<serde_json::Value>` would collapse `null` to `None` (serde's
/// default behaviour for `Option<T>`), making absent and explicit-null
/// indistinguishable. Using `serde_json::Map` directly preserves both via
/// `Map::contains_key` + `Value::is_null`.
const KNOWN_POLICY_KEYS: &[&str] = &[
    "tool_policy",
    "tool_categories",
    "memory_access",
    "channel_tool_rules",
];

/// Reject lists with empty/whitespace-only entries or duplicates so the
/// resolver can never trip over a `""` glob (matches every tool) or a
/// duplicate-as-typo that silently shadows the user's intended pattern.
fn validate_string_list(label: &str, items: &[String]) -> Result<(), String> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "{label} contains an empty or whitespace-only entry"
            ));
        }
        if !seen.insert(trimmed) {
            return Err(format!("{label} contains duplicate entry '{trimmed}'"));
        }
    }
    Ok(())
}

fn validate_tool_policy(p: &UserToolPolicy) -> Result<(), String> {
    validate_string_list("tool_policy.allowed_tools", &p.allowed_tools)?;
    validate_string_list("tool_policy.denied_tools", &p.denied_tools)?;
    Ok(())
}

fn validate_tool_categories(c: &UserToolCategories) -> Result<(), String> {
    validate_string_list("tool_categories.allowed_groups", &c.allowed_groups)?;
    validate_string_list("tool_categories.denied_groups", &c.denied_groups)?;
    Ok(())
}

fn validate_memory_access(a: &UserMemoryAccess) -> Result<(), String> {
    validate_string_list("memory_access.readable_namespaces", &a.readable_namespaces)?;
    validate_string_list("memory_access.writable_namespaces", &a.writable_namespaces)?;
    // RBAC invariant: a user can only write where they can read. Without
    // this check an admin could grant `writable=["proactive"]` while
    // leaving `readable=[]`, producing a write-only ACL the kernel's
    // `can_read`/`can_write` paths can't reason about consistently
    // (writes succeed, but the same user can't read the entry back).
    // Enforced newly here — there is no upstream validation today; the
    // kernel resolver assumes the config is well-formed.
    for w in &a.writable_namespaces {
        if !a.readable_namespaces.iter().any(|r| r == w) {
            return Err(format!(
                "memory_access.writable_namespaces['{w}'] is not in readable_namespaces \
                 (writable must be a subset of readable)"
            ));
        }
    }
    Ok(())
}

/// Channel-name keys are written verbatim into `config.toml` and matched
/// against channel-adapter identifiers (`telegram`, `slack`, `discord`,
/// `whatsapp`, `feishu`, `dingtalk`, …). The trim-then-empty check alone
/// lets through embedded newlines, control chars, multi-KB blobs, and
/// non-ASCII keys that would either corrupt the TOML round-trip or never
/// match a real adapter. Cap length and lock the charset here at the same
/// boundary that already validates the inner allow/deny lists.
const MAX_CHANNEL_NAME_LEN: usize = 64;

fn validate_channel_rules(rules: &HashMap<String, ChannelToolPolicy>) -> Result<(), String> {
    for (channel, rule) in rules {
        let trimmed = channel.trim();
        if trimmed.is_empty() {
            return Err("channel_tool_rules contains an empty channel name".to_string());
        }
        if trimmed.len() > MAX_CHANNEL_NAME_LEN {
            return Err(format!(
                "channel_tool_rules contains invalid channel name {trimmed:?}: \
                 longer than {MAX_CHANNEL_NAME_LEN} chars"
            ));
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(format!(
                "channel_tool_rules contains invalid channel name {trimmed:?}: \
                 embedded control characters are not allowed"
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "channel_tool_rules contains invalid channel name {trimmed:?}: \
                 must match [a-zA-Z0-9_-]+"
            ));
        }
        validate_string_list(
            &format!("channel_tool_rules['{trimmed}'].allowed_tools"),
            &rule.allowed_tools,
        )?;
        validate_string_list(
            &format!("channel_tool_rules['{trimmed}'].denied_tools"),
            &rule.denied_tools,
        )?;
    }
    Ok(())
}

/// Decode a single key out of the raw PUT body into one of three states.
enum FieldUpdate<T> {
    Absent,
    Clear,
    Set(T),
}

fn decode_field<T: serde::de::DeserializeOwned>(
    label: &str,
    body: &serde_json::Map<String, serde_json::Value>,
) -> Result<FieldUpdate<T>, String> {
    match body.get(label) {
        None => Ok(FieldUpdate::Absent),
        Some(serde_json::Value::Null) => Ok(FieldUpdate::Clear),
        Some(v) => serde_json::from_value::<T>(v.clone())
            .map(FieldUpdate::Set)
            .map_err(|e| format!("{label} payload is invalid: {e}")),
    }
}

#[utoipa::path(
    get,
    path = "/api/users/{name}/policy",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    responses(
        (status = 200, description = "Per-user policy slice", body = serde_json::Value),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_user_policy(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_ref();
    match cfg.users.iter().find(|u| u.name == name) {
        Some(u) => Json(UserPolicyView::from(u)).into_response(),
        None => err_response(StatusCode::NOT_FOUND, format!("user '{name}' not found")),
    }
}

/// PUT /api/users/{name}/policy — upsert the per-user RBAC M3 policy slice.
///
/// Body shape:
/// ```json
/// {
///   "tool_policy":        {...} | null,
///   "tool_categories":    {...} | null,
///   "memory_access":      {...} | null,
///   "channel_tool_rules": {"telegram": {...}, ...}
/// }
/// ```
///
/// Each top-level key is independently optional:
///   * key absent       → preserve existing value (so a partial edit
///                        from the dashboard never clobbers a section
///                        the form didn't expose),
///   * key present null → clear the field,
///   * key present obj  → replace.
///
/// `channel_tool_rules` collapses absent/null to "preserve" (use an empty
/// object `{}` to clear all rules), since a `null` map is rarely what an
/// operator means.
///
/// Owner-only — covered by `middleware::is_owner_only_write` which gates
/// every mutating call under `/api/users*`. Per-user policy edits change
/// someone's authorization surface, which is unambiguously an Owner action.
#[utoipa::path(
    put,
    path = "/api/users/{name}/policy",
    tag = "users",
    params(("name" = String, Path, description = "User name (case-sensitive)")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Updated user policy slice", body = serde_json::Value),
        (status = 400, description = "Validation error"),
        (status = 404, description = "Not found"),
    )
)]
pub async fn update_user_policy(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(raw): Json<serde_json::Value>,
) -> impl IntoResponse {
    let body = match raw {
        serde_json::Value::Object(map) => map,
        _ => {
            return err_response(
                StatusCode::BAD_REQUEST,
                "request body must be a JSON object",
            )
        }
    };

    // Reject unknown top-level keys early so a typo (e.g. `tool_polices`)
    // doesn't silently no-op — without this the absent/null distinction
    // turns every typo into "preserve existing".
    for k in body.keys() {
        if !KNOWN_POLICY_KEYS.iter().any(|known| known == k) {
            return err_response(
                StatusCode::BAD_REQUEST,
                format!(
                    "unknown field '{k}' (expected one of: {})",
                    KNOWN_POLICY_KEYS.join(", ")
                ),
            );
        }
    }

    let tool_policy_update = match decode_field::<UserToolPolicy>("tool_policy", &body) {
        Ok(v) => v,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let tool_categories_update = match decode_field::<UserToolCategories>("tool_categories", &body)
    {
        Ok(v) => v,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let memory_access_update = match decode_field::<UserMemoryAccess>("memory_access", &body) {
        Ok(v) => v,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let channel_rules_update =
        match decode_field::<HashMap<String, ChannelToolPolicy>>("channel_tool_rules", &body) {
            Ok(v) => v,
            Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
        };

    if let FieldUpdate::Set(p) = &tool_policy_update {
        if let Err(e) = validate_tool_policy(p) {
            return err_response(StatusCode::BAD_REQUEST, e);
        }
    }
    if let FieldUpdate::Set(c) = &tool_categories_update {
        if let Err(e) = validate_tool_categories(c) {
            return err_response(StatusCode::BAD_REQUEST, e);
        }
    }
    if let FieldUpdate::Set(a) = &memory_access_update {
        if let Err(e) = validate_memory_access(a) {
            return err_response(StatusCode::BAD_REQUEST, e);
        }
    }
    if let FieldUpdate::Set(r) = &channel_rules_update {
        if let Err(e) = validate_channel_rules(r) {
            return err_response(StatusCode::BAD_REQUEST, e);
        }
    }

    let target = name.clone();
    match persist_users(&state, move |users| -> Result<UserConfig, PersistError> {
        let idx = users
            .iter()
            .position(|u| u.name == target)
            .ok_or_else(|| PersistError::NotFound(format!("user '{target}' not found")))?;
        match tool_policy_update {
            FieldUpdate::Absent => {}
            FieldUpdate::Clear => users[idx].tool_policy = None,
            FieldUpdate::Set(p) => users[idx].tool_policy = Some(p),
        }
        match tool_categories_update {
            FieldUpdate::Absent => {}
            FieldUpdate::Clear => users[idx].tool_categories = None,
            FieldUpdate::Set(c) => users[idx].tool_categories = Some(c),
        }
        match memory_access_update {
            FieldUpdate::Absent => {}
            FieldUpdate::Clear => users[idx].memory_access = None,
            FieldUpdate::Set(a) => users[idx].memory_access = Some(a),
        }
        match channel_rules_update {
            FieldUpdate::Absent | FieldUpdate::Clear => {}
            FieldUpdate::Set(r) => users[idx].channel_tool_rules = r,
        }
        Ok(users[idx].clone())
    })
    .await
    {
        Ok(final_cfg) => (StatusCode::OK, Json(UserPolicyView::from(&final_cfg))).into_response(),
        Err(PersistError::NotFound(m)) => err_response(StatusCode::NOT_FOUND, m),
        Err(PersistError::BadRequest(m)) => err_response(StatusCode::BAD_REQUEST, m),
        Err(PersistError::Conflict(m)) => err_response(StatusCode::CONFLICT, m),
        Err(PersistError::Internal(m)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, m),
    }
}
