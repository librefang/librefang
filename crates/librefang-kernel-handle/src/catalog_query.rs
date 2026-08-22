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

    /// Whether the given model supports vision (image) input, resolved from
    /// the model catalog's effective capabilities (#6010). Consulted at
    /// request-build time to decide whether image content blocks may be sent
    /// to the model or must be redacted to a text placeholder first —
    /// text-only OpenAI-compatible models reject image content parts with
    /// HTTP 400 (`unknown variant image_url, expected text`).
    ///
    /// Default impl returns `true` (fail open) so non-overriding mocks and
    /// stubs keep sending images unchanged. The real kernel impl applies user
    /// capability overrides (#4745) via `effective_capabilities` and also
    /// fails open on a catalog miss, so vision-capable models are never
    /// degraded.
    fn supports_vision_for(&self, _model: &str) -> bool {
        true
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

    /// Resolve a model-router profile by name.
    ///
    /// Lets the runtime turn a profile name — `"quick"`, `"coder"` — into the
    /// provider/model pair it stands for, without depending on
    /// `librefang-kernel`, which owns the catalog and depends on the runtime
    /// in the other direction. This is the same trait-injection seam the rest
    /// of `CatalogQuery` uses.
    ///
    /// Returns `None` when no profile of that name exists, so callers can
    /// tell "unknown profile" apart from "no profile asked for".
    ///
    /// Default impl returns `None` so existing stubs and tooling don't have to
    /// opt in; the real kernel impl consults the resolved profile catalog.
    fn resolve_model_profile(
        &self,
        _name: &str,
    ) -> Option<librefang_types::model_profile::ModelProfile> {
        None
    }

    /// The names of every profile in the resolved catalog, ordered.
    ///
    /// Used to make an "unknown profile" failure actionable by telling the
    /// caller what it could have asked for instead. Ordered so the message is
    /// byte-identical across processes (#3298) — it reaches an agent's
    /// conversation as a tool error, and unstable ordering there would churn
    /// the prompt cache on every retry.
    fn model_profile_names(&self) -> Vec<String> {
        Vec::new()
    }
}
