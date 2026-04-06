//! Context engine plugin management — install, remove, list, scaffold.
//!
//! Plugins live at `~/.librefang/plugins/<name>/` and contain:
//! - `plugin.toml`     — manifest (name, version, hooks, requirements)
//! - `hooks/`          — Python hook scripts (ingest.py, after_turn.py, etc.)
//! - `requirements.txt` — optional Python dependencies
//!
//! # Install sources
//! - **GitHub registry**: configurable `owner/repo` (default: `librefang/librefang-registry`)
//! - **Local path**: copy from a local directory
//! - **Git URL**: clone a git repo into the plugins directory

use librefang_types::config::PluginManifest;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Validate that a plugin name is a safe directory component (no path traversal).
pub fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Plugin name cannot be empty".to_string());
    }
    if name.len() > 128 {
        return Err(format!(
            "Invalid plugin name: exceeds maximum length of 128 characters (got {})",
            name.len()
        ));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name == "." {
        return Err(format!(
            "Invalid plugin name '{name}': must be a simple identifier (no /, \\, or ..)"
        ));
    }
    // Only allow alphanumeric, hyphens, underscores
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Invalid plugin name '{name}': only alphanumeric, hyphens, and underscores allowed"
        ));
    }
    Ok(())
}

/// Default plugin directory: `~/.librefang/plugins/`.
pub fn plugins_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| {
            warn!("HOME directory not set; using temporary directory for plugins");
            #[cfg(unix)]
            let fallback = PathBuf::from("/tmp/librefang");
            #[cfg(windows)]
            let fallback =
                PathBuf::from(std::env::var("TEMP").unwrap_or_else(|_| r"C:\Temp".to_string()))
                    .join("librefang");
            #[cfg(not(any(unix, windows)))]
            let fallback = PathBuf::from(".librefang");
            fallback
        })
        .join(".librefang")
        .join("plugins")
}

/// Ensure the plugins directory exists.
pub fn ensure_plugins_dir() -> std::io::Result<PathBuf> {
    let dir = plugins_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Information about an installed plugin, returned by list/get operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    /// Absolute path to the plugin directory.
    pub path: PathBuf,
    /// Whether all declared hook scripts exist on disk.
    pub hooks_valid: bool,
    /// Size of the plugin directory in bytes.
    pub size_bytes: u64,
}

/// Source for plugin installation.
#[derive(Debug, Clone)]
pub enum PluginSource {
    /// Install from a GitHub registry (`owner/repo`).
    /// `None` defaults to `librefang/librefang-registry`.
    Registry {
        name: String,
        github_repo: Option<String>,
    },
    /// Install from a local directory (copy).
    Local { path: PathBuf },
    /// Install from a git URL (clone).
    Git { url: String, branch: Option<String> },
}

/// Load and validate a plugin manifest from a directory.
pub fn load_plugin_manifest(plugin_dir: &Path) -> Result<PluginManifest, String> {
    let manifest_path = plugin_dir.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "plugin.toml not found at {}",
            manifest_path.display()
        ));
    }

    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {}: {e}", manifest_path.display()))?;

    let manifest: PluginManifest =
        toml::from_str(&content).map_err(|e| format!("Invalid plugin.toml: {e}"))?;

    Ok(manifest)
}

/// Get detailed info about a single installed plugin.
pub fn get_plugin_info(plugin_name: &str) -> Result<PluginInfo, String> {
    validate_plugin_name(plugin_name)?;
    let plugin_dir = plugins_dir().join(plugin_name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{plugin_name}' is not installed"));
    }

    let manifest = load_plugin_manifest(&plugin_dir)?;

    // Validate hook scripts exist
    let hooks_valid = check_hooks_exist(&plugin_dir, &manifest);

    // Calculate directory size
    let size_bytes = dir_size(&plugin_dir);

    Ok(PluginInfo {
        manifest,
        path: plugin_dir,
        hooks_valid,
        size_bytes,
    })
}

/// Doctor entry for a single installed plugin.
///
/// Tells the user whether the plugin is structurally valid (hook scripts
/// exist) *and* whether the runtime it asks for is usable on this host.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginDoctorEntry {
    pub name: String,
    /// Canonical runtime tag (`python`, `v`, ...). Falls back to the
    /// dispatcher's default (`python`) for plugins that don't declare one.
    pub runtime: String,
    /// `true` when the declared runtime's launcher resolved on PATH
    /// (or for `native`, always `true`).
    pub runtime_available: bool,
    /// `true` when every hook script declared in `plugin.toml` exists.
    pub hooks_valid: bool,
    /// Install hint surfaced when `runtime_available` is `false`.
    pub install_hint: String,
}

/// Aggregate doctor report: per-runtime availability + per-plugin readiness.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DoctorReport {
    /// Availability of every supported runtime, in stable order.
    pub runtimes: Vec<crate::plugin_runtime::RuntimeStatus>,
    /// One entry per installed plugin.
    pub plugins: Vec<PluginDoctorEntry>,
}

/// Probe the environment and return a diagnostic report.
///
/// Spawns one subprocess per runtime (`{launcher} --version`) — caller
/// should wrap in `tokio::task::spawn_blocking` if used from async.
pub fn run_doctor() -> DoctorReport {
    use crate::plugin_runtime::{check_runtime_status, PluginRuntime};

    let runtimes: Vec<_> = PluginRuntime::all()
        .iter()
        .map(|r| check_runtime_status(*r))
        .collect();

    // Index by runtime tag so per-plugin entries can look up availability
    // without re-probing subprocesses.
    let availability: std::collections::HashMap<&str, (bool, &str)> = runtimes
        .iter()
        .map(|s| (s.runtime.as_str(), (s.available, s.install_hint.as_str())))
        .collect();

    let plugins = list_plugins()
        .into_iter()
        .map(|info| {
            let runtime_kind = PluginRuntime::from_tag(info.manifest.hooks.runtime.as_deref());
            let tag = runtime_kind.label();
            let (available, hint) = availability.get(tag).copied().unwrap_or((false, ""));
            PluginDoctorEntry {
                name: info.manifest.name,
                runtime: tag.to_string(),
                runtime_available: available,
                hooks_valid: info.hooks_valid,
                install_hint: hint.to_string(),
            }
        })
        .collect();

    DoctorReport { runtimes, plugins }
}

