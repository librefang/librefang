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
use librefang_kernel::skill_workshop::candidate::{
    CandidateKind, CandidateSkill, CaptureSource, Provenance,
};
use librefang_kernel::skill_workshop::storage;
use librefang_kernel::AgentSubsystemApi;
use librefang_kernel::MemorySubsystemApi;
use librefang_kernel::SkillsSubsystemApi;
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
            extra_params: std::collections::BTreeMap::new(),
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
        kind: CandidateKind::Create,
        target_skill_id: None,
        current_version: None,
        proposed_version: None,
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
    // Names must differ across the two seeds because the dedup check
    // (same source kind + same name → skip) would otherwise drop the
    // second save and break the assertion that both ids appear in the
    // listing.
    let mut a = fixture_candidate(agent, id_a);
    a.name = "fmt_before_commit_a".to_string();
    let mut b = fixture_candidate(agent, id_b);
    b.name = "fmt_before_commit_b".to_string();
    storage::save_candidate(&root, &a, 20, None).unwrap();
    storage::save_candidate(&root, &b, 20, None).unwrap();

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
        None,
    )
    .unwrap();
    storage::save_candidate(
        &root,
        &fixture_candidate(agent_b, "bbbbbbbb-0000-0000-0000-000000000002"),
        20,
        None,
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
    storage::save_candidate(&skills_root(&h), &fixture_candidate(agent, id), 20, None).unwrap();

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
    storage::save_candidate(&root, &fixture_candidate(agent, id), 20, None).unwrap();

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

/// Phantom-pending recovery: when the active skill already exists (e.g.
/// a previous approve promoted the candidate but the pending-file
/// cleanup failed for transient reasons), a re-approve must idempotently
/// drop the pending row and return 200 instead of 409-ing forever — the
/// reviewer otherwise has no UI action to clear the entry.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_already_installed_clears_pending_and_returns_200() {
    let h = boot().await;
    let agent = "11111111-1111-1111-1111-111111111111";
    let first_id = "abababab-0000-0000-0000-000000000001";
    let second_id = "abababab-0000-0000-0000-000000000002";
    let root = skills_root(&h);

    // First approve plants the active skill and clears its pending file.
    storage::save_candidate(&root, &fixture_candidate(agent, first_id), 20, None).unwrap();
    let (status, _) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{first_id}/approve"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first approve must succeed");
    assert!(
        root.join("fmt_before_commit").join("skill.toml").exists(),
        "active skill seeded"
    );

    // Re-stage a pending file with the same skill name to mimic the
    // phantom case (write succeeded; cleanup never ran).
    storage::save_candidate(&root, &fixture_candidate(agent, second_id), 20, None).unwrap();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{second_id}/approve"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "phantom-pending must clear: {body:?}"
    );
    assert_eq!(body["status"], "already_promoted");
    assert_eq!(body["candidate_id"], second_id);
    assert_eq!(body["skill_name"], "fmt_before_commit");
    assert!(
        storage::load_candidate(&root, second_id).is_err(),
        "phantom pending file must be removed after recovery"
    );
}

/// Name collision (NOT a phantom): the user already has an unrelated
/// active skill with the same name (manual install / marketplace /
/// prior `evolve` / `synth_name` fallback collision). A re-approve must
/// NOT silently drop the pending row — the reviewer would lose the
/// candidate they actually wanted promoted. Returns 409 with
/// `kind: "name_collision"` and keeps the pending file so the user can
/// rename and retry.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_name_collision_with_different_body_returns_409() {
    let h = boot().await;
    let agent = "11111111-1111-1111-1111-111111111111";
    let id = "abababab-0000-0000-0000-000000000005";
    let root = skills_root(&h);

    // Plant an unrelated active skill named `fmt_before_commit` whose
    // body is NOT what the workshop captured — emulates the user
    // having installed / written this skill via another path.
    let active_dir = root.join("fmt_before_commit");
    std::fs::create_dir_all(&active_dir).unwrap();
    std::fs::write(
        active_dir.join("skill.toml"),
        "name = \"fmt_before_commit\"\n\
         description = \"a totally different rule\"\n\
         version = \"1.0.0\"\n\
         allowed_tools = []\n\
         is_workspace_skill = false\n\
         allow_hot_reload = true\n",
    )
    .unwrap();
    std::fs::write(
        active_dir.join("prompt_context.md"),
        "# A totally different rule\n\nThis is NOT what the candidate carries.\n",
    )
    .unwrap();

    // Stage a pending candidate with the standard fixture body — body
    // differs from the planted active skill above.
    storage::save_candidate(&root, &fixture_candidate(agent, id), 20, None).unwrap();
    let pending_path = root.join("pending").join(agent).join(format!("{id}.toml"));
    assert!(pending_path.exists(), "pending file must be staged");

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{id}/approve"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "real name collision must surface as 409, not silent 200: {body:?}"
    );
    assert_eq!(body["kind"], "name_collision");
    assert_eq!(body["skill_name"], "fmt_before_commit");
    assert_eq!(body["candidate_id"], id);
    // Critical invariant: the pending file MUST survive a 409 collision
    // so the reviewer can rename and retry — silently dropping it would
    // be a data-loss bug.
    assert!(
        pending_path.exists(),
        "pending file must NOT be dropped on a real name collision"
    );
    assert!(
        storage::load_candidate(&root, id).is_ok(),
        "candidate must still be loadable after a collision response"
    );
}

