//! Cluster pulled out of mod.rs in #4713 phase 3e/2.
//!
//! Hosts the kernel's self-handle wiring (`set_log_reloader`,
//! `set_self_handle`, `kernel_handle`) and the agent-binding management
//! surface (`list_bindings`, `add_binding`, `remove_binding`). Both
//! surfaces touch `OnceLock`/`Mutex` slots on `LibreFangKernel` and are
//! consumed by the API layer + boot sequence; grouping them keeps the
//! one-shot wiring helpers together with the per-binding mutators.
//!
//! Sibling submodule of `kernel::mod`, so it retains access to
//! `LibreFangKernel`'s private fields and inherent methods without any
//! visibility surgery.

use super::*;

impl LibreFangKernel {
    /// Install a [`crate::log_reload::LogLevelReloader`].
    ///
    /// Idempotent: subsequent calls are silently ignored (the slot is a
    /// `OnceLock`). The injected reloader is invoked when
    /// [`crate::config_reload::HotAction::ReloadLogLevel`] fires during
    /// hot-reload — see `apply_hot_actions_inner`.
    pub fn set_log_reloader(&self, reloader: crate::log_reload::LogLevelReloaderArc) {
        let _ = self.log_reloader.set(reloader);
    }

    /// Set the weak self-reference for trigger dispatch.
    ///
    /// Must be called once after the kernel is wrapped in `Arc`.
    pub fn set_self_handle(self: &Arc<Self>) {
        // The `self_handle` slot is a `OnceLock` — calling `set()` twice is
        // a silent no-op. Gate hook registration on the same first-call
        // signal so a defensive double-invocation doesn't register the
        // auto-dream hook twice (which would make every `AgentLoopEnd`
        // fire two spawned gate-check tasks that race on the file lock).
        if self.self_handle.set(Arc::downgrade(self)).is_ok() {
            // First call — wire up the AgentLoopEnd hook now that the Arc
            // exists so the handler can hold a Weak<Self>. Event-driven is
            // the primary trigger; the scheduler loop is a sparse (1-day)
            // backstop for agents that never finish a turn.
            self.hooks.register(
                librefang_types::agent::HookEvent::AgentLoopEnd,
                std::sync::Arc::new(crate::auto_dream::AutoDreamTurnEndHook::new(
                    Arc::downgrade(self),
                )),
            );
            // Skill workshop (#3328) — same wiring shape as auto_dream:
            // registers a Weak<Self>-holding handler on AgentLoopEnd so the
            // captured workflow's pending file write happens off the agent
            // loop's return path. Default-off; per-agent opt-in via
            // `[skill_workshop] enabled` in agent.toml — see
            // `crate::skill_workshop`.
            self.hooks.register(
                librefang_types::agent::HookEvent::AgentLoopEnd,
                std::sync::Arc::new(crate::skill_workshop::SkillWorkshopTurnEndHook::new(
                    Arc::downgrade(self),
                )),
            );
            // Best-effort cleanup of `.toml.tmp` orphans left over from
            // crashes mid-write between previous daemon runs. Pushed to
            // a background task so kernel boot stays off the FS path
            // entirely when no agent has skill_workshop enabled — the
            // public contract is "default-off, ~zero hot-path cost",
            // which includes startup, not just per-turn work.
            //
            // `set_self_handle` has historically been a sync call that
            // does not require a tokio runtime; we only spawn when one
            // happens to be current (production daemon boot via
            // `setup_router` and every `#[tokio::test]` test do have a
            // runtime; pure-sync test harnesses or Drop impls do not).
            // When no runtime is available we fall back to the inline
            // call so behaviour is unchanged for those callers.
            let pending_root = self.home_dir().join("skills");
            let prune = move || match crate::skill_workshop::storage::prune_orphan_temp_files(
                &pending_root,
            ) {
                Ok(0) => {}
                Ok(n) => tracing::info!(
                    pruned = n,
                    "skill_workshop: removed orphan .toml.tmp files left from a previous crash"
                ),
                Err(e) => tracing::warn!(
                    error = %e,
                    "skill_workshop: failed to scan pending dir for orphan tmp files"
                ),
            };
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    handle.spawn_blocking(prune);
                }
                Err(_) => prune(),
            }
            // Install the kernel-handle weak ref on the proactive-memory
            // extractor so its `extract_memories_with_agent_id` path can
            // route through `run_forked_agent_oneshot` for cache alignment
            // with the parent agent turn. Rule-based extractor (no LLM)
            // doesn't need this; it short-circuits before touching the
            // kernel. Safe to no-op when the extractor wasn't configured
            // (OnceLock::get returns None).
            if let Some(extractor) = self.proactive_memory_extractor.get() {
                let weak: std::sync::Weak<dyn librefang_runtime::kernel_handle::KernelHandle> =
                    Arc::downgrade(self) as _;
                extractor.install_kernel_handle(weak);
            }
        }
    }

    /// Upgrade the weak `self_handle` into a strong `Arc<dyn KernelHandle>`.
    ///
    /// Production call sites (cron dispatch, channel bridges, inter-agent
    /// tools, …) all need this conversion to plumb kernel access into the
    /// runtime's tool layer. Previously every site repeated a 4-line
    /// `self.self_handle.get().and_then(|w| w.upgrade()).map(|arc| arc as _)`
    /// incantation that produced an `Option`, then silently no-op'd downstream
    /// when the upgrade failed — masking bootstrap-order bugs (issue #3652).
    ///
    /// This helper panics instead. The `self_handle` slot is populated by
    /// [`Self::set_self_handle`] right after the kernel is wrapped in `Arc`,
    /// before any code path that dispatches an agent turn can run. Reaching
    /// this method with an empty slot means the bootstrap sequence was
    /// violated, which is a programmer error — fail loud, not silently.
    ///
    /// Public boundary methods that accept `Option<Arc<dyn KernelHandle>>`
    /// (`send_message_with_handle`, etc.) keep the `Option` for test stubs;
    /// they call this helper to materialize a handle when the caller passes
    /// `None`.
    pub(crate) fn kernel_handle(&self) -> Arc<dyn KernelHandle> {
        self.self_handle
            .get()
            .and_then(|w| w.upgrade())
            .map(|arc| arc as Arc<dyn KernelHandle>)
            .expect("kernel self_handle accessed before set_self_handle — bootstrap order bug")
    }

    // ─── Agent Binding management ──────────────────────────────────────

    /// List all agent bindings.
    pub fn list_bindings(&self) -> Vec<librefang_types::config::AgentBinding> {
        self.bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Add a binding at runtime.
    pub fn add_binding(&self, binding: librefang_types::config::AgentBinding) {
        let mut bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        bindings.push(binding);
        // Sort by specificity descending
        bindings.sort_by_key(|b| std::cmp::Reverse(b.match_rule.specificity()));
    }

    /// Remove a binding by index, returns the removed binding if valid.
    pub fn remove_binding(&self, index: usize) -> Option<librefang_types::config::AgentBinding> {
        let mut bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        if index < bindings.len() {
            Some(bindings.remove(index))
        } else {
            None
        }
    }
}
