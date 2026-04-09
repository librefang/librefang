//! Trait abstraction for kernel operations needed by the agent runtime.
//!
//! This trait allows `librefang-runtime` to call back into the kernel for
//! inter-agent operations (spawn, send, list, kill) without creating
//! a circular dependency. The kernel implements this trait and passes
//! it into the agent loop.

use async_trait::async_trait;

/// Agent info returned by list and discovery operations.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub state: String,
    pub model_provider: String,
    pub model_name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub tools: Vec<String>,
}

/// Handle to kernel operations, passed into the agent loop so agents
/// can interact with each other via tools.
#[async_trait]
pub trait KernelHandle: Send + Sync {
    /// Spawn a new agent from a TOML manifest string.
    /// `parent_id` is the UUID string of the spawning agent (for lineage tracking).
    /// Returns (agent_id, agent_name) on success.
    async fn spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), String>;

    /// Send a message to another agent and get the response.
    async fn send_to_agent(&self, agent_id: &str, message: &str) -> Result<String, String>;

    /// List all running agents.
    fn list_agents(&self) -> Vec<AgentInfo>;

    /// Kill an agent by ID.
    fn kill_agent(&self, agent_id: &str) -> Result<(), String>;

    /// Store a value in shared memory (cross-agent accessible).
    /// Shared memory is scoped to the caller's tenant. When `peer_id` is
    /// `Some`, the key is further scoped to that peer so different end users
    /// within the same tenant get isolated namespaces.
    fn memory_store(
        &self,
        caller_agent_id: &str,
        key: &str,
        value: serde_json::Value,
        peer_id: Option<&str>,
    ) -> Result<(), String>;

    /// Recall a value from shared memory.
    /// When `peer_id` is `Some`, only returns values stored under that peer's
    /// namespace within the caller's tenant.
    fn memory_recall(
        &self,
        caller_agent_id: &str,
        key: &str,
        peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String>;

    /// List all keys in shared memory.
    /// When `peer_id` is `Some`, only returns keys within that peer's
    /// namespace within the caller's tenant.
    fn memory_list(
        &self,
        caller_agent_id: &str,
        peer_id: Option<&str>,
    ) -> Result<Vec<String>, String>;

    /// Find agents by query (matches on name substring, tag, or tool name; case-insensitive).
    fn find_agents(&self, query: &str) -> Vec<AgentInfo>;

    /// Post a task to the shared task queue. Returns the task ID.
    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, String>;

    /// Claim the next available task (optionally filtered by assignee). Returns task JSON or None.
    async fn task_claim(&self, agent_id: &str) -> Result<Option<serde_json::Value>, String>;

    /// Mark a task as completed with a result string.
    async fn task_complete(&self, task_id: &str, result: &str) -> Result<(), String>;

    /// List tasks, optionally filtered by status.
    async fn task_list(&self, status: Option<&str>) -> Result<Vec<serde_json::Value>, String>;

    /// Delete a task by ID. Returns true if deleted.
    async fn task_delete(&self, task_id: &str) -> Result<bool, String>;

    /// Retry a task by resetting it to pending. Returns true if reset.
    async fn task_retry(&self, task_id: &str) -> Result<bool, String>;

    /// Publish a custom event that can trigger proactive agents.
    async fn publish_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), String>;

    /// Publish a custom event with an explicit tenant/account scope.
    async fn publish_event_scoped(
        &self,
        event_type: &str,
        payload: serde_json::Value,
        account_id: Option<&str>,
    ) -> Result<(), String> {
        if account_id.is_some() {
            return Err(
                "Tenant-scoped event publishing is not available on this kernel handle."
                    .to_string(),
            );
        }
        self.publish_event(event_type, payload).await
    }

    /// Publish a custom event using the caller agent's tenant/account scope.
    async fn publish_event_for_agent(
        &self,
        caller_agent_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        let _ = caller_agent_id;
        self.publish_event(event_type, payload).await
    }

    /// Add an entity to the knowledge graph.
    async fn knowledge_add_entity(
        &self,
        caller_agent_id: &str,
        entity: librefang_types::memory::Entity,
    ) -> Result<String, String>;

    /// Add a relation to the knowledge graph.
    async fn knowledge_add_relation(
        &self,
        caller_agent_id: &str,
        relation: librefang_types::memory::Relation,
    ) -> Result<String, String>;

    /// Query the knowledge graph with a pattern.
    async fn knowledge_query(
        &self,
        caller_agent_id: &str,
        pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, String>;

    /// Create a cron job for the calling agent.
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        let _ = (agent_id, job_json);
        Err("Cron scheduler not available".to_string())
    }

    /// List cron jobs for the calling agent.
    async fn cron_list(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let _ = agent_id;
        Err("Cron scheduler not available".to_string())
    }

    /// Cancel a cron job owned by the calling agent.
    async fn cron_cancel(&self, agent_id: &str, job_id: &str) -> Result<(), String> {
        let _ = (agent_id, job_id);
        Err("Cron scheduler not available".to_string())
    }

    /// Check if a tool requires approval based on current policy.
    fn requires_approval(&self, tool_name: &str) -> bool {
        let _ = tool_name;
        false
    }

    /// Check if a tool requires approval, taking sender and channel context
    /// into account.  Falls back to `requires_approval()` by default.
    fn requires_approval_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        let _ = (sender_id, channel);
        self.requires_approval(tool_name)
    }

    /// Check whether a tool is hard-denied for the given sender/channel context.
    fn is_tool_denied_with_context(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> bool {
        let _ = (tool_name, sender_id, channel);
        false
    }

    /// Request approval for a tool execution. Blocks until approved/denied/timed out.
    async fn request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
    ) -> Result<librefang_types::approval::ApprovalDecision, String> {
        let _ = (agent_id, tool_name, action_summary);
        Ok(librefang_types::approval::ApprovalDecision::Approved)
    }

    /// Submit a tool for approval without blocking. Returns request UUID immediately.
    async fn submit_tool_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        deferred: librefang_types::tool::DeferredToolExecution,
    ) -> Result<librefang_types::tool::ToolApprovalSubmission, String> {
        let _ = (agent_id, tool_name, action_summary, deferred);
        Err("Approval system not available".to_string())
    }

    /// Resolve an approval request and get the deferred payload.
    async fn resolve_tool_approval(
        &self,
        request_id: uuid::Uuid,
        decision: librefang_types::approval::ApprovalDecision,
        decided_by: Option<String>,
    ) -> Result<
        (
            librefang_types::approval::ApprovalResponse,
            Option<librefang_types::tool::DeferredToolExecution>,
        ),
        String,
    > {
        let _ = (request_id, decision, decided_by);
        Err("Approval system not available".to_string())
    }

    /// Check current status of an approval request.
    fn get_approval_status(
        &self,
        request_id: uuid::Uuid,
    ) -> Result<Option<librefang_types::approval::ApprovalDecision>, String> {
        let _ = request_id;
        Ok(None)
    }

    /// List available Hands and their activation status.
    async fn hand_list(&self) -> Result<Vec<serde_json::Value>, String> {
        Err("Hands system not available".to_string())
    }

    /// Install a Hand from TOML content.
    async fn hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<serde_json::Value, String> {
        let _ = (toml_content, skill_content);
        Err("Hands system not available".to_string())
    }

    /// Activate a Hand — spawns a specialized autonomous agent.
    async fn hand_activate(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let _ = (hand_id, config);
        Err("Hands system not available".to_string())
    }

    /// Check the status and dashboard metrics of an active Hand.
    async fn hand_status(&self, hand_id: &str) -> Result<serde_json::Value, String> {
        let _ = hand_id;
        Err("Hands system not available".to_string())
    }

    /// Deactivate a running Hand and stop its agent.
    async fn hand_deactivate(&self, instance_id: &str) -> Result<(), String> {
        let _ = instance_id;
        Err("Hands system not available".to_string())
    }

    /// List discovered external A2A agents as (name, url) pairs.
    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        vec![]
    }

    /// Get the URL of a discovered external A2A agent by name.
    fn get_a2a_agent_url(&self, name: &str) -> Option<String> {
        let _ = name;
        None
    }

    /// Send a message to a user on a named channel adapter (e.g., "email", "telegram").
    /// When `thread_id` is provided, the message is sent as a thread reply.
    /// Returns a confirmation string on success.
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, message, thread_id);
        Err("Channel send not available".to_string())
    }

    /// Send media content (image/file) to a user on a named channel adapter.
    /// `media_type` is "image" or "file", `media_url` is the URL, `caption` is optional text.
    /// When `thread_id` is provided, the media is sent as a thread reply.
    #[allow(clippy::too_many_arguments)]
    async fn send_channel_media(
        &self,
        channel: &str,
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (
            channel, recipient, media_type, media_url, caption, filename, thread_id,
        );
        Err("Channel media send not available".to_string())
    }

    /// Send a local file (raw bytes) to a user on a named channel adapter.
    /// Used by the `channel_send` tool when `file_path` is provided.
    /// When `thread_id` is provided, the file is sent as a thread reply.
    async fn send_channel_file_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, data, filename, mime_type, thread_id);
        Err("Channel file data send not available".to_string())
    }

    /// Touch the agent's `last_active` timestamp to prevent heartbeat false-positives
    /// during long-running operations (e.g., LLM calls).
    fn touch_heartbeat(&self, agent_id: &str) {
        let _ = agent_id;
    }

    /// Spawn an agent with capability inheritance enforcement.
    /// `parent_caps` are the parent's granted capabilities. The kernel MUST verify
    /// that every capability in the child manifest is covered by `parent_caps`.
    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[librefang_types::capability::Capability],
    ) -> Result<(String, String), String> {
        // Default: delegate to spawn_agent (no enforcement)
        // The kernel MUST override this with real enforcement
        let _ = parent_caps;
        self.spawn_agent(manifest_toml, parent_id).await
    }

    /// Get the running experiment for an agent (if any). Default: None.
    fn get_running_experiment(
        &self,
        _agent_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptExperiment>, String> {
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
    ) -> Result<(), String> {
        Ok(())
    }

    /// Get a prompt version by ID. Default: None.
    fn get_prompt_version(
        &self,
        _version_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptVersion>, String> {
        Ok(None)
    }

    /// List all prompt versions for an agent. Default: empty vec.
    fn list_prompt_versions(
        &self,
        _agent_id: librefang_types::agent::AgentId,
    ) -> Result<Vec<librefang_types::agent::PromptVersion>, String> {
        Ok(Vec::new())
    }

    /// Create a new prompt version. Default: error.
    fn create_prompt_version(
        &self,
        _version: librefang_types::agent::PromptVersion,
    ) -> Result<(), String> {
        Err("Prompt store not available".to_string())
    }

    /// Delete a prompt version. Default: error.
    fn delete_prompt_version(&self, _version_id: &str) -> Result<(), String> {
        Err("Prompt store not available".to_string())
    }

    /// Set a prompt version as active. Default: error.
    fn set_active_prompt_version(&self, _version_id: &str, _agent_id: &str) -> Result<(), String> {
        Err("Prompt store not available".to_string())
    }

    /// List all experiments for an agent. Default: empty vec.
    fn list_experiments(
        &self,
        _agent_id: librefang_types::agent::AgentId,
    ) -> Result<Vec<librefang_types::agent::PromptExperiment>, String> {
        Ok(Vec::new())
    }

    /// Create a new experiment. Default: error.
    fn create_experiment(
        &self,
        _experiment: librefang_types::agent::PromptExperiment,
    ) -> Result<(), String> {
        Err("Prompt store not available".to_string())
    }

    /// Get an experiment by ID. Default: None.
    fn get_experiment(
        &self,
        _experiment_id: &str,
    ) -> Result<Option<librefang_types::agent::PromptExperiment>, String> {
        Ok(None)
    }

    /// Update experiment status. Default: error.
    fn update_experiment_status(
        &self,
        _experiment_id: &str,
        _status: librefang_types::agent::ExperimentStatus,
    ) -> Result<(), String> {
        Err("Prompt store not available".to_string())
    }

    /// Get experiment metrics. Default: empty vec.
    fn get_experiment_metrics(
        &self,
        _experiment_id: &str,
    ) -> Result<Vec<librefang_types::agent::ExperimentVariantMetrics>, String> {
        Ok(Vec::new())
    }

    /// Auto-track prompt version if the system prompt changed. Default: no-op.
    fn auto_track_prompt_version(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _system_prompt: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Tool execution timeout in seconds (from config). Default: 120.
    fn tool_timeout_secs(&self) -> u64 {
        120
    }

    /// Maximum inter-agent call depth (from config). Default: 5.
    fn max_agent_call_depth(&self) -> u32 {
        5
    }

    /// List active goals (pending or in_progress), optionally filtered by agent ID.
    /// Returns a JSON array of goal objects.
    fn goal_list_active(
        &self,
        _caller_agent_id: &str,
        _agent_id: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        Ok(Vec::new())
    }

    /// Update a goal's status and/or progress. Returns the updated goal JSON.
    fn goal_update(
        &self,
        _caller_agent_id: &str,
        _goal_id: &str,
        _status: Option<&str>,
        _progress: Option<u8>,
    ) -> Result<serde_json::Value, String> {
        Err("Goal system not available".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FallbackEventKernel {
        published: Mutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait]
    impl KernelHandle for FallbackEventKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), String> {
            Err("not used".to_string())
        }

        async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
            Err("not used".to_string())
        }

        fn list_agents(&self) -> Vec<AgentInfo> {
            Vec::new()
        }

        fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
            Err("not used".to_string())
        }

        fn memory_store(
            &self,
            _caller_agent_id: &str,
            _key: &str,
            _value: serde_json::Value,
            _peer_id: Option<&str>,
        ) -> Result<(), String> {
            Err("not used".to_string())
        }

        fn memory_recall(
            &self,
            _caller_agent_id: &str,
            _key: &str,
            _peer_id: Option<&str>,
        ) -> Result<Option<serde_json::Value>, String> {
            Err("not used".to_string())
        }

        fn memory_list(
            &self,
            _caller_agent_id: &str,
            _peer_id: Option<&str>,
        ) -> Result<Vec<String>, String> {
            Err("not used".to_string())
        }

        fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
            Vec::new()
        }

        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, String> {
            Err("not used".to_string())
        }

        async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
            Err("not used".to_string())
        }

        async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
            Err("not used".to_string())
        }

        async fn task_list(&self, _status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
            Err("not used".to_string())
        }

        async fn task_delete(&self, _task_id: &str) -> Result<bool, String> {
            Err("not used".to_string())
        }

        async fn task_retry(&self, _task_id: &str) -> Result<bool, String> {
            Err("not used".to_string())
        }

        async fn publish_event(
            &self,
            event_type: &str,
            payload: serde_json::Value,
        ) -> Result<(), String> {
            self.published
                .lock()
                .unwrap()
                .push((event_type.to_string(), payload));
            Ok(())
        }

        async fn knowledge_add_entity(
            &self,
            _caller_agent_id: &str,
            _entity: librefang_types::memory::Entity,
        ) -> Result<String, String> {
            Err("not used".to_string())
        }

        async fn knowledge_add_relation(
            &self,
            _caller_agent_id: &str,
            _relation: librefang_types::memory::Relation,
        ) -> Result<String, String> {
            Err("not used".to_string())
        }

        async fn knowledge_query(
            &self,
            _caller_agent_id: &str,
            _pattern: librefang_types::memory::GraphPattern,
        ) -> Result<Vec<librefang_types::memory::GraphMatch>, String> {
            Err("not used".to_string())
        }
    }

    #[tokio::test]
    async fn test_publish_event_scoped_default_fails_closed_with_account_scope() {
        let handle = FallbackEventKernel {
            published: Mutex::new(Vec::new()),
        };

        let result = handle
            .publish_event_scoped(
                "tenant.alert",
                serde_json::json!({"ok": true}),
                Some("tenant-a"),
            )
            .await;

        assert_eq!(
            result.unwrap_err(),
            "Tenant-scoped event publishing is not available on this kernel handle."
        );
        assert!(handle.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_publish_event_scoped_default_falls_back_without_account_scope() {
        let handle = FallbackEventKernel {
            published: Mutex::new(Vec::new()),
        };

        handle
            .publish_event_scoped("tenant.alert", serde_json::json!({"ok": true}), None)
            .await
            .unwrap();

        assert_eq!(
            handle.published.lock().unwrap().clone(),
            vec![("tenant.alert".to_string(), serde_json::json!({"ok": true}))]
        );
    }
}