/// List all installed plugins.
pub fn list_plugins() -> Vec<PluginInfo> {
    let dir = plugins_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            match get_plugin_info(&name) {
                Ok(info) => Some(info),
                Err(e) => {
                    warn!(plugin = name, error = %e, "Skipping invalid plugin");
                    None
                }
            }
        })
        .collect()
}

/// Install a plugin from a source.
pub async fn install_plugin(source: &PluginSource) -> Result<PluginInfo, String> {
    let plugins = ensure_plugins_dir().map_err(|e| format!("Cannot create plugins dir: {e}"))?;

    match source {
        PluginSource::Local { path } => install_from_local(path, &plugins),
        PluginSource::Registry { name, github_repo } => {
            let repo = github_repo
                .as_deref()
                .unwrap_or("librefang/librefang-registry");
            install_from_registry(name, repo, &plugins).await
        }
        PluginSource::Git { url, branch } => {
            install_from_git(url, branch.as_deref(), &plugins).await
        }
    }
}

/// Install from a local directory by copying.
fn install_from_local(src: &Path, plugins_dir: &Path) -> Result<PluginInfo, String> {
    // Canonicalize the source path to resolve symlinks and relative components
    let canonical_src = src
        .canonicalize()
        .map_err(|e| format!("Failed to resolve local path '{}': {e}", src.display()))?;

    // Reject paths that still contain '..' after canonicalization (should not happen, but defense in depth)
    if canonical_src
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "Refusing to install from path with '..' components: {}",
            canonical_src.display()
        ));
    }

    warn!(
        path = %canonical_src.display(),
        "Installing plugin from local path"
    );

    // Validate source has a plugin.toml
    let manifest = load_plugin_manifest(&canonical_src)?;
    // Validate manifest name is safe for use as a directory name
    validate_plugin_name(&manifest.name)?;
    let target_dir = plugins_dir.join(&manifest.name);

    if target_dir.exists() {
        return Err(format!(
            "Plugin '{}' already installed at {}. Remove it first.",
            manifest.name,
            target_dir.display()
        ));
    }

    copy_dir_recursive(&canonical_src, &target_dir)
        .map_err(|e| format!("Failed to copy plugin: {e}"))?;

    info!(plugin = manifest.name, "Installed plugin from local path");
    get_plugin_info(&manifest.name)
}

/// Validate that a GitHub repo string looks like `owner/repo`.
fn validate_github_repo(repo: &str) -> Result<(), String> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts[0].is_empty()
        || parts[1].is_empty()
        || repo.contains("..")
        || repo.contains(' ')
    {
        return Err(format!(
            "Invalid GitHub repo '{repo}': must be 'owner/repo'"
        ));
    }
    Ok(())
}

/// Install from a GitHub plugin registry (`owner/repo`).
async fn install_from_registry(
    name: &str,
    github_repo: &str,
    plugins_dir: &Path,
) -> Result<PluginInfo, String> {
    validate_plugin_name(name)?;
    validate_github_repo(github_repo)?;
    let target_dir = plugins_dir.join(name);
    if target_dir.exists() {
        return Err(format!(
            "Plugin '{name}' already installed. Remove it first."
        ));
    }

    let base_url = format!("https://api.github.com/repos/{github_repo}/contents/plugins");
    let listing_url = format!("{base_url}/{name}");

    let client = crate::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    // List files in the plugin directory
    let resp = client
        .get(&listing_url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch plugin '{name}' from registry: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Plugin '{name}' not found in registry (HTTP {})",
            resp.status()
        ));
    }

    let files: Vec<GitHubContent> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse registry response: {e}"))?;

    // Create target directory
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed to create plugin dir: {e}"))?;

    // Download each file — cleanup on failure
    let download_result = async {
        for file in &files {
            download_github_entry(&client, file, &target_dir, 0).await?;
        }
        Ok::<(), String>(())
    }
    .await;

    if let Err(e) = download_result {
        // Clean up partial download
        let _ = std::fs::remove_dir_all(&target_dir);
        return Err(format!("Failed to download plugin '{name}': {e}"));
    }

    info!(plugin = name, "Installed plugin from registry");
    get_plugin_info(name)
}

/// Lightweight entry returned when browsing a registry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegistryPluginEntry {
    pub name: String,
    pub registry: String,
}

/// List available plugin directory names from a GitHub registry.
pub async fn list_registry_plugins(github_repo: &str) -> Result<Vec<RegistryPluginEntry>, String> {
    validate_github_repo(github_repo)?;
    let url = format!("https://api.github.com/repos/{github_repo}/contents/plugins");
    let client = crate::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch registry '{github_repo}': {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Registry '{github_repo}' not accessible (HTTP {})",
            resp.status()
        ));
    }

    let entries: Vec<GitHubContent> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse registry listing: {e}"))?;

    Ok(entries
        .into_iter()
        .filter(|e| e.content_type == "dir")
        .map(|e| RegistryPluginEntry {
            name: e.name,
            registry: github_repo.to_string(),
        })
        .collect())
}

