//! Dead-route audit (refs #3721, Phase 1).
//!
//! Catches a recurring class of bugs where a handler is added in
//! `crates/librefang-api/src/routes/*.rs` (and annotated with
//! `#[utoipa::path(...)]`, so it appears in the OpenAPI surface) but
//! the corresponding `.route(...)` registration in
//! `crates/librefang-api/src/server.rs` (or one of its sub-routers) is
//! forgotten. The handler is then unreachable at runtime even though
//! the spec advertises it.
//!
//! Strategy:
//! 1. Boot the real production router via `server::build_router()`.
//! 2. Iterate every path declared in `ApiDoc::openapi()`.
//! 3. For each path, fill any `{param}` segments with a synthetic
//!    placeholder and dispatch a `GET` request through the router via
//!    `tower::ServiceExt::oneshot`.
//! 4. Distinguish *router-level* 404 (path not registered) from
//!    *handler-level* 404 (handler ran and decided "agent not found")
//!    by inspecting the response. Axum's default fallback for an
//!    unmatched path returns `404` with `Content-Type: text/plain` and
//!    a literal body of `"Not Found"`. Every real handler in the
//!    codebase returns JSON when it produces a 404 (`ApiErrorResponse`
//!    and `application/json`). Only a `text/plain` "Not Found" counts as
//!    a dead route. Anything else — `200`, `204`, `400`, `401`, `403`,
//!    `405`, JSON `404`, `415`, `422`, `5xx`, … — means the route is
//!    wired and the handler ran.
//!
//! This is the automated replacement for Steps 4-6 of the legacy
//! "Live Integration Testing" curl checklist that lived in CLAUDE.md.
//! Phase 2 (payload smoke against TestServer for hot-path endpoints)
//! and Phase 3 (live LLM metering side-effect verification) are
//! tracked as follow-ups under #3721.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::openapi::ApiDoc;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::collections::BTreeSet;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;
use utoipa::OpenApi;

/// Substitute for `{param}` segments in OpenAPI path templates. Chosen to
/// be a simple ASCII identifier so it satisfies axum's path matchers
/// (which accept any non-`/` segment by default) and is unlikely to be
/// confused with a real entity ID.
const PATH_PLACEHOLDER: &str = "_audit_placeholder";

/// Paths the audit must skip. Each entry needs an explicit reason — we
/// do **not** want this list to absorb genuine bugs. Keep it tiny.
fn skip_paths() -> BTreeSet<&'static str> {
    BTreeSet::from([
        // OpenAPI spec endpoint itself is mounted by `build_router` via a
        // dedicated `.merge(SwaggerUi::...)` call rather than a single
        // `.route()`. Hitting it would still return 200, so it is harmless
        // either way; listed here purely for documentation completeness.
        // (No actual skip needed, but keep the set machinery in place so
        // future additions have a clear precedent.)
    ])
}

/// Boot a full production router on top of an in-memory tempdir-backed
/// kernel. Mirrors the `start_full_router` helper used elsewhere in
/// `tests/api_integration_test.rs` but kept self-contained so this file
/// can compile independently.
async fn boot_full_router() -> (Router, Arc<AppState>, TempDir) {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    // Populate the model registry so the kernel boots without warnings.
    librefang_runtime::registry_sync::sync_registry(
        tmp.path(),
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        // Empty api_key disables auth (`is_public` allowlist still
        // applies, but most routes accept the request without auth at
        // all when no key is configured). This keeps the audit focused
        // on routing, not authentication — a 401 from a configured-key
        // run would still pass the "not 404" assertion, but skipping
        // auth here makes the failure mode singular and obvious.
        api_key: String::new(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    (app, state, tmp)
}

/// Replace every `{name}` segment in a path template with the audit
/// placeholder. The placeholder is the same for every parameter — we
/// only care that the segment matches axum's matcher, not that the
/// downstream handler can find a real entity.
fn substitute_path_params(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // Skip until matching '}'
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }
            out.push_str(PATH_PLACEHOLDER);
        } else {
            out.push(c);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn dead_route_audit_every_openapi_path_is_registered_in_router() {
    let (app, state, _tmp) = boot_full_router().await;

    let spec = ApiDoc::openapi();
    let spec_json = spec
        .to_json()
        .expect("OpenAPI spec must serialize for the audit to enumerate it");
    let parsed: serde_json::Value =
        serde_json::from_str(&spec_json).expect("OpenAPI spec must be valid JSON");

    let paths = parsed["paths"]
        .as_object()
        .expect("OpenAPI spec must declare a `paths` object");

    let skip = skip_paths();
    let mut missing: Vec<String> = Vec::new();
    let mut audited: usize = 0;

    for (template, _ops) in paths {
        if skip.contains(template.as_str()) {
            continue;
        }

        let request_path = substitute_path_params(template);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&request_path)
                    .body(Body::empty())
                    .expect("synthetic audit request should build"),
            )
            .await
            .expect("router oneshot must not panic");

        audited += 1;
        let status = response.status();

        if status != StatusCode::NOT_FOUND {
            // Route is wired and the handler ran (or another tower
            // layer such as auth / body-limit returned an error).
            // Either way it is reachable — not a dead route.
            continue;
        }

        // Distinguish router-fallback 404 from handler 404. Axum's
        // default `not_found` service returns `text/plain` + the
        // literal body "Not Found". Real handlers that return 404
        // (e.g. "agent not found") use `ApiErrorResponse` which is
        // serialized as `application/json`.
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap_or_default();
        let body_str = String::from_utf8_lossy(&body_bytes);

        let looks_like_router_fallback = content_type.starts_with("text/plain")
            && body_str.trim().eq_ignore_ascii_case("not found");

        if looks_like_router_fallback {
            missing.push(template.clone());
        }
    }

    // Cleanup before assertion so a failure does not leak the kernel.
    state.kernel.shutdown();

    assert!(
        audited > 100,
        "expected to audit 100+ OpenAPI paths, only saw {audited} — \
         either the spec regressed or the audit logic broke"
    );

    assert!(
        missing.is_empty(),
        "Dead-route audit found {} OpenAPI path(s) that returned the axum \
         router fallback (`404 Not Found`, `text/plain`). Each entry below \
         is declared via `#[utoipa::path]` on a handler in \
         `crates/librefang-api/src/routes/` but is missing a matching \
         `.route(...)` registration in `crates/librefang-api/src/server.rs` \
         (or one of the sub-routers it merges). Add the registration or, \
         if the path was retired, remove the `#[utoipa::path]` annotation.\n\n{:#?}",
        missing.len(),
        missing,
    );
}

#[test]
fn substitute_path_params_replaces_every_placeholder() {
    assert_eq!(
        substitute_path_params("/api/agents/{id}/sessions/{session_id}/trajectory"),
        format!(
            "/api/agents/{p}/sessions/{p}/trajectory",
            p = PATH_PLACEHOLDER
        ),
    );
    assert_eq!(substitute_path_params("/api/health"), "/api/health");
    assert_eq!(
        substitute_path_params("/api/tools/{name}"),
        format!("/api/tools/{}", PATH_PLACEHOLDER),
    );
}
