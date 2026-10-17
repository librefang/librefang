//! Integration tests for the body-size limits on `POST /api/agents/{id}/upload` (#8181).
//!
//! The upload router carries its own, larger `RequestBodyLimitLayer` sized to `max_upload_size_bytes`, and the comment above it has claimed since it was written that uploads therefore bypass the global `max_request_body_bytes` cap.
//! They did not: the global layer was applied to the finished `app`, which wraps every route already registered, merged sub-routers included, and the smaller of two nested limits is the one that cuts.
//! An operator who raised `max_upload_size_bytes` to 100 MB still had every upload over the 1 MB global cap killed mid-stream, which reaches a browser as `NetworkError` rather than a status.
//!
//! These tests run against the production router (`server::build_router`), because that is where the ordering lives — a hand-rolled router in the test would assert on a layer stack nobody serves.
//!
//! Run: cargo test -p librefang-api --test upload_body_limit_test

use axum::body::Body;
use axum::http::{Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

/// `AgentId` is a UUID and the handler parses it before anything else, so a name-shaped id gets a 400 that never reaches the body.
/// The agent need not exist: the upload handler writes the file and does not look the agent up.
const TEST_AGENT_ID: &str = "00000000-0000-4000-8000-000000000001";

/// 1 KiB global cap against a 64 KiB upload cap: small enough that an 8 KiB body sits unambiguously between them, so a test body cannot accidentally satisfy both.
const GLOBAL_BODY_CAP: usize = 1024;
const UPLOAD_BODY_CAP: usize = 64 * 1024;
const BETWEEN_THE_CAPS: usize = 8 * 1024;

/// The production default for `max_upload_size_bytes`, so the case below is the one an operator who changed nothing actually hits.
const DEFAULT_UPLOAD_BODY_CAP: usize = 10 * 1024 * 1024;

/// Above axum's own 2 MiB `DefaultBodyLimit` — the limit the `Bytes` extractor applies when nothing overrides it — and below `DEFAULT_UPLOAD_BODY_CAP`, so nothing but the extractor limit can refuse it.
/// The caps the rest of this file uses are all under 2 MiB, which is exactly why they could not see this limit (#8185).
const OVER_THE_EXTRACTOR_DEFAULT: usize = 3 * 1024 * 1024;

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    boot_with_upload_cap(UPLOAD_BODY_CAP).await
}

async fn boot_with_upload_cap(upload_cap: usize) -> Harness {
    boot_with_caps(upload_cap, KernelConfig::default().max_concurrent_uploads).await
}

async fn boot_with_concurrency_cap(concurrency_cap: usize) -> Harness {
    boot_with_caps(UPLOAD_BODY_CAP, concurrency_cap).await
}

async fn boot_with_caps(upload_cap: usize, concurrency_cap: usize) -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        max_request_body_bytes: GLOBAL_BODY_CAP,
        max_upload_size_bytes: upload_cap,
        max_concurrent_uploads: concurrency_cap,
        // Without this, `upload_file` resolves its destination via
        // `effective_file_download_dir()`, whose fallback is
        // `std::env::temp_dir().join("librefang_uploads")` — a directory shared by every test
        // binary and never cleaned up. Every `201 CREATED` case below would otherwise leave a
        // file plus a `.meta.json` sidecar there permanently, growing across CI runs.
        channels: librefang_types::config::ChannelsConfig {
            file_download_dir: Some(tmp.path().join("uploads").to_string_lossy().into_owned()),
            ..Default::default()
        },
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
    };

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.clone().set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn upload_request(len: usize) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/agents/{TEST_AGENT_ID}/upload"))
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .header("content-type", "text/plain")
        .header("x-filename", "attachment.txt")
        .header("content-length", len.to_string())
        .body(Body::from(vec![b'a'; len]))
        .expect("request")
}

/// A body frame for [`stalled_upload_request`]'s channel, in the shape `Body::from_stream` accepts.
type BodyChunk = Result<axum::body::Bytes, std::io::Error>;

