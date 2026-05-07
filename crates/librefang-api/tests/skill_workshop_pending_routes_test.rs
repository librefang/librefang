//! Integration tests for the `/api/skills/pending/*` HTTP surface (#3328).
//!
//! Covers the four endpoints registered in
//! `crates/librefang-api/src/routes/skills.rs`:
//!
//!   * `GET  /api/skills/pending`
//!   * `GET  /api/skills/pending/{id}`
//!   * `POST /api/skills/pending/{id}/approve`
//!   * `POST /api/skills/pending/{id}/reject`
//!
//! Same `tower::oneshot` + `MockKernelBuilder` + `TestAppState` pattern
//! used by `auto_dream_routes_integration.rs`. We seed the pending tree
//! directly through `librefang_kernel::skill_workshop::storage::save_candidate`
//! (the same path the after-turn hook would take in production) so the
//! test does not depend on a live LLM driver.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use librefang_api::routes::{self, AppState};
use librefang_kernel::skill_workshop::candidate::{CandidateSkill, CaptureSource, Provenance};
use librefang_kernel::skill_workshop::storage;
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn skills_root(harness: &Harness) -> PathBuf {
    harness.state.kernel.home_dir().join("skills")
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        // Same non-LLM provider trick as auto_dream tests — the workshop
        // routes don't dispatch any LLM calls themselves, but the kernel
        // boot wires up a default driver for everything else.
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::HashMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));

    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::skills::router())
        .with_state(state.clone());

    Harness {
        app,
        state,
        _test: test,
    }
}

