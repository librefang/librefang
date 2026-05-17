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
///
/// `## Today` / `## Calendar` / `## Tasks` are *ambiguous* — a legitimate
/// "what does my day look like" help reply produces these freely, so 2+ of
/// them alone is intentionally NOT a leak (houko-flagged false positive,
/// guarded by `thematic_headers_alone_are_legitimate`).
const THEMATIC_HEADERS: &[&str] = &[
    "## Sender",
    "## Today",
    "## Calendar",
    "## Tasks",
    "## Response Style",
];

/// Subset of [`THEMATIC_HEADERS`] that describe the *prompt frame itself*,
/// not reply content. An agent never legitimately emits `## Sender`
/// (it does not narrate who messaged it back to the user) or
/// `## Response Style` (a meta-instruction block, never reply prose) in a
/// genuine reply — these only appear when the model regurgitates the
/// scaffolding verbatim. They are therefore as diagnostic as a structural
/// turn-frame marker. This closes the all-thematic bypass
/// (`## Sender\n…\n## Today\n…\n## Tasks` — three thematic, zero structural)
/// flagged in issue #5141 without regressing the legitimate day-summary
/// reply (which only ever uses the ambiguous `## Today`/`## Calendar`/
/// `## Tasks` subset).
const SCAFFOLD_ONLY_HEADERS: &[&str] = &["## Sender", "## Response Style"];

/// Structural turn-frame markers that almost never appear in legitimate
/// agent replies.
const STRUCTURAL_TURN_FRAMES: &[&str] = &["User asked:", "I responded:", "[Past exchange]"];

