//! Role traits for kernel operations needed by the agent runtime.
//!
//! Historically this crate exposed a single 50+ method `KernelHandle`
//! god-trait (issue #3746). It is now split into role traits — `AgentControl`,
//! `MemoryAccess`, `TaskQueue`, `EventBus`, `KnowledgeGraph`, `CronControl`,
//! `ApprovalGate`, `HandsControl`, `A2ARegistry`, `ChannelSender`,
//! `PromptStore`, `WorkflowRunner`, `GoalControl`, `ToolPolicy` — so that
//!
//! 1. the trait file no longer mixes 14 unrelated domains in one place,
//! 2. callers can express narrower bounds (e.g. `T: ApprovalGate`) instead of
//!    pulling the whole kernel surface in,
//! 3. test stubs/mocks group their fakes by capability and a missing
//!    capability is a compile error in the role-trait impl, not a silent
//!    `Err("not available")` at first runtime call.
//!
//! `KernelHandle` is preserved as a *supertrait alias* requiring all role
//! traits, with a blanket impl, so existing `Arc<dyn KernelHandle>` call
//! sites (117 of them at split time) keep working unchanged. Future PRs can
//! narrow individual sites without further churn here.
//!
//! ### Default impls
//!
//! Defaults that hide a missing capability behind a runtime
//! `Err("X not available")` are preserved as-is for now to keep this PR a
//! pure structural refactor (zero behavior change). They are gathered onto
//! the role trait that owns them, so a follow-up PR can tighten each role's
//! contract independently rather than having to land 30+ default removals
//! atomically.

use async_trait::async_trait;

// ============================================================================
// Typed kernel-op errors (#3541)
// ============================================================================
//
// `KernelOpError` is a re-export of `librefang_types::error::LibreFangError`
// — the canonical structured business-error enum that already existed in
// the workspace before this migration. The trait surface uses the alias
// for two reasons:
//
//   1. Callers that crossed the runtime↔kernel seam used to get
//      `Result<_, String>`, throwing away the variant info and forcing
//      substring-matching back to a category. The alias resolves that
//      directly: `match err { LibreFangError::AgentNotFound(_) => 404,
//      CapabilityDenied(_) => 403, Unavailable(_) => 503, … }`.
//   2. Reusing the existing enum (rather than introducing a parallel
//      "kernel handle error") keeps every layer (runtime, kernel, api)
//      working with the same vocabulary, so converting between layers is
//      a no-op rather than a `match`-and-rewrap dance.
//
// Use [`KernelResult<T>`] in new role-trait method signatures so the
// shape `Result<T, LibreFangError>` is consistent and self-documenting.
pub use librefang_types::error::LibreFangError as KernelOpError;

/// Canonical result type for `KernelHandle` role-trait methods (#3541).
/// Use this in new method signatures rather than respelling
/// `Result<T, KernelOpError>` each time.
pub type KernelResult<T> = Result<T, KernelOpError>;

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

// ============================================================================
// 1. AgentControl — agent lifecycle, inter-agent send, listing, heartbeats,
//    forked one-shot calls, plus a couple of agent-scoped config queries
//    (`max_agent_call_depth`, `fire_agent_step`).
// ============================================================================

