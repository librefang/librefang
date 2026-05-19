//! Plugin install / removal / registry listing.
//!
//! Source-of-install dispatch (`PluginSource::Registry` / `Local` / `Git`),
//! the actual install paths (`install_from_registry`,
//! `install_from_local`, `install_from_git`), the GitHub-content downloader
//! (`download_github_entry`), the registry-listing cache, and the
//! Python-requirements installer that runs once per plugin.

use super::registry::{
    default_registry_cache_ttl_secs, fetch_verified_index, load_registry_cache,
    registry_cache_path, save_registry_cache,
};
use super::*;
use librefang_types::config::PluginI18n;
use std::collections::HashMap;

pub async fn install_plugin(source: &PluginSource) -> Result<PluginInfo, String> {
    let plugins = ensure_plugins_dir().map_err(|e| format!("Cannot create plugins dir: {e}"))?;

    let info = match source {
        PluginSource::Local { path } => {
            // install_from_local walks/copies a directory tree synchronously;
            // run it on the blocking pool so we don't stall the async runtime.
            let path = path.clone();
            let plugins = plugins.clone();
            tokio::task::spawn_blocking(move || install_from_local(&path, &plugins))
                .await
                .map_err(|e| format!("install_from_local task panicked: {e}"))?
        }
        PluginSource::Registry { name, github_repo } => {
            let repo = github_repo
                .as_deref()
                .unwrap_or("librefang/librefang-registry");
            install_from_registry(name, repo, &plugins).await
        }
        PluginSource::Git { url, branch } => {
            install_from_git(url, branch.as_deref(), &plugins).await
        }
    }?;

    // Check that all declared plugin dependencies are already installed.
    let raw_toml = tokio::fs::read_to_string(info.path.join("plugin.toml"))
        .await
        .unwrap_or_default();
    let needs = extract_needs(&raw_toml);
    if let Err(e) = check_plugin_needs(&needs) {
        // Don't remove the partially-installed plugin — let the user decide.
        // Just warn so they know what to install next.
        warn!("{e}");
    }

    // Warn about missing system binaries declared in [[requires]].
    let missing_bins = check_system_requires(&info.manifest.requires);
    for (bin, hint) in &missing_bins {
        let hint_str = hint.as_deref().unwrap_or("(no install hint provided)");
        warn!(
            "Plugin '{}' requires system binary '{}' which was not found on PATH. {}",
            info.manifest.name, bin, hint_str
        );
    }

    Ok(info)
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
    tokio::fs::create_dir_all(&target_dir)
        .await
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
        let _ = tokio::fs::remove_dir_all(&target_dir).await;
        return Err(format!("Failed to download plugin '{name}': {e}"));
    }

    // Verify checksum if a checksum file exists. This catches in-flight
    // tampering of the manifest bytes between GitHub and the daemon, but
    // does NOT serve as a fallback for the index-membership check below
    // (PR re-review HIGH-NEW-C): the .sha256 file lives on the same
    // GitHub repo as the plugin, so an attacker who controls the listing
    // can forge a matching checksum trivially.
    if let Some(expected) = fetch_checksum(&client, &listing_url, name).await {
        let manifest_bytes = tokio::fs::read(target_dir.join("plugin.toml"))
            .await
            .unwrap_or_default();
        if let Err(e) = verify_checksum(&manifest_bytes, &expected) {
            let _ = tokio::fs::remove_dir_all(&target_dir).await;
            return Err(e);
        }
        info!(plugin = name, "Checksum verified OK");
    } else {
        debug!(
            plugin = name,
            "No checksum file alongside this plugin release — relying on \
             signed index membership instead."
        );
    }

    // Per-plugin Ed25519 archive signatures are NOT served by the official
    // registry — the older code that fetched `{listing_url}.sig` was always
    // a 404 (PR review CRITICAL #3) and silently passed every install.
    // Instead, gate the install on membership in the signed plugins-index:
    // an attacker who can serve a malicious GitHub Contents listing for
    // `<name>` cannot also forge an entry for `<name>` in the worker's
    // Ed25519-signed flat index (the worker won't sign content it didn't
    // pull from the registry repo's committed `plugins-index.json`).
    // Note that `checksum_verified` is now NOT a fallback for index-fetch
    // failure (PR re-review HIGH-NEW-C). The SHA-256 checksum file lives
    // on the same attacker-controlled GitHub repo as the plugin itself,
    // so an attacker who can serve a doctored manifest with a matching
    // checksum AND DoS stats.librefang.ai gets a free pass on the older
    // logic. Refuse to install on any index-fetch failure: a real
    // operational network issue should stop installs (which fail safe),
    // not silently downgrade to a weaker integrity check.
    if std::env::var("LIBREFANG_ARCHIVE_VERIFY").as_deref() == Ok("0") {
        debug!("Index-membership verification disabled via LIBREFANG_ARCHIVE_VERIFY=0");
    } else {
        // Single retry with backoff catches transient network blips
        // without papering over a sustained outage / active downgrade.
        let mut last_err: Option<String> = None;
        let mut entries: Option<Vec<serde_json::Value>> = None;
        for attempt in 0..2 {
            match fetch_verified_index(&client, github_repo).await {
                Ok(es) => {
                    entries = Some(es);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt == 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }

        let Some(index_entries) = entries else {
            let _ = tokio::fs::remove_dir_all(&target_dir).await;
            return Err(format!(
                "Cannot verify plugin '{name}' integrity: signed registry index \
                 fetch failed after retry. {} Refusing to install — install must \
                 fail safe when the trust root is unreachable, regardless of any \
                 SHA-256 checksum on the GitHub repo (which an attacker who can \
                 serve a doctored manifest can also forge).",
                last_err.unwrap_or_default()
            ));
        };

        let in_index = index_entries
            .iter()
            .any(|e| e.get("name").and_then(|v| v.as_str()) == Some(name));
        if !in_index {
            let _ = tokio::fs::remove_dir_all(&target_dir).await;
            return Err(format!(
                "Plugin '{name}' is not present in the signed registry index. \
                 Refusing to install — the GitHub Contents listing alone is not \
                 a sufficient trust root. If this is a brand-new plugin, wait \
                 for the registry's CI to regenerate plugins-index.json and \
                 re-sign before installing."
            ));
        }
        info!(plugin = name, "Plugin presence in signed index confirmed");
    }

    // Bug #3804 — verify hook script integrity after install.
    //
    // The checksum above only covers plugin.toml (the manifest).  Hook scripts
    // that are referenced in the manifest but NOT listed in its [integrity]
    // section bypass all content verification — an attacker who controls the
    // download can serve a legitimate manifest with a valid checksum while
    // substituting malicious hook scripts.
    //
    // If the manifest declares hook scripts, every one of them MUST have a
    // corresponding entry in [integrity].  Missing entries are a hard error
    // for registry-installed plugins; authors who intentionally omit integrity
    // hashes (e.g. during development) can install via Local or Git sources.
    {
        let manifest_path = target_dir.join("plugin.toml");
        let manifest_opt = tokio::fs::read_to_string(&manifest_path)
            .await
            .ok()
            .and_then(|s| toml::from_str::<PluginManifest>(&s).ok());
        match manifest_opt {
            Some(manifest) => {
                let missing_integrity = manifest_missing_integrity_hooks(&manifest);
                if !missing_integrity.is_empty() {
                    // Hard error: registry plugins must declare integrity hashes for
                    // every hook script.  Without them, the hook content is unverified
                    // and could have been substituted after the manifest was signed.
                    let _ = tokio::fs::remove_dir_all(&target_dir).await;
                    return Err(format!(
                        "Plugin '{}' is missing [integrity] hashes for hook script(s): {}. \
                         Registry-installed plugins must provide SHA-256 checksums for every \
                         hook script declared in [hooks] so that tampered scripts are detected \
                         at load time. Add an [integrity] section to plugin.toml with \
                         \"hooks/<script>\" = \"<sha256hex>\" entries, or install via a local \
                         path (PluginSource::Local) to bypass this requirement.",
                        manifest.name,
                        missing_integrity.join(", ")
                    ));
                }
            }
            None => {
                // Manifest could not be re-read after install — treat as integrity failure.
                let _ = tokio::fs::remove_dir_all(&target_dir).await;
                return Err(format!(
                    "Plugin '{name}': failed to re-read plugin.toml after install \
                     — cannot verify hook script integrity"
                ));
            }
        }
    }

    info!(
        plugin = name,
        "Plugin installed successfully (manifest + hook script integrity verified)"
    );

    // Bust the registry cache so subsequent searches see an up-to-date index.
    let cache_path = registry_cache_path(github_repo);
    let _ = tokio::fs::remove_file(&cache_path).await;

    get_plugin_info(name)
}

/// Lightweight entry returned when browsing a registry.
///
/// Populated from each plugin's `plugin.toml` when available. Fields beyond
/// `name`/`registry` are optional so that registries that fail to serve a
/// manifest still degrade gracefully to a name-only listing.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RegistryPluginEntry {
    pub name: String,
    pub registry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Hook names declared by the plugin (e.g. `ingest`, `after_turn`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<String>,
    /// Per-language overrides for `name` / `description`. Keyed by BCP-47
    /// tag (`zh`, `zh-TW`, …). API routes resolve `Accept-Language` against
    /// this and fall back to the English values above.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub i18n: HashMap<String, PluginI18n>,
}

/// Disk cache file for an enriched registry listing.
///
/// Stored separately from the `index.json` cache so that listings built from
/// the GitHub Contents API + per-plugin manifest fetches do not clobber a
/// signed index cache.
fn registry_listing_cache_path(registry: &str) -> std::path::PathBuf {
    let cache_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".librefang")
        .join("registry_cache");
    let safe_name: String = registry
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_dir.join(format!("{safe_name}__listing.json"))
}

