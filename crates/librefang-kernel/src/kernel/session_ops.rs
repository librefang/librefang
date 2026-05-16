//! Cluster pulled out of mod.rs in #4713 phase 3d.
//!
//! Hosts the per-session lifecycle surface: `inject_message` /
//! `inject_message_for_session` (mid-turn message injection, #956),
//! injection-channel setup/teardown helpers, agent-relative module path
//! resolution, session reset / reboot / clear-history flows, multi-session
//! enumeration and switching (`list_agent_sessions`, `create_agent_session`,
//! `switch_agent_session`), `export_session` / `import_session`, and the
//! private helpers used by reset paths (`inject_reset_prompt`,
//! `evaluate_condition`, `save_session_summary`).
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Inject a message into a running agent's tool-execution loop (#956).
    ///
    /// If the agent is currently executing tools (mid-turn), the message will be
    /// picked up between tool calls and interrupt the remaining sequence.
    /// Returns `Ok(true)` if the message was sent, `Ok(false)` if no active
    /// loop is running for this agent, or `Err` if the agent doesn't exist.
    pub async fn inject_message(&self, agent_id: AgentId, message: &str) -> KernelResult<bool> {
        self.inject_message_for_session(agent_id, None, message)
            .await
    }

    /// Session-aware variant of [`Self::inject_message`]; `None` fans out to all live sessions.
    ///
    /// Returns:
    /// - `Ok(true)`  — at least one live session accepted the message.
    /// - `Ok(false)` — no live loop is running for this agent (every target
    ///   was closed, or there were zero targets).
    /// - `Err(KernelError::Backpressure)` — every live target's bounded
    ///   channel was full; the caller should retry. The API layer maps this
    ///   to HTTP 503 (#3575).
    pub async fn inject_message_for_session(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
        message: &str,
    ) -> KernelResult<bool> {
        // Verify the agent exists
        if self.agents.registry.get(agent_id).is_none() {
            return Err(KernelError::LibreFang(LibreFangError::AgentNotFound(
                agent_id.to_string(),
            )));
        }

        // Collect targets first so we don't hold any DashMap shard lock
        // across the `try_send` calls (which themselves can briefly block on
        // the per-channel internal lock).
        let targets: Vec<(
            (AgentId, SessionId),
            tokio::sync::mpsc::Sender<AgentLoopSignal>,
        )> = if let Some(sid) = session_id {
            self.events
                .injection_senders
                .get(&(agent_id, sid))
                .map(|entry| (*entry.key(), entry.value().clone()))
                .into_iter()
                .collect()
        } else {
            self.events
                .injection_senders
                .iter()
                .filter(|e| e.key().0 == agent_id)
                .map(|e| (*e.key(), e.value().clone()))
                .collect()
        };

        if targets.is_empty() {
            return Ok(false);
        }

        let mut delivered = false;
        let mut full_keys: Vec<(AgentId, SessionId)> = Vec::new();
        let mut closed_keys: Vec<(AgentId, SessionId)> = Vec::new();
        for (key, tx) in targets {
            match tx.try_send(AgentLoopSignal::Message {
                content: message.to_string(),
            }) {
                Ok(()) => {
                    info!(
                        agent_id = %agent_id,
                        session_id = %key.1,
                        "Mid-turn message injected"
                    );
                    delivered = true;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    warn!(
                        agent_id = %agent_id,
                        session_id = %key.1,
                        "Injection channel full — applying backpressure"
                    );
                    full_keys.push(key);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    // Receiver dropped — loop is no longer running.
                    closed_keys.push(key);
                }
            }
        }
        for key in &closed_keys {
            self.events.injection_senders.remove(key);
        }
        // If at least one live session accepted the message, the inject is a
        // success from the caller's POV. If every live (non-closed) target
        // was full, surface backpressure so the API can return 503 instead
        // of pretending the message was queued.
        if !delivered && !full_keys.is_empty() {
            return Err(KernelError::Backpressure(format!(
                "all {} injection channel(s) for agent {} are full; retry shortly",
                full_keys.len(),
                agent_id
            )));
        }
        // No live loop at all (every target was closed, or zero targets after
        // we filtered) — preserve the historical Ok(false) signal.
        Ok(delivered)
    }

    /// Creates the injection channel for `(agent_id, session_id)` and returns the receiver.
    pub(crate) fn setup_injection_channel(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<AgentLoopSignal>>> {
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentLoopSignal>(8);
        self.events
            .injection_senders
            .insert((agent_id, session_id), tx);
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        self.events
            .injection_receivers
            .insert((agent_id, session_id), Arc::clone(&rx));
        rx
    }

    /// Tears down the `(agent_id, session_id)` injection channel after the loop finishes.
    pub(crate) fn teardown_injection_channel(&self, agent_id: AgentId, session_id: SessionId) {
        self.events
            .injection_senders
            .remove(&(agent_id, session_id));
        self.events
            .injection_receivers
            .remove(&(agent_id, session_id));
    }

    /// Resolve a module path relative to the kernel's home directory.
    ///
    /// If the path is absolute, return it as-is. Otherwise, resolve relative
    /// to `config.home_dir`.
    pub(crate) fn resolve_module_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.home_dir_boot.join(path)
        }
    }

    /// Reset an agent's session(s) — auto-saves a summary to memory, then
    /// clears messages and prepares a fresh session.
    ///
    /// `scope` chooses between agent-wide and per-session semantics (#4868):
    ///
    /// - [`ResetScope::Agent`] — historical behaviour. Saves a summary for
    ///   every session (default + per-channel + cron-spawned), deletes all
    ///   rows, creates one fresh registry-pointer session, resets quota.
    ///   Used by the dashboard / explicit `POST /api/agents/{id}/session/reset`.
    /// - [`ResetScope::Session(sid)`] — scoped delete. Saves the summary for
    ///   that one session, deletes only that row + its FTS index + its JSONL
    ///   mirror, eagerly recreates an empty session at the same deterministic
    ///   sid (so the channel resolver lands on it on the next inbound
    ///   message), and leaves all other sessions untouched. Quota is **not**
    ///   reset (per-channel resets must not give one user a way to clear an
    ///   agent-wide token-quota state). Used by channel `/new`.
    pub async fn reset_session(&self, agent_id: AgentId, scope: ResetScope) -> KernelResult<()> {
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        match scope {
            ResetScope::Session(sid) => self.reset_one_session(agent_id, sid, &entry, true).await,
            ResetScope::Agent => self.reset_all_sessions(agent_id, &entry, true).await,
        }
    }

    /// Hard-reboot an agent's session(s) — clears conversation history WITHOUT
    /// saving a summary to memory. Keeps agent config, system prompt, and
    /// tools intact. More aggressive than `reset_session` (which auto-saves a
    /// summary) but less destructive than `clear_agent_history` (which wipes
    /// the canonical session as well).
    ///
    /// `scope` follows the same agent-wide vs. per-session split as
    /// [`Self::reset_session`] (#4868).
    pub async fn reboot_session(&self, agent_id: AgentId, scope: ResetScope) -> KernelResult<()> {
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        match scope {
            ResetScope::Session(sid) => self.reset_one_session(agent_id, sid, &entry, false).await,
            ResetScope::Agent => self.reset_all_sessions(agent_id, &entry, false).await,
        }
    }

    /// Implementation of [`ResetScope::Agent`] — wipe every session for this
    /// agent. `save_summary` distinguishes reset (true) vs. reboot (false).
    ///
    /// Lock discipline (#4868 review): `send_message_full` and the streaming
    /// path acquire EITHER `agent_msg_locks[agent_id]` OR
    /// `session_msg_locks[sid]` depending on whether the caller supplied a
    /// `session_id_override` (multi-tab WS / scoped HTTP — #2959). Holding
    /// just the agent lock would leave a revive-after-delete window for
    /// any concurrent override-using turn: it could finish its work and
    /// UPSERT a deleted session row back into existence. So we acquire the
    /// agent lock, snapshot the live sids under it, then take every
    /// per-session lock (sorted to keep a deterministic order across
    /// callers). All guards are held through the delete + recreate.
    async fn reset_all_sessions(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        save_summary: bool,
    ) -> KernelResult<()> {
        let agent_lock = self
            .agents
            .agent_msg_locks
            .entry(agent_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _agent_guard = agent_lock.lock_owned().await;

        // Auto-save session summaries for ALL sessions (default + per-channel)
        // before clearing, so no channel's conversation history is silently lost.
        // Also emit session:end for each active session before deletion.
        let mut pre_delete_sids = self
            .memory
            .substrate
            .get_agent_session_ids(agent_id)
            .unwrap_or_default();
        // Sort so two concurrent agent-wide resets on different agents that
        // somehow shared a sid (impossible today — sids hash in agent_id —
        // but cheap insurance) can never form a deadlock cycle on the
        // session locks.
        pre_delete_sids.sort_by_key(|s| s.0);
        let mut _session_guards: Vec<tokio::sync::OwnedMutexGuard<()>> =
            Vec::with_capacity(pre_delete_sids.len());
        for sid in &pre_delete_sids {
            let lock = self
                .agents
                .session_msg_locks
                .entry(*sid)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone();
            _session_guards.push(lock.lock_owned().await);
        }
        for sid in &pre_delete_sids {
            if let Ok(Some(old_session)) = self.memory.substrate.get_session(*sid) {
                // Fire session:end before removing the old session.
                self.governance.external_hooks.fire(
                    crate::hooks::ExternalHookEvent::SessionEnd,
                    serde_json::json!({
                        "agent_id": agent_id.to_string(),
                        "session_id": old_session.id.0.to_string(),
                    }),
                );
                if save_summary && old_session.messages.len() >= 2 {
                    self.save_session_summary(agent_id, entry, &old_session);
                }
            }
        }

        // Delete ALL sessions for this agent (default + per-channel).
        // Propagate the error so callers see a half-failed reset instead
        // of silently leaving orphan rows in `sessions` / `sessions_fts`
        // (#3470). The deletion itself is transactional inside
        // `delete_agent_sessions`.
        self.memory
            .substrate
            .delete_agent_sessions(agent_id)
            .map_err(KernelError::LibreFang)?;

        // JSONL mirrors live outside the SQLite transaction (#4868 follow-up).
        // Best-effort cleanup so deleted sessions don't accumulate as orphan
        // transcripts on disk. SQLite is the source of truth for the API
        // surface; a file-system failure here is logged but not fatal.
        Self::purge_jsonl_files(entry, &pre_delete_sids);

        // Create a fresh session and inject reset prompt if configured
        let mut new_session = self
            .memory
            .substrate
            .create_session(agent_id)
            .map_err(KernelError::LibreFang)?;
        self.inject_reset_prompt(&mut new_session, agent_id);

        // Update registry with new session ID
        self.agents
            .registry
            .update_session_id(agent_id, new_session.id)
            .map_err(KernelError::LibreFang)?;

        // Reset quota tracking so /new clears "token quota exceeded"
        self.agents.scheduler.reset_usage(agent_id);

        // Fire external session:reset hook (fire-and-forget).
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionReset,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        // Fire session:start for the newly created session.
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionStart,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        info!(
            agent_id = %agent_id,
            save_summary,
            op = if save_summary { "reset" } else { "reboot" },
            "Agent-wide session wipe complete"
        );
        Ok(())
    }

    /// Implementation of [`ResetScope::Session`] — wipe exactly one session
    /// (sibling sessions untouched). `save_summary` distinguishes reset
    /// (true) vs. reboot (false).
    ///
    /// The recreated session reuses the same deterministic sid because
    /// channel session ids are derived via [`SessionId::for_sender_scope`]
    /// — the next inbound message on that channel will resolve back to the
    /// same id and we want it to land on a fresh empty session, not a 404.
    ///
    /// Lock discipline (#4868 review, lock-race fix): `send_message_full`
    /// acquires EITHER `agent_msg_locks[agent_id]` OR `session_msg_locks[sid]`
    /// — never both, branching on `session_id_override`. Without acquiring
    /// both here, an in-flight turn on either path could finish after the
    /// delete and UPSERT the row back via `save_session`. We take agent
    /// then session; no deadlock cycle is possible because
    /// `send_message_full` only ever holds one of the two, so the second
    /// lock here is never blocked by a caller already holding the first.
    ///
    /// JSONL asymmetry: the recreated empty session is persisted to SQL
    /// but its `<workspace>/sessions/{sid}.jsonl` mirror is intentionally
    /// NOT created at reset time — the next inbound turn's
    /// `write_jsonl_mirror` does it under truncate-create semantics. So
    /// in the brief gap between reset and the next turn, the directory
    /// shows the row in SQL with no file; this is the same shape lazy
    /// session creation produces and is harmless.
    async fn reset_one_session(
        &self,
        agent_id: AgentId,
        sid: SessionId,
        entry: &AgentEntry,
        save_summary: bool,
    ) -> KernelResult<()> {
        let agent_lock = self
            .agents
            .agent_msg_locks
            .entry(agent_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _agent_guard = agent_lock.lock().await;
        let session_lock = self
            .agents
            .session_msg_locks
            .entry(sid)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _session_guard = session_lock.lock().await;

        // Validate ownership: prevent cross-agent reset (callers compute the
        // sid from a (channel, chat) pair which is trusted but the typed
        // `SessionId` argument itself isn't — better to fail loudly than to
        // delete another agent's session because of a wiring bug.
        let old_session = self
            .memory
            .substrate
            .get_session(sid)
            .map_err(KernelError::LibreFang)?;
        if let Some(ref s) = old_session {
            if s.agent_id != agent_id {
                return Err(KernelError::LibreFang(LibreFangError::InvalidInput(
                    format!("session {sid} does not belong to agent {agent_id}"),
                )));
            }
        }

        // Fire SessionEnd + save summary only when the session actually
        // existed (no point summarising a never-touched per-channel sid).
        if let Some(ref s) = old_session {
            self.governance.external_hooks.fire(
                crate::hooks::ExternalHookEvent::SessionEnd,
                serde_json::json!({
                    "agent_id": agent_id.to_string(),
                    "session_id": s.id.0.to_string(),
                }),
            );
            if save_summary && s.messages.len() >= 2 {
                self.save_session_summary(agent_id, entry, s);
            }

            // Delete the SQL row + FTS index transactionally. Skip when the
            // session never existed — `delete_session` would just no-op.
            self.memory
                .substrate
                .delete_session(sid)
                .map_err(KernelError::LibreFang)?;

            // Best-effort JSONL cleanup (see `reset_all_sessions` for rationale).
            Self::purge_jsonl_files(entry, std::slice::from_ref(&sid));
        }

        // Eagerly recreate an empty session at the SAME deterministic sid.
        // Without this, the next channel inbound would lazily materialise an
        // empty session inside the agent loop — but external SessionStart /
        // SessionReset hooks would never fire for that lazy creation, so
        // hook subscribers would see an asymmetric "End without Start".
        let mut new_session = librefang_memory::session::Session {
            id: sid,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        self.inject_reset_prompt(&mut new_session, agent_id);
        // `inject_reset_prompt` only persists when it actually pushed messages
        // — explicitly save the empty case so the row exists for the next
        // inbound message instead of round-tripping back through lazy
        // creation.
        if new_session.messages.is_empty() {
            self.memory
                .substrate
                .save_session(&new_session)
                .map_err(KernelError::LibreFang)?;
        }

        // Update the registry pointer only when it was pointing at the
        // session we just reset. For channel-derived sids (the common case)
        // the pointer is a different sid and stays untouched — that's the
        // whole point of per-session reset.
        if entry.session_id == sid {
            self.agents
                .registry
                .update_session_id(agent_id, sid)
                .map_err(KernelError::LibreFang)?;
        }

        // Quota is intentionally NOT reset here. Token quota is agent-wide;
        // letting any one channel reset it would give a per-channel user a
        // way to clear an agent-wide quota-exceeded state by typing /new.
        // The dashboard "Reset agent" path (ResetScope::Agent) keeps the
        // quota-clearing semantic.

        // Fire session:reset + session:start so external hooks see the
        // standard "fresh session" lifecycle pair, mirroring the agent-wide
        // path.
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionReset,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": sid.0.to_string(),
            }),
        );
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionStart,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": sid.0.to_string(),
            }),
        );

        info!(
            agent_id = %agent_id,
            session_id = %sid,
            existed = old_session.is_some(),
            save_summary,
            op = if save_summary { "reset" } else { "reboot" },
            "Per-session wipe complete (sibling sessions untouched)"
        );
        Ok(())
    }

    /// Best-effort removal of `<workspace>/sessions/{sid}.jsonl` mirrors after
    /// a session row is deleted from SQLite. Without this, `/new`, `/reboot`,
    /// and `clear_agent_history` accumulate orphan transcripts indefinitely
    /// (#4868 follow-up).
    ///
    /// Failures are logged but not propagated: the SQL row is gone, so the
    /// API surface and FTS search no longer expose the deleted session. A
    /// leftover JSONL is a hygiene issue, not a privacy regression — the
    /// data was already on disk and removing the index is the user-visible
    /// "delete" guarantee.
    fn purge_jsonl_files(entry: &AgentEntry, sids: &[SessionId]) {
        let Some(ref workspace) = entry.manifest.workspace else {
            return;
        };
        let sessions_dir = workspace.join("sessions");
        for sid in sids {
            let path = sessions_dir.join(format!("{}.jsonl", sid.0));
            if !path.exists() {
                continue;
            }
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(
                    session_id = %sid,
                    path = %path.display(),
                    error = %e,
                    "Failed to remove orphan session JSONL after delete; \
                     manual cleanup required (DB row already gone)"
                );
            }
        }
    }

    /// Clear ALL conversation history for an agent (sessions + canonical).
    ///
    /// Creates a fresh empty session afterward so the agent is still usable.
    ///
    /// Acquires `agents.agent_msg_locks[agent_id]` so an in-flight inbound
    /// turn cannot save its appended history back over the cleared session
    /// rows (#4868 review).
    pub async fn clear_agent_history(&self, agent_id: AgentId) -> KernelResult<()> {
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let lock = self
            .agents
            .agent_msg_locks
            .entry(agent_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        // Emit session:end for each active session before deletion. Capture
        // the sids list once so we can both fire hooks here and purge JSONL
        // mirrors after the SQL delete (#4868 follow-up).
        let pre_delete_sids = self
            .memory
            .substrate
            .get_agent_session_ids(agent_id)
            .unwrap_or_default();
        for sid in &pre_delete_sids {
            self.governance.external_hooks.fire(
                crate::hooks::ExternalHookEvent::SessionEnd,
                serde_json::json!({
                    "agent_id": agent_id.to_string(),
                    "session_id": sid.0.to_string(),
                }),
            );
        }

        // Delete all regular sessions then the canonical (cross-channel)
        // session. Propagate either failure: a half-cleared agent leaves
        // orphan rows in `sessions` / `sessions_fts` / `canonical_sessions`
        // and is the silent-data-loss vector behind #3470.
        self.memory
            .substrate
            .delete_agent_sessions(agent_id)
            .map_err(KernelError::LibreFang)?;
        self.memory
            .substrate
            .delete_canonical_session(agent_id)
            .map_err(KernelError::LibreFang)?;

        // Best-effort JSONL cleanup. Without this, every history-clear leaks
        // an orphan transcript file the API surface no longer indexes
        // (#4868 follow-up).
        Self::purge_jsonl_files(&entry, &pre_delete_sids);

        // Create a fresh session and inject reset prompt if configured
        let mut new_session = self
            .memory
            .substrate
            .create_session(agent_id)
            .map_err(KernelError::LibreFang)?;
        self.inject_reset_prompt(&mut new_session, agent_id);

        // Update registry with new session ID
        self.agents
            .registry
            .update_session_id(agent_id, new_session.id)
            .map_err(KernelError::LibreFang)?;

        // Reset quota tracking
        self.agents.scheduler.reset_usage(agent_id);

        // Fire external session:reset hook (fire-and-forget).
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionReset,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        // Fire session:start for the newly created session.
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionStart,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        info!(agent_id = %agent_id, "All agent history cleared");
        Ok(())
    }

    /// List all sessions for a specific agent.
    pub fn list_agent_sessions(&self, agent_id: AgentId) -> KernelResult<Vec<serde_json::Value>> {
        // Verify agent exists
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let mut sessions = self
            .memory
            .substrate
            .list_agent_sessions(agent_id)
            .map_err(KernelError::LibreFang)?;

        // `active` means "an agent loop is currently running against this
        // session" — matching `/api/sessions` (#4290) and the dashboard's
        // green-dot/pulse rendering. The legacy "is registry pointer"
        // meaning is preserved as `is_canonical`, which forks /
        // `agent_send` defaults still rely on. See #4293.
        let running = self.running_session_ids();
        let canonical_sid = entry.session_id.0.to_string();
        for s in &mut sessions {
            if let Some(obj) = s.as_object_mut() {
                let sid_str = obj.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
                let is_active = uuid::Uuid::parse_str(sid_str)
                    .map(|u| running.contains(&SessionId(u)))
                    .unwrap_or(false);
                let is_canonical = sid_str == canonical_sid;
                obj.insert("active".to_string(), serde_json::json!(is_active));
                obj.insert("is_canonical".to_string(), serde_json::json!(is_canonical));
            }
        }

        Ok(sessions)
    }

    /// Create a new named session for an agent.
    pub fn create_agent_session(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> KernelResult<serde_json::Value> {
        // Verify agent exists
        let _entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let mut session = self
            .memory
            .substrate
            .create_session_with_label(agent_id, label)
            .map_err(KernelError::LibreFang)?;
        self.inject_reset_prompt(&mut session, agent_id);

        // Switch to the new session
        self.agents
            .registry
            .update_session_id(agent_id, session.id)
            .map_err(KernelError::LibreFang)?;

        // Fire external session:start hook for the newly created session.
        self.governance.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionStart,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": session.id.0.to_string(),
            }),
        );

        info!(agent_id = %agent_id, label = ?label, "Created new session");

        Ok(serde_json::json!({
            "session_id": session.id.0.to_string(),
            "label": session.label,
        }))
    }

    /// Switch an agent to an existing session by session ID.
    pub fn switch_agent_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<()> {
        // Verify agent exists
        let _entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // Verify session exists and belongs to this agent
        let session = self
            .memory
            .substrate
            .get_session(session_id)
            .map_err(KernelError::LibreFang)?
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::Internal("Session not found".to_string()))
            })?;

        if session.agent_id != agent_id {
            return Err(KernelError::LibreFang(LibreFangError::Internal(
                "Session belongs to a different agent".to_string(),
            )));
        }

        self.agents
            .registry
            .update_session_id(agent_id, session_id)
            .map_err(KernelError::LibreFang)?;

        info!(agent_id = %agent_id, session_id = %session_id.0, "Switched session");
        Ok(())
    }

    /// Export a session to a portable JSON-serializable struct for hibernation.
    pub fn export_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<librefang_memory::session::SessionExport> {
        let entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .substrate
            .get_session(session_id)
            .map_err(KernelError::LibreFang)?
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::Internal("Session not found".to_string()))
            })?;

        if session.agent_id != agent_id {
            return Err(KernelError::LibreFang(LibreFangError::Internal(
                "Session belongs to a different agent".to_string(),
            )));
        }

        let export = librefang_memory::session::SessionExport {
            version: 1,
            agent_name: entry.name.clone(),
            agent_id: agent_id.0.to_string(),
            session_id: session_id.0.to_string(),
            messages: session.messages.clone(),
            context_window_tokens: session.context_window_tokens,
            label: session.label.clone(),
            exported_at: chrono::Utc::now().to_rfc3339(),
            metadata: std::collections::HashMap::new(),
        };

        info!(agent_id = %agent_id, session_id = %session_id.0, "Exported session");
        Ok(export)
    }

    /// Import a previously exported session, creating a new session under the given agent.
    pub fn import_session(
        &self,
        agent_id: AgentId,
        export: librefang_memory::session::SessionExport,
    ) -> KernelResult<SessionId> {
        // Verify agent exists
        let _entry = self.agents.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // Validate version
        if export.version != 1 {
            return Err(KernelError::LibreFang(LibreFangError::Internal(format!(
                "Unsupported session export version: {}",
                export.version
            ))));
        }

        // Validate agent_id matches (prevent importing another agent's session)
        if !export.agent_id.is_empty() && export.agent_id != agent_id.to_string() {
            return Err(KernelError::LibreFang(LibreFangError::Internal(format!(
                "Session was exported from agent '{}', cannot import into '{}'",
                export.agent_id, agent_id
            ))));
        }

        // Validate messages are not empty
        if export.messages.is_empty() {
            return Err(KernelError::LibreFang(LibreFangError::Internal(
                "Cannot import session with no messages".to_string(),
            )));
        }

        // Create a new session with imported data
        let new_session = librefang_memory::session::Session {
            id: SessionId::new(),
            agent_id,
            messages: export.messages,
            context_window_tokens: export.context_window_tokens,
            label: export.label,
            model_override: None,

            messages_generation: 0,
            last_repaired_generation: None,
        };
        // Sync save_session: caller `import_session` is a sync fn, no `.await` allowed.
        self.memory
            .substrate
            .save_session(&new_session)
            .map_err(KernelError::LibreFang)?;

        info!(
            new_session_id = %new_session.id.0,
            imported_messages = new_session.messages.len(),
            "Imported session from export"
        );
        Ok(new_session.id)
    }

    /// Inject the configured `session.reset_prompt` and any `context_injection`
    /// entries into a newly created session. Also runs `on_session_start_script`
    /// if configured.
    ///
    /// Injection order:
    /// 1. `InjectionPosition::System` entries (global then agent-level)
    /// 2. `reset_prompt` (if set)
    /// 3. `InjectionPosition::AfterReset` entries (global then agent-level)
    /// 4. `InjectionPosition::BeforeUser` entries are stored but only matter
    ///    relative to future user messages — appended at the end for now.
    pub(crate) fn inject_reset_prompt(
        &self,
        session: &mut librefang_memory::session::Session,
        agent_id: AgentId,
    ) {
        let cfg = self.config.load();
        use librefang_types::config::InjectionPosition;
        use librefang_types::message::Message;

        // Collect agent-level injections (if the agent is registered).
        let agent_injections: Vec<librefang_types::config::ContextInjection> = self
            .agents
            .registry
            .get(agent_id)
            .map(|entry| entry.manifest.context_injection.clone())
            .unwrap_or_default();

        // Collect agent tags for condition evaluation.
        let agent_tags: Vec<String> = self
            .agents
            .registry
            .get(agent_id)
            .map(|entry| entry.manifest.tags.clone())
            .unwrap_or_default();

        // Merge global + agent injections (global first).
        let all_injections: Vec<&librefang_types::config::ContextInjection> = cfg
            .session
            .context_injection
            .iter()
            .chain(agent_injections.iter())
            .collect();

        // Helper: check if a condition is satisfied.
        let condition_met =
            |cond: &Option<String>| -> bool { Self::evaluate_condition(cond, &agent_tags) };

        // Phase 1: System-position injections.
        for inj in &all_injections {
            if inj.position == InjectionPosition::System && condition_met(&inj.condition) {
                session.push_message(Message::system(inj.content.clone()));
                debug!(
                    session_id = %session.id.0,
                    injection = %inj.name,
                    "Injected context (system position)"
                );
            }
        }

        // Phase 2: Legacy reset_prompt.
        if let Some(ref prompt) = cfg.session.reset_prompt {
            if !prompt.is_empty() {
                session.push_message(Message::system(prompt.clone()));
                debug!(
                    session_id = %session.id.0,
                    "Injected session reset prompt"
                );
            }
        }

        // Phase 3: AfterReset-position injections.
        for inj in &all_injections {
            if inj.position == InjectionPosition::AfterReset && condition_met(&inj.condition) {
                session.push_message(Message::system(inj.content.clone()));
                debug!(
                    session_id = %session.id.0,
                    injection = %inj.name,
                    "Injected context (after_reset position)"
                );
            }
        }

        // Phase 4: BeforeUser-position injections (appended; they logically
        // precede user messages that haven't arrived yet).
        //
        // Track message count before injection so we can roll back the
        // in-memory state if the persist fails (issue #3672). Without a
        // rollback, the next pass sees the injected messages in-memory but
        // not on-disk, re-injects them, and silently invalidates the prompt
        // cache.
        let pre_before_user_len = session.messages.len();
        for inj in &all_injections {
            if inj.position == InjectionPosition::BeforeUser && condition_met(&inj.condition) {
                session.push_message(Message::system(inj.content.clone()));
                debug!(
                    session_id = %session.id.0,
                    injection = %inj.name,
                    "Injected context (before_user position)"
                );
            }
        }

        // Persist if anything was injected.
        // Sync save_session: caller `inject_reset_prompt` is a sync fn, no `.await` allowed.
        if !session.messages.is_empty() {
            if let Err(e) = self.memory.substrate.save_session(session) {
                // Persist failed — roll back the Phase 4 BeforeUser injections
                // from the in-memory session so the next call does not
                // re-inject the same items (which would cause duplicate
                // context and invalidate the prompt cache).
                let after_len = session.messages.len();
                if after_len > pre_before_user_len {
                    session.messages.truncate(pre_before_user_len);
                    session.mark_messages_mutated();
                }
                tracing::error!(
                    session_id = %session.id.0,
                    error = %e,
                    rolled_back = after_len.saturating_sub(pre_before_user_len),
                    "Failed to persist session after before_user injection; \
                     rolled back in-memory mutations to prevent duplicate injection \
                     and prompt-cache invalidation"
                );
            }
        }

        // Run on_session_start_script if configured (fire-and-forget).
        if let Some(ref script) = cfg.session.on_session_start_script {
            if !script.is_empty() {
                let script = script.clone();
                let aid = agent_id.to_string();
                let sid = session.id.0.to_string();
                std::thread::spawn(move || {
                    match std::process::Command::new(&script)
                        .arg(&aid)
                        .arg(&sid)
                        .output()
                    {
                        Ok(output) => {
                            if !output.status.success() {
                                tracing::warn!(
                                    script = %script,
                                    status = %output.status,
                                    "on_session_start_script exited with non-zero status"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                script = %script,
                                error = %e,
                                "Failed to run on_session_start_script"
                            );
                        }
                    }
                });
            }
        }
    }

    /// Evaluate a simple condition expression against agent tags.
    ///
    /// Currently supports:
    /// - `"agent.tags contains '<tag>'"` — true if the agent has the given tag
    /// - `None` or empty string — always true
    pub(crate) fn evaluate_condition(condition: &Option<String>, agent_tags: &[String]) -> bool {
        let cond = match condition {
            Some(c) if !c.is_empty() => c,
            _ => return true,
        };

        // Parse "agent.tags contains 'value'"
        if let Some(rest) = cond.strip_prefix("agent.tags contains ") {
            let tag = rest.trim().trim_matches('\'').trim_matches('"');
            return agent_tags.iter().any(|t| t == tag);
        }

        // Unknown condition format — default to false (strict). Prevents accidental injection.
        tracing::warn!(condition = %cond, "Unknown condition format, skipping injection");
        false
    }

    /// Save a summary of the about-to-be-deleted session to agent memory (#4869).
    ///
    /// The fire-and-forget summary path on `reset_session` had three
    /// independent defects before this rewrite:
    ///
    /// 1. **Last-10-messages window** — for any non-trivial session, the
    ///    trailing 10 turns are dominated by closing pleasantries
    ///    ("thanks", "sure", "you too") or mid-tool-loop plumbing. Real
    ///    user goals from earlier in the session were never visible.
    /// 2. **Text-only user messages** — `MessageContent::Text` was the
    ///    only variant considered; sessions that ended on a tool-result
    ///    turn produced **no summary at all**, because the early-return
    ///    on `topics.is_empty()` fired before anything was written.
    /// 3. **Collision-prone key** — `session_{date}_{slug}` overwrote
    ///    itself across same-day sessions whose first user message
    ///    slugged identically ("Thanks", "OK", "Sure").
    ///
    /// The fix: route the summary through the auxiliary LLM
    /// (`AuxTask::SessionSummary`) over the **entire** session message
    /// vector, then key the resulting `kv_store` entry by the session
    /// id (collision-free). When no auxiliary chain is configured, log
    /// a WARN and fall back to the historical trivial summary so the
    /// path remains graceful — operators get a visible degraded-mode
    /// signal instead of silent quality loss.
    fn save_session_summary(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        session: &librefang_memory::session::Session,
    ) {
        let memory = Arc::clone(&self.memory.substrate);
        let aux = self.llm.aux_client.load_full();
        let catalog = self.llm.model_catalog.load_full();
        let agent_name = entry.name.clone();
        let workspace = entry.manifest.workspace.clone();
        let messages = session.messages.clone();
        let session_id = session.id;

        // Every production caller of `reset_session` runs inside an axum
        // handler so a tokio runtime is present, but `reset_session` itself
        // is sync (`pub fn`); a future sync caller would crash the daemon
        // if we used `Handle::current()`. `try_current()` lets us spawn the
        // aux-LLM path when a runtime is available and degrade to a
        // synchronous trivial-digest write when it is not.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let summary = build_session_summary(
                        aux.as_ref(),
                        catalog.as_ref(),
                        agent_id,
                        session_id,
                        &agent_name,
                        &messages,
                    )
                    .await;
                    persist_session_summary(
                        memory.as_ref(),
                        agent_id,
                        session_id,
                        workspace.as_deref(),
                        &summary,
                    );
                });
            }
            Err(_) => {
                warn!(
                    agent_id = %agent_id,
                    session_id = %session_id.0,
                    "save_session_summary called outside a tokio runtime; \
                     writing trivial digest synchronously (aux-LLM path skipped)"
                );
                let summary = build_trivial_session_summary(&messages);
                persist_session_summary(
                    memory.as_ref(),
                    agent_id,
                    session_id,
                    workspace.as_deref(),
                    &summary,
                );
            }
        }
    }
}