/// An upload request whose body stays empty until the test sends the declared bytes over the
/// returned channel and drops it.
///
/// `limit_concurrent_uploads` acquires its permit before the `Bytes` extractor awaits a single
/// frame, so a request parked on this channel is provably mid-flight — holding its permit —
/// regardless of how the scheduler interleaves the test's own polls.
fn stalled_upload_request(len: usize) -> (Request<Body>, mpsc::Sender<BodyChunk>) {
    let (sender, receiver) = mpsc::channel::<BodyChunk>(1);
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/agents/{TEST_AGENT_ID}/upload"))
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .header("content-type", "text/plain")
        .header("x-filename", "attachment.txt")
        .header("content-length", len.to_string())
        .body(Body::from_stream(ReceiverStream::new(receiver)))
        .expect("request");
    (request, sender)
}

/// Polls `future` once with a waker that never fires: the caller drives every future by hand
/// from a single task, so there is nothing to wake.
fn poll_noop<F: Future>(future: &mut std::pin::Pin<&mut F>) -> Poll<F::Output> {
    let mut cx = Context::from_waker(std::task::Waker::noop());
    future.as_mut().poll(&mut cx)
}

/// The regression itself: a body the operator's `max_upload_size_bytes` allows and the global cap does not must reach the handler.
#[tokio::test(flavor = "multi_thread")]
async fn upload_between_the_global_cap_and_the_upload_cap_is_accepted() {
    let harness = boot().await;

    let response = harness
        .app
        .clone()
        .oneshot(upload_request(BETWEEN_THE_CAPS))
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "an {BETWEEN_THE_CAPS}-byte upload is under max_upload_size_bytes ({UPLOAD_BODY_CAP}) and must be accepted even though it exceeds max_request_body_bytes ({GLOBAL_BODY_CAP})"
    );
}

/// The stream cap is not the only cap: `upload_file` extracts `axum::body::Bytes`, whose own limit is 2 MiB unless `DefaultBodyLimit` overrides it.
///
/// Every other case in this file runs under caps below 2 MiB, so they all pass whether or not that override is there — which is how the limit survived the first fix.
/// A real attachment is usually bigger than 2 MiB, so without the override raising `max_upload_size_bytes` buys the operator nothing (#8185).
#[tokio::test(flavor = "multi_thread")]
async fn upload_over_the_extractor_default_is_accepted_under_a_larger_cap() {
    let harness = boot_with_upload_cap(DEFAULT_UPLOAD_BODY_CAP).await;

    let response = harness
        .app
        .clone()
        .oneshot(upload_request(OVER_THE_EXTRACTOR_DEFAULT))
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a {OVER_THE_EXTRACTOR_DEFAULT}-byte upload is well under max_upload_size_bytes ({DEFAULT_UPLOAD_BODY_CAP}) and must be accepted; a 413 here means the Bytes extractor is still capped at axum's 2 MiB default"
    );
}

/// A body over the upload cap gets an answer, not a dropped connection, and the answer names the cap.
#[tokio::test(flavor = "multi_thread")]
async fn upload_over_the_upload_cap_gets_a_413_naming_the_cap() {
    let harness = boot().await;

    let response = harness
        .app
        .clone()
        .oneshot(upload_request(UPLOAD_BODY_CAP + 1))
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a body over max_upload_size_bytes must be refused with a status the client can render"
    );

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("error body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("error body is JSON");
    assert_eq!(
        json["max_upload_size_bytes"], UPLOAD_BODY_CAP,
        "the refusal must tell the client which cap it hit, so 'too large' is distinguishable from 'daemon unreachable': {json}"
    );
}

