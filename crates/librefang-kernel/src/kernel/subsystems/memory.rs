//! Memory subsystem — primary substrate, wiki vault, proactive memory,
//! and the prompt versioning store.
//!
//! Bundles five memory-side handles that previously sat as a flat
//! cluster on `LibreFangKernel`. The original `memory` field is
//! renamed to `substrate` here to avoid the
//! `self.memory.memory` collision once the subsystem is named
//! `memory`.

use std::sync::{Arc, OnceLock};

use librefang_memory::{MemorySubstrate, ProactiveMemoryStore, PromptStore};
use librefang_memory_wiki::WikiVault;
use librefang_runtime::proactive_memory::LlmMemoryExtractor;

/// Where memory extraction ended up, as boot actually wired it.
///
/// Two independent derivations of "which model extracts memories" is the bug
/// this type exists to prevent (#7828 follow-up): boot resolves a provider and
/// a model, builds a driver that can *fail*, and may be bypassed altogether by
/// an out-of-process sidecar — while a reporting surface that re-derives the
/// answer from `KernelConfig` sees none of that and confidently names a model
/// that is not running. Boot records the outcome here once; every surface reads
/// it.
///
/// This is deliberately a boot-time snapshot rather than a live view.
/// `HotAction::UpdateProactiveMemory` pushes the new `[proactive_memory]` table
/// onto the existing store but does **not** rebuild the extraction driver, so
/// after a reload the raw `extraction_model` setting and the model actually in
/// use legitimately differ — and the honest report is the one that says what is
/// running, not what the file now says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryExtractionResolution {
    /// Nothing extracts in this process: `[proactive_memory] enabled = false`,
    /// or both `auto_memorize` and `auto_retrieve` are off, so no store was
    /// built.
    Inactive,
    /// An LLM extractor is wired to `target`.
    Llm { target: MemoryExtractionTarget },
    /// An out-of-process `[proactive_memory.extractor_sidecar]` performs the
    /// extraction. No LLM is involved and any resolved model is bypassed, so
    /// naming one here would be a fiction.
    Sidecar { command: String },
    /// The LLM driver could not be built, so extraction fell back to substring
    /// matching with no model at all. `target` is what boot was *trying* to
    /// reach, which is the useful thing to show next to the failure.
    DegradedToSubstring {
        target: MemoryExtractionTarget,
        reason: String,
    },
}

impl MemoryExtractionResolution {
    /// Stable machine-readable discriminant for reporting surfaces.
    pub fn status(&self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::Llm { .. } => "llm",
            Self::Sidecar { .. } => "sidecar",
            Self::DegradedToSubstring { .. } => "degraded_substring",
        }
    }

    /// Whether an LLM performs the extraction at all.
    ///
    /// `false` for every path that reaches memories without a model — a
    /// disabled subsystem, a sidecar, or the substring fallback after a driver
    /// build failure.
    pub fn llm_active(&self) -> bool {
        matches!(self, Self::Llm { .. })
    }

    /// The provider and model that actually perform the extraction.
    ///
    /// `None` for every path where no model runs — deliberately including the
    /// degraded case. Naming a model there is the exact misreport this type
    /// exists to prevent: the driver failed to build, so that model is not
    /// extracting anything. What was attempted is in
    /// [`Self::degraded_reason`], which names the provider and model that
    /// failed.
    pub fn effective_target(&self) -> Option<&MemoryExtractionTarget> {
        match self {
            Self::Llm { target } => Some(target),
            Self::Inactive | Self::Sidecar { .. } | Self::DegradedToSubstring { .. } => None,
        }
    }

    /// The target boot resolved, whether or not it ended up usable.
    ///
    /// Only [`MemoryExtractionTarget::source`] is read off this — "did an
    /// operator pick this model" stays a meaningful question after a build
    /// failure.
    pub fn resolved_target(&self) -> Option<&MemoryExtractionTarget> {
        match self {
            Self::Llm { target } | Self::DegradedToSubstring { target, .. } => Some(target),
            Self::Inactive | Self::Sidecar { .. } => None,
        }
    }

    /// Human-readable explanation of why no LLM is in play, when none is.
    ///
    /// Names the provider and model that failed to build, so the one field
    /// that reports the failure also reports what was attempted.
    pub fn degraded_reason(&self) -> Option<&str> {
        match self {
            Self::DegradedToSubstring { reason, .. } => Some(reason.as_str()),
            Self::Inactive | Self::Llm { .. } | Self::Sidecar { .. } => None,
        }
    }
}

/// The provider and model memory extraction resolved to, already split.
///
/// `provider` and `model` are the two halves boot hands to the driver, after
/// `resolve_extraction_model_target` has picked the provider out of a
/// `provider:model` / `provider/model` spec and `strip_provider_prefix` has
/// taken the prefix back off the model name. Reporting the raw spec instead
/// leaks the unsplit form to every surface and cannot answer "which provider".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryExtractionTarget {
    /// `[proactive_memory] extraction_model` as boot read it; `None` when
    /// unset, meaning extraction inherits `[default_model]`.
    pub configured_spec: Option<String>,
    /// Provider the spec resolved to.
    pub provider: String,
    /// Model name in the form the upstream API receives — provider prefix
    /// stripped.
    pub model: String,
}

