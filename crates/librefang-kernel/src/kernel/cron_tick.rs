//! Cron scheduler tick loop — extracted from kernel/mod.rs.
//!
//! Fires due cron jobs every 15 seconds, dispatching `SystemEvent` /
//! `AgentTurn` / `Workflow` actions via the shared `cron_lane` semaphore.
//! Lifted out of an inline `spawn_logged("cron_scheduler", async move { … })`
//! so the body — historically the longest closure in mod.rs and the
//! landing zone for #4683 et al. — can be edited and reviewed in
//! isolation. Behaviour-preserving (vs. origin/main): the body is moved
//! byte-for-byte; only the outer wrapper changed (closure → free
//! `pub(super) async fn`). Includes the #4683 SummarizeTrim path.

use std::sync::Arc;

use librefang_channels::types::SenderContext;
use librefang_types::agent::{AgentId, AgentState, SessionId};
use librefang_types::event::{Event, EventPayload, EventTarget};

use tracing::{debug, warn};

use super::cron_bridge::{cron_deliver_response, cron_fan_out_targets};
use super::cron_compaction::{
    apply_cron_prune, cron_clamp_keep_recent, cron_compute_keep_count,
    cron_resolve_compaction_mode, try_summarize_trim,
};
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
        if kernel.agents.supervisor.is_shutting_down() {
            // Persist on shutdown
            let _ = kernel.workflows.cron_scheduler.persist();
            break;
        }

        let due = kernel.workflows.cron_scheduler.due_jobs();
        // Snapshot the cron_lane semaphore once per tick so we
        // can move an Arc clone into each spawned job task (#3738).
        let cron_sem = kernel
            .workflows
            .command_queue
            .semaphore_for_lane(librefang_runtime::command_lane::Lane::Cron);
        for job in due {
            let job_id = job.id;
            let agent_id = job.agent_id;
            let job_name = job.name.clone();

            match &job.action {
                librefang_types::scheduler::CronAction::SystemEvent { text } => {
                    tracing::debug!(job = %job_name, "Cron: firing system event");
                    let payload_bytes = match serde_json::to_vec(&serde_json::json!({
                        "type": format!("cron.{}", job_name),
                        "text": text,
                        "job_id": job_id.to_string(),
                    })) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            // Publishing an empty payload here would emit an
                            // event subscribers can't decode and the cron
                            // fire would look "successful" — record the
                            // failure and skip this job's tick instead
                            // (#5137).
                            tracing::error!(
                                job = %job_name,
                                job_id = %job_id,
                                error = %e,
                                "Cron: failed to encode system event payload; skipping fire"
                            );
                            kernel
                                .workflows
                                .cron_scheduler
                                .record_failure(job_id, &format!("payload encode failed: {e}"));
                            continue;
                        }
                    };
                    let event = Event::new(
                        AgentId::new(), // system-originated
                        EventTarget::Broadcast,
                        EventPayload::Custom(payload_bytes),
                    );
                    kernel.publish_event(event).await;
                    kernel.workflows.cron_scheduler.record_success(job_id);
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
                        .agents
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
                        kernel.workflows.cron_scheduler.record_skipped(job_id);
                        continue;
                    }

                    // Wake-gate: run pre_check_script and check for
                    // {"wakeAgent": false} in the last non-empty output line.
                    // Only fires when the script exits successfully.
                    if let Some(script_path) = pre_check_script {
                        // Resolve the agent workspace so cron_script_wake_gate
                        // can restrict the child's cwd to the agent's own directory.
                        let agent_ws = kernel
                            .agents
                            .registry
                            .get(agent_id)
                            .and_then(|e| e.manifest.workspace.clone());
                        if !cron_script_wake_gate(&job_name, script_path, agent_ws.as_deref()).await
                        {
                            tracing::info!(
                                job = %job_name,
                                "cron: script gate wakeAgent=false, skipping agent"
                            );
                            kernel.workflows.cron_scheduler.record_success(job_id);
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
                        .agents
                        .registry
                        .get(agent_id)
                        .map(|entry| entry.manifest.session_mode);
                    let effective_session_mode = job.session_mode.or(manifest_session_mode);
                    let wants_new_session =
                        effective_session_mode == Some(librefang_types::agent::SessionMode::New);
                    // #3692: emit a structured event recording how
                    // the cron fire's session id was resolved, so
                    // operators can grep logs to confirm whether
                    // their `session_mode = "new"` (per-job or
                    // manifest) was honored — or silently ignored
                    // because neither path set it.
                    let resolution_source = if job.session_mode.is_some() {
                        "cron-job-override"
                    } else if manifest_session_mode
                        == Some(librefang_types::agent::SessionMode::New)
                    {
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
                        is_internal_system: true,
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

                        // Prune (or summarize-and-trim) the persistent cron
                        // session before firing if the user has configured a
                        // size cap, and emit a tracing::warn! when the
                        // post-compaction size is approaching the provider
                        // context window (#3693).
                        if !wants_new_session {
                            let cfg_snap = kernel_job.config.load();
                            let max_tokens_raw = cfg_snap.cron_session_max_tokens;
                            let max_messages_raw = cfg_snap.cron_session_max_messages;
                            let warn_fraction = cfg_snap.cron_session_warn_fraction;
                            let warn_fallback = cfg_snap.cron_session_warn_total_tokens;
                            let compaction_mode = cfg_snap.cron_session_compaction_mode;
                            let keep_recent_cfg =
                                cfg_snap.cron_session_compaction_keep_recent.max(1);
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
                                // #3443 / cron-prune-lock-across-llm-await:
                                // the per-session mutex is acquired around the
                                // read-then-write cycle so two cron fires for
                                // the same agent cannot clobber each other's
                                // keep-set. **The lock MUST NOT be held across
                                // `try_summarize_trim().await`** — under a
                                // congested provider the LLM call can stall
                                // for tens of seconds, and the persistent
                                // `(agent, "cron")` lock would jam every
                                // sibling cron fire for the duration. Instead:
                                //
                                //   1. Take the lock, snapshot the messages
                                //      and record `messages_generation`.
                                //   2. Drop the lock.
                                //   3. Run `try_summarize_trim` lock-free.
                                //   4. Re-acquire the lock; if
                                //      `messages_generation` is unchanged,
                                //      apply the trim result; otherwise drop
                                //      our result (a concurrent fire won the
                                //      write-back) and log the conflict.
                                //
                                // `Session.messages_generation` (memory crate)
                                // is the canonical CAS counter and is already
                                // bumped by `set_messages` /
                                // `mark_messages_mutated`, so no new state is
                                // introduced.
                                let prune_lock = kernel_job
                                    .agents
                                    .session_msg_locks
                                    .entry(cron_sid)
                                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                                    .clone();
                                cron_prune_session(
                                    &kernel_job,
                                    agent_id,
                                    cron_sid,
                                    &job_name,
                                    prune_lock,
                                    max_tokens,
                                    max_messages,
                                    warn_threshold,
                                    warn_fallback,
                                    compaction_mode,
                                    keep_recent_cfg,
                                )
                                .await;
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
                                kernel_job.workflows.cron_scheduler.record_success(job_id);
                                // Persist last_run before delivery
                                // so a slow/failed channel push
                                // can't strand last_run on disk.
                                if let Err(e) = kernel_job.workflows.cron_scheduler.persist() {
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
                                    // failure-isolated). Skip the whole
                                    // call when there are no targets so
                                    // we never construct a fan-out engine
                                    // for the common no-webhook job (#5127).
                                    if !delivery_targets.is_empty() {
                                        cron_fan_out_targets(
                                            &kernel_job,
                                            &job_name,
                                            &result.response,
                                            &delivery_targets,
                                        )
                                        .await;
                                    }
                                }
                            }
                            Ok(Err(e)) => {
                                let err_msg = format!("{e}");
                                tracing::warn!(job = %job_name, error = %err_msg, "Cron job failed");
                                kernel_job
                                    .workflows
                                    .cron_scheduler
                                    .record_failure(job_id, &err_msg);
                                if let Err(e) = kernel_job.workflows.cron_scheduler.persist() {
                                    tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                }
                            }
                            Err(_) => {
                                tracing::warn!(job = %job_name, timeout_s, "Cron job timed out");
                                kernel_job.workflows.cron_scheduler.record_failure(
                                    job_id,
                                    &format!("timed out after {timeout_s}s"),
                                );
                                if let Err(e) = kernel_job.workflows.cron_scheduler.persist() {
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

                        // Resolve workflow by UUID first, then by name (case-insensitive,
                        // matching WorkflowRunner::run_workflow and the trigger-workflow
                        // dispatch path so cron/tool/trigger all agree on the same name).
                        let resolved_id =
                            if let Ok(uuid) = uuid::Uuid::parse_str(&workflow_id_owned) {
                                Some(crate::workflow::WorkflowId(uuid))
                            } else {
                                let name_lower = workflow_id_owned.to_lowercase();
                                let workflows = kernel_job.workflows.engine.list_workflows().await;
                                workflows
                                    .iter()
                                    .find(|w| w.name.to_lowercase() == name_lower)
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
                                        kernel_job.workflows.cron_scheduler.record_success(job_id);
                                        if let Err(e) =
                                            kernel_job.workflows.cron_scheduler.persist()
                                        {
                                            tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                        }
                                        cron_deliver_response(
                                            &kernel_job,
                                            agent_id,
                                            &output,
                                            &delivery,
                                        )
                                        .await;
                                        // Skip the fan-out call when no
                                        // targets are configured so we
                                        // don't construct an engine for
                                        // the common no-webhook job (#5127).
                                        if !delivery_targets.is_empty() {
                                            cron_fan_out_targets(
                                                &kernel_job,
                                                &job_name,
                                                &output,
                                                &delivery_targets,
                                            )
                                            .await;
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        let err_msg = format!("{e}");
                                        tracing::warn!(job = %job_name, error = %err_msg, "Cron workflow failed");
                                        kernel_job
                                            .workflows
                                            .cron_scheduler
                                            .record_failure(job_id, &err_msg);
                                        if let Err(e) =
                                            kernel_job.workflows.cron_scheduler.persist()
                                        {
                                            tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                        }
                                    }
                                    Err(_) => {
                                        tracing::warn!(job = %job_name, timeout_s, "Cron workflow timed out");
                                        kernel_job.workflows.cron_scheduler.record_failure(
                                            job_id,
                                            &format!("workflow timed out after {timeout_s}s"),
                                        );
                                        if let Err(e) =
                                            kernel_job.workflows.cron_scheduler.persist()
                                        {
                                            tracing::warn!(job = %job_name, "Cron post-run persist failed: {e}");
                                        }
                                    }
                                }
                            }
                            None => {
                                let err_msg = format!("workflow not found: {workflow_id_owned}");
                                tracing::warn!(job = %job_name, error = %err_msg, "Cron workflow lookup failed");
                                kernel_job
                                    .workflows
                                    .cron_scheduler
                                    .record_failure(job_id, &err_msg);
                                if let Err(e) = kernel_job.workflows.cron_scheduler.persist() {
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
            if let Err(e) = kernel.workflows.cron_scheduler.persist() {
                tracing::warn!("Cron persist failed: {e}");
            }
        }
    }
}

/// What the lock-held planning phase decided to do.
///
/// Returned from `cron_prune_plan` so the SummarizeTrim branch can release
/// the mutex *before* awaiting the LLM call. The cheap branches (Prune,
/// nothing-to-do, warn-only) are fully resolved while the lock is held.
enum CronPrunePlan {
    /// Nothing left to do — either no session exists, no compaction is
    /// needed, and the warn block (if any) already fired under the lock.
    Done,
    /// SummarizeTrim was selected. The LLM call must run lock-free; the
    /// snapshot here is what we pass to `try_summarize_trim`, and
    /// `snapshot_generation` is the `Session.messages_generation` value at
    /// snapshot time — checked under a re-acquired lock before the
    /// trimmed result is written back (CAS).
    SummarizeTrim {
        snapshot: Vec<librefang_types::message::Message>,
        effective_keep_recent: usize,
        keep_count: usize,
        model: String,
        echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
        snapshot_generation: u64,
    },
}

/// Cron-session prune / compaction with the lock released across the LLM
/// `try_summarize_trim` await.
///
/// See the in-place comment in `run_cron_scheduler_loop` (the call site)
/// for the full rationale — the short version is that holding the
/// per-session mutex across a slow provider call stalls every sibling
/// cron fire for the same `(agent, "cron")` session. We split the work
/// into three phases:
///
///   1. **Plan** under the lock: load the session, decide what to do,
///      and either finish (Prune / nothing) or return a snapshot for the
///      LLM call along with the current `messages_generation`.
///   2. **Summarize** lock-free.
///   3. **Apply** under a re-acquired lock, gated by a generation-counter
///      CAS so a concurrent writer's result is never silently overwritten.
#[allow(clippy::too_many_arguments)]
async fn cron_prune_session(
    kernel_job: &Arc<LibreFangKernel>,
    agent_id: AgentId,
    cron_sid: SessionId,
    job_name: &str,
    prune_lock: Arc<tokio::sync::Mutex<()>>,
    max_tokens: Option<u64>,
    max_messages: Option<usize>,
    warn_threshold: Option<u64>,
    warn_fallback: Option<u64>,
    compaction_mode: librefang_types::config::CronCompactionMode,
    keep_recent_cfg: usize,
) {
    // Phase 1 — plan under the lock.
    let plan = {
        let _g = prune_lock.lock().await;
        cron_prune_plan(
            kernel_job,
            agent_id,
            cron_sid,
            job_name,
            max_tokens,
            max_messages,
            warn_threshold,
            warn_fallback,
            compaction_mode,
            keep_recent_cfg,
        )
        .await
    };

    let (snapshot, effective_keep_recent, keep_count, model, echo_policy, snapshot_generation) =
        match plan {
            CronPrunePlan::Done => return,
            CronPrunePlan::SummarizeTrim {
                snapshot,
                effective_keep_recent,
                keep_count,
                model,
                echo_policy,
                snapshot_generation,
            } => (
                snapshot,
                effective_keep_recent,
                keep_count,
                model,
                echo_policy,
                snapshot_generation,
            ),
        };

    // Phase 2 — summarize lock-free. Tens of seconds on a congested
    // provider; sibling cron fires for this same `(agent, "cron")`
    // session must not be blocked here.
    let new_messages_opt = try_summarize_trim(
        &snapshot,
        effective_keep_recent,
        {
            kernel_job
                .llm
                .aux_client
                .load()
                .driver_for(librefang_types::config::AuxTask::Compression)
        },
        &model,
        echo_policy,
    )
    .await;

    // Phase 3 — re-acquire and CAS on `messages_generation`. If another
    // fire already wrote to this session (its `set_messages` /
    // `mark_messages_mutated` bumped the counter), our trimmed result is
    // based on a stale snapshot and we drop it; the concurrent writer wins.
    let _g = prune_lock.lock().await;
    let mut session = match kernel_job.memory.substrate.get_session(cron_sid) {
        Ok(Some(s)) => s,
        Ok(None) | Err(_) => {
            // Session disappeared (race with reset) — nothing to write back.
            return;
        }
    };
    if session.messages_generation != snapshot_generation {
        tracing::info!(
            agent_id = %agent_id,
            session_id = %cron_sid,
            job = %job_name,
            snapshot_generation,
            current_generation = session.messages_generation,
            "cron SummarizeTrim: session changed during LLM call; \
             a concurrent writer won the write-back, dropping our result"
        );
        return;
    }

    match new_messages_opt {
        Some(new_messages) => {
            let kept = new_messages.len();
            session.set_messages(new_messages);
            tracing::info!(
                agent_id = %agent_id,
                session_id = %cron_sid,
                job = %job_name,
                kept,
                "cron session summarize-and-trim complete"
            );
        }
        None => {
            // LLM unavailable, returned a fallback placeholder, or the
            // tool-pair adjustment left nothing to summarize — fall back
            // to a plain prune so the fire is not blocked. `keep_count`
            // was computed from the snapshot; the generation CAS above
            // guarantees the live session is identical, so this drop_count
            // is still correct.
            tracing::warn!(
                agent_id = %agent_id,
                session_id = %cron_sid,
                job = %job_name,
                "cron SummarizeTrim: LLM summarization failed or returned empty; \
                 falling back to Prune"
            );
            let drop_count = session.messages.len() - keep_count;
            apply_cron_prune(&mut session, drop_count);
        }
    }

    // Post-compaction approach-warn (#3693), now against the post-trim
    // session under the re-acquired lock so it sees the freshest state.
    cron_emit_warn_threshold(
        agent_id,
        cron_sid,
        job_name,
        &session,
        warn_threshold,
        max_tokens,
        warn_fallback,
        /* post_compaction = */ true,
    );

    let _ = kernel_job
        .memory
        .substrate
        .save_session_async(&session)
        .await;
}

/// Under-lock planning step. Either resolves a cheap (no-LLM) compaction
/// path fully (and returns `Done`), or returns a snapshot for the
/// SummarizeTrim path that will run lock-free.
#[allow(clippy::too_many_arguments)]
async fn cron_prune_plan(
    kernel_job: &Arc<LibreFangKernel>,
    agent_id: AgentId,
    cron_sid: SessionId,
    job_name: &str,
    max_tokens: Option<u64>,
    max_messages: Option<usize>,
    warn_threshold: Option<u64>,
    warn_fallback: Option<u64>,
    compaction_mode: librefang_types::config::CronCompactionMode,
    keep_recent_cfg: usize,
) -> CronPrunePlan {
    let mut session = match kernel_job.memory.substrate.get_session(cron_sid) {
        Ok(Some(s)) => s,
        Ok(None) | Err(_) => return CronPrunePlan::Done,
    };

    let keep_count = cron_compute_keep_count(&session.messages, max_messages, max_tokens);
    let needs_compaction = keep_count < session.messages.len();

    if !needs_compaction {
        // Even with no compaction we still want to fire the
        // post-compaction warn so operators see the trend.
        cron_emit_warn_threshold(
            agent_id,
            cron_sid,
            job_name,
            &session,
            warn_threshold,
            max_tokens,
            warn_fallback,
            /* post_compaction = */ false,
        );
        return CronPrunePlan::Done;
    }

    use librefang_types::config::CronCompactionMode;
    // #4683 review: re-route SummarizeTrim → Prune when the cap is too
    // tight for [summary] + 1-msg-tail (keep_count < 2). Without this,
    // SummarizeTrim would always write 2 messages back into a session
    // whose cap permits at most keep_count, and the next fire would
    // re-enter SummarizeTrim → endless aux LLM round-trips with no
    // convergence.
    let effective_compaction_mode = cron_resolve_compaction_mode(compaction_mode, keep_count);
    if compaction_mode == CronCompactionMode::SummarizeTrim
        && effective_compaction_mode == CronCompactionMode::Prune
    {
        tracing::warn!(
            agent_id = %agent_id,
            session_id = %cron_sid,
            job = %job_name,
            keep_count,
            "cron SummarizeTrim: cap too tight for [summary] + tail (keep_count < 2); falling back to Prune"
        );
    }

    match effective_compaction_mode {
        CronCompactionMode::Prune => {
            // Fully resolved under the lock — no await.
            let drop_count = session.messages.len() - keep_count;
            apply_cron_prune(&mut session, drop_count);
            cron_emit_warn_threshold(
                agent_id,
                cron_sid,
                job_name,
                &session,
                warn_threshold,
                max_tokens,
                warn_fallback,
                /* post_compaction = */ true,
            );
            let _ = kernel_job
                .memory
                .substrate
                .save_session_async(&session)
                .await;
            CronPrunePlan::Done
        }
        CronCompactionMode::SummarizeTrim => {
            // Snapshot + drop lock; the LLM call runs lock-free and a
            // re-acquired lock + generation CAS guards the write-back.
            //
            // Model: use the agent's model when available, otherwise an
            // empty string. `try_summarize_trim` fast-fails on empty
            // model names so we skip the LLM call (and the resulting
            // lock-free await) entirely and route straight to the
            // plain-prune fallback in the apply phase. Missing the
            // agent from the registry mid-cron is a symptom of a
            // registry / scheduler inconsistency worth surfacing.
            let model = match kernel_job.agents.registry.get(agent_id) {
                Some(e) => librefang_runtime::agent_loop::strip_provider_prefix(
                    &e.manifest.model.model,
                    &e.manifest.model.provider,
                ),
                None => {
                    tracing::warn!(
                        agent_id = %agent_id,
                        session_id = %cron_sid,
                        job = %job_name,
                        "cron SummarizeTrim: agent missing from registry; \
                         skipping LLM summary and falling back to plain prune"
                    );
                    String::new()
                }
            };
            let effective_keep_recent = cron_clamp_keep_recent(keep_recent_cfg, keep_count);
            let echo_policy = kernel_job.lookup_reasoning_echo_policy(&model);
            CronPrunePlan::SummarizeTrim {
                snapshot: session.messages.clone(),
                effective_keep_recent,
                keep_count,
                model,
                echo_policy,
                snapshot_generation: session.messages_generation,
            }
        }
    }
}

/// Emit the post-compaction "approaching context budget" warn (#3693).
///
/// Lifted from the inline block so both the lock-held cheap-path and the
/// re-acquired lock-held SummarizeTrim path can call it against the
/// freshest `session.messages` snapshot they hold.
#[allow(clippy::too_many_arguments)]
fn cron_emit_warn_threshold(
    agent_id: AgentId,
    cron_sid: SessionId,
    job_name: &str,
    session: &librefang_memory::session::Session,
    warn_threshold: Option<u64>,
    max_tokens: Option<u64>,
    warn_fallback: Option<u64>,
    post_compaction: bool,
) {
    let Some(threshold) = warn_threshold else {
        return;
    };
    use librefang_runtime::compactor::estimate_token_count;
    let estimated = estimate_token_count(&session.messages, None, None) as u64;
    if estimated < threshold {
        return;
    }
    let budget = max_tokens.or(warn_fallback);
    // `post_compaction` distinguishes "we just shrank and the session is
    // still over the soft threshold" (real signal — the current fire's
    // content is large) from "the session was already over threshold
    // before any compaction" (operator should tighten the cap). After
    // SummarizeTrim succeeds, the synthetic summary message can itself
    // be large enough to keep the estimate above threshold, so this warn
    // landing right after a successful compaction is expected — not a bug.
    tracing::warn!(
        agent_id = %agent_id,
        session_id = %cron_sid,
        job = %job_name,
        tokens = estimated,
        threshold = threshold,
        budget = ?budget,
        messages = session.messages.len(),
        post_compaction = post_compaction,
        "cron session approaching context budget — \
         consider lowering cron_session_max_tokens, \
         enabling cron_session_max_messages, or \
         setting session_mode = \"new\" on this job"
    );
}

#[cfg(test)]
mod tests {
    //! Structural tests for the `cron_prune_session` lock-release pattern
    //! introduced by the cron-prune-lock-across-llm-await fix. These tests
    //! do not construct a full `LibreFangKernel`; instead they reproduce
    //! the exact mutex + generation-CAS shape used in the helper above
    //! and assert the two load-bearing invariants:
    //!
    //!   1. Two concurrent prune fires whose "LLM summary" step takes
    //!      ~200ms must finish in ~200ms wall-clock, not ~400ms — i.e.
    //!      the lock is **not** held across the slow await.
    //!   2. When a concurrent writer bumps `messages_generation` during
    //!      the lock-free window, the loser's CAS check fires and its
    //!      trimmed result is dropped instead of overwriting the winner.
    //!
    //! Full end-to-end coverage against a real kernel + LLM driver lives
    //! in `crates/librefang-kernel/tests/cron_compaction_test.rs` and the
    //! `librefang-api` integration suite.

    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::Mutex;

    /// Mirror of the production three-phase pattern, with an injected slow
    /// "summarize" closure standing in for `try_summarize_trim`. Returns
    /// `(applied, snapshot_at_lock)` so tests can assert both the CAS
    /// decision and the snapshot freshness.
    async fn prune_with_seam<F, Fut>(
        lock: Arc<Mutex<()>>,
        shared_generation: Arc<std::sync::atomic::AtomicU64>,
        summarize: F,
    ) -> (bool, u64)
    where
        F: FnOnce(u64) -> Fut,
        Fut: std::future::Future<Output = u64>,
    {
        use std::sync::atomic::Ordering;

        // Phase 1: lock + snapshot.
        let snapshot_generation = {
            let _g = lock.lock().await;
            shared_generation.load(Ordering::Acquire)
        };

        // Phase 2: lock-free await.
        let new_value = summarize(snapshot_generation).await;

        // Phase 3: re-acquire + CAS.
        let _g = lock.lock().await;
        let current = shared_generation.load(Ordering::Acquire);
        if current != snapshot_generation {
            return (false, snapshot_generation);
        }
        shared_generation.fetch_add(1, Ordering::Release);
        // (Production code would apply `new_value` to the session here.)
        let _ = new_value;
        (true, snapshot_generation)
    }

    /// Two concurrent fires whose summarize step takes 200ms each must
    /// finish in ~200ms wall-clock (parallel), not ~400ms (serial). If
    /// the prune lock were held across the await, the second fire would
    /// block on the first's lock until its summarize completed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cron_prune_release_lock_across_summarize_concurrent() {
        let lock = Arc::new(Mutex::new(()));
        let gen_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));

        let lock_a = lock.clone();
        let gen_a = gen_counter.clone();
        let task_a = tokio::spawn(async move {
            prune_with_seam(lock_a, gen_a, |snap| async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                snap
            })
            .await
        });

        let lock_b = lock.clone();
        let gen_b = gen_counter.clone();
        let task_b = tokio::spawn(async move {
            prune_with_seam(lock_b, gen_b, |snap| async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                snap
            })
            .await
        });

        let start = Instant::now();
        let (res_a, res_b) = tokio::join!(task_a, task_b);
        let elapsed = start.elapsed();

        // Both tasks completed.
        res_a.expect("task A panicked");
        res_b.expect("task B panicked");

        // Wall-clock budget: 200ms ideal parallel, 400ms is fully
        // serialized. Set the bound at 350ms to allow generous CI
        // jitter while still failing loudly if the lock is re-introduced
        // across the await.
        assert!(
            elapsed < Duration::from_millis(350),
            "concurrent prune fires ran serially (elapsed={elapsed:?}); \
             the prune lock is being held across the summarize await"
        );
    }

    /// When the generation counter is bumped during the lock-free window
    /// (simulating a concurrent writer landing its own trim), the second
    /// CAS check must fire and the loser's apply must be skipped.
    #[tokio::test(flavor = "multi_thread")]
    async fn cron_prune_cas_drops_stale_writeback() {
        use std::sync::atomic::Ordering;

        let lock = Arc::new(Mutex::new(()));
        let gen_counter = Arc::new(std::sync::atomic::AtomicU64::new(7));

        // Run a single "fire" whose summarize step bumps the generation
        // mid-flight (as if another fire wrote between our snapshot and
        // our re-acquisition).
        let gen_for_seam = gen_counter.clone();
        let (applied, snapshot) = prune_with_seam(lock, gen_counter.clone(), |snap| async move {
            // Concurrent writer: bump the counter while the seam owner
            // is mid-await. In production this is another cron fire
            // that completed its own apply phase.
            gen_for_seam.fetch_add(1, Ordering::Release);
            snap
        })
        .await;

        assert_eq!(snapshot, 7, "snapshot must reflect the pre-bump generation");
        assert!(
            !applied,
            "the loser's apply must be skipped after a concurrent generation bump"
        );
        // Generation reflects only the concurrent writer's bump, not ours.
        assert_eq!(
            gen_counter.load(Ordering::Acquire),
            8,
            "loser must not have bumped the generation again"
        );
    }

    /// Without a concurrent bump, the single-fire CAS path completes
    /// normally and the generation advances by one (our own apply).
    #[tokio::test(flavor = "multi_thread")]
    async fn cron_prune_cas_applies_when_uncontended() {
        use std::sync::atomic::Ordering;

        let lock = Arc::new(Mutex::new(()));
        let gen_counter = Arc::new(std::sync::atomic::AtomicU64::new(3));

        let (applied, snapshot) = prune_with_seam(lock, gen_counter.clone(), |snap| async move {
            // No bump from a sibling writer.
            tokio::time::sleep(Duration::from_millis(10)).await;
            snap
        })
        .await;

        assert_eq!(snapshot, 3);
        assert!(applied, "uncontested single fire must apply its result");
        assert_eq!(gen_counter.load(Ordering::Acquire), 4);
    }
}
