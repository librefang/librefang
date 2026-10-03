//! The gate on the dashboard shell is the session, not the path.
//!
//! `/dashboard/agents` answered the login page without a session while `/` answered the full SPA shell, because `/` was `PublicRoute::exact_any("/")` in `PUBLIC_ROUTES_ALWAYS` — a table consulted before the shell rule ever runs.
//! Whichever URL the browser arrives by, the same rule has to apply.
//!
//! That rule keys on whether a login screen exists to show at all.
//! With only an `api_key` configured there is none — `login_page.html` speaks username and password and posts to `/api/auth/dashboard-login` — so the shell stays public and the SPA renders its own API-key entry.
//! That case is #2305, and the second test here pins it, because closing `/` unconditionally would lock such an operator out with nothing to type credentials into.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

/// The `<title>` of `login_page.html`, and of nothing else the daemon serves at these paths.
/// The SPA shell and the "dashboard still syncing" placeholder are both titled plain `LibreFang`, so this tells a login page apart from either.
const LOGIN_PAGE_MARKER: &str = "LibreFang — Sign in";

/// Requires auth unconditionally.
/// A 401 here proves auth really is configured, so a 200 at `/` in the same fixture is a deliberate exemption rather than a daemon that simply has no credentials set.
const AUTHED_PATH: &str = "/api/status";

fn base_config(home: &std::path::Path) -> KernelConfig {
    KernelConfig {
        home_dir: home.to_path_buf(),
        data_dir: home.join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    }
}

async fn boot(config: KernelConfig) -> axum::Router {
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    server::build_router(kernel, "127.0.0.1:0".parse().expect("addr"))
        .await
        .0
}

fn home_fixture(home: &std::path::Path) {
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(home);
    std::fs::create_dir_all(home.join("data")).expect("data dir");
}

/// Status, content type and body of an unauthenticated GET.
/// All three matter: a login page and a dashboard shell are both `text/html`, so status alone cannot tell them apart.
async fn anonymous_get(app: axum::Router, path: &str) -> (StatusCode, String, String) {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .expect("request");
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:54321".parse::<SocketAddr>().expect("peer"),
    ));

    let response = app.oneshot(req).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// With a dashboard password configured, every entry to the shell answers the login page.
/// `/` used to answer the SPA instead, which is the whole bug: the gate was on the path, so entering by the root walked around it.
#[tokio::test(flavor = "multi_thread")]
async fn every_shell_entry_answers_the_login_page_without_a_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    home_fixture(tmp.path());

    let app = boot(KernelConfig {
        dashboard_user: "operator".to_string(),
        dashboard_pass: "correct-horse-battery-staple".to_string(),
        ..base_config(tmp.path())
    })
    .await;

    // `/dashboard/agents` was already correct and is the reference behaviour; `/` is the one that got around it.
    // Asserting both in one test is what makes "the gate is the session, not the route" a single property rather than two independent facts free to drift apart again.
    for path in ["/", "/dashboard/agents"] {
        let (status, content_type, body) = anonymous_get(app.clone(), path).await;

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "GET {path} without a session must not be served"
        );
        assert!(
            content_type.starts_with("text/html"),
            "GET {path} must answer the browser with HTML, got content-type {content_type:?}"
        );
        assert!(
            body.contains(LOGIN_PAGE_MARKER),
            "GET {path} must answer the login page; body was {} bytes starting {:?}",
            body.len(),
            body.chars().take(200).collect::<String>()
        );
    }
}

/// RFC 6265 §5.1.4 path-match: does a cookie scoped to `cookie_path` ride along on a request for `request_path`?
///
/// The rule is "the cookie-path is a prefix of the request-path, and either the cookie-path ends in `/` or the first character of the remainder is `/`".
/// `/dashboard` is not a prefix of `/` — which is the whole bug — while `/` is a prefix of every path.
fn cookie_path_covers(cookie_path: &str, request_path: &str) -> bool {
    match request_path.strip_prefix(cookie_path) {
        Some(rest) => cookie_path.ends_with('/') || rest.starts_with('/'),
        None => false,
    }
}

/// The `Path` the login cookie was issued with, and the token to send it back.
fn session_cookie(login: &axum::http::HeaderMap) -> (String, String) {
    let raw = login
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("librefang_session="))
        .expect("login must issue a librefang_session cookie");
    let path = raw
        .split(';')
        .map(str::trim)
        .find_map(|attr| attr.strip_prefix("Path="))
        .expect("the session cookie must declare a Path")
        .to_string();
    let token = raw
        .split(';')
        .next()
        .and_then(|kv| kv.trim().strip_prefix("librefang_session="))
        .expect("the first attribute is the name=value pair")
        .to_string();
    (path, token)
}

