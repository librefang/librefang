//! Canonical silent-response detection.
//!
//! The agent runtime supports several "silent reply" sentinels that the LLM
//! emits to indicate the current turn does not warrant a user-visible
//! response. Historically this detection was reimplemented at 4+ call sites
//! (agent_loop, session_repair, claude_code driver, gateway), each subtly
//! different — a class of bugs (`OB-02`, `OB-03`, `OB-07`) traced back to the
//! divergence between those copies.
//!
//! This module is now the **single source of truth** for silent-response
//! classification. Every call-site MUST delegate here.
//!
//! Recognised sentinels (case-insensitive, surrounded by optional whitespace
//! and trailing `[\s.!?]+`):
//!
//! - `NO_REPLY`
//! - `[no reply needed]` (optional outer brackets)
//! - `no reply needed`
//!
//! Whole-message semantics: the input must consist *entirely* of one of the
//! sentinels (after trim). A sentence containing a sentinel as a substring is
//! NOT silent. The runtime is conservative: when in doubt, deliver the reply.
//!
//! Historical compatibility: the legacy `is_no_reply` helper accepted a
//! sentinel anywhere at the **end** of the text (e.g. `"all good. NO_REPLY"`).
//! That is preserved here behind the trailing-suffix branch — many existing
//! prompts accumulate trailing tokens and we cannot regress them silently.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Env-flag rollback hatch: setting `LIBREFANG_SILENT_V2=off` reverts to
/// the legacy (pre-Phase-2) detector semantics — exact match or trailing
/// suffix only, no emoji/punctuation tolerance, no bracket-form
/// case-folding. Captured once at first call to avoid per-call env reads.
fn v2_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        !matches!(
            std::env::var("LIBREFANG_SILENT_V2")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "off" | "0" | "false" | "no"
        )
    })
}

/// Legacy detector — bit-for-bit equivalent to the pre-Phase-2 `is_no_reply`
/// helper that lived in `agent_loop.rs`. Used when `LIBREFANG_SILENT_V2=off`.
fn legacy_is_silent(text: &str) -> bool {
    let t = text.trim();
    t == "NO_REPLY"
        || t.ends_with("NO_REPLY")
        || t == "[no reply needed]"
        || t.ends_with("[no reply needed]")
        || t == "no reply needed"
        || t.ends_with("no reply needed")
}

/// Reason classification for a silent decision. Used in structured logs
/// (`event = "silent_response_detected"`) so observability tooling can
/// distinguish a sentinel-driven silence from a directive-driven one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SilentReason {
    /// LLM emitted a sentinel token (`NO_REPLY`, `[no reply needed]`, …).
    NoReply,
    /// Group-gating decided the turn was addressed to another participant.
    NotAddressed,
    /// Policy filter blocked the response (PII, safety, …).
    PolicyBlock,
}

/// Canonical detector. Returns true when `text` should be treated as a
/// silent (zero-length) reply — e.g. its content is one of the recognised
/// sentinels, possibly with whitespace, trailing punctuation, or a trailing
/// emoji.
///
/// See module-level docs for the exact accepted forms and the "whole
/// message" / "trailing suffix" semantics.
pub fn is_silent_response(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    if !v2_enabled() {
        return legacy_is_silent(text);
    }

    // Strip trailing punctuation/whitespace and trailing emoji codepoints
    // (anything that is not alphanumeric, underscore, bracket, or space).
    let stripped = strip_trailing_noise(trimmed);

    if matches_canonical(stripped) {
        return true;
    }

    // Trailing-suffix tolerance: legacy prompts sometimes put context BEFORE
    // the sentinel ("all good. NO_REPLY"). The sentinel must follow a
    // non-word boundary (whitespace, punctuation, newline, or emoji), and
    // it must be the LAST token (after the same trailing-noise strip).
    ends_with_canonical(stripped)
}

/// Strip trailing characters that don't belong to a sentinel token: ASCII
/// whitespace, common punctuation, and any non-ASCII char (catches emojis
/// without dragging in `unicode-segmentation`).
fn strip_trailing_noise(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        // Walk backwards by whole UTF-8 chars.
        let ch_start = (0..end)
            .rev()
            .find(|&i| (bytes[i] & 0xC0) != 0x80)
            .unwrap_or(0);
        let ch = &s[ch_start..end];
        let c = ch.chars().next().unwrap();
        let strip = c.is_ascii_whitespace()
            || matches!(c, '.' | ',' | ';' | ':' | '!' | '?')
            || !c.is_ascii(); // emojis, NBSP, etc.
        if strip {
            end = ch_start;
        } else {
            break;
        }
    }
    &s[..end]
}

/// Whole-token match against the canonical sentinel set.
fn matches_canonical(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "no_reply" | "[no reply needed]" | "no reply needed"
    )
}

