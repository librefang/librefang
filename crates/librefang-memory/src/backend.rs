//! Backend traits for memory subsystem storage — agent registry, sessions,
//! key-value store, task queue, usage events, devices, prompt versioning, and
//! knowledge graph — all part of the `surrealdb-storage-swap` migration plan.
//!
//! Each trait defines a minimal surface matching the calls the kernel and API
//! layers make today.  Implementations exist for:
//! - [`crate::MemorySubstrate`] (rusqlite, always compiled in as fallback)
//! - `SurrealMemoryBackend` / `SurrealSessionBackend` / etc. behind
//!   `#[cfg(feature = "surreal-backend")]`

use crate::MemorySubstrate;
use async_trait::async_trait;
use librefang_types::agent::{
    AgentEntry, AgentId, AgentId as PromptAgentId, ExperimentStatus, ExperimentVariantMetrics,
    PromptExperiment, PromptVersion, SessionId, UserId,
};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::message::Message;
use std::collections::HashMap;
use uuid::Uuid;

// Re-import Session from the session module so trait impls can use it.
use crate::session::Session;
use crate::usage::{ModelUsage, UsageRecord, UsageSummary};
use librefang_types::memory::{
    ConsolidationReport, Entity, GraphMatch, GraphPattern, MemoryFilter, MemoryFragment, MemoryId,
    MemorySource, Relation,
};

// ── MemoryBackend ────────────────────────────────────────────────────────────

/// Backend-agnostic interface for agent registry persistence.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    /// Persist an agent definition.
    fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()>;

    /// Look up a single agent by id.
    fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>>;

    /// Remove an agent and cascade-delete its sessions.
    fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()>;

    /// Load every persisted agent definition.
    fn load_all_agents(&self) -> LibreFangResult<Vec<AgentEntry>>;
}

#[async_trait]
impl MemoryBackend for MemorySubstrate {
    fn save_agent(&self, entry: &AgentEntry) -> LibreFangResult<()> {
        MemorySubstrate::save_agent(self, entry)
    }

    fn load_agent(&self, agent_id: AgentId) -> LibreFangResult<Option<AgentEntry>> {
        MemorySubstrate::load_agent(self, agent_id)
    }

    fn remove_agent(&self, agent_id: AgentId) -> LibreFangResult<()> {
        MemorySubstrate::remove_agent(self, agent_id)
    }

    fn load_all_agents(&self) -> LibreFangResult<Vec<AgentEntry>> {
        MemorySubstrate::load_all_agents(self)
    }
}

// ── SessionBackend ────────────────────────────────────────────────────────────

/// Backend-agnostic interface for session and canonical-session persistence.
///
/// Covers the ~11 call sites in `kernel/mod.rs` that previously reached
/// `self.memory.{get,save,delete}_session`, `append_canonical`, and
/// `delete_canonical_session` directly on the SQLite substrate.
pub trait SessionBackend: Send + Sync {
    /// Retrieve a session by ID.
    fn get_session(&self, session_id: SessionId) -> LibreFangResult<Option<Session>>;

    /// Persist a session (insert or update).
    fn save_session(&self, session: &Session) -> LibreFangResult<()>;

    /// Return all session IDs for an agent.
    fn get_agent_session_ids(&self, agent_id: AgentId) -> LibreFangResult<Vec<SessionId>>;

    /// Delete all sessions for an agent.
    fn delete_agent_sessions(&self, agent_id: AgentId) -> LibreFangResult<()>;

    /// Append messages to the agent's canonical session.
    fn append_canonical(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        compaction_threshold: Option<usize>,
        session_id: Option<SessionId>,
    ) -> LibreFangResult<()>;

    /// Delete the canonical session for an agent.
    fn delete_canonical_session(&self, agent_id: AgentId) -> LibreFangResult<()>;
}

impl SessionBackend for MemorySubstrate {
    fn get_session(&self, session_id: SessionId) -> LibreFangResult<Option<Session>> {
        MemorySubstrate::get_session(self, session_id)
    }

    fn save_session(&self, session: &Session) -> LibreFangResult<()> {
        MemorySubstrate::save_session(self, session)
    }

    fn get_agent_session_ids(&self, agent_id: AgentId) -> LibreFangResult<Vec<SessionId>> {
        MemorySubstrate::get_agent_session_ids(self, agent_id)
    }

