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
use crate::middleware::{AuthenticatedApiUser, RequestLanguage};
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

/// Outcome of a successful `create_backup_blocking` run.
///
/// Carries everything the async handler needs to build the JSON
/// response and record an audit entry, so the spawn_blocking closure
/// stays purely sync and owns no axum / kernel handles.
struct BackupOutcome {
    filename: String,
    backup_path: std::path::PathBuf,
    size_bytes: u64,
    components: Vec<String>,
    created_at: String,
}

/// Categorised failure mode for `create_backup_blocking`.
///
/// Maps 1:1 onto the original handler's distinct ApiErrorResponse
/// branches so the translated client-facing message stays identical
/// after the spawn_blocking refactor.
enum BackupBuildError {
    CreateDir(String),
    CreateFile(String),
    Finalize(String),
}

/// Sync, blocking implementation of `create_backup`. Walks the home
/// directory tree (`walkdir` + `std::fs::read`) and produces a zip
/// archive. Must be dispatched via `tokio::task::spawn_blocking` —
/// running it directly on the axum/tokio worker stalls every other
/// request scheduled on that worker for the duration of the walk
/// (refs `docs/issues/blocking-fs-on-executor.md`).
fn create_backup_blocking(home_dir: std::path::PathBuf) -> Result<BackupOutcome, BackupBuildError> {
    let backups_dir = home_dir.join("backups");
    std::fs::create_dir_all(&backups_dir)
        .map_err(|e| BackupBuildError::CreateDir(e.to_string()))?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("librefang_backup_{timestamp}.zip");
    let backup_path = backups_dir.join(&filename);

    let mut components: Vec<String> = Vec::new();

    // Create zip archive
    let file = std::fs::File::create(&backup_path)
        .map_err(|e| BackupBuildError::CreateFile(e.to_string()))?;
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

    zip.finish()
        .map_err(|e| BackupBuildError::Finalize(e.to_string()))?;

    let size_bytes = std::fs::metadata(&backup_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(BackupOutcome {
        filename,
        backup_path,
        size_bytes,
        components,
        created_at: manifest.created_at,
    })
}

/// POST /api/backup — Create a backup archive of kernel state.
///
/// Returns the backup metadata including the filename. The archive is stored
/// in `<home_dir>/backups/` with a timestamped filename.
///
/// The actual zip-build work (`walkdir` + `std::fs::read`/`write` over the
/// whole `~/.librefang/` tree) is dispatched onto
/// `tokio::task::spawn_blocking` — running it directly on the axum/tokio
/// worker would stall every other request scheduled on that worker for
/// the duration of the walk (seconds, on a multi-GB home).
#[utoipa::path(post, path = "/api/backup", tag = "system", responses((status = 200, description = "Backup created", body = crate::types::JsonObject)))]
pub async fn create_backup(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
) -> impl IntoResponse {
    let home_dir = state.kernel.home_dir().to_path_buf();

    // Dispatch the heavy `walkdir` + `std::fs` work onto a blocking
    // thread. We must not hold any `!Send` value (notably
    // `ErrorTranslator`, which wraps the fluent bundle) across this
    // `.await` — the axum `Handler` bound rejects non-Send futures
    // with a cryptic trait-bound error. The translator is constructed
    // separately on each error branch below so it never crosses the
    // suspend point.
    let result = tokio::task::spawn_blocking(move || create_backup_blocking(home_dir)).await;

    let outcome = match result {
        Ok(Ok(o)) => o,
        Ok(Err(BackupBuildError::CreateDir(msg))) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-create-dir-failed", &[("error", &msg)]),
            )
            .into_json_tuple();
        }
        Ok(Err(BackupBuildError::CreateFile(msg))) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-create-file-failed", &[("error", &msg)]),
            )
            .into_json_tuple();
        }
        Ok(Err(BackupBuildError::Finalize(msg))) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-finalize-failed", &[("error", &msg)]),
            )
            .into_json_tuple();
        }
        Err(join_err) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-finalize-failed",
                &[("error", &format!("backup task join: {join_err}"))],
            ))
            .into_json_tuple();
        }
    };

    tracing::info!(
        "Backup created: {} ({} bytes, {} components)",
        outcome.filename,
        outcome.size_bytes,
        outcome.components.len()
    );
    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("Backup created: {}", outcome.filename),
        "completed",
        user_id,
        Some("api".to_string()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "filename": outcome.filename,
            "path": outcome.backup_path.to_string_lossy(),
            "size_bytes": outcome.size_bytes,
            "components": outcome.components,
            "created_at": outcome.created_at,
        })),
    )
}

