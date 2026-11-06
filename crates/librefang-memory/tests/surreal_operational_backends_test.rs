//! Round-trip tests for the operational SurrealDB backends against the real schema: usage, paired devices, prompt versions and experiments, the kv store and the agent registry.
//!
//! Each test applies every operational migration first, so a schema/backend mismatch (an unset optional sent as JSON `null`, a JSON value the column type rejects, a removed SurrealQL function) fails here rather than in a running daemon.
//! The embedded variant runs on `kv-surrealkv` in a temp dir; the remote variant runs the same body over `ws://` and `http://` when `BOSSFANG_TEST_SURREAL_URL` is set, and prints an explicit SKIP line when it is not.

#![cfg(feature = "surreal-backend")]

use librefang_memory::usage::UsageRecord;
use librefang_memory::{
    DeviceBackend, KvBackend, MemoryBackend, PromptBackend, SurrealDeviceStore, SurrealKvBackend,
    SurrealMemoryBackend, SurrealPromptStore, SurrealUsageStore, UsageBackend,
};
use librefang_storage::config::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};
use librefang_storage::migrations::{apply_pending, OPERATIONAL_MIGRATIONS};
use librefang_storage::{SurrealConnectionPool, SurrealSession};
use librefang_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentState, ExperimentStatus, PromptExperiment,
    PromptVersion, SessionId,
};
use serde_json::json;
use uuid::Uuid;

async fn exercise(store: &SurrealSession) {
    apply_pending(store.client(), OPERATIONAL_MIGRATIONS)
        .await
        .expect("operational migrations");
    usage(store);
    devices(store);
    prompts(store);
    kv(store);
    agents(store).await;
}

/// Unattributed records (no user / channel / session) must be accepted, and the cost queries must sum every matching row.
fn usage(store: &SurrealSession) {
    let usage = SurrealUsageStore::open(store);
    let agent = AgentId::new();
    for cost in [1.5, 2.0] {
        usage
            .record(&UsageRecord {
                agent_id: agent,
                provider: "p".to_string(),
                model: "m".to_string(),
                input_tokens: 10,
                output_tokens: 20,
                cost_usd: cost,
                tool_calls: 1,
                latency_ms: 5,
                ..Default::default()
            })
            .expect("record unattributed usage");
    }
    assert_eq!(usage.query_hourly(agent).expect("hourly"), 3.5);
    assert_eq!(usage.query_daily(agent).expect("daily"), 3.5);
    assert_eq!(usage.query_monthly(agent).expect("monthly"), 3.5);
    let summary = usage.query_summary(Some(agent)).expect("summary");
    assert_eq!(summary.call_count, 2);
    assert_eq!(summary.total_input_tokens, 20);
    assert_eq!(summary.total_cost_usd, 3.5);
    assert_eq!(
        usage.query_hourly(AgentId::new()).expect("no rows"),
        0.0,
        "an agent without usage sums to zero"
    );
    // Quota checks read the same sums, so a cap below the running total must trip.
    let over = UsageRecord {
        agent_id: agent,
        model: "m".to_string(),
        cost_usd: 1.0,
        ..Default::default()
    };
    assert!(usage.check_quota_and_record(&over, 4.0, 0.0, 0.0).is_err());
}

fn devices(store: &SurrealSession) {
    let devices = SurrealDeviceStore::open(store);
    devices
        .save_paired_device(
            "ios:dev/1",
            "Phone",
            "ios",
            "2026-09-27",
            "2026-09-27",
            None,
            "hash",
        )
        .expect("save device without push token");
    let loaded = devices.load_paired_devices().expect("load devices");
    let device = loaded
        .iter()
        .find(|d| d["device_id"] == "ios:dev/1")
        .expect("device loaded");
    assert_eq!(device["push_token"], serde_json::Value::Null);
    assert_eq!(device["api_key_hash"], "hash");
}