/// Install from a git URL by cloning.
async fn install_from_git(
    url: &str,
    branch: Option<&str>,
    plugins_dir: &Path,
) -> Result<PluginInfo, String> {
    // Validate URL to prevent argument injection (git interprets `-` prefixed args as flags)
    if url.starts_with('-') {
        return Err("Invalid git URL: must not start with '-'".to_string());
    }
    if !url.starts_with("https://")
        && !url.starts_with("http://")
        && !url.starts_with("git://")
        && !url.starts_with("ssh://")
        && !url.contains('@')
    {
        return Err(
            "Invalid git URL: must start with https://, http://, git://, or ssh://".to_string(),
        );
    }
    if let Some(b) = branch {
        if b.starts_with('-') {
            return Err("Invalid branch name: must not start with '-'".to_string());
        }
    }

    // Clone into a temp dir, validate, then move
    let temp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;

    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("clone").arg("--depth=1");
    if let Some(b) = branch {
        cmd.arg("--branch").arg(b);
    }
    // Use `--` to separate options from positional args
    cmd.arg("--").arg(url).arg(temp_dir.path());

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run git clone: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git clone failed: {stderr}"));
    }

    // Validate the cloned repo has a plugin.toml with a safe name
    let manifest = load_plugin_manifest(temp_dir.path())?;
    validate_plugin_name(&manifest.name)?;
    let target_dir = plugins_dir.join(&manifest.name);

    if target_dir.exists() {
        return Err(format!(
            "Plugin '{}' already installed. Remove it first.",
            manifest.name
        ));
    }

    // Move (rename) from temp to plugins dir
    copy_dir_recursive(temp_dir.path(), &target_dir)
        .map_err(|e| format!("Failed to install plugin: {e}"))?;

    // Remove .git directory to save space
    let git_dir = target_dir.join(".git");
    if git_dir.exists() {
        let _ = std::fs::remove_dir_all(&git_dir);
    }

    info!(plugin = manifest.name, "Installed plugin from git");
    get_plugin_info(&manifest.name)
}

/// Remove an installed plugin.
pub fn remove_plugin(name: &str) -> Result<(), String> {
    validate_plugin_name(name)?;
    let plugin_dir = plugins_dir().join(name);
    if !plugin_dir.exists() {
        return Err(format!("Plugin '{name}' is not installed"));
    }

    // Validate it's actually a plugin directory (has plugin.toml)
    if !plugin_dir.join("plugin.toml").exists() {
        return Err(format!(
            "Directory {} does not appear to be a plugin (no plugin.toml)",
            plugin_dir.display()
        ));
    }

    std::fs::remove_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to remove plugin '{name}': {e}"))?;

    info!(plugin = name, "Removed plugin");
    Ok(())
}

/// Create a scaffold for a new plugin. `runtime` defaults to `"python"`;
/// pass `"v"` / `"node"` / `"go"` / `"deno"` / `"native"` to generate a
/// template for that language instead.
pub fn scaffold_plugin(
    name: &str,
    description: &str,
    runtime: Option<&str>,
) -> Result<PathBuf, String> {
    validate_plugin_name(name)?;
    let plugins = ensure_plugins_dir().map_err(|e| format!("Cannot create plugins dir: {e}"))?;
    let plugin_dir = plugins.join(name);

    if plugin_dir.exists() {
        return Err(format!("Plugin '{name}' already exists"));
    }

    let hooks_dir = plugin_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Failed to create plugin directory: {e}"))?;

    // Normalize the runtime tag via PluginRuntime so aliases (py/js/golang/...)
    // resolve the same way the hook dispatcher will at runtime.
    let runtime_kind = crate::plugin_runtime::PluginRuntime::from_tag(runtime);
    let runtime_tag = runtime_kind.label();

    // Each runtime declares its own hook filenames + template body so the
    // manifest + files stay in sync.
    let (ingest_file, ingest_body, after_file, after_body) = hook_templates(runtime_kind);

    // Write plugin.toml as a hand-crafted string so we can include comments
    // that guide users toward the new hook slots.
    let runtime_line = if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python) {
        String::new()
    } else {
        format!("runtime = \"{runtime_tag}\"\n")
    };
    let requirements_line =
        if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python) {
            "requirements = \"requirements.txt\"\n".to_string()
        } else {
            String::new()
        };
    let manifest_toml = format!(
        r#"name = "{name}"
version = "0.1.0"
description = "{description}"

[hooks]
# --- Always-on hooks (uncomment to activate) ---
ingest = "hooks/{ingest_file}"
after_turn = "hooks/{after_file}"
{runtime_line}
# --- Optional lifecycle hooks ---
# bootstrap      = "hooks/bootstrap.{ext}"   # called once on engine init
# assemble       = "hooks/assemble.{ext}"    # control what the LLM sees (most powerful)
# compact        = "hooks/compact.{ext}"     # custom context compression
# prepare_subagent = "hooks/prepare_subagent.{ext}"
# merge_subagent   = "hooks/merge_subagent.{ext}"
{requirements_line}"#,
        name = name,
        description = description,
        ingest_file = ingest_file,
        after_file = after_file,
        runtime_line = runtime_line,
        requirements_line = requirements_line,
        ext = runtime_kind.script_extension(),
    );
    std::fs::write(plugin_dir.join("plugin.toml"), manifest_toml)
        .map_err(|e| format!("Failed to write plugin.toml: {e}"))?;

    let ingest_path = hooks_dir.join(ingest_file);
    let after_path = hooks_dir.join(after_file);
    std::fs::write(&ingest_path, ingest_body)
        .map_err(|e| format!("Failed to write {ingest_file}: {e}"))?;
    std::fs::write(&after_path, after_body)
        .map_err(|e| format!("Failed to write {after_file}: {e}"))?;

    // Native plugins exec the file directly, so the scaffolded shell wrapper
    // needs the executable bit. No-op on Windows (which uses extension-based
    // execution) and on other runtimes (interpreter handles execution).
    if runtime_kind.requires_executable_bit() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&ingest_path, &after_path] {
                if let Ok(meta) = std::fs::metadata(path) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o755);
                    let _ = std::fs::set_permissions(path, perms);
                }
            }
        }
    }

    // Python plugins get requirements.txt; other runtimes manage deps
    // their own way (go.mod, package.json, v.mod, ...).
    if matches!(runtime_kind, crate::plugin_runtime::PluginRuntime::Python) {
        std::fs::write(
            plugin_dir.join("requirements.txt"),
            "# Python dependencies\n",
        )
        .map_err(|e| format!("Failed to write requirements.txt: {e}"))?;
    }

    info!(
        plugin = name,
        runtime = runtime_tag,
        "Scaffolded new plugin"
    );
    Ok(plugin_dir)
}

