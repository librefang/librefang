//! Session history validation and repair.
//!
//! Before sending message history to the LLM, this module validates and
//! repairs common issues:
//! - Orphaned ToolResult blocks (no matching ToolUse)
//! - Misplaced ToolResults (not immediately after their matching ToolUse)
//! - Missing ToolResults for ToolUse blocks (synthetic error insertion)
//! - Duplicate ToolResults for the same tool_use_id
//! - Empty messages with no content
//! - Aborted assistant messages (empty blocks before tool results)
//! - Consecutive same-role messages (Anthropic API requires alternation)
//! - ToolResult blocks misplaced in assistant-role messages (crash artifacts)
//! - Oversized or potentially malicious tool result content

use librefang_types::message::{ContentBlock, Message, MessageContent, Role};
use librefang_types::tool::ToolExecutionStatus;
use std::collections::{HashMap, HashSet};
use tracing::{debug, warn};

/// Statistics from a repair operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairStats {
    /// Number of orphaned ToolResult blocks removed.
    pub orphaned_results_removed: usize,
    /// Number of empty messages removed.
    pub empty_messages_removed: usize,
    /// Number of consecutive same-role messages merged.
    pub messages_merged: usize,
    /// Number of ToolResults reordered to follow their ToolUse.
    pub results_reordered: usize,
    /// Number of synthetic error results inserted for unmatched ToolUse.
    pub synthetic_results_inserted: usize,
    /// Number of duplicate ToolResults removed.
    pub duplicates_removed: usize,
    /// Number of ToolResult blocks rescued from assistant-role messages.
    pub misplaced_results_rescued: usize,
    /// Number of synthetic error results inserted by the positional
    /// Phase 2a1 pair-aware check (distinct from Phase 2c `synthetic_results_inserted`).
    pub positional_synthetic_inserted: usize,
}

/// Validate and repair a message history for LLM consumption.
///
/// This ensures the message list is well-formed:
/// 1. Drops orphaned ToolResult blocks that have no matching ToolUse
/// 2. Drops empty messages
///    - 2a. Rescues ToolResult blocks from assistant-role messages (crash artifacts)
///    - 2a1. Enforces adjacent tool_result pairing per strict wire contract
///    - 2b. Reorders misplaced ToolResults to follow their matching ToolUse
///    - 2c. Inserts synthetic error results for unmatched ToolUse blocks
///    - 2d. Deduplicates ToolResults with the same tool_use_id
/// 3. Merges consecutive same-role messages
pub fn validate_and_repair(messages: &[Message]) -> Vec<Message> {
    validate_and_repair_with_stats(messages).0
}

