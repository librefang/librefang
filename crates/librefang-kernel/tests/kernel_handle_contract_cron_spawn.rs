use librefang_kernel_handle::KernelHandle;

mod common;

use common::boot_kernel as boot;

fn minimal_manifest() -> &'static str {
    r#"
name = "test-agent"
version = "0.1.0"
description = "test"
author = "test"
module = "builtin:chat"

[model]
provider = "none"
model = "none"
system_prompt = "test"
"#
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cron_create_preserves_peer_id() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (agent_id, _name) = kh
        .spawn_agent(minimal_manifest(), None)
        .await
        .expect("spawn failed");

    let job = serde_json::json!({
        "name": "test-cron",
        "agent_id": agent_id,
        "schedule": { "kind": "every", "every_secs": 60 },
        "action": { "kind": "system_event", "text": "tick" },
        "peer_id": "peer-abc",
        "session_mode": "persistent",
        "one_shot": false
    });

    let result = kh.cron_create(&agent_id, job).await;
    assert!(result.is_ok(), "cron_create failed: {:?}", result.err());

    let jobs = kh.cron_list(&agent_id).await.expect("cron_list failed");
    assert!(!jobs.is_empty(), "expected at least one cron job");

    let created = &jobs[0];
    assert_eq!(
        created["peer_id"].as_str(),
        Some("peer-abc"),
        "peer_id should be preserved"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cron_create_without_peer_id() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (agent_id, _name) = kh
        .spawn_agent(minimal_manifest(), None)
        .await
        .expect("spawn failed");

    let job = serde_json::json!({
        "name": "test-cron-no-peer",
        "agent_id": agent_id,
        "schedule": { "kind": "every", "every_secs": 60 },
        "action": { "kind": "system_event", "text": "tick" },
        "session_mode": "persistent",
        "one_shot": false
    });

    let result = kh.cron_create(&agent_id, job).await;
    assert!(result.is_ok(), "cron_create failed: {:?}", result.err());

    let jobs = kh.cron_list(&agent_id).await.expect("cron_list failed");
    assert!(!jobs.is_empty(), "expected at least one cron job");

    let created = &jobs[0];
    assert!(
        created["peer_id"].is_null(),
        "peer_id should be null when not provided, got: {:?}",
        created["peer_id"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_spawn_agent_returns_valid_identity() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (id, name) = kh
        .spawn_agent(minimal_manifest(), None)
        .await
        .expect("spawn failed");

    assert!(!id.is_empty(), "agent id should not be empty");
    assert_eq!(name, "test-agent");

    let agents = kh.list_agents();
    let found = agents
        .iter()
        .find(|a| a.id == id)
        .expect("spawned agent should appear in list_agents");
    assert_eq!(found.name, "test-agent");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_list_agents_returns_manifest_metadata() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (id, _name) = kh
        .spawn_agent(minimal_manifest(), None)
        .await
        .expect("spawn failed");

    let agents = kh.list_agents();
    let info = agents
        .iter()
        .find(|a| a.id == id)
        .expect("spawned agent should appear in list_agents");

    assert_eq!(info.name, "test-agent");
    assert_eq!(info.description, "test");
    assert!(!info.id.is_empty());

    let found = kh.find_agents("test-agent");
    assert!(
        found.iter().any(|a| a.id == id),
        "find_agents(\"test-agent\") should return the spawned agent"
    );

    let missing = kh.find_agents("nonexistent");
    assert!(
        missing.is_empty(),
        "find_agents(\"nonexistent\") should return empty"
    );
}
