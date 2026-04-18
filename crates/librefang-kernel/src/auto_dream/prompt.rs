//! The 4-phase "Orient → Gather → Consolidate → Prune" consolidation prompt.
//!
//! Ported from libre-code's `consolidationPrompt.ts` and adapted for the
//! librefang memory model. Differences from the source:
//!
//!   * No file system — memories live in the SQLite substrate, so the prompt
//!     directs the agent at its `memory_recall` / `memory_store` tools rather
//!     than at `ls` / grepping transcript files.
//!   * No entrypoint file / MEMORY.md — librefang has no equivalent index.
//!     Pruning focuses on duplicate, stale, and contradicted fragments.
//!
//! The wording intentionally keeps the four-phase structure so users who've
//! seen libre-code's dream output recognise the pattern. Session IDs and
//! tool-constraint guidance are injected at build time (matching the source's
//! `buildConsolidationPrompt(memoryRoot, transcriptDir, extra)` signature)
//! so the model gets concrete targets to narrow its gather phase against.

/// Input to [`build_consolidation_prompt`]. Keeps the signature typed so
/// callers can't confuse positional arguments and forgetting a field is
/// caught at compile time.
pub struct ConsolidationPromptInput<'a> {
    /// Session IDs that were touched since this agent's last dream, newest
    /// first. Capped to a sensible size by the caller; the prompt just
    /// renders whatever it gets.
    pub session_ids: &'a [String],
    /// Total count of touched sessions. May exceed `session_ids.len()` when
    /// the caller truncated — we print this verbatim so the model sees the
    /// real workload.
    pub total_sessions: u32,
    /// Free-form agent-specific context the scheduler wants to inject
    /// (e.g. "agent prefers Chinese responses"). Passed through unchanged,
    /// empty string omits the section entirely.
    pub extra: &'a str,
}

