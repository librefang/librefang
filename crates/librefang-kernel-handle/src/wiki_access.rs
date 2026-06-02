use super::*;

// ============================================================================
// 2b. WikiAccess — durable markdown knowledge vault (issue #3329)
// ============================================================================
//
// `WikiAccess` mirrors `MemoryAccess` but targets the `librefang-memory-wiki`
// vault instead of the SQLite/vector substrate. Results cross the seam as
// `serde_json::Value` so this trait does not need to depend on
// `librefang-memory-wiki`; the kernel impl serialises owned vault types
// (`WikiPage`, `SearchHit`, `WikiWriteOutcome`) before returning. Each method
// returns `KernelOpError::unavailable(...)` by default so test stubs keep
// compiling unchanged when `[memory_wiki]` is off (the kernel-side impl
// overrides these only when the vault is constructed).

pub trait WikiAccess: Send + Sync {
    /// Fetch a single wiki page. Returns a JSON object of the shape
    /// `{ "topic": ..., "frontmatter": { ... }, "body": "..." }`.
    ///
    /// `KernelOpError::unavailable("wiki")` when the vault is disabled,
    /// and `KernelOpError::not_found(topic)` when the topic does not exist.
    fn wiki_get(&self, topic: &str) -> Result<serde_json::Value, KernelOpError> {
        let _ = topic;
        Err(KernelOpError::unavailable("wiki_get"))
    }

    /// Naive case-insensitive substring search across every page body.
    /// Returns a JSON array of `{ "topic": ..., "snippet": ..., "score": ... }`
    /// sorted by score descending; topic-name hits outrank body hits.
    fn wiki_search(&self, query: &str, limit: usize) -> Result<serde_json::Value, KernelOpError> {
        let _ = (query, limit);
        Err(KernelOpError::unavailable("wiki_search"))
    }

    /// Write or update a wiki page.
    ///
    /// `body` may use `[[topic]]` placeholders for cross-references — the
    /// vault rewrites them according to its render mode (`native` keeps the
    /// markdown link form `[topic](topic.md)`; `obsidian` keeps `[[topic]]`).
    ///
    /// `provenance` must be a JSON object carrying at least `agent` (string)
    /// and may carry `session`, `channel`, `turn`, `at` (RFC 3339). The
    /// vault appends it to the existing provenance list — provenance is
    /// monotonic, never overwritten.
    ///
    /// `force = false` (default) refuses to silently overwrite a page whose
    /// on-disk mtime *or* sha256 has drifted since the last compiler run —
    /// the caller gets `KernelOpError::conflict(...)` so they can re-read
    /// the file before deciding what to do. `force = true` preserves the
    /// external body and only appends the new provenance entry.
    fn wiki_write(
        &self,
        topic: &str,
        body: &str,
        provenance: serde_json::Value,
        force: bool,
    ) -> Result<serde_json::Value, KernelOpError> {
        let _ = (topic, body, provenance, force);
        Err(KernelOpError::unavailable("wiki_write"))
    }
}