/// Maximum byte length of conversation text fed to the aux LLM. Keeps
/// the prompt within a haiku-class context window even on very long
/// sessions; when the transcript exceeds this, content is dropped
/// from the **head** so the most-recent context (the load-bearing
/// part for a summary) survives. Named in bytes — not characters —
/// because `String::len` is the unit we actually measure against.
const MAX_SUMMARY_INPUT_BYTES: usize = 48_000;

/// Cap on the bytes we write to `kv_store` / `memory/*.md`. Aux models
/// rarely emit more than a few kB of summary, but a misbehaving model
/// can produce a runaway response — bound the disk + DB cost.
const MAX_SUMMARY_OUTPUT_BYTES: usize = 16_384;

/// Hard cap on the wall-clock time the aux LLM has to produce a summary.
/// The summary is fire-and-forget so a slow path doesn't block
/// `reset_session`, but we still want the spawned task to terminate.
const SESSION_SUMMARY_LLM_TIMEOUT_SECS: u64 = 30;

/// Build the session summary, preferring the auxiliary LLM and falling
/// back to a trivial digest if no aux chain resolves or the call fails.
async fn build_session_summary(
    aux: &librefang_runtime::aux_client::AuxClient,
    catalog: &librefang_runtime::model_catalog::ModelCatalog,
    agent_id: AgentId,
    session_id: SessionId,
    agent_name: &str,
    messages: &[librefang_types::message::Message],
) -> String {
    use librefang_runtime::llm_driver::CompletionRequest;
    use librefang_types::config::AuxTask;
    use librefang_types::message::Message;

    let resolution = aux.resolve(AuxTask::SessionSummary);

    if resolution.used_primary {
        // No aux chain resolved — keep the on-reset write useful but
        // log loudly so operators see the degraded mode.
        warn!(
            agent_id = %agent_id,
            session_id = %session_id.0,
            "Session-summary aux chain unconfigured (or all entries skipped); falling back to trivial summary. \
             Configure [llm.auxiliary] session_summary in config.toml for high-quality summaries."
        );
        return build_trivial_session_summary(messages);
    }

    let transcript = render_session_transcript(messages, MAX_SUMMARY_INPUT_BYTES);
    if transcript.is_empty() {
        return build_trivial_session_summary(messages);
    }

    let model = resolution
        .resolved
        .first()
        .map(|(_, m)| m.clone())
        .unwrap_or_default();
    let echo_policy = catalog
        .find_model(&model)
        .map(|e| e.reasoning_echo_policy)
        .unwrap_or_default();

    let system = "You summarise agent sessions. Output plain markdown only — no preamble, no \
                  meta-commentary, no code fences. Aim for 5–10 bullets covering the user's goal, \
                  the work actually performed (including tool calls), entities or files written, \
                  decisions taken, and the final state of the session.";
    let user_prompt = format!(
        "Agent: {agent_name}\n\
         Session: {session_id}\n\
         \n\
         Conversation transcript follows. Summarise it per the instructions.\n\
         \n\
         {transcript}",
        session_id = session_id.0,
    );

    let req = CompletionRequest {
        model,
        messages: std::sync::Arc::new(vec![Message::user(user_prompt)]),
        tools: std::sync::Arc::new(vec![]),
        max_tokens: 1024,
        temperature: 0.2,
        system: Some(system.to_string()),
        thinking: None,
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        response_format: None,
        timeout_secs: Some(SESSION_SUMMARY_LLM_TIMEOUT_SECS),
        extra_body: None,
        agent_id: Some(agent_id.to_string()),
        session_id: Some(session_id.0.to_string()),
        step_id: None,
        reasoning_echo_policy: echo_policy,
    };

    invoke_summary_driver(
        resolution.driver.as_ref(),
        req,
        std::time::Duration::from_secs(SESSION_SUMMARY_LLM_TIMEOUT_SECS),
        agent_id,
        session_id,
        messages,
    )
    .await
}

