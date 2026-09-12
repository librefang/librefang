//! Hierarchical goal types for the LibreFang Goals system.
//!
//! Goals represent high-level objectives that agents work toward.
//! They support parent-child hierarchies for organizing complex objectives
//! into smaller, trackable sub-goals.

use crate::agent::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// GoalId
// ---------------------------------------------------------------------------

/// Unique identifier for a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(pub Uuid);

impl GoalId {
    /// Generate a new random GoalId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for GoalId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for GoalId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

// ---------------------------------------------------------------------------
// GoalStatus
// ---------------------------------------------------------------------------

/// The current status of a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Not yet started.
    Pending,
    /// Currently being worked on.
    InProgress,
    /// Successfully completed.
    Completed,
    /// Cancelled or abandoned.
    Cancelled,
}

impl std::fmt::Display for GoalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalStatus::Pending => write!(f, "pending"),
            GoalStatus::InProgress => write!(f, "in_progress"),
            GoalStatus::Completed => write!(f, "completed"),
            GoalStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

// ---------------------------------------------------------------------------
// Goal
// ---------------------------------------------------------------------------

/// Maximum title length in characters.
const MAX_TITLE_LEN: usize = 256;

/// Maximum description length in characters.
const MAX_DESCRIPTION_LEN: usize = 4096;

/// A hierarchical goal that agents work toward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    /// Unique goal identifier.
    pub id: GoalId,
    /// Short title for the goal (max 256 chars).
    pub title: String,
    /// Longer description of the goal (max 4096 chars).
    #[serde(default)]
    pub description: String,
    /// Optional parent goal ID for hierarchy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<GoalId>,
    /// Current status of the goal.
    pub status: GoalStatus,
    /// Progress percentage (0-100).
    #[serde(default)]
    pub progress: u8,
    /// Optional agent assigned to this goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Enable loop engineering: a verifier gate on the agent's own claim of
    /// completion, an optional independent evaluator model, and capture of
    /// `GOAL_LEARNED:` markers into a skill.
    /// Default `false`, which reproduces the plain goal loop exactly — no
    /// extra LLM calls, no extra prompt sections.
    #[serde(default)]
    pub loop_engineering: bool,
    /// Agent that judges the generator's output after each iteration.
    /// Only consulted when `loop_engineering` is true.
    /// Its purpose is the one rule the pattern exists for: an agent does not
    /// grade its own work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_agent_id: Option<AgentId>,
    /// Model that judges whether the goal condition is met, independently of
    /// the agent's `GOAL_DONE` marker.
    /// Only consulted when `loop_engineering` is true.
    /// A small, cheap model is the intended choice — the judgement is a
    /// yes/no read of the goal against the latest output.
    /// When unset there is no evaluator and the agent's marker stands on its
    /// own, still subject to the verifier gate.
    ///
    /// The id is stored as written and **not** validated at save time, unlike
    /// [`Self::verify_agent_id`].
    /// A verifier id has a checkable shape (a UUID) and no correct value that
    /// fails the check; a model id has neither — whether one resolves depends
    /// on the provider configuration at the moment of the call, so a save-time
    /// 400 would reject a model the operator is about to configure and would
    /// still not guarantee the id resolves when the run reaches it.
    /// An unresolvable id therefore degrades rather than fails: the runner logs
    /// a `WARN` per iteration ("evaluator call failed; falling back to the
    /// agent's marker") and the run behaves as if no evaluator were set.
    /// Evaluator failures do not count toward the run's consecutive-failure
    /// circuit breaker, because the tick itself succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_model: Option<String>,
    /// When the goal was created.
    pub created_at: DateTime<Utc>,
    /// When the goal was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Goal {
    /// Validate this goal's fields.
    pub fn validate(&self) -> Result<(), String> {
        if self.title.is_empty() {
            return Err("title must not be empty".into());
        }
        if self.title.chars().count() > MAX_TITLE_LEN {
            return Err(format!(
                "title too long ({} chars, max {MAX_TITLE_LEN})",
                self.title.chars().count()
            ));
        }
        if self.description.chars().count() > MAX_DESCRIPTION_LEN {
            return Err(format!(
                "description too long ({} chars, max {MAX_DESCRIPTION_LEN})",
                self.description.chars().count()
            ));
        }
        if self.progress > 100 {
            return Err(format!("progress must be 0-100, got {}", self.progress));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shared storage location
// ---------------------------------------------------------------------------

/// Well-known shared-memory key under which all goals are persisted as a
/// single JSON array. Shared by the API CRUD routes and the kernel-side goal
/// runner so both read and write the same store.
pub const GOALS_STORAGE_KEY: &str = "__librefang_goals";

/// The reserved sentinel agent ID that owns the goals KV entry. Goals are a
/// global, cross-agent resource, so they live under a fixed ID rather than any
/// real agent's namespace.
pub fn goals_storage_agent_id() -> AgentId {
    AgentId(Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

// ---------------------------------------------------------------------------
// Argument parsing for `/goal`
// ---------------------------------------------------------------------------

/// The `/goal` flag that switches the run into loop-engineering mode.
pub const GOAL_LOOP_ENGINEERING_FLAG: &str = "--loop-engineering";

/// Split a raw `/goal` argument string into `(description, loop_engineering)`.
///
/// Returns `None` when the input is empty or whitespace-only, or when the
/// flag was the only thing supplied, which callers surface as a usage hint.
pub fn parse_goal_args(args: &str) -> Option<(String, bool)> {
    let loop_engineering = args.contains(GOAL_LOOP_ENGINEERING_FLAG);
    let description = if loop_engineering {
        args.replace(GOAL_LOOP_ENGINEERING_FLAG, " ")
    } else {
        args.to_string()
    };
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        None
    } else {
        Some((description, loop_engineering))
    }
}

// ---------------------------------------------------------------------------
// GoalRunState — long-horizon autonomous execution (#5744)
// ---------------------------------------------------------------------------

/// Lifecycle phase of an autonomous goal run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalRunPhase {
    /// The runner loop is active and driving the assigned agent.
    Running,
    /// The goal reached `Completed`/`Cancelled` (or 100% progress); loop ended.
    Finished,
    /// The iteration cap was hit before the goal completed.
    MaxIterationsReached,
    /// The loop stopped on the provider rate-limit circuit breaker.
    RateLimited,
    /// An operator stopped the run.
    Stopped,
}

impl std::fmt::Display for GoalRunPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalRunPhase::Running => write!(f, "running"),
            GoalRunPhase::Finished => write!(f, "finished"),
            GoalRunPhase::MaxIterationsReached => write!(f, "max_iterations_reached"),
            GoalRunPhase::RateLimited => write!(f, "rate_limited"),
            GoalRunPhase::Stopped => write!(f, "stopped"),
        }
    }
}

