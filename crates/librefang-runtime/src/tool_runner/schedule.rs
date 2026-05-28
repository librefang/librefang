//! `schedule_*` tools — high-level wrappers around the CronScheduler engine.
//!
//! Accept natural language schedules ("daily at 9am") and delegate to
//! `kh.cron_create/list/cancel`, which use the real kernel tick loop (#2024).
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576) — second slice after `tool_runner::cron`. The internal
//! `parse_schedule_to_cron` / `parse_time_to_hour` helpers keep their
//! `Result<_, String>` shape (a pure sub-layer with no kernel contact) and
//! are mapped to `ToolError::InvalidParameter` at the tool boundary.

use super::error::{ToolError, ToolResult};
use super::{caller_agent_id_missing, require_kernel_typed};
use crate::kernel_handle::prelude::*;
use std::sync::Arc;

/// Parse a natural language schedule into a cron expression.
pub(super) fn parse_schedule_to_cron(input: &str) -> Result<String, String> {
    let input = input.trim().to_lowercase();

    // If it already looks like a cron expression (5 space-separated fields), pass through
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 5
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_digit() || "*/,-".contains(c)))
    {
        return Ok(input);
    }

    // Natural language patterns
    if let Some(rest) = input.strip_prefix("every ") {
        if rest == "minute" || rest == "1 minute" {
            return Ok("* * * * *".to_string());
        }
        if let Some(mins) = rest.strip_suffix(" minutes") {
            let n: u32 = mins
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 59 {
                return Err(format!("Minutes must be 1-59, got {n}"));
            }
            return Ok(format!("*/{n} * * * *"));
        }
        if rest == "hour" || rest == "1 hour" {
            return Ok("0 * * * *".to_string());
        }
        if let Some(hrs) = rest.strip_suffix(" hours") {
            let n: u32 = hrs
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 23 {
                return Err(format!("Hours must be 1-23, got {n}"));
            }
            return Ok(format!("0 */{n} * * *"));
        }
        if rest == "day" || rest == "1 day" {
            return Ok("0 0 * * *".to_string());
        }
        if rest == "week" || rest == "1 week" {
            return Ok("0 0 * * 0".to_string());
        }
    }

    // "daily at Xam/pm"
    if let Some(time_str) = input.strip_prefix("daily at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * *"));
    }

    // "weekdays at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekdays at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 1-5"));
    }

    // "weekends at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekends at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 0,6"));
    }

    // "hourly" / "daily" / "weekly" / "monthly"
    match input.as_str() {
        "hourly" => return Ok("0 * * * *".to_string()),
        "daily" => return Ok("0 0 * * *".to_string()),
        "weekly" => return Ok("0 0 * * 0".to_string()),
        "monthly" => return Ok("0 0 1 * *".to_string()),
        _ => {}
    }

    Err(format!(
        "Could not parse schedule '{input}'. Try: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', or a cron expression like '0 */5 * * *'"
    ))
}

/// Parse a time string like "9am", "6pm", "14:00", "9:30am" into an hour (0-23).
pub(super) fn parse_time_to_hour(s: &str) -> Result<u32, String> {
    let s = s.trim().to_lowercase();

    // Handle "9am", "6pm", "12pm", "12am"
    if let Some(h) = s.strip_suffix("am") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(0),
            1..=11 => Ok(hour),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }
    if let Some(h) = s.strip_suffix("pm") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(12),
            1..=11 => Ok(hour + 12),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }

    // Handle "14:00" or "9:30"
    if let Some((h, _m)) = s.split_once(':') {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        if hour > 23 {
            return Err(format!("Hour must be 0-23, got {hour}"));
        }
        return Ok(hour);
    }

    // Plain number
    let hour: u32 = s.parse().map_err(|_| format!("Invalid time: {s}"))?;
    if hour > 23 {
        return Err(format!("Hour must be 0-23, got {hour}"));
    }
    Ok(hour)
}

