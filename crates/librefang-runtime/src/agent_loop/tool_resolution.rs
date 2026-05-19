//! Per-loop resolution and caching of the `tools` field passed to the LLM.
//!
//! Splits the lazy-mode tool gating logic (#3044) and the per-iteration
//! `ToolDefinition` clone cache (#3586) out of the main loop so the
//! caching contract has a single owner instead of being inlined alongside
//! the dispatch path.

use librefang_types::tool::ToolDefinition;

/// Lazy tool loading kicks in only when the agent's granted-tool set is
/// larger than this threshold. Below this, it's cheaper and simpler to just
/// ship everything — the agent has already been restricted (by profile or
/// explicit `capabilities.tools`) and the payload is already small.
///
/// ~30 is roughly `ALWAYS_NATIVE_TOOLS.len() + 10`, so any agent with a
/// trimmed-down profile (Coding = 5, Research = 4, Messaging = 6) stays
/// well below and bypasses lazy mode entirely. Agents with access to the
/// full ~75 builtin catalog (issue #3044's problem case) trigger lazy mode.
pub(super) const LAZY_TOOLS_THRESHOLD: usize = 30;

/// Build the `tools` field for a `CompletionRequest`. When `lazy_mode` is on
/// AND the granted-tool set is large enough to benefit AND `tool_load` is
/// actually reachable from the granted set, ship only the always-native
/// subset plus any tools the LLM has loaded this turn via `tool_load(name)`.
/// The LLM can discover + load more on-demand.
///
/// When lazy_mode is off, the set is already small, or `tool_load` is not in
/// the allowlist (the LLM would have no way to pull a stripped tool back in),
/// pass everything through so behavior matches the eager baseline. Missing
/// `tool_load` in an allowlisted-but-over-threshold agent was a silent
/// tool-disappearance bug — see issue #3044 follow-up review.
pub(super) fn resolve_request_tools(
    available_tools: &[ToolDefinition],
    session_loaded: &[ToolDefinition],
    lazy_mode: bool,
) -> Vec<ToolDefinition> {
    let has_tool_load = available_tools.iter().any(|t| t.name == "tool_load");
    if !lazy_mode || available_tools.len() <= LAZY_TOOLS_THRESHOLD || !has_tool_load {
        return available_tools.to_vec();
    }
    let mut out = crate::tool_runner::select_native_tools(available_tools);
    let seen: std::collections::HashSet<String> = out.iter().map(|t| t.name.clone()).collect();
    for t in session_loaded {
        if !seen.contains(&t.name) {
            out.push(t.clone());
        }
    }
    out
}

/// Per-loop cache for the resolved tool list passed into `CompletionRequest`.
///
/// Before #3586 the agent loop called `resolve_request_tools` (which cloned
/// every `ToolDefinition` via `available_tools.to_vec()`) on every iteration,
/// even though the granted-tool set is constant for the duration of a turn
/// and the lazy-mode fallback only grows when the LLM successfully invokes
/// `tool_load`.  This cache hands out a shared `Arc<Vec<ToolDefinition>>` and
/// only rebuilds when the lazy-mode `session_loaded_tools` vector grew since
/// the last iteration — turning the per-iteration cost from a deep clone of
/// the entire tool catalog into a refcount bump.
pub(super) struct ResolvedToolsCache {
    cached: std::sync::Arc<Vec<ToolDefinition>>,
    /// Snapshot of `session_loaded_tools.len()` at the time `cached` was
    /// built.  Length-only is sufficient because `session_loaded_tools` is
    /// only ever mutated via `push()` in the loop — never reordered or
    /// removed — so a stable length implies stable content.
    cached_loaded_len: usize,
    lazy_mode: bool,
}

impl ResolvedToolsCache {
    pub(super) fn new(
        available_tools: &[ToolDefinition],
        session_loaded: &[ToolDefinition],
        lazy_mode: bool,
    ) -> Self {
        Self {
            cached: std::sync::Arc::new(resolve_request_tools(
                available_tools,
                session_loaded,
                lazy_mode,
            )),
            cached_loaded_len: session_loaded.len(),
            lazy_mode,
        }
    }

    /// Return a cheap `Arc` clone of the resolved tool list, rebuilding only
    /// when the lazy-mode loaded-tool set has grown since the last call.
    pub(super) fn get(
        &mut self,
        available_tools: &[ToolDefinition],
        session_loaded: &[ToolDefinition],
    ) -> std::sync::Arc<Vec<ToolDefinition>> {
        if self.lazy_mode && session_loaded.len() != self.cached_loaded_len {
            self.cached = std::sync::Arc::new(resolve_request_tools(
                available_tools,
                session_loaded,
                self.lazy_mode,
            ));
            self.cached_loaded_len = session_loaded.len();
        }
        std::sync::Arc::clone(&self.cached)
    }
}
