//! [`kernel_handle::PromptStore`] — prompt-version + experiment management.
//! All ops require `prompt_intelligence.enabled = true` and a populated
//! prompt store; otherwise they return `Unavailable` or no-op cleanly.
//! `update_experiment_status(Completed)` auto-activates the winning
//! variant's prompt version.

use librefang_runtime::kernel_handle;
use librefang_types::agent::AgentId;

use super::super::LibreFangKernel;

impl kernel_handle::PromptStore for LibreFangKernel {
    fn get_running_experiment(
        &self,
        agent_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptExperiment>, kernel_handle::KernelOpError>
    {
        let cfg = self.config.load();
        if !cfg.prompt_intelligence.enabled {
            return Ok(None);
        }
        let id: AgentId = agent_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid agent ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.get_running_experiment(id).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to get experiment: {e}"))
        })
    }

    fn record_experiment_request(
        &self,
        experiment_id: &str,
        variant_id: &str,
        latency_ms: u64,
        cost_usd: f64,
        success: bool,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let exp_id: uuid::Uuid = experiment_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid experiment ID: {e}"))
        })?;
        let var_id: uuid::Uuid = variant_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid variant ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or(kernel_handle::KernelOpError::unavailable("Prompt store"))?;
        store
            .record_request(exp_id, var_id, latency_ms, cost_usd, success)
            .map_err(|e| {
                kernel_handle::KernelOpError::Internal(format!("Failed to record request: {e}"))
            })
    }

    fn get_prompt_version(
        &self,
        version_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptVersion>, kernel_handle::KernelOpError> {
        let id: uuid::Uuid = version_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid version ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or(kernel_handle::KernelOpError::unavailable("Prompt store"))?;
        store.get_version(id).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to get version: {e}"))
        })
    }

    fn list_prompt_versions(
        &self,
        agent_id: librefang_types::agent::AgentId,
    ) -> Result<Vec<librefang_types::agent::PromptVersion>, kernel_handle::KernelOpError> {
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.list_versions(agent_id).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to list versions: {e}"))
        })
    }

    fn create_prompt_version(
        &self,
        version: &librefang_types::agent::PromptVersion,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let cfg = self.config.load();
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or(kernel_handle::KernelOpError::unavailable("Prompt store"))?;
        let agent_id = version.agent_id;
        // Clone here — the store owns the value. Trade-off accepted by
        // #3553: callers (API handlers) no longer have to clone first.
        store.create_version(version.clone()).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to create version: {e}"))
        })?;
        // Prune old versions if over the configured limit
        let max = cfg.prompt_intelligence.max_versions_per_agent;
        let _ = store.prune_old_versions(agent_id, max);
        Ok(())
    }

    fn delete_prompt_version(&self, version_id: &str) -> Result<(), kernel_handle::KernelOpError> {
        let id: uuid::Uuid = version_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid version ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.delete_version(id).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to delete version: {e}"))
        })
    }

    fn set_active_prompt_version(
        &self,
        version_id: &str,
        agent_id: &str,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let id: uuid::Uuid = version_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid version ID: {e}"))
        })?;
        let agent: librefang_types::agent::AgentId = agent_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid agent ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.set_active_version(id, agent).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to set active version: {e}"))
        })
    }

    fn list_experiments(
        &self,
        agent_id: librefang_types::agent::AgentId,
    ) -> Result<Vec<librefang_types::agent::PromptExperiment>, kernel_handle::KernelOpError> {
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.list_experiments(agent_id).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to list experiments: {e}"))
        })
    }

    fn create_experiment(
        &self,
        experiment: &librefang_types::agent::PromptExperiment,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or(kernel_handle::KernelOpError::unavailable("Prompt store"))?;
        // Clone here — the store owns the value. See #3553.
        store.create_experiment(experiment.clone()).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to create experiment: {e}"))
        })
    }

    fn get_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptExperiment>, kernel_handle::KernelOpError>
    {
        let id: uuid::Uuid = experiment_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid experiment ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.get_experiment(id).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to get experiment: {e}"))
        })
    }

    fn update_experiment_status(
        &self,
        experiment_id: &str,
        status: librefang_types::agent::ExperimentStatus,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let id: uuid::Uuid = experiment_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid experiment ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.update_experiment_status(id, status).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!(
                "Failed to update experiment status: {e}"
            ))
        })?;

        // When completing an experiment, auto-activate the winning variant's prompt version
        if status == librefang_types::agent::ExperimentStatus::Completed {
            let metrics = store.get_experiment_metrics(id).map_err(|e| {
                kernel_handle::KernelOpError::Internal(format!(
                    "Failed to get experiment metrics: {e}"
                ))
            })?;
            if let Some(winner) = metrics.iter().max_by(|a, b| {
                a.success_rate
                    .partial_cmp(&b.success_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                if let Some(exp) = store.get_experiment(id).map_err(|e| {
                    kernel_handle::KernelOpError::Internal(format!("Failed to get experiment: {e}"))
                })? {
                    if let Some(variant) = exp.variants.iter().find(|v| v.id == winner.variant_id) {
                        let _ = store.set_active_version(variant.prompt_version_id, exp.agent_id);
                        tracing::info!(
                            experiment_id = %id,
                            winner_variant = %winner.variant_name,
                            success_rate = winner.success_rate,
                            "Auto-activated winning variant's prompt version"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    fn get_experiment_metrics(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<librefang_types::agent::ExperimentVariantMetrics>, kernel_handle::KernelOpError>
    {
        let id: uuid::Uuid = experiment_id.parse().map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Invalid experiment ID: {e}"))
        })?;
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or("Prompt store not initialized")?;
        store.get_experiment_metrics(id).map_err(|e| {
            kernel_handle::KernelOpError::Internal(format!("Failed to get experiment metrics: {e}"))
        })
    }

    fn auto_track_prompt_version(
        &self,
        agent_id: librefang_types::agent::AgentId,
        system_prompt: &str,
    ) -> Result<(), kernel_handle::KernelOpError> {
        let cfg = self.config.load();
        if !cfg.prompt_intelligence.enabled {
            return Ok(());
        }
        let store = self
            .memory
            .prompt_store
            .get()
            .ok_or(kernel_handle::KernelOpError::unavailable("Prompt store"))?;
        match store.create_version_if_changed(agent_id, system_prompt, "auto") {
            Ok(true) => {
                tracing::debug!(agent_id = %agent_id, "Auto-tracked new prompt version");
                // Prune old versions
                let max = cfg.prompt_intelligence.max_versions_per_agent;
                let _ = store.prune_old_versions(agent_id, max);
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(e) => Err(kernel_handle::KernelOpError::Internal(format!(
                "Failed to auto-track prompt version: {e}"
            ))),
        }
    }
}