/// Enhanced validate_and_repair that also returns statistics.
pub fn validate_and_repair_with_stats(messages: &[Message]) -> (Vec<Message>, RepairStats) {
    let mut stats = RepairStats::default();

    // Optimization: skip tool-related phases (1, 2a-2d) when the history
    // contains neither ToolUse nor ToolResult blocks. Only empty-message
    // removal, same-role merge, and text-coalesce are relevant for
    // plain-text sessions. We check both block kinds because orphan
    // ToolResults (no matching ToolUse) still need to be filtered out.
    let has_tool_blocks = messages.iter().any(message_has_tool_blocks);

    let mut cleaned: Vec<Message>;
    if has_tool_blocks {
        // Phase 1: Collect all ToolUse IDs from assistant messages
        let tool_use_ids: HashSet<String> = collect_tool_use_ids(messages);

        // Phase 2: Filter orphaned ToolResults and empty messages
        cleaned = Vec::with_capacity(messages.len());
        for msg in messages {
            let new_content = match &msg.content {
                MessageContent::Text(s) if is_empty_text_content(s) => {
                    stats.empty_messages_removed += 1;
                    continue;
                }
                MessageContent::Text(s) => MessageContent::Text(s.clone()),
                MessageContent::Blocks(blocks) => {
                    let original_len = blocks.len();
                    let filtered: Vec<ContentBlock> = blocks
                        .iter()
                        .filter(|b| match b {
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                let keep = tool_use_ids.contains(tool_use_id);
                                if !keep {
                                    stats.orphaned_results_removed += 1;
                                }
                                keep
                            }
                            _ => true,
                        })
                        .cloned()
                        .collect();
                    if filtered.is_empty() {
                        if original_len > 0 {
                            debug!(
                                role = ?msg.role,
                                original_blocks = original_len,
                                "Dropped message: all blocks filtered out"
                            );
                        }
                        stats.empty_messages_removed += 1;
                        continue;
                    }
                    MessageContent::Blocks(filtered)
                }
            };
            cleaned.push(Message {
                role: msg.role,
                content: new_content,
                pinned: msg.pinned,
                timestamp: msg.timestamp,
            });
        }

        // Phase 2a: Rescue ToolResult blocks stuck in assistant-role messages.
        let rescued_count = rescue_misplaced_tool_results(&mut cleaned);
        stats.misplaced_results_rescued = rescued_count;

        // Phase 2a1: Pair-aware positional validation of assistant tool_calls
        stats.positional_synthetic_inserted = enforce_adjacent_tool_result_pairs(&mut cleaned);

        // Phase 2b: Reorder misplaced ToolResults
        let reordered_count = reorder_tool_results(&mut cleaned);
        stats.results_reordered = reordered_count;

        // Phase 2c: Insert synthetic error results for unmatched ToolUse blocks
        let synthetic_count = insert_synthetic_results(&mut cleaned);
        stats.synthetic_results_inserted = synthetic_count;

        // Phase 2d: Deduplicate ToolResults
        let dedup_count = deduplicate_tool_results(&mut cleaned);
        stats.duplicates_removed = dedup_count;

        // Phase 2e: Skip aborted/errored assistant messages
        let pre_aborted_len = cleaned.len();
        cleaned = remove_aborted_assistant_messages(cleaned);
        let aborted_removed = pre_aborted_len - cleaned.len();
        if aborted_removed > 0 {
            stats.empty_messages_removed += aborted_removed;
            debug!(
                removed = aborted_removed,
                "Removed aborted assistant messages"
            );
        }
    } else {
        // No tool use in session — only remove empty messages and
        // aborted assistant messages (empty text / blank blocks).
        cleaned = messages
            .iter()
            .filter(|m| {
                if m.role == Role::Assistant && is_empty_or_blank_content(&m.content) {
                    stats.empty_messages_removed += 1;
                    return false;
                }
                match &m.content {
                    MessageContent::Text(s) => {
                        if is_empty_text_content(s) {
                            stats.empty_messages_removed += 1;
                            return false;
                        }
                        true
                    }
                    MessageContent::Blocks(b) => {
                        if is_empty_blocks_content(b) {
                            stats.empty_messages_removed += 1;
                            return false;
                        }
                        true
                    }
                }
            })
            .cloned()
            .collect();
    }

    // Phase 3: Merge consecutive same-role messages
    //
    // Anthropic's API requires each `ToolUse` block to be followed by its
    // matching `ToolResult` block in the very next message — they cannot
    // be separated by other text/tool blocks. A naive same-role merge can
    // break that invariant: e.g. merging
    //   Assistant[ToolUse#1] + Assistant[Text]   →   Assistant[ToolUse#1, Text]
    // leaves ToolUse#1 with no immediately-following ToolResult, and the
    // next API call returns 400 with no way to recover. Issue #2353.
    //
    // Skip the merge whenever it would splice across a tool-call boundary:
    //   • `last` ends with a ToolUse — the next message MUST be a
    //     same-shape ToolResult delivery, not a merged content blob.
    //   • `msg` is a pure tool-result delivery — keep it as its own
    //     message so the pairing stays intact.
    let pre_merge_len = cleaned.len();
    let mut merged: Vec<Message> = Vec::with_capacity(cleaned.len());
    for msg in cleaned {
        // Snapshot the would-be merge target's index before borrowing
        // `merged` mutably below — `merged.last_mut()` holds the borrow
        // for the rest of the if-let scope.
        let target_idx = merged.len().wrapping_sub(1);
        if let Some(last) = merged.last_mut() {
            if last.role == msg.role
                && !message_has_tool_use(last)
                && !message_is_only_tool_results(&msg)
                && !message_has_tool_use(&msg)
                && !message_is_only_tool_results(last)
            {
                let last_chars = content_char_len(&last.content);
                let msg_chars = content_char_len(&msg.content);
                let role = last.role;
                debug!(
                    target_idx,
                    role = ?role,
                    last_chars,
                    msg_chars,
                    "Merging consecutive same-role messages"
                );
                merge_content(&mut last.content, msg.content);
                stats.messages_merged += 1;
                continue;
            }
        }
        merged.push(msg);
    }
    let post_merge_len = merged.len();
    if pre_merge_len != post_merge_len {
        debug!(
            before = pre_merge_len,
            after = post_merge_len,
            "Merged consecutive same-role messages"
        );
    }

    // Normalize each message's blocks: collapse adjacent Text blocks into a
    // single Text. Why this lives here, not in each driver:
    //   • After consecutive same-role messages get merged above, a typical
    //     attachment send produces `Blocks([Text(attach_header+content),
    //     Text(user_prompt)])`. Provider APIs accept array content, but
    //     small chat-tuned local models behind Ollama / llama.cpp / vLLM /
    //     LM Studio frequently attend only to the first or last Text part
    //     and drop the rest — the user reports "the model didn't see my
    //     attachment". Frontier models handle multi-part fine, but they
    //     don't actually need it for plain-text payloads either; they
    //     happily read one big text part.
    //   • Image / ToolUse / ToolResult / Thinking blocks stay separate so
    //     vision and tool-calling pipelines are unchanged.
    // Doing it here keeps every driver's serialization logic simple and
    // delivers the same "attachments work everywhere" UX without a
    // backend-detection special case in each driver.
    let mut text_blocks_coalesced = 0usize;
    for msg in merged.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            let saved = coalesce_adjacent_text_blocks(blocks);
            text_blocks_coalesced += saved;
        }
    }
    if text_blocks_coalesced > 0 {
        debug!(
            text_blocks_coalesced,
            "Coalesced adjacent Text blocks within messages"
        );
    }

    // Distinguish "real repair" (data-integrity issues we had to clean
    // up) from "routine normalization" (consecutive same-role merge or
    // tool-result reordering — both are legitimate session-history
    // shapes that this pass intentionally collapses every turn).
    // `messages_merged` fires on every multi-turn streaming session with
    // back-to-back assistant chunks, so logging it at WARN trains
    // operators to ignore the message — and a real
    // `orphaned`/`synthetic`/`rescued`/`positional_synthetic`/
    // `duplicates`/`empty_messages` event later gets tuned out with it.
    let had_real_repair = stats.orphaned_results_removed > 0
        || stats.empty_messages_removed > 0
        || stats.synthetic_results_inserted > 0
        || stats.duplicates_removed > 0
        || stats.misplaced_results_rescued > 0
        || stats.positional_synthetic_inserted > 0;

    if had_real_repair {
        warn!(
            orphaned = stats.orphaned_results_removed,
            empty = stats.empty_messages_removed,
            merged = stats.messages_merged,
            reordered = stats.results_reordered,
            synthetic = stats.synthetic_results_inserted,
            duplicates = stats.duplicates_removed,
            rescued = stats.misplaced_results_rescued,
            positional_synthetic = stats.positional_synthetic_inserted,
            messages_before = pre_merge_len,
            messages_after = post_merge_len,
            "Session repair applied fixes"
        );
    } else if stats != RepairStats::default() {
        debug!(
            merged = stats.messages_merged,
            reordered = stats.results_reordered,
            messages_before = pre_merge_len,
            messages_after = post_merge_len,
            "Session repair normalized history (no integrity issues)"
        );
    }

    (merged, stats)
}