    fn delete_agent_sessions(&self, agent_id: AgentId) -> LibreFangResult<()> {
        MemorySubstrate::delete_agent_sessions(self, agent_id)
    }

    fn append_canonical(
        &self,
        agent_id: AgentId,
        messages: &[Message],
        compaction_threshold: Option<usize>,
        session_id: Option<SessionId>,
    ) -> LibreFangResult<()> {
        MemorySubstrate::append_canonical(
            self,
            agent_id,
            messages,
            compaction_threshold,
            session_id,
        )
    }

    fn delete_canonical_session(&self, agent_id: AgentId) -> LibreFangResult<()> {
        MemorySubstrate::delete_canonical_session(self, agent_id)
    }
}

// ── KvBackend ─────────────────────────────────────────────────────────────────

/// Backend-agnostic interface for per-agent key-value storage.
///
/// Covers the 3 `self.memory.structured_get` call sites in `kernel/mod.rs`.
pub trait KvBackend: Send + Sync {
    /// Retrieve a value by (agent_id, key).
    fn structured_get(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> LibreFangResult<Option<serde_json::Value>>;

    /// Insert or update a (agent_id, key) → value mapping.
    fn structured_set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> LibreFangResult<()>;

    /// Delete a key for an agent.
    fn structured_delete(&self, agent_id: AgentId, key: &str) -> LibreFangResult<()>;
}

impl KvBackend for MemorySubstrate {
    fn structured_get(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> LibreFangResult<Option<serde_json::Value>> {
        MemorySubstrate::structured_get(self, agent_id, key)
    }

    fn structured_set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> LibreFangResult<()> {
        MemorySubstrate::structured_set(self, agent_id, key, value)
    }

    fn structured_delete(&self, agent_id: AgentId, key: &str) -> LibreFangResult<()> {
        MemorySubstrate::structured_delete(self, agent_id, key)
    }
}

// ── ProactiveMemoryBackend ────────────────────────────────────────────────────

/// Backend-agnostic interface for proactive memory (decay, consolidation, VACUUM).
///
/// Covers the 3 call sites in `kernel/mod.rs` that previously reached
/// `self.memory.run_decay`, `consolidate`, and `vacuum_if_shrank` directly.
#[async_trait]
pub trait ProactiveMemoryBackend: Send + Sync {
    /// Apply the decay policy to stale memories.  Returns the count removed.
    fn run_decay(
        &self,
        config: &librefang_types::config::MemoryDecayConfig,
    ) -> LibreFangResult<usize>;

    /// Merge and decay memories to free space.
    async fn consolidate(&self) -> LibreFangResult<ConsolidationReport>;

    /// Optional SQLite VACUUM after large prune operations; no-op on SurrealDB.
    fn vacuum_if_shrank(&self, pruned_count: usize) -> LibreFangResult<()>;
}

#[async_trait]
impl ProactiveMemoryBackend for MemorySubstrate {
    fn run_decay(
        &self,
        config: &librefang_types::config::MemoryDecayConfig,
    ) -> LibreFangResult<usize> {
        MemorySubstrate::run_decay(self, config)
    }

    async fn consolidate(&self) -> LibreFangResult<ConsolidationReport> {
        // Delegate to the Memory trait impl on MemorySubstrate.
        <MemorySubstrate as librefang_types::memory::Memory>::consolidate(self).await
    }

    fn vacuum_if_shrank(&self, pruned_count: usize) -> LibreFangResult<()> {
        MemorySubstrate::vacuum_if_shrank(self, pruned_count)
    }
}

// ── TaskBackend ───────────────────────────────────────────────────────────────

/// Backend-agnostic interface for the per-agent task queue.
#[async_trait]
pub trait TaskBackend: Send + Sync {
    /// Reset tasks that have been running longer than `ttl_secs` back to
    /// 'pending' so they can be retried.  Returns the IDs of reset tasks.
    async fn task_reset_stuck(
        &self,
        ttl_secs: u64,
        max_retries: u32,
    ) -> LibreFangResult<Vec<String>>;
}

#[async_trait]
impl TaskBackend for MemorySubstrate {
    async fn task_reset_stuck(
        &self,
        ttl_secs: u64,
        max_retries: u32,
    ) -> LibreFangResult<Vec<String>> {
        MemorySubstrate::task_reset_stuck(self, ttl_secs, max_retries).await
    }
}

// ── UsageBackend ──────────────────────────────────────────────────────────────

/// Backend-agnostic interface for LLM usage metering and quota enforcement.
///
/// Mirrors the complete API surface of [`crate::usage::UsageStore`] so that
/// [`librefang_kernel_metering::MeteringEngine`] can hold an
/// `Arc<dyn UsageBackend>` and switch between SQLite and SurrealDB at boot.
pub trait UsageBackend: Send + Sync {
    /// Record a single usage event.
    fn record(&self, record: &UsageRecord) -> LibreFangResult<()>;

