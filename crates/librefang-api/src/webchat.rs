//! Dashboard pages and static assets served by the API daemon.
//!
//! Assets are resolved in order:
//! 1. Runtime directory: `~/.librefang/dashboard/` (downloaded/updated at startup)
//! 2. Compile-time embedded: `static/react/` via `include_dir!` (fallback)
//!
//! This allows the dashboard to be updated without recompiling, while still
//! providing a working dashboard in single-binary distributions.
//!
//! ## Opt-out: embedded-only mode
//!
//! Setting `LIBREFANG_DASHBOARD_EMBEDDED_ONLY=1` pins the resolver to the
//! compile-time-embedded assets and short-circuits [`sync_dashboard`]. This is
//! the right setting when you want the dashboard served by the daemon to
//! exactly match the binary you built, e.g.:
//!
//! - Iterating on the dashboard locally against your own `cargo build`.
//! - Running in a packaged environment where the dashboard is intentionally
//!   frozen to the build artifact and must not mutate at runtime.
//!
//! Accepted truthy values: `1`, `true`, `yes`, `on` (case-insensitive). Any
//! other value — or the absence of the variable — leaves the default
//! runtime-sync behavior intact.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use include_dir::{include_dir, Dir};
use std::sync::Arc;

/// Compile-time ETag based on the crate version.
const ETAG: &str = concat!("\"librefang-", env!("CARGO_PKG_VERSION"), "\"");

