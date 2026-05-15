//! Integration tests for declarative `[[triggers]]` in `agent.toml` (#5014).
//!
//! Covers:
//! 1. `manifest_triggers_register_on_spawn` — spawning an agent with
//!    `manifest.triggers` populated registers each entry with the runtime
//!    `TriggerEngine`, visible via `GET /api/triggers?agent_id=...`.
//! 2. `manifest_triggers_reconcile_is_idempotent_across_spawns` — a second
//!    spawn (simulated by re-running the kernel reconcile via reload) with
//!    the same manifest list neither duplicates nor disturbs the
//!    runtime store.
//! 3. `manifest_triggers_orphan_keep_preserves_api_created` — API-created
//!    triggers survive a reload when `reconcile_orphans = "keep"` (the
//!    default), even when the manifest declares a different trigger.
//! 4. `manifest_triggers_orphan_delete_reaps_api_created` — same setup
//!    with `reconcile_orphans = "delete"` reaps the API-created trigger.
//! 5. `manifest_triggers_update_in_place_toml_wins` — declaratively
//!    declared trigger whose fields drift on reload (max_fires bumped,
//!    enabled flipped) updates in place.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, ManifestTrigger, OrphanPolicy, SessionMode};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness mirrors trigger_workflow_test.rs (#4844): same nested router, same
// `TestAppState` over the mock kernel so all axum/handler plumbing is real.
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

fn manifest_with_triggers(name: &str, triggers: Vec<ManifestTrigger>) -> AgentManifest {
    AgentManifest {
        name: name.to_string(),
        triggers,
        ..AgentManifest::default()
    }
}

/// Helper: spawn an agent with the given manifest and return its id.
fn spawn(state: &Arc<AppState>, manifest: AgentManifest) -> AgentId {
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent_typed must succeed in test kernel")
}

/// Helper: list triggers for an agent via the HTTP API and return the
/// parsed entries.
async fn list_triggers(h: &Harness, agent_id: AgentId) -> Vec<serde_json::Value> {
    let (status, body) = json_request(
        h,
        Method::GET,
        &format!("/api/triggers?agent_id={agent_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list_triggers failed: {body:?}");
    body["triggers"].as_array().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Test 1: spawn registers manifest triggers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn manifest_triggers_register_on_spawn() {
    let h = boot().await;

    let manifest = manifest_with_triggers(
        &format!("decl-trig-{}", uuid::Uuid::new_v4()),
        vec![
            ManifestTrigger {
                pattern: serde_json::json!({ "task_posted": {} }),
                prompt_template: "task: {{event}}".to_string(),
                max_fires: 0,
                cooldown_secs: 30,
                session_mode: Some(SessionMode::New),
                target_agent: None,
                workflow_id: None,
                enabled: true,
            },
            ManifestTrigger {
                pattern: serde_json::json!({ "content_match": { "substring": "deploy" } }),
                prompt_template: "deploy mention: {{event}}".to_string(),
                max_fires: 5,
                cooldown_secs: 0,
                session_mode: None,
                target_agent: None,
                workflow_id: None,
                enabled: false,
            },
        ],
    );
    let agent_id = spawn(&h.state, manifest);

    let triggers = list_triggers(&h, agent_id).await;
    assert_eq!(triggers.len(), 2, "expected 2 declarative triggers");

    // Find each by prompt_template — order is non-deterministic across DashMap.
    let task_trigger = triggers
        .iter()
        .find(|t| t["prompt_template"] == "task: {{event}}")
        .expect("task_posted trigger must be registered");
    assert_eq!(task_trigger["max_fires"], 0);
    assert_eq!(task_trigger["cooldown_secs"], 30);
    assert_eq!(task_trigger["session_mode"], "new");
    assert_eq!(task_trigger["enabled"], true);

    let deploy_trigger = triggers
        .iter()
        .find(|t| t["prompt_template"] == "deploy mention: {{event}}")
        .expect("content_match trigger must be registered");
    assert_eq!(deploy_trigger["max_fires"], 5);
    assert_eq!(deploy_trigger["enabled"], false);
}

// ---------------------------------------------------------------------------
// Test 2: reconcile is idempotent across reloads
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn manifest_triggers_reconcile_is_idempotent_across_reloads() {
    let h = boot().await;

    let manifest = manifest_with_triggers(
        &format!("idem-{}", uuid::Uuid::new_v4()),
        vec![ManifestTrigger {
            pattern: serde_json::json!({ "task_posted": {} }),
            prompt_template: "task: {{event}}".to_string(),
            max_fires: 0,
            cooldown_secs: 0,
            session_mode: None,
            target_agent: None,
            workflow_id: None,
            enabled: true,
        }],
    );
    let agent_id = spawn(&h.state, manifest.clone());

    let before = list_triggers(&h, agent_id).await;
    let before_id = before[0]["id"]
        .as_str()
        .expect("id must be a string")
        .to_string();
    assert_eq!(before.len(), 1);

    // Re-running reconcile with the same manifest must NOT create a new
    // trigger. We exercise reconcile directly through the kernel handle
    // because the mock kernel does not own an `agent.toml` on disk
    // (reload_agent_from_disk relies on a real path) — the engine call
    // is the same code path the spawn / reload sites both invoke.
    let kernel = h.state.kernel.clone();
    let engine = kernel.trigger_engine();
    let report = engine.reconcile_manifest_triggers(
        agent_id,
        &manifest.triggers,
        OrphanPolicy::Keep,
        |_| None,
    );
    assert!(!report.mutated(), "idempotent reconcile must not mutate");

    let after = list_triggers(&h, agent_id).await;
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0]["id"].as_str().unwrap(),
        before_id,
        "trigger id must stay stable across reconciles"
    );
}

