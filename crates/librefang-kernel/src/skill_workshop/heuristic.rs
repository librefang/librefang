//! Cheap pattern-match scanners that decide whether a turn carried a
//! "user is teaching the agent a workflow" signal (#3328).
//!
//! Each scanner returns at most one [`HeuristicHit`] — the [`mod.rs`]
//! pipeline merges hits from all enabled scanners and forwards the
//! winners to the LLM review stage (or directly to disk, depending on
//! [`librefang_types::agent::ReviewMode`]).
//!
//! Scanners are intentionally conservative: false negatives are cheap
//! (the user can re-teach later), false positives spam the pending
//! queue. When in doubt, drop.

use crate::skill_workshop::candidate::CaptureSource;
use regex::Regex;
use std::sync::OnceLock;

/// Output of a successful heuristic match — enough to seed a
/// [`CandidateSkill`](super::candidate::CandidateSkill); the workshop
/// pipeline fills in id / agent_id / session_id / captured_at /
/// turn_index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeuristicHit {
    /// Suggested skill name in snake_case.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Markdown body for the future skill's `prompt_context.md`.
    pub prompt_context: String,
    /// Which signal fired.
    pub source: CaptureSource,
    /// Truncated user message excerpt for provenance.
    pub user_message_excerpt: String,
    /// Truncated assistant response excerpt, if any.
    pub assistant_response_excerpt: Option<String>,
}

/// Triggers that mark imperative teaching: "from now on, always X",
/// "remember to X", etc. Listed in priority order — the first match
/// becomes the recorded trigger label on the candidate.
const EXPLICIT_TRIGGERS: &[&str] = &[
    r"(?i)\bfrom now on\b",
    r"(?i)\bplease always\b",
    r"(?i)\b(you|we) should always\b",
    r"(?i)\bremember to\b",
    r"(?i)\balways (run|use|prefer|check|do|invoke|call|include|add)\b",
    r"(?i)\bthe way to (do|handle|fix|build) ",
    r"(?i)\bnever (run|use|skip|forget|omit) ",
];

/// Triggers that mark a user correction: "no, do it like X", "not Y but Z".
const CORRECTION_TRIGGERS: &[&str] = &[
    r"(?i)\bno,?\s+(do it|that's|that is|the (right|correct))\b",
    r"(?i)\bnot\s+\S+\s+but\s+\S+",
    r"(?i)\b(don't|do not)\s+(use|run|call|invoke|do)\b",
    r"(?i)\bshould(n't| not| be)\s+\S+.*\binstead\b",
    r"(?i)\bwrong\s*[,—:]\s+\S+",
    r"(?i)\bactually,?\s+(it'?s|the way)\b",
];

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(p).expect("hardcoded regex must compile"))
        .collect()
}

fn explicit_regexes() -> &'static [Regex] {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    CELL.get_or_init(|| compile(EXPLICIT_TRIGGERS))
}

fn correction_regexes() -> &'static [Regex] {
    static CELL: OnceLock<Vec<Regex>> = OnceLock::new();
    CELL.get_or_init(|| compile(CORRECTION_TRIGGERS))
}

/// Inspect a user message for an explicit teaching imperative.
///
/// Returns `Some` when one of [`EXPLICIT_TRIGGERS`] matches the user's
/// most recent message. The `prompt_context` is built from the matched
/// sentence — keeping the user's own phrasing prevents the workshop from
/// inventing intent.
pub fn extract_explicit_instruction(user_message: &str) -> Option<HeuristicHit> {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return None;
    }
    for re in explicit_regexes() {
        if let Some(m) = re.find(trimmed) {
            let trigger = m.as_str().to_string();
            let sentence = sentence_around(trimmed, m.start(), m.end());
            // Skip noisy false positives: too short to teach anything,
            // or trigger appears inside a question.
            if sentence.chars().count() < 12 || sentence.trim_end().ends_with('?') {
                continue;
            }
            let name = synth_name(&sentence, "rule");
            let description = one_line_summary(&sentence, 120);
            let prompt_context = build_explicit_prompt_context(&sentence, &trigger);
            return Some(HeuristicHit {
                name,
                description,
                prompt_context,
                source: CaptureSource::ExplicitInstruction { trigger },
                user_message_excerpt: super::candidate::truncate_excerpt(trimmed),
                assistant_response_excerpt: None,
            });
        }
    }
    None
}