/// Ensure the message history starts with a user turn.
///
/// After context trimming the drain boundary may land on an assistant turn,
/// leaving it at position 0. Providers (especially Gemini) require the first
/// message to be from the user. This function drops leading assistant turns so
/// the history starts with a user turn.
///
/// After draining, it removes ToolResult blocks whose ToolUse no longer
/// survives. It intentionally does not run the full repair pipeline; callers
/// should run full repair before this function if they need global
/// normalization.
pub(crate) fn ensure_starts_with_user(mut messages: Vec<Message>) -> Vec<Message> {
    loop {
        match messages.iter().position(|m| m.role == Role::User) {
            Some(0) | None => break,
            Some(i) => {
                warn!(
                    dropped = i,
                    "Dropping leading assistant turn(s) to ensure history starts with user"
                );
                messages.drain(..i);
                let surviving_tool_use_ids: HashSet<String> = collect_tool_use_ids(&messages);
                for msg in &mut messages {
                    if let MessageContent::Blocks(blocks) = &mut msg.content {
                        blocks.retain(|b| match b {
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                surviving_tool_use_ids.contains(tool_use_id)
                            }
                            _ => true,
                        });
                    }
                }
                messages.retain(|m| match &m.content {
                    MessageContent::Text(s) => !s.is_empty(),
                    MessageContent::Blocks(b) => !b.is_empty(),
                });
            }
        }
    }
    messages
}

/// Phase 2a: Rescue ToolResult blocks from assistant-role messages.
///
/// After a crash, ToolResult blocks may end up inside an assistant-role message
/// instead of a user-role message. Per OpenAI/Moonshot API contract, tool results
/// MUST be in user-role messages. This pass extracts such misplaced ToolResult
/// blocks and moves them into a user-role message immediately after the assistant
/// message they were found in.
fn rescue_misplaced_tool_results(messages: &mut Vec<Message>) -> usize {
    // Collect (assistant_msg_idx, Vec<ToolResult blocks>) for assistant messages
    // that contain ToolResult blocks.
    let mut to_rescue: Vec<(usize, Vec<ContentBlock>)> = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role != Role::Assistant {
            continue;
        }
        if let MessageContent::Blocks(blocks) = &msg.content {
            let misplaced: Vec<ContentBlock> = blocks
                .iter()
                .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
                .cloned()
                .collect();
            if !misplaced.is_empty() {
                to_rescue.push((idx, misplaced));
            }
        }
    }

    if to_rescue.is_empty() {
        return 0;
    }

    let total_rescued: usize = to_rescue.iter().map(|(_, blocks)| blocks.len()).sum();

    // Remove ToolResult blocks from assistant messages
    for (idx, _) in &to_rescue {
        if let MessageContent::Blocks(blocks) = &mut messages[*idx].content {
            blocks.retain(|b| !matches!(b, ContentBlock::ToolResult { .. }));
        }
    }

    // Insert rescued blocks into user-role messages after each assistant message.
    // Process in reverse order so indices stay valid during insertion.
    for (assistant_idx, rescued_blocks) in to_rescue.into_iter().rev() {
        let insert_pos = assistant_idx + 1;
        if insert_pos < messages.len() && messages[insert_pos].role == Role::User {
            // Append to existing user message
            if let MessageContent::Blocks(existing) = &mut messages[insert_pos].content {
                existing.extend(rescued_blocks);
            } else {
                let old = std::mem::replace(
                    &mut messages[insert_pos].content,
                    MessageContent::Text(String::new()),
                );
                let mut new_blocks = content_to_blocks(old);
                new_blocks.extend(rescued_blocks);
                messages[insert_pos].content = MessageContent::Blocks(new_blocks);
            }
        } else {
            // Create a new user message for the rescued blocks
            messages.insert(
                insert_pos.min(messages.len()),
                Message {
                    role: Role::User,
                    content: MessageContent::Blocks(rescued_blocks),
                    pinned: false,
                    timestamp: None,
                },
            );
        }

        debug!(
            assistant_idx,
            "Rescued ToolResult blocks from assistant-role message"
        );
    }

    // Remove any assistant messages that became empty after extraction
    messages.retain(|m| match &m.content {
        MessageContent::Text(s) => !s.is_empty(),
        MessageContent::Blocks(b) => !b.is_empty(),
    });

    total_rescued
}

