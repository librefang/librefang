//! Backup / restore endpoints — extracted from `system.rs` (#3749).
//!
//! Handles creating zip archives of the kernel home directory
//! (`POST /api/backup`), listing existing archives (`GET /api/backups`),
//! deleting individual archives (`DELETE /api/backups/{filename}`), and
//! restoring kernel state from an archive (`POST /api/restore`).
//!
//! Public route paths and handler names are preserved so the utoipa path
//! bindings in `openapi.rs` (`routes::create_backup`, etc.) continue to
//! resolve through the glob re-export in `routes/mod.rs`.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build routes for the backup / restore sub-domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/backup", axum::routing::post(create_backup))
        .route("/backups", axum::routing::get(list_backups))
        .route("/backups/{filename}", axum::routing::delete(delete_backup))
        .route("/restore", axum::routing::post(restore_backup))
}

/// Metadata stored inside every backup archive as `manifest.json`.
#[derive(serde::Serialize, serde::Deserialize)]
struct BackupManifest {
    version: u32,
    created_at: String,
    hostname: String,
    librefang_version: String,
    components: Vec<String>,
}

/// POST /api/backup — Create a backup archive of kernel state.
///
/// Returns the backup metadata including the filename. The archive is stored
/// in `<home_dir>/backups/` with a timestamped filename.
#[utoipa::path(post, path = "/api/backup", tag = "system", responses((status = 200, description = "Backup created", body = crate::types::JsonObject)))]
pub async fn create_backup(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let home_dir = &state.kernel.home_dir();
    let backups_dir = home_dir.join("backups");
    if let Err(e) = std::fs::create_dir_all(&backups_dir) {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-backup-create-dir-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("librefang_backup_{timestamp}.zip");
    let backup_path = backups_dir.join(&filename);

    let mut components: Vec<String> = Vec::new();

    // Create zip archive
    let file = match std::fs::File::create(&backup_path) {
        Ok(f) => f,
        Err(e) => {
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-create-file-failed",
                &[("error", &e.to_string())],
            ))
            .into_json_tuple();
        }
    };
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Helper: add a single file to the zip relative to home_dir
    let add_file = |zip: &mut zip::ZipWriter<std::fs::File>,
                    src: &std::path::Path,
                    archive_name: &str|
     -> Result<(), String> {
        let data = std::fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?;
        zip.start_file(archive_name, options)
            .map_err(|e| format!("zip start {archive_name}: {e}"))?;
        std::io::Write::write_all(zip, &data)
            .map_err(|e| format!("zip write {archive_name}: {e}"))?;
        Ok(())
    };

    // Helper: recursively add a directory to the zip
    let add_dir = |zip: &mut zip::ZipWriter<std::fs::File>,
                   dir: &std::path::Path,
                   prefix: &str|
     -> Result<u64, String> {
        let mut count = 0u64;
        if !dir.exists() {
            return Ok(0);
        }
        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| format!("strip prefix: {e}"))?;
            let archive_name = if prefix.is_empty() {
                rel.to_string_lossy().to_string()
            } else {
                format!("{prefix}/{}", rel.to_string_lossy())
            };
            if path.is_file() {
                let data =
                    std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
                zip.start_file(&archive_name, options)
                    .map_err(|e| format!("zip start {archive_name}: {e}"))?;
                std::io::Write::write_all(zip, &data)
                    .map_err(|e| format!("zip write {archive_name}: {e}"))?;
                count += 1;
            }
        }
        Ok(count)
    };

    // 1. config.toml
    let config_path = home_dir.join("config.toml");
    if config_path.exists() {
        if let Err(e) = add_file(&mut zip, &config_path, "config.toml") {
            tracing::warn!("Backup: skipping config.toml: {e}");
        } else {
            components.push("config".to_string());
        }
    }

    // 2. data/cron_jobs.json
    let cron_path = home_dir.join("data").join("cron_jobs.json");
    if cron_path.exists() {
        if let Err(e) = add_file(&mut zip, &cron_path, "data/cron_jobs.json") {
            tracing::warn!("Backup: skipping cron_jobs.json: {e}");
        } else {
            components.push("cron_jobs".to_string());
        }
    }

    // 3. data/hand_state.json
    let hand_state_path = home_dir.join("data").join("hand_state.json");
    if hand_state_path.exists() {
        if let Err(e) = add_file(&mut zip, &hand_state_path, "data/hand_state.json") {
            tracing::warn!("Backup: skipping hand_state.json: {e}");
        } else {
            components.push("hand_state".to_string());
        }
    }

    // 4. data/custom_models.json
    let custom_models_path = home_dir.join("data").join("custom_models.json");
    if custom_models_path.exists() {
        if let Err(e) = add_file(&mut zip, &custom_models_path, "data/custom_models.json") {
            tracing::warn!("Backup: skipping custom_models.json: {e}");
        } else {
            components.push("custom_models".to_string());
        }
    }

    // 5. agents/ directory (user templates)
    let agents_dir = home_dir.join("workspaces").join("agents");
    if agents_dir.exists() {
        match add_dir(&mut zip, &agents_dir, "agents") {
            Ok(n) if n > 0 => components.push("agents".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping agents/: {e}"),
        }
    }

    // 6. skills/ directory
    let skills_dir = home_dir.join("skills");
    if skills_dir.exists() {
        match add_dir(&mut zip, &skills_dir, "skills") {
            Ok(n) if n > 0 => components.push("skills".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping skills/: {e}"),
        }
    }

    // 7. workflows/ directory
    let workflows_dir = home_dir.join("workflows");
    if workflows_dir.exists() {
        match add_dir(&mut zip, &workflows_dir, "workflows") {
            Ok(n) if n > 0 => components.push("workflows".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping workflows/: {e}"),
        }
    }

    // 8. data/ directory (SQLite DB, memory, etc.)
    let data_dir = home_dir.join("data");
    if data_dir.exists() {
        match add_dir(&mut zip, &data_dir, "data") {
            Ok(n) if n > 0 => components.push("data".to_string()),
            Ok(_) => {}
            Err(e) => tracing::warn!("Backup: skipping data/: {e}"),
        }
    }

    // Write manifest
    let manifest = BackupManifest {
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        hostname: super::system::hostname_string(),
        librefang_version: env!("CARGO_PKG_VERSION").to_string(),
        components: components.clone(),
    };
    if let Ok(manifest_json) = serde_json::to_string_pretty(&manifest) {
        if let Err(e) = zip.start_file("manifest.json", options).and_then(|()| {
            std::io::Write::write_all(&mut zip, manifest_json.as_bytes())
                .map_err(zip::result::ZipError::Io)
        }) {
            tracing::warn!("Failed to write manifest.json into export archive: {e}");
        }
    }

    if let Err(e) = zip.finish() {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-backup-finalize-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }

    let size = std::fs::metadata(&backup_path)
        .map(|m| m.len())
        .unwrap_or(0);

    tracing::info!(
        "Backup created: {filename} ({} bytes, {} components)",
        size,
        components.len()
    );
    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        &format!("Backup created: {filename}"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "filename": filename,
            "path": backup_path.to_string_lossy(),
            "size_bytes": size,
            "components": components,
            "created_at": manifest.created_at,
        })),
    )
}

