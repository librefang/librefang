//! Event-driven agent triggers — agents auto-activate when events match patterns.
//!
//! Agents register triggers that describe which events should wake them.
//! When a matching event arrives on the EventBus, the trigger system
//! sends the event content as a message to the subscribing agent.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use librefang_types::agent::AgentId;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::event::{Event, EventPayload, LifecycleEvent, SystemEvent};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Default cooldown duration after a trigger fires (in seconds).
const DEFAULT_COOLDOWN_SECS: u64 = 5;

/// Maximum byte length of a `workflow_id` string on a trigger.
/// Mirrors the same limit used for cron `CronAction::Workflow`.
pub const MAX_WORKFLOW_ID_LEN: usize = 256;

/// Default maximum number of triggers that can fire from a single event.
const DEFAULT_MAX_TRIGGERS_PER_EVENT: usize = 10;

/// Error returned by [`TriggerEngine::register_with_target_enabled`]
/// (and the convenience wrappers) when registering a new trigger
/// would push the owning agent past [`MAX_TRIGGERS_PER_AGENT`]. The
/// audit explicitly forbids silent truncation: operators must see
/// when their `agent.toml` is over the cap, so this error carries
/// the three fields needed to log it actionably.
#[derive(Debug, Clone, thiserror::Error)]
#[error(
    "agent {agent_id} already has {current_count} triggers (cap is {max}); refusing to register more"
)]
pub struct TriggerCapExceeded {
    pub agent_id: AgentId,
    pub current_count: usize,
    pub max: usize,
}

/// Hard cap on the number of triggers a single agent can hold in the
/// runtime store.
///
/// Pre-cap, `register_with_target_enabled` unconditionally pushed onto
/// `agent_triggers[agent_id]`, and `reconcile_manifest_triggers` walked
/// the manifest's `triggers: Vec<ManifestTrigger>` creating one runtime
/// trigger per undeclared entry. A malicious or buggy `agent.toml`
/// declaring 100k triggers would load them all, blowing up
/// `triggers.json` storage and turning per-event match scanning (which
/// is O(N) over the agent's triggers) into a DoS on every fire. The
/// `DEFAULT_MAX_TRIGGERS_PER_EVENT` cap only bounded the *fire* set,
/// not the scanned set.
///
/// 50 is the same per-agent ceiling cron already enforces
/// (`librefang-types/src/scheduler.rs::MAX_JOBS_PER_AGENT`); aligning
/// the two means an operator's mental model for "how much an agent
/// can ask for" is the same across both schedulers. (audit:
/// trigger-engine-no-per-agent-cap)
pub const MAX_TRIGGERS_PER_AGENT: usize = 50;

// Re-export defaults so tests can use TriggerEngine::new() without config.
// The constants above are kept as fallbacks; production code threads values
// from TriggersConfig via `TriggerEngine::with_config`.

/// Unique identifier for a trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TriggerId(pub Uuid);

impl TriggerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TriggerId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TriggerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What kind of events a trigger matches on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPattern {
    /// Match any lifecycle event (agent spawned, started, terminated, etc.).
    Lifecycle,
    /// Match when a specific agent is spawned.
    AgentSpawned { name_pattern: String },
    /// Match when any agent is terminated.
    AgentTerminated,
    /// Match any system event.
    System,
    /// Match a specific system event by keyword.
    SystemKeyword { keyword: String },
    /// Match any memory update event.
    MemoryUpdate,
    /// Match memory updates for a specific key pattern.
    MemoryKeyPattern { key_pattern: String },
    /// Match all events (wildcard).
    All,
    /// Match custom events by content substring.
    ContentMatch { substring: String },
    /// Match when a task is posted to the Task Board.
    ///
    /// `assignee_match` narrows the match to tasks assigned to a specific
    /// agent:
    /// - `Some("self")` — only fire for tasks assigned to the trigger-owning agent.
    ///   Accepts both the agent's UUID and its display name.
    /// - `Some("unassigned")` — only fire for tasks no agent owns.
    ///   Matches both an absent `assigned_to` and the empty string, because the stuck-task sweeper releases a claim by writing `assigned_to = ''` rather than NULL.
    /// - `Some("<uuid>"|"<name>")` — only fire for tasks assigned to that specific agent.
    ///   `"self"` and `"unassigned"` are keywords, so an agent named either must be addressed by UUID.
    /// - `None` — fire for every `TaskPosted` event (legacy behavior).
    ///
    /// The field is `#[serde(default)]` so legacy triggers persisted or
    /// transmitted as the bare JSON string `"task_posted"` still parse via
    /// the `preprocess_pattern_json` helper (see API route).
    TaskPosted {
        #[serde(default)]
        assignee_match: Option<String>,
    },
    /// Match when a task is claimed from the Task Board.
    ///
    /// `creator_match` narrows the match to tasks originally posted by a
    /// specific agent (mirror of `TaskPosted`'s `assignee_match`):
    /// - `Some("self")` — only fire for tasks posted by the trigger-owning agent.
    ///   Accepts both the agent's UUID and its display name.
    /// - `Some("unassigned")` — accepted because the identity filter is shared with `assignee_match`, and here means "no recorded creator" (absent or empty `created_by`).
    ///   Unlike `assigned_to`, nothing in the system currently writes an empty `created_by`, so this is a consequence of the shared helper rather than a designed filter — treat it as reserved.
    /// - `Some("<uuid>"|"<name>")` — only fire for tasks posted by that specific agent.
    ///   `"self"` and `"unassigned"` are keywords.
    /// - `None` — fire for every `TaskClaimed` event (legacy behavior).
    ///
    /// The field is `#[serde(default)]` so legacy triggers persisted or
    /// transmitted as the bare JSON string `"task_claimed"` still parse via
    /// the `normalize_pattern_json` helper (see API route).
    TaskClaimed {
        #[serde(default)]
        creator_match: Option<String>,
    },
    /// Match when a task is completed on the Task Board.
    ///
    /// `creator_match` narrows the match to tasks originally posted by a
    /// specific agent (mirror of `TaskPosted`'s `assignee_match`):
    /// - `Some("self")` — only fire for tasks posted by the trigger-owning agent.
    ///   Accepts both the agent's UUID and its display name.
    /// - `Some("unassigned")` — accepted because the identity filter is shared with `assignee_match`, and here means "no recorded creator" (absent or empty `created_by`).
    ///   Unlike `assigned_to`, nothing in the system currently writes an empty `created_by`, so this is a consequence of the shared helper rather than a designed filter — treat it as reserved.
    /// - `Some("<uuid>"|"<name>")` — only fire for tasks posted by that specific agent.
    ///   `"self"` and `"unassigned"` are keywords.
    /// - `None` — fire for every `TaskCompleted` event (legacy behavior).
    ///
    /// The field is `#[serde(default)]` so legacy triggers persisted or
    /// transmitted as the bare JSON string `"task_completed"` still parse via
    /// the `normalize_pattern_json` helper (see API route).
    TaskCompleted {
        #[serde(default)]
        creator_match: Option<String>,
    },
}

/// A registered trigger definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    /// Unique trigger ID.
    pub id: TriggerId,
    /// Which agent owns this trigger.
    pub agent_id: AgentId,
    /// The event pattern to match.
    pub pattern: TriggerPattern,
    /// Prompt template to send when triggered. Use `{{event}}` for event description.
    pub prompt_template: String,
    /// Whether this trigger is currently active.
    pub enabled: bool,
    /// When this trigger was created.
    pub created_at: DateTime<Utc>,
    /// How many times this trigger has fired.
    pub fire_count: u64,
    /// Maximum number of times this trigger can fire (0 = unlimited).
    pub max_fires: u64,
    /// If set, route the triggered message to this agent instead of the owner.
    /// Enables cross-session wake: one agent's trigger can wake a different agent.
    #[serde(default)]
    pub target_agent: Option<AgentId>,
    /// Cooldown duration in seconds after this trigger fires before the same window may fire again.
    ///
    /// For patterns that name a subject — the task-board, memory-key and agent-lifecycle kinds, see `cooldown_subject` — the window is `(trigger, subject)`, so the trigger can fire again immediately for a *different* subject and this bound applies per subject rather than per trigger (#6756).
    /// Every other pattern keeps a single trigger-wide window.
    ///
    /// Worth pairing carefully with `max_fires`: a subject-scoped trigger fires once per subject, so a burst of distinct subjects consumes the fire budget as fast as they arrive rather than at one per window.
    ///
    /// `None` means use the default cooldown (`DEFAULT_COOLDOWN_SECS`).
    /// Set to `Some(0)` to disable cooldown for this trigger.
    #[serde(default)]
    pub cooldown_secs: Option<u64>,
    /// Per-trigger session mode override.
    /// `None` inherits from the target agent's manifest `session_mode`.
    /// `Some(mode)` overrides for this specific trigger.
    #[serde(default)]
    pub session_mode: Option<librefang_types::agent::SessionMode>,
    /// Wall-clock timestamp of the last time this trigger fired.
    ///
    /// Persisted to disk so that cooldown state survives daemon restarts.
    /// `None` means the trigger has never fired (or the field was not present
    /// in an older persisted file — `#[serde(default)]` handles both cases).
    #[serde(default)]
    pub last_fired_at: Option<DateTime<Utc>>,
    /// If set, the trigger fires a workflow run (identified by this string,
    /// resolved as a UUID first, then by name) instead of sending a prompt
    /// to an agent via `send_message_full`.
    ///
    /// `prompt_template` is still rendered (with `{{event}}` substituted) and
    /// used as the workflow's initial input string.
    ///
    /// `target_agent` and `workflow_id` may coexist — `target_agent` is used
    /// for agent-path routing only and is ignored when `workflow_id` is set.
    ///
    /// `#[serde(default)]` ensures old persisted triggers (without this field)
    /// deserialise cleanly as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
}

/// Whether a stored trigger already delivers `TaskPosted` events to a given assignee — the precedence input for the built-in assignee wake.
///
/// The three states are distinct on purpose: `Dormant` is a wake that an operator once configured and that can no longer fire, which is worth saying out loud when the built-in path takes over from it, while `None` is an installation that never declared one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskPostedCoverage {
    /// A stored trigger can currently fire for this assignee.
    /// The built-in wake stands down and the operator's trigger owns delivery.
    Covered(TriggerId),
    /// Triggers address this assignee but none can fire — each is disabled or has exhausted `max_fires`.
    /// The built-in wake fires and names these, since a dead record is a gap, not a decision to stay silent.
    Dormant(Vec<TriggerId>),
    /// No stored trigger addresses this assignee at all.
    None,
}

/// What produced a [`TriggerMatch`].
///
/// Dispatch is uniform across the variants — the dispatcher consumes one list — but the two differ in what they can be keyed on for diagnostics and for `SessionMode::New` session derivation, and a log line that says only "trigger fired" cannot answer "which trigger?" for a match that has no trigger behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerMatchSource {
    /// A stored trigger record matched the event.
    Registered(TriggerId),
    /// The kernel synthesized this match for the assignee of a `TaskPosted` event that no stored trigger currently covers (issue #6728).
    /// Carries the task id so the wake is traceable to the task that caused it, and so `SessionId::for_task_wake` has a stable key.
    TaskBoardAssigneeWake { task_id: String },
}

impl std::fmt::Display for TriggerMatchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registered(id) => write!(f, "trigger:{id}"),
            Self::TaskBoardAssigneeWake { task_id } => write!(f, "assignee_wake:{task_id}"),
        }
    }
}

/// A trigger match result with optional session mode override.
#[derive(Debug, Clone)]
pub struct TriggerMatch {
    /// The agent to dispatch the triggered message to.
    pub agent_id: AgentId,
    /// The rendered message to send.
    pub message: String,
    /// Per-trigger session mode override (None = inherit from agent manifest).
    pub session_mode_override: Option<librefang_types::agent::SessionMode>,
    /// If set, dispatch fires a workflow run instead of `send_message_full`.
    pub workflow_id: Option<String>,
    /// What produced this match, for telemetry and session derivation.
    pub source: TriggerMatchSource,
}

/// Patch payload for updating an existing trigger.
///
/// All fields are optional — `None` means "leave unchanged".
/// `cooldown_secs` and `session_mode` use `Option<Option<T>>` so callers can
/// explicitly clear a value by passing `Some(None)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriggerPatch {
    pub pattern: Option<TriggerPattern>,
    pub prompt_template: Option<String>,
    pub enabled: Option<bool>,
    pub max_fires: Option<u64>,
    /// `Some(None)` clears the override (reverts to engine default).
    pub cooldown_secs: Option<Option<u64>>,
    /// `Some(None)` clears the override (inherits from agent manifest).
    pub session_mode: Option<Option<librefang_types::agent::SessionMode>>,
    /// `Some(None)` clears the target (reverts to owner routing).
    /// `Some(Some(id))` sets a new cross-session wake target.
    pub target_agent: Option<Option<AgentId>>,
    /// `Some(None)` clears the workflow_id (reverts to agent dispatch).
    /// `Some(Some(s))` sets a new workflow target.
    pub workflow_id: Option<Option<String>>,
}

/// The trigger engine manages event-to-agent routing.
pub struct TriggerEngine {
    /// All registered triggers.
    triggers: DashMap<TriggerId, Trigger>,
    /// Index: agent_id → list of trigger IDs belonging to that agent.
    agent_triggers: DashMap<AgentId, Vec<TriggerId>>,
    /// Last fire wall-clock timestamp per cooldown window (issue #6756).
    ///
    /// Keyed on `(trigger, subject)` rather than on the trigger alone, where the subject is what the event is *about* — a task id, a memory key — for the patterns that name one.
    /// A window keyed on the trigger cannot tell "the same thing fired twice" from "two different things happened a second apart", and silently discarded the second, which for a task board means the work is never announced again.
    ///
    /// Entries with `None` are the trigger-wide window used by patterns that identify no subject (`All`, `System`, `ContentMatch`, …), so a catch-all keeps exactly the rate cap it has today.
    ///
    /// Uses `DateTime<Utc>` rather than `std::time::Instant` so the trigger-wide entry can be round-tripped through `Trigger.last_fired_at` on disk, surviving daemon restarts without resetting all cooldown windows.
    /// Per-subject entries stay in memory: a restart then costs at most one extra fire per subject, which is the safe direction, and it keeps the persisted field meaning what it says.
    last_fired: DashMap<(TriggerId, Option<String>), DateTime<Utc>>,
    /// Maximum number of triggers that can fire from a single event.
    max_triggers_per_event: usize,
    /// Default cooldown duration (seconds) applied when a trigger has no override.
    default_cooldown_secs: u64,
    /// Path to the persistence file (`<home>/trigger_jobs.json`).
    /// `None` means no persistence (used in tests).
    persist_path: Option<PathBuf>,
    /// Serializes `persist()` writes so concurrent callers (event
    /// dispatch, API routes, restart handlers) within a single process
    /// don't `O_TRUNC` the same `.tmp.{pid}` path and produce a torn
    /// file before rename.  Mirrors `CronScheduler::persist_lock`.
    persist_lock: std::sync::Mutex<()>,
}

fn lock_trigger_persistence(lock: &std::sync::Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    lock.lock().unwrap_or_else(|poisoned| {
        warn!("trigger persistence lock poisoned; recovering write serialization");
        lock.clear_poison();
        poisoned.into_inner()
    })
}

impl TriggerEngine {
    /// Create a new trigger engine with default settings and no persistence.
    pub fn new() -> Self {
        Self {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            last_fired: DashMap::new(),
            max_triggers_per_event: DEFAULT_MAX_TRIGGERS_PER_EVENT,
            default_cooldown_secs: DEFAULT_COOLDOWN_SECS,
            persist_path: None,
            persist_lock: std::sync::Mutex::new(()),
        }
    }

    /// Create a trigger engine configured from a `TriggersConfig`, with persistence.
    ///
    /// `home_dir` is the LibreFang data directory; triggers are persisted to
    /// `<home_dir>/trigger_jobs.json`.
    pub fn with_config(config: &librefang_types::config::TriggersConfig, home_dir: &Path) -> Self {
        Self {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            last_fired: DashMap::new(),
            max_triggers_per_event: config.max_per_event.max(1),
            default_cooldown_secs: config.cooldown_secs,
            persist_path: Some(home_dir.join("trigger_jobs.json")),
            persist_lock: std::sync::Mutex::new(()),
        }
    }

    /// Create a new trigger engine with a custom per-event trigger budget.
    ///
    /// `max` is clamped to a minimum of 1; passing 0 would cause the budget
    /// check (`matches.len() >= max`) to be true immediately, preventing any
    /// trigger from ever firing.
    pub fn with_max_triggers_per_event(max: usize) -> Self {
        Self {
            max_triggers_per_event: max.max(1),
            ..Self::new()
        }
    }

    // -- Persistence ----------------------------------------------------------