#[async_trait]
pub trait AgentControl: Send + Sync {
    /// Spawn a new agent from a TOML manifest string.
    /// `parent_id` is the UUID string of the spawning agent (for lineage tracking).
    /// Returns (agent_id, agent_name) on success.
    async fn spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), KernelOpError>;

    /// Spawn an agent with capability inheritance enforcement.
    /// `parent_caps` are the parent's granted capabilities. The kernel MUST verify
    /// that every capability in the child manifest is covered by `parent_caps`.
    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[librefang_types::capability::Capability],
    ) -> Result<(String, String), KernelOpError> {
        // Default: delegate to spawn_agent (no enforcement)
        // The kernel MUST override this with real enforcement
        let _ = parent_caps;
        self.spawn_agent(manifest_toml, parent_id).await
    }

    /// Send a message to another agent and get the response.
    async fn send_to_agent(&self, agent_id: &str, message: &str) -> Result<String, KernelOpError>;

    /// Like [`send_to_agent`](Self::send_to_agent), but records that the
    /// call was made on behalf of `parent_agent_id`, so a `/stop` issued to
    /// the parent cascades into the callee's loop (issue #3044). Defaults
    /// to the plain `send_to_agent` behavior for implementations that
    /// don't support cancel cascading — a trace log flags the fallthrough
    /// so operators can tell a non-standard handle is in play.
    async fn send_to_agent_as(
        &self,
        agent_id: &str,
        message: &str,
        parent_agent_id: &str,
    ) -> Result<String, KernelOpError> {
        tracing::trace!(
            agent = %agent_id,
            parent = %parent_agent_id,
            "send_to_agent_as: default impl — cancel cascade not supported by this handle"
        );
        self.send_to_agent(agent_id, message).await
    }

    /// List all running agents.
    fn list_agents(&self) -> Vec<AgentInfo>;

    /// Kill an agent by ID.
    fn kill_agent(&self, agent_id: &str) -> Result<(), KernelOpError>;

    /// Find agents by query (matches on name substring, tag, or tool name; case-insensitive).
    fn find_agents(&self, query: &str) -> Vec<AgentInfo>;

    /// Touch the agent's `last_active` timestamp to prevent heartbeat false-positives
    /// during long-running operations (e.g., LLM calls).
    fn touch_heartbeat(&self, agent_id: &str) {
        let _ = agent_id;
    }

    /// Fire an `agent:step` external hook event.
    /// Called by the runtime at the start of each agent loop iteration.
    fn fire_agent_step(&self, _agent_id: &str, _step: u32) {}

    /// Run a forked agent turn that collapses to a single text response —
    /// the "structured-output via forked call" primitive. Used by the
    /// proactive memory extractor so its LLM call shares the parent
    /// turn's `(system + tools + messages)` prefix for Anthropic prompt
    /// cache alignment, instead of issuing a standalone `driver.complete()`
    /// that always starts cold.
    ///
    /// Internally: spawn `run_forked_agent_streaming`, drain to completion,
    /// return the final assistant text. Fork semantics apply — the call's
    /// messages do NOT persist into the agent's canonical session, and the
    /// turn-end hook fires with `is_fork: true` so auto-dream won't
    /// recurse.
    ///
    /// `allowed_tools = Some(vec![])` keeps the fork single-turn (no tool
    /// calls permitted — model returns text). Pass a larger allowlist only
    /// when the caller actually expects tool use (e.g. future extractors
    /// that want the fork to call `memory_store` directly).
    ///
    /// Default: error. The real kernel overrides; tests / stubs that
    /// don't implement the full streaming path just fall back to a
    /// standalone driver call through the extractor's own path.
    async fn run_forked_agent_oneshot(
        &self,
        _agent_id: &str,
        _prompt: &str,
        _allowed_tools: Option<Vec<String>>,
    ) -> Result<String, KernelOpError> {
        Err(KernelOpError::unavailable("run_forked_agent_oneshot"))
    }

    /// Maximum inter-agent call depth (from config). Default: 5.
    fn max_agent_call_depth(&self) -> u32 {
        5
    }
}

// ============================================================================
// 2. MemoryAccess — shared cross-agent memory + per-user RBAC ACL resolution
// ============================================================================

pub trait MemoryAccess: Send + Sync {
    /// Store a value in shared memory (cross-agent accessible).
    /// When `peer_id` is `Some`, the key is scoped to that peer so different
    /// users of the same agent get isolated memory namespaces.
    fn memory_store(
        &self,
        key: &str,
        value: serde_json::Value,
        peer_id: Option<&str>,
    ) -> Result<(), KernelOpError>;

