//! Filesystem and patch tools: `file_read`, `file_write`, `file_list`,
//! `apply_patch`, plus the shared workspace-sandbox / named-workspace /
//! checkpoint-snapshot helpers used by the dispatcher to enforce the
//! agent's filesystem boundary.

//! #3576: the four tool fns (`tool_file_read/write/list/apply_patch`) return
//! `Result<String, ToolError>`. Missing params -> `MissingParameter`; the
//! shared `resolve_file_path_ext` / `parse_patch` (both still `Result<_,
//! String>`) -> `InvalidParameter` with the message preserved; the `io::Error`
//! sites -> `ToolError::Upstream` keeping the prefix and source. The path /
//! checkpoint helpers below keep their `Result<_, String>` / `Option<String>`
//! shapes (shared with the dispatcher and unmigrated tools).

use super::error::{ToolError, ToolResult};
use super::io_retry;
use crate::kernel_handle::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

const DEFAULT_MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
const HARD_MAX_READ_BYTES: u64 = 128 * 1024 * 1024;
const DEFAULT_DIR_LIMIT: usize = 1000;
const DEFAULT_DIR_OFFSET: usize = 0;

fn reject_backslash_on_unix(raw_path: &str) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        if raw_path.contains('\\') {
            return Err(format!(
                "path '{}' contains backslash separators which are forbidden on this platform",
                raw_path
            ));
        }
    }
    let _ = raw_path;
    Ok(())
}

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
    reject_backslash_on_unix(raw_path)?;
    let root = workspace_root.ok_or(
        "Workspace sandbox not configured: file operations are disabled. \
         Set a workspace_root in the agent manifest or kernel config to enable file tools.",
    )?;
    crate::workspace_sandbox::resolve_sandbox_path_ext(raw_path, root, additional_roots)
}