/// Default per-run iteration cap when a start request omits one. Bounds a
/// long-horizon run so a goal the agent never marks done cannot loop forever.
pub const DEFAULT_GOAL_MAX_ITERATIONS: u32 = 25;

/// Observable state of a goal's autonomous run, surfaced via the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalRunState {
    /// The goal being pursued.
    pub goal_id: GoalId,
    /// The agent driving the goal.
    pub agent_id: AgentId,
    /// Current lifecycle phase.
    pub phase: GoalRunPhase,
    /// Number of completed iterations (agent turns) so far.
    pub iteration: u32,
    /// Iteration cap for this run.
    pub max_iterations: u32,
    /// Last progress value (0-100) observed from the agent.
    pub last_progress: u8,
    /// Last error message, if the most recent tick failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Verifier agent for this run, copied from the goal at run start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_agent_id: Option<AgentId>,
    /// How many verification rounds one iteration gets before the run moves
    /// on without the verifier's blessing.
    /// `1` is a single verdict with no rework; each additional round is one
    /// more verifier turn plus one more generator turn.
    /// `0` means "unset"; the runner clamps it to at least 1.
    #[serde(default)]
    pub verify_max_retries: u32,
    /// Evaluator model for this run, copied from the goal at run start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_model: Option<String>,
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the most recent tick completed.
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_goal() -> Goal {
        Goal {
            id: GoalId::new(),
            title: "Ship v1.0".into(),
            description: "Release the first stable version".into(),
            parent_id: None,
            status: GoalStatus::Pending,
            progress: 0,
            agent_id: None,
            loop_engineering: false,
            verify_agent_id: None,
            evaluator_model: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn goal_id_display_roundtrip() {
        let id = GoalId::new();
        let s = id.to_string();
        let parsed: GoalId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn goal_id_default() {
        let a = GoalId::default();
        let b = GoalId::default();
        assert_ne!(a, b);
    }

    #[test]
    fn valid_goal_passes() {
        assert!(valid_goal().validate().is_ok());
    }

    #[test]
    fn empty_title_rejected() {
        let mut g = valid_goal();
        g.title = String::new();
        let err = g.validate().unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn long_title_rejected() {
        let mut g = valid_goal();
        g.title = "a".repeat(257);
        let err = g.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn long_description_rejected() {
        let mut g = valid_goal();
        g.description = "a".repeat(4097);
        let err = g.validate().unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn progress_over_100_rejected() {
        let mut g = valid_goal();
        g.progress = 101;
        let err = g.validate().unwrap_err();
        assert!(err.contains("0-100"), "{err}");
    }

    #[test]
    fn progress_100_ok() {
        let mut g = valid_goal();
        g.progress = 100;
        assert!(g.validate().is_ok());
    }

    #[test]
    fn serde_roundtrip() {
        let goal = valid_goal();
        let json = serde_json::to_string(&goal).unwrap();
        let back: Goal = serde_json::from_str(&json).unwrap();
        assert_eq!(back.title, goal.title);
        assert_eq!(back.id, goal.id);
    }

    #[test]
    fn serde_status_tags() {
        let json = serde_json::to_string(&GoalStatus::InProgress).unwrap();
        assert_eq!(json, "\"in_progress\"");

        let back: GoalStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, GoalStatus::InProgress);
    }

    #[test]
    fn goal_with_parent() {
        let parent_id = GoalId::new();
        let mut g = valid_goal();
        g.parent_id = Some(parent_id);
        let json = serde_json::to_string(&g).unwrap();
        assert!(json.contains("parent_id"));
        let back: Goal = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parent_id, Some(parent_id));
    }

    #[test]
    fn goal_without_parent_omits_field() {
        let g = valid_goal();
        let json = serde_json::to_string(&g).unwrap();
        assert!(!json.contains("parent_id"));
    }

    #[test]
    fn parse_goal_args_extracts_description_and_flag() {
        assert_eq!(
            parse_goal_args("ship the release"),
            Some(("ship the release".to_string(), false))
        );
        assert_eq!(
            parse_goal_args("ship the release --loop-engineering"),
            Some(("ship the release".to_string(), true))
        );
    }

    #[test]
    fn parse_goal_args_collapses_whitespace_around_the_flag() {
        assert_eq!(
            parse_goal_args("ship   --loop-engineering   the release"),
            Some(("ship the release".to_string(), true))
        );
    }

    #[test]
    fn parse_goal_args_rejects_empty_and_flag_only_input() {
        assert_eq!(parse_goal_args(""), None);
        assert_eq!(parse_goal_args("   "), None);
        assert_eq!(parse_goal_args("--loop-engineering"), None);
    }

    #[test]
    fn goal_document_written_before_loop_engineering_still_loads() {
        let json = serde_json::json!({
            "id": GoalId::new().to_string(),
            "title": "Written by an older daemon",
            "description": "",
            "status": "pending",
            "progress": 0,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
        });
        let back: Goal = serde_json::from_value(json).unwrap();
        assert!(!back.loop_engineering);
        assert_eq!(back.verify_agent_id, None);
        assert_eq!(back.evaluator_model, None);
    }

    #[test]
    fn loop_engineering_fields_round_trip_and_omit_when_unset() {
        let mut g = valid_goal();
        let json = serde_json::to_string(&g).unwrap();
        assert!(!json.contains("verify_agent_id"));
        assert!(!json.contains("evaluator_model"));

        let verifier = AgentId::new();
        g.loop_engineering = true;
        g.verify_agent_id = Some(verifier);
        g.evaluator_model = Some("haiku".into());
        let back: Goal = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert!(back.loop_engineering);
        assert_eq!(back.verify_agent_id, Some(verifier));
        assert_eq!(back.evaluator_model.as_deref(), Some("haiku"));
    }
}
