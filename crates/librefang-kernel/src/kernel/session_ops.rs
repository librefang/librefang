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
        if self.registry.get(agent_id).is_none() {
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
            self.injection_senders
                .get(&(agent_id, sid))
                .map(|entry| (*entry.key(), entry.value().clone()))
                .into_iter()
                .collect()
        } else {
            self.injection_senders
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
            self.injection_senders.remove(key);
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
        self.injection_senders.insert((agent_id, session_id), tx);
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        self.injection_receivers
            .insert((agent_id, session_id), Arc::clone(&rx));
        rx
    }

    /// Tears down the `(agent_id, session_id)` injection channel after the loop finishes.
    pub(crate) fn teardown_injection_channel(&self, agent_id: AgentId, session_id: SessionId) {
        self.injection_senders.remove(&(agent_id, session_id));
        self.injection_receivers.remove(&(agent_id, session_id));
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

    /// Reset an agent's session — auto-saves a summary to memory, then clears messages
    /// and creates a fresh session ID.
    pub fn reset_session(&self, agent_id: AgentId) -> KernelResult<()> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // Auto-save session summaries for ALL sessions (default + per-channel)
        // before clearing, so no channel's conversation history is silently lost.
        // Also emit session:end for each active session before deletion.
        if let Ok(session_ids) = self.memory.get_agent_session_ids(agent_id) {
            for sid in session_ids {
                if let Ok(Some(old_session)) = self.memory.get_session(sid) {
                    // Fire session:end before removing the old session.
                    self.external_hooks.fire(
                        crate::hooks::ExternalHookEvent::SessionEnd,
                        serde_json::json!({
                            "agent_id": agent_id.to_string(),
                            "session_id": old_session.id.0.to_string(),
                        }),
                    );
                    if old_session.messages.len() >= 2 {
                        self.save_session_summary(agent_id, &entry, &old_session);
                    }
                }
            }
        }

        // Delete ALL sessions for this agent (default + per-channel).
        // Propagate the error so callers see a half-failed reset instead
        // of silently leaving orphan rows in `sessions` / `sessions_fts`
        // (#3470). The deletion itself is transactional inside
        // `delete_agent_sessions`.
        self.memory
            .delete_agent_sessions(agent_id)
            .map_err(KernelError::LibreFang)?;

        // Create a fresh session and inject reset prompt if configured
        let mut new_session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::LibreFang)?;
        self.inject_reset_prompt(&mut new_session, agent_id);

        // Update registry with new session ID
        self.registry
            .update_session_id(agent_id, new_session.id)
            .map_err(KernelError::LibreFang)?;

        // Reset quota tracking so /new clears "token quota exceeded"
        self.scheduler.reset_usage(agent_id);

        // Fire external session:reset hook (fire-and-forget).
        self.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionReset,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        // Fire session:start for the newly created session.
        self.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionStart,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        info!(agent_id = %agent_id, "Session reset (summary saved to memory)");
        Ok(())
    }

    /// Hard-reboot an agent's session — clears conversation history WITHOUT saving
    /// a summary to memory.  Keeps agent config, system prompt, and tools intact.
    /// More aggressive than `reset_session` (which auto-saves a summary) but less
    /// destructive than `clear_agent_history` (which wipes ALL sessions).
    pub fn reboot_session(&self, agent_id: AgentId) -> KernelResult<()> {
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // Emit session:end for each active session before deletion.
        if let Ok(session_ids) = self.memory.get_agent_session_ids(agent_id) {
            for sid in session_ids {
                self.external_hooks.fire(
                    crate::hooks::ExternalHookEvent::SessionEnd,
                    serde_json::json!({
                        "agent_id": agent_id.to_string(),
                        "session_id": sid.0.to_string(),
                    }),
                );
            }
        }

        // Delete ALL sessions for this agent (default + per-channel).
        // Propagate so a failed reboot is visible instead of silently
        // leaving the old history in place (#3470).
        self.memory
            .delete_agent_sessions(agent_id)
            .map_err(KernelError::LibreFang)?;

        // Create a fresh session
        let new_session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::LibreFang)?;

        // Update registry with new session ID
        self.registry
            .update_session_id(agent_id, new_session.id)
            .map_err(KernelError::LibreFang)?;

        // Reset quota tracking
        self.scheduler.reset_usage(agent_id);

        // Fire external session:reset hook (fire-and-forget).
        self.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionReset,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        // Fire session:start for the newly created session to match the
        // behaviour of other new-session flows.
        self.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionStart,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        info!(agent_id = %agent_id, "Session rebooted (no summary saved)");
        Ok(())
    }

    /// Clear ALL conversation history for an agent (sessions + canonical).
    ///
    /// Creates a fresh empty session afterward so the agent is still usable.
    pub fn clear_agent_history(&self, agent_id: AgentId) -> KernelResult<()> {
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // Emit session:end for each active session before deletion.
        if let Ok(session_ids) = self.memory.get_agent_session_ids(agent_id) {
            for sid in session_ids {
                self.external_hooks.fire(
                    crate::hooks::ExternalHookEvent::SessionEnd,
                    serde_json::json!({
                        "agent_id": agent_id.to_string(),
                        "session_id": sid.0.to_string(),
                    }),
                );
            }
        }

        // Delete all regular sessions then the canonical (cross-channel)
        // session. Propagate either failure: a half-cleared agent leaves
        // orphan rows in `sessions` / `sessions_fts` / `canonical_sessions`
        // and is the silent-data-loss vector behind #3470.
        self.memory
            .delete_agent_sessions(agent_id)
            .map_err(KernelError::LibreFang)?;
        self.memory
            .delete_canonical_session(agent_id)
            .map_err(KernelError::LibreFang)?;

        // Create a fresh session and inject reset prompt if configured
        let mut new_session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::LibreFang)?;
        self.inject_reset_prompt(&mut new_session, agent_id);

        // Update registry with new session ID
        self.registry
            .update_session_id(agent_id, new_session.id)
            .map_err(KernelError::LibreFang)?;

        // Reset quota tracking
        self.scheduler.reset_usage(agent_id);

        // Fire external session:reset hook (fire-and-forget).
        self.external_hooks.fire(
            crate::hooks::ExternalHookEvent::SessionReset,
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "session_id": new_session.id.0.to_string(),
            }),
        );

        // Fire session:start for the newly created session.
        self.external_hooks.fire(
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
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let mut sessions = self
            .memory
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
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let mut session = self
            .memory
            .create_session_with_label(agent_id, label)
            .map_err(KernelError::LibreFang)?;
        self.inject_reset_prompt(&mut session, agent_id);

        // Switch to the new session
        self.registry
            .update_session_id(agent_id, session.id)
            .map_err(KernelError::LibreFang)?;

        // Fire external session:start hook for the newly created session.
        self.external_hooks.fire(
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
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        // Verify session exists and belongs to this agent
        let session = self
            .memory
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

        self.registry
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
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::LibreFang(LibreFangError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
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
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
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
            messages_generation: 0,
            last_repaired_generation: None,
        };
        // Sync save_session: caller `import_session` is a sync fn, no `.await` allowed.
        self.memory
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
            .registry
            .get(agent_id)
            .map(|entry| entry.manifest.context_injection.clone())
            .unwrap_or_default();

        // Collect agent tags for condition evaluation.
        let agent_tags: Vec<String> = self
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
            if let Err(e) = self.memory.save_session(session) {
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

    /// Save a summary of the current session to agent memory before reset.
    fn save_session_summary(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        session: &librefang_memory::session::Session,
    ) {
        use librefang_types::message::{MessageContent, Role};

        // Take last 10 messages (or all if fewer)
        let recent = &session.messages[session.messages.len().saturating_sub(10)..];

        // Extract key topics from user messages
        let topics: Vec<&str> = recent
            .iter()
            .filter(|m| m.role == Role::User)
            .filter_map(|m| match &m.content {
                MessageContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();

        if topics.is_empty() {
            return;
        }

        // Generate a slug from first user message (first 6 words, slugified)
        let slug: String = topics[0]
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .take(60)
            .collect();

        let date = chrono::Utc::now().format("%Y-%m-%d");
        let summary = format!(
            "Session on {date}: {slug}\n\nKey exchanges:\n{}",
            topics
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, t)| {
                    let truncated = librefang_types::truncate_str(t, 200);
                    format!("{}. {}", i + 1, truncated)
                })
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Save to structured memory store (key = "session_{date}_{slug}")
        let key = format!("session_{date}_{slug}");
        let _ =
            self.memory
                .structured_set(agent_id, &key, serde_json::Value::String(summary.clone()));

        // Also write to workspace memory/ dir if workspace exists
        if let Some(ref workspace) = entry.manifest.workspace {
            let mem_dir = workspace.join("memory");
            let filename = format!("{date}-{slug}.md");
            let _ = std::fs::write(mem_dir.join(&filename), &summary);
        }

        debug!(
            agent_id = %agent_id,
            key = %key,
            "Saved session summary to memory before reset"
        );
    }
}
