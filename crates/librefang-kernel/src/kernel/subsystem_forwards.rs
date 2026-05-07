//! Forward implementations of every per-subsystem trait on
//! `LibreFangKernel` (refs #3565 follow-up #3).
//!
//! `LibreFangKernel` already implements the fat `KernelApi` trait
//! exposed to the API layer. This module additionally re-implements
//! each focused `*SubsystemApi` trait by delegating to the
//! corresponding subsystem field. The point is to let new callers (and
//! tests / mocks) bind against a focused trait such as
//! `Arc<dyn AgentSubsystemApi>` or
//! `&dyn MeteringSubsystemApi` without dragging in the entire
//! `KernelApi` surface — while the existing `Arc<dyn KernelApi>` flows
//! continue to work unchanged.

use std::sync::{Arc, RwLock};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use librefang_extensions::catalog::McpCatalog;
use librefang_extensions::health::HealthMonitor;
use librefang_memory::{MemorySubstrate, ProactiveMemoryStore};
use librefang_runtime::a2a::{A2aTaskStore, AgentCard};
use librefang_runtime::audit::AuditLog;
use librefang_runtime::browser::BrowserManager;
use librefang_runtime::embedding::EmbeddingDriver;
use librefang_runtime::mcp::McpConnection;
use librefang_runtime::mcp_oauth::{McpAuthStates, McpOAuthProvider};
use librefang_runtime::media::MediaDriverCache;
use librefang_runtime::media_understanding::MediaEngine;
use librefang_runtime::model_catalog::ModelCatalog;
use librefang_runtime::tts::TtsEngine;
use librefang_runtime::web_search::WebToolsContext;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::config::{
    AgentBinding, BroadcastConfig, BudgetConfig, DefaultModelConfig, McpServerConfigEntry,
};
use librefang_types::tool::{AgentLoopSignal, DecisionTrace, ToolDefinition};
use librefang_wire::{PeerNode, PeerRegistry};

use super::subsystems::{
    AgentSubsystemApi, EventSubsystemApi, GovernanceSubsystemApi, LlmSubsystemApi, McpSubsystemApi,
    MediaSubsystemApi, MemorySubsystemApi, MeshSubsystemApi, MeteringSubsystemApi,
    ProcessSubsystemApi, SecuritySubsystemApi, SkillsSubsystemApi, WorkflowSubsystemApi,
};
use super::DeliveryTracker;
use super::LibreFangKernel;
use crate::agent_identity_registry::AgentIdentityRegistry;
use crate::approval::ApprovalManager;
use crate::auth::AuthManager;
use crate::cron::CronScheduler;
use crate::event_bus::EventBus;
use crate::pairing::PairingManager;
use crate::registry::AgentRegistry;
use crate::scheduler::AgentScheduler;
use crate::session_lifecycle::SessionLifecycleBus;
use crate::supervisor::Supervisor;
use crate::triggers::TriggerEngine;
use crate::workflow::{WorkflowEngine, WorkflowTemplateRegistry};
use librefang_channels::types::ChannelAdapter;
use librefang_hands::registry::HandRegistry;
use librefang_runtime::command_lane::CommandQueue;
use librefang_runtime::hooks::HookRegistry;
use librefang_skills::registry::SkillRegistry;

impl AgentSubsystemApi for LibreFangKernel {
    #[inline]
    fn agent_registry_ref(&self) -> &AgentRegistry {
        self.agents.agent_registry_ref()
    }
    #[inline]
    fn identities_ref(&self) -> &Arc<AgentIdentityRegistry> {
        self.agents.identities_ref()
    }
    #[inline]
    fn scheduler_ref(&self) -> &AgentScheduler {
        self.agents.scheduler_ref()
    }
    #[inline]
    fn supervisor_ref(&self) -> &Supervisor {
        self.agents.supervisor_ref()
    }
    #[inline]
    fn traces(&self) -> &DashMap<AgentId, Vec<DecisionTrace>> {
        self.agents.traces()
    }
}

impl EventSubsystemApi for LibreFangKernel {
    #[inline]
    fn event_bus_ref(&self) -> &EventBus {
        self.events.event_bus_ref()
    }
    #[inline]
    fn lifecycle_bus(&self) -> Arc<SessionLifecycleBus> {
        self.events.lifecycle_bus()
    }
    #[inline]
    fn injection_senders_ref(
        &self,
    ) -> &DashMap<(AgentId, SessionId), tokio::sync::mpsc::Sender<AgentLoopSignal>> {
        self.events.injection_senders_ref()
    }
}

impl GovernanceSubsystemApi for LibreFangKernel {
    #[inline]
    fn approvals(&self) -> &ApprovalManager {
        self.governance.approvals()
    }
    #[inline]
    fn hook_registry(&self) -> &HookRegistry {
        self.governance.hook_registry()
    }
}

impl LlmSubsystemApi for LibreFangKernel {
    #[inline]
    fn model_catalog_swap(&self) -> &ArcSwap<ModelCatalog> {
        self.llm.model_catalog_swap()
    }
    #[inline]
    fn model_catalog_load(&self) -> arc_swap::Guard<Arc<ModelCatalog>> {
        self.llm.model_catalog_load()
    }
    #[inline]
    fn clear_driver_cache(&self) {
        self.llm.clear_driver_cache();
    }
    #[inline]
    fn embedding(&self) -> Option<&Arc<dyn EmbeddingDriver + Send + Sync>> {
        self.llm.embedding()
    }
    #[inline]
    fn default_model_override_ref(&self) -> &RwLock<Option<DefaultModelConfig>> {
        self.llm.default_model_override_ref()
    }
}

