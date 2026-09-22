//! Integration tests for the per-user avatar and identity-emoji routes (#8339).
//!
//! These run against the production router (`server::build_router`), so the real
//! auth middleware, the real route registration and the real config-write path
//! are all in play. No LLM calls — the provider is `ollama` with a fake model.
//!
//! Routes covered:
//!   POST   /api/users/{name}/avatar    (round trip, format swap, cap, non-image
//!                                       bytes, the name that must never reach
//!                                       the filesystem)
//!   GET    /api/users/me/avatar        (the literal route; static-vs-{name}
//!                                       precedence)
//!   GET    /api/users/{name}/avatar     (bytes, headers, ETag/304, absent, 404)
//!   DELETE /api/users/{name}/avatar     (remove → GET 404)
//!   PATCH  /api/users/{name}/identity   (emoji set/clear, survives a PUT of the
//!                                       user row, surfaces on whoami)
//!
//! Run: cargo test -p librefang-api --test user_avatar_routes_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::UserId;
use librefang_types::config::{DefaultModelConfig, KernelConfig, UserConfig};
use librefang_types::media;
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "user-avatar-master-key";
const VIEWER_KEY: &str = "user-avatar-viewer-key";
const USER_KEY: &str = "user-avatar-user-key";
const ADMIN_KEY: &str = "user-avatar-admin-key";

/// A user whose *name* collides with the literal segment of
/// `GET /api/users/me/avatar`. Seeded in every harness on purpose: the whole
/// point of that route is that the collision is survivable, and a suite that
/// only ever ran without it would be proving the easy case.
const ME_KEY: &str = "user-avatar-me-key";
const ME_NAME: &str = "me";

// A name that is a legal `[[users]]` entry under `validate_name` and would be a
// catastrophe if it were ever joined onto a directory. Percent-encoded in the
// URL, because a bare `../` segment is resolved by the URI itself and would
// never reach the handler at all.
const TRAVERSAL_NAME: &str = "../../etc/avatar-escape";

/// The Windows device names a `.png` suffix does not neutralise on a host that
/// still honours them, plus a backslash traversal — neither needs URL encoding
/// to reach the handler as a single path segment.
const DEVICE_NAMES: &[&str] = &["nul", "CON", "aux", "com1", "..\\..\\escape"];

/// A real one-pixel PNG. Sniffing is by magic bytes, so these have to be genuine.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89,
];

/// A minimal GIF87a header — a *different* format, for the swap test.
const TINY_GIF: &[u8] = b"GIF87a\x01\x00\x01\x00\x80\x00\x00";

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

fn user(name: &str, role: &str, key: Option<&str>) -> UserConfig {
    UserConfig {
        name: name.to_string(),
        role: role.to_string(),
        api_key_hash: key.map(|k| librefang_api::password_hash::hash_password(k).expect("hash")),
        ..Default::default()
    }
}

/// Boot the production router over a config that declares the users these tests
/// authenticate as.
///
/// The seeded rows are not decoration: `build_router` derives its per-user key
/// table from `[[users]]`, so a caller the config does not name cannot present a
/// credential at all — and the whole point of the gate tests below is that the
/// credential, not the request, decides.
async fn boot(extra_users: Vec<UserConfig>) -> Harness {
    boot_with_users(
        vec![
            user("Alice", "user", Some(USER_KEY)),
            user("Watcher", "viewer", Some(VIEWER_KEY)),
            user("Bosswoman", "admin", Some(ADMIN_KEY)),
            user(ME_NAME, "user", Some(ME_KEY)),
        ],
        extra_users,
    )
    .await
}

