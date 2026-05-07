//! Skill workshop (#3328) — passive after-turn capture of reusable
//! workflows from successful agent-user interactions.
//!
//! # Wiring
//!
//! 1. `LibreFangKernel::set_self_handle` registers a
//!    [`SkillWorkshopTurnEndHook`] on the runtime's `AgentLoopEnd`
//!    event (mirrors `auto_dream`'s registration site, which sits in
//!    the same set_self_handle block — keep both registrations
//!    together so the bootstrap order stays obvious).
//! 2. Each non-fork turn fires the hook. The hook does cheap
//!    synchronous gating (event type, `is_fork`, kernel weak upgrade)
//!    and dispatches to a tokio task so the agent loop's return path
//!    is never blocked on FS / SQL.
//! 3. The detached task re-checks the per-agent
//!    [`SkillWorkshopConfig`](librefang_types::agent::SkillWorkshopConfig),
//!    pulls the agent's most recent session, runs the heuristic
//!    scanners, optionally consults the auxiliary LLM, and persists
//!    accepted candidates under `<home_dir>/skills/pending/<agent>/`.
//!
//! # Default-off philosophy
//!
//! The whole subsystem is off by default. An agent only sees capture
//! when its `agent.toml` carries `[skill_workshop] enabled = true`, and
//! even then candidates land in `pending/` unless `approval_policy =
//! "auto"`. The `auto` policy still gates writes through the same
//! prompt-injection scanner that protects marketplace skills — see
//! [`storage::save_candidate`].

pub mod candidate;
pub mod heuristic;
pub mod llm_review;
pub mod storage;

pub use candidate::{
    truncate_excerpt, CandidateSkill, CaptureSource, Provenance, PROVENANCE_EXCERPT_MAX_CHARS,
};
pub use heuristic::HeuristicHit;
pub use llm_review::ReviewDecision;
pub use storage::WorkshopError;

use crate::kernel::LibreFangKernel;
use librefang_runtime::aux_client::AuxClient;
use librefang_runtime::hooks::{HookContext, HookHandler};
use librefang_types::agent::{
    AgentId, AgentManifest, ApprovalPolicy, HookEvent, ReviewMode, Role, SessionId,
    SkillWorkshopConfig,
};
use librefang_types::config::AuxTask;
use librefang_types::message::{ContentBlock, MessageContent};
use std::sync::{Arc, Weak};
use tracing::{debug, warn};
use uuid::Uuid;

/// `HookHandler` that wires the runtime's `AgentLoopEnd` event into the
/// skill workshop capture pipeline.
///
/// Holds a `Weak<LibreFangKernel>` so the hook can survive kernel
/// shutdown without dangling references — `upgrade()` returning `None`
/// is the signal to no-op.
pub struct SkillWorkshopTurnEndHook {
    kernel: Weak<LibreFangKernel>,
}

impl SkillWorkshopTurnEndHook {
    pub fn new(kernel: Weak<LibreFangKernel>) -> Self {
        Self { kernel }
    }
}

impl HookHandler for SkillWorkshopTurnEndHook {
    fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
        // Only act on AgentLoopEnd. The registry already filters on
        // event type so this is defensive.
        if ctx.event != HookEvent::AgentLoopEnd {
            return Ok(());
        }
        // Skip fork turns: they're ephemeral runs (auto-dream, planning
        // forks, …) and any "user message" is synthetic prompting, not
        // a teaching signal. Mirrors auto_dream's identical check.
        if ctx
            .data
            .get("is_fork")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(());
        }
        let Some(kernel) = self.kernel.upgrade() else {
            return Ok(());
        };
        let Ok(uuid) = Uuid::parse_str(ctx.agent_id) else {
            debug!(
                agent_id = %ctx.agent_id,
                "skill_workshop: AgentLoopEnd hook saw non-UUID agent_id, skipping",
            );
            return Ok(());
        };
        let agent_id = AgentId(uuid);

        // Cheap pre-filter: if config says off, skip the spawn entirely.
        // The detached task re-checks (mirroring auto_dream) since the
        // operator could flip the toggle in the microseconds between
        // pre-filter and the task being scheduled.
        match read_workshop_config(&kernel, agent_id) {
            Some(cfg) if cfg.enabled && cfg.auto_capture && cfg.max_pending > 0 => {}
            _ => return Ok(()),
        }

        crate::supervised_spawn::spawn_supervised(
            "skill_workshop_capture",
            run_capture(kernel, agent_id),
        );
        Ok(())
    }
}