impl McpSubsystemApi for LibreFangKernel {
    #[inline]
    fn mcp_catalog_swap(&self) -> &ArcSwap<McpCatalog> {
        self.mcp.mcp_catalog_swap()
    }
    #[inline]
    fn mcp_catalog_load(&self) -> arc_swap::Guard<Arc<McpCatalog>> {
        self.mcp.mcp_catalog_load()
    }
    #[inline]
    fn health(&self) -> &HealthMonitor {
        self.mcp.health()
    }
    #[inline]
    fn connections_ref(&self) -> &tokio::sync::Mutex<Vec<McpConnection>> {
        self.mcp.connections_ref()
    }
    #[inline]
    fn auth_states_ref(&self) -> &McpAuthStates {
        self.mcp.auth_states_ref()
    }
    #[inline]
    fn oauth_provider_ref(&self) -> &Arc<dyn McpOAuthProvider + Send + Sync> {
        self.mcp.oauth_provider_ref()
    }
    #[inline]
    fn tools_ref(&self) -> &std::sync::Mutex<Vec<ToolDefinition>> {
        self.mcp.tools_ref()
    }
    #[inline]
    fn effective_servers_ref(&self) -> &std::sync::RwLock<Vec<McpServerConfigEntry>> {
        self.mcp.effective_servers_ref()
    }
}

impl MediaSubsystemApi for LibreFangKernel {
    #[inline]
    fn web_tools(&self) -> &WebToolsContext {
        self.media.web_tools()
    }
    #[inline]
    fn browser(&self) -> &BrowserManager {
        self.media.browser()
    }
    #[inline]
    fn media_engine(&self) -> &MediaEngine {
        self.media.media_engine()
    }
    #[inline]
    fn tts(&self) -> &TtsEngine {
        self.media.tts()
    }
    #[inline]
    fn drivers(&self) -> &MediaDriverCache {
        self.media.drivers()
    }
}

impl MemorySubsystemApi for LibreFangKernel {
    #[inline]
    fn substrate_ref(&self) -> &Arc<MemorySubstrate> {
        self.memory.substrate_ref()
    }
    #[inline]
    fn proactive_store(&self) -> Option<&Arc<ProactiveMemoryStore>> {
        self.memory.proactive_store()
    }
}

impl MeshSubsystemApi for LibreFangKernel {
    #[inline]
    fn a2a_tasks(&self) -> &A2aTaskStore {
        self.mesh.a2a_tasks()
    }
    #[inline]
    fn a2a_agents(&self) -> &std::sync::Mutex<Vec<(String, AgentCard)>> {
        self.mesh.a2a_agents()
    }
    #[inline]
    fn channel_adapters_ref(&self) -> &DashMap<String, Arc<dyn ChannelAdapter>> {
        self.mesh.channel_adapters_ref()
    }
    #[inline]
    fn bindings_ref(&self) -> &std::sync::Mutex<Vec<AgentBinding>> {
        self.mesh.bindings_ref()
    }
    #[inline]
    fn broadcast_ref(&self) -> &BroadcastConfig {
        self.mesh.broadcast_ref()
    }
    #[inline]
    fn delivery(&self) -> &DeliveryTracker {
        self.mesh.delivery()
    }
    #[inline]
    fn peer_registry_ref(&self) -> Option<&PeerRegistry> {
        self.mesh.peer_registry_ref()
    }
    #[inline]
    fn peer_node_ref(&self) -> Option<&Arc<PeerNode>> {
        self.mesh.peer_node_ref()
    }
}

impl MeteringSubsystemApi for LibreFangKernel {
    #[inline]
    fn audit_log(&self) -> &Arc<AuditLog> {
        self.metering.audit_log()
    }
    #[inline]
    fn metering_engine(&self) -> &Arc<crate::metering::MeteringEngine> {
        self.metering.metering_engine()
    }
    #[inline]
    fn current_budget(&self) -> BudgetConfig {
        self.metering.current_budget()
    }
}

impl ProcessSubsystemApi for LibreFangKernel {
    #[inline]
    fn process_manager_ref(&self) -> &Arc<librefang_runtime::process_manager::ProcessManager> {
        self.processes.process_manager_ref()
    }
    #[inline]
    fn process_registry_ref(&self) -> &Arc<librefang_runtime::process_registry::ProcessRegistry> {
        self.processes.process_registry_ref()
    }
}

impl SecuritySubsystemApi for LibreFangKernel {
    #[inline]
    fn auth_ref(&self) -> &AuthManager {
        self.security.auth_ref()
    }
    #[inline]
    fn pairing_ref(&self) -> &PairingManager {
        self.security.pairing_ref()
    }
}

impl SkillsSubsystemApi for LibreFangKernel {
    #[inline]
    fn skill_registry_ref(&self) -> &std::sync::RwLock<SkillRegistry> {
        self.skills.skill_registry_ref()
    }
    #[inline]
    fn hand_registry_ref(&self) -> &HandRegistry {
        self.skills.hand_registry_ref()
    }
}

impl WorkflowSubsystemApi for LibreFangKernel {
    #[inline]
    fn engine_ref(&self) -> &WorkflowEngine {
        self.workflows.engine_ref()
    }
    #[inline]
    fn templates_ref(&self) -> &WorkflowTemplateRegistry {
        self.workflows.templates_ref()
    }
    #[inline]
    fn triggers_ref(&self) -> &TriggerEngine {
        self.workflows.triggers_ref()
    }
    #[inline]
    fn cron_ref(&self) -> &CronScheduler {
        self.workflows.cron_ref()
    }
    #[inline]
    fn command_queue_ref(&self) -> &CommandQueue {
        self.workflows.command_queue_ref()
    }
}