// ---------------------------------------------------------------------------
// Test 3: orphan policy Keep preserves API-created triggers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn manifest_triggers_orphan_keep_preserves_api_created() {
    let h = boot().await;

    // Spawn an agent with one declarative trigger.
    let mut manifest = manifest_with_triggers(
        &format!("keep-{}", uuid::Uuid::new_v4()),
        vec![ManifestTrigger {
            pattern: serde_json::json!({ "task_posted": {} }),
            prompt_template: "manifest task: {{event}}".to_string(),
            ..Default::default()
        }],
    );
    manifest.reconcile_orphans = OrphanPolicy::Keep;
    let agent_id = spawn(&h.state, manifest.clone());

    // Add a runtime trigger via the API.
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "pattern": "task_posted",
            "prompt_template": "api-created task: {{event}}",
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "POST /api/triggers failed: {body:?}"
    );

    // Confirm both are visible.
    let before = list_triggers(&h, agent_id).await;
    assert_eq!(before.len(), 2);

    // Run reconcile again with the same manifest. The API-created trigger
    // is an orphan but Keep policy must preserve it.
    let report = h.state.kernel.trigger_engine().reconcile_manifest_triggers(
        agent_id,
        &manifest.triggers,
        OrphanPolicy::Keep,
        |_| None,
    );
    assert_eq!(report.deleted, 0);
    assert_eq!(report.orphans_kept, 1);

    let after = list_triggers(&h, agent_id).await;
    assert_eq!(after.len(), 2, "Keep policy must not remove orphans");
}

// ---------------------------------------------------------------------------
// Test 4: orphan policy Delete reaps API-created triggers
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn manifest_triggers_orphan_delete_reaps_api_created() {
    let h = boot().await;

    let mut manifest = manifest_with_triggers(
        &format!("delete-{}", uuid::Uuid::new_v4()),
        vec![ManifestTrigger {
            pattern: serde_json::json!({ "task_posted": {} }),
            prompt_template: "manifest task: {{event}}".to_string(),
            ..Default::default()
        }],
    );
    manifest.reconcile_orphans = OrphanPolicy::Delete;
    let agent_id = spawn(&h.state, manifest.clone());

    // Add a runtime trigger via the API.
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/triggers",
        Some(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "pattern": "task_posted",
            "prompt_template": "api-created task: {{event}}",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let before = list_triggers(&h, agent_id).await;
    assert_eq!(before.len(), 2);

    // Re-reconcile with Delete policy — the API-created orphan must be
    // removed, the manifest entry stays.
    let report = h.state.kernel.trigger_engine().reconcile_manifest_triggers(
        agent_id,
        &manifest.triggers,
        OrphanPolicy::Delete,
        |_| None,
    );
    assert_eq!(report.deleted, 1);
    assert!(report.mutated());

    let after = list_triggers(&h, agent_id).await;
    assert_eq!(
        after.len(),
        1,
        "Delete policy must remove every orphan; got {after:?}"
    );
    assert_eq!(after[0]["prompt_template"], "manifest task: {{event}}");
}

// ---------------------------------------------------------------------------
// Test 5: drift in manifest fields updates the existing trigger in place
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn manifest_triggers_update_in_place_toml_wins() {
    let h = boot().await;

    let mut manifest = manifest_with_triggers(
        &format!("update-{}", uuid::Uuid::new_v4()),
        vec![ManifestTrigger {
            pattern: serde_json::json!({ "task_posted": {} }),
            prompt_template: "task: {{event}}".to_string(),
            max_fires: 0,
            cooldown_secs: 0,
            session_mode: None,
            target_agent: None,
            workflow_id: None,
            enabled: true,
        }],
    );
    let agent_id = spawn(&h.state, manifest.clone());

    let before = list_triggers(&h, agent_id).await;
    assert_eq!(before.len(), 1);
    let before_id = before[0]["id"].as_str().unwrap().to_string();

    // Operator edits agent.toml: max_fires bumped to 5, cooldown set,
    // disabled. Re-running reconcile picks the new values up.
    manifest.triggers[0].max_fires = 5;
    manifest.triggers[0].cooldown_secs = 30;
    manifest.triggers[0].enabled = false;

    let report = h.state.kernel.trigger_engine().reconcile_manifest_triggers(
        agent_id,
        &manifest.triggers,
        OrphanPolicy::Keep,
        |_| None,
    );
    assert_eq!(report.created, 0);
    assert_eq!(report.updated, 1);

    let after = list_triggers(&h, agent_id).await;
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0]["id"].as_str().unwrap(),
        before_id,
        "in-place update must keep the same trigger id"
    );
    assert_eq!(after[0]["max_fires"], 5);
    assert_eq!(after[0]["cooldown_secs"], 30);
    assert_eq!(after[0]["enabled"], false);
}
