//! Auto-dream: periodic per-agent background memory consolidation.
//!
//! Port of libre-code's `autoDream` — a time-gated background task that asks
//! each opt-in agent to reflect on its own memory and consolidate recent
//! signal via a 4-phase prompt (Orient / Gather / Consolidate / Prune).
//!
//! # Gates (cheapest first, per agent)
//!
//! 1. **Global enabled** — `config.auto_dream.enabled` must be true.
//! 2. **Per-agent opt-in** — the agent's manifest must have
//!    `auto_dream_enabled = true`.
//! 3. **Time** — at least `min_hours` must have elapsed since the last
//!    recorded consolidation for *that agent* (mtime of its lock file).
//! 4. **Session count** — at least `min_sessions` of that agent's sessions
//!    must have been touched since the last consolidation. Prevents
//!    consolidating an idle agent. Set `min_sessions = 0` to disable.
//! 5. **Lock** — per-agent filesystem lock with PID-staleness detection
//!    prevents two daemons on the same data directory from double-firing.
//!
//! # Progress tracking and abort
//!
//! Running dreams stream `StreamEvent`s from the LLM driver. Each event is
//! folded into a per-agent `DreamProgress` entry held in a process-local
//! `DashMap`. This is what lets the dashboard show "dream in progress:
//! editing memory XYZ, 3 turns" while the agent is still running, and what
//! the abort endpoint reaches into to cancel a detached dream task.
//!
//! A failed, aborted, or timed-out dream rolls back the lock mtime so the
//! time gate reopens on the next tick.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use dashmap::DashMap;
use librefang_channels::types::SenderContext;
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::AgentId;
use librefang_types::error::LibreFangResult;
use tokio::sync::oneshot;

use crate::kernel::LibreFangKernel;

pub mod lock;
pub mod prompt;

pub use lock::ConsolidationLock;

/// Channel name used for auto-dream invocations, so session scoping and
/// auditing distinguish dream turns from cron/user/channel turns.
pub const AUTO_DREAM_CHANNEL: &str = "auto_dream";

/// Default subdirectory under `data_dir` holding per-agent lock files.
const DEFAULT_LOCK_DIR: &str = "auto_dream";

/// Cap on turns retained in the progress entry. Matches libre-code's
/// `MAX_TURNS = 30` — enough to scroll through the reasoning without
/// ballooning memory for long dreams.
const MAX_TURNS: usize = 30;

/// Tool names whose invocation should be counted as "memory was modified"
/// for the filesTouched-equivalent tracking. librefang's memory is in a
/// SQLite substrate, not files — but we still want to show the user "the
/// dream wrote N memories" as progress signal. Keep this list aligned with
/// the tools actually registered in `librefang_runtime::tool_runner` —
/// listing ghost names here just makes the counter under-report.
const MEMORY_WRITE_TOOLS: &[&str] = &["memory_store"];

/// Tools the dream loop is allowed to call. Used to post-filter
/// `available_tools` when `sender.channel == AUTO_DREAM_CHANNEL` so a
/// prompt-injected dream can't escape into shell / file-edit / network
/// tools even if the target agent's manifest would otherwise permit them.
/// Matches libre-code's `createAutoMemCanUseTool(memoryRoot)` restriction.
pub const DREAM_ALLOWED_TOOLS: &[&str] = &["memory_store", "memory_recall", "memory_list"];

// ---------------------------------------------------------------------------
// Progress types
// ---------------------------------------------------------------------------

/// Lifecycle state of a single dream invocation.
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DreamStatus {
    Running,
    Completed,
    Failed,
    Aborted,
}

/// One assistant turn observed during a dream. Tool uses are collapsed to a
/// count so the progress payload stays small; see libre-code's `DreamTurn`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DreamTurn {
    pub text: String,
    pub tool_use_count: u32,
}

/// Live progress for a dream. One entry per agent — overwritten when that
/// agent dreams again, never evicted on its own.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DreamProgress {
    pub task_id: String,
    pub agent_id: String,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub status: DreamStatus,
    pub phase: String,
    /// Number of tool calls observed across all turns of this dream.
    pub tool_use_count: u32,
    /// Memory identifiers / previews touched by `memory_store`-family calls.
    /// Deduplicated, insertion-ordered. Analogue of libre-code's
    /// `filesTouched`.
    pub memories_touched: Vec<String>,
    /// Recent assistant turns, oldest first, capped at [`MAX_TURNS`].
    pub turns: Vec<DreamTurn>,
    /// Last error or abort reason, if any.
    pub error: Option<String>,
    /// Token usage for the completed dream. Populated on `Completed` status;
    /// remains `None` while `Running` and for `Failed` / `Aborted` dreams
    /// where the loop result isn't available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<DreamUsage>,
}