    /// Recall a value from shared memory.
    /// When `peer_id` is `Some`, only returns values stored under that peer's namespace.
    fn memory_recall(
        &self,
        key: &str,
        peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, KernelOpError>;

    /// List all keys in shared memory.
    /// When `peer_id` is `Some`, only returns keys within that peer's namespace.
    fn memory_list(&self, peer_id: Option<&str>) -> Result<Vec<String>, KernelOpError>;

    /// Resolve the per-user memory ACL for the given sender + channel
    /// pair (RBAC M3, #3054 Phase 2). Returns the resolved
    /// `UserMemoryAccess` so the runtime can build a
    /// `MemoryNamespaceGuard` and gate proactive-memory reads.
    ///
    /// `None` means RBAC is disabled (no registered users) or the sender
    /// could not be attributed to any registered user — callers should
    /// treat this as "no per-user restriction" so the existing single-user
    /// behaviour is preserved.
    ///
    /// Default impl returns `None` so embedders / stubs that haven't
    /// wired RBAC keep the pre-M3 behaviour.
    fn memory_acl_for_sender(
        &self,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> Option<librefang_types::user_policy::UserMemoryAccess> {
        let _ = (sender_id, channel);
        None
    }
}

// ============================================================================
// 3. TaskQueue — shared task queue: post / claim / complete / list / etc.
// ============================================================================

#[async_trait]
pub trait TaskQueue: Send + Sync {
    /// Post a task to the shared task queue. Returns the task ID.
    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, KernelOpError>;

    /// Claim the next available task (optionally filtered by assignee). Returns task JSON or None.
    async fn task_claim(&self, agent_id: &str) -> Result<Option<serde_json::Value>, KernelOpError>;

    /// Mark a task as completed with a result string. `agent_id` identifies the completer.
    async fn task_complete(
        &self,
        agent_id: &str,
        task_id: &str,
        result: &str,
    ) -> Result<(), KernelOpError>;

    /// List tasks, optionally filtered by status.
    async fn task_list(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, KernelOpError>;

    /// Delete a task by ID. Returns true if deleted.
    async fn task_delete(&self, task_id: &str) -> Result<bool, KernelOpError>;

    /// Retry a task by resetting it to pending. Returns true if reset.
    async fn task_retry(&self, task_id: &str) -> Result<bool, KernelOpError>;

    /// Get a single task by ID including its result and retry_count.
    async fn task_get(&self, task_id: &str) -> Result<Option<serde_json::Value>, KernelOpError>;

    /// Update a task's status to `pending` (reset) or `cancelled`.
    /// Returns true if the task was found and updated.
    async fn task_update_status(
        &self,
        task_id: &str,
        new_status: &str,
    ) -> Result<bool, KernelOpError>;
}

// ============================================================================
// 4. EventBus — fire-and-forget custom events for proactive triggers
// ============================================================================

#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish a custom event that can trigger proactive agents.
    async fn publish_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), KernelOpError>;
}

// ============================================================================
// 5. KnowledgeGraph — entity/relation insert + pattern query
// ============================================================================

#[async_trait]
pub trait KnowledgeGraph: Send + Sync {
    /// Add an entity to the knowledge graph.
    ///
    /// Takes `entity` by reference so callers that already hold an owned
    /// value (e.g. proactive memory extractors that may retry the call)
    /// avoid forced moves and downstream `.clone()` chains. The kernel
    /// implementation clones into the underlying store when it actually
    /// needs ownership; total clone count is unchanged but the choice
    /// moves from caller to callee. See issue #3553.
    async fn knowledge_add_entity(
        &self,
        entity: &librefang_types::memory::Entity,
    ) -> Result<String, KernelOpError>;

    /// Add a relation to the knowledge graph.
    ///
    /// Takes `relation` by reference for the same reason as
    /// [`knowledge_add_entity`](Self::knowledge_add_entity). See #3553.
    async fn knowledge_add_relation(
        &self,
        relation: &librefang_types::memory::Relation,
    ) -> Result<String, KernelOpError>;

    /// Query the knowledge graph with a pattern.
    async fn knowledge_query(
        &self,
        pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, KernelOpError>;
}

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

// ============================================================================
// 7. ApprovalGate — approval policy queries + pending-approval lifecycle
// ============================================================================

#[async_trait]
pub trait ApprovalGate: Send + Sync {
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

    /// Resolve the per-user RBAC gate for a tool invocation (RBAC M3,
    /// issue #3054 Phase 2).
    ///
    /// Combines the user's `UserToolPolicy`, `channel_tool_rules`,
    /// `tool_categories`, and role-based approval escalation into a single
    /// runtime-facing verdict. Returns:
    ///
    /// * `Allow` — no per-user objection; continue with the existing
    ///   approval/capability gates.
    /// * `Deny` — hard deny; the dispatcher refuses without prompting.
    /// * `NeedsApproval` — user's own role would block, but a higher role
    ///   could authorise; route through the approval queue.
    ///
    /// Default impl returns `Allow` so installations without a real
    /// kernel (test stubs, embedded callers without an `AuthManager`)
    /// keep their pre-M3 behaviour. The real kernel always overrides
    /// this; flipping the default to `NeedsApproval` was discussed
    /// during PR #3205 review but rejected because it broke ~8 unrelated
    /// runtime tests that rely on the default mock — the loudness gain
    /// is not worth a fragile contract for stub kernels.
    fn resolve_user_tool_decision(
        &self,
        tool_name: &str,
        sender_id: Option<&str>,
        channel: Option<&str>,
    ) -> librefang_types::user_policy::UserToolGate {
        let _ = (tool_name, sender_id, channel);
        librefang_types::user_policy::UserToolGate::Allow
    }