/// Fetch and parse `plugins/<name>/plugin.toml` from a registry, extracting the
/// fields we care about for a browse-listing card. Network and parse errors
/// degrade to `None` so a single bad plugin does not sink the whole listing.
async fn fetch_registry_plugin_meta(
    client: &reqwest::Client,
    github_repo: &str,
    name: &str,
) -> RegistryPluginEntry {
    let mut entry = RegistryPluginEntry {
        name: name.to_string(),
        registry: github_repo.to_string(),
        ..Default::default()
    };

    let url =
        format!("https://raw.githubusercontent.com/{github_repo}/main/plugins/{name}/plugin.toml");
    let text = match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => resp.text().await.ok(),
        _ => None,
    };
    let Some(text) = text else { return entry };

    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return entry;
    };
    if let Some(v) = value.get("version").and_then(|v| v.as_str()) {
        entry.version = Some(v.to_string());
    }
    if let Some(v) = value.get("description").and_then(|v| v.as_str()) {
        entry.description = Some(v.to_string());
    }
    if let Some(v) = value.get("author").and_then(|v| v.as_str()) {
        entry.author = Some(v.to_string());
    }
    if let Some(hooks) = value.get("hooks").and_then(|v| v.as_table()) {
        entry.hooks = hooks.keys().cloned().collect();
        entry.hooks.sort();
    }
    entry.i18n = parse_plugin_i18n_blocks(&value);
    entry
}