/// Inspect a user message for a correction relative to the previous
/// assistant turn.
///
/// `prev_assistant` is the assistant's most recent reply — used as
/// context for the prompt body so the future skill explains *what* the
/// agent should do differently. Without it the correction has no
/// reference frame and the scanner returns `None`.
pub fn extract_user_correction(
    prev_assistant: Option<&str>,
    user_message: &str,
) -> Option<HeuristicHit> {
    let trimmed = user_message.trim();
    let prev = prev_assistant.unwrap_or("").trim();
    if trimmed.is_empty() || prev.is_empty() {
        return None;
    }
    for re in correction_regexes() {
        if let Some(m) = re.find(trimmed) {
            let trigger = m.as_str().to_string();
            let sentence = sentence_around(trimmed, m.start(), m.end());
            if sentence.chars().count() < 8 {
                continue;
            }
            let name = synth_name(&sentence, "correction");
            let description = one_line_summary(&sentence, 120);
            let prompt_context = build_correction_prompt_context(prev, &sentence, &trigger);
            return Some(HeuristicHit {
                name,
                description,
                prompt_context,
                source: CaptureSource::UserCorrection { trigger },
                user_message_excerpt: super::candidate::truncate_excerpt(trimmed),
                assistant_response_excerpt: Some(super::candidate::truncate_excerpt(prev)),
            });
        }
    }
    None
}

/// Detect a tool-call sequence that has been repeated three or more times
/// across the recent invocation history.
///
/// `recent_tools` is an ordered list of tool names — newest last. We
/// scan for subsequences of length 1..=`MAX_PATTERN_LEN` that appear
/// `MIN_REPEATS` or more times in the history; the longest such pattern
/// becomes the candidate (longer patterns are more useful as packaged
/// skills than single repeated tools).
pub fn extract_repeated_tool_pattern(recent_tools: &[String]) -> Option<HeuristicHit> {
    /// Maximum subsequence length to consider. Keeps the cost O(N·L).
    const MAX_PATTERN_LEN: usize = 4;
    /// Minimum non-overlapping occurrences before a pattern is reported.
    const MIN_REPEATS: u32 = 3;
    /// Bound on history scanned, so a long-lived agent doesn't keep
    /// re-flagging the same primordial pattern.
    const HISTORY_WINDOW: usize = 30;

    if recent_tools.is_empty() {
        return None;
    }
    let window_start = recent_tools.len().saturating_sub(HISTORY_WINDOW);
    let window = &recent_tools[window_start..];

    let mut best: Option<(Vec<String>, u32)> = None;
    for len in (1..=MAX_PATTERN_LEN.min(window.len())).rev() {
        for start in 0..=window.len().saturating_sub(len) {
            let pattern = &window[start..start + len];
            // Filter degenerate single-tool patterns: a length-1 pattern
            // for a hyper-common tool like "shell" would fire constantly.
            // Require length-1 patterns be non-trivial: more conservative
            // than length>1, since longer patterns are inherently rarer.
            let needed = if len == 1 {
                MIN_REPEATS + 1
            } else {
                MIN_REPEATS
            };
            let count = count_non_overlapping(window, pattern);
            if count >= needed {
                let pattern_owned: Vec<String> = pattern.to_vec();
                match &best {
                    None => best = Some((pattern_owned, count)),
                    Some((cur, _)) if cur.len() < len => best = Some((pattern_owned, count)),
                    _ => {}
                }
            }
        }
        if best.as_ref().map(|(p, _)| p.len()) == Some(len) {
            // Found at this length; longer search is also done since we
            // descend from MAX_PATTERN_LEN. Break early once we have a
            // candidate at the longest viable length.
            break;
        }
    }

    let (pattern, count) = best?;
    let tools_label = pattern.join(",");
    let name = format!(
        "tool_sequence_{}",
        sanitise_name_segment(&pattern.join("_"))
    );
    let description = format!(
        "Bundle the recurring tool sequence ({tools_label}) repeated {count}× into a single skill"
    );
    let prompt_context = build_repeated_pattern_prompt_context(&pattern, count);
    Some(HeuristicHit {
        name,
        description,
        prompt_context,
        source: CaptureSource::RepeatedToolPattern {
            tools: tools_label,
            repeat_count: count,
        },
        user_message_excerpt: format!(
            "[no user message — tool-pattern capture from last {} invocations]",
            window.len()
        ),
        assistant_response_excerpt: None,
    })
}

// ── prompt body builders ─────────────────────────────────────────────

