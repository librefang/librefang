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

    // Order is hot-path-first: `matches_placeholder_tag` rejects in two
    // byte loads when the trimmed text lacks a leading `<`; the other
    // two predicates lowercase-allocate the input.
    if matches_placeholder_tag(stripped) {
        return true;
    }

    if matches_canonical(stripped) {
        return true;
    }

    // Trailing-suffix tolerance: legacy prompts sometimes put context BEFORE
    // the sentinel ("all good. NO_REPLY"). The sentinel must follow a
    // non-word boundary (whitespace, punctuation, newline, or emoji), and
    // it must be the LAST token (after the same trailing-noise strip).
    ends_with_canonical(stripped)
}

/// Recognise short `<placeholder>` strings the model emits when it
/// misinterprets "respond with no message" as "respond with the
/// placeholder for nothing". Whole-message only — embedded tags inside
/// real prose stay live. The 32-char ceiling and the no-whitespace
/// constraint together protect real HTML / XML payloads (attributes,
/// nested content, longer tag names).
fn matches_placeholder_tag(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 3 || bytes.len() > 32 {
        return false;
    }
    if bytes[0] != b'<' || bytes[bytes.len() - 1] != b'>' {
        return false;
    }
    let inner = &s[1..s.len() - 1];
    if inner.is_empty() {
        return false;
    }
    // Reject anything resembling a real HTML/XML tag: attributes
    // (whitespace), closing slashes, nested brackets.
    inner
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whole-message template-wrap pairs the model occasionally emits when it
/// "answers" by reciting its own context. No legitimate prompt instruction
/// asks for that wrapping.
const PROMPT_LEAK_TAG_PAIRS: &[(&str, &str)] = &[
    ("<answer>", "</answer>"),
    ("<response>", "</response>"),
    ("<reply>", "</reply>"),
];

/// Minimum number of system-prompt-shaped `## ` headers required to flag
/// a response. Two avoids false-positives on legit summaries that share
/// only one section name (e.g. an answer with a single `## Tasks`
/// section).
const PROMPT_LEAK_HEADER_THRESHOLD: usize = 2;

/// System-prompt section header lower-cased prefixes. Real replies don't
/// emit `## Sender` or `## Calendar`; the system prompt does. Match is
/// case-insensitive on the header text after `## `.
///
/// **Drift hazard**: this list mirrors section names emitted by
/// `prompt_builder.rs`. Adding or renaming a section there without
/// updating this list silently weakens the regurgitation guard. A
/// follow-up should either bind both to a shared const, or add a
/// regression test that builds a stub system prompt and asserts every
/// keyword still appears as a real header.
const PROMPT_LEAK_HEADER_KEYWORDS: &[&str] = &[
    "sender",
    "today",
    "calendar",
    "tasks",
    "memory",
    "memories",
    "persona",
    "tools",
    "current date",
    "context",
    "skills",
    "agents",
    "operational",
];

/// Classify a response as a system-prompt leak: the model regurgitated
/// chunks of its own context (memory bullets, dynamic sections, persona
/// blocks) wrapped in a pseudo-template tag like `<answer>...</answer>`,
/// or dumped a sequence of Markdown H2 headers matching system-prompt
/// section names that never appear in legitimate replies.
///
/// On match the runtime swallows the response (silent completion), logs
/// `event=system_prompt_regurgitated`, and lets the operator inspect the
/// original text via the structured log.
pub fn is_prompt_leak(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with('<') {
        for (open, close) in PROMPT_LEAK_TAG_PAIRS {
            if t.starts_with(open) && t.ends_with(close) {
                return true;
            }
        }
    }
    if !t.contains("## ") {
        return false;
    }
    let prompt_shaped_headers = t
        .lines()
        .filter(|line| {
            let s = line.trim_start();
            let Some(rest) = s.strip_prefix("## ") else {
                return false;
            };
            let header = rest.trim().as_bytes();
            // Keywords are ASCII-only English section names, so byte-wise
            // case-insensitive compare is safe and zero-alloc. Match must
            // be whole-word to avoid prefix over-fire (e.g. `## Senders
            // Update` should not match `sender`, `## Contextual Note`
            // should not match `context`). The boundary check accepts the
            // header end OR any non-alphanumeric trailing char (space,
            // colon, punctuation).
            PROMPT_LEAK_HEADER_KEYWORDS.iter().any(|kw| {
                let kw = kw.as_bytes();
                if header.len() < kw.len() || !header[..kw.len()].eq_ignore_ascii_case(kw) {
                    return false;
                }
                match header.get(kw.len()) {
                    None => true,
                    Some(b) => !b.is_ascii_alphanumeric(),
                }
            })
        })
        .take(PROMPT_LEAK_HEADER_THRESHOLD)
        .count();
    prompt_shaped_headers >= PROMPT_LEAK_HEADER_THRESHOLD
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

    // --- Placeholder-tag defensive guard ---
    #[test]
    fn empty_placeholder_tag_is_silent() {
        // Production leak: model emits `<empty>` when prompt says
        // "return an empty message"; without this guard the literal
        // string reaches the user.
        assert!(is_silent_response("<empty>"));
        assert!(is_silent_response("<response>"));
        assert!(is_silent_response("<silent>"));
        assert!(is_silent_response("<no_reply>"));
        assert!(is_silent_response("<NO_REPLY>"));
        // Trailing punctuation / whitespace tolerated by the upstream
        // strip_trailing_noise pass.
        assert!(is_silent_response("<empty>."));
        assert!(is_silent_response("  <response>  "));
        assert!(is_silent_response("<silent>\n"));
    }

    #[test]
    fn real_html_xml_payload_is_not_silent() {
        // Tags with attributes, inner content, or closing markers must
        // stay deliverable — the user actually wrote HTML/XML.
        assert!(!is_silent_response("<a href=\"x\">link</a>"));
        assert!(!is_silent_response("<p>Hello</p>"));
        assert!(!is_silent_response("<br/>"));
        assert!(!is_silent_response("<img src=\"x\">"));
        // A bare-but-long tag name is still a payload, not a sentinel.
        assert!(!is_silent_response(
            "<reallyLongTagNameThatIsForRealUseFortyChars>"
        ));
        // Nested brackets, multiple tags, or surrounding text are real
        // content.
        assert!(!is_silent_response("Result: <empty>."));
        assert!(!is_silent_response("<a><b>"));
    }

    #[test]
    fn malformed_placeholder_is_not_silent() {
        // Half-tags and inner whitespace must not match.
        assert!(!is_silent_response("<empty"));
        assert!(!is_silent_response("empty>"));
        assert!(!is_silent_response("<<empty>>"));
        assert!(!is_silent_response("<empty response>"));
        assert!(!is_silent_response("<>"));
    }

    // --- is_prompt_leak — output guard ---

    #[test]
    fn template_wrapped_response_is_dropped() {
        assert!(is_prompt_leak(
            "<answer>\nUser asked: foo\nI responded: bar\n</answer>"
        ));
        assert!(is_prompt_leak("<response>some content</response>"));
        assert!(is_prompt_leak("<reply>x</reply>"));
        assert!(is_prompt_leak("  <answer>x</answer>  "));
    }

    #[test]
    fn system_prompt_shaped_headers_are_dropped() {
        // The shape we observed in the 2026-05-06 incident: model
        // regurgitates dynamic system-prompt sections verbatim.
        assert!(is_prompt_leak(
            "## Sender\nMessage from: X\n## Today\nWednesday\n## Calendar\nNo events.\n## Tasks\npending\n"
        ));
        // Two distinct prompt-shaped headers is enough to flag.
        assert!(is_prompt_leak("## Sender\nfoo\n## Calendar\nbar"));
        // Case-insensitive on the header text.
        assert!(is_prompt_leak("## SENDER\nfoo\n## tasks\nbar"));
    }

    #[test]
    fn legitimate_technical_summary_is_delivered() {
        // Three or more H2 headers are fine when the keywords are
        // domain-content, not system-prompt section names.
        assert!(!is_prompt_leak(
            "## Approach\nUse async\n## Implementation\nspawn a task\n## Risks\nrace on shutdown"
        ));
        assert!(!is_prompt_leak(
            "## Setup\nfoo\n## Run\nbar\n## Verify\nbaz\n## Cleanup\nqux"
        ));
    }

    #[test]
    fn plain_reply_is_delivered() {
        assert!(!is_prompt_leak(""));
        assert!(!is_prompt_leak("Subito, Signore."));
        assert!(!is_prompt_leak("Ho registrato la spesa."));
        assert!(!is_prompt_leak("## Update\nFatto."));
        // Even one prompt-shaped header alone is allowed (legit "## Tasks"
        // section in a status reply); two are required.
        assert!(!is_prompt_leak("## Sender\nfoo"));
    }

    #[test]
    fn unwrapped_angle_bracket_tag_is_delivered() {
        // A `<answer>` mention without a matching close tag is real
        // content (the model is explaining or quoting), not a wrap.
        assert!(!is_prompt_leak("<answer> is a literal tag I'm explaining"));
        assert!(!is_prompt_leak("Use the <answer> element here"));
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
