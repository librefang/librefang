//! [`kernel_handle::WorkflowRunner`] — execute a workflow by UUID or by
//! name. Resolves the name to an id by scanning [`crate::workflow`]'s
//! registered workflows, then delegates to the inherent
//! [`LibreFangKernel::run_workflow`].

use std::sync::OnceLock;

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

/// Render the operator-facing timeout text the async-task tracker emits
/// when an `async_tasks.default_timeout_secs` elapses on a workflow run.
///
/// Pulled out as a free function so the format can be pinned by a
/// string-equality test (refs #5033 review: the operator log-scraper
/// contract claims the text is stable; without a regression test for the
/// exact bytes, a renderer change ship-broke the contract silently).
/// Format: `workflow run timed out after Ns (agent-side default_timeout_secs)`.
pub(crate) fn render_workflow_timeout_text(timeout_secs: u64) -> String {
    format!("workflow run timed out after {timeout_secs}s (agent-side default_timeout_secs)")
}

/// Module-level cache for the `{{var}}` placeholder regex used by the
/// describe-workflow auto-detect path. Compiled exactly once per
/// process — re-compiling per call (the original site) wastes work on
/// every `workflow_describe` invocation and shows up under load. The
/// pattern is a static literal and cannot fail at runtime; we `expect`
/// rather than fall back to "workflow not found" because a regex
/// compile failure is a real bug, not a missing workflow (NIT — see
/// PR #5075 review).
fn placeholder_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"\{\{(\w+)\}\}").expect("workflow_describe placeholder regex compiles")
    })
}

/// Upper bound on the number of steps an agent may put in one workflow.
///
/// `workflow_create` takes LLM-authored input, so every ceiling here is enforced rather than merely advertised — a JSON-schema `maxItems` is advice the calling model is free to ignore.
/// The tool schema publishes the same numbers so a model is never told one ceiling and rejected at another.
pub(crate) const MAX_CREATED_WORKFLOW_STEPS: usize = 50;

/// Upper bound on a single step's `timeout_secs` in an agent-created workflow (1 hour).
pub(crate) const MAX_CREATED_STEP_TIMEOUT_SECS: u64 = 3_600;

/// Upper bound on an agent-created workflow's `total_timeout_secs` (24 hours).
pub(crate) const MAX_CREATED_TOTAL_TIMEOUT_SECS: u64 = 86_400;

/// Upper bound on the length of an agent-supplied workflow name.
pub(crate) const MAX_CREATED_WORKFLOW_NAME_LEN: usize = 64;

/// Does this workflow advertise input parameters an agent can discover?
///
/// True when an explicit `[[input_schema]]` block was authored **or** any step's `prompt_template` carries a `{{var}}` placeholder — the auto-detect fallback, which mirrors `Workflow::to_template()` so the discovery surface reads the same for both authoring styles (#4982 — gap 2).
fn has_discoverable_input_schema(workflow: &crate::workflow::Workflow) -> bool {
    let has_explicit = workflow
        .input_schema
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    has_explicit
        || workflow
            .steps
            .iter()
            .any(|s| s.prompt_template.contains("{{") && s.prompt_template.contains("}}"))
}

/// Build the `WorkflowSummary` the trait boundary hands back for a workflow.
fn summarize_workflow(workflow: &crate::workflow::Workflow) -> kernel_handle::WorkflowSummary {
    kernel_handle::WorkflowSummary::new(
        workflow.id.0.to_string(),
        workflow.name.clone(),
        workflow.description.clone(),
        workflow.steps.len(),
        has_discoverable_input_schema(workflow),
    )
}

/// The agent-authored payload `workflow_create` accepts.
///
/// Deliberately a `Deserialize` over the canonical [`crate::workflow::WorkflowStep`] / [`crate::workflow::WorkflowInputParam`] types rather than a hand-written `Value` walk: the tool's accepted shape is then the struct the engine executes, by construction, and cannot drift from it the way the `POST /api/workflows` parser has.
/// It also means an input parameter's `param_type` is the key the struct actually reads — the same spelling `workflow_describe` reports back — so a described workflow round-trips.
#[derive(serde::Deserialize)]
struct WorkflowCreateSpec {
    name: String,
    #[serde(default)]
    description: String,
    steps: Vec<crate::workflow::WorkflowStep>,
    #[serde(default)]
    total_timeout_secs: Option<u64>,
    #[serde(default)]
    input_schema: Option<Vec<crate::workflow::WorkflowInputParam>>,
}

