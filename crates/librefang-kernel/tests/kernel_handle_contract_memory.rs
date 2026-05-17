//! Contract tests for the MemoryAccess trait — per-agent isolation,
//! cross-agent prevention, invalid-input rejection, legacy fallback,
//! and shared-namespace backward compatibility.

use librefang_kernel_handle::KernelHandle;

mod common;

use common::boot_kernel as boot;

const AGENT_A: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1";
const AGENT_B: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1";

#[test]
fn test_per_agent_isolation_writes_and_recalls() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    // Agent A writes
    kh.memory_store("key1", serde_json::json!("a-val"), Some(AGENT_A), None)
        .unwrap();
    // Agent B writes different value under same key
    kh.memory_store("key1", serde_json::json!("b-val"), Some(AGENT_B), None)
        .unwrap();

    // Each agent sees only its own value
    assert_eq!(
        kh.memory_recall("key1", Some(AGENT_A), None).unwrap(),
        Some(serde_json::json!("a-val"))
    );
    assert_eq!(
        kh.memory_recall("key1", Some(AGENT_B), None).unwrap(),
        Some(serde_json::json!("b-val"))
    );
    // None (shared namespace) is independent
    assert_eq!(kh.memory_recall("key1", None, None).unwrap(), None);
}

#[test]
fn test_per_agent_isolation_prevents_cross_agent_read() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.memory_store("secret", serde_json::json!("a-data"), Some(AGENT_A), None)
        .unwrap();
    assert_eq!(
        kh.memory_recall("secret", Some(AGENT_B), None).unwrap(),
        None,
        "Agent B must not read Agent A's data"
    );
}

#[test]
fn test_per_agent_isolation_prevents_cross_agent_list() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.memory_store("a-key", serde_json::json!(1), Some(AGENT_A), None)
        .unwrap();
    kh.memory_store("b-key", serde_json::json!(2), Some(AGENT_B), None)
        .unwrap();

    let a_keys = kh.memory_list(Some(AGENT_A), None).unwrap();
    let b_keys = kh.memory_list(Some(AGENT_B), None).unwrap();
    assert!(a_keys.contains(&"a-key".to_string()));
    assert!(!a_keys.contains(&"b-key".to_string()));
    assert!(b_keys.contains(&"b-key".to_string()));
    assert!(!b_keys.contains(&"a-key".to_string()));
}

#[test]
fn test_invalid_agent_id_returns_error() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let err = kh
        .memory_store("k", serde_json::json!(1), Some("not-a-uuid"), None)
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid agent_id"),
        "Expected InvalidInput for invalid agent_id, got: {msg}"
    );
}

#[test]
fn test_empty_agent_id_returns_error() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    let err = kh
        .memory_store("k", serde_json::json!(1), Some(""), None)
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("empty string"),
        "Expected InvalidInput for empty agent_id, got: {msg}"
    );
}

#[test]
fn test_cross_agent_and_cross_peer_composition() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    // Agent A stores peer-scoped "pref" under peer-1
    kh.memory_store(
        "pref",
        serde_json::json!("a-pref"),
        Some(AGENT_A),
        Some("peer-1"),
    )
    .unwrap();
    // Agent B stores the SAME peer and key
    kh.memory_store(
        "pref",
        serde_json::json!("b-pref"),
        Some(AGENT_B),
        Some("peer-1"),
    )
    .unwrap();

    assert_eq!(
        kh.memory_recall("pref", Some(AGENT_A), Some("peer-1"))
            .unwrap(),
        Some(serde_json::json!("a-pref"))
    );
    assert_eq!(
        kh.memory_recall("pref", Some(AGENT_B), Some("peer-1"))
            .unwrap(),
        Some(serde_json::json!("b-pref"))
    );
    // Agent B must not see Agent A's peer-scoped key
    assert_eq!(
        kh.memory_recall("pref", Some(AGENT_B), Some("peer-1"))
            .unwrap(),
        Some(serde_json::json!("b-pref")),
        "Agent B must not see Agent A's peer-scoped key"
    );
}

#[test]
fn test_shared_namespace_still_works_for_backward_compat() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    // agent_id=None writes to shared namespace
    kh.memory_store("global", serde_json::json!("v"), None, None)
        .unwrap();
    assert_eq!(
        kh.memory_recall("global", None, None).unwrap(),
        Some(serde_json::json!("v"))
    );
}

