//! [`kernel_handle::CronControl`] — cron job CRUD against
//! [`crate::cron_scheduler`]. Validates the JSON shape up front (so a bad
//! field produces a clear error before the job is added) and persists after
//! every mutation.

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

#[async_trait::async_trait]
impl kernel_handle::CronControl for LibreFangKernel {
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        use librefang_types::scheduler::{
            CronAction, CronDelivery, CronDeliveryTarget, CronJob, CronJobId, CronSchedule,
        };

        let name = job_json["name"]
            .as_str()
            .ok_or_else(|| {
                KernelOpError::InvalidInput("name: missing or not a string".to_string())
            })?
            .to_string();
        let schedule: CronSchedule = serde_json::from_value(job_json["schedule"].clone())
            .map_err(|e| KernelOpError::InvalidInput(format!("schedule: {e}")))?;
        let action: CronAction = serde_json::from_value(job_json["action"].clone())
            .map_err(|e| KernelOpError::InvalidInput(format!("action: {e}")))?;
        let delivery: CronDelivery = if job_json["delivery"].is_object() {
            serde_json::from_value(job_json["delivery"].clone())
                .map_err(|e| KernelOpError::InvalidInput(format!("delivery: {e}")))?
        } else {
            // Default to LastChannel so cron jobs created by an agent in
            // a channel context actually deliver their output back to
            // that channel. The previous default (`None`) silently
            // dropped every result and gave users no way to recover the
            // originating channel without explicit `delivery` config.
            // Issue #2338.
            CronDelivery::LastChannel
        };
        // At-schedules are inherently single-execution; default one_shot=true for them
        // so the job auto-deletes after firing instead of lingering as a zombie (#2808).
        let is_at_schedule = matches!(schedule, CronSchedule::At { .. });
        let one_shot = job_json["one_shot"].as_bool().unwrap_or(is_at_schedule);

        let aid = librefang_types::agent::AgentId(
            uuid::Uuid::parse_str(agent_id)
                .map_err(|e| KernelOpError::InvalidInput(format!("agent_id: {e}")))?,
        );

        let session_mode: Option<librefang_types::agent::SessionMode> =
            if job_json["session_mode"].is_string() {
                serde_json::from_value(job_json["session_mode"].clone())
                    .map_err(|e| KernelOpError::InvalidInput(format!("session_mode: {e}")))?
            } else {
                None
            };

        // Multi-destination fan-out targets. Optional; missing/null = empty.
        // Validate each entry up front so a bad shape produces a clear error
        // before the job is added (rather than failing silently at fire time).
        let delivery_targets: Vec<CronDeliveryTarget> = if job_json["delivery_targets"].is_array() {
            serde_json::from_value(job_json["delivery_targets"].clone())
                .map_err(|e| KernelOpError::InvalidInput(format!("delivery_targets: {e}")))?
        } else {
            Vec::new()
        };

        let job = CronJob {
            id: CronJobId::new(),
            agent_id: aid,
            name,
            schedule,
            action,
            delivery,
            delivery_targets,
            peer_id: job_json["peer_id"].as_str().map(|s| s.to_string()),
            session_mode,
            enabled: true,
            created_at: chrono::Utc::now(),
            next_run: None,
            last_run: None,
        };

        let id = self
            .cron_scheduler
            .add_job(job, one_shot)
            .map_err(|e| KernelOpError::Internal(e.to_string()))?;

        // Persist after adding
        if let Err(e) = self.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(serde_json::json!({
            "job_id": id.to_string(),
            "status": "created"
        })
        .to_string())
    }

    async fn cron_list(
        &self,
        agent_id: &str,
    ) -> Result<Vec<serde_json::Value>, kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let aid = librefang_types::agent::AgentId(
            uuid::Uuid::parse_str(agent_id)
                .map_err(|e| KernelOpError::InvalidInput(format!("agent_id: {e}")))?,
        );
        let jobs = self.cron_scheduler.list_jobs(aid);
        let json_jobs: Vec<serde_json::Value> = jobs
            .into_iter()
            .map(|j| serde_json::to_value(&j).unwrap_or_default())
            .collect();
        Ok(json_jobs)
    }

    async fn cron_cancel(&self, job_id: &str) -> Result<(), kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let id = librefang_types::scheduler::CronJobId(
            uuid::Uuid::parse_str(job_id)
                .map_err(|e| KernelOpError::InvalidInput(format!("job_id: {e}")))?,
        );
        self.cron_scheduler
            .remove_job(id)
            .map_err(|e| KernelOpError::Internal(e.to_string()))?;

        // Persist after removal
        if let Err(e) = self.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(())
    }
}