impl MemoryExtractionTarget {
    /// `"configured"` when an operator picked the model, `"inherited_default"`
    /// when it fell through to `[default_model]`.
    pub fn source(&self) -> &'static str {
        if self.configured_spec.is_some() {
            "configured"
        } else {
            "inherited_default"
        }
    }
}

/// Focused memory API.
pub trait MemorySubsystemApi: Send + Sync {
    /// Primary memory substrate handle.
    fn substrate_ref(&self) -> &Arc<MemorySubstrate>;
    /// Optional proactive memory store (initialised lazily).
    fn proactive_store(&self) -> Option<&Arc<ProactiveMemoryStore>>;
    /// What boot actually wired memory extraction to.
    ///
    /// `None` only before boot has reached the proactive-memory step; a booted
    /// kernel always records an outcome, including
    /// [`MemoryExtractionResolution::Inactive`].
    fn extraction_resolution(&self) -> Option<&MemoryExtractionResolution>;
}

/// Memory cluster — see module docs.
pub struct MemorySubsystem {
    /// Primary memory substrate (renamed from the original `memory`
    /// field — see module docs).
    pub(crate) substrate: Arc<MemorySubstrate>,
    /// Memory wiki vault (#3329). `None` when `[memory_wiki] enabled =
    /// false`.
    pub(crate) wiki_vault: Option<Arc<WikiVault>>,
    /// Proactive memory store (mem0-style auto_retrieve / auto_memorize).
    pub(crate) proactive_memory: OnceLock<Arc<ProactiveMemoryStore>>,
    /// Concrete handle to the LLM-backed memory extractor used by
    /// `proactive_memory`.
    pub(crate) proactive_memory_extractor: OnceLock<Arc<LlmMemoryExtractor>>,
    /// Prompt versioning and A/B experiment store.
    pub(crate) prompt_store: OnceLock<PromptStore>,
    /// What boot resolved memory extraction to — the single source of truth
    /// every reporting surface reads instead of re-deriving it from config.
    pub(crate) extraction_resolution: OnceLock<MemoryExtractionResolution>,
}

impl MemorySubsystem {
    pub(crate) fn new(substrate: Arc<MemorySubstrate>, wiki_vault: Option<Arc<WikiVault>>) -> Self {
        Self {
            substrate,
            wiki_vault,
            proactive_memory: OnceLock::new(),
            proactive_memory_extractor: OnceLock::new(),
            prompt_store: OnceLock::new(),
            extraction_resolution: OnceLock::new(),
        }
    }
}

impl MemorySubsystemApi for MemorySubsystem {
    #[inline]
    fn substrate_ref(&self) -> &Arc<MemorySubstrate> {
        &self.substrate
    }

    #[inline]
    fn proactive_store(&self) -> Option<&Arc<ProactiveMemoryStore>> {
        self.proactive_memory.get()
    }

    #[inline]
    fn extraction_resolution(&self) -> Option<&MemoryExtractionResolution> {
        self.extraction_resolution.get()
    }
}

#[cfg(test)]
mod extraction_resolution_tests {
    use super::*;

    fn target(configured: Option<&str>) -> MemoryExtractionTarget {
        MemoryExtractionTarget {
            configured_spec: configured.map(str::to_string),
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
        }
    }

    #[test]
    fn an_unset_spec_reports_as_inherited_and_a_set_one_as_configured() {
        assert_eq!(target(None).source(), "inherited_default");
        assert_eq!(target(Some("ollama/test-model")).source(), "configured");
    }

    #[test]
    fn a_live_llm_is_the_only_resolution_with_an_effective_target() {
        let llm = MemoryExtractionResolution::Llm {
            target: target(None),
        };
        assert_eq!(llm.status(), "llm");
        assert!(llm.llm_active());
        assert_eq!(
            llm.effective_target().map(|t| t.model.as_str()),
            Some("test-model")
        );
        assert_eq!(llm.degraded_reason(), None);
    }

    /// The regression this type exists for: a driver that failed to build must
    /// not hand any surface a model to call "effective". The model is still
    /// reachable through `resolved_target` — that is what `source` is read off,
    /// since "did someone choose this" stays answerable after a failure.
    #[test]
    fn a_failed_driver_build_has_no_effective_target_only_a_resolved_one() {
        let degraded = MemoryExtractionResolution::DegradedToSubstring {
            target: target(Some("ollama/test-model")),
            reason: "failed to build the ollama driver for extraction model test-model: nope"
                .to_string(),
        };
        assert_eq!(degraded.status(), "degraded_substring");
        assert!(!degraded.llm_active());
        assert!(
            degraded.effective_target().is_none(),
            "no model is effective when extraction has no LLM"
        );
        assert_eq!(
            degraded.resolved_target().map(|t| t.source()),
            Some("configured")
        );
        assert!(degraded
            .degraded_reason()
            .is_some_and(|r| r.contains("ollama") && r.contains("test-model")));
    }

    #[test]
    fn a_sidecar_and_an_inactive_subsystem_name_no_model_at_all() {
        for r in [
            MemoryExtractionResolution::Inactive,
            MemoryExtractionResolution::Sidecar {
                command: "/usr/local/bin/extractor".to_string(),
            },
        ] {
            assert!(!r.llm_active(), "{} must not claim an LLM", r.status());
            assert!(r.effective_target().is_none());
            assert!(r.resolved_target().is_none());
            assert_eq!(r.degraded_reason(), None);
        }
    }
}