/// Phase 2a1: Pair-aware positional validation of assistant tool_calls.
///
/// Returns the number of synthetic ToolResult blocks inserted.
fn enforce_adjacent_tool_result_pairs(messages: &mut Vec<Message>) -> usize {
    // For each assistant with ToolUse blocks, check the
    // IMMEDIATELY FOLLOWING message for satisfaction. Missing ids get
    // a synthetic inserted in the adjacent user (or a new user is inserted
    // / appended as needed).
    let mut positional_synthetic: usize = 0;
    let mut i: usize = 0;
    while i < messages.len() {
        // Extract tool_use_ids from this message if it's an assistant with uses.
        let ids_needed: Vec<String> = match (&messages[i].role, &messages[i].content) {
            (Role::Assistant, MessageContent::Blocks(blocks)) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        if ids_needed.is_empty() {
            i += 1;
            continue;
        }

        // Collect tool_use_ids from the adjacent (i+1) user message, if any.
        let adjacent_results: HashSet<String> = messages
            .get(i + 1)
            .filter(|m| m.role == Role::User)
            .and_then(|m| match &m.content {
                MessageContent::Blocks(bs) => Some(
                    bs.iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                Some(tool_use_id.clone())
                            }
                            _ => None,
                        })
                        .collect::<HashSet<String>>(),
                ),
                _ => None,
            })
            .unwrap_or_default();

        let missing: Vec<String> = ids_needed
            .into_iter()
            .filter(|id| !adjacent_results.contains(id))
            .collect();

        if missing.is_empty() {
            i += 1;
            continue;
        }

        let missing_count = missing.len();
        let synthetic_blocks: Vec<ContentBlock> = missing
            .into_iter()
            .map(|id| ContentBlock::ToolResult {
                tool_use_id: id,
                tool_name: String::new(),
                content: "[Tool execution was interrupted or lost]".to_string(),
                is_error: true,
                status: ToolExecutionStatus::Error,
                approval_request_id: None,
            })
            .collect();

        if i + 1 < messages.len() {
            if messages[i + 1].role == Role::User {
                // Amend the adjacent user: either extend its Blocks, or upgrade
                // its Text content to Blocks with the original text preserved.
                let next = &mut messages[i + 1];
                match &mut next.content {
                    MessageContent::Blocks(bs) => {
                        bs.extend(synthetic_blocks);
                    }
                    MessageContent::Text(_) => {
                        let old = std::mem::replace(
                            &mut next.content,
                            MessageContent::Text(String::new()),
                        );
                        let mut new_blocks = content_to_blocks(old);
                        new_blocks.extend(synthetic_blocks);
                        next.content = MessageContent::Blocks(new_blocks);
                    }
                }
            } else {
                // Next message is not a User — insert a new user message
                // immediately after this assistant.
                messages.insert(
                    i + 1,
                    Message {
                        role: Role::User,
                        content: MessageContent::Blocks(synthetic_blocks),
                        pinned: false,
                        timestamp: None,
                    },
                );
            }
        } else {
            // Tail of history — append a new user message.
            messages.push(Message {
                role: Role::User,
                content: MessageContent::Blocks(synthetic_blocks),
                pinned: false,
                timestamp: None,
            });
        }

        positional_synthetic += missing_count;
        // Skip the user we just amended/inserted.
        i += 2;
    }

    positional_synthetic
}

