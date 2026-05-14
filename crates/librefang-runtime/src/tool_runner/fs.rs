//! Filesystem and patch tools: `file_read`, `file_write`, `file_list`,
//! `apply_patch`, plus the shared workspace-sandbox / named-workspace /
//! checkpoint-snapshot helpers used by the dispatcher to enforce the
//! agent's filesystem boundary.

use crate::kernel_handle::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

/// Resolve a file path through the workspace sandbox, with optional
/// additional canonical roots that should also be considered "inside the
/// sandbox" — used to honor named workspaces declared in the agent's
/// manifest.
///
/// SECURITY: Returns an error when `workspace_root` is `None` to prevent
/// unrestricted filesystem access. All file operations MUST be confined to
/// the agent's workspace directory or one of the explicitly allow-listed
/// `additional_roots`.
pub(super) fn resolve_file_path_ext(
    raw_path: &str,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<PathBuf, String> {
    let root = workspace_root.ok_or(
        "Workspace sandbox not configured: file operations are disabled. \
         Set a workspace_root in the agent manifest or kernel config to enable file tools.",
    )?;
    crate::workspace_sandbox::resolve_sandbox_path_ext(raw_path, root, additional_roots)
}

/// Fetch the named-workspace prefixes (all modes) for the calling agent.
/// Returns an empty vec when either kernel or agent id is missing.
pub(super) fn named_ws_prefixes(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Vec<PathBuf> {
    match (kernel, caller_agent_id) {
        (Some(k), Some(aid)) => k
            .named_workspace_prefixes(aid)
            .into_iter()
            .map(|(p, _)| p)
            .collect(),
        _ => Vec::new(),
    }
}

/// Like [`named_ws_prefixes`] but only returns prefixes for read-write
/// workspaces. Used by `file_write` to widen the writable allowlist.
pub(super) fn named_ws_prefixes_writable(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Vec<PathBuf> {
    match (kernel, caller_agent_id) {
        (Some(k), Some(aid)) => k
            .named_workspace_prefixes(aid)
            .into_iter()
            .filter(|(_, mode)| *mode == librefang_types::agent::WorkspaceMode::ReadWrite)
            .map(|(p, _)| p)
            .collect(),
        _ => Vec::new(),
    }
}

/// Like [`named_ws_prefixes`] but only returns prefixes for read-only
/// workspaces. Used by `apply_patch` (#3662) to enforce a deny-list at the
/// write call site in addition to the dispatch-level path check.
pub(super) fn named_ws_prefixes_readonly(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Vec<PathBuf> {
    match (kernel, caller_agent_id) {
        (Some(k), Some(aid)) => k.readonly_workspace_prefixes(aid),
        _ => Vec::new(),
    }
}

/// Validate that a file path stays inside the agent's allowed
/// workspace set BEFORE the path is forwarded to an ACP client
/// (#3313 review).
///
/// Returns `Some(error_message)` if the path is rejected (either
/// because it contains `..` traversal or because the absolute path
/// escapes every allowed prefix). Returns `None` for relative paths
/// without `..` and for absolute paths inside the workspace.
///
/// **Why this is the editor's threat surface but ours too:** ACP
/// editors faithfully serve whatever path the agent asks for. Without
/// this guard an LLM could ask the editor to read `/etc/shadow` or
/// `~/.ssh/id_ed25519` and the contents would land in the agent's
/// next prompt as legitimate tool output. The editor has no way to
/// distinguish "agent asked for a file" from "user clicked a file in
/// the IDE." So the LibreFang side has to enforce the same workspace
/// jail it applies to the local-fs path.
///
/// SECURITY (#3313 follow-up): `Path::starts_with` is component-based
/// and does NOT collapse `..`, so the previous revision of this fn
/// accepted `/<workspace_root>/../etc/shadow` because the first
/// `<workspace_root>` components matched as a prefix and the
/// `..`/`etc`/`shadow` components were ignored. Reject any `..`
/// component up front — mirrors the input filter
/// [`crate::workspace_sandbox::resolve_sandbox_path_ext`] applies on
/// the local-fs side. Same rejection regardless of absolute-vs-
/// relative so a relative `../../etc/shadow` (resolved by the editor
/// against its declared cwd) can't slip past either.
pub(super) fn check_absolute_path_inside_workspace(
    raw_path: Option<&str>,
    workspace_root: Option<&Path>,
    allowed_prefixes: &[PathBuf],
) -> Option<String> {
    let raw = raw_path?;
    let p = Path::new(raw);
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Some(format!(
            "path '{raw}' contains '..' components which are forbidden; \
             absolute paths must resolve inside the agent's workspace \
             without traversal"
        ));
    }
    if !p.is_absolute() {
        return None;
    }
    if let Some(root) = workspace_root {
        if p.starts_with(root) {
            return None;
        }
    }
    if allowed_prefixes.iter().any(|prefix| p.starts_with(prefix)) {
        return None;
    }
    Some(format!(
        "path '{raw}' is outside the agent's workspace and named-workspace allowlist; \
         absolute paths must reside inside the agent's declared filesystem boundary"
    ))
}

/// Take a snapshot of `workspace_root` before a file-mutating operation.
///
/// If an explicit `CheckpointManager` is provided (injected from the kernel),
/// it is used.  When `mgr` is `None` no snapshot is taken — callers that
/// pass `None` are test or ephemeral contexts that do not need filesystem
/// rollback coverage.
///
/// Failures are **non-fatal**: they are logged as warnings and the calling
/// tool proceeds normally.
///
/// ## Async safety
///
/// `CheckpointManager::snapshot` spawns `git` subprocesses and calls
/// blocking I/O.  This wrapper offloads the work to a dedicated thread pool
/// via `tokio::task::spawn_blocking` so that tokio worker threads are never
/// blocked by slow git operations.
pub(super) async fn maybe_snapshot(
    mgr: &Option<&Arc<crate::checkpoint_manager::CheckpointManager>>,
    workspace_root: Option<&Path>,
    reason: &str,
) {
    let Some(root) = workspace_root else {
        return;
    };
    let Some(m) = mgr else {
        // No manager injected — skip snapshot entirely.
        // (Test call sites pass None deliberately; production code always
        // passes Some via the kernel.)
        return;
    };

    let mgr_arc = Arc::clone(m);
    let root_owned = root.to_path_buf();
    let reason_owned = reason.to_string();

    // Offload blocking git I/O to the blocking thread pool.
    let result =
        tokio::task::spawn_blocking(move || mgr_arc.snapshot(&root_owned, &reason_owned)).await;

    match result {
        Ok(Err(e)) => {
            warn!(reason, root = %root.display(), "checkpoint snapshot failed (non-fatal): {e}")
        }
        Err(e) => warn!(reason, root = %root.display(), "checkpoint spawn_blocking panicked: {e}"),
        Ok(Ok(_)) => {}
    }
}

pub(super) async fn tool_file_read(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
    tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))
}

/// `file_read` deduplication shim (#4971).
///
/// Returns the content the model should see — either the original content
/// unchanged, a short "already read" stub, or the original content prefixed
/// with a "file updated" header. The transformation is bypassed (i.e. always
/// returns `content` unchanged) when any of the gating conditions don't hold:
///
/// - no kernel handle (legacy / test call sites),
/// - no session id (can't isolate state across concurrent sessions),
/// - `[context_engine] deduplicate_file_reads = false`.
pub(super) fn maybe_dedup_file_read(
    kernel: Option<&Arc<dyn KernelHandle>>,
    session_id: Option<librefang_types::agent::SessionId>,
    path: &Path,
    content: String,
) -> String {
    let Some(k) = kernel else { return content };
    let Some(sid) = session_id else {
        return content;
    };
    if !k.deduplicate_file_reads() {
        return content;
    }
    match crate::file_read_tracker::with_session(sid, |t| t.observe(path, &content)) {
        crate::file_read_tracker::ReadOutcome::First => content,
        crate::file_read_tracker::ReadOutcome::Unchanged { first_turn } => {
            crate::file_read_tracker::unchanged_stub(first_turn)
        }
        crate::file_read_tracker::ReadOutcome::Changed { previous_turn } => {
            format!(
                "{}\n\n{}",
                crate::file_read_tracker::changed_header(previous_turn),
                content
            )
        }
    }
}

pub(super) async fn tool_file_write(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
    let content = input["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {e}"))?;
    }
    tokio::fs::write(&resolved, content)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        resolved.display()
    ))
}

