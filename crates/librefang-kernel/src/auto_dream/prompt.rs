//! The 4-phase "Orient → Gather → Consolidate → Prune" consolidation prompt.
//!
//! Ported from libre-code's `consolidationPrompt.ts` and adapted for the
//! librefang memory model. Differences from the source:
//!
//!   * No file system — memories live in the SQLite substrate, so the prompt
//!     directs the agent at its `memory_search` / `memory_save` tools rather
//!     than at `ls` / grepping transcript files.
//!   * No entrypoint file / MEMORY.md — librefang has no equivalent index.
//!     Pruning focuses on duplicate, stale, and contradicted fragments.
//!
//! The wording intentionally keeps the four-phase structure so users who've
//! seen libre-code's dream output recognise the pattern.

/// Build the dream message delivered to the target agent. The result is a
/// plain user message — no tool constraints baked in, because tool allowlists
/// are controlled by the agent manifest, not the prompt.
pub fn build_consolidation_prompt() -> String {
    r#"# Dream: Memory Consolidation

You are performing a dream — a reflective pass over your memory store. Synthesize what you've learned recently into durable, well-organized memories so future sessions can orient quickly.

---

## Phase 1 — Orient

- List or search your existing memories to see what's already there.
- Note which topics are well-covered and which are thin or missing.
- Skim categories so you improve existing memories rather than duplicating them.

## Phase 2 — Gather recent signal

Look for new information worth persisting. Sources in rough priority order:

1. **Recent conversations** — facts, preferences, and decisions from the last few sessions.
2. **Drifted memories** — stored facts that contradict something you know to be true now (the user corrected you, the code changed, the project pivoted).
3. **Implicit patterns** — recurring user preferences you've noticed but never explicitly recorded.

Don't exhaustively trawl — look for things you already suspect matter.

## Phase 3 — Consolidate

For each thing worth remembering:

- **Merge** into an existing memory if one covers the same topic — prefer updating over creating near-duplicates.
- **Convert relative dates** ("yesterday", "last week") to absolute dates so the memory stays interpretable after time passes.
- **Delete contradicted facts** — if today's investigation disproves an old memory, fix it at the source rather than adding a contradiction.

Focus on durable, actionable knowledge: preferences, non-obvious constraints, recurring pitfalls, project-specific vocabulary. Skip ephemeral task state.

## Phase 4 — Prune

- Remove memories that are stale, wrong, or superseded by a newer fragment.
- Collapse near-duplicates into a single canonical entry.
- Resolve contradictions — if two memories disagree, fix the wrong one.

---

Return a brief summary of what you consolidated, updated, or pruned. If nothing changed (your memory is already tight), say so.
"#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_four_phases() {
        let p = build_consolidation_prompt();
        assert!(p.contains("Phase 1 — Orient"));
        assert!(p.contains("Phase 2 — Gather"));
        assert!(p.contains("Phase 3 — Consolidate"));
        assert!(p.contains("Phase 4 — Prune"));
    }

    #[test]
    fn prompt_is_nonempty() {
        assert!(build_consolidation_prompt().len() > 200);
    }
}
