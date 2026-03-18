//! Context engine plugin management — install, remove, list, scaffold.
//!
//! Plugins live at `~/.librefang/plugins/<name>/` and contain:
//! - `plugin.toml`     — manifest (name, version, hooks, requirements)
//! - `hooks/`          — Python hook scripts (ingest.py, after_turn.py, etc.)
//! - `requirements.txt` — optional Python dependencies
//!
//! # Install sources
//! - **GitHub registry**: `librefang/plugin-registry` repo, one directory per plugin
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
    /// Install from the official GitHub registry (`librefang/plugin-registry`).
    Registry { name: String },
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
        PluginSource::Registry { name } => install_from_registry(name, &plugins).await,
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

/// Install from the GitHub registry.
async fn install_from_registry(name: &str, plugins_dir: &Path) -> Result<PluginInfo, String> {
    validate_plugin_name(name)?;
    let target_dir = plugins_dir.join(name);
    if target_dir.exists() {
        return Err(format!(
            "Plugin '{name}' already installed. Remove it first."
        ));
    }

    // Download from GitHub registry: librefang/plugin-registry/tree/main/plugins/<name>/
    let base_url = "https://api.github.com/repos/librefang/plugin-registry/contents/plugins";
    let listing_url = format!("{base_url}/{name}");

    let client = reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
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

/// Create a scaffold for a new plugin.
pub fn scaffold_plugin(name: &str, description: &str) -> Result<PathBuf, String> {
    validate_plugin_name(name)?;
    let plugins = ensure_plugins_dir().map_err(|e| format!("Cannot create plugins dir: {e}"))?;
    let plugin_dir = plugins.join(name);

    if plugin_dir.exists() {
        return Err(format!("Plugin '{name}' already exists"));
    }

    let hooks_dir = plugin_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("Failed to create plugin directory: {e}"))?;

    // Write plugin.toml — use toml serialization to avoid injection
    let manifest = PluginManifest {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        description: Some(description.to_string()),
        author: None,
        hooks: librefang_types::config::ContextEngineHooks {
            ingest: Some("hooks/ingest.py".to_string()),
            after_turn: Some("hooks/after_turn.py".to_string()),
        },
        requirements: None,
    };
    let manifest_toml =
        toml::to_string_pretty(&manifest).map_err(|e| format!("Failed to serialize TOML: {e}"))?;
    std::fs::write(plugin_dir.join("plugin.toml"), manifest_toml)
        .map_err(|e| format!("Failed to write plugin.toml: {e}"))?;

    // Write template ingest hook
    let ingest_template = r#"#!/usr/bin/env python3
"""Context engine ingest hook.

Receives via stdin:
    {"type": "ingest", "agent_id": "...", "message": "user message text"}

Should print to stdout:
    {"type": "ingest_result", "memories": [{"content": "recalled fact"}]}
"""
import json
import sys

def main():
    request = json.loads(sys.stdin.read())
    agent_id = request["agent_id"]
    message = request["message"]

    # TODO: Implement your custom recall logic here.
    # Example: query a vector database, search a knowledge base, etc.
    memories = []

    result = {"type": "ingest_result", "memories": memories}
    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#;
    std::fs::write(hooks_dir.join("ingest.py"), ingest_template)
        .map_err(|e| format!("Failed to write ingest.py: {e}"))?;

    // Write template after_turn hook
    let after_turn_template = r#"#!/usr/bin/env python3
"""Context engine after_turn hook.

Receives via stdin:
    {"type": "after_turn", "agent_id": "...", "messages": [...]}

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
    std::fs::write(hooks_dir.join("after_turn.py"), after_turn_template)
        .map_err(|e| format!("Failed to write after_turn.py: {e}"))?;

    // Write empty requirements.txt
    std::fs::write(
        plugin_dir.join("requirements.txt"),
        "# Python dependencies\n",
    )
    .map_err(|e| format!("Failed to write requirements.txt: {e}"))?;

    info!(plugin = name, "Scaffolded new plugin");
    Ok(plugin_dir)
}

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
        let manifest_content = format!(
            r#"name = "test-scaffold"
version = "0.1.0"
description = "Test scaffold"
author = ""

[hooks]
ingest = "hooks/ingest.py"
after_turn = "hooks/after_turn.py"
"#
        );
        let manifest: PluginManifest = toml::from_str(&manifest_content).unwrap();
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
