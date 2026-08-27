use async_trait::async_trait;

use super::*;

// ============================================================================
// 12. WorkflowRunner — declarative workflow execution
// ============================================================================

/// Summary of a registered workflow definition, used by `workflow_list`.
///
/// `#[non_exhaustive]` because the #4982 rich-invocation work is staged
/// across PRs and additional fields (param-type strictness, dashboard
/// hints) are expected next; future additions stay non-breaking for
/// external consumers that pattern-match.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub step_count: usize,
    /// `true` when the workflow advertises typed input parameters that the
    /// agent can discover via `workflow_describe`. `false` when the workflow
    /// has neither an explicit `input_schema` nor any `{{var}}` placeholder
    /// in its step templates (i.e. nothing parametric to discover).
    pub has_input_schema: bool,
    /// The principal the workflow was recorded as belonging to (#7744), or
    /// `None` when the creating turn resolved none.
    pub owner: Option<librefang_types::principal::Principal>,
}

/// One parameter advertised by a workflow's input schema (#4982 — gap 2).
///
/// Authored explicitly via `[[input_schema]]` blocks in the workflow TOML
/// **or** auto-detected from `{{var_name}}` placeholders in step
/// `prompt_template`s when no explicit schema is present (matching the
/// existing `Workflow::to_template()` extraction behaviour).
///
/// Lives on the trait boundary as a plain struct (no `serde` derives) so
/// `librefang-kernel-handle` stays free of a `serde` dep — consumers
/// (`librefang-runtime::tool_runner`) build the JSON shape they ship to
/// the agent by hand from these fields.
///
/// `#[non_exhaustive]` — see [`WorkflowSummary`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowInputParam {
    /// Parameter name — corresponds to the `{{name}}` placeholder key in
    /// step prompt templates and to the JSON-object key the caller passes
    /// in `workflow_run` / `workflow_start` input.
    pub name: String,
    /// Expected value type. One of `"string" | "number" | "boolean" |
    /// "file" | "image" | "agent_id"`. `"file"` / `"image"` indicate the
    /// caller may pass an `{"_artifact": "sha256:<64-hex>"}` reference
    /// (#4982 — gap 3) that the runtime resolves to the artifact-store
    /// handle string before the workflow engine substitutes it into the
    /// step prompt.
    pub param_type: String,
    /// Whether the caller must supply this parameter. Defaults to `true`
    /// when auto-detected (every `{{var}}` is presumed required absent
    /// schema information).
    pub required: bool,
    /// Optional human-readable description shown in the discovery surface.
    pub description: Option<String>,
}

/// Result of `workflow_describe` — workflow metadata plus the input schema
/// the agent needs to call `workflow_run` / `workflow_start` correctly.
///
/// `#[non_exhaustive]` — see [`WorkflowSummary`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowDescription {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Each step's display name. **Preserves declaration order** —
    /// downstream consumers (and the agent's user-facing confirmation
    /// dialog) rely on this being the same order the steps execute, so
    /// the "stage 3 output" lookup by index lines up.
    pub step_names: Vec<String>,
    /// Parameters the caller can supply. **Sorted by name** for
    /// deterministic LLM prompt output (#3298); the workflow's authoring
    /// order is intentionally not preserved here.
    pub input_schema: Vec<WorkflowInputParam>,
}

/// One step's name + final output in a completed workflow run. Returned
/// alongside the top-level workflow output so the agent can navigate into
/// intermediate-stage results rather than only seeing the final string
/// (#4982 — gap 3 / "structured results").
///
/// `#[non_exhaustive]` — see [`WorkflowSummary`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StepOutputSummary {
    pub step_name: String,
    pub output: String,
}

/// Summary of a workflow run instance, used by `workflow_status`.
///
/// `#[non_exhaustive]` — see [`WorkflowSummary`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowRunSummary {
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub step_count: usize,
    pub last_step_name: Option<String>,
    /// Per-step name + output in execution order (#4982 — structured
    /// results). Empty for runs that have not yet produced any step
    /// output. The full step prompt / token-usage / duration shape stays
    /// on the kernel-side `StepResult`; this trimmed view ships only the
    /// fields the agent navigates against.
    pub step_outputs: Vec<StepOutputSummary>,
}