/// Return scaffolded hook filenames + body content for a given runtime.
///
/// Returns `(ingest_filename, ingest_body, after_turn_filename, after_turn_body)`.
/// The scaffolded code is deliberately minimal — it shows the stdin/stdout
/// protocol, picks "no-op" defaults, and leaves a `TODO` comment.
fn hook_templates(
    runtime: crate::plugin_runtime::PluginRuntime,
) -> (&'static str, &'static str, &'static str, &'static str) {
    use crate::plugin_runtime::PluginRuntime as R;
    match runtime {
        R::Python => ("ingest.py", PY_INGEST, "after_turn.py", PY_AFTER_TURN),
        R::V => ("ingest.v", V_INGEST, "after_turn.v", V_AFTER_TURN),
        R::Node => ("ingest.js", NODE_INGEST, "after_turn.js", NODE_AFTER_TURN),
        R::Deno => ("ingest.ts", DENO_INGEST, "after_turn.ts", DENO_AFTER_TURN),
        R::Go => ("ingest.go", GO_INGEST, "after_turn.go", GO_AFTER_TURN),
        R::Ruby => ("ingest.rb", RUBY_INGEST, "after_turn.rb", RUBY_AFTER_TURN),
        R::Bash => ("ingest.sh", BASH_INGEST, "after_turn.sh", BASH_AFTER_TURN),
        // Bun uses TypeScript by convention; same format Deno uses.
        R::Bun => ("ingest.ts", BUN_INGEST, "after_turn.ts", BUN_AFTER_TURN),
        R::Php => ("ingest.php", PHP_INGEST, "after_turn.php", PHP_AFTER_TURN),
        R::Lua => ("ingest.lua", LUA_INGEST, "after_turn.lua", LUA_AFTER_TURN),
        R::Native => (
            // For native, we scaffold a shell wrapper so the plugin works
            // out of the box; users replace the script body with a real
            // pre-compiled binary (or a shebang'd interpreted script).
            "ingest",
            NATIVE_INGEST,
            "after_turn",
            NATIVE_AFTER_TURN,
        ),
    }
}

// --- Python templates (the original, kept verbatim for backwards compat) ---

const PY_INGEST: &str = r#"#!/usr/bin/env python3
"""Context engine ingest hook.

Receives via stdin:
    {
      "type": "ingest",
      "agent_id": "...",
      "message": "user message text",
      "peer_id": "platform-user-id-or-null"
    }

Should print to stdout:
    {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}

Tip: scope your recall to peer_id when present to prevent cross-user leaks.
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    agent_id = request["agent_id"]
    message = request["message"]
    peer_id = request.get("peer_id")  # None when called directly via API

    # TODO: Implement your custom recall logic here.
    # Example: query a vector database, search a knowledge base, etc.
    memories = []

    print(json.dumps({"type": "ingest_result", "memories": memories}))

if __name__ == "__main__":
    main()
"#;

const PY_AFTER_TURN: &str = r#"#!/usr/bin/env python3
"""Context engine after_turn hook.

Receives via stdin:
    {
      "type": "after_turn",
      "agent_id": "...",
      "messages": [{"role": "user"|"assistant", "content": "...", "pinned": false}, ...]
    }

Note: message content is truncated to 500 chars per message for performance.

Should print to stdout:
    {"type": "ok"}
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    agent_id = request["agent_id"]
    messages = request["messages"]

    # TODO: Implement your post-turn logic here.
    # Example: update indexes, persist state, log analytics, etc.

    print(json.dumps({"type": "ok"}))

if __name__ == "__main__":
    main()
"#;

const PY_ASSEMBLE: &str = r#"#!/usr/bin/env python3
"""Context engine assemble hook — controls what the LLM sees.

This is the most powerful hook. Called before every LLM request.

Receives via stdin:
    {
      "type": "assemble",
      "system_prompt": "...",
      "messages": [
        {"role": "user"|"assistant"|"tool", "content": <text or blocks>, "pinned": false},
        ...
      ],
      "context_window_tokens": 200000
    }

Messages use the full LibreFang message format — content can be a plain string
or a list of blocks (text, tool_use, tool_result, image, thinking).

Should print to stdout:
    {"type": "assemble_result", "messages": [...]}

Return a trimmed/reordered subset of messages that fits the token budget.
If you return an empty list or fail, LibreFang falls back to its default
overflow recovery (trim oldest, then compact).
"""
import json
import sys

