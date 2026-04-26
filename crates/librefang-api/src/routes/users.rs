//! User RBAC management endpoints (Phase 4 / RBAC M6).
//!
//! These endpoints expose CRUD over `[[users]]` entries in `config.toml`,
//! plus a bulk-import endpoint used by the dashboard CSV-import wizard.
//!
//! Auth: NOT in the public allowlist — every request goes through the
//! authenticated middleware path. Loopback / configured api_key / dashboard
//! session all satisfy that. We deliberately do NOT layer a second
//! `UserRole`-based check here: the M3 per-user-policy slice (#3205) will
//! introduce dashboard-session-aware role propagation; until then the
//! existing api_key gate is the single source of truth and the closed-by-
//! default middleware is what protects this surface.
//!
//! Persistence model: we read the live `KernelConfig`, mutate the `users`
//! vector, then rewrite the `[[users]]` array-of-tables in `config.toml`
//! using `toml_edit` so unrelated comments/sections are preserved. After
//! every successful write we trigger a kernel reload so the in-memory
//! `AuthManager` picks up the change without restart.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::config::UserConfig;
use serde::{Deserialize, Serialize};

use super::AppState;

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
}

// ---------------------------------------------------------------------------
// View models
// ---------------------------------------------------------------------------

/// Sanitized user view returned over the wire — never echoes the
/// `api_key_hash` value, only its presence.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct UserView {
    pub name: String,
    pub role: String,
    pub channel_bindings: HashMap<String, String>,
    pub has_api_key: bool,
}

impl From<&UserConfig> for UserView {
    fn from(cfg: &UserConfig) -> Self {
        Self {
            name: cfg.name.clone(),
            role: cfg.role.clone(),
            channel_bindings: cfg.channel_bindings.clone(),
            has_api_key: cfg
                .api_key_hash
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
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

    let new_cfg = UserConfig {
        name: req.name.trim().to_string(),
        role,
        channel_bindings: req.channel_bindings,
        api_key_hash: req.api_key_hash.filter(|s| !s.trim().is_empty()),
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

    // The PUT body's `name` is treated as the desired final name; the URL
    // path identifies the user being updated. Allow rename so the dashboard
    // can edit the display name without a delete-and-recreate dance.
    let renamed_to = req.name.trim().to_string();
    let updated = UserConfig {
        name: renamed_to.clone(),
        role,
        channel_bindings: req.channel_bindings,
        api_key_hash: req.api_key_hash.filter(|s| !s.trim().is_empty()),
    };

    let target_existing = name.clone();
    let updated_clone = updated.clone();
    match persist_users(&state, move |users| {
        let idx = users
            .iter()
            .position(|u| u.name == target_existing)
            .ok_or_else(|| PersistError::NotFound(format!("user '{target_existing}' not found")))?;
        // If renaming, ensure no collision with another existing user.
        if updated_clone.name != target_existing
            && users.iter().any(|u| u.name == updated_clone.name)
        {
            return Err(PersistError::Conflict(format!(
                "another user named '{}' already exists",
                updated_clone.name
            )));
        }
        users[idx] = updated_clone;
        Ok(())
    })
    .await
    {
        Ok(()) => (StatusCode::OK, Json(UserView::from(&updated))).into_response(),
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
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","deleted":name})),
        )
            .into_response(),
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
            Ok(UserConfig {
                name: row.name.trim().to_string(),
                role,
                channel_bindings: row.channel_bindings.clone(),
                api_key_hash: row.api_key_hash.clone().filter(|s| !s.trim().is_empty()),
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
                users[idx] = new_u.clone();
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

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

enum PersistError {
    BadRequest(String),
    Conflict(String),
    NotFound(String),
    Internal(String),
}

/// Read `config.toml`, run `mutate` on a clone of the current `users`
/// vector, then rewrite the `[[users]]` array-of-tables and reload the
/// kernel. The mutator returns a `PersistError` to abort the write with a
/// chosen status code.
async fn persist_users<F>(state: &Arc<AppState>, mutate: F) -> Result<(), PersistError>
where
    F: FnOnce(&mut Vec<UserConfig>) -> Result<(), PersistError>,
{
    let _guard = state.config_write_lock.lock().await;

    let mut users: Vec<UserConfig> = state.kernel.config_ref().users.clone();
    mutate(&mut users)?;

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

    let raw = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = raw.parse().unwrap_or_default();

    // Replace the entire `users` key with a freshly built array-of-tables
    // (or remove it when the vector is empty so we don't leave a stranded
    // `users = []` behind).
    if users.is_empty() {
        doc.remove("users");
    } else {
        let mut aot = toml_edit::ArrayOfTables::new();
        for u in &users {
            let mut t = toml_edit::Table::new();
            t["name"] = toml_edit::value(u.name.clone());
            t["role"] = toml_edit::value(u.role.clone());
            if !u.channel_bindings.is_empty() {
                let mut bindings = toml_edit::Table::new();
                bindings.set_implicit(false);
                // Stable ordering for deterministic diffs.
                let mut keys: Vec<&String> = u.channel_bindings.keys().collect();
                keys.sort();
                for k in keys {
                    bindings[k] =
                        toml_edit::value(u.channel_bindings.get(k).cloned().unwrap_or_default());
                }
                t["channel_bindings"] = toml_edit::Item::Table(bindings);
            }
            if let Some(hash) = &u.api_key_hash {
                if !hash.trim().is_empty() {
                    t["api_key_hash"] = toml_edit::value(hash.clone());
                }
            }
            aot.push(t);
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

    std::fs::write(&config_path, &new_toml)
        .map_err(|e| PersistError::Internal(format!("write failed: {e}")))?;

    if let Err(e) = state.kernel.reload_config().await {
        // The file is on disk; surface a soft error so the dashboard can
        // show the reason without rolling back. The next manual reload (or
        // restart) will pick it up.
        tracing::warn!(error = %e, "user config reload failed after write");
        return Err(PersistError::Internal(format!("reload failed: {e}")));
    }

    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        "users updated".to_string(),
        "completed",
    );

    Ok(())
}