/// POST `path` with a JSON body, as the login form does.
async fn post_json(app: axum::Router, path: &str, body: &str) -> axum::response::Response {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:54321".parse::<SocketAddr>().expect("peer"),
    ));
    app.oneshot(req).await.expect("response")
}

/// GET `path` carrying the session cookie a browser would attach.
async fn get_with_cookie(
    app: axum::Router,
    path: &str,
    token: &str,
) -> (StatusCode, String, String) {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::COOKIE, format!("librefang_session={token}"))
        .body(Body::empty())
        .expect("request");
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:54321".parse::<SocketAddr>().expect("peer"),
    ));

    let response = app.oneshot(req).await.expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// The cookie the login issues is sent to the URL the login page navigates to.
///
/// The inline login page ends a successful sign-in by replacing its location with the SPA shell at `/`, and `/` is a gated entry of the shell.
/// The session it just established is therefore only usable if the browser attaches the cookie to that next request.
/// A cookie scoped to `/dashboard` is not attached to `/`, so the gate answers the login page again and the operator loops: the same form, a 200, the same form.
/// That is the shape of the report this test comes from — repeated `POST /api/auth/dashboard-login` 200s next to repeated `GET /` 401s, and not one request for a dashboard asset.
///
/// The assertion is on the scope, not on the 200 that follows.
/// A raw HTTP client sends a cookie whatever its `Path` says — only a browser applies the rule — so the second half passes on the broken build too and cannot be the custody; it is here to show the cookie the login actually issued does authenticate `/`, which makes the scope the only thing that stood between the two.
#[tokio::test(flavor = "multi_thread")]
async fn the_login_cookie_is_scoped_to_the_url_the_login_redirects_to() {
    let tmp = tempfile::tempdir().expect("tempdir");
    home_fixture(tmp.path());

    let app = boot(KernelConfig {
        dashboard_user: "operator".to_string(),
        dashboard_pass: "correct-horse-battery-staple".to_string(),
        ..base_config(tmp.path())
    })
    .await;

    let login = post_json(
        app.clone(),
        "/api/auth/dashboard-login",
        r#"{"username":"operator","password":"correct-horse-battery-staple"}"#,
    )
    .await;
    assert_eq!(
        login.status(),
        StatusCode::OK,
        "fixture is meaningless unless the credentials actually check out"
    );

    let (path, token) = session_cookie(login.headers());
    assert!(
        cookie_path_covers(&path, "/"),
        "the login page navigates to `/`, so a cookie scoped to {path:?} is never sent \
         with that request and the shell gate answers the login page again (#8279 + #2785)"
    );

    let (status, content_type, body) = get_with_cookie(app.clone(), "/", &token).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the session the login just issued must authenticate the redirect target"
    );
    assert!(
        content_type.starts_with("text/html"),
        "GET / must serve the shell, got content-type {content_type:?}"
    );
    assert!(
        !body.contains(LOGIN_PAGE_MARKER),
        "GET / must serve the shell, not the login page the operator just completed"
    );
}

/// #2305: an `api_key`-only deployment has no login screen to show, so the shell stays public and the SPA collects the key itself.
/// The `/api/status` assertion is what stops this from passing vacuously against a daemon with no auth configured at all.
#[tokio::test(flavor = "multi_thread")]
async fn the_shell_stays_public_when_no_dashboard_password_is_configured() {
    let tmp = tempfile::tempdir().expect("tempdir");
    home_fixture(tmp.path());

    let app = boot(KernelConfig {
        api_key: "test-secret-key".to_string(),
        ..base_config(tmp.path())
    })
    .await;

    let (status, _, _) = anonymous_get(app.clone(), AUTHED_PATH).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "fixture is meaningless unless auth is actually configured"
    );

    let (status, content_type, body) = anonymous_get(app.clone(), "/").await;
    assert_eq!(status, StatusCode::OK, "GET / must still serve the shell");
    assert!(
        content_type.starts_with("text/html"),
        "GET / must serve HTML, got content-type {content_type:?}"
    );
    assert!(
        !body.contains(LOGIN_PAGE_MARKER),
        "GET / must serve the shell, not a login page nobody can complete"
    );
}