// Constructors for the `#[non_exhaustive]` types above. The attribute
// blocks struct-literal construction from outside this crate; downstream
// crates (`librefang-kernel`, `librefang-runtime`'s tests + tool surface)
// build instances through these `new()` methods instead. Future field
// additions land here as `with_<field>(self, …)` setters so existing
// callers keep compiling.
impl WorkflowSummary {
    pub fn new(
        id: String,
        name: String,
        description: String,
        step_count: usize,
        has_input_schema: bool,
        owner: Option<librefang_types::principal::Principal>,
    ) -> Self {
        Self {
            id,
            name,
            description,
            step_count,
            has_input_schema,
            owner,
        }
    }
}

impl WorkflowInputParam {
    pub fn new(
        name: String,
        param_type: String,
        required: bool,
        description: Option<String>,
    ) -> Self {
        Self {
            name,
            param_type,
            required,
            description,
        }
    }
}

impl WorkflowDescription {
    pub fn new(
        id: String,
        name: String,
        description: String,
        step_names: Vec<String>,
        input_schema: Vec<WorkflowInputParam>,
    ) -> Self {
        Self {
            id,
            name,
            description,
            step_names,
            input_schema,
        }
    }
}

impl StepOutputSummary {
    pub fn new(step_name: String, output: String) -> Self {
        Self { step_name, output }
    }
}

impl WorkflowRunSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: String,
        workflow_id: String,
        workflow_name: String,
        state: String,
        started_at: String,
        completed_at: Option<String>,
        output: Option<String>,
        error: Option<String>,
        step_count: usize,
        last_step_name: Option<String>,
        step_outputs: Vec<StepOutputSummary>,
    ) -> Self {
        Self {
            run_id,
            workflow_id,
            workflow_name,
            state,
            started_at,
            completed_at,
            output,
            error,
            step_count,
            last_step_name,
            step_outputs,
        }
    }
}

#[async_trait]
pub trait WorkflowRunner: Send + Sync {
    /// Run a workflow by ID or name. The `workflow_id` can be a UUID string or a
    /// workflow name. The `input` is an arbitrary string (typically JSON-encoded
    /// parameters) passed to the first step. Returns `(run_id, output)` on success.
    async fn run_workflow(
        &self,
        workflow_id: &str,
        input: &str,
    ) -> Result<(String, String), KernelOpError> {
        let _ = (workflow_id, input);
        Err(KernelOpError::unavailable("Workflow engine"))
    }

    /// Owner-aware variant of [`Self::run_workflow`] (#7714).
    ///
    /// `caller_agent_id` is the agent that invoked the `workflow_run` tool.
    /// The kernel records it on the run as the run's owner, which is what lets
    /// two owners drive the same shared step-agent type and still keep their
    /// attribution apart.
    ///
    /// `&str` rather than `AgentId` for trait-object compatibility, matching
    /// [`Self::start_workflow_async_tracked`]; the kernel parses it and treats
    /// an unparseable or absent value as an ownerless run.
    ///
    /// The default impl forwards to [`Self::run_workflow`] and drops the
    /// owner, so an implementor that has not adopted ownership keeps its
    /// current behaviour.
    async fn run_workflow_owned(
        &self,
        workflow_id: &str,
        input: &str,
        caller_agent_id: Option<&str>,
    ) -> Result<(String, String), KernelOpError> {
        let _ = caller_agent_id;
        self.run_workflow(workflow_id, input).await
    }

    /// List all registered workflow definitions, sorted by name for determinism.
    async fn list_workflows(&self) -> Vec<WorkflowSummary> {
        Vec::new()
    }

    /// Describe a workflow by ID or name — returns its declared input
    /// parameters, step names, and human-readable description so the agent
    /// can discover *how to call* a workflow before invoking it (#4982 —
    /// gap 2). Returns `None` when no workflow matches.
    async fn describe_workflow(&self, workflow_id: &str) -> Option<WorkflowDescription> {
        let _ = workflow_id;
        None
    }

    /// Get the status of a workflow run by its UUID string.
    /// Returns `None` if the run ID is not found (including UUID parse failure).
    async fn get_workflow_run(&self, run_id: &str) -> Option<WorkflowRunSummary> {
        let _ = run_id;
        None
    }