def estimate_tokens(text: str) -> int:
    """Rough token estimate: ~4 chars per token."""
    return max(1, len(text) // 4)

def message_text(msg: dict) -> str:
    content = msg.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(
            b.get("text", b.get("content", ""))
            for b in content
            if isinstance(b, dict)
        )
    return ""

def main():
    request = json.loads(sys.stdin.read())
    messages = request["messages"]
    context_window_tokens = request["context_window_tokens"]

    # Reserve tokens for system prompt and response headroom
    budget = context_window_tokens - 4000

    # Keep messages newest-first until we exceed the budget, then stop
    kept = []
    used = 0
    for msg in reversed(messages):
        tokens = estimate_tokens(message_text(msg))
        if used + tokens > budget:
            break
        kept.append(msg)
        used += tokens

    kept.reverse()
    print(json.dumps({"type": "assemble_result", "messages": kept}))

if __name__ == "__main__":
    main()
"#;

const PY_COMPACT: &str = r#"#!/usr/bin/env python3
"""Context engine compact hook — custom context compression.

Called when the context window is under pressure.

Receives via stdin:
    {
      "type": "compact",
      "agent_id": "...",
      "messages": [...],   # full message list (same format as assemble)
      "model": "llama-3.3-70b-versatile",
      "context_window_tokens": 200000
    }

Should print to stdout:
    {"type": "compact_result", "messages": [...]}

Return a compacted version of the message list. If you fail or return
an empty list, LibreFang falls back to its built-in LLM-based compaction.
"""
import json
import sys

def message_text(msg: dict) -> str:
    content = msg.get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(
            b.get("text", b.get("content", ""))
            for b in content
            if isinstance(b, dict)
        )
    return ""

def main():
    request = json.loads(sys.stdin.read())
    messages = request["messages"]

    # Simple strategy: keep the first (system/context) message and the last 10
    pinned = [m for m in messages if m.get("pinned")]
    rest = [m for m in messages if not m.get("pinned")]

    summary_text = "... (older messages summarized) ..."
    summary_msg = {"role": "assistant", "content": summary_text, "pinned": False}

    if len(rest) > 10:
        compacted = pinned + [summary_msg] + rest[-10:]
    else:
        compacted = pinned + rest

    print(json.dumps({"type": "compact_result", "messages": compacted}))

if __name__ == "__main__":
    main()
"#;

// --- V language templates ---

const V_INGEST: &str = r#"// Context engine ingest hook (V).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "user message text"}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}
//
// Run with: `v run ingest.v` (or pre-compile: `v ingest.v`)
module main

import os
import json

struct IngestRequest {
	@type     string @[json: 'type']
	agent_id  string
	message   string
}

struct Memory {
	content string
}

struct IngestResult {
	@type    string   @[json: 'type']
	memories []Memory
}

fn main() {
	input := os.get_raw_stdin().bytestr()
	req := json.decode(IngestRequest, input) or {
		eprintln('ingest: invalid JSON on stdin: ${err}')
		exit(1)
	}
	_ := req.agent_id
	_ := req.message

	// TODO: Implement your custom recall logic here.
	result := IngestResult{
		@type: 'ingest_result'
		memories: []
	}
	println(json.encode(result))
}
"#;

const V_AFTER_TURN: &str = r#"// Context engine after_turn hook (V).
//
// Receives on stdin:
//   {"type": "after_turn", "agent_id": "...", "messages": [...]}
// Emits on stdout:
//   {"type": "ok"}
module main

import os
import json

struct AfterTurnRequest {
	@type    string @[json: 'type']
	agent_id string
}

struct Ok {
	@type string @[json: 'type']
}

fn main() {
	input := os.get_raw_stdin().bytestr()
	_ := json.decode(AfterTurnRequest, input) or {
		eprintln('after_turn: invalid JSON on stdin: ${err}')
		exit(1)
	}

	// TODO: persist state, update indexes, log analytics, ...

	println(json.encode(Ok{ @type: 'ok' }))
}
"#;

// --- Node templates ---

const NODE_INGEST: &str = r#"#!/usr/bin/env node
// Context engine ingest hook (Node.js).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "user message text"}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}

"use strict";

let buf = "";
process.stdin.on("data", (chunk) => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const agentId = req.agent_id;
  const message = req.message;

  // TODO: Implement your custom recall logic here.
  const memories = [];

  process.stdout.write(JSON.stringify({ type: "ingest_result", memories }) + "\n");
});
"#;

const NODE_AFTER_TURN: &str = r#"#!/usr/bin/env node
// Context engine after_turn hook (Node.js).

"use strict";

let buf = "";
process.stdin.on("data", (chunk) => { buf += chunk.toString("utf8"); });
process.stdin.on("end", () => {
  const req = JSON.parse(buf);
  const _agentId = req.agent_id;
  const _messages = req.messages;

  // TODO: persist state, update indexes, log analytics, ...

  process.stdout.write(JSON.stringify({ type: "ok" }) + "\n");
});
"#;

// --- Deno / TypeScript templates ---

const DENO_INGEST: &str = r#"// Context engine ingest hook (Deno / TypeScript).
//
// Run via `deno run --allow-read ingest.ts`.

interface IngestRequest { type: "ingest"; agent_id: string; message: string; }
interface Memory { content: string; }
interface IngestResult { type: "ingest_result"; memories: Memory[]; }

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
const req = JSON.parse(raw) as IngestRequest;
void req.agent_id; void req.message;

// TODO: Implement your custom recall logic here.
const result: IngestResult = { type: "ingest_result", memories: [] };
console.log(JSON.stringify(result));
"#;

const DENO_AFTER_TURN: &str = r#"// Context engine after_turn hook (Deno / TypeScript).

const raw = new TextDecoder().decode(await Deno.readAll(Deno.stdin));
void JSON.parse(raw);

// TODO: persist state, update indexes, log analytics, ...