/// Drive the aux LLM call for a session summary and translate the four
/// terminal states (ok-text, ok-empty, driver error, timeout) into
/// either the model's text or the trivial fallback. Extracted from
/// [`build_session_summary`] so tests can inject a `MockLlmDriver`
/// without standing up an `AuxClient`.
async fn invoke_summary_driver(
    driver: &dyn librefang_runtime::llm_driver::LlmDriver,
    req: librefang_runtime::llm_driver::CompletionRequest,
    timeout: std::time::Duration,
    agent_id: AgentId,
    session_id: SessionId,
    fallback_messages: &[librefang_types::message::Message],
) -> String {
    let outcome = tokio::time::timeout(timeout, driver.complete(req)).await;

    match outcome {
        Ok(Ok(resp)) => {
            let text = resp.text().trim().to_string();
            if text.is_empty() {
                warn!(
                    agent_id = %agent_id,
                    session_id = %session_id.0,
                    "Session-summary aux LLM returned empty text; falling back to trivial summary"
                );
                build_trivial_session_summary(fallback_messages)
            } else {
                text
            }
        }
        Ok(Err(e)) => {
            warn!(
                agent_id = %agent_id,
                session_id = %session_id.0,
                error = %e,
                "Session-summary aux LLM call failed; falling back to trivial summary"
            );
            build_trivial_session_summary(fallback_messages)
        }
        Err(_) => {
            warn!(
                agent_id = %agent_id,
                session_id = %session_id.0,
                timeout_ms = timeout.as_millis() as u64,
                "Session-summary aux LLM call timed out; falling back to trivial summary"
            );
            build_trivial_session_summary(fallback_messages)
        }
    }
}

