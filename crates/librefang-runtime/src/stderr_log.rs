//! Live-streaming helpers for child-process stderr.
//!
//! `plugin_runtime` (hooks) and `python_runtime` (Python tool calls) both
//! tail child stderr line-by-line to give operators a "still working"
//! signal during long runs (issue #3256). Each call site exposes a
//! `pub const _STDERR_TARGET` so log filters / journalctl pipelines can
//! key off a stable string; this module owns the trim-and-skip predicate
//! they share.
//!
//! The full line (including the trailing newline) is *always* appended
//! to the post-exit summary buffer regardless of what this helper
//! returns — `info!` streaming and the `debug!` summary are independent
//! channels by design.

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
}
