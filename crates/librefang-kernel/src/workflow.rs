//! Workflow engine — multi-step agent pipeline execution.
//!
//! A workflow defines a sequence of steps where each step routes
//! a task to a specific agent. Steps can:
//! - Pass their output as input to the next step
//! - Run in sequence (pipeline) or in parallel (fan-out)
//! - Conditionally skip based on previous output
//! - Loop until a condition is met
//! - Store outputs in named variables for later reference
//!
//! Workflows are defined as Rust structs or loaded from JSON/TOML files.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use librefang_memory::{WorkflowRunRow, WorkflowStore};
use librefang_types::agent::{AgentId, SessionMode};
use librefang_types::subagent::SubagentContext;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Error type returned by [`WorkflowEngine::cancel_run`].
#[derive(Debug, Clone)]
pub enum CancelRunError {
    /// No run with that id exists in the engine.
    NotFound(WorkflowRunId),
    /// The run is already in a terminal state and cannot be cancelled.
    AlreadyTerminal {
        run_id: WorkflowRunId,
        /// One of `"completed"`, `"failed"`, or `"cancelled"`.
        state: &'static str,
    },
}

impl std::fmt::Display for CancelRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CancelRunError::NotFound(id) => write!(f, "Workflow run not found: {id}"),
            CancelRunError::AlreadyTerminal { run_id, state } => {
                write!(f, "Cannot cancel workflow run {run_id}: already {state}")
            }
        }
    }
}

impl std::error::Error for CancelRunError {}

/// Error type returned by [`WorkflowEngine::pause_run`].
#[derive(Debug, Clone)]
pub enum PauseRunError {
    /// No run with that id exists in the engine.
    NotFound(WorkflowRunId),
    /// The run is already paused. Returns the hash of the existing token so
    /// callers can confirm idempotency without leaking the plaintext token.
    AlreadyPaused {
        run_id: WorkflowRunId,
        resume_token_hash: String,
    },
    /// The run has already finished and cannot be paused.
    AlreadyTerminal {
        run_id: WorkflowRunId,
        /// One of `"completed"`, `"failed"`, or `"cancelled"`.
        state: &'static str,
    },
}

impl std::fmt::Display for PauseRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PauseRunError::NotFound(id) => write!(f, "Workflow run not found: {id}"),
            PauseRunError::AlreadyPaused { run_id, .. } => {
                write!(f, "Workflow run {run_id} is already paused")
            }
            PauseRunError::AlreadyTerminal { run_id, state } => {
                write!(f, "Cannot pause workflow run {run_id}: already {state}")
            }
        }
    }
}

impl std::error::Error for PauseRunError {}

/// Error type returned by [`WorkflowEngine::resume_run`].
#[derive(Debug, Clone)]
pub enum ResumeRunError {
    /// No run with that id exists in the engine.
    NotFound(WorkflowRunId),
    /// The run is not in the `Paused` state.
    NotPaused {
        run_id: WorkflowRunId,
        /// Actual state name, e.g. `"running"`, `"completed"`.
        state: &'static str,
    },
    /// The supplied token does not match the stored hash.
    TokenMismatch { run_id: WorkflowRunId },
    /// The workflow uses DAG dependencies, which do not yet support pause/resume.
    DagUnsupported { run_id: WorkflowRunId },
    /// The resume itself failed (step error, persist error, etc.).
    ExecutionFailed {
        run_id: WorkflowRunId,
        detail: String,
    },
}

impl std::fmt::Display for ResumeRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResumeRunError::NotFound(id) => write!(f, "Workflow run not found: {id}"),
            ResumeRunError::NotPaused { run_id, state } => {
                write!(
                    f,
                    "Cannot resume workflow run {run_id}: state is {state}, expected Paused"
                )
            }
            ResumeRunError::TokenMismatch { run_id } => {
                write!(f, "Resume token mismatch for run {run_id}: presented token does not match stored hash")
            }
            ResumeRunError::DagUnsupported { run_id } => {
                write!(
                    f,
                    "Resuming a DAG workflow run {run_id} is not yet supported"
                )
            }
            ResumeRunError::ExecutionFailed { run_id, detail } => {
                write!(f, "Resume of run {run_id} failed: {detail}")
            }
        }
    }
}

impl std::error::Error for ResumeRunError {}

/// Unique identifier for a workflow definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub Uuid);

impl WorkflowId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a running workflow instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowRunId(pub Uuid);

impl WorkflowRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A workflow definition — a named sequence of steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique identifier.
    #[serde(default)]
    pub id: WorkflowId,
    /// Human-readable name.
    pub name: String,
    /// Description of what this workflow does.
    pub description: String,
    /// The steps in execution order.
    pub steps: Vec<WorkflowStep>,
    /// Created at.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    /// Optional canvas layout data (nodes, edges, positions) for the visual editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<serde_json::Value>,
    /// Maximum wall-clock time for the entire workflow run, in seconds.
    /// `None` means fall back to the kernel-level default
    /// (`KernelConfig::workflow_default_total_timeout_secs`). When that is
    /// also `None` the workflow runs unbounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_timeout_secs: Option<u64>,
}

/// A single step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Step name for logging/display.
    pub name: String,
    /// Which agent to route this step to.
    pub agent: StepAgent,
    /// The prompt template. Use `{{input}}` for previous output, `{{var_name}}` for variables.
    pub prompt_template: String,
    /// Execution mode for this step.
    #[serde(default)]
    pub mode: StepMode,
    /// Maximum time for this step in seconds (default: 120).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Error handling mode for this step (default: Fail).
    #[serde(default)]
    pub error_mode: ErrorMode,
    /// Optional variable name to store this step's output in.
    #[serde(default)]
    pub output_var: Option<String>,
    /// Whether to inject parent workflow context into this step's prompt.
    /// Default is `None`, which defers to the agent's `inherit_parent_context`
    /// setting. Set to `Some(false)` to force disable context injection for
    /// this step regardless of agent config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_context: Option<bool>,
    /// Names of steps this step depends on (for DAG execution).
    /// When non-empty, the workflow engine uses topological ordering
    /// instead of the default sequential/mode-based execution.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Per-step override for the target agent's `SessionMode`.
    ///
    /// Resolution precedence (per CLAUDE.md): per-step override > target
    /// agent manifest `session_mode` > kernel default (`Persistent`).
    ///
    /// - `Some(Persistent)` — reuse the target registry agent's persistent
    ///   `(agent, _)` session, even if its manifest defaults to `New`. Used
    ///   to thread a workflow's step output into an agent's long-running
    ///   context.
    /// - `Some(New)` — mint a fresh `SessionId` for this step's invocation,
    ///   even if the target agent's manifest defaults to `Persistent`. Used
    ///   to isolate a workflow step from any prior agent state.
    /// - `None` (default) — defer to the target agent's manifest.
    ///
    /// Has no effect when the target agent's module is `wasm:`/`python:`
    /// (those modules don't use the session abstraction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<librefang_types::agent::SessionMode>,
}

fn default_timeout() -> u64 {
    120
}

/// How to identify the agent for a step.
///
/// Deserialization accepts THREE on-wire shapes for operator ergonomics
/// (the issue / PR docs use the bare-string form; the kernel and HTTP
/// payloads use the tagged forms):
///
/// 1. Bare string: `agent = "researcher"` → [`StepAgent::ByName`].
/// 2. Tagged object: `{ name = "researcher" }` → [`StepAgent::ByName`].
/// 3. Tagged object: `{ id = "<uuid>" }` → [`StepAgent::ById`].
///
/// Exactly one of `id` / `name` must be present in the tagged form;
/// supplying both or neither is a deserialization error.
///
/// Serialization continues to emit the tagged-object form (`Serialize`
/// derive on the untagged-style enum picks the matching variant cleanly).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum StepAgent {
    /// Reference an agent by UUID.
    ById { id: String },
    /// Reference an agent by name (first match).
    ByName { name: String },
}

impl<'de> Deserialize<'de> for StepAgent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        // Accept either a bare string (treated as `ByName`) or an object
        // with exactly one of `id` / `name`. Going through `serde_json::Value`
        // keeps the impl format-agnostic — TOML, JSON, and YAML deserializers
        // all feed through serde's data model and produce a `Value` here.
        let v = serde_json::Value::deserialize(deserializer)?;
        match v {
            serde_json::Value::String(s) => Ok(StepAgent::ByName { name: s }),
            serde_json::Value::Object(map) => {
                let id = map.get("id").and_then(|x| x.as_str());
                let name = map.get("name").and_then(|x| x.as_str());
                match (id, name) {
                    (Some(_), Some(_)) => Err(D::Error::custom(
                        "StepAgent: object form must set exactly one of `id` or `name`, not both",
                    )),
                    (Some(id), None) => Ok(StepAgent::ById { id: id.to_string() }),
                    (None, Some(name)) => Ok(StepAgent::ByName {
                        name: name.to_string(),
                    }),
                    (None, None) => Err(D::Error::custom(
                        "StepAgent: object form must set exactly one of `id` or `name`",
                    )),
                }
            }
            other => Err(D::Error::custom(format!(
                "StepAgent: expected string or object, got {}",
                match other {
                    serde_json::Value::Null => "null",
                    serde_json::Value::Bool(_) => "bool",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::Array(_) => "array",
                    _ => "other",
                }
            ))),
        }
    }
}

/// Execution mode for a workflow step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepMode {
    /// Execute sequentially — this step runs after the previous completes.
    #[default]
    Sequential,
    /// Fan-out — this step runs in parallel with subsequent FanOut steps until Collect.
    FanOut,
    /// Collect results from all preceding fan-out steps.
    Collect,
    /// Conditional — skip this step if previous output doesn't contain `condition` (case-insensitive).
    Conditional { condition: String },
    /// Loop — repeat this step until output contains `until` or `max_iterations` reached.
    Loop { max_iterations: u32, until: String },
}

/// Error handling mode for a workflow step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorMode {
    /// Abort the workflow on error (default).
    #[default]
    Fail,
    /// Skip this step on error and continue.
    Skip,
    /// Retry the step up to N times before failing.
    ///
    /// `backoff_ms`: base delay in milliseconds before attempt 2 (doubled each
    /// attempt, capped at 60 000 ms). `None` preserves the historical
    /// immediate-retry behaviour.
    ///
    /// `jitter_pct`: if set, the backoff is perturbed by ±`jitter_pct`%
    /// (clamped to 0–100) using a uniform random draw. `None` means no
    /// jitter. Both fields default to `None` so old persisted workflows
    /// (`{"retry": {"max_retries": 3}}`) deserialize cleanly.
    Retry {
        max_retries: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backoff_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jitter_pct: Option<u8>,
    },
}

/// The current state of a workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunState {
    Pending,
    Running,
    /// The run was paused mid-execution and is waiting for an external
    /// signal (an approval, a human-supplied input, etc.) before it
    /// continues. Carries the hex-encoded SHA-256 hash of the resume
    /// token (never the plaintext), the wall-clock pause time, and a
    /// human-readable reason for log/UI surfaces.
    ///
    /// The plaintext token is returned to the caller by `pause_run` and
    /// exposed at the HTTP layer only. It is never persisted.
    ///
    /// Per-run snapshot data (step index, variable bindings, current
    /// input) lives in fields on `WorkflowRun` rather than this variant
    /// because runs are also used as raw step-history records — the
    /// snapshot needs to be readable without matching on the state. See
    /// #3335.
    Paused {
        /// Hex-encoded SHA-256 hash of the resume token. The plaintext
        /// token exists only in memory at `pause_run` return time.
        resume_token_hash: String,
        reason: String,
        /// Wall-clock pause time. Surfaced in logs / UI today; future
        /// follow-up will use this to drive a TTL-based GC sweep that
        /// auto-expires Paused runs older than a configurable threshold
        /// (#3335 GC follow-up).
        paused_at: DateTime<Utc>,
    },
    Completed,
    Failed,
    /// The run was explicitly cancelled via [`WorkflowEngine::cancel_run`].
    Cancelled,
}

impl WorkflowRunState {
    /// True when the run is in the `Paused` variant.
    pub fn is_paused(&self) -> bool {
        matches!(self, WorkflowRunState::Paused { .. })
    }
}

/// A running workflow instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// Run instance ID.
    pub id: WorkflowRunId,
    /// The workflow being run.
    pub workflow_id: WorkflowId,
    /// Workflow name (copied for quick access).
    pub workflow_name: String,
    /// Initial input to the workflow.
    pub input: String,
    /// Current state.
    pub state: WorkflowRunState,
    /// Results from each completed step.
    pub step_results: Vec<StepResult>,
    /// Final output (set when workflow completes).
    pub output: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Started at.
    pub started_at: DateTime<Utc>,
    /// Completed at.
    pub completed_at: Option<DateTime<Utc>>,
    /// Pause request set externally via [`WorkflowEngine::pause_run`].
    /// Honored at the next step boundary in
    /// [`WorkflowEngine::execute_run_sequential`] — at which point this
    /// field is consumed and the state transitions to
    /// [`WorkflowRunState::Paused`] using the same `resume_token`. A
    /// non-`None` value with state still `Running` means the engine has
    /// not yet observed the request. See #3335.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_request: Option<PauseRequest>,
    /// When the run is paused, the index of the next step to execute on
    /// resume (so already-completed steps are not re-run). Set when the
    /// engine honors a pause request; cleared when resume completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_step_index: Option<usize>,
    /// Variable bindings as of the pause point. Restored into the local
    /// `variables` map on resume so subsequent steps see the same
    /// `{{var_name}}` substitutions they would have seen if the workflow
    /// had not paused. `BTreeMap` rather than `HashMap` so the persisted
    /// JSON has a stable key order — re-serializing a paused run twice
    /// in a row produces byte-identical output, which keeps audit logs
    /// and external snapshots clean.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub paused_variables: BTreeMap<String, String>,
    /// `current_input` (output of the last completed step) as of the
    /// pause point. Restored on resume so the paused step receives the
    /// same `{{input}}` it would have seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_current_input: Option<String>,
}

impl WorkflowRun {
    /// Wipe every pause-related field on the run. Called from terminal
    /// transitions (Completed / Failed) so a Pause request lodged after
    /// the loop's last boundary check or a snapshot left over from a
    /// failed resume does not survive on a finished run as ghost data.
    /// See #3335 review.
    fn clear_pause_state(&mut self) {
        self.pause_request = None;
        self.paused_step_index = None;
        self.paused_variables.clear();
        self.paused_current_input = None;
    }
}

/// External pause request lodged on a `WorkflowRun`. Pre-generates the
/// resume token so the caller can begin waiting for the corresponding
/// approval / input artifact before the execution loop has actually
/// transitioned the run to `WorkflowRunState::Paused`. The execution loop
/// reuses the hash when it honors the request. See #3335.
///
/// **Security model**: the plaintext token is generated in `pause_run`,
/// returned to the caller as `Result::Ok(Uuid)`, and never persisted. Only
/// the hex-encoded SHA-256 hash is stored here and in
/// `WorkflowRunState::Paused`. Anyone with read access to
/// `~/.librefang/workflow_runs.json` sees only the hash and cannot reverse
/// it to obtain a valid resume credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PauseRequest {
    /// Human-readable explanation surfaced in logs and UI.
    ///
    /// Do not pass secrets, PII, or approval-gating tokens here — use a
    /// side channel referenced by id instead.
    pub reason: String,
    /// Hex-encoded SHA-256 hash of the resume token. The plaintext token
    /// exists only at the `pause_run` call site (returned to the caller)
    /// and at the HTTP response boundary. It is never written to disk.
    pub resume_token_hash: String,
}

/// Result from a single workflow step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step name.
    pub step_name: String,
    /// Agent that executed this step.
    pub agent_id: String,
    /// Agent name.
    pub agent_name: String,
    /// The actual prompt sent to the agent (after variable expansion and context injection).
    pub prompt: String,
    /// Output from this step.
    pub output: String,
    /// Token usage.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// Preview of a single step produced by a dry-run (no LLM calls made).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunStep {
    /// Step name.
    pub step_name: String,
    /// Resolved agent name, or `None` when the agent was not found.
    pub agent_name: Option<String>,
    /// Whether the agent was successfully resolved.
    pub agent_found: bool,
    /// The resolved prompt (after variable expansion with the provided sample input).
    pub resolved_prompt: String,
    /// Whether this step would be skipped (e.g. conditional step whose condition
    /// evaluates to false against the sample input).
    pub skipped: bool,
    /// Human-readable reason for skipping, if applicable.
    pub skip_reason: Option<String>,
}

/// The workflow engine — manages definitions and executes pipeline runs.
///
/// `runs` is a [`DashMap`] so concurrent step writes for *different* runs
/// take independent per-shard locks rather than serializing on a single
/// global `RwLock` (#3717).  Reads and mutations within the same run still
/// use the entry's exclusive shard lock, which is correct because a single
/// run is always driven by one execution task at a time.
#[derive(Clone)]
pub struct WorkflowEngine {
    /// Registered workflow definitions.
    workflows: Arc<RwLock<HashMap<WorkflowId, Workflow>>>,
    /// Active and completed workflow runs.
    runs: Arc<DashMap<WorkflowRunId, WorkflowRun>>,
    /// Optional path to persist completed/failed runs (`~/.librefang/workflow_runs.json`).
    /// Retained for backward compatibility and JSON-to-SQLite migration.
    persist_path: Option<PathBuf>,
    /// Serializes `persist_runs` writes so concurrent callers within a
    /// single process don't `O_TRUNC` the same `.tmp.{pid}` path and
    /// produce a torn file before rename.  `Arc` so the engine stays
    /// `Clone` (mutexes are shared, not duplicated).
    persist_lock: Arc<std::sync::Mutex<()>>,
    /// SQLite-backed workflow store. When `Some`, all persistence goes
    /// through SQLite instead of the JSON file. The JSON path is still
    /// kept for the one-time migration (`migrate_from_json`).
    store: Option<WorkflowStore>,
    /// Directory for persisting workflow definitions (`~/.librefang/workflows/`).
    workflows_dir: Option<PathBuf>,
    /// Kernel-level default total timeout for workflow runs (seconds).
    /// Individual workflows can override this via `Workflow::total_timeout_secs`.
    /// `None` means unbounded.
    pub(crate) default_total_timeout_secs: Option<u64>,
    /// Per-run cancellation notifiers. `cancel_run` calls `notify_waiters()`
    /// on the entry for `run_id` so that retry sleeps can wake up immediately
    /// instead of blocking for the full backoff duration.
    cancel_notify: Arc<DashMap<WorkflowRunId, Arc<tokio::sync::Notify>>>,
}

/// Format the error returned when a workflow step's `agent_resolver` returns
/// `None` — i.e. the referenced registry agent does not exist (or, for
/// `StepAgent::ById`, the UUID is malformed / unregistered).
///
/// We surface both the step name and the agent reference the workflow
/// configured so an operator reading the failure can fix the workflow without
/// having to map "step X" back to "which agent did that step want". Without
/// the agent name in the error, the user only knew which step failed — not
/// which spelling / id mismatch caused it (#4834).
fn format_missing_agent_error(step_name: &str, agent: &StepAgent) -> String {
    match agent {
        StepAgent::ByName { name } => format!(
            "Registry agent '{name}' not found for workflow step '{step_name}' \
             (referenced by name; ensure the agent is registered in the kernel)"
        ),
        StepAgent::ById { id } => format!(
            "Registry agent with id '{id}' not found for workflow step '{step_name}' \
             (referenced by id; check the agent exists and the id is well-formed)"
        ),
    }
}

/// Evaluate a conditional expression against the previous step output.
///
/// Supports:
/// - Simple substring match: `"keyword"` — true if `input` contains `keyword`
/// - Negation: `"!keyword"` — true if `input` does NOT contain `keyword`
/// - AND: `"a && b"` — true if both `a` and `b` are found
/// - OR: `"a || b"` — true if either `a` or `b` is found
///
/// AND binds tighter than OR (standard precedence). All matching is
/// case-insensitive (caller should pass lowercased input).
fn evaluate_condition(input: &str, condition: &str) -> bool {
    let condition = condition.trim();

    // OR: split on `||` first (lower precedence), but only outside quotes.
    if let Some(parts) = split_outside_quotes(condition, "||") {
        // Filter out empty parts to prevent `"a || || b"` from being always-true
        let parts: Vec<_> = parts
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() > 1 {
            return parts.iter().any(|branch| evaluate_condition(input, branch));
        }
    }

    // AND: split on `&&`, only outside quotes.
    if let Some(parts) = split_outside_quotes(condition, "&&") {
        let parts: Vec<_> = parts
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() > 1 {
            return parts.iter().all(|part| evaluate_condition(input, part));
        }
    }

    // Negation
    if let Some(inner) = condition.strip_prefix('!') {
        let inner = inner.trim();
        let cond_lower = inner.to_lowercase();
        return !input.contains(&cond_lower);
    }

    // Simple substring match
    let cond_lower = condition.to_lowercase();
    input.contains(&cond_lower)
}

/// Split `s` on `delimiter` only when the delimiter occurs outside of quoted
/// strings (both single and double quotes). Returns `None` if the string
/// contains no occurrence of `delimiter` outside quotes, otherwise returns the
/// parts.
fn split_outside_quotes(s: &str, delimiter: &str) -> Option<Vec<String>> {
    let delim_bytes = delimiter.as_bytes();
    let delim_len = delim_bytes.len();
    let bytes = s.as_bytes();

    let mut parts = Vec::new();
    let mut current_start = 0;
    let mut in_quote: Option<u8> = None; // tracks the quote character we're inside
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i];

        // Toggle quote state on unescaped quote characters
        if ch == b'"' || ch == b'\'' {
            match in_quote {
                Some(q) if q == ch => in_quote = None,
                None => in_quote = Some(ch),
                _ => {} // inside a different quote type, ignore
            }
            i += 1;
            continue;
        }

        // Check for delimiter match only when outside quotes
        if in_quote.is_none()
            && i + delim_len <= bytes.len()
            && &bytes[i..i + delim_len] == delim_bytes
        {
            parts.push(s[current_start..i].to_string());
            i += delim_len;
            current_start = i;
            continue;
        }

        i += 1;
    }

    // Push the remaining segment
    parts.push(s[current_start..].to_string());

    if parts.len() > 1 {
        Some(parts)
    } else {
        None
    }
}

/// Compute the backoff duration for a workflow step retry.
///
/// Resolution order (first match wins):
///
/// 1. **Explicit retry hint embedded in the error string.** Driver-side
///    `LlmError::RateLimited` / `LlmError::Overloaded` Display formats
///    embed `retry after Nms`, and provider 429 messages frequently
///    inline `Retry-After: N` from the upstream HTTP header. Both are
///    parsed by [`librefang_llm_driver::llm_errors::extract_retry_delay`]
///    (the canonical, single-source-of-truth parser already used by the
///    driver retry layer — keeps kernel and driver semantics in lockstep
///    instead of maintaining a second regex here). Returns ms; we honour
///    it directly so a server asking for 120 s isn't retried at 65 s and
///    429ed again. Capped at 5 minutes; a value over the cap is WARN'd
///    so operators can spot a hostile or misconfigured provider.
/// 2. **Burst / rate-limit substring (case-insensitive).** Anthropic
///    emits `"rate limit"`, OpenAI emits `"rate_limit"` /
///    `"Rate limit"`, Gemini emits `"RATE_LIMIT_EXCEEDED"`. The
///    case-sensitive check that shipped with the first version of
///    this classifier silently fell through on every variant except
///    Anthropic's. Lowercasing once at entry covers all of them and
///    pins the 65 s sliding-window backoff (60 s window + 5 s
///    safety margin).
/// 3. **Everything else** uses exponential backoff `2^attempt`
///    capped at 60 s — the historical default for transient network
///    errors.
fn classify_backoff(err: &str, attempt: u32) -> std::time::Duration {
    /// 60-second sliding window + 5-second safety margin.
    const BURST_WINDOW_BACKOFF: std::time::Duration = std::time::Duration::from_secs(65);
    /// Upper bound on a server-supplied retry hint so a hostile or
    /// misconfigured provider can't park a workflow indefinitely.
    const RETRY_AFTER_CAP: std::time::Duration = std::time::Duration::from_secs(300);

    if let Some(ms) = librefang_llm_driver::llm_errors::extract_retry_delay(err) {
        let asked = std::time::Duration::from_millis(ms);
        if asked > RETRY_AFTER_CAP {
            warn!(
                asked_ms = ms,
                cap_ms = RETRY_AFTER_CAP.as_millis() as u64,
                "Provider retry hint exceeds workflow cap; clamping"
            );
            return RETRY_AFTER_CAP;
        }
        return asked;
    }

    let lowered = err.to_ascii_lowercase();
    let needs_window_clear = lowered.contains("burst")
        || lowered.contains("rate limit")
        || lowered.contains("rate_limit");
    if needs_window_clear {
        return BURST_WINDOW_BACKOFF;
    }

    std::time::Duration::from_secs(2u64.saturating_pow(attempt).min(60))
}

