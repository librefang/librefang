//! Tool-result history fold — mechanism 3 of #3347.
//!
//! After `history_fold_after_turns` assistant turns have elapsed, tool-result
//! messages from those older turns are "stale" and contribute noise to the
//! context window without materially helping the agent.  This module folds
//! them into compact summaries so the LLM sees the history without the raw
//! payload bulk.
//!
//! # Algorithm (#4866 — batched-call + persistence)
//!
//! 1. Walk `messages` and count assistant turns.  Any tool-result user message
//!    that was answered before the most recent `history_fold_after_turns`
//!    assistant turns is marked stale.
//! 2. Collect every stale `ContentBlock::ToolResult` across those messages
//!    and ask the aux-LLM (or the primary driver when no aux chain is
//!    configured) for **one** batched call that returns a JSON array of
//!    per-`tool_use_id` summaries.
//! 3. Rewrite each stale `ContentBlock::ToolResult.content` in place using
//!    the matching per-id summary.  `tool_use_id` / `tool_name` / `is_error`
//!    / `status` are preserved, only `content` is replaced with
//!    `"[history-fold] <summary>"`.
//! 4. Return a `FoldResult.rewrites` map (`tool_use_id → "[history-fold] …"`)
//!    so the caller can replay the same rewrite onto the durable
//!    `session.messages` and call `session.mark_messages_mutated()` — without
//!    that step the fold runs from scratch every turn (axis 2 of #4866).
//! 5. Pinned messages are never folded (they are protected work product).
//!
//! Earlier drafts of this module (pre-#4866) made one aux-LLM call per
//! `group_consecutive` run of stale indices.  In practice
//! `collect_stale_indices` always returned indices interleaved with
//! assistant turns (e.g. `[2, 4, 6, …]`), so every group was size 1 and
//! the cost-amortiser knob (`min_batch_size`) gated the *pass* but not the
//! per-block calls — yielding `O(stale_count × turns)` aux-LLM round-trips.
//! The batched call collapses the pass to one round-trip; the persistence
//! step collapses the lifetime cost to `O(stale_count)` overall.
//!
//! # tool_use ↔ tool_result pairing — why we don't replace messages
//!
//! Earlier drafts replaced each stale tool-result message with a single
//! `Message::user(Text("[history-fold] ..."))` plain-text message.  That
//! broke conversation invariants: provider APIs (Anthropic Messages,
//! OpenAI Responses, Gemini function_call) require every assistant
//! `tool_use` block to be answered by a `tool_result` block carrying the
//! same `tool_use_id`.  Replacing the user message with free-form text
//! left the matching assistant `tool_use` orphaned, and the next provider
//! call returned `400 invalid_request_error: messages: tool_use ids must
//! match tool_result tool_use_ids`.
//!
//! Rewriting `ToolResult.content` in place keeps the `Blocks([ToolResult{
//! tool_use_id, ...}])` shape so every original `tool_use` still has its
//! matching `tool_result` — only the payload contracts.
//!
//! # Boundary choice
//!
//! Folding runs at the **pre-LLM-call boundary** (same as context compression)
//! so that the LLM always sees the compacted history regardless of whether the
//! session was loaded from disk mid-flight.  Running at session-load would also
//! work but would require async I/O at load time, complicating the sync path.
//!
//! # Fallback
//!
//! When the aux-LLM call fails (no key configured, network error, empty
//! response) the fold falls back to a static stub:
//! `[history-fold: summarisation unavailable]`.  When the call succeeds but
//! the response is not valid JSON the raw response text is applied to every
//! stale block instead (degrading to Option-1 bulk-summary semantics rather
//! than wasting the round-trip).  Either way the stale payload is removed
//! from context.

use crate::aux_client::AuxClient;
use crate::llm_driver::{CompletionRequest, LlmDriver};
use librefang_types::config::{AuxTask, ResponseFormat};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Prefix used in folded summary messages so agents and downstream code can
/// recognise that earlier tool results were compacted.
const FOLD_PREFIX: &str = "[history-fold]";

/// Number of characters shown in the per-tool preview inside the fold prompt.
const FOLD_PREVIEW_CHARS: usize = 500;

/// Static-stub text used when the aux-LLM summarisation call fails outright.
const FALLBACK_SUMMARY: &str = "[summarisation unavailable]";

/// Hard cap on `max_tokens` requested from the summariser.  Several
/// providers (Groq Llama-3.1, Cerebras, older Anthropic SKUs) reject
/// `max_tokens` above 8192 with a 400; 1-2 sentence summaries × ~125
/// blocks already saturates ~8k completion tokens, so the cap is
/// generous in practice.  A catalog-driven per-model cap would be the
/// long-term shape but bridges into the model catalog crate; static 8k
/// is the simplest safe default until a fold pass actually exceeds it.
const MAX_FOLD_OUTPUT_TOKENS: usize = 8_192;

/// Per-block token budget for the summariser.  Sized so a pretty-printing
/// model (one JSON key per line, plus an ~80-token 1-2 sentence summary)
/// fits without overrunning the JSON contract mid-string.  See #5203
/// post-mortem on the 64 tok/block regression.
///
/// **Cap interaction**: when `blocks.len() * FOLD_TOKENS_PER_BLOCK`
/// exceeds [`MAX_FOLD_OUTPUT_TOKENS`] (currently at `blocks.len() > 64`)
/// the per-block budget is silently clamped down by the cap.  The
/// summariser emits a `warn!` in that regime so operators can spot
/// truncation risk; the long-term fix is a catalog-driven per-model
/// cap.  See [`summarise_batch`] for the warn site.
const FOLD_TOKENS_PER_BLOCK: usize = 128;

/// Minimum `max_tokens` floor for a fold call, regardless of block
/// count.  Keeps the smallest size-1 fold pass (which is the most
/// common case after #4866 persistence) from being starved by a
/// pretty-printing model.  Sized at `4 × FOLD_TOKENS_PER_BLOCK` — the
/// same headroom four blocks would receive — so a tiny fold pass is
/// never less generous than a small batched one.
const FOLD_TOKENS_FLOOR: usize = 512;

/// Result of a single fold pass.
#[derive(Debug, Default)]
pub struct FoldResult {
    /// `1` when a fold pass actually rewrote at least one stale block; `0`
    /// otherwise.  Retained as a `usize` for backward compatibility with
    /// pre-#4866 call sites that branch on `groups_folded > 0`.
    pub groups_folded: usize,
    /// Total tool-result messages that were replaced.
    pub messages_replaced: usize,
    /// `1` when the aux-LLM batched call failed (network/empty) and the
    /// pass fell back to [`FALLBACK_SUMMARY`]; `0` otherwise.  Kept as a
    /// counter (rather than a bool) so future per-chunk batching can
    /// surface partial failures.
    pub groups_used_fallback: usize,
    /// Map of `tool_use_id → "[history-fold] <summary>"` content.  The
    /// agent loop replays this onto the durable `session.messages` (and
    /// calls `mark_messages_mutated()`) so subsequent turns don't refold
    /// the same blocks — addressing axis 2 of #4866.
    pub rewrites: BTreeMap<String, String>,
}

/// Tuning knobs for a single [`fold_stale_tool_results`] pass.
///
/// Bundled so the function signature stays under clippy's
/// `too_many_arguments` cap as new dispatch fields (e.g. catalog-driven
/// `reasoning_echo_policy`, #4842) accrue.  Sourced from the kernel's
/// runtime config — see `KernelConfig.history_fold` in
/// `librefang-types`.
#[derive(Debug, Clone, Copy)]
pub struct FoldConfig {
    /// Fold tool results older than this many assistant turns.
    pub fold_after_turns: u32,
    /// Only run a fold pass when at least this many newly-stale (i.e.
    /// not already-folded) tool-result messages have accumulated.
    /// Amortises the aux-LLM cost on long sessions where each new turn
    /// would otherwise drag exactly one new message across the staleness
    /// boundary and trigger another fold call. Set to `1` to disable the
    /// batch threshold (fold every turn); `0` is treated as `1`.
    pub min_batch_size: u32,
}

