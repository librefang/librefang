//! Skills + Hands subsystem — plugin skill registry, hand registry, and
//! the bookkeeping behind background skill reviews.
//!
//! Bundles the five fields that historically sat as a flat cluster on
//! `LibreFangKernel`. Hand instances and skills both materialise as
//! tools in the agent prompt and share the auto-review machinery, so
//! they cohabit a single subsystem.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use dashmap::DashMap;
use librefang_hands::registry::HandRegistry;
use librefang_skills::registry::SkillRegistry;
use tokio::sync::Semaphore;

/// Focused skills + hands API.
pub trait SkillsSubsystemApi: Send + Sync {
    /// Plugin skill registry handle.
    fn skill_registry_ref(&self) -> &std::sync::RwLock<SkillRegistry>;
    /// Curated hand registry.
    fn hand_registry_ref(&self) -> &HandRegistry;
}

/// Skill registry + hand registry + skill review bookkeeping —
/// see module docs.
pub struct SkillsSubsystem {
    /// Skill registry for plugin skills (`RwLock` for hot-reload on
    /// install/uninstall).
    pub(crate) skill_registry: std::sync::RwLock<SkillRegistry>,
    /// Hand registry — curated autonomous capability packages.
    pub(crate) hand_registry: HandRegistry,
    /// Generation counter for skill registry — bumped on every
    /// hot-reload. Used by the tool list cache to detect staleness.
    pub(crate) skill_generation: AtomicU64,
    /// Per-agent cooldown tracker for background skill reviews.
    pub(crate) skill_review_cooldowns: DashMap<String, i64>,
    /// Global in-flight review counter — caps concurrent background
    /// skill reviews kernel-wide.
    pub(crate) skill_review_concurrency: Arc<Semaphore>,
}

impl SkillsSubsystem {
    pub(crate) fn new(
        skill_registry: SkillRegistry,
        hand_registry: HandRegistry,
        max_inflight_skill_reviews: usize,
    ) -> Self {
        Self {
            skill_registry: std::sync::RwLock::new(skill_registry),
            hand_registry,
            skill_generation: AtomicU64::new(0),
            skill_review_cooldowns: DashMap::new(),
            skill_review_concurrency: Arc::new(Semaphore::new(max_inflight_skill_reviews)),
        }
    }
}

impl SkillsSubsystemApi for SkillsSubsystem {
    #[inline]
    fn skill_registry_ref(&self) -> &std::sync::RwLock<SkillRegistry> {
        &self.skill_registry
    }

    #[inline]
    fn hand_registry_ref(&self) -> &HandRegistry {
        &self.hand_registry
    }
}
