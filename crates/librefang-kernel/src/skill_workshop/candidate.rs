//! Skill workshop candidate types (#3328).
//!
//! A `CandidateSkill` is a draft produced by the after-turn capture
//! pipeline. It carries enough provenance for a human reviewer to decide
//! whether to promote, edit, or drop it, and serialises to a single TOML
//! file under `~/.librefang/skills/pending/<agent_id>/<id>.toml`.
//!
//! Candidates are NOT loaded into the active skill registry. Promotion
//! happens via `storage::approve_candidate`, which routes through
//! `librefang_skills::evolution::create_skill` so the same security
//! pipeline that gates marketplace skills also gates approved
//! candidates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A draft skill captured from a successful interaction.
///
/// Stored on disk as TOML. The `prompt_context` field becomes the body of
/// the resulting `prompt_context.md` if the candidate is approved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSkill {
    /// UUID v4 used as the on-disk filename and the CLI / dashboard id.
    pub id: String,
    /// Agent that produced the candidate. Pending candidates are scoped
    /// per-agent so that approving one doesn't accidentally hand a
    /// workflow to a different agent.
    pub agent_id: String,
    /// Session this turn belonged to, if known. Best-effort metadata —
    /// missing on manifest-driven test fixtures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// When the heuristic / LLM classifier accepted the candidate.
    pub captured_at: DateTime<Utc>,
    /// What signal triggered the capture.
    pub source: CaptureSource,
    /// Suggested skill name (snake_case-ish, validated at approval time
    /// by `librefang_skills::evolution::validate_name`).
    pub name: String,
    /// One-line skill description (≤1024 chars, enforced by
    /// `evolution::create_skill` at promotion time).
    pub description: String,
    /// Body of the future `prompt_context.md`. Free-form Markdown.
    pub prompt_context: String,
    /// Trace back to the conversation turn that produced this candidate.
    pub provenance: Provenance,
}

/// What signal in the turn led the workshop to produce this candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureSource {
    /// User said something like "from now on, always X" — the
    /// workshop pulled the imperative directly from the message.
    ExplicitInstruction {
        /// The trigger phrase that matched, kept for log / audit.
        trigger: String,
    },
    /// User corrected the agent's previous turn ("no, do it like Y").
    /// The captured workflow describes the corrected approach.
    UserCorrection {
        /// The correction phrase that matched.
        trigger: String,
    },
    /// The agent ran the same tool sequence three or more times across
    /// recent turns; the workshop suggests packaging it as a skill so
    /// future runs use a single invocation.
    RepeatedToolPattern {
        /// Comma-joined tool names that formed the repeating sequence.
        tools: String,
        /// How many times the sequence was observed.
        repeat_count: u32,
    },
}

/// Conversation context the candidate was extracted from.
///
/// Excerpts are truncated to keep the on-disk TOML small and to avoid
/// pasting full secrets / large pastes into a long-lived file. The
/// approval CLI surfaces these excerpts so a reviewer can decide
/// whether the candidate matches their intent without spelunking
/// the session log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// The user message that triggered the capture, truncated.
    pub user_message_excerpt: String,
    /// The assistant's most recent response, truncated. `None` for
    /// `RepeatedToolPattern` captures, which are not tied to a single
    /// turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_response_excerpt: Option<String>,
    /// Sequential turn number within the session (1-based). Matches
    /// what `librefang skill pending show` displays.
    pub turn_index: u32,
}

/// Maximum characters retained in a provenance excerpt. Keeps pending
/// files small and bounds the worst case if a reviewer eyeballs the
/// raw TOML.
pub const PROVENANCE_EXCERPT_MAX_CHARS: usize = 800;

/// Truncate a string to at most [`PROVENANCE_EXCERPT_MAX_CHARS`]
/// characters, appending an ellipsis marker so it is obvious the value
/// was clipped. Operates on chars, not bytes, so multibyte characters
/// are not split.
pub fn truncate_excerpt(s: &str) -> String {
    if s.chars().count() <= PROVENANCE_EXCERPT_MAX_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(PROVENANCE_EXCERPT_MAX_CHARS).collect();
    format!("{head}… [truncated]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_excerpt_short_passes_through() {
        assert_eq!(truncate_excerpt("hello"), "hello");
    }

    #[test]
    fn truncate_excerpt_long_clips_with_marker() {
        let long: String = "x".repeat(PROVENANCE_EXCERPT_MAX_CHARS + 50);
        let out = truncate_excerpt(&long);
        assert!(out.ends_with("… [truncated]"));
        // Head plus the marker; head is exactly PROVENANCE_EXCERPT_MAX_CHARS chars.
        let head_chars = out.chars().count() - "… [truncated]".chars().count();
        assert_eq!(head_chars, PROVENANCE_EXCERPT_MAX_CHARS);
    }

    #[test]
    fn truncate_excerpt_does_not_split_multibyte_chars() {
        // Each emoji is multi-byte; ensure the boundary lands on a char.
        let s: String = "🐯".repeat(PROVENANCE_EXCERPT_MAX_CHARS + 5);
        let out = truncate_excerpt(&s);
        // No panic on .chars() round-trip means the boundary was clean.
        assert!(out.starts_with("🐯"));
    }

    #[test]
    fn candidate_round_trips_through_toml() {
        let candidate = CandidateSkill {
            id: "00000000-0000-0000-0000-000000000001".to_string(),
            agent_id: "agent-x".to_string(),
            session_id: Some("session-y".to_string()),
            captured_at: Utc::now(),
            source: CaptureSource::ExplicitInstruction {
                trigger: "from now on".to_string(),
            },
            name: "cargo_fmt_before_commit".to_string(),
            description: "Always run cargo fmt before commit".to_string(),
            prompt_context: "# Cargo fmt before commit\n\nRun `cargo fmt --all` before staging.\n"
                .to_string(),
            provenance: Provenance {
                user_message_excerpt: "from now on always run cargo fmt before commit".to_string(),
                assistant_response_excerpt: Some("Got it.".to_string()),
                turn_index: 3,
            },
        };
        let toml = toml::to_string_pretty(&candidate).expect("serialise");
        let parsed: CandidateSkill = toml::from_str(&toml).expect("deserialise");
        assert_eq!(parsed.id, candidate.id);
        assert_eq!(parsed.name, candidate.name);
        assert_eq!(parsed.source, candidate.source);
    }

    #[test]
    fn capture_source_serialises_with_tag_kind() {
        let src = CaptureSource::RepeatedToolPattern {
            tools: "shell,write_file".to_string(),
            repeat_count: 3,
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["kind"], "repeated_tool_pattern");
        assert_eq!(json["tools"], "shell,write_file");
        assert_eq!(json["repeat_count"], 3);
    }
}
