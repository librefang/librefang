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
use librefang_skills::evolution::load_installed_skill_from_disk;
use librefang_types::agent::{AgentId, SkillWorkshopConfig};
use librefang_types::goal::{
    goals_storage_agent_id, Goal, GoalId, GoalRunState, GOALS_STORAGE_KEY,
};

use super::{LibreFangKernel, SYSTEM_CHANNEL_AUTONOMOUS};
use crate::registry::AgentRegistry;
use crate::MemorySubsystemApi;

impl LibreFangKernel {
    /// Start an autonomous run that drives `agent_id` toward `goal_id`.
    ///
    /// Each tick is a full agent turn; the runner parses the agent's reply for
    /// `GOAL_PROGRESS:` / `GOAL_DONE` markers and updates the goal until it is
    /// complete, the iteration cap is reached, an operator stops it, or the
    /// kernel shuts down.
    ///
    /// `max_iterations` stays an `Option` all the way down to [`crate::goal_runner::GoalRunner::start`], which is the only layer that knows whether this call is resuming a paused run.
    /// Substituting [`librefang_types::goal::DEFAULT_GOAL_MAX_ITERATIONS`] here would overwrite the cap that run was already under, because by then a `None` is indistinguishable from an operator asking for the default.
    #[allow(clippy::too_many_arguments)]
    pub fn goal_run_start(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
        loop_engineering: bool,
        verify_agent_id: Option<AgentId>,
        verify_max_retries: Option<u32>,
        evaluator_model: Option<String>,
    ) -> bool {
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

        let send_kernel = kernel.clone();
        let send = move |aid: AgentId, msg: String| {
            let k = send_kernel.clone();
            async move {
                // Trusted internal system path — reuse the autonomous-channel
                // sentinel so the RBAC resolver applies the system carve-out
                // (see background_lifecycle.rs).
                let sender = goal_tick_sender_context(aid, goal_id);
                match k.send_message_with_sender_context(aid, &msg, &sender).await {
                    Ok(r) => Ok(r.response),
                    Err(e) => Err(e.to_string()),
                }
            }
        };

        // Completion judge. A one-shot call on a model of the operator's
        // choosing, deliberately NOT a turn on the goal's own agent: routing it
        // there would both bill a full agent turn and put the worker back in
        // charge of grading itself. With no model configured the runner never
        // calls this, so the arm returning `Ok(false)` exists only to give the
        // closure a single concrete type.
        let eval_kernel = kernel.clone();
        let eval_model = evaluator_model.clone();
        let evaluate = move |goal_description: String, output: String| {
            let k = eval_kernel.clone();
            let eval_model = eval_model.clone();
            async move {
                let Some(model) = eval_model else {
                    return Ok(false);
                };
                let prompt = format!(
                    "You are judging whether a goal has been achieved. Read the goal and \
                     the worker's latest output, then answer with the single word YES or \
                     NO — YES only if the goal is fully achieved, NO if any work remains.\
                     \n\nGOAL: {goal_description}\n\nLATEST OUTPUT:\n{output}\n\nAchieved?"
                );
                k.one_shot_llm_call(&model, &prompt)
                    .await
                    .map(|reply| evaluator_reply_is_yes(&reply))
            }
        };

        // Lessons the run captured become a skill *proposal*, so the next goal
        // can start from them instead of rediscovering them — but only once a
        // human has read them. They go into the skill workshop's `pending/`
        // queue (#3328), the same place every other machine-proposed skill
        // waits, rather than straight into the installed skills directory.
        //
        // The workshop's cap / TTL settings are read once here, at run start:
        // the hook fires on a background task after the run ends, and reaching
        // back into the registry from there would mean carrying a kernel handle
        // for two `u32`s.
        let skills_dir = self.home_dir().join("skills");
        let workshop = resolve_workshop_config(&self.agents.registry, agent_id);
        let goal_title = self
            .goal_by_id(goal_id)
            .map(|g| g.title)
            .unwrap_or_else(|| format!("Goal {goal_id}"));
        let on_learnings = move |learnings: Vec<String>| {
            queue_learnings_as_pending_skill(
                &skills_dir,
                agent_id,
                &workshop,
                goal_id,
                &goal_title,
                &learnings,
            );
        };

        self.workflows.goal_runner.start(
            goal_id,
            agent_id,
            max_iterations,
            substrate,
            send,
            on_learnings,
            evaluate,
            loop_engineering,
            verify_agent_id,
            verify_max_retries,
            evaluator_model,
        )
    }