/// GET /api/backups — List existing backups.
#[utoipa::path(get, path = "/api/backups", tag = "system", responses((status = 200, description = "List backups", body = Vec<serde_json::Value>)))]
pub async fn list_backups(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let backups_dir = state.kernel.home_dir().join("backups");
    match tokio::task::spawn_blocking(move || list_backups_blocking(&backups_dir)).await {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(error)) => {
            ApiErrorResponse::internal_scrub(format!("backup listing failed: {error}"))
                .into_response()
        }
        Err(error) => {
            ApiErrorResponse::internal_scrub(format!("backup listing task failed: {error}"))
                .into_response()
        }
    }
}

fn list_backups_blocking(backups_dir: &std::path::Path) -> std::io::Result<serde_json::Value> {
    let mut backups: Vec<serde_json::Value> = Vec::new();
    let entries = match std::fs::read_dir(backups_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::json!({"backups": [], "total": 0}));
        }
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("zip") {
            continue;
        }
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let metadata = entry.metadata()?;
        let size = metadata.len();
        let modified = metadata.modified().ok().map(|t| {
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

    // Sort by filename descending (newest first since filenames contain timestamps)
    backups.sort_by(|a, b| {
        let fa = a["filename"].as_str().unwrap_or("");
        let fb = b["filename"].as_str().unwrap_or("");
        fb.cmp(fa)
    });

    let total = backups.len();
    Ok(serde_json::json!({"backups": backups, "total": total}))
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

fn delete_backup_blocking(backups_dir: &std::path::Path, filename: &str) -> std::io::Result<bool> {
    let Some(backup_path) = find_backup_path(backups_dir, filename)? else {
        return Ok(false);
    };
    std::fs::remove_file(backup_path)?;
    Ok(true)
}

/// DELETE /api/backups/{filename} — Delete a specific backup.
#[utoipa::path(delete, path = "/api/backups/{filename}", tag = "system", params(("filename" = String, Path, description = "Backup filename")), responses((status = 204, description = "Backup deleted")))]
pub async fn delete_backup(
    State(state): State<Arc<AppState>>,
    Path(filename): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    // Sanitize filename to prevent path traversal
    if is_invalid_backup_filename(&filename) {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        return ApiErrorResponse::bad_request(t.t("api-error-backup-invalid-filename"))
            .into_json_tuple();
    }
    if !filename.ends_with(".zip") {
        let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
        return ApiErrorResponse::bad_request(t.t("api-error-backup-must-be-zip"))
            .into_json_tuple();
    }

    let backups_dir = state.kernel.home_dir().join("backups");
    let filename_for_task = filename.clone();
    let result = tokio::task::spawn_blocking(move || {
        delete_backup_blocking(&backups_dir, &filename_for_task)
    })
    .await;
    match result {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Ok(Err(e)) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-delete-failed",
                &[("error", &e.to_string())],
            ))
            .into_json_tuple();
        }
        Err(e) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-delete-failed",
                &[("error", &format!("backup delete task failed: {e}"))],
            ))
            .into_json_tuple();
        }
    }

    tracing::info!("Backup deleted: {filename}");
    let user_id = api_user.as_ref().map(|user| user.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!("Backup deleted: {filename}"),
        "completed",
        user_id,
        Some("api".to_string()),
    );
    (StatusCode::NO_CONTENT, Json(serde_json::json!(null)))
}

/// Categorised failure mode for `restore_backup_blocking`, mapped 1:1 onto
/// the original handler's distinct ApiErrorResponse branches so the
/// translated client-facing message stays identical after the
/// spawn_blocking refactor.
#[derive(Debug)]
enum RestoreError {
    NotFound,
    Open(String),
    InvalidArchive(String),
    MissingManifest,
    ResourceLimit(String),
}

const MAX_RESTORE_ENTRIES: usize = 10_000;
const MAX_RESTORE_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RESTORE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RESTORE_COMPRESSION_RATIO: u64 = 100;
const MAX_RESTORE_MANIFEST_BYTES: u64 = 1024 * 1024;

