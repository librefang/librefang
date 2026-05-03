//! Multi-layer tool result budget enforcement.
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
//! 3. **Layer 3 (per-turn aggregate spill)**: After all tool results in a
//!    single assistant turn have been collected, if their combined size exceeds
//!    [`PER_TURN_BUDGET`] (default 200 KB), the largest non-persisted results
//!    are spilled to disk in descending-size order until the aggregate is under
//!    budget.
//!
//! 4. **Layer 4 (per-turn hard cap, issue #3347 mechanism (1))**: An outer
//!    operator-tunable safety net configured via `[runtime.tool_results]
//!    max_bytes_per_turn` (default 50 000 bytes). After Layer 3 has run, if the
//!    cumulative size of tool-result content for the turn still exceeds the
//!    cap, the **oldest** results are tail-truncated inline (UTF-8-safe) using
//!    the same `... [truncated, N total bytes]` marker the per-result tools
//!    already use. Operates entirely in-memory — no disk I/O — so it always
//!    succeeds even on read-only filesystems. See
//!    [`enforce_per_turn_byte_cap`].

use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Default per-result persistence threshold (50 KB).
pub const PER_RESULT_THRESHOLD: usize = 50 * 1024;

/// Default per-turn aggregate budget (200 KB).
pub const PER_TURN_BUDGET: usize = 200 * 1024;

/// Default per-turn hard byte cap when no operator value is supplied
/// (issue #3347 mechanism (1)). Matches the
/// `[runtime.tool_results].max_bytes_per_turn` config default in
/// `librefang-types::config::ToolResultsConfig`.
pub const DEFAULT_PER_TURN_BYTE_CAP: usize = 50_000;

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
        candidates.sort_by_key(|b| std::cmp::Reverse(b.1));

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
// Layer 4: per-turn hard byte cap (issue #3347 mechanism (1))
// ──────────────────────────────────────────────────────────────────────────────

/// Marker the inline-truncation pass leaves on shrunken results so a downstream
/// pass can recognise them and avoid double-truncation. Matches the format
/// already produced by individual tool implementations
/// (`tool_runner.rs` web fetch, shell exec stdout/stderr, image preview).
#[cfg(test)]
const TRUNCATED_MARKER: &str = "[truncated,";

/// Tail-truncate `content` to at most `keep_bytes` bytes (UTF-8 safe) and
/// append the canonical `... [truncated, N total bytes]` suffix that the
/// existing per-result truncation sites already use, so observers can rely on
/// a single grep pattern.
///
/// Returns the original `content` unchanged when it already fits.
fn tail_truncate_with_marker(content: &str, keep_bytes: usize) -> String {
    if content.len() <= keep_bytes {
        return content.to_string();
    }
    let original_len = content.len();
    let truncated = crate::str_utils::safe_truncate_str(content, keep_bytes);
    format!("{truncated}... [truncated, {original_len} total bytes]")
}

