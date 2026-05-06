//! Three-layer tool result budget enforcement.
//!
//! Defense against context-window overflow from large tool outputs:
//!
//! 1. **Layer 1 (per-tool)**: Each tool pre-truncates its own output before
//!    returning. This is handled inside individual tool implementations and is
//!    not the responsibility of this module.
//!
//! 2. **Layer 2 (per-result)**: After a tool returns, if its output exceeds
//!    [`PER_RESULT_THRESHOLD`] (default 50 KB), the full content is spilled
//!    to the **artifact store** ([`crate::artifact_store::maybe_spill`]) and
//!    replaced in context with a compact `[tool_result: ... | sha256:... |
//!    N bytes | preview:]` stub.  The handle is content-addressed so the
//!    LLM can fetch the original via `read_artifact(handle, offset, length)`.
//!    Fallback: if the write fails, the content is truncated inline.
//!
//! 3. **Layer 3 (per-turn aggregate)**: After all tool results in a single
//!    assistant turn have been collected, if their combined size exceeds
//!    [`PER_TURN_BUDGET`] (default 200 KB), the largest non-persisted results
//!    are spilled to the same artifact store in descending-size order until
//!    the aggregate is under budget.
//!
//! # Why the artifact store
//!
//! Earlier drafts wrote both layers to a parallel `/tmp/librefang-results/`
//! directory with `.txt` filenames keyed by `tool_use_id`.  That had three
//! problems:
//! * the LLM had no tool to read those files back — `read_artifact` only
//!   accepts `sha256:<hex>` handles, so Layer 2/3-spilled data was
//!   permanently lost from the model's perspective;
//! * the `.txt` files weren't in the artifact-store GC's `.bin`/`.tmp`
//!   allowlist, so long-running daemons accumulated the directory forever;
//! * the two stub formats (`[Tool output too large ...]` vs `[tool_result:
//!   ...]`) split the prompt-engineering surface for no benefit.
//!
//! Routing both layers through `crate::artifact_store::maybe_spill` unifies
//! the spill format, makes every spilled result `read_artifact`-recoverable,
//! and lets the existing startup GC reclaim the bytes.

use std::path::PathBuf;
use tracing::{debug, warn};

/// Default per-result persistence threshold (50 KB).
pub const PER_RESULT_THRESHOLD: usize = 50 * 1024;

/// Default per-turn aggregate budget (200 KB).
pub const PER_TURN_BUDGET: usize = 200 * 1024;

/// Marker substring used to detect already-spilled results (Layer 3 skip
/// guard).  Matches the prefix of the stub produced by
/// [`crate::artifact_store::build_spill_stub`].
const PERSISTED_MARKER: &str = "[tool_result:";

/// A single tool result entry used by the per-turn budget enforcer.
#[derive(Debug)]
pub struct ToolResultEntry {
    /// The `tool_use_id` for this result.  Forwarded to the artifact-store
    /// spill so per-tool-call traces show up in spill log lines (see
    /// `tracing::debug!(tool_use_id = …)` below).  Not used as a filename
    /// stem any more — `artifact_store` is content-addressed.
    pub tool_use_id: String,
    /// Content of the result. May be replaced in-place by the enforcer.
    pub content: String,
}

/// Enforces per-result and per-turn-aggregate size budgets on tool outputs.
///
/// Constructed once per agent loop instantiation and reused across turns.
/// All spill I/O is delegated to [`crate::artifact_store`] so:
/// * spilled results are `read_artifact`-recoverable from the LLM
/// * the artifact-store startup GC reclaims old bytes (#3347 4/N)
/// * every spilled tool result lands under the same content-addressed
///   directory, regardless of which layer triggered the write.
pub struct ToolBudgetEnforcer {
    /// Layer 2 threshold: results larger than this are spilled.
    pub per_result_threshold: usize,
    /// Layer 3 threshold: if total bytes across all results in a turn
    /// exceeds this, the largest non-persisted results are spilled.
    pub per_turn_budget: usize,
    /// Per-artifact byte cap forwarded to
    /// [`crate::artifact_store::maybe_spill`].  Above this the spill is
    /// rejected and the enforcer falls back to inline truncation.
    pub max_artifact_bytes: u64,
    /// Resolved canonical artifact-store directory.  Same value as
    /// [`crate::artifact_store::default_artifact_storage_dir`]; cached on
    /// the enforcer so we don't re-resolve env vars per tool call.
    artifact_dir: PathBuf,
}

