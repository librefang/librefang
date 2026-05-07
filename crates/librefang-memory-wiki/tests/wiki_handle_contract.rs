//! `WikiAccess` trait-shape contract test (issue #3329).
//!
//! `librefang-kernel-handle` defines the JSON shape that `wiki_get`,
//! `wiki_search`, and `wiki_write` must return when a kernel impl wires
//! the vault, but the kernel-handle crate has no vault to test against.
//! `librefang-kernel` has a real impl but lives behind dev-deps the
//! sandboxed docker image cannot build (libdbus / gdk).
//!
//! This test bridges the gap. It implements `WikiAccess` on a thin
//! wrapper around `Option<Arc<WikiVault>>` — mirroring the production
//! kernel-side adaptor verbatim — and asserts the JSON shape every
//! caller (tool dispatcher, future HTTP route, dashboard) is allowed to
//! rely on. Drift between the kernel impl and this shadow impl gets
//! flagged at PR time instead of at first prod call.

use std::sync::Arc;

use librefang_kernel_handle::{KernelOpError, WikiAccess};
use librefang_memory_wiki::{
    MemoryWikiIngestFilter, ProvenanceEntry, RenderMode, WikiError, WikiVault,
};
use serde_json::Value;
use tempfile::TempDir;

struct WikiHandle(Option<Arc<WikiVault>>);

impl WikiAccess for WikiHandle {
    fn wiki_get(&self, topic: &str) -> Result<Value, KernelOpError> {
        let vault = self
            .0
            .as_ref()
            .ok_or_else(|| KernelOpError::unavailable("wiki_get"))?;
        match vault.get(topic) {
            Ok(page) => serde_json::to_value(&page)
                .map_err(|e| KernelOpError::Internal(format!("Wiki get serialize: {e}"))),
            Err(WikiError::NotFound(_)) => Err(KernelOpError::Internal(format!(
                "wiki topic `{topic}` not found"
            ))),
            Err(err) => Err(KernelOpError::Internal(format!("Wiki get failed: {err}"))),
        }
    }

    fn wiki_search(&self, query: &str, limit: usize) -> Result<Value, KernelOpError> {
        let vault = self
            .0
            .as_ref()
            .ok_or_else(|| KernelOpError::unavailable("wiki_search"))?;
        let hits = vault
            .search(query, limit)
            .map_err(|e| KernelOpError::Internal(format!("Wiki search failed: {e}")))?;
        serde_json::to_value(&hits)
            .map_err(|e| KernelOpError::Internal(format!("Wiki search serialize: {e}")))
    }

    fn wiki_write(
        &self,
        topic: &str,
        body: &str,
        provenance: Value,
        force: bool,
    ) -> Result<Value, KernelOpError> {
        let vault = self
            .0
            .as_ref()
            .ok_or_else(|| KernelOpError::unavailable("wiki_write"))?;
        let prov: ProvenanceEntry = serde_json::from_value(provenance).map_err(|e| {
            KernelOpError::InvalidInput(format!(
                "wiki_write `provenance` must be {{agent, [session], [channel], [turn], at}}: {e}"
            ))
        })?;
        match vault.write(topic, body, prov, force) {
            Ok(outcome) => serde_json::to_value(&outcome)
                .map_err(|e| KernelOpError::Internal(format!("Wiki write serialize: {e}"))),
            Err(WikiError::HandEditConflict { topic }) => Err(KernelOpError::Internal(format!(
                "wiki page `{topic}` was edited externally; re-read the file or pass force=true"
            ))),
            Err(WikiError::InvalidTopic { topic, reason }) => Err(KernelOpError::InvalidInput(
                format!("wiki_write topic `{topic}`: {reason}"),
            )),
            Err(WikiError::BodyTooLarge { topic, size, cap }) => {
                Err(KernelOpError::InvalidInput(format!(
                    "wiki_write body for `{topic}` is {size} bytes; exceeds the {cap}-byte cap"
                )))
            }
            Err(err) => Err(KernelOpError::Internal(format!("Wiki write failed: {err}"))),
        }
    }
}

fn vault_handle(dir: &TempDir) -> WikiHandle {
    let vault = WikiVault::with_root(
        dir.path().to_path_buf(),
        RenderMode::Native,
        MemoryWikiIngestFilter::Tagged,
    )
    .unwrap();
    WikiHandle(Some(Arc::new(vault)))
}