/// Validate an agent-supplied workflow name.
///
/// `[A-Za-z0-9_-]`, 1–[`MAX_CREATED_WORKFLOW_NAME_LEN`] chars.
/// The name never reaches the filesystem — persistence is keyed by id (`<uuid>.workflow.json`) — so this is not a traversal guard.
/// It exists because the name is how agents address the workflow afterwards and it lands verbatim in prompt text: control characters, newlines and leading/trailing whitespace all produce a workflow that is awkward or impossible to name back.
fn validate_created_workflow_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.chars().count() > MAX_CREATED_WORKFLOW_NAME_LEN {
        return Err(format!(
            "workflow name must be 1-{MAX_CREATED_WORKFLOW_NAME_LEN} characters, got {}",
            name.chars().count()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "workflow name '{name}' must use only letters, digits, '_' and '-'"
        ));
    }
    Ok(())
}

/// Turn an agent-authored spec into a registrable [`crate::workflow::Workflow`], or explain why it cannot be one.
///
/// Pulled out of the trait method as a free function so every rejection branch is unit-testable without booting a kernel — the branches are the whole security surface of an always-available, LLM-driven creation tool.
///
/// The returned `Err` is relayed to the model verbatim, so each one names the offending field and the limit it broke; a model that cannot see which ceiling it hit retries the same payload.
fn build_created_workflow(spec: &serde_json::Value) -> Result<crate::workflow::Workflow, String> {
    use crate::workflow::{Workflow, WorkflowId};

    let spec: WorkflowCreateSpec = serde_json::from_value(spec.clone())
        .map_err(|e| format!("not a valid workflow definition: {e}"))?;

    validate_created_workflow_name(&spec.name)?;

    if spec.steps.is_empty() {
        return Err("a workflow needs at least one step".to_string());
    }
    if spec.steps.len() > MAX_CREATED_WORKFLOW_STEPS {
        return Err(format!(
            "a workflow may declare at most {MAX_CREATED_WORKFLOW_STEPS} steps, got {}",
            spec.steps.len()
        ));
    }

    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for step in &spec.steps {
        if step.name.is_empty() {
            return Err("every step needs a non-empty 'name'".to_string());
        }
        if !seen.insert(step.name.as_str()) {
            return Err(format!(
                "step name '{}' is used twice — step names address dependencies, so they must be unique",
                step.name
            ));
        }
        if step.timeout_secs > MAX_CREATED_STEP_TIMEOUT_SECS {
            return Err(format!(
                "step '{}' timeout_secs={} exceeds the {MAX_CREATED_STEP_TIMEOUT_SECS}s per-step ceiling",
                step.name, step.timeout_secs
            ));
        }
    }
    // Dependencies are checked in a second pass so a step may depend on one declared after it — DAG execution is topological, not positional, and rejecting a forward reference would forbid a legal workflow.
    for step in &spec.steps {
        for dep in &step.depends_on {
            if !seen.contains(dep.as_str()) {
                return Err(format!(
                    "step '{}' depends on '{dep}', which is not a step in this workflow",
                    step.name
                ));
            }
        }
    }

    if let Some(total) = spec.total_timeout_secs {
        if total > MAX_CREATED_TOTAL_TIMEOUT_SECS {
            return Err(format!(
                "total_timeout_secs={total} exceeds the {MAX_CREATED_TOTAL_TIMEOUT_SECS}s per-workflow ceiling"
            ));
        }
    }

    let workflow = Workflow {
        id: WorkflowId::new(),
        name: spec.name,
        description: spec.description,
        steps: spec.steps,
        created_at: chrono::Utc::now(),
        layout: None,
        total_timeout_secs: spec.total_timeout_secs,
        input_schema: spec.input_schema,
    };

    // The same semantic pass `POST /api/workflows` and the canvas run: empty Transform code, unparseable Tera templates, zero / over-cap Wait durations, operator nodes wired into a DAG.
    let errs = workflow.validate();
    if !errs.is_empty() {
        let detail = errs
            .iter()
            .map(|(step, reason)| format!("step '{step}': {reason}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(detail);
    }

    Ok(workflow)
}

#[async_trait::async_trait]
impl kernel_handle::WorkflowRunner for LibreFangKernel {
    async fn run_workflow(
        &self,
        workflow_id: &str,
        input: &str,
    ) -> Result<(String, String), kernel_handle::KernelOpError> {
        use crate::workflow::WorkflowId;
        use kernel_handle::KernelOpError;

        // Try parsing as UUID first, then fall back to name lookup.
        let wf_id = if let Ok(uuid) = uuid::Uuid::parse_str(workflow_id) {
            WorkflowId(uuid)
        } else {
            // Name-based lookup: scan all registered workflows.
            let name_lower = workflow_id.to_lowercase();
            let workflows = self.workflows.engine.list_workflows().await;
            workflows
                .iter()
                .find(|w| w.name.to_lowercase() == name_lower)
                .map(|w| w.id)
                .ok_or_else(|| {
                    KernelOpError::Internal(format!("workflow `{}` not found", workflow_id))
                })?
        };

        // Preserve a policy refusal as a policy refusal (refs #6659).
        // The nesting cap in `LibreFangKernel::run_workflow` raises `CapabilityDenied`, and folding it into `Internal` here would deliver it to `tool_workflow_run` as an opaque upstream failure — a 5xx-class shape that reads as a downstream crash and invites retry, which is exactly the confusion the comment in `tool_agent_send` argues against.
        // `KernelOpError` *is* `LibreFangError`, so the variant survives verbatim.
        // Everything else keeps the historical stringified `Internal` shape, including its prefix.
        let (run_id, output) = LibreFangKernel::run_workflow(self, wf_id, input.to_string())
            .await
            .map_err(|e| match e {
                crate::error::KernelError::LibreFang(
                    librefang_types::error::LibreFangError::CapabilityDenied(msg),
                ) => KernelOpError::CapabilityDenied(msg),
                other => KernelOpError::Internal(format!("Workflow execution failed: {other}")),
            })?;

        Ok((run_id.to_string(), output))
    }

    async fn list_workflows(&self) -> Vec<kernel_handle::WorkflowSummary> {
        let mut summaries: Vec<kernel_handle::WorkflowSummary> = self
            .workflows
            .engine
            .list_workflows()
            .await
            .into_iter()
            .map(|w| summarize_workflow(&w))
            .collect();
        // Sort by name for deterministic prompt output (#3298).
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        summaries
    }

    async fn describe_workflow(
        &self,
        workflow_id: &str,
    ) -> Option<kernel_handle::WorkflowDescription> {
        use crate::workflow::{WorkflowId, WorkflowInputParam as KernelParam};

        // Resolve UUID or name — mirrors run_workflow's resolution path.
        let workflows = self.workflows.engine.list_workflows().await;
        let wf = if let Ok(uuid) = uuid::Uuid::parse_str(workflow_id) {
            let target = WorkflowId(uuid);
            workflows.into_iter().find(|w| w.id == target)?
        } else {
            let name_lower = workflow_id.to_lowercase();
            workflows
                .into_iter()
                .find(|w| w.name.to_lowercase() == name_lower)?
        };

        // Resolve the input parameter list: prefer the explicit
        // `input_schema` when authored, otherwise auto-detect from
        // `{{var}}` placeholders across all steps (same logic as
        // `Workflow::to_template()`, kept narrow rather than calling
        // `to_template` to avoid spinning up the full template
        // structure for a discovery query).
        let params: Vec<kernel_handle::WorkflowInputParam> =
            if let Some(declared) = wf.input_schema.as_ref().filter(|s| !s.is_empty()) {
                let mut out: Vec<kernel_handle::WorkflowInputParam> = declared
                    .iter()
                    .map(|p: &KernelParam| {
                        kernel_handle::WorkflowInputParam::new(
                            p.name.clone(),
                            p.param_type.clone(),
                            p.required,
                            p.description.clone(),
                        )
                    })
                    .collect();
                out.sort_by(|a, b| a.name.cmp(&b.name));
                out
            } else {
                // Auto-detect `{{var}}` placeholders. `{{input}}` is the
                // workflow-engine reserved name for "previous step output";
                // skip it so the agent's parameter list contains only
                // user-supplied keys. Regex is cached at module level so
                // we don't recompile per call.
                let re = placeholder_regex();
                let mut seen = std::collections::BTreeSet::new();
                for step in &wf.steps {
                    for cap in re.captures_iter(&step.prompt_template) {
                        let name = cap[1].to_string();
                        if name == "input" {
                            continue;
                        }
                        seen.insert(name);
                    }
                }
                seen.into_iter()
                    .map(|name| {
                        let description = Some(format!(
                            "Auto-detected from {{{{{name}}}}} placeholder in step prompt"
                        ));
                        kernel_handle::WorkflowInputParam::new(
                            name,
                            "string".to_string(),
                            true,
                            description,
                        )
                    })
                    .collect()
            };

        let step_names = wf.steps.iter().map(|s| s.name.clone()).collect();

        Some(kernel_handle::WorkflowDescription::new(
            wf.id.0.to_string(),
            wf.name,
            wf.description,
            step_names,
            params,
        ))
    }

    async fn get_workflow_run(&self, run_id: &str) -> Option<kernel_handle::WorkflowRunSummary> {
        use crate::workflow::WorkflowRunId;

        let uuid = uuid::Uuid::parse_str(run_id).ok()?;
        let run = self.workflows.engine.get_run(WorkflowRunId(uuid)).await?;

        let state = serde_json::to_value(&run.state)
            .ok()
            .and_then(|v| {
                // `WorkflowRunState` serializes as snake_case string or object for Paused.
                // Extract the variant name string.
                if v.is_string() {
                    v.as_str().map(|s| s.to_string())
                } else if let Some(obj) = v.as_object() {
                    obj.keys().next().map(|k| k.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        // Per-step name + output, trimmed view for #4982 structured-
        // results delivery. Kept in execution order so the agent can
        // navigate "stage 3 output" by index. Full
        // `StepResult { agent_id, prompt, tokens, duration_ms, ... }`
        // stays kernel-side for dashboard / audit consumers.
        let step_outputs = run
            .step_results
            .iter()
            .map(|r| kernel_handle::StepOutputSummary::new(r.step_name.clone(), r.output.clone()))
            .collect();

        let step_count = run.step_results.len();
        let last_step_name = run.step_results.last().map(|r| r.step_name.clone());
        Some(kernel_handle::WorkflowRunSummary::new(
            run.id.0.to_string(),
            run.workflow_id.0.to_string(),
            run.workflow_name,
            state,
            run.started_at.to_rfc3339(),
            run.completed_at.map(|t| t.to_rfc3339()),
            run.output,
            run.error,
            step_count,
            last_step_name,
            step_outputs,
        ))
    }

    async fn start_workflow_async(
        &self,
        workflow_id: &str,
        input: &str,
    ) -> Result<String, kernel_handle::KernelOpError> {
        // Forward to the tracker-aware variant with no caller context.
        // Historical call sites (cron, triggers) that don't carry an
        // `(agent, session)` keep their previous behaviour — the
        // async-task tracker simply does not register an entry, so no
        // `TaskCompletionEvent` is injected. Refs #4983.
        self.start_workflow_async_tracked(workflow_id, input, None, None)
            .await
    }

    async fn start_workflow_async_tracked(
        &self,
        workflow_id: &str,
        input: &str,
        caller_agent_id: Option<&str>,
        caller_session_id: Option<&str>,
    ) -> Result<String, kernel_handle::KernelOpError> {
        use crate::workflow::WorkflowId;
        use kernel_handle::KernelOpError;
        use librefang_types::agent::{AgentId, SessionId};
        use librefang_types::task::{TaskKind, TaskStatus};

        // Resolve workflow_id (UUID or name) — same logic as run_workflow.
        let wf_id = if let Ok(uuid) = uuid::Uuid::parse_str(workflow_id) {
            WorkflowId(uuid)
        } else {
            let name_lower = workflow_id.to_lowercase();
            let workflows = self.workflows.engine.list_workflows().await;
            workflows
                .iter()
                .find(|w| w.name.to_lowercase() == name_lower)
                .map(|w| w.id)
                .ok_or_else(|| {
                    KernelOpError::Internal(format!("workflow `{}` not found", workflow_id))
                })?
        };

        let run_id = self
            .workflows
            .engine
            .create_run(wf_id, input.to_string())
            .await
            .ok_or_else(|| KernelOpError::Internal("Workflow not found".to_string()))?;

        // Async task tracker registration (#4983). Only register
        // when both pieces of caller context were supplied AND parse
        // successfully; otherwise spawn the workflow without tracking so
        // historical cron / trigger callers keep working unchanged.
        //
        // Also pull the caller agent's `[async_tasks]` manifest block
        // while we have the `AgentId` so the spawned
        // task below can honour `default_timeout_secs` /
        // `notify_on_timeout`. Cached here (rather than re-fetched in
        // the spawned closure) because the agent registry lookup is a
        // sync DashMap op and we want it to fail fast at registration
        // time if the agent disappears mid-flight.
        let (task_id, async_cfg) = match (caller_agent_id, caller_session_id) {
            (Some(aid), Some(sid)) => match (aid.parse::<AgentId>(), sid.parse::<SessionId>()) {
                (Ok(agent_id), Ok(session_id)) => {
                    let handle = self.register_async_task(
                        agent_id,
                        session_id,
                        TaskKind::Workflow { run_id },
                        None,
                    );
                    let cfg = self
                        .agents
                        .registry
                        .get(agent_id)
                        .map(|entry| entry.manifest.async_tasks.clone())
                        .unwrap_or_default();
                    (Some(handle.id), Some(cfg))
                }
                _ => {
                    tracing::debug!(
                        caller_agent_id = %aid,
                        caller_session_id = %sid,
                        run_id = %run_id,
                        "start_workflow_async_tracked: caller context failed to parse; skipping registry registration"
                    );
                    (None, None)
                }
            },
            _ => (None, None),
        };

        // Spawn execution in the background via self_handle (same pattern as
        // trigger dispatch — upgrade the stored Weak<LibreFangKernel> so the
        // spawned task can call send_message through the full kernel).
        let kernel_arc = self
            .self_handle
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| {
                KernelOpError::Internal(
                    "kernel not yet initialised for async workflow spawn".to_string(),
                )
            })?;

        tokio::spawn(async move {
            // Both closures must be `Fn` (not `FnOnce`), so we clone the Arc
            // on each invocation rather than moving it into the closure body.
            let k1 = std::sync::Arc::clone(&kernel_arc);
            let k2 = std::sync::Arc::clone(&kernel_arc);
            let resolver =
                move |agent_ref: &crate::workflow::StepAgent| k1.resolve_step_agent(agent_ref);
            // `session_mode_override` carries `WorkflowStep::session_mode`
            // (#4834). Threaded through `send_message_full`'s existing
            // session-mode-override slot so the async-spawn path matches the
            // synchronous `run_workflow` path in precedence: per-step
            // override > target agent manifest > Persistent default.
            let send_message = move |agent_id: librefang_types::agent::AgentId,
                                     message: String,
                                     session_mode_override: Option<
                librefang_types::agent::SessionMode,
            >| {
                let k = std::sync::Arc::clone(&k2);
                async move {
                    let handle = k.kernel_handle();
                    k.send_message_full(
                        agent_id,
                        &message,
                        handle,
                        None,
                        None,
                        session_mode_override,
                        None,
                        None,
                    )
                    .await
                    .map(|r| {
                        (
                            r.response,
                            r.total_usage.input_tokens,
                            r.total_usage.output_tokens,
                        )
                    })
                    .map_err(|e| format!("{e}"))
                }
            };
            // (#4983) honour the caller's `[async_tasks]
            // default_timeout_secs` so a workflow that hangs gets
            // cancelled and surfaced to the agent as a Failed
            // completion. None → run unbounded (timeout ownership is
            // agent-side, per the module-level design).
            let timeout = async_cfg
                .as_ref()
                .and_then(|c| c.default_timeout_secs)
                .map(std::time::Duration::from_secs);
            let notify_on_timeout = async_cfg
                .as_ref()
                .map(|c| c.notify_on_timeout)
                .unwrap_or(true);

            // Don't swallow the result — without a log the agent that
            // called workflow_start has no way to learn the run failed
            // except by polling get_workflow_run for the Failed state.
            let exec_fut = kernel_arc
                .workflows
                .engine
                .execute_run(run_id, resolver, send_message);
            let exec_result: Result<Result<String, String>, ()> = match timeout {
                Some(d) => match tokio::time::timeout(d, exec_fut).await {
                    Ok(inner) => Ok(inner),
                    Err(_elapsed) => Err(()),
                },
                None => Ok(exec_fut.await),
            };

            // Async task tracker delivery (#4983). Only emit a
            // completion event if a registration happened above.
            if let Some(task_id) = task_id {
                let terminal_status = match &exec_result {
                    Ok(Ok(output)) => TaskStatus::Completed(serde_json::json!({
                        "run_id": run_id.0.to_string(),
                        "output": output,
                    })),
                    Ok(Err(e)) => TaskStatus::Failed(format!("workflow run failed: {e}")),
                    Err(()) => {
                        let secs = timeout.map(|d| d.as_secs()).unwrap_or(0);
                        TaskStatus::Failed(render_workflow_timeout_text(secs))
                    }
                };

                // `notify_on_timeout = false` suppresses ONLY the
                // timeout-specific Failed event; success / non-timeout
                // failures still surface as today. Step-3 design
                // decision: operationally meaningful only for batch
                // agents whose sessions are never read by a human.
                let suppress = matches!(exec_result, Err(())) && !notify_on_timeout;
                if !suppress {
                    if let Err(err) = kernel_arc
                        .complete_async_task(task_id, terminal_status)
                        .await
                    {
                        tracing::warn!(
                            task_id = %task_id,
                            run_id = %run_id,
                            "Failed to inject TaskCompletionEvent: {err}"
                        );
                    }
                }
            }

            match &exec_result {
                Ok(Err(e)) => tracing::warn!(
                    run_id = %run_id,
                    "Async workflow execution failed: {e}"
                ),
                Err(()) => tracing::warn!(
                    run_id = %run_id,
                    "Async workflow execution timed out after {}s",
                    timeout.map(|d| d.as_secs()).unwrap_or(0)
                ),
                Ok(Ok(_)) => {}
            }
        });

        Ok(run_id.0.to_string())
    }

    async fn create_workflow(
        &self,
        spec: &serde_json::Value,
        caller_agent_id: Option<&str>,
    ) -> Result<kernel_handle::WorkflowSummary, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;

        let workflow = build_created_workflow(spec).map_err(KernelOpError::InvalidInput)?;
        // Built before the move into the engine; the engine returns only the id.
        let summary = summarize_workflow(&workflow);
        let name = workflow.name.clone();

        // `register_unique_name`, not `list_workflows()`-then-`register()`: the name check and the insert have to be one operation or two agents creating the same name concurrently both win, and name-based lookup then resolves to whichever duplicate the registry iterator reaches first (#6934).
        let id = self
            .workflows
            .engine
            .register_unique_name(workflow)
            .await
            .map_err(|taken| KernelOpError::Conflict(taken.to_string()))?;

        // Provenance trace, not an authorization gate: workflows have no ownership model, and an agent-authored one is executable by any agent the moment it is registered.
        // Logging who asked for it is what makes that reviewable after the fact.
        tracing::info!(
            workflow_id = %id,
            workflow_name = %name,
            caller_agent_id = caller_agent_id.unwrap_or("<unattributed>"),
            step_count = summary.step_count,
            "Agent created a workflow"
        );

        Ok(summary)
    }

    async fn cancel_workflow_run(&self, run_id: &str) -> Result<(), kernel_handle::KernelOpError> {
        use crate::workflow::{CancelRunError, WorkflowRunId};
        use kernel_handle::KernelOpError;

        let uuid = uuid::Uuid::parse_str(run_id)
            .map_err(|_| KernelOpError::Internal(format!("Invalid run_id UUID: {run_id}")))?;

        self.workflows
            .engine
            .cancel_run(WorkflowRunId(uuid))
            .await
            .map_err(|e| match e {
                CancelRunError::NotFound(_) => {
                    KernelOpError::Internal(format!("workflow run not found: {run_id}"))
                }
                CancelRunError::AlreadyTerminal { state, .. } => {
                    KernelOpError::Internal(format!("cannot cancel: run is already {state}"))
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal spec that `build_created_workflow` accepts, as JSON, so each test below can mutate exactly the field it is about.
    fn spec(steps: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "name": "nightly-report",
            "description": "summarise the day",
            "steps": steps,
        })
    }

    fn one_step() -> serde_json::Value {
        serde_json::json!([{
            "name": "write",
            "agent": "writer",
            "prompt_template": "Summarise {{input}}",
        }])
    }

    #[test]
    fn build_created_workflow_accepts_a_minimal_spec() {
        let wf = build_created_workflow(&spec(one_step())).expect("minimal spec is valid");
        assert_eq!(wf.name, "nightly-report");
        assert_eq!(wf.steps.len(), 1);
        // Bare-string `agent` is the documented shorthand for by-name routing.
        assert!(matches!(
            &wf.steps[0].agent,
            crate::workflow::StepAgent::ByName { name } if name == "writer"
        ));
        // Serde defaults fill the fields the model left out, rather than a hand-written parser inventing its own.
        assert_eq!(wf.steps[0].timeout_secs, 120);
        assert!(matches!(
            wf.steps[0].mode,
            crate::workflow::StepMode::Sequential
        ));
    }

    /// The input-parameter key is `param_type` — the spelling `WorkflowInputParam` deserializes and `workflow_describe` reports — so a described workflow round-trips back through creation with its declared types intact.
    #[test]
    fn build_created_workflow_preserves_declared_param_types() {
        let mut s = spec(one_step());
        s["input_schema"] = serde_json::json!([
            { "name": "cover", "param_type": "image", "required": false },
            { "name": "topic" },
        ]);
        let wf = build_created_workflow(&s).expect("input schema is valid");
        let schema = wf.input_schema.expect("input_schema survives");
        assert_eq!(schema[0].param_type, "image");
        assert!(!schema[0].required);
        // Defaults apply to the parameter the model under-specified.
        assert_eq!(schema[1].param_type, "string");
        assert!(schema[1].required);
    }

    #[test]
    fn build_created_workflow_rejects_a_spec_without_steps() {
        let err = build_created_workflow(&spec(serde_json::json!([])))
            .expect_err("a stepless workflow can never run");
        assert!(err.contains("at least one step"), "{err}");
    }

    #[test]
    fn build_created_workflow_rejects_a_step_without_an_agent() {
        let s = spec(serde_json::json!([{ "name": "write", "prompt_template": "go" }]));
        let err = build_created_workflow(&s).expect_err("a step needs an agent");
        assert!(err.contains("not a valid workflow definition"), "{err}");
    }

    /// The ceiling is inclusive: exactly `MAX_CREATED_WORKFLOW_STEPS` steps is a legal workflow, one more is not.
    /// Pinned in one test so a future edit cannot quietly move the boundary by one.
    #[test]
    fn build_created_workflow_caps_the_step_count() {
        let steps = |n: usize| {
            serde_json::Value::Array(
                (0..n)
                    .map(|i| {
                        serde_json::json!({
                            "name": format!("step-{i}"),
                            "agent": "writer",
                            "prompt_template": "go",
                        })
                    })
                    .collect(),
            )
        };
        assert!(build_created_workflow(&spec(steps(MAX_CREATED_WORKFLOW_STEPS))).is_ok());
        let err = build_created_workflow(&spec(steps(MAX_CREATED_WORKFLOW_STEPS + 1)))
            .expect_err("one step over the ceiling must be rejected");
        assert!(
            err.contains(&MAX_CREATED_WORKFLOW_STEPS.to_string()),
            "{err}"
        );
    }

    #[test]
    fn build_created_workflow_caps_both_timeouts() {
        let mut s = spec(serde_json::json!([{
            "name": "write",
            "agent": "writer",
            "prompt_template": "go",
            "timeout_secs": MAX_CREATED_STEP_TIMEOUT_SECS + 1,
        }]));
        let err = build_created_workflow(&s).expect_err("step timeout over the ceiling");
        assert!(err.contains("per-step ceiling"), "{err}");

        s = spec(one_step());
        s["total_timeout_secs"] = serde_json::json!(MAX_CREATED_TOTAL_TIMEOUT_SECS + 1);
        let err = build_created_workflow(&s).expect_err("total timeout over the ceiling");
        assert!(err.contains("per-workflow ceiling"), "{err}");
    }

    /// Step names address dependencies, so a duplicate makes `depends_on` ambiguous and an unknown target makes it unsatisfiable — both produce a workflow that cannot run, and both are cheap to catch at creation.
    #[test]
    fn build_created_workflow_rejects_broken_step_dependencies() {
        let dup = spec(serde_json::json!([
            { "name": "write", "agent": "writer", "prompt_template": "a" },
            { "name": "write", "agent": "writer", "prompt_template": "b" },
        ]));
        let err = build_created_workflow(&dup).expect_err("duplicate step names");
        assert!(err.contains("used twice"), "{err}");

        let dangling = spec(serde_json::json!([{
            "name": "write",
            "agent": "writer",
            "prompt_template": "a",
            "depends_on": ["research"],
        }]));
        let err = build_created_workflow(&dangling).expect_err("unknown dependency");
        assert!(err.contains("not a step in this workflow"), "{err}");
    }

    /// A dependency on a step declared later is legal — DAG execution is topological, not positional.
    #[test]
    fn build_created_workflow_allows_a_forward_dependency() {
        let s = spec(serde_json::json!([
            { "name": "publish", "agent": "writer", "prompt_template": "a", "depends_on": ["research"] },
            { "name": "research", "agent": "analyst", "prompt_template": "b" },
        ]));
        assert!(build_created_workflow(&s).is_ok());
    }

    #[test]
    fn created_workflow_names_are_constrained() {
        assert!(validate_created_workflow_name("nightly-report").is_ok());
        assert!(validate_created_workflow_name("report_v2").is_ok());
        assert!(validate_created_workflow_name("").is_err());
        assert!(validate_created_workflow_name("../../etc/passwd").is_err());
        assert!(validate_created_workflow_name("has space").is_err());
        assert!(validate_created_workflow_name("line\nbreak").is_err());
        assert!(validate_created_workflow_name(&"a".repeat(MAX_CREATED_WORKFLOW_NAME_LEN)).is_ok());
        assert!(
            validate_created_workflow_name(&"a".repeat(MAX_CREATED_WORKFLOW_NAME_LEN + 1)).is_err()
        );
    }

    /// The semantic pass `POST /api/workflows` runs must run here too — an agent-authored operator node is exactly as unrunnable as a hand-written one.
    #[test]
    fn build_created_workflow_runs_the_shared_semantic_validation() {
        let s = spec(serde_json::json!([{
            "name": "pause",
            "agent": "writer",
            "prompt_template": "",
            "mode": { "wait": { "duration_secs": 0 } },
        }]));
        let err = build_created_workflow(&s).expect_err("a zero-second wait is rejected");
        assert!(err.contains("wait.duration_secs"), "{err}");
    }

    /// The ceilings the tool schema advertises must be the ceilings this module enforces.
    ///
    /// They live in two crates — the schema in `librefang-runtime`, the limits here — so nothing but a test keeps them in step, and a model told one number and rejected at another burns a turn discovering the difference.
    /// The kernel is the only place both are visible.
    #[test]
    fn the_tool_schema_advertises_the_ceilings_this_module_enforces() {
        let defs = librefang_runtime::tool_runner::builtin_tool_definitions();
        let schema = &defs
            .iter()
            .find(|d| d.name == "workflow_create")
            .expect("workflow_create must be a builtin tool")
            .input_schema;

        assert_eq!(
            schema["properties"]["steps"]["maxItems"].as_u64(),
            Some(MAX_CREATED_WORKFLOW_STEPS as u64),
            "advertised step ceiling must match the enforced one"
        );
        assert_eq!(
            schema["properties"]["steps"]["items"]["properties"]["timeout_secs"]["maximum"]
                .as_u64(),
            Some(MAX_CREATED_STEP_TIMEOUT_SECS),
            "advertised per-step timeout ceiling must match the enforced one"
        );
        assert_eq!(
            schema["properties"]["total_timeout_secs"]["maximum"].as_u64(),
            Some(MAX_CREATED_TOTAL_TIMEOUT_SECS),
            "advertised total timeout ceiling must match the enforced one"
        );
        let name_doc = schema["properties"]["name"]["description"]
            .as_str()
            .expect("the name property must be documented");
        assert!(
            name_doc.contains(&format!("1-{MAX_CREATED_WORKFLOW_NAME_LEN}")),
            "the name length rule must be advertised as enforced, got: {name_doc}"
        );
    }

    /// Every step field `WorkflowCreateSpec` accepts and acts on must be advertised in the tool schema.
    ///
    /// `WorkflowCreateSpec` deserialises the canonical [`crate::workflow::WorkflowStep`], so the tool silently accepts every field that struct grows — agent-type routing, `required_skills`, `session_mode` — whether or not the published schema mentions them.
    /// A field accepted but unadvertised is a field no model will ever send, and the `workflow-creator` skill would be documenting keys the tool's own schema denies.
    /// This asserts the acceptance and the advertisement together, so the two cannot drift apart in either direction.
    #[test]
    fn the_tool_schema_advertises_the_step_routing_fields_the_spec_accepts() {
        use crate::workflow::StepAgent;
        use librefang_types::agent::SessionMode;

        let wf = build_created_workflow(&spec(serde_json::json!([{
            "name": "review",
            "agent": {"type": "code-reviewer"},
            "prompt_template": "Review {{input}}",
            "required_skills": ["git-expert"],
            "session_mode": "new",
            "inherit_context": false,
        }])))
        .expect("agent-type routing, required_skills and session_mode are accepted");

        let step = &wf.steps[0];
        assert!(
            matches!(&step.agent, StepAgent::ByType { template } if template == "code-reviewer"),
            "`{{\"type\": …}}` must bind find-or-spawn, got {:?}",
            step.agent
        );
        assert_eq!(step.required_skills, vec!["git-expert".to_string()]);
        assert!(matches!(step.session_mode, Some(SessionMode::New)));
        assert_eq!(step.inherit_context, Some(false));

        let defs = librefang_runtime::tool_runner::builtin_tool_definitions();
        let step_props = &defs
            .iter()
            .find(|d| d.name == "workflow_create")
            .expect("workflow_create must be a builtin tool")
            .input_schema["properties"]["steps"]["items"]["properties"];

        for field in ["required_skills", "session_mode", "inherit_context"] {
            assert!(
                step_props[field].is_object(),
                "step field `{field}` is accepted by workflow_create but absent from its schema"
            );
        }
        let agent_doc = step_props["agent"]["description"]
            .as_str()
            .expect("the agent property must be documented");
        for routing_key in crate::workflow::STEP_AGENT_ROUTING_KEYS {
            assert!(
                agent_doc.contains(&format!("\"{routing_key}\"")),
                "the agent binding doc must name the `{routing_key}` routing key, got: {agent_doc}"
            );
        }
    }

    /// Pins the operator-facing timeout text format.
    /// Operators scrape for `"workflow run timed out after"` and pull the seconds field; any drift in this string is a breaking change to the contract the PR explicitly locks in.
    /// If you need to change the format, announce it in the changelog under a breaking-change bullet and update this assertion.
    #[test]
    fn workflow_timeout_text_format_is_stable() {
        assert_eq!(
            render_workflow_timeout_text(30),
            "workflow run timed out after 30s (agent-side default_timeout_secs)"
        );
        assert_eq!(
            render_workflow_timeout_text(0),
            "workflow run timed out after 0s (agent-side default_timeout_secs)"
        );
    }
}
