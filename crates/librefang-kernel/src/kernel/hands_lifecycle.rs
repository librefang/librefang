//! Cluster pulled out of mod.rs in #4713 phase 3c.
//!
//! Hosts the Hand instance lifecycle: activation, deactivation, reload,
//! pause/resume, the runtime-override pipeline (per-agent model knobs
//! injected on top of the manifest), and the persistence helpers that
//! checkpoint hand state to disk.
//!
//! Sibling submodule of `kernel::mod`. Public methods retain their
//! existing visibility because they form the kernel's outward "manage
//! hands" surface — the API crate and external callers reach them
//! unchanged. Private helpers stay private; their only callers are
//! other methods inside this cluster.

use std::sync::Arc;

use librefang_types::agent::AgentId;

use crate::error::KernelResult;

use super::*;

/// Floor for `exec_policy.timeout_secs` materialized onto a hand agent that inherits the operator's global `[exec_policy]` (#6594).
///
/// Hand activation used to hardcode a 300s / 120s timeout pair alongside a fabricated `Full` mode.
/// The mode is now inherited from global config, but inheriting the timeouts wholesale would cut long-running hand commands short at the 30s `ExecPolicy::default()` value, so the historically generous numbers survive as a floor.
/// Timeouts are not a security property; `mode` is, which is why only the latter is inherited verbatim.
const HAND_EXEC_TIMEOUT_FLOOR_SECS: u64 = 300;

/// Floor for `exec_policy.no_output_timeout_secs` on the same path — see [`HAND_EXEC_TIMEOUT_FLOOR_SECS`].
const HAND_EXEC_NO_OUTPUT_TIMEOUT_FLOOR_SECS: u64 = 120;

fn lock_hand_instance_recover(
    lock: &std::sync::Mutex<()>,
    instance_id: uuid::Uuid,
) -> std::sync::MutexGuard<'_, ()> {
    lock.lock().unwrap_or_else(|poisoned| {
        warn!(
            instance = %instance_id,
            "hand_instance_lock poisoned, recovering write serialization"
        );
        lock.clear_poison();
        poisoned.into_inner()
    })
}