/// True iff `s` ends with a canonical sentinel preceded by a non-word char
/// (or start-of-string). Used to catch "context. NO_REPLY" style trailing
/// leaks the legacy detector accepted.
fn ends_with_canonical(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    for needle in ["no_reply", "[no reply needed]", "no reply needed"] {
        if lower.ends_with(needle) {
            // Boundary check: char immediately before the needle must not
            // be alphanumeric/underscore (avoid `NO_REPLYING`,
            // `noreply@example.com`).
            let cut = lower.len() - needle.len();
            if cut == 0 {
                return true;
            }
            let prev = lower[..cut].chars().next_back().unwrap();
            let is_word = prev.is_ascii_alphanumeric() || prev == '_';
            if !is_word {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Canonical positive cases (whole-message sentinels) ---
    #[test]
    fn exact_no_reply() {
        assert!(is_silent_response("NO_REPLY"));
    }

    #[test]
    fn lowercase_no_reply() {
        assert!(is_silent_response("no_reply"));
    }

    #[test]
    fn mixed_case_no_reply() {
        assert!(is_silent_response("No_Reply"));
    }

    #[test]
    fn trailing_punctuation() {
        assert!(is_silent_response("NO_REPLY."));
        assert!(is_silent_response("NO_REPLY!"));
        assert!(is_silent_response("NO_REPLY?"));
    }

    #[test]
    fn surrounding_whitespace() {
        assert!(is_silent_response("  NO_REPLY  "));
        assert!(is_silent_response("NO_REPLY "));
        assert!(is_silent_response("NO_REPLY\n"));
        assert!(is_silent_response("NO_REPLY\n\n"));
    }

    #[test]
    fn bracketed_form() {
        assert!(is_silent_response("[no reply needed]"));
        assert!(is_silent_response("[NO REPLY NEEDED]"));
        assert!(is_silent_response("[no reply needed]."));
    }

    #[test]
    fn unbracketed_form() {
        assert!(is_silent_response("no reply needed"));
        assert!(is_silent_response("NO REPLY NEEDED"));
    }

    #[test]
    fn glued_to_emoji() {
        // The emoji is stripped as trailing non-ASCII, leaving the sentinel.
        assert!(is_silent_response("NO_REPLY 😐"));
        assert!(is_silent_response("NO_REPLY🎩"));
    }

    // --- Trailing-suffix legacy compatibility ---
    #[test]
    fn trailing_after_context() {
        assert!(is_silent_response("Let me think.\nNO_REPLY"));
        assert!(is_silent_response("I'll stay quiet. NO_REPLY"));
        assert!(is_silent_response("Some context. [no reply needed]"));
        assert!(is_silent_response("...a Sua disposizione. 🎩NO_REPLY"));
    }

    // --- Negatives ---
    #[test]
    fn empty_is_not_sentinel() {
        // Empty string is silent by being blanked, not by sentinel detection.
        assert!(!is_silent_response(""));
        assert!(!is_silent_response("   "));
        assert!(!is_silent_response("\n\n"));
    }

    #[test]
    fn normal_text_is_not_silent() {
        assert!(!is_silent_response("Ok"));
        assert!(!is_silent_response("Confermato, rispondo dopo"));
        assert!(!is_silent_response("Reply no needed here explicitly"));
    }

    #[test]
    fn word_boundary() {
        assert!(!is_silent_response("NO_REPLYING"));
        assert!(!is_silent_response("noreply@example.com"));
    }

    #[test]
    fn embedded_substring_not_silent() {
        // Sentinel appears in the middle, not at the end → NOT silent.
        assert!(!is_silent_response("the NO_REPLY sentinel is documented"));
    }

    #[test]
    fn ambiguous_prefix_does_not_short_circuit() {
        // Real reply that happens to mention NO_REPLY mid-sentence.
        assert!(!is_silent_response(
            "Ok NO_REPLY received but here is your real answer"
        ));
    }

    // --- SilentReason serialization ---
    #[test]
    fn silent_reason_serializes_snake_case() {
        let no_reply = serde_json::to_string(&SilentReason::NoReply).unwrap();
        assert_eq!(no_reply, "\"no_reply\"");
        let not_addressed = serde_json::to_string(&SilentReason::NotAddressed).unwrap();
        assert_eq!(not_addressed, "\"not_addressed\"");
        let policy_block = serde_json::to_string(&SilentReason::PolicyBlock).unwrap();
        assert_eq!(policy_block, "\"policy_block\"");
    }

    #[test]
    fn silent_reason_roundtrip() {
        for r in [
            SilentReason::NoReply,
            SilentReason::NotAddressed,
            SilentReason::PolicyBlock,
        ] {
            let s = serde_json::to_string(&r).unwrap();
            let back: SilentReason = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
    }
}