/// Compute the sleep duration between retry attempts.
///
/// Resolution order:
/// 1. If `backoff_ms` is `Some(b)`, the base delay is `b * 2^attempt`
///    (exponential doubling), capped at 60 000 ms.
///    Optional `jitter_pct` (0–100) perturbs the base by ±j% using a
///    uniform random draw so simultaneous retries of multiple steps do not
///    all pile up at the same instant.
/// 2. If `backoff_ms` is `None`, fall through to `classify_backoff` which
///    handles rate-limit hints and the historical exponential default.
fn compute_retry_backoff(
    err: &str,
    attempt: u32,
    backoff_ms: Option<u64>,
    jitter_pct: Option<u8>,
) -> std::time::Duration {
    const MAX_BACKOFF_MS: u64 = 60_000;

    if let Some(base_ms) = backoff_ms {
        // Exponential: base * 2^attempt, capped.
        let raw_ms = base_ms
            .saturating_mul(2u64.saturating_pow(attempt))
            .min(MAX_BACKOFF_MS);

        let final_ms = if let Some(pct) = jitter_pct {
            // Apply ±pct% jitter. Clamp pct to [0, 100].
            let pct_clamped = pct.min(100) as u64;
            let delta = raw_ms.saturating_mul(pct_clamped) / 100;
            if delta == 0 {
                raw_ms
            } else {
                // Draw a uniform value in [0, 2*delta] and subtract delta to get
                // a value in [-delta, +delta] relative to raw_ms.
                let range = delta.saturating_mul(2).saturating_add(1);
                let jitter = rand::random::<u64>() % range;
                raw_ms
                    .saturating_add(jitter)
                    .saturating_sub(delta)
                    .min(MAX_BACKOFF_MS)
            }
        } else {
            raw_ms
        };

        std::time::Duration::from_millis(final_ms)
    } else {
        // No configured backoff — use the existing classifier.
        classify_backoff(err, attempt)
    }
}

