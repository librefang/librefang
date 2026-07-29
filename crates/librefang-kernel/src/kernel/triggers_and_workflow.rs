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

/// Prompt for the built-in Task Board assignee wake (issue #6728).
///
/// Compiled in rather than configurable: it is the fallback for an installation that declared nothing, so it has to work with no setup at all.
/// An operator who wants different wording writes their own trigger, which then takes precedence.
///
/// The three-claim cap is deliberate and not arbitrary politeness.
/// `task_claim` takes no arguments, so "drain until empty" issues byte-identical calls, and `librefang-runtime::loop_guard` blocks a tool after `block_threshold` (5) identical calls — or sooner, at `outcome_block_threshold` (3) identical results, which an empty board produces immediately.
/// An unbounded drain therefore ends in blocked tool calls rather than an empty queue.
/// Three claims stay clear of both limits, and since every post wakes the assignee again, a deeper backlog still drains — one wake per task, which is the accounting this fix is built on.
const ASSIGNEE_WAKE_PROMPT: &str = "[TASK BOARD] Task {task_id} was assigned to you: \"{title}\".

Claim it with `task_claim`, do the work, then record the outcome with `task_complete(task_id, result)`.

`task_claim` returns the next task assigned to you, or nothing when there is none left.
If it returns nothing, stop immediately — do not call it again.
Claim at most 3 tasks in this activation; anything still queued is picked up on your next wake.";

/// Prompt for the level-triggered reconcile wake (issue #6728).
///
/// Separate from `ASSIGNEE_WAKE_PROMPT` because the situation is different and saying so changes what a model does with it: this fires when work has been sitting rather than when it has just arrived, so it names the backlog instead of a single new task, and it does not imply the agent is seeing the task for the first time.
const RECONCILE_WAKE_PROMPT: &str =
    "[TASK BOARD] {count} task(s) assigned to you are still unclaimed, the oldest being {task_id}.

Claim with `task_claim`, do the work, then record the outcome with `task_complete(task_id, result)`.

`task_claim` returns the next task assigned to you, or nothing when there is none left.
If it returns nothing, stop immediately — do not call it again.
Claim at most 3 tasks in this activation; anything still queued is picked up on your next wake.";