/// Snapshot of token / cost / latency for a completed dream. Mirrors the
/// fields libre-code logs via `tengu_auto_dream_completed`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DreamUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub iterations: u32,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Process-local progress registry. Keyed by agent since only one dream per
/// agent can be in flight (the file lock enforces that).
static DREAM_PROGRESS: LazyLock<DashMap<AgentId, DreamProgress>> = LazyLock::new(DashMap::new);

/// Interior of an abort slot — the oneshot sender is taken out on abort
/// so a second call can distinguish "already signalled" from "nothing in
/// flight". Wrapped in a `std::sync::Mutex` because DashMap entries are
/// accessed through shared references and `Sender::send` consumes self.
type AbortSlot = Mutex<Option<oneshot::Sender<()>>>;

/// Abort channels for in-flight manually-triggered dreams. Sending on the
/// oneshot notifies `run_dream`'s drain loop to break out and run the
/// `finalize_abort` cleanup path (lock rollback + inner-LLM-task abort).
///
/// Scheduled dreams run inline inside the scheduler loop to keep token
/// spend serial and don't install an abort channel — the scheduler can't
/// be interrupted without disrupting other agents' turn in the queue.
///
/// Entries are removed by the owning `run_dream` on any terminal state
/// (complete / fail / abort / timeout) so `ABORT_HANDLES.contains_key` is
/// a reliable "a manual dream is still running" signal for `can_abort` in
/// the status endpoint.
static ABORT_HANDLES: LazyLock<DashMap<AgentId, Arc<AbortSlot>>> = LazyLock::new(DashMap::new);

fn insert_progress(agent_id: AgentId, progress: DreamProgress) {
    DREAM_PROGRESS.insert(agent_id, progress);
}

fn mutate_progress<F: FnOnce(&mut DreamProgress)>(agent_id: AgentId, f: F) {
    if let Some(mut entry) = DREAM_PROGRESS.get_mut(&agent_id) {
        f(entry.value_mut());
    }
}

/// Read the current progress entry for an agent. Returns `None` when no
/// dream has ever run for that agent.
pub fn get_progress(agent_id: AgentId) -> Option<DreamProgress> {
    DREAM_PROGRESS.get(&agent_id).map(|r| r.value().clone())
}

