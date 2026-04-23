//! Workspace layout, identity files, and on-disk helpers.
//!
//! Pure functions extracted from `kernel.rs` to keep the main file
//! focused on `LibreFangKernel` impls. None of these touch
//! `LibreFangKernel` itself — they only manipulate paths and TOML
//! manifests.

use crate::error::{KernelError, KernelResult};
use librefang_types::agent::{AgentId, AgentManifest, WorkspaceMode};
use librefang_types::error::LibreFangError;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use tracing::info;

/// Ensure workspaces directory structure exists.
pub(super) fn ensure_workspaces_layout(home_dir: &Path) -> KernelResult<()> {
    let workspaces_dir = home_dir.join("workspaces");
    let agents_dir = workspaces_dir.join("agents");
    let hands_dir = workspaces_dir.join("hands");
    for dir in [&workspaces_dir, &agents_dir, &hands_dir] {
        std::fs::create_dir_all(dir).map_err(|e| {
            KernelError::LibreFang(LibreFangError::Internal(format!(
                "Failed to create {}: {e}",
                dir.display()
            )))
        })?;
    }
    Ok(())
}

/// One-shot migration from the legacy `<home>/agents/<name>/` layout to the
/// canonical `<home>/workspaces/agents/<name>/` layout.
///
/// Prior releases (and the `migrate` subcommand's output) placed per-agent
/// manifests under `<home>/agents/<name>/agent.toml`, while the runtime
/// reads from `<home>/workspaces/agents/<name>/`. This function moves any
/// stray directories on boot so existing installations keep working after
/// unification. Destinations that already exist are left alone — the
/// workspaces copy wins.
pub(super) fn migrate_legacy_agent_dirs(home_dir: &Path, workspaces_agents_dir: &Path) {
    let legacy = home_dir.join("agents");
    if !legacy.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&legacy) else {
        return;
    };
    let mut moved = 0usize;
    for entry in entries.flatten() {
        let src = entry.path();
        if !src.is_dir() || !src.join("agent.toml").exists() {
            continue;
        }
        let Some(name) = src.file_name() else {
            continue;
        };
        let dest = workspaces_agents_dir.join(name);
        if dest.exists() {
            tracing::warn!(
                src = %src.display(),
                dest = %dest.display(),
                "Legacy agent dir skipped — destination already exists"
            );
            continue;
        }
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::rename(&src, &dest) {
            Ok(()) => {
                moved += 1;
                tracing::info!(
                    src = %src.display(),
                    dest = %dest.display(),
                    "Migrated legacy agent dir"
                );
            }
            Err(e) => tracing::warn!(
                src = %src.display(),
                dest = %dest.display(),
                "Failed to migrate legacy agent dir: {e}"
            ),
        }
    }
    if moved > 0 {
        // Remove the legacy parent if it is now empty.
        let _ = std::fs::remove_dir(&legacy);
    }
}

/// One-shot migration: relocate stray `*.bak*` files left behind by older
/// versions at the home-dir root into `backups/`. Known producers:
/// `config.toml.bak`, `config.toml.bak.<ts>`, `integrations.toml.bak.<ts>`.
pub(super) fn migrate_root_backups(home_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(home_dir) else {
        return;
    };
    let candidates: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            if name.starts_with("config.toml.bak") || name.starts_with("integrations.toml.bak") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    if candidates.is_empty() {
        return;
    }
    let backups_dir = home_dir.join("backups");
    if std::fs::create_dir_all(&backups_dir).is_err() {
        return;
    }
    for src in candidates {
        let Some(name) = src.file_name().map(|n| n.to_os_string()) else {
            continue;
        };
        let dest = backups_dir.join(&name);
        if dest.exists() {
            // Already relocated in a prior run — discard the stray duplicate.
            let _ = std::fs::remove_file(&src);
            continue;
        }
        if let Err(e) = std::fs::rename(&src, &dest) {
            tracing::warn!(
                src = %src.display(),
                dest = %dest.display(),
                "Failed to relocate backup: {e}"
            );
        }
    }
}

