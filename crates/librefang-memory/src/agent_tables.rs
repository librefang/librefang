//! The one list of agent-scoped tables.
//!
//! Every table an agent's rows can live in appears exactly once, in [`AGENT_SCOPED_TABLES`], and both the `remove_agent` cascade and anything that has to answer "does this agent still have rows anywhere?" read it from here.
//! A second, hand-maintained copy of the list is precisely how `pending_approvals` came to survive `remove_agent` for eleven schema versions (audit: `agent-cascade-delete-missing-tables`), so the cascade builds its `DELETE`s from these entries rather than spelling them out.

use librefang_types::error::{LibreFangError, LibreFangResult};

/// Which of the three cascade helpers clears a table.
///
/// The split is not cosmetic: `StructuredStore::remove_agent` and `SessionStore::delete_agent_sessions` are each callable on their own and must clear exactly their own store's tables, while `MemorySubstrate::remove_agent` runs all three inside one transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTableGroup {
    /// Cleared by `execute_session_agent_deletes`.
    Session,
    /// Cleared by `execute_structured_agent_deletes`.
    Structured,
    /// Cleared by `execute_ephemeral_run_agent_deletes`.
    EphemeralRun,
}

/// Every table an agent's rows live in, as `(owning cascade, table, agent-scoping column)`.
///
/// The column belongs in the entry because the predicates are not uniform: `events` keys on `source_agent`, `ephemeral_runs` on `parent_agent_id`, `agents` on its own `id`, and everything else on `agent_id`.
///
/// Order matters within [`AgentTableGroup::Structured`] — it is the `DELETE` order, and `agents` stays last so the rows that foreign-key it are gone first.
///
/// Two things are deliberately absent.
/// `audit_entries` is agent-keyed but is the append-only WORM Merkle trail (#6553): each row's `prev_hash` links to the previous row's `hash`, so deleting an agent's rows opens a gap that is indistinguishable from tampering when `security verify` walks the chain.
/// An audit trail records what happened, including to agents later removed, so it is neither purged nor scanned.
/// (`approval_audit` below is a separate flat table with its own time-based retention, not the Merkle chain, so it does stay in the cascade.)
/// `experiment_metrics` and `experiment_variants` are agent-scoped only through `experiment_id IN (SELECT id FROM prompt_experiments WHERE agent_id = ?)`, which is not a `(table, column)` predicate; `execute_structured_agent_deletes` spells those two out and runs them before `prompt_experiments` is cleared, or the subquery would match nothing.
pub const AGENT_SCOPED_TABLES: &[(AgentTableGroup, &str, &str)] = &[
    (
        AgentTableGroup::Structured,
        "prompt_experiments",
        "agent_id",
    ),
    (AgentTableGroup::Structured, "prompt_versions", "agent_id"),
    (AgentTableGroup::Structured, "approval_audit", "agent_id"),
    (AgentTableGroup::Structured, "usage_events", "agent_id"),
    (AgentTableGroup::Structured, "memories", "agent_id"),
    (
        AgentTableGroup::Structured,
        "canonical_sessions",
        "agent_id",
    ),
    (AgentTableGroup::Structured, "kv_store", "agent_id"),
    (AgentTableGroup::Structured, "task_queue", "agent_id"),
    (AgentTableGroup::Structured, "entities", "agent_id"),
    (AgentTableGroup::Structured, "relations", "agent_id"),
    (AgentTableGroup::Structured, "events", "source_agent"),
    // pending_approvals (v26 — #3611) was missing from this cascade; the audit
    // found that on `remove_agent` the table would retain rows for the deleted
    // agent and a stale approval could fail-open on restart recovery.
    // (audit: agent-cascade-delete-missing-tables)
    (AgentTableGroup::Structured, "pending_approvals", "agent_id"),
    // goal_runs (v42) persists per-agent goal-run state; purge it on agent
    // removal so deleting an agent doesn't orphan its runs.
    (AgentTableGroup::Structured, "goal_runs", "agent_id"),
    (AgentTableGroup::Structured, "agents", "id"),
    // `sessions` and `sessions_fts` MUST be cleared together — `search_sessions`
    // reads from `sessions_fts` without joining `sessions`, so an orphan FTS row
    // leaves a deleted agent's content searchable (a privacy regression, #3501).
    (AgentTableGroup::Session, "sessions", "agent_id"),
    (AgentTableGroup::Session, "sessions_fts", "agent_id"),
    (
        AgentTableGroup::EphemeralRun,
        "ephemeral_runs",
        "parent_agent_id",
    ),
];

/// Run one cascade group's `DELETE`s for an agent inside the caller's transaction.
///
/// Table and column names come from [`AGENT_SCOPED_TABLES`], which is a compile-time constant, so the formatted SQL carries no caller-supplied identifiers; the agent id is bound as a parameter.
pub(crate) fn execute_agent_deletes(
    tx: &rusqlite::Transaction<'_>,
    agent_id: &str,
    group: AgentTableGroup,
) -> LibreFangResult<()> {
    for (owner, table, column) in AGENT_SCOPED_TABLES {
        if *owner != group {
            continue;
        }
        tx.execute(
            &format!("DELETE FROM {table} WHERE {column} = ?1"),
            rusqlite::params![agent_id],
        )
        .map_err(LibreFangError::memory)?;
    }
    Ok(())
}