/// Phase 2b: Reorder misplaced ToolResults -- ensure each result follows its use.
///
/// Builds a map of tool_use_id to the index of the assistant message containing it.
/// For each user message containing ToolResults, checks if the previous message is
/// the correct assistant message. If not, moves the ToolResult to the correct position.
fn reorder_tool_results(messages: &mut Vec<Message>) -> usize {
    // Build map: tool_use_id → index of the assistant message containing it.
    // Ids that appear in more than one assistant turn are collision ids
    // (e.g. Moonshot/Kimi reuses per-completion counters like `memory_store:6`
    // across turns). Reordering by a collision id would move a result from one
    // turn to follow a different turn's ToolUse, corrupting the session.
    // Those ids are excluded from the index so Phase 2b leaves their results
    // in place (the existing `tool_use_index.get(id)` → None branch).
    // Phase 2d uses an identical guard pattern (see `deduplicate_tool_results`).
    let mut tool_use_turn_count: HashMap<String, usize> = HashMap::new();
    let mut first_idx: HashMap<String, usize> = HashMap::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Role::Assistant {
            if let MessageContent::Blocks(blocks) = &msg.content {
                for block in blocks {
                    if let ContentBlock::ToolUse { id, .. } = block {
                        *tool_use_turn_count.entry(id.clone()).or_insert(0) += 1;
                        first_idx.entry(id.clone()).or_insert(idx);
                    }
                }
            }
        }
    }
    // Only ids with exactly ONE producing assistant message are safe to reorder by.
    // Colliding ids (driver reuse across turns, e.g. Moonshot/Kimi) stay where
    // Phase 2a1 placed them.
    let tool_use_index: HashMap<String, usize> = first_idx
        .into_iter()
        .filter(|(id, _)| tool_use_turn_count.get(id).copied().unwrap_or(0) == 1)
        .collect();

    // Collect misplaced ToolResult blocks that need to move.
    // Track (msg_idx, tool_use_id, block, target_assistant_idx).
    let mut misplaced: Vec<(usize, String, ContentBlock, usize)> = Vec::new();

    for (msg_idx, msg) in messages.iter().enumerate() {
        if msg.role != Role::User {
            continue;
        }
        if let MessageContent::Blocks(blocks) = &msg.content {
            for block in blocks {
                if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                    if let Some(&assistant_idx) = tool_use_index.get(tool_use_id) {
                        let expected_idx = assistant_idx + 1;
                        if msg_idx != expected_idx {
                            misplaced.push((
                                msg_idx,
                                tool_use_id.clone(),
                                block.clone(),
                                assistant_idx,
                            ));
                        }
                    }
                }
            }
        }
    }

    if misplaced.is_empty() {
        return 0;
    }

    let reorder_count = misplaced.len();

    // Build a set of (msg_idx, tool_use_id) pairs that are misplaced,
    // so we only remove blocks from the specific messages they came from.
    let misplaced_sources: HashSet<(usize, String)> = misplaced
        .iter()
        .map(|(msg_idx, id, _, _)| (*msg_idx, id.clone()))
        .collect();

    // Remove misplaced blocks from their specific source messages only
    for (msg_idx, msg) in messages.iter_mut().enumerate() {
        if msg.role != Role::User {
            continue;
        }
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            blocks.retain(|b| {
                if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                    // Only remove if this specific (msg_idx, tool_use_id) is misplaced
                    !misplaced_sources.contains(&(msg_idx, tool_use_id.clone()))
                } else {
                    true
                }
            });
        }
    }

    // Remove any now-empty messages
    messages.retain(|m| match &m.content {
        MessageContent::Text(s) => !s.is_empty(),
        MessageContent::Blocks(b) => !b.is_empty(),
    });

    // Group misplaced results by their target assistant index.
    let mut insertions: HashMap<usize, Vec<ContentBlock>> = HashMap::new();
    for (_msg_idx, _id, block, assistant_idx) in misplaced {
        insertions.entry(assistant_idx).or_default().push(block);
    }

    // Re-index after removals: find current positions of assistant messages by
    // looking up their tool_use blocks.
    let mut current_assistant_positions: HashMap<usize, usize> = HashMap::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Role::Assistant {
            if let MessageContent::Blocks(blocks) = &msg.content {
                for block in blocks {
                    if let ContentBlock::ToolUse { id, .. } = block {
                        if let Some(&orig_idx) = tool_use_index.get(id) {
                            current_assistant_positions.insert(orig_idx, idx);
                        }
                    }
                }
            }
        }
    }

    // Insert in reverse order so indices remain valid
    let mut sorted_insertions: Vec<(usize, Vec<ContentBlock>)> = insertions.into_iter().collect();
    sorted_insertions.sort_by_key(|b| std::cmp::Reverse(b.0));

    for (orig_assistant_idx, blocks) in sorted_insertions {
        if let Some(&current_idx) = current_assistant_positions.get(&orig_assistant_idx) {
            let insert_pos = (current_idx + 1).min(messages.len());
            // Check if there's already a user message at insert_pos with ToolResults
            // If so, append to it; otherwise create a new message.
            if insert_pos < messages.len() && messages[insert_pos].role == Role::User {
                if let MessageContent::Blocks(existing) = &mut messages[insert_pos].content {
                    existing.extend(blocks);
                } else {
                    let text_content = std::mem::replace(
                        &mut messages[insert_pos].content,
                        MessageContent::Text(String::new()),
                    );
                    let mut new_blocks = content_to_blocks(text_content);
                    new_blocks.extend(blocks);
                    messages[insert_pos].content = MessageContent::Blocks(new_blocks);
                }
            } else {
                messages.insert(
                    insert_pos,
                    Message {
                        role: Role::User,
                        content: MessageContent::Blocks(blocks),
                        pinned: false,
                        timestamp: None,
                    },
                );
            }
        }
    }

    reorder_count
}

/// Phase 2c: Insert synthetic error results for unmatched ToolUse blocks.
///
/// If an assistant message contains a ToolUse block but there is no matching
/// ToolResult anywhere in the history, a synthetic error result is inserted
/// immediately after the assistant message to prevent API validation errors.
fn insert_synthetic_results(messages: &mut Vec<Message>) -> usize {
    // Collect existing ToolResult IDs from user-role messages only.
    // ToolResult blocks in assistant-role messages are invalid per the API
    // contract and should have been rescued by Phase 2a already, but we
    // guard here as well to ensure orphaned tool_use IDs get synthetic results.
    let existing_result_ids: HashSet<String> = messages
        .iter()
        .filter(|m| m.role == Role::User)
        .flat_map(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => vec![],
        })
        .collect();

    // Find ToolUse blocks without matching results
    let mut orphaned_uses: Vec<(usize, String)> = Vec::new(); // (assistant_msg_idx, tool_use_id)
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Role::Assistant {
            if let MessageContent::Blocks(blocks) = &msg.content {
                for block in blocks {
                    if let ContentBlock::ToolUse { id, .. } = block {
                        if !existing_result_ids.contains(id) {
                            orphaned_uses.push((idx, id.clone()));
                        }
                    }
                }
            }
        }
    }

    if orphaned_uses.is_empty() {
        return 0;
    }

    let count = orphaned_uses.len();

    // Group by assistant message index
    let mut grouped: HashMap<usize, Vec<ContentBlock>> = HashMap::new();
    for (idx, tool_use_id) in orphaned_uses {
        grouped
            .entry(idx)
            .or_default()
            .push(ContentBlock::ToolResult {
                tool_use_id,
                tool_name: String::new(),
                content: "[Tool execution was interrupted or lost]".to_string(),
                is_error: true,
                status: ToolExecutionStatus::Error,
                approval_request_id: None,
            });
    }

    // Insert in reverse order so indices stay valid
    let mut sorted: Vec<(usize, Vec<ContentBlock>)> = grouped.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.0));

    for (assistant_idx, blocks) in sorted {
        let insert_pos = assistant_idx + 1;
        if insert_pos < messages.len() && messages[insert_pos].role == Role::User {
            // Check if this user message already has ToolResult blocks
            if let MessageContent::Blocks(existing) = &mut messages[insert_pos].content {
                existing.extend(blocks);
            } else {
                let old = std::mem::replace(
                    &mut messages[insert_pos].content,
                    MessageContent::Text(String::new()),
                );
                let mut new_blocks = content_to_blocks(old);
                new_blocks.extend(blocks);
                messages[insert_pos].content = MessageContent::Blocks(new_blocks);
            }
        } else {
            messages.insert(
                insert_pos.min(messages.len()),
                Message {
                    role: Role::User,
                    content: MessageContent::Blocks(blocks),
                    pinned: false,
                    timestamp: None,
                },
            );
        }
    }

    count
}

