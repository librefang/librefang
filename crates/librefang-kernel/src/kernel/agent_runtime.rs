//! Agent runtime control surface — extracted from `kernel::mod` in
//! Phase 3e of #4713. Hosts cost reporting, run-cancellation fan-out,
//! suspend/resume, kill (with optional identity purge), session
//! compaction, the context-window report, and the per-agent watcher
//! tracker. These methods form the imperative control surface the
//! API and CLI layers reach for when an operator presses "stop",
//! "kill", or "compact".
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use std::sync::Arc;

use librefang_types::agent::{AgentId, RunningSessionSnapshot, RunningSessionState, SessionId};
use librefang_types::error::LibreFangError;
use tracing::{info, warn};

use crate::error::{KernelError, KernelResult};
use crate::metering::MeteringEngine;

use super::cron_script::atomic_write_toml;
use super::LibreFangKernel;

impl LibreFangKernel {
    /// Get session token usage and estimated cost for an agent.
    pub fn session_usage_cost(&self, agent_id: AgentId) -> KernelResult<(u64, u64, f64)> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::LibreFang)?;

        let (input_tokens, output_tokens) = session
            .map(|s| {
                let mut input = 0u64;
                let mut output = 0u64;
                // Estimate tokens from message content length (rough: 1 token ≈ 4 chars)
                for msg in &s.messages {
                    let len = msg.content.text_content().len() as u64;
                    let tokens = len / 4;
                    match msg.role {
                        librefang_types::message::Role::User => input += tokens,
                        librefang_types::message::Role::Assistant => output += tokens,
                        librefang_types::message::Role::System => input += tokens,
                    }
                }
                (input, output)
            })
            .unwrap_or((0, 0));

        let model = &entry.manifest.model.model;
        let cost = MeteringEngine::estimate_cost_with_catalog(
            &self.model_catalog.load(),
            model,
            input_tokens,
            output_tokens,
            0, // no cache token breakdown available from session history
            0,
        );

        Ok((input_tokens, output_tokens, cost))
    }

    /// Cancel **every** in-flight LLM task for an agent. Fans out across
    /// all `(agent, session)` entries so an agent that owns multiple
    /// concurrent loops (parallel `session_mode = "new"` triggers,
    /// `agent_send` fan-out, parallel channel chats) is fully halted.
    ///
    /// Two signals are sent per session:
    /// 1. `AbortHandle::abort()` — terminates the tokio task at the next
    ///    `.await` point (fast but coarse).
    /// 2. `SessionInterrupt::cancel()` — sets the per-session atomic flag so
    ///    in-flight tool futures that poll `is_cancelled()` can bail out
    ///    gracefully before the task is actually dropped.
    ///
    /// Returns `true` when at least one session was stopped, `false` when
    /// the agent had no active loops. Callers that need session-scoped
    /// stop should use [`Self::stop_session_run`] instead.
    ///
    /// **Snapshot semantics:** session keys are collected into a `Vec` first,
    /// then iterated to remove. A session that finishes between the snapshot
    /// and the removal is silently absent from the count (already gone, so
    /// the removal is a no-op). A session inserted **after** the snapshot is
    /// not aborted by this call — `stop_agent_run` is best-effort against the
    /// instant it observes. Concurrent dispatches that race with stop are
    /// expected to either be aborted or to start cleanly afterward; partial
    /// abort of a half-spawned loop would be more surprising than missing
    /// it. Callers that need a strict "freeze, then abort" should suspend
    /// the agent first via [`Self::suspend_agent`] (which itself fans out
    /// through this method).
    pub fn stop_agent_run(&self, agent_id: AgentId) -> KernelResult<bool> {
        let sessions: Vec<SessionId> = self
            .running_tasks
            .iter()
            .filter(|e| e.key().0 == agent_id)
            .map(|e| e.key().1)
            .collect();
        let interrupt_sessions: Vec<SessionId> = self
            .session_interrupts
            .iter()
            .filter(|e| e.key().0 == agent_id)
            .map(|e| e.key().1)
            .collect();
        // Signal interrupts first so tools see cancellation before the
        // tokio tasks are dropped at the next .await.
        for sid in &interrupt_sessions {
            if let Some((_, interrupt)) = self.session_interrupts.remove(&(agent_id, *sid)) {
                interrupt.cancel();
            }
        }
        let mut stopped = 0usize;
        for sid in &sessions {
            if let Some((_, task)) = self.running_tasks.remove(&(agent_id, *sid)) {
                task.abort.abort();
                stopped += 1;
            }
        }
        if stopped > 0 {
            info!(agent_id = %agent_id, sessions = stopped, "Agent run cancelled (fan-out)");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Cancel a single in-flight `(agent, session)` loop without affecting
    /// the agent's other concurrent sessions. Mirrors [`Self::stop_agent_run`]
    /// signal pair (interrupt first, then abort) but scoped to one entry.
    ///
    /// Returns `true` when the entry existed and was aborted, `false` when
    /// no loop was running for that pair (already finished, never started,
    /// or the session belongs to a different agent).
    pub fn stop_session_run(&self, agent_id: AgentId, session_id: SessionId) -> KernelResult<bool> {
        if let Some((_, interrupt)) = self.session_interrupts.remove(&(agent_id, session_id)) {
            interrupt.cancel();
        }
        if let Some((_, task)) = self.running_tasks.remove(&(agent_id, session_id)) {
            task.abort.abort();
            info!(agent_id = %agent_id, session_id = %session_id, "Session run cancelled");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Snapshot every in-flight `(agent, session)` loop owned by `agent_id`.
    /// Empty `Vec` when the agent has no active loops. Order is unspecified
    /// (DashMap iteration order); callers that need a stable order should
    /// sort by `started_at` themselves.
    pub fn list_running_sessions(&self, agent_id: AgentId) -> Vec<RunningSessionSnapshot> {
        self.running_tasks
            .iter()
            .filter(|e| e.key().0 == agent_id)
            .map(|e| RunningSessionSnapshot {
                session_id: e.key().1,
                started_at: e.value().started_at,
                state: RunningSessionState::Running,
            })
            .collect()
    }

    /// Cheap check used by `librefang-api/src/ws.rs` to gate state-event
    /// fan-out — true when `agent_id` has at least one session in flight.
    pub fn agent_has_active_session(&self, agent_id: AgentId) -> bool {
        self.running_tasks.iter().any(|e| e.key().0 == agent_id)
    }

    /// Snapshot of every `SessionId` whose agent loop is currently in flight,
    /// kernel-wide. Used by `/api/sessions` and per-agent session-listing
    /// endpoints to populate the `active` field with "loop is currently
    /// running" semantics — matching the dashboard's green-dot/pulse
    /// rendering (see #4290, #4293). DashMap iteration is unordered; the
    /// caller treats the result as a set lookup, never as a list. Cheap:
    /// one `(AgentId, SessionId)` clone per running task.
    pub fn running_session_ids(&self) -> std::collections::HashSet<SessionId> {
        self.running_tasks.iter().map(|e| e.key().1).collect()
    }

    /// Suspend an agent — sets state to Suspended, persists enabled=false to TOML.
    pub fn suspend_agent(&self, agent_id: AgentId) -> KernelResult<()> {
        use librefang_types::agent::AgentState;
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;
        let _ = self.registry.set_state(agent_id, AgentState::Suspended);
        // Stop every active session for the agent — same fan-out as
        // `stop_agent_run` so a multi-session agent is fully halted.
        let _ = self.stop_agent_run(agent_id);
        // Persist enabled=false to agent.toml
        self.persist_agent_enabled(agent_id, &entry.name, false);
        info!(agent_id = %agent_id, "Agent suspended");
        Ok(())
    }

    /// Resume a suspended agent — sets state back to Running, persists enabled=true.
    pub fn resume_agent(&self, agent_id: AgentId) -> KernelResult<()> {
        use librefang_types::agent::AgentState;
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;
        let _ = self.registry.set_state(agent_id, AgentState::Running);
        // Persist enabled=true to agent.toml
        self.persist_agent_enabled(agent_id, &entry.name, true);
        info!(agent_id = %agent_id, "Agent resumed");
        Ok(())
    }

    /// Write enabled flag to agent's TOML file.
    fn persist_agent_enabled(&self, _agent_id: AgentId, name: &str, enabled: bool) {
        let cfg = self.config.load();
        // Check both workspaces/agents/ and workspaces/hands/ directories
        let agents_path = cfg
            .effective_agent_workspaces_dir()
            .join(name)
            .join("agent.toml");
        let hands_path = cfg
            .effective_hands_workspaces_dir()
            .join(name)
            .join("agent.toml");
        let toml_path = if agents_path.exists() {
            agents_path
        } else if hands_path.exists() {
            hands_path
        } else {
            return;
        };
        match std::fs::read_to_string(&toml_path) {
            Ok(content) => {
                // Simple: replace or append enabled field
                let new_content = if content.contains("enabled =") || content.contains("enabled=") {
                    content
                        .lines()
                        .map(|line| {
                            if line.trim_start().starts_with("enabled") && line.contains('=') {
                                format!("enabled = {enabled}")
                            } else {
                                line.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    // Append after [agent] section or at end
                    format!("{content}\nenabled = {enabled}\n")
                };
                if let Err(e) = atomic_write_toml(&toml_path, &new_content) {
                    warn!("Failed to persist enabled={enabled} for {name}: {e}");
                }
            }
            Err(e) => warn!("Failed to read agent TOML for {name}: {e}"),
        }
    }

    /// Compact an agent's session using LLM-based summarization.
    ///
    /// Replaces the existing text-truncation compaction with an intelligent
    /// LLM-generated summary of older messages, keeping only recent messages.
    pub async fn compact_agent_session(&self, agent_id: AgentId) -> KernelResult<String> {
        self.compact_agent_session_with_id(agent_id, None).await
    }

    /// Compact a specific session. When `session_id_override` is `Some`,
    /// that session is loaded instead of the one currently attached to
    /// the agent's registry entry — needed by the streaming pre-loop
    /// hook, which operates on an `effective_session_id` derived from
    /// sender context / session_mode that can legitimately differ from
    /// `entry.session_id`. Without this override, the streaming path's
    /// pre-compaction call loaded the wrong (often empty) session and
    /// logged `No compaction needed (0 messages, ...)` while the real
    /// in-turn session had hundreds of messages and was about to
    /// overflow the model's context.
    pub async fn compact_agent_session_with_id(
        &self,
        agent_id: AgentId,
        session_id_override: Option<SessionId>,
    ) -> KernelResult<String> {
        let cfg = self.config.load_full();
        use librefang_runtime::compactor::{compact_session, needs_compaction, CompactionConfig};

        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let target_session_id = session_id_override.unwrap_or(entry.session_id);
        let session = self
            .memory
            .get_session(target_session_id)
            .map_err(KernelError::LibreFang)?
            .unwrap_or_else(|| librefang_memory::session::Session {
                id: target_session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
                messages_generation: 0,
                last_repaired_generation: None,
            });

        let config = CompactionConfig::from_toml(&cfg.compaction);

        if !needs_compaction(&session, &config) {
            return Ok(format!(
                "No compaction needed ({} messages, threshold {})",
                session.messages.len(),
                config.threshold
            ));
        }

        // Strip provider prefix so the model name is valid for the upstream API.
        let model = librefang_runtime::agent_loop::strip_provider_prefix(
            &entry.manifest.model.model,
            &entry.manifest.model.provider,
        );

        // Resolve the agent's actual context window from the model catalog.
        // Filter out 0 so image/audio entries (no context window) fall back
        // to the 200K default instead of feeding 0 into compaction math.
        let agent_ctx_window = self
            .model_catalog
            .load()
            .find_model(&entry.manifest.model.model)
            .map(|m| m.context_window as usize)
            .filter(|w| *w > 0)
            .unwrap_or(200_000);

        // Compaction is a side task — route through the auxiliary chain when
        // configured (issue #3314) so users with `[llm.auxiliary] compression`
        // pay cheap-tier rates rather than the agent's primary model. When no
        // aux entry can be initialised, the resolver returns a driver
        // equivalent to `resolve_driver(&entry.manifest)` (the kernel's
        // default driver chain), so behaviour matches the pre-issue-#3314
        // baseline.
        let driver = self
            .aux_client
            .load()
            .driver_for(librefang_types::config::AuxTask::Compression);

        // Delegate to the context engine when available (and allowed for this agent),
        // otherwise fall back to the built-in compactor directly.
        let result = if let Some(engine) = self.context_engine_for_agent(&entry.manifest) {
            engine
                .compact(
                    agent_id,
                    &session.messages,
                    Arc::clone(&driver),
                    &model,
                    agent_ctx_window,
                )
                .await
                .map_err(KernelError::LibreFang)?
        } else {
            compact_session(driver, &model, &session, &config)
                .await
                .map_err(|e| KernelError::LibreFang(LibreFangError::Internal(e)))?
        };

        // Store the LLM summary in the canonical session
        self.memory
            .store_llm_summary(agent_id, &result.summary, result.kept_messages.clone())
            .map_err(KernelError::LibreFang)?;

        // Post-compaction audit: validate and repair the kept messages
        let (repaired_messages, repair_stats) =
            librefang_runtime::session_repair::validate_and_repair_with_stats(
                &result.kept_messages,
            );

        // Also update the regular session with the repaired messages
        let mut updated_session = session;
        updated_session.set_messages(repaired_messages);
        self.memory
            .save_session_async(&updated_session)
            .await
            .map_err(KernelError::LibreFang)?;

        // Build result message with audit summary
        let mut msg = format!(
            "Compacted {} messages into summary ({} chars), kept {} recent messages.",
            result.compacted_count,
            result.summary.len(),
            updated_session.messages.len()
        );

        let repairs = repair_stats.orphaned_results_removed
            + repair_stats.synthetic_results_inserted
            + repair_stats.duplicates_removed
            + repair_stats.messages_merged;
        if repairs > 0 {
            msg.push_str(&format!(" Post-audit: repaired ({} orphaned removed, {} synthetic inserted, {} merged, {} deduped).",
                repair_stats.orphaned_results_removed,
                repair_stats.synthetic_results_inserted,
                repair_stats.messages_merged,
                repair_stats.duplicates_removed,
            ));
        } else {
            msg.push_str(" Post-audit: clean.");
        }

        Ok(msg)
    }

    /// Generate a context window usage report for an agent.
    pub fn context_report(
        &self,
        agent_id: AgentId,
    ) -> KernelResult<librefang_runtime::compactor::ContextReport> {
        use librefang_runtime::compactor::generate_context_report;

        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::LibreFang)?
            .unwrap_or_else(|| librefang_memory::session::Session {
                id: entry.session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
                messages_generation: 0,
                last_repaired_generation: None,
            });
        let system_prompt = &entry.manifest.model.system_prompt;
        // Use the agent's actual filtered tools instead of all builtins
        let tools = self.available_tools(agent_id);
        // Use 200K default or the model's known context window
        let context_window = if session.context_window_tokens > 0 {
            session.context_window_tokens
        } else {
            200_000
        };

        Ok(generate_context_report(
            &session.messages,
            Some(system_prompt),
            Some(&tools),
            context_window as usize,
        ))
    }

    /// Track a per-agent fire-and-forget background task so `kill_agent`
    /// can abort it and free its semaphore permit. Drops finished entries
    /// opportunistically to keep the vec bounded (#3705).
    pub(crate) fn register_agent_watcher(
        &self,
        agent_id: AgentId,
        handle: tokio::task::JoinHandle<()>,
    ) {
        let slot = self
            .agent_watchers
            .entry(agent_id)
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(Vec::new())))
            .clone();
        // The trailing `;` matters: without it the if-let is the function's
        // tail expression, which keeps the LockResult's temporaries borrowing
        // `slot` until function exit — and `slot` itself drops at the same
        // point, tripping E0597. The semicolon ends the statement so the
        // temporaries (and the guard) drop before `slot` does.
        if let Ok(mut guard) = slot.lock() {
            guard.retain(|h| !h.is_finished());
            guard.push(handle);
        };
    }

    /// Abort and drop every tracked watcher task for `agent_id`.
    fn abort_agent_watchers(&self, agent_id: AgentId) {
        if let Some((_, slot)) = self.agent_watchers.remove(&agent_id) {
            if let Ok(mut guard) = slot.lock() {
                for h in guard.drain(..) {
                    h.abort();
                }
            }
        }
    }

    /// Kill an agent. By default the canonical UUID registry entry
    /// (refs #4614) is **kept** so a later respawn of the same name lands
    /// on the same `AgentId`. Use [`Self::kill_agent_with_purge`] to also
    /// drop the canonical-UUID binding (i.e. fully orphan history).
    pub fn kill_agent(&self, agent_id: AgentId) -> KernelResult<()> {
        self.kill_agent_with_purge(agent_id, false)
    }

    /// Kill an agent and optionally purge its canonical UUID binding from
    /// the identity registry (refs #4614).
    ///
    /// `purge_identity = false` (the default for `kill_agent`) is the
    /// safe choice — sessions and memories tied to this UUID stay
    /// reachable on respawn.
    ///
    /// `purge_identity = true` permanently removes the `name → uuid`
    /// mapping. The next spawn under the same name will derive a fresh
    /// UUID via `AgentId::from_name`, and any prior history is orphaned.
    /// This is the destructive path the issue describes ("explicit
    /// delete + confirmation"); confirmation is enforced at the API/CLI
    /// layer.
    pub fn kill_agent_with_purge(
        &self,
        agent_id: AgentId,
        purge_identity: bool,
    ) -> KernelResult<()> {
        let entry = self
            .registry
            .remove(agent_id)
            .map_err(KernelError::LibreFang)?;
        self.background.stop_agent(agent_id);
        // Abort any per-agent fire-and-forget tasks (skill reviews, …) so
        // they release semaphore permits and stop spending tokens on
        // behalf of a now-deleted agent (#3705).
        self.abort_agent_watchers(agent_id);
        self.scheduler.unregister(agent_id);
        self.capabilities.revoke_all(agent_id);
        self.event_bus.unsubscribe_agent(agent_id);
        self.triggers.remove_agent_triggers(agent_id);
        if let Err(e) = self.triggers.persist() {
            warn!("Failed to persist trigger jobs after agent deletion: {e}");
        }

        // Remove cron jobs so they don't linger as orphans (#504)
        let cron_removed = self.cron_scheduler.remove_agent_jobs(agent_id);
        if cron_removed > 0 {
            if let Err(e) = self.cron_scheduler.persist() {
                warn!("Failed to persist cron jobs after agent deletion: {e}");
            }
        }

        // Remove from persistent storage
        let _ = self.memory.remove_agent(agent_id);

        // Clean up proactive memories for this agent
        if let Some(pm) = self.proactive_memory.get() {
            let aid = agent_id.0.to_string();
            if let Err(e) = pm.reset(&aid) {
                warn!("Failed to clean up proactive memories for agent {agent_id}: {e}");
            }
        }

        // Refs #4614: canonical UUID registry. Default `kill_agent` keeps
        // the binding so a respawn under the same name reuses this UUID.
        // `kill_agent_with_purge(agent, true)` (gated behind explicit
        // confirmation at the API/CLI surface) also drops the entry,
        // which is the destructive path the issue describes.
        if purge_identity {
            if let Some(dropped) = self.agent_identities.purge(&entry.name) {
                info!(
                    agent = %entry.name,
                    id = %dropped,
                    "Purged canonical UUID from agent_identities registry (#4614)"
                );
            }
        }

        // SECURITY: Record agent kill in audit trail
        self.audit_log.record(
            agent_id.to_string(),
            librefang_runtime::audit::AuditAction::AgentKill,
            format!("name={}, purge_identity={}", entry.name, purge_identity),
            "ok",
        );

        // Lifecycle: agent has been removed from the registry; sessions tied
        // to this agent are no longer active. Use the agent name as the
        // best-effort reason — call sites that need richer context can extend
        // the variant in a future change.
        self.session_lifecycle_bus.publish(
            crate::session_lifecycle::SessionLifecycleEvent::AgentTerminated {
                agent_id,
                reason: format!("kill_agent(name={})", entry.name),
            },
        );

        info!(agent = %entry.name, id = %agent_id, "Agent killed");
        Ok(())
    }

    // Hand lifecycle (`activate_hand`, `deactivate_hand`, `pause_hand`,
    // `resume_hand`, `update_hand_agent_runtime_override`, …) lives in
    // `kernel::hands_lifecycle` since #4713 phase 3c.
}