console.log(JSON.stringify({ type: "ok" }));
"#;

// --- Go templates ---

const GO_INGEST: &str = r#"// Context engine ingest hook (Go).
//
// Run with: `go run ingest.go`
package main

import (
	"encoding/json"
	"io"
	"os"
)

type IngestRequest struct {
	Type    string `json:"type"`
	AgentID string `json:"agent_id"`
	Message string `json:"message"`
}

type Memory struct {
	Content string `json:"content"`
}

type IngestResult struct {
	Type     string   `json:"type"`
	Memories []Memory `json:"memories"`
}

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req IngestRequest
	if err := json.Unmarshal(raw, &req); err != nil {
		os.Exit(1)
	}
	_ = req.AgentID
	_ = req.Message

	// TODO: Implement your custom recall logic here.
	out, _ := json.Marshal(IngestResult{Type: "ingest_result", Memories: []Memory{}})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

const GO_AFTER_TURN: &str = r#"// Context engine after_turn hook (Go).
package main

import (
	"encoding/json"
	"io"
	"os"
)

func main() {
	raw, err := io.ReadAll(os.Stdin)
	if err != nil {
		os.Exit(1)
	}
	var req map[string]any
	_ = json.Unmarshal(raw, &req)

	// TODO: persist state, update indexes, log analytics, ...

	out, _ := json.Marshal(map[string]string{"type": "ok"})
	os.Stdout.Write(out)
	os.Stdout.Write([]byte("\n"))
}
"#;

// --- Native (bring-your-own-binary) templates ---

const NATIVE_INGEST: &str = r#"#!/bin/sh
# Native plugin ingest hook.
#
# Replace this shell wrapper with your own pre-compiled binary
# (V / Rust / Go / Zig / C++ — anything that speaks the JSON
# stdin/stdout protocol).
#
# Receives on stdin:
#   {"type": "ingest", "agent_id": "...", "message": "..."}
# Emits on stdout:
#   {"type": "ingest_result", "memories": [...]}
#
# chmod +x hooks/ingest to make this executable.

read -r _input
printf '{"type":"ingest_result","memories":[]}\n'
"#;

const NATIVE_AFTER_TURN: &str = r#"#!/bin/sh
# Native plugin after_turn hook — replace with your binary.
read -r _input
printf '{"type":"ok"}\n'
"#;

// --- Ruby templates ---

const RUBY_INGEST: &str = r#"# Context engine ingest hook (Ruby).
#
# Receives on stdin:
#   {"type": "ingest", "agent_id": "...", "message": "..."}
# Emits on stdout:
#   {"type": "ingest_result", "memories": [{"content": "..."}]}
require "json"

req = JSON.parse($stdin.read)
_agent_id = req["agent_id"]
_message  = req["message"]

# TODO: Implement your custom recall logic here.
memories = []

puts JSON.generate({ "type" => "ingest_result", "memories" => memories })
"#;

const RUBY_AFTER_TURN: &str = r#"# Context engine after_turn hook (Ruby).
require "json"

req = JSON.parse($stdin.read)
_agent_id = req["agent_id"]
_messages = req["messages"]

# TODO: Implement your post-turn logic here.

puts JSON.generate({ "type" => "ok" })
"#;

// --- Bash templates ---

const BASH_INGEST: &str = r#"#!/usr/bin/env bash
# Context engine ingest hook (Bash).
#
# Receives on stdin:
#   {"type":"ingest","agent_id":"...","message":"..."}
# Emits on stdout:
#   {"type":"ingest_result","memories":[]}
#
# For non-trivial logic, pipe stdin through `jq` or call out to a helper binary.
set -euo pipefail

_input=$(cat)
# TODO: parse "$_input" and build your recall result.
printf '{"type":"ingest_result","memories":[]}\n'
"#;

const BASH_AFTER_TURN: &str = r#"#!/usr/bin/env bash
# Context engine after_turn hook (Bash).
set -euo pipefail

_input=$(cat)
# TODO: persist state, update indexes, etc.
printf '{"type":"ok"}\n'
"#;

// --- Bun templates (TypeScript via Bun) ---

const BUN_INGEST: &str = r#"// Context engine ingest hook (Bun / TypeScript).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "..."}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "..."}]}
//
// Run with: `bun run ingest.ts`

interface IngestRequest {
  type: "ingest";
  agent_id: string;
  message: string;
}

interface Memory { content: string }

const input = await Bun.stdin.text();
const req = JSON.parse(input) as IngestRequest;
void req.agent_id;
void req.message;

// TODO: Implement your custom recall logic here.
const memories: Memory[] = [];

console.log(JSON.stringify({ type: "ingest_result", memories }));
"#;

const BUN_AFTER_TURN: &str = r#"// Context engine after_turn hook (Bun / TypeScript).
const input = await Bun.stdin.text();
const _req = JSON.parse(input);

// TODO: Implement your post-turn logic here.

console.log(JSON.stringify({ type: "ok" }));
"#;

// --- PHP templates ---

const PHP_INGEST: &str = r#"<?php
// Context engine ingest hook (PHP).
//
// Receives on stdin:
//   {"type": "ingest", "agent_id": "...", "message": "..."}
// Emits on stdout:
//   {"type": "ingest_result", "memories": [{"content": "..."}]}

$raw = stream_get_contents(STDIN);
$req = json_decode($raw, true);
$_agentId = $req["agent_id"] ?? null;
$_message = $req["message"] ?? null;

// TODO: Implement your custom recall logic here.
$memories = [];

echo json_encode(["type" => "ingest_result", "memories" => $memories]), "\n";
"#;