#[test]
fn test_memory_store_recall_isolates_peer_namespaces() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.memory_store("k", serde_json::json!("a"), None, Some("peer-a"))
        .unwrap();
    kh.memory_store("k", serde_json::json!("b"), None, Some("peer-b"))
        .unwrap();

    // Peer-a sees its own value
    assert_eq!(
        kh.memory_recall("k", None, Some("peer-a")).unwrap(),
        Some(serde_json::json!("a"))
    );
    // Peer-b sees its own value
    assert_eq!(
        kh.memory_recall("k", None, Some("peer-b")).unwrap(),
        Some(serde_json::json!("b"))
    );
    // Unscoped none sees neither (both are peer-scoped)
    assert_eq!(kh.memory_recall("k", None, None).unwrap(), None);
}

#[test]
fn test_memory_list_separates_global_and_peer_keys() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    kh.memory_store("global", serde_json::json!(1), None, None)
        .unwrap();
    kh.memory_store("peer-k", serde_json::json!(2), None, Some("peer-x"))
        .unwrap();

    // Global list (no peer_id) hides peer-scoped keys
    let global_keys = kh.memory_list(None, None).unwrap();
    assert!(global_keys.contains(&"global".to_string()));
    assert!(!global_keys.contains(&"peer-k".to_string()));

    // Peer-scoped list shows the prefixed key stripped
    let peer_keys = kh.memory_list(None, Some("peer-x")).unwrap();
    assert!(peer_keys.contains(&"peer-k".to_string()));
    assert!(!peer_keys.contains(&"global".to_string()));
}

#[test]
fn test_memory_recall_nonexistent_key_returns_none() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    assert_eq!(
        kh.memory_recall("nonexistent", None, None)
            .expect("recall nonexistent"),
        None
    );
    assert_eq!(
        kh.memory_recall("nonexistent", None, Some("peer-x"))
            .expect("recall nonexistent peer"),
        None
    );
}

#[test]
fn test_memory_recall_falls_back_to_legacy_shared_namespace() {
    let (kernel, _tmp) = boot();
    let kh: &dyn KernelHandle = &kernel;

    // Write directly to the shared namespace via the trait
    kh.memory_store("legacy_k", serde_json::json!("legacy"), None, None)
        .unwrap();
    // Per-agent recall misses the agent's own row but should fall back
    assert_eq!(
        kh.memory_recall("legacy_k", Some(AGENT_A), None).unwrap(),
        Some(serde_json::json!("legacy"))
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
        .memory_store(
            "peer:victim:user_name",
            serde_json::json!("planted"),
            None,
            None,
        )
        .expect_err("LLM-supplied 'peer:' key must be rejected (#5120)");
    assert!(
        matches!(err, KernelOpError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );

    // SIDE EFFECT: the planted row truly didn't land — `victim`'s namespace
    // must stay empty.
    let victim_keys = kh
        .memory_list(None, Some("victim"))
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
        .memory_recall("peer:victim:user_name", None, None)
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
        .memory_list(None, Some("u:42"))
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
        .memory_store("user_name", serde_json::json!("alice"), None, Some("T1:U2"))
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
        .memory_store("user_name", serde_json::json!("alice"), None, Some(""))
        .expect_err("empty peer_id must be rejected on store (#5119)");
    assert!(
        matches!(store_err, KernelOpError::InvalidInput(_)),
        "expected InvalidInput, got {store_err:?}"
    );

    let recall_err = kh
        .memory_recall("user_name", None, Some(""))
        .expect_err("empty peer_id must be rejected on recall (#5119)");
    assert!(matches!(recall_err, KernelOpError::InvalidInput(_)));

    let list_err = kh
        .memory_list(None, Some(""))
        .expect_err("empty peer_id must be rejected on list (#5119)");
    assert!(matches!(list_err, KernelOpError::InvalidInput(_)));

    // SIDE EFFECT: the global namespace was never written through the
    // ambiguous empty-peer path.
    let global_keys = kh.memory_list(None, None).expect("list global");
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

    kh.memory_store("user_name", serde_json::json!("alice"), None, Some("u"))
        .expect("colon-free peer_id with regular key must succeed");
    assert_eq!(
        kh.memory_recall("user_name", None, Some("u"))
            .expect("recall colon-free peer"),
        Some(serde_json::json!("alice"))
    );
    let keys = kh
        .memory_list(None, Some("u"))
        .expect("list colon-free peer");
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
        .memory_list(None, Some("victim"))
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
    let collision = kh.memory_list(None, Some("victim:sub"));
    assert!(
        matches!(collision, Err(KernelOpError::InvalidInput(_))),
        "colon-bearing list query must be rejected outright, got {collision:?}"
    );
}
