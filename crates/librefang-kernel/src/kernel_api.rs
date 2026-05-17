//! `KernelApi` — the trait surface consumed by the HTTP API layer (#3566).
//!
//! ## Why this exists
//!
//! `librefang-api` historically held `Arc<LibreFangKernel>` in `AppState`,
//! which means every public inherent method on the kernel struct was
//! automatically part of the HTTP layer's coupling surface. Renaming any
//! kernel internal forced edits in `routes/`, the API layer could not be
//! versioned independently of the kernel, and route tests could not stub
//! the kernel without dragging the whole runtime along.
//!
//! `KernelApi` is the *single explicit contract* between the API and the
//! kernel. `AppState.kernel` is `Arc<dyn KernelApi>`; routes call methods
//! through this trait. The trait is the only place where the API↔kernel
//! coupling is permitted, so widening it is an explicit choice rather than
//! an accidental side-effect of adding a method on `LibreFangKernel`.
//!
//! Distinction from [`crate::kernel_handle`]: that crate exposes
//! kernel-ops needed by the *runtime* (so an agent loop can call back
//! into the kernel without a circular crate dep). `KernelApi` is the
//! analogous trait for the *HTTP layer*. The two trait surfaces overlap
//! conceptually but their scopes diverge — the runtime cares about
//! agent/memory/task primitives, while the API cares about admin /
//! observability surface (audit, config, MCP wiring, hot-reload, …).

// `register_trigger_with_target` and `send_message_with_incognito` mirror
// the kernel-inherent signatures verbatim — both already exceed clippy's
// 7-arg threshold on the kernel side. Splitting the trait method into a
// builder would diverge the trait surface from the inherent surface and
// break the "trait is a thin facade" invariant of #3566. Per-method
// `#[allow(clippy::too_many_arguments)]` is applied at each call site
// rather than as a module-level blanket so unrelated methods that
// accidentally creep over 7 args still trip the lint.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use librefang_types::agent::{
    AgentId, AgentManifest, ResetScope, RunningSessionSnapshot, SessionId,
};
use librefang_types::error::LibreFangResult;

use crate::approval::ApprovalManager;
use crate::audit::AuditLog;
use crate::auth::AuthManager;
use crate::auto_dream::{AbortOutcome, AutoDreamStatus};
use crate::config_reload::ReloadPlan;
use crate::cron::CronScheduler;
use crate::error::{KernelError, KernelResult};
use crate::event_bus::EventBus;
use crate::inbox::InboxStatus;
use crate::kernel_handle::KernelHandle;
use crate::pairing::PairingManager;
use crate::registry::AgentRegistry;
use crate::scheduler::AgentScheduler;
use crate::session_stream_hub::SessionStreamHub;
use crate::supervisor::Supervisor;
use crate::trajectory::TrajectoryBundle;
use crate::triggers::{Trigger, TriggerId, TriggerMatch, TriggerPattern};
use crate::workflow::WorkflowEngine;
use crate::DeliveryTracker;
use crate::LibreFangKernel;

use librefang_kernel_metering::MeteringEngine;
use librefang_memory::MemorySubstrate;
use librefang_types::config::{AgentBinding, BudgetConfig, KernelConfig};
use librefang_types::tool::ToolDefinition;

/// HTTP-API-facing kernel trait.
///
/// `AppState.kernel` is `Arc<dyn KernelApi>`. Routes interact with the
/// kernel exclusively through this trait — there is no `state.kernel.X`
/// path that bypasses it.
///
/// Extends [`KernelHandle`] (the runtime-facing super-trait that bundles
/// every role trait) so `Arc<dyn KernelApi>` can upcast to
/// `Arc<dyn KernelHandle>` (and to any individual role trait) for the
/// runtime-call paths the dashboard uses (e.g. proxying task-board /
/// channel-send / approval-resolve through the same kernel object).
#[async_trait]
pub trait KernelApi: KernelHandle + Send + Sync {
    // ====================================================================
    // Subsystem accessors — return refs/handles to internal subsystems.
    // ====================================================================

    fn agent_registry(&self) -> &AgentRegistry;
    fn agent_identities(&self) -> &Arc<crate::agent_identity_registry::AgentIdentityRegistry>;
    fn approvals(&self) -> &ApprovalManager;
    fn audit(&self) -> &Arc<AuditLog>;
    fn auth_manager(&self) -> &AuthManager;
    fn browser(&self) -> &librefang_runtime::browser::BrowserManager;
    fn cron(&self) -> &CronScheduler;
    fn delivery(&self) -> &DeliveryTracker;
    fn event_bus_ref(&self) -> &EventBus;
    fn hands(&self) -> &librefang_hands::registry::HandRegistry;
    fn home_dir(&self) -> &Path;
    fn media(&self) -> &librefang_runtime::media_understanding::MediaEngine;
    fn media_drivers(&self) -> &librefang_runtime::media::MediaDriverCache;
    fn memory_substrate(&self) -> &Arc<MemorySubstrate>;
    fn metering_ref(&self) -> &Arc<MeteringEngine>;
    fn pairing_ref(&self) -> &PairingManager;
    fn proactive_memory_store(&self) -> Option<&Arc<librefang_memory::ProactiveMemoryStore>>;
    fn processes(&self) -> &Arc<librefang_runtime::process_manager::ProcessManager>;
    fn process_registry(&self) -> &Arc<librefang_runtime::process_registry::ProcessRegistry>;
    fn scheduler_ref(&self) -> &AgentScheduler;
    fn session_stream_hub(&self) -> Arc<SessionStreamHub>;
    fn supervisor_ref(&self) -> &Supervisor;
    fn templates(&self) -> &crate::workflow::WorkflowTemplateRegistry;
    fn tts(&self) -> &librefang_runtime::tts::TtsEngine;
    fn web_tools(&self) -> &librefang_runtime::web_search::WebToolsContext;
    fn workflow_engine(&self) -> &WorkflowEngine;

    // `*_ref` accessors that expose internal state for mutation. These are
    // necessary for routes that need to read or mutate live kernel state
    // (model catalog, MCP wiring, event bus, …).
    fn command_queue_ref(&self) -> &librefang_runtime::command_lane::CommandQueue;
    fn config_ref(&self) -> arc_swap::Guard<Arc<KernelConfig>>;
    fn config_snapshot(&self) -> Arc<KernelConfig>;
    fn context_engine_ref(&self) -> Option<&dyn librefang_runtime::context_engine::ContextEngine>;
    fn default_model_override_ref(
        &self,
    ) -> &std::sync::RwLock<Option<librefang_types::config::DefaultModelConfig>>;
    fn mcp_auth_states_ref(&self) -> &librefang_runtime::mcp_oauth::McpAuthStates;
    fn mcp_connections_ref(
        &self,
    ) -> &tokio::sync::Mutex<Vec<librefang_runtime::mcp::McpConnection>>;
    fn mcp_tools_ref(&self) -> &std::sync::Mutex<Vec<ToolDefinition>>;
    fn model_catalog_ref(
        &self,
    ) -> &arc_swap::ArcSwap<librefang_runtime::model_catalog::ModelCatalog>;
    fn oauth_provider_ref(
        &self,
    ) -> Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider + Send + Sync>;
    fn peer_node_ref(&self) -> Option<&Arc<librefang_wire::PeerNode>>;
    fn peer_registry_ref(&self) -> Option<&librefang_wire::PeerRegistry>;
    fn skill_registry_ref(&self) -> &std::sync::RwLock<librefang_skills::registry::SkillRegistry>;

