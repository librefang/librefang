//! Out-of-process memory extractor.
//!
//! `SidecarMemoryExtractor` implements
//! [`MemoryExtractor`](librefang_types::memory::MemoryExtractor) by delegating
//! extraction to a subprocess over the shared
//! [`SupervisedTransport`](librefang_subprocess::SupervisedTransport).
//!
//! Unlike compaction or the LLM-bearing context-engine `compact` hook — which
//! must reuse the daemon's configured driver (cost accounting, streaming,
//! prompt cache) and so stay in Rust — memory extraction is a background,
//! non-streaming, fire-and-forget task. A sidecar is free to do extraction
//! however it likes (its own LLM key, a cheap local model, embeddings, a
//! vector DB) and hand back the structured result; the daemon keeps the
//! substrate (the SQLite store) and the dedup decision (`decide_action`'s
//! default heuristic) in Rust.
//!
//! # Wire protocol
//!
//! Request (one JSON object per line, via the shared transport):
//! `{"id", "method": "extract_memories", "params": {"messages": […],
//! "categories": […]}}`.
//! Reply: `{"id", "ok": {"memories": [{"content", "category"?, "level"?,
//! "metadata"?}, …], "relations": [<RelationTriple>]?, "has_content"?,
//! "trigger"?}}` or `{"id", "error": "<msg>"}`. The sidecar returns *simple*
//! memory items — the daemon assigns each a UUID and `created_at` and stamps
//! `source = "sidecar"`, so a sidecar author never invents ids or timestamps.

