//! [`kernel_handle::CatalogQuery`] (#4842) — read-side projection of the
//! model catalog used by drivers at request-build time.
//!
//! Currently surfaces `reasoning_echo_policy_for(model)` so the
//! OpenAI-compat driver can dispatch the right wire shape for
//! `reasoning_content` per model by catalog lookup, replacing a substring
//! match that lived in the driver. Looks up the model by id or alias; a
//! catalog miss returns `ReasoningEchoPolicy::None`, which signals the
//! driver to fall back to substring detection.

use librefang_runtime::kernel_handle;
use librefang_types::model_catalog::{ReasoningEchoPolicy, VisionSupport};

use super::super::LibreFangKernel;
use crate::kernel_api::KernelApi;

impl LibreFangKernel {
    /// Inherent mirror of [`kernel_handle::CatalogQuery::reasoning_echo_policy_for`]
    /// so `LibreFangKernel`'s own internal `CompletionRequest`-construction
    /// sites can dispatch the policy without bringing the `CatalogQuery`
    /// trait into scope.
    pub(crate) fn lookup_reasoning_echo_policy(&self, model: &str) -> ReasoningEchoPolicy {
        self.model_catalog_ref()
            .load()
            .find_model(model)
            .map(|entry| entry.reasoning_echo_policy)
            .unwrap_or_default()
    }

    /// Inherent mirror of [`kernel_handle::CatalogQuery::vision_support_for`] (#6010, refs #7957).
    ///
    /// Resolves what the catalog actually *knows* about the model's image-input support, honouring operator capability overrides (#4745).
    /// Three answers, not two: a catalog miss and an entry whose flag was inferred from the model's name both return [`VisionSupport::Unknown`], because they carry the same amount of information and the gate must therefore behave identically for both.
    /// Only a declared `supports_vision = false` returns `Unsupported`, and only that answer removes images from a request.
    pub(crate) fn lookup_vision_support(&self, model: &str) -> VisionSupport {
        self.model_catalog_ref().load().vision_support_for(model)
    }
}

impl kernel_handle::CatalogQuery for LibreFangKernel {
    fn reasoning_echo_policy_for(&self, model: &str) -> ReasoningEchoPolicy {
        self.lookup_reasoning_echo_policy(model)
    }

    fn vision_support_for(&self, model: &str) -> VisionSupport {
        self.lookup_vision_support(model)
    }

    /// Resolve the per-agent `extraction_model` for proactive memory
    /// (#5475). The chain is: agent manifest `[proactive_memory]
    /// extraction_model` → kernel-global `[proactive_memory]
    /// extraction_model` → `None`. Empty strings on either side are
    /// treated as unset.
    ///
    /// `agent_id` is the UUID string the proactive-memory store
    /// already stamps onto its `user_id` and forwards through the
    /// `_with_agent_id` extractor entry point. A malformed UUID
    /// returns `None` and the extractor falls back to the boot-time
    /// model — same behaviour as the pre-#5475 single-model path.
    fn proactive_memory_extraction_model_for(&self, agent_id: &str) -> Option<String> {
        use librefang_types::agent::AgentId;
        use std::str::FromStr;

        let aid = AgentId::from_str(agent_id).ok()?;
        let entry = self.agents.registry.get_arc(aid)?;
        let cfg = self.config.load();
        entry
            .manifest
            .proactive_memory
            .resolve_extraction_model(&cfg.proactive_memory)
    }
}