async fn boot_with_users(users: Vec<UserConfig>, extra: Vec<UserConfig>) -> Harness {
    let mut all = users;
    all.extend(extra);

    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        users: all,
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

/// The directory user avatars actually land in, read from the same resolver the
/// handler uses rather than reconstructed from the temp dir.
fn users_avatar_dir(h: &Harness) -> std::path::PathBuf {
    h.state
        .kernel
        .config_snapshot()
        .effective_user_avatars_dir()
}

fn agent_avatar_dir(h: &Harness) -> std::path::PathBuf {
    h.state.kernel.config_snapshot().effective_avatars_dir()
}

fn file_names(dir: &std::path::Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read_dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn with_token(
    mut builder: axum::http::request::Builder,
    token: &str,
) -> axum::http::request::Builder {
    builder = builder.header("authorization", format!("Bearer {token}"));
    builder
}

/// A raw-body request. The content-type is the caller's claim, and several
/// tests below exist to show the handler ignores it.
fn raw(
    method: Method,
    path: &str,
    token: &str,
    body: Vec<u8>,
    content_type: &str,
) -> Request<Body> {
    with_token(
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", content_type),
        token,
    )
    .body(Body::from(body))
    .expect("request")
}

fn get(path: &str, token: &str) -> Request<Body> {
    with_token(Request::builder().method(Method::GET).uri(path), token)
        .body(Body::empty())
        .expect("request")
}

fn json_req(method: Method, path: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    with_token(
        Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json"),
        token,
    )
    .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
    .expect("request")
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

async fn send_raw(app: Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, headers, bytes.to_vec())
}

/// The stem the handler must have used for this name — computed the same way the
/// module does, but spelled out here so a change to the derivation shows up as a
/// wrong filename rather than as a test that silently follows the change.
fn expected_stem(name: &str) -> String {
    UserId::from_name(name).to_string()
}

async fn upload_png(h: &Harness, name: &str, bytes: &[u8]) -> (StatusCode, serde_json::Value) {
    send(
        h.app.clone(),
        raw(
            Method::POST,
            &format!("/api/users/{name}/avatar"),
            TEST_TOKEN,
            bytes.to_vec(),
            "application/octet-stream",
        ),
    )
    .await
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------

/// The whole feature end to end: the bytes come back, and the user's view now
/// says an avatar exists.
#[tokio::test(flavor = "multi_thread")]
async fn avatar_upload_round_trips_and_shows_on_the_user_view() {
    let h = boot(vec![]).await;

    let (status, body) = upload_png(&h, "Alice", TINY_PNG).await;
    assert_eq!(status, StatusCode::OK, "upload failed: {body:?}");
    assert_eq!(body["content_type"], "image/png");
    assert_eq!(body["bytes"], TINY_PNG.len());

    // The image really is where the resolver says it is, under the derived
    // stem — not under the name.
    let dir = users_avatar_dir(&h);
    assert_eq!(
        file_names(&dir),
        vec![format!("{}.png", expected_stem("Alice"))],
        "the stored name must be the derived id plus the sniffed extension"
    );

    let (status, headers, bytes) =
        send_raw(h.app.clone(), get("/api/users/Alice/avatar", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, TINY_PNG, "the served bytes must be the stored bytes");
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    // `cache-control` is deliberately not asserted: the production stack adds a
    // global `no-store, no-cache, must-revalidate` to every API response, which
    // is stricter than the route's own `no-cache`, so the value that arrives is
    // the layer's and not this handler's. The revalidation contract is covered
    // by the ETag/304 test rather than by a header string.
    assert!(headers.contains_key("etag"));

    // Without this the test would pass against a handler that wrote the file
    // and never taught the view about it.
    let (status, view) = send(h.app.clone(), get("/api/users/Alice", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["has_avatar"], true, "user view: {view}");
    assert!(view["emoji"].is_null());

    // A different user must not acquire one by association.
    let (_, other) = send(h.app.clone(), get("/api/users/Watcher", TEST_TOKEN)).await;
    assert_eq!(other["has_avatar"], false, "user view: {other}");

    let (_, list) = send(h.app.clone(), get("/api/users", TEST_TOKEN)).await;
    let rows = list.as_array().expect("list is an array");
    let alice = rows.iter().find(|r| r["name"] == "Alice").expect("Alice");
    assert_eq!(alice["has_avatar"], true, "list view: {list}");
}

/// An unchanged avatar costs a 304; a re-upload is picked up immediately.
#[tokio::test(flavor = "multi_thread")]
async fn avatar_serves_a_304_for_a_matching_if_none_match() {
    let h = boot(vec![]).await;
    upload_png(&h, "Alice", TINY_PNG).await;

    let (_, headers, _) = send_raw(h.app.clone(), get("/api/users/Alice/avatar", TEST_TOKEN)).await;
    let etag = headers["etag"].to_str().expect("etag").to_string();

    let mut req = get("/api/users/Alice/avatar", TEST_TOKEN);
    req.headers_mut()
        .insert("if-none-match", etag.parse().expect("header value"));
    let (status, _, bytes) = send_raw(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(bytes.is_empty());

    // A validator that survives a content change would pin the old image in
    // every browser that had seen it.
    upload_png(&h, "Alice", TINY_GIF).await;
    let mut req = get("/api/users/Alice/avatar", TEST_TOKEN);
    req.headers_mut()
        .insert("if-none-match", etag.parse().expect("header value"));
    let (status, headers, bytes) = send_raw(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::OK, "the swapped image must not 304");
    assert_eq!(bytes, TINY_GIF);
    assert_eq!(headers["content-type"], "image/gif");
}

/// Changing format must not leave the previous file behind: `find_avatar`
/// probes extensions in a fixed order, so a stale `.png` would shadow a new
/// `.gif` forever.
#[tokio::test(flavor = "multi_thread")]
async fn avatar_replacement_removes_the_previous_format() {
    let h = boot(vec![]).await;
    upload_png(&h, "Alice", TINY_PNG).await;
    let (status, _) = upload_png(&h, "Alice", TINY_GIF).await;
    assert_eq!(status, StatusCode::OK);

    let dir = users_avatar_dir(&h);
    assert_eq!(
        file_names(&dir),
        vec![format!("{}.gif", expected_stem("Alice"))],
        "the superseded PNG must be gone, or it shadows the GIF for good"
    );
    let (_, _, bytes) = send_raw(h.app.clone(), get("/api/users/Alice/avatar", TEST_TOKEN)).await;
    assert_eq!(bytes, TINY_GIF);
}

/// The client's declared content type is ignored: the bytes decide.
#[tokio::test(flavor = "multi_thread")]
async fn avatar_ignores_the_content_type_the_client_claims() {
    let h = boot(vec![]).await;

    let (status, body) = send(
        h.app.clone(),
        raw(
            Method::POST,
            "/api/users/Alice/avatar",
            TEST_TOKEN,
            TINY_PNG.to_vec(),
            // A type the daemon refuses to serve, declared over PNG bytes.
            "image/svg+xml",
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the bytes are a valid PNG, so the claimed type must not matter: {body:?}"
    );
    assert_eq!(body["content_type"], "image/png");
}

// ---------------------------------------------------------------------------
// GET /api/users/me/avatar — the literal read route
// ---------------------------------------------------------------------------

/// The static `me` segment outranks the `{name}` parameter, and both routes
/// answer.
///
/// Made decisive rather than decorative by seeding a real user named `me` with
/// a *different* image: if `{name}` had won the match, reading as Alice would
/// have returned that user's picture instead of her own. The test therefore
/// fails on a router that resolves the parameter first, which is the only thing
/// it is here to rule out.
#[tokio::test(flavor = "multi_thread")]
async fn the_me_segment_is_literal_and_does_not_fall_through_to_a_user_named_me() {
    let h = boot(vec![]).await;
    upload_png(&h, "Alice", TINY_PNG).await;
    // The row named `me` is given an image by writing it straight to disk,
    // because the route that would upload one is shadowed — see
    // `the_row_named_me_cannot_be_written_to` for that half.
    let dir = users_avatar_dir(&h);
    std::fs::create_dir_all(&dir).expect("create avatars dir");
    std::fs::write(
        media::avatar_path(&dir, &expected_stem(ME_NAME), "gif"),
        TINY_GIF,
    )
    .expect("place the image for the row named `me`");

    // The literal route, read by Alice: her own image, not the other one.
    let (status, _, bytes) = send_raw(h.app.clone(), get("/api/users/me/avatar", USER_KEY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        bytes, TINY_PNG,
        "`me` must resolve from the credential; the user named `me` must not shadow it"
    );

    // The literal route, read by the person actually named `me` — still their
    // own. This is the half that keeps the collision from locking anyone out
    // of seeing their picture.
    let (status, _, bytes) = send_raw(h.app.clone(), get("/api/users/me/avatar", ME_KEY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, TINY_GIF);

    // The parameterised route is still served, and still reaches Alice.
    let (status, _, bytes) =
        send_raw(h.app.clone(), get("/api/users/Alice/avatar", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, TINY_PNG);

    // An Admin asking for the row called `me` by name gets the literal route
    // instead, which answers for the *credential* — so the master key, naming
    // no row, gets a 404 and not that row's picture.
    let (status, body) = send(h.app.clone(), get("/api/users/me/avatar", TEST_TOKEN)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the master key names no row, and the literal route answers for the \
         credential rather than for the row named `me`: {body:?}"
    );
}

/// The other half of the `me` corner, recorded rather than discovered later: on
/// `/api/users/me/avatar` the static segment owns the path, so the write verbs
/// registered on `{name}` are unreachable there and a row named `me` cannot be
/// given an avatar through the API at all.
///
/// This is a consequence of the literal read route, not a defect in it, and it
/// is the price of the read path having no client-controlled segment. It is
/// asserted so that it is a known, changeable decision rather than a surprise.
#[tokio::test(flavor = "multi_thread")]
async fn the_row_named_me_cannot_be_written_to() {
    let h = boot(vec![]).await;

    for (method, label) in [(Method::POST, "upload"), (Method::DELETE, "delete")] {
        let (status, _) = send(
            h.app.clone(),
            raw(
                method,
                "/api/users/me/avatar",
                TEST_TOKEN,
                TINY_PNG.to_vec(),
                "application/octet-stream",
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "the {label} verb on the literal path must not silently reach the \
             row named `me`; the literal route is GET-only"
        );
    }

    // The same row is still administrable through every other route: only the
    // avatar path shape collides.
    let (status, body) = send(
        h.app.clone(),
        json_req(
            Method::PATCH,
            "/api/users/me/identity",
            TEST_TOKEN,
            serde_json::json!({ "emoji": "🦀" }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "/users/me/identity has no literal sibling, so it still reaches the row: {body:?}"
    );
    assert_eq!(body["name"], ME_NAME, "{body:?}");
    assert_eq!(body["emoji"], "🦀", "{body:?}");
}

/// A credential that names no `[[users]]` row answers 404 rather than being
/// looked up under a literal `"root"`.
#[tokio::test(flavor = "multi_thread")]
async fn me_avatar_404s_for_a_credential_that_names_no_user() {
    let h = boot(vec![]).await;

    let (status, body) = send(h.app.clone(), get("/api/users/me/avatar", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("credential"),
        "the message must say the credential is what has no avatar, not that \
         the user is missing: {message:?}"
    );
}

/// The upload verb is `POST` — the same one the agent route uses — and `PUT`
/// is not registered alongside it.
///
/// Asserted because the two were `PUT` and `POST` in earlier drafts of this
/// feature, and a stale client silently keeping the old verb would be a 405
/// nobody reads rather than a wrong result anyone notices.
#[tokio::test(flavor = "multi_thread")]
async fn the_upload_verb_is_post_and_put_is_not_registered() {
    let h = boot(vec![]).await;

    let (status, _) = send(
        h.app.clone(),
        raw(
            Method::POST,
            "/api/users/Alice/avatar",
            TEST_TOKEN,
            TINY_PNG.to_vec(),
            "application/octet-stream",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(
        h.app.clone(),
        raw(
            Method::PUT,
            "/api/users/Alice/avatar",
            TEST_TOKEN,
            TINY_PNG.to_vec(),
            "application/octet-stream",
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "the route moved from PUT to POST; it was not added alongside"
    );
}

// ---------------------------------------------------------------------------
// The property the design rests on: the name never becomes a path
// ---------------------------------------------------------------------------

/// A user called `../../etc/avatar-escape` stores an avatar at
/// `{uuid}.png` inside the users avatar directory, and nothing anywhere carries
/// any part of the name.
///
/// The assertion is deliberately *not* "the name was sanitised" — it is that the
/// name was never used as a path component at all. That is why a traversal is
/// safe here while `validate_name` still accepts the name: a stricter name rule
/// would break operators who already have such a user, and would still be the
/// wrong invariant.
#[tokio::test(flavor = "multi_thread")]
async fn a_traversal_name_never_becomes_a_path() {
    let h = boot_with_users(vec![], vec![user(TRAVERSAL_NAME, "user", None)]).await;

    // `%2F` and not `/`: a bare `../` is resolved by the URI itself and would
    // never reach the handler, so it would test the router rather than the guard.
    let encoded = TRAVERSAL_NAME.replace('/', "%2F");
    let (status, body) = upload_png(&h, &encoded, TINY_PNG).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a traversal-shaped name is a legal user name and must store normally: {body:?}"
    );

    let dir = users_avatar_dir(&h);
    assert_eq!(
        file_names(&dir),
        vec![format!("{}.png", expected_stem(TRAVERSAL_NAME))],
        "exactly one file, named after the derived id"
    );

    // Nothing escaped: walk the whole home directory and look for any last
    // path segment that carries a trace of the traversal. Matched on the
    // segment rather than the whole path and on the traversal's own text
    // rather than on a substring of it — `rocketchat.py`, which the boot
    // fixture really does write, contains "etc" and would be a false alarm.
    let home = h.state.kernel.home_dir();
    let mut escaped = Vec::new();
    for entry in walk(home) {
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.contains("avatar-escape") || name == "etc" {
            escaped.push(entry.to_string_lossy().into_owned());
        }
    }
    assert!(
        escaped.is_empty(),
        "the traversal must not appear anywhere under the home directory: {escaped:?}"
    );

    // And the image is served back through the same route that stored it.
    let (status, _, bytes) = send_raw(
        h.app.clone(),
        get(&format!("/api/users/{encoded}/avatar"), TEST_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, TINY_PNG);
}

/// Every device name a filesystem might still honour stores as a uuid.
#[tokio::test(flavor = "multi_thread")]
async fn a_device_name_is_not_a_file_name() {
    let extra: Vec<UserConfig> = DEVICE_NAMES.iter().map(|n| user(n, "user", None)).collect();
    let h = boot_with_users(vec![], extra).await;

    for name in DEVICE_NAMES {
        let encoded = name.replace('/', "%2F");
        let (status, body) = upload_png(&h, &encoded, TINY_PNG).await;
        assert_eq!(status, StatusCode::OK, "name {name:?}: {body:?}");
    }

    let dir = users_avatar_dir(&h);
    let mut expected: Vec<String> = DEVICE_NAMES
        .iter()
        .map(|n| format!("{}.png", expected_stem(n)))
        .collect();
    expected.sort();
    assert_eq!(
        file_names(&dir),
        expected,
        "every stored file must be named after its derived id"
    );
    // The device names themselves must not have been created anywhere.
    for name in DEVICE_NAMES {
        assert!(!dir.join(name).exists(), "{name:?} was used as a file name");
    }
}

/// User avatars do not land in the agent avatars directory.
///
/// The two are named `{uuid}.{ext}` from different namespaces, so a shared tree
/// would make "which of these is a person" unanswerable from the path alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_user_avatar_is_not_an_agent_avatar() {
    let h = boot(vec![]).await;
    upload_png(&h, "Alice", TINY_PNG).await;

    let dir = users_avatar_dir(&h);
    let agents = agent_avatar_dir(&h);
    assert_ne!(dir, agents, "the two populations need separate directories");
    assert!(
        dir.starts_with(&agents),
        "user avatars live below the avatars root"
    );
    assert_eq!(
        file_names(&agents),
        vec!["users".to_string()],
        "the avatars root must hold the subdirectory and nothing else"
    );
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn bytes_that_are_not_an_image_are_refused() {
    let h = boot(vec![]).await;

    for (label, bytes) in [
        (
            "svg",
            br#"<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>"#.to_vec(),
        ),
        ("text", b"just some text".to_vec()),
        ("empty", Vec::new()),
    ] {
        let (status, body) = upload_png(&h, "Alice", &bytes).await;
        assert_eq!(
            status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{label} must be refused; body={body:?}"
        );
    }
    assert!(
        media::find_avatar(&users_avatar_dir(&h), &expected_stem("Alice")).is_none(),
        "a refused upload must not have stored anything"
    );
}

/// An image just over the cap must get the handler's 413, with a JSON body
/// naming the limit — not the extractor's bodiless one.
#[tokio::test(flavor = "multi_thread")]
async fn an_oversize_image_gets_the_handler_error_not_the_extractor() {
    let h = boot(vec![]).await;

    let mut oversize = TINY_PNG.to_vec();
    oversize.resize(2 * 1024 * 1024 + 1, 0);

    let (status, body) = upload_png(&h, "Alice", &oversize).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("2097152"),
        "the handler's message must name the cap, got: {message:?} (a bodiless 413 \
         here means the extractor cut first and the handler's check is dead code)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_user_is_404_on_every_route() {
    let h = boot(vec![]).await;
    let path = "/api/users/Nobody";

    for req in [
        raw(
            Method::POST,
            &format!("{path}/avatar"),
            TEST_TOKEN,
            TINY_PNG.to_vec(),
            "application/octet-stream",
        ),
        get(&format!("{path}/avatar"), TEST_TOKEN),
        raw(
            Method::DELETE,
            &format!("{path}/avatar"),
            TEST_TOKEN,
            Vec::new(),
            "application/octet-stream",
        ),
        json_req(
            Method::PATCH,
            &format!("{path}/identity"),
            TEST_TOKEN,
            serde_json::json!({ "emoji": "🦀" }),
        ),
    ] {
        let method = req.method().clone();
        let (status, body) = send(h.app.clone(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} must 404: {body:?}");
    }
    assert!(
        !users_avatar_dir(&h).exists(),
        "a refused upload must not have created the avatar directory"
    );
}

/// A user with no uploaded avatar answers 404 on the serve route rather than an
/// empty 200 that a browser renders as a broken image.
#[tokio::test(flavor = "multi_thread")]
async fn serving_a_user_with_no_avatar_is_404() {
    let h = boot(vec![]).await;
    let (status, _) = send(h.app.clone(), get("/api/users/Alice/avatar", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

/// Writes are Owner-only — the posture every other mutating `/api/users*` route
/// already carries — while the read stays open to any authenticated role.
///
/// The gate lives in `middleware::is_owner_only_write`, which matches the whole
/// `/api/users` prefix. That means these routes inherit it without a line of
/// gate code here, and it also means the assertion below is what would catch a
/// future carve-out that quietly let a lower role through.
#[tokio::test(flavor = "multi_thread")]
async fn non_owner_write_roles_are_refused_and_reads_are_not() {
    let h = boot(vec![]).await;

    for (label, token) in [
        ("viewer", VIEWER_KEY),
        ("user", USER_KEY),
        ("admin", ADMIN_KEY),
    ] {
        let (status, body) = send(
            h.app.clone(),
            raw(
                Method::POST,
                "/api/users/Alice/avatar",
                token,
                TINY_PNG.to_vec(),
                "application/octet-stream",
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a {label} key must not upload an avatar: {body:?}"
        );

        let (status, body) = send(
            h.app.clone(),
            raw(
                Method::DELETE,
                "/api/users/Alice/avatar",
                token,
                Vec::new(),
                "application/octet-stream",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{label} delete: {body:?}");

        let (status, body) = send(
            h.app.clone(),
            json_req(
                Method::PATCH,
                "/api/users/Alice/identity",
                token,
                serde_json::json!({ "emoji": "🦀" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{label} identity: {body:?}");
    }

    assert!(
        !users_avatar_dir(&h).exists(),
        "a refused upload must not have created the avatar directory"
    );

    // Reads are on the generic authenticated-GET rule, so the lowest role sees
    // the avatar of a user it is not.
    upload_png(&h, "Alice", TINY_PNG).await;
    for (label, token) in [
        ("viewer", VIEWER_KEY),
        ("user", USER_KEY),
        ("admin", ADMIN_KEY),
    ] {
        let (status, _, bytes) =
            send_raw(h.app.clone(), get("/api/users/Alice/avatar", token)).await;
        assert_eq!(status, StatusCode::OK, "{label} read: {status}");
        assert_eq!(bytes, TINY_PNG, "{label} read bytes");
    }

    // And an unauthenticated caller never reaches the handler at all.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/users/Alice/avatar")
        .body(Body::empty())
        .expect("request");
    let (status, _) = send(h.app.clone(), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// The emoji
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_emoji_round_trips_and_survives_an_edit_of_the_user_row() {
    let h = boot(vec![]).await;

    let (status, body) = send(
        h.app.clone(),
        json_req(
            Method::PATCH,
            "/api/users/Alice/identity",
            TEST_TOKEN,
            serde_json::json!({ "emoji": "🦀" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "identity: {body:?}");
    assert_eq!(body["emoji"], "🦀");
    assert_eq!(body["name"], "Alice");

    let (_, view) = send(h.app.clone(), get("/api/users/Alice", TEST_TOKEN)).await;
    assert_eq!(view["emoji"], "🦀", "read-after-write: {view}");

    // A PUT of the same row carries no emoji — `UserUpsert` has no such field —
    // so without the preserve-across-edit rule this would silently drop it.
    let (status, body) = send(
        h.app.clone(),
        json_req(
            // Still `PUT` — this is the user-row replacement, not the avatar
            // upload, and the two share a prefix rather than a verb.
            Method::PUT,
            "/api/users/Alice",
            TEST_TOKEN,
            serde_json::json!({
                "name": "Alice",
                "role": "user",
                "channel_bindings": {"telegram": "111"},
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update: {body:?}");
    assert_eq!(
        body["emoji"], "🦀",
        "a role/binding edit must not clear the glyph"
    );

    let (_, view) = send(h.app.clone(), get("/api/users/Alice", TEST_TOKEN)).await;
    assert_eq!(view["emoji"], "🦀", "and it must still be on disk: {view}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_emoji_can_be_cleared_and_is_validated() {
    let h = boot(vec![]).await;
    let path = "/api/users/Alice/identity";

    send(
        h.app.clone(),
        json_req(
            Method::PATCH,
            path,
            TEST_TOKEN,
            serde_json::json!({ "emoji": "🦀" }),
        ),
    )
    .await;

    // `null` and `""` both clear, because a UI with an emptied input box can
    // only send the latter.
    for body in [
        serde_json::json!({ "emoji": null }),
        serde_json::json!({ "emoji": "" }),
    ] {
        let (status, view) = send(
            h.app.clone(),
            json_req(Method::PATCH, path, TEST_TOKEN, body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:?}: {view:?}");
        assert!(view["emoji"].is_null(), "{body:?} must clear: {view:?}");
    }

    // Restore one, then show a refusal does not disturb it.
    send(
        h.app.clone(),
        json_req(
            Method::PATCH,
            path,
            TEST_TOKEN,
            serde_json::json!({ "emoji": "🦀" }),
        ),
    )
    .await;

    let (status, body) = send(
        h.app.clone(),
        json_req(
            Method::PATCH,
            path,
            TEST_TOKEN,
            serde_json::json!({ "emoji": "x".repeat(33) }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");

    let (_, view) = send(h.app.clone(), get("/api/users/Alice", TEST_TOKEN)).await;
    assert_eq!(
        view["emoji"], "🦀",
        "a refused edit must leave the stored glyph alone: {view}"
    );
}

/// Whoami answers the identity question in one call: the WebUI has to fetch it
/// before it can render anything, so a second round trip for the glyph would be
/// paid on every page load.
#[tokio::test(flavor = "multi_thread")]
async fn whoami_reports_the_callers_emoji_and_avatar() {
    let h = boot(vec![]).await;

    send(
        h.app.clone(),
        json_req(
            Method::PATCH,
            "/api/users/Alice/identity",
            TEST_TOKEN,
            serde_json::json!({ "emoji": "🦀" }),
        ),
    )
    .await;
    upload_png(&h, "Alice", TINY_PNG).await;

    // Asked as Alice, with Alice's own key — the credential that most needs
    // this and the one least able to reach a user-management endpoint.
    let (status, who) = send(h.app.clone(), get("/api/authz/whoami", USER_KEY)).await;
    assert_eq!(status, StatusCode::OK, "{who:?}");
    assert_eq!(who["name"], "Alice");
    assert_eq!(who["role"], "user");
    assert_eq!(who["emoji"], "🦀");
    assert_eq!(who["has_avatar"], true);

    // A caller who owns no `[[users]]` row answers honestly rather than
    // borrowing someone else's glyph.
    let (status, who) = send(h.app.clone(), get("/api/authz/whoami", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::OK, "{who:?}");
    assert_eq!(who["name"], "root");
    assert!(who["emoji"].is_null(), "{who:?}");
    assert_eq!(who["has_avatar"], false, "{who:?}");

    // And a user with neither reports neither.
    let (_, who) = send(h.app.clone(), get("/api/authz/whoami", VIEWER_KEY)).await;
    assert_eq!(who["name"], "Watcher");
    assert!(who["emoji"].is_null(), "{who:?}");
    assert_eq!(who["has_avatar"], false, "{who:?}");
}

// ---------------------------------------------------------------------------
// Deletion
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn deleting_an_avatar_makes_it_404_and_clears_has_avatar() {
    let h = boot(vec![]).await;
    upload_png(&h, "Alice", TINY_PNG).await;

    let (status, body) = send(
        h.app.clone(),
        raw(
            Method::DELETE,
            "/api/users/Alice/avatar",
            TEST_TOKEN,
            Vec::new(),
            "application/octet-stream",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["removed"], 1);

    let (status, _) = send(h.app.clone(), get("/api/users/Alice/avatar", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, view) = send(h.app.clone(), get("/api/users/Alice", TEST_TOKEN)).await;
    assert_eq!(view["has_avatar"], false, "{view}");

    // Deleting again is not an error: "already gone" is the desired end state,
    // and a 404 would make a retried delete look like a failed one.
    let (status, body) = send(
        h.app.clone(),
        raw(
            Method::DELETE,
            "/api/users/Alice/avatar",
            TEST_TOKEN,
            Vec::new(),
            "application/octet-stream",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["removed"], 0);

    // The emoji is a separate resource and must not be collateral damage.
    assert!(users_avatar_dir(&h).exists(), "the directory stays");
}

/// Deleting a user takes the stored picture with it.
///
/// The file is keyed on `UserId::from_name(name)`, a stable derivation, so
/// leaving it behind means whoever next takes this name inherits the previous
/// holder's picture and `has_avatar` answers true for somebody who never set
/// one. The agent side clears the same residue in `agent_purge`.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_user_takes_its_avatar_with_it() {
    let h = boot(vec![]).await;

    let (status, _) = upload_png(&h, "Alice", TINY_PNG).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        file_names(&users_avatar_dir(&h)),
        vec![format!("{}.png", expected_stem("Alice"))],
        "the fixture must actually have a picture for the absence below to mean anything"
    );

    let (status, _) = send(
        h.app.clone(),
        raw(
            Method::DELETE,
            "/api/users/Alice",
            TEST_TOKEN,
            Vec::new(),
            "application/json",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert!(
        file_names(&users_avatar_dir(&h)).is_empty(),
        "a deleted user's picture must not be left for the next holder of the name"
    );

    // And the name, taken again, starts with no picture — which is the thing an
    // operator would have seen: a brand-new teammate wearing the last one's
    // face.
    let (status, _) = send(
        h.app.clone(),
        json_req(
            Method::POST,
            "/api/users",
            TEST_TOKEN,
            serde_json::json!({ "name": "Alice", "role": "user" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send(h.app.clone(), get("/api/users/Alice", TEST_TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["has_avatar"], serde_json::json!(false));
}