/// Pull `[i18n.<lang>]` tables off a parsed plugin TOML, keeping only the
/// `name` and `description` overrides. Empty entries (neither field set)
/// are dropped to keep the map tight.
///
/// Exposed as `pub(crate)` so it can be unit-tested without a network
/// round-trip; the production caller is `fetch_registry_plugin_meta`.
pub(crate) fn parse_plugin_i18n_blocks(value: &toml::Value) -> HashMap<String, PluginI18n> {
    let mut out: HashMap<String, PluginI18n> = HashMap::new();
    let Some(i18n) = value.get("i18n").and_then(|v| v.as_table()) else {
        return out;
    };
    for (lang, body) in i18n {
        let Some(tbl) = body.as_table() else { continue };
        let pi = PluginI18n {
            name: tbl
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: tbl
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        };
        if pi.name.is_some() || pi.description.is_some() {
            out.insert(lang.clone(), pi);
        }
    }
    out
}

/// List available plugins in a GitHub registry, enriched with manifest metadata.
///
/// Lists `plugins/` via the GitHub Contents API, then fetches each plugin's
/// `plugin.toml` concurrently to populate `version/description/author/hooks`.
/// Results are cached to disk with the same TTL as the signed index cache
/// to avoid hammering GitHub on every dashboard reload.
pub async fn list_registry_plugins(github_repo: &str) -> Result<Vec<RegistryPluginEntry>, String> {
    validate_github_repo(github_repo)?;

    let ttl = std::env::var("LIBREFANG_REGISTRY_CACHE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(default_registry_cache_ttl_secs);
    let skip_cache = std::env::var("LIBREFANG_REGISTRY_NO_CACHE").as_deref() == Ok("1");
    let cache_path = registry_listing_cache_path(github_repo);

    if !skip_cache {
        if let Some(bytes) = load_registry_cache(&cache_path, ttl) {
            if let Ok(cached) = serde_json::from_slice::<Vec<RegistryPluginEntry>>(&bytes) {
                debug!(
                    "Using cached registry listing for {github_repo} ({} plugins)",
                    cached.len()
                );
                return Ok(cached);
            }
        }
    }

    let client = crate::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let url = format!("https://api.github.com/repos/{github_repo}/contents/plugins");
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

    let names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.content_type == "dir")
        .map(|e| e.name)
        .collect();

    let futs = names
        .iter()
        .map(|n| fetch_registry_plugin_meta(&client, github_repo, n));
    let mut plugins: Vec<RegistryPluginEntry> = futures::future::join_all(futs).await;
    plugins.sort_by(|a, b| a.name.cmp(&b.name));

    if !skip_cache {
        if let Ok(bytes) = serde_json::to_vec(&plugins) {
            save_registry_cache(&cache_path, &bytes);
        }
    }

    Ok(plugins)
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

    // Validate the cloned repo has a plugin.toml with a safe name.
    // load_plugin_manifest reads files synchronously; run on the blocking pool.
    let manifest_dir = temp_dir.path().to_path_buf();
    let manifest = tokio::task::spawn_blocking(move || load_plugin_manifest(&manifest_dir))
        .await
        .map_err(|e| format!("load_plugin_manifest task failed: {e}"))??;
    validate_plugin_name(&manifest.name)?;
    let target_dir = plugins_dir.join(&manifest.name);

    if target_dir.exists() {
        return Err(format!(
            "Plugin '{}' already installed. Remove it first.",
            manifest.name
        ));
    }

    // Move (rename) from temp to plugins dir.
    // copy_dir_recursive walks/copies a directory tree synchronously; run on the
    // blocking pool so we don't stall the async runtime.
    let copy_src = temp_dir.path().to_path_buf();
    let copy_dst = target_dir.clone();
    tokio::task::spawn_blocking(move || copy_dir_recursive(&copy_src, &copy_dst))
        .await
        .map_err(|e| format!("copy_dir_recursive task failed: {e}"))?
        .map_err(|e| format!("Failed to install plugin: {e}"))?;

    // Remove .git directory to save space
    let git_dir = target_dir.join(".git");
    if git_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&git_dir).await;
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

