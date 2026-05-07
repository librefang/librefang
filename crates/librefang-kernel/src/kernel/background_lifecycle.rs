//! Cluster pulled out of mod.rs in #4713 phase 3e/5.
//!
//! Hosts the kernel's background-lifecycle surface: spawning the
//! continuous / periodic / proactive agent loops at boot
//! (`start_background_agents`), the cron scheduler, the auto-dream gate
//! poller, the various sweeper tasks, and the orderly `shutdown` /
//! `shutdown_with_reason` paths that drain in-flight turns and
//! persist agent state. This is the longest contiguous control-plane
//! cluster — bundling spawn, scheduling, and teardown together keeps
//! the lifecycle invariants reviewable in one file.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Start background loops for all non-reactive agents.
    ///
    /// Must be called after the kernel is wrapped in `Arc` (e.g., from the daemon).
    /// Iterates the agent registry and starts background tasks for agents with
    /// `Continuous`, `Periodic`, or `Proactive` schedules.
    /// Hands activated on first boot when no `hand_state.json` exists yet.
    /// By default, NO hands are activated to prevent unexpected token consumption.
    pub async fn start_background_agents(self: &Arc<Self>) {
        // Fire external gateway:startup hook (fire-and-forget) before starting agents.
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::GatewayStartup,
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
            }),
        );

        let cfg = self.config.load_full();

        // #3347 4/N: artifact-store GC at daemon startup.
        // Spawns a background task that walks the spill directory once and
        // deletes any `<hash>.bin` (or orphan `<hash>.<pid>.<nanos>.tmp`)
        // file with mtime older than `[tool_results] artifact_max_age_days`.
        // Set to `0` in config to disable.  Idempotent across the lifetime
        // of the process — repeat calls are no-ops.
        //
        // Resolve the directory via `default_artifact_storage_dir()`, not
        // `self.data_dir_boot`: the spill writers in `librefang-runtime`
        // use the env-based path (`LIBREFANG_HOME/data/artifacts` or
        // `~/.librefang/data/artifacts`) and would silently diverge from
        // `config.data_dir` whenever an operator overrode `[data] data_dir`
        // in `config.toml` without also setting `LIBREFANG_HOME` — GC
        // would scan an empty directory while the artifact store grew
        // unbounded under the env path.
        let max_age_days = cfg.tool_results.artifact_max_age_days;
        if max_age_days > 0 {
            let artifact_dir = librefang_runtime::artifact_store::default_artifact_storage_dir();
            let max_age = std::time::Duration::from_secs(max_age_days as u64 * 24 * 60 * 60);
            librefang_runtime::artifact_store::run_startup_gc_once(&artifact_dir, max_age);
        }

        // Restore previously active hands from persisted state
        let state_path = self.home_dir_boot.join("data").join("hand_state.json");
        let saved_hands = librefang_hands::registry::HandRegistry::load_state_detailed(&state_path);
        if !saved_hands.entries.is_empty() {
            info!("Restoring {} persisted hand(s)", saved_hands.entries.len());
            for saved_hand in saved_hands.entries {
                let hand_id = saved_hand.hand_id;
                let config = saved_hand.config;
                let agent_runtime_overrides = saved_hand.agent_runtime_overrides;
                let old_agent_id = saved_hand.old_agent_ids;
                let status = saved_hand.status;
                let persisted_instance_id = saved_hand.instance_id;
                // The persisted coordinator role is informational here.
                // `activate_hand_with_id` always re-derives the coordinator from the
                // latest hand definition before spawning agents.
                // Check if hand's agent.toml has enabled=false — skip reactivation
                let hand_agent_name = format!("{}-hand", hand_id);
                let hand_toml = cfg
                    .effective_hands_workspaces_dir()
                    .join(&hand_agent_name)
                    .join("agent.toml");
                if hand_toml.exists() {
                    if let Ok(content) = std::fs::read_to_string(&hand_toml) {
                        if toml_enabled_false(&content) {
                            info!(hand = %hand_id, "Hand disabled in config — skipping reactivation");
                            continue;
                        }
                    }
                }
                let timestamps = saved_hand
                    .activated_at
                    .and_then(|a| saved_hand.updated_at.map(|u| (a, u)));
                match self.activate_hand_with_id(
                    &hand_id,
                    config,
                    agent_runtime_overrides.clone(),
                    persisted_instance_id,
                    timestamps,
                ) {
                    Ok(inst) => {
                        if matches!(status, librefang_hands::HandStatus::Paused) {
                            if let Err(e) = self.pause_hand(inst.instance_id) {
                                warn!(hand = %hand_id, error = %e, "Failed to restore paused state");
                            } else {
                                info!(hand = %hand_id, instance = %inst.instance_id, "Hand restored (paused)");
                            }
                        } else {
                            info!(hand = %hand_id, instance = %inst.instance_id, status = %status, "Hand restored");
                        }
                        // Reassign cron jobs and triggers from the pre-restart
                        // agent IDs to the newly spawned agents so scheduled tasks
                        // and event triggers survive daemon restarts (issues
                        // #402, #519). activate_hand only handles reassignment
                        // when an existing agent is found in the live registry,
                        // which is empty on a fresh boot.
                        for (role, old_id) in &old_agent_id {
                            if let Some(&new_id) = inst.agent_ids.get(role) {
                                if old_id.0 != new_id.0 {
                                    let migrated = self
                                        .workflows
                                        .cron_scheduler
                                        .reassign_agent_jobs(*old_id, new_id);
                                    if migrated > 0 {
                                        info!(
                                            hand = %hand_id,
                                            role = %role,
                                            old_agent = %old_id,
                                            new_agent = %new_id,
                                            migrated,
                                            "Reassigned cron jobs after restart"
                                        );
                                        if let Err(e) = self.workflows.cron_scheduler.persist() {
                                            warn!(
                                                "Failed to persist cron jobs after hand restore: {e}"
                                            );
                                        }
                                    }
                                    let t_migrated = self
                                        .workflows
                                        .triggers
                                        .reassign_agent_triggers(*old_id, new_id);
                                    if t_migrated > 0 {
                                        info!(
                                            hand = %hand_id,
                                            role = %role,
                                            old_agent = %old_id,
                                            new_agent = %new_id,
                                            migrated = t_migrated,
                                            "Reassigned triggers after restart"
                                        );
                                        if let Err(e) = self.workflows.triggers.persist() {
                                            warn!(
                                                "Failed to persist trigger jobs after hand restore: {e}"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => warn!(hand = %hand_id, error = %e, "Failed to restore hand"),
                }
            }
        } else if !state_path.exists() {
            // First boot: scaffold workspace directories and identity files for all
            // registry hands without activating them. Activation (DB entries, session
            // spawning, agent registration) only happens when the user explicitly
            // enables a hand — not unconditionally on every fresh install.
            let defs = self.skills.hand_registry.list_definitions();
            if !defs.is_empty() {
                info!(
                    "First boot — scaffolding {} hand workspace(s) (files only, no activation)",
                    defs.len()
                );
                let hands_ws_dir = cfg.effective_hands_workspaces_dir();
                for def in &defs {
                    for (role, agent) in &def.agents {
                        let safe_hand = safe_path_component(&def.id, "hand");
                        let safe_role = safe_path_component(role, "agent");
                        let workspace = hands_ws_dir.join(&safe_hand).join(&safe_role);
                        if let Err(e) = ensure_workspace(&workspace) {
                            warn!(hand = %def.id, role = %role, error = %e, "Failed to scaffold hand workspace");
                            continue;
                        }
                        migrate_identity_files(&workspace);
                        let resolved_ws = ensure_named_workspaces(
                            &cfg.effective_workspaces_dir(),
                            &agent.manifest.workspaces,
                            &cfg.allowed_mount_roots,
                        );
                        generate_identity_files(&workspace, &agent.manifest, &resolved_ws);
                    }
                }
                // Write an empty state file so subsequent boots skip this block.
                self.persist_hand_state();
            }
        }

        // ── Orphaned hand-agent GC ────────────────────────────────────────
        // After the boot restore loop above, `hand_registry.list_instances()`
        // contains every agent id that belongs to a currently active hand.
        // Any `is_hand = true` row in SQLite whose id is not in that live
        // set is orphaned — it belonged to a previous activation that was
        // deactivated or failed to restore, and since the #a023519d fix
        // skips `is_hand` rows in `load_all_agents`, it will never be
        // reconstructed. Remove it (and its sessions via the cascade in
        // `memory.remove_agent`) so the DB doesn't accumulate garbage
        // across restart cycles.
        //
        // Non-hand agents are untouched; we filter on `entry.is_hand`
        // before considering a row for deletion.
        //
        // Hand agents restore from `hand_state.json`, not from the generic
        // SQLite boot path. The `is_hand = true` SQLite rows are secondary
        // state used for continuity and cleanup only. If `hand_state.json`
        // is unreadable, skip GC so a transient parse failure cannot delete
        // the only surviving hand-agent metadata.
        if saved_hands.status != librefang_hands::registry::LoadStateStatus::ParseFailed {
            let live_hand_agents: std::collections::HashSet<AgentId> = self
                .skills
                .hand_registry
                .list_instances()
                .iter()
                .flat_map(|inst| inst.agent_ids.values().copied().collect::<Vec<_>>())
                .collect();
            match self.memory.substrate.load_all_agents_async().await {
                Ok(all) => {
                    let mut removed = 0usize;
                    for entry in all {
                        if !entry.is_hand {
                            continue;
                        }
                        if live_hand_agents.contains(&entry.id) {
                            continue;
                        }
                        match self.memory.substrate.remove_agent_async(entry.id).await {
                            Ok(()) => {
                                removed += 1;
                                info!(
                                    agent = %entry.name,
                                    id = %entry.id,
                                    "GC: removed orphaned hand-agent row from SQLite"
                                );
                            }
                            Err(e) => warn!(
                                agent = %entry.name,
                                id = %entry.id,
                                error = %e,
                                "GC: failed to remove orphaned hand-agent row"
                            ),
                        }
                    }
                    if removed > 0 {
                        info!("GC: removed {removed} orphaned hand-agent row(s) from SQLite");
                    }
                }
                Err(e) => warn!("GC: failed to enumerate agents for orphan scan: {e}"),
            }
        } else {
            warn!(
                path = %state_path.display(),
                "Skipping orphaned hand-agent GC because hand_state.json failed to parse"
            );
        }

        // Context-engine bootstrap is async; run it at daemon startup so hook
        // script/path validation fails early instead of on first hook call.
        if let Some(engine) = self.context_engine.as_deref() {
            match engine.bootstrap(&self.context_engine_config).await {
                Ok(()) => info!("Context engine bootstrap complete"),
                Err(e) => warn!("Context engine bootstrap failed: {e}"),
            }
        }

        // ── Startup API key health check ──────────────────────────────────
        // Verify that configured API keys are present in the environment.
        // Missing keys are logged as warnings so the operator can fix them
        // before they cause runtime errors.
        {
            let mut missing: Vec<String> = Vec::new();

            // Default LLM provider — prefer explicit api_key_env, then resolve.
            // Skip providers that run locally (ollama, vllm, lmstudio, …) —
            // they don't need a key and flagging them confuses operators.
            if !librefang_runtime::provider_health::is_local_provider(&cfg.default_model.provider) {
                let llm_env = if !cfg.default_model.api_key_env.is_empty() {
                    cfg.default_model.api_key_env.clone()
                } else {
                    cfg.resolve_api_key_env(&cfg.default_model.provider)
                };
                if std::env::var(&llm_env).unwrap_or_default().is_empty() {
                    missing.push(format!(
                        "LLM ({}): ${}",
                        cfg.default_model.provider, llm_env
                    ));
                }
            }

            // Fallback LLM providers — same local-provider exemption.
            for fb in &cfg.fallback_providers {
                if librefang_runtime::provider_health::is_local_provider(&fb.provider) {
                    continue;
                }
                let env_var = if !fb.api_key_env.is_empty() {
                    fb.api_key_env.clone()
                } else {
                    cfg.resolve_api_key_env(&fb.provider)
                };
                if std::env::var(&env_var).unwrap_or_default().is_empty() {
                    missing.push(format!("LLM fallback ({}): ${}", fb.provider, env_var));
                }
            }

            // Search provider
            let search_env = match cfg.web.search_provider {
                librefang_types::config::SearchProvider::Brave => {
                    Some(("Brave", cfg.web.brave.api_key_env.clone()))
                }
                librefang_types::config::SearchProvider::Tavily => {
                    Some(("Tavily", cfg.web.tavily.api_key_env.clone()))
                }
                librefang_types::config::SearchProvider::Perplexity => {
                    Some(("Perplexity", cfg.web.perplexity.api_key_env.clone()))
                }
                librefang_types::config::SearchProvider::Jina => {
                    Some(("Jina", cfg.web.jina.api_key_env.clone()))
                }
                _ => None,
            };
            if let Some((name, env_var)) = search_env {
                if std::env::var(&env_var).unwrap_or_default().is_empty() {
                    missing.push(format!("Search ({}): ${}", name, env_var));
                }
            }

            if missing.is_empty() {
                info!("Startup health check: all configured API keys present");
            } else {
                warn!(
                    count = missing.len(),
                    "Startup health check: missing API keys — affected services may fail"
                );
                for m in &missing {
                    warn!("  ↳ {}", m);
                }
                // Notify owner about missing keys
                self.notify_owner_bg(format!(
                    "⚠️ Startup: {} API key(s) missing — {}. Set the env vars and restart.",
                    missing.len(),
                    missing.join(", ")
                ));
            }
        }

        let agents = self.agents.registry.list();
        let mut bg_agents: Vec<(librefang_types::agent::AgentId, String, ScheduleMode)> =
            Vec::new();

        for entry in &agents {
            if matches!(entry.manifest.schedule, ScheduleMode::Reactive) || !entry.manifest.enabled
            {
                continue;
            }
            bg_agents.push((
                entry.id,
                entry.name.clone(),
                entry.manifest.schedule.clone(),
            ));
        }

        if !bg_agents.is_empty() {
            let count = bg_agents.len();
            let kernel = Arc::clone(self);
            // Stagger agent startup to prevent rate-limit storm on shared providers.
            // Each agent gets a 500ms delay before the next one starts.
            spawn_logged("background_agents_staggered_start", async move {
                for (i, (id, name, schedule)) in bg_agents.into_iter().enumerate() {
                    kernel.start_background_for_agent(id, &name, &schedule);
                    if i > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
                info!("Started {count} background agent loop(s) (staggered)");
            });
        }

        // Start heartbeat monitor for agent health checking
        self.start_heartbeat_monitor();

        // Start file inbox watcher if enabled
        crate::inbox::start_inbox_watcher(Arc::clone(self));

        // Start OFP peer node if network is enabled
        if cfg.network_enabled && !cfg.network.shared_secret.is_empty() {
            let kernel = Arc::clone(self);
            spawn_logged("ofp_node", async move {
                kernel.start_ofp_node().await;
            });
        }

        // Probe local providers for reachability and model discovery.
        //
        // Runs once immediately on boot, then every `LOCAL_PROBE_INTERVAL_SECS`
        // so the catalog tracks local servers that start / stop after boot
        // (common: user installs Ollama while LibreFang is running, or `brew
        // services stop ollama`). Without periodic reprobing a one-shot
        // failure at startup sticks in the catalog forever.
        //
        // The set of providers the user actually relies on (default + fallback
        // chain) gets a `warn!` when offline — those are real misconfigurations
        // or stopped services. Every other local provider in the built-in
        // catalog drops to `debug!`: it's informational (the catalog still
        // records `LocalOffline` so the dashboard shows the right state), but
        // an unconfigured provider being offline is the expected case and
        // shouldn't spam every boot.
        {
            let kernel = Arc::clone(self);
            let relevant_providers: std::collections::HashSet<String> =
                std::iter::once(cfg.default_model.provider.to_lowercase())
                    .chain(
                        cfg.fallback_providers
                            .iter()
                            .map(|fb| fb.provider.to_lowercase()),
                    )
                    .collect();
            // Probe interval comes from `[providers] local_probe_interval_secs`
            // (default 60). Values below the 2s probe timeout are nonsensical
            // — clamp to the default so a mis-configured TOML doesn't
            // stampede the local daemon.
            let probe_interval_secs = if cfg.local_probe_interval_secs >= 2 {
                cfg.local_probe_interval_secs
            } else {
                60
            };
            let mut shutdown_rx = self.agents.supervisor.subscribe();
            spawn_logged("local_provider_probe", async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(probe_interval_secs));
                // Race the tick against the shutdown watch so daemon
                // shutdown breaks immediately instead of blocking up to
                // `probe_interval_secs` (60s by default) on the next tick.
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            probe_all_local_providers_once(&kernel, &relevant_providers).await;
                        }
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                break;
                            }
                        }
                    }
                }
            });
        }

        // Periodic usage data cleanup (every 24 hours, retain 90 days)
        {
            let kernel = Arc::clone(self);
            spawn_logged("metering_cleanup", async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
                interval.tick().await; // Skip first immediate tick
                loop {
                    interval.tick().await;
                    if kernel.agents.supervisor.is_shutting_down() {
                        break;
                    }
                    match kernel.metering.engine.cleanup(90) {
                        Ok(removed) if removed > 0 => {
                            info!("Metering cleanup: removed {removed} old usage records");
                        }
                        Err(e) => {
                            warn!("Metering cleanup failed: {e}");
                        }
                        _ => {}
                    }
                }
            });
        }

        // Periodic DB retention sweep — hard-deletes soft-deleted memories
        // (#3467), finished task_queue rows (#3466), and approval_audit
        // rows (#3468). Runs once a day on the same cadence as the audit
        // prune below; each sub-step is independent so a config of `0` for
        // any one of them only disables that step. Failures only log; the
        // sweep is best-effort and re-runs at the next interval.
        {
            let memory_retention = cfg.memory.soft_delete_retention_days;
            let queue_retention = cfg.queue.task_queue_retention_days;
            let approval_retention = cfg.approval.audit_retention_days;
            let any_enabled = memory_retention > 0 || queue_retention > 0 || approval_retention > 0;
            if any_enabled {
                let kernel = Arc::clone(self);
                tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
                    interval.tick().await; // skip immediate tick
                    loop {
                        interval.tick().await;
                        if kernel.agents.supervisor.is_shutting_down() {
                            break;
                        }
                        if memory_retention > 0 {
                            match kernel
                                .memory
                                .substrate
                                .prune_soft_deleted_memories(memory_retention)
                            {
                                Ok(n) if n > 0 => info!(
                                    "Memory retention: hard-deleted {n} soft-deleted memories \
                                     (older than {memory_retention} days)"
                                ),
                                Ok(_) => {}
                                Err(e) => warn!("Memory retention sweep failed: {e}"),
                            }
                        }
                        if queue_retention > 0 {
                            match kernel
                                .memory
                                .substrate
                                .task_prune_finished(queue_retention)
                                .await
                            {
                                Ok(n) if n > 0 => info!(
                                    "Task queue retention: pruned {n} finished tasks \
                                     (older than {queue_retention} days)"
                                ),
                                Ok(_) => {}
                                Err(e) => warn!("Task queue retention sweep failed: {e}"),
                            }
                        }
                        if approval_retention > 0 {
                            let n = kernel
                                .governance
                                .approval_manager
                                .prune_audit(approval_retention);
                            if n > 0 {
                                info!(
                                    "Approval audit retention: pruned {n} rows \
                                     (older than {approval_retention} days)"
                                );
                            }
                        }
                    }
                });
                info!(
                    "DB retention sweep scheduled daily \
                     (memory={memory_retention}d, task_queue={queue_retention}d, \
                     approval_audit={approval_retention}d)"
                );
            }
        }

        // Periodic audit log pruning (daily, respects audit.retention_days)
        {
            let kernel = Arc::clone(self);
            let retention = cfg.audit.retention_days;
            if retention > 0 {
                spawn_logged("audit_log_pruner", async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
                    interval.tick().await; // Skip first immediate tick
                    loop {
                        interval.tick().await;
                        if kernel.agents.supervisor.is_shutting_down() {
                            break;
                        }
                        let pruned = kernel.metering.audit_log.prune(retention);
                        if pruned > 0 {
                            info!("Audit log pruning: removed {pruned} entries older than {retention} days");
                        }
                    }
                });
                info!("Audit log pruning scheduled daily (retention_days={retention})");
            }
        }

        // Periodic audit retention trim (M7) — per-action retention with
        // chain-anchor preservation. Distinct from the legacy day-based
        // `prune` above: this one honors `audit.retention.retention_days_by_action`,
        // enforces `max_in_memory_entries`, and writes a self-audit
        // `RetentionTrim` row so trims are themselves auditable. The
        // legacy `prune` keeps running in parallel for operators who
        // only set the coarse `retention_days` field.
        {
            let trim_interval = cfg.audit.retention.trim_interval_secs.unwrap_or(0);
            // 0 / unset disables the trim job entirely — matches the
            // "default = preserve forever" rule for the per-action map.
            if trim_interval > 0 {
                let kernel = Arc::clone(self);
                let retention = cfg.audit.retention.clone();
                spawn_logged("audit_retention_trim", async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs(trim_interval));
                    interval.tick().await; // Skip first immediate tick.
                    loop {
                        interval.tick().await;
                        if kernel.agents.supervisor.is_shutting_down() {
                            break;
                        }
                        let report = kernel
                            .metering
                            .audit_log
                            .trim(&retention, chrono::Utc::now());
                        if !report.is_empty() {
                            // Detail is JSON of the per-action drop counts.
                            // Keeping it small + structured so a downstream
                            // dashboard can parse a `RetentionTrim` row
                            // without a separate metrics surface.
                            let detail = serde_json::json!({
                                "dropped_by_action": report.dropped_by_action,
                                "total_dropped": report.total_dropped,
                                "new_chain_anchor": report.new_chain_anchor,
                            })
                            .to_string();
                            kernel.metering.audit_log.record(
                                "system",
                                librefang_runtime::audit::AuditAction::RetentionTrim,
                                detail,
                                "ok",
                            );
                            info!(
                                total_dropped = report.total_dropped,
                                "Audit retention trim: dropped {} entries (per-action: {:?})",
                                report.total_dropped,
                                report.dropped_by_action,
                            );
                        }
                    }
                });
                info!(
                    "Audit retention trim scheduled every {trim_interval}s \
                     (per-action policy: {} rules, max_in_memory={:?})",
                    cfg.audit.retention.retention_days_by_action.len(),
                    cfg.audit.retention.max_in_memory_entries,
                );
            }
        }

        // Periodic session retention cleanup (prune expired / excess sessions)
        {
            let session_cfg = cfg.session.clone();
            let needs_cleanup =
                session_cfg.retention_days > 0 || session_cfg.max_sessions_per_agent > 0;
            if needs_cleanup && session_cfg.cleanup_interval_hours > 0 {
                let kernel = Arc::clone(self);
                spawn_logged("session_retention_cleanup", async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                        u64::from(session_cfg.cleanup_interval_hours) * 3600,
                    ));
                    interval.tick().await; // Skip first immediate tick
                    loop {
                        interval.tick().await;
                        if kernel.agents.supervisor.is_shutting_down() {
                            break;
                        }
                        let mut total = 0u64;
                        if session_cfg.retention_days > 0 {
                            match kernel
                                .memory
                                .substrate
                                .cleanup_expired_sessions(session_cfg.retention_days)
                            {
                                Ok(n) => total += n,
                                Err(e) => {
                                    warn!("Session retention cleanup (expired) failed: {e}");
                                }
                            }
                        }
                        if session_cfg.max_sessions_per_agent > 0 {
                            match kernel
                                .memory
                                .substrate
                                .cleanup_excess_sessions(session_cfg.max_sessions_per_agent)
                            {
                                Ok(n) => total += n,
                                Err(e) => {
                                    warn!("Session retention cleanup (excess) failed: {e}");
                                }
                            }
                        }
                        if total > 0 {
                            info!("Session retention cleanup: removed {total} session(s)");
                        }
                    }
                });
                info!(
                    "Session retention cleanup scheduled every {} hour(s) (retention_days={}, max_per_agent={})",
                    session_cfg.cleanup_interval_hours,
                    session_cfg.retention_days,
                    session_cfg.max_sessions_per_agent,
                );
            }
        }

        // Startup session prune + VACUUM: run once at boot before background
        // agents start. Mirrors Hermes `maybe_auto_prune_and_vacuum()` — only
        // VACUUM when rows were actually deleted so the rewrite is worthwhile.
        {
            let session_cfg = cfg.session.clone();
            let needs_cleanup =
                session_cfg.retention_days > 0 || session_cfg.max_sessions_per_agent > 0;
            if needs_cleanup {
                let mut pruned_total: u64 = 0;
                if session_cfg.retention_days > 0 {
                    match self
                        .memory
                        .substrate
                        .cleanup_expired_sessions(session_cfg.retention_days)
                    {
                        Ok(n) => pruned_total += n,
                        Err(e) => warn!("Startup session prune (expired) failed: {e}"),
                    }
                }
                if session_cfg.max_sessions_per_agent > 0 {
                    match self
                        .memory
                        .substrate
                        .cleanup_excess_sessions(session_cfg.max_sessions_per_agent)
                    {
                        Ok(n) => pruned_total += n,
                        Err(e) => warn!("Startup session prune (excess) failed: {e}"),
                    }
                }
                if let Err(e) = self
                    .memory
                    .substrate
                    .vacuum_if_shrank_async(pruned_total as usize)
                    .await
                {
                    warn!("Startup VACUUM after session prune failed: {e}");
                }
                if pruned_total > 0 {
                    info!("Startup session prune: removed {pruned_total} session(s)");
                }
            }
        }

        // Periodic cleanup of expired image uploads (24h TTL)
        {
            let kernel = Arc::clone(self);
            spawn_logged("upload_cleanup", async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); // every hour
                interval.tick().await; // skip first immediate tick
                loop {
                    interval.tick().await;
                    if kernel.agents.supervisor.is_shutting_down() {
                        break;
                    }
                    let upload_dir = kernel.config_ref().channels.effective_file_download_dir();
                    if let Ok(mut entries) = tokio::fs::read_dir(&upload_dir).await {
                        let cutoff = std::time::SystemTime::now()
                            - std::time::Duration::from_secs(24 * 3600);
                        let mut removed = 0u64;
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            if let Ok(meta) = entry.metadata().await {
                                let expired = meta.modified().map(|t| t < cutoff).unwrap_or(false);
                                if expired && tokio::fs::remove_file(entry.path()).await.is_ok() {
                                    removed += 1;
                                }
                            }
                        }
                        if removed > 0 {
                            info!("Image upload cleanup: removed {removed} expired file(s)");
                        }
                    }
                }
            });
            info!("Image upload cleanup scheduled every 1 hour (TTL=24h)");
        }

        // Periodic memory consolidation (decays stale memory confidence)
        {
            let interval_hours = cfg.memory.consolidation_interval_hours;
            if interval_hours > 0 {
                let kernel = Arc::clone(self);
                spawn_logged("memory_consolidation", async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                        interval_hours * 3600,
                    ));
                    interval.tick().await; // Skip first immediate tick
                    loop {
                        interval.tick().await;
                        if kernel.agents.supervisor.is_shutting_down() {
                            break;
                        }
                        match kernel.memory.substrate.consolidate().await {
                            Ok(report) => {
                                if report.memories_decayed > 0 || report.memories_merged > 0 {
                                    info!(
                                        merged = report.memories_merged,
                                        decayed = report.memories_decayed,
                                        duration_ms = report.duration_ms,
                                        "Memory consolidation completed"
                                    );
                                }
                            }
                            Err(e) => {
                                warn!("Memory consolidation failed: {e}");
                            }
                        }
                    }
                });
                info!("Memory consolidation scheduled every {interval_hours} hour(s)");
            }
        }

        // Periodic memory decay (deletes stale SESSION/AGENT memories by TTL)
        {
            let decay_config = cfg.memory.decay.clone();
            if decay_config.enabled && decay_config.decay_interval_hours > 0 {
                let kernel = Arc::clone(self);
                let interval_hours = decay_config.decay_interval_hours;
                spawn_logged("memory_decay", async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                        u64::from(interval_hours) * 3600,
                    ));
                    interval.tick().await; // Skip first immediate tick
                    loop {
                        interval.tick().await;
                        if kernel.agents.supervisor.is_shutting_down() {
                            break;
                        }
                        match kernel.memory.substrate.run_decay(&decay_config) {
                            Ok(n) => {
                                if n > 0 {
                                    info!(deleted = n, "Memory decay sweep completed");
                                }
                            }
                            Err(e) => {
                                warn!("Memory decay sweep failed: {e}");
                            }
                        }
                    }
                });
                info!("Memory decay scheduled every {interval_hours} hour(s)");
            }
        }

        // Periodic GC sweep for unbounded in-memory caches (every 5 minutes)
        {
            let kernel = Arc::clone(self);
            spawn_logged("gc_sweep", async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
                interval.tick().await; // Skip first immediate tick
                loop {
                    interval.tick().await;
                    if kernel.agents.supervisor.is_shutting_down() {
                        break;
                    }
                    kernel.gc_sweep();
                }
            });
            info!("In-memory GC sweep scheduled every 5 minutes");
        }

        // Connect to configured + extension MCP servers
        let has_mcp = self
            .mcp
            .effective_mcp_servers
            .read()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if has_mcp {
            let kernel = Arc::clone(self);
            spawn_logged("connect_mcp_servers", async move {
                kernel.connect_mcp_servers().await;
            });
        }

        // Start extension health monitor background task
        {
            let kernel = Arc::clone(self);
            // #3740: spawn_logged so panics in the health loop surface in logs.
            spawn_logged("mcp_health_loop", async move {
                kernel.run_mcp_health_loop().await;
            });
        }

        // Auto-dream scheduler (background memory consolidation). Inert when
        // disabled in config — the spawned task checks on every tick and
        // bails cheaply.
        crate::auto_dream::spawn_scheduler(Arc::clone(self));

        // Cron scheduler tick loop — fires due jobs every 15 seconds.
        // The body lives in `kernel::cron_tick::run_cron_scheduler_loop`
        // (#4713 phase 3b); only the spawn wrapper stays here.
        {
            let kernel = Arc::clone(self);
            // #3740: spawn_logged so panics in the cron loop surface in logs.
            spawn_logged("cron_scheduler", cron_tick::run_cron_scheduler_loop(kernel));
            if self.workflows.cron_scheduler.total_jobs() > 0 {
                info!(
                    "Cron scheduler active with {} job(s)",
                    self.workflows.cron_scheduler.total_jobs()
                );
            }
        }

        // Log network status from config
        if cfg.network_enabled {
            info!("OFP network enabled — peer discovery will use shared_secret from config");
        }

        // Discover configured external A2A agents
        if let Some(ref a2a_config) = cfg.a2a {
            if a2a_config.enabled && !a2a_config.external_agents.is_empty() {
                let kernel = Arc::clone(self);
                let agents = a2a_config.external_agents.clone();
                spawn_logged("a2a_discover_external", async move {
                    let discovered =
                        librefang_runtime::a2a::discover_external_agents(&agents).await;
                    if let Ok(mut store) = kernel.mesh.a2a_external_agents.lock() {
                        *store = discovered;
                    }
                });
            }
        }

        // Start WhatsApp Web gateway if WhatsApp channel is configured
        if cfg.channels.whatsapp.is_some() {
            let kernel = Arc::clone(self);
            spawn_logged("whatsapp_gateway_starter", async move {
                crate::whatsapp_gateway::start_whatsapp_gateway(&kernel).await;
            });
        }
    }

    /// Start the heartbeat monitor background task.
    /// Start the OFP peer networking node.
    ///
    /// Binds a TCP listener, registers with the peer registry, and connects
    /// to bootstrap peers from config.
    async fn start_ofp_node(self: &Arc<Self>) {
        let cfg = self.config.load_full();
        use librefang_wire::{PeerConfig, PeerNode, PeerRegistry};

        let listen_addr_str = cfg
            .network
            .listen_addresses
            .first()
            .cloned()
            .unwrap_or_else(|| "0.0.0.0:9090".to_string());

        // Parse listen address — support both multiaddr-style and plain socket addresses
        let listen_addr: std::net::SocketAddr = if listen_addr_str.starts_with('/') {
            // Multiaddr format like /ip4/0.0.0.0/tcp/9090 — extract IP and port
            let parts: Vec<&str> = listen_addr_str.split('/').collect();
            let ip = parts.get(2).unwrap_or(&"0.0.0.0");
            let port = parts.get(4).unwrap_or(&"9090");
            format!("{ip}:{port}")
                .parse()
                .unwrap_or_else(|_| "0.0.0.0:9090".parse().unwrap())
        } else {
            listen_addr_str
                .parse()
                .unwrap_or_else(|_| "0.0.0.0:9090".parse().unwrap())
        };

        // SECURITY (#3873): Load (or generate + persist) this node's
        // Ed25519 keypair AND a stable node_id from the data directory.
        // Both are bundled in `peer_keypair.json` so a daemon restart
        // resumes under the same OFP identity. Falling back to a fresh
        // `Uuid::new_v4()` per restart — the prior behavior — silently
        // defeated TOFU pinning, since legitimate peers always presented
        // a "new" node_id and the mismatch-detection branch never fired.
        let mut key_mgr = librefang_wire::keys::PeerKeyManager::new(self.data_dir_boot.clone());
        let (keypair, node_id) = match key_mgr.load_or_generate() {
            Ok(kp) => {
                let kp = kp.clone();
                let id = key_mgr
                    .node_id()
                    .expect("node_id is Some after successful load_or_generate")
                    .to_string();
                (Some(kp), id)
            }
            Err(e) => {
                // Identity load failed — refuse to start OFP rather than
                // silently degrading to ephemeral identity, which would
                // lose TOFU continuity without operator awareness.
                error!(
                    error = %e,
                    data_dir = %self.data_dir_boot.display(),
                    "OFP: failed to load or generate peer identity; OFP networking will not start",
                );
                return;
            }
        };
        let node_name = gethostname().unwrap_or_else(|| "librefang-node".to_string());

        let peer_config = PeerConfig {
            listen_addr,
            node_id: node_id.clone(),
            node_name: node_name.clone(),
            shared_secret: cfg.network.shared_secret.clone(),
            max_messages_per_peer_per_minute: cfg.network.max_messages_per_peer_per_minute,
            max_llm_tokens_per_peer_per_hour: cfg.network.max_llm_tokens_per_peer_per_hour,
        };

        let registry = PeerRegistry::new();

        let handle: Arc<dyn librefang_wire::peer::PeerHandle> = self.self_arc();

        // SECURITY (#3873, PR-4): Pass data_dir so the persistent
        // TrustedPeers store is hydrated on boot and updated whenever a
        // new peer is pinned via TOFU. Pins now survive daemon restarts.
        match PeerNode::start_with_identity(
            peer_config,
            registry.clone(),
            handle.clone(),
            keypair,
            Some(self.data_dir_boot.clone()),
        )
        .await
        {
            Ok((node, _accept_task)) => {
                let addr = node.local_addr();
                info!(
                    node_id = %node_id,
                    listen = %addr,
                    "OFP peer node started"
                );

                // Safe one-time initialization via OnceLock (replaces previous unsafe pointer mutation).
                let _ = self.mesh.peer_registry.set(registry.clone());
                let _ = self.mesh.peer_node.set(node.clone());

                // Connect to bootstrap peers
                for peer_addr_str in &cfg.network.bootstrap_peers {
                    // Parse the peer address — support both multiaddr and plain formats
                    let peer_addr: Option<std::net::SocketAddr> = if peer_addr_str.starts_with('/')
                    {
                        let parts: Vec<&str> = peer_addr_str.split('/').collect();
                        let ip = parts.get(2).unwrap_or(&"127.0.0.1");
                        let port = parts.get(4).unwrap_or(&"9090");
                        format!("{ip}:{port}").parse().ok()
                    } else {
                        peer_addr_str.parse().ok()
                    };

                    if let Some(addr) = peer_addr {
                        match node.connect_to_peer(addr, handle.clone()).await {
                            Ok(()) => {
                                info!(peer = %addr, "OFP: connected to bootstrap peer");
                            }
                            Err(e) => {
                                warn!(peer = %addr, error = %e, "OFP: failed to connect to bootstrap peer");
                            }
                        }
                    } else {
                        warn!(addr = %peer_addr_str, "OFP: invalid bootstrap peer address");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "OFP: failed to start peer node");
            }
        }
    }

    /// Get the kernel's strong Arc reference from the stored weak handle.
    fn self_arc(self: &Arc<Self>) -> Arc<Self> {
        Arc::clone(self)
    }

    ///
    /// Periodically checks all running agents' last_active timestamps and
    /// publishes `HealthCheckFailed` events for unresponsive agents.
    fn start_heartbeat_monitor(self: &Arc<Self>) {
        use crate::heartbeat::{check_agents, is_quiet_hours, HeartbeatConfig};
        use std::collections::HashSet;

        let kernel = Arc::clone(self);
        let config = HeartbeatConfig::from_toml(&kernel.config.load().heartbeat);
        let interval_secs = config.check_interval_secs;

        spawn_logged("heartbeat_monitor", async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(config.check_interval_secs));
            // Track which agents are already known-unresponsive to avoid
            // spamming repeated WARN logs and HealthCheckFailed events.
            let mut known_unresponsive: HashSet<AgentId> = HashSet::new();

            loop {
                interval.tick().await;

                if kernel.agents.supervisor.is_shutting_down() {
                    info!("Heartbeat monitor stopping (shutdown)");
                    break;
                }

                let statuses = check_agents(&kernel.agents.registry, &config);
                for status in &statuses {
                    // Skip agents in quiet hours (per-agent config)
                    if let Some(entry) = kernel.agents.registry.get(status.agent_id) {
                        if let Some(ref auto_cfg) = entry.manifest.autonomous {
                            if let Some(ref qh) = auto_cfg.quiet_hours {
                                if is_quiet_hours(qh) {
                                    continue;
                                }
                            }
                        }
                    }

                    if status.unresponsive {
                        // Only warn and publish event on the *transition* to unresponsive
                        if known_unresponsive.insert(status.agent_id) {
                            warn!(
                                agent = %status.name,
                                inactive_secs = status.inactive_secs,
                                "Agent is unresponsive"
                            );
                            let event = Event::new(
                                status.agent_id,
                                EventTarget::System,
                                EventPayload::System(SystemEvent::HealthCheckFailed {
                                    agent_id: status.agent_id,
                                    unresponsive_secs: status.inactive_secs as u64,
                                }),
                            );
                            kernel.events.event_bus.publish(event).await;

                            // Fan out to operator notification channels
                            // (notification.alert_channels and matching
                            // notification.agent_rules) so the same delivery
                            // path that handles tool_failure / task_failed
                            // also surfaces unresponsive-agent alerts. Routing
                            // and event-type matching live in
                            // push_notification; the event_type to use in
                            // agent_rules.events is "health_check_failed".
                            let msg = format!(
                                "Agent \"{}\" is unresponsive (inactive for {}s)",
                                status.name, status.inactive_secs,
                            );
                            // health_check_failed is agent-level, not
                            // session-scoped — pass None so the alert
                            // doesn't get a misleading [session=…] suffix.
                            kernel
                                .push_notification(
                                    &status.agent_id.to_string(),
                                    "health_check_failed",
                                    &msg,
                                    None,
                                )
                                .await;
                        }
                    } else {
                        // Agent recovered — remove from known-unresponsive set
                        if known_unresponsive.remove(&status.agent_id) {
                            info!(
                                agent = %status.name,
                                "Agent recovered from unresponsive state"
                            );
                        }
                    }
                }
            }
        });

        info!("Heartbeat monitor started (interval: {}s)", interval_secs);
    }

    /// Start the background loop / register triggers for a single agent.
    pub fn start_background_for_agent(
        self: &Arc<Self>,
        agent_id: AgentId,
        name: &str,
        schedule: &ScheduleMode,
    ) {
        // For proactive agents, auto-register triggers from conditions.
        // Skip patterns already present (loaded from trigger_jobs.json on restart).
        if let ScheduleMode::Proactive { conditions } = schedule {
            let mut registered = false;
            for condition in conditions {
                if let Some(pattern) = background::parse_condition(condition) {
                    if self
                        .workflows
                        .triggers
                        .agent_has_pattern(agent_id, &pattern)
                    {
                        continue;
                    }
                    let prompt = format!(
                        "[PROACTIVE ALERT] Condition '{condition}' matched: {{{{event}}}}. \
                         Review and take appropriate action. Agent: {name}"
                    );
                    self.workflows
                        .triggers
                        .register(agent_id, pattern, prompt, 0);
                    registered = true;
                }
            }
            if registered {
                if let Err(e) = self.workflows.triggers.persist() {
                    warn!(agent = %name, id = %agent_id, "Failed to persist proactive triggers: {e}");
                }
                info!(agent = %name, id = %agent_id, "Registered proactive triggers");
            }
        }

        // Start continuous/periodic loops.
        //
        // RBAC carve-out (issue #3243): autonomous ticks have no inbound
        // user. Without a synthetic `SenderContext { channel:"autonomous" }`
        // the runtime would call `resolve_user_tool_decision(.., None, None)`
        // → `guest_gate` → `NeedsApproval` for any non-safe-list tool, and
        // every tick would flood the approval queue when `[[users]]` is
        // configured. The `"autonomous"` channel sentinel matches the same
        // `system_call=true` carve-out as cron (see
        // `resolve_user_tool_decision` in this file).
        let kernel = Arc::clone(self);
        self.workflows
            .background
            .start_agent(agent_id, name, schedule, move |aid, msg| {
                let k = Arc::clone(&kernel);
                tokio::spawn(async move {
                    let sender = SenderContext {
                        channel: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
                        user_id: aid.to_string(),
                        display_name: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
                        is_group: false,
                        was_mentioned: false,
                        thread_id: None,
                        account_id: None,
                        is_internal_cron: false,
                        ..Default::default()
                    };
                    match k.send_message_with_sender_context(aid, &msg, &sender).await {
                        Ok(_) => {}
                        Err(e) => {
                            // send_message already records the panic in supervisor,
                            // just log the background context here
                            warn!(agent_id = %aid, error = %e, "Background tick failed");
                        }
                    }
                })
            });
    }

    /// Gracefully shutdown the kernel.
    ///
    /// This cleanly shuts down in-memory state but preserves persistent agent
    /// data so agents are restored on the next boot.
    pub fn shutdown(&self) {
        info!("Shutting down LibreFang kernel...");

        // Signal background tasks to stop (e.g., approval expiry sweep)
        let _ = self.shutdown_tx.send(true);

        // Kill WhatsApp gateway child process if running
        if let Ok(guard) = self.whatsapp_gateway_pid.lock() {
            if let Some(pid) = *guard {
                info!("Stopping WhatsApp Web gateway (PID {pid})...");
                // Best-effort kill — don't block shutdown on failure
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();
                }
            }
        }

        self.agents.supervisor.shutdown();

        // Drain in-flight workflow runs (#3335). `Running` / `Pending`
        // are deliberately not persisted by `persist_runs` (no durable
        // boundary), so without this the dashboard would silently lose
        // every workflow that happened to be mid-flight at stop time.
        // Transition them to `Paused` with a fresh resume_token so the
        // operator (or the stale-timeout sweep at next boot) can decide
        // whether to resume or fail them.
        //
        // Run *after* `supervisor.shutdown()` has signalled the
        // supervisor to stop accepting new work. Concurrent in-flight
        // agent loops may still mutate runs during this drain — DashMap
        // per-entry locks keep individual writes coherent, but the
        // final on-disk state reflects whichever side wrote last.
        // Best-effort by design; the daemon process is exiting
        // immediately after this block, and the next-boot
        // `recover_stale_running_runs` sweep is the safety net for any
        // crash-shutdown residue.
        let drained = self.workflows.engine.drain_on_shutdown();
        if drained > 0 {
            tracing::info!(drained, "Paused in-flight workflow runs for shutdown");
        }

        // Update agent states to Suspended in persistent storage (not delete).
        // Track failures so we can emit a single critical summary if any
        // agent could not be persisted — without this, a partial-shutdown
        // would leave on-disk state at the old `Running` value with only a
        // per-agent error in the log, easy to miss (#3665).
        let mut total = 0usize;
        let mut state_failures = 0usize;
        let mut save_failures = 0usize;
        for entry in self.agents.registry.list() {
            total += 1;
            if let Err(e) = self
                .agents
                .registry
                .set_state(entry.id, AgentState::Suspended)
            {
                state_failures += 1;
                tracing::error!(agent_id = %entry.id, "failed to set agent state to Suspended on shutdown: {e}");
            }
            // Re-save with Suspended state for clean resume on next boot
            if let Some(updated) = self.agents.registry.get(entry.id) {
                if let Err(e) = self.memory.substrate.save_agent(&updated) {
                    save_failures += 1;
                    tracing::error!(agent_id = %entry.id, "failed to persist agent state on shutdown: {e}");
                }
            }
        }

        if state_failures > 0 || save_failures > 0 {
            tracing::error!(
                total_agents = total,
                state_failures,
                save_failures,
                "Kernel shutdown completed with persistence errors — some agents \
                 may resume in stale state on next boot. Inspect data/agents.* \
                 before restarting."
            );
        }

        info!(
            "LibreFang kernel shut down ({} agents preserved)",
            self.agents.registry.list().len()
        );
    }
}