fn prompts(store: &SurrealSession) {
    let prompts = SurrealPromptStore::open(store);
    let agent = AgentId::new();
    let version = PromptVersion {
        id: Uuid::new_v4(),
        agent_id: agent,
        version: 1,
        content_hash: "h".to_string(),
        system_prompt: "sys".to_string(),
        tools: vec!["search".to_string(), "fetch".to_string()],
        variables: vec!["name".to_string()],
        created_at: chrono::Utc::now(),
        created_by: "test".to_string(),
        is_active: false,
        description: None,
    };
    prompts
        .create_version(version.clone())
        .expect("create version without description");
    let got = prompts
        .get_version(version.id)
        .expect("get version")
        .expect("version exists");
    assert_eq!(got.tools, version.tools);
    assert_eq!(got.variables, version.variables);
    assert_eq!(got.description, None);
    assert_eq!(prompts.list_versions(agent).expect("list").len(), 1);
    prompts
        .set_active_version(version.id, agent)
        .expect("activate");
    assert_eq!(
        prompts
            .get_active_version(agent)
            .expect("active")
            .map(|v| v.id),
        Some(version.id)
    );

    let experiment = PromptExperiment {
        id: Uuid::new_v4(),
        name: "exp".to_string(),
        agent_id: agent,
        status: ExperimentStatus::Draft,
        traffic_split: vec![50, 50],
        success_criteria: Default::default(),
        started_at: None,
        ended_at: None,
        created_at: chrono::Utc::now(),
        variants: vec![],
    };
    prompts
        .create_experiment(experiment.clone())
        .expect("create experiment without start time");
    assert_eq!(
        prompts
            .get_experiment(experiment.id)
            .expect("get experiment")
            .map(|e| e.id),
        Some(experiment.id)
    );

    let variant = Uuid::new_v4();
    prompts
        .record_request(experiment.id, variant, 10, 0.25, true)
        .expect("first request");
    prompts
        .record_request(experiment.id, variant, 30, 0.75, false)
        .expect("second request updates the metrics row");
    let metrics = prompts
        .get_variant_metrics(variant)
        .expect("metrics")
        .expect("metrics row");
    assert_eq!(metrics.total_requests, 2);
    assert_eq!(metrics.successful_requests, 1);
    assert_eq!(metrics.failed_requests, 1);
    assert_eq!(metrics.total_cost_usd, 1.0);
}

fn kv(store: &SurrealSession) {
    let kv = SurrealKvBackend::open(store);
    let agent = AgentId::new();
    for (key, value) in [
        ("string", json!("hello")),
        ("number", json!(3)),
        ("bool", json!(true)),
        ("array", json!([1, {"a": 2}])),
        ("object", json!({"nested": {"k": [1, 2]}})),
    ] {
        kv.structured_set(agent, key, value.clone())
            .unwrap_or_else(|e| panic!("kv set {key}: {e}"));
        assert_eq!(
            kv.structured_get(agent, key).expect("kv get"),
            Some(value),
            "kv {key}"
        );
    }
}

async fn agents(store: &SurrealSession) {
    let agents = SurrealMemoryBackend::new(store.clone())
        .await
        .expect("agent backend");
    let entry = AgentEntry {
        id: AgentId::new(),
        name: "registry-probe".to_string(),
        manifest: AgentManifest::default(),
        state: AgentState::Running,
        created_at: chrono::Utc::now(),
        last_active: chrono::Utc::now(),
        session_id: SessionId::new(),
        ..Default::default()
    };
    agents.save_agent(&entry).expect("save agent");
    let loaded = agents
        .load_agent(entry.id)
        .expect("load agent")
        .expect("agent exists");
    assert_eq!(loaded.name, "registry-probe");
}

#[tokio::test(flavor = "multi_thread")]
async fn operational_backends_round_trip_embedded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = StorageConfig {
        backend: StorageBackendKind::embedded(dir.path().join("ops.surreal")),
        namespace: "librefang".into(),
        database: "main".into(),
        legacy_sqlite_path: None,
    };
    let store = SurrealConnectionPool::new()
        .open(&cfg)
        .await
        .expect("open embedded");
    exercise(&store).await;
}

fn remote_urls(url: &str) -> Vec<String> {
    let pairs = [("ws://", "http://"), ("wss://", "https://")];
    for (ws, http) in pairs {
        if let Some(rest) = url.strip_prefix(ws).or_else(|| url.strip_prefix(http)) {
            return vec![format!("{ws}{rest}"), format!("{http}{rest}")];
        }
    }
    vec![url.to_string()]
}

#[tokio::test(flavor = "multi_thread")]
async fn operational_backends_round_trip_remote() {
    let Ok(url) = std::env::var("BOSSFANG_TEST_SURREAL_URL") else {
        eprintln!("SKIP remote surreal: BOSSFANG_TEST_SURREAL_URL unset");
        return;
    };
    let username = std::env::var("BOSSFANG_TEST_SURREAL_USER").unwrap_or_else(|_| "root".into());
    let password_env = std::env::var("BOSSFANG_TEST_SURREAL_PASS_ENV")
        .unwrap_or_else(|_| "BOSSFANG_TEST_SURREAL_PASS".into());
    for url in remote_urls(&url) {
        let database = format!("ops_rt_{}", Uuid::new_v4().simple());
        eprintln!("remote surreal: operational backends against {url} db={database}");
        let store = SurrealConnectionPool::new()
            .open_remote(&RemoteSurrealConfig {
                url: url.clone(),
                namespace: "bossfang_test".into(),
                database: database.clone(),
                username: username.clone(),
                password_env: password_env.clone(),
                tls_skip_verify: false,
            })
            .await
            .unwrap_or_else(|e| panic!("open remote {url}: {e}"));
        exercise(&store).await;
        store
            .client()
            .query(format!("REMOVE DATABASE IF EXISTS {database}"))
            .await
            .expect("drop test database");
    }
}