/// One-shot cleanup: remove stray log files left at the home-dir root by
/// older CLI builds. Both were created (and truncated) by every CLI
/// invocation via a `tracing_subscriber` file layer, so they never held
/// useful history — modern CLI builds write to `logs/` (or drop the layer
/// entirely for one-shot commands) and the real daemon logs already live
/// in `logs/daemon.log`.
pub(super) fn cleanup_legacy_root_logs(home_dir: &Path) {
    for name in ["daemon.log", "tui.log"] {
        let path = home_dir.join(name);
        if path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// One-shot migration: relocate runtime state JSON files that older versions
/// wrote at the home-dir root into `data/`. Moves only the filenames LibreFang
/// is known to produce; unknown files are left alone.
pub(super) fn migrate_root_state_files(home_dir: &Path) {
    const STATE_FILES: &[&str] = &[
        "cron_jobs.json",
        "hand_state.json",
        "sessions.json",
        "workflow_runs.json",
        "custom_models.json",
        "model_overrides.json",
        "suppressed_providers.json",
        "webhooks.json",
    ];
    let data_dir = home_dir.join("data");
    let mut created_data_dir = false;
    for name in STATE_FILES {
        let src = home_dir.join(name);
        if !src.is_file() {
            continue;
        }
        if !created_data_dir {
            if std::fs::create_dir_all(&data_dir).is_err() {
                return;
            }
            created_data_dir = true;
        }
        let dest = data_dir.join(name);
        if dest.exists() {
            // Newer version already wrote the canonical copy — discard the
            // stale root duplicate.
            let _ = std::fs::remove_file(&src);
            continue;
        }
        if let Err(e) = std::fs::rename(&src, &dest) {
            tracing::warn!(
                src = %src.display(),
                dest = %dest.display(),
                "Failed to relocate state file: {e}"
            );
        }
    }
}

/// Initialize a git repo in the home directory for config version control.
pub(super) fn init_git_if_missing(home_dir: &Path) {
    if home_dir.join(".git").exists() {
        return;
    }
    let ok = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(home_dir)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return;
    }
    let gitignore = home_dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(
            &gitignore,
            "secrets.env\nvault.enc\ndaemon.json\nlogs/\ncache/\nregistry/\ndata/\ndashboard/\nbackups/\ninbox/\n.vscode/\n*.db\n*.db-shm\n*.db-wal\n",
        );
    }
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(home_dir)
        .status();
    let _ = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=LibreFang",
            "-c",
            "user.email=noreply@librefang.ai",
            "commit",
            "-q",
            "-m",
            "chore: initial librefang config",
        ])
        .current_dir(home_dir)
        .status();
    info!("Initialized git repo in {}", home_dir.display());
}

/// Create workspace directory structure for an agent.
pub(super) fn ensure_workspace(workspace: &Path) -> KernelResult<()> {
    for subdir in &[
        ".identity",
        "data",
        "output",
        "sessions",
        "skills",
        "logs",
        "memory",
    ] {
        std::fs::create_dir_all(workspace.join(subdir)).map_err(|e| {
            KernelError::LibreFang(LibreFangError::Internal(format!(
                "Failed to create workspace dir {}/{subdir}: {e}",
                workspace.display()
            )))
        })?;
    }
    // Write agent metadata file (best-effort)
    let meta = serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "workspace": workspace.display().to_string(),
    });
    let _ = std::fs::write(
        workspace.join("AGENT.json"),
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    );
    Ok(())
}