pub(super) async fn tool_schedule_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    sender_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("schedule_create"))?;
    let description = input["description"]
        .as_str()
        .ok_or(ToolError::MissingParameter("description"))?;
    let schedule_str = input["schedule"]
        .as_str()
        .ok_or(ToolError::MissingParameter("schedule"))?;
    let message = input["message"].as_str().unwrap_or(description);

    let cron_expr =
        parse_schedule_to_cron(schedule_str).map_err(|reason| ToolError::InvalidParameter {
            name: "schedule",
            reason,
        })?;

    // CronJob name only allows alphanumeric + space/hyphen/underscore (max 128 chars).
    // Sanitize the user-provided description to fit these constraints.
    let name: String = description
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .take(128)
        .collect();
    let name = if name.is_empty() {
        "scheduled-task".to_string()
    } else {
        name
    };

    // Build CronJob JSON compatible with kh.cron_create()
    let tz = input["tz"].as_str();
    let schedule = if let Some(tz_str) = tz {
        serde_json::json!({ "kind": "cron", "expr": cron_expr, "tz": tz_str })
    } else {
        serde_json::json!({ "kind": "cron", "expr": cron_expr })
    };
    let mut job_json = serde_json::json!({
        "name": name,
        "schedule": schedule,
        "action": { "kind": "agent_turn", "message": message },
        "delivery": { "kind": "none" },
    });
    if let Some(obj) = job_json.as_object_mut() {
        if !obj.contains_key("peer_id") {
            if let Some(pid) = sender_id {
                if !pid.is_empty() {
                    obj.insert(
                        "peer_id".to_string(),
                        serde_json::Value::String(pid.to_string()),
                    );
                }
            }
        }
    }

    let result = kh
        .cron_create(agent_id, job_json)
        .await
        .map_err(ToolError::upstream)?;
    Ok(format!(
        "Schedule created and will execute automatically.\n  Cron: {cron_expr}\n  Original: {schedule_str}\n  {result}"
    ))
}

pub(super) async fn tool_schedule_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("schedule_list"))?;
    let jobs = kh.cron_list(agent_id).await.map_err(ToolError::upstream)?;

    if jobs.is_empty() {
        return Ok("No scheduled tasks.".to_string());
    }

    let mut output = format!("Scheduled tasks ({}):\n\n", jobs.len());
    for j in &jobs {
        let enabled = j["enabled"].as_bool().unwrap_or(true);
        let status = if enabled { "active" } else { "paused" };
        let schedule_display = j["schedule"]["expr"]
            .as_str()
            .or_else(|| j["schedule"]["every_secs"].as_u64().map(|_| "interval"))
            .unwrap_or("?");
        output.push_str(&format!(
            "  [{status}] {} — {}\n    Schedule: {}\n    Next run: {}\n\n",
            j["id"].as_str().unwrap_or("?"),
            j["name"].as_str().unwrap_or("?"),
            schedule_display,
            j["next_run"].as_str().unwrap_or("pending"),
        ));
    }

    Ok(output)
}

pub(super) async fn tool_schedule_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> ToolResult {
    let kh = require_kernel_typed(kernel)?;
    // Accept either "id" or "job_id" for backward compatibility
    let id = input["id"]
        .as_str()
        .or_else(|| input["job_id"].as_str())
        .ok_or(ToolError::MissingParameter("id"))?;
    let agent_id = caller_agent_id.ok_or_else(|| caller_agent_id_missing("schedule_delete"))?;
    // Authorize: the caller may only delete jobs that belong to them.
    // `KernelHandle::cron_cancel` removes by UUID with no ownership check
    // (see `kernel/handles/cron_control.rs`), and the sibling `cron_cancel`
    // tool already enforces this guard at the tool layer — `schedule_delete`
    // must too, or it is a trivial bypass: any agent with this tool could
    // cancel another agent's job by learning its UUID.
    let owned = kh.cron_list(agent_id).await.map_err(ToolError::upstream)?;
    let owns_job = owned.iter().any(|job| {
        job.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|jid| jid == id)
    });
    if !owns_job {
        // Collapse "not owned" and "doesn't exist" into one NotFound — see
        // the variant doc on `ToolError::NotFound` for the side-channel
        // rationale (mirrors `cron_cancel`).
        return Err(ToolError::NotFound {
            kind: "Schedule",
            id: id.to_string(),
        });
    }
    kh.cron_cancel(id).await.map_err(ToolError::upstream)?;
    Ok(format!("Schedule '{id}' deleted."))
}

#[cfg(test)]
mod tests {
    //! Pure validation / wiring-boundary tests that run BEFORE any kernel
    //! call. The ownership-check path in `tool_schedule_delete` (which needs
    //! `cron_list` to return jobs) lives in the integration test file
    //! `tests/tool_runner_forwarding_task_cron.rs` alongside the cron
    //! equivalents, where the `CapturingKernel` stub is available — same split
    //! as `tool_runner::cron`.
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn schedule_create_without_kernel_returns_unavailable() {
        let r = tool_schedule_create(&json!({}), None, Some("agent-a"), None).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn schedule_list_without_kernel_returns_unavailable() {
        let r = tool_schedule_list(None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[tokio::test]
    async fn schedule_delete_without_kernel_returns_unavailable() {
        let r = tool_schedule_delete(&json!({"id": "x"}), None, Some("agent-a")).await;
        assert!(matches!(r, Err(ToolError::Unavailable("Kernel handle"))));
    }

    #[test]
    fn caller_agent_id_missing_surfaces_as_missing_parameter() {
        let e = caller_agent_id_missing("schedule_delete");
        assert!(
            matches!(e, ToolError::MissingParameter("agent_id")),
            "expected MissingParameter(\"agent_id\"), got {e:?}"
        );
        assert!(e.to_string().contains("agent_id"));
    }
}
