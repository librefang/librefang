//! Low-level cron tools — direct KernelHandle::cron_* wrappers.
//!
//! For the natural-language `schedule_*` family see `tool_runner::schedule`.
//!
//! First module migrated from `Result<String, String>` to
//! `Result<String, ToolError>` (#3576). See
//! `docs/architecture/error-contracts.md` for the migration sequence.

use super::error::{ToolError, ToolResult};
use super::{caller_agent_id_missing, require_kernel_typed};
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

pub(super) async fn tool_cron_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("cron_create"))?;
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
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("cron_list"))?;
    let jobs = kh.cron_list(agent_id).await.map_err(ToolError::upstream)?;
    // `?`-bubble via `From<serde_json::Error> for ToolError`, which preserves
    // the underlying `serde_json::Error` on the `source()` chain rather than
    // stringifying it.
    Ok(serde_json::to_string_pretty(&jobs)?)
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
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("cron_cancel"))?;
    // Authorize: the caller may only cancel jobs that belong to them.
    // Otherwise an agent with the cron_cancel tool could delete any other
    // agent's jobs as long as it learns their UUID (via side-channel or
    // social engineering).
    let owned = kh.cron_list(agent_id).await.map_err(ToolError::upstream)?;
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
    kh.cron_cancel(job_id).await.map_err(ToolError::upstream)?;
    Ok(format!("Cron job '{job_id}' cancelled."))
}

#[cfg(test)]
mod tests {
    //! Pure unit tests for the validation / wiring boundary that runs
    //! BEFORE any kernel call. Cases that require a live KernelHandle
    //! round-trip (every `Ok` path, every `NotFound`/`Upstream` path that
    //! depends on a kernel result) live in the integration test file
    //! `tests/tool_runner_forwarding_task_cron.rs`, which has the full
    //! `CapturingKernel` stub. Unit-test scope here is intentionally narrow
    //! — exercising a sub-trait of `KernelHandle` would mean duplicating
    //! the ~20-supertrait stub and would test the stub, not these fns.
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
    async fn cron_list_without_kernel_returns_unavailable() {
        let r = tool_cron_list(None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn cron_cancel_without_kernel_returns_unavailable() {
        let r = tool_cron_cancel(&json!({"job_id": "x"}), None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[test]
    fn caller_agent_id_missing_surfaces_as_missing_parameter() {
        // The MCP HTTP route (`/mcp`) legitimately passes `None` when the
        // `X-LibreFang-Agent-Id` header is missing — that is a user-input
        // gap, not a server bug, so the variant must lift to InvalidInput
        // (400) rather than Internal (500) at the LibreFangError boundary.
        // Operators still see which tool dropped attribution via the
        // tracing `warn!` next to the constructor.
        let e = caller_agent_id_missing("cron_create");
        assert!(
            matches!(e, ToolError::MissingParameter("agent_id")),
            "expected MissingParameter(\"agent_id\"), got {e:?}"
        );
        assert!(
            e.to_string().contains("agent_id"),
            "rendered message must surface the parameter name, got {}",
            e
        );
    }
}