/// #3576 thin wrapper over [`resolve_file_path_ext`] that maps its
/// stringly-typed rejection (sandbox-escape / not-configured) onto a typed
/// `ToolError::InvalidParameter` while preserving the message. The underlying
/// resolver keeps its `Result<_, String>` shape because it is shared with
/// tools that haven't migrated yet.
fn resolve_path(
    raw_path: &str,
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
) -> Result<PathBuf, ToolError> {
    resolve_file_path_ext(raw_path, workspace_root, additional_roots).map_err(|reason| {
        ToolError::InvalidParameter {
            name: "path",
            reason,
        }
    })
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

/// Fetch the named-workspace `@alias` → resolved-path mapping for the calling
/// agent. Returns an empty vec when either kernel or agent id is missing.
pub(super) fn named_ws_aliases(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Vec<(String, PathBuf)> {
    match (kernel, caller_agent_id) {
        (Some(k), Some(aid)) => k
            .named_workspace_aliases(aid)
            .into_iter()
            .map(|(name, path, _)| (name, path))
            .collect(),
        _ => Vec::new(),
    }
}

/// Tool input keys that carry a workspace-sandbox path, and so accept the `@name` form the kernel advertises in `TOOLS.md` (#8051).
///
/// A tool may appear more than once: `media_transcribe` takes both the input file (`path`) and the transcript destination (`out_path`), and a tool with two paths must not have one of them silently left unexpanded.
/// Keys that name a path *inside a skill directory* (`skill_read_file`, `skill_evolve_*`) are deliberately absent — they are not workspace paths, so `@name` has nothing to resolve against.
///
/// `apply_patch` is deliberately absent: its paths live inside the patch text under `*** Update File:` headers rather than in a path argument, so there is no key to rewrite.
/// It gets the same treatment at the same dispatch barrier, one step later: the parsed [`crate::apply_patch::PatchOp`]s are expanded by [`expand_aliases_in_patch_ops`] instead of the raw text, so a `+*** Add File:` line inside a hunk can never be mistaken for a header.
const ALIAS_PATH_KEYS: &[(&str, &str)] = &[
    ("file_read", "path"),
    ("file_write", "path"),
    ("file_list", "path"),
    ("code_search", "path"),
    ("image_analyze", "path"),
    ("media_describe", "path"),
    ("media_transcribe", "path"),
    ("media_transcribe", "out_path"),
    ("speech_to_text", "path"),
    ("channel_send", "file_path"),
    ("web_fetch_to_file", "dest_path"),
];

/// Expand a leading `@name/` in the path argument of `tool_name`, if it has one.
///
/// `Ok(None)` means "nothing to rewrite" and the caller keeps the original
/// input; `Ok(Some(v))` is the rewritten input; `Err` is a message for the
/// model.
///
/// This runs at the dispatch boundary, *before* any arm inspects a path, so
/// every downstream check — the sandbox resolver, the absolute-path
/// containment gate, the read-only workspace pre-checks — sees an ordinary
/// absolute path and enforces against exactly the string that will be opened.
/// Expanding later, inside the individual tools, would let the two disagree.
pub(super) fn expand_alias_in_tool_input(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<Option<serde_json::Value>, String> {
    let keys: Vec<&str> = ALIAS_PATH_KEYS
        .iter()
        .filter(|(t, _)| *t == tool_name)
        .map(|(_, key)| *key)
        .collect();
    if keys.is_empty() {
        return Ok(None);
    }
    // Resolving aliases canonicalizes every declared mount, so do not pay for it when no argument is an alias in the first place.
    let has_alias_path = keys.iter().any(|key| {
        input
            .get(*key)
            .and_then(|v| v.as_str())
            .is_some_and(|raw| raw.starts_with('@'))
    });
    if !has_alias_path {
        return Ok(None);
    }
    let aliases = named_ws_aliases(kernel, caller_agent_id);
    let mut rewritten: Option<serde_json::Value> = None;
    for key in keys {
        let Some(raw) = input.get(key).and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(expanded) = expand_alias_path(raw, &aliases)? else {
            continue;
        };
        let target = rewritten.get_or_insert_with(|| input.clone());
        target[key] = serde_json::Value::String(expanded);
    }
    Ok(rewritten)
}

/// Resolve `@name` / `@name/sub/path` against `aliases`.
///
/// The remainder is rejected if it is absolute or contains `..`. Both would be
/// caught downstream (`Path::join` lets an absolute right-hand side replace the
/// root, and the sandbox resolver refuses `..` and re-checks containment after
/// canonicalizing), but an alias is a convenience the agent was *told* to use,
/// so it should fail with a message about the alias rather than a traversal
/// error about a path the agent never typed.
pub(super) fn expand_workspace_alias(
    raw: &str,
    aliases: &[(String, PathBuf)],
) -> Result<String, String> {
    let body = raw.strip_prefix('@').unwrap_or(raw);
    let (name, rest) = body.split_once('/').unwrap_or((body, ""));

    let Some((_, root)) = aliases.iter().find(|(n, _)| n == name) else {
        let mut known: Vec<String> = aliases.iter().map(|(n, _)| format!("@{n}")).collect();
        known.sort();
        return Err(format!(
            "unknown named workspace '@{name}'. Declared for this agent: {}. \
             If you meant a file whose name starts with '@', pass its absolute path instead",
            known.join(", ")
        ));
    };

    if rest.is_empty() {
        return Ok(root.display().to_string());
    }
    let rest_path = Path::new(rest);
    if rest_path.is_absolute() {
        return Err(format!(
            "path '{raw}' is not valid: the part after '@{name}/' must be relative"
        ));
    }
    if rest_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "path '{raw}' is not valid: '..' components are forbidden inside a named workspace"
        ));
    }
    Ok(root.join(rest_path).display().to_string())
}

/// Expand a leading `@name/` in a *tool argument* or a *patch header* path.
///
/// This is the one place the `@name` gate is written: [`expand_alias_in_tool_input`] applies it to `ALIAS_PATH_KEYS` fields, and [`expand_aliases_in_patch_ops`] applies it to the paths inside a patch.
/// Both callers therefore agree on the three cases — not starting with `@`, no alias declared, and unknown alias — instead of each spelling them out again.
///
/// `Ok(None)` means there is nothing to rewrite (the path does not start with `@`, or the agent declares no named workspace, in which case `@` is a filename character); `Err` is a message for the model.
fn expand_alias_path(raw: &str, aliases: &[(String, PathBuf)]) -> Result<Option<String>, String> {
    if !raw.starts_with('@') || aliases.is_empty() {
        return Ok(None);
    }
    expand_workspace_alias(raw, aliases).map(Some)
}