/// `upload_file` extracts `axum::body::Bytes`, buffering the whole body into RAM before it runs,
/// and nothing but `max_concurrent_uploads` bounds how many of those buffers can be resident at
/// once — the per-request caps above don't help, an attacker just opens N connections at the cap.
///
/// This test used to drive the three requests with `tokio::join!` and rely on the combinator
/// polling in submission order: the first two were expected to reach `upload_file`'s
/// `tokio::fs::write` — an `.await` point — and yield there, still holding their permits, before
/// the third arm was polled. That premise is not sound, and CI caught it on a loaded shard with
/// `[201, 201, 201]` (#8608). Every `tokio::fs` operation is a blocking-pool task, and the join
/// handle for a blocking task that has already finished is `Ready` on its very first poll
/// (`tokio`'s `asyncify` spawns the task and polls its join handle without yielding in between).
/// Under load the polling thread can be descheduled inside that window, so a request — body
/// extraction, directory creation, `fs::write`, metadata sidecar and all — can run start to
/// finish within a single poll of its future, dropping its permit back into the pool before the
/// combinator ever reaches the third arm. The assertion then counted three 201s and the test
/// failed while the production cap was behaving correctly.
///
/// This version pins the mechanism instead of the scheduler. The first two requests are fed from
/// a body channel the test controls, and the concurrency middleware acquires its permit before
/// the `Bytes` extractor awaits a single frame, so an empty channel parks each of them at a
/// point *known* to be behind the permit — with no dependency on whether any filesystem await
/// happens to yield. Both futures are then polled by hand, with a no-op waker, until they report
/// `Pending`; only then is the third request polled, and it must be refused with 429 in that
/// same poll, because `try_acquire` fails and `limit_concurrent_uploads` builds the refusal
/// without awaiting. A `Pending` third poll would mean the request got past the semaphore — the
/// cap failing, which this test reports as exactly that rather than as a scheduling flake.
/// The assertion set is unchanged: exactly two accepted, exactly one rejected with
/// `StatusCode::TOO_MANY_REQUESTS`.
#[tokio::test(flavor = "multi_thread")]
async fn upload_over_the_concurrency_cap_is_rejected_with_429() {
    let harness = boot_with_concurrency_cap(2).await;
    let app = &harness.app;

    let (request1, release1) = stalled_upload_request(BETWEEN_THE_CAPS);
    let (request2, release2) = stalled_upload_request(BETWEEN_THE_CAPS);

    let mut first = pin!(app.clone().oneshot(request1));
    let mut second = pin!(app.clone().oneshot(request2));
    let mut third = pin!(app.clone().oneshot(upload_request(BETWEEN_THE_CAPS)));

    // A request parked here is past `limit_concurrent_uploads`'s `try_acquire` (that middleware
    // wraps the extractor) and has produced no response, so it holds its permit. A `Ready` here
    // would mean the request settled before its body was sent — impossible while the channel is
    // open and empty — so it is a hook for future reordering, not a race to tolerate.
    for (name, future) in [("first", &mut first), ("second", &mut second)] {
        assert!(
            matches!(poll_noop(future), Poll::Pending),
            "the {name} upload settled before its body was released, so the test cannot have two \
             permits checked out and must not go on to assert anything about the third request"
        );
    }

    // Both permits are checked out now, so the third request must be refused in its first poll.
    let refused = match poll_noop(&mut third) {
        Poll::Ready(response) => response.expect("router responds"),
        Poll::Pending => panic!(
            "the third upload got past the concurrency semaphore even though two uploads were \
             mid-flight, so max_concurrent_uploads (2) is not enforced; this is the cap regression \
             itself, not a scheduling artifact"
        ),
    };
    assert_eq!(
        refused.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the request over the concurrency cap must get 429, not queue or succeed"
    );

    // Release the two admitted uploads; each sends the bytes its `content-length` declared.
    release1
        .send(Ok(axum::body::Bytes::from(vec![b'a'; BETWEEN_THE_CAPS])))
        .await
        .expect("first body receiver is still attached");
    drop(release1);
    release2
        .send(Ok(axum::body::Bytes::from(vec![b'a'; BETWEEN_THE_CAPS])))
        .await
        .expect("second body receiver is still attached");
    drop(release2);

    let (r1, r2) = tokio::join!(first, second);
    let statuses = [
        r1.expect("router responds").status(),
        r2.expect("router responds").status(),
        refused.status(),
    ];

    let accepted = statuses
        .iter()
        .filter(|s| **s == StatusCode::CREATED)
        .count();
    let rejected = statuses
        .iter()
        .filter(|s| **s == StatusCode::TOO_MANY_REQUESTS)
        .count();
    assert_eq!(
        accepted, 2,
        "exactly max_concurrent_uploads (2) requests may be mid-flight at once: {statuses:?}"
    );
    assert_eq!(
        rejected, 1,
        "the request over the concurrency cap must get 429, not queue or succeed: {statuses:?}"
    );
}