/// Fold stale tool-result messages in `messages`.
///
/// `cfg` carries the fold-tuning knobs (see [`FoldConfig`]).
/// `model` — model slug forwarded to the summariser.
/// `aux_client` — optional aux-LLM client; when `None`, the primary driver
///   is used.  With the persistence fix the cost is `O(1)` per stale block
///   over the session lifetime, so the primary-driver fallback is no longer
///   the silent financial-DoS it was before #4866.
/// `driver` — primary driver (used when aux chain resolves to primary).
/// `reasoning_echo_policy` — catalog-driven dispatch hint for the
/// OpenAI-compatible driver (#4842).
///
/// Returns the (possibly modified) message list and a [`FoldResult`]
/// summary.  Callers MUST also replay `result.rewrites` onto the durable
/// `session.messages` and mark the session mutated — otherwise the fold
/// is performed again from scratch on the next turn.  See
/// [`apply_fold_rewrites`] for the recommended replay helper.
pub async fn fold_stale_tool_results(
    mut messages: Vec<Message>,
    cfg: FoldConfig,
    model: &str,
    aux_client: Option<&AuxClient>,
    driver: Arc<dyn LlmDriver>,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> (Vec<Message>, FoldResult) {
    let FoldConfig {
        fold_after_turns,
        min_batch_size,
    } = cfg;
    if fold_after_turns == 0 {
        return (messages, FoldResult::default());
    }

    // Walk backwards to find the stale boundary.  Count assistant turns from
    // the end; tool-result messages whose assistant-turn distance exceeds
    // `fold_after_turns` are stale.
    let stale_indices = collect_stale_indices(&messages, fold_after_turns as usize);

    if stale_indices.is_empty() {
        return (messages, FoldResult::default());
    }

    // Cost amortiser: on a long-running session every new turn pushes
    // exactly one fresh message across the staleness boundary, which would
    // trigger an aux-LLM call per turn just to fold a single message.  Skip
    // the pass until at least `min_batch_size` newly-stale messages have
    // accumulated.
    let batch_size = std::cmp::max(min_batch_size, 1) as usize;
    if stale_indices.len() < batch_size {
        debug!(
            stale_count = stale_indices.len(),
            min_batch_size = batch_size,
            "history_fold: skip — newly-stale below batch threshold"
        );
        return (messages, FoldResult::default());
    }

    debug!(
        stale_count = stale_indices.len(),
        fold_after_turns, "history_fold: folding stale tool-result messages"
    );

    // Resolve the summarisation driver (aux preferred, primary as fallback).
    let summary_driver = aux_client
        .map(|c| c.driver_for(AuxTask::Fold))
        .unwrap_or_else(|| Arc::clone(&driver));

    // Collect every stale tool-result block, in message+block order, with
    // its `tool_use_id` / `tool_name` / preview-bounded content.  This is
    // the payload of the batched summariser call.
    let stale_blocks = collect_stale_blocks(&messages, &stale_indices);

    let mut result = FoldResult::default();

    // One batched LLM call returns either:
    //   - `Ok(per-id map)` on a happy-path JSON response,
    //   - `Err(Parse{raw,…})` when the model produced a non-JSON string —
    //     we still apply `raw` as a bulk summary to every block (Option-1
    //     fallback) rather than waste the round-trip,
    //   - `Err(Call|Empty)` when the aux-LLM is unreachable / returned
    //     nothing — we apply [`FALLBACK_SUMMARY`] to every block.
    let stale_count = stale_blocks.len();
    let (summaries_by_id, bulk_fallback) = match summarise_batch(
        &stale_blocks,
        model,
        &*summary_driver,
        reasoning_echo_policy,
    )
    .await
    {
        Ok(map) => {
            info!(
                count = stale_count,
                "history_fold: summarised tool-result(s) in 1 batched call"
            );
            // Surface model-returned ids that do not match any stale
            // tool_use_id AND stale ids that the model silently omitted.
            // Common failure modes: trailing whitespace, double-quoting
            // (`"\"tid_3\""`), Unicode-normalised dash variants, model
            // under-delivery (returns summaries for K of N stale blocks).
            // Without these warns the affected blocks silently fall back
            // to `FALLBACK_SUMMARY` and operators have no way to spot
            // the drift.
            let stale_id_set: std::collections::BTreeSet<&str> = stale_blocks
                .iter()
                .map(|b| b.tool_use_id.as_str())
                .collect();
            let unmatched: Vec<&str> = map
                .keys()
                .map(String::as_str)
                .filter(|id| !stale_id_set.contains(id))
                .collect();
            if !unmatched.is_empty() {
                warn!(
                    unmatched_ids = ?unmatched,
                    "history_fold: model returned ids that did not match any stale tool_use_id — those blocks fall back to the static stub"
                );
            }
            let missing: Vec<&str> = stale_id_set
                .iter()
                .copied()
                .filter(|id| !map.contains_key(*id))
                .collect();
            if !missing.is_empty() {
                warn!(
                    missing_ids = ?missing,
                    "history_fold: model omitted summaries for some stale tool_use_ids — those blocks fall back to the static stub"
                );
            }
            (map, None)
        }
        Err(BatchSummariseFailure::Parse { raw, error }) => {
            warn!(
                count = stale_count,
                error = %error,
                "history_fold: JSON parse failed — applying raw response as bulk summary"
            );
            (BTreeMap::new(), Some(raw))
        }
        Err(err) => {
            warn!(
                count = stale_count,
                error = %err,
                "history_fold: aux-LLM batched call failed — using fallback stub"
            );
            result.groups_used_fallback = 1;
            (BTreeMap::new(), Some(FALLBACK_SUMMARY.to_string()))
        }
    };

    // Apply per-block.  Preserve `tool_use_id` / `tool_name` / `is_error`
    // / `status` so every original assistant `tool_use` block keeps its
    // matching `tool_result` block — provider APIs reject mismatched
    // tool_use_ids with `400 invalid_request_error`.  (Pairing-preservation
    // is module-doc invariant; see the test suite below.)
    for &i in &stale_indices {
        if let MessageContent::Blocks(blocks) = &mut messages[i].content {
            for block in blocks.iter_mut() {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    let summary = summaries_by_id
                        .get(tool_use_id)
                        .cloned()
                        .or_else(|| bulk_fallback.clone())
                        .unwrap_or_else(|| FALLBACK_SUMMARY.to_string());
                    let new_content = format!("{FOLD_PREFIX} {summary}");
                    if *content != new_content {
                        *content = new_content.clone();
                        result.rewrites.insert(tool_use_id.clone(), new_content);
                    }
                }
            }
        }
    }

    if !result.rewrites.is_empty() {
        result.groups_folded = 1;
    }
    result.messages_replaced = stale_indices.len();

    (messages, result)
}

