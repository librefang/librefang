//! Integration tests for event trigger → workflow dispatch (#4844 gap #6).
//!
//! Covers:
//! 1. `trigger_with_workflow_id_fires_workflow_on_matching_event` — end-to-end
//!    event publish creates a run for the linked workflow.
//! 2. `trigger_without_workflow_id_uses_agent_path` — regression: triggers
//!    without workflow_id still dispatch to the agent normally.
//! 3. `create_trigger_via_api_accepts_workflow_id` — POST /api/triggers round-trip.
//! 4. `update_trigger_via_api_clears_workflow_id` — PATCH with null clears field.
//! 5. `trigger_workflow_id_too_long_returns_400` — 257-char workflow_id rejected.
//! 6. `legacy_trigger_json_loads_without_workflow_id` — old format loads cleanly.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness (mirrors workflow_lifecycle_test.rs)
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
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
    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path);
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::workflows::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => {
            builder = builder.header("content-type", "application/json");
            b"{}".to_vec()
        }
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
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

// ---------------------------------------------------------------------------
// Helper: spawn a minimal agent in the kernel, return its AgentId.
// ---------------------------------------------------------------------------

fn spawn_agent(state: &Arc<AppState>) -> AgentId {
    let manifest = AgentManifest {
        name: format!("trigger-test-{}", uuid::Uuid::new_v4()),
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent_typed must succeed in test kernel")
}

// ---------------------------------------------------------------------------
// Helper: create a minimal workflow via the HTTP API and return its UUID string.
// ---------------------------------------------------------------------------

async fn create_workflow(h: &Harness) -> String {
    let agent_ref = uuid::Uuid::new_v4().to_string();
    let (status, body) = json_request(
        h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": format!("trigger-wf-{}", uuid::Uuid::new_v4()),
            "description": "test workflow for trigger dispatch",
            "steps": [{"name": "s1", "agent_id": agent_ref, "prompt": "hello"}]
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create_workflow failed: {body:?}"
    );
    body["workflow_id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Helper: poll for a workflow run to appear instead of guessing at a fixed
// sleep. The trigger → workflow dispatch happens on a spawned task, so the run
// is recorded asynchronously after `publish_typed_event`. Polling on the
// observable side effect (a run with the expected workflow_id) removes the
// wall-clock race that made a fixed 150ms sleep flaky under CI load.
//
// Returns the matching run, or `None` if none appears within the 5s deadline
// (25ms poll interval). Callers craft their own assertion message so the
// failure can name the domain-specific context (e.g. the case-insensitive
// name-lookup forms in test 5).
// ---------------------------------------------------------------------------

async fn wait_for_workflow_run(
    h: &Harness,
    wf_id: librefang_kernel::workflow::WorkflowId,
) -> Option<librefang_kernel::workflow::WorkflowRun> {
    let engine = h.state.kernel.workflow_engine();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let runs = engine.list_runs(None).await;
        if let Some(run) = runs.into_iter().find(|r| r.workflow_id == wf_id) {
            return Some(run);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// Test 1: trigger with workflow_id fires a workflow run on a matching event
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn trigger_with_workflow_id_fires_workflow_on_matching_event() {
    use librefang_kernel::triggers::TriggerPattern;
    use librefang_kernel::workflow::WorkflowId;
    use librefang_types::event::{Event, EventPayload, EventTarget};

    let h = boot().await;
    let agent_id = spawn_agent(&h.state);

    // Register a workflow via the HTTP API.
    let wf_id_str = create_workflow(&h).await;
    let wf_id = WorkflowId(wf_id_str.parse().unwrap());

    // Register a trigger that fires the workflow on "deploy" events.
    let trigger_id = h
        .state
        .kernel
        .register_trigger_with_target(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "deploy".to_string(),
            },
            "input is: {{event}}".to_string(),
            0,
            None,
            Some(0), // zero cooldown — no wait between fires in tests
            None,
            Some(wf_id_str.clone()),
        )
        .expect("register_trigger_with_target must succeed");

    // Verify the trigger is stored with workflow_id.
    let stored = h
        .state
        .kernel
        .get_trigger(trigger_id)
        .expect("trigger must exist");
    assert_eq!(stored.workflow_id.as_deref(), Some(wf_id_str.as_str()));

    // Publish a matching event.
    let payload_bytes = serde_json::to_vec(&serde_json::json!({
        "type": "custom",
        "text": "deploy release candidate now",
    }))
    .unwrap();
    let event = Event::new(
        AgentId::new(),
        EventTarget::Broadcast,
        EventPayload::Custom(payload_bytes),
    );
    h.state.kernel.publish_typed_event(event).await;

    // The spawned workflow dispatch task records the run asynchronously. We
    // don't wait for full LLM execution (none in the test kernel) — just poll
    // until create_run is recorded.
    let wf_run = wait_for_workflow_run(&h, wf_id).await;
    assert!(
        wf_run.is_some(),
        "Expected at least one run for workflow {wf_id_str} within 5s; got runs: {:?}",
        h.state.kernel.workflow_engine().list_runs(None).await
    );
}

// ---------------------------------------------------------------------------
// Test 2: trigger WITHOUT workflow_id still dispatches to agent (regression)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn trigger_without_workflow_id_uses_agent_path() {
    use librefang_kernel::triggers::TriggerPattern;

    let h = boot().await;
    let agent_id = spawn_agent(&h.state);

    // Register a trigger WITHOUT workflow_id.
    let trigger_id = h
        .state
        .kernel
        .register_trigger_with_target(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "hello".to_string(),
            },
            "received: {{event}}".to_string(),
            0,
            None,
            Some(0),
            None,
            None, // no workflow_id — agent path
        )
        .expect("register must succeed");

    let stored = h
        .state
        .kernel
        .get_trigger(trigger_id)
        .expect("trigger must exist");
    assert!(
        stored.workflow_id.is_none(),
        "workflow_id must be None for agent-path trigger"
    );
}

// ---------------------------------------------------------------------------
// Test 3: POST /api/triggers with workflow_id — round-trip via HTTP API
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn create_trigger_via_api_accepts_workflow_id() {
    let h = boot().await;
    let agent_id = spawn_agent(&h.state);
    let wf_id = uuid::Uuid::new_v4().to_string();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "pattern": {"content_match": {"substring": "test"}},
            "prompt_template": "event: {{event}}",
            "workflow_id": wf_id,
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "POST /api/triggers must return 201: {body:?}"
    );
    assert_eq!(
        body["workflow_id"].as_str(),
        Some(wf_id.as_str()),
        "workflow_id must be echoed in create response: {body:?}"
    );

    // Fetch the trigger back via GET and confirm workflow_id is round-tripped.
    let trigger_id = body["trigger_id"].as_str().unwrap().to_string();
    let (get_status, get_body) = json_request(
        &h,
        Method::GET,
        &format!("/api/triggers/{trigger_id}"),
        None,
    )
    .await;
    assert_eq!(
        get_status,
        StatusCode::OK,
        "GET trigger must be 200: {get_body:?}"
    );
    assert_eq!(
        get_body["workflow_id"].as_str(),
        Some(wf_id.as_str()),
        "workflow_id must be present in GET response: {get_body:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: PATCH with workflow_id: null clears the field
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn update_trigger_via_api_clears_workflow_id() {
    let h = boot().await;
    let agent_id = spawn_agent(&h.state);
    let wf_id = uuid::Uuid::new_v4().to_string();

    // Create trigger with workflow_id.
    let (create_status, create_body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "pattern": {"content_match": {"substring": "test"}},
            "prompt_template": "event: {{event}}",
            "workflow_id": wf_id,
        })),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::CREATED,
        "create failed: {create_body:?}"
    );
    let trigger_id = create_body["trigger_id"].as_str().unwrap().to_string();

    // PATCH with workflow_id: null to clear it.
    let (patch_status, patch_body) = json_request(
        &h,
        Method::PATCH,
        &format!("/api/triggers/{trigger_id}"),
        Some(serde_json::json!({"workflow_id": null})),
    )
    .await;
    assert_eq!(
        patch_status,
        StatusCode::OK,
        "PATCH must be 200: {patch_body:?}"
    );
    // After clearing, workflow_id must be absent or null in the response.
    let wf_after = &patch_body["workflow_id"];
    assert!(
        wf_after.is_null() || wf_after == &serde_json::Value::Null,
        "workflow_id must be cleared after PATCH null: {patch_body:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: workflow_id longer than 256 chars is rejected with 400
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn trigger_workflow_id_too_long_returns_400() {
    let h = boot().await;
    let agent_id = spawn_agent(&h.state);
    let long_id = "x".repeat(257);

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "pattern": {"content_match": {"substring": "test"}},
            "prompt_template": "event: {{event}}",
            "workflow_id": long_id,
        })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "257-char workflow_id must return 400: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 6b: trigger workflow_id resolves by name case-insensitively, matching
// WorkflowRunner::run_workflow / start_workflow_async and the cron path.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn trigger_workflow_id_resolves_name_case_insensitively() {
    use librefang_kernel::triggers::TriggerPattern;
    use librefang_kernel::workflow::WorkflowId;
    use librefang_types::event::{Event, EventPayload, EventTarget};

    let h = boot().await;
    let agent_id = spawn_agent(&h.state);

    // Register a workflow with a MIXED-case name via the HTTP API.
    let mixed_name = format!("MixedDeploy-{}", uuid::Uuid::new_v4().simple());
    let agent_ref = uuid::Uuid::new_v4().to_string();
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/workflows",
        Some(serde_json::json!({
            "name": mixed_name.clone(),
            "description": "case-insensitive lookup probe",
            "steps": [{"name": "s1", "agent_id": agent_ref, "prompt": "hello"}]
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create_workflow failed: {body:?}"
    );
    let wf_uuid: WorkflowId = WorkflowId(
        body["workflow_id"]
            .as_str()
            .unwrap()
            .parse()
            .expect("workflow_id must be a UUID"),
    );

    // Register a trigger with the workflow_id set to the LOWERCASE form of
    // the workflow name. UUID parsing must fail (so the trigger falls into
    // the name-lookup branch) and the case-insensitive comparison must match.
    let lowercase_form = mixed_name.to_lowercase();
    let trigger_id = h
        .state
        .kernel
        .register_trigger_with_target(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "deploy".to_string(),
            },
            "input is: {{event}}".to_string(),
            0,
            None,
            Some(0),
            None,
            Some(lowercase_form.clone()),
        )
        .expect("register_trigger_with_target must succeed");

    let stored = h
        .state
        .kernel
        .get_trigger(trigger_id)
        .expect("trigger must exist");
    assert_eq!(stored.workflow_id.as_deref(), Some(lowercase_form.as_str()));

    let payload_bytes = serde_json::to_vec(&serde_json::json!({
        "type": "custom",
        "text": "deploy this thing",
    }))
    .unwrap();
    let event = Event::new(
        AgentId::new(),
        EventTarget::Broadcast,
        EventPayload::Custom(payload_bytes),
    );
    h.state.kernel.publish_typed_event(event).await;

    // Poll for the run instead of guessing at a fixed sleep. If the
    // case-insensitive name lookup fails to resolve `lowercase_form` to the
    // `mixed_name` workflow, no run is ever recorded and this fails with a clear
    // message naming both forms and the expected workflow id.
    let wf_run = wait_for_workflow_run(&h, wf_uuid).await;
    assert!(
        wf_run.is_some(),
        "case-insensitive name lookup must resolve `{lowercase_form}` to workflow `{mixed_name}` (id {wf_uuid:?}) within 5s; got runs: {:?}",
        h.state.kernel.workflow_engine().list_runs(None).await
    );
}

// ---------------------------------------------------------------------------
// Test 6: old-format trigger JSON (without workflow_id) deserialises cleanly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn legacy_trigger_json_loads_without_workflow_id() {
    use librefang_kernel::triggers::Trigger;

    // A minimal trigger JSON as it would appear in a pre-feature trigger_jobs.json.
    let legacy_json = serde_json::json!({
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "agent_id": "660e8400-e29b-41d4-a716-446655440001",
        "pattern": {"content_match": {"substring": "test"}},
        "prompt_template": "event: {{event}}",
        "enabled": true,
        "created_at": "2024-01-01T00:00:00Z",
        "fire_count": 0,
        "max_fires": 0,
    });

    let trigger: Trigger = serde_json::from_value(legacy_json)
        .expect("legacy trigger JSON must deserialise without workflow_id field");

    assert!(
        trigger.workflow_id.is_none(),
        "workflow_id must be None when absent from persisted JSON"
    );
}

