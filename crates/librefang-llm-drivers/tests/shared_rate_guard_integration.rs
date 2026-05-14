//! Integration test for the cross-process / cross-restart rate-limit guard.
//!
//! Spins up a tiny TCP-based HTTP stub that always replies 429, then:
//!
//! 1. Calls the OpenAI driver once → server is hit, lockout is recorded.
//! 2. Calls the OpenAI driver a second time on a fresh driver instance
//!    (simulates a sibling process: new struct, no in-memory state) →
//!    request must short-circuit *without touching the network*.
//!
//! The "did the second call hit the network" assertion uses an atomic
//! counter on the stub server — the only way the counter can advance is via
//! a real connection.
//!
//! This locks down the issue's acceptance criteria:
//!   * 429 produces an atomic file under `~/.librefang/rate_limits/`.
//!   * A second client instantiated during the cooldown observes the same
//!     lockout and never reaches the provider.
//!   * Lockouts persist across "process boundaries" (here: across driver
//!     instances using only the file as shared state).

use librefang_llm_driver::{CompletionRequest, LlmDriver, LlmError};
use librefang_llm_drivers::backoff;
use librefang_llm_drivers::drivers::openai::OpenAIDriver;
use librefang_types::message::Message;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Read until we've consumed the request headers (terminator is `\r\n\r\n`),
/// then ignore the body — we don't care for the test.
async fn drain_request<S: AsyncReadExt + Unpin>(stream: &mut S) {
    let mut buf = [0u8; 4096];
    // Best-effort: read once, then bail. Real HTTP parsers do more, but the
    // reqwest client always sends the full request before reading the
    // response, so a single chunk usually contains the headers.
    let _ = tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await;
}

/// Spawn a TCP listener that:
/// - replies HTTP/1.1 429 with `x-ratelimit-reset-requests-1h: 3540` to
///   every request,
/// - increments `hit_count` once per accepted connection.
///
/// Returns `(base_url, hit_count)`.
async fn spawn_stub_429_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = hits.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            hits_clone.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                drain_request(&mut stream).await;
                let body = b"{\"error\":{\"message\":\"rate limited\",\"type\":\"rate_limit\"}}";
                // NOTE: deliberately omit `retry-after` so the driver's
                // backoff loop uses its default 2-second base instead of
                // sleeping for the full minute. The shared-rate-guard
                // file should still be written from the
                // `x-ratelimit-reset-requests-1h` header.
                let response = format!(
                    "HTTP/1.1 429 Too Many Requests\r\n\
                     content-type: application/json\r\n\
                     content-length: {}\r\n\
                     x-ratelimit-limit-requests-1h: 1000\r\n\
                     x-ratelimit-remaining-requests-1h: 0\r\n\
                     x-ratelimit-reset-requests-1h: 3540\r\n\
                     connection: close\r\n\
                     \r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(body).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    (format!("http://{addr}"), hits)
}

fn simple_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: std::sync::Arc::new(vec![Message::user("hello")]),
        tools: std::sync::Arc::new(Vec::new()),
        max_tokens: 16,
        temperature: 0.0,
        system: None,
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy::default(),
    }
}

#[tokio::test]
async fn shared_rate_guard_short_circuits_second_client() {
    // Isolate filesystem state via a fresh LIBREFANG_HOME.
    let home = tempfile::tempdir().expect("tempdir");
    // Use a process-unique key so we don't collide with sibling tests that
    // share the same working dir.
    let api_key = format!("sk-itest-{}", std::process::id());
    let _backoff_guard = backoff::enable_test_zero_backoff();
    std::env::set_var("LIBREFANG_HOME", home.path());
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("no_proxy", "127.0.0.1,localhost");

    let (base_url, hits) = spawn_stub_429_server().await;

    // ── First client: hits the server, records the lockout. ────────────
    let driver_a =
        OpenAIDriver::with_proxy_and_timeout(api_key.clone(), base_url.clone(), None, Some(2));
    let req = simple_request("gpt-test");
    let result_a = driver_a.complete(req.clone()).await;
    assert!(
        matches!(result_a, Err(LlmError::RateLimited { .. })),
        "first call should bubble up RateLimited, got {:?}",
        result_a
    );
    let hits_after_first = hits.load(Ordering::SeqCst);
    assert!(
        hits_after_first >= 1,
        "first client must have hit the server at least once, got {hits_after_first}"
    );

    // ── Second client: brand-new instance (= simulated other process). ──
    // It must short-circuit before any TCP connect.
    let driver_b =
        OpenAIDriver::with_proxy_and_timeout(api_key.clone(), base_url.clone(), None, Some(2));
    let result_b = driver_b.complete(req.clone()).await;
    assert!(
        matches!(result_b, Err(LlmError::RateLimited { .. })),
        "second client must also fail with RateLimited, got {:?}",
        result_b
    );
    let hits_after_second = hits.load(Ordering::SeqCst);
    assert_eq!(
        hits_after_second, hits_after_first,
        "second client must not touch the network — but counter went \
         {hits_after_first} → {hits_after_second}"
    );

    // ── The persisted file must exist and be readable JSON. ────────────
    let dir = home.path().join("rate_limits");
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("rate_limits dir")
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "exactly one lockout file should exist in {}",
        dir.display()
    );
    let bytes = std::fs::read(entries[0].path()).expect("read lockout file");
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json");
    assert_eq!(
        v.get("provider").and_then(|x| x.as_str()),
        Some("openai-compat")
    );
    let until = v
        .get("until_unix")
        .and_then(|x| x.as_u64())
        .expect("until_unix");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // The 1h header (3540s) wins over the 60s retry-after, so we expect
    // the lockout to extend ~hours into the future.
    assert!(
        until > now + 3000,
        "until_unix should be ~1h into the future, but it is now+{}s",
        until.saturating_sub(now)
    );
}