    // ====================================================================
    // Config / lifecycle
    // ====================================================================

    fn budget_config(&self) -> BudgetConfig;
    /// Mutate the live budget config in-place. The closure is called under
    /// the kernel's internal lock; keep it short and side-effect-free
    /// outside of `BudgetConfig` mutation.
    fn update_budget_config(&self, f: &dyn Fn(&mut BudgetConfig));
    fn shutdown(&self);
    fn clear_driver_cache(&self);
    fn relocate_legacy_agent_dirs(&self);
    fn validate_config_for_reload(&self, config: &KernelConfig) -> Result<(), Vec<String>>;
    async fn reload_config(&self) -> Result<ReloadPlan, String>;

    // ====================================================================
    // Vault — sensitive secret read/write/recovery.
    // ====================================================================

    fn vault_get(&self, key: &str) -> Option<String>;
    fn vault_set(&self, key: &str, value: &str) -> Result<(), String>;
    fn vault_redeem_recovery_code(&self, code: &str) -> Result<bool, String>;

    // ====================================================================
    // MCP install façade — routes through the kernel's cached vault and
    // catalog so HTTP request handlers don't open vault.enc and run the
    // unlock-time Argon2id KDF on every install request (#3598). The trait
    // exposes the high-level installer; the underlying `vault_handle` stays
    // an inherent method to keep the trait surface small.
    // ====================================================================

    fn install_integration(
        &self,
        template_id: &str,
        provided_keys: &std::collections::HashMap<String, String>,
    ) -> librefang_extensions::ExtensionResult<librefang_extensions::installer::InstallResult>;

    // ====================================================================
    // Inbox / auto-dream observability
    // ====================================================================

    fn inbox_status(&self) -> InboxStatus;
    async fn auto_dream_status(&self) -> AutoDreamStatus;
    async fn auto_dream_abort(&self, agent_id: AgentId) -> AbortOutcome;
    fn auto_dream_set_enabled(&self, agent_id: AgentId, enabled: bool) -> LibreFangResult<()>;

    // ====================================================================
    // Agent lifecycle / sessions
    // ====================================================================