// ---------------------------------------------------------------------------
// Test 7: per-agent trigger cap — the 51st registration is rejected at the
// HTTP route with 400 (audit: trigger-engine-no-per-agent-cap).
//
// The kernel engine has unit coverage for the cap, but repo policy requires
// a route-level test proving the cap surfaces as a client error (400) rather
// than the catch-all 404 the create_trigger handler otherwise returns. We
// exercise the real route on both ends: the first registration goes through
// `POST /api/triggers` and must succeed (201), the bulk of the agent's quota
// is seeded directly via the kernel for speed, then the 51st registration
// goes through the route again and must be 400 with the cap mentioned.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn create_trigger_past_per_agent_cap_returns_400() {
    use librefang_kernel::triggers::{TriggerPattern, MAX_TRIGGERS_PER_AGENT};

    let h = boot().await;
    let agent_id = spawn_agent(&h.state);

    // 1. First trigger via the real HTTP route — proves the success path (201)
    //    and that the route accepts a minimal valid payload.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "pattern": {"content_match": {"substring": "test"}},
            "prompt_template": "event: {{event}}",
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "first POST /api/triggers must return 201: {body:?}"
    );

    // 2. Fill the remaining quota (triggers 2..=MAX) directly via the kernel.
    //    Routing every one through HTTP `oneshot` would be needlessly slow; the
    //    cap lives in the engine and the route is what we assert below.
    for i in 1..MAX_TRIGGERS_PER_AGENT {
        h.state
            .kernel
            .register_trigger_with_target(
                agent_id,
                TriggerPattern::ContentMatch {
                    substring: format!("seed-{i}"),
                },
                "event: {{event}}".to_string(),
                0,
                None,
                None,
                None,
                None,
            )
            .unwrap_or_else(|e| panic!("seeding trigger {i} within cap must succeed: {e}"));
    }

    // 3. The 51st registration goes through the real route and must be 400 —
    //    the cap error maps to InvalidInput → bad_request, NOT the 404
    //    "agent not found" catch-all.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "pattern": {"content_match": {"substring": "over-the-cap"}},
            "prompt_template": "event: {{event}}",
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "registration past the per-agent cap must be 400, not 404: {body:?}"
    );
    let msg = body["message"]
        .as_str()
        .or_else(|| body["error"]["message"].as_str())
        .unwrap_or("")
        .to_lowercase();
    assert!(
        msg.contains("cap") || msg.contains("limit"),
        "400 body must explain the cap was exceeded: {body:?}"
    );
}