fn build_explicit_prompt_context(sentence: &str, trigger: &str) -> String {
    format!(
        "# User-taught rule\n\n\
         The user explicitly instructed (trigger phrase: `{trigger}`):\n\n\
         > {sentence}\n\n\
         Apply this rule whenever the situation it describes arises in future turns.\n"
    )
}

fn build_correction_prompt_context(prev: &str, correction: &str, trigger: &str) -> String {
    let prev_summary = one_line_summary(prev, 200);
    format!(
        "# User-corrected behaviour\n\n\
         Previously the agent did:\n\n\
         > {prev_summary}\n\n\
         The user corrected (trigger: `{trigger}`):\n\n\
         > {correction}\n\n\
         Prefer the corrected approach in future similar situations.\n"
    )
}

fn build_repeated_pattern_prompt_context(pattern: &[String], count: u32) -> String {
    let steps: String = pattern
        .iter()
        .enumerate()
        .map(|(i, tool)| format!("{}. `{}`\n", i + 1, tool))
        .collect();
    format!(
        "# Recurring tool sequence\n\n\
         The agent ran this sequence {count}× recently:\n\n\
         {steps}\n\
         Consider packaging it as a higher-level skill so future runs use a single invocation.\n"
    )
}

// ── small string helpers ────────────────────────────────────────────

/// Slice the sentence (`.!?\n`-bounded) that contains the byte range
/// `[start, end)` of `text`. Falls back to the whole string if no
/// terminator is found nearby.
fn sentence_around(text: &str, start: usize, end: usize) -> String {
    let preceding = &text[..start];
    let following = &text[end..];
    let lo = preceding
        .rfind(['.', '!', '?', '\n'])
        .map(|i| i + 1)
        .unwrap_or(0);
    let hi = following
        .find(['.', '!', '?', '\n'])
        .map(|i| end + i + 1)
        .unwrap_or(text.len());
    text[lo..hi].trim().to_string()
}

/// Build a short snake_case name from the captured sentence. `kind` is a
/// disambiguator suffix so two captures from the same conversation don't
/// collide with the same name.
fn synth_name(sentence: &str, kind: &str) -> String {
    let head: String = sentence
        .chars()
        .take(40)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase();
    let head = sanitise_name_segment(&head);
    if head.is_empty() {
        format!("captured_{kind}")
    } else {
        format!("{head}_{kind}")
    }
}

/// Restrict to ASCII alphanumerics + underscore so the result is a valid
/// `librefang_skills::evolution::validate_name` input.
fn sanitise_name_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_underscore = false;
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '_';
        if ok {
            out.push(c.to_ascii_lowercase());
            last_underscore = c == '_';
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

fn one_line_summary(s: &str, max_chars: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        let head: String = collapsed.chars().take(max_chars - 1).collect();
        format!("{head}…")
    }
}