pub(super) async fn tool_file_list(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or(
        "Missing 'path' parameter — retry with {\"path\": \".\"} to list the workspace root",
    )?;
    let resolved = resolve_file_path_ext(raw_path, workspace_root, additional_roots)?;
    let mut entries = tokio::fs::read_dir(&resolved)
        .await
        .map_err(|e| format!("Failed to list directory: {e}"))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let suffix = match metadata {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        files.push(format!("{name}{suffix}"));
    }
    files.sort();
    Ok(files.join("\n"))
}

pub(super) async fn tool_apply_patch(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
    readonly_roots: &[&Path],
) -> Result<String, String> {
    let patch_str = input["patch"].as_str().ok_or("Missing 'patch' parameter")?;
    let root = workspace_root.ok_or("apply_patch requires a workspace root")?;
    let ops = crate::apply_patch::parse_patch(patch_str)?;
    // SECURITY #3662: defense-in-depth — pass readonly named-workspace prefixes
    // through to `apply_patch_ext` so any resolved target path that lands
    // inside a read-only workspace is rejected at the write site as well as
    // at dispatch.
    let result =
        crate::apply_patch::apply_patch_ext(&ops, root, additional_roots, readonly_roots).await;
    if result.is_ok() {
        Ok(result.summary())
    } else {
        Err(format!(
            "Patch partially applied: {}. Errors: {}",
            result.summary(),
            result.errors.join("; ")
        ))
    }
}