impl WorkflowEngine {
    /// Create a new workflow engine (no persistence).
    pub fn new() -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(DashMap::new()),
            persist_path: None,
            persist_lock: Arc::new(std::sync::Mutex::new(())),
            store: None,
            workflows_dir: None,
            default_total_timeout_secs: None,
            cancel_notify: Arc::new(DashMap::new()),
        }
    }

    /// Create a new workflow engine with run persistence (JSON file).
    ///
    /// Completed and failed runs are persisted to `<home_dir>/data/workflow_runs.json`.
    pub fn new_with_persistence(home_dir: &Path) -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(DashMap::new()),
            persist_path: Some(home_dir.join("data").join("workflow_runs.json")),
            persist_lock: Arc::new(std::sync::Mutex::new(())),
            store: None,
            workflows_dir: Some(home_dir.join("workflows")),
            default_total_timeout_secs: None,
            cancel_notify: Arc::new(DashMap::new()),
        }
    }

    /// Create a new workflow engine backed by SQLite.
    ///
    /// All state transitions are persisted immediately to the database.
    /// The `home_dir` is retained so `migrate_from_json` can find the
    /// legacy `workflow_runs.json` file for one-time import.
    pub fn new_with_store(store: WorkflowStore, home_dir: &Path) -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(DashMap::new()),
            persist_path: Some(home_dir.join("data").join("workflow_runs.json")),
            persist_lock: Arc::new(std::sync::Mutex::new(())),
            store: Some(store),
            workflows_dir: Some(home_dir.join("workflows")),
            default_total_timeout_secs: None,
            cancel_notify: Arc::new(DashMap::new()),
        }
    }

    // -- Token hashing --------------------------------------------------------

    /// Hash a plaintext resume token for at-rest storage.
    ///
    /// Uses SHA-256 (already a dependency via `sha2`) and returns a
    /// hex-encoded 32-byte digest. The plaintext UUID is serialized to its
    /// canonical lowercase hyphenated form before hashing so the result is
    /// stable across restarts regardless of endianness.
    pub fn hash_resume_token(token: &Uuid) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.to_string().as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Constant-time comparison of two hex-encoded token hashes.
    ///
    /// Uses `subtle::ConstantTimeEq` on raw bytes so the comparison does not
    /// leak timing information about how many prefix bytes match, which would
    /// be a partial-guess oracle for the stored hash.
    fn hashes_equal(a: &str, b: &str) -> bool {
        // Length check first. Unequal lengths can be compared in constant time
        // by padding the shorter, but two SHA-256 hex strings are always 64
        // bytes — a length mismatch is always a mismatch and reveals nothing.
        if a.len() != b.len() {
            return false;
        }
        a.as_bytes().ct_eq(b.as_bytes()).into()
    }

    // -- Run Persistence ------------------------------------------------------

    /// Load persisted runs into memory.
    ///
    /// When a SQLite store is configured, loads from the database.
    /// Otherwise falls back to the legacy JSON file. Returns the number
    /// of runs loaded. If no data source exists, returns `Ok(0)`.
    pub fn load_runs(&self) -> Result<usize, String> {
        if let Some(ref store) = self.store {
            return self.load_runs_from_sqlite(store);
        }
        self.load_runs_from_json()
    }

    /// Load runs from the SQLite store into the in-memory DashMap.
    fn load_runs_from_sqlite(&self, store: &WorkflowStore) -> Result<usize, String> {
        let rows = store
            .load_all_runs()
            .map_err(|e| format!("workflow SQLite load failed: {e}"))?;
        let total = rows.len();
        let mut loaded: usize = 0;
        let mut skipped: usize = 0;
        for row in rows {
            match row_to_workflow_run(&row) {
                Ok(run) => {
                    self.runs.insert(run.id, run);
                    loaded += 1;
                }
                Err(e) => {
                    skipped += 1;
                    warn!(
                        run_id = %row.id,
                        error = %e,
                        "Skipping unreadable workflow run from SQLite"
                    );
                }
            }
        }
        debug!(loaded, skipped, total, "Loaded workflow runs from SQLite");
        Ok(loaded)
    }

    /// Load runs from the legacy JSON file into the in-memory DashMap.
    fn load_runs_from_json(&self) -> Result<usize, String> {
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(0),
        };
        if !path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read workflow runs: {e}"))?;

        // Per-entry tolerant parse rather than `Vec<WorkflowRun>` in one
        // shot: an old daemon reading a file written by a newer daemon
        // may encounter rows with shapes it doesn't know (e.g. the
        // tagged `Paused { ... }` variant added in #3335). Failing the
        // whole load on the first bad row would drop *all* persisted
        // history; instead, skip unrecognized rows with a WARN. Newer
        // daemons reading older files still parse cleanly because every
        // post-foundation field is `#[serde(default)]`.
        let raw_rows: Vec<serde_json::Value> = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse workflow runs (top-level array): {e}"))?;
        let total = raw_rows.len();
        let mut runs: Vec<WorkflowRun> = Vec::with_capacity(total);
        let mut skipped: usize = 0;
        for (idx, row) in raw_rows.into_iter().enumerate() {
            match serde_json::from_value::<WorkflowRun>(row) {
                Ok(run) => runs.push(run),
                Err(e) => {
                    skipped += 1;
                    warn!(
                        index = idx,
                        error = %e,
                        "Skipping unrecognized workflow run during load (likely a newer schema; \
                         downgrade-safe rollback)"
                    );
                }
            }
        }

        let count = runs.len();
        for run in runs {
            self.runs.insert(run.id, run);
        }
        debug!(
            count,
            skipped, total, "Loaded persisted workflow runs from disk"
        );
        Ok(count)
    }

    /// Persist runs to the backing store.
    ///
    /// When a SQLite store is configured, iterates all runs in the
    /// DashMap and upserts each one. A WAL checkpoint is issued after
    /// writing terminal-state runs to ensure durability.
    ///
    /// Without a SQLite store, falls back to the legacy JSON atomic
    /// write.
    fn persist_runs(&self) {
        if let Some(ref store) = self.store {
            self.persist_runs_to_sqlite(store);
            return;
        }
        self.persist_runs_to_json();
    }

    /// Persist all runs to SQLite via upsert.
    fn persist_runs_to_sqlite(&self, store: &WorkflowStore) {
        let mut wrote_terminal = false;
        for entry in self.runs.iter() {
            let run = entry.value();
            let row = workflow_run_to_row(run);
            if let Err(e) = store.upsert_run(&row) {
                warn!(run_id = %run.id, error = %e, "Failed to persist workflow run to SQLite");
            }
            if matches!(
                run.state,
                WorkflowRunState::Completed
                    | WorkflowRunState::Failed
                    | WorkflowRunState::Paused { .. }
            ) {
                wrote_terminal = true;
            }
        }
        if wrote_terminal {
            if let Err(e) = store.wal_checkpoint() {
                warn!("WAL checkpoint after workflow persist failed: {e}");
            }
        }
        debug!("Persisted workflow runs to SQLite");
    }

    /// Persist completed/failed/paused runs to JSON via atomic write (legacy path).
    fn persist_runs_to_json(&self) {
        let _guard = self.persist_lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = match &self.persist_path {
            Some(p) => p,
            None => return,
        };
        // Collect terminal/paused runs from the DashMap.
        // DashMap provides shared iteration via `iter()` without a global lock.
        // Persist:
        //   - terminal runs (Completed / Failed) for history queries
        //   - paused runs (#3335) so a daemon restart preserves the
        //     snapshot needed to resume — without this, a paused run
        //     becomes unrecoverable on restart.
        // Pending / Running are not persisted: they have no durable
        // boundary on which to roll forward and would otherwise come
        // back as zombie runs after a crash.
        let terminal: Vec<WorkflowRun> = self
            .runs
            .iter()
            .filter(|r| {
                matches!(
                    r.state,
                    WorkflowRunState::Completed
                        | WorkflowRunState::Failed
                        | WorkflowRunState::Paused { .. }
                )
            })
            .map(|r| r.value().clone())
            .collect();
        let data = match serde_json::to_string_pretty(&terminal) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to serialize workflow runs: {e}");
                return;
            }
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Failed to create workflow runs dir: {e}");
                return;
            }
        }
        let tmp_path = crate::persist_tmp_path(path);
        {
            use std::io::Write as _;
            let write_result = (|| -> std::io::Result<()> {
                let mut f = std::fs::File::create(&tmp_path)?;
                f.write_all(data.as_bytes())?;
                f.sync_all()?;
                Ok(())
            })();
            if let Err(e) = write_result {
                warn!("Failed to write workflow runs temp file: {e}");
                let _ = std::fs::remove_file(&tmp_path);
                return;
            }
        }
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            warn!("Failed to rename workflow runs file: {e}");
            let _ = std::fs::remove_file(&tmp_path);
            return;
        }
        debug!("Persisted workflow runs to disk");
    }

    /// Async wrapper for `persist_runs` — delegates to a blocking task.
    ///
    /// Returns `Err` when the spawn_blocking task itself panics so callers
    /// can surface the failure instead of silently dropping it (#3753).
    /// The inner `persist_runs` already logs and swallows IO/serde errors;
    /// only a JoinError (panic / cancel) bubbles up here.
    async fn persist_runs_async(&self) -> Result<(), String> {
        if self.persist_path.is_none() {
            return Ok(());
        }
        let engine = self.clone();
        match tokio::task::spawn_blocking(move || engine.persist_runs()).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // JoinError = the blocking task panicked or was cancelled.
                // Log loudly AND propagate so the workflow run reports the
                // persistence failure instead of pretending to have saved.
                error!("workflow persist task panicked: {e}");
                Err(format!("workflow persist task panicked: {e}"))
            }
        }
    }

    /// Register a new workflow definition and persist it to disk.
    ///
    /// Persistence is atomic: serialise → write `<id>.workflow.json.tmp` →
    /// rename to `<id>.workflow.json`. A crash mid-write leaves the `.tmp`
    /// side-file (ignored by `load_from_dir_sync`'s extension filter) but
    /// never a half-written `<id>.workflow.json` that would later refuse to
    /// parse and stall startup.
    pub async fn register(&self, workflow: Workflow) -> WorkflowId {
        let id = workflow.id;
        if let Some(ref dir) = self.workflows_dir {
            let path = dir.join(format!("{id}.workflow.json"));
            let tmp_path = dir.join(format!("{id}.workflow.json.tmp"));
            match serde_json::to_string_pretty(&workflow) {
                Ok(json) => {
                    if let Err(e) = tokio::fs::create_dir_all(dir).await {
                        warn!(workflow_id = %id, error = %e, "Failed to create workflows dir");
                    } else if let Err(e) = tokio::fs::write(&tmp_path, &json).await {
                        warn!(workflow_id = %id, error = %e, "Failed to persist workflow definition (tmp write)");
                    } else if let Err(e) = tokio::fs::rename(&tmp_path, &path).await {
                        warn!(workflow_id = %id, error = %e, "Failed to persist workflow definition (atomic rename)");
                        // Best-effort cleanup so the next register attempt isn't
                        // blocked by a stale tmp file.
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                    } else {
                        debug!(workflow_id = %id, path = %path.display(), "Persisted workflow definition");
                    }
                }
                Err(e) => {
                    warn!(workflow_id = %id, error = %e, "Failed to serialize workflow definition");
                }
            }
        }
        self.workflows.write().await.insert(id, workflow);
        info!(workflow_id = %id, "Workflow registered");
        id
    }

    /// Load and register all workflow definitions from a directory (sync version for boot).
    ///
    /// Scans for `*.workflow.toml` and `*.workflow.json` files. Each file is
    /// parsed into a [`Workflow`] and registered in the engine.  Invalid files
    /// are logged as warnings and skipped.
    ///
    /// This is a blocking version intended for use during daemon startup before
    /// the async runtime is processing concurrent requests.
    pub fn load_from_dir_sync(&self, dir: &Path) -> usize {
        if !dir.is_dir() {
            debug!(path = %dir.display(), "Workflows directory does not exist, skipping");
            return 0;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(path = %dir.display(), "Failed to read workflows directory: {e}");
                return 0;
            }
        };

        // Maximum workflow file size (1 MiB). Files larger than this are
        // skipped to prevent accidental memory bloat from huge files.
        const MAX_WORKFLOW_FILE_SIZE: u64 = 1024 * 1024;

        let mut count = 0;
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(dir = %dir.display(), "Failed to read directory entry: {e}");
                    continue;
                }
            };
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();

            let is_toml = name.ends_with(".workflow.toml");
            let is_json = name.ends_with(".workflow.json");
            if !is_toml && !is_json {
                continue;
            }

            // Check file size before reading into memory.
            match std::fs::metadata(&path) {
                Ok(meta) if meta.len() > MAX_WORKFLOW_FILE_SIZE => {
                    warn!(
                        path = %path.display(),
                        size = meta.len(),
                        max = MAX_WORKFLOW_FILE_SIZE,
                        "Workflow file exceeds maximum size limit, skipping"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(path = %path.display(), "Failed to stat workflow file: {e}");
                    continue;
                }
                _ => {}
            }

            let workflow = if is_toml {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match toml::from_str::<Workflow>(&content) {
                        Ok(w) => Some(w),
                        Err(e) => {
                            warn!(path = %path.display(), "Invalid workflow TOML: {e}");
                            None
                        }
                    },
                    Err(e) => {
                        warn!(path = %path.display(), "Failed to read workflow file: {e}");
                        None
                    }
                }
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str::<Workflow>(&content) {
                        Ok(w) => Some(w),
                        Err(e) => {
                            warn!(path = %path.display(), "Invalid workflow JSON: {e}");
                            None
                        }
                    },
                    Err(e) => {
                        warn!(path = %path.display(), "Failed to read workflow file: {e}");
                        None
                    }
                }
            };

            if let Some(wf) = workflow {
                let wf_name = wf.name.clone();
                let wf_id = wf.id;
                let mut map = self.workflows.blocking_write();
                if map.contains_key(&wf_id) {
                    warn!(
                        workflow_id = %wf_id,
                        file = %path.display(),
                        "Workflow ID already registered — overwriting with file version"
                    );
                }
                map.insert(wf_id, wf);
                drop(map);
                info!(workflow_id = %wf_id, name = %wf_name, path = %path.display(), "Auto-registered workflow from disk");
                count += 1;
            }
        }

        count
    }

    /// List all registered workflows.
    pub async fn list_workflows(&self) -> Vec<Workflow> {
        self.workflows.read().await.values().cloned().collect()
    }

    /// Get a specific workflow by ID.
    pub async fn get_workflow(&self, id: WorkflowId) -> Option<Workflow> {
        self.workflows.read().await.get(&id).cloned()
    }

    /// Update an existing workflow definition in place.
    /// Returns `true` if the workflow existed and was updated.
    pub async fn update_workflow(&self, id: WorkflowId, mut workflow: Workflow) -> bool {
        let mut workflows = self.workflows.write().await;
        if let std::collections::hash_map::Entry::Occupied(mut entry) = workflows.entry(id) {
            workflow.id = id; // ensure ID stays the same
            entry.insert(workflow);
            true
        } else {
            false
        }
    }

    /// Remove a workflow definition and its persisted file.
    pub async fn remove_workflow(&self, id: WorkflowId) -> bool {
        let removed = self.workflows.write().await.remove(&id).is_some();
        if removed {
            if let Some(ref dir) = self.workflows_dir {
                let path = dir.join(format!("{id}.workflow.json"));
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        warn!(workflow_id = %id, error = %e, "Failed to delete workflow definition file");
                    }
                }
            }
        }
        removed
    }

    /// Maximum number of retained workflow runs. Oldest completed/failed
    /// runs are evicted when this limit is exceeded.
    const MAX_RETAINED_RUNS: usize = 200;

    /// Start a workflow run. Returns the run ID and a handle to check progress.
    ///
    /// The actual execution is driven externally by calling `execute_run()`
    /// with the kernel handle, since the workflow engine doesn't own the kernel.
    pub async fn create_run(
        &self,
        workflow_id: WorkflowId,
        input: String,
    ) -> Option<WorkflowRunId> {
        let workflow = self.workflows.read().await.get(&workflow_id)?.clone();
        let run_id = WorkflowRunId::new();

        let run = WorkflowRun {
            id: run_id,
            workflow_id,
            workflow_name: workflow.name,
            input,
            state: WorkflowRunState::Pending,
            step_results: Vec::new(),
            output: None,
            error: None,
            started_at: Utc::now(),
            completed_at: None,
            pause_request: None,
            paused_step_index: None,
            paused_variables: BTreeMap::new(),
            paused_current_input: None,
        };

        // Persist the freshly-created Pending row before it goes into
        // the DashMap. The batch `persist_runs` family deliberately
        // skips Pending — without an explicit per-row upsert here, a
        // newly created run that crashes before being dispatched
        // disappears entirely on restart. The store happens to be cheap
        // enough (one indexed insert) that doing it inline is fine; if
        // it ever becomes hot, the right move is to debounce the
        // upsert, not drop it.
        if self.store.is_some() {
            self.upsert_run_to_store(&run);
        }

        self.runs.insert(run_id, run);
        // Seat a fresh Notify so retry sleeps can be woken on cancellation.
        self.cancel_notify
            .insert(run_id, Arc::new(tokio::sync::Notify::new()));

        // Evict oldest terminal runs (Completed / Failed / Cancelled) when
        // we exceed the cap. Cancelled must be included here, otherwise a
        // burst of user-initiated cancels would pin those records in the
        // DashMap forever and push out evictable Completed/Failed runs.
        if self.runs.len() > Self::MAX_RETAINED_RUNS {
            let mut evictable: Vec<(WorkflowRunId, DateTime<Utc>)> = self
                .runs
                .iter()
                .filter(|r| {
                    matches!(
                        r.state,
                        WorkflowRunState::Completed
                            | WorkflowRunState::Failed
                            | WorkflowRunState::Cancelled
                    )
                })
                .map(|r| (*r.key(), r.started_at))
                .collect();

            // Sort oldest first
            evictable.sort_by_key(|(_, t)| *t);

            let to_remove = self.runs.len() - Self::MAX_RETAINED_RUNS;
            for (id, _) in evictable.into_iter().take(to_remove) {
                self.runs.remove(&id);
                debug!(run_id = %id, "Evicted old workflow run");
            }
        }

        Some(run_id)
    }

    /// Get the current state of a workflow run.
    pub async fn get_run(&self, run_id: WorkflowRunId) -> Option<WorkflowRun> {
        self.runs.get(&run_id).map(|r| r.clone())
    }

    /// Recover workflow runs left in `Running` or `Pending` state after a daemon crash.
    ///
    /// Called once at boot. Any run whose `started_at` age exceeds `stale_timeout` is
    /// transitioned to `Failed` with a "Interrupted by daemon restart" error message.
    /// Returns the number of runs recovered. A `stale_timeout` of zero is
    /// treated as "feature disabled" and returns `0` without inspecting any
    /// runs — kernel boot guards on this anyway, but keeping the no-op here
    /// means a future direct caller can't accidentally fail every run.
    pub fn recover_stale_running_runs(&self, stale_timeout: std::time::Duration) -> usize {
        if stale_timeout.is_zero() {
            return 0;
        }
        let now = Utc::now();
        let stale_secs = stale_timeout.as_secs() as i64;
        let mut recovered = 0usize;
        // DashMap's `iter_mut` takes a per-shard write lock as the iterator
        // visits each entry — no global write lock and no awaiting required.
        for mut entry in self.runs.iter_mut() {
            let run = entry.value_mut();
            if !matches!(
                run.state,
                WorkflowRunState::Running | WorkflowRunState::Pending
            ) {
                continue;
            }
            let age = now.signed_duration_since(run.started_at).num_seconds();
            if age < stale_secs {
                continue;
            }
            warn!(
                run_id = %run.id,
                state = ?run.state,
                started_at = %run.started_at,
                age_secs = age,
                "Recovering stale workflow run interrupted by daemon restart"
            );
            run.state = WorkflowRunState::Failed;
            run.error = Some("Interrupted by daemon restart".to_string());
            run.completed_at = Some(now);
            run.clear_pause_state();
            // Persist the recovered Failed state immediately. Without
            // this, the run lives in the DashMap as Failed but the
            // SQLite row is whatever was on disk before recovery — so a
            // second crash before the next batch persist would resurface
            // the same run as a stale Running again.
            if self.store.is_some() {
                self.upsert_run_to_store(run);
            }
            recovered += 1;
        }
        recovered
    }

    /// Pause every in-flight workflow run on graceful shutdown so it
    /// survives the restart, then flush the result to disk.
    ///
    /// Pre-fix (#3335): `Running` and `Pending` runs lived in
    /// memory only. `persist_runs` deliberately skips both states (no
    /// durable boundary to roll forward across), so the last `O_TRUNC +
    /// rename` cycle of `workflow_runs.json` did not include them. On
    /// graceful daemon stop the runs vanished — `load_runs` reloads only
    /// what was on disk, and the dashboard's `list_runs` call after
    /// restart no longer surfaces the in-flight workload.
    ///
    /// Reasoning behind transitioning to `Paused` rather than `Failed`:
    /// a graceful shutdown is a clean process boundary — the daemon
    /// reached this code without a mid-write crash, and the in-memory
    /// step results / paused-variables snapshot is internally
    /// consistent. `Paused` carries a fresh `resume_token` so the
    /// operator can either resume via the existing
    /// `WorkflowEngine::resume_run` API or, depending on policy, the
    /// stale-timeout sweep at next boot
    /// ([`Self::recover_stale_running_runs`]) eventually demotes them
    /// to `Failed`. Either way, the run is *visible* in the dashboard
    /// after restart instead of silently disappearing.
    ///
    /// Crash shutdown (SIGKILL / OOM / power loss) is **not** addressed
    /// by this method: the daemon never reaches `shutdown` in those
    /// paths, so the only durable state is whatever `persist_runs`
    /// already flushed. The stale-running-runs recovery sweep on the
    /// next boot is the existing safety net for that case.
    ///
    /// Returns the number of runs whose state was changed. The caller
    /// is expected to invoke this once during the shutdown sequence,
    /// after the agent supervisor has stopped accepting new work.
    pub fn drain_on_shutdown(&self) -> usize {
        let now = Utc::now();
        let mut drained = 0usize;
        // DashMap's `iter_mut` takes a per-shard write lock as the
        // iterator visits each entry — no global write lock and no
        // awaiting required, mirroring `recover_stale_running_runs`.
        for mut entry in self.runs.iter_mut() {
            let run = entry.value_mut();
            if !matches!(
                run.state,
                WorkflowRunState::Running | WorkflowRunState::Pending
            ) {
                continue;
            }
            info!(
                run_id = %run.id,
                state = ?run.state,
                "Pausing in-flight workflow run for shutdown"
            );
            // Generate a fresh token so the operator can resume via HTTP,
            // but only store the hash at rest.
            let shutdown_token = Uuid::new_v4();
            let shutdown_token_hash = Self::hash_resume_token(&shutdown_token);
            run.state = WorkflowRunState::Paused {
                resume_token_hash: shutdown_token_hash,
                reason: "Interrupted by daemon shutdown".to_string(),
                paused_at: now,
            };
            // The shutdown_token plaintext is intentionally discarded here.
            // Shutdown-paused runs must be resumed via the HTTP endpoint with
            // a fresh pause+resume cycle if the token is unknown to the caller.
            drained += 1;
        }
        // Flush only when something actually changed — `persist_runs`
        // is a full O_TRUNC + rename of `workflow_runs.json`, no point
        // in paying that I/O if there's nothing to add.
        if drained > 0 {
            self.persist_runs();
        }
        drained
    }

    /// List all workflow runs (optionally filtered by state).
    pub async fn list_runs(&self, state_filter: Option<&str>) -> Vec<WorkflowRun> {
        self.runs
            .iter()
            .filter(|r| {
                state_filter
                    .map(|f| match f {
                        "pending" => matches!(r.state, WorkflowRunState::Pending),
                        "running" => matches!(r.state, WorkflowRunState::Running),
                        "completed" => matches!(r.state, WorkflowRunState::Completed),
                        "failed" => matches!(r.state, WorkflowRunState::Failed),
                        "cancelled" => matches!(r.state, WorkflowRunState::Cancelled),
                        _ => true,
                    })
                    .unwrap_or(true)
            })
            .map(|r| r.value().clone())
            .collect()
    }

    /// Build a `SubagentContext` from the current workflow state and format the
    /// prompt with context preamble prepended (if applicable).
    ///
    /// Returns the (possibly enriched) prompt. Context is injected only when:
    /// 1. The step's `inherit_context` is not explicitly `Some(false)`, AND
    /// 2. The agent's `inherit_parent_context` manifest field is true.
    fn build_context_prompt(
        prompt: &str,
        step: &WorkflowStep,
        step_index: usize,
        workflow_name: &str,
        step_results: &[StepResult],
        agent_inherit: bool,
    ) -> String {
        // Check whether context injection is enabled for this step
        let inherit = step.inherit_context.unwrap_or(agent_inherit);
        if !inherit {
            return prompt.to_string();
        }

        let ctx = SubagentContext {
            parent_agent_name: None,
            parent_session_summary: None,
            workflow_name: Some(workflow_name.to_string()),
            step_index,
            previous_outputs: step_results
                .iter()
                .map(|r| {
                    (
                        r.step_name.clone(),
                        SubagentContext::truncate_output_preview(&r.output),
                    )
                })
                .collect(),
        };

        match ctx.format_preamble() {
            Some(preamble) => format!("{preamble}{prompt}"),
            None => prompt.to_string(),
        }
    }

    /// Replace `{{var_name}}` references in a template with stored variable values.
    fn expand_variables(template: &str, input: &str, vars: &HashMap<String, String>) -> String {
        let mut result = template.replace("{{input}}", input);
        for (key, value) in vars {
            result = result.replace(&format!("{{{{{key}}}}}"), value);
        }
        result
    }

    /// Execute a single step with error mode handling. Returns (output, input_tokens, output_tokens).
    async fn execute_step_with_error_mode<F, Fut>(
        step: &WorkflowStep,
        agent_id: AgentId,
        prompt: String,
        send_message: &F,
        run_id: WorkflowRunId,
        cancel_notify: &Arc<DashMap<WorkflowRunId, Arc<tokio::sync::Notify>>>,
    ) -> Result<Option<(String, u64, u64)>, String>
    where
        F: Fn(AgentId, String, Option<SessionMode>) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let timeout_dur = std::time::Duration::from_secs(step.timeout_secs);
        let session_mode = step.session_mode;

        match &step.error_mode {
            ErrorMode::Fail => {
                let result =
                    tokio::time::timeout(timeout_dur, send_message(agent_id, prompt, session_mode))
                        .await
                        .map_err(|_| {
                            format!(
                                "Step '{}' timed out after {}s",
                                step.name, step.timeout_secs
                            )
                        })?
                        .map_err(|e| format!("Step '{}' failed: {}", step.name, e))?;
                Ok(Some(result))
            }
            ErrorMode::Skip => {
                match tokio::time::timeout(
                    timeout_dur,
                    send_message(agent_id, prompt, session_mode),
                )
                .await
                {
                    Ok(Ok(result)) => Ok(Some(result)),
                    Ok(Err(e)) => {
                        warn!("Step '{}' failed (skipping): {e}", step.name);
                        Ok(None)
                    }
                    Err(_) => {
                        warn!(
                            "Step '{}' timed out (skipping) after {}s",
                            step.name, step.timeout_secs
                        );
                        Ok(None)
                    }
                }
            }
            ErrorMode::Retry {
                max_retries,
                backoff_ms,
                jitter_pct,
            } => {
                let mut last_err = String::new();
                for attempt in 0..=*max_retries {
                    match tokio::time::timeout(
                        timeout_dur,
                        send_message(agent_id, prompt.clone(), session_mode),
                    )
                    .await
                    {
                        Ok(Ok(result)) => return Ok(Some(result)),
                        Ok(Err(e)) => {
                            last_err = e.to_string();
                            if attempt < *max_retries {
                                let sleep_dur = compute_retry_backoff(
                                    &last_err,
                                    attempt,
                                    *backoff_ms,
                                    *jitter_pct,
                                );
                                warn!(
                                    "Step '{}' attempt {} failed: {e}, retrying in {:?}",
                                    step.name,
                                    attempt + 1,
                                    sleep_dur
                                );
                                let notify = cancel_notify.get(&run_id).map(|n| Arc::clone(&*n));
                                tokio::select! {
                                    _ = tokio::time::sleep(sleep_dur) => {}
                                    _ = async {
                                        match notify {
                                            Some(n) => n.notified().await,
                                            None => std::future::pending::<()>().await,
                                        }
                                    } => {
                                        // Cancellation observed during retry sleep.
                                        return Err("workflow run cancelled".into());
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            last_err = format!("timed out after {}s", step.timeout_secs);
                            if attempt < *max_retries {
                                let sleep_dur = compute_retry_backoff(
                                    &last_err,
                                    attempt,
                                    *backoff_ms,
                                    *jitter_pct,
                                );
                                warn!(
                                    "Step '{}' attempt {} timed out, retrying in {:?}",
                                    step.name,
                                    attempt + 1,
                                    sleep_dur
                                );
                                let notify = cancel_notify.get(&run_id).map(|n| Arc::clone(&*n));
                                tokio::select! {
                                    _ = tokio::time::sleep(sleep_dur) => {}
                                    _ = async {
                                        match notify {
                                            Some(n) => n.notified().await,
                                            None => std::future::pending::<()>().await,
                                        }
                                    } => {
                                        // Cancellation observed during retry sleep.
                                        return Err("workflow run cancelled".into());
                                    }
                                }
                            }
                        }
                    }
                }
                Err(format!(
                    "Step '{}' failed after {} retries: {last_err}",
                    step.name, max_retries
                ))
            }
        }
    }

    /// Build a dependency graph from workflow steps.
    ///
    /// Returns a map from step index to the list of step indices it depends on.
    fn build_dependency_graph(
        steps: &[WorkflowStep],
    ) -> Result<HashMap<usize, Vec<usize>>, String> {
        // Check for duplicate step names
        let mut name_to_idx: HashMap<&str, usize> = HashMap::new();
        for (i, s) in steps.iter().enumerate() {
            if let Some(prev) = name_to_idx.insert(s.name.as_str(), i) {
                return Err(format!(
                    "Duplicate step name '{}' at positions {} and {}",
                    s.name, prev, i
                ));
            }
        }

        let mut graph: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, step) in steps.iter().enumerate() {
            let mut deps = Vec::new();
            for dep_name in &step.depends_on {
                let &dep_idx = name_to_idx.get(dep_name.as_str()).ok_or_else(|| {
                    format!(
                        "Step '{}' depends on '{}' which does not exist",
                        step.name, dep_name
                    )
                })?;
                deps.push(dep_idx);
            }
            graph.insert(i, deps);
        }
        Ok(graph)
    }

    /// Topological sort using Kahn's algorithm.
    ///
    /// Returns layers of step indices — steps within the same layer can run
    /// in parallel, and layers must execute sequentially.
    /// Returns `Err` if a cycle is detected.
    fn topological_sort(steps: &[WorkflowStep]) -> Result<Vec<Vec<usize>>, String> {
        let dep_graph = Self::build_dependency_graph(steps)?;
        let n = steps.len();

        // Build in-degree count and reverse adjacency (dependents)
        let mut in_degree = vec![0usize; n];
        let mut dependents: HashMap<usize, Vec<usize>> = HashMap::new();
        for (&node, deps) in &dep_graph {
            in_degree[node] = deps.len();
            for &dep in deps {
                dependents.entry(dep).or_default().push(node);
            }
        }

        // Start with all nodes that have no dependencies
        let mut queue: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut layers: Vec<Vec<usize>> = Vec::new();
        let mut processed = 0;

        while !queue.is_empty() {
            // Current queue forms one parallel layer
            let current_layer = std::mem::take(&mut queue);
            for &node in &current_layer {
                processed += 1;
                if let Some(deps) = dependents.get(&node) {
                    for &dependent in deps {
                        in_degree[dependent] -= 1;
                        if in_degree[dependent] == 0 {
                            queue.push(dependent);
                        }
                    }
                }
            }
            layers.push(current_layer);
        }

        if processed != n {
            return Err("Cycle detected in workflow step dependencies".to_string());
        }

        Ok(layers)
    }

    /// Request that an in-flight workflow run pause at the next step
    /// boundary. Returns the `resume_token` the caller must hand back to
    /// [`Self::resume_run`].
    ///
    /// The actual transition to [`WorkflowRunState::Paused`] happens inside
    /// the execution loop — the loop reads `pause_request` at the top of
    /// each step iteration and, if set, snapshots
    /// `(step_index, variables, current_input)` and updates the run's
    /// state with the pre-generated token. Calling `pause_run` on an
    /// already-paused run is idempotent: the existing token is returned.
    ///
    /// **DAG workflows fail-closed on pause.** Workflows whose steps use
    /// `depends_on` are routed through the DAG executor, which does not
    /// yet support pause. If `pause_run` is lodged on a DAG workflow,
    /// the next `execute_run` call will mark the run `Failed` with a
    /// "DAG pause not supported" error rather than silently completing.
    /// Callers that don't know which executor a workflow targets should
    /// either inspect `Workflow::steps` first or accept the fail-closed
    /// behavior. Per-layer DAG pause checkpoints track as a follow-up
    /// against #3335.
    ///
    /// **SECURITY:** the `reason` string is persisted plaintext to
    /// `workflow_runs.json` and surfaced to operators. Do not include
    /// secrets, PII, or approval-gating values in it.
    ///
    /// **Token security:** the returned `Uuid` is the plaintext token. It
    /// exists only in memory at this call site. Only the SHA-256 hash is
    /// stored on the run and persisted to disk. Present the token to
    /// `resume_run` to continue the workflow.
    ///
    /// Errors:
    /// - [`PauseRunError::NotFound`] if the run is unknown.
    /// - [`PauseRunError::AlreadyPaused`] if the run is already in `Paused`
    ///   state (returns the existing token hash for idempotency verification).
    /// - [`PauseRunError::AlreadyTerminal`] if the run has finished.
    pub async fn pause_run(
        &self,
        run_id: WorkflowRunId,
        reason: impl Into<String>,
    ) -> Result<Uuid, PauseRunError> {
        let mut run = self
            .runs
            .get_mut(&run_id)
            .ok_or(PauseRunError::NotFound(run_id))?;
        // Inspect state first; we need the state borrow to end before we
        // can write pause_request below.
        match &run.state {
            WorkflowRunState::Pending | WorkflowRunState::Running => {}
            WorkflowRunState::Paused {
                resume_token_hash, ..
            } => {
                return Err(PauseRunError::AlreadyPaused {
                    run_id,
                    resume_token_hash: resume_token_hash.clone(),
                })
            }
            WorkflowRunState::Completed => {
                return Err(PauseRunError::AlreadyTerminal {
                    run_id,
                    state: "completed",
                })
            }
            WorkflowRunState::Failed => {
                return Err(PauseRunError::AlreadyTerminal {
                    run_id,
                    state: "failed",
                })
            }
            WorkflowRunState::Cancelled => {
                return Err(PauseRunError::AlreadyTerminal {
                    run_id,
                    state: "cancelled",
                })
            }
        }
        // If a pause was already lodged (Pending/Running but not yet
        // honored by the executor), reuse the existing token to stay
        // idempotent across concurrent callers.
        if let Some(ref existing) = run.pause_request {
            // We only stored the hash, so we cannot return the plaintext.
            // Generate a fresh one that hashes to the same stored value?
            // No — we cannot invert a hash. Instead, treat a pre-existing
            // pause_request as if it's already paused and return the hash.
            return Err(PauseRunError::AlreadyPaused {
                run_id,
                resume_token_hash: existing.resume_token_hash.clone(),
            });
        }
        let token = Uuid::new_v4();
        let hash = Self::hash_resume_token(&token);
        run.pause_request = Some(PauseRequest {
            reason: reason.into(),
            resume_token_hash: hash,
        });
        Ok(token)
    }

    /// Cancel a workflow run.
    ///
    /// Transitions `Pending`, `Running`, or `Paused` runs to
    /// [`WorkflowRunState::Cancelled`] atomically under the DashMap shard
    /// guard and persists the change immediately so a crash after this call
    /// does not revert the run to a non-terminal state on restart.
    ///
    /// Returns `Err(`[`CancelRunError`]`)` if the run is not found or is
    /// already in a terminal state (`Completed`, `Failed`, or `Cancelled`).
    ///
    /// After the state transition the per-run [`tokio::sync::Notify`] is
    /// signalled so any retry sleep currently waiting on the notifier wakes
    /// up immediately rather than sleeping for the full backoff duration.
    ///
    /// The executor also observes cancellation at every step boundary: it
    /// peeks the run state at the top of each iteration and exits early if
    /// it finds `Cancelled`.
    pub async fn cancel_run(&self, run_id: WorkflowRunId) -> Result<(), CancelRunError> {
        let already_paused = {
            let mut run = self
                .runs
                .get_mut(&run_id)
                .ok_or(CancelRunError::NotFound(run_id))?;
            match &run.state {
                WorkflowRunState::Pending
                | WorkflowRunState::Running
                | WorkflowRunState::Paused { .. } => {
                    let was_paused = run.state.is_paused();
                    run.state = WorkflowRunState::Cancelled;
                    run.completed_at = Some(Utc::now());
                    // Clear any pending pause request so the executor cannot
                    // re-pause a cancelled run.
                    run.pause_request = None;
                    was_paused
                }
                WorkflowRunState::Completed => {
                    return Err(CancelRunError::AlreadyTerminal {
                        run_id,
                        state: "completed",
                    })
                }
                WorkflowRunState::Failed => {
                    return Err(CancelRunError::AlreadyTerminal {
                        run_id,
                        state: "failed",
                    })
                }
                WorkflowRunState::Cancelled => {
                    return Err(CancelRunError::AlreadyTerminal {
                        run_id,
                        state: "cancelled",
                    })
                }
            }
            // shard guard dropped here
        };

        // Wake any retry sleep that is parked on this run's notifier.
        if let Some(n) = self.cancel_notify.get(&run_id) {
            n.notify_waiters();
        }

        // Clear pause snapshot outside the shard guard (clear_pause_state
        // takes a separate get_mut).
        if already_paused {
            if let Some(mut run) = self.runs.get_mut(&run_id) {
                run.clear_pause_state();
            }
        }

        // Persist immediately so a restart does not revert to Running/Pending.
        if let Some(run) = self.runs.get(&run_id) {
            self.upsert_run_to_store(&run);
        }
        if let Err(e) = self.persist_runs_async().await {
            warn!(run_id = %run_id, error = %e, "Failed to persist cancelled run state");
        }

        info!(run_id = %run_id, "Workflow run cancelled");
        Ok(())
    }

    /// Resume a paused workflow run from where it stopped.
    ///
    /// Verifies that the run is in [`WorkflowRunState::Paused`] and that
    /// the supplied `resume_token` matches the one stored on the run, then
    /// restores the snapshotted bindings + input and re-enters
    /// `execute_run_sequential` at the saved step index. The DAG path
    /// does not yet support pause/resume — see
    /// [`Self::execute_run_dag`] for the explicit guard.
    ///
    /// Returns the eventual workflow output (or step error) just like the
    /// original `execute_run`.
    pub async fn resume_run<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        resume_token: Uuid,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String, bool)>,
        send_message: F,
    ) -> Result<String, ResumeRunError>
    where
        F: Fn(AgentId, String, Option<SessionMode>) -> Fut + Sync,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>> + Send,
    {
        // Validate state + token, snapshot what we need, flip state back to
        // Running, then drop the lock before re-entering execution. The
        // execution path takes the same DashMap shard lock per step, so we
        // must not hold it across the await.
        let workflow = {
            let workflow_id = {
                let mut run = self
                    .runs
                    .get_mut(&run_id)
                    .ok_or(ResumeRunError::NotFound(run_id))?;
                // Validate token inside a scope so the immutable borrow of
                // `run.state` ends before we mutate it below.
                {
                    match &run.state {
                        WorkflowRunState::Paused {
                            resume_token_hash: stored_hash,
                            ..
                        } => {
                            // Hash the presented token and compare in constant
                            // time so we do not leak partial-match timing.
                            let presented_hash = Self::hash_resume_token(&resume_token);
                            if !Self::hashes_equal(stored_hash, &presented_hash) {
                                return Err(ResumeRunError::TokenMismatch { run_id });
                            }
                        }
                        WorkflowRunState::Pending => {
                            return Err(ResumeRunError::NotPaused {
                                run_id,
                                state: "pending",
                            })
                        }
                        WorkflowRunState::Running => {
                            return Err(ResumeRunError::NotPaused {
                                run_id,
                                state: "running",
                            })
                        }
                        WorkflowRunState::Completed => {
                            return Err(ResumeRunError::NotPaused {
                                run_id,
                                state: "completed",
                            })
                        }
                        WorkflowRunState::Failed => {
                            return Err(ResumeRunError::NotPaused {
                                run_id,
                                state: "failed",
                            })
                        }
                        WorkflowRunState::Cancelled => {
                            return Err(ResumeRunError::NotPaused {
                                run_id,
                                state: "cancelled",
                            })
                        }
                    }
                }
                // Flip back to Running and clear pause_request so the loop
                // does not re-pause itself immediately.
                run.state = WorkflowRunState::Running;
                run.pause_request = None;
                run.workflow_id
                // `run` (DashMap shard guard) is dropped here
            };
            self.workflows
                .read()
                .await
                .get(&workflow_id)
                .cloned()
                .ok_or_else(|| ResumeRunError::ExecutionFailed {
                    run_id,
                    detail: format!("Workflow definition {workflow_id} not found"),
                })?
        };

        // Re-enter the sequential path. It looks at paused_step_index /
        // paused_variables / paused_current_input on the run and resumes
        // from there. The dispatch over has_dag_deps mirrors execute_run.
        let has_dag_deps = workflow.steps.iter().any(|s| !s.depends_on.is_empty());
        let result = if has_dag_deps {
            // Symmetric guard with execute_run_dag's check; we only reach
            // this path if a run was paused via the sequential path before
            // the DAG branch was selected, which the DAG guard refuses to
            // do today. Belt-and-suspenders so resume can never fan out
            // into the unsupported DAG executor by accident.
            Err(ResumeRunError::DagUnsupported { run_id })
        } else {
            // `input` here is unused on the resume path because the loop
            // pulls `paused_current_input` off the run when present.
            self.execute_run_sequential(run_id, &workflow, "", &agent_resolver, &send_message)
                .await
                .map_err(|e| ResumeRunError::ExecutionFailed { run_id, detail: e })
        };
        self.cleanup_terminal_pause_state(run_id).await;
        // If persistence panicked, surface it instead of returning a fake Ok.
        if let Err(persist_err) = self.persist_runs_async().await {
            return Err(match result {
                Ok(_) => ResumeRunError::ExecutionFailed {
                    run_id,
                    detail: persist_err,
                },
                Err(run_err) => ResumeRunError::ExecutionFailed {
                    run_id,
                    detail: format!("{run_err}; additionally: {persist_err}"),
                },
            });
        }
        result
    }

    /// Execute a workflow run step-by-step.
    ///
    /// This method takes a closure that sends messages to agents,
    /// so the workflow engine remains decoupled from the kernel.
    ///
    /// The `agent_resolver` returns `(AgentId, agent_name, inherit_parent_context)`.
    /// When `inherit_parent_context` is true and the step doesn't override it,
    /// previous step outputs are prepended to the prompt as context.
    pub async fn execute_run<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String, bool)>,
        send_message: F,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String, Option<SessionMode>) -> Fut + Sync,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>> + Send,
    {
        // Get the run and workflow. Mutate the run's state synchronously
        // via DashMap's get_mut, then drop the shard guard before the
        // async workflow lookup.
        let (workflow_id, input) = {
            let mut run = self.runs.get_mut(&run_id).ok_or("Workflow run not found")?;
            // Mutate via DerefMut — `mut` on the binding required to invoke it.
            run.state = WorkflowRunState::Running;
            (run.workflow_id, run.input.clone())
            // `run` (DashMap RefMut shard guard) is dropped here
        };
        let workflow = self
            .workflows
            .read()
            .await
            .get(&workflow_id)
            .ok_or("Workflow definition not found")?
            .clone();

        info!(
            run_id = %run_id,
            workflow = %workflow.name,
            steps = workflow.steps.len(),
            "Starting workflow execution"
        );

        // Resolve total-timeout: workflow field wins over kernel default.
        let total_timeout = workflow
            .total_timeout_secs
            .or(self.default_total_timeout_secs);

        // Check if any step has non-empty depends_on — if so, use DAG execution
        let has_dag_deps = workflow.steps.iter().any(|s| !s.depends_on.is_empty());

        let inner_fut = async {
            if has_dag_deps {
                self.execute_run_dag(run_id, &workflow, &input, &agent_resolver, &send_message)
                    .await
            } else {
                self.execute_run_sequential(
                    run_id,
                    &workflow,
                    &input,
                    &agent_resolver,
                    &send_message,
                )
                .await
            }
        };

        let result = if let Some(secs) = total_timeout {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), inner_fut).await {
                Ok(r) => r,
                Err(_elapsed) => {
                    let msg = format!("workflow exceeded total_timeout of {secs}s");
                    // Only overwrite state if not already Cancelled.
                    if let Some(mut run) = self.runs.get_mut(&run_id) {
                        if !matches!(run.state, WorkflowRunState::Cancelled) {
                            run.state = WorkflowRunState::Failed;
                            run.error = Some(msg.clone());
                            run.completed_at = Some(Utc::now());
                        }
                    }
                    Err(msg)
                }
            }
        } else {
            inner_fut.await
        };

        self.cleanup_terminal_pause_state(run_id).await;
        // Surface persist panics instead of swallowing them (#3753).
        if let Err(persist_err) = self.persist_runs_async().await {
            return Err(match result {
                Ok(_) => persist_err,
                Err(run_err) => format!("{run_err}; additionally: {persist_err}"),
            });
        }
        result
    }

    /// Wipe pause-related fields on the run if it ended up in a terminal
    /// state (Completed / Failed). Called once at the bottom of
    /// `execute_run` and `resume_run` so every terminal transition gets
    /// the same cleanup, regardless of which inner branch (sequential
    /// happy path, DAG entry-guard refuse, mid-step Failed) ran. Avoids
    /// scattering identical clear-five-fields blocks across ~10 sites.
    /// See #3335 review.
    async fn cleanup_terminal_pause_state(&self, run_id: WorkflowRunId) {
        if let Some(mut run) = self.runs.get_mut(&run_id) {
            if matches!(
                run.state,
                WorkflowRunState::Completed
                    | WorkflowRunState::Failed
                    | WorkflowRunState::Cancelled
            ) {
                run.clear_pause_state();
                // Drop the per-run notifier — the run is terminal and no
                // retry sleep will ever need to be woken again.
                drop(run);
                self.cancel_notify.remove(&run_id);
            }
        }
    }

    /// Sequential workflow execution (extracted for persistence wrapping).
    async fn execute_run_sequential<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        workflow: &Workflow,
        input: &str,
        agent_resolver: &impl Fn(&StepAgent) -> Option<(AgentId, String, bool)>,
        send_message: &F,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String, Option<SessionMode>) -> Fut + Sync,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>> + Send,
    {
        // Resume snapshot: when the run was previously paused, the prior
        // execution stored `(step_index, variables, current_input)` on the
        // run before transitioning to `Paused`. Clone (not take!) so a
        // mid-resume failure leaves the snapshot intact for future
        // retry-from-failure paths — the snapshot is only authoritatively
        // cleared on successful completion, see the Completed transition
        // at the bottom of this function. The pause-mid-loop branch
        // overwrites the snapshot with fresh values when it transitions
        // back to Paused.
        let (mut current_input, mut variables, mut i) = {
            if let Some(run) = self.runs.get(&run_id) {
                if let Some(saved_idx) = run.paused_step_index {
                    let saved_vars: HashMap<String, String> = run
                        .paused_variables
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    let saved_input = run.paused_current_input.clone().unwrap_or_default();
                    debug!(
                        run_id = %run_id,
                        resume_step = saved_idx,
                        var_count = saved_vars.len(),
                        "Resuming workflow run from saved snapshot"
                    );
                    (saved_input, saved_vars, saved_idx)
                } else {
                    (input.to_string(), HashMap::new(), 0_usize)
                }
            } else {
                (input.to_string(), HashMap::new(), 0_usize)
            }
        };
        let mut all_outputs: Vec<String> = Vec::new();

        while i < workflow.steps.len() {
            // Pause-request gate. Honored at the top of every step
            // iteration so an in-flight step is allowed to finish before
            // the run pauses — partial-step rollback would be a much
            // larger feature than #3335 requires.
            //
            // Atomically take pause_request AND apply the Paused state
            // transition under a single get_mut shard lock. Splitting
            // the take and the state-set across two get_mut calls would
            // let a concurrent pause_run() lodge a fresh request between
            // them, leaving state=Paused{token=A} but pause_request=
            // Some{token=B} — a token mismatch that breaks resume (#3716).
            let pending_pause = if let Some(mut run) = self.runs.get_mut(&run_id) {
                if let Some(pause) = run.pause_request.take() {
                    run.paused_step_index = Some(i);
                    run.paused_variables = variables
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    run.paused_current_input = Some(current_input.clone());
                    run.state = WorkflowRunState::Paused {
                        resume_token_hash: pause.resume_token_hash.clone(),
                        reason: pause.reason.clone(),
                        paused_at: Utc::now(),
                    };
                    Some(pause)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(pause) = pending_pause {
                // Persist the Paused state immediately. Without this, a
                // SIGKILL between here and the end-of-execute_run batch
                // persist would lose the resume_token — the contract
                // handed to the operator on a pause-for-approval flow.
                // Mirrors the create_run / recover_stale_running_runs
                // wiring; remaining per-step Failed/Completed
                // transitions are tracked separately (see PR body).
                if let Some(run) = self.runs.get(&run_id) {
                    self.upsert_run_to_store(&run);
                }
                info!(
                    run_id = %run_id,
                    resume_step = i,
                    reason = %pause.reason,
                    "Workflow run paused at step boundary"
                );
                return Ok(current_input);
            }

            // Cancellation gate — checked after the pause gate so a
            // concurrent cancel_run() is always observable at every step
            // boundary. `cancel_run` already set state=Cancelled; we just
            // need to exit the loop cleanly without overwriting that state.
            if self
                .runs
                .get(&run_id)
                .map(|r| matches!(r.state, WorkflowRunState::Cancelled))
                .unwrap_or(false)
            {
                info!(run_id = %run_id, step = i, "Workflow run cancelled at step boundary");
                return Err("workflow run cancelled".into());
            }

            let step = &workflow.steps[i];

            debug!(
                step = i + 1,
                name = %step.name,
                "Executing workflow step"
            );

            match &step.mode {
                StepMode::Sequential => {
                    let (agent_id, agent_name, agent_inherit) = match agent_resolver(&step.agent) {
                        Some(v) => v,
                        None => {
                            let e = format_missing_agent_error(&step.name, &step.agent);
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                if !matches!(r.state, WorkflowRunState::Cancelled) {
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(e.clone());
                                    r.completed_at = Some(Utc::now());
                                }
                            }
                            return Err(e);
                        }
                    };

                    let raw_prompt =
                        Self::expand_variables(&step.prompt_template, &current_input, &variables);

                    // Snapshot step results for context injection
                    let prev_results: Vec<StepResult> = self
                        .runs
                        .get(&run_id)
                        .map(|r| r.step_results.clone())
                        .unwrap_or_default();
                    let prompt = Self::build_context_prompt(
                        &raw_prompt,
                        step,
                        i,
                        &workflow.name,
                        &prev_results,
                        agent_inherit,
                    );

                    let prompt_sent = prompt.clone();
                    let start = std::time::Instant::now();
                    let result = Self::execute_step_with_error_mode(
                        step,
                        agent_id,
                        prompt,
                        &send_message,
                        run_id,
                        &self.cancel_notify,
                    )
                    .await;
                    let duration_ms = start.elapsed().as_millis() as u64;

                    match result {
                        Ok(Some((output, input_tokens, output_tokens))) => {
                            let step_result = StepResult {
                                step_name: step.name.clone(),
                                agent_id: agent_id.to_string(),
                                agent_name,
                                prompt: prompt_sent,
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms,
                            };
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                r.step_results.push(step_result);
                            }

                            if let Some(ref var) = step.output_var {
                                variables.insert(var.clone(), output.clone());
                            }

                            all_outputs.push(output.clone());
                            current_input = output;
                            info!(step = i + 1, name = %step.name, duration_ms, "Step completed");
                        }
                        Ok(None) => {
                            // Step was skipped (ErrorMode::Skip)
                            info!(step = i + 1, name = %step.name, "Step skipped");
                        }
                        Err(e) => {
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                if !matches!(r.state, WorkflowRunState::Cancelled) {
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(e.clone());
                                    r.completed_at = Some(Utc::now());
                                }
                            }
                            return Err(e);
                        }
                    }
                }

                StepMode::FanOut => {
                    // Collect consecutive FanOut steps and run them in parallel
                    let mut fan_out_steps = vec![(i, step)];
                    let mut j = i + 1;
                    while j < workflow.steps.len() {
                        if matches!(workflow.steps[j].mode, StepMode::FanOut) {
                            fan_out_steps.push((j, &workflow.steps[j]));
                            j += 1;
                        } else {
                            break;
                        }
                    }

                    // Build all futures
                    let mut futures = Vec::new();
                    let mut step_infos = Vec::new();
                    let mut step_prompts: Vec<String> = Vec::new();

                    // Snapshot step results once for all fan-out steps
                    let prev_results: Vec<StepResult> = self
                        .runs
                        .get(&run_id)
                        .map(|r| r.step_results.clone())
                        .unwrap_or_default();

                    for (idx, fan_step) in &fan_out_steps {
                        let (agent_id, agent_name, agent_inherit) =
                            match agent_resolver(&fan_step.agent) {
                                Some(v) => v,
                                None => {
                                    let e =
                                        format_missing_agent_error(&fan_step.name, &fan_step.agent);
                                    if let Some(mut r) = self.runs.get_mut(&run_id) {
                                        if !matches!(r.state, WorkflowRunState::Cancelled) {
                                            r.state = WorkflowRunState::Failed;
                                            r.error = Some(e.clone());
                                            r.completed_at = Some(Utc::now());
                                        }
                                    }
                                    return Err(e);
                                }
                            };
                        let raw_prompt = Self::expand_variables(
                            &fan_step.prompt_template,
                            &current_input,
                            &variables,
                        );
                        let prompt = Self::build_context_prompt(
                            &raw_prompt,
                            fan_step,
                            *idx,
                            &workflow.name,
                            &prev_results,
                            agent_inherit,
                        );
                        let timeout_dur = std::time::Duration::from_secs(fan_step.timeout_secs);

                        step_infos.push((*idx, fan_step.name.clone(), agent_id, agent_name));
                        step_prompts.push(prompt.clone());
                        futures.push(tokio::time::timeout(
                            timeout_dur,
                            send_message(agent_id, prompt, fan_step.session_mode),
                        ));
                    }

                    let start = std::time::Instant::now();
                    let results = futures::future::join_all(futures).await;
                    let duration_ms = start.elapsed().as_millis() as u64;

                    for (k, result) in results.into_iter().enumerate() {
                        let (_, ref step_name, agent_id, ref agent_name) = step_infos[k];
                        let fan_step = fan_out_steps[k].1;

                        match result {
                            Ok(Ok((output, input_tokens, output_tokens))) => {
                                let step_result = StepResult {
                                    step_name: step_name.clone(),
                                    agent_id: agent_id.to_string(),
                                    agent_name: agent_name.clone(),
                                    prompt: step_prompts[k].clone(),
                                    output: output.clone(),
                                    input_tokens,
                                    output_tokens,
                                    duration_ms,
                                };
                                if let Some(mut r) = self.runs.get_mut(&run_id) {
                                    r.step_results.push(step_result);
                                }
                                if let Some(ref var) = fan_step.output_var {
                                    variables.insert(var.clone(), output.clone());
                                }
                                all_outputs.push(output.clone());
                                current_input = output;
                            }
                            Ok(Err(e)) => {
                                let error_msg =
                                    format!("FanOut step '{}' failed: {}", step_name, e);
                                warn!(%error_msg);
                                if let Some(mut r) = self.runs.get_mut(&run_id) {
                                    if !matches!(r.state, WorkflowRunState::Cancelled) {
                                        r.state = WorkflowRunState::Failed;
                                        r.error = Some(error_msg.clone());
                                        r.completed_at = Some(Utc::now());
                                    }
                                }
                                return Err(error_msg);
                            }
                            Err(_) => {
                                let error_msg = format!(
                                    "FanOut step '{}' timed out after {}s",
                                    step_name, fan_step.timeout_secs
                                );
                                warn!(%error_msg);
                                if let Some(mut r) = self.runs.get_mut(&run_id) {
                                    if !matches!(r.state, WorkflowRunState::Cancelled) {
                                        r.state = WorkflowRunState::Failed;
                                        r.error = Some(error_msg.clone());
                                        r.completed_at = Some(Utc::now());
                                    }
                                }
                                return Err(error_msg);
                            }
                        }
                    }

                    info!(
                        count = fan_out_steps.len(),
                        duration_ms, "FanOut steps completed"
                    );

                    // Skip past the fan-out steps we just processed
                    i = j;
                    continue;
                }

                StepMode::Collect => {
                    // Build structured JSON from step results accumulated so far.
                    // NOTE: This intentionally outputs JSON (not plain text with `---` separators)
                    // so downstream steps can parse individual fan-out results programmatically.
                    let step_results: Vec<StepResult> = self
                        .runs
                        .get(&run_id)
                        .map(|r| r.step_results.clone())
                        .unwrap_or_default();

                    // Collect results that correspond to the current all_outputs batch.
                    // Take the last N step results where N = all_outputs.len().
                    let n = all_outputs.len();
                    let relevant = if step_results.len() >= n {
                        &step_results[step_results.len() - n..]
                    } else {
                        &step_results[..]
                    };

                    let results_json: Vec<serde_json::Value> = relevant
                        .iter()
                        .map(|sr| {
                            serde_json::json!({
                                "agent": sr.agent_name,
                                "output": sr.output,
                            })
                        })
                        .collect();

                    let merged = serde_json::json!({ "results": results_json });
                    current_input = serde_json::to_string(&merged).unwrap_or_default();
                    all_outputs.clear();
                    all_outputs.push(current_input.clone());
                    if let Some(ref var) = step.output_var {
                        variables.insert(var.clone(), current_input.clone());
                    }
                }

                StepMode::Conditional { condition } => {
                    let prev_lower = current_input.to_lowercase();

                    let condition_met = evaluate_condition(&prev_lower, condition);

                    if !condition_met {
                        info!(
                            step = i + 1,
                            name = %step.name,
                            condition,
                            "Conditional step skipped (condition not met)"
                        );
                        i += 1;
                        continue;
                    }

                    // Condition met — execute like sequential
                    let (agent_id, agent_name, agent_inherit) = agent_resolver(&step.agent)
                        .ok_or_else(|| format_missing_agent_error(&step.name, &step.agent))?;

                    let raw_prompt =
                        Self::expand_variables(&step.prompt_template, &current_input, &variables);
                    let prev_results: Vec<StepResult> = self
                        .runs
                        .get(&run_id)
                        .map(|r| r.step_results.clone())
                        .unwrap_or_default();
                    let prompt = Self::build_context_prompt(
                        &raw_prompt,
                        step,
                        i,
                        &workflow.name,
                        &prev_results,
                        agent_inherit,
                    );

                    let prompt_sent = prompt.clone();
                    let start = std::time::Instant::now();
                    let result = Self::execute_step_with_error_mode(
                        step,
                        agent_id,
                        prompt,
                        &send_message,
                        run_id,
                        &self.cancel_notify,
                    )
                    .await;
                    let duration_ms = start.elapsed().as_millis() as u64;

                    match result {
                        Ok(Some((output, input_tokens, output_tokens))) => {
                            let step_result = StepResult {
                                step_name: step.name.clone(),
                                agent_id: agent_id.to_string(),
                                agent_name,
                                prompt: prompt_sent,
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms,
                            };
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                r.step_results.push(step_result);
                            }
                            if let Some(ref var) = step.output_var {
                                variables.insert(var.clone(), output.clone());
                            }
                            all_outputs.push(output.clone());
                            current_input = output;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                if !matches!(r.state, WorkflowRunState::Cancelled) {
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(e.clone());
                                    r.completed_at = Some(Utc::now());
                                }
                            }
                            return Err(e);
                        }
                    }
                }

                StepMode::Loop {
                    max_iterations,
                    until,
                } => {
                    let (agent_id, agent_name, agent_inherit) = agent_resolver(&step.agent)
                        .ok_or_else(|| format_missing_agent_error(&step.name, &step.agent))?;

                    let until_lower = until.to_lowercase();

                    for loop_iter in 0..*max_iterations {
                        let raw_prompt = Self::expand_variables(
                            &step.prompt_template,
                            &current_input,
                            &variables,
                        );
                        // Re-snapshot step results each iteration (accumulates loop outputs)
                        let prev_results: Vec<StepResult> = self
                            .runs
                            .get(&run_id)
                            .map(|r| r.step_results.clone())
                            .unwrap_or_default();
                        let prompt = Self::build_context_prompt(
                            &raw_prompt,
                            step,
                            i,
                            &workflow.name,
                            &prev_results,
                            agent_inherit,
                        );

                        let prompt_sent = prompt.clone();
                        let start = std::time::Instant::now();
                        let result = Self::execute_step_with_error_mode(
                            step,
                            agent_id,
                            prompt,
                            &send_message,
                            run_id,
                            &self.cancel_notify,
                        )
                        .await;
                        let duration_ms = start.elapsed().as_millis() as u64;

                        match result {
                            Ok(Some((output, input_tokens, output_tokens))) => {
                                let step_result = StepResult {
                                    step_name: format!("{} (iter {})", step.name, loop_iter + 1),
                                    agent_id: agent_id.to_string(),
                                    agent_name: agent_name.clone(),
                                    prompt: prompt_sent,
                                    output: output.clone(),
                                    input_tokens,
                                    output_tokens,
                                    duration_ms,
                                };
                                if let Some(mut r) = self.runs.get_mut(&run_id) {
                                    r.step_results.push(step_result);
                                }

                                current_input = output.clone();

                                if output.to_lowercase().contains(&until_lower) {
                                    info!(
                                        step = i + 1,
                                        name = %step.name,
                                        iterations = loop_iter + 1,
                                        "Loop terminated (until condition met)"
                                    );
                                    break;
                                }

                                if loop_iter + 1 == *max_iterations {
                                    info!(
                                        step = i + 1,
                                        name = %step.name,
                                        "Loop terminated (max iterations reached)"
                                    );
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                if let Some(mut r) = self.runs.get_mut(&run_id) {
                                    if !matches!(r.state, WorkflowRunState::Cancelled) {
                                        r.state = WorkflowRunState::Failed;
                                        r.error = Some(e.clone());
                                        r.completed_at = Some(Utc::now());
                                    }
                                }
                                return Err(e);
                            }
                        }
                    }

                    if let Some(ref var) = step.output_var {
                        variables.insert(var.clone(), current_input.clone());
                    }
                    all_outputs.push(current_input.clone());
                }
            }

            i += 1;
        }

        // Mark workflow as completed. Clear the pause snapshot fields and
        // any orphan `pause_request` that may have been lodged after the
        // last step boundary check — a pause requested between the loop's
        // final iteration and this transition would otherwise survive on a
        // Completed run as dead data.
        let final_output = current_input.clone();
        if let Some(mut r) = self.runs.get_mut(&run_id) {
            r.state = WorkflowRunState::Completed;
            r.output = Some(final_output.clone());
            r.completed_at = Some(Utc::now());
            r.pause_request = None;
            r.paused_step_index = None;
            r.paused_variables.clear();
            r.paused_current_input = None;
        }

        info!(run_id = %run_id, "Workflow completed successfully");
        Ok(final_output)
    }

    /// DAG-based workflow execution.
    ///
    /// Steps are topologically sorted into layers based on `depends_on`.
    /// Steps within the same layer run concurrently; layers execute
    /// sequentially. Each step receives the workflow input plus any
    /// variables produced by its dependencies via `output_var`.
    async fn execute_run_dag<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        workflow: &Workflow,
        input: &str,
        agent_resolver: &impl Fn(&StepAgent) -> Option<(AgentId, String, bool)>,
        send_message: &F,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String, Option<SessionMode>) -> Fut + Sync,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>> + Send,
    {
        // Pause/resume support is sequential-path only in #3335. If a pause
        // request was lodged before the engine routed into the DAG branch,
        // refuse cleanly rather than silently dropping the request — the
        // DAG executor would never observe `pause_request` and the run
        // would just complete normally, leaving the caller's `resume_run`
        // call hanging. The follow-up to add per-layer pause checkpoints
        // tracks against #3335.
        let dag_pause_requested = self
            .runs
            .get(&run_id)
            .and_then(|r| r.pause_request.as_ref().map(|p| p.reason.clone()));
        if let Some(reason) = dag_pause_requested {
            // Mark the run Failed and consume the lingering pause_request
            // before returning. Without this the run stays Running with
            // `pause_request: Some(_)` forever, looking like a live
            // workflow that just never executes — and `cleanup_terminal_pause_state`
            // (called by execute_run after we return) wouldn't run because
            // state isn't terminal. Set Failed so the cleanup pass picks it up.
            if let Some(mut run) = self.runs.get_mut(&run_id) {
                run.state = WorkflowRunState::Failed;
                run.error = Some(format!(
                    "DAG workflow refused to start: pause requested ({reason}) \
                     but pause/resume is supported on the sequential path only \
                     (#3335 follow-up)"
                ));
                run.completed_at = Some(Utc::now());
            }
            return Err(format!(
                "Pause requested ({reason}) but the workflow uses DAG dependencies; \
                 pause/resume is supported on the sequential path only (#3335 follow-up)"
            ));
        }

        let layers = Self::topological_sort(&workflow.steps)?;
        let mut variables: HashMap<String, String> = HashMap::new();
        // Track which step names have failed so we can skip dependents
        let mut failed_steps: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut last_output = input.to_string();

        info!(
            run_id = %run_id,
            layers = layers.len(),
            "Executing workflow in DAG mode"
        );

        for (layer_idx, layer) in layers.iter().enumerate() {
            debug!(
                layer = layer_idx + 1,
                steps = layer.len(),
                "Executing DAG layer"
            );

            if layer.len() == 1 {
                // Single step in layer — execute directly (no concurrency overhead)
                let step_idx = layer[0];
                let step = &workflow.steps[step_idx];

                // Check if any dependency failed
                let dep_failed = step.depends_on.iter().any(|dep| failed_steps.contains(dep));

                if dep_failed {
                    match step.error_mode {
                        ErrorMode::Fail => {
                            let error_msg =
                                format!("Step '{}' skipped: dependency failed", step.name);
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                r.state = WorkflowRunState::Failed;
                                r.error = Some(error_msg.clone());
                                r.completed_at = Some(Utc::now());
                            }
                            return Err(error_msg);
                        }
                        _ => {
                            warn!(
                                step = %step.name,
                                "Skipping step due to failed dependency"
                            );
                            failed_steps.insert(step.name.clone());
                            continue;
                        }
                    }
                }

                let (agent_id, agent_name, _agent_inherit) = agent_resolver(&step.agent)
                    .ok_or_else(|| format_missing_agent_error(&step.name, &step.agent))?;

                let prompt = Self::expand_variables(&step.prompt_template, input, &variables);
                let prompt_sent = prompt.clone();
                let start = std::time::Instant::now();
                let result = Self::execute_step_with_error_mode(
                    step,
                    agent_id,
                    prompt,
                    send_message,
                    run_id,
                    &self.cancel_notify,
                )
                .await;
                let duration_ms = start.elapsed().as_millis() as u64;

                match result {
                    Ok(Some((output, input_tokens, output_tokens))) => {
                        let step_result = StepResult {
                            step_name: step.name.clone(),
                            agent_id: agent_id.to_string(),
                            agent_name,
                            prompt: prompt_sent,
                            output: output.clone(),
                            input_tokens,
                            output_tokens,
                            duration_ms,
                        };
                        if let Some(mut r) = self.runs.get_mut(&run_id) {
                            r.step_results.push(step_result);
                        }
                        if let Some(ref var) = step.output_var {
                            variables.insert(var.clone(), output.clone());
                        }
                        last_output = output;
                        info!(
                            step = %step.name,
                            duration_ms,
                            "DAG step completed"
                        );
                    }
                    Ok(None) => {
                        info!(step = %step.name, "DAG step skipped (error mode)");
                        failed_steps.insert(step.name.clone());
                    }
                    Err(e) => {
                        failed_steps.insert(step.name.clone());
                        if matches!(step.error_mode, ErrorMode::Fail) {
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                r.state = WorkflowRunState::Failed;
                                r.error = Some(e.clone());
                                r.completed_at = Some(Utc::now());
                            }
                            return Err(e);
                        }
                        warn!(step = %step.name, error = %e, "DAG step failed (non-fatal)");
                    }
                }
            } else {
                // Multiple steps in layer — execute concurrently
                let mut futures = Vec::new();
                let mut step_metas: Vec<(usize, String, AgentId, String, bool)> = Vec::new();
                let mut step_prompts: Vec<String> = Vec::new();

                for &step_idx in layer {
                    let step = &workflow.steps[step_idx];

                    let dep_failed = step.depends_on.iter().any(|dep| failed_steps.contains(dep));

                    let (agent_id, agent_name, _agent_inherit) = agent_resolver(&step.agent)
                        .ok_or_else(|| format_missing_agent_error(&step.name, &step.agent))?;

                    step_metas.push((
                        step_idx,
                        step.name.clone(),
                        agent_id,
                        agent_name,
                        dep_failed,
                    ));

                    // Each future returns (result, duration_ms) for per-step timing
                    if dep_failed {
                        step_prompts.push(String::new());
                        let step_name = step.name.clone();
                        let error_mode = step.error_mode.clone();
                        futures.push(Box::pin(async move {
                            let r = if matches!(error_mode, ErrorMode::Fail) {
                                Err(format!("Step '{}' skipped: dependency failed", step_name))
                            } else {
                                Ok(None)
                            };
                            (r, 0u64)
                        })
                            as std::pin::Pin<
                                Box<
                                    dyn std::future::Future<
                                            Output = (
                                                Result<Option<(String, u64, u64)>, String>,
                                                u64,
                                            ),
                                        > + Send,
                                >,
                            >);
                    } else {
                        let prompt =
                            Self::expand_variables(&step.prompt_template, input, &variables);
                        step_prompts.push(prompt.clone());
                        let timeout_dur = std::time::Duration::from_secs(step.timeout_secs);
                        let err_mode = step.error_mode.clone();
                        let step_name = step.name.clone();
                        let step_session_mode = step.session_mode;

                        futures.push(Box::pin(async move {
                            let step_start = std::time::Instant::now();
                            let result = tokio::time::timeout(
                                timeout_dur,
                                send_message(agent_id, prompt, step_session_mode),
                            )
                            .await;
                            let step_duration = step_start.elapsed().as_millis() as u64;
                            let r = match result {
                                Ok(Ok(output)) => Ok(Some(output)),
                                Ok(Err(e)) => match err_mode {
                                    ErrorMode::Fail => {
                                        Err(format!("Step '{}' failed: {}", step_name, e))
                                    }
                                    _ => Ok(None),
                                },
                                Err(_) => match err_mode {
                                    ErrorMode::Fail => {
                                        Err(format!("Step '{}' timed out", step_name))
                                    }
                                    _ => Ok(None),
                                },
                            };
                            (r, step_duration)
                        })
                            as std::pin::Pin<
                                Box<
                                    dyn std::future::Future<
                                            Output = (
                                                Result<Option<(String, u64, u64)>, String>,
                                                u64,
                                            ),
                                        > + Send,
                                >,
                            >);
                    }
                }

                let layer_start = std::time::Instant::now();
                let results = futures::future::join_all(futures).await;
                let layer_duration_ms = layer_start.elapsed().as_millis() as u64;

                for (k, (result, step_duration_ms)) in results.into_iter().enumerate() {
                    let (step_idx, ref step_name, agent_id, ref agent_name, _dep_failed) =
                        step_metas[k];
                    let step = &workflow.steps[step_idx];

                    match result {
                        Ok(Some((output, input_tokens, output_tokens))) => {
                            let step_result = StepResult {
                                step_name: step_name.clone(),
                                agent_id: agent_id.to_string(),
                                agent_name: agent_name.clone(),
                                prompt: step_prompts[k].clone(),
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms: step_duration_ms,
                            };
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                r.step_results.push(step_result);
                            }
                            if let Some(ref var) = step.output_var {
                                variables.insert(var.clone(), output.clone());
                            }
                            last_output = output;
                            info!(
                                step = %step_name,
                                duration_ms = step_duration_ms,
                                "DAG step completed"
                            );
                        }
                        Ok(None) => {
                            info!(step = %step_name, "DAG step skipped");
                            failed_steps.insert(step_name.clone());
                        }
                        Err(e) => {
                            failed_steps.insert(step_name.clone());
                            if let Some(mut r) = self.runs.get_mut(&run_id) {
                                r.state = WorkflowRunState::Failed;
                                r.error = Some(e.clone());
                                r.completed_at = Some(Utc::now());
                            }
                            return Err(e);
                        }
                    }
                }

                info!(
                    layer = layer_idx + 1,
                    count = layer.len(),
                    duration_ms = layer_duration_ms,
                    "DAG layer completed"
                );
            }
        }

        // Mark workflow as completed
        if let Some(mut r) = self.runs.get_mut(&run_id) {
            r.state = WorkflowRunState::Completed;
            r.output = Some(last_output.clone());
            r.completed_at = Some(Utc::now());
        }

        info!(run_id = %run_id, "Workflow DAG execution completed successfully");
        Ok(last_output)
    }

    /// Dry-run a workflow: resolve agents and expand prompts without making any LLM calls.
    ///
    /// Returns a per-step preview describing what would be executed. Useful for
    /// validating a workflow definition before committing to a real run.
    ///
    /// The `agent_resolver` returns `(AgentId, agent_name, inherit_parent_context)`.
    pub async fn dry_run(
        &self,
        workflow_id: WorkflowId,
        input: &str,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String, bool)>,
    ) -> Result<Vec<DryRunStep>, String> {
        let workflow = self
            .workflows
            .read()
            .await
            .get(&workflow_id)
            .ok_or_else(|| format!("Workflow '{workflow_id}' not found"))?
            .clone();

        let mut preview = Vec::new();
        let mut variables: HashMap<String, String> = HashMap::new();
        let mut current_input = input.to_string();

        for (i, step) in workflow.steps.iter().enumerate() {
            let raw_prompt =
                Self::expand_variables(&step.prompt_template, &current_input, &variables);

            match &step.mode {
                StepMode::Conditional { condition } => {
                    let prev_lower = current_input.to_lowercase();
                    let condition_met = evaluate_condition(&prev_lower, condition);
                    let (agent_name, agent_found) = match agent_resolver(&step.agent) {
                        Some((_, name, _)) => (Some(name), true),
                        None => (None, false),
                    };
                    preview.push(DryRunStep {
                        step_name: step.name.clone(),
                        agent_name,
                        agent_found,
                        resolved_prompt: raw_prompt,
                        skipped: !condition_met,
                        skip_reason: if !condition_met {
                            Some(format!(
                                "Condition '{condition}' not met against current input"
                            ))
                        } else {
                            None
                        },
                    });
                    // In dry-run, don't advance current_input for skipped steps
                }
                _ => {
                    let (agent_name, agent_found) = match agent_resolver(&step.agent) {
                        Some((_, name, _)) => (Some(name), true),
                        None => (None, false),
                    };
                    preview.push(DryRunStep {
                        step_name: step.name.clone(),
                        agent_name: agent_name.clone(),
                        agent_found,
                        resolved_prompt: raw_prompt.clone(),
                        skipped: false,
                        skip_reason: None,
                    });
                    // Advance with a placeholder so later steps can expand {{input}}
                    if let Some(ref var) = step.output_var {
                        variables.insert(var.clone(), format!("<output of step {}>", i + 1));
                    }
                    current_input = format!("<output of step {} ({})>", i + 1, step.name);
                }
            }
        }

        Ok(preview)
    }

    // -- SQLite per-transition persistence ------------------------------------

    /// Persist a single run to SQLite immediately after a state transition.
    ///
    /// This is the key durability improvement: each state change is
    /// durable on its own rather than waiting for the full batch
    /// `persist_runs`. A WAL checkpoint follows terminal-state writes.
    pub fn upsert_run_to_store(&self, run: &WorkflowRun) {
        if let Some(ref store) = self.store {
            let row = workflow_run_to_row(run);
            if let Err(e) = store.upsert_run(&row) {
                // The caller (`create_run` / `recover_stale_running_runs` / state
                // transitions) has already updated the in-memory DashMap, so the
                // run looks persisted from outside, but a crash before the next
                // batch persist would lose this state. Bumping a counter here
                // gives ops a Prometheus signal beyond log inspection — log
                // sampling under load tends to drop exactly the failures we
                // care about. The caller signature is intentionally not
                // changed to `Result<(), _>` because that would ripple
                // through every state-transition site for marginal gain
                // over a counter + warn.
                metrics::counter!(
                    "librefang_kernel_workflow_upsert_failed_total",
                    "phase" => "upsert_run",
                )
                .increment(1);
                warn!(run_id = %run.id, error = %e, "Immediate SQLite upsert failed");
            }
            if matches!(
                run.state,
                WorkflowRunState::Completed
                    | WorkflowRunState::Failed
                    | WorkflowRunState::Paused { .. }
            ) {
                if let Err(e) = store.wal_checkpoint() {
                    metrics::counter!(
                        "librefang_kernel_workflow_upsert_failed_total",
                        "phase" => "wal_checkpoint",
                    )
                    .increment(1);
                    warn!("WAL checkpoint after terminal upsert failed: {e}");
                }
            }
        }
    }

    /// One-time migration from JSON to SQLite.
    ///
    /// If the legacy `workflow_runs.json` exists and has content, and
    /// the SQLite table has zero rows, import all runs from JSON into
    /// SQLite, then rename the JSON file to `.bak`. Idempotent: if
    /// SQLite already has rows, or the JSON file is missing/empty, this
    /// is a no-op.
    pub fn migrate_from_json(&self) -> Result<usize, String> {
        let store = match &self.store {
            Some(s) => s,
            None => return Ok(0),
        };
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(0),
        };
        if !path.exists() {
            return Ok(0);
        }

        // Only import when SQLite is empty — prevents double-import.
        let existing = store
            .count_runs()
            .map_err(|e| format!("count_runs during migration: {e}"))?;
        if existing > 0 {
            debug!(
                existing,
                "SQLite workflow_runs already populated, skipping JSON migration"
            );
            return Ok(0);
        }

        let data = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        if data.trim().is_empty() || data.trim() == "[]" {
            return Ok(0);
        }

        let json_runs: Vec<WorkflowRun> = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
        if json_runs.is_empty() {
            return Ok(0);
        }

        let rows: Vec<WorkflowRunRow> = json_runs.iter().map(workflow_run_to_row).collect();
        let imported = store
            .bulk_upsert_runs(&rows)
            .map_err(|e| format!("bulk_upsert_runs during migration: {e}"))?;

        // Rename the old file so we never re-import.
        let bak = path.with_extension("json.bak");
        if let Err(e) = std::fs::rename(path, &bak) {
            warn!(
                "Imported {imported} workflow runs but could not rename {} to {}: {e}",
                path.display(),
                bak.display()
            );
        } else {
            info!(
                "Migrated {imported} workflow run(s) from {} to SQLite (renamed to {})",
                path.display(),
                bak.display()
            );
        }
        Ok(imported)
    }
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Auto-registration of workflow definition files from disk
// ---------------------------------------------------------------------------