fn count_non_overlapping(haystack: &[String], pattern: &[String]) -> u32 {
    if pattern.is_empty() || haystack.len() < pattern.len() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + pattern.len() <= haystack.len() {
        if haystack[i..i + pattern.len()] == *pattern {
            count += 1;
            i += pattern.len();
        } else {
            i += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ExplicitInstruction ───────────────────────────────────────

    #[test]
    fn explicit_from_now_on_matches() {
        let hit = extract_explicit_instruction("from now on always run cargo fmt before commit.")
            .expect("must match");
        assert!(matches!(
            hit.source,
            CaptureSource::ExplicitInstruction { ref trigger } if trigger.to_lowercase() == "from now on"
        ));
        assert!(hit.prompt_context.contains("cargo fmt"));
    }

    #[test]
    fn explicit_remember_to_matches() {
        let hit = extract_explicit_instruction("Please remember to lint the file before saving.")
            .expect("must match");
        assert!(matches!(
            hit.source,
            CaptureSource::ExplicitInstruction { .. }
        ));
    }

    #[test]
    fn explicit_inside_question_is_dropped() {
        // "from now on, should I always commit?" is an inquiry, not a directive.
        assert!(extract_explicit_instruction("from now on should I always commit?").is_none());
    }

    #[test]
    fn explicit_too_short_is_dropped() {
        assert!(extract_explicit_instruction("always.").is_none());
    }

    #[test]
    fn explicit_unrelated_text_returns_none() {
        assert!(extract_explicit_instruction("the weather is nice today").is_none());
    }

    #[test]
    fn explicit_name_is_snake_case_and_alnum() {
        let hit = extract_explicit_instruction("from now on always run cargo fmt before commit.")
            .unwrap();
        assert!(hit
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'));
        assert!(!hit.name.is_empty());
    }

    // ── UserCorrection ────────────────────────────────────────────

    #[test]
    fn correction_no_do_it_like_matches() {
        let hit = extract_user_correction(
            Some("I ran `git push --force` to overwrite the branch."),
            "no, do it like a regular push, never force.",
        )
        .expect("must match");
        assert!(matches!(hit.source, CaptureSource::UserCorrection { .. }));
        assert!(hit.prompt_context.contains("regular push"));
    }

    #[test]
    fn correction_not_x_but_y_matches() {
        let hit = extract_user_correction(
            Some("Compiled with `cargo build --release`."),
            "not build but check — `cargo check` is enough.",
        )
        .expect("must match");
        assert!(matches!(hit.source, CaptureSource::UserCorrection { .. }));
    }

    #[test]
    fn correction_without_prev_returns_none() {
        // A correction with no assistant context can't be turned into a useful skill.
        assert!(extract_user_correction(None, "no, do it like X.").is_none());
    }

    #[test]
    fn correction_unrelated_text_returns_none() {
        assert!(extract_user_correction(Some("Done."), "Thanks!").is_none());
    }

    // ── RepeatedToolPattern ───────────────────────────────────────

    #[test]
    fn repeated_three_step_sequence_matches() {
        let history: Vec<String> = ["read", "edit", "shell"]
            .iter()
            .cycle()
            .take(9)
            .map(|s| s.to_string())
            .collect();
        let hit = extract_repeated_tool_pattern(&history).expect("3× sequence must match");
        match hit.source {
            CaptureSource::RepeatedToolPattern {
                ref tools,
                repeat_count,
            } => {
                assert_eq!(tools, "read,edit,shell");
                assert_eq!(repeat_count, 3);
            }
            _ => panic!("wrong source variant"),
        }
    }

    #[test]
    fn repeated_short_history_returns_none() {
        let history: Vec<String> = vec!["read".into(), "edit".into()];
        assert!(extract_repeated_tool_pattern(&history).is_none());
    }

    #[test]
    fn repeated_only_twice_returns_none() {
        let history: Vec<String> = ["read", "edit"]
            .iter()
            .cycle()
            .take(4)
            .map(|s| s.to_string())
            .collect();
        assert!(extract_repeated_tool_pattern(&history).is_none());
    }

    #[test]
    fn repeated_single_tool_threshold_higher() {
        // "shell" three times is too noisy for a length-1 pattern; require ≥4.
        let history: Vec<String> = vec![
            "shell".into(),
            "shell".into(),
            "shell".into(),
            "read".into(),
        ];
        assert!(extract_repeated_tool_pattern(&history).is_none());
    }

    #[test]
    fn repeated_single_tool_four_times_matches() {
        let history: Vec<String> = ["shell", "shell", "shell", "shell"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let hit = extract_repeated_tool_pattern(&history).expect("4× single tool must match");
        match hit.source {
            CaptureSource::RepeatedToolPattern { ref tools, .. } => assert_eq!(tools, "shell"),
            _ => panic!("wrong source variant"),
        }
    }

    #[test]
    fn repeated_picks_longest_pattern() {
        // "a,b,a,b,a,b" — both length-1 ("a"×3) and length-2 ("a,b"×3) match;
        // pick length-2.
        let history: Vec<String> = vec![
            "a".into(),
            "b".into(),
            "a".into(),
            "b".into(),
            "a".into(),
            "b".into(),
        ];
        let hit = extract_repeated_tool_pattern(&history).expect("must match");
        match hit.source {
            CaptureSource::RepeatedToolPattern { tools, .. } => assert_eq!(tools, "a,b"),
            _ => panic!(),
        }
    }

    // ── helpers ──────────────────────────────────────────────────

    #[test]
    fn sentence_around_extracts_imperative() {
        let s = "Hi there. From now on always run cargo fmt. Got it?";
        let m = explicit_regexes()[0].find(s).unwrap();
        let sentence = sentence_around(s, m.start(), m.end());
        assert!(sentence.contains("cargo fmt"));
        assert!(!sentence.contains("Got it?"));
    }

    #[test]
    fn sanitise_name_segment_strips_punctuation() {
        assert_eq!(sanitise_name_segment("hello, world!"), "hello_world");
        assert_eq!(sanitise_name_segment("__foo__bar__"), "foo__bar");
    }
}