async fn json_request(h: &Harness, method: Method, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

fn fixture_candidate(agent_id: &str, id: &str) -> CandidateSkill {
    CandidateSkill {
        id: id.to_string(),
        agent_id: agent_id.to_string(),
        session_id: Some("session-x".to_string()),
        captured_at: Utc::now(),
        source: CaptureSource::ExplicitInstruction {
            trigger: "from now on".to_string(),
        },
        name: "fmt_before_commit".to_string(),
        description: "Run cargo fmt before commit".to_string(),
        prompt_context: "# Cargo fmt before commit\n\nRun `cargo fmt --all`.\n".to_string(),
        provenance: Provenance {
            user_message_excerpt: "from now on always run cargo fmt before commit".to_string(),
            assistant_response_excerpt: Some("Got it.".to_string()),
            turn_index: 1,
        },
    }
}

// ---------------------------------------------------------------------------
// GET /api/skills/pending
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pending_list_empty_returns_empty_array() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/skills/pending").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body["candidates"].is_array(),
        "candidates must be an array even when empty: {body:?}"
    );
    assert_eq!(body["candidates"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_list_returns_seeded_candidates() {
    let h = boot().await;
    let agent = "11111111-1111-1111-1111-111111111111";
    let id_a = "00000000-0000-0000-0000-00000000000a";
    let id_b = "00000000-0000-0000-0000-00000000000b";
    let root = skills_root(&h);
    storage::save_candidate(&root, &fixture_candidate(agent, id_a), 20).unwrap();
    storage::save_candidate(&root, &fixture_candidate(agent, id_b), 20).unwrap();

    let (status, body) = json_request(&h, Method::GET, "/api/skills/pending").await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let arr = body["candidates"].as_array().unwrap();
    assert_eq!(arr.len(), 2, "{body:?}");
    let ids: Vec<&str> = arr.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&id_a));
    assert!(ids.contains(&id_b));
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_list_filters_by_agent() {
    let h = boot().await;
    let root = skills_root(&h);
    let agent_a = "11111111-1111-1111-1111-111111111111";
    let agent_b = "22222222-2222-2222-2222-222222222222";
    storage::save_candidate(
        &root,
        &fixture_candidate(agent_a, "aaaaaaaa-0000-0000-0000-000000000001"),
        20,
    )
    .unwrap();
    storage::save_candidate(
        &root,
        &fixture_candidate(agent_b, "bbbbbbbb-0000-0000-0000-000000000002"),
        20,
    )
    .unwrap();

    let (status, body) = json_request(
        &h,
        Method::GET,
        &format!("/api/skills/pending?agent={agent_a}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let arr = body["candidates"].as_array().unwrap();
    assert_eq!(arr.len(), 1, "filter must scope to single agent: {body:?}");
    assert_eq!(arr[0]["agent_id"], agent_a);
}

// ---------------------------------------------------------------------------
// GET /api/skills/pending/{id}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pending_show_returns_full_candidate() {
    let h = boot().await;
    let id = "cccccccc-0000-0000-0000-000000000003";
    let agent = "11111111-1111-1111-1111-111111111111";
    storage::save_candidate(&skills_root(&h), &fixture_candidate(agent, id), 20).unwrap();

    let (status, body) = json_request(&h, Method::GET, &format!("/api/skills/pending/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let candidate = &body["candidate"];
    assert_eq!(candidate["id"], id);
    assert_eq!(candidate["agent_id"], agent);
    assert_eq!(candidate["name"], "fmt_before_commit");
    assert_eq!(candidate["source"]["kind"], "explicit_instruction");
    assert!(candidate["prompt_context"]
        .as_str()
        .unwrap()
        .contains("cargo fmt"),);
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_show_unknown_id_returns_404() {
    let h = boot().await;
    // UUID-shaped id that has never been saved — must surface as 404,
    // not 400. Pre-#4741 this used "no-such-id" which now hits the new
    // UUID validation gate first.
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/skills/pending/00000000-0000-0000-0000-deadbeefdead",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_show_non_uuid_id_returns_400() {
    // Defence in depth: anything that isn't UUID-shaped must be
    // rejected at the route boundary, never reach the FS layer where
    // a future bug in path-joining could escape `pending/`.
    //
    // Restricted to single-path-segment inputs because axum splits
    // strings containing `/` into multiple segments before our handler
    // is reached, so `../etc` would surface as a route mismatch (404
    // from the router, not 400 from our extractor). Path-traversal
    // safety from those shapes is enforced separately by
    // `agent_pending_dir`'s UUID parse — see the storage unit tests.
    let h = boot().await;
    for bad in ["no-such-id", "12345", "AGENT-A"] {
        let (status, body) =
            json_request(&h, Method::GET, &format!("/api/skills/pending/{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?}: {body:?}");
        assert!(body["error"].as_str().unwrap_or("").contains("UUID"));
    }
}

// ---------------------------------------------------------------------------
// POST /api/skills/pending/{id}/approve
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_promotes_and_drops_pending() {
    let h = boot().await;
    let id = "dddddddd-0000-0000-0000-000000000004";
    let agent = "11111111-1111-1111-1111-111111111111";
    let root = skills_root(&h);
    storage::save_candidate(&root, &fixture_candidate(agent, id), 20).unwrap();

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{id}/approve"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "approved");
    assert_eq!(body["candidate_id"], id);
    assert_eq!(body["skill_name"], "fmt_before_commit");

    // Pending file gone, active skill landed.
    assert!(
        storage::load_candidate(&root, id).is_err(),
        "pending file must be removed after approve"
    );
    assert!(
        root.join("fmt_before_commit").join("skill.toml").exists(),
        "active skill not written to skills_root"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_unknown_id_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/pending/00000000-0000-0000-0000-deadbeefdead/approve",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_non_uuid_id_returns_400() {
    let h = boot().await;
    let (status, body) =
        json_request(&h, Method::POST, "/api/skills/pending/no-such-id/approve").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(body["error"].as_str().unwrap_or("").contains("UUID"));
}

// ---------------------------------------------------------------------------
// POST /api/skills/pending/{id}/reject
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pending_reject_removes_file() {
    let h = boot().await;
    let id = "eeeeeeee-0000-0000-0000-000000000005";
    let agent = "11111111-1111-1111-1111-111111111111";
    let root = skills_root(&h);
    storage::save_candidate(&root, &fixture_candidate(agent, id), 20).unwrap();
    assert!(
        storage::load_candidate(&root, id).is_ok(),
        "seed precondition"
    );

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{id}/reject"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "rejected");
    assert_eq!(body["candidate_id"], id);
    assert!(
        storage::load_candidate(&root, id).is_err(),
        "pending file must be removed after reject"
    );
    // No active skill should have been created.
    assert!(!root.join("fmt_before_commit").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_reject_unknown_id_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/pending/00000000-0000-0000-0000-deadbeefdead/reject",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(body["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_reject_non_uuid_id_returns_400() {
    let h = boot().await;
    let (status, body) =
        json_request(&h, Method::POST, "/api/skills/pending/no-such-id/reject").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(body["error"].as_str().unwrap_or("").contains("UUID"));
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_list_non_uuid_agent_filter_returns_400() {
    // `?agent=…` with a non-UUID value used to 500 with whatever
    // `read_dir` produced. The route layer now translates it to a
    // structured 400 before any FS work.
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/skills/pending?agent=../etc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(body["error"].as_str().unwrap_or("").contains("UUID"));
}