/// Replay a fold pass's [`FoldResult::rewrites`] onto a durable message
/// list (typically `session.messages`).  Matches by `tool_use_id` so the
/// caller does not have to keep the working-copy and durable lists in
/// lock-step.  Returns `true` when at least one block was rewritten — the
/// caller is responsible for invoking `session.mark_messages_mutated()`
/// in that case.
///
/// This is the companion to [`fold_stale_tool_results`] and the
/// [`FoldResult::rewrites`] field added in #4866 to fix axis 2 (fold was
/// previously applied only to a working clone, leaving `session.messages`
/// to be re-folded from scratch every turn).
pub fn apply_fold_rewrites(messages: &mut [Message], rewrites: &BTreeMap<String, String>) -> bool {
    if rewrites.is_empty() {
        return false;
    }
    let mut changed = false;
    for msg in messages.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            for block in blocks.iter_mut() {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    if let Some(new_content) = rewrites.get(tool_use_id) {
                        if content != new_content {
                            *content = new_content.clone();
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    changed
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// One stale tool-result block flattened for the batched summariser
/// prompt.  Kept as a struct (not a tuple) so each call site is
/// self-documenting and the test mock can construct fixtures without
/// positional confusion.
struct StaleBlock {
    tool_use_id: String,
    tool_name: String,
    content: String,
}

fn collect_stale_blocks(messages: &[Message], stale_indices: &[usize]) -> Vec<StaleBlock> {
    let mut out: Vec<StaleBlock> = Vec::new();
    for &i in stale_indices {
        if let MessageContent::Blocks(blocks) = &messages[i].content {
            for b in blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    tool_name,
                    content,
                    ..
                } = b
                {
                    out.push(StaleBlock {
                        tool_use_id: tool_use_id.clone(),
                        tool_name: tool_name.clone(),
                        content: content.clone(),
                    });
                }
            }
        }
    }
    out
}

/// Return the indices (into `messages`) of tool-result user messages that are
/// older than `fold_after_turns` assistant turns from the end.
///
/// A message is a "tool-result message" when its content is a `Blocks` vec
/// that contains at least one `ContentBlock::ToolResult` block AND it has the
/// `User` role.  Pinned messages are never stale.
fn collect_stale_indices(messages: &[Message], fold_after_turns: usize) -> Vec<usize> {
    // Walk backwards, count assistant messages, mark the boundary index.
    let mut assistant_turns_seen = 0usize;
    let mut boundary_idx = messages.len(); // exclusive upper bound for "recent" turns

    for (i, msg) in messages.iter().enumerate().rev() {
        if msg.role == Role::Assistant {
            assistant_turns_seen += 1;
            if assistant_turns_seen == fold_after_turns {
                // Everything at index < i is from before this boundary.
                boundary_idx = i;
                break;
            }
        }
    }

    if assistant_turns_seen < fold_after_turns {
        // Not enough history yet.
        return Vec::new();
    }

    // Collect stale tool-result indices.
    // Messages that already start with FOLD_PREFIX are previously-folded stubs;
    // skip them to avoid redundant re-summarisation on every subsequent turn.
    messages
        .iter()
        .enumerate()
        .filter(|(i, msg)| {
            *i < boundary_idx
                && !msg.pinned
                && msg.role == Role::User
                && is_tool_result_message(msg)
                && !is_already_folded(msg)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Returns true when `msg` is a user message whose content consists entirely
/// (or partially) of `ToolResult` blocks.
fn is_tool_result_message(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { .. })),
        _ => false,
    }
}

/// Returns true when `msg`'s tool-result blocks have already been collapsed
/// by a prior fold pass.  Detection: any `ToolResult.content` starts with
/// `FOLD_PREFIX`.  These messages are cheap stubs and re-folding them would
/// produce summary-of-summary output AND burn another aux-LLM call.
fn is_already_folded(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks.iter().any(|b| {
            matches!(
                b,
                ContentBlock::ToolResult { content, .. } if content.starts_with(FOLD_PREFIX)
            )
        }),
        // Legacy: previous draft emitted plain-text stubs. Recognise those
        // too so an in-flight session that was folded by an older binary
        // doesn't get re-folded on first run after upgrade.
        MessageContent::Text(t) => t.starts_with(FOLD_PREFIX),
    }
}

/// Failure modes for the batched summariser.  `Parse` retains the raw
/// model response so the caller can fall back to Option-1 bulk-summary
/// semantics instead of wasting the round-trip.
enum BatchSummariseFailure {
    Call(String),
    Empty,
    Parse { raw: String, error: String },
}

impl std::fmt::Display for BatchSummariseFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchSummariseFailure::Call(e) => write!(f, "LLM call failed: {e}"),
            BatchSummariseFailure::Empty => write!(f, "LLM returned empty response"),
            BatchSummariseFailure::Parse { error, .. } => write!(f, "{error}"),
        }
    }
}

/// Ask the LLM to summarise every stale tool-result block in one batched
/// call, returning a `tool_use_id → summary` map.  See module-doc for the
/// algorithm rationale and the `BatchSummariseFailure` variants for the
/// fall-back paths the caller must handle.
async fn summarise_batch(
    blocks: &[StaleBlock],
    model: &str,
    driver: &dyn LlmDriver,
    reasoning_echo_policy: librefang_types::model_catalog::ReasoningEchoPolicy,
) -> Result<BTreeMap<String, String>, BatchSummariseFailure> {
    // Build the batched prompt: a JSON-array contract, one labelled line
    // per stale block.  The `[id]` prefix is the round-trip key the model
    // must echo back in its JSON output.
    let mut prompt = String::from(
        "Summarise each tool execution result below in 1-2 sentences. \
         Capture what the tool did and what it returned, omitting raw data. \
         Output ONLY a JSON array of objects with this shape: \
         [{\"id\":\"<tool_use_id>\",\"summary\":\"...\"}]. \
         Echo every id verbatim. No preamble, no markdown fences.\n\n\
         Results:\n",
    );
    for b in blocks {
        let preview: String = b.content.chars().take(FOLD_PREVIEW_CHARS).collect();
        let has_more = b.content.len() > FOLD_PREVIEW_CHARS;
        prompt.push_str(&format!("[{}] {}: {}", b.tool_use_id, b.tool_name, preview));
        if has_more {
            prompt.push_str(" ...[truncated]");
        }
        prompt.push('\n');
    }

    // Headroom for N short summaries plus JSON wrapping.
    // `FOLD_TOKENS_PER_BLOCK` gives comfortable slack for models that
    // pretty-print (one field per line) rather than emitting compact
    // single-line JSON — the previous 64 tok/block budget was routinely
    // overrun by verbose-output models such as ollama:gemma4, causing
    // serde_json to fail with "EOF while parsing a string" and every fold
    // pass to degrade to the raw-text bulk fallback (#5203).  Floor
    // (`FOLD_TOKENS_FLOOR`) sized at 4 × per-block so a size-1 pass
    // (#4866 steady state) is never less generous than a small batch.
    // Capped at `MAX_FOLD_OUTPUT_TOKENS` (provider 400 ceiling); when
    // the requested per-block budget exceeds the cap we warn so operators
    // can spot the truncation risk that the cap silently re-introduces.
    let requested = std::cmp::max(
        FOLD_TOKENS_FLOOR,
        FOLD_TOKENS_PER_BLOCK.saturating_mul(blocks.len()),
    );
    if requested > MAX_FOLD_OUTPUT_TOKENS {
        let effective_per_block = MAX_FOLD_OUTPUT_TOKENS / blocks.len().max(1);
        warn!(
            block_count = blocks.len(),
            requested_max_tokens = requested,
            capped_max_tokens = MAX_FOLD_OUTPUT_TOKENS,
            effective_per_block_tokens = effective_per_block,
            fold_tokens_per_block = FOLD_TOKENS_PER_BLOCK,
            "history_fold: per-block token budget exceeds MAX_FOLD_OUTPUT_TOKENS — \
             verbose-JSON responses may still be truncated for large batches \
             (see #5203). Effective per-block budget is reduced by the cap."
        );
    }
    let max_tokens = requested.min(MAX_FOLD_OUTPUT_TOKENS) as u32;

    let request = CompletionRequest {
        model: model.to_string(),
        messages: Arc::new(vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: prompt,
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        }]),
        tools: Arc::new(vec![]),
        max_tokens,
        temperature: 0.3,
        system: Some(
            "You are a concise summariser. Produce short factual summaries of tool outputs \
             as a JSON array, echoing the supplied ids verbatim."
                .to_string(),
        ),
        thinking: None,
        // Each fold call embeds distinct previews, so there is no shared
        // prefix to amortise.  Caching only adds the cache-write cost
        // without any subsequent hit.
        prompt_caching: false,
        cache_ttl: None,
        prompt_cache_strategy: None,
        // Request structured JSON output (`{"type": "json_object"}` on
        // OpenAI-compat drivers). Without this, DeepSeek / OpenAI /
        // Mistral / Gemini are free to emit natural-language numbered
        // lists, `parse_labeled_summaries` fails, and the fold falls
        // back to "applying raw response as bulk summary" — losing the
        // per-tool_use_id granularity (tool_use_id / tool_name /
        // is_error / status) we just spent the aux-LLM call building.
        // Providers that don't honour the flag ignore it without
        // error; the system prompt above already contains the word
        // "JSON" (DeepSeek's requirement for json_object mode).
        // Same pattern as proactive_memory.rs (#5287).
        response_format: Some(ResponseFormat::Json),
        timeout_secs: None,
        extra_body: None,
        agent_id: None,
        session_id: None,
        step_id: None,
        reasoning_echo_policy,
    };

    let response = driver
        .complete(request)
        .await
        .map_err(|e| BatchSummariseFailure::Call(format!("{e}")))?;
    let raw = response.text();
    if raw.is_empty() {
        return Err(BatchSummariseFailure::Empty);
    }

    parse_labeled_summaries(&raw).map_err(|error| BatchSummariseFailure::Parse {
        raw: raw.clone(),
        error,
    })
}

