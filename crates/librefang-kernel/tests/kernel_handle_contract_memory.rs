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
