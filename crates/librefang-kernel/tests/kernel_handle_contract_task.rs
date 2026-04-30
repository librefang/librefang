use librefang_kernel_handle::KernelHandle;

mod common;

use common::boot_kernel as boot;

fn minimal_manifest_toml() -> &'static str {
    r#"
name = "task-agent"
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
async fn test_task_post_preserves_assigned_to_and_created_by() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (agent_id, _) = kh
        .spawn_agent(minimal_manifest_toml(), None)
        .await
        .expect("spawn_agent failed");

    let task_id = kh
        .task_post("title", "desc", Some(&agent_id), Some("creator-1"))
        .await
        .expect("task_post failed");

    let tasks = kh.task_list(None).await.expect("task_list failed");
    let task = tasks
        .iter()
        .find(|t| t["id"].as_str() == Some(&task_id))
        .expect("task not found in list");

    assert_eq!(task["assigned_to"].as_str().unwrap(), agent_id);
    assert_eq!(task["created_by"].as_str().unwrap(), "creator-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_task_claim_returns_assigned_task() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (agent_id, _) = kh
        .spawn_agent(minimal_manifest_toml(), None)
        .await
        .expect("spawn_agent failed");

    let task_id = kh
        .task_post("title", "desc", Some(&agent_id), None)
        .await
        .expect("task_post failed");

    let claimed = kh.task_claim(&agent_id).await.expect("task_claim failed");

    assert!(claimed.is_some(), "expected a claimed task");
    let task = claimed.unwrap();
    assert_eq!(task["id"].as_str().unwrap(), task_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_task_complete_updates_status() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let (agent_id, _) = kh
        .spawn_agent(minimal_manifest_toml(), None)
        .await
        .expect("spawn_agent failed");

    let task_id = kh
        .task_post("title", "desc", Some(&agent_id), None)
        .await
        .expect("task_post failed");

    kh.task_claim(&agent_id).await.expect("task_claim failed");

    kh.task_complete(&agent_id, &task_id, "done!")
        .await
        .expect("task_complete failed");

    let task = kh
        .task_get(&task_id)
        .await
        .expect("task_get failed")
        .expect("task not found after complete");

    assert_eq!(task["status"].as_str().unwrap(), "completed");
    assert_eq!(task["result"].as_str().unwrap(), "done!");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_task_post_with_no_assignment() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let task_id = kh
        .task_post("title", "desc", None, None)
        .await
        .expect("task_post failed");

    let tasks = kh.task_list(None).await.expect("task_list failed");
    let task = tasks
        .iter()
        .find(|t| t["id"].as_str() == Some(&task_id))
        .expect("task not found in list");

    let assigned = task["assigned_to"].as_str().unwrap_or("");
    let created = task["created_by"].as_str().unwrap_or("");
    assert!(
        assigned.is_empty() || task["assigned_to"].is_null(),
        "expected null or empty assigned_to, got: {assigned:?}"
    );
    assert!(
        created.is_empty() || task["created_by"].is_null(),
        "expected null or empty created_by, got: {created:?}"
    );
}