/// Snapshot all progress entries (for the status endpoint).
fn all_progress() -> std::collections::HashMap<AgentId, DreamProgress> {
    DREAM_PROGRESS
        .iter()
        .map(|r| (*r.key(), r.value().clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Lock helpers
// ---------------------------------------------------------------------------

fn lock_dir_for_kernel(kernel: &LibreFangKernel) -> PathBuf {
    let cfg = kernel.config_snapshot();
    if cfg.auto_dream.lock_dir.is_empty() {
        kernel.data_dir().join(DEFAULT_LOCK_DIR)
    } else {
        PathBuf::from(&cfg.auto_dream.lock_dir)
    }
}

fn lock_for_agent(kernel: &LibreFangKernel, agent_id: AgentId) -> ConsolidationLock {
    let path = lock_dir_for_kernel(kernel).join(format!("{agent_id}.lock"));
    ConsolidationLock::new(path)
}

// ---------------------------------------------------------------------------
// Gate check
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum AgentGateResult {
    Fire { prior_mtime: u64 },
    TooSoon { hours_remaining: f64 },
    NoActivity { sessions_since: u32, required: u32 },
    LockHeld,
    Skipped(String),
}

/// Resolve the effective `(min_hours, min_sessions)` for an agent: manifest
/// override wins over global config when `Some`. Separated out so the
/// status endpoint and the scheduler converge on the same values.
fn effective_thresholds(kernel: &LibreFangKernel, agent_id: AgentId) -> (f64, u32) {
    let cfg = kernel.config_snapshot();
    let (hours, sessions) = kernel
        .agent_registry()
        .get(agent_id)
        .map(|e| {
            (
                e.manifest.auto_dream_min_hours,
                e.manifest.auto_dream_min_sessions,
            )
        })
        .unwrap_or((None, None));
    (
        hours.unwrap_or(cfg.auto_dream.min_hours),
        sessions.unwrap_or(cfg.auto_dream.min_sessions),
    )
}

async fn check_agent_gates(
    kernel: &LibreFangKernel,
    agent_id: AgentId,
    bypass_time_gate: bool,
) -> AgentGateResult {
    let lock = lock_for_agent(kernel, agent_id);

    // Per-agent overrides take precedence over the global defaults. A quiet
    // agent can set `auto_dream_min_hours = 168` for weekly dreams; a chatty
    // one can set `auto_dream_min_sessions = 1` to consolidate after every
    // session. Fall back to config.toml when unset.
    let (effective_min_hours, effective_min_sessions) = effective_thresholds(kernel, agent_id);

    let last_at = match lock.read_last_consolidated_at().await {
        Ok(t) => t,
        Err(e) => return AgentGateResult::Skipped(format!("read lock failed: {e}")),
    };

    if !bypass_time_gate {
        let now = now_ms();
        let hours_since = ((now.saturating_sub(last_at)) as f64) / 3_600_000.0;
        if hours_since < effective_min_hours {
            return AgentGateResult::TooSoon {
                hours_remaining: effective_min_hours - hours_since,
            };
        }
    }

    if effective_min_sessions > 0 && !bypass_time_gate {
        match kernel
            .memory_substrate()
            .count_agent_sessions_touched_since(agent_id, last_at)
        {
            Ok(count) if count < effective_min_sessions => {
                return AgentGateResult::NoActivity {
                    sessions_since: count,
                    required: effective_min_sessions,
                };
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(agent = %agent_id, error = %e, "auto_dream: session count query failed, skipping gate");
            }
        }
    }

    match lock.try_acquire().await {
        Ok(Some(prior_mtime)) => AgentGateResult::Fire { prior_mtime },
        Ok(None) => AgentGateResult::LockHeld,
        Err(e) => AgentGateResult::Skipped(format!("lock acquire failed: {e}")),
    }
}

// ---------------------------------------------------------------------------
// The streaming dream loop
// ---------------------------------------------------------------------------

/// Fold a single [`StreamEvent`] into the progress entry for this agent.
/// Called once per event received from the LLM driver.
fn apply_stream_event(agent_id: AgentId, ev: &StreamEvent, pending: &mut PendingTurn) {
    match ev {
        StreamEvent::PhaseChange { phase, .. } => {
            let phase = phase.clone();
            mutate_progress(agent_id, |p| p.phase = phase);
        }
        StreamEvent::TextDelta { text } => {
            pending.text.push_str(text);
        }
        StreamEvent::ToolUseStart { .. } => {
            pending.tool_use_count += 1;
        }
        StreamEvent::ToolUseEnd { name, input, .. } => {
            if MEMORY_WRITE_TOOLS.contains(&name.as_str()) {
                // The `memory_store` tool takes `{key, value}` (see
                // `librefang-runtime/src/tool_runner.rs`). Prefer the key
                // as a human-meaningful preview; fall back to a clipped
                // value when the key is absent, and to the tool name only
                // if neither is present (shouldn't happen for valid calls,
                // but we don't want to panic).
                let preview = input
                    .get("key")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| {
                        input.get("value").and_then(|v| v.as_str()).map(|s| {
                            let first_line = s.lines().next().unwrap_or(s);
                            // char-safe truncation — byte slicing would
                            // panic on multi-byte characters (CJK, emoji)
                            // landing on a boundary inside an 80-byte window.
                            if first_line.chars().count() > 80 {
                                let clipped: String = first_line.chars().take(80).collect();
                                format!("{clipped}…")
                            } else {
                                first_line.to_string()
                            }
                        })
                    })
                    .unwrap_or_else(|| name.clone());
                mutate_progress(agent_id, |p| {
                    if !p.memories_touched.iter().any(|m| m == &preview) {
                        p.memories_touched.push(preview);
                    }
                    if p.phase == "starting" || p.phase.is_empty() {
                        p.phase = "updating".to_string();
                    }
                });
            }
        }
        StreamEvent::ContentComplete { .. } => {
            // Flush the pending turn. An empty, toolless turn is a no-op —
            // skip it to match libre-code's addDreamTurn behaviour.
            let text = std::mem::take(&mut pending.text).trim().to_string();
            let count = std::mem::take(&mut pending.tool_use_count);
            if !text.is_empty() || count > 0 {
                mutate_progress(agent_id, |p| {
                    if p.turns.len() >= MAX_TURNS {
                        p.turns.remove(0);
                    }
                    p.turns.push(DreamTurn {
                        text,
                        tool_use_count: count,
                    });
                    p.tool_use_count = p.tool_use_count.saturating_add(count);
                });
            }
        }
        _ => {}
    }
}

/// Scratchpad for accumulating a single turn's deltas before flushing on
/// `ContentComplete`. Lives inside the consumer loop so it doesn't pollute
/// the global progress map.
#[derive(Default)]
struct PendingTurn {
    text: String,
    tool_use_count: u32,
}

/// Run one dream for one agent end-to-end. This is the body shared by both
/// the scheduler (awaits inline, `abort_rx = None`) and the manual-trigger
/// path (spawned with an abort channel). When `abort_rx` is `Some` and
/// fires, the drain loop exits, the inner LLM task is aborted, and the
/// finalizer rolls the lock mtime back so the time gate reopens.
async fn run_dream(
    kernel: Arc<LibreFangKernel>,
    target: AgentId,
    prior_mtime: u64,
    abort_rx: Option<oneshot::Receiver<()>>,
) {
    let task_id = uuid::Uuid::new_v4().to_string();
    insert_progress(
        target,
        DreamProgress {
            task_id: task_id.clone(),
            agent_id: target.to_string(),
            started_at_ms: now_ms(),
            ended_at_ms: None,
            status: DreamStatus::Running,
            phase: "starting".to_string(),
            tool_use_count: 0,
            memories_touched: Vec::new(),
            turns: Vec::new(),
            error: None,
            usage: None,
        },
    );
    kernel.audit().record(
        target.to_string(),
        librefang_runtime::audit::AuditAction::DreamConsolidation,
        format!("phase=start task_id={task_id}"),
        "ok",
    );

    // Query the sessions touched since the prior dream so we can embed their
    // IDs in the prompt, letting the model narrow its gather phase instead
    // of cold-searching the memory substrate. Match libre-code's behaviour
    // of surfacing concrete IDs. Capped to a reasonable list length — the
    // scheduler's session-count gate already capped the number of eligible
    // dreams, but a very-active agent could touch hundreds of sessions.
    const MAX_SESSION_IDS_IN_PROMPT: u32 = 50;
    let session_ids = kernel
        .memory_substrate()
        .list_agent_sessions_touched_since(target, prior_mtime, MAX_SESSION_IDS_IN_PROMPT)
        .unwrap_or_default();
    let total_sessions = kernel
        .memory_substrate()
        .count_agent_sessions_touched_since(target, prior_mtime)
        .unwrap_or(session_ids.len() as u32);

    let prompt_text = prompt::build_consolidation_prompt(prompt::ConsolidationPromptInput {
        session_ids: &session_ids,
        total_sessions,
        extra: "",
    });
    let sender = SenderContext {
        channel: AUTO_DREAM_CHANNEL.to_string(),
        user_id: String::new(),
        display_name: AUTO_DREAM_CHANNEL.to_string(),
        is_group: false,
        was_mentioned: false,
        thread_id: None,
        account_id: None,
        ..Default::default()
    };

    let timeout_secs = kernel.config_snapshot().auto_dream.timeout_secs;
    let timeout = Duration::from_secs(timeout_secs.max(30));

    // Hold the abort receiver owned from here on so we can drop it before
    // every finalizer — preventing `abort_dream` from reporting
    // `aborted=true` while the dream is already on its way out.
    let mut abort_rx = abort_rx;

    // Kick off streaming.
    let (mut rx, join_handle) = match kernel
        .send_message_streaming_with_sender_context_and_routing(target, &prompt_text, None, &sender)
        .await
    {
        Ok(pair) => pair,
        Err(e) => {
            abort_rx.take();
            finalize_failure(
                &kernel,
                target,
                prior_mtime,
                format!("stream start failed: {e}"),
            )
            .await;
            return;
        }
    };

    // Drain events, applying to progress. The loop exits when the channel
    // closes (natural completion), the overall timeout elapses, OR the
    // abort signal fires (manual trigger only). For the abort case we
    // need to both tear down the inner LLM task (so it stops burning
    // tokens) and run `finalize_abort` to roll the lock back — neither
    // of which would happen if the outer spawn were aborted directly.
    let deadline = tokio::time::Instant::now() + timeout;
    let mut pending = PendingTurn::default();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            tracing::warn!(agent = %target, "auto_dream: deadline exceeded during stream");
            join_handle.abort();
            abort_rx.take();
            finalize_failure(&kernel, target, prior_mtime, "timed out".to_string()).await;
            return;
        }

        // With an abort channel: race the event stream against the abort
        // signal. Without one (scheduler path): just drain events.
        let recv_result = if let Some(rx_abort) = abort_rx.as_mut() {
            tokio::select! {
                // biased: if both arms are ready, prefer honouring the
                // abort so a rapidly-firing dream can still be cancelled.
                biased;
                _ = rx_abort => {
                    join_handle.abort();
                    // The receiver already resolved inside this select arm;
                    // dropping it via `take` is defensive — a second abort
                    // attempt should report "already signalled" rather than
                    // hitting a resolved receiver.
                    abort_rx.take();
                    finalize_abort(&kernel, target, prior_mtime).await;
                    return;
                }
                result = tokio::time::timeout(remaining, rx.recv()) => result,
            }
        } else {
            tokio::time::timeout(remaining, rx.recv()).await
        };

        match recv_result {
            Ok(Some(ev)) => {
                apply_stream_event(target, &ev, &mut pending);
            }
            Ok(None) => break, // channel closed — stream finished
            Err(_) => {
                join_handle.abort();
                abort_rx.take();
                finalize_failure(&kernel, target, prior_mtime, "timed out".to_string()).await;
                return;
            }
        }
    }

    // Drain loop exited naturally. Drop the abort receiver before we move
    // into the finalizer path so any abort signal arriving from here on
    // fails `oneshot::Sender::send` — `abort_dream` surfaces that as
    // "dream already finished before abort landed" instead of falsely
    // claiming a running dream was cancelled.
    abort_rx.take();

    // Channel closed — wait for the join handle to surface the final result.
    match join_handle.await {
        Ok(Ok(result)) => {
            let usage = DreamUsage {
                input_tokens: result.total_usage.input_tokens,
                output_tokens: result.total_usage.output_tokens,
                cache_read_input_tokens: result.total_usage.cache_read_input_tokens,
                cache_creation_input_tokens: result.total_usage.cache_creation_input_tokens,
                iterations: result.iterations,
                latency_ms: result.latency_ms,
                cost_usd: result.cost_usd,
            };
            mutate_progress(target, |p| {
                p.status = DreamStatus::Completed;
                p.ended_at_ms = Some(now_ms());
                p.phase = "completed".to_string();
                p.usage = Some(usage.clone());
            });
            let memories_touched_count = get_progress(target)
                .map(|p| p.memories_touched.len())
                .unwrap_or(0);
            tracing::info!(
                agent = %target,
                task_id = %task_id,
                iterations = usage.iterations,
                input_tokens = usage.input_tokens,
                output_tokens = usage.output_tokens,
                cache_read = usage.cache_read_input_tokens,
                cost_usd = ?usage.cost_usd,
                "auto_dream: consolidation completed",
            );
            kernel.audit().record(
                target.to_string(),
                librefang_runtime::audit::AuditAction::DreamConsolidation,
                format!(
                    "phase=complete task_id={task_id} memories_touched={memories_touched_count} \
                     input_tokens={input} output_tokens={output} cache_read={cache_read} \
                     cost_usd={cost}",
                    input = usage.input_tokens,
                    output = usage.output_tokens,
                    cache_read = usage.cache_read_input_tokens,
                    cost = usage
                        .cost_usd
                        .map(|c| format!("{c:.6}"))
                        .unwrap_or_else(|| "null".to_string()),
                ),
                "ok",
            );
            // Success cleanup — removes the abort entry so the status
            // endpoint stops advertising `can_abort: true` for a dream
            // that's already done.
            ABORT_HANDLES.remove(&target);
        }
        Ok(Err(e)) => {
            finalize_failure(
                &kernel,
                target,
                prior_mtime,
                format!("agent loop failed: {e}"),
            )
            .await;
        }
        Err(e) if e.is_cancelled() => {
            // Marked aborted elsewhere by the abort endpoint.
            finalize_abort(&kernel, target, prior_mtime).await;
        }
        Err(e) => {
            finalize_failure(&kernel, target, prior_mtime, format!("join failed: {e}")).await;
        }
    }
}

