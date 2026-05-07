//! Cron session compaction helpers — split out of mod.rs in #4713 phase 3
//! to keep the body editable. These were originally introduced by #4683
//! (`fix(cron): summarize-and-trim compaction mode for Persistent
//! sessions`); the file contains the four helper fns the cron tick body
//! and `kernel::tests` reference.

/// Compute how many messages to keep in the cron session after applying both
/// size caps, without mutating the slice.
///
/// Returns the number of messages (from the tail) that should survive. The
/// caller uses this to determine the split point for either plain-drain or
/// summarize-and-trim (#3693).
pub(crate) fn cron_compute_keep_count(
    messages: &[librefang_types::message::Message],
    max_messages: Option<usize>,
    max_tokens: Option<u64>,
) -> usize {
    use librefang_runtime::compactor::estimate_token_count;

    let n = messages.len();
    // Start with the full set; apply message-count cap first.
    let after_msg_cap = if let Some(max_msgs) = max_messages {
        n.min(max_msgs)
    } else {
        n
    };

    // Then apply token cap by trimming from the front.
    if let Some(max_tok) = max_tokens {
        // Linear scan from the cap downward (O(n) iterations × O(n) token estimation = O(n²);
        // acceptable for cron session sizes seen in practice).
        let mut keep = after_msg_cap;
        while keep > 0 {
            let start = n - keep;
            let est = estimate_token_count(&messages[start..], None, None);
            if est <= max_tok as usize {
                break;
            }
            keep -= 1;
        }
        keep
    } else {
        after_msg_cap
    }
}

/// Clamp the configured `cron_session_compaction_keep_recent` so that
/// `[summary] + tail` never exceeds the size cap.
///
/// `keep_count` is the result of `cron_compute_keep_count` — i.e. how many
/// messages the cap would allow on its own. After `SummarizeTrim`, the
/// session contains `1 + tail_size` messages, so `tail_size` must be at
/// most `keep_count - 1`. We also enforce a floor of `1` to keep at least
/// one message of context after the summary.
///
/// Without this clamp, e.g. `cron_session_max_messages = 5` and
/// `cron_session_compaction_keep_recent = 8` would yield `1 + 8 = 9`
/// messages on the first fire (cap violated) and on subsequent fires
/// `try_summarize_trim` would summarize a single message into a single
/// summary, burning aux LLM calls without ever shrinking the session
/// (#3693 review feedback on PR #4683).
pub(crate) fn cron_clamp_keep_recent(keep_recent_cfg: usize, keep_count: usize) -> usize {
    keep_recent_cfg.min(keep_count.saturating_sub(1)).max(1)
}

/// Decide which compaction mode to actually apply for this cron fire.
///
/// `SummarizeTrim` with `keep_count < 2` would write `[summary, last_msg]`
/// (always 2 messages) into a session whose cap permits at most
/// `keep_count` messages. The next fire would re-trigger `SummarizeTrim`
/// and re-summarize 1 message into 1 summary forever, burning aux LLM
/// calls without converging. We re-route those fires to `Prune` so the
/// session shrinks deterministically.
///
/// `keep_count == 0` is the same shape — happens when even the single
/// newest message exceeds `cron_session_max_tokens`. `Prune` empties the
/// session in that case (which the LLM call would never do).
pub(crate) fn cron_resolve_compaction_mode(
    configured: librefang_types::config::CronCompactionMode,
    keep_count: usize,
) -> librefang_types::config::CronCompactionMode {
    use librefang_types::config::CronCompactionMode;
    match configured {
        CronCompactionMode::SummarizeTrim if keep_count < 2 => CronCompactionMode::Prune,
        m => m,
    }
}

/// Apply a plain prune (drain-from-front) to a cron session, keeping the
/// newest `session.messages.len() - drop_count` messages.
///
/// Used uniformly by the `Prune` main path, the `SummarizeTrim` fallback
/// path, and the tool-pair-adjusted skip path so that all three paths
/// produce identical side effects (`mark_messages_mutated`).
pub(super) fn apply_cron_prune(
    session: &mut librefang_memory::session::Session,
    drop_count: usize,
) {
    if drop_count == 0 {
        return;
    }
    session.messages.drain(0..drop_count);
    session.mark_messages_mutated();
}