#[cfg(test)]
mod path_check_tests {
    use super::*;

    #[test]
    fn absolute_inside_workspace_passes() {
        let root = PathBuf::from("/ws");
        assert!(
            check_absolute_path_inside_workspace(Some("/ws/file.txt"), Some(&root), &[]).is_none()
        );
    }

    #[test]
    fn relative_path_passes() {
        let root = PathBuf::from("/ws");
        assert!(
            check_absolute_path_inside_workspace(Some("subdir/file.txt"), Some(&root), &[])
                .is_none()
        );
    }

    #[test]
    fn missing_path_passes() {
        let root = PathBuf::from("/ws");
        assert!(check_absolute_path_inside_workspace(None, Some(&root), &[]).is_none());
    }

    #[test]
    fn absolute_outside_workspace_blocked() {
        // Windows treats `/etc/passwd` as relative (no drive letter), so
        // pick a path that `Path::is_absolute()` agrees with on the host.
        let (root, outside) = if cfg!(windows) {
            (PathBuf::from(r"C:\ws"), r"D:\etc\passwd")
        } else {
            (PathBuf::from("/ws"), "/etc/passwd")
        };
        let err = check_absolute_path_inside_workspace(Some(outside), Some(&root), &[])
            .expect("path outside workspace must be blocked");
        assert!(err.contains("outside the agent's workspace"));
    }

    #[test]
    fn absolute_inside_named_workspace_passes() {
        let root = PathBuf::from("/ws");
        let extra = PathBuf::from("/shared");
        assert!(check_absolute_path_inside_workspace(
            Some("/shared/data.txt"),
            Some(&root),
            std::slice::from_ref(&extra),
        )
        .is_none());
    }

    /// SECURITY regression test: `Path::starts_with` is component-based
    /// and does not collapse `..`, so an absolute path of the form
    /// `<root>/../<elsewhere>` previously bypassed the workspace jail
    /// and reached the editor's `fs/read_text_file` because the leading
    /// `<root>` components matched as a prefix and the `..` was ignored
    /// during the comparison. The fix rejects any `..` component up
    /// front.
    #[test]
    fn absolute_with_dotdot_traversal_blocked_even_under_workspace_root() {
        let root = PathBuf::from("/ws");
        let err = check_absolute_path_inside_workspace(Some("/ws/../etc/shadow"), Some(&root), &[])
            .expect("`..` traversal must be blocked");
        assert!(
            err.contains("'..'"),
            "error must call out the forbidden component, got: {err}"
        );
    }

    #[test]
    fn absolute_with_dotdot_traversal_blocked_under_named_workspace() {
        let root = PathBuf::from("/ws");
        let extra = PathBuf::from("/shared");
        let err = check_absolute_path_inside_workspace(
            Some("/shared/../etc/shadow"),
            Some(&root),
            std::slice::from_ref(&extra),
        )
        .expect("`..` traversal under named workspace must also be blocked");
        assert!(err.contains("'..'"));
    }

    #[test]
    fn relative_with_dotdot_traversal_blocked() {
        // The editor would resolve relative paths against its declared
        // cwd, but a relative `..` chain still trivially escapes the
        // editor's own project root. Mirror the local-fs sandbox's
        // refusal of `..` so neither wire path leaks the difference.
        let root = PathBuf::from("/ws");
        let err = check_absolute_path_inside_workspace(Some("../../etc/shadow"), Some(&root), &[])
            .expect("relative `..` must be blocked");
        assert!(err.contains("'..'"));
    }
}
