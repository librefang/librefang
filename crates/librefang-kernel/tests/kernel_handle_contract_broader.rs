use librefang_kernel::KernelApi;
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

/// The #7086 security boundary, exercised end to end against a real `MemorySubstrate` — so the assertion is over the store's SQL, not over a mock's willingness to agree with it.
///
/// `channel_members` reports everyone a platform enumerates into a channel; `channel_dm` may address only the people observed speaking there.
/// The narrowing is the `AND source = 'observed'` predicate in `RosterStore::observed_members`.
/// Delete that predicate and `U7` appears in both lists and this test fails, which is the point: bulk enumeration must never widen who an agent can privately message.
#[test]
fn enumerated_members_are_reportable_but_never_dm_authorized() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.roster_upsert("slack", "C0DESIGN", "U1", "Ana", Some("ana"))
        .expect("observed upsert failed");
    kernel
        .memory_substrate()
        .roster()
        .upsert_enumerated("slack", "C0DESIGN", "U7", "Never Spoken", None)
        .expect("enumerated upsert failed");

    let reported = kh
        .roster_members("slack", "C0DESIGN")
        .expect("roster_members failed");
    let reported_ids: Vec<&str> = reported
        .iter()
        .map(|m| m["user_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        reported_ids,
        vec!["U1", "U7"],
        "channel_members must be able to report the platform's full member list"
    );
    assert_eq!(reported[0]["source"], "observed");
    assert_eq!(reported[1]["source"], "enumerated");

    let authorized = kh
        .roster_observed_members("slack", "C0DESIGN")
        .expect("roster_observed_members failed");
    let authorized_ids: Vec<&str> = authorized
        .iter()
        .map(|m| m["user_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        authorized_ids,
        vec!["U1"],
        "channel_dm's authorization set must exclude a member who has only ever been enumerated"
    );
}

/// Speaking is what earns DM reachability, and it earns it immediately.
/// The promotion has to survive a later enumeration sweep, or a security control would switch itself off on the member-list TTL.
#[test]
fn speaking_promotes_an_enumerated_member_and_a_later_sweep_does_not_revoke_it() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;
    let roster = kernel.memory_substrate().roster();

    roster
        .upsert_enumerated("slack", "C0DESIGN", "U7", "U7", None)
        .expect("enumerated upsert failed");
    assert!(kh
        .roster_observed_members("slack", "C0DESIGN")
        .expect("roster_observed_members failed")
        .is_empty());

    kh.roster_upsert("slack", "C0DESIGN", "U7", "Nina", Some("nina"))
        .expect("observed upsert failed");
    roster
        .upsert_enumerated("slack", "C0DESIGN", "U7", "Nina", None)
        .expect("re-enumeration failed");

    let authorized = kh
        .roster_observed_members("slack", "C0DESIGN")
        .expect("roster_observed_members failed");
    assert_eq!(authorized.len(), 1);
    assert_eq!(authorized[0]["user_id"], "U7");
    assert_eq!(authorized[0]["display_name"], "Nina");
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