/// Attempt to summarize the messages that fall before `tail_start` using an
/// LLM aux call, then stitch the result back as `[summary_msg] + tail`.
///
/// Returns `Some(new_messages)` on a real LLM success (non-empty, non-fallback
/// summary). Returns `None` when the LLM fails or returns a fallback placeholder,
/// so the caller can apply a plain prune instead.
///
/// The tool-pair boundary is adjusted with `adjust_split_for_tool_pair` before
/// slicing so that `Assistant{ToolUse}` / `User{ToolResult}` pairs are never
/// separated across the summary / tail boundary. If the adjustment pushes
/// `tail_start` to `0` (nothing left to summarize) or to `messages.len()` (the
/// keep_recent window consumed everything), `None` is returned immediately so the
/// caller can decide whether to prune or skip.
///
/// Note: the caller holds the per-session mutex (`_prune_guard`) across this
/// await, which spans the entire LLM summary call. This is intentional — it
/// serialises `SummarizeTrim` runs on the same session so two concurrent fires
/// cannot each start a summary against the same un-compacted snapshot.
pub(super) async fn try_summarize_trim(
    messages: &[librefang_types::message::Message],
    keep_recent: usize,
    driver: std::sync::Arc<dyn librefang_runtime::llm_driver::LlmDriver>,
    model: &str,
) -> Option<Vec<librefang_types::message::Message>> {
    use librefang_runtime::compactor::{
        adjust_split_for_tool_pair, compact_messages, CompactionConfig,
    };
    use librefang_types::message::{Message, MessageContent, Role};

    // Fast-fail when the caller could not resolve a model name (e.g. the agent
    // disappeared from the registry between cron tick and prune). With an empty
    // model name `compact_messages` would still walk stage-1 (with retries) →
    // stage-2 (chunked) → stage-3 (fallback placeholder) and only then return
    // `used_fallback = true`, holding the per-session mutex and one cron_lane
    // slot for the full fail-out chain. Skip straight to the prune fallback.
    if model.is_empty() {
        return None;
    }

    let raw_tail_start = messages.len().saturating_sub(keep_recent);

    // Adjust so we never split an Assistant{ToolUse} / User{ToolResult} pair.
    let tail_start = adjust_split_for_tool_pair(messages, raw_tail_start, keep_recent);

    // Nothing to summarize — skip gracefully.
    if tail_start == 0 || tail_start == messages.len() {
        return None;
    }

    let to_summarize = &messages[..tail_start];
    let kept_tail = messages[tail_start..].to_vec();

    // We've already split off the kept tail above; tell `compact_messages` to
    // summarise the entire input it receives by setting `keep_recent = 0`.
    //
    // `max_retries = 1` (cron-only override): the per-session mutex and one
    // cron_lane slot are held across this LLM call. The default of 3 retries
    // inside stage-1 single-pass + the additional stage-2 chunked attempt can
    // stretch to tens of seconds on a flaky provider, blocking sibling fires
    // for the same `(agent, "cron")` session. Cron's failure mode is "fall
    // back to plain prune", so a single attempt is sufficient — operators who
    // want more aggressive retries should configure them in the aux driver
    // itself, not amplify them here.
    let compact_cfg = CompactionConfig {
        threshold: 0,
        keep_recent: 0,
        max_retries: 1,
        ..CompactionConfig::default()
    };

    // Only accept a real LLM summary — reject empty summaries and fallback
    // placeholders (used_fallback = true means the LLM was unavailable and
    // compact_messages substituted a synthetic "[Session compacted: ...]" stub).
    match compact_messages(driver, model, to_summarize, &compact_cfg).await {
        Ok(result) if !result.summary.is_empty() && !result.used_fallback => {
            let summary_msg = Message {
                role: Role::Assistant,
                content: MessageContent::Text(format!(
                    "[Cron session summary — {} messages compacted]\n\n{}",
                    result.compacted_count, result.summary,
                )),
                pinned: false,
                timestamp: None,
            };
            let mut new_messages = vec![summary_msg];
            new_messages.extend(kept_tail);
            Some(new_messages)
        }
        Ok(_) | Err(_) => None,
    }
}