/// Intermediate struct for deserializing workflow files where `id` and
/// `created_at` are optional. When omitted the loader generates them
/// automatically so users only need to supply `name`, `description`, and
/// `steps`.
#[derive(Debug, Clone, Deserialize)]
struct WorkflowFile {
    #[serde(default)]
    id: Option<WorkflowId>,
    name: String,
    description: String,
    steps: Vec<WorkflowStep>,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
}

impl From<WorkflowFile> for Workflow {
    fn from(f: WorkflowFile) -> Self {
        Self {
            id: f.id.unwrap_or_default(),
            name: f.name,
            description: f.description,
            steps: f.steps,
            created_at: f.created_at.unwrap_or_else(Utc::now),
            layout: None,
            total_timeout_secs: None,
        }
    }
}

/// Scan a directory for workflow definition files (`.yaml`, `.yml`, `.toml`)
/// and return the parsed [`Workflow`] objects. Files that fail to parse are
/// logged as warnings and skipped.
pub fn load_workflow_definitions(dir: &Path) -> Vec<Workflow> {
    let mut workflows = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            // The directory not existing is expected on fresh installs — only
            // warn when it exists but cannot be read.
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    path = %dir.display(),
                    error = %e,
                    "Failed to read workflows directory"
                );
            }
            return workflows;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if !matches!(ext.as_str(), "yaml" | "yml" | "toml") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read workflow file"
                );
                continue;
            }
        };

        let parsed: Result<WorkflowFile, String> = match ext.as_str() {
            "yaml" | "yml" => {
                serde_yaml::from_str(&contents).map_err(|e| format!("YAML parse error: {e}"))
            }
            "toml" => toml::from_str(&contents).map_err(|e| format!("TOML parse error: {e}")),
            _ => continue,
        };

        match parsed {
            Ok(wf_file) => {
                let wf: Workflow = wf_file.into();
                debug!(
                    name = %wf.name,
                    id = %wf.id,
                    path = %path.display(),
                    "Parsed workflow definition from file"
                );
                workflows.push(wf);
            }
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Skipping invalid workflow file"
                );
            }
        }
    }

    workflows
}

