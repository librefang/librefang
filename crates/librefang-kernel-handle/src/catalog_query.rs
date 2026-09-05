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

use librefang_types::model_catalog::VisionSupport;

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

    /// What is known about the given model's vision (image) input support, resolved from the model catalog (#6010, refs #7957).
    /// Consulted at request-build time to decide whether image content blocks may be sent to the model or must be redacted to a text placeholder first — text-only OpenAI-compatible models reject image content parts with HTTP 400 (`unknown variant image_url, expected text`).
    ///
    /// Returns a tri-state rather than a `bool` because the gate has to treat a *declared* absence of vision support differently from an *unproven* one.
    /// #7957: a gateway-discovered model whose operator-chosen alias missed the catalog's name heuristic was recorded as blind, and the gate then removed the images with no error and no log — while a catalog miss, which knows strictly less, correctly kept sending them.
    /// [`VisionSupport::Unknown`] is what makes the two paths agree.
    ///
    /// Default impl returns [`VisionSupport::Unknown`], so non-overriding mocks and stubs keep sending images unchanged.
    /// The real kernel impl applies operator capability overrides (#4745), honours the entry's `vision_known` provenance flag, and returns `Unknown` on a catalog miss.
    fn vision_support_for(&self, _model: &str) -> VisionSupport {
        VisionSupport::Unknown
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

    /// The spawning agent's own per-agent model-router constraints, from its
    /// manifest `[model.router_override]` (#7789 review).
    ///
    /// Lets the runtime enforce the same constraints the per-turn profile
    /// router applies when `agent_spawn` names a profile. Without it,
    /// delegation is a way around every per-agent constraint the profile
    /// layer introduces: an agent budgeted at `cheap` could spawn a helper on
    /// the most expensive profile in the catalog, and a `fixed = true` pin
    /// would not bind the agents it spawns either.
    ///
    /// `agent_id` is the UUID string of the agent whose manifest should be
    /// consulted.
    ///
    /// The three outcomes are deliberately distinct, because this gates a
    /// spend cap and the two "no" answers mean opposite things:
    ///
    /// - `Ok(None)` — the agent resolved and declares no override, so it is
    ///   genuinely unconstrained.
    /// - `Ok(Some(_))` — the agent's constraints, to be enforced and passed on.
    /// - `Err(reason)` — the agent could **not** be looked up. A live agent
    ///   taking a turn always resolves, so this means something is wrong
    ///   rather than that the agent is unconstrained. Callers must fail closed:
    ///   the cost of guessing wrong is spend an operator had capped.
    ///
    /// Default impl returns `Ok(None)` so existing stubs and tooling keep
    /// treating every profile as permitted; the real kernel impl reads the
    /// agent's manifest from the registry.
    fn model_router_override_for(
        &self,
        _agent_id: &str,
    ) -> Result<Option<librefang_types::model_profile::AgentRouterOverride>, String> {
        Ok(None)
    }
}
