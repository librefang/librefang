//! Accessors / lifecycle helpers spun off from the giant `impl LibreFangKernel`
//! in `kernel::mod` — see Phase 3a of #4713. Hosts the public-facade getters
//! (config, home dir, registry, scheduler, …), per-subsystem refs used by the
//! API crate, lazily-cached vault helpers, the periodic GC sweep, and the
//! background sweep-task spawners (approval expiry, task-board claim TTL,
//! session stream hub idle GC).
//!
//! The block is a sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery (Rust scopes private items to the module of declaration
//! *and its descendants*).

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::subsystems::{
    AgentSubsystemApi, EventSubsystemApi, GovernanceSubsystemApi, LlmSubsystemApi, McpSubsystemApi,
    MediaSubsystemApi, MemorySubsystemApi, MeshSubsystemApi, MeteringSubsystemApi,
    ProcessSubsystemApi, SecuritySubsystemApi, SkillsSubsystemApi, WorkflowSubsystemApi,
};

use tracing::{debug, info, warn};

use librefang_memory::MemorySubstrate;
use librefang_runtime::audit::AuditLog;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::config::KernelConfig;
use librefang_types::error::LibreFangError;
use librefang_types::tool::{AgentLoopSignal, ToolDefinition};

use crate::auth::AuthManager;
use crate::error::{KernelError, KernelResult};
use crate::event_bus::EventBus;
use crate::metering::MeteringEngine;
use crate::registry::AgentRegistry;
use crate::scheduler::AgentScheduler;
use crate::supervisor::Supervisor;
use crate::triggers::TriggerEngine;
use crate::workflow::{WorkflowEngine, WorkflowTemplateRegistry};

use super::workspace_setup::migrate_legacy_agent_dirs;
use super::{DeliveryTracker, LibreFangKernel};

impl LibreFangKernel {
    /// Full kernel configuration (atomically loaded snapshot).
    #[inline]
    pub fn config_ref(&self) -> arc_swap::Guard<std::sync::Arc<KernelConfig>> {
        self.config.load()
    }

    /// Snapshot of current config — use when holding config across `.await` points.
    pub fn config_snapshot(&self) -> std::sync::Arc<KernelConfig> {
        self.config.load_full()
    }

    /// Return a snapshot of the current budget configuration. Delegates
    /// to [`MeteringSubsystem::current_budget`].
    pub fn budget_config(&self) -> librefang_types::config::BudgetConfig {
        self.metering.current_budget()
    }

    /// Safely mutate the runtime budget configuration. Delegates to
    /// [`MeteringSubsystem::update_budget`].
    pub fn update_budget_config(&self, f: impl Fn(&mut librefang_types::config::BudgetConfig)) {
        self.metering.update_budget(f);
    }

    /// LibreFang home directory path (boot-time immutable).
    #[inline]
    pub fn home_dir(&self) -> &Path {
        &self.home_dir_boot
    }

    /// Snapshot the inbox subsystem's status (config + on-disk file counts).
    ///
    /// Provided as a kernel-surface method so API callers do not need to reach
    /// into the `librefang_kernel::inbox` module directly. See issue #3744.
    pub fn inbox_status(&self) -> crate::inbox::InboxStatus {
        let cfg = self.config_ref();
        crate::inbox::inbox_status(&cfg.inbox, self.home_dir())
    }

    /// Snapshot of the auto-dream subsystem's status (global config + per-agent
    /// rows) for the dashboard `/api/auto-dream/status` endpoint.
    ///
    /// Provided as a kernel-surface method so API callers do not need to reach
    /// into the `librefang_kernel::auto_dream` module directly. See issue #3744.
    pub async fn auto_dream_status(&self) -> crate::auto_dream::AutoDreamStatus {
        crate::auto_dream::current_status(self).await
    }

    /// Manually fire an auto-dream consolidation for `agent_id`, bypassing
    /// time and session gates but respecting the per-agent dream lock.
    ///
    /// Provided as a kernel-surface method so API callers do not need to reach
    /// into the `librefang_kernel::auto_dream` module directly. See issue #3744.
    pub async fn auto_dream_trigger_manual(
        self: std::sync::Arc<Self>,
        agent_id: librefang_types::agent::AgentId,
    ) -> crate::auto_dream::TriggerOutcome {
        crate::auto_dream::trigger_manual(self, agent_id).await
    }

    /// Abort an in-flight manual auto-dream for `agent_id`. Scheduled dreams
    /// cannot be aborted.
    ///
    /// Provided as a kernel-surface method so API callers do not need to reach
    /// into the `librefang_kernel::auto_dream` module directly. See issue #3744.
    pub async fn auto_dream_abort(
        &self,
        agent_id: librefang_types::agent::AgentId,
    ) -> crate::auto_dream::AbortOutcome {
        crate::auto_dream::abort_dream(agent_id).await
    }

    /// Toggle an agent's `auto_dream_enabled` opt-in flag. Returns `Err` if
    /// the agent doesn't exist; the scheduler picks up the change on its
    /// next tick.
    ///
    /// Provided as a kernel-surface method so API callers do not need to reach
    /// into the `librefang_kernel::auto_dream` module directly. See issue #3744.
    pub fn auto_dream_set_enabled(
        &self,
        agent_id: librefang_types::agent::AgentId,
        enabled: bool,
    ) -> librefang_types::error::LibreFangResult<()> {
        crate::auto_dream::set_agent_enabled(self, agent_id, enabled)
    }