    /// Cost spent by `agent_id` in the last calendar hour.
    fn query_hourly(&self, agent_id: AgentId) -> LibreFangResult<f64>;

    /// Cost spent by `agent_id` since the start of the current UTC day.
    fn query_daily(&self, agent_id: AgentId) -> LibreFangResult<f64>;

    /// Cost spent by `agent_id` since the start of the current UTC month.
    fn query_monthly(&self, agent_id: AgentId) -> LibreFangResult<f64>;

    /// Total cost across all agents in the last calendar hour.
    fn query_global_hourly(&self) -> LibreFangResult<f64>;

    /// Total cost across all agents since the start of the current UTC day.
    fn query_today_cost(&self) -> LibreFangResult<f64>;

    /// Total cost across all agents since the start of the current UTC month.
    fn query_global_monthly(&self) -> LibreFangResult<f64>;

    /// Aggregate usage summary, optionally filtered to a single agent.
    fn query_summary(&self, agent_id: Option<AgentId>) -> LibreFangResult<UsageSummary>;

    /// Usage totals grouped by model.
    fn query_by_model(&self) -> LibreFangResult<Vec<ModelUsage>>;

    /// Per-agent quota check + atomic record.
    fn check_quota_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()>;

    /// Global budget check + atomic record.
    fn check_global_budget_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()>;

    /// Combined per-agent quota + global budget + per-provider budget check + record.
    #[allow(clippy::too_many_arguments)]
    fn check_all_with_provider_and_record(
        &self,
        record: &UsageRecord,
        agent_max_hourly: f64,
        agent_max_daily: f64,
        agent_max_monthly: f64,
        global_max_hourly: f64,
        global_max_daily: f64,
        global_max_monthly: f64,
        provider_max_hourly: f64,
        provider_max_daily: f64,
        provider_max_monthly: f64,
        provider_max_tokens_per_hour: u64,
    ) -> LibreFangResult<()>;

    /// Cost for a given provider in the last calendar hour.
    fn query_provider_hourly(&self, provider: &str) -> LibreFangResult<f64>;

    /// Cost for a given provider since the start of the current UTC day.
    fn query_provider_daily(&self, provider: &str) -> LibreFangResult<f64>;

    /// Cost for a given provider since the start of the current UTC month.
    fn query_provider_monthly(&self, provider: &str) -> LibreFangResult<f64>;

    /// Input tokens consumed by a provider in the last calendar hour.
    fn query_provider_tokens_hourly(&self, provider: &str) -> LibreFangResult<u64>;

    /// Delete usage records older than `days` days.  Returns rows deleted.
    fn cleanup_old(&self, days: u32) -> LibreFangResult<usize>;

    /// Cost for a given user in the last calendar hour.
    fn query_user_hourly(&self, user_id: UserId) -> LibreFangResult<f64>;

    /// Cost for a given user since the start of the current UTC day.
    fn query_user_daily(&self, user_id: UserId) -> LibreFangResult<f64>;

    /// Cost for a given user since the start of the current UTC month.
    fn query_user_monthly(&self, user_id: UserId) -> LibreFangResult<f64>;
}

impl UsageBackend for crate::usage::UsageStore {
    fn record(&self, record: &UsageRecord) -> LibreFangResult<()> {
        crate::usage::UsageStore::record(self, record)
    }

    fn query_hourly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_hourly(self, agent_id)
    }

    fn query_daily(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_daily(self, agent_id)
    }

    fn query_monthly(&self, agent_id: AgentId) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_monthly(self, agent_id)
    }