/// Apply the per-turn cumulative byte cap (Layer 4) to a slice of tool results.
///
/// Mechanism (1) of issue #3347: after each assistant turn the runtime sums
/// the byte length of every `ToolResult.content` block produced this turn; if
/// the sum exceeds `max_bytes_per_turn`, the **oldest** results in the slice
/// are shrunk first via [`tail_truncate_with_marker`] until the running total
/// fits under the cap.
///
/// Why oldest-first (vs Layer 3's largest-first): Layer 3 evicts to disk, so
/// largest-first is rational — biggest payoff per write. Layer 4 is the
/// in-context cap; oldest-first preserves the **most recent** tool outputs,
/// which is what the model needs to keep reasoning. The two passes compose:
/// Layer 3 spills oversized blobs first, Layer 4 then enforces the hard
/// in-context cap on whatever remains.
///
/// Setting `max_bytes_per_turn = 0` is treated as "disabled" — the cap is
/// skipped entirely (existing per-result truncation still applies).
///
/// Returns the number of bytes shrunk in total (sum over all modified
/// results). `0` means no work was done.
pub fn enforce_per_turn_byte_cap(
    results: &mut [ToolResultEntry],
    max_bytes_per_turn: usize,
) -> usize {
    if max_bytes_per_turn == 0 {
        return 0;
    }
    let total: usize = results.iter().map(|r| r.content.len()).sum();
    if total <= max_bytes_per_turn {
        return 0;
    }

    debug!(
        total_bytes = total,
        cap = max_bytes_per_turn,
        results = results.len(),
        "tool_budget: per-turn byte cap exceeded, shrinking oldest results (Layer 4)"
    );

    let mut running_total = total;
    let mut bytes_shrunk: usize = 0;

    // Walk oldest → newest. Each oversized entry is tail-truncated to the
    // largest size that, combined with the entries we have NOT yet visited
    // (still at full size) and the entries we have ALREADY visited (now
    // potentially shrunk), keeps the running total under the cap.
    for entry in results.iter_mut() {
        if running_total <= max_bytes_per_turn {
            break;
        }
        let entry_len = entry.content.len();
        // Bytes contributed by every other entry in its current state.
        let other_bytes = running_total - entry_len;
        // The most we can keep for this entry while still fitting the cap.
        // Saturating sub: if `other_bytes >= max_bytes_per_turn` we must
        // shrink to zero (well, to the marker-only string).
        let allowed = max_bytes_per_turn.saturating_sub(other_bytes);
        if allowed >= entry_len {
            // Already small enough relative to this turn — nothing to do.
            continue;
        }
        // `tail_truncate_with_marker` adds a suffix; reserve some slack so
        // the *result* still fits under `allowed`. The marker grows with the
        // original length but is bounded — clamp `keep` so the final string
        // is at most `allowed` bytes. If `allowed` is too small to even hold
        // the marker, keep zero content bytes; the marker overhead is
        // accepted as the unavoidable floor.
        let marker_overhead = format!("... [truncated, {entry_len} total bytes]").len();
        let keep = allowed.saturating_sub(marker_overhead);
        let new_content = tail_truncate_with_marker(&entry.content, keep);
        let new_len = new_content.len();
        let shrunk_by = entry_len.saturating_sub(new_len);
        bytes_shrunk = bytes_shrunk.saturating_add(shrunk_by);
        running_total = running_total - entry_len + new_len;
        debug!(
            tool_use_id = %entry.tool_use_id,
            from_bytes = entry_len,
            to_bytes = new_len,
            "tool_budget: tail-truncated result for per-turn cap (Layer 4)"
        );
        entry.content = new_content;
    }

    if bytes_shrunk > 0 {
        tracing::info!(
            bytes_shrunk,
            final_total = running_total,
            cap = max_bytes_per_turn,
            "tool_budget: per-turn byte cap enforced (mechanism (1) of issue #3347)"
        );
    }

    bytes_shrunk
}

