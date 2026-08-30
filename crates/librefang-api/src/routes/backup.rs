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

/// Which part of the archive a backup component owns.
///
/// The archive path is not always the home-relative filesystem path: the
/// `agents` component is stored under the `agents/` prefix but its files are
/// read from — and must be written back to — the agent workspaces directory.
#[derive(Clone, Copy)]
enum ArchiveScope {
    /// Exactly one archive entry.
    File(&'static str),
    /// Every archive entry under this prefix, given without a trailing slash.
    Tree(&'static str),
}

/// The backup layout: each component's name as it appears in `manifest.json`,
/// paired with the part of the archive it owns, in the order `create_backup`
/// writes them.
///
/// Single source of truth for three things that used to be three independent
/// if/else ladders — what `create_backup` archives, which component a restored
/// entry belongs to, and which names the `components` request field accepts.
/// Drift between the create side and the restore side is what left the `agents`
/// component unable to round-trip for months (see `restore_root`).
///
/// Scopes deliberately overlap. `data/cron_jobs.json` is owned by the
/// `cron_jobs` row *and* by the `data` row, so both `components: ["cron_jobs"]`
/// and `components: ["data"]` restore it. A first-match-wins classifier instead
/// punches a hole in `data` at exactly the three named JSON files.
const BACKUP_LAYOUT: &[(&str, ArchiveScope)] = &[
    ("config", ArchiveScope::File("config.toml")),
    ("cron_jobs", ArchiveScope::File("data/cron_jobs.json")),
    ("hand_state", ArchiveScope::File("data/hand_state.json")),
    (
        "custom_models",
        ArchiveScope::File("data/custom_models.json"),
    ),
    ("agents", ArchiveScope::Tree(AGENTS_ARCHIVE_PREFIX)),
    ("skills", ArchiveScope::Tree("skills")),
    ("workflows", ArchiveScope::Tree("workflows")),
    ("data", ArchiveScope::Tree("data")),
];

/// Archive prefix holding the agent workspaces tree.
///
/// Called out by name because it is the one component whose archive prefix and
/// filesystem location differ, so both `backup_source` and `restore_root`
/// have to special-case it.
const AGENTS_ARCHIVE_PREFIX: &str = "agents";

/// Every accepted `components` value, comma-joined for an error message.
fn backup_component_names() -> String {
    BACKUP_LAYOUT
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Is `name` a component `create_backup` can actually write?
///
/// A `components` entry that no row owns can never match an archive entry, so
/// accepting it would turn a typo into a silent no-op restore.
fn is_known_backup_component(name: &str) -> bool {
    BACKUP_LAYOUT.iter().any(|(known, _)| *known == name)
}

/// Does an archive entry (a `/`-separated, archive-relative path) fall inside
/// this scope?
fn scope_contains(scope: ArchiveScope, entry: &str) -> bool {
    match scope {
        ArchiveScope::File(path) => entry == path,
        ArchiveScope::Tree(prefix) => entry
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/')),
    }
}

/// Is this archive entry owned by the named component?
fn entry_belongs_to(entry: &str, component: &str) -> bool {
    BACKUP_LAYOUT
        .iter()
        .any(|(name, scope)| *name == component && scope_contains(*scope, entry))
}

/// Is this archive entry owned by any component at all?
///
/// Entries no component owns (`manifest.json`-adjacent metadata, anything a
/// future release adds) are restored regardless of the selection, so a narrow
/// `components` list never silently drops them.
fn entry_is_classified(entry: &str) -> bool {
    BACKUP_LAYOUT
        .iter()
        .any(|(_, scope)| scope_contains(*scope, entry))
}

/// Is this archive entry owned by at least one of the selected components?
fn entry_is_selected(entry: &str, selected: &[String]) -> bool {
    selected.iter().any(|c| entry_belongs_to(entry, c))
}

/// Directory a `Tree` scope's files are read from at backup time.
///
/// The exact mirror of `restore_root`: `agents` comes out of the agent
/// workspaces directory, everything else out of `<home>/<archive prefix>`.
fn backup_source(
    home_dir: &std::path::Path,
    agent_workspaces_dir: &std::path::Path,
    archive: &str,
) -> std::path::PathBuf {
    if archive == AGENTS_ARCHIVE_PREFIX {
        agent_workspaces_dir.to_path_buf()
    } else {
        home_dir.join(archive)
    }
}

/// Filesystem path an archive entry must be written back to.
///
/// Everything is home-relative except the `agents/` prefix. `create_backup`
/// reads that tree out of the agent workspaces directory
/// (`<home>/workspaces/agents/` unless `workspaces_dir` is set), so restore has
/// to put it back there. Sending it to `<home>/agents/` instead — which restore
/// did from #444 until this function existed — dropped the files into the
/// pre-unification legacy layout, where they were only ever picked up by the
/// kernel's `migrate_legacy_agent_dirs` boot migration, and only when the
/// canonical destination did not already exist. Restoring onto a running system
/// therefore left the archived agent workspaces stranded and unread.
///
/// The archive prefix has been `agents/` in every release, so redirecting it is
/// correct for archives written under either layout: an old archive's `agents/`
/// entries came from the legacy `<home>/agents/`, whose contents the kernel now
/// relocates to exactly this destination anyway.
fn restore_root<'a>(
    home_dir: &'a std::path::Path,
    agent_workspaces_dir: &'a std::path::Path,
    entry: &std::path::Path,
) -> (&'a std::path::Path, std::path::PathBuf) {
    match entry.strip_prefix(AGENTS_ARCHIVE_PREFIX) {
        Ok(rest) => (agent_workspaces_dir, rest.to_path_buf()),
        Err(_) => (home_dir, entry.to_path_buf()),
    }
}

/// SQLite's shared-memory index sidecar.
///
/// `-shm` is the WAL index for the connections currently mapping the database, not state: SQLite
/// recreates it on demand, a snapshot of one means nothing to any other process, and writing one
/// over a live database is wrong on every platform.
/// On Windows it is also impossible — SQLite memory-maps the file, and truncating a file with an
/// active mapped section fails with `ERROR_USER_MAPPED_FILE` (os error 1224), which is what left
/// two `data/` entries failing every restore on that platform.
fn is_sqlite_shared_memory_index(name: &str) -> bool {
    name.ends_with("-shm")
}

/// Normalise an archive entry path to the `/`-separated string the layout
/// table is written in.
///
/// `enclosed_name` hands back a `Path`, whose rendering is platform-dependent;
/// zip entry names are always `/`-separated, so rebuilding from components
/// keeps the classification identical on Windows.
fn archive_entry_key(entry: &std::path::Path) -> String {
    entry
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
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
fn create_backup_blocking(
    home_dir: std::path::PathBuf,
    agent_workspaces_dir: std::path::PathBuf,
) -> Result<BackupOutcome, BackupBuildError> {
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

    // Archive names already written.
    //
    // `BACKUP_LAYOUT`'s scopes overlap by design — `data/cron_jobs.json` is
    // covered by both the `cron_jobs` row and the `data` tree — and `zip`
    // rejects a duplicate entry name outright rather than storing a second
    // copy. Without this set the `data` walk aborted on the first of those
    // three collisions, so the whole `data/` tree (the SQLite database
    // included) was left out of the archive while the response still reported
    // success and only a `tracing::warn!` recorded the loss.
    //
    // A `BTreeSet` rather than a `HashSet` so the skip decisions, and hence the
    // archive's entry order, are the same on every run.
    let mut written: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Helper: add a single file to the zip under `archive_name`.
    let add_file = |zip: &mut zip::ZipWriter<std::fs::File>,
                    written: &mut std::collections::BTreeSet<String>,
                    src: &std::path::Path,
                    archive_name: &str|
     -> Result<(), String> {
        if !written.insert(archive_name.to_string()) {
            // Already stored by an earlier scope; the entry is in the archive.
            return Ok(());
        }
        let data = std::fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?;
        zip.start_file(archive_name, options)
            .map_err(|e| format!("zip start {archive_name}: {e}"))?;
        std::io::Write::write_all(zip, &data)
            .map_err(|e| format!("zip write {archive_name}: {e}"))?;
        Ok(())
    };

    // Helper: recursively add a directory to the zip under `prefix`.
    // Returns how many files the tree contributes to the archive.
    let add_dir = |zip: &mut zip::ZipWriter<std::fs::File>,
                   written: &mut std::collections::BTreeSet<String>,
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
            if !path.is_file() {
                continue;
            }
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_sqlite_shared_memory_index)
            {
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| format!("strip prefix: {e}"))?;
            // Zip entry names are `/`-separated by spec. Rendering the
            // relative `Path` directly yields `\` on Windows, producing an
            // archive whose nested entry names `enclosed_name` then refuses on
            // read — the files would be backed up and never restored.
            let rel_name = archive_entry_key(rel);
            let archive_name = if prefix.is_empty() {
                rel_name
            } else {
                format!("{prefix}/{rel_name}")
            };
            // Counted before the dedup check: a file an earlier scope already
            // stored is still part of this component.
            count += 1;
            if !written.insert(archive_name.clone()) {
                continue;
            }
            let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
            zip.start_file(&archive_name, options)
                .map_err(|e| format!("zip start {archive_name}: {e}"))?;
            std::io::Write::write_all(zip, &data)
                .map_err(|e| format!("zip write {archive_name}: {e}"))?;
        }
        Ok(count)
    };

