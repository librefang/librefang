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
pub(crate) const ENVELOPE_STANDALONE_MARKERS: &[&str] = &["[Forwarded]", "[Stranger]", "[User]"];

/// True when an `ENVELOPE_STANDALONE_MARKER` appears as a **line-leader**
/// preceded by `\n` AND followed by whitespace. This is the canonical
/// chat-template tokenizer-bleed signature: the model emits a silent sentinel
/// then the chat template continues with `[User]\n<fake_question>`.
///
/// Gated on `is_silent_response(text_before_marker)` so legitimate replies
/// that quote a marker on its own line are not silenced.
fn has_envelope_marker_as_line_leader(text: &str) -> bool {
    let bytes = text.as_bytes();
    for m in ENVELOPE_STANDALONE_MARKERS {
        let mut offset = 0;
        while let Some(rel_idx) = text[offset..].find(m) {
            let idx = offset + rel_idx;
            let starts_line = idx > 0 && bytes[idx - 1] == b'\n';
            let after = &text[idx + m.len()..];
            let leads_content = matches!(after.chars().next(), Some(c) if c.is_whitespace());
            if starts_line && leads_content {
                let before = &text[..idx];
                if is_silent_response(before) {
                    return true;
                }
            }
            offset = idx + m.len();
        }
    }
    false
}

/// Prompt section headers that, when paired with a structural marker,
/// indicate cascade scaffolding regurgitation.
///
/// **Drift-pin reconciliation** (post-#5053 / #5073 prompt-builder layout):
/// before this reconciliation the list contained five legacy strings
/// (`## Sender`, `## Today`, `## Calendar`, `## Tasks`, `## Response Style`)
/// none of which the current builder emits — the `1 structural + 1
/// thematic` branch of [`is_cascade_leak`] was therefore dead, and the
/// detector silently degraded to a `2+ structural` rule (drift exposed by
/// the `silent_response::tests::thematic_and_scaffold_headers_match_prompt_builder_output`
/// drift-pin in PR #5344). This list now mirrors the actual headers
/// enumerated by `crates/librefang-runtime/src/prompt_builder/*.rs` plus
/// the legacy five kept as belt-and-suspenders for any deployment still
/// on the older builder (cheap to leave; same false-positive risk as
/// before).
///
/// **Ambiguous subset** — `## Today`, `## Calendar`, `## Tasks`, and
/// `## Current Date` are not promoted into [`SCAFFOLD_ONLY_HEADERS`]:
/// a legitimate "what does my day look like" / "what's today's date"
/// reply may produce these. The houko-flagged false-positive guard
/// `cascade_leak_ambiguous_thematic_headers_alone_are_legitimate` keeps
/// that invariant locked.
const THEMATIC_HEADERS: &[&str] = &[
    // -- Current builder (post-#5053 / #5073) -------------------------------
    "## Active Goals",
    "## Channel",
    "## Connected Tool Servers (MCP)",
    "## Current Date",
    "## First-Run Protocol",
    "## Heartbeat Checklist",
    "## Identity",
    "## Live Context",
    "## Long-Term Memory",
    "## Memory",
    "## Operational Guidelines",
    "## Output Channels",
    "## Peer Agents",
    "## Persona",
    "## Provider-Supplied Context",
    "## Safety",
    "## Skills",
    "## Tool Call Behavior",
    "## User Context",
    "## User Profile",
    "## Workspace",
    "## Your Tools",
    // -- Legacy (pre-#5053) — kept defensively -------------------------------
    "## Sender",
    "## Today",
    "## Calendar",
    "## Tasks",
    "## Response Style",
];