    /// Start a workflow asynchronously (fire-and-forget). Creates the run,
    /// spawns execution in the background, and returns the `run_id`
    /// immediately without blocking. Use `get_workflow_run` to poll status.
    ///
    /// Default impl forwards to [`Self::start_workflow_async_tracked`]
    /// with no caller context — historical callers that don't carry an
    /// `(agent, session)` keep working but get no async-task tracker
    /// registration (#4983).
    async fn start_workflow_async(
        &self,
        workflow_id: &str,
        input: &str,
    ) -> Result<String, KernelOpError> {
        self.start_workflow_async_tracked(workflow_id, input, None, None)
            .await
    }

    /// Tracker-aware variant of [`Self::start_workflow_async`] introduced
    /// for the async task tracker (#4983). When the optional
    /// `caller_agent_id` and `caller_session_id` are both `Some`, the
    /// kernel registers a [`librefang_types::task::TaskKind::Workflow`]
    /// entry against the originating session and will inject a
    /// [`librefang_types::task::TaskCompletionEvent`] when the workflow
    /// reaches a terminal state.
    ///
    /// Both inputs are `&str` for trait-object compatibility: the kernel
    /// parses them into `AgentId` / `SessionId` internally. If either
    /// parses to `None`, the call still spawns the workflow normally but
    /// skips the registry registration (no completion event will be
    /// injected). This mirrors the existing pattern in
    /// `KernelHandle::run_workflow`'s string-id surface.
    async fn start_workflow_async_tracked(
        &self,
        workflow_id: &str,
        input: &str,
        caller_agent_id: Option<&str>,
        caller_session_id: Option<&str>,
    ) -> Result<String, KernelOpError> {
        let _ = (workflow_id, input, caller_agent_id, caller_session_id);
        Err(KernelOpError::unavailable("Workflow engine"))
    }

    /// Create and register a new workflow from an agent-authored spec (#6934).
    ///
    /// `spec` is the tool payload as received — `{ name, description, steps[], input_schema?, total_timeout_secs? }`.
    /// It is passed through as JSON rather than as a typed struct because the workflow types live in `librefang-kernel`, which the runtime cannot depend on; the kernel deserializes it into its own `Workflow` so the wire shape can never drift from the struct the engine executes.
    ///
    /// Note that this is **not** the `POST /api/workflows` body: that endpoint has its own hand-written parser over different field names (`agent_id` / `agent_name` / `prompt`), while this path deserializes the canonical workflow shape (`agent` / `prompt_template`) — the same one `*.workflow.toml` files use.
    ///
    /// Every check lives on the kernel side of this boundary — name legality, resource caps, `Workflow::validate()`, and the uniqueness of the name — because the caller is an LLM and a JSON-schema constraint is advice it is free to ignore.
    ///
    /// `caller_agent_id` is the *executor* — the agent whose turn assembled the spec — and stays a trace on the registration log line.
    ///
    /// `owner` is the *principal that turn was acting for*, and it is a different thing (#7744): the agent is who typed it, the principal is who it belongs to.
    /// It is recorded on the `Workflow` itself rather than only in a log, because a log rotates and the workflow outlives it — which is the concrete gap this parameter closes.
    /// It is still not an authorization gate: nothing consults it to decide who may run, edit or delete the workflow, and an agent-authored workflow remains executable by any agent the moment it is registered.
    /// `None` records the workflow as unowned, which is what a turn with no authenticated caller, no manifest `owner` and no `default_owner` produces.
    ///
    /// It is passed separately from `spec` because `spec` is forwarded to the deserializer verbatim, and an owner the model could put in its own tool input would be an owner the model could choose.
    ///
    /// Errors: `InvalidInput` for a spec that cannot become a valid workflow, `Conflict` when the name is already registered, `Unavailable` when no workflow engine is wired.
    async fn create_workflow(
        &self,
        spec: &serde_json::Value,
        caller_agent_id: Option<&str>,
        owner: Option<librefang_types::principal::Principal>,
    ) -> Result<WorkflowSummary, KernelOpError> {
        let _ = (spec, caller_agent_id, owner);
        Err(KernelOpError::unavailable("Workflow engine"))
    }

    /// Cancel a running or paused workflow run by its UUID string.
    /// Returns `Ok(())` on success, or an error describing why cancellation
    /// failed (not found, already in a terminal state, etc.).
    async fn cancel_workflow_run(&self, run_id: &str) -> Result<(), KernelOpError> {
        let _ = run_id;
        Err(KernelOpError::unavailable("Workflow engine"))
    }
}
