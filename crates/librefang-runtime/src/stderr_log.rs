//! Live-streaming helpers for child-process stderr.
//!
//! `plugin_runtime` (hooks) and `python_runtime` (Python tool calls) both
//! tail child stderr line-by-line to give operators a "still working"
//! signal during long runs (issue #3256). Each call site exposes a
//! `pub const _STDERR_TARGET` so log filters / journalctl pipelines can
//! key off a stable string; this module owns the trim-and-skip predicate
//! they share, plus the bounded-buffer wrapper used for the post-exit
//! summary.
//!
//! The full line (including the trailing newline) is *always* offered to
//! [`CappedSummary`] regardless of what `trim_for_log` returns — `info!`
//! streaming and the `debug!` summary are independent channels by
//! design. Streaming continues even after the summary buffer caps, so a
//! runaway hook can't deadlock the daemon by filling our memory.

/// Soft cap on the post-exit summary buffer. A misbehaving hook that
/// dumps gigabytes of stderr would otherwise OOM the daemon — but we
/// can't simply stop reading the pipe (that deadlocks the child), so
/// the streaming `info!` path stays uncapped while [`CappedSummary`]
/// drops further data after this many bytes.
///
/// 1 MiB is large enough to hold a typical multi-frame Python traceback
/// or a few dozen progress lines, and small enough that a stuck hook
/// can't snowball memory usage on a daemon hosting many concurrent
/// hooks.
pub(crate) const STDERR_SUMMARY_CAP_BYTES: usize = 1024 * 1024;

/// Trim trailing whitespace from a raw `read_line` chunk. Returns `None`
/// for empty / whitespace-only inputs so callers can skip the
/// `tracing::info!` emission without affecting the post-exit summary
/// buffer.
pub(crate) fn trim_for_log(line: &str) -> Option<&str> {
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Bounded String collector for the post-exit `debug!` summary.
///
/// Once the buffer reaches [`STDERR_SUMMARY_CAP_BYTES`] further `push`
/// calls are silently dropped, with a single truncation marker recorded
/// at the cut point. Streaming via `tracing::info!` is performed at the
/// call site *before* `push` and is unaffected — operators who need the
/// full output beyond 1 MiB can capture it from the live `*_stderr`
/// tracing target.
pub(crate) struct CappedSummary {
    buf: String,
    truncated: bool,
}

impl CappedSummary {
    pub(crate) fn new() -> Self {
        Self {
            buf: String::new(),
            truncated: false,
        }
    }

    /// Append a raw `read_line` chunk to the summary. After the cap is
    /// reached, all further calls silently drop their input.
    pub(crate) fn push(&mut self, line: &str) {
        if self.truncated {
            return;
        }
        if self.buf.len() + line.len() > STDERR_SUMMARY_CAP_BYTES {
            // Drop the line that would overflow rather than splitting
            // mid-UTF-8 / mid-record; record a single marker so the
            // summary reader can tell something was elided.
            self.buf.push_str(
                "\n[stderr summary truncated at 1 MiB \
                 — see live `plugin_stderr` / `python_stderr` tracing logs for full output]\n",
            );
            self.truncated = true;
            return;
        }
        self.buf.push_str(line);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.buf
    }

    pub(crate) fn into_string(self) -> String {
        self.buf
    }
}

impl Default for CappedSummary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_empty_and_whitespace_only_lines() {
        // `read_line` regularly hands us a bare `\n` between log
        // statements. Streaming those would just spam the operator.
        assert_eq!(trim_for_log(""), None);
        assert_eq!(trim_for_log("\n"), None);
        assert_eq!(trim_for_log("\r\n"), None);
        assert_eq!(trim_for_log("   \t\n"), None);
    }

    #[test]
    fn strips_trailing_newline_keeps_leading_whitespace() {
        // Indentation-as-structure (e.g. tracebacks) must survive — only
        // the trailing newline gets eaten by the streaming layer.
        assert_eq!(trim_for_log("hello\n"), Some("hello"));
        assert_eq!(trim_for_log("step 3/5\r\n"), Some("step 3/5"));
        assert_eq!(
            trim_for_log("    File \"x.py\", line 1\n"),
            Some("    File \"x.py\", line 1")
        );
    }

    #[test]
    fn capped_summary_collects_lines_under_cap() {
        let mut s = CappedSummary::new();
        s.push("first\n");
        s.push("second\n");
        assert_eq!(s.as_str(), "first\nsecond\n");
        assert!(!s.is_empty());
    }

    #[test]
    fn capped_summary_drops_overflow_with_marker() {
        // Confirm the marker appears exactly once, that further pushes
        // are no-ops, and that the buffer doesn't grow past
        // `cap + marker_len`.
        let big = "X".repeat(STDERR_SUMMARY_CAP_BYTES);
        let mut s = CappedSummary::new();
        s.push(&big);
        let pre_cap_len = s.as_str().len();
        s.push("overflow line\n");
        assert!(s.as_str().len() > pre_cap_len, "marker should be appended");
        assert!(s.as_str().contains("truncated at 1 MiB"));
        let after_first_overflow = s.as_str().len();
        s.push("second overflow\n");
        s.push("third overflow\n");
        assert_eq!(
            s.as_str().len(),
            after_first_overflow,
            "subsequent pushes should be silently dropped, no double-marker",
        );
    }

    #[test]
    fn capped_summary_into_string_round_trip() {
        let mut s = CappedSummary::new();
        s.push("alpha\n");
        s.push("beta\n");
        let out = s.into_string();
        assert_eq!(out, "alpha\nbeta\n");
    }
}