    /// Load a persisted [`Goal`] by id from the shared goals document.
    pub fn goal_by_id(&self, goal_id: GoalId) -> Option<Goal> {
        let Ok(Some(serde_json::Value::Array(arr))) = self
            .substrate_ref()
            .structured_get(goals_storage_agent_id(), GOALS_STORAGE_KEY)
        else {
            return None;
        };
        let target = goal_id.to_string();
        arr.into_iter()
            .find(|g| g.get("id").and_then(|v| v.as_str()) == Some(target.as_str()))
            .and_then(|g| serde_json::from_value(g).ok())
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
    ///
    /// A `None` `max_iterations` restores the cap the paused run was under; an explicit value re-budgets it.
    /// See [`crate::goal_runner::GoalRunner::start`] for why that precedence is resolved down there rather than here.
    ///
    /// The loop-engineering arguments are taken from the caller for the same
    /// reason [`Self::goal_run_start`] takes them: the configuration lives on
    /// the goal document, and resolving it at the API boundary keeps one
    /// definition of where a verifier comes from rather than two.
    #[allow(clippy::too_many_arguments)]
    pub fn goal_run_resume(
        &self,
        goal_id: GoalId,
        agent_id: AgentId,
        max_iterations: Option<u32>,
        loop_engineering: bool,
        verify_agent_id: Option<AgentId>,
        verify_max_retries: Option<u32>,
        evaluator_model: Option<String>,
    ) -> bool {
        self.goal_run_start(
            goal_id,
            agent_id,
            max_iterations,
            loop_engineering,
            verify_agent_id,
            verify_max_retries,
            evaluator_model,
        )
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

/// Read an evaluator's free-text reply as achieved / not achieved.
///
/// The model is asked for a bare YES or NO and routinely wraps it in prose, so
/// the verdict has to be found rather than compared. Substring matching is the
/// wrong tool: "NOTHING left to do" and "ready to ANNOUNCE" both contain "no".
/// Split on non-letters and look for a standalone `YES` / `NO` token instead.
///
/// An explicit `NO` anywhere beats a `YES`, and a reply that reaches no verdict
/// at all is not achieved — the conservative direction, since the cost of a
/// false "not yet" is one more iteration and the cost of a false "done" is a
/// goal closed on unfinished work.
fn evaluator_reply_is_yes(reply: &str) -> bool {
    let mut saw_yes = false;
    let mut saw_no = false;
    for token in reply.split(|c: char| !c.is_ascii_alphabetic()) {
        match token.to_ascii_uppercase().as_str() {
            "YES" => saw_yes = true,
            "NO" => saw_no = true,
            _ => {}
        }
    }
    saw_yes && !saw_no
}

/// Turn a goal title into a skill-name slug.
///
/// `create_skill` accepts `[a-z0-9_-]` starting alphanumeric, up to 64 chars,
/// so a title in a non-Latin script or one made entirely of punctuation slugs
/// down to nothing. The goal id is appended in all cases: it keeps the name
/// unique across goals that share a title, and it is what the name falls back
/// to when the slug is empty.
fn learned_skill_name(goal_id: GoalId, goal_title: &str) -> String {
    const MAX_SLUG: usize = 24;
    let mut slug = String::with_capacity(MAX_SLUG);
    let mut last_was_dash = false;
    for c in goal_title.chars() {
        if slug.len() >= MAX_SLUG {
            break;
        }
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !slug.is_empty() && !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    // The id fragment is hex, so the name always starts alphanumeric even when
    // the slug contributes nothing.
    let id_fragment: String = goal_id
        .to_string()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if slug.is_empty() {
        format!("goal-learned-{id_fragment}")
    } else {
        format!("goal-learned-{slug}-{id_fragment}")
    }
}

/// Render a run's lessons as a skill body.
fn learned_skill_body(goal_title: &str, learnings: &[String]) -> String {
    let mut body =
        String::from("## What this is\n\nLessons captured while autonomously pursuing the goal \"");
    body.push_str(goal_title);
    body.push_str("\". They are the run's own findings, not vetted guidance — weigh them as such.\n\n## Lessons\n\n");
    for (i, l) in learnings.iter().enumerate() {
        body.push_str(&format!("{}. {l}\n", i + 1));
    }
    body
}

/// Sentinel trigger tag identifying a candidate that came from a goal run's
/// `GOAL_LEARNED:` markers rather than from a conversation turn.
///
/// The workshop's non-conversational producers reuse
/// [`crate::skill_workshop::candidate::CaptureSource::ExplicitInstruction`]
/// with a sentinel rather than growing a variant per producer — the background
/// skill reviewer already does this with `auto_evolve_reviewer`.
/// The tag is what a reviewer sees in `librefang skill pending show`, so it names the producer, not the shape.
const LEARNED_CAPTURE_TRIGGER: &str = "goal_learned";

/// Whether a goal run's captured `GOAL_LEARNED:` lessons should be queued as
/// a pending skill draft at all.
///
/// The workshop is default-OFF and opted into per agent (`agent.toml:
/// [skill_workshop] enabled = true`) — see [`SkillWorkshopConfig::default`].
/// Without this gate a goal run queued a draft for every agent regardless of
/// that setting, because `on_learnings_captured` had no reason to read it:
/// nothing else in the goal-run path consults the workshop config, only the
/// approval-side CLI / API / dashboard do. `auto_capture` is checked too —
/// it is the independent "run the capture scan at all" toggle every other
/// automatic capture path in the workshop already gates on (see
/// `skill_workshop::mod.rs`).
fn should_queue_learnings(workshop: &SkillWorkshopConfig) -> bool {
    workshop.enabled && workshop.auto_capture
}

/// Resolve the skill-workshop config a goal run's learnings capture is
/// gated by. An agent absent from the registry — deleted, never spawned,
/// or simply mistyped — denies via [`SkillWorkshopConfig::default`]
/// (`enabled: false`) rather than assuming any particular default should
/// permit queuing for an id nobody registered.
fn resolve_workshop_config(registry: &AgentRegistry, agent_id: AgentId) -> SkillWorkshopConfig {
    registry
        .get(agent_id)
        .map(|e| e.manifest.skill_workshop)
        .unwrap_or_default()
}

/// Queue a run's lessons as a pending skill draft awaiting human approval.
///
/// The lessons are model-authored text an autonomous loop wrote about itself, so they go where every other machine-proposed skill goes: the workshop's `pending/` queue (#3328), promoted only by an explicit `librefang skill pending approve` / `POST /api/skills/pending/{id}/approve`.
/// Installing them directly would have handed a goal run the one power the workshop deliberately withholds — authoring a skill that loads itself into the next prompt with nobody having read it.
///
/// A goal can be run more than once, and by then the draft its first run produced may already be an installed skill.
/// That case is what [`crate::skill_workshop::candidate::CandidateKind::Update`] exists for: the draft targets the installed skill and approval routes through `evolution::update_skill` instead of failing on the name already existing and silently dropping everything the second run learned.
///
/// Nothing here is the durable record — that is the runner's own `goal_learnings_<id>` store entry, written before this is called.
/// A draft that is capped out, deduped, or rejected loses the agent a convenience, not the lessons.
fn queue_learnings_as_pending_skill(
    skills_dir: &std::path::Path,
    agent_id: AgentId,
    workshop: &SkillWorkshopConfig,
    goal_id: GoalId,
    goal_title: &str,
    learnings: &[String],
) {
    use crate::skill_workshop::candidate::{
        CandidateKind, CandidateSkill, CaptureSource, Provenance, PROVENANCE_EXCERPT_MAX_CHARS,
    };

    if !should_queue_learnings(workshop) || learnings.is_empty() {
        return;
    }
    let name = learned_skill_name(goal_id, goal_title);
    // A goal title is capped at 256 chars, so the description stays well
    // inside the 1024 `create_skill` enforces at approval time.
    let description = format!("Lessons captured while pursuing the goal: {goal_title}");
    let installed = load_installed_skill_from_disk(skills_dir, &name).ok();
    let candidate = CandidateSkill {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: agent_id.to_string(),
        session_id: None,
        captured_at: chrono::Utc::now(),
        source: CaptureSource::ExplicitInstruction {
            trigger: LEARNED_CAPTURE_TRIGGER.to_string(),
        },
        name: name.clone(),
        description,
        prompt_context: learned_skill_body(goal_title, learnings),
        provenance: Provenance {
            // The goal is the whole context this draft has; there is no
            // conversation turn behind it.
            user_message_excerpt: goal_title
                .chars()
                .take(PROVENANCE_EXCERPT_MAX_CHARS)
                .collect(),
            assistant_response_excerpt: None,
            turn_index: 0,
        },
        kind: if installed.is_some() {
            CandidateKind::Update
        } else {
            CandidateKind::Create
        },
        target_skill_id: installed.as_ref().map(|_| name.clone()),
        current_version: installed
            .as_ref()
            .map(|s| s.manifest.skill.version.clone())
            .filter(|v| !v.is_empty()),
        proposed_version: None,
    };

    match crate::skill_workshop::storage::save_candidate(
        skills_dir,
        &candidate,
        workshop.max_pending,
        workshop.max_pending_age_days,
    ) {
        Ok(true) => tracing::info!(%goal_id, skill = %name, count = learnings.len(),
                                   "Goal run: queued captured lessons as a pending skill draft for human approval"),
        Ok(false) => tracing::debug!(%goal_id, skill = %name,
                                     "Goal run: pending draft skipped (duplicate or max_pending=0)"),
        Err(e) => {
            // Most likely the prompt-injection scan rejecting model-authored
            // text, which is the scan doing its job. The lessons are still in
            // the runner's durable store either way.
            tracing::warn!(%goal_id, skill = %name, error = %e,
                           "Goal run: failed to queue captured lessons as a pending draft");
        }
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
fn goal_tick_sender_context(agent_id: AgentId, goal_id: GoalId) -> SenderContext {
    SenderContext {
        channel: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
        user_id: agent_id.to_string(),
        chat_id: Some(goal_id.to_string()),
        display_name: SYSTEM_CHANNEL_AUTONOMOUS.to_string(),
        is_internal_system: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workshop is default-OFF: a goal run must not queue a pending
    /// skill draft for an agent that never opted in.
    #[test]
    fn should_queue_learnings_is_false_by_default() {
        assert!(!should_queue_learnings(&SkillWorkshopConfig::default()));
    }

    #[test]
    fn should_queue_learnings_is_true_once_opted_in() {
        let workshop = SkillWorkshopConfig {
            enabled: true,
            ..SkillWorkshopConfig::default()
        };
        assert!(should_queue_learnings(&workshop));
    }

    /// `auto_capture` is the independent scan toggle within an enabled
    /// workshop — turning it off must still block queuing, matching every
    /// other automatic capture path.
    #[test]
    fn should_queue_learnings_is_false_when_auto_capture_is_off() {
        let workshop = SkillWorkshopConfig {
            enabled: true,
            auto_capture: false,
            ..SkillWorkshopConfig::default()
        };
        assert!(!should_queue_learnings(&workshop));
    }

    /// The half of the opt-in gate a pure predicate test can't reach: an
    /// agent that isn't in the registry at all (deleted, never spawned,
    /// mistyped id) must still deny queuing, not fall through to some other
    /// default that happens to allow it.
    #[test]
    fn resolve_workshop_config_denies_when_the_agent_is_absent_from_the_registry() {
        let registry = AgentRegistry::new();
        let config = resolve_workshop_config(&registry, AgentId::new());
        assert!(
            !should_queue_learnings(&config),
            "an agent absent from the registry must resolve to a denying config"
        );
    }

    /// The other half: a REGISTERED agent's own manifest setting is what
    /// gets read, not a hardcoded value — otherwise the denial above would
    /// be indistinguishable from the function ignoring the registry
    /// entirely and always returning a fixed config.
    #[test]
    fn resolve_workshop_config_reads_the_registered_agents_own_manifest() {
        use librefang_types::agent::{AgentEntry, AgentManifest};

        let registry = AgentRegistry::new();
        let agent_id = AgentId::new();
        registry
            .register(AgentEntry {
                id: agent_id,
                name: format!("workshop-test-{agent_id}"),
                manifest: AgentManifest {
                    skill_workshop: SkillWorkshopConfig {
                        enabled: true,
                        ..SkillWorkshopConfig::default()
                    },
                    ..AgentManifest::default()
                },
                ..AgentEntry::default()
            })
            .expect("registering a fresh agent must succeed");

        let config = resolve_workshop_config(&registry, agent_id);
        assert!(
            should_queue_learnings(&config),
            "a registered agent's own opted-in workshop config must be the one read"
        );
    }

    #[test]
    fn evaluator_verdict_reads_a_bare_yes_or_no() {
        assert!(evaluator_reply_is_yes("YES"));
        assert!(evaluator_reply_is_yes("yes"));
        assert!(!evaluator_reply_is_yes("NO"));
        assert!(!evaluator_reply_is_yes("no"));
    }

    #[test]
    fn evaluator_verdict_survives_the_prose_models_wrap_it_in() {
        assert!(evaluator_reply_is_yes("YES, the goal is fully achieved."));
        assert!(!evaluator_reply_is_yes("NO, more work is needed."));
        assert!(evaluator_reply_is_yes("The answer is: yes."));
    }

    /// Substring matching would read all of these as "NO".
    #[test]
    fn a_word_merely_containing_no_is_not_a_verdict() {
        assert!(evaluator_reply_is_yes(
            "YES. There is nothing more to do here."
        ));
        assert!(evaluator_reply_is_yes("YES - ready to announce."));
        assert!(evaluator_reply_is_yes("Yes, the report is now complete."));
    }

    /// Closing a goal on unfinished work costs more than one extra iteration,
    /// so ambiguity resolves to "not yet".
    #[test]
    fn an_ambiguous_or_absent_verdict_is_not_achieved() {
        assert!(!evaluator_reply_is_yes("Yes and no - NO, not done."));
        assert!(!evaluator_reply_is_yes("I am not sure."));
        assert!(!evaluator_reply_is_yes(""));
    }

    #[test]
    fn skill_name_slugs_the_goal_title() {
        let id = GoalId::new();
        let name = learned_skill_name(id, "Ship the Q3 Report!");
        assert!(
            name.starts_with("goal-learned-ship-the-q3-report-"),
            "got {name}"
        );
    }

    /// `create_skill` only accepts `[a-z0-9_-]` starting alphanumeric, and a
    /// title in a non-Latin script or made of punctuation slugs to nothing.
    #[test]
    fn skill_name_stays_valid_for_a_title_that_slugs_to_nothing() {
        for title in ["", "!!!", "目标", "---"] {
            let name = learned_skill_name(GoalId::new(), title);
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{title:?} produced an invalid skill name: {name}"
            );
            assert!(!name.ends_with('-'), "{title:?} produced {name}");
            assert!(name.len() <= 64, "{title:?} produced {name}");
        }
    }

    #[test]
    fn skill_name_is_bounded_for_a_very_long_title() {
        let name = learned_skill_name(GoalId::new(), &"extremely wordy title ".repeat(20));
        assert!(name.len() <= 64, "got {} chars: {name}", name.len());
    }

    #[test]
    fn skill_name_distinguishes_goals_that_share_a_title() {
        let a = learned_skill_name(GoalId::new(), "Same title");
        let b = learned_skill_name(GoalId::new(), "Same title");
        assert_ne!(a, b);
    }

    /// The opt-in gate lives inside this function (moved from its one
    /// production caller, `goal_run_start`'s `on_learnings` closure, so
    /// there is exactly one place to verify it): a disabled workshop must
    /// queue nothing, no matter how many lessons a run captured.
    #[test]
    fn queue_learnings_as_pending_skill_does_nothing_when_the_workshop_is_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = tmp.path();
        let agent = AgentId::new();
        let goal_id = GoalId::new();
        queue_learnings_as_pending_skill(
            skills,
            agent,
            &SkillWorkshopConfig::default(),
            goal_id,
            "Ship the report",
            &["Back off before retrying".to_string()],
        );

        assert!(
            crate::skill_workshop::storage::list_pending(skills, &agent.to_string())
                .unwrap()
                .is_empty(),
            "a disabled workshop must not queue a pending draft"
        );
    }

    /// A goal run is an autonomous loop, and the lessons it captures are
    /// model-authored text. Writing them straight into the installed skills
    /// directory would let an agent author a skill that loads itself into the
    /// next prompt with nobody having read it — the exact power the workshop
    /// (#3328) withholds by parking every machine-proposed skill in `pending/`.
    #[test]
    fn goal_lessons_wait_in_pending_instead_of_installing_themselves() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = tmp.path();
        let agent = AgentId::new();
        let goal_id = GoalId::new();
        let workshop = SkillWorkshopConfig {
            enabled: true,
            ..SkillWorkshopConfig::default()
        };
        queue_learnings_as_pending_skill(
            skills,
            agent,
            &workshop,
            goal_id,
            "Ship the report",
            &["Back off before retrying".to_string()],
        );

        let name = learned_skill_name(goal_id, "Ship the report");
        assert!(
            load_installed_skill_from_disk(skills, &name).is_err(),
            "a goal run must not install a skill without human approval"
        );

        let pending =
            crate::skill_workshop::storage::list_pending(skills, &agent.to_string()).unwrap();
        assert_eq!(pending.len(), 1, "the lessons must be queued for review");
        assert_eq!(pending[0].name, name);
        assert!(pending[0]
            .prompt_context
            .contains("Back off before retrying"));
    }

    /// The other half of the same boundary: approval is what installs the
    /// skill, and it is the only thing that does.
    #[test]
    fn approving_the_pending_lesson_is_what_installs_it() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = tmp.path();
        let agent = AgentId::new();
        let goal_id = GoalId::new();
        let workshop = SkillWorkshopConfig {
            enabled: true,
            ..SkillWorkshopConfig::default()
        };
        queue_learnings_as_pending_skill(
            skills,
            agent,
            &workshop,
            goal_id,
            "Ship the report",
            &["Back off before retrying".to_string()],
        );

        let pending =
            crate::skill_workshop::storage::list_pending(skills, &agent.to_string()).unwrap();
        let id = pending
            .first()
            .expect("a candidate must be waiting to approve")
            .id
            .clone();
        crate::skill_workshop::storage::approve_candidate(skills, skills, &id)
            .expect("approval must install the skill");

        let name = learned_skill_name(goal_id, "Ship the report");
        let installed = load_installed_skill_from_disk(skills, &name)
            .expect("the approved lesson must now be an installed skill");
        assert!(installed
            .manifest
            .prompt_context
            .as_deref()
            .unwrap_or_default()
            .contains("Back off before retrying"));
        assert!(
            crate::skill_workshop::storage::list_pending(skills, &agent.to_string())
                .unwrap()
                .is_empty(),
            "approval consumes the pending draft"
        );
    }

    #[test]
    fn skill_body_lists_every_lesson_under_the_goal_title() {
        let body = learned_skill_body(
            "Ship the report",
            &[
                "Back off before retrying".to_string(),
                "Cite sources".to_string(),
            ],
        );
        assert!(body.contains("Ship the report"));
        assert!(body.contains("1. Back off before retrying"));
        assert!(body.contains("2. Cite sources"));
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

        let ctx_a = goal_tick_sender_context(agent, goal_a);
        let ctx_b = goal_tick_sender_context(agent, goal_b);

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

        let first = goal_tick_sender_context(agent, goal);
        let second = goal_tick_sender_context(agent, goal);

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

        let ctx_a = goal_tick_sender_context(agent_a, goal);
        let ctx_b = goal_tick_sender_context(agent_b, goal);

        assert_ne!(
            derived_session_id(&ctx_a, agent_a),
            derived_session_id(&ctx_b, agent_b),
        );
    }
}
