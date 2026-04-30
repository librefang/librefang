use librefang_kernel_handle::KernelHandle;

mod common;

use common::boot_kernel as boot;

#[test]
fn test_roster_roundtrip() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.roster_upsert("telegram", "chat1", "user1", "Alice", Some("@alice"))
        .expect("roster_upsert user1 failed");
    kh.roster_upsert("telegram", "chat1", "user2", "Bob", None)
        .expect("roster_upsert user2 failed");

    let members = kh
        .roster_members("telegram", "chat1")
        .expect("roster_members failed");
    assert_eq!(members.len(), 2);

    kh.roster_remove_member("telegram", "chat1", "user1")
        .expect("roster_remove_member failed");

    let members = kh
        .roster_members("telegram", "chat1")
        .expect("roster_members failed");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["user_id"].as_str().unwrap(), "user2");
    assert_eq!(members[0]["display_name"].as_str().unwrap(), "Bob");
}

#[test]
fn test_goal_list_active_default_empty() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let result = kh.goal_list_active(None);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_list_a2a_agents_default_empty() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let agents = kh.list_a2a_agents();
    assert!(agents.is_empty());
}

#[test]
fn test_get_a2a_agent_url_default_none() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let url = kh.get_a2a_agent_url("any-agent");
    assert!(url.is_none());
}

#[test]
fn test_kill_agent_unknown_returns_error() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let result = kh.kill_agent("nonexistent-id");
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_publish_event_succeeds() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let result = kh
        .publish_event("test_event", serde_json::json!({"key": "value"}))
        .await;
    assert!(result.is_ok());
}