/// GET /api/backups — List existing backups.
#[utoipa::path(get, path = "/api/backups", tag = "system", responses((status = 200, description = "List backups", body = Vec<serde_json::Value>)))]
pub async fn list_backups(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let backups_dir = state.kernel.home_dir().join("backups");
    if !backups_dir.exists() {
        return Json(serde_json::json!({"backups": [], "total": 0}));
    }

    let mut backups: Vec<serde_json::Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&backups_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("zip") {
                continue;
            }
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                });

            // Try to read manifest from the zip
            let manifest = read_backup_manifest(&path);

            backups.push(serde_json::json!({
                "filename": filename,
                "path": path.to_string_lossy(),
                "size_bytes": size,
                "modified_at": modified,
                "components": manifest.as_ref().map(|m| &m.components),
                "librefang_version": manifest.as_ref().map(|m| &m.librefang_version),
                "created_at": manifest.as_ref().map(|m| &m.created_at),
            }));
        }
    }

    // Sort by filename descending (newest first since filenames contain timestamps)
    backups.sort_by(|a, b| {
        let fa = a["filename"].as_str().unwrap_or("");
        let fb = b["filename"].as_str().unwrap_or("");
        fb.cmp(fa)
    });

    let total = backups.len();
    Json(serde_json::json!({"backups": backups, "total": total}))
}

fn is_invalid_backup_filename(filename: &str) -> bool {
    if filename.is_empty() {
        return true;
    }
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return true;
    }
    std::path::Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        != Some(filename)
}

fn find_backup_path(
    backups_dir: &std::path::Path,
    filename: &str,
) -> std::io::Result<Option<std::path::PathBuf>> {
    let entries = std::fs::read_dir(backups_dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("zip") {
            continue;
        }
        if entry.file_name().to_str() == Some(filename) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// DELETE /api/backups/{filename} — Delete a specific backup.
#[utoipa::path(delete, path = "/api/backups/{filename}", tag = "system", params(("filename" = String, Path, description = "Backup filename")), responses((status = 200, description = "Backup deleted")))]
pub async fn delete_backup(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    // Sanitize filename to prevent path traversal
    if is_invalid_backup_filename(&filename) {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-invalid-filename"))
            .into_json_tuple();
    }
    if !filename.ends_with(".zip") {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-must-be-zip"))
            .into_json_tuple();
    }

    let backups_dir = state.kernel.home_dir().join("backups");
    let backup_path = match find_backup_path(&backups_dir, &filename) {
        Ok(Some(path)) => path,
        Ok(None) => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) => {
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-delete-failed",
                &[("error", &e.to_string())],
            ))
            .into_json_tuple();
        }
    };

    if let Err(e) = std::fs::remove_file(&backup_path) {
        return ApiErrorResponse::internal(t.t_args(
            "api-error-backup-delete-failed",
            &[("error", &e.to_string())],
        ))
        .into_json_tuple();
    }

    tracing::info!("Backup deleted: {filename}");
    (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
}

