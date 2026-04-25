//! Workspace filesystem sandboxing.
//!
//! Confines agent file operations to their workspace directory.
//! Prevents path traversal, symlink escapes, and access outside the sandbox.

use std::path::{Path, PathBuf};

/// Error prefix emitted when a `..` component is found in a user-supplied path.
/// Used by `agent_loop` to identify sandbox rejections as soft (recoverable) failures.
pub const ERR_PATH_TRAVERSAL: &str = "Path traversal denied";

/// Error prefix emitted when a path canonicalizes to outside the workspace root.
/// Used by `agent_loop` to identify sandbox rejections as soft (recoverable) failures.
pub const ERR_SANDBOX_ESCAPE: &str = "resolves outside workspace";

/// Resolve a user-supplied path within a workspace sandbox.
///
/// - Rejects `..` components outright.
/// - Relative paths are joined with `workspace_root`.
/// - Absolute paths are checked against the workspace root after canonicalization.
/// - For new files: canonicalizes the parent directory and appends the filename.
/// - The final canonical path must start with the canonical workspace root.
pub fn resolve_sandbox_path(user_path: &str, workspace_root: &Path) -> Result<PathBuf, String> {
    resolve_sandbox_path_ext(user_path, workspace_root, &[])
}

/// Resolve a user-supplied path within a workspace sandbox, allowing additional
/// canonical roots (e.g. named workspaces declared in the agent manifest).
///
/// Behavior:
/// - Rejects `..` components outright.
/// - Relative paths join with `workspace_root` (the primary workspace remains
///   the implicit base — named workspaces are addressed by their absolute path).
/// - Absolute paths are accepted if they canonicalize underneath the primary
///   workspace root OR any of the supplied `additional_roots`.
/// - `additional_roots` are expected to be ALREADY canonical. Callers that
///   maintain a list of named-workspace prefixes should canonicalize once at
///   construction time rather than per-call.
/// - Symlink-escape protection is preserved: a symlink whose target leaves
///   every allowed root is still rejected because the canonicalized candidate
///   no longer starts with any allowed prefix.
pub fn resolve_sandbox_path_ext(
    user_path: &str,
    workspace_root: &Path,
    additional_roots: &[&Path],
) -> Result<PathBuf, String> {
    let path = Path::new(user_path);

    // Reject any `..` components
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(format!(
                "{ERR_PATH_TRAVERSAL}: '..' components are forbidden"
            ));
        }
    }

    // Build the candidate path
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };

    // Canonicalize the workspace root
    let canon_root = workspace_root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve workspace root: {e}"))?;

    // Canonicalize the candidate (or its parent for new files)
    let canon_candidate = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path: {e}"))?
    } else {
        // For new files: canonicalize the parent and append the filename
        // If the parent doesn't exist yet, return the joined path and let
        // the caller create the directory structure.
        let parent = candidate
            .parent()
            .ok_or_else(|| "Invalid path: no parent directory".to_string())?;
        let filename = candidate
            .file_name()
            .ok_or_else(|| "Invalid path: no filename".to_string())?;
        if parent.exists() {
            let canon_parent = parent
                .canonicalize()
                .map_err(|e| format!("Failed to resolve parent directory: {e}"))?;
            canon_parent.join(filename)
        } else {
            // Parent doesn't exist yet. Build the path from the *canonical* root
            // it lives under so the starts_with check below passes on platforms
            // where the root itself is a symlink (e.g. macOS /tmp -> /private/tmp).
            //
            // For an absolute candidate whose ancestor is one of the additional
            // roots, rebase onto that canonical root. Otherwise rebase onto the
            // canonical primary workspace root. This is safe because:
            // 1. We already rejected `..` components.
            // 2. The relative suffix is appended to a canonical root and no
            //    symlinks can exist in the (non-existent) subtree.
            let mut rebased: Option<PathBuf> = None;
            if path.is_absolute() {
                for root in additional_roots {
                    if let Ok(rel) = candidate.strip_prefix(root) {
                        rebased = Some(root.join(rel));
                        break;
                    }
                }
            }
            if let Some(p) = rebased {
                p
            } else {
                let relative = candidate
                    .strip_prefix(workspace_root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|_| candidate.clone());
                canon_root.join(relative)
            }
        }
    };

    // Verify the canonical path is inside the primary workspace OR one of the
    // additional allowed roots.
    let inside_primary = canon_candidate.starts_with(&canon_root);
    let inside_additional = additional_roots
        .iter()
        .any(|root| canon_candidate.starts_with(root));
    if !inside_primary && !inside_additional {
        return Err(format!(
            "Access denied: path '{}' {ERR_SANDBOX_ESCAPE}. \
             If you have an MCP filesystem server configured, use the \
             mcp_filesystem_* tools (e.g. mcp_filesystem_read_file, \
             mcp_filesystem_list_directory) to access files outside \
             the workspace.",
            user_path
        ));
    }

    Ok(canon_candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_relative_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("test.txt"), "hello").unwrap();

        let result = resolve_sandbox_path("data/test.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn test_absolute_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.txt"), "ok").unwrap();
        let abs_path = dir.path().join("file.txt");

        let result = resolve_sandbox_path(abs_path.to_str().unwrap(), dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_absolute_path_outside_workspace_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = std::env::temp_dir().join("outside_test.txt");
        std::fs::write(&outside, "nope").unwrap();

        let result = resolve_sandbox_path(outside.to_str().unwrap(), dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));

        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn test_dotdot_component_blocked() {
        let dir = TempDir::new().unwrap();
        let result = resolve_sandbox_path("../../../etc/passwd", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path traversal denied"));
    }

    #[test]
    fn test_nonexistent_file_with_valid_parent() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let result = resolve_sandbox_path("data/new_file.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("new_file.txt"));
    }

    #[test]
    fn test_nonexistent_file_with_nonexistent_parent() {
        let dir = TempDir::new().unwrap();
        // Parent directory doesn't exist yet
        let result = resolve_sandbox_path("nested/deep/file.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("file.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();

        // Create a symlink inside the workspace pointing outside
        let link_path = dir.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link_path).unwrap();

        let result = resolve_sandbox_path("escape/secret.txt", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));
    }

    // ---- additional_roots tests (named-workspace read-side support) ----

    #[test]
    fn test_relative_path_inside_primary_workspace_with_additional() {
        // Relative paths still resolve under the primary workspace root, even
        // when additional roots are supplied — additional roots are absolute-only.
        let primary = TempDir::new().unwrap();
        let extra = TempDir::new().unwrap();
        std::fs::write(primary.path().join("hello.txt"), "hi").unwrap();

        let extra_canon = extra.path().canonicalize().unwrap();
        let result =
            resolve_sandbox_path_ext("hello.txt", primary.path(), &[extra_canon.as_path()]);
        assert!(result.is_ok(), "got: {:?}", result);
        let resolved = result.unwrap();
        assert!(resolved.starts_with(primary.path().canonicalize().unwrap()));
    }

    #[test]
    fn test_absolute_path_inside_additional_root_allowed() {
        let primary = TempDir::new().unwrap();
        let extra = TempDir::new().unwrap();
        std::fs::write(extra.path().join("shared.txt"), "shared").unwrap();
        let extra_canon = extra.path().canonicalize().unwrap();
        let abs = extra_canon.join("shared.txt");

        let result = resolve_sandbox_path_ext(
            abs.to_str().unwrap(),
            primary.path(),
            &[extra_canon.as_path()],
        );
        assert!(result.is_ok(), "got: {:?}", result);
        let resolved = result.unwrap();
        assert!(resolved.starts_with(&extra_canon));
    }

    #[test]
    fn test_absolute_path_outside_all_roots_blocked() {
        let primary = TempDir::new().unwrap();
        let extra = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        std::fs::write(other.path().join("nope.txt"), "no").unwrap();
        let extra_canon = extra.path().canonicalize().unwrap();
        let abs = other.path().join("nope.txt");

        let result = resolve_sandbox_path_ext(
            abs.to_str().unwrap(),
            primary.path(),
            &[extra_canon.as_path()],
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Access denied"), "got: {err}");
    }

    #[test]
    fn test_dotdot_still_blocked_with_additional_roots() {
        let primary = TempDir::new().unwrap();
        let extra = TempDir::new().unwrap();
        let extra_canon = extra.path().canonicalize().unwrap();

        let result = resolve_sandbox_path_ext(
            "../../../etc/passwd",
            primary.path(),
            &[extra_canon.as_path()],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path traversal denied"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_still_blocked_via_additional_root() {
        // A symlink that lives inside an additional root but points to a third
        // directory (outside both primary and additional) must still be denied.
        let primary = TempDir::new().unwrap();
        let extra = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();

        let extra_canon = extra.path().canonicalize().unwrap();
        let link = extra_canon.join("escape");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let abs = extra_canon.join("escape").join("secret.txt");

        let result = resolve_sandbox_path_ext(
            abs.to_str().unwrap(),
            primary.path(),
            &[extra_canon.as_path()],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));
    }
}
