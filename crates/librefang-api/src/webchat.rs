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
<title>LibreFang</title>
<style>
  body{font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f8f9fa;color:#333}
  .c{text-align:center}
  .spinner{width:32px;height:32px;border:3px solid #e0e0e0;border-top-color:#666;border-radius:50%;animation:spin .8s linear infinite;margin:0 auto 16px}
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
<title>LibreFang</title>
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
  <p>LibreFang could not load the dashboard assets from disk, and the runtime download did not complete.</p>
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

/// Returns `true` when the operator has opted into embedded-only dashboard
/// mode via [`EMBEDDED_ONLY_ENV`]. Any of `1`, `true`, `yes`, `on` (case
/// insensitive) counts as truthy.
fn embedded_only_mode() -> bool {
    is_embedded_only_value(std::env::var(EMBEDDED_ONLY_ENV).ok().as_deref())
}

/// Pure parser split out so tests can exercise the value grammar without
/// touching process-global environment state.
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

/// Validate a dashboard asset sub-path.
///
/// Returns `true` if every segment (split on both `/` and `\`) is a benign,
/// non-empty filename — i.e. not `.`, not `..`, contains no null byte. This
/// is the path-traversal guard for [`react_asset`]; the previous substring
/// check `path.contains("..")` was bypassable on Windows via `\..\` segments
/// and, in principle, by URL-encoded `%2e%2e` if the decode order ever
/// changed. Splitting on both separators and rejecting `..` per-segment
/// closes both bypasses without depending on filesystem canonicalization
/// (which would require the asset to already exist).
fn is_safe_asset_path(path: &str) -> bool {
    // Reject embedded null bytes anywhere — defense in depth against C
    // string truncation in any downstream consumer.
    if path.contains('\0') {
        return false;
    }
    // Reject Windows UNC / authority-style prefixes outright. `\\server\share`
    // and `//server/share` carry no `..` segment, so the per-segment check
    // below passes them — but `Path::join` REPLACES the base with an absolute
    // or UNC path on Windows, so `home/dashboard`.join("\\server\share")
    // resolves to `\\server\share`, escaping the dashboard directory entirely.
    // A legitimate dashboard asset is always a relative sub-path, never an
    // authority reference, so refuse the prefix on every platform.
    if path.starts_with("\\\\") || path.starts_with("//") {
        return false;
    }
    // Split on BOTH forward slash and backslash. Backslash is a path
    // separator on Windows; on Unix it's a legal filename character but
    // a dashboard asset would never legitimately contain one.
    let mut saw_segment = false;
    for seg in path.split(['/', '\\']) {
        if seg.is_empty() {
            // Empty segments come from leading/trailing/repeated separators;
            // they're benign in URLs ("/a//b" canonicalizes to "/a/b") but
            // we tolerate them only between real segments — the resolver
            // path-joins on each, and double-separators don't traverse.
            continue;
        }
        if seg == "." || seg == ".." {
            return false;
        }
        saw_segment = true;
    }
    saw_segment
}

/// First path segments the React SPA owns under `/dashboard/`.
///
/// `react_asset` only serves `index.html` for an extensionless path whose
/// first segment is in this set; every other extensionless miss returns 404.
/// Without the allowlist, *any* `/dashboard/<word>` resolved to the dashboard
/// shell, so an attacker could craft `…/dashboard/security-alert` (or any
/// plausible-looking slug) and have it render the trusted UI chrome — a
/// ready-made phishing surface on the operator's own origin.
///
/// Source of truth: the top-level route paths in
/// `crates/librefang-api/dashboard/src/router.tsx` (the router uses
/// `basepath: "/dashboard"`, so a router `path: "/agents"` arrives here as the
/// asset path `agents`). Matching the FIRST segment also covers nested dynamic
/// routes such as `/dashboard/agents/<id>` and `/dashboard/config/general`.
/// Keep this list in sync when adding a new top-level dashboard route.
///
/// Sorted for readability / deterministic review diffs; lookup is a linear
/// scan over a handful of entries.
const SPA_ROUTES: &[&str] = &[
    "a2a",
    "agents",
    "analytics",
    "approvals",
    "audit",
    "canvas",
    "channels",
    "chat",
    "comms",
    "config",
    "connect",
    "goals",
    "hands",
    "logs",
    "mcp-servers",
    "media",
    "memory",
    "models",
    "network",
    "overview",
    "plugins",
    "providers",
    "runtime",
    "scheduler",
    "sessions",
    "settings",
    "skills",
    "telemetry",
    "terminal",
    "users",
    "wizard",
    "workflows",
];

/// True when `asset_path`'s first segment is a route the React SPA owns, so an
/// extensionless miss should fall back to `index.html` rather than 404.
fn is_spa_route(asset_path: &str) -> bool {
    let first = asset_path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("");
    SPA_ROUTES.contains(&first)
}

/// GET /dashboard/{*path} — Serve React build assets.
pub async fn react_asset(
    State(state): State<Arc<crate::routes::AppState>>,
    Path(path): Path<String>,
) -> Response {
    if !is_safe_asset_path(&path) {
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
            // SPA fallback: serve index.html so browser-history routing works
            // (e.g. /dashboard/config/general) — but ONLY for extensionless
            // paths whose first segment is a known SPA route. An extensionless
            // path that isn't an SPA route (e.g. /dashboard/security-alert)
            // returns 404 rather than rendering the trusted dashboard chrome,
            // closing the phishing-surface amplification.
            let has_ext = asset_path
                .rsplit('/')
                .next()
                .is_some_and(|s| s.contains('.'));
            if !has_ext && is_spa_route(asset_path) {
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

    #[test]
    fn safe_asset_path_accepts_legit_paths() {
        for p in [
            "index.html",
            "assets/app.js",
            "assets/img/logo.png",
            "sub/dir/file.css",
            "deep/nested/path/asset.svg",
        ] {
            assert!(is_safe_asset_path(p), "expected {p:?} to be accepted");
        }
    }

    #[test]
    fn safe_asset_path_rejects_dotdot_segment() {
        // ASCII traversal — what the old `path.contains("..")` check caught.
        for p in [
            "..",
            "../etc/passwd",
            "assets/../secret.toml",
            "sub/../../etc/passwd",
        ] {
            assert!(!is_safe_asset_path(p), "expected {p:?} to be rejected");
        }
    }

    #[test]
    fn safe_asset_path_rejects_backslash_dotdot_windows_bypass() {
        // The Windows-bypass class: substring `path.contains("..")` happens
        // to catch literal `..\\`, but it does NOT prevent the segment from
        // being treated as parent-of by Windows path resolution. The
        // segment-level validator rejects it explicitly regardless of host
        // OS, so the audit fix holds on every platform.
        for p in [
            "..\\etc\\passwd",
            "assets\\..\\secret.toml",
            "sub\\..\\..\\etc\\passwd",
            "..\\..\\Windows\\System32\\config\\SAM",
        ] {
            assert!(!is_safe_asset_path(p), "expected {p:?} to be rejected");
        }
    }

    #[test]
    fn safe_asset_path_rejects_url_decoded_dotdot() {
        // Axum's `Path<String>` extractor URL-decodes capture segments
        // before the handler sees them, so `%2e%2e` arrives as `..`. This
        // test pins the post-decode behaviour explicitly so a future
        // extractor change can't reopen the bypass silently.
        let decoded = percent_decode("..%2Fetc%2Fpasswd");
        assert_eq!(decoded, "../etc/passwd");
        assert!(!is_safe_asset_path(&decoded));

        let decoded = percent_decode("%2e%2e%2fetc%2fpasswd");
        assert_eq!(decoded, "../etc/passwd");
        assert!(!is_safe_asset_path(&decoded));

        let decoded = percent_decode("..%5Cetc%5Cpasswd");
        assert_eq!(decoded, "..\\etc\\passwd");
        assert!(!is_safe_asset_path(&decoded));
    }

    #[test]
    fn safe_asset_path_rejects_single_dot_and_null_byte() {
        assert!(!is_safe_asset_path("."));
        assert!(!is_safe_asset_path("assets/./app.js"));
        assert!(!is_safe_asset_path("assets/app.js\0.png"));
        // Pure-empty path has no real segment — refuse rather than
        // ambiguously resolving to dashboard root.
        assert!(!is_safe_asset_path(""));
        assert!(!is_safe_asset_path("/"));
    }

    #[test]
    fn safe_asset_path_rejects_unc_and_authority_prefix() {
        // Windows UNC (`\\server\share`) and protocol-relative authority
        // (`//server/share`) carry no `..` segment, so the per-segment guard
        // alone lets them through — but `Path::join` REPLACES the dashboard
        // base with the UNC/absolute path on Windows, escaping the directory.
        // The explicit leading-prefix reject closes that on every platform.
        for p in [
            "\\\\server\\share",
            "\\\\server\\share\\file.js",
            "//server/share",
            "//server/share/file.js",
            "\\\\?\\C:\\Windows\\System32",
        ] {
            assert!(!is_safe_asset_path(p), "expected {p:?} to be rejected");
        }
    }

    #[test]
    fn spa_route_matches_known_first_segments() {
        // Exact top-level routes.
        for p in ["agents", "config", "skills", "mcp-servers"] {
            assert!(is_spa_route(p), "expected {p:?} to be an SPA route");
        }
        // Nested dynamic / child routes match on the first segment.
        for p in [
            "agents/some-agent-id",
            "config/general",
            "config/security",
            "users/alice/budget",
            "/agents", // tolerate a leading slash defensively
        ] {
            assert!(is_spa_route(p), "expected {p:?} to be an SPA route");
        }
    }

    #[test]
    fn spa_route_rejects_unknown_first_segments() {
        // Phishing-style slugs and non-routes must NOT fall back to the shell.
        for p in [
            "security-alert",
            "login-here",
            "totally-made-up",
            "agentss", // near-miss, not an exact first-segment match
            "assetx",
        ] {
            assert!(!is_spa_route(p), "expected {p:?} to NOT be an SPA route");
        }
    }

    #[test]
    fn spa_routes_list_is_sorted_and_deduped() {
        // Keep the allowlist sorted + unique so review diffs stay clean and a
        // duplicate entry can't mask a typo.
        let mut sorted = SPA_ROUTES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            SPA_ROUTES,
            &sorted[..],
            "SPA_ROUTES must be sorted and free of duplicates"
        );
    }

    /// Minimal percent-decoder for the URL-decode regression tests. We
    /// don't want to pull `percent-encoding` into the test build just for
    /// `%XX` -> byte; this covers exactly what axum's extractor does for
    /// path captures (`%2e` -> `.`, `%2f` -> `/`, `%5c` -> `\`).
    fn percent_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8(out).expect("decoded UTF-8")
    }
}
