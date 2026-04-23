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
