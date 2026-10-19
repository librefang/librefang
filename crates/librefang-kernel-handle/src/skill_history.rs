// ============================================================================
// SkillHistory — record skill version-history snapshots (#8021)
//
// The agent-facing `skill_evolve_*` tools mutate a skill's `skill.toml` +
// `prompt_context.md` through `librefang_skills::evolution` directly, which
// holds no database handle: the evolution module is filesystem-only, and the
// runtime deliberately does not reach into the kernel's SQLite pool. This
// trait is the seam that lets a tool hand the snapshot it just produced to the
// kernel's `SkillVersionStore`, the same way `AgentControl::create_agent_type`
// lets the `agent_type_create` tool reach the template history.
//
// The HTTP write paths do not need this seam — `librefang-api` already holds
// the `AppState` and calls the store directly — so this trait exists for the
// runtime half of the funnel. The kernel implementation is what writes; the
// default impl is a no-op so test stubs and non-kernel embedders do not have
// to opt in, matching the `CatalogQuery` defaults elsewhere in this crate.
// ============================================================================

use crate::KernelResult;

pub trait SkillHistory: Send + Sync {
    /// Record one version snapshot for `skill_name`.
    ///
    /// `manifest_toml` is the full `skill.toml` and `prompt_context` the full
    /// `prompt_context.md`; together they are the v1 snapshot. `author` names
    /// the surface that made the change (`agent:<id>` for tool calls) and
    /// `source` is the closed `change_source` vocabulary shared with the HTTP
    /// funnel (see [`librefang_types::skill_history::SkillWrite`]).
    ///
    /// **Best-effort contract.** Callers invoke this *after* the skill content
    /// is already on disk, so an `Err` costs one history row, never the write —
    /// callers log a warning and continue. The error is returned rather than
    /// swallowed here so a caller that *is* able to surface it can.
    ///
    /// Default impl is a no-op `Ok(())` so stubs and non-kernel callers keep
    /// compiling without implementing history.
    fn record_skill_version(
        &self,
        _skill_name: &str,
        _manifest_toml: &str,
        _prompt_context: &str,
        _author: &str,
        _source: &str,
    ) -> KernelResult<()> {
        Ok(())
    }
}