/// Phase 2d: Drop duplicate ToolResults for the same tool_use_id.
///
/// If multiple ToolResult blocks exist for the same tool_use_id across the
/// message history, keep the strongest result so approval placeholders can be
/// replaced by their later terminal outcome. Returns the count of duplicates removed.
fn deduplicate_tool_results(messages: &mut Vec<Message>) -> usize {
    // Ids that appear in more than one assistant turn are positional duplicates
    // (e.g. Moonshot reuses per-completion counters like `schedule_delete:6`).
    // Deduplicating them globally would remove legitimate per-turn results, so
    // we skip dedup for any id that is used by multiple assistant messages.
    let mut tool_use_turn_count: HashMap<String, usize> = HashMap::new();
    for msg in messages.iter() {
        if msg.role != Role::Assistant {
            continue;
        }
        if let MessageContent::Blocks(blocks) = &msg.content {
            for block in blocks {
                if let ContentBlock::ToolUse { id, .. } = block {
                    *tool_use_turn_count.entry(id.clone()).or_insert(0) += 1;
                }
            }
        }
    }
    let collision_ids: HashSet<String> = tool_use_turn_count
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(id, _)| id)
        .collect();

    let mut kept_results: HashMap<String, ToolExecutionStatus> = HashMap::new();

    for msg in messages.iter() {
        if let MessageContent::Blocks(blocks) = &msg.content {
            for block in blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    status,
                    ..
                } = block
                {
                    if collision_ids.contains(tool_use_id) {
                        continue;
                    }
                    kept_results
                        .entry(tool_use_id.clone())
                        .and_modify(|kept_status| {
                            if should_replace_kept_tool_result(*kept_status, *status) {
                                *kept_status = *status;
                            }
                        })
                        .or_insert(*status);
                }
            }
        }
    }

    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut removed = 0usize;

    for msg in messages.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            let before_len = blocks.len();
            blocks.retain(|b| {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    status,
                    ..
                } = b
                {
                    // Never dedup results whose id is shared across multiple turns.
                    if collision_ids.contains(tool_use_id) {
                        return true;
                    }
                    let keep_status = kept_results.get(tool_use_id).copied().unwrap_or(*status);
                    if seen_ids.contains(tool_use_id) || *status != keep_status {
                        return false;
                    }
                    seen_ids.insert(tool_use_id.clone());
                }
                true
            });
            removed += before_len - blocks.len();
        }
    }

    // Remove any messages that became empty after deduplication
    messages.retain(|m| match &m.content {
        MessageContent::Text(s) => !s.is_empty(),
        MessageContent::Blocks(b) => !b.is_empty(),
    });

    removed
}

fn should_replace_kept_tool_result(
    kept_status: ToolExecutionStatus,
    candidate_status: ToolExecutionStatus,
) -> bool {
    kept_status == ToolExecutionStatus::WaitingApproval
        && candidate_status != ToolExecutionStatus::WaitingApproval
}

/// Phase 2e: Remove empty assistant messages.
///
/// An assistant message with no content blocks (or only empty text / unknown
/// blocks) is always invalid. Providers like Moonshot/Kimi reject the whole
/// session with HTTP 400 ("assistant message must not be empty") when such a
/// message survives — including when it sits at the tail of the transcript.
/// This pass strips them unconditionally regardless of position (fixes #2809).
fn remove_aborted_assistant_messages(messages: Vec<Message>) -> Vec<Message> {
    let mut result = Vec::with_capacity(messages.len());

    for (i, msg) in messages.into_iter().enumerate() {
        if msg.role == Role::Assistant && is_empty_or_blank_content(&msg.content) {
            debug!(index = i, "Removing empty assistant message");
            continue;
        }
        result.push(msg);
    }

    result
}

/// Check if a message's content is effectively empty (no blocks or only empty text).
fn is_empty_or_blank_content(content: &MessageContent) -> bool {
    match content {
        MessageContent::Text(s) => is_empty_text_content(s),
        MessageContent::Blocks(blocks) => is_empty_blocks_content(blocks),
    }
}

fn is_empty_text_content(s: &str) -> bool {
    s.trim().is_empty()
}

fn is_empty_blocks_content(blocks: &[ContentBlock]) -> bool {
    blocks.is_empty()
        || blocks.iter().all(|b| match b {
            ContentBlock::Text { text, .. } => is_empty_text_content(text),
            ContentBlock::Unknown => true,
            _ => false,
        })
}

