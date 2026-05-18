//! [`kernel_handle::GoalControl`] — list / update agent goals. Goals are
//! stored as a JSON array under the shared-memory agent's
//! `__librefang_goals` key; this trait centralizes the mutation pattern so
//! callers never reach into the substrate directly.

use librefang_runtime::kernel_handle;

use super::super::{shared_memory_agent_id, LibreFangKernel};

impl kernel_handle::GoalControl for LibreFangKernel {
    fn goal_list_active(
        &self,
        agent_id_filter: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
        let shared_id = shared_memory_agent_id();
        let goals: Vec<serde_json::Value> = match self
            .memory
            .substrate
            .structured_get(shared_id, "__librefang_goals")
        {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            Ok(_) => return Ok(Vec::new()),
            Err(e) => {
                return Err(kernel_handle::KernelOpError::Internal(format!(
                    "Failed to load goals: {e}"
                )))
            }
        };
        let active: Vec<serde_json::Value> = goals
            .into_iter()
            .filter(|g| {
                let status = g["status"].as_str().unwrap_or("");
                let is_active = status == "pending" || status == "in_progress";
                if !is_active {
                    return false;
                }
                match agent_id_filter {
                    Some(aid) => g["agent_id"].as_str() == Some(aid),
                    None => true,
                }
            })
            .collect();
        Ok(active)
    }

    fn goal_update(
        &self,
        goal_id: &str,
        status: Option<&str>,
        progress: Option<u8>,
    ) -> Result<serde_json::Value, kernel_handle::KernelOpError> {
        let shared_id = shared_memory_agent_id();
        // RMW under a single BEGIN IMMEDIATE transaction (#5138). Two
        // concurrent `goal_update` / `POST /api/goals` / `PUT
        // /api/goals/{id}` calls previously each loaded the same array,
        // each edited it, and the last writer clobbered the other's goal
        // (lost update). `structured_modify` serializes the load+mutate+
        // store so neither write is lost.
        self.memory
            .substrate
            .structured_modify(shared_id, "__librefang_goals", |current| {
                let mut goals: Vec<serde_json::Value> = match current {
                    Some(serde_json::Value::Array(arr)) => arr,
                    _ => {
                        return Err(kernel_handle::KernelOpError::Internal(format!(
                            "goal `{goal_id}` not found"
                        )))
                    }
                };

                let mut updated_goal = None;
                for g in goals.iter_mut() {
                    if g["id"].as_str() == Some(goal_id) {
                        if let Some(s) = status {
                            g["status"] = serde_json::Value::String(s.to_string());
                        }
                        if let Some(p) = progress {
                            g["progress"] = serde_json::json!(p);
                        }
                        g["updated_at"] =
                            serde_json::Value::String(chrono::Utc::now().to_rfc3339());
                        updated_goal = Some(g.clone());
                        break;
                    }
                }

                let result = updated_goal.ok_or_else(|| {
                    kernel_handle::KernelOpError::Internal(format!("goal `{goal_id}` not found"))
                })?;

                Ok((serde_json::Value::Array(goals), result))
            })
            .map_err(|e| match e {
                kernel_handle::KernelOpError::Internal(_)
                | kernel_handle::KernelOpError::InvalidInput(_) => e,
                other => {
                    kernel_handle::KernelOpError::Internal(format!("Failed to save goals: {other}"))
                }
            })
    }
}
