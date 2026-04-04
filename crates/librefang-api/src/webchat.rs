//! Dashboard pages and static assets served by the API daemon.
//!
//! Assets are resolved in order:
//! 1. Runtime directory: `~/.librefang/dashboard/` (downloaded/updated at startup)
//! 2. Compile-time embedded: `static/react/` via `include_dir!` (fallback)
//!
//! This allows the dashboard to be updated without recompiling, while still
//! providing a working dashboard in single-binary distributions.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use include_dir::{include_dir, Dir};
use std::sync::Arc;

/// Compile-time ETag based on the crate version.
const ETAG: &str = concat!("\"librefang-", env!("CARGO_PKG_VERSION"), "\"");

/// Compile-time embedded dashboard (fallback).
static REACT_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/static/react");

/// Embedded logo PNG for single-binary deployment.
const LOGO_PNG: &[u8] = include_bytes!("../static/logo.png");

/// Embedded favicon ICO for browser tabs.
const FAVICON_ICO: &[u8] = include_bytes!("../static/favicon.ico");
const LOCALE_EN: &str = include_str!("../static/locales/en.json");
const LOCALE_ZH_CN: &str = include_str!("../static/locales/zh-CN.json");
const LOCALE_JA: &str = include_str!("../static/locales/ja.json");

/// Resolve a dashboard file: try runtime dir first, then embedded fallback.
fn resolve_dashboard_file(
    home_dir: Option<&std::path::Path>,
    relative_path: &str,
) -> Option<Vec<u8>> {
    // 1. Try runtime directory
    if let Some(home) = home_dir {
        let runtime_path = home.join("dashboard").join(relative_path);
        if let Ok(data) = std::fs::read(&runtime_path) {
            return Some(data);
        }
    }

    // 2. Fall back to embedded
    REACT_DIST
        .get_file(relative_path)
        .map(|f| f.contents().to_vec())
}

/// GET /logo.png — Serve the LibreFang logo.
pub async fn logo_png() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        LOGO_PNG,
    )
}

/// GET /favicon.ico — Serve the LibreFang favicon.
pub async fn favicon_ico() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        FAVICON_ICO,
    )
}

pub async fn locale_en() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        LOCALE_EN,
    )
}

pub async fn locale_zh_cn() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        LOCALE_ZH_CN,
    )
}

pub async fn locale_ja() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        LOCALE_JA,
    )
}

/// GET / — Serve the React dashboard shell.
pub async fn webchat_page(State(state): State<Arc<crate::routes::AppState>>) -> impl IntoResponse {
    let home_dir = Some(state.kernel.home_dir().to_path_buf());
    match resolve_dashboard_file(home_dir.as_deref(), "index.html") {
        Some(data) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::ETAG, ETAG),
                (
                    header::CACHE_CONTROL,
                    "public, max-age=300, must-revalidate",
                ),
            ],
            data,
        )
            .into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "React dashboard build missing (expected static/react/index.html)",
        )
            .into_response(),
    }
}

/// GET /dashboard/{*path} — Serve React build assets.
pub async fn react_asset(
    State(state): State<Arc<crate::routes::AppState>>,
    Path(path): Path<String>,
) -> Response {
    if path.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid asset path").into_response();
    }

    let asset_path = path.trim_start_matches('/');
    let home_dir = Some(state.kernel.home_dir().to_path_buf());
    match resolve_dashboard_file(home_dir.as_deref(), asset_path) {
        Some(data) => (
            [
                (header::CONTENT_TYPE, content_type_for(asset_path)),
                (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
            ],
            data,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "asset not found").into_response(),
    }
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".json") {
        "application/json; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

/// Sync dashboard assets from GitHub to `~/.librefang/dashboard/`.
///
/// Downloads the dashboard-dist branch tarball and extracts it.
/// Called during daemon startup (non-blocking).
pub async fn sync_dashboard(home_dir: &std::path::Path) {
    let dashboard_dir = home_dir.join("dashboard");
    let version_file = dashboard_dir.join(".version");

    // Skip if already synced for this version
    let current_version = env!("CARGO_PKG_VERSION");
    if let Ok(cached) = std::fs::read_to_string(&version_file) {
        if cached.trim() == current_version {
            tracing::debug!("Dashboard already synced for v{current_version}");
            return;
        }
    }

    let url =
        "https://github.com/librefang/librefang/releases/latest/download/dashboard-dist.tar.gz";
    tracing::info!("Syncing dashboard assets from release...");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    let response = match client.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::debug!(
                "Dashboard sync skipped (HTTP {}), using embedded fallback",
                r.status()
            );
            return;
        }
        Err(e) => {
            tracing::debug!("Dashboard sync skipped ({e}), using embedded fallback");
            return;
        }
    };

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Failed to download dashboard: {e}");
            return;
        }
    };

    // Extract tarball
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(&bytes));
    let mut archive = tar::Archive::new(decoder);

    let tmp_dir = dashboard_dir.with_file_name("dashboard_tmp");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        tracing::warn!("Failed to create tmp dir: {e}");
        return;
    }

    if let Err(e) = archive.unpack(&tmp_dir) {
        tracing::warn!("Failed to extract dashboard archive: {e}");
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return;
    }

    // Find the extracted directory (tarball root may have a prefix)
    let extracted = std::fs::read_dir(&tmp_dir)
        .ok()
        .and_then(|mut entries| entries.next())
        .and_then(|e| e.ok())
        .map(|e| e.path());

    let source = if let Some(ref dir) = extracted {
        if dir.is_dir() && dir.join("index.html").exists() {
            dir.as_path()
        } else {
            &tmp_dir
        }
    } else {
        &tmp_dir
    };

    // Atomic swap
    let _ = std::fs::remove_dir_all(&dashboard_dir);
    if let Err(e) = std::fs::rename(source, &dashboard_dir) {
        // rename may fail across filesystems, fall back to copy
        tracing::debug!("rename failed ({e}), falling back to copy");
        if let Err(e) = copy_dir_recursive(source, &dashboard_dir) {
            tracing::warn!("Failed to copy dashboard: {e}");
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return;
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Write version marker
    let _ = std::fs::write(&version_file, current_version);
    tracing::info!("Dashboard synced to v{current_version}");
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}