// ---------------------------------------------------------------------------
// Workflow -> Template conversion
// ---------------------------------------------------------------------------

use librefang_types::workflow_template::{
    ParameterType, TemplateParameter, WorkflowTemplate, WorkflowTemplateStep,
};
use regex_lite::Regex;
use std::collections::HashSet;

impl WorkflowEngine {
    /// Convert an existing workflow into a reusable [`WorkflowTemplate`].
    ///
    /// Each `WorkflowStep` is mapped to a `WorkflowTemplateStep`. The method
    /// auto-detects parameters by scanning `prompt_template` fields for
    /// `{{var}}` placeholders and creates a [`TemplateParameter`] for each
    /// unique variable found.
    pub fn workflow_to_template(workflow: &Workflow) -> WorkflowTemplate {
        workflow.to_template()
    }
}

impl Workflow {
    /// Convert this workflow into a reusable [`WorkflowTemplate`].
    ///
    /// Each `WorkflowStep` is mapped to a `WorkflowTemplateStep`. Parameters
    /// are auto-detected by scanning `prompt_template` fields for `{{var}}`
    /// placeholders, with one [`TemplateParameter`] created per unique name.
    ///
    /// Exposed as an inherent method so callers outside the kernel (e.g. the
    /// API crate) can perform the conversion without importing
    /// `WorkflowEngine` directly.
    pub fn to_template(&self) -> WorkflowTemplate {
        let workflow = self;
        // Slugify workflow name -> template ID
        let id = workflow
            .name
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
            .trim_matches('-')
            .to_string();

        // Collect all {{var}} placeholders across all steps
        let re = Regex::new(r"\{\{(\w+)\}\}").expect("valid regex");
        let mut seen_params = HashSet::new();
        let mut parameters = Vec::new();

        let steps: Vec<WorkflowTemplateStep> = workflow
            .steps
            .iter()
            .map(|step| {
                // Extract parameters from this step's prompt_template
                for cap in re.captures_iter(&step.prompt_template) {
                    let var_name = cap[1].to_string();
                    if seen_params.insert(var_name.clone()) {
                        parameters.push(TemplateParameter {
                            name: var_name.clone(),
                            description: Some(format!(
                                "Parameter '{}' used in step '{}'",
                                var_name, step.name
                            )),
                            param_type: ParameterType::String,
                            default: None,
                            required: true,
                        });
                    }
                }

                // Map agent to optional string name
                let agent = match &step.agent {
                    StepAgent::ByName { name } => Some(name.clone()),
                    StepAgent::ById { id } => Some(id.clone()),
                };

                WorkflowTemplateStep {
                    name: step.name.clone(),
                    prompt_template: step.prompt_template.clone(),
                    agent,
                    depends_on: step.depends_on.clone(),
                }
            })
            .collect();

        WorkflowTemplate {
            id,
            name: workflow.name.clone(),
            description: workflow.description.clone(),
            category: None,
            parameters,
            steps,
            tags: vec![],
            created_at: Some(chrono::Utc::now().to_rfc3339()),
            i18n: Default::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// WorkflowTemplateRegistry — in-memory store for workflow templates
// ---------------------------------------------------------------------------

/// Convert a `serde_json::Value` to a plain string for template substitution.
fn value_to_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// In-memory registry for storing and retrieving [`WorkflowTemplate`]s.
///
/// Thread-safe: the registry is designed to be wrapped in an `Arc` and shared
/// across async tasks. Internal synchronisation uses a `RwLock`.
pub struct WorkflowTemplateRegistry {
    templates: RwLock<HashMap<String, WorkflowTemplate>>,
}

impl WorkflowTemplateRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            templates: RwLock::new(HashMap::new()),
        }
    }

    /// Register (insert or update) a template.
    ///
    /// If a template with the same `id` already exists it is replaced and the
    /// old value is returned.
    pub async fn register(&self, template: WorkflowTemplate) -> Option<WorkflowTemplate> {
        let mut map = self.templates.write().await;
        map.insert(template.id.clone(), template)
    }

    /// Retrieve a template by id, returning a cloned copy.
    pub async fn get(&self, id: &str) -> Option<WorkflowTemplate> {
        let map = self.templates.read().await;
        map.get(id).cloned()
    }

    /// List all registered templates (order is arbitrary).
    pub async fn list(&self) -> Vec<WorkflowTemplate> {
        let map = self.templates.read().await;
        map.values().cloned().collect()
    }

    /// Remove a template by id, returning it if it existed.
    pub async fn remove(&self, id: &str) -> Option<WorkflowTemplate> {
        let mut map = self.templates.write().await;
        map.remove(id)
    }

    /// Load templates from a directory. Only reads top-level `*.toml` files.
    ///
    /// Safe to call from any context — filesystem I/O and the lock acquisition
    /// are performed on a dedicated thread that is fully outside the Tokio
    /// runtime, avoiding the "cannot block the current thread from within a
    /// runtime" panic on constrained platforms (e.g. Termux/Android).
    pub fn load_templates_from_dir(&self, dir: &std::path::Path) -> usize {
        use tracing::{info, warn};

        // Phase 1 — read & parse template files (pure sync I/O, no lock needed).
        let dir = dir.to_path_buf();
        let parsed: Vec<Result<WorkflowTemplate, (std::path::PathBuf, String)>> = {
            if !dir.is_dir() {
                return 0;
            }

            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) => {
                    warn!("Cannot read template directory {}: {e}", dir.display());
                    return 0;
                }
            };

            let mut results = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                // Skip files > 1 MiB
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.len() > 1_048_576 {
                        warn!("Skipping oversized template file: {}", path.display());
                        continue;
                    }
                }
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        results.push(Err((path, e.to_string())));
                        continue;
                    }
                };
                match toml::from_str::<WorkflowTemplate>(&content) {
                    Ok(tpl) => results.push(Ok(tpl)),
                    Err(e) => results.push(Err((path, e.to_string()))),
                }
            }
            results
        };

        if parsed.is_empty() {
            return 0;
        }

        // Phase 2 — acquire the write lock and insert.
        // Use a scoped OS thread so `blocking_write()` never sees a Tokio
        // runtime context, which would panic on single-threaded /
        // thread-constrained runtimes.
        let count = std::thread::scope(|s| {
            s.spawn(|| {
                let mut map = self.templates.blocking_write();
                let mut count = 0usize;
                for result in parsed {
                    match result {
                        Ok(tpl) => {
                            info!(id = %tpl.id, name = %tpl.name, "Loaded workflow template");
                            map.insert(tpl.id.clone(), tpl);
                            count += 1;
                        }
                        Err((path, e)) => {
                            warn!("Failed to load template {}: {e}", path.display());
                        }
                    }
                }
                count
            })
            .join()
            .unwrap_or(0)
        });
        count
    }

    /// Instantiate a concrete [`Workflow`] from a template by substituting
    /// parameter values into step prompt templates.
    ///
    /// Returns an error if any required parameter is missing and has no default.
    pub fn instantiate(
        &self,
        template: &WorkflowTemplate,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<Workflow, String> {
        // Build the resolved parameter map (apply defaults, validate required).
        let mut resolved: HashMap<String, String> = HashMap::new();
        for p in &template.parameters {
            if let Some(val) = params.get(&p.name) {
                resolved.insert(p.name.clone(), value_to_string(val));
            } else if let Some(ref default) = p.default {
                resolved.insert(p.name.clone(), value_to_string(default));
            } else if p.required {
                return Err(format!("Missing required parameter: {}", p.name));
            }
        }

        // Also include any extra params the caller provided that aren't declared
        // (pass-through), so users can use ad-hoc placeholders.
        for (k, v) in params {
            resolved
                .entry(k.clone())
                .or_insert_with(|| value_to_string(v));
        }

        // Convert template steps → workflow steps.
        let steps = template
            .steps
            .iter()
            .map(|ts| {
                let mut prompt = ts.prompt_template.clone();
                for (k, v) in &resolved {
                    prompt = prompt.replace(&format!("{{{{{}}}}}", k), v);
                }
                WorkflowStep {
                    name: ts.name.clone(),
                    agent: match &ts.agent {
                        Some(a) => StepAgent::ByName { name: a.clone() },
                        None => StepAgent::ByName {
                            name: "default".into(),
                        },
                    },
                    prompt_template: prompt,
                    mode: StepMode::Sequential,
                    timeout_secs: 120,
                    error_mode: ErrorMode::Fail,
                    // Use step name as output_var so subsequent steps can reference via {{step_name}}
                    output_var: Some(ts.name.clone()),
                    inherit_context: None,
                    depends_on: ts.depends_on.clone(),
                    session_mode: None,
                }
            })
            .collect();

        Ok(Workflow {
            id: WorkflowId::new(),
            name: template.name.clone(),
            description: template.description.clone(),
            steps,
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        })
    }
}

impl Default for WorkflowTemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// WorkflowRun <-> WorkflowRunRow conversion
// ---------------------------------------------------------------------------

/// Convert a `WorkflowRun` to a flat `WorkflowRunRow` for SQLite storage.
fn workflow_run_to_row(run: &WorkflowRun) -> WorkflowRunRow {
    // The `resume_token` column in WorkflowRunRow now stores the hex-encoded
    // SHA-256 hash (not the plaintext UUID). The column name is unchanged for
    // backward compat with the SQLite schema; the value is the hash.
    let (state_str, resume_token, pause_reason, paused_at) = match &run.state {
        WorkflowRunState::Pending => ("pending".to_string(), None, None, None),
        WorkflowRunState::Running => ("running".to_string(), None, None, None),
        WorkflowRunState::Paused {
            resume_token_hash,
            reason,
            paused_at,
        } => (
            "paused".to_string(),
            Some(resume_token_hash.clone()),
            Some(reason.clone()),
            Some(paused_at.to_rfc3339()),
        ),
        WorkflowRunState::Completed => ("completed".to_string(), None, None, None),
        WorkflowRunState::Failed => ("failed".to_string(), None, None, None),
        WorkflowRunState::Cancelled => ("cancelled".to_string(), None, None, None),
    };

    let step_results_json =
        serde_json::to_string(&run.step_results).unwrap_or_else(|_| "[]".to_string());

    let paused_variables_json = if run.paused_variables.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&run.paused_variables).unwrap_or_else(|_| "{}".to_string()))
    };

    WorkflowRunRow {
        id: run.id.to_string(),
        workflow_id: run.workflow_id.to_string(),
        workflow_name: run.workflow_name.clone(),
        state: state_str,
        input: run.input.clone(),
        output: run.output.clone(),
        error: run.error.clone(),
        resume_token,
        pause_reason,
        paused_at,
        paused_step_index: run.paused_step_index.map(|i| i as i64),
        paused_variables: paused_variables_json,
        paused_current_input: run.paused_current_input.clone(),
        step_results: step_results_json,
        started_at: run.started_at.to_rfc3339(),
        completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
        created_at: run.started_at.to_rfc3339(),
    }
}