/// Rewrite the `@name/...` paths carried by a parsed patch into the mounts they name.
///
/// `apply_patch` keeps its paths in the patch body (`*** Add File:`, `*** Update File:`, `*** Delete File:`, and the `old -> new` form of `*** Update File:`) rather than in a path argument, so [`ALIAS_PATH_KEYS`] cannot reach it.
/// Expanding the *parsed* operations instead of the raw text is what keeps file content untouched: a `+*** Add File: x` line inside a hunk is a `new_line`, not an operation, so it is never inspected here.
///
/// A patch target that names a directory is rejected: the spellings `@name`, `@name/`, `@name/.` and `@name/sub/` all do, and so does `@name/notes` when `notes` already exists as a directory.
/// Handing any of them to the patch would surface as `File exists` / `Is a directory` from the filesystem and, for `*** Add File:`, leave a staging file before the write failed.
/// The spelling is checked first because it also catches a directory that does not exist yet, and because it keeps the diagnostic about what the model wrote rather than about where it resolved to.
fn expand_aliases_in_patch_ops(
    ops: Vec<crate::apply_patch::PatchOp>,
    aliases: &[(String, PathBuf)],
) -> Result<Vec<crate::apply_patch::PatchOp>, String> {
    use crate::apply_patch::PatchOp;

    if aliases.is_empty() {
        // Nothing is declared, so a leading `@` is just the first character of a filename, and rewriting it would break a legitimate path.
        return Ok(ops);
    }

    // A patch target must be a file.
    // Every directory-shaped spelling is caught here, including `@name/.`, which resolves to the mount root like `@name` does but does not end with a slash — a `ends_with('/')` test alone lets it through to the filesystem.
    // A directory that already exists is caught as well: `@name/notes` has none of those spellings, and without the second test it reached `atomic_create` and surfaced `File exists`.
    let expand_file = |raw: String| -> Result<String, String> {
        let Some(expanded) = expand_alias_path(&raw, aliases)? else {
            return Ok(raw);
        };
        let body = raw.strip_prefix('@').unwrap_or(&raw);
        let name = body.split('/').next().unwrap_or_default();
        // `..` never reaches this point: `expand_alias_path` above rejects a ParentDir remainder with its own diagnostic.
        let names_directory = !body.contains('/') || raw.ends_with('/') || raw.ends_with("/.");
        if names_directory || Path::new(&expanded).is_dir() {
            return Err(format!(
                "path '{raw}' names a directory, not a file; a patch target must be a file path under the '@{name}' named workspace (e.g. '@{name}/notes.md')"
            ));
        }
        Ok(expanded)
    };

    ops.into_iter()
        .map(|op| match op {
            PatchOp::AddFile { path, content } => Ok(PatchOp::AddFile {
                path: expand_file(path)?,
                content,
            }),
            PatchOp::DeleteFile { path } => Ok(PatchOp::DeleteFile {
                path: expand_file(path)?,
            }),
            PatchOp::UpdateFile {
                path,
                move_to,
                hunks,
            } => {
                // Only an alias target is held to this rule: a relative destination next
                // to an absolute source is the long-standing behaviour for an agent's own
                // workspace, and rejecting it would break renames that never touched a
                // named workspace.
                let source_from_alias = path.starts_with('@');
                let path = expand_file(path)?;
                let move_to = match move_to {
                    Some(dest) => {
                        let dest = expand_file(dest)?;
                        // A relative destination resolves against the agent workspace, not against the directory the source lives in (`resolve_patch_path`).
                        // For an `@name` source that turns a rename into a move *out* of the share, and the source is removed afterwards, so the only trace is a file in a directory the user never looked at.
                        if source_from_alias && !Path::new(&dest).is_absolute() {
                            return Err(format!(
                                "path '{dest}' is relative, so it would resolve against the agent workspace rather than the named workspace '{path}' came from; give an absolute or '@name/...' destination"
                            ));
                        }
                        Some(dest)
                    }
                    None => None,
                };
                Ok(PatchOp::UpdateFile {
                    path,
                    move_to,
                    hunks,
                })
            }
        })
        .collect()
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
    if let Err(e) = reject_backslash_on_unix(raw) {
        return Some(e);
    }
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
) -> ToolResult {
    let raw_path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;
    let resolved = resolve_path(raw_path, workspace_root, additional_roots)?;
    let max_bytes = input["max_bytes"]
        .as_u64()
        .unwrap_or(DEFAULT_MAX_READ_BYTES)
        .min(HARD_MAX_READ_BYTES);
    let file = tokio::fs::File::open(&resolved)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to open file '{}': {e}", resolved.display()),
            source: Some(Box::new(e)),
        })?;
    let metadata = file.metadata().await.map_err(|e| ToolError::Upstream {
        message: format!("Failed to read metadata for '{}': {e}", resolved.display()),
        source: Some(Box::new(e)),
    })?;
    let file_size = metadata.len();
    if file_size > max_bytes {
        return Err(ToolError::InvalidParameter {
            name: "max_bytes",
            reason: format!(
                "File '{}' is {} bytes, exceeding the read limit of {} bytes",
                resolved.display(),
                file_size,
                max_bytes
            ),
        });
    }
    use tokio::io::AsyncReadExt;
    let alloc = (file_size.min(max_bytes)) as usize;
    let mut buf = Vec::with_capacity(alloc);
    // The stat above is only a fast path: a file another process is appending to, or a FIFO that reports length 0, can deliver far more than it claimed.
    // Bounding the read itself at one byte past the limit is what actually enforces `max_bytes`.
    let mut reader = tokio::io::BufReader::new(file).take(max_bytes.saturating_add(1));
    reader
        .read_to_end(&mut buf)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to read file '{}': {e}", resolved.display()),
            source: Some(Box::new(e)),
        })?;
    if buf.len() as u64 > max_bytes {
        return Err(ToolError::InvalidParameter {
            name: "max_bytes",
            reason: format!(
                "File '{}' grew past the read limit of {} bytes while being read",
                resolved.display(),
                max_bytes
            ),
        });
    }
    String::from_utf8(buf).map_err(|e| ToolError::InvalidParameter {
        name: "path",
        reason: format!(
            "File '{}' contains non-UTF-8 content: {e}",
            resolved.display()
        ),
    })
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
) -> ToolResult {
    let raw_path = input["path"]
        .as_str()
        .ok_or(ToolError::MissingParameter("path"))?;
    let resolved = resolve_path(raw_path, workspace_root, additional_roots)?;
    if let Some(root) = workspace_root {
        crate::workspace_sandbox::deny_own_manifest_write(&resolved, root).map_err(|reason| {
            ToolError::InvalidParameter {
                name: "path",
                reason,
            }
        })?;
    }
    let content = input["content"]
        .as_str()
        .ok_or(ToolError::MissingParameter("content"))?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ToolError::Upstream {
                message: format!(
                    "Failed to create directories for '{}': {e}",
                    resolved.display()
                ),
                source: Some(Box::new(e)),
            })?;
    }
    let tmp = resolved.with_extension(format!("{}.tmp", rand::random::<u64>()));
    // The rename replaces the target's inode, so the temp file must carry the existing file's mode or an edited `run.sh` silently loses its executable bit.
    // Unix only: on Windows `Permissions` is just the readonly flag, and a readonly target already makes the rename fail.
    #[cfg(unix)]
    let existing_perms = tokio::fs::metadata(&resolved)
        .await
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.permissions());
    // Every failure below removes the temp file, so a failed write (disk full) or rename (target is a directory) does not leave `<name>.<n>.tmp` debris behind on each retry.
    if let Err(e) = tokio::fs::write(&tmp, content).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(ToolError::Upstream {
            message: format!("Failed to write file '{}': {e}", resolved.display()),
            source: Some(Box::new(e)),
        });
    }
    #[cfg(unix)]
    if let Some(perms) = existing_perms {
        if let Err(e) = tokio::fs::set_permissions(&tmp, perms).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(ToolError::Upstream {
                message: format!(
                    "Failed to preserve permissions of '{}': {e}",
                    resolved.display()
                ),
                source: Some(Box::new(e)),
            });
        }
    }
    if let Err(e) = tokio::fs::rename(&tmp, &resolved).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(ToolError::Upstream {
            message: format!(
                "Failed to atomically rename '{}' -> '{}': {e}",
                tmp.display(),
                resolved.display()
            ),
            source: Some(Box::new(e)),
        });
    }
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
) -> ToolResult {
    // Keep the self-correction hint the pre-#3576 message carried: an LLM that
    // calls file_list with no path recovers in one turn by re-calling with
    // {"path": "."}. MissingParameter can't carry free text, so use
    // InvalidParameter to preserve the guidance.
    let raw_path = input["path"].as_str().ok_or(ToolError::InvalidParameter {
        name: "path",
        reason: "retry with {\"path\": \".\"} to list the workspace root".to_string(),
    })?;
    let resolved = resolve_path(raw_path, workspace_root, additional_roots)?;
    let limit = input["limit"].as_u64().map(|l| l as usize);
    let offset = input["offset"]
        .as_u64()
        .map(|o| o as usize)
        .unwrap_or(DEFAULT_DIR_OFFSET);
    // #8050: both directory calls go through the EINTR-retrying wrappers.
    // On a cloud-synced mount an interrupted syscall is routine, and surfacing it failed the tool call with a message that reads like a filesystem outage.
    let mut entries = io_retry::read_dir(&resolved)
        .await
        .map_err(|e| ToolError::Upstream {
            message: format!("Failed to list directory '{}': {e}", resolved.display()),
            source: Some(Box::new(e)),
        })?;
    let mut files = Vec::new();
    while let Some(entry) =
        io_retry::next_entry(&mut entries)
            .await
            .map_err(|e| ToolError::Upstream {
                message: format!("Failed to read entry in '{}': {e}", resolved.display()),
                source: Some(Box::new(e)),
            })?
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
    let total = files.len();
    let skipped = offset.min(files.len());
    let limited: Vec<String> = files
        .into_iter()
        .skip(skipped)
        .take(limit.unwrap_or(DEFAULT_DIR_LIMIT))
        .collect();
    let mut out = limited.join("\n");
    // The trailer is needed most when the caller passed no `limit`: the default cap truncates silently otherwise, and the model concludes the directory holds only what it was shown.
    if skipped + limited.len() < total {
        out.push_str(&format!(
            "\n--- showing {} of {} entries (offset: {}); pass \"offset\": {} to list more ---",
            limited.len(),
            total,
            skipped,
            skipped + limited.len()
        ));
    }
    Ok(out)
}