    /// Build a redacted trajectory bundle for an agent's session.
    ///
    /// Encapsulates `librefang_kernel::trajectory` (exporter + policy + agent
    /// context) so API callers do not need to import that module directly.
    /// Sessions are persisted lazily on first message; if the session row is
    /// missing but the requested ID matches the agent's currently-registered
    /// session, an empty bundle is returned instead of a not-found error.
    /// See issue #3744.
    pub fn export_session_trajectory(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<crate::trajectory::TrajectoryBundle> {
        use crate::trajectory::{AgentContext, RedactionPolicy, TrajectoryExporter};

        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // Build redaction policy. Use the agent's workspace as the
        // path-collapse root when present.
        let mut policy = RedactionPolicy::default();
        if let Some(ws) = entry.manifest.workspace.clone() {
            policy = policy.with_workspace_root(ws);
        }

        let exporter = TrajectoryExporter::new(self.memory.substrate.clone(), policy);
        let agent_ctx = AgentContext {
            name: entry.name.clone(),
            model: entry.manifest.model.model.clone(),
            provider: entry.manifest.model.provider.clone(),
            system_prompt: entry.manifest.model.system_prompt.clone(),
        };

        match self.memory.substrate.get_session(session_id) {
            Ok(None) if session_id == entry.session_id => {
                Ok(exporter.empty_bundle(agent_id, session_id, agent_ctx))
            }
            Ok(_) => exporter
                .export_session(agent_id, session_id, agent_ctx)
                .map_err(KernelError::LibreFang),
            Err(e) => Err(KernelError::LibreFang(e)),
        }
    }

    /// Validate a `KernelConfig` candidate for hot-reload eligibility.
    ///
    /// Provided as a kernel-surface method so API callers do not need to
    /// reach into the `librefang_kernel::config_reload` module directly.
    /// See issue #3744.
    pub fn validate_config_for_reload(
        &self,
        config: &librefang_types::config::KernelConfig,
    ) -> Result<(), Vec<String>> {
        crate::config_reload::validate_config_for_reload(config)
    }

    /// Build the roots list for a specific MCP server config.
    ///
    /// Starts with the default roots (workspaces directory) and, for stdio
    /// servers, appends any absolute-path arguments the user configured.
    /// This ensures that filesystem-aware MCP servers (e.g.
    /// `@modelcontextprotocol/server-filesystem`) receive the directories
    /// explicitly passed in their args — such as `/mnt/obsidian` — rather
    /// than being silently restricted to the agent workspace.
    pub(super) fn mcp_roots_for_server(
        &self,
        server_config: &librefang_types::config::McpServerConfigEntry,
    ) -> Vec<String> {
        use librefang_types::config::McpTransportEntry;
        let mut roots = self.default_mcp_roots();
        if let Some(McpTransportEntry::Stdio { args, .. }) = &server_config.transport {
            for arg in args {
                let p = std::path::Path::new(arg.as_str());
                if p.is_absolute() && !roots.contains(arg) {
                    roots.push(arg.clone());
                }
            }
        }
        roots
    }

    /// Hand out an [`Arc::clone`] of the kernel's live taint-rules swap to a
    /// fresh `McpServerConfig`. All connected servers share the same swap,
    /// so `[[taint_rules]]` edits applied via [`Self::reload_config`]
    /// immediately reach every server's next scan call. The scanner takes a
    /// single `.load()` per tool call to keep the rule view stable across a
    /// single argument-tree walk.
    pub(super) fn snapshot_taint_rules(&self) -> librefang_runtime::mcp::TaintRuleSetsHandle {
        std::sync::Arc::clone(&self.taint_rules_swap)
    }

    /// Build the default list of root directories to advertise to MCP servers
    /// via the MCP Roots capability.
    ///
    /// Includes the librefang home directory and the agent workspaces directory
    /// so that filesystem-aware MCP servers (e.g. morphllm, filesystem) know
    /// which paths they are allowed to operate on without needing hard-coded
    /// allowed-directories in their own server args.
    fn default_mcp_roots(&self) -> Vec<String> {
        // Advertise only the workspaces directory, not the entire home dir.
        // Scoping roots to workspaces_dir means per-agent pools are actually
        // created for agent-specific workspaces (which are subdirectories of
        // workspaces_dir), giving MCP servers an appropriately narrow view.
        // Advertising home_dir would cause every agent workspace to be "already
        // covered", silently disabling per-agent workspace scoping.
        let mut roots = Vec::new();
        let workspaces = self.config.load().effective_workspaces_dir();
        // Use to_str() rather than to_string_lossy() so that non-UTF-8 paths
        // are silently skipped instead of being silently corrupted (U+FFFD).
        if let Some(ws) = workspaces.to_str() {
            roots.push(ws.to_owned());
        }
        roots
    }

    /// Create a fresh, per-execution MCP connection pool for a single agent run.
    ///
    /// Adds `agent_workspace` to the default roots so filesystem-aware MCP
    /// servers (morphllm, filesystem, …) scope their access to the agent's
    /// specific working directory rather than the broad workspace base.
    ///
    /// Returns `None` — and callers fall back to the daemon-global pool — when:
    /// - no MCP servers are configured,
    /// - `agent_workspace` is `None` (no workspace to scope),
    /// - the workspace is already a sub-path of an existing default root
    ///   (per-agent pool would be identical to the global pool), or
    /// - all per-agent connections fail.
    pub(super) async fn build_agent_mcp_pool(
        &self,
        agent_workspace: Option<&std::path::Path>,
    ) -> Option<tokio::sync::Mutex<Vec<librefang_runtime::mcp::McpConnection>>> {
        use librefang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
        use librefang_types::config::McpTransportEntry;

        let servers = self
            .mcp
            .effective_mcp_servers
            .read()
            .map(|s| s.clone())
            .unwrap_or_default();

        if servers.is_empty() {
            return None;
        }

        let mut roots = self.default_mcp_roots();

        // Add the agent workspace only when it genuinely extends the default
        // roots.  Use Path::starts_with (component-level comparison) rather
        // than str::starts_with so that "/project2" is not mistakenly treated
        // as a sub-path of "/project".
        //
        // When there is no workspace, or when the workspace is already covered,
        // the roots would be identical to those in the daemon-global pool —
        // creating a fresh per-agent pool would be pure overhead.
        match agent_workspace {
            None => return None,
            Some(ws) => {
                let already_covered = roots
                    .iter()
                    .any(|r| ws.starts_with(std::path::Path::new(r)));
                if already_covered {
                    return None;
                }
                // Use to_str() for consistency with default_mcp_roots(); non-UTF-8
                // workspace paths fall back to the global pool.
                let ws_str = match ws.to_str() {
                    Some(s) => s.to_owned(),
                    None => return None,
                };
                if !roots.contains(&ws_str) {
                    roots.push(ws_str);
                }
            }
        }

        let mut connections = Vec::new();
        for server_config in &servers {
            let transport_entry = match &server_config.transport {
                Some(t) => t,
                None => {
                    tracing::warn!(name = %server_config.name, "MCP server has no transport configured, skipping");
                    continue;
                }
            };
            let transport = match transport_entry {
                McpTransportEntry::Stdio { command, args } => McpTransport::Stdio {
                    command: command.clone(),
                    args: args.clone(),
                },
                McpTransportEntry::Sse { url } => McpTransport::Sse { url: url.clone() },
                McpTransportEntry::Http { url } => McpTransport::Http { url: url.clone() },
                McpTransportEntry::HttpCompat {
                    base_url,
                    headers,
                    tools,
                } => McpTransport::HttpCompat {
                    base_url: base_url.clone(),
                    headers: headers.clone(),
                    tools: tools.clone(),
                },
            };

            // Merge agent workspace into server-specific roots.
            let mut server_roots = self.mcp_roots_for_server(server_config);
            for r in &roots {
                if !server_roots.contains(r) {
                    server_roots.push(r.clone());
                }
            }

            let mcp_config = McpServerConfig {
                name: server_config.name.clone(),
                transport,
                timeout_secs: server_config.timeout_secs,
                env: server_config.env.clone(),
                headers: server_config.headers.clone(),
                oauth_provider: Some(self.oauth_provider_ref()),
                oauth_config: server_config.oauth.clone(),
                taint_scanning: server_config.taint_scanning,
                taint_policy: server_config.taint_policy.clone(),
                taint_rule_sets: self.snapshot_taint_rules(),
                roots: server_roots,
            };

            match McpConnection::connect(mcp_config).await {
                Ok(conn) => connections.push(conn),
                Err(e) => warn!(
                    server = %server_config.name,
                    error = %e,
                    "Per-agent MCP connection failed; agent will lack this server's tools"
                ),
            }
        }

        if connections.is_empty() {
            None
        } else {
            Some(tokio::sync::Mutex::new(connections))
        }
    }

    /// Relocate any legacy `<home>/agents/<name>/` directories into the
    /// canonical `workspaces/agents/<name>/` layout. This is the same pass
    /// that runs at boot and is exposed so runtime flows (e.g. the migrate
    /// API route) can trigger it without requiring a daemon restart.
    pub fn relocate_legacy_agent_dirs(&self) {
        let workspaces_agents = self.config.load().effective_agent_workspaces_dir();
        migrate_legacy_agent_dirs(&self.home_dir_boot, &workspaces_agents);
    }

    /// Data directory path (boot-time immutable).
    #[inline]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir_boot
    }