/// Whether `task_claim` actually reaches this agent, deciding whether waking it can accomplish anything.
///
/// Mirrors the three independent filters `kernel::tools_and_skills::available_tools` applies, because a wake is only useful if the tool survives all of them — and each one alone is enough to withhold it:
///
/// 1. `capabilities.tools` — the declared set, and the one that is easy to read backwards.
///    **Empty means "unrestricted — every tool"**, not "no tools": that is the convention `available_tools` spells out as `tools_unrestricted`, and it is what `AgentManifest::default()` produces, since `ManifestCapabilities::tools` is an empty `Vec`.
///    Treating the raw field as a deny-list inverts it for exactly the installations that configured nothing at all — the common case this wake exists to serve.
/// 2. `tool_allowlist` — narrows further when non-empty and can only ever remove (#6609), so an allowlist that omits `task_claim` withholds it even from an otherwise unrestricted agent.
/// 3. `tool_blocklist` — strips unconditionally at Step 4, and is the mechanism this codebase documents for withholding a tool an agent would otherwise have.
///
/// All three are matched with [`glob_matches`] rather than string equality, the same way `available_tools` resolves them, so `task_*` grants here exactly as it grants at dispatch and a blocklist entry of `task_*` withholds just as broadly.
/// A bare `*` is subsumed: `glob_matches` returns `true` for it unconditionally.
fn can_claim_tasks(manifest: &AgentManifest) -> bool {
    use librefang_types::capability::glob_matches;
    const TASK_CLAIM: &str = "task_claim";

    let declared = &manifest.capabilities.tools;
    if !declared.is_empty() && !declared.iter().any(|t| glob_matches(t, TASK_CLAIM)) {
        return false;
    }
    if !manifest.tool_allowlist.is_empty()
        && !manifest
            .tool_allowlist
            .iter()
            .any(|a| glob_matches(a, TASK_CLAIM))
    {
        return false;
    }
    !manifest
        .tool_blocklist
        .iter()
        .any(|b| glob_matches(b, TASK_CLAIM))
}

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
        let catalog = self.llm.model_catalog.load_full();
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

            let echo_policy = catalog
                .find_model(&model)
                .map(|e| e.reasoning_echo_policy)
                .unwrap_or_default();
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
                prompt_cache_strategy: None,
                response_format: None,
                timeout_secs: None,
                extra_body: None,
                agent_id: Some(agent_id.to_string()),
                session_id: Some(session_id.0.to_string()),
                step_id: None,
                reasoning_echo_policy: echo_policy,
                ..Default::default()
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

        let echo_policy = self.lookup_reasoning_echo_policy(model);
        let request = CompletionRequest {
            model: model.to_string(),
            messages: std::sync::Arc::new(vec![Message::user(prompt.to_string())]),
            tools: std::sync::Arc::new(vec![]),
            max_tokens: 50, // enough for YES/NO + brief rationale
            temperature: 0.0,
            system: None,
            thinking: None,
            prompt_caching: false,
            cache_ttl: None,
            prompt_cache_strategy: None,
            response_format: None,
            timeout_secs: None,
            extra_body: None,
            agent_id: None,
            session_id: None,
            step_id: None,
            reasoning_echo_policy: echo_policy,

            ..Default::default()
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
        let (mut triggered, trigger_state_mutated) = self
            .workflows
            .triggers
            .evaluate_with_resolver(&event, |id| {
                self.agents.registry.get(id).map(|e| e.name.clone())
            });
        if !triggered.is_empty() || trigger_state_mutated {
            if let Err(e) = self.workflows.triggers.persist() {
                warn!("Failed to persist trigger jobs after fire: {e}");
            }
        }

        // Built-in Task Board assignee wake (issue #6728). Appended after the
        // stored-trigger pass so the operator's own triggers keep evaluation
        // order and the per-event budget, and so this can only ever add the
        // one match the budget-capped pass structurally cannot produce: a
        // wake for an addressee nobody subscribed on behalf of.
        if let Some(wake) = self.synthesize_assignee_wake(&event) {
            triggered.push(wake);
        }

        // Tell the reconcile ladder that these agents have just been woken
        // about this task, so the floor does not second-guess an activation
        // that is still resolving.
        //
        // Without this the two paths cannot see each other: the event wake is
        // fire-and-forget, the ladder is only written by the reconcile itself,
        // and its first check for an unseen assignee is seeded to fire
        // immediately — so an activation that outlives `pending_grace_secs`
        // (one LLM turn with a few tool calls before it reaches `task_claim`,
        // which 60s does not comfortably cover) earns a second wake telling
        // the agent about work it may have just finished.
        //
        // Stored triggers are stamped too, not just the synthesized wake: an
        // operator's own `TaskPosted` trigger suppresses the synthesized one,
        // so covering only the built-in path would leave exactly the
        // installations that configured a trigger exposed to the double wake.
        if let librefang_types::event::EventPayload::System(
            librefang_types::event::SystemEvent::TaskPosted { task_id, .. },
        ) = &event.payload
        {
            for m in triggered.iter().filter(|m| m.workflow_id.is_none()) {
                self.note_task_wake_dispatched(m.agent_id, task_id);
            }
        }

        // Capture event.timestamp before the bus move — the trigger
        // dispatcher below uses it as the deterministic fire instant for
        // `SessionId::for_trigger_fire`. audit: trigger-new-session-non-deterministic.
        let event_timestamp = event.timestamp;

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
        // We materialize a deterministic `SessionId::for_trigger_fire`
        // override only when the resolved mode is `New`; persistent fires
        // reuse the canonical session and must
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
        self.dispatch_trigger_matches(&triggered, event_timestamp);

        triggered
    }

    /// Dispatch a list of trigger matches to their agents.
    ///
    /// Single point where "a match exists" becomes "an agent runs", shared by both producers: the event-driven pass in [`Self::publish_event_inner`] and the state-driven reconcile in the Task Board sweeper (#6728).
    /// Both must inherit the same concurrency, ordering and timeout guarantees, and the only way to guarantee that is for there to be one implementation.
    ///
    /// `fire_time` is the instant the matches were resolved.
    /// It keys the deterministic `SessionMode::New` session ids, so callers pass the event timestamp (event path) or the tick instant (reconcile path) rather than reading the clock here — the id must be reproducible from a log line.
    fn dispatch_trigger_matches(
        &self,
        matches: &[crate::triggers::TriggerMatch],
        fire_time: chrono::DateTime<chrono::Utc>,
    ) {
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
                /// `Some` for agent-path dispatches; `None` for workflow-path
                /// dispatches where no per-agent semaphore applies.
                agent_sem: Option<Arc<tokio::sync::Semaphore>>,
                /// When set, fire a workflow run instead of send_message_full.
                workflow_id: Option<String>,
                /// What produced this dispatch — a stored trigger, or the
                /// built-in Task Board assignee wake (#6728).
                source: crate::triggers::TriggerMatchSource,
            }

            let mut dispatches: Vec<TriggerDispatch> = Vec::with_capacity(matches.len());
            for trigger_match in matches {
                let kernel = match weak.upgrade() {
                    Some(k) => k,
                    None => continue,
                };
                let aid = trigger_match.agent_id;
                let msg = trigger_match.message.clone();
                let mode_override = trigger_match.session_mode_override;
                let workflow_id = trigger_match.workflow_id.clone();
                let source = trigger_match.source.clone();

                // For workflow-dispatch triggers, skip the agent-registry lookup —
                // the agent_id on the TriggerMatch is the trigger owner and is not
                // the dispatch target. For agent-dispatch triggers, look up the
                // manifest session_mode and skip if the agent has been deleted.
                let (session_id_override, agent_sem) = if workflow_id.is_some() {
                    // Workflow path: per-agent semaphore is acquired per step
                    // inside the `run_workflow::send_message` closure (keyed on
                    // the resolved step target), not here on the trigger owner.
                    // Session is materialized per step by the resolver too.
                    (None, None)
                } else {
                    // Agent path: resolve effective session mode.
                    let manifest_mode = match kernel.agents.registry.get(aid) {
                        Some(entry) => entry.manifest.session_mode,
                        None => continue,
                    };
                    let effective_mode = mode_override.unwrap_or(manifest_mode);
                    // audit: trigger-new-session-non-deterministic
                    // `SessionId::new()` here was a random v4 UUID, which
                    // made "trigger X fired at T" log lines impossible to
                    // correlate to the actual SessionId for diagnostics.
                    // Mirror cron's `for_cron_run` shape: derive a
                    // deterministic v5 UUID from
                    // `(agent, trigger_id, fire_time)` so the SessionId
                    // is reproducible from logs after the fact. `fire_time`
                    // is the canonical fire instant — the same value the
                    // trigger registry stamps into `last_fired_at` (captured
                    // before the event was moved into the bus publish).
                    //
                    // The built-in assignee wake (#6728) has no trigger id to
                    // key on, so it derives from `(agent, task_id, fire_time)`
                    // instead — same property, different stable key.
                    let sid_override = match effective_mode {
                        librefang_types::agent::SessionMode::New => Some(match &source {
                            crate::triggers::TriggerMatchSource::Registered(tid) => {
                                SessionId::for_trigger_fire(aid, tid.0, fire_time)
                            }
                            crate::triggers::TriggerMatchSource::TaskBoardAssigneeWake {
                                task_id,
                            } => SessionId::for_task_wake(aid, task_id, fire_time),
                        }),
                        librefang_types::agent::SessionMode::Persistent => None,
                    };
                    let agent_sem = kernel.agent_concurrency_for(aid);
                    (sid_override, Some(agent_sem))
                };

                let trigger_sem = kernel
                    .workflows
                    .command_queue
                    .semaphore_for_lane(librefang_runtime::command_lane::Lane::Trigger);

                dispatches.push(TriggerDispatch {
                    kernel,
                    aid,
                    msg,
                    mode_override,
                    session_id_override,
                    trigger_sem,
                    agent_sem,
                    workflow_id,
                    source,
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
                                workflow_id,
                                source,
                            } = d;

                            // (1) Global trigger lane permit.
                            let _lane_permit = match trigger_sem.acquire_owned().await {
                                Ok(p) => p,
                                Err(_) => return, // lane closed during shutdown
                            };
                            // (2) Per-agent permit (agent path only; workflow path skips).
                            let _agent_permit = if let Some(sem) = agent_sem {
                                match sem.acquire_owned().await {
                                    Ok(p) => Some(p),
                                    Err(_) => continue,
                                }
                            } else {
                                None
                            };

                            if let Some(ref wid_str) = workflow_id {
                                // Workflow dispatch path: resolve workflow by UUID, then by
                                // name (case-insensitive — matches WorkflowRunner::run_workflow
                                // and start_workflow_async so `daily report` and `Daily Report`
                                // resolve to the same workflow whether the entry point is a
                                // tool call or a trigger).
                                let wid_str = wid_str.clone();
                                let wid_lower = wid_str.to_lowercase();
                                let resolved_id = if let Ok(uuid) = wid_str.parse::<uuid::Uuid>() {
                                    Some(crate::workflow::WorkflowId(uuid))
                                } else {
                                    let workflows = kernel.workflows.engine.list_workflows().await;
                                    workflows
                                        .iter()
                                        .find(|w| w.name.to_lowercase() == wid_lower)
                                        .map(|w| w.id)
                                };
                                match resolved_id {
                                    Some(wf_id) => {
                                        info!(
                                            source = %source,
                                            workflow_id = %wid_str,
                                            "Trigger fired workflow (async)"
                                        );
                                        // Hold the Lane::Trigger permit for the full
                                        // duration of the workflow run, NOT just the
                                        // resolution above. Earlier code released the
                                        // permit as soon as this iteration yielded
                                        // (the permit lived on the loop stack, not in
                                        // the spawn), so N bursty workflow triggers
                                        // produced N concurrent workflow runs that
                                        // escaped the `queue.concurrency.trigger_lane`
                                        // invariant (default 8). The audit doc
                                        // `docs/issues/workflow-path-drops-lane-permit.md`
                                        // calls this out; fix is option 1 (move the
                                        // permit into the spawn). Per-fire
                                        // `fire_timeout` still bounds permit-hold
                                        // duration, so a stuck workflow cannot pin
                                        // Lane::Trigger kernel-wide.
                                        let lane_permit_for_spawn = _lane_permit;
                                        let kernel_for_spawn = std::sync::Arc::clone(&kernel);
                                        let wid_for_spawn = wid_str.clone();
                                        let source_for_spawn = source.clone();
                                        let timeout_for_spawn = fire_timeout;
                                        tokio::spawn(async move {
                                            match tokio::time::timeout(
                                                timeout_for_spawn,
                                                kernel_for_spawn.run_workflow(wf_id, msg),
                                            )
                                            .await
                                            {
                                                Ok(Ok((run_id, _output))) => {
                                                    info!(
                                                        source = %source_for_spawn,
                                                        run_id = %run_id,
                                                        workflow_id = %wid_for_spawn,
                                                        "Trigger workflow run completed"
                                                    );
                                                }
                                                Ok(Err(e)) => {
                                                    warn!(
                                                        source = %source_for_spawn,
                                                        workflow_id = %wid_for_spawn,
                                                        "Trigger workflow run failed: {e}"
                                                    );
                                                }
                                                Err(_) => {
                                                    warn!(
                                                        source = %source_for_spawn,
                                                        workflow_id = %wid_for_spawn,
                                                        timeout_secs = timeout_for_spawn.as_secs(),
                                                        "Trigger workflow run timed out"
                                                    );
                                                }
                                            }
                                            // Lane::Trigger permit is dropped here,
                                            // when the spawned future ends — held for
                                            // the full workflow run, not just for
                                            // resolution. Explicit drop documents
                                            // intent (the `_`-prefixed binding hint
                                            // would otherwise allow Rust to drop it
                                            // immediately).
                                            drop(lane_permit_for_spawn);
                                        });
                                    }
                                    None => {
                                        warn!(
                                            source = %source,
                                            workflow_id = %wid_str,
                                            run_id = "(unresolved)",
                                            "Trigger: workflow not found, skipping dispatch"
                                        );
                                    }
                                }
                            } else {
                                // Agent dispatch path (existing behavior).
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
                        }
                    });
                spawn_logged("trigger_dispatch", task);
            }
        }
    }

    /// Record that `agent_id` has just been woken about `task_id`, so the
    /// reconcile floor gives that activation room before stepping in.
    ///
    /// Writes the same ladder the reconcile keeps, which is what lets the two
    /// paths see each other at all. The rung is *set* to at least one rather
    /// than incremented: a burst of posts is one situation, not N escalating
    /// failures, and incrementing would push the floor exponentially further
    /// out precisely when a backlog makes it most useful. One rung means the
    /// floor waits `pending_grace_secs` doubled — long enough for an ordinary
    /// turn to reach `task_claim`, and still bounded, so a wake that achieved
    /// nothing is followed by a real one rather than by silence.
    ///
    /// `woken_for` records the task so the reconcile's progress check can see
    /// it disappear from `pending` and reset the ladder on the next tick.
    fn note_task_wake_dispatched(&self, agent_id: AgentId, task_id: &str) {
        use crate::kernel::subsystems::governance::AssigneeWakeState;
        use std::collections::BTreeSet;

        let now = chrono::Utc::now();
        let mut state = self
            .governance
            .assignee_wake_state
            .entry(agent_id)
            .or_insert_with(|| AssigneeWakeState {
                last_wake: now,
                ineffective_wakes: 0,
                woken_for: BTreeSet::new(),
            });
        state.last_wake = now;
        state.ineffective_wakes = state.ineffective_wakes.max(1);
        state.woken_for.insert(task_id.to_string());
    }

    /// Level-triggered floor under Task Board delivery (issue #6728): wake the
    /// assignee of any task that is still `pending` past the grace window.
    ///
    /// Called from the task-board sweeper on every tick. Where
    /// [`Self::synthesize_assignee_wake`] reacts to a `TaskPosted` event, this
    /// reacts to task **state**, which is what makes delivery survive a lost
    /// event — a trigger cooldown that discarded it, a daemon restart between
    /// the substrate write and the dispatch, an agent whose turn failed
    /// mid-claim, a trigger deleted while tasks were queued. An edge-triggered
    /// wake can only ever be as reliable as the edge.
    ///
    /// Deliberately does **not** consult
    /// [`TriggerEngine::task_posted_coverage_for`](crate::triggers::TriggerEngine::task_posted_coverage_for).
    /// That check belongs on the event path, where it prevents two wakes in
    /// the same instant. Here it would be a category error: a task still
    /// `pending` past the grace window is evidence that whatever was
    /// configured did not deliver it, whatever the configuration says. State
    /// is the input, not intent.
    ///
    /// Rate limiting is keyed on the **assignee**, not the task, because the
    /// wake prompt is drain-style — one wake covers everything addressed to
    /// that agent, where per-task keying would wake it N times for N items.
    /// Wakes that leave the pending set unchanged back off exponentially to
    /// `wake_backoff_max_secs`, so an agent that cannot make progress does not
    /// burn a turn every tick; any task leaving `pending` resets the ladder.
    /// Returns the wakes it produced, which is the seam the tests assert on:
    /// dispatch itself needs an LLM behind `send_message_full`, while the
    /// decision of *whom* to wake and *which* task to name is the part worth
    /// pinning. The sweeper ignores the value.
    pub(crate) async fn reconcile_pending_task_wakes(&self) -> Vec<crate::triggers::TriggerMatch> {
        /// One assignee's overdue tasks, newest-first until sorted, each paired
        /// with the `created_at` that decides which one the wake names.
        type OverdueTasks = Vec<(chrono::DateTime<chrono::Utc>, String)>;

        use crate::triggers::{TriggerMatch, TriggerMatchSource};
        use std::collections::{BTreeSet, HashMap};

        let cfg = self.config.load();
        let grace_secs = cfg.task_board.pending_grace_secs;
        if grace_secs == 0 {
            return Vec::new();
        }
        let global_wake = cfg.task_board.assignee_wake;
        let backoff_cap = cfg.task_board.wake_backoff_max_secs.max(grace_secs);

        let pending = match self.memory.substrate.task_list(Some("pending")).await {
            Ok(tasks) => tasks,
            Err(e) => {
                warn!(error = %e, "Task board reconcile: failed to list pending tasks");
                return Vec::new();
            }
        };
        if pending.is_empty() {
            // Nothing outstanding — drop any backoff ladders so the next
            // backlog starts from the fast end rather than inheriting a
            // penalty earned by tasks that are long gone.
            self.governance.assignee_wake_state.clear();
            return Vec::new();
        }

        let now = chrono::Utc::now();
        let grace = chrono::Duration::seconds(grace_secs as i64);

        // Group the overdue tasks by resolved assignee. Sorted before
        // dispatch below: `AgentId` is not `Ord`, and hash order would make
        // *which* agents win the trigger lane differ between ticks.
        // Values carry `created_at` alongside the id: task ids are random v4
        // UUIDs (`substrate::task_post`), so ordering them lexicographically
        // says nothing about age, and both the log field and the prompt claim
        // to name the task that has waited longest.
        let mut by_agent: HashMap<AgentId, OverdueTasks> = HashMap::new();
        for task in &pending {
            let Some(task_id) = task["id"].as_str() else {
                continue;
            };
            let assigned_to = task["assigned_to"].as_str().unwrap_or("").trim();
            if assigned_to.is_empty() {
                // Pool task: claimable by anyone, addressed to no one. Waking
                // every capable agent is a policy call this path does not make.
                continue;
            }
            let created_at = task["created_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            match created_at {
                // Inside the grace window the event path owns this task.
                Some(created) if now - created < grace => continue,
                Some(_) => {}
                None => {
                    // An unparsable timestamp must not make a task invisible
                    // to the floor; treat it as overdue and say so once.
                    debug!(
                        %task_id,
                        raw = ?task["created_at"],
                        "Task board reconcile: unparsable created_at, treating task as overdue"
                    );
                }
            }

            let entry = match assigned_to.parse::<AgentId>() {
                Ok(id) => self.agents.registry.get(id),
                Err(_) => self.agents.registry.find_by_name(assigned_to),
            };
            let Some(entry) = entry else {
                continue; // already reported at post time
            };
            if !entry.manifest.assignee_wake.unwrap_or(global_wake) {
                continue;
            }
            if !can_claim_tasks(&entry.manifest) {
                continue; // reported at post time; re-warning every tick would be noise
            }
            // An unparsable stamp sorts oldest: it is already being treated as
            // overdue above, and letting it read as brand new would hide it
            // behind every other task when the wake names one.
            by_agent.entry(entry.id).or_default().push((
                created_at.unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC),
                task_id.to_string(),
            ));
        }

        // Drop ladders for agents with nothing outstanding, so a later backlog
        // is not penalised by an earlier one.
        self.governance
            .assignee_wake_state
            .retain(|agent_id, _| by_agent.contains_key(agent_id));

        let mut ordered: Vec<(AgentId, OverdueTasks)> = by_agent.into_iter().collect();
        ordered.sort_by_key(|(agent_id, _)| agent_id.0);

        let mut matches: Vec<TriggerMatch> = Vec::new();
        for (agent_id, mut overdue) in ordered {
            // Oldest first, ties broken by id so the reported task is stable
            // across ticks when two were posted in the same instant.
            overdue.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            let overdue_ids: BTreeSet<String> = overdue.iter().map(|(_, id)| id.clone()).collect();
            let due = {
                let mut state = self
                    .governance
                    .assignee_wake_state
                    .entry(agent_id)
                    .or_insert_with(
                        || crate::kernel::subsystems::governance::AssigneeWakeState {
                            // Never woken: `last_wake` far enough back that the
                            // first tick past the grace window fires immediately.
                            last_wake: now - grace - chrono::Duration::seconds(1),
                            ineffective_wakes: 0,
                            woken_for: BTreeSet::new(),
                        },
                    );

                // Progress check: anything we previously woke for that is no
                // longer pending means the agent is working, so the ladder
                // resets even if new tasks arrived in the meantime.
                let picked_up = state.woken_for.iter().any(|id| !overdue_ids.contains(id));
                if picked_up {
                    state.ineffective_wakes = 0;
                }

                let backoff = std::cmp::min(
                    grace_secs.saturating_mul(1u64 << state.ineffective_wakes.min(20)),
                    backoff_cap,
                );
                let elapsed = (now - state.last_wake).num_seconds();
                if elapsed < backoff as i64 {
                    continue;
                }

                state.last_wake = now;
                state.ineffective_wakes = state.ineffective_wakes.saturating_add(1);
                state.woken_for = overdue_ids.clone();
                true
            };
            if !due {
                continue;
            }

            // Genuinely the longest-waiting task, now that the list is sorted
            // by `created_at` — it addresses the wake and keys its session id.
            let oldest = overdue
                .first()
                .map(|(_, id)| id.clone())
                .unwrap_or_default();
            warn!(
                agent_id = %agent_id,
                pending = overdue.len(),
                oldest_task = %oldest,
                grace_secs,
                "Task board reconcile: tasks still pending past the grace window — \
                 waking the assignee (an event-driven wake either never happened or \
                 did not result in a claim)"
            );
            matches.push(TriggerMatch {
                agent_id,
                message: RECONCILE_WAKE_PROMPT
                    .replace("{count}", &overdue.len().to_string())
                    .replace("{task_id}", &oldest),
                session_mode_override: None,
                workflow_id: None,
                source: TriggerMatchSource::TaskBoardAssigneeWake { task_id: oldest },
            });
        }

        if !matches.is_empty() {
            self.dispatch_trigger_matches(&matches, now);
        }
        matches
    }

    /// Synthesize the built-in Task Board wake for a `TaskPosted` event whose assignee no stored trigger currently covers (issue #6728).
    ///
    /// Returns a [`TriggerMatch`](crate::triggers::TriggerMatch) that the caller appends to the dispatch list, so the wake inherits the whole dispatch stack — trigger lane, per-agent semaphore, per-fire timeout, sequential ordering within one event, and the `PUBLISH_EVENT_DEPTH` cycle guard — rather than re-implementing any of it.
    /// Nothing is persisted: the wake leaves no record in `trigger_jobs.json` and is invisible to `trigger list`, so disabling the knob fully reverts it.
    ///
    /// `session_mode_override` is left `None` so the assignee's manifest governs.
    /// With the default `Persistent`, `agent_concurrency_for` clamps the per-agent semaphore to 1, which serializes wakes for that agent — a burst of posts queues rather than racing, and each wake drains what it finds.
    ///
    /// Diagnostics are emitted only where delivery actually breaks: an assignee nothing can resolve, a wake switched off with no trigger to take over, an assignee that cannot claim, or a trigger that exists but can no longer fire.
    /// The silent-failure mode this issue reports is exactly the absence of these lines.
    fn synthesize_assignee_wake(&self, event: &Event) -> Option<crate::triggers::TriggerMatch> {
        use crate::triggers::{TaskPostedCoverage, TriggerMatch, TriggerMatchSource};
        use librefang_types::event::{EventPayload, SystemEvent};

        let EventPayload::System(SystemEvent::TaskPosted {
            task_id,
            title,
            assigned_to,
            ..
        }) = &event.payload
        else {
            return None;
        };

        // An unassigned task is a pool task: `substrate::task_claim` matches
        // `assigned_to = ''` for any claimant, so it is claimable — but with
        // no addressee there is nobody in particular to wake, and fanning out
        // to every capable agent is a policy call this path does not make.
        // A pool worker subscribes with its own trigger.
        let assigned_to = assigned_to
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;

        // Resolve the addressee the same way `KernelHandle::task_claim`
        // does — UUID first, then display name — so anything that can be
        // claimed can also be woken.
        let entry = match assigned_to.parse::<AgentId>() {
            Ok(id) => self.agents.registry.get(id),
            Err(_) => self.agents.registry.find_by_name(assigned_to),
        };
        let Some(entry) = entry else {
            warn!(
                task_id = %task_id,
                assigned_to = %assigned_to,
                "Task assigned to an unregistered agent — no trigger can match it and \
                 no claim will find it, so it will stay pending until reassigned"
            );
            return None;
        };
        let assignee_id = entry.id;

        // Precedence: an operator's own trigger owns delivery, including its
        // prompt, cooldown, session mode and workflow routing. Checked
        // declaratively rather than per-event, so a trigger that is merely
        // cooling down is still coverage and never gets doubled up on.
        match self.workflows.triggers.task_posted_coverage_for(
            assignee_id,
            Some(&entry.name),
            |id| self.agents.registry.get(id).map(|e| e.name.clone()),
        ) {
            TaskPostedCoverage::Covered(trigger_id) => {
                debug!(
                    task_id = %task_id,
                    agent_id = %assignee_id,
                    %trigger_id,
                    "Assignee wake stood down — a stored trigger covers this assignee"
                );
                return None;
            }
            TaskPostedCoverage::Dormant(ids) => {
                warn!(
                    task_id = %task_id,
                    agent_id = %assignee_id,
                    triggers = ?ids,
                    "Assignee's task_posted trigger(s) cannot fire (disabled or \
                     max_fires exhausted) — the built-in wake is taking over; set \
                     assignee_wake = false to suppress it instead"
                );
            }
            TaskPostedCoverage::None => {}
        }

        if !entry
            .manifest
            .assignee_wake
            .unwrap_or_else(|| self.config.load().task_board.assignee_wake)
        {
            warn!(
                task_id = %task_id,
                agent_id = %assignee_id,
                "Task assigned to an agent with assignee_wake disabled and no trigger \
                 covering it — nothing will wake it, so the task stays pending until \
                 something else claims it"
            );
            return None;
        }

        // Wake only agents that can act on the task themselves. An
        // installation whose board is drained on an agent's behalf — an
        // external claimer against the HTTP route, or a human triaging by
        // hand — declares a tool list that withholds `task_claim`, and must
        // not start racing its own claimant on upgrade. Withholding is an
        // explicit list without it; declaring nothing grants everything.
        if !can_claim_tasks(&entry.manifest) {
            warn!(
                task_id = %task_id,
                agent_id = %assignee_id,
                "Task assigned to an agent without the task_claim capability — it \
                 cannot claim the task, so nothing will move it out of pending"
            );
            return None;
        }

        Some(TriggerMatch {
            agent_id: assignee_id,
            message: ASSIGNEE_WAKE_PROMPT
                .replace("{task_id}", task_id)
                .replace("{title}", title),
            session_mode_override: None,
            workflow_id: None,
            source: TriggerMatchSource::TaskBoardAssigneeWake {
                task_id: task_id.clone(),
            },
        })
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
            None,
        )
    }

    /// Register a trigger with an optional cross-session target agent.
    ///
    /// When `target_agent` is `Some`, the triggered message is routed to that
    /// agent instead of the owner. Both owner and target must exist.
    ///
    /// When `workflow_id` is `Some`, a matching event fires a workflow run
    /// (resolved by UUID then by name) instead of `send_message_full`.
    /// `prompt_template` is rendered and used as the workflow's initial input.
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
        workflow_id: Option<String>,
    ) -> KernelResult<TriggerId> {
        // Verify owner agent exists
        if self.agents.registry.get(agent_id).is_none() {
            return Err(KernelError::LibreFang(LibreFangError::AgentNotFound(
                agent_id.to_string(),
            )));
        }
        // Verify target agent exists (if specified)
        if let Some(target) = target_agent {
            if self.agents.registry.get(target).is_none() {
                return Err(KernelError::LibreFang(LibreFangError::AgentNotFound(
                    target.to_string(),
                )));
            }
        }
        // Propagate the per-agent cap as InvalidInput rather than
        // silently dropping (audit: trigger-engine-no-per-agent-cap).
        // The route handler will return 400 so the operator sees
        // exactly why the registration failed — same envelope as
        // every other client-error path through this endpoint.
        let id = self
            .workflows
            .triggers
            .register_with_target(
                agent_id,
                pattern,
                prompt_template,
                max_fires,
                target_agent,
                cooldown_secs,
                session_mode,
                workflow_id,
            )
            .map_err(|e| KernelError::LibreFang(LibreFangError::InvalidInput(e.to_string())))?;
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

        // Bound nested workflow runs (refs #6659).
        // The `workflow_run` tool executes the whole target workflow inline on the calling task, and the `send_message` closure below nests a complete agent turn per step.
        // So an agent whose workflow step targets an agent that runs a workflow again recursed with no depth accounting at all — the only bound was the wall-clock `triggers.max_workflow_secs`, by which point the worker thread's stack is already gone.
        //
        // Charge workflow nesting to the same budget inter-agent `agent_send` hops already use (`max_agent_call_depth`) instead of inventing a second counter: `A --agent_send--> B --workflow--> C` stacks turns exactly as `A -> B -> C` does, so one operator knob should cap both.
        //
        // `CapabilityDenied` rather than `Internal`, matching the precedent in `tool_agent_send`: this is a self-imposed kernel-policy quota, so a capped chain records a policy refusal (HTTP 403) on the step instead of an opaque 5xx that would read as a downstream crash to retry logic.
        // Checked before `create_run` so a refusal leaves no orphan `Pending` run behind.
        // `>=` with no lower clamp, matching `tool_agent_send` byte for byte (`tool_runner/agent.rs`).
        // At the legal value `max_agent_call_depth = 0` that refuses a nested `workflow_run` outright, which is the same thing `agent_send` has always done at 0 — the knob means "permit no nesting", and honouring it on only one of the two paths it now governs would defeat the point of sharing it.
        // Before this change `workflow_run` ignored the knob entirely, so an operator running 0 could nest workflows while `agent_send` was blocked; that inconsistency is what goes away here.
        let max_depth = cfg.max_agent_call_depth;
        let current_depth = librefang_runtime::tool_runner::current_agent_depth();
        if current_depth >= max_depth {
            return Err(KernelError::LibreFang(LibreFangError::CapabilityDenied(
                format!(
                    "Nested workflow run depth exceeded (max {max_depth}); this run is already \
                     {current_depth} agent turns deep. Start the workflow asynchronously \
                     (workflow_start) instead of nesting it inside the current turn."
                ),
            )));
        }

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
                    let entry = self.agents.registry.get(agent_id)?;
                    let inherit = entry.manifest.inherit_parent_context;
                    Some((agent_id, entry.name.clone(), inherit))
                }
                StepAgent::ByName { name } => {
                    let entry = self.agents.registry.find_by_name(name)?;
                    let inherit = entry.manifest.inherit_parent_context;
                    Some((entry.id, entry.name.clone(), inherit))
                }
            }
        };

        // Message sender: sends to agent and returns (output, in_tokens, out_tokens).
        //
        // `session_mode_override` carries the per-step `WorkflowStep::session_mode`
        // (#4834). When `Some`, it overrides the target registry agent's
        // manifest `session_mode` for this dispatch — per CLAUDE.md
        // precedence: per-step override > target agent manifest default.
        // Threaded into `send_message_full`'s existing `session_mode_override`
        // slot so workflow-step-driven dispatch reuses the same session-id
        // resolution path as cron and trigger fires.
        //
        // Per-agent semaphore (audit fix for `triggers_and_workflow.rs:334-336`):
        // The trigger-dispatcher path intentionally skips the per-agent
        // semaphore for workflow-id triggers because the actual per-agent
        // LLM call happens here — one acquire per workflow step, keyed on
        // the *step target* (which may differ from the workflow owner). A
        // fan-out layer that targets the same agent N times now serializes
        // through `agent_concurrency_for(agent_id)` instead of bypassing
        // `max_concurrent_invocations`. The permit is held across
        // `send_message_full` and released on drop at the end of this
        // future, exactly as the trigger and cron paths do.
        let send_message =
            |agent_id: AgentId,
             message: String,
             session_mode_override: Option<librefang_types::agent::SessionMode>| async move {
                let sem = self.agent_concurrency_for(agent_id);
                let _agent_permit = match sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        return Err(format!(
                            "agent {agent_id} concurrency semaphore closed during workflow step"
                        ))
                    }
                };
                // Account for the nesting (refs #6659).
                // A workflow step runs a whole agent turn inline on this task, so it belongs one level deeper in the same chain `agent_send` tracks — that is what makes the `max_agent_call_depth` check at the top of `run_workflow` bound a workflow that re-runs itself through a step target's `workflow_run`.
                // The helper also boxes the turn's future, so each nesting level costs a pointer rather than another inlined copy of the agent-loop state machine.
                librefang_runtime::tool_runner::with_agent_call_depth(self.send_message_full(
                    agent_id,
                    &message,
                    self.kernel_handle(),
                    None,
                    None,
                    session_mode_override,
                    None,
                    None,
                ))
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
                        let entry = self.agents.registry.get(agent_id)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((agent_id, entry.name.clone(), inherit))
                    }
                    StepAgent::ByName { name } => {
                        let entry = self.agents.registry.find_by_name(name)?;
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

// ========================================================================
// #4977 step 2 — HITL operator-step kernel bridges.
//
// `WorkflowEngine` is decoupled from the channel adapters / agent registry
// (same reason `execute_run` takes closures). These two thin bridges
// implement the engine-side traits on top of the concrete kernel: the
// notifier reaches `send_channel_message` for #5135; the resume driver
// rebuilds the same resolver/sender closures `run_workflow` uses and
// re-enters `resolve_operator_timeout` for #5134. Both are installed once
// from `set_self_handle` (post-`Arc::new(kernel)`); mirrors the
// `KernelCronBridge` shape.
// ========================================================================

/// Operator-step notification bridge (#5135). Holds a `Weak<LibreFangKernel>`
/// so the engine's `OnceLock`-stored handle does not pin the kernel Arc
/// alive (which would form a self-cycle through `kernel.workflows.engine`
/// and break `Arc::try_unwrap` on shutdown / restart). Send path goes
/// through the kernel's existing `send_channel_message` after `upgrade()`.
struct KernelOperatorBridge {
    kernel: Weak<LibreFangKernel>,
}

#[async_trait::async_trait]
impl crate::workflow::OperatorNotifier for KernelOperatorBridge {
    async fn notify_operator(&self, recipient: &str, message: &str) -> Result<(), String> {
        let Some(kernel) = self.kernel.upgrade() else {
            return Err("operator notify dropped: kernel no longer alive".to_string());
        };
        // `notify` entries are `scheme:target` (e.g. `telegram:@pakman`,
        // `dashboard:`). Split on the FIRST colon: the scheme maps to the
        // channel adapter key, the remainder is the platform recipient.
        // `dashboard:` has an empty target — the dashboard surfaces the
        // pause via the runs API rather than a pushed message, so treat an
        // empty target as a successful no-op (the run is already visible
        // in the Approvals/runs UI).
        let (scheme, target) = match recipient.split_once(':') {
            Some((s, t)) => (s, t),
            None => {
                return Err(format!(
                    "operator notify recipient '{recipient}' is not 'scheme:target'"
                ))
            }
        };
        if scheme == "dashboard" || target.is_empty() {
            // Dashboard / webhook-less surfaces: nothing to push; the
            // pause is already inspectable via the workflow runs API.
            return Ok(());
        }
        use librefang_runtime::kernel_handle::ChannelSender;
        kernel
            .send_channel_message(scheme, target, message, None, None)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Operator-step timeout resume driver (#5134). Held as `Weak<LibreFangKernel>`
/// for the same self-cycle reason as `KernelOperatorBridge`. On wake it
/// rebuilds the same resolver/sender closures `LibreFangKernel::run_workflow`
/// uses and re-enters `WorkflowEngine::resolve_operator_timeout`; if the
/// kernel has been dropped by the time the watchdog fires, the auto-resolve
/// is silently skipped (there is no kernel left to drive).
struct KernelOperatorResumeDriver {
    kernel: Weak<LibreFangKernel>,
}

#[async_trait::async_trait]
impl crate::workflow::OperatorResumeDriver for KernelOperatorResumeDriver {
    async fn drive_operator_timeout(
        &self,
        run_id: WorkflowRunId,
        operator_step_index: usize,
        timeout_action: crate::workflow::OperatorTimeoutAction,
    ) {
        let Some(kernel) = self.kernel.upgrade() else {
            tracing::debug!(
                run_id = %run_id,
                "Operator timeout auto-resolve skipped: kernel dropped"
            );
            return;
        };
        let resolver = {
            let kernel = kernel.clone();
            move |agent_ref: &StepAgent| -> Option<(AgentId, String, bool)> {
                match agent_ref {
                    StepAgent::ById { id } => {
                        let agent_id: AgentId = id.parse().ok()?;
                        let entry = kernel.agents.registry.get(agent_id)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((agent_id, entry.name.clone(), inherit))
                    }
                    StepAgent::ByName { name } => {
                        let entry = kernel.agents.registry.find_by_name(name)?;
                        let inherit = entry.manifest.inherit_parent_context;
                        Some((entry.id, entry.name.clone(), inherit))
                    }
                }
            }
        };
        let send_kernel = kernel.clone();
        let send_message =
            move |agent_id: AgentId,
                  message: String,
                  session_mode_override: Option<librefang_types::agent::SessionMode>| {
                let k = send_kernel.clone();
                async move {
                    // Mirror the per-agent semaphore acquire from
                    // `run_workflow::send_message`: the timeout-driven
                    // resume path also invokes step LLM calls that must
                    // honour `max_concurrent_invocations` keyed on the
                    // resolved target agent.
                    let sem = k.agent_concurrency_for(agent_id);
                    let _agent_permit = match sem.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => {
                            return Err(format!(
                            "agent {agent_id} concurrency semaphore closed during workflow resume"
                        ))
                        }
                    };
                    // Account for the nesting, exactly as `run_workflow::send_message` does (refs #6659).
                    // This closure is the twin of that one — same per-agent semaphore, same whole-agent-turn dispatch — and an operator-timeout resume drives the run's remaining steps through it.
                    // Left unwrapped, those steps ran at depth 0 while every other path charged them at 1, so a timed-out HITL workflow got a budget of one extra stacked agent turn and the boxing that keeps each level to a pointer did not apply.
                    librefang_runtime::tool_runner::with_agent_call_depth(k.send_message_full(
                        agent_id,
                        &message,
                        k.kernel_handle(),
                        None,
                        None,
                        session_mode_override,
                        None,
                        None,
                    ))
                    .await
                    .map(|r| {
                        (
                            r.response,
                            r.total_usage.input_tokens,
                            r.total_usage.output_tokens,
                        )
                    })
                    .map_err(|e| format!("{e}"))
                }
            };
        if let Err(e) = kernel
            .workflows
            .engine
            .resolve_operator_timeout(
                run_id,
                operator_step_index,
                timeout_action,
                resolver,
                send_message,
            )
            .await
        {
            tracing::warn!(
                run_id = %run_id,
                error = %e,
                "Operator timeout auto-resolve failed"
            );
        }
    }
}

impl LibreFangKernel {
    /// Install the operator-step notifier + timeout-resume driver onto the
    /// workflow engine (#4977 step 2). Called once from `set_self_handle`
    /// after the kernel is wrapped in `Arc` — both bridges need an
    /// `Arc<LibreFangKernel>`.
    pub(crate) fn install_operator_hooks(self: &Arc<Self>) {
        let notifier: Arc<dyn crate::workflow::OperatorNotifier> = Arc::new(KernelOperatorBridge {
            kernel: Arc::downgrade(self),
        });
        let driver: Arc<dyn crate::workflow::OperatorResumeDriver> =
            Arc::new(KernelOperatorResumeDriver {
                kernel: Arc::downgrade(self),
            });
        self.workflows.engine.set_operator_hooks(notifier, driver);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod trigger_dispatch_session_id_tests {
    //! audit: trigger-new-session-non-deterministic
    //!
    //! Contract tests pinning the SessionId materialized for `SessionMode::New`
    //! trigger fires. Mirrors `fire_session_override_new_matches_for_cron_run_contract_3657`
    //! in `crates/librefang-kernel/src/cron.rs` so both audit paths fail loudly
    //! if anyone changes the derivation shape (timestamp precision, separator,
    //! ordering, namespace) without updating the corresponding helper.
    //!
    //! These are unit tests on the helper rather than full publish_event paths
    //! because spinning a real LibreFangKernel inside this file would pull in
    //! the full integration harness — the integration suite already covers
    //! the dispatcher end-to-end (`crates/librefang-api/tests/`).
    use chrono::TimeZone;
    use librefang_types::agent::{AgentId, SessionId};

    /// Regression for the audit item: pin the exact session id a `New`-mode
    /// trigger fire receives. If anyone changes the derivation shape without
    /// updating `SessionId::for_trigger_fire`, this test fails loudly. This
    /// pins the helper contract the dispatcher's `New` arm relies on; that the
    /// dispatcher actually calls `for_trigger_fire` (and not the original
    /// random `SessionId::new()`) is exercised end-to-end in
    /// `crates/librefang-api/tests/`.
    #[test]
    fn fire_session_override_new_matches_for_trigger_fire_contract() {
        let agent = AgentId(uuid::Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").unwrap());
        let trigger_id = uuid::Uuid::parse_str("b2b3b4b5-c2c3-d2d3-e2e3-f2f3f4f5f6f7").unwrap();
        let fire_time = chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap();

        let sid = SessionId::for_trigger_fire(agent, trigger_id, fire_time);

        // Determinism: a second call with identical inputs must produce the
        // same SessionId. Random `SessionId::new()` would fail this.
        assert_eq!(
            sid,
            SessionId::for_trigger_fire(agent, trigger_id, fire_time),
            "for_trigger_fire must be deterministic over (agent, trigger_id, fire_time); \
             see docs/issues/trigger-new-session-non-deterministic.md"
        );

        // A different fire_time on the same agent/trigger must NOT collide,
        // even at sub-second resolution — log correlation requires that two
        // burst fires of the same event-triggered job land on distinct ids.
        let later = fire_time + chrono::Duration::nanoseconds(1);
        assert_ne!(
            sid,
            SessionId::for_trigger_fire(agent, trigger_id, later),
            "consecutive fires must yield distinct SessionIds"
        );
    }
}

#[cfg(test)]
mod task_board_reconcile_tests {
    //! Level-triggered floor under Task Board delivery (#6728).
    //!
    //! Exercised against a real booted kernel and a real substrate, because
    //! the whole point of the rule is that it reads task *state* — a mock
    //! would only prove the mock agrees with itself.
    //!
    //! The assertion seam is `governance.assignee_wake_state`: the reconcile
    //! records which tasks it woke an agent for, which proves a wake was
    //! produced without needing an LLM behind the dispatch.
    //!
    //! Built with `boot_with_config` rather than `librefang-testing`'s
    //! builder: that crate dev-depends back on this one, so its
    //! `LibreFangKernel` is a second copy of the type and `pub(crate)` state
    //! is not reachable through it.

    use super::*;
    use librefang_types::agent::{AgentManifest, ManifestCapabilities};

    fn boot(
        label: &str,
        tune: impl FnOnce(&mut KernelConfig),
    ) -> (LibreFangKernel, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join(label);
        std::fs::create_dir_all(&home_dir).unwrap();
        let mut config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            network_enabled: false,
            ..KernelConfig::default()
        };
        tune(&mut config);
        let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boots");
        (kernel, tmp)
    }

    fn worker(name: &str) -> AgentManifest {
        AgentManifest {
            name: name.to_string(),
            description: "task board worker".to_string(),
            author: "test".to_string(),
            module: "builtin:chat".to_string(),
            capabilities: ManifestCapabilities {
                tools: vec!["task_claim".to_string(), "task_complete".to_string()],
                ..ManifestCapabilities::default()
            },
            ..Default::default()
        }
    }

    /// Age a task past the grace window by rewriting the row, so the test does
    /// not sleep for the window it is testing.
    async fn backdate(kernel: &LibreFangKernel, task_id: &str, secs: i64) {
        let stamp = (chrono::Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339();
        let db = kernel.memory.substrate.pool().get().expect("db handle");
        db.execute(
            "UPDATE task_queue SET created_at = ?2 WHERE id = ?1",
            rusqlite::params![task_id, stamp],
        )
        .expect("backdate");
    }

    /// The failure this issue is about: the event was published and lost — to a
    /// trigger cooldown, a restart, a turn that died mid-claim — and nothing
    /// re-announces it. The floor has to pick the task up from state alone,
    /// with no trigger registered anywhere.
    #[tokio::test(flavor = "multi_thread")]
    async fn overdue_pending_task_wakes_its_assignee() {
        let (kernel, _tmp) = boot("reconcile-overdue", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("stranded", "body", Some(&agent.to_string()), Some("boss"))
            .await
            .expect("post");
        backdate(&kernel, &task, 120).await;

        kernel.reconcile_pending_task_wakes().await;

        let state = kernel
            .governance
            .assignee_wake_state
            .get(&agent)
            .expect("the assignee must have been woken");
        assert!(state.woken_for.contains(&task));
    }

    /// A task younger than the grace window belongs to the event path; waking
    /// for it here would double every delegation.
    #[tokio::test(flavor = "multi_thread")]
    async fn task_inside_the_grace_window_is_left_to_the_event_path() {
        let (kernel, _tmp) = boot("reconcile-fresh", |c| {
            c.task_board.pending_grace_secs = 3600;
        });
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        kernel
            .memory
            .substrate
            .task_post("fresh", "body", Some(&agent.to_string()), Some("boss"))
            .await
            .expect("post");

        kernel.reconcile_pending_task_wakes().await;

        assert!(
            kernel.governance.assignee_wake_state.get(&agent).is_none(),
            "a task inside the grace window must not be reconciled"
        );
    }

    /// Ticking over a task nobody claims must not wake the agent every tick —
    /// that turns a delivery guarantee into unbounded spend.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stuck_task_backs_off_instead_of_waking_every_tick() {
        let (kernel, _tmp) = boot("reconcile-backoff", |c| {
            c.task_board.pending_grace_secs = 60;
            c.task_board.wake_backoff_max_secs = 900;
        });
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("stuck", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        backdate(&kernel, &task, 120).await;

        kernel.reconcile_pending_task_wakes().await;
        let first = kernel
            .governance
            .assignee_wake_state
            .get(&agent)
            .map(|s| (s.last_wake, s.ineffective_wakes))
            .expect("first wake");
        assert_eq!(first.1, 1, "the first reconcile wakes and arms the ladder");

        // The next tick is inside the doubled window, so it must not re-wake.
        kernel.reconcile_pending_task_wakes().await;
        let second = kernel
            .governance
            .assignee_wake_state
            .get(&agent)
            .map(|s| (s.last_wake, s.ineffective_wakes))
            .expect("state kept");
        assert_eq!(
            second, first,
            "a tick inside the backoff window must leave the ladder untouched"
        );
    }

    /// An agent that cannot claim is not woken: nothing it could do would move
    /// the task, so the wake is pure cost. The post-time diagnostic already
    /// named this case once, and repeating it every tick would be noise.
    #[tokio::test(flavor = "multi_thread")]
    async fn assignee_without_task_claim_is_not_woken() {
        let (kernel, _tmp) = boot("reconcile-noclaim", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        let mut manifest = worker("no-claim");
        manifest.capabilities.tools = vec!["web_search".to_string()];
        let agent = kernel
            .spawn_agent_inner(manifest, None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("unclaimable", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        backdate(&kernel, &task, 120).await;

        kernel.reconcile_pending_task_wakes().await;

        assert!(kernel.governance.assignee_wake_state.get(&agent).is_none());
    }

    /// The per-agent opt-out is the single suppression mechanism, so it has to
    /// hold here too — otherwise an operator who silenced an agent would find
    /// the sweeper waking it a minute later.
    #[tokio::test(flavor = "multi_thread")]
    async fn per_agent_opt_out_suppresses_the_reconcile() {
        let (kernel, _tmp) = boot("reconcile-optout", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        let mut manifest = worker("opted-out");
        manifest.assignee_wake = Some(false);
        let agent = kernel
            .spawn_agent_inner(manifest, None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("ignored", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        backdate(&kernel, &task, 120).await;

        kernel.reconcile_pending_task_wakes().await;

        assert!(kernel.governance.assignee_wake_state.get(&agent).is_none());
    }

    /// Unassigned tasks are claimable by anyone and addressed to no one, so
    /// there is nobody for the floor to wake.
    #[tokio::test(flavor = "multi_thread")]
    async fn unassigned_tasks_wake_nobody() {
        let (kernel, _tmp) = boot("reconcile-pool", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("pool", "body", None, None)
            .await
            .expect("post");
        backdate(&kernel, &task, 120).await;

        kernel.reconcile_pending_task_wakes().await;

        assert!(kernel.governance.assignee_wake_state.is_empty());
    }

    /// `pending_grace_secs = 0` turns the floor off entirely, leaving delivery
    /// event-driven — the pre-#6728 behaviour, kept reachable on purpose.
    #[tokio::test(flavor = "multi_thread")]
    async fn grace_of_zero_disables_the_reconcile() {
        let (kernel, _tmp) = boot("reconcile-off", |c| {
            c.task_board.pending_grace_secs = 0;
        });
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("stranded", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        backdate(&kernel, &task, 3600).await;

        kernel.reconcile_pending_task_wakes().await;

        assert!(kernel.governance.assignee_wake_state.is_empty());
    }

    /// Multi-tenant safety on the level path, with three agents that could all
    /// claim: a reconcile must wake the addressee and nobody else.
    /// Every other reconcile test runs one agent, so "wake anyone who can
    /// claim" would pass all of them while handing one agent's task to another.
    #[tokio::test(flavor = "multi_thread")]
    async fn reconcile_wakes_only_the_addressed_agent() {
        let (kernel, _tmp) = boot("reconcile-isolation", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        let addressed = kernel
            .spawn_agent_inner(worker("addressed"), None, None, None)
            .expect("spawn");
        let bystander = kernel
            .spawn_agent_inner(worker("bystander"), None, None, None)
            .expect("spawn");
        let third = kernel
            .spawn_agent_inner(worker("third"), None, None, None)
            .expect("spawn");

        let task = kernel
            .memory
            .substrate
            .task_post("addressed only", "body", Some(&addressed.to_string()), None)
            .await
            .expect("post");
        backdate(&kernel, &task, 120).await;

        let matches = kernel.reconcile_pending_task_wakes().await;

        assert_eq!(matches.len(), 1, "exactly one agent may be woken");
        assert_eq!(matches[0].agent_id, addressed);
        assert!(kernel
            .governance
            .assignee_wake_state
            .get(&bystander)
            .is_none());
        assert!(kernel.governance.assignee_wake_state.get(&third).is_none());
    }

    /// The wake names the task that has actually waited longest.
    ///
    /// Task ids are random v4 UUIDs, so ordering them as strings says nothing
    /// about age — a set-order pick would name an arbitrary task while the log
    /// field and the prompt both call it "the oldest". For an issue whose whole
    /// thesis is that a silent failure went unnoticed, a diagnostic that
    /// asserts something untrue is worth a test of its own.
    #[tokio::test(flavor = "multi_thread")]
    async fn reconcile_names_the_longest_waiting_task() {
        let (kernel, _tmp) = boot("reconcile-oldest", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");

        let newer = kernel
            .memory
            .substrate
            .task_post("newer", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        let older = kernel
            .memory
            .substrate
            .task_post("older", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        // Force age and id order to disagree, rather than hoping two random
        // UUIDs happen to. The lexicographically *smaller* id is made the
        // younger task, so a set-order pick names it and the assertions below
        // fail; only a `created_at`-ordered pick names the other one.
        let (younger, older) = if newer < older {
            (newer.clone(), older.clone())
        } else {
            (older.clone(), newer.clone())
        };
        backdate(&kernel, &younger, 120).await;
        backdate(&kernel, &older, 6000).await;

        let matches = kernel.reconcile_pending_task_wakes().await;

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].source,
            crate::triggers::TriggerMatchSource::TaskBoardAssigneeWake {
                task_id: older.clone()
            },
            "the wake must be keyed on the longest-waiting task, not the smallest id"
        );
        assert!(
            matches[0].message.contains(&older) && !matches[0].message.contains(&younger),
            "the prompt must name the oldest task: {}",
            matches[0].message
        );
        assert!(
            matches[0].message.contains('2'),
            "the prompt must report the full backlog count: {}",
            matches[0].message
        );
    }

    /// The two paths must be able to see each other: an activation started by
    /// the event path is given room before the floor concludes nothing
    /// happened.
    ///
    /// Without the ladder stamp the floor fires at the first tick past
    /// `pending_grace_secs`, so a turn that takes longer than 60s to reach
    /// `task_claim` — one LLM call with a few tool calls ahead of it — earns a
    /// second activation telling the agent about work it may have just
    /// finished.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_fresh_event_wake_defers_the_floor() {
        let (kernel, _tmp) = boot("reconcile-defer", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        // No `set_self_handle`: dispatch stays inert, so these tests exercise
        // the wake decision without an LLM behind `send_message_full`. The
        // ladder stamp happens before dispatch, which is the part under test.
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("in flight", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");

        // The event path wakes the agent and records it.
        let woken = kernel
            .publish_event(
                super::super::super::triggers::tests_support_task_posted_event(
                    &task,
                    &agent.to_string(),
                ),
            )
            .await;
        assert_eq!(woken.len(), 1, "the event path must wake the assignee");

        // The task ages past the grace window while that activation is still
        // resolving.
        backdate(&kernel, &task, 120).await;
        let reconciled = kernel.reconcile_pending_task_wakes().await;

        assert!(
            reconciled.is_empty(),
            "the floor must not wake an agent it just woke through the event path"
        );
    }

    /// ...but the deferral is a delay, not an amnesty. Once the doubled window
    /// passes with the task still pending, the floor fires — which is the whole
    /// point of having a floor when an activation dies silently.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_floor_still_fires_once_the_deferral_expires() {
        let (kernel, _tmp) = boot("reconcile-defer-expiry", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        // No `set_self_handle`: dispatch stays inert, so these tests exercise
        // the wake decision without an LLM behind `send_message_full`. The
        // ladder stamp happens before dispatch, which is the part under test.
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("never claimed", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        kernel
            .publish_event(
                super::super::super::triggers::tests_support_task_posted_event(
                    &task,
                    &agent.to_string(),
                ),
            )
            .await;
        backdate(&kernel, &task, 600).await;

        // Age the recorded wake past `pending_grace_secs` doubled.
        if let Some(mut state) = kernel.governance.assignee_wake_state.get_mut(&agent) {
            state.last_wake = chrono::Utc::now() - chrono::Duration::seconds(600);
        }

        let reconciled = kernel.reconcile_pending_task_wakes().await;

        assert_eq!(
            reconciled.len(),
            1,
            "an activation that achieved nothing must still be followed by the floor"
        );
        assert_eq!(reconciled[0].agent_id, agent);
    }

    /// The deferral has to outlast one `pending_grace_secs`, which is the
    /// scenario that motivated it: an activation that takes longer than 60s to
    /// reach `task_claim`. Recording the wake without advancing the rung would
    /// leave the floor firing exactly one grace window later — still inside the
    /// turn it is meant to be waiting for.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_deferral_outlasts_a_single_grace_window() {
        let (kernel, _tmp) = boot("reconcile-defer-rung", |c| {
            c.task_board.pending_grace_secs = 60;
        });
        let agent = kernel
            .spawn_agent_inner(worker("worker"), None, None, None)
            .expect("spawn");
        let task = kernel
            .memory
            .substrate
            .task_post("slow turn", "body", Some(&agent.to_string()), None)
            .await
            .expect("post");
        kernel
            .publish_event(
                super::super::super::triggers::tests_support_task_posted_event(
                    &task,
                    &agent.to_string(),
                ),
            )
            .await;
        backdate(&kernel, &task, 600).await;

        // 90s after the wake: past one grace window, inside the doubled one.
        if let Some(mut state) = kernel.governance.assignee_wake_state.get_mut(&agent) {
            state.last_wake = chrono::Utc::now() - chrono::Duration::seconds(90);
        }

        assert!(
            kernel.reconcile_pending_task_wakes().await.is_empty(),
            "90s after an event wake the activation may still be running; the floor waits"
        );
    }
}