// ---------------------------------------------------------------------------
// Auto-assign promoted skill to the creating agent (#5989)
// ---------------------------------------------------------------------------

/// Register a real agent in the harness's registry so the approve path
/// can resolve `CandidateSkill.agent_id` and append the promoted skill to
/// its allowlist. Returns the agent's id as the canonical UUID string the
/// candidate fixture must carry.
fn register_agent(h: &Harness, name: &str) -> String {
    use librefang_types::agent::{AgentEntry, AgentId};
    let agent_id = AgentId::new();
    let manifest = librefang_types::agent::AgentManifest {
        name: name.to_string(),
        // Start with a NON-EMPTY allowlist so the append is observable.
        // An empty allowlist means "all skills", and the approve path
        // (kernel `assign_skill_to_agent_allowlist`) deliberately leaves
        // an all-skills agent untouched rather than narrowing it to the
        // single promoted skill — so a pre-existing entry is required for
        // the append to be a real, non-destructive addition.
        skills: vec!["pre_existing_skill".to_string()],
        skills_disabled: false,
        ..Default::default()
    };
    let entry = AgentEntry {
        id: agent_id,
        name: name.to_string(),
        manifest,
        ..Default::default()
    };
    h.state
        .kernel
        .agent_registry()
        .register(entry)
        .expect("register agent");
    agent_id.0.to_string()
}

fn agent_skills(h: &Harness, agent_id: &str) -> (Vec<String>, bool) {
    let id: librefang_types::agent::AgentId = agent_id.parse().expect("parse agent id");
    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(id)
        .expect("agent must still be registered");
    (
        entry.manifest.skills.clone(),
        entry.manifest.skills_disabled,
    )
}

/// Approving a candidate whose `agent_id` resolves to a live agent must
/// append the promoted skill to that agent's allowlist and clear
/// `skills_disabled`, so the workshop loop closes without a manual
/// `agent.toml` edit.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_assigns_skill_to_creating_agent() {
    let h = boot().await;
    let agent = register_agent(&h, "creator_agent");
    let id = "f1f1f1f1-0000-0000-0000-000000000001";
    let root = skills_root(&h);
    storage::save_candidate(&root, &fixture_candidate(&agent, id), 20, None).unwrap();

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{id}/approve"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["skill_name"], "fmt_before_commit");

    let (skills, disabled) = agent_skills(&h, &agent);
    assert!(
        skills.contains(&"fmt_before_commit".to_string()),
        "promoted skill must be appended to the creating agent's allowlist: {skills:?}"
    );
    assert!(
        !disabled,
        "skills_disabled must be cleared so the new skill is live"
    );
}

/// Re-approving the same skill for the same agent (phantom-pending
/// recovery → `already_promoted`) must be idempotent: the skill appears
/// exactly once on the allowlist, no duplicate entry.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_assignment_is_idempotent() {
    let h = boot().await;
    let agent = register_agent(&h, "creator_agent");
    let first_id = "f2f2f2f2-0000-0000-0000-000000000001";
    let second_id = "f2f2f2f2-0000-0000-0000-000000000002";
    let root = skills_root(&h);

    storage::save_candidate(&root, &fixture_candidate(&agent, first_id), 20, None).unwrap();
    let (status, _) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{first_id}/approve"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first approve must succeed");

    // Re-stage a same-named candidate to mimic the phantom-pending case
    // (active skill already exists; pending re-approved). The assignment
    // must not double-append the skill name.
    storage::save_candidate(&root, &fixture_candidate(&agent, second_id), 20, None).unwrap();
    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{second_id}/approve"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "re-approve must succeed: {body:?}");

    let (skills, _) = agent_skills(&h, &agent);
    let occurrences = skills.iter().filter(|s| *s == "fmt_before_commit").count();
    assert_eq!(
        occurrences, 1,
        "re-approval must not duplicate the skill on the allowlist: {skills:?}"
    );
}