/// Parse the batched-call response into a `tool_use_id → summary` map.
/// Tolerates a single markdown code-fence wrapper around the JSON array
/// because some providers (notably reasoning-tier models) still emit
/// fenced output even when told not to.
fn parse_labeled_summaries(text: &str) -> Result<BTreeMap<String, String>, String> {
    let body = strip_code_fence(text.trim()).unwrap_or_else(|| text.trim().to_string());
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse failed: {e}"))?;
    // Accept both the documented JSON-array shape AND a bare single
    // object — providers routinely emit `{...}` instead of `[{...}]`
    // when only one stale block was supplied (after #4866 persistence
    // most fold passes are exactly that case).  Lifting the object into
    // a one-element vec preserves per-id granularity instead of
    // degrading to bulk-summary on every size-1 pass.
    let entries: Vec<serde_json::Value> = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(_) => vec![value],
        _ => return Err("expected JSON array or object".into()),
    };
    // Distinguish "model returned an empty array" from "model returned
    // entries but none had the {id,summary} shape" — the two failure
    // modes need different operator interventions and squashing them
    // into one error string costs debugging time.
    if entries.is_empty() {
        return Err("JSON array was empty — model returned no summaries".into());
    }
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for entry in entries {
        if let (Some(id), Some(summary)) = (
            entry.get("id").and_then(|x| x.as_str()),
            entry.get("summary").and_then(|x| x.as_str()),
        ) {
            out.insert(id.to_string(), summary.to_string());
        }
    }
    if out.is_empty() {
        return Err("JSON entries did not contain any {id,summary} pairs".into());
    }
    Ok(out)
}