    fn query_global_hourly(&self) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_global_hourly(self)
    }

    fn query_today_cost(&self) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_today_cost(self)
    }

    fn query_global_monthly(&self) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_global_monthly(self)
    }

    fn query_summary(&self, agent_id: Option<AgentId>) -> LibreFangResult<UsageSummary> {
        crate::usage::UsageStore::query_summary(self, agent_id)
    }

    fn query_by_model(&self) -> LibreFangResult<Vec<ModelUsage>> {
        crate::usage::UsageStore::query_by_model(self)
    }

    fn check_quota_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()> {
        crate::usage::UsageStore::check_quota_and_record(
            self,
            record,
            max_hourly,
            max_daily,
            max_monthly,
        )
    }

    fn check_global_budget_and_record(
        &self,
        record: &UsageRecord,
        max_hourly: f64,
        max_daily: f64,
        max_monthly: f64,
    ) -> LibreFangResult<()> {
        crate::usage::UsageStore::check_global_budget_and_record(
            self,
            record,
            max_hourly,
            max_daily,
            max_monthly,
        )
    }

    fn check_all_with_provider_and_record(
        &self,
        record: &UsageRecord,
        agent_max_hourly: f64,
        agent_max_daily: f64,
        agent_max_monthly: f64,
        global_max_hourly: f64,
        global_max_daily: f64,
        global_max_monthly: f64,
        provider_max_hourly: f64,
        provider_max_daily: f64,
        provider_max_monthly: f64,
        provider_max_tokens_per_hour: u64,
    ) -> LibreFangResult<()> {
        crate::usage::UsageStore::check_all_with_provider_and_record(
            self,
            record,
            agent_max_hourly,
            agent_max_daily,
            agent_max_monthly,
            global_max_hourly,
            global_max_daily,
            global_max_monthly,
            provider_max_hourly,
            provider_max_daily,
            provider_max_monthly,
            provider_max_tokens_per_hour,
        )
    }

    fn query_provider_hourly(&self, provider: &str) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_provider_hourly(self, provider)
    }

    fn query_provider_daily(&self, provider: &str) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_provider_daily(self, provider)
    }

    fn query_provider_monthly(&self, provider: &str) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_provider_monthly(self, provider)
    }

    fn query_provider_tokens_hourly(&self, provider: &str) -> LibreFangResult<u64> {
        crate::usage::UsageStore::query_provider_tokens_hourly(self, provider)
    }

    fn cleanup_old(&self, days: u32) -> LibreFangResult<usize> {
        crate::usage::UsageStore::cleanup_old(self, days)
    }

    fn query_user_hourly(&self, user_id: UserId) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_user_hourly(self, user_id)
    }

    fn query_user_daily(&self, user_id: UserId) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_user_daily(self, user_id)
    }

    fn query_user_monthly(&self, user_id: UserId) -> LibreFangResult<f64> {
        crate::usage::UsageStore::query_user_monthly(self, user_id)
    }
}

// ── DeviceBackend ─────────────────────────────────────────────────────────────

/// Backend-agnostic interface for paired-device persistence.
pub trait DeviceBackend: Send + Sync {
    /// Load all paired devices as JSON objects.
    fn load_paired_devices(&self) -> LibreFangResult<Vec<serde_json::Value>>;

    /// Upsert a paired-device record.
    fn save_paired_device(
        &self,
        device_id: &str,
        display_name: &str,
        platform: &str,
        paired_at: &str,
        last_seen: &str,
        push_token: Option<&str>,
    ) -> LibreFangResult<()>;

    /// Remove a paired device by id.
    fn remove_paired_device(&self, device_id: &str) -> LibreFangResult<()>;
}

impl DeviceBackend for MemorySubstrate {
    fn load_paired_devices(&self) -> LibreFangResult<Vec<serde_json::Value>> {
        MemorySubstrate::load_paired_devices(self)
    }

    fn save_paired_device(
        &self,
        device_id: &str,
        display_name: &str,
        platform: &str,
        paired_at: &str,
        last_seen: &str,
        push_token: Option<&str>,
    ) -> LibreFangResult<()> {
        MemorySubstrate::save_paired_device(
            self,
            device_id,
            display_name,
            platform,
            paired_at,
            last_seen,
            push_token,
        )
    }