async fn finalize_failure(
    kernel: &LibreFangKernel,
    target: AgentId,
    prior_mtime: u64,
    reason: String,
) {
    tracing::warn!(agent = %target, reason = %reason, "auto_dream: dream failed, rolling back lock");
    let task_id = get_progress(target).map(|p| p.task_id).unwrap_or_default();
    mutate_progress(target, |p| {
        p.status = DreamStatus::Failed;
        p.ended_at_ms = Some(now_ms());
        p.phase = "failed".to_string();
        p.error = Some(reason.clone());
    });
    kernel.audit().record(
        target.to_string(),
        librefang_runtime::audit::AuditAction::DreamConsolidation,
        format!("phase=fail task_id={task_id} reason={reason}"),
        "fail",
    );
    let lock = lock_for_agent(kernel, target);
    if let Err(e) = lock.rollback(prior_mtime).await {
        tracing::warn!(error = %e, "auto_dream: rollback after failure also failed");
    }
    ABORT_HANDLES.remove(&target);
}

async fn finalize_abort(kernel: &LibreFangKernel, target: AgentId, prior_mtime: u64) {
    tracing::info!(agent = %target, "auto_dream: dream aborted, rolling back lock");
    let task_id = get_progress(target).map(|p| p.task_id).unwrap_or_default();
    mutate_progress(target, |p| {
        p.status = DreamStatus::Aborted;
        p.ended_at_ms = Some(now_ms());
        p.phase = "aborted".to_string();
        p.error.get_or_insert_with(|| "aborted by user".to_string());
    });
    kernel.audit().record(
        target.to_string(),
        librefang_runtime::audit::AuditAction::DreamConsolidation,
        format!("phase=abort task_id={task_id}"),
        "aborted",
    );
    let lock = lock_for_agent(kernel, target);
    if let Err(e) = lock.rollback(prior_mtime).await {
        tracing::warn!(error = %e, "auto_dream: rollback after abort also failed");
    }
    ABORT_HANDLES.remove(&target);
}