// ── Detached capture pipeline ─────────────────────────────────────────

/// Public for direct invocation from tests / CLI; the `Hook` wires the
/// async path on every non-fork turn.
pub async fn run_capture(kernel: Arc<LibreFangKernel>, agent_id: AgentId) {
    if kernel.supervisor.is_shutting_down() {
        return;
    }
    let Some(cfg) = read_workshop_config(&kernel, agent_id) else {
        return;
    };
    if !cfg.enabled || !cfg.auto_capture || cfg.max_pending == 0 {
        return;
    }

    let recent = match load_recent_turn(&kernel, agent_id) {
        Some(r) => r,
        None => {
            debug!(%agent_id, "skill_workshop: no recent session for agent");
            return;
        }
    };

    let mut hits: Vec<HeuristicHit> = Vec::new();
    if let Some(h) = heuristic::extract_explicit_instruction(&recent.last_user_message) {
        hits.push(h);
    }
    if let Some(h) = heuristic::extract_user_correction(
        recent.prev_assistant_response.as_deref(),
        &recent.last_user_message,
    ) {
        hits.push(h);
    }
    if let Some(h) = heuristic::extract_repeated_tool_pattern(&recent.recent_tool_names) {
        hits.push(h);
    }

    if hits.is_empty() {
        return;
    }

    for hit in hits {
        capture_one(&kernel, agent_id, &cfg, &recent.session_id, hit).await;
    }
}

async fn capture_one(
    kernel: &Arc<LibreFangKernel>,
    agent_id: AgentId,
    cfg: &SkillWorkshopConfig,
    session_id: &SessionId,
    hit: HeuristicHit,
) {
    let mut accepted_hit = match cfg.review_mode {
        ReviewMode::None => return,
        ReviewMode::Heuristic => hit,
        ReviewMode::ThresholdLlm => match run_llm_review(kernel, &hit).await {
            ReviewDecision::Accept {
                refined_name,
                refined_description,
                ..
            } => apply_refinements(hit, refined_name, refined_description),
            ReviewDecision::Reject { reason } => {
                debug!(%agent_id, reason, "skill_workshop: LLM review rejected candidate");
                return;
            }
            ReviewDecision::Indeterminate { reason } => {
                debug!(
                    %agent_id,
                    reason,
                    "skill_workshop: LLM review indeterminate; falling back to heuristic verdict"
                );
                hit
            }
        },
        ReviewMode::Both => {
            // `Both` accepts if either gate accepts, refines via the LLM
            // when it spoke, and falls back to the heuristic on
            // indeterminate. Reject only when the LLM explicitly
            // rejects — heuristic alone has already accepted.
            match run_llm_review(kernel, &hit).await {
                ReviewDecision::Accept {
                    refined_name,
                    refined_description,
                    ..
                } => apply_refinements(hit, refined_name, refined_description),
                ReviewDecision::Reject { reason } => {
                    debug!(%agent_id, reason, "skill_workshop: LLM rejected candidate under Both");
                    return;
                }
                ReviewDecision::Indeterminate { .. } => hit,
            }
        }
    };
    let now = chrono::Utc::now();
    let id = Uuid::new_v4().to_string();
    accepted_hit.user_message_excerpt =
        candidate::truncate_excerpt(&accepted_hit.user_message_excerpt);
    accepted_hit.assistant_response_excerpt = accepted_hit
        .assistant_response_excerpt
        .as_deref()
        .map(candidate::truncate_excerpt);

    let candidate = CandidateSkill {
        id,
        agent_id: agent_id.to_string(),
        session_id: Some(session_id.to_string()),
        captured_at: now,
        source: accepted_hit.source.clone(),
        name: accepted_hit.name.clone(),
        description: accepted_hit.description.clone(),
        prompt_context: accepted_hit.prompt_context.clone(),
        provenance: Provenance {
            user_message_excerpt: accepted_hit.user_message_excerpt,
            assistant_response_excerpt: accepted_hit.assistant_response_excerpt,
            turn_index: 0, // best-effort: real turn index would require session walking we already did, but we don't carry it through. Not load-bearing — provenance is descriptive, not structural.
        },
    };

    let skills_root = kernel.home_dir_boot.join("skills");
    match cfg.approval_policy {
        ApprovalPolicy::Pending => {
            match storage::save_candidate(&skills_root, &candidate, cfg.max_pending) {
                Ok(true) => {
                    debug!(%agent_id, id = %candidate.id, "skill_workshop: pending candidate written")
                }
                Ok(false) => {}
                Err(WorkshopError::SecurityBlocked(msg)) => {
                    warn!(%agent_id, msg, "skill_workshop: candidate blocked by security scan");
                }
                Err(e) => {
                    warn!(%agent_id, error = %e, "skill_workshop: failed to write pending candidate");
                }
            }
        }
        ApprovalPolicy::Auto => {
            // Auto policy promotes directly to active. We still write
            // the pending file first — that gives the security scan a
            // chance to fail loudly before evolution::create_skill
            // touches the active tree, and leaves an audit trail in
            // case the auto-write surprises the operator.
            let written = match storage::save_candidate(&skills_root, &candidate, cfg.max_pending) {
                Ok(b) => b,
                Err(WorkshopError::SecurityBlocked(msg)) => {
                    warn!(%agent_id, msg, "skill_workshop: auto candidate blocked by security scan");
                    return;
                }
                Err(e) => {
                    warn!(%agent_id, error = %e, "skill_workshop: failed to stage auto candidate");
                    return;
                }
            };
            if !written {
                return;
            }
            match storage::approve_candidate(&skills_root, &skills_root, &candidate.id) {
                Ok(_) => {
                    debug!(%agent_id, id = %candidate.id, "skill_workshop: auto-promoted candidate")
                }
                Err(e) => {
                    warn!(%agent_id, error = %e, "skill_workshop: auto promotion failed; candidate left in pending/");
                }
            }
        }
    }
}