    /// Spawn a new agent from an already-parsed [`AgentManifest`]. Distinct
    /// from `AgentControl::spawn_agent` (which takes a TOML string and is
    /// the runtime-side variant) — routes parse the manifest before calling
    /// in so they can return validation errors with line/col info.
    fn spawn_agent_typed(&self, manifest: AgentManifest) -> KernelResult<AgentId>;
    /// Kill an agent by typed [`AgentId`]. Distinct from
    /// `AgentControl::kill_agent` (which takes a `&str`) — routes already
    /// hold a typed id and we don't want to round-trip through string
    /// parsing.
    fn kill_agent_typed(&self, agent_id: AgentId) -> KernelResult<()>;
    fn kill_agent_with_purge(&self, agent_id: AgentId, purge_identity: bool) -> KernelResult<()>;
    fn stop_agent_run(&self, agent_id: AgentId) -> KernelResult<bool>;
    fn stop_session_run(&self, agent_id: AgentId, session_id: SessionId) -> KernelResult<bool>;
    fn suspend_agent(&self, agent_id: AgentId) -> KernelResult<()>;
    fn resume_agent(&self, agent_id: AgentId) -> KernelResult<()>;
    async fn compact_agent_session(&self, agent_id: AgentId) -> KernelResult<String>;
    /// Compact a specific session id; channel `/compact` calls this with the
    /// per-channel session so it doesn't accidentally summarise the agent's
    /// registry-pointer session (#4868).
    async fn compact_agent_session_with_id(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
    ) -> KernelResult<String>;
    /// Reset an agent's session(s). See [`ResetScope`] for the agent-wide vs.
    /// per-session split (#4868). Async because it acquires the same
    /// per-agent / per-session message lock that `send_message_full` holds,
    /// to serialize against in-flight turns.
    async fn reset_session(&self, agent_id: AgentId, scope: ResetScope) -> KernelResult<()>;
    /// Hard-reboot an agent's session(s) — no summary saved. See
    /// [`ResetScope`] for the agent-wide vs. per-session split (#4868).
    async fn reboot_session(&self, agent_id: AgentId, scope: ResetScope) -> KernelResult<()>;
    async fn clear_agent_history(&self, agent_id: AgentId) -> KernelResult<()>;
    fn list_agent_sessions(&self, agent_id: AgentId) -> KernelResult<Vec<serde_json::Value>>;
    fn create_agent_session(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> KernelResult<serde_json::Value>;
    fn switch_agent_session(&self, agent_id: AgentId, session_id: SessionId) -> KernelResult<()>;
    fn export_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<librefang_memory::session::SessionExport>;
    fn import_session(
        &self,
        agent_id: AgentId,
        export: librefang_memory::session::SessionExport,
    ) -> KernelResult<SessionId>;
    fn export_session_trajectory(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<TrajectoryBundle>;
    fn persist_manifest_to_disk(&self, agent_id: AgentId);
    fn reload_agent_from_disk(&self, agent_id: AgentId) -> KernelResult<()>;
    fn update_manifest(&self, agent_id: AgentId, new_manifest: AgentManifest) -> KernelResult<()>;
    fn set_agent_skills(&self, agent_id: AgentId, skills: Vec<String>) -> KernelResult<()>;
    fn set_agent_mcp_servers(&self, agent_id: AgentId, servers: Vec<String>) -> KernelResult<()>;
    /// Update an agent's schedule mode and restart its background loop so
    /// the change takes effect immediately, without a daemon restart.
    /// See [`LibreFangKernel::set_agent_schedule`] for the full contract.
    fn set_agent_schedule(
        self: Arc<Self>,
        agent_id: AgentId,
        schedule: librefang_types::agent::ScheduleMode,
    ) -> KernelResult<()>;
    fn set_agent_tool_filters(
        &self,
        agent_id: AgentId,
        capabilities_tools: Option<Vec<String>>,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> KernelResult<()>;
    fn list_running_sessions(&self, agent_id: AgentId) -> Vec<RunningSessionSnapshot>;
    fn running_session_ids(&self) -> std::collections::HashSet<SessionId>;
    fn verify_signed_manifest(&self, signed_json: &str) -> KernelResult<String>;
    fn available_tools(&self, agent_id: AgentId) -> Arc<Vec<ToolDefinition>>;

    // ====================================================================
    // Messaging
    // ====================================================================

    async fn send_message(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    async fn send_message_ephemeral(
        &self,
        agent_id: AgentId,
        message: &str,
        sender_context: Option<&librefang_channels::types::SenderContext>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    /// Send a message to an agent with an optional per-call `session_mode` override.
    ///
    /// The default implementation delegates to [`KernelApi::send_message`] when
    /// the override is `None`, so test mocks of `KernelApi` keep compiling
    /// without manually implementing this method. When `Some(_)` is requested
    /// the default refuses with `InvalidInput` rather than silently dropping
    /// the override — silent fallback is the bug class this method exists to
    /// prevent. Production impls (`LibreFangKernel`) override and honor the
    /// per-step / per-trigger session_mode resolver.
    async fn send_message_with_session_mode(
        &self,
        agent_id: AgentId,
        message: &str,
        session_mode_override: Option<librefang_types::agent::SessionMode>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        if let Some(mode) = session_mode_override {
            return Err(KernelError::LibreFang(
                librefang_types::error::LibreFangError::InvalidInput(format!(
                    "KernelApi::send_message_with_session_mode: this impl does not honor \
                     session_mode_override={mode:?}; override the method on your KernelApi impl"
                )),
            ));
        }
        self.send_message(agent_id, message).await
    }

    // ====================================================================
    // Hands
    // ====================================================================

    fn activate_hand(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> KernelResult<librefang_hands::HandInstance>;
    fn deactivate_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()>;
    fn pause_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()>;
    fn resume_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()>;
    fn reload_hands(&self) -> (usize, usize);
    fn invalidate_hand_route_cache(&self);
    fn persist_hand_state(&self);
    fn clear_hand_agent_runtime_override(&self, agent_id: AgentId) -> KernelResult<()>;
    fn trigger_all_hands(&self);

    // ====================================================================
    // MCP — connection lifecycle (Arc<Self> receivers because they spawn
    // background tasks that need a strong self-reference).
    // ====================================================================

    fn mcp_health(&self) -> &librefang_extensions::health::HealthMonitor;
    fn mcp_catalog_load(&self) -> arc_swap::Guard<Arc<librefang_extensions::catalog::McpCatalog>>;
    async fn connect_mcp_servers(self: Arc<Self>);
    async fn disconnect_mcp_server(&self, name: &str) -> bool;
    async fn retry_mcp_connection(self: Arc<Self>, server_name: &str);
    async fn reload_mcp_servers(self: Arc<Self>) -> Result<usize, String>;
    async fn reconnect_mcp_server(self: Arc<Self>, id: &str) -> Result<usize, String>;

    // ====================================================================
    // Triggers / workflows / events
    // ====================================================================

    fn list_triggers(&self, agent_id: Option<AgentId>) -> Vec<Trigger>;
    fn get_trigger(&self, trigger_id: TriggerId) -> Option<Trigger>;
    #[allow(clippy::too_many_arguments)]
    fn register_trigger_with_target(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
        target_agent: Option<AgentId>,
        cooldown_secs: Option<u64>,
        session_mode: Option<librefang_types::agent::SessionMode>,
        workflow_id: Option<String>,
    ) -> KernelResult<TriggerId>;
    fn remove_trigger(&self, trigger_id: TriggerId) -> bool;
    fn update_trigger(
        &self,
        trigger_id: TriggerId,
        patch: crate::triggers::TriggerPatch,
    ) -> Option<Trigger>;
    async fn register_workflow(
        &self,
        workflow: crate::workflow::Workflow,
    ) -> crate::workflow::WorkflowId;
    /// Run a workflow by typed id. Distinct from
    /// `WorkflowRunner::run_workflow` (which takes `&str`) — routes hold a
    /// typed `WorkflowId` and want the typed `WorkflowRunId` back.
    async fn run_workflow_typed(
        &self,
        workflow_id: crate::workflow::WorkflowId,
        input: String,
    ) -> KernelResult<(crate::workflow::WorkflowRunId, String)>;
    async fn dry_run_workflow(
        &self,
        workflow_id: crate::workflow::WorkflowId,
        input: String,
    ) -> KernelResult<Vec<crate::workflow::DryRunStep>>;
    /// Publish a typed [`Event`](librefang_types::event::Event) and return
    /// the matched trigger fires. Distinct from `EventBus::publish_event`
    /// (which takes `(event_type: &str, payload: Value)`).
    async fn publish_typed_event(&self, event: librefang_types::event::Event) -> Vec<TriggerMatch>;

    // ====================================================================
    // Agent bindings (channel ↔ agent mapping)
    // ====================================================================

    fn list_bindings(&self) -> Vec<AgentBinding>;
    fn add_binding(&self, binding: AgentBinding);
    fn remove_binding(&self, index: usize) -> Option<AgentBinding>;

    // ====================================================================
    // Skills / driver caches / model catalog
    // ====================================================================

    fn reload_skills(&self);
    fn model_catalog_load(
        &self,
    ) -> arc_swap::Guard<Arc<librefang_runtime::model_catalog::ModelCatalog>>;
    /// Mutate the live model catalog. The closure is invoked under the
    /// kernel's internal lock; if the caller needs the closure's return
    /// value, capture it via `&mut Option<R>` from the surrounding scope.
    fn model_catalog_update(
        &self,
        f: &mut dyn FnMut(&mut librefang_runtime::model_catalog::ModelCatalog),
    );

    // ====================================================================
    // Background spawning
    // ====================================================================

    fn start_background_for_agent(
        self: Arc<Self>,
        agent_id: AgentId,
        name: &str,
        schedule: &librefang_types::agent::ScheduleMode,
    );

    /// Count of currently-running background loops. Exposed for tests that
    /// need to assert schedule changes actually start / stop the loop (the
    /// silent-drop regression from #4984). See
    /// [`LibreFangKernel::background_active_count`].
    fn background_active_count(&self) -> usize;

    // ====================================================================
    // Additional kernel-inherent methods used by API/WS/server.rs.
    //
    // (Methods on the role traits — TaskQueue, PromptStore, ChannelSender,
    // ApprovalGate, CronControl, ApiAuth, EventBus, … — are reachable via
    // the `KernelHandle` super-trait, so they are not duplicated here. Routes
    // that call e.g. `state.kernel.task_get(&id)` resolve through TaskQueue;
    // bring the role traits into scope with
    // `use librefang_kernel::kernel_handle::prelude::*;`.)
    // ====================================================================

    fn agent_has_active_session(&self, agent_id: AgentId) -> bool;
    fn a2a_agents(&self) -> &std::sync::Mutex<Vec<(String, librefang_runtime::a2a::AgentCard)>>;
    fn a2a_tasks(&self) -> &librefang_runtime::a2a::A2aTaskStore;
    fn context_report(
        &self,
        agent_id: AgentId,
    ) -> KernelResult<librefang_runtime::compactor::ContextReport>;
    fn effective_mcp_servers_ref(
        &self,
    ) -> &std::sync::RwLock<Vec<librefang_types::config::McpServerConfigEntry>>;
    fn embedding(
        &self,
    ) -> Option<&Arc<dyn librefang_runtime::embedding::EmbeddingDriver + Send + Sync>>;
    async fn inject_message_for_session(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
        message: &str,
    ) -> KernelResult<bool>;
    fn provider_unconfigured_flag(&self) -> &std::sync::atomic::AtomicBool;
    fn session_usage_cost(&self, agent_id: AgentId) -> KernelResult<(u64, u64, f64)>;
    fn set_agent_model(
        &self,
        agent_id: AgentId,
        model: &str,
        explicit_provider: Option<&str>,
    ) -> KernelResult<()>;
    /// Returns a per-agent partial-failure list `(agent_name, error)`;
    /// empty means every eligible agent migrated cleanly.
    fn sync_default_model_agents(
        &self,
        old_provider: &str,
        dm: &librefang_types::config::DefaultModelConfig,
    ) -> Vec<(String, String)>;
    fn traces(&self) -> &dashmap::DashMap<AgentId, Vec<librefang_types::tool::DecisionTrace>>;
    fn update_hand_agent_runtime_override(
        &self,
        agent_id: AgentId,
        override_config: librefang_hands::HandAgentRuntimeOverride,
    ) -> KernelResult<()>;

    // ====================================================================
    // Streaming + handle-aware send_message variants.
    // ====================================================================

    async fn send_message_with_handle(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    async fn send_message_with_handle_and_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        content_blocks: Option<Vec<librefang_types::message::ContentBlock>>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    #[allow(clippy::too_many_arguments)]
    async fn send_message_with_incognito(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        sender_context: Option<librefang_channels::types::SenderContext>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
        incognito: bool,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    async fn send_message_streaming_with_routing(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )>;
    async fn send_message_streaming_with_incognito(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        session_id_override: Option<SessionId>,
        incognito: bool,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )>;
    async fn send_message_streaming_with_sender_context_routing_thinking_and_session(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        sender: librefang_channels::types::SenderContext,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )>;

    // ====================================================================
    // Spawn-tasks / probe — Arc<Self> receivers.
    // ====================================================================

    fn spawn_key_validation(self: Arc<Self>);
    async fn auto_dream_trigger_manual(
        self: Arc<Self>,
        agent_id: AgentId,
    ) -> crate::auto_dream::TriggerOutcome;
    async fn probe_local_provider(
        self: Arc<Self>,
        provider_id: &str,
        base_url: &str,
        log_offline_as_warn: bool,
    ) -> librefang_runtime::provider_health::ProbeResult;

    // ====================================================================
    // Additional channel / trigger / engine accessors and send_message
    // variants used by channel_bridge and routes.
    // ====================================================================

    fn channel_adapters_ref(
        &self,
    ) -> &dashmap::DashMap<String, Arc<dyn librefang_channels::types::ChannelAdapter>>;
    fn trigger_engine(&self) -> &crate::triggers::TriggerEngine;
    fn broadcast_ref(&self) -> &librefang_types::config::BroadcastConfig;
    fn auto_reply(&self) -> &crate::auto_reply::AutoReplyEngine;
    async fn one_shot_llm_call(&self, model: &str, prompt: &str) -> Result<String, String>;
    async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        blocks: Vec<librefang_types::message::ContentBlock>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    async fn send_message_with_sender_context(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: librefang_channels::types::SenderContext,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    async fn send_message_with_blocks_and_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        blocks: Vec<librefang_types::message::ContentBlock>,
        sender: librefang_channels::types::SenderContext,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult>;
    async fn send_message_streaming_with_sender_context_and_routing(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        sender: librefang_channels::types::SenderContext,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )>;

    // ====================================================================
    // Test-only / boot-only kernel ops surfaced for integration tests
    // (`tests/api_integration_test.rs`, `network_routes_integration`, …).
    // ====================================================================

    fn data_dir(&self) -> &Path;
    fn install_peer_registry_for_test(
        &self,
        registry: librefang_wire::PeerRegistry,
    ) -> Result<(), librefang_wire::PeerRegistry>;
    fn set_self_handle(self: Arc<Self>);

    // ====================================================================
    // Async task tracker (#4983) — exposed on KernelApi so integration
    // tests can drive the registry through the same trait object the
    // dashboard and route handlers use, instead of needing the
    // concrete `LibreFangKernel`.
    // ====================================================================

    /// Register a pending async task. Returns the typed
    /// [`librefang_types::task::TaskHandle`] the spawning agent stashes
    /// to correlate the eventual completion event.
    fn register_async_task(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        kind: librefang_types::task::TaskKind,
    ) -> librefang_types::task::TaskHandle;

    /// Mark a registered async task as terminated with `status`. The
    /// kernel removes the registry entry (delete-on-delivery) and
    /// either injects an
    /// [`librefang_types::tool::AgentLoopSignal::TaskCompleted`] mid-turn
    /// or, if no agent loop is currently attached for the originating
    /// session, spawns a fresh turn whose body is the rendered
    /// completion text. Idempotent — a second call for the same id is
    /// a no-op.
    async fn complete_async_task(
        &self,
        task_id: librefang_types::task::TaskId,
        status: librefang_types::task::TaskStatus,
    ) -> KernelResult<bool>;

    /// Test introspection: number of currently-registered async tasks.
    #[doc(hidden)]
    fn pending_async_task_count(&self) -> usize;

    /// Test introspection: borrow the per-(agent, session) injection
    /// senders dashmap so tests can attach a synthetic receiver without
    /// driving a full agent loop. Lives next to the async task tracker
    /// surface because step-3 integration tests need it to assert
    /// mid-turn delivery from the `complete_async_task` path.
    ///
    /// `#[doc(hidden)]` and named `_ref` so it does not appear in the
    /// public `KernelApi` docs and production route handlers do not
    /// reach for it. Future cleanup (#5033 review nit) should split
    /// this into a `KernelApiTestExt` trait gated by a `test-support`
    /// feature; kept on the main trait for now to match the existing
    /// `set_self_handle` / `install_peer_registry_for_test` precedent.
    #[doc(hidden)]
    fn injection_senders_ref(
        &self,
    ) -> &dashmap::DashMap<
        (AgentId, SessionId),
        tokio::sync::mpsc::Sender<librefang_types::tool::AgentLoopSignal>,
    >;
}

#[async_trait]
impl KernelApi for LibreFangKernel {
    // -- Subsystem accessors --
    fn agent_registry(&self) -> &AgentRegistry {
        <Self as crate::AgentSubsystemApi>::agent_registry_ref(self)
    }
    fn agent_identities(&self) -> &Arc<crate::agent_identity_registry::AgentIdentityRegistry> {
        <Self as crate::AgentSubsystemApi>::identities_ref(self)
    }
    fn approvals(&self) -> &ApprovalManager {
        <Self as crate::GovernanceSubsystemApi>::approvals(self)
    }
    fn audit(&self) -> &Arc<AuditLog> {
        <Self as crate::MeteringSubsystemApi>::audit_log(self)
    }
    fn auth_manager(&self) -> &AuthManager {
        <Self as crate::SecuritySubsystemApi>::auth_ref(self)
    }
    fn browser(&self) -> &librefang_runtime::browser::BrowserManager {
        <Self as crate::MediaSubsystemApi>::browser(self)
    }
    fn cron(&self) -> &CronScheduler {
        <Self as crate::WorkflowSubsystemApi>::cron_ref(self)
    }
    fn delivery(&self) -> &DeliveryTracker {
        <Self as crate::MeshSubsystemApi>::delivery(self)
    }
    fn event_bus_ref(&self) -> &EventBus {
        <Self as crate::EventSubsystemApi>::event_bus_ref(self)
    }
    fn hands(&self) -> &librefang_hands::registry::HandRegistry {
        <Self as crate::SkillsSubsystemApi>::hand_registry_ref(self)
    }
    fn home_dir(&self) -> &Path {
        Self::home_dir(self)
    }
    fn media(&self) -> &librefang_runtime::media_understanding::MediaEngine {
        <Self as crate::MediaSubsystemApi>::media_engine(self)
    }
    fn media_drivers(&self) -> &librefang_runtime::media::MediaDriverCache {
        <Self as crate::MediaSubsystemApi>::drivers(self)
    }
    fn memory_substrate(&self) -> &Arc<MemorySubstrate> {
        <Self as crate::MemorySubsystemApi>::substrate_ref(self)
    }
    fn metering_ref(&self) -> &Arc<MeteringEngine> {
        <Self as crate::MeteringSubsystemApi>::metering_engine(self)
    }
    fn pairing_ref(&self) -> &PairingManager {
        <Self as crate::SecuritySubsystemApi>::pairing_ref(self)
    }
    fn proactive_memory_store(&self) -> Option<&Arc<librefang_memory::ProactiveMemoryStore>> {
        <Self as crate::MemorySubsystemApi>::proactive_store(self)
    }
    fn processes(&self) -> &Arc<librefang_runtime::process_manager::ProcessManager> {
        <Self as crate::ProcessSubsystemApi>::process_manager_ref(self)
    }
    fn process_registry(&self) -> &Arc<librefang_runtime::process_registry::ProcessRegistry> {
        <Self as crate::ProcessSubsystemApi>::process_registry_ref(self)
    }
    fn scheduler_ref(&self) -> &AgentScheduler {
        <Self as crate::AgentSubsystemApi>::scheduler_ref(self)
    }
    fn session_stream_hub(&self) -> Arc<SessionStreamHub> {
        Self::session_stream_hub(self)
    }
    fn supervisor_ref(&self) -> &Supervisor {
        <Self as crate::AgentSubsystemApi>::supervisor_ref(self)
    }
    fn templates(&self) -> &crate::workflow::WorkflowTemplateRegistry {
        <Self as crate::WorkflowSubsystemApi>::templates_ref(self)
    }
    fn tts(&self) -> &librefang_runtime::tts::TtsEngine {
        <Self as crate::MediaSubsystemApi>::tts(self)
    }
    fn web_tools(&self) -> &librefang_runtime::web_search::WebToolsContext {
        <Self as crate::MediaSubsystemApi>::web_tools(self)
    }
    fn workflow_engine(&self) -> &WorkflowEngine {
        <Self as crate::WorkflowSubsystemApi>::engine_ref(self)
    }

    fn command_queue_ref(&self) -> &librefang_runtime::command_lane::CommandQueue {
        <Self as crate::WorkflowSubsystemApi>::command_queue_ref(self)
    }
    fn config_ref(&self) -> arc_swap::Guard<Arc<KernelConfig>> {
        Self::config_ref(self)
    }
    fn config_snapshot(&self) -> Arc<KernelConfig> {
        Self::config_snapshot(self)
    }
    fn context_engine_ref(&self) -> Option<&dyn librefang_runtime::context_engine::ContextEngine> {
        Self::context_engine_ref(self)
    }
    fn default_model_override_ref(
        &self,
    ) -> &std::sync::RwLock<Option<librefang_types::config::DefaultModelConfig>> {
        <Self as crate::LlmSubsystemApi>::default_model_override_ref(self)
    }
    fn mcp_auth_states_ref(&self) -> &librefang_runtime::mcp_oauth::McpAuthStates {
        <Self as crate::McpSubsystemApi>::auth_states_ref(self)
    }
    fn mcp_connections_ref(
        &self,
    ) -> &tokio::sync::Mutex<Vec<librefang_runtime::mcp::McpConnection>> {
        <Self as crate::McpSubsystemApi>::connections_ref(self)
    }
    fn mcp_tools_ref(&self) -> &std::sync::Mutex<Vec<ToolDefinition>> {
        <Self as crate::McpSubsystemApi>::tools_ref(self)
    }
    fn model_catalog_ref(
        &self,
    ) -> &arc_swap::ArcSwap<librefang_runtime::model_catalog::ModelCatalog> {
        <Self as crate::LlmSubsystemApi>::model_catalog_swap(self)
    }
    fn oauth_provider_ref(
        &self,
    ) -> Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider + Send + Sync> {
        Arc::clone(<Self as crate::McpSubsystemApi>::oauth_provider_ref(self))
    }
    fn peer_node_ref(&self) -> Option<&Arc<librefang_wire::PeerNode>> {
        <Self as crate::MeshSubsystemApi>::peer_node_ref(self)
    }
    fn peer_registry_ref(&self) -> Option<&librefang_wire::PeerRegistry> {
        <Self as crate::MeshSubsystemApi>::peer_registry_ref(self)
    }
    fn skill_registry_ref(&self) -> &std::sync::RwLock<librefang_skills::registry::SkillRegistry> {
        <Self as crate::SkillsSubsystemApi>::skill_registry_ref(self)
    }

    // -- Config / lifecycle --
    fn budget_config(&self) -> BudgetConfig {
        <Self as crate::MeteringSubsystemApi>::current_budget(self)
    }
    fn update_budget_config(&self, f: &dyn Fn(&mut BudgetConfig)) {
        Self::update_budget_config(self, f);
    }
    fn shutdown(&self) {
        Self::shutdown(self);
    }
    fn clear_driver_cache(&self) {
        <Self as crate::LlmSubsystemApi>::clear_driver_cache(self);
    }
    fn relocate_legacy_agent_dirs(&self) {
        Self::relocate_legacy_agent_dirs(self);
    }
    fn validate_config_for_reload(&self, config: &KernelConfig) -> Result<(), Vec<String>> {
        Self::validate_config_for_reload(self, config)
    }
    async fn reload_config(&self) -> Result<ReloadPlan, String> {
        Self::reload_config(self).await
    }

    // -- Vault --
    fn vault_get(&self, key: &str) -> Option<String> {
        Self::vault_get(self, key)
    }
    fn vault_set(&self, key: &str, value: &str) -> Result<(), String> {
        Self::vault_set(self, key, value)
    }
    fn vault_redeem_recovery_code(&self, code: &str) -> Result<bool, String> {
        Self::vault_redeem_recovery_code(self, code)
    }

    // -- MCP install façade --
    fn install_integration(
        &self,
        template_id: &str,
        provided_keys: &std::collections::HashMap<String, String>,
    ) -> librefang_extensions::ExtensionResult<librefang_extensions::installer::InstallResult> {
        Self::install_integration(self, template_id, provided_keys)
    }

    // -- Inbox / auto-dream --
    fn inbox_status(&self) -> InboxStatus {
        Self::inbox_status(self)
    }
    async fn auto_dream_status(&self) -> AutoDreamStatus {
        Self::auto_dream_status(self).await
    }
    async fn auto_dream_abort(&self, agent_id: AgentId) -> AbortOutcome {
        Self::auto_dream_abort(self, agent_id).await
    }
    fn auto_dream_set_enabled(&self, agent_id: AgentId, enabled: bool) -> LibreFangResult<()> {
        Self::auto_dream_set_enabled(self, agent_id, enabled)
    }

    // -- Agent lifecycle --
    fn spawn_agent_typed(&self, manifest: AgentManifest) -> KernelResult<AgentId> {
        Self::spawn_agent(self, manifest)
    }
    fn kill_agent_typed(&self, agent_id: AgentId) -> KernelResult<()> {
        Self::kill_agent(self, agent_id)
    }
    fn kill_agent_with_purge(&self, agent_id: AgentId, purge_identity: bool) -> KernelResult<()> {
        Self::kill_agent_with_purge(self, agent_id, purge_identity)
    }
    fn stop_agent_run(&self, agent_id: AgentId) -> KernelResult<bool> {
        Self::stop_agent_run(self, agent_id)
    }
    fn stop_session_run(&self, agent_id: AgentId, session_id: SessionId) -> KernelResult<bool> {
        Self::stop_session_run(self, agent_id, session_id)
    }
    fn suspend_agent(&self, agent_id: AgentId) -> KernelResult<()> {
        Self::suspend_agent(self, agent_id)
    }
    fn resume_agent(&self, agent_id: AgentId) -> KernelResult<()> {
        Self::resume_agent(self, agent_id)
    }
    async fn compact_agent_session(&self, agent_id: AgentId) -> KernelResult<String> {
        Self::compact_agent_session(self, agent_id).await
    }
    async fn compact_agent_session_with_id(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
    ) -> KernelResult<String> {
        Self::compact_agent_session_with_id(self, agent_id, session_id).await
    }
    async fn reset_session(&self, agent_id: AgentId, scope: ResetScope) -> KernelResult<()> {
        Self::reset_session(self, agent_id, scope).await
    }
    async fn reboot_session(&self, agent_id: AgentId, scope: ResetScope) -> KernelResult<()> {
        Self::reboot_session(self, agent_id, scope).await
    }
    async fn clear_agent_history(&self, agent_id: AgentId) -> KernelResult<()> {
        Self::clear_agent_history(self, agent_id).await
    }
    fn list_agent_sessions(&self, agent_id: AgentId) -> KernelResult<Vec<serde_json::Value>> {
        Self::list_agent_sessions(self, agent_id)
    }
    fn create_agent_session(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> KernelResult<serde_json::Value> {
        Self::create_agent_session(self, agent_id, label)
    }
    fn switch_agent_session(&self, agent_id: AgentId, session_id: SessionId) -> KernelResult<()> {
        Self::switch_agent_session(self, agent_id, session_id)
    }
    fn export_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<librefang_memory::session::SessionExport> {
        Self::export_session(self, agent_id, session_id)
    }
    fn import_session(
        &self,
        agent_id: AgentId,
        export: librefang_memory::session::SessionExport,
    ) -> KernelResult<SessionId> {
        Self::import_session(self, agent_id, export)
    }
    fn export_session_trajectory(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<TrajectoryBundle> {
        Self::export_session_trajectory(self, agent_id, session_id)
    }
    fn persist_manifest_to_disk(&self, agent_id: AgentId) {
        Self::persist_manifest_to_disk(self, agent_id);
    }
    fn reload_agent_from_disk(&self, agent_id: AgentId) -> KernelResult<()> {
        Self::reload_agent_from_disk(self, agent_id)
    }
    fn update_manifest(&self, agent_id: AgentId, new_manifest: AgentManifest) -> KernelResult<()> {
        Self::update_manifest(self, agent_id, new_manifest)
    }
    fn set_agent_skills(&self, agent_id: AgentId, skills: Vec<String>) -> KernelResult<()> {
        Self::set_agent_skills(self, agent_id, skills)
    }
    fn set_agent_mcp_servers(&self, agent_id: AgentId, servers: Vec<String>) -> KernelResult<()> {
        Self::set_agent_mcp_servers(self, agent_id, servers)
    }
    fn set_agent_schedule(
        self: Arc<Self>,
        agent_id: AgentId,
        schedule: librefang_types::agent::ScheduleMode,
    ) -> KernelResult<()> {
        LibreFangKernel::set_agent_schedule(&self, agent_id, schedule)
    }
    fn set_agent_tool_filters(
        &self,
        agent_id: AgentId,
        capabilities_tools: Option<Vec<String>>,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> KernelResult<()> {
        Self::set_agent_tool_filters(self, agent_id, capabilities_tools, allowlist, blocklist)
    }
    fn list_running_sessions(&self, agent_id: AgentId) -> Vec<RunningSessionSnapshot> {
        Self::list_running_sessions(self, agent_id)
    }
    fn running_session_ids(&self) -> std::collections::HashSet<SessionId> {
        Self::running_session_ids(self)
    }
    fn verify_signed_manifest(&self, signed_json: &str) -> KernelResult<String> {
        Self::verify_signed_manifest(self, signed_json)
    }
    fn available_tools(&self, agent_id: AgentId) -> Arc<Vec<ToolDefinition>> {
        Self::available_tools(self, agent_id)
    }

    async fn send_message(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message(self, agent_id, message).await
    }
    async fn send_message_ephemeral(
        &self,
        agent_id: AgentId,
        message: &str,
        sender_context: Option<&librefang_channels::types::SenderContext>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_ephemeral(self, agent_id, message, sender_context).await
    }
    async fn send_message_with_session_mode(
        &self,
        agent_id: AgentId,
        message: &str,
        session_mode_override: Option<librefang_types::agent::SessionMode>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_with_session_mode(self, agent_id, message, session_mode_override).await
    }

    // -- Hands --
    fn activate_hand(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> KernelResult<librefang_hands::HandInstance> {
        Self::activate_hand(self, hand_id, config)
    }
    fn deactivate_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        Self::deactivate_hand(self, instance_id)
    }
    fn pause_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        Self::pause_hand(self, instance_id)
    }
    fn resume_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        Self::resume_hand(self, instance_id)
    }
    fn reload_hands(&self) -> (usize, usize) {
        Self::reload_hands(self)
    }
    fn invalidate_hand_route_cache(&self) {
        Self::invalidate_hand_route_cache(self);
    }
    fn persist_hand_state(&self) {
        Self::persist_hand_state(self);
    }
    fn clear_hand_agent_runtime_override(&self, agent_id: AgentId) -> KernelResult<()> {
        Self::clear_hand_agent_runtime_override(self, agent_id)
    }
    fn trigger_all_hands(&self) {
        Self::trigger_all_hands(self);
    }

    // -- MCP --
    fn mcp_health(&self) -> &librefang_extensions::health::HealthMonitor {
        <Self as crate::McpSubsystemApi>::health(self)
    }
    fn mcp_catalog_load(&self) -> arc_swap::Guard<Arc<librefang_extensions::catalog::McpCatalog>> {
        <Self as crate::McpSubsystemApi>::mcp_catalog_load(self)
    }
    async fn connect_mcp_servers(self: Arc<Self>) {
        LibreFangKernel::connect_mcp_servers(&self).await;
    }
    async fn disconnect_mcp_server(&self, name: &str) -> bool {
        Self::disconnect_mcp_server(self, name).await
    }
    async fn retry_mcp_connection(self: Arc<Self>, server_name: &str) {
        LibreFangKernel::retry_mcp_connection(&self, server_name).await;
    }
    async fn reload_mcp_servers(self: Arc<Self>) -> Result<usize, String> {
        LibreFangKernel::reload_mcp_servers(&self).await
    }
    async fn reconnect_mcp_server(self: Arc<Self>, id: &str) -> Result<usize, String> {
        LibreFangKernel::reconnect_mcp_server(&self, id).await
    }

    // -- Triggers / workflows / events --
    fn list_triggers(&self, agent_id: Option<AgentId>) -> Vec<Trigger> {
        Self::list_triggers(self, agent_id)
    }
    fn get_trigger(&self, trigger_id: TriggerId) -> Option<Trigger> {
        Self::get_trigger(self, trigger_id)
    }
    #[allow(clippy::too_many_arguments)]
    fn register_trigger_with_target(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
        target_agent: Option<AgentId>,
        cooldown_secs: Option<u64>,
        session_mode: Option<librefang_types::agent::SessionMode>,
        workflow_id: Option<String>,
    ) -> KernelResult<TriggerId> {
        Self::register_trigger_with_target(
            self,
            agent_id,
            pattern,
            prompt_template,
            max_fires,
            target_agent,
            cooldown_secs,
            session_mode,
            workflow_id,
        )
    }
    fn remove_trigger(&self, trigger_id: TriggerId) -> bool {
        Self::remove_trigger(self, trigger_id)
    }
    fn update_trigger(
        &self,
        trigger_id: TriggerId,
        patch: crate::triggers::TriggerPatch,
    ) -> Option<Trigger> {
        Self::update_trigger(self, trigger_id, patch)
    }
    async fn register_workflow(
        &self,
        workflow: crate::workflow::Workflow,
    ) -> crate::workflow::WorkflowId {
        Self::register_workflow(self, workflow).await
    }
    async fn run_workflow_typed(
        &self,
        workflow_id: crate::workflow::WorkflowId,
        input: String,
    ) -> KernelResult<(crate::workflow::WorkflowRunId, String)> {
        Self::run_workflow(self, workflow_id, input).await
    }
    async fn dry_run_workflow(
        &self,
        workflow_id: crate::workflow::WorkflowId,
        input: String,
    ) -> KernelResult<Vec<crate::workflow::DryRunStep>> {
        Self::dry_run_workflow(self, workflow_id, input).await
    }
    async fn publish_typed_event(&self, event: librefang_types::event::Event) -> Vec<TriggerMatch> {
        Self::publish_event(self, event).await
    }

    // -- Bindings --
    fn list_bindings(&self) -> Vec<AgentBinding> {
        Self::list_bindings(self)
    }
    fn add_binding(&self, binding: AgentBinding) {
        Self::add_binding(self, binding);
    }
    fn remove_binding(&self, index: usize) -> Option<AgentBinding> {
        Self::remove_binding(self, index)
    }

    // -- Skills / driver caches / model catalog --
    fn reload_skills(&self) {
        Self::reload_skills(self);
    }
    fn model_catalog_load(
        &self,
    ) -> arc_swap::Guard<Arc<librefang_runtime::model_catalog::ModelCatalog>> {
        <Self as crate::LlmSubsystemApi>::model_catalog_load(self)
    }
    fn model_catalog_update(
        &self,
        f: &mut dyn FnMut(&mut librefang_runtime::model_catalog::ModelCatalog),
    ) {
        Self::model_catalog_update(self, |catalog| f(catalog));
    }

    // -- Background spawning --
    fn start_background_for_agent(
        self: Arc<Self>,
        agent_id: AgentId,
        name: &str,
        schedule: &librefang_types::agent::ScheduleMode,
    ) {
        LibreFangKernel::start_background_for_agent(&self, agent_id, name, schedule);
    }

    fn background_active_count(&self) -> usize {
        LibreFangKernel::background_active_count(self)
    }

    // -- Additional inherent methods --
    fn agent_has_active_session(&self, agent_id: AgentId) -> bool {
        Self::agent_has_active_session(self, agent_id)
    }
    fn a2a_agents(&self) -> &std::sync::Mutex<Vec<(String, librefang_runtime::a2a::AgentCard)>> {
        <Self as crate::MeshSubsystemApi>::a2a_agents(self)
    }
    fn a2a_tasks(&self) -> &librefang_runtime::a2a::A2aTaskStore {
        <Self as crate::MeshSubsystemApi>::a2a_tasks(self)
    }
    fn context_report(
        &self,
        agent_id: AgentId,
    ) -> KernelResult<librefang_runtime::compactor::ContextReport> {
        Self::context_report(self, agent_id)
    }
    fn effective_mcp_servers_ref(
        &self,
    ) -> &std::sync::RwLock<Vec<librefang_types::config::McpServerConfigEntry>> {
        <Self as crate::McpSubsystemApi>::effective_servers_ref(self)
    }
    fn embedding(
        &self,
    ) -> Option<&Arc<dyn librefang_runtime::embedding::EmbeddingDriver + Send + Sync>> {
        <Self as crate::LlmSubsystemApi>::embedding(self)
    }
    async fn inject_message_for_session(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
        message: &str,
    ) -> KernelResult<bool> {
        Self::inject_message_for_session(self, agent_id, session_id, message).await
    }
    fn provider_unconfigured_flag(&self) -> &std::sync::atomic::AtomicBool {
        Self::provider_unconfigured_flag(self)
    }
    fn session_usage_cost(&self, agent_id: AgentId) -> KernelResult<(u64, u64, f64)> {
        Self::session_usage_cost(self, agent_id)
    }
    fn set_agent_model(
        &self,
        agent_id: AgentId,
        model: &str,
        explicit_provider: Option<&str>,
    ) -> KernelResult<()> {
        Self::set_agent_model(self, agent_id, model, explicit_provider)
    }
    fn sync_default_model_agents(
        &self,
        old_provider: &str,
        dm: &librefang_types::config::DefaultModelConfig,
    ) -> Vec<(String, String)> {
        Self::sync_default_model_agents(self, old_provider, dm)
    }
    fn traces(&self) -> &dashmap::DashMap<AgentId, Vec<librefang_types::tool::DecisionTrace>> {
        <Self as crate::AgentSubsystemApi>::traces(self)
    }
    fn update_hand_agent_runtime_override(
        &self,
        agent_id: AgentId,
        override_config: librefang_hands::HandAgentRuntimeOverride,
    ) -> KernelResult<()> {
        Self::update_hand_agent_runtime_override(self, agent_id, override_config)
    }

    // -- Streaming + handle-aware send_message variants --
    async fn send_message_with_handle(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_with_handle(self, agent_id, message, kernel_handle).await
    }
    async fn send_message_with_handle_and_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        content_blocks: Option<Vec<librefang_types::message::ContentBlock>>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_with_handle_and_blocks(
            self,
            agent_id,
            message,
            kernel_handle,
            content_blocks,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn send_message_with_incognito(
        &self,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        sender_context: Option<librefang_channels::types::SenderContext>,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
        incognito: bool,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_with_incognito(
            self,
            agent_id,
            message,
            kernel_handle,
            sender_context.as_ref(),
            thinking_override,
            session_id_override,
            incognito,
        )
        .await
    }
    async fn send_message_streaming_with_routing(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )> {
        LibreFangKernel::send_message_streaming_with_routing(
            &self,
            agent_id,
            message,
            kernel_handle,
        )
        .await
    }
    async fn send_message_streaming_with_incognito(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        session_id_override: Option<SessionId>,
        incognito: bool,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )> {
        LibreFangKernel::send_message_streaming_with_incognito(
            &self,
            agent_id,
            message,
            kernel_handle,
            session_id_override,
            incognito,
        )
        .await
    }
    async fn send_message_streaming_with_sender_context_routing_thinking_and_session(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        sender: librefang_channels::types::SenderContext,
        thinking_override: Option<bool>,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )> {
        LibreFangKernel::send_message_streaming_with_sender_context_routing_thinking_and_session(
            &self,
            agent_id,
            message,
            kernel_handle,
            &sender,
            thinking_override,
            session_id_override,
        )
        .await
    }

    // -- Spawn-tasks / probe --
    fn spawn_key_validation(self: Arc<Self>) {
        LibreFangKernel::spawn_key_validation(self);
    }
    async fn auto_dream_trigger_manual(
        self: Arc<Self>,
        agent_id: AgentId,
    ) -> crate::auto_dream::TriggerOutcome {
        LibreFangKernel::auto_dream_trigger_manual(self, agent_id).await
    }
    async fn probe_local_provider(
        self: Arc<Self>,
        provider_id: &str,
        base_url: &str,
        log_offline_as_warn: bool,
    ) -> librefang_runtime::provider_health::ProbeResult {
        LibreFangKernel::probe_local_provider(&self, provider_id, base_url, log_offline_as_warn)
            .await
    }

    // -- Channel / trigger / engine accessors and send_message variants --
    fn channel_adapters_ref(
        &self,
    ) -> &dashmap::DashMap<String, Arc<dyn librefang_channels::types::ChannelAdapter>> {
        <Self as crate::MeshSubsystemApi>::channel_adapters_ref(self)
    }
    fn trigger_engine(&self) -> &crate::triggers::TriggerEngine {
        <Self as crate::WorkflowSubsystemApi>::triggers_ref(self)
    }
    fn broadcast_ref(&self) -> &librefang_types::config::BroadcastConfig {
        <Self as crate::MeshSubsystemApi>::broadcast_ref(self)
    }
    fn auto_reply(&self) -> &crate::auto_reply::AutoReplyEngine {
        Self::auto_reply(self)
    }
    async fn one_shot_llm_call(&self, model: &str, prompt: &str) -> Result<String, String> {
        Self::one_shot_llm_call(self, model, prompt).await
    }
    async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        message: &str,
        blocks: Vec<librefang_types::message::ContentBlock>,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_with_blocks(self, agent_id, message, blocks).await
    }
    async fn send_message_with_sender_context(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: librefang_channels::types::SenderContext,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_with_sender_context(self, agent_id, message, &sender).await
    }
    async fn send_message_with_blocks_and_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        blocks: Vec<librefang_types::message::ContentBlock>,
        sender: librefang_channels::types::SenderContext,
    ) -> KernelResult<librefang_runtime::agent_loop::AgentLoopResult> {
        Self::send_message_with_blocks_and_sender(self, agent_id, message, blocks, &sender).await
    }
    async fn send_message_streaming_with_sender_context_and_routing(
        self: Arc<Self>,
        agent_id: AgentId,
        message: &str,
        kernel_handle: Option<Arc<dyn crate::kernel_handle::KernelHandle>>,
        sender: librefang_channels::types::SenderContext,
    ) -> KernelResult<(
        tokio::sync::mpsc::Receiver<librefang_runtime::llm_driver::StreamEvent>,
        tokio::task::JoinHandle<KernelResult<librefang_runtime::agent_loop::AgentLoopResult>>,
    )> {
        LibreFangKernel::send_message_streaming_with_sender_context_and_routing(
            &self,
            agent_id,
            message,
            kernel_handle,
            &sender,
        )
        .await
    }

    // -- Test-only / boot-only ops --
    fn data_dir(&self) -> &Path {
        Self::data_dir(self)
    }
    fn install_peer_registry_for_test(
        &self,
        registry: librefang_wire::PeerRegistry,
    ) -> Result<(), librefang_wire::PeerRegistry> {
        Self::install_peer_registry_for_test(self, registry)
    }
    fn set_self_handle(self: Arc<Self>) {
        LibreFangKernel::set_self_handle(&self);
    }

    // -- Async task tracker (#4983) --
    fn register_async_task(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        kind: librefang_types::task::TaskKind,
    ) -> librefang_types::task::TaskHandle {
        LibreFangKernel::register_async_task(self, agent_id, session_id, kind)
    }
    async fn complete_async_task(
        &self,
        task_id: librefang_types::task::TaskId,
        status: librefang_types::task::TaskStatus,
    ) -> KernelResult<bool> {
        LibreFangKernel::complete_async_task(self, task_id, status).await
    }
    fn pending_async_task_count(&self) -> usize {
        LibreFangKernel::pending_async_task_count(self)
    }
    fn injection_senders_ref(
        &self,
    ) -> &dashmap::DashMap<
        (AgentId, SessionId),
        tokio::sync::mpsc::Sender<librefang_types::tool::AgentLoopSignal>,
    > {
        <Self as crate::EventSubsystemApi>::injection_senders_ref(self)
    }
}