    // Walk the layout table rather than a hand-written ladder, so the set of
    // components written here cannot drift from the set the restore classifier
    // recognises. Order is the table's order, which keeps `manifest.json`'s
    // `components` list stable: the narrow `File` scopes come before the `data`
    // tree that also covers them, so each entry is stored once, under the
    // component that names it.
    for (component, scope) in BACKUP_LAYOUT {
        match *scope {
            ArchiveScope::File(archive) => {
                let src = home_dir.join(archive);
                if !src.exists() {
                    continue;
                }
                match add_file(&mut zip, &mut written, &src, archive) {
                    Ok(()) => components.push((*component).to_string()),
                    Err(e) => tracing::warn!("Backup: skipping {archive}: {e}"),
                }
            }
            ArchiveScope::Tree(archive) => {
                let src = backup_source(&home_dir, &agent_workspaces_dir, archive);
                if !src.exists() {
                    continue;
                }
                match add_dir(&mut zip, &mut written, &src, archive) {
                    Ok(n) if n > 0 => components.push((*component).to_string()),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Backup: skipping {archive}/: {e}"),
                }
            }
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
    // Resolved rather than assumed: `workspaces_dir` can move the agent
    // workspaces tree off `<home>/workspaces`, and a backup that read the
    // default path on such a host archived nothing under `agents/`.
    let agent_workspaces_dir = state
        .kernel
        .config_snapshot()
        .effective_agent_workspaces_dir();

    // Dispatch the heavy `walkdir` + `std::fs` work onto a blocking
    // thread. We must not hold any `!Send` value (notably
    // `ErrorTranslator`, which wraps the fluent bundle) across this
    // `.await` — the axum `Handler` bound rejects non-Send futures
    // with a cryptic trait-bound error. The translator is constructed
    // separately on each error branch below so it never crosses the
    // suspend point.
    let result =
        tokio::task::spawn_blocking(move || create_backup_blocking(home_dir, agent_workspaces_dir))
            .await;

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
/// Declared size below which the compression-ratio check does not apply.
///
/// The ratio guard exists to reject an entry that decompresses to far more
/// than it costs to ship; an entry this small cannot bloat the restore no
/// matter its ratio, and the absolute per-entry and total caps above still
/// apply to it. Flooring the check matters because genuinely sparse files are
/// normal in a LibreFang home: SQLite's `-shm` and `-wal` sidecars are mostly
/// zero pages and deflate several hundred to one, so an unfloored 100:1 limit
/// rejected archives this very endpoint had just written.
const MAX_RESTORE_RATIO_FLOOR_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RESTORE_MANIFEST_BYTES: u64 = 1024 * 1024;

fn prepare_restore_target(
    canonical_root: &std::path::Path,
    entry_name: &std::path::Path,
) -> Result<std::path::PathBuf, RestoreError> {
    let mut parent = canonical_root.to_path_buf();
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
            if !canonical_parent.starts_with(canonical_root) {
                return Err(RestoreError::InvalidArchive(
                    "restore path escapes the restore root".to_string(),
                ));
            }
        }
    }

    let target = canonical_root.join(entry_name);
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
    agent_workspaces_dir: std::path::PathBuf,
    keep_config: bool,
    components: Option<Vec<String>>,
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
    let mut archive_has_agent_entries = false;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| RestoreError::InvalidArchive(e.to_string()))?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| RestoreError::InvalidArchive("unsafe entry name".to_string()))?;
        if name.starts_with(AGENTS_ARCHIVE_PREFIX) {
            archive_has_agent_entries = true;
        }
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
        // A header claiming content but no compressed bytes is a lie about the
        // stream rather than a ratio, so it is rejected at any size.
        if declared_size > 0 && compressed_size == 0 {
            return Err(RestoreError::ResourceLimit(format!(
                "entry {} declares content but no compressed bytes",
                name.display()
            )));
        }
        if declared_size > MAX_RESTORE_RATIO_FLOOR_BYTES
            && declared_size > compressed_size.saturating_mul(MAX_RESTORE_COMPRESSION_RATIO)
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

    // `agents/` entries restore into the agent workspaces directory, which can
    // sit outside the home directory and need not exist yet on a fresh host.
    // Resolve it once, and only when the archive actually carries agent
    // entries, so an ordinary restore never creates a stray directory.
    let canonical_agent_workspaces = if archive_has_agent_entries {
        std::fs::create_dir_all(&agent_workspaces_dir).map_err(|error| {
            RestoreError::Open(format!("create agent workspaces directory: {error}"))
        })?;
        std::fs::canonicalize(&agent_workspaces_dir).map_err(|error| {
            RestoreError::Open(format!("resolve agent workspaces directory: {error}"))
        })?
    } else {
        canonical_home.clone()
    };

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

        // Both filters are answered by `BACKUP_LAYOUT`, the same table
        // `create_backup_blocking` archives from, so a component can never
        // mean one set of entries on the way out and another on the way back.
        let entry_key = archive_entry_key(&entry_name);
        // Archives written before the exclusion above still carry a `-shm`, and restoring one is
        // what fails on Windows. Skipping it is not a partial restore: SQLite rebuilds the index
        // from the database and its `-wal` on the next connection.
        if is_sqlite_shared_memory_index(&entry_key) {
            continue;
        }
        if keep_config && entry_belongs_to(&entry_key, "config") {
            continue;
        }
        if let Some(selected) = &components {
            // Entries no component owns are archive metadata rather than
            // state, so a narrow selection restores them anyway.
            if entry_is_classified(&entry_key) && !entry_is_selected(&entry_key, selected) {
                continue;
            }
        }

        // `agents/` was archived out of the agent workspaces directory rather
        // than out of `<home>/agents/`, so it restores under a different root.
        // Splitting the mapping into a root plus a relative path lets the
        // traversal and symlink hardening below run against whichever root the
        // entry actually lands in.
        let (root, relative) =
            restore_root(&canonical_home, &canonical_agent_workspaces, &entry_name);

        if entry.is_dir() {
            prepare_restore_target(root, &relative.join("placeholder"))?;
            continue;
        }

        let target = prepare_restore_target(root, &relative)?;

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
        // `entry_name` is a `Path`, so rendering it directly would hand back `\` separators on Windows and the restored list would stop matching the `/`-separated archive keys every caller compares against.
        // `entry_key`, computed above for the classification filters, is already normalised to `/`.
        restored.push(entry_key);
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
/// Two optional fields narrow what gets written:
///
/// - `keep_config` (bool, default `false`) skips `config.toml`, so the target
///   keeps its own API key, bind port and paths (clone mode).
/// - `components` (array of strings) limits the restore to the named
///   components — `config`, `cron_jobs`, `hand_state`, `custom_models`,
///   `agents`, `skills`, `workflows`, `data`. **Omit the field** to restore
///   everything; `[]` and any unrecognised name are rejected with `400` rather
///   than quietly restoring nothing. Archive entries no component owns are
///   restored regardless of the selection.
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

    // Selective restore: `keep_config` skips config.toml (clone mode — the
    // target keeps its own key, port and paths), and `components` limits the
    // restore to the names in `BACKUP_LAYOUT`.
    //
    // Both are validated rather than coerced. A restore is destructive and not
    // undoable, and every way of misreading these two fields fails silently
    // with a plausible-looking 200: reading `keep_config: "true"` as `false`
    // overwrites the very config the operator asked to keep, and reading a
    // malformed `components` as "no filter" overwrites everything. So a
    // malformed selection is a 400, never a best-effort restore.
    let keep_config = match req.get("keep_config") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(b)) => *b,
        Some(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-backup-invalid-keep-config"))
                .into_json_tuple();
        }
    };
    let components: Option<Vec<String>> = match req.get("components") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Array(items)) => {
            // `[]` is rejected rather than read as either extreme. "Restore
            // nothing" and "restore everything" are the two furthest-apart
            // outcomes this endpoint has, an empty list is far more often a
            // client that built the array wrong than a deliberate request to
            // restore nothing, and omitting the field is already the
            // unambiguous way to ask for everything.
            if items.is_empty() {
                return ApiErrorResponse::bad_request(t.t("api-error-backup-empty-components"))
                    .into_json_tuple();
            }
            let mut names: Vec<String> = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item.as_str() else {
                    return ApiErrorResponse::bad_request(
                        t.t("api-error-backup-invalid-components"),
                    )
                    .into_json_tuple();
                };
                // A name no component owns matches no archive entry, so
                // accepting it would turn `"agent"` for `"agents"` into a
                // successful restore of nothing.
                if !is_known_backup_component(name) {
                    return ApiErrorResponse::bad_request(t.t_args(
                        "api-error-backup-unknown-component",
                        &[("component", name), ("valid", &backup_component_names())],
                    ))
                    .into_json_tuple();
                }
                names.push(name.to_string());
            }
            Some(names)
        }
        Some(_) => {
            return ApiErrorResponse::bad_request(t.t("api-error-backup-invalid-components"))
                .into_json_tuple();
        }
    };

    let home_dir = state.kernel.home_dir().to_path_buf();
    let agent_workspaces_dir = state
        .kernel
        .config_snapshot()
        .effective_agent_workspaces_dir();
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
        restore_backup_blocking(
            backup_path,
            home_dir,
            agent_workspaces_dir,
            keep_config,
            components,
        )
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

    /// Joined form of `restore_root`, kept for the round-trip assertions
    /// below: they are about the mapping, not about how the extract loop
    /// consumes it.
    fn restore_target(
        home_dir: &std::path::Path,
        agent_workspaces_dir: &std::path::Path,
        entry: &std::path::Path,
    ) -> std::path::PathBuf {
        let (root, relative) = restore_root(home_dir, agent_workspaces_dir, entry);
        root.join(relative)
    }

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
        // Above `MAX_RESTORE_RATIO_FLOOR_BYTES`, so the ratio guard applies.
        zip.write_all(&vec![0_u8; 16 * 1024 * 1024]).unwrap();
        zip.finish().unwrap();

        let result = restore_backup_blocking(
            archive_path,
            restore_dir.clone(),
            restore_dir.join("workspaces").join("agents"),
            false,
            None,
        );
        assert!(matches!(result, Err(RestoreError::ResourceLimit(_))));
        assert!(!restore_dir.join("data/bomb.bin").exists());
    }

    /// SQLite's `-shm` / `-wal` sidecars are mostly zero pages and deflate far
    /// past 100:1, so an unfloored ratio guard rejected archives `create_backup`
    /// had just written. Small entries are exempt; the absolute caps still hold.
    #[test]
    fn restore_accepts_a_small_highly_compressible_entry() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("sparse.zip");
        let restore_dir = temp.path().join("restore");
        std::fs::create_dir(&restore_dir).unwrap();

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            br#"{"version":1,"created_at":"now","hostname":"host","librefang_version":"test","components":["data"]}"#,
        )
        .unwrap();
        zip.start_file("data/a2a_tasks.db-wal", options).unwrap();
        zip.write_all(&vec![0_u8; 32 * 1024]).unwrap();
        zip.finish().unwrap();

        let outcome = restore_backup_blocking(
            archive_path,
            restore_dir.clone(),
            restore_dir.join("workspaces").join("agents"),
            false,
            None,
        )
        .unwrap();
        assert_eq!(outcome.restored, vec!["data/a2a_tasks.db-wal"]);
        assert_eq!(
            std::fs::metadata(restore_dir.join("data").join("a2a_tasks.db-wal"))
                .unwrap()
                .len(),
            32 * 1024
        );
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

        let outcome = restore_backup_blocking(
            archive_path,
            restore_dir.clone(),
            restore_dir.join("workspaces").join("agents"),
            false,
            None,
        )
        .unwrap();
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

        let result = restore_backup_blocking(
            archive_path,
            restore_dir.clone(),
            restore_dir.join("workspaces").join("agents"),
            false,
            None,
        );
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

        let result = restore_backup_blocking(
            archive_path,
            restore_dir.clone(),
            restore_dir.join("workspaces").join("agents"),
            false,
            None,
        );
        assert!(matches!(result, Err(RestoreError::InvalidArchive(_))));
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"keep me");
    }

    /// `data/` owns every entry under it, the three named JSON files included.
    /// A first-match-wins classifier gave `components: ["data"]` a hole at
    /// exactly those three, so selecting `data` skipped the cron jobs, hand
    /// state and custom models it was meant to bring back.
    #[test]
    fn data_owns_the_named_json_files_inside_it() {
        for entry in [
            "data/cron_jobs.json",
            "data/hand_state.json",
            "data/custom_models.json",
            "data/memory.sqlite",
        ] {
            assert!(entry_belongs_to(entry, "data"), "`data` must own {entry}");
        }
        // …and each still belongs to its own narrower component.
        assert!(entry_belongs_to("data/cron_jobs.json", "cron_jobs"));
        assert!(entry_belongs_to("data/hand_state.json", "hand_state"));
        assert!(entry_belongs_to("data/custom_models.json", "custom_models"));
        assert!(!entry_belongs_to("data/memory.sqlite", "cron_jobs"));
    }

    /// A `Tree` scope matches on path components, never on a string prefix, so
    /// a sibling directory whose name merely starts the same way is not swept
    /// into the selection.
    #[test]
    fn tree_scopes_match_whole_path_components() {
        assert!(entry_belongs_to("skills/a/b.md", "skills"));
        assert!(!entry_belongs_to("skills-archive/a.md", "skills"));
        assert!(!entry_belongs_to("data-old/cron_jobs.json", "data"));
    }

    /// Entries no component owns are archive metadata rather than state, so a
    /// narrow selection must leave them alone rather than skip them.
    #[test]
    fn unowned_entries_are_unclassified() {
        assert!(!entry_is_classified("integrations.toml"));
        assert!(entry_is_classified("config.toml"));
        assert!(entry_is_classified("workflows/nightly.toml"));
    }

    /// `backup_source` and `restore_target` are the two halves of one mapping;
    /// they drifted for the `agents` component and the component silently
    /// stopped round-tripping. Assert they are inverse for every tree in the
    /// layout.
    #[test]
    fn backup_source_and_restore_target_are_inverse() {
        let home = std::path::Path::new("/home/.librefang");
        let agents = std::path::Path::new("/elsewhere/workspaces/agents");
        for (_, scope) in BACKUP_LAYOUT {
            let ArchiveScope::Tree(prefix) = *scope else {
                continue;
            };
            let src = backup_source(home, agents, prefix);
            let entry = std::path::Path::new(prefix).join("nested").join("f.toml");
            assert_eq!(
                restore_target(home, agents, &entry),
                src.join("nested").join("f.toml"),
                "{prefix}/ must restore to the directory it was archived from"
            );
        }
    }

    /// The `agents/` prefix is the one case where the archive path is not the
    /// home-relative path, and getting it wrong is invisible: the files land in
    /// the pre-unification `<home>/agents/` layout that nothing reads.
    #[test]
    fn agents_entries_restore_into_the_agent_workspaces_dir() {
        let home = std::path::Path::new("/home/.librefang");
        let agents = home.join("workspaces").join("agents");
        assert_eq!(
            restore_target(
                home,
                &agents,
                std::path::Path::new("agents/scout/agent.toml")
            ),
            agents.join("scout").join("agent.toml")
        );
        // A home-relative component is unaffected.
        assert_eq!(
            restore_target(home, &agents, std::path::Path::new("skills/a.md")),
            home.join("skills").join("a.md")
        );
    }

    #[test]
    fn only_layout_names_are_accepted_components() {
        assert!(is_known_backup_component("agents"));
        assert!(!is_known_backup_component("agent"));
        assert!(!is_known_backup_component(""));
        let valid = backup_component_names();
        assert!(valid.contains("agents") && valid.contains("custom_models"));
    }

    /// Archive entry names are `/`-separated regardless of host, so the key the
    /// layout is matched against must be too.
    #[test]
    fn archive_entry_key_is_slash_separated() {
        assert_eq!(
            archive_entry_key(
                std::path::Path::new("data")
                    .join("cron_jobs.json")
                    .as_path()
            ),
            "data/cron_jobs.json"
        );
    }

    /// A `-shm` in an archive written before the exclusion must be skipped, not written.
    ///
    /// On Windows SQLite memory-maps the file and truncating it fails with
    /// `ERROR_USER_MAPPED_FILE`, which is what made every restore report a partial failure there.
    /// Skipping loses nothing: SQLite rebuilds the index from the database and its `-wal`.
    #[test]
    fn restore_skips_the_sqlite_shared_memory_index() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("legacy.zip");
        let restore_dir = temp.path().join("restore");
        std::fs::create_dir(&restore_dir).unwrap();

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            br#"{"version":1,"created_at":"now","hostname":"host","librefang_version":"test","components":["data"]}"#,
        )
        .unwrap();
        zip.start_file("data/memory.sqlite", options).unwrap();
        zip.write_all(b"db-bytes").unwrap();
        zip.start_file("data/memory.sqlite-shm", options).unwrap();
        zip.write_all(b"index-bytes").unwrap();
        zip.start_file("data/memory.sqlite-wal", options).unwrap();
        zip.write_all(b"wal-bytes").unwrap();
        zip.finish().unwrap();

        let outcome = restore_backup_blocking(
            archive_path,
            restore_dir.clone(),
            restore_dir.join("workspaces").join("agents"),
            false,
            None,
        )
        .unwrap();

        // Skipped, not failed: the `-shm` must never reach the errors list either.
        assert_eq!(outcome.errors, Vec::<String>::new());
        assert_eq!(
            outcome.restored,
            vec!["data/memory.sqlite", "data/memory.sqlite-wal"]
        );
        assert!(
            !restore_dir.join("data").join("memory.sqlite-shm").exists(),
            "the shared-memory index must not be written over a live database"
        );
    }

    /// The exclusion has to hold on the create side too, or every archive keeps carrying an entry
    /// the restore then has to skip.
    #[test]
    fn the_sqlite_shared_memory_index_is_never_archived() {
        assert!(is_sqlite_shared_memory_index("data/memory.sqlite-shm"));
        assert!(is_sqlite_shared_memory_index("a2a_tasks.db-shm"));
        assert!(!is_sqlite_shared_memory_index("data/memory.sqlite"));
        assert!(!is_sqlite_shared_memory_index("data/memory.sqlite-wal"));
        assert!(!is_sqlite_shared_memory_index("data/shm"));
    }

    /// The reported list is a contract, not a debug string: callers match it against the `/`-separated archive keys.
    /// Building the expectation with `Path::join` makes the assertion itself platform-agnostic, so a host that renders `\` cannot quietly agree with itself.
    #[test]
    fn restored_entries_are_reported_with_archive_separators() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("nested.zip");
        let restore_dir = temp.path().join("restore");
        std::fs::create_dir(&restore_dir).unwrap();

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            br#"{"version":1,"created_at":"now","hostname":"host","librefang_version":"test","components":["data"]}"#,
        )
        .unwrap();
        zip.start_file("data/cron_jobs.json", options).unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();

        let outcome = restore_backup_blocking(
            archive_path,
            restore_dir.clone(),
            restore_dir.join("workspaces").join("agents"),
            false,
            None,
        )
        .unwrap();

        assert_eq!(outcome.errors, Vec::<String>::new());
        assert_eq!(outcome.restored, vec!["data/cron_jobs.json"]);
        assert!(
            !outcome.restored.iter().any(|entry| entry.contains('\\')),
            "restored entries must never carry a host separator: {:?}",
            outcome.restored
        );
    }
}