const PHP_AFTER_TURN: &str = r#"<?php
// Context engine after_turn hook (PHP).
$raw = stream_get_contents(STDIN);
$_req = json_decode($raw, true);

// TODO: Implement your post-turn logic here.

echo json_encode(["type" => "ok"]), "\n";
"#;

// --- Lua templates ---

const LUA_INGEST: &str = r#"-- Context engine ingest hook (Lua).
--
-- Receives on stdin:
--   {"type": "ingest", "agent_id": "...", "message": "..."}
-- Emits on stdout:
--   {"type": "ingest_result", "memories": [{"content": "..."}]}
--
-- Requires a JSON library on LUA_PATH (`luarocks install dkjson`).
local json = require("dkjson")

local raw = io.read("*a")
local req = json.decode(raw)
local _agent_id = req.agent_id
local _message  = req.message

-- TODO: Implement your custom recall logic here.
local memories = {}

io.write(json.encode({ type = "ingest_result", memories = memories }), "\n")
"#;

const LUA_AFTER_TURN: &str = r#"-- Context engine after_turn hook (Lua).
local json = require("dkjson")

local raw = io.read("*a")
local _req = json.decode(raw)

-- TODO: Implement your post-turn logic here.

io.write(json.encode({ type = "ok" }), "\n")
"#;

