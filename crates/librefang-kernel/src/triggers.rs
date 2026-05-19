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
    /// - `Some("self")` — only fire for tasks assigned to the trigger-owning
    ///   agent. Accepts both the agent's UUID and its display name.
    /// - `Some("<uuid>"|"<name>")` — only fire for tasks assigned to that
    ///   specific agent.
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
    TaskClaimed,
    /// Match when a task is completed on the Task Board.
    TaskCompleted,
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
    /// Cooldown duration in seconds after this trigger fires before it can fire again.
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
    /// The trigger ID that produced this match, for telemetry.
    pub trigger_id: TriggerId,
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
    /// Per-trigger last fire wall-clock timestamp for cooldown enforcement.
    ///
    /// Uses `DateTime<Utc>` rather than `std::time::Instant` so that the state
    /// can be round-tripped through the `Trigger.last_fired_at` field on disk,
    /// surviving daemon restarts without resetting all cooldown windows.
    last_fired: DashMap<TriggerId, DateTime<Utc>>,
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
    /// Restores `last_fired` state from `Trigger.last_fired_at` so that
    /// cooldown windows survive daemon restarts.
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
            // Restore cooldown state from the persisted last_fired_at timestamp.
            // This ensures that a trigger which fired shortly before a restart
            // still honours its cooldown window after the daemon comes back up.
            if let Some(last_fired_at) = trigger.last_fired_at {
                self.last_fired.insert(id, last_fired_at);
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
        let _guard = self.persist_lock.lock().unwrap_or_else(|e| e.into_inner());
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
                if let Some(ts) = self.last_fired.get(&t.id) {
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

    /// Register a new trigger.
    /// Returns `true` if `agent_id` already has an enabled trigger with this exact pattern.
    /// Used to skip duplicate registration of proactive triggers on restart.
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

    pub fn register(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
    ) -> TriggerId {
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
    ) -> TriggerId {
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
    ) -> TriggerId {
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
        self.agent_triggers.entry(agent_id).or_default().push(id);

        info!(trigger_id = %id, agent_id = %agent_id, ?target_agent, enabled, "Trigger registered");
        id
    }

    /// Convenience: register a cross-agent trigger where the owner's trigger
    /// wakes a different target agent.
    pub fn register_cross_agent_trigger(
        &self,
        owner: AgentId,
        target: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
    ) -> TriggerId {
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
            self.last_fired.remove(&trigger_id);
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
                self.last_fired.remove(&id);
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
                self.last_fired.remove(&id);
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
        // Pattern change means the trigger is logically new — clear any stale cooldown timer.
        if pattern_changed {
            self.last_fired.remove(&id);
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
    /// 1. **Per-trigger cooldown** — after firing, a trigger is suppressed for
    ///    `cooldown_secs` (default `DEFAULT_COOLDOWN_SECS`). Set `cooldown_secs = Some(0)`
    ///    on a trigger to disable its cooldown.
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

            // Check per-trigger cooldown using wall-clock timestamps so that
            // cooldown windows survive daemon restarts.
            let cooldown =
                Duration::from_secs(trigger.cooldown_secs.unwrap_or(self.default_cooldown_secs));
            if !cooldown.is_zero() {
                if let Some(last) = self.last_fired.get(&trigger.id) {
                    // `now - *last` is negative when `*last > now`, which can happen
                    // if the wall clock stepped backwards (NTP correction, manual
                    // adjustment, VM snapshot restore) or if the persisted
                    // `last_fired_at` was imported from a future-dated state.
                    // `to_std()` then errors; the old `unwrap_or(Duration::ZERO)`
                    // collapsed elapsed to 0 and wedged the trigger off until the
                    // wall clock caught up (#5115). Treat the anomaly as
                    // elapsed-exceeded so the trigger fires once: the subsequent
                    // `self.last_fired.insert(trigger.id, now)` below stamps a
                    // sane timestamp and self-heals the entry.
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
                        total_registered = self.triggers.len(),
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
                    trigger_id: trigger.id,
                });
                trigger.fire_count += 1;
                state_mutated = true;
                self.last_fired.insert(trigger.id, now);

                debug!(
                    trigger_id = %trigger.id,
                    owner = %trigger.agent_id,
                    recipient = %recipient,
                    fire_count = trigger.fire_count,
                    "Trigger fired"
                );
            }
        }

        (matches, state_mutated)
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
                    let new_id = self.register_with_target_enabled(
                        agent_id,
                        pattern,
                        mt.prompt_template.clone(),
                        mt.max_fires,
                        target_agent,
                        cooldown_secs,
                        mt.session_mode,
                        workflow_id,
                        mt.enabled,
                    );
                    claimed.insert(new_id);
                    report.created += 1;
                    info!(
                        agent = %agent_id,
                        trigger_id = %new_id,
                        "Registered manifest trigger"
                    );
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
        Some(tag @ "task_posted") => serde_json::json!({ tag: {} }),
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
                match assignee_match {
                    None => true,
                    Some(filter) => {
                        // Empty assigned_to can't match any filter — the task
                        // isn't assigned to anyone, so an assignee_match
                        // predicate is definitionally false.
                        let Some(assigned) = assigned_to else {
                            return false;
                        };
                        match filter.as_str() {
                            "self" => match &owner {
                                Some((id, name)) => {
                                    assigned == &id.to_string()
                                        || name.as_deref() == Some(assigned.as_str())
                                }
                                None => false,
                            },
                            other => assigned == other,
                        }
                    }
                }
            }
            _ => false,
        },
        TriggerPattern::TaskClaimed => matches!(
            event.payload,
            EventPayload::System(SystemEvent::TaskClaimed { .. })
        ),
        TriggerPattern::TaskCompleted => matches!(
            event.payload,
            EventPayload::System(SystemEvent::TaskCompleted { .. })
        ),
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
            } => {
                format!("Task claimed: {task_id} by {claimed_by}")
            }
            SystemEvent::TaskCompleted {
                task_id,
                completed_by,
                result,
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
                        format!("{}...", &s[..300])
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
    fn test_register_trigger() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine.register(
            agent_id,
            TriggerPattern::All,
            "Event occurred: {{event}}".to_string(),
            0,
        );
        assert!(engine.get(id).is_some());
    }

    #[test]
    fn test_evaluate_lifecycle() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine.register(
            watcher,
            TriggerPattern::Lifecycle,
            "Lifecycle: {{event}}".to_string(),
            0,
        );

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
        engine.register(
            watcher,
            TriggerPattern::AgentSpawned {
                name_pattern: "coder".to_string(),
            },
            "Coder spawned: {{event}}".to_string(),
            0,
        );

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
        let tid = engine.register(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            2, // max 2 fires
        );
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
        let id = engine.register(agent_id, TriggerPattern::All, "msg".to_string(), 0);
        assert!(engine.remove(id));
        assert!(engine.get(id).is_none());
    }

    #[test]
    fn test_remove_agent_triggers() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(agent_id, TriggerPattern::All, "a".to_string(), 0);
        engine.register(agent_id, TriggerPattern::System, "b".to_string(), 0);
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 2);

        engine.remove_agent_triggers(agent_id);
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 0);
    }

    #[test]
    fn test_content_match() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "quota".to_string(),
            },
            "Alert: {{event}}".to_string(),
            0,
        );

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
        engine.register(old_agent, TriggerPattern::All, "a".to_string(), 0);
        engine.register(old_agent, TriggerPattern::System, "b".to_string(), 0);

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
        engine.register(agent_a, TriggerPattern::All, "a".to_string(), 0);

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
        engine.register(agent_a, TriggerPattern::All, "a".to_string(), 0);
        engine.register(agent_b, TriggerPattern::System, "b".to_string(), 0);

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
        engine.register(
            old_agent,
            TriggerPattern::ContentMatch {
                substring: "deploy".to_string(),
            },
            "Deploy alert: {{event}}".to_string(),
            5,
        );
        engine.register(old_agent, TriggerPattern::Lifecycle, "lc".to_string(), 0);

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
        let tid = engine.register(old_agent, TriggerPattern::All, "a".to_string(), 0);
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
        engine.register(
            owner,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            0,
        );

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
        engine.register_with_target(
            owner,
            TriggerPattern::All,
            "Cross-wake: {{event}}".to_string(),
            0,
            Some(target),
            None,
            None,
            None,
        );

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
        let tid = engine.register_cross_agent_trigger(
            owner,
            target,
            TriggerPattern::AgentSpawned {
                name_pattern: "worker".to_string(),
            },
            "Worker spawned: {{event}}".to_string(),
        );

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

        engine.register_with_target(
            old_owner,
            TriggerPattern::System,
            "sys: {{event}}".to_string(),
            0,
            Some(target),
            None,
            None,
            None,
        );

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
        engine.register(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            0,
        );

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
        let tid = engine.register(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            0,
        );

        // Simulate a future-dated `last_fired_at` — far enough ahead that
        // the bug's `unwrap_or(Duration::ZERO)` path would suppress every
        // fire for the next hour.
        let future = Utc::now() + chrono::Duration::hours(1);
        engine.last_fired.insert(tid, future);

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
        let stamped = *engine.last_fired.get(&tid).unwrap();
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
        let tid = engine.register(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            0,
        );
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
            let tid = engine.register(
                *agent_id,
                TriggerPattern::All,
                "Event: {{event}}".to_string(),
                0,
            );
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
        let tid = engine.register(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            0,
        );

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // Fire to create a last_fired entry
        engine.evaluate(&event);
        assert!(engine.last_fired.contains_key(&tid));

        // Remove should clean up
        engine.remove(tid);
        assert!(!engine.last_fired.contains_key(&tid));
    }

    #[test]
    fn test_restore_preserves_cooldown_secs() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        let tid = engine.register(old_agent, TriggerPattern::All, "a".to_string(), 0);
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
        engine.register(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "deploy".to_string(),
            },
            "Deploy alert: {{event}}".to_string(),
            0,
        );

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
        engine.register(
            watcher,
            TriggerPattern::MemoryUpdate,
            "Memory changed: {{event}}".to_string(),
            0,
        );

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
        engine.register(
            watcher,
            TriggerPattern::MemoryKeyPattern {
                key_pattern: "user.".to_string(),
            },
            "User memory changed: {{event}}".to_string(),
            0,
        );

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

        engine.register(
            worker,
            TriggerPattern::TaskPosted {
                assignee_match: Some("self".to_string()),
            },
            "claim and work on {{event}}".to_string(),
            0,
        );

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

    // -- session_mode_override propagation (#3754) ---------------------------------

    /// Per-trigger `session_mode: Some(New)` must surface as `Some(New)` on
    /// every `TriggerMatch` produced by that trigger — the dispatcher uses this
    /// to materialise a fresh `SessionId` instead of reusing the canonical one.
    #[test]
    fn session_mode_new_override_propagates_to_trigger_match() {
        use librefang_types::agent::SessionMode;

        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let tid = engine.register_with_target(
            agent_id,
            TriggerPattern::All,
            "event: {{event}}".to_string(),
            0,
            None,
            Some(0), // zero cooldown so the trigger fires immediately on every evaluation
            Some(SessionMode::New),
            None,
        );

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
        engine.register_with_target(
            agent_id,
            TriggerPattern::All,
            "event: {{event}}".to_string(),
            0,
            None,
            Some(0),
            Some(SessionMode::Persistent),
            None,
        );

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
        engine.register_with_target(
            agent_id,
            TriggerPattern::All,
            "event: {{event}}".to_string(),
            0,
            None,
            Some(0),
            None, // no per-trigger session mode
            None,
        );

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
        let tid = engine.register_with_target(
            agent_id,
            TriggerPattern::All,
            "event: {{event}}".to_string(),
            0,
            None,
            Some(0),
            Some(SessionMode::New),
            None,
        );

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
        let tid = engine1.register_with_target(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            0,
            None,
            Some(60),
            None,
            None,
        );

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
        assert!(engine1.last_fired.contains_key(&tid));

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
        let runtime_id = engine.register(
            agent,
            TriggerPattern::Lifecycle,
            "runtime {{event}}".to_string(),
            0,
        );

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

        let runtime_id = engine.register(
            agent,
            TriggerPattern::Lifecycle,
            "runtime {{event}}".to_string(),
            0,
        );

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

        let runtime_id = engine.register(
            agent,
            TriggerPattern::Lifecycle,
            "runtime {{event}}".to_string(),
            0,
        );

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
}
