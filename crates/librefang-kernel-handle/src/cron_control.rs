use async_trait::async_trait;

use super::*;

// ============================================================================
// 6. CronControl — agent-owned scheduled jobs
// ============================================================================

#[async_trait]
pub trait CronControl: Send + Sync {
    /// Create a cron job for the calling agent.
    ///
    /// `owner` is the principal the creating turn was acting for (#7744) — recorded on the job, never read as an authorization.
    /// It is a typed parameter rather than a key inside `job_json` because `job_json` is the model's own tool input: a field the model can write is a field the model can choose, and an owner the creating turn can name is not an owner.
    /// `None` records the job as unowned, which is what a turn with no authenticated caller, no manifest `owner` and no `default_owner` produces.
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
        owner: Option<librefang_types::principal::Principal>,
    ) -> Result<String, KernelOpError> {
        let _ = (agent_id, job_json, owner);
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

    /// Enable or disable a cron job by ID, preserving its configuration.
    ///
    /// This is the agent-facing alternative to `cron_cancel`: disabling a job pauses it without losing the schedule / action / delivery config, so an operator can recover what was set up and re-enable it later.
    /// Agent tools route their "stop this job" action here rather than to `cron_cancel`; hard deletion stays a human-only dashboard operation (#6159).
    async fn cron_set_enabled(&self, job_id: &str, enabled: bool) -> Result<(), KernelOpError> {
        let _ = (job_id, enabled);
        Err(KernelOpError::unavailable("Cron scheduler"))
    }
}
