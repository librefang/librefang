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

    let result = kh.cron_create(&agent_id, job, None).await;
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

    let result = kh.cron_create(&agent_id, job, None).await;
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

/// A cron job created with an acting principal records it, and it survives the
/// round trip through `cron_list` — i.e. through the scheduler's own storage,
/// not just the struct literal (#7744).
#[tokio::test(flavor = "multi_thread")]
async fn cron_create_records_the_owner_and_it_survives_the_store() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (agent_id, _name) = kh
        .spawn_agent(minimal_manifest(), None)
        .await
        .expect("spawn failed");

    let owner = librefang_types::principal::Principal::group_named("oncall");
    let job = serde_json::json!({
        "name": "owned-cron",
        "agent_id": agent_id,
        "schedule": { "kind": "every", "every_secs": 60 },
        "action": { "kind": "system_event", "text": "tick" },
        "one_shot": false
    });

    let raw = kh
        .cron_create(&agent_id, job, Some(owner))
        .await
        .expect("cron_create failed");
    let created: serde_json::Value = serde_json::from_str(&raw).expect("cron_create returns JSON");
    assert_eq!(created["owner"], serde_json::json!(owner.to_string()));

    let jobs = kh.cron_list(&agent_id).await.expect("cron_list failed");
    let stored = jobs
        .iter()
        .find(|j| j["name"] == "owned-cron")
        .expect("the created job must be listed");
    assert_eq!(
        stored["owner"],
        serde_json::to_value(owner).unwrap(),
        "the owner must round-trip through the scheduler's store as the tagged struct"
    );
}

/// The stated meaning of `None`: unowned, and the key is absent rather than
/// null — so an existing `cron_jobs.json` is not rewritten by the upgrade.
#[tokio::test(flavor = "multi_thread")]
async fn cron_create_without_a_principal_records_no_owner_key() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (agent_id, _name) = kh
        .spawn_agent(minimal_manifest(), None)
        .await
        .expect("spawn failed");

    let job = serde_json::json!({
        "name": "unowned-cron",
        "agent_id": agent_id,
        "schedule": { "kind": "every", "every_secs": 60 },
        "action": { "kind": "system_event", "text": "tick" },
        "one_shot": false
    });
    kh.cron_create(&agent_id, job, None)
        .await
        .expect("cron_create failed");

    let jobs = kh.cron_list(&agent_id).await.expect("cron_list failed");
    let stored = jobs
        .iter()
        .find(|j| j["name"] == "unowned-cron")
        .expect("the created job must be listed");
    assert!(stored.get("owner").is_none());
}