pub(super) fn safe_path_component(input: &str, fallback: &str) -> String {
    let sanitized: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

pub(super) fn has_unsafe_relative_components(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
}

pub(super) fn resolve_workspace_dir(
    workspaces_root: &Path,
    requested: Option<PathBuf>,
    agent_name: &str,
    agent_id: AgentId,
) -> KernelResult<PathBuf> {
    std::fs::create_dir_all(workspaces_root).map_err(|e| {
        KernelError::LibreFang(LibreFangError::Internal(format!(
            "Failed to create workspaces root {}: {e}",
            workspaces_root.display()
        )))
    })?;
    let root = workspaces_root.to_path_buf();

    if let Some(path) = requested {
        if path.is_absolute() || has_unsafe_relative_components(&path) {
            return Err(KernelError::LibreFang(LibreFangError::Internal(
                "Invalid workspace path".to_string(),
            )));
        }
        return Ok(root.join(path));
    }

    let fallback = agent_id.to_string();
    let component = safe_path_component(agent_name, &fallback);
    Ok(root.join(component))
}

/// Resolve the correct workspace directory for lazy backfill, respecting
/// hand agents whose workspace lives under `workspaces/hands/<hand>/<role>/`
/// rather than `workspaces/agents/<name>/`.
pub(super) fn backfill_workspace_dir(
    cfg: &librefang_types::config::KernelConfig,
    tags: &[String],
    agent_name: &str,
    agent_id: AgentId,
) -> KernelResult<PathBuf> {
    // Check if this is a hand agent by looking for "hand:<id>" and "hand_role:<role>" tags.
    let hand_id = tags.iter().find_map(|t| t.strip_prefix("hand:"));
    let hand_role = tags.iter().find_map(|t| t.strip_prefix("hand_role:"));

    if let (Some(hid), Some(role)) = (hand_id, hand_role) {
        let safe_hand = safe_path_component(hid, "hand");
        let safe_role = safe_path_component(role, "agent");
        let dir = cfg
            .effective_hands_workspaces_dir()
            .join(&safe_hand)
            .join(&safe_role);
        std::fs::create_dir_all(&dir).map_err(|e| {
            KernelError::LibreFang(LibreFangError::Internal(format!(
                "Failed to create hand workspace {}: {e}",
                dir.display()
            )))
        })?;
        Ok(dir)
    } else {
        resolve_workspace_dir(
            &cfg.effective_agent_workspaces_dir(),
            None,
            agent_name,
            agent_id,
        )
    }
}

/// Generate workspace identity files for an agent (SOUL.md, USER.md, TOOLS.md, MEMORY.md).
/// Files are written to `{workspace}/.identity/` to keep the workspace root clean and
/// allow multiple agents to share the same workspace without collisions.
///
/// User-editable files (SOUL, USER, MEMORY, AGENTS, BOOTSTRAP, IDENTITY) use `create_new`
/// to preserve manual edits. TOOLS.md is always rewritten so named workspace paths stay
/// current after the agent manifest is updated.
pub(super) fn generate_identity_files(
    workspace: &Path,
    manifest: &AgentManifest,
    resolved_workspaces: &HashMap<String, (PathBuf, WorkspaceMode)>,
) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let identity_dir = workspace.join(".identity");

    let soul_content = format!(
        "# Soul\n\
         You are {}. {}\n\
         Be genuinely helpful. Have opinions. Be resourceful before asking.\n\
         Treat user data with respect \u{2014} you are a guest in their life.\n",
        manifest.name,
        if manifest.description.is_empty() {
            "You are a helpful AI agent."
        } else {
            &manifest.description
        }
    );

    let user_content = "# User\n\
         <!-- Updated by the agent as it learns about the user -->\n\
         - Name:\n\
         - Timezone:\n\
         - Preferences:\n";

    let tools_content = build_tools_content(resolved_workspaces);

    let memory_content = "# Long-Term Memory\n\
         <!-- Curated knowledge the agent preserves across sessions -->\n";

    let agents_content = "# Agent Behavioral Guidelines\n\n\
         ## Core Principles\n\
         - Act first, narrate second. Use tools to accomplish tasks rather than describing what you'd do.\n\
         - Batch tool calls when possible \u{2014} don't output reasoning between each call.\n\
         - When a task is ambiguous, ask ONE clarifying question, not five.\n\
         - Store important context in memory (memory_store) proactively.\n\
         - Search memory (memory_recall) before asking the user for context they may have given before.\n\n\
         ## Tool Usage Protocols\n\
         - file_read BEFORE file_write \u{2014} always understand what exists.\n\
         - web_search for current info, web_fetch for specific URLs.\n\
         - browser_* for interactive sites that need clicks/forms.\n\
         - shell_exec: explain destructive commands before running.\n\n\
         ## Response Style\n\
         - Lead with the answer or result, not process narration.\n\
         - Keep responses concise unless the user asks for detail.\n\
         - Use formatting (headers, lists, code blocks) for readability.\n\
         - If a task fails, explain what went wrong and suggest alternatives.\n";

    let bootstrap_content = format!(
        "# First-Run Bootstrap\n\n\
         On your FIRST conversation with a new user, follow this protocol:\n\n\
         1. **Greet** \u{2014} Introduce yourself as {name} with a one-line summary of your specialty.\n\
         2. **Discover** \u{2014} Ask the user's name and one key preference relevant to your domain.\n\
         3. **Store** \u{2014} Use memory_store to save: user_name, their preference, and today's date as first_interaction.\n\
         4. **Orient** \u{2014} Briefly explain what you can help with (2-3 bullet points, not a wall of text).\n\
         5. **Serve** \u{2014} If the user included a request in their first message, handle it immediately after steps 1-3.\n\n\
         After bootstrap, this protocol is complete. Focus entirely on the user's needs.\n",
        name = manifest.name
    );

    let identity_content = format!(
        "---\n\
         name: {name}\n\
         archetype: assistant\n\
         vibe: helpful\n\
         emoji:\n\
         avatar_url:\n\
         greeting_style: warm\n\
         color:\n\
         ---\n\
         # Identity\n\
         <!-- Visual identity and personality at a glance. Edit these fields freely. -->\n",
        name = manifest.name
    );

    // User-editable files — never overwrite (preserve manual edits)
    let editable_files: &[(&str, &str)] = &[
        ("SOUL.md", &soul_content),
        ("USER.md", user_content),
        ("MEMORY.md", memory_content),
        ("AGENTS.md", agents_content),
        ("BOOTSTRAP.md", &bootstrap_content),
        ("IDENTITY.md", &identity_content),
    ];

    // Conditionally generate HEARTBEAT.md for autonomous agents
    let heartbeat_content = if manifest.autonomous.is_some() {
        Some(
            "# Heartbeat Checklist\n\
             <!-- Proactive reminders to check during heartbeat cycles -->\n\n\
             ## Every Heartbeat\n\
             - [ ] Check for pending tasks or messages\n\
             - [ ] Review memory for stale items\n\n\
             ## Daily\n\
             - [ ] Summarize today's activity for the user\n\n\
             ## Weekly\n\
             - [ ] Archive old sessions and clean up memory\n"
                .to_string(),
        )
    } else {
        None
    };

    for (filename, content) in editable_files {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(identity_dir.join(filename))
        {
            Ok(mut f) => {
                let _ = f.write_all(content.as_bytes());
            }
            Err(_) => {
                // File already exists — preserve user edits
            }
        }
    }

    // TOOLS.md is auto-generated config — always rewrite so named workspace paths stay current
    match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(identity_dir.join("TOOLS.md"))
    {
        Ok(mut f) => {
            let _ = f.write_all(tools_content.as_bytes());
        }
        Err(e) => {
            tracing::warn!("Failed to write TOOLS.md for {}: {e}", workspace.display());
        }
    }

    // Write HEARTBEAT.md for autonomous agents
    if let Some(ref hb) = heartbeat_content {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(identity_dir.join("HEARTBEAT.md"))
        {
            Ok(mut f) => {
                let _ = f.write_all(hb.as_bytes());
            }
            Err(_) => {
                // File already exists — preserve user edits
            }
        }
    }
}