    fn remove_paired_device(&self, device_id: &str) -> LibreFangResult<()> {
        MemorySubstrate::remove_paired_device(self, device_id)
    }
}

// ── PromptBackend ─────────────────────────────────────────────────────────────

/// Backend-agnostic interface for prompt versioning and A/B experiment
/// management.
///
/// Covers the call sites in `kernel/mod.rs` that previously reached
/// `self.prompt_store.get().unwrap().{method}` directly on the SQLite
/// `PromptStore`.
pub trait PromptBackend: Send + Sync {
    /// Create a new prompt version.
    fn create_version(&self, version: PromptVersion) -> LibreFangResult<()>;

    /// List all prompt versions for an agent, ordered by version descending.
    fn list_versions(&self, agent_id: PromptAgentId) -> LibreFangResult<Vec<PromptVersion>>;

    /// Retrieve a single prompt version by UUID.
    fn get_version(&self, id: Uuid) -> LibreFangResult<Option<PromptVersion>>;

    /// Retrieve the currently-active prompt version for an agent.
    fn get_active_version(&self, agent_id: PromptAgentId)
        -> LibreFangResult<Option<PromptVersion>>;

    /// Set the active version for an agent (clears any previous active flag).
    fn set_active_version(&self, id: Uuid, agent_id: PromptAgentId) -> LibreFangResult<()>;

    /// Delete a prompt version by UUID.
    fn delete_version(&self, id: Uuid) -> LibreFangResult<()>;

    /// Remove versions beyond `max_versions` (oldest first).
    fn prune_old_versions(&self, agent_id: PromptAgentId, max_versions: u32)
        -> LibreFangResult<()>;

    /// Create a version only when the system prompt content has changed.
    /// Returns `true` if a new version was created, `false` if unchanged.
    fn create_version_if_changed(
        &self,
        agent_id: PromptAgentId,
        system_prompt: &str,
        created_by: &str,
    ) -> LibreFangResult<bool>;

    /// Latest version number for an agent (0 if none).
    fn get_latest_version_number(&self, agent_id: PromptAgentId) -> LibreFangResult<u32>;

    /// Create a new A/B experiment.
    fn create_experiment(&self, experiment: PromptExperiment) -> LibreFangResult<()>;

    /// List all experiments for an agent.
    fn list_experiments(&self, agent_id: PromptAgentId) -> LibreFangResult<Vec<PromptExperiment>>;

    /// Retrieve a single experiment by UUID.
    fn get_experiment(&self, id: Uuid) -> LibreFangResult<Option<PromptExperiment>>;

    /// Update the status of an experiment.
    fn update_experiment_status(&self, id: Uuid, status: ExperimentStatus) -> LibreFangResult<()>;

    /// Get the running experiment for an agent, if any.
    fn get_running_experiment(
        &self,
        agent_id: PromptAgentId,
    ) -> LibreFangResult<Option<PromptExperiment>>;

    /// Record a request against an experiment variant.
    fn record_request(
        &self,
        experiment_id: Uuid,
        variant_id: Uuid,
        latency_ms: u64,
        cost_usd: f64,
        success: bool,
    ) -> LibreFangResult<()>;

    /// Retrieve metrics for a single experiment variant (by variant_id).
    fn get_variant_metrics(
        &self,
        variant_id: Uuid,
    ) -> LibreFangResult<Option<ExperimentVariantMetrics>>;

    /// Retrieve metrics for all variants in an experiment.
    fn get_experiment_metrics(
        &self,
        experiment_id: Uuid,
    ) -> LibreFangResult<Vec<ExperimentVariantMetrics>>;
}

// ── KnowledgeBackend ──────────────────────────────────────────────────────────

/// Backend-agnostic interface for the knowledge graph (entities + relations).
///
/// The kernel calls `add_entity`, `add_relation`, and `query_graph` through
/// the `Memory` trait on `MemorySubstrate`.  This trait provides the same
/// surface so a `SurrealKnowledgeBackend` can replace the SQLite path when
/// `surreal-backend` is enabled.
#[async_trait]
pub trait KnowledgeBackend: Send + Sync {
    /// Add (or upsert) an entity.  Returns the entity's ID.
    async fn add_entity(&self, entity: Entity) -> LibreFangResult<String>;

