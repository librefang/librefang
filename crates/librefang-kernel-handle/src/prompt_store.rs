use super::*;

// ============================================================================
// 11. PromptStore — prompt versions + experiment metadata + auto-tracking
// ============================================================================

pub trait PromptStore: Send + Sync {
    /// Get the running experiment for an agent (if any). Default: None.
    fn get_running_experiment(
        &self,
        _agent_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptExperiment>, KernelOpError> {
        Ok(None)
    }

    /// Record metrics for an experiment variant after a request. Default: no-op.
    fn record_experiment_request(
        &self,
        _experiment_id: &str,
        _variant_id: &str,
        _latency_ms: u64,
        _cost_usd: f64,
        _success: bool,
    ) -> Result<(), KernelOpError> {
        Ok(())
    }

    /// Get a prompt version by ID. Default: None.
    fn get_prompt_version(
        &self,
        _version_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptVersion>, KernelOpError> {
        Ok(None)
    }

    /// List all prompt versions for an agent. Default: empty vec.
    fn list_prompt_versions(
        &self,
        _agent_id: librefang_types::agent::AgentId,
    ) -> Result<Vec<librefang_types::agent::PromptVersion>, KernelOpError> {
        Ok(Vec::new())
    }

    /// Create a new prompt version. Default: error.
    ///
    /// Takes `version` by reference; the kernel clones into the
    /// underlying store. Lets API handlers keep a copy for the response
    /// JSON without forcing two clones. See #3553.
    fn create_prompt_version(
        &self,
        _version: &librefang_types::agent::PromptVersion,
    ) -> Result<(), KernelOpError> {
        Err(KernelOpError::unavailable("Prompt store"))
    }

    /// Delete a prompt version. Default: error.
    fn delete_prompt_version(&self, _version_id: &str) -> Result<(), KernelOpError> {
        Err(KernelOpError::unavailable("Prompt store"))
    }

    /// Set a prompt version as active. Default: error.
    fn set_active_prompt_version(
        &self,
        _version_id: &str,
        _agent_id: &str,
    ) -> Result<(), KernelOpError> {
        Err(KernelOpError::unavailable("Prompt store"))
    }

    /// List all experiments for an agent. Default: empty vec.
    fn list_experiments(
        &self,
        _agent_id: librefang_types::agent::AgentId,
    ) -> Result<Vec<librefang_types::agent::PromptExperiment>, KernelOpError> {
        Ok(Vec::new())
    }

    /// Create a new experiment. Default: error.
    ///
    /// Takes `experiment` by reference for the same reason as
    /// [`create_prompt_version`](Self::create_prompt_version). See #3553.
    fn create_experiment(
        &self,
        _experiment: &librefang_types::agent::PromptExperiment,
    ) -> Result<(), KernelOpError> {
        Err(KernelOpError::unavailable("Prompt store"))
    }

    /// Get an experiment by ID. Default: None.
    fn get_experiment(
        &self,
        _experiment_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptExperiment>, KernelOpError> {
        Ok(None)
    }

    /// Update experiment status. Default: error.
    fn update_experiment_status(
        &self,
        _experiment_id: &str,
        _status: librefang_types::agent::ExperimentStatus,
    ) -> Result<(), KernelOpError> {
        Err(KernelOpError::unavailable("Prompt store"))
    }

    /// Get experiment metrics. Default: empty vec.
    fn get_experiment_metrics(
        &self,
        _experiment_id: &str,
    ) -> Result<Vec<librefang_types::agent::ExperimentVariantMetrics>, KernelOpError> {
        Ok(Vec::new())
    }

    /// Auto-track prompt version if the system prompt changed. Default: no-op.
    fn auto_track_prompt_version(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _system_prompt: &str,
    ) -> Result<(), KernelOpError> {
        Ok(())
    }
}