fn apply_refinements(
    mut hit: HeuristicHit,
    refined_name: Option<String>,
    refined_description: Option<String>,
) -> HeuristicHit {
    if let Some(name) = refined_name {
        // Defensive sanitisation: validate_name in evolution.rs is
        // strict, and we'd rather drop a malformed refinement than
        // poison the candidate. Letting the heuristic name win on
        // garbage refinement is a graceful degradation.
        let trimmed = name.trim();
        if !trimmed.is_empty() && trimmed.len() <= 64 {
            hit.name = trimmed.to_string();
        }
    }
    if let Some(desc) = refined_description {
        let trimmed = desc.trim();
        if !trimmed.is_empty() && trimmed.len() <= 200 {
            hit.description = trimmed.to_string();
        }
    }
    hit
}

async fn run_llm_review(kernel: &Arc<LibreFangKernel>, hit: &HeuristicHit) -> ReviewDecision {
    let aux: Arc<AuxClient> = kernel.aux_client.load_full();
    let resolution = aux.resolve(AuxTask::SkillReview);
    if resolution.used_primary {
        // Primary fallback for SkillReview means the user has no aux
        // chain configured AND no default-chain providers credentialled.
        // Issuing the review against the agent's primary model would
        // burn user budget on every turn — skip and treat as
        // indeterminate so the heuristic verdict carries.
        return ReviewDecision::Indeterminate {
            reason: "no auxiliary driver configured for skill_review".to_string(),
        };
    }
    // Use a known cheap-tier alias as the model name; aux drivers
    // expand it provider-side.
    let model = resolution
        .resolved
        .first()
        .map(|(_, m)| m.clone())
        .unwrap_or_else(|| "haiku".to_string());
    llm_review::review_candidate(resolution.driver, &model, hit).await
}

/// Per-turn snapshot of the conversation needed by the heuristic
/// scanners. Filled by [`load_recent_turn`].
#[derive(Debug, Clone)]
struct RecentTurn {
    session_id: SessionId,
    last_user_message: String,
    /// Assistant turn that came BEFORE `last_user_message` — used by
    /// the user-correction scanner to ground the correction in
    /// concrete prior behaviour.
    prev_assistant_response: Option<String>,
    /// Tool names from the last 30 messages, oldest first.
    recent_tool_names: Vec<String>,
}

/// Look up an agent's manifest and clone its workshop config out.
/// Returns `None` for missing agents.
fn read_workshop_config(
    kernel: &Arc<LibreFangKernel>,
    agent_id: AgentId,
) -> Option<SkillWorkshopConfig> {
    kernel
        .agent_registry()
        .get(agent_id)
        .map(|entry: librefang_types::agent::AgentEntry| {
            let _: &AgentManifest = &entry.manifest;
            entry.manifest.skill_workshop
        })
}

