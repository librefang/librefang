//! [`kernel_handle::ToolPolicy`] — per-tool timeout, env-passthrough policy,
//! channel upload directories, and read-only / named workspace prefixes.
//! Glob lookups for `tool_timeouts` use longest-pattern-wins to keep
//! resolution deterministic in the face of HashMap iteration order.

use librefang_runtime::kernel_handle;
use librefang_types::agent::*;

use super::super::{workspace_setup, LibreFangKernel};

impl kernel_handle::ToolPolicy for LibreFangKernel {
    fn tool_timeout_secs(&self) -> u64 {
        let cfg = self.config.load();
        cfg.tool_timeout_secs
    }

    fn tool_timeout_secs_for(&self, tool_name: &str) -> u64 {
        let cfg = self.config.load();
        // 1. Exact match
        if let Some(&t) = cfg.tool_timeouts.get(tool_name) {
            return t;
        }
        // 2. Best glob match — longest pattern wins (most specific first).
        // HashMap iteration is unordered; picking the longest matching pattern
        // gives deterministic resolution when multiple globs match.
        let best = cfg
            .tool_timeouts
            .iter()
            .filter(|(pattern, _)| librefang_types::capability::glob_matches(pattern, tool_name))
            .max_by_key(|(pattern, _)| pattern.len());
        if let Some((_, &timeout)) = best {
            return timeout;
        }
        // 3. Global fallback
        cfg.tool_timeout_secs
    }

    fn skill_env_passthrough_policy(
        &self,
    ) -> Option<librefang_types::config::EnvPassthroughPolicy> {
        let cfg = self.config.load();
        librefang_types::config::EnvPassthroughPolicy::from_skills_config(&cfg.skills)
    }

    fn channel_file_download_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.config.load().channels.effective_file_download_dir())
    }

    fn effective_upload_dir(&self) -> std::path::PathBuf {
        self.config_ref().channels.effective_file_download_dir()
    }

    fn readonly_workspace_prefixes(&self, agent_id: &str) -> Vec<std::path::PathBuf> {
        self.named_workspace_prefixes(agent_id)
            .into_iter()
            .filter(|(_, mode)| *mode == WorkspaceMode::ReadOnly)
            .map(|(p, _)| p)
            .collect()
    }

    fn named_workspace_prefixes(&self, agent_id: &str) -> Vec<(std::path::PathBuf, WorkspaceMode)> {
        let Ok(aid) = agent_id.parse::<AgentId>() else {
            return vec![];
        };
        let Some(entry) = self.registry.get(aid) else {
            return vec![];
        };
        if entry.manifest.workspaces.is_empty() {
            return vec![];
        }
        let cfg = self.config.load();
        let workspaces_root = cfg.effective_workspaces_dir();
        let canonical_mount_roots =
            workspace_setup::canonicalize_allowed_mount_roots(&cfg.allowed_mount_roots);
        entry
            .manifest
            .workspaces
            .iter()
            .filter_map(|(name, decl)| {
                workspace_setup::resolve_workspace_decl(
                    name,
                    decl,
                    &workspaces_root,
                    &canonical_mount_roots,
                )
            })
            .collect()
    }
}