    /// Add a relation between two entities.  Returns the relation's ID.
    async fn add_relation(&self, relation: Relation) -> LibreFangResult<String>;

    /// Query the graph with a pattern filter.
    async fn query_graph(&self, pattern: GraphPattern) -> LibreFangResult<Vec<GraphMatch>>;

    /// Delete all entities and relations belonging to an agent.
    async fn delete_by_agent(&self, agent_id: &str) -> LibreFangResult<u64>;
}

#[async_trait]
impl KnowledgeBackend for MemorySubstrate {
    async fn add_entity(&self, entity: Entity) -> LibreFangResult<String> {
        // Delegate to the Memory trait impl which calls KnowledgeStore::add_entity
        librefang_types::memory::Memory::add_entity(self, entity).await
    }

    async fn add_relation(&self, relation: Relation) -> LibreFangResult<String> {
        librefang_types::memory::Memory::add_relation(self, relation).await
    }

    async fn query_graph(&self, pattern: GraphPattern) -> LibreFangResult<Vec<GraphMatch>> {
        librefang_types::memory::Memory::query_graph(self, pattern).await
    }

    async fn delete_by_agent(&self, agent_id: &str) -> LibreFangResult<u64> {
        let store = self.knowledge().clone();
        let agent_id = agent_id.to_string();
        tokio::task::spawn_blocking(move || store.delete_by_agent(&agent_id))
            .await
            .map_err(|e| librefang_types::error::LibreFangError::Internal(e.to_string()))?
    }
}

impl PromptBackend for crate::prompt::PromptStore {
    fn create_version(&self, version: PromptVersion) -> LibreFangResult<()> {
        crate::prompt::PromptStore::create_version(self, version)
    }

    fn list_versions(&self, agent_id: PromptAgentId) -> LibreFangResult<Vec<PromptVersion>> {
        crate::prompt::PromptStore::list_versions(self, agent_id)
    }

    fn get_version(&self, id: Uuid) -> LibreFangResult<Option<PromptVersion>> {
        crate::prompt::PromptStore::get_version(self, id)
    }

    fn get_active_version(
        &self,
        agent_id: PromptAgentId,
    ) -> LibreFangResult<Option<PromptVersion>> {
        crate::prompt::PromptStore::get_active_version(self, agent_id)
    }

    fn set_active_version(&self, id: Uuid, agent_id: PromptAgentId) -> LibreFangResult<()> {
        crate::prompt::PromptStore::set_active_version(self, id, agent_id)
    }

    fn delete_version(&self, id: Uuid) -> LibreFangResult<()> {
        crate::prompt::PromptStore::delete_version(self, id)
    }

    fn prune_old_versions(
        &self,
        agent_id: PromptAgentId,
        max_versions: u32,
    ) -> LibreFangResult<()> {
        crate::prompt::PromptStore::prune_old_versions(self, agent_id, max_versions)
    }

    fn create_version_if_changed(
        &self,
        agent_id: PromptAgentId,
        system_prompt: &str,
        created_by: &str,
    ) -> LibreFangResult<bool> {
        crate::prompt::PromptStore::create_version_if_changed(
            self,
            agent_id,
            system_prompt,
            created_by,
        )
    }

    fn get_latest_version_number(&self, agent_id: PromptAgentId) -> LibreFangResult<u32> {
        crate::prompt::PromptStore::get_latest_version_number(self, agent_id)
    }

    fn create_experiment(&self, experiment: PromptExperiment) -> LibreFangResult<()> {
        crate::prompt::PromptStore::create_experiment(self, experiment)
    }

    fn list_experiments(&self, agent_id: PromptAgentId) -> LibreFangResult<Vec<PromptExperiment>> {
        crate::prompt::PromptStore::list_experiments(self, agent_id)
    }

    fn get_experiment(&self, id: Uuid) -> LibreFangResult<Option<PromptExperiment>> {
        crate::prompt::PromptStore::get_experiment(self, id)
    }

    fn update_experiment_status(&self, id: Uuid, status: ExperimentStatus) -> LibreFangResult<()> {
        crate::prompt::PromptStore::update_experiment_status(self, id, status)
    }

    fn get_running_experiment(
        &self,
        agent_id: PromptAgentId,
    ) -> LibreFangResult<Option<PromptExperiment>> {
        crate::prompt::PromptStore::get_running_experiment(self, agent_id)
    }

