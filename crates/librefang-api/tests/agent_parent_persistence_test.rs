//! `parent_agent_id` must survive a daemon restart (#7930).
//!
//! `AgentEntry.parent` had no column in the `agents` table, so every hydration site in `librefang-memory`'s `StructuredStore` reconstructed it as `None`.
//! `routes/agents/mod.rs` serialises that field as `parent_agent_id`, which made the defect user-visible rather than merely dead: after any restart the API positively asserted that every agent had no parent, whatever its real lineage.
//! The pre-fix kernel test (`kernel::tests`, `assert_eq!(entry.parent, Some(parent))`) passed throughout, because it reads the in-memory registry and never round-trips through the store.
//!
//! These tests close that gap the only way that actually proves it: two `LibreFangKernel::boot_with_config` calls against the same `home_dir` / `data_dir`, with the production router (`server::build_router`) in front of each.
//! Nothing in the second boot has ever seen the first boot's registry, so a `parent_agent_id` that is still correct on the far side can only have come off disk.
//!
//! `reload_agent_from_disk` — the in-process restart stand-in used by `agent_manifest_persist_test.rs` — is deliberately NOT used here.
//! It re-reads `agent.toml` and swaps only the manifest, and lineage is not a manifest field, so a test built on it would pass vacuously against the broken code.
//!
//! No test here reaches an LLM: every assertion lands on registry and store state.
//!
//! Run: cargo test -p librefang-api --test agent_parent_persistence_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentManifest, ManifestCapabilities};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// One booted daemon: the concrete kernel (for `spawn_agent_with_parent`, which is an inherent method and therefore not reachable through `AppState`'s `Arc<dyn KernelApi>`), the production router, and the shared state.
struct Daemon {
    kernel: Arc<LibreFangKernel>,
    app: axum::Router,
    state: Arc<AppState>,
}

impl Daemon {
    fn shutdown(self) {
        self.state.kernel.shutdown();
    }
}

fn test_config(home: &std::path::Path) -> KernelConfig {
    KernelConfig {
        home_dir: home.to_path_buf(),
        data_dir: home.join("data"),
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
        ..KernelConfig::default()
    }
}

/// Boot a daemon against `home`. Calling this twice with the same path is the restart.
async fn boot_at(home: &std::path::Path) -> Daemon {
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(home);
    let kernel =
        Arc::new(LibreFangKernel::boot_with_config(test_config(home)).expect("kernel should boot"));
    kernel.set_self_handle();
    // Keep the concrete handle before `build_router` coerces it to `Arc<dyn KernelApi>`.
    let concrete = kernel.clone();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    Daemon {
        kernel: concrete,
        app,
        state,
    }
}

/// A manifest with enough capability surface to be a legal parent.
/// `spawn_agent_inner` enforces that a child's capabilities are a subset of its parent's, so the parent must be the broader of the two.
fn parent_manifest(name: &str) -> AgentManifest {
    AgentManifest {
        name: name.to_string(),
        description: "lineage test parent".to_string(),
        capabilities: ManifestCapabilities {
            tools: vec!["file_read".to_string()],
            ..Default::default()
        },
        ..AgentManifest::default()
    }
}

fn child_manifest(name: &str) -> AgentManifest {
    AgentManifest {
        name: name.to_string(),
        description: "lineage test child".to_string(),
        ..AgentManifest::default()
    }
}

/// `GET /api/agents`, returning the `items` array.
/// `enrich_agent_json` — and therefore `parent_agent_id` — is reached from the list route; `GET /api/agents/{id}` does not emit it.
async fn list_agents(app: &axum::Router) -> Vec<serde_json::Value> {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/agents?include_hands=true&limit=500")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET /api/agents must succeed"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    json["items"]
        .as_array()
        .expect("paginated envelope must carry `items`")
        .clone()
}