    /// Load persisted triggers from disk and rebuild the agent index.
    ///
    /// Restores `last_fired` state from `Trigger.last_fired_at` so that the trigger-wide cooldown window survives daemon restarts.
    ///
    /// This covers the trigger-wide window only (#6756).
    /// Per-subject windows — the task-board, memory-key and agent-lifecycle pattern kinds, see `cooldown_subject` — are in-memory and do not survive a restart: such a trigger comes back with no suppression for any subject, including one it fired on moments earlier.
    /// The trade is deliberate, since persisting a window per subject would grow the file without bound and the failure direction is an extra delivery rather than a lost one.
    ///
    /// Returns the number of triggers loaded. Returns `Ok(0)` if the
    /// persistence file does not exist or no path is configured.
    pub fn load(&self) -> LibreFangResult<usize> {
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(0),
        };
        if !path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(path)
            .map_err(|e| LibreFangError::Internal(format!("Failed to read trigger jobs: {e}")))?;
        let mut raw: Vec<serde_json::Value> = serde_json::from_str(&data)
            .map_err(|e| LibreFangError::Internal(format!("Failed to parse trigger jobs: {e}")))?;
        // Migrate legacy unit-variant patterns to struct form so old persisted
        // files survive enum additions. Currently covers `"task_posted"` which
        // gained `assignee_match` (the only struct variant with optional fields).
        for entry in &mut raw {
            if let Some(pattern) = entry.get_mut("pattern") {
                if matches!(pattern.as_str(), Some("task_posted")) {
                    *pattern = serde_json::json!({ "task_posted": {} });
                }
            }
        }
        let triggers: Vec<Trigger> = raw
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .map_err(|e| LibreFangError::Internal(format!("Failed to parse trigger jobs: {e}")))?;
        let count = triggers.len();
        for trigger in triggers {
            let id = trigger.id;
            let agent_id = trigger.agent_id;
            // Restore cooldown state from the persisted last_fired_at timestamp, so a trigger that fired shortly before a restart still honours its window afterwards.
            //
            // This covers the trigger-wide window only.
            // Per-subject windows (#6756) are in-memory, so a subject-scoped pattern starts a restart with no suppression at all — including for a subject it fired on moments earlier.
            // The trade is deliberate: persisting one entry per subject would grow `trigger_jobs.json` without bound, and the failure direction here is an extra delivery rather than a lost one.
            // `subject_scoped_cooldown_does_not_survive_restart` pins the behaviour so the cost stays visible.
            if let Some(last_fired_at) = trigger.last_fired_at {
                self.last_fired.insert((id, None), last_fired_at);
            }
            self.triggers.insert(id, trigger);
            // Guard against duplicate IDs in a corrupted file: only add to the
            // per-agent index if this ID isn't already present.
            let mut ids = self.agent_triggers.entry(agent_id).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        info!(count, "Loaded trigger jobs from disk");
        Ok(count)
    }

