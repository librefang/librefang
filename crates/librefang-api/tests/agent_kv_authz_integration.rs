//! Integration coverage for owner-scoping on the per-agent KV store
//! (`/api/memory/agents/{id}/kv*` and `/api/agents/{id}/memory/{export,import}`).
//!
//! The list endpoint (`GET /kv`) was already gated by an inline
//! owner-or-admin check, but the single-key get / set / delete and the
//! export / import handlers were missed when the family was first
//! introduced — any authenticated non-admin caller could read or mutate
//! `user.preferences`, `oncall.contact`, `api.tokens`, etc. on any agent
//! as long as they knew the key name.
//!
//! These tests pin the helper `assert_kv_owner_or_admin` extracted in
//! the #3749 11/N follow-up: viewer != author returns 404, while owner,
//! admin, and trusted no-auth compatibility requests proceed.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware::AuthenticatedApiUser;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, UserId};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    // Mount both sub-routers so /memory/agents/{id}/kv* and
    // /agents/{id}/memory/{export,import} are reachable from one harness.
    let app = Router::new()
        .nest(
            "/api",
            routes::memory::router().merge(routes::agents::router()),
        )
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

fn spawn_owned_by(state: &Arc<AppState>, name: &str, author: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        source_template: None,
        author: author.to_string(),
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

fn admin(name: &str) -> AuthenticatedApiUser {
    AuthenticatedApiUser {
        name: name.to_string(),
        role: UserRole::Admin,
        user_id: UserId::from_name(name),
    }
}

fn viewer(name: &str) -> AuthenticatedApiUser {
    AuthenticatedApiUser {
        name: name.to_string(),
        role: UserRole::Viewer,
        user_id: UserId::from_name(name),
    }
}

fn req(method: Method, uri: &str, user: Option<AuthenticatedApiUser>) -> Request<Body> {
    req_with_body(method, uri, user, Body::empty())
}

fn req_with_body(
    method: Method,
    uri: &str,
    user: Option<AuthenticatedApiUser>,
    body: Body,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    if let Some(u) = user {
        request.extensions_mut().insert(u);
    }
    request
}

async fn run(h: &Harness, request: Request<Body>) -> StatusCode {
    run_response(h, request).await.status()
}

async fn run_response(h: &Harness, request: Request<Body>) -> axum::response::Response {
    h.app.clone().oneshot(request).await.unwrap()
}

async fn run_json(h: &Harness, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = run_response(h, request).await;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read response body");
    let body = serde_json::from_slice(&bytes).expect("JSON response body");
    (status, body)
}

// ---------------------------------------------------------------------------
// GET /api/memory/agents/{id}/kv  — list (already had owner-scoping; pinned
// here as a baseline so a regression to the inline check is caught alongside
// the new helper).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_kv_admin_can_read_any_agent() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req(
            Method::GET,
            &format!("/api/memory/agents/{id}/kv"),
            Some(admin("ops")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_kv_viewer_other_author_is_404() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req(
            Method::GET,
            &format!("/api/memory/agents/{id}/kv"),
            Some(viewer("mallory")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_kv_viewer_owner_is_ok() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req(
            Method::GET,
            &format!("/api/memory/agents/{id}/kv"),
            Some(viewer("alice")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_kv_trusted_no_auth_mode_proceeds() {
    // No `AuthenticatedApiUser` extension models the explicitly trusted
    // no-auth compatibility mode. Production authentication middleware is
    // tested separately; this router-level harness intentionally omits it.
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req(Method::GET, &format!("/api/memory/agents/{id}/kv"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Single-key get / set / delete — these were the missing checks; pre-fix a
// non-author viewer could read / write / delete any key on any agent.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_single_key_viewer_other_author_is_404() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req(
            Method::GET,
            &format!("/api/memory/agents/{id}/kv/api.token"),
            Some(viewer("mallory")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn put_single_key_viewer_other_author_is_404() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req_with_body(
            Method::PUT,
            &format!("/api/memory/agents/{id}/kv/api.token"),
            Some(viewer("mallory")),
            Body::from(r#"{"value":"pwned"}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_single_key_viewer_other_author_is_404() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req(
            Method::DELETE,
            &format!("/api/memory/agents/{id}/kv/api.token"),
            Some(viewer("mallory")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_single_key_admin_proceeds_against_any_agent() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let key_uri = format!("/api/memory/agents/{id}/kv/admin.visible");

    let seed_status = run(
        &h,
        req_with_body(
            Method::PUT,
            &key_uri,
            Some(viewer("alice")),
            Body::from(r#"{"value":"seeded-by-owner"}"#),
        ),
    )
    .await;
    assert_eq!(seed_status, StatusCode::OK);

    let (status, body) = run_json(&h, req(Method::GET, &key_uri, Some(admin("ops")))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["key"], "admin.visible");
    assert_eq!(body["value"], "seeded-by-owner");
}

#[tokio::test(flavor = "multi_thread")]
async fn single_key_owner_round_trip_is_ok() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let key_uri = format!("/api/memory/agents/{id}/kv/user.preference");

    let put = run(
        &h,
        req_with_body(
            Method::PUT,
            &key_uri,
            Some(viewer("alice")),
            Body::from(r#"{"value":{"theme":"dark"}}"#),
        ),
    )
    .await;
    assert_eq!(put, StatusCode::OK);

    let (get, body) = run_json(&h, req(Method::GET, &key_uri, Some(viewer("alice")))).await;
    assert_eq!(get, StatusCode::OK);
    assert_eq!(body["key"], "user.preference");
    assert_eq!(body["value"], serde_json::json!({"theme": "dark"}));

    let delete = run(&h, req(Method::DELETE, &key_uri, Some(viewer("alice")))).await;
    assert_eq!(delete, StatusCode::NO_CONTENT);

    let missing = run(&h, req(Method::GET, &key_uri, Some(viewer("alice")))).await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Bulk export / import — same omission as above; pin.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn export_viewer_other_author_is_404() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req(
            Method::GET,
            &format!("/api/agents/{id}/memory/export"),
            Some(viewer("mallory")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn import_viewer_other_author_is_404() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let status = run(
        &h,
        req_with_body(
            Method::POST,
            &format!("/api/agents/{id}/memory/import"),
            Some(viewer("mallory")),
            Body::from(r#"{"kv":{"k":"v"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_import_then_export_round_trip_is_ok() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");

    let (import_status, import_body) = run_json(
        &h,
        req_with_body(
            Method::POST,
            &format!("/api/agents/{id}/memory/import"),
            Some(viewer("alice")),
            Body::from(r#"{"kv":{"oncall.contact":"alice@example.com"}}"#),
        ),
    )
    .await;
    assert_eq!(import_status, StatusCode::OK);
    assert_eq!(import_body["status"], "imported");
    assert_eq!(import_body["keys_imported"], 1);

    let (export_status, export_body) = run_json(
        &h,
        req(
            Method::GET,
            &format!("/api/agents/{id}/memory/export"),
            Some(viewer("alice")),
        ),
    )
    .await;
    assert_eq!(export_status, StatusCode::OK);
    assert_eq!(export_body["agent_id"], id.to_string());
    assert_eq!(export_body["kv"]["oncall.contact"], "alice@example.com");
}

// ---------------------------------------------------------------------------
// Reserved `compaction_summary_` namespace (#8645): the per-conversation compaction mirrors are
// platform state the prompt path trusts, so the KV surface neither shows nor accepts them.
// ---------------------------------------------------------------------------

/// Seed a real compaction mirror the way the compactor does, and return its key.
fn seed_compaction_mirror(state: &Arc<AppState>, id: AgentId) -> String {
    let session = librefang_types::agent::SessionId::new();
    state
        .kernel
        .memory_substrate()
        .store_llm_summary(
            id,
            "private summary of one conversation",
            vec![librefang_types::message::Message::user("kept")],
            Some(session),
        )
        .expect("store_llm_summary");
    let key = librefang_memory::compaction_summary_key(&session);
    assert!(
        state
            .kernel
            .memory_substrate()
            .structured_get(id, &key)
            .unwrap()
            .is_some(),
        "the compactor must have written the mirror"
    );
    key
}

#[tokio::test(flavor = "multi_thread")]
async fn list_and_export_hide_compaction_mirrors() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let mirror_key = seed_compaction_mirror(&h.state, id);
    h.state
        .kernel
        .memory_substrate()
        .structured_set(id, "oncall.contact", serde_json::json!("alice"))
        .unwrap();

    let (status, body) = run_json(
        &h,
        req(
            Method::GET,
            &format!("/api/memory/agents/{id}/kv"),
            Some(viewer("alice")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let keys: Vec<&str> = body["kv_pairs"]
        .as_array()
        .expect("kv_pairs array")
        .iter()
        .map(|pair| pair["key"].as_str().unwrap())
        .collect();
    assert!(
        !keys.contains(&mirror_key.as_str()),
        "the mirror must not be listed: {keys:?}"
    );
    assert!(
        keys.contains(&"oncall.contact"),
        "agent keys stay listed: {keys:?}"
    );

    let (status, body) = run_json(
        &h,
        req(
            Method::GET,
            &format!("/api/agents/{id}/memory/export"),
            Some(viewer("alice")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let exported = body["kv"].as_object().expect("kv object");
    assert!(
        !exported.contains_key(&mirror_key),
        "the mirror must not be exported: {exported:?}"
    );
    assert_eq!(exported["oncall.contact"], "alice");
}

#[tokio::test(flavor = "multi_thread")]
async fn import_rejects_a_compaction_mirror_and_writes_nothing() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let planted = format!(
        "compaction_summary_{}",
        librefang_types::agent::SessionId::new().0
    );
    let body = serde_json::json!({
        "kv": { "oncall.contact": "alice", planted.clone(): "planted summary" },
    });

    let status = run(
        &h,
        req_with_body(
            Method::POST,
            &format!("/api/agents/{id}/memory/import"),
            Some(viewer("alice")),
            Body::from(body.to_string()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let substrate = h.state.kernel.memory_substrate();
    assert!(
        substrate.structured_get(id, &planted).unwrap().is_none(),
        "a rejected import must not plant the mirror"
    );
    assert!(
        substrate
            .structured_get(id, "oncall.contact")
            .unwrap()
            .is_none(),
        "a rejected import must not write its other keys either"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn import_with_clear_keeps_compaction_mirrors() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let mirror_key = seed_compaction_mirror(&h.state, id);

    let (status, body) = run_json(
        &h,
        req_with_body(
            Method::POST,
            &format!("/api/agents/{id}/memory/import"),
            Some(viewer("alice")),
            Body::from(r#"{"kv":{"oncall.contact":"alice"},"clear_existing":true}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "imported");
    assert!(
        h.state
            .kernel
            .memory_substrate()
            .structured_get(id, &mirror_key)
            .unwrap()
            .is_some(),
        "clearing the KV surface must not delete state it does not expose"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn put_single_key_rejects_a_compaction_mirror() {
    let h = boot();
    let id = spawn_owned_by(&h.state, "owned-by-alice", "alice");
    let planted = format!(
        "compaction_summary_{}",
        librefang_types::agent::SessionId::new().0
    );
    let status = run(
        &h,
        req_with_body(
            Method::PUT,
            &format!("/api/memory/agents/{id}/kv/{planted}"),
            Some(admin("ops")),
            Body::from(r#"{"value":"planted summary"}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(h
        .state
        .kernel
        .memory_substrate()
        .structured_get(id, &planted)
        .unwrap()
        .is_none());
}
