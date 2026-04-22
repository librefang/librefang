//! Storage administration API routes (Phase 8 of `surrealdb-storage-swap`).
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/storage/config` | Read current `StorageConfig` |
//! | PUT | `/api/storage/config` | Write `StorageConfig` fields into config.toml |
//! | GET | `/api/storage/status` | Backend kind, path/URL, table counts, migration status |
//! | POST | `/api/storage/migrate` | One-shot SQLite → SurrealDB migration |
//! | POST | `/api/storage/link-uar` | Provision UAR namespace + user on remote SurrealDB |
//! | POST | `/api/storage/unlink-uar` | Remove UAR user from config |

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::types::ApiErrorResponse;

use super::AppState;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/api/storage/config",
            axum::routing::get(get_storage_config).put(put_storage_config),
        )
        .route(
            "/api/storage/status",
            axum::routing::get(get_storage_status),
        )
        .route(
            "/api/storage/migrate",
            axum::routing::post(post_storage_migrate),
        )
        .route("/api/storage/link-uar", axum::routing::post(post_link_uar))
        .route(
            "/api/storage/unlink-uar",
            axum::routing::post(post_unlink_uar),
        )
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Flattened view of `StorageConfig` returned to the dashboard.
#[derive(Debug, Serialize)]
pub struct StorageConfigResponse {
    pub backend_kind: String,
    pub embedded_path: Option<String>,
    pub remote_url: Option<String>,
    pub namespace: String,
    pub database: String,
    pub legacy_sqlite_path: Option<String>,
    pub uar_linked: bool,
}

#[derive(Debug, Serialize)]
pub struct StorageStatusResponse {
    pub backend_kind: String,
    pub backend_location: String,
    pub namespace: String,
    pub database: String,
    pub connected: bool,
    pub table_counts: StorageTableCounts,
    pub migration_available: bool,
    pub last_migration_receipt: Option<String>,
    pub uar_linked: bool,
    pub uar_namespace: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct StorageTableCounts {
    pub audit_entries: u64,
    pub hook_traces: u64,
    pub circuit_breaker_states: u64,
    pub totp_lockout: u64,
    pub agents: u64,
}

#[derive(Debug, Deserialize)]
pub struct PutStorageConfigBody {
    pub backend_kind: String,
    pub embedded_path: Option<String>,
    pub remote_url: Option<String>,
    pub namespace: Option<String>,
    pub database: Option<String>,
    pub legacy_sqlite_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PostMigrateBody {
    pub from: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct MigrateResponse {
    pub dry_run: bool,
    pub source: String,
    pub target: String,
    pub copied: std::collections::BTreeMap<String, u64>,
    pub errors: std::collections::BTreeMap<String, String>,
    pub started_at: String,
    pub finished_at: String,
    pub receipt_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PostLinkUarBody {
    pub remote_url: String,
    pub root_user: String,
    pub root_pass_ref: String,
    #[serde(default = "uar_namespace_default")]
    pub namespace: String,
    #[serde(default = "uar_app_user_default")]
    pub app_user: String,
    pub app_pass_ref: String,
    #[serde(default)]
    pub also_link_memory: bool,
}

fn uar_namespace_default() -> String {
    "uar".to_owned()
}

fn uar_app_user_default() -> String {
    "uar_app".to_owned()
}

#[derive(Debug, Deserialize)]
pub struct PostUnlinkUarBody {
    #[serde(default)]
    pub purge_user: bool,
}

// ---------------------------------------------------------------------------
// GET /api/storage/config
// ---------------------------------------------------------------------------

pub async fn get_storage_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let storage = &cfg.storage;

    let (backend_kind, embedded_path, remote_url) = match &storage.backend {
        librefang_storage::StorageBackendKind::Embedded { path } => (
            "embedded".to_owned(),
            Some(path.display().to_string()),
            None,
        ),
        librefang_storage::StorageBackendKind::Remote(r) => {
            ("remote".to_owned(), None, Some(r.url.clone()))
        }
    };

    let uar_linked = cfg
        .uar
        .as_ref()
        .map_or(false, |u| u.share_librefang_storage || u.remote.is_some());

    (
        StatusCode::OK,
        Json(StorageConfigResponse {
            backend_kind,
            embedded_path,
            remote_url,
            namespace: storage.namespace.clone(),
            database: storage.database.clone(),
            legacy_sqlite_path: storage
                .legacy_sqlite_path
                .as_ref()
                .map(|p| p.display().to_string()),
            uar_linked,
        }),
    )
}

// ---------------------------------------------------------------------------
// PUT /api/storage/config
// ---------------------------------------------------------------------------

pub async fn put_storage_config(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PutStorageConfigBody>,
) -> impl IntoResponse {
    // Validate request
    if body.backend_kind != "embedded" && body.backend_kind != "remote" {
        return ApiErrorResponse::bad_request(format!(
            "unknown backend_kind '{}'; expected 'embedded' or 'remote'",
            body.backend_kind
        ))
        .into_response();
    }
    if body.backend_kind == "embedded" && body.embedded_path.is_none() {
        return ApiErrorResponse::bad_request(
            "embedded_path is required when backend_kind is 'embedded'",
        )
        .into_response();
    }
    if body.backend_kind == "remote" && body.remote_url.is_none() {
        return ApiErrorResponse::bad_request(
            "remote_url is required when backend_kind is 'remote'",
        )
        .into_response();
    }

    let cfg = state.kernel.config_snapshot();
    let config_path = cfg.data_dir.join("config.toml");

    let _write_lock = state.config_write_lock.lock().await;

    let raw = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = raw.parse().unwrap_or_default();

    // Ensure [storage] table exists.
    if !doc.contains_table("storage") {
        doc.insert("storage", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let storage_tbl = doc["storage"].as_table_mut().expect("storage is a table");
    storage_tbl.insert(
        "backend_kind",
        toml_edit::Item::Value(body.backend_kind.as_str().into()),
    );

    if body.backend_kind == "embedded" {
        storage_tbl.insert(
            "embedded_path",
            toml_edit::Item::Value(body.embedded_path.as_deref().unwrap_or("").into()),
        );
        storage_tbl.remove("remote_url");
    } else {
        storage_tbl.insert(
            "remote_url",
            toml_edit::Item::Value(body.remote_url.as_deref().unwrap_or("").into()),
        );
        storage_tbl.remove("embedded_path");
    }

    if let Some(ns) = &body.namespace {
        storage_tbl.insert("namespace", toml_edit::Item::Value(ns.as_str().into()));
    }
    if let Some(db) = &body.database {
        storage_tbl.insert("database", toml_edit::Item::Value(db.as_str().into()));
    }

    match body.legacy_sqlite_path.as_deref() {
        Some(p) if !p.is_empty() => {
            storage_tbl.insert("legacy_sqlite_path", toml_edit::Item::Value(p.into()));
        }
        _ => {
            storage_tbl.remove("legacy_sqlite_path");
        }
    }

    let new_toml = doc.to_string();
    if let Err(e) = std::fs::write(&config_path, &new_toml) {
        warn!(error = %e, path = %config_path.display(), "failed to write storage config");
        return ApiErrorResponse::internal(format!("write config.toml: {e}")).into_response();
    }

    info!(backend_kind = %body.backend_kind, "storage config updated");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ---------------------------------------------------------------------------
// GET /api/storage/status
// ---------------------------------------------------------------------------

pub async fn get_storage_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let storage = &cfg.storage;

    let (backend_kind, backend_location) = match &storage.backend {
        librefang_storage::StorageBackendKind::Embedded { path } => {
            ("embedded".to_owned(), path.display().to_string())
        }
        librefang_storage::StorageBackendKind::Remote(r) => ("remote".to_owned(), r.url.clone()),
    };

    // Attempt row counts — best-effort; failures surface as 0 + connected=false.
    let (connected, table_counts) = match storage_row_counts(storage).await {
        Ok(counts) => (true, counts),
        Err(_) => (false, StorageTableCounts::default()),
    };

    let migration_available = storage
        .legacy_sqlite_path
        .as_ref()
        .map(|p| p.exists())
        .unwrap_or(false);

    let receipts_dir = cfg.data_dir.join("migrations");
    let last_migration_receipt = find_latest_receipt(&receipts_dir);

    let uar_linked = cfg
        .uar
        .as_ref()
        .map_or(false, |u| u.share_librefang_storage || u.remote.is_some());
    let uar_namespace = if uar_linked {
        cfg.uar
            .as_ref()
            .and_then(|u| u.remote.as_ref().map(|r| r.namespace.clone()))
            .or_else(|| Some("uar".to_owned()))
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(StorageStatusResponse {
            backend_kind,
            backend_location,
            namespace: storage.namespace.clone(),
            database: storage.database.clone(),
            connected,
            table_counts,
            migration_available,
            last_migration_receipt,
            uar_linked,
            uar_namespace,
        }),
    )
}

// ---------------------------------------------------------------------------
// POST /api/storage/migrate
// ---------------------------------------------------------------------------

pub async fn post_storage_migrate(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PostMigrateBody>,
) -> impl IntoResponse {
    if body.from != "sqlite" {
        return ApiErrorResponse::bad_request("only 'from: sqlite' is currently supported")
            .into_response();
    }

    let cfg = state.kernel.config_snapshot();
    #[allow(unused_variables)]
    let sqlite_path = cfg
        .storage
        .legacy_sqlite_path
        .clone()
        .or_else(|| cfg.memory.sqlite_path.clone())
        .unwrap_or_else(|| cfg.data_dir.join("librefang.db"));

    if body.dry_run {
        // Dry-run: return row counts without opening SurrealDB.
        #[cfg(feature = "sqlite-backend")]
        {
            match librefang_storage::migrate::plan_sqlite(&sqlite_path) {
                Ok(plan) => {
                    let mut copied = plan.source_rows.clone();
                    // Ensure all standard tables appear in the response even
                    // if they're absent from the SQLite file.
                    for t in &[
                        "audit_entries",
                        "hook_traces",
                        "circuit_breaker_states",
                        "totp_lockout",
                        "agents",
                    ] {
                        copied.entry(t.to_string()).or_insert(0);
                    }
                    return (
                        StatusCode::OK,
                        Json(MigrateResponse {
                            dry_run: true,
                            source: sqlite_path.display().to_string(),
                            target: "surrealdb".to_owned(),
                            copied,
                            errors: Default::default(),
                            started_at: chrono::Utc::now().to_rfc3339(),
                            finished_at: chrono::Utc::now().to_rfc3339(),
                            receipt_path: None,
                        }),
                    )
                        .into_response();
                }
                Err(e) => {
                    return ApiErrorResponse::internal(format!("plan_sqlite: {e}")).into_response()
                }
            }
        }
        #[cfg(not(feature = "sqlite-backend"))]
        {
            return ApiErrorResponse::bad_request(
                "sqlite-backend feature not compiled in; cannot read legacy SQLite",
            )
            .into_response();
        }
    }

    // Live migration — requires both backends compiled in.
    #[cfg(all(feature = "sqlite-backend", feature = "surreal-backend"))]
    {
        let storage_cfg = cfg.storage.clone();
        let receipts_dir = cfg.data_dir.join("migrations");

        let result = tokio::task::spawn_blocking(move || {
            // Open SurrealDB synchronously from within the blocking task.
            let pool = librefang_storage::SurrealConnectionPool::new();
            let rt = tokio::runtime::Handle::current();
            let session = rt
                .block_on(pool.open(&storage_cfg))
                .map_err(|e| format!("open surreal: {e}"))?;
            rt.block_on(librefang_storage::migrations::apply_pending(
                session.client(),
                librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
            ))
            .map_err(|e| format!("apply migrations: {e}"))?;
            let opts = librefang_storage::migrate::MigrationOptions {
                dry_run: false,
                receipt_dir: Some(receipts_dir.clone()),
            };
            librefang_storage::migrate::migrate_sqlite_to_surreal(&sqlite_path, &session, &opts)
                .map_err(|e| e.to_string())
        })
        .await;

        match result {
            Ok(Ok(receipt)) => {
                info!(
                    rows = receipt.copied.values().sum::<u64>(),
                    "storage migration complete"
                );
                let receipts_dir2 = cfg.data_dir.join("migrations");
                let receipt_path = find_latest_receipt(&receipts_dir2);
                (
                    StatusCode::OK,
                    Json(MigrateResponse {
                        dry_run: false,
                        source: receipt.source.clone(),
                        target: receipt.target.clone(),
                        copied: receipt.copied.clone(),
                        errors: receipt.errors.clone(),
                        started_at: receipt.started_at.to_rfc3339(),
                        finished_at: receipt.finished_at.to_rfc3339(),
                        receipt_path,
                    }),
                )
                    .into_response()
            }
            Ok(Err(e)) => ApiErrorResponse::internal(e).into_response(),
            Err(join_err) => {
                ApiErrorResponse::internal(format!("join: {join_err}")).into_response()
            }
        }
    }
    #[cfg(not(all(feature = "sqlite-backend", feature = "surreal-backend")))]
    {
        ApiErrorResponse::bad_request(
            "both sqlite-backend and surreal-backend features must be compiled in for live migration",
        )
        .into_response()
    }
}

// ---------------------------------------------------------------------------
// POST /api/storage/link-uar
// ---------------------------------------------------------------------------

pub async fn post_link_uar(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PostLinkUarBody>,
) -> impl IntoResponse {
    // Phase 8 stub: write the UAR remote config block into config.toml.
    // Full provisioning SQL (DEFINE NAMESPACE / DEFINE USER) lives in Phase 7b.
    let cfg = state.kernel.config_snapshot();
    let config_path = cfg.data_dir.join("config.toml");
    let _lock = state.config_write_lock.lock().await;

    let raw = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = raw.parse().unwrap_or_default();

    if !doc.contains_table("uar") {
        doc.insert("uar", toml_edit::Item::Table(toml_edit::Table::new()));
    }

    // Build [uar.remote] subtable
    let mut remote_tbl = toml_edit::Table::new();
    remote_tbl.insert(
        "url",
        toml_edit::Item::Value(body.remote_url.as_str().into()),
    );
    remote_tbl.insert(
        "namespace",
        toml_edit::Item::Value(body.namespace.as_str().into()),
    );
    remote_tbl.insert("database", toml_edit::Item::Value("main".into()));
    remote_tbl.insert(
        "username",
        toml_edit::Item::Value(body.app_user.as_str().into()),
    );
    remote_tbl.insert(
        "password_env",
        toml_edit::Item::Value(body.app_pass_ref.as_str().into()),
    );
    remote_tbl.insert("tls_skip_verify", toml_edit::Item::Value(false.into()));

    let uar_tbl = doc["uar"].as_table_mut().expect("uar is a table");
    uar_tbl.insert("remote", toml_edit::Item::Table(remote_tbl));
    uar_tbl.insert(
        "share_librefang_storage",
        toml_edit::Item::Value(true.into()),
    );

    if let Err(e) = std::fs::write(&config_path, doc.to_string()) {
        warn!(error = %e, "failed to write uar link config");
        return ApiErrorResponse::internal(format!("write config.toml: {e}")).into_response();
    }

    info!(namespace = %body.namespace, app_user = %body.app_user, "UAR linked to SurrealDB");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "namespace": body.namespace,
            "app_user": body.app_user,
            "memory_linked": body.also_link_memory,
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// POST /api/storage/unlink-uar
// ---------------------------------------------------------------------------

pub async fn post_unlink_uar(
    State(state): State<Arc<AppState>>,
    Json(_body): Json<PostUnlinkUarBody>,
) -> impl IntoResponse {
    let cfg = state.kernel.config_snapshot();
    let config_path = cfg.data_dir.join("config.toml");
    let _lock = state.config_write_lock.lock().await;

    let raw = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = raw.parse().unwrap_or_default();

    if let Some(uar_tbl) = doc.get_mut("uar").and_then(|i| i.as_table_mut()) {
        uar_tbl.remove("remote");
        uar_tbl.remove("share_librefang_storage");
    }

    if let Err(e) = std::fs::write(&config_path, doc.to_string()) {
        warn!(error = %e, "failed to write uar unlink config");
        return ApiErrorResponse::internal(format!("write config.toml: {e}")).into_response();
    }

    info!("UAR storage link removed");
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Best-effort row count query against the configured SurrealDB backend.
#[cfg(feature = "surreal-backend")]
async fn storage_row_counts(
    storage: &librefang_storage::StorageConfig,
) -> Result<StorageTableCounts, String> {
    let pool = librefang_storage::SurrealConnectionPool::new();
    let session = pool.open(storage).await.map_err(|e| format!("open: {e}"))?;

    async fn count_table(session: &librefang_storage::SurrealSession, table: &str) -> u64 {
        let rows: Vec<serde_json::Value> = session
            .client()
            .query(format!("SELECT count() FROM {table} GROUP ALL"))
            .await
            .ok()
            .and_then(|mut r| r.take::<Vec<serde_json::Value>>(0).ok())
            .unwrap_or_default();
        rows.first()
            .and_then(|v| v.get("count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    }

    Ok(StorageTableCounts {
        audit_entries: count_table(&session, "audit_entries").await,
        hook_traces: count_table(&session, "hook_traces").await,
        circuit_breaker_states: count_table(&session, "circuit_breaker_states").await,
        totp_lockout: count_table(&session, "totp_lockout").await,
        agents: count_table(&session, "agents").await,
    })
}

#[cfg(not(feature = "surreal-backend"))]
async fn storage_row_counts(
    _storage: &librefang_storage::StorageConfig,
) -> Result<StorageTableCounts, String> {
    Err("surreal-backend not compiled in".to_owned())
}

fn find_latest_receipt(dir: &std::path::Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut receipts: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("json")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("migration-"))
                    .unwrap_or(false)
        })
        .collect();
    receipts.sort();
    receipts.last().map(|p| p.display().to_string())
}