    fn record_request(
        &self,
        experiment_id: Uuid,
        variant_id: Uuid,
        latency_ms: u64,
        cost_usd: f64,
        success: bool,
    ) -> LibreFangResult<()> {
        crate::prompt::PromptStore::record_request(
            self,
            experiment_id,
            variant_id,
            latency_ms,
            cost_usd,
            success,
        )
    }

    fn get_variant_metrics(
        &self,
        variant_id: Uuid,
    ) -> LibreFangResult<Option<ExperimentVariantMetrics>> {
        crate::prompt::PromptStore::get_variant_metrics(self, variant_id)
    }

    fn get_experiment_metrics(
        &self,
        experiment_id: Uuid,
    ) -> LibreFangResult<Vec<ExperimentVariantMetrics>> {
        crate::prompt::PromptStore::get_experiment_metrics(self, experiment_id)
    }
}

// ── SemanticBackend ───────────────────────────────────────────────────────────

/// Backend-agnostic interface for semantic (vector / text) memory storage.
///
/// Abstracting over this surface lets the kernel and runtime select the correct
/// persistence layer at boot time — SurrealDB (HNSW + BM25 hybrid) when
/// `surreal-backend` is compiled in, or the SQLite [`crate::MemorySubstrate`]
/// path for upstream compatibility.
///
/// ## Implementations
///
/// | Type | Feature flag | Engine |
/// |------|-------------|--------|
/// | [`crate::MemorySubstrate`] | always | SQLite LIKE + in-process cosine |
/// | `SurrealSemanticBackend` | `surreal-backend` | SurrealDB HNSW v5 + BM25 hybrid |
#[async_trait]
pub trait SemanticBackend: Send + Sync {
    /// Store a memory fragment.
    ///
    /// An optional pre-computed `embedding` may be provided. When absent the
    /// implementation stores the content and defers embedding to a later pass.
    async fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<Vec<f32>>,
    ) -> LibreFangResult<MemoryId>;

    /// Semantic search.
    ///
    /// When `query_embedding` is `Some`, implementations should prefer
    /// approximate-nearest-neighbour (ANN) vector search. When `None`, fall
    /// back to full-text / BM25 search.
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<Vec<f32>>,
    ) -> LibreFangResult<Vec<MemoryFragment>>;

    /// Soft-delete a memory by ID. Returns `true` if a row was removed.
    async fn forget(&self, id: MemoryId) -> LibreFangResult<bool>;

    /// Count memories matching `filter`.
    async fn count(&self, filter: MemoryFilter) -> LibreFangResult<u64>;

    /// Touch the access timestamp and increment the access counter.
    async fn update_access(&self, id: MemoryId) -> LibreFangResult<()>;

    /// Return the name of this backend (e.g. `"surreal"`, `"sqlite"`).
    fn backend_name(&self) -> &str;
}

// ── SemanticBackend impl for MemorySubstrate (SQLite compatibility path) ──────

#[async_trait]
impl SemanticBackend for MemorySubstrate {
    async fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<Vec<f32>>,
    ) -> LibreFangResult<MemoryId> {
        self.remember_with_embedding_async(
            agent_id,
            content,
            source,
            scope,
            metadata,
            embedding.as_deref(),
        )
        .await
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<Vec<f32>>,
    ) -> LibreFangResult<Vec<MemoryFragment>> {
        self.recall_with_embedding_async(query, limit, filter, query_embedding.as_deref())
            .await
    }

    async fn forget(&self, id: MemoryId) -> LibreFangResult<bool> {
        let store = self.semantic_store_clone();
        tokio::task::spawn_blocking(move || store.forget(id).map(|_| true))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn count(&self, filter: MemoryFilter) -> LibreFangResult<u64> {
        let store = self.semantic_store_clone();
        tokio::task::spawn_blocking(move || store.count_by_filter(filter))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    async fn update_access(&self, id: MemoryId) -> LibreFangResult<()> {
        let store = self.semantic_store_clone();
        tokio::task::spawn_blocking(move || store.update_access(id))
            .await
            .map_err(|e| LibreFangError::Internal(e.to_string()))?
    }

    fn backend_name(&self) -> &str {
        "sqlite"
    }
}