use async_trait::async_trait;
use librefang_subprocess::{SupervisedTransport, TransportConfig};
use librefang_types::error::LibreFangResult;
use librefang_types::memory::{
    ExtractionResult, MemoryExtractor, MemoryItem, MemoryLevel, RelationTriple,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use tracing::warn;

/// A [`MemoryExtractor`] backed by an out-of-process implementation.
///
/// The dedup decision (`decide_action`) is intentionally left to the trait's
/// default heuristic — the sidecar's job is extraction, not the store's
/// conflict resolution.
pub struct SidecarMemoryExtractor {
    transport: SupervisedTransport,
}

/// The simple shape a sidecar returns; mapped to [`MemoryItem`] by the daemon.
#[derive(Deserialize)]
struct SidecarExtraction {
    #[serde(default)]
    memories: Vec<SidecarMemory>,
    #[serde(default)]
    relations: Vec<RelationTriple>,
    #[serde(default)]
    has_content: bool,
    #[serde(default)]
    trigger: String,
}

#[derive(Deserialize)]
struct SidecarMemory {
    content: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    level: MemoryLevel,
    #[serde(default)]
    metadata: HashMap<String, serde_json::Value>,
}

impl SidecarMemory {
    fn into_item(self) -> MemoryItem {
        // `MemoryItem::new` assigns the id, created_at, etc.; the sidecar only
        // supplies content/category/level/metadata.
        let mut item = MemoryItem::new(self.content, self.level);
        item.category = self.category;
        item.metadata = self.metadata;
        item.source = Some("sidecar".to_string());
        item
    }
}

impl SidecarMemoryExtractor {
    /// Build an extractor that talks to `command`/`args`. The child is spawned
    /// lazily on first use and re-spawned after a crash (see
    /// `SupervisedTransport`); a `request_timeout_secs` of 0 uses 30s.
    pub fn new(command: String, args: Vec<String>, request_timeout_secs: u64) -> Self {
        let timeout = Duration::from_secs(if request_timeout_secs == 0 {
            30
        } else {
            request_timeout_secs
        });
        let transport = SupervisedTransport::new(TransportConfig::new(
            command,
            args,
            timeout,
            "memory_extractor",
        ));
        Self { transport }
    }

    /// An empty result — used when the sidecar is unavailable, so a down
    /// extractor degrades to "nothing memorized this turn" rather than erroring
    /// the (best-effort, post-turn) auto-memorize path.
    fn empty() -> ExtractionResult {
        ExtractionResult {
            memories: Vec::new(),
            relations: Vec::new(),
            has_content: false,
            trigger: String::new(),
            conflicts: Vec::new(),
        }
    }
}

#[async_trait]
impl MemoryExtractor for SidecarMemoryExtractor {
    async fn extract_memories(
        &self,
        messages: &[serde_json::Value],
        categories: &[String],
    ) -> LibreFangResult<ExtractionResult> {
        let params = json!({
            "method": "extract_memories",
            "params": { "messages": messages, "categories": categories },
        });
        let value = match self.transport.request(params).await {
            Ok(value) => value,
            Err(e) => {
                warn!(error = %e,
                    "memory extractor sidecar call failed; memorizing nothing this turn");
                return Ok(Self::empty());
            }
        };
        match serde_json::from_value::<SidecarExtraction>(value) {
            Ok(ext) => {
                let memories: Vec<MemoryItem> = ext
                    .memories
                    .into_iter()
                    .map(SidecarMemory::into_item)
                    .collect();
                let has_content = ext.has_content || !memories.is_empty();
                Ok(ExtractionResult {
                    memories,
                    relations: ext.relations,
                    has_content,
                    trigger: ext.trigger,
                    conflicts: Vec::new(),
                })
            }
            Err(e) => {
                warn!(error = %e,
                    "memory extractor sidecar: unparseable reply; memorizing nothing");
                Ok(Self::empty())
            }
        }
    }

    // `extract_memories_with_agent_id` and `decide_action` use the trait
    // defaults: the sidecar doesn't route through a forked agent turn, and the
    // store's heuristic dedup stays in Rust.

    fn format_context(&self, memories: &[MemoryItem]) -> String {
        // Pure formatting — no sidecar round-trip. Mirrors the built-in
        // extractors so recalled memories render identically regardless of who
        // extracted them.
        librefang_types::memory::format_memories_with_budget(
            memories,
            librefang_types::memory::FORMAT_CONTEXT_MAX_CHARS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python() -> Option<&'static str> {
        ["python3", "python"].into_iter().find(|cmd| {
            std::process::Command::new(cmd)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
    }

    #[tokio::test]
    async fn extracts_via_sidecar() {
        let Some(py) = python() else {
            eprintln!("skipping: no python3");
            return;
        };
        // Returns one simple memory echoing the message count, in the first
        // category — proving params reach the sidecar and the daemon maps the
        // simple item into a full MemoryItem.
        let body = r#"
import sys, json
while True:
    line = sys.stdin.readline()
    if not line:
        break
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    p = req.get("params", {})
    n = len(p.get("messages", []))
    cat = (p.get("categories") or ["fact"])[0]
    ok = {"memories": [{"content": f"seen {n} messages", "category": cat}], "has_content": True}
    sys.stdout.write(json.dumps({"id": req["id"], "ok": ok}) + "\n")
    sys.stdout.flush()
"#;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("extract.py");
        std::fs::write(&script, body).unwrap();

        let extractor = SidecarMemoryExtractor::new(
            py.to_string(),
            vec![script.to_str().unwrap().to_string()],
            5,
        );
        let messages = vec![json!({"role": "user", "content": "hi"})];
        let result = extractor
            .extract_memories(&messages, &["fact".to_string()])
            .await
            .unwrap();

        assert!(result.has_content);
        assert_eq!(result.memories.len(), 1);
        assert_eq!(result.memories[0].content, "seen 1 messages");
        assert_eq!(result.memories[0].category.as_deref(), Some("fact"));
        // The daemon, not the sidecar, assigns the id and provenance.
        assert!(!result.memories[0].id.is_empty());
        assert_eq!(result.memories[0].source.as_deref(), Some("sidecar"));
    }

    #[tokio::test]
    async fn missing_sidecar_memorizes_nothing() {
        let extractor =
            SidecarMemoryExtractor::new("/nonexistent/memory-extractor".to_string(), vec![], 5);
        let result = extractor
            .extract_memories(
                &[json!({"role": "user", "content": "hi"})],
                &["fact".to_string()],
            )
            .await
            .unwrap();
        assert!(!result.has_content);
        assert!(result.memories.is_empty());
    }
}
