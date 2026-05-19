//! Integration tests for credential pool routes (#4965).
//!
//! Boots a real kernel via `MockKernelBuilder` with a configured
//! `[[credential_pools]]` entry and hits `GET /api/credential-pools`
//! against the real axum router built by `server::build_router`. No
//! mocking of the kernel or the route handler — exercises the same
//! `KernelApi::credential_pool_summaries()` path the daemon serves.

use librefang_api::server;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{
    CredentialPoolConfig, CredentialPoolKeyConfig, CredentialPoolStrategy,
};
use serde_json::Value as Json;
use std::sync::Arc;
use tower::ServiceExt;

/// Use distinct env-var names per test so concurrent test runs (cargo
/// nextest spawns threads) don't trip over a shared `std::env::set_var`.
fn ev(name: &str) -> String {
    format!("LIBREFANG_TEST_CP_{name}")
}

fn pool_cfg(
    provider: &str,
    strategy: CredentialPoolStrategy,
    keys: &[(&str, &str, u32)],
) -> CredentialPoolConfig {
    CredentialPoolConfig {
        provider: provider.to_string(),
        strategy,
        keys: keys
            .iter()
            .map(|(label, env_var, priority)| CredentialPoolKeyConfig {
                api_key_env: env_var.to_string(),
                label: label.to_string(),
                priority: *priority,
            })
            .collect(),
    }
}

async fn boot_with_pools(pools: Vec<CredentialPoolConfig>) -> Arc<librefang_api::routes::AppState> {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.default_model.provider = "ollama".to_string();
        cfg.default_model.model = "test-model".to_string();
        cfg.default_model.api_key_env = "OLLAMA_API_KEY".to_string();
        cfg.credential_pools = pools.clone();
    }));
    let (state, _tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();
    // Leak the tmp dir handle for the test's lifetime; dropping it would
    // wipe the SQLite file from under the kernel mid-test.
    Box::leak(Box::new(_tmp));
    state
}