    /// Request approval for a tool execution. Blocks until approved/denied/timed out.
    async fn request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        session_id: Option<&str>,
    ) -> Result<librefang_types::approval::ApprovalDecision, KernelOpError> {
        let _ = (agent_id, tool_name, action_summary, session_id);
        Ok(librefang_types::approval::ApprovalDecision::Approved)
    }

    /// Submit a tool for approval without blocking. Returns request UUID immediately.
    async fn submit_tool_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
        deferred: librefang_types::tool::DeferredToolExecution,
        session_id: Option<&str>,
    ) -> Result<librefang_types::tool::ToolApprovalSubmission, KernelOpError> {
        let _ = (agent_id, tool_name, action_summary, deferred, session_id);
        Err(KernelOpError::unavailable("Approval system"))
    }

    /// Resolve an approval request and get the deferred payload.
    async fn resolve_tool_approval(
        &self,
        request_id: uuid::Uuid,
        decision: librefang_types::approval::ApprovalDecision,
        decided_by: Option<String>,
        totp_verified: bool,
        user_id: Option<&str>,
    ) -> Result<
        (
            librefang_types::approval::ApprovalResponse,
            Option<librefang_types::tool::DeferredToolExecution>,
        ),
        KernelOpError,
    > {
        let _ = (request_id, decision, decided_by, totp_verified, user_id);
        Err(KernelOpError::unavailable("Approval system"))
    }

    /// Check current status of an approval request.
    fn get_approval_status(
        &self,
        request_id: uuid::Uuid,
    ) -> Result<Option<librefang_types::approval::ApprovalDecision>, KernelOpError> {
        let _ = request_id;
        Ok(None)
    }
}

// ============================================================================
// 8. HandsControl — Hand (specialized agent) lifecycle
// ============================================================================