/// Detect a cascade scaffolding leak: an agent response that contains
/// scaffolding markers in a configuration real replies almost never
/// produce.
///
/// Trip condition: **2+ structural** OR **1 structural + 1 thematic**.
/// `2+ *ambiguous* thematic alone` is intentionally NOT a leak (legitimate
/// day-summary replies). The scaffold-only headers (`## Sender`,
/// `## Response Style` — see [`SCAFFOLD_ONLY_HEADERS`]) count toward the
/// structural tally because no genuine reply emits them, so the
/// all-thematic regurgitation `## Sender\n…\n## Today\n…\n## Tasks`
/// (issue #5141) now trips: `## Sender` is structural-equivalent, and it
/// pairs with the `## Today`/`## Tasks` thematic hits.
///
/// Used both by the assembled-response guard (non-streaming and streaming
/// EndTurn) and by the incremental streaming abort path.
pub fn is_cascade_leak(text: &str) -> bool {
    let mut structural_hits = 0u8;

    for m in STRUCTURAL_TURN_FRAMES
        .iter()
        .chain(ENVELOPE_LINE_PREFIXES.iter())
        .chain(ENVELOPE_STANDALONE_MARKERS.iter())
        .chain(SCAFFOLD_ONLY_HEADERS.iter())
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
    // Pair the structural hit with an *ambiguous* thematic header only.
    // Scaffold-only headers are already counted as structural above —
    // re-counting them here would let a single `## Response Style` (1
    // structural + itself as "thematic") trip on a legitimate one-section
    // reply. The ambiguous subset (## Today / ## Calendar / ## Tasks) is
    // exactly the day-summary content a real reply pairs with scaffolding
    // when the model regurgitates the frame.
    THEMATIC_HEADERS
        .iter()
        .filter(|h| !SCAFFOLD_ONLY_HEADERS.contains(h))
        .any(|m| text.contains(m))
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

/// Legacy detector — used when `LIBREFANG_SILENT_V2=off`. Historically this
/// was bit-for-bit `t == S || t.ends_with(S)` for each sentinel, but the
/// unbounded `ends_with` let adversarial sender input coerce the LLM into
/// appending `NO_REPLY` *directly after a side-effecting tool's output*
/// (e.g. `...deleted 200 rowsNO_REPLY`) to suppress the user-visible reply
/// while the tool already ran (issue #5141). The trailing-suffix form now
/// requires a boundary (the char before the sentinel must not be
/// alphanumeric / `_`) — mirroring the v2 [`ends_with_canonical`] boundary
/// rule — so `xNO_REPLY` no longer matches but `x. NO_REPLY` /
/// `x\nNO_REPLY` (legitimate trailing-token prompts) still do. Exact-match
/// behaviour is unchanged.
fn legacy_is_silent(text: &str) -> bool {
    let t = text.trim();
    t == "NO_REPLY"
        || legacy_ends_with_boundary(t, "NO_REPLY")
        || t == "[no reply needed]"
        || legacy_ends_with_boundary(t, "[no reply needed]")
        || t == "no reply needed"
        || legacy_ends_with_boundary(t, "no reply needed")
}

/// True iff `t` ends with `needle` AND the character immediately before
/// `needle` is a non-word boundary (whitespace, punctuation, bracket, …)
/// or `needle` is at the start. Prevents `...rowsNO_REPLY` /
/// `NO_REPLYING` from suppressing a reply while preserving the legacy
/// "context.\nNO_REPLY" trailing-token tolerance.
fn legacy_ends_with_boundary(t: &str, needle: &str) -> bool {
    let Some(cut) = t.len().checked_sub(needle.len()) else {
        return false;
    };
    if !t.ends_with(needle) {
        return false;
    }
    if cut == 0 {
        return true;
    }
    let prev = t[..cut].chars().next_back().unwrap();
    !(prev.is_alphanumeric() || prev == '_')
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

    // --- #5141: legacy detector trailing-NO_REPLY boundary ---
    // Tested against `legacy_is_silent` directly: `is_silent_response`
    // routes here only when `LIBREFANG_SILENT_V2=off`, and `v2_enabled()`
    // memoises the env read in a process-wide `OnceLock`, so going through
    // the public entrypoint would be order-dependent / flaky.
    #[test]
    fn legacy_rejects_glued_no_reply_after_tool_output() {
        // ATTACK: adversarial sender input coerces the LLM to append
        // NO_REPLY directly after a side-effecting tool's output so the
        // user never sees that the tool ran. No boundary before the
        // sentinel → must NOT be treated as silent.
        assert!(!legacy_is_silent("deleted 200 rowsNO_REPLY"));
        assert!(!legacy_is_silent(
            "Transferred $5000 to account 12345NO_REPLY"
        ));
        assert!(!legacy_is_silent("doneno reply needed"));
        // Alphanumeric / `_` immediately before sentinel: still not silent.
        assert!(!legacy_is_silent("xNO_REPLY"));
        assert!(!legacy_is_silent("foo_NO_REPLY"));
        assert!(!legacy_is_silent("NO_REPLYING"));
    }

    #[test]
    fn legacy_still_accepts_bounded_trailing_no_reply() {
        // POSITIVE: legitimate trailing-token prompts (the historical
        // behaviour we cannot regress) must still be silent.
        assert!(legacy_is_silent("NO_REPLY"));
        assert!(legacy_is_silent("all good. NO_REPLY"));
        assert!(legacy_is_silent("Let me think.\nNO_REPLY"));
        assert!(legacy_is_silent("[no reply needed]"));
        assert!(legacy_is_silent("Some context. [no reply needed]"));
        assert!(legacy_is_silent("no reply needed"));
        assert!(legacy_is_silent("done; no reply needed"));
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
    fn cascade_leak_ambiguous_thematic_headers_alone_are_legitimate() {
        // 2+ *ambiguous* thematic headers alone must NOT trigger the guard
        // — legitimate "what does my day look like" replies use these
        // freely. NOTE (#5141): the previous version of this test asserted
        // `"## Sender …\n## Response Style …"` was also legitimate. That
        // was the vulnerability: `## Sender` / `## Response Style` are
        // pure scaffolding (a genuine reply never narrates the sender or a
        // meta response-style block), so that exact shape is now correctly
        // a leak — see `cascade_leak_scaffold_only_headers_trip_*`.
        assert!(!is_cascade_leak(
            "## Today\n2024-01-01\n\n## Calendar\nstandup 9am\n\n## Tasks\nreview"
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

    // --- #5141: all-thematic scaffolding-regurgitation bypass ---

    #[test]
    fn cascade_leak_scaffold_only_headers_trip_without_structural() {
        // ATTACK: the LLM is coerced into regurgitating the system-prompt
        // scaffolding with zero structural markers and only thematic
        // headers — `## Sender` + `## Today` + `## Tasks`. Before the fix
        // this had 0 structural hits → not a leak → fully delivered to the
        // user. `## Sender` is a scaffold-only header (no genuine reply
        // emits it), so it now counts toward the structural tally and the
        // leak trips.
        assert!(is_cascade_leak(
            "## Sender\nAlice (+15551234567)\n\n## Today\n2024-01-01\n\n## Tasks\n- review PR"
        ));
        // `## Response Style` is the other scaffold-only header; two
        // scaffold-only headers alone (2 structural-equiv) also trip.
        assert!(is_cascade_leak(
            "## Sender\nBob\n\n## Response Style\nFormal, concise."
        ));
    }

    #[test]
    fn cascade_leak_ambiguous_thematic_alone_still_legitimate() {
        // REGRESSION GUARD: the houko-flagged false positive must stay
        // fixed — a legitimate "what does my day look like" reply uses the
        // *ambiguous* thematic headers (## Today/## Calendar/## Tasks) and
        // must NOT be classified as a leak. Crucially these contain NO
        // scaffold-only header.
        assert!(!is_cascade_leak(
            "## Today\nWednesday\n\n## Calendar\nno events\n\n## Tasks\npending"
        ));
        assert!(!is_cascade_leak(
            "## Calendar\n- meeting at 5pm\n\n## Tasks\n- send follow-up"
        ));
        // Single scaffold-only header alone is not enough (needs a 2nd
        // structural OR a thematic to pair with).
        assert!(!is_cascade_leak("## Response Style\nBe brief."));
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