fn agent_named<'a>(items: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    items
        .iter()
        .find(|a| a["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("agent `{name}` must be listed; got {items:#?}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The exact regression from #7930.
///
/// Before the fix the second boot's `GET /api/agents` reported `parent_agent_id: null` for the child, because `load_all_agents` had no column to read and hardcoded `parent: None`.
#[tokio::test(flavor = "multi_thread")]
async fn parent_agent_id_survives_a_daemon_restart() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();

    let (parent_id, child_id) = {
        let daemon = boot_at(&home).await;

        let parent_id = daemon
            .kernel
            .spawn_agent(parent_manifest("lineage-parent"))
            .expect("parent spawn");
        // A child agent gets a random id rather than a name-derived one, so it has to be captured from the spawn return value.
        let child_id = daemon
            .kernel
            .spawn_agent_with_parent(child_manifest("lineage-child"), Some(parent_id))
            .expect("child spawn");

        // Baseline: the live registry already had this right before the fix.
        // Asserting it here is what makes the post-restart assertion below a persistence claim rather than a spawn one.
        let items = list_agents(&daemon.app).await;
        assert_eq!(
            agent_named(&items, "lineage-child")["parent_agent_id"].as_str(),
            Some(parent_id.to_string().as_str()),
            "the live registry must know the child's parent before the restart"
        );

        daemon.shutdown();
        (parent_id, child_id)
    };

    // --- restart: a second kernel that has never seen the first one's registry ---
    let daemon = boot_at(&home).await;
    let items = list_agents(&daemon.app).await;

    let child = agent_named(&items, "lineage-child");
    assert_eq!(
        child["id"].as_str(),
        Some(child_id.to_string().as_str()),
        "the restored child must be the same agent, not a respawn"
    );
    assert_eq!(
        child["parent_agent_id"].as_str(),
        Some(parent_id.to_string().as_str()),
        "#7930: parent_agent_id must survive the restart instead of reverting to null"
    );
    assert_eq!(
        child["parent_unknown"],
        serde_json::Value::Bool(false),
        "a row written after schema v54 has authoritative lineage"
    );

    // `children` is derived from the stored `parent_id` edges rather than persisted, so this is only correct if the derivation ran on the reload path.
    let parent = agent_named(&items, "lineage-parent");
    assert_eq!(
        parent["children"],
        serde_json::json!([child_id.to_string()]),
        "the parent's derived children list must name the restored child"
    );
    assert_eq!(
        parent["parent_agent_id"],
        serde_json::Value::Null,
        "a top-level agent stays parentless across the restart"
    );
    assert_eq!(
        parent["parent_unknown"],
        serde_json::Value::Bool(false),
        "a genuine root agent must be reported as a KNOWN root"
    );

    daemon.shutdown();
}

/// A row written before schema v54 must come back as "lineage unknown", never as a root agent.
///
/// The bug produced `parent_agent_id: null`, which is wrong.
/// Reading a pre-v54 row as a root agent would replace that with a *confidently* wrong answer — the API would be asserting a fact about lineage that was never recorded.
/// The `parent_recorded INTEGER NOT NULL DEFAULT 0` column exists exactly so that the two cases stay distinguishable, and this test reproduces the on-disk state that the `ALTER TABLE` leaves behind for a row that already existed.
#[tokio::test(flavor = "multi_thread")]
async fn a_pre_v54_row_is_reported_as_unknown_lineage_not_as_a_root_agent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();

    let db_path: std::path::PathBuf = {
        let daemon = boot_at(&home).await;
        let parent_id = daemon
            .kernel
            .spawn_agent(parent_manifest("legacy-parent"))
            .expect("parent spawn");
        daemon
            .kernel
            .spawn_agent_with_parent(child_manifest("legacy-child"), Some(parent_id))
            .expect("child spawn");
        daemon.shutdown();
        // `boot.rs` resolves the substrate to `data_dir/librefang.db` when `[memory] sqlite_path` is unset, which `test_config` leaves at its default.
        home.join("data").join("librefang.db")
    };

    // Rewind the child's row to exactly what a pre-v54 database holds: no recorded parent, and the `DEFAULT 0` provenance flag the migration backfilled.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open agents db");
        let touched = conn
            .execute(
                "UPDATE agents SET parent_id = NULL, parent_recorded = 0 WHERE name = 'legacy-child'",
                [],
            )
            .expect("rewind the child row to its pre-v54 state");
        assert_eq!(touched, 1, "exactly one child row must be rewound");
    }

    let daemon = boot_at(&home).await;
    let items = list_agents(&daemon.app).await;

    let child = agent_named(&items, "legacy-child");
    assert_eq!(
        child["parent_agent_id"],
        serde_json::Value::Null,
        "a pre-v54 row has no recoverable parent"
    );
    assert_eq!(
        child["parent_unknown"],
        serde_json::Value::Bool(true),
        "a pre-v54 row must NOT start claiming to be a root agent"
    );

    let parent = agent_named(&items, "legacy-parent");
    assert_eq!(
        parent["children"],
        serde_json::json!([]),
        "an unrecorded edge must not be invented on the parent side either"
    );

    daemon.shutdown();
}