/// Render every message in the session as plain text — including
/// tool-use and tool-result blocks — capped at `max_bytes` of UTF-8
/// (matches `String::len`). The **head** is dropped when the
/// transcript overflows, because the most recent turns are usually
/// the most load-bearing for a summary (final decisions, last file
/// edits). `ContentBlock::Thinking` is intentionally excluded so
/// private chain-of-thought never reaches the on-disk summary.
fn render_session_transcript(
    messages: &[librefang_types::message::Message],
    max_bytes: usize,
) -> String {
    use librefang_types::message::{ContentBlock, MessageContent, Role};

    let mut rendered: Vec<String> = Vec::with_capacity(messages.len());
    for msg in messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        };
        let body = match &msg.content {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Blocks(blocks) => {
                let mut parts: Vec<String> = Vec::new();
                for block in blocks {
                    match block {
                        ContentBlock::Text { text, .. } => parts.push(text.clone()),
                        ContentBlock::ToolUse { name, input, .. } => {
                            parts.push(format!("[tool_use: {name}({input})]"));
                        }
                        ContentBlock::ToolResult { content, .. } => {
                            parts.push(format!("[tool_result: {content}]"));
                        }
                        ContentBlock::Thinking { .. } => {
                            // Internal model reasoning is intentionally
                            // dropped: a session summary lands in
                            // `kv_store` and `{workspace}/memory/*.md`,
                            // both of which persist across sessions.
                            // Chain-of-thought traces are private to
                            // the originating turn and must not leak
                            // there.
                        }
                        ContentBlock::Image { .. } | ContentBlock::ImageFile { .. } => {
                            parts.push("[image]".to_string());
                        }
                        ContentBlock::Unknown => {}
                    }
                }
                parts.join(" ")
            }
        };
        let trimmed = body.trim();
        if trimmed.is_empty() {
            continue;
        }
        rendered.push(format!("{role}: {trimmed}"));
    }

    let mut out = rendered.join("\n\n");
    if out.len() > max_bytes {
        // Drop from the head — keep the recent context the summary cares about.
        let overflow = out.len() - max_bytes;
        let mut split = overflow;
        while split < out.len() && !out.is_char_boundary(split) {
            split += 1;
        }
        out = out.split_off(split);
    }
    out
}