async fn get_json(
    state: &Arc<librefang_api::routes::AppState>,
    path: &str,
) -> (axum::http::StatusCode, Json) {
    // `MockKernelBuilder` defaults to an empty `api_key`. The auth middleware
    // still requires that origin be loopback (or `LIBREFANG_ALLOW_NO_AUTH=1`
    // be set) to grant access — without a real `oneshot` socket, no
    // `ConnectInfo` extension is attached and the middleware fails closed.
    // Inject a loopback `ConnectInfo` ourselves, mirroring the existing
    // `route_smoke.rs` / `auth_public_allowlist.rs` pattern.
    let kernel = state.kernel.clone();
    let (app, _state) = server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;
    let mut req = axum::http::Request::builder()
        .uri(path)
        .body(axum::body::Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Json = if bytes.is_empty() {
        Json::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn credential_pools_endpoint_returns_configured_pools() {
    // Set up two env vars so the pool actually materializes at boot.
    let key1 = ev("ROUTES_A_1");
    let key2 = ev("ROUTES_A_2");
    std::env::set_var(&key1, "sk-test-aaaa1111");
    std::env::set_var(&key2, "sk-test-bbbb2222");

    let pools = vec![pool_cfg(
        "openai",
        CredentialPoolStrategy::RoundRobin,
        &[("Primary", &key1, 10), ("Backup", &key2, 5)],
    )];
    let state = boot_with_pools(pools).await;

    let (status, body) = get_json(&state, "/api/credential-pools").await;
    assert_eq!(status, 200, "endpoint should be reachable, body: {body}");

    let arr = body.as_array().expect("response is an array");
    assert_eq!(arr.len(), 1, "exactly one pool configured");
    let pool = &arr[0];
    assert_eq!(pool["provider"], "openai");
    assert_eq!(pool["strategy"], "round_robin");
    assert_eq!(pool["available_count"], 2);
    assert_eq!(pool["total_count"], 2);

    let creds = pool["credentials"].as_array().unwrap();
    assert_eq!(creds.len(), 2);

    // Priority-descending order: Primary (10) before Backup (5).
    assert_eq!(creds[0]["label"], "Primary");
    assert_eq!(creds[0]["priority"], 10);
    assert_eq!(creds[0]["is_exhausted"], false);
    assert_eq!(creds[0]["request_count"], 0);
    // Redaction: key hint must be the last 4 chars only — never the raw key.
    let hint0 = creds[0]["key_hint"].as_str().unwrap();
    assert!(
        hint0.starts_with("****"),
        "key_hint should be redacted: {hint0}"
    );
    assert!(
        !hint0.contains("sk-test"),
        "raw key must never leak: {hint0}"
    );
    assert_eq!(hint0, "****1111");

    assert_eq!(creds[1]["label"], "Backup");
    assert_eq!(creds[1]["priority"], 5);
    assert_eq!(creds[1]["key_hint"], "****2222");
}

#[tokio::test(flavor = "multi_thread")]
async fn credential_pools_endpoint_returns_empty_array_when_no_pools_configured() {
    let state = boot_with_pools(vec![]).await;
    let (status, body) = get_json(&state, "/api/credential-pools").await;
    assert_eq!(status, 200);
    let arr = body.as_array().expect("response is an array");
    assert!(arr.is_empty(), "expected empty array, got {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn credential_pools_response_is_sorted_by_provider_name() {
    // Two pools, configured out of alphabetical order. The endpoint must
    // return them alphabetically so dashboard / CLI output is deterministic.
    let key_a = ev("ROUTES_SORT_A");
    let key_z = ev("ROUTES_SORT_Z");
    std::env::set_var(&key_a, "sk-aaaaAAAA");
    std::env::set_var(&key_z, "sk-zzzzZZZZ");

    let pools = vec![
        pool_cfg(
            "zeta",
            CredentialPoolStrategy::FillFirst,
            &[("only", &key_z, 1)],
        ),
        pool_cfg(
            "alpha",
            CredentialPoolStrategy::FillFirst,
            &[("only", &key_a, 1)],
        ),
    ];
    let state = boot_with_pools(pools).await;

    let (status, body) = get_json(&state, "/api/credential-pools").await;
    assert_eq!(status, 200);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["provider"], "alpha");
    assert_eq!(arr[1]["provider"], "zeta");
}

#[tokio::test(flavor = "multi_thread")]
async fn credential_pools_skips_pool_with_no_resolvable_env_keys() {
    // Pool refs an env var that isn't set — boot logs a warning and the
    // pool isn't materialized. Endpoint should return empty array (not 500).
    let pools = vec![pool_cfg(
        "openai",
        CredentialPoolStrategy::FillFirst,
        &[("ghost", &ev("ROUTES_DOES_NOT_EXIST"), 1)],
    )];
    let state = boot_with_pools(pools).await;
    let (status, body) = get_json(&state, "/api/credential-pools").await;
    assert_eq!(
        status, 200,
        "unresolved env keys should not crash the endpoint"
    );
    let arr = body.as_array().unwrap();
    assert!(
        arr.is_empty(),
        "pool with no resolvable keys should be skipped at boot"
    );
}

/// Regression for Codex PR #5260 review (P2):
///
/// When a configured pool has TWO keys but boot can resolve only the second
/// one (env var for the higher-priority key is unset), the snapshot endpoint
/// must attribute the materialized credential to the **second** configured
/// label — never the first one's label. The previous implementation indexed
/// `labels[0]` against `summary.credentials[0]`, shifting the first label
/// onto the only surviving key and telling the operator the wrong credential
/// was exhausted / valid.
#[tokio::test(flavor = "multi_thread")]
async fn credential_pools_label_tracks_materialized_key_when_higher_priority_env_unset() {
    // Use distinct env-var names so other tests cannot trip over us under
    // nextest's threaded harness.
    let unset_env = ev("PARTIAL_PRIMARY_UNSET");
    let set_env = ev("PARTIAL_BACKUP_SET");
    // Make sure the "unset" env var really is unset for this test, even if a
    // prior leaked invocation left it populated.
    std::env::remove_var(&unset_env);
    std::env::set_var(&set_env, "sk-test-only-backup-9999");

    // Higher priority on the UNSET entry so the historical positional bug
    // would have moved its "Primary" label onto the surviving "Backup" key.
    let pools = vec![pool_cfg(
        "openai",
        CredentialPoolStrategy::FillFirst,
        &[
            ("Primary", &unset_env, 10), // priority 10, but env var unset → skipped
            ("Backup", &set_env, 5),     // priority 5, env var set → materialized
        ],
    )];
    let state = boot_with_pools(pools).await;

    let (status, body) = get_json(&state, "/api/credential-pools").await;
    assert_eq!(status, 200);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1, "exactly one pool was configured");
    let pool = &arr[0];
    assert_eq!(pool["provider"], "openai");
    assert_eq!(
        pool["total_count"], 1,
        "only the resolvable key materializes"
    );
    let creds = pool["credentials"].as_array().unwrap();
    assert_eq!(creds.len(), 1, "snapshot exposes only the resolved key");

    // The critical assertion: the surviving entry must be labelled "Backup"
    // (the materialized key's own label), NOT "Primary" (the skipped key's
    // label that the positional implementation would have shifted onto it).
    assert_eq!(
        creds[0]["label"], "Backup",
        "label must track the materialized key, not be reassigned by positional indexing"
    );
    assert_eq!(creds[0]["priority"], 5);
    assert_eq!(creds[0]["key_hint"], "****9999");

    // Cleanup so the env var doesn't leak into later tests.
    std::env::remove_var(&set_env);
}
