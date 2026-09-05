//! Kernel-side wiring for the autonomous goal runner (#5744).
//!
//! Bridges the standalone [`crate::goal_runner::GoalRunner`] to the live agent
//! send path: each goal-run tick is an autonomous agent turn driven through
//! `send_message_with_sender_context` with the reserved `"autonomous"` channel
//! sentinel (same RBAC carve-out as the continuous / cron background loops).
//!
//! These are inherent helpers; the `KernelApi` trait methods (`start_goal_run`
//! etc.) delegate here so the HTTP layer can reach them through
//! `Arc<dyn KernelApi>`.

use librefang_channels::types::SenderContext;
use librefang_types::agent::AgentId;
use librefang_types::goal::{GoalId, GoalRunState, DEFAULT_GOAL_MAX_ITERATIONS};

use super::{LibreFangKernel, SYSTEM_CHANNEL_AUTONOMOUS};
use crate::MemorySubsystemApi;

impl LibreFangKernel {
    /// Start an autonomous run that drives `agent_id` toward `goal_id`.
    ///
    /// Each tick is a full agent turn; the runner parses the agent's reply for
    /// `GOAL_PROGRESS:` / `GOAL_DONE` markers and updates the goal until it is
    /// complete, the iteration cap (`max_iterations`, default
    /// [`DEFAULT_GOAL_MAX_ITERATIONS`]) is reached, an operator stops it, or the
    /// kernel shuts down.
    pub fn goal_run_start(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
    ) -> bool {
        let max = max_iterations.unwrap_or(DEFAULT_GOAL_MAX_ITERATIONS).max(1);
        let substrate = self.substrate_ref().clone();

        // The tick closure drives a real agent turn, which needs an owned
        // `Arc<LibreFangKernel>`. Upgrade the self-handle (set right after the
        // kernel is wrapped in `Arc` at boot).
        let kernel = match self.self_handle.get().and_then(|w| w.upgrade()) {
            Some(k) => k,
            None => {
                tracing::warn!(%goal_id, "Cannot start goal run: kernel self-handle unset");
                return false;
            }
        };

        let send = move |aid: AgentId, msg: String| {
            let k = kernel.clone();
            async move {
                // Trusted internal system path — reuse the autonomous-channel
                // sentinel so the RBAC resolver applies the system carve-out
                // (see background_lifecycle.rs).
                let sender = goal_tick_sender_context(aid, goal_id, SYSTEM_CHANNEL_AUTONOMOUS);
                match k.send_message_with_sender_context(aid, &msg, &sender).await {
                    Ok(r) => Ok(r.response),
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        self.workflows
            .goal_runner
            .start(goal_id, agent_id, max, substrate, send)
    }

    /// Stop an active goal run. Returns whether a run was stopped.
    ///
    /// Terminal: discards any resume checkpoint, so starting the goal again
    /// begins from iteration 0. Use [`Self::goal_run_pause`] to suspend a run
    /// that should later continue where it left off.
    pub fn goal_run_stop(&self, goal_id: GoalId) -> bool {
        self.workflows.goal_runner.stop(goal_id)
    }

    /// Pause an active goal run, checkpointing its iteration count and
    /// progress. Returns whether a live run was signalled.
    ///
    /// The loop finishes the turn it is on before checkpointing and exiting
    /// in [`librefang_types::goal::GoalRunPhase::Paused`], so a `true` return
    /// means the pause was accepted, not that the run has already stopped —
    /// poll [`Self::goal_run_status`] for the phase to reach `Paused`.
    pub fn goal_run_pause(&self, goal_id: GoalId) -> bool {
        self.workflows.goal_runner.pause(goal_id)
    }

    /// Resume a previously-paused goal run from its checkpoint.
    ///
    /// Identical to [`Self::goal_run_start`] — `GoalRunner::start` auto-detects
    /// and resumes from a pause checkpoint when one exists, so this is the
    /// same start path. Callers that want to refuse a resume when there is no
    /// checkpoint (rather than silently starting a fresh run) should check
    /// [`Self::goal_run_status`] for [`librefang_types::goal::GoalRunPhase::Paused`]
    /// before calling.
    pub fn goal_run_resume(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
    ) -> bool {
        self.goal_run_start(goal_id, agent_id, max_iterations)
    }

    /// Snapshot the observable state of a goal's run, if one is active.
    pub fn goal_run_status(&self, goal_id: GoalId) -> Option<GoalRunState> {
        self.workflows.goal_runner.state(goal_id)
    }

    /// Recover goal runs interrupted by a prior crash or restart.
    ///
    /// Boot calls this once, mirroring the workflow stale-recovery sweep:
    /// persisted runs still in `Running` phase and older than `stale_timeout`
    /// are demoted to `Stopped` ("Interrupted by daemon restart"). Runs are not
    /// auto-resumed — an in-flight LLM call cannot be replayed. Returns the
    /// recovered goal ids.
    pub fn recover_stale_goal_runs(&self, stale_timeout: std::time::Duration) -> Vec<GoalId> {
        self.workflows.goal_runner.recover_stale_runs(stale_timeout)
    }
}

/// Build the [`SenderContext`] a goal-run tick is dispatched with.
///
/// ## Why `chat_id` carries the goal id
///
/// `send_message_full`'s channel branch derives the session as
/// `SessionId::for_sender_scope(agent, channel, chat_id)`, which collapses to
/// `for_channel(agent, "autonomous")` when `chat_id` is absent. Every goal of
/// a given agent would then resolve to one single session: two goals running
/// concurrently would interleave their prompts into one conversation history,
/// and each would read back the other's turns as its own context.
///
/// Scoping by goal id splits them without costing prompt-cache reuse: cache
/// reuse depends on consecutive turns of *one* goal sharing a session prefix,
/// and they still do, since the scope is a function of the goal rather than
/// of the tick. What changes is only that a *different* goal no longer lands
/// on that same id.
fn goal_tick_sender_context(
    agent_id: AgentId,
    goal_id: GoalId,
    display_name: &str,
) -> SenderContext {
    SenderContext {
        channel: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
        user_id: agent_id.to_string(),
        chat_id: Some(goal_id.to_string()),
        display_name: display_name.to_string(),
        is_internal_system: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod goal_session_scope_tests {
    use super::*;
    use librefang_types::agent::SessionId;

    /// Reproduce the session id `send_message_full` derives for a goal tick.
    /// Mirrors the channel branch of `messaging.rs::send_message_full_inner`
    /// verbatim — `resolve_scope_channel` then `SessionId::for_sender_scope`
    /// — so this asserts against the real derivation rather than a local
    /// re-statement of it.
    fn derived_session_id(ctx: &SenderContext, agent_id: AgentId) -> SessionId {
        let scope = LibreFangKernel::resolve_scope_channel(&ctx.channel, ctx.is_internal_system);
        SessionId::for_sender_scope(agent_id, &scope, ctx.chat_id.as_deref())
    }

    /// Two loop-mode goals driven by the SAME agent must not share a session.
    ///
    /// Before the fix every goal tick synthesized `chat_id: None`, collapsing
    /// to `SessionId::for_channel(agent, "autonomous")` — so two concurrent
    /// goal runs interleaved their prompts into one conversation history.
    #[test]
    fn two_goals_on_one_agent_do_not_share_a_session() {
        let agent = AgentId::new();
        let goal_a = GoalId::new();
        let goal_b = GoalId::new();

        let ctx_a = goal_tick_sender_context(agent, goal_a, SYSTEM_CHANNEL_AUTONOMOUS);
        let ctx_b = goal_tick_sender_context(agent, goal_b, SYSTEM_CHANNEL_AUTONOMOUS);

        assert_ne!(
            derived_session_id(&ctx_a, agent),
            derived_session_id(&ctx_b, agent),
            "two goals of the same agent resolved to one session — their \
             prompts interleave in a single conversation history"
        );
    }

    /// Isolation is per GOAL, not per tick: every tick of one goal must keep
    /// landing on the same session, or turn-to-turn context and the provider
    /// prompt cache are both destroyed mid-run.
    #[test]
    fn repeated_ticks_of_one_goal_share_its_session() {
        let agent = AgentId::new();
        let goal = GoalId::new();

        let first = goal_tick_sender_context(agent, goal, SYSTEM_CHANNEL_AUTONOMOUS);
        let second = goal_tick_sender_context(agent, goal, SYSTEM_CHANNEL_AUTONOMOUS);

        assert_eq!(
            derived_session_id(&first, agent),
            derived_session_id(&second, agent),
        );
    }

    /// The same goal id under two different agents stays separate — the agent
    /// dimension is still part of the key.
    #[test]
    fn one_goal_across_two_agents_does_not_share_a_session() {
        let goal = GoalId::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        let ctx_a = goal_tick_sender_context(agent_a, goal, SYSTEM_CHANNEL_AUTONOMOUS);
        let ctx_b = goal_tick_sender_context(agent_b, goal, SYSTEM_CHANNEL_AUTONOMOUS);

        assert_ne!(
            derived_session_id(&ctx_a, agent_a),
            derived_session_id(&ctx_b, agent_b),
        );
    }
}