fn prepare_restore_target(
    canonical_home: &std::path::Path,
    entry_name: &std::path::Path,
) -> Result<std::path::PathBuf, RestoreError> {
    let mut parent = canonical_home.to_path_buf();
    if let Some(relative_parent) = entry_name.parent() {
        for component in relative_parent.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(RestoreError::InvalidArchive(
                    "unsafe restore path component".to_string(),
                ));
            };
            parent.push(component);
            match std::fs::symlink_metadata(&parent) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(RestoreError::InvalidArchive(
                        "restore path contains a symbolic link".to_string(),
                    ));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    return Err(RestoreError::InvalidArchive(
                        "restore path contains a non-directory component".to_string(),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&parent).map_err(|error| {
                        RestoreError::InvalidArchive(format!(
                            "failed to create restore directory: {error}"
                        ))
                    })?;
                }
                Err(error) => {
                    return Err(RestoreError::InvalidArchive(format!(
                        "failed to inspect restore directory: {error}"
                    )));
                }
            }
            let canonical_parent = std::fs::canonicalize(&parent).map_err(|error| {
                RestoreError::InvalidArchive(format!(
                    "failed to resolve restore directory: {error}"
                ))
            })?;
            if !canonical_parent.starts_with(canonical_home) {
                return Err(RestoreError::InvalidArchive(
                    "restore path escapes the home directory".to_string(),
                ));
            }
        }
    }

    let target = canonical_home.join(entry_name);
    if let Ok(metadata) = std::fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(RestoreError::InvalidArchive(
                "restore target is a symbolic link".to_string(),
            ));
        }
        if metadata.is_dir() {
            return Err(RestoreError::InvalidArchive(
                "restore target is a directory".to_string(),
            ));
        }
    }
    Ok(target)
}

fn open_restore_target(target: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(target)
}

/// Result of a successful restore extraction.
struct RestoreOutcome {
    restored: Vec<String>,
    errors: Vec<String>,
    manifest: Option<BackupManifest>,
}

fn restore_completion(errors: &[String]) -> (StatusCode, &'static str) {
    if errors.is_empty() {
        (StatusCode::OK, "completed")
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "failed")
    }
}

