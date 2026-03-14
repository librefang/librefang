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
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
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
pub struct WorkflowEngine {
    /// Registered workflow definitions.
    workflows: Arc<RwLock<HashMap<WorkflowId, Workflow>>>,
    /// Active and completed workflow runs.
    runs: Arc<RwLock<HashMap<WorkflowRunId, WorkflowRun>>>,
}

impl WorkflowEngine {
    /// Create a new workflow engine.
    pub fn new() -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(RwLock::new(HashMap::new())),
        }
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

    /// Execute a workflow run step-by-step.
    ///
    /// This method takes a closure that sends messages to agents,
    /// so the workflow engine remains decoupled from the kernel.
    pub async fn execute_run<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: F,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
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

        let mut current_input = input;
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
                    let (agent_id, agent_name) = agent_resolver(&step.agent)
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

                    let prompt =
                        Self::expand_variables(&step.prompt_template, &current_input, &variables);

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

                    for (idx, fan_step) in &fan_out_steps {
                        let (agent_id, agent_name) =
                            agent_resolver(&fan_step.agent).ok_or_else(|| {
                                format!("Agent not found for step '{}'", fan_step.name)
                            })?;
                        let prompt = Self::expand_variables(
                            &fan_step.prompt_template,
                            &current_input,
                            &variables,
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
                    current_input = all_outputs.join("\n\n---\n\n");
                    all_outputs.clear();
                    all_outputs.push(current_input.clone());
                    if let Some(ref var) = step.output_var {
                        variables.insert(var.clone(), current_input.clone());
                    }
                }

                StepMode::Conditional { condition } => {
                    let prev_lower = current_input.to_lowercase();
                    let cond_lower = condition.to_lowercase();

                    if !prev_lower.contains(&cond_lower) {
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
                    let (agent_id, agent_name) = agent_resolver(&step.agent)
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

                    let prompt =
                        Self::expand_variables(&step.prompt_template, &current_input, &variables);

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
                    let (agent_id, agent_name) = agent_resolver(&step.agent)
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

                    let until_lower = until.to_lowercase();

                    for loop_iter in 0..*max_iterations {
                        let prompt = Self::expand_variables(
                            &step.prompt_template,
                            &current_input,
                            &variables,
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
// Workflow templates & marketplace
// ---------------------------------------------------------------------------

/// Unique identifier for a workflow template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowTemplateId(pub Uuid);

impl WorkflowTemplateId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowTemplateId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowTemplateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A step definition inside a workflow template (agent referenced by name placeholder).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTemplateStep {
    /// Step name.
    pub name: String,
    /// Suggested agent name — the user can remap at instantiation time.
    pub agent_name: String,
    /// Prompt template (uses `{{input}}` / `{{var_name}}`).
    pub prompt_template: String,
    /// Execution mode.
    #[serde(default)]
    pub mode: StepMode,
    /// Timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Error handling.
    #[serde(default)]
    pub error_mode: ErrorMode,
    /// Optional output variable.
    #[serde(default)]
    pub output_var: Option<String>,
    /// Steps this one depends on (by name).
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// A reusable workflow template with marketplace metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTemplate {
    /// Unique template identifier.
    pub id: WorkflowTemplateId,
    /// Human-readable name.
    pub name: String,
    /// Description of the template.
    pub description: String,
    /// Author / publisher.
    pub author: String,
    /// Semantic version string.
    pub version: String,
    /// Category for marketplace browsing.
    pub category: String,
    /// Searchable tags.
    pub tags: Vec<String>,
    /// The steps that make up this template.
    pub steps: Vec<WorkflowTemplateStep>,
    /// When this template was created / last updated.
    pub created_at: DateTime<Utc>,
}

/// Registry that holds workflow templates (both bundled and user-saved).
pub struct WorkflowTemplateRegistry {
    templates: Arc<RwLock<HashMap<WorkflowTemplateId, WorkflowTemplate>>>,
}

impl WorkflowTemplateRegistry {
    /// Create a new registry pre-loaded with the bundled templates.
    pub fn new() -> Self {
        let mut map = HashMap::new();
        for t in Self::bundled_templates() {
            map.insert(t.id, t);
        }
        Self {
            templates: Arc::new(RwLock::new(map)),
        }
    }

    /// List all templates, optionally filtered by category.
    pub async fn list(&self, category: Option<&str>) -> Vec<WorkflowTemplate> {
        self.templates
            .read()
            .await
            .values()
            .filter(|t| category.map(|c| t.category == c).unwrap_or(true))
            .cloned()
            .collect()
    }

    /// Get a template by ID.
    pub async fn get(&self, id: WorkflowTemplateId) -> Option<WorkflowTemplate> {
        self.templates.read().await.get(&id).cloned()
    }

    /// Save a user-defined template.
    pub async fn save(&self, template: WorkflowTemplate) -> WorkflowTemplateId {
        let id = template.id;
        self.templates.write().await.insert(id, template);
        info!(template_id = %id, "Workflow template saved");
        id
    }

    /// Delete a template.
    pub async fn delete(&self, id: WorkflowTemplateId) -> bool {
        self.templates.write().await.remove(&id).is_some()
    }

    /// Instantiate a template into a concrete `Workflow` ready for registration.
    pub fn instantiate(template: &WorkflowTemplate) -> Workflow {
        let steps: Vec<WorkflowStep> = template
            .steps
            .iter()
            .map(|ts| WorkflowStep {
                name: ts.name.clone(),
                agent: StepAgent::ByName {
                    name: ts.agent_name.clone(),
                },
                prompt_template: ts.prompt_template.clone(),
                mode: ts.mode.clone(),
                timeout_secs: ts.timeout_secs,
                error_mode: ts.error_mode.clone(),
                output_var: ts.output_var.clone(),
            })
            .collect();

        Workflow {
            id: WorkflowId::new(),
            name: template.name.clone(),
            description: template.description.clone(),
            steps,
            created_at: Utc::now(),
        }
    }

    // ------------------------------------------------------------------
    // Bundled templates
    // ------------------------------------------------------------------

    fn bundled_templates() -> Vec<WorkflowTemplate> {
        let now = Utc::now();
        vec![
            WorkflowTemplate {
                id: WorkflowTemplateId(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                ),
                name: "research-pipeline".to_string(),
                description: "Multi-step research workflow: search, analyse, and summarise."
                    .to_string(),
                author: "LibreFang".to_string(),
                version: "1.0.0".to_string(),
                category: "research".to_string(),
                tags: vec![
                    "research".into(),
                    "analysis".into(),
                    "summarisation".into(),
                ],
                steps: vec![
                    WorkflowTemplateStep {
                        name: "search".to_string(),
                        agent_name: "search-agent".to_string(),
                        prompt_template: "Search for information about: {{input}}".to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Fail,
                        output_var: Some("search_results".into()),
                        depends_on: vec![],
                    },
                    WorkflowTemplateStep {
                        name: "analyze".to_string(),
                        agent_name: "analyzer-agent".to_string(),
                        prompt_template:
                            "Analyse these search results and extract key findings:\n{{search_results}}"
                                .to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Fail,
                        output_var: Some("analysis".into()),
                        depends_on: vec!["search".into()],
                    },
                    WorkflowTemplateStep {
                        name: "summarize".to_string(),
                        agent_name: "writer-agent".to_string(),
                        prompt_template:
                            "Write a concise summary based on this analysis:\n{{analysis}}"
                                .to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Fail,
                        output_var: None,
                        depends_on: vec!["analyze".into()],
                    },
                ],
                created_at: now,
            },
            WorkflowTemplate {
                id: WorkflowTemplateId(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                ),
                name: "content-creation".to_string(),
                description: "Generate, review, and polish content with parallel critique."
                    .to_string(),
                author: "LibreFang".to_string(),
                version: "1.0.0".to_string(),
                category: "content".to_string(),
                tags: vec!["content".into(), "writing".into(), "review".into()],
                steps: vec![
                    WorkflowTemplateStep {
                        name: "draft".to_string(),
                        agent_name: "writer-agent".to_string(),
                        prompt_template: "Write a first draft about: {{input}}".to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 180,
                        error_mode: ErrorMode::Fail,
                        output_var: Some("draft".into()),
                        depends_on: vec![],
                    },
                    WorkflowTemplateStep {
                        name: "critique-style".to_string(),
                        agent_name: "editor-agent".to_string(),
                        prompt_template:
                            "Review this draft for style and clarity:\n{{draft}}".to_string(),
                        mode: StepMode::FanOut,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Skip,
                        output_var: Some("style_feedback".into()),
                        depends_on: vec!["draft".into()],
                    },
                    WorkflowTemplateStep {
                        name: "critique-accuracy".to_string(),
                        agent_name: "fact-checker-agent".to_string(),
                        prompt_template:
                            "Verify factual claims in this draft:\n{{draft}}".to_string(),
                        mode: StepMode::FanOut,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Skip,
                        output_var: Some("accuracy_feedback".into()),
                        depends_on: vec!["draft".into()],
                    },
                    WorkflowTemplateStep {
                        name: "collect-feedback".to_string(),
                        agent_name: "writer-agent".to_string(),
                        prompt_template: "Incorporate this feedback into the final version:\nStyle: {{style_feedback}}\nAccuracy: {{accuracy_feedback}}\n\nOriginal draft:\n{{draft}}".to_string(),
                        mode: StepMode::Collect,
                        timeout_secs: 180,
                        error_mode: ErrorMode::Fail,
                        output_var: None,
                        depends_on: vec![
                            "critique-style".into(),
                            "critique-accuracy".into(),
                        ],
                    },
                ],
                created_at: now,
            },
            WorkflowTemplate {
                id: WorkflowTemplateId(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
                ),
                name: "code-review".to_string(),
                description:
                    "Automated code review pipeline: lint, security scan, and summarise."
                        .to_string(),
                author: "LibreFang".to_string(),
                version: "1.0.0".to_string(),
                category: "code-analysis".to_string(),
                tags: vec![
                    "code".into(),
                    "review".into(),
                    "security".into(),
                    "quality".into(),
                ],
                steps: vec![
                    WorkflowTemplateStep {
                        name: "lint".to_string(),
                        agent_name: "code-agent".to_string(),
                        prompt_template:
                            "Review this code for style, best practices, and potential bugs:\n{{input}}"
                                .to_string(),
                        mode: StepMode::FanOut,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Skip,
                        output_var: Some("lint_results".into()),
                        depends_on: vec![],
                    },
                    WorkflowTemplateStep {
                        name: "security-scan".to_string(),
                        agent_name: "security-agent".to_string(),
                        prompt_template:
                            "Scan this code for security vulnerabilities:\n{{input}}".to_string(),
                        mode: StepMode::FanOut,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Skip,
                        output_var: Some("security_results".into()),
                        depends_on: vec![],
                    },
                    WorkflowTemplateStep {
                        name: "summary".to_string(),
                        agent_name: "code-agent".to_string(),
                        prompt_template: "Compile a code review summary from these findings:\nLint: {{lint_results}}\nSecurity: {{security_results}}".to_string(),
                        mode: StepMode::Collect,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Fail,
                        output_var: None,
                        depends_on: vec!["lint".into(), "security-scan".into()],
                    },
                ],
                created_at: now,
            },
            WorkflowTemplate {
                id: WorkflowTemplateId(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap(),
                ),
                name: "data-processing-pipeline".to_string(),
                description: "Extract, transform, and load data with validation.".to_string(),
                author: "LibreFang".to_string(),
                version: "1.0.0".to_string(),
                category: "data-processing".to_string(),
                tags: vec![
                    "data".into(),
                    "etl".into(),
                    "pipeline".into(),
                    "transform".into(),
                ],
                steps: vec![
                    WorkflowTemplateStep {
                        name: "extract".to_string(),
                        agent_name: "data-agent".to_string(),
                        prompt_template: "Extract structured data from this input:\n{{input}}"
                            .to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Fail,
                        output_var: Some("raw_data".into()),
                        depends_on: vec![],
                    },
                    WorkflowTemplateStep {
                        name: "transform".to_string(),
                        agent_name: "data-agent".to_string(),
                        prompt_template:
                            "Clean, normalise, and transform this data:\n{{raw_data}}".to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Retry { max_retries: 2 },
                        output_var: Some("clean_data".into()),
                        depends_on: vec!["extract".into()],
                    },
                    WorkflowTemplateStep {
                        name: "validate".to_string(),
                        agent_name: "data-agent".to_string(),
                        prompt_template:
                            "Validate this processed data for completeness and correctness:\n{{clean_data}}"
                                .to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 60,
                        error_mode: ErrorMode::Fail,
                        output_var: None,
                        depends_on: vec!["transform".into()],
                    },
                ],
                created_at: now,
            },
            WorkflowTemplate {
                id: WorkflowTemplateId(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap(),
                ),
                name: "iterative-refinement".to_string(),
                description: "Iteratively refine output using a feedback loop.".to_string(),
                author: "LibreFang".to_string(),
                version: "1.0.0".to_string(),
                category: "general".to_string(),
                tags: vec![
                    "loop".into(),
                    "refinement".into(),
                    "iteration".into(),
                ],
                steps: vec![
                    WorkflowTemplateStep {
                        name: "initial-attempt".to_string(),
                        agent_name: "worker-agent".to_string(),
                        prompt_template: "{{input}}".to_string(),
                        mode: StepMode::Sequential,
                        timeout_secs: 120,
                        error_mode: ErrorMode::Fail,
                        output_var: Some("current".into()),
                        depends_on: vec![],
                    },
                    WorkflowTemplateStep {
                        name: "refine".to_string(),
                        agent_name: "critic-agent".to_string(),
                        prompt_template: "Review and improve this output. Reply 'DONE' if it is already excellent, otherwise provide an improved version:\n{{current}}".to_string(),
                        mode: StepMode::Loop {
                            max_iterations: 3,
                            until: "DONE".to_string(),
                        },
                        timeout_secs: 120,
                        error_mode: ErrorMode::Fail,
                        output_var: None,
                        depends_on: vec!["initial-attempt".into()],
                    },
                ],
                created_at: now,
            },
        ]
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
                },
            ],
            created_at: Utc::now(),
        }
    }

    fn mock_resolver(agent: &StepAgent) -> Option<(AgentId, String)> {
        let _ = agent;
        Some((AgentId::new(), "mock-agent".to_string()))
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
                },
            ],
            created_at: Utc::now(),
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
                },
            ],
            created_at: Utc::now(),
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
            }],
            created_at: Utc::now(),
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
            }],
            created_at: Utc::now(),
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
                },
            ],
            created_at: Utc::now(),
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
            }],
            created_at: Utc::now(),
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
                },
            ],
            created_at: Utc::now(),
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
                },
            ],
            created_at: Utc::now(),
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
        let toml1 = format!(
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

    // -----------------------------------------------------------------------
    // Workflow template tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_template_registry_loads_bundled() {
        let registry = WorkflowTemplateRegistry::new();
        let all = registry.list(None).await;
        assert!(all.len() >= 5, "Should have at least 5 bundled templates");
    }

    #[tokio::test]
    async fn test_template_filter_by_category() {
        let registry = WorkflowTemplateRegistry::new();
        let research = registry.list(Some("research")).await;
        assert_eq!(research.len(), 1);
        assert_eq!(research[0].name, "research-pipeline");
    }

    #[tokio::test]
    async fn test_template_get_by_id() {
        let registry = WorkflowTemplateRegistry::new();
        let id =
            WorkflowTemplateId(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());
        let t = registry
            .get(id)
            .await
            .expect("should find bundled template");
        assert_eq!(t.name, "research-pipeline");
    }

    #[tokio::test]
    async fn test_template_instantiate() {
        let registry = WorkflowTemplateRegistry::new();
        let id =
            WorkflowTemplateId(Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap());
        let t = registry.get(id).await.unwrap();
        let workflow = WorkflowTemplateRegistry::instantiate(&t);
        assert_eq!(workflow.name, "code-review");
        assert_eq!(workflow.steps.len(), 3);
    }

    #[tokio::test]
    async fn test_template_save_and_delete() {
        let registry = WorkflowTemplateRegistry::new();
        let custom = WorkflowTemplate {
            id: WorkflowTemplateId::new(),
            name: "my-custom".to_string(),
            description: "test".to_string(),
            author: "tester".to_string(),
            version: "0.1.0".to_string(),
            category: "custom".to_string(),
            tags: vec!["test".into()],
            steps: vec![],
            created_at: Utc::now(),
        };
        let id = custom.id;
        registry.save(custom).await;
        assert!(registry.get(id).await.is_some());
        assert!(registry.delete(id).await);
        assert!(registry.get(id).await.is_none());
    }
}
