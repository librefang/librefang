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
use std::collections::HashSet;
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
    if let Some(obj) = job.as_object_mut() {
        match sender_id {
            Some(pid) if !pid.is_empty() => {
                // Always override peer_id with authenticated sender_id —
                // caller cannot inject arbitrary peer_id.
                obj.insert(
                    "peer_id".to_string(),
                    serde_json::Value::String(pid.to_string()),
                );
            }
            _ => {
                obj.remove("peer_id");
            }
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

/// Disable a cron job by ID — the agent-facing "stop this job" action.
///
/// Despite the historical `cron_cancel` tool name, this pauses the job via `cron_set_enabled(false)` rather than hard-deleting it: the schedule / action / delivery config is preserved so the operator can recover what was set up and the agent (or a human) can re-enable it later.
/// Hard deletion is a human-only dashboard operation (#6159).
/// Re-enable via `tool_cron_enable`.
pub(super) async fn tool_cron_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let job_id = require_owned_job_id(input, kernel, caller_agent_id, "cron_cancel").await?;
    let kh = require_kernel_typed(kernel)?;
    kh.cron_set_enabled(&job_id, false)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!(
        "Cron job '{job_id}' disabled (paused). Its configuration is preserved — re-enable it with cron_enable."
    ))
}

/// Enable (resume) a previously disabled cron job by ID.
pub(super) async fn tool_cron_enable(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let job_id = require_owned_job_id(input, kernel, caller_agent_id, "cron_enable").await?;
    let kh = require_kernel_typed(kernel)?;
    kh.cron_set_enabled(&job_id, true)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!("Cron job '{job_id}' enabled (resumed)."))
}

/// Validate the `job_id` parameter and confirm the caller owns the job.
///
/// `KernelHandle::cron_set_enabled` toggles by UUID with no ownership check (see `kernel/handles/cron_control.rs`), so the ownership guard must live at the tool layer — otherwise any agent could pause/resume another agent's job by learning its UUID.
/// Returns the validated, owned `job_id` as an owned `String` (the borrow on `kh.cron_list(...)` ends before the caller's own kernel call).
async fn require_owned_job_id(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    tool: &'static str,
) -> Result<String, ToolError> {
    let job_id = input["job_id"]
        .as_str()
        .ok_or(ToolError::MissingParameter("job_id"))?;
    if job_id.is_empty() {
        return Err(ToolError::InvalidParameter {
            name: "job_id",
            reason: "must not be empty".to_string(),
        });
    }
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing(tool))?;
    let owned = kh.cron_list(agent_id).await.map_err(ToolError::upstream)?;
    let owned_ids: HashSet<&str> = owned
        .iter()
        .filter_map(|job| job.get("id").and_then(|v| v.as_str()))
        .collect();
    if !owned_ids.contains(job_id) {
        return Err(ToolError::NotFound {
            kind: "Cron job",
            id: job_id.to_string(),
        });
    }
    Ok(job_id.to_string())
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

    #[tokio::test]
    async fn cron_cancel_empty_job_id_rejected() {
        let r = tool_cron_cancel(&json!({"job_id": ""}), None, Some("agent-a")).await;
        match r {
            Err(ToolError::InvalidParameter { name, reason }) => {
                assert_eq!(name, "job_id");
                assert!(
                    reason.contains("empty"),
                    "reason should mention empty: {reason}"
                );
            }
            other => panic!("expected InvalidParameter, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cron_enable_without_kernel_returns_unavailable() {
        let r = tool_cron_enable(&json!({"job_id": "x"}), None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn cron_enable_empty_job_id_rejected() {
        let r = tool_cron_enable(&json!({"job_id": ""}), None, Some("agent-a")).await;
        match r {
            Err(ToolError::InvalidParameter { name, reason }) => {
                assert_eq!(name, "job_id");
                assert!(
                    reason.contains("empty"),
                    "reason should mention empty: {reason}"
                );
            }
            other => panic!("expected InvalidParameter, got {other:?}"),
        }
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
