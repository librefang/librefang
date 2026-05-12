// Integration tests for the `web_fetch_to_file` tool.
//
// Drives `tool_web_fetch_to_file` directly (no kernel needed) against a
// wiremock HTTP server. Each test:
//   - builds a `WebToolsContext` whose fetch config allow-lists 127.0.0.1 so
//     wiremock URLs survive the SSRF check;
//   - allocates a tempdir for the agent workspace;
//   - calls the tool and asserts on returned text + on-disk side effects.

use std::sync::Arc;
use std::time::Duration;

use librefang_runtime::web_cache::WebCache;
use librefang_runtime::web_fetch::WebFetchEngine;
use librefang_runtime::web_fetch_to_file::tool_web_fetch_to_file;
use librefang_runtime::web_search::{WebSearchEngine, WebToolsContext};
use librefang_types::config::{WebConfig, WebFetchConfig};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

fn build_web_ctx(max_file_bytes: u64) -> WebToolsContext {
    let cache = Arc::new(WebCache::new(Duration::from_secs(60)));
    // Wiremock binds to 127.0.0.1, which is loopback — SSRF blocks it by
    // default. Open the door explicitly for the tests.
    let fetch_cfg = WebFetchConfig {
        ssrf_allowed_hosts: vec!["127.0.0.1".to_string()],
        max_file_bytes,
        ..Default::default()
    };
    WebToolsContext {
        search: WebSearchEngine::new(WebConfig::default(), cache.clone(), vec![]),
        fetch: WebFetchEngine::new(fetch_cfg, cache),
    }
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn writes_response_to_workspace_and_returns_metadata() {
    let server = MockServer::start().await;
    let body = "# Paper\n\nSome markdown body.";
    Mock::given(method("GET"))
        .and(path("/paper.md"))
        .respond_with(
            // `set_body_string` auto-sets Content-Type to text/plain even
            // when `insert_header` runs later in the builder chain, so use
            // the raw bytes form to keep our explicit Content-Type intact.
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/markdown")
                .set_body_bytes(body.as_bytes().to_vec()),
        )
        .mount(&server)
        .await;

    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);
    let url = format!("{}/paper.md", server.uri());

    let result = tool_web_fetch_to_file(
        &json!({
            "url": url,
            "dest_path": "papers/2412.0001.md",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect("tool should succeed");

    let on_disk = std::fs::read_to_string(ws.path().join("papers/2412.0001.md"))
        .expect("file should exist on disk");
    assert_eq!(on_disk, body);

    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    let expected_sha = format!("{:x}", hasher.finalize());

    assert!(
        result.contains(&format!("{} bytes", body.len())),
        "result: {result}"
    );
    assert!(result.contains(&expected_sha), "result: {result}");
    assert!(result.contains("text/markdown"), "result: {result}");
    assert!(result.contains("status: 200"), "result: {result}");
}

// ---------------------------------------------------------------------------
// Workspace jail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_dest_path_with_dotdot() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;
    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);

    let err = tool_web_fetch_to_file(
        &json!({
            "url": format!("{}/x", server.uri()),
            "dest_path": "../escape.md",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("traversal should be rejected");
    assert!(
        err.contains("Path traversal denied"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn rejects_absolute_dest_path_outside_workspace() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;
    let ws = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);

    let absolute_outside = other.path().join("escape.md");

    let err = tool_web_fetch_to_file(
        &json!({
            "url": format!("{}/x", server.uri()),
            "dest_path": absolute_outside.to_string_lossy(),
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("absolute path outside workspace should be rejected");
    // resolve_sandbox_path_ext rebases to canonical workspace root → "resolves outside"
    assert!(
        err.contains("resolves outside workspace") || err.contains("Path traversal"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// Size caps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_response_when_content_length_exceeds_configured_cap() {
    let server = MockServer::start().await;
    let body = "x".repeat(2048);
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(1024); // hard cap: 1 KiB

    let err = tool_web_fetch_to_file(
        &json!({
            "url": format!("{}/big", server.uri()),
            "dest_path": "big.bin",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("response should be rejected over cap");
    assert!(err.contains("exceeds cap"), "unexpected error: {err}");
    // And the file must not exist after rejection.
    assert!(!ws.path().join("big.bin").exists());
}

#[tokio::test]
async fn per_call_max_bytes_clamps_below_configured_cap() {
    let server = MockServer::start().await;
    let body = "x".repeat(2048);
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024); // hard cap large

    let err = tool_web_fetch_to_file(
        &json!({
            "url": format!("{}/big", server.uri()),
            "dest_path": "big.bin",
            "max_bytes": 512u64,
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("per-call cap should kick in");
    assert!(err.contains("exceeds cap"), "unexpected error: {err}");
}

// ---------------------------------------------------------------------------
// SSRF
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ssrf_blocks_loopback_when_not_in_allowlist() {
    let ws = tempfile::tempdir().unwrap();
    // Build a context WITHOUT the 127.0.0.1 allowlist entry.
    let cache = Arc::new(WebCache::new(Duration::from_secs(60)));
    let fetch_cfg = WebFetchConfig::default(); // ssrf_allowed_hosts is empty
    let ctx = WebToolsContext {
        search: WebSearchEngine::new(WebConfig::default(), cache.clone(), vec![]),
        fetch: WebFetchEngine::new(fetch_cfg, cache),
    };

    let err = tool_web_fetch_to_file(
        &json!({
            "url": "http://127.0.0.1:9/never-reached",
            "dest_path": "paper.md",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("loopback URL must be blocked by SSRF");
    assert!(err.contains("SSRF blocked"), "unexpected error: {err}");
}

// ---------------------------------------------------------------------------
// HTTP error pass-through
// ---------------------------------------------------------------------------

#[tokio::test]
async fn surfaces_http_error_status_without_writing_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);

    let err = tool_web_fetch_to_file(
        &json!({
            "url": format!("{}/missing", server.uri()),
            "dest_path": "should-not-exist.md",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("404 should bubble up as error");
    assert!(err.contains("HTTP 404"), "unexpected error: {err}");
    assert!(!ws.path().join("should-not-exist.md").exists());
}

// ---------------------------------------------------------------------------
// Required-param validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_url_returns_clear_error() {
    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);
    let err = tool_web_fetch_to_file(
        &json!({ "dest_path": "x.md" }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("missing url");
    assert!(err.contains("'url'"), "unexpected error: {err}");
}

#[tokio::test]
async fn missing_dest_path_returns_clear_error() {
    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);
    let err = tool_web_fetch_to_file(
        &json!({ "url": "http://example.com/x" }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("missing dest_path");
    assert!(err.contains("'dest_path'"), "unexpected error: {err}");
}

// ---------------------------------------------------------------------------
// HTTP method whitelist
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_unsupported_http_method() {
    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);
    let err = tool_web_fetch_to_file(
        &json!({
            "url": "http://example.com/x",
            "dest_path": "x.md",
            "method": "HEAD",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("unsupported method should be rejected");
    assert!(
        err.contains("Unsupported HTTP method 'HEAD'"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// HTTP error body preview
// ---------------------------------------------------------------------------

#[tokio::test]
async fn http_error_includes_body_preview_in_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/problem"))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_bytes(br#"{"type":"validation","detail":"missing arxiv id"}"#.to_vec()),
        )
        .mount(&server)
        .await;

    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);

    let err = tool_web_fetch_to_file(
        &json!({
            "url": format!("{}/problem", server.uri()),
            "dest_path": "out.md",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("4xx should bubble up");
    assert!(err.contains("HTTP 422"), "unexpected error: {err}");
    assert!(
        err.contains("missing arxiv id"),
        "error should include body preview: {err}"
    );
    assert!(!ws.path().join("out.md").exists());
}

// ---------------------------------------------------------------------------
// Streaming cap when Content-Length is absent
// ---------------------------------------------------------------------------

/// Minimal chunked-encoding TCP server used to exercise the streaming bound
/// check — wiremock always emits `Content-Length`, so without this we can't
/// cover the "server omitted Content-Length and pushed past the cap" branch.
async fn spawn_chunked_server(total_bytes: usize, chunk_size: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Drain the request line / headers; we don't actually parse.
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\n\
                      Content-Type: application/octet-stream\r\n\
                      Transfer-Encoding: chunked\r\n\
                      \r\n",
                )
                .await;
            let mut sent = 0;
            while sent < total_bytes {
                let this_chunk = chunk_size.min(total_bytes - sent);
                let header = format!("{this_chunk:x}\r\n");
                if stream.write_all(header.as_bytes()).await.is_err() {
                    return;
                }
                if stream.write_all(&vec![b'x'; this_chunk]).await.is_err() {
                    return;
                }
                if stream.write_all(b"\r\n").await.is_err() {
                    return;
                }
                sent += this_chunk;
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
            let _ = stream.shutdown().await;
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn streaming_cap_aborts_when_content_length_is_absent() {
    // Server sends 2 KiB via chunked encoding (no Content-Length header);
    // cap is 1 KiB so the streaming bound check is the only line of defense.
    let base = spawn_chunked_server(2048, 256).await;
    let ws = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(1024);

    let err = tool_web_fetch_to_file(
        &json!({
            "url": format!("{base}/big"),
            "dest_path": "big.bin",
        }),
        Some(&ctx),
        Some(ws.path()),
        &[],
    )
    .await
    .expect_err("streaming bound check should fire");
    assert!(
        err.contains("exceeded cap"),
        "expected streaming-cap message, got: {err}"
    );
    assert!(!ws.path().join("big.bin").exists());
}

// ---------------------------------------------------------------------------
// Read-write named workspace via additional_roots
// ---------------------------------------------------------------------------

#[tokio::test]
async fn writes_into_rw_named_workspace_via_additional_roots() {
    let server = MockServer::start().await;
    let body = b"named-workspace-data";
    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
        .mount(&server)
        .await;

    let primary = tempfile::tempdir().unwrap();
    let named = tempfile::tempdir().unwrap();
    let ctx = build_web_ctx(50 * 1024 * 1024);

    // Canonicalise the named-workspace prefix the same way the kernel would
    // before passing it to the tool — matches `KernelHandle::named_workspace_prefixes`.
    let named_canon = named.path().canonicalize().unwrap();
    let dest = named_canon.join("downloaded.bin");

    let result = tool_web_fetch_to_file(
        &json!({
            "url": format!("{}/x", server.uri()),
            "dest_path": dest.to_string_lossy(),
        }),
        Some(&ctx),
        Some(primary.path()),
        &[named_canon.as_path()],
    )
    .await
    .expect("RW named workspace write should succeed");

    let on_disk = std::fs::read(&dest).expect("file should land in named workspace");
    assert_eq!(on_disk, body);
    assert!(
        result.contains(&format!("{} bytes", body.len())),
        "result: {result}"
    );
}