impl Default for ToolBudgetEnforcer {
    fn default() -> Self {
        Self::new(
            PER_RESULT_THRESHOLD,
            PER_TURN_BUDGET,
            crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
        )
    }
}

impl ToolBudgetEnforcer {
    /// Create an enforcer with custom thresholds.  `artifact_dir` resolves
    /// to [`crate::artifact_store::default_artifact_storage_dir`] so reader
    /// (`read_artifact`), writer (this enforcer + web-tool spill), and the
    /// startup GC all touch the same directory.  Long history: the writer
    /// path used to point at `/tmp/librefang-results/` while the reader
    /// only knew about `~/.librefang/data/artifacts/`, which is why
    /// Layer-3-spilled data was unreachable from the LLM.
    pub fn new(
        per_result_threshold: usize,
        per_turn_budget: usize,
        max_artifact_bytes: u64,
    ) -> Self {
        Self {
            per_result_threshold,
            per_turn_budget,
            max_artifact_bytes,
            artifact_dir: crate::artifact_store::default_artifact_storage_dir(),
        }
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Layer 2: per-result
    // ──────────────────────────────────────────────────────────────────────────

    /// Apply Layer 2 budget to a single tool result.
    ///
    /// If `content` is within the threshold, it is returned unchanged.
    /// Otherwise the full content is spilled to the artifact store and a
    /// compact `[tool_result: ... | sha256:... | N bytes | preview:]` stub
    /// is returned instead — recoverable by the LLM via `read_artifact`.
    ///
    /// **Fallback**: if the spill fails (per-artifact cap exceeded, disk
    /// full), the content is truncated inline to `per_result_threshold`
    /// bytes.  Never panics.
    pub fn maybe_persist_result(&self, content: &str, tool_use_id: &str) -> String {
        if content.len() <= self.per_result_threshold {
            return content.to_string();
        }

        let original_len = content.len();
        let bytes = content.as_bytes();
        match crate::artifact_store::maybe_spill(
            // The artifact-store stub embeds `tool_name`; we pass
            // `tool_use_id` here so spilled stubs trace back to the
            // originating tool call instead of being labelled "unknown".
            tool_use_id,
            bytes,
            self.per_result_threshold as u64,
            self.max_artifact_bytes,
            &self.artifact_dir,
        ) {
            Some(stub) => {
                debug!(
                    tool_use_id,
                    bytes = original_len,
                    "tool_budget: spilled oversized result to artifact store (Layer 2)"
                );
                stub
            }
            None => {
                warn!(
                    tool_use_id,
                    bytes = original_len,
                    "tool_budget: artifact spill failed at Layer 2, falling back to inline truncation"
                );
                inline_truncate(content, self.per_result_threshold)
            }
        }
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Layer 3: per-turn aggregate
    // ──────────────────────────────────────────────────────────────────────────

    /// Apply Layer 3 budget across all results collected in one assistant turn.
    ///
    /// If the total byte count of all entries is within [`Self::per_turn_budget`],
    /// this is a no-op. Otherwise the largest non-persisted results are spilled
    /// to disk (largest first) until the aggregate is under budget.
    ///
    /// Already-persisted results (those whose content starts with the
    /// [`PERSISTED_MARKER`]) are counted toward the total but are never
    /// re-persisted.
    pub fn enforce_turn_budget(&self, results: &mut [ToolResultEntry]) {
        let total: usize = results.iter().map(|r| r.content.len()).sum();
        if total <= self.per_turn_budget {
            return;
        }

        debug!(
            total_bytes = total,
            budget = self.per_turn_budget,
            "tool_budget: per-turn budget exceeded, spilling largest results (Layer 3)"
        );

        // Build a candidate list: (index, size) for non-persisted results,
        // sorted largest-first.
        let mut candidates: Vec<(usize, usize)> = results
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.content.starts_with(PERSISTED_MARKER))
            .map(|(i, r)| (i, r.content.len()))
            .collect();
        candidates.sort_by_key(|b| std::cmp::Reverse(b.1));

        let mut running_total = total;

        for (idx, size) in candidates {
            if running_total <= self.per_turn_budget {
                break;
            }

            let entry = &mut results[idx];
            let bytes = entry.content.as_bytes();
            // Layer 3 spills with `threshold = 1` so any non-empty content
            // is materialised — the budget exceedance is the gate, not the
            // size of an individual result. `max_artifact_bytes` and the
            // shared `artifact_dir` route the spill through the same
            // content-addressed store as Layer 2.
            let replacement = match crate::artifact_store::maybe_spill(
                &entry.tool_use_id,
                bytes,
                1,
                self.max_artifact_bytes,
                &self.artifact_dir,
            ) {
                Some(stub) => {
                    debug!(
                        tool_use_id = %entry.tool_use_id,
                        bytes = size,
                        "tool_budget: spilled result for turn budget (Layer 3)"
                    );
                    stub
                }
                None => {
                    warn!(
                        tool_use_id = %entry.tool_use_id,
                        bytes = size,
                        "tool_budget: artifact spill failed at Layer 3, truncating inline"
                    );
                    inline_truncate(&entry.content, self.per_result_threshold)
                }
            };

            running_total = running_total - size + replacement.len();
            entry.content = replacement;
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Free helpers (pure, no I/O)
// ──────────────────────────────────────────────────────────────────────────────

/// Truncate `content` to at most `max_bytes` UTF-8 bytes (snapping to a char
/// boundary) and append a notice. Used as the fallback when artifact spill
/// fails.
fn inline_truncate(content: &str, max_bytes: usize) -> String {
    let truncated = truncate_to_byte_boundary(content, max_bytes);
    format!("{truncated}\n[Truncated: could not save full output]")
}

/// Return a `&str` slice of `s` that is at most `max_bytes` bytes long,
/// snapping back to the last valid UTF-8 char boundary.
fn truncate_to_byte_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk backwards from max_bytes to find a char boundary.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_enforcer(tmpdir: &std::path::Path) -> ToolBudgetEnforcer {
        ToolBudgetEnforcer {
            per_result_threshold: 100,
            per_turn_budget: 300,
            max_artifact_bytes: crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
            artifact_dir: tmpdir.to_path_buf(),
        }
    }

    #[test]
    fn layer2_small_result_passthrough() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = make_enforcer(dir.path());
        let content = "x".repeat(50);
        let result = enforcer.maybe_persist_result(&content, "id-1");
        assert_eq!(result, content);
        // No file should be written.
        assert!(dir.path().read_dir().unwrap().next().is_none());
    }

    #[test]
    fn layer2_large_result_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = make_enforcer(dir.path());
        let content = "y".repeat(200);
        let result = enforcer.maybe_persist_result(&content, "id-2");
        // Stub from `artifact_store::build_spill_stub` carries the
        // `PERSISTED_MARKER` prefix and an `sha256:<hex>` handle.
        assert!(result.starts_with(PERSISTED_MARKER));
        assert!(result.contains("sha256:"));
        assert!(result.contains("read_artifact"));
        // A `<hash>.bin` file should exist in the artifact dir.
        let any_bin = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "bin"));
        assert!(any_bin, "Layer 2 must materialise a .bin artifact");
    }

    #[test]
    #[cfg(unix)]
    fn layer2_fallback_when_artifact_cap_blocks_spill() {
        // Drive the spill into the fallback by setting `max_artifact_bytes`
        // below the content size — `artifact_store::write` rejects the
        // write and `maybe_spill` returns `None`.  Was previously a
        // bad-path test; since spill now goes through `artifact_store`,
        // the per-artifact cap is the cleanest way to trigger fallback.
        let dir = tempfile::tempdir().unwrap();
        let enforcer = ToolBudgetEnforcer {
            per_result_threshold: 10,
            per_turn_budget: 1000,
            max_artifact_bytes: 8, // smaller than `content`, write rejected
            artifact_dir: dir.path().to_path_buf(),
        };
        let content = "z".repeat(100);
        let result = enforcer.maybe_persist_result(&content, "bad-id");
        assert!(result.ends_with("[Truncated: could not save full output]"));
        assert!(result.len() <= 10 + 50); // truncated portion + notice
    }

    #[test]
    fn layer3_no_op_under_budget() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = make_enforcer(dir.path());
        let mut entries = vec![
            ToolResultEntry {
                tool_use_id: "a".into(),
                content: "x".repeat(50),
            },
            ToolResultEntry {
                tool_use_id: "b".into(),
                content: "y".repeat(50),
            },
        ];
        enforcer.enforce_turn_budget(&mut entries);
        // Nothing should change — total is 100, budget is 300.
        assert_eq!(entries[0].content.len(), 50);
        assert_eq!(entries[1].content.len(), 50);
    }

    #[test]
    fn layer3_spills_largest_first() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = make_enforcer(dir.path());
        // Total = 200 + 150 = 350 > budget (300).
        let mut entries = vec![
            ToolResultEntry {
                tool_use_id: "small".into(),
                content: "s".repeat(150),
            },
            ToolResultEntry {
                tool_use_id: "large".into(),
                content: "L".repeat(200),
            },
        ];
        enforcer.enforce_turn_budget(&mut entries);
        // The largest entry (200 bytes, index 1) should be persisted.
        let large_entry = entries.iter().find(|e| e.tool_use_id == "large").unwrap();
        assert!(large_entry.content.starts_with(PERSISTED_MARKER));
        assert!(large_entry.content.contains("sha256:"));
    }

    #[test]
    fn layer3_skips_already_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = make_enforcer(dir.path());
        // Synthesise a stub that already starts with `PERSISTED_MARKER`
        // (matches `artifact_store::build_spill_stub`'s output prefix),
        // so `enforce_turn_budget` recognises it as already-spilled and
        // skips it.
        let persisted_content = format!(
            "{} pretool | sha256:abcd | 99999 bytes | preview:]\nfoo\n-- truncated. Use read_artifact",
            PERSISTED_MARKER
        );
        let mut entries = vec![
            ToolResultEntry {
                tool_use_id: "persisted".into(),
                content: persisted_content.clone(),
            },
            ToolResultEntry {
                tool_use_id: "fresh".into(),
                content: "F".repeat(250),
            },
        ];
        // Total > 300, but "persisted" should not be touched.
        enforcer.enforce_turn_budget(&mut entries);
        assert_eq!(entries[0].content, persisted_content);
    }

    #[test]
    fn truncate_to_byte_boundary_ascii() {
        assert_eq!(truncate_to_byte_boundary("hello world", 5), "hello");
    }

    #[test]
    fn truncate_to_byte_boundary_multibyte() {
        // "日本語" is 9 bytes (3 bytes per char); truncate at 7 should give "日本" (6 bytes).
        let s = "日本語";
        let t = truncate_to_byte_boundary(s, 7);
        assert_eq!(t, "日本");
    }

    /// Verify that the per-turn budget enforcer counts post-spill (rewritten) bytes,
    /// not the original raw content size.  A 50 KB raw result is first collapsed to a
    /// compact summary stub by Layer 2 (`maybe_persist_result`).  When Layer 3
    /// (`enforce_turn_budget`) runs on the already-rewritten entries, the stub is far
    /// below the per-turn budget so no further spill occurs — confirming the enforcer
    /// operates on post-rewrite content.
    #[test]
    fn layer3_counts_post_spill_bytes_not_raw() {
        let dir = tempfile::tempdir().unwrap();
        // Layer 2 threshold = 1 KB; per-turn budget = 10 KB.
        let enforcer = ToolBudgetEnforcer {
            per_result_threshold: 1024,
            per_turn_budget: 10 * 1024,
            max_artifact_bytes: crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
            artifact_dir: dir.path().to_path_buf(),
        };

        // Simulate a ~50 KB raw result — well above per_result_threshold.
        let raw_50kb = "R".repeat(50 * 1024);

        // Layer 2: collapse to a persisted summary stub.
        let post_l2 = enforcer.maybe_persist_result(&raw_50kb, "tool-big");
        assert!(
            post_l2.starts_with(PERSISTED_MARKER),
            "Layer 2 should have persisted the large result"
        );
        // The stub is a few hundred bytes at most.
        assert!(
            post_l2.len() < 2048,
            "post-L2 stub should be small, got {} bytes",
            post_l2.len()
        );

        // Layer 3: run on the already-rewritten entry.  Total is the stub size, which
        // is well under the 10 KB budget, so the entry must remain unchanged.
        let mut entries = vec![ToolResultEntry {
            tool_use_id: "tool-big".into(),
            content: post_l2.clone(),
        }];
        enforcer.enforce_turn_budget(&mut entries);
        assert_eq!(
            entries[0].content, post_l2,
            "Layer 3 must not re-spill an already-persisted stub"
        );
    }
}
