//! Three-layer tool result budget enforcement.
//!
//! Defense against context-window overflow from large tool outputs:
//!
//! 1. **Layer 1 (per-tool)**: Each tool pre-truncates its own output before
//!    returning. This is handled inside individual tool implementations and is
//!    not the responsibility of this module.
//!
//! 2. **Layer 2 (per-result)**: After a tool returns, if its output exceeds
//!    [`PER_RESULT_THRESHOLD`] (default 50 KB), the full content is written to
//!    a temp file and the in-context content is replaced with a compact summary
//!    block containing a file path and a short preview. Fallback: if the write
//!    fails, the content is truncated inline and a notice is appended.
//!
//! 3. **Layer 3 (per-turn aggregate)**: After all tool results in a single
//!    assistant turn have been collected, if their combined size exceeds
//!    [`PER_TURN_BUDGET`] (default 200 KB), the largest non-persisted results
//!    are spilled to disk in descending-size order until the aggregate is under
//!    budget.

use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Default per-result persistence threshold (50 KB).
pub const PER_RESULT_THRESHOLD: usize = 50 * 1024;

/// Default per-turn aggregate budget (200 KB).
pub const PER_TURN_BUDGET: usize = 200 * 1024;

/// Number of characters shown in the preview block.
const PREVIEW_CHARS: usize = 500;

/// Marker string used to detect already-persisted results (Layer 3 skip guard).
const PERSISTED_MARKER: &str = "[Tool output too large";

/// A single tool result entry used by the per-turn budget enforcer.
#[derive(Debug)]
pub struct ToolResultEntry {
    /// The `tool_use_id` for this result (used as the spill filename stem).
    pub tool_use_id: String,
    /// Content of the result. May be replaced in-place by the enforcer.
    pub content: String,
}

/// Enforces per-result and per-turn-aggregate size budgets on tool outputs.
///
/// Constructed once per agent loop instantiation and reused across turns.
/// All file I/O uses only `std::fs` — no async, no external dependencies.
pub struct ToolBudgetEnforcer {
    /// Layer 2 threshold: results larger than this are persisted to disk.
    pub per_result_threshold: usize,
    /// Layer 3 threshold: if total bytes across all results in a turn
    /// exceeds this, the largest non-persisted results are spilled.
    pub per_turn_budget: usize,
    /// Directory used for spill files. Created lazily on first use.
    temp_dir: PathBuf,
}

impl Default for ToolBudgetEnforcer {
    fn default() -> Self {
        Self::new(PER_RESULT_THRESHOLD, PER_TURN_BUDGET)
    }
}

impl ToolBudgetEnforcer {
    /// Create an enforcer with custom thresholds.
    ///
    /// `temp_dir` defaults to `std::env::temp_dir()/librefang-results`.
    pub fn new(per_result_threshold: usize, per_turn_budget: usize) -> Self {
        let temp_dir = std::env::temp_dir().join("librefang-results");
        Self {
            per_result_threshold,
            per_turn_budget,
            temp_dir,
        }
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Layer 2: per-result
    // ──────────────────────────────────────────────────────────────────────────

    /// Apply Layer 2 budget to a single tool result.
    ///
    /// If `content` is within the threshold, it is returned unchanged.
    /// Otherwise the full content is written to a temp file and a compact
    /// summary block (file path + 500-char preview) is returned instead.
    ///
    /// **Fallback**: if the file write fails for any reason, the content is
    /// truncated to `per_result_threshold` bytes and a notice is appended.
    /// This function never panics.
    pub fn maybe_persist_result(&self, content: &str, tool_use_id: &str) -> String {
        if content.len() <= self.per_result_threshold {
            return content.to_string();
        }

        let original_len = content.len();
        let file_path = self.temp_dir.join(format!("{tool_use_id}.txt"));

        match self.write_spill_file(&file_path, content) {
            Ok(()) => {
                debug!(
                    tool_use_id,
                    bytes = original_len,
                    path = %file_path.display(),
                    "tool_budget: persisted oversized result (Layer 2)"
                );
                build_persisted_summary(content, original_len, &file_path)
            }
            Err(e) => {
                warn!(
                    tool_use_id,
                    bytes = original_len,
                    error = %e,
                    "tool_budget: failed to persist result, falling back to inline truncation"
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
        candidates.sort_by(|a, b| b.1.cmp(&a.1));

        let mut running_total = total;

        for (idx, size) in candidates {
            if running_total <= self.per_turn_budget {
                break;
            }

            let entry = &mut results[idx];
            let file_path = self
                .temp_dir
                .join(format!("{}-budget.txt", entry.tool_use_id));

            let replacement = match self.write_spill_file(&file_path, &entry.content) {
                Ok(()) => {
                    debug!(
                        tool_use_id = %entry.tool_use_id,
                        bytes = size,
                        path = %file_path.display(),
                        "tool_budget: spilled result for turn budget (Layer 3)"
                    );
                    build_persisted_summary(&entry.content, size, &file_path)
                }
                Err(e) => {
                    warn!(
                        tool_use_id = %entry.tool_use_id,
                        bytes = size,
                        error = %e,
                        "tool_budget: turn-budget spill failed, truncating inline"
                    );
                    inline_truncate(&entry.content, self.per_result_threshold)
                }
            };

            running_total = running_total - size + replacement.len();
            entry.content = replacement;
        }
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Internal helpers
    // ──────────────────────────────────────────────────────────────────────────

    /// Create the spill directory if needed, then write `content` to `path`.
    fn write_spill_file(&self, path: &Path, content: &str) -> std::io::Result<()> {
        fs::create_dir_all(&self.temp_dir)?;
        fs::write(path, content.as_bytes())?;
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Free helpers (pure, no I/O)
// ──────────────────────────────────────────────────────────────────────────────

/// Build the compact summary block shown in-context when a result is persisted.
fn build_persisted_summary(content: &str, original_bytes: usize, path: &Path) -> String {
    let preview: String = content.chars().take(PREVIEW_CHARS).collect();
    let has_more = content.chars().count() > PREVIEW_CHARS;
    let mut out = format!(
        "[Tool output too large ({original_bytes} bytes). Saved to: {}]\n\
         Preview (first {PREVIEW_CHARS} chars):\n\
         {preview}",
        path.display()
    );
    if has_more {
        out.push_str("\n...");
    }
    out
}

/// Truncate `content` to at most `max_bytes` UTF-8 bytes (snapping to a char
/// boundary) and append a notice. Used as the fallback when file I/O fails.
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
            temp_dir: tmpdir.to_path_buf(),
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
        assert!(result.starts_with(PERSISTED_MARKER));
        assert!(result.contains("id-2.txt"));
        // File should exist and contain the original content.
        let written = fs::read_to_string(dir.path().join("id-2.txt")).unwrap();
        assert_eq!(written, content);
    }

    #[test]
    fn layer2_fallback_on_bad_path() {
        // Use an unwriteable path to force the fallback.
        let enforcer = ToolBudgetEnforcer {
            per_result_threshold: 10,
            per_turn_budget: 1000,
            temp_dir: PathBuf::from("/proc/no-such-dir-librefang-test"),
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
    }

    #[test]
    fn layer3_skips_already_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = make_enforcer(dir.path());
        let persisted_content = format!(
            "{} (99999 bytes). Saved to: /tmp/old.txt]\nPreview (first 500 chars):\nabc",
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
}
