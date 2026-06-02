use async_trait::async_trait;

use super::*;

// ============================================================================
// 6. CronControl — agent-owned scheduled jobs
// ============================================================================

#[async_trait]
pub trait CronControl: Send + Sync {
    /// Create a cron job for the calling agent.
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, KernelOpError> {
        let _ = (agent_id, job_json);
        Err(KernelOpError::unavailable("Cron scheduler"))
    }

    /// List cron jobs for the calling agent.
    async fn cron_list(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, KernelOpError> {
        let _ = agent_id;
        Err(KernelOpError::unavailable("Cron scheduler"))
    }

    /// Cancel a cron job by ID.
    async fn cron_cancel(&self, job_id: &str) -> Result<(), KernelOpError> {
        let _ = job_id;
        Err(KernelOpError::unavailable("Cron scheduler"))
    }
}
