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

        let (run_id, output) = LibreFangKernel::run_workflow(self, wf_id, input.to_string())
            .await
            .map_err(|e| KernelOpError::Internal(format!("Workflow execution failed: {e}")))?;

        Ok((run_id.to_string(), output))
    }

    async fn list_workflows(&self) -> Vec<kernel_handle::WorkflowSummary> {
        let mut summaries: Vec<kernel_handle::WorkflowSummary> = self
            .workflows
            .engine
            .list_workflows()
            .await
            .into_iter()
            .map(|w| {
                // `has_input_schema` is true when either an explicit
                // `[[input_schema]]` block was authored on the workflow
                // OR any step's prompt_template references at least one
                // `{{var}}` placeholder (auto-detect path). The fallback
                // mirrors `Workflow::to_template()` so the discovery
                // surface stays consistent across both authoring styles
                // (#4982 — gap 2).
                let has_explicit = w
                    .input_schema
                    .as_ref()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_auto = !has_explicit
                    && w.steps.iter().any(|s| {
                        s.prompt_template.contains("{{") && s.prompt_template.contains("}}")
                    });
                kernel_handle::WorkflowSummary::new(
                    w.id.0.to_string(),
                    w.name,
                    w.description,
                    w.steps.len(),
                    has_explicit || has_auto,
                )
            })
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
            let resolver = move |agent_ref: &crate::workflow::StepAgent| {
                use librefang_types::agent::AgentId;
                match agent_ref {
                    crate::workflow::StepAgent::ById { id } => {
                        let agent_id: AgentId = id.parse().ok()?;
                        let entry = k1.agents.registry.get(agent_id)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((agent_id, entry.name.clone(), inherit))
                    }
                    crate::workflow::StepAgent::ByName { name } => {
                        let entry = k1.agents.registry.find_by_name(name)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((entry.id, entry.name.clone(), inherit))
                    }
                }
            };
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

    /// Pins the operator-facing timeout text format. Operators scrape
    /// for `"workflow run timed out after"` and pull the seconds field;
    /// any drift in this string is a breaking change to the contract
    /// the PR explicitly locks in. If you need to change the format,
    /// announce it in the changelog under a breaking-change bullet and
    /// update this assertion.
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
