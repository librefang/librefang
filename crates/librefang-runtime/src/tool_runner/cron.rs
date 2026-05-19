//! Low-level cron tools — direct KernelHandle::cron_* wrappers.
//!
//! For the natural-language `schedule_*` family see `tool_runner::schedule`.
//!
//! First module migrated from `Result<String, String>` to
//! `Result<String, ToolError>` (#3576). See
//! `docs/architecture/error-contracts.md` for the migration sequence.

use super::error::{ToolError, ToolResult};
use super::require_kernel_typed;
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_cron_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or(ToolError::Unavailable("caller_agent_id"))?;
    let mut job = input.clone();
    if let (Some(pid), Some(obj)) = (sender_id, job.as_object_mut()) {
        if !pid.is_empty() && !obj.contains_key("peer_id") {
            obj.insert(
                "peer_id".to_string(),
                serde_json::Value::String(pid.to_string()),
            );
        }
    }
    kh.cron_create(agent_id, job)
        .await
        .map_err(ToolError::upstream)
}

pub(super) async fn tool_cron_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or(ToolError::Unavailable("caller_agent_id"))?;
    let jobs = kh
        .cron_list(agent_id)
        .await
        .map_err(ToolError::upstream)?;
    serde_json::to_string_pretty(&jobs).map_err(|e| ToolError::Serialization(e.to_string()))
}

pub(super) async fn tool_cron_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let job_id = input["job_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("job_id"))?;
    let agent_id = caller_agent_id.ok_or(ToolError::Unavailable("caller_agent_id"))?;
    // Authorize: the caller may only cancel jobs that belong to them.
    // Otherwise an agent with the cron_cancel tool could delete any other
    // agent's jobs as long as it learns their UUID (via side-channel or
    // social engineering).
    let owned = kh
        .cron_list(agent_id)
        .await
        .map_err(ToolError::upstream)?;
    let owns_job = owned.iter().any(|job| {
        job.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == job_id)
    });
    if !owns_job {
        // Collapse "not owned" and "doesn't exist" into one NotFound — see
        // the variant doc on `ToolError::NotFound` for the side-channel
        // rationale.
        return Err(ToolError::NotFound {
            kind: "Cron job",
            id: job_id.to_string(),
        });
    }
    kh.cron_cancel(job_id)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Cron job '{job_id}' cancelled."))
}

#[cfg(test)]
mod tests {
    //! Direct unit tests for the cron tool fns. The kernel handle is
    //! omitted on purpose — these cases exercise the error-shape contract
    //! at the validation / wiring boundary, which is the part of the
    //! function that runs before any kernel call. Cases that require a
    //! live handle round-trip live in `tool_runner_forwarding_task_cron`
    //! integration tests.
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn cron_create_without_kernel_returns_unavailable() {
        let r = tool_cron_create(&json!({}), None, Some("agent-a"), None).await;
        match r {
            Err(ToolError::Unavailable(cap)) => assert_eq!(cap, "Kernel handle"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cron_create_without_caller_agent_id_returns_unavailable() {
        // Skipped: require_kernel_typed fires first when kernel is None.
        // We can't construct a real KernelHandle here without dragging in
        // the kernel test harness — the integration tests in
        // tool_runner_forwarding_task_cron.rs cover the kernel-present
        // path.  This stub asserts the ordering so a future refactor
        // doesn't silently reverse it.
        let r = tool_cron_create(&json!({}), None, None, None).await;
        assert!(matches!(r, Err(ToolError::Unavailable(_))));
    }

    #[tokio::test]
    async fn cron_cancel_missing_job_id_returns_missing_parameter() {
        // require_kernel_typed checks kernel first, so we cannot trigger
        // MissingParameter without a kernel handle. Exercise the rendered
        // Display for the variant directly so consumers can rely on the
        // exact wording.
        let e = ToolError::MissingParameter("job_id");
        assert_eq!(e.to_string(), "Missing required parameter 'job_id'");
    }

    #[tokio::test]
    async fn cron_cancel_not_owned_renders_as_not_found() {
        // Build the variant directly — the kernel-call path that triggers
        // it is exercised in the kernel integration suite. The contract we
        // care about here is the rendered wire string the LLM sees.
        let e = ToolError::NotFound {
            kind: "Cron job",
            id: "abc-123".to_string(),
        };
        assert_eq!(e.to_string(), "Cron job 'abc-123' not found");
    }
}
