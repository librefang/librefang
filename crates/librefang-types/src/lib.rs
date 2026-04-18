//! Core types and traits for the LibreFang Agent Operating System.
//!
//! This crate defines all shared data structures used across the LibreFang kernel,
//! runtime, memory substrate, and wire protocol. It contains no business logic.

/// The LibreFang version, derived from the workspace `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod agent;
pub mod approval;
pub mod capability;
pub mod comms;
pub mod config;
pub mod error;
pub mod event;
pub mod goal;
pub mod i18n;
pub mod manifest_signing;
pub mod media;
pub mod memory;
pub mod message;
pub mod model_catalog;
pub mod registry_schema;
pub mod scheduler;
pub mod serde_compat;
pub mod subagent;
pub mod taint;
pub mod tool;
pub mod tool_compat;
pub mod tool_policy;
pub mod webhook;
pub mod workflow_template;

/// Check if a response is a NO\_REPLY sentinel. Matches:
/// - Exact `"NO_REPLY"` (original behaviour)
/// - Text ending with `NO_REPLY` (model sometimes adds context before it,
///   either on the same line or on a new line)
/// - Exact `"[no reply needed]"` — the runtime writes this placeholder back
///   into the session when the agent chooses silence, so the LLM sometimes
///   mimics it on later turns.
/// - Text ending with `"[no reply needed]"` (same reasoning as above)
/// - Exact `"no reply needed"` — unbracketed variant the model occasionally
///   emits. Only the exact-match form; `ends_with` is intentionally omitted
///   because it false-positives on English prose ("I filed the bug; no reply
///   needed.").
pub fn is_no_reply_sentinel(text: &str) -> bool {
    let t = text.trim();
    t == "NO_REPLY"
        || t.ends_with("NO_REPLY")
        || t == "[no reply needed]"
        || t.ends_with("[no reply needed]")
        || t == "no reply needed"
}

/// Safely truncate a string to at most `max_bytes`, never splitting a UTF-8 char.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_str_ascii() {
        assert_eq!(truncate_str("hello world", 5), "hello");
    }

    #[test]
    fn truncate_str_chinese() {
        // Each Chinese character is 3 bytes
        let s = "\u{4F60}\u{597D}\u{4E16}\u{754C}"; // 你好世界
        assert_eq!(truncate_str(s, 6), "\u{4F60}\u{597D}"); // 你好
        assert_eq!(truncate_str(s, 7), "\u{4F60}\u{597D}"); // still 你好 (7 is mid-char)
        assert_eq!(truncate_str(s, 9), "\u{4F60}\u{597D}\u{4E16}"); // 你好世
    }

    #[test]
    fn truncate_str_emoji() {
        let s = "hi\u{1F600}there"; // hi😀there — emoji is 4 bytes
        assert_eq!(truncate_str(s, 3), "hi"); // 3 is mid-emoji
        assert_eq!(truncate_str(s, 6), "hi\u{1F600}"); // after emoji
    }

    #[test]
    fn truncate_str_em_dash() {
        // Em dash (—) is 3 bytes (0xE2 0x80 0x94) — the exact char that caused
        // production panics in kernel.rs and session.rs (issue #104)
        let s = "Here is a summary — with details";
        assert_eq!(truncate_str(s, 19), "Here is a summary ");
        assert_eq!(truncate_str(s, 20), "Here is a summary ");
        assert_eq!(truncate_str(s, 21), "Here is a summary \u{2014}");
    }

    #[test]
    fn truncate_str_no_truncation() {
        assert_eq!(truncate_str("short", 100), "short");
    }

    #[test]
    fn truncate_str_empty() {
        assert_eq!(truncate_str("", 10), "");
    }
}
