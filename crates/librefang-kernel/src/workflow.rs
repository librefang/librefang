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
use librefang_types::agent::AgentId;
use librefang_types::subagent::SubagentContext;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

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
}

fn default_timeout() -> u64 {
    120
}

/// How to identify the agent for a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepAgent {
    /// Reference an agent by UUID.
    ById { id: String },
    /// Reference an agent by name (first match).
    ByName { name: String },
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
    Retry { max_retries: u32 },
}

/// The current state of a workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunState {
    Pending,
    Running,
    /// The run was paused mid-execution and is waiting for an external
    /// signal (an approval, a human-supplied input, etc.) before it
    /// continues. Carry the `resume_token` the caller must present to
    /// `WorkflowEngine::resume_run`, the wall-clock pause time, and a
    /// human-readable reason for log/UI surfaces.
    ///
    /// Per-run snapshot data (step index, variable bindings, current
    /// input) lives in fields on `WorkflowRun` rather than this variant
    /// because runs are also used as raw step-history records — the
    /// snapshot needs to be readable without matching on the state. See
    /// #3335.
    Paused {
        resume_token: Uuid,
        reason: String,
        /// Wall-clock pause time. Surfaced in logs / UI today; future
        /// follow-up will use this to drive a TTL-based GC sweep that
        /// auto-expires Paused runs older than a configurable threshold
        /// (#3335 GC follow-up).
        paused_at: DateTime<Utc>,
    },
    Completed,
    Failed,
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
/// `resume_token` so the caller can begin waiting for the corresponding
/// approval / input artifact before the execution loop has actually
/// transitioned the run to `WorkflowRunState::Paused`. The execution loop
/// reuses this same token when it honors the request, guaranteeing the
/// caller's resume call will match. See #3335.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PauseRequest {
    /// Human-readable explanation surfaced in logs and UI.
    ///
    /// **SECURITY:** persisted plaintext to `workflow_runs.json` and
    /// shown back to operators. Do not pass secrets, PII, or
    /// approval-gating tokens here — use a side channel referenced by
    /// id instead.
    pub reason: String,
    /// Token the caller must present to [`WorkflowEngine::resume_run`].
    ///
    /// **SECURITY:** today this is stored plaintext in `workflow_runs.json`
    /// alongside the run. Anyone with read access to the daemon home
    /// directory can recover live tokens and resume an arbitrary paused
    /// workflow. Acceptable for the kernel-internal foundation in this
    /// PR; the future REST handler must hash tokens at rest before that
    /// surface ships. Tracked as a follow-up against #3335.
    pub resume_token: Uuid,
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
    persist_path: Option<PathBuf>,
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