/// Build the TOOLS.md content, injecting named workspace paths and modes.
fn build_tools_content(resolved_workspaces: &HashMap<String, (PathBuf, WorkspaceMode)>) -> String {
    let mut content = "# Tools & Environment\n\
        <!-- Auto-generated by LibreFang — do not edit manually, changes will be overwritten on spawn -->\n"
        .to_string();

    if !resolved_workspaces.is_empty() {
        content.push_str("\n## Shared Workspaces\n");
        let mut entries: Vec<_> = resolved_workspaces.iter().collect();
        entries.sort_by_key(|(name, _)| name.as_str());
        for (name, (path, mode)) in &entries {
            let mode_str = match mode {
                WorkspaceMode::ReadWrite => "read-write",
                WorkspaceMode::ReadOnly => "read-only",
            };
            content.push_str(&format!(
                "- **@{name}** → `{}` ({mode_str})\n",
                path.display()
            ));
        }
        content.push_str(
            "\nUse the paths above when reading or writing shared content.\n\
             Read-only workspaces reject write operations.\n",
        );
    }

    content
}

/// One-shot migration: move identity files from the workspace root into `.identity/`.
/// Called on every spawn; files that are already in `.identity/` are left alone.
pub(super) fn migrate_identity_files(workspace: &Path) {
    const IDENTITY_FILES: &[&str] = &[
        "SOUL.md",
        "USER.md",
        "TOOLS.md",
        "MEMORY.md",
        "AGENTS.md",
        "BOOTSTRAP.md",
        "IDENTITY.md",
        "HEARTBEAT.md",
    ];
    let identity_dir = workspace.join(".identity");
    let _ = std::fs::create_dir_all(&identity_dir);

    for name in IDENTITY_FILES {
        let src = workspace.join(name);
        if !src.is_file() {
            continue;
        }
        let dest = identity_dir.join(name);
        if dest.exists() {
            // `.identity/` copy wins — discard the stale root duplicate.
            let _ = std::fs::remove_file(&src);
            continue;
        }
        if let Err(e) = std::fs::rename(&src, &dest) {
            tracing::warn!(
                src = %src.display(),
                dest = %dest.display(),
                "Failed to migrate identity file: {e}"
            );
        }
    }
}

