// ============================================================================
// CatalogQuery (#4842)
// ============================================================================
//
// Read-side projection of model-catalog metadata that drivers need at
// request-build time. Currently surfaces `reasoning_echo_policy_for(model)`
// so the OpenAI-compat driver can dispatch the right wire shape for
// `reasoning_content` per model by catalog lookup, replacing the substring
// match that lived in the driver. Default impl returns `None`, letting
// existing mocks and the legacy substring fallback continue to work for
// catalog misses.
// ============================================================================

pub trait CatalogQuery: Send + Sync {
    /// How the OpenAI-compatible driver must handle `reasoning_content`
    /// on historical assistant turns for the given model. Default impl
    /// returns [`librefang_types::model_catalog::ReasoningEchoPolicy::None`],
    /// which causes the driver to fall back to substring-based detection
    /// — see librefang/librefang#4842 for the migration plan.
    fn reasoning_echo_policy_for(
        &self,
        _model: &str,
    ) -> librefang_types::model_catalog::ReasoningEchoPolicy {
        librefang_types::model_catalog::ReasoningEchoPolicy::None
    }

    /// Resolve the effective proactive-memory `extraction_model` for the
    /// agent identified by `agent_id` (#5475). Looks at the agent's
    /// manifest `[proactive_memory] extraction_model` and falls back to
    /// the kernel-global `[proactive_memory] extraction_model`. Returns
    /// `None` when neither is set — the extractor then uses whatever
    /// model it was constructed with at boot.
    ///
    /// Default impl returns `None` so existing test stubs and tooling
    /// don't have to opt in; the real kernel impl threads through the
    /// agent registry + active `KernelConfig` to perform the lookup.
    fn proactive_memory_extraction_model_for(&self, _agent_id: &str) -> Option<String> {
        None
    }
}