/// POST /api/restore — Restore kernel state from a backup archive.
///
/// Accepts a JSON body with `{"filename": "librefang_backup_20260315_120000.zip"}`.
/// The file must exist in `<home_dir>/backups/`.
///
/// **Warning**: This overwrites existing state files. The daemon should be
/// restarted after a restore for all changes to take effect.
#[utoipa::path(post, path = "/api/restore", tag = "system", request_body = crate::types::JsonObject, responses((status = 200, description = "Backup restored", body = crate::types::JsonObject)))]
pub async fn restore_backup(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    let filename = match req.get("filename").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => {
            return ApiErrorResponse::bad_request(t.t("api-error-backup-missing-filename"))
                .into_json_tuple();
        }
    };

    // Sanitize
    if is_invalid_backup_filename(&filename) {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-invalid-filename"))
            .into_json_tuple();
    }
    if !filename.ends_with(".zip") {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-must-be-zip"))
            .into_json_tuple();
    }

    let home_dir = &state.kernel.home_dir();
    let backups_dir = home_dir.join("backups");
    let backup_path = match find_backup_path(&backups_dir, &filename) {
        Ok(Some(path)) => path,
        Ok(None) => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Err(e) => {
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-open-failed", &[("error", &e.to_string())]),
            )
            .into_json_tuple();
        }
    };

    // Open zip
    let file = match std::fs::File::open(&backup_path) {
        Ok(f) => f,
        Err(e) => {
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-open-failed", &[("error", &e.to_string())]),
            )
            .into_json_tuple();
        }
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            return ApiErrorResponse::bad_request(t.t_args(
                "api-error-backup-invalid-archive",
                &[("error", &e.to_string())],
            ))
            .into_json_tuple();
        }
    };

    // Validate manifest
    let manifest: Option<BackupManifest> = {
        match archive.by_name("manifest.json") {
            Ok(mut entry) => {
                let mut buf = String::new();
                if std::io::Read::read_to_string(&mut entry, &mut buf).is_ok() {
                    serde_json::from_str(&buf).ok()
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    };

    if manifest.is_none() {
        return ApiErrorResponse::bad_request(t.t("api-error-backup-missing-manifest"))
            .into_json_tuple();
    }

    let mut restored: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // Extract all files to home_dir, skipping manifest.json itself
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                errors.push(format!("Failed to read entry {i}: {e}"));
                continue;
            }
        };

        let entry_name = match entry.enclosed_name() {
            Some(name) => name.to_path_buf(),
            None => {
                errors.push(format!("Skipped unsafe entry name at index {i}"));
                continue;
            }
        };

        if entry_name.to_string_lossy() == "manifest.json" {
            continue;
        }

        let target = home_dir.join(&entry_name);

        if entry.is_dir() {
            if let Err(e) = std::fs::create_dir_all(&target) {
                errors.push(format!("mkdir {}: {e}", entry_name.display()));
            }
            continue;
        }

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                errors.push(format!("mkdir parent for {}: {e}", entry_name.display()));
                continue;
            }
        }

        let mut data = Vec::new();
        if let Err(e) = std::io::Read::read_to_end(&mut entry, &mut data) {
            errors.push(format!("read {}: {e}", entry_name.display()));
            continue;
        }
        if let Err(e) = std::fs::write(&target, &data) {
            errors.push(format!("write {}: {e}", entry_name.display()));
            continue;
        }
        restored.push(entry_name.to_string_lossy().to_string());
    }

    let total_restored = restored.len();
    tracing::info!(
        "Restore from {filename}: {total_restored} files restored, {} errors",
        errors.len()
    );
    state.kernel.audit().record(
        "system",
        librefang_runtime::audit::AuditAction::ConfigChange,
        &format!("Backup restored: {filename} ({total_restored} files)"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "restored_files": total_restored,
            "errors": errors,
            "manifest": manifest,
            "message": "Restore complete. Restart the daemon for all changes to take effect.",
        })),
    )
}

/// Read the `manifest.json` from a backup zip without extracting everything.
fn read_backup_manifest(path: &std::path::Path) -> Option<BackupManifest> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name("manifest.json").ok()?;
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut entry, &mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}
