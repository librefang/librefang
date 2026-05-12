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

// ---------------------------------------------------------------------------
// Cascade-leak detection — canonical home for `is_cascade_leak` and the
// constants it references. Historically these lived in `agent_loop.rs`; they
// are here so the streaming early-abort path and its tests can share the same
// implementation without a circular module dependency.
// ---------------------------------------------------------------------------

/// Channel-envelope prefixes the gateway prepends to inbound text.
/// Shared between `is_cascade_leak` (drop on output) and
/// `sanitize_for_memory` in `agent_loop.rs` (strip on persist).
pub(crate) const ENVELOPE_LINE_PREFIXES: &[&str] = &[
    "[Group message from ",
    "[In risposta a:",
    "[Replying to:",
    "[Stranger from ",
];

/// Standalone envelope markers that occupy their own line.
pub(crate) const ENVELOPE_STANDALONE_MARKERS: &[&str] = &["[Stranger]", "[Forwarded]", "[User]"];

/// Prompt section headers that, when paired with a structural marker,
/// indicate cascade scaffolding regurgitation.
const THEMATIC_HEADERS: &[&str] = &[
    "## Sender",
    "## Today",
    "## Calendar",
    "## Tasks",
    "## Response Style",
];

/// Structural turn-frame markers that almost never appear in legitimate
/// agent replies.
const STRUCTURAL_TURN_FRAMES: &[&str] = &["User asked:", "I responded:", "[Past exchange]"];

/// Detect a cascade scaffolding leak: an agent response that contains
/// scaffolding markers in a configuration real replies almost never
/// produce.
///
/// Trip condition: **2+ structural** OR **1 structural + 1 thematic**.
/// `2+ thematic alone` is intentionally NOT a leak.
///
/// Used both by the assembled-response guard (non-streaming and streaming
/// EndTurn) and by the incremental streaming abort path.
pub fn is_cascade_leak(text: &str) -> bool {
    let mut structural_hits = 0u8;

    for m in STRUCTURAL_TURN_FRAMES
        .iter()
        .chain(ENVELOPE_LINE_PREFIXES.iter())
        .chain(ENVELOPE_STANDALONE_MARKERS.iter())
    {
        if text.contains(m) {
            structural_hits += 1;
            if structural_hits >= 2 {
                return true;
            }
        }
    }

    if structural_hits == 0 {
        return false;
    }
    THEMATIC_HEADERS.iter().any(|m| text.contains(m))
}

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
    /// The streaming response was aborted early because incremental
    /// cascade-leak detection fired (system-prompt regurgitation).
    PromptRegurgitated,
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
    // "no reply needed" without brackets is omitted here: it false-positives
    // on English prose ("I filed the bug; no reply needed"). Only the
    // bracketed form and the underscore token are unambiguous as suffixes.
    for needle in ["no_reply", "[no reply needed]"] {
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
        let prompt_regurgitated = serde_json::to_string(&SilentReason::PromptRegurgitated).unwrap();
        assert_eq!(prompt_regurgitated, "\"prompt_regurgitated\"");
    }

    #[test]
    fn silent_reason_roundtrip() {
        for r in [
            SilentReason::NoReply,
            SilentReason::NotAddressed,
            SilentReason::PolicyBlock,
            SilentReason::PromptRegurgitated,
        ] {
            let s = serde_json::to_string(&r).unwrap();
            let back: SilentReason = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
    }

    // --- Cascade-leak detection ---

    #[test]
    fn cascade_leak_two_structural_markers() {
        // "User asked:" + "I responded:" → 2 structural → leak
        assert!(is_cascade_leak("User asked: foo\nI responded: bar"));
    }

    #[test]
    fn cascade_leak_structural_plus_thematic() {
        // 1 structural + 1 thematic → leak
        assert!(is_cascade_leak("User asked: hello\n## Sender\nAlice"));
        assert!(is_cascade_leak("[Past exchange]\n## Today\n2024-01-01"));
    }

    #[test]
    fn cascade_leak_thematic_headers_alone_are_legitimate() {
        // 2+ thematic headers alone must NOT trigger the guard —
        // legitimate markdown help replies use these freely.
        assert!(!is_cascade_leak(
            "## Sender\nAlice\n\n## Today\n2024-01-01\n\n## Response Style\nFormal"
        ));
        assert!(!is_cascade_leak(
            "## Tasks\n- buy milk\n\n## Calendar\nTuesday"
        ));
    }

    #[test]
    fn cascade_leak_normal_reply_no_false_positive() {
        // Completely legitimate agent reply — no markers at all.
        assert!(!is_cascade_leak("Sure, I can help you with that!"));
        assert!(!is_cascade_leak(
            "Here is a summary:\n\n1. Point one\n2. Point two"
        ));
    }

    #[test]
    fn cascade_leak_envelope_prefix_counts_as_structural() {
        // Envelope prefix ([Group message from …]) is structural; pairing
        // with a thematic header trips the guard.
        assert!(is_cascade_leak(
            "[Group message from Alice]\n## Sender\nAlice"
        ));
        // Two envelope lines → 2 structural.
        assert!(is_cascade_leak(
            "[Group message from Alice]\n[Replying to: Bob]"
        ));
    }

    // --- Incremental cascade-leak check (simulates streaming delta accumulation) ---

    /// Simulate feeding text delta-by-delta and check at which point the
    /// incremental `is_cascade_leak` check fires.
    fn feed_deltas(deltas: &[&str]) -> (bool, usize) {
        let mut accumulated = String::new();
        for (i, delta) in deltas.iter().enumerate() {
            accumulated.push_str(delta);
            if is_cascade_leak(&accumulated) {
                return (true, i);
            }
        }
        (false, deltas.len())
    }

    #[test]
    fn incremental_fires_on_second_structural_header() {
        // Assemble "## Sender\nname\n\n## Today\nfoo\n\n## Style" in pieces.
        // The thematic headers alone don't fire; "User asked:" is the
        // structural marker needed to pair with the first thematic header.
        let deltas = [
            "User asked: ",
            "what time is it?\n",
            "## Today\n",
            "2024-01-01\n",
        ];
        let (fired, idx) = feed_deltas(&deltas);
        assert!(fired, "cascade leak should have fired");
        // Must fire no later than after the 3rd delta (which adds "## Today")
        assert!(idx <= 3, "should fire by delta index 3, fired at {idx}");
    }

    #[test]
    fn incremental_single_structural_no_thematic_does_not_fire() {
        // Only one structural marker, no thematic — should not fire.
        let deltas = ["User asked: something\n", "and some prose follows.\n"];
        let (fired, _) = feed_deltas(&deltas);
        assert!(!fired, "single structural marker should not trigger leak");
    }

    #[test]
    fn incremental_legitimate_reply_no_false_positive() {
        let deltas = [
            "Sure! Here is what I found:\n",
            "\n",
            "## Summary\n",
            "The answer is 42.\n",
        ];
        let (fired, _) = feed_deltas(&deltas);
        assert!(
            !fired,
            "legitimate reply must not trigger cascade-leak guard"
        );
    }

    #[test]
    fn incremental_two_structural_markers_fires() {
        // Feed structural markers one at a time.
        let deltas = [
            "Here is a recap.\n",
            "User asked: foo\n",
            "I responded: bar\n",
        ];
        let (fired, _) = feed_deltas(&deltas);
        assert!(fired, "two structural markers should trigger the guard");
    }
}