/// Subset of [`THEMATIC_HEADERS`] that describe the *prompt frame itself*,
/// not reply content. An agent never legitimately emits `## Persona`
/// (it does not narrate its own meta-frame to the user) or `## Memory`,
/// `## Skills`, `## Workspace`, `## Safety`, …  in a genuine reply — these
/// only appear when the model regurgitates the scaffolding verbatim. They
/// are therefore as diagnostic as a structural turn-frame marker. This
/// closes the all-thematic bypass (`## Persona\n…\n## Memory\n…\n## Skills`
/// — three thematic, zero structural in the legacy spelling — would have
/// slipped through `2+ structural`) without regressing the legitimate
/// "what does my day look like" reply (which uses the *ambiguous* subset
/// — `## Today`, `## Calendar`, `## Tasks`, `## Current Date` — that
/// remains in [`THEMATIC_HEADERS`] but is intentionally NOT in this list).
const SCAFFOLD_ONLY_HEADERS: &[&str] = &[
    // -- Current builder (post-#5053 / #5073) — all are pure prompt-frame ----
    // (`## Current Date` is intentionally NOT here — see THEMATIC_HEADERS
    // comment for the ambiguous-subset rationale.)
    "## Active Goals",
    "## Channel",
    "## Connected Tool Servers (MCP)",
    "## First-Run Protocol",
    "## Heartbeat Checklist",
    "## Identity",
    "## Live Context",
    "## Long-Term Memory",
    "## Memory",
    "## Operational Guidelines",
    "## Output Channels",
    "## Peer Agents",
    "## Persona",
    "## Provider-Supplied Context",
    "## Safety",
    "## Skills",
    "## Tool Call Behavior",
    "## User Context",
    "## User Profile",
    "## Workspace",
    "## Your Tools",
    // -- Legacy (pre-#5053) — kept defensively -------------------------------
    "## Sender",
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
    if has_envelope_marker_as_line_leader(text) {
        return true;
    }

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

    // --- Post-#5053 / #5073 prompt-builder shapes (reconciliation) --------

    #[test]
    fn cascade_leak_current_builder_scaffold_only_pair_trips() {
        // Two SCAFFOLD-ONLY headers from the current builder → 2
        // structural-equivalent hits → leak. Pre-reconciliation this
        // exact shape slipped through (legacy THEMATIC_HEADERS contained
        // only the pre-#5053 strings).
        assert!(is_cascade_leak(
            "## Persona\nAmbrogio, butler.\n\n## Memory\n- prefers Italian\n- key=value"
        ));
        assert!(is_cascade_leak(
            "## Skills\n- file_read\n- notify_owner\n\n## Workspace\n/tmp/ws"
        ));
        assert!(is_cascade_leak(
            "## Identity\nAmbrogio v1\n\n## Channel\ntelegram"
        ));
    }

    #[test]
    fn cascade_leak_current_builder_scaffold_plus_envelope_trips() {
        // 1 envelope prefix (structural) + 1 SCAFFOLD-ONLY (also
        // structural-equivalent) → 2 structural hits → leak.
        assert!(is_cascade_leak(
            "[Group message from Alice]\n## Persona\nAmbrogio"
        ));
    }

    #[test]
    fn cascade_leak_current_builder_ambiguous_alone_legitimate() {
        // `## Current Date` is the only AMBIGUOUS header in today's
        // builder set — a legitimate "what's today?" reply may produce
        // it. Pinned NOT a leak when standalone.
        assert!(!is_cascade_leak(
            "## Current Date\nFriday, 2026-05-16\n\nYour day looks clear."
        ));
    }

    #[test]
    fn cascade_leak_current_builder_ambiguous_plus_structural_trips() {
        // 1 ambiguous thematic + 1 structural → leak (the canonical
        // "1+1" branch that was dead pre-reconciliation).
        assert!(is_cascade_leak(
            "[Past exchange]\nYou said hello.\n\n## Current Date\n2026-05-16"
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

    // -----------------------------------------------------------------------
    // Drift pin: cascade-leak markers vs real `build_system_prompt` output.
    //
    // This is the narrower re-port of the `keywords_match_real_prompt_headers`
    // test from closed PR #4760, retargeted for the post-#5053 layout and the
    // post-#5073 `granted_tool_hints` `PromptContext` shape. The closing
    // comment on #4760 singled this test out as "the most valuable single
    // piece" of that branch.
    //
    // Why it exists: `is_cascade_leak` (#4907) hard-codes string literals in
    // `THEMATIC_HEADERS`, `SCAFFOLD_ONLY_HEADERS`, `STRUCTURAL_TURN_FRAMES`,
    // `ENVELOPE_LINE_PREFIXES`, and `ENVELOPE_STANDALONE_MARKERS`. The
    // detector trips when 2+ of these markers appear in an agent reply.
    //
    // Two failure modes the pin guards against:
    //
    // 1. **Positive drift** — the prompt builder renames a section the
    //    detector watches for (e.g. `## Sender` → `## From`). The detector
    //    keeps looking for the old string, so the cascade-leak it was tuned
    //    to catch (#5141: model regurgitates `## Sender\n…\n## Today\n…`)
    //    silently stops firing. CI stays green because no test ever ran the
    //    builder and grepped its output for the marker strings.
    //
    // 2. **Negative drift** — the prompt builder starts emitting a string
    //    the detector treats as a leak indicator (e.g. a new
    //    `## Live Context\nUser asked: …` template, or an envelope marker
    //    bleeding into a builder section). Real replies would then trip on
    //    their own scaffolding. The pin catches this by also asserting
    //    the `STRUCTURAL_TURN_FRAMES` / `ENVELOPE_*` markers do **not**
    //    appear in any fully-populated `build_system_prompt` output.
    // -----------------------------------------------------------------------

    /// Build a maximally-populated `PromptContext` so every conditional
    /// section in `build_system_prompt` fires. The exact values don't
    /// matter — only the resulting `## <header>` shapes and any
    /// gateway-marker bleed do.
    ///
    /// Kept inside this test module so adding a new `PromptContext` field
    /// downstream produces a compile error here too, forcing the drift-pin
    /// fixture to stay current.
    fn fully_populated_prompt_context() -> crate::prompt_builder::PromptContext {
        use std::collections::BTreeMap;
        let mut granted_tool_hints = BTreeMap::new();
        granted_tool_hints.insert("file_read".to_string(), "read file contents".to_string());
        granted_tool_hints.insert(
            "notify_owner".to_string(),
            "send a private message to the owner".to_string(),
        );

        crate::prompt_builder::PromptContext {
            agent_name: "ambrogio".to_string(),
            agent_description: "butler".to_string(),
            base_system_prompt: "You are Ambrogio.".to_string(),
            granted_tools: vec![
                "file_read".to_string(),
                // Triggers Section 9.6 (`## Output Channels`).
                "notify_owner".to_string(),
            ],
            granted_tool_hints,
            recalled_memories: vec![("k".to_string(), "v".to_string())],
            skill_summary: "skill-a\nskill-b".to_string(),
            skill_count: 2,
            skill_prompt_context: String::new(),
            skill_config_section: String::new(),
            mcp_summary: "mempalace: 19 tools".to_string(),
            workspace_path: Some("/tmp/ws".to_string()),
            soul_md: Some("Be helpful.".to_string()),
            user_md: Some("Signore.".to_string()),
            memory_md: Some("notes".to_string()),
            canonical_context: Some("ctx".to_string()),
            // Deliberately `None` so the first-run protocol section fires
            // (it only injects when no `user_name` memory exists and the
            // `user_name` context field is unset).
            user_name: None,
            channel_type: Some("telegram".to_string()),
            sender_display_name: Some("Signore".to_string()),
            sender_user_id: Some("123".to_string()),
            is_group: true,
            was_mentioned: true,
            is_subagent: false,
            is_autonomous: true,
            agents_md: Some("agents".to_string()),
            bootstrap_md: Some("bootstrap".to_string()),
            workspace_context: Some("ws ctx".to_string()),
            identity_md: Some("identity".to_string()),
            heartbeat_md: Some("heartbeat".to_string()),
            tools_md: Some("tools".to_string()),
            peer_agents: vec![("peer".to_string(), "Idle".to_string(), "haiku".to_string())],
            current_date: Some("Friday, 2026-05-16".to_string()),
            active_goals: vec![("goal".to_string(), "in_progress".to_string(), 50)],
            context_md: Some("ctx-md".to_string()),
            dynamic_sections: vec![crate::hooks::DynamicSection {
                provider: "test".to_string(),
                heading: "Recent Activity".to_string(),
                body: "User logged in at 09:00.".to_string(),
            }],
        }
    }

    /// **Negative pin** — markers the detector relies on being absent
    /// from `build_system_prompt`. If any of these strings leak into the
    /// rendered prompt, `is_cascade_leak` would false-positive on the
    /// agent's own scaffolding the moment the model paraphrased it back.
    #[test]
    fn structural_and_envelope_markers_absent_from_prompt_builder() {
        let prompt = crate::prompt_builder::build_system_prompt(&fully_populated_prompt_context());

        for marker in STRUCTURAL_TURN_FRAMES {
            assert!(
                !prompt.contains(marker),
                "STRUCTURAL_TURN_FRAMES entry {marker:?} is now emitted by \
                 build_system_prompt. `is_cascade_leak` would false-positive \
                 on the agent's own context the moment the model paraphrased \
                 the section back. Either rename the builder section, drop \
                 the marker from STRUCTURAL_TURN_FRAMES, or split the marker \
                 list into prompt-safe / runtime-only halves.\n\nRendered \
                 prompt:\n{prompt}",
            );
        }

        for marker in ENVELOPE_LINE_PREFIXES {
            assert!(
                !prompt.contains(marker),
                "ENVELOPE_LINE_PREFIXES entry {marker:?} is now emitted by \
                 build_system_prompt. Envelope prefixes are gateway-injected \
                 inbound markers — they must never appear in the system \
                 prompt or the cascade-leak guard will trip on its own \
                 scaffolding.\n\nRendered prompt:\n{prompt}",
            );
        }

        for marker in ENVELOPE_STANDALONE_MARKERS {
            assert!(
                !prompt.contains(marker),
                "ENVELOPE_STANDALONE_MARKERS entry {marker:?} is now emitted \
                 by build_system_prompt. Standalone envelope markers \
                 ([Stranger] / [Forwarded] / [User]) are gateway-injected — \
                 they must never appear in the system prompt or the \
                 cascade-leak guard will trip on its own scaffolding.\n\n\
                 Rendered prompt:\n{prompt}",
            );
        }
    }

    /// **Positive pin** — snapshot the present-day mapping between every
    /// cascade-leak marker header and whether `build_system_prompt`
    /// actually emits that exact header today. Any future divergence
    /// from this snapshot — whether a section is renamed, removed, or
    /// (re)introduced under one of these names — fails the test.
    ///
    /// The snapshot is the load-bearing piece. Markers tagged `present`
    /// must match a `## <header>` line in the rendered prompt; markers
    /// tagged `absent` must NOT match any such line. Drift in either
    /// direction is meaningful:
    ///
    /// - `present → absent`: a section the detector is tuned against has
    ///   been renamed or removed in `prompt_builder.rs`. The detector
    ///   keeps looking for the dead string and silently weakens the
    ///   real cascade-leak pattern (#5141) it was meant to catch.
    /// - `absent → present`: a builder section just adopted one of these
    ///   names. Real replies that paraphrase that section now look like
    ///   regurgitation to the detector and trip on legitimate output.
    ///
    /// Fix path on failure: update the snapshot deliberately, and in
    /// the same commit either (a) rename the marker constant to the
    /// builder's new section name, (b) restore the section in the
    /// builder under the marker's name, or (c) drop the marker from
    /// `THEMATIC_HEADERS` / `SCAFFOLD_ONLY_HEADERS`.
    ///
    /// Snapshot taken against `upstream/main` at the post-#5053 layout
    /// and post-#5073 `granted_tool_hints` `PromptContext` shape. The
    /// `false` entries here document a pre-existing divergence: the
    /// detector's thematic and scaffold-only header lists were
    /// calibrated against the pre-refactor prompt builder and no longer
    /// match any section name in today's `build_system_prompt`. This is
    /// the drift the closed-PR-#4760 reviewer asked the re-port to
    /// surface — reconciling the marker constants (or restoring the
    /// sections) is out of scope for this test PR.
    #[test]
    fn thematic_and_scaffold_headers_match_prompt_builder_output() {
        // Snapshot: (marker, whether it is currently emitted as a
        // `## <header>` line by `build_system_prompt`). MUST be kept in
        // lock-step with `THEMATIC_HEADERS` + `SCAFFOLD_ONLY_HEADERS`.
        //
        // Post-reconciliation (this PR — fix/silent-response-reconcile-thematic-headers):
        // the legacy five (`## Sender`, `## Today`, `## Calendar`,
        // `## Tasks`, `## Response Style`) stay tagged `false` — they are
        // kept in THEMATIC_HEADERS as a defensive belt-and-suspenders
        // layer for any deployment still on the pre-#5053 builder. The
        // current-builder strings (`## Persona`, `## Memory`, …) are now
        // tagged `true` and the detector's "1 structural + 1 thematic"
        // branch is finally able to trip on the post-#5053 leak shape.
        const EXPECTED_EMISSION: &[(&str, bool)] = &[
            // -- Current builder (post-#5053 / #5073) — all emitted by
            // `build_system_prompt(fully_populated_prompt_context())` ---------
            ("## Active Goals", true),
            ("## Channel", true),
            ("## Connected Tool Servers (MCP)", true),
            ("## Current Date", true),
            ("## First-Run Protocol", true),
            ("## Heartbeat Checklist", true),
            ("## Identity", true),
            ("## Live Context", true),
            ("## Long-Term Memory", true),
            ("## Memory", true),
            ("## Operational Guidelines", true),
            ("## Output Channels", true),
            ("## Peer Agents", true),
            ("## Persona", true),
            ("## Provider-Supplied Context", true),
            ("## Safety", true),
            ("## Skills", true),
            ("## Tool Call Behavior", true),
            ("## User Context", true),
            ("## User Profile", true),
            ("## Workspace", true),
            ("## Your Tools", true),
            // -- Legacy (pre-#5053) — defensively retained, not emitted ------
            ("## Sender", false),
            ("## Today", false),
            ("## Calendar", false),
            ("## Tasks", false),
            ("## Response Style", false),
        ];

        // Cross-check: every marker in `THEMATIC_HEADERS` and
        // `SCAFFOLD_ONLY_HEADERS` must appear in the snapshot. Adding a
        // new marker constant without updating the snapshot fails here.
        for marker in THEMATIC_HEADERS.iter().chain(SCAFFOLD_ONLY_HEADERS.iter()) {
            assert!(
                EXPECTED_EMISSION.iter().any(|(m, _)| m == marker),
                "Cascade-leak marker {marker:?} is in THEMATIC_HEADERS / \
                 SCAFFOLD_ONLY_HEADERS but missing from the drift-pin \
                 snapshot. Add an entry to EXPECTED_EMISSION above with \
                 the appropriate `true`/`false` reflecting whether \
                 build_system_prompt currently emits that header.",
            );
        }

        // Reverse cross-check: every snapshot entry must still be a
        // current marker. Dropping a marker from THEMATIC_HEADERS /
        // SCAFFOLD_ONLY_HEADERS without pruning the snapshot fails here.
        for (marker, _) in EXPECTED_EMISSION {
            assert!(
                THEMATIC_HEADERS.contains(marker) || SCAFFOLD_ONLY_HEADERS.contains(marker),
                "Drift-pin snapshot entry {marker:?} is no longer in \
                 THEMATIC_HEADERS or SCAFFOLD_ONLY_HEADERS. Remove it \
                 from EXPECTED_EMISSION above (the marker was dropped \
                 — the snapshot must shrink with it).",
            );
        }

        let prompt = crate::prompt_builder::build_system_prompt(&fully_populated_prompt_context());

        // Headers seen in the rendered prompt, with the leading `## `
        // stripped, so we can compare against marker strings using a
        // case-insensitive header-equality check (matching the
        // `is_cascade_leak` `.contains()` semantics conservatively: a
        // marker `.contains()`-matches the prompt iff some `## <header>`
        // line is exactly that header).
        let emitted_headers: Vec<&str> = prompt
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("## "))
            .collect();

        for (marker, expected_present) in EXPECTED_EMISSION {
            let needle = marker.trim_start_matches("## ");
            let actually_present = emitted_headers
                .iter()
                .any(|h| h.eq_ignore_ascii_case(needle));

            if *expected_present && !actually_present {
                panic!(
                    "Cascade-leak marker {marker:?} was tagged `present` in \
                     the drift-pin snapshot but no longer matches any \
                     `## <header>` emitted by build_system_prompt. The \
                     detector in `is_cascade_leak` is now calibrated \
                     against a ghost — a section that used to exist (or \
                     was renamed) in prompt_builder.rs without a \
                     corresponding update here. Either restore the \
                     section under the old name, rename the marker to \
                     match the new section name, drop the marker from \
                     THEMATIC_HEADERS / SCAFFOLD_ONLY_HEADERS, or update \
                     the snapshot to `false` deliberately.\n\n\
                     Headers emitted today:\n  ## {}",
                    emitted_headers.join("\n  ## "),
                );
            }

            if !*expected_present && actually_present {
                panic!(
                    "Cascade-leak marker {marker:?} was tagged `absent` in \
                     the drift-pin snapshot but build_system_prompt now \
                     emits a `## <header>` line that matches it. A \
                     legitimate reply that paraphrases the new section \
                     will trip `is_cascade_leak` on its own scaffolding. \
                     Either rename the new builder section, drop the \
                     marker from THEMATIC_HEADERS / SCAFFOLD_ONLY_HEADERS, \
                     or update the snapshot to `true` deliberately (and \
                     audit `is_cascade_leak` for false-positive risk on \
                     the now-legitimate header).\n\n\
                     Headers emitted today:\n  ## {}",
                    emitted_headers.join("\n  ## "),
                );
            }
        }
    }

    // -- Chat-template tokenizer-bleed tests (2026-05-20 WhatsApp incident) --

    #[test]
    fn cascade_leak_observed_no_reply_then_user_template_bleed() {
        let observed =
            "NO_REPLY\n\n[User]\nEhi puoi dirmi che skill hai e in linea di massima cosa sai fare?";
        assert!(
            is_cascade_leak(observed),
            "observed NO_REPLY + [User] line-leader template bleed must trip"
        );
    }

    #[test]
    fn cascade_leak_line_leader_requires_silent_sentinel_before() {
        assert!(is_cascade_leak("NO_REPLY\n[User]\nfake question"));
        assert!(is_cascade_leak("[no reply needed]\n[User]\nfake question"));
        assert!(is_cascade_leak(
            "NO_REPLY\n\n[Stranger]\nciao da sconosciuto"
        ));
        assert!(is_cascade_leak("NO_REPLY\n[Forwarded]\nti inoltro questo"));
    }

    #[test]
    fn cascade_leak_line_leader_without_sentinel_does_not_trip_alone() {
        assert!(!is_cascade_leak("preamble\n[User]\nfake question"));
        assert!(!is_cascade_leak("[User]\nfake question"));
        assert!(!is_cascade_leak("[User] inline content on same line"));
    }

    #[test]
    fn cascade_leak_bare_user_marker_at_end_not_leak() {
        assert!(!is_cascade_leak("the [User]"));
        assert!(!is_cascade_leak("see [Stranger]"));
    }

    #[test]
    fn cascade_leak_marker_substring_not_line_leader() {
        assert!(!is_cascade_leak("the [User] tag is documented in the spec"));
        assert!(!is_cascade_leak(
            "see the [Forwarded] convention used by gateways"
        ));
    }

    #[test]
    fn cascade_leak_explanatory_envelope_mid_text_not_silenced() {
        assert!(!is_cascade_leak(
            "Here is what an inbound looks like:\n\n[User]\nHello, can you help?"
        ));
    }

    #[test]
    fn incremental_fires_on_user_template_bleed() {
        let deltas = [
            "NO_REPLY",
            "\n\n",
            "[User]\n",
            "Ehi puoi dirmi che skill hai…",
        ];
        let (fired, idx) = feed_deltas(&deltas);
        assert!(
            fired,
            "[User] line-leader bleed should trip incremental guard"
        );
        assert!(
            idx <= 2,
            "should fire no later than after the [User]\\n delta, fired at {idx}"
        );
    }

    #[test]
    fn cascade_leak_crlf_and_tab_after_marker() {
        assert!(is_cascade_leak(
            "NO_REPLY\n[User]\r\nfake question via CRLF"
        ));
        assert!(is_cascade_leak("NO_REPLY\n[Stranger]\tcontent after tab"));
    }

    #[test]
    fn lowercase_marker_not_detected_by_design() {
        assert!(!is_cascade_leak("NO_REPLY\n[user]\nfake question"));
        assert!(!is_cascade_leak("NO_REPLY\n[stranger]\nciao"));
    }
}
