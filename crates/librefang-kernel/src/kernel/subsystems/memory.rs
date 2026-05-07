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

/// Focused memory API.
pub trait MemorySubsystemApi: Send + Sync {
    /// Primary memory substrate handle.
    fn substrate_ref(&self) -> &Arc<MemorySubstrate>;
    /// Optional proactive memory store (initialised lazily).
    fn proactive_store(&self) -> Option<&Arc<ProactiveMemoryStore>>;
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
}

impl MemorySubsystem {
    pub(crate) fn new(substrate: Arc<MemorySubstrate>, wiki_vault: Option<Arc<WikiVault>>) -> Self {
        Self {
            substrate,
            wiki_vault,
            proactive_memory: OnceLock::new(),
            proactive_memory_extractor: OnceLock::new(),
            prompt_store: OnceLock::new(),
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
}