#[async_trait]
pub trait HandsControl: Send + Sync {
    /// List available Hands and their activation status.
    async fn hand_list(&self) -> Result<Vec<serde_json::Value>, KernelOpError> {
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Install a Hand from TOML content.
    async fn hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<serde_json::Value, KernelOpError> {
        let _ = (toml_content, skill_content);
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Activate a Hand — spawns a specialized autonomous agent.
    async fn hand_activate(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, KernelOpError> {
        let _ = (hand_id, config);
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Check the status and dashboard metrics of an active Hand.
    async fn hand_status(&self, hand_id: &str) -> Result<serde_json::Value, KernelOpError> {
        let _ = hand_id;
        Err(KernelOpError::unavailable("Hands system"))
    }

    /// Deactivate a running Hand and stop its agent.
    async fn hand_deactivate(&self, instance_id: &str) -> Result<(), KernelOpError> {
        let _ = instance_id;
        Err(KernelOpError::unavailable("Hands system"))
    }
}

// ============================================================================
// 9. A2ARegistry — discovered external A2A agents (read-only directory)
// ============================================================================

pub trait A2ARegistry: Send + Sync {
    /// List discovered external A2A agents as (name, url) pairs.
    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        vec![]
    }

    /// Get the URL of a discovered external A2A agent by name.
    fn get_a2a_agent_url(&self, name: &str) -> Option<String> {
        let _ = name;
        None
    }
}

// ============================================================================
// 10. ChannelSender — outbound channel adapters (text / media / file / poll)
// ============================================================================

#[async_trait]
pub trait ChannelSender: Send + Sync {
    /// Send a message to a user on a named channel adapter (e.g., "email", "telegram").
    /// When `thread_id` is provided, the message is sent as a thread reply.
    /// When `account_id` is provided, routes through the specific configured bot with that ID.
    /// Returns a confirmation string on success.
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<String, KernelOpError> {
        let _ = (channel, recipient, message, thread_id, account_id);
        Err(KernelOpError::unavailable("Channel send"))
    }

    /// Send media content (image/file) to a user on a named channel adapter.
    /// `media_type` is "image" or "file", `media_url` is the URL, `caption` is optional text.
    /// When `thread_id` is provided, the media is sent as a thread reply.
    /// When `account_id` is provided, routes through the specific configured bot with that ID.
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
        account_id: Option<&str>,
    ) -> Result<String, KernelOpError> {
        let _ = (
            channel, recipient, media_type, media_url, caption, filename, thread_id, account_id,
        );
        Err(KernelOpError::unavailable("Channel media send"))
    }

    /// Send a local file (raw bytes) to a user on a named channel adapter.
    /// Used by the `channel_send` tool when `file_path` is provided.
    /// When `thread_id` is provided, the file is sent as a thread reply.
    /// When `account_id` is provided, routes through the specific configured bot with that ID.
    ///
    /// `data` is a `bytes::Bytes` so wrapping layers (metering, retry,
    /// fan-out to multiple adapters) can `clone()` it for free instead
    /// of cloning the underlying buffer. With the 10 MiB upload bump
    /// (#3514) this avoids per-send buffer copies in every wrapping
    /// layer. See issue #3553.
    #[allow(clippy::too_many_arguments)]
    async fn send_channel_file_data(
        &self,
        channel: &str,
        recipient: &str,
        data: bytes::Bytes,
        filename: &str,
        mime_type: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<String, KernelOpError> {
        let _ = (
            channel, recipient, data, filename, mime_type, thread_id, account_id,
        );
        Err(KernelOpError::unavailable("Channel file data send"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_channel_poll(
        &self,
        channel: &str,
        recipient: &str,
        question: &str,
        options: &[String],
        is_quiz: bool,
        correct_option_id: Option<u8>,
        explanation: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<(), KernelOpError> {
        let _ = (
            channel,
            recipient,
            question,
            options,
            is_quiz,
            correct_option_id,
            explanation,
            account_id,
        );
        Err(KernelOpError::unavailable("Channel poll send"))
    }

    /// Upsert a group roster member (channel bridge → persistent storage).
    fn roster_upsert(
        &self,
        _channel: &str,
        _chat_id: &str,
        _user_id: &str,
        _display_name: &str,
        _username: Option<&str>,
    ) -> Result<(), KernelOpError> {
        Ok(())
    }

    /// List group roster members for a (channel, chat_id) pair.
    fn roster_members(
        &self,
        _channel: &str,
        _chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, KernelOpError> {
        Ok(Vec::new())
    }

    /// Remove a member from the group roster.
    fn roster_remove_member(
        &self,
        _channel: &str,
        _chat_id: &str,
        _user_id: &str,
    ) -> Result<(), KernelOpError> {
        Ok(())
    }
}

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

// ============================================================================
// 12. WorkflowRunner — declarative workflow execution
// ============================================================================

#[async_trait]
pub trait WorkflowRunner: Send + Sync {
    /// Run a workflow by ID or name. The `workflow_id` can be a UUID string or a
    /// workflow name. The `input` is an arbitrary string (typically JSON-encoded
    /// parameters) passed to the first step. Returns `(run_id, output)` on success.
    async fn run_workflow(
        &self,
        workflow_id: &str,
        input: &str,
    ) -> Result<(String, String), KernelOpError> {
        let _ = (workflow_id, input);
        Err(KernelOpError::unavailable("Workflow engine"))
    }
}

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

// ============================================================================
// 14. ToolPolicy — tool/agent config queries (timeouts, env passthrough,
//     workspace prefixes). Pure read-side surface used by the runtime to
//     parameterize tool execution against operator config.
// ============================================================================

pub trait ToolPolicy: Send + Sync {
    /// Tool execution timeout in seconds (from config). Default: 120.
    fn tool_timeout_secs(&self) -> u64 {
        120
    }

    /// Per-tool timeout override lookup.
    ///
    /// Resolution order:
    /// 1. Exact match in `config.tool_timeouts`
    /// 2. Longest glob match in `config.tool_timeouts` (most specific wins)
    /// 3. Global `config.tool_timeout_secs`
    ///
    /// The default impl delegates to `tool_timeout_secs()` (no per-tool config).
    fn tool_timeout_secs_for(&self, _tool_name: &str) -> u64 {
        self.tool_timeout_secs()
    }

    /// Operator-side gate over skill `env_passthrough` requests, derived from
    /// `[skills]` config. `None` = no operator gate (only the built-in
    /// FORBIDDEN/kernel-reserved hard blocks apply). Default impl returns
    /// `None`; the kernel overrides this to pull from `KernelConfig.skills`.
    fn skill_env_passthrough_policy(
        &self,
    ) -> Option<librefang_types::config::EnvPassthroughPolicy> {
        None
    }

    /// Return the canonicalized absolute paths of named workspaces declared as `read-only`
    /// for the given agent. Used by file-write tools to enforce workspace access modes.
    /// Default: no read-only prefixes (all writes allowed by the sandbox).
    fn readonly_workspace_prefixes(&self, _agent_id: &str) -> Vec<std::path::PathBuf> {
        vec![]
    }

    /// Return the canonicalized absolute paths of ALL named workspaces declared
    /// for the given agent, paired with their access modes. Used by file-read,
    /// file-list, file-write, and apply-patch tools to widen the sandbox
    /// accept-list to include declared shared workspaces (PR #2958 wired
    /// `[workspaces]` into write-side denial only; this surfaces the full
    /// allowlist to the read-side path resolver).
    ///
    /// Default: no named workspaces — read-side resolution falls back to the
    /// primary workspace root only.
    fn named_workspace_prefixes(
        &self,
        _agent_id: &str,
    ) -> Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)> {
        Vec::new()
    }

    /// Return the effective directory channel bridges write downloaded
    /// attachments to, when configured. The runtime widens the `file_read` /
    /// `file_list` sandbox accept-list with this prefix so agents can open
    /// the files the bridge hands them via paths like
    /// `/tmp/librefang_uploads/<uuid>.<ext>` (issue #4434).
    ///
    /// Returns `None` for stub kernels without channels wired; the runtime
    /// then falls back to workspace-only resolution.
    fn channel_file_download_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Return the effective directory for storing runtime-generated uploads
    /// (image_generate, browser_screenshot, etc.). Honors operator-configured
    /// `[channels].file_download_dir` when set, otherwise falls back to the
    /// legacy `<temp>/librefang_uploads`. See issue #4435.
    fn effective_upload_dir(&self) -> std::path::PathBuf {
        std::env::temp_dir().join("librefang_uploads")
    }
}

// ============================================================================
// 15. ApiAuth — raw auth-config values needed by the HTTP server layer to
//     build middleware token tables and bind-safety checks at startup.
//
//     Deliberately returns *raw* (unresolved) config strings so the API
//     server can apply its own credential-resolution logic (env-var override,
//     vault: prefix, literal) without pulling KernelConfig into that layer.
// ============================================================================

/// A snapshot of the user-config values needed for API-key table construction.
#[derive(Debug, Clone, Default)]
pub struct ApiUserConfigSnapshot {
    pub name: String,
    pub role: String,
    pub api_key_hash: Option<String>,
}

/// Raw dashboard credential strings from config (before env-var / vault
/// resolution). The HTTP server resolves them with `LIBREFANG_DASHBOARD_USER`,
/// `LIBREFANG_DASHBOARD_PASS`, and the `vault:KEY` prefix logic.
#[derive(Debug, Clone, Default)]
pub struct DashboardRawConfig {
    pub user: String,
    pub pass: String,
    pub pass_hash: String,
}

/// One-shot snapshot of every auth-relevant config field. Returned by
/// [`ApiAuth::auth_snapshot`] from a single `config.load()` so all fields
/// observe the same hot-reload generation — preventing per-request
/// middleware (`valid_api_tokens`, `paired_device_user_keys`) from mixing
/// pre-reload and post-reload config when a reload races with the request.
#[derive(Debug, Clone, Default)]
pub struct ApiAuthSnapshot {
    /// Raw `api_key` value from config (may be empty when auth is open).
    pub api_key: String,
    /// Raw dashboard credential strings (before env-var / vault resolution).
    pub dashboard: DashboardRawConfig,
    /// Absolute path to the daemon home directory (owned so the snapshot
    /// is fully self-contained and not tied to the kernel's lifetime).
    pub home_dir: std::path::PathBuf,
    /// Paired-device (mobile) API key hashes: `(device_id, api_key_hash)`.
    pub device_api_keys: Vec<(String, String)>,
    /// Per-user config entries used to build the user API-key table.
    pub config_users: Vec<ApiUserConfigSnapshot>,
}

pub trait ApiAuth: Send + Sync {
    /// Atomic snapshot of every auth-relevant config field. Implementations
    /// MUST acquire all values from a single config snapshot so callers see
    /// a consistent view across hot-reload boundaries.
    fn auth_snapshot(&self) -> ApiAuthSnapshot;
}

// ============================================================================
// 16. SessionWriter — pre-inject content blocks into an agent session before
//     an LLM turn, used by the HTTP attachment upload path (#3744).
//
//     Abstracts over `agent_registry()` + `memory_substrate()` so callers
//     in librefang-api do not need to import the concrete kernel type.
// ============================================================================

pub trait SessionWriter: Send + Sync {
    /// Pre-insert `blocks` as a User-role message into the agent's current
    /// session so the LLM sees the content in the next turn.  No-op (with a
    /// `warn!`) when the agent is not found; best-effort on save failure.
    ///
    /// **Blocking I/O notice.**  The current production implementation
    /// (`LibreFangKernel`) calls `MemorySubstrate::save_session` synchronously,
    /// which blocks on a SQLite write.  Callers running inside an async
    /// runtime should wrap the call in `tokio::task::spawn_blocking` to
    /// avoid stalling worker threads under contention. (#3579 will move the
    /// substrate to `tokio::fs`-aware async; once that lands, the trait
    /// itself can become `async fn` and this caveat goes away.)
    fn inject_attachment_blocks(
        &self,
        agent_id: librefang_types::agent::AgentId,
        blocks: Vec<librefang_types::message::ContentBlock>,
    );
}

// ============================================================================
// KernelHandle — supertrait alias of all 16 role traits.
//
// Existing call sites take `Arc<dyn KernelHandle>`; that keeps working because
// any type that impls every role trait automatically gets `KernelHandle` via
// the blanket impl below. To narrow a new call site, take only the role bounds
// you need (e.g. `fn foo<T: ApprovalGate + Send + Sync>(h: &T)`).
// ============================================================================

pub trait KernelHandle:
    AgentControl
    + MemoryAccess
    + TaskQueue
    + EventBus
    + KnowledgeGraph
    + CronControl
    + ApprovalGate
    + HandsControl
    + A2ARegistry
    + ChannelSender
    + PromptStore
    + WorkflowRunner
    + GoalControl
    + ToolPolicy
    + ApiAuth
    + SessionWriter
    + Send
    + Sync
{
}

impl<T> KernelHandle for T where
    T: AgentControl
        + MemoryAccess
        + TaskQueue
        + EventBus
        + KnowledgeGraph
        + CronControl
        + ApprovalGate
        + HandsControl
        + A2ARegistry
        + ChannelSender
        + PromptStore
        + WorkflowRunner
        + GoalControl
        + ToolPolicy
        + ApiAuth
        + SessionWriter
        + Send
        + Sync
        + ?Sized
{
}

/// Prelude — glob-import this to bring `KernelHandle` plus every role trait
/// into scope so that method calls like `kernel.send_channel_message(...)`
/// resolve. Replaces the pre-#3746 single-trait import pattern.
pub mod prelude {
    pub use super::{
        A2ARegistry, AgentControl, AgentInfo, ApiAuth, ApiAuthSnapshot, ApiUserConfigSnapshot,
        ApprovalGate, ChannelSender, CronControl, DashboardRawConfig, EventBus, GoalControl,
        HandsControl, KernelHandle, KnowledgeGraph, MemoryAccess, PromptStore, SessionWriter,
        TaskQueue, ToolPolicy, WorkflowRunner,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Compile-only stub that implements every role trait, used to prove that:
    ///   1. `KernelHandle` is reachable purely via the blanket impl,
    ///   2. `Arc<dyn KernelHandle>` can be constructed from such a type,
    ///   3. each role trait is individually object-safe.
    struct StubKernel;

    #[async_trait]
    impl AgentControl for StubKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), super::KernelOpError> {
            Err("stub".into())
        }
        async fn send_to_agent(
            &self,
            _agent_id: &str,
            _message: &str,
        ) -> Result<String, super::KernelOpError> {
            Err("stub".into())
        }
        fn list_agents(&self) -> Vec<AgentInfo> {
            vec![]
        }
        fn kill_agent(&self, _agent_id: &str) -> Result<(), super::KernelOpError> {
            Err("stub".into())
        }
        fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
            vec![]
        }
    }

    impl MemoryAccess for StubKernel {
        fn memory_store(
            &self,
            _key: &str,
            _value: serde_json::Value,
            _peer_id: Option<&str>,
        ) -> Result<(), super::KernelOpError> {
            Err("stub".into())
        }
        fn memory_recall(
            &self,
            _key: &str,
            _peer_id: Option<&str>,
        ) -> Result<Option<serde_json::Value>, super::KernelOpError> {
            Ok(None)
        }
        fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, super::KernelOpError> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl TaskQueue for StubKernel {
        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, super::KernelOpError> {
            Err("stub".into())
        }
        async fn task_claim(
            &self,
            _agent_id: &str,
        ) -> Result<Option<serde_json::Value>, super::KernelOpError> {
            Ok(None)
        }
        async fn task_complete(
            &self,
            _agent_id: &str,
            _task_id: &str,
            _result: &str,
        ) -> Result<(), super::KernelOpError> {
            Err("stub".into())
        }
        async fn task_list(
            &self,
            _status: Option<&str>,
        ) -> Result<Vec<serde_json::Value>, super::KernelOpError> {
            Ok(vec![])
        }
        async fn task_delete(&self, _task_id: &str) -> Result<bool, super::KernelOpError> {
            Ok(false)
        }
        async fn task_retry(&self, _task_id: &str) -> Result<bool, super::KernelOpError> {
            Ok(false)
        }
        async fn task_get(
            &self,
            _task_id: &str,
        ) -> Result<Option<serde_json::Value>, super::KernelOpError> {
            Ok(None)
        }
        async fn task_update_status(
            &self,
            _task_id: &str,
            _new_status: &str,
        ) -> Result<bool, super::KernelOpError> {
            Ok(false)
        }
    }

    #[async_trait]
    impl EventBus for StubKernel {
        async fn publish_event(
            &self,
            _event_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), super::KernelOpError> {
            Ok(())
        }
    }

    #[async_trait]
    impl KnowledgeGraph for StubKernel {
        async fn knowledge_add_entity(
            &self,
            _entity: &librefang_types::memory::Entity,
        ) -> Result<String, super::KernelOpError> {
            Err("stub".into())
        }
        async fn knowledge_add_relation(
            &self,
            _relation: &librefang_types::memory::Relation,
        ) -> Result<String, super::KernelOpError> {
            Err("stub".into())
        }
        async fn knowledge_query(
            &self,
            _pattern: librefang_types::memory::GraphPattern,
        ) -> Result<Vec<librefang_types::memory::GraphMatch>, super::KernelOpError> {
            Ok(vec![])
        }
    }

    impl CronControl for StubKernel {}
    impl ApprovalGate for StubKernel {}
    impl HandsControl for StubKernel {}
    impl A2ARegistry for StubKernel {}
    impl ChannelSender for StubKernel {}
    impl PromptStore for StubKernel {}
    impl WorkflowRunner for StubKernel {}
    impl GoalControl for StubKernel {}
    impl ToolPolicy for StubKernel {}
    impl ApiAuth for StubKernel {
        fn auth_snapshot(&self) -> ApiAuthSnapshot {
            ApiAuthSnapshot::default()
        }
    }
    impl SessionWriter for StubKernel {
        fn inject_attachment_blocks(
            &self,
            _agent_id: librefang_types::agent::AgentId,
            _blocks: Vec<librefang_types::message::ContentBlock>,
        ) {
        }
    }

    #[test]
    fn stub_satisfies_kernel_handle_via_blanket_impl() {
        fn assert_kernel_handle<T: KernelHandle + ?Sized>(_: &T) {}
        let s = StubKernel;
        assert_kernel_handle(&s);
    }

    #[test]
    fn dyn_kernel_handle_is_object_safe() {
        let _arc: Arc<dyn KernelHandle> = Arc::new(StubKernel);
    }

    #[test]
    fn role_traits_are_individually_object_safe() {
        // If any role trait gained a non-object-safe method (generic, Self by
        // value, etc.), this stops compiling. That's the point.
        let _agent: Arc<dyn AgentControl> = Arc::new(StubKernel);
        let _mem: Arc<dyn MemoryAccess> = Arc::new(StubKernel);
        let _tq: Arc<dyn TaskQueue> = Arc::new(StubKernel);
        let _ev: Arc<dyn EventBus> = Arc::new(StubKernel);
        let _kg: Arc<dyn KnowledgeGraph> = Arc::new(StubKernel);
        let _cron: Arc<dyn CronControl> = Arc::new(StubKernel);
        let _appr: Arc<dyn ApprovalGate> = Arc::new(StubKernel);
        let _hand: Arc<dyn HandsControl> = Arc::new(StubKernel);
        let _a2a: Arc<dyn A2ARegistry> = Arc::new(StubKernel);
        let _ch: Arc<dyn ChannelSender> = Arc::new(StubKernel);
        let _ps: Arc<dyn PromptStore> = Arc::new(StubKernel);
        let _wf: Arc<dyn WorkflowRunner> = Arc::new(StubKernel);
        let _goal: Arc<dyn GoalControl> = Arc::new(StubKernel);
        let _tp: Arc<dyn ToolPolicy> = Arc::new(StubKernel);
        let _auth: Arc<dyn ApiAuth> = Arc::new(StubKernel);
        let _sw: Arc<dyn SessionWriter> = Arc::new(StubKernel);
    }
}