/// Parse the patch and expand the `@name` paths it carries.
///
/// Split out of the apply path so the dispatch arm can run its read-only pre-check against the *expanded* paths (#8744 review): the pre-check used to look at the raw header text, so an `@alias` target slipped past it and the per-operation check inside `apply_patch_ext` turned a whole-patch denial into a partial apply.
///
/// A patch that cannot be parsed or expanded is rejected here, before the caller snapshots the workspace or touches the filesystem.
pub(super) fn parse_and_expand_patch(
    input: &serde_json::Value,
    aliases: &[(String, PathBuf)],
) -> Result<Vec<crate::apply_patch::PatchOp>, ToolError> {
    let patch_str = input["patch"]
        .as_str()
        .ok_or(ToolError::MissingParameter("patch"))?;
    let ops = crate::apply_patch::parse_patch(patch_str).map_err(|reason| {
        ToolError::InvalidParameter {
            name: "patch",
            reason,
        }
    })?;
    expand_aliases_in_patch_ops(ops, aliases).map_err(|reason| ToolError::InvalidParameter {
        name: "patch",
        reason,
    })
}

/// Apply already-parsed, already-expanded patch operations.
///
/// SECURITY #3662: `readonly_roots` goes through to `apply_patch_ext` so any resolved target path that lands inside a read-only workspace is rejected at the write site as well as at dispatch.
pub(super) async fn apply_patch_ops(
    ops: &[crate::apply_patch::PatchOp],
    workspace_root: Option<&Path>,
    additional_roots: &[&Path],
    readonly_roots: &[&Path],
) -> ToolResult {
    let root = workspace_root.ok_or(ToolError::Unavailable("workspace directory"))?;
    let result =
        crate::apply_patch::apply_patch_ext(ops, root, additional_roots, readonly_roots).await;
    if result.is_ok() {
        Ok(result.summary())
    } else {
        // A partial apply is a downstream outcome, not bad input — keep the
        // summary + per-hunk errors verbatim.
        Err(ToolError::upstream_msg(format!(
            "Patch partially applied: {}. Errors: {}",
            result.summary(),
            result.errors.join("; ")
        )))
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

    #[cfg(not(windows))]
    #[test]
    fn backslash_rejected_on_unix() {
        let root = PathBuf::from("/ws");
        let err = check_absolute_path_inside_workspace(Some("/ws/sub\\dir"), Some(&root), &[])
            .expect("backslash must be blocked on Unix");
        assert!(
            err.contains("backslash"),
            "error must mention backslash, got: {err}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_rejects_backslash_on_unix() {
        let root = PathBuf::from("/ws");
        let err = resolve_file_path_ext("sub\\dir", Some(&root), &[]).unwrap_err();
        assert!(err.contains("backslash"));
    }
}

#[cfg(test)]
mod toolerror_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn file_read_missing_path_is_missing_parameter() {
        let r = tool_file_read(&json!({}), None, &[]).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("path"))));
    }

    #[tokio::test]
    async fn file_write_missing_path_is_missing_parameter() {
        let r = tool_file_write(&json!({}), None, &[]).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("path"))));
    }

    #[tokio::test]
    async fn file_list_missing_path_is_invalid_parameter_with_hint() {
        // file_list maps a missing path to InvalidParameter so it can carry the
        // {"path": "."} self-correction hint (MissingParameter is name-only).
        let r = tool_file_list(&json!({}), None, &[]).await;
        match r {
            Err(ToolError::InvalidParameter { name, reason }) => {
                assert_eq!(name, "path");
                assert!(reason.contains("\"path\": \".\""), "got: {reason}");
            }
            other => panic!("expected InvalidParameter, got {other:?}"),
        }
    }

    /// Every workspace path a tool declares must be expandable, or `@name` silently
    /// resolves against the agent workspace instead — the bug this table exists to
    /// prevent, one renamed key away.
    ///
    /// Keys that name a path *inside a skill directory* are excluded: they are not
    /// workspace paths, so there is no mount for `@name` to resolve against.
    #[test]
    fn every_workspace_path_argument_is_in_the_alias_table() {
        const NOT_WORKSPACE_PATHS: &[(&str, &str)] = &[
            ("skill_read_file", "path"),
            ("skill_evolve_write_file", "path"),
            ("skill_evolve_remove_file", "path"),
        ];

        let mut missing = Vec::new();
        for tool in crate::tool_runner::builtin_tool_definitions() {
            let Some(properties) = tool.input_schema.get("properties") else {
                continue;
            };
            let Some(properties) = properties.as_object() else {
                continue;
            };
            for key in properties.keys() {
                if !key.contains("path") {
                    continue;
                }
                let excluded = NOT_WORKSPACE_PATHS
                    .iter()
                    .any(|(tool_name, key_name)| *tool_name == tool.name && key_name == key);
                let listed = ALIAS_PATH_KEYS
                    .iter()
                    .any(|(tool_name, key_name)| *tool_name == tool.name && key_name == key);
                if !listed && !excluded {
                    missing.push(format!("{}.{key}", tool.name));
                }
            }
        }
        missing.sort();

        assert!(
            missing.is_empty(),
            "rows missing from ALIAS_PATH_KEYS: {missing:?} — an `@name` in those arguments would resolve against the agent workspace"
        );

        // The reverse direction: a row that matches no tool, or a tool that has no
        // such property, covers nothing while looking like coverage — and a typo in
        // either half is how the real key ends up unexpanded.
        let definitions = crate::tool_runner::builtin_tool_definitions();
        let mut orphaned = Vec::new();
        for (tool_name, key) in ALIAS_PATH_KEYS {
            let Some(tool) = definitions.iter().find(|tool| tool.name == *tool_name) else {
                orphaned.push(format!("{tool_name}.{key} (no such tool)"));
                continue;
            };
            let has_property = tool
                .input_schema
                .get("properties")
                .and_then(|properties| properties.as_object())
                .is_some_and(|properties| properties.contains_key(*key));
            if !has_property {
                orphaned.push(format!("{tool_name}.{key} (no such property)"));
            }
        }
        assert!(
            orphaned.is_empty(),
            "rows in ALIAS_PATH_KEYS that cover nothing: {orphaned:?}"
        );
    }

    #[tokio::test]
    async fn apply_patch_missing_patch_is_missing_parameter() {
        let r = parse_and_expand_patch(&json!({}), &[]);
        assert!(matches!(r, Err(ToolError::MissingParameter("patch"))));
    }

    #[tokio::test]
    async fn apply_patch_without_workspace_is_unavailable() {
        let r = apply_patch_ops(&[], None, &[], &[]).await;
        assert!(matches!(
            r,
            Err(ToolError::Unavailable("workspace directory"))
        ));
    }
    // With no `limit` the default cap used to truncate silently, so the model saw 1000 names and concluded that was the whole directory.
    #[tokio::test]
    async fn file_list_default_limit_reports_truncation() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..1005 {
            std::fs::write(dir.path().join(format!("f{i:04}.txt")), b"").unwrap();
        }
        let first = tool_file_list(&json!({"path": "."}), Some(dir.path()), &[])
            .await
            .unwrap();
        assert_eq!(first.lines().filter(|l| l.ends_with(".txt")).count(), 1000);
        assert!(first.contains("showing 1000 of 1005 entries"), "{first}");
        assert!(first.contains("\"offset\": 1000"), "{first}");

        let rest = tool_file_list(&json!({"path": ".", "offset": 1000}), Some(dir.path()), &[])
            .await
            .unwrap();
        let names: Vec<&str> = rest.lines().collect();
        assert_eq!(
            names,
            [
                "f1000.txt",
                "f1001.txt",
                "f1002.txt",
                "f1003.txt",
                "f1004.txt"
            ]
        );
    }

    #[test]
    fn file_list_schema_documents_paging() {
        let defs = crate::tool_runner::builtin_tool_definitions();
        let def = defs.iter().find(|d| d.name == "file_list").unwrap();
        let props = &def.input_schema["properties"];
        assert!(props.get("limit").is_some());
        assert!(props.get("offset").is_some());
    }

    // A FIFO reports length 0, so the stat check always passes; only a bounded read enforces `max_bytes`.
    #[cfg(unix)]
    #[tokio::test]
    async fn file_read_enforces_max_bytes_on_fifo_growth() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let wpath = fifo.clone();
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&wpath)
                .unwrap();
            // A BrokenPipe once the reader stops is expected and ignored.
            let _ = f.write_all(&[b'a'; 4096]);
        });
        let r = tool_file_read(
            &json!({"path": "pipe", "max_bytes": 1024}),
            Some(dir.path()),
            &[],
        )
        .await;
        writer.join().unwrap();
        assert!(
            matches!(
                r,
                Err(ToolError::InvalidParameter {
                    name: "max_bytes",
                    ..
                })
            ),
            "got {r:?}"
        );
    }

    #[tokio::test]
    async fn file_read_within_limit_still_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let r = tool_file_read(
            &json!({"path": "a.txt", "max_bytes": 5}),
            Some(dir.path()),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(r, "hello");
    }

    // The rename replaces the inode, so without copying the mode an edited script lost its executable bit.
    #[cfg(unix)]
    #[tokio::test]
    async fn file_write_preserves_existing_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("run.sh");
        std::fs::write(&script, "old").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        tool_file_write(
            &json!({"path": "run.sh", "content": "echo hi"}),
            Some(dir.path()),
            &[],
        )
        .await
        .unwrap();
        let meta = std::fs::metadata(&script).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        assert_eq!(std::fs::read_to_string(&script).unwrap(), "echo hi");
    }

    #[tokio::test]
    async fn file_write_over_directory_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let r = tool_file_write(
            &json!({"path": "sub", "content": "x"}),
            Some(dir.path()),
            &[],
        )
        .await;
        assert!(r.is_err(), "{r:?}");
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["sub"]);
    }

    // The workspace root holds the agent's persisted manifest, which boot adopts when it differs from the database, so file_write must not reach it under any spelling.
    #[tokio::test]
    async fn file_write_refuses_own_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.toml"), "name = \"a\"\n").unwrap();
        for path in ["agent.toml", "./AGENT.toml"] {
            let r = tool_file_write(
                &json!({"path": path, "content": "[capabilities]\nshell = [\"*\"]\n"}),
                Some(dir.path()),
                &[],
            )
            .await;
            assert!(
                matches!(r, Err(ToolError::InvalidParameter { name: "path", .. })),
                "{path}: {r:?}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("agent.toml")).unwrap(),
            "name = \"a\"\n"
        );
        tool_file_write(
            &json!({"path": "sub/agent.toml", "content": "name = \"b\"\n"}),
            Some(dir.path()),
            &[],
        )
        .await
        .unwrap();
    }
}
