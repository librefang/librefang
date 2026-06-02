use super::*;

// ============================================================================
// 13. GoalControl — list and update agent goals
// ============================================================================

pub trait GoalControl: Send + Sync {
    /// List active goals (pending or in_progress), optionally filtered by agent ID.
    /// Returns a JSON array of goal objects.
    fn goal_list_active(
        &self,
        _agent_id: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, KernelOpError> {
        Ok(Vec::new())
    }

    /// Update a goal's status and/or progress. Returns the updated goal JSON.
    fn goal_update(
        &self,
        _goal_id: &str,
        _status: Option<&str>,
        _progress: Option<u8>,
    ) -> Result<serde_json::Value, KernelOpError> {
        Err(KernelOpError::unavailable("Goal system"))
    }
}