/// Auto-assign is best-effort: when the creating agent was deleted
/// between capture and approval (its `agent_id` no longer resolves), the
/// approve must still promote the skill and return 200 — assignment is a
/// convenience, not a hard precondition.
#[tokio::test(flavor = "multi_thread")]
async fn pending_approve_succeeds_when_creating_agent_gone() {
    let h = boot().await;
    // A well-formed UUID that was never registered in the harness.
    let agent = "deadbeef-0000-0000-0000-000000000000";
    let id = "f3f3f3f3-0000-0000-0000-000000000001";
    let root = skills_root(&h);
    storage::save_candidate(&root, &fixture_candidate(agent, id), 20, None).unwrap();

    let (status, body) = json_request(
        &h,
        Method::POST,
        &format!("/api/skills/pending/{id}/approve"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "approve must succeed even when the creating agent is gone: {body:?}"
    );
    assert_eq!(body["status"], "approved");
    assert!(
        root.join("fmt_before_commit").join("skill.toml").exists(),
        "skill must still be promoted when the agent can't be resolved"
    );
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
    storage::save_candidate(&root, &fixture_candidate(agent, id), 20, None).unwrap();
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

// ---------------------------------------------------------------------------
// ApprovalPolicy::Auto end-to-end
// ---------------------------------------------------------------------------

/// Auto-policy hits go through `save → approve → reload_skills` without
/// human review. This test boots a real kernel, registers an agent with
/// `[skill_workshop] approval_policy = "auto"`, plants a session
/// containing a canonical `from now on …` user message, and runs the
/// workshop's `run_capture` pipeline directly. Asserts the resulting
/// active skill landed at `<skills_root>/<name>/skill.toml` (which only
/// happens when `evolution::create_skill` ran end-to-end after the
/// pending stage). The `kernel.reload_skills()` call inside the auto
/// branch is `.await`-ed so the assertion is deterministic.
#[tokio::test(flavor = "multi_thread")]
async fn auto_policy_promotes_to_active_and_reloads_registry() {
    use librefang_kernel::skill_workshop;
    use librefang_memory::session::Session;
    use librefang_types::agent::{
        AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, ApprovalPolicy, EvolutionMode,
        ReviewMode, SessionId, SkillWorkshopConfig,
    };
    use librefang_types::message::Message;

    // Direct `MockKernelBuilder::build()` rather than TestAppState —
    // `run_capture` takes a concrete `Arc<LibreFangKernel>` (it pokes
    // at private kernel internals that aren't in the `KernelApi`
    // trait), and TestAppState would only hand us `Arc<dyn KernelApi>`.
    // Hold the TempDir for the duration so the temp pending tree is
    // not deleted out from under the test.
    let (kernel, _tmp) = MockKernelBuilder::new().build();
    let skills_root_path = kernel.home_dir().join("skills");

    // Register an agent whose manifest opts the workshop into auto-promote.
    let agent_id = AgentId::new();
    let session_id = SessionId::new();
    let manifest = AgentManifest {
        name: "auto_workshop_agent".to_string(),
        description: "test".to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        skill_workshop: SkillWorkshopConfig {
            enabled: true,
            auto_capture: true,
            approval_policy: ApprovalPolicy::Auto,
            review_mode: ReviewMode::Heuristic,
            max_pending: 20,
            max_pending_age_days: None,
            evolution_mode: EvolutionMode::Free,
        },
        ..Default::default()
    };
    let entry = AgentEntry {
        id: agent_id,
        name: "auto_workshop_agent".to_string(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: Utc::now(),
        last_active: Utc::now(),
        session_id,
        ..Default::default()
    };
    kernel
        .agent_registry_ref()
        .register(entry)
        .expect("register agent");

    // Plant a session with a teaching signal the heuristic must capture.
    let session = Session {
        id: session_id,
        agent_id,
        messages: vec![
            Message::user("from now on always run cargo fmt before committing."),
            Message::assistant("Got it, I'll run cargo fmt first."),
        ],
        context_window_tokens: 0,
        label: None,
        model_override: None,
        messages_generation: 1,
        last_repaired_generation: None,
        peer_id: None,
    };
    kernel
        .substrate_ref()
        .save_session(&session)
        .expect("save_session");

    // Compute the expected name via the same heuristic the hook uses —
    // pinning the literal would tie the test to whatever
    // `synth_name` happens to produce today, while the contract under
    // test is "auto branch promotes whatever the heuristic captured".
    let expected_name = librefang_kernel::skill_workshop::heuristic::extract_explicit_instruction(
        "from now on always run cargo fmt before committing.",
    )
    .expect("explicit_instruction must match the canonical example")
    .name;

    // Run the capture pipeline. Inside, the auto branch saves the
    // pending file, promotes via `evolution::create_skill`, then awaits
    // `reload_skills` via `spawn_blocking` — when this future resolves
    // the active skill is on disk and the in-memory registry has been
    // refreshed.
    skill_workshop::run_capture(kernel.clone(), agent_id).await;

    let skill_dir = skills_root_path.join(&expected_name);
    assert!(
        skill_dir.join("skill.toml").exists(),
        "auto-promoted skill.toml must land under skills_root/{}; got skills_root={}",
        expected_name,
        skills_root_path.display()
    );

    // Auto branch must reload the kernel's in-memory skill registry
    // after promotion — otherwise the next turn's prompt build will not
    // see the new skill until daemon restart, and the whole point of
    // auto-promote (vs pending review) is "agent picks it up
    // immediately". This is the assertion that locks the
    // `kernel.reload_skills()` call site in `mod.rs::capture_one`'s
    // Auto branch; without the reload, this assertion fails even
    // though the on-disk file landed.
    let registry = kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let registered: Vec<String> = registry
        .list()
        .iter()
        .map(|s| s.manifest.skill.name.clone())
        .collect();
    assert!(
        registered.iter().any(|n| n == &expected_name),
        "auto-promoted skill must be visible in kernel.skill_registry after reload; got {registered:?}, expected to include {expected_name:?}"
    );
    drop(registry);

    // Pending file must have been removed by `approve_candidate` after
    // promotion succeeded. The directory may or may not exist depending
    // on whether `agent_pending_dir` ever ran (the auto path always
    // calls `save_candidate` first, which always creates it, so we
    // require it to exist as a regression guard against a future
    // refactor that skips the staging write).
    let pending_dir = skills_root_path.join("pending").join(agent_id.to_string());
    assert!(
        pending_dir.exists(),
        "auto path should still stage the pending file before promotion; pending_dir={} missing",
        pending_dir.display()
    );
    let leftovers: Vec<_> = std::fs::read_dir(&pending_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s == "toml")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "auto-promotion should drop the pending file; found {} leftover .toml file(s)",
        leftovers.len()
    );
}

/// Approving a pending CREATE through the kernel's `approve_pending_skill`
/// (the path the `POST /api/skills/pending/{id}/approve` route now uses)
/// auto-assigns the promoted skill to the creating agent's allowlist (#5844).
/// An agent with a non-empty `skills` allowlist can then actually use the
/// skill it created.
#[tokio::test(flavor = "multi_thread")]
async fn approve_create_auto_assigns_skill_to_creator_allowlist() {
    use librefang_kernel::skill_workshop::storage;
    use librefang_types::agent::{
        AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, EvolutionMode, SessionId,
        SkillWorkshopConfig,
    };

    let (kernel, _tmp) = MockKernelBuilder::new().build();
    let skills_root = kernel.home_dir().join("skills");

    let agent_id = AgentId::new();
    let session_id = SessionId::new();
    let manifest = AgentManifest {
        name: "allowlist_creator".to_string(),
        description: "test".to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        // Non-empty allowlist → auto-assign is meaningful.
        skills: vec!["preexisting".to_string()],
        skill_workshop: SkillWorkshopConfig {
            enabled: true,
            evolution_mode: EvolutionMode::Free,
            ..Default::default()
        },
        ..Default::default()
    };
    let entry = AgentEntry {
        id: agent_id,
        name: "allowlist_creator".to_string(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: Utc::now(),
        last_active: Utc::now(),
        session_id,
        ..Default::default()
    };
    kernel
        .agent_registry_ref()
        .register(entry)
        .expect("register agent");

    // Seed a pending CREATE candidate owned by this agent.
    let mut candidate = fixture_candidate(&agent_id.to_string(), &uuid::Uuid::new_v4().to_string());
    candidate.name = "agent_made_skill".to_string();
    candidate.kind = CandidateKind::Create;
    let cand_id = candidate.id.clone();
    storage::save_candidate(&skills_root, &candidate, 20, None).expect("save create candidate");

    // Approve via the kernel path the route uses.
    let result = kernel
        .approve_pending_skill(&cand_id)
        .expect("approve should succeed");
    assert_eq!(result.skill_name, "agent_made_skill");

    // The creating agent's allowlist now contains the new skill, alongside
    // the pre-existing entry.
    let skills = kernel
        .agent_registry_ref()
        .get(agent_id)
        .expect("agent still registered")
        .manifest
        .skills;
    assert!(
        skills.contains(&"agent_made_skill".to_string()),
        "created skill must be auto-assigned to creator allowlist; got {skills:?}"
    );
    assert!(
        skills.contains(&"preexisting".to_string()),
        "pre-existing allowlist entry must be preserved; got {skills:?}"
    );
}

/// Regression test for the "orphan-pending death loop" corner: if a
/// previous `evolution::create_skill` attempt failed and left an
/// orphan pending file behind, the next turn's auto-promote attempt
/// hits the dedup short-circuit (same `(source kind, name,
/// prompt_context)` already on disk) and returns Ok(false) from
/// `save_candidate`. The auto branch must detect this case and
/// retry `approve_candidate` against the existing orphan id rather
/// than silently leaving it stuck for every future turn.
///
/// Set up: stage an orphan pending file directly (simulating a prior
/// failed promotion), then run capture with a session whose teaching
/// signal would heuristically produce the SAME `(kind, name,
/// prompt_context)` as the orphan. After `run_capture` returns, the
/// orphan must be promoted (active skill on disk + visible in the
/// in-memory registry) and the pending file must be gone.
#[tokio::test(flavor = "multi_thread")]
async fn auto_policy_recovers_orphaned_pending_via_retry() {
    use librefang_kernel::skill_workshop::{self, candidate::CandidateSkill, storage};
    use librefang_memory::session::Session;
    use librefang_types::agent::{
        AgentEntry, AgentId, AgentManifest, AgentMode, AgentState, ApprovalPolicy, EvolutionMode,
        ReviewMode, SessionId, SkillWorkshopConfig,
    };
    use librefang_types::message::Message;

    let (kernel, _tmp) = MockKernelBuilder::new().build();
    let skills_root_path = kernel.home_dir().join("skills");

    let agent_id = AgentId::new();
    let session_id = SessionId::new();
    let manifest = AgentManifest {
        name: "orphan_retry_agent".to_string(),
        description: "test".to_string(),
        author: "test".to_string(),
        module: "builtin:chat".to_string(),
        skill_workshop: SkillWorkshopConfig {
            enabled: true,
            auto_capture: true,
            approval_policy: ApprovalPolicy::Auto,
            review_mode: ReviewMode::Heuristic,
            max_pending: 20,
            max_pending_age_days: None,
            evolution_mode: EvolutionMode::Free,
        },
        ..Default::default()
    };
    let entry = AgentEntry {
        id: agent_id,
        name: "orphan_retry_agent".to_string(),
        manifest,
        state: AgentState::Running,
        mode: AgentMode::default(),
        created_at: Utc::now(),
        last_active: Utc::now(),
        session_id,
        ..Default::default()
    };
    kernel
        .agent_registry_ref()
        .register(entry)
        .expect("register agent");

    // Compute the heuristic-derived candidate from the canonical
    // teaching signal so the orphan we plant matches the dedup key
    // the next `run_capture` will produce.
    let user_msg = "from now on always run cargo fmt before committing.";
    let hit = librefang_kernel::skill_workshop::heuristic::extract_explicit_instruction(user_msg)
        .expect("heuristic must match");

    let orphan_id = uuid::Uuid::new_v4().to_string();
    let orphan = CandidateSkill {
        id: orphan_id.clone(),
        agent_id: agent_id.to_string(),
        session_id: Some(session_id.to_string()),
        captured_at: Utc::now() - chrono::Duration::seconds(60),
        source: hit.source.clone(),
        name: hit.name.clone(),
        description: hit.description.clone(),
        prompt_context: hit.prompt_context.clone(),
        provenance: librefang_kernel::skill_workshop::candidate::Provenance {
            user_message_excerpt: hit.user_message_excerpt.clone(),
            assistant_response_excerpt: hit.assistant_response_excerpt.clone(),
            turn_index: 1,
        },
        kind: CandidateKind::Create,
        target_skill_id: None,
        current_version: None,
        proposed_version: None,
    };
    storage::save_candidate(&skills_root_path, &orphan, 20, None)
        .expect("seed orphan pending file");

    // Plant the same teaching signal in the session so `run_capture`
    // produces an identically-keyed candidate.
    let session = Session {
        id: session_id,
        agent_id,
        messages: vec![Message::user(user_msg), Message::assistant("Got it.")],
        context_window_tokens: 0,
        label: None,
        model_override: None,
        messages_generation: 1,
        last_repaired_generation: None,
        peer_id: None,
    };
    kernel
        .substrate_ref()
        .save_session(&session)
        .expect("save_session");

    skill_workshop::run_capture(kernel.clone(), agent_id).await;

    // Active skill must land — the orphan was retried and promoted.
    let skill_dir = skills_root_path.join(&hit.name);
    assert!(
        skill_dir.join("skill.toml").exists(),
        "orphan retry should promote the existing pending entry; expected skill.toml at {}",
        skill_dir.display()
    );

    // Orphan pending file is gone.
    let pending_dir = skills_root_path.join("pending").join(agent_id.to_string());
    let leftovers: Vec<_> = std::fs::read_dir(&pending_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s == "toml")
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "orphan retry should clear the pending file; got {} leftover .toml file(s)",
        leftovers.len()
    );

    // Registry sees the new skill.
    let registry = kernel
        .skill_registry_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let registered: Vec<String> = registry
        .list()
        .iter()
        .map(|s| s.manifest.skill.name.clone())
        .collect();
    assert!(
        registered.iter().any(|n| n == &hit.name),
        "orphan-retry-promoted skill must be visible in registry; got {registered:?}, expected {:?}",
        hit.name
    );
}

/// Internal-error scrub regression (#3): a corrupt pending file on
/// disk makes `storage::load_candidate` fail at the TOML-parse step
/// (`toml::from_str`), which surfaces as a `WorkshopError` other than
/// `NotFound` / `InvalidId` — i.e. the route's catch-all 500 arm.
/// Before the fix that arm echoed `format!("failed to load candidate:
/// {e}")`, leaking the raw deserialize error (struct field names like
/// `agent_id`, parser position, "missing field"). The body must now be
/// the generic "Internal server error" with no parser / schema detail.
#[tokio::test(flavor = "multi_thread")]
async fn pending_show_corrupt_file_returns_scrubbed_500() {
    let h = boot().await;
    let agent = "11111111-1111-1111-1111-111111111111";
    let id = "ffffffff-0000-0000-0000-00000000000c";
    let root = skills_root(&h);

    // Stage a syntactically-valid-TOML file under the candidate's
    // expected path whose contents do NOT satisfy `CandidateSkill`'s
    // required fields. `toml::from_str::<CandidateSkill>` fails with a
    // "missing field `…`" deserialize error — the catch-all 500 arm.
    let pending_dir = root.join("pending").join(agent);
    std::fs::create_dir_all(&pending_dir).unwrap();
    std::fs::write(
        pending_dir.join(format!("{id}.toml")),
        "name = \"only_a_name_no_other_required_fields\"\n",
    )
    .unwrap();

    let (status, body) = json_request(&h, Method::GET, &format!("/api/skills/pending/{id}")).await;

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "corrupt candidate must surface as a 500: {body:?}"
    );
    // `internal_scrub` returns the canonical `ApiErrorResponse`, which
    // serializes to the #3639 nested envelope: the scrubbed message
    // lives at `error.message` (and the flat deprecated `message`
    // alias), not as a bare `error` string. Assert the generic text at
    // both surfaces.
    assert_eq!(
        body["error"]["message"].as_str().unwrap_or_default(),
        "Internal server error",
        "500 nested `error.message` must be the generic scrubbed message: {body:?}"
    );
    assert_eq!(
        body["message"].as_str().unwrap_or_default(),
        "Internal server error",
        "500 flat `message` must be the generic scrubbed message: {body:?}"
    );
    // Explicit leak guard: scan the *entire* serialized body, not one
    // field — none of the raw-deserialize / schema tokens that the
    // pre-fix `format!("failed to load candidate: {e}")` body carried
    // may appear anywhere in the response.
    let whole = body.to_string().to_lowercase();
    for needle in [
        "missing field",
        "agent_id",
        "toml",
        "expected",
        "failed to load",
        "deserialize",
    ] {
        assert!(
            !whole.contains(needle),
            "scrubbed 500 body leaked internal token {needle:?}: {body:?}"
        );
    }
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