// ---------------------------------------------------------------------------
// Scheduler loop
// ---------------------------------------------------------------------------

fn enrolled_agents(kernel: &LibreFangKernel) -> Vec<(AgentId, String)> {
    kernel
        .agent_registry()
        .list()
        .into_iter()
        .filter(|e| e.manifest.auto_dream_enabled)
        .map(|e| (e.id, e.name))
        .collect()
}

/// All agents with their current opt-in state, for the settings UI toggle
/// list. The scheduler uses `enrolled_agents` — this is UI-only.
fn all_agents_dream_state(kernel: &LibreFangKernel) -> Vec<(AgentId, String, bool)> {
    kernel
        .agent_registry()
        .list()
        .into_iter()
        .map(|e| (e.id, e.name, e.manifest.auto_dream_enabled))
        .collect()
}

/// Toggle an agent's auto-dream opt-in. Returns `Err` if the agent doesn't
/// exist. The scheduler picks up the change on its next tick.
pub fn set_agent_enabled(
    kernel: &LibreFangKernel,
    agent_id: AgentId,
    enabled: bool,
) -> LibreFangResult<()> {
    kernel
        .agent_registry()
        .update_auto_dream_enabled(agent_id, enabled)?;
    kernel.audit().record(
        agent_id.to_string(),
        librefang_runtime::audit::AuditAction::ConfigChange,
        format!("auto_dream_enabled={enabled}"),
        "ok",
    );
    Ok(())
}

