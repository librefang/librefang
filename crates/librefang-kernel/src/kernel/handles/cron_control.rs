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
        owner: Option<librefang_types::principal::Principal>,
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
            // #7744: the principal the creating turn acted for. Note it is
            // read from the typed parameter and never from `job_json`, which
            // is the model's own input — the same reason `peer_id` above is
            // overwritten by the caller rather than trusted.
            owner,
            session_mode,
            enabled: true,
            created_at: chrono::Utc::now(),
            next_run: None,
            last_run: None,
        };

        let id = self.workflows.cron_scheduler.add_job(job, one_shot)?;

        // Same contract as the workflow path: unowned is supported, and worth
        // saying out loud exactly once so an operator who meant to attribute
        // their artifacts finds out from the log (#7744).
        if owner.is_none() {
            librefang_types::principal::warn_once_unowned("cron job");
            tracing::debug!(
                cron_job_id = %id,
                "Cron job recorded without an owner — the creating turn had no authenticated caller, the agent manifest declares no `owner`, and `config.toml` declares no `default_owner`"
            );
        }

        // Persist after adding
        if let Err(e) = self.workflows.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(serde_json::json!({
            "job_id": id.to_string(),
            "status": "created",
            // Canonical `kind:uuid`, so the model can report who the job was
            // recorded for instead of guessing. Absent when unowned.
            "owner": owner.map(|p| p.to_string()),
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
        let jobs = self.workflows.cron_scheduler.list_jobs(aid);
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
        self.workflows
            .cron_scheduler
            .remove_job(id)
            .map_err(|e| KernelOpError::Internal(e.to_string()))?;

        // Persist after removal
        if let Err(e) = self.workflows.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(())
    }

    async fn cron_set_enabled(
        &self,
        job_id: &str,
        enabled: bool,
    ) -> Result<(), kernel_handle::KernelOpError> {
        use kernel_handle::KernelOpError;
        let id = librefang_types::scheduler::CronJobId(
            uuid::Uuid::parse_str(job_id)
                .map_err(|e| KernelOpError::InvalidInput(format!("job_id: {e}")))?,
        );
        self.workflows
            .cron_scheduler
            .set_enabled(id, enabled)
            .map_err(|e| KernelOpError::Internal(e.to_string()))?;

        // Persist after the toggle, mirroring `toggle_cron_job` in the HTTP route (#3515): an in-memory enable/disable that never reaches disk would silently revert on daemon restart.
        if let Err(e) = self.workflows.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::LibreFangKernel;
    use librefang_runtime::kernel_handle::CronControl;
    use librefang_types::config::KernelConfig;

    /// An agent-authored job must not be able to point the daemon-spawned wake gate at an executable outside `<home>/scripts/`.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn cron_create_rejects_pre_check_script_outside_scripts_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let home_dir = dir.path().to_path_buf();
        std::fs::create_dir_all(home_dir.join("data")).unwrap();
        std::fs::create_dir_all(home_dir.join("scripts")).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
        let agent_id = kernel
            .agents
            .registry
            .list()
            .iter()
            .find(|a| a.name == "assistant")
            .expect("assistant should exist")
            .id
            .to_string();

        let job_json = serde_json::json!({
            "name": "pre-check-escape",
            "schedule": { "kind": "cron", "expr": "0 * * * *" },
            "action": {
                "kind": "agent_turn",
                "message": "hi",
                "pre_check_script": "/bin/sh",
            },
        });
        let err = kernel
            .cron_create(&agent_id, job_json, None)
            .await
            .expect_err("pre_check_script outside <home>/scripts must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("pre_check_script") && msg.contains("allowlist"),
            "got: {msg}"
        );
        assert!(kernel
            .cron_list(&agent_id)
            .await
            .unwrap()
            .iter()
            .all(|j| j["name"].as_str() != Some("pre-check-escape")));

        kernel.shutdown();
    }
}