/// Fallback summary used when the aux LLM is unavailable. Captures the
/// session length, first / last substantive user turn, and the rough
/// shape of the work — enough to identify the session later when
/// browsing `kv_store`, but explicitly degraded relative to the aux-
/// LLM path. Replaces the pre-#4869 "last-10-messages" digest which
/// produced "thanks / sure / you too" on long sessions.
fn build_trivial_session_summary(messages: &[librefang_types::message::Message]) -> String {
    use librefang_types::message::{ContentBlock, MessageContent, Role};

    let mut user_turns: Vec<String> = Vec::new();
    let mut tool_use_count: usize = 0;
    let mut tool_result_count: usize = 0;
    let mut assistant_turns: usize = 0;

    for msg in messages {
        match msg.role {
            Role::User => {
                let body = msg.content.text_content();
                let trimmed = body.trim();
                if !trimmed.is_empty() {
                    user_turns.push(trimmed.to_string());
                }
                if let MessageContent::Blocks(blocks) = &msg.content {
                    for b in blocks {
                        if matches!(b, ContentBlock::ToolResult { .. }) {
                            tool_result_count += 1;
                        }
                    }
                }
            }
            Role::Assistant => {
                assistant_turns += 1;
                if let MessageContent::Blocks(blocks) = &msg.content {
                    for b in blocks {
                        if matches!(b, ContentBlock::ToolUse { .. }) {
                            tool_use_count += 1;
                        }
                    }
                }
            }
            Role::System => {}
        }
    }

    let date = chrono::Utc::now().format("%Y-%m-%d");
    let mut out = format!("Session on {date} (auto-summary, aux LLM unavailable)\n\n");
    out.push_str(&format!(
        "- Turns: {} user / {} assistant\n",
        user_turns.len(),
        assistant_turns
    ));
    out.push_str(&format!(
        "- Tool activity: {tool_use_count} tool calls, {tool_result_count} tool results\n"
    ));
    if let Some(first) = user_turns.first() {
        out.push_str(&format!(
            "- First user goal: {}\n",
            librefang_types::truncate_str(first, 240)
        ));
    }
    if user_turns.len() > 1 {
        if let Some(last) = user_turns.last() {
            out.push_str(&format!(
                "- Last user turn: {}\n",
                librefang_types::truncate_str(last, 240)
            ));
        }
    }
    out
}

