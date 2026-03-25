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
use librefang_types::agent::AgentId;
use librefang_types::subagent::SubagentContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    Completed,
    Failed,
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
    /// Output from this step.
    pub output: String,
    /// Token usage.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// The workflow engine — manages definitions and executes pipeline runs.
#[derive(Clone)]
pub struct WorkflowEngine {
    /// Registered workflow definitions.
    workflows: Arc<RwLock<HashMap<WorkflowId, Workflow>>>,
    /// Active and completed workflow runs.
    runs: Arc<RwLock<HashMap<WorkflowRunId, WorkflowRun>>>,
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

    // OR: split on `||` first (lower precedence)
    if condition.contains("||") {
        return condition
            .split("||")
            .any(|branch| evaluate_condition(input, branch.trim()));
    }

    // AND: split on `&&`
    if condition.contains("&&") {
        return condition
            .split("&&")
            .all(|part| evaluate_condition(input, part.trim()));
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

impl WorkflowEngine {
    /// Create a new workflow engine (no persistence).
    pub fn new() -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(RwLock::new(HashMap::new())),
            persist_path: None,
        }
    }

    /// Create a new workflow engine with run persistence.
    ///
    /// Completed and failed runs are persisted to `<home_dir>/workflow_runs.json`.
    pub fn new_with_persistence(home_dir: &Path) -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(RwLock::new(HashMap::new())),
            persist_path: Some(home_dir.join("workflow_runs.json")),
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
        let runs: Vec<WorkflowRun> = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse workflow runs: {e}"))?;
        let count = runs.len();
        let mut map = self.runs.blocking_write();
        for run in runs {
            map.insert(run.id, run);
        }
        debug!(count, "Loaded persisted workflow runs from disk");
        Ok(count)
    }

    /// Persist completed/failed runs to disk via atomic write.
    fn persist_runs(&self) {
        let path = match &self.persist_path {
            Some(p) => p,
            None => return,
        };
        // Acquire a blocking read — called from async context after state update.
        let runs = self.runs.blocking_read();
        let terminal: Vec<&WorkflowRun> = runs
            .values()
            .filter(|r| {
                matches!(
                    r.state,
                    WorkflowRunState::Completed | WorkflowRunState::Failed
                )
            })
            .collect();
        let data = match serde_json::to_string_pretty(&terminal) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to serialize workflow runs: {e}");
                return;
            }
        };
        drop(runs);
        let tmp_path = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp_path, data.as_bytes()) {
            warn!("Failed to write workflow runs temp file: {e}");
            return;
        }
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            warn!("Failed to rename workflow runs file: {e}");
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
        };

        let mut runs = self.runs.write().await;
        runs.insert(run_id, run);

        // Evict oldest completed/failed runs when we exceed the cap
        if runs.len() > Self::MAX_RETAINED_RUNS {
            let mut evictable: Vec<(WorkflowRunId, DateTime<Utc>)> = runs
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.state,
                        WorkflowRunState::Completed | WorkflowRunState::Failed
                    )
                })
                .map(|(id, r)| (*id, r.started_at))
                .collect();

            // Sort oldest first
            evictable.sort_by_key(|(_, t)| *t);

            let to_remove = runs.len() - Self::MAX_RETAINED_RUNS;
            for (id, _) in evictable.into_iter().take(to_remove) {
                runs.remove(&id);
                debug!(run_id = %id, "Evicted old workflow run");
            }
        }

        Some(run_id)
    }

    /// Get the current state of a workflow run.
    pub async fn get_run(&self, run_id: WorkflowRunId) -> Option<WorkflowRun> {
        self.runs.read().await.get(&run_id).cloned()
    }

    /// List all workflow runs (optionally filtered by state).
    pub async fn list_runs(&self, state_filter: Option<&str>) -> Vec<WorkflowRun> {
        self.runs
            .read()
            .await
            .values()
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
            .cloned()
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
        // Get the run and workflow
        let (workflow, input) = {
            let mut runs = self.runs.write().await;
            let run = runs.get_mut(&run_id).ok_or("Workflow run not found")?;
            run.state = WorkflowRunState::Running;

            let workflow = self
                .workflows
                .read()
                .await
                .get(&run.workflow_id)
                .ok_or("Workflow definition not found")?
                .clone();

            (workflow, run.input.clone())
        };

        info!(
            run_id = %run_id,
            workflow = %workflow.name,
            steps = workflow.steps.len(),
            "Starting workflow execution"
        );

        // Check if any step has non-empty depends_on — if so, use DAG execution
        let has_dag_deps = workflow.steps.iter().any(|s| !s.depends_on.is_empty());
        if has_dag_deps {
            let result = self
                .execute_run_dag(run_id, &workflow, &input, &agent_resolver, &send_message)
                .await;
            self.persist_runs_async().await;
            return result;
        }

        let result = self
            .execute_run_sequential(run_id, &workflow, &input, &agent_resolver, &send_message)
            .await;
        self.persist_runs_async().await;
        result
    }

    /// Sequential workflow execution (extracted for persistence wrapping).
    async fn execute_run_sequential<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        workflow: &Workflow,
        input: &str,
        agent_resolver: &(impl Fn(&StepAgent) -> Option<(AgentId, String, bool)>),
        send_message: &F,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String) -> Fut + Sync,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>> + Send,
    {
        let mut current_input = input.to_string();
        let mut all_outputs: Vec<String> = Vec::new();
        let mut variables: HashMap<String, String> = HashMap::new();
        let mut i = 0;

        while i < workflow.steps.len() {
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
                        .read()
                        .await
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
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms,
                            };
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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

                    // Snapshot step results once for all fan-out steps
                    let prev_results: Vec<StepResult> = self
                        .runs
                        .read()
                        .await
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
                                    output: output.clone(),
                                    input_tokens,
                                    output_tokens,
                                    duration_ms,
                                };
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                    let step_results: Vec<StepResult> = self
                        .runs
                        .read()
                        .await
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
                        .read()
                        .await
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
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms,
                            };
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                            .read()
                            .await
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
                                    output: output.clone(),
                                    input_tokens,
                                    output_tokens,
                                    duration_ms,
                                };
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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

        // Mark workflow as completed
        let final_output = current_input.clone();
        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
            r.state = WorkflowRunState::Completed;
            r.output = Some(final_output.clone());
            r.completed_at = Some(Utc::now());
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
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                            output: output.clone(),
                            input_tokens,
                            output_tokens,
                            duration_ms,
                        };
                        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms: step_duration_ms,
                            };
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
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
        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
            r.state = WorkflowRunState::Completed;
            r.output = Some(last_output.clone());
            r.completed_at = Some(Utc::now());
        }

        info!(run_id = %run_id, "Workflow DAG execution completed successfully");
        Ok(last_output)
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
    /// **Must not be called from async context** — uses blocking I/O.
    pub fn load_templates_from_dir(&self, dir: &std::path::Path) -> usize {
        use tracing::{info, warn};

        if !dir.is_dir() {
            return 0;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                warn!("Cannot read template directory {}: {e}", dir.display());
                return 0;
            }
        };

        let mut map = self.templates.blocking_write();
        let mut count = 0;

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
                    warn!("Cannot read {}: {e}", path.display());
                    continue;
                }
            };
            match toml::from_str::<WorkflowTemplate>(&content) {
                Ok(tpl) => {
                    info!(id = %tpl.id, name = %tpl.name, "Loaded workflow template");
                    map.insert(tpl.id.clone(), tpl);
                    count += 1;
                }
                Err(e) => {
                    warn!("Failed to parse template {}: {e}", path.display());
                }
            }
        }
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
                    depends_on: vec![],
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
        // Collect step joins all outputs
        assert!(output.contains("Done: Task A"));
        assert!(output.contains("Done: Task B"));
        assert!(output.contains("---"));
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
                output: "done".to_string(),
                input_tokens: 10,
                output_tokens: 20,
                duration_ms: 100,
            }],
            output: Some("final output".to_string()),
            error: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
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
            {
                let mut runs = engine.runs.blocking_write();
                runs.insert(run_completed.id, run_completed);
                runs.insert(run_failed.id, run_failed);
            }
            engine.persist_runs();
        }

        // Load into a fresh engine and verify.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            let count = engine.load_runs().unwrap();
            assert_eq!(count, 2);

            let runs = engine.runs.blocking_read();
            let c = runs.get(&completed_id).expect("completed run missing");
            assert!(matches!(c.state, WorkflowRunState::Completed));
            assert_eq!(c.workflow_name, "persist-test");
            assert_eq!(c.output.as_deref(), Some("final output"));
            assert_eq!(c.step_results.len(), 1);
            assert_eq!(c.step_results[0].step_name, "step-1");

            let f = runs.get(&failed_id).expect("failed run missing");
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
        };
        let running_id = running.id;

        // Persist — should only write the completed run, not the running one.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            {
                let mut runs = engine.runs.blocking_write();
                runs.insert(completed.id, completed);
                runs.insert(running.id, running);
            }
            engine.persist_runs();
        }

        // Load and verify only 1 run came back.
        {
            let engine = WorkflowEngine::new_with_persistence(tmp.path());
            let count = engine.load_runs().unwrap();
            assert_eq!(count, 1);

            let runs = engine.runs.blocking_read();
            assert!(runs.contains_key(&completed_id));
            assert!(!runs.contains_key(&running_id));
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
}