impl WorkflowEngine {
    /// Create a new workflow engine (no persistence).
    pub fn new() -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(DashMap::new()),
            persist_path: None,
        }
    }

    /// Create a new workflow engine with run persistence.
    ///
    /// Completed and failed runs are persisted to `<home_dir>/data/workflow_runs.json`.
    pub fn new_with_persistence(home_dir: &Path) -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(DashMap::new()),
            persist_path: Some(home_dir.join("data").join("workflow_runs.json")),
        }
    }

    // -- Run Persistence ------------------------------------------------------

    /// Load persisted runs from disk into memory.
    ///
    /// Returns the number of runs loaded. If the file does not exist,
    /// returns `Ok(0)` without error.
    pub fn load_runs(&self) -> Result<usize, String> {
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

    /// Persist completed/failed runs to disk via atomic write.
    fn persist_runs(&self) {
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
        let tmp_path = path.with_extension(format!("json.tmp.{}", std::process::id()));
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
    async fn persist_runs_async(&self) {
        if self.persist_path.is_none() {
            return;
        }
        let engine = self.clone();
        let _ = tokio::task::spawn_blocking(move || engine.persist_runs()).await;
    }

    /// Register a new workflow definition.
    pub async fn register(&self, workflow: Workflow) -> WorkflowId {
        let id = workflow.id;
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

    /// Remove a workflow definition.
    pub async fn remove_workflow(&self, id: WorkflowId) -> bool {
        self.workflows.write().await.remove(&id).is_some()
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

        self.runs.insert(run_id, run);

        // Evict oldest completed/failed runs when we exceed the cap
        if self.runs.len() > Self::MAX_RETAINED_RUNS {
            let mut evictable: Vec<(WorkflowRunId, DateTime<Utc>)> = self
                .runs
                .iter()
                .filter(|r| {
                    matches!(
                        r.state,
                        WorkflowRunState::Completed | WorkflowRunState::Failed
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
            recovered += 1;
        }
        recovered
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
    ) -> Result<Option<(String, u64, u64)>, String>
    where
        F: Fn(AgentId, String) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let timeout_dur = std::time::Duration::from_secs(step.timeout_secs);

        match &step.error_mode {
            ErrorMode::Fail => {
                let result = tokio::time::timeout(timeout_dur, send_message(agent_id, prompt))
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
                match tokio::time::timeout(timeout_dur, send_message(agent_id, prompt)).await {
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
            ErrorMode::Retry { max_retries } => {
                let mut last_err = String::new();
                for attempt in 0..=*max_retries {
                    match tokio::time::timeout(timeout_dur, send_message(agent_id, prompt.clone()))
                        .await
                    {
                        Ok(Ok(result)) => return Ok(Some(result)),
                        Ok(Err(e)) => {
                            last_err = e.to_string();
                            if attempt < *max_retries {
                                warn!(
                                    "Step '{}' attempt {} failed: {e}, retrying",
                                    step.name,
                                    attempt + 1
                                );
                            }
                        }
                        Err(_) => {
                            last_err = format!("timed out after {}s", step.timeout_secs);
                            if attempt < *max_retries {
                                warn!(
                                    "Step '{}' attempt {} timed out, retrying",
                                    step.name,
                                    attempt + 1
                                );
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
    /// Errors:
    /// - `Err` if the run is unknown.
    /// - `Err` if the run has already finished (`Completed` / `Failed`).
    pub async fn pause_run(
        &self,
        run_id: WorkflowRunId,
        reason: impl Into<String>,
    ) -> Result<Uuid, String> {
        let mut run = self
            .runs
            .get_mut(&run_id)
            .ok_or_else(|| format!("Workflow run not found: {run_id}"))?;
        // We clone values out of the borrowed state before mutating, so we
        // need to inspect the state in a block that ends before the write.
        let existing_token = match &run.state {
            WorkflowRunState::Pending | WorkflowRunState::Running => {
                run.pause_request.as_ref().map(|r| r.resume_token)
            }
            WorkflowRunState::Paused { resume_token, .. } => return Ok(*resume_token),
            WorkflowRunState::Completed | WorkflowRunState::Failed => {
                return Err(format!(
                    "Cannot pause workflow run {run_id}: state is terminal"
                ))
            }
        };
        if let Some(token) = existing_token {
            return Ok(token);
        }
        let token = Uuid::new_v4();
        run.pause_request = Some(PauseRequest {
            reason: reason.into(),
            resume_token: token,
        });
        Ok(token)
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
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String) -> Fut + Sync,
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
                    .ok_or_else(|| format!("Workflow run not found: {run_id}"))?;
                // Validate token inside a scope so the immutable borrow of
                // `run.state` ends before we mutate it below.
                {
                    match &run.state {
                        WorkflowRunState::Paused {
                            resume_token: stored,
                            ..
                        } => {
                            if *stored != resume_token {
                                return Err(format!(
                                    "Resume token mismatch for run {run_id}: presented token does not match stored token"
                                ));
                            }
                        }
                        other => {
                            return Err(format!(
                                "Cannot resume workflow run {run_id}: state is {other:?}, expected Paused"
                            ));
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
                .ok_or_else(|| format!("Workflow definition {workflow_id} not found"))?
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
            Err(
                "Resuming a workflow with DAG dependencies is not yet supported (#3335 follow-up)"
                    .to_string(),
            )
        } else {
            // `input` here is unused on the resume path because the loop
            // pulls `paused_current_input` off the run when present.
            self.execute_run_sequential(run_id, &workflow, "", &agent_resolver, &send_message)
                .await
        };
        self.cleanup_terminal_pause_state(run_id).await;
        self.persist_runs_async().await;
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
        F: Fn(AgentId, String) -> Fut + Sync,
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

        // Check if any step has non-empty depends_on — if so, use DAG execution
        let has_dag_deps = workflow.steps.iter().any(|s| !s.depends_on.is_empty());
        let result = if has_dag_deps {
            self.execute_run_dag(run_id, &workflow, &input, &agent_resolver, &send_message)
                .await
        } else {
            self.execute_run_sequential(run_id, &workflow, &input, &agent_resolver, &send_message)
                .await
        };
        self.cleanup_terminal_pause_state(run_id).await;
        self.persist_runs_async().await;
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
                WorkflowRunState::Completed | WorkflowRunState::Failed
            ) {
                run.clear_pause_state();
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
        F: Fn(AgentId, String) -> Fut + Sync,
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
            let pending_pause = self.runs.get(&run_id).and_then(|r| r.pause_request.clone());
            if let Some(pause) = pending_pause {
                if let Some(mut run) = self.runs.get_mut(&run_id) {
                    run.paused_step_index = Some(i);
                    run.paused_variables = variables
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    run.paused_current_input = Some(current_input.clone());
                    run.pause_request = None;
                    run.state = WorkflowRunState::Paused {
                        resume_token: pause.resume_token,
                        reason: pause.reason.clone(),
                        paused_at: Utc::now(),
                    };
                }
                info!(
                    run_id = %run_id,
                    resume_step = i,
                    reason = %pause.reason,
                    "Workflow run paused at step boundary"
                );
                return Ok(current_input);
            }

            let step = &workflow.steps[i];

            debug!(
                step = i + 1,
                name = %step.name,
                "Executing workflow step"
            );

            match &step.mode {
                StepMode::Sequential => {
                    let (agent_id, agent_name, agent_inherit) = agent_resolver(&step.agent)
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

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
                    let result =
                        Self::execute_step_with_error_mode(step, agent_id, prompt, &send_message)
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
                                r.state = WorkflowRunState::Failed;
                                r.error = Some(e.clone());
                                r.completed_at = Some(Utc::now());
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
                        let (agent_id, agent_name, agent_inherit) = agent_resolver(&fan_step.agent)
                            .ok_or_else(|| {
                                format!("Agent not found for step '{}'", fan_step.name)
                            })?;
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
                            send_message(agent_id, prompt),
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
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(error_msg.clone());
                                    r.completed_at = Some(Utc::now());
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
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(error_msg.clone());
                                    r.completed_at = Some(Utc::now());
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
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

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
                    let result =
                        Self::execute_step_with_error_mode(step, agent_id, prompt, &send_message)
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
                                r.state = WorkflowRunState::Failed;
                                r.error = Some(e.clone());
                                r.completed_at = Some(Utc::now());
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
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

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
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(e.clone());
                                    r.completed_at = Some(Utc::now());
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
        F: Fn(AgentId, String) -> Fut + Sync,
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
                    .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

                let prompt = Self::expand_variables(&step.prompt_template, input, &variables);
                let prompt_sent = prompt.clone();
                let start = std::time::Instant::now();
                let result =
                    Self::execute_step_with_error_mode(step, agent_id, prompt, send_message).await;
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
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

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

                        futures.push(Box::pin(async move {
                            let step_start = std::time::Instant::now();
                            let result =
                                tokio::time::timeout(timeout_dur, send_message(agent_id, prompt))
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
        })
    }
}

impl Default for WorkflowTemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
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

        let sender = |_id: AgentId, msg: String| async move {
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "all good".to_string())
            .await
            .unwrap();

        let sender =
            |_id: AgentId, msg: String| async move { Ok((format!("OK: {msg}"), 10u64, 5u64)) };

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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // This sender returns output containing "ERROR"
        let sender = |_id: AgentId, _msg: String| async move {
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
            }],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "draft".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String| {
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
            }],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let sender = |_id: AgentId, _msg: String| async move {
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String| {
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
                error_mode: ErrorMode::Retry { max_retries: 2 },
                output_var: None,
                inherit_context: None,
                depends_on: vec![],
            }],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String| {
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "start".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, msg: String| {
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let sender =
            |_id: AgentId, msg: String| async move { Ok((format!("Done: {msg}"), 10u64, 5u64)) };

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

        let retry_json = serde_json::to_string(&ErrorMode::Retry { max_retries: 3 }).unwrap();
        let retry: ErrorMode = serde_json::from_str(&retry_json).unwrap();
        assert!(matches!(retry, ErrorMode::Retry { max_retries: 3 }));
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
        let sender = move |_id: AgentId, msg: String| {
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
        let sender = move |_id: AgentId, msg: String| {
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "raw data".to_string())
            .await
            .unwrap();

        let received_prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rp = received_prompts.clone();
        let sender = move |_id: AgentId, msg: String| {
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let received_prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rp = received_prompts.clone();
        let sender = move |_id: AgentId, msg: String| {
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };

        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Sender always fails
        let sender = |_id: AgentId, _msg: String| async move {
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

        let sender = move |_id: AgentId, msg: String| {
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
        let sender_resume = |_id: AgentId, msg: String| async move {
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
        let sender = |_id: AgentId, msg: String| async move {
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
        let sender2 = |_id: AgentId, msg: String| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        let err = engine
            .resume_run(run_id, bogus_token, mock_resolver, sender2)
            .await
            .expect_err("resume_run with wrong token must error");
        assert!(err.contains("token"), "error should mention token: {err}");

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
        let sender = |_id: AgentId, msg: String| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        let err = engine
            .resume_run(run_id, Uuid::new_v4(), mock_resolver, sender)
            .await
            .expect_err("resume_run on non-paused run must error");
        assert!(
            err.contains("expected Paused"),
            "error should mention required state: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paused_run_round_trips_through_persist_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let original_token: Uuid;
        let original_run_id: WorkflowRunId;

        // Phase 1: build a paused run on engine instance #1, then persist.
        {
            let engine = Arc::new(WorkflowEngine::new_with_persistence(tmp.path()));
            let wf_id = engine.register(test_workflow()).await;
            let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();
            original_run_id = run_id;
            original_token = engine
                .pause_run(run_id, "before-start pause")
                .await
                .unwrap();
            let sender = |_id: AgentId, msg: String| async move {
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
        // Paused-state run came back with its token + snapshot intact.
        // `load_runs` uses `blocking_write` internally, so wrap in
        // `block_in_place` (requires multi-thread runtime, which this
        // test selects via the `flavor` attribute).
        let engine = WorkflowEngine::new_with_persistence(tmp.path());
        let count = tokio::task::block_in_place(|| engine.load_runs()).unwrap();
        assert_eq!(count, 1);
        let run = engine.get_run(original_run_id).await.unwrap();
        match &run.state {
            WorkflowRunState::Paused { resume_token, .. } => {
                assert_eq!(
                    *resume_token, original_token,
                    "persisted resume_token must match across daemon restart"
                );
            }
            other => panic!("expected Paused after reload, got {:?}", other),
        }
        assert_eq!(run.paused_step_index, Some(0));
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
                },
            ],
            created_at: Utc::now(),
            layout: None,
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Lodge a pause request *before* execute_run — DAG executor must
        // refuse cleanly rather than silently dropping the request.
        let _ = engine.pause_run(run_id, "dag pause").await.unwrap();
        let sender = |_id: AgentId, msg: String| async move {
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
    async fn pause_run_is_idempotent_returns_same_token() {
        let engine = WorkflowEngine::new();
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let token1 = engine.pause_run(run_id, "first").await.unwrap();
        let token2 = engine.pause_run(run_id, "second").await.unwrap();
        let token3 = engine.pause_run(run_id, "third").await.unwrap();
        assert_eq!(token1, token2);
        assert_eq!(token2, token3);

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
        let sender = |_id: AgentId, msg: String| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .unwrap();
        let sender2 = |_id: AgentId, msg: String| async move {
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
        let sender3 = |_id: AgentId, msg: String| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        let err = engine
            .resume_run(run_id, token, mock_resolver, sender3)
            .await
            .expect_err("double-resume on a completed run must error");
        assert!(
            err.contains("expected Paused"),
            "error should explain state mismatch: {err}"
        );
    }

    #[tokio::test]
    async fn pause_then_execute_on_pending_pauses_at_step_zero() {
        let engine = WorkflowEngine::new();
        let wf_id = engine.register(test_workflow()).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // Run is Pending — pause is lodged before any step has executed.
        let token = engine.pause_run(run_id, "pre-start").await.unwrap();
        let sender = |_id: AgentId, msg: String| async move {
            Ok((format!("Processed: {msg}"), 1_u64, 1_u64))
        };
        engine
            .execute_run(run_id, mock_resolver, sender)
            .await
            .expect("pause-at-zero path must not error");

        let run = engine.get_run(run_id).await.unwrap();
        match &run.state {
            WorkflowRunState::Paused { resume_token, .. } => {
                assert_eq!(*resume_token, token);
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
}