/// Build the dream message delivered to the target agent. The prompt
/// concludes with tool-use guidance matching libre-code's constraint note
/// (which it enforces via a restricted `canUseTool` hook). librefang
/// enforces the same restriction in the kernel — `available_tools` is
/// filtered against [`super::DREAM_ALLOWED_TOOLS`] whenever the sender
/// channel is [`super::AUTO_DREAM_CHANNEL`] (see the filter in
/// `kernel/mod.rs`). The prompt mirrors that constraint as defence in
/// depth so the model doesn't try to call tools that were pre-stripped
/// from its schema.
pub fn build_consolidation_prompt(input: ConsolidationPromptInput<'_>) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(
        "# Dream: Memory Consolidation\n\
\n\
You are performing a dream — a reflective pass over your memory store. \
Synthesize what you've learned recently into durable, well-organized \
memories so future sessions can orient quickly.\n\
\n\
---\n\
\n\
## Phase 1 — Orient\n\
\n\
- Use `memory_recall` or `memory_list` to see what's already stored.\n\
- Note which topics are well-covered and which are thin or missing.\n\
- Skim categories so you improve existing memories rather than duplicating them.\n\
\n\
## Phase 2 — Gather recent signal\n\
\n\
Look for new information worth persisting. Sources in rough priority order:\n\
\n\
1. **Recent sessions** — facts, preferences, and decisions from the sessions listed below.\n\
2. **Drifted memories** — stored facts that contradict something you know to be true now (the user corrected you, the code changed, the project pivoted).\n\
3. **Implicit patterns** — recurring user preferences you've noticed but never explicitly recorded.\n\
\n\
Don't exhaustively trawl — look for things you already suspect matter.\n\
\n\
## Phase 3 — Consolidate\n\
\n\
For each thing worth remembering:\n\
\n\
- **Merge** into an existing memory if one covers the same topic — prefer updating over creating near-duplicates.\n\
- **Convert relative dates** (\"yesterday\", \"last week\") to absolute dates so the memory stays interpretable after time passes.\n\
- **Delete contradicted facts** — if today's investigation disproves an old memory, fix it at the source rather than adding a contradiction.\n\
\n\
Focus on durable, actionable knowledge: preferences, non-obvious constraints, recurring pitfalls, project-specific vocabulary. Skip ephemeral task state.\n\
\n\
## Phase 4 — Prune\n\
\n\
- Remove memories that are stale, wrong, or superseded by a newer fragment.\n\
- Collapse near-duplicates into a single canonical entry.\n\
- Resolve contradictions — if two memories disagree, fix the wrong one.\n\
\n\
---\n\
\n",
    );

    // Session list — matches libre-code's
    //   Sessions since last consolidation (N):
    //   - id1
    //   - id2
    // The model uses these as concrete grep targets rather than guessing.
    //
    // Three cases:
    //   total=0              → skip the section entirely
    //   total>0, ids non-empty → render header + bulleted list
    //   total>0, ids empty   → render header only (graceful fallback when
    //                          the ID lookup failed but the count succeeded)
    if input.total_sessions > 0 {
        out.push_str(&format!(
            "## Sessions to review\n\n{} session(s) touched since your last dream",
            input.total_sessions
        ));
        if !input.session_ids.is_empty() && input.session_ids.len() < input.total_sessions as usize
        {
            out.push_str(&format!(
                " (showing the {} most recent)",
                input.session_ids.len()
            ));
        }
        if input.session_ids.is_empty() {
            out.push_str(
                ". Use `memory_recall` to browse them — the IDs weren't available \
at prompt-build time.\n\n",
            );
        } else {
            out.push_str(":\n\n");
            for id in input.session_ids {
                out.push_str(&format!("- `{id}`\n"));
            }
            out.push('\n');
        }
    }

    // Tool constraints — mirrors libre-code's injected `extra` block. Note
    // the model is currently trusted to self-enforce; kernel-level
    // allowlisting for the `auto_dream` channel is a follow-up.
    out.push_str(
        "## Tool constraints\n\
\n\
The kernel restricts this session to memory tools only — attempts to call \
shell, file-edit, or network-mutating tools will be refused before the \
driver sees them. Stick to:\n\
\n\
- `memory_store` — write a new memory.\n\
- `memory_recall` — search existing memories.\n\
- `memory_list` — enumerate stored keys.\n\
\n\
Session transcripts live in the memory substrate — use `memory_recall` \
rather than trying to open files directly.\n\
\n",
    );

    if !input.extra.is_empty() {
        out.push_str("## Additional context\n\n");
        out.push_str(input.extra);
        if !input.extra.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str(
        "---\n\
\n\
Return a brief summary of what you consolidated, updated, or pruned. If \
nothing changed (your memory is already tight), say so.\n",
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_input() -> ConsolidationPromptInput<'static> {
        ConsolidationPromptInput {
            session_ids: &[],
            total_sessions: 0,
            extra: "",
        }
    }

    #[test]
    fn prompt_contains_four_phases() {
        let p = build_consolidation_prompt(empty_input());
        assert!(p.contains("Phase 1 — Orient"));
        assert!(p.contains("Phase 2 — Gather"));
        assert!(p.contains("Phase 3 — Consolidate"));
        assert!(p.contains("Phase 4 — Prune"));
    }

    #[test]
    fn prompt_includes_tool_constraints() {
        let p = build_consolidation_prompt(empty_input());
        assert!(p.contains("Tool constraints"));
        // Real tool names from librefang-runtime/src/tool_runner.rs, not
        // libre-code's `memory_save` / `memory_delete` which don't exist here.
        assert!(p.contains("memory_store"));
        assert!(p.contains("memory_recall"));
        assert!(p.contains("memory_list"));
    }

    #[test]
    fn prompt_omits_session_list_when_zero() {
        let p = build_consolidation_prompt(empty_input());
        assert!(!p.contains("Sessions to review"));
    }

    #[test]
    fn prompt_renders_session_ids() {
        let ids = vec!["sess-a".to_string(), "sess-b".to_string()];
        let p = build_consolidation_prompt(ConsolidationPromptInput {
            session_ids: &ids,
            total_sessions: 2,
            extra: "",
        });
        assert!(p.contains("2 session(s)"));
        assert!(p.contains("sess-a"));
        assert!(p.contains("sess-b"));
        assert!(!p.contains("most recent"));
    }

    #[test]
    fn prompt_flags_truncation_when_list_shorter_than_total() {
        let ids = vec!["sess-1".to_string()];
        let p = build_consolidation_prompt(ConsolidationPromptInput {
            session_ids: &ids,
            total_sessions: 50,
            extra: "",
        });
        assert!(p.contains("50 session(s)"));
        assert!(p.contains("showing the 1 most recent"));
    }

    #[test]
    fn prompt_falls_back_to_prose_when_total_positive_but_ids_empty() {
        // Mimics list_agent_sessions_touched_since failing while the count
        // query succeeded — render the count as a hint, don't emit an empty
        // bulleted list.
        let p = build_consolidation_prompt(ConsolidationPromptInput {
            session_ids: &[],
            total_sessions: 7,
            extra: "",
        });
        assert!(p.contains("7 session(s)"));
        assert!(p.contains("memory_recall"));
        // The fallback sentence is the positive signal. The negative
        // signal is that "most recent" (the truncation marker) does not
        // appear — we can't assert "no bullets" because the tool-constraints
        // section legitimately uses bulleted list items.
        assert!(!p.contains("most recent"));
        // And no session header bullet renders when the IDs are empty:
        // the "Sessions to review" section ends with the fallback prose,
        // not a list.
        let sessions_start = p.find("## Sessions to review").unwrap();
        let tool_start = p.find("## Tool constraints").unwrap();
        let sessions_block = &p[sessions_start..tool_start];
        assert!(
            !sessions_block.contains("- `"),
            "sessions block should contain the fallback prose, not bullets; got:\n{sessions_block}"
        );
    }

    #[test]
    fn prompt_appends_extra_section() {
        let p = build_consolidation_prompt(ConsolidationPromptInput {
            session_ids: &[],
            total_sessions: 0,
            extra: "Favour concise bullet points.",
        });
        assert!(p.contains("Additional context"));
        assert!(p.contains("Favour concise bullet points."));
    }
}