/// Visible for tests in this module; downstream code should never need this.
#[cfg(test)]
fn marker_str() -> &'static str {
    TRUNCATED_MARKER
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
    #[cfg(unix)]
    fn layer2_fallback_on_bad_path() {
        // Use an unwriteable path to force the fallback. `/proc` on Linux is a
        // read-only virtual filesystem; macOS has no `/proc` so `create_dir_all`
        // fails at the filesystem root. Either way, the write must fail so the
        // fallback path in `maybe_persist_result` runs.
        //
        // Skipped on Windows because `/proc/...` gets resolved to
        // `C:\proc\...`, which is writeable under a standard user account.
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

    // ────────────────────────────────────────────────────────────────────────
    // Layer 4 (issue #3347 mechanism (1)) — per-turn byte cap
    // ────────────────────────────────────────────────────────────────────────

    fn make_entry(id: &str, n: usize, ch: char) -> ToolResultEntry {
        ToolResultEntry {
            tool_use_id: id.into(),
            content: ch.to_string().repeat(n),
        }
    }

    #[test]
    fn layer4_short_session_is_unchanged() {
        // Regression: small turn must NOT trip the cap (no spurious shrink).
        let mut entries = vec![make_entry("a", 100, 'x'), make_entry("b", 200, 'y')];
        let shrunk = enforce_per_turn_byte_cap(&mut entries, 50_000);
        assert_eq!(
            shrunk, 0,
            "no shrink expected for 300-byte total under 50k cap"
        );
        assert_eq!(entries[0].content.len(), 100);
        assert_eq!(entries[1].content.len(), 200);
    }

    #[test]
    fn layer4_disabled_when_cap_zero() {
        // max_bytes_per_turn = 0 means "disabled".
        let mut entries = vec![make_entry("a", 1_000_000, 'x')];
        let shrunk = enforce_per_turn_byte_cap(&mut entries, 0);
        assert_eq!(shrunk, 0);
        assert_eq!(entries[0].content.len(), 1_000_000);
    }

    #[test]
    fn layer4_caps_ten_medium_results_under_max_bytes_per_turn() {
        // Ten 8 KB results = 80 KB. Cap = 50 KB. Final aggregate must fit
        // under (or at) the cap, and the OLDEST entries must be the ones
        // shrunk. This is the canonical AC test from issue #3347.
        let cap = 50_000;
        let mut entries: Vec<ToolResultEntry> = (0..10)
            .map(|i| make_entry(&format!("id-{i}"), 8_000, 'A'))
            .collect();
        let before_total: usize = entries.iter().map(|e| e.content.len()).sum();
        assert_eq!(before_total, 80_000);

        let shrunk = enforce_per_turn_byte_cap(&mut entries, cap);

        let after_total: usize = entries.iter().map(|e| e.content.len()).sum();
        assert!(after_total <= cap, "final total {after_total} > cap {cap}");
        assert!(shrunk > 0, "expected some shrink for 80k > 50k cap");

        // Oldest-first eviction: the FIRST entries must show the truncated
        // marker; the LAST entries must remain at their original size (the
        // model needs the most recent outputs).
        assert!(
            entries[0].content.contains(marker_str()),
            "oldest entry should be truncated, got: {:?}",
            &entries[0].content[..entries[0].content.len().min(80)]
        );
        assert_eq!(
            entries.last().unwrap().content.len(),
            8_000,
            "newest entry must NOT be touched"
        );
    }

    #[test]
    fn layer4_byte_delta_observable() {
        // The "before vs after byte delta" called out in the AC.
        let cap = 5_000;
        let mut entries: Vec<ToolResultEntry> = (0..5)
            .map(|i| make_entry(&format!("id-{i}"), 4_000, 'B'))
            .collect();
        let before_total: usize = entries.iter().map(|e| e.content.len()).sum();
        assert_eq!(before_total, 20_000);

        let shrunk = enforce_per_turn_byte_cap(&mut entries, cap);
        let after_total: usize = entries.iter().map(|e| e.content.len()).sum();

        let delta = before_total.saturating_sub(after_total);
        assert!(
            delta >= shrunk,
            "reported shrunk={shrunk} ≤ observed delta={delta}"
        );
        assert!(
            after_total <= cap,
            "after_total={after_total} must fit cap={cap}"
        );
    }

    #[test]
    fn layer4_marker_format_matches_existing_tools() {
        // The cap reuses the exact "... [truncated, N total bytes]" suffix
        // that web/shell tools already emit, so log scrapers see one format.
        let mut entries = vec![make_entry("only", 10_000, 'C')];
        let _ = enforce_per_turn_byte_cap(&mut entries, 1_000);
        let s = &entries[0].content;
        assert!(
            s.contains("[truncated, 10000 total bytes]"),
            "marker not found in: {s}"
        );
        assert!(
            s.starts_with("CCC"),
            "leading content preserved (tail-truncate)"
        );
    }

    #[test]
    fn layer4_handles_utf8_boundaries() {
        // 1000 × 3-byte chars = 3000 bytes; must not panic on multi-byte.
        let big: String = "日".repeat(1000);
        let mut entries = vec![ToolResultEntry {
            tool_use_id: "ja".into(),
            content: big,
        }];
        let cap = 500;
        let _ = enforce_per_turn_byte_cap(&mut entries, cap);
        // Result must (a) fit under cap, (b) be valid UTF-8 (just not panic).
        assert!(entries[0].content.len() <= cap.max(60));
        assert!(entries[0]
            .content
            .is_char_boundary(entries[0].content.len()));
    }

    #[test]
    fn layer4_preserves_already_small_newest_when_oldest_alone_overflows() {
        // One huge oldest entry + one small newest. Only the oldest should
        // be shrunk.
        let cap = 1_000;
        let mut entries = vec![
            make_entry("oldest", 10_000, 'O'),
            make_entry("newest", 200, 'N'),
        ];
        let _ = enforce_per_turn_byte_cap(&mut entries, cap);
        assert!(entries[0].content.contains(marker_str()));
        assert_eq!(entries[1].content.len(), 200, "newest must be untouched");
        let total: usize = entries.iter().map(|e| e.content.len()).sum();
        assert!(
            total <= cap.max(100),
            "total={total} must fit under cap+marker overhead"
        );
    }
}