/// Sync, blocking implementation of `restore_backup`: opens the zip,
/// validates the manifest, and extracts every entry into `home_dir`.
/// Must be dispatched via `tokio::task::spawn_blocking` — the
/// decompress-and-write loop otherwise stalls the axum/tokio worker for
/// the full archive, matching the `create_backup_blocking` contract above.
fn restore_backup_blocking(
    backup_path: std::path::PathBuf,
    home_dir: std::path::PathBuf,
) -> Result<RestoreOutcome, RestoreError> {
    let canonical_home = std::fs::canonicalize(&home_dir)
        .map_err(|e| RestoreError::Open(format!("resolve restore directory: {e}")))?;
    let file = std::fs::File::open(&backup_path).map_err(|e| RestoreError::Open(e.to_string()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| RestoreError::InvalidArchive(e.to_string()))?;
    if archive.len() > MAX_RESTORE_ENTRIES {
        return Err(RestoreError::ResourceLimit(format!(
            "archive contains {} entries; maximum is {MAX_RESTORE_ENTRIES}",
            archive.len()
        )));
    }
    let mut declared_total = 0_u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| RestoreError::InvalidArchive(e.to_string()))?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| RestoreError::InvalidArchive("unsafe entry name".to_string()))?;
        let declared_size = entry.size();
        let compressed_size = entry.compressed_size();
        if name.to_string_lossy() == "manifest.json" {
            if declared_size > MAX_RESTORE_MANIFEST_BYTES {
                return Err(RestoreError::ResourceLimit(
                    "manifest exceeds the restore limit".to_string(),
                ));
            }
            continue;
        }
        if declared_size > MAX_RESTORE_ENTRY_BYTES {
            return Err(RestoreError::ResourceLimit(format!(
                "entry {} exceeds the per-entry decompression limit",
                name.display()
            )));
        }
        if declared_size > 0
            && (compressed_size == 0
                || declared_size > compressed_size.saturating_mul(MAX_RESTORE_COMPRESSION_RATIO))
        {
            return Err(RestoreError::ResourceLimit(format!(
                "entry {} exceeds the compression-ratio limit",
                name.display()
            )));
        }
        declared_total = declared_total.saturating_add(declared_size);
        if declared_total > MAX_RESTORE_TOTAL_BYTES {
            return Err(RestoreError::ResourceLimit(
                "archive exceeds the total decompression limit".to_string(),
            ));
        }
    }

    // Validate manifest before touching the filesystem.
    let manifest: Option<BackupManifest> = match archive.by_name("manifest.json") {
        Ok(mut entry) => {
            let mut buf = String::new();
            let mut limited = std::io::Read::take(&mut entry, MAX_RESTORE_MANIFEST_BYTES + 1);
            if std::io::Read::read_to_string(&mut limited, &mut buf).is_ok()
                && buf.len() as u64 <= MAX_RESTORE_MANIFEST_BYTES
            {
                serde_json::from_str(&buf).ok()
            } else {
                None
            }
        }
        Err(_) => None,
    };
    if manifest.is_none() {
        return Err(RestoreError::MissingManifest);
    }

    let mut restored: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut total_uncompressed = 0_u64;

    // Extract all files to home_dir, skipping manifest.json itself.
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

        if entry.is_dir() {
            prepare_restore_target(&canonical_home, &entry_name.join("placeholder"))?;
            continue;
        }

        let target = prepare_restore_target(&canonical_home, &entry_name)?;

        let mut output = match open_restore_target(&target) {
            Ok(file) => file,
            Err(e) => {
                errors.push(format!("create {}: {e}", entry_name.display()));
                continue;
            }
        };
        let mut limited = std::io::Read::take(&mut entry, MAX_RESTORE_ENTRY_BYTES + 1);
        let written = match std::io::copy(&mut limited, &mut output) {
            Ok(bytes) => bytes,
            Err(e) => {
                let _ = std::fs::remove_file(&target);
                errors.push(format!("extract {}: {e}", entry_name.display()));
                continue;
            }
        };
        if written > MAX_RESTORE_ENTRY_BYTES {
            let _ = std::fs::remove_file(&target);
            return Err(RestoreError::ResourceLimit(format!(
                "entry {} exceeded the per-entry decompression limit",
                entry_name.display()
            )));
        }
        total_uncompressed = total_uncompressed.saturating_add(written);
        if total_uncompressed > MAX_RESTORE_TOTAL_BYTES {
            let _ = std::fs::remove_file(&target);
            return Err(RestoreError::ResourceLimit(
                "archive exceeded the total decompression limit".to_string(),
            ));
        }
        restored.push(entry_name.to_string_lossy().to_string());
    }

    Ok(RestoreOutcome {
        restored,
        errors,
        manifest,
    })
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
    api_user: Option<axum::Extension<crate::middleware::AuthenticatedApiUser>>,
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

    let home_dir = state.kernel.home_dir().to_path_buf();
    let backups_dir = home_dir.join("backups");
    let filename_for_task = filename.clone();

    // Drop the `!Send` ErrorTranslator before the spawn_blocking `.await`
    // (the axum Handler bound rejects a non-Send future). Each error branch
    // below reconstructs it after the await, like `create_backup`.
    drop(t);

    // Dispatch the blocking open + decompress + write loop onto a blocking
    // thread so it does not stall the axum/tokio worker (refs
    // blocking-fs-on-executor).
    let result = tokio::task::spawn_blocking(move || {
        let backup_path = match find_backup_path(&backups_dir, &filename_for_task) {
            Ok(Some(path)) => path,
            Ok(None) => return Err(RestoreError::NotFound),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(RestoreError::NotFound);
            }
            Err(error) => return Err(RestoreError::Open(error.to_string())),
        };
        restore_backup_blocking(backup_path, home_dir)
    })
    .await;

    let outcome = match result {
        Ok(Ok(o)) => o,
        Ok(Err(RestoreError::NotFound)) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::not_found(t.t("api-error-backup-not-found"))
                .into_json_tuple();
        }
        Ok(Err(RestoreError::Open(msg))) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(
                t.t_args("api-error-backup-open-failed", &[("error", &msg)]),
            )
            .into_json_tuple();
        }
        Ok(Err(RestoreError::InvalidArchive(msg))) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(
                t.t_args("api-error-backup-invalid-archive", &[("error", &msg)]),
            )
            .into_json_tuple();
        }
        Ok(Err(RestoreError::MissingManifest)) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::bad_request(t.t("api-error-backup-missing-manifest"))
                .into_json_tuple();
        }
        Ok(Err(RestoreError::ResourceLimit(msg))) => {
            tracing::warn!(filename, %msg, "Backup restore rejected by resource limits");
            return ApiErrorResponse::bad_request("Backup archive exceeds restore resource limits")
                .into_json_tuple();
        }
        Err(join_err) => {
            let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
            return ApiErrorResponse::internal(t.t_args(
                "api-error-backup-open-failed",
                &[("error", &format!("restore task join: {join_err}"))],
            ))
            .into_json_tuple();
        }
    };
    let restored = outcome.restored;
    let errors = outcome.errors;
    let manifest = outcome.manifest;

    let total_restored = restored.len();
    let error_count = errors.len();
    let (response_status, audit_status) = restore_completion(&errors);
    if errors.is_empty() {
        tracing::info!("Restore from {filename}: {total_restored} files restored");
    } else {
        tracing::error!(
            filename,
            total_restored,
            error_count,
            errors = ?errors,
            "Backup restore completed with partial filesystem failures"
        );
    }
    let user_id = api_user.as_ref().map(|u| u.0.user_id);
    state.kernel.audit().record_with_context(
        "system",
        librefang_kernel::audit::AuditAction::ConfigChange,
        format!(
            "Backup restore {audit_status}: {filename} ({total_restored} files, {error_count} errors)"
        ),
        audit_status,
        user_id,
        Some("api".to_string()),
    );

    if response_status.is_server_error() {
        return (
            response_status,
            Json(serde_json::json!({
                "error": "Backup restore incomplete",
                "restored_files": total_restored,
                "error_count": error_count,
            })),
        );
    }

    (
        response_status,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn partial_restore_is_a_failed_server_response() {
        assert_eq!(restore_completion(&[]), (StatusCode::OK, "completed"));
        assert_eq!(
            restore_completion(&["disk full".to_string()]),
            (StatusCode::INTERNAL_SERVER_ERROR, "failed")
        );
    }

    #[test]
    fn restore_rejects_high_compression_ratio_before_writing_files() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("bomb.zip");
        let restore_dir = temp.path().join("restore");
        std::fs::create_dir(&restore_dir).unwrap();

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(br#"{"version":"test"}"#).unwrap();
        zip.start_file("data/bomb.bin", options).unwrap();
        zip.write_all(&vec![0_u8; 1024 * 1024]).unwrap();
        zip.finish().unwrap();

        let result = restore_backup_blocking(archive_path, restore_dir.clone());
        assert!(matches!(result, Err(RestoreError::ResourceLimit(_))));
        assert!(!restore_dir.join("data/bomb.bin").exists());
    }

    #[test]
    fn restore_streams_normal_archive_within_limits() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("normal.zip");
        let restore_dir = temp.path().join("restore");
        std::fs::create_dir(&restore_dir).unwrap();

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            br#"{"version":1,"created_at":"now","hostname":"host","librefang_version":"test","components":["config"]}"#,
        )
        .unwrap();
        zip.start_file("config.toml", options).unwrap();
        zip.write_all(b"[kernel]\nname = \"test\"\n").unwrap();
        zip.finish().unwrap();

        let outcome = restore_backup_blocking(archive_path, restore_dir.clone()).unwrap();
        assert_eq!(outcome.restored, vec!["config.toml"]);
        assert!(outcome.errors.is_empty());
        assert_eq!(
            std::fs::read_to_string(restore_dir.join("config.toml")).unwrap(),
            "[kernel]\nname = \"test\"\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn restore_rejects_parent_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("escape.zip");
        let restore_dir = temp.path().join("restore");
        let outside_dir = temp.path().join("outside");
        std::fs::create_dir(&restore_dir).unwrap();
        std::fs::create_dir(&outside_dir).unwrap();
        std::os::unix::fs::symlink(&outside_dir, restore_dir.join("data")).unwrap();

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            br#"{"version":1,"created_at":"now","hostname":"host","librefang_version":"test","components":["data"]}"#,
        )
        .unwrap();
        zip.start_file("data/escaped.txt", options).unwrap();
        zip.write_all(b"escaped").unwrap();
        zip.finish().unwrap();

        let result = restore_backup_blocking(archive_path, restore_dir);
        assert!(matches!(result, Err(RestoreError::InvalidArchive(_))));
        assert!(!outside_dir.join("escaped.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn restore_rejects_leaf_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("leaf-escape.zip");
        let restore_dir = temp.path().join("restore");
        let outside_file = temp.path().join("outside.txt");
        std::fs::create_dir(&restore_dir).unwrap();
        std::fs::write(&outside_file, b"keep me").unwrap();
        std::os::unix::fs::symlink(&outside_file, restore_dir.join("config.toml")).unwrap();

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            br#"{"version":1,"created_at":"now","hostname":"host","librefang_version":"test","components":["config"]}"#,
        )
        .unwrap();
        zip.start_file("config.toml", options).unwrap();
        zip.write_all(b"replaced").unwrap();
        zip.finish().unwrap();

        let result = restore_backup_blocking(archive_path, restore_dir);
        assert!(matches!(result, Err(RestoreError::InvalidArchive(_))));
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"keep me");
    }
}