    /// Default LLM model configuration.
    #[inline]
    pub fn default_model(&self) -> librefang_types::config::DefaultModelConfig {
        self.config.load().default_model.clone()
    }

    /// Agent registry. Delegates to [`AgentSubsystem::registry_ref`].
    #[inline]
    pub fn agent_registry(&self) -> &AgentRegistry {
        self.agents.registry_ref()
    }

    /// Canonical agent UUID registry (refs #4614). Delegates to
    /// [`AgentSubsystem::identities_ref`].
    #[inline]
    pub fn agent_identities(&self) -> &Arc<crate::agent_identity_registry::AgentIdentityRegistry> {
        self.agents.identities_ref()
    }

    /// Memory substrate. Delegates to
    /// [`MemorySubsystem::substrate_ref`].
    #[inline]
    pub fn memory_substrate(&self) -> &Arc<MemorySubstrate> {
        self.memory.substrate_ref()
    }

    /// Proactive memory store. Delegates to
    /// [`MemorySubsystem::proactive_store`].
    #[inline]
    pub fn proactive_memory_store(&self) -> Option<&Arc<librefang_memory::ProactiveMemoryStore>> {
        self.memory.proactive_store()
    }

    /// Merkle hash chain audit trail.
    #[inline]
    pub fn audit(&self) -> &Arc<AuditLog> {
        self.metering.audit_log()
    }

    /// Cost metering engine. Delegates to
    /// [`MeteringSubsystem::engine`].
    #[inline]
    pub fn metering_ref(&self) -> &Arc<MeteringEngine> {
        self.metering.engine()
    }

    /// Agent scheduler. Delegates to [`AgentSubsystem::scheduler_ref`].
    #[inline]
    pub fn scheduler_ref(&self) -> &AgentScheduler {
        self.agents.scheduler_ref()
    }

    /// Model catalog (`ArcSwap` since #3384). Delegates to
    /// [`LlmSubsystem::catalog_swap`].
    #[inline]
    pub fn model_catalog_ref(
        &self,
    ) -> &arc_swap::ArcSwap<librefang_runtime::model_catalog::ModelCatalog> {
        self.llm.catalog_swap()
    }

    /// Snapshot the current model catalog. Delegates to
    /// [`LlmSubsystem::catalog_load`].
    #[inline]
    pub fn model_catalog_load(
        &self,
    ) -> arc_swap::Guard<Arc<librefang_runtime::model_catalog::ModelCatalog>> {
        self.llm.catalog_load()
    }

    /// Atomically mutate the model catalog using the RCU pattern: clone the
    /// current snapshot, hand the closure a `&mut` to the clone, and store
    /// the result. Used by API/probe paths that previously held a write
    /// lock. Concurrent updates serialize correctly via the underlying CAS
    /// loop in `arc_swap::ArcSwap::rcu`.
    ///
    /// The closure may run multiple times under contention, so it must be
    /// idempotent on `cat`. The returned `R` reflects the **final** (winning)
    /// attempt — useful for surfacing booleans like
    /// `add_alias`/`remove_alias`/`add_custom_model` to the caller.
    /// Delegates to [`LlmSubsystem::catalog_update`].
    pub fn model_catalog_update<F, R>(&self, f: F) -> R
    where
        F: FnMut(&mut librefang_runtime::model_catalog::ModelCatalog) -> R,
    {
        self.llm.catalog_update(f)
    }

    /// Spawn background tasks to validate API keys for every `Configured` provider.
    ///
    /// Called at daemon boot and whenever a new key is set via the dashboard.
    /// Results (ValidatedKey / InvalidKey) are written back into the catalog.
    pub fn spawn_key_validation(self: Arc<Self>) {
        use librefang_types::model_catalog::AuthStatus;

        let to_validate = self.llm.model_catalog.load().providers_needing_validation();

        if to_validate.is_empty() {
            return;
        }

        tokio::spawn(async move {
            let handles: Vec<_> = to_validate
                .into_iter()
                .map(|(id, base_url, key_env)| {
                    let kernel = Arc::clone(&self);
                    tokio::spawn(async move {
                        // Resolve the actual key via primary env var, alt env var,
                        // and credential files. This is needed for AutoDetected
                        // providers whose key lives in a fallback env var (e.g.
                        // GOOGLE_API_KEY for gemini, not GEMINI_API_KEY).
                        let key = librefang_runtime::drivers::resolve_provider_api_key(&id)
                            .or_else(|| {
                                std::env::var(&key_env)
                                    .ok()
                                    .filter(|k| !k.trim().is_empty())
                            })
                            .unwrap_or_default();
                        if key.is_empty() {
                            return;
                        }
                        let result =
                            librefang_runtime::model_catalog::probe_api_key(&id, &base_url, &key)
                                .await;
                        if let Some(valid) = result.key_valid {
                            let status = if valid {
                                AuthStatus::ValidatedKey
                            } else {
                                AuthStatus::InvalidKey
                            };
                            tracing::info!(provider = %id, valid, "provider key validation result");
                            let available_models = result.available_models.clone();
                            kernel.model_catalog_update(|catalog| {
                                catalog.set_provider_auth_status(&id, status);
                                // Store available models so downstream can check
                                // whether a configured model actually exists.
                                if !available_models.is_empty() {
                                    catalog.set_provider_available_models(
                                        &id,
                                        available_models.clone(),
                                    );
                                }
                            });
                        }
                    })
                })
                .collect();
            futures::future::join_all(handles).await;
        });
    }

    /// Invalidate all cached LLM drivers so the next request rebuilds them
    /// with current provider URLs / API keys. Delegates to
    /// [`LlmSubsystem::clear_driver_cache`].
    #[inline]
    pub fn clear_driver_cache(&self) {
        self.llm.clear_driver_cache();
    }