impl LibreFangKernel {
    /// Activate a hand: check requirements, create instance, spawn agent.
    ///
    /// When `instance_id` is `Some`, the instance is created with that UUID
    /// so that deterministic agent IDs remain stable across daemon restarts.
    pub fn activate_hand(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> KernelResult<librefang_hands::HandInstance> {
        self.activate_hand_with_id(
            hand_id,
            config,
            std::collections::BTreeMap::new(),
            None,
            None,
        )
    }

    /// Like [`activate_hand`](Self::activate_hand) but allows specifying an
    /// existing instance UUID (used during daemon restart recovery).
    pub fn activate_hand_with_id(
        &self,
        hand_id: &str,
        mut config: std::collections::HashMap<String, serde_json::Value>,
        agent_runtime_overrides: std::collections::BTreeMap<
            String,
            librefang_hands::HandAgentRuntimeOverride,
        >,
        instance_id: Option<uuid::Uuid>,
        timestamps: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    ) -> KernelResult<librefang_hands::HandInstance> {
        let cfg = self.config.load();

        let def = self
            .skills
            .hand_registry
            .get_definition(hand_id)
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::AgentNotFound(format!(
                    "Hand not found: {hand_id}"
                )))
            })?
            .clone();

        // Check requirements — warn but don't block activation.
        // Hands can still be activated and paused (pre-install); the user
        // gets a degraded experience until dependencies are installed.
        if let Ok(results) = self.skills.hand_registry.check_requirements(hand_id) {
            let missing: Vec<_> = results
                .iter()
                .filter(|(_, satisfied)| !satisfied)
                .map(|(req, _)| req.label.clone())
                .collect();
            if !missing.is_empty() {
                warn!(
                    hand = %hand_id,
                    "Hand has unsatisfied requirements (degraded): {}",
                    missing.join(", ")
                );
            }
        }

        // Seed schema defaults so persisted state matches what
        // `resolve_settings` shows. Lets schema default changes require an
        // explicit operator action and disambiguates "accepted default" from
        // "never reviewed" on disk.
        for setting in &def.settings {
            config
                .entry(setting.key.clone())
                .or_insert_with(|| serde_json::Value::String(setting.default.clone()));
        }

        // Pre-flight (#5956): validate every role's manifest is spawnable
        // BEFORE creating the instance or killing any existing agents. The
        // bulk of spawn failures — module-path escape, reserved name,
        // missing tool_exec backend — are pure, side-effect-free checks.
        // Running them up front means a malformed hand definition is
        // rejected while the previously-active agents and their cron jobs /
        // triggers are still intact, instead of after the reactivation kill
        // loop below has already torn them down. Validate against the same
        // `{hand_id}:` prefixed name the spawn loop assigns.
        for hand_agent in def.agents.values() {
            let prefixed_name = format!("{hand_id}:{}", hand_agent.manifest.name);
            self.validate_spawnable(&hand_agent.manifest, &prefixed_name)?;
        }

        // Create the instance in the registry
        let instance = self
            .skills.hand_registry
            .activate_with_id(
                hand_id,
                config,
                agent_runtime_overrides,
                instance_id,
                timestamps,
            )
            // #3711: propagate the typed `HandError` instead of collapsing
            // it to `LibreFangError::Internal(String)`. Display output is
            // preserved by `#[error(transparent)]` on `KernelError::Hand`,
            // so existing log/UI strings remain identical while upstream
            // callers gain the ability to match on the typed variant
            // (e.g., `AlreadyActive` → 409 Conflict).
            .map_err(KernelError::from)?;

        // Serialize the rest of activation against `update_hand_config` and the runtime-override paths: from here on we read this instance's config and write the live `AgentRegistry` entries it owns, and a settings save interleaving with the spawn loop would commit new values and then have them overwritten by the prompt this pass renders from its own snapshot.
        // Taken after `activate_with_id` because the instance id is only known once the instance exists.
        let lock = self.hand_instance_lock(instance.instance_id);
        let _instance_guard = lock_hand_instance_recover(&lock, instance.instance_id);

        // Re-read under the lock: a save that committed between `activate_with_id` and the lock acquisition is in the registry but not in the snapshot above, and rendering from the stale copy is exactly the #6636 symptom.
        let instance = self
            .skills
            .hand_registry
            .get_instance(instance.instance_id)
            .unwrap_or(instance);

        // Pre-compute shared overrides from hand definition.
        // The system-prompt tail is materialized later (after per-role manifest cloning) via `rerender_hand_prompt_tails` — keep this block aligned with the env-var allowlist only.
        // `resolve_hand_allowed_env` carries the security rationale for the blocklist filter and is shared with `update_hand_config` so the two paths cannot drift.
        let allowed_env = resolve_hand_allowed_env(&def, &instance.config);

        let is_multi_agent = def.is_multi_agent();
        let coordinator_role = def.coordinator().map(|(role, _)| role.to_string());

        // Kill existing agents with matching hand tag (reactivation cleanup)
        let hand_tag = format!("hand:{hand_id}");
        let mut saved_triggers = std::collections::BTreeMap::new();
        // Snapshot cron jobs per-role BEFORE kill_agent destroys them.
        // kill_agent calls remove_agent_jobs() which deletes the jobs from
        // memory and persists an empty cron_jobs.json to disk. The
        // reassign_agent_jobs() call below would always be a no-op without
        // this snapshot — same pattern as saved_triggers above. Fixes the
        // silent loss of cron jobs across every daemon restart for
        // hand-style agents.
        let mut saved_crons: std::collections::BTreeMap<
            String,
            Vec<librefang_types::scheduler::CronJob>,
        > = std::collections::BTreeMap::new();
        for entry in self.agents.registry.list() {
            if entry.tags.contains(&hand_tag) {
                let old_id = entry.id;
                // Extract role from tag (hand_role:xxx) to migrate cron to correct new agent
                let old_role = entry
                    .tags
                    .iter()
                    .find_map(|t| t.strip_prefix("hand_role:"))
                    .unwrap_or("main")
                    .to_string();
                let taken_triggers = self.workflows.triggers.take_agent_triggers(entry.id);
                if !taken_triggers.is_empty() {
                    saved_triggers
                        .entry(old_role.clone())
                        .or_insert_with(Vec::new)
                        .extend(taken_triggers);
                }
                let taken_crons = self.workflows.cron_scheduler.list_jobs(old_id);
                if !taken_crons.is_empty() {
                    // Dedupe by job id within this snapshot: if two registry
                    // entries somehow tag the same role (concurrent activation
                    // racing the `kill_agent` cleanup, or a bug that left two
                    // tagged agents alive), the same `CronJob` could be
                    // collected twice and re-added twice — yielding duplicate
                    // jobs that fire side-by-side. Deterministically keep
                    // exactly one per `CronJobId`.
                    let bucket: &mut Vec<librefang_types::scheduler::CronJob> =
                        saved_crons.entry(old_role.clone()).or_default();
                    let seen: std::collections::HashSet<librefang_types::scheduler::CronJobId> =
                        bucket.iter().map(|j| j.id).collect();
                    bucket.extend(taken_crons.into_iter().filter(|j| !seen.contains(&j.id)));
                }
                if let Err(e) = self.kill_agent(old_id) {
                    warn!(agent = %old_id, error = %e, "Failed to kill old hand agent");
                }
                // Belt-and-braces: also reassign any jobs that somehow still
                // reference the old UUID. After kill_agent's remove_agent_jobs
                // wipes everything, this is a no-op in practice — the snapshot
                // above is the primary mechanism. Kept as a safety net for
                // edge cases like out-of-band cron creation between kill and
                // respawn.
                let new_id = AgentId::from_hand_agent(hand_id, &old_role, instance_id);
                let migrated = self
                    .workflows
                    .cron_scheduler
                    .reassign_agent_jobs(old_id, new_id);
                if migrated > 0 {
                    // Without this, the migrated cron→agent mappings live
                    // only in memory and are lost on the next restart, so a
                    // reassigned hand agent silently stops firing its cron
                    // jobs. Mirror the warn pattern used at the
                    // restoration-persist site below.
                    if let Err(e) = self.workflows.cron_scheduler.persist() {
                        warn!(
                            error = %e,
                            "Failed to persist cron jobs after hand-agent reassignment"
                        );
                    }
                }
            }
        }

        // Spawn an agent for each role in the hand definition
        let mut agent_ids_map = std::collections::BTreeMap::new();
        let mut last_manifest_path = None;
        // Captures the first per-role spawn failure so rollback is handled
        // once after the loop instead of via an in-loop `return` (which was
        // the structural reason the snapshot was silently dropped — #5956).
        let mut spawn_error: Option<KernelError> = None;

        for (role, hand_agent) in &def.agents {
            let mut manifest = hand_agent.manifest.clone();
            let runtime_override = instance.agent_runtime_overrides.get(role).cloned();

            // Prefix hand agent name with hand_id to avoid colliding with
            // standalone specialist agents spawned by routing.
            manifest.name = format!("{hand_id}:{}", manifest.name);

            // Reuse existing hand agent if one with the same prefixed name is already running.
            // NOTE: this check-then-spawn is not atomic, but is safe because hand activation
            // is serialized by the activate_lock mutex at the HandRegistry level.
            if let Some(existing) = self.agents.registry.find_by_name(&manifest.name) {
                agent_ids_map.insert(role.clone(), existing.id);
                continue;
            }

            // Inherit kernel defaults when hand declares "default" sentinel.
            // Provider and model are resolved independently so that a hand
            // can pin one while inheriting the other (e.g. provider="openai"
            // with model="default" inherits the global default model name).
            //
            // When inheriting provider, also fill api_key_env / base_url
            // from global config — but only if the hand didn't set them
            // explicitly, to preserve legacy HAND.toml credential overrides.
            if manifest.model.provider == "default" {
                manifest.model.provider = cfg.default_model.provider.clone();
                if manifest.model.api_key_env.is_none() {
                    manifest.model.api_key_env = Some(cfg.default_model.api_key_env.clone());
                }
                if manifest.model.base_url.is_none() {
                    manifest.model.base_url = cfg.default_model.base_url.clone();
                }
            }
            if manifest.model.model == "default" {
                manifest.model.model = cfg.default_model.model.clone();
            }

            // Merge extra_params from default_model (agent-level keys take precedence)
            for (key, value) in &cfg.default_model.extra_params {
                manifest
                    .model
                    .extra_params
                    .entry(key.clone())
                    .or_insert(value.clone());
            }

            // Hand-level tool inheritance: hand controls WHICH tools are available,
            // but preserve agent-level capability fields (network, shell, memory, etc.)
            let mut tools = def.tools.clone();
            if is_multi_agent && !tools.contains(&"agent_send".to_string()) {
                tools.push("agent_send".to_string());
            }
            manifest.capabilities.tools = tools;

            // Tags: append hand-level tags to agent's existing tags
            manifest.tags.extend([
                format!("hand:{hand_id}"),
                format!("hand_instance:{}", instance.instance_id),
                format!("hand_role:{role}"),
            ]);
            manifest.is_hand = true;

            // Skills merge semantics:
            //   hand skills = []  (empty)     → no restriction, agent keeps its own list
            //   hand skills = ["a", "b"]      → allowlist; agent list is intersected
            //   hand skills = ["a"] + agent [] → agent gets hand's list
            //   hand skills = ["a"] + agent ["a","c"] → agent gets ["a"] (intersection)
            if !def.skills.is_empty() {
                if manifest.skills.is_empty() {
                    // Agent has no preference → use hand allowlist
                    manifest.skills = def.skills.clone();
                } else {
                    // Agent has its own list → intersect with hand allowlist
                    manifest.skills.retain(|s| def.skills.contains(s));
                }
            }

            // MCP servers: same merge logic as skills
            if !def.mcp_servers.is_empty() {
                if manifest.mcp_servers.is_empty() {
                    manifest.mcp_servers = def.mcp_servers.clone();
                } else {
                    manifest.mcp_servers.retain(|s| def.mcp_servers.contains(s));
                }
            }

            // Plugins: same merge logic as skills/mcp_servers
            if !def.allowed_plugins.is_empty() {
                if manifest.allowed_plugins.is_empty() {
                    manifest.allowed_plugins = def.allowed_plugins.clone();
                } else {
                    manifest
                        .allowed_plugins
                        .retain(|p| def.allowed_plugins.contains(p));
                }
            }

            // Scheduling (#6595): activation deliberately does not touch `manifest.schedule`.
            // It used to synthesize `ScheduleMode::Continuous` whenever `manifest.autonomous` was present, but `AutonomousConfig` is also the carrier for the flat HAND.toml `max_iterations` field — the agent-loop iteration cap resolved in `librefang_runtime::agent_loop` — so a hand author asking for a loop-depth cap got a permanent 30s wake-up cycle (the `heartbeat_interval_secs` default) they never declared.
            //
            // A role now leaves `ScheduleMode::Reactive` only if its own manifest says so: an explicit `schedule` (the only route to the `Periodic` cron and `Proactive` variants) or an explicit `[autonomous]` block, which `librefang_hands::apply_explicit_autonomous_schedule` resolves into `Continuous` at parse time, where the raw TOML still distinguishes an author-written block from the synthesized one.
            // Honouring `schedule` verbatim is also what `spawn_agent_inner` does for a standalone agent, so hand roles and standalone agents no longer disagree about what the field means.

            // Shell exec policy (#6594): a hand definition is third-party content, so activation must never hand out a stronger `mode` than the operator configured globally.
            // Inherit the global `[exec_policy]` instead of fabricating `Full`; a hand that genuinely needs elevated exec opts in by declaring its own `[exec_policy]`, which the `declared_mode.is_none()` guard below respects.
            //
            // Only the timeouts deviate from global, and only upwards — see `HAND_EXEC_TIMEOUT_FLOOR_SECS` for why.
            //
            // The trigger set must match `spawn_agent_inner`'s exec-policy fallback (`shell_exec` OR the `*` wildcard, read from `capabilities.tools`) exactly.
            // That fallback still promotes an agent with no `exec_policy` to `Full`, so leaving `None` on any manifest it would match — a hand declaring `tools = ["*"]`, say — just relocates the escalation instead of removing it.
            // `manifest.capabilities.tools` was assigned from `def.tools` above, and is what the spawn path actually reads.
            if manifest
                .capabilities
                .tools
                .iter()
                .any(|t| t == "shell_exec" || t == "*")
            {
                let declared_mode = manifest.exec_policy.as_ref().map(|p| p.mode);
                let effective_mode = declared_mode.unwrap_or(cfg.exec_policy.mode);
                let policy_source = if declared_mode.is_some() {
                    "hand manifest"
                } else {
                    "global config"
                };
                if declared_mode.is_none() {
                    manifest.exec_policy = Some(librefang_types::config::ExecPolicy {
                        timeout_secs: cfg
                            .exec_policy
                            .timeout_secs
                            .max(HAND_EXEC_TIMEOUT_FLOOR_SECS),
                        no_output_timeout_secs: cfg
                            .exec_policy
                            .no_output_timeout_secs
                            .max(HAND_EXEC_NO_OUTPUT_TIMEOUT_FLOOR_SECS),
                        ..cfg.exec_policy.clone()
                    });
                }
                // Surface the resolved mode at activation time.
                // Without this the only signal was the per-call "Shell exec in full mode" warning, long after the operator could react.
                // The wording is mode-neutral on purpose: this same line fires when the inherited mode is `Deny`, so it must not claim exec was granted.
                info!(
                    hand = %hand_id,
                    role = %role,
                    agent = %manifest.name,
                    exec_mode = ?effective_mode,
                    policy_source,
                    "Hand agent exec_policy resolved"
                );
            }

            if !def.tools.is_empty() {
                manifest.profile = Some(ToolProfile::Custom);
            }

            if let Some(runtime_override) = runtime_override {
                if let Some(provider) = runtime_override.provider {
                    manifest.model.provider = provider;
                }
                if let Some(model) = runtime_override.model {
                    manifest.model.model = model;
                }
                if let Some(api_key_env) = runtime_override.api_key_env {
                    manifest.model.api_key_env = api_key_env;
                }
                if let Some(base_url) = runtime_override.base_url {
                    manifest.model.base_url = base_url;
                }
                if let Some(max_tokens) = runtime_override.max_tokens {
                    manifest.model.max_tokens = max_tokens;
                }
                if let Some(temperature) = runtime_override.temperature {
                    manifest.model.temperature = temperature;
                }
                if let Some(mode) = runtime_override.web_search_augmentation {
                    manifest.web_search_augmentation = mode;
                }
            }

            // Render the whole prompt — base plus the settings, reference-knowledge and team tails — through the canonical helper.
            // The boot-time TOML drift loop and the settings-save path call the same function, so all three agree on content and ordering; the drift loop overwrites the DB blob with the bare disk TOML, which carries none of the tails, and without an identical re-render there the agent would silently lose its configured values, skill discoverability and peer awareness on every restart.
            //
            // `allowed_env` was resolved once above from the same (definition, config) pair, so the returned value is identical per role; it is discarded rather than shadowing the outer binding to keep that obvious.
            let _ = rerender_hand_prompt_tails(&mut manifest, role, &def, &instance.config);
            set_hand_allowed_env(&mut manifest, &allowed_env);

            // Hand workspace: workspaces/<hand-id>/
            // Agent workspace nested under hand: workspaces/hands/<hand-id>/<role>/
            let safe_hand = safe_path_component(hand_id, "hand");
            let hand_dir = cfg.effective_hands_workspaces_dir().join(&safe_hand);

            // Write hand definition to workspace
            let hand_toml_path = hand_dir.join("hand.toml");
            if !hand_toml_path.exists() {
                if let Err(e) = std::fs::create_dir_all(&hand_dir) {
                    warn!(path = %hand_dir.display(), "Failed to create dir: {e}");
                } else {
                    match toml::to_string_pretty(&def) {
                        Ok(toml_str) => {
                            // The workspace hand.toml is an informational
                            // mirror of the canonical in-memory registry
                            // (set via `hand_registry.set_agents` below). A
                            // failed write previously vanished silently, so
                            // the on-disk copy was missing with no trace.
                            if let Err(e) = std::fs::write(&hand_toml_path, &toml_str) {
                                warn!(
                                    path = %hand_toml_path.display(),
                                    error = %e,
                                    "Failed to write hand.toml to workspace"
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                hand = %hand_id,
                                error = %e,
                                "Failed to serialize hand definition to TOML"
                            );
                        }
                    }
                }
            }
            last_manifest_path = Some(hand_toml_path.clone());

            // Relative path resolved by spawn_agent_inner against workspaces root:
            // workspaces/ + hands/<hand>/<role> = workspaces/hands/<hand>/<role>/
            let safe_role = safe_path_component(role, "agent");
            manifest.workspace = Some(std::path::PathBuf::from(format!(
                "hands/{safe_hand}/{safe_role}"
            )));

            // Deterministic agent ID: hand_id + role [+ instance_id].
            // When `instance_id` is None (first activation via `activate_hand`),
            // uses the legacy format so existing hands keep their original IDs.
            // When `instance_id` is Some (multi-instance or restart recovery),
            // uses the new format with instance UUID for uniqueness.
            let deterministic_id = AgentId::from_hand_agent(hand_id, role, instance_id);
            match self.spawn_agent_inner(
                manifest,
                None,
                Some(hand_toml_path),
                Some(deterministic_id),
            ) {
                Ok(id) => {
                    agent_ids_map.insert(role.clone(), id);
                }
                Err(e) => {
                    // Defer rollback to the single post-loop handler so the
                    // `saved_triggers` / `saved_crons` snapshots are moved
                    // exactly once (the success path below consumes them).
                    spawn_error = Some(e);
                    break;
                }
            }
        }

        // If any role failed to spawn, roll back this activation. The
        // pre-flight `validate_spawnable` pass above already rejected
        // malformed definitions before the kill loop ran, so reaching here
        // means a rarer runtime failure (e.g. session creation) after the
        // previously-active agents were already torn down. Kill the
        // partially-spawned agents and deactivate the instance. The triggers
        // / cron jobs snapshotted from the killed previous agents cannot be
        // safely re-homed — the failed role has no live agent, and restoring
        // them to its deterministic id would dispatch to a dead agent every
        // tick — so they are dropped here. Log loudly rather than silently so
        // the loss is visible (#5956).
        if let Some(e) = spawn_error {
            for spawned_id in agent_ids_map.values() {
                if let Err(kill_err) = self.kill_agent(*spawned_id) {
                    warn!(
                        hand = %hand_id,
                        agent = %spawned_id,
                        error = %kill_err,
                        "Failed to rollback agent during hand activation failure"
                    );
                }
            }
            if let Err(de) = self.skills.hand_registry.deactivate(instance.instance_id) {
                warn!(
                    instance_id = %instance.instance_id,
                    error = %de,
                    "Failed to deactivate hand instance during rollback"
                );
            }
            if !saved_triggers.is_empty() || !saved_crons.is_empty() {
                let lost_trigger_roles: Vec<&str> =
                    saved_triggers.keys().map(String::as_str).collect();
                let lost_cron_roles: Vec<&str> = saved_crons.keys().map(String::as_str).collect();
                warn!(
                    hand = %hand_id,
                    error = %e,
                    ?lost_trigger_roles,
                    ?lost_cron_roles,
                    "Hand activation failed after teardown; snapshotted triggers/cron jobs \
                     for the previous agents are lost (no live agent to restore them to)"
                );
            }
            return Err(e);
        }

        // Restore saved triggers to the same role after reactivation.
        if !saved_triggers.is_empty() {
            for (role, triggers) in saved_triggers {
                if let Some(&new_id) = agent_ids_map.get(&role) {
                    let restored = self.workflows.triggers.restore_triggers(new_id, triggers);
                    if restored > 0 {
                        info!(
                            hand = %hand_id,
                            role = %role,
                            agent = %new_id,
                            restored,
                            "Restored triggers after hand reactivation"
                        );
                    }
                } else {
                    warn!(
                        hand = %hand_id,
                        role = %role,
                        "Dropping saved triggers for removed hand role during reactivation"
                    );
                }
            }
            if let Err(e) = self.workflows.triggers.persist() {
                warn!("Failed to persist trigger jobs after hand reactivation: {e}");
            }
        }

        // Restore cron jobs that were snapshotted before kill_agent. They're
        // re-added under the new agent_id for the same role. Runtime state
        // (last_run) is reset and `next_run` is recomputed from the schedule
        // so jobs resume on a clean future tick instead of immediately on
        // the next scheduler poll.
        if !saved_crons.is_empty() {
            let mut total_restored = 0usize;
            for (role, jobs) in saved_crons {
                if let Some(&new_id) = agent_ids_map.get(&role) {
                    let mut restored = 0usize;
                    for mut job in jobs {
                        job.agent_id = new_id;
                        // Compute the next future fire time from the
                        // schedule explicitly. `add_job` will overwrite this
                        // with `compute_next_run` too, but writing it here
                        // makes the intent ("don't refire immediately just
                        // because we restored") obvious to readers and
                        // resilient to future changes in `add_job`.
                        job.next_run = Some(crate::cron::compute_next_run(&job.schedule));
                        job.last_run = None;
                        if self.workflows.cron_scheduler.add_job(job, false).is_ok() {
                            restored += 1;
                        }
                    }
                    if restored > 0 {
                        info!(
                            hand = %hand_id,
                            role = %role,
                            agent = %new_id,
                            restored,
                            "Restored cron jobs after hand reactivation"
                        );
                    }
                    total_restored += restored;
                } else {
                    warn!(
                        hand = %hand_id,
                        role = %role,
                        "Dropping saved cron jobs for removed hand role during reactivation"
                    );
                }
            }
            if total_restored > 0 {
                if let Err(e) = self.workflows.cron_scheduler.persist() {
                    warn!("Failed to persist cron jobs after restoration: {e}");
                }
            }
        }

        // Link all agents to instance
        self.skills.hand_registry
            .set_agents(
                instance.instance_id,
                agent_ids_map.clone(),
                coordinator_role.clone(),
            )
            // #3711: propagate typed HandError; Display preserved by
            // `#[error(transparent)]` on `KernelError::Hand`.
            .map_err(KernelError::from)?;

        let display_manifest_path = last_manifest_path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        info!(
            hand = %hand_id,
            instance = %instance.instance_id,
            agents = %agent_ids_map.len(),
            source = %display_manifest_path,
            "Hand activated with agent(s)"
        );

        // Persist hand state so it survives restarts
        self.persist_hand_state();

        // Return instance with agent set
        Ok(self
            .skills
            .hand_registry
            .get_instance(instance.instance_id)
            .unwrap_or(instance))
    }

    /// Deactivate a hand: kill agent and remove instance.
    pub fn deactivate_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        // Serialize against `update_hand_config` and the runtime-override paths: this tears down the very `AgentRegistry` entries a concurrent settings save is rewriting, and the save would otherwise leave a freshly rendered prompt on an agent that is being killed.
        let lock = self.hand_instance_lock(instance_id);
        let guard = lock_hand_instance_recover(&lock, instance_id);

        let instance = self
            .skills.hand_registry
            .deactivate(instance_id)
            // #3711: propagate typed HandError (Display preserved).
            .map_err(KernelError::from)?;

        // Collect every hand-agent id touched by this instance so we can both
        // kill the live runtime and scrub the persisted SQLite rows below.
        //
        // `kill_agent` already calls `memory.remove_agent` on its happy path,
        // but it bails out with `Err` at `registry.remove(agent_id)?` when the
        // agent isn't in the in-memory registry — which is exactly what
        // happens to hand-agents across a restart since the boot fix in
        // #a023519d skips `is_hand=true` rows in `load_all_agents`. On the
        // error path the SQLite row is never touched, so without the explicit
        // `memory.remove_agent` pass below the orphan accumulates every
        // deactivate/reactivate cycle.
        let mut affected_agents: Vec<AgentId> = Vec::new();
        if !instance.agent_ids.is_empty() {
            for &agent_id in instance.agent_ids.values() {
                affected_agents.push(agent_id);
                if let Err(e) = self.kill_agent(agent_id) {
                    warn!(agent = %agent_id, error = %e, "Failed to kill hand agent (may already be dead)");
                }
            }
        } else {
            // Fallback: if agent_ids was never set (incomplete activation), search by hand tag
            let hand_tag = format!("hand:{}", instance.hand_id);
            for entry in self.agents.registry.list() {
                if entry.tags.contains(&hand_tag) {
                    affected_agents.push(entry.id);
                    if let Err(e) = self.kill_agent(entry.id) {
                        warn!(agent = %entry.id, error = %e, "Failed to kill orphaned hand agent");
                    } else {
                        info!(agent_id = %entry.id, hand_id = %instance.hand_id, "Cleaned up orphaned hand agent");
                    }
                }
            }
        }

        // Remove the SQLite rows for every hand-agent we just tore down.
        // `remove_agent` cascades to session rows, so we don't need a
        // separate `delete_agent_sessions` call here.
        for agent_id in &affected_agents {
            if let Err(e) = self.memory.substrate.remove_agent(*agent_id) {
                warn!(
                    agent = %agent_id,
                    hand_id = %instance.hand_id,
                    error = %e,
                    "Failed to remove hand-agent row from SQLite on deactivate"
                );
            }
        }

        // Release before dropping the map entry: holding a mutex while deleting the entry that publishes it lets the next caller mint a fresh mutex and run concurrently with this call's tail, which is the opposite of what the lock is for.
        drop(guard);

        // Drop the per-instance mutex so reactivating with a fresh `instance_id` doesn't leak entries here.
        self.agents.hand_instance_locks.remove(&instance_id);

        // Persist hand state so it survives restarts
        self.persist_hand_state();
        Ok(())
    }

    /// Reload hand definitions from disk (hot reload).
    pub fn reload_hands(&self) -> (usize, usize) {
        let (added, updated) = self
            .skills
            .hand_registry
            .reload_from_disk(&self.home_dir_boot);
        info!(added, updated, "Reloaded hand definitions from disk");
        (added, updated)
    }

    /// Invalidate the hand route resolution cache.
    ///
    /// Thin wrapper around `librefang_kernel_router::invalidate_hand_route_cache`
    /// so API callers don't need to reach into the router crate path directly
    /// (refs #3744).
    pub fn invalidate_hand_route_cache(&self) {
        router::invalidate_hand_route_cache();
    }

    /// Persist active hand state to disk.
    pub fn persist_hand_state(&self) {
        let state_path = self.home_dir_boot.join("data").join("hand_state.json");
        if let Err(e) = self.skills.hand_registry.persist_state(&state_path) {
            warn!(error = %e, "Failed to persist hand state");
        }
    }

    fn persist_hand_state_result(&self) -> KernelResult<()> {
        let state_path = self.home_dir_boot.join("data").join("hand_state.json");
        self.skills.hand_registry
            .persist_state(&state_path)
            // #3711: propagate typed HandError (Display preserved).
            .map_err(KernelError::from)
    }

    /// Per-instance serialization lock for mutations that span the `HandInstance` and the live `AgentRegistry` entries it owns.
    /// See the field comment on `hand_instance_locks` for the race this guards against.
    fn hand_instance_lock(&self, instance_id: uuid::Uuid) -> Arc<std::sync::Mutex<()>> {
        self.agents
            .hand_instance_locks
            .entry(instance_id)
            .or_insert_with(|| Arc::new(std::sync::Mutex::new(())))
            .clone()
    }

    /// Merge a `[[settings]]` delta into a hand instance's config **and** push the result into the live agents' system prompts (#6636).
    ///
    /// Writing the instance config alone is not enough: settings only reach an LLM as the rendered `## User Configuration` tail on each role's `model.system_prompt`, and that tail is materialized at activation.
    /// Before this method existed, `PUT /api/hands/{id}/settings` updated the registry and persisted `hand_state.json`, so the new values took effect on the *next daemon restart* — boot replays every persisted hand through `activate_hand_with_id` — while a running agent kept answering from the HAND.toml defaults, with no error to explain why.
    /// For a hand like the Trading Hand, whose prompt branches on `trading_mode` and `approval_mode`, that gap is safety-relevant.
    ///
    /// **Takes a delta, not a full map.** Callers pass only the keys they are changing and the merge happens here, under the lock.
    /// A caller that read the config, merged, and passed the whole map would have its read-modify-write straddle the lock, so two concurrent saves would each write their own snapshot and the later one would silently revert the earlier — now visibly, because the revert reaches the LLM immediately.
    ///
    /// Rollback is all-or-nothing across both halves.
    /// Each role's previous prompt and env allowlist are snapshotted before it is rewritten, so a failure part-way through the loop restores the roles already touched as well as the config; otherwise the persisted file would say `paper` while a live agent ran on `live`, which is the exact disagreement this method exists to prevent.
    /// Every rollback step logs on failure rather than discarding its `Result` — a rollback that itself fails is the one case an operator most needs to see.
    ///
    /// No SQLite write: hand agents are skipped by the boot restore loop (`if entry.is_hand { continue; }`) and rebuilt from `hand_state.json` plus HAND.toml instead, so the persisted agent blob is never read back for them.
    /// Same reason the runtime-override path doesn't save either.
    ///
    /// Returns the updated instance so HTTP callers can echo the merged config back.
    pub fn update_hand_config(
        &self,
        instance_id: uuid::Uuid,
        delta: std::collections::HashMap<String, serde_json::Value>,
    ) -> KernelResult<librefang_hands::HandInstance> {
        // Existence is checked before the lock is minted: `hand_instance_lock`
        // inserts on miss, so locking first would let any caller with an arbitrary
        // UUID grow `hand_instance_locks` with an entry nothing ever removes.
        // The window this opens is harmless — the authoritative re-read below runs
        // under the lock and returns `InstanceNotFound` if the instance vanished.
        if self
            .skills
            .hand_registry
            .get_instance(instance_id)
            .is_none()
        {
            return Err(KernelError::from(
                librefang_hands::HandError::InstanceNotFound(instance_id),
            ));
        }

        let lock = self.hand_instance_lock(instance_id);
        let _guard = lock_hand_instance_recover(&lock, instance_id);

        // Authoritative read: this is both the merge base and the rollback snapshot.
        let previous = self
            .skills
            .hand_registry
            .get_instance(instance_id)
            .ok_or_else(|| {
                KernelError::from(librefang_hands::HandError::InstanceNotFound(instance_id))
            })?;

        let mut merged = previous.config.clone();
        merged.extend(delta);

        self.skills
            .hand_registry
            .update_config(instance_id, merged)
            .map_err(KernelError::from)?;

        if let Err(err) = self.persist_hand_state_result() {
            self.restore_hand_config(instance_id, previous.config, false);
            return Err(err);
        }

        let instance = self
            .skills
            .hand_registry
            .get_instance(instance_id)
            .ok_or_else(|| {
                KernelError::from(librefang_hands::HandError::InstanceNotFound(instance_id))
            })?;

        // A hand whose definition is no longer loaded (uninstalled from disk
        // while an instance lingers) has nothing to render against. The
        // config is already saved and persisted; report success rather than
        // failing a write that did land.
        let Some(def) = self.skills.hand_registry.get_definition(&instance.hand_id) else {
            warn!(
                hand = %instance.hand_id,
                instance = %instance_id,
                "Hand definition not loaded — saved settings will render on next activation"
            );
            return Ok(instance);
        };

        let allowed_env = resolve_hand_allowed_env(&def, &instance.config);

        // (agent_id, prompt, allowed_env) as they were before this pass, so a
        // mid-loop failure can put every already-rewritten role back.
        let mut rewritten: Vec<(AgentId, String, Vec<String>)> = Vec::new();

        for (role, agent_id) in &instance.agent_ids {
            // A role recorded on the instance but absent from the current
            // definition (HAND.toml renamed a role and `POST /api/hands/reload`
            // ran without respawning) cannot be rendered: the team helper would
            // advertise peers under names that no longer exist and the skill
            // helper would substitute the hand-shared playbook for the role's own.
            // Skip loudly, as `clear_hand_agent_runtime_override` does.
            if !def.agents.contains_key(role) {
                warn!(
                    hand = %instance.hand_id,
                    role = %role,
                    agent = %agent_id,
                    "Hand definition has no entry for role; skipping prompt re-render \
                     (re-activate the hand to resynchronise its agents)"
                );
                continue;
            }

            let Some(entry) = self.agents.registry.get(*agent_id) else {
                // Role recorded on the instance but no live agent — e.g. the
                // hand is paused, or a spawn failed. The next activation
                // renders from the config we just persisted.
                continue;
            };
            let previous_prompt = entry.manifest.model.system_prompt.clone();
            let previous_env: Vec<String> = entry
                .manifest
                .metadata
                .get("hand_allowed_env")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let mut manifest = entry.manifest.clone();
            rerender_hand_prompt_tails(&mut manifest, role, &def, &instance.config);
            match self.agents.registry.update_hand_rendered_prompt(
                *agent_id,
                manifest.model.system_prompt,
                allowed_env.clone(),
            ) {
                Ok(()) => rewritten.push((*agent_id, previous_prompt, previous_env)),
                Err(e) => {
                    // Put the roles already rewritten in this pass back before
                    // reverting the config, so neither half is left on the new
                    // values.
                    for (done_id, prompt, env) in rewritten {
                        if let Err(restore_err) = self
                            .agents
                            .registry
                            .update_hand_rendered_prompt(done_id, prompt, env)
                        {
                            warn!(
                                agent = %done_id,
                                error = %restore_err,
                                "Failed to restore hand agent prompt after a partial settings \
                                 re-render; this agent is running on settings the persisted \
                                 config does not reflect until the hand is re-activated"
                            );
                        }
                    }
                    self.restore_hand_config(instance_id, previous.config, true);
                    return Err(KernelError::LibreFang(e));
                }
            }
        }

        info!(
            hand = %instance.hand_id,
            instance = %instance_id,
            roles = instance.agent_ids.len(),
            rewritten = rewritten.len(),
            "Hand settings saved and re-rendered into live agent prompts"
        );
        Ok(instance)
    }

    /// Put a hand instance's config back after a failed [`Self::update_hand_config`], logging rather than discarding either step's error.
    ///
    /// `repersist` is false on the persist-failure path — the write that just failed is the reason we are here, and retrying it would only produce a second identical error; the in-memory revert alone restores agreement with whatever is on disk.
    fn restore_hand_config(
        &self,
        instance_id: uuid::Uuid,
        previous: std::collections::HashMap<String, serde_json::Value>,
        repersist: bool,
    ) {
        if let Err(e) = self
            .skills
            .hand_registry
            .update_config(instance_id, previous)
        {
            warn!(
                instance = %instance_id,
                error = %e,
                "Failed to roll back hand settings; the in-memory config keeps values the \
                 save did not fully apply"
            );
            return;
        }
        if repersist {
            if let Err(e) = self.persist_hand_state_result() {
                warn!(
                    instance = %instance_id,
                    error = %e,
                    "Rolled back hand settings in memory but could not re-persist \
                     hand_state.json; a restart will replay the values the save did not apply"
                );
            }
        }
    }

    fn apply_hand_agent_runtime_override_to_registry(
        &self,
        agent_id: AgentId,
        default_manifest: &librefang_types::agent::AgentManifest,
        merged: &librefang_hands::HandAgentRuntimeOverride,
    ) -> KernelResult<()> {
        if merged.model.is_some()
            || merged.provider.is_some()
            || merged.api_key_env.is_some()
            || merged.base_url.is_some()
        {
            let (default_model, default_provider, default_api_key_env, default_base_url) =
                self.resolve_hand_agent_model_defaults(default_manifest);
            self.agents
                .registry
                .update_model_provider_config(
                    agent_id,
                    merged.model.clone().unwrap_or(default_model),
                    merged.provider.clone().unwrap_or(default_provider),
                    merged.api_key_env.clone().unwrap_or(default_api_key_env),
                    merged.base_url.clone().unwrap_or(default_base_url),
                )
                .map_err(KernelError::LibreFang)?;
        }
        if let Some(max_tokens) = merged.max_tokens {
            self.agents
                .registry
                .update_max_tokens(agent_id, max_tokens)
                .map_err(KernelError::LibreFang)?;
        }
        if let Some(temperature) = merged.temperature {
            self.agents
                .registry
                .update_temperature(agent_id, temperature)
                .map_err(KernelError::LibreFang)?;
        }
        if let Some(mode) = merged.web_search_augmentation {
            self.agents
                .registry
                .update_web_search_augmentation(agent_id, mode)
                .map_err(KernelError::LibreFang)?;
        }
        Ok(())
    }

    fn resolve_hand_agent_model_defaults(
        &self,
        manifest: &librefang_types::agent::AgentManifest,
    ) -> (String, String, Option<String>, Option<String>) {
        let cfg = self.config.load();
        let mut provider = manifest.model.provider.clone();
        let mut model = manifest.model.model.clone();
        let mut api_key_env = manifest.model.api_key_env.clone();
        let mut base_url = manifest.model.base_url.clone();
        if provider == "default" {
            provider = cfg.default_model.provider.clone();
            if api_key_env.is_none() {
                api_key_env = Some(cfg.default_model.api_key_env.clone());
            }
            if base_url.is_none() {
                base_url = cfg.default_model.base_url.clone();
            }
        }
        if model == "default" {
            model = cfg.default_model.model.clone();
        }
        // Match the spawn-time normalization in `spawn_agent` (~line 3802):
        // a `provider/model` or `provider:model` model id collapses to bare
        // `model`. Without this, clear/update over a default-resolved model
        // (e.g. cfg.default_model.model = "claude-code/sonnet" + provider
        // "claude-code") leaves the live AgentRegistry holding the prefixed
        // form while spawn stored the bare form — the two paths disagree,
        // and `clear_hand_agent_runtime_override_resets_manifest_and_state`
        // catches it.
        let stripped = strip_provider_prefix(&model, &provider);
        if stripped != model {
            model = stripped;
        }
        (model, provider, api_key_env, base_url)
    }

    pub fn update_hand_agent_runtime_override(
        &self,
        agent_id: AgentId,
        override_config: librefang_hands::HandAgentRuntimeOverride,
    ) -> KernelResult<()> {
        let instance = self
            .skills
            .hand_registry
            .find_by_agent(agent_id)
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
            })?;
        // Serialize the entire merge → persist → apply flow per hand
        // instance. The DashMap shard lock inside
        // `merge_agent_runtime_override` only covers the merge step; without
        // this outer guard, two concurrent PATCHes can interleave their
        // `apply_hand_agent_runtime_override_to_registry` calls and leave
        // the live AgentRegistry inconsistent with `hand_state.json`.
        let lock = self.hand_instance_lock(instance.instance_id);
        let _guard = lock_hand_instance_recover(&lock, instance.instance_id);
        // Re-read the instance under the lock so any concurrent
        // mutation (e.g. an in-flight clear) is reflected in the
        // `previous` snapshot used for rollback.
        let instance = self
            .skills
            .hand_registry
            .find_by_agent(agent_id)
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
            })?;
        let role = instance
            .agent_ids
            .iter()
            .find_map(|(role, id)| (*id == agent_id).then(|| role.clone()))
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::Internal(format!(
                    "Hand role not found for agent {agent_id}"
                )))
            })?;
        let def = self
            .skills
            .hand_registry
            .get_definition(&instance.hand_id)
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::Internal(format!(
                    "Hand definition not loaded for {}",
                    instance.hand_id
                )))
            })?;
        let agent_def = def.agents.get(&role).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::Internal(format!(
                "Hand role not found for agent {agent_id}"
            )))
        })?;

        let previous = instance.agent_runtime_overrides.get(&role).cloned();
        let merged = self
            .skills.hand_registry
            .merge_agent_runtime_override(instance.instance_id, &role, override_config)
            // #3711: propagate typed HandError (Display preserved).
            .map_err(KernelError::from)?;
        if let Err(err) = self.persist_hand_state_result() {
            let _ = self.skills.hand_registry.restore_agent_runtime_override(
                instance.instance_id,
                &role,
                previous,
            );
            return Err(err);
        }
        if let Err(err) = self.apply_hand_agent_runtime_override_to_registry(
            agent_id,
            &agent_def.manifest,
            &merged,
        ) {
            if let Err(restore_err) = self.skills.hand_registry.restore_agent_runtime_override(
                instance.instance_id,
                &role,
                previous,
            ) {
                warn!(
                    hand = %instance.hand_id,
                    instance = %instance.instance_id,
                    role = %role,
                    agent = %agent_id,
                    error = %restore_err,
                    "Failed to restore hand runtime override after live registry update failed"
                );
            } else if let Err(persist_err) = self.persist_hand_state_result() {
                warn!(
                    hand = %instance.hand_id,
                    instance = %instance.instance_id,
                    role = %role,
                    agent = %agent_id,
                    error = %persist_err,
                    "Restored hand runtime override in memory but failed to persist rollback; a restart may replay the unapplied override"
                );
            }
            return Err(err);
        }
        Ok(())
    }

    /// Clear all runtime overrides for a hand agent, restoring the live
    /// manifest to the defaults declared in the owning hand's HAND.toml.
    ///
    /// Returns [`LibreFangError::AgentNotFound`] if the agent id is not
    /// attached to any active hand. Returns an `Internal` error with the
    /// `Hand role not found` prefix if the hand instance exists but no role
    /// maps to the given agent id (should not happen in practice — guarded
    /// so the HTTP layer can surface a 409 instead of a silent 500).
    ///
    /// Unlike [`Self::update_hand_agent_runtime_override`], this is a full
    /// reset: the per-role entry in `agent_runtime_overrides` is dropped and
    /// the agent's `model`, `provider`, `api_key_env`, `base_url`,
    /// `max_tokens`, `temperature`, and `web_search_augmentation` fields
    /// are rewritten from `def.agents[role].manifest`. State is persisted
    /// before the live AgentRegistry rewrite so a partial failure leaves
    /// the persisted file as the source of truth — and the in-memory
    /// override is restored if either persist or AgentRegistry-write
    /// fails. Mirrors the rollback discipline in
    /// [`Self::update_hand_agent_runtime_override`].
    pub fn clear_hand_agent_runtime_override(&self, agent_id: AgentId) -> KernelResult<()> {
        let instance = self
            .skills
            .hand_registry
            .find_by_agent(agent_id)
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
            })?;
        // See the matching block in `update_hand_agent_runtime_override`:
        // serialize per instance so PATCH and DELETE on the same hand
        // can't interleave their AgentRegistry writes.
        let lock = self.hand_instance_lock(instance.instance_id);
        let _guard = lock_hand_instance_recover(&lock, instance.instance_id);
        // Re-read after taking the lock so a concurrent update isn't
        // silently overwritten by a stale snapshot.
        let instance = self
            .skills
            .hand_registry
            .find_by_agent(agent_id)
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
            })?;
        let role = instance
            .agent_ids
            .iter()
            .find_map(|(role, id)| (*id == agent_id).then(|| role.clone()))
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::Internal(format!(
                    "Hand role not found for agent {agent_id}"
                )))
            })?;

        // Snapshot the current override so we can roll back the
        // persisted state if the live AgentRegistry rewrite fails.
        let previous = instance.agent_runtime_overrides.get(&role).cloned();

        // Step 1: clear from the in-memory hand registry (atomic under
        // the DashMap shard lock). If `previous` was already None this
        // returns Ok(None) — idempotent.
        self.skills.hand_registry
            .clear_agent_runtime_override(instance.instance_id, &role)
            // #3711: propagate typed HandError (Display preserved).
            .map_err(KernelError::from)?;

        // Step 2: persist before touching live state. If the disk write
        // fails, restore the in-memory entry and bail — the operator
        // sees the original override on retry.
        if let Err(err) = self.persist_hand_state_result() {
            let _ = self.skills.hand_registry.restore_agent_runtime_override(
                instance.instance_id,
                &role,
                previous,
            );
            return Err(err);
        }

        // Step 3: rewrite the live AgentRegistry to the HAND.toml
        // defaults. Errors here roll back both the in-memory override
        // and the persisted file so the next PATCH/DELETE sees a
        // coherent snapshot.
        let def = self.skills.hand_registry.get_definition(&instance.hand_id);
        if let Some(def) = def {
            if let Some(agent_def) = def.agents.get(&role) {
                // Start from the raw HAND.toml manifest and re-apply the
                // same "default" sentinel resolution that `activate_hand_with_id`
                // runs at activation time. Going through the raw manifest
                // would leave `model = "default"` on disk, which the LLM
                // driver can't route.
                let (model, provider, api_key_env, base_url) =
                    self.resolve_hand_agent_model_defaults(&agent_def.manifest);

                let apply_result = (|| -> KernelResult<()> {
                    self.agents
                        .registry
                        .update_model_provider_config(
                            agent_id,
                            model,
                            provider,
                            api_key_env,
                            base_url,
                        )
                        .map_err(KernelError::LibreFang)?;
                    self.agents
                        .registry
                        .update_max_tokens(agent_id, agent_def.manifest.model.max_tokens)
                        .map_err(KernelError::LibreFang)?;
                    self.agents
                        .registry
                        .update_temperature(agent_id, agent_def.manifest.model.temperature)
                        .map_err(KernelError::LibreFang)?;
                    self.agents
                        .registry
                        .update_web_search_augmentation(
                            agent_id,
                            agent_def.manifest.web_search_augmentation,
                        )
                        .map_err(KernelError::LibreFang)?;
                    Ok(())
                })();

                if let Err(err) = apply_result {
                    if let Err(restore_err) = self
                        .skills
                        .hand_registry
                        .restore_agent_runtime_override(instance.instance_id, &role, previous)
                    {
                        warn!(
                            hand = %instance.hand_id,
                            instance = %instance.instance_id,
                            role = %role,
                            agent = %agent_id,
                            error = %restore_err,
                            "Failed to restore cleared hand runtime override after live registry reset failed"
                        );
                    } else if let Err(persist_err) = self.persist_hand_state_result() {
                        warn!(
                            hand = %instance.hand_id,
                            instance = %instance.instance_id,
                            role = %role,
                            agent = %agent_id,
                            error = %persist_err,
                            "Restored cleared hand runtime override in memory but failed to persist rollback; a restart may replay the unapplied clear"
                        );
                    }
                    return Err(err);
                }
            } else {
                warn!(
                    agent = %agent_id,
                    hand = %instance.hand_id,
                    role = %role,
                    "Hand definition has no entry for role; skipping manifest reset"
                );
            }
        } else {
            warn!(
                agent = %agent_id,
                hand = %instance.hand_id,
                "Hand definition not loaded; skipping manifest reset on clear"
            );
        }

        Ok(())
    }

    /// Pause a hand (marks it paused and suspends background loop ticks).
    pub fn pause_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        // Pause the background loop for all of this hand's agents
        if let Some(instance) = self.skills.hand_registry.get_instance(instance_id) {
            for &agent_id in instance.agent_ids.values() {
                self.workflows.background.pause_agent(agent_id);
            }
        }
        self.skills.hand_registry
            .pause(instance_id)
            // #3711: propagate typed HandError (Display preserved).
            .map_err(KernelError::from)?;
        self.persist_hand_state();
        Ok(())
    }

    /// Resume a paused hand (restores background loop ticks).
    pub fn resume_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        self.skills.hand_registry
            .resume(instance_id)
            // #3711: propagate typed HandError (Display preserved).
            .map_err(KernelError::from)?;
        // Resume the background loop for all of this hand's agents
        if let Some(instance) = self.skills.hand_registry.get_instance(instance_id) {
            for &agent_id in instance.agent_ids.values() {
                self.workflows.background.resume_agent(agent_id);
            }
        }
        self.persist_hand_state();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_hand_instance_lock_recovers_and_remains_exclusive() {
        let lock = Arc::new(std::sync::Mutex::new(()));
        let instance_id = uuid::Uuid::new_v4();
        let poison_lock = Arc::clone(&lock);
        let poison = std::thread::spawn(move || {
            let _guard = poison_lock.lock().unwrap();
            panic!("poison hand instance lock");
        })
        .join();
        assert!(poison.is_err());
        assert!(lock.is_poisoned());

        let recovered = lock_hand_instance_recover(&lock, instance_id);
        assert!(!lock.is_poisoned());
        assert!(matches!(
            lock.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));
        drop(recovered);
        assert!(lock.lock().is_ok());
    }
}