/// Convert a flat `WorkflowRunRow` back into a `WorkflowRun`.
fn row_to_workflow_run(row: &WorkflowRunRow) -> Result<WorkflowRun, String> {
    let id = WorkflowRunId(
        Uuid::parse_str(&row.id).map_err(|e| format!("invalid run id '{}': {e}", row.id))?,
    );
    let workflow_id = WorkflowId(
        Uuid::parse_str(&row.workflow_id)
            .map_err(|e| format!("invalid workflow_id '{}': {e}", row.workflow_id))?,
    );

    let state = match row.state.as_str() {
        "pending" => WorkflowRunState::Pending,
        "running" => WorkflowRunState::Running,
        "paused" => {
            // The resume_token column holds the hex SHA-256 hash of the
            // plaintext token since the at-rest hashing change. Old rows
            // that stored a plaintext UUID (pre-migration) will be rejected
            // here with a clear error rather than silently treated as a
            // valid hash — the hash is 64 hex chars while a UUID string is
            // 36 chars, so a length check is a reliable discriminator.
            let resume_token_hash = match row.resume_token.as_deref() {
                Some(s) if s.len() == 64 => s.to_string(),
                Some(old_value) => {
                    return Err(format!(
                        "run '{}' has a legacy plaintext resume_token (len={}) \
                         that cannot be used after the at-rest hashing migration; \
                         the run must be re-paused",
                        row.id,
                        old_value.len()
                    ))
                }
                None => {
                    return Err(format!(
                        "run '{}' has state=paused but no resume_token in the store",
                        row.id
                    ))
                }
            };
            let reason = row
                .pause_reason
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let paused_at = row
                .paused_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            WorkflowRunState::Paused {
                resume_token_hash,
                reason,
                paused_at,
            }
        }
        "completed" => WorkflowRunState::Completed,
        "failed" => WorkflowRunState::Failed,
        other => return Err(format!("unknown workflow run state: {other}")),
    };

    let started_at = DateTime::parse_from_rfc3339(&row.started_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("invalid started_at '{}': {e}", row.started_at))?;

    let completed_at = row
        .completed_at
        .as_deref()
        .map(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| format!("invalid completed_at '{s}': {e}"))
        })
        .transpose()?;

    let step_results: Vec<StepResult> = serde_json::from_str(&row.step_results).unwrap_or_else(|e| {
        // Corrupted JSON should never happen given workflow_run_to_row
        // serializes from a typed Vec<StepResult>, but if it does, log
        // the run_id rather than silently zero out the history. The
        // alternative (returning Err) would block boot recovery on a
        // single bad row, which is worse than visible truncation.
        warn!(
            run_id = %row.id,
            error = %e,
            "row_to_workflow_run: step_results JSON failed to parse; reloading run with empty history"
        );
        Vec::new()
    });

    let paused_variables: BTreeMap<String, String> = row
        .paused_variables
        .as_deref()
        .map(|s| {
            serde_json::from_str(s).unwrap_or_else(|e| {
                warn!(
                    run_id = %row.id,
                    error = %e,
                    "row_to_workflow_run: paused_variables JSON failed to parse; reloading run with empty variables"
                );
                BTreeMap::new()
            })
        })
        .unwrap_or_default();

    Ok(WorkflowRun {
        id,
        workflow_id,
        workflow_name: row.workflow_name.clone(),
        input: row.input.clone(),
        state,
        step_results,
        output: row.output.clone(),
        error: row.error.clone(),
        started_at,
        completed_at,
        pause_request: None,
        paused_step_index: row.paused_step_index.map(|i| i as usize),
        paused_variables,
        paused_current_input: row.paused_current_input.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workflow() -> Workflow {
        Workflow {
            id: WorkflowId::new(),
            name: "test-pipeline".to_string(),
            description: "A test pipeline".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "analyze".to_string(),
                    agent: StepAgent::ByName {
                        name: "analyst".to_string(),
                    },
                    prompt_template: "Analyze this: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "summarize".to_string(),
                    agent: StepAgent::ByName {
                        name: "writer".to_string(),
                    },
                    prompt_template: "Summarize this analysis: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        }
    }

    fn mock_resolver(agent: &StepAgent) -> Option<(AgentId, String, bool)> {
        let _ = agent;
        Some((AgentId::new(), "mock-agent".to_string(), true))
    }

    fn mock_resolver_no_inherit(agent: &StepAgent) -> Option<(AgentId, String, bool)> {
        let _ = agent;
        Some((AgentId::new(), "mock-agent".to_string(), false))
    }

    #[tokio::test]
    async fn test_register_workflow() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let id = engine.register(wf.clone()).await;
        assert_eq!(id, wf.id);

        let retrieved = engine.get_workflow(id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test-pipeline");
    }

    // Multi-thread flavor because `load_from_dir_sync` uses
    // `blocking_write` on the workflows RwLock, which panics on the default
    // current-thread runtime ("Cannot block the current thread from within
    // a runtime").
    #[tokio::test(flavor = "multi_thread")]
    async fn register_writes_atomically_and_cleans_tmp() {
        // Atomic-write invariant: after a successful register, the persisted
        // file exists at <id>.workflow.json and the staging path
        // <id>.workflow.json.tmp must NOT exist — otherwise the loader on
        // the next boot would happily skip the .tmp file (extension filter)
        // and the rename clearly didn't fire.
        let tmp = tempfile::tempdir().unwrap();
        let engine = WorkflowEngine::new_with_persistence(tmp.path());
        let wf = test_workflow();
        let id = engine.register(wf.clone()).await;

        let final_path = tmp
            .path()
            .join("workflows")
            .join(format!("{id}.workflow.json"));
        let tmp_path = tmp
            .path()
            .join("workflows")
            .join(format!("{id}.workflow.json.tmp"));
        assert!(
            final_path.exists(),
            "final file must exist after register: {}",
            final_path.display()
        );
        assert!(
            !tmp_path.exists(),
            "tmp staging file must be cleaned after successful rename: {}",
            tmp_path.display()
        );

        // The file must round-trip: load_from_dir_sync picks it up and the
        // engine recognises the registered workflow by id. Drive
        // `load_from_dir_sync` via `block_in_place` because it acquires
        // `blocking_write` internally.
        let engine2 = WorkflowEngine::new_with_persistence(tmp.path());
        let loaded = tokio::task::block_in_place(|| {
            engine2.load_from_dir_sync(&tmp.path().join("workflows"))
        });
        assert_eq!(loaded, 1, "expected exactly one workflow loaded back");
        assert!(engine2.get_workflow(id).await.is_some());

        // remove_workflow deletes the file too.
        assert!(engine.remove_workflow(id).await);
        assert!(
            !final_path.exists(),
            "persisted file must be gone after remove_workflow"
        );
    }

    #[tokio::test]
    async fn test_create_run() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;

        let run_id = engine.create_run(wf_id, "test input".to_string()).await;
        assert!(run_id.is_some());

        let run = engine.get_run(run_id.unwrap()).await.unwrap();
        assert_eq!(run.input, "test input");
        assert!(matches!(run.state, WorkflowRunState::Pending));
    }

    #[tokio::test]
    async fn test_list_workflows() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        engine.register(wf).await;

        let list = engine.list_workflows().await;
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_remove_workflow() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let id = engine.register(wf).await;

        assert!(engine.remove_workflow(id).await);
        assert!(engine.get_workflow(id).await.is_none());
    }

    #[tokio::test]
    async fn test_execute_pipeline() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "raw data".to_string())
            .await
            .unwrap();

        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 100u64, 50u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("Processed:"));

        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Completed));
        assert_eq!(run.step_results.len(), 2);
        assert!(run.output.is_some());
    }

    #[tokio::test]
    async fn test_conditional_skip() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "conditional-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "first".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "only-if-error".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Fix: {{input}}".to_string(),
                    mode: StepMode::Conditional {
                        condition: "ERROR".to_string(),
                    },
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "all good".to_string())
            .await
            .unwrap();

        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("OK: {msg}"), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Only 1 step executed (conditional was skipped)
        assert_eq!(run.step_results.len(), 1);
    }

    #[tokio::test]
    async fn test_conditional_executes() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "conditional-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "first".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "only-if-error".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Fix: {{input}}".to_string(),
                    mode: StepMode::Conditional {
                        condition: "ERROR".to_string(),
                    },
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // This sender returns output containing "ERROR"
        let sender = |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
            Ok(("Found an ERROR in the data".to_string(), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Both steps executed
        assert_eq!(run.step_results.len(), 2);
    }

    #[tokio::test]
    async fn test_loop_until_condition() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "loop-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "refine".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "Refine: {{input}}".to_string(),
                mode: StepMode::Loop {
                    max_iterations: 5,
                    until: "DONE".to_string(),
                },
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            }],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "draft".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String, _sm: Option<SessionMode>| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n >= 2 {
                    Ok(("Result: DONE".to_string(), 10u64, 5u64))
                } else {
                    Ok(("Still working...".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("DONE"));
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_loop_max_iterations() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "loop-max-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "refine".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Loop {
                    max_iterations: 3,
                    until: "NEVER_MATCH".to_string(),
                },
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            }],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let sender = |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
            Ok(("iteration output".to_string(), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        assert_eq!(run.step_results.len(), 3); // max_iterations
    }

    #[tokio::test]
    async fn test_error_mode_skip() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "skip-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "will-fail".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Skip,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "succeeds".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String, _sm: Option<SessionMode>| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    Err("simulated error".to_string())
                } else {
                    Ok(("success".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Only 1 step result (the first was skipped due to error)
        assert_eq!(run.step_results.len(), 1);
        assert!(matches!(run.state, WorkflowRunState::Completed));
    }

    #[tokio::test]
    async fn test_error_mode_retry() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "retry-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "flaky".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Retry {
                    max_retries: 2,
                    backoff_ms: None,
                    jitter_pct: None,
                },
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            }],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String, _sm: Option<SessionMode>| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    Err("transient error".to_string())
                } else {
                    Ok(("finally worked".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "finally worked");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_output_variables() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "vars-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "produce".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("first_result".to_string()),
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "transform".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("second_result".to_string()),
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "combine".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "First: {{first_result}} | Second: {{second_result}}"
                        .to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "start".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, msg: String, _sm: Option<SessionMode>| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match n {
                    0 => Ok(("alpha".to_string(), 10u64, 5u64)),
                    1 => Ok(("beta".to_string(), 10u64, 5u64)),
                    _ => Ok((format!("Combined: {msg}"), 10u64, 5u64)),
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        // The third step receives "First: alpha | Second: beta" as its prompt
        assert!(output.contains("First: alpha"));
        assert!(output.contains("Second: beta"));
    }

    #[tokio::test]
    async fn test_fan_out_parallel() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "fanout-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "task-a".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Task A: {{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "task-b".to_string(),
                    agent: StepAgent::ByName {
                        name: "b".to_string(),
                    },
                    prompt_template: "Task B: {{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "collect".to_string(),
                    agent: StepAgent::ByName {
                        name: "c".to_string(),
                    },
                    prompt_template: "unused".to_string(),
                    mode: StepMode::Collect,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Done: {msg}"), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        // Collect step outputs JSON with results array
        assert!(output.contains("Done: Task A"));
        assert!(output.contains("Done: Task B"));
        assert!(output.contains("results"));
    }

    #[tokio::test]
    async fn test_expand_variables() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("task".to_string(), "code review".to_string());

        let result = WorkflowEngine::expand_variables(
            "Hello {{name}}, please do {{task}} on {{input}}",
            "main.rs",
            &vars,
        );
        assert_eq!(result, "Hello Alice, please do code review on main.rs");
    }

    #[tokio::test]
    async fn test_error_mode_serialization() {
        let fail_json = serde_json::to_string(&ErrorMode::Fail).unwrap();
        assert_eq!(fail_json, "\"fail\"");

        let skip_json = serde_json::to_string(&ErrorMode::Skip).unwrap();
        assert_eq!(skip_json, "\"skip\"");

        let retry_json = serde_json::to_string(&ErrorMode::Retry {
            max_retries: 3,
            backoff_ms: None,
            jitter_pct: None,
        })
        .unwrap();
        let retry: ErrorMode = serde_json::from_str(&retry_json).unwrap();
        assert!(matches!(retry, ErrorMode::Retry { max_retries: 3, .. }));
    }

    #[tokio::test]
    async fn test_step_mode_conditional_serialization() {
        let mode = StepMode::Conditional {
            condition: "error".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: StepMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, StepMode::Conditional { condition } if condition == "error"));
    }

    #[tokio::test]
    async fn test_step_mode_loop_serialization() {
        let mode = StepMode::Loop {
            max_iterations: 5,
            until: "done".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: StepMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, StepMode::Loop { max_iterations: 5, until } if until == "done"));
    }

    // ---- load_from_dir_sync tests ----

    #[test]
    fn test_load_from_dir_sync_valid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let toml_content = r#"
name = "my-workflow"
description = "a workflow"

[[steps]]
name = "step1"
prompt_template = "Do {{input}}"
[steps.agent]
name = "agent-a"
"#;
        std::fs::write(dir.path().join("wf.workflow.toml"), toml_content).unwrap();

        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(dir.path());
        assert_eq!(loaded, 1);

        let workflows = engine.workflows.blocking_read();
        assert_eq!(workflows.len(), 1);
        let wf = workflows.values().next().unwrap();
        assert_eq!(wf.name, "my-workflow");
        assert_eq!(wf.steps.len(), 1);
    }

    #[test]
    fn test_load_from_dir_sync_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let json_content = r#"{
            "name": "json-workflow",
            "description": "from json",
            "steps": [{
                "name": "s1",
                "agent": { "name": "agent-b" },
                "prompt_template": "{{input}}"
            }]
        }"#;
        std::fs::write(dir.path().join("wf.workflow.json"), json_content).unwrap();

        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(dir.path());
        assert_eq!(loaded, 1);

        let workflows = engine.workflows.blocking_read();
        assert_eq!(workflows.len(), 1);
        let wf = workflows.values().next().unwrap();
        assert_eq!(wf.name, "json-workflow");
    }

    #[test]
    fn test_load_from_dir_sync_invalid_file() {
        let dir = tempfile::tempdir().unwrap();
        // Write invalid TOML that cannot parse as a Workflow
        std::fs::write(dir.path().join("bad.workflow.toml"), "not valid {{{{").unwrap();

        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(dir.path());
        assert_eq!(loaded, 0);
        assert!(engine.workflows.blocking_read().is_empty());
    }

    #[test]
    fn test_load_from_dir_sync_empty_dir() {
        let dir = tempfile::tempdir().unwrap();

        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(dir.path());
        assert_eq!(loaded, 0);
        assert!(engine.workflows.blocking_read().is_empty());
    }

    #[test]
    fn test_load_from_dir_sync_nonexistent_dir() {
        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(Path::new("/tmp/does-not-exist-workflow-dir"));
        assert_eq!(loaded, 0);
    }

    #[test]
    fn test_load_from_dir_sync_duplicate_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let _toml1 = format!(
            r#"
[id]
# WorkflowId is a newtype over Uuid, serialised transparently
id = "{id}"

[bogus]
"#
        );
        // We need to use JSON for precise control over the id field
        let json = format!(
            r#"{{
                "id": "{id}",
                "name": "dup-1",
                "description": "first",
                "steps": [{{
                    "name": "s",
                    "agent": {{"name": "a"}},
                    "prompt_template": "{{{{input}}}}"
                }}]
            }}"#
        );
        let json2 = format!(
            r#"{{
                "id": "{id}",
                "name": "dup-2",
                "description": "second",
                "steps": [{{
                    "name": "s",
                    "agent": {{"name": "a"}},
                    "prompt_template": "{{{{input}}}}"
                }}]
            }}"#
        );
        std::fs::write(dir.path().join("a.workflow.json"), &json).unwrap();
        std::fs::write(dir.path().join("b.workflow.json"), &json2).unwrap();

        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(dir.path());
        // Both files load successfully (second overwrites first)
        assert_eq!(loaded, 2);
        // But only one entry in the map since they share an ID
        let workflows = engine.workflows.blocking_read();
        assert_eq!(workflows.len(), 1);
    }

    #[test]
    fn test_load_from_dir_sync_file_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        // Create a file larger than 1 MiB
        let large_content = "x".repeat(1024 * 1024 + 1);
        std::fs::write(dir.path().join("huge.workflow.toml"), &large_content).unwrap();

        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(dir.path());
        assert_eq!(loaded, 0);
        assert!(engine.workflows.blocking_read().is_empty());
    }

    #[test]
    fn test_load_from_dir_sync_ignores_non_workflow_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("config.toml"), "[foo]\nbar = 1").unwrap();

        let engine = WorkflowEngine::new();
        let loaded = engine.load_from_dir_sync(dir.path());
        assert_eq!(loaded, 0);
    }

    #[test]
    fn test_workflow_deserialize_defaults() {
        // Verify that id and created_at get default values when omitted
        let json = r#"{
            "name": "minimal",
            "description": "no id or created_at",
            "steps": [{
                "name": "s1",
                "agent": { "name": "a" },
                "prompt_template": "{{input}}"
            }]
        }"#;
        let wf: Workflow = serde_json::from_str(json).unwrap();
        assert_eq!(wf.name, "minimal");
        // id should be a valid (non-nil) UUID
        assert_ne!(wf.id.0, Uuid::nil());
        // created_at should be roughly now (within last 5 seconds)
        let diff = Utc::now() - wf.created_at;
        assert!(diff.num_seconds() < 5);
    }

    // -- WorkflowTemplateRegistry tests --

    use librefang_types::workflow_template::{
        ParameterType, TemplateParameter, WorkflowTemplateStep,
    };

    fn test_template(id: &str) -> WorkflowTemplate {
        WorkflowTemplate {
            id: id.to_string(),
            name: format!("Template {id}"),
            description: "test".into(),
            category: None,
            parameters: vec![TemplateParameter {
                name: "lang".into(),
                description: None,
                param_type: ParameterType::String,
                default: None,
                required: true,
            }],
            steps: vec![WorkflowTemplateStep {
                name: "step1".into(),
                prompt_template: "do {{lang}}".into(),
                agent: None,
                depends_on: vec![],
            }],
            tags: vec![],
            created_at: None,
            i18n: Default::default(),
        }
    }

    #[tokio::test]
    async fn registry_register_and_get() {
        let reg = WorkflowTemplateRegistry::new();
        let tpl = test_template("t1");

        assert!(reg.get("t1").await.is_none());

        let prev = reg.register(tpl.clone()).await;
        assert!(prev.is_none());

        let fetched = reg.get("t1").await.unwrap();
        assert_eq!(fetched.id, "t1");
        assert_eq!(fetched.name, "Template t1");
    }

    #[tokio::test]
    async fn registry_register_overwrites() {
        let reg = WorkflowTemplateRegistry::new();
        reg.register(test_template("t1")).await;

        let mut updated = test_template("t1");
        updated.name = "Updated".into();
        let old = reg.register(updated).await;
        assert!(old.is_some());
        assert_eq!(old.unwrap().name, "Template t1");

        let fetched = reg.get("t1").await.unwrap();
        assert_eq!(fetched.name, "Updated");
    }

    #[tokio::test]
    async fn registry_list() {
        let reg = WorkflowTemplateRegistry::new();
        assert!(reg.list().await.is_empty());

        reg.register(test_template("a")).await;
        reg.register(test_template("b")).await;
        reg.register(test_template("c")).await;

        let all = reg.list().await;
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn registry_remove() {
        let reg = WorkflowTemplateRegistry::new();
        reg.register(test_template("r1")).await;

        let removed = reg.remove("r1").await;
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "r1");

        assert!(reg.get("r1").await.is_none());
        assert!(reg.remove("r1").await.is_none());
    }

    /// Regression test for #1764: calling `load_templates_from_dir` from inside
    /// a Tokio runtime (as the daemon does via `rt.block_on`) must not panic
    /// with "Cannot block the current thread from within a runtime".
    #[tokio::test]
    async fn load_templates_from_dir_does_not_panic_inside_runtime() {
        let reg = WorkflowTemplateRegistry::new();
        let dir = std::env::temp_dir().join("librefang_test_no_templates");
        // Non-existent directory — should return 0 without panicking.
        let count = reg.load_templates_from_dir(&dir);
        assert_eq!(count, 0);
    }

    /// Regression test for #1764: verify templates are actually loaded when
    /// called from inside a Tokio runtime context.
    #[tokio::test]
    async fn load_templates_from_dir_loads_inside_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let tpl_content = r#"
id = "regression-1764"
name = "Regression Test"
description = "test"

[[parameters]]
name = "x"
param_type = "string"
required = true

[[steps]]
name = "s1"
prompt_template = "do {{x}}"
"#;
        std::fs::write(tmp.path().join("test.toml"), tpl_content).unwrap();

        let reg = WorkflowTemplateRegistry::new();
        let count = reg.load_templates_from_dir(tmp.path());
        assert_eq!(count, 1);
        assert!(reg.get("regression-1764").await.is_some());
    }

    /// Regression test for #1764: the exact scenario that caused the panic —
    /// `current_thread` runtime (most constrained, similar to Termux).
    #[tokio::test(flavor = "current_thread")]
    async fn load_templates_from_dir_safe_on_current_thread_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let tpl_content = r#"
id = "regression-1764"
name = "Regression Test"
description = "test"

[[parameters]]
name = "x"
param_type = "string"
required = true

