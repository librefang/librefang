//! Contract tests for the `KernelHandle` memory methods on `LibreFangKernel`.
//!
//! Validates that `memory_store`, `memory_recall`, and `memory_list` correctly
//! isolate global vs peer-scoped namespaces.

use librefang_kernel_handle::KernelHandle;

mod common;

use common::boot_kernel as boot;

#[test]
fn test_memory_store_recall_isolates_peer_namespaces() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.memory_store("key1", serde_json::json!("global_val"), None)
        .expect("store global");
    kh.memory_store("key1", serde_json::json!("peer_a_val"), Some("peer-a"))
        .expect("store peer-a");
    kh.memory_store("key1", serde_json::json!("peer_b_val"), Some("peer-b"))
        .expect("store peer-b");

    assert_eq!(
        kh.memory_recall("key1", None).expect("recall global"),
        Some(serde_json::json!("global_val"))
    );
    assert_eq!(
        kh.memory_recall("key1", Some("peer-a"))
            .expect("recall peer-a"),
        Some(serde_json::json!("peer_a_val"))
    );
    assert_eq!(
        kh.memory_recall("key1", Some("peer-b"))
            .expect("recall peer-b"),
        Some(serde_json::json!("peer_b_val"))
    );
    assert_eq!(
        kh.memory_recall("key1", Some("peer-c"))
            .expect("recall peer-c"),
        None
    );
}

#[test]
fn test_memory_list_separates_global_and_peer_keys() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.memory_store("g1", serde_json::json!(1), None)
        .expect("store g1");
    kh.memory_store("g2", serde_json::json!(2), None)
        .expect("store g2");
    kh.memory_store("p1", serde_json::json!(3), Some("peer-a"))
        .expect("store p1");
    kh.memory_store("p2", serde_json::json!(4), Some("peer-a"))
        .expect("store p2");

    let global_keys = kh.memory_list(None).expect("list global");
    assert!(global_keys.contains(&"g1".to_string()));
    assert!(global_keys.contains(&"g2".to_string()));
    assert!(!global_keys.contains(&"p1".to_string()));
    assert!(!global_keys.contains(&"p2".to_string()));

    let peer_keys = kh.memory_list(Some("peer-a")).expect("list peer-a");
    assert!(peer_keys.contains(&"p1".to_string()));
    assert!(peer_keys.contains(&"p2".to_string()));
    assert!(!peer_keys.contains(&"g1".to_string()));
    assert!(!peer_keys.contains(&"g2".to_string()));
}

#[test]
fn test_memory_recall_nonexistent_key_returns_none() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    assert_eq!(
        kh.memory_recall("nonexistent", None)
            .expect("recall nonexistent global"),
        None
    );
    assert_eq!(
        kh.memory_recall("nonexistent", Some("peer-x"))
            .expect("recall nonexistent peer"),
        None
    );
}

// ---------------------------------------------------------------------------
// SECURITY (#5119 + #5120) — colon-bearing / empty peer_id + `peer:` key
// prefix are rejected at the kernel-handle boundary so the tool layer can't
// (a) plant rows that surface to `memory_list(Some("victim"))` as if "victim"
// wrote them, or (b) split a peer namespace across two distinct peer_ids that
// happen to share a colon prefix. Every test asserts the SIDE EFFECT (the
// victim's list stays empty), not just the returned error variant.
// ---------------------------------------------------------------------------

use librefang_kernel::MemorySubsystemApi;
use librefang_kernel_handle::KernelOpError;
use librefang_types::agent::AgentId;

/// The well-known shared-memory agent id (`00000000-0000-0000-0000-000000000001`).
/// `librefang_kernel::shared_memory_agent_id()` is crate-private, so the
/// integration test reconstructs the documented constant directly.
fn shared_mem_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

#[test]
fn test_memory_store_rejects_peer_prefix_key() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let err = kh
        .memory_store("peer:victim:user_name", serde_json::json!("planted"), None)
        .expect_err("LLM-supplied 'peer:' key must be rejected (#5120)");
    assert!(
        matches!(err, KernelOpError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );

    // SIDE EFFECT: the planted row truly didn't land — `victim`'s namespace
    // must stay empty.
    let victim_keys = kh
        .memory_list(Some("victim"))
        .expect("list as victim must succeed when victim never wrote anything");
    assert!(
        victim_keys.is_empty(),
        "planted peer-prefixed row leaked to victim: {victim_keys:?}"
    );
}