/// Strip a leading ` ```json ` / ` ``` ` and trailing ` ``` ` fence if
/// present.  Returns `None` when there is no fence so the caller can fall
/// back to the unfenced body.
fn strip_code_fence(s: &str) -> Option<String> {
    let s = s.trim();
    let after_open = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```JSON"))
        .or_else(|| s.strip_prefix("```"))?;
    let inner = after_open.trim_start_matches(['\n', '\r', ' ']);
    let body = inner.strip_suffix("```").unwrap_or(inner);
    Some(body.trim().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmError};
    use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn assistant_msg(text: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: MessageContent::Text(text.to_string()),
            pinned: false,
            timestamp: None,
        }
    }

    fn tool_result_msg(tool_name: &str, content: &str) -> Message {
        tool_result_msg_with_id("id-1", tool_name, content)
    }

    fn tool_result_msg_with_id(tool_use_id: &str, tool_name: &str, content: &str) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                content: content.to_string(),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::Completed,
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        }
    }

    fn user_msg(text: &str) -> Message {
        Message {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
            pinned: false,
            timestamp: None,
        }
    }

    // ── Mock drivers ─────────────────────────────────────────────────────────

    /// Driver that always returns a fixed text body (NOT JSON) and counts
    /// the number of `complete()` invocations.  Lets tests assert both
    /// "batched-call ran exactly once" (#4866 axis 1) and "second pass
    /// is a no-op when fold was persisted" (axis 2).
    struct CountingTextDriver {
        text: String,
        calls: Arc<AtomicUsize>,
    }

    impl CountingTextDriver {
        fn new(text: &str) -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                CountingTextDriver {
                    text: text.to_string(),
                    calls: Arc::clone(&calls),
                },
                calls,
            )
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for CountingTextDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.text.clone(),
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                usage: librefang_types::message::TokenUsage::default(),
                actual_provider: None,
            })
        }
    }

    /// Driver that always returns a fixed text body (no call counter).
    /// Kept as the `OkDriver` shorthand the legacy tests reach for.
    struct OkDriver(String);

    #[async_trait::async_trait]
    impl LlmDriver for OkDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: self.0.clone(),
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                usage: librefang_types::message::TokenUsage::default(),
                actual_provider: None,
            })
        }
    }

    /// Driver that truncates its response to `max_tokens * 4` bytes
    /// (the rough provider convention of ~4 chars/token).  Models that
    /// pretty-print JSON saturate their `max_tokens` budget on payload
    /// shape rather than substance, so a too-tight budget cuts the
    /// response mid-string — the exact failure mode #5203 was filed
    /// against.  Used by the regression test to prove that bumping the
    /// per-block budget to `FOLD_TOKENS_PER_BLOCK` (and the floor to
    /// `FOLD_TOKENS_FLOOR`) actually lets a realistic verbose response
    /// survive the round-trip.
    struct TruncatingDriver {
        full_response: String,
    }

    #[async_trait::async_trait]
    impl LlmDriver for TruncatingDriver {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            // Mirror the provider convention: ~4 characters per token.
            // A pretty-printing model emits one JSON key per line, so a
            // tight token budget cuts the response mid-string and
            // serde_json fails with "EOF while parsing a string".
            let byte_limit = (req.max_tokens as usize).saturating_mul(4);
            let truncated: String = self.full_response.chars().take(byte_limit).collect();
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: truncated,
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                usage: librefang_types::message::TokenUsage::default(),
                actual_provider: None,
            })
        }
    }

    /// Driver that records the `max_tokens` it was asked for, then
    /// returns a benign per-id JSON.  Used to assert that the
    /// production budget arithmetic produces the expected per-block
    /// allocation under both small (floor-bound) and large (per-block
    /// scaled) batches.
    struct MaxTokensRecordingDriver {
        observed: Arc<std::sync::Mutex<Vec<u32>>>,
    }

    impl MaxTokensRecordingDriver {
        fn new() -> (Self, Arc<std::sync::Mutex<Vec<u32>>>) {
            let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
            (
                MaxTokensRecordingDriver {
                    observed: Arc::clone(&observed),
                },
                observed,
            )
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for MaxTokensRecordingDriver {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.observed.lock().unwrap().push(req.max_tokens);
            // Echo back a per-id summary for every stale block in the
            // prompt so the fold succeeds end-to-end — we only care
            // about the recorded `max_tokens` here, not the content.
            let prompt = match &req.messages[0].content {
                MessageContent::Blocks(blocks) => match &blocks[0] {
                    ContentBlock::Text { text, .. } => text.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            // Recover the labelled ids from the prompt (format: `[id] tool: ...`).
            let mut entries: Vec<String> = Vec::new();
            for line in prompt.lines() {
                if let Some(rest) = line.strip_prefix('[') {
                    if let Some(end) = rest.find(']') {
                        let id = &rest[..end];
                        entries.push(format!("{{\"id\":\"{id}\",\"summary\":\"ok\"}}"));
                    }
                }
            }
            let body = format!("[{}]", entries.join(","));
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: body,
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                usage: librefang_types::message::TokenUsage::default(),
                actual_provider: None,
            })
        }
    }

    /// Driver that always returns an error.
    struct FailDriver;

    #[async_trait::async_trait]
    impl LlmDriver for FailDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Http("simulated failure".to_string()))
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────

    /// Build a message list that simulates `n_turns` turns, each containing
    /// one user message, one assistant message, and one tool-result
    /// message.  Each tool result gets a unique `tool_use_id` so the
    /// batched-call labelled-summary tests can match per-id.
    fn build_history(n_turns: usize) -> Vec<Message> {
        let mut msgs = vec![user_msg("initial question")];
        for i in 0..n_turns {
            msgs.push(assistant_msg(&format!("assistant response {i}")));
            msgs.push(tool_result_msg_with_id(
                &format!("tid_{i}"),
                "shell_exec",
                &format!("output of turn {i}"),
            ));
        }
        msgs
    }

    /// Returns true when any `ToolResult.content` in `msg` starts with
    /// `FOLD_PREFIX` — the post-#1-review fold-detection predicate.
    fn has_folded_tool_result(msg: &Message) -> bool {
        matches!(&msg.content, MessageContent::Blocks(blocks)
        if blocks.iter().any(|b| matches!(
            b,
            ContentBlock::ToolResult { content, .. } if content.starts_with(FOLD_PREFIX)
        )))
    }

    /// Helper: extract every `tool_use_id` from `msg`'s ToolResult blocks.
    /// Used by pairing-preservation tests to confirm fold did not drop ids.
    fn tool_use_ids_in(msg: &Message) -> Vec<String> {
        match &msg.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    #[tokio::test]
    async fn fold_after_8_folds_old_turns() {
        // 10 turns total; fold_after=8 → turns 0 and 1 are stale (oldest 2).
        let messages = build_history(10);
        let pre_ids: Vec<Vec<String>> = messages.iter().map(tool_use_ids_in).collect();
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("nice summary".to_string()));

        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 8,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(
            result.groups_folded, 1,
            "expected a single batched fold pass to have run"
        );
        assert!(
            result.messages_replaced >= 1,
            "expected at least one message replaced"
        );
        assert!(
            !result.rewrites.is_empty(),
            "expected the rewrites map to capture per-tool_use_id stubs for the caller \
             to replay onto session.messages"
        );
        // tool_use_ids must survive — pairing invariant.
        let post_ids: Vec<Vec<String>> = out.iter().map(tool_use_ids_in).collect();
        assert_eq!(
            pre_ids, post_ids,
            "fold must preserve every ToolResult.tool_use_id (pairing invariant)"
        );
        // Folded ToolResult.content must carry the FOLD_PREFIX marker.
        assert!(
            out.iter().any(has_folded_tool_result),
            "expected at least one folded ToolResult in output"
        );
    }

    #[tokio::test]
    async fn no_fold_when_not_enough_turns() {
        // Only 5 turns; fold_after=8 → nothing should be folded.
        let messages = build_history(5);
        let original_len = messages.len();
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("summary".to_string()));

        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 8,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(result.groups_folded, 0);
        assert_eq!(result.messages_replaced, 0);
        assert!(result.rewrites.is_empty());
        assert_eq!(out.len(), original_len, "history unchanged");
    }

    #[tokio::test]
    async fn fallback_stub_when_llm_unavailable() {
        // 10 turns, fold_after=8, but the LLM driver always fails.
        let messages = build_history(10);
        let driver: Arc<dyn LlmDriver> = Arc::new(FailDriver);

        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 8,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        // Should still fold (with fallback stubs).
        assert_eq!(
            result.groups_used_fallback, 1,
            "single batched call → exactly one fallback recorded"
        );
        assert_eq!(result.groups_folded, 1);
        let fallback_present = out.iter().any(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().any(|b| match b {
                ContentBlock::ToolResult { content, .. } => {
                    content.starts_with(FOLD_PREFIX)
                        && content.contains("summarisation unavailable")
                }
                _ => false,
            }),
            _ => false,
        });
        assert!(
            fallback_present,
            "expected fallback summary in a folded ToolResult.content"
        );
    }

    /// Pairing invariant explicit check: feed a turn whose assistant message
    /// has a `ToolUse{id="abc"}` and a corresponding user `ToolResult{
    /// tool_use_id="abc"}`, fold, and confirm the tool_use_id survives.
    #[tokio::test]
    async fn fold_preserves_tool_use_id_pairing() {
        let mut msgs: Vec<Message> = Vec::new();
        msgs.push(user_msg("user question"));
        // Stale turn (fold_after=2 means turns >= 2 from end are stale).
        msgs.push(Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "tid_stale".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"cmd": "ls"}),
                provider_metadata: None,
            }]),
            pinned: false,
            timestamp: None,
        });
        msgs.push(Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tid_stale".to_string(),
                tool_name: "shell".to_string(),
                content: "<original 50KB stale output>".to_string(),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::Completed,
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        });
        // Two recent turns to push the stale one across the boundary.
        for i in 0..3 {
            msgs.push(assistant_msg(&format!("recent {i}")));
            msgs.push(tool_result_msg("recent_tool", &format!("recent {i}")));
        }
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("compact summary".to_string()));
        let (out, _result) = fold_stale_tool_results(
            msgs,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        // The stale ToolResult must still exist with its tool_use_id intact,
        // only `content` rewritten — without this the assistant's ToolUse{
        // id="tid_stale"} would be orphaned and the next provider call would
        // 400 with "tool_use ids must match tool_result tool_use_ids".
        let stale_tr_block = out.iter().find_map(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "tid_stale" => Some(content.clone()),
                _ => None,
            }),
            _ => None,
        });
        let content = stale_tr_block.expect(
            "stale ToolResult{tool_use_id=tid_stale} must survive fold to keep \
             tool_use/tool_result pairing intact",
        );
        assert!(
            content.starts_with(FOLD_PREFIX),
            "stale ToolResult.content must be rewritten to a fold stub, got: {content:?}"
        );
    }

    /// Cost amortiser: when `min_batch_size > stale_count` the fold pass
    /// exits early without calling the aux-LLM.
    #[tokio::test]
    async fn min_batch_size_skips_fold_when_below_threshold() {
        // 3 stale turns; min_batch_size=4 → no fold, no aux call.
        let messages = build_history(11);
        let stale = collect_stale_indices(&messages, 8);
        assert_eq!(stale.len(), 3, "test setup: expected 3 stale tool results");

        let (driver_inner, calls) =
            CountingTextDriver::new("should never be called — fold should skip below threshold");
        let driver: Arc<dyn LlmDriver> = Arc::new(driver_inner);
        let (out, result) = fold_stale_tool_results(
            messages.clone(),
            FoldConfig {
                fold_after_turns: 8,
                min_batch_size: 4,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(
            result.groups_folded, 0,
            "fold must skip when stale_count < min_batch_size"
        );
        assert_eq!(result.messages_replaced, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "no aux call expected");
        // History returns unchanged.
        assert_eq!(out.len(), messages.len());
    }

    /// Axis 1 regression: every stale tool-result must be summarised by
    /// **one** batched LLM call, not N per-block calls.
    #[tokio::test]
    async fn batched_call_invokes_llm_once_per_pass() {
        let messages = build_history(10); // 8 stale, fold_after=2
        let (driver_inner, calls) = CountingTextDriver::new("bulk summary");
        let driver: Arc<dyn LlmDriver> = Arc::new(driver_inner);

        let (_, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "fold must produce exactly one aux-LLM call per pass — previously \
             every stale tool-result triggered its own call (issue #4866 axis 1)"
        );
        assert!(result.messages_replaced >= 8);
    }

    /// Axis 2 regression: once `rewrites` is replayed onto a durable
    /// message list, a second fold pass on the SAME list must be a no-op
    /// (zero aux-LLM calls, zero rewrites).  Without the persistence fix
    /// every subsequent turn re-folded the same stale payloads from
    /// scratch.
    #[tokio::test]
    async fn second_pass_is_no_op_after_rewrites_applied() {
        let messages = build_history(10);
        let (driver_inner, calls) = CountingTextDriver::new("bulk summary");
        let driver: Arc<dyn LlmDriver> = Arc::new(driver_inner);

        // First pass: should call the aux-LLM once and produce rewrites.
        let (mut durable, first) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            Arc::clone(&driver),
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!first.rewrites.is_empty());

        // Simulate the agent_loop replay step: caller would apply the
        // rewrites onto session.messages.  `durable` here already has
        // the stubs (it is the returned working copy), so `apply_fold_rewrites`
        // is just a no-op safety check.
        let _ = apply_fold_rewrites(&mut durable, &first.rewrites);

        // Second pass: stubs are present, fold should bail out before
        // ever calling the aux-LLM.
        let (_, second) = fold_stale_tool_results(
            durable,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second pass must NOT call the aux-LLM — previously fold re-ran \
             from scratch every turn (issue #4866 axis 2)"
        );
        assert_eq!(second.groups_folded, 0);
        assert_eq!(second.messages_replaced, 0);
        assert!(second.rewrites.is_empty());
    }

    /// JSON happy-path: when the model returns a proper `[{id,summary}…]`
    /// array each stale block receives its own per-id summary (Option 2
    /// per-tool granularity preserved).
    #[tokio::test]
    async fn json_response_assigns_per_tool_use_id_summary() {
        let messages = build_history(10); // tid_0..tid_9
                                          // Hand-craft a JSON response that names a subset of stale ids.
                                          // Anything not named falls back to FALLBACK_SUMMARY because
                                          // `bulk_fallback` is None on the happy path.
        let json = r#"[
            {"id":"tid_0","summary":"listed files"},
            {"id":"tid_1","summary":"read config"},
            {"id":"tid_2","summary":"ran tests"},
            {"id":"tid_3","summary":"committed change"},
            {"id":"tid_4","summary":"pushed branch"},
            {"id":"tid_5","summary":"opened PR"},
            {"id":"tid_6","summary":"merged PR"},
            {"id":"tid_7","summary":"deployed staging"}
        ]"#;
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver(json.to_string()));

        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        // Every named id should now carry its specific summary.
        let by_id: BTreeMap<String, String> = out
            .iter()
            .flat_map(|m| match &m.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => Some((tool_use_id.clone(), content.clone())),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(
            by_id.get("tid_0").map(String::as_str),
            Some("[history-fold] listed files")
        );
        assert_eq!(
            by_id.get("tid_3").map(String::as_str),
            Some("[history-fold] committed change")
        );
        // groups_used_fallback must stay 0 on the happy path.
        assert_eq!(result.groups_used_fallback, 0);
        // rewrites must contain at least one entry per stale id we named.
        assert!(result.rewrites.contains_key("tid_0"));
        assert!(result.rewrites.contains_key("tid_7"));
    }

    /// Tolerate ```json … ``` fences around the JSON body — common with
    /// reasoning-tier models that ignore "no markdown" instructions.
    #[tokio::test]
    async fn json_response_with_markdown_fence_is_accepted() {
        let messages = build_history(5);
        let fenced = "```json\n[{\"id\":\"tid_0\",\"summary\":\"fenced ok\"}]\n```";
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver(fenced.to_string()));
        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert_eq!(result.groups_used_fallback, 0);
        let tid_0_content = out.iter().find_map(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "tid_0" => Some(content.clone()),
                _ => None,
            }),
            _ => None,
        });
        assert_eq!(
            tid_0_content.as_deref(),
            Some("[history-fold] fenced ok"),
            "fenced JSON response should parse identically to bare JSON"
        );
    }

    /// Single-object JSON response (no surrounding `[]`) must still
    /// produce a per-id summary.  Providers commonly emit a bare object
    /// when only one stale block was supplied — which is the common
    /// case after the #4866 persistence fix, since each subsequent fold
    /// pass tends to carry exactly one newly-stale block.
    #[tokio::test]
    async fn json_response_single_object_is_lifted_to_one_element() {
        let messages = build_history(5);
        let single = r#"{"id":"tid_0","summary":"single-object reply"}"#;
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver(single.to_string()));
        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;
        assert_eq!(
            result.groups_used_fallback, 0,
            "single-object happy path must NOT fall back to the static stub"
        );
        let tid_0_content = out.iter().find_map(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "tid_0" => Some(content.clone()),
                _ => None,
            }),
            _ => None,
        });
        assert_eq!(
            tid_0_content.as_deref(),
            Some("[history-fold] single-object reply"),
            "bare-object JSON response must be lifted into a single-element array \
             so per-id granularity is preserved on size-1 fold passes"
        );
        assert!(result.rewrites.contains_key("tid_0"));
    }

    /// Non-JSON response should NOT crash the fold — degrade to Option-1
    /// bulk summary applied to every block.  This keeps user-facing
    /// behaviour identical to the pre-#4866 single-group fast-path so
    /// existing call sites that rely on `messages_replaced > 0` still
    /// signal correctly.
    #[tokio::test]
    async fn parse_failure_falls_back_to_raw_response_as_bulk_summary() {
        let messages = build_history(10);
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver(
            "not json at all — just prose the model produced".to_string(),
        ));

        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert!(result.messages_replaced >= 8);
        // groups_used_fallback stays 0 because the LLM call itself
        // succeeded — only the JSON parse failed.  This matters for
        // observability: an operator chasing "why is fold falling back"
        // can distinguish "aux unreachable" from "model produced prose".
        assert_eq!(result.groups_used_fallback, 0);
        // Every stale block should carry the raw response prose.
        let all_stale_carry_raw_prose =
            out.iter()
                .filter(|m| has_folded_tool_result(m))
                .all(|m| match &m.content {
                    MessageContent::Blocks(blocks) => blocks.iter().all(|b| match b {
                        ContentBlock::ToolResult { content, .. } => content.contains("just prose"),
                        _ => true,
                    }),
                    _ => false,
                });
        assert!(
            all_stale_carry_raw_prose,
            "parse failure must apply the raw response as bulk summary, not the static stub"
        );
    }

    /// JSON parse succeeds but every returned `id` is bogus (no overlap
    /// with stale `tool_use_id`s).  The Ok-path runs, the per-block
    /// apply loop finds no matching summary, every block falls back to
    /// the static stub.  `groups_used_fallback` stays 0 (the aux-LLM
    /// call itself succeeded — the operator-visible drift is surfaced
    /// via the unmatched-ids warn, not this counter).
    #[tokio::test]
    async fn parse_succeeds_but_all_ids_bogus_falls_back_per_block() {
        let messages = build_history(10); // tid_0..tid_9
        let bogus = r#"[{"id":"not_a_real_id","summary":"won't match anything"}]"#;
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver(bogus.to_string()));

        let (out, result) = fold_stale_tool_results(
            messages,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        // Ok-path → aux-call-failure counter stays 0 even though every
        // block ended up on the static stub.
        assert_eq!(result.groups_used_fallback, 0);
        // But every folded block should carry the static FALLBACK_SUMMARY
        // marker, not arbitrary text.
        let stale_count = out
            .iter()
            .filter(|m| match &m.content {
                MessageContent::Blocks(blocks) => blocks.iter().any(|b| match b {
                    ContentBlock::ToolResult { content, .. } => {
                        content.starts_with(FOLD_PREFIX)
                            && content.contains("summarisation unavailable")
                    }
                    _ => false,
                }),
                _ => false,
            })
            .count();
        assert!(
            stale_count >= 8,
            "every stale block must carry the static fallback stub when no \
             returned id matches; saw {stale_count}"
        );
    }

    /// `apply_fold_rewrites` must walk a separate message list (typically
    /// `session.messages`) and replay the rewrites by `tool_use_id`.
    /// Crucial for axis 2: the working clone and the durable list can
    /// drift in length / ordering, so id-matching is the only reliable
    /// way to project fold onto the durable record.
    #[test]
    fn apply_fold_rewrites_matches_by_tool_use_id_across_lists() {
        let mut durable = vec![
            user_msg("q"),
            assistant_msg("a"),
            tool_result_msg_with_id("tid_A", "shell", "raw output A"),
            assistant_msg("a2"),
            tool_result_msg_with_id("tid_B", "shell", "raw output B"),
        ];
        let mut rewrites = BTreeMap::new();
        rewrites.insert("tid_A".to_string(), "[history-fold] A summary".to_string());
        // tid_C does not exist in `durable` — must be silently skipped.
        rewrites.insert("tid_C".to_string(), "[history-fold] dangling".to_string());

        let changed = apply_fold_rewrites(&mut durable, &rewrites);
        assert!(changed, "expected at least one match (tid_A)");

        let a_content = match &durable[2].content {
            MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        assert_eq!(a_content, "[history-fold] A summary");

        // tid_B was not in the rewrites map — must remain untouched.
        let b_content = match &durable[4].content {
            MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        assert_eq!(b_content, "raw output B");
    }

    #[test]
    fn collect_stale_indices_boundary() {
        // Build: user, asst, tool, asst, tool, asst, tool  (3 asst turns)
        // fold_after=2 → turn at index 0 is stale; tool-result at index 2 is stale.
        let msgs = vec![
            user_msg("q"),
            assistant_msg("a0"),
            tool_result_msg("t", "r0"),
            assistant_msg("a1"),
            tool_result_msg("t", "r1"),
            assistant_msg("a2"),
            tool_result_msg("t", "r2"),
        ];
        let stale = collect_stale_indices(&msgs, 2);
        // Tool-result at index 2 should be stale (before the last 2 assistant turns).
        assert!(stale.contains(&2), "index 2 should be stale, got {stale:?}");
        // Tool-result at index 4 should NOT be stale (within the last 2 turns).
        assert!(
            !stale.contains(&4),
            "index 4 should not be stale, got {stale:?}"
        );
    }

    #[tokio::test]
    async fn already_folded_stub_not_re_folded() {
        // Build history where one "stale" message is already a fold stub
        // (Blocks with a ToolResult.content prefixed by FOLD_PREFIX). It
        // must never be re-selected for folding regardless of turn count.
        let mut msgs = vec![user_msg("initial question")];
        msgs.push(assistant_msg("prior assistant turn"));
        msgs.push(Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tid_prior".to_string(),
                tool_name: "shell".to_string(),
                content: format!("{FOLD_PREFIX} prior summary text"),
                is_error: false,
                status: librefang_types::tool::ToolExecutionStatus::Completed,
                approval_request_id: None,
            }]),
            pinned: false,
            timestamp: None,
        });
        // Add enough assistant turns to push the stub into the stale window.
        for i in 0..10 {
            msgs.push(assistant_msg(&format!("response {i}")));
            msgs.push(tool_result_msg("shell", &format!("output {i}")));
        }
        let driver: Arc<dyn LlmDriver> = Arc::new(OkDriver("new summary".to_string()));
        let (out, result) = fold_stale_tool_results(
            msgs,
            FoldConfig {
                fold_after_turns: 8,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        // The existing fold stub must still be present in the output unchanged.
        let prior_stub_present = out.iter().any(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().any(|b| match b {
                ContentBlock::ToolResult { content, .. } => {
                    content.starts_with(FOLD_PREFIX) && content.contains("prior summary text")
                }
                _ => false,
            }),
            _ => false,
        });
        assert!(
            prior_stub_present,
            "pre-existing fold stub must survive unchanged: {result:?}"
        );
    }

    #[test]
    fn pinned_messages_not_folded() {
        let mut msgs = vec![user_msg("q"), assistant_msg("a0")];
        // Pinned tool result — must not be folded.
        let mut pinned = tool_result_msg("t", "important pinned result");
        pinned.pinned = true;
        msgs.push(pinned);
        for _ in 0..8 {
            msgs.push(assistant_msg("ax"));
            msgs.push(tool_result_msg("t", "recent"));
        }
        let stale = collect_stale_indices(&msgs, 8);
        // The pinned message at index 2 must not appear.
        assert!(
            !stale.contains(&2),
            "pinned message should never be stale: {stale:?}"
        );
    }

    #[test]
    fn strip_code_fence_handles_json_label() {
        let fenced = "```json\n[{\"id\":\"x\",\"summary\":\"y\"}]\n```";
        let body = strip_code_fence(fenced).expect("should strip ```json fence");
        assert!(body.starts_with('['));
        assert!(body.ends_with(']'));
    }

    #[test]
    fn strip_code_fence_handles_bare_triple_backtick() {
        let fenced = "```\n[1,2,3]\n```";
        let body = strip_code_fence(fenced).expect("should strip ``` fence");
        assert_eq!(body, "[1,2,3]");
    }

    #[test]
    fn strip_code_fence_returns_none_when_unfenced() {
        assert!(strip_code_fence("plain text").is_none());
        assert!(strip_code_fence("[1,2,3]").is_none());
    }

    /// Regression for #5203: a verbose-JSON fold model that pretty-prints
    /// its output (one JSON key per line) must not be truncated mid-string
    /// at the production per-block token budget.
    ///
    /// The driver here is a [`TruncatingDriver`] that mirrors the real
    /// provider convention of ~4 characters per token: any response longer
    /// than `max_tokens * 4` bytes gets cut off.  With the pre-#5203 budget
    /// (64 tok/block, 256 floor) a realistic ~1680-byte verbose response is
    /// chopped mid-string and serde_json fails with "EOF while parsing a
    /// string" — every fold pass falls back to bulk-summary semantics.
    ///
    /// This test asserts both halves of the regression guard:
    /// 1. **Old budget would have failed** — when the response is
    ///    truncated at the pre-#5203 budget (256 tok × 4 chars/tok =
    ///    1024 bytes for a 4-block batch) the pretty-printed JSON gets
    ///    cut mid-summary and the parse step reports failure.  This is
    ///    exercised against [`parse_labeled_summaries`] directly so we
    ///    don't have to plumb a custom budget through `summarise_batch`.
    /// 2. **Current budget survives** — at the production floor
    ///    (`FOLD_TOKENS_FLOOR = 512`) the same verbose response parses
    ///    cleanly and each stale block receives its specific per-id
    ///    summary.
    #[tokio::test]
    async fn verbose_json_fold_output_not_truncated_mid_string() {
        // Build 4 stale tool-result messages (fold_after=2 with 6 total
        // assistant turns leaves turns 0-3 stale).
        let mut msgs = vec![user_msg("initial question")];
        for i in 0..6 {
            msgs.push(assistant_msg(&format!("assistant response {i}")));
            msgs.push(tool_result_msg_with_id(
                &format!("tid_{i}"),
                "shell_exec",
                &format!("output of turn {i}"),
            ));
        }

        // Realistic verbose pretty-printed response from a model like
        // gemma4 — each entry spans ~9 lines with longer summaries,
        // total ~1680 chars.  This is well above the pre-#5203
        // 256-token (1024-byte) ceiling but comfortably under the
        // post-fix 512-token (2048-byte) floor, so the byte-budget
        // simulation in [`TruncatingDriver`] separates the two regimes.
        let pretty_json = r#"[
  {
    "id": "tid_0",
    "summary": "The shell command executed successfully and listed all files in the current working directory including hidden dotfiles, build artifacts, vendored dependencies, the test fixture directory containing all the integration test data, the documentation tree under docs/, the build scripts in scripts/, and the workspace-level Cargo.lock recording every transitive crate version."
  },
  {
    "id": "tid_1",
    "summary": "The read_file tool returned the contents of the configuration file, revealing the database connection string, API endpoint settings, retry policy knobs, the cache eviction thresholds for the in-memory layer, the rate-limit defaults per provider, the OAuth client registration entries for each MCP server, and the telemetry exporter endpoints configured for the observability pipeline."
  },
  {
    "id": "tid_2",
    "summary": "The test runner completed all forty-seven unit tests with zero failures across the workspace crates, confirming that the recent refactoring of the persistence adapter did not introduce any regressions in the durability path, the message-history trimming logic, the trigger dispatch concurrency guards, or the agent-loop tool-result rewriting pipeline that the fold persistence axis depends on."
  },
  {
    "id": "tid_3",
    "summary": "The git commit tool staged and committed the changes to the feature branch with the conventional-commit message format as required by the contribution guide and the project's commit-msg server-side hook, then verified the working tree returned to a clean state with no leftover untracked files or staged hunks that could leak into a follow-up PR."
  }
]"#;

        // Sanity-check the regression itself: at the OLD budget the
        // driver would truncate the response, so the parser should
        // report failure.  This is the "test can actually falsify the
        // bug" half of the regression guard — without it, the happy-
        // path assertion below would pass even at the buggy 64/256
        // budget (because no real truncation was simulated).
        let old_budget_max_tokens: u32 = 256; // pre-#5203 floor (4 blocks × 64)
        let old_byte_limit = (old_budget_max_tokens as usize) * 4;
        let old_truncated: String = pretty_json.chars().take(old_byte_limit).collect();
        let parsed_old = parse_labeled_summaries(&old_truncated);
        assert!(
            parsed_old.is_err(),
            "pre-#5203 budget should truncate verbose JSON mid-string and fail to parse; \
             got Ok({:?}) which means the test cannot falsify the bug",
            parsed_old.ok()
        );

        // Happy path: production budget (`FOLD_TOKENS_FLOOR = 512`)
        // leaves the response intact.  Drive it through the full fold
        // pass so the per-block rewrite path is also exercised.
        let driver: Arc<dyn LlmDriver> = Arc::new(TruncatingDriver {
            full_response: pretty_json.to_string(),
        });

        let (out, result) = fold_stale_tool_results(
            msgs,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        assert_eq!(
            result.groups_used_fallback, 0,
            "verbose-JSON pretty-printed response must parse successfully — \
             groups_used_fallback > 0 indicates the truncation bug regressed"
        );
        assert_eq!(
            result.groups_folded, 1,
            "expected fold pass to have run for the 4 stale blocks"
        );

        // Verify each of the 4 stale blocks received its specific per-id summary.
        let by_id: BTreeMap<String, String> = out
            .iter()
            .flat_map(|m| match &m.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => Some((tool_use_id.clone(), content.clone())),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();

        assert!(
            by_id
                .get("tid_0")
                .is_some_and(|c| c.contains("listed all files")),
            "tid_0 must carry its verbose per-id summary"
        );
        assert!(
            by_id
                .get("tid_3")
                .is_some_and(|c| c.contains("feature branch")),
            "tid_3 must carry its verbose per-id summary"
        );
    }

    /// Behavioural guard for the per-block budget arithmetic (#5203
    /// review P1.2).  Asserts that `summarise_batch` requests at
    /// least `FOLD_TOKENS_FLOOR` for any small batch and at least
    /// `FOLD_TOKENS_PER_BLOCK * n` for any batch large enough to
    /// exceed the floor — capped only by `MAX_FOLD_OUTPUT_TOKENS`.
    /// Catches accidental regressions of the magic-number budget
    /// even without staging a real truncation driver.
    #[tokio::test]
    async fn summarise_batch_max_tokens_respects_floor_and_per_block() {
        // Small batch (1 block) → floor-bound.
        // 3 turns with fold_after=2 leaves turn 0 stale → 1 block.
        let small_msgs = build_history(3);

        let (rec, observed) = MaxTokensRecordingDriver::new();
        let driver: Arc<dyn LlmDriver> = Arc::new(rec);

        let _ = fold_stale_tool_results(
            small_msgs,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        let small_observed = observed.lock().unwrap().clone();
        assert_eq!(
            small_observed.len(),
            1,
            "expected exactly one batched call for the small fold pass"
        );
        assert!(
            small_observed[0] >= FOLD_TOKENS_FLOOR as u32,
            "small-batch max_tokens={} must be >= FOLD_TOKENS_FLOOR ({})",
            small_observed[0],
            FOLD_TOKENS_FLOOR
        );

        // Larger batch (8 blocks) → per-block-scaled.
        // 10 turns with fold_after=2 leaves turns 0-7 stale → 8 blocks.
        let large_msgs = build_history(10);

        let (rec2, observed2) = MaxTokensRecordingDriver::new();
        let driver2: Arc<dyn LlmDriver> = Arc::new(rec2);

        let _ = fold_stale_tool_results(
            large_msgs,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver2,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        let large_observed = observed2.lock().unwrap().clone();
        assert_eq!(large_observed.len(), 1);
        // 8 stale blocks × 128 tok/block = 1024, well under the cap.
        assert!(
            large_observed[0] >= (8 * FOLD_TOKENS_PER_BLOCK) as u32,
            "large-batch max_tokens={} must be >= 8 * FOLD_TOKENS_PER_BLOCK ({})",
            large_observed[0],
            8 * FOLD_TOKENS_PER_BLOCK
        );
    }

    /// Driver that records the `response_format` flag on every request,
    /// then returns a benign per-id JSON (same shape as
    /// [`MaxTokensRecordingDriver`], parameterised on the field we
    /// care about for #5287).
    struct ResponseFormatRecordingDriver {
        observed: Arc<std::sync::Mutex<Vec<Option<ResponseFormat>>>>,
    }

    impl ResponseFormatRecordingDriver {
        fn new() -> (Self, Arc<std::sync::Mutex<Vec<Option<ResponseFormat>>>>) {
            let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
            (
                ResponseFormatRecordingDriver {
                    observed: Arc::clone(&observed),
                },
                observed,
            )
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for ResponseFormatRecordingDriver {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.observed.lock().unwrap().push(req.response_format.clone());
            // Echo back per-id JSON so the fold proceeds end-to-end —
            // we only care about the recorded `response_format` here.
            let prompt = match &req.messages[0].content {
                MessageContent::Blocks(blocks) => match &blocks[0] {
                    ContentBlock::Text { text, .. } => text.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            let mut entries: Vec<String> = Vec::new();
            for line in prompt.lines() {
                if let Some(rest) = line.strip_prefix('[') {
                    if let Some(end) = rest.find(']') {
                        let id = &rest[..end];
                        entries.push(format!("{{\"id\":\"{id}\",\"summary\":\"ok\"}}"));
                    }
                }
            }
            let body = format!("[{}]", entries.join(","));
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: body,
                    provider_metadata: None,
                }],
                tool_calls: vec![],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                usage: librefang_types::message::TokenUsage::default(),
                actual_provider: None,
            })
        }
    }

    /// Regression guard for #5287 — `batch_summarise` must pin
    /// `response_format = Some(ResponseFormat::Json)` on the aux-LLM
    /// request. Without it, DeepSeek / OpenAI / Mistral / Gemini are
    /// free to emit natural-language numbered lists,
    /// `parse_labeled_summaries` fails, and the fold silently
    /// degrades to "applying raw response as bulk summary" — losing
    /// per-tool_use_id granularity on every long session.
    #[tokio::test]
    async fn fold_request_pins_response_format_json_for_aux_llm() {
        let msgs = build_history(3);

        let (rec, observed) = ResponseFormatRecordingDriver::new();
        let driver: Arc<dyn LlmDriver> = Arc::new(rec);

        let _ = fold_stale_tool_results(
            msgs,
            FoldConfig {
                fold_after_turns: 2,
                min_batch_size: 1,
            },
            "test-model",
            None,
            driver,
            librefang_types::model_catalog::ReasoningEchoPolicy::None,
        )
        .await;

        let observed = observed.lock().unwrap().clone();
        assert_eq!(
            observed.len(),
            1,
            "expected exactly one batched call for this fold pass"
        );
        assert_eq!(
            observed[0],
            Some(ResponseFormat::Json),
            "fold aux-LLM request must pin response_format = Json so \
             strict-output providers (DeepSeek / OpenAI / Mistral / \
             Gemini) emit a JSON array instead of free-form prose"
        );
    }
}
