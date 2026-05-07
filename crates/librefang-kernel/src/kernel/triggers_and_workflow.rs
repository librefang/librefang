//! Cluster pulled out of mod.rs in #4713 phase 3e/4.
//!
//! Hosts the trigger / event-publish surface and the workflow execution
//! entry points: `spawn_session_label_generation`, `publish_event` plus
//! its private `publish_event_inner`, `register_trigger`,
//! `register_trigger_with_target`, and `run_workflow`. These methods sit
//! at the boundary between the event bus / trigger engine / workflow
//! engine substrates and the kernel's public dispatch surface — grouping
//! them keeps the trigger-id allocation, target resolution, and
//! workflow-step plumbing reviewable in one place.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Auto-generate a short session title via the auxiliary cheap-tier
    /// LLM and persist it to `sessions.label`. Fire-and-forget — runs in
    /// a tokio task so the originating turn is never blocked.
    ///
    /// No-op when:
    /// - the session already has a label (user-set or previously generated)
    /// - the session lacks at least one non-empty user + one non-empty
    ///   assistant message (nothing to summarise yet)
    /// - the aux driver call fails or times out
    /// - the model returns empty / all-whitespace text
    pub fn spawn_session_label_generation(&self, agent_id: AgentId, session_id: SessionId) {
        let memory = Arc::clone(&self.memory.substrate);
        let aux = self.llm.aux_client.load_full();
        tokio::spawn(async move {
            // Bail early if the label is already set — preserves user
            // overrides and prevents repeated billing on the same session.
            let session = match memory.get_session(session_id) {
                Ok(Some(s)) => s,
                Ok(None) => return,
                Err(e) => {
                    debug!(
                        session_id = %session_id.0,
                        error = %e,
                        "session-label: failed to load session"
                    );
                    return;
                }
            };
            if session.label.is_some() {
                return;
            }
            let Some((user_text, assistant_text)) = extract_label_seed(&session.messages) else {
                return;
            };

            let resolution = aux.resolve(librefang_types::config::AuxTask::Title);
            let driver = resolution.driver;
            // When the chain resolved a concrete (provider, model) use it; if
            // we fell back to the primary driver `resolved` is empty — the
            // driver will pick its own configured model.
            let model = resolution
                .resolved
                .first()
                .map(|(_, m)| m.clone())
                .unwrap_or_default();

            let prompt = format!(
                "Conversation so far:\nUser: {user}\nAssistant: {asst}\n\n\
                 Write a 3 to 6 word title for this conversation. \
                 Reply with the title text only — no quotes, no punctuation, no prefix.",
                user = librefang_types::truncate_str(&user_text, 800),
                asst = librefang_types::truncate_str(&assistant_text, 800),
            );

            let req = CompletionRequest {
                model,
                messages: std::sync::Arc::new(vec![librefang_types::message::Message::user(
                    prompt,
                )]),
                tools: std::sync::Arc::new(vec![]),
                max_tokens: 32,
                temperature: 0.2,
                system: Some(
                    "You generate short, descriptive session titles. \
                     Reply with the title text only."
                        .to_string(),
                ),
                thinking: None,
                prompt_caching: false,
                cache_ttl: None,
                response_format: None,
                timeout_secs: None,
                extra_body: None,
                agent_id: Some(agent_id.to_string()),
                session_id: Some(session_id.0.to_string()),
                step_id: None,
            };

            let resp = match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                driver.complete(req),
            )
            .await
            {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    debug!(
                        agent_id = %agent_id,
                        session_id = %session_id.0,
                        error = %e,
                        "session-label: aux LLM call failed"
                    );
                    return;
                }
                Err(_) => {
                    debug!(
                        agent_id = %agent_id,
                        session_id = %session_id.0,
                        "session-label: aux LLM call timed out (10s)"
                    );
                    return;
                }
            };

            let title = sanitize_session_title(&resp.text());
            if title.is_empty() {
                return;
            }

            // Re-check the label right before writing — a concurrent
            // user-set label via PUT /api/sessions/:id/label must win.
            if let Ok(Some(s)) = memory.get_session(session_id) {
                if s.label.is_some() {
                    return;
                }
            }

            if let Err(e) = memory.set_session_label(session_id, Some(&title)) {
                debug!(
                    agent_id = %agent_id,
                    session_id = %session_id.0,
                    error = %e,
                    "session-label: failed to persist label"
                );
            } else {
                info!(
                    agent_id = %agent_id,
                    session_id = %session_id.0,
                    title = %title,
                    "Auto-generated session label"
                );
            }
        });
    }

    /// Lightweight one-shot LLM call for classification tasks (e.g., reply precheck).
    ///
    /// Uses the default driver with low max_tokens and 0 temperature.
    /// Returns `Err` on LLM error or timeout (caller should fail-open).
    pub async fn one_shot_llm_call(&self, model: &str, prompt: &str) -> Result<String, String> {
        use librefang_runtime::llm_driver::CompletionRequest;
        use librefang_types::message::Message;

        let request = CompletionRequest {
            model: model.to_string(),
            messages: std::sync::Arc::new(vec![Message::user(prompt.to_string())]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 10,
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
        };

        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.llm.default_driver.complete(request),
        )
        .await
        {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(format!("LLM call failed: {e}")),
            Err(_) => return Err("LLM call timed out (5s)".to_string()),
        };

        Ok(result.text())
    }

    /// Publish an event to the bus and evaluate triggers.
    ///
    /// Any matching triggers will dispatch messages to the subscribing agents.
    /// Returns the list of trigger matches that were dispatched.
    /// Includes depth limiting to prevent circular trigger chains.
    pub async fn publish_event(&self, event: Event) -> Vec<crate::triggers::TriggerMatch> {
        let already_scoped = PUBLISH_EVENT_DEPTH.try_with(|_| ()).is_ok();

        if already_scoped {
            self.publish_event_inner(event).await
        } else {
            // Top-level invocation — establish an isolated per-chain scope.
            PUBLISH_EVENT_DEPTH
                .scope(std::cell::Cell::new(0), self.publish_event_inner(event))
                .await
        }
    }

    /// Inner body of [`publish_event`]; requires `PUBLISH_EVENT_DEPTH` scope to be active.
    async fn publish_event_inner(&self, event: Event) -> Vec<crate::triggers::TriggerMatch> {
        let cfg = self.config.load_full();
        let max_trigger_depth = cfg.triggers.max_depth as u32;

        let depth = PUBLISH_EVENT_DEPTH.with(|c| {
            let d = c.get();
            c.set(d + 1);
            d
        });

        if depth >= max_trigger_depth {
            // Restore before returning — no drop guard in the early-exit path.
            PUBLISH_EVENT_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
            warn!(
                depth,
                "Trigger depth limit reached, skipping evaluation to prevent circular chain"
            );
            return vec![];
        }

        // Decrement on all exit paths via drop guard.
        struct DepthGuard;
        impl Drop for DepthGuard {
            fn drop(&mut self) {
                // Guard is only created after the early-exit check, so the scope is always active.
                let _ = PUBLISH_EVENT_DEPTH.try_with(|c| c.set(c.get().saturating_sub(1)));
            }
        }
        let _guard = DepthGuard;

        // Evaluate triggers before publishing (so describe_event works on the event)
        let (triggered, trigger_state_mutated) = self
            .workflows
            .triggers
            .evaluate_with_resolver(&event, |id| self.registry.get(id).map(|e| e.name.clone()));
        if !triggered.is_empty() || trigger_state_mutated {
            if let Err(e) = self.workflows.triggers.persist() {
                warn!("Failed to persist trigger jobs after fire: {e}");
            }
        }

        // Publish to the event bus
        self.events.event_bus.publish(event).await;

        // Actually dispatch triggered messages to agents.
        //
        // Concurrency model — three layered semaphores, in order:
        //   1. Global Lane::Trigger (config: queue.concurrency.trigger_lane).
        //      Caps total in-flight trigger dispatches kernel-wide so a
        //      runaway producer (50× task_post in a tight loop) can't spawn
        //      unbounded tokio tasks racing for everyone else's mutexes.
        //   2. Per-agent semaphore (config: manifest.max_concurrent_invocations
        //      → fallback queue.concurrency.default_per_agent → 1).
        //      Caps how many of THIS agent's fires run in parallel.
        //   3. Per-session mutex (existing session_msg_locks at
        //      send_message_full).  Reached only when we materialize a
        //      `session_id_override` here for `session_mode = "new"`
        //      effective mode — otherwise the inner code path falls back
        //      to the per-agent lock and blocks parallelism inside
        //      send_message_full regardless of how many permits we hold.
        //
        // Resolution order for effective session mode:
        //   trigger_match.session_mode_override → manifest.session_mode.
        // We materialize `SessionId::new()` only when the resolved mode is
        // `New`; persistent fires reuse the canonical session and must
        // serialize at the per-agent mutex, so we leave session_id_override
        // = None for them.
        // Bug #3841: burst events fire triggers out-of-order via independent
        // tokio::spawn.  Fix: collect all trigger dispatches for this event
        // into a single spawned task and execute them **sequentially** inside
        // it.  Each individual dispatch still acquires the global trigger-lane
        // semaphore and per-agent semaphore, preserving all existing
        // concurrency limits — but triggers produced by the same event are
        // now guaranteed to reach agents in evaluation order, not in arbitrary
        // tokio scheduler order.
        if let Some(weak) = self.self_handle.get() {
            // Pre-resolve per-trigger data before spawning so the spawned
            // future does not borrow `self` or `triggered` across the await.
            struct TriggerDispatch {
                kernel: Arc<LibreFangKernel>,
                aid: AgentId,
                msg: String,
                mode_override: Option<librefang_types::agent::SessionMode>,
                session_id_override: Option<SessionId>,
                trigger_sem: Arc<tokio::sync::Semaphore>,
                agent_sem: Arc<tokio::sync::Semaphore>,
            }

            let mut dispatches: Vec<TriggerDispatch> = Vec::with_capacity(triggered.len());
            for trigger_match in &triggered {
                let kernel = match weak.upgrade() {
                    Some(k) => k,
                    None => continue,
                };
                let aid = trigger_match.agent_id;
                let msg = trigger_match.message.clone();
                let mode_override = trigger_match.session_mode_override;

                // Resolve the effective session mode now so we can decide
                // whether to materialize a fresh session id. Skip dispatch
                // if the agent has been deleted between trigger evaluation
                // and dispatch — preserves prior behavior.
                let manifest_mode = match kernel.registry.get(aid) {
                    Some(entry) => entry.manifest.session_mode,
                    None => continue,
                };
                let effective_mode = mode_override.unwrap_or(manifest_mode);
                let session_id_override = match effective_mode {
                    librefang_types::agent::SessionMode::New => Some(SessionId::new()),
                    librefang_types::agent::SessionMode::Persistent => None,
                };

                let trigger_sem = kernel
                    .workflows
                    .command_queue
                    .semaphore_for_lane(librefang_runtime::command_lane::Lane::Trigger);
                let agent_sem = kernel.agent_concurrency_for(aid);

                dispatches.push(TriggerDispatch {
                    kernel,
                    aid,
                    msg,
                    mode_override,
                    session_id_override,
                    trigger_sem,
                    agent_sem,
                });
            }

            // Per-fire timeout cap (#3446): one stuck send_message_full
            // must NOT pin Lane::Trigger permits indefinitely.
            let fire_timeout_s = self
                .config
                .load()
                .queue
                .concurrency
                .trigger_fire_timeout_secs;
            let fire_timeout = std::time::Duration::from_secs(fire_timeout_s);

            if !dispatches.is_empty() {
                // CRITICAL: tokio task-locals do NOT propagate across
                // tokio::spawn.  Without re-establishing the
                // PUBLISH_EVENT_DEPTH scope inside the spawned task,
                // every send_message_full -> publish_event chain
                // started from a triggered dispatch would observe an
                // unscoped depth, fall into the "top-level scope"
                // branch, and reset depth=0 — the exact path that
                // breaks circular trigger detection across the spawn
                // boundary (audit of #3929 / #3780).  Capture the
                // parent depth here on the caller's task and rebuild
                // the scope inside the spawn so trigger chains
                // accumulate correctly.
                let parent_depth = PUBLISH_EVENT_DEPTH.try_with(|c| c.get()).unwrap_or(0);
                let task =
                    PUBLISH_EVENT_DEPTH.scope(std::cell::Cell::new(parent_depth), async move {
                        // Execute trigger dispatches sequentially to preserve
                        // the order in which the trigger engine evaluated them.
                        // Each dispatch still acquires its semaphore permits
                        // (global trigger-lane + per-agent) before calling
                        // send_message_full, so back-pressure and concurrency
                        // caps continue to apply correctly.
                        for d in dispatches {
                            let TriggerDispatch {
                                kernel,
                                aid,
                                msg,
                                mode_override,
                                session_id_override,
                                trigger_sem,
                                agent_sem,
                            } = d;

                            // (1) Global trigger lane permit.
                            let _lane_permit = match trigger_sem.acquire_owned().await {
                                Ok(p) => p,
                                Err(_) => return, // lane closed during shutdown
                            };
                            // (2) Per-agent permit.
                            let _agent_permit = match agent_sem.acquire_owned().await {
                                Ok(p) => p,
                                Err(_) => continue,
                            };
                            // (3) Inner per-session mutex applies inside
                            //     send_message_full when session_id_override is Some.
                            let handle = kernel.kernel_handle();
                            let home_channel = kernel.resolve_agent_home_channel(aid);
                            // Bound permit-hold duration so a stuck LLM
                            // call cannot pin Lane::Trigger kernel-wide.
                            // Note: timeout drops this future on expiry,
                            // but any tokio::spawn'd child tasks inside
                            // send_message_full are NOT cancelled — they
                            // run to completion independently.
                            match tokio::time::timeout(
                                fire_timeout,
                                kernel.send_message_full(
                                    aid,
                                    &msg,
                                    handle,
                                    None,
                                    home_channel.as_ref(),
                                    mode_override,
                                    None,
                                    session_id_override,
                                ),
                            )
                            .await
                            {
                                Ok(Ok(_)) => {}
                                Ok(Err(e)) => {
                                    warn!(agent = %aid, "Trigger dispatch failed: {e}");
                                }
                                Err(_) => {
                                    warn!(
                                        agent = %aid,
                                        timeout_secs = fire_timeout.as_secs(),
                                        "Trigger dispatch timed out; releasing lane permit"
                                    );
                                }
                            }
                        }
                    });
                spawn_logged("trigger_dispatch", task);
            }
        }

        triggered
    }

    /// Register a trigger for an agent.
    pub fn register_trigger(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
    ) -> KernelResult<TriggerId> {
        self.register_trigger_with_target(
            agent_id,
            pattern,
            prompt_template,
            max_fires,
            None,
            None,
            None,
        )
    }

    /// Register a trigger with an optional cross-session target agent.
    ///
    /// When `target_agent` is `Some`, the triggered message is routed to that
    /// agent instead of the owner. Both owner and target must exist.
    #[allow(clippy::too_many_arguments)]
    pub fn register_trigger_with_target(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
        target_agent: Option<AgentId>,
        cooldown_secs: Option<u64>,
        session_mode: Option<librefang_types::agent::SessionMode>,
    ) -> KernelResult<TriggerId> {
        // Verify owner agent exists
        if self.registry.get(agent_id).is_none() {
            return Err(KernelError::LibreFang(LibreFangError::AgentNotFound(
                agent_id.to_string(),
            )));
        }
        // Verify target agent exists (if specified)
        if let Some(target) = target_agent {
            if self.registry.get(target).is_none() {
                return Err(KernelError::LibreFang(LibreFangError::AgentNotFound(
                    target.to_string(),
                )));
            }
        }
        let id = self.workflows.triggers.register_with_target(
            agent_id,
            pattern,
            prompt_template,
            max_fires,
            target_agent,
            cooldown_secs,
            session_mode,
        );
        if let Err(e) = self.workflows.triggers.persist() {
            warn!(trigger_id = %id, "Failed to persist trigger jobs after register: {e}");
        }
        Ok(id)
    }

    /// Remove a trigger by ID.
    pub fn remove_trigger(&self, trigger_id: TriggerId) -> bool {
        let removed = self.workflows.triggers.remove(trigger_id);
        if removed {
            if let Err(e) = self.workflows.triggers.persist() {
                warn!(%trigger_id, "Failed to persist trigger jobs after remove: {e}");
            }
        }
        removed
    }

    /// Enable or disable a trigger. Returns true if found.
    pub fn set_trigger_enabled(&self, trigger_id: TriggerId, enabled: bool) -> bool {
        let found = self.workflows.triggers.set_enabled(trigger_id, enabled);
        if found {
            if let Err(e) = self.workflows.triggers.persist() {
                warn!(%trigger_id, "Failed to persist trigger jobs after set_enabled: {e}");
            }
        }
        found
    }

    /// List all triggers (optionally filtered by agent).
    pub fn list_triggers(&self, agent_id: Option<AgentId>) -> Vec<crate::triggers::Trigger> {
        match agent_id {
            Some(id) => self.workflows.triggers.list_agent_triggers(id),
            None => self.workflows.triggers.list_all(),
        }
    }

    /// Get a single trigger by ID.
    pub fn get_trigger(&self, trigger_id: TriggerId) -> Option<crate::triggers::Trigger> {
        self.workflows.triggers.get_trigger(trigger_id)
    }

    /// Update mutable fields of an existing trigger.
    pub fn update_trigger(
        &self,
        trigger_id: TriggerId,
        patch: crate::triggers::TriggerPatch,
    ) -> Option<crate::triggers::Trigger> {
        let result = self.workflows.triggers.update(trigger_id, patch);
        if result.is_some() {
            if let Err(e) = self.workflows.triggers.persist() {
                warn!(%trigger_id, "Failed to persist trigger jobs after update: {e}");
            }
        }
        result
    }

    /// Register a workflow definition.
    pub async fn register_workflow(&self, workflow: Workflow) -> WorkflowId {
        self.workflows.engine.register(workflow).await
    }

    /// Run a workflow pipeline end-to-end.
    ///
    /// **Naming**: this inherent method takes typed `WorkflowId` /
    /// `WorkflowRunId`. The role-trait
    /// [`kernel_handle::WorkflowRunner::run_workflow`] takes `&str` and
    /// returns `String` shapes for backward compat. From `Arc<dyn KernelApi>`
    /// callers, reach the typed shape via
    /// [`KernelApi::run_workflow_typed`](crate::kernel_api::KernelApi::run_workflow_typed)
    /// rather than going through the trait method.
    pub async fn run_workflow(
        &self,
        workflow_id: WorkflowId,
        input: String,
    ) -> KernelResult<(WorkflowRunId, String)> {
        let cfg = self.config.load_full();
        let run_id = self
            .workflows
            .engine
            .create_run(workflow_id, input)
            .await
            .ok_or_else(|| {
                KernelError::LibreFang(LibreFangError::Internal("Workflow not found".to_string()))
            })?;

        // Agent resolver: looks up by name or ID in the registry.
        // Returns (AgentId, agent_name, inherit_parent_context).
        let resolver = |agent_ref: &StepAgent| -> Option<(AgentId, String, bool)> {
            match agent_ref {
                StepAgent::ById { id } => {
                    let agent_id: AgentId = id.parse().ok()?;
                    let entry = self.registry.get(agent_id)?;
                    let inherit = entry.manifest.inherit_parent_context;
                    Some((agent_id, entry.name.clone(), inherit))
                }
                StepAgent::ByName { name } => {
                    let entry = self.registry.find_by_name(name)?;
                    let inherit = entry.manifest.inherit_parent_context;
                    Some((entry.id, entry.name.clone(), inherit))
                }
            }
        };

        // Message sender: sends to agent and returns (output, in_tokens, out_tokens)
        let send_message = |agent_id: AgentId, message: String| async move {
            self.send_message(agent_id, &message)
                .await
                .map(|r| {
                    (
                        r.response,
                        r.total_usage.input_tokens,
                        r.total_usage.output_tokens,
                    )
                })
                .map_err(|e| format!("{e}"))
        };

        // SECURITY: Global workflow timeout to prevent runaway execution.
        let max_workflow_secs = cfg.triggers.max_workflow_secs;

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(max_workflow_secs),
            self.workflows
                .engine
                .execute_run(run_id, resolver, send_message),
        )
        .await
        .map_err(|_| {
            KernelError::LibreFang(LibreFangError::Internal(format!(
                "Workflow timed out after {max_workflow_secs}s"
            )))
        })?
        .map_err(|e| {
            KernelError::LibreFang(LibreFangError::Internal(format!("Workflow failed: {e}")))
        })?;

        Ok((run_id, output))
    }

    /// Dry-run a workflow: resolve agents and expand prompts without making any LLM calls.
    ///
    /// Returns a per-step preview useful for validating a workflow before running it for real.
    pub async fn dry_run_workflow(
        &self,
        workflow_id: WorkflowId,
        input: String,
    ) -> KernelResult<Vec<DryRunStep>> {
        let resolver =
            |agent_ref: &StepAgent| -> Option<(librefang_types::agent::AgentId, String, bool)> {
                match agent_ref {
                    StepAgent::ById { id } => {
                        let agent_id: librefang_types::agent::AgentId = id.parse().ok()?;
                        let entry = self.registry.get(agent_id)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((agent_id, entry.name.clone(), inherit))
                    }
                    StepAgent::ByName { name } => {
                        let entry = self.registry.find_by_name(name)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((entry.id, entry.name.clone(), inherit))
                    }
                }
            };

        self.workflows
            .engine
            .dry_run(workflow_id, &input, resolver)
            .await
            .map_err(|e| {
                KernelError::LibreFang(LibreFangError::Internal(format!(
                    "Workflow dry-run failed: {e}"
                )))
            })
    }
}