#[test]
fn test_memory_recall_rejects_peer_prefix_key() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let err = kh
        .memory_recall("peer:victim:user_name", None)
        .expect_err("LLM-supplied 'peer:' key must be rejected on recall (#5120)");
    assert!(
        matches!(err, KernelOpError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

#[test]
fn test_memory_list_rejects_peer_id_with_colon() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let err = kh
        .memory_list(Some("u:42"))
        .expect_err("colon-bearing peer_id must be rejected on list (#5119)");
    assert!(
        matches!(err, KernelOpError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

#[test]
fn test_memory_store_rejects_peer_id_with_colon() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let err = kh
        .memory_store("user_name", serde_json::json!("alice"), Some("T1:U2"))
        .expect_err("Slack-style colon-bearing peer_id must be rejected (#5119)");
    assert!(
        matches!(err, KernelOpError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

#[test]
fn test_memory_store_rejects_empty_peer_id() {
    // (#5119 / review #3) An empty peer_id passes a naive `contains(':')`
    // check but yields `peer::{key}`, ambiguous with a `None`-scope key
    // literally named `:{key}`. It must be rejected, and nothing must land.
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let store_err = kh
        .memory_store("user_name", serde_json::json!("alice"), Some(""))
        .expect_err("empty peer_id must be rejected on store (#5119)");
    assert!(
        matches!(store_err, KernelOpError::InvalidInput(_)),
        "expected InvalidInput, got {store_err:?}"
    );

    let recall_err = kh
        .memory_recall("user_name", Some(""))
        .expect_err("empty peer_id must be rejected on recall (#5119)");
    assert!(matches!(recall_err, KernelOpError::InvalidInput(_)));

    let list_err = kh
        .memory_list(Some(""))
        .expect_err("empty peer_id must be rejected on list (#5119)");
    assert!(matches!(list_err, KernelOpError::InvalidInput(_)));

    // SIDE EFFECT: the global namespace was never written through the
    // ambiguous empty-peer path.
    let global_keys = kh.memory_list(None).expect("list global");
    assert!(
        global_keys.is_empty(),
        "empty-peer_id store leaked into global scope: {global_keys:?}"
    );
}

#[test]
fn test_memory_store_with_clean_peer_id_still_works() {
    // Sanity: the rejection logic must not affect legitimate (colon-free,
    // non-empty) peer_ids carrying a non-`peer:` key.
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.memory_store("user_name", serde_json::json!("alice"), Some("u"))
        .expect("colon-free peer_id with regular key must succeed");
    assert_eq!(
        kh.memory_recall("user_name", Some("u"))
            .expect("recall colon-free peer"),
        Some(serde_json::json!("alice"))
    );
    let keys = kh.memory_list(Some("u")).expect("list colon-free peer");
    assert_eq!(keys, vec!["user_name".to_string()]);
}

#[test]
fn test_prefix_planted_rows_not_enumerable_via_tool_memory_list() {
    // The write path now rejects `peer:`-prefixed keys, but rows planted
    // *before* the fix can still sit at `peer:...` in the shared substrate.
    // Plant them via the raw substrate (simulating an on-disk pre-fix row)
    // and prove the tool-boundary `memory_list` round-trip guard refuses to
    // enumerate the structurally-impossible ones.
    let (kernel, _tmp) = boot();
    let agent_id = shared_mem_agent_id();
    let substrate = MemorySubsystemApi::substrate_ref(&kernel);

    // (1) Nested / double-scoped plant: even if an attacker had landed
    //     `peer:victim:peer:other:secret` pre-fix, listing as "victim" must
    //     not surface `peer:other:secret` — the recovered inner key is itself
    //     `peer:`-prefixed and fails the strict round-trip.
    substrate
        .structured_set(
            agent_id,
            "peer:victim:peer:other:secret",
            serde_json::json!("planted-nested"),
        )
        .expect("plant nested pre-fix row");

    // (2) Colon-collision plant (#5119): a pre-fix Slack peer "victim:sub"
    //     writing `car` produced `peer:victim:sub:car`. An attacker peer
    //     "victim" listing must not strip `peer:victim:` and recover
    //     `sub:car`. The colon-bearing query is rejected outright; this row
    //     is parked here to prove the rejection happens before any recovery.
    substrate
        .structured_set(
            agent_id,
            "peer:victim:sub:car",
            serde_json::json!("planted-collision"),
        )
        .expect("plant colon-collision pre-fix row");

    let kh: &dyn KernelHandle = &kernel;

    // The legitimate `victim` peer lists its namespace. The nested plant's
    // recovered inner key `peer:other:secret` is dropped by the round-trip
    // guard, so it must NOT appear.
    let victim_keys = kh
        .memory_list(Some("victim"))
        .expect("list as victim must succeed");
    assert!(
        !victim_keys.iter().any(|k| k.starts_with("peer:")),
        "nested pre-fix plant enumerated to victim via tool path: {victim_keys:?}"
    );
    assert!(
        !victim_keys.contains(&"peer:other:secret".to_string()),
        "double-scoped plant leaked: {victim_keys:?}"
    );

    // The colon-collision attacker query is rejected before recovery runs,
    // so the foreign peer's `sub:car` can never be stripped into view.
    let collision = kh.memory_list(Some("victim:sub"));
    assert!(
        matches!(collision, Err(KernelOpError::InvalidInput(_))),
        "colon-bearing list query must be rejected outright, got {collision:?}"
    );
}

// ---------------------------------------------------------------------------
// #5138 — substrate-boundary input validation + atomic RMW.
// ---------------------------------------------------------------------------

#[test]
fn test_memory_store_rejects_empty_key_5138() {
    // memory_store(key="") would land a row at (shared_agent, "") and
    // memory_list(None) would then surface a nameless "" entry.
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let err = kh
        .memory_store("", serde_json::json!("x"), None)
        .expect_err("empty key must be rejected");
    assert!(matches!(err, KernelOpError::InvalidInput(_)), "got {err:?}");
    // The rejected write must not have created a "" key.
    let keys = kh.memory_list(None).expect("list global");
    assert!(
        !keys.iter().any(|k| k.is_empty()),
        "rejected empty-key write must not have landed a row: {keys:?}"
    );
    // Recall with an empty key is rejected symmetrically.
    let rerr = kh.memory_recall("", None);
    assert!(matches!(rerr, Err(KernelOpError::InvalidInput(_))));
}

#[test]
fn test_memory_store_rejects_oversized_value_5138() {
    // An agent (or a peer prompt coercing the agent) must not be able to
    // wedge the substrate with a giant blob. The cap is enforced at the
    // substrate before the INSERT, surfaced as InvalidInput.
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    // Comfortably over the 256 KiB cap.
    let big = serde_json::json!("z".repeat(300 * 1024));
    let err = kh
        .memory_store("huge", big, None)
        .expect_err("oversized value must be rejected");
    assert!(matches!(err, KernelOpError::InvalidInput(_)), "got {err:?}");
    // The rejected write must not have created the key.
    assert_eq!(
        kh.memory_recall("huge", None).expect("recall"),
        None,
        "oversized write must not have landed a row"
    );

    // A reasonably-sized value still works.
    kh.memory_store("ok", serde_json::json!("small"), None)
        .expect("normal value must still store");
    assert_eq!(
        kh.memory_recall("ok", None).expect("recall ok"),
        Some(serde_json::json!("small"))
    );
}

#[test]
fn test_concurrent_goal_update_loses_no_writes_5138() {
    use librefang_kernel_handle::GoalControl;

    // Seed two goals under the shared `__librefang_goals` key.
    let (kernel, _tmp) = boot();
    let shared = shared_mem_agent_id();
    let g1 = uuid::Uuid::new_v4().to_string();
    let g2 = uuid::Uuid::new_v4().to_string();
    MemorySubsystemApi::substrate_ref(&kernel)
        .structured_set(
            shared,
            "__librefang_goals",
            serde_json::json!([
                {"id": g1, "title": "one", "status": "pending", "progress": 0},
                {"id": g2, "title": "two", "status": "pending", "progress": 0},
            ]),
        )
        .expect("seed goals");

    // Two concurrent goal_update calls, each touching a *different* goal.
    // With the pre-fix get->mutate->set the second writer's blob clobbered
    // the first writer's edit (lost update). The transactional
    // structured_modify must preserve both.
    std::thread::scope(|s| {
        let k = &kernel;
        let g1 = g1.clone();
        let g2 = g2.clone();
        s.spawn(move || {
            for _ in 0..20 {
                let _ = k.goal_update(&g1, Some("in_progress"), Some(50));
            }
        });
        s.spawn(move || {
            for _ in 0..20 {
                let _ = k.goal_update(&g2, Some("completed"), Some(100));
            }
        });
    });

    let goals = match MemorySubsystemApi::substrate_ref(&kernel)
        .structured_get(shared, "__librefang_goals")
        .unwrap()
    {
        Some(serde_json::Value::Array(a)) => a,
        other => panic!("expected array, got {other:?}"),
    };
    assert_eq!(goals.len(), 2, "no goal may be dropped");
    let find = |id: &str| {
        goals
            .iter()
            .find(|g| g["id"].as_str() == Some(id))
            .cloned()
            .unwrap_or_else(|| panic!("goal {id} lost"))
    };
    let goal1 = find(&g1);
    let goal2 = find(&g2);
    assert_eq!(
        goal1["status"], "in_progress",
        "writer 1's update was clobbered by writer 2: {goal1:?}"
    );
    assert_eq!(
        goal2["status"], "completed",
        "writer 2's update was clobbered by writer 1: {goal2:?}"
    );
}