    /// Spawn the approval expiry sweep task.
    ///
    /// This periodically checks for expired pending approval requests and
    /// handles their resolution (e.g., timing out deferred tool executions).
    pub fn spawn_approval_sweep_task(self: Arc<Self>) {
        let handle = tokio::runtime::Handle::current();
        if self
            .governance
            .approval_sweep_started
            .swap(true, Ordering::AcqRel)
        {
            debug!("Approval expiry sweep task already running");
            return;
        }

        let kernel = Arc::clone(&self);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        handle.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let (escalated, expired) = kernel.governance.approval_manager.expire_pending_requests();
                        for escalated_req in escalated {
                            kernel
                                .notify_escalated_approval(&escalated_req.request, escalated_req.request_id)
                                .await;
                        }
                        for (request_id, decision, deferred) in expired {
                            kernel.handle_approval_resolution(
                                request_id, decision, deferred
                            ).await;
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
            kernel
                .governance.approval_sweep_started
                .store(false, Ordering::Release);
            tracing::debug!("Approval expiry sweep task stopped");
        });
    }

    /// Spawn the task-board stuck-task sweep loop (issue #2923 / #2926).
    ///
    /// Periodically scans the `task_queue` for `in_progress` rows whose
    /// `claimed_at` is older than `config.task_board.claim_ttl_secs`. Stuck
    /// tasks are flipped back to `pending` and their `assigned_to` is cleared
    /// so another worker (or the same one on the next trigger fire) can pick
    /// them up.
    ///
    /// Idempotent: re-calling while the loop is already running is a no-op.
    /// The interval and TTL are read *live* from the kernel config on every
    /// tick, so hot-reloading `[task_board]` does not require a kernel
    /// restart. `claim_ttl_secs = 0` disables the sweeper (tick is a no-op)
    /// for deployments that legitimately hold tasks `in_progress` for hours
    /// (human-in-the-loop workflows).
    pub fn spawn_task_board_sweep_task(self: Arc<Self>) {
        let handle = tokio::runtime::Handle::current();
        if self
            .governance
            .task_board_sweep_started
            .swap(true, Ordering::AcqRel)
        {
            debug!("Task board sweep task already running");
            return;
        }

        let kernel = Arc::clone(&self);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        handle.spawn(async move {
            loop {
                // Read sweeper knobs live — hot reload takes effect on next tick.
                let (interval_secs, ttl_secs, max_retries) = {
                    let cfg = kernel.config.load();
                    (
                        cfg.task_board.sweep_interval_secs.max(1),
                        cfg.task_board.claim_ttl_secs,
                        cfg.task_board.max_retries,
                    )
                };

                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs)) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                        continue;
                    }
                }

                if ttl_secs == 0 {
                    // Sweeper disabled by operator — keep the loop alive so a
                    // later hot-reload can flip it back on without restart.
                    continue;
                }

                match kernel
                    .memory
                    .substrate
                    .task_reset_stuck(ttl_secs, max_retries)
                    .await
                {
                    Ok(reset) if !reset.is_empty() => {
                        warn!(
                            count = reset.len(),
                            ttl_secs,
                            task_ids = ?reset,
                            "Auto-reset stuck in_progress tasks past claim TTL (issue #2923)"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(error = %e, "Task board sweep failed");
                    }
                }
            }

            kernel
                .governance
                .task_board_sweep_started
                .store(false, Ordering::Release);
            tracing::debug!("Task board sweep task stopped");
        });
    }

    /// Spawn the session-stream-hub idle GC loop.
    ///
    /// `SessionStreamHub` lazily creates a broadcast sender per session on
    /// first publish or first attach. Without periodic pruning, the senders
    /// map grows unbounded under churn (many short-lived sessions, many
    /// reconnects). This task calls `gc_idle()` every 60s to drop entries
    /// with zero live receivers.
    ///
    /// Idempotent: re-calling while already running is a no-op.
    pub fn spawn_session_stream_hub_gc_task(self: Arc<Self>) {
        let handle = tokio::runtime::Handle::current();
        if self
            .events
            .session_stream_hub_gc_started
            .swap(true, Ordering::AcqRel)
        {
            debug!("Session stream hub GC task already running");
            return;
        }

        let kernel = Arc::clone(&self);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        handle.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            // Skip the immediate first tick — nothing to GC at boot.
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let pruned = kernel.events.session_stream_hub.gc_idle();
                        if pruned > 0 {
                            tracing::debug!(pruned, "Session stream hub GC pruned idle sessions");
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
            kernel
                .events
                .session_stream_hub_gc_started
                .store(false, Ordering::Release);
            tracing::debug!("Session stream hub GC task stopped");
        });
    }

    /// Skill registry (RwLock — hot-reload on install/uninstall).
    #[inline]
    pub fn skill_registry_ref(
        &self,
    ) -> &std::sync::RwLock<librefang_skills::registry::SkillRegistry> {
        self.skills.registry_ref()
    }

    /// Hand registry (curated autonomous capability packages).
    #[inline]
    pub fn hands(&self) -> &librefang_hands::registry::HandRegistry {
        self.skills.hand_registry_ref()
    }

    /// MCP catalog — returns the `ArcSwap` for lock-free reads.
    ///
    /// Prefer [`mcp_catalog_load`] for simple single-snapshot reads;
    /// use this accessor when you need the `ArcSwap` handle directly.
    #[inline]
    pub fn mcp_catalog(&self) -> &arc_swap::ArcSwap<librefang_extensions::catalog::McpCatalog> {
        self.mcp.catalog_swap()
    }

    /// Load a snapshot of the MCP catalog — lock-free, no blocking.
    ///
    /// The returned `Guard` holds a reference-counted snapshot; the catalog
    /// can be swapped concurrently without invalidating it.
    #[inline]
    pub fn mcp_catalog_load(
        &self,
    ) -> arc_swap::Guard<std::sync::Arc<librefang_extensions::catalog::McpCatalog>> {
        self.mcp.catalog_load()
    }

    /// Reload the MCP catalog from disk, replacing the current snapshot
    /// atomically via RCU. Readers in flight keep the old snapshot until
    /// they drop their `Guard`.
    pub fn mcp_catalog_reload(&self, home_dir: &std::path::Path) -> usize {
        let mut fresh = librefang_extensions::catalog::McpCatalog::new(home_dir);
        let count = fresh.load(home_dir);
        self.mcp.mcp_catalog.store(std::sync::Arc::new(fresh));
        count
    }

    /// MCP server health monitor.
    #[inline]
    pub fn mcp_health(&self) -> &librefang_extensions::health::HealthMonitor {
        self.mcp.health()
    }

    /// Cron job scheduler.
    #[inline]
    pub fn cron(&self) -> &crate::cron::CronScheduler {
        self.workflows.cron_ref()
    }

    /// Execution approval manager.
    #[inline]
    pub fn approvals(&self) -> &crate::approval::ApprovalManager {
        self.governance.approvals()
    }

    /// Lazily open and unlock the credential vault, caching the result for
    /// the lifetime of this kernel (#3598).
    ///
    /// The first call pays a single Argon2id KDF (inside `unlock()`) and
    /// reads `vault.enc` from disk; every subsequent call returns the cached
    /// `Arc<RwLock<…>>` with no I/O and no KDF. `vault_set` writes through
    /// the same handle and persists via `CredentialVault::set` →
    /// `save()` (that path still re-derives a per-write key — at-rest
    /// security is unchanged).
    ///
    /// Returns `Err(_)` only when the vault file exists but cannot be
    /// unlocked (bad master key, corrupt file, missing keyring entry).
    /// A missing vault file is **not** an error: the cache is populated
    /// with an unopened vault and the first `set()` call will `init()` it.
    pub(super) fn vault_handle(
        &self,
    ) -> Result<
        std::sync::Arc<std::sync::RwLock<librefang_extensions::vault::CredentialVault>>,
        String,
    > {
        // Fast path: cache already populated.
        if let Some(handle) = self.security.vault_cache.get() {
            return Ok(std::sync::Arc::clone(handle));
        }

        // Slow path: build the vault, unlock if it exists, install once.
        // OnceLock::set() losing a race is fine — both racers built an
        // equivalent unlocked vault; we just discard ours and use the
        // installed one. Argon2id runs at most a small bounded number of
        // times during the brief race window (in practice ≤ 2).
        let vault_path = self.home_dir_boot.join("vault.enc");
        let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
        if vault.exists() {
            vault
                .unlock()
                .map_err(|e| format!("Vault unlock failed: {e}"))?;
        }
        let handle = std::sync::Arc::new(std::sync::RwLock::new(vault));
        match self
            .security
            .vault_cache
            .set(std::sync::Arc::clone(&handle))
        {
            Ok(()) => Ok(handle),
            Err(_) => Ok(std::sync::Arc::clone(
                self.security.vault_cache.get().expect(
                    "OnceLock::set() returned Err; another thread must have installed a value",
                ),
            )),
        }
    }

    /// Read a secret from the encrypted vault.
    ///
    /// First call lazily unlocks the vault (one Argon2id KDF + one disk
    /// read) and caches the result on the kernel; subsequent calls — for
    /// any key — are pure in-memory `HashMap` lookups. See `vault_handle`
    /// and #3598.
    ///
    /// Returns `None` if the vault does not exist, cannot be unlocked, or
    /// the key is missing.
    pub fn vault_get(&self, key: &str) -> Option<String> {
        let handle = match self.vault_handle() {
            Ok(h) => h,
            Err(_) => return None,
        };
        let guard = handle.read().unwrap_or_else(|e| e.into_inner());
        if !guard.is_unlocked() {
            // Vault file did not exist when the cache was populated and no
            // `set()` has initialised it yet — nothing to read.
            return None;
        }
        guard.get(key).map(|s| s.to_string())
    }

    /// Write a secret to the encrypted vault.
    ///
    /// Uses the cached, already-unlocked vault when available (#3598) so
    /// the unlock-time Argon2id KDF runs at most once per kernel lifetime
    /// instead of once per call. The save-time KDF inside
    /// `CredentialVault::set` still runs on every write — at-rest
    /// security is unchanged. Creates the vault if it does not exist.
    pub fn vault_set(&self, key: &str, value: &str) -> Result<(), String> {
        let handle = self.vault_handle()?;
        let mut guard = handle.write().unwrap_or_else(|e| e.into_inner());
        if !guard.is_unlocked() {
            // Vault did not exist at cache-population time; create it now.
            guard
                .init()
                .map_err(|e| format!("Vault init failed: {e}"))?;
        }
        guard
            .set(key.to_string(), zeroize::Zeroizing::new(value.to_string()))
            .map_err(|e| format!("Vault write failed: {e}"))
    }

    /// Atomically redeem a TOTP recovery code.
    ///
    /// Acquires `vault_recovery_codes_mutex`, reads the stored code list,
    /// verifies `code`, removes it from the list, and writes back the
    /// updated list — all under the lock.  This prevents the TOCTOU race
    /// in issue #3560 where two concurrent requests could both succeed with
    /// the same code before either had written the updated (shortened) list.
    ///
    /// Returns:
    /// - `Ok(true)`  — code matched and was consumed (vault updated).
    /// - `Ok(false)` — code did not match (vault unchanged).
    /// - `Err(e)`    — vault read/write error, or vault_set failed (#3633).
    pub fn vault_redeem_recovery_code(&self, code: &str) -> Result<bool, String> {
        // Hold the mutex for the entire read-verify-write sequence.
        let _guard = self
            .security
            .vault_recovery_codes_mutex
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let stored = match self.vault_get("totp_recovery_codes") {
            Some(s) => s,
            None => return Err("No recovery codes configured".to_string()),
        };

        match crate::approval::ApprovalManager::verify_recovery_code(&stored, code) {
            Ok((true, updated)) => {
                // #3633: if the vault write fails, treat the attempt as failed
                // rather than granting access with a still-valid code.
                self.vault_set("totp_recovery_codes", &updated)
                    .map_err(|e| {
                        warn!("vault_set failed when consuming recovery code: {e}");
                        "Internal error persisting recovery code consumption".to_string()
                    })?;
                Ok(true)
            }
            Ok((false, _)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Workflow engine.
    #[inline]
    pub fn workflow_engine(&self) -> &WorkflowEngine {
        self.workflows.engine_ref()
    }

    /// Workflow template registry.
    #[inline]
    pub fn templates(&self) -> &WorkflowTemplateRegistry {
        self.workflows.templates_ref()
    }

    /// Convert a workflow into a reusable template.
    ///
    /// Thin wrapper around [`WorkflowEngine::workflow_to_template`] so that
    /// callers (e.g. `librefang-api`) do not need to import the engine type
    /// directly. See issue #3744 for the broader API/kernel decoupling effort.
    #[inline]
    pub fn workflow_to_template(
        &self,
        workflow: &crate::workflow::Workflow,
    ) -> librefang_types::workflow_template::WorkflowTemplate {
        WorkflowEngine::workflow_to_template(workflow)
    }

    /// Event-driven trigger engine.
    #[inline]
    pub fn trigger_engine(&self) -> &TriggerEngine {
        self.workflows.triggers_ref()
    }

    /// Process supervisor.
    #[inline]
    pub fn supervisor_ref(&self) -> &Supervisor {
        self.agents.supervisor_ref()
    }

    /// RBAC authentication manager.
    #[inline]
    pub fn auth_manager(&self) -> &AuthManager {
        self.security.auth_ref()
    }

    /// Device pairing manager.
    #[inline]
    pub fn pairing_ref(&self) -> &crate::pairing::PairingManager {
        self.security.pairing_ref()
    }

    /// Web tools context (search + fetch). Delegates to
    /// [`MediaSubsystem::web_tools`].
    #[inline]
    pub fn web_tools(&self) -> &librefang_runtime::web_search::WebToolsContext {
        self.media.web_tools()
    }

    /// Browser automation manager. Delegates to
    /// [`MediaSubsystem::browser`].
    #[inline]
    pub fn browser(&self) -> &librefang_runtime::browser::BrowserManager {
        self.media.browser()
    }

    /// Media understanding engine. Delegates to
    /// [`MediaSubsystem::engine`].
    #[inline]
    pub fn media(&self) -> &librefang_runtime::media_understanding::MediaEngine {
        self.media.engine()
    }

    /// Text-to-speech engine. Delegates to [`MediaSubsystem::tts`].
    #[inline]
    pub fn tts(&self) -> &librefang_runtime::tts::TtsEngine {
        self.media.tts()
    }

    /// Media generation driver cache (video, music, etc.). Delegates to
    /// [`MediaSubsystem::drivers`].
    #[inline]
    pub fn media_drivers(&self) -> &librefang_runtime::media::MediaDriverCache {
        self.media.drivers()
    }

    /// MCP server connections (Mutex — lazily initialized).
    #[inline]
    pub fn mcp_connections_ref(
        &self,
    ) -> &tokio::sync::Mutex<Vec<librefang_runtime::mcp::McpConnection>> {
        self.mcp.connections_ref()
    }

    /// Per-server MCP OAuth authentication states.
    #[inline]
    pub fn mcp_auth_states_ref(&self) -> &librefang_runtime::mcp_oauth::McpAuthStates {
        self.mcp.auth_states_ref()
    }

    /// Pluggable OAuth provider for MCP server auth flows.
    #[inline]
    pub fn oauth_provider_ref(
        &self,
    ) -> Arc<dyn librefang_runtime::mcp_oauth::McpOAuthProvider + Send + Sync> {
        Arc::clone(self.mcp.oauth_provider_ref())
    }

    /// MCP tool definitions cache.
    #[inline]
    pub fn mcp_tools_ref(&self) -> &std::sync::Mutex<Vec<ToolDefinition>> {
        self.mcp.tools_ref()
    }

    /// Effective MCP server list (config + extensions merged).
    #[inline]
    pub fn effective_mcp_servers_ref(
        &self,
    ) -> &std::sync::RwLock<Vec<librefang_types::config::McpServerConfigEntry>> {
        self.mcp.effective_servers_ref()
    }

    /// A2A task store.
    #[inline]
    pub fn a2a_tasks(&self) -> &librefang_runtime::a2a::A2aTaskStore {
        self.mesh.a2a_tasks()
    }

    /// Discovered external A2A agent cards.
    #[inline]
    pub fn a2a_agents(
        &self,
    ) -> &std::sync::Mutex<Vec<(String, librefang_runtime::a2a::AgentCard)>> {
        self.mesh.a2a_agents()
    }

    /// Delivery receipt tracker.
    #[inline]
    pub fn delivery(&self) -> &DeliveryTracker {
        self.mesh.delivery()
    }

    /// First currently-active `SessionInterrupt` registered for `agent_id`,
    /// across any of its sessions. Used by fork / subagent paths that just
    /// need a cancellation handle to chain off the parent — they don't care
    /// which specific session, only that aborting any one of the agent's
    /// in-flight loops cascades into them.
    ///
    /// If the agent has multiple concurrent loops the choice is unspecified
    /// (DashMap iteration order). Returns `None` when no loop is in flight.
    pub(crate) fn any_session_interrupt_for_agent(
        &self,
        agent_id: AgentId,
    ) -> Option<librefang_runtime::interrupt::SessionInterrupt> {
        self.agents
            .session_interrupts
            .iter()
            .find(|e| e.key().0 == agent_id)
            .map(|e| e.value().clone())
    }

    /// First currently-active `(parent_session_id, parent_interrupt)` pair
    /// for `agent_id`. Same DashMap-iteration-order semantics as
    /// [`Self::any_session_interrupt_for_agent`], but also returns the
    /// session key the interrupt was registered under so fork-spawn sites
    /// can pin themselves to the parent turn's actual session — rather
    /// than re-reading `entry.session_id`, which is a TOCTOU race against
    /// `switch_agent_session` (#4291).
    pub(crate) fn any_session_interrupt_with_id_for_agent(
        &self,
        agent_id: AgentId,
    ) -> Option<(SessionId, librefang_runtime::interrupt::SessionInterrupt)> {
        self.agents
            .session_interrupts
            .iter()
            .find(|e| e.key().0 == agent_id)
            .map(|e| (e.key().1, e.value().clone()))
    }

    /// Per-agent decision traces.
    #[inline]
    pub fn traces(&self) -> &dashmap::DashMap<AgentId, Vec<librefang_types::tool::DecisionTrace>> {
        self.agents.traces()
    }

    /// Channel adapters map.
    #[inline]
    pub fn channel_adapters_ref(
        &self,
    ) -> &dashmap::DashMap<String, Arc<dyn librefang_channels::types::ChannelAdapter>> {
        self.mesh.channel_adapters_ref()
    }

    /// Agent bindings for multi-account routing.
    #[inline]
    pub fn bindings_ref(&self) -> &std::sync::Mutex<Vec<librefang_types::config::AgentBinding>> {
        self.mesh.bindings_ref()
    }

    /// Broadcast configuration.
    #[inline]
    pub fn broadcast_ref(&self) -> &librefang_types::config::BroadcastConfig {
        self.mesh.broadcast_ref()
    }

    /// Uptime since kernel boot.
    #[inline]
    pub fn uptime(&self) -> std::time::Duration {
        self.booted_at.elapsed()
    }

    /// Embedding driver (None = text fallback).
    #[inline]
    pub fn embedding(
        &self,
    ) -> Option<&Arc<dyn librefang_runtime::embedding::EmbeddingDriver + Send + Sync>> {
        self.llm.embedding()
    }

    /// Command queue.
    #[inline]
    pub fn command_queue_ref(&self) -> &librefang_runtime::command_lane::CommandQueue {
        self.workflows.command_queue_ref()
    }

    /// Resolve the per-agent concurrency semaphore, lazily creating it on
    /// first use. Capacity comes from `AgentManifest.max_concurrent_invocations`,
    /// falling back to `KernelConfig.queue.concurrency.default_per_agent`,
    /// floored at 1 (covers a manifest typo of `Some(0)`). The returned
    /// `Arc<Semaphore>` is cheap to clone and safe to move into a
    /// spawned task via `acquire_owned()`.
    ///
    /// The semaphore is removed by `gc_sweep` only when the agent leaves
    /// the registry (kill / despawn). It is NOT invalidated on
    /// `manifest_swap` hot-reload — to pick up a new cap operators must
    /// kill the agent and let it respawn (or restart the daemon). An
    /// in-place activate / status flip that keeps the agent in the
    /// registry will silently retain the old capacity. This avoids a
    /// permit-loss race during live config reloads.
    pub(crate) fn agent_concurrency_for(&self, agent_id: AgentId) -> Arc<tokio::sync::Semaphore> {
        if let Some(existing) = self.agents.agent_concurrency.get(&agent_id) {
            return existing.clone();
        }
        // Single registry read so cap and session_mode come from the
        // same manifest snapshot — avoids a TOCTOU window where two
        // separate gets see manifests on either side of a swap.
        let (manifest_cap, session_mode) = match self.agents.registry.get(agent_id) {
            Some(e) => (
                e.manifest.max_concurrent_invocations.map(|n| n as usize),
                e.manifest.session_mode,
            ),
            None => (None, librefang_types::agent::SessionMode::default()),
        };
        // Clamp `persistent` agents to 1: parallel writes to the same
        // session's message history are undefined. Emit a warn so a
        // misconfigured manifest is visible at boot rather than silently
        // ignored. The check lives here (the resolver) instead of a
        // dedicated validator because the rule is structural to the
        // dispatch path, not a TOML-time concern.
        let resolved_cap = match (session_mode, manifest_cap) {
            (librefang_types::agent::SessionMode::Persistent, Some(n)) if n > 1 => {
                tracing::warn!(
                    agent_id = %agent_id,
                    requested = n,
                    "max_concurrent_invocations > 1 ignored — session_mode = \
                     \"persistent\" cannot run parallel invocations safely; \
                     clamped to 1. Set session_mode = \"new\" on the manifest \
                     to enable parallel fires (per-trigger overrides cannot \
                     escape the clamp — the per-agent semaphore is sized once \
                     from the manifest default).",
                );
                1
            }
            (_, Some(n)) => n,
            (_, None) => self.config.load().queue.concurrency.default_per_agent,
        }
        .max(1);
        self.agents
            .agent_concurrency
            .entry(agent_id)
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(resolved_cap)))
            .clone()
    }

    /// Persistent process manager. Delegates to
    /// [`ProcessSubsystem::manager`].
    #[inline]
    pub fn processes(&self) -> &Arc<librefang_runtime::process_manager::ProcessManager> {
        self.processes.manager()
    }

    /// Background process registry for fire-and-forget shell_exec
    /// processes. Delegates to [`ProcessSubsystem::registry`].
    #[inline]
    pub fn process_registry(&self) -> &Arc<librefang_runtime::process_registry::ProcessRegistry> {
        self.processes.registry()
    }

    /// OFP peer registry (set once at startup).
    #[inline]
    pub fn peer_registry_ref(&self) -> Option<&librefang_wire::PeerRegistry> {
        self.mesh.peer_registry_ref()
    }

    /// Test-only: install a `PeerRegistry` without booting the OFP node.
    /// Used by route-handler regression tests for #3644 — never call from
    /// production code; the OFP startup path owns this initialization
    /// (see `start_peer_node` -> `self.mesh.peer_registry.set(...)`).
    #[doc(hidden)]
    pub fn install_peer_registry_for_test(
        &self,
        registry: librefang_wire::PeerRegistry,
    ) -> Result<(), librefang_wire::PeerRegistry> {
        self.mesh.peer_registry.set(registry)
    }

    /// Hook registry.
    #[inline]
    pub fn hook_registry(&self) -> &librefang_runtime::hooks::HookRegistry {
        self.governance.hook_registry()
    }

    /// Auto-reply engine.
    #[inline]
    pub fn auto_reply(&self) -> &crate::auto_reply::AutoReplyEngine {
        &self.auto_reply_engine
    }

    /// Default model override (hot-reloadable).
    #[inline]
    pub fn default_model_override_ref(
        &self,
    ) -> &std::sync::RwLock<Option<librefang_types::config::DefaultModelConfig>> {
        self.llm.default_model_override_ref()
    }

    /// Tool policy override (hot-reloadable).
    #[inline]
    pub fn tool_policy_override_ref(
        &self,
    ) -> &std::sync::RwLock<Option<librefang_types::tool_policy::ToolPolicy>> {
        &self.tool_policy_override
    }

    /// WhatsApp gateway PID.
    #[inline]
    pub fn whatsapp_pid(&self) -> &Arc<std::sync::Mutex<Option<u32>>> {
        &self.whatsapp_gateway_pid
    }

    /// Per-(agent, session) message injection senders.
    #[inline]
    pub fn injection_senders_ref(
        &self,
    ) -> &dashmap::DashMap<(AgentId, SessionId), tokio::sync::mpsc::Sender<AgentLoopSignal>> {
        self.events.injection_senders_ref()
    }

    /// Context engine (pluggable memory recall + assembly).
    #[inline]
    pub fn context_engine_ref(
        &self,
    ) -> Option<&dyn librefang_runtime::context_engine::ContextEngine> {
        self.context_engine.as_deref()
    }

    /// Event bus.
    #[inline]
    pub fn event_bus_ref(&self) -> &EventBus {
        self.events.event_bus_ref()
    }

    /// Session lifecycle event bus (clone-shared `Arc` so subscribers can hold
    /// it across tasks).
    #[inline]
    pub fn session_lifecycle_bus(&self) -> Arc<crate::session_lifecycle::SessionLifecycleBus> {
        self.events.lifecycle_bus()
    }

    /// OFP peer node (set once at startup).
    #[inline]
    pub fn peer_node_ref(&self) -> Option<&Arc<librefang_wire::PeerNode>> {
        self.mesh.peer_node_ref()
    }

    /// Provider unconfigured log flag (atomic).
    #[inline]
    pub fn provider_unconfigured_flag(&self) -> &std::sync::atomic::AtomicBool {
        &self.provider_unconfigured_logged
    }

    /// Periodic garbage collection sweep for unbounded in-memory caches.
    ///
    /// Removes stale entries from DashMaps keyed by agent ID (retaining only
    /// agents still present in the registry), evicts expired assistant route
    /// cache entries, and caps prompt metadata cache size.
    pub(crate) fn gc_sweep(&self) {
        let live_agents: std::collections::HashSet<AgentId> =
            self.agents.registry.list().iter().map(|e| e.id).collect();
        let mut total_removed: usize = 0;

        // 1. running_tasks — abort and remove handles for dead agents; also
        //    remove handles for agents that are still live but whose task has
        //    already finished (is_finished() == true).  Without this, every
        //    completed agent turn leaves an orphan AbortHandle in the map
        //    that is never cleaned up by stop_agent_run / suspend_agent.
        //    Map is keyed by `(agent, session)` post-#3172, so the sweep
        //    fans out across all sessions for each dead/finished agent.
        {
            let finished: Vec<(AgentId, SessionId)> = self
                .agents
                .running_tasks
                .iter()
                .filter(|e| !live_agents.contains(&e.key().0) || e.value().abort.is_finished())
                .map(|e| *e.key())
                .collect();
            total_removed += finished.len();
            for key in finished {
                self.agents.running_tasks.remove(&key);
            }
        }

        // 3. agent_msg_locks — remove locks for dead agents
        {
            let stale: Vec<AgentId> = self
                .agents
                .agent_msg_locks
                .iter()
                .filter(|e| !live_agents.contains(e.key()))
                .map(|e| *e.key())
                .collect();
            total_removed += stale.len();
            for id in stale {
                self.agents.agent_msg_locks.remove(&id);
            }
        }

        // 3a. session_msg_locks — remove idle entries.  This map grows
        // unbounded (#3444): every (agent, session) pair gets a fresh
        // Mutex on first use and was never reclaimed, so long-lived
        // daemons accumulate entries proportional to total session
        // count.  SessionId itself does not carry the owning agent
        // (deterministic UUID-v5 derivations hash that away), so we
        // can't filter by `live_agents`; instead we rely on Arc strong
        // count: an entry is safely removable when the only outstanding
        // reference is the map's own slot — `Arc::strong_count == 1` —
        // because acquirers always clone the Arc out via `entry().
        // or_insert().clone()` before awaiting `lock()`.  A reused
        // session gets a fresh Mutex on next access; that's correct
        // because the previous lock had no waiters.
        {
            let candidates: Vec<SessionId> = self
                .agents
                .session_msg_locks
                .iter()
                .filter(|e| Arc::strong_count(e.value()) == 1)
                .map(|e| *e.key())
                .collect();
            for sid in candidates {
                // Re-check under the shard lock so a writer that grabbed
                // the Arc between iter() and remove() doesn't lose it.
                if self
                    .agents
                    .session_msg_locks
                    .remove_if(&sid, |_, arc| Arc::strong_count(arc) == 1)
                    .is_some()
                {
                    total_removed += 1;
                }
            }
        }

        // 3b. agent_concurrency — remove per-agent invocation semaphores
        // for dead agents. Mirrors the agent_msg_locks pass above; lazy
        // re-init on next dispatch will pick up any updated manifest cap.
        {
            let stale: Vec<AgentId> = self
                .agents
                .agent_concurrency
                .iter()
                .filter(|e| !live_agents.contains(e.key()))
                .map(|e| *e.key())
                .collect();
            total_removed += stale.len();
            for id in stale {
                self.agents.agent_concurrency.remove(&id);
            }
        }

        // 4. injection_senders / injection_receivers — remove for dead agents.
        {
            let stale: Vec<(AgentId, SessionId)> = self
                .events
                .injection_senders
                .iter()
                .filter(|e| !live_agents.contains(&e.key().0))
                .map(|e| *e.key())
                .collect();
            total_removed += stale.len();
            for key in &stale {
                self.events.injection_senders.remove(key);
                self.events.injection_receivers.remove(key);
            }
        }

        // 5. assistant_routes — evict entries unused for >30 minutes
        {
            let ttl = std::time::Duration::from_secs(30 * 60);
            let stale: Vec<String> = self
                .events
                .assistant_routes
                .iter()
                .filter(|e| e.value().1.elapsed() > ttl)
                .map(|e| e.key().clone())
                .collect();
            total_removed += stale.len();
            for key in stale {
                self.events.assistant_routes.remove(&key);
            }
        }

        // 6. decision_traces — remove dead agents, cap per-agent at 15
        {
            let stale: Vec<AgentId> = self
                .agents
                .decision_traces
                .iter()
                .filter(|e| !live_agents.contains(e.key()))
                .map(|e| *e.key())
                .collect();
            total_removed += stale.len();
            for id in stale {
                self.agents.decision_traces.remove(&id);
            }
            // Cap surviving entries
            for mut entry in self.agents.decision_traces.iter_mut() {
                let traces = entry.value_mut();
                if traces.len() > 15 {
                    let drain = traces.len() - 15;
                    traces.drain(..drain);
                }
            }
        }

        // 7. prompt_metadata_cache — clear expired + cap at 100 entries
        {
            self.prompt_metadata_cache
                .workspace
                .retain(|_, v| !v.is_expired());
            self.prompt_metadata_cache
                .skills
                .retain(|_, v| !v.is_expired());
            self.prompt_metadata_cache
                .tools
                .retain(|_, v| !v.is_expired());
            // Hard cap to prevent unbounded growth under extreme load
            if self.prompt_metadata_cache.workspace.len() > 100 {
                self.prompt_metadata_cache.workspace.clear();
            }
            if self.prompt_metadata_cache.skills.len() > 100 {
                self.prompt_metadata_cache.skills.clear();
            }
            if self.prompt_metadata_cache.tools.len() > 100 {
                self.prompt_metadata_cache.tools.clear();
            }
        }

        // 8. route_divergence — remove keys no longer present in assistant_routes
        {
            let stale: Vec<String> = self
                .events
                .route_divergence
                .iter()
                .filter(|e| !self.events.assistant_routes.contains_key(e.key()))
                .map(|e| e.key().clone())
                .collect();
            total_removed += stale.len();
            for key in stale {
                self.events.route_divergence.remove(&key);
            }
        }

        // 9. skill_review_cooldowns — remove entries for dead agents
        {
            let stale: Vec<String> = self
                .skills
                .skill_review_cooldowns
                .iter()
                .filter(|e| {
                    e.key()
                        .parse::<AgentId>()
                        .map(|id| !live_agents.contains(&id))
                        .unwrap_or(false)
                })
                .map(|e| e.key().clone())
                .collect();
            total_removed += stale.len();
            for id in stale {
                self.skills.skill_review_cooldowns.remove(&id);
            }
        }

        // 10. delivery_tracker — remove receipts for dead agents
        total_removed += self.mesh.delivery_tracker.gc_stale_agents(&live_agents);

        // 11. event_bus agent channels — remove channels for dead agents
        total_removed += self.events.event_bus.gc_stale_channels(&live_agents);

        // 10. sessions — delete orphan sessions for agents no longer in registry
        {
            let live_ids: Vec<librefang_types::agent::AgentId> =
                live_agents.iter().copied().collect();
            match self.memory_substrate().cleanup_orphan_sessions(&live_ids) {
                Ok(n) if n > 0 => {
                    info!(deleted = n, "Cleaned up orphan sessions");
                    total_removed += n as usize;
                }
                Err(e) => warn!("Failed to cleanup orphan sessions: {e}"),
                _ => {}
            }
        }

        if total_removed > 0 {
            info!(
                removed = total_removed,
                live_agents = live_agents.len(),
                "GC sweep completed"
            );
        }
    }
}