/// Install Python requirements for a plugin.
pub async fn install_requirements(plugin_name: &str) -> Result<String, String> {
    validate_plugin_name(plugin_name)?;
    let plugin_dir = plugins_dir().join(plugin_name);
    let requirements = plugin_dir.join("requirements.txt");

    if !requirements.exists() {
        return Ok("No requirements.txt found — nothing to install".to_string());
    }

    // In virtualenv/conda environments, pip forbids --user installs.
    let in_venv = std::env::var("VIRTUAL_ENV").is_ok() || std::env::var("CONDA_PREFIX").is_ok();
    let mut args = vec!["-m", "pip", "install"];
    if !in_venv {
        args.push("--user");
    }
    args.push("-r");

    warn!(
        plugin = plugin_name,
        requirements = %requirements.display(),
        venv = in_venv,
        "Installing Python requirements"
    );

    let output = tokio::process::Command::new("python")
        .args(&args)
        .arg(&requirements)
        .output()
        .await
        .map_err(|e| format!("Failed to run python -m pip: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("python -m pip install failed: {stderr}"))
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

            tokio::fs::write(&target_path, &content)
                .await
                .map_err(|e| format!("Failed to write {}: {e}", target_path.display()))?;

            debug!(
                file = entry.name,
                bytes = content.len(),
                "Downloaded plugin file"
            );
        }
        "dir" => {
            tokio::fs::create_dir_all(&target_path)
                .await
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