/// Install Python requirements for a plugin.
pub async fn install_requirements(plugin_name: &str) -> Result<String, String> {
    validate_plugin_name(plugin_name)?;
    let plugin_dir = plugins_dir().join(plugin_name);
    let requirements = plugin_dir.join("requirements.txt");

    if !requirements.exists() {
        return Ok("No requirements.txt found — nothing to install".to_string());
    }

    warn!(
        plugin = plugin_name,
        requirements = %requirements.display(),
        "Installing Python requirements with pip3 --user"
    );

    let output = tokio::process::Command::new("pip3")
        .args(["install", "--user", "-r"])
        .arg(&requirements)
        .output()
        .await
        .map_err(|e| format!("Failed to run pip3: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("pip3 install failed: {stderr}"))
    }
}

// ---------------------------------------------------------------------------
// GitHub API types
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct GitHubContent {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    download_url: Option<String>,
    url: Option<String>,
}

/// Recursively download a GitHub directory entry.
///
/// `depth` limits recursion to prevent unbounded traversal (max 10 levels).
async fn download_github_entry(
    client: &reqwest::Client,
    entry: &GitHubContent,
    target_dir: &Path,
    depth: usize,
) -> Result<(), String> {
    if depth > 10 {
        return Err("GitHub directory recursion depth exceeded (max 10 levels)".to_string());
    }

    // Validate entry.name to prevent path traversal attacks
    if entry.name.contains('/')
        || entry.name.contains('\\')
        || entry.name.contains("..")
        || entry.name.contains('\0')
    {
        return Err(format!(
            "Refusing to download entry with unsafe name: '{}'",
            entry.name
        ));
    }

    let target_path = target_dir.join(&entry.name);

    match entry.content_type.as_str() {
        "file" => {
            let download_url = entry
                .download_url
                .as_ref()
                .ok_or_else(|| format!("No download URL for {}", entry.name))?;

            let resp = client
                .get(download_url)
                .send()
                .await
                .map_err(|e| format!("Failed to download {}: {e}", entry.name))?;

            // Check Content-Length before downloading to reject oversized files early
            const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB per file
            if let Some(len) = resp.content_length() {
                if len > MAX_FILE_SIZE {
                    return Err(format!(
                        "File '{}' too large ({len} bytes, max {MAX_FILE_SIZE})",
                        entry.name
                    ));
                }
            }

            let content = resp
                .bytes()
                .await
                .map_err(|e| format!("Failed to read {}: {e}", entry.name))?;

            if content.len() as u64 > MAX_FILE_SIZE {
                return Err(format!(
                    "File '{}' too large ({} bytes, max {MAX_FILE_SIZE})",
                    entry.name,
                    content.len()
                ));
            }

            std::fs::write(&target_path, &content)
                .map_err(|e| format!("Failed to write {}: {e}", target_path.display()))?;

            debug!(
                file = entry.name,
                bytes = content.len(),
                "Downloaded plugin file"
            );
        }
        "dir" => {
            std::fs::create_dir_all(&target_path)
                .map_err(|e| format!("Failed to create dir: {e}"))?;

            // Recursively list and download subdirectory
            let sub_url = entry
                .url
                .as_ref()
                .ok_or_else(|| format!("No API URL for dir {}", entry.name))?;

            let resp = client
                .get(sub_url)
                .header("Accept", "application/vnd.github.v3+json")
                .send()
                .await
                .map_err(|e| format!("Failed to list dir {}: {e}", entry.name))?;

            let sub_entries: Vec<GitHubContent> = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse dir listing: {e}"))?;

            for sub_entry in &sub_entries {
                Box::pin(download_github_entry(
                    client,
                    sub_entry,
                    &target_path,
                    depth + 1,
                ))
                .await?;
            }
        }
        other => {
            debug!(
                name = entry.name,
                r#type = other,
                "Skipping unknown entry type"
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check that all declared hook scripts exist on disk and are within the plugin directory.
fn check_hooks_exist(plugin_dir: &Path, manifest: &PluginManifest) -> bool {
    // Canonicalize plugin_dir first so the starts_with check works even when
    // the input path contains symlinks (e.g. /tmp → /private/tmp on macOS).
    let canonical_dir = match plugin_dir.canonicalize() {
        Ok(d) => d,
        Err(_) => return false,
    };
    let check = |rel_path: &str| -> bool {
        let joined = canonical_dir.join(rel_path);
        // Canonicalize to resolve any `..` and verify the resolved path
        // stays inside the plugin directory. If canonicalize fails (file
        // doesn't exist), the hook is missing.
        match joined.canonicalize() {
            Ok(abs) => abs.starts_with(&canonical_dir),
            Err(_) => false,
        }
    };

    let mut valid = true;
    if let Some(ref p) = manifest.hooks.ingest {
        if !check(p) {
            valid = false;
        }
    }
    if let Some(ref p) = manifest.hooks.after_turn {
        if !check(p) {
            valid = false;
        }
    }
    valid
}

/// Calculate total size of a directory recursively.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = entry.metadata();
            if let Ok(m) = meta {
                if m.is_file() {
                    total += m.len();
                } else if m.is_dir() {
                    total += dir_size(&entry.path());
                }
            }
        }
    }
    total
}

/// Recursively copy a directory. Symlinks are skipped for security.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        // Skip symlinks to prevent following links outside the plugin directory
        if ft.is_symlink() {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugins_dir() {
        let dir = plugins_dir();
        assert!(dir.ends_with("plugins"));
        assert!(dir.to_string_lossy().contains(".librefang"));
    }

    #[test]
    fn test_list_plugins_no_panic() {
        // Should not panic even if plugins dir doesn't exist
        let _ = list_plugins();
    }

    #[test]
    fn test_get_plugin_not_installed() {
        let result = get_plugin_info("nonexistent-test-plugin-xyz");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not installed"));
    }

    #[test]
    fn test_remove_not_installed() {
        let result = remove_plugin("nonexistent-test-plugin-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_scaffold_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        // Override HOME to use temp dir
        let plugin_dir = tmp.path().join("test-scaffold-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        // Test manifest parsing from scaffold content
        let manifest_content = r#"name = "test-scaffold"
version = "0.1.0"
description = "Test scaffold"
author = ""

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
"#;
        let manifest: PluginManifest = toml::from_str(manifest_content).unwrap();
        assert_eq!(manifest.name, "test-scaffold");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.hooks.ingest.as_deref(), Some("hooks/ingest.py"));
        assert_eq!(
            manifest.hooks.after_turn.as_deref(),
            Some("hooks/after_turn.py")
        );
    }

    #[test]
    fn test_copy_dir_recursive() {
        let tmp_src = tempfile::tempdir().unwrap();
        let tmp_dst = tempfile::tempdir().unwrap();

        // Create source structure
        std::fs::create_dir_all(tmp_src.path().join("hooks")).unwrap();
        std::fs::write(tmp_src.path().join("plugin.toml"), "name = \"test\"").unwrap();
        std::fs::write(tmp_src.path().join("hooks/ingest.py"), "# hook").unwrap();

        let dst = tmp_dst.path().join("copied");
        copy_dir_recursive(tmp_src.path(), &dst).unwrap();

        assert!(dst.join("plugin.toml").exists());
        assert!(dst.join("hooks/ingest.py").exists());
    }

    #[test]
    fn test_dir_size() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "world!").unwrap();
        let size = dir_size(tmp.path());
        assert_eq!(size, 11); // 5 + 6
    }

    #[test]
    fn test_check_hooks_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
        std::fs::write(plugin_dir.join("hooks/ingest.py"), "").unwrap();

        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            author: None,
            hooks: librefang_types::config::ContextEngineHooks {
                ingest: Some("hooks/ingest.py".to_string()),
                after_turn: Some("hooks/after_turn.py".to_string()), // missing
                runtime: None,
            },
            requirements: None,
        };

        assert!(!check_hooks_exist(&plugin_dir, &manifest));

        // Now create the missing file
        std::fs::write(plugin_dir.join("hooks/after_turn.py"), "").unwrap();
        assert!(check_hooks_exist(&plugin_dir, &manifest));

        // Path traversal: hook pointing outside plugin dir should fail
        let manifest_escape = PluginManifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            author: None,
            hooks: librefang_types::config::ContextEngineHooks {
                ingest: Some("../../etc/passwd".to_string()),
                after_turn: None,
                runtime: None,
            },
            requirements: None,
        };
        assert!(!check_hooks_exist(&plugin_dir, &manifest_escape));
    }

    /// Integration test: install from GitHub registry, run hook, then remove.
    /// Ignored by default — requires network access.
    #[tokio::test]
    #[ignore]
    async fn test_registry_install_run_remove() {
        // 1. Install echo-memory from registry
        let source = PluginSource::Registry {
            name: "echo-memory".to_string(),
            github_repo: None,
        };
        let info = install_plugin(&source)
            .await
            .expect("registry install failed");
        assert_eq!(info.manifest.name, "echo-memory");
        assert_eq!(info.manifest.version, "0.1.0");
        assert!(info.hooks_valid);

        // 2. List should include it
        let plugins = list_plugins();
        assert!(plugins.iter().any(|p| p.manifest.name == "echo-memory"));

        // 3. Run ingest hook
        let ingest_path = info.path.join("hooks/ingest.py");
        assert!(ingest_path.exists());

        let mut child = tokio::process::Command::new("python3")
            .arg(&ingest_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("python3 should be available");

        {
            use tokio::io::AsyncWriteExt;
            let stdin = child.stdin.as_mut().unwrap();
            stdin
                .write_all(br#"{"type":"ingest","agent_id":"test-001","message":"Hello world"}"#)
                .await
                .unwrap();
        }
        child.stdin.take(); // close stdin
        let out = child.wait_with_output().await.unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("ingest_result"), "got: {stdout}");
        assert!(stdout.contains("echo-memory"), "got: {stdout}");

        // 4. Remove
        remove_plugin("echo-memory").expect("remove failed");
        assert!(get_plugin_info("echo-memory").is_err());
    }
}