#[test]
fn disabled_handle_returns_per_method_unavailable() {
    let handle = WikiHandle(None);
    match handle.wiki_get("x") {
        Err(KernelOpError::Unavailable(c)) if c == "wiki_get" => {}
        other => panic!("expected Unavailable(\"wiki_get\"), got {other:?}"),
    }
    match handle.wiki_search("x", 10) {
        Err(KernelOpError::Unavailable(c)) if c == "wiki_search" => {}
        other => panic!("expected Unavailable(\"wiki_search\"), got {other:?}"),
    }
    match handle.wiki_write("x", "y", serde_json::json!({}), false) {
        Err(KernelOpError::Unavailable(c)) if c == "wiki_write" => {}
        other => panic!("expected Unavailable(\"wiki_write\"), got {other:?}"),
    }
}

#[test]
fn wiki_write_response_shape_is_stable() {
    let dir = TempDir::new().unwrap();
    let handle = vault_handle(&dir);
    let prov = serde_json::json!({
        "agent": "agent_x",
        "at": chrono::Utc::now().to_rfc3339(),
    });
    let value = handle
        .wiki_write("widgets", "the body", prov, false)
        .unwrap();
    let obj = value.as_object().expect("write returns a JSON object");
    assert_eq!(obj.get("topic").and_then(Value::as_str), Some("widgets"));
    assert!(obj.get("path").and_then(Value::as_str).is_some());
    assert!(obj
        .get("content_sha256")
        .and_then(Value::as_str)
        .map(|s| s.len() == 64)
        .unwrap_or(false));
    assert_eq!(
        obj.get("merged_with_external_edit"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn wiki_write_rejects_malformed_provenance_with_invalid_input() {
    let dir = TempDir::new().unwrap();
    let handle = vault_handle(&dir);
    // No `agent` field — must surface as InvalidInput, not Internal.
    let bad = serde_json::json!({"not_an_agent": 7});
    match handle.wiki_write("topic", "body", bad, false) {
        Err(KernelOpError::InvalidInput(msg)) => {
            assert!(
                msg.contains("provenance"),
                "msg should mention provenance: {msg}"
            );
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn wiki_get_returns_topic_frontmatter_body_object() {
    let dir = TempDir::new().unwrap();
    let handle = vault_handle(&dir);
    let prov = serde_json::json!({"agent": "a", "at": chrono::Utc::now().to_rfc3339()});
    handle
        .wiki_write("notes", "real body", prov, false)
        .unwrap();

    let value = handle.wiki_get("notes").unwrap();
    let obj = value.as_object().expect("get returns a JSON object");
    assert_eq!(obj.get("topic").and_then(Value::as_str), Some("notes"));
    assert!(obj.get("body").and_then(Value::as_str).is_some());
    let fm = obj
        .get("frontmatter")
        .and_then(Value::as_object)
        .expect("frontmatter is an object");
    assert_eq!(fm.get("topic").and_then(Value::as_str), Some("notes"));
    assert!(fm.get("created").is_some());
    assert!(fm.get("updated").is_some());
    assert!(fm.get("content_sha256").is_some());
    let provenance = fm
        .get("provenance")
        .and_then(Value::as_array)
        .expect("provenance is an array");
    assert_eq!(provenance.len(), 1);
    assert_eq!(
        provenance[0].get("agent").and_then(Value::as_str),
        Some("a")
    );
}

#[test]
fn wiki_search_returns_array_of_topic_snippet_score_objects() {
    let dir = TempDir::new().unwrap();
    let handle = vault_handle(&dir);
    let prov = serde_json::json!({"agent": "a", "at": chrono::Utc::now().to_rfc3339()});
    handle
        .wiki_write("alpha", "the quick brown fox", prov.clone(), false)
        .unwrap();
    handle
        .wiki_write("beta", "lorem ipsum dolor", prov, false)
        .unwrap();

    let value = handle.wiki_search("fox", 10).unwrap();
    let arr = value.as_array().expect("search returns a JSON array");
    assert!(!arr.is_empty(), "fox should match alpha");
    let hit = arr[0].as_object().expect("each hit is a JSON object");
    assert!(hit.get("topic").and_then(Value::as_str).is_some());
    assert!(hit.get("snippet").and_then(Value::as_str).is_some());
    assert!(hit.get("score").and_then(Value::as_f64).is_some());
}