/// Pull the most recently touched session for `agent_id` and walk it
/// for the data the heuristic scanners need.
fn load_recent_turn(kernel: &Arc<LibreFangKernel>, agent_id: AgentId) -> Option<RecentTurn> {
    use librefang_memory::SessionStore;

    let session_ids = kernel.memory.get_agent_session_ids(agent_id).ok()?;
    let session_id = *session_ids.first()?;
    let session = kernel.memory.get_session(session_id).ok().flatten()?;

    // Walk newest-last (the natural append order in librefang sessions).
    let messages = &session.messages;
    let mut last_user_idx: Option<usize> = None;
    for (i, m) in messages.iter().enumerate().rev() {
        if m.role == Role::User {
            last_user_idx = Some(i);
            break;
        }
    }
    let last_user_idx = last_user_idx?;
    let last_user_message = extract_text(&messages[last_user_idx].content);

    let prev_assistant_response: Option<String> = messages[..last_user_idx]
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| extract_text(&m.content));

    let recent_tool_names = collect_recent_tool_names(messages, 30);

    Some(RecentTurn {
        session_id,
        last_user_message,
        prev_assistant_response,
        recent_tool_names,
    })
}

/// Concatenate plain-text portions of a message's content into a single
/// string. ToolUse / ToolResult / Image / Thinking blocks are omitted —
/// the heuristics only look at conversational text.
fn extract_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(s) => s.clone(),
        MessageContent::Blocks(blocks) => {
            let mut out = String::new();
            for b in blocks {
                if let ContentBlock::Text { text, .. } = b {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
            out
        }
    }
}

/// Collect tool names from `ToolUse` blocks across the last `window`
/// messages, oldest first. Used by the repeated-tool-pattern scanner.
fn collect_recent_tool_names(
    messages: &[librefang_types::message::Message],
    window: usize,
) -> Vec<String> {
    let start = messages.len().saturating_sub(window);
    let mut out = Vec::new();
    for m in &messages[start..] {
        if m.role != Role::Assistant {
            continue;
        }
        if let MessageContent::Blocks(blocks) = &m.content {
            for b in blocks {
                if let ContentBlock::ToolUse { name, .. } = b {
                    out.push(name.clone());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::Message;

    #[test]
    fn extract_text_handles_text_variant() {
        let c = MessageContent::Text("hello".to_string());
        assert_eq!(extract_text(&c), "hello");
    }

    #[test]
    fn extract_text_concatenates_text_blocks_skipping_others() {
        let c = MessageContent::Blocks(vec![
            ContentBlock::Text {
                text: "one".to_string(),
                provider_metadata: None,
            },
            ContentBlock::ToolUse {
                id: "1".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({}),
                provider_metadata: None,
            },
            ContentBlock::Text {
                text: "two".to_string(),
                provider_metadata: None,
            },
        ]);
        assert_eq!(extract_text(&c), "one\ntwo");
    }

    #[test]
    fn collect_recent_tool_names_walks_only_assistant_turns_in_window() {
        let mut messages: Vec<Message> = Vec::new();
        // Old turn that should be outside the window.
        for _ in 0..50 {
            messages.push(Message::user("noise"));
        }
        // Last 5 messages contain assistant tool uses.
        for tool in ["read", "edit", "shell", "edit", "shell"] {
            messages.push(Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: "x".to_string(),
                    name: tool.to_string(),
                    input: serde_json::json!({}),
                    provider_metadata: None,
                }]),
                pinned: false,
                timestamp: None,
            });
        }
        let names = collect_recent_tool_names(&messages, 30);
        // Only the trailing 5 assistant tool-uses should have been captured.
        assert_eq!(names, vec!["read", "edit", "shell", "edit", "shell"]);
    }

    #[test]
    fn apply_refinements_keeps_heuristic_on_empty_or_oversized() {
        let base = HeuristicHit {
            name: "orig_name".to_string(),
            description: "orig desc".to_string(),
            prompt_context: "body".to_string(),
            source: CaptureSource::ExplicitInstruction {
                trigger: "from now on".to_string(),
            },
            user_message_excerpt: "u".to_string(),
            assistant_response_excerpt: None,
        };
        let refined = apply_refinements(base.clone(), Some("".to_string()), Some("".to_string()));
        assert_eq!(refined.name, "orig_name");
        assert_eq!(refined.description, "orig desc");
        let too_long = "x".repeat(300);
        let refined = apply_refinements(base.clone(), Some("y".repeat(100)), Some(too_long));
        assert_eq!(refined.name, "orig_name", "oversized name dropped");
        assert_eq!(refined.description, "orig desc", "oversized desc dropped");
        let refined = apply_refinements(
            base,
            Some("good_name".to_string()),
            Some("good description".to_string()),
        );
        assert_eq!(refined.name, "good_name");
        assert_eq!(refined.description, "good description");
    }
}