/// Loading page shown while dashboard assets are being downloaded.
const LOADING_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<meta http-equiv="refresh" content="3">
<title>BossFang</title>
<style>
  body{font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f8f9fa;color:#333}
  .c{text-align:center}
  .spinner{width:32px;height:32px;border:3px solid #e0e0e0;border-top-color:#E04E28;border-radius:50%;animation:spin .8s linear infinite;margin:0 auto 16px}
  @keyframes spin{to{transform:rotate(360deg)}}
</style>
</head>
<body>
<div class="c">
  <div class="spinner"></div>
  <p>Downloading dashboard assets…</p>
</div>
</body>
</html>"#;

/// Error page shown when dashboard sync failed and no embedded fallback exists.
const DASHBOARD_UNAVAILABLE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>BossFang</title>
<style>
  body{font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f8f9fa;color:#333}
  .c{max-width:520px;padding:24px;text-align:center}
  h1{font-size:20px;margin:0 0 12px}
  p{line-height:1.5;margin:0 0 12px}
  code{background:#eee;padding:2px 6px;border-radius:4px}
</style>
</head>
<body>
<div class="c">
  <h1>Dashboard assets unavailable</h1>
  <p>BossFang could not load the dashboard assets from disk, and the runtime download did not complete.</p>
  <p>Restart the app after network access is available, or build the desktop app with embedded dashboard assets.</p>
</div>
</body>
</html>"#;

/// Compile-time embedded dashboard (fallback).
static REACT_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/static/react");

/// Embedded logo PNG for single-binary deployment.
const LOGO_PNG: &[u8] = include_bytes!("../static/logo.png");

/// Embedded favicon ICO for browser tabs.
const FAVICON_ICO: &[u8] = include_bytes!("../static/favicon.ico");
const LOCALE_EN: &str = include_str!("../static/locales/en.json");
const LOCALE_ZH_CN: &str = include_str!("../static/locales/zh-CN.json");
const LOCALE_JA: &str = include_str!("../static/locales/ja.json");

const DASHBOARD_SYNC_ERROR_FILE: &str = ".sync-error";

/// Environment variable that, when set to a truthy value, forces the dashboard
/// resolver to serve the compile-time-embedded assets and skips the release
/// sync entirely. See the module-level docs for details.
const EMBEDDED_ONLY_ENV: &str = "LIBREFANG_DASHBOARD_EMBEDDED_ONLY";

fn embedded_dashboard_available() -> bool {
    REACT_DIST.get_file("index.html").is_some()
}

/// BossFang fork: always serve the compile-time-embedded dashboard so the
/// upstream `sync_dashboard` path can never overwrite our branded assets by
/// downloading from `github.com/librefang/librefang`.
///
/// Default is **embedded-only** (returns `true`). Opt out by setting
/// `LIBREFANG_DASHBOARD_EMBEDDED_ONLY=0` (or `false`, `no`, `off`) if you
/// explicitly want the runtime-dir override for local dev.
fn embedded_only_mode() -> bool {
    // BossFang default: embedded-only=true unless explicitly opted out.
    match std::env::var(EMBEDDED_ONLY_ENV).ok().as_deref() {
        Some(v) => {
            let n = v.trim().to_ascii_lowercase();
            // A falsy value opts OUT of embedded-only (i.e. allows runtime dir).
            !matches!(n.as_str(), "0" | "false" | "no" | "off")
        }
        None => true, // default: always use embedded BossFang assets
    }
}

/// Pure parser split out so tests can exercise the value grammar without
/// touching process-global environment state.
#[cfg(test)]
fn is_embedded_only_value(raw: Option<&str>) -> bool {
    match raw {
        Some(v) => {
            let normalized = v.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        None => false,
    }
}

fn dashboard_sync_error_path(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir.join("dashboard").join(DASHBOARD_SYNC_ERROR_FILE)
}

/// Resolve a dashboard file: try runtime dir first, then embedded fallback.
///
/// In embedded-only mode (see [`embedded_only_mode`]) the runtime directory
/// is skipped entirely so the compile-time assets win regardless of whatever
/// stale copy may still be sitting in `$LIBREFANG_HOME/dashboard/` from a
/// previous sync.
fn resolve_dashboard_file(
    home_dir: Option<&std::path::Path>,
    relative_path: &str,
) -> Option<Vec<u8>> {
    resolve_dashboard_file_with_mode(home_dir, relative_path, embedded_only_mode())
}

/// Testable variant of [`resolve_dashboard_file`] that takes the
/// embedded-only decision as a parameter instead of reading it from the
/// environment. Keeps the public entry point ergonomic while letting unit
/// tests exercise both branches deterministically.
fn resolve_dashboard_file_with_mode(
    home_dir: Option<&std::path::Path>,
    relative_path: &str,
    embedded_only: bool,
) -> Option<Vec<u8>> {
    // 1. Try runtime directory (skipped when embedded-only mode is on).
    if !embedded_only {
        if let Some(home) = home_dir {
            let runtime_path = home.join("dashboard").join(relative_path);
            if let Ok(data) = std::fs::read(&runtime_path) {
                return Some(data);
            }
        }
    }

    // 2. Fall back to embedded
    REACT_DIST
        .get_file(relative_path)
        .map(|f| f.contents().to_vec())
}

/// GET /logo.png — Serve the BossFang logo.
pub async fn logo_png() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        LOGO_PNG,
    )
}

/// GET /boss-libre.png — Serve the BossFang mascot logo embedded from the
/// Vite build output. `App.tsx` references `/boss-libre.png` directly (not
/// under `/dashboard/`), so it needs its own top-level route.
pub async fn boss_libre_png() -> impl IntoResponse {
    match REACT_DIST.get_file("boss-libre.png") {
        Some(f) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
            ],
            f.contents().to_vec(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "boss-libre.png not found").into_response(),
    }
}

/// GET /favicon.ico — Serve the BossFang favicon.
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
        None => {
            let body = if embedded_dashboard_available() {
                LOADING_HTML
            } else if let Some(home) = home_dir.as_deref() {
                if dashboard_sync_error_path(home).exists() {
                    DASHBOARD_UNAVAILABLE_HTML
                } else {
                    LOADING_HTML
                }
            } else {
                LOADING_HTML
            };
            (
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                body,
            )
                .into_response()
        }
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
        None => {
            // SPA fallback: if the path has no file extension, serve index.html
            // so that browser-history routing works (e.g. /dashboard/config/general).
            let has_ext = asset_path
                .rsplit('/')
                .next()
                .is_some_and(|s| s.contains('.'));
            if !has_ext {
                if let Some(index) = resolve_dashboard_file(home_dir.as_deref(), "index.html") {
                    return ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], index)
                        .into_response();
                }
            }
            (StatusCode::NOT_FOUND, "asset not found").into_response()
        }
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
///
/// Short-circuits when [`EMBEDDED_ONLY_ENV`] is truthy so local builds and
/// frozen deployments aren't silently replaced by the release artifact.
pub async fn sync_dashboard(home_dir: &std::path::Path) {
    if embedded_only_mode() {
        tracing::info!(
            "{EMBEDDED_ONLY_ENV} is set; skipping dashboard sync and serving embedded assets only"
        );
        return;
    }

    let dashboard_dir = home_dir.join("dashboard");
    let version_file = dashboard_dir.join(".version");
    let sync_error_file = dashboard_sync_error_path(home_dir);

    // Skip if already synced for this version
    let current_version = env!("CARGO_PKG_VERSION");
    if let Ok(cached) = std::fs::read_to_string(&version_file) {
        if cached.trim() == current_version {
            tracing::debug!("Dashboard already synced for v{current_version}");
            let _ = std::fs::remove_file(&sync_error_file);
            return;
        }
    }

    let url =
        "https://github.com/librefang/librefang/releases/latest/download/dashboard-dist.tar.gz";
    tracing::info!("Syncing dashboard assets from release...");

    // Use librefang-http so dashboard sync respects [proxy] config (#3577).
    let client = librefang_http::proxied_client_builder()
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
            let _ = std::fs::write(
                &sync_error_file,
                format!("dashboard sync skipped: HTTP {}", r.status()),
            );
            return;
        }
        Err(e) => {
            tracing::debug!("Dashboard sync skipped ({e}), using embedded fallback");
            let _ = std::fs::write(&sync_error_file, format!("dashboard sync skipped: {e}"));
            return;
        }
    };

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Failed to download dashboard: {e}");
            let _ = std::fs::write(&sync_error_file, format!("dashboard download failed: {e}"));
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
        let _ = std::fs::write(&sync_error_file, format!("dashboard tmp dir failed: {e}"));
        return;
    }

    if let Err(e) = archive.unpack(&tmp_dir) {
        tracing::warn!("Failed to extract dashboard archive: {e}");
        let _ = std::fs::write(&sync_error_file, format!("dashboard extract failed: {e}"));
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

    // Atomic-ish swap: rename old dir to backup, move new dir in, then clean up.
    // If the swap fails, the backup is restored so we never lose a working dashboard.
    let backup_dir = dashboard_dir.with_file_name("dashboard_old");
    let _ = std::fs::remove_dir_all(&backup_dir);
    let had_existing = dashboard_dir.exists();
    if had_existing {
        if let Err(e) = std::fs::rename(&dashboard_dir, &backup_dir) {
            tracing::warn!("Failed to back up old dashboard: {e}");
            let _ = std::fs::write(&sync_error_file, format!("dashboard backup failed: {e}"));
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return;
        }
    }

    if let Err(e) = std::fs::rename(source, &dashboard_dir) {
        tracing::debug!("rename failed ({e}), falling back to copy");
        if let Err(e) = copy_dir_recursive(source, &dashboard_dir) {
            tracing::warn!("Failed to install dashboard: {e}");
            let _ = std::fs::write(&sync_error_file, format!("dashboard install failed: {e}"));
            // Restore backup
            if had_existing {
                let _ = std::fs::rename(&backup_dir, &dashboard_dir);
            }
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return;
        }
    }

    let _ = std::fs::remove_dir_all(&backup_dir);
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Write version marker
    let _ = std::fs::write(&version_file, current_version);
    let _ = std::fs::remove_file(&sync_error_file);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_only_unset_is_false() {
        assert!(!is_embedded_only_value(None));
    }

    #[test]
    fn embedded_only_truthy_values() {
        for v in [
            "1", "true", "TRUE", "True", "yes", "YES", "on", "ON", " 1 ", "\tTrue\n",
        ] {
            assert!(
                is_embedded_only_value(Some(v)),
                "expected {v:?} to be truthy"
            );
        }
    }

    #[test]
    fn embedded_only_falsy_values() {
        for v in [
            "",
            "0",
            "false",
            "no",
            "off",
            "FALSE",
            "nope",
            "anything-else",
        ] {
            assert!(
                !is_embedded_only_value(Some(v)),
                "expected {v:?} to be falsy"
            );
        }
    }

    #[test]
    fn resolve_dashboard_prefers_runtime_dir_when_not_embedded_only() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dashboard = tmp.path().join("dashboard");
        std::fs::create_dir_all(&dashboard).unwrap();
        let marker = b"runtime-dir-wins";
        std::fs::write(dashboard.join("test-marker.txt"), marker).unwrap();

        let got = resolve_dashboard_file_with_mode(Some(tmp.path()), "test-marker.txt", false);
        assert_eq!(got.as_deref(), Some(marker.as_slice()));
    }

    #[test]
    fn resolve_dashboard_skips_runtime_dir_in_embedded_only_mode() {
        // Put a file in the runtime dir that does NOT exist in the embedded
        // bundle — in embedded-only mode the resolver must ignore it and
        // return `None` instead of serving the stale runtime copy.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dashboard = tmp.path().join("dashboard");
        std::fs::create_dir_all(&dashboard).unwrap();
        std::fs::write(
            dashboard.join("definitely-not-in-embedded-bundle.bin"),
            b"stale-runtime",
        )
        .unwrap();

        let got = resolve_dashboard_file_with_mode(
            Some(tmp.path()),
            "definitely-not-in-embedded-bundle.bin",
            true,
        );
        assert!(
            got.is_none(),
            "embedded-only mode must not consult runtime dir"
        );
    }
}