/// The global cap has to stay OUTSIDE the JSON depth guard, not just outside the upload router.
///
/// The guard buffers an `application/json` body whole under its own 8 MiB `HARD_CEILING_BYTES` before anything downstream sees it, so a limit nested inside it lets an authenticated caller stage 8 MiB against a 1 KiB cap and only then get refused.
/// The status cannot tell the two orderings apart — the guard answers 413 as well (`middleware.rs`, `to_bytes` error arm) — so the assertion is on *whose* refusal it is: the guard names itself in the body, and reaching that message means it buffered the 9 MiB the cap was supposed to have cut.
#[tokio::test(flavor = "multi_thread")]
async fn oversized_json_is_cut_by_the_cap_not_staged_by_the_depth_guard() {
    let harness = boot().await;

    let over_the_guard_ceiling = 9 * 1024 * 1024;
    let mut body = Vec::with_capacity(over_the_guard_ceiling);
    body.extend_from_slice(b"[\"");
    body.resize(over_the_guard_ceiling - 2, b'a');
    body.extend_from_slice(b"\"]");

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/{TEST_AGENT_ID}/message"))
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .header("content-type", "application/json")
                .header("content-length", body.len().to_string())
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "max_request_body_bytes ({GLOBAL_BODY_CAP}) must refuse this body"
    );

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("error body");
    let body = String::from_utf8_lossy(&bytes);
    assert!(
        !body.contains("depth guard"),
        "the refusal must come from the body-size cap, not from the JSON depth guard: reaching the guard's message means it buffered 9 MiB before a {GLOBAL_BODY_CAP}-byte cap ran, which is the staging this ordering exists to prevent. Body: {body}"
    );
}

/// Uploads are outside the depth guard, which is the point of merging them after it.
///
/// `application/json` is an allowed upload type (`EXTRA_ALLOWED_UPLOAD_TYPES`), and the guard's ceiling was a third undeclared cap on those uploads — the same class of override as the global limit this issue is about.
/// A tiny deeply-nested body separates the two arrangements without needing a large one: inside the guard it is a 400, outside it is stored like any other bytes.
#[tokio::test(flavor = "multi_thread")]
async fn deeply_nested_json_upload_is_stored_rather_than_parsed() {
    let harness = boot().await;

    // 60 levels comfortably exceeds MAX_JSON_BODY_DEPTH (32).
    let body = format!("{}0{}", "[".repeat(60), "]".repeat(60));

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/{TEST_AGENT_ID}/upload"))
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .header("content-type", "application/json")
                .header("x-filename", "payload.json")
                .header("content-length", body.len().to_string())
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the upload handler writes bytes to disk and never parses them, so the JSON depth guard must not sit in front of it"
    );
}

/// The control for the fix: moving the global limit off `app` must not remove it from the routes it was protecting.
#[tokio::test(flavor = "multi_thread")]
async fn the_global_cap_still_applies_off_the_upload_path() {
    let harness = boot().await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/{TEST_AGENT_ID}/message"))
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                // Not `application/json`, so the JSON depth guard forwards it untouched and the body limit is the only thing that can reject it.
                .header("content-type", "text/plain")
                .header("content-length", BETWEEN_THE_CAPS.to_string())
                .body(Body::from(vec![b'a'; BETWEEN_THE_CAPS]))
                .expect("request"),
        )
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a non-upload route must still be capped at max_request_body_bytes ({GLOBAL_BODY_CAP})"
    );
}