pub fn spawn_scheduler(kernel: Arc<LibreFangKernel>) {
    tokio::spawn(async move {
        {
            let cfg = kernel.config_snapshot();
            if cfg.auto_dream.enabled {
                tracing::info!(
                    min_hours = cfg.auto_dream.min_hours,
                    min_sessions = cfg.auto_dream.min_sessions,
                    check_interval_s = cfg.auto_dream.check_interval_secs,
                    "auto_dream: enabled (per-agent opt-in via manifest)"
                );
            } else {
                tracing::debug!("auto_dream: disabled");
            }
        }

        loop {
            let interval_s = {
                let cfg = kernel.config_snapshot();
                // Floor matches the config-schema minimum (see
                // `routes/config.rs`), so a direct TOML edit can't sneak
                // a shorter cadence past the UI's validation.
                cfg.auto_dream.check_interval_secs.max(60)
            };
            tokio::time::sleep(Duration::from_secs(interval_s)).await;

            if kernel.supervisor.is_shutting_down() {
                tracing::debug!("auto_dream: shutdown detected, scheduler exiting");
                return;
            }

            let cfg = kernel.config_snapshot();
            if !cfg.auto_dream.enabled {
                continue;
            }

            for (agent_id, name) in enrolled_agents(&kernel) {
                match check_agent_gates(&kernel, agent_id, false).await {
                    AgentGateResult::Fire { prior_mtime } => {
                        // Scheduled dreams run inline — serial token spend.
                        // `None` for abort_rx: scheduled dreams can't be
                        // cancelled individually without stalling the
                        // queue; users who want to interrupt wait for the
                        // dream's own timeout or disable auto-dream.
                        run_dream(Arc::clone(&kernel), agent_id, prior_mtime, None).await;
                    }
                    AgentGateResult::TooSoon { hours_remaining } => {
                        tracing::trace!(agent = %agent_id, hours_remaining, "auto_dream: time gate not yet open");
                    }
                    AgentGateResult::NoActivity {
                        sessions_since,
                        required,
                    } => {
                        tracing::trace!(agent = %agent_id, sessions_since, required, "auto_dream: agent idle, session gate not met");
                    }
                    AgentGateResult::LockHeld => {
                        tracing::debug!(agent = %agent_id, "auto_dream: lock held, skipping tick");
                    }
                    AgentGateResult::Skipped(reason) => {
                        tracing::warn!(agent = %agent_id, name = %name, reason, "auto_dream: skipped agent this tick");
                    }
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// HTTP-facing status / trigger / abort helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoDreamAgentStatus {
    pub agent_id: String,
    pub agent_name: String,
    pub auto_dream_enabled: bool,
    pub last_consolidated_at_ms: u64,
    pub next_eligible_at_ms: u64,
    pub hours_since_last: f64,
    pub sessions_since_last: u32,
    /// Resolved `min_hours` for this agent — manifest override if set,
    /// otherwise the global `[auto_dream] min_hours`.
    pub effective_min_hours: f64,
    /// Resolved `min_sessions` for this agent — manifest override if set,
    /// otherwise the global `[auto_dream] min_sessions`. `0` means the
    /// session-count gate is disabled.
    pub effective_min_sessions: u32,
    pub lock_path: String,
    /// Current or most recent dream progress for this agent. `None` when
    /// the agent has never dreamed on this daemon's uptime.
    pub progress: Option<DreamProgress>,
    /// True when an abort-capable dream is in flight (manual trigger only).
    pub can_abort: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoDreamStatus {
    pub enabled: bool,
    pub min_hours: f64,
    pub min_sessions: u32,
    pub check_interval_secs: u64,
    pub lock_dir: String,
    pub agents: Vec<AutoDreamAgentStatus>,
}

pub async fn current_status(kernel: &LibreFangKernel) -> AutoDreamStatus {
    let cfg = kernel.config_snapshot();
    let lock_dir = lock_dir_for_kernel(kernel);
    let now = now_ms();
    let progress_map = all_progress();

    let mut agents = Vec::new();
    for (agent_id, name, enabled) in all_agents_dream_state(kernel) {
        // Per-agent effective thresholds may diverge from the global config
        // when the manifest overrides either knob.
        let (eff_hours, eff_sessions) = effective_thresholds(kernel, agent_id);
        let min_ms = (eff_hours * 3_600_000.0) as u64;

        // Runtime stats (lock, sessions-since) only matter for opted-in
        // agents. For opt-out agents we still return a row so the settings
        // UI can render a toggle — but fields stay at zero/defaults.
        let (last, sessions_since, lock_path, progress, can_abort) = if enabled {
            let lock = lock_for_agent(kernel, agent_id);
            let last = lock.read_last_consolidated_at().await.unwrap_or(0);
            let sessions_since = kernel
                .memory_substrate()
                .count_agent_sessions_touched_since(agent_id, last)
                .unwrap_or(0);
            let progress = progress_map.get(&agent_id).cloned();
            let can_abort = ABORT_HANDLES.contains_key(&agent_id)
                && progress
                    .as_ref()
                    .map(|p| p.status == DreamStatus::Running)
                    .unwrap_or(false);
            (
                last,
                sessions_since,
                lock.path().display().to_string(),
                progress,
                can_abort,
            )
        } else {
            (0, 0, String::new(), None, false)
        };

        let hours_since = if last == 0 {
            f64::INFINITY
        } else {
            ((now.saturating_sub(last)) as f64) / 3_600_000.0
        };

        agents.push(AutoDreamAgentStatus {
            agent_id: agent_id.to_string(),
            agent_name: name,
            auto_dream_enabled: enabled,
            last_consolidated_at_ms: last,
            next_eligible_at_ms: last.saturating_add(min_ms),
            hours_since_last: hours_since,
            sessions_since_last: sessions_since,
            effective_min_hours: eff_hours,
            effective_min_sessions: eff_sessions,
            lock_path,
            progress,
            can_abort,
        });
    }

    AutoDreamStatus {
        enabled: cfg.auto_dream.enabled,
        min_hours: cfg.auto_dream.min_hours,
        min_sessions: cfg.auto_dream.min_sessions,
        check_interval_secs: cfg.auto_dream.check_interval_secs,
        lock_dir: lock_dir.display().to_string(),
        agents,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TriggerOutcome {
    pub fired: bool,
    pub agent_id: String,
    pub task_id: Option<String>,
    pub reason: String,
}

pub async fn trigger_manual(kernel: Arc<LibreFangKernel>, agent_id: AgentId) -> TriggerOutcome {
    let id_str = agent_id.to_string();
    let cfg = kernel.config_snapshot();
    if !cfg.auto_dream.enabled {
        return TriggerOutcome {
            fired: false,
            agent_id: id_str,
            task_id: None,
            reason: "auto-dream is disabled in config".to_string(),
        };
    }

    match kernel.agent_registry().get(agent_id) {
        None => {
            return TriggerOutcome {
                fired: false,
                agent_id: id_str,
                task_id: None,
                reason: "agent not found".to_string(),
            };
        }
        Some(entry) if !entry.manifest.auto_dream_enabled => {
            // Matches the UI: the `Dream now` button only renders for
            // opted-in agents. A direct API caller hitting this path
            // probably forgot to toggle; returning `fired=false` with a
            // specific reason is clearer than firing silently.
            return TriggerOutcome {
                fired: false,
                agent_id: id_str,
                task_id: None,
                reason: "agent is not enrolled (auto_dream_enabled=false on manifest)".to_string(),
            };
        }
        Some(_) => {}
    }

    match check_agent_gates(&kernel, agent_id, true).await {
        AgentGateResult::Fire { prior_mtime } => {
            // Install the abort channel *before* spawning so a racing
            // abort call sees the slot. The slot is removed by
            // `run_dream` on any terminal state; aborting after that is
            // a no-op with a "nothing in flight" reason.
            let (abort_tx, abort_rx) = oneshot::channel::<()>();
            let slot = Arc::new(Mutex::new(Some(abort_tx)));
            ABORT_HANDLES.insert(agent_id, slot);
            let k = Arc::clone(&kernel);
            tokio::spawn(async move {
                run_dream(k, agent_id, prior_mtime, Some(abort_rx)).await;
            });
            // task_id becomes available once run_dream installs the
            // progress entry; in practice insert_progress runs on the very
            // first await point. Read it back for the response.
            let task_id = get_progress(agent_id).map(|p| p.task_id);
            TriggerOutcome {
                fired: true,
                agent_id: id_str,
                task_id,
                reason: "consolidation fired".to_string(),
            }
        }
        AgentGateResult::LockHeld => TriggerOutcome {
            fired: false,
            agent_id: id_str,
            task_id: None,
            reason: "lock is held — consolidation already in progress".to_string(),
        },
        AgentGateResult::Skipped(reason) => TriggerOutcome {
            fired: false,
            agent_id: id_str,
            task_id: None,
            reason,
        },
        AgentGateResult::TooSoon { .. } | AgentGateResult::NoActivity { .. } => TriggerOutcome {
            fired: false,
            agent_id: id_str,
            task_id: None,
            reason: "unexpected gate outcome for manual trigger".to_string(),
        },
    }
}

/// Outcome of an abort request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AbortOutcome {
    pub aborted: bool,
    pub agent_id: String,
    pub reason: String,
}

/// Cancel an in-flight manually-triggered dream. Scheduled dreams cannot be
/// aborted — the scheduler awaits them inline on its own task, and tearing
/// that down would disrupt other agents' turn in the queue. Users who want
/// to cancel a scheduled dream should wait (dreams have a configurable
/// timeout) or disable auto-dream globally.
///
/// Signals `run_dream` via the oneshot channel installed by
/// `trigger_manual`. `run_dream`'s drain loop notices, aborts the inner
/// LLM task, and runs `finalize_abort` (progress → Aborted, lock mtime
/// rolled back, `ABORT_HANDLES` entry removed).
///
/// We intentionally do NOT abort the outer spawn directly — doing so
/// would drop `run_dream`'s future mid-await, leaking the inner LLM task
/// and leaving the lock mtime at "now" so the time gate stays closed.
pub async fn abort_dream(agent_id: AgentId) -> AbortOutcome {
    let id_str = agent_id.to_string();
    // Don't `remove` here — `run_dream`'s finalizer is the single owner
    // of cleanup. Removing now would cause a racing `run_dream` that's
    // already mid-finalize to `remove` a different entry (a new one
    // installed by the next `trigger_manual`).
    let Some(slot_ref) = ABORT_HANDLES.get(&agent_id).map(|r| Arc::clone(r.value())) else {
        return AbortOutcome {
            aborted: false,
            agent_id: id_str,
            reason: "no abort-capable dream in flight for this agent".to_string(),
        };
    };
    let sender = match slot_ref.lock() {
        Ok(mut g) => g.take(),
        // A poisoned mutex would mean a prior panic while the sender was
        // held; recover the inner data and continue rather than deadlock.
        Err(poisoned) => poisoned.into_inner().take(),
    };
    let Some(tx) = sender else {
        return AbortOutcome {
            aborted: false,
            agent_id: id_str,
            reason: "abort already signalled for this dream".to_string(),
        };
    };
    // `send` fails only if the receiver dropped — which means `run_dream`
    // already finished or its future was dropped. Either way there's
    // nothing to abort and the finalizer ran (or is about to run) on its
    // own. Treat this as aborted=false with a descriptive reason rather
    // than pretending the signal landed.
    if tx.send(()).is_err() {
        return AbortOutcome {
            aborted: false,
            agent_id: id_str,
            reason: "dream already finished before abort landed".to_string(),
        };
    }
    AbortOutcome {
        aborted: true,
        agent_id: id_str,
        reason: "abort signalled; lock will roll back shortly".to_string(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