/// Write the summary to `kv_store` (keyed by session id) and, if a
/// workspace is configured, to a per-session markdown file. Both
/// writes are best-effort — failures log and continue.
fn persist_session_summary(
    memory: &librefang_memory::MemorySubstrate,
    agent_id: AgentId,
    session_id: SessionId,
    workspace: Option<&std::path::Path>,
    raw_summary: &str,
) {
    let summary = if raw_summary.len() > MAX_SUMMARY_OUTPUT_BYTES {
        // Truncate at a UTF-8 boundary so the JSON value stays valid.
        let mut cutoff = MAX_SUMMARY_OUTPUT_BYTES;
        while cutoff > 0 && !raw_summary.is_char_boundary(cutoff) {
            cutoff -= 1;
        }
        let mut t = raw_summary[..cutoff].to_string();
        t.push_str("\n\n…[truncated]");
        t
    } else {
        raw_summary.to_string()
    };

    if summary.trim().is_empty() {
        debug!(
            agent_id = %agent_id,
            session_id = %session_id.0,
            "Skipping empty session summary"
        );
        return;
    }

    // Collision-free key: session id is unique per session. Pre-#4869
    // this was `session_{date}_{slug}` which silently overwrote across
    // sessions ending with the same slugified user message.
    let key = format!("session_{}", session_id.0);
    if let Err(e) =
        memory.structured_set(agent_id, &key, serde_json::Value::String(summary.clone()))
    {
        warn!(
            agent_id = %agent_id,
            session_id = %session_id.0,
            error = %e,
            "Failed to persist session summary to kv_store"
        );
    } else {
        debug!(
            agent_id = %agent_id,
            session_id = %session_id.0,
            key = %key,
            bytes = summary.len(),
            "Saved session summary to memory before reset"
        );
    }

    if let Some(workspace) = workspace {
        let mem_dir = workspace.join("memory");
        if mem_dir.exists() {
            let filename = format!("session-{}.md", session_id.0);
            let path = mem_dir.join(&filename);
            if let Err(e) = std::fs::write(&path, &summary) {
                debug!(
                    agent_id = %agent_id,
                    session_id = %session_id.0,
                    path = %path.display(),
                    error = %e,
                    "Failed to write session summary file to workspace"
                );
            }
        }
    }
}

