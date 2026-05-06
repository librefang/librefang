//! Dashboard pages and static assets served by the API daemon.
//!
//! Assets are compiled into the binary at build time via `include_dir!`.
//! `crates/librefang-api/build.rs` runs `pnpm run build` in the
//! `dashboard/` sub-crate and the Vite output lands in `static/react/`,
//! which is then embedded here.
//!
//! There is no runtime download or sync step — the binary is self-contained.

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use include_dir::{include_dir, Dir};

/// Compile-time ETag based on the crate version.
const ETAG: &str = concat!("\"librefang-", env!("CARGO_PKG_VERSION"), "\"");

/// Compile-time embedded dashboard built by `pnpm run build`.
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
/// sync entirely.
const EMBEDDED_ONLY_ENV: &str = "LIBREFANG_DASHBOARD_EMBEDDED_ONLY";

fn embedded_only_mode() -> bool {
    is_embedded_only_value(std::env::var(EMBEDDED_ONLY_ENV).ok().as_deref())
}

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

#[cfg(test)]
fn resolve_dashboard_file_with_mode(
    home_dir: Option<&std::path::Path>,
    relative_path: &str,
    embedded_only: bool,
) -> Option<Vec<u8>> {
    if !embedded_only {
        if let Some(home) = home_dir {
            let runtime_path = home.join("dashboard").join(relative_path);
            if let Ok(data) = std::fs::read(&runtime_path) {
                return Some(data);
            }
        }
    }
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
pub async fn webchat_page() -> impl IntoResponse {
    match REACT_DIST.get_file("index.html") {
        Some(f) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::ETAG, ETAG),
                (
                    header::CACHE_CONTROL,
                    "public, max-age=300, must-revalidate",
                ),
            ],
            f.contents().to_vec(),
        )
            .into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "dashboard not built — run pnpm build in crates/librefang-api/dashboard",
        )
            .into_response(),
    }
}

/// GET /dashboard/{*path} — Serve React build assets.
pub async fn react_asset(Path(path): Path<String>) -> Response {
    if path.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid asset path").into_response();
    }

    let asset_path = path.trim_start_matches('/');
    match REACT_DIST.get_file(asset_path) {
        Some(f) => (
            [
                (header::CONTENT_TYPE, content_type_for(asset_path)),
                (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
            ],
            f.contents().to_vec(),
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
                if let Some(index) = REACT_DIST.get_file("index.html") {
                    return (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        index.contents().to_vec(),
                    )
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