[[steps]]
name = "s1"
prompt_template = "do {{x}}"
"#;
        std::fs::write(tmp.path().join("test.toml"), tpl_content).unwrap();

        let reg = WorkflowTemplateRegistry::new();
        let count = reg.load_templates_from_dir(tmp.path());
        assert_eq!(count, 1);
        assert!(reg.get("regression-1764").await.is_some());
    }

    // ---- Subagent context inheritance tests ----

    #[tokio::test]
    async fn test_context_injected_in_second_step() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "raw data".to_string())
            .await
            .unwrap();

        let received_prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rp = received_prompts.clone();
        let sender = move |_id: AgentId, msg: String, _sm: Option<SessionMode>| {
            let rp = rp.clone();
            async move {
                rp.lock().unwrap().push(msg.clone());
                Ok(("Output for step".to_string(), 10u64, 5u64))
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let prompts = received_prompts.lock().unwrap();
        // First step: no previous outputs, so no context preamble
        assert!(!prompts[0].contains("[Parent workflow context]"));
        // Second step: should contain context from first step
        assert!(prompts[1].contains("[Parent workflow context]"));
        assert!(prompts[1].contains("Previous steps completed:"));
        assert!(prompts[1].contains("analyze:"));
    }

    #[tokio::test]
    async fn test_context_disabled_via_agent_manifest() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "raw data".to_string())
            .await
            .unwrap();

        let received_prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rp = received_prompts.clone();
        let sender = move |_id: AgentId, msg: String, _sm: Option<SessionMode>| {
            let rp = rp.clone();
            async move {
                rp.lock().unwrap().push(msg.clone());
                Ok(("Output".to_string(), 10u64, 5u64))
            }
        };

        // Use resolver that returns inherit_parent_context=false
        let result = engine
            .execute_run(run_id, mock_resolver_no_inherit, sender)
            .await;
        assert!(result.is_ok());

        let prompts = received_prompts.lock().unwrap();
        // Neither step should have context injected
        assert!(!prompts[0].contains("[Parent workflow context]"));
        assert!(!prompts[1].contains("[Parent workflow context]"));
    }

    #[tokio::test]
    async fn test_context_disabled_via_step_override() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "override-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "first".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "second".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Do: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: Some(false),
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "raw data".to_string())
            .await
            .unwrap();

        let received_prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rp = received_prompts.clone();
        let sender = move |_id: AgentId, msg: String, _sm: Option<SessionMode>| {
            let rp = rp.clone();
            async move {
                rp.lock().unwrap().push(msg.clone());
                Ok(("Output".to_string(), 10u64, 5u64))
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let prompts = received_prompts.lock().unwrap();
        // First step: no previous outputs, so no context preamble
        assert!(!prompts[0].contains("[Parent workflow context]"));
        // Second step: inherit_context=Some(false) overrides agent setting,
        // so no context should be injected
        assert!(!prompts[1].contains("[Parent workflow context]"));
    }
    // ---- DAG execution tests ----

    #[test]
    fn test_dag_topological_sort_simple() {
        // Linear chain: A -> B -> C
        let steps = vec![
            WorkflowStep {
                name: "A".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "B".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec!["A".to_string()],
                session_mode: None,
            },
            WorkflowStep {
                name: "C".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec!["B".to_string()],
                session_mode: None,
            },
        ];

        let layers = WorkflowEngine::topological_sort(&steps).unwrap();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0], vec![0]); // A
        assert_eq!(layers[1], vec![1]); // B
        assert_eq!(layers[2], vec![2]); // C
    }

    #[tokio::test]
    async fn test_dag_parallel_execution() {
        // A and B are independent, C depends on both
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "dag-parallel".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "A".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Task A: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("a_result".to_string()),
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "B".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Task B: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("b_result".to_string()),
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "C".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Combine: {{a_result}} + {{b_result}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    // Explicitly disable context for this step
                    inherit_context: Some(false),
                    depends_on: vec!["A".to_string(), "B".to_string()],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let received_prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rp = received_prompts.clone();
        let sender = move |_id: AgentId, msg: String, _sm: Option<SessionMode>| {
            let rp = rp.clone();
            async move {
                rp.lock().unwrap().push(msg.clone());
                Ok(("done".to_string(), 10u64, 5u64))
            }
        };

        // Agent says inherit=true, but step overrides to false
        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let prompts = received_prompts.lock().unwrap();
        // Second step should NOT have context despite agent allowing it
        assert!(!prompts[1].contains("[Parent workflow context]"));
    }

    #[test]
    fn test_build_context_prompt_no_results() {
        let step = WorkflowStep {
            name: "s".to_string(),
            agent: StepAgent::ByName {
                name: "a".to_string(),
            },
            prompt_template: "do it".to_string(),
            mode: StepMode::Sequential,
            timeout_secs: 10,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
            session_mode: None,
        };
        let result = WorkflowEngine::build_context_prompt("hello", &step, 0, "wf", &[], true);
        // No previous results => no preamble
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_build_context_prompt_with_results() {
        let step = WorkflowStep {
            name: "s2".to_string(),
            agent: StepAgent::ByName {
                name: "a".to_string(),
            },
            prompt_template: "do it".to_string(),
            mode: StepMode::Sequential,
            timeout_secs: 10,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
            session_mode: None,
        };
        let results = vec![StepResult {
            step_name: "s1".to_string(),
            agent_id: "id-1".to_string(),
            agent_name: "agent-1".to_string(),
            prompt: "do analysis".to_string(),
            output: "analysis complete".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            duration_ms: 100,
        }];
        let prompt = WorkflowEngine::build_context_prompt(
            "summarize",
            &step,
            1,
            "my-pipeline",
            &results,
            true,
        );
        assert!(prompt.contains("[Parent workflow context]"));
        assert!(prompt.contains("Workflow: my-pipeline"));
        assert!(prompt.contains("- s1: analysis complete"));
        assert!(prompt.ends_with("summarize"));
    }

    #[test]
    fn test_build_context_prompt_truncates_long_output() {
        let step = WorkflowStep {
            name: "s2".to_string(),
            agent: StepAgent::ByName {
                name: "a".to_string(),
            },
            prompt_template: "do it".to_string(),
            mode: StepMode::Sequential,
            timeout_secs: 10,
            error_mode: ErrorMode::Fail,
            output_var: None,
            inherit_context: None,
            depends_on: vec![],
            session_mode: None,
        };
        let results = vec![StepResult {
            step_name: "s1".to_string(),
            agent_id: "id-1".to_string(),
            agent_name: "agent-1".to_string(),
            prompt: "do it".to_string(),
            output: "x".repeat(2000),
            input_tokens: 10,
            output_tokens: 5,
            duration_ms: 100,
        }];
        let prompt = WorkflowEngine::build_context_prompt("next", &step, 1, "wf", &results, true);
        assert!(prompt.contains("..."));
        // The full 2000-char output should NOT appear
        assert!(!prompt.contains(&"x".repeat(2000)));
    }

    #[test]
    fn test_inherit_context_step_field_serde_default() {
        // When inherit_context is omitted from JSON, it should default to None
        let json = r#"{
            "name": "s1",
            "agent": { "name": "a" },
            "prompt_template": "{{input}}"
        }"#;
        let step: WorkflowStep = serde_json::from_str(json).unwrap();
        assert!(step.inherit_context.is_none());
    }

    #[test]
    fn test_inherit_context_step_field_explicit_false() {
        let json = r#"{
            "name": "s1",
            "agent": { "name": "a" },
            "prompt_template": "{{input}}",
            "inherit_context": false
        }"#;
        let step: WorkflowStep = serde_json::from_str(json).unwrap();
        assert_eq!(step.inherit_context, Some(false));
    }

    #[test]
    fn test_dag_topological_sort_layers() {
        // Verify topological order: A and B in first layer, C in second
        let layers = WorkflowEngine::topological_sort(&[
            WorkflowStep {
                name: "A".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "B".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            },
            WorkflowStep {
                name: "C".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec!["A".to_string(), "B".to_string()],
                session_mode: None,
            },
        ])
        .unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].len(), 2); // A and B in parallel
        assert_eq!(layers[1], vec![2]); // C
    }

    #[test]
    fn test_dag_cycle_detection() {
        // A -> B -> A (cycle)
        let steps = vec![
            WorkflowStep {
                name: "A".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec!["B".to_string()],
                session_mode: None,
            },
            WorkflowStep {
                name: "B".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec!["A".to_string()],
                session_mode: None,
            },
        ];

        let result = WorkflowEngine::topological_sort(&steps);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cycle detected"));
    }

    #[test]
    fn test_workflow_to_template_preserves_depends_on_graph() {
        let workflow = Workflow {
            id: WorkflowId::new(),
            name: "dag template".to_string(),
            description: "preserve dependencies".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "A".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "step a".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "B".to_string(),
                    agent: StepAgent::ByName {
                        name: "b".to_string(),
                    },
                    prompt_template: "step b".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "C".to_string(),
                    agent: StepAgent::ByName {
                        name: "c".to_string(),
                    },
                    prompt_template: "step c".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec!["A".to_string(), "B".to_string()],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };

        let template = WorkflowEngine::workflow_to_template(&workflow);

        assert!(template.steps[0].depends_on.is_empty());
        assert!(template.steps[1].depends_on.is_empty());
        assert_eq!(
            template.steps[2].depends_on,
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[tokio::test]
    async fn test_dag_dependency_failure_propagation() {
        // A fails, B depends on A -> B should be skipped and workflow fails
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "dag-fail-prop".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "A".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "B".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec!["A".to_string()],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };

        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Sender always fails
        let sender = |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async move {
            Err::<(String, u64, u64), String>("simulated failure".to_string())
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed"));

        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Failed));
        // A failed, B was never attempted
        assert_eq!(run.step_results.len(), 0);
    }

    // -- Persistence tests ---------------------------------------------------

    /// Helper: build a WorkflowRun in a terminal state for persistence tests.
    fn make_terminal_run(state: WorkflowRunState) -> WorkflowRun {
        WorkflowRun {
            id: WorkflowRunId::new(),
            workflow_id: WorkflowId::new(),
            workflow_name: "persist-test".to_string(),
            input: "hello".to_string(),
            state,
            step_results: vec![StepResult {
                step_name: "step-1".to_string(),
                agent_id: "agent-abc".to_string(),
                agent_name: "test-agent".to_string(),
                prompt: "test prompt".to_string(),
                output: "done".to_string(),
                input_tokens: 10,
                output_tokens: 20,
                duration_ms: 100,
            }],
            output: Some("final output".to_string()),
            error: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            pause_request: None,
            paused_step_index: None,
            paused_variables: BTreeMap::new(),
            paused_current_input: None,
        }
    }

    #[test]
    fn test_persist_and_load_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let run_completed = make_terminal_run(WorkflowRunState::Completed);
        let run_failed = {
            let mut r = make_terminal_run(WorkflowRunState::Failed);
            r.error = Some("something went wrong".to_string());
            r.output = None;
            r
        };
        let completed_id = run_completed.id;
        let failed_id = run_failed.id;

        // Persist runs from one engine instance.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            engine.runs.insert(run_completed.id, run_completed);
            engine.runs.insert(run_failed.id, run_failed);
            engine.persist_runs();
        }

        // Load into a fresh engine and verify.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            let count = engine.load_runs().unwrap();
            assert_eq!(count, 2);

            let c = engine
                .runs
                .get(&completed_id)
                .expect("completed run missing");
            assert!(matches!(c.state, WorkflowRunState::Completed));
            assert_eq!(c.workflow_name, "persist-test");
            assert_eq!(c.output.as_deref(), Some("final output"));
            assert_eq!(c.step_results.len(), 1);
            assert_eq!(c.step_results[0].step_name, "step-1");

            let f = engine.runs.get(&failed_id).expect("failed run missing");
            assert!(matches!(f.state, WorkflowRunState::Failed));
            assert_eq!(f.error.as_deref(), Some("something went wrong"));
        }
    }

    #[test]
    fn test_persist_skips_running_state() {
        let tmp = tempfile::tempdir().unwrap();
        let completed = make_terminal_run(WorkflowRunState::Completed);
        let completed_id = completed.id;
        let running = WorkflowRun {
            id: WorkflowRunId::new(),
            workflow_id: WorkflowId::new(),
            workflow_name: "in-progress".to_string(),
            input: "data".to_string(),
            state: WorkflowRunState::Running,
            step_results: vec![],
            output: None,
            error: None,
            started_at: Utc::now(),
            completed_at: None,
            pause_request: None,
            paused_step_index: None,
            paused_variables: BTreeMap::new(),
            paused_current_input: None,
        };
        let running_id = running.id;

        // Persist — should only write the completed run, not the running one.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            engine.runs.insert(completed.id, completed);
            engine.runs.insert(running.id, running);
            engine.persist_runs();
        }

        // Load and verify only 1 run came back.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            let count = engine.load_runs().unwrap();
            assert_eq!(count, 1);

            assert!(engine.runs.contains_key(&completed_id));
            assert!(!engine.runs.contains_key(&running_id));
        }
    }

    /// Regression for #3335: graceful shutdown must transition every
    /// in-flight run to `Paused` and persist the change so the dashboard
    /// still surfaces them after a restart.
    ///
    /// Pre-fix the same scenario produced an empty list on the second
    /// boot — `persist_runs` filters out Running/Pending, so a daemon
    /// stop with three in-flight runs left only the unrelated Completed
    /// row in `workflow_runs.json`.
    #[test]
    fn drain_on_shutdown_pauses_running_and_pending_and_persists() {
        let tmp = tempfile::tempdir().unwrap();

        let completed = make_terminal_run(WorkflowRunState::Completed);
        let completed_id = completed.id;

        // Failed is the other terminal state — drain must skip it just
        // like Completed. Without an explicit assertion below, a future
        // edit that loosens the matches!() guard could regress without
        // a single test failing.
        let failed = WorkflowRun {
            state: WorkflowRunState::Failed,
            ..make_terminal_run(WorkflowRunState::Pending)
        };
        let failed_id = failed.id;

        let running = WorkflowRun {
            state: WorkflowRunState::Running,
            ..make_terminal_run(WorkflowRunState::Pending)
        };
        let running_id = running.id;

        let pending = WorkflowRun {
            state: WorkflowRunState::Pending,
            ..make_terminal_run(WorkflowRunState::Pending)
        };
        let pending_id = pending.id;

        // Pre-existing Paused run must survive untouched (drain only
        // touches Running/Pending) — proves we don't clobber an
        // already-paused workflow's resume_token_hash.
        let preexisting_paused_token = Uuid::new_v4();
        let preexisting_paused_hash = WorkflowEngine::hash_resume_token(&preexisting_paused_token);
        let preexisting_paused = WorkflowRun {
            state: WorkflowRunState::Paused {
                resume_token_hash: preexisting_paused_hash.clone(),
                reason: "user pause".to_string(),
                paused_at: Utc::now(),
            },
            ..make_terminal_run(WorkflowRunState::Pending)
        };
        let preexisting_paused_id = preexisting_paused.id;

        // Drive shutdown drain on one engine, then reload from the
        // persisted JSON to prove durability across the restart
        // boundary.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            engine.runs.insert(completed.id, completed);
            engine.runs.insert(failed.id, failed);
            engine.runs.insert(running.id, running);
            engine.runs.insert(pending.id, pending);
            engine
                .runs
                .insert(preexisting_paused.id, preexisting_paused);

            let drained = engine.drain_on_shutdown();
            assert_eq!(
                drained, 2,
                "drain must transition exactly the Running + Pending pair \
                 (Completed / Failed / Paused must be skipped)"
            );
        }

        let engine = WorkflowEngine::new_with_persistence(tmp.path());
        let count = engine.load_runs().unwrap();
        assert_eq!(
            count, 5,
            "all five runs (Completed + Failed + drained pair + preexisting Paused) must reload"
        );

        let c = engine.runs.get(&completed_id).expect("completed missing");
        assert!(matches!(c.state, WorkflowRunState::Completed));

        let f = engine.runs.get(&failed_id).expect("failed missing");
        assert!(
            matches!(f.state, WorkflowRunState::Failed),
            "Failed must not be drained: {:?}",
            f.state
        );

        let r = engine.runs.get(&running_id).expect("running missing");
        match &r.state {
            WorkflowRunState::Paused { reason, .. } => {
                assert_eq!(reason, "Interrupted by daemon shutdown");
            }
            other => panic!("running run not paused: {:?}", other),
        }

        let p = engine.runs.get(&pending_id).expect("pending missing");
        match &p.state {
            WorkflowRunState::Paused { reason, .. } => {
                assert_eq!(reason, "Interrupted by daemon shutdown");
            }
            other => panic!("pending run not paused: {:?}", other),
        }

        let pre = engine
            .runs
            .get(&preexisting_paused_id)
            .expect("preexisting paused missing");
        match &pre.state {
            WorkflowRunState::Paused {
                resume_token_hash,
                reason,
                ..
            } => {
                assert_eq!(
                    resume_token_hash, &preexisting_paused_hash,
                    "preexisting hash must survive drain_on_shutdown"
                );
                assert_eq!(reason, "user pause");
            }
            other => panic!("preexisting paused was rewritten: {:?}", other),
        }
    }

    /// `drain_on_shutdown` returns 0 and does not touch disk when there
    /// is nothing to drain — important so an idle daemon's stop sequence
    /// does not gratuitously rewrite `workflow_runs.json` on every shutdown.
    #[test]
    fn drain_on_shutdown_is_a_noop_when_no_in_flight_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = WorkflowEngine::new_with_persistence(tmp.path());
        engine.runs.insert(
            WorkflowRunId::new(),
            make_terminal_run(WorkflowRunState::Completed),
        );

        let drained = engine.drain_on_shutdown();
        assert_eq!(drained, 0);

        // No persistence write happened — `workflow_runs.json` should
        // still be absent because nothing in the engine triggered the
        // write path.
        let runs_json = tmp.path().join("data").join("workflow_runs.json");
        assert!(
            !runs_json.exists(),
            "drain_on_shutdown must not write when nothing was drained"
        );
    }

    #[test]
    fn test_load_runs_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = WorkflowEngine::new_with_persistence(tmp.path());
        let count = engine.load_runs().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_load_runs_no_persistence_path() {
        let engine = WorkflowEngine::new();
        let count = engine.load_runs().unwrap();
        assert_eq!(count, 0);
    }

    // --- evaluate_condition tests ----------------------------------------

    #[test]
    fn test_evaluate_condition_simple_contains() {
        assert!(evaluate_condition("hello world", "hello"));
        assert!(!evaluate_condition("hello world", "goodbye"));
    }

    #[test]
    fn test_evaluate_condition_negation() {
        assert!(evaluate_condition("hello world", "!goodbye"));
        assert!(!evaluate_condition("hello world", "!hello"));
    }

    #[test]
    fn test_evaluate_condition_and() {
        assert!(evaluate_condition("hello world", "hello && world"));
        assert!(!evaluate_condition("hello world", "hello && goodbye"));
    }

    #[test]
    fn test_evaluate_condition_or() {
        assert!(evaluate_condition("hello world", "hello || goodbye"));
        assert!(evaluate_condition("hello world", "goodbye || hello"));
        assert!(!evaluate_condition("hello world", "goodbye || missing"));
    }

    #[test]
    fn test_evaluate_condition_combined_and_or() {
        // OR has lower precedence: parsed as (a && b) || c
        assert!(evaluate_condition(
            "hello world",
            "hello && world || goodbye"
        ));
        // First AND branch fails, but OR branch succeeds
        assert!(evaluate_condition(
            "hello world",
            "hello && goodbye || world"
        ));
        // Both branches fail
        assert!(!evaluate_condition(
            "hello world",
            "missing && goodbye || absent"
        ));
    }

    #[test]
    fn test_evaluate_condition_negation_and() {
        // !missing is true, hello is true => true
        assert!(evaluate_condition("hello world", "!missing && hello"));
        // !hello is false, world is true => false (AND requires both)
        assert!(!evaluate_condition("hello world", "!hello && world"));
    }

    #[test]
    fn test_evaluate_condition_negation_or() {
        // !hello is false, world is true => true
        assert!(evaluate_condition("hello world", "!hello || world"));
        // !hello is false, missing is false => false
        assert!(!evaluate_condition("hello world", "!hello || !world"));
    }

    #[test]
    fn test_evaluate_condition_case_insensitivity() {
        // The condition is lowercased internally, so uppercase conditions
        // should still match lowercase input.
        assert!(evaluate_condition("hello world", "HELLO"));
        assert!(evaluate_condition("hello world", "Hello && World"));
        assert!(evaluate_condition("hello world", "!GOODBYE"));
    }

    #[test]
    fn test_evaluate_condition_empty_and_whitespace() {
        // Empty condition => empty string is always contained in any string
        assert!(evaluate_condition("hello world", ""));
        assert!(evaluate_condition("hello world", "  "));
        // Empty input with non-empty condition
        assert!(!evaluate_condition("", "hello"));
        // Both empty
        assert!(evaluate_condition("", ""));
    }

    // -- #3335 pause / resume tests ----------------------------------------

    #[tokio::test]
    async fn pause_after_first_step_then_resume_finishes_workflow() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let engine = Arc::new(WorkflowEngine::new());
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine
            .create_run(wf_id, "raw data".to_string())
            .await
            .unwrap();

        // Shared state: capture the resume token from inside the sender so
        // the test can present it later. The first sender invocation
        // triggers `pause_run` so the loop honors the request *before*
        // step 2 — verifying that pause respects step boundaries.
        let captured_token = Arc::new(std::sync::Mutex::new(None::<Uuid>));
        let pause_requested = Arc::new(AtomicBool::new(false));

        let engine_for_sender = Arc::clone(&engine);
        let captured_for_sender = Arc::clone(&captured_token);
        let pause_for_sender = Arc::clone(&pause_requested);

        let sender = move |_id: AgentId, msg: String, _sm: Option<SessionMode>| {
            let engine = Arc::clone(&engine_for_sender);
            let captured = Arc::clone(&captured_for_sender);
            let pause_flag = Arc::clone(&pause_for_sender);
            async move {
                if !pause_flag.swap(true, Ordering::SeqCst) {
                    let token = engine
                        .pause_run(run_id, "test pause request")
                        .await
                        .expect("pause_run should succeed on a Running workflow");
                    *captured.lock().unwrap() = Some(token);
                }
                Ok((format!("Processed: {msg}"), 100_u64, 50_u64))
            }
        };

        // First execute_run: runs step 1, sender requests pause, loop
        // honors at the next step boundary (before step 2).
        engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .expect("execute_run should pause cleanly without erroring");

        let run = engine.get_run(run_id).await.unwrap();
        assert!(
            matches!(run.state, WorkflowRunState::Paused { .. }),
            "expected Paused, got {:?}",
            run.state
        );
        assert_eq!(
            run.paused_step_index,
            Some(1),
            "should pause before step index 1 (step 2)"
        );
        assert_eq!(
            run.step_results.len(),
            1,
            "exactly one step should have completed before the pause"
        );

        let token = captured_token
            .lock()
            .unwrap()
            .expect("sender should have captured a token");

        // Resume: replay from saved snapshot, executing the remaining step.
        let sender_resume = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 100_u64, 50_u64))
        };
        let result = engine
            .resume_run(run_id, token, mock_resolver, sender_resume)
            .await
            .expect("resume_run should succeed");
        assert!(result.contains("Processed:"));

        let run = engine.get_run(run_id).await.unwrap();
        assert!(
            matches!(run.state, WorkflowRunState::Completed),
            "expected Completed, got {:?}",
            run.state
        );
        assert_eq!(run.step_results.len(), 2);
        assert!(
            run.paused_step_index.is_none(),
            "snapshot should be cleared after resume"
        );
        assert!(run.paused_variables.is_empty());
        assert!(run.paused_current_input.is_none());
    }

    #[tokio::test]
    async fn resume_run_with_wrong_token_is_rejected() {
        let engine = WorkflowEngine::new();
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Pause-before-execute: loop will pause at step 0 immediately,
        // giving us a Paused state with a known token.
        let real_token = engine
            .pause_run(run_id, "before-start pause")
            .await
            .unwrap();
        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .unwrap();

        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Paused { .. }));

        let bogus_token = Uuid::new_v4();
        assert_ne!(bogus_token, real_token);
        let sender2 = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        let err = engine
            .resume_run(run_id, bogus_token, mock_resolver, sender2)
            .await
            .expect_err("resume_run with wrong token must error");
        assert!(
            matches!(err, ResumeRunError::TokenMismatch { .. }),
            "expected TokenMismatch, got: {err}"
        );

        // Run is still Paused — a failed resume attempt does not flip state.
        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Paused { .. }));
    }

    #[tokio::test]
    async fn resume_run_on_non_paused_run_is_rejected() {
        let engine = WorkflowEngine::new();
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();
        // No pause requested — run is in Pending.
        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        let err = engine
            .resume_run(run_id, Uuid::new_v4(), mock_resolver, sender)
            .await
            .expect_err("resume_run on non-paused run must error");
        assert!(
            matches!(err, ResumeRunError::NotPaused { .. }),
            "expected NotPaused, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paused_run_round_trips_through_persist_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let original_hash: String;
        let original_run_id: WorkflowRunId;

        // Phase 1: build a paused run on engine instance #1, then persist.
        {
            let engine = Arc::new(WorkflowEngine::new_with_persistence(tmp.path()));
            let wf_id = engine.register(test_workflow()).await;
            let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();
            original_run_id = run_id;
            // pause_run now returns the plaintext token; compute its hash for
            // later verification (the hash is what's persisted).
            let plaintext_token = engine
                .pause_run(run_id, "before-start pause")
                .await
                .unwrap();
            original_hash = WorkflowEngine::hash_resume_token(&plaintext_token);
            let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
                Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
            };
            engine
                .execute_run(run_id, mock_resolver, sender)
                .await
                .unwrap();
            // `execute_run` ends with `persist_runs_async().await`, which
            // routes through `spawn_blocking` and respects the runtime.
            // Awaiting `execute_run` already implies the persist completed.
        }

        // Phase 2: load from disk into a fresh engine, verify the
        // Paused-state run came back with its hash + snapshot intact.
        // `load_runs` uses `blocking_write` internally, so wrap in
        // `block_in_place` (requires multi-thread runtime, which this
        // test selects via the `flavor` attribute).
        let engine = WorkflowEngine::new_with_persistence(tmp.path());
        let count = tokio::task::block_in_place(|| engine.load_runs()).unwrap();
        assert_eq!(count, 1);
        let run = engine.get_run(original_run_id).await.unwrap();
        match &run.state {
            WorkflowRunState::Paused {
                resume_token_hash, ..
            } => {
                assert_eq!(
                    resume_token_hash, &original_hash,
                    "persisted resume_token_hash must match across daemon restart"
                );
            }
            other => panic!("expected Paused after reload, got {:?}", other),
        }
        assert_eq!(run.paused_step_index, Some(0));
    }

    /// A Pending run created on engine #1 must survive a crash that
    /// happens before any state transition or `persist_runs` call.
    /// This exercises the create_run -> upsert_run_to_store wiring; the
    /// batch `persist_runs_to_sqlite` only fires at end-of-execute_run /
    /// end-of-resume / drain_on_shutdown, so without per-row upsert at
    /// create time, a crash in the dispatch window loses the run.
    #[tokio::test(flavor = "multi_thread")]
    async fn pending_run_survives_crash_before_first_persist() {
        use r2d2::Pool;
        use r2d2_sqlite::SqliteConnectionManager;

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("workflows.db");

        let make_store = || {
            let pool = Pool::builder()
                .max_size(2)
                .build(SqliteConnectionManager::file(&db_path))
                .unwrap();
            librefang_memory::migration::run_migrations(&pool.get().unwrap())
                .expect("migrations must apply");
            librefang_memory::WorkflowStore::new(pool)
        };

        let original_run_id: WorkflowRunId;

        // Phase 1: create a Pending run, then drop the engine WITHOUT
        // calling execute_run / persist_runs. The crash window we care
        // about is between insert into the DashMap and first dispatch
        // — historically a daemon kill here lost the run.
        {
            let store = make_store();
            let engine = WorkflowEngine::new_with_store(store, tmp.path());
            let wf_id = engine.register(test_workflow()).await;
            original_run_id = engine
                .create_run(wf_id, "data".to_string())
                .await
                .expect("create_run must succeed");
            // Confirm the run is Pending in memory.
            let run = engine.get_run(original_run_id).await.unwrap();
            assert!(matches!(run.state, WorkflowRunState::Pending));
            // Engine drops here. No `persist_runs_async`. No execute.
        }

        // Phase 2: re-open the SAME database file with a fresh store
        // and engine, load runs, assert the Pending row came back.
        let store = make_store();
        let engine = WorkflowEngine::new_with_store(store, tmp.path());
        let count =
            tokio::task::block_in_place(|| engine.load_runs()).expect("load_runs must succeed");
        assert_eq!(count, 1, "expected exactly one persisted run");
        let run = engine
            .get_run(original_run_id)
            .await
            .expect("Pending run must be reloadable");
        assert!(
            matches!(run.state, WorkflowRunState::Pending),
            "expected Pending after reload, got {:?}",
            run.state
        );
    }

    #[tokio::test]
    async fn pause_request_on_dag_workflow_returns_explicit_error() {
        let engine = Arc::new(WorkflowEngine::new());
        // Build a 2-step workflow whose second step `depends_on` the
        // first — that's enough for `execute_run` to route into the
        // DAG executor.
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "dag".into(),
            description: "".into(),
            steps: vec![
                WorkflowStep {
                    name: "a".into(),
                    agent: StepAgent::ByName { name: "x".into() },
                    prompt_template: "{{input}}".into(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "b".into(),
                    agent: StepAgent::ByName { name: "y".into() },
                    prompt_template: "{{input}}".into(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec!["a".into()],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Lodge a pause request *before* execute_run — DAG executor must
        // refuse cleanly rather than silently dropping the request.
        let _ = engine.pause_run(run_id, "dag pause").await.unwrap();
        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        let err = engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .expect_err("DAG executor with pause request must return an explicit error");
        assert!(
            err.contains("DAG"),
            "error should mention DAG limitation: {err}"
        );

        // Refuse path must also flip the run to Failed and clear the
        // lingering pause_request — otherwise a buggy caller sees a run
        // stuck in Running with a pause_request that nothing will ever
        // honor (review feedback on #3418).
        let run = engine.get_run(run_id).await.unwrap();
        assert!(
            matches!(run.state, WorkflowRunState::Failed),
            "DAG refuse must mark the run Failed, got {:?}",
            run.state
        );
        assert!(
            run.pause_request.is_none(),
            "pause_request must be cleared after DAG refuse"
        );
        assert!(run.error.as_deref().unwrap_or("").contains("DAG"));
    }

    #[tokio::test]
    async fn pause_run_second_call_returns_already_paused_with_hash() {
        let engine = WorkflowEngine::new();
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // First call returns the plaintext token.
        let token1 = engine.pause_run(run_id, "first").await.unwrap();
        let expected_hash = WorkflowEngine::hash_resume_token(&token1);

        // Second call (pause_request already set) returns AlreadyPaused with
        // the existing hash so callers can confirm idempotency.
        let err2 = engine
            .pause_run(run_id, "second")
            .await
            .expect_err("second pause_run must return AlreadyPaused");
        match &err2 {
            PauseRunError::AlreadyPaused {
                resume_token_hash, ..
            } => {
                assert_eq!(
                    resume_token_hash, &expected_hash,
                    "AlreadyPaused must echo the stored hash"
                );
            }
            other => panic!("expected AlreadyPaused, got: {other:?}"),
        }

        // Reason from the *first* call wins — later calls must not
        // overwrite the message that surfaces in logs / UI.
        let run = engine.get_run(run_id).await.unwrap();
        let lodged = run.pause_request.expect("request should be present");
        assert_eq!(lodged.reason, "first");
    }

    #[tokio::test]
    async fn resume_run_after_completion_is_rejected() {
        let engine = Arc::new(WorkflowEngine::new());
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Pause-before-execute → loop pauses at step 0 → resume runs
        // the workflow to completion.
        let token = engine.pause_run(run_id, "before-start").await.unwrap();
        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .unwrap();
        let sender2 = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        engine
            .resume_run(run_id, token, mock_resolver, sender2)
            .await
            .unwrap();

        // Run is now Completed. A second resume_run with the same token
        // must error rather than silently re-running the workflow.
        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Completed));
        let sender3 = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        let err = engine
            .resume_run(run_id, token, mock_resolver, sender3)
            .await
            .expect_err("double-resume on a completed run must error");
        assert!(
            matches!(
                err,
                ResumeRunError::NotPaused {
                    state: "completed",
                    ..
                }
            ),
            "expected NotPaused(completed), got: {err}"
        );
    }

    #[tokio::test]
    async fn pause_then_execute_on_pending_pauses_at_step_zero() {
        let engine = WorkflowEngine::new();
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Run is Pending — pause is lodged before any step has executed.
        let token = engine.pause_run(run_id, "pre-start").await.unwrap();
        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .expect("pause-at-zero path must not error");

        let run = engine.get_run(run_id).await.unwrap();
        let expected_hash = WorkflowEngine::hash_resume_token(&token);
        match &run.state {
            WorkflowRunState::Paused {
                resume_token_hash, ..
            } => {
                assert_eq!(resume_token_hash, &expected_hash);
            }
            other => panic!("expected Paused, got {:?}", other),
        }
        assert_eq!(
            run.paused_step_index,
            Some(0),
            "pre-start pause should snapshot at step index 0"
        );
        assert!(
            run.step_results.is_empty(),
            "no steps should have executed before the pause"
        );
    }

    /// Regression for #3716: the pause-gate must take pause_request and
    /// transition state to Paused atomically under one shard lock. If a
    /// concurrent pause_run() lodges a fresh request between the take and
    /// the state-set, the state would carry the old token while
    /// pause_request held a new token — breaking resume.
    ///
    /// We assert the simpler invariant after a single pause+honor cycle:
    /// once execute_run returns having honored a pause, pause_request is
    /// cleared AND the resume_token_hash in state matches the hash of the
    /// token returned by pause_run. Both must be true, simultaneously, on
    /// the same run.
    #[tokio::test]
    async fn pause_take_and_state_set_are_atomic() {
        let engine = WorkflowEngine::new();
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "x".to_string()).await.unwrap();
        let token = engine.pause_run(run_id, "atomic-take").await.unwrap();
        let sender = |_id: AgentId, msg: String, _sm: Option<SessionMode>| async move {
            Ok((format!("R:{msg}"), 1_u64, 1_u64))
        };
        engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .expect("pause must not error");

        let run = engine.get_run(run_id).await.unwrap();
        // pause_request was taken under the same lock as state set.
        assert!(run.pause_request.is_none(), "pause_request must be taken");
        // The hash in state must correspond to the token pause_run returned —
        // not some stale value left from a split-lock race.
        let expected_hash = WorkflowEngine::hash_resume_token(&token);
        match run.state {
            WorkflowRunState::Paused {
                resume_token_hash, ..
            } => {
                assert_eq!(
                    resume_token_hash, expected_hash,
                    "hash mismatch implies torn pause"
                );
            }
            other => panic!("expected Paused, got {:?}", other),
        }
    }

    /// Regression for #3717: writes to different runs must not block each
    /// other. With the legacy `RwLock<HashMap>` design, two concurrent
    /// writers would serialize even on different keys; with the current
    /// `DashMap`-based `runs` field they hit independent shards. We spawn
    /// many concurrent pause_run calls against distinct run_ids and
    /// assert they all complete — a simple liveness check that fails
    /// fast if a single global lock is reintroduced.
    #[tokio::test]
    async fn concurrent_pause_run_does_not_serialize_across_runs() {
        let engine = std::sync::Arc::new(WorkflowEngine::new());
        let wf_id = engine.register(test_workflow()).await;

        let mut run_ids = Vec::with_capacity(32);
        for _ in 0..32 {
            run_ids.push(engine.create_run(wf_id, "data".to_string()).await.unwrap());
        }

        let mut handles = Vec::with_capacity(run_ids.len());
        for rid in run_ids.iter().copied() {
            let e = engine.clone();
            handles.push(tokio::spawn(
                async move { e.pause_run(rid, "concurrent").await },
            ));
        }
        for h in handles {
            h.await.unwrap().expect("pause_run must succeed");
        }
        for rid in &run_ids {
            let r = engine.get_run(*rid).await.unwrap();
            assert!(
                r.pause_request.is_some(),
                "every run should carry a pause_request"
            );
        }
    }

    #[test]
    fn classify_backoff_burst_always_65s() {
        assert_eq!(
            classify_backoff("Token burst limit would be exceeded", 0),
            std::time::Duration::from_secs(65)
        );
        assert_eq!(
            classify_backoff("Token burst limit would be exceeded", 5),
            std::time::Duration::from_secs(65)
        );
    }

    #[test]
    fn classify_backoff_rate_limit_always_65s() {
        assert_eq!(
            classify_backoff("Tool call rate limit exceeded: 10 per minute", 0),
            std::time::Duration::from_secs(65)
        );
    }

    /// Provider error strings come in every casing under the sun
    /// (Anthropic: "rate limit", OpenAI: "Rate limit"/"rate_limit",
    /// Gemini: "RATE_LIMIT_EXCEEDED"). Without case-insensitive matching
    /// the burst window kicks in only for Anthropic and everything else
    /// falls through to exponential — exactly the bug v1 of this
    /// classifier shipped with.
    #[test]
    fn classify_backoff_rate_limit_is_case_insensitive() {
        for variant in [
            "Rate Limit Exceeded",
            "RATE LIMIT EXCEEDED",
            "rate_limit_exceeded",
            "RATE_LIMIT_EXCEEDED",
            "OpenAI: Rate limit reached for gpt-4",
            "Gemini error: RATE_LIMIT_EXCEEDED for project foo",
            "Token Burst Limit Reached",
        ] {
            assert_eq!(
                classify_backoff(variant, 0),
                std::time::Duration::from_secs(65),
                "expected 65s burst-window backoff for variant: {variant}"
            );
        }
    }

    /// `Retry-After` from the upstream provider beats the constant 65s.
    /// A server explicitly asking for 120s must NOT be retried at 65s
    /// and 429ed again — that's exactly the loop the original v1
    /// classifier failed to break. Driver 429 messages frequently inline
    /// the upstream HTTP `Retry-After` header value.
    #[test]
    fn classify_backoff_honours_retry_after_seconds() {
        assert_eq!(
            classify_backoff("HTTP 429 from provider — Retry-After: 120 (rate limit)", 0),
            std::time::Duration::from_secs(120)
        );
    }

    /// `LlmError::RateLimited` and `LlmError::Overloaded` Display as
    /// `"... retry after Nms ..."`. The kernel sees this string verbatim
    /// (the runtime stringifies the driver error before bubbling). Pin
    /// the contract against the *real* driver Display output — using a
    /// synthetic `retry_after_ms=N` form here would silently let the two
    /// parsers drift, which is exactly the gap that motivated this test.
    #[test]
    fn classify_backoff_honours_llm_error_display_strings() {
        use librefang_llm_driver::LlmError;

        let rate_limited = LlmError::RateLimited {
            retry_after_ms: 8000,
            message: Some("hit your limit · resets 10am".to_string()),
        };
        assert_eq!(
            classify_backoff(&rate_limited.to_string(), 0),
            std::time::Duration::from_millis(8000),
            "RateLimited Display: {}",
            rate_limited
        );

        let overloaded = LlmError::Overloaded {
            retry_after_ms: 12_000,
        };
        assert_eq!(
            classify_backoff(&overloaded.to_string(), 0),
            std::time::Duration::from_millis(12_000),
            "Overloaded Display: {}",
            overloaded
        );
    }

    /// A hostile or buggy provider returning Retry-After: 86400 (one
    /// day) must not park a workflow indefinitely. Cap at 5 minutes
    /// and let the next attempt re-classify.
    #[test]
    fn classify_backoff_caps_retry_after_at_five_minutes() {
        assert_eq!(
            classify_backoff("Retry-After: 86400", 0),
            std::time::Duration::from_secs(300)
        );
    }

    /// Plain rate-limit text without an embedded retry hint still
    /// routes through the burst branch — pin against a regression
    /// where `extract_retry_delay` over-matches a `rate_limit` /
    /// `rate-limit` substring as if it were a `retry-after` prefix.
    #[test]
    fn classify_backoff_no_retry_after_falls_back_to_burst_branch() {
        assert_eq!(
            classify_backoff("Plain rate_limit error, no header info", 0),
            std::time::Duration::from_secs(65)
        );
    }

    #[test]
    fn classify_backoff_generic_exponential_capped() {
        assert_eq!(
            classify_backoff("Resource quota exceeded: something", 0),
            std::time::Duration::from_secs(1)
        );
        assert_eq!(
            classify_backoff("Resource quota exceeded: something", 1),
            std::time::Duration::from_secs(2)
        );
        assert_eq!(
            classify_backoff("Resource quota exceeded: something", 3),
            std::time::Duration::from_secs(8)
        );
        // attempt 10: 2^10 = 1024, capped at 60
        assert_eq!(
            classify_backoff("some other error", 10),
            std::time::Duration::from_secs(60)
        );
    }

    // -------------------------------------------------------------------------
    // R1 Tests: behavioral executor tests (cancel, timeout, backoff math)
    // -------------------------------------------------------------------------

    /// Test 1: cancel mid-step stops execution and leaves state = Cancelled.
    ///
    /// - 3 sequential steps
    /// - Step 1 returns immediately
    /// - Step 2 waits on a Notify (simulating a long-running agent call)
    /// - Step 3 would return immediately
    /// - After step 1 completes (confirmed via a separate Notify), we cancel
    ///   the run. Then we unblock step 2 to return.
    /// - Asserts: executor returns Err("cancelled"), state is Cancelled,
    ///   step_results.len() < 3 (step 3 never fired)
    ///
    /// Uses an atomic call counter to tell apart step 1 (call 0) from
    /// step 2 (call 1) regardless of how the prompt template expands.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_mid_step_stops_execution_and_state_is_cancelled() {
        use std::sync::{
            atomic::{AtomicU32, Ordering},
            Arc,
        };
        use tokio::sync::Notify;

        let engine = Arc::new(WorkflowEngine::new());
        // step2_gate: blocks inside the step-2 send_message until notified.
        let step2_gate = Arc::new(Notify::new());
        // step1_done_signal: fires as soon as step 1 returns, before step 2
        // is even called. Lets the test-driver know it's safe to cancel.
        let step1_done_signal = Arc::new(Notify::new());
        let call_count = Arc::new(AtomicU32::new(0));

        let wf = Workflow {
            id: WorkflowId::new(),
            name: "cancel-mid-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "step1".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "s1".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "step2".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "s2".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
                WorkflowStep {
                    name: "step3".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "s3".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    inherit_context: None,
                    depends_on: vec![],
                    session_mode: None,
                },
            ],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };

        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "input".to_string()).await.unwrap();

        let gate = step2_gate.clone();
        let s1_done = step1_done_signal.clone();
        let counter = call_count.clone();
        let engine_exec = engine.clone();

        // Spawn execute_run so cancel can race it.
        let handle = tokio::spawn(async move {
            engine_exec
                .execute_run(
                    run_id,
                    mock_resolver,
                    move |_id: AgentId, _msg: String, _sm: Option<SessionMode>| {
                        let gate = gate.clone();
                        let s1_done = s1_done.clone();
                        let counter = counter.clone();
                        async move {
                            let call = counter.fetch_add(1, Ordering::SeqCst);
                            if call == 0 {
                                // step 1: return immediately, signal the driver.
                                s1_done.notify_one();
                                Ok(("step1_done".to_string(), 1u64, 1u64))
                            } else {
                                // step 2 (or 3): block until the gate opens.
                                gate.notified().await;
                                Ok(("step_done".to_string(), 1u64, 1u64))
                            }
                        }
                    },
                )
                .await
        });

        // Wait until step 1 has signalled completion.
        step1_done_signal.notified().await;
        // Give the executor a brief moment to record the step result and
        // reach the step-2 send_message call before we cancel.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Cancel while executor is parked inside step-2's send_message.
        engine
            .cancel_run(run_id)
            .await
            .expect("cancel must succeed");

        // Unblock step 2's send_message so the executor can observe the
        // cancellation at the next step boundary.
        step2_gate.notify_waiters();

        // Await the spawned task.
        let result = handle.await.expect("task must not panic");
        assert!(
            result.is_err(),
            "execute_run must return Err after cancel, got Ok"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("cancel"),
            "error must mention 'cancel', got: {err_msg}"
        );

        // State must be Cancelled.
        let run = engine.get_run(run_id).await.unwrap();
        assert!(
            matches!(run.state, WorkflowRunState::Cancelled),
            "state must be Cancelled, got {:?}",
            run.state
        );
        // Step 3 must never have fired.
        assert!(
            run.step_results.len() < 3,
            "step 3 must not have executed, got {} step_results",
            run.step_results.len()
        );
    }

    /// Test 2: total_timeout fires and transitions the run to Failed.
    ///
    /// Uses `tokio::time::pause()` + `advance()` so the test completes
    /// instantly without sleeping real time.
    #[tokio::test(flavor = "current_thread")]
    async fn total_timeout_fires_and_sets_state_to_failed() {
        tokio::time::pause();

        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "timeout-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "slow-step".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 300, // per-step timeout: longer than workflow timeout
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            }],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: Some(1), // 1 second total timeout
        };

        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "input".to_string()).await.unwrap();

        // execute_run will tokio::time::timeout(1s, inner_fut). The sender
        // sleeps for 3s. We advance time by 2s to fire the timeout.
        let exec_fut = engine.execute_run(
            run_id,
            mock_resolver,
            |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                Ok(("done".to_string(), 0u64, 0u64))
            },
        );

        // Drive execute_run and advance time concurrently. With time paused
        // the sleep inside the sender won't advance unless we explicitly
        // advance. We need to poll the future a bit first to get it parked
        // in the sleep, then advance.
        let result = tokio::select! {
            r = exec_fut => r,
            _ = async {
                // Let the executor get parked in the sleep first.
                tokio::task::yield_now().await;
                tokio::time::advance(std::time::Duration::from_secs(2)).await;
                std::future::pending::<()>().await
            } => unreachable!(),
        };

        assert!(
            result.is_err(),
            "execute_run must return Err on timeout, got Ok"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("workflow exceeded total_timeout"),
            "error must mention timeout, got: {err_msg}"
        );

        let run = engine.get_run(run_id).await.unwrap();
        assert!(
            matches!(run.state, WorkflowRunState::Failed),
            "state must be Failed after timeout, got {:?}",
            run.state
        );
        let error_field = run.error.as_deref().unwrap_or("");
        assert!(
            error_field.contains("workflow exceeded total_timeout"),
            "run.error must contain timeout message, got: {error_field}"
        );
    }

    /// Test 3: compute_retry_backoff math — pure unit test.
    #[test]
    fn compute_retry_backoff_math() {
        use std::time::Duration;

        // Fixed base, no jitter: exponential doubling.
        assert_eq!(
            compute_retry_backoff("err", 0, Some(100), None),
            Duration::from_millis(100),
            "attempt 0: base_ms * 2^0"
        );
        assert_eq!(
            compute_retry_backoff("err", 1, Some(100), None),
            Duration::from_millis(200),
            "attempt 1: base_ms * 2^1"
        );
        assert_eq!(
            compute_retry_backoff("err", 2, Some(100), None),
            Duration::from_millis(400),
            "attempt 2: base_ms * 2^2"
        );
        // Very high attempt — must cap at MAX_BACKOFF_MS = 60_000 ms.
        assert_eq!(
            compute_retry_backoff("err", 20, Some(100), None),
            Duration::from_millis(60_000),
            "attempt 20: must be capped at 60_000 ms"
        );

        // With jitter_pct = 25 at attempt 1 (raw = 200ms):
        // delta = 200 * 25 / 100 = 50ms, range = 201
        // result in [200 - 50, 200 + 50] = [150ms, 250ms]
        let mut values = std::collections::HashSet::new();
        for _ in 0..20 {
            let d = compute_retry_backoff("err", 1, Some(100), Some(25));
            let ms = d.as_millis();
            assert!(
                (150..=250).contains(&ms),
                "jitter result {ms}ms must be in [150, 250]"
            );
            values.insert(ms);
        }
        assert!(
            values.len() >= 2,
            "jitter must produce variation over 20 calls, got unique values: {values:?}"
        );

        // No backoff_ms → falls through to classify_backoff.
        assert_eq!(
            compute_retry_backoff("err", 0, None, None),
            classify_backoff("err", 0),
            "None backoff_ms must delegate to classify_backoff"
        );
    }

    /// Test 4: cancel_run returns NotFound for an unknown run id.
    #[tokio::test]
    async fn cancel_run_returns_not_found_for_unknown_id() {
        let engine = WorkflowEngine::new();
        let unknown = WorkflowRunId(uuid::Uuid::new_v4());
        let result = engine.cancel_run(unknown).await;
        assert!(
            matches!(result, Err(CancelRunError::NotFound(_))),
            "expected NotFound, got: {:?}",
            result
        );
    }

    /// Test 5: cancel_during_retry_sleep_aborts_promptly.
    ///
    /// Step fails every time; backoff is 30s. We cancel during the sleep
    /// and verify the executor returns well before the sleep would have
    /// expired.
    #[tokio::test(flavor = "current_thread")]
    async fn cancel_during_retry_sleep_aborts_promptly() {
        use std::sync::Arc;
        tokio::time::pause();

        let engine = Arc::new(WorkflowEngine::new());
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "retry-cancel-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "always-fail".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 5,
                error_mode: ErrorMode::Retry {
                    max_retries: 5,
                    backoff_ms: Some(30_000), // 30 second backoff
                    jitter_pct: None,
                },
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            }],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };

        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "input".to_string()).await.unwrap();

        let engine_exec = engine.clone();
        // Spawn execute_run. The step always fails, so it enters the retry
        // sleep (30s) after the first attempt.
        let handle = tokio::spawn(async move {
            engine_exec
                .execute_run(
                    run_id,
                    mock_resolver,
                    |_id: AgentId, _msg: String, _sm: Option<SessionMode>| async {
                        Err("forced failure".to_string())
                    },
                )
                .await
        });

        // Advance time by 100ms — enough for the first attempt to fail and
        // the executor to enter the retry sleep, but nowhere near the 30s backoff.
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(100)).await;
        tokio::task::yield_now().await;

        // Cancel the run — this notifies the retry sleep's select branch.
        engine
            .cancel_run(run_id)
            .await
            .expect("cancel must succeed");

        // The handle should resolve promptly (well under 500ms of real time).
        // We use a real-time timeout here to guard against the test hanging.
        tokio::time::resume();
        let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("execute_run must return promptly after cancel, not wait 30s")
            .expect("task must not panic");

        assert!(
            result.is_err(),
            "execute_run must return Err after cancel, got Ok"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("cancel"),
            "error must mention cancel, got: {err}"
        );

        let run = engine.get_run(run_id).await.unwrap();
        assert!(
            matches!(run.state, WorkflowRunState::Cancelled),
            "state must be Cancelled, got {:?}",
            run.state
        );
    }

    /// Regression: Cancelled runs must be evictable when the total exceeds
    /// the retention cap (`MAX_RETAINED_RUNS`). Without this, a burst of
    /// cancels would pin those records in the DashMap forever and push out
    /// evictable Completed/Failed runs.
    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_runs_are_evictable_when_over_cap() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "evict-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "s".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "x".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 1,
                error_mode: ErrorMode::Fail,
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
                session_mode: None,
            }],
            created_at: Utc::now(),
            layout: None,
            total_timeout_secs: None,
        };
        let wf_id = engine.register(wf).await;

        // Create + immediately cancel well past the retention cap (200).
        // Without Cancelled in the eviction filter, this loop grows the
        // `runs` map unboundedly.
        for _ in 0..250usize {
            let run_id = engine
                .create_run(wf_id, "x".to_string())
                .await
                .expect("create_run");
            engine.cancel_run(run_id).await.expect("cancel_run");
        }

        let all_runs = engine.list_runs(None).await;
        assert!(
            all_runs.len() <= 200,
            "cancelled runs over the cap must be evictable, retained {} (cap 200)",
            all_runs.len()
        );
    }

    // ----------------------------------------------------------------------
    // StepAgent deserialization — bare-string ergonomics regression tests
    //
    // The on-wire shape documented in the workflow issue / PR body is the
    // bare `agent = "researcher"` form; if the custom `Deserialize` impl
    // ever regresses to the derived untagged enum, the operator will hit
    // an opaque untagged-enum error on first run. These tests guard the
    // contract at the type boundary.
    // ----------------------------------------------------------------------

    #[test]
    fn step_agent_deserializes_bare_string_as_by_name() {
        let v: StepAgent = serde_json::from_str("\"researcher\"").expect("bare string");
        match v {
            StepAgent::ByName { name } => assert_eq!(name, "researcher"),
            other => panic!("expected ByName, got {other:?}"),
        }
    }

    #[test]
    fn step_agent_deserializes_object_by_name() {
        let v: StepAgent =
            serde_json::from_str(r#"{"name":"researcher"}"#).expect("object form by name");
        match v {
            StepAgent::ByName { name } => assert_eq!(name, "researcher"),
            other => panic!("expected ByName, got {other:?}"),
        }
    }

    #[test]
    fn step_agent_deserializes_object_by_id() {
        let v: StepAgent =
            serde_json::from_str(r#"{"id":"agent-uuid-123"}"#).expect("object by id");
        match v {
            StepAgent::ById { id } => assert_eq!(id, "agent-uuid-123"),
            other => panic!("expected ById, got {other:?}"),
        }
    }

    #[test]
    fn step_agent_rejects_garbage_input() {
        // Number: not a string or object.
        assert!(serde_json::from_str::<StepAgent>("42").is_err());
        // Array: not a string or object.
        assert!(serde_json::from_str::<StepAgent>("[\"researcher\"]").is_err());
        // Both fields set: ambiguous.
        assert!(
            serde_json::from_str::<StepAgent>(r#"{"id":"a","name":"b"}"#).is_err(),
            "must reject both id and name set"
        );
        // Neither field set: empty object.
        assert!(
            serde_json::from_str::<StepAgent>("{}").is_err(),
            "must reject empty object"
        );
        // Null is also invalid.
        assert!(serde_json::from_str::<StepAgent>("null").is_err());
    }

    #[test]
    fn step_agent_round_trip_bare_string_through_toml() {
        // TOML doesn't allow top-level scalar values, so wrap in a struct
        // that holds a `StepAgent` field. The bare-string form is the form
        // operators are expected to use in TOML workflow files.
        #[derive(serde::Deserialize)]
        struct Wrap {
            agent: StepAgent,
        }
        let parsed: Wrap = toml::from_str(r#"agent = "researcher""#).expect("toml bare string");
        match parsed.agent {
            StepAgent::ByName { name } => assert_eq!(name, "researcher"),
            other => panic!("expected ByName, got {other:?}"),
        }
    }

    #[test]
    fn step_agent_round_trip_object_through_toml() {
        #[derive(serde::Deserialize)]
        struct Wrap {
            agent: StepAgent,
        }
        let by_name: Wrap =
            toml::from_str("agent = { name = \"researcher\" }").expect("toml object by name");
        match by_name.agent {
            StepAgent::ByName { name } => assert_eq!(name, "researcher"),
            other => panic!("expected ByName, got {other:?}"),
        }
        let by_id: Wrap =
            toml::from_str("agent = { id = \"agent-uuid-123\" }").expect("toml object by id");
        match by_id.agent {
            StepAgent::ById { id } => assert_eq!(id, "agent-uuid-123"),
            other => panic!("expected ById, got {other:?}"),
        }
    }
}