fn message_has_tool_blocks(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks.iter().any(|b| {
            matches!(
                b,
                ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. }
            )
        }),
        MessageContent::Text(_) => false,
    }
}

fn collect_tool_use_ids(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .filter_map(|m| match &m.content {
            MessageContent::Blocks(blocks) => Some(blocks),
            MessageContent::Text(_) => None,
        })
        .flat_map(|blocks| {
            blocks.iter().filter_map(|b| match b {
                ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
        })
        .collect()
}

/// Strip untrusted details from ToolResult content.
///
/// Prevents feeding potentially-malicious tool output details back to the LLM:
/// - Truncates to 10K chars maximum
/// - Strips base64 blobs (sequences >1000 chars of base64-like content)
/// - Removes potential prompt injection markers
pub fn strip_tool_result_details(content: &str) -> String {
    let max_len = 10_000;

    // First pass: strip base64-like blobs (long sequences of alphanumeric + /+= chars)
    let stripped = strip_base64_blobs(content);

    // Second pass: remove prompt injection markers
    let cleaned = strip_injection_markers(&stripped);

    // Final pass: truncate if needed
    if cleaned.len() <= max_len {
        cleaned
    } else {
        format!(
            "{}...[truncated from {} chars]",
            crate::str_utils::safe_truncate_str(&cleaned, max_len),
            cleaned.len()
        )
    }
}

/// Strip base64-like blobs longer than 1000 characters.
///
/// Identifies sequences that look like base64 (alphanumeric + /+=) and replaces
/// them with a placeholder if they exceed the length threshold.
fn strip_base64_blobs(content: &str) -> String {
    const BASE64_THRESHOLD: usize = 1000;
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Check if we're at the start of a potential base64 blob
        if is_base64_char(chars[i]) {
            let start = i;
            while i < chars.len() && is_base64_char(chars[i]) {
                i += 1;
            }
            let blob_len = i - start;
            if blob_len > BASE64_THRESHOLD {
                result.push_str(&format!("[base64 blob, {} chars removed]", blob_len));
            } else {
                // Short sequence, keep it
                for ch in &chars[start..i] {
                    result.push(*ch);
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Check if a character could be part of a base64 string.
fn is_base64_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='
}

/// Remove common prompt injection markers from content.
fn strip_injection_markers(content: &str) -> String {
    // These patterns are commonly used in prompt injection attempts
    const INJECTION_MARKERS: &[&str] = &[
        "<|system|>",
        "<|im_start|>",
        "<|im_end|>",
        "### SYSTEM:",
        "### System Prompt:",
        "[SYSTEM]",
        "<<SYS>>",
        "<</SYS>>",
        "IGNORE PREVIOUS INSTRUCTIONS",
        "Ignore all previous instructions",
        "ignore the above",
        "disregard previous",
    ];

    let mut result = content.to_string();
    let lower = result.to_lowercase();

    for marker in INJECTION_MARKERS {
        let marker_lower = marker.to_lowercase();
        // Case-insensitive replacement
        if lower.contains(&marker_lower) {
            // Find and replace case-insensitively
            let mut new_result = String::with_capacity(result.len());
            let mut search_pos = 0;
            let result_lower = result.to_lowercase();

            while let Some(found) = result_lower[search_pos..].find(&marker_lower) {
                let abs_pos = search_pos + found;
                new_result.push_str(&result[search_pos..abs_pos]);
                new_result.push_str("[injection marker removed]");
                search_pos = abs_pos + marker.len();
            }
            new_result.push_str(&result[search_pos..]);
            result = new_result;
        }
    }

    result
}

/// Remove NO_REPLY assistant turns and their preceding user-message triggers
/// from session history. Keeps the last `keep_recent` messages intact to avoid
/// pruning recent context.
pub fn prune_heartbeat_turns(messages: &mut Vec<Message>, keep_recent: usize) {
    if messages.len() <= keep_recent {
        return;
    }
    let prune_end = messages.len() - keep_recent;
    let mut to_remove = Vec::new();

    for (i, msg) in messages.iter().enumerate().take(prune_end) {
        if msg.role == Role::Assistant {
            // Delegate to the canonical silent-response detector so the
            // heartbeat prune logic stays in lock-step with the rest of the
            // runtime (single source of truth — see silent_response.rs).
            let is_no_reply = match &msg.content {
                MessageContent::Text(text) => crate::silent_response::is_silent_response(text),
                MessageContent::Blocks(blocks) => {
                    blocks.len() == 1
                        && matches!(&blocks[0], ContentBlock::Text { text, .. } if {
                            crate::silent_response::is_silent_response(text)
                        })
                }
            };
            if is_no_reply {
                to_remove.push(i);
                // Keep the preceding user message — it may contain useful context
                // even when the agent chose not to reply.
            }
        }
    }

    if to_remove.is_empty() {
        return;
    }

    to_remove.sort_unstable();
    to_remove.dedup();
    let pruned = to_remove.len();
    for idx in to_remove.into_iter().rev() {
        messages.remove(idx);
    }
    debug!(
        pruned,
        "Pruned heartbeat NO_REPLY turns from session history"
    );
}

/// In-place coalesce: if the block list contains runs of `ContentBlock::Text`,
/// merge each run into a single Text block (joined with a blank-line
/// separator). All other block kinds — Image, ImageFile, ToolUse,
/// ToolResult, Thinking, Unknown — are kept untouched and act as run
/// boundaries. Returns the number of blocks removed (i.e. how many merges
/// happened) so the caller can summarize the work.
///
/// Provider-side rationale lives at the call site in
/// `validate_and_repair_with_stats` — this is the pure transform.
fn coalesce_adjacent_text_blocks(blocks: &mut Vec<ContentBlock>) -> usize {
    if blocks.len() < 2 {
        return 0;
    }
    let original_len = blocks.len();
    let drained: Vec<ContentBlock> = std::mem::take(blocks);
    let mut out: Vec<ContentBlock> = Vec::with_capacity(drained.len());
    for block in drained {
        match block {
            ContentBlock::Text {
                text,
                provider_metadata,
            } => {
                if let Some(ContentBlock::Text {
                    text: existing,
                    provider_metadata: existing_meta,
                }) = out.last_mut()
                {
                    existing.push_str("\n\n");
                    existing.push_str(&text);
                    // Keep the first non-None provider_metadata; if both
                    // sides set it, keep the existing (older) value so we
                    // don't lose any field the provider needs to round-trip.
                    if existing_meta.is_none() {
                        *existing_meta = provider_metadata;
                    }
                    continue;
                }
                out.push(ContentBlock::Text {
                    text,
                    provider_metadata,
                });
            }
            other => out.push(other),
        }
    }
    *blocks = out;
    original_len.saturating_sub(blocks.len())
}

/// Diagnostic helper: rough char count of a message's text payload.
/// Used only for debug logging when consecutive same-role messages
/// are merged — gives operators a sense of "is this a tiny reconnect
/// duplicate or a large dropped streaming response?". Image data is
/// counted as `[image]` placeholder length, not the base64 size.
fn content_char_len(content: &MessageContent) -> usize {
    match content {
        MessageContent::Text(s) => s.chars().count(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .map(|b| match b {
                ContentBlock::Text { text, .. } => text.chars().count(),
                ContentBlock::Thinking { thinking, .. } => thinking.chars().count(),
                ContentBlock::ToolResult { content, .. } => content.chars().count(),
                ContentBlock::ToolUse { .. } => 16,
                ContentBlock::Image { .. } | ContentBlock::ImageFile { .. } => 8,
                ContentBlock::Unknown => 0,
            })
            .sum(),
    }
}

/// Merge the content of `src` into `dst`.
fn merge_content(dst: &mut MessageContent, src: MessageContent) {
    // Convert both to blocks, then append
    let dst_blocks = content_to_blocks(std::mem::replace(dst, MessageContent::Text(String::new())));
    let src_blocks = content_to_blocks(src);
    let mut combined = dst_blocks;
    combined.extend(src_blocks);
    *dst = MessageContent::Blocks(combined);
}

/// Convert MessageContent to a Vec<ContentBlock>.
fn content_to_blocks(content: MessageContent) -> Vec<ContentBlock> {
    match content {
        MessageContent::Text(s) => vec![ContentBlock::Text {
            text: s,
            provider_metadata: None,
        }],
        MessageContent::Blocks(blocks) => blocks,
    }
}

// ---------------------------------------------------------------------------
// Safe trim helpers
// ---------------------------------------------------------------------------

/// Check if a message contains any `ToolUse` blocks.
pub fn message_has_tool_use(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. })),
        _ => false,
    }
}

/// Check if a message contains only `ToolResult` blocks (i.e. it is a tool-
/// result delivery, not a fresh user question).
pub fn message_is_only_tool_results(msg: &Message) -> bool {
    match &msg.content {
        MessageContent::Blocks(blocks) => {
            !blocks.is_empty()
                && blocks
                    .iter()
                    .all(|b| matches!(b, ContentBlock::ToolResult { .. }))
        }
        _ => false,
    }
}

/// Find the latest safe trim point at or after `min_trim` that does **not**
/// split a ToolUse/ToolResult pair.
///
/// A "safe" trim point is an index where:
/// - `messages[index]` is a `User` message that is a fresh question (not only
///   ToolResult blocks), **or**
/// - `messages[index - 1]` is an `Assistant` message without pending ToolUse
///   blocks (the tool cycle completed).
///
/// Returns `None` only when no safe point exists (caller should fall back to
/// the original `min_trim` value).
pub fn find_safe_trim_point(messages: &[Message], min_trim: usize) -> Option<usize> {
    let len = messages.len();
    if min_trim >= len {
        return None;
    }

    // Upper bound: keep at least 2 messages after trim so the LLM has context.
    let upper = if len > 2 { len - 1 } else { len };

    // Scan forward from min_trim (prefer trimming slightly more over splitting pairs).
    for i in min_trim..upper {
        if is_safe_boundary(messages, i) {
            return Some(i);
        }
    }

    // No safe point forward — scan backward (trim less to avoid splitting).
    (0..min_trim).rev().find(|&i| is_safe_boundary(messages, i))
}

/// Returns `true` when index `i` is a clean conversation-turn boundary.
fn is_safe_boundary(messages: &[Message], i: usize) -> bool {
    let msg = &messages[i];

    // The message at the cut point must be a User message that is a fresh
    // question (not a ToolResult delivery).
    if msg.role != Role::User || message_is_only_tool_results(msg) {
        return false;
    }

    // If there is a preceding message it must be an Assistant message that
    // does NOT contain unresolved ToolUse blocks.
    if i > 0 {
        let prev = &messages[i - 1];
        if prev.role == Role::Assistant && message_has_tool_use(prev) {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests;