#[cfg(test)]
mod session_summary_tests {
    use super::{build_trivial_session_summary, render_session_transcript};
    use librefang_types::message::{ContentBlock, Message, Role};

    fn tool_use(name: &str, input_text: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: librefang_types::message::MessageContent::Blocks(vec![
                ContentBlock::ToolUse {
                    id: "id-1".to_string(),
                    name: name.to_string(),
                    input: serde_json::json!({ "text": input_text }),
                    provider_metadata: None,
                },
            ]),
            pinned: false,
            timestamp: None,
        }
    }

    fn tool_result(content: &str) -> Message {
        Message::user_with_blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "id-1".to_string(),
            tool_name: String::new(),
            content: content.to_string(),
            is_error: false,
            status: librefang_types::tool::ToolExecutionStatus::default(),
            approval_request_id: None,
        }])
    }

    #[test]
    fn trivial_summary_survives_tool_result_only_tail() {
        // Pre-#4869 defect 2: a session ending mid-tool-loop produced
        // *no* summary at all because the filter found zero Text
        // user messages. The trivial summary must still produce output.
        let messages = vec![
            Message::user("Plan meals for the week"),
            Message::assistant("Sure — fetching your pantry"),
            tool_use("read_pantry", "/"),
            tool_result("eggs, rice, garlic"),
        ];
        let summary = build_trivial_session_summary(&messages);
        assert!(
            !summary.trim().is_empty(),
            "tool-result-only tail must still produce a summary"
        );
        assert!(
            summary.contains("Plan meals for the week"),
            "first user goal should appear in trivial summary"
        );
        assert!(
            summary.contains("Tool activity"),
            "trivial summary should mention tool activity"
        );
    }

    #[test]
    fn trivial_summary_reports_turn_counts() {
        let messages = vec![
            Message::user("first"),
            Message::assistant("a1"),
            Message::user("second"),
            Message::assistant("a2"),
            Message::user("third"),
        ];
        let summary = build_trivial_session_summary(&messages);
        assert!(summary.contains("3 user / 2 assistant"));
    }

    #[test]
    fn render_transcript_includes_tool_calls() {
        let messages = vec![
            Message::user("Read the pantry"),
            tool_use("read_pantry", "/"),
            tool_result("eggs, rice"),
        ];
        let rendered = render_session_transcript(&messages, 10_000);
        assert!(rendered.contains("user: Read the pantry"));
        assert!(rendered.contains("[tool_use: read_pantry"));
        assert!(rendered.contains("[tool_result: eggs, rice]"));
    }

    #[test]
    fn render_transcript_truncates_head_preserves_tail() {
        // Build a long synthetic transcript; verify the cap drops the
        // head (oldest content) and keeps the tail (most-recent context,
        // which is the load-bearing part for a summary).
        let mut messages = Vec::new();
        for i in 0..200 {
            messages.push(Message::user(format!("user turn {i:03}: padding text")));
            messages.push(Message::assistant(format!(
                "assistant turn {i:03}: more padding"
            )));
        }
        let rendered = render_session_transcript(&messages, 1_000);
        assert!(rendered.len() <= 1_000);
        assert!(
            !rendered.contains("user turn 000:"),
            "head should be dropped when over cap"
        );
        assert!(
            rendered.contains("turn 199:"),
            "tail (recent context) must survive the cap"
        );
    }

    #[test]
    fn render_transcript_omits_thinking_blocks() {
        // Internal chain-of-thought must not reach kv_store or the
        // workspace markdown — the session summary persists across
        // sessions and would leak private reasoning otherwise.
        let messages = vec![
            Message::user("What's 2+2?"),
            Message {
                role: Role::Assistant,
                content: librefang_types::message::MessageContent::Blocks(vec![
                    ContentBlock::Thinking {
                        thinking: "SECRET_REASONING_should_not_leak".to_string(),
                        provider_metadata: None,
                    },
                    ContentBlock::Text {
                        text: "Four.".to_string(),
                        provider_metadata: None,
                    },
                ]),
                pinned: false,
                timestamp: None,
            },
        ];
        let rendered = render_session_transcript(&messages, 10_000);
        assert!(
            !rendered.contains("SECRET_REASONING_should_not_leak"),
            "thinking text must never appear in the rendered transcript"
        );
        assert!(
            !rendered.contains("[thinking"),
            "no thinking placeholder either — the block is dropped entirely"
        );
        assert!(
            rendered.contains("Four."),
            "visible assistant text still survives"
        );
    }

    // --- aux-LLM driver invocation path ---------------------------------
    //
    // The four terminal states of `invoke_summary_driver`:
    //   ok-text  → return the model's text verbatim
    //   ok-empty → warn + fall back to trivial digest
    //   error    → warn + fall back to trivial digest
    //   timeout  → warn + fall back to trivial digest
    //
    // Pre-#4869 there was no aux call at all, so these are the load-
    // bearing new branches.

    use super::invoke_summary_driver;
    use async_trait::async_trait;
    use librefang_runtime::llm_driver::{
        CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent,
    };
    use librefang_testing::{FailingLlmDriver, MockLlmDriver};
    use librefang_types::agent::{AgentId, SessionId};
    use librefang_types::message::{StopReason, TokenUsage};
    use std::time::Duration;

    fn dummy_request() -> CompletionRequest {
        CompletionRequest {
            model: "test-model".to_string(),
            messages: std::sync::Arc::new(vec![Message::user("hi")]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 256,
            temperature: 0.2,
            system: Some("system".to_string()),
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: Some(5),
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: Default::default(),
        }
    }

    fn trivial_signal_messages() -> Vec<Message> {
        // Trivial summary should be recognisable so tests can tell
        // "the fallback ran" from "the driver text was used".
        vec![
            Message::user("FALLBACK_FIRST_GOAL_MARKER"),
            Message::assistant("ok"),
        ]
    }

    #[tokio::test]
    async fn invoke_summary_driver_returns_aux_text_when_ok() {
        let driver = MockLlmDriver::with_response("- bullet one\n- bullet two");
        let messages = trivial_signal_messages();
        let out = invoke_summary_driver(
            &driver,
            dummy_request(),
            Duration::from_secs(5),
            AgentId::new(),
            SessionId::new(),
            &messages,
        )
        .await;
        assert_eq!(out, "- bullet one\n- bullet two");
        assert!(
            !out.contains("FALLBACK_FIRST_GOAL_MARKER"),
            "trivial fallback must not run on the happy path"
        );
    }

    #[tokio::test]
    async fn invoke_summary_driver_falls_back_when_text_empty() {
        let driver = MockLlmDriver::with_response("");
        let messages = trivial_signal_messages();
        let out = invoke_summary_driver(
            &driver,
            dummy_request(),
            Duration::from_secs(5),
            AgentId::new(),
            SessionId::new(),
            &messages,
        )
        .await;
        assert!(
            out.contains("FALLBACK_FIRST_GOAL_MARKER"),
            "empty aux response should produce the trivial fallback (got: {out:?})"
        );
    }

    #[tokio::test]
    async fn invoke_summary_driver_falls_back_on_driver_error() {
        let driver = FailingLlmDriver::new("simulated 500");
        let messages = trivial_signal_messages();
        let out = invoke_summary_driver(
            &driver,
            dummy_request(),
            Duration::from_secs(5),
            AgentId::new(),
            SessionId::new(),
            &messages,
        )
        .await;
        assert!(
            out.contains("FALLBACK_FIRST_GOAL_MARKER"),
            "driver error should produce the trivial fallback (got: {out:?})"
        );
    }

    /// Driver whose `complete` future never resolves within the test
    /// timeout — pins the `tokio::time::timeout` arm.
    struct SleepyDriver {
        sleep_for: Duration,
    }

    #[async_trait]
    impl LlmDriver for SleepyDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            tokio::time::sleep(self.sleep_for).await;
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "too-late".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
                actual_provider: None,
            })
        }

        async fn stream(
            &self,
            request: CompletionRequest,
            _tx: tokio::sync::mpsc::Sender<StreamEvent>,
        ) -> Result<CompletionResponse, LlmError> {
            self.complete(request).await
        }

        fn is_configured(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn invoke_summary_driver_falls_back_on_timeout() {
        let driver = SleepyDriver {
            sleep_for: Duration::from_secs(60),
        };
        let messages = trivial_signal_messages();
        let out = invoke_summary_driver(
            &driver,
            dummy_request(),
            Duration::from_millis(50),
            AgentId::new(),
            SessionId::new(),
            &messages,
        )
        .await;
        assert!(
            out.contains("FALLBACK_FIRST_GOAL_MARKER"),
            "timeout should produce the trivial fallback (got: {out:?})"
        );
        assert!(
            !out.contains("too-late"),
            "the never-resolving driver text must not leak through"
        );
    }
}
