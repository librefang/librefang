//! [`kernel_handle::GoalControl`] — list / update agent goals. Goals are
//! stored as a JSON array under the shared-memory agent's
//! `__librefang_goals` key; this trait centralizes the mutation pattern so
//! callers never reach into the substrate directly.

use librefang_runtime::kernel_handle;

use super::super::{shared_memory_agent_id, LibreFangKernel};

fn goal_not_found(goal_id: &str) -> kernel_handle::KernelOpError {
    kernel_handle::KernelOpError::ResourceNotFound {
        kind: "goal".to_string(),
        id: goal_id.to_string(),
    }
}

fn decode_goal_store(
    current: Option<serde_json::Value>,
    goal_id: &str,
) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
    match current {
        Some(serde_json::Value::Array(goals)) => Ok(goals),
        None => Err(goal_not_found(goal_id)),
        Some(_) => Err(kernel_handle::KernelOpError::Internal(
            "Malformed goal store: `__librefang_goals` must be a JSON array".to_string(),
        )),
    }
}

fn apply_goal_update(
    mut goals: Vec<serde_json::Value>,
    goal_id: &str,
    status: Option<&str>,
    progress: Option<u8>,
) -> Result<(serde_json::Value, serde_json::Value), kernel_handle::KernelOpError> {
    let mut updated_goal = None;
    for goal in &mut goals {
        if goal["id"].as_str() == Some(goal_id) {
            if let Some(status) = status {
                goal["status"] = serde_json::Value::String(status.to_string());
            }
            if let Some(progress) = progress {
                goal["progress"] = serde_json::json!(progress);
            }
            goal["updated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            updated_goal = Some(goal.clone());
            break;
        }
    }

    let result = updated_goal.ok_or_else(|| goal_not_found(goal_id))?;
    Ok((serde_json::Value::Array(goals), result))
}

fn map_goal_update_error(error: kernel_handle::KernelOpError) -> kernel_handle::KernelOpError {
    match error {
        kernel_handle::KernelOpError::Internal(_)
        | kernel_handle::KernelOpError::InvalidInput(_)
        | kernel_handle::KernelOpError::ResourceNotFound { .. } => error,
        other => kernel_handle::KernelOpError::Internal(format!("Failed to save goals: {other}")),
    }
}

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
                let goals = decode_goal_store(current, goal_id)?;
                apply_goal_update(goals, goal_id, status, progress)
            })
            .map_err(map_goal_update_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_goal_store_is_typed_not_found() {
        let error = decode_goal_store(None, "goal-123").unwrap_err();

        assert!(matches!(
            error,
            kernel_handle::KernelOpError::ResourceNotFound { ref kind, ref id }
                if kind == "goal" && id == "goal-123"
        ));
    }

    #[test]
    fn malformed_goal_store_is_internal() {
        let error =
            decode_goal_store(Some(serde_json::json!({"id": "goal-123"})), "goal-123").unwrap_err();

        assert!(matches!(
            error,
            kernel_handle::KernelOpError::Internal(ref message)
                if message.contains("must be a JSON array")
        ));
    }

    #[test]
    fn missing_id_in_valid_goal_store_is_typed_not_found() {
        let error =
            apply_goal_update(Vec::new(), "goal-123", Some("completed"), Some(100)).unwrap_err();

        assert!(matches!(
            error,
            kernel_handle::KernelOpError::ResourceNotFound { ref kind, ref id }
                if kind == "goal" && id == "goal-123"
        ));
    }

    #[test]
    fn transaction_boundary_preserves_goal_not_found() {
        let error = map_goal_update_error(goal_not_found("goal-123"));

        assert!(matches!(
            error,
            kernel_handle::KernelOpError::ResourceNotFound { ref kind, ref id }
                if kind == "goal" && id == "goal-123"
        ));
    }
}
