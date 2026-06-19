// ============================================================================
// 14. ToolPolicy — tool/agent config queries (timeouts, env passthrough,
//     workspace prefixes). Pure read-side surface used by the runtime to
//     parameterize tool execution against operator config.
// ============================================================================

pub trait ToolPolicy: Send + Sync {
    /// Tool execution timeout in seconds (from config). Default: 120.
    fn tool_timeout_secs(&self) -> u64 {
        120
    }

    /// Per-tool timeout override lookup.
    ///
    /// Resolution order:
    /// 1. Exact match in `config.tool_timeouts`
    /// 2. Longest glob match in `config.tool_timeouts` (most specific wins)
    /// 3. Global `config.tool_timeout_secs`
    ///
    /// The default impl delegates to `tool_timeout_secs()` (no per-tool config).
    fn tool_timeout_secs_for(&self, _tool_name: &str) -> u64 {
        self.tool_timeout_secs()
    }

    /// Operator-side gate over skill `env_passthrough` requests, derived from
    /// `[skills]` config. `None` = no operator gate (only the built-in
    /// FORBIDDEN/kernel-reserved hard blocks apply). Default impl returns
    /// `None`; the kernel overrides this to pull from `KernelConfig.skills`.
    fn skill_env_passthrough_policy(
        &self,
    ) -> Option<librefang_types::config::EnvPassthroughPolicy> {
        None
    }

    /// Return the canonicalized absolute paths of named workspaces declared as `read-only`
    /// for the given agent. Used by file-write tools to enforce workspace access modes.
    /// Default: no read-only prefixes (all writes allowed by the sandbox).
    fn readonly_workspace_prefixes(&self, _agent_id: &str) -> Vec<std::path::PathBuf> {
        vec![]
    }

    /// Return the canonicalized absolute paths of ALL named workspaces declared
    /// for the given agent, paired with their access modes. Used by file-read,
    /// file-list, file-write, and apply-patch tools to widen the sandbox
    /// accept-list to include declared shared workspaces (PR #2958 wired
    /// `[workspaces]` into write-side denial only; this surfaces the full
    /// allowlist to the read-side path resolver).
    ///
    /// Default: no named workspaces — read-side resolution falls back to the
    /// primary workspace root only.
    fn named_workspace_prefixes(
        &self,
        _agent_id: &str,
    ) -> Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)> {
        Vec::new()
    }

    /// Return the effective directory channel bridges write downloaded
    /// attachments to, when configured. The runtime widens the `file_read` /
    /// `file_list` sandbox accept-list with this prefix so agents can open
    /// the files the bridge hands them via paths like
    /// `/tmp/librefang_uploads/<uuid>.<ext>` (issue #4434).
    ///
    /// Returns `None` for stub kernels without channels wired; the runtime
    /// then falls back to workspace-only resolution.
    fn channel_file_download_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Whether the runtime should collapse repeated `file_read` calls on the
    /// same path within a session into a short stub (#4971). Backed by
    /// `[context_engine] deduplicate_file_reads` — default `true`. Stub
    /// implementations leave the legacy "always full content" behaviour by
    /// returning `false` so they don't have to think about session-scoped
    /// state.
    fn deduplicate_file_reads(&self) -> bool {
        false
    }

    /// Return the effective directory for storing runtime-generated uploads
    /// (image_generate, browser_screenshot, etc.). Honors operator-configured
    /// `[channels].file_download_dir` when set, otherwise falls back to the
    /// legacy `<temp>/librefang_uploads`. See issue #4435.
    fn effective_upload_dir(&self) -> std::path::PathBuf {
        std::env::temp_dir().join("librefang_uploads")
    }

    /// Absolute paths the WASM sandbox must never write to regardless of capability grants; default empty.
    fn protected_write_paths(&self) -> Vec<std::path::PathBuf> {
        vec![]
    }
}
