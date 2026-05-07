//! Cron scheduler tick loop — extracted from kernel/mod.rs.
//!
//! Fires due cron jobs every 15 seconds, dispatching `SystemEvent` /
//! `AgentTurn` / `Workflow` actions via the shared `cron_lane` semaphore.
//! Lifted out of an inline `spawn_logged("cron_scheduler", async move { … })`
//! so the body — historically the longest closure in mod.rs and the
//! landing zone for #4683 et al. — can be edited and reviewed in
//! isolation. Behaviour-preserving: the body is moved byte-for-byte;
//! only the outer wrapper changed (closure → free `pub(super) async fn`).

use std::sync::Arc;

use librefang_channels::types::SenderContext;
use librefang_types::agent::{AgentId, AgentState, SessionId, SessionMode};
use librefang_types::event::{Event, EventPayload, EventTarget};

use tracing::{debug, warn};

use super::cron_bridge::{cron_deliver_response, cron_fan_out_targets};
use super::cron_script::cron_script_wake_gate;
use super::{
    resolve_cron_max_messages, resolve_cron_max_tokens, resolve_cron_warn_threshold, spawn_logged,
    LibreFangKernel, SYSTEM_CHANNEL_CRON,
};

/// Drive the cron scheduler tick loop until the kernel begins shutdown.
///
/// Captured state (formerly closure captures): `kernel: Arc<Self>`. The
/// per-tick `cron_sem` and per-job `kernel_job` clones are still
/// constructed inside the loop body, unchanged.
pub(super) async fn run_cron_scheduler_loop(kernel: Arc<LibreFangKernel>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
    // Use Skip to avoid burst-firing after a long job blocks the loop.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut persist_counter = 0u32;
    interval.tick().await; // Skip first immediate tick
    loop {
        interval.tick().await;
        if kernel.supervisor.is_shutting_down() {
            // Persist on shutdown
            let _ = kernel.cron_scheduler.persist();
            break;
        }

        let due = kernel.cron_scheduler.due_jobs();
        // Snapshot the cron_lane semaphore once per tick so we
        // can move an Arc clone into each spawned job task (#3738).
        let cron_sem = kernel
            .command_queue
            .semaphore_for_lane(librefang_runtime::command_lane::Lane::Cron);
        for job in due {
            let job_id = job.id;
            let agent_id = job.agent_id;
            let job_name = job.name.clone();

            match &job.action {
                librefang_types::scheduler::CronAction::SystemEvent { text } => {
                    tracing::debug!(job = %job_name, "Cron: firing system event");
                    let payload_bytes = serde_json::to_vec(&serde_json::json!({
                        "type": format!("cron.{}", job_name),
                        "text": text,
                        "job_id": job_id.to_string(),
                    }))
                    .unwrap_or_default();
                    let event = Event::new(
                        AgentId::new(), // system-originated
                        EventTarget::Broadcast,
                        EventPayload::Custom(payload_bytes),
                    );
                    kernel.publish_event(event).await;
                    kernel.cron_scheduler.record_success(job_id);
                }
                librefang_types::scheduler::CronAction::AgentTurn {
                    message,
                    timeout_secs,
                    pre_check_script,
                    ..
                } => {
                    tracing::debug!(job = %job_name, agent = %agent_id, "Cron: firing agent turn");

                    // Bug #3839: skip cron fires for Suspended agents.
                    // Check agent state before running pre_check_script or
                    // dispatching any message — a Suspended agent cannot run,
                    // and recording success here would be misleading.
                    let is_suspended = kernel
                        .registry
                        .get(agent_id)
                        .map(|e| e.state == AgentState::Suspended)
                        .unwrap_or(false);
                    if is_suspended {
                        warn!(
                            job = %job_name,
                            agent = %agent_id,
                            "Cron: agent is Suspended, skipping fire"
                        );
                        kernel.cron_scheduler.record_skipped(job_id);
                        continue;
                    }

                    // Wake-gate: run pre_check_script and check for
                    // {"wakeAgent": false} in the last non-empty output line.
                    // Only fires when the script exits successfully.
                    if let Some(script_path) = pre_check_script {
                        // Resolve the agent workspace so cron_script_wake_gate
                        // can restrict the child's cwd to the agent's own directory.
                        let agent_ws = kernel
                            .registry
                            .get(agent_id)
                            .and_then(|e| e.manifest.workspace.clone());
                        if !cron_script_wake_gate(&job_name, script_path, agent_ws.as_deref()).await
                        {
                            tracing::info!(
                                job = %job_name,
                                "cron: script gate wakeAgent=false, skipping agent"
                            );
                            kernel.cron_scheduler.record_success(job_id);
                            continue;
                        }
                    }

                    let timeout_s = timeout_secs.unwrap_or(120);
                    let timeout = std::time::Duration::from_secs(timeout_s);
                    let delivery = job.delivery.clone();
                    let delivery_targets = job.delivery_targets.clone();
                    let kh: std::sync::Arc<dyn librefang_runtime::kernel_handle::KernelHandle> =
                        kernel.clone();
                    // Cron jobs synthesize their SenderContext locally
                    // so memory/peer lookups still see channel="cron".
                    //
                    // Session resolution by `job.session_mode`:
                    //   * None / Some(Persistent) — all fires share
                    //     the agent's `(agent, channel="cron")`
                    //     persistent session (historical default).
                    //   * Some(New) — each fire receives a fresh
                    //     deterministic session via
                    //     `SessionId::for_cron_run(agent, run_key)`.
                    //     We pass it as `session_id_override` (rather
                    //     than relying on `session_mode_override`
                    //     alone) because the channel-derived branch
                    //     in `send_message_full` would otherwise
                    //     win over the mode override and route
                    //     every fire back to the persistent
                    //     `(agent, "cron")` session — see
                    //     CLAUDE.md note on cron + session_mode.
                    //
                    // Resolution order (#3597): per-job override >
                    // agent manifest default > historical persistent.
                    // When the job has no per-job `session_mode` set
                    // (`None`), we fall back to the agent manifest's
                    // `session_mode` so that agents with
                    // `session_mode = "new"` in agent.toml get
                    // per-fire isolation for cron jobs as well.
                    // Snapshot the manifest's declared session_mode
                    // separately so the trace below can show what
                    // the agent.toml actually asked for, in
                    // addition to the per-job override.
                    let manifest_session_mode = kernel
                        .registry
                        .get(agent_id)
                        .map(|entry| entry.manifest.session_mode);
                    let effective_session_mode = job.session_mode.or(manifest_session_mode);
                    let wants_new_session = effective_session_mode == Some(SessionMode::New);
                    // #3692: emit a structured event recording how
                    // the cron fire's session id was resolved, so
                    // operators can grep logs to confirm whether
                    // their `session_mode = "new"` (per-job or
                    // manifest) was honored — or silently ignored
                    // because neither path set it.
                    let resolution_source = if job.session_mode.is_some() {
                        "cron-job-override"
                    } else if manifest_session_mode == Some(SessionMode::New) {
                        "cron-manifest-fallback"
                    } else {
                        "cron-default-persistent"
                    };
                    debug!(
                        agent_id = %agent_id,
                        job = %job_name,
                        resolution_source = resolution_source,
                        job_session_mode = ?job.session_mode,
                        manifest_session_mode = ?manifest_session_mode,
                        effective_session_mode = ?effective_session_mode,
                        "cron session_mode resolved"
                    );
                    let cron_sender = SenderContext {
                        channel: SYSTEM_CHANNEL_CRON.to_string(),
                        user_id: job.peer_id.clone().unwrap_or_default(),
                        display_name: SYSTEM_CHANNEL_CRON.to_string(),
                        is_group: false,
                        was_mentioned: false,
                        thread_id: None,
                        account_id: None,
                        is_internal_cron: true,
                        ..Default::default()
                    };
                    let sender_ctx_owned = Some(cron_sender);
                    let (mode_override, fire_session_override) =
                        crate::cron::cron_fire_session_override(
                            agent_id,
                            effective_session_mode,
                            job.id,
                            chrono::Utc::now(),
                        );
                    let message_owned = message.clone();

                    // Spawn each AgentTurn job concurrently, bounded
                    // by the `cron_lane` semaphore (#3738).  We
                    // acquire the permit INSIDE the spawn so a
                    // saturated lane queues spawned tasks rather
                    // than blocking the tick loop — the previous
                    // design awaited the permit here and stalled
                    // the entire `for job in due` dispatch behind
                    // any single slow fire.
                    let cron_sem_for_job = cron_sem.clone();
                    let kernel_job = kernel.clone();
                    // Shadow so outer `job_name` survives the move
                    // for the post-arm per-job persist warn.
                    let job_name = job_name.clone();
                    spawn_logged("cron_agent_turn", async move {
                        // Acquire the lane permit before any work
                        // so concurrent fires are still capped.
                        let _permit = match cron_sem_for_job.acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::error!(
                                    job = %job_name,
                                    "Cron lane semaphore closed; skipping fire"
                                );
                                return;
                            }
                        };

                        // Prune the persistent cron session before firing
                        // if the user has configured a size cap, and emit
                        // a tracing::warn! when the post-prune size is
                        // approaching the provider context window (#3693).
                        if !wants_new_session {
                            let cfg_snap = kernel_job.config.load();
                            let max_tokens_raw = cfg_snap.cron_session_max_tokens;
                            let max_messages_raw = cfg_snap.cron_session_max_messages;
                            let warn_fraction = cfg_snap.cron_session_warn_fraction;
                            let warn_fallback = cfg_snap.cron_session_warn_total_tokens;
                            drop(cfg_snap);
                            let max_messages = resolve_cron_max_messages(max_messages_raw);
                            let max_tokens = resolve_cron_max_tokens(max_tokens_raw);
                            let warn_threshold = resolve_cron_warn_threshold(
                                max_tokens,
                                warn_fallback,
                                warn_fraction,
                            );
                            if max_tokens.is_some()
                                || max_messages.is_some()
                                || warn_threshold.is_some()
                            {
                                let cron_sid = SessionId::for_channel(agent_id, "cron");
                                // #3443: serialize prune through the
                                // per-session mutex so two cron fires
                                // for the same agent cannot both
                                // read-modify-write and clobber each
                                // other's keep-set.  The lock is
                                // dropped before send_message_full
                                // (which uses agent_msg_locks for
                                // persistent cron sessions).
                                let prune_lock = kernel_job
                                    .session_msg_locks
                                    .entry(cron_sid)
                                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                                    .clone();
                                let _prune_guard = prune_lock.lock().await;
                                if let Ok(Some(mut session)) =
                                    kernel_job.memory.get_session(cron_sid)
                                {
                                    use librefang_runtime::compactor::estimate_token_count;
                                    let mut mutated = false;
                                    if let Some(max_msgs) = max_messages {
                                        if session.messages.len() > max_msgs {
                                            let excess = session.messages.len() - max_msgs;
                                            session.messages.drain(0..excess);
                                            session.mark_messages_mutated();
                                            mutated = true;
                                        }
                                    }
                                    if let Some(max_tok) = max_tokens {
                                        loop {
                                            let est =
                                                estimate_token_count(&session.messages, None, None);
                                            if est <= max_tok as usize
                                                || session.messages.is_empty()
                                            {
                                                break;
                                            }
                                            session.messages.remove(0);
                                            session.mark_messages_mutated();
                                            mutated = true;
                                        }
                                    }
                                    // Post-prune approach-warn (#3693):
                                    // estimate once after any drains so
                                    // operators see the trend before the
                                    // provider returns 400. Estimate
                                    // omits system_prompt / tools — those
                                    // are added inside send_message_full
                                    // — which slightly under-counts; the
                                    // warn is intentionally conservative.
                                    if let Some(threshold) = warn_threshold {
                                        let estimated =
                                            estimate_token_count(&session.messages, None, None)
                                                as u64;
                                        if estimated >= threshold {
                                            let budget = max_tokens.or(warn_fallback);
                                            tracing::warn!(
                                                agent_id = %agent_id,
                                                session_id = %cron_sid,
                                                job = %job_name,
                                                tokens = estimated,
                                                threshold = threshold,
                                                budget = ?budget,
                                                messages = session.messages.len(),
                                                "cron session approaching context budget — \
                                                 consider lowering cron_session_max_tokens, \
                                                 enabling cron_session_max_messages, or \
                                                 setting session_mode = \"new\" on this job"
                                            );
                                        }
                                    }
                                    if mutated {
                                        let _ =
                                            kernel_job.memory.save_session_async(&session).await;
                                    }
                                }
                            }
                        }

                        let sender_ctx = sender_ctx_owned.as_ref();
                        match tokio::time::timeout(
                            timeout,
                            kernel_job.send_message_full(
                                agent_id,
                                &message_owned,
                                kh,
                                None,
                                sender_ctx,
                                mode_override,
                                None,
                                fire_session_override,
                            ),
                        )
                        .await
                        {
                            Ok(Ok(result)) => {
                                tracing::info!(job = %job_name, "Cron job completed successfully");
                                kernel_job.cron_scheduler.record_success(job_id);
                                // Persist last_run before delivery
                                // so a slow/failed channel push
                                // can't strand last_run on disk.
                                if let Err(e) = kernel_job.cron_scheduler.persist() {
                                    tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                }
                                // Deliver response to configured channel (skip NO_REPLY/silent)
                                if !result.silent {
                                    cron_deliver_response(
                                        &kernel_job,
                                        agent_id,
                                        &result.response,
                                        &delivery,
                                    )
                                    .await;
                                    // Fan out to multi-destination
                                    // delivery_targets (best-effort,
                                    // failure-isolated).
                                    cron_fan_out_targets(
                                        &kernel_job,
                                        &job_name,
                                        &result.response,
                                        &delivery_targets,
                                    )
                                    .await;
                                }
                            }
                            Ok(Err(e)) => {
                                let err_msg = format!("{e}");
                                tracing::warn!(job = %job_name, error = %err_msg, "Cron job failed");
                                kernel_job.cron_scheduler.record_failure(job_id, &err_msg);
                                if let Err(e) = kernel_job.cron_scheduler.persist() {
                                    tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                }
                            }
                            Err(_) => {
                                tracing::warn!(job = %job_name, timeout_s, "Cron job timed out");
                                kernel_job.cron_scheduler.record_failure(
                                    job_id,
                                    &format!("timed out after {timeout_s}s"),
                                );
                                if let Err(e) = kernel_job.cron_scheduler.persist() {
                                    tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                }
                            }
                        }
                    }); // end tokio::spawn for AgentTurn
                }
                librefang_types::scheduler::CronAction::Workflow {
                    workflow_id,
                    input,
                    timeout_secs,
                } => {
                    tracing::debug!(job = %job_name, workflow = %workflow_id, "Cron: firing workflow");
                    let input_text = input.clone().unwrap_or_default();
                    let delivery = job.delivery.clone();
                    let delivery_targets = job.delivery_targets.clone();
                    let timeout_s = timeout_secs.unwrap_or(300);
                    let timeout = std::time::Duration::from_secs(timeout_s);
                    let workflow_id_owned = workflow_id.clone();

                    // Spawn the workflow fire so a long-running
                    // workflow does not block the cron tick loop
                    // (#3738). Concurrency is capped by the
                    // shared cron_lane semaphore acquired inside
                    // the spawned task.
                    let cron_sem_for_job = cron_sem.clone();
                    let kernel_job = kernel.clone();
                    let job_name = job_name.clone();
                    tokio::spawn(async move {
                        let _permit = match cron_sem_for_job.acquire_owned().await {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::error!(
                                    job = %job_name,
                                    "Cron lane semaphore closed; skipping workflow fire"
                                );
                                return;
                            }
                        };

                        // Resolve workflow by UUID first, then by name
                        let resolved_id =
                            if let Ok(uuid) = uuid::Uuid::parse_str(&workflow_id_owned) {
                                Some(crate::workflow::WorkflowId(uuid))
                            } else {
                                // Search by name
                                let workflows = kernel_job.workflows.list_workflows().await;
                                workflows
                                    .iter()
                                    .find(|w| w.name == workflow_id_owned)
                                    .map(|w| w.id)
                            };

                        match resolved_id {
                            Some(wf_id) => {
                                match tokio::time::timeout(
                                    timeout,
                                    kernel_job.run_workflow(wf_id, input_text),
                                )
                                .await
                                {
                                    Ok(Ok((_run_id, output))) => {
                                        tracing::info!(job = %job_name, "Cron workflow completed successfully");
                                        kernel_job.cron_scheduler.record_success(job_id);
                                        if let Err(e) = kernel_job.cron_scheduler.persist() {
                                            tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                        }
                                        cron_deliver_response(
                                            &kernel_job,
                                            agent_id,
                                            &output,
                                            &delivery,
                                        )
                                        .await;
                                        cron_fan_out_targets(
                                            &kernel_job,
                                            &job_name,
                                            &output,
                                            &delivery_targets,
                                        )
                                        .await;
                                    }
                                    Ok(Err(e)) => {
                                        let err_msg = format!("{e}");
                                        tracing::warn!(job = %job_name, error = %err_msg, "Cron workflow failed");
                                        kernel_job.cron_scheduler.record_failure(job_id, &err_msg);
                                        if let Err(e) = kernel_job.cron_scheduler.persist() {
                                            tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                        }
                                    }
                                    Err(_) => {
                                        tracing::warn!(job = %job_name, timeout_s, "Cron workflow timed out");
                                        kernel_job.cron_scheduler.record_failure(
                                            job_id,
                                            &format!("workflow timed out after {timeout_s}s"),
                                        );
                                        if let Err(e) = kernel_job.cron_scheduler.persist() {
                                            tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                        }
                                    }
                                }
                            }
                            None => {
                                let err_msg = format!("workflow not found: {workflow_id_owned}");
                                tracing::warn!(job = %job_name, error = %err_msg, "Cron workflow lookup failed");
                                kernel_job.cron_scheduler.record_failure(job_id, &err_msg);
                                if let Err(e) = kernel_job.cron_scheduler.persist() {
                                    tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                }
                            }
                        }
                    });
                }
            }
        }

        // Periodic persist as a safety net (every ~5 minutes / 20 ticks * 15s)
        persist_counter += 1;
        if persist_counter >= 20 {
            persist_counter = 0;
            if let Err(e) = kernel.cron_scheduler.persist() {
                tracing::warn!("Cron persist failed: {e}");
            }
        }
    }
}