    /// Persist all triggers to disk via atomic write (write to `.tmp`, then rename).
    ///
    /// Snapshots the current `last_fired` timestamp into each trigger's
    /// `last_fired_at` field before serializing so that cooldown state is
    /// restored correctly on next load.
    ///
    /// Does nothing when no persistence path is configured.
    pub fn persist(&self) -> LibreFangResult<()> {
        let _guard = lock_trigger_persistence(&self.persist_lock);
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(()),
        };
        // Clone triggers and stamp current last_fired timestamps into them so
        // that cooldown state is preserved across restarts.
        let triggers: Vec<Trigger> = self
            .triggers
            .iter()
            .map(|e| {
                let mut t = e.value().clone();
                if let Some(ts) = self.last_fired.get(&(t.id, None)) {
                    t.last_fired_at = Some(*ts);
                }
                t
            })
            .collect();
        let data = serde_json::to_string_pretty(&triggers).map_err(|e| {
            LibreFangError::Internal(format!("Failed to serialize trigger jobs: {e}"))
        })?;
        let tmp_path = crate::persist_tmp_path(path);
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp_path).map_err(|e| {
                LibreFangError::Internal(format!("Failed to create trigger jobs temp file: {e}"))
            })?;
            f.write_all(data.as_bytes()).map_err(|e| {
                LibreFangError::Internal(format!("Failed to write trigger jobs temp file: {e}"))
            })?;
            f.sync_all().map_err(|e| {
                LibreFangError::Internal(format!("Failed to fsync trigger jobs temp file: {e}"))
            })?;
        }
        std::fs::rename(&tmp_path, path).map_err(|e| {
            LibreFangError::Internal(format!("Failed to rename trigger jobs file: {e}"))
        })?;
        debug!(count = triggers.len(), "Persisted trigger jobs");
        Ok(())
    }

    /// Returns `true` if `agent_id` owns a trigger with this exact pattern,
    /// **whatever its `enabled` state**. Used to skip duplicate registration
    /// of proactive triggers on restart.
    ///
    /// Ignoring `enabled` is deliberate, and differs from the coverage rule used by the Task Board assignee wake ([`Self::task_posted_coverage_for`]), which treats a disabled record as no coverage.
    /// The two questions are not the same one:
    ///
    /// - Proactive triggers are auto-registered from `ScheduleMode::Proactive` conditions with `max_fires = 0` (`kernel/spawn.rs`, `kernel/background_lifecycle.rs`), so they can never become disabled by exhausting a budget — `enabled = false` there means exactly one thing, that an operator turned the trigger off.
    ///   Disabling it is also the *only* off switch: the condition would otherwise be re-registered on the next spawn.
    ///   Skipping on pattern alone is what makes that switch stick.
    /// - The assignee wake has an explicit off switch of its own (`[task_board] assignee_wake`, or the per-agent manifest override), so it can afford to treat a dead record as a gap to fill rather than as a decision to stay silent.
    pub fn agent_has_pattern(&self, agent_id: AgentId, pattern: &TriggerPattern) -> bool {
        let Some(ids) = self.agent_triggers.get(&agent_id) else {
            return false;
        };
        ids.iter().any(|id| {
            self.triggers
                .get(id)
                .map(|t| &t.pattern == pattern)
                .unwrap_or(false)
        })
    }

    /// Register a new trigger.
    pub fn register(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
    ) -> Result<TriggerId, TriggerCapExceeded> {
        self.register_with_target(
            agent_id,
            pattern,
            prompt_template,
            max_fires,
            None,
            None,
            None,
            None,
        )
    }

    /// Register a trigger with an optional target agent for cross-session wake.
    ///
    /// When `target_agent` is `Some`, the triggered message is routed to that
    /// agent instead of the owner (`agent_id`). The owner still "owns" the
    /// trigger for management purposes (list, remove, etc.).
    ///
    /// When `workflow_id` is `Some`, a matching event fires a workflow run
    /// instead of `send_message_full`. `prompt_template` is still rendered
    /// and used as the workflow's initial input string.
    #[allow(clippy::too_many_arguments)]
    pub fn register_with_target(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
        target_agent: Option<AgentId>,
        cooldown_secs: Option<u64>,
        session_mode: Option<librefang_types::agent::SessionMode>,
        workflow_id: Option<String>,
    ) -> Result<TriggerId, TriggerCapExceeded> {
        self.register_with_target_enabled(
            agent_id,
            pattern,
            prompt_template,
            max_fires,
            target_agent,
            cooldown_secs,
            session_mode,
            workflow_id,
            true,
        )
    }

    /// Like [`register_with_target`], but sets the `enabled` flag at
    /// construction so callers that want a disabled trigger do not have
    /// to follow up with [`set_enabled`].
    ///
    /// The follow-up form was racy: the event bus could observe the new
    /// trigger between `register_with_target` (enabled=true) and a
    /// subsequent `set_enabled(false)` call and fire it once before the
    /// mute landed. Reconcile of manifest entries with `enabled = false`
    /// goes through this constructor so the registration is a single
    /// locked operation.
    #[allow(clippy::too_many_arguments)]
    pub fn register_with_target_enabled(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
        target_agent: Option<AgentId>,
        cooldown_secs: Option<u64>,
        session_mode: Option<librefang_types::agent::SessionMode>,
        workflow_id: Option<String>,
        enabled: bool,
    ) -> Result<TriggerId, TriggerCapExceeded> {
        // Per-agent cap (audit: trigger-engine-no-per-agent-cap).
        // Hold the entry's write guard across the check + push so two
        // concurrent registers can't both observe `current == 49` and
        // both succeed. The audit explicitly forbids silent
        // truncation — return the structured error so callers (route
        // handler, `reconcile_manifest_triggers`) can surface "your
        // agent.toml has too many triggers" to the operator instead
        // of dropping bytes on the floor.
        let mut bucket = self.agent_triggers.entry(agent_id).or_default();
        if bucket.len() >= MAX_TRIGGERS_PER_AGENT {
            let err = TriggerCapExceeded {
                agent_id,
                current_count: bucket.len(),
                max: MAX_TRIGGERS_PER_AGENT,
            };
            warn!(
                agent_id = %agent_id,
                current = bucket.len(),
                max = MAX_TRIGGERS_PER_AGENT,
                "Trigger registration refused — per-agent cap exceeded",
            );
            return Err(err);
        }

        let trigger = Trigger {
            id: TriggerId::new(),
            agent_id,
            pattern,
            prompt_template,
            enabled,
            created_at: Utc::now(),
            fire_count: 0,
            max_fires,
            target_agent,
            cooldown_secs,
            session_mode,
            last_fired_at: None,
            workflow_id,
        };
        let id = trigger.id;
        self.triggers.insert(id, trigger);
        bucket.push(id);

        info!(trigger_id = %id, agent_id = %agent_id, ?target_agent, enabled, "Trigger registered");
        Ok(id)
    }

    /// Convenience: register a cross-agent trigger where the owner's trigger
    /// wakes a different target agent.
    pub fn register_cross_agent_trigger(
        &self,
        owner: AgentId,
        target: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
    ) -> Result<TriggerId, TriggerCapExceeded> {
        self.register_with_target(
            owner,
            pattern,
            prompt_template,
            0,
            Some(target),
            None,
            None,
            None,
        )
    }

    /// Remove a trigger.
    pub fn remove(&self, trigger_id: TriggerId) -> bool {
        if let Some((_, trigger)) = self.triggers.remove(&trigger_id) {
            if let Some(mut list) = self.agent_triggers.get_mut(&trigger.agent_id) {
                list.retain(|id| *id != trigger_id);
            }
            self.forget_cooldowns(trigger_id);
            true
        } else {
            false
        }
    }

    /// Remove all triggers for an agent.
    pub fn remove_agent_triggers(&self, agent_id: AgentId) {
        if let Some((_, trigger_ids)) = self.agent_triggers.remove(&agent_id) {
            for id in trigger_ids {
                self.triggers.remove(&id);
                self.forget_cooldowns(id);
            }
        }
    }

    /// Take all triggers for an agent, removing them from the engine.
    ///
    /// Returns the extracted triggers so they can be restored under a
    /// different agent ID via [`restore_triggers`]. This is used during
    /// hand reactivation: triggers must be saved before `kill_agent`
    /// destroys them, then restored with the new agent ID after spawn.
    pub fn take_agent_triggers(&self, agent_id: AgentId) -> Vec<Trigger> {
        let trigger_ids = self
            .agent_triggers
            .remove(&agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let mut taken = Vec::with_capacity(trigger_ids.len());
        for id in trigger_ids {
            if let Some((_, t)) = self.triggers.remove(&id) {
                self.forget_cooldowns(id);
                taken.push(t);
            }
        }
        if !taken.is_empty() {
            info!(
                agent = %agent_id,
                count = taken.len(),
                "Took triggers for agent (pending reassignment)"
            );
        }
        taken
    }

    /// Restore previously taken triggers under a new agent ID.
    ///
    /// Each trigger keeps its original pattern, prompt template, fire count,
    /// and max_fires, but is re-keyed to `new_agent_id`. New trigger IDs are
    /// generated so there are no stale references.
    ///
    /// Returns the number of triggers restored.
    pub fn restore_triggers(&self, new_agent_id: AgentId, triggers: Vec<Trigger>) -> usize {
        let count = triggers.len();
        for old in triggers {
            let new_id = TriggerId::new();
            let trigger = Trigger {
                id: new_id,
                agent_id: new_agent_id,
                pattern: old.pattern,
                prompt_template: old.prompt_template,
                enabled: old.enabled,
                created_at: old.created_at,
                fire_count: old.fire_count,
                max_fires: old.max_fires,
                target_agent: old.target_agent,
                cooldown_secs: old.cooldown_secs,
                session_mode: old.session_mode,
                last_fired_at: old.last_fired_at,
                workflow_id: old.workflow_id,
            };
            self.triggers.insert(new_id, trigger);
            self.agent_triggers
                .entry(new_agent_id)
                .or_default()
                .push(new_id);
        }
        if count > 0 {
            info!(
                agent = %new_agent_id,
                count,
                "Restored triggers under new agent"
            );
        }
        count
    }

    /// Reassign all triggers from one agent to another in place.
    ///
    /// Used during cold boot when the old agent ID (from persisted state) no
    /// longer exists and a new agent was spawned. Updates the `agent_id` field
    /// on each trigger and moves the index entry.
    ///
    /// Returns the number of triggers reassigned.
    pub fn reassign_agent_triggers(&self, old_agent_id: AgentId, new_agent_id: AgentId) -> usize {
        let trigger_ids = self
            .agent_triggers
            .remove(&old_agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let count = trigger_ids.len();
        for id in &trigger_ids {
            if let Some(mut t) = self.triggers.get_mut(id) {
                t.agent_id = new_agent_id;
            }
        }
        if !trigger_ids.is_empty() {
            self.agent_triggers
                .entry(new_agent_id)
                .or_default()
                .extend(trigger_ids);
            info!(
                old_agent = %old_agent_id,
                new_agent = %new_agent_id,
                count,
                "Reassigned triggers to new agent"
            );
        }
        count
    }

    /// Enable or disable a trigger. Returns true if the trigger was found.
    pub fn set_enabled(&self, trigger_id: TriggerId, enabled: bool) -> bool {
        if let Some(mut t) = self.triggers.get_mut(&trigger_id) {
            t.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Patch mutable fields of an existing trigger.
    ///
    /// Only `Some` fields are updated; `None` leaves the current value intact.
    /// Returns the updated trigger, or `None` if the ID was not found.
    pub fn update(&self, trigger_id: TriggerId, patch: TriggerPatch) -> Option<Trigger> {
        let mut entry = self.triggers.get_mut(&trigger_id)?;
        let t = entry.value_mut();
        let pattern_changed = patch.pattern.is_some();
        if let Some(pattern) = patch.pattern {
            t.pattern = pattern;
        }
        if let Some(prompt_template) = patch.prompt_template {
            t.prompt_template = prompt_template;
        }
        if let Some(enabled) = patch.enabled {
            t.enabled = enabled;
        }
        if let Some(max_fires) = patch.max_fires {
            t.max_fires = max_fires;
        }
        if let Some(cooldown_secs) = patch.cooldown_secs {
            t.cooldown_secs = cooldown_secs;
        }
        if let Some(session_mode) = patch.session_mode {
            t.session_mode = session_mode;
        }
        if let Some(target_agent) = patch.target_agent {
            t.target_agent = target_agent;
        }
        if let Some(workflow_id) = patch.workflow_id {
            t.workflow_id = workflow_id;
        }
        let id = t.id;
        drop(entry);
        // Pattern change means the trigger is logically new — clear any stale cooldown timer, including per-subject windows the old pattern opened whose subjects the new one may not even have (#6756).
        if pattern_changed {
            self.forget_cooldowns(id);
        }
        self.triggers.get(&id).map(|t| t.clone())
    }

    /// Get a single trigger by ID.
    pub fn get_trigger(&self, trigger_id: TriggerId) -> Option<Trigger> {
        self.triggers.get(&trigger_id).map(|t| t.clone())
    }

    /// List all triggers for an agent.
    pub fn list_agent_triggers(&self, agent_id: AgentId) -> Vec<Trigger> {
        self.agent_triggers
            .get(&agent_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.triggers.get(id).map(|t| t.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all registered triggers.
    pub fn list_all(&self) -> Vec<Trigger> {
        self.triggers.iter().map(|e| e.value().clone()).collect()
    }

    /// Evaluate an event against all triggers. Returns a list of
    /// (agent_id, message_to_send) pairs for matching triggers.
    ///
    /// Applies two layers of storm prevention:
    /// 1. **Cooldown** — after firing, the window it fired in is suppressed for `cooldown_secs` (default `DEFAULT_COOLDOWN_SECS`).
    ///    For most patterns that window is the trigger itself; for patterns that name a subject it is `(trigger, subject)`, so a second task or memory key is a separate window rather than a suppressed repeat (#6756, see `cooldown_subject`).
    ///    Set `cooldown_secs = Some(0)` on a trigger to disable its cooldown.
    /// 2. **Per-event budget** — at most `max_triggers_per_event` triggers may fire
    ///    from a single event evaluation. Excess matches are dropped with a warning.
    pub fn evaluate(&self, event: &Event) -> (Vec<TriggerMatch>, bool) {
        self.evaluate_with_resolver(event, |_| None)
    }

    /// Like [`evaluate`] but accepts an `agent_id -> name` resolver so
    /// patterns that match on the owning agent's identity
    /// (e.g. `TaskPosted { assignee_match: Some("self") }`) can compare the
    /// event's `assigned_to` string against the trigger-owner's **name** in
    /// addition to its UUID.
    ///
    /// Callers that don't have a name lookup available can still use
    /// [`evaluate`] — `self` matching will then only accept UUID strings.
    pub fn evaluate_with_resolver(
        &self,
        event: &Event,
        resolve_name: impl Fn(AgentId) -> Option<String>,
    ) -> (Vec<TriggerMatch>, bool) {
        let event_description = describe_event(event);
        let mut matches = Vec::new();
        let mut state_mutated = false;
        let now = Utc::now();

        // Iterate in deterministic order.  DashMap's native iterator
        // is order-by-shard-and-hash, so the same trigger set produces
        // a different evaluation order on every event — and when the
        // per-event budget caps the matches, the *set* of triggers
        // that fire is also non-deterministic.  #3923's existing
        // "ordered triggers" wording (and the CLAUDE.md determinism
        // rule for anything that ultimately reaches an LLM prompt
        // through TaskPosted / agent dispatch) calls for a stable
        // order; the audit caught that the evaluator itself was the
        // remaining gap.  Sorting the snapshot of trigger IDs before
        // taking each shard write-lock keeps storm prevention intact
        // (still drops excess matches at the budget) while making
        // *which* matches drop deterministic.
        let mut ids: Vec<TriggerId> = self.triggers.iter().map(|e| *e.key()).collect();
        ids.sort();
        // Snapshot the registered-trigger count *before* the loop takes any
        // shard write-lock. `DashMap::len()` read-locks every shard, so calling
        // it inside the loop — while a `self.triggers.get_mut(&id)` `RefMut`
        // holds that same shard's write-lock — self-deadlocks the evaluator the
        // first time the per-event budget is exhausted (it lands in the `warn!`
        // branch below). `ids.len()` is the same total, taken lock-free.
        let total_registered = ids.len();
        // Set when a fire stamps a cooldown window, so the prune below runs only when there is something new to prune.
        let mut inserted_cooldown = false;
        for id in ids {
            let Some(mut entry) = self.triggers.get_mut(&id) else {
                continue;
            };
            let trigger = entry.value_mut();

            if !trigger.enabled {
                continue;
            }

            // Check max fires
            if trigger.max_fires > 0 && trigger.fire_count >= trigger.max_fires {
                trigger.enabled = false;
                // enabled=false must be persisted even if this event produces no match.
                state_mutated = true;
                continue;
            }

            // Check the cooldown window using wall-clock timestamps so that windows survive daemon restarts.
            // The window is scoped to the event's subject where the pattern names one (#6756), so a second task completing a second after the first is a distinct window rather than a suppressed repeat.
            let subject = cooldown_subject(&trigger.pattern, event);
            let cooldown =
                Duration::from_secs(trigger.cooldown_secs.unwrap_or(self.default_cooldown_secs));
            if !cooldown.is_zero() {
                if let Some(last) = self.last_fired.get(&(trigger.id, subject.clone())) {
                    // `now - *last` is negative when `*last > now`, which can happen
                    // if the wall clock stepped backwards (NTP correction, manual
                    // adjustment, VM snapshot restore) or if the persisted
                    // `last_fired_at` was imported from a future-dated state.
                    // `to_std()` then errors; the old `unwrap_or(Duration::ZERO)`
                    // collapsed elapsed to 0 and wedged the trigger off until the wall clock caught up (#5115).
                    // Treat the anomaly as elapsed-exceeded so the trigger fires once: the subsequent `self.last_fired.insert((trigger.id, subject), now)` below stamps a sane timestamp and self-heals the entry.
                    let elapsed = match (now - *last).to_std() {
                        Ok(e) => e,
                        Err(_) => {
                            warn!(
                                trigger_id = %trigger.id,
                                agent_id = %trigger.agent_id,
                                now = %now,
                                last_fired_at = %*last,
                                "Trigger last_fired_at is in the future relative to now; \
                                 treating cooldown as elapsed (wall-clock backstep or \
                                 imported state). This entry will self-heal on next fire."
                            );
                            Duration::MAX
                        }
                    };
                    if elapsed < cooldown {
                        debug!(
                            trigger_id = %trigger.id,
                            "Trigger skipped (cooldown active)"
                        );
                        continue;
                    }
                }
            }

            let owner_name = resolve_name(trigger.agent_id);
            let owner = Some((trigger.agent_id, owner_name));
            if matches_pattern(&trigger.pattern, event, &event_description, owner) {
                // Enforce per-event trigger budget (storm prevention).
                //
                // We intentionally `break` here rather than `continue` — once the
                // budget is exhausted we stop evaluating entirely. Because
                // `DashMap` iteration order is non-deterministic, the set of
                // triggers that "win" the budget on any given event is effectively
                // random. This is acceptable for storm prevention: the goal is to
                // cap the blast radius of a single event, not to guarantee
                // deterministic priority. If deterministic priority is needed in
                // the future, triggers should be collected and sorted by an
                // explicit priority field before evaluation.
                //
                // The warning log includes the total number of registered
                // triggers so operators can compare it against the budget and
                // tune `max_triggers_per_event` accordingly.
                if matches.len() >= self.max_triggers_per_event {
                    warn!(
                        trigger_id = %trigger.id,
                        budget = self.max_triggers_per_event,
                        total_registered,
                        "Per-event trigger budget exhausted, skipping remaining matches — \
                         consider increasing max_triggers_per_event if too many triggers are starved"
                    );
                    break;
                }

                let message = trigger
                    .prompt_template
                    .replace("{{event}}", &event_description);
                // Route to target_agent if set (cross-session wake), else owner.
                let recipient = trigger.target_agent.unwrap_or(trigger.agent_id);
                matches.push(TriggerMatch {
                    agent_id: recipient,
                    message,
                    session_mode_override: trigger.session_mode,
                    workflow_id: trigger.workflow_id.clone(),
                    source: TriggerMatchSource::Registered(trigger.id),
                });
                trigger.fire_count += 1;
                state_mutated = true;
                // Stamp the window that was actually consulted, and the trigger-wide entry alongside it so `last_fired_at` on disk keeps meaning "when this trigger last fired" for operators and for restart recovery.
                self.last_fired.insert((trigger.id, subject.clone()), now);
                if subject.is_some() {
                    self.last_fired.insert((trigger.id, None), now);
                }
                // Pruning is deferred to after the loop on purpose: `entry` is a live `RefMut` on this trigger's shard, and the prune has to read `self.triggers` to learn the longest window in use.
                // Same-thread write-then-read on one shard is the self-deadlock this function already documents for `DashMap::len()` above.
                inserted_cooldown = true;

                debug!(
                    trigger_id = %trigger.id,
                    owner = %trigger.agent_id,
                    recipient = %recipient,
                    fire_count = trigger.fire_count,
                    "Trigger fired"
                );
            }
        }

        // Safe here and not inside the loop: every `RefMut` taken above has been dropped, so reading `self.triggers` cannot meet a write guard this thread is still holding.
        if inserted_cooldown {
            self.prune_expired_cooldowns(now);
        }

        (matches, state_mutated)
    }

    /// Whether a stored trigger already delivers `TaskPosted` events to
    /// `assignee_id`, for the built-in assignee wake (issue #6728).
    ///
    /// A record covers the assignee when **both** hold:
    /// 1. its pattern would match a `TaskPosted` addressed to that assignee — evaluated through [`agent_identity_filter_matches`], the same helper [`matches_pattern`] uses, against both identity forms the substrate accepts in `assigned_to` (UUID and display name); and
    /// 2. it dispatches *to* that assignee — owner, or `target_agent` when set.
    ///    An orchestrator's `assignee_match = "self"` trigger that happens to target the assignee fails (1) and is correctly not coverage: it fires for tasks addressed to the orchestrator.
    ///
    /// Scans all triggers rather than the `agent_triggers` index because that index is owner-keyed, so a trigger owned by one agent and targeted at another would be missed.
    /// The scan is O(triggers in the installation) — not O(one agent's triggers) — and runs once per `TaskPosted`, inside `publish_event_inner` and therefore ahead of dispatch.
    /// That is accepted rather than overlooked: the alternative is a second, assignee-keyed index that has to stay consistent across register / update / remove / re-key, and the ceiling here is `MAX_TRIGGERS_PER_AGENT` × agent count with a cheap per-entry test (a pattern discriminant and an id comparison reject almost everything before any string work).
    /// If an installation ever makes that product large enough to matter, the index is the fix, and it can be added without changing this function's contract.
    ///
    /// Cooldown state is deliberately not consulted: a cooldown-suppressed trigger is still coverage.
    /// Cooldown is transient dispatch state, and treating it as a gap would re-introduce the double wake that a per-event check causes.
    /// Only `enabled` and fire-exhaustion count.
    pub fn task_posted_coverage_for(
        &self,
        assignee_id: AgentId,
        assignee_name: Option<&str>,
        resolve_name: impl Fn(AgentId) -> Option<String>,
    ) -> TaskPostedCoverage {
        let assignee_uuid = assignee_id.to_string();

        // Filter during the walk so only records that actually address this assignee are materialised.
        // Collecting and sorting every id first, then looking each one up again, paid an O(n log n) sort and n shard lookups on every `TaskPosted` for a candidate set that is almost always empty or a single entry.
        let mut candidates: Vec<(TriggerId, bool)> = Vec::new();
        for entry in self.triggers.iter() {
            let trigger = entry.value();
            let TriggerPattern::TaskPosted { assignee_match } = &trigger.pattern else {
                continue;
            };
            // (2) dispatch target
            if trigger.target_agent.unwrap_or(trigger.agent_id) != assignee_id {
                continue;
            }
            // (1) would the filter accept a task addressed to this assignee,
            // in either of the two identity forms `task_claim` matches on?
            let owner = Some((trigger.agent_id, resolve_name(trigger.agent_id)));
            let addressable =
                agent_identity_filter_matches(assignee_match, Some(assignee_uuid.as_str()), &owner)
                    || assignee_name.is_some_and(|n| {
                        agent_identity_filter_matches(assignee_match, Some(n), &owner)
                    });
            if !addressable {
                continue;
            }

            let exhausted = trigger.max_fires > 0 && trigger.fire_count >= trigger.max_fires;
            candidates.push((trigger.id, trigger.enabled && !exhausted));
        }

        // `DashMap` iterates by shard and hash, so which record gets reported must not depend on iteration order.
        // Ordering the handful that address one assignee gives the same guarantee the whole-store sort used to, without paying for the whole store.
        candidates.sort_by_key(|(id, _)| *id);
        if let Some((id, _)) = candidates.iter().find(|(_, can_fire)| *can_fire) {
            return TaskPostedCoverage::Covered(*id);
        }
        let dormant: Vec<TriggerId> = candidates.into_iter().map(|(id, _)| id).collect();

        if dormant.is_empty() {
            TaskPostedCoverage::None
        } else {
            TaskPostedCoverage::Dormant(dormant)
        }
    }

    /// Drop cooldown entries that can no longer suppress anything.
    ///
    /// Per-subject windows are unbounded in principle — one entry per task id a trigger ever saw — so without this a long-lived daemon on a busy board would accumulate them for the life of the process.
    /// An entry older than the longest window any trigger could be using is dead weight: the check that consults it can only ever conclude "elapsed", so removing it is invisible.
    ///
    /// Runs only once the map is larger than a fire could plausibly need, so the common path pays a length check rather than a scan.
    fn prune_expired_cooldowns(&self, now: DateTime<Utc>) {
        const PRUNE_THRESHOLD: usize = 4096;
        if self.last_fired.len() < PRUNE_THRESHOLD {
            return;
        }
        let longest = self
            .triggers
            .iter()
            .map(|t| t.cooldown_secs.unwrap_or(self.default_cooldown_secs))
            .max()
            .unwrap_or(self.default_cooldown_secs);
        // `cooldown_secs` is a `u64` that arrives from the API unvalidated (`routes/workflows/triggers.rs` reads it with `as_u64` and neither clamps nor bounds it), so this arithmetic has to survive values no sane operator would type.
        // `Duration::seconds` panics past roughly 9.2e15, and a bare `as i64` on something larger wraps negative — a negative horizon makes the retain below drop every window including the live ones, silently disabling same-subject suppression installation-wide.
        // Saturating keeps "absurdly large" meaning "effectively never expires", which is what the operator asked for.
        let horizon = i64::try_from(longest)
            .ok()
            .and_then(chrono::Duration::try_seconds)
            .unwrap_or(chrono::Duration::MAX);
        // Keep the trigger-wide entries regardless: they are bounded by the trigger count and back `last_fired_at` on disk.
        self.last_fired
            .retain(|(_, subject), last| subject.is_none() || now - *last < horizon);
    }

    /// Drop every cooldown window belonging to a trigger — the trigger-wide entry and each per-subject one (#6756).
    /// A trigger that is gone or whose pattern changed must not leave windows behind that would suppress its successor.
    fn forget_cooldowns(&self, trigger_id: TriggerId) {
        self.last_fired.retain(|(id, _), _| *id != trigger_id);
    }

    /// Get a trigger by ID.
    pub fn get(&self, trigger_id: TriggerId) -> Option<Trigger> {
        self.triggers.get(&trigger_id).map(|t| t.clone())
    }

    /// Reconcile the runtime trigger store with an agent's declarative
    /// `[[triggers]]` block from `agent.toml` (#5014).
    ///
    /// Matching key: `(pattern_canonical_json, prompt_template)`. The
    /// rationale: triggers have no natural primary key — the same pattern
    /// can be reused with a different prompt for a different purpose, so
    /// `pattern` alone is too coarse; the `prompt_template` is the
    /// next-most-stable identifier on the operator side. The
    /// `created_at` / `fire_count` / `last_fired_at` runtime fields are
    /// state, not configuration, and intentionally excluded from the key.
    ///
    /// Behaviour:
    /// - **Manifest entry, no runtime match** → register a new trigger
    ///   with the manifest's fields.
    /// - **Manifest entry, runtime match** → update mutable fields
    ///   (`prompt_template` already matches by construction;
    ///   `enabled`, `max_fires`, `cooldown_secs`, `session_mode`,
    ///   `target_agent`, `workflow_id`) on the existing trigger so
    ///   TOML wins.
    /// - **Runtime trigger, no manifest match** (orphan) →
    ///   apply `orphan_policy`. `Keep` is no-op, `Warn` logs and
    ///   keeps, `Delete` removes.
    ///
    /// `resolve_target_agent` translates the manifest's `target_agent`
    /// name to a registered `AgentId`. Returning `None` causes the
    /// trigger to be registered without a target (legacy single-agent
    /// dispatch); the reconcile function logs a warning naming the
    /// unresolved string so operators can spot typos. Empty strings are
    /// treated as `None` before the resolver is consulted.
    ///
    /// The function is idempotent: applying it twice with the same
    /// inputs produces no changes after the first call (modulo timestamps
    /// on logs).
    ///
    /// Returns the number of (created, updated, deleted) triggers so the
    /// caller can decide whether to call `persist()`.
    pub fn reconcile_manifest_triggers(
        &self,
        agent_id: AgentId,
        manifest_triggers: &[librefang_types::agent::ManifestTrigger],
        orphan_policy: librefang_types::agent::OrphanPolicy,
        resolve_target_agent: impl Fn(&str) -> Option<AgentId>,
    ) -> ReconcileReport {
        let mut report = ReconcileReport::default();

        // Snapshot existing triggers for this agent so we can match by
        // (pattern, prompt) and detect orphans in a single pass without
        // holding the DashMap shard lock across mutation calls.
        let existing: Vec<Trigger> = self.list_agent_triggers(agent_id);
        // Track which existing trigger ids were "claimed" by a manifest entry.
        let mut claimed: std::collections::HashSet<TriggerId> = std::collections::HashSet::new();

        for (idx, mt) in manifest_triggers.iter().enumerate() {
            // Normalise + parse the pattern. Skip the entry (with a
            // warning) if it doesn't deserialise — a single bad entry
            // must not abort the rest of the reconcile.
            let normalised = normalize_manifest_pattern_json(mt.pattern.clone());
            let pattern: TriggerPattern = match serde_json::from_value(normalised.clone()) {
                Ok(p) => p,
                Err(e) => {
                    warn!(
                        agent = %agent_id,
                        index = idx,
                        pattern = %normalised,
                        error = %e,
                        "Skipping manifest trigger: invalid pattern"
                    );
                    report.skipped += 1;
                    continue;
                }
            };

            // Resolve target_agent name → AgentId. Empty string == unset.
            let target_agent: Option<AgentId> = match mt.target_agent.as_deref() {
                None | Some("") => None,
                Some(name) => match resolve_target_agent(name) {
                    Some(id) => Some(id),
                    None => {
                        warn!(
                            agent = %agent_id,
                            index = idx,
                            target = %name,
                            "Manifest trigger target_agent name did not resolve; \
                             registering with no target (event will fire on owner)"
                        );
                        None
                    }
                },
            };

            // cooldown_secs: TOML uses u64; the runtime stores Option<u64>
            // where `Some(0)` means "no cooldown" and `None` means "engine
            // default". Map `0` → None so the engine default applies, any
            // other value → Some(v). This matches the API behaviour where
            // the JSON field is optional.
            let cooldown_secs: Option<u64> = if mt.cooldown_secs == 0 {
                None
            } else {
                Some(mt.cooldown_secs)
            };

            let workflow_id = mt.workflow_id.as_ref().filter(|s| !s.is_empty()).cloned();

            // Match by (pattern, prompt_template) against the existing
            // store for this agent. First unclaimed runtime trigger wins
            // and is "claimed" so the next manifest entry with the same
            // key cannot grab it. If the manifest contains N identical
            // entries and the store has M ≤ N runtime triggers with that
            // key, the first M manifest entries update those triggers
            // in place and the remaining N-M fall through to the `None`
            // arm below, which calls `register_with_target` to create a
            // fresh runtime trigger per duplicate. Net effect: the
            // runtime trigger count for that key matches the manifest
            // count (no dedup), and a subsequent reconcile against the
            // same manifest is still idempotent because each of the N
            // entries now has exactly one matching runtime trigger.
            // Orphan handling is unrelated and only applies to runtime
            // triggers that no manifest entry claimed.
            let matched_id = existing.iter().find_map(|t| {
                if claimed.contains(&t.id) {
                    return None;
                }
                if t.pattern == pattern && t.prompt_template == mt.prompt_template {
                    Some(t.id)
                } else {
                    None
                }
            });

            match matched_id {
                Some(id) => {
                    claimed.insert(id);
                    // Update mutable fields in place — TOML wins. Skip the
                    // update if every field already matches so the
                    // reconcile is genuinely idempotent (no persist
                    // thrash, no spurious "trigger changed" log lines).
                    let needs_update = self
                        .triggers
                        .get(&id)
                        .map(|t| {
                            t.enabled != mt.enabled
                                || t.max_fires != mt.max_fires
                                || t.cooldown_secs != cooldown_secs
                                || t.session_mode != mt.session_mode
                                || t.target_agent != target_agent
                                || t.workflow_id != workflow_id
                        })
                        .unwrap_or(false);
                    if needs_update {
                        if let Some(mut entry) = self.triggers.get_mut(&id) {
                            entry.enabled = mt.enabled;
                            entry.max_fires = mt.max_fires;
                            entry.cooldown_secs = cooldown_secs;
                            entry.session_mode = mt.session_mode;
                            entry.target_agent = target_agent;
                            entry.workflow_id = workflow_id.clone();
                        }
                        report.updated += 1;
                        debug!(
                            agent = %agent_id,
                            trigger_id = %id,
                            "Updated trigger from manifest (TOML wins)"
                        );
                    }
                }
                None => {
                    // New manifest entry — register it. Pass `mt.enabled`
                    // at construction so a disabled manifest entry never
                    // exists in the store as enabled=true (closes the
                    // register-then-patch race where the event bus could
                    // fire the trigger between the two operations).
                    match self.register_with_target_enabled(
                        agent_id,
                        pattern,
                        mt.prompt_template.clone(),
                        mt.max_fires,
                        target_agent,
                        cooldown_secs,
                        mt.session_mode,
                        workflow_id,
                        mt.enabled,
                    ) {
                        Ok(new_id) => {
                            claimed.insert(new_id);
                            report.created += 1;
                            info!(
                                agent = %agent_id,
                                trigger_id = %new_id,
                                "Registered manifest trigger"
                            );
                        }
                        Err(cap_err) => {
                            // Audit-required behaviour: the operator
                            // MUST see the truncation. error! at the
                            // failure site is paired with a counted
                            // report field so the caller (kernel
                            // boot, agent reload) can also surface it
                            // as a one-line summary instead of having
                            // to grep the log.
                            tracing::error!(
                                agent_id = %agent_id,
                                current = cap_err.current_count,
                                max = cap_err.max,
                                "Manifest trigger refused — per-agent cap exceeded; \
                                 the remaining manifest entries for this agent will \
                                 be processed (matching existing runtime triggers \
                                 will still update) but no new triggers will be \
                                 created. Trim `triggers = [...]` in agent.toml.",
                            );
                            report.cap_exceeded += 1;
                        }
                    }
                }
            }
        }

        // Orphan handling: every existing trigger not claimed by a
        // manifest entry above.
        let orphans: Vec<TriggerId> = existing
            .iter()
            .filter(|t| !claimed.contains(&t.id))
            .map(|t| t.id)
            .collect();
        match orphan_policy {
            librefang_types::agent::OrphanPolicy::Keep => {
                // No-op — the original ad-hoc trigger(s) survive. Count
                // them so the caller has visibility into the orphan set
                // without scanning the store separately.
                report.orphans_kept = orphans.len();
            }
            librefang_types::agent::OrphanPolicy::Warn => {
                report.orphans_kept = orphans.len();
                for id in &orphans {
                    if let Some(t) = self.triggers.get(id) {
                        warn!(
                            agent = %agent_id,
                            trigger_id = %id,
                            pattern = ?t.pattern,
                            "Runtime trigger has no matching manifest entry \
                             (reconcile_orphans=\"warn\") — keeping"
                        );
                    }
                }
            }
            librefang_types::agent::OrphanPolicy::Delete => {
                for id in orphans {
                    if self.remove(id) {
                        report.deleted += 1;
                        info!(
                            agent = %agent_id,
                            trigger_id = %id,
                            "Removed orphan trigger (reconcile_orphans=\"delete\")"
                        );
                    }
                }
            }
        }

        report
    }
}

impl Default for TriggerEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of a `reconcile_manifest_triggers` call.
///
/// `created + updated + deleted == 0 && skipped == 0` means the manifest
/// and runtime state were already in sync — the caller can safely skip
/// the persist() write. `skipped` counts manifest entries that failed
/// to deserialise (bad `pattern`) and were ignored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Manifest triggers that did not exist before this call.
    pub created: usize,
    /// Existing triggers whose mutable fields were updated from the
    /// manifest.
    pub updated: usize,
    /// Runtime-only triggers removed under
    /// `OrphanPolicy::Delete`.
    pub deleted: usize,
    /// Runtime-only triggers preserved under
    /// `OrphanPolicy::Keep` / `Warn`.
    pub orphans_kept: usize,
    /// Manifest entries skipped because their `pattern` did not
    /// deserialise into a `TriggerPattern`.
    pub skipped: usize,
    /// Manifest entries refused because registering would have
    /// pushed the agent past [`MAX_TRIGGERS_PER_AGENT`]. Logged at
    /// `error!` level at the rejection site; surfaced here as a
    /// count so the caller can decide whether to fail the agent
    /// boot or just persist the cap-truncated state (audit:
    /// trigger-engine-no-per-agent-cap).
    pub cap_exceeded: usize,
}

impl ReconcileReport {
    /// True when the runtime store was mutated.
    pub fn mutated(&self) -> bool {
        self.created > 0 || self.updated > 0 || self.deleted > 0
    }
}

/// Normalise a manifest trigger pattern JSON value (#5014).
///
/// Mirrors `normalize_pattern_json` in the API route so a manifest entry
/// like `pattern = "task_posted"` parses identically to the API form
/// `{"task_posted": {}}`. Extend the match when other variants gain
/// optional fields.
pub fn normalize_manifest_pattern_json(value: serde_json::Value) -> serde_json::Value {
    match value.as_str() {
        Some(tag @ ("task_posted" | "task_claimed" | "task_completed")) => {
            serde_json::json!({ tag: {} })
        }
        _ => value,
    }
}

/// Check if an event matches a trigger pattern.
fn matches_pattern(
    pattern: &TriggerPattern,
    event: &Event,
    description: &str,
    owner: Option<(AgentId, Option<String>)>,
) -> bool {
    match pattern {
        TriggerPattern::All => true,
        TriggerPattern::Lifecycle => {
            matches!(event.payload, EventPayload::Lifecycle(_))
        }
        TriggerPattern::AgentSpawned { name_pattern } => {
            if let EventPayload::Lifecycle(LifecycleEvent::Spawned { name, .. }) = &event.payload {
                name.contains(name_pattern.as_str()) || name_pattern == "*"
            } else {
                false
            }
        }
        TriggerPattern::AgentTerminated => matches!(
            event.payload,
            EventPayload::Lifecycle(LifecycleEvent::Terminated { .. })
                | EventPayload::Lifecycle(LifecycleEvent::Crashed { .. })
        ),
        TriggerPattern::System => {
            matches!(event.payload, EventPayload::System(_))
        }
        TriggerPattern::SystemKeyword { keyword } => {
            if let EventPayload::System(se) = &event.payload {
                let se_str = format!("{:?}", se).to_lowercase();
                se_str.contains(&keyword.to_lowercase())
            } else {
                false
            }
        }
        TriggerPattern::MemoryUpdate => {
            matches!(event.payload, EventPayload::MemoryUpdate(_))
        }
        TriggerPattern::MemoryKeyPattern { key_pattern } => {
            if let EventPayload::MemoryUpdate(delta) = &event.payload {
                delta.key.contains(key_pattern.as_str()) || key_pattern == "*"
            } else {
                false
            }
        }
        TriggerPattern::ContentMatch { substring } => description
            .to_lowercase()
            .contains(&substring.to_lowercase()),
        TriggerPattern::TaskPosted { assignee_match } => match &event.payload {
            EventPayload::System(SystemEvent::TaskPosted { assigned_to, .. }) => {
                agent_identity_filter_matches(assignee_match, assigned_to.as_deref(), &owner)
            }
            _ => false,
        },
        TriggerPattern::TaskClaimed { creator_match } => match &event.payload {
            EventPayload::System(SystemEvent::TaskClaimed { created_by, .. }) => {
                agent_identity_filter_matches(creator_match, created_by.as_deref(), &owner)
            }
            _ => false,
        },
        TriggerPattern::TaskCompleted { creator_match } => match &event.payload {
            EventPayload::System(SystemEvent::TaskCompleted { created_by, .. }) => {
                agent_identity_filter_matches(creator_match, created_by.as_deref(), &owner)
            }
            _ => false,
        },
    }
}

/// Resolve an optional agent-identity filter against a candidate agent string.
///
/// Shared by the task-board patterns so `assignee_match` (`TaskPosted`) and
/// `creator_match` (`TaskClaimed` / `TaskCompleted`) have byte-identical
/// semantics:
/// - `filter == None` → always matches (legacy fire-for-all).
/// - `filter == Some("unassigned")` → matches exactly the tasks no agent owns, which is both the `None` candidate and the empty string.
///   Both spellings reach the event because neither entry point normalises `assigned_to`: `tool_task_post` reads `input["assigned_to"].as_str()` (`librefang_runtime::tool_runner::task`) and `POST /api/tasks` reads `body["assigned_to"].as_str()` (`librefang_api::routes::task_queue`), and both hand the result straight to `task_post`, which copies it into `SystemEvent::TaskPosted` verbatim.
///   `title` and `description` are checked for emptiness at both entry points; `assigned_to` is not.
///   So a model or an API client that sends `"assigned_to": ""` — semantically "nobody", literally not `None` — produces `Some("")`, and a filter that only understood `None` would miss it.
///   This arm must be tested BEFORE the `candidate == None` early-exit below, or it is unreachable for the `None` half.
///
///   Note this is about what the *event* carries, not about what is in the database.
///   The stuck-task sweeper does write `assigned_to = ''` when it releases a claim, but it publishes no event at all — its only caller (`spawn_task_board_sweep_task`) logs the reset ids and stops — so a task it releases never reaches this predicate.
///   Releasing a stuck claim therefore does not wake an `assignee_match = "unassigned"` trigger, which is a real gap, but a separate one; see #6728.
/// - `candidate == None` → never matches any other non-`None` filter (the task field isn't set, so any identity predicate is definitionally false).
/// - `filter == Some("self")` → matches when `candidate` equals the trigger-owner's UUID **or** display name (via the resolver-supplied `owner` tuple).
/// - `filter == Some("<uuid>"|"<name>")` → exact string match against `candidate`.
///
/// `"self"` and `"unassigned"` are keywords, so an agent whose display name is one of those two cannot be addressed by name here — use its UUID.
fn agent_identity_filter_matches(
    filter: &Option<String>,
    candidate: Option<&str>,
    owner: &Option<(AgentId, Option<String>)>,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    // Before the `candidate == None` early-exit: "unassigned" is the one filter for which an absent candidate is a MATCH, not a miss.
    if filter == "unassigned" {
        return candidate.is_none_or(|c| c.is_empty());
    }
    let Some(candidate) = candidate else {
        return false;
    };
    match filter.as_str() {
        "self" => match owner {
            Some((id, name)) => candidate == id.to_string() || name.as_deref() == Some(candidate),
            None => false,
        },
        other => candidate == other,
    }
}

/// Build a `TaskPosted` event for tests in sibling modules.
///
/// Lives here rather than in the test module that uses it because
/// `EventPayload::System` construction is otherwise repeated verbatim in three
/// files, and a drift in the shape should break one place, not three.
#[cfg(test)]
pub(crate) fn tests_support_task_posted_event(task_id: &str, assigned_to: &str) -> Event {
    Event::new(
        AgentId::new(),
        librefang_types::event::EventTarget::Broadcast,
        EventPayload::System(SystemEvent::TaskPosted {
            task_id: task_id.to_string(),
            title: "test task".to_string(),
            assigned_to: Some(assigned_to.to_string()),
            created_by: None,
        }),
    )
}

/// What a matching event is *about*, for the cooldown window (issue #6756).
///
/// `Some(subject)` narrows the window to that subject, so two distinct subjects arriving inside one window no longer suppress each other; `None` keeps the trigger-wide window.
///
/// Deliberately driven by the **pattern**, not by whatever the event happens to carry, and the line is this: a pattern that names *what happened* to an identifiable subject is scoped; a pattern that names a *category* of events keeps the trigger-wide window.
///
/// Scoped, because two subjects are definitionally two units of work and nothing re-announces the second: the three task-board patterns (the task), `MemoryUpdate` and `MemoryKeyPattern` (the key), `AgentSpawned` and `AgentTerminated` (the agent).
///
/// Trigger-wide, because the operator asked for a bounded firehose rather than per-subject delivery: `All`, `System`, `SystemKeyword`, `Lifecycle`, `ContentMatch`.
/// Keying those on the subject would turn "at most once per window" into "once per event", which is the opposite of what setting a cooldown on a catch-all is for.
///
/// `MemoryUpdate` sits on the scoped side even though it carries no filter of its own, because it matches exactly the events `MemoryKeyPattern { key_pattern: "*" }` matches; leaving it trigger-wide would give two triggers with identical match sets opposite semantics, which is a sharper edge than either rule alone.
fn cooldown_subject(pattern: &TriggerPattern, event: &Event) -> Option<String> {
    match pattern {
        TriggerPattern::TaskPosted { .. }
        | TriggerPattern::TaskClaimed { .. }
        | TriggerPattern::TaskCompleted { .. } => match &event.payload {
            EventPayload::System(SystemEvent::TaskPosted { task_id, .. })
            | EventPayload::System(SystemEvent::TaskClaimed { task_id, .. })
            | EventPayload::System(SystemEvent::TaskCompleted { task_id, .. }) => {
                Some(task_id.clone())
            }
            _ => None,
        },
        // `MemoryUpdate` matches exactly the same events as `MemoryKeyPattern { key_pattern: "*" }`, so scoping one and not the other would give two triggers with identical match sets opposite delivery semantics — the asymmetry an operator would hit first.
        TriggerPattern::MemoryUpdate | TriggerPattern::MemoryKeyPattern { .. } => {
            match &event.payload {
                EventPayload::MemoryUpdate(delta) => Some(delta.key.clone()),
                _ => None,
            }
        }
        // Structurally identical to `MemoryKeyPattern`: a substring/wildcard filter over a per-event identifier, which can match two distinct subjects inside one window.
        // Two workers spawning a second apart is the same "a different thing happened" case a second task is.
        //
        // Keyed on the spawned agent's id rather than its name: the pattern filters on the name, but two agents may share one, and the subject here is the agent that appeared, not the label it appeared under.
        TriggerPattern::AgentSpawned { .. } => match &event.payload {
            EventPayload::Lifecycle(LifecycleEvent::Spawned { agent_id, .. }) => {
                Some(agent_id.to_string())
            }
            _ => None,
        },
        // Names a transition rather than a category, and both variants it matches are that transition happening to one identifiable agent.
        // Two agents crashing a second apart are two incidents.
        TriggerPattern::AgentTerminated => match &event.payload {
            EventPayload::Lifecycle(LifecycleEvent::Terminated { agent_id, .. })
            | EventPayload::Lifecycle(LifecycleEvent::Crashed { agent_id, .. }) => {
                Some(agent_id.to_string())
            }
            _ => None,
        },
        _ => None,
    }
}

/// Create a human-readable description of an event for use in prompts.
fn describe_event(event: &Event) -> String {
    match &event.payload {
        EventPayload::Message(msg) => {
            format!("Message from {:?}: {}", msg.role, msg.content)
        }
        EventPayload::ToolResult(tr) => {
            format!(
                "Tool '{}' {} ({}ms): {}",
                tr.tool_id,
                if tr.success { "succeeded" } else { "failed" },
                tr.execution_time_ms,
                librefang_types::truncate_str(&tr.content, 200)
            )
        }
        EventPayload::MemoryUpdate(delta) => {
            format!(
                "Memory {:?} on key '{}' for agent {}",
                delta.operation, delta.key, delta.agent_id
            )
        }
        EventPayload::Lifecycle(le) => match le {
            LifecycleEvent::Spawned { agent_id, name } => {
                format!("Agent '{name}' (id: {agent_id}) was spawned")
            }
            LifecycleEvent::Started { agent_id } => {
                format!("Agent {agent_id} started")
            }
            LifecycleEvent::Suspended { agent_id } => {
                format!("Agent {agent_id} suspended")
            }
            LifecycleEvent::Resumed { agent_id } => {
                format!("Agent {agent_id} resumed")
            }
            LifecycleEvent::Terminated { agent_id, reason } => {
                format!("Agent {agent_id} terminated: {reason}")
            }
            LifecycleEvent::Crashed { agent_id, error } => {
                format!("Agent {agent_id} crashed: {error}")
            }
        },
        EventPayload::Network(ne) => {
            format!("Network event: {:?}", ne)
        }
        EventPayload::System(se) => match se {
            SystemEvent::KernelStarted => "Kernel started".to_string(),
            SystemEvent::KernelStopping => "Kernel stopping".to_string(),
            SystemEvent::QuotaWarning {
                agent_id,
                resource,
                usage_percent,
            } => format!("Quota warning: agent {agent_id}, {resource} at {usage_percent:.1}%"),
            SystemEvent::HealthCheck { status } => {
                format!("Health check: {status}")
            }
            SystemEvent::QuotaEnforced {
                agent_id,
                spent,
                limit,
            } => {
                format!("Quota enforced: agent {agent_id}, spent ${spent:.4} / ${limit:.4}")
            }
            SystemEvent::ModelRouted {
                agent_id,
                complexity,
                model,
            } => {
                format!("Model routed: agent {agent_id}, complexity={complexity}, model={model}")
            }
            SystemEvent::UserAction {
                user_id,
                action,
                result,
            } => {
                format!("User action: {user_id} {action} -> {result}")
            }
            SystemEvent::HealthCheckFailed {
                agent_id,
                unresponsive_secs,
            } => {
                format!(
                    "Health check failed: agent {agent_id}, unresponsive for {unresponsive_secs}s"
                )
            }
            SystemEvent::TaskPosted { task_id, title, .. } => {
                format!("Task posted: {task_id} \"{title}\"")
            }
            SystemEvent::TaskClaimed {
                task_id,
                claimed_by,
                ..
            } => {
                format!("Task claimed: {task_id} by {claimed_by}")
            }
            SystemEvent::TaskCompleted {
                task_id,
                completed_by,
                result,
                ..
            } => {
                format!("Task completed: {task_id} by {completed_by} result={result}")
            }
        },
        EventPayload::ApprovalRequested(ar) => {
            format!(
                "Approval requested: agent {} wants to use tool '{}' (risk: {}): {}",
                ar.agent_id, ar.tool_name, ar.risk_level, ar.description
            )
        }
        EventPayload::ApprovalResolved(ar) => {
            format!(
                "Approval resolved: request {} — {}",
                ar.request_id, ar.decision
            )
        }
        EventPayload::Custom(data) => {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(data) {
                let event_type = val
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let summary = {
                    let s = val.to_string();
                    if s.len() > 300 {
                        // `&s[..300]` panics when byte 300 lands inside a
                        // multi-byte UTF-8 codepoint; Custom payloads are
                        // operator/attacker-controlled. truncate_str snaps to a
                        // char boundary (used the same way at line ~1404).
                        format!("{}...", librefang_types::truncate_str(&s, 300))
                    } else {
                        s
                    }
                };
                format!("Custom event: type={}, payload={}", event_type, summary)
            } else {
                format!("Custom event ({} bytes)", data.len())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::event::*;

    #[test]
    fn poisoned_trigger_persistence_lock_recovers_and_remains_exclusive() {
        let lock = std::sync::Mutex::new(());
        let poison = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = lock.lock().unwrap();
                    panic!("poison trigger persistence lock");
                })
                .join()
        });

        assert!(poison.is_err());
        assert!(lock.is_poisoned());
        let recovered = lock_trigger_persistence(&lock);
        assert!(!lock.is_poisoned());
        assert!(lock.try_lock().is_err());
        drop(recovered);
        let ordinary_guard = lock.lock().unwrap();
        drop(ordinary_guard);
    }

    #[test]
    fn test_register_trigger() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine
            .register(
                agent_id,
                TriggerPattern::All,
                "Event occurred: {{event}}".to_string(),
                0,
            )
            .unwrap();
        assert!(engine.get(id).is_some());
    }

    #[test]
    fn test_evaluate_lifecycle() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine
            .register(
                watcher,
                TriggerPattern::Lifecycle,
                "Lifecycle: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "new-agent".to_string(),
            }),
        );

        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].agent_id, watcher);
        assert!(matches[0].message.contains("new-agent"));
    }

    #[test]
    fn test_evaluate_agent_spawned_pattern() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine
            .register(
                watcher,
                TriggerPattern::AgentSpawned {
                    name_pattern: "coder".to_string(),
                },
                "Coder spawned: {{event}}".to_string(),
                0,
            )
            .unwrap();

        // This should match
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "coder".to_string(),
            }),
        );
        assert_eq!(engine.evaluate(&event).0.len(), 1);

        // This should NOT match
        let event2 = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "researcher".to_string(),
            }),
        );
        assert_eq!(engine.evaluate(&event2).0.len(), 0);
    }

    #[test]
    fn test_max_fires() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let tid = engine
            .register(
                agent_id,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                2, // max 2 fires
            )
            .unwrap();
        // Disable cooldown so we can fire rapidly in the test.
        engine.triggers.get_mut(&tid).unwrap().cooldown_secs = Some(0);

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // First two should match
        assert_eq!(engine.evaluate(&event).0.len(), 1);
        assert_eq!(engine.evaluate(&event).0.len(), 1);
        // Third should not
        assert_eq!(engine.evaluate(&event).0.len(), 0);
    }

    #[test]
    fn test_remove_trigger() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine
            .register(agent_id, TriggerPattern::All, "msg".to_string(), 0)
            .unwrap();
        assert!(engine.remove(id));
        assert!(engine.get(id).is_none());
    }

    #[test]
    fn test_remove_agent_triggers() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine
            .register(agent_id, TriggerPattern::All, "a".to_string(), 0)
            .unwrap();
        engine
            .register(agent_id, TriggerPattern::System, "b".to_string(), 0)
            .unwrap();
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 2);

        engine.remove_agent_triggers(agent_id);
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 0);
    }

    #[test]
    fn test_content_match() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine
            .register(
                agent_id,
                TriggerPattern::ContentMatch {
                    substring: "quota".to_string(),
                },
                "Alert: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::System,
            EventPayload::System(SystemEvent::QuotaWarning {
                agent_id: AgentId::new(),
                resource: "tokens".to_string(),
                usage_percent: 85.0,
            }),
        );
        assert_eq!(engine.evaluate(&event).0.len(), 1);
    }

    // -- reassign_agent_triggers (#519) ------------------------------------

    #[test]
    fn test_reassign_agent_triggers_basic() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        engine
            .register(old_agent, TriggerPattern::All, "a".to_string(), 0)
            .unwrap();
        engine
            .register(old_agent, TriggerPattern::System, "b".to_string(), 0)
            .unwrap();

        let count = engine.reassign_agent_triggers(old_agent, new_agent);
        assert_eq!(count, 2);
        assert_eq!(engine.list_agent_triggers(old_agent).len(), 0);
        assert_eq!(engine.list_agent_triggers(new_agent).len(), 2);

        // Verify triggers actually fire for the new agent
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|m| m.agent_id == new_agent));
    }

    #[test]
    fn test_reassign_agent_triggers_no_match_returns_zero() {
        let engine = TriggerEngine::new();
        let agent_a = AgentId::new();
        engine
            .register(agent_a, TriggerPattern::All, "a".to_string(), 0)
            .unwrap();

        let count = engine.reassign_agent_triggers(AgentId::new(), AgentId::new());
        assert_eq!(count, 0);
        // Original triggers untouched
        assert_eq!(engine.list_agent_triggers(agent_a).len(), 1);
    }

    #[test]
    fn test_reassign_does_not_touch_other_agents() {
        let engine = TriggerEngine::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let agent_c = AgentId::new();
        engine
            .register(agent_a, TriggerPattern::All, "a".to_string(), 0)
            .unwrap();
        engine
            .register(agent_b, TriggerPattern::System, "b".to_string(), 0)
            .unwrap();

        let count = engine.reassign_agent_triggers(agent_a, agent_c);
        assert_eq!(count, 1);
        // agent_b untouched
        assert_eq!(engine.list_agent_triggers(agent_b).len(), 1);
        assert_eq!(engine.list_agent_triggers(agent_c).len(), 1);
    }

    // -- take / restore triggers (#519) ------------------------------------

    #[test]
    fn test_take_and_restore_triggers() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        engine
            .register(
                old_agent,
                TriggerPattern::ContentMatch {
                    substring: "deploy".to_string(),
                },
                "Deploy alert: {{event}}".to_string(),
                5,
            )
            .unwrap();
        engine
            .register(old_agent, TriggerPattern::Lifecycle, "lc".to_string(), 0)
            .unwrap();

        // Take triggers — engine should be empty for old agent
        let taken = engine.take_agent_triggers(old_agent);
        assert_eq!(taken.len(), 2);
        assert_eq!(engine.list_agent_triggers(old_agent).len(), 0);
        assert_eq!(engine.list_all().len(), 0);

        // Restore under new agent
        let restored = engine.restore_triggers(new_agent, taken);
        assert_eq!(restored, 2);
        assert_eq!(engine.list_agent_triggers(new_agent).len(), 2);

        // Verify patterns and max_fires are preserved
        let triggers = engine.list_agent_triggers(new_agent);
        let has_content_match = triggers.iter().any(|t| {
            matches!(&t.pattern, TriggerPattern::ContentMatch { substring } if substring == "deploy")
                && t.max_fires == 5
        });
        assert!(
            has_content_match,
            "ContentMatch trigger with max_fires=5 should be preserved"
        );
    }

    #[test]
    fn test_take_empty_returns_empty() {
        let engine = TriggerEngine::new();
        let taken = engine.take_agent_triggers(AgentId::new());
        assert!(taken.is_empty());
    }

    #[test]
    fn test_restore_preserves_enabled_state() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        let tid = engine
            .register(old_agent, TriggerPattern::All, "a".to_string(), 0)
            .unwrap();
        engine.set_enabled(tid, false);

        let taken = engine.take_agent_triggers(old_agent);
        assert_eq!(taken.len(), 1);
        assert!(!taken[0].enabled);

        engine.restore_triggers(new_agent, taken);
        let restored = engine.list_agent_triggers(new_agent);
        assert_eq!(restored.len(), 1);
        assert!(
            !restored[0].enabled,
            "Disabled state should survive take/restore"
        );
    }

    // -- cross-session wake / target_agent (#967) -----------------------------

    #[test]
    fn test_evaluate_no_target_wakes_owner() {
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        engine
            .register(
                owner,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].agent_id, owner,
            "Without target_agent, owner should be woken"
        );
    }

    #[test]
    fn test_evaluate_with_target_wakes_target() {
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        let target = AgentId::new();
        engine
            .register_with_target(
                owner,
                TriggerPattern::All,
                "Cross-wake: {{event}}".to_string(),
                0,
                Some(target),
                None,
                None,
                None,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].agent_id, target,
            "With target_agent set, target should be woken"
        );
        assert!(matches[0].message.contains("Cross-wake"));
    }

    #[test]
    fn test_register_cross_agent_trigger() {
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        let target = AgentId::new();
        let tid = engine
            .register_cross_agent_trigger(
                owner,
                target,
                TriggerPattern::AgentSpawned {
                    name_pattern: "worker".to_string(),
                },
                "Worker spawned: {{event}}".to_string(),
            )
            .unwrap();

        let trigger = engine.get(tid).unwrap();
        assert_eq!(trigger.agent_id, owner);
        assert_eq!(trigger.target_agent, Some(target));
        assert_eq!(trigger.max_fires, 0); // unlimited by default

        // Verify it fires to the target agent
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "worker-1".to_string(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].agent_id, target);
    }

    #[test]
    fn test_take_restore_preserves_target_agent() {
        let engine = TriggerEngine::new();
        let old_owner = AgentId::new();
        let target = AgentId::new();
        let new_owner = AgentId::new();

        engine
            .register_with_target(
                old_owner,
                TriggerPattern::System,
                "sys: {{event}}".to_string(),
                0,
                Some(target),
                None,
                None,
                None,
            )
            .unwrap();

        let taken = engine.take_agent_triggers(old_owner);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].target_agent, Some(target));

        engine.restore_triggers(new_owner, taken);
        let restored = engine.list_agent_triggers(new_owner);
        assert_eq!(restored.len(), 1);
        assert_eq!(
            restored[0].target_agent,
            Some(target),
            "target_agent should survive take/restore"
        );
    }

    // -- cooldown & per-event budget ----------------------------------------

    #[test]
    fn test_cooldown_suppresses_rapid_refire() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        // Register trigger with default cooldown (5s)
        engine
            .register(
                agent_id,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // First evaluation fires
        assert_eq!(engine.evaluate(&event).0.len(), 1);
        // Immediate second evaluation should be suppressed by cooldown
        assert_eq!(engine.evaluate(&event).0.len(), 0);
    }

    /// Regression test for #5115: when the persisted `last_fired_at` is in
    /// the future relative to `now` (wall-clock backstep, imported state,
    /// VM snapshot restore), the trigger must still fire instead of being
    /// silently wedged off until the wall clock catches up.
    #[test]
    fn test_cooldown_unwedges_on_future_last_fired_at() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let tid = engine
            .register(
                agent_id,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Simulate a future-dated `last_fired_at` — far enough ahead that
        // the bug's `unwrap_or(Duration::ZERO)` path would suppress every
        // fire for the next hour.
        let future = Utc::now() + chrono::Duration::hours(1);
        engine.last_fired.insert((tid, None), future);

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // Trigger must fire despite the future-dated stamp.
        assert_eq!(
            engine.evaluate(&event).0.len(),
            1,
            "trigger must fire when last_fired_at is in the future (#5115)"
        );

        // After firing, `last_fired` is rewritten to `now` (≤ Utc::now() at
        // the assertion point) — the anomaly has self-healed and normal
        // cooldown behaviour resumes.
        let stamped = *engine.last_fired.get(&(tid, None)).unwrap();
        assert!(
            stamped <= Utc::now(),
            "last_fired must be reset to a non-future timestamp after firing"
        );
        // Immediate refire is now suppressed by the normal cooldown path.
        assert_eq!(engine.evaluate(&event).0.len(), 0);
    }

    #[test]
    fn test_zero_cooldown_allows_rapid_refire() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let tid = engine
            .register(
                agent_id,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                0,
            )
            .unwrap();
        // Explicitly disable cooldown
        engine.triggers.get_mut(&tid).unwrap().cooldown_secs = Some(0);

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        assert_eq!(engine.evaluate(&event).0.len(), 1);
        assert_eq!(engine.evaluate(&event).0.len(), 1);
        assert_eq!(engine.evaluate(&event).0.len(), 1);
    }

    #[test]
    fn test_per_event_trigger_budget() {
        // Create engine with a budget of 3 triggers per event
        let engine = TriggerEngine::with_max_triggers_per_event(3);
        let agents: Vec<AgentId> = (0..5).map(|_| AgentId::new()).collect();

        // Register 5 triggers — all match All pattern
        for agent_id in &agents {
            let tid = engine
                .register(
                    *agent_id,
                    TriggerPattern::All,
                    "Event: {{event}}".to_string(),
                    0,
                )
                .unwrap();
            // Disable cooldown so all are eligible
            engine.triggers.get_mut(&tid).unwrap().cooldown_secs = Some(0);
        }

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // Only 3 should fire due to budget
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_cooldown_clears_on_remove() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let tid = engine
            .register(
                agent_id,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // Fire to create a last_fired entry
        engine.evaluate(&event);
        assert!(engine.last_fired.contains_key(&(tid, None)));

        // Remove should clean up
        engine.remove(tid);
        assert!(!engine.last_fired.contains_key(&(tid, None)));
    }

    #[test]
    fn test_restore_preserves_cooldown_secs() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        let tid = engine
            .register(old_agent, TriggerPattern::All, "a".to_string(), 0)
            .unwrap();
        engine.triggers.get_mut(&tid).unwrap().cooldown_secs = Some(30);

        let taken = engine.take_agent_triggers(old_agent);
        assert_eq!(taken[0].cooldown_secs, Some(30));

        engine.restore_triggers(new_agent, taken);
        let restored = engine.list_agent_triggers(new_agent);
        assert_eq!(
            restored[0].cooldown_secs,
            Some(30),
            "cooldown_secs should survive take/restore"
        );
    }

    // -- describe_event: Custom payload decoding (#2438) -----------------------

    #[test]
    fn test_describe_event_custom_json() {
        let payload =
            serde_json::to_vec(&serde_json::json!({"type": "deploy", "data": {"env": "prod"}}))
                .unwrap();
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Custom(payload),
        );
        let desc = describe_event(&event);
        assert!(
            desc.contains("type=deploy"),
            "Should include the event type, got: {desc}"
        );
        assert!(
            desc.contains("prod"),
            "Should include payload data, got: {desc}"
        );
    }

    #[test]
    fn test_describe_event_custom_non_json_fallback() {
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Custom(vec![0xFF, 0xFE, 0x00]),
        );
        let desc = describe_event(&event);
        assert!(
            desc.contains("3 bytes"),
            "Non-JSON should fall back to byte-length description, got: {desc}"
        );
    }

    #[test]
    fn test_describe_event_custom_json_no_type_field() {
        let payload = serde_json::to_vec(&serde_json::json!({"action": "restart"})).unwrap();
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Custom(payload),
        );
        let desc = describe_event(&event);
        assert!(
            desc.contains("type=unknown"),
            "Missing 'type' field should show 'unknown', got: {desc}"
        );
    }

    #[test]
    fn test_content_match_on_custom_json_event() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine
            .register(
                agent_id,
                TriggerPattern::ContentMatch {
                    substring: "deploy".to_string(),
                },
                "Deploy alert: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let payload =
            serde_json::to_vec(&serde_json::json!({"type": "deploy", "data": {"env": "prod"}}))
                .unwrap();
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Custom(payload),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(
            matches.len(),
            1,
            "ContentMatch should match decoded Custom JSON payload"
        );
    }

    // -- MemoryUpdate trigger matching (#2438) ---------------------------------

    #[test]
    fn test_memory_update_trigger_fires() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine
            .register(
                watcher,
                TriggerPattern::MemoryUpdate,
                "Memory changed: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::MemoryUpdate(MemoryDelta {
                operation: MemoryOperation::Created,
                key: "user.prefs".to_string(),
                agent_id: AgentId::new(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert!(matches[0].message.contains("user.prefs"));
    }

    #[test]
    fn test_memory_key_pattern_trigger_fires() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine
            .register(
                watcher,
                TriggerPattern::MemoryKeyPattern {
                    key_pattern: "user.".to_string(),
                },
                "User memory changed: {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Should match
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::MemoryUpdate(MemoryDelta {
                operation: MemoryOperation::Updated,
                key: "user.settings".to_string(),
                agent_id: AgentId::new(),
            }),
        );
        assert_eq!(engine.evaluate(&event).0.len(), 1);

        // Should NOT match (different key)
        let event2 = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::MemoryUpdate(MemoryDelta {
                operation: MemoryOperation::Deleted,
                key: "system.config".to_string(),
                agent_id: AgentId::new(),
            }),
        );
        // Disable cooldown for second evaluation
        for mut entry in engine.triggers.iter_mut() {
            entry.cooldown_secs = Some(0);
        }
        assert_eq!(engine.evaluate(&event2).0.len(), 0);
    }

    #[test]
    fn task_posted_assignee_match_self_filters_by_uuid_and_name() {
        // Regression test for #2924 — `{"task_posted":{"assignee_match":"self"}}`
        // must only fire for tasks assigned to the trigger-owning agent.
        let engine = TriggerEngine::new();
        let worker = AgentId::new();
        let delegator = AgentId::new();

        engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some("self".to_string()),
                },
                "claim and work on {{event}}".to_string(),
                0,
            )
            .unwrap();

        // A task assigned to the delegator must NOT match.
        let event_other = Event::new(
            delegator,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskPosted {
                task_id: "t-1".to_string(),
                title: "Unrelated".to_string(),
                assigned_to: Some(delegator.to_string()),
                created_by: Some(delegator.to_string()),
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&event_other, |id| {
            if id == worker {
                Some("worker".to_string())
            } else {
                None
            }
        });
        assert!(
            matches.is_empty(),
            "assignee_match:self must reject tasks assigned to a different agent"
        );

        // A task assigned to the worker (by UUID) MUST match.
        let event_for_me = Event::new(
            delegator,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskPosted {
                task_id: "t-2".to_string(),
                title: "For me".to_string(),
                assigned_to: Some(worker.to_string()),
                created_by: Some(delegator.to_string()),
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&event_for_me, |id| {
            if id == worker {
                Some("worker".to_string())
            } else {
                None
            }
        });
        assert_eq!(
            matches.len(),
            1,
            "assignee_match:self must fire for tasks assigned to the owner by UUID"
        );

        // A task assigned to the worker (by name) MUST also match.
        let event_for_me_by_name = Event::new(
            delegator,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskPosted {
                task_id: "t-3".to_string(),
                title: "For me by name".to_string(),
                assigned_to: Some("worker".to_string()),
                created_by: Some(delegator.to_string()),
            }),
        );
        // Reset cooldown so we can evaluate a second matching event.
        for mut entry in engine.triggers.iter_mut() {
            entry.cooldown_secs = Some(0);
        }
        let (matches, _) = engine.evaluate_with_resolver(&event_for_me_by_name, |id| {
            if id == worker {
                Some("worker".to_string())
            } else {
                None
            }
        });
        assert_eq!(
            matches.len(),
            1,
            "assignee_match:self must accept the owner's display name too"
        );
    }

    #[test]
    fn task_posted_assignee_match_unassigned_matches_none_and_empty_string() {
        // `assignee_match = "unassigned"` is the "pick up unowned work" filter, so it must match BOTH spellings of unowned that the system actually produces: `None` (task_post never set the field) and `""` (the stuck-task sweeper releases a claim with `SET assigned_to = ''`).
        // Missing the empty-string half would make the trigger go permanently quiet for any task that had ever been claimed.
        let engine = TriggerEngine::new();
        let worker = AgentId::new();
        let delegator = AgentId::new();

        engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some("unassigned".to_string()),
                },
                "claim {{event}}".to_string(),
                0,
            )
            .unwrap();

        let resolver = |id: AgentId| {
            if id == worker {
                Some("worker".to_string())
            } else {
                None
            }
        };
        let posted = |task_id: &str, assigned_to: Option<String>| {
            Event::new(
                delegator,
                EventTarget::Broadcast,
                EventPayload::System(SystemEvent::TaskPosted {
                    task_id: task_id.to_string(),
                    title: "Open work".to_string(),
                    assigned_to,
                    created_by: Some(delegator.to_string()),
                }),
            )
        };
        // Each `evaluate_with_resolver` below needs the cooldown cleared, since a previous match would otherwise suppress the next one.
        let reset_cooldown = || {
            for mut entry in engine.triggers.iter_mut() {
                entry.cooldown_secs = Some(0);
            }
        };

        // `None` — no assignee field at all.
        let (matches, _) = engine.evaluate_with_resolver(&posted("t-1", None), resolver);
        assert_eq!(
            matches.len(),
            1,
            "unassigned must fire when assigned_to is absent"
        );

        // `Some("")` — the sweeper-released form.
        // This is the case that requires the `unassigned` arm to sit BEFORE the `candidate == None` early-exit AND to treat the empty string as unowned.
        reset_cooldown();
        let (matches, _) =
            engine.evaluate_with_resolver(&posted("t-2", Some(String::new())), resolver);
        assert_eq!(
            matches.len(),
            1,
            "unassigned must fire when assigned_to is the empty string the sweeper writes"
        );

        // An addressed task must NOT match, whether addressed to the trigger owner or anyone else — "unassigned" is not a wildcard.
        reset_cooldown();
        let (matches, _) =
            engine.evaluate_with_resolver(&posted("t-3", Some(worker.to_string())), resolver);
        assert!(
            matches.is_empty(),
            "unassigned must reject a task addressed to the trigger owner"
        );

        reset_cooldown();
        let (matches, _) =
            engine.evaluate_with_resolver(&posted("t-4", Some(delegator.to_string())), resolver);
        assert!(
            matches.is_empty(),
            "unassigned must reject a task addressed to another agent"
        );
    }

    #[test]
    fn task_claimed_creator_match_self_filters_by_uuid_and_name() {
        // #5960 — `{"task_claimed":{"creator_match":"self"}}` must only fire for
        // claims of tasks the trigger-owning (orchestrator) agent originally
        // posted, mirroring `TaskPosted`'s `assignee_match:self`.
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        let worker = AgentId::new();

        engine
            .register(
                orchestrator,
                TriggerPattern::TaskClaimed {
                    creator_match: Some("self".to_string()),
                },
                "notify the user about {{event}}".to_string(),
                0,
            )
            .unwrap();

        // A claim of a task posted by someone else must NOT match.
        let claimed_other = Event::new(
            worker,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskClaimed {
                task_id: "t-1".to_string(),
                claimed_by: worker.to_string(),
                created_by: Some(worker.to_string()),
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&claimed_other, |id| {
            if id == orchestrator {
                Some("orchestrator".to_string())
            } else {
                None
            }
        });
        assert!(
            matches.is_empty(),
            "creator_match:self must reject claims of tasks posted by another agent"
        );

        // A claim of a task the orchestrator posted (creator by UUID) MUST match.
        let claimed_mine = Event::new(
            worker,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskClaimed {
                task_id: "t-2".to_string(),
                claimed_by: worker.to_string(),
                created_by: Some(orchestrator.to_string()),
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&claimed_mine, |id| {
            if id == orchestrator {
                Some("orchestrator".to_string())
            } else {
                None
            }
        });
        assert_eq!(
            matches.len(),
            1,
            "creator_match:self must fire for claims of tasks the owner posted (by UUID)"
        );

        // Creator by display name MUST also match.
        let claimed_mine_by_name = Event::new(
            worker,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskClaimed {
                task_id: "t-3".to_string(),
                claimed_by: worker.to_string(),
                created_by: Some("orchestrator".to_string()),
            }),
        );
        for mut entry in engine.triggers.iter_mut() {
            entry.cooldown_secs = Some(0);
        }
        let (matches, _) = engine.evaluate_with_resolver(&claimed_mine_by_name, |id| {
            if id == orchestrator {
                Some("orchestrator".to_string())
            } else {
                None
            }
        });
        assert_eq!(
            matches.len(),
            1,
            "creator_match:self must accept the owner's display name too"
        );
    }

    #[test]
    fn task_claimed_creator_match_unassigned_matches_none_and_empty_string() {
        // `creator_match = "unassigned"` is accepted on `TaskClaimed` because the identity
        // filter is shared with `TaskPosted`'s `assignee_match`, and here means "no recorded
        // creator" — either an absent `created_by` or the empty string a legacy writer left behind.
        // This mirrors `task_posted_assignee_match_unassigned_matches_none_and_empty_string`
        // but exercises the `TaskClaimed` variant directly, since the shared helper being
        // correct for `TaskPosted` does not prove it is wired correctly for this variant too.
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        let other = AgentId::new();

        engine
            .register(
                owner,
                TriggerPattern::TaskClaimed {
                    creator_match: Some("unassigned".to_string()),
                },
                "notify {{event}}".to_string(),
                0,
            )
            .unwrap();

        let claimed = |task_id: &str, created_by: Option<String>| {
            Event::new(
                other,
                EventTarget::Broadcast,
                EventPayload::System(SystemEvent::TaskClaimed {
                    task_id: task_id.to_string(),
                    claimed_by: other.to_string(),
                    created_by,
                }),
            )
        };
        let reset_cooldown = || {
            for mut entry in engine.triggers.iter_mut() {
                entry.cooldown_secs = Some(0);
            }
        };

        let (matches, _) = engine.evaluate_with_resolver(&claimed("t-1", None), |_| None);
        assert_eq!(
            matches.len(),
            1,
            "creator_match:unassigned must fire when created_by is absent"
        );

        reset_cooldown();
        let (matches, _) =
            engine.evaluate_with_resolver(&claimed("t-2", Some(String::new())), |_| None);
        assert_eq!(
            matches.len(),
            1,
            "creator_match:unassigned must fire when created_by is the empty string"
        );

        reset_cooldown();
        let (matches, _) =
            engine.evaluate_with_resolver(&claimed("t-3", Some(owner.to_string())), |_| None);
        assert!(
            matches.is_empty(),
            "creator_match:unassigned must reject a claim whose task has a recorded creator"
        );
    }

    #[test]
    fn task_completed_creator_match_unassigned_matches_none_and_empty_string() {
        // Same gap as `TaskClaimed` above: `creator_match = "unassigned"` reaches
        // `TaskCompleted` through the same shared helper and needs its own direct coverage.
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        let other = AgentId::new();

        engine
            .register(
                owner,
                TriggerPattern::TaskCompleted {
                    creator_match: Some("unassigned".to_string()),
                },
                "notify {{event}}".to_string(),
                0,
            )
            .unwrap();

        let completed = |task_id: &str, created_by: Option<String>| {
            Event::new(
                other,
                EventTarget::Broadcast,
                EventPayload::System(SystemEvent::TaskCompleted {
                    task_id: task_id.to_string(),
                    completed_by: other.to_string(),
                    result: "done".to_string(),
                    created_by,
                }),
            )
        };
        let reset_cooldown = || {
            for mut entry in engine.triggers.iter_mut() {
                entry.cooldown_secs = Some(0);
            }
        };

        let (matches, _) = engine.evaluate_with_resolver(&completed("t-1", None), |_| None);
        assert_eq!(
            matches.len(),
            1,
            "creator_match:unassigned must fire when created_by is absent"
        );

        reset_cooldown();
        let (matches, _) =
            engine.evaluate_with_resolver(&completed("t-2", Some(String::new())), |_| None);
        assert_eq!(
            matches.len(),
            1,
            "creator_match:unassigned must fire when created_by is the empty string"
        );

        reset_cooldown();
        let (matches, _) =
            engine.evaluate_with_resolver(&completed("t-3", Some(owner.to_string())), |_| None);
        assert!(
            matches.is_empty(),
            "creator_match:unassigned must reject a completion whose task has a recorded creator"
        );
    }

    #[test]
    fn task_completed_creator_match_explicit_uuid_filters() {
        // #5960 — an explicit `creator_match` UUID on `TaskCompleted` scopes the
        // fire to completions of tasks that specific agent posted.
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        let worker = AgentId::new();

        engine
            .register(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: Some(orchestrator.to_string()),
                },
                "report {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Completion of a task posted by another agent must NOT match.
        let completed_other = Event::new(
            worker,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskCompleted {
                task_id: "t-1".to_string(),
                completed_by: worker.to_string(),
                result: "done".to_string(),
                created_by: Some(worker.to_string()),
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&completed_other, |_| None);
        assert!(
            matches.is_empty(),
            "explicit creator_match must reject completions of tasks posted by another agent"
        );

        // Completion of a task the orchestrator posted MUST match.
        let completed_mine = Event::new(
            worker,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskCompleted {
                task_id: "t-2".to_string(),
                completed_by: worker.to_string(),
                result: "done".to_string(),
                created_by: Some(orchestrator.to_string()),
            }),
        );
        for mut entry in engine.triggers.iter_mut() {
            entry.cooldown_secs = Some(0);
        }
        let (matches, _) = engine.evaluate_with_resolver(&completed_mine, |_| None);
        assert_eq!(
            matches.len(),
            1,
            "explicit creator_match must fire for completions of the matching poster's task"
        );
    }

    #[test]
    fn task_trigger_creator_match_none_fires_for_all() {
        // #5960 — `creator_match: None` preserves the legacy fire-for-all
        // behaviour for both task-board patterns.
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        let other = AgentId::new();

        engine
            .register(
                owner,
                TriggerPattern::TaskClaimed {
                    creator_match: None,
                },
                "notify {{event}}".to_string(),
                0,
            )
            .unwrap();

        let claimed_by_unrelated = Event::new(
            other,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskClaimed {
                task_id: "t-1".to_string(),
                claimed_by: other.to_string(),
                created_by: Some(other.to_string()),
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&claimed_by_unrelated, |_| None);
        assert_eq!(
            matches.len(),
            1,
            "creator_match:None must fire for every claim regardless of who posted the task"
        );
    }

    // -- session_mode_override propagation (#3754) ---------------------------------

    /// Per-trigger `session_mode: Some(New)` must surface as `Some(New)` on
    /// every `TriggerMatch` produced by that trigger — the dispatcher uses this
    /// to materialise a fresh `SessionId` instead of reusing the canonical one.
    #[test]
    fn session_mode_new_override_propagates_to_trigger_match() {
        use librefang_types::agent::SessionMode;

        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let tid = engine
            .register_with_target(
                agent_id,
                TriggerPattern::All,
                "event: {{event}}".to_string(),
                0,
                None,
                Some(0), // zero cooldown so the trigger fires immediately on every evaluation
                Some(SessionMode::New),
                None,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);

        assert_eq!(matches.len(), 1, "trigger must fire");
        assert_eq!(
            matches[0].session_mode_override,
            Some(SessionMode::New),
            "session_mode_override must be Some(New) when trigger carries session_mode = New"
        );

        // Verify the field is preserved through the full round-trip: take the trigger,
        // restore it under a new agent id, and check it still fires with the same override.
        let taken = engine.take_agent_triggers(agent_id);
        let new_agent = AgentId::new();
        engine.restore_triggers(new_agent, taken);

        let (matches2, _) = engine.evaluate(&event);
        assert_eq!(matches2.len(), 1, "restored trigger must still fire");
        assert_eq!(
            matches2[0].session_mode_override,
            Some(SessionMode::New),
            "session_mode_override must survive take/restore"
        );

        // The trigger should also survive a patch that touches other fields.
        let restored_triggers = engine.list_agent_triggers(new_agent);
        let restored_id = restored_triggers[0].id;
        engine.update(
            restored_id,
            TriggerPatch {
                prompt_template: Some("updated: {{event}}".to_string()),
                ..Default::default()
            },
        );
        let after_patch = engine.get_trigger(restored_id).unwrap();
        assert_eq!(
            after_patch.session_mode,
            Some(SessionMode::New),
            "session_mode must not be touched by a patch that only changes prompt_template"
        );

        let _ = tid; // referenced above
    }

    /// Per-trigger `session_mode: Some(Persistent)` must produce
    /// `session_mode_override = Some(Persistent)` — an explicit override wins
    /// even if the value matches the default.
    #[test]
    fn session_mode_persistent_override_propagates_to_trigger_match() {
        use librefang_types::agent::SessionMode;

        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine
            .register_with_target(
                agent_id,
                TriggerPattern::All,
                "event: {{event}}".to_string(),
                0,
                None,
                Some(0),
                Some(SessionMode::Persistent),
                None,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].session_mode_override,
            Some(SessionMode::Persistent),
            "session_mode_override must be Some(Persistent) for an explicit Persistent trigger"
        );
    }

    /// When `Trigger.session_mode` is `None` the dispatcher falls back to the
    /// agent manifest default.  The trigger engine's job is solely to surface
    /// `None` on `TriggerMatch.session_mode_override` — the actual resolution
    /// (`None` → manifest default) happens in the kernel dispatch loop.
    #[test]
    fn session_mode_none_trigger_yields_none_override() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine
            .register_with_target(
                agent_id,
                TriggerPattern::All,
                "event: {{event}}".to_string(),
                0,
                None,
                Some(0),
                None, // no per-trigger session mode
                None,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let (matches, _) = engine.evaluate(&event);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].session_mode_override, None,
            "session_mode_override must be None when the trigger has no override; \
             the dispatcher should then fall back to the agent manifest default"
        );
    }

    /// Model the dispatcher's `effective_mode = mode_override.unwrap_or(manifest_mode)`
    /// resolution inline so we have a named regression test pinned to exactly
    /// the four documented cases without needing a full kernel.
    #[test]
    fn session_mode_resolution_order_per_trigger_over_manifest() {
        use librefang_types::agent::SessionMode;

        // Helper that mimics the single line in the kernel dispatch loop.
        let resolve = |trigger_override: Option<SessionMode>,
                       manifest: SessionMode|
         -> SessionMode { trigger_override.unwrap_or(manifest) };

        // Case 1: trigger override = New → New regardless of manifest
        assert_eq!(
            resolve(Some(SessionMode::New), SessionMode::Persistent),
            SessionMode::New,
            "per-trigger New must beat manifest Persistent"
        );

        // Case 2: trigger override = Persistent → Persistent regardless of manifest
        assert_eq!(
            resolve(Some(SessionMode::Persistent), SessionMode::New),
            SessionMode::Persistent,
            "per-trigger Persistent must beat manifest New"
        );

        // Case 3: no trigger override → fall through to manifest New
        assert_eq!(
            resolve(None, SessionMode::New),
            SessionMode::New,
            "absent override must yield manifest New"
        );

        // Case 4: no trigger override → fall through to manifest Persistent
        assert_eq!(
            resolve(None, SessionMode::Persistent),
            SessionMode::Persistent,
            "absent override must yield manifest Persistent"
        );
    }

    /// `update()` with `session_mode: Some(None)` must clear the per-trigger
    /// session mode override (revert to inheriting the manifest default).
    #[test]
    fn patch_session_mode_some_none_clears_override() {
        use librefang_types::agent::SessionMode;

        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let tid = engine
            .register_with_target(
                agent_id,
                TriggerPattern::All,
                "event: {{event}}".to_string(),
                0,
                None,
                Some(0),
                Some(SessionMode::New),
                None,
            )
            .unwrap();

        // Sanity: override is present before the patch.
        assert_eq!(
            engine.get_trigger(tid).unwrap().session_mode,
            Some(SessionMode::New)
        );

        // Clear the override.
        engine.update(
            tid,
            TriggerPatch {
                session_mode: Some(None),
                ..Default::default()
            },
        );

        assert_eq!(
            engine.get_trigger(tid).unwrap().session_mode,
            None,
            "patching session_mode = Some(None) must clear the per-trigger override"
        );
    }

    // -- cooldown persistence across restarts (#3779) -------------------------

    /// Verify that `last_fired_at` survives a persist → load round-trip so
    /// that cooldown windows are honoured after a daemon restart.
    #[test]
    fn test_cooldown_state_survives_persist_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let persist_path = dir.path().join("trigger_jobs.json");

        // ── Session 1: fire a trigger and persist ──────────────────────────
        let engine1 = TriggerEngine {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            last_fired: DashMap::new(),
            max_triggers_per_event: DEFAULT_MAX_TRIGGERS_PER_EVENT,
            default_cooldown_secs: DEFAULT_COOLDOWN_SECS,
            persist_path: Some(persist_path.clone()),
            persist_lock: std::sync::Mutex::new(()),
        };
        let agent_id = AgentId::new();
        // Register with a 60-second cooldown so it won't expire during the test.
        let tid = engine1
            .register_with_target(
                agent_id,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                0,
                None,
                Some(60),
                None,
                None,
            )
            .unwrap();

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // Fire once to set last_fired
        let (matches, _) = engine1.evaluate(&event);
        assert_eq!(matches.len(), 1, "First fire must succeed");
        assert!(engine1.last_fired.contains_key(&(tid, None)));

        // Persist (stamps last_fired_at into the trigger JSON)
        engine1.persist().unwrap();

        // ── Session 2: load and verify cooldown is still active ────────────
        let engine2 = TriggerEngine {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            last_fired: DashMap::new(),
            max_triggers_per_event: DEFAULT_MAX_TRIGGERS_PER_EVENT,
            default_cooldown_secs: DEFAULT_COOLDOWN_SECS,
            persist_path: Some(persist_path),
            persist_lock: std::sync::Mutex::new(()),
        };
        let loaded = engine2.load().unwrap();
        assert_eq!(loaded, 1, "Should have loaded exactly one trigger");

        // The loaded trigger must have last_fired populated from last_fired_at
        let triggers = engine2.list_all();
        assert_eq!(triggers.len(), 1);
        assert!(
            triggers[0].last_fired_at.is_some(),
            "last_fired_at must be persisted"
        );

        // The cooldown must still be active — the trigger should NOT fire again
        let (matches2, _) = engine2.evaluate(&event);
        assert_eq!(
            matches2.len(),
            0,
            "Cooldown must be honoured after loading persisted state"
        );
    }

    // -- reconcile_manifest_triggers (#5014) ------------------------------------

    use librefang_types::agent::{ManifestTrigger, OrphanPolicy};

    fn mt(prompt: &str, max_fires: u64, enabled: bool) -> ManifestTrigger {
        ManifestTrigger {
            // `All` is a unit variant — serde uses the bare string form.
            pattern: serde_json::Value::String("all".to_string()),
            prompt_template: prompt.to_string(),
            max_fires,
            cooldown_secs: 0,
            session_mode: None,
            target_agent: None,
            workflow_id: None,
            enabled,
        }
    }

    #[test]
    fn reconcile_creates_missing_triggers() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        let manifest = vec![
            mt("alpha {{event}}", 0, true),
            mt("beta {{event}}", 7, true),
        ];

        let report =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert_eq!(report.created, 2);
        assert_eq!(report.updated, 0);
        assert_eq!(report.deleted, 0);
        assert_eq!(report.orphans_kept, 0);
        assert!(report.mutated());

        let listed = engine.list_agent_triggers(agent);
        assert_eq!(listed.len(), 2);
        // `beta` got its non-default max_fires.
        let beta = listed
            .iter()
            .find(|t| t.prompt_template == "beta {{event}}")
            .expect("beta trigger must be present");
        assert_eq!(beta.max_fires, 7);
    }

    #[test]
    fn reconcile_is_idempotent_second_run_is_noop() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        let manifest = vec![mt("alpha {{event}}", 0, true)];

        let first =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert_eq!(first.created, 1);

        let second =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert!(
            !second.mutated(),
            "second reconcile with identical inputs must be a no-op, got {second:?}"
        );
        assert_eq!(second.created, 0);
        assert_eq!(second.updated, 0);
    }

    #[test]
    fn reconcile_updates_mutable_fields_toml_wins() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();

        // First reconcile: seed the trigger with enabled=true, max_fires=0.
        let manifest_v1 = vec![mt("alpha {{event}}", 0, true)];
        engine.reconcile_manifest_triggers(agent, &manifest_v1, OrphanPolicy::Keep, |_| None);

        // Second reconcile: same pattern + prompt, but max_fires=5 and disabled.
        let mut manifest_v2 = manifest_v1.clone();
        manifest_v2[0].max_fires = 5;
        manifest_v2[0].enabled = false;
        manifest_v2[0].cooldown_secs = 30;

        let report =
            engine.reconcile_manifest_triggers(agent, &manifest_v2, OrphanPolicy::Keep, |_| None);
        assert_eq!(report.created, 0);
        assert_eq!(report.updated, 1);
        assert_eq!(report.deleted, 0);

        let triggers = engine.list_agent_triggers(agent);
        assert_eq!(triggers.len(), 1);
        let t = &triggers[0];
        assert_eq!(t.max_fires, 5);
        assert!(!t.enabled);
        assert_eq!(t.cooldown_secs, Some(30));
    }

    #[test]
    fn reconcile_orphan_keep_preserves_runtime_triggers() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();

        // Register a runtime-only trigger.
        let runtime_id = engine
            .register(
                agent,
                TriggerPattern::Lifecycle,
                "runtime {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Empty manifest, Keep policy → orphan survives.
        let report = engine.reconcile_manifest_triggers(agent, &[], OrphanPolicy::Keep, |_| None);
        assert_eq!(report.created, 0);
        assert_eq!(report.updated, 0);
        assert_eq!(report.deleted, 0);
        assert_eq!(report.orphans_kept, 1);
        assert!(!report.mutated());
        assert!(engine.get(runtime_id).is_some());
    }

    #[test]
    fn reconcile_orphan_warn_preserves_runtime_triggers() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();

        let runtime_id = engine
            .register(
                agent,
                TriggerPattern::Lifecycle,
                "runtime {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Empty manifest, Warn policy → orphan kept, no delete.
        let report = engine.reconcile_manifest_triggers(agent, &[], OrphanPolicy::Warn, |_| None);
        assert_eq!(report.deleted, 0);
        assert_eq!(report.orphans_kept, 1);
        assert!(engine.get(runtime_id).is_some());
    }

    #[test]
    fn reconcile_orphan_delete_removes_runtime_triggers() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();

        let runtime_id = engine
            .register(
                agent,
                TriggerPattern::Lifecycle,
                "runtime {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Empty manifest, Delete policy → orphan removed.
        let report = engine.reconcile_manifest_triggers(agent, &[], OrphanPolicy::Delete, |_| None);
        assert_eq!(report.deleted, 1);
        assert_eq!(report.orphans_kept, 0);
        assert!(report.mutated());
        assert!(engine.get(runtime_id).is_none());
    }

    #[test]
    fn reconcile_target_agent_name_resolves_via_closure() {
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        let target = AgentId::new();

        let mut manifest_entry = mt("notify {{event}}", 0, true);
        manifest_entry.target_agent = Some("downstream".to_string());

        let report = engine.reconcile_manifest_triggers(
            owner,
            std::slice::from_ref(&manifest_entry),
            OrphanPolicy::Keep,
            |name| {
                if name == "downstream" {
                    Some(target)
                } else {
                    None
                }
            },
        );
        assert_eq!(report.created, 1);

        let triggers = engine.list_agent_triggers(owner);
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers[0].target_agent, Some(target));
    }

    #[test]
    fn reconcile_unresolvable_target_logs_and_registers_without_target() {
        let engine = TriggerEngine::new();
        let owner = AgentId::new();

        let mut manifest_entry = mt("notify {{event}}", 0, true);
        manifest_entry.target_agent = Some("nope".to_string());

        let report = engine.reconcile_manifest_triggers(
            owner,
            std::slice::from_ref(&manifest_entry),
            OrphanPolicy::Keep,
            |_| None,
        );
        assert_eq!(report.created, 1);

        let triggers = engine.list_agent_triggers(owner);
        assert!(triggers[0].target_agent.is_none());
    }

    #[test]
    fn reconcile_skips_invalid_pattern_continues_with_rest() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();

        let manifest = vec![
            ManifestTrigger {
                pattern: serde_json::json!({ "bogus_variant": {} }),
                prompt_template: "x".to_string(),
                ..Default::default()
            },
            mt("good {{event}}", 0, true),
        ];

        let report =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.created, 1);
        assert_eq!(engine.list_agent_triggers(agent).len(), 1);
    }

    #[test]
    fn reconcile_string_form_task_posted_normalises_to_struct() {
        // Legacy operators sometimes write `pattern = "task_posted"`. The
        // normalisation helper should turn the bare string into the struct
        // form so it deserialises like the API.
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        let manifest = vec![ManifestTrigger {
            pattern: serde_json::Value::String("task_posted".to_string()),
            prompt_template: "task: {{event}}".to_string(),
            ..Default::default()
        }];

        let report =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert_eq!(report.created, 1);

        let triggers = engine.list_agent_triggers(agent);
        assert!(matches!(
            triggers[0].pattern,
            TriggerPattern::TaskPosted { .. }
        ));
    }

    #[test]
    fn reconcile_disabled_manifest_trigger_persists_disabled() {
        // A new entry with `enabled = false` must end up disabled in the
        // store. The reconcile path routes through
        // `register_with_target_enabled` so the trigger is born disabled
        // (no register-then-patch race window).
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        let manifest = vec![mt("muted {{event}}", 0, false)];

        let report =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert_eq!(report.created, 1);

        let triggers = engine.list_agent_triggers(agent);
        assert_eq!(triggers.len(), 1);
        assert!(!triggers[0].enabled, "manifest enabled=false must stick");
    }

    #[test]
    fn reconcile_duplicate_manifest_entries_create_one_runtime_trigger_each() {
        // Two identical `[[triggers]]` blocks in the manifest. The first
        // entry has no prior runtime match and is registered fresh; the
        // second cannot claim the trigger the first one just created (it
        // is already in `claimed`), so it falls through to the `None`
        // arm and registers its own copy. Net: 2 manifest entries → 2
        // runtime triggers.
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        let dup = mt("identical {{event}}", 3, true);
        let manifest = vec![dup.clone(), dup.clone()];

        let first =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert_eq!(first.created, 2, "two duplicate entries → two creates");
        assert_eq!(first.updated, 0);
        assert_eq!(first.deleted, 0);
        assert_eq!(first.orphans_kept, 0);

        let triggers = engine.list_agent_triggers(agent);
        assert_eq!(triggers.len(), 2, "two runtime triggers must exist");
        for t in &triggers {
            assert_eq!(t.prompt_template, "identical {{event}}");
            assert_eq!(t.max_fires, 3);
            assert!(t.enabled);
        }

        // Second reconcile against the same manifest must be idempotent:
        // entry #1 claims trigger A, entry #2 claims trigger B (because
        // A is already claimed), and neither needs an update.
        let second =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);
        assert!(
            !second.mutated(),
            "re-reconcile against duplicate manifest must be a no-op, got {second:?}"
        );
        assert_eq!(second.created, 0);
        assert_eq!(second.updated, 0);
        assert_eq!(second.deleted, 0);
        assert_eq!(second.orphans_kept, 0);
        assert_eq!(engine.list_agent_triggers(agent).len(), 2);
    }

    // ── Per-agent cap (audit: trigger-engine-no-per-agent-cap) ──────

    /// Registering up to `MAX_TRIGGERS_PER_AGENT` for a single agent
    /// succeeds; the (cap + 1)th registration returns `Err` and the
    /// error carries agent_id + current_count + max so the operator
    /// can act on it. Cap-refused registration must NOT mutate the
    /// runtime store — partial writes would break the contract.
    #[test]
    fn register_refuses_past_max_per_agent_cap() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        for i in 0..MAX_TRIGGERS_PER_AGENT {
            engine
                .register(agent, TriggerPattern::All, format!("p{i}"), 0)
                .unwrap_or_else(|e| panic!("register #{i} must succeed below cap; got {e:?}"));
        }
        assert_eq!(
            engine.list_agent_triggers(agent).len(),
            MAX_TRIGGERS_PER_AGENT,
        );

        let err = engine
            .register(agent, TriggerPattern::All, "over".to_string(), 0)
            .expect_err("register past cap must return Err");
        assert_eq!(err.agent_id, agent);
        assert_eq!(err.current_count, MAX_TRIGGERS_PER_AGENT);
        assert_eq!(err.max, MAX_TRIGGERS_PER_AGENT);

        assert_eq!(
            engine.list_agent_triggers(agent).len(),
            MAX_TRIGGERS_PER_AGENT,
            "refused register must leave the agent's trigger count unchanged",
        );
    }

    /// The cap is per-agent: agent A hitting it must not affect
    /// agent B's headroom.
    #[test]
    fn cap_is_per_agent_not_global() {
        let engine = TriggerEngine::new();
        let a = AgentId::new();
        let b = AgentId::new();
        for i in 0..MAX_TRIGGERS_PER_AGENT {
            engine
                .register(a, TriggerPattern::All, format!("a{i}"), 0)
                .unwrap();
        }
        assert!(engine
            .register(a, TriggerPattern::All, "a-over".to_string(), 0)
            .is_err());
        // Agent B has its own headroom — first register succeeds.
        engine
            .register(b, TriggerPattern::All, "b-first".to_string(), 0)
            .expect("agent B has its own headroom");
        assert_eq!(engine.list_agent_triggers(b).len(), 1);
    }

    /// `reconcile_manifest_triggers` reports cap-exceeded entries
    /// as a counted field on `ReconcileReport` (not silent drop)
    /// so the caller can surface the truncation. Existing triggers
    /// matching the manifest are still updated; only the
    /// over-the-cap NEW entries are refused.
    #[test]
    fn reconcile_counts_cap_exceeded_into_report() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();

        // Pre-seed the agent to the cap from the runtime side, so
        // any new manifest entry is over-the-cap.
        for i in 0..MAX_TRIGGERS_PER_AGENT {
            engine
                .register(agent, TriggerPattern::All, format!("seed{i}"), 0)
                .unwrap();
        }

        // Three NEW manifest entries with a DIFFERENT pattern from
        // the seeds — they can't match-and-update an existing
        // runtime trigger, so each falls through to the `None` arm
        // and trips the cap. `mt(...)` uses `"all"` so we hand-roll
        // entries with `"system"` to avoid the pattern match.
        let make_trigger = |idx: usize| ManifestTrigger {
            pattern: serde_json::Value::String("system".to_string()),
            prompt_template: format!("manifest{idx}"),
            max_fires: 0,
            cooldown_secs: 0,
            session_mode: None,
            target_agent: None,
            workflow_id: None,
            enabled: true,
        };
        let manifest = vec![make_trigger(0), make_trigger(1), make_trigger(2)];

        let report =
            engine.reconcile_manifest_triggers(agent, &manifest, OrphanPolicy::Keep, |_| None);

        assert_eq!(
            report.created, 0,
            "no new manifest trigger should slip past the cap"
        );
        assert_eq!(
            report.cap_exceeded, 3,
            "all three over-cap entries must be counted",
        );
        assert_eq!(
            engine.list_agent_triggers(agent).len(),
            MAX_TRIGGERS_PER_AGENT,
            "cap-refused entries must not bump the agent's trigger count",
        );
    }

    // -- task_posted_coverage_for (#6728) ---------------------------------------

    /// Naming the assignee by UUID or by display name has to reach the same
    /// verdict: the substrate stores `assigned_to` in either form and
    /// `task_claim` matches both, so a coverage rule that only understood one
    /// would wake an agent that already has a trigger (double wake) or stay
    /// silent for one that does not.
    #[test]
    fn coverage_accepts_the_assignee_by_uuid_and_by_name() {
        let engine = TriggerEngine::new();
        let worker = AgentId::new();
        let by_uuid = engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some(worker.to_string()),
                },
                "claim".to_string(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::Covered(by_uuid),
        );

        let engine = TriggerEngine::new();
        let by_name = engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some("worker".to_string()),
                },
                "claim".to_string(),
                0,
            )
            .unwrap();
        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::Covered(by_name),
        );
    }

    /// `"self"` resolves against the trigger owner, so it covers the owner and
    /// nobody else.
    #[test]
    fn coverage_resolves_self_against_the_owner() {
        let engine = TriggerEngine::new();
        let worker = AgentId::new();
        let bystander = AgentId::new();
        let id = engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some("self".to_string()),
                },
                "claim".to_string(),
                0,
            )
            .unwrap();

        let resolver = |id: AgentId| {
            if id == worker {
                Some("worker".to_string())
            } else {
                None
            }
        };
        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), resolver),
            TaskPostedCoverage::Covered(id),
        );
        assert_eq!(
            engine.task_posted_coverage_for(bystander, Some("bystander"), resolver),
            TaskPostedCoverage::None,
            "another agent's self-trigger must not count as coverage",
        );
    }

    /// An orchestrator watching the whole board is not a wake path for the
    /// worker: the match fires, but it dispatches to the orchestrator's own
    /// session. Counting it would leave the assignee exactly as unreachable
    /// as before while looking covered.
    #[test]
    fn coverage_ignores_an_observer_that_dispatches_elsewhere() {
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        let worker = AgentId::new();
        engine
            .register(
                orchestrator,
                TriggerPattern::TaskPosted {
                    assignee_match: None,
                },
                "notify the human about {{event}}".to_string(),
                0,
            )
            .unwrap();

        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::None,
        );
        assert_eq!(
            engine.task_posted_coverage_for(orchestrator, Some("orchestrator"), |_| None),
            TaskPostedCoverage::Covered(engine.list_all()[0].id),
            "the unfiltered observer does cover its own owner",
        );
    }

    /// The owner-keyed `agent_triggers` index cannot answer this: an
    /// orchestrator-owned trigger that routes to the worker via `target_agent`
    /// is real coverage, and missing it would double-wake the worker on every
    /// post.
    #[test]
    fn coverage_finds_a_trigger_targeted_at_the_assignee() {
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        let worker = AgentId::new();
        let id = engine
            .register_with_target(
                orchestrator,
                TriggerPattern::TaskPosted {
                    assignee_match: Some(worker.to_string()),
                },
                "wake the worker".to_string(),
                0,
                Some(worker),
                None,
                None,
                None,
            )
            .unwrap();

        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::Covered(id),
        );
    }

    /// A trigger that cannot fire is a gap to fill, not a decision to stay
    /// silent — the 14h outage in #6728 is what "any record counts" produces.
    /// Both ways a trigger stops firing must report `Dormant` so the built-in
    /// wake takes over and says which record it took over from.
    #[test]
    fn coverage_reports_disabled_and_exhausted_triggers_as_dormant() {
        let engine = TriggerEngine::new();
        let worker = AgentId::new();
        let id = engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some("self".to_string()),
                },
                "claim".to_string(),
                0,
            )
            .unwrap();
        engine.set_enabled(id, false);
        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::Dormant(vec![id]),
        );

        // Fire-exhaustion: max_fires = 1, then burn it.
        let engine = TriggerEngine::new();
        let id = engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some("self".to_string()),
                },
                "claim".to_string(),
                1,
            )
            .unwrap();
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskPosted {
                task_id: "t-1".to_string(),
                title: "burn the budget".to_string(),
                assigned_to: Some(worker.to_string()),
                created_by: None,
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&event, |_| None);
        assert_eq!(matches.len(), 1, "the trigger fires once");
        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::Dormant(vec![id]),
            "a spent max_fires budget leaves the assignee uncovered",
        );
    }

    /// Cooldown is transient dispatch state, not configuration. Treating it as
    /// a gap would make the built-in wake fire alongside a trigger that is
    /// about to fire anyway, which is the double wake the declarative check
    /// exists to avoid.
    #[test]
    fn coverage_holds_while_a_trigger_is_cooling_down() {
        let engine = TriggerEngine::new();
        let worker = AgentId::new();
        let id = engine
            .register(
                worker,
                TriggerPattern::TaskPosted {
                    assignee_match: Some("self".to_string()),
                },
                "claim".to_string(),
                0,
            )
            .unwrap();
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskPosted {
                task_id: "t-1".to_string(),
                title: "first".to_string(),
                assigned_to: Some(worker.to_string()),
                created_by: None,
            }),
        );
        let (matches, _) = engine.evaluate_with_resolver(&event, |_| None);
        assert_eq!(matches.len(), 1);
        // Second evaluation inside the cooldown window produces no match...
        let (matches, _) = engine.evaluate_with_resolver(&event, |_| None);
        assert!(matches.is_empty(), "cooldown suppresses the second fire");
        // ...but the trigger is still coverage.
        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::Covered(id),
        );
    }

    /// Only `TaskPosted` records answer this question — a `TaskClaimed` /
    /// `TaskCompleted` subscription is a notification path, not a wake path.
    #[test]
    fn coverage_ignores_other_task_board_patterns() {
        let engine = TriggerEngine::new();
        let worker = AgentId::new();
        engine
            .register(
                worker,
                TriggerPattern::TaskClaimed {
                    creator_match: None,
                },
                "notify".to_string(),
                0,
            )
            .unwrap();
        engine
            .register(
                worker,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify".to_string(),
                0,
            )
            .unwrap();

        assert_eq!(
            engine.task_posted_coverage_for(worker, Some("worker"), |_| None),
            TaskPostedCoverage::None,
        );
    }

    // -- subject-scoped cooldown (#6756) ---------------------------------------

    fn completion_of(task_id: &str, creator: AgentId) -> Event {
        Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::TaskCompleted {
                task_id: task_id.to_string(),
                completed_by: "worker".to_string(),
                result: format!("result of {task_id}"),
                created_by: Some(creator.to_string()),
            }),
        )
    }

    /// The reported defect, verbatim from the issue: two distinct tasks finishing back to back — what any drain loop produces — must notify twice.
    /// Before this change the second event was discarded, not delayed, and nothing re-announced it.
    #[test]
    fn distinct_task_completions_inside_one_window_both_fire() {
        let engine = TriggerEngine::new(); // default cooldown_secs = 5
        let orchestrator = AgentId::new();
        engine
            .register(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify the human: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let (first, _) =
            engine.evaluate_with_resolver(&completion_of("task-1", orchestrator), |_| None);
        let (second, _) =
            engine.evaluate_with_resolver(&completion_of("task-2", orchestrator), |_| None);

        assert_eq!(first.len(), 1, "first completion notifies");
        assert_eq!(
            second.len(),
            1,
            "a distinct task's completion is a distinct window, not a repeat"
        );
    }

    /// The storm protection the knob exists for still works: the *same* subject arriving twice inside the window is a repeat and is suppressed.
    #[test]
    fn a_repeat_of_the_same_subject_is_still_suppressed() {
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        engine
            .register(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let event = completion_of("task-1", orchestrator);
        let (first, _) = engine.evaluate_with_resolver(&event, |_| None);
        let (again, _) = engine.evaluate_with_resolver(&event, |_| None);

        assert_eq!(first.len(), 1);
        assert!(
            again.is_empty(),
            "the same subject inside the window is exactly what cooldown is for"
        );
    }

    /// A catch-all keeps the rate cap it has today.
    /// Keying its window on the subject would turn "at most once per window" into "once per event" for a trigger whose whole point is a bounded firehose.
    #[test]
    fn a_catch_all_trigger_keeps_its_trigger_wide_window() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine
            .register(
                watcher,
                TriggerPattern::All,
                "saw: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let (first, _) = engine.evaluate_with_resolver(&completion_of("task-1", watcher), |_| None);
        let (second, _) =
            engine.evaluate_with_resolver(&completion_of("task-2", watcher), |_| None);

        assert_eq!(first.len(), 1);
        assert!(
            second.is_empty(),
            "distinct subjects must not widen a catch-all's window"
        );
    }

    /// Memory updates get the same treatment as task-board events: two keys changing inside one window are two facts, not one repeated.
    #[test]
    fn distinct_memory_keys_inside_one_window_both_fire() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        engine
            .register(
                agent,
                TriggerPattern::MemoryKeyPattern {
                    key_pattern: "project/".to_string(),
                },
                "memory changed: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let update = |key: &str| {
            Event::new(
                agent,
                EventTarget::Broadcast,
                EventPayload::MemoryUpdate(librefang_types::event::MemoryDelta {
                    agent_id: agent,
                    key: key.to_string(),
                    operation: librefang_types::event::MemoryOperation::Updated,
                }),
            )
        };

        let (first, _) = engine.evaluate_with_resolver(&update("project/alpha"), |_| None);
        let (second, _) = engine.evaluate_with_resolver(&update("project/beta"), |_| None);

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1, "a different key is a different subject");
    }

    /// `last_fired_at` on disk keeps meaning "when this trigger last fired", so restart recovery and the operator-facing field are unaffected by the window being subject-scoped in memory.
    #[test]
    fn firing_on_a_subject_still_stamps_the_trigger_wide_entry() {
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        let tid = engine
            .register(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let (fired, _) =
            engine.evaluate_with_resolver(&completion_of("task-1", orchestrator), |_| None);
        assert_eq!(fired.len(), 1);

        assert!(
            engine
                .last_fired
                .contains_key(&(tid, Some("task-1".to_string()))),
            "the subject window is what the next check consults"
        );
        assert!(
            engine.last_fired.contains_key(&(tid, None)),
            "the trigger-wide entry backs last_fired_at on disk"
        );
    }

    /// Removing a trigger must take its per-subject windows with it, or a successor registered with the same id space would inherit suppression it never earned.
    #[test]
    fn removing_a_trigger_forgets_every_subject_window() {
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        let tid = engine
            .register(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify: {{event}}".to_string(),
                0,
            )
            .unwrap();
        engine.evaluate_with_resolver(&completion_of("task-1", orchestrator), |_| None);
        engine.evaluate_with_resolver(&completion_of("task-2", orchestrator), |_| None);
        assert!(engine.last_fired.iter().count() >= 2);

        assert!(engine.remove(tid));

        assert_eq!(
            engine
                .last_fired
                .iter()
                .filter(|e| e.key().0 == tid)
                .count(),
            0,
            "no window of a removed trigger may survive"
        );
    }

    /// Two agents appearing a second apart are two events, not one repeated — the same shape as two tasks or two memory keys, since the pattern is a substring filter over a per-event identifier.
    #[test]
    fn distinct_agent_spawns_inside_one_window_both_fire() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine
            .register(
                watcher,
                TriggerPattern::AgentSpawned {
                    name_pattern: "worker".to_string(),
                },
                "a worker appeared: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let spawned = |name: &str| {
            Event::new(
                AgentId::new(),
                EventTarget::Broadcast,
                EventPayload::Lifecycle(LifecycleEvent::Spawned {
                    agent_id: AgentId::new(),
                    name: name.to_string(),
                }),
            )
        };

        let (first, _) = engine.evaluate_with_resolver(&spawned("worker-1"), |_| None);
        let (second, _) = engine.evaluate_with_resolver(&spawned("worker-2"), |_| None);

        assert_eq!(first.len(), 1);
        assert_eq!(
            second.len(),
            1,
            "a second worker spawning is a distinct subject, not a repeat"
        );
    }

    /// Evaluation must survive crossing the prune threshold.
    ///
    /// The prune reads `self.triggers` to find the longest window in use, and the evaluation loop holds a `RefMut` on a trigger's shard while it fires; doing both at once is a same-thread write-then-read on one DashMap shard, which hangs rather than fails.
    /// This walks past the threshold on real evaluations — if the prune ever moves back inside the loop, this test stops returning.
    #[test]
    fn crossing_the_prune_threshold_does_not_wedge_the_evaluator() {
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        engine
            .register(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify: {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Each distinct task id opens its own window, so this grows `last_fired` past PRUNE_THRESHOLD (4096).
        let mut fired = 0usize;
        for i in 0..4_200 {
            let (matches, _) = engine
                .evaluate_with_resolver(&completion_of(&format!("task-{i}"), orchestrator), |_| {
                    None
                });
            fired += matches.len();
        }

        assert_eq!(fired, 4_200, "every distinct subject fires exactly once");
    }

    /// A window older than the longest cooldown any trigger could use can only ever report "elapsed", so keeping it is pure growth.
    /// The trigger-wide entry is exempt: it is bounded by the trigger count and backs `last_fired_at` on disk.
    #[test]
    fn pruning_drops_stale_subject_windows_and_keeps_the_trigger_wide_one() {
        let engine = TriggerEngine::new();
        let orchestrator = AgentId::new();
        let tid = engine
            .register(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify: {{event}}".to_string(),
                0,
            )
            .unwrap();

        // Seed enough stale subject windows to cross the threshold.
        let stale = Utc::now() - chrono::Duration::seconds(3_600);
        for i in 0..4_200 {
            engine
                .last_fired
                .insert((tid, Some(format!("ancient-{i}"))), stale);
        }
        engine.last_fired.insert((tid, None), stale);

        // One real fire triggers the prune after the loop.
        let (matches, _) =
            engine.evaluate_with_resolver(&completion_of("fresh", orchestrator), |_| None);
        assert_eq!(matches.len(), 1);

        assert!(
            engine
                .last_fired
                .iter()
                .filter(|e| e
                    .key()
                    .1
                    .as_deref()
                    .is_some_and(|s| s.starts_with("ancient-")))
                .count()
                == 0,
            "windows that can no longer suppress anything must not accumulate"
        );
        assert!(
            engine.last_fired.contains_key(&(tid, None)),
            "the trigger-wide entry backs last_fired_at and is never pruned"
        );
        assert!(
            engine
                .last_fired
                .contains_key(&(tid, Some("fresh".to_string()))),
            "a window that can still suppress must survive the prune"
        );
    }

    /// `MemoryUpdate` matches exactly what `MemoryKeyPattern { "*" }` matches, so the two must deliver identically.
    /// Scoping one and not the other gave two triggers with the same match set opposite semantics.
    #[test]
    fn bare_memory_update_is_scoped_like_its_filtered_sibling() {
        let engine = TriggerEngine::new();
        let agent = AgentId::new();
        engine
            .register(
                agent,
                TriggerPattern::MemoryUpdate,
                "memory changed: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let update = |key: &str| {
            Event::new(
                agent,
                EventTarget::Broadcast,
                EventPayload::MemoryUpdate(librefang_types::event::MemoryDelta {
                    agent_id: agent,
                    key: key.to_string(),
                    operation: librefang_types::event::MemoryOperation::Updated,
                }),
            )
        };

        let (first, _) = engine.evaluate_with_resolver(&update("project/alpha"), |_| None);
        let (second, _) = engine.evaluate_with_resolver(&update("project/beta"), |_| None);

        assert_eq!(first.len(), 1);
        assert_eq!(
            second.len(),
            1,
            "a batch of memory writes must not collapse into one delivery"
        );
    }

    /// Two agents dying a second apart are two incidents, not one repeated.
    #[test]
    fn distinct_agent_terminations_inside_one_window_both_fire() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine
            .register(
                watcher,
                TriggerPattern::AgentTerminated,
                "an agent died: {{event}}".to_string(),
                0,
            )
            .unwrap();

        let died = || {
            Event::new(
                AgentId::new(),
                EventTarget::Broadcast,
                EventPayload::Lifecycle(LifecycleEvent::Crashed {
                    agent_id: AgentId::new(),
                    error: "boom".to_string(),
                }),
            )
        };

        let (first, _) = engine.evaluate_with_resolver(&died(), |_| None);
        let (second, _) = engine.evaluate_with_resolver(&died(), |_| None);

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1, "a second crash is a second incident");
    }

    /// A `cooldown_secs` large enough to overflow `chrono` must not take the evaluator with it.
    /// The API accepts an unbounded `u64` (`routes/workflows/triggers.rs` neither clamps nor validates it), so the arithmetic is the only thing standing between an operator's typo and a panic on the event-dispatch thread — or, past `i64::MAX`, a negative horizon that prunes every live window and silently disables suppression.
    #[test]
    fn an_absurd_cooldown_neither_panics_nor_wipes_live_windows() {
        for absurd in [u64::MAX, i64::MAX as u64, 10_000_000_000_000_000] {
            let engine = TriggerEngine::new();
            let orchestrator = AgentId::new();
            let tid = engine
                .register_with_target(
                    orchestrator,
                    TriggerPattern::TaskCompleted {
                        creator_match: None,
                    },
                    "notify: {{event}}".to_string(),
                    0,
                    None,
                    Some(absurd),
                    None,
                    None,
                )
                .unwrap();

            // Cross the prune threshold so the horizon arithmetic runs.
            let stale = Utc::now() - chrono::Duration::seconds(3_600);
            for i in 0..4_200 {
                engine
                    .last_fired
                    .insert((tid, Some(format!("seed-{i}"))), stale);
            }

            let (matches, _) =
                engine.evaluate_with_resolver(&completion_of("fresh", orchestrator), |_| None);
            assert_eq!(matches.len(), 1, "cooldown_secs = {absurd} must still fire");
            assert!(
                engine
                    .last_fired
                    .contains_key(&(tid, Some("fresh".to_string()))),
                "cooldown_secs = {absurd} must not prune a window that just opened"
            );
        }
    }

    /// The prune horizon has to be the longest window in use, not any window: a short-cooldown trigger must not evict a long-cooldown trigger's live entries.
    /// Without this, one trigger at 5s would cut another's hour-long window down to five seconds.
    #[test]
    fn the_prune_horizon_respects_the_longest_window_in_use() {
        let engine = TriggerEngine::new();
        let owner = AgentId::new();
        let brief = engine
            .register_with_target(
                owner,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "brief: {{event}}".to_string(),
                0,
                None,
                Some(5),
                None,
                None,
            )
            .unwrap();
        let patient = engine
            .register_with_target(
                owner,
                TriggerPattern::TaskClaimed {
                    creator_match: None,
                },
                "patient: {{event}}".to_string(),
                0,
                None,
                Some(3_600),
                None,
                None,
            )
            .unwrap();

        // A window that is stale for the 5s trigger but live for the 3600s one.
        let middling = Utc::now() - chrono::Duration::seconds(600);
        engine
            .last_fired
            .insert((patient, Some("kept".to_string())), middling);
        for i in 0..4_200 {
            engine.last_fired.insert(
                (brief, Some(format!("ancient-{i}"))),
                Utc::now() - chrono::Duration::seconds(7_200),
            );
        }

        engine.evaluate_with_resolver(&completion_of("trigger-a-prune", owner), |_| None);

        assert!(
            engine
                .last_fired
                .contains_key(&(patient, Some("kept".to_string()))),
            "a window still inside its own trigger's cooldown must survive"
        );
    }

    /// Per-subject windows are in-memory, so a subject-scoped pattern starts a restart with no suppression — including for a subject it fired on moments earlier.
    /// That is a deliberate trade against unbounded growth in `trigger_jobs.json`, and the failure direction is an extra delivery, but it is a real change from the trigger-wide behaviour and is pinned here rather than left to be discovered.
    #[test]
    fn subject_scoped_cooldown_does_not_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trigger_jobs.json");

        let engine = TriggerEngine {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            last_fired: DashMap::new(),
            max_triggers_per_event: DEFAULT_MAX_TRIGGERS_PER_EVENT,
            default_cooldown_secs: DEFAULT_COOLDOWN_SECS,
            persist_path: Some(path.clone()),
            persist_lock: std::sync::Mutex::new(()),
        };
        let orchestrator = AgentId::new();
        engine
            .register_with_target(
                orchestrator,
                TriggerPattern::TaskCompleted {
                    creator_match: None,
                },
                "notify: {{event}}".to_string(),
                0,
                None,
                Some(3_600),
                None,
                None,
            )
            .unwrap();

        let event = completion_of("task-1", orchestrator);
        let (fired, _) = engine.evaluate_with_resolver(&event, |_| None);
        assert_eq!(fired.len(), 1);
        let (suppressed, _) = engine.evaluate_with_resolver(&event, |_| None);
        assert!(
            suppressed.is_empty(),
            "same subject is suppressed in-process"
        );

        engine.persist().unwrap();

        let restarted = TriggerEngine {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
            last_fired: DashMap::new(),
            max_triggers_per_event: DEFAULT_MAX_TRIGGERS_PER_EVENT,
            default_cooldown_secs: DEFAULT_COOLDOWN_SECS,
            persist_path: Some(path),
            persist_lock: std::sync::Mutex::new(()),
        };
        restarted.load().unwrap();
        let (after_restart, _) = restarted.evaluate_with_resolver(&event, |_| None);

        assert_eq!(
            after_restart.len(),
            1,
            "documented trade: the per-subject window is not persisted, so the \
             same subject fires again after a restart"
        );
    }
}
