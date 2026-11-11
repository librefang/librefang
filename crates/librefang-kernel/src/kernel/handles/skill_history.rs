//! [`kernel_handle::SkillHistory`] — record skill version-history snapshots
//! (#8021). The agent-facing `skill_evolve_*` tools write through
//! `librefang_skills::evolution` (filesystem-only) and reach the history store
//! through this role-trait method. The API write paths do not go through here —
//! they hold the `AppState` and call `SkillVersionStore` directly.

use librefang_runtime::kernel_handle;

use super::super::LibreFangKernel;

impl kernel_handle::SkillHistory for LibreFangKernel {
    fn record_skill_version(
        &self,
        skill_name: &str,
        manifest_toml: &str,
        prompt_context: &str,
        author: &str,
        source: &str,
    ) -> kernel_handle::KernelResult<()> {
        let store = librefang_memory::SkillVersionStore::new(self.memory.substrate.pool());
        store.record_version(skill_name, manifest_toml, prompt_context, author, source)
    }
}