/// Create named workspace directories and return their resolved absolute paths with modes.
/// Paths are validated (no absolute paths, no `..` components).
pub(super) fn ensure_named_workspaces(
    workspaces_root: &Path,
    decls: &HashMap<String, librefang_types::agent::WorkspaceDecl>,
) -> HashMap<String, (PathBuf, WorkspaceMode)> {
    let mut resolved = HashMap::new();
    for (name, decl) in decls {
        if decl.path.is_absolute() || has_unsafe_relative_components(&decl.path) {
            tracing::warn!(name, path = %decl.path.display(), "Invalid named workspace path — skipped");
            continue;
        }
        let abs = workspaces_root.join(&decl.path);
        if let Err(e) = std::fs::create_dir_all(&abs) {
            tracing::warn!(name, path = %abs.display(), "Failed to create named workspace: {e}");
            continue;
        }
        // Canonicalize after create_dir_all so the path is consistent with
        // readonly_workspace_prefixes (which also canonicalizes). Skip on failure
        // rather than falling back to a non-canonical path that would silently
        // bypass the readonly check.
        let canonical = match abs.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(name, path = %abs.display(), "Failed to canonicalize named workspace: {e}");
                continue;
            }
        };
        resolved.insert(name.clone(), (canonical, decl.mode.clone()));
    }
    resolved
}

/// Append an assistant response summary to the daily memory log (best-effort, append-only).
/// Caps daily log at 1MB to prevent unbounded growth.
pub(super) fn append_daily_memory_log(workspace: &Path, response: &str) {
    use std::io::Write;
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return;
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_path = workspace.join("memory").join(format!("{today}.md"));
    // Security: cap total daily log to 1MB
    if let Ok(metadata) = std::fs::metadata(&log_path) {
        if metadata.len() > 1_048_576 {
            return;
        }
    }
    // Truncate long responses for the log (UTF-8 safe)
    let summary = librefang_types::truncate_str(trimmed, 500);
    let timestamp = chrono::Utc::now().format("%H:%M:%S").to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "\n## {timestamp}\n{summary}\n");
    }
}

/// Read a workspace identity file with a size cap to prevent prompt stuffing.
/// Checks `.identity/{filename}` first (new layout), falls back to `{filename}` at workspace
/// root (pre-migration layout). Returns None if the file doesn't exist or is empty.
pub(super) fn read_identity_file(workspace: &Path, filename: &str) -> Option<String> {
    const MAX_IDENTITY_FILE_BYTES: usize = 32_768; // 32KB cap

    // Prefer the new `.identity/` location; fall back to root for unmigrated workspaces.
    let candidates = [
        workspace.join(".identity").join(filename),
        workspace.join(filename),
    ];

    let ws_canonical = workspace.canonicalize().ok();

    for path in &candidates {
        // Security: ensure path stays inside workspace.
        // When ws_canonical is None the workspace doesn't exist yet — skip rather than
        // allowing the check to be bypassed.
        match path.canonicalize() {
            Ok(canonical) => match &ws_canonical {
                Some(wsc) if !canonical.starts_with(wsc) => continue,
                None => continue,
                _ => {}
            },
            Err(_) => continue, // file doesn't exist at this location
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if content.trim().is_empty() {
            continue;
        }
        return if content.len() > MAX_IDENTITY_FILE_BYTES {
            Some(librefang_types::truncate_str(&content, MAX_IDENTITY_FILE_BYTES).to_string())
        } else {
            Some(content)
        };
    }
    None
}

/// Get the system hostname as a String.
pub(super) fn gethostname() -> Option<String> {
    #[cfg(unix)]
    {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.trim().to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}
